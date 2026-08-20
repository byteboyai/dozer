# Agent 对话/用量摄取管线 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 现在"每次直读磁盘 JSONL 重新解析"的对话面板/用量面板数据路径,改为 `dozerd` 统一摄取解析、落库到 sqlite,`dozer-app` 侧改为纯 UDS 查询。

**Architecture:** `dozerd` 新增 `TranscriptStore`(两张表:`conversations` 索引 + `conversation_turns` 明细),摄取逻辑由 hook 事件快通道 + agent 转入待命态的兜底扫描 + 启动回填三条路径驱动;协议新增 3 组 `Request`/`Reply`;`dozer-app` 侧 `ConversationMeta`/`ReviewEntry`/`ConversationUsage` 三个既有展示类型保持不变,只把它们的数据来源从"本地读文件解析"换成"从 dozerd 查询后转换",渲染代码(`workspace.rs` 的 `review_content`/`conversation_list_pane`、`usage.rs` 的 `view` 等)完全不动。

**Tech Stack:** Rust workspace;`rusqlite`(bundled feature,已在 `dozerd` 依赖里);`tokio`(异步 UDS server/client,已有);无新增外部依赖。

**Spec:** `docs/superpowers/specs/2026-08-20-dozer-conversation-ingestion-design.md`

## Global Constraints

- 摄取覆盖三家:Claude / CodeBuddy / OpenCode;Codex/Qoder/V8agent 维持"解析返回空"的现状,不在本计划内补齐真实 schema。
- 不做运行中回合的实时流式展示;面板只反映已摄取入库的完整回合。
- 不接入 `dozer-mcp`;不做共享记忆功能本体;不引入 `sqlite-vec`。
- 不做"孤儿行清理"——文件截断重写时只增量 upsert,不删除库里已有但新内容没有对应 `message_key` 的旧行。
- `dozer-app` 侧 `ConversationMeta`(`turns/agent/title/...`展示字段)、`ReviewEntry`(`Human`/`AiTurn`)、`ConversationUsage`(token/工具调用统计字段)三个类型的**字段形状不变**,`workspace.rs`/`app.rs` 里消费它们的渲染与消息处理代码不改——只有各自的"怎么拿到数据"这一层(`spawn_conversations_refresh`/`spawn_review_load`/`usage::spawn_refresh`/`homespace::load_home_recents`)换成走 `dozer_client::Client`。
- `latest_model_mode_and_activity`(Agent 卡片实时指示器用)不在本次迁移范围内,继续直读文件,不改动。
- `crates/dozer-app/src/extensions/project/links.rs` 的 `discover_memory` 用途与对话摄取无关(检查 agent 目录下是否存在 `memory` 子目录),依赖的路径拼接函数迁到 `dozer-core::agent_paths` 供两边共用,`links.rs` 本身逻辑不变。
- 每个任务完成后运行:`cargo build -p <改动的 crate>`、`cargo test -p <改动的 crate>`、`cargo clippy -p <改动的 crate> --all-targets -- -D warnings`、`cargo fmt -- --check`(全 workspace fmt 检查开销小,每个任务都跑)。

---

## Task 1: 协议新增类型与 Request/Reply variant

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces: `ConversationSummary { conversation_id: String, agent: AgentKind, file_path: String, title: String, first_ts: u64, last_ts: u64, turn_count: u32 }`、`TurnRecord { turn_index: i64, role: String, content: String, tools_summary: Vec<String>, thinking: bool, ts: Option<u64> }`、`UsagePayload { turns: u32, tool_calls: u32, mutating_tool_calls: u32, files_touched: BTreeSet<String>, tokens_in: u64, tokens_out: u64, tokens_cache_read: u64, tokens_cache_write: u64 }`(均 `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`,`UsagePayload` 另加 `Default`)。
- Produces: `Request::ListConversations { cwd: String, agent: Option<AgentKind>, limit: u32, offset: u32 }`、`Request::GetConversationTurns { conversation_id: String, after_turn_index: i64, limit: u32 }`、`Request::GetUsageSummary { cwd: String, since_ts: Option<u64> }`。
- Produces: `Reply::Conversations { conversations: Vec<ConversationSummary> }`、`Reply::ConversationTurns { conversation_id: String, turns: Vec<TurnRecord> }`、`Reply::UsageSummary { rows: Vec<(ConversationSummary, UsagePayload)> }`。

- [x] **Step 1: 写失败测试(round-trip 序列化)**

在 `crates/dozer-core/src/protocol.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn conversation_protocol_types_roundtrip() {
        let req = Request::ListConversations {
            cwd: "/proj".into(),
            agent: Some(AgentKind::Claude),
            limit: 50,
            offset: 0,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);

        let turns_req = Request::GetConversationTurns {
            conversation_id: "abc".into(),
            after_turn_index: -1,
            limit: 100,
        };
        let line = encode_line(&turns_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(turns_req, back);

        let usage_req = Request::GetUsageSummary {
            cwd: "/proj".into(),
            since_ts: None,
        };
        let line = encode_line(&usage_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(usage_req, back);

        let summary = ConversationSummary {
            conversation_id: "abc".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/abc.jsonl".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 3,
        };
        let turn = TurnRecord {
            turn_index: 0,
            role: "human".into(),
            content: "你好".into(),
            tools_summary: vec!["Edit README.md".into()],
            thinking: true,
            ts: Some(42),
        };
        let usage = UsagePayload {
            turns: 2,
            tool_calls: 1,
            mutating_tool_calls: 1,
            files_touched: std::collections::BTreeSet::from(["README.md".to_string()]),
            tokens_in: 10,
            tokens_out: 20,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        };
        let reply = Reply::UsageSummary {
            rows: vec![(summary.clone(), usage)],
        };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);

        let reply2 = Reply::ConversationTurns {
            conversation_id: "abc".into(),
            turns: vec![turn],
        };
        let line = encode_line(&reply2);
        let back2: Reply = decode_line(&line).unwrap();
        assert_eq!(reply2, back2);

        let reply3 = Reply::Conversations {
            conversations: vec![summary],
        };
        let line = encode_line(&reply3);
        let back3: Reply = decode_line(&line).unwrap();
        assert_eq!(reply3, back3);
    }
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core conversation_protocol_types_roundtrip`
Expected: 编译失败(类型/variant 不存在)。

- [x] **Step 3: 新增类型与 variant**

在 `AgentKind` 定义之后(`protocol.rs` 第 30 行之后)新增:

```rust
use std::collections::BTreeSet;

/// 单个历史会话(=一份 agent transcript 文件)的索引摘要;由 dozerd 的
/// `TranscriptStore` 摄取落库维护(spec 2026-08-20)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub file_path: String,
    pub title: String,
    pub first_ts: u64,
    pub last_ts: u64,
    pub turn_count: u32,
}

/// 会话内一个回合(人类发言 / AI 回复)的明细;`role` 恒为 `"human"` 或
/// `"ai"`(不用枚举是为了跟 sqlite 存储列直接对应,减一层转换)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_index: i64,
    pub role: String,
    pub content: String,
    pub tools_summary: Vec<String>,
    pub thinking: bool,
    pub ts: Option<u64>,
}

/// 单个会话的用量统计(token/工具调用/改动文件),已按 `message_key` 做过
/// fork/resume 去重(spec"用量去重"一节)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsagePayload {
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: BTreeSet<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}
```

在 `Request` 枚举内新增三个 variant(紧跟现有 `HookEvent` 之后即可):

```rust
    /// 列出某 cwd 下的历史对话(跨 Claude/CodeBuddy/OpenCode 三家合并;
    /// `agent` 非空时只查该家)。spec 2026-08-20。
    ListConversations {
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    },
    /// 单个会话的回合明细,keyset 分页(`after_turn_index=-1` 表示从头)。
    GetConversationTurns {
        conversation_id: String,
        after_turn_index: i64,
        limit: u32,
    },
    /// 某 cwd 下按会话分组的用量统计。
    GetUsageSummary {
        cwd: String,
        since_ts: Option<u64>,
    },
```

在 `Reply` 枚举内新增三个 variant:

```rust
    /// `ListConversations` 应答。
    Conversations {
        conversations: Vec<ConversationSummary>,
    },
    /// `GetConversationTurns` 应答。
    ConversationTurns {
        conversation_id: String,
        turns: Vec<TurnRecord>,
    },
    /// `GetUsageSummary` 应答。
    UsageSummary {
        rows: Vec<(ConversationSummary, UsagePayload)>,
    },
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core conversation_protocol_types_roundtrip`
Expected: PASS

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozer-core && cargo test -p dozer-core && cargo clippy -p dozer-core --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): 新增对话摄取管线协议类型与 Request/Reply variant"
```

---

## Task 2: `dozer-core::agent_paths` —— agent 存储目录路径计算(共用)

**Files:**
- Create: `crates/dozer-core/src/agent_paths.rs`
- Modify: `crates/dozer-core/src/lib.rs`(新增 `pub mod agent_paths;`)

**Interfaces:**
- Produces: `home_dir() -> PathBuf`、`claude_project_dir(cwd: &Path) -> PathBuf`、`claude_project_dir_in(home: &Path, cwd: &Path) -> PathBuf`、`codebuddy_project_dir(cwd: &Path) -> PathBuf`、`codebuddy_project_dir_in(home: &Path, cwd: &Path) -> PathBuf`、`opencode_project_dir(cwd: &Path) -> PathBuf`、`opencode_project_dir_in(home: &Path, cwd: &Path) -> PathBuf`。

这些函数是从 `crates/dozer-app/src/conversation.rs` 原样搬来(逻辑不变,只改可见性为 `pub`),供 `dozerd`(摄取扫描)和 `dozer-app`(`links.rs::discover_memory`,与摄取无关的另一用途)共用,避免两边各存一份、行为漂移。

- [x] **Step 1: 写失败测试**

创建 `crates/dozer-core/src/agent_paths.rs`:

```rust
//! agent 各自的会话存储目录路径计算(从 `dozer-app/src/conversation.rs`
//! 搬出,供 dozerd 摄取扫描与 dozer-app `links.rs` 的记忆目录探测共用)。

use std::path::{Path, PathBuf};

pub fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn project_key(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}

/// CodeBuddy 自己的目录命名规则跟 Claude 不一样:Claude 把开头的 `/` 也
/// 一并换成 `-`(留下开头一个 `-`),CodeBuddy 是先去掉开头 `/` 再替换
/// 剩余的 `/`(不留开头 `-`)——实测 `~/.codebuddy/projects/` 下的真实
/// 目录名核实。
fn codebuddy_project_key(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-")
}

fn project_dir_in(home: &Path, agent_root: &str, cwd: &Path) -> PathBuf {
    home.join(agent_root)
        .join("projects")
        .join(project_key(cwd))
}

pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    claude_project_dir_in(&home_dir(), cwd)
}

pub fn claude_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".claude", cwd)
}

pub fn codebuddy_project_dir(cwd: &Path) -> PathBuf {
    codebuddy_project_dir_in(&home_dir(), cwd)
}

pub fn codebuddy_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    home.join(".codebuddy")
        .join("projects")
        .join(codebuddy_project_key(cwd))
}

pub fn opencode_project_dir(cwd: &Path) -> PathBuf {
    opencode_project_dir_in(&home_dir(), cwd)
}

