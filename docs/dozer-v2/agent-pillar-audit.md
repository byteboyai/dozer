# Agent 支柱分布审计(Q15)

> 文档性质:现状审计,**不是已批准规格**。日期:2026-10-15。基于 `main` = `7cdbffd0` 的静态 grep 与行数统计,未跑运行时。
> 回答 `面板独立性与服务化要求.md` §6 的 Q15:"Agent 能力在 `dozerd` 与 `dozer-app` 中的分布是否也像 Git 一样散落;是否抽成与 Git 对称的底层"。

## 1. 结论

**散落程度与 bytegit 之前的 Git 同量级,但形状不同:Git 是"同一件事在 7 处各写一份",Agent 是"每家 agent 的私有知识按关注点散在 22 个文件里"。** 没有任何一个 crate 或模块是"Agent 的单一事实来源"。

## 2. 新增一个 agent 要改多少处(代理指标)

最近接入的 `V8agent`(2026-08-24)在非测试代码里出现在 **22 个文件、5 个 crate**:

| crate | 文件(出现次数) |
|---|---|
| `dozer-model` | `agent.rs`(3)、`todo.rs`(1) |
| `dozer-core` | `agent_paths.rs`(10)、`protocol.rs`(6) |
| `dozer-mcp` | `install.rs`(2) |
| `dozerd` | `transcripts/{mod(19),parse(18),scan(14)}.rs`、`headless_agent.rs`(17)、`summary_config.rs`(11)、`summary_provider.rs`(4)、`default_agent_config.rs`、`session_summary.rs`、`summary_jobs.rs`、`todo.rs`(各 1) |
| `dozer-app` | `workspace/hook.rs`(10)、`workspace/view.rs`(5)、`extensions/todo/protocol.rs`(4)、`workspace/state.rs`(2)、`extensions/conversations.rs`、`app/layout.rs`(各 1) |
| `dozer-panel-usage` | `aggregate.rs`(4) |

其它 agent 同量级:Codex 34、Codebuddy 24、Goose 23、Opencode 23、Aider 19 个文件。`AgentKind::` 在各 crate 的出现次数:`dozerd` 363、`dozer-app` 278、`dozer-panel-usage` 99、`dozer-core` 47、`dozer-hook` 29、`dozer-model` 24。

## 3. 按关注点的分布

| 关注点 | 位置 | 行数 | 性质 |
|---|---|---|---|
| 身份与展示(label、display_label、颜色、图标) | `dozer-model/agent.rs`、`workspace/view.rs`、`workspace/state.rs` | ~190 | 纯静态事实 |
| 路径与配置(会话/配置目录) | `dozer-core/agent_paths.rs`、`dozerd/default_agent_config.rs` | ~350 | 纯静态事实 |
| 状态检测(hook → 事件) | `dozer-hook/*`(2.5k,含 aider 桥接 580+308、goose、opencode 的安装器)、`workspace/hook.rs` 485、`dozerd/session.rs` 552 | ~3.5k | 每家各写一套 |
| 会话文本解析(transcript) | `dozerd/transcripts/{mod 2160, parse 1905, scan 317}`、`dozer-app/transcript.rs` 827 | ~5.2k | 每家各写一套 |
| 无头一次性调用 | `dozerd/headless_agent.rs` 1038、`summary_provider.rs` | ~1.6k | 每家参数/输出协议不同 |
| MCP 挂载 | `dozer-mcp/install.rs` 472 | ~470 | 每家配置格式不同 |
| 终端/PTY 渲染 | `dozer-app/term/*` 2.1k | ~2.1k | 与 agent 种类无关,但与 Agent 面板、终端区纠缠(H0 的 O1/O6) |

合计 agent 相关代码约 **1.3 万行**。

## 4. 与 Git 的差异(决定能不能照搬 bytegit 的做法)

- bytegit 能成立,是因为 Git 能力是**无状态、同步、有统一语义**的查询与写操作。Agent 的"私有知识"也是静态的(路径、命令行、解析格式、hook 格式),可以收口;但 **PTY 池、会话存活、hook 事件接收是有状态的运行时**,已经在 `dozerd`,不该进库。
- 因此对称的底层不是"一个 `byteagent` 全包",而是两层:
  1. **Agent 档案(profile)**:每家 agent 的静态事实与能力开关,一处声明,其余代码查表。
  2. **按能力的适配器 trait**:`transcript 解析`、`hook 安装/事件映射`、`headless 调用`、`MCP 挂载`,每家一个实现,按 `AgentKind` 注册。运行时(PTY、会话)仍留在 `dozerd`。
- 不能直接照搬的一点:Git 当时有 180 个调用点要搬;Agent 的 22 个文件里大部分是 `match AgentKind` 的**展示与分流分支**,迁移形态是"把分支换成查表",不是"把代码搬到别处"。

## 5. 建议的第一刀(量得出来的)

**目标指标:新增一个 agent 需要改动的非测试文件数,从 22 降到 ≤ 8**(身份/档案 1 处,四类适配器各 1 处,加少量不可避免的 UI)。

