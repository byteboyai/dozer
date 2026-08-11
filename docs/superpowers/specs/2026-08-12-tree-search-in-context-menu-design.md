# 文件树右键"搜索"入口 · 全文搜索弹窗设计

**状态：已批准（需求澄清会话，2026-08-12）**

## 背景

左一项目栏文件树（`crates/dozer-app/src/extensions/files.rs` 的 `FileTree`）已有名称过滤
（顶栏搜索框，只按文件/目录**名**子串过滤可见行，见 `files.rs` 的 `tree_search`/
`search_query`/`search_editing`）。但用户要对某个目录/文件做**内容**级全文搜索时，只能靠
外部工具，没有入口。

dogfooding 反馈：需要在文件树的**右键菜单**顶部加一个"搜索"项，点了在当前目录/文件范围
内做全文搜索。本轮新增的搜索**面板是弹出窗口**（参考文件编辑弹层 `edit_modal` 的形态），
不是再往左一栏堆一个面板图标。

## 目标 / 非目标

**目标**：文件树任意行（文件或目录）右键菜单顶部新增"搜索"项；点击后弹出一个全文搜索
弹窗——形制参考文件编辑弹层 `edit_modal`（`SCRIM` 遮罩 + `CARD` 对话框），作用域预填为
被点的行（目录=其子树，文件=单文件）；输入关键字后在该作用域内做内容子串搜索，列出
`文件:行` 命中的结果列表；点击某条结果在内核里打开该文件预览（复用单击文件行同一条
`OpenFile → PreviewOpenPath` 路径）。

**非目标**（本轮明确不做）：
- **搜索历史 / 结果索引 / 后台增量预扫描**：每次查询实时扫描，不跨会话保存、不做缓存
  索引、也不在后台持续扫。
- 批量替换、跨作用域的全局搜索编排、按文件类型单独过滤。
- 搜索弹窗不挂进左侧图标栏做常驻面板（与数据库/SSH 面板不同，它是瞬态弹窗）。

> 说明：`grep` 内核天然支持正则/整词/大小写选项，但这轮只把**普通字面量子串匹配**作为
> 必做项；进阶查询选项作为可选项留实现时决定，不列入"目标/非目标"的强约束（见
> "关键语义确认"第 4 条）。

## 关键语义确认（需求澄清会话定案）

1. **弹窗形态＝参考文件编辑窗口**：全文搜索用 `edit_modal` 同款形态——全窗 `SCRIM`
   遮罩 + 居中 `CARD` 对话框（`workspace.rs::edit_modal` 的做法：外层
   `container(dialog).padding(40.0).style(SCRIM)`，内含 `CARD` 底 + `BORDER` 描边的
   对话框）。搜索弹窗作为一个叠加浮层挂进 `App::view()` 顶层的互斥浮层判断链，与
   编辑弹层/最大化遮罩同层。
2. **作用域＝右键目标**：目录 → 该目录整棵子树；文件 → 仅该文件。目录递归用
   `grep` 的 `WalkBuilder`/自带 walk（尊重 `.gitignore` 与隐藏文件规则）；单文件则只喂
   那一个路径，无递归。
3. **搜索后台＝`grep` crate**：纯 Rust 的 ripgrep 内核（`grep::printer`/`grep::searcher`），
   与用户态 `rg` 语义一致、尊重 `.gitignore`/hidden。不 spawn 外部 `rg` 二进制、不引入
   Node/Python——符合"核心不依赖外部工具/语言"裁决。新增一个 workspace 依赖。
4. **查询语义**：必做 = 普通字面子串（大小写不敏感默认；`grep` 的 `MatchCase`/正则
   能力作为可选 UI 开关，实装时若成本低则加，否则只在代码里留易扩展的枚举，本期不强求
   做 UI 开关）。
