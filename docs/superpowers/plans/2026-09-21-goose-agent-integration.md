# Goose Agent Integration Implementation Plan

> **Spec:** `docs/superpowers/specs/2026-09-21-goose-agent-integration-design.md`  
> **Goal:** 将 Goose 作为 Dozer 的一等 agent 接入：可启动、可感知状态、可摄取会话、可总结、可派发 Todo。  
> **Tech stack:** Rust 2024、JSONL、Goose Open Plugins hooks。

## Global Constraints

- 先完成真实 Goose spike，再锁定 adapter 字段；不依据猜测写生产 parser。
- 不读写 Goose `sessions.db`，不依赖其内部 schema。
- hook 必须 fail-open、stdout 为空、恒退出 0；Dozer 故障不能改变 Goose 行为。
- 不把 Goose hook journal 伪装成 Claude transcript。
- 不伪造 token usage 或完整 tool output。
- 每个任务先写失败测试，再实现最小代码，再跑定向测试。
- 工作树可能包含用户改动；提交时只加入本任务文件。

---

## Task 0：真实 Goose contract spike

**Files:**

- Create: `spike/goose-adapter/README.md`
- Create: `spike/goose-adapter/plugin.json`
- Create: `spike/goose-adapter/hooks/hooks.json`
- Create: `spike/goose-adapter/capture.sh`

- [ ] 记录 `goose --version`、`goose session --help`、`goose run --help`。
- [ ] 安装临时 project-scoped plugin，捕获全部目标事件的原始 stdin JSON。
- [ ] 在设置 `DOZER_SESSION_ID=spike-session` 后运行真实 `goose session`，验证 hook 环境继承、cwd、事件顺序和 `Stop` 行为。
- [ ] 覆盖：纯文本回复、成功工具、失败工具、文件编辑、用户中断、正常退出。
- [ ] 验证同名用户级 plugin 重装/禁用行为，以及 hook command 路径包含空格时的 quoting。
- [ ] 验证 Goose MCP 的公开持久化配置；若不稳定，记录 `--with-extension` 的可用命令形状。
- [ ] 把去敏后的 fixture 保存到 `crates/dozer-hook/fixtures/goose-*.json`，在 README 写明版本和结论。
- [ ] 若官方 payload 与 spec 不同，先更新 spec，再进入 Task 1。

**Verification:** 手工运行 spike 清单；fixture 必须包含每个受支持事件至少一份。

---

## Task 1：协议层新增 `AgentKind::Goose`

**Files:**

- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: all exhaustive `AgentKind` matches reported by `cargo check --workspace`

- [ ] 在 protocol tests 增加 `"goose"` serde roundtrip、`label() == "goose"`、`display_label() == "Goose"` 的失败测试。
- [ ] 新增 `AgentKind::Goose` 及两个标签分支。
- [ ] 逐个处理编译器报告的穷尽 match；暂不确定行为的分支使用与 `Unknown` 相同的保守降级并写注释。
- [ ] 更新“支持四家/五家”等已过时注释，只改与事实直接相关的措辞。

**Verification:**

```bash
cargo test -p dozer-core
cargo check --workspace
```

---

## Task 2：Goose transcript 路径与扫描

**Files:**

- Modify: `crates/dozer-core/src/agent_paths.rs`
- Modify: `crates/dozerd/src/transcripts/scan.rs`
- Modify: `crates/dozerd/src/transcripts/mod.rs`

- [ ] 测试 `goose_project_dir_in(home, cwd)` 输出 `~/.dozer/agents/goose/projects/<cwd-key>`。
- [ ] 测试全量扫描和单项目扫描都能发现 Goose JSONL。
- [ ] 测试 `list_conversations_in(..., Some(AgentKind::Goose), ...)` 只返回 Goose。
- [ ] 实现路径函数，把 Goose 加入目录型 agent roots、回填候选和查询分支。
- [ ] 把数据库 agent 字符串互转纳入 `goose`。

**Verification:**

```bash
cargo test -p dozer-core agent_paths
cargo test -p dozerd transcripts::scan
cargo test -p dozerd transcripts::tests::list_conversations
```

---

## Task 3：Goose hook plugin 安装器

**Files:**

- Create: `crates/dozer-hook/src/goose_install.rs`
- Modify: `crates/dozer-hook/src/lib.rs`
- Modify: `crates/dozer-hook/src/main.rs`

