//! 外壳区域样式配置:15 个区域(主导航/左右图标栏/放大态浮层/右键菜单/
//! 状态条 + 7 个面板内容区 + 左右面板区整体外框)各自外层容器的背景色/
//! 边框/内边距/子元素间距,编译期内嵌 `assets/theme/workspace.json` 的
//! `regions` 节点,启动时解析一次。`workspace.json` 同时也是 `workspace_font.rs` 的
//! 数据源(`font_sizes` 节点)——两个模块各自只解析自己关心的顶层字段,
//! 互不干扰,合并成一个文件是为了"workspace 相关配置都在一处"。
//!
//! 颜色字段是字符串,支持两种写法:`theme.rs` 现成的令牌名(如
//! `"BORDER"`),或 `#RGB` / `#RGBA` / `#RRGGBB` / `#RRGGBBAA` 字面量十六进制
//! 值(3/4 位按 CSS 简写每位复制成两位,`#ccc` = `#cccccc`)——后者让
//! `background` 这类用户最常自定义的字段脱离令牌表,直接改 JSON 就能
//! 调色,不必碰 Rust 代码。解析失败(格式错误、非法十六进制、未知颜色
//! 令牌名)直接 panic:这是编译期就该发现的开发期配置错误,不是需要
//! 优雅降级的运行时数据(同 `theme.rs` 14 色的定位)。
//!
//! 只覆盖区域**外层容器**的样式;区域内部控件的 active/hover/pressed
//! 等交互态样式(如 `rail_icon_button`)不在这里,留在 Rust 代码里。
//!
//! `resolve_region` 会把每个区域的 `padding`/`gap` 乘过全局 scale，因此
//! 改 `icon_size::scale()` 时区域内部间距也等比放大，与图标/字号同步。
use crate::theme::icon_size;
use crate::theme;
use iced_widget::core::{Border, Color, Padding};
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/theme/workspace.json");

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
    gap: f32,
    /// 区域整体相对外围容器(顶栏/窗口底/相邻面板)的外边距,默认零。
    /// 仅 `left_zone`/`right_zone` 用它做上下留白,其余区域不声明即无边距。
    #[serde(default)]
    margin: Option<RawPadding>,
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
    left_zone: RawRegion,
    right_zone: RawRegion,
}

/// `workspace.json` 顶层结构里本模块只关心的部分——`font_sizes` 节点是
/// `workspace_font.rs` 的地盘,这里不声明,serde 默认忽略未知字段。
#[derive(Deserialize)]
struct RawWorkspaceFile {
    /// 整体背景色:窗口根容器的底色(顶栏/面板之间、放大态遮罩之外的"留白"处),
    /// 以及渲染管线的清屏色。区域各自的 `background` 若未设置则在此之上叠加。
    background: String,
    regions: RawRegions,
}

/// `#RGB` / `#RGBA` / `#RRGGBB` / `#RRGGBBAA` 十六进制字面量 → `Color`。
/// 3/4 位简写按 CSS 规则每位复制成两位(`#ccc` → `#cccccc`,`#ccc8` →
/// `#cccccc88`),让用户直接改 JSON 就能写出惯用的短色值。
fn parse_hex_color(hex: &str) -> Color {
    let raw = hex.strip_prefix('#').unwrap_or(hex);
    // 3/4 位简写展开为 6/8 位(CSS 语义:每位复制成两位)。
    let digits: std::borrow::Cow<str> = match raw.len() {
        3 | 4 => {
            let mut s = String::with_capacity(raw.len() * 2);
            for c in raw.chars() {
                s.push(c);
                s.push(c);
            }
            s.into()
        }
        _ => raw.into(),
    };
    let component = |i: usize| -> f32 {
        u8::from_str_radix(&digits[i..i + 2], 16)
            .unwrap_or_else(|e| panic!("workspace.json: 非法十六进制颜色 \"{hex}\": {e}"))
            as f32
            / 255.0
    };
    match digits.len() {
        6 => Color {
            r: component(0),
            g: component(2),
            b: component(4),
            a: 1.0,
        },
        8 => Color {
            r: component(0),
            g: component(2),
            b: component(4),
            a: component(6),
        },
        _ => panic!("workspace.json: 非法十六进制颜色 \"{hex}\"(需 3/4/6/8 位)"),
    }
}

