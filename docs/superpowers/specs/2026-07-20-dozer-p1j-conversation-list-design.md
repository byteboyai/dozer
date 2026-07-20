# Dozer P1j 设计：项目对话列表（让 P1i 审阅引擎产生用途）

> 状态：设计稿,范围/裁决因用户暂离由实施方按"最小可逆 + 已表意图"代拟,**逐条可推翻**。
> 上游:承接 P1i（transcript 适配器 + 审阅 tab 引擎，同分支 `feat/p1i-conversation-review`）;规格 §3 需求 6、§7 左二"会话审阅"、§8 过程性资产存放未决点、H0"最近的对话"卡。
> 动因(用户 2026-07-20):"P1i 只看当前会话、无实质用途"——审阅引擎缺的是一个"翻阅非当前对话"的入口。对话列表就是那个入口。

## 0. 范围裁决(代拟,可推翻)

**数据源=扫描 Claude 目录为主 + 预留列举接口**(用户暂离,取最小可逆)。实测本仓 `~/.claude/projects/-Users-...-dozer/` 已有 9 个 jsonl,目录名=cwd 把 `/` 换 `-`(确定规则,非哈希),首句人类发言天然成标题。扫描零存储零同步、映射确定;§8 的"自建索引"仅在多 agent/不依赖 Claude 格式时才值得,一期"仅 Claude Code 适配器"前提下不上。用一层 trait/函数抽象"列举项目对话",二期改索引只换实现。

**做**:左一项目栏"对话"区列出当前项目历史对话(标题=首句人类发言、副行=时间+规模)｜点某条 → 左二审阅 tab 渲染该对话(复用 P1i `parse_transcript`+审阅渲染)｜打开项目/回合结束时刷新列表｜**取代 P1i 冗余的"审阅当前会话"入口**——终端"审阅"按钮改为"在对话列表里高亮当前会话"或直接移除,当前会话经列表进入。
**不做(后续)**:跨 agent(仅 Claude JSONL)｜自建索引落库｜对话搜索/过滤｜删除/重命名对话｜H0 独立页的"最近对话"卡(跨项目,二期)｜对话与验收记录联动跳转(c/d)。

## 1. 目标

在左一项目栏给出**当前项目的历史对话列表**(扫 Claude 目录),点击任一条进左二审阅 tab 结构化查看。让 P1i 的审阅引擎从"看当前终端复读机"变成"翻阅项目所有对话史"——兑现规格"对话史是项目的过程性资产"。

## 2. 关键裁决(代拟,可推翻)

- **D1 会话目录映射=确定规则**:`claude_project_dir(cwd) = ~/.claude/projects/<cwd 中 '/' 换 '-'>`。放 `crates/dozer-app/src/conversation.rs`(新)。非 Claude 存储不管(一期)。
- **D2 列举=纯 IO 函数 + 纯解析函数分离**:`list_conversations(dir) -> Vec<ConversationMeta>`(读目录 + 每个 jsonl 取首句人类发言当标题、mtime、size;IO,GUI 侧 spawn_blocking);`conversation_title(jsonl_head) -> Option<String>`(纯函数,便于测——只解析拿标题,不全解析)。`ConversationMeta{path, title, modified_ms, size_bytes}`。为省 IO,标题只读文件**前若干行**直到遇到首个字符串型 user content。
- **D3 列表在左一,复用 project 刷新节奏**:`Workspace` 加 `conversations: Vec<ConversationMeta>`;`ProjectOpened` + `TurnEnded` 时(已有 `spawn_project_git_refresh`)顺带 `spawn_conversations_refresh`。渲染在 `project_pane` 文件树下方加"对话"折叠区。
- **D4 点击进审阅=复用 P1i,泛化入口**:P1i 的 `ReviewOpen(tab_id)` 靠 tab 的 transcript_path。新增 `Message::ConversationOpen(PathBuf)` 直接给路径 → `spawn_review_load` 需泛化为"按路径解析"(现已是 `spawn_review_load(tab_id, path)`,把 source_tab_id 语义放宽:列表来的用哨兵 id 或 Option)。`ReviewView.source_tab_id` 改 `source: ReviewSource{ Session(usize) | File(PathBuf) }`——回合结束刷新只对 `Session` 源生效(File 源是历史快照,不追加)。
- **D5 取代 P1i 冗余入口**:移除终端 tab 的"审阅当前会话"按钮;当前会话若在列表中(其 transcript_path 匹配某条),列表里高亮/标"● 当前"。用户经列表进入任何对话(含当前)。**这是对 P1i 的收敛,不是新增。**
- **D6 dozerd 不参与**:目录扫描/解析全 GUI 侧(延续哑管道);dozerd 仍只在会话活着时传当前 transcript_path(P1i 已有,用于 D5 高亮匹配)。

