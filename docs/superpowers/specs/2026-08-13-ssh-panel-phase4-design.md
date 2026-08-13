# SSH 主机面板 阶段 4:UI 重写 + 终端内嵌 设计

**状态:已批准(brainstorming 会话,2026-08-13)**

## 背景

`extensions::ssh`(`crates/dozer-app/src/extensions/ssh.rs`)已经完成阶段 1
(主机连接管理 + 认证 + 连接测试,见
`docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`)和阶段 2
(SSH 终端,复用现有 `TerminalModel`/`term_view.rs` 渲染管线,见
`docs/superpowers/specs/2026-08-08-ssh-panel-phase2-design.md`)。用户给了新
参考草图,要求两块重构:

1. **视觉重写**:主机卡片/编辑表单目前是纯文字按钮 + 靛蓝/紫色配色(与
   ByteBoy2077 主题不符),草图要求图标化按钮 + 金色主题,表单字段重排、
   真正的单选按钮、删除按钮挪进表单。
2. **终端内嵌**:阶段 2 把 SSH 终端做成了与本地/agent 会话同构的 tab,
   点"终端"会离开 SSH 面板、跳到右侧与 agent 共用的终端 tab 条(见阶段 2
   设计文档目标 #2"SSH tab 与本地 tab 同构")。草图要求 SSH 终端渲染在
   SSH 面板自己的视图里(独立 tab 条 + 终端窗口),不再跳转。

这两块合起来命名"阶段 4",延续现有阶段编号;草图里的第三块(SFTP 文件
传输,tab 条另一个 tab)是文档里从未开工的"阶段 3",单独成
`docs/superpowers/specs/2026-08-13-ssh-panel-phase3-sftp-design.md`,依赖
本文档新建的 tab 基础设施。

## 目标 / 非目标

**目标**:

1. 主机卡片(`host_card`)重写:3 个图标按钮(文件传输/终端/设置——设置
   即原来的"编辑"),取代现有 4 个纯文字按钮(测试连接/终端/编辑/删除)；
   配色改用 `theme::color` 现有 token(`CARD`/`BORDER`/`GOLD`/`CREAM`),不
   再用 iced 默认的靛蓝/紫色。
2. 编辑表单(`host_form`)字段顺序对齐草图:主机名称→Host→port→user
   name→密码/私钥单选(真正的圆形单选按钮,不是现在的 `●`/`○` 文字前缀
   按钮)→凭证输入框→底部三按钮 删除/保存/取消(现有"测试连接"按钮也
   移进表单,放在这三个按钮旁边;不删除这个功能)。
3. "+添加主机"入口从侧栏顶部挪到侧栏底部(草图"+添加"位置),对齐 Todo
   面板重构确立的"操作入口在列表下方"的布局语言。
4. 新增一个 SSH 面板自有的 tab 集合(与 `Workspace.tabs`/`active`——右侧
   共享的 agent/本地终端 tab 条——完全独立),渲染在 SSH 面板自己的视图
   里:顶部 tab 条(可关闭,复用 `tabs::tab_core`) + 下方终端内容区。
5. 点主机卡片"终端"图标 → 在 SSH 面板自己的 tab 条里开一个终端 tab 并
   聚焦,不再跳到右侧共享终端条,不改变 `RightView`/`LeftView`。
6. 终端渲染本身(`TerminalModel`/`term_view::view`/`TabBackend::Ssh` 的
   russh 泵循环)全部复用阶段 2 已有实现,不重写字节处理/PTY 管线。
7. 键盘/粘贴/滚轮/选区四类输入,SSH 面板内嵌终端和右侧共享终端条能够
   同时存在于屏幕上而不互相抢字节——通过给终端相关消息加一个"目标"
   区分字段实现(见"架构与数据流"第 4 节)。

**非目标**:

- 不做 agent(AI)通过任何机制(MCP 或其它)向 SSH 终端写入指令——已核实
  `dozer-mcp` 目前只有一个只读工具(`get_preview_context`),没有任何写入/
  控制类工具;`dispatch_todo_to_existing` 虽然技术上能把文本写进 SSH tab,
  但只能由人在 Todo 面板派发弹层里手动触发,不是 agent 可达的接口。这个
  能力(以及草图第一版终端示例底部曾经出现、后来被用户确认不需要的那个
  空面板)留给以后单独立项。
- 不做看板/多主机同 tab 并排显示之类的布局实验——tab 条严格是"一个
  (主机,种类)= 一个 tab"，横向排列，不分栏。
- 不引入 SFTP(见阶段 3 文档),本文档只搭好"tab 种类可扩展"的基础设施
  (见下方 `SshTabKind`),阶段 3 往里加 `Sftp` 变体。
- 不改变主机数据模型(`SshHost`/`AuthMethod`/keyring 存取方式)、不改变
  `.dozer/ssh_hosts.json` 格式。
- 不做 host-key 信任流程(`TrustHostKey`/`UnknownHostKey`/`KeyChanged`)的
  任何行为改动,只挪按钮位置,消息处理逻辑不变。
- 不迁移侧栏"+添加"按钮之外的其它按钮到 `icons::icon_button_entry`——
  见下方"共享组件评估"一节。

## 架构与数据流

### 1. 主机卡片 / 编辑表单视觉重写(`ssh.rs` 内部)

**新增图标**:草图的"文件传输"图标(文件夹 + 环形箭头)在现有
`icons::IconKind` 里没有对应资源(已核实 `icons.rs` 全部 129 个变体里没有
任何 sync/transfer 语义的图标,`RefreshCw` 语义是"刷新",不能挪用)。新增
一个 Lucide 图标资源(`folder-sync.svg` 或语义等价的图标,写计划阶段核实
具体取用哪个 Lucide 名字与 sketch 最像)+ 对应 `IconKind::FolderSync` 变体
(命名待写计划阶段与阶段 3 的 tab 图标复用点一起定,两处必须用同一个
`IconKind`)。终端图标复用既有 `IconKind::Terminal`,设置图标复用既有
`IconKind::Settings`。

**`host_card` 重写**:3 个图标按钮用 `icons::icon_button_entry`(形状完全
匹配——固定尺寸纯图标方块按钮,草图里这三个就是纯图标、无文字标签,不像
Todo 面板的分类导航按钮那样要塞文字+计数)：

```rust
icons::icon_button_entry(
    IconKind::FolderSync, // 或 Terminal / Settings
    theme::icon_size::row(),
    /* active */ false, // 主机卡片按钮没有"选中态"概念
    hover_t,
    /* card */ true,
    button_size,
    /* interactive */ true,
    on_select_message,
    on_hover,
    tooltip,
)
```

三个按钮点击分别发:`Message::OpenSshTab(host.id.clone(), SshTabKind::
Terminal)`(新变体,替代原 `OpenTerminal`,见下方"Message 变更")、
`Message::OpenSshTab(host.id.clone(), SshTabKind::Sftp)`(阶段 3 才真正
有内容,阶段 4 先接好消息,面板侧可以先不渲染这个按钮或渲染但阶段 3 前
点击无实际效果——写计划阶段按"先不渲染,阶段 3 一起加"处理,避免半成品
按钮出现在阶段 4 交付里)、`Message::EditHostStart(host.id.clone())`(原
"编辑"按钮语义不变,只是从文字按钮换成图标按钮)。

卡片本身容器样式(`CARD` 背景 + `BORDER` 1px 描边 + 8.0 圆角)已经符合
Todo/Project 面板确立的边框卡片规范,不用改;`host.name`/连接串/认证方式
文字展示不变,只去掉原来的"测试连接"文字按钮和状态行(状态行/测试连接
挪进编辑表单,见下)。

**`host_form` 重写**:
- 字段 `text_input` 顺序不变(名字/host/port/username 已经和草图一致),
  占位符文案可以保持现状或对齐草图("主机名称"/"Host"/"port(22)"/
  "user name"),写计划阶段按 UI 文案统一习惯定。
- 密码/私钥选择从两个独立 toggle 按钮(`● 私钥`/`○ 密码`)换成真正的
  单选控件:iced 没有原生 radio widget,用两个圆形 `container`(选中态
  `GOLD` 实心圆点 + `GOLD` 描边,未选中态空心 `BORDER` 描边)包一层
  `MouseArea`/`button`,点击发 `Message::DraftAuthMethodToggled(bool)`
  (消息本身不用改,只改渲染)。
- 底部按钮从"保存/取消"两个,改成"删除/保存/取消"三个 + 拿掉的"测试
  连接"按钮:`row![测试连接, 删除, 保存, 取消]`(顺序:测试连接靠左,
  删除/保存/取消靠右,写计划阶段可以按实际视觉效果微调左右分组,不是
  强约束)。"删除"新按钮发 `Message::DeleteHost(id)`(已有消息,原来
  只能从卡片触发,现在编辑表单里也能触发——`update()` 对 `DeleteHost`
  的处理逻辑不用改,只是新增一个触发点;新建主机时(`draft.id ==
  None`)不显示删除按钮,没有可删的对象)。"测试连接"按钮发既有的
  `Message::TestConnection(id)`,新建主机时(`id` 还没有)禁用/不显示
  (未保存的草图没有 `id`,`TestConnection` 需要先在 `ws_state.hosts`
  里查到,查不到直接 no-op,行为不会崩但体验上不显示更清楚)。
