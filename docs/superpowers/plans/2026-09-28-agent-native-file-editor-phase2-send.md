# Agent-native 文件编辑器 Phase 2:发送到 Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给文件树右键(文件/目录)和预览内文本选区右键各加一个"发送给
Agent"入口,把结构化引用文本写进当前项目当前激活的 agent 终端输入框
(复用既有 `term_paste`),不新增任何持久化存储、不新增 MCP 工具。

**Architecture:** 两个入口共用同一个投递原语(`term_paste`)。文件树侧是
纯 Rust 改动:给既有 `context_menu_spec`/`context_menu_items` 加一项菜单,
判断"当前是否有可见/激活的 agent 终端"要从顶层 `App`(有 `ShellState`)
经函数参数一路传进 `files::update`,不引入模块间直接耦合。预览侧新增一条
JS→Rust 的 webview 事件(`context_menu_requested`),Rust 收到后换算坐标、
弹本地原生/iced 菜单,点击后同样调 `term_paste`。

**Tech Stack:** Rust(iced 0.14 + wry webview)、TypeScript(CodeMirror 6
host,esbuild 打包,`node --test` 跑测试)。

**Spec:** `docs/superpowers/specs/2026-09-28-agent-native-file-editor-phase2-send-design.md`
(另见其上游 `docs/superpowers/specs/2026-09-28-agent-native-file-editor-design.md`
与 `docs/dozer-v2/Dozer-V2_Agent-native_Editor_Intentional_Requirements_v0.1.md`)

## Global Constraints

- 两个"发送"动作都只写终端(`term_paste`),不落库、不新增 MCP 工具、不做
  Context Permission 权限位。
- 选区发送范围严格限定 CodeMirror 承载的文本类预览(与 Phase 1
  TextAdapter 同一范围),不含 JSON tree、图片、tabular。
- 选区为空时右键不拦截 `contextmenu`,浏览器原生菜单照常弹出。
- 预览选区右键菜单只有"发送给 Agent"一项,不重新实现原生 Copy/Cut/Paste;
  ⌘C/⌘V 快捷键不受影响。
- 当前项目没有可见/激活的 agent 终端时,两个菜单项都置灰,不是点击后报错。
- 文本模板不用 Markdown 代码围栏(纯文本拼接),避免选中内容里的反引号/
  特殊字符破坏格式。
- 目录"添加到 Agent 上下文"只发目录路径本身,不展开列举目录下的文件。
- 新增/改造函数参数 ≥7 个、且有多个同类型参数相邻时,用具名字段的参数
  结构体替代位置参数(CLAUDE.md 关键裁决)。
- 新增图标按钮复用统一组件(`icons::icon_button_entry`/`MenuSpecItem`),
  不重新手写。

## Review Focus

- 当前没有任何 agent 终端可见/激活时——两个菜单项都应置灰,而不是点了
  报错或悄悄丢字节。
- 预览内选区为空时右键——不应拦截 `contextmenu`,浏览器原生菜单要正常
  弹出。
- `selected_text` 含反引号/多行内容——文本模板拼接后不破坏格式、不丢内容。
- 文件/目录路径含空格/中文——拼进终端文本不做任何转义或截断。
- 多项目并行时——发送动作应该写进触发它的那个项目当前激活的终端,不能
  跨项目发错。

---

## Task 1: Files 右键菜单参数结构体化(纯重构,不改行为)

把 `context_menu_spec`/`context_menu_items` 的 6 个位置参数(其中 4 个
相邻 `bool`)收进一个具名字段的参数结构体,为 Task 2 新增
`agent_terminal_visible` 字段占位,先不改变任何现有行为、不新增字段。

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/view.rs:874-1042`
  (`context_menu_items`、`context_menu_spec` 两个函数签名与函数体开头)
- Modify: `crates/dozer-app/src/extensions/files/view.rs:1048-1088`
  (`context_menu_popup` 里调 `context_menu_spec` 的那处,约第 1066 行)
- Modify: `crates/dozer-app/src/extensions/files/update.rs:41-95`
  (`Message::ContextMenuOpen` 分支里调 `context_menu_items` 的那处,约
  第 61 行)
- Test: `crates/dozer-app/src/extensions/files/mod.rs:1690-2010`(现有
  12 处 `context_menu_items(...)` 调用,全部改用新结构体,断言不变)

**Interfaces:**
- Consumes: 无(Task 1 不依赖之前的任务)
- Produces: `pub(crate) struct FileContextMenuParams<'a> { target: &'a Path,
  is_dir: bool, is_root: bool, has_clipboard: bool, is_git_repo: bool,
  external_apps: &'a crate::external_apps::ExternalAppsConfig }`,
  `context_menu_spec(params: FileContextMenuParams) -> MenuSpec<Message>`,
  `context_menu_items(params: FileContextMenuParams) ->
  Vec<crate::chrome::native_menu::Item<Message>>`——Task 2 在这个结构体上
  加 `agent_terminal_visible: bool` 字段。

- [x] **Step 1: 在 `files/view.rs` 里定义参数结构体,紧贴 `context_menu_items` 之前**

```rust
/// 文件树右键菜单的参数集合——`target/is_dir/is_root/has_clipboard/
/// is_git_repo/external_apps` 六项里四个是相邻 `bool`,按 CLAUDE.md
/// 关键裁决改具名字段结构体,避免位置传参时顺序传错编译器发现不了。
pub(crate) struct FileContextMenuParams<'a> {
    pub(crate) target: &'a Path,
    pub(crate) is_dir: bool,
    pub(crate) is_root: bool,
    pub(crate) has_clipboard: bool,
    pub(crate) is_git_repo: bool,
    pub(crate) external_apps: &'a crate::external_apps::ExternalAppsConfig,
}
```

- [x] **Step 2: 把 `context_menu_items`/`context_menu_spec` 签名改成吃这个结构体**

```rust
#[cfg(target_os = "macos")]
pub fn context_menu_items(
    params: FileContextMenuParams,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(context_menu_spec(params))
}

pub(crate) fn context_menu_spec(params: FileContextMenuParams) -> MenuSpec<Message> {
    let FileContextMenuParams {
        target,
        is_dir,
        is_root,
        has_clipboard,
        is_git_repo,
        external_apps,
    } = params;
    // 函数体其余部分原样保留(target 后续用到的地方不变,只是来源换了)。
    ...
}
```

- [x] **Step 3: 更新三处生产代码调用点**

`files/update.rs:61` 附近:

```rust
let items = context_menu_items(FileContextMenuParams {
    target: &path,
    is_dir,
    is_root,
    has_clipboard,
    is_git_repo: ws_state.git_is_repo,
    external_apps,
});
```

`files/view.rs:1066` 附近(`context_menu_popup` 内部):

```rust
let spec = context_menu_spec(FileContextMenuParams {
    target: &menu.target,
    is_dir: menu.is_dir,
    is_root,
    has_clipboard,
    is_git_repo: ws_state.git_is_repo,
    external_apps,
});
```

(`is_root`/`has_clipboard` 局部变量名在两处原本就存在,只改调用形式,不改
计算逻辑。)

- [x] **Step 4: 更新 `mod.rs` 里全部 12 处测试调用**

逐个把 `context_menu_items(Path::new(...), bool, bool, bool, bool,
&ExternalAppsConfig::default())` 改成
`context_menu_items(FileContextMenuParams { target: Path::new(...), is_dir:
..., is_root: ..., has_clipboard: ..., is_git_repo: ...,
external_apps: &ExternalAppsConfig::default() })`,字段值与原位置参数
一一对应,断言内容不改。

- [x] **Step 5: 跑现有测试确认零行为变化**

Run: `cargo test -p dozer-app context_menu_items --features ""` (macOS 上;
非 mac 平台这些测试被 `#[cfg(target_os = "macos")]` 排除,跳过即可)
Expected: 全部现有 `context_menu_items_*` 测试原样通过,无新增/删除断言。

