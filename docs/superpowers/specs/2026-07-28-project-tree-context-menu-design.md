# 项目树右键菜单设计

**状态：已批准（brainstorming 会话，2026-07-28）**

## 背景

左一项目栏文件树（`crates/dozer-app/src/project.rs` 的 `FileTree` + `workspace.rs` 的
渲染）目前每行只有左键（目录=展开/收起，文件=预览打开），没有任何右键功能。用户反馈
（dogfooding UI 问题清单第 2 项）：需要基本的右键菜单。

## 目标 / 非目标

**目标**：目录行右键菜单 = 新建文件 / 新建文件夹 / 复制 / 粘贴 / 删除 / 重命名 / 复制绝对
路径 / 复制相对路径；文件行右键菜单 = 复制 / 删除 / 重命名 / 复制绝对路径 / 复制相对路径。

**非目标**：项目根目录本身的右键（根不是 `FileTree` 的一行，改动会牵涉数据模型，留后
续）；多选/批量操作；拖拽移动文件；剪切（只有复制/粘贴，没有"剪切"概念）。

## 关键语义确认（brainstorming 会话定案）

- **"复制"＝文件管理器式**：右键"复制"把该项标记进应用内剪贴槽（`Workspace` 字段，非
  OS 剪贴板）；到目标目录右键"粘贴"才真的在磁盘上复制一份（目录递归复制）。
- **"复制绝对路径"/"复制相对路径"**：这两个才真的写系统剪贴板（文本），与上面的"复制"
  是两回事，故菜单文案刻意加了"路径"二字消歧。
- **删除＝确认框 + 移入系统回收站**：用 `trash` crate（跨平台，不碰 AppleScript/
  NSWorkspace，遵守"架构留门"裁决）；目录整棵子树作为一个回收站条目。
- **新建/重命名＝行内编辑**：与本仓既有的地址栏/验收意见框同款自绘输入（键盘走
  `main.rs` 拦截层，不用 iced 原生 `text_input`），风格统一。新建在目标目录下插入一个空
  白可编辑行；重命名把该行原地换成预填当前名的可编辑行。
- **命名冲突（粘贴/重命名/新建）**：不静默覆盖、不自动改名（如 "file (2)"），行内红字
  报错，用户自己改名重试。

## 架构与数据流

### 1. `Workspace` 新增状态

```rust
/// 右键菜单当前打开状态：定位坐标 + 目标（路径/是否目录）。
struct ContextMenu {
    x: f32,
    y: f32,
    target: PathBuf,
    is_dir: bool,
}

/// 应用内"文件管理器式"剪贴槽：最近一次"复制"的项。
/// (path, is_dir)
type TreeClipboard = Option<(PathBuf, bool)>;

enum TreeEditMode {
    NewFile,
    NewFolder,
    Rename(PathBuf), // 原路径
}

/// 行内编辑态：新建/重命名共用。
struct TreeEdit {
    parent_dir: PathBuf,
    mode: TreeEditMode,
    buffer: String,
}
```

`Workspace` 新增字段：
- `context_menu: Option<ContextMenu>`
- `tree_clipboard: TreeClipboard`
- `tree_edit: Option<TreeEdit>`
- `tree_error: Option<String>` — 冲突/失败时的行内红字文案，下一次任何树操作发起时清空
- `tree_delete_confirm: Option<(PathBuf, bool)>` — 删除确认框的目标
- `last_right_click: (f32, f32)` — 逻辑像素坐标，默认 `(0.0, 0.0)`；只在紧接着的
  `ProjectTreeContextMenu` 消息里被读取，读取前必然已被同一次右键点击的
  `RightClickAt` 更新过（见下面"事件时序"）。

### 2. 新增 `Message` 变体

```rust
RightClickAt { x: f32, y: f32 },              // main.rs 原始层发,更新 last_right_click
ProjectTreeContextMenu { path: PathBuf, is_dir: bool }, // 某行 MouseArea::on_right_press
ProjectTreeContextMenuClose,                   // 点击菜单外/Esc/动作完成后关闭

ProjectTreeNewFile(PathBuf),    // 参数=目标父目录,进入 TreeEdit::NewFile
ProjectTreeNewFolder(PathBuf),  // 同上,NewFolder
ProjectTreeRenameStart(PathBuf),// 参数=被重命名项路径,进入 TreeEdit::Rename
ProjectTreeEditEvent(AddrEvent),// 复用既有 AddrEvent(Text/Backspace/Submit/Cancel)

ProjectTreeCopy(PathBuf, bool), // 设 tree_clipboard,关菜单
ProjectTreePaste(PathBuf),      // 参数=目标目录,异步执行,关菜单
ProjectTreePasteDone(Result<PathBuf, String>), // 异步结果回灌(成功=已刷新的父目录,失败=错误文案)

ProjectTreeDeleteRequest(PathBuf, bool), // 打开确认框,关菜单
ProjectTreeDeleteConfirm,
ProjectTreeDeleteCancel,
ProjectTreeDeleteDone(Result<PathBuf, String>), // 同 PasteDone 语义

ProjectTreeCopyPath(PathBuf, PathKind), // PathKind{Absolute,Relative},main.rs 拦截写系统剪贴板
```

