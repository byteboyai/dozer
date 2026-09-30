# Agent-native 文件编辑器 Phase 3(上下文列表 + 修改历史弹窗)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Agent 终端下方加一条持久化的"Agent 上下文"列表(human 显式添加的文件/目录),并提供一个可 Diff / Locate / Revert / Ask Agent 的"修改历史"独立原生弹窗。

**Architecture:** dozerd 在既有 `dozer.db` 里新增 `agent_context_items` 表(通过 `FileEditHistoryStore` 的第二个 `impl` 块提供方法,避免给 `Stores` 加字段而改 14 处测试构造点),并新增 4 个 UDS 请求(添加/移除/列出上下文项、查询修改历史);GUI 侧新增两个 extension 模块(`agent_context` 每项目状态 + 终端下方条;`edit_history` 弹窗状态)和一个独立原生窗口宿主(克隆 `file_history_overlay.rs`)。Phase 2 的"添加到 Agent 上下文"在写终端之外补一次落库。

**Tech Stack:** Rust workspace(dozer-core / dozerd / dozer-client / dozer-app)、rusqlite、iced 0.14、winit 子窗口 + wry(CodeMirror diff 宿主)。

**Spec:** `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`(前置:`2026-09-28-agent-native-file-editor-design.md` Phase 1、`2026-09-28-agent-native-file-editor-phase2-send-design.md` Phase 2,均已落地)

## 相对 spec 的三处有意偏差(实现前必读,完工后要回写 spec)

1. **上下文条高度恒定,展开的列表浮在终端底部之上,不推挤终端。** spec 写的是"折叠/展开会改变终端高度并重算 PTY 网格"。实测代码里 `terminal_pane_pixel_size` 是只依赖窗口尺寸与 `ShellState` 的纯函数,要让它感知"展开与否"就得给 `ShellState`(测试里有 40+ 处字面量构造)加字段。改为:条恒占 `STRIP_HEIGHT`,展开列表用 `stack!` 浮在其上方,PTY 网格永远不因折叠/展开而变。几何只需无条件多扣一个常量。
2. **弹窗过滤用"过滤标签 + 每条的「只看此文件」按钮",不做文本过滤框。** 独立窗口里的 `text_input` 需要为该窗口单独挂 IME/原生右键菜单(`SearchOverlay` 那套),代价远大于收益;入口本身已能预过滤(条上按钮 = 全部,上下文项的「历史」= 该项)。
3. **Locate 定位到修改起始行,不选中整段范围。** 复用现成的 `App::preview_open_path_at(path, Some(line))`(设置 `pending_jump_line`),不新造 Select 命令通路。

另有一处 **Phase 1 遗留缺口**(本计划不修,只在收尾时告知用户):spec 说 `apply_precise_edit` 会拒绝"有脏 tab 的文件",但 `crates/dozerd` 里并没有这个检查(dozerd 看不到 GUI 的脏 tab)。Revert 因此可能改写一个 GUI 里有未保存修改的文件——此时 T10 磁盘冲突机制会让该 tab 进入显式冲突态,用户的修改不会静默丢失。

## Global Constraints

- 新浮层走独立原生窗口(`platform/overlay_window.rs` 机制),**不**走 iced 内浮层 + 显式隐藏 webview 的旧路(CLAUDE.md 关键裁决)。
- 字体:非代码/终端场景一律 `Font::default()`;diff 内容由 CodeMirror 宿主渲染,不在 iced 里用等宽字体画代码。
- 图标按钮走 `icons::icon_button_entry`/`tabs::tab_core`;本计划的条与弹窗按钮均为**带文字标签的文本按钮**,不属于图标按钮,不套用。
- 所有异步结果带 `project_id` 路由到对应 `Workspace`,不写进"当前聚焦项目"。
- 权限位只在表里留 `permission` 列(默认 `'default'`),不进 UI,`apply_precise_edit` 不校验。
- 不新增 MCP 工具;历史查询只给 GUI。
- 预览选区"发送给 Agent"不落库、不入列表。
- commit 前先 `git status --short` + `git diff --cached --stat` 看 staged 全貌;工作区里现有的 `crates/dozer-app/src/chrome/tab_widget.rs`、`crates/dozer-app/src/workspace/tests.rs` 未提交改动**不属于**本计划,任何 commit 只能带本任务指定的文件(用 `git commit <paths>`)。
- 每个 commit message 以此结尾:`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`

## Review Focus

spec 隐含但没有任务专门测试的输入/故障,已分别加进对应任务的测试:

- 路径里含 `%`、`_`、`\` 的目录做历史过滤(SQL `LIKE` 通配符不能被当成通配) → Task 2。
- `entity_ref` 带尾部 `/`(GUI 或手工传入)要与不带的视为同一项 → Task 2。
- 用户在项目 A 点"添加",应答到达时已切到项目 B:结果只能落进 A → Task 4(`AgentContext(project_id, _)` 路由)+ Task 9 人工验证。
- 历史里 `old_text`/`new_text` 为空(纯插入/纯删除)时 Diff 与 Ask Agent 文本不崩 → Task 6/8。
- 多行 `new_text` 的"修改后结束坐标"(Revert 依赖它)算错会让撤销永远冲突 → Task 2 `end_position_after` 测试。

---

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/dozer-core/src/protocol.rs` | 修改 | `ContextItemInfo`/`FileEditHistoryInfo` + 4 个 `Request` + 3 个 `Reply` |
| `crates/dozerd/src/agent_context.rs` | 新建 | 上下文表 schema、`FileEditHistoryStore` 的第二个 impl(上下文项 CRUD + `list_history`)、路径校验、`end_position_after` |
| `crates/dozerd/src/file_edit_history.rs` | 修改 | `conn` 改 `pub(crate)`;`new()` 里调 `init_schema` |
| `crates/dozerd/src/lib.rs` | 修改 | `pub mod agent_context;` |
| `crates/dozerd/src/server.rs` | 修改 | 4 个请求的处理分支 |
| `crates/dozer-client/src/lib.rs` | 修改 | 4 个 `Client` 方法 |
| `crates/dozerd/tests/agent_context_requests.rs` | 新建 | 真实 dozerd 端到端测试 |
| `crates/dozer-app/src/extensions/agent_context.rs` | 新建 | 每项目状态、`apply`(纯)/`update`(IO)、条与展开面板视图 |
| `crates/dozer-app/src/extensions/edit_history.rs` | 新建 | 弹窗状态、`apply`/`update`、卡片视图、`ask_agent_text` |
| `crates/dozer-app/src/platform/edit_history_overlay.rs` | 新建 | 独立窗口宿主(克隆 `file_history_overlay.rs`) |
| `crates/dozer-app/src/platform/window_events.rs`、`platform/mod.rs` | 修改 | 登记新宿主(10 处) |
| `crates/dozer-app/src/extensions.rs`、`app/message.rs`、`app/app.rs`、`app/update.rs` | 修改 | 模块注册、消息、状态字段、路由 |
| `crates/dozer-app/src/term/terminal.rs`、`app/layout.rs`、`app/app.rs`(测试) | 修改 | 条渲染 + 终端几何扣高 |
| `crates/dozer-app/src/extensions/files/{state,update,view}.rs` | 修改 | Phase 2 动作补落库 |
| `crates/dozer-app/src/extensions/file_history.rs` | 修改 | `format_commit_time` 改 `pub(crate)` |
| `docs/user_guide/panels.md`、spec | 修改 | 文档 + 回写偏差 |

---

### Task 1: 协议类型与请求(dozer-core)

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`(类型加在 `LocateMatch` 之前约 495 行;`Request` 变体加在 `ApplyPreciseEdit { .. },` 之后;`Reply` 变体加在 `MutationResult { .. },` 之后;测试加在 `apply_precise_edit_request_round_trips` 之后)

**Interfaces:**
- Produces(后续任务依赖的精确名字):
  - `ContextItemInfo { id: i64, project_id: i64, entity_kind: String, entity_ref: String, created_ms: u64 }`
  - `FileEditHistoryInfo { id, project_id, target_path, actor, session_id, start_line, start_col, end_line, end_col, new_end_line, new_end_col, old_text, new_text, summary, created_ms }`(`start_*`/`end_*` 是**修改前**坐标,`new_end_*` 是**修改后**结束坐标,起点不变)
  - `Request::{AddContextItem{project_id,entity_kind,entity_ref}, RemoveContextItem{project_id,id}, ListContextItems{project_id}, ListFileEditHistory{project_id,path_filter:Option<String>,limit:u32}}`
  - `Reply::{ContextItem{item}, ContextItems{items}, FileEditHistory{entries}}`

- [ ] **Step 1: 写失败的测试**

在 `protocol.rs` 的 `#[cfg(test)] mod tests` 里、`apply_precise_edit_request_round_trips` 之后追加:

```rust
    #[test]
    fn context_and_history_requests_round_trip() {
        let reqs = [
            Request::AddContextItem {
                project_id: 1,
                entity_kind: "file".into(),
                entity_ref: "src/lib.rs".into(),
            },
            Request::RemoveContextItem { project_id: 1, id: 7 },
            Request::ListContextItems { project_id: 1 },
            Request::ListFileEditHistory {
                project_id: 1,
                path_filter: Some("src".into()),
                limit: 200,
            },
            Request::ListFileEditHistory {
                project_id: 1,
                path_filter: None,
                limit: 50,
            },
        ];
        for req in reqs {
            let json = serde_json::to_string(&req).unwrap();
            let back: Request = serde_json::from_str(&json).unwrap();
            assert_eq!(req, back);
        }
    }

    #[test]
    fn context_and_history_replies_round_trip() {
        let item = ContextItemInfo {
            id: 3,
            project_id: 1,
            entity_kind: "dir".into(),
            entity_ref: "research".into(),
            created_ms: 1_700_000_000_000,
        };
        let entry = FileEditHistoryInfo {
            id: 9,
            project_id: 1,
            target_path: "a.txt".into(),
            actor: "claude".into(),
            session_id: "s1".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            new_end_line: 3,
            new_end_col: 4,
            old_text: "line two".into(),
            new_text: "x\nabc".into(),
            summary: "改".into(),
            created_ms: 1_700_000_000_001,
        };
        let replies = [
            Reply::ContextItem { item: item.clone() },
            Reply::ContextItems { items: vec![item] },
            Reply::FileEditHistory { entries: vec![entry] },
        ];
        for reply in replies {
            let json = serde_json::to_string(&reply).unwrap();
            let back: Reply = serde_json::from_str(&json).unwrap();
            assert_eq!(reply, back);
        }
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-core context_and_history 2>&1 | tail -15`
Expected: 编译失败,`cannot find type ContextItemInfo` / `no variant AddContextItem`。

- [ ] **Step 3: 加类型与变体**

在 `pub struct LocateMatch` 之前插入:

```rust
/// Agent 上下文列表的一项(v0.1 意向文档 §10 Context Scope)。`entity_kind`
/// 本期取值 `"file"`/`"dir"`,日后要纳入 todo/会话/ssh/数据库时直接新增取值,
/// 不需要迁移表结构,所以传输与落库都用字符串而非枚举;读到不认识的取值时
/// 调用方应原样保留、显示为"未知类型"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItemInfo {
    pub id: i64,
    pub project_id: i64,
    pub entity_kind: String,
    /// 本期是项目内相对路径,目录不带尾部 `/`。
    pub entity_ref: String,
    pub created_ms: u64,
}

/// `file_edit_history` 一行,给 GUI 的修改历史弹窗用。`start_*`/`end_*` 是
/// **修改前**的坐标(1-based,结束坐标是最后一个字符之后的位置);
/// `new_end_*` 是**修改后** `new_text` 的结束坐标(起点不变),由 dozerd
/// 在查询时按 `new_text` 推算,Revert 反向调用 `apply_precise_edit` 需要它。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEditHistoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub target_path: String,
    pub actor: String,
    pub session_id: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub new_end_line: u32,
    pub new_end_col: u32,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
    pub created_ms: u64,
}

```

在 `Request` 枚举里 `ApplyPreciseEdit { ... },` 之后插入:

```rust
    /// 把一个实体加入项目的 Agent 上下文列表。重复添加幂等(返回既有项)。
    /// 路径越出项目根目录时应答 `Reply::Error`。
    AddContextItem {
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    },
    /// 移除一项;目标不存在也视为成功。
    RemoveContextItem { project_id: i64, id: i64 },
    /// 按添加时间升序列出项目的上下文项。
    ListContextItems { project_id: i64 },
    /// 按时间倒序列出项目的 agent 修改历史。`path_filter` 命中规则:
    /// `target_path` 等于该值,或位于该目录之下;`None` 不过滤。
    ListFileEditHistory {
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    },
```

在 `Reply` 枚举里 `MutationResult { outcome: MutationOutcome },` 之后插入:

```rust
    /// `AddContextItem` 应答。
    ContextItem {
        item: ContextItemInfo,
    },
    /// `ListContextItems` 应答。
    ContextItems {
        items: Vec<ContextItemInfo>,
    },
    /// `ListFileEditHistory` 应答。
    FileEditHistory {
        entries: Vec<FileEditHistoryInfo>,
    },
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p dozer-core 2>&1 | tail -8`
Expected: 全部 PASS(含新增两条)。若 workspace 里别处对 `Request`/`Reply` 做了穷尽 `match`,此时 `cargo build --workspace 2>&1 | grep -E "^error" -A6` 会报 non-exhaustive,按报错位置补 `_ => ` 分支或对应处理。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add context item and edit history requests

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozer-core/src/protocol.rs
```

---

### Task 2: dozerd 存储层(上下文项 CRUD + 历史查询)

**Files:**
- Create: `crates/dozerd/src/agent_context.rs`
- Modify: `crates/dozerd/src/file_edit_history.rs`(`conn` 字段可见性 + `new()` 调 `init_schema`)
- Modify: `crates/dozerd/src/lib.rs`(加 `pub mod agent_context;`,放在 `pub mod file_edit_history;` 旁)

**Interfaces:**
- Consumes: Task 1 的 `ContextItemInfo`/`FileEditHistoryInfo`。
- Produces:
  - `impl FileEditHistoryStore { pub fn add_context_item(&self, project_id: i64, entity_kind: &str, entity_ref: &str) -> anyhow::Result<ContextItemInfo>; pub fn remove_context_item(&self, project_id: i64, id: i64) -> anyhow::Result<()>; pub fn list_context_items(&self, project_id: i64) -> anyhow::Result<Vec<ContextItemInfo>>; pub fn list_history(&self, project_id: i64, path_filter: Option<&str>, limit: u32) -> anyhow::Result<Vec<FileEditHistoryInfo>> }`
  - `pub fn validate_context_ref(root: &std::path::Path, rel: &str) -> Result<(), String>`
  - `pub fn end_position_after(start_line: u32, start_col: u32, text: &str) -> (u32, u32)`

- [ ] **Step 1: 写失败的测试(新文件只含测试骨架)**

创建 `crates/dozerd/src/agent_context.rs`:

```rust
//! Agent 上下文列表(v0.1 意向文档 §10 Context Scope)的持久化,以及 GUI 修改
//! 历史弹窗的查询。表与 `file_edit_history` 同库同连接,方法挂在
//! `FileEditHistoryStore` 的第二个 `impl` 块上——不另起一个 store,是为了不给
//! `server::Stores` 加字段(那会连带改 14 处测试构造点)。
//! 设计见 `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`。