- [x] **Step 6: `cargo clippy --all-targets` 确认无新增警告**

Run: `cargo clippy -p dozer-app --all-targets`
Expected: 无因这次重构新增的警告。

- [x] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/files/view.rs \
  crates/dozer-app/src/extensions/files/update.rs \
  crates/dozer-app/src/extensions/files/mod.rs
git commit -m "refactor(files): collapse context menu params into a named struct"
```

---

## Task 2: 文件树右键"添加到 Agent 上下文"

给 `FileContextMenuParams` 加 `agent_terminal_visible: bool`,从顶层
`App::terminal_visible()` 一路传进 `files::update`,在文件树右键菜单顶部
组新增一项,点击后按文件/目录模板拼文本、调 `term_paste`。

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/view.rs`(
  `FileContextMenuParams` 加字段、`context_menu_spec` 顶部组加菜单项、
  新增 `Message::SendToAgentContext`)
- Modify: `crates/dozer-app/src/extensions/files/update.rs`(`update()`
  签名加 `agent_terminal_visible: bool` 参数、`ContextMenuOpen` 分支传入、
  新增 `Message::SendToAgentContext` 处理分支——产出一个"待发送文本"消息
  交回内核,原因见 Step 4)
- Modify: `crates/dozer-app/src/app/update.rs:3149-3172`(`Message::Files`
  分支,调用 `files::update` 前算 `self.terminal_visible()`)
- Modify: `crates/dozer-app/src/app/update.rs:5583-5603`
  (`files_project_message`,同样调用前算好)
- Test: `crates/dozer-app/src/extensions/files/mod.rs`(新增测试)

**Interfaces:**
- Consumes: Task 1 的 `FileContextMenuParams`、`context_menu_spec`、
  `context_menu_items`
- Produces: `Message::SendToAgentContext(PathBuf, bool)`(`bool` =
  `is_dir`);`files::update` 新签名
  `update(ws_state, app_state, msg, project_id, handle, emit,
  external_apps, agent_terminal_visible: bool)`;新的自由函数
  `pub(crate) fn agent_context_reference_text(relative_path: &str, is_dir:
  bool) -> String`(Task 5 的选区发送文本模板函数可以仿照同一个"纯函数、
  易单测"的写法,不复用这一个,因为字段形状不同)。

- [x] **Step 1: 写 `agent_context_reference_text` 的失败测试**

在 `crates/dozer-app/src/extensions/files/view.rs` 测试模块(文件底部
`#[cfg(test)] mod tests`,若没有就新建,参照 `mod.rs` 里其他测试的
`use super::*;` 写法)新增:

```rust
#[test]
fn agent_context_reference_text_for_file() {
    assert_eq!(
        agent_context_reference_text("src/main.rs", false),
        "请将 src/main.rs 文件纳入你的工作上下文。"
    );
}

#[test]
fn agent_context_reference_text_for_dir() {
    assert_eq!(
        agent_context_reference_text("research", true),
        "请将 research/ 目录纳入你的工作上下文。"
    );
}

#[test]
fn agent_context_reference_text_keeps_unicode_and_spaces_verbatim() {
    assert_eq!(
        agent_context_reference_text("我的 报告/draft v2.md", false),
        "请将 我的 报告/draft v2.md 文件纳入你的工作上下文。"
    );
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app agent_context_reference_text`
Expected: FAIL,`agent_context_reference_text` 未定义。

- [x] **Step 3: 实现 `agent_context_reference_text`**

紧贴 `context_menu_spec` 之前(`files/view.rs`):

```rust
/// 文件树"添加到 Agent 上下文"发送的引用文本——纯路径引用,不展开列举
/// 目录下的文件(agent 自己有 ls/glob 可以探索)。`relative_path` 不做
/// 任何转义,原样拼接(路径含空格/中文时也不处理)。
pub(crate) fn agent_context_reference_text(relative_path: &str, is_dir: bool) -> String {
    if is_dir {
        format!("请将 {relative_path}/ 目录纳入你的工作上下文。")
    } else {
        format!("请将 {relative_path} 文件纳入你的工作上下文。")
    }
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app agent_context_reference_text`
Expected: PASS(3 个测试)。

- [x] **Step 5: `FileContextMenuParams` 加新字段,顶部组加菜单项**

`view.rs` 结构体定义补一个字段:

```rust
pub(crate) struct FileContextMenuParams<'a> {
    pub(crate) target: &'a Path,
    pub(crate) is_dir: bool,
    pub(crate) is_root: bool,
    pub(crate) has_clipboard: bool,
    pub(crate) is_git_repo: bool,
    pub(crate) agent_terminal_visible: bool,
    pub(crate) external_apps: &'a crate::external_apps::ExternalAppsConfig,
}
```

`context_menu_spec` 解构处加 `agent_terminal_visible`,顶部组(`top`
`Vec`)在文件夹/文件两个分支**之前**统一追加一项(两种目标都要有):

```rust
let FileContextMenuParams {
    target,
    is_dir,
    is_root,
    has_clipboard,
    is_git_repo,
    agent_terminal_visible,
    external_apps,
} = params;
...
let mut top: Vec<MenuSpecItem<Message>> = Vec::new();
top.push(MenuSpecItem::Entry {
    icon: Some(icons::IconKind::MessageSquare),
    icon_color: None,
    label: "添加到 Agent 上下文".into(),
    color: if agent_terminal_visible {
        byteui::theme::color::current().body
    } else {
        dim
    },
    enabled: agent_terminal_visible,
    msg: Message::SendToAgentContext(target.to_path_buf(), is_dir),
});
if is_dir {
    top.push(MenuSpecItem::entry(...搜索...));
    ...
} else if is_git_repo {
    top.push(MenuSpecItem::entry(...回滚...));
    ...
}
```

(注意:`dim` 变量在函数体前面已经算好——`let dim =
byteui::theme::color::current().dim;`——直接复用,不重复计算。)

- [x] **Step 6: 新增 `Message::SendToAgentContext` 变体**

`files/state.rs`(或 `Message` 枚举所在处,同文件顶部"消息类型定义"区域):

```rust
SendToAgentContext(PathBuf, bool),
```

- [x] **Step 7: `files::update` 签名加 `agent_terminal_visible` 参数,处理新消息**

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
    external_apps: &crate::external_apps::ExternalAppsConfig,
    agent_terminal_visible: bool,
) {
    match msg {
        ...
        Message::SendToAgentContext(target, is_dir) => {
            let Some(tree) = ws_state.file_tree.as_ref() else {
                return;
            };
            let relative = crate::project::path_string(
                crate::project::PathKind::Relative,
                &target,
                tree.root(),
            );
            let text = agent_context_reference_text(&relative, is_dir);
            // 真正的 PTY 写入需要 main.rs 的终端句柄,`files::update` 拿不到,
            // 同 `CopyPath` 的既有模式——把消息 `emit` 回内核顶层拦截处理。
            emit(Message::RequestSendToAgentTerminal(text));
        }
        ...
    }
}
```

(`Message::RequestSendToAgentTerminal(String)` 是本步同时新增的一个顶层
`crate::app::Message` 变体,不是 `files::Message`——`files::update` 产出
的 `files::Message` 只能经 `emit` 回内核,`CopyPath`/`OpenSearch` 已经是
这个模式的先例,见 `files/update.rs:69-82` 的注释。为了让这条消息能跨到
顶层 `Message` 而不是继续留在 `files::Message` 里,`ContextMenuOpen`
处理块里 `emit(msg)` 的 `msg` 类型是 `crate::app::Message`——这次同理,
在 `files::update` 内部构造并 `emit` 一个顶层 `Message` 变体,而不是
`files::Message` 变体。)

- [x] **Step 8: 顶层新增 `Message::RequestSendToAgentTerminal`,接线到 `term_paste`**

`app/message.rs`(顶层 `Message` 枚举):

```rust
RequestSendToAgentTerminal(String),
```

`app/update.rs`(`App::update` 的顶层 `match message`,新增一支,与
`Message::Files(msg)` 平级):

```rust
Message::RequestSendToAgentTerminal(text) => {
    self.term_paste(terminal::TermTarget::Shared, text);
}
```

- [x] **Step 9: 两处 `files::update` 调用点补 `agent_terminal_visible` 实参**

`app/update.rs:3149` 附近(`Message::Files(msg)` 分支)——`self.terminal_visible()`
是 `&self` 方法,必须在 `app_files`/`ws` 两处可变借用**之前**求值,和
`external_apps.clone()` 那行一样提前算:

```rust
Message::Files(msg) => {
    let Some(project_id) = self.active_project_id else {
        return;
    };
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Files(m));
    };
    let external_apps = self.external_apps.clone();
    let agent_terminal_visible = self.terminal_visible();
    let app_files = &mut self.files;
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    files::update(
        &mut ws.files,
        app_files,
        msg,
        project_id,
        &handle,
        emit,
        &external_apps,
        agent_terminal_visible,
    );
}
```

`app/update.rs:5583` 附近(`files_project_message`)同样处理:

```rust
pub(crate) fn files_project_message(&mut self, project_id: i64, msg: files::Message) {
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Files(m));
    };
    let external_apps = self.external_apps.clone();
    let agent_terminal_visible = self.terminal_visible();
    let app_files = &mut self.files;
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    files::update(
        &mut ws.files,
        app_files,
        msg,
        project_id,
        &handle,
        emit,
        &external_apps,
        agent_terminal_visible,
    );
}
```

- [x] **Step 10: `ContextMenuOpen` 分支里把 `agent_terminal_visible` 传进 `FileContextMenuParams`**

`files/update.rs` 的 `Message::ContextMenuOpen` 分支(macOS 与非 mac 两条
路径都要更新——macOS 走 `context_menu_items(FileContextMenuParams { ...
agent_terminal_visible, ... })`;非 mac 走 `app_state.context_menu =
Some(ContextMenu { ... })` 那条,`ContextMenu` 结构体本身不需要存
`agent_terminal_visible`——`context_menu_popup` 渲染时另外从
`ws_state`/`app_state` 现算,或者更简单:把 `agent_terminal_visible` 存进
非 mac 的 `ContextMenu` 结构体一并带上,渲染时直接用,不用重新计算)。

- [x] **Step 11: 写"菜单项在无终端时置灰"的失败测试**

```rust
#[cfg(target_os = "macos")]
#[test]
fn context_menu_items_disables_send_to_context_without_terminal() {
    let items = context_menu_items(FileContextMenuParams {
        target: Path::new("/proj/src"),
        is_dir: false,
        is_root: false,
        has_clipboard: false,
        is_git_repo: false,
        agent_terminal_visible: false,
        external_apps: &crate::external_apps::ExternalAppsConfig::default(),
    });
    let send = items.iter().find_map(|i| match i {
        crate::chrome::native_menu::Item::Entry {
            msg: Message::SendToAgentContext(..),
            enabled,
            ..
        } => Some(*enabled),
        _ => None,
    });
    assert_eq!(send, Some(false), "无可见终端时该菜单项应置灰");
}