/// 颜色字符串 → `Color`:`#` 开头按十六进制字面量解析,否则按 `theme.rs`
/// 令牌名查表。未知名字/非法格式直接 panic——配置写错在启动时就能发现,
/// 不会带着错误的透明色静默跑起来。
fn resolve_color(name: &str) -> Color {
    if name.starts_with('#') {
        return parse_hex_color(name);
    }
    match name {
        "BG" => theme::color::BG,
        "PANEL" => theme::color::PANEL,
        "TERM_BG" => theme::color::TERM_BG,
        "CARD" => theme::color::CARD,
        "BORDER" => theme::color::BORDER,
        "CREAM" => theme::color::CREAM,
        "BODY" => theme::color::BODY,
        "DIM" => theme::color::DIM,
        "GOLD" => theme::color::GOLD,
        "CYAN" => theme::color::CYAN,
        "GREEN" => theme::color::GREEN,
        "PURPLE" => theme::color::PURPLE,
        "RED" => theme::color::RED,
        "SCRIM" => theme::color::SCRIM,
        other => panic!("workspace.json: 未知颜色令牌 \"{other}\""),
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
    /// 区域整体相对外围容器的外边距(默认零,仅 `left_zone`/
    /// `right_zone` 用它做上下留白)。渲染时套一层外容器做 inset,
    /// 几何侧(`preview_content_bounds`/`terminal_pane_pixel_size`)要同步
    /// 扣减,否则原生子视图(webview/PTY)会戳出边框。
    pub margin: Padding,
}

fn resolve_region(r: RawRegion) -> RegionStyle {
    RegionStyle {
        background: r.background.as_deref().map(resolve_color),
        border: r.border.as_ref().map(resolve_border),
        padding: r.padding.into(),
        gap: r.gap,
        margin: r.margin.map(Into::into).unwrap_or_default(),
    }
}

/// 把区域样式按当前全局 scale 折算：padding 四边与 gap 都乘 `icon_size::scale()`。
/// 在**每个 accessor** 调用时执行（不经 `REGIONS` 缓存），这样运行时改
/// `scale`（Ctrl +/-）时区域内部间距能跟着重排，而不是冻结在启动时刻。
fn scaled_region(r: RegionStyle) -> RegionStyle {
    let s = icon_size::scale();
    let mut padding = r.padding;
    padding.top *= s;
    padding.right *= s;
    padding.bottom *= s;
    padding.left *= s;
    let mut margin = r.margin;
    margin.top *= s;
    margin.right *= s;
    margin.bottom *= s;
    margin.left *= s;
    RegionStyle {
        padding,
        margin,
        gap: r.gap * s,
        ..r
    }
}

/// 放大态浮层样式按当前全局 scale 折算：scrim 内边距与金色描边盒的
/// 线宽/圆角都乘 `icon_size::scale()`（同 `scaled_region`，逐 accessor 应用）。
fn scaled_overlay(r: MaximizeOverlayStyle) -> MaximizeOverlayStyle {
    let s = icon_size::scale();
    MaximizeOverlayStyle {
        scrim_background: r.scrim_background,
        scrim_padding: r.scrim_padding * s,
        border: Border {
            color: r.border.color,
            width: r.border.width * s,
            radius: r.border.radius * s,
        },
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
    left_zone: RegionStyle,
    right_zone: RegionStyle,
}

fn load(raw: &str) -> ResolvedRegions {
    let file: RawWorkspaceFile =
        serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败,regions 节点)");
    let parsed = file.regions;
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
        left_zone: resolve_region(parsed.left_zone),
        right_zone: resolve_region(parsed.right_zone),
    }
}

static REGIONS: LazyLock<ResolvedRegions> = LazyLock::new(|| load(RAW));

