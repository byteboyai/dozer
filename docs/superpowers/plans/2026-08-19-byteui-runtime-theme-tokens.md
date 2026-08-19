# byteui font/geometry/icon_size 基础尺寸改为运行时可替换 token Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `byteui::theme::font`/`geometry`/`icon_size` 三个模块的静态基础尺寸从"编译期内嵌一份复制的 `workspace.json`"改成和 `theme::color` 完全一致的模式(Rust 编译期默认值 + 运行时 `set_theme()` 整体替换),删除 `byteui` 自带的 `workspace.json`,`dozer-app` 重新变回唯一真相源。

**Architecture:** 三个模块各自独立新增一个 `XxxTokens` 结构体(`Clone`+`Copy`+`Deserialize`)、一个 `byteboy2077()` const 默认值、`current()`/`set_theme()`(`RwLock` 包装,照抄 `color.rs` 的写法);原有 accessor 函数签名不变,内部从读 `LazyLock<...>` 静态量改成读 `current()`。`icon_size` 的运行时缩放机制(`AtomicU32`/`scale()`/`set_scale()`/`zoom_by()`/`init_scale`/`persist_scale`/`reset_scale`)完全不动。改完三个模块后,把 `byteui` 那份 `workspace.json` 里已经领先的 `font_sizes`(`b3f2763` 刚定的 10/11/12/13/14/15/16)同步回 `dozer-app` 自己的 `workspace.json`,删除 `byteui` 那份文件,`dozer-app` 新增 `theme::init()` 在启动时用自己的 JSON 调用三次 `set_theme()`。全程不改任何调用点(`byteui::theme::font::body()` 这类函数签名和调用方式一字不变)。

**Tech Stack:** Rust 2024,`serde`/`serde_json`(`byteui`/`dozer-app` 均已有此依赖),`std::sync::RwLock`(照抄 `byteui::theme::color` 现有模式)。

**Spec:** `docs/superpowers/specs/2026-08-19-byteui-runtime-theme-tokens-design.md`

## Global Constraints

- **独立分支开发,不直接提交 main。** 在新分支(如 `feature/byteui-runtime-theme-tokens`)上完成全部 6 个 Task,提请审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑 `git branch --show-current` 确认当前分支是自己的迁移分支——这个仓库检出目录经常被多个 agent/用户本人共享并行工作,过去已经在这条分支上撞见过对方未提交的改动,开工前先 `git status` 确认干净,发现不相关的改动要 `git stash push -u -m "..."` 保留而不是清掉。
- **不改变任何调用点。** `byteui::theme::font::body()`/`byteui::theme::geometry::icon_rail_width()` 这类函数的名字、签名、调用方式一字不动——四个迁移子项目已经改过的约 500+ 处调用点这次不需要碰。
- **`icon_size` 的运行时缩放机制不动。** `scale()`/`set_scale()`/`zoom_by()`/`init_scale(path)`/`persist_scale(path)`/`reset_scale(path)`/`SCALE_MIN`/`SCALE_MAX`/`load_from`/`save_to` 这套已经是"调用方传路径、不内置 Dozer 专属约定"的正确设计,这次只动 6 个静态尺寸(`rail`/`row`/`chevron`/`tab_arrow`/`home`/`tree_row_gap`)和 1 个基准缩放值(`scale` 字段,注意这是 `IconSizeTokens.scale` 而不是运行时的 `CURRENT_SCALE`,两者含义不同,不要混淆)。
- **`geometry.rs` 里 4 个函数不进 `GeometryTokens`,保持现状不动**:`default_split_ratio()`(硬编码 `0.35`)、`scrollbar_width()`(硬编码 `10.0 * icon_size::scale()`)、`scrollbar_thumb_width()`(硬编码 `4.0 * icon_size::scale()`)、`min_window_width()`(推导函数,调用 `icon_rail_width()`+`divider_width()`+`min_zone_width()` 算出)。这 4 个函数现在就不读 JSON,这次不新增字段把它们也塞进去——那是功能扩展,不在这次范围。
- **Task 4(同步 JSON + 删 `byteui` 那份文件)必须在 Task 1-3 全部完成之后才能做。** 中间状态下 `byteui` 那份 `workspace.json` 还要留着,否则还没改完的模块的 `include_str!` 找不到文件。
- **Task 5(`dozer-app::theme::init()`)必须在 Task 4 完成之后才能做**——需要 `dozer-app` 自己的 `workspace.json` 已经同步了最新 `font_sizes`,也需要三个 `byteui` 模块已经有 `set_theme()` 可调。

---

### Task 1: `byteui::theme::font` → `FontTokens` + 运行时 `set_theme`

**Files:**
- Modify: `crates/byteui/src/theme/font.rs`

**Interfaces:**
- Produces: `pub struct FontTokens { pub dot_sm: u32, pub caption_sm: u32, pub caption: u32, pub label: u32, pub body: u32, pub subtitle: u32, pub title: u32 }`(`Clone`+`Copy`+`Debug`+`Deserialize`),`FontTokens::byteboy2077() -> Self`,`pub fn current() -> FontTokens`,`pub fn set_theme(tokens: FontTokens)`。既有 `dot_sm()`/`caption_sm()`/`caption()`/`label()`/`body()`/`subtitle()`/`title()` 七个函数签名不变。
- Consumes: `super::icon_size::scale()`(不变,内部依赖)

- [ ] **Step 1: 用下面内容整体替换 `crates/byteui/src/theme/font.rs`**

