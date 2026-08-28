# 会话总结批量补录(项目修复触发,headless 一次性 agent 调用)

**状态:已批准(brainstorming 会话,2026-08-28)**

## 背景

会话总结生成管线(`docs/superpowers/specs/2026-08-27-session-summary-pipeline-design.md`)
已落地并合并 main:关闭 tab 时,若 `tab.agent` 属于对话摄取管线已覆盖的四家
(Claude/CodeBuddy/OpenCode/V8agent)且走 daemon 后端,会往活的 PTY 注入总结
prompt,agent 经 `dozer-mcp` 写工具把标题+摘要落进 `session_summaries` 表;
超时兜底走启发式(拼接人类回合)。

该 spec 明确把"项目新建/修复时批量补总结"列为非目标,理由有二:一是依赖一个
当时不存在的"headless 一次性 agent 调用机制"(启动一个 agent CLI 处理单次
prompt 后退出,不经过 PTY/Session/registry 那套长驻机制);二是 `dozer-app`
的 `extensions/project.rs` 不该直接耦合会话总结逻辑。本设计就是落地这个被
显式搁置的子项目。

**现状复核(2026-08-28,写这份 spec 前实测确认)**:

- `session_summaries` 表已存在(`crates/dozerd/src/session_summary.rs`),但
  该 store 自身没有"哪些 session 缺总结"的查询能力——这个能力已经在更上层
  拼好了:`Request::ListConversationsWithSummaries` 处理器
  (`crates/dozerd/src/server.rs:424-452`)把 `transcripts.list_conversations`
  与 `session_summaries.get_many` 按 `conversation_id` 拼成
  `Vec<(ConversationSummary, Option<SessionSummaryPayload>)>`,第二个元素是
  `None` 的行就是"缺总结的对话"。本设计直接复用这个查询,不在
  `session_summary.rs` 里新增 SQL。
- 全仓库(`dozerd`/`dozer-app`/`dozer-core`)搜索不到任何"非交互式启动 agent
  CLI 处理单条 prompt"的先例——唯一启动这四家二进制的地方是交互式 PTY 机制
  (`crates/dozerd/src/session.rs:76` 的 `CommandBuilder`)。本设计是这个
  能力的第一次实现。
- 项目生命周期依然没有 broadcast/pub-sub 机制,连现有的转录补录
  (`client.backfill_project_transcripts`,`extensions/project.rs:431`)本身
  都是 `project.rs` 直接硬编码调用 `dozer-client` 方法。之前 spec 提过的
  "项目生命周期动作应该走消息广播"是一条尚未兑现的原则性方向,本设计不趁机
  把它补上(那是独立体量的基础设施项目),照抄现有转录补录的直接调用模式,
  保持一致。
- "修复项目"(`Message::RepairProject`,`extensions/project.rs:381-383`)当前
  跑 4 个同步 scaffold 步骤(缓存目录/README/git 仓库/项目文档与 Agent
  记忆,`project_scaffold.rs:89-108`)+ 1 个异步转录补录步骤,全部跑完后才
  产出一份 `ScaffoldReport`,渲染成一行灰字摘要(`extensions/project.rs`
  807-879 区域)。没有任何"边跑边看进度"的 UI——本设计顺带把这个交互改造成
  实时清单弹窗(见下)。
- 四家 headless 一次性调用能力现状(2026-08-28 查证):
  - **Claude Code**(`claude`):`-p`/`--print` 官方支持,可配 stdin 输入、
    `--output-format text|json|stream-json`。
  - **CodeBuddy**(`codebuddy`):CLI 表面几乎是 Claude Code 的平行实现,同款
    `-p`/`--print`,但非交互模式**必须**加 `-y`/`--dangerously-skip-permissions`
    才能执行任何需要授权的操作。
  - **OpenCode**(`opencode`):子命令形式 `opencode run <message..>`,
    `--format json` 是 NDJSON 事件流而非单一 JSON 块,MCP 配置来自项目/用户
    配置文件,没有单次调用覆盖的 flag。
  - **v8agent**(`v8agent`,本机源码在
    `/Users/chrischiang/Projects/CoralProjects/byteboy/v8agent`,
    `crates/v8agent-cli/src/main.rs`):**没有任何参数解析**,只认几个环境
    变量;`repl.rs` 靠"stdin 不是 tty 时逐行读、EOF 退出"这个副作用凑出一个
    脆弱的"伪一次性"模式,且要求 prompt 必须是单行(换行会被拆成多轮);
    `render()` 把工具调用 trace(`[tool] name(args)`)和模型流式文本交织
    写进同一个 stdout,没有干净的"最终答案"输出。四家里独此一家需要**跨
    仓库同步改动**才能满足本设计需要的"一次性调用 + 可解析的最终输出"。

