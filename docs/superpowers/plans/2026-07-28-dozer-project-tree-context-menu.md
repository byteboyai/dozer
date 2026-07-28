# 项目树右键菜单实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 项目树目录/文件行加右键菜单：新建文件/新建文件夹/复制/粘贴/删除/重命名/复制绝对路径/复制相对路径（文件行没有新建/粘贴）。

**Architecture:** 菜单是 `iced_widget::stack!` 叠出的浮层，位置由右键点击的像素坐标手算 padding 定位（与本仓 `ime_cursor_area`/`preview_content_bounds` 同款手算风格）。右键坐标经 `main.rs` 已有的原始 `WindowEvent` 拦截层（本来就在给分隔线拖拽做同样的事）捕获；具体点了哪一行走 `iced_widget::MouseArea::on_right_press` 的正常 iced 分发。新建/重命名走行内自绘编辑框（与地址栏/验收意见框同款，不用原生 `text_input`）。删除走 `trash` crate 移入系统回收站。复制/粘贴是应用内剪贴槽（不碰 OS 剪贴板），复制路径才真写系统剪贴板。

**Tech Stack:** Rust, iced 0.14（`iced_widget::stack!`/`MouseArea::on_right_press`/`Padding`），新依赖 `trash`（跨平台回收站）。

## Global Constraints

- 颜色只取自 `crates/dozer-app/src/theme.rs`，禁止新增硬编码色值。
- 每 task 收尾 `cargo clippy -p dozer-app --all-targets` clean、`cargo fmt -p dozer-app -- --check` 干净、`cargo test -p dozer-app` 绿。
- `Workspace::update` 里凡触碰磁盘 IO 一律 `self.handle.spawn(async move { tokio::task::spawn_blocking(...).await; proxy.send_event(...) })`，UI 线程绝不同步阻塞——这是本仓已有的既定模式（`spawn_project_git_refresh`/`spawn_conversations_refresh` 都这么写），本计划的粘贴/删除/新建/重命名全部照抄这个模式。
- iced 0.14 API 若与本计划字面不符，按编译器提示与本仓既有用法适配。
- 命名冲突（粘贴/重命名/新建）不静默覆盖、不自动改名，行内红字报错，编辑框/操作保持可重试状态。
- 设计依据：`docs/superpowers/specs/2026-07-28-project-tree-context-menu-design.md`。

---

### Task 1: `FileTree` 纯逻辑扩展（刷新缓存 + 目录递归复制 + 路径字符串）

**Files:**
- Modify: `crates/dozer-app/src/project.rs`（新增 `refresh`/`ensure_expanded`/`copy_dir_recursive`/`paste_item`/`path_string` + 测试）

**Interfaces:**
- Produces:
  - `impl FileTree { pub fn refresh(&mut self, dir: &Path); pub fn ensure_expanded(&mut self, dir: &Path); }`
  - `pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()>`
  - `pub fn paste_item(source: &Path, source_is_dir: bool, target_dir: &Path) -> Result<PathBuf, String>`（返回值=新建出的目标路径，供调用方知道该刷新哪个目录/给成功文案用）
  - `pub enum PathKind { Absolute, Relative }`（`#[derive(Debug, Clone, Copy, PartialEq)]`）
  - `pub fn path_string(kind: PathKind, path: &Path, project_root: &Path) -> String`

这四个函数/方法本任务内暂无调用方（Task 2-4 才用到），会被 `cargo build` 的 `dead_code` lint 标记——即使是 `pub fn`，本 crate 是纯 `[[bin]]`（无 `[lib]` target），`pub` 不豁免 dead_code（P1L 分支同款问题已验证过）。本任务对它们逐个加 `#[allow(dead_code)]` + 注明"留给 Task N"，后续任务接上真实调用点时移除。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/project.rs` 的 `#[cfg(test)] mod tests`（文件末尾）里，`use super::*;` 之后追加：

```rust
    #[test]
    fn refresh_picks_up_new_files_on_disk() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src")); // 展开,缓存 src 的子项(此时只有 main.rs)
        std::fs::write(d.path().join("src/lib.rs"), "").unwrap();
        // 磁盘变了,refresh 前 visible_rows 还是旧缓存
        let before: Vec<_> = t.visible_rows().iter().map(|r| r.name.clone()).collect();
        assert!(!before.contains(&"lib.rs".to_string()));
        t.refresh(&d.path().join("src"));
        let after: Vec<_> = t.visible_rows().iter().map(|r| r.name.clone()).collect();
        assert!(after.contains(&"lib.rs".to_string()));
    }

    #[test]
    fn ensure_expanded_is_idempotent_on_already_expanded() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src"));
        assert_eq!(t.visible_rows().len(), 3); // src, main.rs, README.md
        t.ensure_expanded(&d.path().join("src")); // 已展开,应无副作用
        assert_eq!(t.visible_rows().len(), 3);
    }

    #[test]
    fn ensure_expanded_expands_a_collapsed_dir() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        assert_eq!(t.visible_rows().len(), 2); // src(收起), README.md
        t.ensure_expanded(&d.path().join("src"));
        assert_eq!(t.visible_rows().len(), 3); // src, main.rs, README.md
    }

    #[test]
    fn ensure_expanded_on_empty_dir_stays_collapsed() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("empty")).unwrap();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.ensure_expanded(&d.path().join("empty")); // 空目录,toggle 内部不标 expanded
        let row = t
            .visible_rows()
            .into_iter()
            .find(|r| r.name == "empty")
            .unwrap();
        assert!(!row.expanded);
    }

    #[test]
    fn copy_dir_recursive_copies_nested_structure() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src_dir");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.txt"), "A").unwrap();
        std::fs::create_dir(src.join("nested")).unwrap();
        std::fs::write(src.join("nested/b.txt"), "B").unwrap();

        let dst = d.path().join("dst_dir");
        copy_dir_recursive(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "A");
        assert_eq!(
            std::fs::read_to_string(dst.join("nested/b.txt")).unwrap(),
            "B"
        );
    }

    #[test]
    fn copy_dir_recursive_refuses_existing_target() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src_dir");
        std::fs::create_dir(&src).unwrap();
        let dst = d.path().join("dst_dir");
        std::fs::create_dir(&dst).unwrap(); // 目标已存在
        let result = copy_dir_recursive(&src, &dst);
        assert!(result.is_err());
    }

    #[test]
    fn paste_item_copies_file_into_target_dir() {
        let d = tempfile::tempdir().unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let target_dir = d.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();

        let result = paste_item(&src_file, false, &target_dir).unwrap();
        assert_eq!(result, target_dir.join("a.txt"));
        assert_eq!(std::fs::read_to_string(&result).unwrap(), "hello");
    }

    #[test]
    fn paste_item_rejects_name_collision() {
        let d = tempfile::tempdir().unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let target_dir = d.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();
        std::fs::write(target_dir.join("a.txt"), "existing").unwrap(); // 已存在同名

        let result = paste_item(&src_file, false, &target_dir);
        assert!(result.is_err());
        // 冲突不覆盖:目标文件内容应保持原样
        assert_eq!(
            std::fs::read_to_string(target_dir.join("a.txt")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn path_string_absolute_is_full_path() {
        let root = Path::new("/repo");
        let p = Path::new("/repo/src/main.rs");
        assert_eq!(path_string(PathKind::Absolute, p, root), "/repo/src/main.rs");
    }

    #[test]
    fn path_string_relative_strips_root() {
        let root = Path::new("/repo");
        let p = Path::new("/repo/src/main.rs");
        assert_eq!(path_string(PathKind::Relative, p, root), "src/main.rs");
    }

    #[test]
    fn path_string_relative_falls_back_to_absolute_when_not_under_root() {
        let root = Path::new("/repo");
        let p = Path::new("/other/file.rs");
        assert_eq!(
            path_string(PathKind::Relative, p, root),
            "/other/file.rs"
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib project::tests::refresh_picks_up_new_files_on_disk`
Expected: FAIL（`refresh` 方法未定义，编译错误）。