第一刀只做第 3 节的前两行(身份展示 + 路径配置):建一张 `AgentProfile` 表,把 `label`、`display_label`、颜色、图标、启动命令名、会话/配置目录、能力开关收进去;`workspace/view.rs`、`workspace/state.rs`、`agent_paths.rs`、`default_agent_config.rs` 里按 `AgentKind` 的 `match` 改为查表。**不碰**解析器、hook、headless、PTY。收益是可量的(文件数、`match AgentKind` 数),风险低(纯静态数据,行为可由现有测试钉住)。

## 6. 未解决 / 需要用户裁决

- **Q15-1:** Digger 要不要 agent?Digger 的文档里 agent 是写作辅助,不一定是 PTY 里的 CLI agent;若 Digger 只要"头像 + 用量归属",第一刀的收益已够,不必做适配器层。
- **Q15-2:** 适配器层(第 3 节后五行)是否在 Agent 档案之后继续做,取决于近期是否还要接入新 agent(Kilo 仍是已知缺口,见 memory)。没有新 agent 需求时,建议停在档案层,与 H8 的"有第二个消费者再做"同一原则。
- **Q15-3:** `term/*` 与 Agent 面板的纠缠(O1/O6)不在本审计范围,需单独评估。

## 7. 实测:qodercn L1 接入(2026-10-15,分支 `feat/agent-qodercn`,提交 `bc550767`)

第 2 节的代理指标有了一个真实样本:接入 Qoder CLI CN 的 L1(终端启动、hook 状态检测、MCP 注册、展示),**改了 16 个非测试/含测试的文件,分布在 6 个 crate**(`dozer-model`、`dozer-core`、`dozer-hook`、`dozer-mcp`、`dozerd`、`dozer-app`),与 V8agent 的 22 个文件同量级。

编译器只抓出了其中一部分。**有两类位置编译器抓不到,是这套结构的真正风险:**

1. **带 `_ => Unknown` 兜底的字符串映射**:`session_summary.rs`、`summary_jobs.rs` 各有一份 `agent_from_str`。`agent_to_str` 经 `label()` 会写出 `"qodercn"`,读回却变成 `Unknown`——**写得进、读不回,且没有任何编译错误或运行时报错**。靠新增的往返测试才发现(三处映射现各有一个 `every_agent_*round_trip*` 测试)。
2. **"无解析器却照样被调用"的副作用**:qodercn 的每个 hook 事件都带 `transcript_path`,会触发 `ingest_session`。即使解析器返回空,它仍会建出 0 轮的空会话行并把 `parsed_offset` 推到文件尾——将来做 L2 时已有内容就读不到了。已在入口短路,并有"去掉保护则失败"的测试。

这两点支持第 5 节的方向:把 agent 的静态事实与持久化名收进**一张档案表、双向映射由同一处生成**,比继续在各处手写 `match` 更安全。

另一个相关发现:`byteui` 的语义色已被 8 家 agent 用完(`ignored` 与 `dim` 数值相同)。qodercn 目前用 `red`,有"像错误色"的歧义;根治需要 byteui 增加 agent 专用色并发版。

L1 之外未做(各需单独验证):transcript 解析与用量/会话历史(需要真实对话样本)、headless 调用与总结 provider(需核实 `-p`/只读权限)、前端里的 agent 映射(`usage-content`、`review-trace`)。

## 8. 跟进:AgentKind 表化第一刀(2026-10-16,分支 `feat/v2-hygiene`)

第 5 节建议的"第一刀"落了第一步,只做**身份/档案 + 路径配置**这一层,不碰解析器/hook/headless/PTY:

- `dozer-model/src/agent.rs` 新增 `AgentKind::ALL`(`[AgentKind; 9]`)、`known()` 迭代器、
  `from_label()`;测试里的 `variant_index` 用无通配 `match` 强制"新增变体即编译失败"。
- `dozer-core/src/agent_paths.rs` 新增 `projects_root_in()` / `project_session_dir_in()`(无通配,
  新变体必须表态),`dozerd/src/transcripts/scan.rs` 与 `transcripts/mod.rs` 的目录枚举改为表驱动,
  删掉 `AGENT_ROOTS`。**这顺带修掉一个 bug**:v8agent 的删除路径原先漏在表外(见下)。
- `workspace/view.rs::agent_picker_spec()` 的 8 项硬编码菜单改为从 `AgentKind::known()` 派生
  (按展示名字母序),新 agent 自动出现。
- `session_summary.rs` / `summary_jobs.rs` 的手写 `agent_from_str` 收敛为
  `AgentKind::from_label(...).unwrap_or(Unknown)`,靠已有往返测试钉住。

**量出来的结果:接入新 agent 的"档案 + 路径"部分从手写 22 处降到填表 + 少量不可避免的 UI
分支。** 剩下的非通配 `match`(颜色/图标/启动命令/hook/headless/解析器)按 §1 的清单逐处表态,
编译器会一条条报出来。

**清单文档:** 接新 agent 时逐条对照 [`adding-an-agent.md`](./adding-an-agent.md)——它把"编译器
抓得到的点位、抓不到的 `_ =>`/前端字符串映射、持久化名不可改、门禁"都固化下来。

**仍未解决(维持 §6):** §6 的 Q15-1/Q15-2/Q15-3 裁决未变;`byteui` agent 专用色仍未加。
