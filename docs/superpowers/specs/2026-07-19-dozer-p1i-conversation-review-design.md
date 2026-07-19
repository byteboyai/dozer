# Dozer P1i 设计：对话可审阅性（首片：锚点 + 折叠）

> 状态：设计稿,用户已认可要点(范围 a+b、折叠粒度=显正文/折叠 thinking+工具),待写 spec 后复审。
> 上游:规格 §3 需求 6(对话可审阅性)、§6 数据流 3、§7 左二"会话审阅"、§4"每 agent 一个 transcript 适配器(Claude Code JSONL 先行)"。
> 起点:P1h 并入 main 后的工作区。

## 0. 范围

需求 6 含四子行为,首片只做前两个(用户选定):
- **a) 人类发言为导航锚点** ✅
- **b) AI 回合过程性输出默认折叠**（过程=thinking/工具；正文=结论,默认显示）✅
- c) 提问/待决/交付声明置顶 ⏳后续(需启发式)
- d) 产物优先于叙述(与验收/预览联动) ⏳后续

先只做 Claude Code 的 transcript 适配器；多 agent 适配器留后续。终端 pane 仍是裸对话的事实真相,本视图是并行的结构化预览(不解析裸 PTY)。

## 1. 目标

把当前 claude 会话的 transcript 结构化呈现在左二"会话审阅" tab:人类发言醒目成锚点、AI 回合默认折叠过程(thinking+工具)只留正文、可展开看过程。打开时解析、回合结束刷新。

## 2. 关键裁决

- **D1 transcript 路径经 dozerd 从 hook data 提取**:Claude Code 的 `transcript_path` 由 hook 事件 `data` 携带(现收到但丢弃)。dozerd 在 `HookEvent` 处理里提取 `data["transcript_path"]`,存到 Session(像 agent_state);`AgentEvent` 加 `transcript_path: Option<String>` 广播(GUI 更新对应终端 tab);`SessionInfo` 加 `transcript_path: Option<String>`(`#[serde(default)]`,晚 attach 恢复)。edge:首个 hook 若是未映射状态的事件携路径则该次不广播 AgentEvent——实践中 SessionStart(→Idle)/UserPromptSubmit(→Running) 都映射且早发,足够。
- **D2 transcript 适配器纯函数**(`crates/dozer-app/src/transcript.rs`):`parse_transcript(jsonl: &str) -> Vec<ReviewEntry>`。`ReviewEntry = Human{text}` / `AiTurn{text, tools: Vec<String>, thinking: bool}`。规则:逐行 JSON;`type=="user"` 且 `message.content` 是字符串 → `Human`;`message.content` 是 list(工具结果) → 跳过;`type=="assistant"` → `AiTurn`(text 块拼正文、tool_use 块收成一行、有 thinking 块则 `thinking=true`);其余 type/坏行忽略。
  **工具一行摘要**(确定规则,便于测):`<name> <主参>`,主参取法——`input.file_path` 存在则取其 basename(Edit/Write/Read 等);否则 `input.command` 存在则取前 40 字(Bash);再否则 `input.pattern`/`input.path`;都无则仅 `<name>`。例:`Edit workspace.rs`、`Bash cargo test`、`Grep`。
- **D3 左二会话审阅 tab**:`TabKind::Review`,iced 直绘(不产 webview,激活时其余 webview 隐藏——同 Acceptance)。渲染:人类锚点行(CREAM 醒目)+ AI 回合(正文显示 + 折叠区"▸ 过程:思考 + N 工具",展开显 thinking 灰字与工具行)。每回合展开态在视图内存(`expanded: HashSet<usize>`)。
- **D4 解析在 GUI 侧 spawn_blocking**:transcript 可较大,`std::fs::read_to_string` + `parse_transcript` 丢 spawn_blocking,结果经 proxy 回 UI 线程(同 git 刷新)。dozerd 不解析 transcript(哑管道:只传路径)。
- **D5 入口 = 终端 tab"审阅"按钮**:当前会话有 `transcript_path` 时,终端 tab 栏出现"审阅"入口,点击 → 打开该会话的 Review tab 并解析。
- **D6 刷新**:打开时解析一次 + 回合结束(TurnEnded)重解析(transcript 增长)。切换/关闭 tab 同预览 tab 机制。

