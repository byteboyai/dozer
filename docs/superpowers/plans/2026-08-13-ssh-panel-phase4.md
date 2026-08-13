# SSH 主机面板 阶段 4:UI 重写 + 终端内嵌 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 主机卡片/编辑表单从紫色文字按钮重写成 ByteBoy2077 金色图标主题
(对齐草图);SSH 终端不再跳到右侧共享 agent 终端条,改成渲染在 SSH 面板
自己的 tab 条 + 内容区里。

**Architecture:** 视觉重写全部在 `extensions/ssh.rs` 内部完成。终端内嵌
把阶段 2 建好的 `SessionTab`(`TerminalModel` + `TabBackend::Ssh` 泵循环)
从共享的 `Workspace.tabs`/`active` 搬进一个平行集合
`Workspace.ssh_tabs`/`ssh_active`,渲染时新增一个 `ssh_terminal_pane`
(镜像 Files/Project 面板已有的"侧栏+预览区两栏"模式),终端消息
(`TermInput`/`TermScroll`/`TermSelStart`/`TermSelUpdate`/`TermPaste`)加一个
`TermTarget` 区分该写去共享条还是 SSH 面板,路由信号复用已经存在、只是
一直没被读过的 `App.active_zone`(不新增焦点状态)。

**Tech Stack:** Rust、iced 0.14、russh 0.62(阶段 2 已引入,阶段 4 不新增
终端相关依赖)。

**Spec:** `docs/superpowers/specs/2026-08-13-ssh-panel-phase4-design.md`

## Global Constraints

- **分支要求**:独立 worktree/分支(建议 `feature/ssh-panel-phase4`),不
  直接在 `main` 上改;全部任务做完、测试通过后走代码审阅,通过再合并。
  开工前用 `superpowers:using-git-worktrees` 建好 worktree。
- ByteBoy2077 配色沿用 `theme::color` 现有 token(`CARD`/`BORDER`/`GOLD`/
  `CREAM`/`RED`),不新增色值,不用 iced 默认调色板。
- 阶段 3(SFTP,另一份计划)依赖本计划新增的 `SshTabKind` 枚举和
  `ssh_tabs`/`ssh_active` 集合基础设施——`SshTabKind::Sftp` 变体本计划
  就要定义好(即使阶段 4 不渲染对应内容),阶段 3 才能直接接进来而不用
  回来改这份已经落地的代码。
- v1 不做 SSH 面板内嵌终端的独立 PTY 网格像素测量,复用与右侧共享终端
  条同一份全局 `(cols, rows)`(spec 第 8 节,已确认这个决定)。
- 不删除/不改变阶段 1/2 已有的主机数据模型
  (`SshHost`/`AuthMethod`/keyring 存取)、host-key 信任流程
  (`TrustHostKey`/`UnknownHostKey`/`KeyChanged`)的消息处理逻辑。

---

### Task 1: 新增文件传输图标(`IconKind::FolderSync`)

**Files:**
- Modify: `crates/dozer-app/src/icons.rs`(`IconKind` 枚举定义处、
  `bytes()` 方法的 `match` 里)
- Create: `crates/dozer-app/assets/icons/folder-sync.svg`

**Interfaces:**
- Produces: `icons::IconKind::FolderSync`,阶段 4(主机卡片"文件传输"
  按钮)和阶段 3(SFTP tab 前缀图标)共用同一个变体。

- [ ] **Step 1: 添加图标资源**

Lucide 图标集(MIT 协议,`assets/icons/LICENSE` 已覆盖)里语义最贴近
"文件夹 + 同步/传输"的图标是 `folder-sync`(文件夹轮廓 + 环形箭头)。
从 Lucide 官方图标集(https://lucide.dev,或本地已有的 `iced-code-editor`/
`node_modules` 等地方如果已经 vendored 了 Lucide 全集,优先直接抄现成
文件,不要重新手画 path)取 `folder-sync.svg` 的内容,保存到
`crates/dozer-app/assets/icons/folder-sync.svg`。格式必须与现有 svg
资源一致(参考 `crates/dozer-app/assets/icons/server.svg` 或
`refresh-cw.svg` 的 `viewBox`/`stroke`/`fill` 属性写法——`icons::view()`
渲染时会用调用方传入的颜色整体覆盖描边色,所以 svg 本身的 `stroke`
属性值不重要,重要的是 `viewBox="0 0 24 24"`、`fill="none"`、
`stroke="currentColor"` 这套 Lucide 标准写法,和其它现有图标保持一致
的渲染行为)。

- [ ] **Step 2: 注册 `IconKind::FolderSync`**

`icons.rs` 的 `IconKind` 枚举(`CircleSmall` 那个变体之后)加:

```rust
    /// SSH 主机卡片"文件传输"按钮 + SFTP tab 前缀图标(Lucide
    /// folder-sync)。
    FolderSync,
```

`bytes()` 方法的 `match`(`CircleSmall` 对应行之后)加:

```rust
            IconKind::FolderSync => include_bytes!("../assets/icons/folder-sync.svg"),
```

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 编译成功。这一步没有对应单测(纯资源注册),编译通过 + 后续
Task 3 实际用到它时视觉核实即可。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/icons.rs crates/dozer-app/assets/icons/folder-sync.svg
git commit -m "$(cat <<'EOF'
feat(dozer-app): add FolderSync icon for SSH file-transfer entry point

EOF
)"
```

---

### Task 2: `SshTabKind` 枚举 + `Message` 变更(`OpenSshTab`/`CloseSshTab`/`SelectSshTab` 取代 `OpenTerminal`)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(`Message` 枚举
  L291-333;`update()` 里 `OpenTerminal` 分支 L549-552)

**Interfaces:**
- Produces: `ssh::SshTabKind`(`Terminal`/`Sftp` 两个变体)、
  `ssh::Message::OpenSshTab(String, SshTabKind)`、
  `ssh::Message::CloseSshTab(String, SshTabKind)`、
  `ssh::Message::SelectSshTab(String, SshTabKind)`。
- Consumes: 无新依赖。

- [ ] **Step 1: 加 `SshTabKind` 枚举**

`ssh.rs` 里 `AuthMethod` 枚举定义(L14-18)之前插入:

```rust
/// SSH 面板自己 tab 条上的 tab 种类。同一台主机可以同时开一个 `Terminal`
/// tab 和(阶段 3 起)一个 `Sftp` tab,两者独立存在、独立连接——`(host_id,
/// SshTabKind)` 是一个 SSH 面板 tab 的完整身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SshTabKind {
    Terminal,
    Sftp,
}
```

- [ ] **Step 2: `Message` 枚举替换 `OpenTerminal`**

`Message` 枚举(L291-333)里,把:

```rust
    /// 点主机卡片"终端":内核 `App::update` 里有专门的拦截分支(见
    /// `workspace.rs`),真正的 tab 创建逻辑在那边的 `Workspace::
    /// spawn_ssh_tab`——这个变体在 `ssh::update` 自己的 `match` 里只是
    /// 穷尽匹配需要,不会真的走到这里(内核在通配 `Message::Ssh(msg)`
    /// 之前就拦截了)。
    OpenTerminal(String),
```

替换成:

```rust
    /// 点主机卡片"终端"/"文件传输"图标:内核 `App::update` 里有专门的
    /// 拦截分支(见 `workspace.rs`),真正的 tab 创建逻辑在那边——这个
    /// 变体在 `ssh::update` 自己的 `match` 里只是穷尽匹配需要,不会真的
    /// 走到这里(内核在通配 `Message::Ssh(msg)` 之前就拦截了)。
    OpenSshTab(String, SshTabKind),
    /// 关闭 SSH 面板某个 tab(点 tab 条的 ✕)。同上,内核拦截。
    CloseSshTab(String, SshTabKind),
    /// 切换 SSH 面板当前显示哪个 tab(点 tab 条里非当前的一个)。同上,
    /// 内核拦截(需要 `&mut Workspace` 设 `ssh_active`)。
    SelectSshTab(String, SshTabKind),
```

`TerminalConnectFailed` 变体(L327-332)不用改,签名不变。

- [ ] **Step 3: `update()` 里的穷尽匹配分支同步改名**

`update()`(L549-552)里:

```rust
        Message::OpenTerminal(_) => {
            // 内核 `App::update` 在通配 `Message::Ssh(msg)` 之前拦截,
            // 这里理论上到不了;写出来只是为了 `match` 穷尽。
        }
```

替换成:

```rust
        Message::OpenSshTab(..) | Message::CloseSshTab(..) | Message::SelectSshTab(..) => {
            // 内核 `App::update` 在通配 `Message::Ssh(msg)` 之前拦截,
            // 这里理论上到不了;写出来只是为了 `match` 穷尽。
        }