```rust
//! 工作区（UI 控件）字号 token 化：`ByteBoy2077` 是编译期默认值，
//! `set_theme` 可在运行时整体替换成另一份产品的取值（同 `theme::color`
//! 的模式）——组件内部一律读 `current()`，不直接引用 `byteboy2077()`。
//!
//! 每个 accessor 返回的字号都乘过 `icon_size::scale()`（全局缩放因子），
//! 因此改 `scale` 即整体缩放全部控件文字，与图标尺寸同步。
use serde::Deserialize;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct FontTokens {
    pub dot_sm: u32,
    pub caption_sm: u32,
    pub caption: u32,
    pub label: u32,
    pub body: u32,
    pub subtitle: u32,
    pub title: u32,
}

impl FontTokens {
    /// 逐一对应 `dozer-app` 当前 `assets/theme/workspace.json` 的
    /// `font_sizes` 节点，仅作未显式 `set_theme()` 时的兜底默认值。
    pub const fn byteboy2077() -> Self {
        Self {
            dot_sm: 10,
            caption_sm: 11,
            caption: 12,
            label: 13,
            body: 14,
            subtitle: 15,
            title: 16,
        }
    }
}

static CURRENT: RwLock<FontTokens> = RwLock::new(FontTokens::byteboy2077());

/// 当前生效的字号 token（默认 ByteBoy2077）。
pub fn current() -> FontTokens {
    *CURRENT.read().expect("byteui font RwLock poisoned")
}

/// 整体替换当前字号 token——供调用方（如 `dozer-app::theme::init()`）在
/// 启动时用自己的 `workspace.json` 覆盖默认值。
pub fn set_theme(tokens: FontTokens) {
    *CURRENT.write().expect("byteui font RwLock poisoned") = tokens;
}

pub fn dot_sm() -> u32 {
    scale(current().dot_sm)
}
pub fn caption_sm() -> u32 {
    scale(current().caption_sm)
}
pub fn caption() -> u32 {
    scale(current().caption)
}
pub fn label() -> u32 {
    scale(current().label)
}
pub fn body() -> u32 {
    scale(current().body)
}
pub fn subtitle() -> u32 {
    scale(current().subtitle)
}
pub fn title() -> u32 {
    scale(current().title)
}

/// 把设计基准字号按全局 scale 折算成实际像素字号（四舍五入）。
fn scale(base: u32) -> u32 {
    ((base as f32) * super::icon_size::scale()).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：`byteboy2077()` 的每个字段值必须和 `dozer-app` 当前
    /// `assets/theme/workspace.json` 的 `font_sizes` 字面量一致。
    #[test]
    fn byteboy2077_matches_dozer_app_baseline() {
        let t = FontTokens::byteboy2077();
        assert_eq!(t.dot_sm, 10);
        assert_eq!(t.caption_sm, 11);
        assert_eq!(t.caption, 12);
        assert_eq!(t.label, 13);
        assert_eq!(t.body, 14);
        assert_eq!(t.subtitle, 15);
        assert_eq!(t.title, 16);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        let c = current();
        assert_eq!(c.body, FontTokens::byteboy2077().body);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = FontTokens::byteboy2077();
        custom.body = 99;
        set_theme(custom);
        assert_eq!(current().body, 99);
        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_theme(FontTokens::byteboy2077());
    }
}
```

- [ ] **Step 2: 编译验证(独立编译,不依赖 `dozer-app`)**

Run: `cargo build -p byteui`
Expected: 编译成功

- [ ] **Step 3: 测试验证**

Run: `cargo test -p byteui theme::font`
Expected: 3 个测试全部通过(`byteboy2077_matches_dozer_app_baseline`/`current_defaults_to_byteboy2077`/`set_theme_replaces_current_and_is_visible_globally`)

- [ ] **Step 4: 确认 `dozer-app` 仍能编译(此时 `dozer-app` 侧还没改动,call site 应该不受影响)**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/byteui/src/theme/font.rs
git commit -m "refactor(byteui): theme::font 改为运行时可替换 FontTokens,不再内嵌 JSON"
```

---

### Task 2: `byteui::theme::geometry` → `GeometryTokens` + 运行时 `set_theme`

**Files:**
- Modify: `crates/byteui/src/theme/geometry.rs`

**Interfaces:**
- Produces: `pub struct GeometryTokens { ... 30 个 pub f32 字段 ... }`(`Clone`+`Copy`+`Debug`+`Deserialize`),`GeometryTokens::byteboy2077() -> Self`,`pub fn current() -> GeometryTokens`,`pub fn set_theme(tokens: GeometryTokens)`。既有 32 个 accessor 签名不变。
- Consumes: `super::icon_size::scale()`(不变)

- [ ] **Step 1: 用下面内容整体替换 `crates/byteui/src/theme/geometry.rs`**

```rust
//! 外壳布局的几何常量(图标栏宽、分隔线宽、区域最小宽、窗口尺寸下限、
//! 顶栏/状态栏高、右键菜单尺寸、终端 chrome 开销估算、页签估算宽度等)。
//! `ByteBoy2077` 是编译期默认值,`set_theme` 可在运行时整体替换成另一份
//! 产品的取值(同 `theme::color`/`theme::font` 的模式)——组件内部一律
//! 读 `current()`,不直接引用 `byteboy2077()`。
use super::icon_size;
use serde::Deserialize;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct GeometryTokens {
    pub icon_rail_width: f32,
    pub divider_width: f32,
    pub min_zone_width: f32,
    pub min_split_ratio: f32,
    pub max_split_ratio: f32,
    pub initial_window_width: f32,
    pub initial_window_height: f32,
    pub min_window_height: f32,
    pub top_bar_height: f32,
    /// 顶栏项目页签的"默认/合适宽"(设计基准 200,已含全局 scale)。少数页签时
    /// 每片统一用这个宽(固定,左对齐不撑爆);页签多到塞不下这个宽时,
    /// `project_tabs_row` 按可用宽均分把它收窄到低于此值。它既是默认宽也是上限宽。
    pub project_tab_max_width: f32,
    /// 顶栏页签行最右"＋"按钮的估算宽(设计基准 36,已含全局 scale)。`project_tabs_row`
    /// 用它在布局期从可用宽里预留出"＋"的位置,避免页签在拥挤时被压到"＋"上。
    pub project_tab_add_button_width: f32,
    pub status_bar_height: f32,
    /// 底部 footbar 系统信息条高度（设计基准 22，已含全局 scale）。与
    /// `status_bar_height` 解耦——in-pane status bar 保持 26，footbar 单独更矮更紧凑。
    pub footbar_height: f32,
    pub context_menu_width: f32,
    pub context_menu_height: f32,
    pub chrome_width_px: f32,
    pub chrome_height_px: f32,
    pub preview_chrome_top_px: f32,
    pub browser_chrome_top_px: f32,
    pub maximize_overlay_padding: f32,
    pub project_tab_gap: f32,
    pub tab_bar_avail_px: f32,
    /// 左/右图标栏按钮（rail_icon_button）的方形命中区边长（设计基准 32）。
    pub rail_button_size: f32,
    /// tab 栏内小方形图标按钮通用命中区边长（关闭 × / 星标 / 收藏夹，
    /// 设计基准 24）。翻页箭头走更小的 `tab_arrow_button_size`。
    pub tab_button_size: f32,
    /// 翻页箭头按钮专属命中区边长，小于 `tab_button_size`（设计基准 18）。
    /// `tab_button_size` 同时给关闭 ×/星标/收藏夹按钮用，不能跟着箭头一起
    /// 缩小；箭头独立一个更紧凑的方形，让 `<`/`>` 的横向留白随之变窄。
    pub tab_arrow_button_size: f32,
    /// 右键菜单项（menu_item）固定宽（设计基准 180）；`context_menu_width`
    /// 由它 + 菜单列表左右 padding 推导，两者需同步缩放。
    pub menu_item_width: f32,
    /// 菜单项内"图标↔文字"间距（设计基准 8）。
    pub menu_gap: f32,
    /// 菜单项上下内边距（设计基准 6）。
    pub menu_pad_v: f32,
    /// 菜单项左右内边距（设计基准 10）。
    pub menu_pad_h: f32,
    /// H0 项目中心左栏固定宽（设计基准 248，Figma 同值）。
    pub h0_sidebar_width: f32,
}

