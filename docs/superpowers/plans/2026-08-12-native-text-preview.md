# 文件预览文本/代码类扩展名原生化(阶段1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `preview.rs` 白名单文本/代码扩展名(`is_editable_extension` 覆盖的一批)改走原生
`iced-code-editor` 只读渲染,不再经过 wry/flyfish;图片/PDF/Office 长尾继续走现有 wry 路径,
两者在同一个 `PreviewPane` 里按扩展名分发共存。

**Architecture:** `vendor/iced-code-editor` 的 `CodeEditor` 新增 `read_only` 标志(内部用
"允许清单"而非"屏蔽清单"partition 消息,默认拦截、显式放行滚动/光标移动/选区/复制)。
`preview.rs` 的 `PreviewTab` 新增 `editor: Option<CodeEditor>` 字段,构造时按扩展名二选一;
`desired_webviews()` 排除已原生化的 tab。预览与编辑弹层各自持有独立 `CodeEditor` 实例,但
共用同一套主题/语法映射函数(从 `workspace.rs` 挪进 `preview.rs`)。

**Tech Stack:** Rust, iced 0.14(`iced_widget`/`iced_wgpu`/`iced_winit`/`iced_renderer`),
`vendor/iced-code-editor`(本地 path 依赖,syntect 语法高亮的 canvas 编辑器),`syntect` 5.3。

## Global Constraints

- GUI 只用 iced 0.14 生态,不引入 Swift/AppKit 专属能力(见 `CLAUDE.md`)——这份计划只
  减少原生子视图(wry)使用面,不新增任何平台专属代码。
- ByteBoy2077 配色(bg `#0a0e16`、金 `#F2D94E` 甲方动作专属、奶油 `#FFE5B4`、青 `#47DEF0`、
  绿 `#1AD585`)——本计划复用已有的 `dozer_editor_style`/`dozer_syntax_theme`,不引入新配色。
- 新增 icon 按钮/tab 类 UI 需优先复用 `icons::icon_button_entry`/`tabs::tab_core`——本计划
  不新增任何 icon 按钮或 tab UI,不适用。
- 核心不依赖 Node/Python——不涉及,本计划纯 Rust。
- 一期范围以 `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md` §3 为准;本计划的
  阶段 1 范围以 `docs/superpowers/specs/2026-08-12-native-text-preview-design.md` 为准,
  已用户确认批准,不在执行期间擅自扩大范围(图片/Markdown 渲染态/PDF 不在这次任务里)。

---

### Task 1: vendor/iced-code-editor —— `CodeEditor` 新增 `read_only` 标志

**Files:**
- Modify: `vendor/iced-code-editor/src/canvas_editor/mod.rs`

**Interfaces:**
- Produces: `CodeEditor::read_only(&self) -> bool`(getter)、
  `CodeEditor::set_read_only(&mut self, enabled: bool)`(mutator)、
  `CodeEditor::with_read_only(self, enabled: bool) -> Self`(链式 builder,默认
  `read_only: false`,现有调用点不用改)。

- [ ] **Step 1: 写失败测试**

在 `vendor/iced-code-editor/src/canvas_editor/mod.rs` 靠近现有 `vim_enabled`
测试(`#[cfg(test)] mod tests` 区块,搜索 `fn test_vim_enabled` 附近)新增:

```rust
#[test]
fn test_read_only_defaults_to_false_and_is_settable() {
    let mut editor = CodeEditor::new("hello", "txt");
    assert!(!editor.read_only());

    editor.set_read_only(true);
    assert!(editor.read_only());

    let editor2 = CodeEditor::new("hello", "txt").with_read_only(true);
    assert!(editor2.read_only());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p iced-code-editor --lib test_read_only_defaults_to_false_and_is_settable -- --nocapture`
Expected: 编译失败(`read_only`/`set_read_only`/`with_read_only` 未定义)。

- [ ] **Step 3: 实现**

在 `CodeEditor` 结构体字段列表(`pub(crate) vim_enabled: bool,` 那一行附近,约第 433 行)
新增一个字段:

```rust
    /// 只读模式:`true` 时 `update()` 只放行滚动/光标移动/选区/复制,拦截
    /// 一切会修改缓冲区内容的消息。给"文件预览"只读 tab 用——与可编辑的
    /// `CodeEditor` 实例(编辑弹层)是同一份渲染实现,只是这个标志不同。
    pub(crate) read_only: bool,
```