`AddrEvent`（已存在，见 `workspace.rs`）直接复用，不新增等价枚举——`Submit`/`Cancel`/
`Text`/`Backspace` 四个语义正好对上"确认改名或新建/取消/敲字/退格"。

### 3. 事件时序：右键定位

`iced_widget::MouseArea::on_right_press` 只能挂一个固定 `Message`，拿不到点击的像素坐标
（对照 `on_move` 才有 `impl Fn(Point) -> Message` 闭包）。定位坐标改走 `main.rs` 里已经
在用的原始 `WindowEvent` 拦截层（`Runner::on_window_event`，本来就在给分隔线拖拽做同样
的事）：

1. `on_window_event` 新增一个 `MouseInput { state: Pressed, button: Right, .. }` 分支：
   按现有的 `scale_factor`/`cursor_phys` 换算逻辑像素坐标，直接
   `workspace.update(Message::RightClickAt { x, y })`（同步调用，见 `main.rs` 既有的
   ⌘V 那种直接 `workspace.update` 写法）。
2. `on_window_event` 返回后，该 `WindowEvent` 照常继续流进 iced 正常分发（`main.rs`
   `window_event` 函数本体，`on_window_event` 只是先跑一遍，不吞事件）。若右键点在某个
   树行的 `MouseArea` 命中区内，`on_right_press` 触发，产生
   `Message::ProjectTreeContextMenu { path, is_dir }`，随本轮 `pending_messages` 一起
   在 `on_window_event` 返回之后被 `dispatch`/`workspace.update` 处理。
3. 因为步骤 1 是同步直调、步骤 2 是随后处理的消息队列，`ProjectTreeContextMenu` 的
   handler 执行时 `self.last_right_click` 保证已经是本次右键的坐标——不会读到上一次
   点击的陈旧值。
4. 若右键点在空白处（没有任何行的 `MouseArea` 命中），只有 `RightClickAt` 触发，没有
   `ProjectTreeContextMenu` 跟上，菜单不打开——符合"非目标：项目根/空白区菜单"。

### 4. 菜单渲染：浮层定位

用 `iced_widget::stack!`（`Stack` 把多个子元素叠在同一份 bounds 上，逐层渲染，不是各自
独立定位——需要手动用 `container` 的 padding 把内容推到点击坐标，这也是本仓一贯的手算
像素定位风格，`ime_cursor_area`/`preview_content_bounds` 都是同款）：

```rust
stack![
    base_view,                              // 原有四栏内容
    dismiss_layer(ws.context_menu.is_some()), // 全窗透明 MouseArea,点击=Close,菜单未开时不渲染
    menu_popup(ws),                          // 菜单本体,用 padding([y,0,0,x]) 推到点击位置
]
```

`dismiss_layer` 仅在 `context_menu.is_some()` 时才真正插入 stack（菜单关闭时该层不存
在，避免吞掉正常点击）。`Esc` 键关闭菜单走 `main.rs` 既有的键盘拦截层，加一条
"若 `context_menu` 打开且按 Esc → `ProjectTreeContextMenuClose`" 的分支。

菜单本体是纵向 `column!`，每项一个 `button`，样式复用 tab pill/按钮的既有配色
（`theme::CARD` 底 + `theme::BORDER` 描边 + `theme::CREAM`/`theme::DIM` 文字，"粘贴"在
`tree_clipboard.is_none()` 时用 `theme::DIM` 且不挂 `on_press`——参照本仓 P1L 已经用过的
"到头箭头变灰且不可点"同款处理）。

### 5. `FileTree` 新增方法

```rust
/// 强制重读一个目录的子项缓存(增删改后调用,让 visible_rows() 反映最新磁盘状态)。
/// 目录本身若已展开,保持展开;若未展开,不强行展开(仅刷新缓存,下次展开时是最新的)。
pub fn refresh(&mut self, dir: &Path) {
    self.children.insert(dir.to_path_buf(), read_children(dir));
}

/// 确保目录处于展开态(新建文件/文件夹后自动展开父目录,让新项可见)。
pub fn ensure_expanded(&mut self, dir: &Path) {
    if !self.expanded.contains(dir) {
        self.toggle(dir); // toggle 内部已处理"空目录不标 expanded"的情况
    }
}
```