- [ ] 用 tempdir 写失败测试：首次安装、重复安装、升级覆盖自有文件、卸载、保留未知文件、带空格二进制路径、无效现有 JSON 时拒绝覆盖。
- [ ] 实现 `plugins_dir()`，支持 `DOZER_GOOSE_PLUGINS_DIR` 测试覆盖，默认 `~/.agents/plugins`。
- [ ] 生成 `dozer/plugin.json` 和 `dozer/hooks/hooks.json`；注册 spec 中八类事件，timeout 为 1 秒，不写 `on_failure:block`。
- [ ] 使用临时文件 + rename 写入；卸载只删除已知文件，未知文件存在时保留目录并 warning。
- [ ] `dozer-hook install goose` / `uninstall goose` 路由到专用安装器。

**Verification:**

```bash
cargo test -p dozer-hook goose_install
```

---

## Task 4：hook 规范化、journal 写入与转发

**Files:**

- Create: `crates/dozer-hook/src/goose.rs`
- Modify: `crates/dozer-hook/src/main.rs`
- Add fixtures from Task 0 under: `crates/dozer-hook/fixtures/`

- [ ] 测试 `parse_agent("goose")`。
- [ ] 为每个 fixture 写表驱动测试，锁定事件名、Goose session ID、cwd 和关键字段的规范化。
- [ ] 测试 journal 路径、目录创建、逐行 append、1 MiB 截断、畸形 JSON、缺 `DOZER_SESSION_ID` 静默跳过。
- [ ] 测试写 journal 后 `data.transcript_path` 与真实文件完全一致。
- [ ] 实现 schema v1 journal 行和追加写；cwd 按 Task 0 结论选择 payload 字段或 hook 当前目录。
- [ ] 在 `forward` 的 Goose 分支先 append journal，再补 `transcript_path`，最后复用现有 UDS 单向发送。
- [ ] 确保所有路径 stderr 仅诊断、stdout 始终为空、主程序退出 0。

**Verification:**

```bash
cargo test -p dozer-hook goose
cargo test -p dozer-hook
```

---

## Task 5：Goose transcript parser

**Files:**

- Modify: `crates/dozerd/src/transcripts/parse.rs`

- [ ] 用 journal fixtures 写 `parse_chunk(AgentKind::Goose, ...)` 失败测试，覆盖 human、assistant final、tool start、tool success/failure、file edit、空 Stop、未知事件、未知 schema version。
- [ ] 为 `AfterFileEdit` 写断言：`mutating_tool_calls == 1` 且 `files_touched` 包含规范化路径。
- [ ] 为 `PreToolUse` 写 trace detail 测试：保留 tool name、pretty JSON input 和 `tool_call_id`。
- [ ] 实现 `parse_goose_hook_chunk`，按 spec 映射为 `ParsedTurn`，不 unwrap 外部字段。
- [ ] 在 `extract_turn_trace_detail` 新增 Goose 分支。
- [ ] 确认增量 offset 场景下半行仍由现有 transcript store 正确缓存，不在 parser 重造 framing。

**Verification:**

```bash
cargo test -p dozerd transcripts::parse
cargo test -p dozerd transcripts
```

---

## Task 6：状态机与自动安装接线

**Files:**

- Modify: `crates/dozerd/src/server.rs`（或 `agent_state_for` 当前所在文件）
- Modify: `crates/dozer-app/src/workspace/hook.rs`
- Modify: `crates/dozer-app/src/workspace/tests.rs`

- [ ] 写状态映射测试：prompt/tool/edit → Running，Stop → TurnEnded，SessionEnd → Idle；不产生虚假的 AwaitingInput。
- [ ] 扩展 `HookInstallTarget` 为 Goose plugin 分支，`ensure_hook_installed(Goose)` 调专用安装器。
- [ ] 测试 GUI 传入的 sibling `dozer-hook` 绝对路径写入配置，而不是误用 `dozer-app` 当前可执行文件。
- [ ] 把 `should_summarize_on_close` 和 agent-card activity gate 纳入 Goose。

**Verification:**

```bash
cargo test -p dozerd agent_state
cargo test -p dozer-app --lib workspace::
```

---

## Task 7：Picker、品牌展示与启动命令

**Files:**

- Modify: `crates/dozer-app/src/workspace/view.rs`
- Modify: `crates/dozer-app/src/workspace/hook.rs`
- Modify: `crates/dozer-app/src/workspace/tests.rs`
- Modify icon assets only if a reviewed, license-compatible Goose mark is available