在 `CodeEditor::new`(约第 900 行)的字段初始化列表里加 `read_only: false,`(挨着
`vim_enabled: false,` 那一行,约第 943 行)。

在 `vim_enabled`/`set_vim_enabled`/`with_vim_enabled` 三个方法附近(约第 1187-1219 行)
新增对应三个方法,复制同一套模式:

```rust
    /// 设置只读模式。见字段文档。
    pub fn set_read_only(&mut self, enabled: bool) {
        self.read_only = enabled;
    }

    /// 链式设置只读模式(构造后立即调用,如
    /// `CodeEditor::new(&text, &syntax).with_read_only(true)`)。
    pub fn with_read_only(mut self, enabled: bool) -> Self {
        self.set_read_only(enabled);
        self
    }

    /// 当前是否只读。
    pub fn read_only(&self) -> bool {
        self.read_only
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p iced-code-editor --lib test_read_only_defaults_to_false_and_is_settable`
Expected: PASS

- [ ] **Step 5: 跑整个 vendor crate 测试套件确认没有回归**

Run: `cargo test -p iced-code-editor --lib`
Expected: 327 个既有测试 + 1 个新测试,全部 PASS(0 failed)。

- [ ] **Step 6: 提交**

```bash
git add vendor/iced-code-editor/src/canvas_editor/mod.rs
git commit -m "feat(iced-code-editor): add read_only flag to CodeEditor"
```

---

### Task 2: vendor/iced-code-editor —— `update()` 只读态消息放行清单

**Files:**
- Modify: `vendor/iced-code-editor/src/canvas_editor/update.rs`

**Interfaces:**
- Consumes: `CodeEditor.read_only`(Task 1)。
- Produces: `read_only == true` 时,`update()` 对未放行的消息变体直接返回
  `Task::none()`,不进入原有 match 分支,缓冲区/光标以外的状态(如 `vim_enabled`)也
  不受影响。

**决定:用"允许清单"(allowlist)而非"屏蔽清单"(blocklist)**——`Message` 有 65 个
变体,多数(搜索/跳转行/折叠/IME/上下文菜单自定义项/LSP 跳转……)既不是"编辑类"也不是
单纯"滚动/光标/选区"。设计文档原文措辞是"只放行滚动/光标移动/选区(用于复制)",这是一
个正向清单,不是"拦掉编辑类、其余都放行"。允许清单对 vendor crate 未来新增的消息变体更
安全——新变体默认被拦,不会意外在只读预览里放开一个没设计过的编辑入口。

放行清单(只读态下这些消息正常处理,其余一律早退):
`ArrowKey`、`MouseClick`、`MouseDrag`、`MouseHover`、`MouseRelease`、`DoubleClick`、
`TripleClick`、`SelectAll`、`Copy`、`Scrolled`、`HorizontalScrolled`、`PageUp`、
`PageDown`、`Home`、`End`、`CtrlHome`、`CtrlEnd`、`Tick`、`CanvasFocusGained`、
`CanvasFocusLost`。

- [ ] **Step 1: 写失败测试**

在 `vendor/iced-code-editor/src/canvas_editor/mod.rs` 的 `#[cfg(test)] mod tests` 区块
(与 Task 1 测试相邻)新增两个测试:

```rust
#[test]
fn test_read_only_blocks_character_input() {
    let mut editor = CodeEditor::new("hello", "txt").with_read_only(true);
    let _ = editor.update(&Message::CharacterInput('!'));
    assert_eq!(editor.content(), "hello", "只读态下键入不应修改缓冲区");
}

#[test]
fn test_read_only_allows_arrow_key_and_scroll() {
    let mut editor = CodeEditor::new("hello\nworld", "txt").with_read_only(true);
    let _ = editor.update(&Message::ArrowKey(ArrowDirection::Right, false));
    let (line, col) = editor.cursor_position();
    assert_eq!((line, col), (0, 1), "只读态下光标移动应正常生效");
}
```

