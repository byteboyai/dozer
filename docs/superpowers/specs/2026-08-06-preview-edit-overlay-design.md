# 预览域：可插拔渲染口子 + 文本编辑弹层（设计）

日期：2026-08-06

## 背景

预览域（`crates/dozer-app/src/preview.rs` 的 `PreviewPane`）目前对每个
`TabKind::File(path)`，无条件把路径拼成 flyfish（vendored `@file-viewer/web`
JS 包，经 `dozer://flyfish/host.html?p=<encoded path>` 由 wry webview 渲染）的
URL。渲染路径完全没有分支点：任何文件都走同一套 webview 管线。

本次要做两件事：

1. 在"file tab → 用什么渲染"这条路径上留出一个决策点，为未来接入 flyfish
   以外的预览方式打底（不做注册表/trait 体系——目前只有一个具体实现，
   过度抽象是浪费）。
2. 新增一个简单的原生文本编辑器，通过预览 tab 上的"编辑"按钮以模态弹层形式
   打开，可编辑并保存回磁盘。

## 现状梳理（决定设计形状的关键约束）

- `PreviewPane`（`preview.rs`）是纯数据结构，不碰 wry/iced；
  `desired_webviews()` 产出的 `WebviewSpec{id,url,visible}` 由 main.rs 的
  `sync_webview_pool`（`main.rs:338`）做差集同步：建缺失、毁多余、
  `set_visible`/`set_bounds`/`load_url`。这是唯一改 webview 实际状态的地方。
- 已有一个"非 webview、iced 原生绘制"的先例：`TabKind::Acceptance`。它不产
  `WebviewSpec`（`preview.rs:223`），且激活时通过 `acceptance_active()`
  强制其余 tab 的 `visible=false`（`preview.rs:210-232`）。这证明
  **仅靠 `visible=false` 就能完全隐藏 wry 原生子视图**，不需要动
  `preview_content_bounds`（那是另一套只服务"折叠/放大"窗口级坐标的开关，
  两者正交）。
- `Workspace`（workspace.rs:1347）是单项目的 UI 状态容器，已有
  `acceptance: Option<AcceptanceView>`、`review: Option<ReviewView>` 这类
  "进行中的域视图"字段，以及 `tree_delete_confirm`/`context_menu`/
  `agent_picker_open` 这类模态状态。`Workspace::view` 顶层用
  `stack![base, dismiss, popup]` 的固定套路叠加模态（`workspace.rs:4054-4108`
  的 `popped`/`maximized` 分支链），每种模态一个 `else if` 分支。
- `iced_widget` 已启用但从未使用过 `text_editor` widget（其余所有可编辑文本
  都是手撸 `String` 缓冲区 + IME 事件转发，是为兼容 macOS IME 走的历史路径，
  单行地址栏场景下合理；多行编辑器没有理由重新造这套轮子）。
- 文件类型判断目前只有两张扩展名表：`assets.rs::mime_for`（协议 Content-Type）
  和 `icons.rs::icon_for_file`（树图标）。没有内容嗅探。

## 设计

### 1. 渲染决策口子（最小抽取，不做注册表）

把 `desired_webviews()` 里内联拼 flyfish URL 的那行抽成具名函数：

```rust
fn flyfish_url(path: &Path) -> String {
    format!("dozer://flyfish/host.html?p={}", encode_component(&path.to_string_lossy()))
}
```

目前 `TabKind::File` 只有这一条路径，函数只是把"决策"和"拼接"分开命名，
为将来"某些扩展名不用 flyfish、走 Acceptance 式 iced 原生 pane"的分叉
预留一个函数级的插入点。不新增 trait、不新增注册表、不新增第二个具体实现——
按 YAGNI，等真的出现第二种预览类型时再决定抽象形状。

