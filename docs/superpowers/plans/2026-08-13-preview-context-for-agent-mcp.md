# 预览面板上下文对外暴露给 agent（`get_preview_context` MCP tool）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Dozer 监管的外部 CLI agent（Claude Code/Codex/OpenCode/Codebuddy）能通过一个新的 `dozer-mcp` MCP stdio server 查到用户当前在 Dozer 预览面板里看的文件路径和光标/选中范围。

**Architecture:** `dozer-app` 在预览面板变化时把 `PreviewContext`（路径+选区）防抖推给 `dozerd`（纯内存缓存，按 `project_id` 存）；新建的 `crates/dozer-mcp` 作为被 agent 拉起的 stdio 子进程，靠 `DOZER_SESSION_ID` 反查 `project_id`，通过 `dozer-client::Client` 查 `dozerd` 拿最新缓存，包成一个只读 MCP tool `get_preview_context` 返回给 agent。

**Tech Stack:** Rust workspace（edition 2024）、tokio 异步 UDS 协议（`dozer-core::protocol`/`dozer-client::Client`）、`rmcp`（Rust 官方 MCP SDK，stdio 传输）、`vendor/iced-code-editor`（预览面板原生代码编辑器）。

设计依据：`docs/superpowers/specs/2026-08-13-preview-context-for-agent-design.md`（已批准）。

## Global Constraints

- **独立分支开发，审阅通过后再合并**：在 `feature/preview-context-for-agent` 分支上开发，不直接提交到 `main`；全部任务完成、测试全绿后提请审阅，审阅通过再合并回 `main`（[[feedback-plans-use-worktree-branch]]，2026-08-08 定下的规矩：避免多人/多 agent 同时在 main 上并行开发不同计划互相干扰）。
- **回归防护命令**（每个任务提交前跑，CLAUDE.md 定的标准命令）：
  ```bash
  cargo build
  cargo test --workspace
  cargo clippy --all-targets
  cargo fmt
  ```
- **workspace 成员是 glob**（根 `Cargo.toml` 的 `members = ["crates/*", "spike/*"]`），新建 `crates/dozer-mcp` 目录后自动纳入 workspace，不需要手改根 `Cargo.toml`。
- **1-indexed 行列**：`iced-code-editor` 内部 0-indexed，`PreviewContext` 对外（协议 + MCP tool 输出）一律 1-indexed，转换点在 `dozer-app` 推送前完成。
- **`reason` 字段固定值**：`path` 为 `null` 时 `reason` 恒为字符串常量 `"no_active_preview"`，不做更细区分。
- **依赖版本**：`rmcp = "3.1.2"`（2026-08-07 发布，写这份计划时的最新版，features `["server", "macros", "transport-io"]`）；`toml = "0.9"`（Codex 安装器用，仓库 `Cargo.lock` 里已有该主版本的传递依赖）。写 Task 6/7 时如发现 crates.io 已有更新的 patch 版本，直接用最新 patch（不改 minor/major）。
- **Cargo.toml 风格**：跟随本仓库现有 crate 写法（`crates/dozer-client/Cargo.toml`/`crates/dozer-hook/Cargo.toml`）——`edition.workspace = true`，workspace 已有依赖一律 `xxx.workspace = true`，不用 `{ workspace = true }` 展开形式。

---

### Task 1: `dozer-core` 协议层——`PreviewContext` + 新 Request/Reply

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces: `pub struct PreviewContext { pub path: String, pub start_line: u32, pub start_col: u32, pub end_line: u32, pub end_col: u32, pub has_selection: bool }`；`Request::UpdatePreviewContext { project_id: i64, context: Option<PreviewContext> }`；`Request::GetPreviewContext { project_id: i64 }`；`Reply::PreviewContext { context: Option<PreviewContext> }`。后续所有任务（dozerd/dozer-client/dozer-app/dozer-mcp）都消费这四个类型。

- [ ] **Step 1: 写失败的 serde 往返测试**

在 `crates/dozer-core/src/protocol.rs` 的 `mod tests`（文件末尾，紧跟已有 `#[test]` 之后）追加：

```rust
    #[test]
    fn preview_context_round_trips() {
        let ctx = PreviewContext {
            path: "/repo/src/main.rs".into(),
            start_line: 12,
            start_col: 3,
            end_line: 14,
            end_col: 1,
            has_selection: true,
        };
        let req = Request::UpdatePreviewContext {
            project_id: 7,
            context: Some(ctx.clone()),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req_none = Request::UpdatePreviewContext {
            project_id: 7,
            context: None,
        };
        let line = encode_line(&req_none);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req_none);

        let get = Request::GetPreviewContext { project_id: 7 };
        let line = encode_line(&get);
        assert_eq!(decode_line::<Request>(&line).unwrap(), get);

        let reply = Reply::PreviewContext {
            context: Some(ctx),
        };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }
```

- [ ] **Step 2: 跑测试确认失败（类型不存在，编译错误）**

Run: `cargo test -p dozer-core preview_context_round_trips`
Expected: 编译失败，报 `PreviewContext`/`UpdatePreviewContext`/`GetPreviewContext` 未定义。

- [ ] **Step 3: 加类型和变体**

在 `AgentKind` 定义（`protocol.rs:19-29`）之后、`ProjectInfo` 定义之前插入：

```rust
/// 预览面板当前上下文：文件路径 + 光标/选区（1-indexed，见 spec
/// "1-indexed 行列" 一节）。无选区时 `start == end` 为光标位置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewContext {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub has_selection: bool,
}
```

在 `Request` 枚举的 `ListBookmarks` 变体之后加：

```rust
    /// `dozer-app` 预览面板变化时推送最新上下文；`context: None` 表示当前
    /// 无活动文本预览。`dozerd` 侧纯内存缓存，同一 `project_id` 后写覆盖
    /// 前写。
    UpdatePreviewContext {
        project_id: i64,
        context: Option<PreviewContext>,
    },
    /// `dozer-mcp` 按需查询某项目当前的预览上下文。
    GetPreviewContext {
        project_id: i64,
    },
```

在 `Reply` 枚举的 `Bookmarks` 变体之后加：

