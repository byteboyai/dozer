# 项目脚手架：打开即 ensure + "修复项目"落地 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 项目面板"修复项目"按钮从空占位变成真正的 README/`.dozer`/git 仓库/agent 历史数据四步 ensure，并且这四步在每次项目被"打开成新 tab"时也静默跑一遍——两条触发路径共用同一套幂等步骤，只是要不要把结果展示成状态文字不同。

**Architecture:** 三个同步、可能阻塞的步骤（`.dozer` 目录/README/git init）打包进一次 `tokio::task::spawn_blocking`（复用 `files.rs` 现有 `GitInit` 处理的手法），agent 历史数据导入是新增的 dozerd 请求 `BackfillProjectTranscripts`（复用已有的 `TranscriptStore::ingest_session` 增量摄取和 `backfill.rs` 的批量循环，只是把"扫全部项目"收窄成"扫一个项目的三个 agent 目录"）。步骤登记成一个可扩展的 `Vec<ScaffoldStep>`，这次只填 README/`.dozer`/git 三项，其它 extension 以后按需加。

**Tech Stack:** Rust（`dozer-core`/`dozerd`/`dozer-client`/`dozer-app`），tokio（`spawn_blocking`），rusqlite。

**Spec:** `docs/superpowers/specs/2026-08-22-project-scaffold-repair-design.md`

## Global Constraints

- 不改 `todo`/`ssh`/`database`/`links` 等其它 extension——它们目前"文件缺失=空状态"已经正确，这次只登记 README/`.dozer`/git 三个同步步骤 + agent 历史一个异步步骤。
- 不做"删除项目"功能——独立设计，不在本计划范围。
- 触发范围严格限定在"一个项目变成新打开的 tab"（`app.rs::project_tab_opened` 里 `focus_project_tab(...)` 返回 `false` 的分支），不包括在已打开的 tab 间切换。
- 步骤失败不能互相阻塞：一步失败，其它步骤照常跑完，结果各自独立记录。
- 不新建 toast/通知系统：修复结果用项目面板内联状态文字展示。

---

## Task 1: `dozer-core::protocol` — 新增 `BackfillProjectTranscripts` 请求/应答

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`（`Request`/`Reply` 两个枚举，紧邻 `OpenProject`/`ListProjects`）
- Modify: `crates/dozer-core/src/protocol.rs`（`conversation_protocol_types_roundtrip` 测试，或新增一个独立的 roundtrip 测试）

**Interfaces:**
- Produces: `Request::BackfillProjectTranscripts { cwd: String }`、`Reply::BackfillDone { imported_files: u32 }`（两者都走既有的 `#[serde(tag = "type", rename_all = "snake_case")]` 外层枚举属性，不单独加 derive）。Task 5（`dozerd::server`）、Task 6（`dozer-client`）消费这两个变体。

- [ ] **Step 1: 加两个新枚举变体**

在 `crates/dozer-core/src/protocol.rs` 的 `Request` 枚举里，紧邻 `OpenProject { path: String }`（约 302-304 行）之后加：

```rust
    /// 补录一个项目目录下、dozerd 还没摄取过的 agent transcript 历史
    /// (以及追平任何已摄取文件里新增的尾部内容)。`cwd` 是项目根目录的
    /// 绝对路径字符串，跟 `OpenProject.path` 同一种形状。
    BackfillProjectTranscripts {
        cwd: String,
    },
```

在 `Reply` 枚举里，紧邻 `Project { project: Option<ProjectInfo> }` 之后加：

```rust
    /// `BackfillProjectTranscripts` 应答:这次实际导入(全新摄取)的文件数。
    /// 0 表示这个项目的三家 agent 目录里没有还没摄取过的文件——不代表
    /// 出错。
    BackfillDone {
        imported_files: u32,
    },
```

- [ ] **Step 2: 写失败的 roundtrip 单测**

紧邻 `crates/dozer-core/src/protocol.rs` 里已有的 `conversation_protocol_types_roundtrip` 测试之后新增一个独立测试：

```rust
#[test]
fn backfill_project_transcripts_protocol_types_roundtrip() {
    let req = Request::BackfillProjectTranscripts {
        cwd: "/home/x/proj".into(),
    };
    let line = encode_line(&req);
    let back: Request = decode_line(&line).unwrap();
    assert_eq!(req, back);

    let reply = Reply::BackfillDone { imported_files: 3 };
    let line = encode_line(&reply);
    let back: Reply = decode_line(&line).unwrap();
    assert_eq!(reply, back);
}
```

- [ ] **Step 3: 跑测试确认失败再确认通过**

Run: `cargo test -p dozer-core backfill_project_transcripts_protocol_types_roundtrip`
Expected: 加变体前 FAIL（`BackfillProjectTranscripts`/`BackfillDone` 未定义）；加完 Step 1 后重跑 PASS。

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -p dozer-core && cargo clippy -p dozer-core --all-targets`
Expected: 无警告。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): 新增 BackfillProjectTranscripts 请求/应答"
```

---

## Task 2: `dozerd::transcripts::scan` — 按项目收窄的文件发现

**Files:**
- Modify: `crates/dozerd/src/transcripts/scan.rs`

**Interfaces:**
- Consumes: `dozer_core::agent_paths::{claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in}`（已有，`agent_paths.rs:34,42,52`，签名均为 `(home: &Path, cwd: &Path) -> PathBuf`）。
- Produces: `pub fn discover_project_transcript_files(cwd: &Path) -> Vec<(AgentKind, PathBuf)>`、`pub fn discover_project_transcript_files_in(home: &Path, cwd: &Path) -> Vec<(AgentKind, PathBuf)>`。Task 4 消费 `discover_project_transcript_files_in`（可测）/`discover_project_transcript_files`（生产入口）。