impl GeometryTokens {
    /// 逐一对应 `dozer-app` 当前 `assets/theme/workspace.json` 的
    /// `geometry` 节点，仅作未显式 `set_theme()` 时的兜底默认值。
    pub const fn byteboy2077() -> Self {
        Self {
            icon_rail_width: 44.0,
            divider_width: 8.0,
            min_zone_width: 320.0,
            min_split_ratio: 0.2,
            max_split_ratio: 0.8,
            initial_window_width: 1440.0,
            initial_window_height: 900.0,
            min_window_height: 480.0,
            top_bar_height: 40.0,
            project_tab_max_width: 140.0,
            project_tab_add_button_width: 36.0,
            status_bar_height: 26.0,
            footbar_height: 22.0,
            context_menu_width: 180.0,
            context_menu_height: 310.0,
            chrome_width_px: 16.0,
            chrome_height_px: 50.0,
            preview_chrome_top_px: 38.0,
            browser_chrome_top_px: 72.0,
            maximize_overlay_padding: 40.0,
            project_tab_gap: 4.0,
            tab_bar_avail_px: 360.0,
            rail_button_size: 32.0,
            tab_button_size: 24.0,
            tab_arrow_button_size: 18.0,
            menu_item_width: 160.0,
            menu_gap: 8.0,
            menu_pad_v: 6.0,
            menu_pad_h: 10.0,
            h0_sidebar_width: 248.0,
        }
    }
}

static CURRENT: RwLock<GeometryTokens> = RwLock::new(GeometryTokens::byteboy2077());

/// 当前生效的几何 token（默认 ByteBoy2077）。
pub fn current() -> GeometryTokens {
    *CURRENT.read().expect("byteui geometry RwLock poisoned")
}

/// 整体替换当前几何 token——供调用方（如 `dozer-app::theme::init()`）在
/// 启动时用自己的 `workspace.json` 覆盖默认值。
pub fn set_theme(tokens: GeometryTokens) {
    *CURRENT.write().expect("byteui geometry RwLock poisoned") = tokens;
}

/// 图标栏固定宽度(逻辑像素)，左右各一条。已含全局 scale——`rail_button_size`
/// 同步缩放，否则放大后按钮会撑破图标栏。
pub fn icon_rail_width() -> f32 {
    current().icon_rail_width * icon_size::scale()
}

/// 每条分隔线的命中区/渲染宽度(逻辑像素)。视觉线本身 2px,居中于此区间内。
pub fn divider_width() -> f32 {
    current().divider_width
}

pub fn min_zone_width() -> f32 {
    current().min_zone_width
}

pub fn min_split_ratio() -> f32 {
    current().min_split_ratio
}

pub fn max_split_ratio() -> f32 {
    current().max_split_ratio
}

/// 左右双栏 zone 的统一"列表侧"默认占比——不进 `GeometryTokens`,硬编码
/// 字面量 0.35,现状如此,不属于这次改动范围。
pub fn default_split_ratio() -> f32 {
    0.35
}

/// 建窗时的初始窗口逻辑尺寸——仅在从未持久化过窗口尺寸时用作兜底。
pub fn initial_window_size() -> (f32, f32) {
    (current().initial_window_width, current().initial_window_height)
}

/// 高度最小值。已含全局 scale。
pub fn min_window_height() -> f32 {
    current().min_window_height * icon_size::scale()
}

/// 窗口最小内尺寸(逻辑像素,宽)。推导值,不进 `GeometryTokens`——由
/// `icon_rail_width`/`divider_width`/`min_zone_width` 三者算出,避免和
/// 它们各写各的、迟早对不上。
pub fn min_window_width() -> f32 {
    2.0 * icon_rail_width() + divider_width() + 2.0 * min_zone_width()
}

/// 顶栏固定高（逻辑像素）。已含全局 scale。
pub fn top_bar_height() -> f32 {
    current().top_bar_height * icon_size::scale()
}

/// 顶栏项目页签的"默认/合适宽"(逻辑像素),已含全局 scale。
pub fn project_tab_max_width() -> f32 {
    current().project_tab_max_width * icon_size::scale()
}

/// 顶栏页签行最右"＋"按钮的估算宽(逻辑像素),已含全局 scale。
pub fn project_tab_add_button_width() -> f32 {
    current().project_tab_add_button_width * icon_size::scale()
}

/// 统一滚动条(轨道)宽度(逻辑像素),已含全局 scale。不进 `GeometryTokens`,
/// 硬编码字面量 10.0,现状如此,不属于这次改动范围。
pub fn scrollbar_width() -> f32 {
    10.0 * icon_size::scale()
}

