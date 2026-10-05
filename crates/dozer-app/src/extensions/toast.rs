//! 统一消息 Toast 的纯逻辑层:去重、堆叠上限、按级别到期。时间由调用方
//! 注入(`now`),不在内部取 `Instant::now()`,便于单测。App 级状态(挂
//! `App.toast`,不挂 `Workspace`——跨所有项目页签共享)。设计见
//! `docs/superpowers/specs/2026-09-30-unified-toast-design.md`。

use std::time::{Duration, Instant};

use dozer_core::log::Scope;

dozer_core::scope!(LOG, module, "toast");

/// 同屏最多显示的 Toast 条数;超出时挤掉最旧的(瞬时消息过期即无意义,不排队)。
pub const MAX_VISIBLE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    // 预留的级别:目前的调用点都是失败/警告,成功/信息类提示暂无调用点。
    #[allow(dead_code)]
    Info,
    #[allow(dead_code)]
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
        /// 调用方的日志来源(只用于写日志,不进入 `ToastCenter`)。
        scope: Scope,
        level: Level,
        text: String,
        key: Option<String>,
    },
}

pub fn update(state: &mut ToastCenter, msg: Message, now: Instant) {
    match msg {
        Message::Push {
            level, text, key, ..
        } => {
            state.push(level, &text, key, now);
        }
    }
}

/// 每条 Toast 自动写一条日志。`tracing` 的 `target:` 必须是常量,而这里的
/// `scope` 是运行时形参,所以固定用 `module::toast` 作 target,调用方来源放进
/// 字段 `scope`(值是它的 target 字符串,如 `dozer::panel::agent`)。写**原始
/// 完整文本**(未折叠空白、未被固定卡片高度裁剪);归一化后为空的不写。
pub fn log_toast(scope: Scope, level: Level, text: &str) {
    if normalize(text).is_empty() {
        return;
    }
    let from = scope.target;
    match level {
        Level::Info | Level::Success => dozer_core::log_info!(LOG, scope = from, "{text}"),
        Level::Warning => dozer_core::log_warn!(LOG, scope = from, "{text}"),
        Level::Error => dozer_core::log_error!(LOG, scope = from, "{text}"),
    }
}

/// 后台任务(拿不到 `App`)里把一次失败变成可经 `proxy.send_event(Message::Toast(..))`
/// 发回主线程的 Toast 消息;`Ok` 返回 `None`(什么都不发)。文案是 `"{what}: {e}"`。
pub fn failure_message<T, E: std::fmt::Display>(
    scope: Scope,
    what: &str,
    res: &Result<T, E>,
) -> Option<Message> {
    res.as_ref().err().map(|e| Message::Push {
        scope,
        level: Level::Error,
        text: format!("{what}: {e}"),
        key: None,
    })
}

/// 一条待发的提示。
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub scope: Scope,
    pub level: Level,
    pub text: String,
    pub key: Option<String>,
}

/// extension state 里的待发提示。extension 的 `update` 拿不到 `App`,失败时往
/// 自己 state 的 `outbox` 里 `push`,`App::update` 的包装函数每次处理完消息后统一
/// 排空成 Toast。纯数据,可单测;extension 因此不依赖 `ToastCenter`/`App`。
#[derive(Debug, Default)]
pub struct Outbox {
    items: Vec<Pending>,
}

impl Outbox {
    pub fn push(&mut self, scope: Scope, level: Level, text: impl Into<String>) {
        self.items.push(Pending {
            scope,
            level,
            text: text.into(),
            key: None,
        });
    }

    /// 带去重键的版本:同 key 再推只刷新文本与计时(语义同 `App::push_toast_keyed`)。
    pub fn push_keyed(
        &mut self,
        scope: Scope,
        level: Level,
        text: impl Into<String>,
        key: impl Into<String>,
    ) {
        self.items.push(Pending {
            scope,
            level,
            text: text.into(),
            key: Some(key.into()),
        });
    }