- [ ] **Step 3: 实现**

在 `crates/dozer-app/src/project.rs` 顶部 `use` 区（`use std::path::{Path, PathBuf};` 之后）新增：

```rust
/// 复制路径展示形式：绝对路径 vs 相对项目根目录。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathKind {
    Absolute,
    Relative,
}
```

在 `impl FileTree {` 块内，`toggle` 方法之后（`push_rows` 之前）新增：

```rust
    /// 强制重读一个目录的子项缓存（增删改后调用，让 `visible_rows()`
    /// 反映最新磁盘状态）。目录本身若已展开保持展开；未展开的话，本次
    /// 调用不强行展开，只刷新缓存——下次展开时自然是最新的。
    #[allow(dead_code)] // 留给 Task 3(粘贴)/Task 4(删除)/Task 5(重命名)/Task 6(新建)
    pub fn refresh(&mut self, dir: &Path) {
        self.children.insert(dir.to_path_buf(), read_children(dir));
    }

    /// 确保目录处于展开态（新建文件/文件夹前调用，让新项有可见位置）。
    #[allow(dead_code)] // 留给 Task 6(新建文件/文件夹)
    pub fn ensure_expanded(&mut self, dir: &Path) {
        if !self.expanded.contains(dir) {
            self.toggle(dir); // toggle 内部已处理"空目录不标 expanded"
        }
    }
```

在文件末尾（`impl FileTree` 块之后、`#[cfg(test)]` 之前）新增：

```rust
/// 递归复制目录（文件树"粘贴"目标是目录时用）。目标已存在 → `Err`，不覆盖、
/// 不做部分复制后中止的脏状态清理（失败时目标目录可能已建但不完整——
/// 调用方(`paste_item`)已在动手前检查过存在性，这里的检查是防御递归过程中
/// 子目录层面的二次冲突）。
#[allow(dead_code)] // 留给 Task 3(粘贴)
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} 已存在", dst.display()),
        ));
    }
    std::fs::create_dir(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 把 `source`（文件或目录）复制进 `target_dir` 下，用源的文件名。目标已
/// 存在同名项 → `Err`（冲突文案）。成功返回新建出的完整路径。
#[allow(dead_code)] // 留给 Task 3(粘贴)
pub fn paste_item(
    source: &Path,
    source_is_dir: bool,
    target_dir: &Path,
) -> Result<PathBuf, String> {
    let dest = target_dir.join(source.file_name().unwrap_or_default());
    if dest.exists() {
        return Err(format!("{} 已存在同名项", dest.display()));
    }
    let result = if source_is_dir {
        copy_dir_recursive(source, &dest)
    } else {
        std::fs::copy(source, &dest).map(|_| ())
    };
    result.map(|_| dest).map_err(|e| e.to_string())
}

/// 路径的展示字符串：绝对路径原样；相对路径去掉 `project_root` 前缀，
/// 若不在 `project_root` 之下（理论上树里的项恒在其下，此分支是防御性
/// 兜底）就退化成绝对路径。
#[allow(dead_code)] // 留给 Task 2(右键菜单"复制绝对/相对路径")
pub fn path_string(kind: PathKind, path: &Path, project_root: &Path) -> String {
    match kind {
        PathKind::Absolute => path.display().to_string(),
        PathKind::Relative => path
            .strip_prefix(project_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string()),
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib project::`
Expected: 全绿（含新增 10 个测试）。

- [ ] **Step 5: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译通过 / clean / 无输出。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/project.rs
git commit -m "feat(tree-context-menu): FileTree 刷新/展开 + 目录递归复制 + 路径字符串(纯逻辑,Task2-6接线)"
```

---

### Task 2: 右键菜单外壳（定位/开关/复制路径两项）

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`（新增 `trash` 依赖，为 Task 4 预置——本任务不用，但一次性加好，避免后续任务再碰 `Cargo.toml`）
- Modify: `crates/dozer-app/src/main.rs`（`mod` 无需改；`on_window_event` 新增右键坐标捕获 + Esc 关菜单；`dispatch()` 新增 `ProjectTreeCopyPath` 拦截）
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增 6 个变体；`Workspace` 新增 `context_menu`/`last_right_click` 字段；`update()` 新增对应分支；`view()` 包一层 `stack!`；tree row 包 `MouseArea::on_right_press`；新增菜单渲染函数）

**Interfaces:**
- Consumes: Task 1 的 `project::PathKind`/`project::path_string`。
- Produces: `struct ContextMenu { x: f32, y: f32, target: PathBuf, is_dir: bool }`（私有，`workspace.rs` 内部）；`Message::RightClickAt`/`ProjectTreeContextMenu`/`ProjectTreeContextMenuClose`/`ProjectTreeCopyPath`；`Workspace::dragging_divider`-同款的新 getter（若菜单渲染需要，见 Step 中说明）。

- [ ] **Step 1: 加 `trash` 依赖**

Run: `cd crates/dozer-app && cargo add trash`
Expected: `Cargo.toml` 新增一行 `trash = "..."`（具体版本号由 cargo 解析当前可用最新版，不用手填）；`Cargo.lock` 相应更新。

Run: `cargo build -p dozer-app`
Expected: 编译通过（本任务还不调用 `trash` 的任何 API，只是把依赖加上，Task 4 才用）。

- [ ] **Step 2: `Message` 新增变体**

在 `crates/dozer-app/src/workspace.rs` 的 `pub enum Message {` 里，紧邻 `AcceptanceCountLoaded(Option<u64>),`（枚举最后一个变体）之后新增：

```rust
    /// 项目树:右键按下的窗口逻辑坐标(main.rs 原始事件层发,供随后可能
    /// 触发的 `ProjectTreeContextMenu` 定位弹出菜单)。
    RightClickAt { x: f32, y: f32 },
    /// 项目树:某行右键命中,打开菜单(位置取 `last_right_click`)。
    ProjectTreeContextMenu { path: PathBuf, is_dir: bool },
    /// 项目树:关闭菜单(点击外部/Esc/动作完成后)。
    ProjectTreeContextMenuClose,
    /// 项目树:菜单选"复制绝对/相对路径"→ main.rs 拦截写系统剪贴板,
    /// 不落 `Workspace::update`(同 `PreviewPickFile` 模式)。
    ProjectTreeCopyPath(PathBuf, project::PathKind),
```

（其余 11 个 `ProjectTree*` 变体——新建/重命名/复制/粘贴/删除相关——留给 Task 3-6 逐个加，本任务只加这 4 个。）