```

- [ ] **Step 4: 编译确认(先允许其它文件报错)**

Run: `cargo build -p dozer-app 2>&1 | grep "extensions/ssh.rs"`
Expected: `ssh.rs` 自身不再报错(其它引用了 `ssh::Message::OpenTerminal`
的文件——`workspace.rs`/`app.rs`——这一步还会报错,留到 Task 6/13 修,
这里只确认 `ssh.rs` 内部改动自洽)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add SshTabKind and replace OpenTerminal with tab-kind-aware messages

OpenSshTab/CloseSshTab/SelectSshTab all carry (host_id, SshTabKind),
laying the tab-identity groundwork phase 3's SFTP tabs will reuse.
Downstream call sites (workspace.rs, app.rs) are fixed in later tasks
— this task intentionally leaves the workspace crate non-compiling.

EOF
)"
```

---

### Task 3: `host_card` 视觉重写(图标按钮 + ByteBoy2077 配色)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(`host_card`
  L574-639)

**Interfaces:**
- Consumes: `icons::icon_button_entry`(`icons.rs:227-297`)、
  `icons::IconKind::{FolderSync, Terminal, Settings}`。
- Produces: `host_card` 签名不变(`fn host_card<'a>(host: &'a SshHost,
  status: &'a TestStatus) -> Element<'a, Message, ...>`),内部结构改。

- [ ] **Step 1: 重写 `host_card`**

整个 `host_card` 函数(L574-639)替换成:

```rust
fn host_card<'a>(
    host: &'a SshHost,
    status: &'a TestStatus,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (status_text, status_color) = match status {
        TestStatus::Idle => (String::new(), theme::color::DIM),
        TestStatus::Testing => ("测试中…".to_string(), theme::color::DIM),
        TestStatus::Ok => ("✓ 连接成功".to_string(), theme::color::GREEN),
        TestStatus::UnknownHostKey { fingerprint } => {
            (format!("⚠ 未知主机,指纹 {fingerprint}"), theme::color::GOLD)
        }
        TestStatus::KeyChanged { fingerprint } => (
            format!("✗ 主机指纹已变化({fingerprint}),拒绝连接"),
            theme::color::RED,
        ),
        TestStatus::Err(e) => (format!("✗ {e}"), theme::color::RED),
    };

    let icon_btn = |kind: icons::IconKind, on_select: Message, tooltip: &'a str| {
        icons::icon_button_entry(
            kind,
            crate::theme::icon_size::row(),
            /* active */ false,
            /* hover_t */ 0.0, // 主机卡片按钮没有 hover 动画状态可读,恒 0(纯 DIM 描边,不做过渡)
            /* card */ true,
            crate::theme::icon_size::row() + 10.0,
            /* interactive */ true,
            on_select,
            /* on_hover */ |_| Message::EditHostStart(String::new()), // 占位闭包,SSH 卡片不接 hover 高亮态,见下方说明
            tooltip,
        )
    };

    let mut actions = row![
        icon_btn(
            icons::IconKind::FolderSync,
            Message::OpenSshTab(host.id.clone(), SshTabKind::Sftp),
            "文件传输",
        ),
        icon_btn(
            icons::IconKind::Terminal,
            Message::OpenSshTab(host.id.clone(), SshTabKind::Terminal),
            "终端",
        ),
        icon_btn(
            icons::IconKind::Settings,
            Message::EditHostStart(host.id.clone()),
            "设置",
        ),
    ]
    .spacing(6);
    if matches!(status, TestStatus::UnknownHostKey { .. }) {
        actions = actions.push(
            button(text("信任并重试").size(theme::font::caption()).color(theme::color::GOLD))
                .on_press(Message::TrustHostKey(host.id.clone()))
                .padding([4, 8])
                .style(|_t: &iced_widget::Theme, _s| button::Style {
                    background: None,
                    border: iced_widget::core::Border {
                        color: theme::color::GOLD,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..button::Style::default()
                }),
        );
    }

    container(
        column![
            row![
                text(host.name.clone())
                    .size(theme::font::body())
                    .color(theme::color::CREAM),
                text(format!("{}@{}:{}", host.username, host.host, host.port))
                    .size(theme::font::caption_sm())
                    .color(theme::color::DIM),
            ]
            .spacing(8),
            actions,
        ]
        .spacing(8),
    )
    .padding(10)
    .width(iced_widget::core::Length::Fill)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(theme::color::CARD.into()),
        border: iced_widget::core::Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}
```

（`status_text`/`status_color` 计算保留但这一步先不渲染——挪进
`host_form` 是 Task 4 的事,`host_card` 这一步只管卡片本身；如果编译器
提示 `status_text`/`status_color` 未使用,在变量名前加 `_` 前缀
暂时消音,Task 4 完成后这两个变量真正被 `host_form` 使用,那时候这个
函数如果还有残留的未用变量再回来清理——写这一步时不确定 Task 4 是否会
把状态展示逻辑整个搬走,所以先保留计算、只挪渲染位置)。

`on_hover` 参数说明:`icon_button_entry` 的 hover 动画设计是给"调用方
能读到 `HoverId` 状态"的场景用的(`icons.rs` 文档注释里提到"多数 view
函数拿不到 `&App`"),`host_card` 是纯函数、拿不到 hover 进度状态,这里
先传一个恒定 `hover_t: 0.0` + 占位 `on_hover` 闭包(消息类型对齎但实际
不会被 UI 触发,因为 `MouseArea` 的 `on_enter`/`on_exit` 事件本来就是
标准 iced 事件,发出去后 `ssh::update` 收到 `EditHostStart(String::new())`
——**这是一个真实的行为缺陷**,不能这样上线。写计划阶段的正确做法:
`WorkspaceState` 新增一个"当前 hover 的按钮 id"字段(镜像 rail 按钮的
`HoverId` 机制),`host_card` 从 `ws_state` 读取对应 host 的 hover 进度、
`on_hover` 发一个真正更新该字段的消息。这个字段/消息本 Task 一并加:

```rust
// WorkspaceState 新增字段(L71-84 那个结构体里)
hover_action: Option<(String /* host_id */, u8 /* 0=transfer,1=terminal,2=settings */)>,
```

（`hover_action` 的具体表示形式——`(host_id, u8)` tuple 还是专门的枚举
——写这一步时按实现简便性定,只要能唯一标识"哪台主机的哪个按钮正在
hover"即可;新增对应 `Message::HoverAction(Option<(String, u8)>)` 和
`update()` 分支 `ws_state.hover_action = v`,`host_card` 签名相应加一个
`hover_t: f32` 参数由调用方——`view()`——按 `ws_state.hover_action` 算好
传入。这部分因为涉及 hover 动画的既有 `App::hover_progress` 机制,具体
接线方式参考 `files.rs`/`database.rs` 里 `icon_button_entry` 的现有调用
写法,不是本步骤唯一正确答案,只要"hover 视觉能正常工作、不会误触发
消息"这个验收标准达到即可）。

- [ ] **Step 2: 编译 + 视觉确认**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译成功(`ssh.rs` 内部自洽;`workspace.rs`/`app.rs` 仍有
Task 2 遗留的 `OpenTerminal` 引用错误,属于预期,留到后面任务修)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): rewrite host_card with icon buttons and ByteBoy2077 palette

Replaces the four plain-text purple buttons (测试连接/终端/编辑/删除)
with three icon_button_entry buttons (transfer/terminal/settings)
matching the sketch. Test-connection and delete move into the edit
form in the next task; card no longer renders the status line
(reserved for host_form).