(`content()`/`cursor_position()` 是既有公开方法,`vendor/iced-code-editor/src/canvas_editor/mod.rs`
第 1179 行、第 2541 行。)

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p iced-code-editor --lib test_read_only_blocks_character_input test_read_only_allows_arrow_key_and_scroll -- --nocapture`
Expected: FAIL——`test_read_only_blocks_character_input` 断言失败(缓冲区变成了
`"h!ello"` 或类似,因为读写保护还没实现)。

- [ ] **Step 3: 实现**

在 `vendor/iced-code-editor/src/canvas_editor/update.rs` 的 `pub fn update(&mut self,
message: &Message) -> Task<Message>` 函数体最开头(`self.pre_edit_line = ...` 那行之前,
约第 2606 行)插入早退守卫:

```rust
        if self.read_only
            && !matches!(
                message,
                Message::ArrowKey(..)
                    | Message::MouseClick(..)
                    | Message::MouseDrag(..)
                    | Message::MouseHover(..)
                    | Message::MouseRelease
                    | Message::DoubleClick(..)
                    | Message::TripleClick(..)
                    | Message::SelectAll
                    | Message::Copy
                    | Message::Scrolled(..)
                    | Message::HorizontalScrolled(..)
                    | Message::PageUp
                    | Message::PageDown
                    | Message::Home(..)
                    | Message::End(..)
                    | Message::CtrlHome
                    | Message::CtrlEnd
                    | Message::Tick
                    | Message::CanvasFocusGained
                    | Message::CanvasFocusLost
            )
        {
            return Task::none();
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p iced-code-editor --lib test_read_only_blocks_character_input test_read_only_allows_arrow_key_and_scroll`
Expected: PASS

- [ ] **Step 5: 跑整个 vendor crate 测试套件确认没有回归**

Run: `cargo test -p iced-code-editor --lib`
Expected: 全部 PASS(既有 327 个 + Task 1/2 新增 3 个)。

- [ ] **Step 6: 提交**

```bash
git add vendor/iced-code-editor/src/canvas_editor/mod.rs vendor/iced-code-editor/src/canvas_editor/update.rs
git commit -m "feat(iced-code-editor): read-only mode blocks edit messages via allowlist"
```

---

### Task 3: dozer-app —— 主题/语法映射函数从 `workspace.rs` 挪进 `preview.rs`

纯移动重构,为 Task 4 铺路(预览 tab 构造原生 `CodeEditor` 时要用同一套主题/语法映射,
`preview.rs` 不该反向依赖 `workspace.rs`——现状是 `workspace.rs` 已经 `use
crate::preview::{...}`,方向不能倒过来)。这一步不新增测试,靠"移动后现有测试原样通过"
验证没有引入行为变化。

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:583-863`(删除三个函数,改调用点)
- Modify: `crates/dozer-app/src/preview.rs`(新增三个函数)

**Interfaces:**
- Produces: `preview::dozer_editor_style() -> iced_code_editor::theme::Style`、
  `preview::dozer_syntax_theme() -> syntect::highlighting::Theme`、
  `preview::extension_to_syntax(path: &Path) -> String`(均为 `pub(crate) fn`,不再是
  `Workspace` 的关联函数)。

- [ ] **Step 1: 移动函数**

把 `crates/dozer-app/src/workspace.rs` 里 `fn dozer_editor_style()`(约 583-609 行,
含其上的文档注释)、`fn dozer_syntax_theme()`(约 611-715 行)、`fn extension_to_syntax()`
(约 816-863 行)三个函数**原样**(签名不变,内部逻辑一字不改)从 `impl Workspace` 块里
剪切,粘到 `crates/dozer-app/src/preview.rs` 文件末尾(`#[cfg(test)] mod tests` 区块
**之前**),改成模块级自由函数,可见性从(隐式)`Workspace` 私有关联函数改成
`pub(crate) fn`:

```rust
pub(crate) fn dozer_editor_style() -> iced_code_editor::theme::Style {
    // ……原函数体一字不改
}

pub(crate) fn dozer_syntax_theme() -> syntect::highlighting::Theme {
    // ……原函数体一字不改
}

pub(crate) fn extension_to_syntax(path: &std::path::Path) -> String {
    // ……原函数体一字不改
}
```

`preview.rs` 顶部按需补 `use` (`dozer_syntax_theme` 内部用到
`syntect::highlighting::{Color, ScopeSelectors, StyleModifier, ThemeItem}`,已在函数体内
用完整路径/局部 `use`,应该不需要额外顶层 `use`;`dozer_editor_style` 用到
`iced_widget::core::Color` 和 `crate::theme`,后者 `preview.rs` 目前没有 `use crate::theme`,
需要补一行 `use crate::theme;` 或按函数体实际写法补全)。

- [ ] **Step 2: 改调用点**

`crates/dozer-app/src/workspace.rs:730-734`(`preview_edit_open` 内)三处调用从
`Self::extension_to_syntax(&path)`/`Self::dozer_editor_style()`/`Self::dozer_syntax_theme()`
改成 `crate::preview::extension_to_syntax(&path)`/`crate::preview::dozer_editor_style()`/
`crate::preview::dozer_syntax_theme()`。

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 编译通过,无 `unused import`/`dead_code` 新增警告。

- [ ] **Step 4: 跑现有测试套件确认无回归**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 与移动前同样的通过/失败数量(本计划开工前已确认
`terminal_grid_state_sizes_hidden_terminal_as_if_shown`、
`terminal_pane_pixel_size_right_maximized_matches_overlay_box` 两个测试在 `main` 上
本来就失败,与这次改动无关,不用管;其余全部应保持 PASS)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/preview.rs
git commit -m "refactor(preview): move syntax/theme helpers from workspace.rs into preview.rs"
```

---

### Task 4: dozer-app —— `PreviewTab` 原生编辑器字段 + 构造 + `desired_webviews()` 排除

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`

**Interfaces:**
- Consumes: `is_editable_extension(&Path) -> bool`(已有)、
  `preview::extension_to_syntax`/`dozer_editor_style`/`dozer_syntax_theme`(Task 3)、
  `iced_code_editor::CodeEditor::with_read_only`(Task 1)。
- Produces: `PreviewTab.editor: Option<CodeEditor>`、
  `PreviewPane::editor_mut(&mut self, tab_id: usize) -> Option<&mut CodeEditor>`
  (供 Task 6 的事件路由用)、`PreviewPane::active_webview_id()` 改为原生 tab 返回
  `None`。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/preview.rs` 的 `#[cfg(test)] mod tests` 区块新增(需要真实
临时文件,用 `std::env::temp_dir()` + 唯一文件名,仿照 vendor crate 测试风格,写完
测试后自行清理):