`workspace.rs` 顶部 `use crate::project::FileTree;` 改为：

```rust
use crate::project::{self, FileTree};
```

（`project::PathKind` 需要这个 `self` 引入才能在 `Message` 变体里写 `project::PathKind`。）

- [ ] **Step 3: `Workspace` 新增状态**

在 `pub struct Workspace {` 内，紧邻 `dragging: Option<Divider>,`（最后一个字段）之后新增：

```rust
    /// 项目树右键菜单当前打开状态(None=未打开)。
    context_menu: Option<ContextMenu>,
    /// 最近一次右键点击的窗口逻辑坐标,给 `ProjectTreeContextMenu` 定位菜单用。
    last_right_click: (f32, f32),
```

在 `pub struct Workspace {` 定义之前（紧邻 `PanelLayout`/`Divider` 定义附近，或任何模块级位置皆可，建议紧邻 `Divider` 枚举之后）新增：

```rust
/// 项目树右键菜单当前打开状态：定位坐标 + 目标（路径/是否目录）。
#[derive(Debug, Clone, PartialEq)]
struct ContextMenu {
    x: f32,
    y: f32,
    target: PathBuf,
    is_dir: bool,
}
```

`bootstrap`/`with_daemon_error` 两处构造字面量，紧邻 `dragging: None,` 之后新增：

```rust
            context_menu: None,
            last_right_click: (0.0, 0.0),
```

- [ ] **Step 4: `update()` 新增分支**

在 `update()` 的 `match message {` 里，紧邻 `Message::AcceptanceCountLoaded(n) => { self.project_acceptance_count = n; }`（最后一个分支）之前新增：

```rust
            Message::RightClickAt { x, y } => {
                self.last_right_click = (x, y);
            }
            Message::ProjectTreeContextMenu { path, is_dir } => {
                let (x, y) = self.last_right_click;
                self.context_menu = Some(ContextMenu {
                    x,
                    y,
                    target: path,
                    is_dir,
                });
            }
            Message::ProjectTreeContextMenuClose => {
                self.context_menu = None;
            }
            Message::ProjectTreeCopyPath(_, _) => {} // 副作用在 main.rs(写系统剪贴板需 Clipboard 句柄)
```

新增一个只读 getter（紧邻 `dragging_divider` 之后，供 `main.rs` 判断 Esc 是否该关菜单）：

```rust
    /// 项目树右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn context_menu_open(&self) -> bool {
        self.context_menu.is_some()
    }
```

- [ ] **Step 5: 菜单渲染 + `view()` 接线**

在 `workspace.rs` 顶部 `use iced_widget::core::{Border, Color, Element, Length};` 改为：

```rust
use iced_widget::core::{Border, Color, Element, Length, Padding};
```

`use iced_widget::{MouseArea, button, column, container, row, text};` 改为：

```rust
use iced_widget::{MouseArea, button, column, container, row, stack, text};
```

新增菜单项按钮的小助手（放在 `divider_bar` 函数之后即可）：

```rust
/// 右键菜单一项:纯文字按钮,CARD 底+BORDER 描边悬停态由 iced 默认
/// button 交互色处理(本仓其余按钮同款,不额外定制)。
fn menu_item<'a>(label: &'static str, msg: Message) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(text(label).size(13).color(theme::CREAM))
        .on_press(msg)
        .width(Length::Fixed(180.0))
        .padding([6, 10])
        .style(|_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            ..button::Style::default()
        })
        .into()
}

/// 右键菜单浮层本体:纵向按钮列表,`container` 用 `Padding{top,left,..}`
/// 手算定位到点击坐标——`Stack` 各层共享同一份 bounds,不像原生系统菜单
/// 那样自带绝对定位,这是本仓一贯的手算像素定位风格(`ime_cursor_area`/
/// `preview_content_bounds` 同款)。
fn context_menu_popup(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(menu) = &ws.context_menu else {
        return column![].into();
    };
    let project_root = ws
        .project
        .as_ref()
        .map(|p| PathBuf::from(&p.path))
        .unwrap_or_default();
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
        Vec::new();
    items.push(menu_item(
        "复制绝对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Absolute),
    ));
    items.push(menu_item(
        "复制相对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Relative),
    ));
    let _ = project_root; // Task 3-6 会用到;本任务先只有这两项路径复制不需要它

    let list = container(column(items).spacing(2))
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

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

把 `view()`（约在 `terminal_pane`/`ai_pane` 定义之后的 `impl Workspace` 块内）：

```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        let top = top_bar(self);
        let col1 = project_pane(self);
        let col2 = preview_pane(self);
        let col3 = terminal_pane(self);
        let col4 = ai_pane(self);
        column![
            top,
            row![col1, divider_bar(Divider::ProjectPreview), col2, divider_bar(Divider::PreviewTerminal), col3, divider_bar(Divider::TerminalAi), col4]
        ]
        .into()
    }
```

改为：

```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        let top = top_bar(self);
        let col1 = project_pane(self);
        let col2 = preview_pane(self);
        let col3 = terminal_pane(self);
        let col4 = ai_pane(self);
        let base = column![
            top,
            row![col1, divider_bar(Divider::ProjectPreview), col2, divider_bar(Divider::PreviewTerminal), col3, divider_bar(Divider::TerminalAi), col4]
        ];

        if self.context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeContextMenuClose);
            stack![base, dismiss, context_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base.into()
        }
    }
```

（`context_menu.is_some()` 判断放在 `view()` 而不是让 `context_menu_popup`/`dismiss` 永远存在——菜单关闭时 stack 完全退化回三栏原样，不会有透明层吞掉正常点击。）

- [ ] **Step 6: tree row 包 `MouseArea::on_right_press`**

把 `project_pane` 里（约在 `if let Some(tree) = &ws.file_tree {` 循环内）：

```rust
                    content = content.push(button(line).on_press(msg).width(Length::Fill).style(
                        |_t, _s| button::Style {
                            background: None,
                            text_color: theme::BODY,
                            ..button::Style::default()
                        },
                    ));
```

改为：

```rust
                    let row_btn = button(line).on_press(msg).width(Length::Fill).style(
                        |_t, _s| button::Style {
                            background: None,
                            text_color: theme::BODY,
                            ..button::Style::default()
                        },
                    );
                    content = content.push(
                        MouseArea::new(row_btn)
                            .on_right_press(Message::ProjectTreeContextMenu {
                                path: row.path.clone(),
                                is_dir: row.is_dir,
                            })
                            .into(),
                    );
```

- [ ] **Step 7: `main.rs` 右键坐标捕获**

在 `on_window_event` 的 `match event {` 里，紧邻 `MouseInput { state: Pressed, button: Left, .. }` 分支之后（`_ => {}` 之前）新增：

```rust
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: winit::event::MouseButton::Right,
                    ..
                } => {
                    let scale = window.scale_factor();
                    let x = (cursor_phys.x / scale) as f32;
                    let y = (cursor_phys.y / scale) as f32;
                    workspace.update(Message::RightClickAt { x, y });
                    // 不在这里 request_redraw——右键若真的命中某行,该行的
                    // `MouseArea::on_right_press` 随本轮事件走 iced 正常分发,
                    // 那条路径自会触发重绘;若点在空白处,菜单本就不该开,不必
                    // 额外重绘。
                }
```

- [ ] **Step 8: `main.rs` Esc 关闭菜单**

在 `on_window_event` 里找到键盘事件处理的最外层分发点（`let bytes = match event { ... }`，处理常规按键→PTY 字节的地方，在"地址栏/验收意见编辑态"块之后、"⌘ 组合键"块的后面）。在该函数顶部——所有其它分支之前——新增一个提前拦截（放在"地址栏/验收意见编辑态"那个 `if to_preview || to_comment { ... return; }` 块之前，与它同级）：

```rust
            // 右键菜单打开时,Esc 优先关菜单,不进正常键盘分发(不然会被当作
            // 普通按键继续往下走,可能被地址栏/终端等其它分支消费掉)。
            if workspace.context_menu_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                workspace.update(Message::ProjectTreeContextMenuClose);
                window.request_redraw();
                return;
            }