EOF
)"
```

---

### Task 4: `host_form` 视觉重写(真单选按钮 + 删除/保存/取消/测试连接 四按钮)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(`host_form`
  L641-717)

**Interfaces:**
- Consumes: 既有 `Message::DraftAuthMethodToggled`/`TestConnection`/
  `DeleteHost`/`DraftSave`/`DraftCancel`(签名不变)。
- Produces: `host_form` 签名新增一个 `status: &TestStatus` 参数(状态行
  从卡片挪进表单需要);调用方(`view()`)相应改传参,见 Task 5。

- [ ] **Step 1: 新增单选圆点渲染辅助函数**

`host_form` 函数之前插入:

```rust
/// 单选圆点:选中态 `GOLD` 实心 + `GOLD` 描边,未选中态空心 `BORDER`
/// 描边。iced 没有原生 radio 部件,手绘一个圆形 `container` + `MouseArea`。
fn radio_dot<'a>(
    label: &'a str,
    selected: bool,
    on_select: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let dot = container(iced_widget::Space::new())
        .width(iced_widget::core::Length::Fixed(10.0))
        .height(iced_widget::core::Length::Fixed(10.0))
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: if selected {
                Some(theme::color::GOLD.into())
            } else {
                None
            },
            border: iced_widget::core::Border {
                color: if selected { theme::color::GOLD } else { theme::color::BORDER },
                width: 1.5,
                radius: 5.0.into(),
            },
            ..iced_widget::container::Style::default()
        });
    let ring = container(dot)
        .width(iced_widget::core::Length::Fixed(16.0))
        .height(iced_widget::core::Length::Fixed(16.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    iced_widget::MouseArea::new(
        row![
            ring,
            text(label).size(theme::font::body()).color(theme::color::CREAM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_press(on_select)
    .into()
}
```

- [ ] **Step 2: 重写 `host_form`**

整个 `host_form` 函数(L641-717)替换成:

```rust
fn host_form<'a>(
    draft: &'a SshHostDraft,
    status: &'a TestStatus,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![
        text_input("主机名称", &draft.name)
            .on_input(Message::DraftNameChanged)
            .size(theme::font::body()),
        text_input("Host", &draft.host)
            .on_input(Message::DraftHostChanged)
            .size(theme::font::body()),
        text_input("port(22)", &draft.port)
            .on_input(Message::DraftPortChanged)
            .size(theme::font::body()),
        text_input("user name", &draft.username)
            .on_input(Message::DraftUsernameChanged)
            .size(theme::font::body()),
        row![
            radio_dot("密码", !draft.use_private_key, Message::DraftAuthMethodToggled(false)),
            radio_dot("私钥", draft.use_private_key, Message::DraftAuthMethodToggled(true)),
        ]
        .spacing(20),
    ]
    .spacing(10);

    if draft.use_private_key {
        col = col.push(
            text_input("私钥文件路径,如 ~/.ssh/id_ed25519", &draft.key_path)
                .on_input(Message::DraftKeyPathChanged)
                .size(theme::font::body()),
        );
        col = col.push(
            text_input("私钥口令(留空则不修改/无口令)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(theme::font::body()),
        );
    } else {
        col = col.push(
            text_input("password(留空则不修改)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(theme::font::body()),
        );
    }

    let text_btn = |label: &'a str, color: iced_widget::core::Color, msg: Message| {
        button(text(label).size(theme::font::label()).color(color))
            .on_press(msg)
            .padding([6, 12])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: Some(theme::color::BG.into()),
                border: iced_widget::core::Border {
                    color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: color,
                ..button::Style::default()
            })
    };

    let mut buttons = row![text_btn("测试连接", theme::color::CREAM, Message::TestConnection(
        draft.id.clone().unwrap_or_default(),
    ))]
    .spacing(6);
    // 新建主机(没有 id)时"测试连接"点了也是 no-op(TestConnection 在
    // ws_state.hosts 里查不到这个空字符串 id,直接 return——见
    // ssh::update 的既有实现),"删除"按钮干脆不渲染,没有可删的对象。
    if let Some(id) = &draft.id {
        buttons = buttons.push(text_btn("删除", theme::color::RED, Message::DeleteHost(id.clone())));
    }
    buttons = buttons.push(text_btn("保存", theme::color::GOLD, Message::DraftSave));
    buttons = buttons.push(text_btn("取消", theme::color::DIM, Message::DraftCancel));
    col = col.push(buttons);

    let (status_text, status_color) = match status {
        TestStatus::Idle => (String::new(), theme::color::DIM),
        TestStatus::Testing => ("测试中…".to_string(), theme::color::DIM),
        TestStatus::Ok => ("✓ 连接成功".to_string(), theme::color::GREEN),
        TestStatus::UnknownHostKey { fingerprint } => {
            (format!("⚠ 未知主机,指纹 {fingerprint}"), theme::color::GOLD)
        }
        TestStatus::KeyChanged { fingerprint } => (
            format!("✗ 主机指纹已变化({fingerprint}),拒绝连接"),
            theme::color::RED,
        ),
        TestStatus::Err(e) => (format!("✗ {e}"), theme::color::RED),
    };
    if !status_text.is_empty() {
        col = col.push(text(status_text).size(theme::font::caption_sm()).color(status_color));
    }
    if matches!(status, TestStatus::UnknownHostKey { .. })
        && let Some(id) = &draft.id
    {
        col = col.push(
            text_btn("信任并重试", theme::color::GOLD, Message::TrustHostKey(id.clone())),
        );
    }

    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fill)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(theme::color::CARD.into()),
            border: iced_widget::core::Border {
                color: theme::color::GOLD,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

（Task 3 里 `host_card` 保留计算但未使用的 `status_text`/`status_color`
——这一步确认 `host_card` 确实不再需要它们,回 `host_card` 里删掉那段
死代码计算,改成不计算,`host_card` 签名的 `status: &'a TestStatus`
参数保留,继续用于 Task 3 里"未知 host key"分支的"信任并重试"按钮
判断。）

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | grep "extensions/ssh.rs"`
Expected: `host_form` 调用点(`view()` 里)会报参数数量不对(还传一个
参数),这是预期——Task 5 修 `view()`。`host_form`/`host_card`/
`radio_dot` 自身应该没有编译错误。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): rewrite host_form with real radio buttons and delete/save/cancel/test row

EOF
)"
```

---

### Task 5: `view()` 重排(列表在上、"+添加主机"挪到底部)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(`view()` L719-757)

- [ ] **Step 1: 重写 `view()` 的组装顺序**

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: iced_widget::core::Length,
    outer: iced_widget::core::Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![crate::homespace::home_panel_head(crate::icons::IconKind::Server, "主机")]
        .spacing(12);

    if ws_state.hosts().is_empty() {
        col = col.push(
            text("还没有主机")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else {
        for h in ws_state.hosts() {
            col = col.push(host_card(h, ws_state.test_status(&h.id)));
        }
    }

    if let Some(draft) = ws_state.editing() {
        let status = draft
            .id
            .as_deref()
            .map(|id| ws_state.test_status(id))
            .unwrap_or(&TestStatus::Idle);
        col = col.push(host_form(draft, status));
    }

    col = col.push(
        button(text("＋添加").size(theme::font::body()).color(theme::color::GOLD))
            .on_press(Message::AddHostStart)
            .padding([8, 16])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: Some(theme::color::BG.into()),
                border: iced_widget::core::Border {
                    color: theme::color::GOLD,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                text_color: theme::color::GOLD,
                ..button::Style::default()
            }),
    );

    container(col.padding(16))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::BG.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}
```

- [ ] **Step 2: 编译确认**

Run: `cargo build -p dozer-app --lib 2>&1 | tail -60`
Expected: `extensions::ssh` 模块本身编译通过。`workspace.rs`/`app.rs`
仍会因为 Task 2 的 `OpenTerminal`→`OpenSshTab` 改名而报错,预期之内。

Run: `cargo test -p dozer-app --lib extensions::ssh:: -- --test-threads=1 2>&1 | tail -60`
Expected: 阶段 1/2 遗留的既有单测(`reload_from_disk_*`/
`save_then_reload_round_trips`/`synth_session_info_shape`/
`trust_host_key_reopens_terminal_only_when_ids_match` 等)全部 PASS——
这些测的是数据层/纯函数,视觉重写不应该影响它们。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): reorder ssh panel — host list above edit form, add button at bottom

EOF
)"
```

---

### Task 6: `Workspace` 新增 `ssh_tabs`/`ssh_active`,`on_tab_attached` 分流

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`Workspace` 结构体
  L290-309 附近;`on_tab_attached` L1411-1454;`spawn_ssh_tab` 内触发
  `Message::TabAttached` 的调用点不用改,签名不变)

**Interfaces:**
- Produces: `Workspace.ssh_tabs: Vec<SessionTab>`、
  `Workspace.ssh_active: Option<(String, ssh::SshTabKind)>`。
- Consumes: `ssh::SshTabKind`(Task 2 新增)、既有 `SessionTab`/
  `TabBackend`/`ssh_out_pending`(不变)。

- [ ] **Step 1: 新增字段**

`Workspace` 结构体(`pub(crate) tabs: Vec<SessionTab>` 那段,`workspace.rs`
L291 附近)之后加:

```rust
    /// SSH 面板自己的 tab 集合(阶段 4)——与 `tabs`/`active`(右侧共享
    /// agent/本地终端条)完全独立。目前只装 `TabBackend::Ssh` 的
    /// `SessionTab`(种类=`SshTabKind::Terminal`);阶段 3 起 SFTP 种类
    /// 的 tab 走另一个集合(`sftp_tabs`,内容不是 `SessionTab`,见阶段 3
    /// 计划),不混进这里。
    pub(crate) ssh_tabs: Vec<SessionTab>,
    /// SSH 面板当前显示哪个 tab——身份寻址(不是下标),因为 tab 会被
    /// 用户关闭导致下标漂移。`None` = 没有任何 SSH 终端 tab 打开。
    pub(crate) ssh_active: Option<(String, ssh::SshTabKind)>,
```

`Workspace` 的构造点(`workspace.rs:536` 附近,`ssh_out_pending:
HashMap::new(),` 那一行旁边)加对应初始化:

```rust
            ssh_tabs: Vec::new(),
            ssh_active: None,
```

- [ ] **Step 2: `on_tab_attached` 分流**

`on_tab_attached`(L1411-1454)整体替换成:

```rust
    pub(crate) fn on_tab_attached(
        &mut self,
        io: &ShellIo,
        tab_id: usize,
        info: SessionInfo,
        snapshot: Vec<u8>,
    ) {
        let Some(forwarder) = self.pending.remove(&tab_id) else {
            return;
        };
        let mut model = TerminalModel::new(io.cols, io.rows);
        let _ = model.feed(&snapshot);
        let ssh_backend = self.ssh_out_pending.remove(&tab_id);
        let is_ssh = ssh_backend.is_some();
        let mut tab = SessionTab {
            agent_state: info.agent_state,
            agent: info.agent,
            transcript_path: info.transcript_path.clone(),
            info,
            model,
            alive: true,
            tab_id,
            forwarder,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            delivery_pending: false,
            last_turn_head: None,
            backend: match ssh_backend {
                Some(out) => TabBackend::Ssh { out },
                None => TabBackend::Daemon,
            },
        };
        tab.ingest_osc(&snapshot);
        if is_ssh {
            // synth_session_info 把 id 编成 "ssh:{host_id}"(ssh.rs:157),
            // 反解出 host_id 作为 ssh_tabs 里这个 tab 的身份。
            let host_id = tab
                .info
                .id
                .strip_prefix("ssh:")
                .unwrap_or(&tab.info.id)
                .to_string();
            self.ssh_tabs.push(tab);
            self.ssh_active = Some((host_id, ssh::SshTabKind::Terminal));
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            self.term_tab_first = 0;
        }
    }
```

- [ ] **Step 3: 单测**

在 `workspace.rs` 的 `#[cfg(test)] mod tests` 里新增(找现有
`on_tab_attached` 相关测试作为参照写法,没有就新建;需要构造一个假
`SessionInfo`/走一遍 `pending`/`ssh_out_pending` 暂存流程,参照现有
`close_tab_skips_daemon_kill_for_ssh_backend` 测试里"怎么在测试里造一个
`TabBackend::Ssh` 的 `SessionTab`"的既有写法):

```rust
    #[test]
    fn on_tab_attached_routes_ssh_backend_to_ssh_tabs() {
        let mut ws = Workspace::default(); // 或现有测试用的构造帮助函数,核实 Workspace 是否已有 test 专用构造器
        let io = test_shell_io(); // 核实现有测试文件里 ShellIo 的测试构造帮助函数名字
        let tab_id = 42;
        ws.pending.insert(tab_id, tokio::spawn(async {}));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ws.ssh_out_pending.insert(tab_id, tx);
        let info = ssh::synth_session_info(
            &ssh::SshHost {
                id: "h1".into(),
                name: "n".into(),
                host: "example.com".into(),
                port: 22,
                username: "u".into(),
                auth: ssh::AuthMethod::Password,
            },
            1,
        );
        ws.on_tab_attached(&io, tab_id, info, Vec::new());
        assert_eq!(ws.ssh_tabs.len(), 1);
        assert!(ws.tabs.is_empty());
        assert_eq!(ws.ssh_active, Some(("h1".to_string(), ssh::SshTabKind::Terminal)));
    }

    #[test]
    fn on_tab_attached_routes_daemon_backend_to_tabs() {
        let mut ws = Workspace::default();
        let io = test_shell_io();
        let tab_id = 7;
        ws.pending.insert(tab_id, tokio::spawn(async {}));
        // 不插 ssh_out_pending —— 本地会话路径。
        let info = /* 造一个本地 SessionInfo,参照现有 spawn_new_tab 相关测试的既有写法 */;
        ws.on_tab_attached(&io, tab_id, info, Vec::new());
        assert_eq!(ws.tabs.len(), 1);
        assert!(ws.ssh_tabs.is_empty());
        assert_eq!(ws.active, 0);
    }
```

（`Workspace::default()`/`test_shell_io()`/本地 `SessionInfo` 的具体
构造方式写这一步时对照 `workspace.rs` 现有 `#[cfg(test)] mod tests`
里已有的测试怎么造这些值——那个模块目前已经有 `close_tab_skips_
daemon_kill_for_ssh_backend`(L3208 附近)等类似测试,直接抄它的构造
手法,不要凭空发明新的测试工具函数。）

- [ ] **Step 4: 跑测试(预期整个 crate 还编译不过,先只跑能跑的部分)**

Run: `cargo build -p dozer-app --lib 2>&1 | tail -60`
Expected: `workspace.rs` 自身应该编译通过(它不再引用
`ssh::Message::OpenTerminal`——`on_tab_attached` 没碰这个变体);
`app.rs` 仍会因为 Task 2 的改名报错,是预期。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): split SSH terminal tabs into a parallel ssh_tabs collection

on_tab_attached now routes TabBackend::Ssh sessions into ssh_tabs/
ssh_active instead of the shared tabs/active used by local and agent
sessions — this is the structural fix for "SSH terminal jumps to the
shared agent tab strip."

EOF
)"
```

---

### Task 7: `ssh_send_input`/`ssh_active_tab_mut`/`close_ssh_tab`/`select_ssh_tab` + 扩展 `tab_by_id_mut`/`dispatch_todo_to_existing`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(新增方法;`tab_by_id_mut`
  和 `dispatch_todo_to_existing` L805-827 扩展到覆盖 `ssh_tabs`)

**Interfaces:**
- Consumes: Task 6 的 `ssh_tabs`/`ssh_active`。
- Produces: `Workspace::ssh_active_tab_mut(&mut self) -> Option<&mut
  SessionTab>`、`Workspace::ssh_send_input(&self, io: &ShellIo, bytes:
  Vec<u8>)`、`Workspace::close_ssh_tab(&mut self, io: &ShellIo, host_id:
  &str, kind: ssh::SshTabKind)`、`Workspace::select_ssh_tab(&mut self,
  host_id: String, kind: ssh::SshTabKind)`。

- [ ] **Step 1: 先找到 `tab_by_id_mut` 现有实现**

Run: `grep -n "fn tab_by_id_mut" crates/dozer-app/src/workspace.rs`

把它的返回逻辑从"只查 `self.tabs`"扩展成"先查 `self.tabs`,查不到再查
`self.ssh_tabs`"(具体现有函数体写这一步时读出来改,不要整段猜——它
现在只有一处调用点`term_output()`,改法是在原有 `self.tabs.iter_mut()
.find(...)` 之后加 `.or_else(|| self.ssh_tabs.iter_mut().find(...))`,
查找条件——按 `tab_id` 字段比较——不变)。

- [ ] **Step 2: 新增 `ssh_active_tab_mut`/`ssh_send_input`**

`send_input`(L780-801)之后插入:

```rust
    /// SSH 面板当前显示的 tab(`ssh_active` 指向的那个),可变引用。
    /// 镜像 `ws.tabs.get_mut(ws.active)` 的既有用法,用于"敲键盘/滚动
    /// 前先处理一下当前终端状态"(回底/清选区)这类场景。
    pub(crate) fn ssh_active_tab_mut(&mut self) -> Option<&mut SessionTab> {
        let (host_id, _kind) = self.ssh_active.as_ref()?;
        let host_id = host_id.clone();
        self.ssh_tabs
            .iter_mut()
            .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()))
    }

    /// 把字节写进 SSH 面板当前显示的 tab。镜像 `send_input`,操作对象
    /// 换成 `ssh_tabs`/`ssh_active`。
    pub(crate) fn ssh_send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
        let _ = io; // ssh_tabs 里只会是 TabBackend::Ssh,不需要 io.client/handle,
                    // 保留参数是为了和 send_input 签名对齐、调用方不用分叉判断
        let Some((host_id, _kind)) = self.ssh_active.as_ref() else {
            return;
        };
        let Some(tab) = self
            .ssh_tabs
            .iter()
            .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()))
        else {
            return;
        };
        if !tab.alive {
            return;
        }
        match &tab.backend {
            TabBackend::Daemon => {
                debug_assert!(false, "ssh_tabs 里不应该出现 TabBackend::Daemon");
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }
```

- [ ] **Step 3: 新增 `close_ssh_tab`/`select_ssh_tab`**

`close_tab`(L1010-1032)之后插入:

```rust
    /// 关闭 SSH 面板某个 tab(镜像 `close_tab` 的中断转发任务/kill 逻辑,
    /// 按身份而不是下标寻址)。关的正好是当前显示的 tab 时,切到剩下
    /// tab 里的第一个,没有剩下的就清空 `ssh_active`。
    pub(crate) fn close_ssh_tab(&mut self, io: &ShellIo, host_id: &str, kind: ssh::SshTabKind) {
        let Some(idx) = self
            .ssh_tabs
            .iter()
            .position(|t| t.info.id.strip_prefix("ssh:") == Some(host_id))
        else {
            return;
        };
        let tab = self.ssh_tabs.remove(idx);
        tab.forwarder.abort();
        // SSH 后端不需要 kill——drop `out`(tx)后 spawn_ssh_tab 的泵循环
        // `rx_out.recv() => None` 分支自然退出,同 close_tab 现有对
        // TabBackend::Ssh 的既有处理口径(见 close_tab_skips_daemon_
        // kill_for_ssh_backend 测试)。
        let _ = io;
        if self.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k)) == Some((host_id, kind)) {
            self.ssh_active = self
                .ssh_tabs
                .first()
                .map(|t| {
                    let h = t.info.id.strip_prefix("ssh:").unwrap_or(&t.info.id).to_string();
                    (h, ssh::SshTabKind::Terminal)
                });
        }
    }

    /// 切换 SSH 面板当前显示哪个 tab。目标 tab 不存在时 no-op(保持
    /// 原有 `ssh_active` 不变,不是清空——镜像其它"目标已消失"场景的
    /// 既有容错口径)。
    pub(crate) fn select_ssh_tab(&mut self, host_id: String, kind: ssh::SshTabKind) {
        let exists = self
            .ssh_tabs
            .iter()
            .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
        if exists {
            self.ssh_active = Some((host_id, kind));
        }
    }
