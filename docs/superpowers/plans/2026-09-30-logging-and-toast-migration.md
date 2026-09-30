# 统一日志服务 + Toast 迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地 `dozer-core` 统一日志服务(每条日志带来源,面板日志写明面板名),并把审计确认的"用户操作失败却毫无反馈"的点接入 Toast;Toast 每条自动写日志。

**Architecture:** `dozer-core::log` 分两层:始终编译的纯 std 层(`Scope`/`scope!`/`Component`/UTC 时间/追加写/保留清理/`plain_*!`)与 `logging` feature 后的 `tracing` 层(`init`/`log_*!` 宏)。来源通过 `tracing` 的 `target`(`dozer::panel::<名>`/`dozer::module::<名>`)写进日志行。Toast 在此基础上加 `Scope` 参数、自动写日志,并新增 `Outbox`(extension state 里的待发提示,由 `App::update` 包装函数统一排空),解决 extension 的 `update` 无法触达 App 的问题。

**Tech Stack:** Rust、`tracing`/`tracing-subscriber`/`tracing-appender`(已是 workspace 依赖)、iced 0.14、winit 0.30。

**Specs:**
- `docs/superpowers/specs/2026-09-30-unified-logging-design.md`
- `docs/superpowers/specs/2026-09-30-unified-toast-design.md`
- `docs/superpowers/specs/2026-09-30-error-feedback-audit.md`

## 第一份计划(`2026-09-30-unified-toast.md`)的审核结论

在 `feat/unified-toast`(基于当时 main 头 `1a75909c`,6 个提交)上审核:

- `cargo build -p dozer-app` 通过;`cargo test -p dozer-app` 1491 通过 / 1 失败;`cargo fmt --check` 干净。
- **唯一失败** `extensions::files::tests::delete_confirm_spec_reflects_pending_target`(期望 `"报告.pdf"`,实际 `"/tmp/报告.pdf"`)在 **main 上同样失败**,不是该分支引入,与本计划无关,不要顺手改。
- 该分支比 main 多 **3 个 dead_code 警告**:`Message::Toast`、`toast::Message::Push`、`Level::Info/Success` "never constructed"。这是预期的(当时没有调用点使用),本计划 Task 3/6/7 起会有真实使用;若某个 Task 完成后它们仍在,说明有迁移点漏接。
- 代码与计划一致:Toast 窗口点击穿透+不聚焦(`with_active(false)` + `set_cursor_hittest(false)`)、`sync_toast_overlay` 在 `about_to_wait` 驱动、`about_to_wait` 唤醒数组 6→7、`agent_context.notice` 改 `take_notice()` 取走后推 Toast、`daemon_error` 的两处一次性事件改推 Toast、`CLAUDE.md` 已加 Toast 约定。
- **我无法验证的**:计划里 Task 3 Step 9 的手工 GUI 验收 A–H(焦点不被抢、叠在 webview 之上、与模态并存、到期销毁、缩放、长文本裁剪、中文渲染、条数上限)。这些需要真实运行,本计划**不假设它们已通过**——见下方 Task 0 的前置检查。
- 合并注意:该分支提交了 `docs/superpowers/specs/2026-09-30-unified-toast-design.md`,而主 checkout 里这个文件目前是**未跟踪**的,直接合并会因"未跟踪文件将被覆盖"失败;计划文件也仍是未跟踪状态。

## 审计文档的一处更正(影响 Task 9)

审计把 `tree_error` 判为"文件树加载/读取失败(面板内容状态),保留",**这是错的**。读 `extensions/files/update.rs` 后确认:`tree_error` 的全部写入点都是**文件操作反馈**——粘贴失败、回滚失败、新建/重命名/删除失败(`OpDone`)、拖拽移动被拒/失败、移动对话框校验、行内重命名/新建校验——不存在"加载失败"用法。因此它整体属于"用户刚做了一个操作、操作被拒或失败"。渲染点也是两处:面板头(`view.rs:223`)与另一处(`view.rs:1466`,疑为移动对话框正文,Task 9 第一步核实)。Task 9 据此重新设计。

## Global Constraints

- **分支**:独立 worktree/分支 `feat/logging-and-toast-migration`,从 main 建(**Task 0 通过后**),完成后审阅再合并;dispatch 子代理时必须带完整 worktree 绝对路径前缀。
- **不改行为**:迁移日志/接入 Toast 时**不得改动非日志逻辑**;遇到删日志语句导致变量未使用(如 `hovered`),只清理该变量的日志用途,不改控制流。
- **来源必须**:`dozerd`、`dozer-app` 里不得再出现裸 `tracing::{error,warn,info,debug,trace}!` 与 `eprintln!`(Task 11 用脚本门禁);一律 `dozer_core::log_*!(LOG, ...)`,`LOG` 是该文件顶部 `dozer_core::scope!` 声明的常量。
- **`tracing` 的 `target:` 必须是常量**:所以运行时才知道的来源(如 Toast 传入的 `Scope` 形参)**不能**当 target;Toast 的日志用固定来源 `module::toast` 并把调用方来源放进字段 `scope = <target 字符串>`(见 Task 3)。
- **级别**按 `2026-09-30-unified-logging-design.md` §7:`error` 用户操作失败/数据未落盘;`warn` 已降级仍可用;`info` 生命周期;`debug` 诊断;`trace` 逐事件。不要把逐帧/逐击键路径的失败加成日志或 Toast。
- **字体/主题**规则同第一份计划(Toast 视图已完成,本计划不动视图)。
- **extension 间只能消息通信**:extension 不得持有 `ToastCenter`;允许持有 `toast::Outbox`(纯数据类型)。
- 每个 Task 结束:`cargo build`(涉及 dozerd/hook/mcp 的 Task 用全 workspace)、`cargo test -p <受影响 crate>`、`cargo clippy -p <crate> --all-targets` 无新增告警、`cargo fmt` 干净。提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。commit 前 `git status`/`git diff --cached` 看 staged 全貌。
- 与第一份计划遗留的 1 个已知失败测试(见上)区分:对比基线时以"main 上同样失败"为准。

## 范围外(不擅自做)

- 审计表 C1 #9(agent 触发的预览失败是否让用户看到)与 #10(会话列表查询失败的面板空态)——产品立场未定,保持日志。
- `workspace/state.rs::send_input` 的写终端失败(逐击键路径):daemon 不通时每个按键都会失败,直接接 Toast+日志会刷屏,需要单独设计限频;本计划只接两个一次性的启动写(Task 8)。
- `project.error`(改名/保存失败,内联在面板顶部):审计定为可选,倾向保留。
- 日志查看器、JSON 输出、大小上限(spec 已决不做)。
- `push_toast` 之外的面板(Todo/SSH/Database/Files)的其它 `Option<String>` 内联错误字段:保留。

## Review Focus

- `scope!` 传入非法名字(大写、连字符、空串)→ 必须编译失败或被 const 断言拦下,不能悄悄产生不可过滤的 target。(Task 1)
- 日志目录不存在/不可写 → 任何写日志入口都不得 panic,也不得让 hook/mcp 本身失败。(Task 1/2)
- 保留清理只删自己的日志文件(四种前缀 + `.` + 日期),不得误删同目录其它文件;15 天前删、1 天前留。(Task 1)
- 多个 hook 进程并发追加同一文件 → 每行完整,不交错、不丢行。(Task 1)
- `init` 被多次调用(测试进程内)不得 panic。(Task 2)
- 同一故障高频重复 → Toast 去重(只显示一条),日志每次都写(历史);同时不给逐击键/逐帧路径加 Toast。(Task 3;`send_input` 见范围外)
- Outbox 漏排空:`App::update` 的某条分支提前 `return` 时仍必须排空。(Task 3:排空在包装函数里,不在 `match` 内)
- 迁移后某个文件仍保留裸 `tracing::`/`eprintln!` → 门禁脚本必须能发现。(Task 11)
- 拆 `tree_error` 后:移动对话框内的校验错误仍在对话框里显示且不残留;操作失败走 Toast;不再有面板头常驻红字。(Task 9)
- 三个 `dozer-app` 里手写 `panel = ?panel` 字段的日志,迁移后字段仍在(来源标"代码属于谁",字段标"替谁工作")。(Task 5)

---

### Task 0: 前置检查(不写代码)

- [ ] **Step 1: 确认第一份计划已合并且人工验收已通过**

向用户确认两件事,**未确认不要开工**:
1. `feat/unified-toast` 已合并进 main(`git log main --oneline | rg "toast"` 能看到 6 个 toast 提交)。合并时需先处理主 checkout 里未跟踪的 `docs/superpowers/specs/2026-09-30-unified-toast-design.md`(移走或先提交同内容再合并),否则合并会失败。
2. Task 3 Step 9 的手工 GUI 验收 A–H 是否已由人执行并通过。任何一项未通过,先回到第一份计划修复,本计划的 Toast 迁移建立在"Toast 窗口不抢焦点、能显示"之上。

- [ ] **Step 2: 建 worktree 并确认基线**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-logging -b feat/logging-and-toast-migration main
cd ../dozer-logging
cargo build 2>&1 | tail -3
cargo test -p dozer-app 2>&1 | rg "test result|FAILED"
```

Expected:`Finished`;测试只有 `delete_confirm_spec_reflects_pending_target` 一个失败(已知基线)。若有其它失败,停下汇报。后续命令都在 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-logging` 下执行。

---

### Task 1: `dozer-core::log` 纯 std 层

**Files:**
- Modify: `crates/dozer-core/Cargo.toml`(feature 与可选依赖、dev-dependencies)
- Modify: `crates/dozer-core/src/lib.rs`(`pub mod log;`)
- Create: `crates/dozer-core/src/log.rs`

