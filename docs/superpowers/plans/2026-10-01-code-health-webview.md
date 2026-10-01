# 代码健康度面板内容侧迁移到 WebView(含架构地图)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把代码健康度面板内容侧整体换成 Preact + esbuild 离线打包的 webview,并在其上新增用 Cytoscape.js + dagre 渲染的「架构」分类。

**Architecture:** 仿用量面板(`extensions/usage/`)的"长驻单槽 webview + Rust 声明式推送"模式:Rust 侧 `protocol.rs` 把 `WorkspaceState` 算成一份可序列化的 `CodeHealthViewPayload`,每帧与上次送达值比较,不同且 webview ready 才经 `dispatch_script` 注入;webview 只负责渲染,点击事件(扫描/跳转文件/交给 Agent 分析)经 `window.ipc.postMessage` 回传。右侧分类导航仍是原生 iced。分两阶段:阶段一迁四个现有分类并删旧渲染;阶段二新增架构分类。

**Tech Stack:** Rust(iced 0.14、wry、serde)、Preact 10.29.8、esbuild 0.28.2、TypeScript 5.9.3、Cytoscape.js 3.34.3 + cytoscape-dagre 4.0.1 + dagre 0.8.5(均 MIT)、`node --test`。

**Spec:** `docs/superpowers/specs/2026-10-01-code-health-webview-design.md`

## Global Constraints

- **工作位置**:必须在独立 worktree 分支 `codehealth-webview` 里做(见 Task 0)。**所有路径、命令前缀一律用 worktree 绝对路径** `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview`,不要漂回 main 工作树(main 上常有并发 WIP 与未提交改动,例如 `webview_geometry.rs`)。
- 只读:不加任何编辑入口(CLAUDE.md 核心原则)。
- 字体:webview 内一律系统默认字体(`-apple-system, "PingFang SC", sans-serif`),不上等宽字体。
- 主题:ByteBoy2077 令牌(bg `#0a0e16`、金 `#F2D94E` 仅给甲方动作、奶油 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`);沿用 `usage-content` 的 `:root[data-theme]` CSS 变量定义。
- 前端离线:无 CDN、无运行时 Node、不输出 sourcemap、esbuild `format: 'iife'`、`jsx: 'automatic'` + `jsxImportSource: 'preact'`;host.html CSP 恒为 `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'`,不声明 `connect-src`。
- 日志:只用 `dozer_core::log_*!(LOG, ...)`,禁止裸 `tracing::*!`/`eprintln!`(`scripts/check-log-scope.sh` 门禁);面板来源名必须是 `code_health`;第一次写日志时才声明 `LOG`,不预先声明未使用的来源。
- 瞬时失败(跳转文件失败等)走 `App::push_toast`;`daemon_unavailable` 走顶栏徽标,不在内容区自画。
- 新增函数参数 ≥7 个且多个同类型参数相邻时,用具名字段参数结构体。
- 变化与严重度不只靠颜色,文字标签保留。
- 直接切换:迁完即删旧 iced 内容渲染,不留两套并存,不保留旧渲染作 webview 失败回退。
- WebView 恒在 iced 之上:本面板内容列整块由 webview 覆盖,右侧导航仍是 iced,不被盖住。
- Cargo 不改依赖(本计划只加 npm 依赖);`Cargo.lock`/`Cargo.toml` 在 main 上有未提交改动,与本计划无关,**不要**把它们带进本分支的任何提交。
- 提交只 `git add` 本任务列出的具体路径;提交前 `git diff --cached --stat` 核对没有无关文件。
- 每个提交消息末尾加:`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

(spec 隐含但没有任务专属测试覆盖、最可能咬到使用者的输入/条件,每条在所属任务里都有测试)

1. **webview 发来的跳转路径是绝对路径或含 `..`**:不得被信任去打开项目外文件 → Task 4 `is_safe_relative_path` 测试,拒绝并打日志。
2. **webview 永远不发 `Ready`(加载失败/被拦截)**:不能永远空白 → Task 1 `observe_availability` 超时置 `failed`,Task 4 内容区回落原生"失败原因 + 重试"页。
3. **快速连点分类,旧推送晚到覆盖新状态** → Task 1 `revision` 单调递增,Task 5 前端丢弃小于当前值的推送。
4. **旧 schema 报告 / 无架构数据 / 非 Cargo 项目**:架构页不得显示成"零风险" → Task 8 `status`/`status_note` 测试,Task 11 前端分三种空态。
5. **大图**:模块数超上限时前端不得卡死 → Task 8 `MAX_PAYLOAD_NODES` 裁成 crate 层并带截断说明。
6. **项目切换 / 面板切走再切回**:webview 被销毁重建后新实例必须收到完整当前内容 → Task 1 `set_ready(true)` 清 `last_sent`。

---

## File Structure

**新建**

| 路径 | 职责 |
|---|---|
| `crates/dozer-app/src/extensions/codehealth/protocol.rs` | Rust→webview 的 payload 类型、`current_view_payload`、webview→Rust 事件、推送 envelope、`WebviewPushState` |
| `crates/dozer-app/web/codehealth-content/` | 前端工程(package.json / build.mjs / tsconfig / render-smoke.mjs / src/**) |
| `crates/dozer-app/assets/codehealth-content/` | 构建产物(host.html / codehealth-content.js / codehealth-content.css),**提交进仓库** |
| `docs/superpowers/plans/2026-10-01-code-health-webview.md` | 本文件 |

**修改**

| 路径 | 改动 |
|---|---|
| `src/extensions/codehealth/mod.rs` | 声明 `protocol` 模块;新增 `Message::ContentRetry`;阶段一删 `StructureFilter`/`StructureFilterSet`;阶段二加 `Architecture` 分类 |
| `src/extensions/codehealth/view_model.rs` | 接收 `format_ms`、标签函数(从 `view.rs` 搬来) |
| `src/extensions/codehealth/view.rs` | 阶段一结束时只剩 `list_pane` + 原生失败占位的 `content_pane` |
| `src/webview_geometry.rs` | 抽出共享 `pair_content_pane_bounds_for`;新增 `codehealth_content_pane_bounds_for` |
| `src/assets.rs` | `codehealth-content/` 路由与测试 |
| `src/app/app.rs` | `CODEHEALTH_CONTENT_ID_OFFSET`、`codehealth_webview` 字段、`take_codehealth_content_script`、`preview_desired` 分支 |
| `src/app/message.rs` / `update.rs` / `runtime.rs` / `platform/window_events.rs` | 新 Message、事件处理、IPC 分发、推送消费点 |
| `.gitignore` | 忽略新前端工程 `node_modules` |
| `docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md` | 「布局与渲染」顶部加取代说明 |
| `docs/superpowers/specs/2026-10-01-code-health-webview-design.md` | 订正两处与现状不符的描述(Task 4) |

---

## Phase 0

### Task 0: 建立隔离 worktree 与基线

**Files:** 无代码改动。

- [ ] **Step 1: 创建 worktree**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add .worktrees/codehealth-webview -b codehealth-webview main
cd .worktrees/codehealth-webview && git status --short && git log --oneline -1
```

Expected: `git status --short` 无输出(干净);`git log` 显示 main 的 HEAD(应包含 `6099dbd3 docs: add code-health webview migration design`)。

- [ ] **Step 2: 确认基线能编译、相关测试全绿**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app extensions::codehealth 2>&1 | tail -15
cargo test -p dozer-app webview_geometry::tests::usage 2>&1 | tail -15
```

Expected: 两条都以 `test result: ok.` 结束。记下 `extensions::codehealth` 的通过数量,Task 7 删旧测试后对照。

- [ ] **Step 3: 确认前端工具链**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/usage-content
npm ci && npm test 2>&1 | tail -8
```

Expected: `# pass` 大于 0、`# fail 0`。若 `npm ci` 失败(无网络),停下来向用户报告,不要继续。

---

## Phase 1:骨架 + 四个现有分类

### Task 1: Rust 协议模块 `protocol.rs`

**Files:**
- Create: `crates/dozer-app/src/extensions/codehealth/protocol.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`(声明模块)
- Modify: `crates/dozer-app/src/extensions/codehealth/view_model.rs`(接收 `format_ms`/标签函数)
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `WorkspaceState` 的 `report()/previous_report()/diff()/hotspots()/git()/scanned_at_ms()/scanning()/scan_error()/save_error()/category()`(`mod.rs`);`view_model::{change_summary, hotspot_rows, hotspot_reasons, scope_summary, ui_consistency_applicable, empty_state, legacy_report_note, signed_delta}`。
- Produces(后续任务依赖的精确名字):
  - `pub enum CategoryKey { Overview, Structure, UiConsistency, ScanScope }`(阶段二加 `Architecture`)
  - `pub struct ScanBar { scanning: bool, scanned_at: Option<String>, scan_error: Option<String>, save_error: Option<String> }`
  - `pub struct FindingRowDto`、`pub enum Body`、`pub struct CodeHealthViewPayload { scan: ScanBar, category: CategoryKey, body: Body }`
  - `pub fn current_view_payload(ws: &WorkspaceState) -> CodeHealthViewPayload`
  - `pub enum CodeHealthWebviewEvent { Ready, ScanRequested, OpenLocation { path: PathBuf, line: usize }, AnalyzeFinding { id: String }, Failed { reason: String } }`
  - `pub fn parse_codehealth_event(body: &str) -> Result<CodeHealthWebviewEvent, String>`
  - `pub fn is_safe_relative_path(path: &Path) -> bool`
  - `pub fn encode_codehealth_push(revision: u64, payload: CodeHealthViewPayload) -> String`
  - `pub struct WebviewPushState`,方法 `set_ready(bool)`、`pending_push(&self, &CodeHealthViewPayload) -> Option<CodeHealthViewPayload>`、`mark_sent(&mut self, CodeHealthViewPayload) -> u64`(返回本次 revision)、`observe_availability(&mut self, available: bool, now: Instant)`、`failed(&self) -> Option<&str>`、`set_failed(String)`、`clear_failed()`、`READY_TIMEOUT`。

- [ ] **Step 1: 把 `format_ms`/`civil_from_days`/标签函数搬进 `view_model.rs`**

`view.rs` 里的 `tier_label`、`sev_label`、`change_label`、`format_ms`、`civil_from_days` 会随 `view.rs` 一起被删,协议层要用它们,所以先搬(原样复制到 `view_model.rs` 文件末尾 `#[cfg(test)]` 之前,加 `pub(super)`),`view.rs` 暂时保留自己的副本不动(Task 7 一起删)。

在 `view_model.rs` 的 `use` 区加 `HealthTier`:

```rust
use dozer_codehealth::{FindingChange, FindingSeverity, HealthTier, ProjectReport, ReportDiff};
```

在文件末尾 `#[cfg(test)]` 之前追加:

```rust
pub(super) fn tier_label(tier: HealthTier) -> &'static str {
    match tier {
        HealthTier::Healthy => "健康",
        HealthTier::Watch => "需要关注",
        HealthTier::Critical => "警戒",
    }
}

pub(super) fn sev_label(s: FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Critical => "警戒",
        FindingSeverity::Watch => "关注",
    }
}

pub(super) fn change_label(c: FindingChange) -> &'static str {
    match c {
        FindingChange::New => "本轮新增",
        FindingChange::Worsened => "本轮恶化",
        FindingChange::Improved => "本轮改善",
        FindingChange::Persisting => "持续存在",
        FindingChange::Resolved => "已解决",
    }
}

/// 同 `git_log.rs::format_commit_time`:展示 UTC,不引入时区库。
pub(super) fn format_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}
```

并在 `view_model.rs` 的 `mod tests` 里加:

```rust
    #[test]
    fn format_ms_matches_expected_layout() {
        assert_eq!(format_ms(1_789_891_086_991), "2026-09-20 07:58:06 UTC");
        assert_eq!(format_ms(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn tier_labels_include_text_for_all_states() {
        assert_eq!(tier_label(HealthTier::Healthy), "健康");
        assert_eq!(tier_label(HealthTier::Watch), "需要关注");
        assert_eq!(tier_label(HealthTier::Critical), "警戒");
    }
```

- [ ] **Step 2: 写失败测试(协议模块骨架 + 测试)**

先在 `mod.rs` 顶部 `mod view;` 附近加声明(`pub(crate) mod view_model;` 之后):

```rust
pub(crate) mod protocol;
pub(crate) use protocol::*;
```

创建 `protocol.rs`,先只放 `use` 与测试(类型尚未定义,编译会失败——这就是"红"):

```rust
//! 代码健康度内容侧 webview 推送协议:把 `WorkspaceState` 算成要序列化推给
//! webview 的 `CodeHealthViewPayload`。前端只渲染,不做聚合/排序/差异计算——
//! 这些全在 `view_model.rs`/`aggregate.rs`/`git_hotspots.rs`。
//! 形态仿 `extensions/usage/protocol.rs`(声明式比较 + 单槽)。

use super::view_model::{self, ChangeSummary, EmptyState};
use super::{CodeHealthCategory, WorkspaceState};
use dozer_codehealth::{FindingChange, FindingEvidence, FindingSeverity, HealthTier, rule_ids};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{
        ArchitectureReport, Finding, FindingCategory, HealthTier, ProjectReport, SCHEMA_VERSION,
        ScanMetadata, ScanStatus,
    };

    fn sample_report() -> ProjectReport {
        ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                status: ScanStatus::Complete,
                analyzed_files: 3,
                ..ScanMetadata::default()
            },
            git: None,
            findings: vec![],
            architecture: ArchitectureReport::not_applicable(),
            total_loc: 100,
            total_functions: 5,
            critical_functions: 1,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Watch,
            overall_tier: HealthTier::Watch,
            functions: vec![],
            color_findings: vec![],
            color_tier: HealthTier::Healthy,
            spacing_findings: vec![],
            spacing_tier: HealthTier::Healthy,
            font_findings: vec![],
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: vec![],
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }

    fn structure_finding(id: &str, signal: usize) -> Finding {
        Finding {
            id: id.into(),
            rule_id: rule_ids::STRUCTURE_COMPLEXITY.into(),
            category: FindingCategory::Structure,
            severity: FindingSeverity::Critical,
            path: PathBuf::from("src/a.rs"),
            start_line: 7,
            symbol: Some("run".into()),
            title: "run".into(),
            evidence: FindingEvidence::Structure {
                complexity_signal: signal,
                loc: 40,
                widget_nesting_depth: 0,
                event_handler_count: 0,
            },
            applicability: Default::default(),
        }
    }

    fn ws_with(report: ProjectReport) -> WorkspaceState {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Loaded(
                1,
                Box::new(super::super::PanelState {
                    report: Some(report),
                    scanned_at_ms: Some(0),
                    ..Default::default()
                }),
            ),
        );
        ws
    }

    #[test]
    fn no_report_yields_empty_never_scanned() {
        let p = current_view_payload(&WorkspaceState::default());
        assert!(matches!(p.body, Body::Empty { .. }));
        assert!(!p.scan.scanning);
        assert_eq!(p.scan.scanned_at, None);
    }

    #[test]
    fn scanning_without_report_says_scanning() {
        let mut ws = WorkspaceState::default();
        super::super::update(&mut ws, super::super::Message::ScanRequested);
        match current_view_payload(&ws).body {
            Body::Empty { message } => assert_eq!(message, "扫描中…"),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn scan_error_without_report_reports_failure() {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Scanned(1, Err("boom".into())),
        );
        let p = current_view_payload(&ws);
        assert_eq!(p.scan.scan_error.as_deref(), Some("boom"));
        match p.body {
            Body::Empty { message } => assert_eq!(message, "扫描失败。"),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn overview_first_scan_has_first_scan_change_and_tier() {
        let ws = ws_with(sample_report());
        let p = current_view_payload(&ws);
        assert_eq!(p.category, CategoryKey::Overview);
        assert_eq!(p.scan.scanned_at.as_deref(), Some("1970-01-01 00:00:00 UTC"));
        match p.body {
            Body::Overview(o) => {
                assert_eq!(o.tier_label, "需要关注");
                assert_eq!(o.summary, "核心代码 100 行 · 1 个函数存在明显结构问题");
                assert!(matches!(o.change, ChangeDto::FirstScan));
                assert!(o.priorities.is_empty());
            }
            other => panic!("expected Overview, got {other:?}"),
        }
    }

    #[test]
    fn structure_page_carries_all_structure_findings_with_titles() {
        let mut report = sample_report();
        report.findings.push(structure_finding("f1", 12));
        let mut ws = ws_with(report);
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::Structure),
        );
        match current_view_payload(&ws).body {
            Body::Structure(s) => {
                assert_eq!(s.metric_note, "指标口径:控制流信号(非标准圈复杂度)");
                assert_eq!(s.findings.len(), 1);
                assert_eq!(s.findings[0].title, "run 控制流信号 12");
                assert_eq!(s.findings[0].path, "src/a.rs");
                assert_eq!(s.findings[0].line, 7);
                assert_eq!(s.findings[0].severity_label, "警戒");
                assert_eq!(s.findings[0].change, None);
            }
            other => panic!("expected Structure, got {other:?}"),
        }
    }

    #[test]
    fn ui_consistency_not_applicable_without_iced() {
        let mut ws = ws_with(sample_report());
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::UiConsistency),
        );
        match current_view_payload(&ws).body {
            Body::UiConsistency(u) => {
                assert!(!u.applicable);
                assert!(u.groups.is_empty());
            }
            other => panic!("expected UiConsistency, got {other:?}"),
        }
    }

    #[test]
    fn scan_scope_reports_counts_and_status() {
        let mut ws = ws_with(sample_report());
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::ScanScope),
        );
        match current_view_payload(&ws).body {
            Body::ScanScope(s) => {
                assert_eq!(s.status_label, "完整");
                assert_eq!(s.analyzed_files, 3);
                assert_eq!(s.schema_version, SCHEMA_VERSION);
                assert!(s.skipped.is_empty());
            }
            other => panic!("expected ScanScope, got {other:?}"),
        }
    }

    #[test]
    fn payload_serializes_with_kind_tags() {
        let ws = ws_with(sample_report());
        let json = serde_json::to_value(current_view_payload(&ws)).unwrap();
        assert_eq!(json["category"], "overview");
        assert_eq!(json["body"]["kind"], "overview");
        assert_eq!(json["body"]["change"]["kind"], "first_scan");
        assert_eq!(json["scan"]["scanning"], false);
    }

    // ---- 事件 ----

    #[test]
    fn parses_all_event_kinds() {
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"ready"}"#).unwrap(),
            CodeHealthWebviewEvent::Ready
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"scan_requested"}"#).unwrap(),
            CodeHealthWebviewEvent::ScanRequested
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"open_location","path":"src/a.rs","line":7}"#)
                .unwrap(),
            CodeHealthWebviewEvent::OpenLocation {
                path: PathBuf::from("src/a.rs"),
                line: 7
            }
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"analyze_finding","id":"f1"}"#).unwrap(),
            CodeHealthWebviewEvent::AnalyzeFinding { id: "f1".into() }
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"failed","reason":"x"}"#).unwrap(),
            CodeHealthWebviewEvent::Failed { reason: "x".into() }
        );
        assert!(parse_codehealth_event("not json").is_err());
        assert!(parse_codehealth_event(r#"{"kind":"nope"}"#).is_err());
    }

    /// Review Focus 1:webview 发来的路径不能是绝对路径或含 `..`。
    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(is_safe_relative_path(Path::new("src/a.rs")));
        assert!(is_safe_relative_path(Path::new("crates/x/src/lib.rs")));
        assert!(!is_safe_relative_path(Path::new("/etc/passwd")));
        assert!(!is_safe_relative_path(Path::new("../outside.rs")));
        assert!(!is_safe_relative_path(Path::new("src/../../outside.rs")));
        assert!(!is_safe_relative_path(Path::new("")));
    }

    // ---- 推送状态 ----

    fn payload() -> CodeHealthViewPayload {
        current_view_payload(&WorkspaceState::default())
    }

    #[test]
    fn pending_push_requires_ready_and_changes() {
        let mut s = WebviewPushState::default();
        assert_eq!(s.pending_push(&payload()), None, "未 ready 不推");
        s.set_ready(true);
        let p = payload();
        assert_eq!(s.pending_push(&p), Some(p.clone()));
        let rev = s.mark_sent(p.clone());
        assert_eq!(rev, 1);
        assert_eq!(s.pending_push(&p), None, "同值不重推");
    }

    /// Review Focus 6:webview 重建后新实例第一次 ready 必须强制重发。
    #[test]
    fn set_ready_true_forces_resend() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        let p = payload();
        s.mark_sent(p.clone());
        assert_eq!(s.pending_push(&p), None);
        s.set_ready(true);
        assert_eq!(s.pending_push(&p), Some(p));
    }

    /// Review Focus 3:revision 单调递增。
    #[test]
    fn revision_increments_per_send() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        assert_eq!(s.mark_sent(payload()), 1);
        assert_eq!(s.mark_sent(payload()), 2);
    }

    /// Review Focus 2:一直不 ready 超时 → failed。
    #[test]
    fn never_ready_times_out_to_failed() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, t0);
        assert_eq!(s.failed(), None);
        s.observe_availability(true, t0 + READY_TIMEOUT + Duration::from_millis(1));
        assert!(s.failed().is_some());
        s.clear_failed();
        assert_eq!(s.failed(), None);
    }

    #[test]
    fn ready_before_timeout_never_fails() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, t0);
        s.set_ready(true);
        s.observe_availability(true, t0 + READY_TIMEOUT * 10);
        assert_eq!(s.failed(), None);
    }

    #[test]
    fn unavailable_resets_ready() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        s.observe_availability(false, Instant::now());
        assert_eq!(s.pending_push(&payload()), None);
    }

    #[test]
    fn envelope_carries_version_and_revision() {
        let json: serde_json::Value =
            serde_json::from_str(&encode_codehealth_push(5, payload())).unwrap();
        assert_eq!(json["protocol_version"], CODEHEALTH_PROTOCOL_VERSION);
        assert_eq!(json["revision"], 5);
        assert!(json["payload"]["scan"].is_object());
    }
}
```

- [ ] **Step 3: 运行测试确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app extensions::codehealth::protocol 2>&1 | tail -20
```

Expected: 编译失败,报 `cannot find type CodeHealthViewPayload` / `current_view_payload` 等未定义。

- [ ] **Step 4: 实现协议类型与 `current_view_payload`**

在 `protocol.rs` 的 `use` 之后、`#[cfg(test)]` 之前写入:

```rust
pub const CODEHEALTH_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 面板分类(JSON 里的小写下划线形式)。阶段二会加 `Architecture`。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKey {
    Overview,
    Structure,
    UiConsistency,
    ScanScope,
}

impl From<CodeHealthCategory> for CategoryKey {
    fn from(c: CodeHealthCategory) -> Self {
        match c {
            CodeHealthCategory::Overview => CategoryKey::Overview,
            CodeHealthCategory::Structure => CategoryKey::Structure,
            CodeHealthCategory::UiConsistency => CategoryKey::UiConsistency,
            CodeHealthCategory::ScanScope => CategoryKey::ScanScope,
        }
    }
}

/// 页头(扫描按钮 + 状态条)需要的数据。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScanBar {
    pub scanning: bool,
    /// 已格式化的"2026-09-20 07:58:06 UTC";`None` = 尚未扫描。
    pub scanned_at: Option<String>,
    pub scan_error: Option<String>,
    pub save_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeverityKey {
    Critical,
    Watch,
}

impl From<FindingSeverity> for SeverityKey {
    fn from(s: FindingSeverity) -> Self {
        match s {
            FindingSeverity::Critical => SeverityKey::Critical,
            FindingSeverity::Watch => SeverityKey::Watch,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKey {
    New,
    Worsened,
    Improved,
    Persisting,
    Resolved,
}

impl From<FindingChange> for ChangeKey {
    fn from(c: FindingChange) -> Self {
        match c {
            FindingChange::New => ChangeKey::New,
            FindingChange::Worsened => ChangeKey::Worsened,
            FindingChange::Improved => ChangeKey::Improved,
            FindingChange::Persisting => ChangeKey::Persisting,
            FindingChange::Resolved => ChangeKey::Resolved,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TierKey {
    Healthy,
    Watch,
    Critical,
}

impl From<HealthTier> for TierKey {
    fn from(t: HealthTier) -> Self {
        match t {
            HealthTier::Healthy => TierKey::Healthy,
            HealthTier::Watch => TierKey::Watch,
            HealthTier::Critical => TierKey::Critical,
        }
    }
}

/// 一条发现行(总览优先处理、结构复杂度、UI 一致性共用)。文案在 Rust 里
/// 算好,前端不拼业务文案。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FindingRowDto {
    pub id: String,
    pub title: String,
    /// 相对项目根的路径(与报告里的 `Finding.path` 一致)。
    pub path: String,
    pub line: usize,
    pub severity: SeverityKey,
    pub severity_label: &'static str,
    pub change: Option<ChangeKey>,
    pub change_label: Option<&'static str>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeDto {
    FirstScan,
    HasChange {
        loc_delta_text: String,
        functions_delta_text: String,
        new_risks: usize,
        resolved_risks: usize,
        worsened: usize,
        improved: usize,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OverviewBody {
    /// "main · abc123 · 有未提交改动";无 Git 时为 `None`。
    pub git_line: Option<String>,
    pub tier: TierKey,
    pub tier_label: &'static str,
    pub summary: String,
    pub change: ChangeDto,
    pub priorities: Vec<FindingRowDto>,
    /// "已分析 N 个文件 · … · 排除 N · 跳过 N";无摘要时为空串。
    pub scope_line: String,
    pub legacy_note: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StructureBody {
    pub metric_note: &'static str,
    /// 全部结构复杂度发现;「本轮新增 / 全部」筛选由前端按 `change` 本地做
    /// (`new`/`worsened` 算本轮新增,`change == None`(无差异数据)不算)。
    pub findings: Vec<FindingRowDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiGroup {
    pub title: &'static str,
    pub findings: Vec<FindingRowDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiBody {
    pub applicable: bool,
    pub groups: Vec<UiGroup>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkippedDto {
    pub path: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScopeBody {
    pub status_label: &'static str,
    pub analyzed_files: usize,
    pub excluded_files: usize,
    pub skipped_count: usize,
    pub languages_detail: String,
    pub duration_ms: u64,
    pub schema_version: u32,
    pub git_baseline: Option<String>,
    pub skipped: Vec<SkippedDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Empty { message: String },
    Overview(OverviewBody),
    Structure(StructureBody),
    UiConsistency(UiBody),
    ScanScope(ScopeBody),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodeHealthViewPayload {
    pub scan: ScanBar,
    pub category: CategoryKey,
    pub body: Body,
}

fn finding_change(ws: &WorkspaceState, id: &str) -> Option<FindingChange> {
    ws.diff().map(|d| {
        if d.new.iter().any(|x| x.id == id) {
            FindingChange::New
        } else if d.worsened.iter().any(|x| x.id == id) {
            FindingChange::Worsened
        } else if d.improved.iter().any(|x| x.id == id) {
            FindingChange::Improved
        } else {
            FindingChange::Persisting
        }
    })
}

fn row_dto(
    id: String,
    title: String,
    path: &Path,
    line: usize,
    severity: FindingSeverity,
    change: Option<FindingChange>,
    reasons: Vec<String>,
) -> FindingRowDto {
    FindingRowDto {
        id,
        title,
        path: path.to_string_lossy().into_owned(),
        line,
        severity: severity.into(),
        severity_label: view_model::sev_label(severity),
        change: change.map(Into::into),
        change_label: change.map(view_model::change_label),
        reasons,
    }
}

fn overview_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> OverviewBody {
    let git_line = ws.git().map(|git| {
        let branch = git.branch.clone().unwrap_or_else(|| "detached".to_string());
        let head = git.head.clone().unwrap_or_default();
        let mut s = format!("{branch} · {head}");
        if git.dirty {
            s.push_str(" · 有未提交改动");
        }
        s
    });
    let scope_line = view_model::scope_summary(ws)
        .map(|s| {
            format!(
                "已分析 {} 个文件 · {} · 排除 {} · 跳过 {}",
                s.analyzed_files, s.languages, s.excluded_files, s.skipped_files
            )
        })
        .unwrap_or_default();
    let change = match view_model::change_summary(report, ws.previous_report(), ws.diff()) {
        ChangeSummary::FirstScan => ChangeDto::FirstScan,
        ChangeSummary::HasChange(c) => ChangeDto::HasChange {
            loc_delta_text: view_model::signed_delta(c.loc_delta),
            functions_delta_text: view_model::signed_delta(c.functions_delta),
            new_risks: c.new_risks,
            resolved_risks: c.resolved_risks,
            worsened: c.worsened,
            improved: c.improved,
        },
    };
    let priorities = view_model::hotspot_rows(ws.hotspots(), 50)
        .into_iter()
        .map(|r| {
            row_dto(
                r.finding_id,
                r.title,
                &r.path,
                r.line,
                r.severity,
                Some(r.change),
                r.reasons,
            )
        })
        .collect();
    OverviewBody {
        git_line,
        tier: report.overall_tier.into(),
        tier_label: view_model::tier_label(report.overall_tier),
        summary: format!(
            "核心代码 {} 行 · {} 个函数存在明显结构问题",
            report.total_loc, report.critical_functions
        ),
        change,
        priorities,
        scope_line,
        legacy_note: view_model::legacy_report_note(report.schema_version),
    }
}

fn structure_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> StructureBody {
    let findings = report
        .findings
        .iter()
        .filter(|f| f.rule_id == rule_ids::STRUCTURE_COMPLEXITY)
        .map(|f| {
            let metric = match &f.evidence {
                FindingEvidence::Structure {
                    complexity_signal, ..
                } => *complexity_signal,
                _ => 0,
            };
            row_dto(
                f.id.clone(),
                format!("{} 控制流信号 {}", f.symbol.as_deref().unwrap_or("?"), metric),
                &f.path,
                f.start_line,
                f.severity,
                finding_change(ws, &f.id),
                Vec::new(),
            )
        })
        .collect();
    StructureBody {
        metric_note: "指标口径:控制流信号(非标准圈复杂度)",
        findings,
    }
}

fn ui_body(report: &dozer_codehealth::ProjectReport) -> UiBody {
    if !view_model::ui_consistency_applicable(report) {
        return UiBody {
            applicable: false,
            groups: Vec::new(),
        };
    }
    let rules: [(&'static str, &str); 6] = [
        ("颜色硬编码", rule_ids::COLOR_HARDCODE),
        ("边距硬编码", rule_ids::SPACING_HARDCODE),
        ("字体硬编码", rule_ids::FONT_HARDCODE),
        ("组件树嵌套深度", rule_ids::NESTING_DEPTH),
        ("事件回调密度", rule_ids::EVENT_HANDLER_DENSITY),
        ("组件化重复结构", rule_ids::DUPLICATE_STRUCTURE),
    ];
    let groups = rules
        .iter()
        .map(|(title, rule)| UiGroup {
            title,
            findings: report
                .findings
                .iter()
                .filter(|f| f.rule_id == *rule)
                .map(|f| {
                    row_dto(
                        f.id.clone(),
                        f.title.clone(),
                        &f.path,
                        f.start_line,
                        f.severity,
                        None,
                        Vec::new(),
                    )
                })
                .collect(),
        })
        .collect();
    UiBody {
        applicable: true,
        groups,
    }
}

fn scope_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> ScopeBody {
    let scan = &report.scan;
    let status_label = match scan.status {
        dozer_codehealth::ScanStatus::Complete => "完整",
        dozer_codehealth::ScanStatus::Partial => "部分完成",
        dozer_codehealth::ScanStatus::Failed => "失败",
    };
    let languages_detail = scan
        .languages
        .iter()
        .map(|l| {
            if l.analyzed {
                format!("{}({} 个文件,结构分析)", l.language, l.files)
            } else {
                format!("{}({} 个文件,仅统计)", l.language, l.files)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let git_baseline = ws.git().map(|git| {
        format!(
            "{}{}",
            git.branch.clone().unwrap_or_else(|| "detached".into()),
            git.head
                .clone()
                .map(|h| format!(" @ {h}"))
                .unwrap_or_default()
        )
    });
    let skipped = scan
        .skipped_files
        .iter()
        .map(|s| SkippedDto {
            path: s.path.to_string_lossy().into_owned(),
            reason: match s.reason {
                dozer_codehealth::SkipReason::Ignored => "忽略规则",
                dozer_codehealth::SkipReason::UnsupportedLanguage => "不支持的语言",
                dozer_codehealth::SkipReason::Generated => "生成文件",
                dozer_codehealth::SkipReason::NonUtf8 => "非 UTF-8",
                dozer_codehealth::SkipReason::ReadFailed => "读取失败",
                dozer_codehealth::SkipReason::ParseFailed => "解析失败",
            },
        })
        .collect();
    ScopeBody {
        status_label,
        analyzed_files: scan.analyzed_files,
        excluded_files: scan.excluded_files,
        skipped_count: scan.skipped_files.len(),
        languages_detail,
        duration_ms: scan.duration_ms,
        schema_version: report.schema_version,
        git_baseline,
        skipped,
    }
}

fn empty_message(ws: &WorkspaceState) -> String {
    if ws.scanning() {
        return "扫描中…".to_string();
    }
    match view_model::empty_state(ws) {
        EmptyState::NeverScanned => "这个项目还没有扫描过。",
        EmptyState::NoSupportedCode => "没有发现受支持的代码。",
        EmptyState::PartialFailure => "扫描部分完成,部分文件失败(见扫描范围)。",
        EmptyState::ScanFailed => "扫描失败。",
    }
    .to_string()
}

/// 当前该显示什么。**唯一权威实现**:`view.rs` 旧渲染删除后不再有第二份判定。
pub fn current_view_payload(ws: &WorkspaceState) -> CodeHealthViewPayload {
    let scan = ScanBar {
        scanning: ws.scanning(),
        scanned_at: ws.scanned_at_ms().map(view_model::format_ms),
        scan_error: ws.scan_error().map(str::to_owned),
        save_error: ws.save_error().map(str::to_owned),
    };
    let category: CategoryKey = ws.category().into();
    let body = match ws.report() {
        None => Body::Empty {
            message: empty_message(ws),
        },
        Some(report) => match category {
            CategoryKey::Overview => Body::Overview(overview_body(ws, report)),
            CategoryKey::Structure => Body::Structure(structure_body(ws, report)),
            CategoryKey::UiConsistency => Body::UiConsistency(ui_body(report)),
            CategoryKey::ScanScope => Body::ScanScope(scope_body(ws, report)),
        },
    };
    CodeHealthViewPayload {
        scan,
        category,
        body,
    }
}

// ---------------------------------------------------------------------------
// webview → Rust 事件
// ---------------------------------------------------------------------------

/// webview 发回的事件。不复用 `preview::webview_protocol::EditorEvent`
/// (那套带 CodeMirror 专属变体)。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeHealthWebviewEvent {
    Ready,
    ScanRequested,
    OpenLocation { path: PathBuf, line: usize },
    AnalyzeFinding { id: String },
    Failed { reason: String },
}

pub fn parse_codehealth_event(body: &str) -> Result<CodeHealthWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// webview 内容不可信任到能指定任意路径:只接受非空、无 `..`、非绝对的相对路径
/// (发现项的路径本来就是相对项目根的规范化路径)。
pub fn is_safe_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

// ---------------------------------------------------------------------------
// Rust → webview 推送
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CodeHealthPushEnvelope {
    pub protocol_version: u32,
    /// 单调递增;前端丢弃小于已应用值的推送。
    pub revision: u64,
    pub payload: CodeHealthViewPayload,
}

pub fn encode_codehealth_push(revision: u64, payload: CodeHealthViewPayload) -> String {
    serde_json::to_string(&CodeHealthPushEnvelope {
        protocol_version: CODEHEALTH_PROTOCOL_VERSION,
        revision,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

/// App 级(固定单槽,不按项目分)内容侧 webview 推送状态。纯状态,可单测,
/// 不碰 webview 池——调用方(`App::take_codehealth_content_script`)据
/// `pending_push` 组 envelope,真正注入后调 `mark_sent`。
#[derive(Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<CodeHealthViewPayload>,
    revision: u64,
    /// webview 已进池但尚未 ready 的起点,用于判定加载超时。
    waiting_since: Option<Instant>,
    /// 加载失败/超时原因;`Some` 时内容区回落原生占位页(重试清除)。
    failed: Option<String>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发当前内容。
            self.last_sent = None;
            self.waiting_since = None;
        }
    }

    /// 每帧由消费点调用:`available` = webview 当前在池里。
    pub fn observe_availability(&mut self, available: bool, now: Instant) {
        if !available {
            self.ready = false;
            self.waiting_since = None;
            return;
        }
        if self.ready {
            self.waiting_since = None;
            return;
        }
        let since = *self.waiting_since.get_or_insert(now);
        if self.failed.is_none() && now.duration_since(since) > READY_TIMEOUT {
            self.failed = Some("面板页面加载超时".to_string());
        }
    }

    pub fn pending_push(&self, desired: &CodeHealthViewPayload) -> Option<CodeHealthViewPayload> {
        if !self.ready || self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    /// 记录已送达并返回本次 revision(从 1 起)。
    pub fn mark_sent(&mut self, payload: CodeHealthViewPayload) -> u64 {
        self.revision += 1;
        self.last_sent = Some(payload);
        self.revision
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn set_failed(&mut self, reason: String) {
        self.failed = Some(reason);
        self.ready = false;
        self.last_sent = None;
        self.waiting_since = None;
    }

    pub fn clear_failed(&mut self) {
        self.failed = None;
        self.waiting_since = None;
    }
}
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app extensions::codehealth 2>&1 | tail -25
```

Expected: `test result: ok.`,包含 `protocol::tests::*` 全部与 `view_model::tests::format_ms_*`。若 `StructureBody.metric_note` 的全角/半角标点与测试不一致,以代码为准统一成半角冒号与括号(测试与实现必须逐字符一致)。

- [ ] **Step 6: clippy + 提交**

```bash
cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app/src/extensions/codehealth/protocol.rs crates/dozer-app/src/extensions/codehealth/mod.rs crates/dozer-app/src/extensions/codehealth/view_model.rs
git diff --cached --stat
git commit -m "feat(codehealth): add webview push protocol and view payload

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: clippy 无 error(有 `dead_code` warning 可接受,Task 4 接线后消失)。`--stat` 只含上述三个文件。

---

### Task 2: 几何——抽出共享函数并新增 `codehealth_content_pane_bounds_for`

**Files:**
- Modify: `crates/dozer-app/src/webview_geometry.rs`(`usage_content_pane_bounds_for` 约 544–652 行;测试区约 1490 行后)

**Interfaces:**
- Consumes: 现有 `pair_content_width`、`pair_columns`、`left_zone_width`、`right_zone_width`、`maximized_box_x_range`、`maximized_box_height`、`ShellState`、`Side`、`MaximizedPane`。
- Produces: `pub fn codehealth_content_pane_bounds_for(side: Side, window_width: f32, window_height: f32, state: &ShellState, content_desired: bool) -> (f32, f32, f32, f32)`;私有 `struct PairPane`、`fn pair_content_pane_bounds_for(side, window_width, window_height, state, pane: PairPane)`。`usage_content_pane_bounds_for` 签名与行为**不变**(现有 8 个测试是护栏)。

- [ ] **Step 1: 写失败测试**

在 `webview_geometry.rs` 的 `mod tests` 里、紧跟 `usage_content_maximized_stays_clear_of_overlay_corners` 测试之后追加:

```rust
    #[test]
    fn codehealth_content_zero_when_not_desired() {
        let state = ShellState {
            left_view: PanelKind::CodeHealth,
            ..test_state()
        };
        let (_, _, w, h) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, false);
        assert_eq!((w, h), (0.0, 0.0));
    }

    #[test]
    fn codehealth_content_zero_when_side_collapsed() {
        let state = ShellState {
            left_view: PanelKind::CodeHealth,
            left_collapsed: true,
            ..test_state()
        };
        let (_, _, w, h) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert_eq!((w, h), (0.0, 0.0));
    }

    #[test]
    fn codehealth_content_zero_when_panel_kind_is_not_codehealth() {
        let state = test_state(); // left_view: PanelKind::Files
        let (_, _, w, h) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert_eq!((w, h), (0.0, 0.0));
    }

    /// 内容 | 分隔线 | 分类导航 两栏恒存在,所以 webview 宽度必须严格小于整区宽度。
    #[test]
    fn codehealth_content_is_narrower_than_zone_and_follows_split() {
        let state = ShellState {
            left_view: PanelKind::CodeHealth,
            ..test_state()
        };
        let (_, _, w, h) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert!(w > 100.0 && h > 100.0, "w={w} h={h}");
        let mut wider_list = state.clone();
        wider_list.dims.codehealth_split = (state.dims.codehealth_split + 0.2).min(0.9);
        let (_, _, w2, _) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &wider_list, true);
        assert!(w2 != w, "拖动分栏后宽度应变化: {w} vs {w2}");
    }

    /// 本面板内容列顶部没有原生面板头(分类导航那一列才有),webview 紧贴区顶。
    #[test]
    fn codehealth_content_y_starts_at_zone_top() {
        let state = ShellState {
            left_view: PanelKind::CodeHealth,
            ..test_state()
        };
        let (_, y, _, _) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        let zone_top = byteui::theme::geometry::top_bar_height()
            + theme::region::left_zone().margin.top;
        assert!((y - zone_top).abs() < 1.0, "y={y} zone_top={zone_top}");
    }

    #[test]
    fn codehealth_content_maximized_stays_inside_box() {
        let state = ShellState {
            left_view: PanelKind::CodeHealth,
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, y, w, h) =
            codehealth_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert!(w > 100.0 && h > 100.0, "w={w} h={h}");
        let (x0, avail_w) = maximized_box_x_range(1600.0);
        assert!(x >= x0 && x + w <= x0 + avail_w, "x={x} w={w}");
        assert!(y + h <= maximized_box_height(900.0) + 200.0);
    }
