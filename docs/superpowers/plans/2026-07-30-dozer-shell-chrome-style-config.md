# Dozer 外壳区域样式配置化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Dozer GUI 外壳 13 个区域(主导航、左右图标栏、放大态浮层、右键菜单、状态条 + 7 个面板内容区)各自外层容器的背景色/边框/内边距/子元素间距,从散落在 `workspace.rs` 里的字面量,搬进一份编译期内嵌的 JSON 配置文件,不改变任何现有视觉效果。

**Architecture:** 新模块 `crates/dozer-app/src/chrome_style.rs` 用 `include_str!` 编译期内嵌 `crates/dozer-app/assets/theme/regions.json`,启动时 `serde_json` 解析一次到 `LazyLock<ResolvedRegions>`;颜色字段是字符串,引用 `theme.rs` 现成的令牌名(不重复写十六进制,`theme.rs` 仍是颜色数值唯一真相源);每个区域一个访问函数,`workspace.rs` 里 13 处原本内联的 `container::Style { background: Some(theme::PANEL.into()), .. }` 改成读取访问函数的返回值。

**Tech Stack:** Rust、serde/serde_json(已是 `dozer-app` 的 workspace 依赖,无需改 `Cargo.toml`)、iced_widget 0.14。

## Global Constraints

- 本轮**不改变任何现有视觉效果**——每个区域配置化前后的背景色/边框/内边距/间距数值必须完全一致,这是一次纯代码搬家。
- **不做运行时读盘**——JSON 编译期通过 `include_str!` 内嵌进二进制,解析失败(格式错误/未知颜色令牌名)在启动时直接 `panic`,不做运行时降级。
- **不覆盖区域内部控件的交互态样式**——按钮/tab 的 active/hover/pressed 态(如 `rail_icon_button`、`tab_arrow_button`)保持现状,留在 Rust 代码里,不进这份配置。
- **不新增/不修改 `theme.rs` 现有的 14 个锁定颜色值**——只允许新增一个 `SCRIM` 常量(纯增量)。
- 每个任务改完后跑 `cargo build -p dozer-app` 和 `cargo test -p dozer-app` 确认无回归,再提交。
- 参考 spec:`docs/superpowers/specs/2026-07-30-dozer-shell-chrome-style-config-design.md`。

---

## 区域清单与当前值(13 个,含实现时发现的 `status_bar`)

实现前重新核对代码发现 spec 里遗漏了一个区域:`status_bar_container`(项目栏底/终端栏底状态条共用的外框函数,`workspace.rs:3610`),它和其余 12 个一样是"背景色+边框+内边距"的容器,符合 spec 的收录标准,只是搬 spec 时的函数名扫描漏掉了这个共享 helper。Task 2 会顺带把 spec 文档的区域表补上这一行。

| key | 对应函数/位置(执行时以 `grep -n` 现查为准,行号会因为其他并行改动漂移) | background | border | padding | gap |
|---|---|---|---|---|---|
| `top_bar` | `top_bar` | `BG` | `{BORDER, 0.0, 0.0}` | `[0, 12]` | `16` |
| `left_icon_rail` | `left_icon_rail` | `PANEL` | `null` | `[16, 6, 0, 6]` | `12` |
| `right_icon_rail` | `right_icon_rail` | `PANEL` | `null` | `[16, 6, 0, 6]` | `12` |
| `project_pane` | `project_pane` 的 `body` | `PANEL` | `null` | `8` | `4` |
| `preview_pane` | `preview_pane` | `PANEL` | `null` | `8` | `4` |
| `browser_pane` | `browser_pane` | `PANEL` | `null` | `8` | `4` |
| `agent_list_pane` | `agent_list_pane` | `PANEL` | `null` | `12` | `8` |
| `terminal_pane` | `terminal_pane` 的 `body` | `TERM_BG` | `null` | `8` | `4` |
| `conversation_list_pane` | `conversation_list_pane` | `PANEL` | `null` | `12` | `8` |
| `review_content_pane` | `review_content_pane` | `PANEL` | `null` | `8` | `4` |
| `status_bar` | `status_bar_container` | `PANEL` | `{BORDER, 1.0, 0.0}` | `[0, 8]` | (不用,该函数不自建 spacing) |
| `maximize_overlay` | `maximize_overlay`(复合:scrim + border 两部分) | scrim=`SCRIM`,padding 40 | `{GOLD, 1.5, 10.0}` | — | — |
| `context_menu` | `context_menu_popup` 的 `list` | `CARD` | `{BORDER, 1.0, 6.0}` | `6` | `2` |

`padding` 数组约定:`[top, right, bottom, left]`(与 iced `core::Padding` 字段顺序一致,已用 iced 源码核对),`[v, h]` 两元素简写(`top=bottom=v, right=left=h`)同 iced 自身 `From<[f32;2]>` 语义,单个数字表示四边相同。

