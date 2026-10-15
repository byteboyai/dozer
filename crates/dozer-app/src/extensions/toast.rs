//! 统一消息 Toast 的纯逻辑层:去重、堆叠上限、按级别到期。时间由调用方
//! 注入(`now`),不在内部取 `Instant::now()`,便于单测。App 级状态(挂
//! `App.toast`,不挂 `Workspace`——跨所有项目页签共享)。设计见
//! `docs/superpowers/specs/2026-09-30-unified-toast-design.md`。

use std::time::{Duration, Instant};

use dozer_core::log::Scope;
use dozer_model::LocalizedText;

dozer_core::scope!(LOG, module, "toast");

/// 同屏最多显示的 Toast 条数;超出时挤掉最旧的(瞬时消息过期即无意义,不排队)。
pub const MAX_VISIBLE: usize = 3;

/// 读失败类提示的冷却:轮询会每秒重试一次,持续性失败若每次都弹,Toast 会一直被续期且每次写日志;
/// 相同 key 下相同文本在这段时间内只提示一次(见 [`Outbox::push_keyed_cooldown`])。
pub const LOAD_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

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
    /// 文案存为 `LocalizedText`(owner = `dozer.host`),渲染时才按当前 locale
    /// 解析——因此切换语言只需重绘,不新增通知、不改 id/去重键/到期时间(spec §4.3)。
    pub text: LocalizedText,
    /// 去重键:同 key 再推 = 刷新文本/级别/计时,不新增。
    pub key: Option<String>,
    pub expires: Instant,
}

impl Toast {
    /// 语言无关的稳定渲染值(英文 fallback + 已插值参数、折叠空白),供测试与
    /// 去重比較,与界面显示一致。
    #[cfg(test)]
    pub fn text_stable(&self) -> String {
        normalize(&crate::i18n::host_render_stable(&self.text))
    }
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
        text: LocalizedText,
        key: Option<String>,
    },
}

pub fn update(state: &mut ToastCenter, msg: Message, now: Instant) {
    match msg {
        Message::Push {
            level, text, key, ..
        } => {
            state.push(level, text, key, now);
        }
    }
}

/// 每条 Toast 自动写一条日志。`tracing` 的 `target:` 必须是常量,而这里的
/// `scope` 是运行时形参,所以固定用 `module::toast` 作 target,调用方来源放进
/// 字段 `scope`(值是它的 target 字符串,如 `dozer::panel::agent`)。
///
/// 日志写**语言无关的稳定值**:Host 兜底文案(英文 fallback)插值后的完整文本,
/// 不随当前 locale 变化(spec「日志记录稳定事件,渲染使用当前快照」)。写完整
/// 文本(未折叠空白、未被固定卡片高度裁剪);归一化后为空的不写。
pub fn log_toast(scope: Scope, level: Level, text: impl Into<LocalizedText>) {
    let text = text.into();
    let stable = stable_value(&text);
    if stable.is_empty() {
        return;
    }
    let from = scope.target;
    match level {
        Level::Info | Level::Success => dozer_core::log_info!(LOG, scope = from, "{stable}"),
        Level::Warning => dozer_core::log_warn!(LOG, scope = from, "{stable}"),
        Level::Error => dozer_core::log_error!(LOG, scope = from, "{stable}"),
    }
}

/// 后台任务(拿不到 `App`)里把一次失败变成可经 `proxy.send_event(Message::Toast(..))`
/// 发回主线程的 Toast 消息;`Ok` 返回 `None`(什么都不发)。
///
/// `key`/`fallback` 是**操作语义**的模板(如 `"todo.process_failed"` /
/// `"Failed to process task: {e}"`),渲染时按当前 locale 解析;`e` 是服务返回的
/// 原始诊断(原样保留、不翻译)。生产端拿不到 locale,故只存 keyed `LocalizedText`。
pub fn failure_message<T, E: std::fmt::Display>(
    scope: Scope,
    key: &str,
    fallback: &str,
    res: &Result<T, E>,
) -> Option<Message> {
    res.as_ref().err().map(|e| Message::Push {
        scope,
        level: Level::Error,
        text: LocalizedText::keyed(crate::i18n::HOST_OWNER, key, fallback)
            .with_arg("e", e.to_string()),
        key: None,
    })
}

/// 便捷构造一条 Host 归属、无 key 的文案(fallback 即字面文本)。
///
/// 供尚未迁移的调用点过渡使用:把裸字符串(含已本地化好的中文)作为 fallback
/// 包成 `LocalizedText`,渲染时直接显示。迁移到资源 key 的面板/宿主代码应改用
/// `LocalizedText::keyed(HOST_OWNER, key, fallback)` 或 `i18n::host_text*`。
pub fn literal(text: impl Into<String>) -> LocalizedText {
    LocalizedText::literal(crate::i18n::HOST_OWNER, text)
}

