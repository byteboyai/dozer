# Agent 面板卡片化 + LLM/Mode/工作区展示 设计

**状态:已批准(brainstorming 会话,2026-08-16)**

## 背景

Agent 域面板(`crates/dozer-app/src/workspace.rs` 的 `agent_list_pane`/
`agent_list_row`,workspace.rs:2149-2236)目前是紧凑列表行:状态点 + 状态
文字 + 会话标题,按 `AgentKind` 分组(`claude(1)` 这类标题)。用户给了一张
参考草图,要求把每行换成卡片,卡片内新增三项信息:

- **LLM**:如 `Sonnet 5`
- **Mode**:如 `auto`
- **工作区**:如 `main`(分支名,附带脏标)

brainstorming 过程中查证到两处关键事实:

1. Claude Code transcript(JSONL)的 `user`/`assistant`/`permission-mode`
   三种行的顶层都带 `permissionMode` 字段,assistant 行的 `message.model`
   带模型 id——用真实 transcript 样本抓取验证过,值分别是 `"auto"` 和
   `"claude-sonnet-5"`,与草图吻合。现有 `transcript.rs` 只解析
   `text`/`tool_use`/`thinking`,这两个字段目前被当噪音丢弃。
2. `agent_state`(运行中/待输入等)的现有更新链路是纯推送:hook 事件 →
   dozerd 广播 → dozer-client 转发 → `App::agent_state_changed`
   (app.rs:4836)。这个函数在**每次** hook 事件(不只是回合结束
   `TurnEnded`)都会被调用,是"实时"更新 model/mode 的天然挂载点,不需要
   新建轮询/文件监听子系统。

## 目标 / 非目标

**目标**:

1. `agent_list_row` 换成卡片渲染(`agent_card`):agent 名 → LLM 行(仅
   `AgentKind::Claude` 且已解析到值时显示,否则整行不渲染)→ Mode 行(同上
   条件)→ 工作区行(分支名 + 脏标,所有 agent 都显示,非 git 项目显示
   `—`)→ 状态点 + 状态文字。分组标题(`claude(1)`)保留。
2. `SessionTab` 新增 `llm_model: Option<String>`、`permission_mode:
   Option<String>` 两个字段(**不叫 `model`**——`SessionTab` 已有一个
   `model: TerminalModel` 字段是终端显示缓冲区,同名会直接编译报错,写计划
   时必须用 `llm_model` 这个名字),来源是 transcript 尾部轻量提取(新函数
   `transcript::latest_model_and_mode`),只在 `agent == AgentKind::Claude`
   时才会被填充,其余 agent 恒 `None`(卡片对应行不渲染)。
3. 工作区分支名 + 脏标:默认复用 `ws.project_panel` 已有的项目级 git 缓存
   (因为目前一个项目下所有会话默认共享项目根目录这个 cwd,见"背景"调研);
   仅当某个 session 的 `effective_cwd()` 偏离项目根目录(用户在该会话的
   shell 里 `cd` 过)时,才为这个 session 单独查一次,存进 `SessionTab`
   新增的 `workspace_override: Option<WorkspaceGitInfo>` 字段。
4. 上述三项(llm_model/mode/workspace_override)的刷新时机统一挂在
   `App::agent_state_changed` 里,每次 hook 事件都触发一次(不止
   `TurnEnded`),复用现有 hook 推送链路,不新建轮询或文件 tail 机制。
5. model id 的展示美化:`claude-sonnet-5` → `Sonnet 5`(去 `claude-` 前缀,
   按 `-` 分词、首字母大写),不认识的形状原样显示。

**非目标**:

