# Dozer 图标系统实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 Lucide 开源图标集(经 iced 0.14 原生 `svg` 组件渲染、编译期内嵌)替换项目树/顶栏/右键菜单里现有的 Unicode 字符占位(`▸/▾/⚙` 等)。

**Architecture:** 新增 `crates/dozer-app/src/icons.rs` 模块，穷举一个 `IconKind` 枚举(每个变体对应一个 vendor 进仓库的 `.svg` 文件，`include_bytes!` 内嵌，同 `theme.rs` "精选常量表"哲学)，统一渲染入口 `icons::view(kind, size, color)` 返回 `Element`，调用方传主题色，用法与现有 `text().color(...)` 一致。图标内容来自 Lucide(MIT)，逐个 `curl` 下载 vendor 进仓库，不引入 Node/Python 或任何图标处理工具链。

**Tech Stack:** Rust, iced 0.14(`iced_widget::svg` 组件，需新开 `svg` cargo feature)。

## Global Constraints

- 颜色只取自 `crates/dozer-app/src/theme.rs`，禁止新增硬编码色值。
- 每 task 收尾 `cargo build -p dozer-app`、`cargo clippy -p dozer-app --all-targets` clean、`cargo fmt -p dozer-app -- --check` 干净、`cargo test -p dozer-app` 绿。
- `dozer-app` 是纯 `[[bin]]` crate(无 `[lib]` target)，`pub` 不豁免 `dead_code` lint——新模块未被消费的条目需要 `#[allow(dead_code)]`（本计划采用 `theme.rs` 的模块级 `#![allow(dead_code)]` 方案，见 Task 2）。
- 图标来源仅 Lucide（MIT），不引入按语言品牌化的多色 logo 图标、不做状态指示点的形状重设计（这两条已在设计文档 §3.1/§3.4 明确排除）。
- 设计依据：`docs/superpowers/specs/2026-07-28-dozer-icon-system-design.md`。

---

### Task 1: Vendor Lucide 图标资产 + 开启 `svg` cargo feature

**Files:**
- Create: `crates/dozer-app/assets/icons/*.svg`（18 个文件，见下方清单）
- Create: `crates/dozer-app/assets/icons/LICENSE`
- Modify: `crates/dozer-app/Cargo.toml`（`iced_widget` 依赖行新增 `svg` feature）

**Interfaces:**
- Produces: 18 个 vendor 好的 `.svg` 文件（供 Task 2 的 `include_bytes!` 消费）；`iced_widget` 的 `svg` feature 开启（供 Task 2 的 `iced_widget::svg`/`svg::Handle`/`svg::Style` 消费）。

本任务纯资产准备，不写 Rust 代码。

- [ ] **Step 1: 下载 18 个 Lucide 图标 SVG**

Lucide 仓库把每个图标存成独立文件，稳定 URL 形如
`https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/<name>.svg`。逐个下载：

```bash
mkdir -p crates/dozer-app/assets/icons
cd crates/dozer-app/assets/icons

for name in chevron-right chevron-down folder folder-open file-code file-json \
            file-text file-cog file-image file search settings file-plus \
            folder-plus copy clipboard-paste trash-2 pen-line; do
  curl -fsSL -o "${name}.svg" \
    "https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/${name}.svg"
done
```

Run: `ls crates/dozer-app/assets/icons/*.svg | wc -l`
Expected: `17`（上面列了 17 个 slug；加上下一步单独处理的 `file.svg` 命名冲突说明，实际最终 18 个变体对应 17 个不同文件——见下方"命名说明"）。

**命名说明**：`IconKind::FileGeneric` 用的通用文件图标，Lucide 里的文件名就是 `file.svg`——上面循环里已经包含了 `file`（`for` 列表中的裸 `file`），所以 17 个 slug 实际产出 17 个文件，`FileGeneric` 复用其中的 `file.svg`，不是第 18 个文件。18 是 `IconKind` 枚举变体数（Task 2 会定义），不是文件数——两者不必一一对应，`Copy` 图标目前只被右键菜单两个"复制路径"变体共用同一个 `copy.svg`，那是调用方复用，与本任务的文件数量无关。