```rust
#[test]
fn open_path_builds_native_editor_for_whitelisted_extension_only() {
    let dir = std::env::temp_dir();
    let rs_path = dir.join(format!("preview_native_test_{}.rs", std::process::id()));
    let png_path = dir.join(format!("preview_native_test_{}.png", std::process::id()));
    std::fs::write(&rs_path, "fn main() {}").unwrap();
    std::fs::write(&png_path, [0u8; 4]).unwrap();

    let mut p = PreviewPane::default();
    p.open_path(rs_path.clone());
    p.open_path(png_path.clone());

    assert!(
        p.tabs()[0].editor.is_some(),
        ".rs 扩展名应构造原生 editor"
    );
    assert!(
        p.tabs()[1].editor.is_none(),
        ".png 扩展名不应构造原生 editor,继续走 wry"
    );

    let specs = p.desired_webviews();
    assert_eq!(
        specs.len(),
        1,
        "原生 tab 不应出现在 wry 期望清单里,只剩 .png 那个"
    );
    assert_eq!(
        specs[0].url,
        format!(
            "dozer://flyfish/host.html?p={}",
            encode_component(&png_path.to_string_lossy())
        ),
        "剩下的唯一一条 wry 期望清单条目应该是 .png 那个,URL 编码规则同 flyfish_url"
    );

    std::fs::remove_file(&rs_path).ok();
    std::fs::remove_file(&png_path).ok();
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer open_path_builds_native_editor_for_whitelisted_extension_only -- --nocapture`
Expected: 编译失败(`PreviewTab.editor` 字段不存在)。

- [ ] **Step 3: 实现**

`PreviewTab` 结构体(约第 11-20 行)去掉 `Clone`/`PartialEq` derive(`CodeEditor` 两者都
没实现,派生会编译失败;`TabKind` 自己的 `#[derive(Debug, Clone, PartialEq)]` 不受影响,
留在原处不动),手写一个跳过 `editor` 字段的 `Debug`:

```rust
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    pub reload_nonce: u64,
    /// 仅白名单扩展名(`is_editable_extension`)的文件 tab 有值。非空即代表这个
    /// tab 走原生渲染路径,`desired_webviews()` 据此把它从 wry 期望清单里排除。
    /// `CodeEditor` 没有实现 `Clone`/`PartialEq`,这也是 `PreviewTab` 摘掉这两个
    /// derive 的原因(见下方手写的 `Debug`)。
    pub editor: Option<iced_code_editor::CodeEditor>,
}

impl std::fmt::Debug for PreviewTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTab")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .field("reload_nonce", &self.reload_nonce)
            .field("editor", &self.editor.is_some())
            .finish()
    }
}
```