- 不做 Codex/Qoder/Opencode/Kilo/V8agent 的 LLM/Mode 支持——这几家的
  transcript 解析目前本就是空的(`transcript.rs:187` 附近),没有对得上的
  "模型"/"模式"概念可抓,卡片上这两行直接不渲染,不显示"—"占位(与"没有
  解析到值"视觉上等价,不需要额外区分"agent 不支持"和"还没解析到")。
- 不做增量 tail 读(字节偏移追踪、半行缓冲)。model/mode 每次都整份重读
  transcript 文件——与现有"回合结束时整份重读"给审阅面板用同一种代价,
  只是触发更频繁(每个 hook 事件,而不只是回合结束)。如果长会话下这个
  代价被证明扛不住,留作后续优化,这轮不做。
- 不改 `dozer-core::protocol::SessionInfo` / `dozerd` / `dozer-client`——
  model/mode/workspace_override 是 dozer-app 本地从 transcript/git 现查
  的展示态,不需要经 daemon 持久化或跨进程同步,daemon 重启/晚 attach 场景
  下这几个字段就是空,等下一次 hook 事件自然补上,不视为异常。
- 不做 model id 美化的硬编码型号对照表(容易随新模型发布过期);超出
  `claude-<词>-<数字...>` 简单形状的 id(比如带日期后缀的旧式 id)美化
  结果可能不好看(比如 `Opus 4 1 20250805`),这轮接受,不特殊处理。
- 不做"卡片/紧凑列表"双视图切换——完全替换,不保留旧紧凑行作为可选模式。
- 不新增独立的工作区脏标轮询;`workspace_override` 只在 hook 事件触发时
  重算,不脱离这条链路单独定时刷新。

## 架构与数据流

### 1. `transcript.rs`:新增轻量提取函数

```rust
/// 从 transcript 尾部提取最后一次出现的 model id / permissionMode(后出现
/// 的覆盖先出现的,只关心最新值,不像 `parse_transcript` 那样建完整的
/// `ReviewEntry` 列表)。`permissionMode` 在 `user`/`assistant`/
/// `permission-mode` 三种行的顶层都会出现,统一按顶层键取,不区分行类型。
/// 解析失败的行跳过,不中断整体扫描(同 `parse_claude_shaped_jsonl` 的
/// 既有容错口径)。
pub fn latest_model_and_mode(jsonl: &str) -> (Option<String>, Option<String>) {
    let mut model = None;
    let mut mode = None;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(m) = v
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|s| s.as_str())
        {
            model = Some(m.to_string());
        }
        if let Some(pm) = v.get("permissionMode").and_then(|s| s.as_str()) {
            mode = Some(pm.to_string());
        }
    }
    (model, mode)
}
```

只对 `agent == AgentKind::Claude` 的会话调用(其余 agent 的 transcript
schema 未知/未解析,调用方在 `spawn_agent_card_refresh` 里按 agent 类型
短路,不会对非 Claude 会话跑这个函数)。

### 2. `workspace.rs`:model 展示美化

```rust
/// `claude-sonnet-5` → `Sonnet 5`:去掉 `claude-` 前缀,按 `-` 分词、每词
/// 首字母大写、空格拼接。不以 `claude-` 开头的原样返回(不确定形状,不
/// 强行摘,避免拍出乱码;见 spec 非目标——不维护型号对照表)。
pub(crate) fn format_model_label(raw: &str) -> String {
    match raw.strip_prefix("claude-") {
        Some(rest) => rest
            .split('-')
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => raw.to_string(),
    }
}
```

`permission_mode` 值(`auto`/`plan`/`acceptEdits`/`bypassPermissions`)已经
是短小写词,直接原样展示,不做转换。

### 3. `SessionTab` 新增字段(workspace.rs:208-241)

```rust
pub struct SessionTab {
    // ...既有字段(含已存在的 `model: TerminalModel`——终端显示缓冲区,
    // 与下面这个新字段是两回事,故新字段不能叫 `model`)...
    /// 最近一次 hook 事件后从 transcript 尾部提取的模型 id(原始,未美化;
    /// 渲染时经 `format_model_label`)。仅 `agent == AgentKind::Claude` 会
    /// 被填充,其余 agent 恒 `None`。
    pub llm_model: Option<String>,
    /// 同上,来自 transcript 顶层 `permissionMode`。
    pub permission_mode: Option<String>,
    /// 工作区分支/脏标覆盖:仅当这个会话的 `effective_cwd()` 偏离项目根
    /// 目录时才会被填充;为 `None` 时渲染层直接读 `ws.project_panel` 的
    /// 项目级缓存(见目标 3)。
    pub workspace_override: Option<WorkspaceGitInfo>,
}

/// 单个会话的工作区展示态,来自 `delivery::branch`/`delivery::is_dirty`。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceGitInfo {
    pub branch: Option<String>,
    pub dirty: bool,
}
```

三个既有 `SessionTab { .. }` 构造点(workspace.rs:437、workspace.rs:1646、
测试辅助函数 `make_test_tab` workspace.rs:3530)各补一行三字段初始化为
`None`。

### 4. 刷新触发:`App::agent_state_changed`(app.rs:4836)

在既有 `tab.transcript_path` 更新之后(app.rs:4850-4852 一带)、不依赖
`state == AgentState::TurnEnded` 这个条件(该条件只保留给现有的交付检测/
审阅重解析两段),无条件调用新方法:

```rust
ws.spawn_agent_card_refresh(
    io,
    tab_id,
    agent,
    tab.transcript_path.clone(),
    tab.effective_cwd(),
    active_repo.clone(), // 已在函数开头取出的项目路径
);
```

`Workspace::spawn_agent_card_refresh`(仿 `spawn_review_load`,
workspace.rs:969-990 的写法):

```rust
pub(crate) fn spawn_agent_card_refresh(
    &self,
    io: &ShellIo,
    tab_id: usize,
    agent: AgentKind,
    transcript_path: Option<String>,
    cwd: PathBuf,
    project_root: Option<PathBuf>,
) {
    let Some(project_id) = self.project_id() else {
        return;
    };
    let needs_model_mode = agent == AgentKind::Claude;
    let needs_workspace = project_root.as_deref() != Some(cwd.as_path());
    if !needs_model_mode && !needs_workspace {
        return; // 常见情形(非 Claude + cwd 就是项目根)零额外 IO
    }
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let (llm_model, mode, workspace) = tokio::task::spawn_blocking(move || {
            let (llm_model, mode) = if needs_model_mode {
                transcript_path
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|s| transcript::latest_model_and_mode(&s))
                    .unwrap_or((None, None))
            } else {
                (None, None)
            };
            let workspace = if needs_workspace {
                delivery::repo_root(&cwd).map(|repo| WorkspaceGitInfo {
                    branch: delivery::branch(&repo),
                    dirty: delivery::is_dirty(&repo),
                })
            } else {
                None
            };
            (llm_model, mode, workspace)
        })
        .await
        .unwrap_or((None, None, None));
        let _ = proxy.send_event(Message::AgentCardRefreshed(
            project_id, tab_id, llm_model, mode, workspace,
        ));
    });
}
```

`needs_workspace` 判断用 `cwd` 与 `project_root` 直接比较路径——两者都已
是绝对路径(`effective_cwd()`/`project.path`),不做符号链接规整,和现有
`effective_project_repo` 的既有口径一致(不新增路径规整逻辑)。

### 5. `Message::AgentCardRefreshed`(app.rs 顶层 `Message`,仿
`GitRefreshed`/`DeliveryChecked` 的 5 元组写法)