```

（插入位置：紧邻 Step 7 新增的右键 `MouseInput` 分支所在的 `match event { ... }` 块**之后**、"⌘ 组合键"检查**之前**——即 `on_window_event` 函数体中部，两个既有大块之间。）

- [ ] **Step 9: `main.rs` `dispatch()` 拦截 `ProjectTreeCopyPath`**

把 `dispatch()` 里的：

```rust
            match message {
                Message::PreviewPickFile => {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        workspace.update(Message::PreviewOpenPath(path));
                    }
                }
                Message::ProjectPickFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        workspace.update(Message::ProjectOpen(dir));
                    }
                }
                other => workspace.update(other),
            }
```

改为：

```rust
            match message {
                Message::PreviewPickFile => {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        workspace.update(Message::PreviewOpenPath(path));
                    }
                }
                Message::ProjectPickFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        workspace.update(Message::ProjectOpen(dir));
                    }
                }
                Message::ProjectTreeCopyPath(path, kind) => {
                    let root = workspace
                        .active_project_path()
                        .unwrap_or_else(|| path.clone());
                    let s = workspace::project::path_string(kind, &path, &root);
                    clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
                    workspace.update(Message::ProjectTreeContextMenuClose);
                    window.request_redraw();
                }
                other => workspace.update(other),
            }
```

这里用到 `workspace.active_project_path()`——若尚不存在这个方法，在 `workspace.rs` 的 `impl Workspace` 里新增（放在 `context_menu_open` 附近即可）：

```rust
    /// 当前项目根路径(供 main.rs 算相对路径用;未打开项目时 None)。
    pub fn active_project_path(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| PathBuf::from(&p.path))
    }
```

`crates/dozer-app/src/main.rs` 顶部若尚未 `pub` 暴露 `project` 子模块的路径给 `workspace::project::path_string` 这个引用方式，改用完整路径 `crate::project::path_string` 更符合本仓惯例（`main.rs` 里其余对 `workspace` 内部类型/函数的引用都是 `workspace::xxx` 形式，`project` 模块本身在 `main.rs` 顶部已有 `mod project;` 声明，直接 `crate::project::path_string(...)` 即可，不必绕经 `workspace::project`）：把上面 `dispatch()` 新增代码里的 `workspace::project::path_string` 改成 `crate::project::path_string`，并确认 `crate::project::PathKind`/`path_string` 是 `pub`（Task 1 已经是 `pub`）。

- [ ] **Step 10: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译/测试通过；clean；无输出。若 `project::path_string`/`PathKind` 仍报 dead_code（因为本任务已经是它们的真实调用方，Task 1 的 `#[allow(dead_code)]` 理应可以摘掉）——摘掉 `path_string`/`PathKind` 上 Task 1 加的 `#[allow(dead_code)]`（`refresh`/`ensure_expanded`/`copy_dir_recursive`/`paste_item` 仍留着，本任务不消费它们）。

- [ ] **Step 11: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：在文件树任意行右键，弹出菜单（此时只有"复制绝对路径"/"复制相对路径"两项，正常——其余项 Task 3-6 才加）；点其中一项，菜单关闭；到别处（比如备忘录/终端）⌘V 粘贴，应粘出预期的路径字符串；点菜单外部或按 Esc 关闭菜单不触发任何动作。

- [ ] **Step 12: Commit**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(tree-context-menu): 右键菜单外壳(定位/开关/Esc) + 复制绝对/相对路径两项"
```

---

### Task 3: 复制 / 粘贴

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增 3 个变体；`Workspace` 新增 `tree_clipboard`/`tree_error` 字段；`update()` 新增分支；菜单加"复制"/"粘贴"两项；tree 视图渲染 `tree_error`）

**Interfaces:**
- Consumes: Task 1 的 `project::paste_item`；Task 2 的 `ContextMenu`/菜单外壳/`menu_item`。
- Produces: `Message::ProjectTreeCopy(PathBuf, bool)` / `ProjectTreePaste(PathBuf)` / `ProjectTreePasteDone(Result<PathBuf, String>)`。

- [ ] **Step 1: `Message`/`Workspace` 新增**

在 `Message` 枚举里紧邻 `ProjectTreeCopyPath(PathBuf, project::PathKind),` 之后新增：

```rust
    /// 项目树:菜单选"复制"→ 标记应用内剪贴槽(参数=路径,是否目录)。
    ProjectTreeCopy(PathBuf, bool),
    /// 项目树:菜单选"粘贴"→ 异步复制剪贴槽项到目标目录(参数=目标目录)。
    ProjectTreePaste(PathBuf),
    /// 项目树:粘贴异步结果(Ok=新建出的路径,Err=错误文案)。
    ProjectTreePasteDone(Result<PathBuf, String>),
```

`Workspace` 结构体里紧邻 `last_right_click: (f32, f32),` 之后新增：

```rust
    /// 项目树"文件管理器式"剪贴槽:最近一次"复制"的项(路径,是否目录)。
    tree_clipboard: Option<(PathBuf, bool)>,
    /// 项目树操作的行内报错文案(冲突/失败时显示;下次树操作发起时清空)。
    tree_error: Option<String>,
```

两处构造字面量紧邻 `last_right_click: (0.0, 0.0),` 之后新增：

```rust
            tree_clipboard: None,
            tree_error: None,
```

- [ ] **Step 2: `update()` 新增分支**

在 `update()` 里紧邻 `Message::ProjectTreeCopyPath(_, _) => {}` 之后新增：

```rust
            Message::ProjectTreeCopy(path, is_dir) => {
                self.tree_clipboard = Some((path, is_dir));
                self.context_menu = None;
            }
            Message::ProjectTreePaste(target_dir) => {
                self.context_menu = None;
                self.tree_error = None;
                let Some((source, source_is_dir)) = self.tree_clipboard.clone() else {
                    return;
                };
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        project::paste_item(&source, source_is_dir, &target_dir)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy.send_event(Message::ProjectTreePasteDone(result));
                });
            }
            Message::ProjectTreePasteDone(result) => match result {
                Ok(new_path) => {
                    if let (Some(tree), Some(parent)) =
                        (&mut self.file_tree, new_path.parent())
                    {
                        tree.refresh(parent);
                    }
                }
                Err(e) => self.tree_error = Some(e),
            },