- 状态行(现有 `host_card` 里的"✓ 连接成功"/"⚠ 未知主机…"文字)一并挪
  进表单,展示在按钮行下方——这是"测试连接"结果自然的落点,现有
  `ws_state.test_status(&host.id)` 查询方式不变。

**"+添加主机"位置**:`view()` 里现在是
`column![home_panel_head(...), button("＋新增主机"), [editing?], [host
list]]`(ssh.rs:724-744);改成
`column![home_panel_head(...), [host list], [editing?], add_button]`——
列表在上、编辑表单(如果打开)在列表下方(编辑已有主机时,表单显示在
被编辑的那张卡片下方还是整个列表下方,写计划阶段按实现简便性决定,草图
只画了"新建"场景,没有区分),"+添加"固定在最下面。

### 2. `SshTabKind`:SSH 面板自有 tab 的种类区分

新枚举,阶段 4 只用到 `Terminal` 变体,阶段 3 加 `Sftp`:

```rust
// extensions/ssh.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SshTabKind {
    Terminal,
    Sftp,
}
```

`(host_id, SshTabKind)` 是一个 SSH 面板 tab 的身份——同一台主机可以同时
开一个终端 tab 和(阶段 3 起)一个 SFTP tab,两者独立存在、独立连接(阶段
3 决定"不复用连接",见其设计文档)。

