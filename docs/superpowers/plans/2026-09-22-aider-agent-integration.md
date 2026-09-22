# Aider Agent Integration Implementation Plan

> **Spec:** `docs/superpowers/specs/2026-09-22-aider-agent-integration-design.md`  
> **Goal:** 通过 Rust launcher bridge 把 Aider 接成 Dozer 一等 agent，同时保留原生 Aider TUI。  
> **Tech stack:** Rust 2024、PTY、Aider CLI、Markdown、JSONL。

## Global Constraints

- Task 0 的真实 Aider spike 是生产实现前置条件；没有 fixture 不进入 parser 实现。
- 不 import Aider Python API，不解析 ANSI 屏幕，不修改用户 `.aider.conf.yml`。
- 只覆盖 Dozer 会话的 history/notification CLI 参数；model/provider/git/确认策略归用户。
- Aider Markdown 只在 `dozer-hook` bridge 内解析；dozerd 只认 canonical JSONL。
- 不伪造 token、tool call、files touched 或 mutating count。
- bridge/notification 失败不得阻塞 Aider。
- 每个任务先写失败测试，再实现；提交时只加入本任务文件，保留工作树中的用户改动。

---

## Task 0：真实 Aider contract spike

**Files:**

- Create: `spike/aider-adapter/Cargo.toml`
- Create: `spike/aider-adapter/src/lib.rs`
- Create: `spike/aider-adapter/README.md`
- Add sanitized fixtures under: `crates/dozer-hook/fixtures/aider/`

- [ ] 记录 `aider --version` 和相关 `aider --help` 段。
- [ ] 在 tempdir 运行交互 Aider，显式指定 chat/input history 和 notification command。
- [ ] 捕获普通 prompt、多行 prompt、slash command、代码 fence 中含 `####`、确认问答、失败响应、architect mode 的 history。
- [ ] 捕获 input history 的 prompt-toolkit 格式与写入时机。
- [ ] notification 脚本记录 cwd、`DOZER_SESSION_ID`、所有 `DOZER_AIDER_*` 和调用次数。
- [ ] 验证 Ctrl+C、中断生成、正常 `/exit`、模型错误时 notification 与退出行为。
- [ ] 验证 `aider --message ... --no-stream --no-pretty` 的 stdout/stderr/exit code。
- [ ] 将去敏样例和明确结论写进 README；如果和 spec 不一致，先修 spec。

**Verification:** fixture 必须来自真实 Aider，且覆盖 spec §7 六项。

---

## Task 1：协议与所有穷尽分支新增 Aider

**Files:**

- Modify: `crates/dozer-core/src/protocol.rs`
- Modify exhaustive matches reported by compiler

- [ ] 新增 serde roundtrip、`label() == "aider"`、`display_label() == "Aider"` 测试。
- [ ] 新增 `AgentKind::Aider`。
- [ ] 运行 workspace check，逐一处理 exhaustive match；尚未接线的位置明确保守降级，不用 wildcard 掩盖。
- [ ] 更新固定 agent 数量的注释和测试。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-core
env RUSTC_WRAPPER= cargo check --workspace --all-targets
```

---

## Task 2：Aider 路径模型

**Files:**

- Modify: `crates/dozer-core/src/agent_paths.rs`

- [ ] 新增 `aider_project_dir(_in)` 测试。
- [ ] 定义具名 `AiderSessionPaths`，包含 chat/input/canonical/state 四条路径，避免四个相邻 `PathBuf` 参数。
- [ ] session ID 校验测试：允许 UUID、`task-<id>-<ts>`；拒绝 `/`、`\\`、`..`、NUL 和空串。
- [ ] 实现 `aider_session_paths(home, cwd, session_id)`。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-core agent_paths
```

---

## Task 3：抽取通用 HookEvent 发送函数

**Files:**

- Modify: `crates/dozer-hook/src/main.rs`
- Create or modify: `crates/dozer-hook/src/forward.rs`

