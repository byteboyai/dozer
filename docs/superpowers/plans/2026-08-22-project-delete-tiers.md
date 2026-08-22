# 删除项目三级方案 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 项目面板"删除项目"从空占位变成真正可用的三级删除：只删 dozer 关联/含 agent 缓存/含项目文件本身，统一走系统回收站（`trash::delete`），dozerd 侧登记先删、失败即中止，文件系统步骤尽力而为。

**Architecture:** 新增两个 dozerd 请求（`RemoveProject`/`DeleteProjectTranscripts`），`project.rs` 新增一个 `delete` 子模块登记 `DeleteScope` 三选一状态机 + 执行编排。"确认删除"这一步需要关掉当前项目 tab（跨 `Workspace` 的操作，单个 extension 的 `update` 够不到），沿用代码里已有的"由内核拦截处理"惯例（同 `LinkContextMenu` 的先例）在 `app.rs` 里单独接住。

**Tech Stack:** Rust（`dozer-core`/`dozerd`/`dozer-client`/`dozer-app`），rusqlite，`trash` crate（已有依赖）。

**Spec:** `docs/superpowers/specs/2026-08-22-project-delete-tiers-design.md`

## Global Constraints

- 三个层级统一用 `trash::delete`（移入系统回收站，可找回），不引入 `remove_dir_all` 永久删除。
- 不要求输入项目名二次确认——普通取消/确认弹窗即可，同 `ssh.rs`/`files.rs` 现有删除功能标准。
- dozerd 两步（`RemoveProject`、层级 ≥2 时的 `DeleteProjectTranscripts`）先跑，任一失败整体中止、不碰文件系统；文件系统步骤各自独立、尽力而为。
- 不改动 `project_scaffold.rs` 的 ensure 逻辑，两者只共享"项目根目录路径"这个输入。

## 重要修正（相对 spec 的实现细节澄清）

spec 里 `delete_project_transcripts` 草稿假设 `conversations.dir` 列存的是项目 `cwd` 原始字符串——**这是错的**。实际上 `dir` 存的是每个 agent 各自的 transcript 存储目录（如 `~/.claude/projects/<encoded>/`），由 `agent_paths::{claude,codebuddy,opencode}_project_dir_in(home, cwd)` 算出来，`list_conversations_in`（`mod.rs:327-369`）就是这么过滤的。本计划的 Task 3 按这个更正后的口径实现：对三个 agent 各自算出的目录字符串分别执行 `DELETE`，不是对着裸 `cwd` 删一次。

## File Structure

- `crates/dozer-core/src/protocol.rs` — 新增两个 `Request` 变体 + 一个 `Reply` 变体（`RemoveProject` 复用已有的裸 `Reply::Ok`）。
- `crates/dozerd/src/projects.rs` — `ProjectStore::remove`。
- `crates/dozerd/src/transcripts/mod.rs` — `TranscriptStore::delete_project_transcripts`/`_in`。
- `crates/dozerd/src/server.rs` — 两个新请求的路由。
- `crates/dozer-client/src/lib.rs` — `remove_project`/`delete_project_transcripts` 包装方法。
- `crates/dozer-app/src/extensions/project/delete.rs`（新文件，`project/` 子模块，同 `links.rs` 的既有组织方式）— `DeleteScope` 枚举 + `spawn_delete_project` 执行编排（dozerd 两步 fail-fast + 文件系统三步尽力而为）。
- `crates/dozer-app/src/extensions/project.rs` — `delete` 子模块登记、`WorkspaceState.delete_pending` 字段、`Message` 新变体、确认弹窗 UI、`view` 里的 stack 叠加。
- `crates/dozer-app/src/app.rs` — 新增顶层 `Message::ProjectDeleteDone(Vec<String>)`、拦截 `Message::Project(project::Message::DeleteProjectConfirm)`、`App::project_delete_confirm` 方法。

---

## Task 1: `dozer-core::protocol` — 新增删除相关请求/应答

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`（`Request`/`Reply` 枚举，紧邻 `RenameProject`/`Project`）

**Interfaces:**
- Produces: `Request::RemoveProject { id: i64 }`（应答复用已有的裸 `Reply::Ok`，`protocol.rs:377`）；`Request::DeleteProjectTranscripts { cwd: String }` → `Reply::DeletedTranscripts { conversations: u32 }`。Task 4（`dozerd::server`）、Task 5（`dozer-client`）消费这些。

- [ ] **Step 1: 加两个 `Request` 变体 + 一个 `Reply` 变体**

在 `crates/dozer-core/src/protocol.rs` 的 `Request` 枚举里，紧邻 `RenameProject { id: i64, name: String }` 之后加：

```rust
    /// 从 dozerd 登记里彻底移除这个项目(层级 `DozerOnly` 起都会发)。
    /// `id` 不存在 → `Reply::Error`。成功 → 裸 `Reply::Ok`。
    RemoveProject {
        id: i64,
    },
    /// 删掉这个项目在三家 agent 存储目录下已摄取的 conversations/
    /// conversation_turns 数据(层级 `WithAgentCache` 起才发)。`cwd` 跟
    /// `OpenProject.path` 同一种形状——原始项目根目录绝对路径,不是
    /// agent 存储目录本身(那个由 dozerd 内部用 `agent_paths` 算)。
    DeleteProjectTranscripts {
        cwd: String,
    },