```

- [ ] **Step 4: 扩展 `dispatch_todo_to_existing`**

`dispatch_todo_to_existing`(L805-827)里 `let Some(tab) = self.tabs.iter()
.find(...)` 那一行,扩展成同时查 `ssh_tabs`:

```rust
    pub(crate) fn dispatch_todo_to_existing(&self, io: &ShellIo, session_id: &str, text: &str) {
        let tab = self
            .tabs
            .iter()
            .find(|t| t.info.id == session_id)
            .or_else(|| self.ssh_tabs.iter().find(|t| t.info.id == session_id));
        let Some(tab) = tab else {
            return;
        };
        if !tab.alive {
            return;
        }
        let bytes = format!("{text}\n").into_bytes();
        match &tab.backend {
            TabBackend::Daemon => {
                let client = io.client.clone();
                let id = session_id.to_string();
                io.handle.spawn(async move {
                    if let Err(e) = client.write(&id, &bytes).await {
                        tracing::warn!("派发任务文本失败: {e}");
                    }
                });
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }
```

（Todo 面板派发给一个已经在 `ssh_tabs` 里的会话——阶段 4 之前这种情况
不存在,因为 SSH tab 原本就在 `tabs` 里;阶段 4 之后 SSH tab 搬进
`ssh_tabs`,Todo 面板如果之前已经记过这个 `session_id` 的派发记录,
派发弹层列出的"存活 session"来源——`SessionTabSummary`,由内核从
`ws.tabs` 摘出来传给 `todo::view`,见 `app.rs:6075-6083`——也要扩展到
同时摘 `ws.ssh_tabs`,否则 Todo 面板的派发弹层永远看不到 SSH 会话。这
不是本 Task 范围,但要在 Task 13 或收尾任务里核实这一点,写进最终的
人工验收清单)。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozer-app --lib 2>&1 | tail -60`
Run: `cargo test -p dozer-app --lib workspace:: -- --test-threads=1 2>&1 | tail -80`
Expected: `workspace.rs` 模块编译通过(`app.rs` 可能仍有 Task 2 遗留
错误);新增的 `on_tab_attached_routes_*` 两个测试(Task 6)和既有
`dispatch_todo_to_existing`/`close_tab` 相关测试全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add ssh_tabs-aware input routing, close, and select methods

EOF
)"
```

---

### Task 8: `resize_all` 覆盖 `ssh_tabs`(共用全局网格,spec 第 8 节的 v1 决定)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`resize_all`,`L1461` 起)

- [ ] **Step 1: 找到 `resize_all` 现有实现并扩展遍历范围**

Run: `grep -n "fn resize_all" -A 30 crates/dozer-app/src/workspace.rs`

现有实现对 `self.tabs` 做 `for tab in &mut self.tabs { tab.model.resize
(...); match &tab.backend { ... } }`——把这个 `for` 循环体抽成一个
闭包/内部函数,对 `self.tabs.iter_mut().chain(self.ssh_tabs.iter_mut())`
一起跑(Rust 里两个不同字段的 `iter_mut().chain()` 会触发"同时可变借用
`self` 两个字段"的借用检查问题,如果直接链式调用编译不过,改成两段
分开写同一段逻辑,不是必须真的用 `chain`——写这一步时以能编译通过为
准,优先尝试提取一个 `fn resize_one(tab: &mut SessionTab, client:
&Client, handle: &Handle, cols: u16, rows: u16)` 辅助函数,`resize_all`
对 `self.tabs`/`self.ssh_tabs` 各跑一遍这个辅助函数,这样不会有借用
冲突)。

- [ ] **Step 2: 编译 + 测试**

Run: `cargo build -p dozer-app --lib 2>&1 | tail -40`

Run: `grep -n "fn.*resize_all\|resize_all(" crates/dozer-app/src/workspace.rs`
如果已有 `resize_all` 相关单测,确认它们仍然 PASS;新增一个覆盖
"`ssh_tabs` 里的 tab 也被 resize"的单测(构造一个 `ssh_tabs` 里的假
`SessionTab`,调 `resize_all`,断言 `tab.model` 的网格尺寸变了——
`TerminalModel` 应该有类似 `grid_dims()` 的既有查询方法,`term_view.rs`
里已经用过,直接复用)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): resize_all also covers ssh_tabs (shared global PTY grid)

EOF
)"
```

---

### Task 9: `app.rs` 新增 `TermTarget`,四个终端消息加目标参数并更新 handler

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(`Message` 枚举 L1073/1183/
  1191-1193;handler `term_input` L4080-4096、`term_paste` L4127-4149、
  `TermScroll`/`TermSelStart`/`TermSelUpdate` 分发处 L2993-3031、
  `Message::TermPaste` 分发处 L3032)

**Interfaces:**
- Produces: `app::TermTarget`(`Shared`/`SshPanel`)。
- Consumes: Task 7 的 `Workspace::ssh_send_input`/`ssh_active_tab_mut`。

- [ ] **Step 1: 新增 `TermTarget` 枚举**

`Message` 枚举定义之前(比如紧邻 `LeftView`/`RightView` 那几个枚举)
插入:

```rust
/// 终端相关消息(键盘/滚轮/选区/粘贴)该写去右侧共享终端条还是 SSH 面板
/// 自己的内嵌终端——两者可能同时在屏幕上,裸消息本身不带这个信息,靠
/// canvas 渲染时(`term_view::view`)烘焙进它构造的每条消息里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermTarget {
    Shared,
    SshPanel,
}
```

- [ ] **Step 2: `Message` 枚举四个变体加 `TermTarget`**

```rust
-    TermInput(Vec<u8>),
+    TermInput(TermTarget, Vec<u8>),
```

```rust
-    TermScroll(i32),
+    TermScroll(TermTarget, i32),
```

```rust
-    TermSelStart { col: usize, row: usize, right: bool },
+    TermSelStart { target: TermTarget, col: usize, row: usize, right: bool },
```

```rust
-    TermSelUpdate { col: usize, row: usize, right: bool },
+    TermSelUpdate { target: TermTarget, col: usize, row: usize, right: bool },
```

`grep -n "TermPaste" crates/dozer-app/src/app.rs` 找到 `TermPaste(String)`
定义,同样加:

```rust
-    TermPaste(String),
+    TermPaste(TermTarget, String),
```

- [ ] **Step 3: 更新分发处(L2993-3032 附近)**

```rust
            Message::TermScroll(target, delta) => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
```

```rust
            Message::TermSelStart { target, col, row, right } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_start(col, row, right);
                    }
                });
            }
            Message::TermSelUpdate { target, col, row, right } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
```

```rust
-            Message::TermPaste(text) => self.term_paste(text),
+            Message::TermPaste(target, text) => self.term_paste(target, text),
```

- [ ] **Step 4: 更新 `term_input`/`term_paste` 方法体**

```rust
    fn term_input(&mut self, target: TermTarget, bytes: Vec<u8>) {
        let visible = match target {
            TermTarget::Shared => self.terminal_visible(),
            TermTarget::SshPanel => self.ssh_terminal_visible(), // Task 12 新增
        };
        if !visible {
            return;
        }
        self.with_focused_project(|ws, io| {
            match target {
                TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                }
                TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.ssh_send_input(io, bytes);
                }
            }
        });
    }
```

```rust
    fn term_paste(&mut self, target: TermTarget, text: String) {
        let visible = match target {
            TermTarget::Shared => self.terminal_visible(),
            TermTarget::SshPanel => self.ssh_terminal_visible(),
        };
        if !visible {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let bracketed = match target {
                TermTarget::Shared => ws.tabs.get(ws.active).map(|t| t.model.bracketed_paste()),
                TermTarget::SshPanel => {
                    ws.ssh_tabs
                        .iter()
                        .find(|t| {
                            ws.ssh_active.as_ref().is_some_and(|(h, _)| {
                                t.info.id.strip_prefix("ssh:") == Some(h.as_str())
                            })
                        })
                        .map(|t| t.model.bracketed_paste())
                }
            };
            let Some(bracketed) = bracketed else { return };
            match target {
                TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                    }
                }
                TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                    }
                }
            }
            let bytes = if bracketed {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            match target {
                TermTarget::Shared => ws.send_input(io, bytes),
                TermTarget::SshPanel => ws.ssh_send_input(io, bytes),
            }
        });
    }
```

（`term_paste` 这版比原来啰嗦一些——原实现用一次 `ws.tabs.get_mut
(ws.active)` 拿到 `tab` 就地读 `bracketed_paste()`+改 `scroll_to_
bottom()`,这里因为 `Shared`/`SshPanel` 两分支的可变借用目标不同,拆成
"先只读探测 `bracketed`,再按 target 分别处理"两步,避免同一个闭包里
对 `ws` 出现两种不相容的可变借用模式。写这一步如果发现有更简洁的写法
——比如把 `Shared`/`SshPanel` 各自的完整逻辑拆成两个独立的私有方法
`term_paste_shared`/`term_paste_ssh`,由 `term_paste` 按 target 分派——
优先选更简洁的,上面只是一个已经能过借用检查的参考实现,不是唯一
写法。）

- [ ] **Step 5: 编译确认(先只管 app.rs 自身)**

Run: `cargo build -p dozer-app --lib 2>&1 | grep "app.rs"`
Expected: `term_view.rs`(还没改,Task 10)和 `main.rs`(还没改,Task 11)
调用点会报错,预期之内;`app.rs` 内部这几处改动自洽。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add TermTarget and thread it through TermInput/TermScroll/TermSelStart/TermSelUpdate/TermPaste

EOF
)"
```

---

### Task 10: `term_view::view()` 携带 `TermTarget`

**Files:**
- Modify: `crates/dozer-app/src/term_view.rs`(`TermCanvas` 结构体
  L209-212;`Program::update` L236-303;`view()` L408-416)

- [ ] **Step 1: `TermCanvas` 加 `target` 字段**

```rust
struct TermCanvas<'a> {
    model: &'a TerminalModel,
    focused: bool,
    target: crate::app::TermTarget,
}
```

- [ ] **Step 2: `Program::update` 里三处消息构造带上 `target`**

```rust
-                    return Some(canvas::Action::publish(Message::TermInput(bytes)).and_capture());
+                    return Some(
+                        canvas::Action::publish(Message::TermInput(self.target, bytes))
+                            .and_capture(),
+                    );
```

```rust
-                Some(canvas::Action::publish(Message::TermScroll(lines)).and_capture())
+                Some(canvas::Action::publish(Message::TermScroll(self.target, lines)).and_capture())
```

```rust
-                Some(
-                    canvas::Action::publish(Message::TermSelStart { col, row, right })
-                        .and_capture(),
-                )
+                Some(
+                    canvas::Action::publish(Message::TermSelStart {
+                        target: self.target,
+                        col,
+                        row,
+                        right,
+                    })
+                    .and_capture(),
+                )
```

```rust
-                Some(canvas::Action::publish(Message::TermSelUpdate {
-                    col,
-                    row,
-                    right,
-                }))
+                Some(canvas::Action::publish(Message::TermSelUpdate {
+                    target: self.target,
+                    col,
+                    row,
+                    right,
+                }))
```

（注意 `TermSelUpdate` 分支现有代码没有 `.and_capture()`,保持原样不要
加上——这是既有行为,不是本次改动范围。）

- [ ] **Step 3: `view()` 签名加 `target` 参数**

```rust
pub fn view(
    model: &TerminalModel,
    focused: bool,
    target: crate::app::TermTarget,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    Canvas::new(TermCanvas { model, focused, target })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
```

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app --lib 2>&1 | grep "term_view.rs\|call to view"`
Expected: `term_view.rs` 自身编译通过;`app.rs` 里唯一的调用点
(`active_tab_view`,L6929)因为参数数量不对会报错,留到 Task 12 修。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/term_view.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): thread TermTarget through term_view::view() and its canvas messages