文本编辑器**不占用**这个决策点：它不是 `TabKind::File` 的替代渲染方式，
是叠加在预览 tab 之上的独立模态（见下），二者正交，编辑时预览 tab 本身
不变身、不切 `TabKind`。

### 2. 编辑会话状态

新增（位置：`workspace.rs`，紧邻 `AcceptanceView`）：

```rust
pub struct EditSession {
    pub tab_id: usize,
    pub path: PathBuf,
    pub content: iced_widget::text_editor::Content,
    pub dirty: bool,
    pub error: Option<String>,
    pub confirm_discard: bool,
}
```

挂载：`Workspace.edit_session: Option<EditSession>`，与 `acceptance`/`review`
同级字段。全局至多一个编辑会话（弹层是模态，不允许并行编辑多个文件）。

**id 口径**：`EditSession.tab_id` 存的是 `PreviewTab.id`（webview 池用的稳定
id），不是 `tabs` vec 下标——`PreviewEditOpen(usize)` 消息本身仍按既有
`Message::Preview*` 系列的约定传 vec 下标（因为按钮是从渲染时的下标发出的），
但在 `update()` 里解析出对应 `PreviewTab` 后，存进 `EditSession` 的要取它的
`.id` 字段，而不是原样保留下标。理由：编辑弹层是应用级模态（遮罩盖住整个
`base`，用户在弹层开着时点不到 tab 栏），tab 顺序在会话期间不会变，
下标本身不会失效；但 §3 的 `bump_reload` 要和 `desired_webviews()`/
webview 池对齐着同一套稳定 id 语义，混用下标会在未来任何一处忘记这个
"仅弹层期间下标不失效"的前提时留坑，所以从一开始就统一用 `.id`。

生命周期与消息（新增到 `Message` 枚举，均落在 `workspace.rs:890` 起的
`Message` 定义里）：

| 消息 | 触发 | 行为 |
|---|---|---|
| `PreviewEditOpen(usize)` | tab chip 的"编辑"按钮，携带 tab 下标 | 按下标取 `PreviewTab`，`std::fs::read_to_string(&tab.path)`；成功则 `edit_session = Some(EditSession{ tab_id: tab.id, content: Content::with_text(&text), .. })`；失败写 `ws.preview_error`（复用既有错误文案槽位），不开弹层 |
| `PreviewEditAction(text_editor::Action)` | `text_editor` widget 回调 | `content.perform(action)`；若 `action.is_edit()` 则 `dirty = true` |
| `PreviewEditSave` | 弹层"保存"按钮 / ⌘S | `std::fs::write(path, content.text())`；成功：`dirty = false`，并调 `ws.preview.bump_reload(session.tab_id)`（按 `PreviewTab.id` 查找，见 §3 缓存刷新）；失败：写 `session.error`，弹层不关 |
| `PreviewEditCloseRequest` | 弹层×按钮 / 遮罩点击 | `dirty` 为真 → `confirm_discard = true`（叠二次确认）；否则直接 `edit_session = None` |
| `PreviewEditConfirmDiscard` | 二次确认"放弃改动" | `edit_session = None` |
| `PreviewEditConfirmCancel` | 二次确认"取消" | `session.confirm_discard = false`，回到编辑态 |

### 3. 隐藏 webview + 保存后刷新预览缓存

**隐藏**：在 `preview_desired()`（`workspace.rs:2936`，`App` 级方法，
`main.rs::sync_previews` 的调用入口）里，若 `ws.edit_session.is_some()`，
对 `ws.preview.desired_webviews()` 的结果做一次 `.map` 强制
`visible = false`。`PreviewPane::desired_webviews()` 自身签名不变，
继续对编辑态一无所知——隐藏逻辑留在离 `Workspace`（有权访问 `edit_session`）
最近的调用点。