5. **命中列表面板**：每条结果显示 `相对路径:行号: 命中行文本`（参考 `rg -n` 输出）。
   点击结果 = 在内核打开该文件预览，复刻文件树单击文件行的既有路径
   （`files::Message::OpenFile` → 内核映射 `Message::PreviewOpenPath`）。
6. **打开弹窗时互斥清理**：与既有浮层互斥口径一致——开搜索弹窗时关掉文件树右键菜单
   （`ctx_menu`）、收起分支弹层等任何其它浮层（同 `edit_modal`/`maximize_overlay`
   打开时清 `context_menu` 的约定）。
7. **关闭弹窗**：× 按钮、Esc、点遮罩均关闭。无脏状态（搜索不产生持久改动），不需要
   二次确认（与编辑弹层的"放弃未保存改动"确认不同）。

## 架构与数据流

### 1. 右键菜单加"搜索"项（最顶上）

`files::context_menu_popup` 的纵向按钮列表，在现有 `if menu.is_dir { 新建文件/新建文件夹 }`
**之前**（即菜单最顶）插入一项：

```rust
items.push(menu_item(
    Some(icons::IconKind::Search),
    "搜索",
    Message::SearchInContextMenu {
        path: menu.target.clone(),
        is_dir: menu.is_dir,
    },
));
```

目录与文件行都显示"搜索"（与目标是否项目根无关）。`Menu::SearchInContextMenu` 由内核拦截，
不进 `files::update`：把 `context_menu` 置 `None`（互斥清理），再在聚焦 Workspace 上打开
搜索弹窗、预填作用域。

### 2. 新增 `extensions::search` 模块

仿 `extensions::ssh` 的扩展化布局，但不接入 `LeftView`，而是提供供内核挂弹窗的
`WorkspaceState` + `Message` + `view`（弹窗窗口）+ `update`：

```rust
// 搜索作用域：右键目标。
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Dir(PathBuf),   // 目录整棵子树
    File(PathBuf),  // 单文件
}

// 挂在 Workspace 上，弹窗是每个 Workspace 各自的状态（同 edit_session）。
#[derive(Default)]
pub struct WorkspaceState {
    open: bool,
    scope: Option<Scope>,
    query: String,        // 自绘输入框草稿（同 files 搜索框）
    query_editing: bool,
    running: bool,
    results: Vec<SearchHit>,   // 空结果态 vs 未搜索态要区分
    has_searched: bool,
    error: Option<String>,
    /// 乱序结果按 文件:行 排序后再展示。
    sort: SortState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line_no: u64,
    pub line_text: String,
}
```

`.dozer` 无持久化（非目标）。弹窗状态不随项目切换保留——`Workspace` 各自有各自的
`search`，打开时若已有关键字则保留上次（同一项目内跨次打开可复用），跨项目自然各自独立。

### 3. 新增 `Message` 变体

```rust
// files.rs：右键菜单 → 内核拦截 → 打开搜索。
Message::SearchInContextMenu { path: PathBuf, is_dir: bool },

// extensions::search
SearchOpen(Scope),            // 打开弹窗并预填作用域
SearchClose,                  // ×/Esc/点遮罩
SearchQueryChanged(String),   // 自绘输入草稿
SearchQueryEditing(bool),
SearchSubmit,                 // 回车/点"搜索"→ 启动异步搜索
SearchResults(i64, Result<Vec<(String, Vec<SearchHit>)>, String>), // 异步结果回灌
SearchPick(SearchHit),        // 点击结果 → 打开预览（内核拦截映射 PreviewOpenPath）
SearchCancel,                 // 搜索进行中取消（可选）
```

### 4. 异步搜索

`grep` crate 的调用包一个纯函数（好单测）：

```rust
/// 在 scope 内做字面子串搜索，返回按文件分组的命中。
/// 返回字符串错误（不会 panic）。
fn search_scope(scope: &Scope, query: &str, gitignore: bool) -> Result<Vec<(String, Vec<SearchHit>)>, String>
```