/// 统一滚动条滑块(thumb)宽度(逻辑像素),已含全局 scale。不进
/// `GeometryTokens`,硬编码字面量 4.0,现状如此,不属于这次改动范围。
pub fn scrollbar_thumb_width() -> f32 {
    4.0 * icon_size::scale()
}

/// 单条状态栏固定高（逻辑像素）。已含全局 scale。
pub fn status_bar_height() -> f32 {
    current().status_bar_height * icon_size::scale()
}

/// 底部 footbar 系统信息条高度（逻辑像素）。已含全局 scale。
pub fn footbar_height() -> f32 {
    current().footbar_height * icon_size::scale()
}

/// 右键菜单浮层的最坏情形外接宽/高（逻辑像素）。已含全局 scale。
pub fn context_menu_width() -> f32 {
    current().context_menu_width * icon_size::scale()
}

pub fn context_menu_height() -> f32 {
    current().context_menu_height * icon_size::scale()
}

/// 终端栏内"非网格"开销的近似值。已含全局 scale。
pub fn chrome_width_px() -> f32 {
    current().chrome_width_px * icon_size::scale()
}

pub fn chrome_height_px() -> f32 {
    current().chrome_height_px * icon_size::scale()
}

/// 文件预览分支内容区上方的 chrome 高度。已含全局 scale。
pub fn preview_chrome_top_px() -> f32 {
    current().preview_chrome_top_px * icon_size::scale()
}

/// 浏览器分支内容区上方的 chrome 高度。已含全局 scale。
pub fn browser_chrome_top_px() -> f32 {
    current().browser_chrome_top_px * icon_size::scale()
}

/// `maximize_overlay` 里 dim 背景到金色描边盒子的内边距(逻辑像素)。
pub fn maximize_overlay_padding() -> f32 {
    current().maximize_overlay_padding
}

/// 项目页签之间的间距。
pub fn project_tab_gap() -> f32 {
    current().project_tab_gap
}

/// 顶栏留给项目页签(裁剪窗口内)的估算可视宽,逻辑像素。
pub fn tab_bar_avail_px() -> f32 {
    current().tab_bar_avail_px
}

/// 左/右图标栏按钮方形命中区边长，已含全局 scale。
pub fn rail_button_size() -> f32 {
    current().rail_button_size * icon_size::scale()
}

/// 顶栏页签翻页箭头按钮方形命中区边长，已含全局 scale。
pub fn tab_button_size() -> f32 {
    current().tab_button_size * icon_size::scale()
}

/// 翻页箭头（`tab_arrow_button`）专属方形命中区边长，已含全局 scale。
pub fn tab_arrow_button_size() -> f32 {
    current().tab_arrow_button_size * icon_size::scale()
}

/// 右键菜单项固定宽，已含全局 scale。
pub fn menu_item_width() -> f32 {
    current().menu_item_width * icon_size::scale()
}

/// 菜单项内"图标↔文字"间距，已含全局 scale。
pub fn menu_gap() -> f32 {
    current().menu_gap * icon_size::scale()
}

/// 菜单项上下内边距，已含全局 scale。
pub fn menu_pad_v() -> f32 {
    current().menu_pad_v * icon_size::scale()
}

/// 菜单项左右内边距，已含全局 scale。
pub fn menu_pad_h() -> f32 {
    current().menu_pad_h * icon_size::scale()
}

/// H0 项目中心左栏固定宽（逻辑像素），已含全局 scale。
pub fn h0_sidebar_width() -> f32 {
    current().h0_sidebar_width * icon_size::scale()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：`byteboy2077()` 的每个字段值必须和 `dozer-app` 当前
    /// `assets/theme/workspace.json` 的 `geometry` 字面量一致。
    #[test]
    fn byteboy2077_matches_dozer_app_baseline() {
        let t = GeometryTokens::byteboy2077();
        assert_eq!(t.icon_rail_width, 44.0);
        assert_eq!(t.divider_width, 8.0);
        assert_eq!(t.min_zone_width, 320.0);
        assert_eq!(t.min_split_ratio, 0.2);
        assert_eq!(t.max_split_ratio, 0.8);
        assert_eq!(t.initial_window_width, 1440.0);
        assert_eq!(t.initial_window_height, 900.0);
        assert_eq!(t.min_window_height, 480.0);
        assert_eq!(t.top_bar_height, 40.0);
        assert_eq!(t.status_bar_height, 26.0);
        assert_eq!(t.footbar_height, 22.0);
        assert_eq!(t.context_menu_width, 180.0);
        assert_eq!(t.context_menu_height, 310.0);
        assert_eq!(t.chrome_width_px, 16.0);
        assert_eq!(t.chrome_height_px, 50.0);
        assert_eq!(t.preview_chrome_top_px, 38.0);
        assert_eq!(t.browser_chrome_top_px, 72.0);
        assert_eq!(t.maximize_overlay_padding, 40.0);
        assert_eq!(t.project_tab_gap, 4.0);
        assert_eq!(t.project_tab_max_width, 140.0);
        assert_eq!(t.tab_bar_avail_px, 360.0);
        assert_eq!(t.rail_button_size, 32.0);
        assert_eq!(t.tab_button_size, 24.0);
        assert_eq!(t.tab_arrow_button_size, 18.0);
        assert_eq!(t.menu_item_width, 160.0);
        assert_eq!(t.menu_gap, 8.0);
        assert_eq!(t.menu_pad_v, 6.0);
        assert_eq!(t.menu_pad_h, 10.0);
        assert_eq!(t.h0_sidebar_width, 248.0);
    }

    #[test]
    fn min_window_width_is_derived_not_duplicated() {
        assert_eq!(
            min_window_width(),
            2.0 * icon_rail_width() + divider_width() + 2.0 * min_zone_width()
        );
        assert_eq!(min_window_width(), 736.0);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        let c = current();
        assert_eq!(c.icon_rail_width, GeometryTokens::byteboy2077().icon_rail_width);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = GeometryTokens::byteboy2077();
        custom.icon_rail_width = 999.0;
        set_theme(custom);
        assert_eq!(current().icon_rail_width, 999.0);
        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_theme(GeometryTokens::byteboy2077());
    }
}
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p byteui`
Expected: 编译成功

- [ ] **Step 3: 测试验证**

Run: `cargo test -p byteui theme::geometry`
Expected: 4 个测试全部通过

- [ ] **Step 4: 确认 `dozer-app` 仍能编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/byteui/src/theme/geometry.rs
git commit -m "refactor(byteui): theme::geometry 改为运行时可替换 GeometryTokens,不再内嵌 JSON"
```