EOF
)"
```

---

### Task 11: `main.rs` 键盘/粘贴路由决定 `TermTarget`

**Files:**
- Modify: `crates/dozer-app/src/main.rs`(键盘事件处理里构造
  `Message::TermInput`/`Message::TermPaste` 的两处,L905/914/1040)
- Modify: `crates/dozer-app/src/app.rs`(新增 `keyboard_term_target`
  纯函数)

**Interfaces:**
- Consumes: 既有 `App.left_view`/`App.active_zone`(均已存在,不新增
  字段)。
- Produces: `pub(crate) fn keyboard_term_target(app: &App) -> TermTarget`

- [ ] **Step 1: `app.rs` 新增 `keyboard_term_target`**

放在 `terminal_visible` 自由函数(L969-973)附近:

```rust
/// 键盘/粘贴事件此刻该写给右侧共享终端条还是 SSH 面板自己的内嵌终端。
/// 复用既有 `active_zone`(点击左右面板区任意位置就会更新,已经在驱动
/// `left_zone`/`right_zone` 的高亮边框,见 `App::set_active_zone`)——
/// SSH 面板在左侧且左侧是当前聚焦区时走 SSH 面板,否则走现状的共享
/// 终端条(不需要新增专门的终端焦点状态)。
pub(crate) fn keyboard_term_target(left_view: LeftView, active_zone: Option<ZoneSide>) -> TermTarget {
    if left_view == LeftView::Ssh && active_zone == Some(ZoneSide::Left) {
        TermTarget::SshPanel
    } else {
        TermTarget::Shared
    }
}
```

（参数用 `left_view`/`active_zone` 两个值而不是 `&App` 整体——`main.rs`
拿到的是 `&App`(不可变借用),这两个字段现有的访问方式写这一步时核实
是不是已经有 `pub(crate)` getter,没有就加一个,或者直接内联判断,不
一定要专门抽成这个纯函数,只要行为一致即可。）

- [ ] **Step 2: `main.rs` 两处构造点接入**

`main.rs:905`(⌘V 粘贴,剪贴板文本)和 `main.rs:914`(⌘V 粘贴,截图
临时文件路径)、`main.rs:1040`(普通按键/IME/拖文件):

```rust
-                                app.update(Message::TermPaste(text));
+                                let target = app::keyboard_term_target(app.left_view(), app.active_zone());
+                                app.update(Message::TermPaste(target, text));
```

```rust
-                                app.update(Message::TermPaste(path.to_string_lossy().into_owned()));
+                                let target = app::keyboard_term_target(app.left_view(), app.active_zone());
+                                app.update(Message::TermPaste(target, path.to_string_lossy().into_owned()));
```

```rust
-            if let Some(bytes) = bytes {
-                app.update(Message::TermInput(bytes));
-                window.request_redraw();
-            }
+            if let Some(bytes) = bytes {
+                let target = app::keyboard_term_target(app.left_view(), app.active_zone());
+                app.update(Message::TermInput(target, bytes));
+                window.request_redraw();
+            }
```

（`app.left_view()`/`app.active_zone()` 这两个 getter 如果 `App` 目前
没有对外暴露(字段是 `pub(crate)` 还是私有,写这一步时核实——`main.rs`
和 `app.rs` 是同一个 crate,`pub(crate)` 字段在 `main.rs` 里能直接读,
不一定需要额外 getter 方法,按实际可见性决定要不要加)。

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: `main.rs`/`app.rs` 之间的调用点全部对齐;剩下的编译错误应该
只集中在 Task 12/13/14 还没做的 `active_tab_view`(`term_view::view`
调用点缺 `target` 参数)、`ssh_terminal_visible`(还未定义)、内核对
`ssh::Message::OpenTerminal` 的拦截分支(还没改名)、`LeftView::Ssh`
渲染分支(还没加 `ssh_terminal_pane`)几处。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/main.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): route keyboard/paste TermTarget via existing active_zone

EOF
)"
```