use anyhow::Result;
use dozer_core::protocol::{ContextItemInfo, FileEditHistoryInfo};
use rusqlite::{Connection, params};
use std::path::{Component, Path};

use crate::file_edit_history::FileEditHistoryStore;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_edit_history::NewFileEditHistoryEntry;

    fn temp_db() -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/dz-agent-context-test-{}.db",
            uuid::Uuid::new_v4()
        ))
    }

    fn edit(project_id: i64, path: &str, new_text: &str) -> NewFileEditHistoryEntry {
        NewFileEditHistoryEntry {
            project_id,
            target_path: path.to_string(),
            actor: "claude".into(),
            session_id: "s1".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            old_text: "line two".into(),
            new_text: new_text.into(),
            summary: "改".into(),
        }
    }

    #[test]
    fn add_is_idempotent_and_trims_trailing_slash() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let a = store.add_context_item(1, "dir", "research").unwrap();
        let b = store.add_context_item(1, "dir", "research/").unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.entity_ref, "research");
        assert_eq!(a.created_ms, b.created_ms, "重复添加不能改 created_ms");
        assert_eq!(store.list_context_items(1).unwrap().len(), 1);
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn list_is_ascending_and_scoped_by_project() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let a = store.add_context_item(1, "file", "a.rs").unwrap();
        let b = store.add_context_item(1, "file", "b.rs").unwrap();
        store.add_context_item(2, "file", "a.rs").unwrap();
        let items = store.list_context_items(1).unwrap();
        assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![a.id, b.id]);
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn remove_is_idempotent_and_project_scoped() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let a = store.add_context_item(1, "file", "a.rs").unwrap();
        // 别的项目不能删掉这一项。
        store.remove_context_item(2, a.id).unwrap();
        assert_eq!(store.list_context_items(1).unwrap().len(), 1);
        store.remove_context_item(1, a.id).unwrap();
        store.remove_context_item(1, a.id).unwrap(); // 再删一次不报错
        assert!(store.list_context_items(1).unwrap().is_empty());
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn unknown_entity_kind_round_trips_verbatim() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.add_context_item(1, "todo", "42").unwrap();
        assert_eq!(store.list_context_items(1).unwrap()[0].entity_kind, "todo");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_is_newest_first_with_limit() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let id1 = store.record(edit(1, "a.txt", "x")).unwrap();
        let id2 = store.record(edit(1, "a.txt", "y")).unwrap();
        let id3 = store.record(edit(1, "b.txt", "z")).unwrap();
        let all = store.list_history(1, None, 100).unwrap();
        assert_eq!(all.iter().map(|e| e.id).collect::<Vec<_>>(), vec![id3, id2, id1]);
        assert_eq!(store.list_history(1, None, 2).unwrap().len(), 2);
        assert!(store.list_history(2, None, 100).unwrap().is_empty());
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_filter_matches_file_or_directory_subtree_only() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(edit(1, "src/a/x.rs", "1")).unwrap();
        store.record(edit(1, "src/ab/y.rs", "2")).unwrap();
        store.record(edit(1, "src/a", "3")).unwrap(); // 恰好等于过滤值的文件
        let hits = store.list_history(1, Some("src/a"), 100).unwrap();
        let mut paths: Vec<_> = hits.iter().map(|e| e.target_path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec!["src/a", "src/a/x.rs"], "src/ab 不能被 src/a 误命中");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_filter_treats_like_wildcards_literally() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(edit(1, "100%_done/a.txt", "1")).unwrap();
        store.record(edit(1, "100xxdone/a.txt", "2")).unwrap();
        let hits = store.list_history(1, Some("100%_done"), 100).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target_path, "100%_done/a.txt");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_reports_post_edit_end_position() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(edit(1, "a.txt", "x\nabc")).unwrap();
        let e = &store.list_history(1, None, 10).unwrap()[0];
        assert_eq!((e.new_end_line, e.new_end_col), (3, 4));
        assert_eq!((e.start_line, e.start_col), (2, 1));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn end_position_after_single_line_multi_line_and_empty() {
        assert_eq!(end_position_after(2, 1, "replaced"), (2, 9));
        assert_eq!(end_position_after(2, 5, ""), (2, 5));
        assert_eq!(end_position_after(2, 1, "a\nbc"), (3, 3));
        assert_eq!(end_position_after(2, 1, "a\n"), (3, 1));
        assert_eq!(end_position_after(1, 1, "你好\n世"), (2, 2));
    }

    #[test]
    fn validate_rejects_escape_and_accepts_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(validate_context_ref(root, "src/lib.rs").is_ok());
        assert!(validate_context_ref(root, "尚未创建的/文件 v2.md").is_ok());
        assert!(validate_context_ref(root, "").is_err());
        assert!(validate_context_ref(root, "../x").is_err());
        assert!(validate_context_ref(root, "a/../../x").is_err());
        assert!(validate_context_ref(root, "/etc/passwd").is_err());
    }

    #[test]
    fn validate_rejects_symlink_escape_for_existing_paths() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(validate_context_ref(root.path(), "link").is_err());
    }
}
```

- [ ] **Step 2: 运行确认失败**

在 `lib.rs` 里 `pub mod file_edit_history;` 下一行加 `pub mod agent_context;`,然后:

Run: `cargo test -p dozerd agent_context 2>&1 | tail -20`
Expected: 编译失败,找不到 `add_context_item` / `validate_context_ref` / `end_position_after` 等。

- [ ] **Step 3: 实现**

`file_edit_history.rs` 两处改动:
1. `pub struct FileEditHistoryStore { conn: Mutex<Connection> }` 里字段改为 `pub(crate) conn: Mutex<Connection>,`。
2. `new()` 里 `.context("建表")?;` 之后、`Ok(Self {` 之前加:

```rust
        crate::agent_context::init_schema(&conn).context("建上下文表")?;
```

在 `agent_context.rs` 的 `#[cfg(test)]` 之前(`use` 之后)写入:

```rust
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    // `permission`:v0.1 §11 权限位的占位列,本期不读不写不进 UI。
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_context_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL,
            entity_kind TEXT NOT NULL,
            entity_ref TEXT NOT NULL,
            permission TEXT NOT NULL DEFAULT 'default',
            created_ms INTEGER NOT NULL,
            UNIQUE(project_id, entity_kind, entity_ref)
         );",
    )
}

/// 校验要加入上下文的相对路径落在项目根目录内。路径在磁盘上不存在是允许的
/// (agent 稍后可能创建它,列表项只是声明不是快照),所以先做纯词法检查
/// (拒绝空串/绝对路径/`..`),再对**已存在**的路径做 canonicalize 防符号链接逃逸。
pub fn validate_context_ref(root: &Path, rel: &str) -> Result<(), String> {
    if rel.is_empty() {
        return Err("路径为空".into());
    }
    let p = Path::new(rel);
    if p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err("路径越界".into());
    }
    let joined = root.join(p);
    if let (Ok(real), Ok(real_root)) = (joined.canonicalize(), root.canonicalize())
        && !real.starts_with(&real_root)
    {
        return Err("路径越界".into());
    }
    Ok(())
}

/// 把 `text` 替换进 `(start_line, start_col)` 之后,`text` 的结束坐标(1-based,
/// 与 `file_mutation::offset_to_line_col` 同口径:结束坐标是最后一个字符之后的
/// 位置;列按字符计,不按字节)。起点在替换前后不变。
pub fn end_position_after(start_line: u32, start_col: u32, text: &str) -> (u32, u32) {
    match text.rfind('\n') {
        None => (start_line, start_col + text.chars().count() as u32),
        Some(idx) => {
            let newlines = text.matches('\n').count() as u32;
            (
                start_line + newlines,
                text[idx + 1..].chars().count() as u32 + 1,
            )
        }
    }
}

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<ContextItemInfo> {
    Ok(ContextItemInfo {
        id: r.get(0)?,
        project_id: r.get(1)?,
        entity_kind: r.get(2)?,
        entity_ref: r.get(3)?,
        created_ms: r.get::<_, i64>(4)? as u64,
    })
}

/// SQL `LIKE ... ESCAPE '\'` 里把字面的 `\`/`%`/`_` 转义,免得目录名里的
/// 通配符字符被当成通配。
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl FileEditHistoryStore {
    /// 加入上下文列表。`(project_id, entity_kind, entity_ref)` 唯一,重复添加
    /// 返回既有行且不改 `created_ms`。`entity_ref` 尾部的 `/` 会被裁掉。
    pub fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: &str,
        entity_ref: &str,
    ) -> Result<ContextItemInfo> {
        let entity_ref = entity_ref.trim_end_matches('/');
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT OR IGNORE INTO agent_context_items
                (project_id, entity_kind, entity_ref, created_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![project_id, entity_kind, entity_ref, now_ms() as i64],
        )?;
        let item = conn.query_row(
            "SELECT id, project_id, entity_kind, entity_ref, created_ms
             FROM agent_context_items
             WHERE project_id = ?1 AND entity_kind = ?2 AND entity_ref = ?3",
            params![project_id, entity_kind, entity_ref],
            row_to_item,
        )?;
        Ok(item)
    }

    /// 按项目移除一项;目标不存在或属于别的项目都视为成功(不报错)。
    pub fn remove_context_item(&self, project_id: i64, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "DELETE FROM agent_context_items WHERE project_id = ?1 AND id = ?2",
            params![project_id, id],
        )?;
        Ok(())
    }

    pub fn list_context_items(&self, project_id: i64) -> Result<Vec<ContextItemInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, entity_kind, entity_ref, created_ms
             FROM agent_context_items
             WHERE project_id = ?1
             ORDER BY created_ms ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![project_id], row_to_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 项目内 agent 修改历史,时间倒序。`path_filter` 命中 `target_path == 值`
    /// 或位于该目录之下(`值/…`)。
    pub fn list_history(
        &self,
        project_id: i64,
        path_filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<FileEditHistoryInfo>> {
        let filter = path_filter.map(|f| f.trim_end_matches('/'));
        let like = filter.map(|f| format!("{}/%", escape_like(f)));
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, target_path, actor, session_id,
                    start_line, start_col, end_line, end_col,
                    old_text, new_text, summary, created_ms
             FROM file_edit_history
             WHERE project_id = ?1
               AND (?2 IS NULL OR target_path = ?2 OR target_path LIKE ?3 ESCAPE '\\')
             ORDER BY created_ms DESC, id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![project_id, filter, like, limit as i64], |r| {
            let start_line: u32 = r.get(5)?;
            let start_col: u32 = r.get(6)?;
            let new_text: String = r.get(10)?;
            let (new_end_line, new_end_col) = end_position_after(start_line, start_col, &new_text);
            Ok(FileEditHistoryInfo {
                id: r.get(0)?,
                project_id: r.get(1)?,
                target_path: r.get(2)?,
                actor: r.get(3)?,
                session_id: r.get(4)?,
                start_line,
                start_col,
                end_line: r.get(7)?,
                end_col: r.get(8)?,
                new_end_line,
                new_end_col,
                old_text: r.get(9)?,
                new_text,
                summary: r.get(11)?,
                created_ms: r.get::<_, i64>(12)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p dozerd agent_context 2>&1 | tail -20`
Expected: 10 条新测试 PASS。再跑 `cargo test -p dozerd file_edit_history 2>&1 | tail -6` 确认旧测试仍 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd/src/agent_context.rs crates/dozerd/src/file_edit_history.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): persist agent context items and query edit history

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozerd/src/agent_context.rs crates/dozerd/src/file_edit_history.rs crates/dozerd/src/lib.rs
```

---

### Task 3: 服务端处理分支 + Client 方法 + 端到端测试

**Files:**
- Modify: `crates/dozerd/src/server.rs`(在 `Request::ListCategories { project_id } => {` 这一行**之前**插入 4 个分支)
- Modify: `crates/dozer-client/src/lib.rs`(在 `apply_precise_edit` 方法之后插入 4 个方法)
- Create: `crates/dozerd/tests/agent_context_requests.rs`

**Interfaces:**
- Consumes: Task 1 请求/应答、Task 2 的 store 方法与 `validate_context_ref`。
- Produces(Client):
  - `add_context_item(&self, project_id: i64, entity_kind: &str, entity_ref: &str) -> Result<ContextItemInfo>`
  - `remove_context_item(&self, project_id: i64, id: i64) -> Result<()>`
  - `list_context_items(&self, project_id: i64) -> Result<Vec<ContextItemInfo>>`
  - `list_file_edit_history(&self, project_id: i64, path_filter: Option<&str>, limit: u32) -> Result<Vec<FileEditHistoryInfo>>`

- [ ] **Step 1: 写失败的端到端测试**

创建 `crates/dozerd/tests/agent_context_requests.rs`(`start_daemon` 与 `file_mutation_requests.rs` 同构):

```rust
use dozer_client::Client;
use dozer_core::protocol::MutationOutcome;
use std::sync::Arc;

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (
    std::path::PathBuf,
    CleanupGuard,
    Arc<dozerd::projects::ProjectStore>,
) {
    let sock = std::path::PathBuf::from(format!("/tmp/dz-agent-ctx-{}.sock", uuid::Uuid::new_v4()));
    let db = std::path::PathBuf::from(format!("/tmp/dz-agent-ctx-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: projects.clone(),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

#[tokio::test]
async fn add_list_remove_and_idempotent() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("research")).unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    let a = client.add_context_item(project.id, "dir", "research").await.unwrap();
    let again = client.add_context_item(project.id, "dir", "research/").await.unwrap();
    assert_eq!(a.id, again.id);
    // 磁盘上不存在的路径也允许添加。
    client.add_context_item(project.id, "file", "later.md").await.unwrap();

    let items = client.list_context_items(project.id).await.unwrap();
    assert_eq!(items.len(), 2);

    client.remove_context_item(project.id, a.id).await.unwrap();
    client.remove_context_item(project.id, a.id).await.unwrap();
    assert_eq!(client.list_context_items(project.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn add_rejects_out_of_bounds_and_unknown_project() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    assert!(client.add_context_item(project.id, "file", "../secret").await.is_err());
    assert!(client.add_context_item(project.id, "file", "/etc/passwd").await.is_err());
    assert!(client.add_context_item(9999, "file", "a.txt").await.is_err());
    assert!(client.list_context_items(project.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn history_lists_newest_first_filters_and_reports_new_end() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("sub/b.txt"), "alpha\n").unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    let o1 = client
        .apply_precise_edit(project.id, "a.txt", 2, 1, 2, 4, "two", "2\n2b", "改 two", "claude", "s1")
        .await
        .unwrap();
    assert!(matches!(o1, MutationOutcome::Applied { .. }));
    let o2 = client
        .apply_precise_edit(project.id, "sub/b.txt", 1, 1, 1, 6, "alpha", "ALPHA", "大写", "codex", "s2")
        .await
        .unwrap();
    assert!(matches!(o2, MutationOutcome::Applied { .. }));

    let all = client.list_file_edit_history(project.id, None, 50).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].target_path, "sub/b.txt", "最新的在前");
    assert_eq!(all[1].actor, "claude");
    assert_eq!((all[1].new_end_line, all[1].new_end_col), (3, 3), "\"2\\n2b\" 结束于第 3 行第 3 列");

    let only_sub = client
        .list_file_edit_history(project.id, Some("sub"), 50)
        .await
        .unwrap();
    assert_eq!(only_sub.len(), 1);
    assert_eq!(only_sub[0].target_path, "sub/b.txt");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozerd --test agent_context_requests 2>&1 | tail -12`
Expected: 编译失败,`Client` 没有 `add_context_item` 等方法。

- [ ] **Step 3: 实现 Client 方法**

`crates/dozer-client/src/lib.rs` 里 `apply_precise_edit` 方法结束的 `}` 之后追加:

```rust
    pub async fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: &str,
        entity_ref: &str,
    ) -> Result<dozer_core::protocol::ContextItemInfo> {
        match self
            .roundtrip(&Request::AddContextItem {
                project_id,
                entity_kind: entity_kind.into(),
                entity_ref: entity_ref.into(),
            })
            .await?
        {
            Reply::ContextItem { item } => Ok(item),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn remove_context_item(&self, project_id: i64, id: i64) -> Result<()> {
        match self
            .roundtrip(&Request::RemoveContextItem { project_id, id })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_context_items(
        &self,
        project_id: i64,
    ) -> Result<Vec<dozer_core::protocol::ContextItemInfo>> {
        match self
            .roundtrip(&Request::ListContextItems { project_id })
            .await?
        {
            Reply::ContextItems { items } => Ok(items),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::FileEditHistoryInfo>> {
        match self
            .roundtrip(&Request::ListFileEditHistory {
                project_id,
                path_filter: path_filter.map(String::from),
                limit,
            })
            .await?
        {
            Reply::FileEditHistory { entries } => Ok(entries),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 4: 实现服务端分支**

`server.rs` 里紧挨在 `Request::ListCategories { project_id } => {` 这一行之前插入:

```rust
                        Request::AddContextItem { project_id, entity_kind, entity_ref } => {
                            let root = projects
                                .list()
                                .ok()
                                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                                .map(|p| std::path::PathBuf::from(p.path));
                            match root {
                                None => Reply::Error {
                                    message: "项目不存在".into(),
                                },
                                Some(root) => {
                                    match crate::agent_context::validate_context_ref(
                                        &root,
                                        &entity_ref,
                                    ) {
                                        Err(message) => Reply::Error { message },
                                        Ok(()) => match file_edit_history.add_context_item(
                                            project_id,
                                            &entity_kind,
                                            &entity_ref,
                                        ) {
                                            Ok(item) => Reply::ContextItem { item },
                                            Err(e) => Reply::Error {
                                                message: format!("添加上下文项失败: {e}"),
                                            },
                                        },
                                    }
                                }
                            }
                        }
                        Request::RemoveContextItem { project_id, id } => {
                            match file_edit_history.remove_context_item(project_id, id) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error {
                                    message: format!("移除上下文项失败: {e}"),
                                },
                            }
                        }
                        Request::ListContextItems { project_id } => {
                            match file_edit_history.list_context_items(project_id) {
                                Ok(items) => Reply::ContextItems { items },
                                Err(e) => Reply::Error {
                                    message: format!("列上下文项失败: {e}"),
                                },
                            }
                        }
                        Request::ListFileEditHistory { project_id, path_filter, limit } => {
                            match file_edit_history.list_history(
                                project_id,
                                path_filter.as_deref(),
                                limit,
                            ) {
                                Ok(entries) => Reply::FileEditHistory { entries },
                                Err(e) => Reply::Error {
                                    message: format!("查修改历史失败: {e}"),
                                },
                            }
                        }
```

- [ ] **Step 5: 运行确认通过**

Run: `cargo test -p dozerd --test agent_context_requests 2>&1 | tail -12`
Expected: 3 条 PASS。再跑 `cargo test -p dozerd -p dozer-client -p dozer-mcp 2>&1 | grep -E "test result|FAILED"` 确认既有测试未受影响。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/server.rs crates/dozer-client/src/lib.rs crates/dozerd/tests/agent_context_requests.rs
git commit -m "feat(dozerd): serve context item and edit history requests

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozerd/src/server.rs crates/dozer-client/src/lib.rs crates/dozerd/tests/agent_context_requests.rs
```

---

### Task 4: GUI 上下文条(状态、视图、终端几何、刷新)

**Files:**
- Create: `crates/dozer-app/src/extensions/agent_context.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(`pub mod agent_context;`,放 `pub mod browser;` 之前保持字母序)
- Modify: `crates/dozer-app/src/app/message.rs`(加 `AgentContext(i64, crate::extensions::agent_context::Message)`,放在 `FileHistory(file_history::Message),` 附近)
- Modify: `crates/dozer-app/src/workspace/state.rs`(`Workspace` 加 `pub(crate) agent_context: crate::extensions::agent_context::State,`,放 `browser: browser::State,` 之后;编译器会列出所有 `Workspace` 字面量构造点,逐个补 `agent_context: Default::default(),`)
- Modify: `crates/dozer-app/src/term/terminal.rs::terminal_pane`
- Modify: `crates/dozer-app/src/app/layout.rs::terminal_pane_pixel_size`
- Modify: `crates/dozer-app/src/app/app.rs`(`adopt_panel_layout` 末尾;两条几何测试)
- Modify: `crates/dozer-app/src/app/update.rs`(`App::update` 加 `Message::AgentContext` 分支;新增 `agent_context_refresh`)

**Interfaces:**
- Produces:
  - `agent_context::{STRIP_HEIGHT: f32, strip_reserved_height() -> f32}`
  - `agent_context::ContextEntry { pub info: ContextItemInfo, pub missing: bool }`
  - `agent_context::State`(`Default`),访问器 `items() -> &[ContextEntry]`、`expanded() -> bool`、`notice() -> Option<&str>`、`entry(id: i64) -> Option<&ContextEntry>`
  - `agent_context::Message { Loaded(Result<Vec<ContextEntry>,String>), ToggleExpanded, Remove(i64), Added(Result<ContextItemInfo,String>), Removed(Result<(),String>), Open(i64), OpenHistory(Option<i64>), DismissNotice }`
  - `agent_context::apply(&mut State, Message) -> Followup`、`Followup { None, Refresh, RemoveItem(i64) }`
  - `agent_context::request_refresh(project_id: i64, root: PathBuf, client: &Client, handle: &Handle, emit: impl Fn(Message)+Send+'static)`
  - `agent_context::request_add(project_id: i64, is_dir: bool, relative: String, client: &Client, handle: &Handle, emit: impl Fn(Message)+Send+'static)`
  - `agent_context::strip(&State) -> Element<Message>`、`agent_context::expanded_panel(&State) -> Option<Element<Message>>`
  - `App::agent_context_refresh(&mut self, project_id: i64)`(`pub(crate)`)

- [ ] **Step 1: 写失败的测试(纯状态机)**

创建 `crates/dozer-app/src/extensions/agent_context.rs`,先只写模块头和测试:

```rust
//! Agent 上下文列表(v0.1 意向文档 §10 Context Scope):终端下方的一条,列出
//! human 显式送进来的文件/目录。数据权威在 dozerd(`agent_context_items` 表),
//! 本模块只持有每项目一份的展示状态。设计见
//! `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`。

use std::path::PathBuf;

use byteui::interaction::icons;
use dozer_core::protocol::ContextItemInfo;
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, r: &str, missing: bool) -> ContextEntry {
        ContextEntry {
            info: ContextItemInfo {
                id,
                project_id: 1,
                entity_kind: "file".into(),
                entity_ref: r.into(),
                created_ms: id as u64,
            },
            missing,
        }
    }

    #[test]
    fn loaded_ok_replaces_items_and_err_sets_notice() {
        let mut s = State::default();
        assert_eq!(apply(&mut s, Message::Loaded(Ok(vec![entry(1, "a", false)]))), Followup::None);
        assert_eq!(s.items().len(), 1);
        apply(&mut s, Message::Loaded(Err("boom".into())));
        assert!(s.notice().unwrap().contains("boom"));
        assert_eq!(s.items().len(), 1, "加载失败不清空已有列表");
    }

    #[test]
    fn toggle_flips_expanded() {
        let mut s = State::default();
        assert!(!s.expanded());
        apply(&mut s, Message::ToggleExpanded);
        assert!(s.expanded());
        apply(&mut s, Message::ToggleExpanded);
        assert!(!s.expanded());
    }

    #[test]
    fn remove_requests_io_and_removed_refreshes() {
        let mut s = State::default();
        assert_eq!(apply(&mut s, Message::Remove(5)), Followup::RemoveItem(5));
        assert_eq!(apply(&mut s, Message::Removed(Ok(()))), Followup::Refresh);
        assert_eq!(apply(&mut s, Message::Removed(Err("x".into()))), Followup::Refresh);
        assert!(s.notice().unwrap().contains("x"));
    }

    #[test]
    fn added_failure_still_refreshes_and_tells_the_user_it_was_pasted() {
        let mut s = State::default();
        let info = ContextItemInfo {
            id: 1,
            project_id: 1,
            entity_kind: "file".into(),
            entity_ref: "a".into(),
            created_ms: 1,
        };
        assert_eq!(apply(&mut s, Message::Added(Ok(info))), Followup::Refresh);
        assert!(s.notice().is_none());
        assert_eq!(apply(&mut s, Message::Added(Err("db 挂了".into()))), Followup::Refresh);
        let n = s.notice().unwrap();
        assert!(n.contains("已发送到终端") && n.contains("db 挂了"), "{n}");
    }

    #[test]
    fn open_messages_are_not_handled_here_and_dismiss_clears_notice() {
        let mut s = State::default();
        assert_eq!(apply(&mut s, Message::Open(1)), Followup::None);
        assert_eq!(apply(&mut s, Message::OpenHistory(None)), Followup::None);
        apply(&mut s, Message::Loaded(Err("e".into())));
        apply(&mut s, Message::DismissNotice);
        assert!(s.notice().is_none());
    }

    #[test]
    fn entry_lookup_and_reserved_height() {
        let mut s = State::default();
        apply(&mut s, Message::Loaded(Ok(vec![entry(1, "a", false), entry(2, "b", true)])));
        assert!(s.entry(2).unwrap().missing);
        assert!(s.entry(9).is_none());
        assert!(strip_reserved_height() > STRIP_HEIGHT, "必须包含 column 间距");
    }
}
```

- [ ] **Step 2: 运行确认失败**

在 `extensions.rs` 加 `pub mod agent_context;`,然后:

Run: `cargo test -p dozer-app agent_context 2>&1 | tail -15`
Expected: 编译失败,`State`/`apply`/`Followup` 等未定义。

- [ ] **Step 3: 实现状态机与 IO**

在 `agent_context.rs` 的 `use` 之后、`#[cfg(test)]` 之前写入:

```rust
/// 条的固定高度。折叠/展开都不改变它:展开的列表用 `stack!` 浮在终端底部
/// 之上,不推挤终端,所以 PTY 网格不会因折叠/展开重算(见 plan「有意偏差 1」)。
pub const STRIP_HEIGHT: f32 = 28.0;
/// 展开列表的最大高度,超出滚动。
const EXPANDED_MAX_HEIGHT: f32 = 168.0;

/// 终端几何要为条预留的总高度 = 条高 + 终端 pane column 的子元素间距。
/// `terminal_pane_pixel_size` 与 `terminal_pane` 渲染共用这一个值。
pub fn strip_reserved_height() -> f32 {
    STRIP_HEIGHT + crate::theme::region::terminal_pane().gap
}

/// 一项上下文 + 它在磁盘上是否已不存在(刷新时在后台线程判定,视图层不读盘)。
#[derive(Debug, Clone, PartialEq)]
pub struct ContextEntry {
    pub info: ContextItemInfo,
    pub missing: bool,
}

#[derive(Debug, Default)]
pub struct State {
    items: Vec<ContextEntry>,
    expanded: bool,
    notice: Option<String>,
}

impl State {
    pub fn items(&self) -> &[ContextEntry] {
        &self.items
    }
    pub fn expanded(&self) -> bool {
        self.expanded
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    pub fn entry(&self, id: i64) -> Option<&ContextEntry> {
        self.items.iter().find(|e| e.info.id == id)
    }
}

/// 所有消息都由 `App` 以 `Message::AgentContext(project_id, msg)` 信封携带
/// 项目 id 路由到对应 `Workspace`——异步结果不会写进"当前聚焦项目"。
#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Result<Vec<ContextEntry>, String>),
    ToggleExpanded,
    Remove(i64),
    Added(Result<ContextItemInfo, String>),
    Removed(Result<(), String>),
    /// 点某一项:App 拦截(需要预览/文件树句柄),不进 `apply`。
    Open(i64),
    /// 打开修改历史弹窗:`None` = 全部;`Some(item_id)` = 预过滤到该项。App 拦截。
    OpenHistory(Option<i64>),
    DismissNotice,
}

/// `apply` 之后调用方还要做的 IO。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followup {
    None,
    Refresh,
    RemoveItem(i64),
}

/// 纯状态转移(可单测);IO 由 `update` 按 `Followup` 执行。
pub fn apply(state: &mut State, msg: Message) -> Followup {
    match msg {
        Message::Loaded(Ok(items)) => {
            state.items = items;
            Followup::None
        }
        Message::Loaded(Err(e)) => {
            state.notice = Some(format!("加载上下文列表失败: {e}"));
            Followup::None
        }
        Message::ToggleExpanded => {
            state.expanded = !state.expanded;
            Followup::None
        }
        Message::Remove(id) => Followup::RemoveItem(id),
        Message::Removed(res) => {
            if let Err(e) = res {
                state.notice = Some(format!("移除失败: {e}"));
            }
            Followup::Refresh
        }
        Message::Added(res) => {
            if let Err(e) = res {
                // 落库失败时终端粘贴已经发生(见 Task 5),这里如实告知。
                state.notice = Some(format!("已发送到终端,但未能记录到上下文列表: {e}"));
            }
            Followup::Refresh
        }
        Message::Open(_) | Message::OpenHistory(_) => Followup::None,
        Message::DismissNotice => {
            state.notice = None;
            Followup::None
        }
    }
}

/// 拉取列表并在后台线程判定每项是否已缺失,完成后 `emit(Loaded)`。
pub fn request_refresh(
    project_id: i64,
    root: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let res = match client.list_context_items(project_id).await {
            Err(e) => Err(e.to_string()),
            Ok(items) => tokio::task::spawn_blocking(move || {
                items
                    .into_iter()
                    .map(|info| {
                        let missing = !root.join(&info.entity_ref).exists();
                        ContextEntry { info, missing }
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| e.to_string()),
        };
        emit(Message::Loaded(res));
    });
}

/// 把文件/目录加入项目上下文,完成后 `emit(Added)`。`relative` 不带尾部 `/`。
pub fn request_add(
    project_id: i64,
    is_dir: bool,
    relative: String,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    let kind = if is_dir { "dir" } else { "file" };
    handle.spawn(async move {
        let res = client
            .add_context_item(project_id, kind, &relative)
            .await
            .map_err(|e| e.to_string());
        emit(Message::Added(res));
    });
}

/// 执行 `apply` 并按 `Followup` 做 IO。`emit` 需 `Clone`(Refresh 与
/// RemoveItem 各自 spawn 一次);`App` 里的 `emit` 只捕获 `proxy`(Clone)与
/// `project_id`(Copy),天然满足。
pub fn update(
    state: &mut State,
    project_id: i64,
    root: PathBuf,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    match apply(state, msg) {
        Followup::None => {}
        Followup::Refresh => request_refresh(project_id, root, client, handle, emit),
        Followup::RemoveItem(id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .remove_context_item(project_id, id)
                    .await
                    .map_err(|e| e.to_string());
                emit(Message::Removed(res));
            });
        }
    }
}
```

- [ ] **Step 4: 运行状态机测试确认通过**

Run: `cargo test -p dozer-app agent_context 2>&1 | tail -15`
Expected: 6 条 PASS。

- [ ] **Step 5: 实现视图**

继续在 `agent_context.rs`(`update` 之后、`#[cfg(test)]` 之前)追加:

```rust
fn flat_button(color: iced_widget::core::Color) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, _s| button::Style {
        background: None,
        text_color: color,
        ..button::Style::default()
    }
}

/// 终端下方恒定高度的一条:`Agent 上下文 · N 项`(点击展开/收起) + 提示 +
/// `修改历史`。字体走系统默认(非代码/终端场景)。
pub fn strip(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let arrow = if state.expanded {
        icons::IconKind::ChevronDown
    } else {
        icons::IconKind::ChevronUp
    };
    let toggle = button(
        row![
            icons::view(arrow, byteui::theme::icon_size::row(), colors.dim),
            text(format!("Agent 上下文 · {} 项", state.items.len()))
                .size(byteui::theme::font::label())
                .color(colors.cream),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .on_press(Message::ToggleExpanded)
    .style(flat_button(colors.cream));

    let mut bar = row![toggle].spacing(12).align_y(Alignment::Center);
    if let Some(n) = state.notice() {
        bar = bar.push(
            button(text(n.to_string()).size(byteui::theme::font::label()).color(colors.red))
                .padding(0)
                .on_press(Message::DismissNotice)
                .style(flat_button(colors.red)),
        );
    }
    bar = bar.push(iced_widget::space::horizontal()).push(
        button(text("修改历史").size(byteui::theme::font::label()).color(colors.gold))
            .padding(0)
            .on_press(Message::OpenHistory(None))
            .style(flat_button(colors.gold)),
    );
    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(STRIP_HEIGHT))
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 展开态的逐项列表(折叠、或列表为空时返回 `None`)。由 `terminal_pane` 用
/// `stack!` 浮在终端底部、条的正上方。缺失项灰显并标"缺失"。
pub fn expanded_panel(
    state: &State,
) -> Option<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if !state.expanded || state.items.is_empty() {
        return None;
    }
    let colors = byteui::theme::color::current();
    let mut list = column![].spacing(4);
    for e in &state.items {
        let kind_icon = if e.info.entity_kind == "dir" {
            icons::IconKind::Folder
        } else {
            icons::IconKind::FileText
        };
        let label_color = if e.missing { colors.dim } else { colors.cream };
        let label = if e.missing {
            format!("{}(缺失)", e.info.entity_ref)
        } else {
            e.info.entity_ref.clone()
        };
        let mut name = button(text(label).size(byteui::theme::font::label()).color(label_color))
            .padding(0)
            .width(Length::Fill)
            .style(flat_button(label_color));
        if !e.missing {
            name = name.on_press(Message::Open(e.info.id));
        }
        list = list.push(
            row![
                icons::view(kind_icon, byteui::theme::icon_size::row(), label_color),
                name,
                button(text("历史").size(byteui::theme::font::label()).color(colors.gold))
                    .padding(0)
                    .on_press(Message::OpenHistory(Some(e.info.id)))
                    .style(flat_button(colors.gold)),
                button(text("移除").size(byteui::theme::font::label()).color(colors.dim))
                    .padding(0)
                    .on_press(Message::Remove(e.info.id))
                    .style(flat_button(colors.dim)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    Some(
        container(scrollable(list))
            .width(Length::Fill)
            .max_height(EXPANDED_MAX_HEIGHT)
            .padding(8)
            .style(crate::dialog::card_style)
            .into(),
    )
}
```

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" -A8 | head -40`
Expected: 仅剩"未使用"类警告或与后续接线相关的错误;若 `icons::IconKind::FileText`/`Folder`/`ChevronUp`/`ChevronDown` 报不存在,以 `crates/byteui/src/interaction/icons.rs` 里实际变体名为准替换(本计划调研时这四个都存在)。`byteui::theme::color::current()` 的字段名 `cream/dim/gold/red` 已在现有代码中使用。

- [ ] **Step 6: 接线 —— 消息、Workspace 字段、路由、刷新**

1. `app/message.rs`:在 `FileHistory(file_history::Message),` 之后加

```rust
    /// 终端下方 Agent 上下文条的消息。`i64` 是 `project_id`:所有异步结果都按它
    /// 路由到对应 `Workspace`,不落进"当前聚焦项目"。
    AgentContext(i64, crate::extensions::agent_context::Message),
```

2. `workspace/state.rs`:`Workspace` 加字段 `pub(crate) agent_context: crate::extensions::agent_context::State,`;`cargo build -p dozer-app` 后按报错给每个字面量构造补 `agent_context: Default::default(),`。

3. `app/update.rs` 的 `App::update` 里(`Message::FileHistory(msg) => {…}` 之后)加:

```rust
            Message::AgentContext(project_id, msg) => self.agent_context_message(project_id, msg),
```

并在 `impl App` 里新增两个方法(放在 `preview_open_path_at` 附近):

```rust
    /// 上下文条的消息路由。`project_id` 来自信封,异步应答即使在用户切到别的
    /// 项目之后到达,也只写回发起它的那个 `Workspace`。
    pub(crate) fn agent_context_message(
        &mut self,
        project_id: i64,
        msg: crate::extensions::agent_context::Message,
    ) {
        use crate::extensions::agent_context as ctx;
        let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
            .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
        else {
            return;
        };
        match msg {
            ctx::Message::Open(id) => {
                let Some(entry) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.agent_context.entry(id).cloned())
                else {
                    return;
                };
                let path = root.join(&entry.info.entity_ref);
                if entry.info.entity_kind == "dir" {
                    // 目录:在文件树里选中它(不展开/收起,避免误 toggle)。
                    if let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) {
                        ws.files.set_tree_selected(path);
                    }
                } else {
                    self.preview_open_path(path);
                }
            }
            ctx::Message::OpenHistory(item) => {
                let filter = item.and_then(|id| {
                    loaded_workspace_mut(&mut self.projects, project_id)
                        .and_then(|ws| ws.agent_context.entry(id))
                        .map(|e| e.info.entity_ref.clone())
                });
                self.open_edit_history(project_id, filter);
            }
            other => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::AgentContext(project_id, m));
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                ctx::update(
                    &mut ws.agent_context,
                    project_id,
                    root,
                    other,
                    &client,
                    &handle,
                    emit,
                );
            }
        }
    }

    /// 拉取某项目的上下文列表(切入项目时调用;工作区未加载则空操作)。
    pub(crate) fn agent_context_refresh(&mut self, project_id: i64) {
        let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
            .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
        else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::agent_context::request_refresh(
            project_id,
            root,
            &client,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::AgentContext(project_id, m));
            },
        );
    }
```

`open_edit_history` 在 Task 6 才存在;为让本任务能独立编译,先在同一 `impl App` 里放一个临时存根,Task 6 Step 5 会把它替换为真实实现:

```rust
    pub(crate) fn open_edit_history(&mut self, _project_id: i64, _filter: Option<String>) {}
```

4. `app/app.rs::adopt_panel_layout` 末尾(函数最后一条语句之后)加 `self.agent_context_refresh(id);`。再 `grep -n "WorkspaceSlot::Loaded(" crates/dozer-app/src` 找到"启动时恢复出来的项目工作区变为 Loaded"的位置,在那里也调一次 `agent_context_refresh`(未加载则空操作),保证重启后列表不是空的;找不到明确的恢复点就只保留 `adopt_panel_layout` 这一处,并在 Task 9 的人工验证里检查重启后是否显示。

- [ ] **Step 7: 接线 —— 终端渲染与几何,并更新既有几何测试**

1. `term/terminal.rs::terminal_pane`:把 `content = content.push(active_tab_view(app, ws));` 之后到 `let body = container(...)` 之前改为

```rust
    content = content.push(active_tab_view(app, ws));

    // 上下文条:恒定高度,几何见 `agent_context::strip_reserved_height`。
    let project_id = ws.project.as_ref().map(|p| p.id).unwrap_or(0);
    content = content.push(
        crate::extensions::agent_context::strip(&ws.agent_context)
            .map(move |m| Message::AgentContext(project_id, m)),
    );

    let base = container(content.spacing(region.gap).padding(region.padding))
        .width(Length::Fill)
        .height(Length::Fill);
    // 展开的列表浮在终端底部、条的正上方,不推挤终端。
    let layered: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match crate::extensions::agent_context::expanded_panel(&ws.agent_context) {
            Some(panel) => {
                let floating = container(
                    panel.map(move |m| Message::AgentContext(project_id, m)),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Bottom)
                .padding(iced_widget::core::Padding {
                    bottom: region.padding.bottom
                        + crate::extensions::agent_context::STRIP_HEIGHT,
                    ..region.padding
                });
                iced_widget::stack![base, floating].into()
            }
            None => base.into(),
        };

    let body = container(layered)
```

并把原来紧随其后的 `container(content.spacing(region.gap).padding(region.padding))` 那一段删掉(已并入 `base`),保留 `.width(Length::Fill).height(Length::Fill).style(move |_theme| …)` 这条链接在新的 `body` 上。`terminal_pane` 现有代码里 `content` 的类型/生命周期若与 `Element<'a, …>` 标注冲突,以编译器提示为准调整标注,不改结构。

2. `app/layout.rs::terminal_pane_pixel_size`:两个分支里的 `pane_height` 各再减 `crate::extensions::agent_context::strip_reserved_height()`:

放大态分支:
```rust
        let pane_height = (maximized_box_height(window_height)
            - byteui::theme::geometry::chrome_height_px()
            - crate::extensions::agent_context::strip_reserved_height())
        .max(0.0);
```
常规分支:
```rust
    let pane_height = (window_height
        - byteui::theme::geometry::top_bar_height()
        - byteui::theme::geometry::chrome_height_px()
        - m.top
        - m.bottom
        - crate::extensions::agent_context::strip_reserved_height())
    .max(0.0);
```

3. 更新 `app/app.rs` 里两条既有测试(数字已因条而变,改成用预留高度表达,不再写死):

`terminal_pane_height_excludes_top_bar_and_chrome`:把断言里的
`(byteui::theme::geometry::top_bar_height() + m.top + m.bottom)` 改为
`(byteui::theme::geometry::top_bar_height() + m.top + m.bottom + crate::extensions::agent_context::strip_reserved_height())`,断言消息改为 `"终端 pane 高度必须再扣顶栏+right_zone 上下 margin+上下文条"`。

`terminal_pane_pixel_size_right_maximized_matches_overlay_box`:
- `assert!((h - 730.0).abs() < 0.1, …)` → `assert!((h - (730.0 - crate::extensions::agent_context::strip_reserved_height())).abs() < 0.1, "h={h}");`
- `assert!((normal.1 - 804.0).abs() < 0.1, …)` → `assert!((normal.1 - (804.0 - crate::extensions::agent_context::strip_reserved_height())).abs() < 0.1, "平时 h={}", normal.1);`
- 两条 `grid_size` 断言的行数会变:运行测试,按失败信息里的实际值把 `(95, 43)`、`(51, 47)` 的**行数**改成新值,并确认新值 = `floor((旧高度 - 预留) / 16.8)`(`LINE_HEIGHT_PX`=16.8,列数不变)。在注释里补一句"含上下文条预留"。

再新增一条专门锁定"折叠/展开不影响几何"的测试(紧跟上面两条之后):

```rust
    #[test]
    fn context_strip_reserve_is_constant_regardless_of_expansion() {
        // 展开态列表是浮层,不参与几何:预留高度只依赖常量与 region.gap,
        // 与 State.expanded 无关,PTY 网格不会因折叠/展开重算。
        let a = crate::extensions::agent_context::strip_reserved_height();
        let mut s = crate::extensions::agent_context::State::default();
        crate::extensions::agent_context::apply(
            &mut s,
            crate::extensions::agent_context::Message::ToggleExpanded,
        );
        assert!(s.expanded());
        assert_eq!(a, crate::extensions::agent_context::strip_reserved_height());
        assert_eq!(
            a,
            crate::extensions::agent_context::STRIP_HEIGHT
                + crate::theme::region::terminal_pane().gap
        );
    }
```

- [ ] **Step 8: 全量验证**

Run: `cargo build -p dozer-app 2>&1 | tail -5 && cargo test -p dozer-app 2>&1 | grep -E "test result|FAILED|panicked" | head`
Expected: 编译通过;dozer-app 测试全绿。注意 CLAUDE 的 memory 记录过"合并前已存在的 2 个 footbar margin 测试失败"——若出现,先 `git stash` 对照确认是否本就失败,不是本任务引入就不处理。

- [ ] **Step 9: Commit**

```bash
git status --short && git diff --cached --stat
git add crates/dozer-app/src/extensions/agent_context.rs crates/dozer-app/src/extensions.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/workspace/state.rs crates/dozer-app/src/term/terminal.rs crates/dozer-app/src/app/layout.rs crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(app): agent context strip under the terminal

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozer-app/src/extensions/agent_context.rs crates/dozer-app/src/extensions.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/workspace/state.rs crates/dozer-app/src/term/terminal.rs crates/dozer-app/src/app/layout.rs crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/update.rs
```
(若编译器要求补 `agent_context` 字段的 `Workspace` 构造点分布在其它文件,把这些文件也加进 `git add`/`git commit` 的路径列表。)

---

### Task 5: Phase 2 动作补落库

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/state.rs`(`RequestSendToAgentTerminal` 改为带数据的结构变体)
- Modify: `crates/dozer-app/src/extensions/files/update.rs`(`SendToAgentContext` 分支 + `unreachable!` 分支模式;新增纯函数 `send_payload` + 测试)
- Modify: `crates/dozer-app/src/app/update.rs`(拦截分支先落库再粘贴)

**Interfaces:**
- Consumes: Task 4 的 `agent_context::request_add(project_id, is_dir, relative, &client, &handle, emit)`。
- Produces: `files::Message::RequestSendToAgentTerminal { text: String, is_dir: bool, relative: String }`;`files::update::send_payload(root: &Path, target: &Path, is_dir: bool) -> (String /*relative*/, String /*text*/)`(私有到 update 模块即可,测试在同文件)。

- [ ] **Step 1: 写失败的测试**

`files/update.rs` 末尾(没有 `#[cfg(test)]` 就新建)追加:

```rust
#[cfg(test)]
mod send_payload_tests {
    use super::send_payload;
    use std::path::Path;

    #[test]
    fn relative_path_and_text_for_file_and_dir() {
        let root = Path::new("/proj");
        let (rel, text) = send_payload(root, Path::new("/proj/src/main.rs"), false);
        assert_eq!(rel, "src/main.rs");
        assert_eq!(text, "请将 src/main.rs 文件纳入你的工作上下文。");
        let (rel, text) = send_payload(root, Path::new("/proj/research"), true);
        assert_eq!(rel, "research", "落库用的相对路径不带尾部 /");
        assert_eq!(text, "请将 research/ 目录纳入你的工作上下文。");
    }

    #[test]
    fn keeps_spaces_and_unicode_verbatim() {
        let root = Path::new("/proj");
        let (rel, text) = send_payload(root, Path::new("/proj/我的 报告/draft v2.md"), false);
        assert_eq!(rel, "我的 报告/draft v2.md");
        assert_eq!(text, "请将 我的 报告/draft v2.md 文件纳入你的工作上下文。");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-app send_payload 2>&1 | tail -10`
Expected: 编译失败,`send_payload` 未定义。

- [ ] **Step 3: 实现**

`files/state.rs`:把

```rust
    RequestSendToAgentTerminal(String),
```
改为
```rust
    RequestSendToAgentTerminal {
        text: String,
        is_dir: bool,
        /// 项目内相对路径,目录不带尾部 `/`,落库用。
        relative: String,
    },
```
(保留其上方文档注释,并补一句"同时携带落库所需的 `is_dir`/`relative`"。)

`files/update.rs`:在文件顶部 `update` 函数之前加纯函数:

```rust
/// 右键"添加到 Agent 上下文"的负载:项目内相对路径(与"复制相对路径"同一套
/// `path_string` 逻辑,原样不转义)+ 要写进终端的模板文本。
fn send_payload(root: &std::path::Path, target: &std::path::Path, is_dir: bool) -> (String, String) {
    let relative = crate::project::path_string(crate::project::PathKind::Relative, target, root);
    let text = super::agent_context_reference_text(&relative, is_dir);
    (relative, text)
}
```

`SendToAgentContext` 分支改为:

```rust
        Message::SendToAgentContext(target, is_dir) => {
            let Some(tree) = ws_state.file_tree.as_ref() else {
                return;
            };
            let (relative, text) = send_payload(tree.root(), &target, is_dir);
            emit(Message::RequestSendToAgentTerminal {
                text,
                is_dir,
                relative,
            });
        }
        Message::RequestSendToAgentTerminal { .. } => {
            // 内核拦截处理(`App::update` 的对应分支),永远不该落回这里。
            unreachable!("RequestSendToAgentTerminal 由内核拦截处理");
        }
```
(把原来这两个分支整体替换;保留原有的解释性注释。)

`app/update.rs` 里把

```rust
            Message::Files(files::Message::RequestSendToAgentTerminal(text)) => {
                …
                self.term_paste(terminal::TermTarget::Shared, text);
            }
```
改为

```rust
            Message::Files(files::Message::RequestSendToAgentTerminal {
                text,
                is_dir,
                relative,
            }) => {
                // 先发起落库(异步),再写终端;落库失败不影响粘贴——上下文条会
                // 显示"已发送到终端,但未能记录到上下文列表"(见 `agent_context::apply`)。
                if let Some(project_id) = self.active_project_id {
                    let client = self.client.clone();
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    crate::extensions::agent_context::request_add(
                        project_id,
                        is_dir,
                        relative,
                        &client,
                        &handle,
                        move |m| {
                            let _ = proxy.send_event(Message::AgentContext(project_id, m));
                        },
                    );
                }
                self.term_paste(terminal::TermTarget::Shared, text);
            }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p dozer-app send_payload 2>&1 | tail -6 && cargo build -p dozer-app 2>&1 | tail -3`
Expected: 2 条 PASS,编译通过(`files/mod.rs` 里既有测试只匹配 `SendToAgentContext(..)`,不受影响)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/files/state.rs crates/dozer-app/src/extensions/files/update.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(files): persist 'add to agent context' into the context list

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozer-app/src/extensions/files/state.rs crates/dozer-app/src/extensions/files/update.rs crates/dozer-app/src/app/update.rs
```

---

### Task 6: 修改历史弹窗 —— 状态、视图、独立窗口宿主(不含 Diff webview)

**Files:**
- Create: `crates/dozer-app/src/extensions/edit_history.rs`
- Create: `crates/dozer-app/src/platform/edit_history_overlay.rs`(克隆 `file_history_overlay.rs`)
- Modify: `crates/dozer-app/src/extensions.rs`(`pub mod edit_history;`)
- Modify: `crates/dozer-app/src/extensions/file_history.rs`(`fn format_commit_time` → `pub(crate) fn`)
- Modify: `crates/dozer-app/src/platform/mod.rs`(`pub mod edit_history_overlay;`)
- Modify: `crates/dozer-app/src/platform/window_events.rs`(10 处登记,见 Step 6)
- Modify: `crates/dozer-app/src/app/message.rs`、`app/app.rs`、`app/update.rs`

**Interfaces:**
- Consumes: Task 3 的 `Client::list_file_edit_history` / `Client::apply_precise_edit`;Task 4 的 `App::open_edit_history` 存根。
- Produces:
  - `edit_history::State::new(project_id: i64, filter: Option<String>, agent_terminal_visible: bool) -> State`;访问器 `project_id()`、`filter()`、`entries() -> Option<&Result<Vec<FileEditHistoryInfo>,String>>`、`selected() -> Option<i64>`、`entry(id) -> Option<&FileEditHistoryInfo>`、`selected_entry()`、`agent_terminal_visible()`、`revert_status() -> Option<&(i64, RevertStatus)>`
  - `edit_history::RevertStatus { Pending, Done, Failed(String) }`
  - `edit_history::Message { Close, Loaded{project_id, filter, result}, Select(i64), SetFilter(Option<String>), Locate(i64), AskAgent(i64), Revert(i64), RevertDone{id, result: Result<MutationOutcome,String>} }`
  - `edit_history::apply(&mut Option<State>, Message) -> Followup`;`Followup { None, Reload, Revert(FileEditHistoryInfo) }`
  - `edit_history::update(state: &mut Option<State>, msg, client, handle, emit)`、`edit_history::request_load(project_id, filter, client, handle, emit)`
  - `edit_history::edit_history_card(&State) -> Element<Message>`
  - 布局常量 `edit_history::{LIST_WIDTH: f32 = 300.0, ACTIONS_HEIGHT: f32 = 36.0}`
  - `App::open_edit_history(&mut self, project_id: i64, filter: Option<String>)`(替换 Task 4 的存根)

- [ ] **Step 1: 写失败的状态机测试**

创建 `crates/dozer-app/src/extensions/edit_history.rs`,先写模块头与测试:

```rust
//! Agent 修改历史弹窗(v0.1 意向文档 §18 Change History):读 dozerd 的
//! `file_edit_history`,提供 Diff / Locate / Revert / Ask Agent。独立原生窗口宿主
//! 见 `platform/edit_history_overlay.rs`。设计见
//! `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`。

use byteui::interaction::icons;
use dozer_core::protocol::{FileEditHistoryInfo, MutationOutcome};
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: i64, path: &str) -> FileEditHistoryInfo {
        FileEditHistoryInfo {
            id,
            project_id: 1,
            target_path: path.into(),
            actor: "claude".into(),
            session_id: "s".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            new_end_line: 2,
            new_end_col: 9,
            old_text: "old".into(),
            new_text: "new".into(),
            summary: "改".into(),
            created_ms: id as u64,
        }
    }

    fn open(filter: Option<&str>) -> Option<State> {
        Some(State::new(1, filter.map(String::from), true))
    }

    #[test]
    fn loaded_selects_first_and_drops_stale_results() {
        let mut st = open(None);
        // 过期:项目不同 / 过滤不同 → 丢弃。
        apply(&mut st, Message::Loaded { project_id: 2, filter: None, result: Ok(vec![info(1, "a")]) });
        assert!(st.as_ref().unwrap().entries().is_none());
        apply(&mut st, Message::Loaded { project_id: 1, filter: Some("x".into()), result: Ok(vec![info(1, "a")]) });
        assert!(st.as_ref().unwrap().entries().is_none());
        // 有效。
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(5, "a"), info(4, "b")]) });
        let s = st.as_ref().unwrap();
        assert_eq!(s.selected(), Some(5));
        assert_eq!(s.selected_entry().unwrap().target_path, "a");
    }

    #[test]
    fn reload_keeps_selection_when_still_present_else_selects_first() {
        let mut st = open(None);
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(5, "a"), info(4, "b")]) });
        apply(&mut st, Message::Select(4));
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(6, "a"), info(5, "a"), info(4, "b")]) });
        assert_eq!(st.as_ref().unwrap().selected(), Some(4));
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(9, "z")]) });
        assert_eq!(st.as_ref().unwrap().selected(), Some(9));
    }

    #[test]
    fn set_filter_clears_entries_and_requests_reload() {
        let mut st = open(None);
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(1, "a")]) });
        assert_eq!(apply(&mut st, Message::SetFilter(Some("a".into()))), Followup::Reload);
        let s = st.as_ref().unwrap();
        assert_eq!(s.filter(), Some("a"));
        assert!(s.entries().is_none(), "切换过滤后先清空,等新结果");
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn select_resets_diff_bookkeeping_only_when_path_changes() {
        let mut st = open(None);
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(3, "a"), info(2, "a"), info(1, "b")]) });
        st.as_mut().unwrap().set_diff_webview_ready(true);
        st.as_mut().unwrap().set_diff_sent_for(3);
        apply(&mut st, Message::Select(2)); // 同一路径:webview 不重载
        let s = st.as_ref().unwrap();
        assert!(s.diff_webview_ready());
        assert_eq!(s.diff_sent_for(), None, "换了条目就要重新推内容");
        apply(&mut st, Message::Select(1)); // 换路径:URL 会变,webview 重载
        assert!(!st.as_ref().unwrap().diff_webview_ready());
    }

    #[test]
    fn revert_lifecycle() {
        let mut st = open(None);
        apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![info(7, "a")]) });
        let f = apply(&mut st, Message::Revert(7));
        assert!(matches!(f, Followup::Revert(ref e) if e.id == 7));
        assert!(matches!(st.as_ref().unwrap().revert_status(), Some((7, RevertStatus::Pending))));

        let applied = MutationOutcome::Applied { new_start_line: 2, new_start_col: 1, new_end_line: 2, new_end_col: 4, history_id: 8 };
        assert_eq!(apply(&mut st, Message::RevertDone { id: 7, result: Ok(applied) }), Followup::Reload);
        assert!(matches!(st.as_ref().unwrap().revert_status(), Some((7, RevertStatus::Done))));

        apply(&mut st, Message::Revert(7));
        apply(&mut st, Message::RevertDone { id: 7, result: Ok(MutationOutcome::Conflict { actual_text: "别的".into() }) });
        match st.as_ref().unwrap().revert_status() {
            Some((7, RevertStatus::Failed(m))) => assert!(m.contains("已在此后被修改"), "{m}"),
            other => panic!("{other:?}"),
        }
        apply(&mut st, Message::Revert(7));
        apply(&mut st, Message::RevertDone { id: 7, result: Err("连接断了".into()) });
        assert!(matches!(st.as_ref().unwrap().revert_status(), Some((7, RevertStatus::Failed(m))) if m.contains("连接断了")));
    }

    #[test]
    fn revert_of_unknown_entry_is_a_noop() {
        let mut st = open(None);
        assert_eq!(apply(&mut st, Message::Revert(99)), Followup::None);
    }

    #[test]
    fn close_drops_state_and_late_results_are_ignored() {
        let mut st = open(None);
        apply(&mut st, Message::Close);
        assert!(st.is_none());
        assert_eq!(
            apply(&mut st, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![]) }),
            Followup::None
        );
    }
}
```

- [ ] **Step 2: 运行确认失败**

在 `extensions.rs` 加 `pub mod edit_history;`,然后:

Run: `cargo test -p dozer-app edit_history 2>&1 | tail -12`
Expected: 编译失败,`State`/`Message`/`apply` 未定义。

- [ ] **Step 3: 实现状态机与 IO**

在 `edit_history.rs` 的 `use` 之后、`#[cfg(test)]` 之前写入:

```rust
/// 列表栏宽与底部操作栏高。视图与宿主里的 diff 区几何计算共用这两个常量。
pub const LIST_WIDTH: f32 = 300.0;
pub const ACTIONS_HEIGHT: f32 = 36.0;
/// 默认加载条数上限。
pub const DEFAULT_LIMIT: u32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum RevertStatus {
    Pending,
    Done,
    Failed(String),
}

pub struct State {
    project_id: i64,
    filter: Option<String>,
    /// `None` = 加载中。
    entries: Option<Result<Vec<FileEditHistoryInfo>, String>>,
    selected: Option<i64>,
    /// 打开弹窗时终端是否可见,决定 Ask Agent 是否置灰(弹窗持有期间主窗口被
    /// 遮罩挡住,可见性不会变)。
    agent_terminal_visible: bool,
    diff_webview_ready: bool,
    diff_sent_for: Option<i64>,
    revert: Option<(i64, RevertStatus)>,
}

impl State {
    pub fn new(project_id: i64, filter: Option<String>, agent_terminal_visible: bool) -> Self {
        Self {
            project_id,
            filter,
            entries: None,
            selected: None,
            agent_terminal_visible,
            diff_webview_ready: false,
            diff_sent_for: None,
            revert: None,
        }
    }
    pub fn project_id(&self) -> i64 {
        self.project_id
    }
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }
    pub fn entries(&self) -> Option<&Result<Vec<FileEditHistoryInfo>, String>> {
        self.entries.as_ref()
    }
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }
    pub fn entry(&self, id: i64) -> Option<&FileEditHistoryInfo> {
        match self.entries.as_ref()? {
            Ok(v) => v.iter().find(|e| e.id == id),
            Err(_) => None,
        }
    }
    pub fn selected_entry(&self) -> Option<&FileEditHistoryInfo> {
        self.entry(self.selected?)
    }
    pub fn agent_terminal_visible(&self) -> bool {
        self.agent_terminal_visible
    }
    pub fn revert_status(&self) -> Option<&(i64, RevertStatus)> {
        self.revert.as_ref()
    }
    pub fn diff_webview_ready(&self) -> bool {
        self.diff_webview_ready
    }
    pub fn set_diff_webview_ready(&mut self, v: bool) {
        self.diff_webview_ready = v;
    }
    pub fn diff_sent_for(&self) -> Option<i64> {
        self.diff_sent_for
    }
    pub fn set_diff_sent_for(&mut self, id: i64) {
        self.diff_sent_for = Some(id);
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// `project_id`/`filter` 用于核对结果落地时还是不是当前请求(弹窗重开或
    /// 切了过滤后,迟到的旧结果直接丢弃)。
    Loaded {
        project_id: i64,
        filter: Option<String>,
        result: Result<Vec<FileEditHistoryInfo>, String>,
    },
    Select(i64),
    SetFilter(Option<String>),
    /// App 拦截(需要预览句柄)。
    Locate(i64),
    /// App 拦截(需要终端句柄)。
    AskAgent(i64),
    Revert(i64),
    RevertDone {
        id: i64,
        result: Result<MutationOutcome, String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Followup {
    None,
    Reload,
    Revert(FileEditHistoryInfo),
}

/// 撤销失败的用户可读原因。
fn revert_failure(outcome: MutationOutcome) -> Option<String> {
    match outcome {
        MutationOutcome::Applied { .. } => None,
        MutationOutcome::Conflict { .. } => Some("文件已在此后被修改,无法撤销".into()),
        MutationOutcome::NotFound => Some("文件已不存在,无法撤销".into()),
        MutationOutcome::PathOutOfBounds => Some("路径越界,无法撤销".into()),
        MutationOutcome::Unwritable { reason } => Some(format!("无法写入: {reason}")),
    }
}

/// 纯状态转移(可单测);IO 由 `update` 按 `Followup` 执行。
pub fn apply(state: &mut Option<State>, msg: Message) -> Followup {
    if let Message::Close = msg {
        *state = None;
        return Followup::None;
    }
    let Some(s) = state.as_mut() else {
        return Followup::None; // 弹窗已关,迟到的消息忽略
    };
    match msg {
        Message::Close => unreachable!("上面已处理"),
        Message::Loaded {
            project_id,
            filter,
            result,
        } => {
            if s.project_id != project_id || s.filter != filter {
                return Followup::None;
            }
            let first = match &result {
                Ok(v) => v.first().map(|e| e.id),
                Err(_) => None,
            };
            let keep = match (&result, s.selected) {
                (Ok(v), Some(id)) if v.iter().any(|e| e.id == id) => Some(id),
                _ => first,
            };
            if keep != s.selected {
                s.diff_sent_for = None;
            }
            s.selected = keep;
            s.entries = Some(result);
            Followup::None
        }
        Message::Select(id) => {
            let old_path = s.selected_entry().map(|e| e.target_path.clone());
            let new_path = s.entry(id).map(|e| e.target_path.clone());
            s.selected = Some(id);
            s.diff_sent_for = None;
            // URL 由文件路径决定:路径变了 webview 会重载,`ready` 要重置。
            if old_path != new_path {
                s.diff_webview_ready = false;
            }
            Followup::None
        }
        Message::SetFilter(filter) => {
            s.filter = filter;
            s.entries = None;
            s.selected = None;
            s.diff_sent_for = None;
            s.diff_webview_ready = false;
            Followup::Reload
        }
        Message::Locate(_) | Message::AskAgent(_) => Followup::None,
        Message::Revert(id) => {
            let Some(entry) = s.entry(id).cloned() else {
                return Followup::None;
            };
            s.revert = Some((id, RevertStatus::Pending));
            Followup::Revert(entry)
        }
        Message::RevertDone { id, result } => match result {
            Ok(outcome) => match revert_failure(outcome) {
                None => {
                    s.revert = Some((id, RevertStatus::Done));
                    Followup::Reload
                }
                Some(reason) => {
                    s.revert = Some((id, RevertStatus::Failed(reason)));
                    Followup::None
                }
            },
            Err(e) => {
                s.revert = Some((id, RevertStatus::Failed(e)));
                Followup::None
            }
        },
    }
}

pub fn request_load(
    project_id: i64,
    filter: Option<String>,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_file_edit_history(project_id, filter.as_deref(), DEFAULT_LIMIT)
            .await
            .map_err(|e| e.to_string());
        emit(Message::Loaded {
            project_id,
            filter,
            result,
        });
    });
}

/// 执行 `apply` 并按 `Followup` 做 IO。Revert:反向调用 `apply_precise_edit`——
/// `expected_text` = 该条 `new_text`,`new_text` = 该条 `old_text`,坐标用该条
/// **修改后**的区间(起点不变,结束坐标取 `new_end_*`)。署名 `actor = "dozer"`
/// (human 经 GUI 触发,不冒充 agent);走同一套 Conflict Detection,磁盘在那次
/// 修改之后又变过就会得到 `Conflict` 而不是盲目覆盖。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match apply(state, msg) {
        Followup::None => {}
        Followup::Reload => {
            let Some(s) = state.as_ref() else { return };
            request_load(s.project_id, s.filter.clone(), client, handle, emit);
        }
        Followup::Revert(e) => {
            let client = client.clone();
            handle.spawn(async move {
                let result = client
                    .apply_precise_edit(
                        e.project_id,
                        &e.target_path,
                        e.start_line,
                        e.start_col,
                        e.new_end_line,
                        e.new_end_col,
                        &e.new_text,
                        &e.old_text,
                        &format!("撤销:{}", e.summary),
                        "dozer",
                        "dozer-gui",
                    )
                    .await
                    .map_err(|err| err.to_string());
                emit(Message::RevertDone { id: e.id, result });
            });
        }
    }
}
```

