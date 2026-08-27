# 对话面板导航改版:session 列表 → session 详情

**状态:已批准(brainstorming 会话,2026-08-27)**

## 背景

对话面板(`crates/dozer-app/src/conversation.rs`,rail 图标"对话",`PanelKind::
Conversations`)6 天前(commit `36e7fbc`,2026-08-21)刚从"按 session 分树展开"改成
"当前项目全部 session 的回合拍平成一份按时间倒序列表",理由是"树状展开/懒加载 UX 不
直观"。本设计不是简单 revert 那次改动——要把导航模式改回"先看 session 列表",但列表
项要接入姊妹项目 A(`docs/superpowers/specs/2026-08-27-session-summary-pipeline-design.md`)
产出的 `session_summaries` 数据,不是回到 6 天前那套无摘要的树。

A 负责产出并持久化会话总结(`session_summaries` 表,`dozer-client::get_session_summary`),
B(本设计)负责把这份数据在对话面板里展示出来。这次 brainstorming 已用户确认的关键
决定:①彻底替代现有扁平回合列表,不做视图共存;②没有总结的历史 session(旧数据/纯
shell/SSH/Codex/Kilo,A 明确排除的类型)降级显示现有 `ConversationSummary.title`,不加
额外的"未总结"标记;③右侧列表面板本来就常驻可见(主从布局),`review_content_pane`
现有的"上一个/下一个话题"(`prev_topic`/`next_topic`/`TopicPreview`)整体删除,改成
直接点列表切换。

**已确认的关键架构约束(2026-08-27 调研)**:

- `session_summaries`(键 `session_id`,A 项目的表)和 `conversations`(键
  `conversation_id`,`TranscriptStore` 的表)是**两个独立 sqlite 连接**——`session_id`
  是 `dozerd::registry` 自己生成的 PTY 会话 id,`conversation_id` 是 transcript 文件名
  的 `file_stem()`(比如 Claude 自己分配的 session id),两者是不相关的 UUID 空间,唯一
  的桥是 `Session::info().transcript_path`。这意味着 SQL 层做不到跨库 JOIN,必须先在
  A 那边给 `session_summaries` 补一个 `conversation_id` 列(A 的 plan 已经就此打了补丁,
  但**截至本设计定稿时该补丁尚未被实现**——写 B 的实现计划前必须先确认 A 那边的
  `conversation_id` 列/`get_many` 方法已经落地,否则 B 的 Task 1 会卡住)。