---

### Task 12: `ssh_terminal_visible()` + `active_tab_view` 更新 + 删除死状态 `term_focused`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(新增 `ssh_terminal_visible`
  自由函数;`active_tab_view` L6924-6939;删除 `term_focused` 字段
  L1377、初始化 L1712、唯一读取点 L6929)

- [ ] **Step 1: 新增 `ssh_terminal_visible`**

紧邻 `terminal_visible`(L969-973)之后:

```rust
/// SSH 面板内嵌终端此刻是否真的呈现在用户眼前——镜像 `terminal_visible`,
/// 判定对象换成左侧:SSH 面板必须是当前左视图,且没有被"右侧放大"盖住
/// (角色与 `terminal_visible` 的 `MaximizedPane::Left` 判断对调:终端在
/// 右、被左侧放大遮住;SSH 面板在左、被右侧放大遮住)。
pub(crate) fn ssh_terminal_visible(state: &ShellState) -> bool {
    state.left_view == LeftView::Ssh && state.maximized != Some(MaximizedPane::Right)
}
```

`App` 加一个方法包装(镜像 `terminal_visible` 方法包装,L2429 附近):

```rust
    fn ssh_terminal_visible(&self) -> bool {
        ssh_terminal_visible(&self.shell_state())
    }
```

- [ ] **Step 2: `active_tab_view` 改用 `keyboard_term_target`**