- [ ] 先更新测试，要求 picker 出现 Goose、显示名为 `Goose`、启动命令为 `goose session`、Unknown 仍返回 None。
- [ ] 将 `agent_cli_command` 从“所有已知 agent 直接用 label”改为显式 match；必要时返回 `Option<String>`。
- [ ] 给 Goose 分配非 gold 分类色；gold 继续只表示用户该行动。
- [ ] 首期图标默认 `IconKind::Bot`；只有素材来源与授权明确时才新增品牌图标。
- [ ] 更新所有依赖固定 agent 数量/顺序的菜单、筛选和测试。

**Verification:**

```bash
cargo test -p dozer-app --lib workspace::tests::agent_cli_command
cargo test -p dozer-app --lib workspace::tests::agent_picker
```

---

## Task 8：Headless 总结与 Todo 派发

**Files:**

- Modify: `crates/dozerd/src/headless_agent.rs`
- Modify: `crates/dozerd/src/default_agent_config.rs`
- Modify: `crates/dozerd/src/todo.rs`
- Modify: `crates/dozerd/src/session_summary.rs`
- Modify related tests

- [ ] 测试 `bare_program_name(Goose) == "goose"`。
- [ ] 测试总结命令包含 `run --no-session --quiet --text` 且移除 `DOZER_SESSION_ID`。
- [ ] 测试 Todo 命令设置 project cwd、保留 session ID、传入完整 prompt。
- [ ] 实现 Goose 两类 command builder；继续复用现有登录 shell PATH 解析、timeout、summary marker 提取。
- [ ] 默认 agent 配置和 Todo agent 字符串解析接受 `goose`。
- [ ] 若 Task 0 证明必须显式挂 Dozer MCP，增加经过纯函数测试的 `--with-extension` 参数；否则扩展 MCP 安装器并单独测试格式保留。

**Verification:**

```bash
cargo test -p dozerd headless_agent
cargo test -p dozerd default_agent_config
cargo test -p dozerd todo
```

---

## Task 9：对话、Usage 与总结链路回归

**Files:**

- Modify only as required by failing tests in `crates/dozerd/src/transcripts/`, `crates/dozer-app/src/conversation.rs`, `crates/dozer-app/src/extensions/usage.rs`

- [ ] 写端到端 fixture 测试：journal → ingest → list conversation → get turns → usage summary。
- [ ] 断言 Goose 会话可见、turn 顺序正确、文件修改计数正确。
- [ ] 断言缺 usage 时 token 计数保持 0/unknown，不继承相邻会话数据。
- [ ] 断言关闭总结能读取 Goose human/assistant turns，并在 headless 失败时走启发式兜底。
- [ ] 不为每日趋势图硬塞 Goose 列；如 UI 对 0 usage 有误导，改文案为“暂无用量数据”，不要制造数字。

**Verification:**

```bash
cargo test -p dozerd transcripts
cargo test -p dozerd session_summary
cargo test -p dozer-app --lib conversation
cargo test -p dozer-app --lib usage
```

---

## Task 10：文档、全量验证与人工验收

**Files:**

- Modify: `docs/user_guide/agents.md`
- Modify: `docs/user_guide/getting-started.md` if agent list is duplicated there
- Modify: `README.md` only if it names supported agents

- [ ] 文档写清 Goose CLI 安装/配置是用户前置条件，Dozer 只安装自己的 hook。
- [ ] 写清 token usage、外部历史导入和 ACP UI 不在首期。
- [ ] 运行格式化、全 workspace tests 和 clippy。
- [ ] 用发行布局验证带空格的 app bundle 路径。
- [ ] 人工验收：启动、prompt、成功/失败工具、文件编辑、中断、退出、重启回填、关闭总结、Todo 派发、dozerd 离线。
- [ ] 检查 `git diff --check` 与 `git status --short`，确认没有混入用户无关改动。

**Verification:**

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

## Done Definition

- 自动化测试全绿。
- 真实 Goose 验收清单全过。
- 安装/卸载可逆且不触碰其他插件或 Goose 私有数据库。
- Goose 会话在 Dozer 中具备启动、状态、对话、工具轨迹、文件修改、总结和 Todo 能力。
- 已知限制在用户文档中明确呈现，没有用伪数据掩盖 usage/tool output 缺口。