- [ ] **Step 1: 写失败的单测**

在 `crates/dozerd/src/transcripts/scan.rs` 测试模块（`mod tests`，紧邻已有的 `discovers_jsonl_files_across_three_agent_roots`）新增：

```rust
#[test]
fn discover_project_transcript_files_scans_only_that_projects_agent_dirs() {
    let home = tempfile::tempdir().unwrap();
    let cwd = std::path::Path::new("/proj/a");
    let claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
    let codebuddy_dir = dozer_core::agent_paths::codebuddy_project_dir_in(home.path(), cwd);
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::create_dir_all(&codebuddy_dir).unwrap();
    std::fs::write(claude_dir.join("s1.jsonl"), "{}").unwrap();
    std::fs::write(codebuddy_dir.join("s2.jsonl"), "{}").unwrap();
    // 别的项目的目录不该被扫进来。
    let other_cwd = std::path::Path::new("/proj/b");
    let other_claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), other_cwd);
    std::fs::create_dir_all(&other_claude_dir).unwrap();
    std::fs::write(other_claude_dir.join("s3.jsonl"), "{}").unwrap();

    let mut found = discover_project_transcript_files_in(home.path(), cwd);
    found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].0, AgentKind::Claude);
    assert_eq!(found[1].0, AgentKind::Codebuddy);
}

#[test]
fn discover_project_transcript_files_empty_when_no_agent_dirs_exist() {
    let home = tempfile::tempdir().unwrap();
    let cwd = std::path::Path::new("/proj/never-opened");
    assert!(discover_project_transcript_files_in(home.path(), cwd).is_empty());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd discover_project_transcript_files`
Expected: FAIL（函数未定义）。

- [ ] **Step 3: `jsonl_files_in` 改 `pub(crate)`，新增两个发现函数**

`crates/dozerd/src/transcripts/scan.rs:16`，把：

```rust
fn jsonl_files_in(dir: &std::path::Path) -> Vec<PathBuf> {
```

改成：

```rust
pub(crate) fn jsonl_files_in(dir: &std::path::Path) -> Vec<PathBuf> {
```

在文件顶部 `use` 块（`scan.rs:6-8`）加一行：

```rust
use dozer_core::agent_paths::{
    claude_project_dir_in, codebuddy_project_dir_in, home_dir, opencode_project_dir_in,
};
```

（替换掉原来只 `use dozer_core::agent_paths::home_dir;` 那一行。）

在 `discover_all_transcript_files_in` 函数之后（`scan.rs:49` 之后）新增：

```rust
/// 按项目收窄的文件发现:只看这一个项目在三家 agent 各自存储目录下的
/// `.jsonl` 文件,跟 `discover_all_transcript_files_in`(扫全部项目)相反
/// 方向——这个是"已知 cwd,只要这一个项目的"(见 `agent_paths` 模块头
/// 注释里"已知 cwd → 算出该项目的存储目录"这条)。
pub fn discover_project_transcript_files(cwd: &std::path::Path) -> Vec<(AgentKind, PathBuf)> {
    discover_project_transcript_files_in(&home_dir(), cwd)
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn discover_project_transcript_files_in(
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> Vec<(AgentKind, PathBuf)> {
    let mut out = Vec::new();
    for (agent, dir) in [
        (AgentKind::Claude, claude_project_dir_in(home, cwd)),
        (AgentKind::Codebuddy, codebuddy_project_dir_in(home, cwd)),
        (AgentKind::Opencode, opencode_project_dir_in(home, cwd)),
    ] {
        for f in jsonl_files_in(&dir) {
            out.push((agent, f));
        }
    }
    out
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd discover_project_transcript_files`
Expected: 两个新测试 PASS，且 `discovers_jsonl_files_across_three_agent_roots`/`missing_agent_root_yields_no_entries_for_it` 两个既有测试仍然 PASS。

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/transcripts/scan.rs
git commit -m "feat(dozerd): 新增按项目收窄的 agent transcript 文件发现"
```

---

## Task 3: `dozerd::backfill` — 抽出可复用的批量摄取循环

**Files:**
- Modify: `crates/dozerd/src/backfill.rs`

**Interfaces:**
- Consumes: `TranscriptStore::ingest_session(&self, agent: AgentKind, file_path: &Path) -> anyhow::Result<()>`（已有，`crates/dozerd/src/transcripts/mod.rs:117`）。
- Produces: `pub fn ingest_files(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) -> u32`（返回成功摄取的文件数）。Task 4 消费这个函数。`backfill_all` 签名不变，内部改成调用它。

- [ ] **Step 1: 写失败的单测**

在 `crates/dozerd/src/backfill.rs` 测试模块新增（紧邻已有两个测试）：

```rust
#[test]
fn ingest_files_returns_count_of_successfully_ingested_files() {
    let tmp = tempfile::tempdir().unwrap();
    let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
    let f1 = tmp.path().join("a.jsonl");
    std::fs::write(&f1, "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一\"}}\n").unwrap();
    let missing = tmp.path().join("does-not-exist.jsonl");

    let n = ingest_files(&store, vec![(AgentKind::Claude, f1), (AgentKind::Claude, missing)]);
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd ingest_files_returns_count_of_successfully_ingested_files`
Expected: FAIL（`ingest_files` 未定义）。

- [ ] **Step 3: 抽出 `ingest_files`，`backfill_all` 改用它**

`crates/dozerd/src/backfill.rs`，把：

```rust
pub fn backfill_all(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) {
    for (agent, path) in files {
        if let Err(e) = store.ingest_session(agent, &path) {
            tracing::warn!(error = %e, path = %path.display(), "启动回填摄取失败,跳过该文件");
        }
    }
}
```

改成：

```rust
/// 对一批 `(agent, 文件路径)` 逐个 `ingest_session`,单个文件失败只记警告
/// 跳过,不中断其它文件——`backfill_all`(daemon 启动全量回填)和
/// `TranscriptStore::backfill_project`(单项目按需回填,Task 4)共用这同一个
/// 循环,只是喂给它的 `files` 来源不同(前者 `discover_all_transcript_files`,
/// 后者 `discover_project_transcript_files`)。返回成功摄取的文件数。
pub fn ingest_files(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) -> u32 {
    let mut imported = 0u32;
    for (agent, path) in files {
        match store.ingest_session(agent, &path) {
            Ok(()) => imported += 1,
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "回填摄取失败,跳过该文件");
            }
        }
    }
    imported
}