```

> `ShellState` 需要 `Clone`:若 `state.clone()` 编译报 `Clone` 未实现,把测试里 `let mut wider_list = state.clone();` 改成重新构造一个 `ShellState { dims: PanelDims { codehealth_split: ..., ..PanelDims::default() }, left_view: PanelKind::CodeHealth, ..test_state() }`。

- [ ] **Step 2: 运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app webview_geometry::tests::codehealth 2>&1 | tail -10
```

Expected: 编译失败 `cannot find function codehealth_content_pane_bounds_for`。

- [ ] **Step 3: 抽共享函数**

对 `usage_content_pane_bounds_for`(`webview_geometry.rs` 约 544 行起)做**机械改写**,三步:

(a) 在它**上方**(其文档注释之前)加参数结构体:

```rust
/// "内容在前、列表在后"两栏面板(Usage、CodeHealth)的内容侧 webview 几何参数。
struct PairPane {
    kind: PanelKind,
    /// 列表列占比(`dims.usage_split` / `dims.codehealth_split`)。
    split: f32,
    /// 内容顶部要让出的原生面板头高度。
    chrome_top: f32,
    content_desired: bool,
    list_visible: bool,
}
```

(b) 把原函数整体改名为私有的 `pair_content_pane_bounds_for`,签名改为:

```rust
fn pair_content_pane_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
    pane: PairPane,
) -> (f32, f32, f32, f32) {
```

函数体内只做 5 处替换(其余一字不动,包括所有注释):

| 原文 | 改为 |
|---|---|
| `if !content_desired \|\| collapsed \|\| kind != PanelKind::Usage {` | `if !pane.content_desired \|\| collapsed \|\| kind != pane.kind {` |
| `state.layout.rail_layout.side_of(PanelKind::Usage) != PanelKind::Usage.default_side()` | `state.layout.rail_layout.side_of(pane.kind) != pane.kind.default_side()` |
| `let chrome_top = theme::geometry::usage_content_chrome_top_px();` | `let chrome_top = pane.chrome_top;` |
| `if list_visible {`(`compute` 闭包里) | `if pane.list_visible {` |
| `pair_columns(pair_w, state.dims.usage_split, !mirrored)` | `pair_columns(pair_w, pane.split, !mirrored)` |

(c) 在其**下方**(原函数结束的 `}` 之后)新增两个公开包装,其中 `usage_content_pane_bounds_for` 保留**原来的文档注释与签名**:

```rust
pub fn usage_content_pane_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
    content_desired: bool,
    list_visible: bool,
) -> (f32, f32, f32, f32) {
    pair_content_pane_bounds_for(
        side,
        window_width,
        window_height,
        state,
        PairPane {
            kind: PanelKind::Usage,
            split: state.dims.usage_split,
            chrome_top: theme::geometry::usage_content_chrome_top_px(),
            content_desired,
            list_visible,
        },
    )
}

/// 代码健康度内容侧 webview 矩形。与用量面板同构("内容在前、列表在后"),
/// 差别:分类导航列恒存在(`list_visible = true`),内容列顶部没有原生面板头
/// (`chrome_top = 0`,原生 `content_pane` 不画头),分栏占比用
/// `dims.codehealth_split`。不可摆放(`!content_desired` / 该侧收起 / 不是
/// CodeHealth / 放大的是另一侧)时返回零尺寸矩形。
pub fn codehealth_content_pane_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
    content_desired: bool,
) -> (f32, f32, f32, f32) {
    pair_content_pane_bounds_for(
        side,
        window_width,
        window_height,
        state,
        PairPane {
            kind: PanelKind::CodeHealth,
            split: state.dims.codehealth_split,
            chrome_top: 0.0,
            content_desired,
            list_visible: true,
        },
    )
}
```

并把原文档注释里属于"Usage 专属"的段落留在 `usage_content_pane_bounds_for` 上,共享函数 `pair_content_pane_bounds_for` 上加一行 `/// 共享实现,见 [`usage_content_pane_bounds_for`] / [`codehealth_content_pane_bounds_for`]。`。

- [ ] **Step 4: 运行测试(新测试 + 用量护栏)**

```bash
cargo test -p dozer-app webview_geometry 2>&1 | tail -12
```

Expected: `test result: ok.`,其中 `usage_content_*`(8 个)与 `codehealth_content_*`(6 个)均通过。若 `codehealth_content_maximized_stays_inside_box` 的最后一条断言过松/过紧导致失败,把它收紧为与 `usage_content_maximized_stays_clear_of_overlay_corners` 同款的盒子边界断言(读那个测试照抄推导),不要删掉断言。

- [ ] **Step 5: 提交**

```bash
cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app/src/webview_geometry.rs
git diff --cached --stat
git commit -m "feat(webview_geometry): shared pair pane bounds + codehealth content bounds

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: assets 路由 `dozer://codehealth-content/`