**Interfaces:**
- Consumes: `crate::paths::logs_dir()`。
- Produces(后续 Task 依赖的精确签名):
  - `pub enum Component { App, Daemon, Hook, Mcp }`(`Debug, Clone, Copy, PartialEq, Eq`),`Component::file_prefix(self) -> &'static str`(`"dozer-app.log"`/`"dozerd.log"`/`"dozer-hook.log"`/`"dozer-mcp.log"`),`Component::ALL: [Component; 4]`
  - `pub struct Scope { pub kind: &'static str, pub name: &'static str, pub target: &'static str }`(`Debug, Clone, Copy, PartialEq, Eq`)
  - `pub const fn is_valid_scope_name(name: &str) -> bool`
  - `scope!` 宏:`dozer_core::scope!([pub(crate)] LOG, panel|module, "name");`
  - `pub enum PlainLevel { Error, Warn, Info }`
  - `pub fn plain_write(component: Component, scope: &Scope, level: PlainLevel, msg: &str)`
  - `pub fn plain_write_in(dir: &Path, component: Component, scope: &Scope, level: PlainLevel, msg: &str, now: SystemTime)`
  - `plain_error!/plain_warn!/plain_info!(component, scope, "格式串", args...)`
  - `pub const RETENTION_DAYS: u64 = 14`,`pub fn prune_old_logs(dir: &Path, now: SystemTime, keep_days: u64) -> usize`
  - `pub fn format_utc(unix_secs: i64, millis: u32) -> String`(`2026-09-30T09:12:03.123Z`),`pub fn date_string(unix_secs: i64) -> String`(`2026-09-30`)

- [ ] **Step 1: 依赖与模块声明**

`crates/dozer-core/Cargo.toml` 增加:

```toml
[features]
logging = ["dep:tracing", "dep:tracing-subscriber", "dep:tracing-appender"]

[dependencies]
# (已有的 anyhow/directories/serde/serde_json 保持不动)
tracing = { workspace = true, optional = true }
tracing-subscriber = { workspace = true, optional = true }
tracing-appender = { workspace = true, optional = true }

[dev-dependencies]
tempfile = "3"
```

`lib.rs` 加 `pub mod log;`(与 `pub mod paths;` 并列)。

- [ ] **Step 2: 写失败的测试(建文件骨架)**

创建 `crates/dozer-core/src/log.rs`,先只放 `use` 与测试模块,不写实现:

```rust
//! 统一日志服务。设计见 `docs/superpowers/specs/2026-09-30-unified-logging-design.md`。
//!
//! 分两层:本文件里**始终编译**的部分只用 std(供 `dozer-hook`/`dozer-mcp` 这类
//! 不能引入 `tracing` 的进程使用),`tracing` 后端在文件末尾的 `logging` feature 下。

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        plain_write_in(dir.path(), Component::Hook, &TEST_MODULE, PlainLevel::Warn, "first", at(1_767_225_600));
        plain_write_in(dir.path(), Component::Hook, &TEST_MODULE, PlainLevel::Error, "second", at(1_767_225_601));
        let path = dir.path().join("dozer-hook.log.2026-01-01");
        let text = fs::read_to_string(path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("2026-01-01T00:00:00.000Z  WARN dozer::module::hook: first"));
        assert!(lines[1].contains("ERROR dozer::module::hook: second"));
    }

    #[test]
    fn plain_write_keeps_one_line_per_message_even_with_newlines() {
        let dir = tempfile::tempdir().unwrap();
        plain_write_in(dir.path(), Component::Mcp, &TEST_MODULE, PlainLevel::Info, "a\nb", at(0));
        let text = fs::read_to_string(dir.path().join("dozer-mcp.log.1970-01-01")).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("a\\nb"));
    }

    #[test]
    fn plain_write_creates_missing_dir_and_never_panics_when_unwritable() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/logs");
        plain_write_in(&nested, Component::Hook, &TEST_MODULE, PlainLevel::Info, "x", at(0));
        assert!(nested.join("dozer-hook.log.1970-01-01").exists());
        // 目录位置被一个普通文件占着:创建目录必失败,只能静默返回。
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        plain_write_in(&blocker.join("logs"), Component::Hook, &TEST_MODULE, PlainLevel::Info, "x", at(0));
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
                        plain_write_in(&p, Component::Hook, &TEST_MODULE, PlainLevel::Info, &format!("t{t}-i{i}-END"), at(0));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = fs::read_to_string(path.join("dozer-hook.log.1970-01-01")).unwrap();
        assert_eq!(text.lines().count(), 8 * 50);
        assert!(text.lines().all(|l| l.ends_with("-END") && l.contains("dozer::module::hook: t")));
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
            touch_with_mtime(&dir.path().join(format!("{}.2026-01-01", c.file_prefix())), old);
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
        assert_eq!(prune_old_logs(Path::new("/nonexistent/dozer-logs-xyz"), at(1_000_000_000), 14), 0);
    }
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p dozer-core log:: 2>&1 | tail -12`
Expected:编译失败,`cannot find macro scope` / `cannot find type Component` 等。

- [ ] **Step 4: 实现**

在 `log.rs` 的 `use` 与 `#[cfg(test)]` 之间插入:

```rust
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
        let ok = (c >= b'a' && c <= b'z') || (c >= b'0' && c <= b'9') || c == b'_';
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
```

- [ ] **Step 5: 运行确认通过**

Run: `cargo test -p dozer-core log:: 2>&1 | tail -15`
Expected:全部通过(10 个测试)。`plain_*!` 三个宏在本 Task 无调用点(hook 在 Task 10 才用),它们是 `#[macro_export]` 宏,不产生 dead_code 警告。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozer-core --all-targets 2>&1 | tail -5
git add crates/dozer-core/Cargo.toml crates/dozer-core/src/lib.rs crates/dozer-core/src/log.rs
git diff --cached --stat
git commit -m "feat(core): add log scope, plain appender, UTC time and retention pruning

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `tracing` 后端(`init`、`log_*!`)+ 两个 spike

**Files:**
- Modify: `crates/dozer-core/src/log.rs`(文件末尾追加 `logging` feature 下的后端)
- Create: `crates/dozer-core/tests/log_init.rs`
- Create(临时,Step 2 用完删除): `clippy.toml`

**Interfaces:**
- Consumes: Task 1 的全部产出。
- Produces:
  - `log_error!/log_warn!/log_info!/log_debug!/log_trace!(scope, [字段…,] "格式串", args…)`(仅 `logging` feature)
  - `pub use tracing as __tracing;`(宏内部引用,调用方不需要自己依赖 `tracing`)
  - `pub enum Console { None, Stdout, Stderr }`
  - `pub struct Guard`(持有落盘线程的存活凭证,调用方绑到 `_guard` 直到进程退出)
  - `pub fn init(component: Component, version: &'static str) -> Guard`(`App`→stderr,`Daemon`→stdout,与 dozerd 现状一致)
  - `pub fn init_at(component: Component, version: &'static str, dir: &Path, console: Console) -> Guard`

- [ ] **Step 1: spike A — `target:` 是否接受 `const` 结构体字段**

在 `log.rs` 末尾先只加宏与一个编译验证测试:

```rust
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
```

Run: `cargo build -p dozer-core --features logging 2>&1 | tail -10`,再在 `tests` 模块(`#[cfg(all(test, feature = "logging"))]`)里加一条使用 `crate::log_warn!(TEST_PANEL, "x {}", 1)` 的测试并 `cargo test -p dozer-core --features logging log::`。

- **通过**:继续 Step 3。
- **失败(`target` 不是常量表达式)**:**停下汇报**,把编译器报错原文贴给用户。由用户决定是否改用"字段 `scope = "todo"` + 固定 target"的方案(会失去 `RUST_LOG` 按面板过滤这个 spec 里的核心收益)。不要自行选择降级方案,也不要继续 Task 3 及之后的 Task——它们全部依赖这个机制。

- [ ] **Step 2: spike B — clippy `disallowed_macros` 对包装宏的展开是否误报**

在仓库根临时建 `clippy.toml`:

```toml
disallowed-macros = [
    { path = "tracing::error", reason = "use dozer_core::log_error!" },
    { path = "tracing::warn", reason = "use dozer_core::log_warn!" },
    { path = "tracing::info", reason = "use dozer_core::log_info!" },
    { path = "tracing::debug", reason = "use dozer_core::log_debug!" },
    { path = "tracing::trace", reason = "use dozer_core::log_trace!" },
]
```

在 `log.rs` 的测试里(如上)已有 `log_warn!(TEST_PANEL, …)` 调用。运行 `cargo clippy -p dozer-core --features logging --all-targets 2>&1 | rg "disallowed"`。

- **无命中**:clippy 门禁可用,Task 11 启用 `clippy.toml`。
- **有命中**(包装宏展开的 `tracing::warn!` 被判为违规,且无法在展开点局部豁免):**不启用 clippy 这一层**,Task 11 只保留兜底脚本作为门禁。

无论结果如何,**删除这个临时 `clippy.toml`**(Task 11 才正式加)。把两个 spike 的结果(通过/失败与现象)写进本 Task 的提交信息正文。

- [ ] **Step 3: 写失败的测试(后端)**

在 `log.rs` 末尾追加测试模块(`logging` feature 下)——按来源写日志行 + 按来源过滤:

```rust
#[cfg(all(test, feature = "logging"))]
mod backend_tests {
    use super::*;
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
        assert!(out.contains("WARN"));
        assert!(out.contains("dozer::panel::todo"));
        assert!(out.contains("写失败 boom"));
        assert!(out.contains("project_id=7"));
    }

    #[test]
    fn filtering_by_scope_target_enables_only_that_panel() {
        let out = capture("info,dozer::panel::files=debug", || {
            crate::log_debug!(FILES, "files-debug");
            crate::log_debug!(TODO, "todo-debug");
            crate::log_info!(TODO, "todo-info");
        });
        assert!(out.contains("files-debug"));
        assert!(!out.contains("todo-debug"));
        assert!(out.contains("todo-info"));
    }
}
```

创建 `crates/dozer-core/tests/log_init.rs`(独立进程,`init_at` 只装一次全局 subscriber):