pub fn backfill_all(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) {
    ingest_files(store, files);
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd backfill`
Expected: 新测试 `ingest_files_returns_count_of_successfully_ingested_files` PASS，既有的 `backfill_all_ingests_every_discovered_file`/`backfill_all_skips_unreadable_file_without_panicking` 仍然 PASS。

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/backfill.rs
git commit -m "refactor(dozerd): backfill 抽出 ingest_files,返回摄取计数供单项目回填复用"
```

---

## Task 4: `dozerd::transcripts::mod` — `TranscriptStore::backfill_project`

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`（`impl TranscriptStore` 块，紧邻 `get_conversation_turns` 或文件末尾任意方法之后）

**Interfaces:**
- Consumes: `scan::discover_project_transcript_files(cwd: &Path) -> Vec<(AgentKind, PathBuf)>`（Task 2 产出）；`crate::backfill::ingest_files(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) -> u32`（Task 3 产出）。
- Produces: `pub fn backfill_project(&self, cwd: &str) -> u32`。Task 5（`dozerd::server`）消费。

- [ ] **Step 1: 写失败的单测**

在 `crates/dozerd/src/transcripts/mod.rs` 测试模块新增（紧邻 `get_conversation_turns_surfaces_thinking_text_and_tool_calls`，如果那个还没落地就紧邻 `get_conversation_turns_paginates_by_keyset`）：

```rust
#[test]
fn backfill_project_ingests_only_that_projects_transcripts() {
    let home = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let store = TranscriptStore::open(&db_dir.path().join("t.db")).unwrap();
    let cwd = "/proj/a";
    let claude_dir =
        dozer_core::agent_paths::claude_project_dir_in(home.path(), std::path::Path::new(cwd));
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("s1.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
    )
    .unwrap();

    // `backfill_project` 内部用真实 `home_dir()`,这个测试没法直接控制
    // `HOME` 环境变量、也不该在单测里改全局状态——直接调用
    // `scan::discover_project_transcript_files_in` + `backfill::ingest_files`
    // 这条底层组合验证"两个函数拼起来行为对不对",`backfill_project`
    // 本身只是这两行的固定拼接,不单独测(实现阶段如果签名变化,这个测试
    // 需要跟着挪)。
    let files = super::scan::discover_project_transcript_files_in(
        home.path(),
        std::path::Path::new(cwd),
    );
    let n = crate::backfill::ingest_files(&store, files);
    assert_eq!(n, 1);
    assert_eq!(store.get_conversation_turns("s1", -1, 10).unwrap().len(), 1);
}
```

- [ ] **Step 2: 跑测试确认通过（不需要新代码——这一步先确认底层两个函数拼起来行为正确）**

Run: `cargo test -p dozerd backfill_project_ingests_only_that_projects_transcripts`
Expected: PASS（Task 2/3 已经落地，这个测试只是验证组合，不依赖 `backfill_project` 本身存在）。

- [ ] **Step 3: 加 `backfill_project` 方法（生产入口，供 Task 5 调用）**

在 `crates/dozerd/src/transcripts/mod.rs` 的 `impl TranscriptStore` 块内新增（紧邻 `get_conversation_turns` 之后）：

```rust
    /// 按需回填单个项目的 agent transcript 历史(区别于 `crate::backfill::
    /// backfill_all` 的"daemon 启动全量回填")——`OpenProject` 之外的独立
    /// 请求(`Request::BackfillProjectTranscripts`,见 `server.rs`),供"新建
    /// 项目"/"修复项目"按需触发。返回本次实际摄取的文件数,0 不代表出错
    /// (可能这个项目此前已经全部摄取过)。
    pub fn backfill_project(&self, cwd: &str) -> u32 {
        let files =
            super::scan::discover_project_transcript_files(std::path::Path::new(cwd));
        crate::backfill::ingest_files(self, files)
    }
```

- [ ] **Step 4: 跑全量 dozerd 测试**

Run: `cargo test -p dozerd`
Expected: 全部 PASS。

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): TranscriptStore::backfill_project 按需回填单项目历史"
```

---

## Task 5: `dozerd::server` — 接入 `BackfillProjectTranscripts` 请求处理

**Files:**
- Modify: `crates/dozerd/src/server.rs`（`Request::OpenProject`/`Request::GetConversationTurns` 所在的大 `match` 块）

**Interfaces:**
- Consumes: `transcripts.backfill_project(cwd: &str) -> u32`（Task 4 产出，`transcripts` 是该 `match` 作用域内已有的 `TranscriptStore` 变量，同 `Request::GetConversationTurns` 分支复用的那个）。
- Produces: 无(叶子——请求处理，没有下游任务消费它，Task 8 通过 `dozer-client` 间接调用)。

- [ ] **Step 1: 加新的 `match` 分支**

在 `crates/dozerd/src/server.rs` 里，紧邻 `Request::GetConversationTurns { .. } => { ... }` 分支（约 321-326 行）之后加：

```rust
                        Request::BackfillProjectTranscripts { cwd } => {
                            let imported_files = transcripts.backfill_project(&cwd);
                            Reply::BackfillDone { imported_files }
                        }
```

- [ ] **Step 2: 全量编译确认 `match` 穷尽**

Run: `cargo build -p dozerd 2>&1 | tail -40`
Expected: 编译成功（如果报"non-exhaustive match"，说明分支加错了位置或者遗漏了别的地方也 match 了 `Request`——按报错定位补齐）。

- [ ] **Step 3: 跑全量 dozerd 测试**

Run: `cargo test -p dozerd`
Expected: 全部 PASS（这个改动本身没有独立单测——请求路由是纯粹的转发，行为已经被 Task 4 的 `backfill_project` 单测和 Task 1 的协议 roundtrip 测试覆盖，`server.rs` 里其它 `Request` 分支也是这个测试密度）。

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd/src/server.rs
git commit -m "feat(dozerd): server 接入 BackfillProjectTranscripts 请求路由"
```

---

## Task 6: `dozer-client` — `backfill_project_transcripts` 包装方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`（`impl Client` 块，紧邻 `get_conversation_turns`）

**Interfaces:**
- Consumes: `Request::BackfillProjectTranscripts`/`Reply::BackfillDone`（Task 1 产出）。
- Produces: `pub async fn backfill_project_transcripts(&self, cwd: &str) -> Result<u32>`。Task 8（`dozer-app::extensions::project`）消费。

- [ ] **Step 1: 加包装方法**

在 `crates/dozer-client/src/lib.rs` 的 `impl Client` 块里，紧邻 `get_conversation_turns` 之后新增：

```rust
    pub async fn backfill_project_transcripts(&self, cwd: &str) -> Result<u32> {
        match self
            .roundtrip(&Request::BackfillProjectTranscripts { cwd: cwd.into() })
            .await?
        {
            Reply::BackfillDone { imported_files } => Ok(imported_files),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 2: 全量编译确认通过**

Run: `cargo build -p dozer-client`
Expected: 编译成功。这个方法是纯 RPC 转发，跟 `get_conversation_turns` 等既有方法一样没有独立单测（对应的端到端行为由 Task 1 的协议测试 + Task 4/5 的 dozerd 测试覆盖）。

- [ ] **Step 3: fmt + clippy**

Run: `cargo fmt -p dozer-client && cargo clippy -p dozer-client --all-targets`
Expected: 无警告。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): 新增 backfill_project_transcripts 包装方法"
```

---

## Task 7: `dozer-app::project_scaffold`（新文件）— ensure 步骤机制

**Files:**
- Create: `crates/dozer-app/src/project_scaffold.rs`
- Modify: `crates/dozer-app/src/main.rs:5`（注册新模块）

**Interfaces:**
- Consumes: `crate::extensions::project::links::discover_docs(repo: &Path) -> Vec<links::LinkEntry>`（已有，`project/links.rs:83`）；`crate::delivery::init_repo(repo: &Path) -> Result<(), String>`（已有，`delivery.rs:460`）。
- Produces: `pub enum ScaffoldStepResult { AlreadyOk, Created(String), Failed(String) }`（`Debug, Clone, PartialEq`）；`pub struct ScaffoldStep { pub label: &'static str, pub run: fn(&Path) -> ScaffoldStepResult }`；`pub fn scaffold_steps() -> Vec<ScaffoldStep>`；`pub fn run_sync_steps(repo: &Path) -> Vec<(String, ScaffoldStepResult)>`；`pub struct ScaffoldReport { pub steps: Vec<(String, ScaffoldStepResult)> }`（`Debug, Clone, PartialEq`）；`pub fn format_scaffold_report(report: &ScaffoldReport) -> String`。Task 8 消费全部这些。

- [ ] **Step 1: 写失败的单测（覆盖三个同步步骤的幂等判据 + 格式化函数）**

创建 `crates/dozer-app/src/project_scaffold.rs`，先写测试模块：

```rust
//! 项目 ensure/repair 的可扩展步骤机制(spec 2026-08-22)。README/`.dozer`
//! 目录/git 仓库三个同步步骤登记在这里；agent 历史数据导入是异步的
//! dozerd 请求，不在这个纯函数列表里，由调用方(`extensions::project`)
//! 单独跑完再拼进同一份 `ScaffoldReport`。

use crate::extensions::project::links;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum ScaffoldStepResult {
    AlreadyOk,
    Created(String),
    Failed(String),
}

pub struct ScaffoldStep {
    pub label: &'static str,
    pub run: fn(&Path) -> ScaffoldStepResult,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldReport {
    pub steps: Vec<(String, ScaffoldStepResult)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_dozer_dir_creates_when_missing_and_reports_already_ok_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            ensure_dozer_dir(tmp.path()),
            ScaffoldStepResult::Created("已创建 .dozer 目录".into())
        );
        assert!(tmp.path().join(".dozer").is_dir());
        assert_eq!(ensure_dozer_dir(tmp.path()), ScaffoldStepResult::AlreadyOk);
    }

    #[test]
    fn ensure_readme_creates_when_no_doc_files_present() {
        let tmp = tempfile::tempdir().unwrap();
        let result = ensure_readme(tmp.path());
        assert!(matches!(result, ScaffoldStepResult::Created(_)));
        assert!(tmp.path().join("README.md").is_file());
        let content = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
        assert!(content.starts_with("# "));
    }

    #[test]
    fn ensure_readme_skips_when_readme_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "既有内容").unwrap();
        assert_eq!(ensure_readme(tmp.path()), ScaffoldStepResult::AlreadyOk);
        // 不覆盖既有内容。
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("README.md")).unwrap(),
            "既有内容"
        );
    }

    #[test]
    fn ensure_readme_skips_when_other_doc_prefix_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("CHANGELOG.md"), "x").unwrap();
        assert_eq!(ensure_readme(tmp.path()), ScaffoldStepResult::AlreadyOk);
        assert!(!tmp.path().join("README.md").exists());
    }

    #[test]
    fn ensure_git_repo_skips_when_dot_git_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        assert_eq!(ensure_git_repo(tmp.path()), ScaffoldStepResult::AlreadyOk);
    }

    #[test]
    fn run_sync_steps_covers_all_registered_steps_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let results = run_sync_steps(tmp.path());
        let labels: Vec<&str> = results.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["缓存目录", "README", "git 仓库"]);
    }

    #[test]
    fn format_scaffold_report_lists_created_already_ok_and_failed_separately() {
        let report = ScaffoldReport {
            steps: vec![
                ("git 仓库".into(), ScaffoldStepResult::Created("已初始化".into())),
                ("缓存目录".into(), ScaffoldStepResult::AlreadyOk),
                (
                    "agent 历史".into(),
                    ScaffoldStepResult::Failed("daemon 断开".into()),
                ),
            ],
        };
        let text = format_scaffold_report(&report);
        assert!(text.contains("已修复:git 仓库"));
        assert!(text.contains("已是最新:缓存目录"));
        assert!(text.contains("失败:agent 历史(daemon 断开)"));
    }

    #[test]
    fn format_scaffold_report_all_already_ok_shows_single_summary() {
        let report = ScaffoldReport {
            steps: vec![
                ("缓存目录".into(), ScaffoldStepResult::AlreadyOk),
                ("README".into(), ScaffoldStepResult::AlreadyOk),
            ],
        };
        assert_eq!(format_scaffold_report(&report), "一切正常");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app project_scaffold`
Expected: FAIL（`ensure_dozer_dir`/`ensure_readme`/`ensure_git_repo`/`run_sync_steps`/`format_scaffold_report`/`scaffold_steps` 均未定义；这一步先确认模块能被 `main.rs` 找到——如果编译报"找不到 project_scaffold 模块"，先完成 Step 4 的模块注册再回来看测试输出）。

- [ ] **Step 3: 实现三个同步步骤 + `scaffold_steps`/`run_sync_steps`/`format_scaffold_report`**

在 `crates/dozer-app/src/project_scaffold.rs` 里，测试模块**之前**新增：

```rust
fn ensure_dozer_dir(repo: &Path) -> ScaffoldStepResult {
    let dir = repo.join(".dozer");
    if dir.is_dir() {
        return ScaffoldStepResult::AlreadyOk;
    }
    match std::fs::create_dir_all(&dir) {
        Ok(()) => ScaffoldStepResult::Created("已创建 .dozer 目录".into()),
        Err(e) => ScaffoldStepResult::Failed(e.to_string()),
    }
}

/// 判据复用 `links::discover_docs`——根目录里已经有任何"看起来像文档"的
/// 文件(readme/changelog/contributing/license 前缀)就跳过,不额外判断
/// 严格意义上的 `README.md`,避免误判"已经有 CHANGELOG 但没有 README"
/// 这种情况下重复造一份。
fn ensure_readme(repo: &Path) -> ScaffoldStepResult {
    let has_doc_file = links::discover_docs(repo)
        .iter()
        .any(|e| matches!(e.kind, links::LinkKind::File));
    if has_doc_file {
        return ScaffoldStepResult::AlreadyOk;
    }
    let name = repo
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "项目".to_string());
    let content = format!("# {name}\n");
    match std::fs::write(repo.join("README.md"), content) {
        Ok(()) => ScaffoldStepResult::Created("已创建 README.md".into()),
        Err(e) => ScaffoldStepResult::Failed(e.to_string()),
    }
}

fn ensure_git_repo(repo: &Path) -> ScaffoldStepResult {
    if repo.join(".git").exists() {
        return ScaffoldStepResult::AlreadyOk;
    }
    match crate::delivery::init_repo(repo) {
        Ok(()) => ScaffoldStepResult::Created("已初始化 git 仓库".into()),
        Err(e) => ScaffoldStepResult::Failed(e),
    }
}

/// 当前登记的同步步骤。以后其它 extension(todo/ssh/database 等)需要
/// 真正的 ensure/repair 逻辑时,在这里加一行调用自己模块里的函数——不
/// 需要改 `ScaffoldStep`/`ScaffoldStepResult` 这两个类型本身。
pub fn scaffold_steps() -> Vec<ScaffoldStep> {
    vec![
        ScaffoldStep {
            label: "缓存目录",
            run: ensure_dozer_dir,
        },
        ScaffoldStep {
            label: "README",
            run: ensure_readme,
        },
        ScaffoldStep {
            label: "git 仓库",
            run: ensure_git_repo,
        },
    ]
}

/// 顺序跑完全部同步步骤。这几步都可能阻塞(尤其 `ensure_git_repo` 会
/// shell 出子进程),调用方(`extensions::project::spawn_scaffold_run`,
/// Task 8)负责把这个函数整体包进 `tokio::task::spawn_blocking`,这里
/// 本身不做任何异步处理。
pub fn run_sync_steps(repo: &Path) -> Vec<(String, ScaffoldStepResult)> {
    scaffold_steps()
        .into_iter()
        .map(|step| (step.label.to_string(), (step.run)(repo)))
        .collect()
}

/// `ScaffoldReport` → "修复项目"按钮旁边展示的一行状态文字。全部
/// `AlreadyOk` 时给一句"一切正常"的简短总结,否则按 已修复/已是最新/失败
/// 三类分组列出各自涉及的步骤标签,失败项带上错误信息。
pub fn format_scaffold_report(report: &ScaffoldReport) -> String {
    let mut created = Vec::new();
    let mut already_ok = Vec::new();
    let mut failed = Vec::new();
    for (label, result) in &report.steps {
        match result {
            ScaffoldStepResult::Created(_) => created.push(label.as_str()),
            ScaffoldStepResult::AlreadyOk => already_ok.push(label.as_str()),
            ScaffoldStepResult::Failed(msg) => failed.push(format!("{label}({msg})")),
        }
    }
    if created.is_empty() && failed.is_empty() {
        return "一切正常".to_string();
    }
    let mut parts = Vec::new();
    if !created.is_empty() {
        parts.push(format!("已修复:{}", created.join("、")));
    }
    if !already_ok.is_empty() {
        parts.push(format!("已是最新:{}", already_ok.join("、")));
    }
    if !failed.is_empty() {
        parts.push(format!("失败:{}", failed.join("、")));
    }
    parts.join(";")
}
```

- [ ] **Step 4: 在 `main.rs` 注册新模块**

`crates/dozer-app/src/main.rs:5`，把：

```rust
mod delivery;
```

改成：

```rust
mod delivery;
mod project_scaffold;
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app project_scaffold`
Expected: 全部 PASS（7 个测试）。

- [ ] **Step 6: fmt + clippy**

Run: `cargo fmt -p dozer-app -- crates/dozer-app/src/project_scaffold.rs && cargo clippy -p dozer-app --all-targets 2>&1 | tail -60`
Expected: 无警告（clippy 这一步大概率因为 Task 8 还没接完而报其它模块的错——只要 `project_scaffold.rs` 自己没有警告即可，不用等全 crate 干净）。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/project_scaffold.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): 新增 project_scaffold 模块,README/.dozer/git 三步 ensure"
```

---

## Task 8: `dozer-app::extensions::project` — 接入"修复项目"，状态展示

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`（`Message` 枚举、`update()`、`WorkspaceState`、`project_footer_bar`）

