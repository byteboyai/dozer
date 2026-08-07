# Dozer 多 agent 支持——Codex、Qoder、Kilo 观测层 + 启动器接入设计

> 状态：设计中，待用户终审。
> 需求来源：2026-08-07 用户提出"下一步接入 codex cli、qoder cli、kilo cli 三个 agent"，讨论后确认
> 沿用 2026-07-31 CodeBuddy/OpenCode 那次接入定下的观测层架构，并**顺带把启动器菜单也扩到这三家**
> （与上次"只做观测层，不碰启动器"的裁剪不同——这次范围更大）。
> 范围：三个 adapter 共享同一套协议改动，但各自独立交付、互不阻塞；任一 adapter 的实现前
> spike 验证失败，该 adapter 单独降级为"暂不支持"，不影响其余两个。

## 1. 现状与动机

Dozer 现有 `AgentKind` 支持 Claude、CodeBuddy、OpenCode 三家，观测层架构见
`docs/superpowers/specs/2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md`：agent 原生事件
经 `dozer-hook`（进程内 hook）或语言插件翻译成 7 个规范事件名之一，转发
`Request::HookEvent{session_id, agent, event, ts_ms, data}` 给 dozerd，`agent_state_for` 完全不
感知具体 agent 种类。

Dozer 还有一个已存在、上次接入 CodeBuddy/OpenCode 时**明确排除在范围外**的启动器：
`workspace.rs::agent_picker_popup`——固定挂在窗口右上角的菜单，列出 Claude/CodeBuddy/OpenCode/
纯 Shell/Git Shell 五项，选中后 attach 一个新 shell PTY 并自动键入对应 CLI 名字（不是 dozerd 直接
spawn 该进程）。这次用户明确要求"顺带做启动器"，所以本设计把这块也纳入范围。

调研确认 Codex CLI（OpenAI）、Qoder CLI、Kilo Code CLI 三者的可扩展机制分属现有两个 adapter 家族：

- **Codex**：`~/.codex/hooks.json`（或 `config.toml` 内联 `[hooks]`）注册 hook，事件名
  （`SessionStart`/`UserPromptSubmit`/`PreToolUse`/`PostToolUse`/`PermissionRequest`/`PreCompact`/
  `PostCompact`/`SubagentStart`/`SubagentStop`/`Stop`/`SessionEnd`）和 stdin JSON 字段名
  （`session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input`/`tool_response`）
  与 Claude Code 高度对齐——属于"hook 进程"家族，跟 `codebuddy.rs` 同构。
- **Qoder**：`~/.qoder/settings.json`（含项目级覆盖）注册 hook，事件名和 stdin 字段名与 Claude Code
  逐字对齐，是三家里跟 CodeBuddy 最像的一个，同属"hook 进程"家族。
- **Kilo**：没有进程级 hook 机制，只有 TS/JS 插件（`.kilo/plugin/` 自动发现或 `.kilo/kilo.json`
  显式注册），订阅内部事件总线（`session.*`/`message.*`/`tool.execute.*`/`permission.*`/`file.*`/
  `lsp.*`），属于"进程内插件"家族，跟 `opencode_plugin/` 同构。

## 2. 范围裁剪（已与用户核对）

- **观测层 + 启动器都做**：识别/展示三家的会话事件与 transcript，**并且**在 `agent_picker_popup`
  菜单里新增三项，选中后键入对应 CLI 名字——不新增"dozerd 直接 spawn 该进程"这条新路径，沿用
  "输入命令到已 attach 的 shell"的现状手法。
- **一份 spec，三个 adapter 独立交付**：协议层改动（`AgentKind` 加变体、翻译机制）只设计一次；
  Codex/Qoder（hook 家族）与 Kilo（插件家族）的具体实现可分头交付、互不阻塞。
- **`agent` 由首个 hook 事件坐实，不是启动器选择时预设的**：即使用户从启动器菜单选了"Codex"，
  `SessionInfo.agent` 仍然初始为 `Unknown`，直到第一条 `HookEvent` 到达才坐实——启动器只负责键入
  命令，不直接告诉 dozerd "这个会话是 Codex"（维持"agent 身份以观测到的事实为准"这条既有原则，
  用户在菜单里点错、或点了菜单后又手动切换到别的 CLI，状态机不会被误导）。

## 3. 架构总览