```rust
#![cfg(feature = "logging")]

use dozer_core::log::{Component, Console, init_at};
use std::fs;

dozer_core::scope!(LOG, panel, "todo");

#[test]
fn init_at_writes_banner_and_scoped_lines_to_the_component_file() {
    let dir = tempfile::tempdir().unwrap();
    let guard = init_at(Component::App, "9.9.9", dir.path(), Console::None);
    dozer_core::log_warn!(LOG, "hello-from-todo");
    drop(guard); // 释放 WorkerGuard,把缓冲刷到文件

    let file = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("dozer-app.log."))
        .expect("应生成 dozer-app.log.<日期>");
    let text = fs::read_to_string(file.path()).unwrap();
    assert!(text.contains("9.9.9"), "缺启动横幅版本: {text}");
    assert!(text.contains("dozer::module::log"), "缺横幅来源: {text}");
    assert!(text.contains("dozer::panel::todo"), "缺面板来源: {text}");
    assert!(text.contains("hello-from-todo"));
}

#[test]
fn init_at_with_uncreatable_dir_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "x").unwrap();
    // 第二次 init:全局 subscriber 已装,必须静默降级而不是 panic。
    let _guard = init_at(Component::Daemon, "9.9.9", &blocker.join("logs"), Console::None);
}
```

(两个集成测试在同一进程顺序不保证;第二个只断言"不 panic",第一个断言内容——为避免全局 subscriber 竞争,第一个测试里的断言若被另一个测试抢先装了 subscriber 会失败。**若出现这种竞争,把两个测试合并成一个 `#[test]`,按"先正常目录、再不可创建目录"的顺序执行。**)

Run: `cargo test -p dozer-core --features logging 2>&1 | tail -15`
Expected:编译失败(`init_at`/`Console` 未定义)。

- [ ] **Step 4: 实现后端**

在 `log.rs` 中宏之后、`#[cfg(test)]` 之前追加:

```rust
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
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into());
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
        let stdout_layer =
            (console == Console::Stdout).then(tracing_subscriber::fmt::layer);
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
```

Run: `cargo test -p dozer-core --features logging 2>&1 | tail -15`
Expected:全部通过。另确认不带 feature 也能编译测试:`cargo test -p dozer-core 2>&1 | tail -5`(Task 1 的 10 个测试仍通过,后端测试不参与)。

- [ ] **Step 5: 提交**

```bash
cargo fmt && cargo clippy -p dozer-core --all-targets --features logging 2>&1 | tail -5
git status --short   # 确认临时 clippy.toml 已删除、不在 staged 里
git add crates/dozer-core/src/log.rs crates/dozer-core/tests/log_init.rs
git diff --cached --stat
git commit -m "feat(core): add tracing backend (init, log_* macros, panic hook)

spike A (target: const struct field): <通过/失败,写实际结果>
spike B (clippy disallowed_macros on wrapper macros): <无误报/有误报,写实际结果>

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Toast 接入来源与日志 + `Outbox`

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`(`dozer-core` 开 `logging`;`tracing-subscriber` 仍保留供测试)
- Modify: `crates/dozer-app/src/extensions/toast.rs`(`Scope`、`log_toast`、`Outbox`/`Pending`)
- Modify: `crates/dozer-app/src/app/app.rs`(`emit_toast`、新签名、`flush_outbox`)
- Modify: `crates/dozer-app/src/app/update.rs`(`update` 包装 + `update_inner`、`drain_outboxes`、`Message::Toast` 分支、两处现有调用点)
- Modify: `crates/dozer-app/src/extensions/agent_context.rs`(`notice` → `Outbox`)
- Modify: 各含 `outbox` 的 state 结构体(见 Step 5)

**Interfaces:**
- Consumes: Task 1/2;第一份计划的 `ToastCenter`/`Level`/`Message`/`update`/`push_toast`。
- Produces:
  - `toast::Message::Push { scope: Scope, level: Level, text: String, key: Option<String> }`
  - `toast::log_toast(scope: Scope, level: Level, text: &str)`
  - `toast::Pending { pub scope: Scope, pub level: Level, pub text: String, pub key: Option<String> }`
  - `toast::Outbox`(`Debug, Default`):`push(&mut self, scope: Scope, level: Level, text: impl Into<String>)`、`push_keyed(&mut self, scope, level, text: impl Into<String>, key: &str)`、`push_err<T, E: Display>(&mut self, scope: Scope, what: &str, res: &Result<T, E>)`、`take(&mut self) -> Vec<Pending>`、`is_empty(&self) -> bool`
  - `App::push_toast(&mut self, scope: Scope, level: toast::Level, text: impl AsRef<str>)`、`App::push_toast_keyed(&mut self, scope: Scope, level, text: impl AsRef<str>, key: &str)`(**旧的无 `Scope` 签名被替换,无兼容垫片**)
  - 每个持有 outbox 的 extension state 提供 `pub(crate) outbox: toast::Outbox` 字段与 `pub fn take_outbox(&mut self) -> Vec<toast::Pending>`

- [ ] **Step 1: 依赖**

`crates/dozer-app/Cargo.toml` 中 `dozer-core` 依赖改为带 feature:`dozer-core = { path = "../dozer-core", features = ["logging"] }`。`cargo build -p dozer-app` 应仍通过(此时还没有使用点)。

- [ ] **Step 2: 写失败的测试(toast.rs)**

在 `extensions/toast.rs` 的测试模块追加(并在文件顶部 `use dozer_core::log::Scope;` 及 `dozer_core::scope!(LOG, module, "toast");`——`LOG` 用于 `log_toast` 的固定来源):

```rust
    dozer_core::scope!(TEST_TODO, panel, "todo");

    #[test]
    fn outbox_take_drains_and_preserves_order() {
        let mut o = Outbox::default();
        assert!(o.is_empty());
        o.push(TEST_TODO, Level::Error, "a");
        o.push_keyed(TEST_TODO, Level::Warning, "b", "k");
        assert!(!o.is_empty());
        let got = o.take();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "a");
        assert_eq!(got[0].key, None);
        assert_eq!(got[1].key.as_deref(), Some("k"));
        assert_eq!(got[1].scope, TEST_TODO);
        assert!(o.is_empty());
        assert!(o.take().is_empty());
    }

    #[test]
    fn outbox_push_err_pushes_only_on_err_with_what_prefix() {
        let mut o = Outbox::default();
        o.push_err(TEST_TODO, "保存失败", &Ok::<(), String>(()));
        assert!(o.is_empty());
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
    fn update_push_message_still_delegates_and_carries_scope() {
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
    }
```

同时把已有测试 `update_push_message_delegates_to_push` 里 `Message::Push { level, text, key }` 的构造补上 `scope: TEST_TODO,`(或直接删除该旧测试,由上面的 `update_push_message_still_delegates_and_carries_scope` 取代)。

Run: `cargo test -p dozer-app toast:: 2>&1 | tail -12`
Expected:编译失败(`Outbox`/`log_toast`/`Scope` 字段未定义)。

- [ ] **Step 3: 实现 toast.rs 部分**

```rust
use dozer_core::log::Scope;

dozer_core::scope!(LOG, module, "toast");

// Message::Push 增加 scope
#[derive(Debug, Clone)]
pub enum Message {
    Push {
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
/// 完整文本**(未折叠空白、未被固定卡片高度裁剪);文本归一化后为空不写。
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

    pub fn push_keyed(
        &mut self,
        scope: Scope,
        level: Level,
        text: impl Into<String>,
        key: &str,
    ) {
        self.items.push(Pending {
            scope,
            level,
            text: text.into(),
            key: Some(key.to_string()),
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

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
```

Run: `cargo test -p dozer-app toast:: 2>&1 | tail -15`
Expected:toast 相关测试全部通过(第一份计划的 11 个 + 新增 5 个)。此时 `App` 侧还没接,`Message::Toast` 构造点(若旧代码构造了 `Message::Push` 需补 `scope`)——`cargo build -p dozer-app` 若报 `missing field scope`,按报错位置补(目前只有测试构造它)。

- [ ] **Step 4: `App` 接入(emit_toast、新签名、排空包装)**

`app/app.rs`:把第一份计划的 `push_toast`/`push_toast_keyed` 换成:

```rust
    /// 直接推一条 Toast 并写日志(`toast::log_toast` 是唯一写 Toast 日志的函数)。
    fn emit_toast(
        &mut self,
        scope: dozer_core::log::Scope,
        level: toast::Level,
        text: &str,
        key: Option<String>,
    ) {
        toast::log_toast(scope, level, text);
        self.toast
            .push(level, text, key, std::time::Instant::now());
    }

    pub fn push_toast(
        &mut self,
        scope: dozer_core::log::Scope,
        level: toast::Level,
        text: impl AsRef<str>,
    ) {
        self.emit_toast(scope, level, text.as_ref(), None);
    }

    pub fn push_toast_keyed(
        &mut self,
        scope: dozer_core::log::Scope,
        level: toast::Level,
        text: impl AsRef<str>,
        key: &str,
    ) {
        self.emit_toast(scope, level, text.as_ref(), Some(key.to_string()));
    }

    /// 把各 extension outbox 里排出来的待发提示推成 Toast。
    pub(crate) fn flush_outbox(&mut self, pending: Vec<toast::Pending>) {
        for p in pending {
            self.emit_toast(p.scope, p.level, &p.text, p.key);
        }
    }
```

`app/update.rs`:

1. 把现有 `pub fn update(&mut self, message: Message) { match message { ...` 重命名为 `fn update_inner(&mut self, message: Message)`,并在其前面加包装(其它文件对 `App::update` 的调用不变):

```rust
    pub fn update(&mut self, message: Message) {
        self.update_inner(message);
        // 放在包装里而不是 `match` 末尾:`update_inner` 的许多分支会提前 `return`。
        self.drain_outboxes();
    }

    /// 排空所有 extension state 的 outbox 并推成 Toast。遍历已加载工作区的
    /// 代价只是几个 `Vec::is_empty`,可以每条消息都做。
    fn drain_outboxes(&mut self) {
        let mut pending: Vec<toast::Pending> = Vec::new();
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                pending.extend(ws.files.take_outbox());
                pending.extend(ws.todo.take_outbox());
                pending.extend(ws.ssh.take_outbox());
                pending.extend(ws.database.take_outbox());
                pending.extend(ws.agent_context.take_outbox());
            }
        }
        pending.extend(self.database.take_outbox());
        if !pending.is_empty() {
            self.flush_outbox(pending);
        }
    }
```