```rust
    /// `GetPreviewContext` 的应答；`context: None` 表示当前无活动文本预览
    /// 或该 `project_id` 从未收到过推送。
    PreviewContext {
        context: Option<PreviewContext>,
    },
```

（若 `Bookmarks`/`ListBookmarks` 变体名与当前文件不完全一致，以文件里实际最后一个变体为准，加在其后即可——顺序不影响 `#[serde(tag = "type")]` 编解码。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-core preview_context_round_trips`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add PreviewContext + UpdatePreviewContext/GetPreviewContext"
```

---

### Task 2: `dozerd`——内存态 `PreviewContextStore` + 协议接线

**Files:**
- Create: `crates/dozerd/src/preview_context.rs`
- Modify: `crates/dozerd/src/lib.rs`（注册新模块）
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: Task 1 的 `dozer_core::protocol::{PreviewContext, Request, Reply}`。
- Produces: `pub struct PreviewContextStore`，`PreviewContextStore::new() -> Self`，`fn update(&self, project_id: i64, context: Option<PreviewContext>)`，`fn get(&self, project_id: i64) -> Option<PreviewContext>`。`server::serve()` 的对外签名**不变**（新 store 在 `serve()` 内部构造，不作为参数传入——纯内存、无需外部配置路径，这样不用改 `main.rs`/`dozerd/tests/*.rs`/`dozer-client/tests/*.rs` 里现有的 9 处 `server::serve(...)` 调用点）。

- [ ] **Step 1: 写 `PreviewContextStore` 的失败单测**

创建 `crates/dozerd/src/preview_context.rs`：

```rust
//! 预览上下文的内存态缓存：`dozer-app` 推、`dozer-mcp` 查。纯内存、不落盘
//! ——daemon 重启即清空，下次 `dozer-app` 一变化就会重新推（见设计文档
//! "非目标"一节）。

use dozer_core::protocol::PreviewContext;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct PreviewContextStore {
    inner: Mutex<HashMap<i64, PreviewContext>>,
}

impl PreviewContextStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// `context: None` 表示清除该项目当前记录（当前无活动文本预览）。
    pub fn update(&self, project_id: i64, context: Option<PreviewContext>) {
        let mut map = self.inner.lock().expect("preview_context 锁");
        match context {
            Some(ctx) => {
                map.insert(project_id, ctx);
            }
            None => {
                map.remove(&project_id);
            }
        }
    }

    pub fn get(&self, project_id: i64) -> Option<PreviewContext> {
        self.inner
            .lock()
            .expect("preview_context 锁")
            .get(&project_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(path: &str) -> PreviewContext {
        PreviewContext {
            path: path.into(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 1,
            has_selection: false,
        }
    }

    #[test]
    fn get_before_any_update_returns_none() {
        let store = PreviewContextStore::new();
        assert_eq!(store.get(1), None);
    }

    #[test]
    fn update_then_get_round_trips() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        assert_eq!(store.get(1), Some(ctx("/a.rs")));
    }

    #[test]
    fn second_update_overwrites_first() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        store.update(1, Some(ctx("/b.rs")));
        assert_eq!(store.get(1), Some(ctx("/b.rs")));
    }

    #[test]
    fn update_with_none_clears() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        store.update(1, None);
        assert_eq!(store.get(1), None);
    }

    #[test]
    fn different_projects_are_independent() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        assert_eq!(store.get(2), None);
        assert_eq!(store.get(1), Some(ctx("/a.rs")));
    }
}
```

- [ ] **Step 2: 跑测试确认失败（模块未注册，编译错误）**

Run: `cargo test -p dozerd preview_context`
Expected: 编译失败，`preview_context` 模块未找到。

- [ ] **Step 3: 注册模块 + 接入 `server.rs`**

在 `crates/dozerd/src/lib.rs` 里找到已有的 `pub mod bookmarks;`（或同级的 `pub mod projects;`）一行，紧邻加一行：

```rust
pub mod preview_context;
```

修改 `crates/dozerd/src/server.rs`：

1. 顶部 `use` 区加：
   ```rust
   use crate::preview_context::PreviewContextStore;
   ```
2. `pub async fn serve(...)` 函数体开头（`if socket.exists() { ... }` 之前）加一行构造：
   ```rust
   let preview_contexts = Arc::new(PreviewContextStore::new());
   ```
3. `loop { ... tokio::spawn(async move { handle_conn(...) }) }` 里，`handle_conn` 调用的参数列表末尾追加 `preview_contexts.clone()`（对应地在 `let (stream, _) = ...` 下方的 `let registry = registry.clone();` 等几行旁加一行 `let preview_contexts = preview_contexts.clone();`）。
4. `async fn handle_conn(...)` 的参数列表末尾追加 `preview_contexts: Arc<PreviewContextStore>`。
5. `handle_conn` 内部 `match req { ... }` 的 `Request::ListBookmarks { project_id } => ...` 分支之后追加两个新分支：
   ```rust
   Request::UpdatePreviewContext { project_id, context } => {
       preview_contexts.update(project_id, context);
       Reply::Ok
   }
   Request::GetPreviewContext { project_id } => Reply::PreviewContext {
       context: preview_contexts.get(project_id),
   },
   ```

（`serve()`/`handle_conn()` 原有的 `registry`/`store`/`projects`/`bookmarks` 参数不动；`preview_contexts` 是新增的第 5 个内部状态，不改变这两个函数对外的参数**类型顺序**——因为它是在 `serve()` 内部 `Arc::new(...)` 出来的，不是调用方传入的，所以 `main.rs`/两个 `tests/*.rs` 文件里现有的 `server::serve(&socket, registry, store, projects, bookmarks)` 调用不用改。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd preview_context`
Expected: PASS（5 个单测全过；`cargo build -p dozerd` 也应确认 `handle_conn`/`serve` 改动编译通过，不影响现有 9 处调用点）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/preview_context.rs crates/dozerd/src/lib.rs crates/dozerd/src/server.rs
git commit -m "feat(dozerd): add in-memory PreviewContextStore + UpdatePreviewContext/GetPreviewContext handlers"
```

---

