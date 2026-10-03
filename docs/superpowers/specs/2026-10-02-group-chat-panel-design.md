# 群聊面板（Group Chat）设计

日期：2026-10-02
状态：设计已在对话中逐段确认，待用户审阅本文后进入实现计划
关联：`docs/superpowers/specs/2026-09-23-agent-memory-sharing-design.md`（多 agent 共享记忆，相邻但不同话题）、`docs/superpowers/specs/2026-10-02-todo-webview-design.md`（webview 面板宿主参考）、`crates/dozerd/src/headless_agent.rs`（无头调用既有能力）

## 1. 目标与已确认的需求

**一句话**：新增「群聊」面板，让 human 邀请多个 agent CLI（第一版为 Claude Code 与 Codex）进入同一个群，它们共享**群聊上下文**（不是共享各自的 transcript），由 human 用 `@agent` 点名决定谁发言；群聊只做讨论，需要动手的事转为待办。

已和用户确认的决定：

| 项 | 决定 |
|---|---|
| 群的性质 | 纯讨论，agent 不改文件、不执行命令；需要干活时由 human 把消息推送到 Todo；**群聊不分配任务，任务分配由 Todo 负责** |
| 交互载体 | chat UI，不要求是 PTY 终端 |
| 发言推进 | 仅 human `@agent` 触发；一条消息 `@` 多个时**串行**，按首次出现顺序 |
| `@` 语义 | `@` 只表示"请发言"；转待办是对消息的单独操作（不用 `@` 兼任） |
| 第一版 agent | 仅 Claude Code、Codex（其余 agent 多为"干活型"，不入群） |
| 只读浏览项目 | 允许；写入与命令执行必须被执行层禁止 |
| 面板 | 新 `PanelKind::GroupChat`，图标 lucide `square-sparkles`，默认右栏、排在 `Agent` 之后；群聊 UI 用 webview |
| 驱动方案 | 方案 1：每次发言一次**无状态**无头调用，群聊记录是唯一真相源 |

## 2. 非目标（第一版明确排除）

- agent 之间互相 `@`、自动多轮对话、共识检测、loop guard（human 独占点名权，故不需要）
- 摘要压缩上下文（超预算只丢最早的消息）
- 每 agent 可恢复会话 / 增量投递（方案 2，作为后续优化）
- 接入除 Claude Code、Codex 以外的 agent 入群
- 用户编辑、删除已有消息
- 多用户共享群
- 预设流程模板（"写-审"阶段化等），仅靠 human 手动 `@` 顺序实现"按次序/按角色"

## 3. 调研结论（对设计的支撑）

GitHub 上已有同类项目，均为独立聊天工具，无一站在"委托人治理与验收"一侧：