**Interfaces:**
- Consumes: `crate::project_scaffold::{run_sync_steps, format_scaffold_report, ScaffoldReport, ScaffoldStepResult}`（Task 7 产出）；`client.backfill_project_transcripts(cwd: &str) -> Result<u32>`（Task 6 产出）。
- Produces: `pub fn spawn_scaffold_run(repo_path: PathBuf, visible: bool, client: dozer_client::Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`。Task 9（`app.rs::project_tab_opened`）消费这个函数。

- [ ] **Step 1: 写失败的单测（`ScaffoldDone` 更新 `WorkspaceState` 的可见性逻辑）**

在 `crates/dozer-app/src/extensions/project.rs` 测试模块新增（找一个已有 `update(...)` 调用的测试作参照，沿用同样的 `WorkspaceState`/`update` 构造方式）：

```rust
#[test]
fn scaffold_done_stores_report_only_when_visible() {
    let mut ws_state = WorkspaceState::new(None, links::LinksState::default());
    let noop_client = dozer_client::Client::new(std::path::PathBuf::from("/tmp/dozer.sock"));
    let handle = tokio::runtime::Handle::try_current()
        .unwrap_or_else(|_| tokio::runtime::Runtime::new().unwrap().handle().clone());
    let report = project_scaffold::ScaffoldReport {
        steps: vec![("README".into(), project_scaffold::ScaffoldStepResult::AlreadyOk)],
    };

    update(
        &mut ws_state,
        Message::ScaffoldDone(report.clone(), false),
        1,
        "demo",
        std::path::Path::new("/tmp/demo"),
        &noop_client,
        &handle,
        |_| {},
    );
    assert_eq!(ws_state.scaffold_report, None);

    update(
        &mut ws_state,
        Message::ScaffoldDone(report.clone(), true),
        1,
        "demo",
        std::path::Path::new("/tmp/demo"),
        &noop_client,
        &handle,
        |_| {},
    );
    assert_eq!(ws_state.scaffold_report, Some(report));
}
```

