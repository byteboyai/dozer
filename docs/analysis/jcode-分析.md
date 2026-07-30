# jcode 代码级分析

> 分析对象:https://github.com/1jehuang/jcode (本地副本 `/Users/chrischiang/AI/jcode`)
> 分析日期:2026-07-28 · Rust 70.7 万行 / 80 crate / 611 测试 / MIT · 12,320 star

## 一句话定位,以及一个范畴警告

jcode **不是**又一个 kooky/orca/workmux 式的"agent 编排/驾驭层"——它自己就是一个完整的编码 agent harness,**直接对标并替代 Claude Code/Codex CLI 本身**,自带对 20+ 家模型供应商(Anthropic/OpenAI/Gemini/Bedrock/OpenRouter/Copilot/Antigravity/Cursor/DeepSeek/Groq/…)的原生 tool-calling 循环,不需要外部 agent CLI 存在。三个前序分析对象(kooky/orca/workmux)全部"驾驭已有的 agent CLI",jcode 是**要取代它们**。

这个范畴差异直接影响怎么读它:jcode 的整体产品形态(自己做模型循环、自己做 TUI 前端)与 Dozer"agent 中立、站在用户侧治理已有 agent"的路线是**相反**的选择,不构成架构模板;但它 80 个 crate 里有好几块子系统——命令风险分级、agent 记忆、多 agent swarm 协作、RAM 工程——精确对应 Dozer 自己规格里"二期编排/三期记忆"的路线图空位,以及"验收/治理"这个核心定位下还没人做过的具体机制,值得单独拆开看。

代码体量上,jcode 70.7 万行是 workmux(7.6 万)的 9.3 倍、kooky(2.1 万)的 33.7 倍,比 orca(39.4 万)还大 1.8 倍——四个分析对象里规模最大,`jcode-tui` 一个 crate(19.3 万行)就比 workmux 全部代码还多。这个体量对应的是"自己实现完整 agent 循环 + 20 家供应商适配 + 自绘 TUI 渲染引擎 + 语义记忆图 + swarm 协作服务端",是四个项目里唯一同时吃下"agent 循环"和"终端渲染"两座山的。

## 工程结构:80 个按能力切分的 crate

无单体 `src/`,主 workspace 是薄壳(`src/` 仅 4 个文件),逻辑全在 `crates/*` 里按能力切碎——供应商(`jcode-provider-{anthropic,openai,gemini,bedrock,openrouter,copilot,antigravity,cursor}[-runtime]`)、TUI(`jcode-tui-{core,render,markdown,mermaid,permissions,session-picker,account-picker,usage-overlay,visual-debug,workspace,style,anim,tool-display}`)、记忆(`jcode-memory-types`/`jcode-compaction-core`/`jcode-embedding`)、协作(`jcode-swarm-core`/`jcode-overnight-core`)、平台(`jcode-desktop`/`jcode-desktop2`,GUI 壳;`ios/`,原生 iOS 客户端筹备中)。最大的四个 crate——`jcode-tui`(19.3 万)、`jcode-app-core`(12.9 万)、`jcode-base`(10.1 万)、`jcode-desktop`(7.3 万)——占了总量的一半以上,是真正的核心。

## 核心机制一:命令风险分级(`jcode-command-risk`)——四个项目里唯一的"治理"实现

这是对 Dozer 最直接相关的一块。`jcode-command-risk` 是一个纯静态、零网络调用的 shell 命令风险分类器,给 agent 每次 `bash` 工具调用做二级门禁的第一级:

```
Safe        无破坏可能,直接放行
Low         破坏有界(工作目录内/可 git 恢复/临时目录下),放行但记录
Confirm     不可逆且越出工作目录,要求模型对着用户原始请求"再证明一次"才放行(reflection turn)
Catastrophic 会摧毁用户主目录/根目录/凭证,硬拒绝,不接受任何模型说辞
```