---

### Task 3: `byteui::theme::icon_size` → `IconSizeTokens` + 运行时 `set_theme`(缩放机制不动)

**Files:**
- Modify: `crates/byteui/src/theme/icon_size.rs`

**Interfaces:**
- Produces: `pub struct IconSizeTokens { pub rail: f32, pub row: f32, pub chevron: f32, pub tab_arrow: f32, pub home: f32, pub tree_row_gap: f32, pub scale: f32 }`(`Clone`+`Copy`+`Debug`+`Deserialize`),`IconSizeTokens::byteboy2077() -> Self`,`pub fn current() -> IconSizeTokens`,`pub fn set_theme(tokens: IconSizeTokens)`。既有 `rail()`/`row()`/`chevron()`/`tab_arrow()`/`home()`/`tree_row_gap()`/`scale()`/`set_scale()`/`zoom_by()`/`init_scale(path)`/`persist_scale(path)`/`reset_scale(path)`/`SCALE_MIN`/`SCALE_MAX`/`load_from`/`save_to` 全部签名不变。
- Consumes: 无新增外部依赖

- [ ] **Step 1: 用下面内容整体替换 `crates/byteui/src/theme/icon_size.rs`**

```rust
//! 图标尺寸 token 化：`ByteBoy2077` 是编译期默认值，`set_theme` 可在
//! 运行时整体替换成另一份产品的取值（同 `theme::color`/`theme::font`/
//! `theme::geometry` 的模式）——组件内部一律读 `current()`，不直接引用
//! `byteboy2077()`。这里只管控件内部图标的"设计基准尺寸"，不越界。
//!
//! 本模块所有尺寸 accessor（`rail`/`row`/`chevron`/`tree_row_gap`）返回的值
//! 都已乘过 `scale()`，因此改 `scale` 即整体缩放全部图标与图标相关间距
//! （一个旋钮控制全局）；`icons::view` 是纯渲染入口，不再二次乘 scale。
//! 改 `rail/row/chevron/tree_row_gap` 则只调某类位置的相对大小。
//!
//! `scale()` 是**运行时可变**的：启动默认值取 `IconSizeTokens.scale`
//! （见下方 `current().scale`），可被环境变量 `DOZER_ICON_SCALE` 覆盖；
//! 运行时由 `set_scale` / `zoom_by`（Ctrl + / Ctrl - 快捷键入口）改写，
//! 下一帧布局即按新值重排——所有 accessor 每帧都实时读 `scale()`，不
//! 缓存缩放结果。**这套运行时缩放机制和 `IconSizeTokens`（静态设计基准
//! 尺寸）是两回事，不要混淆**：`set_theme` 换的是"设计基准值"，
//! `set_scale`/`zoom_by` 改的是"运行时倍数"，两者独立正交。
//!
//! 改过的 scale 需要**跨重启保留**：用户在会话里放大/缩小后退出，下次重开
//! 应回到退出时的 scale。`init_scale`/`persist_scale`/`reset_scale` 都改吃
//! 调用方传入的落盘路径（`byteui` 不内置任何 Dozer 专属路径约定）；环境
//! 变量 `DOZER_ICON_SCALE` 是显式覆盖，优先级高于落盘值（见 `init_scale`）。
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct IconSizeTokens {
    /// 主导航图标栏按钮(左/右 rail)与顶栏设置齿轮：16x16。
    pub rail: f32,
    /// 文件树行 / 右键菜单项 / tab 箭头 / 最大化按钮等：14x14。
    pub row: f32,
    /// 文件树展开/收起箭头：12x12（比同行文件图标略小）。
    pub chevron: f32,
    /// 面板 tab 栏翻页箭头（`<` / `>`）：10x10，比文件树 chevron 略小。
    pub tab_arrow: f32,
    /// 顶栏 "Dozer Home" tab 的品牌图标(house)：12x12。
    pub home: f32,
    /// 文件树行内"箭头↔图标"之间的间距（设计基准 2px）。
    pub tree_row_gap: f32,
    /// 设计基准缩放因子：1.0 = 设计基准；作为 `base_scale()` 的兜底值，
    /// 和运行时可变的 `CURRENT_SCALE` 是两回事。
    pub scale: f32,
}

impl IconSizeTokens {
    /// 逐一对应 `dozer-app` 当前 `assets/theme/workspace.json` 的
    /// `icon_sizes` 节点，仅作未显式 `set_theme()` 时的兜底默认值。
    pub const fn byteboy2077() -> Self {
        Self {
            rail: 16.0,
            row: 14.0,
            chevron: 12.0,
            tab_arrow: 9.0,
            home: 12.0,
            tree_row_gap: 2.0,
            scale: 1.0,
        }
    }
}

static CURRENT: RwLock<IconSizeTokens> = RwLock::new(IconSizeTokens::byteboy2077());

/// 当前生效的图标尺寸 token（默认 ByteBoy2077）。
pub fn current() -> IconSizeTokens {
    *CURRENT.read().expect("byteui icon_size RwLock poisoned")
}

/// 整体替换当前图标尺寸 token——供调用方（如 `dozer-app::theme::init()`）
/// 在启动时用自己的 `workspace.json` 覆盖默认值。**不影响**运行时缩放
/// 倍数（`CURRENT_SCALE`），那是独立机制，见模块文档。
pub fn set_theme(tokens: IconSizeTokens) {
    *CURRENT.write().expect("byteui icon_size RwLock poisoned") = tokens;
}

pub fn rail() -> f32 {
    current().rail * scale()
}
pub fn row() -> f32 {
    current().row * scale()
}
pub fn chevron() -> f32 {
    current().chevron * scale()
}
/// 面板 tab 栏翻页箭头（`<` / `>`）尺寸，已含全局 scale。
pub fn tab_arrow() -> f32 {
    current().tab_arrow * scale()
}
/// 顶栏 "Dozer Home" tab 品牌图标尺寸，已含全局 scale。
pub fn home() -> f32 {
    current().home * scale()
}
/// 文件树行内"箭头↔图标"间距，已含全局 scale。
pub fn tree_row_gap() -> f32 {
    current().tree_row_gap * scale()
}

/// 全局缩放因子的运行时当前值（逻辑像素倍数）。`u32::MAX` 是哨兵，表示
/// "尚未被运行时改写"，此时回落到 `current().scale`（token 基准值）。
static CURRENT_SCALE: AtomicU32 = AtomicU32::new(u32::MAX);

/// 启动基准 scale：`DOZER_ICON_SCALE` 环境变量优先，否则用 token 的 `scale`。
fn base_scale() -> f32 {
    std::env::var("DOZER_ICON_SCALE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|&v| v > 0.0)
        .unwrap_or(current().scale)
}

/// 全局缩放因子：所有 token accessor 都会乘它，因此改这一个值即整体缩放
/// 全部图标与图标相关间距（一个旋钮控制全局）。运行时可变——见
/// `set_scale` / `zoom_by`。
pub fn scale() -> f32 {
    let bits = CURRENT_SCALE.load(Ordering::Relaxed);
    if bits == u32::MAX {
        base_scale()
    } else {
        f32::from_bits(bits)
    }
}

/// 把全局 scale 直接设为目标值，超出 `[SCALE_MIN, SCALE_MAX]` 会被钳制。
/// 下一帧布局自动按新值重排。
pub fn set_scale(target: f32) {
    let clamped = target.clamp(SCALE_MIN, SCALE_MAX);
    CURRENT_SCALE.store(clamped.to_bits(), Ordering::Relaxed);
}

/// 相对缩放（Ctrl + / Ctrl - 的入口）：在当前值基础上乘 `factor`。
pub fn zoom_by(factor: f32) {
    set_scale(scale() * factor);
}

/// 还原到启动基准 scale（Ctrl+1 入口）：清空运行时改写，
/// 让 `scale()` 回落到 `base_scale()`（`DOZER_ICON_SCALE` 或 token `scale`）；
/// 同时把落盘值复位成出厂默认，使"还原"在下次重启后依然生效。
pub fn reset_scale(path: &Path) {
    CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    save_persisted_scale(path, current().scale);
}

/// 启动时把上次退出前落盘的 scale 读回并应用为当前值,调用方传入落盘路径
/// (`byteui` 不内置任何 Dozer 专属路径约定)。`DOZER_ICON_SCALE` 环境变量
/// 是显式覆盖,优先级高于落盘值。
pub fn init_scale(path: &Path) {
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
    if let Some(v) = load_persisted_scale(path) {
        set_scale(v);
    }
}

/// 把当前 scale 落盘到调用方指定的路径,供下次启动 `init_scale` 读回。
pub fn persist_scale(path: &Path) {
    save_persisted_scale(path, scale());
}

#[derive(Serialize, Deserialize)]
struct PersistedScale {
    scale: f32,
}

/// 读存盘 scale：文件缺失/损坏/越界/非有限数都回落 `None`（调用方保持默认）。
fn load_persisted_scale(path: &Path) -> Option<f32> {
    load_from(path)
}

/// 写存盘 scale：目录不存在先建；任何 IO 失败静默（缩放是体验增强，不阻断
/// 主流程，同 `dozer-hook` 的"任何错误都静默"定位）。
fn save_persisted_scale(path: &Path, v: f32) {
    let _ = save_to(path, v);
}

/// 显式路径读取，供单测指向临时文件，不碰用户真实配置目录。
/// 越界值夹回 `[SCALE_MIN, SCALE_MAX]`（手改过的大数不至于让 UI 失控），
/// 非有限数/非正数视为无效，回落 `None`。
pub(crate) fn load_from(path: &Path) -> Option<f32> {
    let raw = std::fs::read_to_string(path).ok()?;
    let p: PersistedScale = serde_json::from_str(&raw).ok()?;
    if p.scale.is_finite() && p.scale > 0.0 {
        Some(p.scale.clamp(SCALE_MIN, SCALE_MAX))
    } else {
        None
    }
}

/// 显式路径写入，供单测指向临时文件，不碰用户真实配置目录。
pub(crate) fn save_to(path: &Path, v: f32) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(&PersistedScale { scale: v }).unwrap_or_default();
    std::fs::write(path, s)
}

/// UI 缩放允许的下限/上限（逻辑像素倍数），防止缩到不可读或放到失控。
pub const SCALE_MIN: f32 = 0.5;
pub const SCALE_MAX: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：`byteboy2077()` 的每个字段值必须和 `dozer-app` 当前
    /// `assets/theme/workspace.json` 的 `icon_sizes` 字面量一致。
    #[test]
    fn byteboy2077_matches_dozer_app_baseline() {
        let t = IconSizeTokens::byteboy2077();
        assert_eq!(t.rail, 16.0);
        assert_eq!(t.row, 14.0);
        assert_eq!(t.chevron, 12.0);
        assert_eq!(t.tab_arrow, 9.0);
        assert_eq!(t.home, 12.0);
        assert_eq!(t.tree_row_gap, 2.0);
        assert_eq!(t.scale, 1.0);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        assert_eq!(current().rail, IconSizeTokens::byteboy2077().rail);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = IconSizeTokens::byteboy2077();
        custom.rail = 999.0;
        set_theme(custom);
        assert_eq!(current().rail, 999.0);
        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_theme(IconSizeTokens::byteboy2077());
    }

    #[test]
    fn accessors_reflect_current_at_default_scale() {
        set_theme(IconSizeTokens::byteboy2077());
        CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(rail(), 16.0);
        assert_eq!(row(), 14.0);
        assert_eq!(chevron(), 12.0);
        assert_eq!(tab_arrow(), 9.0);
    }

    /// 落盘 round-trip：写出去的值读回来和写的一致，且会夹进合法范围。
    #[test]
    fn persisted_scale_round_trips_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui_scale.json");

        save_to(&path, 1.5).unwrap();
        assert_eq!(load_from(&path), Some(1.5));

        // 越界值落盘后被夹回合法范围（写入时不夹，读取时夹）。
        save_to(&path, 99.0).unwrap();
        assert_eq!(load_from(&path), Some(SCALE_MAX));
        save_to(&path, 0.01).unwrap();
        assert_eq!(load_from(&path), Some(SCALE_MIN));
    }

    /// 损坏/缺失/非有限数文件都回落 `None`，不污染默认 scale。
    #[test]
    fn corrupted_or_missing_scale_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.json");
        assert_eq!(load_from(&missing), None);

        let bad = dir.path().join("ui_scale.json");
        std::fs::write(&bad, "not json").unwrap();
        assert_eq!(load_from(&bad), None);

        std::fs::write(&bad, r#"{"scale": "x"}"#).unwrap();
        assert_eq!(load_from(&bad), None);
    }

    /// 用临时目录里的 `ui_scale.json` 当落盘路径，隔离用户真实配置目录；
    /// 跑完复位运行时改写，避免污染同进程里之后的测试。
    fn with_temp_scale_file<F: FnOnce(&Path)>(f: F) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui_scale.json");
        f(&path);
        CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    }

    /// `init_scale` 把落盘值应用成当前 scale；`reset_scale` 清除运行时改写
    /// 并落盘复位成出厂默认。两者都吃显式临时路径，不再碰用户真实配置目录。
    #[test]
    fn init_applies_persisted_and_reset_clears_it() {
        if std::env::var("DOZER_ICON_SCALE").is_ok() {
            return;
        }
        with_temp_scale_file(|path| {
            save_to(path, 2.0).unwrap();
            init_scale(path);
            assert_eq!(scale(), 2.0);

            reset_scale(path);
            assert_eq!(scale(), current().scale);
            assert_eq!(load_from(path), Some(current().scale));
        });
    }
}
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p byteui`
Expected: 编译成功