- [AI Agent Room](https://github.com/soft-jp-com/ai-agent-room)：无头 CLI 驱动（`claude -p --output-format json`、`codex exec --json`），上下文按增量投递，与本方案同源；0 星，实验性。
- [agentchattr](https://github.com/bcurts/agentchattr)：PTY/tmux 注入 + MCP `chat_read`，有 loop guard 与角色/阶段模板；1.5k 星，最成熟。
- [solace-agentic-chats](https://github.com/Kryhr/solace-agentic-chats)：每轮起官方 CLI 子进程，"群聊是协调通道，不是 transcript"，与本需求措辞一致。

取舍：采用"Dozer 主动投递上下文"而非"agent 自己经 MCP 拉取"（后者行为不可控、依赖 agent 自觉）；`agentchattr` 的角色/阶段模板留作后续流程预设的参考。

## 4. 架构

### 4.1 三层分工

1. **`dozer-core`**：共享类型（群、成员、消息、状态）与 UDS 协议新增的请求/事件。
2. **`dozerd`**：群聊权威状态与调度。持久化消息、解析 `@`、按序调用无头适配器、写回回复。放 dozerd 是因为 agent 启动/托管本就归它，GUI 重启不丢进行中的一轮讨论。
3. **`dozer-app`**：`extensions/group_chat`，仅展示与输入；不持有 agent 进程；失败经 `outbox` → `App::push_toast` 路径（见 CLAUDE.md Toast 约定）。轮询由 `about_to_wait` 定时唤醒驱动（不再起每群一个循环任务），**仅在面板可见且有发言进行中时运行**，空闲时自降频退避。

### 4.2 一轮发言的数据流

human 发消息（含 `@claude @codex`）→ dozerd 落库该消息并为每个被点名成员**一次性创建全部排队占位**（`Queued`，`seq` 连续）→ 解析 mentions → 对每个被点名成员**按序**：占位转 `Running` → 拼装提示词 → 调用适配器 → 成功写入正文/`Done`，失败 `Failed` → 全部结束后停下等 human。GUI 用 `rev` 增量轮询（`ListGroupMessages{after_rev}`）获取新增与变更，不依赖 dozerd 主动推送事件。

## 5. 数据模型（类型放 `dozer-core`）

- **`Group`**：id、所属项目、群主题（讨论目标）、成员列表、创建时间。
- **`Member`**：id、`AgentKind`（第一版仅 Claude / Codex）、群内唯一 `handle`（如 `@架构师`）、角色设定文本。同一种 agent 可多次入群扮演不同角色——"按角色发言"即 `@` 不同 handle，不另设机制。
- **`Message`**：群内单调递增 `seq`、作者（`Human` / `Member(id)` / `System`）、正文、`mentions`、状态（`Queued` / `Running` / `Done` / `Failed(原因)` / `Cancelled`，仅 agent 消息有）、耗时、创建时间、可选的 Todo 关联 id。human 消息落库时一次性为所有被点名成员创建排队占位，保证 `seq` 连续。
- 持久化沿用 dozerd 现有存储方式；实现计划阶段读 `todo.rs` 等既有存储后确定具体形态（本设计不预设）。

## 6. 上下文拼装

轮到某成员发言时，dozerd 现场拼一份提示词：

1. **固定头部**：群主题 + 该成员角色设定 + 硬规则（"只讨论，不修改文件、不执行命令；需要动手请建议 human 转为待办"）。
2. **群聊历史**：按 `seq` 排序，每条标明作者，**只含最终文本**（无工具调用、无思考过程）；`Failed` / `Cancelled` 消息不进入上下文。
3. **本次触发消息**：置于末尾并明确标出"现在请你回应这一条"。

**预算**：总字符预算 + 单条上限，超预算从最早的消息丢弃并留一行"（更早的 N 条已省略）"，口径与 `headless_agent.rs` 的 `MAX_TRANSCRIPT_CHARS` / `MAX_TURN_CHARS` 一致。串行保证后一位拼装时前一位的回复已落库，可见可回应。

## 7. 调度器（dozerd，每群一条串行队列）

- **触发规则**：`@` 按首次出现顺序去重；未知 handle 在消息下提示且不触发任何人；无 `@` 的消息只作上下文，不触发发言；**agent 回复中的 `@` 不解析**。
- **同群单飞**：同群新的 human 消息排队尾；不同群互不阻塞。
- **取消**：支持"停止当前发言"与"停止本轮（清空剩余队列）"；取消须真正终止子进程，状态记 `Cancelled`。
- **失败**：超时 / 启动失败 / 非零退出 / 空输出 → 该消息标 `Failed` 并内联显示原因与"重试"；**默认继续**本轮剩余成员发言；重试只重跑该成员（基于当前历史）。失败写 `dozer_core::log`，不弹 Toast（它是该消息的持久状态）。

## 8. 适配器

复用 `headless_agent.rs` 的进程层（`resolve_binary_path`、超时、`env_remove(DOZER_SESSION_ID)` 防止 hook 误记会话）。与总结任务的区别：不用分隔符 JSON 协议，直接取最终文本作为回复；工作目录设为项目目录（允许只读浏览）。

- **Claude**：`claude -p --no-session-persistence`，提示词走 stdin，只读靠 `--allowedTools "Read,Grep,Glob" --disallowedTools "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch"`；不持久化群聊无头 session，避免污染 Conversations / Usage。
- **Codex**：`codex exec --ephemeral --sandbox read-only --skip-git-repo-check --output-last-message <file>`（最终文本取文件，回退 stdout），提示词走位置参数；同样禁止持久化 session。

**只读约束必须靠执行层**（Codex sandbox、Claude 工具白名单/plan 模式），不能只靠提示词：群历史含其他 agent 的输出，不应被当作可信指令。

### 前置核实项（实现计划第 0 步，冒烟验证，**已由 Task 0 实测**）

1. 两家能否稳定拿到**干净的最终文本**（Codex stdout 是否夹杂进度输出、是否需 `--output-last-message`）。
2. Claude 的只读限制具体参数组合（`--permission-mode plan` / 工具白名单等）；允许读文件、禁止写与执行。
3. 无头调用默认会留下 session 文件并污染 Conversations / Usage；实现统一使用两家 CLI 原生的非持久化参数，从源头隔离。
4. `headless_agent.rs` 里 Codex 分支的注释自述"参数名/版本待真实验证、不宣称已 smoke"，本功能须一并验证。

## 9. 界面（webview）

- 宿主接线对照 Todo / Usage 的 webview 面板做法，细节在计划阶段确认。
- 布局（**两栏，2026-10-03 修订**）：面板为「左侧聊天详情 webview + 右侧原生群列表」两栏，同会话/Usage/CodeHealth 的做法（走 `PairPane`，`pair_split_ratio(GroupChat)` 返回 `Some`，新增独立 `Divider::GroupChatSplit`）。右侧原生列表承载选择群、新建群（内联文本输入）、删除群（内联二次确认），可拖拽分栏、可收起（`group_chat_split` / `group_chat_list_collapsed`）。左侧 webview 自上而下：成员条（每成员一个色块 chip，可添加/编辑角色）→ 消息流 → 底部输入框（`@` 补全）。原「群切换条」已移除——切群改由右侧原生列表完成。
- `@` 补全与成员选择在网页 DOM 内，不涉及原生浮层；**添加/编辑成员、转为待办仍是 webview 内 DOM 对话框**（不走标准对话框独立窗口机制）。**新建群、删除群改在右侧原生列表内联完成**（新建=内联文本输入，删除=内联二次确认），不再用 webview 对话框。
- 视觉：human 消息与发送按钮用金色（甲方动作专属）；各 agent 成员用青/绿等区分；正文用系统字体，代码块目前用系统等宽字体（见 §13）；Markdown 渲染采用**最小安全子集，原始 HTML 与链接一律以纯文本显示**。发言中用现有 loading 动画（不新增 Spinner）；agent 消息有"停止"，失败有"重试"。
- 遵循核心原则：消息只读展示，不提供编辑入口。

## 10. 转为待办

**边界：群聊只负责把任务推送到 Todo，不做任何分配；谁来做、指派给哪个 agent，全部由 Todo 面板及其既有链路负责。**

- 每条消息的操作含"转为待办"：弹对话框，预填消息内容可编辑，确认后在 Todo 中新建一条待办。对话框内**没有**指派 agent 的选项。
- 创建后消息显示待办徽标，点击跳转 Todo 面板。
- 走现有 Todo 的新增入口。来源链接**只在消息侧记录**（消息的 `todo_id` 字段），Todo 表不加反向字段，不改动 Todo。指派不在本功能范围内，不为它改动 Todo。

## 11. 错误与空状态

- dozerd 不可用：沿用顶栏 `daemon_badge`，输入框置灰。
- 创建群失败等一次性事件：走 Toast（对话框提交后**立即关闭**，失败由 Rust 侧经 `outbox` → Toast 反馈，不在对话框内停留）。
- 成员名称在前端先做**即时校验**（镜像后端 `dozerd::group_mentions::validate_handle` 规则，含空/超长/非法字符/重复），提交前就能给出提示；后端仍为权威，两侧规则须同步。
- 无成员：显示引导。
- 无 `@` 的消息：轻提示"无人被点名，仅作为上下文"。
- 日志来源名用面板名 `group_chat`（`app/state.rs` 测试强制）；不记录敏感内容。

## 12. 测试

- dozerd 单测：`@` 解析（去重/顺序/未知 handle）、上下文拼装预算与截断、命令构造。
- 调度器以假适配器测：串行顺序、后者可见前者回复、取消、失败后继续、单成员重试。
- 真实 CLI 冒烟默认 `#[ignore]`，手动运行，对应第 8 节前置核实项。
- webview 宿主事件的归属校验参照 Flyfish `HostBinding` 做法。

## 13. 未决项（计划阶段处理）

- 群与消息的具体持久化形态（读 dozerd 既有存储后定）。
- 总字符预算 / 单条上限 / 单次发言超时的具体数值（需实测，不在设计里定死）。
- 无头调用 session 污染已通过 `--no-session-persistence` / `--ephemeral` 解决（见 8 节核实项 3）。
- **群聊代码块字体**：目前用系统等宽；是否改用 JetBrains Mono 待用户决定（需扩 host CSP 的 `font-src` 并拷贝字体到 `assets/group-chat-content/`）。
- **轮询间隔** `POLL_INTERVAL = 500ms`（带 `POLL_BACKOFF = 5s` 退避）为初值，待实测调整。
- 实现须在独立分支进行，审阅后合并（见仓库惯例）。