#[cfg(target_os = "macos")]
#[test]
fn context_menu_items_enables_send_to_context_with_terminal() {
    let items = context_menu_items(FileContextMenuParams {
        target: Path::new("/proj/src"),
        is_dir: false,
        is_root: false,
        has_clipboard: false,
        is_git_repo: false,
        agent_terminal_visible: true,
        external_apps: &crate::external_apps::ExternalAppsConfig::default(),
    });
    let send = items.iter().find_map(|i| match i {
        crate::chrome::native_menu::Item::Entry {
            msg: Message::SendToAgentContext(..),
            enabled,
            ..
        } => Some(*enabled),
        _ => None,
    });
    assert_eq!(send, Some(true));
}
```

- [x] **Step 12: 跑测试确认失败,然后通过**

Run: `cargo test -p dozer-app context_menu_items_disables_send_to_context`
Expected: 先 FAIL(字段/变体不存在),补完 Step 5-10 的实现后 PASS。

- [x] **Step 13: 更新 Task 1 遗留的 12 处旧测试调用,补 `agent_terminal_visible` 字段**

Task 1 的测试调用点(`mod.rs` 里 12 处)都要加
`agent_terminal_visible: true`(默认按"有终端"场景断言原有行为,除非
测试本身就是在测这个字段)。

- [x] **Step 14: `cargo test -p dozer-app` 全量跑通**

Run: `cargo test -p dozer-app`
Expected: 全部通过,无编译错误。

- [x] **Step 15: Commit**

```bash
git add crates/dozer-app/src/extensions/files/ crates/dozer-app/src/app/
git commit -m "feat(files): add 'send to agent context' menu action"
```

---

## Task 3: `EditorEvent::ContextMenuRequested` 协议新增(Rust + TS 镜像)

只做协议层:新事件变体的 Rust 定义、序列化测试,以及 TypeScript 侧类型
镜像。不接 UI,不接 `term_paste`——那是 Task 5 的事,这里先把"线"的两端
定义对齐并独立验证。

**Files:**
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs`(`EditorEvent`
  枚举、新增测试)
- Modify: `crates/dozer-app/web/editor/src/protocol.ts`(`EditorEvent`
  联合类型)
- Test: `crates/dozer-app/src/preview/webview_protocol.rs`(内嵌
  `#[cfg(test)] mod tests`)
- Test: `crates/dozer-app/web/editor/src/protocol.test.ts`