- [ ] **Step 3: 测试验证**

Run: `cargo test -p byteui theme::icon_size`
Expected: 8 个测试全部通过

- [ ] **Step 4: 确认 `dozer-app` 仍能编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/byteui/src/theme/icon_size.rs
git commit -m "refactor(byteui): theme::icon_size 静态尺寸改为运行时可替换 IconSizeTokens,缩放机制不动"
```

---

### Task 4: 同步 `dozer-app` 的 `workspace.json`,删除 `byteui` 自带的那份

**Files:**
- Modify: `crates/dozer-app/assets/theme/workspace.json`
- Delete: `crates/byteui/assets/theme/workspace.json`

**Interfaces:**
- Consumes: Task 1-3 已完成(`byteui` 三个模块都已有 `set_theme()`,不再需要 `include_str!` 自己的 JSON)
- Produces: `dozer-app` 的 `workspace.json` 是仓库里唯一一份该文件,`font_sizes` 字段已同步到 `b3f2763` 定的最新值

- [ ] **Step 1: 确认两份文件当前的差异范围(执行前核对,只应有 `font_sizes` 不同)**

Run: `diff crates/byteui/assets/theme/workspace.json crates/dozer-app/assets/theme/workspace.json`
Expected: 只有 `font_sizes` 那 7 行不同(`byteui` 侧 10/11/12/13/14/15/16,`dozer-app` 侧仍是 9/10/11/12/13/14/15)。如果还有其它字段也不同,先手动核对那些差异是否也需要同步,不要假设只有 `font_sizes`。

- [ ] **Step 2: 把 `byteui` 那份最新的 `font_sizes` 同步进 `dozer-app` 的 `workspace.json`**

```bash
python3 - <<'EOF'
import json
byteui = json.load(open("crates/byteui/assets/theme/workspace.json"))
dozer_path = "crates/dozer-app/assets/theme/workspace.json"
dozer = json.load(open(dozer_path))
dozer["font_sizes"] = byteui["font_sizes"]
json.dump(dozer, open(dozer_path, "w"), indent=2, ensure_ascii=False)
with open(dozer_path, "a") as f:
    f.write("\n")