（`WorkspaceState::new(description, links)` 的确切构造函数签名需要在这一步之前先打开 `project.rs` 确认——如果跟这里假设的不一致，按文件里实际签名调整测试构造部分，不影响后续步骤的实现代码。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app scaffold_done_stores_report_only_when_visible`
Expected: FAIL（`Message::ScaffoldDone`/`WorkspaceState.scaffold_report`/`project_scaffold` 引用均未定义）。

- [ ] **Step 3: `Message` 枚举加 `ScaffoldDone`，`WorkspaceState` 加字段**

`crates/dozer-app/src/extensions/project.rs`，在 `Message` 枚举末尾（`RepairProject`/`DeleteProject` 之后）加：

```rust
    /// 一次 ensure/repair 批跑完成。`visible=true`(修复项目按钮触发)才
    /// 把结果存进 `WorkspaceState.scaffold_report` 供状态文字展示;
    /// `visible=false`(打开项目时静默触发)只是让副作用(README/.dozer/
    /// git/agent 历史)落地,不展示——两条触发路径共用同一个
    /// `spawn_scaffold_run`,靠这个布尔位区分要不要展示。
    ScaffoldDone(project_scaffold::ScaffoldReport, bool),
```

在 `WorkspaceState` 结构体末尾（`error: Option<String>,` 之后）加：

```rust
    pub(crate) scaffold_report: Option<project_scaffold::ScaffoldReport>,