源码注释直接引用了触发这个设计的真实事故:"issue #604,一个用户丢了整个 home 目录"——`ToolRegistry::execute` 原本只有一个默认关闭的 `pre_tool` hook,模型决定跑 `rm -rf ~` 就会被直接执行。

两个设计决策值得记下来:

1. **按"破坏半径"分类,不按命令名分类**——denylist 挡得住 `rm -rf` 挡不住 `find -delete`、`shred`、`truncate`、`dd`、`> file`。分类器做的是语义判断(会毁掉什么、能不能恢复),不是字符串匹配。
2. **宁可错杀,不可放过(bias toward recall)**——解析有歧义时一律升级到更高风险档,因为"假阳性成本是一次多余的 reflection turn,假阴性成本是一个 home 目录"。

`Confirm` 档不是硬拦截,是**逼模型对着用户真实意图重新自证**(reflection gate,阶段二,只有阶段一非 Safe 才触发,常见安全路径零开销);`Catastrophic` 档才是绝对拒绝,且**不依赖命令解析的正确性**——路径式的黑名单兜底,承认"解析可以被 `sh -c "$(printf ...)"` 这类构造绕过"这一现实,防御纵深不是沙盒。

配套的 `jcode-tui-permissions`(866 行)是待决权限请求的审阅队列 UI——批量列出、逐条批准/拒绝、支持"deny 附理由"。

**对 Dozer 的意义**:kooky/orca/workmux 里没有一个做"agent 要执行的动作值不值得被拦下来"这件事(workmux 的 sandbox 是外部隔离边界,不是判断)。这恰好是 Dozer"治理"定位下应该有、但目前 spec 里完全没提过的一层。关键的可行性问题是:**jcode 能做风险分级是因为它自己就是 tool-calling 循环的执行者,能在执行前拦下来;Dozer 不是**——Dozer 只能通过 Claude Code 的 `PreToolUse` hook(dozer-hook 已经订阅这个事件)拿到工具名和参数,要拦截需要让 hook 返回非零退出码(Claude Code 的 hook 协议支持用退出码 2 阻断工具调用)。这条路径理论上可行但目前 `dozer-hook`/`dozerd` 完全没有走到"根据内容判断、决定阻断"这一步,只是被动记录状态——如果要做,`RiskLevel` 的四档分类逻辑(而不是分级机制本身,机制要重写)是最直接能搬的部分。

## 核心机制二:Agent 记忆(`jcode-memory-types` + `jcode-compaction-core` + `jcode-embedding`)

对应 Dozer 路线图"三期:自治与记忆、多 agent memory 共享(agent 平台)"这个目前只有一句话的空位,jcode 给出了一份已经在生产里跑的具体实现:

- **写入路径**:每个对话回合被嵌入为语义向量;每隔一定轮次/语义漂移量/会话结束等触发点,一个"记忆侧 agent"(memory sideagent)从对话里抽取记忆条目,写入一张记忆图(`MemoryGraph`,带 `Edge`/`EdgeKind`/`TagEntry`,是图不是扁平列表)
- **读取路径**:每个新回合都对记忆图做一次余弦相似度检索,命中的记忆注入上下文;可选再过一层验证 side-agent 判断相关性,避免"检索到了但不相关"污染上下文
- **主动检索**:harness 提供显式记忆工具(agent 可以主动查/存),外加传统 RAG 式的历史会话搜索,被动注入和主动调用两条路都留了
- **维护**:ambient 模式下定期整理记忆图——查重、查过期、查冲突,不是写入后永久不变
- **配套的上下文压缩**(`jcode-compaction-core`):独立的 token 预算管理,`DEFAULT_TOKEN_BUDGET=200_000` 对齐 Claude 实际上下文上限,80% 阈值触发压缩、95% 阈值触发同步硬压缩(丢老消息保证这次 API 调用不失败)、双档压缩策略(保留最近 N 轮 + emergency 模式砍工具结果/图片到字符上限)——这套参数化的压缩策略本身就是一份可直接参考的"上下文快满了怎么办"的工程清单