Run: `for f in crates/dozer-app/assets/icons/*.svg; do echo "$f: $(wc -c < "$f") bytes"; done`
Expected: 17 行输出，每个文件都是非零字节数（几百到几千字节，具体数值不重要，关键是没有 0 字节或明显异常小的文件——0 字节通常意味着该 URL 404 但 `curl -f` 本该已经让命令非零退出；如果某一行显示可疑地小，比如几十字节，`cat` 该文件确认内容是不是一段 GitHub 404 HTML 而不是 SVG）。

**兜底**：如果某个 slug 返回 404（`curl -f` 会让该行报错退出，此时上面 `wc -l` 的计数会小于 17），去
`https://lucide.dev/icons` 搜索对应概念（如"trash"/"pen"/"cog"）确认 Lucide 当前版本的正确图标名，
替换 `for` 循环里的 slug 后重跑。这不是设计层面的模糊，只是第三方图标库命名可能随版本演进——找到正确
slug 后按新名字保存到同样的 `crates/dozer-app/assets/icons/<正确名字>.svg` 路径，并同步告知后续 Task 2
（如果slug 变了，Task 2 里对应的 `include_bytes!` 路径也要用新文件名）。

- [ ] **Step 2: 用 xmllint 或肉眼确认每个文件是合法 SVG**

Run: `for f in crates/dozer-app/assets/icons/*.svg; do head -c 60 "$f"; echo " <- $f"; done`
Expected: 每一行都以 `<svg` 开头（可能前面有 `<?xml ...?>` 声明），不是 HTML（`<!DOCTYPE` 或 `<html`）。

- [ ] **Step 3: 加 Lucide 署名文件**

Lucide 是 ISC 协议（功能上等同 MIT，均为宽松许可，允许免署名复制但保留版权声明是惯例）。写入
`crates/dozer-app/assets/icons/LICENSE`：

```
ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT).
All other copyright (c) for Lucide are held by Lucide Contributors 2022.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY
AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR
OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
PERFORMANCE OF THIS SOFTWARE.
```

- [ ] **Step 4: 开启 `svg` cargo feature**

把 `crates/dozer-app/Cargo.toml` 里的：

```toml
iced_widget = { version = "0.14", features = ["wgpu", "canvas"] }
```

改为：

```toml
iced_widget = { version = "0.14", features = ["wgpu", "canvas", "svg"] }
```

- [ ] **Step 5: 确认编译通过**

