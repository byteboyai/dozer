# Agent 域面板设计

**状态：已批准（brainstorming 会话，2026-08-05）**

## 背景

2026-07-14 的需求讨论定下"项目与 Agent 是两个顶级元素"（见 memory `dozer-vision`/`dozer-open-questions`），
右栏映射为"Agent 域"。P2b 多 agent foundation 已经把地基铺好——`AgentKind` 协议枚举、
`SessionTab.agent`/`agent_state` 字段、`dot_color`/`agent_state_label` 渲染辅助函数、
`conversation_list_pane` 里 `ConversationMeta.agent`——但消费端从没接上：

- `agent_list_pane`（`crates/dozer-app/src/workspace.rs:4518` 附近，`RightView::Agent` 视图的
  内容侧）目前渲染的就是一行文字"Agents（后续）"，纯占位符。
- `conversation_list_pane` 只把 `c.agent.label()` 拼进副标题文字，没有视觉区分。
- 完全没有"新建会话选 agent"的入口——现有"＋"（`Message::NewTab`）恒开一个裸 `$SHELL`，
  用户进去自己手敲 `claude`/`codebuddy`/`opencode`。

本设计把这三处补上，只用已经落地的数据（`ws.tabs`），不新增协议字段，不碰 `dozerd`。

## 目标 / 非目标

**目标**：
1. Agent 域面板按 `AgentKind` 把当前项目的会话分组展示，每行给出四态状态与点击可切换焦点。
2. 面板顶部新增"＋新建"入口，选 agent 后开一个新终端 tab 并自动帮用户键入对应 CLI 名字。
3. 对话历史列表用按 agent 着色的圆点区分来源，替代纯文字标签。

**非目标**：
- **用量/配额统计**——没有任何 token/额度数据源，做等于新开一个独立大功能，本次不做。
- **品牌 logo 图标**——不新增图标资源，只用已有调色板着色，避免商标风险与额外美术工作。
- **改变 dozerd 协议/`SessionSpec`/PTY 生命周期语义**——自动键入 CLI 名字是纯 dozer-app 侧
  的"模拟打字"，agent 进程退出后 tab 仍是活的 shell，跟用户手敲效果逐字节一致；不会像
  "直接把 agent 当 PTY 前台进程跑"那样让 agent 退出=整个 tab 死掉。
- **改造终端 tab 条现有的"＋"**（`Message::NewTab`）——它继续开裸 shell，跟 Agent 面板新增
  的"＋新建"是两个并行入口，互不影响。

## 关键语义确认（brainstorming 会话定案）

- Agent 简卡 v1 范围只做"会话分组 + 状态"，用量整体推后。
- "新建会话选 agent"放在 Agent 域面板顶部的"＋新建"，不改造现有 tab 条的"＋"。
- 选完 agent 后开普通 shell，attach 成功后自动向 PTY 写入 agent CLI 名字 + 回车，不直接把
  agent 二进制当 PTY 前台进程跑。
- 对话列表的 agent 区分用现有调色板着色圆点（Claude=`CYAN`、CodeBuddy=`PURPLE`、
  OpenCode=`GREEN`、未知=`DIM`），不新增图标资源，避开 `GOLD`（CLAUDE.md 规定甲方动作
  专属色，不能被 agent 分类语义借用）。

## 架构与数据流

### 1. Agent 域面板分组

新增纯函数（不碰 iced，可独立单测）：