**Interfaces:**
- Consumes: 无
- Produces: Rust `EditorEvent::ContextMenuRequested { x: f32, y: f32,
  range: TextRange, selected_text: String }`;TS
  `{ kind: 'context_menu_requested'; x: number; y: number; range: Range;
  selected_text: string }`。Task 4(JS)、Task 5(Rust 接收端)都消费这个
  形状。

- [x] **Step 1: 写 Rust 侧反序列化的失败测试**

在 `webview_protocol.rs` 的 `#[cfg(test)] mod tests`(已有
`parses_selection_and_view_state` 之类的测试,紧邻着加):

```rust
#[test]
fn parses_context_menu_requested() {
    let env: WebviewEnvelope<EditorEvent> = serde_json::from_str(
        r#"{"protocol_version":1,"project_id":1,"panel":"files","tab_id":1,"document_id":"d","revision":1,"payload":{"kind":"context_menu_requested","x":120.5,"y":48.0,"range":{"start":{"line":3,"column":1},"end":{"line":3,"column":10}},"selected_text":"let x = 1;"}}"#,
    )
    .unwrap();
    match env.payload {
        EditorEvent::ContextMenuRequested {
            x,
            y,
            range,
            selected_text,
        } => {
            assert_eq!(x, 120.5);
            assert_eq!(y, 48.0);
            assert_eq!(range.start.line, 3);
            assert_eq!(selected_text, "let x = 1;");
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app parses_context_menu_requested`
Expected: FAIL,`ContextMenuRequested` 变体不存在,编译错误。

- [x] **Step 3: `EditorEvent` 枚举新增变体**

`webview_protocol.rs`,紧贴 `SelectionChanged` 之后:

```rust
/// 预览内选区右键(Phase 2):选区非空时上报,携带右键发生时刻的坐标与
/// 选区内容(现场取值,不依赖 `SelectionChanged` 的旧缓存,避免两者之间
/// 选区已变的竞态)。`x`/`y` 是 webview 本地坐标,Rust 侧还需加上 webview
/// 在窗口里的原点才是窗口坐标。
ContextMenuRequested {
    x: f32,
    y: f32,
    range: TextRange,
    selected_text: String,
},
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app parses_context_menu_requested`
Expected: PASS。

- [x] **Step 5: TS 侧镜像类型**

`protocol.ts` 的 `EditorEvent` 联合类型,紧贴 `selection_changed` 之后:

```typescript
  | {
      kind: 'context_menu_requested';
      x: number;
      y: number;
      range: Range;
      selected_text: string;
    }
```

- [x] **Step 6: 写 TS 侧的信封往返测试**

`protocol.test.ts` 底部追加:

```typescript
test('context_menu_requested payload round-trips through envelope', () => {
  const raw = encodeEnvelope(
    envelope({
      kind: 'context_menu_requested',
      x: 12,
      y: 34,
      range: { start: { line: 1, column: 1 }, end: { line: 1, column: 5 } },
      selected_text: 'abcd',
    }),
  );
  const decoded = decodeEnvelope(raw);
  assert.ok(decoded);
  assert.deepEqual(decoded?.payload, {
    kind: 'context_menu_requested',
    x: 12,
    y: 34,
    range: { start: { line: 1, column: 1 }, end: { line: 1, column: 5 } },
    selected_text: 'abcd',
  });
});
```

- [x] **Step 7: 跑 TS 测试与类型检查**

Run(在 `crates/dozer-app/web/editor/` 目录下): `npm test && npm run typecheck`
Expected: 全部通过。

- [x] **Step 8: Commit**

```bash
git add crates/dozer-app/src/preview/webview_protocol.rs \
  crates/dozer-app/web/editor/src/protocol.ts \
  crates/dozer-app/web/editor/src/protocol.test.ts
git commit -m "feat(preview): add context_menu_requested editor event"
```

---

## Task 4: JS 侧 contextmenu 监听器(main.ts)

CodeMirror host 新增 `contextmenu` 拦截逻辑:选区非空时阻止浏览器原生
菜单并上报 Task 3 定义的事件;选区为空时完全不介入。

**Files:**
- Modify: `crates/dozer-app/web/editor/src/main.ts`

**Interfaces:**
- Consumes: Task 3 的 `EditorEvent`(`context_menu_requested` 形状)、
  已有的 `post()`(约第 190 行)、`currentRange()`(约第 286 行)、模块级
  `view: EditorView`(约第 183 行声明,构造完成后本步的监听器才能安全
  引用)。
- Produces: 无新导出——纯 DOM 事件副作用,供人工在预览里右键验证。

- [x] **Step 1: 定位 `view` 构造完成的位置**

在 `main.ts` 里搜索 `view = new EditorView`(赋值语句,不是声明),确认
新监听器要注册在这条赋值语句**之后**(此前 `view` 未初始化,访问
`view.state` 会抛错)。若该赋值发生在某个初始化函数内部而非模块顶层,
在同一函数末尾、`return`/函数体结束前注册。

- [x] **Step 2: 添加 `contextmenu` 监听器**

紧邻 Step 1 定位到的位置(或紧邻现有的两个 `document.addEventListener`
调用,约第 215/243 行附近,保持风格一致):

```typescript
// Phase 2:预览内选区右键"发送给 Agent"。选区为空时完全不拦截——右键
// 照常弹浏览器原生菜单;不做"发送整篇文件"这类超出范围的功能。选区非空
// 时现场取值上报,不依赖 `emitSelection` 的节流缓存(避免与最后一次
// selection_changed 之间的时序竞态)。
view.contentDOM.addEventListener(
  'contextmenu',
  (e) => {
    const sel = view.state.selection.main;
    if (sel.empty) return;
    e.preventDefault();
    post({
      kind: 'context_menu_requested',
      x: e.clientX,
      y: e.clientY,
      range: currentRange(),
      selected_text: view.state.sliceDoc(sel.from, sel.to),
    });
  },
  true,
);
```

- [x] **Step 3: `npm run typecheck` 确认类型对齐**

Run(`crates/dozer-app/web/editor/` 目录下): `npm run typecheck`
Expected: 无类型错误(`post()` 的参数类型 `EditorEvent` 已在 Task 3
包含 `context_menu_requested` 分支,`currentRange()` 返回值类型
匹配 `range` 字段)。

- [x] **Step 4: 重新打包**

Run: `npm run build`
Expected: 生成新的 `crates/dozer-app/assets/editor/editor.js`/
`editor.css`(构建脚本 `build.mjs` 负责产出到这个目标目录,若路径不同以
`build.mjs` 里实际写的输出路径为准)。

- [ ] **Step 5: 人工验证(无自动化 DOM 测试基础设施)**

本仓库这个 TS 项目没有 jsdom/happy-dom,`node --test` 现有测试
(`protocol.test.ts`)只覆盖纯函数逻辑,不模拟 DOM 事件——`main.ts` 里
已有的键盘/剪贴板监听器同样没有自动化测试先例。跑起 `cargo run -p
dozer-app`,打开一个文本文件预览,验证:
  1. 不选中任何内容右键 → 弹系统原生菜单(有复制/粘贴等选项)。
  2. 选中一段文本右键 → 系统原生菜单不出现(Task 5 完成前,`post()` 发出
     的事件 Rust 侧还没人处理,属预期,此步只验证"原生菜单被正确拦下")。