## 目标 / 非目标

**目标**:

1. 新增本地配置文件(路径 `dozer_core::paths::config_dir().join("config.toml")`
   ——`config_dir()` 现有但此前从未被真正写入过任何内容),字段
   `default_agent: "claude" | "codebuddy" | "opencode" | "v8agent"`。无 GUI
   入口,用户手动编辑;`dozerd` 每次处理补总结请求时惰性读取(改文件不需要
   重启 `dozerd`),文件不存在或字段非法时回退到硬编码默认值(`claude`)并
   `tracing::warn!`。
2. `dozerd` 新增一个 headless 一次性 agent 调用子系统,按 `AgentKind` 分派
   到四套不同的进程调用方式(见"架构与数据流"),统一约定:无论走哪家 CLI,
   都要求模型把结果包进固定分隔符里的 JSON(`title`/`summary` 两个字段),
   `dozerd` 拿到完整 stdout 后用固定分隔符抠出这段文本再解析,不依赖各家
   CLI 私有的结构化输出包装格式(四家格式差异太大,统一在"最终文本内容"这一
   层面对齐,而不是在"进程输出协议"这一层面对齐)。
3. `crates/v8agent-cli`(独立仓库
   `/Users/chrischiang/Projects/CoralProjects/byteboy/v8agent`)同步新增:
   一个真正的参数解析后的一次性模式(单个 prompt 输入,不再依赖"stdin 非
   tty + 单行"这个副作用式技巧)、一个不与工具调用 trace 交织的"最终结构化
   输出"通道。这是本设计范围内**唯一**跨越到 Dozer 仓库之外的改动。
4. `dozer-core::protocol` 新增 `Request::BackfillSessionSummaries { cwd }`
   (fire-and-forget,立即 `Reply::Ok`,不阻塞调用方)与
   `Request::GetSessionSummaryBackfillStatus { cwd }` →
   `Reply::BackfillStatus { total: u32, completed: u32 }`(供 UI 轮询进度);
   `dozer-client::Client` 加对应两个方法。
5. `dozerd` 收到 `BackfillSessionSummaries` 后,`tokio::spawn` 一个后台任务:
   复用 `ListConversationsWithSummaries` 的查询逻辑筛出该 `cwd` 下"有对话
   记录但没有 `session_summaries` 行"的会话列表,**依次串行**(不并发,避免
   同时拉起多个 agent 进程抢机器资源)对每条调用 headless 总结;成功则
   `status=ai_generated` 落库,失败/超时(90 秒,理由与既有关闭时管线的 60
   秒同款"经验值"风格——这里更长是因为要冷启动一个全新进程,比复用一个已经
   活着的 PTY 慢)复用现成的 `heuristic_from_turns` 走 `status=
   heuristic_fallback` 兜底落库,过程中维护一个内存态 `(total, completed)`
   计数器供 `GetSessionSummaryBackfillStatus` 查询。
6. `dozer-app` 侧 `extensions/project.rs`:
   - 在 `spawn_scaffold_run` 现有 4 个同步步骤 + 1 个转录补录步骤之后,追加
     一次 `client.backfill_session_summaries(cwd)` 调用——直接调用,不新增
     消息广播机制,与现有转录补录同款模式保持一致。
   - **"修复项目"交互改造**:点击后弹出一个模态弹窗(复用
     `project_delete_confirm_popup` 的 `Option<T> + stack![base, dismiss,
     popup]` 样板,`extensions/project.rs` 782-797/885 区域),列出全部 5
     个既有步骤(逐条 pending→running→done/failed 实时刷新,复用 `emit`/
     `EventLoopProxy::send_event` 本来就支持多次发送这一事实)+ 新增第 6
     行"补总结(completed/total)"聚合进度行,轮询
     `GetSessionSummaryBackfillStatus` 驱动数字跳动,不展开成逐 session 的
     单独行。弹窗进行中不可关闭(无点击遮罩关闭),全部 6 项(含补总结)完成
     后才能关闭。
   - 这次改造**不影响**项目静默打开时的隐式 scaffold 调用
     (`app.rs:5206-5215`,`visible=false`)——补总结步骤只在
     `Message::RepairProject`(显式点击)路径触发,静默路径维持现状不变、不
     弹窗、不补总结。
