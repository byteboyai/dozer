# Dozer P1e 设计：agent 集成基础层

> 状态：设计稿,待用户审阅。范围裁决因用户暂离由实施方按"最小可逆"原则代拟,**用户可整体推翻**(见 §0)。
> 上游:规格 `2026-07-14-dozer-phase1-design.md` §3 需求 1(P1c 达成标注点名 OSC 7/133 与 agent 状态感知留待 P1e)、§6 数据流 3、§9 顺序建议。
> 起点:P1d 并入 main(`2cdc79e`),dozerd 仅有会话存活内核,dozer-hook 仅有事件信封骨架。

## 0. 范围裁决(代拟,可推翻)

**P1e = agent 集成基础层**,不含验收闭环 UI。理由:
- 规格 §9 顺序下一步是⑤验收闭环,但其核心数据流("agent 干活 → hook/transcript 事件 → 交付声明")依赖 hook 事件通路,而 dozerd 连 hook 接入端点都没有——基础层是⑤的前置依赖;
- P1c 验收档已白纸黑字把 OSC 7/133 + agent 状态感知划给 P1e;
- 延续 P1a-d 每段一天左右的小步节奏。

**顺延 P1f**:验收闭环 UI(S0/S0b 立项定标、交付横幅 CTA、S2 验收、S3 演进史、SQLite + git ref)。
**顺延 P1f/g**:transcript 适配器与左二会话审阅 tab;状态胶囊里的 Context %(数据源在 transcript,随适配器来)。

## 1. 目标

1. **shell 集成**:OSC 7(cwd 跟踪)与 OSC 133(命令边界 + 退出码)从字节流中解析出来;zsh 会话自动注入发射端。
2. **hook 事件通路**:Claude Code hooks → `dozer-hook` → dozerd 事件总线 → 所有已连接 GUI。这是验收闭环与对话审阅共用的数据面。
3. **agent 状态感知**:终端 tab 状态胶囊,四态 `空闲/运行中/待输入/回合结束`,由 hook 事件驱动。
4. **hooks 注册**:`dozer-hook install|uninstall` 幂等命令行注册(W1 一键注册 GUI 属周边页面阶段,复用同一逻辑)。

## 2. 关键裁决

- **D1 OSC 解析在客户端**(dozer-app),不进 dozerd。有状态字节扫描器挂在 attach 输出流上、alacritty 解析之前;跨 chunk 截断的序列要能续接。理由:ring 回放在 attach 时天然重建 cwd/命令状态;daemon 保持哑字节管道(P1b 原则"网格入 daemon 留二期");已核实 alacritty_terminal 0.26 的 `Event` 枚举无 OSC 7/133 透传,自建扫描器是唯一路径。备选"daemon 侧解析"被否:要给事件标 offset 保序,协议面陡增,且违背哑管道。
- **D2 zsh 发射端经 ZDOTDIR 包装注入**(kitty/ghostty 同款姿势):dozerd 对交互式 shell 会话设 `ZDOTDIR` 指向 Dozer 自带的包装 zshrc,包装先加载用户原 zshrc,再挂 precmd/preexec 发 OSC 7/133。逃生门 `DOZER_SHELL_INTEGRATION=0`。mac 先发 zsh-only,bash/fish 留门(检测 `$SHELL` 非 zsh 时不注入、功能静默降级)。
- **D3 hook 通路复用现有 UDS + JSON Lines**:协议增 `Request::HookEvent{session_id, event, ts_ms, data}`(dozer-hook 单向发完即走)与 `Reply::AgentEvent{...}`(dozerd 广播给所有已连接客户端);registry 记每会话最新 agent 状态,`SessionInfo` 增可选字段 `agent_state`,GUI 晚 attach 也能拿到现状。**不上 SQLite**——事件持久化随 P1f 验收闭环一起定(过程性资产存放是规格 §8 显式未决项,此处只做易失总线,最小可逆)。
- **D4 会话归属靠环境变量**:dozerd spawn 时注入 `DOZER_SESSION_ID`;claude 在 shell 里手动跑也能继承到 hook 进程。无此变量的 hook 事件丢弃并记日志(非 Dozer 会话里的 claude 不该打扰 dozerd)。
- **D5 hooks 注册是显式可逆命令**:`dozer-hook install` 增量合并写入 `~/.claude/settings.json` 的 hooks 段(只添加/更新自己的条目,不动用户已有 hooks),`uninstall` 精确移除;二进制绝对路径写入,幂等可重复执行。
- **D6 状态机四态**:`Idle/Running/AwaitingInput/TurnEnded`。映射(实现期对表 Claude Code hooks 文档校准,以下为设计意图):`UserPromptSubmit→Running`;`Notification`(权限请求/空闲提醒)`→AwaitingInput`;`Stop→TurnEnded`;会话退出/`SessionEnd→Idle`。TurnEnded 即 P1f 交付横幅的挂点,本期只呈现状态。

