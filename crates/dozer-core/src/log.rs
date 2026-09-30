//! 统一日志服务。设计见 `docs/superpowers/specs/2026-09-30-unified-logging-design.md`。
//!
//! 分两层:本文件里**始终编译**的部分只用 std(供 `dozer-hook`/`dozer-mcp` 这类
//! 不能引入 `tracing` 的进程使用),`tracing` 后端在文件末尾的 `logging` feature 下。

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 日志组件(进程)。决定日志文件名前缀:`<前缀>.<UTC 日期>`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    App,
    Daemon,
    Hook,
    Mcp,
}

impl Component {
    pub const ALL: [Component; 4] = [
        Component::App,
        Component::Daemon,
        Component::Hook,
        Component::Mcp,
    ];

    pub const fn file_prefix(self) -> &'static str {
        match self {
            Component::App => "dozer-app.log",
            Component::Daemon => "dozerd.log",
            Component::Hook => "dozer-hook.log",
            Component::Mcp => "dozer-mcp.log",
        }
    }
}

/// 日志来源。`target` 格式固定为 `dozer::<kind>::<name>`,面板日志即
/// `dozer::panel::<面板名>`。用 `scope!` 声明,不要手写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    pub kind: &'static str,
    pub name: &'static str,
    pub target: &'static str,
}

/// 来源名字符集:非空,仅 `[a-z0-9_]`。`const fn`,供 `scope!` 在编译期断言。
pub const fn is_valid_scope_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_';
        if !ok {
            return false;
        }
        i += 1;
    }
    true
}

/// 声明一个日志来源常量:`dozer_core::scope!(LOG, panel, "todo");`。
/// 名字不合法时编译失败(`const` 断言)。
#[macro_export]
macro_rules! scope {
    ($vis:vis $ident:ident, panel, $name:literal) => {
        $vis const $ident: $crate::log::Scope = {
            assert!(
                $crate::log::is_valid_scope_name($name),
                "log scope name must be non-empty [a-z0-9_]"
            );
            $crate::log::Scope {
                kind: "panel",
                name: $name,
                target: concat!("dozer::panel::", $name),
            }
        };
    };
    ($vis:vis $ident:ident, module, $name:literal) => {
        $vis const $ident: $crate::log::Scope = {
            assert!(
                $crate::log::is_valid_scope_name($name),
                "log scope name must be non-empty [a-z0-9_]"
            );
            $crate::log::Scope {
                kind: "module",
                name: $name,
                target: concat!("dozer::module::", $name),
            }
        };
    };
}

// ---- UTC 时间(不引入 chrono/time) ----