新增一个只读文件构造 `CodeEditor` 的辅助函数(放在 `flyfish_url` 附近):

```rust
/// 读盘并按白名单扩展名构造一个只读 `CodeEditor`。内容不是合法 UTF-8 时降级
/// 用 lossy 转换(不当错误);其余读取失败(不存在/权限不够等)原样透传
/// `std::io::Error`,调用方(`push_tab`/`bump_reload`)按现有"打开失败"路径
/// 处理,不在这里新增错误类型。
fn read_and_build_native_editor(path: &std::path::Path) -> std::io::Result<iced_code_editor::CodeEditor> {
    let text = std::fs::read_to_string(path).or_else(|e| {
        // 白名单扩展名但内容不是合法 UTF-8:降级用 lossy 转换,不当错误处理
        // (多数文本查看器的通行做法,见设计文档"错误处理"一节)。
        if e.kind() == std::io::ErrorKind::InvalidData {
            std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        } else {
            Err(e)
        }
    })?;
    let mut editor =
        iced_code_editor::CodeEditor::new(&text, &extension_to_syntax(path)).with_read_only(true);
    editor.set_theme(dozer_editor_style());
    editor.set_syntax_theme(dozer_syntax_theme());
    editor.set_font(crate::fonts::code_font());
    editor.set_font_size(theme::font::body() as f32, false);
    Ok(editor)
}
```

`push_tab` 改成:

```rust
fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
    let id = self.next_id;
    self.next_id += 1;
    let editor = match &kind {
        TabKind::File(path) if is_editable_extension(path) => {
            read_and_build_native_editor(path).ok()
        }
        _ => None,
    };
    self.tabs.push(PreviewTab {
        id,
        kind,
        title,
        reload_nonce: 0,
        editor,
    });
    self.active = self.tabs.len() - 1;
    id
}
```

`desired_webviews()` 的 `.map(...)` 前加一道 `.filter(|(_, tab)| tab.editor.is_none())`:

```rust
pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
    self.tabs
        .iter()
        .enumerate()
        .filter(|(_, tab)| tab.editor.is_none())
        .map(|(idx, tab)| {
            // ……函数体其余部分不变
        })
        .collect()
}
```

`active_webview_id()` 改成原生 tab 返回 `None`:

```rust
pub fn active_webview_id(&self) -> Option<usize> {
    self.tabs
        .get(self.active)
        .filter(|t| t.editor.is_none())
        .map(|t| t.id)
}
```

新增按 tab id 取可变 editor 引用的方法(供 Task 6 用):

```rust
/// 按 tab id 取该 tab 的原生 editor 可变引用。tab 不存在或该 tab 走 wry
/// 路径(没有 editor)都返回 `None`。main.rs 的 `Message::PreviewEditorEvent`
/// 桥接器用它把 `iced_code_editor::Message` 转发给正确的 tab。
pub fn editor_mut(&mut self, tab_id: usize) -> Option<&mut iced_code_editor::CodeEditor> {
    self.tabs
        .iter_mut()
        .find(|t| t.id == tab_id)
        .and_then(|t| t.editor.as_mut())
}
```

`open_select_close_tabs`/`reopening_same_file_reuses_tab` 等现有测试用的是**不存在的
临时路径**(如 `/tmp/a.md`),这些路径在测试环境下大概率读不到文件——需要检查这两个
现有测试是否会因为 `push_tab` 现在尝试读盘而改变行为(之前 `push_tab` 完全不碰文件系统)。
若 `read_and_build_native_editor` 读取失败(`/tmp/a.md` 不存在),`.ok()` 让 `editor`
变成 `None`,tab 仍然照常创建——这两个现有测试不会因此失败,但要在 Step 4 实际跑一遍
确认,不能只靠推理。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer open_path_builds_native_editor_for_whitelisted_extension_only`
Expected: PASS

再跑现有测试确认没有回归:

Run: `cargo test -p dozer-app --bin dozer`
Expected: 除了 Task 3 已确认的两个已知无关失败外,全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/preview.rs
git commit -m "feat(preview): native read-only CodeEditor for whitelisted extensions"
```

---

