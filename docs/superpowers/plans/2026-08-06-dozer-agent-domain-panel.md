# Agent 域面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把右栏"Agent"视图从占位文案接成真实功能：按 `AgentKind` 分组展示当前项目会话、对话历史列表加 agent 着色圆点、面板顶部新增"＋新建"入口(选 agent 后自动帮用户在新 shell 里键入对应 CLI 名字)。

**Architecture:** 全部改动落在 `crates/dozer-app/src/workspace.rs`(+ `main.rs` 一处键盘路由),只消费已落地的 `ws.tabs`/`AgentKind`/`AgentState` 数据,不新增协议字段、不碰 `dozerd`。三个纯函数(`group_tabs_by_agent`/`agent_dot_color`/`agent_cli_command`)各自独立单测覆盖;"＋新建"菜单复用项目树右键菜单已有的"`stack!` 浮层 + 全窗透明 `MouseArea` 点外面关闭"模式,新建会话仍走现成的 `spawn_new_tab`(加一个 `launch: Option<AgentKind>` 参数),自动键入 CLI 名字复用 `acceptance_reject` 已有的"`client.write` 失败就 `tracing::warn!` 降级,不升级成用户可见错误"手法。

**Tech Stack:** Rust,iced 0.14(`iced_widget`),现有 `dozer-client`/`dozer-core` crate。

## Global Constraints

- 一期范围裁剪(spec"非目标",不得擅自扩大):不做用量/配额统计;不新增品牌 logo 图标资源,只用已有 `theme.rs` 调色板着色;不改变 `dozerd` 协议/`SessionSpec`/PTY 生命周期语义(自动键入是"模拟打字",不是把 agent 当 PTY 前台进程跑);不改造终端 tab 条现有的"＋"(`Message::NewTab` 继续开裸 shell)。
- 对话列表/Agent 面板着色统一用现有调色板:Claude=`theme::CYAN`、CodeBuddy=`theme::PURPLE`、OpenCode=`theme::GREEN`、Unknown=`theme::DIM`。**绝不使用 `theme::GOLD`**——CLAUDE.md 明文规定 GOLD 是甲方动作专属色(金 `#F2D94E`),不能被 agent 分类语义借用。
- agent CLI 命令名(自动键入用)固定小写:`claude`/`codebuddy`/`opencode`,与 `AgentKind::label()` 对已知变体的返回值逐字节一致;`Unknown` 不产生任何 CLI 名字。
- 新增的纯函数(`group_tabs_by_agent`/`agent_dot_color`/`agent_cli_command`)一律用私有 `fn`,不用 `pub fn`——同文件里 `dot_color`/`agent_state_label`/`tab_title` 等同类"渲染用纯函数"全部是私有的,测试通过同模块 `mod tests { use super::*; }` 直接访问,没有理由破例开 `pub`(spec 草稿里写的 `pub fn` 是笔误,不是硬性要求)。
- `agent_picker_open` 状态挂在 `Workspace`(不是 `App`)——spec 正文明确写"新增 Workspace 状态",跟"Agent 域面板"绑定当前聚焦项目的既有设计一致(同 `tree_delete_confirm`/`tree_edit` 等项目级浮层状态);spec 后半段渲染伪代码里出现的 `dismiss_layer(app.agent_picker_open)` 是笔误,本计划一律用 `ws.agent_picker_open`。
- 不存在可复用的 `dismiss_layer(bool)` 辅助函数——spec 提到的这个名字在当前代码里没有对应实现,本仓库现有模式是每个浮层分支各自内联一个 `MouseArea::new(container(column![]).width(Fill).height(Fill)).on_press(<关闭消息>)`(见 `workspace.rs` 里 `tree_delete_confirm`/`context_menu` 两个已有分支)。本计划新增的 agent 选择菜单浮层照抄这个内联写法,不新造抽象。

---

