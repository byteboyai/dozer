# 会话总结批量补录(Dozer 侧)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 点击"修复项目"时,对该项目下已摄取对话但缺 `session_summaries` 行的会话,用配置的默认 agent 以 headless 一次性调用生成标题+摘要并落库,并把"修复项目"改造成一个逐步骤实时反馈的弹窗。

**Architecture:** `dozerd` 新增一个 headless 一次性 agent 调用子系统(按 `AgentKind` 分派到不同 CLI 调用方式,统一用分隔符包裹的 JSON 做输出协议)+ 一个内存态补总结进度登记表;`dozer-core::protocol`/`dozer-client` 加两个新的 Request/Reply 及方法;`dozer-app` 把"修复项目"从"一次性最终报告"改造成"逐步骤实时弹窗",新增的补总结步骤靠轮询驱动聚合进度。

**Tech Stack:** Rust workspace,tokio(`tokio::process` 起子进程),rusqlite,iced 0.14(`iced_widget`),serde/serde_json/toml。

**Spec:** `docs/superpowers/specs/2026-08-28-session-summary-backfill-design.md`

**状态(2026-08-28):** Task 1-9 全部执行完并已提交(`02e843b`/`2d2a2cd`/
`50a31b0`/`e8ea412`/`efe8cea`/`143dab9`/`78195b4`)。Task 9 的自动化验证
(build/test/clippy/fmt)全绿;唯一没做的是 Task 9 Step 4 的真机人工点击
验证(需要真实项目+真实 agent CLI,过一遍"修复项目"弹窗的实际交互),
留给用户自己找时间跑。审阅过程中发现过一次真实缺口:Task 7/8(dozer-app
弹窗 UI)最初被漏做,后端(Task 1-6)已就绪但完全不可达,补做后才算真正
完成——细节见 memory `dozer-session-summary-backfill-plan.md`。

## Global Constraints

- GUI 只用 iced 0.14 生态,不引入新的 GUI 框架/组件库。
- 核心(`dozerd`/`dozer-core`/`dozer-client`)不依赖 Node/Python。
- 新 UI 若涉及 icon 按钮/tab 类交互,优先复用 `icons::icon_button_entry`/`tabs::tab_core`——本计划的弹窗是卡片+按钮+文字行,不是这两类,不适用,不强套。
- 每个任务完成后跑:`cargo build`(全 workspace)、涉及 crate 的 `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt -- --check`。
- 提交信息用中文,风格对齐现有 `git log`(`feat(dozerd): ...`/`fix(dozer-app): ...` 前缀)。
- 本计划只覆盖 Dozer 仓库;`v8agent-cli` 侧的 headless 一次性模式是独立仓库的独立计划(`v8agent` 仓库 `docs/superpowers/plans/`),互不阻塞——本计划的 V8agent 适配器在对方就绪前会因为找不到预期行为而走 `HeadlessError` 降级,这是设计内行为,不是 bug。

---

## Task 1: 协议扩展(`dozer-core::protocol`)

**重要:** `dozerd::server::handle_conn` 里的 `match req { ... }` 是穷尽匹配
(没有 `_ =>` 通配分支)。本任务一旦给 `Request` 加新变体,`dozerd` 整个
crate 会立刻编译失败(非穷尽匹配),而 Task 3/4/5 都要跑
`cargo test -p dozerd`——如果不在本任务里顺手给 `server.rs` 打两个占位
分支,Task 3/4/5 会因为同一个 crate 里另一个文件编译不过而全部失败,
即使它们自己的新代码完全正确。所以本任务的 Step 5 会顺带给
`server.rs` 打两个"还没实现"的占位分支,Task 6 再把这两个占位分支换成
真正的实现——这不是范围蔓延,是保证"每个任务跑完 `cargo build`/
`cargo test` 全绿"这条 Global Constraint 在任务粒度上真的成立。

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozerd/src/server.rs`(仅加两个占位 match 分支,不改
  `serve`/`handle_conn` 的函数签名——签名改动是 Task 6 的事)

**Interfaces:**
- Produces: `Request::BackfillSessionSummaries { cwd: String }`、
  `Request::GetSessionSummaryBackfillStatus { cwd: String }`、
  `Reply::BackfillStatus { total: u32, completed: u32 }`——后续所有任务依赖
  这三个类型。

- [x] **Step 1: 写失败的 roundtrip 测试**

在 `protocol.rs` 的 `#[cfg(test)] mod tests` 里追加:

```rust
    #[test]
    fn backfill_session_summaries_request_roundtrips() {
        let req = Request::BackfillSessionSummaries {
            cwd: "/repo/x".into(),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn get_session_summary_backfill_status_roundtrips() {
        let req = Request::GetSessionSummaryBackfillStatus {
            cwd: "/repo/x".into(),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let reply = Reply::BackfillStatus {
            total: 5,
            completed: 2,
        };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-core backfill_session_summaries_request_roundtrips get_session_summary_backfill_status_roundtrips`
Expected: 编译失败(`Request::BackfillSessionSummaries`/`Reply::BackfillStatus` 不存在)。

- [x] **Step 3: 加变体**

在 `pub enum Request` 里,`CloseWithSummary` 变体之后追加:

```rust
    /// 补录一个项目目录下缺失 `session_summaries` 行的历史会话总结(项目
    /// "修复"按钮触发)。立即返回 `Reply::Ok`——实际处理在 dozerd 后台异步
    /// 完成,`total` 在返回 Ok 之前已同步算好并写进内存态进度表,调用方拿到
    /// Ok 后即可放心轮询 `GetSessionSummaryBackfillStatus` 不会撞见"还没算出
    /// total"的空窗期(spec 2026-08-28)。
    BackfillSessionSummaries {
        cwd: String,
    },
    /// 查询某 cwd 下补总结后台任务的进度。找不到对应记录(还没发起过/
    /// dozerd 重启后内存态丢失)时约定回 `total=0, completed=0`,调用方据此
    /// 判定"无进行中任务"。
    GetSessionSummaryBackfillStatus {
        cwd: String,
    },
```

在 `pub enum Reply` 里,`SessionSummary` 变体之后追加:

```rust
    /// `GetSessionSummaryBackfillStatus` 应答。`completed >= total` 表示
    /// 已处理完(`total == 0` 表示这个项目本来就没有缺总结的会话)。
    BackfillStatus {
        total: u32,
        completed: u32,
    },
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-core backfill_session_summaries_request_roundtrips get_session_summary_backfill_status_roundtrips`
Expected: PASS

- [x] **Step 5: 给 `server.rs` 打占位分支,保住 `dozerd` 的可编译性**

在 `crates/dozerd/src/server.rs` 的 `match req { ... }` 里,
`Request::CloseWithSummary { session_id } => { ... }` 分支之后追加:

```rust
                        Request::BackfillSessionSummaries { .. } => {
                            // 占位实现,Task 6 换成真正的后台补总结逻辑。
                            Reply::Error {
                                message: "补总结功能尚未实现".into(),
                            }
                        }
                        Request::GetSessionSummaryBackfillStatus { .. } => {
                            // 占位实现,Task 6 换成真正的进度查询。
                            Reply::BackfillStatus {
                                total: 0,
                                completed: 0,
                            }
                        }
```

- [x] **Step 6: 全量协议测试 + lint**

Run: `cargo build -p dozerd && cargo test -p dozer-core && cargo clippy -p dozer-core -p dozerd --all-targets -- -D warnings && cargo fmt -p dozer-core -p dozerd -- --check`
Expected: 全绿(`cargo build -p dozerd` 这一步就是在验证 Step 5 的占位分支
确实把非穷尽匹配问题堵上了)。

- [x] **Step 7: Commit**