(`WorkspaceSlot` 已在 `app/app.rs` 导入使用;update.rs 里若未导入按需 `use`。)

2. `Message::Toast` 分支改为先写日志再入队:

```rust
            Message::Toast(msg) => {
                let toast::Message::Push {
                    scope, level, ref text, ..
                } = msg;
                toast::log_toast(scope, level, text);
                toast::update(&mut self.toast, msg, std::time::Instant::now());
            }
```

3. 文件顶部(其它 `use` 之后)加 `dozer_core::scope!(LOG, module, "shell");`,并更新两处现有调用点(第一份计划迁移过的):

```rust
// project_tab_opened 失败路径
self.push_toast_keyed(
    LOG,
    toast::Level::Error,
    "打开项目失败,请确认 dozerd 正常后重试",
    "open-project-failed",
);
// ProjectDeleteDone
self.push_toast(
    LOG,
    toast::Level::Warning,
    format!("删除项目未完全成功: {}", errors.join("; ")),
);
```

同时删掉 `agent_context_message` 里第一份计划加的"取走 notice 再推 Toast"那段(`take_notice` 逻辑),恢复为原来的 `ctx::update(...)` 直接调用——`notice` 改由 outbox + `drain_outboxes` 承接(Step 5)。

- [ ] **Step 5: 给各 state 加 `outbox`,并把 `agent_context.notice` 改成 outbox**

对 `files`(`extensions/files/state.rs` 的 `WorkspaceState`)、`todo`、`ssh`、`database`(`extensions/database/state.rs` 的 `WorkspaceState` 与 `AppState` 各自需要则各加)、`agent_context::State`:

```rust
    pub(crate) outbox: crate::extensions::toast::Outbox,
```

构造处补 `outbox: Default::default(),`(或结构体已 `#[derive(Default)]` 则自动获得;逐个用 `rg -n "struct WorkspaceState|struct State|struct AppState"` 定位并按编译器提示补构造)。各 state 增加:

```rust
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }
```

`database` 的 `AppState`(app 级,`self.database`)与 `WorkspaceState`(`ws.database`)都需要 `take_outbox`(`drain_outboxes` 上面两处都调用了);若某个 state 实际不需要 outbox(Task 7 才用到 database/ssh/todo/files),**仍在本 Task 建好空字段与 `take_outbox`**,让 `drain_outboxes` 一次成型,后面的 Task 只往 outbox 里 `push`。

`agent_context.rs`:

- `State` 里 `notice: Option<String>` 换成 `outbox: toast::Outbox`,删除 `take_notice`,新增 `take_outbox`。
- 文件顶部 `dozer_core::scope!(pub(crate) LOG, panel, "agent");`。
- `apply` 里三处 `state.notice = Some(format!(...))` 改成:

```rust
        Message::Loaded(Err(e)) => {
            state.outbox.push(LOG, toast::Level::Error, format!("加载上下文列表失败: {e}"));
            Followup::None
        }
        // Removed(Err) → format!("移除失败: {e}"),Added(Err) → format!("已发送到终端,但未能记录到上下文列表: {e}")
```

(`Added(Err)` 改为 `toast::Level::Warning`:终端粘贴已经发生,只是记录失败——这是第一份计划因 notice 没有级别而统一成 Error 的地方,现在可以区分。)

- 现有测试里 `s.take_notice()` 的用法按下面规则机械替换(逐个测试看,同一状态不要连续两次 `take_outbox()` 期望都有值):
  - `s.take_notice().unwrap().contains("boom")` → `s.take_outbox()[0].text.contains("boom")`
  - `s.take_notice().is_none()` → `s.take_outbox().is_empty()`
  - 为 `Added(Err)` 那个测试补一句 `assert_eq!(got[0].level, toast::Level::Warning)`。
  - 第一份计划新增的 `take_notice_returns_once_then_none` 改名 `take_outbox_returns_once_then_empty`。
- 视图 `strip` 里不再有 notice(第一份计划已删),确认无残留引用:`rg -n "take_notice|\.notice\b" crates/dozer-app/src` 应为空。

- [ ] **Step 6: 构建与测试**

Run: `cargo build -p dozer-app 2>&1 | tail -5 && cargo test -p dozer-app 2>&1 | rg "test result|FAILED"`
Expected:通过;失败只有已知基线 `delete_confirm_spec_reflects_pending_target`。`Level::Info/Success`、`toast::Message::Push` 的 dead_code 警告**仍然存在是正常的**(它们要到 Task 6+ 才有真实构造点);`Message::Toast` 的警告同理。若出现**其它新增**警告,先修。

- [ ] **Step 7: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app
git diff --cached --stat
git commit -m "feat(toast): carry a log Scope, log every toast, add Outbox drained by App::update

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 迁移 `dozerd`(约 40 处 + 初始化)

**Files:**
- Modify: `crates/dozerd/Cargo.toml`(`dozer-core` 开 `logging`,去掉不再直接用的 `tracing-subscriber`/`tracing-appender`)
- Modify: `crates/dozerd/src/main.rs`(删 `init_logging`,改用 `dozer_core::log::init`)
- Modify: 含日志调用的模块(用 `rg` 取全量)

**Interfaces:** Consumes Task 1/2。Produces:无(dozerd 内部)。

- [ ] **Step 1: 盘点**

Run: `rg -n "tracing::(error|warn|info|debug|trace)!|eprintln!|^use tracing" crates/dozerd/src`
记下每个文件的命中数(审计时约:`server.rs` 15、`main.rs` 7、`ide_bridge.rs` 6、`default_agent_config.rs` 3、`summary_service.rs` 2、`session_summary_backfill.rs` 2、`transcripts/parse.rs` 2、`task_processor.rs` 1、`backfill.rs` 1、`session.rs` 1)。若有 `use tracing::{warn, ...}` 写法,后续要一并处理。

- [ ] **Step 2: 初始化换成 core 的**

`crates/dozerd/Cargo.toml`:`dozer-core = { path = "../dozer-core", features = ["logging"] }`;确认 `tracing` 依赖是否还被直接使用(下一步替换后可能不再需要),用 `cargo machete`/`cargo build` 的未使用警告判断,不用的删掉;`tracing-subscriber`、`tracing-appender` 去掉。

`main.rs`:删除 `init_logging` 函数及其 `use tracing_subscriber::…`,把 `let _guard = init_logging();` 换成:

```rust
    let _guard = dozer_core::log::init(dozer_core::log::Component::Daemon, env!("CARGO_PKG_VERSION"));
```

行为差异(有意):新增 14 天保留清理、来源写入 target、启动横幅;日志文件名仍为 `dozerd.log.<日期>`。

- [ ] **Step 3: 声明来源并迁移调用**

来源命名(`kind = module`,一个文件一个 `LOG` 常量,声明在该文件 `use` 之后):

| 文件 | 来源名 |
|---|---|
| `main.rs`、`server.rs` | `server` |
| `session.rs`、`registry.rs`、`ring.rs`、`shell_integration.rs` | `session` |
| `ide_bridge.rs` | `ide_bridge` |
| `summary_*.rs`、`backfill.rs`、`session_summary_backfill.rs` | `summary` |
| `default_agent_config.rs` | `agent_config` |
| `task_processor.rs`、`task_poller.rs` | `task` |
| `transcripts/*` | `transcripts` |
| 其余出现命中的文件 | 文件名(去 `.rs`,小写下划线) |

机械改写规则:每个有命中的文件,顶部加 `dozer_core::scope!(LOG, module, "<名>");`,并把

`tracing::warn!(` → `dozer_core::log_warn!(LOG, `(其余 `error/info/debug/trace` 同理;结构化字段与格式串原样保留,因为宏第一个参数之后透传)。可用:

```bash
perl -0pi -e 's/tracing::(error|warn|info|debug|trace)!\(/dozer_core::log_$1!(LOG, /g' <文件>
```

再 `cargo fmt`。**逐条重定级**(不是机械保持):按 spec §7,用户/agent 发起的操作失败或数据未落盘 → `error`;已自动降级 → `warn`;生命周期 → `info`。不改任何非日志代码。`use tracing::…` 残留的导入按编译器提示删除。

- [ ] **Step 4: 验证**

```bash
cargo build -p dozerd 2>&1 | tail -5
cargo test -p dozerd 2>&1 | rg "test result|FAILED"
rg -n "tracing::(error|warn|info|debug|trace)!|eprintln!" crates/dozerd/src   # 期望:无输出(core 之外不应再有)
```

Expected:构建/测试通过(与基线一致的失败集合),`rg` 无输出。

- [ ] **Step 5: 手工验证**

```bash
cargo run -p dozerd 2>&1 | head -5   # 看到启动横幅一行,含 dozer::module::log 与版本
```

日志目录是 `dozer_core::paths::logs_dir()`(即 `state_dir().join("logs")`,macOS 上在 `~/Library/Application Support/` 下 `ai.byteboy.dozer` 相关目录内,以 `directories` 的实际解析为准)。用 `find ~/Library -path "*dozer*" -name "dozerd.log.*" 2>/dev/null` 找到它,确认出现 `dozerd.log.<日期>`,内容含 `dozer::module::` 来源与启动横幅。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozerd --all-targets 2>&1 | tail -5
git add crates/dozerd
git diff --cached --stat
git commit -m "refactor(dozerd): use dozer-core log service with per-module scopes

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `dozer-app` 初始化 + 外壳/平台/预览/runtime/term 来源 + 删除遗留调试日志

**Files:**
- Modify: `crates/dozer-app/src/main.rs`(初始化)
- Modify: 含日志调用的非面板文件(按下表)
- Modify: `crates/dozer-app/Cargo.toml`(`tracing-subscriber` 若仅测试用则挪到 `[dev-dependencies]`)

**Interfaces:** Consumes Task 1/2。Produces:无。

- [ ] **Step 1: 初始化**

`main.rs`:把 `tracing_subscriber::fmt::init();` 换成

```rust
    let _log_guard = dozer_core::log::init(dozer_core::log::Component::App, env!("CARGO_PKG_VERSION"));
```

`_log_guard` 必须活到进程结束(`main` 在事件循环 `run_app` 返回后才退出,绑在 `main` 函数体顶部即可)。