### Task 1: Agent 域面板分组渲染

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: 既有 `SessionTab`(`agent`/`agent_state`/`alive`/`cwd`/`info` 字段,`workspace.rs:1095`)、既有纯函数 `dot_color(state: AgentState, alive: bool) -> Color`(`workspace.rs:6951`)、`agent_state_label(state: AgentState) -> &'static str`(`workspace.rs:6930`)、`tab_title(agent: AgentKind, cwd: Option<&Path>, fallback: &str) -> String`(`workspace.rs:6865`)、`AgentKind::label(&self) -> &'static str`(`dozer-core/src/protocol.rs`)。
- Produces:
  - `fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)>`——Task 4 不直接调用,但 Task 4 会在同一个 `agent_list_pane` 函数的头部行里追加"＋新建"按钮,依赖本任务先把函数体改造完。
  - `fn agent_list_row(ws: &Workspace, idx: usize) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>`——仅本文件内部使用。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/workspace.rs` 的 `mod tests`(`workspace.rs:7045` 起,`use super::*;` 之后)结尾、最后一个 `}` 之前,加入:

```rust
    fn make_test_tab(rt: &tokio::runtime::Runtime, id: &str, agent: AgentKind) -> SessionTab {
        SessionTab {
            info: SessionInfo {
                id: id.to_string(),
                name: "shell".into(),
                command: "shell".into(),
                cwd: "/tmp".into(),
                alive: true,
                created_ms: 0,
                agent_state: AgentState::Idle,
                transcript_path: None,
                project_id: None,
                agent,
            },
            model: TerminalModel::new(80, 24),
            alive: true,
            agent_state: AgentState::Idle,
            agent,
            transcript_path: None,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            delivery_pending: false,
            last_turn_head: None,
            tab_id: 0,
            forwarder: rt.spawn(async {}),
        }
    }

    #[test]
    fn group_tabs_by_agent_empty_list() {
        let tabs: Vec<SessionTab> = Vec::new();
        assert_eq!(group_tabs_by_agent(&tabs), Vec::<(AgentKind, Vec<usize>)>::new());
    }

    #[test]
    fn group_tabs_by_agent_single_agent_multiple_sessions() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tabs = vec![
            make_test_tab(&rt, "a", AgentKind::Claude),
            make_test_tab(&rt, "b", AgentKind::Claude),
        ];
        assert_eq!(
            group_tabs_by_agent(&tabs),
            vec![(AgentKind::Claude, vec![0, 1])]
        );
    }

    #[test]
    fn group_tabs_by_agent_mixed_fixed_order_no_empty_groups() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // 故意打乱插入顺序(Opencode 先、Claude 后),验证分组输出顺序
        // 固定为 Claude→Codebuddy→Opencode→Unknown,不是插入顺序;
        // 没有任何会话的 Codebuddy 不出现在结果里(空分组不占位)。
        let tabs = vec![
            make_test_tab(&rt, "a", AgentKind::Opencode),
            make_test_tab(&rt, "b", AgentKind::Claude),
            make_test_tab(&rt, "c", AgentKind::Unknown),
            make_test_tab(&rt, "d", AgentKind::Claude),
        ];
        assert_eq!(
            group_tabs_by_agent(&tabs),
            vec![
                (AgentKind::Claude, vec![1, 3]),
                (AgentKind::Opencode, vec![0]),
                (AgentKind::Unknown, vec![2]),
            ]
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app group_tabs_by_agent -- --nocapture`
Expected: FAIL——编译错误 `cannot find function 'group_tabs_by_agent' in this scope`(函数还不存在)。

- [ ] **Step 3: 实现 `group_tabs_by_agent`**

在 `workspace.rs` 里,紧挨着 `agent_list_pane` 函数(当前 `workspace.rs:4996` 附近,`conversation_list_pane` 的闭合 `}` 之后、"Agent 列表面板"那条注释之前)插入:

```rust
/// 按 `AgentKind` 把会话 tab 分组,固定顺序 Claude → Codebuddy → Opencode
/// → Unknown,只返回非空分组(没有该 agent 的会话就不出现,面板不留空
/// 分组占位)。组内保持 `tabs` 原有顺序(tab 打开顺序)。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`,又要用下标回查
/// `ws.tabs[idx]` 取展示字段,直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 4] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let idxs: Vec<usize> = tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.agent == kind)
                .map(|(i, _)| i)
                .collect();
            (!idxs.is_empty()).then_some((kind, idxs))
        })
        .collect()
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app group_tabs_by_agent -- --nocapture`
Expected: PASS,3 个测试全绿。

- [ ] **Step 5: 改写 `agent_list_pane`,接上真实分组渲染**

把 `workspace.rs` 里当前的 `agent_list_pane` 函数体(当前从 `fn agent_list_pane(` 起到其闭合 `}`,约 `workspace.rs:4996-5030`,函数头部注释"当前只有占位文案,真实 agent 托管留后续任务"一并替换)整体替换成:

```rust
/// Agent 列表面板(右面板区"Agent"视图的列表侧):按 `AgentKind` 分组展示
/// 当前项目的会话,组内保留 tab 打开顺序;点击一行 = `Message::SelectTab`
/// 切焦点(同终端 tab 栏点击效果)。
fn agent_list_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::agent_list_pane();
    let mut content = column![
        row![
            lh(text("Agent")
                .size(workspace_font::subtitle())
                .color(theme::CREAM)),
            lh(text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(workspace_font::label())
            .color(theme::DIM)),
        ]
        .spacing(8)
    ]
    .spacing(region.gap);

    if ws.tabs.is_empty() {
        content = content.push(lh(text("暂无会话")
            .size(workspace_font::body())
            .color(theme::DIM)));
    } else {
        for (agent, idxs) in group_tabs_by_agent(&ws.tabs) {
            content = content.push(lh(text(format!("{}（{}）", agent.label(), idxs.len()))
                .size(workspace_font::caption())
                .color(theme::DIM)));
            for idx in idxs {
                content = content.push(agent_list_row(ws, idx));
            }
        }
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// Agent 面板里单条会话行:状态点(`dot_color`)+ 状态文字
/// (`agent_state_label`)+ 会话名(`tab_title`),整行可点选中该 tab
/// (`idx == ws.active` 时 `theme::CARD` 背景高亮,同项目树选中行的手法,
/// 见 `workspace.rs` 里 `is_selected` 那段)。
fn agent_list_row(
    ws: &Workspace,
    idx: usize,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;
    let row_el = row![
        text("●")
            .size(workspace_font::caption())
            .color(dot_color(tab.agent_state, tab.alive)),
        lh(text(agent_state_label(tab.agent_state))
            .size(workspace_font::caption_sm())
            .color(theme::DIM)),
        lh(text(tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name))
            .size(workspace_font::body())
            .color(theme::CREAM)),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);
    button(row_el)
        .on_press(Message::SelectTab(idx))
        .width(Length::Fill)
        .padding(6)
        .style(move |_t, _s| button::Style {
            background: if active {
                Some(theme::CARD.into())
            } else {
                None
            },
            text_color: theme::CREAM,
            ..button::Style::default()
        })
        .into()
}
```

- [ ] **Step 6: 编译确认(渲染代码没有自动化测试,编译通过即本步验收标准)**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译成功,无 error(warning 若有需排查是否本次改动引入,不是本步引入的既有 warning 不用管)。

- [ ] **Step 7: 跑完整测试套件确认无回归**

Run: `cargo test -p dozer-app 2>&1 | tail -30`
Expected: 全绿,含 Step 4 新增的 3 个 `group_tabs_by_agent_*` 测试。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Agent 域面板按 AgentKind 分组展示真实会话"
```