### Task 5: dozer-app —— 外部改动触发原生 tab 刷新

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`

**Interfaces:**
- Consumes: `read_and_build_native_editor`(Task 4,私有,同文件内直接调用)。
- Produces: `bump_reload` 对原生 tab 重建 `editor`;对 wry tab 行为不变(现有
  `reload_nonce` 计数逻辑照旧)。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn bump_reload_rebuilds_native_editor_without_bumping_nonce() {
    let path = std::env::temp_dir().join(format!("preview_reload_test_{}.rs", std::process::id()));
    std::fs::write(&path, "fn one() {}").unwrap();

    let mut p = PreviewPane::default();
    let id = p.open_path(path.clone());
    assert!(p.tabs()[0].editor.is_some());
    let nonce_before = p.tabs()[0].reload_nonce;

    std::fs::write(&path, "fn two() {}").unwrap();
    p.bump_reload(id);

    assert_eq!(
        p.tabs()[0].reload_nonce, nonce_before,
        "原生 tab 的 reload 不该走 reload_nonce 计数(那是 wry URL 换参专用信号)"
    );
    assert!(
        p.tabs()[0].editor.is_some(),
        "reload 后原生 tab 应仍持有(重建后的)editor"
    );

    std::fs::remove_file(&path).ok();
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer bump_reload_rebuilds_native_editor_without_bumping_nonce -- --nocapture`
Expected: FAIL——当前 `bump_reload` 对所有 tab(不分原生/wry)都无条件递增
`reload_nonce`,`nonce_before` 断言会失败。

- [ ] **Step 3: 实现**

`bump_reload` 改成按 tab 是否原生分流:

```rust
pub fn bump_reload(&mut self, tab_id: usize) {
    let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
        return;
    };
    if tab.editor.is_some() {
        // 原生 tab:只读态没有光标/undo 历史值得跨重建保留,直接读盘换新
        // 实例比"原地更新缓冲区"更简单可靠。读取失败保留旧 editor 不动
        // (比闪成空白/丢内容更安全的降级)。
        let TabKind::File(path) = &tab.kind;
        if let Ok(fresh) = read_and_build_native_editor(path) {
            tab.editor = Some(fresh);
        }
    } else {
        tab.reload_nonce += 1;
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer bump_reload_rebuilds_native_editor_without_bumping_nonce`
Expected: PASS

跑一遍 Task 4 里 wry 那半边的既有测试确认没有回归:

Run: `cargo test -p dozer-app --bin dozer bump_reload_appends_query_param_and_only_affects_target_tab`
Expected: PASS(这个测试用的路径不在白名单扩展名里,应该继续走 `reload_nonce` 分支,
不受影响——如果测试用的是白名单扩展名路径,需要先确认它的固定测试路径,若确实是白名单
扩展名则这个测试需要跟着改成断言"不递增 nonce",而是在 Step 1 就该发现这一点;跑到这里
如果失败,回 Step 1 把两个测试的路径改成不冲突的扩展名组合)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/preview.rs
git commit -m "feat(preview): bump_reload rebuilds native editor tabs from disk"
```

---

### Task 6: dozer-app —— 渲染原生 editor + 键盘/剪贴板事件桥接

这是这次改动里最大的一步:原生 tab 需要真的画出来,且要能收滚动/光标/复制事件(Task 2
已经放行的那个消息子集)。`iced_code_editor::Message` 走剪贴板需要 `main.rs` 里持有
`Clipboard` 句柄的 Task 桥接器(`run_editor_task`,现在只服务编辑弹层的单一
`edit_session`),这一步要给原生预览 tab 加一条平行路径。

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(新增 `Message` 变体、`App` 方法、渲染分支)
- Modify: `crates/dozer-app/src/workspace.rs`(新增 `preview_tab_editor_event`、
  `preview_pane()` 渲染分支)
- Modify: `crates/dozer-app/src/main.rs`(新增桥接函数、`dispatch` 分支)

**Interfaces:**
- Consumes: `PreviewPane::editor_mut`(Task 4)、`iced_code_editor::CodeEditor::view(&self)
  -> Element<'_, iced_code_editor::Message>`(vendor crate 既有 API,
  `vendor/iced-code-editor/src/canvas_editor/view.rs:361`;`workspace.rs:2389` 的编辑
  弹层已经用同一个方法这样调用:`session.editor.view().map(Message::EditorEvent)`,这次
  是同一个模式,只是 map 进 `PreviewEditorEvent(tab_id, ev)`)。