- [x] **Step 6: Commit**

```bash
git add crates/dozer-app/web/editor/src/main.ts \
  crates/dozer-app/assets/editor/editor.js \
  crates/dozer-app/assets/editor/editor.css
git commit -m "feat(editor): intercept contextmenu on non-empty selection"
```

---

## Task 5: Rust 侧接收 `ContextMenuRequested`——坐标换算、菜单渲染、`term_paste`

收到新事件后换算成窗口坐标,存一个新的顶层弹层状态,macOS 弹原生菜单、
非 mac 弹 iced fallback,点击"发送给 Agent"后拼模板文本调 `term_paste`。

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs`(新增
  `PreviewSelectionContextMenu` 结构体,紧邻 `ProjectLinkMenu` 定义处,
  约第 678 行)
- Modify: `crates/dozer-app/src/app/app.rs`(`App` 结构体加字段,约第
  375 行附近;`Default`/`new` 初始化加 `None`,约第 801 行附近;
  `App::update` 里 `EditorEvent` 大 `match` 新增分支,约第 273 行附近;
  `preview_desired` 里把新弹层并入 webview 隐藏判断,约第 3405-3417 行)
- Create: `crates/dozer-app/src/extensions/files/view.rs` 里新增
  `preview_selection_context_menu_items`(macOS 原生)、
  `preview_selection_context_menu_popup`(非 mac iced fallback)两个函数
  ——仿照同文件已有的 `context_menu_items`/`context_menu_popup`,不新建
  文件(同一个"右键菜单"职责聚在一处)。
- Modify: `crates/dozer-app/src/app/update.rs`(新增
  `Message::PreviewSelectionMenuClose`、
  `Message::SendSelectionToAgent { path: PathBuf, range:
  crate::preview::TextRange, selected_text: String }` 两个顶层消息处理)
- Test: `crates/dozer-app/src/extensions/files/mod.rs` 或
  `crates/dozer-app/src/app/app.rs` 测试模块(视新函数落在哪个文件,测试
  同放该文件)

**Interfaces:**
- Consumes: Task 3 的 `EditorEvent::ContextMenuRequested`;Task 2 的
  `term_paste`/`terminal::TermTarget::Shared`;既有
  `crate::webview_geometry::preview_content_bounds_for(side, window_width,
  window_height, &ShellState) -> (f32, f32, f32, f32)`;既有
  `state.layout.rail_layout.side_of(PanelKind) -> Side`;既有
  `crate::project::path_string(PathKind, &Path, &Path) -> String`。
- Produces: `pub(crate) fn selection_reference_text(relative_path: &str,
  range: TextRange, selected_text: &str) -> String`(纯函数,单测覆盖
  模板拼接,含 Review Focus 的"选中内容含反引号/多行"场景);
  `Message::SendSelectionToAgent`。

- [x] **Step 1: 写 `selection_reference_text` 的失败测试**

在 `files/view.rs` 测试模块(同 Task 2 Step 1 的位置)新增:

```rust
#[test]
fn selection_reference_text_formats_range_and_body() {
    let range = crate::preview::TextRange {
        start: crate::preview::TextPosition { line: 3, column: 1 },
        end: crate::preview::TextPosition { line: 5, column: 4 },
    };
    let text = selection_reference_text("src/main.rs", range, "let x = 1;");
    assert_eq!(
        text,
        "参考 src/main.rs:3:1-5:4 这段内容:\n\nlet x = 1;"
    );
}

#[test]
fn selection_reference_text_does_not_mangle_backticks_or_newlines() {
    let range = crate::preview::TextRange {
        start: crate::preview::TextPosition { line: 1, column: 1 },
        end: crate::preview::TextPosition { line: 2, column: 1 },
    };
    let body = "```diff\n- old\n+ new\n```";
    let text = selection_reference_text("a.md", range, body);
    assert!(text.ends_with(body), "选中内容需原样保留,不被模板转义/截断");
}
```

（`crate::preview::TextRange`/`TextPosition` 需要 `pub` 可见——若当前是
`pub(crate)` 已经够用,若是模块私有需要在 `preview/mod.rs`/相应位置提升
可见性到 `pub(crate)`,先确认再写测试。）

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app selection_reference_text`
Expected: FAIL,函数不存在。

- [x] **Step 3: 实现 `selection_reference_text`**

`files/view.rs`,紧贴 `agent_context_reference_text` 之后:

```rust
/// 预览内选区"发送给 Agent"的引用文本——坐标沿用 Phase 1 `apply_precise_edit`
/// 同一套 1-based line/col 格式,agent 若要直接照做修改可以跳过
/// `locate_in_file` 直接调 `apply_precise_edit`。不用代码围栏包裹
/// `selected_text`,原样拼接,避免选中内容本身含反引号/围栏时破坏格式。
pub(crate) fn selection_reference_text(
    relative_path: &str,
    range: crate::preview::TextRange,
    selected_text: &str,
) -> String {
    format!(
        "参考 {relative_path}:{}:{}-{}:{} 这段内容:\n\n{selected_text}",
        range.start.line, range.start.column, range.end.line, range.end.column
    )
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app selection_reference_text`
Expected: PASS。

- [x] **Step 5: `app/message.rs` 新增弹层状态结构体**

紧邻 `ProjectLinkMenu`(约第 678 行):

```rust
/// 预览内选区右键"发送给 Agent"浮层状态,镜像 `ProjectLinkMenu`/
/// `TabContextMenu`。`panel`/`tab_id` 供非 mac iced fallback 判断这个
/// 浮层是否盖住了当前正在算隐藏的那个 webview(同 `tab_menu_covers_this`
/// 的用法)。
pub(crate) struct PreviewSelectionContextMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) panel: crate::app::PanelKind,
    pub(crate) tab_id: usize,
    pub(crate) path: std::path::PathBuf,
    pub(crate) range: crate::preview::TextRange,
    pub(crate) selected_text: String,
    pub(crate) agent_terminal_visible: bool,
}
```

- [x] **Step 6: `App` 结构体加字段 + 初始化**

`app/app.rs` 约第 375 行(`project_link_menu` 旁边):

```rust
pub(crate) preview_context_menu: Option<PreviewSelectionContextMenu>,
```

约第 801 行(对应初始化处):

```rust
preview_context_menu: None,
```

- [x] **Step 7: `App::update` 里新增 `EditorEvent::ContextMenuRequested` 分支**

在 `app/update.rs:273` 附近那个 `match event.payload` 里,紧邻
`EditorEvent::SelectionChanged` 分支之后新增(`tab`/`path`/`binding`/
`event.revision` 均已在这个作用域内,同 `SelectionChanged` 分支一样可以
直接用;`self` 的几何/终端可见性查询要在 `tab` 的可变借用**结束之后**
再调用——如果借用检查报冲突,把 `path.clone()` 提前、在这个 `match` 分支
最外层先把 `self.terminal_visible()`/窗口尺寸/`Side` 算好存局部变量,
再进 `tab` 的字段读取,参考本函数里 `SaveRequested` 分支已经用
`path.clone()` 提前结束借用的写法):