- [ ] **Step 2: 删除遗留 `[DIAG]`/`DEBUG` 调试日志(已决:直接删除)**

Run: `rg -n "\[DIAG\]|DEBUG " crates/dozer-app/src`
审计时定位到约 4 处:`platform/window.rs` 的 `[DIAG] mouseDownCanMoveWindow called…`、`platform/window_events.rs` 的 `[DIAG] TOPBAR_CONTROL_HOVERED changed…`、`[DIAG] left MouseInput state=…`、`DEBUG term input fallback fired`。**逐条先读上下文**再删:它们应是纯输出语句;删除后若某个局部变量(如 `hovered`、`mouse_interaction`)只为这条日志存在,则一并删掉该变量,**不改控制流**。全部删完后 `rg -n "\[DIAG\]|DEBUG "` 应无命中。

- [ ] **Step 3: 声明来源并迁移调用**

`dozer-app` 的非面板来源(`kind = module`,每个含日志的文件顶部 `dozer_core::scope!(LOG, module, "<名>");`;同一文件里若需要两个来源,第二个用不同常量名,如 `PREVIEW_LOG`):

| 路径 | 来源名 |
|---|---|
| `app/*`、`workspace/*`、`layout.rs`、`keymap.rs`、`capabilities.rs`、`git_watch.rs`、`open_projects.rs`、`assets/fonts.rs` | `shell` |
| `platform/*`、`chrome/*` | `platform` |
| `preview/*`、`tabular/*`、`assets.rs` | `preview` |
| `runtime.rs` | `runtime` |
| `term/*`、`osc.rs` | `term` |

`app/update.rs` 里既有 shell 又有预览逻辑:顶部声明 `LOG`(shell,Task 3 已加)与 `PREVIEW_LOG`(`module`, `"preview"`);预览相关分支的日志用 `PREVIEW_LOG`。

机械改写同 Task 4 Step 3 的 `perl` 规则(把 `tracing::x!(` → `dozer_core::log_x!(LOG, `),再对预览相关文件里出现在 `PREVIEW_LOG` 语义下的调用手工换成 `PREVIEW_LOG`。

**保留字段**:`app/update.rs` 里三处手写的 `panel = ?panel`/`?kind`(大文件行索引建立失败、`app/update.rs:1178` 附近那条、表格首次加载失败)迁移后**保持该字段**,例如:

```rust
dozer_core::log_warn!(PREVIEW_LOG, panel = ?panel, tab_id, %error, "大文件行索引建立失败");
```

(来源标"代码属于谁",字段标"替谁工作"。)

逐条重定级(spec §7);`runtime.rs` 里约 12 处 "拒绝无效/无法解析 … IPC" 是 webview 发来非法消息的防御,保持 `warn`。`workspace/state.rs:788` 附近的"写入终端失败"(逐击键路径)**保持日志级别为 `warn`,不加 Toast**(见范围外)。

- [ ] **Step 4: 验证**

```bash
cargo build -p dozer-app 2>&1 | tail -5
cargo test -p dozer-app 2>&1 | rg "test result|FAILED"
# 此刻允许出现命中的只有 Task 6-8 还没迁移的面板/弹窗 extension 文件:
rg -n "tracing::(error|warn|info|debug|trace)!|eprintln!" crates/dozer-app/src | rg -v "extensions/"
```

Expected:构建/测试通过(基线失败集合不变);最后一条 `rg`(排除 `extensions/`)无输出。`extensions/` 下的命中留给 Task 6–8。

- [ ] **Step 5: 手工验证**

`cargo run -p dozer-app`(终端启动):stderr 出现一行启动横幅;`logs_dir()` 下出现 `dozer-app.log.<日期>`,内含 `dozer::module::log` 横幅。再 `RUST_LOG=info,dozer::module::shell=debug cargo run -p dozer-app` 确认过滤语法生效(不报错即可)。从 Finder 启动 `.app` 一次,确认日志仍落盘(此前会丢)。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app
git diff --cached --stat
git commit -m "refactor(app): init core log service, scope shell/platform/preview/runtime/term, drop DIAG logs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Files 面板(来源 + 日志 + 外部打开失败 Toast)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/update.rs`(约 :168-180)
- Modify: `crates/dozer-app/src/app/update.rs`(预览工具栏"外部打开"分支,约 :2336-2352)
- Modify: `crates/dozer-app/src/extensions/files/*.rs`(其余含日志调用的文件)

**Interfaces:** Consumes Task 3 的 `Outbox`/`push`、`files::WorkspaceState.outbox`。Produces:无。

- [ ] **Step 1: 写失败的测试(纯函数)**

外部打开失败要给用户一句可读的话。把"拼文案"抽成纯函数并测试(`files/update.rs` 或 `files/mod.rs`,与既有测试同处):

```rust
    #[test]
    fn open_failure_message_names_app_and_file() {
        assert_eq!(
            open_failure_message(Some("Typora"), std::path::Path::new("/p/a/notes.md")),
            "无法用 Typora 打开 notes.md"
        );
        assert_eq!(
            open_failure_message(None, std::path::Path::new("/p/a/notes.md")),
            "无法用系统默认应用打开 notes.md"
        );
        // 没有文件名(如根路径)时退回完整路径,不 panic。
        assert_eq!(
            open_failure_message(None, std::path::Path::new("/")),
            "无法用系统默认应用打开 /"
        );
    }
```

Run: `cargo test -p dozer-app open_failure_message 2>&1 | tail -6`
Expected:编译失败(函数未定义)。

- [ ] **Step 2: 实现并接入**

```rust
/// 外部打开失败时给用户看的一句话。
pub(crate) fn open_failure_message(app_name: Option<&str>, path: &std::path::Path) -> String {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    format!("无法用 {} 打开 {file}", app_name.unwrap_or("系统默认应用"))
}
```

`files/update.rs` 顶部声明 `dozer_core::scope!(pub(crate) LOG, panel, "files");`。把 `OpenWithDefault` 分支里原来的 `tracing::warn!("用外部软件打开失败: …")`(并删掉上方"只记日志,不弹 toast"的旧注释,改成"失败进 outbox → Toast")换成:

```rust
            if let Err(err) = command.spawn() {
                dozer_core::log_warn!(LOG, app = ?app_name, path = %path.display(), %err, "外部打开失败");
                ws_state.outbox.push(
                    LOG,
                    crate::extensions::toast::Level::Error,
                    open_failure_message(app_name.as_deref(), &path),
                );
            }
```

(`push` 排空时 `emit_toast` 还会再写一条 `module::toast` 日志;这里的 `log_warn!` 保留具体的 app/path/err 字段,两条互补。)

`app/update.rs` 的预览工具栏"外部打开"(`std::process::Command::new(program).arg(path).spawn()` 失败处,`with_focused_project(move |ws, _io| {...})` 闭包内)换成:

```rust
                    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
                        dozer_core::log_warn!(PREVIEW_LOG, %e, "外部打开失败");
                        ws.files.outbox.push(
                            PREVIEW_LOG,
                            toast::Level::Error,
                            crate::extensions::files::open_failure_message(None, path),
                        );
                    }
```

(`open_failure_message` 需 `pub(crate)` 并在 `files/mod.rs` 重导出;`ws.files.outbox` 是 `pub(crate)` 字段。)

`files` 目录下其余含 `tracing::`/`eprintln!` 的调用(`rg -n "tracing::|eprintln!" crates/dozer-app/src/extensions/files`)用 Task 4 的 `perl` 规则改成 `dozer_core::log_*!(LOG, …)`。

- [ ] **Step 3: 验证**

```bash
cargo test -p dozer-app open_failure_message 2>&1 | tail -6
cargo build -p dozer-app 2>&1 | tail -5
rg -n "tracing::|eprintln!" crates/dozer-app/src/extensions/files   # 期望无输出
```

Expected:通过、无输出。`Level::Error` 已被构造(dead_code 警告里不再有 `Error`,`Info/Success` 仍在)。

- [ ] **Step 4: 手工验收**

在文件树对某文件右键"用…打开",指定一个不存在的 App 名(或临时改 `external_apps` 配置为不存在的 App):右下角出现红色 Toast"无法用 X 打开 notes.md",约 8 秒消失;日志里有 `dozer::panel::files: 外部打开失败 …` 与 `dozer::module::toast: … scope="dozer::panel::files"` 两条。

- [ ] **Step 5: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app
git diff --cached --stat
git commit -m "feat(files): scope files logs and report external-open failures via Toast

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Todo / SSH / Database(来源 + 日志 + 写失败 Toast)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/update.rs`(`Mutated`/`CategoryMutated`)
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(两处 `save_hosts` 失败)
- Modify: `crates/dozer-app/src/extensions/database/update.rs`(两处 `save_sources` 失败)、`extensions/database/state.rs`(`database_drivers.json` 写失败)
- Modify: 三个模块内其余含日志的调用

**Interfaces:** Consumes Task 3 的 `Outbox::push_err`、各 state 的 `outbox`/`take_outbox`。Produces:无。

- [ ] **Step 1: 写失败的测试**

`push_err` 已在 Task 3 单测。这里为"写盘失败会进 outbox"各加一个针对性测试——用"repo_path 指向一个普通文件"让 `create_dir_all` 必失败(和 core 的测试同一手法)。为让被测代码可直接调用,先把 SSH 与 database 里"保存并把失败推进 outbox"抽成小函数(Step 2),测试针对这些函数:

ssh(`ssh.rs` 测试模块):

```rust
    #[test]
    fn persist_hosts_failure_lands_in_outbox() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let mut ws = WorkspaceState::default();
        persist_hosts(&mut ws, &blocker); // repo_path 是文件,写必失败
        let got = ws.take_outbox();
        assert_eq!(got.len(), 1);
        assert!(got[0].text.contains("SSH"), "{}", got[0].text);
        assert_eq!(got[0].scope.name, "ssh");
    }

    #[test]
    fn persist_hosts_success_leaves_outbox_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = WorkspaceState::default();
        persist_hosts(&mut ws, dir.path());
        assert!(ws.take_outbox().is_empty());
    }
