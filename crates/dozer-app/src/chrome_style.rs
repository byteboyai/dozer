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
    let parsed: RawRegions = serde_json::from_str(raw).expect("regions.json 格式错误(解析失败)");
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