/// 一条待发的提示。
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub scope: Scope,
    pub level: Level,
    pub text: LocalizedText,
    pub key: Option<String>,
}

impl Pending {
    /// 语言无关的稳定渲染值(英文 fallback + 已插值参数、折叠空白),供测试与
    /// 去重比較,与界面显示一致。
    #[cfg(test)]
    pub fn text_stable(&self) -> String {
        normalize(&crate::i18n::host_render_stable(&self.text))
    }
}

/// extension state 里的待发提示。extension 的 `update` 拿不到 `App`,失败时往
/// 自己 state 的 `outbox` 里 `push`,`App::update` 的包装函数每次处理完消息后统一
/// 排空成 Toast。纯数据,可单测;extension 因此不依赖 `ToastCenter`/`App`。
#[derive(Debug, Default)]
pub struct Outbox {
    items: Vec<Pending>,
    /// 每个 key 上一次经 [`Outbox::push_keyed_cooldown`] 推出的文本与时刻。排空(`take`)不清它,
    /// 否则每帧排空一次就等于没有冷却。存语言无关的稳定值,locale 切换不影响冷却。
    cooldowns: std::collections::HashMap<String, (String, Instant)>,
}

impl Outbox {
    pub fn push(&mut self, scope: Scope, level: Level, text: impl Into<LocalizedText>) {
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
        text: impl Into<LocalizedText>,
        key: impl Into<String>,
    ) {
        self.items.push(Pending {
            scope,
            level,
            text: text.into(),
            key: Some(key.into()),
        });
    }

    /// 带冷却的 `push_keyed`:同一 key 下,与上一次推出的文本相同且距今不足
    /// [`LOAD_FAILURE_COOLDOWN`] 则不推;文本变了或已过冷却则推并更新记录。`now` 由调用方注入。
    /// 比较用语言无关的稳定值,切换 locale 后同一失败不会重新弹。
    pub fn push_keyed_cooldown(
        &mut self,
        scope: Scope,
        level: Level,
        text: impl Into<LocalizedText>,
        key: impl Into<String>,
        now: Instant,
    ) {
        let text = text.into();
        let key = key.into();
        let stable = crate::i18n::host_render_stable(&text);
        let repeat = matches!(
            self.cooldowns.get(&key),
            Some((last, at)) if *last == stable && now.duration_since(*at) < LOAD_FAILURE_COOLDOWN
        );
        if repeat {
            return;
        }
        self.cooldowns.insert(key.clone(), (stable, now));
        self.push_keyed(scope, level, text, key);
    }

    pub fn take(&mut self) -> Vec<Pending> {
        std::mem::take(&mut self.items)
    }
}

/// 共享面板库的中性提示 → Dozer 的待发提示:补上日志来源 `scope`,级别一一对应。
pub fn pending_from_notice(scope: Scope, n: dozer_panel_host::Notice) -> Pending {
    use dozer_panel_host::NoticeLevel;
    let level = match n.level {
        NoticeLevel::Info => Level::Info,
        NoticeLevel::Success => Level::Success,
        NoticeLevel::Warning => Level::Warning,
        NoticeLevel::Error => Level::Error,
    };
    Pending { scope, level, text: n.text, key: n.key }
}

/// 折叠所有空白(含换行)成单个空格并去首尾:Toast 是固定高度的单/双行卡片,
/// 多行文本会撑坏几何。
pub(crate) fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Toast 的空判定/去重用的稳定值:无 key 的裸字符串(literal)若 fallback
/// 为空白,视为"没有内容"返回空——避免经 `host_render_stable` 被安全兜底文案
/// (`Something went wrong`)顶替成一条有内容的提示。有 key 的文案仍走完整解析。
fn stable_value(text: &LocalizedText) -> String {
    if text.key.is_none() && text.fallback.trim().is_empty() {
        return String::new();
    }
    crate::i18n::host_render_stable(text)
}