**已知简化（有意接受，不修）**：重命名/删除一个目录后，它在 `children`/`expanded` 里
原路径对应的缓存条目变成孤儿（不再被任何父目录的 entries 列表引用，`push_rows` 递归不
会再访问到）——不是内存泄漏级别的问题（项目生命周期内绝对数量小），不值得为此新增子树
级联失效逻辑。重命名后若用户重新展开该目录（新路径），`toggle()` 走缓存未命中分支重新
`read_children`，自然拿到正确内容。

### 6. 文件系统操作实现

- **复制（标记剪贴槽）**：`ProjectTreeCopy(path, is_dir)` 是同步、纯内存操作——
  `self.tree_clipboard = Some((path, is_dir)); self.context_menu = None;`，不碰磁盘，
  不需要 `handle.spawn`。
- **新建文件/文件夹**：`std::fs::File::create`/`std::fs::create_dir`，`self.handle.spawn`
  异步跑，完成后主线程侧 `file_tree.refresh(&parent)` + `ensure_expanded(&parent)`。
- **复制/粘贴**：`Workspace::update` 里 `ProjectTreePaste(target_dir)` 读
  `self.tree_clipboard`，若为 `None` 直接忽略（粘贴项已在菜单里对空剪贴槽置灰不可点，这
  是双重防御）；否则 `self.handle.spawn` 异步执行——文件用 `std::fs::copy`，目录用一个新
  写的小型递归复制助手函数（标准库没有现成的 `copy_dir_all`，`fs_extra` 之类的 crate 没
  必要为这一个函数引入，本仓已有到处手写小工具函数的风格，直接写一个 `fn copy_dir_recursive
  (src: &Path, dst: &Path) -> std::io::Result<()>`，`walkdir`/`fs_extra` 都不需要新增依
  赖）。目标名冲突（`dst` 已存在）在动手复制前检查，冲突则不执行，回传
  `ProjectTreePasteDone(Err("已存在同名项"))`。
- **删除**：`trash::delete(&path)`（新依赖，支持文件与目录）。`self.handle.spawn` 异步
  跑，完成后 `file_tree.refresh(&parent_of_deleted)`。
- **重命名**：`std::fs::rename(old, new)`（同目录内改名，`new = parent.join(new_name)`）。
  冲突检查同粘贴。完成后 `file_tree.refresh(&parent)`。
- **复制路径**：`ProjectTreeCopyPath` 不落 `Workspace::update`（沿用 `PreviewPickFile` 的
  既有模式——`main.rs` 的 `dispatch()` 直接拦截，不转发）。`main.rs` 里用一个新增的纯函数
  `workspace::tree_item_path_string(kind: PathKind, path: &Path, project_root: &Path) ->
  String`（`Absolute` 直接 `path.display()`；`Relative` 用
  `path.strip_prefix(project_root)`，失败兜底整段绝对路径——不太可能失败，因为树里的项恒
  在 `project_root` 之下）算出字符串，`clipboard.write(Kind::Standard, s)` 写系统剪贴
  板，和现有 ⌘C 复制终端选区走同一条 `clipboard.write` 调用。

## 错误处理

- 所有异步文件操作失败（权限/磁盘满/路径已消失等）：不 panic，`Result` 回灌
  `*Done(Err(String))` 消息，写入 `tree_error` 由树视图顶部或菜单原位置显示一行红字（复
  用 `theme::RED`，参照终端 pane 里 `⚠ {err}` 的既有文案风格）。
- 命名冲突：粘贴前/重命名提交前先检查目标是否已存在，存在则不发起文件系统调用，直接把
  `tree_error` 设成"已存在同名项"，编辑框保持打开（不关闭，方便用户直接改名重试）。
- `trash::delete` 失败（罕见，比如系统回收站不可用）：`tree_delete_confirm` 关闭，
  `tree_error` 显示失败原因，不重试。

## 测试策略

- `FileTree::refresh`/`ensure_expanded` 单测：磁盘增删文件后 `refresh` 前后
  `visible_rows()` 差异符合预期；`ensure_expanded` 对已展开/未展开/空目录三种情况的幂等
  性。
- `copy_dir_recursive` 单测（`tempfile`）：源目录含子目录+文件，复制后目标目录结构逐项
  比对一致；目标已存在时不执行、返回错误。
- `tree_item_path_string` 单测：`Absolute`/`Relative` 两种模式，含 `Relative` 在路径不
  在 `project_root` 下时的兜底分支。
- 冲突检测的纯函数部分（"目标是否已存在"判断）单测覆盖存在/不存在两种输入。
- 右键定位时序（`RightClickAt` → `ProjectTreeContextMenu` 读到正确坐标）、浮层渲染、
  行内编辑的键盘路由：headless 只能编译验证，真机目测留用户（本仓一贯惯例）。

## 依赖变更

`crates/dozer-app/Cargo.toml` 新增 `trash`（跨平台移入系统回收站；调研版本时选取当前
稳定大版本，不锁死具体次版本号，交给实现计划落笔时定）。