- Produces: `Message::PreviewEditorEvent(usize, iced_code_editor::Message)`、
  `Workspace::preview_tab_editor_event(&mut self, tab_id: usize, event:
  iced_code_editor::Message) -> iced_winit::runtime::Task<iced_code_editor::Message>`、
  `App::preview_tab_editor_event(&mut self, tab_id: usize, event:
  iced_code_editor::Message) -> iced_winit::runtime::Task<iced_code_editor::Message>`。

- [ ] **Step 1: `Workspace::preview_tab_editor_event`**

在 `crates/dozer-app/src/workspace.rs`,紧挨着现有 `preview_edit_event`(约第 760-768
行)新增:

```rust
/// 转发 `iced-code-editor` 的内部消息到某个原生预览 tab(按 `tab_id` 定位,
/// 不是"当前聚焦编辑弹层"——一个项目可以同时开好几个原生预览 tab,只有
/// 事件来源的那一个该收到)。tab 不存在或不是原生 tab 时静默 no-op。
pub(crate) fn preview_tab_editor_event(
    &mut self,
    tab_id: usize,
    event: EditorMessage,
) -> iced_winit::runtime::Task<EditorMessage> {
    match self.preview.editor_mut(tab_id) {
        Some(editor) => editor.update(&event),
        None => iced_winit::runtime::Task::none(),
    }
}
```

- [ ] **Step 2: `App::preview_tab_editor_event`**

在 `crates/dozer-app/src/app.rs`,紧挨着现有 `preview_edit_event`(约第 2185-2194 行)
新增:

```rust
/// 转发到聚焦项目里某个原生预览 tab 的 editor,语义同 `preview_edit_event`
/// 但按 `tab_id` 定位而不是"当前编辑弹层"。`Message::PreviewEditorEvent` 经
/// `main.rs::dispatch` 进入后走到这里。
pub fn preview_tab_editor_event(
    &mut self,
    tab_id: usize,
    event: iced_code_editor::Message,
) -> iced_winit::runtime::Task<iced_code_editor::Message> {
    if let Some(ws) = self.active_workspace_mut() {
        ws.preview_tab_editor_event(tab_id, event)
    } else {
        iced_winit::runtime::Task::none()
    }
}
```

在 `Message` 枚举(`crates/dozer-app/src/app.rs`,紧挨着 `EditorEvent` 变体,约第 1025
行)新增:

```rust
/// 原生预览 tab 的 `iced-code-editor` 内部消息,`usize` 是 `PreviewTab.id`。
/// 与 `EditorEvent`(编辑弹层专用)是两条独立路径,互不路由串台——见
/// `preview_tab_editor_event` 的文档。
PreviewEditorEvent(usize, iced_code_editor::Message),
```

- [ ] **Step 3: `main.rs` 桥接**

在 `crates/dozer-app/src/main.rs`,`fn run_editor_task`(约第 1113 行)紧接着新增一个
平行函数(内容与 `run_editor_task` 完全一致,唯一区别是调用
`app.preview_tab_editor_event(tab_id, ev)` 而不是 `app.preview_edit_event(ev)`):

```rust
/// 同 `run_editor_task`,但把消息转发给某个原生预览 tab(按 `tab_id`)而不是
/// 编辑弹层的单一 `edit_session`。两个函数体基本重复——保持"预览/编辑分层"
/// 这条既定决策(见设计文档),不引入一个把两种目标都塞进同一签名的抽象。
fn run_preview_tab_editor_task(
    app: &mut App,
    clipboard: &mut Clipboard,
    tab_id: usize,
    event: iced_code_editor::Message,
) {
    use iced_winit::futures::futures::stream::StreamExt;
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(event);
    while let Some(ev) = queue.pop_front() {
        let t = app.preview_tab_editor_event(tab_id, ev);
        let Some(stream) = task::into_stream(t) else {
            continue;
        };
        let mut outputs: Vec<iced_code_editor::Message> = Vec::new();
        futures::futures::executor::block_on(async {
            let mut stream = stream;
            while let Some(action) = stream.next().await {
                match action {
                    Action::Output(m) => outputs.push(m),
                    Action::Clipboard(cb) => match cb {
                        ClipboardAction::Read { target, channel } => {
                            let text = clipboard.read(target);
                            let _ = channel.send(text);
                        }
                        ClipboardAction::Write { target, contents } => {
                            clipboard.write(target, contents);
                        }
                    },
                    _ => {}
                }
            }
        });
        for m in outputs {
            queue.push_back(m);
        }
    }
}
```