---

### Task 1: `theme.rs` 新增 `SCRIM` 常量

**Files:**
- Modify: `crates/dozer-app/src/theme.rs`

**Interfaces:**
- Produces: `pub const theme::SCRIM: Color`,供 Task 2/3 的 `maximize_overlay` scrim 颜色引用。

- [ ] **Step 1: 加常量**

在 `crates/dozer-app/src/theme.rs` 里,`pub const RED: Color = c(0xFF, 0x6E, 0x6E);` 那一行之后加:

```rust
/// 放大态浮层的变暗遮罩色(半透明黑)。不在设计规格锁死的 14 色之内——
/// 之前是 `maximize_overlay` 函数里的游离字面量,这里给它转正成具名令牌,
/// 值不变,纯增量,不改动上面 14 个锁定颜色。
pub const SCRIM: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.55,
};
```

- [ ] **Step 2: 加回归测试**

在 `theme.rs` 底部 `mod tests` 里(紧跟 `cream_matches_rgb8_conversion` 之后)加:

```rust
    #[test]
    fn scrim_is_half_transparent_black() {
        assert_eq!(SCRIM.r, 0.0);
        assert_eq!(SCRIM.g, 0.0);
        assert_eq!(SCRIM.b, 0.0);
        assert_eq!(SCRIM.a, 0.55);
    }
```

- [ ] **Step 3: 验证**

Run: `cargo test -p dozer-app scrim_is_half_transparent_black`
Expected: `test theme::tests::scrim_is_half_transparent_black ... ok`

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/theme.rs
git commit -m "feat(app): 新增 theme::SCRIM 常量(放大态遮罩色转正)"
```

---

### Task 2: 新建 `regions.json` + 补 spec 文档缺漏

**Files:**
- Create: `crates/dozer-app/assets/theme/regions.json`
- Modify: `docs/superpowers/specs/2026-07-30-dozer-shell-chrome-style-config-design.md`

**Interfaces:**
- Produces: 磁盘上的 JSON 文件,内容即 Task 3 `include_str!` 的输入,13 个顶层 key,形状见下方 Step 1。

- [ ] **Step 1: 建目录 + 写文件**

先确认目录存在:

```bash
mkdir -p crates/dozer-app/assets/theme
```

写入 `crates/dozer-app/assets/theme/regions.json`:

```json
{
  "top_bar": {
    "background": "BG",
    "border": { "color": "BORDER", "width": 0.0, "radius": 0.0 },
    "padding": [0, 12],
    "gap": 16
  },
  "left_icon_rail": {
    "background": "PANEL",
    "border": null,
    "padding": [16, 6, 0, 6],
    "gap": 12
  },
  "right_icon_rail": {
    "background": "PANEL",
    "border": null,
    "padding": [16, 6, 0, 6],
    "gap": 12
  },
  "project_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 8,
    "gap": 4
  },
  "preview_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 8,
    "gap": 4
  },
  "browser_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 8,
    "gap": 4
  },
  "agent_list_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 12,
    "gap": 8
  },
  "terminal_pane": {
    "background": "TERM_BG",
    "border": null,
    "padding": 8,
    "gap": 4
  },
  "conversation_list_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 12,
    "gap": 8
  },
  "review_content_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 8,
    "gap": 4
  },
  "status_bar": {
    "background": "PANEL",
    "border": { "color": "BORDER", "width": 1.0, "radius": 0.0 },
    "padding": [0, 8]
  },
  "maximize_overlay": {
    "scrim": { "background": "SCRIM", "padding": 40.0 },
    "border": { "color": "GOLD", "width": 1.5, "radius": 10.0 }
  },
  "context_menu": {
    "background": "CARD",
    "border": { "color": "BORDER", "width": 1.0, "radius": 6.0 },
    "padding": 6,
    "gap": 2
  }
}
```

- [ ] **Step 2: 补 spec 文档的区域表**

在 `docs/superpowers/specs/2026-07-30-dozer-shell-chrome-style-config-design.md` 的 §2 表格("外壳骨架(5)")最后一行之后,加一行说明发现了第 13 个区域:

在该文件 `| \`context_menu_popup\` | ... |` 那一行之后加一行:

```markdown
| `status_bar` | `status_bar_container`(项目栏底/终端栏底共用) | 背景 `PANEL`;边框 `{BORDER, width 1, radius 0}`;padding `[0,8]`(实现阶段核对代码后补充,原表遗漏此项——见 `docs/superpowers/plans/2026-07-30-dozer-shell-chrome-style-config.md` 开头说明) |
```

- [ ] **Step 3: 校验 JSON 语法**