- [ ] 给现有 CLI forward 行为补测试：缺 session 静默、200ms 写超时、编码带 agent/event/path。
- [ ] 抽取 `send_hook_event(agent, session_id, event, data)`，保持 Claude/CodeBuddy/OpenCode/Codex/Goose 行为不变。
- [ ] 提供 launcher 可调用的库内入口；不要通过再 spawn 一次 `dozer-hook` 上报 SessionStart/End。
- [ ] 运行全部 dozer-hook tests，确认现有 adapter 无回归。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-hook
```

---

## Task 4：Aider Markdown parser 与 canonical bridge

**Files:**

- Create: `crates/dozer-hook/src/aider_bridge.rs`
- Modify: `crates/dozer-hook/src/lib.rs`
- Test with: `crates/dozer-hook/fixtures/aider/*`

- [ ] 从真实 fixture 写 parser 测试，覆盖 user `####`、多行、assistant、blockquote、fence 内 `####`、多次 chat-start、未完成末尾。
- [ ] 定义 `AiderMessage { role, content, occurrence_index }` 和稳定 hash 测试。
- [ ] 定义 versioned `BridgeState`，测试不存在、损坏、旧版本和原子写。
- [ ] 测试重复同步不追加、追加一轮只新增两行、notification 重入不重复、并发同步由文件锁串行化。
- [ ] 测试 canonical 单消息 1 MiB 截断和 history 16 MiB 拒绝策略。
- [ ] 实现 Markdown → message 列表、diff state、append canonical、flush、原子更新 state。
- [ ] 保证失败返回诊断但不修改 Aider 原始 history。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-hook aider_bridge
```

---

## Task 5：Aider launcher 与 input watcher

**Files:**

- Create: `crates/dozer-hook/src/aider_launcher.rs`
- Modify: `crates/dozer-hook/src/main.rs`
- Modify: `crates/dozer-hook/src/lib.rs`

- [ ] 测试命令构造：四条 session path、notification command quoting、用户配置不被改写、stdio inherit。
- [ ] 用 fake aider 可执行文件测试参数、环境、退出码转发、SessionStart/End 顺序。
- [ ] 用真实 input-history fixture 测试 watcher：单行、多行、重复 metadata、截断/重建、slash command 策略。
- [ ] watcher 产生事件时附 canonical `transcript_path`；Stop/SessionEnd 前调用 bridge sync。
- [ ] 实现 `dozer-hook launch aider [-- <extra args>]`。extra args 只作为 `Command::arg` 透传；禁止用户覆盖 bridge 管理的四个参数。
- [ ] 子进程收到 SIGINT/终端 resize 的行为由继承 TTY 保持，不自行吞信号。
- [ ] notification 路径恒退出 0；launcher 本身返回 Aider 退出码。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-hook aider_launcher
env RUSTC_WRAPPER= cargo test -p dozer-hook
```

---

## Task 6：Dozerd canonical transcript 摄取

**Files:**

- Modify: `crates/dozerd/src/transcripts/parse.rs`
- Modify: `crates/dozerd/src/transcripts/scan.rs`
- Modify: `crates/dozerd/src/transcripts/mod.rs`

- [ ] parser 测试：human/ai、稳定 message key、timestamp、截断标记、未知 schema/role、畸形行。
- [ ] 实现 `parse_aider_chunk`，所有 tool/usage/mutation 字段保持零值。
- [ ] scan 测试：全量回填、单项目回填、agent filter、并行 session 不串数据。
- [ ] 将 Aider path 加入目录型 agents 和数据库字符串映射。
- [ ] 端到端测试：真实 Markdown fixture → bridge → canonical JSONL → ingest → list/get turns；重复 sync/ingest 不重复。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozerd transcripts::parse
env RUSTC_WRAPPER= cargo test -p dozerd transcripts
```

---

## Task 7：状态机接线

**Files:**

- Modify: `crates/dozerd/src/server.rs`
- Add integration tests under: `crates/dozerd/tests/`

- [ ] 测试 Aider 序列：SessionStart → Idle、UserPromptSubmit → Running、Stop → TurnEnded、SessionEnd → Idle。
- [ ] 测试没有 Notification/AwaitingInput 的情况下不会误显示待输入。
- [ ] 测试迟到/重复 Stop 幂等，SessionEnd 最终覆盖 TurnEnded。
- [ ] 用 UDS 集成测试验证 launcher 风格 HookEvent 会广播并触发 transcript ingestion。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozerd agent_state
env RUSTC_WRAPPER= cargo test -p dozerd --test hook_events
```

---

## Task 8：dozer-app picker 与展示

**Files:**

- Modify: `crates/dozer-app/src/workspace/view.rs`
- Modify: `crates/dozer-app/src/workspace/hook.rs`
- Modify: `crates/dozer-app/src/workspace/tests.rs`
- Modify: `crates/dozer-app/src/extensions/conversations.rs`

- [ ] picker、固定排序、分组、filter、颜色、Bot fallback 图标测试加入 Aider。
- [ ] 用具名参数结构替代当前静态 `agent_cli_command`，使 Aider 能获得 sibling `dozer-hook` 绝对路径。
- [ ] 测试 app bundle 路径含空格和单引号时的 shell quoting。
- [ ] picker 启动 Aider 时不调用 `ensure_hook_installed`/`ensure_mcp_installed`。
- [ ] `should_summarize_on_close` 与 Agent card activity gate 纳入 Aider。
- [ ] token/tool/mutation 缺数据时 UI 使用“暂无数据”语义，不把 0 宣称为真实统计。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-app workspace::tests::agent_
env RUSTC_WRAPPER= cargo test -p dozer-app extensions::conversations
```

---

## Task 9：Headless 总结与 Todo

**Files:**

- Modify: `crates/dozerd/src/headless_agent.rs`
- Modify: `crates/dozerd/src/default_agent_config.rs`
- Modify: `crates/dozerd/src/todo.rs`
- Modify: `crates/dozerd/src/session_summary.rs`

- [ ] 测试裸程序名和 `aider` 配置字符串解析。
- [ ] 总结命令测试：`--message`、`--no-stream`、`--no-pretty`、`--no-auto-commits`，清理全部 bridge/notification env。
- [ ] Todo 命令测试：项目 cwd、`--message`、`--yes-always`，不强制覆盖用户 auto-commit 设置。
- [ ] 用 fake executable 测试 stdout 清洗、非零退出、stderr、timeout。
- [ ] 按 Task 0 结论实现 banner 清洗器；不能匹配 ANSI 位置或模型名称。
- [ ] 确认 `record_task_turns` 是 Todo 唯一落库路径，不和 bridge 重复。

**Verification:**

```bash
env RUSTC_WRAPPER= cargo test -p dozerd headless_agent
env RUSTC_WRAPPER= cargo test -p dozerd default_agent_config
env RUSTC_WRAPPER= cargo test -p dozerd todo
```

---

## Task 10：文档、打包与完整验收

**Files:**

- Modify: `docs/user_guide/agents.md`
- Modify: `docs/user_guide/getting-started.md`
- Modify: `docs/user_guide/mcp.md`
- Modify packaging metadata only if version policy requires it

- [ ] 文档说明 Aider 安装/provider 配置前置条件、notification 覆盖、无 MCP、无精确 usage/tool/mutation。
- [ ] 确认 app bundle 已包含 `dozer-hook`，launcher 不需要新二进制打包规则。
- [ ] 人工验收两个并行 Aider tab、普通/多行 prompt、代码 fence、architect mode、中断、异常退出、关闭摘要、重启回填和 Todo。
- [ ] 停止 dozerd 后验证 Aider TUI 和文件编辑仍可正常工作。
- [ ] 运行全量格式、测试、clippy 和 diff 检查。

**Verification:**

```bash
cargo fmt --all --check
env RUSTC_WRAPPER= cargo test --workspace
env RUSTC_WRAPPER= cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

## Done Definition

- 真实 Aider fixtures 与人工验收齐全，不以推测替代 contract。
- Aider 原生 TUI、用户模型配置和 git 策略保持可用。
- 状态、对话、摘要、Todo 和回填闭环完成；并行 session 不串数据。
- bridge 重复调用与进程异常退出不会制造重复 turn 或损坏 history。
- 限制在 UI/用户文档明确呈现，不用伪统计掩盖 Aider 缺少结构化事件的事实。