## 3. 组件与数据流

```
claude hook(data.transcript_path) → dozer-hook → dozerd
   dozerd: Session 存 transcript_path;AgentEvent 带 transcript_path 广播 + SessionInfo 带
   ▼(client TermEvent::Agent 或 SessionInfo)
workspace: SessionTab.transcript_path 记住
   │ 点"审阅"
   ▼ spawn_blocking: read_to_string + transcript::parse_transcript
Message::ReviewLoaded(source_tab_id, Vec<ReviewEntry>) → ReviewView
   ▼ 左二 TabKind::Review 渲染:人类锚点 + 折叠 AI 回合
TurnEnded → 重解析(若 Review tab 开着且属该会话)
```

- `crates/dozer-core/src/protocol.rs`:`SessionInfo.transcript_path`、`Reply::AgentEvent.transcript_path`(均 Option<String>,向后兼容)。
- `crates/dozerd/src/session.rs`/`server.rs`:Session 存 transcript_path;HookEvent 提取 data 里的路径;info()/AgentEvent 带上。
- `crates/dozer-client/src/lib.rs`:`TermEvent::Agent` 透传 transcript_path(现只透传 state,扩成 `Agent{state, transcript_path}` 或新增字段)。
- `crates/dozer-app/src/transcript.rs`(新):`ReviewEntry` + `parse_transcript`。
- `crates/dozer-app/src/preview.rs`:`TabKind::Review` + `open_review()`。
- `crates/dozer-app/src/workspace.rs`:`SessionTab.transcript_path`、`ReviewView` 状态、消息、渲染、审阅按钮、刷新接线。

## 4. 错误处理

- 无 transcript_path(claude 没跑/hook 没装):终端 tab 无"审阅"按钮。
- transcript 文件不存在/读失败:Review tab 域内红字"无法读取会话记录"。
- 某行坏 JSON / 未知结构:该行跳过,不崩(适配器容错)。
- content 是 list 但无 text(纯工具结果 user turn):不产生 Human 锚点(正确——那不是人类发言)。
- 空 transcript:Review tab 显"暂无对话"。

## 5. 测试策略

Headless:
- `parse_transcript`:喂一段样例 JSONL(含 user 字符串、user list(工具结果)、assistant 含 thinking+text+tool_use、attachment/mode 噪音行、坏行)→ 断言产出 `[Human, AiTurn{text,tools,thinking}]` 且噪音/工具结果被跳过。
- `protocol`:SessionInfo/AgentEvent 带 transcript_path 的 roundtrip;旧无字段 JSON 可解(serde default)。
- dozerd:HookEvent 带 `data.transcript_path` → info().transcript_path 更新 + AgentEvent 带上(集成测)。
- 渲染纯函数:`ai_turn_summary(tools_len, text_len) -> String`("过程:思考 + N 工具"文案)。

人工验收(草案):Dozer 里跑 claude 聊几轮 + 让它改文件 → 终端 tab 现"审阅" → 点开左二 Review tab:人类每句话成醒目锚点、AI 回合默认只见正文、点"过程"展开见 thinking+工具;claude 再答一轮 → 回合结束后 Review 自动追加;关/切 tab 正常;没跑 claude 的 shell tab 无"审阅"。

## 6. 备选方案(已否)

- **GUI 直接扫 `~/.claude/projects` 猜 transcript**:路径含 claude 自己的会话 uuid,GUI 无从对应到 Dozer 会话;hook data 是权威来源。
- **dozerd 解析 transcript 下发结构化**:违背哑管道,且 transcript 是 GUI 展示层关切;dozerd 只传路径。
- **解析裸 PTY 重建对话**:规格明确否(§6"不解析裸 PTY");hooks/transcript 是结构化数据源。
- **增量解析/tail**:transcript 一期规模全解够快;增量留后续。