7. 全部改动落地后:`cargo build`(全 workspace)、`cargo test -p dozerd -p
   dozer-app -p dozer-core -p dozer-client`、`cargo clippy --all-targets --
   -D warnings`、`cargo fmt -- --check` 全绿;`v8agent` 仓库侧改动在其自己
   的仓库跑通对应测试;真机验证走人工(见"测试策略")。

**非目标**:

- **不做设置面板**——`default_agent` 只是一个用户手动改的本地配置文件字段,
  不新增任何 GUI 配置入口,现状"设置齿轮是纯视觉占位"
  (`topbar.rs:188-190`)维持不变。
- **不做"resume 原 session 上下文"式总结**——headless 总结时喂给默认 agent
  的是该 session 已有的人类回合文本(`conversation_turns` 表,复用
  `heuristic_from_turns` 同一数据源),不尝试让默认 agent 接回原会话的完整
  工具调用历史。这意味着补总结用的 agent 与该 session 原本用的 agent 可以
  是不同家——"默认 agent"是一个通用总结器角色,不是"接管原 agent 的会话"。
- **不引入项目生命周期广播/pub-sub 基础设施**——`project.rs` 继续直接调用
  `dozer-client` 方法,和现有转录补录同款,这条独立的架构原则依然只是方向,
  不在本设计里兑现。
- **不做静默打开项目时的自动补总结**——只有显式点击"修复项目"才触发,理由
  是每条补总结都是真实拉起一个 agent 进程(秒级到十几秒),静默路径若也触发
  会拖慢每次打开项目 tab 的体验。
- **不做补总结的批量数量上限/截断**——聚合进度行天然能容纳"缺很多条"的情况
  (不像逐行展开会导致弹窗过长),因此不额外设置"最多处理 N 条"的截断逻辑;
  一个项目缺几十条总结时,弹窗会显示较长时间的进度增长,这是已知、接受的
  权衡,不在本设计里优化。
- **不覆盖 Codex/Kilo/纯 shell/SSH 会话**——与上一份 spec 的排除范围一致
  (Codex 转录解析器恒返回空、Kilo 无真实摄取、纯 shell 无 agent 可调、SSH
  本来就不走这套清理路径),这四类的历史会话本来就不会出现在
  `ListConversationsWithSummaries` 的候选列表里(它们压根没有
  `conversation_turns` 数据),因此天然被排除,不需要额外过滤逻辑。
- **不支持"补总结进行中取消"**——弹窗进行中不可关闭,不引入取消标志位传递
  给后台循环的机制。

## 架构与数据流

### 默认 agent 配置

新增 `crates/dozerd/src/default_agent_config.rs`(或类似位置),结构:

```rust
#[derive(Deserialize)]
struct DefaultAgentConfig {
    default_agent: AgentKind,
}

fn load_default_agent() -> AgentKind {
    let path = dozer_core::paths::config_dir().join("config.toml");
    match std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str::<DefaultAgentConfig>(&s).ok())
    {
        Some(cfg) => cfg.default_agent,
        None => {
            tracing::warn!("default_agent 配置缺失或非法,回退到 Claude");
            AgentKind::Claude
        }
    }
}
```

每次处理 `BackfillSessionSummaries` 请求时调用一次(不缓存、不需要重启
`dozerd` 生效),开销可忽略(单次文件读取+小对象反序列化)。

### headless 一次性调用子系统

新增 `crates/dozerd/src/headless_agent.rs`,对外一个函数:

```rust
async fn summarize_headless(
    agent: AgentKind,
    human_turns: &[String],
) -> Result<(String, String), HeadlessError>; // (title, summary)
```

内部按 `agent` 分派到四个私有函数,各自构造 `tokio::process::Command` 并
`.output()` 拿到完整 stdout(**不是**流式读取,等进程退出后一次性拿全部
输出——headless 调用本来就是"发一个 prompt、等一个答案"的语义,不需要
边跑边读):

- **Claude / CodeBuddy**:`Command::new("claude"|"codebuddy").arg("-p").arg(FIXED_INSTRUCTION)`,
  人类回合拼接文本经 `.stdin(Stdio::piped())` 写入子进程标准输入。CodeBuddy
  额外加 `-y`(非交互模式跳过授权确认的必需参数,不加会导致调用失败,不是
  可选项)。
- **OpenCode**:`Command::new("opencode").arg("run").arg(format!("{FIXED_INSTRUCTION}\n\n{turns_text}"))`
  ——OpenCode 的 `run` 子命令没有独立的 stdin 输入通道,拼接文本直接作为
  message 参数的一部分传入。