- [ ] **Step 4: 运行状态机测试确认通过**

Run: `cargo test -p dozer-app edit_history 2>&1 | tail -14`
Expected: 7 条 PASS。

- [ ] **Step 5: 卡片视图 + 打开入口**

1. `file_history.rs`:`fn format_commit_time(` 改为 `pub(crate) fn format_commit_time(`。

2. 在 `edit_history.rs`(`update` 之后、`#[cfg(test)]` 之前)追加视图。列表栏是每条一个整行按钮,右侧详情有头部、diff 占位区(Task 7 的 wry webview 会叠在这块区域上)、操作栏:

```rust
fn flat(color: iced_widget::core::Color) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, _s| button::Style {
        background: None,
        text_color: color,
        ..button::Style::default()
    }
}

fn when(ms: u64) -> String {
    crate::extensions::file_history::format_commit_time((ms / 1000) as i64)
}

pub fn edit_history_card(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();

    let mut title = row![
        icons::view(icons::IconKind::History, byteui::theme::icon_size::row(), colors.cream),
        text("Agent 修改历史")
            .size(byteui::theme::font::subtitle())
            .color(colors.cream),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if let Some(f) = state.filter() {
        title = title.push(
            button(
                text(format!("过滤: {f}  ×"))
                    .size(byteui::theme::font::label())
                    .color(colors.gold),
            )
            .padding(0)
            .on_press(Message::SetFilter(None))
            .style(flat(colors.gold)),
        );
    }
    let title = title
        .push(iced_widget::space::horizontal())
        .push(
            button(text("×").size(byteui::theme::font::subtitle()).color(colors.dim))
                .on_press(Message::Close)
                .style(flat(colors.dim)),
        );

    let body = row![
        container(list_view(state))
            .width(Length::Fixed(LIST_WIDTH))
            .height(Length::Fill),
        detail_view(state),
    ]
    .spacing(12)
    .height(Length::Fill);

    container(column![title, body].spacing(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .style(crate::dialog::card_style)
        .into()
}

fn list_view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let msg = |s: String, c| -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        text(s).size(byteui::theme::font::body()).color(c).into()
    };
    match state.entries() {
        None => msg("加载中…".into(), colors.dim),
        Some(Err(e)) => msg(format!("加载失败: {e}"), colors.red),
        Some(Ok(v)) if v.is_empty() => msg("还没有 agent 修改记录".into(), colors.dim),
        Some(Ok(v)) => {
            let mut col = column![].spacing(4);
            for e in v {
                let selected = state.selected() == Some(e.id);
                let c = if selected { colors.gold } else { colors.cream };
                col = col.push(
                    button(
                        column![
                            text(e.summary.clone()).size(byteui::theme::font::body()).color(c),
                            text(format!("{} · {} · {}", e.target_path, e.actor, when(e.created_ms)))
                                .size(byteui::theme::font::label())
                                .color(colors.dim),
                        ]
                        .spacing(2),
                    )
                    .padding(6)
                    .width(Length::Fill)
                    .on_press(Message::Select(e.id))
                    .style(flat(c)),
                );
            }
            scrollable(col).height(Length::Fill).into()
        }
    }
}

fn detail_view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let Some(e) = state.selected_entry() else {
        return container(text("选择左侧一条修改查看详情").size(byteui::theme::font::body()).color(colors.dim))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let header = text(format!("{}:{}:{}", e.target_path, e.start_line, e.start_col))
        .size(byteui::theme::font::label())
        .color(colors.cream);

    // diff 占位:Task 7 的 wry webview 叠在这块区域上,几何见宿主的 `diff_area_bounds`。
    let diff_slot = container(text("")).width(Length::Fill).height(Length::Fill);

    let action = |label: &'static str, msg: Option<Message>, c| {
        let mut b = button(text(label).size(byteui::theme::font::label()).color(c))
            .padding([4, 10])
            .style(flat(c));
        if let Some(m) = msg {
            b = b.on_press(m);
        }
        b
    };
    let ask = if state.agent_terminal_visible() {
        action("Ask Agent", Some(Message::AskAgent(e.id)), colors.gold)
    } else {
        action("Ask Agent", None, colors.dim)
    };
    let mut actions = row![
        action("Locate", Some(Message::Locate(e.id)), colors.gold),
        ask,
        action("Revert", Some(Message::Revert(e.id)), colors.gold),
        action("只看此文件", Some(Message::SetFilter(Some(e.target_path.clone()))), colors.cream),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    match state.revert_status() {
        Some((id, RevertStatus::Pending)) if *id == e.id => {
            actions = actions.push(text("撤销中…").size(byteui::theme::font::label()).color(colors.dim));
        }
        Some((id, RevertStatus::Done)) if *id == e.id => {
            actions = actions.push(text("已撤销").size(byteui::theme::font::label()).color(colors.green));
        }
        Some((id, RevertStatus::Failed(m))) if *id == e.id => {
            actions = actions.push(text(m.clone()).size(byteui::theme::font::label()).color(colors.red));
        }
        _ => {}
    }

    column![
        header,
        diff_slot,
        container(actions).height(Length::Fixed(ACTIONS_HEIGHT)).align_y(iced_widget::core::alignment::Vertical::Center),
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
```