```

- [ ] **Step 3: 菜单加两项**

`Message::ProjectTreePaste` 的目标目录只对"右键在目录上"有意义（文件行没有粘贴，见设计文档）。把 `context_menu_popup` 里：

```rust
    items.push(menu_item(
        "复制绝对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Absolute),
    ));
    items.push(menu_item(
        "复制相对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Relative),
    ));
    let _ = project_root; // Task 3-6 会用到;本任务先只有这两项路径复制不需要它
```

改为：

```rust
    items.push(menu_item(
        "复制",
        Message::ProjectTreeCopy(menu.target.clone(), menu.is_dir),
    ));
    if menu.is_dir {
        let has_clipboard = ws.tree_clipboard.is_some();
        let paste_msg = Message::ProjectTreePaste(menu.target.clone());
        items.push(if has_clipboard {
            menu_item("粘贴", paste_msg)
        } else {
            // 剪贴槽为空:置灰且不挂 on_press,真正不可点(同 P1L tab 箭头
            // "到头变灰"的既有处理口径,不是视觉变灰但仍能点)。
            button(text("粘贴").size(13).color(theme::DIM))
                .width(Length::Fixed(180.0))
                .padding([6, 10])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::DIM,
                    ..button::Style::default()
                })
                .into()
        });
    }
    items.push(menu_item(
        "复制绝对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Absolute),
    ));
    items.push(menu_item(
        "复制相对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Relative),
    ));
    let _ = project_root; // Task 5(重命名)/Task 6(新建)会用到
```

- [ ] **Step 4: 渲染 `tree_error`**

在 `project_pane` 里，`content.push(open_btn);` 之后、`if let Some(tree) = &ws.file_tree {` 之前新增：

```rust
            if let Some(err) = &ws.tree_error {
                content = content.push(text(format!("⚠ {err}")).size(12).color(theme::RED));
            }
```

- [ ] **Step 5: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。摘掉 `project.rs` 里 `paste_item` 上 Task 1 加的 `#[allow(dead_code)]`（本任务已是真实调用方）；`copy_dir_recursive` 保留（`paste_item` 内部调用它，但那不算"外部消费"意义上的摘除依据——实际上 `copy_dir_recursive` 现在也有真实调用方了(`paste_item`)，一并摘掉它的 `#[allow(dead_code)]`）。

- [ ] **Step 6: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：右键文件选"复制"，右键另一目录选"粘贴"——目标目录下出现同名文件，内容一致；右键目录选"复制"再粘贴到另一目录——整个子树被复制；粘贴目标已有同名项——粘贴后原位置出现红字报错，不覆盖；未复制过任何东西时右键目录，"粘贴"项置灰不可点。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/project.rs
git commit -m "feat(tree-context-menu): 复制/粘贴(应用内剪贴槽,目录递归复制,冲突红字报错)"
```

---

### Task 4: 删除（确认框 + 回收站）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增 4 个变体；`Workspace` 新增 `tree_delete_confirm` 字段；`update()` 新增分支；菜单加"删除"项；渲染确认框浮层）

**Interfaces:**
- Consumes: Task 2 的菜单外壳/`stack!` 用法；`trash` crate（Task 2 已加依赖）。
- Produces: `Message::ProjectTreeDeleteRequest(PathBuf, bool)` / `ProjectTreeDeleteConfirm` / `ProjectTreeDeleteCancel` / `ProjectTreeDeleteDone(Result<PathBuf, String>)`。

- [ ] **Step 1: `Message`/`Workspace` 新增**

在 `Message` 枚举里紧邻 `ProjectTreePasteDone(Result<PathBuf, String>),` 之后新增：

```rust
    /// 项目树:菜单选"删除"→ 打开确认框(参数=路径,是否目录)。
    ProjectTreeDeleteRequest(PathBuf, bool),
    /// 项目树:确认框点"删除"。
    ProjectTreeDeleteConfirm,
    /// 项目树:确认框点"取消"。
    ProjectTreeDeleteCancel,
    /// 项目树:删除异步结果(Ok=已删除项的父目录,Err=错误文案)。
    ProjectTreeDeleteDone(Result<PathBuf, String>),
```

`Workspace` 结构体里紧邻 `tree_error: Option<String>,` 之后新增：

```rust
    /// 项目树删除确认框目标(路径,是否目录;None=未打开确认框)。
    tree_delete_confirm: Option<(PathBuf, bool)>,
```

两处构造字面量紧邻 `tree_error: None,` 之后新增：

```rust
            tree_delete_confirm: None,
```

- [ ] **Step 2: `update()` 新增分支**

在 `update()` 里紧邻 Task 3 新增的 `Message::ProjectTreePasteDone(result) => { ... },` 之后新增：

```rust
            Message::ProjectTreeDeleteRequest(path, is_dir) => {
                self.context_menu = None;
                self.tree_delete_confirm = Some((path, is_dir));
            }
            Message::ProjectTreeDeleteCancel => {
                self.tree_delete_confirm = None;
            }
            Message::ProjectTreeDeleteConfirm => {
                let Some((path, _)) = self.tree_delete_confirm.take() else {
                    return;
                };
                self.tree_error = None;
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let parent = path.parent().map(|p| p.to_path_buf());
                    let result = tokio::task::spawn_blocking(move || {
                        trash::delete(&path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = match (result, parent) {
                        (Ok(()), Some(p)) => Ok(p),
                        (Ok(()), None) => Err("删除的是项目根,无父目录可刷新".to_string()),
                        (Err(e), _) => Err(e),
                    };
                    let _ = proxy.send_event(Message::ProjectTreeDeleteDone(outcome));
                });
            }
            Message::ProjectTreeDeleteDone(result) => match result {
                Ok(parent) => {
                    if let Some(tree) = &mut self.file_tree {
                        tree.refresh(&parent);
                    }
                }
                Err(e) => self.tree_error = Some(e),
            },
```

- [ ] **Step 3: 菜单加"删除"项**

把 `context_menu_popup` 里紧邻新增的"粘贴"逻辑块之后（即"复制绝对路径"两项之前）插入：

```rust
    items.push(menu_item(
        "删除",
        Message::ProjectTreeDeleteRequest(menu.target.clone(), menu.is_dir),
    ));
```

- [ ] **Step 4: 渲染确认框浮层**

新增确认框渲染函数（放在 `context_menu_popup` 之后）：

```rust
/// 删除确认框:居中浮层,显示目标文件名 + 确认/取消两个按钮。
fn delete_confirm_popup(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some((path, is_dir)) = &ws.tree_delete_confirm else {
        return column![].into();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let kind = if *is_dir { "文件夹" } else { "文件" };
    let dialog = container(
        column![
            text(format!("删除{kind} \"{name}\"?")).size(14).color(theme::CREAM),
            text("会移入系统回收站,可从回收站找回。").size(12).color(theme::DIM),
            row![
                button(text("取消").size(13).color(theme::CREAM))
                    .on_press(Message::ProjectTreeDeleteCancel)
                    .padding([6, 12])
                    .style(|_t, _s| button::Style {
                        background: Some(theme::CARD.into()),
                        text_color: theme::CREAM,
                        border: Border { color: theme::BORDER, width: 1.0, radius: 4.0.into() },
                        ..button::Style::default()
                    }),
                button(text("删除").size(13).color(theme::RED))
                    .on_press(Message::ProjectTreeDeleteConfirm)
                    .padding([6, 12])
                    .style(|_t, _s| button::Style {
                        background: Some(theme::CARD.into()),
                        text_color: theme::RED,
                        border: Border { color: theme::RED, width: 1.0, radius: 4.0.into() },
                        ..button::Style::default()
                    }),
            ]
            .spacing(8),
        ]
        .spacing(8),
    )
    .padding(16)
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::CARD.into()),
        border: Border { color: theme::BORDER, width: 1.0, radius: 6.0.into() },
        ..container::Style::default()
    });

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
```

把 `view()` 里的：

```rust
        if self.context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeContextMenuClose);
            stack![base, dismiss, context_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base.into()
        }