```rust
/// hook 事件驱动的卡片元信息刷新结果;`model`/`mode` 为 `None` 表示这次
/// 没解析到新值(非 Claude,或 transcript 暂无相关行),**不**覆盖已有值
/// ——一旦拿到过 model,不会因为某次刷新没抓到就退回空白(见路由说明)。
/// `workspace` 同理,`None` 表示这次不需要覆盖(cwd 未偏离项目根)。
AgentCardRefreshed(
    ProjectId,
    usize, /* tab_id */
    Option<String>, /* llm_model */
    Option<String>, /* mode */
    Option<WorkspaceGitInfo>, /* workspace override */
),
```

路由(`App::update`,`with_project` 既有模式):

```rust
Message::AgentCardRefreshed(project_id, tab_id, llm_model, mode, workspace) => {
    self.with_project(project_id, |ws, _io| {
        if let Some(tab) = ws.tab_by_id_mut(tab_id) {
            if llm_model.is_some() {
                tab.llm_model = llm_model;
            }
            if mode.is_some() {
                tab.permission_mode = mode;
            }
            if workspace.is_some() {
                tab.workspace_override = workspace;
            }
        }
    });
}
```

`is_some()` 才覆盖的理由:每次刷新只重新扫描"当前"transcript 内容,如果
某次刷新恰好没有新的 `permissionMode`/`model` 行(理论上不会发生,因为
`latest_model_and_mode` 全量重扫,只要之前出现过就还在——这里的保护针对
`transcript_path` 读取失败等异常情形),不应该让卡片从"有值"闪回"无值"。

### 6. `extensions::project::WorkspaceState` 新增只读访问器

```rust
/// 项目级分支名(main 面板/Agent 卡片工作区行共用读口)。
pub(crate) fn branch(&self) -> Option<&str> {
    self.branch.as_deref()
}

/// 项目级脏标(同上)。
pub(crate) fn dirty(&self) -> bool {
    self.dirty
}
```

### 7. `view`:`agent_list_row` → `agent_card`

```rust
pub(crate) fn agent_card(
    ws: &Workspace,
    idx: usize,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;

    let mut lines = column![
        text(tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name))
            .size(theme::font::body())
            .color(theme::color::CREAM),
    ]
    .spacing(4);

    if let Some(llm_model) = &tab.llm_model {
        lines = lines.push(labeled_row("LLM", &format_model_label(llm_model)));
    }
    if let Some(mode) = &tab.permission_mode {
        lines = lines.push(labeled_row("Mode", mode));
    }

    let (branch, dirty) = match &tab.workspace_override {
        Some(w) => (w.branch.as_deref(), w.dirty),
        None => (ws.project_panel.branch(), ws.project_panel.dirty()),
    };
    lines = lines.push(workspace_row(branch, dirty));

    lines = lines.push(
        row![
            text("●")
                .size(theme::font::caption())
                .color(dot_color(tab.agent_state, tab.alive)),
            text(agent_state_label(tab.agent_state))
                .size(theme::font::caption_sm())
                .color(theme::color::DIM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    button(container(lines).padding(10))
        .on_press(Message::SelectTab(idx))
        .width(Length::Fill)
        .style(move |_t, _s| button::Style {
            background: if active {
                Some(theme::color::CARD.into())
            } else {
                None
            },
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: theme::color::CREAM,
            ..button::Style::default()
        })
        .into()
}
```