3. `app/app.rs`:`App` 加字段(紧跟 `pub(crate) file_history: Option<file_history::State>,` 之后)

```rust
    /// Agent 修改历史弹窗状态——`None` 表示未打开;独立原生窗口宿主见
    /// `platform/edit_history_overlay.rs`。
    pub(crate) edit_history: Option<crate::extensions::edit_history::State>,
```
并在 `App` 构造处(`file_history: None,` 之后)加 `edit_history: None,`。

4. `app/message.rs` 在 `FileHistoryDiffWebviewEvent(...)` 之后加:

```rust
    EditHistory(crate::extensions::edit_history::Message),
    /// 修改历史弹窗 diff webview 发回的已校验协议事件(同 `FileHistoryDiffWebviewEvent`)。
    EditHistoryDiffWebviewEvent(
        crate::preview::HostBinding,
        crate::preview::WebviewEnvelope<crate::preview::EditorEvent>,
    ),
```

5. `app/update.rs`:删掉 Task 4 的 `open_edit_history` 存根,换成真实实现,并加路由(`Locate`/`AskAgent` 的真实处理在 Task 8,这里先让它们关闭弹窗以外的行为为空,保证编译):

```rust
    /// 打开修改历史弹窗(独立原生窗口由 `window_events` 的 sync 按
    /// `edit_history.is_some()` 开出)。`filter`:上下文项的 `entity_ref`,
    /// `None` = 全部。
    pub(crate) fn open_edit_history(&mut self, project_id: i64, filter: Option<String>) {
        self.edit_history = Some(crate::extensions::edit_history::State::new(
            project_id,
            filter.clone(),
            self.terminal_visible(),
        ));
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::edit_history::request_load(
            project_id,
            filter,
            &client,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::EditHistory(m));
            },
        );
    }

    pub(crate) fn edit_history_message(&mut self, msg: crate::extensions::edit_history::Message) {
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::edit_history::update(
            &mut self.edit_history,
            msg,
            &client,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::EditHistory(m));
            },
        );
    }
```
`App::update` 里加分支:

```rust
            Message::EditHistory(msg) => self.edit_history_message(msg),
            Message::EditHistoryDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. })
                    && let Some(s) = self.edit_history.as_mut()
                {
                    s.set_diff_webview_ready(true);
                }
            }
```

- [ ] **Step 6: 独立窗口宿主(克隆 `file_history_overlay.rs`)**

1. 克隆并替换:

```bash
cd crates/dozer-app/src/platform
cp file_history_overlay.rs edit_history_overlay.rs
sed -i '' \
  -e 's/FileHistoryOverlay/EditHistoryOverlay/g' \
  -e 's/file_history_overlay/edit_history_overlay/g' \
  -e 's/extensions::file_history/extensions::edit_history/g' \
  -e 's/use crate::extensions::file_history;/use crate::extensions::edit_history;/' \
  -e 's/app\.file_history/app.edit_history/g' \
  -e 's/file_history::file_history_card/edit_history::edit_history_card/g' \
  -e 's/Message::FileHistory(/Message::EditHistory(/g' \
  -e 's/file_history::Message::Close/edit_history::Message::Close/g' \
  -e 's/"file-history"/"edit-history"/g' \
  -e 's/FileHistoryDiffWebviewEvent/EditHistoryDiffWebviewEvent/g' \
  edit_history_overlay.rs
```
(macOS `sed -i ''`。)