```

改为：

```rust
        if self.tree_delete_confirm.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeDeleteCancel);
            stack![base, dismiss, delete_confirm_popup(self)]
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
            stack![base, dismiss, context_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base.into()
        }
```

（删除确认框优先于右键菜单——两者不会同时打开，因为 `ProjectTreeDeleteRequest` 一进来就把 `context_menu` 设 `None`，但保留这个 `if/else if` 结构以防未来顺序调整时的隐性假设更明确。）

- [ ] **Step 5: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 6: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：右键文件/目录选"删除"→弹确认框（文件名+说明文字）；点"取消"→ 无事发生；点"删除"→ 该项从树里消失，去系统回收站（macOS: 访达"最近删除"）能找到它，能还原。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(tree-context-menu): 删除(确认框 + 移入系统回收站)"
```

---

### Task 5: 重命名（行内编辑）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增 2 个变体；`Workspace` 新增 `tree_edit` 字段；`update()` 新增分支；菜单加"重命名"项；tree row 渲染时替换成编辑框；新增 `tree_edit_row` 渲染函数）
- Modify: `crates/dozer-app/src/main.rs`（键盘拦截层三选一扩展：地址栏/验收意见/项目树编辑）

**Interfaces:**
- Consumes: Task 2 的 `AddrEvent`（复用，不新增枚举）；Task 1 的 `FileTree::refresh`。
- Produces: `Message::ProjectTreeRenameStart(PathBuf)` / `ProjectTreeEditEvent(AddrEvent)`；`Workspace::tree_editing() -> bool`（main.rs 键盘路由用，同款 `preview_addr_editing()`/`acceptance_comment_editing()`）。

- [ ] **Step 1: `Message`/`Workspace` 新增**

在 `Message` 枚举里紧邻 `ProjectTreeDeleteDone(Result<PathBuf, String>),` 之后新增：

```rust
    /// 项目树:菜单选"重命名"→ 进入行内编辑(参数=被改名项路径)。
    ProjectTreeRenameStart(PathBuf),
    /// 项目树:行内编辑框的键盘事件(main.rs 键盘拦截层送入,复用 AddrEvent)。
    ProjectTreeEditEvent(AddrEvent),
```

在 `Workspace` 定义之前（紧邻 `ContextMenu` 结构体定义之后）新增：

```rust
/// 项目树行内编辑的模式：新建文件/新建文件夹/重命名(携带原路径)。
#[derive(Debug, Clone, PartialEq)]
enum TreeEditMode {
    NewFile,
    NewFolder,
    Rename(PathBuf),
}

/// 项目树行内编辑态：新建/重命名共用。`parent_dir` 对 `Rename` 而言是
/// 被改名项的父目录(新路径=parent_dir.join(新名字))；对 `NewFile`/
/// `NewFolder` 就是目标创建位置。
#[derive(Debug, Clone, PartialEq)]
struct TreeEdit {
    parent_dir: PathBuf,
    mode: TreeEditMode,
    buffer: String,
}
```

`Workspace` 结构体里紧邻 `tree_delete_confirm: Option<(PathBuf, bool)>,` 之后新增：

```rust
    /// 项目树行内编辑态(新建/重命名共用;None=未在编辑)。
    tree_edit: Option<TreeEdit>,
```

两处构造字面量紧邻 `tree_delete_confirm: None,` 之后新增：

```rust
            tree_edit: None,
```

- [ ] **Step 2: `update()` 新增分支**

在 `update()` 里紧邻 Task 4 新增的 `Message::ProjectTreeDeleteDone(result) => { ... },` 之后新增：

```rust
            Message::ProjectTreeRenameStart(path) => {
                self.context_menu = None;
                self.tree_error = None;
                let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
                    return;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.tree_edit = Some(TreeEdit {
                    parent_dir: parent,
                    mode: TreeEditMode::Rename(path),
                    buffer: name,
                });
            }
            Message::ProjectTreeEditEvent(ev) => {
                let Some(edit) = &mut self.tree_edit else { return };
                match ev {
                    AddrEvent::Text(s) => edit.buffer.push_str(&s),
                    AddrEvent::Backspace => {
                        edit.buffer.pop();
                    }
                    AddrEvent::Cancel => self.tree_edit = None,
                    AddrEvent::Submit => self.submit_tree_edit(),
                }
            }
```

新增 `submit_tree_edit` 方法（放在 `close_all_tabs_for_switch` 附近，任何 `impl Workspace` 私有方法群里都行）：

```rust
    /// 行内编辑框回车提交：按 `TreeEditMode` 分派成重命名/新建文件/新建
    /// 文件夹的实际文件系统操作(异步,`handle.spawn`)。名字为空或就是原名
    /// (仅 Rename 场景)直接静默取消编辑，不发起任何 IO。
    fn submit_tree_edit(&mut self) {
        let Some(edit) = self.tree_edit.take() else {
            return;
        };
        let name = edit.buffer.trim();
        if name.is_empty() {
            return;
        }
        let new_path = edit.parent_dir.join(name);
        match edit.mode {
            TreeEditMode::Rename(old_path) => {
                if new_path == old_path {
                    return; // 没改名,直接结束编辑
                }
                if new_path.exists() {
                    self.tree_error = Some(format!("{} 已存在同名项", new_path.display()));
                    self.tree_edit = Some(TreeEdit {
                        parent_dir: edit.parent_dir,
                        mode: TreeEditMode::Rename(old_path),
                        buffer: name.to_string(),
                    });
                    return;
                }
                let parent = edit.parent_dir.clone();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    let _ = proxy.send_event(Message::ProjectTreeDeleteDone(outcome));
                });
            }
            TreeEditMode::NewFile | TreeEditMode::NewFolder => {
                // Task 6 补完这两个分支的实现。
            }
        }
    }
```

（`ProjectTreeDeleteDone` 的 `Ok(parent)` 分支语义是"刷新这个父目录"——重命名成功后同样需要刷新父目录，语义完全一致，直接复用这个消息而不新开一个 `ProjectTreeRenameDone`，减少一个几乎重复的变体。`Err(String)` 分支同样复用。）

新增 `tree_editing` getter（紧邻 `context_menu_open` 之后）：

```rust
    /// 项目树是否处于行内编辑态(main.rs 键盘路由用,同款
    /// `preview_addr_editing()`/`acceptance_comment_editing()`)。
    pub fn tree_editing(&self) -> bool {
        self.tree_edit.is_some()
    }
```

- [ ] **Step 3: 菜单加"重命名"项**

把 `context_menu_popup` 里紧邻"删除"项之后（"复制绝对路径"两项之前）插入：