Run: `python3 -m json.tool crates/dozer-app/assets/theme/regions.json > /dev/null && echo OK`
Expected: `OK`

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/theme/regions.json docs/superpowers/specs/2026-07-30-dozer-shell-chrome-style-config-design.md
git commit -m "feat(app): 新增 regions.json(外壳区域样式源数据)"
```

---

### Task 3: `chrome_style.rs` 模块——解析 + 访问函数 + 回归测试

**Files:**
- Create: `crates/dozer-app/src/chrome_style.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod chrome_style;`)
- Modify: `crates/dozer-app/src/workspace.rs`(加 `use crate::chrome_style;`,供 Task 4-15 调用)

**Interfaces:**
- Consumes: `crate::theme::{BG, PANEL, TERM_BG, CARD, BORDER, CREAM, BODY, DIM, GOLD, CYAN, GREEN, PURPLE, RED, SCRIM}`(均为 `Color`);`iced_widget::core::{Border, Color, Padding}`。
- Produces:
  - `pub struct RegionStyle { pub background: Option<Color>, pub border: Option<Border>, pub padding: Padding, pub gap: f32 }`(`Clone, Copy`)
  - `pub struct MaximizeOverlayStyle { pub scrim_background: Color, pub scrim_padding: f32, pub border: Border }`(`Clone, Copy`)
  - 13 个访问函数,签名均为 `pub fn <key>() -> RegionStyle`(除 `maximize_overlay() -> MaximizeOverlayStyle`),函数名与 JSON key 一致:`top_bar()`、`left_icon_rail()`、`right_icon_rail()`、`project_pane()`、`preview_pane()`、`browser_pane()`、`agent_list_pane()`、`terminal_pane()`、`conversation_list_pane()`、`review_content_pane()`、`status_bar()`、`maximize_overlay()`、`context_menu()`。

- [ ] **Step 1: 写模块**

创建 `crates/dozer-app/src/chrome_style.rs`:

```rust
//! 外壳区域样式配置:13 个区域(主导航/左右图标栏/放大态浮层/右键菜单/
//! 状态条 + 7 个面板内容区)各自外层容器的背景色/边框/内边距/子元素
//! 间距,编译期内嵌 `assets/theme/regions.json`,启动时解析一次。
//!
//! 颜色字段是字符串,引用 `theme.rs` 现成的令牌名——`theme.rs` 仍是
//! 颜色数值唯一真相源,这里只负责"这个区域用哪个令牌"。解析失败(格式
//! 错误、未知颜色令牌名)直接 panic:这是编译期就该发现的开发期配置
//! 错误,不是需要优雅降级的运行时数据(同 `theme.rs` 14 色的定位)。
//!
//! 只覆盖区域**外层容器**的样式;区域内部控件的 active/hover/pressed
//! 等交互态样式(如 `rail_icon_button`)不在这里,留在 Rust 代码里。
use crate::theme;
use iced_widget::core::{Border, Color, Padding};
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/theme/regions.json");

/// JSON 里 `padding` 字段的三种形状:单值(四边相同)/ `[v,h]` / `[top,right,bottom,left]`。
/// 数组顺序与 iced `core::Padding` 字段顺序一致(已用 iced 源码核对)。
#[derive(Deserialize)]
#[serde(untagged)]
enum RawPadding {
    Uniform(f32),
    TwoAxis([f32; 2]),
    Four([f32; 4]),
}

impl From<RawPadding> for Padding {
    fn from(p: RawPadding) -> Self {
        match p {
            RawPadding::Uniform(v) => Padding::from(v),
            RawPadding::TwoAxis(a) => Padding::from(a),
            RawPadding::Four([top, right, bottom, left]) => Padding {
                top,
                right,
                bottom,
                left,
            },
        }
    }
}

#[derive(Deserialize)]
struct RawBorder {
    color: String,
    width: f32,
    radius: f32,
}

#[derive(Deserialize)]
struct RawRegion {
    background: Option<String>,
    border: Option<RawBorder>,
    padding: RawPadding,
    #[serde(default)]
    gap: f32,
}

#[derive(Deserialize)]
struct RawScrim {
    background: String,
    padding: f32,
}

#[derive(Deserialize)]
struct RawMaximizeOverlay {
    scrim: RawScrim,
    border: RawBorder,
}

#[derive(Deserialize)]
struct RawRegions {
    top_bar: RawRegion,
    left_icon_rail: RawRegion,
    right_icon_rail: RawRegion,
    project_pane: RawRegion,
    preview_pane: RawRegion,
    browser_pane: RawRegion,
    agent_list_pane: RawRegion,
    terminal_pane: RawRegion,
    conversation_list_pane: RawRegion,
    review_content_pane: RawRegion,
    status_bar: RawRegion,
    maximize_overlay: RawMaximizeOverlay,
    context_menu: RawRegion,
}