---

### Task 2: 对话列表 agent 着色

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: 既有 `ConversationMeta.agent: AgentKind`(`crates/dozer-app/src/conversation.rs:15`)、`conversation_list_pane`(`workspace.rs:4889`)。
- Produces: `fn agent_dot_color(agent: AgentKind) -> Color`——仅本文件内部使用,不被后续任务消费。

- [ ] **Step 1: 写失败测试**

在 `mod tests` 结尾(同 Task 1 的插入位置,`}` 之前)加入:

```rust
    #[test]
    fn agent_dot_color_maps_each_kind_and_avoids_gold() {
        let cases = [
            (AgentKind::Claude, theme::CYAN),
            (AgentKind::Codebuddy, theme::PURPLE),
            (AgentKind::Opencode, theme::GREEN),
            (AgentKind::Unknown, theme::DIM),
        ];
        for (agent, expected) in cases {
            let color = agent_dot_color(agent);
            assert_eq!(color, expected, "{agent:?}");
            assert_ne!(
                color, theme::GOLD,
                "{agent:?} 的对话列表圆点色不能是 GOLD(甲方动作专属,CLAUDE.md 明文规定)"
            );
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app agent_dot_color -- --nocapture`
Expected: FAIL——`cannot find function 'agent_dot_color' in this scope`。