```

在 `Reply` 枚举里，紧邻 `BackfillDone { imported_files: u32 }` 之后加：

```rust
    /// `DeleteProjectTranscripts` 应答:这次实际删掉的 conversations 行数
    /// (三家 agent 加总)。0 不代表出错——可能这个项目本来就没被摄取过。
    DeletedTranscripts {
        conversations: u32,
    },
```

- [ ] **Step 2: 写失败的 roundtrip 单测**

紧邻 `crates/dozer-core/src/protocol.rs` 里已有的 `backfill_project_transcripts_protocol_types_roundtrip` 测试之后新增：

```rust
#[test]
fn remove_project_protocol_types_roundtrip() {
    let req = Request::RemoveProject { id: 7 };
    let line = encode_line(&req);
    let back: Request = decode_line(&line).unwrap();
    assert_eq!(req, back);
}

#[test]
fn delete_project_transcripts_protocol_types_roundtrip() {
    let req = Request::DeleteProjectTranscripts {
        cwd: "/home/x/proj".into(),
    };
    let line = encode_line(&req);
    let back: Request = decode_line(&line).unwrap();
    assert_eq!(req, back);

    let reply = Reply::DeletedTranscripts { conversations: 5 };
    let line = encode_line(&reply);
    let back: Reply = decode_line(&line).unwrap();
    assert_eq!(reply, back);
}
```

- [ ] **Step 3: 跑测试确认失败再确认通过**

Run: `cargo test -p dozer-core remove_project_protocol_types_roundtrip delete_project_transcripts_protocol_types_roundtrip`
Expected: 加变体前 FAIL；加完 Step 1 后重跑 PASS。

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -p dozer-core && cargo clippy -p dozer-core --all-targets`
Expected: 无警告。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): 新增 RemoveProject/DeleteProjectTranscripts 请求/应答"
```

---

## Task 2: `dozerd::projects` — `ProjectStore::remove`

**Files:**
- Modify: `crates/dozerd/src/projects.rs`

**Interfaces:**
- Produces: `pub fn remove(&self, id: i64) -> Result<()>`。Task 4 消费。

- [ ] **Step 1: 写失败的单测**

在 `crates/dozerd/src/projects.rs` 测试模块，紧邻已有的 `rename_missing_id_errors` 之后新增：

```rust
#[test]
fn remove_deletes_project_and_it_no_longer_lists() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
    let p = store.open("/proj/a").unwrap();

    store.remove(p.id).unwrap();

    let remaining = store.list().unwrap();
    assert!(remaining.iter().all(|r| r.id != p.id));
}