```rust
EditorEvent::ContextMenuRequested {
    x,
    y,
    range,
    selected_text,
} => {
    let side = self
        .shell_state()
        .layout
        .rail_layout
        .side_of(binding.panel);
    let (x0, y0, _, _) = crate::webview_geometry::preview_content_bounds_for(
        side,
        self.window_size.0,
        self.window_size.1,
        &self.shell_state(),
    );
    self.preview_context_menu = Some(crate::app::message::PreviewSelectionContextMenu {
        x: x0 + x,
        y: y0 + y,
        panel: binding.panel,
        tab_id: binding.tab_id,
        path: path.clone(),
        range,
        selected_text,
        agent_terminal_visible: self.terminal_visible(),
    });
}
```

（`self.window_size` 的确切字段名/类型以仓库现有用法为准——搜索
`self.window_size` 已有的其它调用点核对,若字段名不同按实际名称调整。）

- [x] **Step 8: 新增两个菜单构造函数(仿照 Files 的 native/iced 双版本)**

已确认 `MenuSpec<Msg> = Vec<MenuSpecItem<Msg>>` 是泛型别名
(`crates/dozer-app/src/menu_spec.rs:31`),`files/view.rs` 里
`context_menu_spec` 的裸 `Message` 是靠文件顶部 `use super::*;`
解析成 `files::Message`(定义在 `files/state.rs:187`)。
`Message::SendSelectionToAgent` 这次不走 `files::update`——触发源是顶层
`EditorEvent` 分发(Task 5 Step 7 在 `App::update` 里直接构造并存状态),
所以定义成顶层 `crate::app::Message` 变体,**不是** `files::Message`。
这两个新函数因此必须显式写 `MenuSpec<crate::app::Message>`,不能用裸
`Message`(裸 `Message` 在这个文件里永远是 `files::Message`,写错会
编译失败或更糟——静默匹配到同名但语义不同的枚举)。

`files/view.rs`,紧贴 `context_menu_popup` 之后:

```rust
/// 预览选区右键菜单——只有一项"发送给 Agent",与文件树右键共用
/// `IconKind::MessageSquare` 强化"同一类操作"的视觉关联。注意消息类型是
/// `crate::app::Message`(顶层),不是本文件裸 `Message`(= `files::Message`)
/// ——触发源是 `EditorEvent` 分发而不是文件树右键。
pub(crate) fn preview_selection_context_menu_spec(
    menu: &crate::app::message::PreviewSelectionContextMenu,
) -> MenuSpec<crate::app::Message> {
    let dim = byteui::theme::color::current().dim;
    vec![MenuSpecItem::Entry {
        icon: Some(icons::IconKind::MessageSquare),
        icon_color: None,
        label: "发送给 Agent".into(),
        color: if menu.agent_terminal_visible {
            byteui::theme::color::current().body
        } else {
            dim
        },
        enabled: menu.agent_terminal_visible,
        msg: crate::app::Message::SendSelectionToAgent {
            path: menu.path.clone(),
            range: menu.range,
            selected_text: menu.selected_text.clone(),
        },
    }]
}

#[cfg(target_os = "macos")]
pub fn preview_selection_context_menu_items(
    menu: &crate::app::message::PreviewSelectionContextMenu,
) -> Vec<crate::chrome::native_menu::Item<crate::app::Message>> {
    crate::menu_spec::to_native(preview_selection_context_menu_spec(menu))
}
```

- [x] **Step 8b: 写"预览选区菜单在无终端时置灰"的测试**

镜像 Task 2 Step 11 对文件树菜单做的同款断言,这次测
`preview_selection_context_menu_spec`(Step 8 新增的函数):

```rust
#[test]
fn preview_selection_menu_disabled_without_agent_terminal() {
    let menu = crate::app::message::PreviewSelectionContextMenu {
        x: 0.0,
        y: 0.0,
        panel: crate::app::PanelKind::Files,
        tab_id: 1,
        path: std::path::PathBuf::from("a.rs"),
        range: crate::preview::TextRange {
            start: crate::preview::TextPosition { line: 1, column: 1 },
            end: crate::preview::TextPosition { line: 1, column: 2 },
        },
        selected_text: "x".into(),
        agent_terminal_visible: false,
    };
    let spec = preview_selection_context_menu_spec(&menu);
    let enabled = spec.iter().find_map(|item| match item {
        MenuSpecItem::Entry { enabled, .. } => Some(*enabled),
        _ => None,
    });
    assert_eq!(enabled, Some(false), "无可见终端时该菜单项应置灰");
}

#[test]
fn preview_selection_menu_enabled_with_agent_terminal() {
    let menu = crate::app::message::PreviewSelectionContextMenu {
        agent_terminal_visible: true,
        ../* 其余字段同上一条测试 */
    };
    let spec = preview_selection_context_menu_spec(&menu);
    let enabled = spec.iter().find_map(|item| match item {
        MenuSpecItem::Entry { enabled, .. } => Some(*enabled),
        _ => None,
    });
    assert_eq!(enabled, Some(true));
}
```

（第二条测试用 `..` 结构更新语法省略重复字段——`PreviewSelectionContextMenu`
需要实现 `Clone`/字段允许这样构造,若嫌麻烦也可以把公共部分提成一个
`fn sample_menu(agent_terminal_visible: bool) -> PreviewSelectionContextMenu`
辅助函数,两条测试都调用它,减少重复。）

跑 `cargo test -p dozer-app preview_selection_menu_` 确认先失败(函数/
字段不存在)、实现 Step 8 后再通过。

- [x] **Step 9: 顶层新增 `Message::SendSelectionToAgent` 处理**

`app/message.rs` 顶层 `Message` 枚举:

```rust
SendSelectionToAgent {
    path: std::path::PathBuf,
    range: crate::preview::TextRange,
    selected_text: String,
},
PreviewSelectionMenuClose,
```

`app/update.rs`,`App::update` 顶层 `match message` 新增两支:

```rust
Message::SendSelectionToAgent {
    path,
    range,
    selected_text,
} => {
    self.preview_context_menu = None;
    let Some(ws) = self.active_workspace() else {
        return;
    };
    let Some(tree) = ws.files.file_tree.as_ref() else {
        return;
    };
    let relative = crate::project::path_string(
        crate::project::PathKind::Relative,
        &path,
        tree.root(),
    );
    let text = files::selection_reference_text(&relative, range, &selected_text);
    self.term_paste(terminal::TermTarget::Shared, text);
}
Message::PreviewSelectionMenuClose => {
    self.preview_context_menu = None;
}
```

（`ws.files.file_tree`/`active_workspace()` 的准确路径以仓库现有
`ContextMenuOpen` 分支同款查找 `ws_state.file_tree` 的写法为准,若顶层
`Workspace` 结构体访问路径不同按实际调整;`files::selection_reference_text`
需要把 Step 3 的函数可见性从 `pub(crate)` 保持不变即可,同模块内直接
`files::selection_reference_text` 调用。）

- [x] **Step 10: 触发菜单弹出——收到 `ContextMenuRequested` 后实际展示**

Step 7 只是存状态,还需要在存状态**之后**(同一分支末尾)按平台弹菜单:

```rust
#[cfg(target_os = "macos")]
{
    if let Some(menu) = &self.preview_context_menu {
        let items = files::preview_selection_context_menu_items(menu);
        let pos = (menu.x, menu.y);
        self.preview_context_menu = None; // 原生菜单是阻塞弹出,弹出前先清状态
        if let Some(msg) = crate::chrome::native_menu::show_align_no_icon_left(items, pos) {
            self.update(msg);
        }
    }
}
// 非 mac 路径:保留 `self.preview_context_menu`,交给 iced 渲染层
// (`preview_selection_context_menu_popup`,Task 6)按状态弹出。
```

（macOS 原生菜单是否真的"阻塞弹出、返回后即完成"需要对照
`extensions/files/update.rs:69-82` 的既有 `ContextMenuOpen` 处理方式
核实——那边是弹出原生菜单**之后**才可能有 `emit(msg)`,菜单状态本身
(`app_state.context_menu`)在 macOS 分支从未被设置过,只在非 mac 分支
才存;本步应该照抄同一个模式:macOS 分支根本不把
`self.preview_context_menu` 设成 `Some`,直接内联算好 `PreviewSelectionContextMenu`
的数据传给 `preview_selection_context_menu_items` 然后弹出,只有非 mac
分支才真的存进 `self.preview_context_menu` 供 iced 渲染。请对照 Files
右键菜单的实际分支结构调整 Step 7/Step 10,不要额外引入"macOS 也存状态
再读出"这个不必要的中间态。）

- [x] **Step 11: 多项目路由——已确认不需要新测试,原因记录如下**

已核实 `crates/dozer-app/src/app/app.rs` 里没有任何测试直接构造一个
完整多项目 `App` 去调用 `preview_desired`/`with_focused_project`——这个
文件里 100+ 个测试的实际惯例(参照
`terminal_visible_only_when_agent_pair_on_screen`、
`webview_hidden_by_panel_popup_*` 等)是只对**纯函数**喂最小夹具
(`ShellState`/几个 `bool`),不构造完整 `App`。`Message::SendSelectionToAgent`
的处理(Task 5 Step 9)本身不包含任何新的"选哪个项目"逻辑——它直接调
`self.term_paste(terminal::TermTarget::Shared, text)`,跨项目路由完全
由 `term_paste` 内部已有的 `self.with_focused_project(...)` 决定,这部分
逻辑是 Phase 1 之前就存在的既有代码,不在本次改动范围内,也没有引入新的
分支。因此 Review Focus"多项目并行时发到正确项目终端"这一条,由 Phase 2
这次改动的角度看,保证方式是**不新增任何跨项目判断代码**(而不是新写一个
测试)——审查这次改动时确认
`Message::SendSelectionToAgent`/`Message::SendToAgentContext`(Task 2)
两处 handler 都只调用既有的 `self.term_paste(...)`,没有自己算"发去哪个
项目",即完成核验,不需要额外构造集成测试。

- [x] **Step 12: `cargo test -p dozer-app` 全量跑通,`cargo clippy` 无新警告**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets`
Expected: 全部通过。

- [x] **Step 13: Commit**

```bash
git add crates/dozer-app/src/app/ crates/dozer-app/src/extensions/files/view.rs
git commit -m "feat(preview): wire selection context menu to term_paste"
```

---

## Task 6: 非 mac iced fallback 弹层 + webview 隐藏接线

非 mac 平台没有原生 NSMenu,`preview_context_menu` 状态需要一个 iced
渲染层(仿 `context_menu_popup`),并且要并入 `preview_desired` 已有的
webview 隐藏判断(`self.files.context_menu_is_some() || tab_menu_covers_this`
这条表达式,`app/app.rs:3405-3417` 附近)。

**已确认的现状**(供实现者省去重新排查):`preview_desired`
(`app/app.rs:3231` 起)已经把 Files 的 `context_menu`/`tab_context_menu`
接进了 webview 隐藏判断——`tab_menu_covers_this`(约 3405 行)判断
`tab_context_menu.kind` 是否匹配当前正在算隐藏的 `kind`(`PanelKind`),
再与 `self.files.context_menu_is_some()` 一起 OR 进
`webview_hidden_by_panel_popup(...)` 的第二个参数(约 3415-3420 行),
最终在 3443 行 `if app_modal_open || tab_overflow_open || panel_popup_open
{ s.visible = false; }` 生效。这条既有链路本身工作正常,不是"未修的 gap"
——本 Task 只需要把新状态接进同一条表达式,不需要改 `webview_hidden_by_panel_popup`
的函数签名。

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/view.rs`(新增
  `preview_selection_context_menu_popup` 渲染函数,仿
  `context_menu_popup`)
- Modify: `crates/dozer-app/src/app/app.rs`(`preview_desired` 里新增
  `preview_menu_covers_this` 判断,并入约 3417 行的 OR 表达式;渲染层
  挂载点——搜索 `context_menu_popup(` 的调用处,同一个地方要挂
  `preview_selection_context_menu_popup`)
- Modify: `crates/dozer-app/src/app/update.rs`(点击菜单外区域/`Escape`
  关闭——仿照 `Message::ContextMenuClose` 的清理时机,新增对
  `Message::PreviewSelectionMenuClose` 的触发点,而不只是处理分支本身)

**Interfaces:**
- Consumes: Task 5 的 `self.preview_context_menu: Option<PreviewSelectionContextMenu>`、
  `preview_selection_context_menu_spec`、`Message::PreviewSelectionMenuClose`
- Produces: 无(UI 终端节点)

- [x] **Step 1: 新增非 mac iced 弹层渲染函数**

已确认 `context_menu_popup` 的实际约定(`files/view.rs:1048-1055`):函数
吃"容器状态的引用",内部自己 `let Some(menu) = &app_state.context_menu
else { return column![].into(); };` 判空,**调用方不用先判断
`is_some()`**,恒调用、恒拿到一个 `Element`(为空时是空 `column![]`)。
新函数照这个约定,吃 `&Option<PreviewSelectionContextMenu>` 而不是
`&PreviewSelectionContextMenu`:

`files/view.rs`,紧贴 Task 5 Step 8 新增的两个函数之后。已确认
`context_menu_popup` 第 1066-1085 行的真实渲染代码——`MenuSpec` 经
`crate::menu_spec::to_iced(spec, Length::Shrink)` 转成 `Element` 列表,
外层套 `container` 用 `Padding{top,left,..}` 手算定位,没有额外的边框/
hover 逻辑(那些已经封装在 `to_iced` 内部,两处菜单都直接复用):

```rust
#[cfg(not(target_os = "macos"))]
pub fn preview_selection_context_menu_popup(
    preview_context_menu: &Option<crate::app::message::PreviewSelectionContextMenu>,
) -> Element<'_, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = preview_context_menu else {
        return column![].into();
    };
    let spec = preview_selection_context_menu_spec(menu);
    let list = crate::menu_spec::to_iced(spec, Length::Shrink);
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: menu.y,
            left: menu.x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}
```

- [x] **Step 2: 挂载点——`crates/dozer-app/src/app/view.rs:126`**

