//! 中性的"待发提示"发件箱:面板的 `update` 拿不到 `App`,失败时往自己 state 的发件箱里 `push`,
//! 产品每条消息后统一排空成自己的 Toast。与 Dozer `extensions::toast::Outbox` 同语义,但**不依赖**
//! `dozer_core::log::Scope` 与 Dozer 的 i18n 解析器,所以共享面板库可以用。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dozer_model::LocalizedText;

/// 读失败类提示的冷却:轮询会每秒重试一次,持续性失败若每次都弹,Toast 会一直被续期;
/// 相同 key 下相同文本在这段时间内只提示一次。
pub const NOTICE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: LocalizedText,
    pub key: Option<String>,
}

#[derive(Debug, Default)]
pub struct NoticeOutbox {
    items: Vec<Notice>,
    /// 每个 key 上一次经 `push_keyed_cooldown` 推出的文本与时刻。排空(`take`)不清它,
    /// 否则每帧排空一次就等于没有冷却。比较用 `LocalizedText` 的结构相等(key + 参数),
    /// 与语言无关,locale 切换不影响冷却。
    cooldowns: HashMap<String, (LocalizedText, Instant)>,
}

impl NoticeOutbox {
    pub fn push(&mut self, level: NoticeLevel, text: impl Into<LocalizedText>) {
        self.items.push(Notice { level, text: text.into(), key: None });
    }

    /// 带去重键的版本:同 key 再推只刷新文本与计时(语义同 `App::push_toast_keyed`)。
    pub fn push_keyed(&mut self, level: NoticeLevel, text: impl Into<LocalizedText>, key: impl Into<String>) {
        self.items.push(Notice { level, text: text.into(), key: Some(key.into()) });
    }

    /// 带冷却的 `push_keyed`:同一 key 下,与上一次推出的文本相同且距今不足 [`NOTICE_COOLDOWN`] 则不推;
    /// 文本变了或已过冷却则推并更新记录。`now` 由调用方注入。
    pub fn push_keyed_cooldown(&mut self, level: NoticeLevel, text: impl Into<LocalizedText>, key: impl Into<String>, now: Instant) {
        let text = text.into();
        let key = key.into();
        let repeat = matches!(self.cooldowns.get(&key), Some((last, at)) if *last == text && now.duration_since(*at) < NOTICE_COOLDOWN);
        if repeat {
            return;
        }
        self.cooldowns.insert(key.clone(), (text.clone(), now));
        self.push_keyed(level, text, key);
    }

    pub fn take(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str) -> LocalizedText {
        LocalizedText::keyed("dozer.todo", key, "fallback")
    }

    #[test]
    fn push_and_take_preserve_order_and_empty_the_box() {
        let mut o = NoticeOutbox::default();
        o.push(NoticeLevel::Error, text("a"));
        o.push_keyed(NoticeLevel::Warning, text("b"), "k");
        let items = o.take();
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].level, items[0].key.as_deref()), (NoticeLevel::Error, None));
        assert_eq!((items[1].level, items[1].key.as_deref()), (NoticeLevel::Warning, Some("k")));
        assert!(o.take().is_empty());
    }

    /// 评审关注点 2:轮询每秒失败一次,相同文本在冷却内只提示一次。
    #[test]
    fn the_same_text_inside_the_cooldown_is_pushed_once() {
        let mut o = NoticeOutbox::default();
        let t0 = Instant::now();
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0);
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0 + Duration::from_secs(1));
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0 + Duration::from_secs(59));
        assert_eq!(o.take().len(), 1);
    }

    #[test]
    fn a_different_text_or_an_expired_cooldown_pushes_again() {
        let mut o = NoticeOutbox::default();
        let t0 = Instant::now();
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0);
        o.push_keyed_cooldown(NoticeLevel::Error, text("other"), "poll", t0 + Duration::from_secs(1));
        o.push_keyed_cooldown(NoticeLevel::Error, text("other"), "poll", t0 + Duration::from_secs(61));
        assert_eq!(o.take().len(), 3, "a changed error shows again, and so does the same one after 60s");
    }

    #[test]
    fn draining_does_not_reset_the_cooldown() {
        let mut o = NoticeOutbox::default();
        let t0 = Instant::now();
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0);
        assert_eq!(o.take().len(), 1);
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0 + Duration::from_secs(1));
        assert!(o.take().is_empty(), "draining every frame must not defeat the cooldown");
    }
}
