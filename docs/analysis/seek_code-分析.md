# SeekCode 代码级分析

> 分析对象:https://github.com/kafkazhang/seek_code (本地副本 `/Users/chrischiang/AI/seek_code`)
> 分析日期:2026-07-28 · 版本 0.5.5 · TypeScript 1.4 万行 / Electron+React / MIT · 26 star

## 一句话定位

SeekCode 和上次分析的 jcode 是**同一范畴**:不是编排层,是agent 本身——Electron 桌面壳里跑一个完整的工具调用循环,深度适配 DeepSeek-V4,自称"融合 Claude Code(结对编程)与 Codex(任务委派)思路"。区别在规模量级:jcode 70.7 万行、80 crate、12K star,是团队级产品;SeekCode 1.4 万行、单体 Electron 应用、26 star,是**个人开发者规模**的同类实现。这个量级差恰好让它成为一份有用的对照——同一类问题(命令安全、出网控制、agent 记忆、子任务编排),在"没有团队资源死磕"的约束下,答案长什么样。

对 Dozer 的意义和 jcode 一样是"拆开看子系统",不是整体架构模板——Dozer 驾驭已有 agent CLI,SeekCode 和 jcode 一样是自己重做 agent 循环,产品路线相反。

## 核心机制一:命令安全——正则黑名单,而非语义分级

`src/main/cmdsafety.ts`(89 行,零 Electron/Node 依赖,纯函数便于单测)做两件事:

- `isAppKiller`:识别"会误伤应用自身"的命令(按进程名批量杀 node/electron/seekcode),硬拒绝
- `isDangerousCommand`:一份相当详尽的破坏性命令正则表(`rm -rf`、`dd`、`mkfs`、`git reset --hard`、`git push --force`、fork bomb 模式、Windows 的 `Remove-Item -Recurse`/`diskpart`、docker 批量清理…)

这和 jcode 的 `RiskLevel` 四档分级(按"破坏半径"语义判断)是**同一问题的两种答案**:jcode 的方案更本质(能识别 `find -delete` 这类不含 `rm` 字样但同样致命的命令),SeekCode 的方案是穷举正则,注释里能看到"新增:xxx"的迭代痕迹——**每次漏判之后补一条正则**,这是资源有限时最诚实的演进路径,也是这类静态分类器天然的维护模式(黑名单会不断长,永远追不完)。

真正值得记的是**审批判断和权限模式的关系**(`src/main/agent/tool_exec.ts:81-88`):

```ts
const dangerousCmd = (t.name === 'run_command' || t.name === 'run_background')
  && isDangerousCommand(args.command ?? '')
...
if (needsApproval(perm, t.name) || dangerousCmd || mcpNeedsApproval || fetchNeedsApproval) {
  // 弹审批,即使 perm === 'auto'
}
```

四档权限模式(`ask`/`acceptEdits`/`plan`/`auto`,README 明确写"参考 Claude Code")本身只决定"默认要不要问";`dangerousCmd` 命中时**无视当前权限模式强制弹审批**,连"全自动"档都拦——这条"静态危险分类器可以否决运行模式"的设计和 jcode 的 `Catastrophic` 档(不接受任何模型说辞)是同一个原则,只是分类器精细度不同。**两个独立项目在这一点上给出了相同答案**,值得当作这类"危险动作强制打断"设计的共识,而不是某个项目的个人偏好。

## 核心机制二:出网白名单闸门——单一收口点,而非沙盒

`src/main/egress.ts`(69 行)是主进程唯一允许出网的入口 `guardedFetch`,三级白名单:

1. **推理出口**:DeepSeek baseURL(用户代码/上下文唯一去处)
2. **生态出口**:固定的公共只读源(GitHub、MCP registry、context7)——用于装 Skills/MCP,不带用户代码
3. **用户显式信任**:用户主动配置的远程 MCP server host,连接时动态登记

非白名单 host 直接抛错拒绝;所有请求走 Electron `net.fetch`,复用渲染层同一套 session/webRequest 栈,不是另开一条不受审计的出网路径。

这是 workmux sandbox 里 CONNECT 代理+域名白名单那套机制的**极简版**——没有 iptables、没有独立代理进程、没有 host↔guest RPC 桥,就是一个函数级的"发请求前检查 host 在不在表里"。工程量差两个数量级,但解决的是同一个问题:**agent 的网络访问面必须显式收敛,不能是"默认能访问任何东西,靠自觉不滥用"**。对不需要容器/VM 级隔离的场景(比如 agent 本身就在受控的宿主进程里跑,不是跑在完全不受信任的沙盒里),这个"单一收口函数+白名单"模式比 sandbox 子系统轻量得多,是更现实的起点。

## 核心机制三:双层记忆——比 jcode 的记忆图更朴素的可行方案

`working_memory.ts`(会话内)+ `memory.ts`(跨会话)分工明确写在注释里:

- **工作记忆**(`working_memory.ts`):内存 KV,按 sessionId 分区,LLM 执行中发现的关键实体(端口号/文件路径/错误信息/配置值)可被工具写入;**上下文压缩时作为独立 system 消息保留,不参与摘要裁剪**——解决"长任务中关键发现随压缩丢失"这个具体问题。有界:单会话最多 30 条,key ≤60 字符、value ≤400 字符,满了淘汰最早条目;会话结束即清空,不跨会话
- **长期记忆**(`memory.ts`,file-backed):`SEEK.md`/`memory.md` 落盘,跨会话保留

对比 jcode 的记忆图(语义嵌入 + 余弦检索 + 侧 agent 验证相关性 + ambient 定期整理):SeekCode 这套没有嵌入、没有检索,就是一个有界 KV + 一个 markdown 文件。**但"工作记忆在压缩时被钉住不裁剪"这个具体机制,jcode 的记忆图设计里没有对应物**——jcode 解决的是"记忆的新写入/检索",SeekCode 解决的是"当前任务的关键事实不能在压缩时被摸掉",是两个正交的问题,后者更小但更容易在 Dozer 现有的 P1 范围内落地(Dozer 目前没有上下文压缩机制,但如果做,这条"压缩时钉住关键工作记忆"的经验值得直接搬)。

## 核心机制四:子代理编排——jcode swarm 的克制版

`subagents.ts`(162 行):主对话 agent 调 `spawn_subagents` 把独立子任务并发委派出去(上限 6 个任务、3 个并发),每个子代理是一次独立 agent 循环,继承父会话权限模式与项目目录,**审批经父会话通道上浮**(摘要带"子代理"标识),完成后回灌一份长度受限(1600 字符裁剪)的结果报告给编排者;**子代理不可再派生子代理**(硬编码防递归,不是软约束)。

和 jcode 的 swarm 对比:jcode 做的是"同仓库内多个平行 agent 之间互相感知、能收到别人改了自己读过的文件的通知、能互相发消息";SeekCode 的子代理**互相看不见**(prompt 里明确写"你看不到主对话历史,任务描述即全部上下文"),只是"并行执行 + 结果收敛回主 agent"的经典 fan-out/fan-in,没有 jcode 那套跨 agent 消息/冲突感知层。这是"如果二期不需要 agent 间实时协作,只需要把一个任务拆成几个独立子任务并行跑"这种更小需求的现成参考——比 jcode swarm 便宜得多,如果 Dozer 二期编排的第一步只是"并行派单"而不是"多 agent 协作同一份代码",这个模型是更合适的起点,不必一步到位抄 swarm。

## 一个明确的取舍:放弃 PTY,换取零原生依赖

`src/main/terminal.ts` 开头写明"轻量终端:用系统默认 shell 执行单条命令并流式回传输出。**非 PTY(无需 node-pty 原生依赖)**,适合 git/npm/构建/测试等命令"。这意味着 SeekCode 内置终端跑不了 vim/top 这类需要真实 TTY 语义的全屏程序,cwd 靠渲染层显式跟踪、每条命令重新传入(不是 shell 进程自己维护的 cwd)。

这是和 kooky/orca/Dozer 的路线**相反**的选择——Dozer spec 明确要"原生 agent 终端…流畅度对标 kooky"、"PTY 由常驻 daemon 持有",这条不适用。但值得记录为一个清晰的成本参照:**放弃 PTY 换来的是"不需要 node-pty 这个常见故障源"(原生模块跨平台编译/权限问题是 Electron 生态出了名的坑)**——如果 Dozer 未来在某个受限场景(比如某个不需要真实终端交互、只需要跑固定命令流的子功能)遇到类似取舍,这是一个已经在生产验证过的"降级方案"存在的证据,不是无人区。

## 对 Dozer 的启示汇总

| SeekCode 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| `dangerousCmd` 无视权限模式强制审批 | spec"治理"定位,与 jcode command-risk 的 `Catastrophic` 档同一原则 | 两个独立项目收敛到同一条:静态危险分类器应该能否决"自动模式"。Dozer 若做命令风险门禁,这条应作为设计约束,不只是 jcode 一家之言 |
| `guardedFetch` 三级白名单单一收口 | 若 Dozer 需要限制 agent 网络访问面,又不想上 workmux 级 sandbox | 比容器/代理轻两个数量级的现实起点:一个函数 + 白名单表,不需要 iptables/RPC 桥 |
| `working_memory.ts` 压缩时钉住关键事实 | 路线图三期"自治与记忆";Dozer 目前无上下文压缩机制 | jcode 记忆图之外的正交小问题的直接解法,比嵌入检索便宜得多,值得在设计压缩机制时一并纳入 |
| `subagents.ts` fan-out/fan-in 无跨 agent 通信 | 路线图二期"编排与资产" | 比 jcode swarm 更小的第一步参考——先做"并行派单+结果收敛",不必一步到位做"多 agent 实时协作同一份代码" |
| `cmdsafety.ts` 正则黑名单 vs jcode 语义分级 | 同上,风险分级机制的实现选择 | 两种真实存在的实现路线,黑名单起步快但会不断追加正则;分级更本质但工程量更大——排期时按团队资源量级选,不是非此即彼 |
| 放弃 PTY 换零原生依赖 | Dozer 已定"原生 agent 终端对标 kooky",此路线不适用 | 仅作为"受限场景下的已验证降级方案"记录,非当前借鉴项 |