```

在文件顶部 `use` 块加一行：

```rust
use crate::project_scaffold;
```

- [ ] **Step 4: 实现 `RepairProject`/`ScaffoldDone` 处理 + `spawn_scaffold_run`**

`crates/dozer-app/src/extensions/project.rs:344-346`，把：

```rust
        // footer-bar 占位按钮:逻辑后续接入,暂不做任何处理。
        Message::RepairProject => {}
        Message::DeleteProject => {}
    }
}
```

改成：

```rust
        Message::RepairProject => {
            spawn_scaffold_run(
                repo_path.to_path_buf(),
                true,
                client.clone(),
                handle,
                emit,
            );
        }
        // 删除项目:独立设计,不在本次范围内(见另一份 spec)。
        Message::DeleteProject => {}
        Message::ScaffoldDone(report, visible) => {
            if visible {
                ws_state.scaffold_report = Some(report);
            }
        }
    }
}

/// 跑一次完整的 ensure/repair:README/`.dozer`/git 三个同步步骤打包进
/// 一次 `spawn_blocking`(复用 `files.rs::Message::GitInit` 处理已经用过
/// 的手法),再 `await` 一次 agent 历史数据回填,四项结果拼进一份
/// `ScaffoldReport` 发回。`visible` 原样透传给 `Message::ScaffoldDone`,
/// 决定这次结果要不要展示(见该消息的文档注释)。`app.rs::project_tab_opened`
/// (新开 tab,`visible=false`)和这个文件的 `RepairProject` 处理
/// (`visible=true`)都调这个函数,不重复实现两遍。
pub fn spawn_scaffold_run(
    repo_path: std::path::PathBuf,
    visible: bool,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let sync_steps = {
            let repo_path = repo_path.clone();
            tokio::task::spawn_blocking(move || project_scaffold::run_sync_steps(&repo_path))
                .await
                .unwrap_or_else(|e| {
                    vec![(
                        "初始化检查".to_string(),
                        project_scaffold::ScaffoldStepResult::Failed(format!("内部错误: {e}")),
                    )]
                })
        };
        let backfill_result = match client.backfill_project_transcripts(&cwd).await {
            Ok(0) => project_scaffold::ScaffoldStepResult::AlreadyOk,
            Ok(n) => project_scaffold::ScaffoldStepResult::Created(format!("导入 {n} 个历史文件")),
            Err(e) => project_scaffold::ScaffoldStepResult::Failed(e.to_string()),
        };
        let mut steps = sync_steps;
        steps.push(("agent 历史".to_string(), backfill_result));
        emit(Message::ScaffoldDone(
            project_scaffold::ScaffoldReport { steps },
            visible,
        ));
    });
}
```

- [ ] **Step 5: footer-bar 展示状态文字**

`crates/dozer-app/src/extensions/project.rs` 里 `project_footer_bar` 函数（约 677-717 行）当前签名是 `fn project_footer_bar() -> Element<...>`（无参数）。改成接收 `ws_state`，在按钮行下方加一行状态文字：把函数签名：

```rust
fn project_footer_bar() -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