**Files:**
- Modify: `crates/dozer-app/src/assets.rs`(约 173 行、265 行、测试区约 437 行后)

**Interfaces:**
- Produces: `dozer://codehealth-content/<file>` 从 `assets/codehealth-content/` 服务;测试 `codehealth_content_bundle_assets_are_present`(依赖 Task 5 的构建产物,**本任务先写出会失败的该测试,Task 5 之后转绿**——为避免中间提交红,本任务只提交路由与服务类测试,"产物齐全"测试放到 Task 5)。

- [ ] **Step 1: 写失败测试**

在 `assets.rs` 的 `usage_content_unknown_subpath_404` 测试之后追加:

```rust
    #[test]
    fn codehealth_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("codehealth-content")).unwrap();
        std::fs::write(
            root.with_file_name("codehealth-content").join("host.html"),
            b"<html>c</html>",
        )
        .unwrap();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://codehealth-content/host.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>c</html>");
    }

    #[test]
    fn codehealth_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("codehealth-content")).unwrap();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://codehealth-content/nope",
        );
        assert_eq!(r.status, 404);
    }

    /// 路径穿越不得逃出 codehealth-content 根。
    #[test]
    fn codehealth_content_rejects_path_traversal() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("codehealth-content")).unwrap();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://codehealth-content/../usage-content/host.html",
        );
        assert_eq!(r.status, 404);
    }
```

- [ ] **Step 2: 运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app assets::tests::codehealth 2>&1 | tail -10
```

Expected: 3 个测试 FAIL(未路由,返回 404;`serves_vendored_files` 断言 200 失败)。

- [ ] **Step 3: 加路由**

在 `usage_content_root_for` 函数之后加:

```rust
/// codehealth-content host(代码健康度内容侧)静态资源根 = flyfish 根的兄弟
/// 目录 `codehealth-content`。同 `usage_content_root_for`。
fn codehealth_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("codehealth-content")
}
```

在 `handle_protocol` 里 `usage-content/` 分支之后加:

```rust
    // codehealth-content host(代码健康度内容侧):同 usage-content,没有 data.json
    // 特判,数据全靠 evaluate_script 推送。
    if let Some(path) = rest.strip_prefix("codehealth-content/") {
        return serve_vendored(&codehealth_content_root_for(assets_root), path);
    }
```

同时把 `handle_protocol` 里那行注释 `html/usage-content 这些命名空间` 改成 `html/usage-content/codehealth-content 这些命名空间`。

- [ ] **Step 4: 运行确认通过**

```bash
cargo test -p dozer-app assets::tests 2>&1 | tail -8
```

Expected: `test result: ok.`(路径穿越测试依赖 `serve_vendored` 已有的穿越防护;若该测试失败,说明 `serve_vendored` 没挡 `..`——此时**停下**,读 `serve_vendored` 补防护并加测试,不要删本测试)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/assets.rs
git diff --cached --stat
git commit -m "feat(assets): serve dozer://codehealth-content host

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: App 接线(常量、状态、消息、IPC、推送消费点、`preview_desired`、原生失败占位)

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`(常量约 652 行;字段约 468、844 行;`take_usage_content_script` 之后;`preview_desired` 的 Usage 分支之后)
- Modify: `crates/dozer-app/src/app/message.rs`(约 626 行)
- Modify: `crates/dozer-app/src/app/update.rs`(约 148 行、1503 行)
- Modify: `crates/dozer-app/src/runtime.rs`(约 554 行)
- Modify: `crates/dozer-app/src/platform/window_events.rs`(约 2555 行)
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`(`Message::ContentRetry`)
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`(`content_pane` 新签名,**暂时保留旧渲染函数不删**)
- Modify: `crates/dozer-app/src/app/view.rs`(约 1116 行调用点)
- Modify: `docs/superpowers/specs/2026-10-01-code-health-webview-design.md`(订正两处)

**Interfaces:**
- Consumes: Task 1 的 `current_view_payload`、`WebviewPushState`、`parse_codehealth_event`、`encode_codehealth_push`、`is_safe_relative_path`;Task 2 的 `codehealth_content_pane_bounds_for`;Task 3 的 `dozer://codehealth-content/host.html`。
- Produces: `App::codehealth_webview: WebviewPushState`;`CODEHEALTH_CONTENT_ID_OFFSET = 5_000_000`;`Message::CodeHealthContentWebviewEvent(CodeHealthWebviewEvent)`;`codehealth::Message::ContentRetry`;`App::code_health_request_scan(&mut self)`;`App::take_codehealth_content_script(&mut self, &HashSet<usize>, Instant) -> Vec<(usize, String)>`。

> 本任务改动集中在接线,逻辑都已在 Task 1 单测过;验证以"全量编译 + 现有测试不退化 + 新增 2 个纯函数测试"为主,GUI 行为在 Task 7 统一人工验收。

- [ ] **Step 1: 写失败测试(`ContentRetry` 与失败占位文案)**

在 `extensions/codehealth/mod.rs` 的 `mod tests` 末尾追加:

```rust
    #[test]
    fn content_retry_message_is_kernel_intercepted() {
        // 与 OpenLocation/AnalyzeFinding 同款:由内核拦截,不进本模块 update。
        let result = std::panic::catch_unwind(|| {
            let mut ws = WorkspaceState::default();
            update(&mut ws, Message::ContentRetry);
        });
        assert!(result.is_err(), "ContentRetry 必须由内核拦截");
    }
```

- [ ] **Step 2: 运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app extensions::codehealth::tests::content_retry 2>&1 | tail -8
```

Expected: 编译失败 `no variant named ContentRetry`。

- [ ] **Step 3: `mod.rs` 加 `ContentRetry`**

`Message` 枚举末尾加:

```rust
    /// 内容侧 webview 加载失败时,原生占位页的"重试"按钮。内核拦截(清除
    /// `App::codehealth_webview` 的失败状态让 webview 重新挂载),不进本模块 `update`。
    ContentRetry,
```

`update` 的 match 末尾加:

```rust
        Message::ContentRetry => {
            unreachable!("由内核拦截处理,见 codehealth::Message::ContentRetry 文档")
        }
```

- [ ] **Step 4: 常量、字段、消息**

`app/app.rs` 在 `USAGE_CONTENT_ID_OFFSET` 定义之后加:

```rust
/// 代码健康度面板内容侧 Preact webview 的固定槽 id(继承 1_000_000 递增序列)。
pub(crate) const CODEHEALTH_CONTENT_ID_OFFSET: usize = 5_000_000;
```

在 `usage_webview` 字段(约 468 行)之后加:

```rust
    /// 代码健康度内容侧 webview 推送状态(App 级,固定单槽)。
    pub(crate) codehealth_webview: crate::extensions::codehealth::WebviewPushState,
```

初始化处(约 844 行 `usage_webview: ...default(),` 之后)加:

```rust
            codehealth_webview: crate::extensions::codehealth::WebviewPushState::default(),
```

`app/message.rs` 在 `UsageContentWebviewEvent(...)` 之后加:

```rust
    /// 代码健康度内容侧 webview 发回的已解析事件(ready/扫描/跳转/交给 Agent/失败)。
    /// 不带 binding——固定单槽、不按项目分,同 `UsageContentWebviewEvent`。
    CodeHealthContentWebviewEvent(crate::extensions::codehealth::CodeHealthWebviewEvent),
```

- [ ] **Step 5: `update.rs` 处理事件与重试**

抽出扫描入口。把 `Message::CodeHealth(codehealth::Message::ScanRequested)` 分支改为:

```rust
            Message::CodeHealth(codehealth::Message::ScanRequested) => {
                self.code_health_request_scan();
            }
            Message::CodeHealth(codehealth::Message::ContentRetry) => {
                self.codehealth_webview.clear_failed();
            }
```

(这两个分支必须放在通用 `Message::CodeHealth(msg)` 分支**之前**。)

在 `code_health_open_location` 之前新增方法:

```rust
    /// 点"扫描"(原生按钮与 webview 事件共用入口)。
    pub(crate) fn code_health_request_scan(&mut self) {
        self.with_focused_project(|ws, io| {
            codehealth::update(&mut ws.codehealth, codehealth::Message::ScanRequested);
            ws.spawn_codehealth_scan(io);
        });
    }
```

在 `Message::UsageContentWebviewEvent(event) => { ... }` 分支之后加:

```rust
            Message::CodeHealthContentWebviewEvent(event) => {
                use crate::extensions::codehealth::CodeHealthWebviewEvent as Ev;
                match event {
                    Ev::Ready => self.codehealth_webview.set_ready(true),
                    Ev::ScanRequested => self.code_health_request_scan(),
                    Ev::AnalyzeFinding { id } => self.code_health_analyze_finding(id),
                    Ev::OpenLocation { path, line } => {
                        // webview 内容不可信:拒绝绝对路径与 `..`。
                        if codehealth::is_safe_relative_path(&path) {
                            self.code_health_open_location(path, line);
                        } else {
                            dozer_core::log_warn!(
                                LOG,
                                panel = "code_health",
                                path = %path.display(),
                                "拒绝 webview 发来的非相对路径跳转"
                            );
                        }
                    }
                    Ev::Failed { reason } => {
                        dozer_core::log_warn!(
                            LOG,
                            panel = "code_health",
                            %reason,
                            "代码健康度内容页渲染失败,回落原生占位"
                        );
                        self.codehealth_webview.set_failed(reason);
                    }
                }
            }
```

- [ ] **Step 6: IPC 分发(`runtime.rs`)**

在 usage-content 的 `else if webview_id == crate::app::USAGE_CONTENT_ID_OFFSET && looks_like_envelope { ... }` 分支**之后**加:

```rust
                                } else if webview_id == crate::app::CODEHEALTH_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // codehealth-content 同 usage-content:固定单槽,按固定
                                    // webview id 识别,不复用任何 binding。
                                    match crate::extensions::codehealth::parse_codehealth_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::CodeHealthContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 codehealth-content IPC");
                                        }
                                    }
```

注意大括号:该分支以 `} else if ... {` 开头,与上一分支的结尾 `}` 衔接,与现有 usage 分支的写法逐字对齐。

- [ ] **Step 7: `take_codehealth_content_script` 与消费点**

`app/app.rs` 在 `take_usage_content_script` 之后加:

```rust
    /// 代码健康度内容侧待下发推送。声明式:每帧比较"当前该显示什么"
    /// (`current_view_payload`)与"上次送达的"(`codehealth_webview.pending_push`)。
    /// 同时驱动加载超时判定(`observe_availability`)。已失败(`failed`)时不推送——
    /// 原生占位页接管。
    pub fn take_codehealth_content_script(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
        now: std::time::Instant,
    ) -> Vec<(usize, String)> {
        let webview_id = CODEHEALTH_CONTENT_ID_OFFSET;
        self.codehealth_webview
            .observe_availability(available_webview_ids.contains(&webview_id), now);
        if !available_webview_ids.contains(&webview_id) || self.codehealth_webview.failed().is_some()
        {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let desired = crate::extensions::codehealth::current_view_payload(&ws.codehealth);
        let Some(payload) = self.codehealth_webview.pending_push(&desired) else {
            return Vec::new();
        };
        let revision = self.codehealth_webview.mark_sent(payload.clone());
        let envelope = crate::extensions::codehealth::encode_codehealth_push(revision, payload);
        vec![(webview_id, crate::preview::dispatch_script(&envelope))]
    }
```

`platform/window_events.rs` 在用量面板消费循环之后加:

```rust
        // 代码健康度内容侧 webview(单固定槽):声明式推送,同用量面板节奏。
        for (webview_id, js) in
            app.take_codehealth_content_script(&available_webview_ids, std::time::Instant::now())
        {
            if let Some((view, _)) = webviews.get(&webview_id) {
                let _ = view.evaluate_script(&js);
            }
        }
```

- [ ] **Step 8: `preview_desired` 分支**

`app/app.rs` 在 `if kind == PanelKind::Usage { ... continue; }` 块**之后**加:

```rust
            if kind == PanelKind::CodeHealth {
                // 已失败(加载超时/渲染异常)时不挂载,原生占位页接管(见
                // `codehealth::content_pane`)。其余时候恒挂载——"扫描中"也由
                // webview 自己展示(保留旧结果 + 状态条)。
                let content_desired = self.codehealth_webview.failed().is_none();
                let bounds = crate::webview_geometry::codehealth_content_pane_bounds_for(
                    side,
                    window_width,
                    window_height,
                    &self.shell_state(),
                    content_desired,
                );
                if bounds.2 > 0.0 && bounds.3 > 0.0 {
                    let spec = WebviewSpec {
                        id: CODEHEALTH_CONTENT_ID_OFFSET,
                        url: format!(
                            "dozer://codehealth-content/host.html?theme={}",
                            crate::preview::scheme_query_value()
                        ),
                        visible: !app_modal_open,
                        editor_binding: None,
                        loading_generation: None,
                        park_offscreen: false,
                    };
                    out.push((spec, bounds));
                }
                continue;
            }
```

- [ ] **Step 9: 原生失败占位 `content_pane`**

`extensions/codehealth/view.rs`:**保留所有旧渲染函数**(Task 7 才删),但把 `content_pane` 的签名与实现替换为:

```rust
/// 内容列原生壳:webview 盖在这块 `container` 之上(同 Files/Usage 现状),
/// 它只负责面板背景/边框;`failed` 为 `Some` 时 webview 不挂载,这里显示
/// 失败原因与"重试"。
pub fn content_pane(
    failed: Option<&str>,
    width: Length,
    outer: Border,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = match failed
    {
        Some(reason) => column![
            text("代码健康度页面加载失败").size(14).color(tokens.body),
            text(reason.to_string()).size(12).color(tokens.dim),
            button(text("重试").size(13)).on_press(Message::ContentRetry),
        ]
        .spacing(10)
        .padding(16)
        .into(),
        None => column![].into(),
    };
    container(body)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().panel.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

> 旧的 `content_pane(ws_state, width, outer)` 被它替换;旧的 `scan_header`、`error_banner`、`category_content`、`overview_content` 等函数此时变成 `dead_code` 警告,**这是预期的**,Task 7 删除。

`app/view.rs` 的调用点(约 1116 行)改为:

```rust
            let content_pane = codehealth::content_pane(
                app.codehealth_webview.failed(),
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::CodeHealth);
```

- [ ] **Step 10: 订正 spec 的两处不符**

在 `docs/superpowers/specs/2026-10-01-code-health-webview-design.md`:

1. 「分工与边界」表中 `扫描中动画 | 原生 iced(同用量面板「统计中」先例)…` 一行替换为:
   `| 扫描中 | webview 内展示(状态条「扫描中…」+ 旧结果时间提示,保留旧内容;尚无报告时只显示「扫描中…」),不卸载 webview——现状 `view.rs::content_pane` 本来就是文字提示而非 `math_curve` 动画 |`
2. 「协议」JS → Rust 表:`SelectFinding` 一行替换为 `ScanRequested`(点页头「扫描」)与 `AnalyzeFinding { id }`(「交给 Agent 分析」,现有功能,原 spec 漏列);删去"仅在 Rust 需要记录选中项时发"的 `SelectFinding`(无需求,YAGNI)。并在「前端结构」里补一句:结构复杂度「本轮新增/全部」筛选状态移到前端本地,Rust 侧 `StructureFilter` 随之删除。

- [ ] **Step 11: 全量编译 + 测试**

```bash
cargo build -p dozer-app 2>&1 | tail -15
cargo test -p dozer-app 2>&1 | tail -15
bash scripts/check-log-scope.sh
```

Expected: build 成功(只有 `dead_code` warning);全部测试通过,数量不少于 Task 0 记下的基线(新增了 Task 1–4 的测试);`log scope check: ok`。

- [ ] **Step 12: 提交**

```bash
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^error" | head
git add crates/dozer-app/src docs/superpowers/specs/2026-10-01-code-health-webview-design.md
git diff --cached --stat
git commit -m "feat(codehealth): wire content webview into app (state, IPC, push, geometry)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

`--stat` 应只含:`app/app.rs`、`app/message.rs`、`app/update.rs`、`app/view.rs`、`runtime.rs`、`platform/window_events.rs`、`extensions/codehealth/{mod,view}.rs`、该 spec 文档。**不得**含 `Cargo.lock`/`Cargo.toml`/`Info.plist`。

---

### Task 5: 前端脚手架 + 页头 + 空态 + 构建产物

**Files:**
- Create: `crates/dozer-app/web/codehealth-content/{package.json,build.mjs,tsconfig.json,render-smoke.mjs}`
- Create: `crates/dozer-app/web/codehealth-content/src/{host.html,main.tsx,types.ts,styles.css,fixtures.ts,render-smoke.tsx,protocol.ts,protocol.test.ts}`
- Create: `crates/dozer-app/web/codehealth-content/src/components/{App.tsx,ScanHeader.tsx,EmptyState.tsx}`
- Create(构建产物): `crates/dozer-app/assets/codehealth-content/{host.html,codehealth-content.js,codehealth-content.css}`
- Modify: `.gitignore`
- Modify: `crates/dozer-app/src/assets.rs`(产物齐全 / CSP 测试)

**Interfaces:**
- Consumes: Task 1 的 JSON 形状(`CodeHealthViewPayload` 的 serde 输出)。
- Produces(前端内部,后续任务依赖):
  - `types.ts`:`ViewPayload`、`Body`(判别联合,`kind` 字段)、`FindingRow`、`ScanBar`、`CategoryKey`
  - `protocol.ts`:`export function applyEnvelope(prev: {revision:number; payload: ViewPayload|null}, json: string): {revision:number; payload: ViewPayload|null}`(丢弃小于已应用 revision 的推送)
  - `fixtures.ts`:`emptyFixture`、`overviewFixture`、`structureFixture`、`uiFixture`、`scopeFixture`(渲染冒烟与测试共用)
  - `components/App.tsx`:`App({ payload }: { payload: ViewPayload })`
  - `post(event)`:`main.tsx` 里不导出,组件经 `src/ipc.ts` 的 `export function send(event: OutEvent): void`

- [ ] **Step 1: 写失败测试(protocol.ts 的 revision 丢弃)**

```bash
mkdir -p /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content/src/components
```

创建 `web/codehealth-content/package.json`:

```json
{
  "name": "dozer-codehealth-content",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Dozer 代码健康度面板内容侧(codehealth-content),Preact 离线打包,无 CDN/无运行时 Node。",
  "scripts": {
    "build": "node build.mjs",
    "typecheck": "tsc --noEmit",
    "test": "node --test src/*.test.ts && node render-smoke.mjs"
  },
  "dependencies": {
    "preact": "10.29.8"
  },
  "devDependencies": {
    "@types/node": "^24",
    "esbuild": "0.28.2",
    "preact-render-to-string": "6.6.3",
    "typescript": "5.9.3"
  }
}
```

创建 `tsconfig.json`(与 usage-content 逐字相同):

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noEmit": true,
    "skipLibCheck": true,
    "allowImportingTsExtensions": true,
    "jsx": "react-jsx",
    "jsxImportSource": "preact",
    "types": ["node"],
    "lib": ["ES2020", "DOM", "DOM.Iterable"]
  },
  "include": ["src/**/*.ts", "src/**/*.tsx"]
}
```

创建 `src/protocol.test.ts`:

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope } from './protocol.ts';

const env = (revision: number, message: string) =>
  JSON.stringify({
    protocol_version: 1,
    revision,
    payload: {
      scan: { scanning: false, scanned_at: null, scan_error: null, save_error: null },
      category: 'overview',
      body: { kind: 'empty', message },
    },
  });

test('applies a newer revision', () => {
  const s0 = { revision: 0, payload: null };
  const s1 = applyEnvelope(s0, env(1, 'a'));
  assert.equal(s1.revision, 1);
  assert.equal((s1.payload!.body as { message: string }).message, 'a');
});

// Review Focus 3:旧推送晚到不得覆盖新状态。
test('drops an older revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(5, 'new'));
  const s2 = applyEnvelope(s1, env(3, 'old'));
  assert.equal(s2.revision, 5);
  assert.equal((s2.payload!.body as { message: string }).message, 'new');
});

test('ignores malformed json and keeps state', () => {
  const s1 = applyEnvelope({ revision: 2, payload: null }, 'not json');
  assert.equal(s1.revision, 2);
  const s2 = applyEnvelope(s1, JSON.stringify({ revision: 9 }));
  assert.equal(s2.revision, 2, '缺 payload 的推送忽略');
});
```

- [ ] **Step 2: 安装依赖并运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content
npm install 2>&1 | tail -3
node --test src/protocol.test.ts 2>&1 | tail -8
```

Expected: FAIL,`Cannot find module './protocol.ts'`。

- [ ] **Step 3: 实现 `types.ts`、`protocol.ts`、`ipc.ts`**

`src/types.ts`(与 Task 1 的 serde 输出逐字段对应):

```ts
export type CategoryKey = 'overview' | 'structure' | 'ui_consistency' | 'scan_scope';
export type SeverityKey = 'critical' | 'watch';
export type ChangeKey = 'new' | 'worsened' | 'improved' | 'persisting' | 'resolved';
export type TierKey = 'healthy' | 'watch' | 'critical';

export interface ScanBar {
  scanning: boolean;
  scanned_at: string | null;
  scan_error: string | null;
  save_error: string | null;
}

export interface FindingRow {
  id: string;
  title: string;
  path: string;
  line: number;
  severity: SeverityKey;
  severity_label: string;
  change: ChangeKey | null;
  change_label: string | null;
  reasons: string[];
}

export type ChangeDto =
  | { kind: 'first_scan' }
  | {
      kind: 'has_change';
      loc_delta_text: string;
      functions_delta_text: string;
      new_risks: number;
      resolved_risks: number;
      worsened: number;
      improved: number;
    };

export interface OverviewBody {
  kind: 'overview';
  git_line: string | null;
  tier: TierKey;
  tier_label: string;
  summary: string;
  change: ChangeDto;
  priorities: FindingRow[];
  scope_line: string;
  legacy_note: string | null;
}

export interface StructureBody {
  kind: 'structure';
  metric_note: string;
  findings: FindingRow[];
}

export interface UiGroup {
  title: string;
  findings: FindingRow[];
}

export interface UiBody {
  kind: 'ui_consistency';
  applicable: boolean;
  groups: UiGroup[];
}

export interface SkippedDto {
  path: string;
  reason: string;
}

export interface ScopeBody {
  kind: 'scan_scope';
  status_label: string;
  analyzed_files: number;
  excluded_files: number;
  skipped_count: number;
  languages_detail: string;
  duration_ms: number;
  schema_version: number;
  git_baseline: string | null;
  skipped: SkippedDto[];
}

export interface EmptyBody {
  kind: 'empty';
  message: string;
}

export type Body = EmptyBody | OverviewBody | StructureBody | UiBody | ScopeBody;

export interface ViewPayload {
  scan: ScanBar;
  category: CategoryKey;
  body: Body;
}

/** 前端 → Rust(与 `CodeHealthWebviewEvent` 的 serde 形状一致)。 */
export type OutEvent =
  | { kind: 'ready' }
  | { kind: 'scan_requested' }
  | { kind: 'open_location'; path: string; line: number }
  | { kind: 'analyze_finding'; id: string }
  | { kind: 'failed'; reason: string };
```

`src/protocol.ts`:

```ts
import type { ViewPayload } from './types.ts';

export interface PushState {
  revision: number;
  payload: ViewPayload | null;
}

/** 应用一条 Rust 推送。revision 小于已应用值的推送(快速切换时晚到的旧响应)
 *  与无法解析/缺 payload 的推送一律忽略,状态原样返回。 */
export function applyEnvelope(prev: PushState, json: string): PushState {
  try {
    const env = JSON.parse(json) as { revision?: number; payload?: ViewPayload };
    if (!env.payload || typeof env.revision !== 'number') return prev;
    if (env.revision < prev.revision) return prev;
    return { revision: env.revision, payload: env.payload };
  } catch {
    return prev;
  }
}
```

`src/ipc.ts`:

```ts
import type { OutEvent } from './types.ts';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __dozer?: { dispatch(json: string): void };
  }
}

export function send(event: OutEvent): void {
  window.ipc?.postMessage(JSON.stringify(event));
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
node --test src/protocol.test.ts 2>&1 | tail -8
```

Expected: `# pass 3`、`# fail 0`。

- [ ] **Step 5: 构建脚本、host、入口、样式、组件壳**

`build.mjs`:

```js
// 生产构建:把代码健康度内容侧打包成离线、无 CDN、无运行时 Node 的确定性
// 产物到 `crates/dozer-app/assets/codehealth-content/`。不输出 source map;
// minify 后去掉所有 legal comments。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/codehealth-content');

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

await build({
  entryPoints: [path.join(here, 'src/main.tsx')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'css' },
  outfile: path.join(outdir, 'codehealth-content.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
```

`src/host.html`:

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8" />
<meta
  http-equiv="Content-Security-Policy"
  content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'"
/>
<title>code health</title>
<link rel="stylesheet" href="codehealth-content.css" />
</head>
<body>
<div id="root"></div>
<script src="codehealth-content.js"></script>
</body>
</html>
```

`src/styles.css`(ByteBoy2077 令牌沿用 usage-content;卡片阴影/圆角/hover/过渡为观感升级点):

```css
:root[data-theme="dark"] {
  --bg: #0d131c;
  --panel: #0a0e16;
  --card: #12202a;
  --card-hover: #162a36;
  --border: #1c3440;
  --cream: #FFE5B4;
  --body: #c9d4dc;
  --dim: #6B7F8F;
  --gold: #F2D94E;
  --cyan: #47DEF0;
  --green: #1AD585;
  --red: #FF5C5C;
  --shadow: 0 1px 2px rgba(0, 0, 0, 0.45), 0 4px 14px rgba(0, 0, 0, 0.25);
}
:root[data-theme="light"] {
  --bg: #fffdf6;
  --panel: #fef2e4;
  --card: #fefdfb;
  --card-hover: #fff8ec;
  --border: #d7dfe5;
  --cream: #16232e;
  --body: #2b3a46;
  --dim: #4c5c68;
  --gold: #118b96;
  --cyan: #0e8a9e;
  --green: #128f5a;
  --red: #c0392b;
  --shadow: 0 1px 2px rgba(22, 35, 46, 0.12), 0 4px 14px rgba(22, 35, 46, 0.08);
}
* { box-sizing: border-box; }
html, body {
  margin: 0; height: 100%;
  background: var(--panel); color: var(--body);
  font: 13px/1.5 -apple-system, "PingFang SC", sans-serif;
}
#root { height: 100%; overflow-y: auto; padding: 14px 16px 24px; }

h2 { margin: 0 0 4px; font-size: 16px; font-weight: 600; color: var(--cream); }
h3 { margin: 18px 0 8px; font-size: 14px; font-weight: 600; color: var(--cream); }
.dim { color: var(--dim); }
.small { font-size: 11px; }
.row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }

/* 页头 */
.scan-header { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
.btn {
  appearance: none; border: 1px solid var(--border); background: var(--card);
  color: var(--cream); font: inherit; padding: 4px 12px; border-radius: 8px;
  cursor: pointer; transition: background .15s, border-color .15s, transform .05s;
}
.btn:hover { background: var(--card-hover); border-color: var(--gold); }
.btn:active { transform: translateY(1px); }
.btn:disabled { opacity: .5; cursor: default; }
.btn.gold { border-color: var(--gold); color: var(--gold); }
.btn.small { font-size: 11px; padding: 2px 8px; }
.banner { padding: 6px 10px; border-radius: 8px; font-size: 12px; margin-bottom: 8px; }
.banner.error { color: var(--red); background: color-mix(in srgb, var(--red) 12%, transparent); }
.banner.note { color: var(--cyan); background: color-mix(in srgb, var(--cyan) 10%, transparent); }

/* 卡片 */
.card {
  background: var(--card); border: 1px solid var(--border); border-radius: 12px;
  padding: 14px 16px; margin-bottom: 10px; box-shadow: var(--shadow);
}
.tier { font-size: 20px; font-weight: 700; }
.tier.healthy { color: var(--green); }
.tier.watch { color: var(--cyan); }
.tier.critical { color: var(--red); }
.kv { display: flex; gap: 14px; flex-wrap: wrap; margin-top: 4px; }
.kv .neg { color: var(--red); }
.kv .pos { color: var(--green); }

/* 发现行 */
.finding {
  background: var(--card); border: 1px solid var(--border); border-radius: 10px;
  padding: 8px 12px; margin-bottom: 8px; cursor: pointer;
  transition: background .15s, border-color .15s, box-shadow .15s;
}
.finding:hover { background: var(--card-hover); border-color: var(--gold); box-shadow: var(--shadow); }
.finding-head { display: flex; align-items: baseline; gap: 8px; }
.badge {
  font-size: 11px; padding: 0 7px; border-radius: 999px; border: 1px solid currentColor;
  white-space: nowrap;
}
.badge.critical { color: var(--red); }
.badge.watch { color: var(--cyan); }
.finding-title { color: var(--cream); font-size: 13px; }
.finding-meta { color: var(--dim); font-size: 11px; margin-top: 2px; word-break: break-all; }
.finding-actions { margin-top: 4px; }

/* 筛选 */
.seg { display: inline-flex; gap: 8px; margin: 8px 0; }
.seg button {
  appearance: none; font: inherit; font-size: 12px; cursor: pointer;
  padding: 3px 10px; border-radius: 8px; color: var(--dim);
  background: transparent; border: 1px solid transparent;
  transition: background .15s, color .15s;
}
.seg button.active { background: var(--card); color: var(--cream); border-color: var(--gold); }
.seg button:hover { color: var(--cream); }

.empty { padding: 24px 4px; font-size: 14px; color: var(--body); }
.scope-line { margin-top: 12px; }
.skip-list { margin: 6px 0 0; padding: 0; list-style: none; }
.skip-list li { font-size: 11px; color: var(--dim); word-break: break-all; padding: 1px 0; }
```

`src/components/ScanHeader.tsx`:

```tsx
import type { ScanBar } from '../types.ts';
import { send } from '../ipc.ts';

export function ScanHeader({ scan, hasReport }: { scan: ScanBar; hasReport: boolean }) {
  const when = scan.scanned_at ? `上次扫描:${scan.scanned_at}` : '尚未扫描';
  return (
    <div>
      <div class="scan-header">
        {hasReport && <span class="dim small">{when}</span>}
        <button
          class="btn"
          disabled={scan.scanning}
          onClick={() => send({ kind: 'scan_requested' })}
        >
          {scan.scanning ? '扫描中…' : '扫描'}
        </button>
        {scan.scanning && hasReport && scan.scanned_at && (
          <span class="dim small">当前展示的是 {scan.scanned_at} 的旧结果</span>
        )}
      </div>
      {scan.scan_error && <div class="banner error">扫描失败,请重试:{scan.scan_error}</div>}
      {scan.save_error && <div class="banner note">{scan.save_error}</div>}
    </div>
  );
}
```

`src/components/EmptyState.tsx`:

```tsx
export function EmptyState({ message }: { message: string }) {
  return <div class="empty">{message}</div>;
}
```

`src/components/App.tsx`(阶段一先只渲染页头 + 空态,四个分类页在 Task 6 接入;未接入的分类先走 `Placeholder`——Task 6 完成后删掉它):

```tsx
import type { ViewPayload } from '../types.ts';
import { ScanHeader } from './ScanHeader.tsx';
import { EmptyState } from './EmptyState.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const { scan, body } = payload;
  return (
    <div>
      <ScanHeader scan={scan} hasReport={body.kind !== 'empty'} />
      {body.kind === 'empty' ? <EmptyState message={body.message} /> : null}
    </div>
  );
}
```

`src/main.tsx`:

```tsx
import { render } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import './styles.css';
import { App } from './components/App.tsx';
import { applyEnvelope, type PushState } from './protocol.ts';
import { send } from './ipc.ts';

// 主题由 URL `?theme=light|dark` 注入(同 dozer://usage-content),缺省回落 dark。
function applyThemeFromUrl() {
  const theme = new URLSearchParams(location.search).get('theme');
  document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark';
}

function Root() {
  const [state, setState] = useState<PushState>({ revision: 0, payload: null });

  useEffect(() => {
    window.__dozer = {
      dispatch(json: string) {
        setState((prev) => applyEnvelope(prev, json));
      },
    };
    window.addEventListener('error', (e) =>
      send({ kind: 'failed', reason: String(e.message || e.error) }),
    );
    // `__dozer.dispatch` 已可用,报回 Rust;Rust 收到后才开始推送。
    send({ kind: 'ready' });
    return () => {
      delete window.__dozer;
    };
  }, []);

  if (!state.payload) return null;
  return <App payload={state.payload} />;
}

applyThemeFromUrl();
render(<Root />, document.getElementById('root')!);
```

`src/fixtures.ts`(阶段一的五类 fixture,Task 6 的冒烟测试复用):

```ts
import type { ViewPayload, FindingRow, ScanBar } from './types.ts';

export const scanIdle: ScanBar = {
  scanning: false,
  scanned_at: '2026-10-01 08:00:00 UTC',
  scan_error: null,
  save_error: null,
};

export const row = (over: Partial<FindingRow> = {}): FindingRow => ({
  id: 'f1',
  title: 'run 控制流信号 12',
  path: 'src/a.rs',
  line: 7,
  severity: 'critical',
  severity_label: '警戒',
  change: 'new',
  change_label: '本轮新增',
  reasons: ['本轮新增', '近 30 天修改 3 次'],
  ...over,
});

export const emptyFixture: ViewPayload = {
  scan: { ...scanIdle, scanned_at: null },
  category: 'overview',
  body: { kind: 'empty', message: '这个项目还没有扫描过。' },
};

export const overviewFixture: ViewPayload = {
  scan: scanIdle,
  category: 'overview',
  body: {
    kind: 'overview',
    git_line: 'main · abc123 · 有未提交改动',
    tier: 'watch',
    tier_label: '需要关注',
    summary: '核心代码 100 行 · 1 个函数存在明显结构问题',
    change: {
      kind: 'has_change',
      loc_delta_text: '+12',
      functions_delta_text: '-1',
      new_risks: 2,
      resolved_risks: 1,
      worsened: 1,
      improved: 0,
    },
    priorities: [row(), row({ id: 'f2', title: '颜色硬编码', change: null, change_label: null, reasons: [] })],
    scope_line: '已分析 3 个文件 · Rust · 排除 0 · 跳过 0',
    legacy_note: null,
  },
};

export const overviewFirstScanFixture: ViewPayload = {
  ...overviewFixture,
  body: { ...(overviewFixture.body as object), change: { kind: 'first_scan' }, priorities: [] } as ViewPayload['body'],
};

export const overviewLegacyFixture: ViewPayload = {
  ...overviewFixture,
  body: {
    ...(overviewFixture.body as object),
    legacy_note: '旧版报告,重新扫描可查看变化与范围',
  } as ViewPayload['body'],
};

export const structureFixture: ViewPayload = {
  scan: scanIdle,
  category: 'structure',
  body: {
    kind: 'structure',
    metric_note: '指标口径:控制流信号(非标准圈复杂度)',
    findings: [row(), row({ id: 'f3', title: 'old 控制流信号 9', change: 'persisting', change_label: '持续存在' })],
  },
};

export const uiFixture: ViewPayload = {
  scan: scanIdle,
  category: 'ui_consistency',
  body: {
    kind: 'ui_consistency',
    applicable: true,
    groups: [
      { title: '颜色硬编码', findings: [row({ id: 'u1', title: '#ff0000', change: null, change_label: null, reasons: [] })] },
      { title: '边距硬编码', findings: [] },
    ],
  },
};

export const uiNotApplicableFixture: ViewPayload = {
  scan: scanIdle,
  category: 'ui_consistency',
  body: { kind: 'ui_consistency', applicable: false, groups: [] },
};

export const scopeFixture: ViewPayload = {
  scan: scanIdle,
  category: 'scan_scope',
  body: {
    kind: 'scan_scope',
    status_label: '部分完成',
    analyzed_files: 3,
    excluded_files: 1,
    skipped_count: 1,
    languages_detail: 'Rust(3 个文件,结构分析)',
    duration_ms: 420,
    schema_version: 3,
    git_baseline: 'main @ abc123',
    skipped: [{ path: 'src/bad.rs', reason: '解析失败' }],
  },
};

export const scanningWithOldResultFixture: ViewPayload = {
  ...overviewFixture,
  scan: { ...scanIdle, scanning: true },
};

export const scanFailedFixture: ViewPayload = {
  ...emptyFixture,
  scan: { ...scanIdle, scanned_at: null, scan_error: 'boom' },
  body: { kind: 'empty', message: '扫描失败。' },
};
```

`render-smoke.mjs`:

```js
// 渲染冒烟:用 esbuild 把 `src/render-smoke.tsx` 打成 node 可执行的 ESM
// (JSX 不能直接被 `node --test` 的类型剥离跑),再 import 执行。
import { build } from 'esbuild';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = await mkdtemp(path.join(tmpdir(), 'codehealth-smoke-'));
const outfile = path.join(dir, 'smoke.mjs');

await build({
  entryPoints: [path.join(here, 'src/render-smoke.tsx')],
  bundle: true,
  format: 'esm',
  platform: 'node',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'empty' },
  outfile,
  logLevel: 'silent',
});

await import(pathToFileURL(outfile).href);
```

`src/render-smoke.tsx`(阶段一先覆盖空态;Task 6 追加其余):

```tsx
import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import { emptyFixture, scanFailedFixture, scanningWithOldResultFixture } from './fixtures.ts';
import type { ViewPayload } from './types.ts';

const html = (p: ViewPayload) => render(<App payload={p} />);

test('empty: never scanned shows message and a scan button', () => {
  const out = html(emptyFixture);
  assert.match(out, /这个项目还没有扫描过。/);
  assert.match(out, /扫描/);
});

test('empty: scan error shows banner', () => {
  const out = html(scanFailedFixture);
  assert.match(out, /扫描失败,请重试:boom/);
});

test('scanning disables the button', () => {
  const out = html(scanningWithOldResultFixture);
  assert.match(out, /disabled/);
  assert.match(out, /扫描中…/);
});
```

> 注意:`scanningWithOldResultFixture` 的 body 是 overview,阶段一的 `App` 此时还不渲染 overview 主体,但页头(按钮 disabled + "扫描中…" + 旧结果时间)已经渲染,这条测试只断言页头,所以现在就能通过。

- [ ] **Step 6: 类型检查、测试、构建**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content
npm run typecheck 2>&1 | tail -8
npm test 2>&1 | tail -12
npm run build 2>&1 | tail -6
ls -la ../../assets/codehealth-content
```

Expected: typecheck 无输出;`npm test` 通过(protocol 3 条 + 冒烟 3 条);build 打印 `built -> …/assets/codehealth-content`;三个产物文件存在且非空(`codehealth-content.css` 由 esbuild 从 `import './styles.css'` 生成,若缺失说明 `loader: {'.css': 'css'}` 与 `import` 没生效,检查 `main.tsx` 的 `import './styles.css'`)。

- [ ] **Step 7: Rust 侧产物齐全/CSP 测试 + .gitignore**

`.gitignore` 在 usage-content 那条之后加:

```
# codehealth-content host deps (built assets under assets/codehealth-content are committed)
crates/dozer-app/web/codehealth-content/node_modules/
```

`assets.rs` 在 `codehealth_content_rejects_path_traversal` 测试之后追加:

```rust
    /// 提交的 codehealth-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn codehealth_content_bundle_assets_are_present() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/codehealth-content"
        ));
        for f in ["host.html", "codehealth-content.js", "codehealth-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 codehealth-content 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "codehealth-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src、无网络引用。
    #[test]
    fn codehealth_content_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/codehealth-content"
        ));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("connect-src"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(
            html.contains("codehealth-content.js") && html.contains("codehealth-content.css")
        );
    }
```

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app assets::tests::codehealth 2>&1 | tail -8
```

Expected: 5 个 codehealth 测试全过。

- [ ] **Step 8: 提交**

```bash
git add .gitignore crates/dozer-app/src/assets.rs crates/dozer-app/web/codehealth-content crates/dozer-app/assets/codehealth-content
git status --short | grep node_modules || true
git diff --cached --stat | tail -30
git commit -m "feat(codehealth): add codehealth-content webview scaffold, header and empty state

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git status --short | grep node_modules` 无输出(依赖未被误加);`package-lock.json` 在提交里。

---

### Task 6: 四个分类页组件 + 观感升级

**Files:**
- Create: `web/codehealth-content/src/components/{FindingRow.tsx,OverviewPage.tsx,StructurePage.tsx,UiPage.tsx,ScopePage.tsx}`
- Create: `web/codehealth-content/src/structureFilter.ts`、`structureFilter.test.ts`
- Modify: `web/codehealth-content/src/components/App.tsx`、`src/render-smoke.tsx`
- Modify(重新构建后提交): `assets/codehealth-content/*`

**Interfaces:**
- Consumes: Task 5 的 `types.ts`/`ipc.ts`/`fixtures.ts`/`styles.css` 类名。
- Produces: `filterStructure(findings: FindingRow[], mode: 'new' | 'all'): FindingRow[]`(`new` = `change` 为 `new` 或 `worsened`;`change === null` 不算);`App` 按 `body.kind` 分派到四页。

- [ ] **Step 1: 写失败测试(筛选 + 四页渲染冒烟)**

`src/structureFilter.test.ts`:

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { filterStructure } from './structureFilter.ts';
import { row } from './fixtures.ts';

const rows = [
  row({ id: 'a', change: 'new' }),
  row({ id: 'b', change: 'worsened' }),
  row({ id: 'c', change: 'improved' }),
  row({ id: 'd', change: 'persisting' }),
  row({ id: 'e', change: null }),
];

// 与旧 view.rs 的 `is_new` 语义逐条对应:新增或恶化算"本轮新增",无差异数据不算。
test('new mode keeps only new and worsened', () => {
  assert.deepEqual(filterStructure(rows, 'new').map((r) => r.id), ['a', 'b']);
});

test('all mode keeps everything in order', () => {
  assert.deepEqual(filterStructure(rows, 'all').map((r) => r.id), ['a', 'b', 'c', 'd', 'e']);
});
```

在 `src/render-smoke.tsx` 末尾追加(并把顶部 import 改为同时引入新 fixture):

```tsx
import {
  overviewFixture,
  overviewFirstScanFixture,
  overviewLegacyFixture,
  structureFixture,
  uiFixture,
  uiNotApplicableFixture,
  scopeFixture,
} from './fixtures.ts';

test('overview: tier, summary, change card, priorities, scope line', () => {
  const out = html(overviewFixture);
  assert.match(out, /需要关注/);
  assert.match(out, /核心代码 100 行/);
  assert.match(out, /代码行 \+12/);
  assert.match(out, /函数数 -1/);
  assert.match(out, /新增风险 \+2/);
  assert.match(out, /已解决风险 1/);
  assert.match(out, /交给 Agent 分析/);
  assert.match(out, /本轮新增/);
  assert.match(out, /已分析 3 个文件/);
  assert.match(out, /main · abc123 · 有未提交改动/);
});

test('overview: first scan never fakes zero change', () => {
  const out = html(overviewFirstScanFixture);
  assert.match(out, /首次扫描,暂无历史可比对/);
  assert.doesNotMatch(out, /新增风险/);
  assert.match(out, /暂无优先处理项。/);
});

test('overview: legacy report note is shown', () => {
  assert.match(html(overviewLegacyFixture), /旧版报告,重新扫描可查看变化与范围/);
});

test('structure: default filter is new-only; severity and change use text labels', () => {
  const out = html(structureFixture);
  assert.match(out, /指标口径:控制流信号/);
  assert.match(out, /run 控制流信号 12/);
  assert.doesNotMatch(out, /old 控制流信号 9/, '默认只看本轮新增');
  assert.match(out, /警戒/);
  assert.match(out, /本轮新增/);
  assert.match(out, /src\/a\.rs:7/);
});

test('ui: groups with counts and empty groups say 无发现', () => {
  const out = html(uiFixture);
  assert.match(out, /颜色硬编码:1 处/);
  assert.match(out, /边距硬编码:0 处/);
  assert.match(out, /无发现/);
});

test('ui: not applicable is not rendered as healthy', () => {
  const out = html(uiNotApplicableFixture);
  assert.match(out, /UI 一致性检测适用于 iced\/Rust/);
  assert.doesNotMatch(out, /无发现/);
});

test('scope: status, counts, skipped details', () => {
  const out = html(scopeFixture);
  assert.match(out, /扫描状态:部分完成/);
  assert.match(out, /已分析 3 个文件 · 排除 1 个 · 跳过 1 个/);
  assert.match(out, /Rust\(3 个文件,结构分析\)/);
  assert.match(out, /扫描耗时:420 ms/);
  assert.match(out, /报告版本:schema v3/);
  assert.match(out, /Git 基准:main @ abc123/);
  assert.match(out, /src\/bad\.rs（?\(?解析失败/);
});
```

> `scope` 最后一条断言的括号写法由组件决定;实现里使用半角括号 `src/bad.rs(解析失败)`,所以把该条正则固定为 `/src\/bad\.rs\(解析失败\)/`(在 Step 3 实现后若不一致,以组件输出为准**同时**改测试与组件为同一种,不要放宽成 `.*`)。

- [ ] **Step 2: 运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content
npm test 2>&1 | tail -20
```

Expected: `structureFilter.ts` 找不到;冒烟里 overview/structure/ui/scope 断言失败。

- [ ] **Step 3: 实现**

`src/structureFilter.ts`:

```ts
import type { FindingRow } from './types.ts';

/** `new` = 本轮新增(`new`/`worsened`);`change === null`(无差异数据)不算新增。 */
export function filterStructure(findings: FindingRow[], mode: 'new' | 'all'): FindingRow[] {
  if (mode === 'all') return findings;
  return findings.filter((f) => f.change === 'new' || f.change === 'worsened');
}
```

`src/components/FindingRow.tsx`:

```tsx
import type { FindingRow as Row } from '../types.ts';
import { send } from '../ipc.ts';

export function FindingRowView({ row, analyze }: { row: Row; analyze?: boolean }) {
  return (
    <div class="finding" onClick={() => send({ kind: 'open_location', path: row.path, line: row.line })}>
      <div class="finding-head">
        <span class={`badge ${row.severity}`}>{row.severity_label}</span>
        <span class="finding-title">{row.title}</span>
        {row.change_label && <span class="dim small">{row.change_label}</span>}
      </div>
      <div class="finding-meta">
        {row.path}:{row.line}
        {row.reasons.length > 0 && <> · {row.reasons.join(' · ')}</>}
      </div>
      {analyze && (
        <div class="finding-actions">
          <button
            class="btn small"
            onClick={(e) => {
              e.stopPropagation();
              send({ kind: 'analyze_finding', id: row.id });
            }}
          >
            交给 Agent 分析
          </button>
        </div>
      )}
    </div>
  );
}
```

`src/components/OverviewPage.tsx`:

```tsx
import type { OverviewBody } from '../types.ts';
import { FindingRowView } from './FindingRow.tsx';