/// 颜色令牌名 → `theme.rs` 常量。未知名字直接 panic——配置写错在启动时
/// 就能发现,不会带着错误的透明色静默跑起来。
fn resolve_color(name: &str) -> Color {
    match name {
        "BG" => theme::BG,
        "PANEL" => theme::PANEL,
        "TERM_BG" => theme::TERM_BG,
        "CARD" => theme::CARD,
        "BORDER" => theme::BORDER,
        "CREAM" => theme::CREAM,
        "BODY" => theme::BODY,
        "DIM" => theme::DIM,
        "GOLD" => theme::GOLD,
        "CYAN" => theme::CYAN,
        "GREEN" => theme::GREEN,
        "PURPLE" => theme::PURPLE,
        "RED" => theme::RED,
        "SCRIM" => theme::SCRIM,
        other => panic!("regions.json: 未知颜色令牌 \"{other}\""),
    }
}

fn resolve_border(b: &RawBorder) -> Border {
    Border {
        color: resolve_color(&b.color),
        width: b.width,
        radius: b.radius.into(),
    }
}

#[derive(Clone, Copy)]
pub struct RegionStyle {
    pub background: Option<Color>,
    pub border: Option<Border>,
    pub padding: Padding,
    pub gap: f32,
}

fn resolve_region(r: RawRegion) -> RegionStyle {
    RegionStyle {
        background: r.background.as_deref().map(resolve_color),
        border: r.border.as_ref().map(resolve_border),
        padding: r.padding.into(),
        gap: r.gap,
    }
}

#[derive(Clone, Copy)]
pub struct MaximizeOverlayStyle {
    pub scrim_background: Color,
    pub scrim_padding: f32,
    pub border: Border,
}

struct ResolvedRegions {
    top_bar: RegionStyle,
    left_icon_rail: RegionStyle,
    right_icon_rail: RegionStyle,
    project_pane: RegionStyle,
    preview_pane: RegionStyle,
    browser_pane: RegionStyle,
    agent_list_pane: RegionStyle,
    terminal_pane: RegionStyle,
    conversation_list_pane: RegionStyle,
    review_content_pane: RegionStyle,
    status_bar: RegionStyle,
    maximize_overlay: MaximizeOverlayStyle,
    context_menu: RegionStyle,
}

fn load(raw: &str) -> ResolvedRegions {
    let parsed: RawRegions =
        serde_json::from_str(raw).expect("regions.json 格式错误(解析失败)");
    ResolvedRegions {
        top_bar: resolve_region(parsed.top_bar),
        left_icon_rail: resolve_region(parsed.left_icon_rail),
        right_icon_rail: resolve_region(parsed.right_icon_rail),
        project_pane: resolve_region(parsed.project_pane),
        preview_pane: resolve_region(parsed.preview_pane),
        browser_pane: resolve_region(parsed.browser_pane),
        agent_list_pane: resolve_region(parsed.agent_list_pane),
        terminal_pane: resolve_region(parsed.terminal_pane),
        conversation_list_pane: resolve_region(parsed.conversation_list_pane),
        review_content_pane: resolve_region(parsed.review_content_pane),
        status_bar: resolve_region(parsed.status_bar),
        maximize_overlay: MaximizeOverlayStyle {
            scrim_background: resolve_color(&parsed.maximize_overlay.scrim.background),
            scrim_padding: parsed.maximize_overlay.scrim.padding,
            border: resolve_border(&parsed.maximize_overlay.border),
        },
        context_menu: resolve_region(parsed.context_menu),
    }
}

static REGIONS: LazyLock<ResolvedRegions> = LazyLock::new(|| load(RAW));