#[test]
fn remove_missing_id_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
    assert!(store.remove(999).is_err());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd remove_deletes_project_and_it_no_longer_lists remove_missing_id_errors`
Expected: FAIL（`remove` 未定义）。

- [ ] **Step 3: 实现 `remove`**

在 `crates/dozerd/src/projects.rs` 的 `impl ProjectStore` 块里，紧邻 `rename`（`projects.rs:161-176`）之后新增：

```rust
    pub fn remove(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let affected = conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        if affected == 0 {
            anyhow::bail!("项目 id={id} 不存在");
        }
        Ok(())
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd remove_deletes_project_and_it_no_longer_lists remove_missing_id_errors`
Expected: PASS。

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/projects.rs
git commit -m "feat(dozerd): ProjectStore::remove 删除项目登记"
```

---

## Task 3: `dozerd::transcripts::mod` — 按项目删除 agent 缓存数据

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`（`impl TranscriptStore` 块）

**Interfaces:**
- Consumes: `dozer_core::agent_paths::{claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in, home_dir}`（已有）。
- Produces: `pub fn delete_project_transcripts(&self, cwd: &str) -> Result<u32>`（生产入口，内部用真实 `home_dir()`）、`pub fn delete_project_transcripts_in(&self, home: &Path, cwd: &str) -> Result<u32>`（测试用，显式传 `home`——同 `list_conversations_in`/`get_conversation_turns` 那批函数的既有 `_in` 命名约定）。Task 4 消费 `delete_project_transcripts`。

- [ ] **Step 1: 写失败的单测**

在 `crates/dozerd/src/transcripts/mod.rs` 测试模块，紧邻 `backfill_project_ingests_only_that_projects_transcripts` 之后新增：

```rust
#[test]
fn delete_project_transcripts_only_removes_target_projects_rows() {
    let home = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let store = TranscriptStore::open(&db_dir.path().join("t.db")).unwrap();

    let cwd_a = "/proj/a";
    let cwd_b = "/proj/b";
    let claude_dir_a =
        dozer_core::agent_paths::claude_project_dir_in(home.path(), std::path::Path::new(cwd_a));
    let claude_dir_b =
        dozer_core::agent_paths::claude_project_dir_in(home.path(), std::path::Path::new(cwd_b));
    std::fs::create_dir_all(&claude_dir_a).unwrap();
    std::fs::create_dir_all(&claude_dir_b).unwrap();
    std::fs::write(
        claude_dir_a.join("s1.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"项目A\"}}\n",
    )
    .unwrap();
    std::fs::write(
        claude_dir_b.join("s2.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"项目B\"}}\n",
    )
    .unwrap();
    let files_a = super::scan::discover_project_transcript_files_in(
        home.path(),
        std::path::Path::new(cwd_a),
    );
    let files_b = super::scan::discover_project_transcript_files_in(
        home.path(),
        std::path::Path::new(cwd_b),
    );
    crate::backfill::ingest_files(&store, files_a);
    crate::backfill::ingest_files(&store, files_b);
    assert_eq!(store.get_conversation_turns("s1", -1, 10).unwrap().len(), 1);
    assert_eq!(store.get_conversation_turns("s2", -1, 10).unwrap().len(), 1);

    let removed = store
        .delete_project_transcripts_in(home.path(), cwd_a)
        .unwrap();
    assert_eq!(removed, 1);
    assert!(store.get_conversation_turns("s1", -1, 10).unwrap().is_empty());
    // 项目 B 的数据完好。
    assert_eq!(store.get_conversation_turns("s2", -1, 10).unwrap().len(), 1);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd delete_project_transcripts_only_removes_target_projects_rows`
Expected: FAIL（`delete_project_transcripts_in` 未定义）。

- [ ] **Step 3: 实现两个方法**

在 `crates/dozerd/src/transcripts/mod.rs` 的 `impl TranscriptStore` 块里，紧邻 `backfill_project`（`mod.rs:298-306`）之后新增：

```rust
    /// 按项目删除已摄取的 agent 历史数据(层级 `WithAgentCache` 起触发,
    /// `Request::DeleteProjectTranscripts`,见 `server.rs`)。`conversations`/
    /// `conversation_turns` 的 `dir` 列存的是每家 agent 各自的 transcript
    /// 存储目录(`agent_paths::*_project_dir_in` 算出来的),不是裸 `cwd`
    /// ——按 `list_conversations_in`(同一个过滤口径)的做法,对三家 agent
    /// 各自的目录字符串分别删,不是对着 `cwd` 删一次。返回三家加总的
    /// `conversations` 行删除数。
    pub fn delete_project_transcripts(&self, cwd: &str) -> Result<u32> {
        self.delete_project_transcripts_in(&dozer_core::agent_paths::home_dir(), cwd)
    }

    /// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
    pub fn delete_project_transcripts_in(&self, home: &Path, cwd: &str) -> Result<u32> {
        let cwd_path = Path::new(cwd);
        let dirs = [
            dozer_core::agent_paths::claude_project_dir_in(home, cwd_path),
            dozer_core::agent_paths::codebuddy_project_dir_in(home, cwd_path),
            dozer_core::agent_paths::opencode_project_dir_in(home, cwd_path),
        ];
        let conn = self.conn.lock().expect("db lock");
        let mut total = 0u32;
        for dir in dirs {
            let dir_s = dir.to_string_lossy().into_owned();
            conn.execute(
                "DELETE FROM conversation_turns WHERE conversation_id IN \
                 (SELECT conversation_id FROM conversations WHERE dir = ?1)",
                [&dir_s],
            )?;
            let affected = conn.execute("DELETE FROM conversations WHERE dir = ?1", [&dir_s])?;
            total += affected as u32;
        }
        Ok(total)
    }
```

（如果这个文件顶部还没有 `use std::path::Path;`,确认已有——`get_conversation_turns`/`backfill_project` 等既有方法已经用了 `&Path`/`std::path::Path::new`,同一个 `use` 应该已经在文件顶部。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd delete_project_transcripts_only_removes_target_projects_rows`
Expected: PASS。

- [ ] **Step 5: 跑全量 dozerd 测试**

Run: `cargo test -p dozerd`
Expected: 全部 PASS。

- [ ] **Step 6: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): TranscriptStore::delete_project_transcripts 按项目删 agent 历史"
```

---

## Task 4: `dozerd::server` — 接入两个新请求

**Files:**
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: `projects.remove(id: i64) -> Result<()>`（Task 2）；`transcripts.delete_project_transcripts(cwd: &str) -> Result<u32>`（Task 3）。
- Produces: 无(叶子)。

- [ ] **Step 1: 加两个 `match` 分支**

在 `crates/dozerd/src/server.rs` 里，紧邻 `Request::RenameProject { .. } => { ... }` 分支之后加：

```rust
                        Request::RemoveProject { id } => match projects.remove(id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error { message: format!("删除项目失败: {e}") },
                        },
```

在紧邻 `Request::BackfillProjectTranscripts { .. } => { ... }` 分支之后加：

```rust
                        Request::DeleteProjectTranscripts { cwd } => {
                            match transcripts.delete_project_transcripts(&cwd) {
                                Ok(conversations) => Reply::DeletedTranscripts { conversations },
                                Err(e) => Reply::Error { message: format!("删除 agent 历史失败: {e}") },
                            }
                        }
```

- [ ] **Step 2: 全量编译确认 `match` 穷尽**

Run: `cargo build -p dozerd 2>&1 | tail -40`
Expected: 编译成功。

- [ ] **Step 3: 跑全量 dozerd 测试**

Run: `cargo test -p dozerd`
Expected: 全部 PASS（这个改动本身没有独立单测——纯转发，行为已被 Task 2/3 的单测 + Task 1 的协议 roundtrip 测试覆盖，同 `BackfillProjectTranscripts` 那次的测试密度）。

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd/src/server.rs
git commit -m "feat(dozerd): server 接入 RemoveProject/DeleteProjectTranscripts 请求路由"
```

---

## Task 5: `dozer-client` — 包装方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes: `Request::RemoveProject`/`Request::DeleteProjectTranscripts`/`Reply::Ok`/`Reply::DeletedTranscripts`（Task 1）。
- Produces: `pub async fn remove_project(&self, id: i64) -> Result<()>`、`pub async fn delete_project_transcripts(&self, cwd: &str) -> Result<u32>`。Task 6 消费。

- [ ] **Step 1: 加两个包装方法**

在 `crates/dozer-client/src/lib.rs` 的 `impl Client` 块里，紧邻 `backfill_project_transcripts` 之后新增：

```rust
    pub async fn remove_project(&self, id: i64) -> Result<()> {
        match self.roundtrip(&Request::RemoveProject { id }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_project_transcripts(&self, cwd: &str) -> Result<u32> {
        match self
            .roundtrip(&Request::DeleteProjectTranscripts { cwd: cwd.into() })
            .await?
        {
            Reply::DeletedTranscripts { conversations } => Ok(conversations),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 2: 全量编译确认通过**

Run: `cargo build -p dozer-client`
Expected: 编译成功（纯 RPC 转发，同 `backfill_project_transcripts` 一样没有独立单测）。

- [ ] **Step 3: fmt + clippy**

Run: `cargo fmt -p dozer-client && cargo clippy -p dozer-client --all-targets`
Expected: 无警告。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): 新增 remove_project/delete_project_transcripts 包装方法"
```

---

## Task 6: `dozer-app::extensions::project::delete`（新文件）— `DeleteScope` + 执行编排

**Files:**
- Create: `crates/dozer-app/src/extensions/project/delete.rs`

**Interfaces:**
- Consumes: `client.remove_project(id) -> Result<()>`/`client.delete_project_transcripts(cwd) -> Result<u32>`（Task 5）。
- Produces: `pub enum DeleteScope { DozerOnly, WithAgentCache, WithProjectFiles }`（`Debug, Clone, Copy, PartialEq`）；`pub fn spawn_delete_project(project_id: i64, repo_path: PathBuf, scope: DeleteScope, client: dozer_client::Client, handle: &tokio::runtime::Handle, on_done: impl Fn(Vec<String>) + Send + 'static)`。Task 8（`app.rs`）消费两者。

这个文件的核心逻辑(dozerd 网络调用 + `trash::delete` 真实文件系统操作)没法干净地单元测试——同 `files.rs` 现有删除功能的现状(只测到 `DeleteRequest`→`DeleteCancel`，从没测过真正调 `trash::delete` 那条路径)。这个 Task 只写一个编译期确认 + 类型定义,真正的执行路径靠 Task 8 结束时的人工 GUI 走查覆盖。

- [ ] **Step 1: 创建文件，定义 `DeleteScope` + `spawn_delete_project`**

```rust
//! "删除项目"三级方案的执行编排(spec 2026-08-22)。dozerd 侧两步
//! (`RemoveProject`/`DeleteProjectTranscripts`)先跑、失败即中止、不碰
//! 文件系统；文件系统三步(`.dozer`/agent 缓存目录/项目文件本身)在其后
//! 尽力而为，统一用 `trash::delete`(可从系统回收站找回)。

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeleteScope {
    /// 只删 dozer 登记 + `.dozer/` 缓存。
    DozerOnly,
    /// 含 `DozerOnly`，再删三家 agent 为这个项目缓存的历史数据。
    WithAgentCache,
    /// 含 `WithAgentCache`，再删项目文件本身(含 `.git`)。
    WithProjectFiles,
}

/// 跑一次完整的删除流程。`on_done` 收到的 `Vec<String>` 是文件系统步骤
/// 里各自独立失败的原因(空 = 全部成功)；dozerd 两步任一失败时，`on_done`
/// 只收到那一条错误、后续步骤(含文件系统步骤)都不会跑。调用方
/// (`app.rs::App::project_delete_confirm`)负责在调这个函数**之前**先把
/// 这个项目的 tab 关掉——这个函数本身不碰任何 `Workspace`/UI 状态，
/// 只认 `project_id`(用于 dozerd 请求，虽然当前两个请求都不需要
/// `project_id`，只需要 `cwd`——保留这个参数是为了跟调用方的日志/未来
/// 扩展对齐，不是死代码；如果实现阶段发现完全用不上，可以去掉)。
pub fn spawn_delete_project(
    _project_id: i64,
    repo_path: PathBuf,
    scope: DeleteScope,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    on_done: impl Fn(Vec<String>) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        if let Err(e) = client.remove_project(_project_id).await {
            on_done(vec![format!("取消项目登记失败: {e}")]);
            return;
        }
        if scope != DeleteScope::DozerOnly {
            if let Err(e) = client.delete_project_transcripts(&cwd).await {
                on_done(vec![format!("删除 agent 历史失败: {e}")]);
                return;
            }
        }
        let repo_path_fs = repo_path.clone();
        let errors = tokio::task::spawn_blocking(move || {
            let mut errors = Vec::new();
            let dozer_dir = repo_path_fs.join(".dozer");
            if dozer_dir.exists()
                && let Err(e) = trash::delete(&dozer_dir)
            {
                errors.push(format!(".dozer 目录: {e}"));
            }
            if scope != DeleteScope::DozerOnly {
                let home = dozer_core::agent_paths::home_dir();
                let agent_dirs = [
                    (
                        "Claude 缓存",
                        dozer_core::agent_paths::claude_project_dir_in(&home, &repo_path_fs),
                    ),
                    (
                        "CodeBuddy 缓存",
                        dozer_core::agent_paths::codebuddy_project_dir_in(&home, &repo_path_fs),
                    ),
                    (
                        "OpenCode 缓存",
                        dozer_core::agent_paths::opencode_project_dir_in(&home, &repo_path_fs),
                    ),
                ];
                for (label, dir) in agent_dirs {
                    if dir.exists()
                        && let Err(e) = trash::delete(&dir)
                    {
                        errors.push(format!("{label}: {e}"));
                    }
                }
            }
            if scope == DeleteScope::WithProjectFiles
                && let Err(e) = trash::delete(&repo_path_fs)
            {
                errors.push(format!("项目文件: {e}"));
            }
            errors
        })
        .await
        .unwrap_or_else(|e| vec![format!("内部错误: {e}")]);
        on_done(errors);
    });
}
```

- [ ] **Step 2: 在 `project.rs` 注册子模块**

`crates/dozer-app/src/extensions/project.rs` 顶部，紧邻 `mod links;`（或等价的既有 `mod` 声明）之后加：

```rust
pub mod delete;
```

（如果 `project.rs` 里 `links` 模块不是用 `mod links;` 声明、而是别的写法，按文件里实际写法照抄同样的可见性/声明方式，只是模块名换成 `delete`。）

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译成功（这一步 `delete.rs` 还没被任何地方调用，只要能编译通过、没有 `unused` 警告导致的 deny-level 错误即可；`dead_code` 警告本身在这个中间状态是预期的，Task 7/8 接上调用后会消失）。

- [ ] **Step 4: fmt + clippy（只看这个新文件自己）**

Run: `cargo fmt -p dozer-app -- crates/dozer-app/src/extensions/project/delete.rs`

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/project/delete.rs crates/dozer-app/src/extensions/project.rs
git commit -m "feat(dozer-app): 新增 project::delete 子模块,DeleteScope 三级删除编排"
```

---

## Task 7: `dozer-app::extensions::project` — 三选一确认弹窗 + 状态机

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`（`Message` 枚举、`WorkspaceState`、`update()`、新增弹窗渲染函数、`view()`）

**Interfaces:**
- Consumes: `delete::DeleteScope`（Task 6）。
- Produces: `WorkspaceState.delete_pending: Option<delete::DeleteScope>`（`pub(crate)`）；`Message::DeleteProjectRequest`/`DeleteProjectScopeSelect(DeleteScope)`/`DeleteProjectCancel`/`DeleteProjectConfirm` 四个新变体（旧的 `Message::DeleteProject` 变体删除，被 `DeleteProjectRequest` 取代）。Task 8 消费 `delete_pending` 字段读取方式和 `DeleteProjectConfirm` 变体(拦截用)。

- [ ] **Step 1: 写失败的单测（状态机的非-Confirm 部分：Request/ScopeSelect/Cancel）**

在 `crates/dozer-app/src/extensions/project.rs` 测试模块，紧邻 `scaffold_done_stores_report_only_when_visible` 之后新增：

```rust
#[test]
fn delete_project_request_then_scope_select_then_cancel() {
    let mut ws_state = WorkspaceState::new(None, links::LinksState::default());
    let noop_client = dozer_client::Client::new(std::path::PathBuf::from("/tmp/dozer.sock"));
    let handle = tokio::runtime::Handle::try_current()
        .unwrap_or_else(|_| tokio::runtime::Runtime::new().unwrap().handle().clone());
    let repo = std::path::Path::new("/tmp/demo");

    update(
        &mut ws_state,
        Message::DeleteProjectRequest,
        1,
        "demo",
        repo,
        &noop_client,
        &handle,
        |_| {},
    );
    assert_eq!(ws_state.delete_pending, Some(delete::DeleteScope::DozerOnly));

    update(
        &mut ws_state,
        Message::DeleteProjectScopeSelect(delete::DeleteScope::WithProjectFiles),
        1,
        "demo",
        repo,
        &noop_client,
        &handle,
        |_| {},
    );
    assert_eq!(
        ws_state.delete_pending,
        Some(delete::DeleteScope::WithProjectFiles)
    );

    update(
        &mut ws_state,
        Message::DeleteProjectCancel,
        1,
        "demo",
        repo,
        &noop_client,
        &handle,
        |_| {},
    );
    assert_eq!(ws_state.delete_pending, None);
}
```

（`DeleteProjectConfirm` 不在这个测试里覆盖——它在 `update()` 里是 `unreachable!()`，由 `app.rs` 拦截处理，见 Task 8。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app delete_project_request_then_scope_select_then_cancel`
Expected: FAIL（新 `Message` 变体/`delete_pending` 字段均未定义）。

- [ ] **Step 3: `WorkspaceState` 加字段，`Message` 换变体**

`crates/dozer-app/src/extensions/project.rs`，`WorkspaceState` 结构体末尾（`scaffold_report` 字段之后）加：

```rust
    pub(crate) delete_pending: Option<delete::DeleteScope>,
```

`Message` 枚举里，把：

```rust
    /// footer-bar「删除项目」按钮(UI 占位,逻辑后续接入)。
    DeleteProject,
```

改成：

```rust
    /// footer-bar「删除项目」按钮:打开三选一确认弹窗,默认选中最轻层级。
    DeleteProjectRequest,
    /// 弹窗内切换单选层级。
    DeleteProjectScopeSelect(delete::DeleteScope),
    /// 弹窗"取消"。
    DeleteProjectCancel,
    /// 弹窗"确认删除"。**由 `app.rs` 拦截处理**(需要关掉当前项目 tab，
    /// 单个 extension 的 `update` 够不到跨 `Workspace` 的操作，同
    /// `LinkContextMenu` 的既有先例)——`update()` 里这个分支是
    /// `unreachable!()`。
    DeleteProjectConfirm,
```

在文件顶部 `use` 块加一行：

```rust
use crate::extensions::project::delete;
```

- [ ] **Step 4: `update()` 里接入四个新分支**

`crates/dozer-app/src/extensions/project.rs`，把：

```rust
        // 删除项目:独立设计,不在本次范围内(见另一份 spec)。
        Message::DeleteProject => {}
```

改成：

```rust
        Message::DeleteProjectRequest => {
            ws_state.delete_pending = Some(delete::DeleteScope::DozerOnly);
        }
        Message::DeleteProjectScopeSelect(scope) => {
            ws_state.delete_pending = Some(scope);
        }
        Message::DeleteProjectCancel => {
            ws_state.delete_pending = None;
        }
        Message::DeleteProjectConfirm => {
            unreachable!("由内核拦截处理,见 App::project_delete_confirm 文档")
        }
```

- [ ] **Step 5: 确认弹窗渲染函数**

在 `crates/dozer-app/src/extensions/project.rs` 文件末尾（`project_footer_bar` 函数之后任意位置）新增：

```rust
/// 「删除项目」三选一确认弹窗,视觉模板同 `ssh.rs::delete_confirm_popup`
/// (卡片 + 取消/确认按钮),单选行复用 `ssh.rs` 已有的 `radio_dot` 视觉
/// (选中态 GOLD 实心描边,未选中态空心 BORDER 描边)。
fn project_delete_confirm_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let selected = ws_state
        .delete_pending
        .unwrap_or(delete::DeleteScope::DozerOnly);

    let radio_row = |scope: delete::DeleteScope, label: &'static str| {
        crate::extensions::ssh::radio_dot(
            selected == scope,
            label,
            Message::DeleteProjectScopeSelect(scope),
        )
    };

    let cancel = button(
        text("取消")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::DeleteProjectCancel)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });
    let confirm = button(
        text("删除")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::DeleteProjectConfirm)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().red,
        border: Border {
            color: byteui::theme::color::current().red,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });

    let dialog = container(
        column![
            text("删除项目")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().cream),
            text("选择删除范围,操作会把对应内容移入系统回收站(可找回)。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            column![
                radio_row(delete::DeleteScope::DozerOnly, "只删 dozer 关联与缓存文件"),
                radio_row(delete::DeleteScope::WithAgentCache, "以上 + 所有 agent 缓存数据"),
                radio_row(
                    delete::DeleteScope::WithProjectFiles,
                    "以上 + 项目文件与版本仓库"
                ),
            ]
            .spacing(8),
            row![cancel, confirm].spacing(8),
        ]
        .spacing(12),
    )
    .padding(16)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(byteui::theme::color::current().card.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..iced_widget::container::Style::default()
    });

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
```

（`crate::extensions::ssh::radio_dot` 目前是不是 `pub`/参数顺序是不是 `(selected: bool, label: &str, on_select: Message)` 需要在这一步之前打开 `ssh.rs:861` 确认——如果签名不完全一致或者是私有的，按 `ssh.rs` 里的真实签名调整这里的调用，必要时把 `radio_dot` 改成 `pub(crate)` 供 `project.rs` 复用；这不影响其它代码。）

- [ ] **Step 6: `view()` 里叠加弹窗**

`crates/dozer-app/src/extensions/project.rs`，把 `view` 函数末尾：

```rust
    let body = column![content, project_footer_bar(ws_state)].spacing(0);

    container(body)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().panel.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}