**保存后刷新**：flyfish 的 WKWebView 是有状态的原生子视图，`load_url` 只在
URL 字符串变化时触发（`main.rs:353`）。同路径保存后若不换 URL，用户关闭
编辑器会看到 stale 内容。给 `PreviewTab` 加一个 `reload_nonce: u64`
字段（默认 0），`PreviewPane` 新增 `bump_reload(&mut self, tab_id: usize)`
方法——`tab_id` 是 `PreviewTab.id`（不是 vec 下标），按 `.id` 在 `self.tabs`
里找到匹配项（同 `open_path`/`open_acceptance` 已有的"按谓词找 tab"写法）
给它的 nonce `+1`；`desired_webviews()` 拼 flyfish URL 时若
`nonce > 0` 追加 `&_r={nonce}` 查询参数，使 URL 随保存次数变化，驱动
`sync_webview_pool` 走 `load_url` 分支重新加载。

### 4. 编辑按钮的显示范围

新增 `fn is_editable_extension(path: &Path) -> bool`（位置同 `mime_for`/
`icon_for_file` 的扩展名表风格，建议放 `preview.rs` 或紧邻
`icons.rs::icon_for_file`），白名单覆盖常见纯文本扩展名起步：
`rs toml md txt json yaml yml sh py js ts tsx jsx html css xml log conf
gitignore`。无扩展名或不在表内 → 不显示编辑按钮（后续要加类型按需扩表，
不做内容嗅探）。

渲染位置：`preview_pane()`（`workspace.rs:6455` 附近）的每个 file 类
tab chip，`is_editable_extension(path)` 为真时在 chip 内追加一个"编辑"
图标按钮，`on_press` 发 `Message::PreviewEditOpen(idx)`。

### 5. 弹层渲染

在 `Workspace::view` 现有的 `stack!` 分支链（`workspace.rs:4054-4108`，
`popped`/`maximized` 那段 `if/else if` 结构）新增一条：

```rust
let popped = if ws.edit_session.is_some() {
    let dismiss = MouseArea::new(container(column![]).width(Fill).height(Fill))
        .on_press(Message::PreviewEditCloseRequest);
    stack![base, dismiss, edit_modal(ws)].width(Fill).height(Fill).into()
} else if ws.tree_delete_confirm.is_some() {
    ...
```

`edit_modal(ws)` 内容：标题行（文件名 basename + 关闭按钮）、
`text_editor(&session.content).on_action(Message::PreviewEditAction)` 主体
（等宽字体，宽高占屏幕大部分——"放大窗口"的产品意图，不是小弹窗）、
错误文案位（`session.error`，红字，风格同 `preview_error`）、底部按钮行
（"保存" primary + "关闭"）。`session.confirm_discard` 为真时，在
`edit_modal` 之上再叠一层复用 `delete_confirm_popup` 视觉风格的二次确认
（"未保存的改动将丢失，确认关闭？" + "放弃改动"/"取消"）。

### 测试

- `is_editable_extension`：各扩展名 + 无扩展名 + 大小写的单测。
- `PreviewPane::bump_reload` + 拼出的 URL 带 `_r` 参数：单测（仿照
  `preview.rs` 现有的 `desired_webviews_builds_urls_and_visibility`
  测法）。
- `EditSession` 状态机（打开失败态/脏标记/保存清脏/关闭前二次确认）：
  能抽出纯函数的部分单测，其余走 `update()` 集成测试。
- 手动验证路径：flyfish 预览 → 点编辑 → 弹层显示正确初始内容 → 改动 →
  点关闭×触发二次确认 → 取消回到编辑 → 保存 → 关闭 → flyfish 预览重新
  可见且内容已刷新（非 stale 缓存）。

## 范围外

- 不做语法高亮、行号、多光标等编辑器高级功能——"simple text editor"。
- 不做内容嗅探判断文本/二进制，纯扩展名白名单。
- 不做插件注册表/trait 体系；第二个预览后端出现前不进一步抽象。
- 不做外部改动冲突检测（编辑期间文件被外部程序修改不处理，保存时直接覆盖）。