```rust
/// 按 `AgentKind` 把会话 tab 分组，固定顺序 Claude → Codebuddy → Opencode
/// → Unknown，只返回非空分组（没有该 agent 的会话就不出现，面板不留空
/// 分组占位）。组内保持 `tabs` 原有顺序（tab 打开顺序）。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`，又要用下标回查
/// `ws.tabs[idx]` 取展示字段，直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
pub fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)>
```

`agent_list_pane`（`workspace.rs:4518`）改写：
- 头部不变（"Agent" + 项目名），新增一个"＋新建"按钮（见下一节）。
- 主体：`group_tabs_by_agent(&ws.tabs)` 遍历，每组一个分组标题行（`agent.label()` +
  `({n})` 计数，`theme::DIM`），组内每行复用：
  - 状态点：`dot_color(tab.agent_state, tab.alive)`（已有函数，`workspace.rs:6338` 附近）
  - 状态文字：`agent_state_label(tab.agent_state)`（已有函数，`workspace.rs:6317`）
  - 会话名：`tab_title(tab.cwd.as_deref(), &tab.info.name)`（已有函数，`tab_item` 同款）
  - 整行包一个 `button`，`on_press(Message::SelectTab(idx))`；`idx == ws.active` 时用
    `theme::CARD` 背景高亮（对齐 `conversation_list_pane` 当前活跃对话的高亮手法）。
- 空态：`ws.tabs.is_empty()` 时显示"暂无会话"（`theme::DIM`），不进分组循环。

### 2. "＋新建"agent 选择菜单

**新增 `Workspace` 状态**：

```rust
/// Agent 面板"＋新建"菜单当前是否打开。不需要坐标——面板顶部固定位置
/// 的下拉，不像项目树右键菜单需要跟随点击坐标。
agent_picker_open: bool,
```

**新增 `Message` 变体**：

```rust
AgentPickerToggle,                    // 点击"＋新建"按钮,开/关菜单
AgentPickerClose,                     // 点击菜单外/Esc,关闭不建会话
AgentPickerSelect(Option<AgentKind>), // 选中一项;None = 纯 Shell
```

**`spawn_new_tab` 加一个参数**（`workspace.rs:1975`），不新建平行方法，避免复制整段
create+attach+forward 逻辑：

```rust
fn spawn_new_tab(&mut self, io: &ShellIo, launch: Option<AgentKind>)
```

原有唯一调用点 `Message::NewTab => ws.spawn_new_tab(io)`（`workspace.rs:3094`）改成
`ws.spawn_new_tab(io, None)`；新增 `Message::AgentPickerSelect(agent) => { ws.agent_picker_open
= false; ws.spawn_new_tab(io, agent); }`。

函数体内，`client.attach` 成功、`proxy.send_event(Message::TabAttached(...))` 之后（原有
`workspace.rs:2007-2013` 那段异步块里），追加：

```rust
if let Some(agent) = launch
    && let Some(cmd) = agent_cli_command(agent)
{
    let bytes = format!("{cmd}\n").into_bytes();
    if let Err(e) = client.write(&info.id, &bytes).await {
        tracing::warn!("自动键入 agent CLI 失败: {e}");
    }
}
```

这跟仓库里已有的"打回验收意见注回 PTY"（`acceptance_reject`，`workspace.rs:2205-2225`，
`io.handle.spawn(async move { client.write(&id, text_out.as_bytes()).await })` +
`tracing::warn!` 失败降级）是同一手法，不需要新的状态机、不需要在 `TabAttached` 之后再
异步补发一次——`client.write` 用的正是 `dozer-client/src/lib.rs:87` 那个已有的
`Client::write(&self, id: &str, data: &[u8]) -> Result<()>`，跟用户在终端里敲字时走的
同一条路径。**因此不需要引入 `pending_agent_type` 这类跨消息状态**——`launch` 参数在
同一个 async 块里从"选中 agent"直接用到"attach 成功后写入"，生命周期完全在一次
`spawn_new_tab` 调用内闭合。

```rust
/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令名。`Unknown`
/// 不该从选择菜单产生（选项只有 Claude/CodeBuddy/OpenCode/纯 Shell 四选
/// 一，纯 Shell 走 `launch: None`，不经过这个函数），但函数保持穷尽
/// match，防止未来枚举新增变体时静默漏写。
pub fn agent_cli_command(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Claude => Some("claude"),
        AgentKind::Codebuddy => Some("codebuddy"),
        AgentKind::Opencode => Some("opencode"),
        AgentKind::Unknown => None,
    }
}
```

**渲染**（`stack!` 浮层，比照项目树右键菜单已有的 `context_menu_popup`/`dismiss_layer`
模式，但更简单——固定挂在"＋新建"按钮下方，不用像素坐标定位）：

```rust
stack![
    base_view,
    dismiss_layer(app.agent_picker_open), // 复用同款"全窗透明 MouseArea,点外面关闭"
    agent_picker_popup(app),              // 固定 padding 推到按钮下方,不算点击坐标
]
```

`agent_picker_popup` 是纵向 `column!`，四个 `button`："Claude"/"CodeBuddy"/"OpenCode"/
"纯 Shell"，样式复用 `context_menu_popup` 的按钮样式（`theme::CARD` 底 + `theme::BORDER`
描边）。`Esc` 关闭走 `main.rs` 既有键盘拦截层，加一条"若 `agent_picker_open` 且按 Esc →
`AgentPickerClose`"分支。两个浮层（项目树右键菜单、agent 选择菜单）互斥：`context_menu`
优先渲染——项目树右键更局部、更常出现在"正在操作某一行"的语境里；`agent_picker_popup`
只在 `context_menu.is_none()` 时才可能出现在 `stack!` 里（正常使用下二者本就不会同时触发，
这条只是防御性排序，不是真实会遇到的场景）。

### 3. 对话列表 agent 着色

新增纯函数：

```rust
/// agent → 对话列表圆点颜色。避开 `theme::GOLD`（甲方动作专属色，
/// CLAUDE.md 明文规定，不能被 agent 分类语义借用）。
pub fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => theme::CYAN,
        AgentKind::Codebuddy => theme::PURPLE,
        AgentKind::Opencode => theme::GREEN,
        AgentKind::Unknown => theme::DIM,
    }
}
```

`conversation_list_pane`（`workspace.rs:4411`）每行卡片标题前加一个
`text("●").color(agent_dot_color(c.agent))`；副标题里现有的 `c.agent.label()` 文字保留——
颜色区分和文字标签是互补关系，不是互斥替换。

## 错误处理

- 自动键入 CLI 名字的 `client.write` 调用失败（daemon 掉线等）：`tracing::warn!` 记录，
  不升级成 `Message::DaemonError` 弹用户可见的错误——tab 本身已经建好，用户看到一个空
  shell，自己手敲即可，不是致命错误（对齐 `acceptance_reject` 现有的降级级别）。
- `agent_picker_open` 状态本身没有失败模式（纯本地布尔翻转），不需要错误处理。

## 测试策略

- `group_tabs_by_agent`：空列表、单 agent 多会话、多 agent 混合（验证分组顺序固定
  Claude→Codebuddy→Opencode→Unknown、空分组不出现）。
- `agent_cli_command`：四个 `AgentKind` 值各自映射，`Unknown` 返回 `None`。
- `agent_dot_color`：四个 `AgentKind` 值各自映射，断言都不等于 `theme::GOLD`。
- `spawn_new_tab` 新增的 `launch` 参数分支：现有测试基础设施（若已有针对 `spawn_new_tab`
  的测试）确认 `None` 时行为与改动前逐字节一致（回归保护）；`Some(agent)` 分支的
  "attach 成功后触发一次 write"这段异步逻辑不易 headless 单测，留给下面的人工验收。
- 浮层渲染/键盘路由/自动键入的真实效果：headless 只能编译验证，真机
  `cargo run -p dozer-app` 目测（本仓一贯惯例，同 P1L 右键菜单）——重点验收点：选
  "Claude"后新 tab 里应该看到 `claude` 被打出来并回车执行；选"纯 Shell"应该跟现有
  "＋"按钮效果完全一样。

## 依赖变更

无新增依赖。