```

改成：

```rust
    let body = column![content, project_footer_bar(ws_state)].spacing(0);

    let base = container(body)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().panel.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        );

    if ws_state.delete_pending.is_some() {
        let dismiss = MouseArea::new(
            container(column![])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(byteui::theme::color::current().scrim.into()),
                    ..iced_widget::container::Style::default()
                }),
        )
        .on_press(Message::DeleteProjectCancel);
        return iced_widget::stack![base, dismiss, project_delete_confirm_popup(ws_state)]
            .width(width)
            .height(Length::Fill)
            .into();
    }

    base.into()
}
```

（`byteui::theme::color::current().scrim` 是不是这个确切字段名，在 `ssh.rs::view` 里已经用过同样的遮罩样式——照抄那边的字段名，不确定就打开 `ssh.rs` 找 `delete_confirm_popup` 调用处的 `dismiss` 构造代码核对。`iced_widget::stack!` 宏需要文件顶部 `use iced_widget::stack;` 或者用完全限定路径 `iced_widget::stack!`——按 `project.rs` 现有的 `iced_widget::` 引用风格决定用哪种写法。）

- [ ] **Step 7: 跑测试确认通过**

Run: `cargo test -p dozer-app delete_project_request_then_scope_select_then_cancel`
Expected: PASS。

Run: `cargo test -p dozer-app extensions::project`
Expected: 全部 PASS（含既有的 link/scaffold 相关测试）。

- [ ] **Step 8: fmt（全量编译要等 Task 8 结束）**

Run: `cargo fmt -p dozer-app -- crates/dozer-app/src/extensions/project.rs`

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs
git commit -m "feat(dozer-app): 删除项目按钮接入三选一确认弹窗状态机"
```