pub fn opencode_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".dozer/agents/opencode", cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn codebuddy_dir_uses_codebuddy_root() {
        let d = codebuddy_project_dir(Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.codebuddy/projects/a-b-c"));
    }

    #[test]
    fn opencode_dir_lives_under_dozer_data_dir() {
        let d = opencode_project_dir(Path::new("/a/b/c"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.dozer/agents/opencode/projects/-a-b-c")
        );
    }
}
```

- [x] **Step 2: 挂进 crate**

在 `crates/dozer-core/src/lib.rs` 新增一行 `pub mod agent_paths;`(找现有 `pub mod paths;` 之类的行,紧跟其后添加)。

- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozer-core agent_paths`
Expected: PASS(3 个测试)

- [x] **Step 4: 全量校验 + 提交**

Run: `cargo build -p dozer-core && cargo test -p dozer-core && cargo clippy -p dozer-core --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozer-core/src/agent_paths.rs crates/dozer-core/src/lib.rs
git commit -m "feat(dozer-core): 新增 agent_paths 模块,供 dozerd/dozer-app 共用存储目录计算"
```

---

## Task 3: `dozerd::transcripts::parse` —— Claude 形状增量解析器

**Files:**
- Create: `crates/dozerd/src/transcripts/parse.rs`

**Interfaces:**
- Produces: `ParsedTurn { message_key: String, role: String, content: String, tools_summary: Vec<String>, thinking: bool, tool_calls: u32, mutating_tool_calls: u32, files_touched: Vec<String>, tokens_in: u64, tokens_out: u64, tokens_cache_read: u64, tokens_cache_write: u64, ts: Option<u64>, raw_json: String }`
- Produces: `parse_chunk(agent: AgentKind, text: &str, conversation_id: &str, starting_turn_index: i64) -> Vec<ParsedTurn>`(本 Task 先实现 Claude 形状分支,Task 4 补 CodeBuddy 分支)。
- Produces: `last_complete_line_boundary(text: &str) -> usize`——返回 `text` 中最后一个 `'\n'` 之后的字节偏移(即"只含完整行"的前缀长度);无换行符时返回 0。

`message_key` 取 Claude 每行顶层的 `uuid` 字段(Claude Code 会话 JSONL 的标准字段,每行一个事件都带 `uuid`/`parentUuid`);取不到时退化为 `"{conversation_id}:{turn_index}"`(spec 已定的退化规则)。**实现前请先用 `head -3 ~/.claude/projects/*/*.jsonl`(任选一份本机真实存在的 Claude 会话文件)确认 `uuid` 字段确实存在于每行顶层**——如果实测字段名不同,把下面 Step 3 里 `v.get("uuid")` 换成实测的正确字段名,不要保留错误假设;单测里已经覆盖了"字段不存在时退化"这条路径,即使字段名一开始就没猜对,行为也不会 panic,只是白白丢了去重能力,发现后改一行字段名即可。

- [x] **Step 1: 写失败测试**

创建 `crates/dozerd/src/transcripts/parse.rs`:

```rust
//! agent transcript(JSONL)增量解析:按 `AgentKind` 分派,产出既带展示
//! 字段又带用量字段的 `ParsedTurn`(合并原 dozer-app `transcript.rs` +
//! `usage.rs` 两套平行解析器,避免长期重复维护——spec"参考调研"一节)。

use dozer_core::protocol::AgentKind;
use serde_json::Value;

pub const MUTATING_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedTurn {
    pub message_key: String,
    pub role: String,
    pub content: String,
    pub tools_summary: Vec<String>,
    pub thinking: bool,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: Vec<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
    pub ts: Option<u64>,
    pub raw_json: String,
}

/// `text` 中"只含完整行"的前缀长度——最后一个 `'\n'` 之后的偏移;没有
/// 换行符(整段都是未写完的半行)时返回 0,即"这次什么都不消费"。
pub fn last_complete_line_boundary(text: &str) -> usize {
    text.rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn fallback_key(conversation_id: &str, turn_index: i64) -> String {
    format!("{conversation_id}:{turn_index}")
}

fn tool_summary(name: &str, input: &Value) -> String {
    let arg = input
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string())
        })
        .or_else(|| {
            input
                .get("command")
                .and_then(|v| v.as_str())
                .map(|c| c.chars().take(40).collect())
        })
        .or_else(|| {
            input
                .get("pattern")
                .or_else(|| input.get("path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    match arg {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    }
}

fn parse_claude_shaped_chunk(
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    let mut out = Vec::new();
    let mut turn_index = starting_turn_index;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(|t| t.as_u64());
        let message_key = v
            .get("uuid")
            .and_then(|u| u.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                let Some(text) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                else {
                    continue;
                };
                out.push(ParsedTurn {
                    message_key,
                    role: "human".into(),
                    content: text.to_string(),
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            Some("assistant") => {
                let Some(blocks) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                let mut content = String::new();
                let mut tools_summary = Vec::new();
                let mut thinking = false;
                let mut tool_calls = 0u32;
                let mut mutating_tool_calls = 0u32;
                let mut files_touched = Vec::new();
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(t);
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            tools_summary.push(tool_summary(name, &input));
                            tool_calls += 1;
                            if MUTATING_TOOLS.contains(&name) {
                                mutating_tool_calls += 1;
                                if let Some(path) =
                                    input.get("file_path").and_then(|p| p.as_str())
                                {
                                    files_touched.push(path.to_string());
                                }
                            }
                        }
                        Some("thinking") => thinking = true,
                        _ => {}
                    }
                }
                let usage = v.get("message").and_then(|m| m.get("usage"));
                let tokens_in = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_out = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_cache_read = usage
                    .and_then(|u| u.get("cache_read_input_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_cache_write = usage
                    .and_then(|u| u.get("cache_creation_input_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                out.push(ParsedTurn {
                    message_key,
                    role: "ai".into(),
                    content,
                    tools_summary,
                    thinking,
                    tool_calls,
                    mutating_tool_calls,
                    files_touched,
                    tokens_in,
                    tokens_out,
                    tokens_cache_read,
                    tokens_cache_write,
                    ts,
                    raw_json: line.to_string(),
                });
                turn_index += 1;
            }
            _ => {}
        }
    }
    out
}

/// 按 agent 分派解析一段(必为完整行)transcript 文本。`starting_turn_index`
/// 是这段文本第一条产出的 `ParsedTurn` 应该编到的 `turn_index`(调用方从
/// `conversations`/`conversation_turns` 已有数据算出,续接编号,不重置)。
pub fn parse_chunk(
    agent: AgentKind,
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codebuddy | AgentKind::Codex | AgentKind::Qoder | AgentKind::V8agent => {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_and_ai_turn_with_usage_and_tools() {
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,",
            "\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"u2\",\"timestamp\":200,\"message\":{",
            "\"role\":\"assistant\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20},",
            "\"content\":[{\"type\":\"thinking\",\"thinking\":\"...\"},",
            "{\"type\":\"text\",\"text\":\"好的，我来改。\"},",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"/r/README.md\"}}]}}\n"
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "改一下 README");
        assert_eq!(turns[0].message_key, "u1");
        assert_eq!(turns[0].ts, Some(100));
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "好的，我来改。");
        assert!(turns[1].thinking);
        assert_eq!(turns[1].tool_calls, 1);
        assert_eq!(turns[1].mutating_tool_calls, 1);
        assert_eq!(turns[1].files_touched, vec!["/r/README.md".to_string()]);
        assert_eq!(turns[1].tokens_in, 10);
        assert_eq!(turns[1].tokens_out, 20);
        assert_eq!(turns[1].message_key, "u2");
    }

    #[test]
    fn missing_uuid_falls_back_to_conversation_and_turn_index_key() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"无 uuid\"}}\n";
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 5);
        assert_eq!(turns[0].message_key, "conv1:5");
    }

    #[test]
    fn starting_turn_index_offsets_fallback_keys() {
        let text = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第一\"}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二\"}}\n",
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 10);
        assert_eq!(turns[0].message_key, "conv1:10");
        assert_eq!(turns[1].message_key, "conv1:11");
    }

    #[test]
    fn unsupported_agents_yield_empty() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"忽略\"}}\n";
        assert!(parse_chunk(AgentKind::Codex, text, "c", 0).is_empty());
        assert!(parse_chunk(AgentKind::Qoder, text, "c", 0).is_empty());
        assert!(parse_chunk(AgentKind::V8agent, text, "c", 0).is_empty());
    }

    #[test]
    fn last_complete_line_boundary_excludes_trailing_half_line() {
        assert_eq!(last_complete_line_boundary("a\nb\n"), 4);
        assert_eq!(last_complete_line_boundary("a\nb"), 2);
        assert_eq!(last_complete_line_boundary("no newline yet"), 0);
        assert_eq!(last_complete_line_boundary(""), 0);
    }
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcripts::parse`
Expected: 编译失败(`crates/dozerd/src/transcripts/` 目录/`lib.rs` 里还没挂 `transcripts` mod)。

为了让本 Task 能独立编译验证,临时在 `crates/dozerd/src/lib.rs` 追加:

```rust
pub mod transcripts {
    pub mod parse;
}
```

(Task 6 会把这行换成正式的三文件 `mod.rs`/`parse.rs`/`scan.rs` 结构,这里先用内联 `pub mod` 让本 Task 可独立验证。)

- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::parse`
Expected: PASS(5 个测试)。**如果 `parses_human_and_ai_turn_with_usage_and_tools` 因为真实 Claude JSONL 里 `uuid` 字段名/位置跟假设不同而需要调整解析代码,现在用本机真实文件核对一次再继续。**

- [x] **Step 4: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/parse.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): 新增 transcripts::parse——Claude 形状增量解析器"
```

---

## Task 4: `dozerd::transcripts::parse` —— CodeBuddy 形状 + 分派收尾

**Files:**
- Modify: `crates/dozerd/src/transcripts/parse.rs`

**Interfaces:**
- Consumes: Task 3 的 `ParsedTurn`、`fallback_key`、`parse_chunk` 分派骨架。
- Produces: `parse_chunk` 对 `AgentKind::Codebuddy` 分支从"返回空"改为真实解析。

CodeBuddy 每行顶层有稳定的 `id` 字段(已用 `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl` 实测确认,见该 fixture 第 1/2/3 行的 `id` 字段),`message_key` 直接取它,不需要退化。`timestamp` 字段同样是顶层数字(epoch ms,已实测确认)。

- [x] **Step 1: 写失败测试**

在 `parse.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn codebuddy_parses_real_fixture_sample_with_usage() {
        let text = include_str!("../../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 2, "1 用户消息 + 1 assistant 消息;快照行跳过");
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "reply with exactly one word: hello");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "hello");
        // fixture 里两条消息 id 不同,取到即视为通过(不用本机真实 fixture
        // 猜数值);关键是不再退化成 fallback_key。
        assert_ne!(turns[0].message_key, "conv1:0");
        assert_ne!(turns[1].message_key, "conv1:1");
        assert!(turns[0].ts.is_some());
    }

    #[test]
    fn codebuddy_joins_multiple_text_blocks_and_skips_snapshot() {
        let text = concat!(
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"input_text\",\"text\":\"第一段\"},",
            "{\"type\":\"input_text\",\"text\":\"第二段\"}]}\n",
            "{\"id\":\"m2\",\"type\":\"message\",\"role\":\"assistant\",\"content\":",
            "[{\"type\":\"output_text\",\"text\":\"回复一\"}]}\n",
            "{\"id\":\"m3\",\"type\":\"file-history-snapshot\"}\n",
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "第一段\n第二段");
        assert_eq!(turns[0].message_key, "m1");
        assert_eq!(turns[1].content, "回复一");
        assert_eq!(turns[1].message_key, "m2");
    }

    #[test]
    fn codebuddy_usage_and_tool_fields_stay_zero() {
        // CodeBuddy fixture 里没见过 tool_use 形状消息,不臆测其结构——
        // tool_calls/mutating_tool_calls/files_touched 恒零/空(同原
        // dozer-app usage.rs::parse_codebuddy_shaped_usage 的既有口径)。
        let text = concat!(
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"x\"}],",
            "\"providerData\":{\"usage\":{\"inputTokens\":5,\"outputTokens\":7}}}\n",
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns[0].tokens_in, 5);
        assert_eq!(turns[0].tokens_out, 7);
        assert_eq!(turns[0].tool_calls, 0);
        assert!(turns[0].files_touched.is_empty());
    }
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcripts::parse::codebuddy`
Expected: FAIL(`turns.len()` 断言失败,因为分派还是返回空)。

- [x] **Step 3: 实现 CodeBuddy 分支**

在 `parse.rs` 里,`parse_claude_shaped_chunk` 函数之后新增:

```rust
fn join_codebuddy_text_blocks(blocks: &[Value], kind: &str) -> String {
    let mut text = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) == Some(kind)
            && let Some(t) = b.get("text").and_then(|t| t.as_str())
        {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    text
}

fn parse_codebuddy_shaped_chunk(
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    let mut out = Vec::new();
    let mut turn_index = starting_turn_index;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let Some(blocks) = v.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(|t| t.as_u64());
        let message_key = v
            .get("id")
            .and_then(|i| i.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
        let usage = v.get("providerData").and_then(|p| p.get("usage"));
        let tokens_in = usage
            .and_then(|u| u.get("inputTokens"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let tokens_out = usage
            .and_then(|u| u.get("outputTokens"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        match v.get("role").and_then(|r| r.as_str()) {
            Some("user") => {
                let content = join_codebuddy_text_blocks(blocks, "input_text");
                if content.is_empty() {
                    continue;
                }
                out.push(ParsedTurn {
                    message_key,
                    role: "human".into(),
                    content,
                    ts,
                    tokens_in,
                    tokens_out,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            Some("assistant") => {
                out.push(ParsedTurn {
                    message_key,
                    role: "ai".into(),
                    content: join_codebuddy_text_blocks(blocks, "output_text"),
                    ts,
                    tokens_in,
                    tokens_out,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            _ => {}
        }
    }
    out
}
```

把 `parse_chunk` 的分派改成:

```rust
pub fn parse_chunk(
    agent: AgentKind,
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codebuddy => {
            parse_codebuddy_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codex | AgentKind::Qoder | AgentKind::V8agent => Vec::new(),
    }
}
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::parse`
Expected: PASS(全部 8 个测试)

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/parse.rs
git commit -m "feat(dozerd): transcripts::parse 补齐 CodeBuddy 形状解析"
```

---

## Task 5: `dozerd::transcripts::scan` —— 目录发现(启动回填用)

**Files:**
- Create: `crates/dozerd/src/transcripts/scan.rs`
- Modify: `crates/dozerd/src/lib.rs`(把 Task 3 临时的内联 `pub mod transcripts { pub mod parse; }` 换成正式三文件结构,见 Step 3)

**Interfaces:**
- Produces: `discover_all_transcript_files() -> Vec<(AgentKind, PathBuf)>`——扫描三家 agent 的 `<home>/<agent_root>/projects/*/*.jsonl`,返回 `(agent, 文件路径)` 扁平列表,供 `dozerd` 启动回填用。

- [x] **Step 1: 写失败测试**

创建 `crates/dozerd/src/transcripts/scan.rs`:

```rust
//! 启动回填用的目录发现:扫描三家 agent 的存储根目录下所有项目子目录,
//! 列出全部 `.jsonl` 文件。跟 `dozer_core::agent_paths` 的方向相反——
//! 那边是"已知 cwd → 算出该项目的存储目录"(单项目查询用),这里是
//! "不知道有哪些项目 → 枚举存储根目录下所有子目录"(启动时全量摄取用)。

use dozer_core::agent_paths::home_dir;
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

const AGENT_ROOTS: [(AgentKind, &str); 3] = [
    (AgentKind::Claude, ".claude"),
    (AgentKind::Codebuddy, ".codebuddy"),
    (AgentKind::Opencode, ".dozer/agents/opencode"),
];

fn jsonl_files_in(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .collect()
}

pub fn discover_all_transcript_files() -> Vec<(AgentKind, PathBuf)> {
    discover_all_transcript_files_in(&home_dir())
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn discover_all_transcript_files_in(home: &std::path::Path) -> Vec<(AgentKind, PathBuf)> {
    let mut out = Vec::new();
    for (agent, root) in AGENT_ROOTS {
        let projects_dir = home.join(root).join("projects");
        let Ok(rd) = std::fs::read_dir(&projects_dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            for f in jsonl_files_in(&dir) {
                out.push((agent, f));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_jsonl_files_across_three_agent_roots() {
        let home = tempfile::tempdir().unwrap();
        let claude_proj = home.path().join(".claude/projects/-a-b-c");
        let codebuddy_proj = home.path().join(".codebuddy/projects/a-b-c");
        std::fs::create_dir_all(&claude_proj).unwrap();
        std::fs::create_dir_all(&codebuddy_proj).unwrap();
        std::fs::write(claude_proj.join("s1.jsonl"), "{}").unwrap();
        std::fs::write(codebuddy_proj.join("s2.jsonl"), "{}").unwrap();
        std::fs::write(claude_proj.join("not-jsonl.txt"), "x").unwrap();

        let mut found = discover_all_transcript_files_in(home.path());
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, AgentKind::Codebuddy);
        assert_eq!(found[1].0, AgentKind::Claude);
    }

    #[test]
    fn missing_agent_root_yields_no_entries_for_it() {
        let home = tempfile::tempdir().unwrap();
        assert!(discover_all_transcript_files_in(home.path()).is_empty());
    }
}
```

- [x] **Step 2: 挂进 crate(正式三文件结构)**

把 `crates/dozerd/src/lib.rs` 里 Task 3 加的临时内联 `pub mod transcripts { pub mod parse; }` 删掉,新建 `crates/dozerd/src/transcripts/mod.rs`:

```rust
//! agent 对话/用量摄取管线(spec 2026-08-20)。

pub mod parse;
pub mod scan;
```

`lib.rs` 里改成:

```rust
pub mod transcripts;
```

(跟 `pub mod acceptance;`/`pub mod bookmarks;` 等现有模块声明同级、同风格。)

- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::scan`
Expected: PASS(2 个测试);同时确认 Task 3/4 的 `transcripts::parse` 测试仍然全绿(`cargo test -p dozerd transcripts::`)。

- [x] **Step 4: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/scan.rs crates/dozerd/src/transcripts/mod.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): 新增 transcripts::scan——启动回填用的目录发现"
```

---

## Task 6: `TranscriptStore` —— 表结构 + `ingest_session` 核心逻辑

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Consumes: Task 3/4 的 `parse::{ParsedTurn, parse_chunk, last_complete_line_boundary}`。
- Produces: `TranscriptStore::open(path: &Path) -> Result<Self>`、`TranscriptStore::ingest_session(&self, agent: AgentKind, file_path: &Path) -> Result<()>`(内部从 `file_path` 派生 `conversation_id` = 文件 stem、`dir` = 父目录字符串)。

这是本计划风险最集中的 Task——增量读取(半行不消费)、截断检测、`(conversation_id, message_key)` upsert。

- [x] **Step 1: 写失败测试**

在 `crates/dozerd/src/transcripts/mod.rs` 追加(紧跟 `pub mod parse; pub mod scan;` 之后):

```rust
use anyhow::{Context, Result};
use dozer_core::protocol::AgentKind;
use rusqlite::{Connection, OptionalExtension, params};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn agent_to_str(a: AgentKind) -> &'static str {
    match a {
        AgentKind::Unknown => "unknown",
        AgentKind::Claude => "claude",
        AgentKind::Codebuddy => "codebuddy",
        AgentKind::Opencode => "opencode",
        AgentKind::Codex => "codex",
        AgentKind::Qoder => "qoder",
        AgentKind::Kilo => "kilo",
        AgentKind::V8agent => "v8agent",
    }
}

fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        "qoder" => AgentKind::Qoder,
        "kilo" => AgentKind::Kilo,
        "v8agent" => AgentKind::V8agent,
        _ => AgentKind::Unknown,
    }
}

pub struct TranscriptStore {
    conn: Mutex<Connection>,
}

impl TranscriptStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                conversation_id TEXT PRIMARY KEY,
                agent_kind TEXT NOT NULL,
                dir TEXT NOT NULL,
                file_path TEXT NOT NULL,
                title TEXT,
                first_ts INTEGER NOT NULL,
                last_ts INTEGER NOT NULL,
                turn_count INTEGER NOT NULL DEFAULT 0,
                parsed_offset INTEGER NOT NULL DEFAULT 0,
                file_size_at_parse INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_conversations_dir ON conversations(dir);
            CREATE TABLE IF NOT EXISTS conversation_turns (
                conversation_id TEXT NOT NULL,
                turn_index INTEGER NOT NULL,
                message_key TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tools_summary TEXT NOT NULL DEFAULT '[]',
                thinking INTEGER NOT NULL DEFAULT 0,
                ts INTEGER,
                tool_calls INTEGER NOT NULL DEFAULT 0,
                mutating_tool_calls INTEGER NOT NULL DEFAULT 0,
                files_touched TEXT NOT NULL DEFAULT '[]',
                tokens_in INTEGER NOT NULL DEFAULT 0,
                tokens_out INTEGER NOT NULL DEFAULT 0,
                tokens_cache_read INTEGER NOT NULL DEFAULT 0,
                tokens_cache_write INTEGER NOT NULL DEFAULT 0,
                raw_json TEXT NOT NULL,
                PRIMARY KEY (conversation_id, message_key)
            );
            CREATE INDEX IF NOT EXISTS idx_turns_order
                ON conversation_turns(conversation_id, turn_index);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn ingest_incremental_then_append() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let file = fixture(
            tmp.path(),
            "s1.jsonl",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"第一句\"}}\n",
        );

        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "第一句");

        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        use std::io::Write;
        writeln!(
            f,
            "{{\"type\":\"assistant\",\"uuid\":\"u2\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"回复\"}}]}}}}"
        )
        .unwrap();
        drop(f);

        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        assert_eq!(turns.len(), 2, "增量摄取应该只新增第二条,不重复第一条");
        assert_eq!(turns[1].content, "回复");
    }

    #[test]
    fn half_written_line_is_not_consumed_until_completed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let file = fixture(tmp.path(), "s1.jsonl", "{\"type\":\"user\",\"uuid\":\"u1\"");
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        assert!(store.get_conversation_turns("s1", -1, 100).unwrap().is_empty());

        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"补完了\"}}\n",
        )
        .unwrap();
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "补完了");
    }

    #[test]
    fn truncated_file_resets_cursor_and_upserts_without_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let file = fixture(
            tmp.path(),
            "s1.jsonl",
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一\"}}\n",
                "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"二\"}}\n",
            ),
        );
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        assert_eq!(store.get_conversation_turns("s1", -1, 100).unwrap().len(), 2);

        // 截断成更短的内容(模拟文件被重写)。
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一(改过)\"}}\n",
        )
        .unwrap();
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        // u1 被 upsert 覆盖成新内容;u2 是"孤儿行"——按 spec"不做孤儿行
        // 清理"策略保留,不删除。
        let u1 = turns.iter().find(|t| t.content.contains("一(改过)"));
        assert!(u1.is_some(), "截断后重新解析应该覆盖 u1 的内容");
    }
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcripts::tests`
Expected: 编译失败(`ingest_session`/`get_conversation_turns` 还不存在)。

- [x] **Step 3: 实现 `ingest_session`(先内联一个最小 `get_conversation_turns` 供测试用,完整版见 Task 8)**

在 `impl TranscriptStore` 块内(`open` 方法之后)新增:

```rust
    /// 摄取(或增量续摄取)一份 transcript 文件。`conversation_id` 从
    /// 文件名(不含扩展名)派生——这是磁盘上天然唯一的会话标识,不依赖
    /// dozerd 自己的 PTY session id(两者是不同的 id 空间:后者只在
    /// agent 活着时存在,回填历史文件时根本没有)。
    pub fn ingest_session(&self, agent: AgentKind, file_path: &Path) -> Result<()> {
        let conversation_id = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if conversation_id.is_empty() {
            anyhow::bail!("无法从文件名派生 conversation_id: {}", file_path.display());
        }
        let dir = file_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_path_str = file_path.to_string_lossy().into_owned();

        let file_size = std::fs::metadata(file_path)
            .with_context(|| format!("读取文件元信息失败: {}", file_path.display()))?
            .len();

        let mut conn = self.conn.lock().expect("db lock");
        let existing: Option<(u64, Option<u64>, Option<String>)> = conn
            .query_row(
                "SELECT parsed_offset, first_ts, title FROM conversations WHERE conversation_id = ?1",
                [&conversation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (mut parsed_offset, existing_first_ts, existing_title) =
            existing.unwrap_or((0, None, None));
        if file_size < parsed_offset {
            // 文件被截断/重写:只回退读取位置,不删除已入库的行(spec
            // "不做孤儿行清理")。
            parsed_offset = 0;
        }

        let mut raw = std::fs::File::open(file_path)
            .with_context(|| format!("打开文件失败: {}", file_path.display()))?;
        raw.seek(SeekFrom::Start(parsed_offset))?;
        let mut buf = String::new();
        raw.read_to_string(&mut buf)
            .with_context(|| format!("读取增量内容失败: {}", file_path.display()))?;
        let consumable = parse::last_complete_line_boundary(&buf);
        if consumable == 0 {
            // 没有新的完整行(半行未写完/没有新增内容),什么都不做。
            return Ok(());
        }
        let chunk = &buf[..consumable];

        let starting_turn_index: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM conversation_turns
                 WHERE conversation_id = ?1",
                [&conversation_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let turns = parse::parse_chunk(agent, chunk, &conversation_id, starting_turn_index);

        let tx = conn.transaction()?;
        let mut turn_index = starting_turn_index;
        let mut first_human_content: Option<String> = None;
        for t in &turns {
            if first_human_content.is_none() && t.role == "human" {
                first_human_content = Some(t.content.clone());
            }
            tx.execute(
                "INSERT INTO conversation_turns
                 (conversation_id, turn_index, message_key, role, content, tools_summary,
                  thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                 ON CONFLICT(conversation_id, message_key) DO UPDATE SET
                    turn_index=excluded.turn_index, role=excluded.role,
                    content=excluded.content, tools_summary=excluded.tools_summary,
                    thinking=excluded.thinking, ts=excluded.ts,
                    tool_calls=excluded.tool_calls,
                    mutating_tool_calls=excluded.mutating_tool_calls,
                    files_touched=excluded.files_touched,
                    tokens_in=excluded.tokens_in, tokens_out=excluded.tokens_out,
                    tokens_cache_read=excluded.tokens_cache_read,
                    tokens_cache_write=excluded.tokens_cache_write,
                    raw_json=excluded.raw_json",
                params![
                    conversation_id,
                    turn_index,
                    t.message_key,
                    t.role,
                    t.content,
                    serde_json::to_string(&t.tools_summary).unwrap_or_else(|_| "[]".into()),
                    t.thinking as i64,
                    t.ts,
                    t.tool_calls,
                    t.mutating_tool_calls,
                    serde_json::to_string(&t.files_touched).unwrap_or_else(|_| "[]".into()),
                    t.tokens_in,
                    t.tokens_out,
                    t.tokens_cache_read,
                    t.tokens_cache_write,
                    t.raw_json,
                ],
            )?;
            turn_index += 1;
        }

        let turn_count: u32 = tx.query_row(
            "SELECT COUNT(*) FROM conversation_turns WHERE conversation_id = ?1",
            [&conversation_id],
            |row| row.get(0),
        )?;
        let last_ts_from_turns: Option<u64> = turns.iter().filter_map(|t| t.ts).max();
        let now = now_ms();
        let new_parsed_offset = parsed_offset + consumable as u64;
        let title = existing_title.or(first_human_content.map(|c| c.chars().take(80).collect()));
        let first_ts = existing_first_ts.unwrap_or(now);
        tx.execute(
            "INSERT INTO conversations
             (conversation_id, agent_kind, dir, file_path, title, first_ts, last_ts,
              turn_count, parsed_offset, file_size_at_parse)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(conversation_id) DO UPDATE SET
                dir=excluded.dir, file_path=excluded.file_path,
                title=COALESCE(conversations.title, excluded.title),
                last_ts=excluded.last_ts, turn_count=excluded.turn_count,
                parsed_offset=excluded.parsed_offset,
                file_size_at_parse=excluded.file_size_at_parse",
            params![
                conversation_id,
                agent_to_str(agent),
                dir,
                file_path_str,
                title,
                first_ts,
                last_ts_from_turns.unwrap_or(now),
                turn_count,
                new_parsed_offset,
                file_size,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 临时最小实现,供本 Task 测试用;Task 8 会替换成带 keyset 分页的
    /// 完整版本(签名不变,调用方不受影响)。
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        _after_turn_index: i64,
        _limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT turn_index, role, content, tools_summary, thinking, ts
             FROM conversation_turns WHERE conversation_id = ?1 ORDER BY turn_index ASC",
        )?;
        let rows = stmt.query_map([conversation_id], |row| {
            let tools_json: String = row.get(3)?;
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tools_summary: serde_json::from_str(&tools_json).unwrap_or_default(),
                thinking: row.get::<_, i64>(4)? != 0,
                ts: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
```

在文件顶部把 `#[allow(dead_code)]` 之类的告警抑制不需要加——`agent_from_str` 目前尚未被调用(留给 Task 7/8 用),先允许 clippy 报 `dead_code` 警告并在下一 Task 消掉;若本 Task 单独跑 `clippy -- -D warnings` 会因 `agent_from_str` 未使用而失败,**在本 Task 结尾临时给它加 `#[allow(dead_code)]`,Task 7 用到后删掉这个 allow**。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::tests`
Expected: PASS(3 个测试)

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): TranscriptStore 表结构 + ingest_session 增量摄取核心逻辑"
```

---

## Task 7: `TranscriptStore::list_conversations`

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Consumes: Task 2 的 `dozer_core::agent_paths::{claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in, home_dir}`;Task 6 的 `agent_to_str`/`agent_from_str`。
- Produces: `TranscriptStore::list_conversations(&self, cwd: &str, agent: Option<AgentKind>, limit: u32, offset: u32) -> Result<Vec<ConversationSummary>>`。

`cwd` → 该项目在各 agent 下的存储目录(`dir` 列的值),用 `dozer_core::agent_paths` 现算,不做"目录名倒推 cwd"这种有损逆运算(spec 设计决定,见计划撰写时的讨论)。`agent` 非空时只查该家对应的一个目录;为空时三家都查、结果按 `last_ts` 倒序合并。

- [x] **Step 1: 写失败测试**

在 `mod.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn list_conversations_merges_three_agents_sorted_by_last_ts() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        let codebuddy_dir = dozer_core::agent_paths::codebuddy_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codebuddy_dir).unwrap();
        let f1 = fixture(
            &claude_dir,
            "a.jsonl",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"claude 对话\"}}\n",
        );
        let f2 = fixture(
            &codebuddy_dir,
            "b.jsonl",
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"codebuddy 对话\"}]}\n",
        );
        store.ingest_session(AgentKind::Claude, &f1).unwrap();
        store.ingest_session(AgentKind::Codebuddy, &f2).unwrap();

        let list = store
            .list_conversations_in(home.path(), "/proj", None, 10, 0)
            .unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|c| c.agent == AgentKind::Claude));
        assert!(list.iter().any(|c| c.agent == AgentKind::Codebuddy));

        let claude_only = store
            .list_conversations_in(home.path(), "/proj", Some(AgentKind::Claude), 10, 0)
            .unwrap();
        assert_eq!(claude_only.len(), 1);
        assert_eq!(claude_only[0].agent, AgentKind::Claude);
    }

    #[test]
    fn list_conversations_respects_limit_and_offset() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        for i in 0..3 {
            let f = fixture(
                &claude_dir,
                &format!("s{i}.jsonl"),
                &format!(
                    "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"message\":{{\"role\":\"user\",\"content\":\"第{i}条\"}}}}\n"
                ),
            );
            store.ingest_session(AgentKind::Claude, &f).unwrap();
        }
        let page = store
            .list_conversations_in(home.path(), "/proj", None, 2, 0)
            .unwrap();
        assert_eq!(page.len(), 2);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcripts::tests::list_conversations`
Expected: 编译失败(`list_conversations_in` 不存在)。

- [x] **Step 3: 实现**

在 `impl TranscriptStore` 块内新增(`ingest_session` 之后):

```rust
    /// 生产入口,内部用 `dozer_core::agent_paths::home_dir()`。
    pub fn list_conversations(
        &self,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<dozer_core::protocol::ConversationSummary>> {
        self.list_conversations_in(&dozer_core::agent_paths::home_dir(), cwd, agent, limit, offset)
    }

    /// `home` 显式传入版本,测试用。
    pub fn list_conversations_in(
        &self,
        home: &Path,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<dozer_core::protocol::ConversationSummary>> {
        use dozer_core::agent_paths::{
            claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in,
        };
        let cwd_path = Path::new(cwd);
        let candidate_dirs: Vec<(AgentKind, String)> = match agent {
            Some(a) => {
                let dir = match a {
                    AgentKind::Claude => claude_project_dir_in(home, cwd_path),
                    AgentKind::Codebuddy => codebuddy_project_dir_in(home, cwd_path),
                    AgentKind::Opencode => opencode_project_dir_in(home, cwd_path),
                    _ => return Ok(Vec::new()),
                };
                vec![(a, dir.to_string_lossy().into_owned())]
            }
            None => vec![
                (AgentKind::Claude, claude_project_dir_in(home, cwd_path).to_string_lossy().into_owned()),
                (AgentKind::Codebuddy, codebuddy_project_dir_in(home, cwd_path).to_string_lossy().into_owned()),
                (AgentKind::Opencode, opencode_project_dir_in(home, cwd_path).to_string_lossy().into_owned()),
            ],
        };

        let conn = self.conn.lock().expect("db lock");
        let mut out = Vec::new();
        for (_, dir) in &candidate_dirs {
            let mut stmt = conn.prepare(
                "SELECT conversation_id, agent_kind, file_path, title, first_ts, last_ts, turn_count
                 FROM conversations WHERE dir = ?1 ORDER BY last_ts DESC",
            )?;
            let rows = stmt.query_map([dir], |row| {
                let agent_kind: String = row.get(1)?;
                Ok(dozer_core::protocol::ConversationSummary {
                    conversation_id: row.get(0)?,
                    agent: agent_from_str(&agent_kind),
                    file_path: row.get(2)?,
                    title: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    first_ts: row.get(4)?,
                    last_ts: row.get(5)?,
                    turn_count: row.get(6)?,
                })
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.last_ts));
        let start = (offset as usize).min(out.len());
        let end = (start + limit as usize).min(out.len());
        Ok(out[start..end].to_vec())
    }
```

删掉 Task 6 结尾给 `agent_from_str` 加的 `#[allow(dead_code)]`(现在用上了)。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::tests`
Expected: PASS(全部测试,包括之前的)

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): TranscriptStore::list_conversations"
```

---

## Task 8: `TranscriptStore::get_conversation_turns` —— keyset 分页正式版

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Produces: 替换 Task 6 的临时 `get_conversation_turns`,签名不变,新增 `after_turn_index`/`limit` 真正生效。

- [x] **Step 1: 写失败测试**

在 `mod.rs` 测试模块新增:

```rust
    #[test]
    fn get_conversation_turns_paginates_by_keyset() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let mut jsonl = String::new();
        for i in 0..5 {
            jsonl.push_str(&format!(
                "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"message\":{{\"role\":\"user\",\"content\":\"第{i}条\"}}}}\n"
            ));
        }
        let file = fixture(tmp.path(), "s1.jsonl", &jsonl);
        store.ingest_session(AgentKind::Claude, &file).unwrap();

        let first_page = store.get_conversation_turns("s1", -1, 2).unwrap();
        assert_eq!(first_page.len(), 2);
        assert_eq!(first_page[0].turn_index, 0);
        assert_eq!(first_page[1].turn_index, 1);

        let last_seen = first_page[1].turn_index;
        let second_page = store.get_conversation_turns("s1", last_seen, 2).unwrap();
        assert_eq!(second_page.len(), 2);
        assert_eq!(second_page[0].turn_index, 2);
        assert_eq!(second_page[1].turn_index, 3);
    }
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcripts::tests::get_conversation_turns_paginates_by_keyset`
Expected: FAIL(临时实现忽略了 `after_turn_index`/`limit`,一次性返回全部 5 条)。

- [x] **Step 3: 替换实现**

把 Task 6 里的临时 `get_conversation_turns` 方法体换成:

```rust
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT turn_index, role, content, tools_summary, thinking, ts
             FROM conversation_turns
             WHERE conversation_id = ?1 AND turn_index > ?2
             ORDER BY turn_index ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![conversation_id, after_turn_index, limit], |row| {
            let tools_json: String = row.get(3)?;
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tools_summary: serde_json::from_str(&tools_json).unwrap_or_default(),
                thinking: row.get::<_, i64>(4)? != 0,
                ts: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::tests`
Expected: PASS(全部测试)

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): get_conversation_turns 改为真正的 keyset 分页"
```

---

## Task 9: `TranscriptStore::get_usage_summary` —— message_key 去重聚合

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Produces: `TranscriptStore::get_usage_summary(&self, cwd: &str, since_ts: Option<u64>) -> Result<Vec<(ConversationSummary, UsagePayload)>>`。

去重规则(spec"用量去重"一节的具体实现):同一个 `message_key` 若出现在多个会话里(fork/resume 复制),只有 **`first_ts` 最早的那个会话**对它拥有"计入用量"的所有权,其余会话对该 `message_key` 的用量计为零(但对话展示层不受影响——`get_conversation_turns` 仍然按 `conversation_id` 独立查询,不做这层去重)。

**平局判定**:启动回填(Task 13)会在很短时间内连续摄取一大批历史文件,多个会话拿到的 `first_ts`(=首次摄取时的 `now_ms()`)完全相同是常态,不是罕见边界——单用 `MIN(first_ts)` 判所有权在平局时会让多个会话同时"自认为"是所有者,导致同一条 `message_key` 被重复计入而不是漏计(比只算一次更糟)。所有权判定必须在 `first_ts` 相等时有确定性的次级排序键,下面的实现用 `first_ts` 拼 `conversation_id` 的字符串联合键(`printf('%020lld|', first_ts) || conversation_id`)做 `MIN`,保证任何时候只有唯一一个会话胜出。

- [x] **Step 1: 写失败测试**

```rust
    #[test]
    fn get_usage_summary_dedupes_forked_message_key_by_earliest_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        // 原始会话:1 条 assistant 消息,10 input token。
        let original = fixture(
            &dir,
            "original.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"shared-1\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5},\"content\":[]}}\n",
        );
        store.ingest_session(AgentKind::Claude, &original).unwrap();

        // fork 出的新会话:复制了同一条 uuid 消息 + 自己新增一条。
        std::thread::sleep(std::time::Duration::from_millis(5));
        let forked = fixture(
            &dir,
            "forked.jsonl",
            concat!(
                "{\"type\":\"assistant\",\"uuid\":\"shared-1\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5},\"content\":[]}}\n",
                "{\"type\":\"assistant\",\"uuid\":\"new-1\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1},\"content\":[]}}\n",
            ),
        );
        store.ingest_session(AgentKind::Claude, &forked).unwrap();

        let rows = store.get_usage_summary_in(home.path(), "/proj", None).unwrap();
        assert_eq!(rows.len(), 2);
        let original_row = rows.iter().find(|(c, _)| c.conversation_id == "original").unwrap();
        let forked_row = rows.iter().find(|(c, _)| c.conversation_id == "forked").unwrap();
        assert_eq!(original_row.1.tokens_in, 10, "原始会话拥有 shared-1 的用量");
        assert_eq!(
            forked_row.1.tokens_in, 3,
            "forked 会话里复制来的 shared-1 不重复计入,只有自己新增的 new-1 计入"
        );
    }

    #[test]
    fn get_usage_summary_breaks_first_ts_ties_deterministically() {
        // 模拟启动回填:两个会话在同一毫秒内首次摄取,first_ts 完全相同。
        // 平局判定必须让恰好一个会话拥有共享的 message_key,不能两个都算
        // (那样比不去重更糟——变成重复计入)。
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        let a = fixture(
            &dir,
            "a.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"shared-tie\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0},\"content\":[]}}\n",
        );
        let b = fixture(
            &dir,
            "b.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"shared-tie\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0},\"content\":[]}}\n",
        );
        // 不 sleep——刻意让两者的 first_ts 尽量接近/相同,复现平局场景。
        store.ingest_session(AgentKind::Claude, &a).unwrap();
        store.ingest_session(AgentKind::Claude, &b).unwrap();

        let rows = store.get_usage_summary_in(home.path(), "/proj", None).unwrap();
        let total_tokens_in: u64 = rows.iter().map(|(_, u)| u.tokens_in).sum();
        assert_eq!(
            total_tokens_in, 10,
            "无论 first_ts 是否平局,shared-tie 只能被恰好一个会话计入一次"
        );
    }
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcripts::tests::get_usage_summary_dedupes`
Expected: 编译失败(`get_usage_summary_in` 不存在)。

- [x] **Step 3: 实现**

```rust
    pub fn get_usage_summary(
        &self,
        cwd: &str,
        since_ts: Option<u64>,
    ) -> Result<Vec<(dozer_core::protocol::ConversationSummary, dozer_core::protocol::UsagePayload)>> {
        self.get_usage_summary_in(&dozer_core::agent_paths::home_dir(), cwd, since_ts)
    }

    pub fn get_usage_summary_in(
        &self,
        home: &Path,
        cwd: &str,
        since_ts: Option<u64>,
    ) -> Result<Vec<(dozer_core::protocol::ConversationSummary, dozer_core::protocol::UsagePayload)>> {
        let conversations = self.list_conversations_in(home, cwd, None, u32::MAX, 0)?;
        let conn = self.conn.lock().expect("db lock");
        let mut out = Vec::new();
        for c in conversations {
            if let Some(since) = since_ts {
                if c.last_ts < since {
                    continue;
                }
            }
            let mut stmt = conn.prepare(
                "WITH owners AS (
                    SELECT t.message_key,
                           MIN(printf('%020lld|', c2.first_ts) || c2.conversation_id) AS owner_key
                    FROM conversation_turns t
                    JOIN conversations c2 ON c2.conversation_id = t.conversation_id
                    GROUP BY t.message_key
                 )
                 SELECT t.role, t.tool_calls, t.mutating_tool_calls, t.files_touched,
                        t.tokens_in, t.tokens_out, t.tokens_cache_read, t.tokens_cache_write
                 FROM conversation_turns t
                 JOIN conversations c1 ON c1.conversation_id = t.conversation_id
                 JOIN owners o ON o.message_key = t.message_key
                 WHERE t.conversation_id = ?1
                   AND (printf('%020lld|', c1.first_ts) || c1.conversation_id) = o.owner_key",
            )?;
            let rows = stmt.query_map([&c.conversation_id], |row| {
                let files_json: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    files_json,
                    row.get::<_, u64>(4)?,
                    row.get::<_, u64>(5)?,
                    row.get::<_, u64>(6)?,
                    row.get::<_, u64>(7)?,
                ))
            })?;
            let mut payload = dozer_core::protocol::UsagePayload::default();
            for r in rows {
                let (_role, tool_calls, mutating, files_json, tin, tout, tcr, tcw) = r?;
                payload.turns += 1;
                payload.tool_calls += tool_calls;
                payload.mutating_tool_calls += mutating;
                payload.tokens_in += tin;
                payload.tokens_out += tout;
                payload.tokens_cache_read += tcr;
                payload.tokens_cache_write += tcw;
                if let Ok(files) = serde_json::from_str::<Vec<String>>(&files_json) {
                    payload.files_touched.extend(files);
                }
            }
            out.push((c, payload));
        }
        Ok(out)
    }
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts::tests`
Expected: PASS(全部测试)

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): get_usage_summary——message_key 全局去重(最早会话拥有)"
```

---

## Task 10: `dozerd::server` —— 挂 `TranscriptStore`,处理三个新 Request

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes: Task 6-9 的 `TranscriptStore`(`open`/`list_conversations`/`get_conversation_turns`/`get_usage_summary`)。
- Produces: `serve(...)` 新增 `transcripts: Arc<TranscriptStore>` 形参;`handle_conn` 处理 `Request::ListConversations`/`GetConversationTurns`/`GetUsageSummary`。

本 Task 先只接线"查询"路径,不接摄取触发(留 Task 11/12)。

- [x] **Step 1: 写失败测试**

在 `crates/dozerd/src/server.rs` 的 `#[cfg(test)] mod tests`(若没有则新增)里加一个纯函数级测试,验证 `serve` 签名可用即可——实际的端到端 UDS 测试留给 `dozer-client` 那边的 `against_real_daemon.rs`(Task 15 会补)。这里先写一个编译期占位测试确认新字段能正确传递:

```rust
    #[test]
    fn transcript_store_field_compiles_into_serve_signature() {
        // 编译期检查:确认 `serve` 函数签名接受 `Arc<TranscriptStore>`。
        fn _assert_signature(
            socket: &std::path::Path,
            registry: std::sync::Arc<crate::registry::SessionRegistry>,
            store: std::sync::Arc<crate::acceptance::AcceptanceStore>,
            projects: std::sync::Arc<crate::projects::ProjectStore>,
            bookmarks: std::sync::Arc<crate::bookmarks::BookmarkStore>,
            transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
        ) {
            let _ = crate::server::serve(socket, registry, store, projects, bookmarks, transcripts);
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd transcript_store_field_compiles_into_serve_signature`
Expected: 编译失败(`serve` 还没有 `transcripts` 形参)。

- [x] **Step 3: 实现**

在 `crates/dozerd/src/server.rs`:

1. `serve` 函数签名新增一个形参 `transcripts: Arc<crate::transcripts::TranscriptStore>`(紧跟 `bookmarks` 之后),函数体里 `let transcripts = transcripts;`(不需要额外 clone 出 loop 外的那份,直接沿用参数);在 `tokio::spawn` 前补一行 `let transcripts = transcripts.clone();`,并把它加进 `handle_conn(...)` 调用参数列表。
2. `handle_conn` 函数签名同步新增 `transcripts: Arc<crate::transcripts::TranscriptStore>` 形参。
3. 在 `match req { ... }` 里(找 `Request::HookEvent` 分支所在的大 `match`)新增三个分支:

```rust
                        Request::ListConversations { cwd, agent, limit, offset } => {
                            match transcripts.list_conversations(&cwd, agent, limit, offset) {
                                Ok(conversations) => Reply::Conversations { conversations },
                                Err(e) => Reply::Error { message: format!("列对话失败: {e}") },
                            }
                        }
                        Request::GetConversationTurns { conversation_id, after_turn_index, limit } => {
                            match transcripts.get_conversation_turns(&conversation_id, after_turn_index, limit) {
                                Ok(turns) => Reply::ConversationTurns { conversation_id, turns },
                                Err(e) => Reply::Error { message: format!("查询回合失败: {e}") },
                            }
                        }
                        Request::GetUsageSummary { cwd, since_ts } => {
                            match transcripts.get_usage_summary(&cwd, since_ts) {
                                Ok(rows) => Reply::UsageSummary { rows },
                                Err(e) => Reply::Error { message: format!("查询用量失败: {e}") },
                            }
                        }
```

在 `crates/dozerd/src/main.rs`,在构造 `bookmarks` 之后新增:

```rust
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

`serve(...)` 调用点补上 `transcripts` 实参(紧跟 `bookmarks` 之后)。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo build -p dozerd && cargo test -p dozerd`
Expected: PASS

- [x] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs
git commit -m "feat(dozerd): server 接线 TranscriptStore,处理三个查询 Request"
```

---

## Task 11: hook 快通道摄取触发

**Files:**
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: Task 10 已接线的 `transcripts` 参数;`extract_transcript_path`(既有函数)。

- [ ] **Step 1: 写失败测试**

在 `server.rs` 测试模块新增(需要构造一个真实的 sqlite 临时库 + fixture 文件,验证 `Request::HookEvent` 处理后触发了摄取)。先看现有 `handle_conn`/`serve` 测试是怎么搭 harness 的(若已有类似"发一个 Request 到内存 pipe,断言 Reply"的测试模式,复用它);若没有端到端 harness,改为对"HookEvent 分支触发摄取"这一段抽出的小函数单独测试:

```rust
    #[test]
    fn hook_event_with_transcript_path_triggers_ingest() {
        let tmp = tempfile::tempdir().unwrap();
        let transcripts = std::sync::Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let file = tmp.path().join("s1.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
        )
        .unwrap();
        let data = serde_json::json!({"transcript_path": file.to_string_lossy()});

        maybe_ingest_from_hook_data(&transcripts, dozer_core::protocol::AgentKind::Claude, &data);

        let turns = transcripts.get_conversation_turns("s1", -1, 10).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "你好");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd hook_event_with_transcript_path_triggers_ingest`
Expected: 编译失败(`maybe_ingest_from_hook_data` 不存在)。

- [ ] **Step 3: 实现**

在 `server.rs` 里 `extract_transcript_path` 函数之后新增:

```rust
/// hook 事件带 `transcript_path` 时触发一次增量摄取。同步执行(不额外
/// spawn 一个 task)——`ingest_session` 内部是"读几行新增内容+写 sqlite",
/// 单会话单文件量级下是毫秒级操作,没必要为它另起异步任务增加复杂度;
/// 摄取失败只记 warn,不影响本次 hook 事件其余处理(设置 agent/状态仍然
/// 照常进行)。
fn maybe_ingest_from_hook_data(
    transcripts: &crate::transcripts::TranscriptStore,
    agent: dozer_core::protocol::AgentKind,
    data: &serde_json::Value,
) {
    let Some(path) = extract_transcript_path(data) else {
        return;
    };
    if let Err(e) = transcripts.ingest_session(agent, std::path::Path::new(path)) {
        tracing::warn!(error = %e, %path, "hook 触发的对话摄取失败");
    }
}
```

在 `handle_conn` 的 `Request::HookEvent { session_id, agent, event, ts_ms, data }` 分支里,`Some(s) => { ... }` 块的末尾(`s.set_agent_state(...)` 调用之后)新增一行:

```rust
                                    maybe_ingest_from_hook_data(&transcripts, agent, &data);
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd hook_event_with_transcript_path_triggers_ingest`
Expected: PASS

- [ ] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/server.rs
git commit -m "feat(dozerd): hook 事件带 transcript_path 时触发增量摄取"
```

---

## Task 12: 待命态兜底摄取触发

**Files:**
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: 既有 `agent_state_for`;`SessionHandle::info()`(读旧状态)。

在 `agent_state_for(&event)` 算出新状态之前先读一次该 session 当前的 `agent_state`,若新状态是 `Idle`/`AwaitingInput` 且与旧状态不同(真正发生了状态转换,不是同一状态的重复事件),且该 session 已知 `transcript_path`,则额外触发一次摄取(补上 hook 事件本身可能没带 `transcript_path` 或文件当时还没写完整的情况)。

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn idle_state_transition_triggers_ingest_backstop() {
        use dozer_core::protocol::AgentState;
        let tmp = tempfile::tempdir().unwrap();
        let transcripts = std::sync::Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let file = tmp.path().join("s1.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"待命态触发\"}}\n",
        )
        .unwrap();

        // 模拟"hook 事件本身没带 transcript_path"(data 为空对象),但
        // 该 session 此前已经知道 transcript_path 了,状态从 Running
        // 转成 TurnEnded(→ Idle 映射前置状态,这里直接测
        // `maybe_ingest_on_state_transition` 本体)。
        maybe_ingest_on_state_transition(
            &transcripts,
            dozer_core::protocol::AgentKind::Claude,
            AgentState::Running,
            AgentState::AwaitingInput,
            Some(file.to_string_lossy().as_ref()),
        );

        let turns = transcripts.get_conversation_turns("s1", -1, 10).unwrap();
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn same_state_repeat_does_not_trigger_ingest() {
        use dozer_core::protocol::AgentState;
        let tmp = tempfile::tempdir().unwrap();
        let transcripts = std::sync::Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let file = tmp.path().join("s1.jsonl");
        std::fs::write(&file, "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n").unwrap();

        maybe_ingest_on_state_transition(
            &transcripts,
            dozer_core::protocol::AgentKind::Claude,
            AgentState::Idle,
            AgentState::Idle,
            Some(file.to_string_lossy().as_ref()),
        );
        assert!(transcripts.get_conversation_turns("s1", -1, 10).unwrap().is_empty(), "同态重复不该触发摄取");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd maybe_ingest_on_state_transition`
Expected: 编译失败(函数不存在)。

- [ ] **Step 3: 实现**

在 `server.rs` 里 `maybe_ingest_from_hook_data` 之后新增:

```rust
/// 会话状态转入 `Idle`/`AwaitingInput` 时的兜底摄取——替代定时轮询,
/// 复用现有状态机,只在"这一刻状态真的变了"才触发,同态重复事件不重复
/// 摄取(避免每次 hook 事件都无谓地读一次文件)。
fn maybe_ingest_on_state_transition(
    transcripts: &crate::transcripts::TranscriptStore,
    agent: dozer_core::protocol::AgentKind,
    old_state: dozer_core::protocol::AgentState,
    new_state: dozer_core::protocol::AgentState,
    transcript_path: Option<&str>,
) {
    use dozer_core::protocol::AgentState::{AwaitingInput, Idle};
    if old_state == new_state {
        return;
    }
    if !matches!(new_state, Idle | AwaitingInput) {
        return;
    }
    let Some(path) = transcript_path else { return };
    if let Err(e) = transcripts.ingest_session(agent, std::path::Path::new(path)) {
        tracing::warn!(error = %e, %path, "待命态兜底摄取失败");
    }
}
```

在 `handle_conn` 的 `Request::HookEvent` 分支里,把:

```rust
                                    match agent_state_for(&event) {
                                        Some(state) => s.set_agent_state(state, &event, ts_ms),
                                        None => tracing::debug!(%event, "未知 hook 事件，不改状态"),
                                    }
```

改成:

```rust
                                    match agent_state_for(&event) {
                                        Some(state) => {
                                            let old_state = s.info().agent_state;
                                            s.set_agent_state(state, &event, ts_ms);
                                            let tp = s.info().transcript_path;
                                            maybe_ingest_on_state_transition(
                                                &transcripts,
                                                agent,
                                                old_state,
                                                state,
                                                tp.as_deref(),
                                            );
                                        }
                                        None => tracing::debug!(%event, "未知 hook 事件，不改状态"),
                                    }
```

(这段紧跟在 Task 11 加的 `maybe_ingest_from_hook_data(&transcripts, agent, &data);` 之后——两次触发都保留:一次是 hook data 自带的路径快通道,一次是状态转换兜底,二者用的是同一个 `ingest_session`,同文件重复调用天然因为 cursor 推进而是廉价 no-op,不用额外去重。)

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd transcripts_or_ingest` (或直接 `cargo test -p dozerd`)
Expected: PASS(全部)

- [ ] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/server.rs
git commit -m "feat(dozerd): 待命态转换触发摄取兜底"
```

---

## Task 13: 启动回填

**Files:**
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes: Task 5 的 `transcripts::scan::discover_all_transcript_files`;Task 6 的 `TranscriptStore::ingest_session`。

- [ ] **Step 1: 写失败测试**

回填逻辑本身是"遍历 + 调用已测试过的 `ingest_session`",没有新的分支逻辑需要单测——直接抽成一个可单测的小函数:

在 `crates/dozerd/src/lib.rs`(或新建 `crates/dozerd/src/backfill.rs`,更整洁)新增:

```rust
//! dozerd 启动时的历史 transcript 回填。
use crate::transcripts::TranscriptStore;
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

pub fn backfill_all(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) {
    for (agent, path) in files {
        if let Err(e) = store.ingest_session(agent, &path) {
            tracing::warn!(error = %e, path = %path.display(), "启动回填摄取失败,跳过该文件");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_all_ingests_every_discovered_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let f1 = tmp.path().join("a.jsonl");
        let f2 = tmp.path().join("b.jsonl");
        std::fs::write(&f1, "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一\"}}\n").unwrap();
        std::fs::write(&f2, "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"二\"}}\n").unwrap();

        backfill_all(&store, vec![(AgentKind::Claude, f1), (AgentKind::Claude, f2)]);

        assert_eq!(store.get_conversation_turns("a", -1, 10).unwrap().len(), 1);
        assert_eq!(store.get_conversation_turns("b", -1, 10).unwrap().len(), 1);
    }

    #[test]
    fn backfill_all_skips_unreadable_file_without_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        backfill_all(
            &store,
            vec![(AgentKind::Claude, tmp.path().join("does-not-exist.jsonl"))],
        );
        // 不 panic 即通过;没有对应数据可断言。
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd backfill`
Expected: 编译失败(`backfill` 模块不存在,`lib.rs` 没挂)。

- [ ] **Step 3: 挂模块 + main.rs 接线**

`crates/dozerd/src/lib.rs` 新增 `pub mod backfill;`。

`crates/dozerd/src/main.rs` 在构造 `transcripts` 之后、进入 `tokio::select!` 之前新增:

```rust
    {
        let files = dozerd::transcripts::scan::discover_all_transcript_files();
        tracing::info!(count = files.len(), "启动回填:发现历史 transcript 文件");
        dozerd::backfill::backfill_all(&transcripts, files);
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd backfill`
Expected: PASS(2 个测试)

- [ ] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd && cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozerd/src/backfill.rs crates/dozerd/src/lib.rs crates/dozerd/src/main.rs
git commit -m "feat(dozerd): 启动时历史 transcript 全量回填"
```

---

## Task 14: `dozer-client` —— 三个新方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的协议类型。
- Produces: `Client::list_conversations(&self, cwd: &str, agent: Option<AgentKind>, limit: u32, offset: u32) -> Result<Vec<ConversationSummary>>`、`Client::get_conversation_turns(&self, conversation_id: &str, after_turn_index: i64, limit: u32) -> Result<Vec<TurnRecord>>`、`Client::get_usage_summary(&self, cwd: &str, since_ts: Option<u64>) -> Result<Vec<(ConversationSummary, UsagePayload)>>`。

- [ ] **Step 1: 写失败测试**

`crates/dozer-client/tests/against_real_daemon.rs` 需要一个真实跑起来的 `dozerd` 进程(现有测试文件的既定模式,直接照抄现有一个测试函数的 harness 搭建方式,如 `list_projects` 或 `list_bookmarks` 对应的测试,新增):

```rust
#[tokio::test]
async fn list_conversations_and_usage_roundtrip_against_real_daemon() {
    let (client, _guard) = spawn_daemon_and_client().await; // 复用文件里已有的 harness 函数名——
    // 若现有函数名不同,用 against_real_daemon.rs 里实际的启动 helper 替换这一行。
    let conversations = client
        .list_conversations("/no/such/project", None, 10, 0)
        .await
        .unwrap();
    assert!(conversations.is_empty());

    let turns = client
        .get_conversation_turns("no-such-id", -1, 10)
        .await
        .unwrap();
    assert!(turns.is_empty());

    let usage = client
        .get_usage_summary("/no/such/project", None)
        .await
        .unwrap();
    assert!(usage.is_empty());
}
```

先读一遍 `crates/dozer-client/tests/against_real_daemon.rs` 现有测试,把上面这段里"复用文件里已有的 harness 函数名"替换成实际存在的启动/连接辅助函数(该文件已经有类似 `list_projects`/`list_bookmarks` 的空库场景测试,照抄其 setup 代码即可,不要臆测函数名)。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-client --test against_real_daemon list_conversations_and_usage_roundtrip`
Expected: 编译失败(`Client` 上三个方法不存在)。

- [ ] **Step 3: 实现**

在 `crates/dozer-client/src/lib.rs` 顶部 `use` 里给 `ConversationSummary`/`TurnRecord`/`UsagePayload` 补上导入(加进现有 `use dozer_core::protocol::{...}` 那一行的花括号列表)。在 `list_bookmarks` 方法之后新增:

```rust
    pub async fn list_conversations(
        &self,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<ConversationSummary>> {
        match self
            .roundtrip(&Request::ListConversations {
                cwd: cwd.into(),
                agent,
                limit,
                offset,
            })
            .await?
        {
            Reply::Conversations { conversations } => Ok(conversations),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<TurnRecord>> {
        match self
            .roundtrip(&Request::GetConversationTurns {
                conversation_id: conversation_id.into(),
                after_turn_index,
                limit,
            })
            .await?
        {
            Reply::ConversationTurns { turns, .. } => Ok(turns),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_usage_summary(
        &self,
        cwd: &str,
        since_ts: Option<u64>,
    ) -> Result<Vec<(ConversationSummary, UsagePayload)>> {
        match self
            .roundtrip(&Request::GetUsageSummary {
                cwd: cwd.into(),
                since_ts,
            })
            .await?
        {
            Reply::UsageSummary { rows } => Ok(rows),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-client --test against_real_daemon list_conversations_and_usage_roundtrip`
Expected: PASS

- [ ] **Step 5: 全量校验 + 提交**

Run: `cargo build -p dozer-client && cargo test -p dozer-client && cargo clippy -p dozer-client --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozer-client): 新增 list_conversations/get_conversation_turns/get_usage_summary"
```

---

## Task 15: `dozer-app` —— `conversation.rs` 瘦身 + `links.rs` 改指

**Files:**
- Modify: `crates/dozer-app/src/conversation.rs`
- Modify: `crates/dozer-app/src/extensions/project/links.rs`

**Interfaces:**
- Produces: `conversation.rs` 只保留 `ConversationMeta`、`is_current_conversation`、新增 `ConversationMeta::from_summary(s: &dozer_core::protocol::ConversationSummary) -> Self`。
- Consumes(links.rs): `dozer_core::agent_paths::{home_dir, claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in}`(Task 2 产物)。

`size_bytes` 字段确认全仓库无渲染消费(仅字段定义/构造出现),这次顺手去掉,避免 `ConversationSummary` 也要背一个没人用的字段(YAGNI)。

- [ ] **Step 1: 写失败测试**

在 `conversation.rs` 的 `#[cfg(test)] mod tests` 里新增(其余测试本 Task 会删掉,因为它们测的是即将删除的函数):

```rust
    #[test]
    fn from_summary_maps_fields() {
        let s = dozer_core::protocol::ConversationSummary {
            conversation_id: "abc".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/abc.jsonl".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 3,
        };
        let meta = ConversationMeta::from_summary(&s);
        assert_eq!(meta.path, std::path::PathBuf::from("/h/.claude/projects/x/abc.jsonl"));
        assert_eq!(meta.title, "标题");
        assert_eq!(meta.modified_ms, 2);
        assert_eq!(meta.agent, AgentKind::Claude);
    }

    #[test]
    fn is_current_matches_open_transcript() {
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation(std::path::Path::new("/t/a.jsonl"), &opens));
        assert!(!is_current_conversation(std::path::Path::new("/t/c.jsonl"), &opens));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app conversation::tests::from_summary_maps_fields`
Expected: 编译失败(`from_summary` 不存在)。

- [ ] **Step 3: 瘦身 `conversation.rs`**

把整个文件替换成:

```rust
//! 历史对话展示用的中间表示(P1j 起步;P2b 扩展到 CodeBuddy/OpenCode;
//! spec 2026-08-20 起数据来源改为查询 dozerd,本文件不再直接碰磁盘)。

use dozer_core::protocol::{AgentKind, ConversationSummary};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMeta {
    pub path: PathBuf,
    pub title: String,
    pub modified_ms: u64,
    pub agent: AgentKind,
}

impl ConversationMeta {
    pub fn from_summary(s: &ConversationSummary) -> Self {
        Self {
            path: PathBuf::from(&s.file_path),
            title: s.title.clone(),
            modified_ms: s.last_ts,
            agent: s.agent,
        }
    }
}

/// 该对话是否是当前活会话(其路径在打开着的 transcript 集合中)。纯函数。
pub fn is_current_conversation(meta_path: &std::path::Path, open_transcripts: &[String]) -> bool {
    let p = meta_path.to_string_lossy();
    open_transcripts.iter().any(|o| o.as_str() == p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_summary_maps_fields() {
        let s = ConversationSummary {
            conversation_id: "abc".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/abc.jsonl".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 3,
        };
        let meta = ConversationMeta::from_summary(&s);
        assert_eq!(
            meta.path,
            std::path::PathBuf::from("/h/.claude/projects/x/abc.jsonl")
        );
        assert_eq!(meta.title, "标题");
        assert_eq!(meta.modified_ms, 2);
        assert_eq!(meta.agent, AgentKind::Claude);
    }

    #[test]
    fn is_current_matches_open_transcript() {
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation(
            std::path::Path::new("/t/a.jsonl"),
            &opens
        ));
        assert!(!is_current_conversation(
            std::path::Path::new("/t/c.jsonl"),
            &opens
        ));
    }
}
```

- [ ] **Step 4: 改 `links.rs` 的导入与调用**

在 `crates/dozer-app/src/extensions/project/links.rs` 第 127 行附近,把:

```rust
crate::conversation::home_dir()
```

改成:

```rust
dozer_core::agent_paths::home_dir()
```

第 134-136 行的 `claude_project_dir_in(home, repo)`/`codebuddy_project_dir_in(home, repo)`/`opencode_project_dir_in(home, repo)` 三处调用改成 `dozer_core::agent_paths::claude_project_dir_in(home, repo)` 等(加前缀,或在文件顶部新增 `use dozer_core::agent_paths::{claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in};` 后保持函数名不变——任选一种,和文件里其余 `use` 风格保持一致)。第 297 行测试代码里的调用同步修改。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p dozer-app conversation::`
Expected: PASS(2 个测试)。此时 `cargo build -p dozer-app` 预期还会因为 `app.rs`/`workspace.rs`/`homespace.rs`/`extensions/usage.rs` 里仍在调用已删除的 `list_all_conversations`/`conversation_title` 等函数而**编译失败**——这是预期的,Task 16-19 会逐个修完。本 Task 到此为止,不要求整个 `dozer-app` 能编译通过。

- [ ] **Step 6: 提交(允许 dozer-app 暂时编译失败,后续 Task 修完)**

```bash
git add crates/dozer-app/src/conversation.rs crates/dozer-app/src/extensions/project/links.rs
git commit -m "refactor(dozer-app): conversation.rs 瘦身为纯展示 DTO,links.rs 改用 dozer_core::agent_paths

后续 4 个 Task(workspace.rs/usage.rs/homespace.rs 的数据源改造)完成前
dozer-app 整体编译不通过，这是本次拆分的预期中间状态。"
```

---

## Task 16: `dozer-app` —— `transcript.rs` 瘦身 + 回合转换函数

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs`

**Interfaces:**
- Produces: 保留 `ReviewEntry`、`latest_model_mode_and_activity`(及其私有辅助函数)不变;新增 `review_entries_from_turns(turns: &[dozer_core::protocol::TurnRecord]) -> Vec<ReviewEntry>`;删除 `parse_transcript`/`parse_claude_shaped_jsonl`/`parse_codebuddy_shaped_jsonl`/`join_codebuddy_text_blocks`/`tool_summary`(已在 Task 3/4 迁到 `dozerd::transcripts::parse`)。

- [ ] **Step 1: 写失败测试**

在 `transcript.rs` 的测试模块里新增:

```rust
    #[test]
    fn review_entries_from_turns_maps_role_and_tools() {
        use dozer_core::protocol::TurnRecord;
        let turns = vec![
            TurnRecord {
                turn_index: 0,
                role: "human".into(),
                content: "你好".into(),
                tools_summary: vec![],
                thinking: false,
                ts: None,
            },
            TurnRecord {
                turn_index: 1,
                role: "ai".into(),
                content: "回复".into(),
                tools_summary: vec!["Edit README.md".into()],
                thinking: true,
                ts: None,
            },
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], ReviewEntry::Human { text: "你好".into() });
        match &entries[1] {
            ReviewEntry::AiTurn { text, tools, thinking } => {
                assert_eq!(text, "回复");
                assert_eq!(tools, &vec!["Edit README.md".to_string()]);
                assert!(*thinking);
            }
            other => panic!("{other:?}"),
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app transcript::tests::review_entries_from_turns_maps_role_and_tools`
Expected: 编译失败(函数不存在)。

- [ ] **Step 3: 删除已迁移函数 + 新增转换函数**

删除 `tool_summary`、`parse_claude_shaped_jsonl`、`parse_codebuddy_shaped_jsonl`、`join_codebuddy_text_blocks`、`parse_transcript` 五个函数(及其专属测试:`parses_human_and_ai_turn_skipping_noise`、`empty_and_all_noise_yield_nothing`、`opencode_reuses_claude_shaped_parser`、`kilo_reuses_claude_shaped_parser`、`codex_and_qoder_yield_empty_until_schema_confirmed`、`codebuddy_parses_real_fixture_sample`、`codebuddy_joins_multiple_text_blocks_and_skips_other_kinds`、`codebuddy_user_row_without_input_text_block_yields_no_human_entry`、`codebuddy_empty_and_all_noise_yield_nothing`、`unknown_falls_back_to_claude_shaped_parser`——这些逻辑已经在 Task 3/4 的 `dozerd::transcripts::parse` 里有等价覆盖)。

`latest_model_mode_and_activity` 及其私有辅助(`extract_line_activity`/`join_text_blocks`/`truncate_activity`)和对应测试**全部保留不动**(Agent 卡片实时指示器用,不在本次迁移范围)。

在文件顶部 `use` 语句里新增 `use dozer_core::protocol::TurnRecord;`,在 `ReviewEntry` 定义之后新增:

```rust
/// dozerd 查询回来的回合明细 → 面板展示用的 `ReviewEntry`。
pub fn review_entries_from_turns(turns: &[TurnRecord]) -> Vec<ReviewEntry> {
    turns
        .iter()
        .map(|t| {
            if t.role == "human" {
                ReviewEntry::Human {
                    text: t.content.clone(),
                }
            } else {
                ReviewEntry::AiTurn {
                    text: t.content.clone(),
                    tools: t.tools_summary.clone(),
                    thinking: t.thinking,
                }
            }
        })
        .collect()
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app transcript::`
Expected: PASS(`review_entries_from_turns` 测试 + 保留下来的 `latest_model_mode_and_activity` 系列测试全绿)

- [ ] **Step 5: 提交(dozer-app 整体仍可能因 workspace.rs/usage.rs/homespace.rs 未改而编译失败,预期中)**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "refactor(dozer-app): transcript.rs 瘦身,parse_transcript 迁至 dozerd,新增 review_entries_from_turns"
```

---

## Task 17: `dozer-app::workspace.rs` —— `spawn_conversations_refresh`/`spawn_review_load` 改走 Client

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 14 的 `Client::list_conversations`/`Client::get_conversation_turns`;Task 15 的 `ConversationMeta::from_summary`;Task 16 的 `review_entries_from_turns`。

`spawn_conversations_refresh`(1032-1049行)、`spawn_review_load`(1109-1131行)是本 Task 唯一要改的两个函数——按调研报告已确认的既有异步范式(`browser.rs::request_bookmarks_refresh` 那种 `io.client.clone()` + `handle.spawn` + `proxy.send_event`)改写,函数签名/调用方(`app.rs`/`workspace.rs` 里 `ws.spawn_conversations_refresh(io)`/`ws.spawn_review_load(io, ...)` 的调用点)**不变**,只改函数体内部。

- [ ] **Step 1: 读现状,确认函数体**

先读 `crates/dozer-app/src/workspace.rs:1032-1049` 和 `:1109-1131` 的准确当前内容(调研报告给的是大致行号,执行前用 `grep -n "fn spawn_conversations_refresh\|fn spawn_review_load" crates/dozer-app/src/workspace.rs` 定位精确行号,再 `Read` 这两段的完整现状),因为这是要改的既有代码,不是新写。

- [ ] **Step 2: 写失败测试(集成层面难以单测异步 IO,改为验证转换逻辑的纯函数测试已在 Task 15/16 覆盖;本 Task 用编译作为主要验证手段,另加一个"排序仍是 mtime 倒序"的纯函数测试防回归)**

若 `workspace.rs` 现有测试模块里有类似"排序断言"的测试,在其旁新增(否则跳过本 Step,直接进入实现,靠 Step 4 的 `cargo build` 作为主要校验——`spawn_conversations_refresh`/`spawn_review_load` 本身是"发一个异步 IO 请求+回填消息"的胶水代码,过去也没有对它们的单元测试,只有集成校验,不强行为胶水代码补测试);改动完成后运行:

Run: `cargo build -p dozer-app 2>&1 | grep -A5 "spawn_conversations_refresh\|spawn_review_load"`
Expected: 目前的构建错误(如果先只改 `conversation.rs`/`transcript.rs` 而没改这两个函数,会报"找不到 `conversation::list_all_conversations`"之类的错误)——这一步只是确认当前失败原因确实是这两个函数,不是别的问题。

- [ ] **Step 3: 实现**

把 `spawn_conversations_refresh` 函数体换成(签名不变,假设原签名是 `pub fn spawn_conversations_refresh(&self, io: &ShellIo)` 或类似形式——**以 Step 1 读到的真实签名为准,只替换函数体**):

```rust
    pub fn spawn_conversations_refresh(&self, io: &ShellIo) {
        let Some(project) = self.project.as_ref() else { return };
        let project_id = project.id;
        let cwd = self.cwd.clone(); // 若字段名不是 `cwd`,以 Workspace 结构体实际字段为准
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let list = client
                .list_conversations(&cwd, None, 500, 0)
                .await
                .unwrap_or_default()
                .iter()
                .map(crate::conversation::ConversationMeta::from_summary)
                .collect();
            let _ = proxy.send_event(Message::ConversationsRefreshed(project_id, list));
        });
    }
```

把 `spawn_review_load` 函数体换成(签名不变,原参数含 `source: ReviewSource`/`path`/`agent` 等,**以 Step 1 读到的真实签名为准**——下面示范假设签名是 `pub fn spawn_review_load(&self, io: &ShellIo, source: ReviewSource, path: String, agent: AgentKind)`):

```rust
    pub fn spawn_review_load(&self, io: &ShellIo, source: ReviewSource, path: String, agent: AgentKind) {
        let Some(project) = self.project.as_ref() else { return };
        let project_id = project.id;
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        // conversation_id 是文件名(不含扩展名)——与 dozerd 摄取时的派生
        // 规则一致(见 TranscriptStore::ingest_session)。
        let conversation_id = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        io.handle.spawn(async move {
            let result = client
                .get_conversation_turns(&conversation_id, -1, 10_000)
                .await
                .map(|turns| crate::transcript::review_entries_from_turns(&turns))
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::ReviewLoaded(project_id, source, result));
        });
    }
```

`limit: 10_000` 是"一次性拿全部回合"的临时上限(keyset 分页机制已就位,但 `review_content` 渲染函数目前假设 `rv.entries` 是完整列表,分页加载 UI 不在本计划范围——单个会话超过 1 万回合极其罕见,这个上限足够,真遇到再另开计划加"加载更多")。

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | grep -c "error\[" || true`
Expected: 错误数量比 Task 16 结束时减少(`spawn_conversations_refresh`/`spawn_review_load` 相关错误消失;`usage.rs`/`homespace.rs` 相关错误预期仍在,留给 Task 18/19)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): spawn_conversations_refresh/spawn_review_load 改走 dozer_client::Client"
```

---

## Task 18: `dozer-app::extensions/usage.rs` —— `spawn_refresh` 改走 Client

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage.rs`

**Interfaces:**
- Consumes: Task 14 的 `Client::get_usage_summary`;Task 15 的 `ConversationMeta::from_summary`。
- Produces: `ConversationUsage` 新增 `From<&dozer_core::protocol::UsagePayload> for ConversationUsage`;删除 `parse_usage`/`parse_claude_shaped_usage`/`parse_codebuddy_shaped_usage`/`MUTATING_TOOLS`(已迁到 `dozerd::transcripts::parse`)。`aggregate`/`group_usage_by_agent`/`daily_totals_by_agent`/`agent_token_share` 四个纯聚合函数**不改**(它们只吃内存里的 `Vec<(ConversationMeta, ConversationUsage)>`,跟数据来源无关)。

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn conversation_usage_from_payload_maps_all_fields() {
        let payload = dozer_core::protocol::UsagePayload {
            turns: 2,
            tool_calls: 1,
            mutating_tool_calls: 1,
            files_touched: std::collections::BTreeSet::from(["README.md".to_string()]),
            tokens_in: 10,
            tokens_out: 20,
            tokens_cache_read: 1,
            tokens_cache_write: 2,
        };
        let usage = ConversationUsage::from(&payload);
        assert_eq!(usage.turns, 2);
        assert_eq!(usage.tool_calls, 1);
        assert_eq!(usage.mutating_tool_calls, 1);
        assert_eq!(usage.files_touched, std::collections::BTreeSet::from(["README.md".to_string()]));
        assert_eq!(usage.tokens_in, 10);
        assert_eq!(usage.tokens_out, 20);
        assert_eq!(usage.tokens_cache_read, 1);
        assert_eq!(usage.tokens_cache_write, 2);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app usage::tests::conversation_usage_from_payload_maps_all_fields`
Expected: 编译失败(`From` 实现不存在)。

- [ ] **Step 3: 实现**

删除 `parse_usage`、`parse_claude_shaped_usage`、`parse_codebuddy_shaped_usage`、`MUTATING_TOOLS` 常量(及顶部不再需要的 `use serde_json::Value;`,若删完这四个函数后该 `use` 变成未使用,一并删掉)。

在 `ConversationUsage` 定义之后新增:

```rust
impl From<&dozer_core::protocol::UsagePayload> for ConversationUsage {
    fn from(p: &dozer_core::protocol::UsagePayload) -> Self {
        Self {
            turns: p.turns,
            tool_calls: p.tool_calls,
            mutating_tool_calls: p.mutating_tool_calls,
            files_touched: p.files_touched.clone(),
            tokens_in: p.tokens_in,
            tokens_out: p.tokens_out,
            tokens_cache_read: p.tokens_cache_read,
            tokens_cache_write: p.tokens_cache_write,
        }
    }
}
```

把 `spawn_refresh` 函数体换成:

```rust
pub fn spawn_refresh(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let cwd = project_path.to_string_lossy().into_owned();
        let rows = client
            .get_usage_summary(&cwd, None)
            .await
            .unwrap_or_default()
            .iter()
            .map(|(summary, payload)| {
                (
                    crate::conversation::ConversationMeta::from_summary(summary),
                    ConversationUsage::from(payload),
                )
            })
            .collect::<Vec<_>>();
        emit(Message::Loaded(project_id, rows));
    });
}
```

签名从 `(project_id, project_path, handle, emit)` 变成多了一个 `client: &dozer_client::Client` 形参——同步更新调用方(在 `crate::app.rs`/`workspace.rs` 里搜 `usage::spawn_refresh(` 的调用点,补上 `&io.client` 或等价的 `Client` 引用实参;具体调用点数量以 `grep -rn "usage::spawn_refresh(" crates/dozer-app/src/` 实测为准,可能不止一处——`Message::Refresh` 处理分支和初始加载各一处)。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app usage::`
Expected: PASS

- [ ] **Step 5: 编译验证 + 提交**

Run: `cargo build -p dozer-app 2>&1 | grep -c "error\[" || true`(错误数量应比 Task 17 结束时更少)

```bash
git add crates/dozer-app/src/extensions/usage.rs
git commit -m "refactor(dozer-app): usage::spawn_refresh 改走 dozer_client::Client,parse_usage 迁至 dozerd"
```

---

## Task 19: `dozer-app::homespace.rs` —— `load_home_recents` 改走 Client

**Files:**
- Modify: `crates/dozer-app/src/homespace.rs`

**Interfaces:**
- Consumes: Task 14 的 `Client::list_conversations`;Task 15 的 `ConversationMeta::from_summary`。

`load_home_recents`(727-764行)原本对每个最近项目调一次 `conversation::list_all_conversations(&cwd)`(本地直读三个目录);改成对每个最近项目调一次 `client.list_conversations(&cwd, None, ...)`。

- [ ] **Step 1: 读现状**

`grep -n "fn load_home_recents" crates/dozer-app/src/homespace.rs` 定位精确行号,`Read` 该函数完整现状(含它在哪个 `spawn_blocking`/`handle.spawn` 里被调用,调用方怎么拿到 `Client`)。

- [ ] **Step 2: 编译作为验证手段(胶水代码,同 Task 17 Step 2 理由,不强行补单元测试)**

Run: `cargo build -p dozer-app 2>&1 | grep -A5 "load_home_recents\|conversation::list_all_conversations"`
Expected: 当前因 `conversation::list_all_conversations` 已删除而报错。

- [ ] **Step 3: 实现**

把函数体里 `for meta in conversation::list_all_conversations(&cwd)` 这段(原本跑在 `spawn_blocking` 里,因为是同步文件 IO)改成走 `client.list_conversations(&cwd_str, None, 500, 0).await`(异步 IO,不再需要包 `spawn_blocking`——如果外层整个函数原本就是 `async move { ... }` 包在 `handle.spawn` 里,直接把内层 `tokio::task::spawn_blocking(move || { ... }).await` 这一层去掉,改成直接 `.await` 调用;`meta` 换成先拿 `ConversationSummary` 再 `ConversationMeta::from_summary` 转换)。具体改法示例(以调研报告给出的 727-764 行区间为参照,**实际改动以 Step 1 读到的真实代码结构为准**):

```rust
// 原来大致是:
// let recents = tokio::task::spawn_blocking(move || {
//     let mut all = Vec::new();
//     for cwd in recent_cwds {
//         for meta in conversation::list_all_conversations(&cwd) {
//             all.push(HomeRecentConversation { meta });
//         }
//     }
//     all.sort_by_key(|c| std::cmp::Reverse(c.meta.modified_ms));
//     all.truncate(3);
//     all
// }).await.unwrap_or_default();

// 改成:
let mut all = Vec::new();
for cwd in recent_cwds {
    let cwd_str = cwd.to_string_lossy().into_owned();
    let summaries = client.list_conversations(&cwd_str, None, 50, 0).await.unwrap_or_default();
    for s in summaries {
        all.push(HomeRecentConversation {
            meta: crate::conversation::ConversationMeta::from_summary(&s),
        });
    }
}
all.sort_by_key(|c| std::cmp::Reverse(c.meta.modified_ms));
all.truncate(3);
let recents = all;
```

`client` 需要在函数签名/调用处能拿到(同 `usage.rs::spawn_refresh` 一样补一个 `&dozer_client::Client` 形参,调用方补实参)。

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tee /tmp/build.log; grep -c "error\[" /tmp/build.log || true`
Expected: 0(整个 `dozer-app` 现在应该能完整编译通过——这是本轮 dozer-app 侧改造的最后一个函数)。

- [ ] **Step 5: 全量测试 + 提交**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -- --check`

```bash
git add crates/dozer-app/src/homespace.rs
git commit -m "refactor(dozer-app): load_home_recents 改走 dozer_client::Client——dozer-app 侧摄取管线改造收尾"
```

---

## Task 20: 全 workspace 校验 + GUI 人工验证

**Files:** 无代码改动,纯验证。

**Interfaces:** 无。

- [ ] **Step 1: 全 workspace 构建与测试**

```bash
cargo build
cargo test -p dozer-core -p dozerd -p dozer-app -p dozer-client
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

Expected: 全绿。若 `dozer-mcp`/`dozer-hook` 因为间接依赖 `dozer-core::protocol` 新增 variant 导致某处 `match` 非穷尽报错,补上对应分支(大概率是 `Reply`/`Request` 上的穷尽 `match`,照抄邻近分支的处理方式——多半是"不认识的 variant 忽略/透传"这类简单分支)。

- [ ] **Step 2: 启动真实 dozerd + dozer-app,人工验证**

按项目 `run` 技能或既有手动流程启动 `dozerd`(`cargo run -p dozerd`)和 `dozer-app`(`cargo run -p dozer-app`),验证:

1. 对话面板(`PanelKind::Conversations`)列表侧展示的历史对话跟迁移前一致(标题、数量、排序)。
2. 点开一条历史对话,内容侧正确渲染人类发言/AI 回合/工具调用摘要,跟迁移前观感一致。
3. 用量面板(`PanelKind::Usage`)顶部统计卡、按天柱状图、按 agent 饼图、逐会话列表数据跟迁移前一致(或至少数量级吻合——由于用量去重逻辑是新引入的,若历史上有 fork/resume 场景,数字可能比迁移前的旧实现更"准确"而非完全相等,这是预期改进,不是 bug)。
4. 首页(homespace)"最近对话"卡片正确展示。
5. 新开一个 agent 会话对话几轮后,不重启 app,对话面板/用量面板在合理延迟内(下一次 hook 事件或 agent 转入待命态)能看到新内容。
6. 重启 `dozerd` 后,历史对话/用量数据不丢(sqlite 落库生效)。
7. `crates/dozer-app/src/extensions/project/links.rs` 的"记忆/链接"发现功能(Project Info 面板)照常工作,未受影响。

完成后关闭这两个临时跑起来的进程,不留后台残留。

- [ ] **Step 3: 提交(若 Step 1 有补丁性修复)**

```bash
git add -A
git commit -m "chore: 摄取管线迁移收尾——全 workspace 构建/测试/clippy/fmt 校验"
```

(若 Step 1/2 未发现任何需要修复的问题,本 Task 无需提交,仅作为完成确认。)
