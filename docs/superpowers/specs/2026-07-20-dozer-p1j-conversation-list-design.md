# Dozer P1j 设计：项目对话列表（右一 AI 栏，让 P1i 审阅引擎产生用途）

> 状态：设计稿,要点经用户逐问确认(见下),待写完复审。
> 上游:承接 P1i（transcript 适配器 + 审阅 tab 引擎，同分支 `feat/p1i-conversation-review`）;规格 §3 需求 6、§7 左四 AI 栏、§8 过程性资产存放未决点、H0"最近的对话"卡。
> 动因(用户 2026-07-20):"P1i 只看当前会话、无实质用途"——审阅引擎缺一个"翻阅项目所有对话"的入口。对话列表就是那个入口。

## 0. 用户确认的关键决定

**目标数据模型(愿景) = `agent ▸ session ▸ conversation` 三层**:agent(Claude Code/codex/…)之下有 session(dozerd PTY/终端 tab),session 之下有 conversation(一次 transcript)。本切片只落**其中一层的扁平视图**,三层分组随后续增量补。

1. **内容 = 扁平一层的项目对话列表(扫 Claude 目录)**,活着的对话置顶标"● 当前"。不做 `session ▸ 多 conversation` 两层树——那需要 Dozer 自建并持久化 session→transcripts 索引(§8"自建索引"),推二期。**理由**:磁盘上对话按项目(cwd)分组而非 session,dozerd 只知每个 session 的"当前"对话不知其历史,且 session 易失/对话持久;扁平一层数据现成零存储,绝大多数时候一个 session 就一个当前对话,够用。
   **多 agent 前瞻(用户补充 2026-07-20)**:扫描是 **Claude Code 专属**(扫 `~/.claude/projects`、认 JSONL)。多 agent 的历史对话需**每 agent 一套"对话来源"**(与规格 §4"每 agent 一个 transcript 适配器"平行)。本切片留两处门:①扫描逻辑圈成"ClaudeCode 对话来源",核心列表不依赖 Claude 细节;②`ConversationMeta` 带 `agent` 字段(现恒 `"claude"`)——UI/模型现在就有 agent 维度,二期按 agent 分组(agent ▸ 对话)是加法。
2. **位置 = 右一 AI 栏**,与 agent 列表可切换,对话列表在前(默认视图)。**理由(用户)**:"哪个 agent 在干活"是长期关注,"会话/对话"是每时每刻关注,后者更该显眼。
3. **取代 P1i 冗余入口**:移除终端 tab 的"审阅当前会话"按钮;当前会话经这个列表进入(它在列表里标"● 当前")。
4. **修订规格 §7 冻结项**:§7 左四原写"Agents 简卡(**不含会话列表**,避免与终端 tabs 重复)"。本设计有意在 AI 栏加对话列表并置于 agent 前——用户裁决:高频关注点优先,重复由"● 当前"标记与终端 tab 呼应而非割裂。

## 1. 目标

右一 AI 栏给出**当前项目的对话列表**(扫 Claude 目录,扁平),活对话置顶标"● 当前";点任一条 → 左二审阅 tab 结构化查看(复用 P1i)。让 P1i 审阅引擎从"看当前终端复读机"变成"翻阅项目所有对话史"——兑现"对话史是项目的过程性资产"。

## 2. 关键裁决

- **D1 会话目录映射=确定规则**:`claude_project_dir(cwd) = ~/.claude/projects/<cwd 中 '/' 换 '-'>`(实测本仓即此规则,非哈希)。放 `crates/dozer-app/src/conversation.rs`(新)。仅 Claude Code(一期)。
- **D2 列举=纯 IO + 纯解析分离,圈成"ClaudeCode 对话来源"**:`list_conversations(dir) -> Vec<ConversationMeta>`(读目录 + 每 jsonl 取首句人类发言当标题、mtime、size;GUI 侧 spawn_blocking);`conversation_title(jsonl_head: &str) -> Option<String>`(纯,只解析拿标题,遇首个字符串型 user content 即返回,省 IO)。`ConversationMeta{path, title, modified_ms, size_bytes, agent: String}`(`agent` 现恒 `"claude"`,为多 agent 留维度)。这些是 ClaudeCode 来源的实现;核心列表/渲染只吃 `ConversationMeta`,不碰 Claude 细节——二期加 agent 只是加一个产出 `ConversationMeta` 的来源。
- **D3 "两者合一"= 扫描列表 + 活标记**:活着的 claude 会话其 transcript 本就在 Claude 目录被实时写,所以扫描已含活对话;dozerd 额外给的只是"哪条现在活在 Dozer 终端 tab + 其 agent 状态",而 workspace 已有(每个 `SessionTab.transcript_path`/`agent_state`,P1i 已接)。渲染时把每个 `ConversationMeta.path` 与打开着的 `SessionTab.transcript_path` 交叉:匹配 → "● 当前" + agent 状态点 + 置顶;其余按 mtime 倒序。纯函数 `is_current_conversation(meta_path, &open_transcript_paths) -> bool`。
- **D4 右一 AI 栏视图切换器**:AI 栏(现 `pane("AI · P1e")` 占位)改真 pane:顶部一排小 tab `[对话 | Agents]`(默认"对话"),`ai_view: AiView{Conversations|Agents}` 状态;"对话"视图=对话列表,"Agents"视图=现占位(后续填)。切换 `Message::AiViewSwitch(AiView)`。
- **D5 刷新节奏复用 project**:`Workspace` 加 `conversations: Vec<ConversationMeta>`;`ProjectOpened` + `TurnEnded`(已有 `spawn_project_git_refresh` 处)顺带 `spawn_conversations_refresh`(spawn_blocking 扫目录)。
- **D6 点击进审阅=复用 P1i,泛化源**:新增 `Message::ConversationOpen(PathBuf)`。P1i 的 `ReviewView.source_tab_id: usize` 泛化为 `source: ReviewSource{ Session(usize) | File(PathBuf) }`——回合结束刷新只对 `Session` 源生效(File 源是历史快照不追加);`spawn_review_load` 已是按路径解析,只需按源取路径。`ConversationOpen` → `review = ReviewView{source: File(path)}` + `open_review` + 解析。
- **D7 取代终端入口 + dozerd 不参与**:移除 `terminal_pane` 的"审阅"按钮(D3/P1i 冗余);目录扫描/解析全 GUI 侧 spawn_blocking(哑管道),dozerd 仍只在会话活着时传当前 transcript_path(P1i 已有)。