---

## Task 8: `dozer-app::app` — 拦截确认、关闭 tab、执行删除

**Files:**
- Modify: `crates/dozer-app/src/app.rs`（顶层 `Message` 枚举、`Message::Project` 相关的 match 分支、新增 `project_delete_confirm` 方法）

**Interfaces:**
- Consumes: `project::delete::spawn_delete_project`（Task 6）；`project::Message::DeleteProjectConfirm`（Task 7）。
- Produces: 无（叶子，全流程最后一步）。

- [ ] **Step 1: 顶层 `Message` 枚举加完成消息**

`crates/dozer-app/src/app.rs`，紧邻 `ConversationTurnGroupOpen(PathBuf, AgentKind, i64, i64)`（或任意顶层 `Message` 变体聚集处）加：

```rust
    /// 一次"删除项目"执行完成。`Vec<String>` 是文件系统步骤各自独立的
    /// 失败原因(空 = 全部成功);dozerd 侧两步(登记/agent 历史)任一失败
    /// 时这里只会收到那一条错误。项目对应的 tab 在发起删除时已经关掉,
    /// 这个消息到达时已经没有面板可以展示状态,统一走 `self.daemon_error`
    /// (同 `project_tab_opened` 失败路径的既有做法)。
    ProjectDeleteDone(Vec<String>),
```