### 3. SSH 面板自有的 tab 集合:不进 `Workspace.tabs`

**关键决定**:阶段 2 把 SSH 终端塞进了与本地/agent 会话共享的
`Workspace.tabs: Vec<SessionTab>` + `Workspace.active: usize`(见
`workspace.rs:291,293`),这是"跳到右侧共享终端条"这个问题的根源——只要
SSH tab 还在这个共享集合里,任何指向它的 `on_tab_attached`/`resize_all`/
`send_input` 都天然是"面向共享条"的语义。阶段 4 把 SSH 终端 tab **搬出**
这个共享集合,单独开一个平行的集合:

```rust
// workspace.rs, Workspace 结构体新增字段
pub(crate) ssh_tabs: Vec<SessionTab>,
/// SSH 面板里"当前显示哪个 tab"，`None` = 没有任何 SSH tab 打开(面板
/// 显示空态)。不是下标而是 `(host_id, SshTabKind)`——SSH tab 会被用户
/// 手动关闭导致下标漂移，用身份而不是位置寻址更省一次"关 tab 后重新
/// 计算 active 下标"的分支。
pub(crate) ssh_active: Option<(String, ssh::SshTabKind)>,
```

`SessionTab`(`workspace.rs:208-241`)结构体本身不变,继续复用——它已经
是"协议无关的字节流查看器"(`model: TerminalModel` + `backend:
TabBackend`),不需要为了区分"共享条 tab"和"SSH 面板 tab"新增字段,区分
靠"它在哪个 Vec 里"。

`Workspace::spawn_ssh_tab`(`workspace.rs:1254-1409`)内部的 SSH 握手 +
russh channel 打开 + PTY 请求 + 读写泵循环(`tokio::select!` 那段,
`workspace.rs:1374-1404`)**完全不变**——这部分已经是纯粹的"字节从
russh channel 到 `Message::TermOutput`/从 `SshOut` 到 channel 写"的搬运,
跟它最终落进哪个 Vec 无关。唯一要改的是它触发的 `Message::TabAttached`
到达后,`on_tab_attached` 怎么处理:

```rust
// workspace.rs:1411-1454, on_tab_attached
pub(crate) fn on_tab_attached(
    &mut self,
    io: &ShellIo,
    tab_id: usize,
    info: SessionInfo,
    snapshot: Vec<u8>,
) {
    let Some(forwarder) = self.pending.remove(&tab_id) else { return; };
    let mut model = TerminalModel::new(io.cols, io.rows);
    let _ = model.feed(&snapshot);
    let backend = match self.ssh_out_pending.remove(&tab_id) {
        Some(out) => Some(TabBackend::Ssh { out }),
        None => None, // 本地会话
    };
    let tab = SessionTab { /* 字段同现状 */ backend: backend.unwrap_or(TabBackend::Daemon), .. };
    if let Some(TabBackend::Ssh { .. }) = Some(&tab.backend)
        && matches!(tab.backend, TabBackend::Ssh { .. })
    {
        // 落进 ssh_tabs,不是 tabs——SSH 面板自己的 tab 条负责展示。
        // host_id 从 info.id 反解(synth_session_info 已经把 id 编成
        // "ssh:{host_id}",见 ssh.rs:157,直接 strip_prefix)。
        let host_id = tab.info.id.strip_prefix("ssh:").unwrap_or(&tab.info.id).to_string();
        self.ssh_tabs.push(tab);
        self.ssh_active = Some((host_id, ssh::SshTabKind::Terminal));
    } else {
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.term_tab_first = 0;
    }
}
```

(以上是设计层面的形状说明,不是最终 diff——`if let`/`matches!` 那段写
计划阶段整理成更干净的写法,比如直接判断 `backend` 局部变量是不是
`Some`,不需要构造完 `SessionTab` 再匹配。)

### 4. 输入路由:终端消息加"目标"区分

`term_view::view(model, focused)` 本身(`term_view.rs:408-416`)已经是
"喂一个 `&TerminalModel` + 一个 focused 布尔值就能渲染"的独立函数,阶段 4
**直接复用**,不改这个函数的渲染逻辑。问题在于它内部的 `TermCanvas`
(`canvas::Program<Message, ...>` 实现,`term_view.rs:233` 起)在鼠标事件
时构造的是裸的顶层消息——`Message::TermInput(bytes)`(`app.rs:1073`)、
`Message::TermScroll(delta)`(`app.rs:1183`)、`Message::TermSelStart {
col, row, right }` / `TermSelUpdate { .. }`(`app.rs:1191-1193`)——这些
消息本身不携带"这次操作来自哪个终端实例"的信息。目前只有一份共享终端
条,`app.rs` 里处理这几个消息时统一按 `ws.tabs.get(ws.active)` 操作
(`term_input`,`app.rs:4080-4096`,`TermScroll`/`TermSelStart`/
`TermSelUpdate` 的 handler 类似,`app.rs:2993/3018/3025`)天然正确。阶段
4 让 SSH 面板也渲染一份 `term_view::view(&ssh_tab.model, ssh_term_focused)`
之后,两份终端画布可能同时在屏幕上(左边 SSH 面板 + 右边共享终端条都
显示),裸消息就分不清该写去 `ws.tabs`/`active` 还是 `ws.ssh_tabs`/
`ssh_active`。

解决方式:给这四个消息加一个目标标签:

```rust
// app.rs, Message 枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermTarget {
    /// 右侧共享终端条(现状,ws.tabs/active)。
    Shared,
    /// SSH 面板自己的内嵌终端(ws.ssh_tabs/ssh_active)。
    SshPanel,
}

TermInput(TermTarget, Vec<u8>),
TermScroll(TermTarget, i32),
TermSelStart { target: TermTarget, col: usize, row: usize, right: bool },
TermSelUpdate { target: TermTarget, col: usize, row: usize, right: bool },
```

`term_view::view()` 签名加一个 `target: TermTarget` 参数,内部构造这四种
消息时把 `target` 原样带上(`TermCanvas` 结构体加一个 `target` 字段,
`Program::update` 里用它)。`active_tab_view()`(`app.rs:6924-6939`,渲染
共享终端条当前 active tab)调用时传 `TermTarget::Shared`;阶段 4 新增的
SSH 面板终端渲染函数(见下方第 5 节)调用时传 `TermTarget::SshPanel`。

各 handler 按 `target` 分支到对应集合:

```rust
fn term_input(&mut self, target: TermTarget, bytes: Vec<u8>) {
    match target {
        TermTarget::Shared => {
            if !self.terminal_visible() { return; }
            self.with_focused_project(|ws, io| {
                if let Some(tab) = ws.tabs.get_mut(ws.active) {
                    tab.model.scroll_to_bottom();
                    tab.model.selection_clear();
                }
                ws.send_input(io, bytes);
            });
        }
        TermTarget::SshPanel => {
            if !self.ssh_terminal_visible() { return; } // 新增,见下
            self.with_focused_project(|ws, io| {
                if let Some(tab) = ws.ssh_active_tab_mut() {
                    tab.model.scroll_to_bottom();
                    tab.model.selection_clear();
                }
                ws.ssh_send_input(io, bytes);
            });
        }
    }
}
```

`TermScroll`/`TermSelStart`/`TermSelUpdate` 的 handler(`app.rs:2993-3030`
附近)按同样的 `match target` 模式分叉,内部逻辑(滚动量计算/选区坐标
换算)完全照抄现有实现,只是操作对象换成 `ws.ssh_active_tab_mut()`。

`Workspace` 新增两个方法(镜像既有 `send_input`/`resize_all`):

```rust
// workspace.rs
/// SSH 面板当前显示的 tab(`ssh_active` 指向的那个),可变引用——供
/// scroll_to_bottom/selection_clear 这类"敲键盘/滚动前先处理一下当前
/// 终端状态"的场景用,镜像 `ws.tabs.get_mut(ws.active)` 的既有用法。
pub(crate) fn ssh_active_tab_mut(&mut self) -> Option<&mut SessionTab> {
    let (host_id, kind) = self.ssh_active.as_ref()?;
    self.ssh_tabs.iter_mut().find(|t| {
        t.info.id.strip_prefix("ssh:") == Some(host_id.as_str())
            && *kind == ssh::SshTabKind::Terminal // 阶段 4 只有这一种
    })
}

/// 把字节写进 SSH 面板当前显示的 tab。镜像 `send_input`,操作对象换成
/// `ssh_tabs`/`ssh_active`。
pub(crate) fn ssh_send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
    let Some((host_id, _kind)) = self.ssh_active.as_ref() else { return; };
    let Some(tab) = self.ssh_tabs.iter().find(|t| {
        t.info.id.strip_prefix("ssh:") == Some(host_id.as_str())
    }) else { return; };
    if !tab.alive { return; }
    match &tab.backend {
        TabBackend::Daemon => unreachable!("ssh_tabs 里只会有 TabBackend::Ssh"),
        TabBackend::Ssh { out } => { let _ = out.send(SshOut::Data(bytes)); }
    }
}
```

`term_output()`(`app.rs:4098-4125`,处理 `Message::TermOutput(project_id,
tab_id, bytes)`)目前用 `ws.tab_by_id_mut(tab_id)` 定位 tab——这个查找
必须扩展到同时查 `ws.tabs` 和 `ws.ssh_tabs`(`tab_id` 是全局唯一递增
计数器 `next_tab_id` 分配的,`workspace.rs:294`,两个集合共用同一个计数
器、不会撞号,只是查找函数要检查两个 Vec)。`dispatch_todo_to_existing`
(`workspace.rs:805-827`)按 `session_id` 而不是 `tab_id` 查找,同理需要
扩展到同时查两个集合——Todo 面板派发任务给一个 SSH tab 时,不应该因为
这个 tab 现在在 `ssh_tabs` 里就失效。

### 5. `LeftView::Ssh` 渲染:sidebar + 终端内嵌区两栏

镜像 `LeftView::Files`/`LeftView::Project` 已有的"侧栏 + 预览区"两栏
模式(`app.rs:6027-6067`/`6099-6129`,`row![sidebar, divider,
content_pane]`)。新增一个渲染函数(放 `workspace.rs` 或 `app.rs`,不放进
`ssh.rs`——它需要顶层 `Message` 类型,不是 `ssh::Message`,跟
`preview_pane`/`project_preview_pane` 同样的理由):