```rust
    items.push(menu_item(
        "重命名",
        Message::ProjectTreeRenameStart(menu.target.clone()),
    ));
```

- [ ] **Step 4: tree row 渲染：命中重命名目标时换成编辑框**

新增行内编辑框渲染函数（放在 `context_menu_popup` 之前）：

```rust
/// 行内编辑框(新建/重命名共用):自绘输入,尾缀 "▏" 模拟光标,与地址栏/
/// 验收意见框同款风格(键盘走 main.rs 拦截层,不用 iced 原生 text_input)。
fn tree_edit_row(depth: usize, buffer: &str) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let indent = "  ".repeat(depth);
    container(
        text(format!("{indent}{buffer}▏"))
            .size(15)
            .color(theme::CREAM),
    )
    .width(Length::Fill)
    .padding([2, 4])
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::CREAM,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
```

把 `project_pane` 里 tree row 循环体（Task 2 已经把普通行包过 `MouseArea` 了，这里在那之前插入"是否是正在重命名的行"判断）：

```rust
                for row in tree.visible_rows() {
                    let is_renaming = matches!(
                        &ws.tree_edit,
                        Some(TreeEdit { mode: TreeEditMode::Rename(p), .. }) if *p == row.path
                    );
                    if is_renaming {
                        let buffer = ws.tree_edit.as_ref().map(|e| e.buffer.as_str()).unwrap_or("");
                        content = content.push(tree_edit_row(row.depth, buffer));
                        continue;
                    }
                    let indent = "  ".repeat(row.depth);
```

（这一段替换原来循环体开头的 `let indent = "  ".repeat(row.depth);` 那一行，把它挪到 `if is_renaming { ... continue; }` 判断之后，其余循环体不变。）

- [ ] **Step 5: `main.rs` 键盘拦截层扩展**

把 `on_window_event` 里的：

```rust
            // 地址栏 / 验收意见编辑态:键盘直达自绘输入(不经 keymap、不进 PTY)。
            let to_preview = workspace.preview_addr_editing();
            let to_comment = workspace.acceptance_comment_editing();
            if to_preview || to_comment {
```

改为：

```rust
            // 地址栏 / 验收意见 / 项目树行内编辑态:键盘直达自绘输入(不经
            // keymap、不进 PTY)。
            let to_preview = workspace.preview_addr_editing();
            let to_comment = workspace.acceptance_comment_editing();
            let to_tree_edit = workspace.tree_editing();
            if to_preview || to_comment || to_tree_edit {
```

把同一块内的：

```rust
                if let Some(ev) = addr_event {
                    // 地址栏优先（二者同真时罕见,以地址栏为准）。
                    let message = if to_preview {
                        Message::PreviewAddrEvent(ev)
                    } else {
                        Message::AcceptanceCommentEvent(ev)
                    };
                    workspace.update(message);
                    window.request_redraw();
                }
                return;
```

改为：

```rust
                if let Some(ev) = addr_event {
                    // 优先级:地址栏 > 验收意见 > 项目树编辑(三者同真时罕见,
                    // 谁先建的编辑态谁优先没有实际冲突场景,这个顺序只是
                    // 一个确定性兜底)。
                    let message = if to_preview {
                        Message::PreviewAddrEvent(ev)
                    } else if to_comment {
                        Message::AcceptanceCommentEvent(ev)
                    } else {
                        Message::ProjectTreeEditEvent(ev)
                    };
                    workspace.update(message);
                    window.request_redraw();
                }
                return;
```

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 7: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：右键文件/目录选"重命名"→ 该行原地变可编辑框(预填当前名,尾巴有光标条)；改名后回车→ 文件系统改名成功、树刷新显示新名；改成已存在的名字→ 红字报错,编辑框留着可重试；改成原名或清空后回车→ 静默退出编辑,不发起改名；编辑中按 Esc → 取消,恢复原名显示。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(tree-context-menu): 重命名(行内编辑,复用 AddrEvent 键盘路由)"
```

---

### Task 6: 新建文件 / 新建文件夹（行内编辑）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增 2 个变体；`update()` 补完 `submit_tree_edit` 的 `NewFile`/`NewFolder` 分支；菜单目录项加两项；tree row 渲染插入新建占位行）

**Interfaces:**
- Consumes: Task 1 的 `FileTree::ensure_expanded`/`refresh`；Task 5 的 `TreeEdit`/`tree_edit_row`/`submit_tree_edit` 骨架。
- Produces: `Message::ProjectTreeNewFile(PathBuf)` / `ProjectTreeNewFolder(PathBuf)`。

- [ ] **Step 1: `Message` 新增**

在 `Message` 枚举里紧邻 `ProjectTreeRenameStart(PathBuf),` 之前新增（保持"新建"在"重命名"之前，对齐设计文档菜单项顺序）：

```rust
    /// 项目树:菜单选"新建文件"→ 进入行内编辑(参数=目标父目录)。
    ProjectTreeNewFile(PathBuf),
    /// 项目树:菜单选"新建文件夹"→ 进入行内编辑(参数=目标父目录)。
    ProjectTreeNewFolder(PathBuf),
```

- [ ] **Step 2: `update()` 新增分支 + 补完 `submit_tree_edit`**

在 `update()` 里紧邻 `Message::ProjectTreeRenameStart(path) => { ... }` 之前新增：

```rust
            Message::ProjectTreeNewFile(parent) => {
                self.start_tree_new(parent, TreeEditMode::NewFile);
            }
            Message::ProjectTreeNewFolder(parent) => {
                self.start_tree_new(parent, TreeEditMode::NewFolder);
            }
```

新增 `start_tree_new` 私有方法（放在 `submit_tree_edit` 之前）：

```rust
    /// "新建文件"/"新建文件夹"的公共起点:关菜单、确保目标目录展开(让
    /// 待插入的空白编辑行有可见位置)、进入空白行内编辑。
    fn start_tree_new(&mut self, parent: PathBuf, mode: TreeEditMode) {
        self.context_menu = None;
        self.tree_error = None;
        if let Some(tree) = &mut self.file_tree {
            tree.ensure_expanded(&parent);
        }
        self.tree_edit = Some(TreeEdit {
            parent_dir: parent,
            mode,
            buffer: String::new(),
        });
    }
```

把 `submit_tree_edit` 里的：

```rust
            TreeEditMode::NewFile | TreeEditMode::NewFolder => {
                // Task 6 补完这两个分支的实现。
            }
```

改为：

```rust
            TreeEditMode::NewFile => {
                if new_path.exists() {
                    self.tree_error = Some(format!("{} 已存在同名项", new_path.display()));
                    self.tree_edit = Some(TreeEdit {
                        parent_dir: edit.parent_dir,
                        mode: TreeEditMode::NewFile,
                        buffer: name.to_string(),
                    });
                    return;
                }
                let parent = edit.parent_dir.clone();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::File::create(&new_path).map(|_| ()).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    let _ = proxy.send_event(Message::ProjectTreeDeleteDone(outcome));
                });
            }
            TreeEditMode::NewFolder => {
                if new_path.exists() {
                    self.tree_error = Some(format!("{} 已存在同名项", new_path.display()));
                    self.tree_edit = Some(TreeEdit {
                        parent_dir: edit.parent_dir,
                        mode: TreeEditMode::NewFolder,
                        buffer: name.to_string(),
                    });
                    return;
                }
                let parent = edit.parent_dir.clone();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::create_dir(&new_path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    let _ = proxy.send_event(Message::ProjectTreeDeleteDone(outcome));
                });
            }