- **v8agent**:`Command::new("v8agent")`,新的一次性 flag(具体名称在
  `v8agent-cli` 那边实现时定,如 `--once`)+ prompt 经 stdin
  写入(拼接文本允许包含换行,依赖 v8agent-cli 侧新增的"整体读 stdin 而非
  逐行读"能力,见下一节)。

四家统一的 `FIXED_INSTRUCTION`(措辞留实现阶段打磨,大意):"请阅读以下这
次会话中用户说过的话,生成一个简短标题和一段摘要,总结这次会话完成的工作。
只输出这一段,不要输出任何其他内容:
`<<<DOZER_SUMMARY_JSON>>>{"title":"...","summary":"..."}<<<END_DOZER_SUMMARY_JSON>>>`"。

拿到完整 stdout 后,`dozerd` 侧用固定分隔符做子串提取
(`<<<DOZER_SUMMARY_JSON>>>` 与 `<<<END_DOZER_SUMMARY_JSON>>>` 之间的内容)
再 `serde_json::from_str` 解析成 `{title, summary}`;提取失败(找不到分隔符、
JSON 解析失败)或子进程超时(90 秒,`tokio::time::timeout` 包裹)一律视为
`HeadlessError`,由调用方(见下)降级到启发式兜底,不重试。

`title`/`summary` 长度上限校验复用既有关闭时管线已有的规则(title ≤ 200、
summary ≤ 8000 字符,超出截断)。

### `crates/v8agent-cli` 跨仓库改动(独立仓库)

在 `/Users/chrischiang/Projects/CoralProjects/byteboy/v8agent` 里:

1. `main.rs` 引入基础参数解析(该二进制目前完全没有,零依赖 clap 之类的
   crate——引入哪种方式留实现阶段决定,可以是极简的手写 `env::args()`
   匹配,不必引入 clap 这种重依赖),新增一个一次性模式入口:读取全部
   stdin(而非 `repl.rs` 现有的逐行读)作为单个 prompt,跑一轮 engine
   调用,不进入 REPL 循环。
2. 新增一个不与工具调用 trace 交织的输出路径:一次性模式下,只在最终答案
   生成完毕后把完整文本一次性写到 stdout(不是 `render_wrapped`/`render()`
   现有的逐 `AgentEvent` 实时流式写法,那套是为交互式 REPL 设计的,会把
   `[tool] name(args)` 这类 trace 行和模型流式文本交织在一起)。
3. `README.md` 补充新模式的用法说明。

这部分改动的验收在 `v8agent` 仓库自己的测试里完成,不属于本 Dozer 仓库的
`cargo test` 范围;Dozer 侧只依赖它新增的命令行调用约定。**若这部分改动
未就绪先合并 Dozer 侧改动**:`headless_agent.rs` 对 `AgentKind::V8agent`
的调用会因为找不到预期的新 flag / 输出格式而失败,天然落入
`HeadlessError` → 启发式兜底路径,不会导致整体功能崩溃,只是 V8agent 的
session 补出来的总结质量退化成拼接文本(与其余三家的失败路径完全一致)。

### `dozer-core::protocol` 与 `dozer-client`

```rust
// Request
BackfillSessionSummaries { cwd: String },
GetSessionSummaryBackfillStatus { cwd: String },

// Reply
Ok, // 复用,BackfillSessionSummaries 立即返回
BackfillStatus { total: u32, completed: u32 },
```

`dozer-client::Client` 新增 `backfill_session_summaries(cwd)` /
`get_session_summary_backfill_status(cwd)`,风格对齐既有
`backfill_project_transcripts`/`list_conversations_with_summaries`。

### `dozerd` 后台任务

`Request::BackfillSessionSummaries` 处理器:

1. 立即返回 `Reply::Ok`。
2. `tokio::spawn` 后台任务:
   - 调用现有 `ListConversationsWithSummaries` 同款查询逻辑(内部直接复用
     `TranscriptStore::list_conversations` + `SessionSummaryStore::get_many`
     的组合,不经过协议层往返),筛出 `summary.is_none()` 的会话列表,得到
     `total`。
   - 把 `(cwd, total, completed=0)` 写入一个内存态
     `Mutex<HashMap<String, BackfillProgress>>`(按 `cwd` 索引,与
     `SessionRegistry` 类似的进程内状态,不持久化——`dozerd` 重启则进度
     丢失,这是可接受的:重启后用户重新点一次"修复项目"即可,和既有关闭时
     管线"轮询任务是内存态,重启即丢"的已知限制同一哲学)。
   - 依次(串行,不并发)对每条会话:读取其 `conversation_turns` 里的人类
     回合 → 调 `summarize_headless` → 成功写 `ai_generated`,失败/超时走
     `heuristic_from_turns` 写 `heuristic_fallback` → `completed += 1` 更新
     进度表。
3. `Request::GetSessionSummaryBackfillStatus` 处理器:直接查上述内存态
   `HashMap`,查不到该 `cwd` 的记录(还没开始/已完成太久被清理)时返回
   `total=completed=0` 之类的"无进行中任务"语义,由客户端据此判断展示
   完成态。

### `dozer-app`:修复项目弹窗改造

新增 step 状态数据模型(现有 `ScaffoldStepResult` 只有终态,补一层):

```rust
enum ScaffoldStepState {
    Pending,
    Running,
    Done(ScaffoldStepResult), // 复用现有 AlreadyOk | Created | Failed
}

enum BackfillStepState {
    Pending,
    Running { completed: u32, total: u32 },
    Done { completed: u32, total: u32 },
}
```

`WorkspaceState` 新增 `scaffold_running: Option<ScaffoldRunState>`
(取代/补充现有 `scaffold_report` 字段的展示用途),`ScaffoldRunState` 持有
5 个 `ScaffoldStepState` + 1 个 `BackfillStepState`。

渲染:`if ws_state.scaffold_running.is_some() { stack![base, dismiss?, scaffold_progress_popup(ws_state)] }`
——是否要 `dismiss` 遮罩层留空(按此前确认"进行中不可关闭",可以不挂
`dismiss` 元素,或挂一个 `.on_press` 指向空操作/不处理,实现阶段按现有
其他不可取消弹窗的写法照抄)。弹窗内容是 6 行列表(4 个 scaffold 步骤 + 1
个转录补录步骤 + 1 个补总结聚合进度行),每行左侧一个状态图标(pending 灰点
/running 转圈/done 对勾/failed 叉),右侧步骤名+简短结果文案,布局参照
`workspace.rs` 里 agent picker 弹窗的行样式(图标+文字+hover)。

`spawn_scaffold_run` 改造:原来"全部跑完一次性 `emit(ScaffoldDone)`"改成
每个步骤前后各 `emit` 一次(`ScaffoldStepStarted(idx)` /
`ScaffoldStepFinished(idx, result)`),补总结步骤额外 `tokio::spawn` 一个
1-2 秒轮询 `get_session_summary_backfill_status` 的循环,每次轮询结果变化
就 `emit(Message::BackfillProgress(completed, total))`,查到
`completed == total`(且 `total > 0`,或一开始查到 `total == 0` 直接视为
"无需补,立即完成")后停止轮询、标记该行 Done,并检查其余 5 项是否也已
全部 Done——全部 Done 时弹窗从"进行中"转为"可关闭"状态(允许点击 dismiss
或一个新出现的"完成"按钮关闭)。

`spawn_scaffold_run` 的 `visible` 参数语义不变:`visible=false`(静默打开
项目)时完全不触发本设计的补总结调用、不弹窗,只跑原有 4+1 步骤(维持现状
行为);`visible=true`(显式点击修复)才是本设计改造的路径。

## 错误处理

- **单条 session 的 headless 调用失败/超时**:降级到 `heuristic_from_turns`
  兜底,`completed` 计数照常 +1(视为"已处理",不是"待重试"),不中断整批
  处理其余 session。
- **配置的 `default_agent` 对应 CLI 二进制在 PATH 上找不到**(子进程启动
  失败,`Command::spawn` 返回 `Err`):整批补总结的每一条都会命中同样的
  spawn 失败,等价于"每条都走 headless 失败路径",全部降级为启发式兜底,
  `completed` 依然正常推进到 `total`,弹窗最终仍会显示"完成",只是内容
  全是拼接文本——不特殊处理"提前整体判定失败并中止"这种分支,保持"每条
  都会被处理"这个不变量,和现有关闭时管线"没等到就兜底"的哲学一致。
- **v8agent-cli 跨仓库改动未就绪**:见"架构与数据流"对应小节,天然落入
  `HeadlessError` 降级路径,不特殊处理。
- **`dozerd` 重启导致后台补总结任务丢失**:内存态进度表清空,`cwd` 对应的
  `GetSessionSummaryBackfillStatus` 查询会返回"无进行中任务";若此时
  `dozer-app` 的弹窗还在轮询,会一直显示上一次查到的进度不再变化——这是
  已知边界情况(`dozerd` 重启期间用户恰好开着修复弹窗),不额外处理,用户
  关闭 App 重开或再次点修复即可恢复;与既有关闭时管线"重启即丢失轮询任务"
  的已知限制同一等级。
- **补总结请求的 `cwd` 与转录数据的 `dir`/`cwd` 字段口径不一致**(路径
  大小写、符号链接、末尾斜杠等差异导致查询筛不出应有的会话):复用
  `ListConversationsWithSummaries`/`backfill_project_transcripts` 现有的
  路径处理逻辑,不新增路径归一化规则——如果现有转录补录能对上,本设计的
  查询用同一套路径处理就应该也能对上,这不是本设计需要重新解决的问题。

## 测试策略

1. `dozerd`:`headless_agent.rs` 单测——针对四种固定 stdout 样本(含正常
   分隔符包裹的 JSON、缺分隔符、JSON 格式错误、空输出)断言解析结果与降级
   判定;超时场景用可配置的短超时值做集成测试(不真的等 90 秒)。
2. `dozerd`:`default_agent_config.rs` 单测——配置文件存在/不存在/字段
   非法三种场景断言回退行为。
3. `dozerd`:`BackfillSessionSummaries` 处理器集成测试——构造若干条缺总结
   的会话 fixture,断言后台任务把它们全部处理完(可 mock
   `summarize_headless` 让测试不真的拉起子进程)、`GetSessionSummaryBackfillStatus`
   返回的 `(completed, total)` 单调递增到位。
4. `dozer-app`:step 状态机单测——`ScaffoldStepState`/`BackfillStepState`
   在收到对应 `Message` 后的转换断言;弹窗"全部完成才可关闭"的门控逻辑
   断言。
5. `v8agent` 仓库:新增一次性模式的独立单测/集成测试(在该仓库自己的测试
   套件里,不属于本次 Dozer 仓库测试范围)。
6. `cargo build`(全 workspace)、`cargo test -p dozerd -p dozer-app -p
   dozer-core -p dozer-client`、`cargo clippy --all-targets -- -D
   warnings`、`cargo fmt -- --check`。
7. **真机验证(单测覆盖不到端到端体验)**:
   - 已装好 Claude CLI:构造一个有历史会话但缺总结的项目,点"修复项目",
     确认弹窗依次跑完 6 项,补总结行数字正确增长到位,`session_summaries`
     表对应行 `status=ai_generated`。
   - 故意把 `default_agent` 配置指向一个本机没装的二进制:确认补总结行
     依然走到"完成",但对应行是 `heuristic_fallback`。
   - CodeBuddy / OpenCode 各重复一次上述正常路径验证,确认各自的一次性
     调用参数(尤其 CodeBuddy 的 `-y`)确实生效。
   - v8agent(需要 `v8agent-cli` 侧改动已合并):同样验证一次;若改动未
     合并,验证"降级到 heuristic_fallback 而不是整体卡死/报错"这条路径。
   - 静默打开项目(不点修复按钮):确认不弹窗、不触发补总结请求,行为与
     改造前一致。

## 排期备注

建议实现顺序:

1. `v8agent-cli` 跨仓库改动(独立仓库,可与 Dozer 侧并行开工,互不阻塞——
   Dozer 侧在它没就绪时天然走降级路径,不依赖它先合并)。
2. `dozerd`:`default_agent_config.rs` + `headless_agent.rs`(Claude/
   CodeBuddy/OpenCode 三家先做,可用真实 CLI 手动验证,不依赖协议层/UI)。
3. `dozer-core::protocol` 扩展 + `dozer-client` 方法 + `dozerd` 的
   `BackfillSessionSummaries`/`GetSessionSummaryBackfillStatus` 处理器
   (纯后端,可用集成测试独立验证)。
4. `dozer-app`:step 状态数据模型 + 弹窗 UI + `spawn_scaffold_run` 改造
   (依赖第 3 步的协议先落地)。
5. `v8agent` 分支在 dozerd 的 `headless_agent.rs` 里补上(依赖第 1 步,若
   第 1 步还没合并可以先把这个分支写成"直接返回 unsupported 走兜底"的
   占位实现,等 v8agent 那边就绪后再补真实调用逻辑)。

真机验证放在最后,需要本机装好各家 CLI 二进制,不是纯 headless 测试环境
能覆盖的部分。