- [ ] **Step 3: 实现 `agent_dot_color`**

在 `workspace.rs` 里紧挨着既有 `dot_color` 函数(`workspace.rs:6951` 附近,其闭合 `}` 之后、`tab_item` 之前)插入:

```rust
/// agent → 对话列表圆点颜色。避开 `theme::GOLD`(甲方动作专属色,
/// CLAUDE.md 明文规定,不能被 agent 分类语义借用)。
fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => theme::CYAN,
        AgentKind::Codebuddy => theme::PURPLE,
        AgentKind::Opencode => theme::GREEN,
        AgentKind::Unknown => theme::DIM,
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app agent_dot_color -- --nocapture`
Expected: PASS。

- [ ] **Step 5: `conversation_list_pane` 接上着色圆点**

在 `conversation_list_pane`(`workspace.rs:4889`)里,把卡片标题这一段(当前内容,大致在 `workspace.rs:4959-4963`):

```rust
        let card = button(
            column![
                lh(text(c.title.clone())
                    .size(workspace_font::body())
                    .color(theme::CREAM)),
                lh(text(sub)
                    .size(workspace_font::caption_sm())
                    .color(sub_color)),
            ]
            .spacing(4),
        )
```

改成(标题前加一个按 agent 着色的圆点,副标题文字不变——颜色区分和文字标签是互补关系):

```rust
        let card = button(
            column![
                row![
                    text("●")
                        .size(workspace_font::caption())
                        .color(agent_dot_color(c.agent)),
                    lh(text(c.title.clone())
                        .size(workspace_font::body())
                        .color(theme::CREAM)),
                ]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center),
                lh(text(sub)
                    .size(workspace_font::caption_sm())
                    .color(sub_color)),
            ]
            .spacing(4),
        )
```

- [ ] **Step 6: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译成功。

- [ ] **Step 7: 跑完整测试套件确认无回归**

Run: `cargo test -p dozer-app 2>&1 | tail -30`
Expected: 全绿,含 Step 4 新增测试。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): 对话列表按 agent 着色圆点区分来源"
```

---

### Task 3: 新建会话自动键入 agent CLI 名字

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: 既有 `AgentKind::label(&self) -> &'static str`、既有 `spawn_new_tab(&mut self, io: &ShellIo)`(`workspace.rs:2016`)、既有 `Client::write(&self, id: &str, data: &[u8]) -> Result<()>`(`crates/dozer-client/src/lib.rs:87`,已导入)。
- Produces:
  - `fn agent_cli_command(agent: AgentKind) -> Option<&'static str>`——Task 4 通过 `spawn_new_tab` 的新参数间接触发,不直接调用。
  - `fn spawn_new_tab(&mut self, io: &ShellIo, launch: Option<AgentKind>)`——**签名变化**(新增第二参数),Task 4 直接调用这个新签名(`ws.spawn_new_tab(io, agent)`,`agent: Option<AgentKind>`)。

- [ ] **Step 1: 写失败测试**

在 `mod tests` 结尾加入:

```rust
    #[test]
    fn agent_cli_command_maps_known_agents_and_none_for_unknown() {
        assert_eq!(agent_cli_command(AgentKind::Claude), Some("claude"));
        assert_eq!(agent_cli_command(AgentKind::Codebuddy), Some("codebuddy"));
        assert_eq!(agent_cli_command(AgentKind::Opencode), Some("opencode"));
        assert_eq!(agent_cli_command(AgentKind::Unknown), None);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app agent_cli_command -- --nocapture`
Expected: FAIL——`cannot find function 'agent_cli_command' in this scope`。

- [ ] **Step 3: 实现 `agent_cli_command`**

在 `workspace.rs` 里紧挨着 `tab_title` 函数(`workspace.rs:6865` 之前)插入:

```rust
/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令名。`Unknown`
/// 不该从选择菜单产生(选项只有 Claude/CodeBuddy/OpenCode/纯 Shell 四选
/// 一,纯 Shell 走 `launch: None`,不经过这个函数),但函数保持穷尽
/// match,防止未来枚举新增变体时静默漏写。已知变体的 CLI 名字与
/// `AgentKind::label()` 逐字节一致(`label()` 本身就是给这三个变体返回
/// 小写 CLI 名),这里直接复用而不重复一份映射表,避免两处拼写分叉。
fn agent_cli_command(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Unknown => None,
        known => Some(known.label()),
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app agent_cli_command -- --nocapture`
Expected: PASS。