```

database(`database/update.rs` 或 `database/mod.rs` 测试模块)同理对 `persist_sources`,断言文案含"数据库"且 `scope.name == "database"`。todo 的 `Mutated`/`CategoryMutated` 直接用 `Outbox::push_err`,由 Task 3 的 `outbox_push_err_*` 覆盖,不再单独造测试。

(若 `WorkspaceState` 没有 `Default`,用该模块里既有测试构造它的方式;以模块内现有测试为准。)

Run: `cargo test -p dozer-app persist_ 2>&1 | tail -8`
Expected:编译失败(函数未定义)。

- [ ] **Step 2: 实现**

**Todo**(`todo/update.rs` 顶部 `dozer_core::scope!(pub(crate) LOG, panel, "todo");`):

```rust
        Message::Mutated(res) => {
            ws_state.outbox.push_err(LOG, "Todo 操作失败", &res);
            request_todos_refresh(project_id, client, handle, emit);
        }
        // CategoryMutated 同理,文案 "分类操作失败"
```

保留原来的 `tracing::warn!`(改成 `dozer_core::log_warn!(LOG, …)` 的带具体错误那条可省——`emit_toast` 会写日志,避免重复:**删掉**这两处原日志,由 outbox → `log_toast` 承担)。

**SSH**(`ssh.rs` 顶部 `dozer_core::scope!(pub(crate) LOG, panel, "ssh");`):

```rust
/// 保存主机列表;失败不 panic,推一条 Toast(用户否则会以为已保存)。
pub(crate) fn persist_hosts(ws_state: &mut WorkspaceState, repo_path: &std::path::Path) {
    if let Err(e) = save_hosts(repo_path, &ws_state.hosts) {
        ws_state.outbox.push(
            LOG,
            crate::extensions::toast::Level::Error,
            format!("SSH 主机未能保存到磁盘: {e}"),
        );
    }
}
```

两处原 `if let Err(e) = save_hosts(repo_path, &ws_state.hosts) { tracing::warn!(...) }` 换成 `persist_hosts(ws_state, repo_path);`(两处的 `ws_state` 与 `repo_path` 变量名以原代码为准)。

**Database**(`database/update.rs` 与 `database/state.rs` 各声明 `dozer_core::scope!(pub(crate) LOG, panel, "database");`,同 crate 内两个模块各一个常量互不冲突;若已有 `mod.rs` 级公共导入避免重复声明,以编译器为准):

```rust
pub(crate) fn persist_sources(ws_state: &mut WorkspaceState, repo_path: &std::path::Path) {
    if let Err(e) = save_sources(repo_path, &ws_state.sources) {
        ws_state.outbox.push(
            LOG,
            crate::extensions::toast::Level::Error,
            format!("数据库连接配置未能保存到磁盘: {e}"),
        );
    }
}
```

两处 `save_sources` 失败换成 `persist_sources(ws_state, repo_path);`。`database/state.rs` 里 `写入 database_drivers.json 失败` 那处所在方法属于哪个 state 就往哪个 state 的 `outbox` 推(`AppState` 已在 Task 3 的 `drain_outboxes` 里排空 `self.database.take_outbox()`;`WorkspaceState` 同理),文案 `"数据库驱动配置未能保存到磁盘: {e}"`。

三个模块里其余 `tracing::`/`eprintln!` 用 `perl` 规则迁移。

- [ ] **Step 3: 验证**

```bash
cargo test -p dozer-app persist_ 2>&1 | tail -8
cargo build -p dozer-app 2>&1 | tail -5
rg -n "tracing::|eprintln!" crates/dozer-app/src/extensions/todo crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions/database   # 期望无输出
```

- [ ] **Step 4: 手工验收**

分别制造失败:让项目目录下的 `.dozer/`(或对应配置文件)不可写(`chmod -w`),再在 SSH 面板保存主机、数据库面板保存连接、Todo 面板增/改条目:每种都出现红色 Toast,日志里有 `dozer::panel::<名>`/`dozer::module::toast` 记录。恢复权限。

- [ ] **Step 5: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app
git diff --cached --stat
git commit -m "feat(todo,ssh,database): scope logs and surface write failures via Toast

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 其余面板/弹窗的来源 + 两个 App 级 Toast + 面板名校验

**Files:**
- Modify: `crates/dozer-app/src/extensions/project/update.rs`、`extensions/settings.rs`、`extensions/search.rs`、`extensions/conversations.rs`(各自的日志调用)
- Modify: `crates/dozer-app/src/app/app.rs`(`persist_open_projects`)
- Modify: `crates/dozer-app/src/workspace/state.rs`(两处一次性启动写失败)
- Modify: `crates/dozer-app/src/app/state.rs`(仅测试:`PanelKind` 面板名表)

**Interfaces:** Consumes Task 3 的 `Message::Toast(Push{scope,…})`(经 `proxy` 从后台任务发回)。Produces:无。

- [ ] **Step 1: 写失败的测试(面板名表)**

`app/state.rs` 测试模块里加(`PanelKind` 的穷举 `match` 保证新增变体时编译报错,`ALL` 数组随之更新):

```rust
    /// 每个 `PanelKind` 对应的日志面板名(单一真相)。`scope!(…, panel, "<名>")` 的
    /// 名字必须出自这张表——由下面的源码扫描测试强制。
    fn expected_log_name(kind: PanelKind) -> &'static str {
        match kind {
            PanelKind::Files => "files",
            PanelKind::GitLog => "git_log",
            PanelKind::Todo => "todo",
            PanelKind::Project => "project",
            PanelKind::Database => "database",
            PanelKind::Ssh => "ssh",
            PanelKind::Web => "web",
            PanelKind::Agent => "agent",
            PanelKind::Conversations => "conversations",
            PanelKind::Usage => "usage",
            PanelKind::CodeHealth => "code_health",
        }
    }

    const ALL_KINDS: [PanelKind; 11] = [
        PanelKind::Files,
        PanelKind::GitLog,
        PanelKind::Todo,
        PanelKind::Project,
        PanelKind::Database,
        PanelKind::Ssh,
        PanelKind::Web,
        PanelKind::Agent,
        PanelKind::Conversations,
        PanelKind::Usage,
        PanelKind::CodeHealth,
    ];

    #[test]
    fn log_names_are_unique_and_valid_scope_names() {
        let mut seen = std::collections::HashSet::new();
        for k in ALL_KINDS {
            let n = expected_log_name(k);
            assert!(dozer_core::log::is_valid_scope_name(n), "{n}");
            assert!(seen.insert(n), "重复的面板日志名: {n}");
        }
    }

    /// 源码里每个 `scope!(_, panel, "x")` 的 x 都必须是某个 `PanelKind` 的日志名;
    /// 弹窗类 extension 与非面板代码必须用 `module`,不能冒充面板。
    #[test]
    fn every_panel_scope_in_source_uses_a_known_panel_name() {
        let known: std::collections::HashSet<&str> =
            ALL_KINDS.iter().map(|k| expected_log_name(*k)).collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut found = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    for line in text.lines() {
                        let Some(rest) = line.split("scope!(").nth(1) else { continue };
                        let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
                        if parts.len() >= 3 && parts[1] == "panel" {
                            let name = parts[2].trim_end_matches(");").trim_matches('"');
                            found += 1;
                            assert!(known.contains(name), "{}: 未知面板日志名 {name}", p.display());
                        }
                    }
                }
            }
        }
        assert!(found >= 3, "至少应有 files/todo/ssh/database/agent 等面板来源,实际扫到 {found}");
    }
```

Run: `cargo test -p dozer-app log_names 2>&1 | tail -6` 与 `cargo test -p dozer-app every_panel_scope 2>&1 | tail -8`
Expected:此时这两个测试应**通过**(表本身与已声明的来源 files/todo/ssh/database/agent 一致)——它们是回归护栏,不是红绿循环的一部分;若 `every_panel_scope` 失败,说明前面某个 Task 里写错了面板名,按报错修正名字。

- [ ] **Step 2: 迁移剩余含日志的 extension**

`rg -n "tracing::|eprintln!" crates/dozer-app/src/extensions` 应只剩 `project/update.rs`(3 处)、`settings.rs`(1 处)、`search.rs`(1 处)、`conversations.rs`(1 处)。来源:

| 文件 | 声明 |
|---|---|
| `extensions/project/update.rs` | `dozer_core::scope!(LOG, panel, "project");` |
| `extensions/settings.rs` | `dozer_core::scope!(LOG, module, "settings");` |
| `extensions/search.rs` | `dozer_core::scope!(LOG, module, "search");` |
| `extensions/conversations.rs` | `dozer_core::scope!(LOG, panel, "conversations");` |

用 `perl` 规则改写并按 spec §7 重定级(`project/update.rs` 里的"提交总结批次失败/查询总结批次失败"已经经 `SummaryBackfillFailed` 消息回到面板状态,保持 `warn`)。

**不要**给还没有日志调用的面板(git_log、web、usage、code_health、footbar 等)提前声明 `LOG`(会触发 dead_code)。面板第一次写日志时,在其模块顶部用 `scope!(…, panel, "<PanelKind 表里的名字>")`——`every_panel_scope_in_source_uses_a_known_panel_name` 会保证名字不写错。

- [ ] **Step 3: 两个 App 级 Toast**

**项目页签集合写盘失败**(`app/app.rs::persist_open_projects`):

```rust
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            if let Err(e) = open_projects::save(&state) {
                let _ = proxy.send_event(Message::Toast(toast::Message::Push {
                    scope: LOG,
                    level: toast::Level::Warning,
                    text: format!("项目页签未能保存,下次启动可能丢失: {e}"),
                    key: Some("persist-open-projects".to_string()),
                }));
            }
        });