- [ ] **Step 2: 拦截 `DeleteProjectConfirm`，实现 `project_delete_confirm`**

`crates/dozer-app/src/app.rs`，在 `Message::Project(project::Message::LinkContextMenu { target, index }) => { ... }` 这类特定拦截分支旁边（紧邻，在通用的 `Message::Project(msg) => { ... project::update(...) }` 分支**之前**）加：

```rust
            Message::Project(project::Message::DeleteProjectConfirm) => {
                self.project_delete_confirm();
            }
```

在 `impl App` 块里，紧邻 `project_tab_close`（`app.rs:4911`）附近新增：

```rust
    /// "删除项目"确认弹窗的"删除"按钮触发,由 `Message::Project(project::
    /// Message::DeleteProjectConfirm)` 拦截调用(见该分支注释)。这个操作
    /// 一定作用在当前聚焦的项目上——删除按钮本来就在那个项目自己的面板
    /// 里,不存在"删除一个没打开的项目"这回事。
    fn project_delete_confirm(&mut self) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(scope) = ws.project_panel.delete_pending.take() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = PathBuf::from(&project.path);
        // 关 tab 必须在发起删除请求之前——删除一旦成功,这个项目在
        // dozerd/磁盘上都可能已经不存在了,`Workspace` 不该继续留着。
        self.project_tab_close(project_id);
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let on_done = move |errors: Vec<String>| {
            let _ = proxy.send_event(Message::ProjectDeleteDone(errors));
        };
        project::delete::spawn_delete_project(
            project_id, repo_path, scope, client, &handle, on_done,
        );
    }
