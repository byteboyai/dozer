//! 统一消息 Toast 的纯逻辑层:去重、堆叠上限、按级别到期。时间由调用方
//! 注入(`now`),不在内部取 `Instant::now()`,便于单测。App 级状态(挂
//! `App.toast`,不挂 `Workspace`——跨所有项目页签共享)。设计见
//! `docs/superpowers/specs/2026-09-30-unified-toast-design.md`。

use std::time::{Duration, Instant};

/// 同屏最多显示的 Toast 条数;超出时挤掉最旧的(瞬时消息过期即无意义,不排队)。
pub const MAX_VISIBLE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    /// 自动消失时长。v1 不支持悬停暂停/手动关闭(见计划「有意偏差」1),
    /// 所有级别都会到期,Error 给最长。
    pub fn duration(self) -> Duration {
        match self {
            Level::Info | Level::Success => Duration::from_secs(3),
            Level::Warning => Duration::from_secs(5),
            Level::Error => Duration::from_secs(8),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    /// 单调递增,窗口宿主用它判断"堆叠是否变了"。
    pub id: u64,
    pub level: Level,
    pub text: String,
    /// 去重键:同 key 再推 = 刷新文本/级别/计时,不新增。
    pub key: Option<String>,
    pub expires: Instant,
}

#[derive(Debug, Default)]
pub struct ToastCenter {
    items: Vec<Toast>,
    next_id: u64,
}

/// 顶层 `Message::Toast(toast::Message::..)` 的载荷。extension 想弹 Toast
/// 时向内核发这条消息,不直接持有 `ToastCenter`。
#[derive(Debug, Clone)]
pub enum Message {
    Push {
        level: Level,
        text: String,
        key: Option<String>,
    },
}

pub fn update(state: &mut ToastCenter, msg: Message, now: Instant) {
    match msg {
        Message::Push { level, text, key } => {
            state.push(level, &text, key, now);
        }
    }
}

/// 折叠所有空白(含换行)成单个空格并去首尾:Toast 是固定高度的单/双行卡片,
/// 多行文本会撑坏几何。
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl ToastCenter {
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// 推一条。返回条目 id;文本归一化后为空则忽略返回 `None`。
    pub fn push(
        &mut self,
        level: Level,
        text: &str,
        key: Option<String>,
        now: Instant,
    ) -> Option<u64> {
        let text = normalize(text);
        if text.is_empty() {
            return None;
        }
        let expires = now + level.duration();
        let existing = self.items.iter_mut().find(|t| match (&key, &t.key) {
            (Some(k), Some(tk)) => k == tk,
            (None, None) => t.level == level && t.text == text,
            _ => false,
        });
        if let Some(t) = existing {
            t.level = level;
            t.text = text;
            t.expires = expires;
            return Some(t.id);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Toast {
            id,
            level,
            text,
            key,
            expires,
        });
        if self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
        Some(id)
    }

    /// 移除到期条目(`expires <= now`)。返回是否有变化。
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.expires > now);
        self.items.len() != before
    }

    /// 距最近一条到期的剩余时间;没有条目返回 `None`(主循环不再空转)。
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        self.items
            .iter()
            .map(|t| t.expires)
            .min()
            .map(|e| e.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center() -> (ToastCenter, Instant) {
        (ToastCenter::default(), Instant::now())
    }

    #[test]
    fn level_durations_follow_spec() {
        assert_eq!(Level::Info.duration(), Duration::from_secs(3));
        assert_eq!(Level::Success.duration(), Duration::from_secs(3));
        assert_eq!(Level::Warning.duration(), Duration::from_secs(5));
        assert_eq!(Level::Error.duration(), Duration::from_secs(8));
    }

    #[test]
    fn push_assigns_increasing_ids_and_expiry() {
        let (mut c, now) = center();
        let a = c.push(Level::Info, "a", None, now).unwrap();
        let b = c.push(Level::Error, "b", None, now).unwrap();
        assert!(b > a);
        assert_eq!(c.items().len(), 2);
        assert_eq!(c.items()[0].expires, now + Duration::from_secs(3));
        assert_eq!(c.items()[1].expires, now + Duration::from_secs(8));
    }

    #[test]
    fn empty_or_whitespace_text_is_ignored() {
        let (mut c, now) = center();
        assert_eq!(c.push(Level::Info, "", None, now), None);
        assert_eq!(c.push(Level::Info, "  \n\t ", None, now), None);
        assert!(c.items().is_empty());
    }

    #[test]
    fn newlines_and_runs_of_whitespace_collapse_to_single_spaces() {
        let (mut c, now) = center();
        c.push(Level::Error, "第一行\n  第二行\t\t第三行", None, now);
        assert_eq!(c.items()[0].text, "第一行 第二行 第三行");
    }

    #[test]
    fn same_key_refreshes_instead_of_adding() {
        let (mut c, now) = center();
        let a = c.push(Level::Error, "old", Some("k".into()), now).unwrap();
        let later = now + Duration::from_secs(2);
        let b = c
            .push(Level::Warning, "new", Some("k".into()), later)
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].text, "new");
        assert_eq!(c.items()[0].level, Level::Warning);
        assert_eq!(c.items()[0].expires, later + Duration::from_secs(5));
    }

    #[test]
    fn without_key_same_level_and_text_dedups_but_different_level_does_not() {
        let (mut c, now) = center();
        c.push(Level::Error, "x", None, now);
        c.push(Level::Error, "x", None, now);
        assert_eq!(c.items().len(), 1);
        c.push(Level::Warning, "x", None, now);
        assert_eq!(c.items().len(), 2);
    }

    #[test]
    fn key_none_and_key_some_never_dedup_against_each_other() {
        let (mut c, now) = center();
        c.push(Level::Error, "x", Some("k".into()), now);
        c.push(Level::Error, "x", None, now);
        assert_eq!(c.items().len(), 2);
    }

    #[test]
    fn overflow_evicts_the_oldest() {
        let (mut c, now) = center();
        for t in ["1", "2", "3", "4"] {
            c.push(Level::Info, t, None, now);
        }
        let texts: Vec<_> = c.items().iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["2", "3", "4"]);
        assert_eq!(c.items().len(), MAX_VISIBLE);
    }

    #[test]
    fn expire_removes_due_items_and_reports_change() {
        let (mut c, now) = center();
        c.push(Level::Info, "short", None, now);
        c.push(Level::Error, "long", None, now);
        assert!(!c.expire(now + Duration::from_secs(2)));
        // 恰好到点(expires == now)也要移除,否则 next_wake 会返回 ZERO 造成空转。
        assert!(c.expire(now + Duration::from_secs(3)));
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].text, "long");
        assert!(c.expire(now + Duration::from_secs(8)));
        assert!(c.items().is_empty());
        assert!(!c.expire(now + Duration::from_secs(60)));
    }

    #[test]
    fn next_wake_is_time_to_earliest_expiry_or_none() {
        let (mut c, now) = center();
        assert_eq!(c.next_wake(now), None);
        c.push(Level::Error, "e", None, now);
        c.push(Level::Info, "i", None, now);
        assert_eq!(c.next_wake(now), Some(Duration::from_secs(3)));
        assert_eq!(
            c.next_wake(now + Duration::from_secs(1)),
            Some(Duration::from_secs(2))
        );
        // 已过期但尚未 expire():饱和为 0,而不是下溢。
        assert_eq!(
            c.next_wake(now + Duration::from_secs(10)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn update_push_message_delegates_to_push() {
        let (mut c, now) = center();
        update(
            &mut c,
            Message::Push {
                level: Level::Success,
                text: "ok".into(),
                key: None,
            },
            now,
        );
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].level, Level::Success);
    }
}