EOF
```

- [ ] **Step 3: 确认这一步只改了 `font_sizes`**

Run: `git diff crates/dozer-app/assets/theme/workspace.json`
Expected: 只有 `font_sizes` 那 7 行数值变化(9→10、10→11、11→12、12→13、13→14、14→15、15→16),其余字段(`geometry`/`icon_sizes`/`regions`)、缩进、字段顺序都不变。如果 diff 出现大范围重排(比如 JSON 键顺序被 Python 打乱),回滚这一步改成手工编辑那 7 行,不要让无关格式变动混进这次改动。

- [ ] **Step 4: 删除 `byteui` 自带的 `workspace.json` 及其空目录**

```bash
rm crates/byteui/assets/theme/workspace.json
rmdir crates/byteui/assets/theme 2>/dev/null || true
```

- [ ] **Step 5: 编译验证(`byteui` 应该已经不再引用这个文件)**

Run: `cargo build -p byteui`
Expected: 编译成功(如果报 `include_str!` 找不到文件,说明 Task 1-3 有遗漏,回去检查)

Run: `grep -rn "include_str.*workspace.json" crates/byteui/src`
Expected: 无输出

- [ ] **Step 6: 确认 `dozer-app` 仍能编译(它目前还没读这份 JSON,只是文件内容变了)**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add crates/dozer-app/assets/theme/workspace.json crates/byteui/assets/theme/workspace.json
git commit -m "chore: 同步 dozer-app workspace.json 的 font_sizes,删除 byteui 内嵌的重复副本"
```

---

### Task 5: `dozer-app::theme::init()` + 接入 `main.rs`

**Files:**
- Modify: `crates/dozer-app/src/theme.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `byteui::theme::font::{FontTokens, set_theme}`、`byteui::theme::geometry::{GeometryTokens, set_theme}`、`byteui::theme::icon_size::{IconSizeTokens, set_theme}`(Task 1-3)
- Produces: `pub(crate) fn init()`,`main.rs` 建窗前调用

- [ ] **Step 1: 在 `crates/dozer-app/src/theme.rs` 追加 `init()`**

在现有 `ui_scale_path()` 函数之后追加:

```rust
const WORKSPACE_JSON: &str = include_str!("../assets/theme/workspace.json");