Run: `cargo build -p dozer-app`
Expected: 编译通过（本任务未写任何消费 `svg` feature 的 Rust 代码，这一步只确认开 feature 本身不引入编译错误、`Cargo.lock` 正常更新）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/assets/icons crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "chore(icons): vendor Lucide 图标资产 + 开启 iced svg feature"
```

---

### Task 2: `icons.rs` 模块——`IconKind` 枚举 + 统一渲染入口 + 文件类型映射

**Files:**
- Create: `crates/dozer-app/src/icons.rs`
- Modify: `crates/dozer-app/src/main.rs`（新增 `mod icons;`）

**Interfaces:**
- Consumes: Task 1 vendor 好的 `crates/dozer-app/assets/icons/*.svg` 文件；`iced_widget` 的 `svg` feature。
- Produces:
  - `pub enum IconKind { ChevronRight, ChevronDown, Folder, FolderOpen, FileCode, FileJson, FileText, FileConfig, FileImage, FileGeneric, Search, Settings, FilePlus, FolderPlus, Copy, ClipboardPaste, Trash, Rename }`（`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`）
  - `pub fn view<'a, Message: 'a>(kind: IconKind, size: f32, color: iced_widget::core::Color) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>`
  - `pub fn icon_for_file(name: &str) -> IconKind`

本任务不接线到任何 UI（Task 3-5 才是消费方），因此模块整体标 `#![allow(dead_code)]`，做法与
`crates/dozer-app/src/theme.rs` 顶部注释里描述的"先整体建好接口，后续任务再消费"完全一致，直接照抄
那个先例，不做逐条 `#[allow(dead_code)]` + 后续任务逐个摘除的繁琐流程。

- [ ] **Step 1: 写失败测试**

创建 `crates/dozer-app/src/icons.rs`，先写文件末尾的 `#[cfg(test)] mod tests`（此时 `icon_for_file`
还不存在，测试会编译失败）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_for_file_maps_code_extensions() {
        assert_eq!(icon_for_file("main.rs"), IconKind::FileCode);
        assert_eq!(icon_for_file("app.TS"), IconKind::FileCode); // 大小写不敏感
        assert_eq!(icon_for_file("script.py"), IconKind::FileCode);
    }

    #[test]
    fn icon_for_file_maps_json() {
        assert_eq!(icon_for_file("package.json"), IconKind::FileJson);
    }

    #[test]
    fn icon_for_file_maps_text() {
        assert_eq!(icon_for_file("README.md"), IconKind::FileText);
        assert_eq!(icon_for_file("notes.txt"), IconKind::FileText);
    }

    #[test]
    fn icon_for_file_maps_config() {
        assert_eq!(icon_for_file("Cargo.toml"), IconKind::FileConfig);
        assert_eq!(icon_for_file("ci.yaml"), IconKind::FileConfig);
        assert_eq!(icon_for_file("ci.yml"), IconKind::FileConfig);
    }

    #[test]
    fn icon_for_file_maps_image() {
        assert_eq!(icon_for_file("logo.png"), IconKind::FileImage);
        assert_eq!(icon_for_file("icon.svg"), IconKind::FileImage);
    }

    #[test]
    fn icon_for_file_falls_back_to_generic_for_unknown_extension() {
        assert_eq!(icon_for_file("LICENSE"), IconKind::FileGeneric);
        assert_eq!(icon_for_file("Makefile"), IconKind::FileGeneric);
        assert_eq!(icon_for_file("data.xyz"), IconKind::FileGeneric);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer icons::`
Expected: FAIL（编译错误，`icon_for_file`/`IconKind` 未定义；此时 `main.rs` 也还没声明 `mod icons;`，
先跳过这条编译错误，进 Step 3 一次性把模块和 `mod` 声明都加上再重跑）。

- [ ] **Step 3: 实现**

在 `crates/dozer-app/src/icons.rs` 顶部（`#[cfg(test)]` 块之前）写入：

```rust
//! Lucide 图标(MIT/ISC，见 `assets/icons/LICENSE`)编译期内嵌 + 统一渲染入口。
//! `IconKind` 是穷举枚举而非开放式字符串——新增图标 = 加一个变体 + 一个 svg 文件，
//! 与 `theme.rs` 精选 14 色而非任意色值同一哲学。
#![allow(dead_code)] // 本模块先整体建好接口，Task 3-5 逐个消费，同 theme.rs 先例

use iced_widget::core::{Color, Element, Length};
use iced_widget::svg;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    ChevronRight,
    ChevronDown,
    Folder,
    FolderOpen,
    FileCode,
    FileJson,
    FileText,
    FileConfig,
    FileImage,
    FileGeneric,
    Search,
    Settings,
    FilePlus,
    FolderPlus,
    Copy,
    ClipboardPaste,
    Trash,
    Rename,
}

impl IconKind {
    fn bytes(self) -> &'static [u8] {
        match self {
            IconKind::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            IconKind::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            IconKind::Folder => include_bytes!("../assets/icons/folder.svg"),
            IconKind::FolderOpen => include_bytes!("../assets/icons/folder-open.svg"),
            IconKind::FileCode => include_bytes!("../assets/icons/file-code.svg"),
            IconKind::FileJson => include_bytes!("../assets/icons/file-json.svg"),
            IconKind::FileText => include_bytes!("../assets/icons/file-text.svg"),
            IconKind::FileConfig => include_bytes!("../assets/icons/file-cog.svg"),
            IconKind::FileImage => include_bytes!("../assets/icons/file-image.svg"),
            IconKind::FileGeneric => include_bytes!("../assets/icons/file.svg"),
            IconKind::Search => include_bytes!("../assets/icons/search.svg"),
            IconKind::Settings => include_bytes!("../assets/icons/settings.svg"),
            IconKind::FilePlus => include_bytes!("../assets/icons/file-plus.svg"),
            IconKind::FolderPlus => include_bytes!("../assets/icons/folder-plus.svg"),
            IconKind::Copy => include_bytes!("../assets/icons/copy.svg"),
            IconKind::ClipboardPaste => include_bytes!("../assets/icons/clipboard-paste.svg"),
            IconKind::Trash => include_bytes!("../assets/icons/trash-2.svg"),
            IconKind::Rename => include_bytes!("../assets/icons/pen-line.svg"),
        }
    }
}

/// 统一图标渲染入口:调用方传主题色，用法与 `text().color(...)` 一致。
/// 泛型于 `Message`(图标本身不接受点击，任何 `Message` 类型都能塞进去)。
pub fn view<'a, Message: 'a>(
    kind: IconKind,
    size: f32,
    color: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    svg(svg::Handle::from_memory(kind.bytes()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_theme: &iced_widget::Theme, _status| svg::Style { color: Some(color) })
        .into()
}

/// 文件名 → 图标类别(按扩展名，类别式而非按语言品牌，见设计文档 §3.1 caveat；
/// 大小写不敏感)。
pub fn icon_for_file(name: &str) -> IconKind {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "go" | "java" | "c" | "cpp" | "h" | "hpp"
        | "rb" | "swift" | "kt" | "sh" => IconKind::FileCode,
        "json" => IconKind::FileJson,
        "md" | "txt" => IconKind::FileText,
        "toml" | "yaml" | "yml" => IconKind::FileConfig,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" => IconKind::FileImage,
        _ => IconKind::FileGeneric,
    }
}
```

在 `crates/dozer-app/src/main.rs` 的 `mod` 列表里，紧邻 `mod goal;` 之后（字母序 goal < icons <
keymap）新增：

```rust
mod icons;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app icons::`
Expected: 6 个测试全绿。

- [ ] **Step 5: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译通过 / clean / 无输出。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/icons.rs crates/dozer-app/src/main.rs
git commit -m "feat(icons): IconKind 枚举 + 统一渲染入口 + 文件扩展名映射(纯逻辑,Task3-5接线)"
```

---

### Task 3: 项目树——展开箭头 + 文件夹/文件类型图标

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`project_pane` 内 tree row 渲染逻辑；删除 `tree_row_glyph` 函数与其测试）

**Interfaces:**
- Consumes: Task 2 的 `icons::IconKind`、`icons::view`、`icons::icon_for_file`。
- Produces: 无新接口——本任务是纯 UI 消费方，不产出后续任务需要的类型/函数。

- [ ] **Step 1: 删除 `tree_row_glyph` 函数与其测试**

删除 `crates/dozer-app/src/workspace.rs` 里的：

```rust
/// 文件树行前导字形：目录展开/收拢三角，文件用中点。不用 emoji（字体毒化，见 fonts.rs）。
fn tree_row_glyph(is_dir: bool, expanded: bool) -> &'static str {
    match (is_dir, expanded) {
        (true, true) => "▾ ",
        (true, false) => "▸ ",
        (false, _) => "· ",
    }
}
```

以及 `#[cfg(test)] mod tests` 里对应的：

```rust
    #[test]
    fn tree_glyph_dir_toggles_file_is_dot() {
        assert_eq!(tree_row_glyph(true, true), "▾ ");
        assert_eq!(tree_row_glyph(true, false), "▸ ");
        assert_eq!(tree_row_glyph(false, false), "· ");
    }
```

- [ ] **Step 2: 改写 tree row 渲染**

把 `project_pane` 里（`if let Some(tree) = &ws.file_tree {` 循环内）的：

```rust
                    let indent = "  ".repeat(row.depth);
                    let glyph = tree_row_glyph(row.is_dir, row.expanded);
                    let status = if row.is_dir {
                        delivery::dir_status(&row.path, &ws.git_statuses)
                    } else {
                        ws.git_statuses.get(&row.path).copied()
                    };
                    let name_color = if row.is_dir {
                        theme::BODY
                    } else {
                        theme::CREAM
                    };
                    let mut line = row![
                        text(format!("{indent}{glyph}{}", row.name))
                            .size(15)
                            .color(name_color)
                    ]
                    .spacing(6);
```

改为：

```rust
                    let indent = "  ".repeat(row.depth);
                    let status = if row.is_dir {
                        delivery::dir_status(&row.path, &ws.git_statuses)
                    } else {
                        ws.git_statuses.get(&row.path).copied()
                    };
                    let name_color = if row.is_dir {
                        theme::BODY
                    } else {
                        theme::CREAM
                    };
                    let row_icon: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
                        if row.is_dir {
                            let chevron = if row.expanded {
                                icons::IconKind::ChevronDown
                            } else {
                                icons::IconKind::ChevronRight
                            };
                            let folder = if row.expanded {
                                icons::IconKind::FolderOpen
                            } else {
                                icons::IconKind::Folder
                            };
                            row![
                                icons::view(chevron, 12.0, theme::DIM),
                                icons::view(folder, 14.0, theme::DIM),
                            ]
                            .spacing(2)
                            .into()
                        } else {
                            icons::view(icons::icon_for_file(&row.name), 14.0, theme::DIM)
                        };
                    let mut line = row![
                        text(indent).size(15).color(name_color),
                        row_icon,
                        text(row.name.clone()).size(15).color(name_color),
                    ]
                    .spacing(6);
```

（`indent` depth=0 时是空字符串，`text("")` 渲染零宽度，无视觉影响，与原逻辑行为一致。`status`/
`name_color` 两行原样保留，只是被移到了新代码块里同样的相对位置。）

- [ ] **Step 3: 加 `use crate::icons;`**

在 `crates/dozer-app/src/workspace.rs` 顶部 `use` 区新增：

```rust
use crate::icons;
```

- [ ] **Step 4: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译通过；测试全绿（比 Task 2 完成时少 1 个——`tree_glyph_dir_toggles_file_is_dot` 已删除）；
clean；无输出。

- [ ] **Step 5: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：项目树目录行显示"箭头(▸/▾ 换成对应图标) + 文件夹图标(开合两态) + 目录名"；
文件行按扩展名显示对应类别图标(`.rs`/`.py` 等显示代码图标，`.json` 显示 JSON 图标，`.md`/`.txt`
显示文本图标，`.toml`/`.yaml` 显示齿轮图标，图片显示图片图标，其余显示通用文件图标)；git 状态彩色圆点
位置/颜色不变(本任务未触碰该逻辑)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(icons): 项目树展开箭头 + 文件夹/文件类型图标"
```

---

### Task 4: 顶栏——搜索框图标 + 设置图标

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`top_bar` 函数）

**Interfaces:**
- Consumes: Task 2 的 `icons::IconKind`、`icons::view`。
- Produces: 无新接口。

- [ ] **Step 1: 搜索框加前缀图标**

把 `top_bar` 函数里的：

```rust
    let search = container(text("搜索作品、会话、产物…  ⌘K").size(13).color(theme::DIM))
        .padding([6, 12])
        .width(Length::Fixed(360.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });
```

改为：

```rust
    let search = container(
        row![
            icons::view(icons::IconKind::Search, 14.0, theme::DIM),
            text("搜索作品、会话、产物…  ⌘K").size(13).color(theme::DIM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .padding([6, 12])
    .width(Length::Fixed(360.0))
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    });
```

- [ ] **Step 2: 设置齿轮换图标**

把 `top_bar` 函数里的：

```rust
    right = right.push(text("⚙").size(15).color(theme::DIM));
```

改为：

```rust
    right = right.push(icons::view(icons::IconKind::Settings, 16.0, theme::DIM));
```

- [ ] **Step 3: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译通过；测试全绿；clean；无输出。

- [ ] **Step 4: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：顶栏搜索框左侧出现放大镜图标，文字不变；顶栏右侧原来的 `⚙` 换成设置图标，
位置/颜色观感一致（两者均未接线点击行为，本任务不新增交互）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(icons): 顶栏搜索框 + 设置图标"
```

---

### Task 5: 右键菜单——每项加前缀图标

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`menu_item` 函数签名扩展 + `context_menu_popup` 全部调用点）

**Interfaces:**
- Consumes: Task 2 的 `icons::IconKind`、`icons::view`。
- Produces: `menu_item` 签名变为 `fn menu_item<'a>(icon: icons::IconKind, label: &'static str, msg: Message) -> Element<'a, ...>`——本计划内无后续任务消费这个签名变化，但如果本仓未来任何地方新增别的右键菜单，这是新的调用约定。

- [ ] **Step 1: 扩展 `menu_item` 签名**

把：

```rust
fn menu_item<'a>(
    label: &'static str,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
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
```

改为：

```rust
fn menu_item<'a>(
    icon: icons::IconKind,
    label: &'static str,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(
        row![
            icons::view(icon, 14.0, theme::CREAM),
            text(label).size(13).color(theme::CREAM),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    )
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
```

- [ ] **Step 2: 更新 `context_menu_popup` 里全部 7 处 `menu_item(...)` 调用**

在 `context_menu_popup` 函数内，把这 7 处调用逐一加上图标参数（其余参数不变，只在第一个参数位置插入
`icons::IconKind::...,`）：

```rust
        items.push(menu_item(
            icons::IconKind::FilePlus,
            "新建文件",
            Message::ProjectTreeNewFile(menu.target.clone()),
        ));
        items.push(menu_item(
            icons::IconKind::FolderPlus,
            "新建文件夹",
            Message::ProjectTreeNewFolder(menu.target.clone()),
        ));
```

```rust
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制",
        Message::ProjectTreeCopy(menu.target.clone(), menu.is_dir),
    ));
```

```rust
        items.push(if has_clipboard {
            menu_item(
                icons::IconKind::ClipboardPaste,
                "粘贴",
                paste_msg,
            )
        } else {
```

```rust
    items.push(menu_item(
        icons::IconKind::Trash,
        "删除",
        Message::ProjectTreeDeleteRequest(menu.target.clone(), menu.is_dir),
    ));
    items.push(menu_item(
        icons::IconKind::Rename,
        "重命名",
        Message::ProjectTreeRenameStart(menu.target.clone()),
    ));
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制绝对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Absolute),
    ));
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制相对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Relative),
    ));
```

- [ ] **Step 3: 禁用态"粘贴"按钮也加图标**

把 `context_menu_popup` 里手写的禁用态粘贴按钮：

```rust
            button(text("粘贴").size(13).color(theme::DIM))
                .width(Length::Fixed(180.0))
                .padding([6, 10])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::DIM,
                    ..button::Style::default()
                })
                .into()
```

改为：

```rust
            button(
                row![
                    icons::view(icons::IconKind::ClipboardPaste, 14.0, theme::DIM),
                    text("粘贴").size(13).color(theme::DIM),
                ]
                .spacing(8)
                .align_y(iced_widget::core::Alignment::Center),
            )
            .width(Length::Fixed(180.0))
            .padding([6, 10])
            .style(|_t, _s| button::Style {
                background: Some(theme::CARD.into()),
                text_color: theme::DIM,
                ..button::Style::default()
            })
            .into()
```

- [ ] **Step 4: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译通过（`menu_item` 签名变了，若本文件外还有其他调用点编译器会报出来，按报错逐一补
`icons::IconKind::...` 参数）；测试全绿；clean；无输出。

- [ ] **Step 5: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：项目树任意行右键，8 个菜单项（目录）/ 5 个菜单项（文件）每项左侧都有对应
图标；剪贴槽为空时"粘贴"置灰且图标也置灰、不可点；"复制绝对路径"与"复制相对路径"用同一个复制图标
（这是本计划的既定选择，见设计文档 §3.3，不是遗漏）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(icons): 右键菜单每项加前缀图标"
```

---

## 收尾（全 task 完成后）

- [ ] **回归 + 人工验收**：全量 `cargo test -p dozer-app` 绿、clippy/fmt 干净；真机对 Task 3-5 每个
  Step 的目测点逐项确认。
- [ ] **落档**：验收通过后勾选本计划全部 box，`docs/superpowers/specs/2026-07-28-dozer-icon-system-design.md`
  视需要补验收记录（参照本仓其余 `*-acceptance.md` 文档惯例）。
- [ ] **分支收尾**：`superpowers:finishing-a-development-branch` 合入 main（若在独立分支上开发）。

## 自检记录（写计划时）

- **Spec 覆盖**：设计文档 §3.1(项目树文件夹/文件图标)→ Task 3；§3.2(顶栏)→ Task 4；§3.3(右键菜单)
  → Task 5；§3.4(状态点不迁移)→ 本计划未新建任何状态点相关 task，与设计一致；§2(技术选型/资产管线)
  → Task 1+2；§4(非目标)→ 无对应 task，符合预期（非目标本就不该有 task）；§5(测试)→ Task 2 的
  `icon_for_file` 单测 + Task 3-5 的"编译+真机目测"约定。
- **占位扫描**：无 TBD/TODO。Task 1 的"兜底"小节（Lucide slug 若 404 怎么办）是应对外部第三方目录
  可能漂移的操作性预案，不是设计层面的模糊留白——不确定的是"外部数据当下是否可用"，不是"我们要做
  什么"。
- **类型/签名一致性**：`icons::IconKind`/`icons::view`/`icons::icon_for_file` 在 Task 2 定义，Task
  3-5 按同名同签名引用一致；`menu_item` 签名变化（Task 5）只影响本文件内的调用点，全部 7 处已在
  Step 2 逐一列出，Step 4 的编译步骤兜底捕获任何遗漏的调用点。
- **风险**：Lucide 图标的具体 SVG 内容(路径数据)不在计划文本里手写，而是运行时下载得到——这是刻意
  选择（见设计文档 §2.3，避免手抄第三方矢量路径数据引入的准确性风险），Task 1 的字节数/首字节检查是
  这条路径的验证手段，不能保证像素级正确，最终视觉效果仍需 Task 3-5 的真机目测确认。`svg` widget 的
  `Style.color` 语义（是否对任意 SVG 做整体 tint，还是只对无自带 fill 的路径生效）本计划假设"整体
  tint"（Lucide 图标本身是 `stroke="currentColor"` 的线性图标，这类图标是该着色模型的典型适用场景），
  若真机目测发现颜色不对（比如图标显示原始黑色而非主题色），提示 iced 版本的 svg 着色行为与预期不同，
  需要停下来诊断而非硬套本计划的颜色调用方式。