pub fn top_bar() -> RegionStyle {
    REGIONS.top_bar
}
pub fn left_icon_rail() -> RegionStyle {
    REGIONS.left_icon_rail
}
pub fn right_icon_rail() -> RegionStyle {
    REGIONS.right_icon_rail
}
pub fn project_pane() -> RegionStyle {
    REGIONS.project_pane
}
pub fn preview_pane() -> RegionStyle {
    REGIONS.preview_pane
}
pub fn browser_pane() -> RegionStyle {
    REGIONS.browser_pane
}
pub fn agent_list_pane() -> RegionStyle {
    REGIONS.agent_list_pane
}
pub fn terminal_pane() -> RegionStyle {
    REGIONS.terminal_pane
}
pub fn conversation_list_pane() -> RegionStyle {
    REGIONS.conversation_list_pane
}
pub fn review_content_pane() -> RegionStyle {
    REGIONS.review_content_pane
}
pub fn status_bar() -> RegionStyle {
    REGIONS.status_bar
}
pub fn maximize_overlay() -> MaximizeOverlayStyle {
    REGIONS.maximize_overlay
}
pub fn context_menu() -> RegionStyle {
    REGIONS.context_menu
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:关键区域的解析结果必须和改动前的字面量完全一致——纯
    /// 代码搬家,数值不该变。以后有人手滑改错配置文件,这个测试会炸。
    #[test]
    fn preview_pane_matches_pre_migration_literals() {
        let s = preview_pane();
        assert_eq!(s.background, Some(theme::PANEL));
        assert!(s.border.is_none());
        assert_eq!(s.padding, Padding::from(8.0));
        assert_eq!(s.gap, 4.0);
    }

    #[test]
    fn top_bar_matches_pre_migration_literals() {
        let s = top_bar();
        assert_eq!(s.background, Some(theme::BG));
        let border = s.border.expect("top_bar 应有边框条目(width=0,视觉不可见)");
        assert_eq!(border.color, theme::BORDER);
        assert_eq!(border.width, 0.0);
        assert_eq!(s.padding, Padding::from([0.0, 12.0]));
        assert_eq!(s.gap, 16.0);
    }

    #[test]
    fn left_icon_rail_matches_pre_migration_literals() {
        let s = left_icon_rail();
        assert_eq!(s.background, Some(theme::PANEL));
        assert!(s.border.is_none());
        assert_eq!(
            s.padding,
            Padding {
                top: 16.0,
                right: 6.0,
                bottom: 0.0,
                left: 6.0,
            }
        );
        assert_eq!(s.gap, 12.0);
    }

    #[test]
    fn status_bar_matches_pre_migration_literals() {
        let s = status_bar();
        assert_eq!(s.background, Some(theme::PANEL));
        let border = s.border.expect("status_bar 应有上边线");
        assert_eq!(border.color, theme::BORDER);
        assert_eq!(border.width, 1.0);
        assert_eq!(s.padding, Padding::from([0.0, 8.0]));
    }

    #[test]
    fn maximize_overlay_matches_pre_migration_literals() {
        let s = maximize_overlay();
        assert_eq!(s.scrim_background, theme::SCRIM);
        assert_eq!(s.scrim_padding, 40.0);
        assert_eq!(s.border.color, theme::GOLD);
        assert_eq!(s.border.width, 1.5);
    }

    #[test]
    fn context_menu_matches_pre_migration_literals() {
        let s = context_menu();
        assert_eq!(s.background, Some(theme::CARD));
        let border = s.border.expect("context_menu 应有边框");
        assert_eq!(border.color, theme::BORDER);
        assert_eq!(border.width, 1.0);
        assert_eq!(s.padding, Padding::from(6.0));
        assert_eq!(s.gap, 2.0);
    }

    #[test]
    #[should_panic(expected = "未知颜色令牌")]
    fn unknown_color_token_panics() {
        resolve_color("NOT_A_REAL_TOKEN");
    }
}
```

- [ ] **Step 2: 接线 `mod`(main.rs)**

`crates/dozer-app/src/main.rs` 顶部 `mod` 列表(第 1-17 行),在 `mod assets;` 之后、`mod conversation;` 之前插入:

```rust
mod chrome_style;
```

- [ ] **Step 3: 接线 `use`(workspace.rs)**

后续 Task 4-15 都要在 `workspace.rs` 里调用 `chrome_style::` 开头的函数(不带 `crate::` 前缀,跟这个文件里 `icons`/`theme`/`preview` 等模块已有的引用习惯一致)。在 `crates/dozer-app/src/workspace.rs` 顶部的 `use crate::` 导入块里,`use crate::icons;` 那一行之前插入:

```rust
use crate::chrome_style;
```

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app chrome_style`
Expected: 7 个测试全部 `ok`(`preview_pane_matches_pre_migration_literals`、
`top_bar_matches_pre_migration_literals`、`left_icon_rail_matches_pre_migration_literals`、
`status_bar_matches_pre_migration_literals`、`maximize_overlay_matches_pre_migration_literals`、
`context_menu_matches_pre_migration_literals`、`unknown_color_token_panics`)