- [ ] **Step 5: `spawn_new_tab` 加 `launch` 参数,attach 成功后自动键入**

把 `workspace.rs:2016` 的函数签名:

```rust
    fn spawn_new_tab(&mut self, io: &ShellIo) {
```

改成:

```rust
    fn spawn_new_tab(&mut self, io: &ShellIo, launch: Option<AgentKind>) {
```

在同一个函数体内,把这一段(当前 `workspace.rs:2037-2062` 附近的 async 块里):

```rust
            match client.attach(&info.id, 0).await {
                Ok((snapshot, _next_offset, rx)) => {
                    if proxy
                        .send_event(Message::TabAttached(project_id, tab_id, info, snapshot))
                        .is_err()
                    {
                        return;
                    }
                    forward_events(project_id, tab_id, rx, proxy).await;
                }
```

改成(`info` 会被下面的 `Message::TabAttached(...)` 按值移走,自动键入要用的会话 id 必须在那一行之前先 `clone()` 出来——同 `acceptance_reject` 已有的"先 `clone` 出 id 再异步用"手法):

```rust
            match client.attach(&info.id, 0).await {
                Ok((snapshot, _next_offset, rx)) => {
                    let session_id = info.id.clone();
                    if proxy
                        .send_event(Message::TabAttached(project_id, tab_id, info, snapshot))
                        .is_err()
                    {
                        return;
                    }
                    if let Some(agent) = launch
                        && let Some(cmd) = agent_cli_command(agent)
                    {
                        let bytes = format!("{cmd}\n").into_bytes();
                        if let Err(e) = client.write(&session_id, &bytes).await {
                            tracing::warn!("自动键入 agent CLI 失败: {e}");
                        }
                    }
                    forward_events(project_id, tab_id, rx, proxy).await;
                }
```

- [ ] **Step 6: 更新两个既有调用点**

第一处,`ensure_project_terminal`(`workspace.rs:2011-2013` 附近):

```rust
    fn ensure_project_terminal(&mut self, io: &ShellIo) {
        if self.project.is_some() && self.tabs.is_empty() {
            self.spawn_new_tab(io);
        }
    }
```

改成:

```rust
    fn ensure_project_terminal(&mut self, io: &ShellIo) {
        if self.project.is_some() && self.tabs.is_empty() {
            self.spawn_new_tab(io, None);
        }
    }
```

第二处,`App::update` 里的 `Message::NewTab` 分支(`workspace.rs:3142`):

```rust
            Message::NewTab => self.with_focused_project(|ws, io| ws.spawn_new_tab(io)),
```

改成:

```rust
            Message::NewTab => self.with_focused_project(|ws, io| ws.spawn_new_tab(io, None)),
```

- [ ] **Step 7: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译成功——若报"参数数量不对"说明还有遗漏的调用点没改全,回头 `grep -n "spawn_new_tab" crates/dozer-app/src/workspace.rs` 核对是否只剩这两处 call site 加上函数定义本身。

- [ ] **Step 8: 跑完整测试套件确认无回归**

Run: `cargo test -p dozer-app 2>&1 | tail -30`
Expected: 全绿,含 Step 4 新增测试。`spawn_new_tab` 本身是异步方法,依赖真实 `ShellIo`(daemon client/tokio handle/事件回灌通道),不易 headless 单测——`None` 分支的"新建会话行为跟改动前逐字节一致"由本步的全量测试回归 + Task 4 完成后的人工验收共同兜底,不是本任务遗漏。