```

（同 Task 5 的重命名一样，复用 `ProjectTreeDeleteDone(Result<PathBuf,String>)` 消息承载"操作完成,刷新这个父目录/报这个错"的语义——四种操作(删除/重命名/新建文件/新建文件夹)结果形状完全一致，没必要各开一个消息变体。）

- [ ] **Step 3: 菜单加两项（仅目录）**

把 `context_menu_popup` 里"复制"项之前插入（新建只对目录有意义，文件行的 `menu.is_dir` 为 `false` 时不显示）：

```rust
    if menu.is_dir {
        items.push(menu_item(
            "新建文件",
            Message::ProjectTreeNewFile(menu.target.clone()),
        ));
        items.push(menu_item(
            "新建文件夹",
            Message::ProjectTreeNewFolder(menu.target.clone()),
        ));
    }
```

（插入位置在 `items.push(menu_item("复制", ...));` 之前，即目录专属项打头，文件/目录共有项在后，与设计文档"目录:新建文件/新建文件夹/复制/粘贴/删除/重命名/…"的顺序一致。）

- [ ] **Step 4: tree row 渲染：插入新建占位行**

把 Task 5 新增的 tree row 循环体（`is_renaming` 判断之后）扩展成同时处理"这一行下面要不要插一个新建占位行"：

```rust
                for row in tree.visible_rows() {
                    let is_renaming = matches!(
                        &ws.tree_edit,
                        Some(TreeEdit { mode: TreeEditMode::Rename(p), .. }) if *p == row.path
                    );
                    if is_renaming {
                        let buffer = ws.tree_edit.as_ref().map(|e| e.buffer.as_str()).unwrap_or("");
                        content = content.push(tree_edit_row(row.depth, buffer));
                        continue;
                    }
                    let indent = "  ".repeat(row.depth);
```

在该循环体末尾（`content = content.push(MouseArea::new(row_btn)...);` 语句之后，循环 `for` 块结束 `}` 之前）追加：

```rust
                    let is_new_target = matches!(
                        &ws.tree_edit,
                        Some(TreeEdit { mode: TreeEditMode::NewFile | TreeEditMode::NewFolder, parent_dir, .. })
                            if *parent_dir == row.path
                    );
                    if is_new_target && row.expanded {
                        let buffer = ws.tree_edit.as_ref().map(|e| e.buffer.as_str()).unwrap_or("");
                        content = content.push(tree_edit_row(row.depth + 1, buffer));
                    }
```

（`row.expanded` 判断防御性保留——`start_tree_new` 已经 `ensure_expanded` 过，正常情况下这里恒为真；空目录 `ensure_expanded` 不会真正展开(`toggle` 内部逻辑)，此时 `row.expanded` 为假，占位行就不会插在一个视觉上"收起"的目录下面显得诡异，改成插在目录行本身的直接位置也是可以接受的降级——这里选择"不显示"是因为空目录展开后没有其它子项作参照，插入位置观感上不够清晰，用户重新点开该目录后所见即所得。）

- [ ] **Step 5: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 6: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：右键目录选"新建文件"→ 该目录自动展开,底下出现空白编辑框；输入任意文件名(含任意后缀,比如 `foo.xyz`)回车→ 磁盘上出现该空文件,树刷新显示；"新建文件夹"同理，输入名字回车创建空目录；输入已存在的名字→ 红字报错,编辑框留着；清空名字回车或 Esc → 取消,不创建任何东西；文件行右键没有"新建文件"/"新建文件夹"两项。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(tree-context-menu): 新建文件/新建文件夹(行内编辑,复用重命名骨架)"
```

---

## 收尾（全 task 完成后）

- [ ] **回归 + 人工验收**：全量 `cargo test -p dozer-app` 绿、clippy/fmt 干净；真机对 Task 2-6 每个 Step 的目测点逐项确认，含设计文档列的两项视觉/交互取舍。
- [ ] **落档**：验收通过后勾选本计划全部 box，`docs/superpowers/specs/2026-07-28-project-tree-context-menu-design.md` 视需要补验收记录（参照本仓其余 `*-acceptance.md` 文档惯例）。
- [ ] **分支收尾**：`superpowers:finishing-a-development-branch` 合入 main（若在独立分支上开发）。

## 自检记录（写计划时）

- **Spec 覆盖**：设计文档"目标"清单（目录 8 项/文件 5 项菜单）→ Task 2(路径复制×2)+Task3(复制/粘贴)+Task4(删除)+Task5(重命名)+Task6(新建×2)，逐项都有对应 task。"关键语义确认"五条（复制=文件管理器式/路径复制才写系统剪贴板/删除=确认+回收站/新建重命名=行内编辑/冲突=红字不覆盖）全部在对应 task 落实。"事件时序"(右键定位)→ Task2 Step 7-9。"已知简化"(孤儿缓存条目不做级联失效)→ Task 1 doc comment 原样保留。
- **占位扫描**：无 TBD/TODO。Task 5 Step 2 里 `TreeEditMode::NewFile | TreeEditMode::NewFolder => { // Task 6 补完 }` 这一行是刻意的、有明确后续 task 编号的过渡代码（不是"以后再说"式占位）——Task 6 Step 2 立即把它替换成完整实现，两个 task 之间没有可合并的窗口期让这行"裸奔"进用户可见路径（`submit_tree_edit` 只有 `TreeEditMode::NewFile`/`NewFolder` 变体存在的分支能到达它，而这两个变体到 Task 6 才有入口消息(`ProjectTreeNewFile`/`ProjectTreeNewFolder`)能构造出来——Task 5 单独跑完时这个分支是不可达代码，不影响任何真实行为）。
- **类型/签名一致性**：`ContextMenu`/`TreeEdit`/`TreeEditMode` 在 Task 2/5 定义，之后各 task 按同名同字段引用一致；四个"操作完成"结果全部收敛成 `Result<PathBuf, String>` 语义并复用 `ProjectTreeDeleteDone` 这一个消息变体承载（删除/重命名/新建文件/新建文件夹），未出现每个操作各开一个 `*Done` 变体的膨胀，Task 3 的 `ProjectTreePasteDone` 是唯一例外（粘贴发生在 Task 3，早于"发现可以统一复用"的 Task 5 才做的决定——若要追求最大一致性可把 `ProjectTreePasteDone` 也合并进 `ProjectTreeDeleteDone`，但那样"删除"完成消息名称语义上不再贴切"粘贴"场景，本计划选择接受这一处轻微不对称，不为了统一而让命名变得词不达意）。
- **风险**：`trash` crate 具体版本号交给 `cargo add` 解析，未来若该 crate 有破坏性 API 变更需按编译器提示适配（Global Constraints 已声明此惯例）；`Padding{top,left,right:0.0,bottom:0.0}` 手算定位菜单若点击点太靠右/靠下会让菜单溢出窗口边界——本计划未做"溢出后向左/上翻转"的处理（设计文档未提出此要求），真机验收若发现视觉问题可后续追加。