/// 自 1970-01-01 起的天数 → 公历 (年, 月, 日)。Howard Hinnant 的 civil_from_days。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2026-09-30`(UTC)。
pub fn date_string(unix_secs: i64) -> String {
    let (y, m, d) = civil_from_days(unix_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `2026-09-30T09:12:03.123Z`(UTC),与 `tracing` 默认时间戳同形。
pub fn format_utc(unix_secs: i64, millis: u32) -> String {
    let (y, m, d) = civil_from_days(unix_secs.div_euclid(86_400));
    let rem = unix_secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// ---- 纯 std 追加写(hook/mcp 用;不写 stderr/stdout) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainLevel {
    Error,
    Warn,
    Info,
}

impl PlainLevel {
    fn label(self) -> &'static str {
        match self {
            PlainLevel::Error => "ERROR",
            PlainLevel::Warn => "WARN",
            PlainLevel::Info => "INFO",
        }
    }
}

/// 追加一行到 `logs_dir()/<组件前缀>.<UTC 日期>`。任何失败都静默忽略——
/// 日志不能让 hook/mcp 本身失败。
pub fn plain_write(component: Component, scope: &Scope, level: PlainLevel, msg: &str) {
    plain_write_in(
        &crate::paths::logs_dir(),
        component,
        scope,
        level,
        msg,
        SystemTime::now(),
    );
}

/// `plain_write` 的可注入版本(目录与时间由调用方给,便于测试)。
///
/// 行格式与 `tracing` 后端一致:`<UTC 时间>  <级别右对齐 5 位> <target>: <消息>`,
/// 消息里的换行折成 `\n` 字面量以保证一条日志一行。用 `O_APPEND` 打开、单行一次
/// `write_all`:小于管道缓冲的单次追加是原子的,多个 hook 进程并发写不会交错。
pub fn plain_write_in(
    dir: &Path,
    component: Component,
    scope: &Scope,
    level: PlainLevel,
    msg: &str,
    now: SystemTime,
) {
    use std::io::Write;
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs() as i64;
    let line = format!(
        "{} {:>5} {}: {}\n",
        format_utc(secs, since.subsec_millis()),
        level.label(),
        scope.target,
        msg.replace('\n', "\\n")
    );
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join(format!("{}.{}", component.file_prefix(), date_string(secs)));
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
}

#[macro_export]
macro_rules! plain_error {
    ($component:expr, $scope:expr, $($arg:tt)+) => {
        $crate::log::plain_write($component, &$scope, $crate::log::PlainLevel::Error, &format!($($arg)+))
    };
}
#[macro_export]
macro_rules! plain_warn {
    ($component:expr, $scope:expr, $($arg:tt)+) => {
        $crate::log::plain_write($component, &$scope, $crate::log::PlainLevel::Warn, &format!($($arg)+))
    };
}
#[macro_export]
macro_rules! plain_info {
    ($component:expr, $scope:expr, $($arg:tt)+) => {
        $crate::log::plain_write($component, &$scope, $crate::log::PlainLevel::Info, &format!($($arg)+))
    };
}

// ---- 保留清理 ----

/// 只按天数保留,不设大小上限(spec 已决)。
pub const RETENTION_DAYS: u64 = 14;

/// 删除 `dir` 下"四种组件前缀 + `.` + 任意后缀"且修改时间早于 `keep_days` 天的
/// 文件,返回删除数。目录不存在/读不了/删不掉一律忽略。只匹配 `<前缀>.`,所以
/// `dozerd.logfile` 与无关文件不受影响。
pub fn prune_old_logs(dir: &Path, now: SystemTime, keep_days: u64) -> usize {
    let Some(cutoff) = now.checked_sub(Duration::from_secs(keep_days * 86_400)) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours = Component::ALL.iter().any(|c| {
            name.strip_prefix(c.file_prefix())
                .is_some_and(|rest| rest.starts_with('.'))
        });
        if !ours {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(mtime) = meta.modified() else { continue };
        if mtime < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

// ---- tracing 后端(`logging` feature) ----

#[cfg(feature = "logging")]
pub use tracing as __tracing;

#[cfg(feature = "logging")]
#[macro_export]
macro_rules! log_error {
    ($scope:expr, $($arg:tt)+) => {
        $crate::log::__tracing::error!(target: $scope.target, $($arg)+)
    };
}
#[cfg(feature = "logging")]
#[macro_export]
macro_rules! log_warn {
    ($scope:expr, $($arg:tt)+) => {
        $crate::log::__tracing::warn!(target: $scope.target, $($arg)+)
    };
}
#[cfg(feature = "logging")]
#[macro_export]
macro_rules! log_info {
    ($scope:expr, $($arg:tt)+) => {
        $crate::log::__tracing::info!(target: $scope.target, $($arg)+)
    };
}
#[cfg(feature = "logging")]
#[macro_export]
macro_rules! log_debug {
    ($scope:expr, $($arg:tt)+) => {
        $crate::log::__tracing::debug!(target: $scope.target, $($arg)+)
    };
}
#[cfg(feature = "logging")]
#[macro_export]
macro_rules! log_trace {
    ($scope:expr, $($arg:tt)+) => {
        $crate::log::__tracing::trace!(target: $scope.target, $($arg)+)
    };
}

#[cfg(feature = "logging")]
pub use backend::{Console, Guard, init, init_at};

#[cfg(feature = "logging")]
mod backend {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Console {
        None,
        Stdout,
        Stderr,
    }

    /// 持有 `tracing-appender` 后台落盘线程的存活凭证;提前 drop 会静默丢日志。
    /// 调用方在 `main` 里绑到 `_guard`,寿命覆盖整个进程。
    pub struct Guard {
        _worker: Option<tracing_appender::non_blocking::WorkerGuard>,
    }

    /// `Daemon` 沿用 dozerd 现状输出到 stdout,`App` 输出到 stderr。
    pub fn init(component: Component, version: &'static str) -> Guard {
        let console = match component {
            Component::Daemon => Console::Stdout,
            _ => Console::Stderr,
        };
        init_at(component, version, &crate::paths::logs_dir(), console)
    }

    /// 只接受 `App`/`Daemon`(走 tracing);`Hook`/`Mcp` 用 `plain_write`。
    /// 全局 subscriber 已被装过时(测试进程)`try_init` 失败,静默降级不 panic。
    pub fn init_at(
        component: Component,
        version: &'static str,
        dir: &Path,
        console: Console,
    ) -> Guard {
        assert!(
            matches!(component, Component::App | Component::Daemon),
            "log::init 只用于 App/Daemon;Hook/Mcp 用 plain_write"
        );
        let filter =
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
        let dir_ok = std::fs::create_dir_all(dir).is_ok();
        if dir_ok {
            prune_old_logs(dir, SystemTime::now(), RETENTION_DAYS);
        }
        let (file_layer, worker) = if dir_ok {
            let appender = tracing_appender::rolling::daily(dir, component.file_prefix());
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_writer(writer)
                        .with_ansi(false),
                ),
                Some(guard),
            )
        } else {
            (None, None)
        };
        let stdout_layer = (console == Console::Stdout).then(tracing_subscriber::fmt::layer);
        let stderr_layer = (console == Console::Stderr)
            .then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stdout_layer)
            .with(stderr_layer)
            .with(file_layer)
            .try_init();
        if !dir_ok {
            tracing::warn!(target: "dozer::module::log", ?dir, "创建日志目录失败,本次运行只输出到控制台");
        }
        // panic 信息落盘后再交给原 hook(GUI 崩溃时此前没有任何落盘记录)。
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(target: "dozer::module::panic", "{info}");
            previous(info);
        }));
        tracing::info!(
            target: "dozer::module::log",
            component = component.file_prefix(),
            version,
            pid = std::process::id(),
            "启动"
        );
        Guard { _worker: worker }
    }
}