export function OverviewPage({ body }: { body: OverviewBody }) {
  return (
    <div>
      <div class="row">
        <h2>代码健康度总览</h2>
        {body.git_line && <span class="dim small">{body.git_line}</span>}
      </div>

      <div class="card">
        <div class={`tier ${body.tier}`}>{body.tier_label}</div>
        <div>{body.summary}</div>
      </div>

      <div class="card">
        <div>本次变化</div>
        {body.change.kind === 'first_scan' ? (
          <div class="dim">首次扫描,暂无历史可比对</div>
        ) : (
          <>
            <div class="kv">
              <span>代码行 {body.change.loc_delta_text}</span>
              <span>函数数 {body.change.functions_delta_text}</span>
            </div>
            <div class="kv">
              <span class="neg">新增风险 +{body.change.new_risks}</span>
              <span class="pos">已解决风险 {body.change.resolved_risks}</span>
            </div>
          </>
        )}
      </div>

      <h3>优先处理</h3>
      {body.priorities.length === 0 ? (
        <div class="dim">暂无优先处理项。</div>
      ) : (
        body.priorities.map((r) => <FindingRowView key={r.id} row={r} analyze />)
      )}

      <div class="dim small scope-line">扫描可信度:{body.scope_line}</div>
      {body.legacy_note && <div class="banner note">{body.legacy_note}</div>}
    </div>
  );
}
```

`src/components/StructurePage.tsx`:

```tsx
import { useState } from 'preact/hooks';
import type { StructureBody } from '../types.ts';
import { filterStructure } from '../structureFilter.ts';
import { FindingRowView } from './FindingRow.tsx';

export function StructurePage({ body }: { body: StructureBody }) {
  const [mode, setMode] = useState<'new' | 'all'>('new');
  const shown = filterStructure(body.findings, mode);
  return (
    <div>
      <div class="row">
        <h2>结构复杂度</h2>
        <span class="dim small">{body.metric_note}</span>
      </div>
      <div class="seg">
        <button class={mode === 'new' ? 'active' : ''} onClick={() => setMode('new')}>
          本轮新增
        </button>
        <button class={mode === 'all' ? 'active' : ''} onClick={() => setMode('all')}>
          全部
        </button>
      </div>
      {shown.length === 0 ? (
        <div class="dim">
          {mode === 'new' ? '本轮没有新增的结构复杂度发现。' : '没有发现结构复杂的函数。'}
        </div>
      ) : (
        shown.map((r) => <FindingRowView key={r.id} row={r} />)
      )}
    </div>
  );
}
```

`src/components/UiPage.tsx`:

```tsx
import type { UiBody } from '../types.ts';
import { FindingRowView } from './FindingRow.tsx';

export function UiPage({ body }: { body: UiBody }) {
  if (!body.applicable) {
    return (
      <div>
        <h2>UI 一致性</h2>
        <div class="dim">UI 一致性检测适用于 iced/Rust,当前项目未识别到适用框架。</div>
      </div>
    );
  }
  return (
    <div>
      <h2>UI 一致性</h2>
      {body.groups.map((g) => (
        <div key={g.title}>
          <h3>
            {g.title}:{g.findings.length} 处
          </h3>
          {g.findings.length === 0 ? (
            <div class="dim small">无发现</div>
          ) : (
            g.findings.map((r) => <FindingRowView key={r.id} row={r} />)
          )}
        </div>
      ))}
    </div>
  );
}
```

`src/components/ScopePage.tsx`:

```tsx
import type { ScopeBody } from '../types.ts';