- 跨会话联查的既有参考模式是 `TranscriptStore::get_usage_summary_in`
  (`crates/dozerd/src/transcripts/mod.rs:618-708`):先
  `list_conversations_in(...)` 拿到一批 `ConversationSummary`,收集 `conversation_id`
  列表拼 `IN (...)` 占位符,**只 `prepare` 一次**(函数注释明确写了这是为了避免"N 条
  会话 prepare N 次"的老问题),把结果按 `conversation_id` 拼回去。B 的联查方法照抄
  这个模式,不重新发明。
- `Request::ListSessionTurnGroups`/`Reply::SessionTurnGroups`/`TurnGroupSummary`
  (8/21 拍平前留下的"session 内子分组"查询)全仓库排查确认**目前零产品代码消费者**
  (只有 `server.rs` 转发本身、`dozer-client` 的 SDK 封装、一条只测"未知 id 报错"的集成
  测试)。B 的 session 详情不做子分组(整段摊平展示),这套连同 `Request::
  ListAllTurnGroups`/`Reply::AllTurnGroups`/`TurnGroupEntry`(6 天前那次改动引入的扁平
  聚合查询,替代后也失去唯一消费者)一并作为死代码清理范围。

## 目标 / 非目标

**目标**:

1. 新增服务端联查:`dozerd` 里 `SessionSummaryStore` 补 `get_many(conversation_ids:
   &[String]) -> HashMap<String, SessionSummaryPayload>` 批量查询(建立在 A 的
   `conversation_id` 列补丁之上,附带给该列建索引)。`TranscriptStore` 或
   `server.rs` 层新增一步"先查 `list_conversations_in`,再用得到的
   `conversation_id` 批量查 `session_summaries`,Rust 侧按 `HashMap` 拼成
   `Vec<(ConversationSummary, Option<SessionSummaryPayload>)>`"。
2. `dozer-core::protocol` 新增 `Request::ListConversationsWithSummaries { cwd,
   agent, limit, offset }` / `Reply::ConversationsWithSummaries { rows }`,
   `dozer-client` 加 `list_conversations_with_summaries` 方法。
3. `dozer-app` 侧:`conversation.rs` 新增"会话行"数据类型(取代 `TurnGroupRow`),
   同时承载 `ConversationSummary` 与可选的 `SessionSummaryPayload`;
   `workspace.rs::conversation_list_pane` 改渲染会话行(标题+摘要预览,无总结时
   降级显示 `ConversationSummary.title`);点击触发新 Message 加载"整个 session"
   而非"一个回合区间"。
4. `review_content_pane` 顶部新增总结展示区(标题+摘要全文,无总结则只显示标题),
   下方是该 session 全部回合的对话式列表,首屏 + "加载更多"翻页(复用
   `get_conversation_turns` 现成的 keyset 分页,不新增分页机制)。
5. 删除 `prev_topic`/`next_topic`/`TopicPreview` 相关代码及其触发逻辑。
6. 死代码清理:`Request::ListAllTurnGroups`/`Reply::AllTurnGroups`/
   `TurnGroupEntry`/`conversation.rs::TurnGroupRow`及`from_entry`,以及
   `Request::ListSessionTurnGroups`/`Reply::SessionTurnGroups`/`TurnGroupSummary`/
   `TranscriptStore::list_turn_groups`/`Client::list_session_turn_groups` 全部
   删除(含协议 variant、server.rs 分支、client 方法、仅剩的一条集成测试)。
7. 全部改动落地后:`cargo build`(全 workspace)、`cargo test -p dozerd -p
   dozer-app -p dozer-core -p dozer-client`、`cargo clippy --all-targets -- -D
   warnings`、`cargo fmt -- --check` 全绿;真机验证走人工(见"测试策略")。

**非目标**:

- **不做 session 内子分组/话题导航**——详情视图整段摊平展示该 session 全部回合,
  不保留"一个 session 里按回合分组展开"的树状/子列表结构(这正是 8/21 那次改动想
  规避的 UX)。
- **不改变 `review_content_pane` 消费 `TurnRecord`→`ReviewEntry` 的转换逻辑**——
  单条回合怎么渲染成 `Human`/`AiTurn` 不在本设计范围,只改"一次加载多少回合、从哪个
  维度分页"。
- **不实现 A 里被搁置的"headless 补总结"/"项目修复兜底扫描"**——那是 A 的姊妹子项目
  ③,依赖被用户显式搁置的 headless 机制,B 只消费 A 已产出的数据,不负责补全缺失总结。
- **不改 `conversation_search`/`conversation_agent_filter` 的过滤语义**——现有按
  文本/agent 过滤的逻辑保留,只是过滤对象从"回合分组"换成"会话行",过滤字段(标题/
  agent)不变。

## 架构与数据流

### `dozerd` 侧:批量总结查询 + 联查

`crates/dozerd/src/session_summary.rs` 新增(依赖 A 的 `conversation_id` 列已存在):

```rust
pub fn get_many(&self, conversation_ids: &[String]) -> Result<HashMap<String, SessionSummaryPayload>> {
    if conversation_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = conversation_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms
         FROM session_summaries WHERE conversation_id IN ({placeholders})"
    );
    // ... prepare 一次,query_map 按 conversation_id 建 HashMap ...
}
```

`conversation_id` 列补建索引:`CREATE INDEX IF NOT EXISTS idx_session_summaries_conversation_id
ON session_summaries(conversation_id)`(仿 `conversation_turns` 已有的
`idx_turns_session_order` 建索引写法)。

`TranscriptStore`(或 `server.rs` 内的组合函数,视实现计划阶段判断哪边更合适——
`TranscriptStore` 不该反过来依赖 `SessionSummaryStore`,联查逻辑更适合放在
`server.rs` 的 handler 里,两个 store 都是 `handle_conn` 已有的参数,不需要新的跨
store 依赖)新增组合逻辑:

```rust
Request::ListConversationsWithSummaries { cwd, agent, limit, offset } => {
    match transcripts.list_conversations(&cwd, agent, limit, offset) {
        Ok(conversations) => {
            let ids: Vec<String> = conversations.iter().map(|c| c.conversation_id.clone()).collect();
            let summaries = session_summaries.get_many(&ids).unwrap_or_default();
            let rows = conversations
                .into_iter()
                .map(|c| { let s = summaries.get(&c.conversation_id).cloned(); (c, s) })
                .collect();
            Reply::ConversationsWithSummaries { rows }
        }
        Err(e) => Reply::Error { message: format!("列对话失败: {e}") },
    }
}
```

### 协议扩展

```rust
// Request
ListConversationsWithSummaries { cwd: String, agent: Option<AgentKind>, limit: u32, offset: u32 },

// Reply
ConversationsWithSummaries { rows: Vec<(ConversationSummary, Option<SessionSummaryPayload>)> },
```

`dozer-client::Client` 新增 `list_conversations_with_summaries` 方法,风格对齐现有
`list_conversations`。

### `dozer-app` 侧:会话行数据类型与渲染

`conversation.rs` 用一个新结构取代 `TurnGroupRow`:

```rust
pub struct SessionRow {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub last_ts: u64,
    /// 列表行标题:有总结用总结的 title,没有降级用 ConversationSummary.title。
    pub display_title: String,
    /// 总结全文(不截断)。列表行渲染时在渲染层截断成预览,详情页直接整段展示
    /// ——存全文是为了详情页不用为总结内容单独发一次请求(列表已经查过了)。
    /// 没有总结时为 None,渲染层回退到现有"agent·相对时间"文案。
    pub summary: Option<String>,
    pub summary_status: Option<SummaryStatus>,
}
```

`Workspace` 状态字段 `conversation_turn_groups: Option<Vec<TurnGroupRow>>` 改名/
改类型为 `conversation_sessions: Option<Vec<SessionRow>>`,加载入口从
`list_all_turn_groups(cwd, 500)` 换成 `list_conversations_with_summaries(cwd, None,
500, 0)`;`conversation_pages`/`CONVERSATION_PAGE_SIZE`/`conversation_visible_count`
分页机制不变,只是分页对象换了类型。`filter_turn_groups` 改造为对 `SessionRow` 按
`display_title`/`agent` 过滤,函数体逻辑不变,只换字段来源。

`conversation_list_pane` 每行渲染:标题行用 `display_title`,副行优先显示
`summary`(有的话,渲染时按现有 `last_activity` 的 60 字符惯例截断),没有则维持
现状"agent·相对时间"文案。点击触发新的
`Message::ConversationSessionOpen(conversation_id: String, agent: AgentKind)`
(取代 `Message::ConversationTurnGroupOpen(path, agent, start, end)`)。

### session 详情加载与展示

`app.rs::conversation_session_open`(取代 `conversation_turn_group_open`)不再需要
算 `prev_topic`/`next_topic`(整块删除),直接:

```rust
fn conversation_session_open(app: &mut App, io: &ShellIo, conversation_id: String, agent: AgentKind) {
    // 从已加载的 ws.conversation_sessions 里找到这一行,取 display_title/摘要全文
    // (摘要全文需要 SessionRow 也带上完整 summary,不只是预览——见下方"数据完整性")
    // 塞一个空 ReviewView(source: ReviewSource::Session(conversation_id.clone()))
    // spawn_review_load(io, ReviewSource::Session(conversation_id), after=-1, limit=首屏页大小)
}
```

`ReviewSource` 新增一个 variant `Session(String)`(取代此前 `FileRange(PathBuf, i64,
i64)` 在对话面板这条路径下的用法——`FileRange` 原本是"文件路径+精确回合区间"的语义,
不适合表达"整个 session,从头分页加载";`FileRange` 是否还被终端 tab 审阅那条路径用到
需要在实现计划阶段核实,若还在用则保留,只是对话面板这条路径改用新 variant)。
`spawn_review_load` 对 `Session` 分支调 `client.get_conversation_turns(&conversation_id,
after_turn_index, limit)`,`ReviewLoaded` 消息把结果追加进 `entries`(而不是替换,支持
"加载更多"逐页累加)。

`review_content_pane` 顶部新增总结展示区,渲染 `ws.review.as_ref().and_then(|r|
r.summary_title.as_deref())` / `r.summary_text.as_deref()`(`ReviewView` 加两个
`Option<String>` 字段)。`conversation_session_open` 打开详情时,直接从已加载的
`ws.conversation_sessions` 里找到被点击那一行的 `SessionRow.display_title`/
`SessionRow.summary`(全文,上一节已定),原样填进 `ReviewView` 的这两个新字段——
不为详情页单独发一次 `get_session_summary` 请求,复用列表已经查到的数据。

### 死代码清理

删除:
- `dozer-core::protocol`:`Request::ListAllTurnGroups`/`Reply::AllTurnGroups`/
  `TurnGroupEntry`/`Request::ListSessionTurnGroups`/`Reply::SessionTurnGroups`/
  `TurnGroupSummary`,以及这几个类型仅有的序列化往返测试。
- `dozerd`:`TranscriptStore::list_all_turn_groups`/`list_all_turn_groups_in`/
  `list_turn_groups`,及其单测。
- `dozer-client`:`list_all_turn_groups`/`list_session_turn_groups` 方法,及
  `tests/against_real_daemon.rs` 里那条只测"未知 id 报错"的集成测试。
- `dozer-app`:`conversation.rs::TurnGroupRow`/`from_entry`;
  `workspace.rs`/`app.rs` 里所有 `TurnGroupRow`/`ConversationTurnGroupOpen`/
  `prev_topic`/`next_topic`/`TopicPreview` 相关代码。

## 错误处理

- **`list_conversations_with_summaries` 时某个 `conversation_id` 在
  `session_summaries` 里查不到**:`get_many` 返回的 `HashMap` 该 key 缺失,`rows`
  里对应元组第二项是 `None`,渲染层走"降级显示 `ConversationSummary.title`"分支
  (目标 3 已定,不是错误,是正常状态)。
- **`session_summaries.get_many` 查询失败**(sqlite 错误):`unwrap_or_default()`
  退化成空 `HashMap`,等价于"这批全部没有总结",不阻断会话列表本身的展示——列表的
  核心数据源是 `conversations` 表,总结是锦上添花,总结查询失败不该让整个列表挂掉。
- **session 详情"加载更多"时 `get_conversation_turns` 失败**:维持现有
  `ReviewLoaded` 消息里已有的错误处理路径(`ReviewView.error` 字段),不新增机制。
- **点击一个 `conversation_id` 在 `ws.conversation_sessions` 里已经不存在了**
  (极小概率:列表刷新与点击之间的竞态):按现有 `conversation_turn_group_open` 对
  "找不到对应行"的既有降级处理方式(读实现计划阶段确认现状怎么处理,沿用同一套,不
  新增分支)。

## 测试策略

1. `dozerd`:`SessionSummaryStore::get_many` 单测——空输入返回空 map、多个 id 批量
   命中、部分命中部分缺失、`IN` 子句 SQL 注入安全性(用含特殊字符的 id 探测,确认走
   参数绑定而不是字符串拼接)。
2. `dozerd`:`ListConversationsWithSummaries` handler 集成测试——构造若干
   conversations,只给部分配 session_summaries,断言 `rows` 里对应位置正确是
   `Some`/`None`。
3. `dozer-client`:`list_conversations_with_summaries` 对真 daemon 的协议往返测试。
4. `dozer-app`:`filter_turn_groups`(改造后按 `SessionRow` 过滤)单测——标题/agent
   过滤仍然正确;`SessionRow` 降级显示逻辑单测(有总结用总结 title,没有用
   `ConversationSummary.title`)。
5. `cargo build`(全 workspace)、`cargo test -p dozerd -p dozer-app -p dozer-core
   -p dozer-client`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --
   check`。
6. **真机验证(单测覆盖不到端到端体验)**:
   - 有总结的 session:列表行显示总结标题+预览,点开详情看到标题+摘要全文+完整
     回合列表。
   - 没有总结的历史 session(旧数据/纯 shell):列表行降级显示原标题,点开详情
     只有回合列表、没有总结展示区。
   - 长会话(回合数超过首屏页大小):详情页"加载更多"正确追加而不是替换已加载内容。
   - 搜索/agent 过滤:输入关键字/切换 agent 过滤器,列表正确收窄。
   - 镜像态(`app.panel_mirrored(PanelKind::Conversations)`):左右互换后列表点击→
     详情联动依然正确。

## 排期备注

**硬依赖**:实现计划开工前必须先确认 A 的 `session_summaries.conversation_id` 列
补丁(见 A 的 plan 文件"⚠️ 修正"章节)已经落地并合并——B 的 Task 1(`get_many`)直接
建立在这一列之上,若 A 还没打这个补丁,B 无从下手,应先去推进/确认 A 那边完成。

建议实现顺序:先落 `dozerd` 侧 `get_many` + 联查 handler + 协议扩展 +
`dozer-client` 方法(纯后端,可独立验证);再落 `dozer-app` 的 `conversation.rs`
数据类型改造 + `conversation_list_pane` 渲染;再落 session 详情加载(`ReviewSource::
Session`+"加载更多"+ 总结展示区);死代码清理放最后一步,确认新路径全部工作正常
后再删旧协议/旧代码,避免中间状态编译不过。