```bash
git add crates/dozer-core/src/protocol.rs crates/dozerd/src/server.rs
git commit -m "$(cat <<'EOF'
feat(dozer-core): add BackfillSessionSummaries / GetSessionSummaryBackfillStatus protocol messages

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `dozer-client` 方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Test: `crates/dozer-client/tests/against_real_daemon.rs`(先只加占位断言,真正的端到端测试留给 Task 6 完成 dozerd 处理器后再补——这里先保证方法能编译、能发出正确的 Request 变体)

**Interfaces:**
- Consumes: `Request::BackfillSessionSummaries`/`Request::GetSessionSummaryBackfillStatus`/`Reply::BackfillStatus`(Task 1)。
- Produces: `Client::backfill_session_summaries(&self, cwd: &str) -> Result<()>`、
  `Client::get_session_summary_backfill_status(&self, cwd: &str) -> Result<(u32, u32)>`(返回
  `(completed, total)`,注意参数顺序与协议里的 `(total, completed)` 字段顺序
  相反——`(completed, total)` 是为了跟 `dozer-app` 侧"已完成/总数"的展示口径
  对齐,调用方在 Task 7 会直接用)。

- [x] **Step 1: 加方法**

在 `crates/dozer-client/src/lib.rs` 的 `close_with_summary` 方法之后追加:

```rust
    /// 触发某 cwd 下缺失总结会话的批量补录(项目"修复"按钮用,spec
    /// 2026-08-28)。立即返回;实际补录在 dozerd 后台完成,进度靠
    /// `get_session_summary_backfill_status` 轮询。
    pub async fn backfill_session_summaries(&self, cwd: &str) -> Result<()> {
        match self
            .roundtrip(&Request::BackfillSessionSummaries { cwd: cwd.into() })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 查询补总结进度,返回 `(completed, total)`。找不到进行中任务时回
    /// `(0, 0)`。
    pub async fn get_session_summary_backfill_status(&self, cwd: &str) -> Result<(u32, u32)> {
        match self
            .roundtrip(&Request::GetSessionSummaryBackfillStatus { cwd: cwd.into() })
            .await?
        {
            Reply::BackfillStatus { total, completed } => Ok((completed, total)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [x] **Step 2: 编译检查**

Run: `cargo build -p dozer-client && cargo test -p dozer-client`
Expected: 全绿(Task 1 已经给 `server.rs` 打好占位分支,`dozerd` 作为
`dozer-client` 的 dev-dependency 能正常编译;这两个新方法目前还没有专门
的往返测试——那个留给 Task 6 Step 6,等 `server.rs` 有真实实现后再测才有
意义,这里先只保证方法本身能编译、既有测试不受影响)。

- [x] **Step 3: lint**

Run: `cargo clippy -p dozer-client --all-targets -- -D warnings && cargo fmt -p dozer-client -- --check`
Expected: 全绿

- [x] **Step 4: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(dozer-client): add backfill_session_summaries / get_session_summary_backfill_status

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 默认 agent 配置(`dozerd::default_agent_config`)

**Files:**
- Create: `crates/dozerd/src/default_agent_config.rs`
- Modify: `crates/dozerd/src/lib.rs`(注册新模块)
- Modify: `crates/dozerd/Cargo.toml`(加 `serde`/`toml` 依赖)

**Interfaces:**
- Produces: `pub fn load_default_agent() -> dozer_core::protocol::AgentKind`、
  `pub fn load_default_agent_from(path: &std::path::Path) -> dozer_core::protocol::AgentKind`
  (测试用,显式传路径,同 `TranscriptStore::list_conversations_in` 的既有
  "生产入口包一层显式路径版本"模式)。

- [x] **Step 1: 加依赖**

在 `crates/dozerd/Cargo.toml` 的 `[dependencies]` 里追加(紧跟 `serde_json.workspace = true` 之后):

```toml
serde.workspace = true
toml = "0.9"
```

- [x] **Step 2: 写失败的测试**

创建 `crates/dozerd/src/default_agent_config.rs`:

```rust
//! "默认 agent"最小配置:本地 TOML 文件,用户手动改,没有 GUI 入口
//! (spec 2026-08-28)。每次补总结请求时惰性读取,不缓存,不需要重启
//! dozerd 生效。

use dozer_core::protocol::AgentKind;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct DefaultAgentConfig {
    default_agent: AgentKind,
}

/// 生产入口,固定用 `dozer_core::paths::config_dir().join("config.toml")`。
pub fn load_default_agent() -> AgentKind {
    load_default_agent_from(&dozer_core::paths::config_dir().join("config.toml"))
}

/// `path` 显式传入版本,测试用。文件不存在、读取失败、内容非法(缺字段/
/// `default_agent` 值不是四家已知 agent 之一)都回落到 `AgentKind::Claude`
/// 并记 `tracing::warn!`——不 panic,不阻塞补总结流程。
pub fn load_default_agent_from(path: &Path) -> AgentKind {
    let fallback = AgentKind::Claude;
    let Ok(text) = std::fs::read_to_string(path) else {
        tracing::warn!(path = %path.display(), "default_agent 配置文件不存在,回退到 Claude");
        return fallback;
    };
    match toml::from_str::<DefaultAgentConfig>(&text) {
        Ok(cfg) => cfg.default_agent,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "default_agent 配置解析失败,回退到 Claude");
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_falls_back_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load_default_agent_from(&path), AgentKind::Claude);
    }

    #[test]
    fn valid_config_returns_configured_agent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_agent = \"opencode\"\n").unwrap();
        assert_eq!(load_default_agent_from(&path), AgentKind::Opencode);
    }

    #[test]
    fn malformed_toml_falls_back_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        assert_eq!(load_default_agent_from(&path), AgentKind::Claude);
    }

    #[test]
    fn unknown_agent_value_falls_back_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // `AgentKind` 反序列化遇到不认识的字符串会报错(不是静默变 Unknown,
        // 因为 `AgentKind` 没有 `#[serde(other)]`),走 malformed 同一条
        // 回退路径。
        std::fs::write(&path, "default_agent = \"chatgpt\"\n").unwrap();
        assert_eq!(load_default_agent_from(&path), AgentKind::Claude);
    }

    #[test]
    fn all_four_supported_agents_parse() {
        for (raw, expected) in [
            ("claude", AgentKind::Claude),
            ("codebuddy", AgentKind::Codebuddy),
            ("opencode", AgentKind::Opencode),
            ("v8agent", AgentKind::V8agent),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, format!("default_agent = \"{raw}\"\n")).unwrap();
            assert_eq!(load_default_agent_from(&path), expected);
        }
    }
}
```

在 `crates/dozerd/src/lib.rs` 里追加一行(按现有字母序插入):

```rust
pub mod default_agent_config;
```

- [x] **Step 3: 跑测试确认通过**

Run: `cargo test -p dozerd default_agent_config`
Expected: PASS(这个模块本身不依赖任何其它未完成的任务,可以独立通过)。

- [x] **Step 4: lint**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -p dozerd -- --check`
Expected: 全绿

- [x] **Step 5: Commit**

```bash
git add crates/dozerd/Cargo.toml crates/dozerd/src/lib.rs crates/dozerd/src/default_agent_config.rs
git commit -m "$(cat <<'EOF'
feat(dozerd): add default_agent config file loader

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: headless 一次性调用——共享解析逻辑 + Claude 适配器

**Files:**
- Create: `crates/dozerd/src/headless_agent.rs`
- Modify: `crates/dozerd/src/lib.rs`(注册新模块)

**Interfaces:**
- Consumes: `dozer_core::protocol::AgentKind`(已有)。
- Produces: `pub enum HeadlessError { Spawn(String), Timeout, NoMarkers, InvalidJson(String), Unsupported }`(`Debug, Clone, PartialEq`)、
  `pub async fn summarize_headless(agent: AgentKind, human_turns: &[String]) -> Result<(String, String), HeadlessError>`
  ——Task 6 直接调这一个函数,不需要知道内部按 agent 分派的细节。

- [x] **Step 1: 写失败的测试(纯函数部分:`extract_summary`)**

创建 `crates/dozerd/src/headless_agent.rs`:

```rust
//! headless 一次性 agent 调用(spec 2026-08-28):不经过 PTY/Session/
//! registry,启动一次 agent CLI 处理单个 prompt 后退出,用固定分隔符包裹
//! 的 JSON 做输出协议——四家 CLI 的私有结构化输出格式互不相同,统一约定
//! "模型最终文本里必须包含这一段"比对齐四种进程输出协议更简单。

use dozer_core::protocol::AgentKind;
use serde::Deserialize;
use std::time::Duration;

const SUMMARY_START_MARKER: &str = "<<<DOZER_SUMMARY_JSON>>>";
const SUMMARY_END_MARKER: &str = "<<<END_DOZER_SUMMARY_JSON>>>";
const HEADLESS_TIMEOUT: Duration = Duration::from_secs(90);
const TITLE_MAX_CHARS: usize = 200;
const SUMMARY_MAX_CHARS: usize = 8000;

#[derive(Debug, Clone, PartialEq)]
pub enum HeadlessError {
    /// 子进程启动失败(二进制不在 PATH 上等)。
    Spawn(String),
    /// 超过 `HEADLESS_TIMEOUT` 仍未退出。
    Timeout,
    /// stdout 里找不到完整的一对分隔符。
    NoMarkers,
    /// 分隔符之间的内容不是合法 JSON,或缺 `title`/`summary` 字段。
    InvalidJson(String),
    /// 该 `AgentKind` 没有对应的 headless 适配器(理论上调用方只会传四家
    /// 已覆盖的 kind,这个分支是防御性的)。
    Unsupported,
}

fn instruction_text() -> String {
    format!(
        "请阅读接下来这段对话记录里用户说过的话,生成一个简短标题和一段摘要,\
         总结这次会话完成的工作。只输出下面这一段,不要输出任何其他内容:\n\
         {SUMMARY_START_MARKER}{{\"title\":\"...\",\"summary\":\"...\"}}{SUMMARY_END_MARKER}"
    )
}

#[derive(Deserialize)]
struct SummaryJson {
    title: String,
    summary: String,
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// 从完整 stdout 里抠出分隔符之间的 JSON 并解析。公开给单测直接调用,不
/// 需要真的起子进程。
pub fn extract_summary(stdout: &str) -> Result<(String, String), HeadlessError> {
    let start = stdout.find(SUMMARY_START_MARKER).ok_or(HeadlessError::NoMarkers)?;
    let after_start = start + SUMMARY_START_MARKER.len();
    let end = stdout[after_start..]
        .find(SUMMARY_END_MARKER)
        .ok_or(HeadlessError::NoMarkers)?;
    let json_str = stdout[after_start..after_start + end].trim();
    let parsed: SummaryJson =
        serde_json::from_str(json_str).map_err(|e| HeadlessError::InvalidJson(e.to_string()))?;
    Ok((
        truncate_chars(&parsed.title, TITLE_MAX_CHARS),
        truncate_chars(&parsed.summary, SUMMARY_MAX_CHARS),
    ))
}

/// 按 agent 构造子进程命令 + (可选)要写进 stdin 的字节。`None` 表示这个
/// `AgentKind` 没有 headless 适配器。
fn build_command(agent: AgentKind, turns_text: &str) -> Option<(tokio::process::Command, Option<Vec<u8>>)> {
    match agent {
        AgentKind::Claude => {
            let mut cmd = tokio::process::Command::new("claude");
            cmd.arg("-p").arg(instruction_text());
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        _ => None, // CodeBuddy/OpenCode/V8agent 在 Task 5 补上
    }
}

/// 跑一个已经构造好的子进程,拿完整 stdout 后解析。跟 `build_command` 分开
/// 是为了这一段能用真实存在的 `sh`/不存在的二进制名做确定性单测,不需要
/// 装 claude/codebuddy/opencode/v8agent 才能测超时/spawn 失败这些分支。
async fn run_and_extract(
    mut cmd: tokio::process::Command,
    stdin_bytes: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<(String, String), HeadlessError> {
    use std::process::Stdio;
    cmd.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| HeadlessError::Spawn(e.to_string()))?;
    if let Some(bytes) = stdin_bytes {
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&bytes).await;
        }
    }
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| HeadlessError::Timeout)?
        .map_err(|e| HeadlessError::Spawn(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    extract_summary(&stdout)
}

/// 对外唯一入口:`human_turns` 是该会话已摄取的人类回合内容(与
/// `session_summary::heuristic_from_turns` 同一数据源),按 `agent` 分派到
/// 对应 CLI 的一次性调用方式,拿到结果或错误(调用方失败时应降级到
/// `heuristic_from_turns`,不是本函数的职责)。
pub async fn summarize_headless(
    agent: AgentKind,
    human_turns: &[String],
) -> Result<(String, String), HeadlessError> {
    let turns_text = human_turns.join("\n");
    let Some((cmd, stdin_bytes)) = build_command(agent, &turns_text) else {
        return Err(HeadlessError::Unsupported);
    };
    run_and_extract(cmd, stdin_bytes, HEADLESS_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_summary_parses_delimited_json() {
        let stdout = format!(
            "some preamble\n{SUMMARY_START_MARKER}{{\"title\":\"标题\",\"summary\":\"摘要\"}}{SUMMARY_END_MARKER}\ntrailing"
        );
        let (title, summary) = extract_summary(&stdout).unwrap();
        assert_eq!(title, "标题");
        assert_eq!(summary, "摘要");
    }

    #[test]
    fn extract_summary_missing_markers_errors() {
        assert_eq!(extract_summary("no markers here"), Err(HeadlessError::NoMarkers));
    }

    #[test]
    fn extract_summary_invalid_json_errors() {
        let stdout = format!("{SUMMARY_START_MARKER}not json{SUMMARY_END_MARKER}");
        assert!(matches!(extract_summary(&stdout), Err(HeadlessError::InvalidJson(_))));
    }

    #[test]
    fn extract_summary_truncates_overlong_title() {
        let long_title = "a".repeat(300);
        let json = format!("{{\"title\":\"{long_title}\",\"summary\":\"s\"}}");
        let stdout = format!("{SUMMARY_START_MARKER}{json}{SUMMARY_END_MARKER}");
        let (title, _) = extract_summary(&stdout).unwrap();
        assert_eq!(title.chars().count(), TITLE_MAX_CHARS + 1); // +1 是省略号
        assert!(title.ends_with('…'));
    }

    #[test]
    fn claude_command_uses_dash_p_and_pipes_turns_via_stdin() {
        let (cmd, stdin) = build_command(AgentKind::Claude, "用户说了什么").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "claude");
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-p");
        assert!(args[1].contains(SUMMARY_START_MARKER));
        assert_eq!(stdin, Some("用户说了什么".as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn run_and_extract_reads_stdout_after_process_exits() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("cat");
        let stdin = Some(
            format!("{SUMMARY_START_MARKER}{{\"title\":\"t\",\"summary\":\"s\"}}{SUMMARY_END_MARKER}")
                .into_bytes(),
        );
        let (title, summary) = run_and_extract(cmd, stdin, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(title, "t");
        assert_eq!(summary, "s");
    }

    #[tokio::test]
    async fn run_and_extract_times_out_on_slow_process() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 5");
        let result = run_and_extract(cmd, None, Duration::from_millis(100)).await;
        assert_eq!(result, Err(HeadlessError::Timeout));
    }

    #[tokio::test]
    async fn run_and_extract_reports_spawn_failure_for_missing_binary() {
        let cmd = tokio::process::Command::new("this-binary-does-not-exist-xyz");
        let result = run_and_extract(cmd, None, Duration::from_secs(5)).await;
        assert!(matches!(result, Err(HeadlessError::Spawn(_))));
    }

    #[tokio::test]
    async fn summarize_headless_returns_unsupported_for_kind_without_adapter() {
        // Codex 目前没有 headless 适配器(Task 5 只补 CodeBuddy/OpenCode/
        // V8agent,Codex 本来就不在覆盖范围内,见 spec 非目标)。
        let result = summarize_headless(AgentKind::Codex, &[]).await;
        assert_eq!(result, Err(HeadlessError::Unsupported));
    }
}
```

在 `crates/dozerd/src/lib.rs` 里追加一行:

```rust
pub mod headless_agent;
```

- [x] **Step 2: 跑测试确认通过**

Run: `cargo test -p dozerd headless_agent`
Expected: PASS(全部 8 个测试)。

- [x] **Step 3: lint**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -p dozerd -- --check`
Expected: 全绿

- [x] **Step 4: Commit**

```bash
git add crates/dozerd/src/lib.rs crates/dozerd/src/headless_agent.rs
git commit -m "$(cat <<'EOF'
feat(dozerd): headless one-shot agent adapter — shared parsing + Claude

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: headless 适配器——CodeBuddy / OpenCode / V8agent

**Files:**
- Modify: `crates/dozerd/src/headless_agent.rs`

**Interfaces:**
- Consumes: `build_command`(Task 4,私有函数,同文件内直接扩展 `match` 分支)。
- Produces: 无新公开接口,`summarize_headless` 现在对 `Claude`/`Codebuddy`/
  `Opencode`/`V8agent` 四家都返回 `Some`。

- [x] **Step 1: 写失败的测试**

在 `headless_agent.rs` 的 `mod tests` 里追加:

```rust
    #[test]
    fn codebuddy_command_includes_dash_y_for_non_interactive_permission() {
        let (cmd, stdin) = build_command(AgentKind::Codebuddy, "内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "codebuddy");
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"-y".to_string()));
        assert_eq!(stdin, Some("内容".as_bytes().to_vec()));
    }

    #[test]
    fn opencode_command_uses_run_subcommand_with_inline_message_no_stdin() {
        let (cmd, stdin) = build_command(AgentKind::Opencode, "用户内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "opencode");
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "run");
        assert!(args[1].contains(SUMMARY_START_MARKER));
        assert!(args[1].contains("用户内容"));
        assert_eq!(stdin, None);
    }

    #[test]
    fn v8agent_command_sets_oneshot_env_and_clears_session_id() {
        let (cmd, stdin) = build_command(AgentKind::V8agent, "用户内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "v8agent");
        let envs: Vec<_> = std_cmd.get_envs().collect();
        assert!(envs
            .iter()
            .any(|(k, v)| *k == "V8AGENT_ONESHOT" && *v == Some(std::ffi::OsStr::new("1"))));
        // DOZER_SESSION_ID 显式清掉,避免 v8agent-cli 误挂载 dozer-mcp
        // (headless 总结走 stdout 解析,不需要 MCP,见 spec)。
        assert!(envs.iter().any(|(k, v)| *k == "DOZER_SESSION_ID" && v.is_none()));
        assert!(stdin.is_some());
    }

    #[test]
    fn unsupported_kinds_return_none() {
        assert!(build_command(AgentKind::Codex, "x").is_none());
        assert!(build_command(AgentKind::Kilo, "x").is_none());
        assert!(build_command(AgentKind::Unknown, "x").is_none());
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd headless_agent`
Expected: FAIL(`codebuddy`/`opencode`/`v8agent` 三个新测试断言不成立,因为
`build_command` 目前对这三家都落进 `_ => None` 分支)。

- [x] **Step 3: 补全 `build_command`**

把 `headless_agent.rs` 里的 `build_command` 函数体替换成:

```rust
fn build_command(agent: AgentKind, turns_text: &str) -> Option<(tokio::process::Command, Option<Vec<u8>>)> {
    match agent {
        AgentKind::Claude => {
            let mut cmd = tokio::process::Command::new("claude");
            cmd.arg("-p").arg(instruction_text());
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        AgentKind::Codebuddy => {
            let mut cmd = tokio::process::Command::new("codebuddy");
            // `-y`/`--dangerously-skip-permissions`:CodeBuddy 非交互模式下
            // 执行任何需要授权的操作(哪怕这里只是让它输出文字)的必需参数,
            // 不加会卡在授权确认上,headless 场景下无人能应答(spec
            // 2026-08-28 调研结论)。
            cmd.arg("-p").arg(instruction_text()).arg("-y");
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        AgentKind::Opencode => {
            let mut cmd = tokio::process::Command::new("opencode");
            // `run` 子命令没有独立 stdin 输入通道,拼接文本直接作为 message
            // 参数的一部分(spec 2026-08-28 调研结论)。
            cmd.arg("run").arg(format!("{}\n\n{}", instruction_text(), turns_text));
            Some((cmd, None))
        }
        AgentKind::V8agent => {
            let mut cmd = tokio::process::Command::new("v8agent");
            cmd.env("V8AGENT_ONESHOT", "1");
            cmd.env_remove("DOZER_SESSION_ID");
            let stdin_text = format!("{}\n\n{}", instruction_text(), turns_text);
            Some((cmd, Some(stdin_text.into_bytes())))
        }
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Kilo => None,
    }
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd headless_agent`
Expected: PASS(全部测试,含 Task 4 遗留的 8 个 + 本任务新增的 4 个)。

- [x] **Step 5: lint**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -p dozerd -- --check`
Expected: 全绿

- [x] **Step 6: Commit**

```bash
git add crates/dozerd/src/headless_agent.rs
git commit -m "$(cat <<'EOF'
feat(dozerd): headless adapters for CodeBuddy / OpenCode / V8agent

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: dozerd 后台补总结任务(进度登记 + 协议处理器)

**Files:**
- Create: `crates/dozerd/src/session_summary_backfill.rs`
- Modify: `crates/dozerd/src/lib.rs`
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: `crate::headless_agent::summarize_headless`(Task 4/5)、
  `crate::default_agent_config::load_default_agent`(Task 3)、
  `crate::session_summary::{SessionSummaryStore, heuristic_from_turns}`(既有)、
  `crate::transcripts::TranscriptStore`(既有)、`Request::BackfillSessionSummaries`/
  `Request::GetSessionSummaryBackfillStatus`/`Reply::BackfillStatus`(Task 1)。
- Produces: `pub struct BackfillRegistry`(`new()`/`start(cwd, total)`/
  `increment(cwd)`/`get(cwd) -> Option<BackfillProgress>`),`server::serve`
  新增第 7 个参数 `backfill_registry: Arc<BackfillRegistry>`——`main.rs` 与
  测试里所有 `serve(...)` 调用点都要同步加这个参数,否则编译不过。

- [x] **Step 1: 写 `BackfillRegistry` 的失败测试**

创建 `crates/dozerd/src/session_summary_backfill.rs`:

```rust
//! 补总结后台任务的内存态进度登记表(spec 2026-08-28)。按 `cwd` 索引,
//! `dozerd` 重启即丢失——与既有 `finalize_session_summary` 轮询任务同一
//! 哲学:这个用例时间尺度是秒级到十几秒,不值得为它做持久化任务队列。

use dozer_core::protocol::{ConversationSummary, SessionSummaryPayload, SummaryStatus};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackfillProgress {
    pub total: u32,
    pub completed: u32,
}

pub struct BackfillRegistry {
    by_cwd: Mutex<HashMap<String, BackfillProgress>>,
}

impl BackfillRegistry {
    pub fn new() -> Self {
        Self {
            by_cwd: Mutex::new(HashMap::new()),
        }
    }

    /// 开始一轮新的补总结:覆盖该 `cwd` 之前的记录(若有)。必须在
    /// `Request::BackfillSessionSummaries` 处理器里、回 `Reply::Ok` **之前**
    /// 调用,保证调用方随后立即轮询也能看到正确的 `total`,不会撞见"任务
    /// 还没算出 total"的空窗期。
    pub fn start(&self, cwd: &str, total: u32) {
        self.by_cwd
            .lock()
            .expect("lock")
            .insert(cwd.to_string(), BackfillProgress { total, completed: 0 });
    }

    /// 处理完一条(无论成功还是降级到启发式兜底,都算"已处理")后 +1。
    pub fn increment(&self, cwd: &str) {
        if let Some(p) = self.by_cwd.lock().expect("lock").get_mut(cwd) {
            p.completed = p.completed.saturating_add(1);
        }
    }

    pub fn get(&self, cwd: &str) -> Option<BackfillProgress> {
        self.by_cwd.lock().expect("lock").get(cwd).copied()
    }
}

impl Default for BackfillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 某 `cwd` 下有对话记录但没有 `session_summaries` 行的会话列表——复用
/// `Request::ListConversationsWithSummaries` 处理器同一套查询组合
/// (`list_conversations` + `get_many`),这里直接内部调用,不走协议层
/// 往返。
pub fn missing_summary_conversations(
    transcripts: &crate::transcripts::TranscriptStore,
    session_summaries: &crate::session_summary::SessionSummaryStore,
    cwd: &str,
) -> Vec<ConversationSummary> {
    let conversations = transcripts
        .list_conversations(cwd, None, u32::MAX, 0)
        .unwrap_or_default();
    let ids: Vec<String> = conversations.iter().map(|c| c.conversation_id.clone()).collect();
    let summaries = session_summaries.get_many(&ids).unwrap_or_default();
    conversations
        .into_iter()
        .filter(|c| !summaries.contains_key(&c.conversation_id))
        .collect()
}

/// 后台任务本体:依次(串行)处理 `missing` 里的每条会话——headless 总结
/// 成功则 `ai_generated` 落库,失败/超时/不支持则降级到
/// `heuristic_from_turns` 走 `heuristic_fallback` 落库,每条处理完都
/// `registry.increment`。没有真实 PTY session,`session_id` 用
/// `backfill:{conversation_id}` 前缀拼出一个唯一键(真实 dozerd session id
/// 是 UUID v4,`session.rs:75`,不带这个前缀,不会撞车)——`conversation_id`
/// 才是这一行真正的关联键,`session_id` 只是满足表主键约束的占位唯一值。
pub async fn run_backfill(
    cwd: String,
    missing: Vec<ConversationSummary>,
    transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
    session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
    registry: std::sync::Arc<BackfillRegistry>,
) {
    let agent = crate::default_agent_config::load_default_agent();
    for conv in missing {
        let turns = transcripts
            .get_conversation_turns(&conv.conversation_id, -1, u32::MAX)
            .unwrap_or_default();
        let human: Vec<String> = turns
            .iter()
            .filter(|t| t.role == "human")
            .map(|t| t.content.clone())
            .collect();
        let (title, summary, status) = match crate::headless_agent::summarize_headless(agent, &human).await
        {
            Ok((t, s)) => (t, s, SummaryStatus::AiGenerated),
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    conversation_id = %conv.conversation_id,
                    "headless 总结失败,走启发式兜底"
                );
                let (t, s) = crate::session_summary::heuristic_from_turns(&turns);
                (t, s, SummaryStatus::HeuristicFallback)
            }
        };
        let payload = SessionSummaryPayload {
            session_id: format!("backfill:{}", conv.conversation_id),
            agent_kind: conv.agent,
            conversation_id: Some(conv.conversation_id.clone()),
            title,
            summary,
            status,
            created_ts_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        if let Err(e) = session_summaries.record(&payload) {
            tracing::error!(error = %e, conversation_id = %conv.conversation_id, "补总结落库失败");
        }
        registry.increment(&cwd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `AgentKind` 只在测试里直接点名构造(生产代码路径只经手
    // `ConversationSummary.agent` 字段搬运,不需要在顶层 `use` 里引入这个
    // 类型名——引入了在非测试编译下会触发 `unused_imports`),所以单独在
    // 测试模块内 `use`。
    use dozer_core::protocol::AgentKind;

    #[test]
    fn registry_start_then_get_returns_total_with_zero_completed() {
        let reg = BackfillRegistry::new();
        reg.start("/p", 5);
        assert_eq!(reg.get("/p"), Some(BackfillProgress { total: 5, completed: 0 }));
    }

    #[test]
    fn registry_increment_advances_completed() {
        let reg = BackfillRegistry::new();
        reg.start("/p", 2);
        reg.increment("/p");
        assert_eq!(reg.get("/p"), Some(BackfillProgress { total: 2, completed: 1 }));
        reg.increment("/p");
        assert_eq!(reg.get("/p"), Some(BackfillProgress { total: 2, completed: 2 }));
    }

    #[test]
    fn registry_get_unknown_cwd_returns_none() {
        let reg = BackfillRegistry::new();
        assert_eq!(reg.get("/never-started"), None);
    }

    #[test]
    fn registry_start_overwrites_previous_run_for_same_cwd() {
        let reg = BackfillRegistry::new();
        reg.start("/p", 3);
        reg.increment("/p");
        reg.start("/p", 1); // 新一轮"修复项目"点击
        assert_eq!(reg.get("/p"), Some(BackfillProgress { total: 1, completed: 0 }));
    }

    #[tokio::test]
    async fn missing_summary_conversations_excludes_already_summarized() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let transcripts = crate::transcripts::TranscriptStore::open(&db).unwrap();
        let session_summaries = crate::session_summary::SessionSummaryStore::open(&db).unwrap();
        // 没有真实摄取数据时,查询应返回空,不 panic。
        let missing = missing_summary_conversations(&transcripts, &session_summaries, "/no/such/project");
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn run_backfill_falls_back_to_heuristic_when_agent_kind_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let transcripts = std::sync::Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
        let session_summaries =
            std::sync::Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());
        let registry = std::sync::Arc::new(BackfillRegistry::new());
        registry.start("/p", 1);
        let conv = ConversationSummary {
            conversation_id: "conv-1".into(),
            agent: AgentKind::Codex, // headless_agent 对 Codex 返回 Unsupported
            file_path: "/x".into(),
            title: "t".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 0,
        };
        run_backfill(
            "/p".into(),
            vec![conv],
            transcripts,
            session_summaries.clone(),
            registry.clone(),
        )
        .await;
        assert_eq!(registry.get("/p"), Some(BackfillProgress { total: 1, completed: 1 }));
        let got = session_summaries.get("backfill:conv-1").unwrap().unwrap();
        assert_eq!(got.status, SummaryStatus::HeuristicFallback);
        assert_eq!(got.conversation_id, Some("conv-1".into()));
    }
}
```

在 `crates/dozerd/src/lib.rs` 里追加:

```rust
pub mod session_summary_backfill;
```

- [x] **Step 2: 跑测试确认通过**

Run: `cargo test -p dozerd session_summary_backfill`
Expected: PASS(6 个测试)。

- [x] **Step 3: 接入 `server.rs`**

在 `crates/dozerd/src/server.rs` 顶部 `pub async fn serve` 签名里加一个参数
(紧跟 `session_summaries` 之后):

```rust
pub async fn serve(
    socket: &Path,
    registry: Arc<SessionRegistry>,
    store: Arc<crate::acceptance::AcceptanceStore>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
) -> Result<()> {
```

在函数体的 accept 循环里,`let session_summaries = session_summaries.clone();`
之后追加:

```rust
        let backfill_registry = backfill_registry.clone();
```

`tokio::spawn(async move { if let Err(e) = handle_conn(...) ... })` 的调用
参数列表里,`session_summaries,` 之后追加 `backfill_registry,`。

`async fn handle_conn` 的签名同样加一个参数(紧跟 `session_summaries` 之
后):

```rust
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
) -> Result<()> {
```

把 Task 1 Step 5 打的两个占位分支(`Request::BackfillSessionSummaries {
.. } => Reply::Error { .. }` 和 `Request::GetSessionSummaryBackfillStatus
{ .. } => Reply::BackfillStatus { total: 0, completed: 0 }`)整段替换成
真正的实现:

```rust
                        Request::BackfillSessionSummaries { cwd } => {
                            let missing = crate::session_summary_backfill::missing_summary_conversations(
                                &transcripts,
                                &session_summaries,
                                &cwd,
                            );
                            let total = missing.len() as u32;
                            backfill_registry.start(&cwd, total);
                            tokio::spawn(crate::session_summary_backfill::run_backfill(
                                cwd.clone(),
                                missing,
                                transcripts.clone(),
                                session_summaries.clone(),
                                backfill_registry.clone(),
                            ));
                            Reply::Ok
                        }
                        Request::GetSessionSummaryBackfillStatus { cwd } => {
                            let progress = backfill_registry.get(&cwd).unwrap_or_default();
                            Reply::BackfillStatus {
                                total: progress.total,
                                completed: progress.completed,
                            }
                        }
```

- [x] **Step 4: 接入 `main.rs`**

在 `crates/dozerd/src/main.rs` 里,`session_summaries` 构造之后追加:

```rust
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
```

`dozerd::server::serve(...)` 调用的参数列表里,`session_summaries,` 之后
追加 `backfill_registry,`。

- [x] **Step 5: 修 `dozer-client` 集成测试的 `start_daemon` 帮助函数**

在 `crates/dozer-client/tests/against_real_daemon.rs` 的 `start_daemon`
函数里,`session_summaries` 构造之后追加:

```rust
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
```

`dozerd::server::serve(...)` 调用参数列表里,`session_summaries,` 之后
追加 `backfill_registry,`。

- [x] **Step 6: 写端到端集成测试**

在 `against_real_daemon.rs` 里追加:

```rust
#[tokio::test]
async fn backfill_session_summaries_on_empty_project_completes_immediately() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    client
        .backfill_session_summaries("/no/such/project")
        .await
        .unwrap();
    // 空项目:total=0,应该立刻可查到"已完成"(completed>=total)。
    let (completed, total) = client
        .get_session_summary_backfill_status("/no/such/project")
        .await
        .unwrap();
    assert_eq!((completed, total), (0, 0));
}

#[tokio::test]
async fn get_backfill_status_for_never_started_cwd_returns_zero() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let (completed, total) = client
        .get_session_summary_backfill_status("/never/touched")
        .await
        .unwrap();
    assert_eq!((completed, total), (0, 0));
}
```

- [x] **Step 7: 跑全部相关测试确认通过**

Run: `cargo test -p dozerd -p dozer-client`
Expected: PASS(含 `handle_conn`/`serve` 签名改动波及的既有测试——若有其它
测试文件也调用了 `serve(...)`,同样按 Step 5 的方式补 `backfill_registry`
参数,先跑一次确认没有遗漏的调用点)。

- [x] **Step 8: lint**

Run: `cargo clippy -p dozerd -p dozer-client --all-targets -- -D warnings && cargo fmt -p dozerd -p dozer-client -- --check`
Expected: 全绿

- [x] **Step 9: Commit**

```bash
git add crates/dozerd/src/lib.rs crates/dozerd/src/session_summary_backfill.rs \
        crates/dozerd/src/server.rs crates/dozerd/src/main.rs \
        crates/dozer-client/tests/against_real_daemon.rs
git commit -m "$(cat <<'EOF'
feat(dozerd): wire background session-summary backfill task into server

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `dozer-app` 状态模型 + 逐步骤 `spawn_repair_run`

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `project_scaffold::{scaffold_steps, ScaffoldStep, ScaffoldStepResult}`(既有)、
  `client.backfill_project_transcripts`/`client.backfill_session_summaries`/
  `client.get_session_summary_backfill_status`(既有 + Task 2)。
- Produces: `pub struct ScaffoldRunState`(字段 `steps: Vec<(String,
  ScaffoldStepState)>`、`backfill: BackfillStepState`,方法 `all_done() ->
  bool`)、`WorkspaceState.scaffold_run: Option<ScaffoldRunState>`、
  `pub fn spawn_repair_run(repo_path, project_id, client, handle, emit)`——
  Task 8(弹窗渲染)直接读 `ws_state.scaffold_run`。

- [x] **Step 1: 加状态类型 + 精简 `WorkspaceState`**

在 `crates/dozer-app/src/extensions/project.rs` 里,把
`WorkspaceState` 的字段:

```rust
    pub(crate) scaffold_report: Option<project_scaffold::ScaffoldReport>,
```

替换成:

```rust
    pub(crate) scaffold_run: Option<ScaffoldRunState>,
```

在 `WorkspaceState` 定义之前(或紧跟其后)加新类型:

```rust
/// 单个同步 scaffold 步骤(缓存目录/README/git 仓库/项目文档与 Agent
/// 记忆)在弹窗里的实时状态。`ScaffoldStepResult` 只有终态,这里补一层
/// pending/running。
#[derive(Debug, Clone, PartialEq)]
pub enum ScaffoldStepState {
    Pending,
    Running,
    Done(project_scaffold::ScaffoldStepResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackfillProgress {
    pub completed: u32,
    pub total: u32,
}

/// "补总结"聚合进度行的状态。`Done` 不区分成功/失败——补总结内部每条都有
/// 自己的降级路径(headless 失败就走启发式,见 `dozerd` 侧),从这个面板的
/// 视角看永远是"处理完了 N/M 条",没有整体失败态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillStepState {
    Pending,
    Running(BackfillProgress),
    Done(BackfillProgress),
}

/// 一次"修复项目"弹窗跑的完整状态:4 个同步步骤 + 转录历史补录 + 补总结
/// 聚合进度。`steps` 里固定 5 项,顺序 = `project_scaffold::scaffold_steps()`
/// 的 4 项 + 追加的"agent 历史"转录补录(与既有 `spawn_scaffold_run` 的
/// 拼接顺序一致)。
#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldRunState {
    pub steps: Vec<(String, ScaffoldStepState)>,
    pub backfill: BackfillStepState,
}

impl ScaffoldRunState {
    /// 初始态:全部步骤 Pending,补总结也 Pending。
    fn pending() -> Self {
        let mut steps: Vec<(String, ScaffoldStepState)> = project_scaffold::scaffold_steps()
            .into_iter()
            .map(|s| (s.label.to_string(), ScaffoldStepState::Pending))
            .collect();
        steps.push(("agent 历史".to_string(), ScaffoldStepState::Pending));
        Self {
            steps,
            backfill: BackfillStepState::Pending,
        }
    }

    pub fn all_done(&self) -> bool {
        self.steps.iter().all(|(_, s)| matches!(s, ScaffoldStepState::Done(_)))
            && matches!(self.backfill, BackfillStepState::Done(_))
    }
}
```

- [x] **Step 2: 加 Message 变体,删掉 `ScaffoldDone`**

删除:

```rust
    ScaffoldDone(project_scaffold::ScaffoldReport, bool),
```

（及其上方对应的文档注释块）。追加:

```rust
    /// 静默 scaffold 跑(打开项目 tab 时触发,不弹窗)完成。目前没有任何
    /// UI 需要消费这个结果——四个同步步骤 + 转录补录的副作用已经落地,这
    /// 条消息只是给 `update()` 一个"忽略"分支占位,不驱动任何状态。
    ScaffoldDone,
    /// "修复项目"弹窗:第 `idx` 个同步步骤(下标对应
    /// `ScaffoldRunState.steps`)进入 Running。
    ScaffoldStepStarted(i64, usize),
    /// 同上,携带该步骤终态。
    ScaffoldStepFinished(i64, usize, project_scaffold::ScaffoldStepResult),
    /// "agent 历史"转录补录步骤(固定是 `steps` 的最后一项)开始。
    TranscriptBackfillStarted(i64),
    /// 同上,携带终态。
    TranscriptBackfillFinished(i64, project_scaffold::ScaffoldStepResult),
    /// 补总结轮询到新的 `(completed, total)`。`completed >= total` 时
    /// `update()` 把 `backfill` 置为 `Done`,否则 `Running`。
    SummaryBackfillProgress(i64, u32, u32),
    /// 弹窗"关闭"按钮(全部完成才可点)。
    ScaffoldPopupClose,
```

- [x] **Step 3: 改 `update()`**

删除原来的:

```rust
        Message::RepairProject => {
            spawn_scaffold_run(repo_path.to_path_buf(), true, client.clone(), handle, emit);
        }
```

替换成:

```rust
        Message::RepairProject => {
            ws_state.scaffold_run = Some(ScaffoldRunState::pending());
            spawn_repair_run(repo_path.to_path_buf(), project_id, client.clone(), handle, emit);
        }
```

删除原来的:

```rust
        Message::ScaffoldDone(report, visible) => {
            if visible {
                ws_state.scaffold_report = Some(report);
            }
        }
```

替换成:

```rust
        Message::ScaffoldDone => {}
        Message::ScaffoldStepStarted(_, idx) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                if let Some((_, state)) = run.steps.get_mut(idx) {
                    *state = ScaffoldStepState::Running;
                }
            }
        }
        Message::ScaffoldStepFinished(_, idx, result) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                if let Some((_, state)) = run.steps.get_mut(idx) {
                    *state = ScaffoldStepState::Done(result);
                }
            }
        }
        Message::TranscriptBackfillStarted(_) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let last = run.steps.len() - 1;
                if let Some((_, state)) = run.steps.get_mut(last) {
                    *state = ScaffoldStepState::Running;
                }
            }
        }
        Message::TranscriptBackfillFinished(_, result) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let last = run.steps.len() - 1;
                if let Some((_, state)) = run.steps.get_mut(last) {
                    *state = ScaffoldStepState::Done(result);
                }
            }
        }
        Message::SummaryBackfillProgress(_, completed, total) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let progress = BackfillProgress { completed, total };
                run.backfill = if completed >= total {
                    BackfillStepState::Done(progress)
                } else {
                    BackfillStepState::Running(progress)
                };
            }
        }
        Message::ScaffoldPopupClose => {
            if matches!(&ws_state.scaffold_run, Some(run) if run.all_done()) {
                ws_state.scaffold_run = None;
            }
        }
```

- [x] **Step 4: 重写 `spawn_scaffold_run`(静默路径,签名精简),新增
  `spawn_repair_run`**

把现有的 `pub fn spawn_scaffold_run(...)` 整个替换成两个函数:

```rust
/// 静默 scaffold 跑(项目 tab 打开时触发,`app.rs::project_tab_opened`
/// 唯一调用点,不弹窗)。跑完四个同步步骤 + 转录历史补录,不产出任何
/// UI 可见结果——`emit(Message::ScaffoldDone)` 只是让调用方知道这批
/// spawn 任务已经跑完(目前没有消费方,`update()` 是空分支),不携带内容。
/// **不触发补总结**:补总结只在显式点击"修复项目"时跑(spec
/// 2026-08-28,避免每次静默打开项目都真实拉起 agent 进程)。
pub fn spawn_scaffold_run(
    repo_path: std::path::PathBuf,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        tokio::task::spawn_blocking(move || project_scaffold::run_sync_steps(&repo_path2))
            .await
            .ok();
        let _ = client.backfill_project_transcripts(&cwd).await;
        emit(Message::ScaffoldDone);
    });
}

const SUMMARY_BACKFILL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// "修复项目"按钮触发的完整跑法:4 个同步步骤逐个 Started/Finished、
/// 转录历史补录 Started/Finished、再触发补总结并轮询进度,全部实时
/// `emit` 给弹窗(spec 2026-08-28)。所有消息都携带 `project_id`,靠
/// `app.rs` 里按 `project_id` 查找 workspace 的路由分支落地(不依赖
/// "当前激活哪个 tab"),避免用户在补总结进行中切换项目 tab 时消息投递到
/// 错误的 workspace。
pub fn spawn_repair_run(
    repo_path: std::path::PathBuf,
    project_id: i64,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let steps = project_scaffold::scaffold_steps();
        for (idx, step) in steps.into_iter().enumerate() {
            emit(Message::ScaffoldStepStarted(project_id, idx));
            let repo_path3 = repo_path.clone();
            let result = tokio::task::spawn_blocking(move || (step.run)(&repo_path3))
                .await
                .unwrap_or_else(|e| {
                    project_scaffold::ScaffoldStepResult::Failed(format!("内部错误: {e}"))
                });
            emit(Message::ScaffoldStepFinished(project_id, idx, result));
        }

        emit(Message::TranscriptBackfillStarted(project_id));
        let backfill_result = match client.backfill_project_transcripts(&cwd).await {
            Ok(0) => project_scaffold::ScaffoldStepResult::AlreadyOk,
            Ok(n) => project_scaffold::ScaffoldStepResult::Created(format!("导入 {n} 个历史文件")),
            Err(e) => project_scaffold::ScaffoldStepResult::Failed(e.to_string()),
        };
        emit(Message::TranscriptBackfillFinished(project_id, backfill_result));

        if let Err(e) = client.backfill_session_summaries(&cwd).await {
            tracing::warn!(error = %e, "补总结请求发送失败,视为无需补");
            emit(Message::SummaryBackfillProgress(project_id, 0, 0));
            return;
        }
        loop {
            tokio::time::sleep(SUMMARY_BACKFILL_POLL_INTERVAL).await;
            match client.get_session_summary_backfill_status(&cwd).await {
                Ok((completed, total)) => {
                    emit(Message::SummaryBackfillProgress(project_id, completed, total));
                    if completed >= total {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "查询补总结进度失败,停止轮询");
                    break;
                }
            }
        }
    });
}
```

- [x] **Step 5: 改 `app.rs` 的静默调用点 + 加 `project_id` 路由**

`app.rs:5215` 附近的调用:

```rust
        project::spawn_scaffold_run(repo_path, false, client, &handle, emit);
```

改成:

```rust
        project::spawn_scaffold_run(repo_path, client, &handle, emit);
```

在 `app.rs` 里,原本这个组合匹配分支(约 4852-4857 行):

```rust
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::AcceptanceCountLoaded(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)),
            ) => {
```

改成(追加新变体到同一个匹配组,复用其后完全相同的处理体——这些新消息
需要的正是"按消息自带的 `project_id` 查 workspace,委托 `project::update`"
这套既有逻辑,不需要另写分支):

```rust
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::AcceptanceCountLoaded(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)),
            ) => {
```

（分支体内部逻辑不变——它已经是"用 `project_id` 找 `ws`,调
`project::update`"的通用写法,不需要改动。）

`Message::Project(project::Message::ScaffoldDone)`(不带 project_id 的
静默完成信号)与 `Message::Project(project::Message::ScaffoldPopupClose)`
不需要特殊路由,会自然落进已有的 `Message::Project(msg) => { ... 用
`self.active_project_id` ... }` 兜底分支——这两个都是"当前激活 tab 触发
的、当下立刻处理"的消息(静默 scaffold 只在打开当前 tab 时触发;
弹窗关闭按钮只有当前激活 tab 的弹窗才可能被点到),不存在跨 tab 切换后
消息投递错位的风险,不需要跟着改。

- [x] **Step 6: 改既有测试**

`project.rs` 测试里引用 `Message::ScaffoldDone(report.clone(), false)`/
`(report.clone(), true)`/`ws_state.scaffold_report` 的用例(约 1640-1670
行)整段替换成针对新状态机的测试:

这几个测试复用文件现有 `mod tests` 里已经存在的三个辅助函数——
`new_ws() -> WorkspaceState`、`test_repo_path() -> PathBuf`、
`test_client() -> dozer_client::Client`(定义于 `mod tests` 顶部,`git
refreshed_updates_four_fields` 等既有测试同款用法),`tokio::runtime::Handle`
现场 `tokio::runtime::Runtime::new().unwrap()` 构造,不新增辅助函数:

```rust
    #[test]
    fn repair_project_sets_pending_scaffold_run() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::RepairProject,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.expect("弹窗状态应已初始化");
        assert_eq!(run.steps.len(), 5);
        assert!(run.steps.iter().all(|(_, s)| matches!(s, ScaffoldStepState::Pending)));
        assert_eq!(run.backfill, BackfillStepState::Pending);
    }

    #[test]
    fn scaffold_step_started_then_finished_updates_that_step_only() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::ScaffoldStepStarted(1, 1),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.as_ref().unwrap();
        assert_eq!(run.steps[0].1, ScaffoldStepState::Pending);
        assert_eq!(run.steps[1].1, ScaffoldStepState::Running);

        update(
            &mut ws,
            Message::ScaffoldStepFinished(1, 1, project_scaffold::ScaffoldStepResult::AlreadyOk),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.as_ref().unwrap();
        assert_eq!(
            run.steps[1].1,
            ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::AlreadyOk)
        );
    }

    #[test]
    fn summary_backfill_progress_marks_done_when_completed_reaches_total() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::SummaryBackfillProgress(1, 2, 5),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.scaffold_run.as_ref().unwrap().backfill,
            BackfillStepState::Running(BackfillProgress { completed: 2, total: 5 })
        );

        update(
            &mut ws,
            Message::SummaryBackfillProgress(1, 5, 5),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.scaffold_run.as_ref().unwrap().backfill,
            BackfillStepState::Done(BackfillProgress { completed: 5, total: 5 })
        );
    }

    #[test]
    fn popup_close_only_clears_state_when_all_done() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::ScaffoldPopupClose,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.scaffold_run.is_some(), "未完成时点关闭不应清空状态");
    }
```

- [x] **Step 7: 跑测试确认通过**

Run: `cargo test -p dozer-app project::`
Expected: PASS

- [x] **Step 8: 全量编译检查(`app.rs` 改动影响面广)**

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: PASS 全绿(这一步会暴露任何遗漏的调用点/字段引用)。

- [x] **Step 9: lint**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全绿

- [x] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): step-by-step repair-run state machine + summary backfill polling

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: 弹窗 UI + footer-bar 接线

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`

**Interfaces:**
- Consumes: `ws_state.scaffold_run`(Task 7)、`project_delete_confirm_popup`
  同款 `stack![base, dismiss, popup]` 视觉模板(既有)。
- Produces: `fn scaffold_progress_popup(&WorkspaceState) -> Element` (私有,
  仅本文件内的 `view`/渲染函数调用)。

- [x] **Step 1: 删掉旧的 footer-bar 状态文字块**

在 `project_footer_bar` 函数里,删除:

```rust
    let status = ws_state.scaffold_report.as_ref().map(|report| {
        text(project_scaffold::format_scaffold_report(report))
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim)
    });

    let mut col = column![top_line, bar].spacing(4);
    if let Some(status) = status {
        col = col.push(status);
    }
```

替换成:

```rust
    let col = column![top_line, bar].spacing(4);
```

（`project_scaffold::format_scaffold_report` 目前只被这里调用,删除这段
后其它地方若已无引用,函数本身连同它的单元测试可以保留在
`project_scaffold.rs` 里不动——它是纯函数,留着不影响任何东西,不必强行
删除跨文件的既有测试覆盖。）

- [x] **Step 2: 加弹窗渲染函数**

在 `project_delete_confirm_popup` 函数定义之前(或之后,同一文件内任意
靠近的位置)新增:

```rust
/// "修复项目"弹窗单行:左侧状态符号 + 步骤名 + 右侧简短详情文字。
fn scaffold_step_row<'a>(
    label: &'a str,
    state: &'a ScaffoldStepState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (glyph, glyph_color, detail): (&str, iced_widget::core::Color, String) = match state {
        ScaffoldStepState::Pending => ("○", byteui::theme::color::current().dim, String::new()),
        ScaffoldStepState::Running => ("…", byteui::theme::color::current().gold, "进行中".to_string()),
        ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::AlreadyOk) => {
            ("✓", byteui::theme::color::current().green, "已是最新".to_string())
        }
        ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::Created(msg)) => {
            ("✓", byteui::theme::color::current().green, msg.clone())
        }
        ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::Failed(msg)) => {
            ("✗", byteui::theme::color::current().red, msg.clone())
        }
    };
    row![
        text(glyph).size(byteui::theme::font::body()).color(glyph_color),
        text(label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream)
            .width(Length::Fixed(140.0)),
        text(detail)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

fn scaffold_backfill_row(
    state: &BackfillStepState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (glyph, glyph_color, detail) = match state {
        BackfillStepState::Pending => ("○".to_string(), byteui::theme::color::current().dim, String::new()),
        BackfillStepState::Running(p) => (
            "…".to_string(),
            byteui::theme::color::current().gold,
            format!("{}/{}", p.completed, p.total),
        ),
        BackfillStepState::Done(p) if p.total == 0 => (
            "✓".to_string(),
            byteui::theme::color::current().green,
            "无需补".to_string(),
        ),
        BackfillStepState::Done(p) => (
            "✓".to_string(),
            byteui::theme::color::current().green,
            format!("{}/{}", p.completed, p.total),
        ),
    };
    row![
        text(glyph).size(byteui::theme::font::body()).color(glyph_color),
        text("补总结")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream)
            .width(Length::Fixed(140.0)),
        text(detail)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

/// "修复项目"进度弹窗:视觉模板同 `project_delete_confirm_popup`(卡片 +
/// 底部按钮)。进行中时"关闭"按钮不可点(`on_press_maybe`),全部完成
/// (`ScaffoldRunState::all_done`)才激活。
fn scaffold_progress_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(run) = &ws_state.scaffold_run else {
        return container(column![]).into();
    };
    let mut rows = column![].spacing(10);
    for (label, state) in &run.steps {
        rows = rows.push(scaffold_step_row(label, state));
    }
    rows = rows.push(scaffold_backfill_row(&run.backfill));

    let done = run.all_done();
    let close_label = if done { "关闭" } else { "进行中…" };
    let close_btn = button(
        text(close_label)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press_maybe(done.then_some(Message::ScaffoldPopupClose))
    .padding([6, 12])
    .style(move |_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: if done {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });

    let card = column![
        text("修复项目")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
        rows,
        container(close_btn).align_x(iced_widget::core::alignment::Horizontal::Right),
    ]
    .spacing(14);

    container(card)
        .width(Length::Fixed(360.0))
        .padding(16)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

- [x] **Step 3: 挂进主渲染栈**

在 `view` 函数里(`if ws_state.delete_pending.is_some() { ... }` 那段
判断附近),追加一个并列分支——`scaffold_run` 和 `delete_pending` 不会
同时非空(修复和删除是两个互斥的 footer-bar 按钮触发的弹窗,产品逻辑上
不需要处理"两个弹窗同时开着"这种情况,`if`/`else if` 顺序判断即可):

```rust
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
        return stack![base, dismiss, project_delete_confirm_popup(ws_state)]
            .width(width)
            .height(Length::Fill)
            .into();
    }

    if ws_state.scaffold_run.is_some() {
        // 进行中不可通过点击遮罩关闭:遮罩本身不挂 `on_press`,只挡住底层
        // 交互(与 `delete_pending` 分支的可点击遮罩故意不同,见 spec
        // "弹窗可取消性"一节:必须等全部步骤完成才能关)。
        let scrim = container(column![])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().scrim.into()),
                ..iced_widget::container::Style::default()
            });
        return stack![base, scrim, scaffold_progress_popup(ws_state)]
            .width(width)
            .height(Length::Fill)
            .into();
    }

    base.into()
```

- [x] **Step 4: 编译检查**

Run: `cargo build -p dozer-app`
Expected: PASS

- [x] **Step 5: 跑相关测试**

Run: `cargo test -p dozer-app project::`
Expected: PASS(渲染函数本身不好写自动化断言——iced `Element` 树没有内建
的"渲染成文本快照"机制,这个任务的正确性主要靠 Task 9 的人工验证,单测
只覆盖 Task 7 已经写好的状态机)。

- [x] **Step 6: lint**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全绿

- [x] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): repair-project checklist popup UI

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: 全量验证 + 人工测试清单

**Files:**
- 无代码改动(纯验证)。

- [x] **Step 1: 全 workspace 构建**

Run: `cargo build`
Expected: 全绿,无警告。

- [x] **Step 2: 全 workspace 测试**

Run: `cargo test`
Expected: 全绿(含 Task 1-8 新增的全部测试)。

- [x] **Step 3: lint + 格式**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 4: 人工验证清单(记录结果,不需要自动化)**

按 spec `测试策略` §7 逐条过:

1. 编辑 `~/Library/Application Support/ai.byteboy.dozer/config.toml`(macOS
   `config_dir()` 实际落点,启动 `dozerd` 后用
   `dozerd --version` 附近日志或直接 `find ~/Library -iname
   'config.toml' -path '*dozer*'` 确认真实路径)写入
   `default_agent = "claude"`,确保本机已登录可用的 Claude CLI。
2. 打开一个此前已经产生过对话历史、但缺总结的项目,点"修复项目",确认
   弹窗依次跑完 6 项(4 同步步骤 + agent 历史 + 补总结),补总结行数字从
   `0/N` 涨到 `N/N`,`session_summaries` 表(`sqlite3 dozer.db "select
   session_id, status from session_summaries"`)对应行 `status=ai_generated`。
3. 把 `default_agent` 改成本机没装的二进制名(如 `"opencode"` 但没装
   opencode),重复步骤 2,确认补总结行仍能走到"完成",但落库的行是
   `status=heuristic_fallback`。
4. 静默打开项目(不点"修复项目"按钮):确认不弹窗、不触发
   `BackfillSessionSummaries` 请求(可通过 `RUST_LOG=dozerd=debug` 观察
   `dozerd` 日志确认没有相关请求进来)。
5. 若本机已装 CodeBuddy/OpenCode CLI:分别切换 `default_agent` 重复步骤
   2,确认对应 CLI 的一次性调用参数(尤其 CodeBuddy 的 `-y`)确实生效、
   不会卡在授权确认上。
6. V8agent:在 `v8agent` 仓库那份计划落地前,预期确认的是"降级到
   `heuristic_fallback` 而不是整体卡死/报错"这条路径;落地之后再补一次
   `status=ai_generated` 的验证。

结果记录:哪些通过、哪些因环境限制(缺某个 CLI)未能验证,写进这次
PR/commit 的描述里,不需要写回本计划文件。

- [ ] **Step 5(可选): 无需 commit**

本任务不改代码,如果 Step 1-3 全绿、Step 4 记录完毕,直接向用户汇报即可。