- `File`：`SearcherBuilder` 只建那一个文件。
- `Dir`：`grep_ignore::IgnoreBuilder` + `WalkBuilder` 递归，尊重 `.gitignore`/hidden。
- 二进制/非法 UTF-8 文件跳过（`grep` 默认行为）。
- 内核 `SearchSubmit` 里 `handle.spawn` 异步跑 `search_scope`，`emit(SearchResults)` 回灌
  （带 `project_id`，理由同数据库/SSH 面板——异步结果不能假设聚焦项目没变）。

### 5. 弹窗渲染与悬浮层整合

`App::view()` 顶层的 `stack![...]`——编辑弹层/最大化遮罩所在的那一层——搜索弹窗作为
一个新元素加入（只在 `ws.search.open` 时渲染，否则空 `column![]`，与 `edit_modal` 返回
空元素的模式一致）：

```rust
stack![base, dismiss, edit_modal(ws), search_modal(ws), ...]
```

`search_modal(ws)`（放 `extensions::search`）形制照抄 `edit_modal`：
`container(dialog).padding(40.0).style(SCRIM)`，对话框 = `CARD` 底 + `BORDER` 描边；
顶部标题行 = 作用域名 + `×`，正文 = 自绘关键字输入框（同 files 顶栏搜索框的键盘路由口径：
编辑态时 `main.rs` 拦截层把按键路由成 `SearchQueryChanged`，不喂 PTY）＋"搜索"按钮＋
结果列表（`Scrollable`）。

### 6. 键盘路由（main.rs）

- `main.rs` 既有键盘拦截层加一条：若聚焦 Workspace 的 `search.query_editing` 为真，按键
  路由成 `Message::Search(search::Message::SearchQueryChanged(...))`，不再下钻 ⌘快捷键/
  地址栏/终端转发（同编辑弹层的按键路由口径，也就是 files 搜索框/行内编辑已用的同一套）。
- Esc：弹窗打开时关弹窗（无脏状态，不需要二次确认）。

### 7. 点击结果 → 打开预览

`SearchPick(hit)` 由 `main.rs` 的 `dispatch()` 拦截映射为既有的 `Message::PreviewOpenPath`
（复用文件树单击文件行那条 `files::Message::OpenFile` → 内核映射的同一路径），打开后在
预览（或编辑弹层）中定位到该文件；是否定位到 `line_no` 属可选项（预览打开为前提）。
点完关闭搜索弹窗（互斥清理口径：预览打开时不保持弹窗悬浮）。

## 错误处理

- 作用域目录不存在 / `.gitignore` 解析失败等：不 panic，`SearchResults(Err(String))` 把
  错误文案显示在弹窗底部一行红字（复用 `theme::RED`，风格同"⚠ {err}"）。
- 空结果：显示"无匹配"中性文案，区别于"尚未搜索"。

## 测试策略

- `search_scope` 单测（`tempfile` + `grep` 内核）：
  - 目录作用域：子树内命中/未命中、`.gitignore` 忽略的文件不命中、隐藏文件不命中。
  - 文件作用域：单文件命中、行号正确。
  - 二进制/非法 UTF-8 文件跳过，不 panic。
  - 查询为空：直接返回空结果（不发起搜索）。
- 排序/分组纯函数单测（按文件分组 + 行号排序）。
- 右键"搜索"项在菜单顶部顺序、`context_menu` 关闭、`search.open` 置真、作用域预填正确：
  headless 编译验证 + 真机目测（本仓惯例）。
- 弹窗键盘路由（自绘输入编辑态）、Esc 关闭、点结果打开预览：headless 编译验证，真机目测
  留用户。

## 依赖变更

`crates/dozer-app/Cargo.toml` 新增 `grep`（ripgrep 内核，纯 Rust；本次用
`grep::searcher` + `grep_ignore`/`grep` 默认 features，不用 `-pcre2`）。版本交实现计划按
`cargo add grep` 实解版本落定，不锁死具体次版本。