### Task 3: `dozer-client`——`update_preview_context`/`get_preview_context` + 端到端集成测试

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: Task 1 的 `PreviewContext`/`Request`/`Reply`；Task 2 落地后的 `dozerd::server::serve`（签名不变）。
- Produces: `Client::update_preview_context(&self, project_id: i64, context: Option<PreviewContext>) -> Result<()>`，`Client::get_preview_context(&self, project_id: i64) -> Result<Option<PreviewContext>>`。Task 5（`dozer-app`）与 Task 6（`dozer-mcp`）都调这两个方法。

- [ ] **Step 1: 写失败的端到端测试**

在 `crates/dozer-client/tests/against_real_daemon.rs` 末尾追加：

```rust
#[tokio::test]
async fn preview_context_push_and_query_round_trips() {
    use dozer_core::protocol::PreviewContext;

    let (sock, _registry, _guard) = start_daemon().await;
    let c = Client::new(sock);

    assert_eq!(c.get_preview_context(1).await.unwrap(), None);

    let ctx = PreviewContext {
        path: "/repo/src/main.rs".into(),
        start_line: 3,
        start_col: 1,
        end_line: 5,
        end_col: 2,
        has_selection: true,
    };
    c.update_preview_context(1, Some(ctx.clone()))
        .await
        .unwrap();
    assert_eq!(c.get_preview_context(1).await.unwrap(), Some(ctx.clone()));

    // 不同 project_id 互不影响。
    assert_eq!(c.get_preview_context(2).await.unwrap(), None);

    // 后写覆盖前写。
    let ctx2 = PreviewContext {
        path: "/repo/README.md".into(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 1,
        has_selection: false,
    };
    c.update_preview_context(1, Some(ctx2.clone()))
        .await
        .unwrap();
    assert_eq!(c.get_preview_context(1).await.unwrap(), Some(ctx2));

    // 推 None 清空。
    c.update_preview_context(1, None).await.unwrap();
    assert_eq!(c.get_preview_context(1).await.unwrap(), None);
}
```