已确认挂载点:`app/view.rs:126` 是
`files::context_menu_popup(&self.files, &ws.files, &self.external_apps)`
被叠进 `stack![...]` 宏调用的地方(与 `:142` 的 `tab_context_menu_popup`
相邻)。`preview_selection_context_menu_popup` 按 Step 1 的约定恒可调用
(内部自己判空),直接在同一个 `stack![...]` 里加一行,不需要额外的
`Option`/`if let` 分支:

```rust
#[cfg(not(target_os = "macos"))]
files::preview_selection_context_menu_popup(&self.preview_context_menu),
```

插入位置紧邻 `:126`/`:142` 两行菜单弹层之后,与它们同属一个 `stack!`
调用里的元素列表。

- [x] **Step 3: `preview_desired` 新增 `preview_menu_covers_this` 判断**

`app/app.rs:3405` 附近,紧邻 `tab_menu_covers_this` 之后:

```rust
let preview_menu_covers_this = self
    .preview_context_menu
    .as_ref()
    .is_some_and(|m| m.panel == kind);
```

第 3417 行的表达式改成:

```rust
let panel_popup_open = webview_hidden_by_panel_popup(
    kind,
    self.files.context_menu_is_some() || tab_menu_covers_this || preview_menu_covers_this,
    self.project_link_menu.is_some(),
    ws.conversations.agent_picker_open(),
);
```

- [x] **Step 4: `preview_menu_covers_this` 的测试策略——已确认走既有精简惯例**

已核实 `app.rs` 里没有任何测试构造完整 `App` 去调用
`preview_desired`——这个函数本身(含 Step 3 新增的
`preview_menu_covers_this`)从未被端到端单测过,包括它现有的
`tab_menu_covers_this`(Files 预览 tab 右键菜单)也是同样待遇:两者都是
一行 `matches!`/`is_some_and` 判断,复杂度和现有的
`webview_hidden_by_panel_popup_*` 系列纯函数测试(`app.rs:3726` 起)不是
一回事——那些测的是 `webview_hidden_by_panel_popup` 这个纯函数本身
(输入几个 `bool`,断言输出),不是 `preview_desired` 整体。
`preview_menu_covers_this` 复用的是同一条既有、未被单独拆出测试的表达式
风格,不新增一个需要完整 `App` 夹具的集成测试。

若想补一条低成本回归测试,可以在 `app.rs` 测试模块里新增一个纯函数
断言,验证 `PanelKind` 匹配逻辑本身(不经过 `App`/`preview_desired`):

```rust
#[test]
fn preview_menu_covers_this_matches_only_same_panel() {
    let menu = crate::app::message::PreviewSelectionContextMenu {
        x: 0.0,
        y: 0.0,
        panel: PanelKind::Files,
        tab_id: 1,
        path: std::path::PathBuf::from("a.rs"),
        range: crate::preview::TextRange {
            start: crate::preview::TextPosition { line: 1, column: 1 },
            end: crate::preview::TextPosition { line: 1, column: 2 },
        },
        selected_text: "x".into(),
        agent_terminal_visible: true,
    };
    assert!(Some(&menu).is_some_and(|m| m.panel == PanelKind::Files));
    assert!(!Some(&menu).is_some_and(|m| m.panel == PanelKind::Project));
}
```

（这条测试只锁定"匹配逻辑本身不写反",不覆盖它接进 `preview_desired`
之后的端到端可见性效果——那部分与 `tab_menu_covers_this` 现状一致,靠
Task 6 Step 8 之后的人工 QA 覆盖,不是这次引入的新测试缺口。）

- [x] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app preview_menu_covers_this_matches_only_same_panel`
Expected: PASS。

- [x] **Step 6: 关闭路径接线——Escape 键**

已确认 Esc 关闭各类右键菜单的互斥链在
`crates/dozer-app/src/platform/window_events.rs:685-705`:一串
`if/else if` 按菜单类型顺序检查"这个浮层是否开着",最先匹配的那个负责
关闭,最后 `else` 兜底关 Files 右键菜单。新增一个 `else if` 分支,插在
`else` 兜底分支**之前**(与 `category_context_menu_open()` 等其余具名
浮层同级):

```rust
} else if app.category_database_source_context_menu_open() {
    app.update(Message::DatabaseSourceContextMenuClose);
} else if app.preview_context_menu.is_some() {
    app.update(Message::PreviewSelectionMenuClose);
} else {
    app.update(Message::Files(
        crate::extensions::files::Message::ContextMenuClose,
    ));
}
```

(按现有链条第 696-698 行的真实顺序插入,上面示例里的
`category_database_source_context_menu_open` 是占位对照——照抄这条链
在 `window_events.rs:685-705` 里实际的最后一个具名 `else if` 分支的
准确方法名,把新分支加在它和最终 `else` 之间即可,顺序本身不影响正确性,
因为这些浮层互斥、同时只会开一个。)

`app/update.rs` 里补 `Message::PreviewSelectionMenuClose` 的处理(已在
Task 5 Step 9 一并写出:`self.preview_context_menu = None;`),这里不用
重复添加。

点击菜单外区域关闭:与 Task 6 Step 2 挂载的 `stack!` 层级——若
`context_menu_popup`/`tab_context_menu_popup` 依赖某个共用的"点击空白
处关闭"`dismiss` 层(`app/view.rs` 里其余几处 `stack![base, dismiss,
...]` 用的那个模式),而 `context_menu_popup` 这组菜单目前不在那个
`dismiss` 序列里(`:126`/`:142` 两行没有配对 `dismiss` 元素,推断 Files
右键菜单点击外部关闭走的是别的机制,例如全局鼠标按下时机核对
`app_state.last_right_click` 与新点击坐标的距离,或者根本没有"点击外部
自动关闭"、只能靠 Esc/选别的菜单项),照抄 Files 右键菜单实际用的那一种
机制,不要凭空加一个新的 `dismiss` 层——这一步需要实现时对照
`context_menu_popup`/`tab_context_menu_popup` 实际的"点击外部关闭"行为
(如果存在的话)来定,而不是本 plan 假设它存在。

- [ ] **Step 7: 人工验证非 mac 平台端到端行为(自动化测试未覆盖的部分)**

Step 4 已说明自动化测试只锁定 `PreviewSelectionContextMenu.panel` 的
匹配逻辑本身,不覆盖它接进 `preview_desired` 之后"webview 真的被隐藏"
这一段端到端效果(与既有 `tab_menu_covers_this` 现状一致,不是这次新开
的缺口,但既然要交付完整功能,仍需人工过一遍)。在非 macOS 环境(或
临时改 `#[cfg(target_os = "macos")]` 条件强制走 iced fallback 路径)下
跑 `cargo run -p dozer-app`,验证:
  1. 预览内选中文本右键 → 弹出 iced 版"发送给 Agent"菜单,菜单没有
     盖住/被 webview 盖住(视觉上可见、可点击)。
  2. 点击"发送给 Agent" → 文本正确写进终端输入框,菜单关闭。
  3. 按 Esc → 菜单关闭,不影响其它右键菜单的既有 Esc 行为
     (文件树右键/Project 链接右键依旧各自正常开关)。

- [x] **Step 8: `cargo test -p dozer-app && cargo clippy --all-targets` 全绿**

- [x] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/files/view.rs crates/dozer-app/src/app/
git commit -m "feat(preview): non-macOS selection menu popup + webview hide wiring"
```