export function ScopePage({ body }: { body: ScopeBody }) {
  return (
    <div>
      <h2>扫描范围</h2>
      <div class="card">
        <div>扫描状态:{body.status_label}</div>
        <div>
          已分析 {body.analyzed_files} 个文件 · 排除 {body.excluded_files} 个 · 跳过{' '}
          {body.skipped_count} 个
        </div>
        <div class="dim">已发现语言:{body.languages_detail}</div>
        <div class="dim">扫描耗时:{body.duration_ms} ms</div>
        <div class="dim">报告版本:schema v{body.schema_version}</div>
        {body.git_baseline && <div class="dim">Git 基准:{body.git_baseline}</div>}
      </div>
      {body.skipped.length > 0 && (
        <>
          <h3>跳过/排除明细:</h3>
          <ul class="skip-list">
            {body.skipped.map((s) => (
              <li key={s.path}>
                {s.path}({s.reason})
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
```

`src/components/App.tsx` 替换为:

```tsx
import type { ViewPayload } from '../types.ts';
import { ScanHeader } from './ScanHeader.tsx';
import { EmptyState } from './EmptyState.tsx';
import { OverviewPage } from './OverviewPage.tsx';
import { StructurePage } from './StructurePage.tsx';
import { UiPage } from './UiPage.tsx';
import { ScopePage } from './ScopePage.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const { scan, body } = payload;
  return (
    <div>
      <ScanHeader scan={scan} hasReport={body.kind !== 'empty'} />
      {body.kind === 'empty' && <EmptyState message={body.message} />}
      {body.kind === 'overview' && <OverviewPage body={body} />}
      {body.kind === 'structure' && <StructurePage body={body} />}
      {body.kind === 'ui_consistency' && <UiPage body={body} />}
      {body.kind === 'scan_scope' && <ScopePage body={body} />}
    </div>
  );
}
```

- [ ] **Step 4: 运行测试/类型检查/构建**

```bash
npm run typecheck 2>&1 | tail -8
npm test 2>&1 | tail -20
npm run build 2>&1 | tail -4
```

Expected: 全绿。冒烟里 structure 的"默认只看本轮新增"依赖 `useState` 初值 `'new'`,`preact-render-to-string` 的同步渲染可覆盖。

- [ ] **Step 5: 提交**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
git add crates/dozer-app/web/codehealth-content crates/dozer-app/assets/codehealth-content
git diff --cached --stat | tail -20
git commit -m "feat(codehealth): render overview/structure/ui/scope pages in webview

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: 删除旧 iced 内容渲染 + 阶段一验收

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`(删旧渲染,只留 `list_pane`、`category_button`、`category_icon`、`content_pane` 与精简测试)
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`(删 `StructureFilter` / `Message::StructureFilterSet` / 字段 / 访问器及对应测试)
- Modify: `crates/dozer-app/src/extensions/codehealth/view_model.rs`(若有仅旧渲染使用的内容,按编译提示清理)

**Interfaces:**
- Consumes: Task 1–6 全部。
- Produces: 阶段一终态——内容渲染只剩 webview;`view.rs` 只含导航与失败占位。

- [ ] **Step 1: 确认删除前基线**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo build -p dozer-app 2>&1 | grep -E "warning: (function|struct|enum|constant).*never (used|constructed)" | head -30
```

Expected: 列出 `overview_content`、`structure_content`、`ui_consistency_content`、`scan_scope_content`、`scan_header`、`error_banner`、`category_content`、`finding_row`、`status_card`、`change_card`、`priority_list`、`filter_buttons`、`empty_content`、`tier_color`、`format_ms` 等 dead_code——正好是要删的集合(dead_code 警告是"路由 bug"的一手信号:若这里**缺**某个预期的旧函数,说明它仍被某处引用,先查清再删)。

- [ ] **Step 2: 精简 `view.rs`**

把 `view.rs` 替换为下面内容(保留原 `category_icon`、`category_button`、`list_pane` 原样,`content_pane` 为 Task 4 的新版;其余全删):

```rust
//! 代码健康度面板原生部分:右侧分类导航 + 内容列原生壳(webview 加载失败时
//! 的占位)。内容渲染全部在 `dozer://codehealth-content` webview 里
//! (见 `web/codehealth-content/` 与 `protocol.rs`)。

use super::{CodeHealthCategory, Message, WorkspaceState};
use byteui::interaction::icons;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, row, space, text};

fn category_icon(category: CodeHealthCategory) -> icons::IconKind {
    match category {
        CodeHealthCategory::Overview => icons::IconKind::BarChart3,
        CodeHealthCategory::Structure => icons::IconKind::FileCode,
        CodeHealthCategory::UiConsistency => icons::IconKind::LayoutList,
        CodeHealthCategory::ScanScope => icons::IconKind::Search,
    }
}

fn category_button(
    category: CodeHealthCategory,
    current: CodeHealthCategory,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = category == current;
    let c = byteui::theme::color::current();
    let fg = if active { c.cream } else { c.dim };
    let icon_color = if active { c.gold } else { c.dim };
    button(
        row![
            icons::view(
                category_icon(category),
                byteui::theme::icon_size::row(),
                icon_color
            ),
            text(category.label())
                .size(byteui::theme::font::body())
                .color(fg),
            space::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::CategorySet(category))
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().gold
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

pub fn list_pane(
    ws_state: &WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = container(crate::chrome::homespace::home_panel_head(
        icons::IconKind::SquareActivity,
        "代码健康度",
    ))
    .padding(iced_widget::core::Padding {
        top: 12.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    });

    let current = ws_state.category();
    let mut nav = column![].spacing(4).padding(iced_widget::core::Padding {
        top: 0.0,
        right: 8.0,
        bottom: 12.0,
        left: 8.0,
    });
    for category in CodeHealthCategory::all() {
        nav = nav.push(category_button(category, current));
    }

    container(column![header, nav])
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 内容列原生壳:webview 盖在这块 `container` 之上(同 Files/Usage 现状),
/// 它只负责面板背景/边框;`failed` 为 `Some` 时 webview 不挂载,这里显示
/// 失败原因与"重试"。
pub fn content_pane(
    failed: Option<&str>,
    width: Length,
    outer: Border,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = match failed
    {
        Some(reason) => column![
            text("代码健康度页面加载失败").size(14).color(tokens.body),
            text(reason.to_string()).size(12).color(tokens.dim),
            button(text("重试").size(13)).on_press(Message::ContentRetry),
        ]
        .spacing(10)
        .padding(16)
        .into(),
        None => column![].into(),
    };
    container(body)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().panel.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_labels_and_all_cover_four_categories() {
        assert_eq!(CodeHealthCategory::all().len(), 4);
        assert_eq!(CodeHealthCategory::Overview.label(), "总览");
        assert_eq!(CodeHealthCategory::Structure.label(), "结构复杂度");
        assert_eq!(CodeHealthCategory::UiConsistency.label(), "UI 一致性");
        assert_eq!(CodeHealthCategory::ScanScope.label(), "扫描范围");
    }
}
```

- [ ] **Step 3: 删 `StructureFilter`**

`mod.rs` 中删除:`pub enum StructureFilter`(含文档)、`WorkspaceState.structure_filter` 字段及其注释、`pub fn structure_filter(&self)`、`Message::StructureFilterSet` 变体及其 `update` 分支。`mod.rs` 的 `tests` 里不存在引用它的测试(原本没有),无需改。再 `grep -rn "StructureFilter" crates/dozer-app/src` 确认全仓无残留(`protocol.rs` 里只有注释中的"筛选由前端做",不引用该类型)。

- [ ] **Step 4: 编译 + 全量测试 + 门禁 + 无 dead_code 残留**

```bash
cargo build -p dozer-app 2>&1 | grep -c "warning" || true
cargo build -p dozer-app 2>&1 | grep -E "never (used|constructed|read)" | head
cargo test -p dozer-app 2>&1 | tail -8
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c | head
cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh
```

Expected: 无 `never used` 类警告(若有,按提示删除对应遗留);全部测试通过;`fmt-ok`;`log scope check: ok`。`view_model.rs` 里 `tier_label/sev_label/change_label` 现在被 `protocol.rs` 使用,`pub(super)` 即可;`format_ms` 同理。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/extensions/codehealth
git diff --cached --stat
git commit -m "refactor(codehealth): remove iced content rendering, webview is the only content path

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: 阶段一人工验收(GUI)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo run -p dozer-app
```

打开一个 Rust 项目 → 切到「代码健康度」面板,逐项核对并把结果(通过/不通过 + 现象)记录给用户:

1. 未扫描项目:显示「这个项目还没有扫描过。」与「扫描」按钮;点扫描→按钮变「扫描中…」→完成后出总览。
2. 总览/结构复杂度/UI 一致性/扫描范围四页信息与旧版逐项一致(对照已批准 Figma `S-CodeHealth 代码健康 · 本次变化与风险热点`);变化与严重度有文字标签。
3. 点发现行 → Files 面板打开该文件并跳到对应行;点「交给 Agent 分析」→ 诊断文本出现在 agent 输入区(不自动发送)。
4. 结构复杂度「本轮新增 / 全部」切换正常;切到别的分类再切回,筛选状态保留。
5. 快速连点右侧四个分类 10 次以上:无白屏闪烁,最终内容与最后一次点击一致。
6. 拖动内容|导航分隔线:webview 随分栏同步变宽,不遮住导航,不超出面板圆角。
7. 窗口放大(maximize)、面板跨栏拖拽、收起展开该侧:webview 位置/可见性正确。
8. 切到别的面板再切回:内容完整重现(不空白)。
9. 扫描中:保留旧结果,顶部有「扫描中…」与「当前展示的是 … 的旧结果」。
10. (可选,制造失败)临时把 `assets/codehealth-content/host.html` 改名后重启 → 约 10 秒后内容区显示「代码健康度页面加载失败 / 页面加载超时」与「重试」;改回名字点重试可恢复。**验完务必恢复文件。**

**阶段一到此可独立合并。** 若人工验收有不通过项,在本任务里修到通过再进阶段二;不要带着已知缺陷往后走。

---

## Phase 2:架构页

### Task 8: Rust 架构 payload

**Files:**
- Create: `crates/dozer-app/src/extensions/codehealth/arch_payload.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`(`mod arch_payload;`、`CodeHealthCategory::Architecture`、`all()` 改 5 个、`label()`)
- Modify: `crates/dozer-app/src/extensions/codehealth/protocol.rs`(`CategoryKey::Architecture`、`Body::Architecture`、`current_view_payload` 分支)
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`(`category_icon`、测试里 4→5)
- Test: `arch_payload.rs` 内 `mod tests`;`view.rs` 测试

**Interfaces:**
- Consumes: `dozer_codehealth::{ArchitectureReport, ArchitectureNode, ArchitectureEdge, ArchitectureNodeKind, ArchitectureEdgeKind, ArchitectureStatus, ArchitectureDiffOutcome, ImpactScope, FindingEvidence, rule_ids}`;`WorkspaceState::{report, architecture_diff, impact}`;Task 1 的 `FindingRowDto`/`row_dto`。
- Produces:
  - `pub const MAX_PAYLOAD_NODES: usize = 3000;`
  - `pub struct ArchitectureBody { status: &'static str, status_note: Option<String>, truncated_note: Option<String>, nodes: Vec<ArchNodeDto>, edges: Vec<ArchEdgeDto>, cycles: Vec<ArchCycleDto>, risks: Vec<ArchRiskDto>, diff: ArchDiffDto, impact: ArchImpactDto, errors: Vec<String>, unresolved_edges: usize }`
  - `pub fn architecture_body(ws: &WorkspaceState, report: &ProjectReport) -> ArchitectureBody`
  - `Body::Architecture(ArchitectureBody)`;`CategoryKey::Architecture`(JSON `"architecture"`);`CodeHealthCategory::Architecture`(标签「架构」)

- [ ] **Step 1: 写失败测试**

先把 `row_dto` 在 `protocol.rs` 里改为 `pub(super) fn row_dto(...)`(`arch_payload.rs` 要用)。创建 `arch_payload.rs`,先只放 `use` 与测试:

```rust
//! 架构页 payload:把 `ProjectReport.architecture`、架构差异、影响范围与架构类
//! 发现项算成前端可直接渲染的 DTO。风险分析(循环/枢纽/越界)、差异、影响范围
//! 全部在 Rust 里算好;前端只做可见子图投影与渲染。

use super::protocol::{FindingRowDto, row_dto};
use super::WorkspaceState;
use dozer_codehealth::{
    ArchitectureDiffOutcome, ArchitectureEdgeKind, ArchitectureNodeKind, ArchitectureStatus,
    FindingEvidence, ProjectReport, rule_ids,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{
        ArchitectureEdge, ArchitectureNode, ArchitectureReport, DependencyCycle, Finding,
        FindingCategory, FindingSeverity, HealthTier, SCHEMA_VERSION, ScanMetadata, ScanStatus,
    };
    use std::path::PathBuf;

    fn node(id: &str, kind: ArchitectureNodeKind, parent: Option<&str>) -> ArchitectureNode {
        ArchitectureNode {
            id: id.into(),
            kind,
            name: id.rsplit(':').next().unwrap_or(id).into(),
            qualified_name: id.trim_start_matches("module:").trim_start_matches("crate:").into(),
            path: Some(PathBuf::from("src/x.rs")),
            parent_id: parent.map(str::to_owned),
            loc: 10,
            fan_in: 1,
            fan_out: 2,
            layer: None,
            external: kind == ArchitectureNodeKind::ExternalCrate,
        }
    }

    fn edge(kind: ArchitectureEdgeKind, from: &str, to: &str) -> ArchitectureEdge {
        ArchitectureEdge {
            id: dozer_codehealth::edge_id(kind, from, to),
            from: from.into(),
            to: to.into(),
            kind,
            evidence: vec![],
        }
    }

    fn report_with(arch: ArchitectureReport, findings: Vec<Finding>) -> ProjectReport {
        ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                status: ScanStatus::Complete,
                analyzed_files: 1,
                ..ScanMetadata::default()
            },
            git: None,
            findings,
            architecture: arch,
            total_loc: 1,
            total_functions: 1,
            critical_functions: 0,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Healthy,
            overall_tier: HealthTier::Healthy,
            functions: vec![],
            color_findings: vec![],
            color_tier: HealthTier::Healthy,
            spacing_findings: vec![],
            spacing_tier: HealthTier::Healthy,
            font_findings: vec![],
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: vec![],
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }

    fn ws_with(report: ProjectReport) -> WorkspaceState {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Loaded(
                1,
                Box::new(super::super::PanelState {
                    report: Some(report),
                    ..Default::default()
                }),
            ),
        );
        ws
    }

    fn arch(nodes: Vec<ArchitectureNode>, edges: Vec<ArchitectureEdge>) -> ArchitectureReport {
        ArchitectureReport {
            status: ArchitectureStatus::Complete,
            nodes,
            edges,
            cycles: vec![],
            unresolved_edges: 0,
            errors: vec![],
        }
    }

    fn cycle_finding(ids: &[&str]) -> Finding {
        Finding {
            id: "arch-cycle-1".into(),
            rule_id: rule_ids::ARCHITECTURE_CYCLE.into(),
            category: FindingCategory::Architecture,
            severity: FindingSeverity::Critical,
            path: PathBuf::from("src/a.rs"),
            start_line: 1,
            symbol: None,
            title: "循环依赖".into(),
            evidence: FindingEvidence::ArchitectureCycle {
                node_ids: ids.iter().map(|s| s.to_string()).collect(),
            },
            applicability: Default::default(),
        }
    }

    #[test]
    fn not_applicable_report_is_not_zero_risk() {
        let ws = ws_with(report_with(ArchitectureReport::not_applicable(), vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.status, "not_applicable");
        assert!(b.status_note.as_deref().unwrap().contains("重新扫描"));
        assert!(b.nodes.is_empty());
    }

    #[test]
    fn external_crates_and_their_edges_are_dropped() {
        let nodes = vec![
            node("crate:a", ArchitectureNodeKind::Crate, None),
            node("crate:b", ArchitectureNodeKind::Crate, None),
            node("external:serde", ArchitectureNodeKind::ExternalCrate, None),
        ];
        let edges = vec![
            edge(ArchitectureEdgeKind::CargoDependency, "crate:a", "crate:b"),
            edge(ArchitectureEdgeKind::CargoDependency, "crate:a", "external:serde"),
        ];
        let ws = ws_with(report_with(arch(nodes, edges), vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.status, "complete");
        assert_eq!(b.nodes.len(), 2);
        assert_eq!(b.edges.len(), 1);
        assert_eq!(b.edges[0].to, "crate:b");
        assert_eq!(b.edges[0].kind, "cargo_dependency");
    }

    #[test]
    fn cycle_finding_becomes_risk_with_node_and_edge_ids() {
        let nodes = vec![
            node("module:a::x", ArchitectureNodeKind::Module, Some("crate:a")),
            node("module:a::y", ArchitectureNodeKind::Module, Some("crate:a")),
        ];
        let edges = vec![
            edge(ArchitectureEdgeKind::ModuleUse, "module:a::x", "module:a::y"),
            edge(ArchitectureEdgeKind::ModuleUse, "module:a::y", "module:a::x"),
        ];
        let ws = ws_with(report_with(
            arch(nodes, edges.clone()),
            vec![cycle_finding(&["module:a::x", "module:a::y"])],
        ));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.risks.len(), 1);
        let r = &b.risks[0];
        assert_eq!(r.kind, "cycle");
        assert_eq!(r.node_ids, vec!["module:a::x", "module:a::y"]);
        let mut got = r.edge_ids.clone();
        got.sort();
        let mut want: Vec<String> = edges.iter().map(|e| e.id.clone()).collect();
        want.sort();
        assert_eq!(got, want, "环内两条边都应归入风险的 edge_ids");
        assert_eq!(r.finding.title, "循环依赖");
    }

    #[test]
    fn hub_and_boundary_findings_map_to_risks() {
        let nodes = vec![
            node("module:a::x", ArchitectureNodeKind::Module, Some("crate:a")),
            node("module:a::y", ArchitectureNodeKind::Module, Some("crate:a")),
        ];
        let e = edge(ArchitectureEdgeKind::ModuleUse, "module:a::x", "module:a::y");
        let hub = Finding {
            id: "hub".into(),
            rule_id: rule_ids::ARCHITECTURE_HIGH_FAN_OUT.into(),
            evidence: FindingEvidence::ArchitectureHub {
                node_id: "module:a::x".into(),
                fan_out: 30,
            },
            ..cycle_finding(&[])
        };
        let boundary = Finding {
            id: "bd".into(),
            rule_id: rule_ids::ARCHITECTURE_LAYER_VIOLATION.into(),
            evidence: FindingEvidence::ArchitectureBoundary {
                edge_id: e.id.clone(),
                from_layer: "ui".into(),
                to_layer: "core".into(),
            },
            ..cycle_finding(&[])
        };
        let ws = ws_with(report_with(arch(nodes, vec![e.clone()]), vec![hub, boundary]));
        let b = architecture_body(&ws, ws.report().unwrap());
        let kinds: BTreeSet<&str> = b.risks.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, BTreeSet::from(["hub", "boundary"]));
        let boundary = b.risks.iter().find(|r| r.kind == "boundary").unwrap();
        assert_eq!(boundary.edge_ids, vec![e.id.clone()]);
        assert_eq!(boundary.node_ids, vec!["module:a::x", "module:a::y"]);
        let hub = b.risks.iter().find(|r| r.kind == "hub").unwrap();
        assert_eq!(hub.node_ids, vec!["module:a::x"]);
    }

    /// Review Focus 5:大图裁成 crate 层,并带截断说明。
    #[test]
    fn oversized_graph_is_cut_to_crate_layer_with_note() {
        let mut nodes = vec![
            node("crate:a", ArchitectureNodeKind::Crate, None),
            node("crate:b", ArchitectureNodeKind::Crate, None),
        ];
        for i in 0..(MAX_PAYLOAD_NODES + 5) {
            nodes.push(node(
                &format!("module:a::m{i}"),
                ArchitectureNodeKind::Module,
                Some("crate:a"),
            ));
        }
        let edges = vec![
            edge(ArchitectureEdgeKind::CargoDependency, "crate:a", "crate:b"),
            edge(ArchitectureEdgeKind::ModuleUse, "module:a::m0", "module:a::m1"),
        ];
        let ws = ws_with(report_with(arch(nodes, edges), vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert!(b.nodes.iter().all(|n| n.kind == "crate"), "只保留 crate 层");
        assert_eq!(b.edges.len(), 1);
        assert!(b.truncated_note.as_deref().unwrap().contains("crate"));
    }

    #[test]
    fn partial_status_carries_errors_and_note() {
        let mut a = arch(vec![node("crate:a", ArchitectureNodeKind::Crate, None)], vec![]);
        a.status = ArchitectureStatus::Partial;
        a.errors = vec!["cargo metadata 失败".into()];
        a.cycles = vec![DependencyCycle {
            id: "c".into(),
            node_ids: vec![],
            edge_ids: vec![],
        }];
        let ws = ws_with(report_with(a, vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.status, "partial");
        assert_eq!(b.errors, vec!["cargo metadata 失败"]);
        assert!(b.status_note.is_some());
    }

    #[test]
    fn no_baseline_diff_is_not_rendered_as_all_new() {
        let ws = ws_with(report_with(
            arch(vec![node("crate:a", ArchitectureNodeKind::Crate, None)], vec![]),
            vec![],
        ));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.diff.state, "no_baseline");
        assert!(b.diff.added_nodes.is_empty() && b.diff.added_edges.is_empty());
    }

    #[test]
    fn payload_serializes_stable_shape() {
        let ws = ws_with(report_with(
            arch(vec![node("crate:a", ArchitectureNodeKind::Crate, None)], vec![]),
            vec![],
        ));
        let v = serde_json::to_value(architecture_body(&ws, ws.report().unwrap())).unwrap();
        assert_eq!(v["status"], "complete");
        assert_eq!(v["nodes"][0]["id"], "crate:a");
        assert_eq!(v["nodes"][0]["kind"], "crate");
        assert!(v["diff"]["state"].is_string());
        assert!(v["impact"]["direct"].is_array());
    }
}
```

> 若 `ArchitectureReport` 的字段名与上面构造体不一致(例如多了未列出的必填字段),以 `crates/dozer-codehealth/src/architecture.rs:129-146` 为准补齐;测试辅助 `arch()` 是唯一构造点,只改它。`dozer_codehealth::edge_id` / `ArchitectureEdge` / `DependencyCycle` 的导出路径以 `lib.rs` 为准(若未在 crate 根导出,改为 `dozer_codehealth::architecture::...`)。

- [ ] **Step 2: 运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo test -p dozer-app extensions::codehealth::arch_payload 2>&1 | tail -10
```

Expected: 编译失败(`architecture_body`/`MAX_PAYLOAD_NODES` 未定义)。

- [ ] **Step 3: 实现**

`mod.rs`:加 `mod arch_payload;`,`CodeHealthCategory` 加 `Architecture`,`label()` 加 `CodeHealthCategory::Architecture => "架构"`,`all()` 返回 `[CodeHealthCategory; 5]` 并追加 `CodeHealthCategory::Architecture`。

`protocol.rs`:`CategoryKey` 加 `Architecture`;`From<CodeHealthCategory>` 加对应分支;`Body` 加 `Architecture(super::arch_payload::ArchitectureBody)`;`current_view_payload` 的 match 加 `CategoryKey::Architecture => Body::Architecture(super::arch_payload::architecture_body(ws, report))`。

`view.rs`:`category_icon` 加 `CodeHealthCategory::Architecture => icons::IconKind::GitBranch`;测试 `category_labels_and_all_cover_four_categories` 改名并断言 `5`、`Architecture.label() == "架构"`。

`arch_payload.rs` 在 `use` 之后、`#[cfg(test)]` 之前写入:

```rust
/// 超过这个节点数就只发 crate 层(模块图太大,前端不遍历完整图)。
pub const MAX_PAYLOAD_NODES: usize = 3000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchNodeDto {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub qualified_name: String,
    pub path: Option<String>,
    pub parent_id: Option<String>,
    pub loc: usize,
    pub fan_in: usize,
    pub fan_out: usize,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchEdgeDto {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub evidence_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchCycleDto {
    pub id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchRiskDto {
    /// `cycle` | `hub` | `boundary`。
    pub kind: &'static str,
    pub finding: FindingRowDto,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchDiffDto {
    /// `no_baseline` | `unavailable` | `compared`。
    pub state: &'static str,
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
    pub added_cycles: Vec<String>,
    pub resolved_cycles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchImpactNodeDto {
    pub node_id: String,
    pub distance: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchImpactDto {
    pub direct: Vec<ArchImpactNodeDto>,
    pub indirect: Vec<ArchImpactNodeDto>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchitectureBody {
    /// `complete` | `partial` | `not_applicable`。
    pub status: &'static str,
    pub status_note: Option<String>,
    pub truncated_note: Option<String>,
    pub nodes: Vec<ArchNodeDto>,
    pub edges: Vec<ArchEdgeDto>,
    pub cycles: Vec<ArchCycleDto>,
    pub risks: Vec<ArchRiskDto>,
    pub diff: ArchDiffDto,
    pub impact: ArchImpactDto,
    pub errors: Vec<String>,
    pub unresolved_edges: usize,
}

fn node_kind(k: ArchitectureNodeKind) -> &'static str {
    match k {
        ArchitectureNodeKind::Workspace => "workspace",
        ArchitectureNodeKind::Crate => "crate",
        ArchitectureNodeKind::Module => "module",
        ArchitectureNodeKind::ExternalCrate => "external_crate",
    }
}

fn edge_kind(k: ArchitectureEdgeKind) -> &'static str {
    match k {
        ArchitectureEdgeKind::CargoDependency => "cargo_dependency",
        ArchitectureEdgeKind::ModuleUse => "module_use",
    }
}

fn diff_dto(outcome: &ArchitectureDiffOutcome) -> ArchDiffDto {
    let empty = ArchDiffDto {
        state: "no_baseline",
        added_nodes: vec![],
        removed_nodes: vec![],
        added_edges: vec![],
        removed_edges: vec![],
        added_cycles: vec![],
        resolved_cycles: vec![],
    };
    match outcome {
        ArchitectureDiffOutcome::NoBaseline => empty,
        ArchitectureDiffOutcome::Unavailable => ArchDiffDto {
            state: "unavailable",
            ..empty
        },
        ArchitectureDiffOutcome::Compared(d) => ArchDiffDto {
            state: "compared",
            added_nodes: d.added_nodes.clone(),
            removed_nodes: d.removed_nodes.clone(),
            added_edges: d.added_edges.clone(),
            removed_edges: d.removed_edges.clone(),
            added_cycles: d.added_cycles.clone(),
            resolved_cycles: d.resolved_cycles.clone(),
        },
    }
}

pub fn architecture_body(ws: &WorkspaceState, report: &ProjectReport) -> ArchitectureBody {
    let arch = &report.architecture;
    let (status, status_note) = match arch.status {
        ArchitectureStatus::Complete => ("complete", None),
        ArchitectureStatus::Partial => (
            "partial",
            Some("架构分析不完整(见扫描范围或下方错误),不能据此判断没有风险。".to_string()),
        ),
        ArchitectureStatus::NotApplicable => (
            "not_applicable",
            Some("没有可用的架构数据(旧版报告或项目内没有可分析的 Rust 代码),请重新扫描。".to_string()),
        ),
    };

    // 外部依赖 crate 默认隐藏:节点与触及它们的边一并丢弃。
    let external: BTreeSet<&str> = arch
        .nodes
        .iter()
        .filter(|n| n.external || n.kind == ArchitectureNodeKind::ExternalCrate)
        .map(|n| n.id.as_str())
        .collect();
    let mut kept_nodes: Vec<&dozer_codehealth::ArchitectureNode> = arch
        .nodes
        .iter()
        .filter(|n| !external.contains(n.id.as_str()))
        .collect();

    let mut truncated_note = None;
    if kept_nodes.len() > MAX_PAYLOAD_NODES {
        let total = kept_nodes.len();
        kept_nodes.retain(|n| n.kind == ArchitectureNodeKind::Crate);
        truncated_note = Some(format!(
            "模块图过大({total} 个节点),仅显示 crate 层;分析结果(循环/枢纽/越界)仍基于完整图。"
        ));
    }
    let kept_ids: BTreeSet<&str> = kept_nodes.iter().map(|n| n.id.as_str()).collect();

    let nodes: Vec<ArchNodeDto> = kept_nodes
        .iter()
        .map(|n| ArchNodeDto {
            id: n.id.clone(),
            kind: node_kind(n.kind),
            name: n.name.clone(),
            qualified_name: n.qualified_name.clone(),
            path: n.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            parent_id: n.parent_id.clone(),
            loc: n.loc,
            fan_in: n.fan_in,
            fan_out: n.fan_out,
            layer: n.layer.clone(),
        })
        .collect();

    // 全部(含被裁掉的)边的端点表,供风险映射用。
    let endpoints: BTreeMap<&str, (&str, &str)> = arch
        .edges
        .iter()
        .map(|e| (e.id.as_str(), (e.from.as_str(), e.to.as_str())))
        .collect();

    let edges: Vec<ArchEdgeDto> = arch
        .edges
        .iter()
        .filter(|e| kept_ids.contains(e.from.as_str()) && kept_ids.contains(e.to.as_str()))
        .map(|e| ArchEdgeDto {
            id: e.id.clone(),
            from: e.from.clone(),
            to: e.to.clone(),
            kind: edge_kind(e.kind),
            evidence_count: e.evidence.len(),
        })
        .collect();

    let cycles = arch
        .cycles
        .iter()
        .map(|c| ArchCycleDto {
            id: c.id.clone(),
            node_ids: c.node_ids.clone(),
            edge_ids: c.edge_ids.clone(),
        })
        .collect();

    let mut risks = Vec::new();
    for f in report
        .findings
        .iter()
        .filter(|f| matches!(f.category, dozer_codehealth::FindingCategory::Architecture))
    {
        let (kind, node_ids, edge_ids): (&'static str, Vec<String>, Vec<String>) =
            match &f.evidence {
                FindingEvidence::ArchitectureCycle { node_ids } => {
                    let set: BTreeSet<&str> = node_ids.iter().map(String::as_str).collect();
                    let eids = arch
                        .edges
                        .iter()
                        .filter(|e| {
                            set.contains(e.from.as_str()) && set.contains(e.to.as_str())
                        })
                        .map(|e| e.id.clone())
                        .collect();
                    ("cycle", node_ids.clone(), eids)
                }
                FindingEvidence::ArchitectureHub { node_id, .. } => {
                    ("hub", vec![node_id.clone()], Vec::new())
                }
                FindingEvidence::ArchitectureBoundary { edge_id, .. } => {
                    let nids = endpoints
                        .get(edge_id.as_str())
                        .map(|(a, b)| vec![a.to_string(), b.to_string()])
                        .unwrap_or_default();
                    ("boundary", nids, vec![edge_id.clone()])
                }
                _ => continue,
            };
        let _ = rule_ids::ARCHITECTURE_CYCLE; // 规则 id 常量由 `category` 过滤覆盖,此处显式引用以便后续按规则细分
        risks.push(ArchRiskDto {
            kind,
            finding: row_dto(
                f.id.clone(),
                f.title.clone(),
                &f.path,
                f.start_line,
                f.severity,
                None,
                Vec::new(),
            ),
            node_ids,
            edge_ids,
        });
    }

    let impact = ws.impact();
    let to_dto = |v: &[dozer_codehealth::ImpactNode]| {
        v.iter()
            .map(|n| ArchImpactNodeDto {
                node_id: n.node_id.clone(),
                distance: n.distance,
            })
            .collect::<Vec<_>>()
    };

    ArchitectureBody {
        status,
        status_note,
        truncated_note,
        nodes,
        edges,
        cycles,
        risks,
        diff: diff_dto(ws.architecture_diff()),
        impact: ArchImpactDto {
            direct: to_dto(&impact.direct),
            indirect: to_dto(&impact.indirect),
            truncated: impact.truncated,
        },
        errors: arch.errors.clone(),
        unresolved_edges: arch.unresolved_edges,
    }
}
```

> `let _ = rule_ids::ARCHITECTURE_CYCLE;` 这一行只是为了避免未使用导入警告,难看。**实现时直接删掉它与 `rule_ids` 的 `use`**(`category == Architecture` 已足够过滤),并从 `use dozer_codehealth::{...}` 里去掉 `rule_ids`——但测试模块仍需要 `rule_ids`,它在测试里通过 `use super::*` 拿不到时,在 `mod tests` 的 `use dozer_codehealth::{...}` 里加上 `rule_ids`。

- [ ] **Step 4: 运行测试**

```bash
cargo test -p dozer-app extensions::codehealth 2>&1 | tail -15
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c | head
```

Expected: 全绿。Task 1 的 `protocol.rs` 里 `category` match 现在要穷尽 5 个值,`payload_serializes_with_kind_tags` 等旧测试不受影响。

- [ ] **Step 5: 重新构建前端以包含新类型?**

本任务没改前端,**不需要**重新构建。提交:

```bash
git add crates/dozer-app/src/extensions/codehealth
git diff --cached --stat
git commit -m "feat(codehealth): architecture payload and architecture category

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

> 此时 GUI 里点「架构」会推一个前端还不认识的 `body.kind = "architecture"`;前端 `App` 没有对应分支会渲染空白(不崩)。Task 11 接入前不要把这个状态单独发版。

---

### Task 9: 前端图投影 `graphProjection.ts`(纯函数 + 测试)

**Files:**
- Create: `web/codehealth-content/src/graph/projection.ts`、`projection.test.ts`
- Modify: `web/codehealth-content/src/types.ts`(加架构类型)

**Interfaces:**
- Consumes: Task 8 的 JSON 形状。
- Produces:
  - 类型 `ArchitectureBody`、`ArchNode`、`ArchEdge`、`ArchRisk`
  - `projectGraph(input: ProjectionInput): VisibleGraph`
  - `interface ProjectionInput { nodes: ArchNode[]; edges: ArchEdge[]; layer: 'crate' | 'module'; expanded: ReadonlySet<string>; riskOnly: boolean; riskNodeIds: ReadonlySet<string>; riskEdgeIds: ReadonlySet<string> }`
  - `interface VNode { id: string; label: string; kind: string; loc: number; childCount: number; collapsed: boolean; representedIds: string[] }`
  - `interface VEdge { id: string; from: string; to: string; count: number; edgeIds: string[] }`
  - `interface VisibleGraph { nodes: VNode[]; edges: VEdge[] }`
  - 语义:
    - `crate` 层:可见 = `kind==='crate'` 的节点;边 = `cargo_dependency`。
    - `module` 层:可见 = 其祖先链(到 crate 为止)的模块全部在 `expanded` 中,或父为 crate(根模块);折叠且有子的节点 `collapsed=true`、`childCount` = 直接子模块数;被折叠隐藏的后代映射到最近的可见祖先;只用 `module_use` 边;映射后两端相同的边丢弃;同一对端点的多条边合并为一条(`count` 累加、`edgeIds` 合并去重,边 id 为 `from->to`)。
    - `riskOnly`:只保留 `riskNodeIds` 中节点(在 module 层按"代表节点"判断:`representedIds` 与 `riskNodeIds` 有交集即保留)及其之间的边;边另外也保留 `riskEdgeIds` 命中的(两端点必须保留)。
    - 输出节点按 `id` 升序、边按 `id` 升序(确定性输入,保证布局稳定)。

- [ ] **Step 1: 写失败测试**

`src/types.ts` 末尾追加:

```ts
export interface ArchNode {
  id: string;
  kind: 'workspace' | 'crate' | 'module' | 'external_crate';
  name: string;
  qualified_name: string;
  path: string | null;
  parent_id: string | null;
  loc: number;
  fan_in: number;
  fan_out: number;
  layer: string | null;
}

export interface ArchEdge {
  id: string;
  from: string;
  to: string;
  kind: 'cargo_dependency' | 'module_use';
  evidence_count: number;
}

export interface ArchRisk {
  kind: 'cycle' | 'hub' | 'boundary';
  finding: FindingRow;
  node_ids: string[];
  edge_ids: string[];
}

export interface ArchImpactNode {
  node_id: string;
  distance: number;
}

export interface ArchitectureBody {
  kind: 'architecture';
  status: 'complete' | 'partial' | 'not_applicable';
  status_note: string | null;
  truncated_note: string | null;
  nodes: ArchNode[];
  edges: ArchEdge[];
  cycles: { id: string; node_ids: string[]; edge_ids: string[] }[];
  risks: ArchRisk[];
  diff: {
    state: 'no_baseline' | 'unavailable' | 'compared';
    added_nodes: string[];
    removed_nodes: string[];
    added_edges: string[];
    removed_edges: string[];
    added_cycles: string[];
    resolved_cycles: string[];
  };
  impact: { direct: ArchImpactNode[]; indirect: ArchImpactNode[]; truncated: boolean };
  errors: string[];
  unresolved_edges: number;
}
```

并把 `Body` 联合改为 `EmptyBody | OverviewBody | StructureBody | UiBody | ScopeBody | ArchitectureBody`,`CategoryKey` 加 `'architecture'`。

`src/graph/projection.test.ts`:

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { projectGraph, type ProjectionInput } from './projection.ts';
import type { ArchNode, ArchEdge } from '../types.ts';

const n = (id: string, kind: ArchNode['kind'], parent: string | null, loc = 10): ArchNode => ({
  id, kind, name: id, qualified_name: id.replace(/^(module|crate):/, ''),
  path: null, parent_id: parent, loc, fan_in: 0, fan_out: 0, layer: null,
});
const e = (kind: ArchEdge['kind'], from: string, to: string): ArchEdge => ({
  id: `edge:${kind}:${from}:${to}`, from, to, kind, evidence_count: 1,
});

const nodes: ArchNode[] = [
  n('crate:a', 'crate', null),
  n('crate:b', 'crate', null),
  n('module:a', 'module', 'crate:a'),
  n('module:a::x', 'module', 'module:a'),
  n('module:a::x::deep', 'module', 'module:a::x'),
  n('module:a::y', 'module', 'module:a'),
  n('module:b', 'module', 'crate:b'),
];
const edges: ArchEdge[] = [
  e('cargo_dependency', 'crate:a', 'crate:b'),
  e('module_use', 'module:a::x', 'module:a::y'),
  e('module_use', 'module:a::x::deep', 'module:a::y'),
  e('module_use', 'module:a::y', 'module:b'),
];
const base: ProjectionInput = {
  nodes, edges, layer: 'crate', expanded: new Set(), riskOnly: false,
  riskNodeIds: new Set(), riskEdgeIds: new Set(),
};

test('crate layer shows crates and cargo edges only', () => {
  const g = projectGraph(base);
  assert.deepEqual(g.nodes.map((v) => v.id), ['crate:a', 'crate:b']);
  assert.deepEqual(g.edges.map((v) => [v.from, v.to]), [['crate:a', 'crate:b']]);
});

test('module layer collapsed shows only root modules, with child counts', () => {
  const g = projectGraph({ ...base, layer: 'module' });
  assert.deepEqual(g.nodes.map((v) => v.id), ['module:a', 'module:b']);
  const a = g.nodes.find((v) => v.id === 'module:a')!;
  assert.equal(a.collapsed, true);
  assert.equal(a.childCount, 2);
  const b = g.nodes.find((v) => v.id === 'module:b')!;
  assert.equal(b.collapsed, false);
  assert.equal(b.childCount, 0);
  // a 的内部边(x→y)映射后两端相同,被丢弃;y→b 映射成 a→b。
  assert.deepEqual(g.edges.map((v) => [v.from, v.to]), [['module:a', 'module:b']]);
});

test('expanding a module reveals its children and routes edges to them', () => {
  const g = projectGraph({ ...base, layer: 'module', expanded: new Set(['module:a']) });
  assert.deepEqual(
    g.nodes.map((v) => v.id),
    ['module:a', 'module:a::x', 'module:a::y', 'module:b'].sort(),
  );
  const x = g.nodes.find((v) => v.id === 'module:a::x')!;
  assert.equal(x.collapsed, true, 'x 有子模块 deep 但未展开');
  const pairs = g.edges.map((v) => `${v.from}->${v.to}`).sort();
  // deep 折叠进 x:x→y 有两条(x→y 与 deep→y)合并为 count=2。
  assert.deepEqual(pairs, ['module:a::x->module:a::y', 'module:a::y->module:b']);
  const xy = g.edges.find((v) => v.from === 'module:a::x' && v.to === 'module:a::y')!;
  assert.equal(xy.count, 2);
  assert.equal(xy.edgeIds.length, 2);
});

test('collapsed node represents its hidden descendants', () => {
  const g = projectGraph({ ...base, layer: 'module', expanded: new Set(['module:a']) });
  const x = g.nodes.find((v) => v.id === 'module:a::x')!;
  assert.deepEqual([...x.representedIds].sort(), ['module:a::x', 'module:a::x::deep']);
});

test('riskOnly keeps risk nodes (via represented ids) and edges between kept nodes', () => {
  const g = projectGraph({
    ...base, layer: 'module', expanded: new Set(['module:a']), riskOnly: true,
    riskNodeIds: new Set(['module:a::x::deep', 'module:a::y']),
  });
  // x 代表 deep(风险)→保留;y 风险→保留;a、b 不保留。
  assert.deepEqual(g.nodes.map((v) => v.id), ['module:a::x', 'module:a::y']);
  assert.deepEqual(g.edges.map((v) => [v.from, v.to]), [['module:a::x', 'module:a::y']]);
});

// 布局稳定性的前提:输入顺序无关,输出按 id 升序。
test('output is independent of input order', () => {
  const g1 = projectGraph({ ...base, layer: 'module', expanded: new Set(['module:a']) });
  const g2 = projectGraph({
    ...base, layer: 'module', expanded: new Set(['module:a']),
    nodes: [...nodes].reverse(), edges: [...edges].reverse(),
  });
  assert.deepEqual(g1, g2);
});

test('crate layer ignores module nodes even when expanded set is non-empty', () => {
  const g = projectGraph({ ...base, expanded: new Set(['module:a']) });
  assert.deepEqual(g.nodes.map((v) => v.id), ['crate:a', 'crate:b']);
});
```

- [ ] **Step 2: 运行确认失败**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content
node --test src/graph/projection.test.ts 2>&1 | tail -8
```

Expected: `Cannot find module './projection.ts'`。同时 `package.json` 的 `test` 脚本通配 `src/*.test.ts` 不包含子目录,**把它改为** `"node --test src/*.test.ts src/graph/*.test.ts && node render-smoke.mjs"`。

- [ ] **Step 3: 实现**

`src/graph/projection.ts`:

```ts
import type { ArchEdge, ArchNode } from '../types.ts';

export interface ProjectionInput {
  nodes: ArchNode[];
  edges: ArchEdge[];
  layer: 'crate' | 'module';
  /** 已展开的模块 id。根模块(父为 crate)恒可见,其子级可见当且仅当父在此集合内。 */
  expanded: ReadonlySet<string>;
  riskOnly: boolean;
  riskNodeIds: ReadonlySet<string>;
  riskEdgeIds: ReadonlySet<string>;
}

export interface VNode {
  id: string;
  label: string;
  kind: string;
  loc: number;
  childCount: number;
  collapsed: boolean;
  /** 本可见节点所代表的原始节点 id(自身 + 被折叠隐藏的后代),升序。 */
  representedIds: string[];
}

export interface VEdge {
  id: string;
  from: string;
  to: string;
  count: number;
  edgeIds: string[];
}

export interface VisibleGraph {
  nodes: VNode[];
  edges: VEdge[];
}

const byId = <T extends { id: string }>(a: T, b: T) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

export function projectGraph(input: ProjectionInput): VisibleGraph {
  const g =
    input.layer === 'crate' ? projectCrateLayer(input) : projectModuleLayer(input);
  return input.riskOnly ? filterRisk(g, input) : g;
}

function projectCrateLayer(input: ProjectionInput): VisibleGraph {
  const crates = input.nodes.filter((n) => n.kind === 'crate').sort(byId);
  const ids = new Set(crates.map((c) => c.id));
  const nodes: VNode[] = crates.map((c) => ({
    id: c.id,
    label: c.name,
    kind: c.kind,
    loc: c.loc,
    childCount: 0,
    collapsed: false,
    representedIds: [c.id],
  }));
  const edges: VEdge[] = input.edges
    .filter((e) => e.kind === 'cargo_dependency' && ids.has(e.from) && ids.has(e.to))
    .map((e) => ({ id: `${e.from}->${e.to}`, from: e.from, to: e.to, count: 1, edgeIds: [e.id] }))
    .sort(byId);
  return { nodes, edges };
}

function projectModuleLayer(input: ProjectionInput): VisibleGraph {
  const modules = input.nodes.filter((n) => n.kind === 'module');
  const modById = new Map(modules.map((m) => [m.id, m]));
  const children = new Map<string, string[]>();
  for (const m of modules) {
    if (m.parent_id && modById.has(m.parent_id)) {
      const list = children.get(m.parent_id) ?? [];
      list.push(m.id);
      children.set(m.parent_id, list);
    }
  }

  // 递归决定可见集合与"代表映射"(每个原始模块 → 最近的可见祖先或自身)。
  const rep = new Map<string, string>();
  const visible = new Set<string>();
  const roots = modules.filter((m) => !m.parent_id || !modById.has(m.parent_id));
  const walk = (id: string, visibleAncestor: string | null) => {
    const self = visibleAncestor ?? id;
    rep.set(id, self);
    if (!visibleAncestor) visible.add(id);
    const open = !visibleAncestor && input.expanded.has(id);
    for (const c of children.get(id) ?? []) {
      // 当前节点可见且已展开 → 子节点自己可见;否则子节点被折叠进当前可见节点。
      walk(c, open ? null : self);
    }
  };
  for (const r of roots) walk(r.id, null);

  const represented = new Map<string, string[]>();
  for (const [orig, v] of rep) {
    const list = represented.get(v) ?? [];
    list.push(orig);
    represented.set(v, list);
  }

  const nodes: VNode[] = [...visible]
    .map((id) => modById.get(id)!)
    .sort(byId)
    .map((m) => {
      const kids = children.get(m.id) ?? [];
      return {
        id: m.id,
        label: m.qualified_name,
        kind: m.kind,
        loc: m.loc,
        childCount: kids.length,
        collapsed: kids.length > 0 && !input.expanded.has(m.id),
        representedIds: (represented.get(m.id) ?? [m.id]).slice().sort(),
      };
    });

  const merged = new Map<string, VEdge>();
  for (const e of input.edges) {
    if (e.kind !== 'module_use') continue;
    const from = rep.get(e.from);
    const to = rep.get(e.to);
    if (!from || !to || from === to) continue;
    const key = `${from}->${to}`;
    const cur = merged.get(key);
    if (cur) {
      cur.count += 1;
      if (!cur.edgeIds.includes(e.id)) cur.edgeIds.push(e.id);
    } else {
      merged.set(key, { id: key, from, to, count: 1, edgeIds: [e.id] });
    }
  }
  const edges = [...merged.values()]
    .map((v) => ({ ...v, edgeIds: v.edgeIds.slice().sort() }))
    .sort(byId);
  return { nodes, edges };
}

function filterRisk(g: VisibleGraph, input: ProjectionInput): VisibleGraph {
  const keep = new Set(
    g.nodes
      .filter((n) => n.representedIds.some((id) => input.riskNodeIds.has(id)))
      .map((n) => n.id),
  );
  const nodes = g.nodes.filter((n) => keep.has(n.id));
  const edges = g.edges.filter((e) => keep.has(e.from) && keep.has(e.to));
  return { nodes, edges };
}
```

- [ ] **Step 4: 运行测试**

```bash
node --test src/graph/projection.test.ts 2>&1 | tail -15
npm run typecheck 2>&1 | tail -5
```

Expected: 7 条全绿。若 `riskOnly` 测试里 `riskEdgeIds` 未参与,属预期(`riskEdgeIds` 保留在 `ProjectionInput` 里供 Task 10 做边高亮,**此处不用它过滤**——若 `noUnusedParameters` 报错,说明未使用字段只是对象属性,不会报错;不要为此删字段)。

- [ ] **Step 5: 提交**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
git add crates/dozer-app/web/codehealth-content/src crates/dozer-app/web/codehealth-content/package.json
git diff --cached --stat
git commit -m "feat(codehealth): graph projection for expand/collapse and risk-only views

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Cytoscape + dagre 布局的确定性与图组件

**Files:**
- Modify: `web/codehealth-content/package.json`(加依赖)
- Create: `web/codehealth-content/src/graph/layout.ts`、`layout.test.ts`
- Create: `web/codehealth-content/src/components/GraphCanvas.tsx`
- Modify: `web/codehealth-content/build.mjs`(无需改,依赖自动打包;仅确认体积)

**Interfaces:**
- Consumes: Task 9 的 `VisibleGraph`。
- Produces:
  - `computeLayout(graph: VisibleGraph): Map<string, { x: number; y: number }>`(dagre 从左到右 `rankdir: 'LR'`,**纯计算,不依赖 DOM**,可在 node 里测)
  - `GraphCanvas` 组件 props:`{ graph: VisibleGraph; selectedId: string | null; riskNodeIds: ReadonlySet<string>; riskEdgeIds: ReadonlySet<string>; impactNodeIds: ReadonlySet<string>; addedNodeIds: ReadonlySet<string>; focusId: string | null; fitSignal: number; onSelect(id: string | null): void; onToggleExpand(id: string): void }`

> 设计取舍(与 spec 的差异,在此记录):spec 写"dagre 对复合节点展开/折叠重排是否稳定,需验证"。本计划**不使用 Cytoscape 复合节点**,展开/折叠由 Task 9 的纯函数投影完成,Cytoscape 只画扁平图,布局用 dagre 单独计算(`dagre` 包本身,不经 `cytoscape-dagre` 插件),因此复合节点稳定性风险被规避;Cytoscape 用 `preset` 布局吃我们算好的坐标。依赖只需 `cytoscape` + `dagre`,**不装** `cytoscape-dagre`、`cytoscape-expand-collapse`。elk 不引入(`elkjs` 为 EPL-2.0/GPL 双许可,与本仓库其余 MIT 依赖不同,需要时另行评估)。

- [ ] **Step 1: 安装依赖**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content
npm install --save-exact cytoscape@3.34.3 dagre@0.8.5
npm install --save-dev --save-exact @types/dagre@0.7.54
npm ls cytoscape dagre @types/dagre 2>&1 | tail -6
```

Expected: 三者安装成功(cytoscape 自带类型 `index.d.ts`,不需要 `@types/cytoscape`)。

- [ ] **Step 2: 写失败测试(布局确定性)**

`src/graph/layout.test.ts`:

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { computeLayout } from './layout.ts';
import type { VisibleGraph } from './projection.ts';

const vn = (id: string) => ({
  id, label: id, kind: 'module', loc: 10, childCount: 0, collapsed: false, representedIds: [id],
});
const graph: VisibleGraph = {
  nodes: ['a', 'b', 'c', 'd'].map(vn),
  edges: [
    { id: 'a->b', from: 'a', to: 'b', count: 1, edgeIds: ['e1'] },
    { id: 'a->c', from: 'a', to: 'c', count: 1, edgeIds: ['e2'] },
    { id: 'b->d', from: 'b', to: 'd', count: 1, edgeIds: ['e3'] },
    { id: 'c->d', from: 'c', to: 'd', count: 1, edgeIds: ['e4'] },
  ],
};

test('every node gets finite coordinates', () => {
  const pos = computeLayout(graph);
  assert.equal(pos.size, 4);
  for (const { x, y } of pos.values()) {
    assert.ok(Number.isFinite(x) && Number.isFinite(y));
  }
});

// 原 spec 验收项:同一份图连续两次布局节点坐标逐点相同。
test('layout is deterministic across runs', () => {
  const a = [...computeLayout(graph).entries()];
  const b = [...computeLayout(graph).entries()];
  assert.deepEqual(a, b);
});

test('left-to-right: a source is left of its dependents', () => {
  const pos = computeLayout(graph);
  assert.ok(pos.get('a')!.x < pos.get('b')!.x);
  assert.ok(pos.get('b')!.x < pos.get('d')!.x);
});

test('cycles do not throw and still place every node', () => {
  const cyc: VisibleGraph = {
    nodes: ['x', 'y'].map(vn),
    edges: [
      { id: 'x->y', from: 'x', to: 'y', count: 1, edgeIds: ['1'] },
      { id: 'y->x', from: 'y', to: 'x', count: 1, edgeIds: ['2'] },
    ],
  };
  assert.equal(computeLayout(cyc).size, 2);
});

test('empty graph yields empty layout', () => {
  assert.equal(computeLayout({ nodes: [], edges: [] }).size, 0);
});
```

```bash
node --test src/graph/layout.test.ts 2>&1 | tail -6
```

Expected: FAIL(`./layout.ts` 不存在)。

- [ ] **Step 3: 实现 `layout.ts`**

```ts
import dagre from 'dagre';
import type { VisibleGraph } from './projection.ts';

const NODE_W = 160;
const NODE_H = 36;

/** 用 dagre 做从左到右的分层布局,返回每个节点中心坐标。纯计算、不依赖 DOM;
 *  输入已按 id 升序(见 `projectGraph`),dagre 对相同输入给出相同输出,
 *  这是"重新扫描后布局不随机跳动"的保证。 */
export function computeLayout(graph: VisibleGraph): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>();
  if (graph.nodes.length === 0) return out;
  const g = new dagre.graphlib.Graph({ multigraph: false });
  g.setGraph({ rankdir: 'LR', nodesep: 24, ranksep: 70, marginx: 20, marginy: 20 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const n of graph.nodes) g.setNode(n.id, { width: NODE_W, height: NODE_H });
  for (const e of graph.edges) g.setEdge(e.from, e.to);
  dagre.layout(g);
  for (const n of graph.nodes) {
    const p = g.node(n.id);
    out.set(n.id, { x: p.x, y: p.y });
  }
  return out;
}

export const NODE_SIZE = { w: NODE_W, h: NODE_H };
```

```bash
node --test src/graph/layout.test.ts 2>&1 | tail -10
```

Expected: 5 条全绿。`import dagre from 'dagre'` 在 node 的 ESM 类型剥离下是否可用:若报 `does not provide an export named default`,把导入改成 `import * as dagreNs from 'dagre'; const dagre = (dagreNs as any).default ?? dagreNs;`(dagre 0.8.5 是 CJS,Node ESM 会把 `module.exports` 当 default;esbuild 打包则两种写法都行)。以测试通过为准,**`typecheck` 也要过**(`esModuleInterop` 缺省时 `import dagre from` 可能报错,此时 tsconfig 加 `"allowSyntheticDefaultImports": true`)。

- [ ] **Step 4: 实现 `GraphCanvas.tsx`**

```tsx
import { useEffect, useRef } from 'preact/hooks';
import cytoscape, { type Core, type ElementDefinition } from 'cytoscape';
import type { VisibleGraph } from '../graph/projection.ts';
import { computeLayout, NODE_SIZE } from '../graph/layout.ts';

export interface GraphCanvasProps {
  graph: VisibleGraph;
  selectedId: string | null;
  riskNodeIds: ReadonlySet<string>;
  riskEdgeIds: ReadonlySet<string>;
  impactNodeIds: ReadonlySet<string>;
  addedNodeIds: ReadonlySet<string>;
  /** 变化时把该节点居中(列表点击定位)。 */
  focusId: string | null;
  /** 值变化时适配窗口(「适配窗口 / 重置视图」按钮)。 */
  fitSignal: number;
  onSelect(id: string | null): void;
  onToggleExpand(id: string): void;
}

function css(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function styleSheet(): cytoscape.StylesheetJson {
  return [
    {
      selector: 'node',
      style: {
        shape: 'round-rectangle',
        width: NODE_SIZE.w,
        height: NODE_SIZE.h,
        label: 'data(label)',
        'font-size': 11,
        'font-family': '-apple-system, "PingFang SC", sans-serif',
        color: css('--cream'),
        'text-valign': 'center',
        'text-halign': 'center',
        'text-wrap': 'ellipsis',
        'text-max-width': `${NODE_SIZE.w - 16}px`,
        'background-color': css('--card'),
        'border-width': 1,
        'border-color': css('--border'),
      },
    },
    { selector: 'node[?collapsed]', style: { 'border-style': 'double', 'border-width': 3 } },
    { selector: 'node.impact', style: { 'border-color': css('--cyan'), 'border-width': 2 } },
    { selector: 'node.added', style: { 'border-color': css('--green'), 'border-width': 2 } },
    { selector: 'node.risk', style: { 'border-color': css('--red'), 'border-width': 3 } },
    { selector: 'node:selected', style: { 'border-color': css('--gold'), 'border-width': 3 } },
    {
      selector: 'edge',
      style: {
        width: 1.2,
        'curve-style': 'bezier',
        'line-color': css('--dim'),
        'target-arrow-color': css('--dim'),
        'target-arrow-shape': 'triangle',
        'arrow-scale': 0.9,
      },
    },
    {
      selector: 'edge.risk',
      style: { 'line-color': css('--red'), 'target-arrow-color': css('--red'), width: 2.2 },
    },
  ] as cytoscape.StylesheetJson;
}

function elements(p: GraphCanvasProps): ElementDefinition[] {
  const pos = computeLayout(p.graph);
  const nodes: ElementDefinition[] = p.graph.nodes.map((n) => ({
    group: 'nodes',
    data: {
      id: n.id,
      label: n.collapsed ? `${n.label} (+${n.childCount})` : n.label,
      collapsed: n.collapsed,
    },
    position: pos.get(n.id)!,
    classes: [
      n.representedIds.some((id) => p.riskNodeIds.has(id)) ? 'risk' : '',
      n.representedIds.some((id) => p.impactNodeIds.has(id)) ? 'impact' : '',
      n.representedIds.some((id) => p.addedNodeIds.has(id)) ? 'added' : '',
    ]
      .filter(Boolean)
      .join(' '),
  }));
  const edges: ElementDefinition[] = p.graph.edges.map((e) => ({
    group: 'edges',
    data: { id: e.id, source: e.from, target: e.to },
    classes: e.edgeIds.some((id) => p.riskEdgeIds.has(id)) ? 'risk' : '',
  }));
  return [...nodes, ...edges];
}

export function GraphCanvas(props: GraphCanvasProps) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const handlers = useRef(props);
  handlers.current = props;

  // 创建一次。
  useEffect(() => {
    if (!host.current) return;
    const core = cytoscape({
      container: host.current,
      elements: [],
      style: styleSheet(),
      layout: { name: 'preset' },
      wheelSensitivity: 0.3,
      minZoom: 0.1,
      maxZoom: 3,
      boxSelectionEnabled: false,
      autounselectify: false,
    });
    core.on('tap', 'node', (ev) => handlers.current.onSelect(ev.target.id()));
    core.on('tap', (ev) => {
      if (ev.target === core) handlers.current.onSelect(null);
    });
    core.on('dbltap', 'node', (ev) => handlers.current.onToggleExpand(ev.target.id()));
    cy.current = core;
    return () => {
      core.destroy();
      cy.current = null;
    };
  }, []);

  // 图或高亮集合变化 → 重建元素并 preset 布局(坐标来自 dagre,确定性)。
  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    const hadNodes = core.nodes().length > 0;
    core.batch(() => {
      core.elements().remove();
      core.add(elements(props));
    });
    core.layout({ name: 'preset' }).run();
    if (!hadNodes) core.fit(undefined, 24);
    if (props.selectedId) core.getElementById(props.selectedId).select();
  }, [props.graph, props.riskNodeIds, props.riskEdgeIds, props.impactNodeIds, props.addedNodeIds]);

  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    core.elements().unselect();
    if (props.selectedId) core.getElementById(props.selectedId).select();
  }, [props.selectedId]);

  useEffect(() => {
    const core = cy.current;
    if (!core || !props.focusId) return;
    const n = core.getElementById(props.focusId);
    if (n.nonempty()) core.animate({ center: { eles: n }, zoom: Math.max(core.zoom(), 1) }, { duration: 200 });
  }, [props.focusId]);

  useEffect(() => {
    cy.current?.fit(undefined, 24);
  }, [props.fitSignal]);

  return <div class="graph-canvas" ref={host} />;
}
```

在 `styles.css` 末尾追加:

```css
/* 架构页 */
.arch-layout { display: flex; flex-direction: column; gap: 10px; height: calc(100vh - 28px); }
.arch-main { display: flex; gap: 10px; min-height: 0; flex: 1; }
.graph-canvas {
  flex: 1; min-width: 0; min-height: 320px;
  background: var(--bg); border: 1px solid var(--border); border-radius: 12px;
}
.arch-side { width: 260px; flex: none; overflow-y: auto; }
.arch-toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.switch { display: inline-flex; align-items: center; gap: 6px; cursor: pointer; user-select: none; }
.legend { display: flex; gap: 12px; flex-wrap: wrap; font-size: 11px; color: var(--dim); }
.dot { display: inline-block; width: 9px; height: 9px; border-radius: 50%; margin-right: 4px; }
.dot.risk { background: var(--red); } .dot.impact { background: var(--cyan); } .dot.added { background: var(--green); }
.risk-item.active { border-color: var(--gold); }
.detail dt { color: var(--dim); font-size: 11px; margin-top: 6px; }
.detail dd { margin: 0; word-break: break-all; }
```

- [ ] **Step 5: 类型检查 + 构建体积记录**

```bash
npm run typecheck 2>&1 | tail -8
npm run build 2>&1 | tail -4
ls -l ../../assets/codehealth-content/codehealth-content.js
```

Expected: typecheck 通过(若 `cytoscape.StylesheetJson` 类型名在 3.34 里改了,以 `index.d.ts` 里导出的样式类型为准:`grep -n "StylesheetJson\|StylesheetStyle\|StylesheetCSS" node_modules/cytoscape/index.d.ts | head`,替换后再跑 typecheck)。**记录** `codehealth-content.js` 的字节数,写进提交消息(spec 风险 2 的数据点;若超过 1.5 MB,向用户报告而不是悄悄接受)。

- [ ] **Step 6: 提交**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
git add crates/dozer-app/web/codehealth-content/package.json crates/dozer-app/web/codehealth-content/package-lock.json crates/dozer-app/web/codehealth-content/tsconfig.json crates/dozer-app/web/codehealth-content/src
git diff --cached --stat
git commit -m "feat(codehealth): dagre layout and cytoscape graph canvas

Bundle size: <填入 codehealth-content.js 的实际字节数>

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

> 提交消息里的尖括号处**必须**替换成 Step 5 记录的真实数字后再提交。产物 `assets/` 在 Task 11 一起重新构建提交(本任务尚未接入 App,不进产物)。

---

### Task 11: 架构页组件(联动、空态、影响范围)+ 接入

**Files:**
- Create: `web/codehealth-content/src/components/ArchitecturePage.tsx`
- Create: `web/codehealth-content/src/graph/linking.ts`、`linking.test.ts`
- Modify: `web/codehealth-content/src/components/App.tsx`、`src/fixtures.ts`、`src/render-smoke.tsx`
- Modify(重新构建后提交): `assets/codehealth-content/*`

**Interfaces:**
- Consumes: Task 8 的 `ArchitectureBody`、Task 9 的 `projectGraph`、Task 10 的 `GraphCanvas`。
- Produces:
  - `riskIndex(risks: ArchRisk[]): { nodeIds: Set<string>; edgeIds: Set<string> }`
  - `riskForNode(risks: ArchRisk[], representedIds: string[]): ArchRisk[]`(点画布节点 → 命中的风险,列表据此高亮)
  - `impactNodeIds(impact): Set<string>`、`addedNodeIds(diff): Set<string>`(仅 `diff.state === 'compared'` 时非空,**`no_baseline`/`unavailable` 必须返回空集,不得把现存节点标成"新增"**)
  - `ArchitecturePage({ body }: { body: ArchitectureBody })`

- [ ] **Step 1: 写失败测试**

`src/graph/linking.test.ts`:

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { riskIndex, riskForNode, impactNodeIds, addedNodeIds } from './linking.ts';
import type { ArchRisk, ArchitectureBody } from '../types.ts';
import { row } from '../fixtures.ts';

const risks: ArchRisk[] = [
  { kind: 'cycle', finding: row({ id: 'r1' }), node_ids: ['m:x', 'm:y'], edge_ids: ['e1', 'e2'] },
  { kind: 'hub', finding: row({ id: 'r2' }), node_ids: ['m:x'], edge_ids: [] },
];

test('riskIndex unions node and edge ids', () => {
  const idx = riskIndex(risks);
  assert.deepEqual([...idx.nodeIds].sort(), ['m:x', 'm:y']);
  assert.deepEqual([...idx.edgeIds].sort(), ['e1', 'e2']);
});

test('riskForNode matches by any represented id', () => {
  assert.deepEqual(riskForNode(risks, ['m:y']).map((r) => r.finding.id), ['r1']);
  assert.deepEqual(riskForNode(risks, ['m:x']).map((r) => r.finding.id), ['r1', 'r2']);
  assert.deepEqual(riskForNode(risks, ['m:z']), []);
});

test('impactNodeIds merges direct and indirect', () => {
  const s = impactNodeIds({
    direct: [{ node_id: 'a', distance: 1 }],
    indirect: [{ node_id: 'b', distance: 2 }],
    truncated: false,
  });
  assert.deepEqual([...s].sort(), ['a', 'b']);
});

type Diff = ArchitectureBody['diff'];
const diff = (state: Diff['state']): Diff => ({
  state, added_nodes: ['n1'], removed_nodes: [], added_edges: [], removed_edges: [],
  added_cycles: [], resolved_cycles: [],
});

// Review Focus 4:没有基线/不可用时不得把现存节点标成"本轮新增"。
test('added nodes only exist when there is a real baseline', () => {
  assert.deepEqual([...addedNodeIds(diff('compared'))], ['n1']);
  assert.equal(addedNodeIds(diff('no_baseline')).size, 0);
  assert.equal(addedNodeIds(diff('unavailable')).size, 0);
});
```

在 `src/fixtures.ts` 末尾追加架构 fixture:

```ts
import type { ArchitectureBody, ArchNode, ArchEdge } from './types.ts';

const an = (id: string, kind: ArchNode['kind'], parent: string | null): ArchNode => ({
  id, kind, name: id.replace(/^(crate|module):/, ''), qualified_name: id.replace(/^(crate|module):/, ''),
  path: null, parent_id: parent, loc: 10, fan_in: 1, fan_out: 1, layer: null,
});
const ae = (kind: ArchEdge['kind'], from: string, to: string): ArchEdge => ({
  id: `edge:${kind}:${from}:${to}`, from, to, kind, evidence_count: 1,
});

export const archBody: ArchitectureBody = {
  kind: 'architecture',
  status: 'complete',
  status_note: null,
  truncated_note: null,
  nodes: [
    an('crate:a', 'crate', null), an('crate:b', 'crate', null),
    an('module:a', 'module', 'crate:a'), an('module:a::x', 'module', 'module:a'),
    an('module:a::y', 'module', 'module:a'),
  ],
  edges: [
    ae('cargo_dependency', 'crate:a', 'crate:b'),
    ae('module_use', 'module:a::x', 'module:a::y'),
    ae('module_use', 'module:a::y', 'module:a::x'),
  ],
  cycles: [{ id: 'c1', node_ids: ['module:a::x', 'module:a::y'], edge_ids: [] }],
  risks: [
    {
      kind: 'cycle',
      finding: row({ id: 'arch-cycle-1', title: '循环依赖', change: null, change_label: null, reasons: [] }),
      node_ids: ['module:a::x', 'module:a::y'],
      edge_ids: [],
    },
  ],
  diff: {
    state: 'no_baseline', added_nodes: [], removed_nodes: [], added_edges: [], removed_edges: [],
    added_cycles: [], resolved_cycles: [],
  },
  impact: { direct: [], indirect: [], truncated: false },
  errors: [],
  unresolved_edges: 0,
};

export const archFixture: ViewPayload = { scan: scanIdle, category: 'architecture', body: archBody };
export const archNotApplicableFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: {
    ...archBody, status: 'not_applicable', nodes: [], edges: [], cycles: [], risks: [],
    status_note: '没有可用的架构数据(旧版报告或项目内没有可分析的 Rust 代码),请重新扫描。',
  },
};
export const archPartialFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: { ...archBody, status: 'partial', status_note: '架构分析不完整(见扫描范围或下方错误),不能据此判断没有风险。', errors: ['cargo metadata 失败'] },
};
export const archTruncatedFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: { ...archBody, truncated_note: '模块图过大(3005 个节点),仅显示 crate 层;分析结果(循环/枢纽/越界)仍基于完整图。' },
};
```

> `fixtures.ts` 顶部已有 `import type { ViewPayload, FindingRow, ScanBar } from './types.ts';`,把新的 `import type { ArchitectureBody, ArchNode, ArchEdge }` 合并进该行,不要出现两条同源 import。

在 `render-smoke.tsx` 追加(冒烟不挂 Cytoscape——`GraphCanvas` 在服务端渲染时只输出一个空 `div`,`useEffect` 不执行,所以安全):

```tsx
import { archFixture, archNotApplicableFixture, archPartialFixture, archTruncatedFixture } from './fixtures.ts';

test('architecture: toolbar, layer switch, risk list with text labels', () => {
  const out = html(archFixture);
  assert.match(out, /架构地图/);
  assert.match(out, /crate/);
  assert.match(out, /module/);
  assert.match(out, /仅看风险/);
  assert.match(out, /适配窗口/);
  assert.match(out, /循环依赖/);
  assert.match(out, /graph-canvas/);
});

// Review Focus 4:没有架构数据时不能显示成零风险。
test('architecture: not applicable shows the note and no canvas', () => {
  const out = html(archNotApplicableFixture);
  assert.match(out, /请重新扫描/);
  assert.doesNotMatch(out, /graph-canvas/);
  assert.doesNotMatch(out, /无风险/);
});

test('architecture: partial shows warning and errors', () => {
  const out = html(archPartialFixture);
  assert.match(out, /不能据此判断没有风险/);
  assert.match(out, /cargo metadata 失败/);
});

test('architecture: truncated shows the truncation note', () => {
  assert.match(html(archTruncatedFixture), /仅显示 crate 层/);
});
```

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview/crates/dozer-app/web/codehealth-content
npm test 2>&1 | tail -15
```

Expected: FAIL(`linking.ts` 不存在、`App` 未分派 architecture)。

- [ ] **Step 2: 实现 `linking.ts`**

```ts
import type { ArchitectureBody, ArchRisk } from '../types.ts';

export function riskIndex(risks: ArchRisk[]): { nodeIds: Set<string>; edgeIds: Set<string> } {
  const nodeIds = new Set<string>();
  const edgeIds = new Set<string>();
  for (const r of risks) {
    r.node_ids.forEach((id) => nodeIds.add(id));
    r.edge_ids.forEach((id) => edgeIds.add(id));
  }
  return { nodeIds, edgeIds };
}

/** 点画布节点 → 命中的风险(节点可能代表被折叠的后代)。 */
export function riskForNode(risks: ArchRisk[], representedIds: string[]): ArchRisk[] {
  const set = new Set(representedIds);
  return risks.filter((r) => r.node_ids.some((id) => set.has(id)));
}

export function impactNodeIds(impact: ArchitectureBody['impact']): Set<string> {
  return new Set([...impact.direct, ...impact.indirect].map((n) => n.node_id));
}

/** 只有真正有基线可比较时才返回新增节点;`no_baseline`/`unavailable`
 *  一律空集,不得把现存节点渲染成"本轮新增"。 */
export function addedNodeIds(diff: ArchitectureBody['diff']): Set<string> {
  return diff.state === 'compared' ? new Set(diff.added_nodes) : new Set();
}
```

- [ ] **Step 3: 实现 `ArchitecturePage.tsx`**

```tsx
import { useMemo, useState } from 'preact/hooks';
import type { ArchitectureBody } from '../types.ts';
import { projectGraph } from '../graph/projection.ts';
import { addedNodeIds, impactNodeIds, riskForNode, riskIndex } from '../graph/linking.ts';
import { GraphCanvas } from './GraphCanvas.tsx';
import { FindingRowView } from './FindingRow.tsx';

export function ArchitecturePage({ body }: { body: ArchitectureBody }) {
  const [layer, setLayer] = useState<'crate' | 'module'>('crate');
  const [riskOnly, setRiskOnly] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [fitSignal, setFitSignal] = useState(0);

  const idx = useMemo(() => riskIndex(body.risks), [body.risks]);
  const impact = useMemo(() => impactNodeIds(body.impact), [body.impact]);
  const added = useMemo(() => addedNodeIds(body.diff), [body.diff]);
  const graph = useMemo(
    () =>
      projectGraph({
        nodes: body.nodes,
        edges: body.edges,
        layer,
        expanded,
        riskOnly,
        riskNodeIds: idx.nodeIds,
        riskEdgeIds: idx.edgeIds,
      }),
    [body.nodes, body.edges, layer, expanded, riskOnly, idx],
  );

  const selected = graph.nodes.find((n) => n.id === selectedId) ?? null;
  const selectedRisks = selected ? riskForNode(body.risks, selected.representedIds) : [];
  const activeRiskIds = new Set(selectedRisks.map((r) => r.finding.id));

  if (body.status === 'not_applicable') {
    return (
      <div>
        <h2>架构地图</h2>
        <div class="banner note">{body.status_note}</div>
      </div>
    );
  }

  const toggleExpand = (id: string) => {
    const node = graph.nodes.find((n) => n.id === id);
    if (!node || node.childCount === 0) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const focusRisk = (nodeIds: string[]) => {
    // 风险涉及的节点在当前投影里可能被折叠进某个可见节点,找第一个可见代表。
    const hit = graph.nodes.find((n) => n.representedIds.some((id) => nodeIds.includes(id)));
    if (hit) {
      setSelectedId(hit.id);
      setFocusId(hit.id);
    }
  };

  return (
    <div class="arch-layout">
      <div class="arch-toolbar">
        <h2>架构地图</h2>
        <div class="seg">
          <button class={layer === 'crate' ? 'active' : ''} onClick={() => setLayer('crate')}>
            crate
          </button>
          <button class={layer === 'module' ? 'active' : ''} onClick={() => setLayer('module')}>
            module
          </button>
        </div>
        <label class="switch">
          <input
            type="checkbox"
            checked={riskOnly}
            onChange={(e) => setRiskOnly((e.currentTarget as HTMLInputElement).checked)}
          />
          仅看风险
        </label>
        <button class="btn small" onClick={() => setFitSignal((n) => n + 1)}>
          适配窗口
        </button>
        <span class="legend">
          <span><i class="dot risk" />风险</span>
          <span><i class="dot impact" />本轮影响范围</span>
          <span><i class="dot added" />本轮新增</span>
        </span>
      </div>

      {body.status_note && <div class="banner note">{body.status_note}</div>}
      {body.truncated_note && <div class="banner note">{body.truncated_note}</div>}
      {body.errors.map((e) => (
        <div class="banner error" key={e}>{e}</div>
      ))}
      {body.diff.state === 'no_baseline' && (
        <div class="dim small">首次扫描(或旧报告),暂无架构变化可比对。</div>
      )}
      {body.impact.truncated && <div class="dim small">影响范围因访问上限被截断。</div>}

      <div class="arch-main">
        <GraphCanvas
          graph={graph}
          selectedId={selectedId}
          riskNodeIds={idx.nodeIds}
          riskEdgeIds={idx.edgeIds}
          impactNodeIds={impact}
          addedNodeIds={added}
          focusId={focusId}
          fitSignal={fitSignal}
          onSelect={setSelectedId}
          onToggleExpand={toggleExpand}
        />
        <div class="arch-side">
          {selected && (
            <div class="card detail">
              <b>{selected.label}</b>
              <dl>
                <dt>类型</dt><dd>{selected.kind}</dd>
                <dt>代码行</dt><dd>{selected.loc}</dd>
                {selected.childCount > 0 && (
                  <>
                    <dt>子模块</dt>
                    <dd>
                      {selected.childCount} 个(
                      <a href="#" onClick={(e) => { e.preventDefault(); toggleExpand(selected.id); }}>
                        {selected.collapsed ? '展开' : '折叠'}
                      </a>
                      )
                    </dd>
                  </>
                )}
              </dl>
            </div>
          )}
          <h3>风险</h3>
          {body.risks.length === 0 ? (
            <div class="dim">没有发现架构风险。</div>
          ) : (
            body.risks.map((r) => (
              <div
                key={r.finding.id}
                class={`risk-item ${activeRiskIds.has(r.finding.id) ? 'active' : ''}`}
                onClick={() => focusRisk(r.node_ids)}
              >
                <FindingRowView row={r.finding} analyze />
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
```

> 风险行点击既要"在图中定位"又要"跳转文件":`FindingRowView` 的根 `div` 自带 `open_location`。为让「点风险项 = 图中定位」成为主动作,把 `risk-item` 外层 `onClick` 保持,并把 `FindingRowView` 增加可选 prop `onClick`:在 `FindingRow.tsx` 里给 `FindingRowView` 加参数 `onActivate?: () => void`,根 `div` 的 `onClick` 改为 `onActivate ?? (() => send({kind:'open_location', ...}))`,架构页传 `onActivate={() => focusRisk(r.node_ids)}`(同时去掉外层 `risk-item` 的 `onClick`,避免双触发)。「交给 Agent 分析」按钮已 `stopPropagation`,不受影响。

`App.tsx` 追加分派:

```tsx
import { ArchitecturePage } from './ArchitecturePage.tsx';
// …
      {body.kind === 'architecture' && <ArchitecturePage body={body} />}
```

- [ ] **Step 4: 运行测试 / 类型检查 / 构建**

```bash
npm run typecheck 2>&1 | tail -8
npm test 2>&1 | tail -20
npm run build 2>&1 | tail -4
ls -l ../../assets/codehealth-content/
```

Expected: 全绿;产物重新生成。

- [ ] **Step 5: 提交**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
git add crates/dozer-app/web/codehealth-content/src crates/dozer-app/assets/codehealth-content
git diff --cached --stat | tail -15
git commit -m "feat(codehealth): architecture page with graph, risk linking and impact scope

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 12: 文档同步 + 全量验证 + 阶段二人工验收

**Files:**
- Modify: `docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md`(「布局与渲染」顶部)
- Modify: `docs/superpowers/specs/2026-10-01-code-health-webview-design.md`(状态与实现偏差)
- Modify: `CLAUDE.md`(仅当有新增的长期裁决——见 Step 2)

- [ ] **Step 1: 在旧架构 spec 顶部加取代说明**

找到 `docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md` 中标题为 `## 布局与渲染` 的一节,在该标题**正下方**插入:

```markdown
> **已被取代(2026-10-01)**:本节的「纯函数布局 + iced Canvas」方案作废。架构页改为
> `dozer://codehealth-content` webview 内用 Cytoscape.js 渲染、dagre 在前端算坐标;展开/折叠
> 由前端对可见子图做纯函数投影(不使用 Cytoscape 复合节点)。见
> `2026-10-01-code-health-webview-design.md` 与 `docs/superpowers/plans/2026-10-01-code-health-webview.md`。
> 本文其余部分(产品原则、数据模型、关系提取、图分析与健康规则、统一发现与差异、
> Agent 诊断上下文、性能约束)继续有效。
```

- [ ] **Step 2: 订正新 spec 的实现偏差并判断是否更新 CLAUDE.md**

在 `2026-10-01-code-health-webview-design.md` 的「实现计划阶段需验证的风险」一节之后追加一节:

```markdown
## 实现期决议(2026-10-01,写计划时确定)

1. **不用 Cytoscape 复合节点、不装 cytoscape-dagre / cytoscape-expand-collapse**:展开/折叠由前端纯函数 `projectGraph` 投影出扁平可见图,`dagre` 单独算坐标,Cytoscape 用 `preset` 布局只负责绘制与交互。风险 1(复合节点稳定性)因此规避;风险 3(布局确定性)由 `layout.test.ts` 固化。
2. **不引入 elk**:`elkjs` 为 EPL-2.0 / GPL 双许可,与仓库其余 MIT 依赖不同;dagre 够用,需要时另行评估。
3. **「前端不做计算」的精确边界**:聚合、排序、差异、风险分析在 Rust;前端只做「由视图状态(层级/展开集合/仅看风险)派生的可见子图投影」与布局坐标,这两者是渲染的一部分,且为纯函数、有单测。
4. **结构复杂度「本轮新增/全部」筛选**移到前端本地(Rust `StructureFilter` 删除);分类 `category` 仍留在 Rust(原生导航需要)。
5. **扫描中**不卸载 webview(原 spec 误以为现状是 `math_curve` 动画,实际是文字提示)。
6. **新增事件 `ScanRequested` / `AnalyzeFinding`**;删除 `SelectFinding`(无需求)。
7. **webview 事件的跳转路径**只接受相对路径且不含 `..`(`is_safe_relative_path`)。
```

并把该 spec 顶部「状态」行改为:`**状态:已批准并已实现(2026-10-01,见实现计划与「实现期决议」)**`。

`CLAUDE.md`:检查是否新增了需长期遵守的裁决。本计划**建议**在「关键裁决」的 WebView 条目附近加一条(一句话):

```markdown
- **代码健康度面板内容侧是 webview**(`web/codehealth-content/` → `assets/codehealth-content/`,见 `docs/superpowers/specs/2026-10-01-code-health-webview-design.md`):Rust 侧 `extensions/codehealth/protocol.rs::current_view_payload` 是内容判定的唯一权威,前端只渲染;**旧 iced 内容渲染已删除,不回归**;改前端后必须 `npm run build` 并提交 `assets/codehealth-content/`(`assets.rs` 有产物齐全测试)。右侧分类导航仍是原生 iced。
```

向用户确认后再写入 `CLAUDE.md`(这是项目级长期指令,不替用户默认定下);用户未明确同意则**跳过**此步,并在最终汇报里说明。

- [ ] **Step 3: 全量自动验证**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/codehealth-webview
cargo build 2>&1 | tail -3
cargo test 2>&1 | grep -E "^test result|FAILED|failed" | head -20
cargo clippy --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c | head
cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh
(cd crates/dozer-app/web/codehealth-content && npm run typecheck && npm test 2>&1 | tail -8)
git status --short
```

Expected:全 workspace 构建与测试通过;clippy 无新增 warning(与 Task 0 基线对比,既有 warning 不算);`fmt-ok`;门禁 ok;前端 typecheck 与测试通过;`git status --short` 干净(产物已提交)。若 `cargo test` 里出现与本分支无关的既有失败(main 上的 `footbar margin` 等已知问题),在汇报里逐条列出名称,不要掩盖也不要擅自修。

- [ ] **Step 4: 阶段二人工验收(GUI)**

```bash
cargo run -p dozer-app
```

对一个有 Cargo workspace 的 Rust 项目(用本仓库自身即可)扫描后,逐项核对并把结果记录给用户:

1. 右侧导航出现第 5 个「架构」(GitBranch 图标);点击后内容区显示架构地图。
2. 默认 crate 层:各 crate 节点按从左到右分层,边带箭头;滚轮缩放、拖拽平移、点击选中(详情卡显示类型/代码行)。
3. 切换 module 层:显示根模块,折叠节点标 `(+N)` 与双线边框;双击或详情卡「展开」后子模块出现、边重新路由、布局只在新子图上重排,已有节点位置大致稳定。
4. 「仅看风险」:只剩风险相关子图;关闭后恢复。「适配窗口」回到全图视野。
5. 在 fixture 项目里制造 `A → B → A` 模块环,重新扫描:风险列表出现新增循环;点列表项→图中选中并居中该节点;点图中风险节点→列表对应项高亮。
6. 「本轮影响范围」(有未提交改动时)节点青色描边;首次扫描时**没有**任何节点被标成「本轮新增」,并显示「首次扫描…暂无架构变化可比对」。
7. 重新扫描同一项目两次:同一图的节点坐标不变(肉眼无跳动)。
8. 破坏项目的 `Cargo.toml` 后重新扫描:module 图仍可用,页面显示 Cargo 分析失败原因(partial 提示 + errors),不显示成「没有风险」。
9. 旧报告(清掉 `architecture` 或用旧快照)进入架构页:显示「请重新扫描」,不显示图。
10. 暗/亮两种主题下图的颜色与对比度可读。
11. 快速在 5 个分类间连点:无白屏、无 JS 报错(必要时在 webview 里右键检查或看日志 `RUST_LOG=info,dozer::panel::code_health=debug`)。

- [ ] **Step 5: 提交文档并汇总**

```bash
git add docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md docs/superpowers/specs/2026-10-01-code-health-webview-design.md
# 仅当用户同意并已修改 CLAUDE.md 时再 add CLAUDE.md
git diff --cached --stat
git commit -m "docs: supersede architecture-map rendering section, record webview implementation decisions

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git log --oneline main..HEAD
```

向用户汇报:各任务提交列表、`cargo test`/前端测试结果、bundle 体积、阶段一与阶段二人工验收逐项结果、以及**合并注意**——main 工作树里 `crates/dozer-app/src/webview_geometry.rs` 有未提交改动,本分支 Task 2 也改了该文件;合并前先让用户决定那份 WIP 的去留,冲突时用 diff3 三方合并,合并后必须重跑 `cargo build` 与全量测试(git 冲突消失≠合并完成)。**不要自行合并进 main**,走 `superpowers:finishing-a-development-branch`。

---

## Self-Review

**1. Spec 覆盖**

| spec 要求 | 任务 |
|---|---|
| 内容侧整体换 webview,Preact+esbuild 离线 | 5、6 |
| 四类页信息结构 1:1 + 渲染升级 | 6、7 |
| 架构页:Cytoscape + dagre、缩放平移选择展开折叠、仅看风险、适配/重置 | 9、10、11 |
| 图与风险列表互相定位、影响范围标注 | 11(`linking.ts`、`ArchitecturePage`) |
| 右侧导航留原生并新增「架构」 | 8(`CodeHealthCategory::Architecture`、icon) |
| 协议 + revision 防乱序 | 1、5 |
| 仿 Usage 的几何/`preview_desired`/队列/消费点 | 2、4 |
| 前端只渲染、Rust 算分析 | 1、8(DTO)+ Task 12 实现期决议 3 |
| 失败回落原生占位、不留旧渲染 | 4、7 |
| 旧报告/无架构/非 Cargo/截断降级 | 8(状态与截断)、11(空态与提示) |
| 日志来源 `code_health`、Toast 约束 | Global Constraints;Task 4 `log_warn!` 带 `panel = "code_health"` |
| 测试清单(快照、协议、几何、冒烟、坐标稳定性、人工验收) | 1、2、5、6、10、11、7/12 |
| 分发布切片(骨架+四类 → 架构页) | Phase 1 / Phase 2,Task 7 与 12 各为独立验收点 |
| 文档同步(旧 spec 取代说明、CLAUDE.md) | 12 |

**已知与 spec 的有意偏差**(已写入 Task 4 Step 10 与 Task 12 Step 2 并在 spec 中订正):扫描中不卸载 webview;`SelectFinding` 删除、`ScanRequested`/`AnalyzeFinding` 补入;不用复合节点与 cytoscape-dagre;不引入 elk(许可)。「Rust 侧裁出可见子图」落实为 `MAX_PAYLOAD_NODES` 超限裁成 crate 层 + 前端投影。

**2. 占位符扫描**:计划中的"待填"仅有一处——Task 10 Step 6 提交消息里的 bundle 字节数,已在同一步骤明确要求先记录再替换后提交;`StructureBody.metric_note` 的标点一致性与 `ArchitectureReport` 构造体字段的核对在对应步骤给了确定的判定标准。无 TBD/TODO/"类似 Task N"。

**3. 类型一致性**:`CodeHealthViewPayload`/`Body`/`CategoryKey`/`FindingRowDto`(Task 1)↔ `types.ts`(Task 5)字段逐一对应;`row_dto` 在 Task 1 定义为私有,Task 8 改为 `pub(super)`(已在 Task 8 Step 1 写明);`WebviewPushState` 方法名(`set_ready`/`pending_push`/`mark_sent`/`observe_availability`/`failed`/`set_failed`/`clear_failed`)在 Task 1 定义、Task 4 使用,一致;`codehealth_content_pane_bounds_for` 参数顺序(Task 2 定义、Task 4 调用)一致;`projectGraph`/`VisibleGraph`/`VNode`(Task 9)被 Task 10/11 引用,字段名一致(`representedIds`、`childCount`、`collapsed`、`edgeIds`)。

**4. Review Focus**:6 条均有对应测试(1→Task 1 `unsafe_paths_are_rejected`;2→`never_ready_times_out_to_failed`;3→`revision_increments_per_send` + `drops an older revision`;4→`not_applicable_report_is_not_zero_risk`、`no_baseline_diff_is_not_rendered_as_all_new`、`added nodes only exist when there is a real baseline`、冒烟 `not applicable`;5→`oversized_graph_is_cut_to_crate_layer_with_note`;6→`set_ready_true_forces_resend`)。