改成：

```rust
fn project_footer_bar(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

在函数末尾、`bar` 变量构造完之后（原来直接 `bar.into()`或类似收尾的地方），改成把状态文字接在 `bar` 下面一起返回：

```rust
    let status = ws_state.scaffold_report.as_ref().map(|report| {
        text(project_scaffold::format_scaffold_report(report))
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim)
    });
    let mut col = column![bar].spacing(4);
    if let Some(status) = status {
        col = col.push(status);
    }
    col.into()
```

（`bar`/其余变量与既有的 padding/样式代码保持不动；这一步只改函数签名 + 收尾那几行，如果实际收尾代码跟这里假设的不完全一样，按文件里的真实结构把"加一行状态文字"这个意图套进去，不改变按钮本身的样式。）

找到调用 `project_footer_bar()` 的地方（`view`/主渲染函数里），改成 `project_footer_bar(ws_state)`（参数名以那个函数里实际持有的 `WorkspaceState` 引用变量名为准）。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p dozer-app scaffold_done_stores_report_only_when_visible`
Expected: PASS。

Run: `cargo test -p dozer-app --lib extensions::project`
Expected: 全部 PASS（含既有的 link 相关测试）。

- [ ] **Step 7: fmt（全量编译要等 Task 9 结束）**

Run: `cargo fmt -p dozer-app -- crates/dozer-app/src/extensions/project.rs`

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs
git commit -m "feat(dozer-app): 修复项目按钮接入 ensure 流程,footer-bar 展示结果状态文字"
```

---

## Task 9: `dozer-app::app` — 打开新 tab 时静默触发 ensure

**Files:**
- Modify: `crates/dozer-app/src/app.rs`（`project_tab_opened` 函数，约 4813-4861 行）

**Interfaces:**
- Consumes: `project::spawn_scaffold_run`（Task 8 产出）。
- Produces: 无（叶子，全流程的最后一步）。

- [ ] **Step 1: 在"新开 tab"分支里插入静默触发**

`crates/dozer-app/src/app.rs`，`project_tab_opened` 函数里，把：

```rust
        let io = self.shell_io();
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        self.sync_terminal_grid();
        self.persist_open_projects();
    }