`labeled_row(label, value)`/`workspace_row(branch, dirty)` 是两个小的
私有渲染帮助函数(`label: value` 一行 caption 文字;`workspace_row` 无
`branch` 时显示 `—`,`dirty` 为真时分支名后缀一个脏标——具体符号/颜色
沿用 `project_branch_label`/`git_log.rs::worktree_strip` 已有的脏标视觉
约定,写计划阶段核对具体取用哪个,不重新发明一套)。`agent_list_pane`
(workspace.rs:2152-2195)里 `content.push(agent_list_row(ws, idx))` 改成
`content.push(agent_card(ws, idx))`,分组遍历逻辑不变。

## 错误处理

- transcript 文件读取失败(路径失效/权限问题):`spawn_agent_card_refresh`
  内 `.ok()` 吞掉,`llm_model`/`mode` 保持 `None`,不报错、不重试(等下一次
  hook 事件自然重试)。
- transcript 内容存在但没有任何 `permissionMode`/`message.model`(极早期
  session,第一个 hook 事件之前):`latest_model_and_mode` 返回
  `(None, None)`,卡片对应行不渲染,不视为异常。
- `delivery::repo_root(&cwd)` 返回 `None`(cwd 不在任何 git 仓库内,理论上
  不会发生,因为 `needs_workspace` 只在 cwd 偏离项目根时才为真,但用户可能
  `cd` 到仓库外):`workspace` 整体为 `None`,`AgentCardRefreshed` 路由时
  `workspace.is_some()` 为假,不覆盖——沿用 `ws.project_panel` 的项目级值
  展示,不显示"未知仓库"之类的额外状态。
- `format_model_label` 遇到不以 `claude-` 开头的 id:原样返回(非目标一节
  已说明,不额外处理)。
- 非 git 项目(`ws.project_panel.branch()` 恒 `None`):`workspace_row`
  显示 `—`,不算错误状态,与现有 `project_branch_label` 逻辑一致。

## 测试策略

- `transcript::latest_model_and_mode`:
  - 只有 `user`/`assistant` 混合行,能分别抓到最新 `model` id(仅 assistant
    行)和最新 `permissionMode`(user/assistant 行都可能带)。
  - 独立 `type:"permission-mode"` 行也能被抓到 `permissionMode`。
  - 同一字段出现多次,取最后一次(不是第一次)。
  - 空字符串/纯噪音行(如 `type:"mode"` 那种不相关字段)、损坏 JSON 行:
    跳过,不 panic,返回 `(None, None)`。
- `format_model_label`:`claude-sonnet-5` → `Sonnet 5`;`claude-opus-5` →
  `Opus 5`;不以 `claude-` 开头的原样返回;空字符串输入不 panic。
- `Workspace::spawn_agent_card_refresh` 的路由决策(不依赖真实 IO,测
  `needs_model_mode`/`needs_workspace` 两个布尔的组合是否正确决定要不要
  起异步任务——可以把判断逻辑拆成独立纯函数单测,异步 spawn 部分留给人工
  验收):非 Claude + cwd 等于项目根 → 不起任务;Claude + cwd 等于项目根
  → 只做 model/mode;非 Claude + cwd 偏离项目根 → 只做 workspace;Claude
  + cwd 偏离项目根 → 两者都做。
- `AgentCardRefreshed` 路由:`model`/`mode`/`workspace` 各自
  `Some`/`None` 时是否正确覆盖/保留原值(尤其验证"None 不覆盖已有值"这条
  行为)。
- `extensions::project::WorkspaceState::branch`/`dirty` 访问器:简单
  getter,补最小单测或跳过(视写计划阶段判断是否值得为纯 getter 开单测)。
- 人工验收:打开一个 Claude 会话,首轮回复后卡片出现 LLM/Mode 行;中途
  Shift+Tab 切换 permission mode,再来一轮后卡片 Mode 跟着变;在该会话
  的 shell 里 `cd` 到另一个 git 仓库,工作区行分支名跟着变、脏标独立于
  项目根仓库;打开一个 Codebuddy/Opencode 会话,确认卡片没有 LLM/Mode 行
  但工作区行正常;非 git 项目打开 agent,工作区行显示 `—`。

## 依赖变更

无新增依赖(`serde_json::Value` 已是既有依赖,`delivery`/`transcript` 模块
本身已存在)。