```
                    ┌─────────────────────────────────────────────────┐
                    │  PTY 会话（dozerd 起的 shell，继承                 │
                    │  DOZER_SESSION_ID env）                          │
                    │                                                  │
   启动器菜单键入 →   │  claude / codebuddy / opencode / codex / qoder /  │
   或用户手敲         │  kilo 进程                                       │
                    └──────────────┬───────────────────────────────────┘
                                   │ 触发各自的 hook/插件机制
       ┌────────────┬──────────────┼──────────────┬────────────┬────────────┐
       ▼            ▼              ▼              ▼            ▼            ▼
   ~/.claude/   CodeBuddy hook  opencode 插件   ~/.codex/    ~/.qoder/   .kilo/plugin/
   settings     注册            （TS，进程内）    hooks.json   settings    （TS/JS，进程内）
   .json                                                      .json
       │            │              │              │            │            │
       └─────┬──────┴──────┬───────┘        ┌─────┴──────┬─────┘            │
             ▼             ▼                ▼            ▼                  ▼
       dozer-hook 二进制：把各 agent 原生事件名翻译成 7 个规范事件名之一，
       转发 Request::HookEvent{ session_id, agent, event, ts_ms, data }
             │
             ▼
        dozerd（Unix socket）—— agent_state_for(event) 签名不变
             │
             ▼
        dozer-app（GUI）：观测侧按 session.agent 分派解析器/落盘目录；
        启动器侧 agent_picker_popup 新增 3 项菜单
```

核心设计决定不变：**规范事件词汇表仍是现有 7 个 Claude 事件名**，新增 agent 只需要写一层"翻译成
规范名字"的适配代码，核心状态机永远不用碰。

## 4. 协议改动

`dozer-core/protocol.rs` 的 `AgentKind` 新增三个变体：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Unknown,
    Claude,
    Codebuddy,
    Opencode,
    Codex,
    Qoder,
    Kilo,
}
```

`label()` 各加一行短标签。`SessionInfo`/`Request::HookEvent`/`Reply::AgentEvent` 已经带
`agent: AgentKind` 字段（P2b 加的），本次不改结构，只是枚举值变多，`#[serde(default)]` 保证旧协议
帧兼容。`agent_state_for(event: &str) -> Option<AgentState>` 签名与实现均不改。

## 5. Codex adapter（hook 家族）

**配置文件**：`~/.codex/hooks.json`（Dozer 只装 JSON 分支，不碰 `config.toml` 内联 `[hooks]`——跟
现有 Claude/CodeBuddy 只碰各自 `settings.json`、不碰其他配置层同一个克制原则）。

**CLI 形态**：`dozer-hook codex <event>`，`parse_agent` 加 `"codex" => AgentKind::Codex`。

**事件翻译表**：

| Codex 原生事件 | 规范事件 | 备注 |
|---|---|---|
| SessionStart | SessionStart | 直通 |
| UserPromptSubmit | UserPromptSubmit | 直通 |
| PreToolUse / PostToolUse | PreToolUse / PostToolUse | 直通 |
| PermissionRequest | Notification | 归并 |
| Stop | Stop | 直通 |
| SessionEnd | SessionEnd | 直通 |
| PreCompact / PostCompact / SubagentStart / SubagentStop | （丢弃） | 不影响顶层四态机 |

**session 关联**：`dozer-hook` 子进程由 Codex 的 hook 机制 spawn，Codex 进程本身是 PTY shell 的子
进程，天然继承 `DOZER_SESSION_ID`——跟 Claude/CodeBuddy 同一套机制，无需额外处理。

**未验证项（实现前必须验证的 spike，不能假设）**：
1. `~/.codex/hooks.json` 的具体 JSON 结构是否和 Claude `settings.json` 的
   `hooks: { EventName: [{ hooks: [{type, command}] }] }` 嵌套形状一致，还是扁平结构
   （`Advanced Configuration` 文档提到"每个 hook 条目有 type + command"，未给出完整 schema）。
2. `transcript_path` 指向文件的精确 schema——决定 `transcript.rs` 的 `Codex` 分支能否复用 Claude
   解析器还是需要独立解析代码（默认假设需要独立解析器，同 CodeBuddy 先例）。
3. 若两项验证失败：Codex adapter 单独降级为"暂不支持"，不阻塞 Qoder/Kilo 独立交付。

## 6. Qoder adapter（hook 家族）

**配置文件**：`~/.qoder/settings.json`（用户级；Qoder 另支持 `${project}/.qoder/settings.json` 和
`.qoder/settings.local.json` 项目级覆盖，Dozer 只装用户级，跟现状一致）。

**CLI 形态**：`dozer-hook qoder <event>`，`parse_agent` 加 `"qoder" => AgentKind::Qoder`。