```rust
fn ssh_terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 顶部 tab 条:遍历 ws.ssh_tabs,每个渲染一个 tab_core 包出来的可关闭
    // tab(prefix = 终端/传输图标 + 主机昵称,取自 SshHost.name,通过
    // host_id 反查 ws.ssh.hosts()),选中态对齐 ssh_active。
    // 下方内容区:ssh_active 指向的 tab 是 Terminal 种类 → term_view::
    // view(&tab.model, ssh_term_focused).map(...)(target 传 SshPanel);
    // 没有任何 SSH tab 打开 → 空态文案("点主机卡片的终端/传输图标开始")。
}
```

`app.rs` 里 `LeftView::Ssh` 的渲染分支(原本只调用
`ssh::view(&ws.ssh, width, border).map(Message::Ssh)`)改成:

```rust
let (list_portion, content_portion) = split_portions(app.dims.ssh_split); // 新增一个面板分屏比例状态,镜像 project_split/files_split
row![
    ssh::view(&ws.ssh, Length::FillPortion(list_portion), zone_pane_border(zone, lc)).map(Message::Ssh),
    divider_bar(Divider::SshSplit, ...),
    ssh_terminal_pane(app, ws, Length::FillPortion(content_portion), zone_pane_border(zone, rc)),
]
```

### 6. 可见性门控:`ssh_terminal_visible()`

镜像既有 `terminal_visible()`(`app.rs:969-973`,门控右侧共享终端条是否
真的在屏幕上、从而决定键盘要不要写进去):

```rust
pub(crate) fn ssh_terminal_visible(state: &ShellState) -> bool {
    state.left_view == LeftView::Ssh && state.maximized != Some(MaximizedPane::Right)
}
```