```

(`LOG` 是 `app/app.rs` 文件顶部 Task 5 已声明的 `shell` 来源;`Message::Toast` 处理分支会写日志,所以这里不再重复 `log_warn!`。**退出路径**里若同一个函数在事件循环已停止时被调用,`send_event` 失败只会丢 Toast 与日志——`app/app.rs` 里 `退出前窗口尺寸写盘失败` 那条本就是仅日志、保持不动;`persist_open_projects` 若在退出路径被调用,需先确认(`rg -n "persist_open_projects" crates/dozer-app/src`),退出路径改用同步 `dozer_core::log_warn!` 并不推 Toast。)

**启动写终端失败**(`workspace/state.rs` 里"自动键入初始命令失败"与"派发任务文本失败"两处,闭包里已有 `proxy`):把 `tracing::warn!("自动键入初始命令失败: {e}")` 换成

```rust
                            let _ = proxy.send_event(Message::Toast(toast::Message::Push {
                                scope: LOG,
                                level: toast::Level::Error,
                                text: format!("未能把启动命令写入终端: {e}"),
                                key: Some("term-initial-write".to_string()),
                            }));
```

"派发任务文本失败"同理,文案 `"未能把任务文本写入终端: {e}"`,key `"term-dispatch-write"`。(`workspace/state.rs` 的 `LOG` 是 Task 5 声明的 `shell`。)**不要**动 `send_input`(逐击键路径,见范围外)。

- [ ] **Step 4: 验证**

```bash
cargo test -p dozer-app 2>&1 | rg "test result|FAILED"
cargo build -p dozer-app 2>&1 | tail -5
rg -n "tracing::(error|warn|info|debug|trace)!|eprintln!" crates/dozer-app/src   # 期望:无输出
```

Expected:通过(基线失败集合不变);最后一条 `rg` **无输出**(此时 `dozer-app` 全部迁完)。dead_code 警告中 `Message::Toast`、`toast::Message::Push` 应已消失(已有真实构造点),只剩 `Level::Info/Success` 仍可能存在——这两个级别目前没有调用点,**保留**,不要为了消警告去删(后续成功类提示会用);若编译器仍报,给这两个变体加 `#[allow(dead_code)] // 预留:成功/信息类提示暂无调用点`。

- [ ] **Step 5: 手工验收**

- 让项目页签集合写盘失败(临时把 `open_projects` 的目标文件设只读),切换/新增页签:出现黄色 Toast(Warning)。
- 新建 agent 会话时让 daemon 断开(使启动命令写入失败):出现红色 Toast。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app
git diff --cached --stat
git commit -m "refactor(app): scope remaining extension logs, toast tab-persist and startup-write failures

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 9: 拆分 Files 的 `tree_error`(移动对话框内联 vs 操作失败 Toast)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/state.rs`、`files/update.rs`、`files/view.rs`
- Modify: `crates/dozer-app/src/app/app.rs`(测试里用 `tree_error` 做路由标记的三处)

**Interfaces:** Consumes Task 3 的 `Outbox`。Produces:`files::WorkspaceState.move_error: Option<String>`(取代 `tree_error`,**仅**承载移动对话框内的校验错误),`move_error()`/`set_move_error()` 测试访问器(取代 `tree_error()`/`set_tree_error()`)。

**背景(审计更正):** `tree_error` 的全部写入点都是文件操作反馈(见本文档开头)。其中**只有** `MoveConfirm` 的两处校验("名字不能为空或包含路径分隔符"、"… 不是有效目录")发生在一个**仍然打开的对话框**里、需要在对话框内显示;其余(粘贴失败、回滚失败、`OpDone` 失败、`FileDropDone` 失败、拖拽进项目内被拒、行内新建/重命名的三处校验)都是"操作被拒/失败",走 Toast。

- [ ] **Step 1: 核实渲染点**

读 `extensions/files/view.rs` 的 `:223` 与 `:1466` 两处 `tree_error` 渲染,确认:`:223` 在文件树面板头部;`:1466` 在**移动对话框**(`PendingMove`)正文里。

- 若确认如上:继续 Step 2。
- 若 `:1466` **不是**移动对话框(或对话框另有独立错误行):**停下汇报**,不要猜——本 Task 的拆分边界依赖这一点。

- [ ] **Step 2: 写失败的测试**

在 `files/mod.rs`(或既有 `files` 测试模块)加,针对 `state.rs` 里行内新建/重命名的校验(它们是同步方法,可直接调用):

```rust
    #[test]
    fn inline_rename_with_path_separator_toasts_and_keeps_editor_open() {
        let mut ws = WorkspaceState::default();
        // 造一个正在重命名的行内编辑,缓冲区含路径分隔符;具体构造方式以本模块既有
        // 行内编辑测试为准(见 `commit_tree_edit` 的现有测试)。
        ws.begin_rename_for_test("/tmp/proj/a.txt", "x/y");
        ws.commit_tree_edit();
        let got = ws.take_outbox();
        assert_eq!(got.len(), 1);
        assert!(got[0].text.contains("路径分隔符"));
        assert_eq!(got[0].scope.name, "files");
        assert!(ws.tree_edit_is_open(), "校验失败后行内编辑框应保持打开");
        assert!(ws.move_error().is_none(), "行内校验不应写进移动对话框的错误位");
    }

    #[test]
    fn move_confirm_validation_stays_inline_in_the_dialog_not_a_toast() {
        let mut ws = WorkspaceState::default();
        ws.set_pending_move_for_test("a/b", "/tmp");
        // 走 `Message::MoveConfirm` 的更新路径(以本模块既有 update 测试的调用方式为准)。
        run_move_confirm(&mut ws);
        assert!(ws.move_error().is_some());
        assert!(ws.take_outbox().is_empty(), "对话框内校验不进 Toast");
    }
```

(`begin_rename_for_test`/`set_pending_move_for_test`/`tree_edit_is_open`/`run_move_confirm` 是**测试辅助**,按本模块既有测试如何构造 `TreeEdit`/`PendingMove`/调用 `update` 来写,不要凭空发明生产代码 API;既有测试文件里已经有可复用的构造方式,先读再写。)

Run: `cargo test -p dozer-app files:: 2>&1 | rg "FAILED|error" | head`
Expected:编译失败或断言失败(`move_error` 未定义)。

- [ ] **Step 3: 实现**

1. `state.rs`:字段 `tree_error` 重命名为 `move_error`(文档注释改为"仅移动对话框内的校验错误");测试访问器 `tree_error()`/`set_tree_error()` 改名为 `move_error()`/`set_move_error()`;`app/app.rs` 里三处用它做路由标记的测试(`loaded_slot`、`slot_marker` 及两处断言)同步改名。
2. `state.rs` 行内新建/重命名的四处赋值(`名字不能包含路径分隔符`、三处 `已存在同名项`)改成往 outbox 推,`tree_edit` 的放回逻辑保持不变:

```rust
self.outbox.push(LOG, crate::extensions::toast::Level::Error, "名字不能包含路径分隔符");
// "{} 已存在同名项" 同理:format!("{} 已存在同名项", new_path.display())
```

3. `update.rs`:

| 位置 | 改成 |
|---|---|
| `PasteDone`、`FileHistoryRollbackDone`、`OpDone`、`FileDropDone` 的 `Err(e) => ws_state.tree_error = Some(e)` | `Err(e) => ws_state.outbox.push(LOG, toast::Level::Error, format!("<操作名>失败: {e}"))`,操作名依次为 `粘贴`、`还原文件`、`文件操作`、`拖拽移动` |
| `FileDrop` 里"已在项目内,不支持用拖拽移动项目内文件" | `ws_state.outbox.push(LOG, toast::Level::Warning, format!("{} 已在项目内,不支持用拖拽移动项目内文件", bad.display()))`,`return` 保持 |
| `MoveConfirm` 的两处校验 | `ws_state.move_error = Some(...)`(内容不变),**不**推 outbox |
| 所有 `ws_state.tree_error = None;` 重置 | 改名为 `ws_state.move_error = None;`(在与移动对话框无关的分支里这些重置本就无意义,改名后语义变成"清除对话框错误",保持原位置不删,避免行为漂移;在纯粹的文件树操作分支里若其唯一作用是清 `tree_error`,可删除,**但每处先读上下文**) |

4. `view.rs`:删除 `:223` 面板头的红字块;`:1466` 移动对话框里的渲染改读 `move_error`。

- [ ] **Step 4: 验证**

```bash
cargo test -p dozer-app files:: 2>&1 | rg "test result|FAILED"
cargo build -p dozer-app 2>&1 | tail -5
rg -n "tree_error" crates/dozer-app/src   # 期望:无输出(全部改名)
```

Expected:files 测试通过(除基线已知失败);`rg` 无输出。

- [ ] **Step 5: 手工验收**

- 在文件树里对同一目录粘贴一个已存在的同名文件夹、重命名成带 `/` 的名字、拖拽项目内文件——每种都出现 Toast,面板头**不再有常驻红字**。
- 打开"移动到…"对话框填非法名字/不存在的目录:错误仍显示在**对话框内**,对话框不关,且没有 Toast。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app
git diff --cached --stat
git commit -m "refactor(files): split tree_error into move-dialog inline error and operation Toasts

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 10: `dozer-hook` 运行期诊断接入纯 std 日志

**Files:**
- Modify: `crates/dozer-hook/src/main.rs`(约 :148)、`crates/dozer-hook/src/goose.rs`(约 :79/:87/:91)
- Modify: `crates/dozer-hook/src/lib.rs` 或 `main.rs`(声明来源常量)

**Interfaces:** Consumes Task 1 的 `Component::Hook`、`scope!`、`plain_warn!`。Produces:无。

**范围结论(读代码后收窄):** `dozer-hook`/`dozer-mcp` 里的 `eprintln!` 绝大多数是**给命令行用户看的结果**(`install`/`uninstall`/`launch` 的用法与失败提示、`dozer-mcp` 的 `main` 错误输出),**不是日志,保持不动**。真正的运行期诊断只有 hook 里 4 处:`main.rs` 的"opencode transcript 落盘失败(已忽略,不影响转发)"与 `goose.rs` 的三处"建 journal 目录/打开 journal/写 journal 失败(已忽略)"。`dozer-mcp` 没有运行期诊断的 `eprintln!`,本 Task 不改 `dozer-mcp`。`dozer-hook` 与 `dozer-mcp` 都**不开** `logging` feature。

- [ ] **Step 1: 盘点并确认**

Run: `rg -n "eprintln!" crates/dozer-hook/src`
逐条对照:只有 `main.rs`(转发路径上"…已忽略,不影响转发")与 `goose.rs`(journal 追加,"已忽略")属于运行期诊断。`install.rs`/`goose_install.rs`/`opencode_install.rs`/`aider_launcher.rs`/`main.rs:57` 的 `eprintln!` 是 CLI 用户输出,保持。若发现盘点与此不符,以实际读到的为准并在提交信息里写明。

- [ ] **Step 2: 声明来源并替换那 4 处**

`dozer-hook/src` 顶层(如 `main.rs` 顶部,`goose.rs` 里 `use crate::LOG`,或各文件自己声明——以现有模块结构为准,选最少改动的):

```rust
dozer_core::scope!(pub(crate) LOG, module, "hook");
```

替换(消息文案保留,末尾 `已忽略` 语义不变):

```rust
// main.rs
dozer_core::plain_warn!(dozer_core::log::Component::Hook, LOG, "opencode transcript 落盘失败(已忽略,不影响转发): {e}");
// goose.rs 三处
dozer_core::plain_warn!(dozer_core::log::Component::Hook, LOG, "建 journal 目录失败(已忽略): {e}");
dozer_core::plain_warn!(dozer_core::log::Component::Hook, LOG, "打开 journal 失败(已忽略): {}", path.display());
dozer_core::plain_warn!(dozer_core::log::Component::Hook, LOG, "写 journal 失败(已忽略): {e}");
```

注意:**不再写 stderr**(hook 的 stderr 可能被 agent 当成 hook 输出展示);这是对现状的有意改变。`goose.rs:72` 的文档注释里"失败只记 `eprintln!`"同步改成"失败只记日志文件"。

- [ ] **Step 3: 验证**

```bash
cargo build -p dozer-hook 2>&1 | tail -5
cargo test -p dozer-hook 2>&1 | rg "test result|FAILED"
cargo tree -p dozer-hook | rg -c tracing   # 期望 0:hook 没有被拉进 tracing
```

Expected:通过;`tracing` 计数为 0。

- [ ] **Step 4: 手工验证**

让 goose journal 目录不可写后触发一次 hook(或直接用 `dozer-hook` 的相应子命令),确认 `logs_dir()/dozer-hook.log.<日期>` 出现一行 `WARN dozer::module::hook: 写 journal 失败(已忽略) …`,且 stderr 无输出。

- [ ] **Step 5: 提交**

```bash
cargo fmt && cargo clippy -p dozer-hook --all-targets 2>&1 | tail -5
git add crates/dozer-hook
git diff --cached --stat
git commit -m "refactor(hook): route runtime diagnostics to the plain log file (CLI output unchanged)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 11: 门禁(clippy + 兜底脚本)+ 文档同步