- [ ] **Step 9: 提交**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): spawn_new_tab 加 launch 参数,attach 成功后自动键入 agent CLI"
```

---

### Task 4: "＋新建" agent 选择菜单

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: Task 1 的 `agent_list_pane`(要在其头部行插入按钮)、Task 3 的 `spawn_new_tab(&mut self, io: &ShellIo, launch: Option<AgentKind>)`、既有 `context_menu_open()`(`workspace.rs:2741`,`App::agent_picker_open()` 照此手法新增)、既有 `with_focused_project(&mut self, f: impl FnOnce(&mut Workspace, &ShellIo))`(`workspace.rs:2518`)、既有 `delete_confirm_popup(ws: &Workspace) -> Element<...>` 同款"固定浮层,不用像素坐标"手法(`workspace.rs:6465`附近)。
- Produces: 无后续任务消费——这是本 spec 范围内的最后一块拼图。

- [ ] **Step 1: `Workspace` 加 `agent_picker_open` 字段**

在 `struct Workspace`(`workspace.rs:1252` 起)里,`tree_edit: Option<TreeEdit>,` 字段(`workspace.rs:1309` 附近)之后加:

```rust
    /// Agent 面板"＋新建"菜单当前是否打开。不需要坐标——面板顶部固定
    /// 位置的下拉,不像项目树右键菜单需要跟随点击坐标。
    agent_picker_open: bool,
```

在 `Workspace::empty_for_project_placeholder()`(`workspace.rs:1538` 起,**这是唯一的字段字面量构造点**——`bootstrap`/`loading_for_project` 都用 `..Self::empty_for_project_placeholder()` 结构更新语法继承缺省值,不需要分别改)里,`tree_edit: None,` 那一行之后加:

```rust
            agent_picker_open: false,
```

- [ ] **Step 2: `Message` 加三个新变体**

在 `pub enum Message`(`workspace.rs:815` 起)里,`NewTab,`(`workspace.rs:864`)之后加:

```rust
    /// Agent 面板"＋新建"按钮:开/关 agent 选择菜单。
    AgentPickerToggle,
    /// agent 选择菜单:点击菜单外/Esc,关闭不建会话。
    AgentPickerClose,
    /// agent 选择菜单:选中一项(`None` = 纯 Shell,同现有"＋"效果;
    /// `Some(agent)` = 新建会话后自动键入该 agent 的 CLI 名字)。
    AgentPickerSelect(Option<AgentKind>),
```

- [ ] **Step 3: `App::update` 接上三个新消息**

在 `App::update` 的 `match message { ... }` 里,`Message::NewTab => ...`(`workspace.rs:3142`)之后加:

```rust
            Message::AgentPickerToggle => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = !ws.agent_picker_open;
                });
            }
            Message::AgentPickerClose => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = false;
                });
            }
            Message::AgentPickerSelect(agent) => {
                self.with_focused_project(|ws, io| {
                    ws.agent_picker_open = false;
                    ws.spawn_new_tab(io, agent);
                });
            }
```

- [ ] **Step 4: `App::agent_picker_open()` 只读访问器**

在 `App::context_menu_open()`(`workspace.rs:2741` 附近)之后加:

```rust
    /// Agent 选择菜单是否打开(main.rs Esc 键路由用)。
    pub fn agent_picker_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.agent_picker_open)
            .unwrap_or(false)
    }
```

- [ ] **Step 5: `agent_list_pane` 头部行加"＋新建"按钮**

把 Task 1 已改好的 `agent_list_pane` 里这一段头部行(`content` 初始化里的第一个 `row![...]`):

```rust
    let mut content = column![
        row![
            lh(text("Agent")
                .size(workspace_font::subtitle())
                .color(theme::CREAM)),
            lh(text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(workspace_font::label())
            .color(theme::DIM)),
        ]
        .spacing(8)
    ]
    .spacing(region.gap);
```

改成(加一个撑开的空白 + "＋新建"按钮):

```rust
    let mut content = column![
        row![
            lh(text("Agent")
                .size(workspace_font::subtitle())
                .color(theme::CREAM)),
            lh(text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(workspace_font::label())
            .color(theme::DIM)),
            iced_widget::space::horizontal(),
            agent_picker_toggle_button(),
        ]
        .spacing(8)
    ]
    .spacing(region.gap);