**对 Dozer 的意义**:三期"记忆"目前在 Dozer spec 里没有任何设计细节,jcode 这套"图结构记忆 + 语义检索 + 侧 agent 验证 + ambient 整理"的架构是目前四个分析对象里唯一给出记忆子系统参考实现的,排三期设计时应该回来看这一节,而不是从零推演。

## 核心机制三:Swarm 多 agent 协作(`jcode-swarm-core` + `jcode-overnight-core`)

对应路线图"二期:编排与资产(orca 视野)"。设计比 orca 分析文档描述的编排更进一步——orca 是"人类在调度台上开 worktree、指派任务",jcode 的 swarm 是**agent 之间直接协作**:

- 同仓库内起两个以上 agent,服务端自动管理;A 改了 B 读过的文件("代码在脚下位移"),服务端主动通知 B,B 自行判断是否需要看 diff、要不要处理冲突
- 消息能力:DM 单个 agent、广播给所有 agent、广播给同仓库内的 agent 三种粒度
- agent 自己可以再 spawn 子 swarm(`MAX_SWARM_MEMBERS=1000`),主 agent 自动变成协调者、子 agent 变成 worker——可以无头(headless)或有头运行
- 完成报告有强制摘要约束(`SWARM_TLDR_REQUIRED_OVER_CHARS=240` 时必须附 ≤200 字符的 `tldr`),防止长报告直接倾倒进其他 agent/协调者的上下文

**对 Dozer 的意义**:这是"文件层面的实时协作感知"(而不是"开 worktree 完全隔离,merge 时才碰面")的具体实现,和 kooky/orca/workmux 共同验证的"一 agent 一 worktree,靠 git 隔离"标准件是**不同的哲学**——jcode 的立场是"worktree 不是给多 agent 协作场景设计的好方案"(README 原话:"git 显然不是为多 agent workflow 设计的,worktree 不是好方案"),二期设计时这是一个值得认真考虑的反方意见,不能因为前三个项目都用 worktree 就当成定论。

## 核心机制四:RAM/性能工程——为什么它自称"最省内存的 harness"

README 给了可验证的基准表(vs. pi/Codex CLI/OpenCode/Copilot CLI/Cursor Agent/Claude Code/Antigravity CLI):单会话 jcode 27.8MB,追加会话每个仅 +9.9~10.4MB;对照组里 Claude Code 单会话 212.7MB(21.5 倍)、OpenCode 追加会话 318.4MB(32.2 倍)。做到这一点的手段:

- 自己实现 scrollback(不满足于原生终端滚动能力,理由是"想要比原生更强的能力,比如局部平滑滚动")
- 因为自定义 scrollback 撞到了终端本身的能力天花板,**索性开始自己写一个终端**(`Handterm`,https://github.com/1jehuang/handterm,独立仓库,WIP)
- 自研 mermaid 渲染库(`mermaid-rs-renderer`),纯 Rust 无浏览器/TypeScript 依赖,号称比现有方案快 1800 倍
- "info widget"(信息控件只占屏幕负空间,没内容就自动让位)、千帧渲染上限(避免闪烁,不是为了真的跑千帧)

**对 Dozer 的意义**:这条不是"抄具体实现"的意义,是"工程决心"的参照——Dozer 已经在 canvas 逐格定位这件事上验证了同样的态度(为了终端渲染正确性,放弃更省事的 rich_text 方案)。jcode 把这条路走得更远(连滚动都不满足于原生终端,连 mermaid 渲染都要自己写),说明"native 终端 UI 这条路能走多深"这件事上限比想象中高,如果 Dozer 后续要打磨终端体验,jcode 的取舍(自绘 scrollback、自绘图表渲染)是可以对标的天花板参照,不是必须抄的下限。

## 工程文化:护栏比测试更早生效

`AGENTS.md` 里 `scripts/check_guardrails.sh` 是 CI 强制的"格式+质量护栏":fmt、`clippy -D warnings`,外加warning 数量、代码体积、测试体积、panic 使用、吞掉的 error、依赖边界、通配符 re-export 这几项的 **ratchet(棘轮)**——只能变好不能变差,新增违规直接挡 CI,但存量问题不需要一次清零(`--fix` 用于有意增长时重新定基线)。这是 707K 行 / 80 crate 规模下防止腐化的具体机制,不是抽象原则。

**对 Dozer 的意义**:Dozer 现在 1 万行,规模小,还没到"需要棘轮防腐化"的痛点,但这类机制**越早引入成本越低**——等到规模膨胀到需要它的时候,存量违规已经多到没法一次清零了。工程文化上的启示:priority 应该是"现在开始积累这类护栏脚本",而不是"等规模大了再补"。

## 一个明确的反例:jcode 尝试过"包一层 Claude Code CLI",后来放弃了

`crates/jcode-provider-claude-cli-runtime` 的文件头注释写着"Deprecated Claude CLI provider runtime (subprocess transport)"——jcode 一度用子进程方式包装真实的 Claude Code CLI 作为一个 provider,后来废弃,转向直接对接 Anthropic API 自己做 tool-calling 循环。同时它保留了"读取其他 harness(Codex/Claude Code/OpenCode/pi)的会话文件、在 jcode 里继续跑"这个会话吸收能力——**包一层 vs 吃进来重做**,jcode 试过前者、选择了后者。

这对 Dozer 是一个值得记住的反面参照:Dozer 的路线是"包一层"(driving 已有 agent CLI,不重做 agent 循环),jcode 用真实产品决策验证了这条路径**存在被放弃的先例**——原因大概率是"包一层"在深度控制力(比如这里分析的命令风险分级、精细化的上下文压缩)上有天花板,子进程边界挡住了很多东西。这不构成"Dozer 应该重新考虑agent 中立"的理由(定位不同,Dozer 的价值在治理层不在模型循环),但如果未来"治理"想做得更深(比如真正意义上的执行前拦截,而不是 hook 退出码这种间接手段),会撞到 jcode 撞过的同一堵墙,提前知道这一点比临时发现更好。

## 对 Dozer 的启示汇总

| jcode 子系统 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| `jcode-command-risk` 四档风险分级 | spec 里"验收/治理"定位,目前无对应机制 | 分级逻辑(按破坏半径而非命令名、宁可多问不可漏判)可直接参考;执行机制需要重新设计,因为 Dozer 没有 jcode 那样的执行前拦截点,只能走 `PreToolUse` hook 退出码这条更弱的路径 |
| `jcode-memory-types`/`compaction-core` | 路线图三期"自治与记忆",目前只有一句话 | 排三期设计时的具体参考起点:图结构记忆 + 语义检索 + 侧 agent 验证 + ambient 整理;压缩策略的分级阈值(80%/95%)可直接参考 |
| `jcode-swarm-core` | 路线图二期"编排与资产",目前只有一句话 | 文件层面实时协作感知(而非纯 worktree 隔离)是二期设计时应该纳入对比的另一种范式,不要默认"一 agent 一 worktree"是唯一答案 |
| RAM 工程(自绘 scrollback/终端/mermaid) | Dozer 已验证的"canvas 逐格定位"同类态度 | 不是具体代码可抄,是"这条路能走多深"的参照上限 |
| `scripts/check_guardrails.sh` 棘轮护栏 | Dozer 当前工程文化 | 规模小的现在就该开始引入,越晚成本越高 |
| claude-cli-runtime 被废弃的历史 | Dozer"agent 中立、包一层"的路线选择 | 不是否定依据,是"这条路的天花板在哪"的提前预警——想在治理深度上更进一步时会撞到同一堵墙 |