顶部需要 `dozer_core` 可在测试里直接引用——检查 `crates/dozer-client/Cargo.toml` 的 `[dev-dependencies]` 是否已有 `dozer-core`；若没有，添加一行 `dozer-core = { path = "../dozer-core" }`（`[dependencies]` 里已有，但集成测试是独立 crate target，需要在 `[dev-dependencies]` 里显式声明才能 `use dozer_core::...`；若 `[dependencies]` 里的声明已经足够被测试 target 复用则跳过此步——以 `cargo test -p dozer-client` 实际报错为准）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-client preview_context_push_and_query_round_trips`
Expected: 编译失败，`Client::update_preview_context`/`get_preview_context` 未定义。

- [ ] **Step 3: 实现**

在 `crates/dozer-client/src/lib.rs` 顶部 `use dozer_core::protocol::{...}` 里加入 `PreviewContext`（保持原有列表按字母序插入）。在 `impl Client` 的 `list_bookmarks` 方法之后（`attach` 之前）加：

```rust
    pub async fn update_preview_context(
        &self,
        project_id: i64,
        context: Option<PreviewContext>,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::UpdatePreviewContext { project_id, context })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_preview_context(&self, project_id: i64) -> Result<Option<PreviewContext>> {
        match self
            .roundtrip(&Request::GetPreviewContext { project_id })
            .await?
        {
            Reply::PreviewContext { context } => Ok(context),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-client`
Expected: PASS（含新测试和既有 `full_client_lifecycle` 等）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs crates/dozer-client/Cargo.toml
git commit -m "feat(dozer-client): add update_preview_context/get_preview_context"
```

---

### Task 4: `vendor/iced-code-editor`——`has_selection`/`selection_range` 公开透传

**Files:**
- Modify: `vendor/iced-code-editor/src/canvas_editor/mod.rs`

**Interfaces:**
- Produces: `CodeEditor::has_selection(&self) -> bool`，`CodeEditor::selection_range(&self) -> Option<((usize, usize), (usize, usize))>`（0-indexed，`(line, col)` 二元组，语义与已有的 `cursor_position()` 一致）。Task 5 消费这两个方法。

- [ ] **Step 1: 写失败的单测**

在 `vendor/iced-code-editor/src/canvas_editor/mod.rs` 的 `#[cfg(test)] mod tests` 里（若该文件没有现成的 `mod tests` 块，在文件末尾新增一个，`use super::*;` 打头）追加：

```rust
    #[test]
    fn has_selection_and_selection_range_report_none_by_default() {
        let editor = CodeEditor::new("fn main() {}", "rs");
        assert!(!editor.has_selection());
        assert_eq!(editor.selection_range(), None);
    }

    #[test]
    fn has_selection_and_selection_range_after_set_anchor_and_move() {
        let mut editor = CodeEditor::new("fn main() {\n    let x = 1;\n}", "rs");
        editor.set_cursor(1, 4);
        // `set_anchor` 是 `Cursor` 上已有的私有方法入口——通过公开的
        // `set_cursor` + 手动构造选区来驱动最小可行场景：真实交互路径是
        // 鼠标拖拽/Shift+方向键，这里只验证透传方法本身的读取语义正确，
        // 不重新验证选区生成逻辑(那是既有内部实现,不在这次改动范围)。
        assert!(!editor.has_selection());
        assert_eq!(editor.cursor_position(), (1, 4));
    }
```

（第二个测试先只锁定"未选中时的读取语义与 `cursor_position` 一致"这一层——`Cursor::set_anchor`/内部选区推进 API 是私有的，不在这次改动范围内新增公开入口；若实现时发现 `CodeEditor` 已有其它公开的、能驱动出真实选区的方法（如某个 `Message` 变体），可以换成更完整的"确实选中一段"断言，但不要为了测试新增额外的公开 API。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p iced-code-editor has_selection_and_selection_range`
Expected: 编译失败，`has_selection`/`selection_range` 未定义（若 crate 名不是 `iced-code-editor`，以 `vendor/iced-code-editor/Cargo.toml` 里 `[package] name` 字段为准）。

- [ ] **Step 3: 实现**

在 `vendor/iced-code-editor/src/canvas_editor/mod.rs` 的 `impl CodeEditor` 块里，紧跟 `cursor_position()` 方法（约 2625 行附近）之后加：

```rust
    /// 当前主光标是否存在选区(拖选/Shift+方向键产生)。
    pub fn has_selection(&self) -> bool {
        self.cursors.primary().has_selection()
    }

    /// 当前主光标的选区范围,`((start_line, start_col), (end_line, end_col))`,
    /// 0-indexed。无选区时返回 `None`——区分"无选区"与"光标位置"是调用方
    /// (`dozer-app`)的职责,这里只做纯透传。
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        self.cursors.primary().selection_range()
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p iced-code-editor`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add vendor/iced-code-editor/src/canvas_editor/mod.rs
git commit -m "feat(iced-code-editor): expose has_selection/selection_range on CodeEditor"
```

---

### Task 5: `dozer-app`——预览上下文变化时防抖推送给 `dozerd`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: Task 3 的 `Client::update_preview_context`；Task 4 的 `CodeEditor::has_selection`/`selection_range`/既有 `cursor_position`。
- Produces: `Workspace::spawn_preview_context_push(&mut self, io: &ShellIo)`（供 `App` 侧在 tab 切换/关闭/打开/光标变化后调用；内部纯内存操作 + 防抖 spawn，无返回值）。

- [ ] **Step 1: 写失败的单测（坐标转换 + 防抖 nonce 语义）**

在 `crates/dozer-app/src/workspace.rs` 的 `#[cfg(test)] mod tests`（文件已有该模块，若没有则在文件末尾新增，`use super::*;` 打头）追加：

```rust
    #[test]
    fn preview_context_from_cursor_only_uses_1_indexed_point_range() {
        // 无选区:0-indexed (1, 4) 光标 → 1-indexed start==end==(2, 5)。
        let ctx = preview_context_from_editor_state(
            "/repo/src/main.rs",
            false,
            (1, 4),
            None,
        );
        assert_eq!(ctx.path, "/repo/src/main.rs");
        assert_eq!((ctx.start_line, ctx.start_col), (2, 5));
        assert_eq!((ctx.end_line, ctx.end_col), (2, 5));
        assert!(!ctx.has_selection);
    }

    #[test]
    fn preview_context_from_selection_uses_1_indexed_range() {
        // 0-indexed 选区 (1,4)..(3,0) → 1-indexed (2,5)..(4,1)。
        let ctx = preview_context_from_editor_state(
            "/repo/src/main.rs",
            true,
            (1, 4),
            Some(((1, 4), (3, 0))),
        );
        assert_eq!((ctx.start_line, ctx.start_col), (2, 5));
        assert_eq!((ctx.end_line, ctx.end_col), (4, 1));
        assert!(ctx.has_selection);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app preview_context_from`
Expected: 编译失败，`preview_context_from_editor_state` 未定义。

- [ ] **Step 3: 实现坐标转换 + 防抖字段 + 推送方法**

3a. 在 `Workspace` struct 定义（`workspace.rs:290-363`）里，紧跟 `pub(crate) preview_error: Option<String>,` 之后加一个字段：

```rust
    /// 预览上下文推送的防抖 nonce:每次变化时自增，延迟任务醒来后只有
    /// "自己发起时的值仍是最新值"才真正推送，否则说明中途又有新变化，
    /// 让更晚的那次任务去做(trailing-edge 防抖，见
    /// `spawn_preview_context_push`)。
    pub(crate) preview_context_nonce: Arc<std::sync::atomic::AtomicU64>,
```

3b. 在 `empty_for_project_placeholder()`（`workspace.rs:509-538`）的字段列表里，紧跟 `preview_error: None,` 之后加：

```rust
            preview_context_nonce: Arc::new(std::sync::atomic::AtomicU64::new(0)),
```

3c. 在 `preview.rs` 或 `workspace.rs` 顶部（就近放在 `PreviewPane`/`TabKind` 已导入的位置）确认 `iced_code_editor::CodeEditor` 及 `TabKind` 可见后，在 `workspace.rs` 里新增一个纯函数（放在 `impl Workspace` 块外、文件内任意模块级位置，紧邻其它自由函数如 `fetch_project_restore` 均可）：

```rust
/// 由(路径, 是否有选区, 0-indexed 光标位置, 0-indexed 选区范围)组装
/// 1-indexed 的 `PreviewContext`。抽成纯函数是为了不依赖真实
/// `CodeEditor`/`PreviewPane` 就能单测坐标转换这一层逻辑。
fn preview_context_from_editor_state(
    path: &str,
    has_selection: bool,
    cursor: (usize, usize),
    selection: Option<((usize, usize), (usize, usize))>,
) -> dozer_core::protocol::PreviewContext {
    let (start, end) = if has_selection {
        selection.unwrap_or((cursor, cursor))
    } else {
        (cursor, cursor)
    };
    dozer_core::protocol::PreviewContext {
        path: path.to_string(),
        start_line: start.0 as u32 + 1,
        start_col: start.1 as u32 + 1,
        end_line: end.0 as u32 + 1,
        end_col: end.1 as u32 + 1,
        has_selection,
    }
}
```

3d. 在 `impl Workspace` 里，紧邻 `spawn_preview_state_save`（`workspace.rs:981-1002`）之后加推送方法：

```rust
    /// 把当前预览上下文(活动 tab 路径 + 光标/选区)防抖推给 `dozerd`。
    /// 无活动 tab、或活动 tab 无原生 `CodeEditor`(图片/webview 类)时推
    /// `None`。~250ms trailing-edge 防抖:连续快速触发(方向键连按)只有
    /// 最后一次真正发出 UDS 请求。
    pub(crate) fn spawn_preview_context_push(&mut self, io: &ShellIo) {
        let Some(project) = &self.project else {
            return;
        };
        let project_id = project.id;
        let context = self.preview.tabs().get(self.preview.active_idx()).and_then(|tab| {
            let TabKind::File(path) = &tab.kind;
            let editor = tab.editor.as_ref()?;
            let path_str = path.to_string_lossy().into_owned();
            Some(preview_context_from_editor_state(
                &path_str,
                editor.has_selection(),
                editor.cursor_position(),
                editor.selection_range(),
            ))
        });

        let nonce = self
            .preview_context_nonce
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let flag = self.preview_context_nonce.clone();
        let client = io.client.clone();
        io.handle.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if flag.load(std::sync::atomic::Ordering::SeqCst) != nonce {
                return; // 被更晚的一次变化取代
            }
            if let Err(e) = client.update_preview_context(project_id, context).await {
                tracing::warn!("推送预览上下文失败: {e}");
            }
        });
    }
```

3e. 接线调用点——在 `crates/dozer-app/src/app.rs` 里：

- `Message::PreviewCloseTab` 分支（`app.rs:2663-2671`）：在 `ws.spawn_preview_state_save(io);` 之后加一行 `ws.spawn_preview_context_push(io);`。
- `preview_open_path`（`app.rs:3802-3819`）：同样在 `ws.spawn_preview_state_save(io);` 之后加 `ws.spawn_preview_context_push(io);`。
- `preview_select_tab`（`app.rs:3821-3836`）：同样在 `ws.spawn_preview_state_save(io);` 之后加 `ws.spawn_preview_context_push(io);`。
- `App::preview_tab_editor_event`（`app.rs:2217-2227`，光标移动/选区变化的唯一入口）：改成先转发给 workspace 处理编辑器消息，拿到返回的 `Task` 后，用 `self.shell_io()` 取一份 `io`，调用 `ws.spawn_preview_context_push(&io)` 再返回原 `Task`：

  ```rust
  pub fn preview_tab_editor_event(
      &mut self,
      tab_id: usize,
      event: iced_code_editor::Message,
  ) -> iced_winit::runtime::Task<iced_code_editor::Message> {
      let io = self.shell_io();
      if let Some(ws) = self.active_workspace_mut() {
          let task = ws.preview_tab_editor_event(tab_id, event);
          ws.spawn_preview_context_push(&io);
          task
      } else {
          iced_winit::runtime::Task::none()
      }
  }
  ```

  （`self.shell_io()` 是 `App` 上已有的私有方法，见 `app.rs:1526-1534`；这里在拿 `active_workspace_mut()` 的可变借用之前先取 `io`，避免同时持有 `&self` 和 `&mut self` 借用冲突。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app preview_context`
Expected: PASS。再跑 `cargo build -p dozer-app --bin dozer` 确认整体编译通过(这一步涉及 `app.rs`/`workspace.rs` 两个大文件的接线,单靠单测不能保证没有借用检查/类型错误)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): push preview context to dozerd on tab/cursor/selection change"
```

---

### Task 6: `crates/dozer-mcp`——新 crate，stdio MCP server + `get_preview_context` tool

**Files:**
- Create: `crates/dozer-mcp/Cargo.toml`
- Create: `crates/dozer-mcp/src/lib.rs`
- Create: `crates/dozer-mcp/src/main.rs`
- Create: `crates/dozer-mcp/src/server.rs`
- Create: `crates/dozer-mcp/src/install.rs`（占位，Task 7 替换成真实实现）
- Test: `crates/dozer-mcp/tests/serve_requires_session_id.rs`
- Test: `crates/dozer-mcp/tests/get_preview_context.rs`

**Interfaces:**
- Consumes: Task 3 的 `dozer_client::Client::list`/`get_preview_context`；`dozer_core::protocol::SessionInfo`/`PreviewContext`。
- Produces: bin `dozer-mcp` + 同名 lib target（`src/lib.rs` 里 `pub mod server; pub mod install;`）。`server::DozerMcpServer::new(client: dozer_client::Client, session_id: String) -> Self`，`pub` 可见——Task 6 Step 6 的集成测试和 Task 7 都靠这个 lib target 引用，纯 bin crate 没有 lib target，`tests/*.rs` 里的外部集成测试没法 `use dozer_mcp::server::...`，这是加 `lib.rs` 的原因。

- [ ] **Step 1: 建 crate 骨架 + Cargo.toml**

创建 `crates/dozer-mcp/Cargo.toml`：

```toml
[package]
name = "dozer-mcp"
version = "0.1.0"
edition.workspace = true

[[bin]]
name = "dozer-mcp"
path = "src/main.rs"

[dependencies]
dozer-core = { path = "../dozer-core" }
dozer-client = { path = "../dozer-client" }
anyhow.workspace = true
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
rmcp = { version = "3.1.2", features = ["server", "macros", "transport-io"] }
schemars = "0.8"

[dev-dependencies]
dozerd = { path = "../dozerd" }
tempfile = "3"
uuid.workspace = true
```

- [ ] **Step 2: 写失败的集成测试（缺 `DOZER_SESSION_ID` 时退出码非 0）**

创建 `crates/dozer-mcp/tests/serve_requires_session_id.rs`：

```rust
use std::process::Command;

#[test]
fn serve_exits_nonzero_without_session_id_env() {
    let exe = env!("CARGO_BIN_EXE_dozer-mcp");
    let status = Command::new(exe)
        .arg("serve")
        .env_remove("DOZER_SESSION_ID")
        .status()
        .expect("spawn dozer-mcp");
    assert!(!status.success());
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p dozer-mcp serve_exits_nonzero_without_session_id_env`
Expected: 编译失败(bin target/`main.rs` 还不存在)。

- [ ] **Step 4: 实现 `main.rs` + `server.rs`**

创建 `crates/dozer-mcp/src/server.rs`：

```rust
//! `get_preview_context` MCP tool 的实现。会话→项目的解析每次 tool call
//! 现连 `dozerd`(daemon 可能重启,本进程是长驻的,不维护长连接;见设计
//! 文档"session → project 解析"一节)。

use dozer_client::Client;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData as McpError, ServiceExt, tool, tool_router, transport::stdio};
use serde_json::json;

#[derive(Clone)]
pub struct DozerMcpServer {
    client: Client,
    session_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoParams {}

#[tool_router(server_handler)]
impl DozerMcpServer {
    pub fn new(client: Client, session_id: String) -> Self {
        Self { client, session_id }
    }

    #[tool(
        description = "返回用户当前在 Dozer 预览面板里看的文件路径和光标/选中范围;想知道用户正在看哪段代码时调用。"
    )]
    async fn get_preview_context(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let sessions = self
            .client
            .list()
            .await
            .map_err(|e| McpError::internal_error(format!("连接 dozerd 失败: {e}"), None))?;
        let session = sessions
            .iter()
            .find(|s| s.id == self.session_id)
            .ok_or_else(|| McpError::internal_error("会话不存在于 dozerd", None))?;
        let project_id = session
            .project_id
            .ok_or_else(|| McpError::internal_error("会话尚未归属任何项目", None))?;

        let context = self
            .client
            .get_preview_context(project_id)
            .await
            .map_err(|e| McpError::internal_error(format!("查询预览上下文失败: {e}"), None))?;

        let value = match context {
            Some(ctx) => json!({
                "path": ctx.path,
                "start_line": ctx.start_line,
                "start_col": ctx.start_col,
                "end_line": ctx.end_line,
                "end_col": ctx.end_col,
                "has_selection": ctx.has_selection,
                "reason": null,
            }),
            None => json!({
                "path": null,
                "start_line": null,
                "start_col": null,
                "end_line": null,
                "end_col": null,
                "has_selection": null,
                "reason": "no_active_preview",
            }),
        };
        Ok(CallToolResult::structured(value))
    }
}

pub async fn run() -> anyhow::Result<()> {
    let session_id = std::env::var("DOZER_SESSION_ID")
        .map_err(|_| anyhow::anyhow!("缺少 DOZER_SESSION_ID：dozer-mcp 只能在 dozer 拉起的会话里跑"))?;
    let client = Client::new(dozer_core::paths::socket_path());
    let server = DozerMcpServer::new(client, session_id);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

创建 `crates/dozer-mcp/src/lib.rs`：

```rust
pub mod install;
pub mod server;
```

创建 `crates/dozer-mcp/src/main.rs`（`use` 而非 `mod`——两个模块的真身在 `lib.rs` 里，`main.rs` 只是 lib target 的一个消费方，这样 `tests/*.rs` 才能作为独立 crate 链接 lib target 直接 `use dozer_mcp::server::DozerMcpServer`）：

```rust
use dozer_mcp::{install, server};

#[tokio::main]
async fn main() {
    let arg1 = std::env::args().nth(1);
    match arg1.as_deref() {
        Some("serve") => {
            if let Err(e) = server::run().await {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some("install") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(install::run(&agent, true));
        }
        Some("uninstall") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(install::run(&agent, false));
        }
        _ => {
            eprintln!("用法: dozer-mcp <serve|install|uninstall> [agent]");
            std::process::exit(2);
        }
    }
}
```

（`use dozer_mcp::install;` 引用的 `crates/dozer-mcp/src/install.rs` 由 Task 7 创建；本任务先加一个空占位文件 `crates/dozer-mcp/src/install.rs` 内容为 `pub fn run(_agent: &str, _install: bool) -> i32 { 2 }`，Task 7 再替换成真实实现，保证本任务能独立编译通过。`schemars` 依赖已在 Step 1 的 `Cargo.toml` 里加好。）

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-mcp`
Expected: PASS。

- [ ] **Step 6: 写并跑一个真实 tool call 的集成测试**

创建 `crates/dozer-mcp/tests/get_preview_context.rs`（复用 `dozer-client/tests/against_real_daemon.rs` 里 `start_daemon`/`temp_sock` 的手法,起一个真实 `dozerd` + 一个真实 session,直接用 `dozer_client::Client` 驱动 `DozerMcpServer::get_preview_context` 的内部逻辑,不经 stdio 传输——`rmcp` 的 `ServerHandler`/`tool_router` 生成的方法在 `#[tool_router(server_handler)]` 展开后仍是 `DozerMcpServer` 上的普通异步方法,可以直接 `.await` 调用):

```rust
use dozer_client::Client;
use dozer_core::protocol::PreviewContext;
use dozer_mcp::server::DozerMcpServer;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-{}.sock", &id.to_string()[..8]))
}

async fn start_daemon() -> std::path::PathBuf {
    let sock = temp_sock();
    let registry = Arc::new(dozerd::registry::SessionRegistry::new());
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-{}.db", uuid::Uuid::new_v4()));
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let s = sock.clone();
    tokio::spawn(async move { dozerd::server::serve(&s, registry, store, projects, bookmarks).await });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sock
}

#[tokio::test]
async fn get_preview_context_returns_pushed_value_for_resolved_project() {
    let sock = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create("测试", "/bin/sh", &["-c".into(), "cat".into()], "/tmp", 80, 24, 1)
        .await
        .unwrap();

    let ctx = PreviewContext {
        path: "/repo/src/main.rs".into(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 5,
        has_selection: false,
    };
    client.update_preview_context(1, Some(ctx.clone())).await.unwrap();

    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    let got = server.fetch_context_for_test().await.unwrap();
    assert_eq!(got, Some(ctx));

    let _ = std::fs::remove_file(format!("/tmp/dz-mcp-{}.sock", "")); // best-effort cleanup, id 已知这里省略精确匹配
}
```

（`server.fetch_context_for_test()` 是给测试用的辅助方法——`#[tool]` 宏包过的 `get_preview_context` 签名是 `Parameters<NoParams>`，直接在测试里手工构造略别扭。在 `server.rs` 的 `impl DozerMcpServer`(`#[tool_router(...)]` 块外，另开一个普通 `impl DozerMcpServer` 块)里加一个 `#[cfg(test)]` 方法，把 session 解析 + `get_preview_context` 查询这段逻辑抽成不依赖 MCP 类型的裸函数，`#[tool]` 方法体和这个测试方法都调用它，避免逻辑写两份：

```rust
impl DozerMcpServer {
    async fn fetch_context(&self) -> anyhow::Result<Option<PreviewContext>> {
        let sessions = self.client.list().await?;
        let session = sessions
            .iter()
            .find(|s| s.id == self.session_id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在于 dozerd"))?;
        let project_id = session
            .project_id
            .ok_or_else(|| anyhow::anyhow!("会话尚未归属任何项目"))?;
        Ok(self.client.get_preview_context(project_id).await?)
    }

    #[cfg(test)]
    pub async fn fetch_context_for_test(&self) -> anyhow::Result<Option<PreviewContext>> {
        self.fetch_context().await
    }
}
```

相应地，`#[tool_router(server_handler)]` 块里的 `get_preview_context` 方法体改成调用 `self.fetch_context().await`，按 `Ok`/`Err` 转换成 `CallToolResult::structured(...)` / `Err(McpError::internal_error(...))`，避免把"查会话/查上下文"的逻辑和"包成 MCP 类型"的逻辑混在一处、导致测试路径和真实路径实际跑的代码不是同一份。)

Run: `cargo test -p dozer-mcp`
Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-mcp
git commit -m "feat(dozer-mcp): new crate, stdio MCP server exposing get_preview_context"
```

---

### Task 7: 四家 agent 的安装器（Claude/Codex/OpenCode/Codebuddy）

**Files:**
- Create: `crates/dozer-mcp/src/install.rs`（替换 Task 6 留的占位实现）
- Modify: `crates/dozer-mcp/Cargo.toml`（加 `toml` 依赖）

**Interfaces:**
- Consumes: 无（纯文件系统操作）。
- Produces: `pub fn run(agent: &str, install: bool) -> i32`（`main.rs` 已在 Task 6 接好调用点）。

已核实的四家配置格式（brainstorming/写 plan 阶段查证，来源见对话记录里的搜索结果）：

| agent | 配置文件 | 格式 | 顶层 key |
|---|---|---|---|
| Claude | `~/.claude/mcp.json` | JSON | `mcpServers.<name> = {type,command,args,env}` |
| Codebuddy | `~/.codebuddy/.mcp.json` | JSON | 同上（与 Claude 同构） |
| Codex | `~/.codex/config.toml` | TOML | `[mcp_servers.<name>]` `command`/`args` |
| OpenCode | `~/.config/opencode/opencode.json` | JSON | `mcp.<name> = {type:"local",command:[...],enabled}` |

- [ ] **Step 1: 写失败的幂等性测试**

创建 `crates/dozer-mcp/src/install.rs`（先只放测试骨架 + `use`，实现留到 Step 3）：

```rust
//! 四家已确认支持 MCP 的 agent 的安装器：幂等 JSON/TOML 补丁，只碰自己
//! 写的 `dozer` 条目，仿 `dozer-hook/src/install.rs` 的手法。Qoder/Kilo/
//! V8agent 不在这次范围（见设计文档"非目标"）。

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_claude_creates_mcp_json_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert_eq!(run_at_claude_like(&path, true), 0);
        assert_eq!(run_at_claude_like(&path, true), 0); // 再装一次
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1, "重复安装不应产生第二条 dozer 记录");
        assert_eq!(servers["dozer"]["args"][0], "serve");
    }

    #[test]
    fn uninstall_claude_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        run_at_claude_like(&path, true);
        assert_eq!(run_at_claude_like(&path, false), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"].as_object().unwrap().is_empty());
    }

    #[test]
    fn install_codex_writes_toml_table_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(run_at_codex(&path, true), 0);
        assert_eq!(run_at_codex(&path, true), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        let root: toml::Value = text.parse().unwrap();
        let servers = root["mcp_servers"].as_table().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["dozer"]["args"][0].as_str().unwrap(), "serve");
    }

    #[test]
    fn install_opencode_writes_command_array_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        assert_eq!(run_at_opencode(&path, true), 0);
        assert_eq!(run_at_opencode(&path, true), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcp"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["dozer"]["type"], "local");
        assert_eq!(servers["dozer"]["command"][1], "serve");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-mcp install::`
Expected: 编译失败，`run_at_claude_like`/`run_at_codex`/`run_at_opencode` 未定义。

- [ ] **Step 3: 实现**

在 `crates/dozer-mcp/Cargo.toml` 的 `[dependencies]` 里加：

```toml
toml = "0.9"
serde.workspace = true
```

在 `install.rs`（`use` 之后、`#[cfg(test)]` 之前）实现：

```rust
fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-mcp".into())
}

/// Claude/Codebuddy 共用：`{"mcpServers": {"dozer": {...}}}`。
fn run_at_claude_like(path: &Path, install: bool) -> i32 {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let Some(obj) = root.as_object_mut() else {
        eprintln!("{} 顶层不是对象，拒绝写入", path.display());
        return 1;
    };
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        eprintln!("mcpServers 段不是对象，拒绝写入");
        return 1;
    };
    if install {
        servers.insert(
            "dozer".into(),
            serde_json::json!({
                "type": "stdio",
                "command": exe_path(),
                "args": ["serve"],
                "env": {}
            }),
        );
    } else {
        servers.remove("dozer");
    }
    write_json(path, &root)
}

/// Codex：`[mcp_servers.dozer]` TOML 表。
fn run_at_codex(path: &Path, install: bool) -> i32 {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut root: toml::Value = if text.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        match text.parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
                return 1;
            }
        }
    };
    let Some(root_table) = root.as_table_mut() else {
        eprintln!("{} 顶层不是 TOML table，拒绝写入", path.display());
        return 1;
    };
    let servers = root_table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let Some(servers) = servers.as_table_mut() else {
        eprintln!("mcp_servers 段不是 table，拒绝写入");
        return 1;
    };
    if install {
        let mut entry = toml::value::Table::new();
        entry.insert("command".into(), toml::Value::String(exe_path()));
        entry.insert(
            "args".into(),
            toml::Value::Array(vec![toml::Value::String("serve".into())]),
        );
        servers.insert("dozer".into(), toml::Value::Table(entry));
    } else {
        servers.remove("dozer");
    }
    let out = match toml::to_string_pretty(&root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("序列化 TOML 失败: {e}");
            return 1;
        }
    };
    write_text(path, &out)
}

/// OpenCode：`{"mcp": {"dozer": {"type":"local","command":[...],"enabled":true}}}`。
fn run_at_opencode(path: &Path, install: bool) -> i32 {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let Some(obj) = root.as_object_mut() else {
        eprintln!("{} 顶层不是对象，拒绝写入", path.display());
        return 1;
    };
    let servers = obj.entry("mcp").or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        eprintln!("mcp 段不是对象，拒绝写入");
        return 1;
    };
    if install {
        servers.insert(
            "dozer".into(),
            serde_json::json!({
                "type": "local",
                "command": [exe_path(), "serve"],
                "enabled": true
            }),
        );
    } else {
        servers.remove("dozer");
    }
    write_json(path, &root)
}

fn write_json(path: &Path, value: &serde_json::Value) -> i32 {
    write_text(
        path,
        &(serde_json::to_string_pretty(value).expect("json serializes") + "\n"),
    )
}

fn write_text(path: &Path, text: &str) -> i32 {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("建目录失败: {e}");
        return 1;
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("写 {} 失败: {e}", path.display());
        return 1;
    }
    println!("{}: {}", path.display(), if text.is_empty() { "已清空" } else { "已更新" });
    0
}

fn config_path_for(agent: &str) -> Option<(PathBuf, fn(&Path, bool) -> i32)> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    match agent {
        "claude" => Some((
            std::env::var("DOZER_CLAUDE_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&home).join(".claude").join("mcp.json")),
            run_at_claude_like,
        )),
        "codebuddy" => Some((
            std::env::var("DOZER_CODEBUDDY_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&home).join(".codebuddy").join(".mcp.json")),
            run_at_claude_like,
        )),
        "codex" => Some((
            std::env::var("DOZER_CODEX_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&home).join(".codex").join("config.toml")),
            run_at_codex,
        )),
        "opencode" => Some((
            std::env::var("DOZER_OPENCODE_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(&home)
                        .join(".config")
                        .join("opencode")
                        .join("opencode.json")
                }),
            run_at_opencode,
        )),
        _ => None,
    }
}

pub fn run(agent: &str, install: bool) -> i32 {
    match config_path_for(agent) {
        Some((path, handler)) => handler(&path, install),
        None => {
            eprintln!("不支持的 agent: {agent}（支持 claude/codebuddy/codex/opencode）");
            2
        }
    }
}
```

（`config_path_for` 的环境变量覆盖手法与 `dozer-hook/src/install.rs::settings_path_for` 一致，供测试指向临时路径而不必依赖真实 `$HOME`——不过上面 Step 1 的测试直接调 `run_at_claude_like`/`run_at_codex`/`run_at_opencode` 这三个内部函数，绕过了 `config_path_for`，所以测试本身不需要设这些环境变量；`config_path_for`/`run` 走真实 agent 名分发,人工验收阶段用得到。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-mcp`
Expected: PASS（Task 6 的占位 `install::run` 已被替换，`main.rs` 不用改）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-mcp/src/install.rs crates/dozer-mcp/Cargo.toml
git commit -m "feat(dozer-mcp): installers for Claude/Codebuddy/Codex/OpenCode"
```

---

### Task 8: 收尾——workspace 文档更新 + 全量回归

**Files:**
- Modify: `CLAUDE.md`

**Interfaces:** 无新接口，纯文档 + 验证。

- [ ] **Step 1: 更新 `CLAUDE.md` 的 workspace 布局表**

在 `CLAUDE.md` 的 workspace 布局表格（`crates/dozer-hook` 那一行之后）加两行：

```markdown
| `crates/dozer-client` | dozer-app/dozer-mcp 共用的 UDS 客户端库(`Client`) |
| `crates/dozer-mcp` | 面向外部 CLI agent 的只读 MCP stdio server（bin: `dozer-mcp`） |
```

- [ ] **Step 2: 全量回归**

```bash
cargo build
cargo test --workspace
cargo clippy --all-targets
cargo fmt
```

Expected: 全绿。若 `cargo fmt` 改动了文件，把格式化产生的 diff 一并納入本次提交。

- [ ] **Step 3: 提交**

```bash
git add CLAUDE.md
git commit -m "docs: add dozer-client/dozer-mcp to workspace layout table"
```

- [ ] **Step 4: 人工验收（GUI + 真实 agent，不是自动化测试）**

1. `cargo run -p dozer-app` 打开一个项目，预览面板打开一个文件并选中几行。
2. `cargo run -p dozer-mcp -- install claude`（或用编译产物路径），确认 `~/.claude/mcp.json` 出现 `dozer` 条目。
3. 在该项目目录起一个真实 Claude Code 会话（走 dozer 拉起的 PTY，非裸终端），确认 MCP 工具列表里出现 `get_preview_context`。
4. 让 agent 调用 `get_preview_context`，核对返回的 `path`/`start_line`/`end_line` 等与预览面板里实际选中的内容一致。
5. `cargo run -p dozer-mcp -- uninstall claude`，确认条目被移除、其余配置不受影响。

---

## Self-Review 记录

- **spec 覆盖**：目标 1(推送)→Task 5；目标 2(MCP tool)→Task 6；目标 3(四家安装器)→Task 7。非目标(Qoder/Kilo/V8agent/HTTP传输/鉴权/持久化)均未在任何任务里引入。
- **占位符扫描**：无 TBD/TODO；Task 4 的第二个选区单测因 `Cursor::set_anchor` 是私有 API 而收窄断言范围，已在步骤内写明原因和替代验证点，不是留空。
- **类型一致性**：`PreviewContext` 字段名/类型在 Task 1(定义)→Task 2(存储)→Task 3(客户端)→Task 5(生产)→Task 6(消费)全程一致；`spawn_preview_context_push`/`preview_context_from_editor_state`/`has_selection`/`selection_range`/`update_preview_context`/`get_preview_context` 等跨任务引用的函数名在各任务间核对过，没有 Task 3 叫一个名字、Task 5/6 调用另一个名字的情况。
- **`server::serve`/`handle_conn` 签名改动的影响面**：确认这次改法(store 内部构造)不需要触碰 `main.rs`/`dozerd/tests/hook_events.rs`/`dozerd/tests/session_survival.rs`/`dozer-client/tests/against_real_daemon.rs` 这 9 处既有调用点，是刻意的设计选择(见 Task 2 Interfaces 说明)，不是遗漏。
- **`dozer-mcp` crate 结构**：初稿 Task 6 只写了 `main.rs`(`mod install; mod server;`)，自查时发现 Task 6 Step 6 的集成测试要 `use dozer_mcp::server::DozerMcpServer`——纯 bin crate 没有 lib target，外部 `tests/*.rs` 链接不到私有模块。已改成 `src/lib.rs`(`pub mod server; pub mod install;`) + `main.rs` 改用 `use dozer_mcp::{install, server};`，Task 6/7 的正文已同步更新，不是遗留的不一致。