impl ToastCenter {
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// 推一条。返回条目 id;文本归一化后为空则忽略返回 `None`。空判定与无 key
    /// 去重都基于语言无关的稳定值(`host_render_stable`),保证 locale 切换不改变
    /// 去重行为。存入的 `LocalizedText` 保留原文案,渲染时才按 locale 解析。
    pub fn push(
        &mut self,
        level: Level,
        text: impl Into<LocalizedText>,
        key: Option<String>,
        now: Instant,
    ) -> Option<u64> {
        let text = text.into();
        let stable = normalize(&stable_value(&text));
        if stable.is_empty() {
            return None;
        }
        let expires = now + level.duration();
        let existing = self.items.iter_mut().find(|t| match (&key, &t.key) {
            (Some(k), Some(tk)) => k == tk,
            (None, None) => t.level == level && normalize(&stable_value(&t.text)) == stable,
            _ => false,
        });
        if let Some(t) = existing {
            t.level = level;
            t.text = text.clone();
            t.expires = expires;
            return Some(t.id);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Toast {
            id,
            level,
            text: text.clone(),
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

    #[test]
    fn a_neutral_notice_becomes_a_pending_toast_with_the_scope_added() {
        use dozer_panel_host::{Notice, NoticeLevel};
        let text = dozer_model::LocalizedText::keyed("dozer.todo", "k", "fallback");
        let n = |level| Notice { level, text: text.clone(), key: Some("x".into()) };
        let p = pending_from_notice(crate::extensions::todo::LOG, n(NoticeLevel::Error));
        assert_eq!((p.level, p.key.as_deref()), (Level::Error, Some("x")));
        assert_eq!(pending_from_notice(crate::extensions::todo::LOG, n(NoticeLevel::Warning)).level, Level::Warning);
    }

    fn center() -> (ToastCenter, Instant) {
        (ToastCenter::default(), Instant::now())
    }

    fn stable(t: &LocalizedText) -> String {
        super::normalize(&crate::i18n::host_render_stable(t))
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
        assert_eq!(stable(&c.items()[0].text), "第一行 第二行 第三行");
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
        assert_eq!(stable(&c.items()[0].text), "new");
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
        let texts: Vec<_> = c.items().iter().map(|t| stable(&t.text)).collect();
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
        assert_eq!(stable(&c.items()[0].text), "long");
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
        assert_eq!(stable(&got[0].text), "a");
        assert_eq!(got[0].key, None);
        assert_eq!(stable(&got[1].text), "b");
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
        assert_eq!(stable(&got[0].text), "停止失败");
        assert_eq!(got[0].level, Level::Error);
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
        assert!(
            failure_message(
                TEST_TODO,
                "todo.process_failed",
                "Failed to process task: {e}",
                &Ok::<(), String>(())
            )
            .is_none()
        );
    }

    #[test]
    fn failure_message_builds_an_error_push_with_operation_key() {
        let msg = failure_message(
            TEST_TODO,
            "todo.process_failed",
            "Failed to process task: {e}",
            &Err::<(), _>("daemon 超时"),
        )
        .expect("Err 应产生消息");
        let Message::Push {
            scope,
            level,
            text,
            key,
        } = msg;
        assert_eq!(scope, TEST_TODO);
        assert_eq!(level, Level::Error);
        assert_eq!(
            crate::i18n::host_render_stable(&text),
            "Failed to process task: daemon 超时"
        );
        assert_eq!(key, None);
    }

    // ---- Outbox::push_keyed_cooldown ----

    dozer_core::scope!(TEST_SCOPE, module, "toast_test");

    fn cooldown_push(out: &mut Outbox, key: &str, text: &str, now: Instant) {
        out.push_keyed_cooldown(TEST_SCOPE, Level::Error, text, key, now);
    }

    #[test]
    fn the_same_text_under_one_key_is_pushed_once_within_the_cooldown() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        cooldown_push(&mut out, "k", "加载失败: boom", t0);
        cooldown_push(&mut out, "k", "加载失败: boom", t0 + Duration::from_secs(1));
        cooldown_push(
            &mut out,
            "k",
            "加载失败: boom",
            t0 + LOAD_FAILURE_COOLDOWN - Duration::from_millis(1),
        );
        let pending = out.take();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].key.as_deref(), Some("k"));
        assert_eq!(pending[0].level, Level::Error);
        assert_eq!(stable(&pending[0].text), "加载失败: boom");
    }

    #[test]
    fn a_different_text_is_pushed_again_immediately() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        cooldown_push(&mut out, "k", "加载失败: a", t0);
        cooldown_push(&mut out, "k", "加载失败: b", t0 + Duration::from_secs(1));
        assert_eq!(out.take().len(), 2);
    }

    #[test]
    fn the_same_text_is_pushed_again_once_the_cooldown_has_passed() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        cooldown_push(&mut out, "k", "加载失败: boom", t0);
        cooldown_push(&mut out, "k", "加载失败: boom", t0 + LOAD_FAILURE_COOLDOWN);
        assert_eq!(out.take().len(), 2);
    }

    #[test]
    fn different_keys_do_not_share_a_cooldown() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        cooldown_push(&mut out, "a", "加载失败: boom", t0);
        cooldown_push(&mut out, "b", "加载失败: boom", t0);
        assert_eq!(out.take().len(), 2);
    }

    #[test]
    fn draining_the_outbox_keeps_the_cooldown_memory() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        cooldown_push(&mut out, "k", "加载失败: boom", t0);
        assert_eq!(out.take().len(), 1);
        cooldown_push(&mut out, "k", "加载失败: boom", t0 + Duration::from_secs(1));
        assert!(
            out.take().is_empty(),
            "排空后冷却记录仍在:轮询每秒失败一次也不重复提示"
        );
    }
}