2. 手工删掉 `sync_diff_webview` 之前依赖旧 `State` 的部分,Task 6 先不做 diff:把 `sync_diff_webview` 整个函数体替换成只保留签名并直接 `let _ = (app, allowed_files, proxy); self.diff_webview = None; self.diff_webview_bounds = None;`(Task 7 再实现)。`diff_area_bounds` 函数按新布局重写为下面这个(列表宽 `LIST_WIDTH`,详情栏头部一行 + 8 间距,底部操作栏 + 8 间距):

```rust
fn diff_area_bounds(card_logical: LogicalSize<f32>) -> (f32, f32, f32, f32) {
    let pad = 16.0;
    let title_h = byteui::theme::font::subtitle() as f32 * 1.2;
    let header_h = byteui::theme::font::label() as f32 * 1.2;
    let row_spacing = 12.0;
    let col_spacing = 8.0;

    let x = pad + edit_history::LIST_WIDTH + row_spacing;
    let y = pad + title_h + row_spacing + header_h + col_spacing;
    let w = (card_logical.width - x - pad).max(0.0);
    let h = (card_logical.height - y - pad - col_spacing - edit_history::ACTIONS_HEIGHT).max(0.0);
    (x, y, w, h)
}
```
并把 `handle_input` 里的 `Escape` 分支保持返回 `Message::EditHistory(edit_history::Message::Close)`(sed 已替换)。`card_logical_size` 保持宽 40%/高 80%。

3. `platform/mod.rs`:`pub mod file_history_overlay;` 下一行加 `pub mod edit_history_overlay;`。

4. `platform/window_events.rs` 共 10 处登记(行号取自调研时的版本,以 `grep -n file_history_overlay` 复核):
   - (a) 约 50 行 `use crate::platform::file_history_overlay;` 后加 `use crate::platform::edit_history_overlay;`
   - (b) 约 213 行 `Ready` 变体字段 `file_history_overlay: Option<…>,` 后加 `edit_history_overlay: Option<edit_history_overlay::EditHistoryOverlay>,`(附一句同类文档注释)
   - (c) 约 274 行 `enum OverlayKind` 加 `EditHistory,`
   - (d) `close_other_overlays`(约 1217/1237):解构列表加 `edit_history_overlay,`;`if keep != OverlayKind::FileHistory {…}` 之后加
     ```rust
            if keep != OverlayKind::EditHistory {
                *edit_history_overlay = None;
            }
     ```
   - (e) 复制约 1355 行的 `sync_file_history_overlay` 整个函数为 `sync_edit_history_overlay`,做同样的名字替换(`file_history_overlay`→`edit_history_overlay`、`app.file_history.is_some()`→`app.edit_history.is_some()`、`OverlayKind::FileHistory`→`OverlayKind::EditHistory`、`FileHistoryOverlay`→`EditHistoryOverlay`);函数末尾的 `overlay.sync_diff_webview(app, app.allowed_files(), proxy.clone());` 保留(Task 6 里它是空操作)。
   - (f) 约 2845 行初始化列表 `file_history_overlay: None,` 后加 `edit_history_overlay: None,`
   - (g) 约 2879 与 4473 行两处 `self.sync_file_history_overlay(event_loop);` 后各加一行 `self.sync_edit_history_overlay(event_loop);`
   - (h) 约 2954–2982 的"file_history overlay 窗口自己那份 WindowId 的事件"整块复制一份,把 `file_history_overlay`→`edit_history_overlay`、`Message::FileHistory(extensions::file_history::Message::Close)`→`Message::EditHistory(extensions::edit_history::Message::Close)`、末尾 `self.sync_file_history_overlay`→`self.sync_edit_history_overlay`,紧跟原块之后放置
   - (i) 约 3307 与 4173:窗口 resize 处理的解构列表加 `edit_history_overlay,`,并在 `if let Some(overlay) = file_history_overlay { overlay.reposition(…); }` 之后复制一份用于 `edit_history_overlay`
   - (j) 约 4288 `CloseRequested` 里加 `*edit_history_overlay = None; // 图干净,Drop 本身就会释放。`

- [ ] **Step 7: 编译与全量测试**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: unused)" -A8 | head -60`
Expected: 无 error。常见需要就地修的点:`window_events.rs` 里解构列表漏了 `edit_history_overlay`(报 `pattern does not mention field`)——按报错位置补;`edit_history_overlay.rs` 中被 sed 改名后残留的 `git2::Oid`/`LoadedDiff` 引用只存在于旧 `sync_diff_webview`,Step 6.2 已整体替换,若仍有残留 import 一并删除。

Run: `cargo test -p dozer-app 2>&1 | grep -E "test result|FAILED|panicked" | head`
Expected: 全绿(edit_history_overlay 里从旧文件带来的 `sync_action_*` 三条测试也应通过)。

- [ ] **Step 8: Commit**

```bash
git status --short && git diff --cached --stat
git add crates/dozer-app/src/extensions/edit_history.rs crates/dozer-app/src/platform/edit_history_overlay.rs crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/file_history.rs crates/dozer-app/src/platform/mod.rs crates/dozer-app/src/platform/window_events.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(app): agent edit history popup in an independent window

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozer-app/src/extensions/edit_history.rs crates/dozer-app/src/platform/edit_history_overlay.rs crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/file_history.rs crates/dozer-app/src/platform/mod.rs crates/dozer-app/src/platform/window_events.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/update.rs
```

---

### Task 7: 弹窗里的 Diff(CodeMirror diff webview)

**Files:**
- Modify: `crates/dozer-app/src/platform/edit_history_overlay.rs`(实现 `sync_diff_webview`)

**Interfaces:**
- Consumes: Task 6 的 `State::selected_entry()`、`diff_webview_ready()`、`diff_sent_for()`、`set_diff_sent_for(i64)`、`Message::EditHistoryDiffWebviewEvent`(已在 `App::update` 里把 `Ready` 翻成 `set_diff_webview_ready(true)`)。
- 复用:`crate::preview::{EditorHostBinding, EditorCommand::SetDiffDocument, dispatch_script, encode_command, extension_to_syntax}`,与 `FileHistoryOverlay::sync_diff_webview` 同一套。

- [ ] **Step 1: 写失败的测试(纯函数:把"该不该推、推什么"抽出来)**

在 `edit_history_overlay.rs` 的 `#[cfg(test)] mod tests`(从旧文件带来的那个)里追加,并在文件里先声明待实现的函数签名对应的测试:

```rust
    use dozer_core::protocol::FileEditHistoryInfo;

    fn entry(id: i64, path: &str, old: &str, new: &str) -> FileEditHistoryInfo {
        FileEditHistoryInfo {
            id,
            project_id: 1,
            target_path: path.into(),
            actor: "claude".into(),
            session_id: "s".into(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 1,
            new_end_line: 1,
            new_end_col: 1,
            old_text: old.into(),
            new_text: new.into(),
            summary: "s".into(),
            created_ms: 0,
        }
    }

    #[test]
    fn diff_push_needed_only_when_ready_and_not_yet_sent() {
        let e = entry(4, "a.rs", "o", "n");
        assert!(!diff_push_needed(false, None, &e), "webview 未 ready 不推");
        assert!(diff_push_needed(true, None, &e));
        assert!(!diff_push_needed(true, Some(4), &e), "同一条已推过不重复推");
        assert!(diff_push_needed(true, Some(3), &e), "换了条目要重推");
    }

    #[test]
    fn diff_command_carries_old_new_and_language_and_is_read_only() {
        let e = entry(4, "src/a.rs", "", "fn x() {}\n"); // 纯插入:old 为空
        let cmd = diff_command(&e);
        match cmd {
            crate::preview::EditorCommand::SetDiffDocument {
                old_text,
                new_text,
                language,
                read_only,
                ..
            } => {
                assert_eq!(old_text, "");
                assert_eq!(new_text, "fn x() {}\n");
                assert!(read_only);
                assert!(!language.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-app edit_history_overlay 2>&1 | tail -10`
Expected: 编译失败,`diff_push_needed` / `diff_command` 未定义。

- [ ] **Step 3: 实现**

在 `edit_history_overlay.rs` 顶层(`SyncAction` 附近)加两个纯函数:

```rust
/// 是否需要向 diff webview 推送当前选中条目的内容:webview 已 ready 且这一条
/// 还没送达过。
fn diff_push_needed(
    webview_ready: bool,
    sent_for: Option<i64>,
    entry: &dozer_core::protocol::FileEditHistoryInfo,
) -> bool {
    webview_ready && sent_for != Some(entry.id)
}

/// 选中条目 → `SetDiffDocument` 命令。`old_text`/`new_text` 就是这次修改的前后
/// 文本(可为空:纯插入/纯删除),恒只读。
fn diff_command(
    entry: &dozer_core::protocol::FileEditHistoryInfo,
) -> crate::preview::EditorCommand {
    let language = crate::preview::extension_to_syntax(std::path::Path::new(&entry.target_path));
    crate::preview::EditorCommand::SetDiffDocument {
        old_text: entry.old_text.clone(),
        new_text: entry.new_text.clone(),
        language: language.to_string(),
        revision: 0,
        read_only: true,
    }
}
```

把 Task 6 里留空的 `sync_diff_webview` 替换为(结构对照 `file_history_overlay.rs`,差异只有"desired 取自选中条目""内容推送用上面两个函数"):