```rust
-        Some(tab) => term_view::view(&tab.model, app.term_focused),
+        Some(tab) => term_view::view(
+            &tab.model,
+            keyboard_term_target(app.left_view, app.active_zone) == TermTarget::Shared,
+            TermTarget::Shared,
+        ),
```

（`app.left_view`/`app.active_zone` 字段可见性同 Task 11 的核实——
`active_tab_view` 本来就在 `app.rs` 里,同 crate 同模块,大概率能直接
读到私有字段,不需要 getter。）

- [ ] **Step 3: 删除死状态 `term_focused`**

- 删除 `App` 结构体里的 `term_focused: bool` 字段(L1377)。
- 删除初始化处的 `term_focused: true,`(L1712)。
- 确认删除后没有其它读取点:
  Run: `grep -n "term_focused" crates/dozer-app/src/app.rs`
  Expected: 无匹配(Step 2 已经把唯一读取点换成
  `keyboard_term_target(...)`)。

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 剩余编译错误应该只集中在 Task 13(内核拦截分支改名)和
Task 14(`LeftView::Ssh` 渲染分支缺 `ssh_terminal_pane`)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add ssh_terminal_visible(), remove dead term_focused flag

EOF
)"
```

---

### Task 13: 内核拦截 `OpenSshTab`/`CloseSshTab`/`SelectSshTab`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(原 `Message::Ssh(ssh::Message::
  OpenTerminal(host_id))` 拦截分支,L3344-3350;`todo::view` 调用点的
  `SessionTabSummary` 摘取逻辑,L6075-6083)

- [ ] **Step 1: 替换拦截分支**

```rust
-            Message::Ssh(ssh::Message::OpenTerminal(host_id)) => {
-                self.with_focused_project(|ws, io| {
-                    ws.ssh.record_reopen_after_trust(host_id.clone());
-                    ws.spawn_ssh_tab(io, host_id);
-                });
-            }
+            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Terminal)) => {
+                self.with_focused_project(|ws, io| {
+                    // 已经开着这台主机的终端 tab 就直接切过去,不重新握手
+                    // 连一遍(阶段 3 SFTP 决定"每个 tab 独立新建连接",但
+                    // 终端 tab 本来就是"一台主机一条常驻连接",重复点
+                    // "终端"图标应该是切换焦点而不是叠加新连接)。
+                    let already_open = ws
+                        .ssh_tabs
+                        .iter()
+                        .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
+                    if already_open {
+                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Terminal);
+                    } else {
+                        ws.ssh.record_reopen_after_trust(host_id.clone());
+                        ws.spawn_ssh_tab(io, host_id);
+                    }
+                });
+            }
+            // Sftp 种类阶段 4 不处理内容(阶段 3 再接),但仍要吃掉这条
+            // 消息、不让它落进下面的通配分支(通配分支会把它转发给
+            // ssh::update,那边的穷尽匹配分支是空 no-op,效果上等价,
+            // 但显式吃掉更清楚地表达"阶段 4 有意不处理"这件事)。
+            Message::Ssh(ssh::Message::OpenSshTab(_, ssh::SshTabKind::Sftp)) => {}
+            Message::Ssh(ssh::Message::CloseSshTab(host_id, kind)) => {
+                self.with_focused_project(|ws, io| {
+                    if kind == ssh::SshTabKind::Terminal {
+                        ws.close_ssh_tab(io, &host_id, kind);
+                    }
+                });
+            }
+            Message::Ssh(ssh::Message::SelectSshTab(host_id, kind)) => {
+                self.with_focused_project(|ws, _io| {
+                    ws.select_ssh_tab(host_id, kind);
+                });
+            }
```

- [ ] **Step 2: Todo 面板派发弹层的会话来源扩展到 `ssh_tabs`**

`app.rs:6075-6083`(`LeftView::Todo` 分支,`SessionTabSummary` 摘取):

```rust
-                let tabs: Vec<todo::SessionTabSummary> = ws
-                    .tabs
-                    .iter()
-                    .map(|t| todo::SessionTabSummary {
-                        session_id: t.info.id.clone(),
-                        title: tab_title(t.agent, t.cwd.as_deref(), &t.info.name),
-                        alive: t.alive,
-                    })
-                    .collect();
+                let tabs: Vec<todo::SessionTabSummary> = ws
+                    .tabs
+                    .iter()
+                    .chain(ws.ssh_tabs.iter())
+                    .map(|t| todo::SessionTabSummary {
+                        session_id: t.info.id.clone(),
+                        title: tab_title(t.agent, t.cwd.as_deref(), &t.info.name),
+                        alive: t.alive,
+                    })
+                    .collect();
```

（这一步是 Task 7 Step 4 结尾留的待办——阶段 4 把 SSH 终端 tab 搬出
`ws.tabs` 后,Todo 面板的派发弹层如果不跟着扩展,会看不到任何 SSH
会话,是一个真实的功能回归,不是可选项。）

- [ ] **Step 3: 编译 + 全量测试**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译错误应该只剩 Task 14(`LeftView::Ssh` 渲染分支缺
`ssh_terminal_pane` 函数、`PanelDims.ssh_split`/`Divider::SshSplit`
不存在)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): intercept OpenSshTab/CloseSshTab/SelectSshTab, extend Todo dispatch to ssh_tabs

EOF
)"
```

---

### Task 14: `LeftView::Ssh` 两栏渲染 + `ssh_terminal_pane`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(`PanelDims` 结构体 L307-330;
  `Divider` 枚举 L421-426;`LeftView::Ssh` 渲染分支 L6144-6153;新增
  `ssh_terminal_pane`/`ssh_tab_bar` 函数)

**Interfaces:**
- Consumes: Task 6/7/9/10/12 的全部基础设施
  (`ssh_tabs`/`ssh_active`/`ssh_active_tab_mut`/`term_view::view`/
  `keyboard_term_target`/`tabs::tab_core`)。

- [ ] **Step 1: `PanelDims` 新增 `ssh_split`**

`PanelDims` 结构体(L307-330 附近,`project_split` 那一行)之后加:

```rust
    /// SSH 面板"主机列表 | 内嵌终端"两栏的分屏比例,镜像 `project_split`。
    pub ssh_split: f32,
```

`PanelDims::default()`(同一个 impl 块里 `project_split:
theme::geometry::default_split_ratio(),` 那一行)之后加:

```rust
            ssh_split: theme::geometry::default_split_ratio(),
```

`grep -n "project_split: clamp_split" crates/dozer-app/src/app.rs` 找到
夹取逻辑那个位置,同样加一行 `ssh_split: clamp_split(d.ssh_split),`。

- [ ] **Step 2: `Divider` 新增 `SshSplit`**

```rust
pub enum Divider {
    LeftRight,
    LeftPairSplit,
    ProjectSplit,
+   /// SSH 面板内部的分隔线:左边主机列表、右边内嵌终端。
+   SshSplit,
    RightPairSplit,
}
```

`grep -n "Divider::ProjectSplit =>" crates/dozer-app/src/app.rs` 找到
`Divider` 相关的其它 `match`(比如拖拽换算宽度那段,通常和
`ProjectSplit`/`FilesPairSplit` 在同一个 `match` 里处理拖拽增量 →
`dims.xxx_split` 的赋值),照着 `ProjectSplit` 那一支加一条
`Divider::SshSplit => self.dims.ssh_split = ratio,` 同款分支(具体位置
写这一步时用 grep 定位,通常在处理 `Message::DividerDragged`-类消息的
函数里)。

- [ ] **Step 3: `ssh_tab_bar` + `ssh_terminal_pane`**

新增两个函数(放 `app.rs` 里,`preview_pane`/`active_tab_view` 附近,
同属"渲染函数,用顶层 `Message`"这一类):