在 `dispatch` 函数(约第 1064-1072 行,`Message::EditorEvent(event) => {...}` 分支
紧邻处)新增:

```rust
Message::PreviewEditorEvent(tab_id, event) => {
    Self::run_preview_tab_editor_task(app, clipboard, tab_id, event);
    window.request_redraw();
}
```

- [ ] **Step 4: 渲染**

在 `crates/dozer-app/src/workspace.rs` 的 `preview_pane()` 函数(约第 2320-2336 行),
`if ws.preview.tabs().is_empty() { ... }` 分支**之后**新增一个 `else` 分支,渲染当前
激活 tab 的原生 editor(若有):

```rust
if !ws.preview.tabs().is_empty() {
    let active_tab = &ws.preview.tabs()[ws.preview.active_idx()];
    if let Some(editor) = &active_tab.editor {
        let tab_id = active_tab.id;
        content = content.push(
            container(editor.view().map(move |ev| Message::PreviewEditorEvent(tab_id, ev)))
                .width(Length::Fill)
                .height(Length::Fill),
        );
    }
    // `editor` 为 `None` 时(wry 路由的 tab)不 push 任何 iced 元素——
    // 那片区域由 `main.rs` 定位的 wry webview 子视图负责渲染,现状不变。
}
```

- [ ] **Step 5: 编译 + 现有测试确认**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

Run: `cargo test -p dozer-app --bin dozer`
Expected: 除 Task 3 已知的两个无关失败外全部 PASS。

- [ ] **Step 6: 人工验收**(GUI 变更,自动化测试覆盖不到渲染/焦点/剪贴板的端到端行为)

跑起真实 app:`cargo run -p dozer-app`。

1. 打开一个 `.rs` 文件预览,确认内容原生渲染(语法高亮、行号、ByteBoy2077 配色),
   且**不会**在 wry `webviews` 池里出现——可以临时在 `sync_webview_pool` 里加一行
   `tracing::debug!("webviews: {:?}", webviews.keys())` 观察,验收完删掉。
2. 该 tab 打开状态下,在文件树上右键,确认菜单正常显示在最上层、预览内容不消失、不
   被裁剪——这是触发这整个阶段性改动的原始 bug 的直接回归验证。
3. 在原生预览里滚动、用方向键移动光标、拖拽选中一段文字、⌘C 复制,粘贴到别处确认内容
   正确;尝试键入字符,确认**不会**修改预览内容。
4. 打开一张图片和一个 PDF,确认两者仍走 wry 路径、⌘K 隐藏预览的既有行为不受影响。
5. 点击原生预览 tab 右键菜单的"编辑",确认弹层正常打开(用的是另一个独立
   `CodeEditor` 实例,`edit_session`);编辑后保存,确认原生预览 tab 内容跟着刷新
   (`bump_reload` 路径,Task 5)。
6. 打开一个体量较大的文件(比如项目里最大的一个 `.log` 或生成产物),观察原生渲染是否
   有明显卡顿——若确实卡顿,记录现象但不在这次任务里解决(设计文档"非目标"已明确这是
   YAGNI,留到发现问题时再处理)。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(preview): render native editor for whitelisted tabs, bridge clipboard/scroll events"
```

---

### Task 7: 全量回归 + 收尾

**Files:** 无新改动,纯验证。

- [ ] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build
cargo test -p dozer-app --bin dozer
cargo test -p iced-code-editor --lib
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部通过(`cargo test -p dozer-app --bin dozer` 允许 Task 3 已确认的两个
既存无关失败;其余三个命令必须全绿,`cargo fmt --check` 若有差异跑 `cargo fmt` 后
重新 `git add`/提交)。

- [ ] **Step 2: 核对设计文档验收清单**

对照 `docs/superpowers/specs/2026-08-12-native-text-preview-design.md` "测试策略"一节
的 4 条人工验收项(已在 Task 6 Step 6 做过),确认全部通过,没有遗漏。

- [ ] **Step 3: 更新设计文档状态**(可选,若这次实现过程中发现了设计文档需要订正的地方)

若 Task 1-6 执行过程中发现设计文档与实际实现有出入(比如 vendor crate 某个 API 名字
跟设计文档假设的不一样),回 spec 文件补一句"实现备注",不要让 spec 与代码长期不一致。

- [ ] **Step 4: 最终提交**

若 Step 1 的 `cargo fmt` 产生了改动:

```bash
git add -A
git commit -m "chore: cargo fmt after native text preview implementation"
```