```

（`ws.project_panel` 是不是 `project::WorkspaceState` 那个字段在 `Workspace` 结构体里的确切名字——`Message::Project(msg) => {...}` 通用分支里 `project::update(&mut ws.project_panel, msg, ...)` 已经这么用了，照抄同一个字段名。）

- [ ] **Step 3: 处理 `ProjectDeleteDone`**

在顶层 `Message::ReviewLoaded`/`Message::ConversationTurnGroupsRefreshed` 那批处理分支旁边加：

```rust
            Message::ProjectDeleteDone(errors) => {
                if !errors.is_empty() {
                    self.daemon_error = Some(format!("删除项目未完全成功: {}", errors.join("; ")));
                }
            }
```

- [ ] **Step 4: 全工作区编译**

Run: `cargo build 2>&1 | tail -100`
Expected: 编译成功。

- [ ] **Step 5: 全工作区测试**

Run: `cargo test 2>&1 | tail -100`
Expected: 全部 PASS。

- [ ] **Step 6: fmt + clippy 全工作区**

Run: `cargo fmt && cargo clippy --all-targets 2>&1 | tail -100`
Expected: 无警告。

- [ ] **Step 7: 人工 GUI 走查**

Run: `cargo run -p dozer-app`

- 打开一个测试用的项目文件夹（不是真实工作项目，避免误删）→ 点"删除项目" → 确认弹窗出现，默认选中"只删 dozer 关联与缓存文件"。
- 选"只删 dozer 关联" → 确认 → 项目 tab 关闭，项目列表里这个项目消失；确认 `.dozer/` 文件夹被移入系统回收站（而不是还在原地）；项目文件夹本身、`.git`（如果有）都还在。
- 重新打开同一个文件夹当新项目（走一遍新建流程）→ 这次选"含 agent 缓存" → 确认 → "会话"面板确认没有历史对话了（数据已删）；agent 各自的缓存目录（如 `~/.claude/projects/<encoded>/`）里对应这个项目的 `.jsonl` 也被移入回收站。
- 再来一次，这次选"含项目文件" → 确认 → 项目文件夹本身（连同 `.git`）从原位置消失（在系统回收站里能找到）。
- 确认弹窗点"取消"不触发任何删除，`delete_pending` 清空，弹窗关闭。

这一步没有自动化断言，走查通过后记录在 commit message 里，走查不通过就回头改前面步骤。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 删除项目接入 tab 关闭 + 三级执行编排

人工 GUI 走查通过:三个层级各自按预期把对应内容移入系统回收站,
dozerd 登记/agent 历史数据/项目文件三块范围递增正确,取消按钮不
触发任何删除,当前项目 tab 在删除前正确关闭。"
```