## 3. 组件与数据流

```
ProjectOpened / TurnEnded
   │ spawn_blocking: conversation::list_conversations(claude_project_dir(项目cwd))
   ▼
Message::ConversationsRefreshed(Vec<ConversationMeta>) → workspace.conversations
   ▼
右一 AI 栏(ai_view=Conversations):每条 title + 时间/规模;
   与打开着的 SessionTab.transcript_path 交叉 → 活的标"● 当前"+agent 点+置顶
   │ 点击某条
   ▼ Message::ConversationOpen(path) → review=ReviewView{source:File(path)} + open_review + spawn_review_load
   ▼ 左二审阅 tab 渲染(复用 P1i review_content)
```

- `crates/dozer-app/src/conversation.rs`(新):`ConversationMeta`、`claude_project_dir(&Path)->PathBuf`、`conversation_title(&str)->Option<String>`(纯)、`list_conversations(&Path)->Vec<ConversationMeta>`(IO)、`is_current_conversation(&Path,&[String])->bool`(纯)。
- `crates/dozer-app/src/workspace.rs`:`conversations` 字段、`ai_view: AiView`、`AiView` 枚举、`Message::{ConversationsRefreshed, ConversationOpen, AiViewSwitch}`、`spawn_conversations_refresh`、`ReviewSource` 枚举(改 `ReviewView.source` + 相关分支)、`ai_pane` 渲染(替换占位)、移除终端"审阅"按钮。
- P1i 的 `review_content`/`parse_transcript`/审阅 tab 复用;`ReviewView`/`spawn_review_load`/`ReviewOpen(Session 源)`小改以容 `ReviewSource`。

## 4. 错误处理

- cwd 无对应 Claude 目录 / read_dir 失败 → 列表空,显"暂无对话记录"(灰字),不崩。
- 某 jsonl 无人类发言/读标题失败 → 标题回落文件名短 uuid 或"(无标题对话)",仍可点开。
- 点开的对话文件读失败 → 审阅 tab 域内红字(P1i `ReviewLoaded` Err 已有)。
- 无当前项目 → AI 栏对话视图显"先打开项目"。
- 当前会话无 transcript_path(hook 没装)→ 列表仍扫出历史对话,只是无"● 当前"标记。

## 5. 测试策略

Headless:
- `claude_project_dir`:`/a/b/c` → 后缀为 `-a-b-c` 的 projects 子目录(HOME 前缀对比)。
- `conversation_title`:首条 user 字符串 content → 标题;首条工具结果 list → 跳过续找;全无 → None。
- `list_conversations`:tempdir 造两 jsonl(不同 mtime/首句)→ 按 mtime 倒序、title/size 正确;空目录返空。
- `is_current_conversation`:路径命中打开集 → true,否则 false。
- `ReviewSource` 分派:`review_should_refresh_on_turn(&ReviewSource)->bool`——Session→true、File→false。

人工验收(草案):打开本仓 → 右一 AI 栏默认"对话"视图,列出历史对话(标题=首句、时间倒序),当前 claude 会话那条标"● 当前"置顶 → 点一条历史对话 → 左二审阅 tab 结构化呈现(人类锚点+折叠) → 切到"Agents"视图/切回 → 正常 → 回合结束:当前那条刷新、历史不动 → 终端不再有"审阅"按钮 → 非本项目/无记录 → "暂无对话记录"。

## 6. 备选方案(已否/推后)

- **三层 agent ▸ session ▸ conversations**:目标模型(§0),需自建索引(session 层,§8)+ 每 agent 对话来源(agent 层);推二期,本切片留 `agent` 字段与来源抽象两处门,分组增量补。
- **放左一(项目域)**:对话列表本可归左一(与文件树同域),但用户裁决放右一 AI 栏、与 agent 并列且在前(高频关注点优先)。
- **放左二(与预览 tab 并列)**:列表是导航不是内容,内容才进左二审阅 tab;分域不符。
- **保留 P1i"审阅当前会话"按钮**:与列表入口冗余,D7 收敛为单一入口。
