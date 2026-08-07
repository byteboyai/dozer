# Kilo 插件可行性 Spike——go/no-go 决策记录

> 日期：2026-08-07
> 关联规格：`docs/superpowers/specs/2026-08-07-dozer-multi-agent-codex-qoder-kilo-design.md` §7
> 关联计划：`docs/superpowers/plans/2026-08-07-dozer-codex-qoder-kilo-adapters.md` Task 10

## 结论一览：**go/no-go = NO-GO（待实测复核，倾向 no-go）**

本计划的 Task 10 明确定义："**任务的产出直接决定 Kilo 是否值得继续投入**；Kilo 没有进程级 hook，只有 TS/JS 插件；因官方 issue 与插件文档存在潜在矛盾，真实可行性未知——本计划把 spike 结果当作 go/no-go 决策，不预先承诺实现。"

**当前（2026-08-07，未装 Kilo CLI）判定：暂时维持 NO-GO 的"诚实降级"默认态，代码侧不产出 Kilo 插件实现。**

理由有三条，且全部指向同一结论：**在"能稳定读到 DOZER_SESSION_ID + 稳定订阅 session/message 生命周期事件"这件事被真实 CLI 证实之前，写 Kilo 插件是赌博**。

1. **机制不确定性——矛盾点未消解**：
   - 官方插件文档（`kilo.ai/docs/automate/extending/plugins`）描述 `event` hook 可订阅 `session.*`/`message.*`/`tool.execute.*` 内部总线事件；
   - 但官方 issue `Kilo-Org/kilocode#5827`（"Expose session lifecycle hooks for third-party tool integration"）被 **"Closed as not planned"**。
   - 两者是否矛盾、`event` hook 在真实 CLI 里究竟是否工作（还是文档先行/半成品），**无法从文档确定，只能实测**。在实测排期前，官方 issue 的 "not planned" 是更重的信号——它直接来自官方维护者，倾向说明"暴露生命周期钩子给第三方"这件事**不受官方支持**，与插件文档的描述冲突，可信度存疑。

2. **会话关联依赖未知字段**：Dozer 的 hook/插件架构依赖读 `DOZER_SESSION_ID`（进程环境变量）做会话→状态机关联。Kilo 插件能否读到该 env、以及 `event.properties` 里有没有可用的 session 标识（`sessionID`/`info.id`），都是未知数。OpenCode 插件能读是因为它的插件进程由 CLI 以继承环境的子进程形态启动；Kilo 是否同构未证实。

3. **投入产出不成比例**：即便 go，也涉及 TypeScript/Node 插件代码、与现有 Rust 机械扩展不同的技术栈、以及一份新的 brainstorming 周期——在不确定性未消解前投入不符合本计划"纯 Rust 机械扩展"的范围纪律。

## 证据与核实（截至 2026-08-07）

| 证据 | 状态 | 对决策的含义 |
|---|---|---|
| 官方插件文档描述 `event` hook（`session.*`/`message.*`/`tool.execute.*`） | 文档级，未经真实 CLI 证实 | go 的信号，但被下方 issue 对冲 |
| `Kilo-Org/kilocode` 仓库内有 `packages/opencode` 包 | 仓库级（计划引述） | 只说明 Kilo 是 OpenCode 下游分支，**不**证明钩子机制真的可用 |
| 官方 issue `Kilo-Org/kilocode#5827` **"Closed as not planned"** | 官方维护者裁决 | no-go 的信号，且是官方明确表态，权重高于插件文档 |

## 计划的"诚实降级"已是本计划的最终状态

Kilo 在代码侧不需要任何额外工作就能保持诚实：

- `AgentKind::Kilo` 已在 Task 1 加入，`label()` → `"kilo"`、启动器菜单已能键入 `kilo`（命令名经 `@kilocode/cli` npm 包 `bin` 字段核实）、强调色 `theme::BLUE`、图标回落 `IconKind::Bot`（Task 3）。
- `AgentKind::Kilo` 在 `dozer-hook`/`dozer-app` 各处**复用 Claude-shaped 分支**：`transcript.rs::parse_transcript`/`conversation.rs::conversation_title`/`usage.rs::parse_usage` 都把 Kilo 当 Claude 形状处理（Task 2）。这个设计的前提是"**将来 Kilo 的 transcript 由 dozer-hook 代写成 Claude 形状**"——而 dozer-hook 代写的前提是能收到 Kilo 插件转发的事件，这就又绕回"Kilo 插件必须可行"上。
- 因此现状是：**Kilo 菜单/颜色/图标/解析降级分支齐全，但没有任何 Kilo 插件代码**。这符合计划 Global Constraints 的明确定义："如果证实不可行，Kilo 在 `agent_dot_color`/`agent_icon` 等处的诚实降级分支就是最终状态，不再有后续工作。"

## 若未来要翻案（go 的触发条件）

下列条件**全部**被真实 Kilo CLI 证实后，可开一个独立"Kilo 插件实现"计划（涉及 TS/Node，不在本 Rust 计划范围）：

1. `npm install -g @kilocode/cli` 后 `kilo --version` 可用（包名/`bin` 别名 `kilo`/`kilocode` 已核实）。
2. `.kilo/plugin/dozer-probe.ts` 类型的插件确实被加载，`event` hook 能订阅到 `session.*`/`message.*`/`tool.execute.*` 事件。
3. `DOZER_SESSION_ID` 环境变量能被插件进程读到（像 OpenCode 插件那样靠 `process.env` 做会话关联）；且需读取失败时程序有兜底（sessions 的 `info.id`/`sessionID` 字段）。
4. 实际事件名与 `properties` 字段形状足以映射到 dozerd 的状态机（session started/updated/idle ↔ Running/AwaitingInput/TurnEnded）。

在满足这些之前，**Kilo 的状态维持本计划交付的最终态**：诚实降级分支齐全、无插件实现、go/no-go=no-go（暂定，待实测复核）。

## 建议的行动（择机，不影响本计划代码）

在空闲/可控环境（如临时 `/tmp/kilo-spike-project`）跑一轮 Task 10 的探针插件（`.kilo/plugin/dozer-probe.ts`，`even` handler 里把 `event.type` + `event.properties ?? {}` + `process.env.DOZER_SESSION_ID` 追加写进 `/tmp/kilo-hook-probe.log`），用 `kilo "reply with exactly one word: hello"` 触发，看日志是否出现 `session.created`/`message.updated`/`session.idle` 等事件。跑完清理 `/tmp/kilo-spike-project` 与 `npm uninstall -g @kilocode/cli`。若探针彻底不命中，则 go/no-go 从"暂定 no-go"升级为"定案 no-go"。