- [ ] **Step 5: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error(此时 `chrome_style` 的公开函数在 `workspace.rs` 里还没有调用方,`unused import`/`dead_code` 警告是预期的,Task 4 起逐个消掉)

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/chrome_style.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(app): chrome_style 模块——解析 regions.json 为区域样式访问函数"
```

---

### Task 4: 接线 `top_bar`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`top_bar` 函数,`grep -n "^fn top_bar" crates/dozer-app/src/workspace.rs` 现查行号)

**Interfaces:**
- Consumes: `chrome_style::top_bar() -> chrome_style::RegionStyle`

- [ ] **Step 1: 改样式来源**

把:

```rust
    let bar = row![title, search, iced_widget::space::horizontal(), right]
        .spacing(16)
        .padding([0, 12])
        .align_y(iced_widget::core::Alignment::Center);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(TOP_BAR_HEIGHT))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BG.into()),
            border: Border {
                color: theme::BORDER,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
```

改成:

```rust
    let region = chrome_style::top_bar();
    let bar = row![title, search, iced_widget::space::horizontal(), right]
        .spacing(region.gap)
        .padding(region.padding)
        .align_y(iced_widget::core::Alignment::Center);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(TOP_BAR_HEIGHT))
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,确认顶栏外观(标题/搜索框/齿轮位置、背景色、内边距)与改动前截图一致。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): top_bar 样式改读 chrome_style 配置"
```

---

### Task 5: 接线 `left_icon_rail` / `right_icon_rail`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`left_icon_rail`、`right_icon_rail` 函数)

**Interfaces:**
- Consumes: `chrome_style::{left_icon_rail, right_icon_rail}() -> chrome_style::RegionStyle`

- [ ] **Step 1: 改 `left_icon_rail`**

把:

```rust
    let content = column![
        rail_icon_button(
            icons::IconKind::Folder,
            ws.left_view == LeftView::Files,
            Message::LeftIconSelect(LeftView::Files),
        ),
        rail_icon_button(
            icons::IconKind::Globe,
            ws.left_view == LeftView::Web,
            Message::LeftIconSelect(LeftView::Web),
        ),
    ]
    .spacing(12)
    .padding(Padding {
        top: 16.0,
        left: 6.0,
        right: 6.0,
        ..Padding::ZERO
    });

    container(content)
        .width(Length::Fixed(ICON_RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
}

/// 右图标栏:Agent / 对话两个图标,语义同 `left_icon_rail`。
fn right_icon_rail(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let content = column![
        rail_icon_button(
            icons::IconKind::Bot,
            ws.right_view == RightView::Agent,
            Message::RightIconSelect(RightView::Agent),
        ),
        rail_icon_button(
            icons::IconKind::MessageSquare,
            ws.right_view == RightView::Conversations,
            Message::RightIconSelect(RightView::Conversations),
        ),
    ]
    .spacing(12)
    .padding(Padding {
        top: 16.0,
        left: 6.0,
        right: 6.0,
        ..Padding::ZERO
    });

    container(content)
        .width(Length::Fixed(ICON_RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
```

改成(两处 `column![...]` 各自独立读各自 key,不合并成一个共享变量——保持 spec 里"每区域一份直给"的决定):

```rust
    let region = chrome_style::left_icon_rail();
    let content = column![
        rail_icon_button(
            icons::IconKind::Folder,
            ws.left_view == LeftView::Files,
            Message::LeftIconSelect(LeftView::Files),
        ),
        rail_icon_button(
            icons::IconKind::Globe,
            ws.left_view == LeftView::Web,
            Message::LeftIconSelect(LeftView::Web),
        ),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(ICON_RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 右图标栏:Agent / 对话两个图标,语义同 `left_icon_rail`。
fn right_icon_rail(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::right_icon_rail();
    let content = column![
        rail_icon_button(
            icons::IconKind::Bot,
            ws.right_view == RightView::Agent,
            Message::RightIconSelect(RightView::Agent),
        ),
        rail_icon_button(
            icons::IconKind::MessageSquare,
            ws.right_view == RightView::Conversations,
            Message::RightIconSelect(RightView::Conversations),
        ),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(ICON_RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error(注意:改完之后 `Padding` 若不再被其它代码使用会有 unused import 警告——检查 `workspace.rs` 顶部 `use iced_widget::core::{Border, Color, Element, Length, Padding};`,若别处(如 `context_menu_popup` 的定位 `Padding`)仍在用则不用改;若警告出现,保留 `Padding` 导入不动,只是这两处不再手写字面量)

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,确认左右图标栏外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): left_icon_rail/right_icon_rail 样式改读 chrome_style 配置"
```

---

### Task 6: 接线 `project_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`project_pane` 函数末尾的 `body` 容器)

- [ ] **Step 1: 改样式来源**

函数开头把:

```rust
    let mut content = column![text("项目").size(14).color(theme::CREAM)].spacing(4);
```

改成:

```rust
    let region = chrome_style::project_pane();
    let mut content = column![text("项目").size(14).color(theme::CREAM)].spacing(region.gap);
```

函数末尾把:

```rust
    let body = container(content.padding(8))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            // 面板不再自带边框,原因同 conversation_list_pane。
            ..container::Style::default()
        });

    container(column![body, project_status_bar(ws)])
        .width(width)
        .height(Length::Fill)
        .into()
```

改成:

```rust
    let body = container(content.padding(region.padding))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        });

    container(column![body, project_status_bar(ws)])
        .width(width)
        .height(Length::Fill)
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,打开一个项目,确认项目树面板外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): project_pane 样式改读 chrome_style 配置"
```

---

### Task 7: 接线 `preview_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`preview_pane` 函数)

- [ ] **Step 1: 改样式来源**

函数开头(`let widths: Vec<f32> = ws.preview...` 那一行之前)加:

```rust
    let region = chrome_style::preview_pane();
```

把函数里的 `let mut content = column![tab_bar, tab_divider()].spacing(4);` 改成:

```rust
    let mut content = column![tab_bar, tab_divider()].spacing(region.gap);
```

把函数末尾:

```rust
    container(content.padding(8))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            // 面板不再自带边框,原因同 conversation_list_pane。
            ..container::Style::default()
        })
        .into()
```

改成:

```rust
    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,打开一个文件预览,确认面板外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): preview_pane 样式改读 chrome_style 配置"
```

---

### Task 8: 接线 `browser_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`browser_pane` 函数)

- [ ] **Step 1: 改样式来源**

函数开头加:

```rust
    let region = chrome_style::browser_pane();
```

把 `let mut content = column![tab_bar, tab_divider(), addr].spacing(4);` 改成:

```rust
    let mut content = column![tab_bar, tab_divider(), addr].spacing(region.gap);
```

把函数末尾:

```rust
    container(content.padding(8))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
```

改成:

```rust
    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,点左图标栏"地球"图标切到浏览器视图,确认外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): browser_pane 样式改读 chrome_style 配置"
```

---

### Task 9: 接线 `agent_list_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`agent_list_pane` 函数)

- [ ] **Step 1: 改样式来源**

把:

```rust
    let content = column![
        row![
            text("Agent").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8),
        text("Agents（后续）").size(13).color(theme::DIM),
    ]
    .spacing(8);

    container(content.padding(12))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            // 面板不再自带边框,原因同 conversation_list_pane。
            ..container::Style::default()
        })
        .into()
```

改成:

```rust
    let region = chrome_style::agent_list_pane();
    let content = column![
        row![
            text("Agent").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8),
        text("Agents（后续）").size(13).color(theme::DIM),
    ]
    .spacing(region.gap);

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

(内层 `row![...].spacing(8)` 是标题行内部的元素间距,不是本区域的顶层 `gap`,不改。)

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,点右图标栏 Agent 图标,确认面板外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): agent_list_pane 样式改读 chrome_style 配置"
```

---

### Task 10: 接线 `terminal_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`terminal_pane` 函数)

- [ ] **Step 1: 改样式来源**

函数开头把 `let mut content = column![tab_bar(ws)].spacing(4);` 改成:

```rust
    let region = chrome_style::terminal_pane();
    let mut content = column![tab_bar(ws)].spacing(region.gap);
```

把函数里的:

```rust
    let body = container(content.spacing(4).padding(8))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::TERM_BG.into()),
            // 面板不再自带边框,原因同 conversation_list_pane。
            ..container::Style::default()
        });
```

改成:

```rust
    let body = container(content.spacing(region.gap).padding(region.padding))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        });
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,确认终端面板外观(含背景色 `TERM_BG`)不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): terminal_pane 样式改读 chrome_style 配置"
```

---

### Task 11: 接线 `conversation_list_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`conversation_list_pane` 函数)

- [ ] **Step 1: 改样式来源**

函数开头(`let mut content = column![` 那一行前)加:

```rust
    let region = chrome_style::conversation_list_pane();
```

把函数最外层的两处 `.spacing(8)`(分别在最外层 `column![row![...].spacing(8)].spacing(8)` 的**外层** `column!` 上——即整个函数体最顶层的 `content` 定义那处,不是标题 `row!` 内部那个)改成 `.spacing(region.gap)`。具体地,把:

```rust
    let mut content = column![
        row![
            text("对话").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8)
    ]
    .spacing(8);
```

改成:

```rust
    let mut content = column![
        row![
            text("对话").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8)
    ]
    .spacing(region.gap);
```

(内层标题行的 `.spacing(8)` 保持字面量不变——那是行内元素间距,不是本区域的顶层 gap。)

把函数末尾:

```rust
    container(content.padding(12))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            // 面板不再自带边框(去重复线,与 divider_bar 合并为单线,见
            // divider_bar 上方注释)。左右边界靠 PANEL 与相邻元素的背景色差分。
            ..container::Style::default()
        })
        .into()
```

改成:

```rust
    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,点右图标栏"对话"图标,确认面板外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): conversation_list_pane 样式改读 chrome_style 配置"
```

---

### Task 12: 接线 `review_content_pane`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`review_content_pane` 函数)

- [ ] **Step 1: 改样式来源**

函数开头(`let maximize_btn = ...` 那一行前)加:

```rust
    let region = chrome_style::review_content_pane();
```

把 `let mut content = column![header].spacing(4);` 改成:

```rust
    let mut content = column![header].spacing(region.gap);
```

把函数末尾:

```rust
    container(content.padding(8))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
```

改成:

```rust
    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,打开一条对话审阅,确认面板外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): review_content_pane 样式改读 chrome_style 配置"
```

---

### Task 13: 接线 `status_bar_container`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`status_bar_container` 函数)

- [ ] **Step 1: 改样式来源**

把:

```rust
fn status_bar_container<'a>(
    inner: impl Into<Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    container(inner)
        .width(Length::Fill)
        .height(Length::Fixed(STATUS_BAR_HEIGHT))
        .padding([0, 8])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
```

改成:

```rust
fn status_bar_container<'a>(
    inner: impl Into<Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::status_bar();
    container(inner)
        .width(Length::Fill)
        .height(Length::Fixed(STATUS_BAR_HEIGHT))
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,确认项目栏底、终端栏底两条状态条外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): status_bar_container 样式改读 chrome_style 配置"
```

---

### Task 14: 接线 `maximize_overlay`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`maximize_overlay` 函数)

- [ ] **Step 1: 改样式来源**

把:

```rust
    let bordered = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: Border {
                color: theme::GOLD,
                width: 1.5,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        });
```

改成:

```rust
    let overlay_style = chrome_style::maximize_overlay();
    let bordered = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: overlay_style.border,
            ..container::Style::default()
        });
```

把:

```rust
    let dim_bg = MouseArea::new(
        container(content_guard)
            .padding(MAXIMIZE_OVERLAY_PADDING)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(
                    Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.55,
                    }
                    .into(),
                ),
                ..container::Style::default()
            }),
    )
    .on_press(Message::MaximizeClose);
```

改成:

```rust
    let dim_bg = MouseArea::new(
        container(content_guard)
            .padding(overlay_style.scrim_padding)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(overlay_style.scrim_background.into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::MaximizeClose);
```

注意:`MAXIMIZE_OVERLAY_PADDING` 常量(几何定位常量,`preview_content_bounds`/`is_in_preview_column` 也在用同一个值换算放大盒子坐标)**不删除**——那两个几何函数仍然直接用这个常量算坐标,不经过 `chrome_style`(spec §7 只覆盖视觉样式,不覆盖几何换算)。`overlay_style.scrim_padding` 的数值(40.0)和 `MAXIMIZE_OVERLAY_PADDING` 现在是两份各自独立的"40.0"字面量来源——这是本次迁移刻意接受的重复(否则要把一个几何常量也塞进样式配置,超出 spec 范围),Task 3 的 `maximize_overlay_matches_pre_migration_literals` 测试已经锁死 `scrim_padding == 40.0`,两边一旦不一致测试会先炸,不会静默漂移。

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error(`Color` 若在这个文件其它地方仍被使用则导入不用动)

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,点某个内容 pane 的放大按钮,确认变暗遮罩 + 金色描边盒子外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): maximize_overlay 样式改读 chrome_style 配置"
```

---

### Task 15: 接线 `context_menu_popup`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`context_menu_popup` 函数)

- [ ] **Step 1: 改样式来源**

把:

```rust
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
```

改成:

```rust
    let region = chrome_style::context_menu();
    let list = container(column(items).spacing(region.gap))
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        });
```

不改动紧接着的定位用 `container(list).padding(Padding{top: menu.y, left: menu.x, ..})`(纯几何定位,不属于本次样式配置范围,spec §4 已明确)。

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 无 error

- [ ] **Step 3: 真机目测**

Run: `cargo run -p dozer-app`,在项目树右键弹出菜单,确认外观不变。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(app): context_menu_popup 样式改读 chrome_style 配置"
```

---

### Task 16: 全量验证 + 收尾

**Files:**
- 无新改动(除非验证发现回归,回去修对应 Task 的文件)

- [ ] **Step 1: 全量测试**

Run: `cargo test -p dozer-app 2>&1 | tail -20`
Expected: 全部 `ok`(含 Task 1/3 新增的测试),数量应比迁移前多 7(`SCRIM` 1 个 + `chrome_style` 6 个)

- [ ] **Step 2: clippy**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -60`
Expected: 无 warning/error

- [ ] **Step 3: fmt**

Run: `cargo fmt -p dozer-app && git diff --stat`
Expected: 若 fmt 有格式化改动,`git add -u && git commit -m "style(app): cargo fmt"`;若无改动则跳过

- [ ] **Step 4: 全量真机目测**

Run: `cargo run -p dozer-app`,依次过一遍:顶栏、左右图标栏、文件预览、Web 浏览器、项目树、Agent 面板、终端、对话列表、对话审阅、右键菜单、放大态浮层、两条状态条——确认 13 个区域外观与改动前(可对照 `git log` 里 Task 1 之前的版本跑一次作为基准)逐一一致,无任何肉眼可见差异。

- [ ] **Step 5: 检查是否有遗漏的散落字面量**

Run:
```bash
grep -n "background: Some(theme::" crates/dozer-app/src/workspace.rs | grep -v "chrome_style\|button::Style\|region\."
```
Expected: 只剩区域内部控件(按钮/tab 活跃态等,`rail_icon_button`/`tab_arrow_button`/`tab_item`/各种 `button::Style` 相关)的 `background`,不应再出现任何一个**区域外层容器**（前面 13 个函数）的内联 `theme::PANEL`/`theme::BG`/`theme::TERM_BG`/`theme::CARD` 字面量。若发现遗漏,回去补上对应 Task。