纯 `&ShellState` 函数,不需要额外传 `&Workspace`——跟既有
`terminal_visible()` 一样,只判定"这块面板此刻有没有被别的东西遮住/
切走",不判定"有没有真的打开一个 tab"(没打开 tab 时,`ws.
ssh_active_tab_mut()` 天然返回 `None`,写入操作静默跳过,不需要在可见性
门控这一层重复判断)。`MaximizedPane::Right` 对应"SSH 面板所在的左侧被
放大态遮住"——两侧角色与既有 `terminal_visible()` 的 `MaximizedPane::
Left` 判断正好对调(终端在右、被"左侧放大"遮住;SSH 面板在左、被"右侧
放大"遮住)。

### 7. 焦点:复用既有 `active_zone`,不新增焦点状态

写计划阶段核实发现 `App` 现有 `term_focused: bool`(`app.rs:1377`,初值
`true`,`app.rs:1712`)其实只在一处被读取(`app.rs:6929`,传给
`term_view::view()` 控制光标是实心还是描边),从未在别处被写过——它不是
真正驱动"键盘写给谁"的开关,只是一个视觉细节,恒为 `true`。真正已经在
驱动"用户此刻在跟左边还是右边交互"的是 `App.active_zone: Option
<ZoneSide>`(`app.rs:1425`,默认 `Some(ZoneSide::Right)`,`app.rs:1725`)
——点击左/右面板区任意位置就会经 `App::set_active_zone`(`app.rs:2355-
2357`)更新它,现状已经用来驱动 `left_zone`/`right_zone` 外边框的金色
高亮(`app.rs:6171/6314`)。这正是"两个终端画布同时在屏幕上时,键盘该
写给哪一个"所需要的信号,不需要新增专门的焦点字段/新的点击接线。

阶段 4 新增一个纯函数(不新增 `App` 字段):

```rust
pub(crate) fn keyboard_term_target(app: &App) -> TermTarget {
    if app.left_view == LeftView::Ssh && app.active_zone == Some(ZoneSide::Left) {
        TermTarget::SshPanel
    } else {
        TermTarget::Shared
    }
}
```

`main.rs` 构造 `Message::TermInput`/`TermPaste` 之前调这个函数决定
`target`(取代现状"无脑发 `TermInput(bytes)`,可见性闸门留给 `app.rs`
内部 `term_input()` 判断"的写法——闸门本身还在,只是现在要先决定往哪个
闸门送)。`active_tab_view()`(`app.rs:6924-6939`,渲染右侧共享终端条)
调用 `term_view::view()` 时,`focused` 参数改传
`keyboard_term_target(app) == TermTarget::Shared`(取代现在恒为 `true`
的 `app.term_focused`);阶段 4 新增的 SSH 面板终端渲染函数同理传
`keyboard_term_target(app) == TermTarget::SshPanel`——两处渲染用同一个
判定函数,光标视觉状态和键盘实际路由天然一致,不会出现"看着像聚焦但
键盘写去了另一边"的不一致。`term_focused` 字段本身在这次改动里可以
删除(它现在完全被 `keyboard_term_target` 取代;写计划阶段确认没有
其它读取点后删除,减少一个恒为真、容易让人误以为在生效的死状态)。

### 8. PTY 网格尺寸:v1 复用共享全局网格,不做独立像素测量

写计划阶段核实 `terminal_pane_pixel_size`(`app.rs:1009-1045`)后发现:
右侧终端 pane 的像素尺寸换算本身相当复杂(要扣 chrome/状态栏/margin,
还要区分放大态,函数注释里带着"Fix round 2/3"级别的历史踩坑记录),要
给 SSH 面板内嵌终端区镜像一份同等精度的独立测量+独立 `PaneResized`
事件链路,是这个中型改造里最重的一块,且与本文档的核心目标(终端不再
跳走,渲染在 SSH 面板自己的 tab 条里)相比,收益(网格像素级贴合)不成
比例。

**v1 决定**:SSH 面板内嵌终端复用与右侧共享终端条**同一个**全局
`(cols, rows)`(`App.cols`/`App.rows`,由 `pane_resized()` 统一维护,
`app.rs:4170-4187`)——`Workspace::resize_all` 目前已经对 `self.tabs`
里所有 tab(含不可见的)套用这同一份网格,阶段 4 只需要让 `ssh_tabs`
里的 tab **也**吃到同一次 `resize_all` 调用(把 `resize_all` 的遍历
范围从"只有 `self.tabs`"扩展成"`self.tabs` 和 `self.ssh_tabs` 都过一遍",
不新增独立的 `resize_ssh_tabs`/独立测量函数/独立 `PaneResized` 事件)。
代价:SSH 面板内嵌终端区域如果实际像素宽高与右侧终端 pane 明显不同,
字符网格可能不是像素级贴满(多余留白或轻微溢出,由 `Canvas` 的
`Length::Fill` 兜底,不会崩溃或裁切出乱码)。这是一个已知的、有意接受
的 v1 局限,不是缺陷——独立像素级测量作为后续可选的打磨项,不在本阶段
范围内。

### 9. `Message` 变更汇总

```rust
// extensions/ssh.rs::Message
- OpenTerminal(String),
+ /// 点主机卡片的终端/传输图标:内核拦截(同现有 OpenTerminal 的拦截
+ /// 方式),真正逻辑在 Workspace::open_ssh_tab(host_id, kind)。
+ OpenSshTab(String, SshTabKind),
+ /// 关闭 SSH 面板某个 tab(点 tab 条的 ✕)。内核拦截,调
+ /// Workspace::close_ssh_tab。
+ CloseSshTab(String, SshTabKind),
+ /// 切换 SSH 面板当前显示哪个 tab(点 tab 条非当前 tab)。内核拦截
+ /// (需要 &mut Workspace 设 ssh_active),同 OpenSshTab 的拦截理由。
+ SelectSshTab(String, SshTabKind),
```

`TerminalConnectFailed` 的第三个参数(`_tab_id: usize`)不变——`tab_id`
依旧是全局计数器分配的,不受"落进哪个 Vec"影响。

`app.rs` 里原来拦截 `Message::Ssh(ssh::Message::OpenTerminal(host_id))`
的分支(`app.rs:3345-3350`)改成拦截 `OpenSshTab`/`CloseSshTab`/
`SelectSshTab` 三条,`OpenSshTab` 分支里按 `kind` 分派(阶段 4 只有
`Terminal` 会真的调 `spawn_ssh_tab`,`Sftp` 变体阶段 4 先不处理消息内容
——写计划阶段决定是直接不渲染传输按钮,还是渲染但 `Sftp` 分支先
no-op,两种都不会崩,倾向前者见"目标"第 5 条)。

## 共享组件评估

- `icons::icon_button_entry`:主机卡片的 3 个动作图标(纯图标、无文字、
  固定尺寸)与它的设计形状完全匹配,**直接采用**——这与 Todo 面板重构
  评估后"不采用"的结论相反,因为 Todo 的按钮要塞文字+计数,SSH 卡片的
  按钮不用。
- `tabs::tab_core`:SSH 面板新增的 tab 条是**可关闭** tab(草图暗示,且
  与右侧共享终端条现有的可关闭语义一致),`tab_core` 本来就是为这种
  形状设计的(现有共享终端条的 `tab_item`/`panel_tab` 已经在用它,见
  `app.rs:6695` 附近),**直接采用**,不需要另起一套。
- "+添加主机"按钮:纯图标+文字,不是纯图标方块,不套 `icon_button_entry`
  (同 Todo 面板"派发"按钮的既有排除理由),继续手写。

## 错误处理

- SSH 握手/连接失败(`TerminalConnectFailed`)的落地展示不变——阶段 4
  只改它触发的 tab 最终进 `ssh_tabs` 而不是 `tabs`,错误消息本身仍然
  落进 `ws_state.test_status`,在主机卡片(现在挪到编辑表单)展示。
- `ssh_active`/`ssh_tabs` 不一致(比如 `ssh_active` 指向的 `(host_id,
  kind)` 在 `ssh_tabs` 里已经找不到——tab 被关闭但 `ssh_active` 没同步
  清空):`ssh_active_tab_mut()`/`ssh_send_input` 找不到就直接返回,UI
  侧空态兜底展示"没有打开的 tab",不 panic。`CloseSshTab` 处理逻辑里
  必须在移除 tab 后同步清空/切换 `ssh_active`(如果关的正好是当前显示
  的那个,切到 tab 条里的下一个,没有就置 `None`)——具体规则镜像现有
  关闭共享终端条 tab 时"关的是 active 就往前挪一个"的既有逻辑,写计划
  阶段核实那段代码的具体位置。
- `TermTarget` 路由错误(理论上不会发生,因为 `term_view::view()` 内部
  把 `target` 焊死在它构造的消息里,调用方拼错了传参才会出问题)——不
  需要运行时兜底,类型系统 + code review 保证正确性。

## 测试策略

- `ssh::Message`/`update()` 新逻辑(`OpenSshTab`/`CloseSshTab`/
  `SelectSshTab` 在 `ssh::update` 里应该都是 `unreachable!()`,同现有
  `OpenTerminal` 的既有处理——内核在到达之前拦截)的穷尽匹配靠编译器
  保证,不需要专门单测。
- `Workspace::on_tab_attached` 分流到 `tabs` vs `ssh_tabs` 的判断逻辑:
  新增单测覆盖"`ssh_out_pending` 有对应条目 → 落进 `ssh_tabs` 且设
  `ssh_active`"和"没有 → 落进 `tabs` 且设 `active`"两条路径(镜像现有
  测试对 `TabBackend::Ssh`/`Daemon` 分支的验证方式,如果已经有类似测试
  就扩展它,没有就新写)。
- `Workspace::ssh_send_input`/`ssh_active_tab_mut` 的查找逻辑:单测覆盖
  "`ssh_active` 为 `None`"、"`ssh_active` 指向的 tab 已被移除"、"正常
  找到"三种情形。
- `host_form`/`host_card` 视觉重写没有对应的自动化单测(渲染函数,同
  Todo 面板重构的既有口径),人工 GUI 验收覆盖。
- 人工 GUI 验收清单(写进实现计划):主机卡片显示 3 个图标按钮且配色
  是金色主题;编辑表单字段顺序、真正的圆形单选、删除/保存/取消三按钮、
  测试连接按钮都在;"+添加主机"在侧栏最下面;点终端图标在 SSH 面板
  自己的 tab 条开一个新 tab,焦点不跳到右侧;同时把右侧切到 Agent 视图、
  左侧切到 SSH 视图,两个终端能各自独立接收键盘输入、互不干扰;关闭
  SSH 面板的 tab 后 `ssh_active` 正确切换或清空;拖动窗口大小,SSH 面板
  内嵌终端和右侧共享终端条的字符网格同步跟着变(v1 共用同一份全局
  `(cols, rows)`,见"架构与数据流"第 8 节)。

## 依赖变更

新增一个 Lucide 图标 svg 资源(文件传输/sync 语义,具体文件名写计划阶段
定)+ 对应 `IconKind` 变体,无新增 crate 依赖(阶段 3 的 `russh-sftp` 见
其自己的设计文档)。