```rust
    pub(crate) fn sync_diff_webview(
        &mut self,
        app: &mut App,
        allowed_files: Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
        proxy: winit::event_loop::EventLoopProxy<Message>,
    ) {
        let Some(entry) = app
            .edit_history
            .as_ref()
            .and_then(|s| s.selected_entry())
            .cloned()
        else {
            self.diff_webview = None;
            self.diff_webview_bounds = None;
            return;
        };

        let binding = crate::preview::EditorHostBinding::new(
            0,
            crate::app::PanelKind::Files,
            0,
            PathBuf::from(&entry.target_path),
        );
        let url = binding.diff_url(crate::preview::scheme_query_value());
        let logical_size: LogicalSize<f32> = self
            .window
            .inner_size()
            .to_logical(self.window.scale_factor());
        let card_logical = card_logical_size(logical_size.width, logical_size.height);
        let card_offset = centered_card_offset(logical_size, card_logical);
        let (x, y, w, h) = diff_area_bounds(card_logical);
        let (x, y) = (x + card_offset.x, y + card_offset.y);
        let bounds = wry::Rect {
            position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
            size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
        };

        match &mut self.diff_webview {
            Some((view, loaded_url)) => {
                if *loaded_url != url {
                    let _ = view.load_url(&url);
                    *loaded_url = url;
                }
                if self.diff_webview_bounds != Some((x, y, w, h)) {
                    let _ = view.set_bounds(bounds);
                    self.diff_webview_bounds = Some((x, y, w, h));
                }
            }
            None => {
                let root = crate::assets::assets_root();
                let ipc_proxy = proxy;
                let expected_binding = binding.clone();
                let built = wry::WebViewBuilder::new()
                    .with_url(&url)
                    .with_bounds(bounds)
                    .with_visible(true)
                    .with_custom_protocol("dozer".into(), move |_id, request| {
                        let allowed = allowed_files.lock().expect("allowed_files 锁");
                        let reply = crate::assets::handle_protocol(
                            &root,
                            &allowed,
                            None,
                            &request.uri().to_string(),
                        );
                        wry::http::Response::builder()
                            .status(reply.status)
                            .header("Content-Type", reply.mime)
                            .body(std::borrow::Cow::Owned(reply.body))
                            .unwrap()
                    })
                    .with_ipc_handler(move |req| {
                        let expected = crate::preview::HostBinding::new(
                            expected_binding.project_id,
                            expected_binding.panel,
                            expected_binding.tab_id,
                            expected_binding.document_id(),
                        );
                        match crate::preview::parse_event(req.body().as_str()) {
                            Ok(event) => {
                                if let Err(error) = event.validate(&expected) {
                                    tracing::warn!(%error, "拒绝无效 edit-history diff IPC");
                                } else {
                                    let _ = ipc_proxy.send_event(
                                        Message::EditHistoryDiffWebviewEvent(expected, event),
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, "无法解析 edit-history diff IPC");
                            }
                        }
                    })
                    .build_as_child(&self.window);
                match built {
                    Ok(view) => {
                        self.diff_webview = Some((view, url));
                        self.diff_webview_bounds = Some((x, y, w, h));
                    }
                    Err(e) => tracing::warn!("修改历史 diff webview 创建失败: {e}"),
                }
            }
        }

        let need_push = app
            .edit_history
            .as_ref()
            .is_some_and(|s| diff_push_needed(s.diff_webview_ready(), s.diff_sent_for(), &entry));
        if need_push && let Some((view, _)) = &self.diff_webview {
            let script = crate::preview::dispatch_script(&crate::preview::encode_command(
                binding.project_id,
                binding.panel,
                binding.tab_id,
                &binding.document_id(),
                0,
                None,
                diff_command(&entry),
            ));
            let _ = view.evaluate_script(&script);
            if let Some(s) = app.edit_history.as_mut() {
                s.set_diff_sent_for(entry.id);
            }
        }
    }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p dozer-app edit_history_overlay 2>&1 | tail -10 && cargo build -p dozer-app 2>&1 | grep -E "^error" -A8 | head -30`
Expected: 测试 PASS,编译通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/platform/edit_history_overlay.rs
git commit -m "feat(app): show before/after diff in the edit history popup

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozer-app/src/platform/edit_history_overlay.rs
```

---

### Task 8: Locate 与 Ask Agent

**Files:**
- Modify: `crates/dozer-app/src/extensions/edit_history.rs`(新增纯函数 `ask_agent_text`)
- Modify: `crates/dozer-app/src/app/update.rs::edit_history_message`(拦截 `Locate`/`AskAgent`)

**Interfaces:**
- Consumes: `App::preview_open_path_at(path: PathBuf, target_line: Option<usize>)`、`App::term_paste(terminal::TermTarget::Shared, String)`、`edit_history::State::entry(id)`/`project_id()`/`agent_terminal_visible()`。
- Produces: `edit_history::ask_agent_text(entry: &FileEditHistoryInfo) -> String`。

- [ ] **Step 1: 写失败的测试**

`edit_history.rs` 的 `mod tests` 里追加:

```rust
    #[test]
    fn ask_agent_text_has_location_summary_before_and_after() {
        let mut e = info(1, "src/a.rs");
        e.start_line = 3;
        e.start_col = 5;
        e.new_end_line = 4;
        e.new_end_col = 2;
        e.summary = "修正拼写".into();
        e.old_text = "teh".into();
        e.new_text = "the".into();
        assert_eq!(
            ask_agent_text(&e),
            "关于 src/a.rs:3:5-4:2 的这次修改(修正拼写):\n\n修改前:\nteh\n\n修改后:\nthe"
        );
    }

    #[test]
    fn ask_agent_text_keeps_backticks_multiline_chinese_and_empty_sides() {
        let mut e = info(1, "我的 报告/d.md");
        e.old_text = String::new(); // 纯插入
        e.new_text = "```rust\nfn main() {}\n```\n中文".into();
        let t = ask_agent_text(&e);
        assert!(t.contains("修改前:\n\n\n修改后:\n```rust\nfn main() {}\n```\n中文"), "{t}");
        assert!(t.starts_with("关于 我的 报告/d.md:"), "{t}");
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-app ask_agent_text 2>&1 | tail -8`
Expected: 编译失败,`ask_agent_text` 未定义。

- [ ] **Step 3: 实现**

`edit_history.rs`(`request_load` 之前)加:

```rust
/// "Ask Agent"要写进终端的文本。不使用代码围栏(原文可能含反引号/diff),
/// 原样拼接;结束坐标用修改**后**的 `new_end_*`,与 Locate/Revert 一致。
pub fn ask_agent_text(e: &FileEditHistoryInfo) -> String {
    format!(
        "关于 {path}:{sl}:{sc}-{el}:{ec} 的这次修改({summary}):\n\n修改前:\n{old}\n\n修改后:\n{new}",
        path = e.target_path,
        sl = e.start_line,
        sc = e.start_col,
        el = e.new_end_line,
        ec = e.new_end_col,
        summary = e.summary,
        old = e.old_text,
        new = e.new_text,
    )
}
```

`app/update.rs::edit_history_message` 改为先拦截两个需要 App 句柄的消息:

```rust
    pub(crate) fn edit_history_message(&mut self, msg: crate::extensions::edit_history::Message) {
        use crate::extensions::edit_history as eh;
        match msg {
            eh::Message::Locate(id) => {
                let Some((entry, project_id)) = self
                    .edit_history
                    .as_ref()
                    .and_then(|s| s.entry(id).cloned().map(|e| (e, s.project_id())))
                else {
                    return;
                };
                let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
                else {
                    return;
                };
                let path = root.join(&entry.target_path);
                self.edit_history = None; // 关弹窗
                if path.is_file() {
                    // 定位到修改起始行(不选中整段范围,见 plan「有意偏差 3」)。
                    self.preview_open_path_at(path, Some(entry.start_line as usize));
                } else {
                    self.with_project(project_id, move |ws, _io| {
                        ws.preview_error =
                            Some(format!("文件已不存在,无法定位: {}", path.display()));
                    });
                }
            }
            eh::Message::AskAgent(id) => {
                let Some(text) = self
                    .edit_history
                    .as_ref()
                    .filter(|s| s.agent_terminal_visible())
                    .and_then(|s| s.entry(id).map(eh::ask_agent_text))
                else {
                    return;
                };
                self.edit_history = None; // 关弹窗
                self.term_paste(terminal::TermTarget::Shared, text);
            }
            other => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                eh::update(&mut self.edit_history, other, &client, &handle, move |m| {
                    let _ = proxy.send_event(Message::EditHistory(m));
                });
            }
        }
    }
```
(替换 Task 6 里的同名方法整体。)

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p dozer-app ask_agent_text 2>&1 | tail -8 && cargo build -p dozer-app 2>&1 | grep -E "^error" -A8 | head -30`
Expected: 2 条 PASS,编译通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/edit_history.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(app): locate and ask-agent actions in the edit history popup

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- crates/dozer-app/src/extensions/edit_history.rs crates/dozer-app/src/app/update.rs
```

---

### Task 9: 文档、spec 回写与人工验证

**Files:**
- Modify: `docs/user_guide/panels.md`
- Modify: `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`
- Modify: `docs/superpowers/plans/2026-09-30-agent-native-file-editor-phase3-context-history.md`(勾选步骤)

- [ ] **Step 1: 用户文档**

`grep -n "Agent\|Files" docs/user_guide/panels.md | head -20` 找到 Agent 面板与 Files 面板的章节,在 Agent 面板小节末尾追加(措辞贴合该文件既有语气):

```markdown
### Agent 上下文与修改历史

终端下方有一条「Agent 上下文 · N 项」。在文件树右键文件或目录选「添加到 Agent 上下文」,
会把它加入这条列表(同时把一句引用写进终端输入框,不自动回车)。点开列表可以打开某项、
移除某项,或查看该项相关的修改历史。列表每个项目一份,重启后保留。

条右侧的「修改历史」打开一个独立窗口,按时间倒序列出 agent 通过 `apply_precise_edit`
做过的所有精确修改。选中一条后可以:**Locate**(在预览中打开并跳到起始行)、
**Ask Agent**(把这次修改的前后文本写进终端输入框)、**Revert**(撤销;文件在那次
修改之后又被改过时会拒绝,不会覆盖)、**只看此文件**(按文件过滤)。
```

- [ ] **Step 2: 回写 spec 的三处偏差**

在 spec 中:
- §2「终端几何(需要专门测试)」小节整段改为:条恒定高度 `STRIP_HEIGHT`,展开列表以 `stack!` 浮在终端底部之上,不参与几何;`terminal_pane_pixel_size` 无条件扣 `strip_reserved_height()`;对应测试改为"预留高度与展开状态无关"。并把 §2 折叠/展开描述与「测试」里"折叠/展开后 PTY 网格行数一致"一条同步改掉。
- §4「列表」里"顶部文件/目录过滤框"改为"过滤标签(点 × 清除)+ 每条的「只看此文件」按钮";「测试」里对应条目同步。
- §4 Locate 描述改为"定位到修改起始行"。
- §4 Revert 末尾追加一行已知缺口:"dozerd 不检查 GUI 脏 tab(Phase 1 spec 的第 3 条校验实际未在 dozerd 侧实现);脏 tab 会由 T10 磁盘冲突机制兜底,不会静默丢失修改。"

- [ ] **Step 3: 全量验证**

Run: `cargo build 2>&1 | tail -3 && cargo test -p dozer-core -p dozerd -p dozer-client -p dozer-mcp -p dozer-app 2>&1 | grep -E "test result|FAILED|panicked"; cargo clippy --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c | head; cargo fmt --check 2>&1 | head -5`
Expected: 全绿、无新增 clippy 警告、fmt 干净。有 fmt 差异就 `cargo fmt` 后单独看 diff 是否只涉及本计划文件。

- [ ] **Step 4: 人工验证(自动化覆盖不到,逐条在 `cargo run -p dozer-app` 里做)**

- [ ] 文件树右键一个文件 →「添加到 Agent 上下文」:终端出现引用文本,条上计数 +1;再对同一文件重复一次,计数不变。
- [ ] 右键一个目录同理;展开列表,目录项显示文件夹图标。
- [ ] 折叠/展开列表:终端内容**不被推挤**,展开的列表浮在终端底部;窗口高度足够时终端最后一行没有被条盖住。
- [ ] 手动删掉磁盘上的一个已入列文件后点刷新场景(切到别的项目再切回):该项显示「(缺失)」且不可点。
- [ ] 打开项目 A,加一项;切到项目 B,B 的列表独立;在 A 里点「添加」后立刻切到 B,B 的列表**不出现** A 的项。
- [ ] 重启 app:列表仍在(若为空,回到 Task 4 Step 6.4,在恢复点补 `agent_context_refresh`)。
- [ ] 让 agent(或用 `dozer-mcp` 的 `apply_precise_edit`)改一个文件,点「修改历史」:出现这条记录;选中后右侧 diff 区显示 before/after(CodeMirror);切换到另一个路径的记录时 diff 正常重载。
- [ ] **Locate**:弹窗关闭,预览打开该文件并跳到起始行;目标文件已删除时给出提示。
- [ ] **Ask Agent**:终端输入框出现引用文本,未自动回车;终端不可见时(收起右侧)按钮灰显。
- [ ] **Revert**:成功后文件还原、列表顶部多一条 `撤销:…`(署名 dozer);先手动改动同一处再 Revert,得到「文件已在此后被修改,无法撤销」且文件未变。
- [ ] 弹窗是独立窗口:叠在 Files/预览 webview 之上不被遮挡;点弹窗外(主窗口)失焦自动关闭;Esc 关闭。
- [ ] 上下文项右键式入口:展开列表点某项的「历史」,弹窗顶部出现过滤标签且只列出该文件/目录下的记录;点标签 × 恢复全部。

- [ ] **Step 5: Commit**

```bash
git add docs/user_guide/panels.md docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md docs/superpowers/plans/2026-09-30-agent-native-file-editor-phase3-context-history.md
git commit -m "docs: document agent context strip and edit history; sync Phase 3 spec

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>" -- docs/user_guide/panels.md docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md docs/superpowers/plans/2026-09-30-agent-native-file-editor-phase3-context-history.md
```

---

## Self-Review 记录

- **Spec 覆盖**:§1 数据层 → Task 1–3;§2 上下文条(折叠/展开、缺失灰显、点击打开、历史入口、per-project、project_id 路由、刷新时机、几何) → Task 4(几何按"有意偏差 1"处理);§3 发送动作补落库(先落库、失败仍粘贴并提示、选区不落库) → Task 5;§4 弹窗(独立窗口、时间线、预过滤、Diff/Locate/Revert/Ask Agent、`dozer` 署名、`撤销:` summary) → Task 6–8;非目标与测试清单逐条对应到各任务测试或 Task 9 人工验证。"文件树右键作为历史入口"spec 本就没要求,未做。
- **占位符扫描**:无 TBD;`window_events.rs` 的 10 处登记给出了逐处位置与代码/替换规则(其中 6 处是"复制既有块并按名字替换",这是该文件既有的克隆式模式,不是省略);Task 4 Step 6.2 的 `Workspace` 字面量补字段依赖编译器枚举,已写明做法。
- **类型一致性**:`ContextEntry`(GUI)≠`ContextItemInfo`(协议),`Loaded` 携带 `ContextEntry`;`Followup` 在两个模块各自定义、含义不同、互不引用;`FileEditHistoryInfo.new_end_*` 由 Task 2 产出、Task 6 Revert 与 Task 8 Ask Agent 消费;`RevertStatus`/`revert_status()` 返回 `Option<&(i64, RevertStatus)>`,视图与测试写法一致;`open_edit_history` 在 Task 4 为存根、Task 6 Step 5 替换。
- **Review Focus 落点**:LIKE 通配符与尾部 `/` → Task 2;多行 `end_position_after` → Task 2;跨项目应答路由 → Task 4 信封 + Task 9 人工验证;空 old/new → Task 7/8 测试。