```

- [ ] **Step 6: 新增 `agent_picker_toggle_button` 与 `agent_picker_popup` 渲染函数**

在 `workspace.rs` 里紧挨着 `agent_list_row`(Task 1 新增,`agent_list_pane` 之后)插入:

```rust
/// Agent 面板头部"＋新建"按钮:点击切换 `agent_picker_open`,弹出 agent
/// 选择菜单(`agent_picker_popup`)。样式复用终端 tab 栏"＋"
/// (`Message::NewTab` 那颗,`workspace.rs` 里 `plus` 变量)同款
/// CARD 底 + BORDER 描边。
fn agent_picker_toggle_button<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(
        text("＋新建")
            .size(workspace_font::label())
            .color(theme::CREAM),
    )
    .on_press(Message::AgentPickerToggle)
    .padding([4, 10])
    .style(|_theme, _status| button::Style {
        background: Some(theme::CARD.into()),
        text_color: theme::CREAM,
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

/// Agent 选择菜单浮层:固定挂在窗口右上角("＋新建"按钮下方——该按钮
/// 就在最靠右的 Agent 面板头部,近似等于窗口右上角),四个选项
/// Claude/CodeBuddy/OpenCode/纯 Shell。跟项目树右键菜单
/// (`context_menu_popup`)同款按钮样式,但不需要像素坐标定位——同
/// `delete_confirm_popup` 一样固定 padding 摆位。`ws.agent_picker_open`
/// 为假时返回空视图,调用方(`App::view`)据此决定要不要把这层塞进
/// `stack!`。
fn agent_picker_popup(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    if !ws.agent_picker_open {
        return column![].into();
    }
    let items: [(&str, Option<AgentKind>); 4] = [
        ("Claude", Some(AgentKind::Claude)),
        ("CodeBuddy", Some(AgentKind::Codebuddy)),
        ("OpenCode", Some(AgentKind::Opencode)),
        ("纯 Shell", None),
    ];
    let mut col = column![].spacing(2);
    for (label, agent) in items {
        col = col.push(
            button(text(label).size(workspace_font::body()).color(theme::CREAM))
                .on_press(Message::AgentPickerSelect(agent))
                .width(Length::Fixed(140.0))
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::CREAM,
                    ..button::Style::default()
                }),
        );
    }
    let list = container(col).padding(6).style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    });
    // 右上角固定偏移:48px 避开顶栏,16px 避开窗口右边缘。这是估算值,
    // 不是像素级对齐"＋新建"按钮(spec 明确"不算点击坐标")——Task 4 最后
    // 一步的人工验收里如果视觉上偏得明显,回来调这两个数字即可,不影响
    // 其余逻辑。
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Right)
        .align_y(iced_widget::core::alignment::Vertical::Top)
        .padding(Padding {
            top: 48.0,
            left: 0.0,
            right: 16.0,
            bottom: 0.0,
        })
        .into()
}
```

- [ ] **Step 7: 接入 `stack!` 浮层(与项目树右键菜单互斥,右键优先)**

在 `App::view` 里那段 `popped` 判断链(`workspace.rs:3871-3889` 附近,`ws.tree_delete_confirm.is_some()` → `self.context_menu.is_some()` → `else`)里,在 `self.context_menu.is_some()` 分支和最终 `else` 之间插入一个新分支:

```rust
        let popped = if ws.tree_delete_confirm.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeDeleteCancel);
            stack![base, dismiss, delete_confirm_popup(ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeContextMenuClose);
            stack![base, dismiss, context_menu_popup(self, ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.agent_picker_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::AgentPickerClose);
            stack![base, dismiss, agent_picker_popup(ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base.into()
        };
```

(只有中间新增的 `else if ws.agent_picker_open { ... }` 分支是新代码,其余原样保留——两个浮层互斥靠 if/else if 链的顺序天然做到:右键菜单打开时 agent 选择菜单不可能同时判断为真,这条只是防御性排序。)

- [ ] **Step 8: main.rs 加 Esc 关闭路由**

在 `crates/dozer-app/src/main.rs` 里,紧挨着现有的右键菜单 Esc 处理块(`main.rs:507-522` 附近,`if app.context_menu_open() && ... { app.update(Message::ProjectTreeContextMenuClose); window.request_redraw(); return; }`)之后加:

```rust
            // Agent 选择菜单打开时,Esc 优先关菜单,同右键菜单的处理口径
            // (见上一段紧邻的 context_menu_open 分支的注释)。
            if app.agent_picker_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::AgentPickerClose);
                window.request_redraw();
                return;
            }
```

- [ ] **Step 9: main.rs 选中 agent 后聚焦终端**

在 `main.rs` 里那段 `FocusIntent` 判断(`main.rs:747-751` 附近):

```rust
            } else if matches!(
                message,
                Message::NewTab | Message::SelectTab(_) | Message::TabAttached(_, _, _, _)
            ) {
                *pending_focus = Some(FocusIntent::Terminal);
            }