#[cfg(all(test, feature = "logging"))]
mod backend_tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    crate::scope!(FILES, panel, "files");
    crate::scope!(TODO, panel, "todo");

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    fn capture(filter: &str, f: impl FnOnce()) -> String {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .finish();
        tracing::subscriber::with_default(sub, f);
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn log_line_carries_the_scope_target_and_structured_fields() {
        let out = capture("info", || {
            crate::log_warn!(TODO, project_id = 7, "写失败 {}", "boom");
        });
        assert!(out.contains("WARN"), "{out}");
        assert!(out.contains("dozer::panel::todo"), "{out}");
        assert!(out.contains("写失败 boom"), "{out}");
        assert!(out.contains("project_id=7"), "{out}");
    }

    #[test]
    fn filtering_by_scope_target_enables_only_that_panel() {
        let out = capture("info,dozer::panel::files=debug", || {
            crate::log_debug!(FILES, "files-debug");
            crate::log_debug!(TODO, "todo-debug");
            crate::log_info!(TODO, "todo-info");
        });
        assert!(out.contains("files-debug"), "{out}");
        assert!(!out.contains("todo-debug"), "{out}");
        assert!(out.contains("todo-info"), "{out}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    crate::scope!(TEST_PANEL, panel, "todo");
    crate::scope!(TEST_MODULE, module, "hook");

    #[test]
    fn scope_macro_builds_target_from_kind_and_name() {
        assert_eq!(TEST_PANEL.target, "dozer::panel::todo");
        assert_eq!(TEST_PANEL.kind, "panel");
        assert_eq!(TEST_PANEL.name, "todo");
        assert_eq!(TEST_MODULE.target, "dozer::module::hook");
    }

    #[test]
    fn scope_name_charset_is_lowercase_digits_underscore_and_nonempty() {
        assert!(is_valid_scope_name("files"));
        assert!(is_valid_scope_name("code_health"));
        assert!(is_valid_scope_name("a1_b2"));
        assert!(!is_valid_scope_name(""));
        assert!(!is_valid_scope_name("Files"));
        assert!(!is_valid_scope_name("code-health"));
        assert!(!is_valid_scope_name("a b"));
        assert!(!is_valid_scope_name("中文"));
    }

    #[test]
    fn component_prefixes_are_distinct_and_all_lists_every_component() {
        let mut seen = std::collections::HashSet::new();
        for c in Component::ALL {
            assert!(seen.insert(c.file_prefix()));
        }
        assert_eq!(seen.len(), 4);
        assert_eq!(Component::App.file_prefix(), "dozer-app.log");
        assert_eq!(Component::Daemon.file_prefix(), "dozerd.log");
    }

    #[test]
    fn format_utc_handles_epoch_leap_day_and_year_boundary() {
        assert_eq!(format_utc(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_utc(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert_eq!(format_utc(1_709_164_800, 5), "2024-02-29T00:00:00.005Z");
        assert_eq!(format_utc(1_767_225_599, 999), "2025-12-31T23:59:59.999Z");
        assert_eq!(format_utc(1_767_225_600, 0), "2026-01-01T00:00:00.000Z");
        assert_eq!(date_string(1_767_225_599), "2025-12-31");
        assert_eq!(date_string(1_767_225_600), "2026-01-01");
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn plain_write_appends_lines_with_level_and_target() {
        let dir = tempfile::tempdir().unwrap();
        plain_write_in(
            dir.path(),
            Component::Hook,
            &TEST_MODULE,
            PlainLevel::Warn,
            "first",
            at(1_767_225_600),
        );
        plain_write_in(
            dir.path(),
            Component::Hook,
            &TEST_MODULE,
            PlainLevel::Error,
            "second",
            at(1_767_225_601),
        );
        let path = dir.path().join("dozer-hook.log.2026-01-01");
        let text = fs::read_to_string(path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("2026-01-01T00:00:00.000Z  WARN dozer::module::hook: first"),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("ERROR dozer::module::hook: second"));
    }

    #[test]
    fn plain_write_keeps_one_line_per_message_even_with_newlines() {
        let dir = tempfile::tempdir().unwrap();
        plain_write_in(
            dir.path(),
            Component::Mcp,
            &TEST_MODULE,
            PlainLevel::Info,
            "a\nb",
            at(0),
        );
        let text = fs::read_to_string(dir.path().join("dozer-mcp.log.1970-01-01")).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("a\\nb"));
    }

    #[test]
    fn plain_write_creates_missing_dir_and_never_panics_when_unwritable() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/logs");
        plain_write_in(
            &nested,
            Component::Hook,
            &TEST_MODULE,
            PlainLevel::Info,
            "x",
            at(0),
        );
        assert!(nested.join("dozer-hook.log.1970-01-01").exists());
        // 目录位置被一个普通文件占着:创建目录必失败,只能静默返回。
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        plain_write_in(
            &blocker.join("logs"),
            Component::Hook,
            &TEST_MODULE,
            PlainLevel::Info,
            "x",
            at(0),
        );
    }

    #[test]
    fn concurrent_appends_keep_every_line_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let p = path.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        plain_write_in(
                            &p,
                            Component::Hook,
                            &TEST_MODULE,
                            PlainLevel::Info,
                            &format!("t{t}-i{i}-END"),
                            at(0),
                        );
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = fs::read_to_string(path.join("dozer-hook.log.1970-01-01")).unwrap();
        assert_eq!(text.lines().count(), 8 * 50);
        assert!(
            text.lines()
                .all(|l| l.ends_with("-END") && l.contains("dozer::module::hook: t"))
        );
    }

    fn touch_with_mtime(p: &Path, t: SystemTime) {
        fs::write(p, "x").unwrap();
        let f = fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(t).unwrap();
    }

    #[test]
    fn prune_removes_only_old_files_with_our_prefixes() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(100 * 86_400);
        let old = at(100 * 86_400 - 15 * 86_400);
        let recent = at(100 * 86_400 - 86_400);
        for c in Component::ALL {
            touch_with_mtime(
                &dir.path().join(format!("{}.2026-01-01", c.file_prefix())),
                old,
            );
        }
        touch_with_mtime(&dir.path().join("dozerd.log.2026-01-02"), recent);
        touch_with_mtime(&dir.path().join("unrelated.txt"), old);
        touch_with_mtime(&dir.path().join("dozerd.logfile"), old); // 前缀相同但不是 `<前缀>.<日期>`
        let removed = prune_old_logs(dir.path(), now, RETENTION_DAYS);
        assert_eq!(removed, 4);
        assert!(dir.path().join("dozerd.log.2026-01-02").exists());
        assert!(dir.path().join("unrelated.txt").exists());
        assert!(dir.path().join("dozerd.logfile").exists());
        assert!(!dir.path().join("dozerd.log.2026-01-01").exists());
    }

    #[test]
    fn prune_on_missing_dir_returns_zero() {
        assert_eq!(
            prune_old_logs(
                Path::new("/nonexistent/dozer-logs-xyz"),
                at(1_000_000_000),
                14
            ),
            0
        );
    }
}