## 3. 组件与数据流

```
zsh(precmd/preexec 发 OSC 7/133)         claude(hooks 触发)
   │ PTY 字节流(混在输出里)                  │ stdin JSON + DOZER_SESSION_ID
   ▼                                        ▼
dozerd(哑管道,ring 缓存)               dozer-hook(读stdin→UDS 发 HookEvent→退出)
   │ Reply::Output(attach 流)               │
   ▼                                        ▼
dozer-app osc.rs 扫描器 ──剥离──▶ dozerd 广播 Reply::AgentEvent + registry 记最新态
   │ OscEvent{Cwd,CmdStart,CmdEnd{exit}}    │
   ▼                                        ▼
workspace 终端域(cwd/退出码显示)      workspace 状态胶囊(四态)
```

- `crates/dozer-app/src/osc.rs`(新):`OscScanner::feed(&[u8]) -> (Vec<OscEvent>, Vec<u8>)`——剥离已识别序列、透传其余字节给 alacritty;单序列长度上限 2KB 防炸。
- `crates/dozerd/src/`:server 增 HookEvent 分支;registry 增 per-session `agent_state`;session spawn 注入 `DOZER_SESSION_ID` 与 ZDOTDIR 包装。
- `crates/dozer-hook/src/`:main 读 stdin JSON + env,组 HookPayload,UDS 单发,200ms 超时静默退出;install/uninstall 子命令。
- `crates/dozer-core/src/protocol.rs`:两个新消息 + SessionInfo 字段(向后兼容,旧字段不动)。
- 资产:`crates/dozerd/assets/dozer-integration.zsh`(包装 zshrc)。

## 4. UI(最小呈现)

- 终端 tab:agent 状态点(Idle 灰/Running 绿 `#1AD585`/AwaitingInput 紫蓝 `#9580FF`/TurnEnded 金 `#F2D94E`——金=甲方该出手了,语义与主题裁决一致)+ 胶囊文字。
- cwd:终端 tab 标题显示 cwd basename(OSC 7 到达后替换默认名;用户手动改名优先)。
- OSC 133:最近命令非零退出时状态条红色短提示(`exit 1`);滚动条命令标记留二期。

## 5. 错误处理

- dozerd 不在/UDS 拒连:dozer-hook 200ms 内静默退出码 0——**绝不拖慢或阻塞 agent**。
- 畸形 hook JSON / 缺 DOZER_SESSION_ID / 未知 session_id:丢弃 + tracing 日志,不回错(单向语义)。
- OSC 序列超长/畸形:扫描器按上限截断放弃、字节原样透传,渲染不受损。
- install:settings.json 不存在则创建;JSON 解析失败则报错退出**不写**(不破坏用户配置);已有其他 hooks 原样保留。
- 非 zsh `$SHELL`:不注入,OSC 功能静默缺席,其余一切照常。

## 6. 测试策略

Headless(延续 P1b-d 惯例):
- osc.rs:OSC 7/133 识别、跨 chunk 截断续接、超长放弃透传、混杂 CJK 输出不破坏——纯函数单测。
- dozerd:HookEvent 入站 → 广播到多客户端 + registry 最新态 → 晚 attach 的 ListSessions 可见——集成测。
- dozer-hook:payload 组装/env 缺失退出码——单测;install/uninstall 幂等与合并保真——tempdir 单测。
- workspace:hook 事件序列 → 四态迁移 → 胶囊/标题渲染断言。

人工验收清单(草案,验收权在用户):
1. 新 tab `cd` 几层 → tab 标题跟随目录名;`false` 一下 → 状态条现 `exit 1` 提示
2. `dozer-hook install` → `~/.claude/settings.json` 出现条目且原有内容无损;重复执行无重复条目
3. tab 里跑 `claude` 派个活 → 状态点绿(运行中)→ 提问时紫蓝(待输入)→ 回合结束金色
4. 关 app 重开 → 状态胶囊恢复(registry 最新态)
5. 停掉 dozerd 后在别处跑 claude → agent 无感知不报错
6. `dozer-hook uninstall` → 条目干净移除

## 7. 备选方案(已否)

- **daemon 侧 OSC 解析**:见 D1。
- **transcript JSONL tail 代替 hooks**:免注册,但格式无契约易碎,且 hooks 是规格 §4 点名的对话结构化数据源;transcript 适配器另有其位(P1f/g 会话审阅)。
- **一步到验收闭环**:UI 面(S0/S0b/S2/S3 四页)+ SQLite + git ref 一起上,步子过大,且交付声明仍要先有本期的事件通路。