#[derive(serde::Deserialize)]
struct RawWorkspaceFile {
    font_sizes: byteui::theme::font::FontTokens,
    geometry: byteui::theme::geometry::GeometryTokens,
    icon_sizes: byteui::theme::icon_size::IconSizeTokens,
}

/// 启动时把 dozer-app 自己的 `workspace.json` 灌进 byteui 三个 token
/// 模块,取代它们编译期内置的 ByteBoy2077 默认值。必须在建窗、任何
/// 渲染逻辑跑之前调用一次。解析失败(格式错误、缺字段)直接 panic——
/// 开发期配置错误,不是需要优雅降级的运行时数据(同 `region.rs`/
/// `terminal_font.rs` 一贯的定位)。
pub(crate) fn init() {
    let raw: RawWorkspaceFile =
        serde_json::from_str(WORKSPACE_JSON).expect("workspace.json 格式错误(解析失败)");
    byteui::theme::font::set_theme(raw.font_sizes);
    byteui::theme::geometry::set_theme(raw.geometry);
    byteui::theme::icon_size::set_theme(raw.icon_sizes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_parses_and_applies_real_workspace_json() {
        init();
        assert_eq!(byteui::theme::font::current().body, 14);
        assert_eq!(byteui::theme::geometry::current().icon_rail_width, 44.0);
        assert_eq!(byteui::theme::icon_size::current().rail, 16.0);
    }

    #[test]
    #[should_panic(expected = "workspace.json 格式错误")]
    fn init_panics_on_malformed_json() {
        let raw = "{not valid json";
        let _: RawWorkspaceFile =
            serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败)");
    }
}
```

- [ ] **Step 2: 在 `main.rs` 建窗前调用 `theme::init()`**

`crates/dozer-app/src/main.rs` 里找到:

```rust
crate::theme::icon_size::init_scale(&crate::theme::ui_scale_path());
```

改成(在它之前插入一行):

```rust
crate::theme::init();
byteui::theme::icon_size::init_scale(&crate::theme::ui_scale_path());
```

- [ ] **Step 3: 确认无残留的 `crate::theme::icon_size::` 引用(Task 5 不应该引入新的这类引用,只是确认之前迁移没有漏改)**

Run: `grep -n "crate::theme::icon_size::init_scale" crates/dozer-app/src/main.rs`
Expected: 无输出(应该已经是 `byteui::theme::icon_size::init_scale`)

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: 测试验证**

Run: `cargo test -p dozer-app --bin dozer theme::tests`
Expected: `init_parses_and_applies_real_workspace_json`/`init_panics_on_malformed_json` 两个测试通过

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): 新增 theme::init(),启动时把 workspace.json 灌进 byteui token 模块"
```

---

### Task 6: 全量验证

**Files:**
- 无修改,纯验证

**Interfaces:**
- Consumes: Task 1-5 已完成

- [ ] **Step 1: 独立编译两个 crate**

Run: `cargo build -p byteui && cargo build -p dozer-app --bin dozer`
Expected: 均编译成功。`cargo build -p byteui` 单独成功尤其重要——验证
`byteui` 现在真的不依赖任何 `dozer-app` 专属数据。

- [ ] **Step 2: 全量测试**

Run: `cargo test -p byteui && cargo test -p dozer-app --bin dozer`
Expected: 全部通过。`dozer-app` 测试数量相比合并前基线会有增减(新增
`theme::tests` 两个测试),不强求总数完全一致,只要求没有非预期失败。

- [ ] **Step 3: clippy + fmt**

Run: `cargo clippy -p byteui -p dozer-app --all-targets -- -D warnings && cargo fmt -p byteui -p dozer-app -- --check`
Expected: 无新增警告、无格式差异(已知的、和这次改动无关的历史遗留
warning/clippy 问题不算,执行时可用 `git stash` 切回 `main` 跑一遍同一
命令比对,确认没有新增)。

- [ ] **Step 4: 确认全仓库没有任何地方还在直接构造/引用旧的
  `LazyLock<...>` 静态量或 `RawWorkspaceFile`(应该已经在 Task 1-3 删完)**

Run: `grep -rn "LazyLock<WorkspaceFonts>\|LazyLock<Geometry>\|LazyLock<IconSizes>" crates/byteui/src`
Expected: 无输出

- [ ] **Step 5: 独立命名的临时二进制视觉核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式 `/Applications/
Dozer AI Coder.app` 或其他调试会话进程名的路径),启动后核对:

- 全 App 字号(标题/正文/次要文字/dot 态)、窗口/面板/菜单/滚动条/tab
  等几何尺寸,和改动前(`byteui` 那份 `workspace.json` 删除前的最新值,
  即 `b3f2763` 定的 font_sizes 10-16、原 geometry/icon_sizes 数值)逐一
  比对无差异。
- Ctrl +/- 缩放、Ctrl+1 还原,确认运行时缩放机制(Task 3 明确不动的
  那部分)行为不变。
- 退出重开,确认缩放值跨重启保留(验证 `persist_scale`/`init_scale`
  没有被这次改动间接影响)。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 6: Commit(如果 Step 1-5 发现并修复了任何问题)**

```bash
git branch --show-current
git add -A
git commit -m "fix: 全量验证发现的问题修复"
```

(如果 Step 1-5 全部一次通过,没有需要修复的问题,这一步跳过,不产生空
commit。)

---

## 完工验收

1. `git log --oneline` 确认全部 commit 都在当前分支上,没有漂到 `main`。
2. `crates/byteui/assets/theme/` 目录已不存在。
3. `cargo build -p byteui` 独立编译成功(不依赖 `dozer-app` 的任何东西,
   也不内嵌任何 Dozer 专属 JSON)。
4. `grep -rn "workspace.json" crates/byteui` 全仓库零匹配。
5. 提请审阅。审阅通过后这是 `byteui` 迁移系列 + 这次补充修正的最终
   状态——`byteui` 不再持有任何 `dozer-app` 专属配置数据,`dozer-app`
   的 `assets/theme/workspace.json` 是唯一真相源。