```

改成：

```rust
        let io = self.shell_io();
        let repo_path = PathBuf::from(&project.path);
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        self.sync_terminal_grid();
        self.persist_open_projects();
        // 新开的项目 tab(区别于"已开着、只是前台化"那条 `focus_project_tab`
        // 早退分支):静默跑一次 ensure(README/.dozer/git/agent 历史),不
        // 展示结果(见 project_scaffold 设计"打开即 ensure"一节)。
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Project(m));
        };
        project::spawn_scaffold_run(repo_path, false, client, &handle, emit);
    }
```

- [ ] **Step 2: 全工作区编译**

Run: `cargo build 2>&1 | tail -100`
Expected: 编译成功。如果报 `project.path` 字段访问错误（比如 `project` 已经被移动走），确认 `repo_path` 的构造语句放在 `ws.adopt_project(&io, project)` **之前**（`project` 是在那一行被 move 进去的）。

- [ ] **Step 3: 全工作区测试**

Run: `cargo test 2>&1 | tail -100`
Expected: 全部 PASS。

- [ ] **Step 4: fmt + clippy 全工作区**

Run: `cargo fmt && cargo clippy --all-targets 2>&1 | tail -100`
Expected: 无警告。

- [ ] **Step 5: 人工 GUI 走查**

Run: `cargo run -p dozer-app`

- 打开一个还没有 README/git 仓库的普通文件夹作为新项目 → 稍等片刻，确认该目录下出现了 `README.md`、`.git/`、`.dozer/`。
- 打开一个已经有这些东西的项目 → 确认不会被覆盖/重复创建。
- 点"修复项目"按钮 → 确认 footer-bar 下方出现状态文字（"一切正常"或"已修复:xxx"）。
- 有真实 Claude/CodeBuddy 历史记录、但之前从没被 dozerd 摄取过的项目（比如先手动删掉对应的 `dozerd` DB 再重新打开）→ 打开后"会话"面板能看到这些历史对话（验证 agent 历史导入确实生效）。

这一步没有自动化断言，走查通过后记录在 commit message 里，走查不通过就回头改前面步骤。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新开项目 tab 时静默触发 README/.dozer/git/agent 历史 ensure

人工 GUI 走查通过:新项目自动补齐 README/git 仓库/.dozer 目录,已有
项目不被覆盖,修复项目按钮展示状态文字,历史 agent 数据能被正确导入。"
```

---

## Self-Review（写完后已完成的检查）

**Spec 覆盖**：`README.md`/`.dozer` 目录/git 仓库三步 → Task 7；agent 历史数据导入 → Task 1-6；可扩展步骤机制（`ScaffoldStep`/`scaffold_steps()`）→ Task 7；触发时机（打开即检查、tab 切换不重复触发）→ Task 9；"修复项目"落地 + 内联状态文字 → Task 8；错误处理"一步失败不阻塞其它步骤" → `spawn_scaffold_run`（Task 8）里三个同步步骤各自独立返回结果、agent 历史失败不影响已经跑完的同步步骤；"已知取舍"三条（其它 extension 不接入/README 最简模板/两条回填路径不互斥）均在对应任务的实现里体现（`scaffold_steps()` 只填三项、`ensure_readme` 只写标题行、`ingest_files` 被两条路径独立调用不加互斥锁）。

**占位符扫描**：无 TBD；Task 8 Step 5 里"如果实际收尾代码跟这里假设的不完全一样，按文件里的真实结构套进去"是因为 `project_footer_bar` 函数体的确切收尾代码没有在研究阶段抓全（只拿到了按钮定义部分），这是一个诚实的实现阶段确认点，不是逃避写代码——核心改动（签名加参数、加状态文字渲染）已经给了完整代码。Task 9 Step 5 的人工走查是该类改动（涉及真实文件系统/git 子进程/dozerd 交互）现状决定的验证方式，同 review-trace 计划里 Task 7 的先例。

**类型一致性**：`ScaffoldStepResult`（Task 7 定义：`AlreadyOk`/`Created(String)`/`Failed(String)`）在 Task 8 的 `spawn_scaffold_run`/测试里用法一致；`ScaffoldReport { steps: Vec<(String, ScaffoldStepResult)> }` 在 Task 7 定义、Task 8 消费时字段名/类型一致；`spawn_scaffold_run` 的签名（`repo_path: PathBuf, visible: bool, client: dozer_client::Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static`）在 Task 8 定义、Task 9 调用时完全一致；`backfill_project_transcripts`/`backfill_project`/`BackfillProjectTranscripts`/`BackfillDone` 四层命名在 Task 1/4/5/6 里保持对应关系不漂移。