/// 整体背景色,解析一次(`#ccc` 这类十六进制字面量或 theme 令牌名)。
static BACKGROUND: LazyLock<Color> = LazyLock::new(|| {
    let file: RawWorkspaceFile =
        serde_json::from_str(RAW).expect("workspace.json 格式错误(解析失败,background 节点)");
    resolve_color(&file.background)
});

/// 整体背景色:窗口根容器底色 / 渲染清屏色。
pub fn background() -> Color {
    *BACKGROUND
}

pub fn top_bar() -> RegionStyle {
    scaled_region(REGIONS.top_bar)
}
pub fn left_icon_rail() -> RegionStyle {
    scaled_region(REGIONS.left_icon_rail)
}
pub fn right_icon_rail() -> RegionStyle {
    scaled_region(REGIONS.right_icon_rail)
}
pub fn project_pane() -> RegionStyle {
    scaled_region(REGIONS.project_pane)
}
pub fn preview_pane() -> RegionStyle {
    scaled_region(REGIONS.preview_pane)
}
pub fn browser_pane() -> RegionStyle {
    scaled_region(REGIONS.browser_pane)
}
pub fn agent_list_pane() -> RegionStyle {
    scaled_region(REGIONS.agent_list_pane)
}
pub fn terminal_pane() -> RegionStyle {
    scaled_region(REGIONS.terminal_pane)
}
pub fn conversation_list_pane() -> RegionStyle {
    scaled_region(REGIONS.conversation_list_pane)
}
pub fn review_content_pane() -> RegionStyle {
    scaled_region(REGIONS.review_content_pane)
}
pub fn status_bar() -> RegionStyle {
    scaled_region(REGIONS.status_bar)
}
pub fn maximize_overlay() -> MaximizeOverlayStyle {
    scaled_overlay(REGIONS.maximize_overlay)
}
pub fn context_menu() -> RegionStyle {
    scaled_region(REGIONS.context_menu)
}
/// 左面板区(项目树+预览,或单个 Web 预览)整体外边框——把"左1左2两栏"
/// 框成一个视觉整体,不是某一栏自己的边框。
pub fn left_zone() -> RegionStyle {
    scaled_region(REGIONS.left_zone)
}
/// 右面板区(Agent 列表+终端,或对话列表+审阅)整体外边框,同 `left_zone`。
pub fn right_zone() -> RegionStyle {
    scaled_region(REGIONS.right_zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:关键区域的解析结果必须和改动前的字面量完全一致——纯
    /// 代码搬家,数值不该变。以后有人手滑改错配置文件,这个测试会炸。
    #[test]
    fn preview_pane_matches_pre_migration_literals() {
        let s = preview_pane();
        assert_eq!(s.background, Some(theme::color::PANEL));
        assert!(s.border.is_none());
        assert_eq!(s.padding, Padding::from(8.0));
        assert_eq!(s.gap, 4.0);
    }

    /// 严格还原 Figma"Dozer Phase 1 UI"node-id=87:31 的 Titlebar:与其余
    /// 区域"防漂移锚=迁移前字面量"不同,这里的锚点是设计稿数值——2026-08-05
    /// 按设计稿改过背景色/边框/内边距/间距,不再是纯代码搬家。
    #[test]
    fn top_bar_matches_figma_design() {
        let s = top_bar();
        assert_eq!(s.background, Some(Color::from_rgb8(0x0e, 0x16, 0x20)));
        let border = s.border.expect("top_bar 应有底部分隔线(设计稿 border-b)");
        assert_eq!(border.color, theme::color::BORDER);
        assert_eq!(border.width, 1.0);
        // left=78:统一工具栏改造后,原生红黄绿交通灯叠在 top_bar 左侧,
        // 这段留白给交通灯让位,不是设计稿本身的数值(设计稿假设交通灯是
        // 画在 flex 行里的图片,现在改成系统原生绘制、不占 flex 布局位置)。
        assert_eq!(
            s.padding,
            Padding {
                top: 0.0,
                right: 16.0,
                bottom: 0.0,
                left: 78.0,
            }
        );
        assert_eq!(s.gap, 8.0);
    }

    #[test]
    fn left_icon_rail_matches_pre_migration_literals() {
        let s = left_icon_rail();
        assert_eq!(s.background, Some(theme::color::PANEL));
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
        assert_eq!(s.background, Some(theme::color::PANEL));
        let border = s.border.expect("status_bar 应有上边线");
        assert_eq!(border.color, theme::color::BORDER);
        assert_eq!(border.width, 1.0);
        assert_eq!(s.padding, Padding::from([0.0, 8.0]));
    }

    #[test]
    fn maximize_overlay_matches_pre_migration_literals() {
        let s = maximize_overlay();
        assert_eq!(s.scrim_background, theme::color::SCRIM);
        assert_eq!(s.scrim_padding, 40.0);
        assert_eq!(s.border.color, theme::color::GOLD);
        assert_eq!(s.border.width, 1.5);
        assert_eq!(s.border.radius, 10.0.into());
    }

    #[test]
    fn context_menu_matches_pre_migration_literals() {
        let s = context_menu();
        assert_eq!(s.background, Some(theme::color::CARD));
        let border = s.border.expect("context_menu 应有边框");
        assert_eq!(border.color, theme::color::BORDER);
        assert_eq!(border.width, 1.0);
        assert_eq!(border.radius, 6.0.into());
        assert_eq!(s.padding, Padding::from(6.0));
        assert_eq!(s.gap, 2.0);
    }

    #[test]
    fn left_zone_matches_config() {
        let s = left_zone();
        // 左右面板区整体用圆角背景浮起:背景填充 CARD 色(比内层面板 PANEL
        // 略亮,在灰底窗口上显出圆角卡片),描边宽为 0 但带圆角(用 0 宽描边
        // 保留"无描边"观感,仅让背景走圆角),四向 margin 做悬浮留白。
        assert_eq!(s.background, Some(theme::color::CARD));
        let border = s.border.expect("left_zone 应有圆角边框(宽 0)");
        assert_eq!(border.color, theme::color::BORDER);
        assert_eq!(border.width, 0.0);
        assert_eq!(border.radius, 8.0.into());
        assert_eq!(s.padding, Padding::from(1.0));
    }

    #[test]
    fn right_zone_matches_config() {
        let s = right_zone();
        assert_eq!(s.background, Some(theme::color::CARD));
        let border = s.border.expect("right_zone 应有圆角边框(宽 0)");
        assert_eq!(border.color, theme::color::BORDER);
        assert_eq!(border.width, 0.0);
        assert_eq!(border.radius, 8.0.into());
        assert_eq!(s.padding, Padding::from(1.0));
    }

    #[test]
    #[should_panic(expected = "未知颜色令牌")]
    fn unknown_color_token_panics() {
        resolve_color("NOT_A_REAL_TOKEN");
    }

    #[test]
    fn hex_color_matches_equivalent_token() {
        assert_eq!(resolve_color("#0a0e16"), theme::color::BG);
        assert_eq!(resolve_color("#12202a"), theme::color::CARD);
    }

    #[test]
    fn hex_color_with_alpha_parses() {
        let c = resolve_color("#00000080");
        assert_eq!(c.r, 0.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 0.0);
        assert_eq!(c.a, 0x80 as f32 / 255.0);
    }

    #[test]
    fn shorthand_hex_color_expands() {
        // `#ccc` 是合法的 CSS 简写,展开为 `#cccccc`(doc 注释也以它为合法示例)。
        assert_eq!(resolve_color("#ccc"), resolve_color("#cccccc"));
        assert_eq!(resolve_color("#ccc"), Color::from_rgb8(0xcc, 0xcc, 0xcc));
        // 4 位带 alpha 简写:每位复制成两位。
        let c = resolve_color("#ccc8");
        assert_eq!(c.r, 0xcc as f32 / 255.0);
        assert_eq!(c.g, 0xcc as f32 / 255.0);
        assert_eq!(c.b, 0xcc as f32 / 255.0);
        assert_eq!(c.a, 0x88 as f32 / 255.0);
    }

    #[test]
    #[should_panic(expected = "非法十六进制颜色")]
    fn malformed_hex_color_panics() {
        resolve_color("#12345");
    }
}