---

## Self-Review（写完后已完成的检查）

**Spec 覆盖**：三个层级的定义与递增关系 → Task 6 的 `DeleteScope`；dozerd 侧新增（`RemoveProject`/`DeleteProjectTranscripts`/`ProjectStore::remove`/`TranscriptStore` 删除方法）→ Task 1-5；文件系统三步 `trash::delete` 尽力而为 → Task 6 的 `spawn_delete_project`；触发流程（Request/ScopeSelect/Cancel/Confirm 四个消息 + 关 tab 时机）→ Task 7（前三个）+ Task 8（Confirm 拦截 + 关 tab + 执行）；确认弹窗视觉模板复用 `ssh.rs` → Task 7 Step 5；错误处理（dozerd 两步 fail-fast、文件系统步骤尽力而为、"项目已不存在"落进同一个 fail-fast 分支）→ Task 6 的 `spawn_delete_project` 实现；测试覆盖清单五条 → 分别对应 Task 2/3/1/7 四个单测 + Task 8 的人工走查（`trash::delete` 不单测的理由在 Task 6 开头单独说明，同 spec"测试"一节的既有取舍）。**Task 6 里"按项目重新校正 `dir` 语义"的修正**在计划开头单独成一节，跟 spec 原文的差异已经标注清楚，不是隐藏偏离。

**占位符扫描**：无 TBD。Task 7 Step 5/6 里"`radio_dot` 签名需要确认"/"`scrim` 字段名需要核对"是诚实的实现阶段确认点（研究阶段没有抓到 `radio_dot` 的精确参数顺序），核心改动（弹窗结构、stack 叠加逻辑）已经给了完整代码，不是逃避实现。Task 8 的人工走查同 `project-scaffold-repair`/`review-trace` 两份计划里涉及真实文件系统/dozerd 交互的收尾任务同款处理方式，不是新先例。

**类型一致性**：`DeleteScope`（Task 6 定义：`DozerOnly`/`WithAgentCache`/`WithProjectFiles`）在 Task 7（`Message::DeleteProjectScopeSelect(DeleteScope)`、`WorkspaceState.delete_pending`）、Task 8（`spawn_delete_project` 调用）里用法一致；`spawn_delete_project` 的签名（`project_id, repo_path, scope, client, handle, on_done`）在 Task 6 定义、Task 8 调用处完全一致；`remove_project`/`delete_project_transcripts`/`RemoveProject`/`DeleteProjectTranscripts`/`DeletedTranscripts` 五层命名在 Task 1/2/3/4/5 里保持对应关系不漂移。