**Files:**
- Create: `scripts/check-log-scope.sh`
- Create(取决于 Task 2 spike B): `clippy.toml`
- Modify: `crates/dozer-hook/src/main.rs`、`crates/dozer-mcp/src/main.rs`(仅当启用 clippy.toml:crate 根 `#![allow(clippy::disallowed_macros)]`)
- Modify: `CLAUDE.md`
- Modify: `docs/superpowers/specs/2026-09-30-unified-logging-design.md`、`2026-09-30-error-feedback-audit.md`

**Interfaces:** Consumes 全部前序 Task。Produces:无。

- [ ] **Step 1: 兜底脚本**

`scripts/check-log-scope.sh`(可执行):

```bash
#!/usr/bin/env bash
# 门禁:dozerd 与 dozer-app 里不允许裸 tracing 宏或 eprintln!(必须走 dozer_core::log_*!,
# 以便日志带来源)。dozer-hook/dozer-mcp 的 eprintln! 多为 CLI 用户输出,不在此检查。
set -euo pipefail
cd "$(dirname "$0")/.."
hits=$(rg -n "tracing::(error|warn|info|debug|trace)!|eprintln!" crates/dozerd/src crates/dozer-app/src || true)
if [ -n "$hits" ]; then
  echo "发现未带来源的日志调用(请改用 dozer_core::log_*!(LOG, ...)):" >&2
  echo "$hits" >&2
  exit 1
fi
echo "log scope check: ok"
```

Run: `chmod +x scripts/check-log-scope.sh && scripts/check-log-scope.sh`
Expected:`log scope check: ok`。再做一次反向验证,确认脚本真的能失败:

```bash
echo 'fn _t() { tracing::warn!("x"); }' >> crates/dozerd/src/lib.rs
scripts/check-log-scope.sh; echo "exit=$?"    # 期望 exit=1,并打印该行
git checkout crates/dozerd/src/lib.rs          # 必须还原
```

(`rg` 不区分注释,所以门禁也会抓到注释里写着 `tracing::warn!` 的文字;迁移时若有这类注释,改写措辞即可。)

- [ ] **Step 2: clippy 门禁(取决于 Task 2 spike B 的结果)**

- spike B **无误报**:在仓库根加 `clippy.toml`(内容同 Task 2 Step 2 的 `disallowed-macros`,另加 `{ path = "std::eprintln", reason = "use dozer_core::log_*! (dozerd/dozer-app); CLI output in hook/mcp is allowed" }`);`dozer-core/src/log.rs` 顶部加 `#![allow(clippy::disallowed_macros)]`(该文件内部合法使用 `tracing::*`);`dozer-hook`、`dozer-mcp`(CLI 输出用 `eprintln!`)的 crate 根加 `#![allow(clippy::disallowed_macros)]`。`cargo clippy --workspace --all-targets` 应无新增 `disallowed_macros` 告警。
- spike B **有误报**:**不建 `clippy.toml`**,只保留 Step 1 的脚本作为门禁,并在提交信息与 `CLAUDE.md` 说明"门禁 = 兜底脚本"。

- [ ] **Step 3: `CLAUDE.md` 关键裁决新增一条**

在「瞬时消息统一走 Toast」条之后加:

```markdown
- **日志统一走 `dozer_core::log`**(见 `docs/superpowers/specs/2026-09-30-unified-logging-design.md`):`dozerd`/`dozer-app` 里禁止裸 `tracing::warn!` 等与 `eprintln!`(`scripts/check-log-scope.sh` 门禁),一律 `dozer_core::log_*!(LOG, ...)`,`LOG` 由文件顶部 `dozer_core::scope!(LOG, panel|module, "<名>")` 声明。**面板日志的来源名必须是 `PanelKind` 对应的面板名**(`files`/`git_log`/`todo`/`project`/`database`/`ssh`/`web`/`agent`/`conversations`/`usage`/`code_health`;`app/state.rs` 的测试会扫源码强制);非面板代码用 `module` 来源(`shell`/`platform`/`preview`/`runtime`/`term`/弹窗类 extension 名)。共享代码里"替哪个面板工作"作为普通字段带上(如 `panel = ?panel`)。面板第一次写日志时才声明 `LOG`,不要预先声明未使用的来源。每条 Toast 都会自动写日志(`toast::log_toast`,`tracing` 的 `target:` 必须是常量,所以运行时来源放进字段 `scope`)。`dozer-hook`/`dozer-mcp` 不引入 `tracing`:运行期诊断用 `dozer_core::plain_*!`,给命令行用户看的 `install`/用法输出仍是 `eprintln!`。日志文件在 `logs_dir()`,按天滚动、保留 14 天,不设大小上限。
```

并把「瞬时消息统一走 Toast」那条里"extension 不得直接持有 `ToastCenter`,只能经内核"补一句"(允许持有纯数据的 `toast::Outbox`,由 `App::update` 包装函数统一排空)"。

- [ ] **Step 4: spec 与审计文档同步**

`2026-09-30-unified-logging-design.md`:
- 「与 Toast 的关系」:接口段改为"`push_toast(scope, level, text)`…已落地";写日志位置改为"`toast::log_toast` 是唯一写 Toast 日志的函数(`App::emit_toast` 与 `Message::Toast` 分支各调用一次)",并加一段"**实施偏差**:`tracing` 的 `target:` 必须是常量,而 Toast 的 `Scope` 是运行时形参,不能当 target;Toast 日志固定用来源 `module::toast`,调用方来源放进字段 `scope`(值为 target 字符串)。因此 Toast 日志不能用 `RUST_LOG=…dozer::panel::files=…` 按面板过滤,但仍然写明面板名。"删掉原文里"带 `toast = true` 字段"的描述。
- §4b 与 §5、迁移批 4:改为"hook/mcp 的 `eprintln!` 绝大多数是 CLI 用户输出;运行期诊断只有 hook 的 4 处,`dozer-mcp` 无;兜底脚本与 clippy 只约束 `dozerd`/`dozer-app`,hook/mcp 的 crate 根允许 `disallowed_macros`"。
- 「已决事项」补:`Toast 自动写日志` 已落地(计划 `2026-09-30-logging-and-toast-migration.md`)。

`2026-09-30-error-feedback-audit.md`:`tree_error` 的更正已在写本计划时提前改好(表 A 与批 4);此处只需核对与 Task 9 实际落地一致(字段名 `move_error`、只有移动对话框内的两处校验留内联),不一致则以落地为准修正文档。

- [ ] **Step 5: 总验证**

```bash
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | rg "test result|FAILED"
scripts/check-log-scope.sh
cargo clippy --workspace --all-targets 2>&1 | tail -5
cargo fmt --check && echo fmt-ok
```

Expected:构建通过;测试只有已知基线失败(`delete_confirm_spec_reflects_pending_target`);脚本 ok;clippy 无新增告警;fmt ok。再手工过一遍:各 Task 里标注的手工验收(至少 Task 5 的落盘、Task 6/7 的 Toast、Task 9 的对话框内联)。

- [ ] **Step 6: 提交并交接审阅**

```bash
git add scripts/check-log-scope.sh CLAUDE.md docs/superpowers/specs crates
git diff --cached --stat
git commit -m "chore(log): add log-scope gate, document logging convention, sync specs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

不要自行合并 `feat/logging-and-toast-migration`;交给用户审阅后再合并(合并前先看主 checkout 有无别的会话的未提交改动/并发提交,合并后重跑全量构建)。