```rust
/// SSH 面板自己的 tab 条:遍历 `ws.ssh_tabs`,每个渲染一个可关闭 tab
/// (复用 `tabs::tab_core`,同右侧共享终端条现有的可关闭语义)。前缀
/// 图标固定用 `IconKind::Terminal`(阶段 4 只有这一种;阶段 3 加 Sftp
/// 变体后按 tab 的种类换图标,写计划阶段核实 `SessionTab` 本身不带
/// `SshTabKind` 字段,种类信息只在 `ws.ssh_active` 里——阶段 4 全部
/// `ssh_tabs` 里的 tab 都是 `Terminal` 种类,这里暂时不需要按 tab 查
/// 种类,阶段 3 扩展这个函数时才需要处理"同一个 host_id 可能对应两个
/// 不同种类的 tab,要分别渲染两个 tab 条目"这件事)。
fn ssh_tab_bar<'a>(
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut bar = row![].spacing(2);
    for tab in &ws.ssh_tabs {
        let host_id = tab.info.id.strip_prefix("ssh:").unwrap_or(&tab.info.id).to_string();
        let is_active = ws
            .ssh_active
            .as_ref()
            .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal);
        let label = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
        let content = row![
            icons::view(icons::IconKind::Terminal, crate::theme::icon_size::row(), theme::color::DIM),
            text(label).size(theme::font::caption()),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center);
        let (select, close) = tabs::tab_core(
            content.into(),
            crate::theme::icon_size::row(),
            theme::color::DIM,
            /* close_interactive */ true,
            Message::Ssh(ssh::Message::SelectSshTab(host_id.clone(), ssh::SshTabKind::Terminal)),
            Message::Ssh(ssh::Message::CloseSshTab(host_id.clone(), ssh::SshTabKind::Terminal)),
            |_hover| Message::Noop, // 同 host_card 的 hover 处理,写计划阶段核实是否需要真实 HoverId 接线
            |_hover| Message::Noop,
        );
        let bg = if is_active { theme::color::CARD } else { theme::color::BG };
        bar = bar.push(
            container(row![select, close].align_y(iced_widget::core::Alignment::Center))
                .padding([6, 10])
                .style(move |_t: &iced_widget::Theme| container::Style {
                    background: Some(bg.into()),
                    ..container::Style::default()
                }),
        );
    }
    bar.into()
}

/// SSH 面板内嵌终端区:tab 条 + 终端画布(或空态)。镜像 `preview_pane`/
/// `project_preview_pane` 的既有模式——渲染函数不属于 `extensions::ssh`
/// 模块,因为它要用顶层 `Message` 直接操作 `ws.ssh_tabs`。
fn ssh_terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active_tab = ws.ssh_active.as_ref().and_then(|(host_id, _kind)| {
        ws.ssh_tabs
            .iter()
            .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()))
    });
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = match active_tab {
        Some(tab) => term_view::view(
            &tab.model,
            keyboard_term_target(app.left_view, app.active_zone) == TermTarget::SshPanel,
            TermTarget::SshPanel,
        )
        .into(),
        None => container(
            text("点主机卡片的终端/文件传输图标开始")
                .size(theme::font::body())
                .color(theme::color::DIM),
        )
        .padding(20)
        .into(),
    };
    container(column![ssh_tab_bar(ws), body].height(Length::Fill))
        .width(width)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BG.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

（`icons::view`/`theme::icon_size::row`/`tab_title` 的具体 import 路径
按 `app.rs` 文件顶部现有 `use` 语句核实,这个文件里大概率已经全部
`use` 过,不需要新增;`app.left_view`/`app.active_zone` 字段可见性同
Task 11/12 的核实。）

- [ ] **Step 4: `LeftView::Ssh` 渲染分支改成两栏**

```rust
            LeftView::Ssh => {
                if ws.project.is_none() {
                    return column![].into();
                }
                let (list_portion, content_portion) = split_portions(app.dims.ssh_split);
                let list_pane =
                    ssh::view(&ws.ssh, Length::FillPortion(list_portion), zone_pane_border(zone, lc))
                        .map(Message::Ssh);
                row![
                    list_pane,
                    divider_bar(
                        Divider::SshSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    ssh_terminal_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
```

- [ ] **Step 5: 编译确认(整个 crate 应该能编译过了)**

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 编译成功,0 error。如果还有残留错误,大概率是本计划前面
13 个任务里某个"写这一步时核实"的细节(字段可见性/辅助函数是否已存在)
需要现场调整,不是架构性问题。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): render SSH panel as sidebar+terminal split with its own tab bar

LeftView::Ssh now mirrors the Files/Project two-pane layout: host list
on the left, ssh_terminal_pane (own closeable tab bar + embedded
term_view canvas) on the right — the terminal no longer leaves the SSH
panel when opened.

EOF
)"
```

---

### Task 15: 收尾 —— 全量测试 + clippy + fmt + 人工 GUI 验收

**Files:** 无新增改动(除非 Step 1/2 发现需要清理的死代码)。

- [ ] **Step 1: 死代码/未使用符号检查**

Run: `cargo build -p dozer-app 2>&1 | grep -i "warning: unused\|warning: never"`
Expected: 空输出。重点核对:`ssh.rs` 里 Task 3 提到的
`status_text`/`status_color` 死代码计算是否已经在 Task 4 清理;
`icon_button_entry` 的占位 `on_hover` 闭包(Task 3 里标注为"真实缺陷,
需要 hover 状态接线")是否已经按 Task 3 Step 1 末尾的说明补上真实的
`HoverId`/`hover_action` 接线——**这一步必须确认这条已经修好,不能带着
"点击卡片按钮误触发 `EditHostStart(String::new())`"这种缺陷进入验收**。

- [ ] **Step 2: 格式化 + 全量 lint**

Run: `cargo fmt -p dozer-app`
Run: `cargo clippy -p dozer-app --all-targets -- -D warnings 2>&1 | tail -100`
Expected: 无 error。

- [ ] **Step 3: 全量测试**

Run: `cargo test -p dozer-app 2>&1 | tail -100`
Expected: 全部 PASS,含本计划新增的 `extensions::ssh::tests::*`(阶段
1/2 遗留)、`workspace::tests::on_tab_attached_routes_*`(Task 6)、
`resize_all` 相关(Task 8)等测试,且不影响其它模块(Todo/Project/Files
等)既有测试。

- [ ] **Step 4: Commit(如果 Step 1/2 有清理动作)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
chore(dozer-app): clean up dead code after ssh panel phase 4 refactor

EOF
)"
```

（如果 Step 1/2 什么都没改,跳过,不要造空 commit。）

- [ ] **Step 5: 人工 GUI 验收清单**

Run: `cargo run -p dozer-app`,打开一个已配置至少一台可达 SSH 主机的
项目(没有就先用 `+添加` 配一台,比如本机 `localhost` 的 SSH 服务),
逐项核实:

- [ ] 主机卡片显示 3 个图标按钮(文件传输/终端/设置),配色是 ByteBoy2077
      金色主题(不再是紫色/靛蓝)。
- [ ] hover 卡片上的图标按钮时有正常的颜色过渡动画,不会误触发消息。
- [ ] 点"设置"图标打开编辑表单;表单字段顺序 主机名称/Host/port/
      user name 正确;密码/私钥是真正的圆形单选按钮(不是 `●`/`○` 文字);
      底部按钮 测试连接/删除/保存/取消 都在(新建主机时没有"删除"按钮)。
- [ ] "+添加"按钮在侧栏最下面(列表和编辑表单下方)。
- [ ] 点主机卡片"终端"图标:在 SSH 面板自己的 tab 条(不是右侧共享终端
      条)开一个新 tab 并显示终端窗口;焦点/视图不会跳到右侧。
- [ ] 同一台主机再点一次"终端"图标:不会新开一个连接,切到已经打开的
      那个 tab。
- [ ] 点 tab 条上的 ✕ 关闭 SSH 面板的终端 tab:内容区正确切到剩下的 tab
      或显示空态。
- [ ] 同时把右侧切到 Agent 视图(打开一个本地/agent 终端 tab)、左侧切到
      SSH 视图并打开一个 SSH 终端:点击左侧终端画布敲键盘,字符写进 SSH
      终端、不写进右侧;点击右侧终端画布敲键盘,反过来。
- [ ] 拖动窗口大小:SSH 面板内嵌终端和右侧共享终端条的字符网格同步
      变化(v1 共用同一份全局网格,这是预期行为,不是 bug)。
- [ ] 拖动 SSH 面板内部的分隔线(主机列表 | 内嵌终端):两栏宽度比例
      正常调整,持久化(切换项目再切回来,比例还在)。
- [ ] Todo 面板派发弹层("派发"按钮打开的选择层)能看到已经打开的 SSH
      终端会话,选中它派发任务文本能正确写进 SSH 终端。
- [ ] 未知 host key/host key 变化的既有流程(阶段 1/2 功能)行为不变。