    /// `Err(e)` 时推一条 `Error` 级 `"{what}: {e}"`;`Ok` 什么都不做。
    pub fn push_err<T, E: std::fmt::Display>(
        &mut self,
        scope: Scope,
        what: &str,
        res: &Result<T, E>,
    ) {
        if let Err(e) = res {
            self.push(scope, Level::Error, format!("{what}: {e}"));
        }
    }

    pub fn take(&mut self) -> Vec<Pending> {
        std::mem::take(&mut self.items)
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

    dozer_core::scope!(TEST_TODO, panel, "todo");

    #[test]
    fn update_push_message_delegates_to_push_and_carries_scope() {
        let (mut c, now) = center();
        update(
            &mut c,
            Message::Push {
                scope: TEST_TODO,
                level: Level::Success,
                text: "ok".into(),
                key: None,
            },
            now,
        );
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].level, Level::Success);
    }

    #[test]
    fn outbox_take_drains_and_preserves_order() {
        let mut o = Outbox::default();
        assert!(o.take().is_empty());
        o.push(TEST_TODO, Level::Error, "a");
        o.push(TEST_TODO, Level::Warning, "b");
        let got = o.take();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "a");
        assert_eq!(got[0].key, None);
        assert_eq!(got[1].text, "b");
        assert_eq!(got[1].level, Level::Warning);
        assert_eq!(got[1].scope, TEST_TODO);
        assert!(o.take().is_empty(), "取走后应为空");
    }

    #[test]
    fn outbox_push_keyed_carries_the_dedupe_key() {
        let mut o = Outbox::default();
        o.push_keyed(TEST_TODO, Level::Error, "停止失败", "apps:act:x");
        let got = o.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].key.as_deref(), Some("apps:act:x"));
        assert_eq!(got[0].text, "停止失败");
        assert_eq!(got[0].level, Level::Error);
    }

    #[test]
    fn outbox_push_err_pushes_only_on_err_with_what_prefix() {
        let mut o = Outbox::default();
        o.push_err(TEST_TODO, "保存失败", &Ok::<(), String>(()));
        assert!(o.take().is_empty(), "Ok 不应推提示");
        o.push_err(TEST_TODO, "保存失败", &Err::<(), _>("磁盘满"));
        let got = o.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, Level::Error);
        assert_eq!(got[0].text, "保存失败: 磁盘满");
    }

    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    fn capture(f: impl FnOnce()) -> String {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(sub, f);
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn log_toast_maps_levels_and_records_caller_scope_in_a_field() {
        let cases = [
            (Level::Info, "INFO"),
            (Level::Success, "INFO"),
            (Level::Warning, "WARN"),
            (Level::Error, "ERROR"),
        ];
        for (level, label) in cases {
            let out = capture(|| log_toast(TEST_TODO, level, "hello"));
            assert!(out.contains(label), "{level:?}: {out}");
            assert!(out.contains("dozer::module::toast"), "{out}");
            assert!(out.contains("scope=\"dozer::panel::todo\""), "{out}");
            assert!(out.contains("hello"), "{out}");
        }
    }

    #[test]
    fn log_toast_keeps_full_multiline_text_and_skips_blank() {
        let out = capture(|| log_toast(TEST_TODO, Level::Error, "line1\nline2"));
        assert!(out.contains("line1\nline2"), "{out}");
        let out = capture(|| log_toast(TEST_TODO, Level::Error, "  \n "));
        assert!(out.trim().is_empty(), "{out}");
    }

    #[test]
    fn failure_message_is_none_on_ok() {
        assert!(failure_message(TEST_TODO, "处理任务失败", &Ok::<(), String>(())).is_none());
    }

    #[test]
    fn failure_message_builds_an_error_push_with_what_prefix() {
        let msg = failure_message(TEST_TODO, "处理任务失败", &Err::<(), _>("daemon 超时"))
            .expect("Err 应产生消息");
        let Message::Push {
            scope,
            level,
            text,
            key,
        } = msg;
        assert_eq!(scope, TEST_TODO);
        assert_eq!(level, Level::Error);
        assert_eq!(text, "处理任务失败: daemon 超时");
        assert_eq!(key, None);
    }
}