## 3. 组件与数据流

```
ProjectOpened / TurnEnded
   │ spawn_blocking: conversation::list_conversations(claude_project_dir(cwd))
   ▼
Message::ConversationsRefreshed(Vec<ConversationMeta>)
   ▼ workspace.conversations 存
project_pane 左一"对话"区:每条 title + 时间/规模,当前会话高亮
   │ 点击某条
   ▼ Message::ConversationOpen(path) → review = ReviewView{source: File(path)} + open_review + spawn_review_load(File 源)
   ▼ 左二审阅 tab 渲染(复用 P1i review_content)
```

- `crates/dozer-app/src/conversation.rs`(新):`ConversationMeta`、`claude_project_dir(cwd)->PathBuf`、`conversation_title(&str)->Option<String>`(纯)、`list_conversations(&Path)->Vec<ConversationMeta>`(IO)。
- `crates/dozer-app/src/workspace.rs`:`conversations` 字段、`ConversationsRefreshed`/`ConversationOpen` 消息、`spawn_conversations_refresh`、`ReviewSource` 枚举(改 `ReviewView.source`)、`spawn_review_load` 按源、左一"对话"区渲染、移除终端"审阅"按钮。
- P1i 的 `review_content`/`parse_transcript`/审阅 tab 全复用,不改。

## 4. 错误处理

- 项目非 git 也无妨(对话列表按 cwd 目录,与 git 无关)；cwd 无对应 Claude 目录 → 列表空,显"暂无对话记录"。
- 某 jsonl 读标题失败/无人类发言 → 标题回落文件名(短 uuid)或"(无标题对话)"，不跳过(仍可点开)。
- 目录 read_dir 失败(权限)→ 列表空 + 灰字提示,不崩。
- 点开的对话文件读失败 → 审阅 tab 域内红字(P1i 已有 ReviewLoaded Err 分支)。
- 当前会话无 transcript_path(hook 没装)→ 列表仍能扫出历史对话,只是不高亮"当前"。

## 5. 测试策略

Headless:
- `conversation_title`:喂 jsonl 头几行(首条 user 字符串 content)→ 返回标题;首条是工具结果 list → 跳过继续找;全无 → None。纯函数。
- `claude_project_dir`:`/a/b/c` → `<home>/.claude/projects/-a-b-c`。纯函数(HOME 注入或对比后缀)。
- `list_conversations`:tempdir 造两个 jsonl(不同 mtime/首句)→ 断言按 mtime 倒序、title/size 正确、空目录返空。
- workspace:`ReviewSource` 分派——File 源回合结束不刷新、Session 源刷新(纯逻辑函数 `review_should_refresh_on_turn(&ReviewSource, tab_id)->bool`)。
- 当前会话高亮:`is_current_conversation(meta_path, current_transcript_path)->bool` 纯函数。

人工验收(草案):打开本仓 → 左一"对话"区列出历史对话(标题=首句、时间倒序),当前会话标"● 当前" → 点一条历史对话 → 左二审阅 tab 结构化呈现那次对话(人类锚点+折叠) → 回合结束:当前会话那条刷新、历史对话不动 → 终端不再有"审阅"按钮(改由列表进入) → 非本项目 cwd/无记录 → "暂无对话记录"。

## 6. 备选方案(已否/已并入)

- **纯自建索引**:见 D0,一期成本不值;抽象层保留改造门。
- **保留 P1i 的"审阅当前会话"按钮 + 新增列表**:两个入口冗余,用户已点出当前会话入口无价值;D5 收敛为单一入口(列表)。
- **对话列表放左二(与预览 tab 并列)**:对话列表是"项目资产的导航",属左一项目域(与文件树同域);选中某对话的"内容"才进左二审阅 tab。放左一符合规格四栏分域。