**事件翻译表**（stdin 字段名 `session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/
`tool_input` 与 Claude 逐字对齐，是三家里最像 CodeBuddy 的一个）：

| Qoder 原生事件 | 规范事件 | 备注 |
|---|---|---|
| SessionStart / SessionEnd / UserPromptSubmit / PreToolUse / PostToolUse / Stop | 直通 | |
| PostToolUseFailure | PostToolUse | 归并 |
| StopFailure | Stop | 归并 |
| PermissionRequest / PermissionDenied | Notification | 归并 |
| PreCompact / PostCompact / SubagentStart / SubagentStop / InstructionsLoaded / ConfigChange / CwdChanged / FileChanged / WorktreeCreate / WorktreeRemove / Elicitation / ElicitationResult | （丢弃） | 不影响顶层四态机 |

**未验证项（spike）**：
1. `~/.qoder/settings.json` 的 hook 注册 JSON 结构是否与 Claude 逐字一致（大概率一致，但按既有
   纪律不能只凭文档假设，需真实装机验证）。
2. `transcript_path` 指向文件的精确 schema。
3. 若验证失败：Qoder adapter 单独降级为"暂不支持"，不阻塞 Codex/Kilo 独立交付。

## 7. Kilo adapter（插件家族）

**安装位置**：`.kilo/plugin/` 自动发现目录（优先于 `.kilo/kilo.json` 显式注册——不需要合并用户
配置，Dozer 独占文件名，install 直接整体写、uninstall 直接删，跟 `opencode_install.rs` 同一套
手法）。

**未验证项（spike，本设计里优先级最高的一项）**：Kilo 是否像 OpenCode 一样对 `.kilo/plugin/` 下
每个 `.ts`/`.js` 文件做平铺式自动加载。如果是，翻译逻辑的纯函数必须像 `dozer-lib/` 一样塞进子
目录，否则会被 Kilo 误当插件工厂调用而报错——OpenCode 当年真实踩过这个坑
（`2026-07-31-opencode-plugin-spike-findings.md`）。

**事件翻译**（Kilo 事件总线字段名与 OpenCode 不同，需要独立的翻译状态机，但整体形态参照
`dozer-translate.ts` 的手法：`createSessionState`/`onXxx` 纯函数 + `dozer.ts` 做 spawn 胶水）：

| Kilo 原生事件 | 规范事件 | 备注 |
|---|---|---|
| `session.created`（非子会话） | SessionStart | 子会话判定字段名需 spike 核对（是否有类似 OpenCode `parentID` 的字段） |
| `message.updated`，role=user | UserPromptSubmit | |
| `tool.execute.before` | PreToolUse | |
| `tool.execute.after` | PostToolUse | |
| `permission.asked` | Notification | |
| `session.idle` | Stop | |
| `session.deleted` | SessionEnd | |
| `session.updated` / `session.error` / `session.compacted` / `message.removed` / `message.part.updated` / `file.*` / `permission.replied` / `lsp.*` | （丢弃或仅用于攒 transcript 文本，不转发规范事件） | `message.part.updated` 的取舍参照 OpenCode `onTextPartUpdated` 手法，具体字段在实现阶段核对 |

**Session 关联**：读 `process.env.DOZER_SESSION_ID`，假定 Kilo 插件进程继承父进程环境（跟
OpenCode 插件同样的假设）。**spike 需确认该假定成立**——若 Kilo 插件运行在沙盒环境看不到父进程
env，需要另想关联方案（本设计不预判，届时单独降级）。

**Transcript 落盘**：Kilo 没有 `transcript_path` 概念，跟 OpenCode 一样由 `dozer-hook kilo <event>`
代写 Claude 形状的 JSONL 到 `~/.dozer/agents/kilo/projects/<cwd-key>/<session-id>.jsonl`。

**异常处理（硬性要求，与 OpenCode 插件同级）**：插件跑在用户 Kilo 进程内部，风险等级最高——每个
事件回调整体包 try/catch，任何 dozer 侧故障不能拖垮或拖慢用户会话。

**未验证项汇总（spike）**：
1. `.kilo/plugin/` 是否平铺加载。
2. 插件进程是否继承 `DOZER_SESSION_ID`。
3. 真实事件字段名核对（`session.created` 的子会话标识、`tool.execute.before/after` 的 payload
   形状、`message.updated` 的 role/content 字段路径）。
4. 若验证失败：Kilo adapter 单独降级为"暂不支持"，不阻塞 Codex/Qoder 独立交付。

## 8. 启动器 UI 改动

`workspace.rs::agent_picker_popup` 菜单从 5 项扩到 8 项，新增 Codex/Qoder/Kilo（位置紧跟
OpenCode 之后、纯 Shell 之前）。三个既有纯函数各加分支：

- **`agent_cli_command`**：复用 `label()`，`Codex → "codex"`、`Qoder → "qoder"`、
  `Kilo → "kilo"`。**这是启动器部分唯一的硬编码假设**——若用户实际安装的可执行文件名与此不同，
  需要在实现前核对确认（不在本设计里画死，作为实现阶段的验证项）。
- **`agent_dot_color`**：`theme.rs` 新增 3 个强调色常量。现状只有 `CYAN`/`GREEN`/`PURPLE`/`RED`
  四个非背景色，CLAUDE.md 锁定的 5 色核心色板（bg/金/奶油/青/绿）之外，`PURPLE` 已是先例式扩展，
  这次比照办理。具体色值留到实现阶段配色评审时定，不在这份 spec 里画死。
- **`agent_icon`**：`icons.rs` 新增 3 个 `IconKind` 变体 + 对应 SVG 资源。素材来源参照 Claude/
  CodeBuddy（Simple Icons）/OpenCode（官网 favicon）的先例，在实现阶段现找；Codex 大概率可复用
  OpenAI 的 Simple Icons 图标，Qoder/Kilo 可能需要去官网扒 favicon。找不到合适素材时的兜底方案：
  回落到通用 `IconKind::Bot`（现状 `Unknown` 的处理方式），不阻塞交付。

## 9. dozer-app 消费侧改动

沿用现有"按 `AgentKind` exhaustive match 分派"套路，编译器会在每个漏改的分支报编译错误：

- **`transcript.rs::parse_transcript`**：Kilo 分支复用 Claude 分支（dozer-hook 代写同形状 JSONL，
  跟 OpenCode 一样）。Codex/Qoder 是否能复用 Claude 分支取决于 §5/§6 spike 验证结果——默认假设
  两者都需要独立 schema 解析器（字段名一致不代表 transcript 文件整体 schema 一致，CodeBuddy 当年
  是反例）。
- **`conversation.rs::project_dir(agent, cwd)`**：Codex/Qoder 各自指向其原生 session 落盘目录
  （精确路径由 spike 探明）；Kilo 指向 Dozer 代写的 `~/.dozer/agents/kilo/projects/`（跟 OpenCode
  同款）。
- **`list_conversations`**：扫描目录从 3 个 agent 扩到 6 个，合并后按 mtime 统一倒序，循环逻辑
  不变，只是 agent 列表变长。
- **`usage.rs`**：`parse_usage` 同 transcript 的分派方式；分组 `ORDER` 常量数组从 3 个 agent 扩到
  6 个；饼图图例颜色直接复用 §8 新增的 `agent_dot_color`。

## 10. 错误处理与降级行为

- Agent 一直是 `Unknown`：正常状态，不特殊处理。
- `dozer-hook`/Kilo 插件转发失败（dozerd 未运行、socket 不存在）：静默失败，绝不影响用户的 agent
  CLI 本身运行。
- 事件 schema 漂移（三家 CLI 未来版本改字段名/事件类型）：翻译层对未知事件/字段一律忽略跳过，不
  panic、不阻塞。
- 任一 adapter 的 spike 验证失败：该 adapter 单独降级为"暂不支持"，不阻塞其余两个独立交付
  （§2 已确认的范围裁剪）。
- 启动器菜单键入的 CLI 名字如果用户机器上未安装：现状本来就允许"键入一个 shell 找不到的命令"
  （比如现在选 CodeBuddy 但没装 CodeBuddy 也是同样行为），不新增校验逻辑。

## 11. 测试策略

- `protocol.rs`：新增三个变体的 roundtrip 测试 + 旧协议帧缺 `agent` 字段回落 `Unknown` 测试，与
  现有 `old_session_info_without_agent_state_decodes_as_idle` 同一套路。
- `dozer-hook`：Codex/Qoder 各自的事件翻译表用纯函数表驱动测试（同 `codebuddy.rs` 风格）；
  `install.rs` 新增 `settings_path_for` 分支 + 幂等/保留无关配置/卸载测试（直接复制现有测试改
  agent 名）。
- Kilo 插件：独立 TS 纯函数（`createSessionState`/`onXxx`）+ 测试文件，风格同
  `dozer-translate.ts`/`dozer-translate.test.ts`；`kilo_install.rs` 的安装/卸载/幂等测试同
  `opencode_install.rs`。
- `dozer-app`：`transcript.rs`/`conversation.rs`/`usage.rs`/`workspace.rs` 里已经是表驱动测试
  （`agent_dot_color_maps_each_kind_and_avoids_gold`、`agent_icon_maps_each_kind_to_brand_icon`、
  `agent_cli_command_maps_known_agents`、usage 的 `ORDER` 分组测试等），直接扩表覆盖新变体。
- 人工验证清单（三个 spike 各自独立，写进 plan 而非自动化测试）：
  1. 真实装 hook/插件 → 跑真实会话 → 确认 `SessionInfo.agent` 从 `Unknown` 正确翻转。
  2. 确认 `transcript_path`/代写 transcript 可解析、历史侧栏能看到这条对话、用量面板能统计到。
  3. 确认启动器菜单三项新条目能正确键入对应 CLI 名字、图标颜色符合预期。