```

改成(加一个 `AgentPickerSelect(_)`——选完 agent 建出新会话后,跟点"＋"一样该把焦点交给新终端,不然用户看到自动键入了 `claude` 却光标还停在别处):

```rust
            } else if matches!(
                message,
                Message::NewTab
                    | Message::SelectTab(_)
                    | Message::TabAttached(_, _, _, _)
                    | Message::AgentPickerSelect(_)
            ) {
                *pending_focus = Some(FocusIntent::Terminal);
            }
```

- [ ] **Step 10: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译成功,无 error。

- [ ] **Step 11: 跑完整测试套件确认无回归**

Run: `cargo test -p dozer-app 2>&1 | tail -30`
Expected: 全绿(本任务不新增自动化测试——浮层渲染/键盘路由/异步自动键入这三样都在"人工验收"覆盖范围,理由见 Step 12)。

- [ ] **Step 12: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): Agent 面板新增＋新建入口,选 agent 自动键入 CLI"
```

- [ ] **Step 13: 人工验收(真机,`cargo run -p dozer-app`)**

本仓一贯惯例(同 P1L 右键菜单):浮层渲染/键盘路由/PTY 自动键入的真实效果 headless 只能编译验证,必须真机过一遍。

```bash
cargo run -p dozer-app
```

验收清单:
1. 打开一个项目,切到右栏"Agent"视图(`RightView::Agent`)。若还没有任何会话,列表侧应显示"暂无会话";点终端区已有的"＋"新建一个裸 shell 后,Agent 面板应该出现一个 `Unknown` 分组(hook 事件到达前默认 `Unknown`),分组标题"未知（1）"。
2. 点 Agent 面板头部的"＋新建"按钮:应弹出一个纵向菜单,四个选项 Claude/CodeBuddy/OpenCode/纯 Shell,大致挂在按钮下方(窗口右上角附近)。若视觉上明显偏离按钮位置,回 Step 6 调 `agent_picker_popup` 里的 `top`/`right` 两个数字。
3. 点菜单外任意位置:菜单应关闭,不建任何新会话(`ws.tabs.len()` 不变)。
4. 再次打开菜单,选"Claude":应新建一个终端 tab 并自动切焦点过去,tab 里应该能看到 `claude` 被打出来并回车执行(或至少看到 `claude` 字样,取决于本机是否装了 `claude` CLI——没装会看到 shell 报 `command not found: claude`,这是预期行为,不是 bug,自动键入只保证"打出那行字",不保证目标程序存在)。
5. 打开菜单,按 Esc:菜单应关闭,不建新会话。
6. 打开菜单,选"纯 Shell":应新建一个终端 tab,效果跟直接点终端区现有的"＋"完全一样(没有任何字被自动打出来)。
7. 切到右栏"对话"视图(`RightView::Conversations`),确认历史对话卡片标题前出现按 agent 着色的圆点(不同 agent 来源的对话圆点颜色不同,均不是金色)。
8. 确认项目树右键菜单打开时,Agent 选择菜单不会同时出现在屏幕上(互斥抽查一次即可)。

若上述任一步骤与预期不符,回退到对应 Task 的实现步骤修正,不要在验收阶段临时打补丁。

---

## 完成检查

- [ ] `cargo test -p dozer-app` 全绿,含本计划新增的全部单测:`group_tabs_by_agent_*`(3 个)、`agent_dot_color_maps_each_kind_and_avoids_gold`、`agent_cli_command_maps_known_agents_and_none_for_unknown`。
- [ ] `cargo build -p dozer-app` 与 `cargo clippy -p dozer-app --all-targets` 均无 error(clippy warning 若非本计划改动引入可不修)。
- [ ] Task 4 Step 13 的 8 条人工验收全部过一遍,真机确认浮层位置、Esc 关闭、点外关闭、自动键入、纯 Shell 分支、对话列表着色、菜单互斥。
- [ ] 本计划范围明确排除(spec"非目标",不是遗漏):用量/配额统计;品牌 logo 图标;`dozerd` 协议/`SessionSpec`/PTY 生命周期语义改动;终端 tab 条现有"＋"的行为改造。
