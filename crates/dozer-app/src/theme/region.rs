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
//! 改 `byteui::theme::icon_size::scale()` 时区域内部间距也等比放大，与图标/字号同步。
use iced_widget::core::{Border, Color, Padding};
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../../assets/theme/workspace.json");

/// JSON 里 `padding` 字段的三种形状:单值(四边相同)/ `[v,h]` / `[top,right,bottom,left]`。
/// 数组顺序与 iced `core::Padding` 字段顺序一致(已用 iced 源码核对)。
#[derive(Deserialize, Clone, Copy)]
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

#[derive(Deserialize, Clone)]
struct RawBorder {
    color: String,
    width: f32,
    radius: f32,
}

#[derive(Deserialize, Clone)]
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

#[derive(Deserialize, Clone)]
struct RawScrim {
    background: String,
    padding: f32,
}

#[derive(Deserialize, Clone)]
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
    dialog: RawRegion,
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
///
/// `pub(crate)`:`homespace_color` 复用同一套"令牌名或十六进制字面量"
/// 解析,与 workspace.json 的 region 颜色保持一致(见 `theme/homespace_color.rs`)。
pub(crate) fn resolve_color(name: &str) -> Color {
    if name.starts_with('#') {
        return parse_hex_color(name);
    }
    match name {
        "BG" => byteui::theme::color::current().bg,
        "PANEL" => byteui::theme::color::current().panel,
        "TERM_BG" => byteui::theme::color::current().term_bg,
        "CARD" => byteui::theme::color::current().card,
        "BORDER" => byteui::theme::color::current().border,
        "CREAM" => byteui::theme::color::current().cream,
        "BODY" => byteui::theme::color::current().body,
        "DIM" => byteui::theme::color::current().dim,
        "GOLD" => byteui::theme::color::current().gold,
        "CYAN" => byteui::theme::color::current().cyan,
        "GREEN" => byteui::theme::color::current().green,
        "PURPLE" => byteui::theme::color::current().purple,
        "RED" => byteui::theme::color::current().red,
        "SCRIM" => byteui::theme::color::current().scrim,
        "TAB_ACTIVE_BORDER" => byteui::theme::color::current().tab_active_border,
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

/// 颜色令牌名→`Color` 的解析每次都现读 `byteui::theme::color::current()`
/// (不缓存),所以这里吃引用、调用方可以对同一份缓存的 `RawRegion`
/// 反复解析——运行时切换配色方案后,下一次调用就能拿到新颜色,同
/// `scaled_region` 对 `scale()` 的"结构缓存、取值现读"分工。
fn resolve_region(r: &RawRegion) -> RegionStyle {
    RegionStyle {
        background: r.background.as_deref().map(resolve_color),
        border: r.border.as_ref().map(resolve_border),
        padding: r.padding.into(),
        gap: r.gap,
        margin: r.margin.map(Into::into).unwrap_or_default(),
    }
}

/// 把区域样式按当前全局 scale 折算：padding 四边与 gap 都乘 `byteui::theme::icon_size::scale()`。
/// 在**每个 accessor** 调用时执行（不经 `REGIONS` 缓存），这样运行时改
/// `scale`（Ctrl +/-）时区域内部间距能跟着重排，而不是冻结在启动时刻。
fn scaled_region(r: RegionStyle) -> RegionStyle {
    let s = byteui::theme::icon_size::scale();
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
/// 线宽/圆角都乘 `byteui::theme::icon_size::scale()`（同 `scaled_region`，逐 accessor 应用）。
fn scaled_overlay(r: MaximizeOverlayStyle) -> MaximizeOverlayStyle {
    let s = byteui::theme::icon_size::scale();
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

fn resolve_maximize_overlay(r: &RawMaximizeOverlay) -> MaximizeOverlayStyle {
    MaximizeOverlayStyle {
        scrim_background: resolve_color(&r.scrim.background),
        scrim_padding: r.scrim.padding,
        border: resolve_border(&r.border),
    }
}

fn load(raw: &str) -> RawRegions {
    let file: RawWorkspaceFile =
        serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败,regions 节点)");
    file.regions
}

/// 只缓存**结构**(padding/gap/颜色令牌名字符串),不缓存颜色解析结果——
/// 颜色令牌名→`Color` 依赖 `byteui::theme::color::current_scheme()`,运行时
/// 可切换(设置弹窗);若在这里就把颜色一起烤进缓存,首次访问后就冻结,
/// 之后切主题对 `top_bar`/`terminal_pane` 等区域样式毫无作用(2026-09-15
/// 用户实测发现的真实 bug:切浅色后顶栏/图标栏/终端底色纹丝不动)。
/// 颜色解析改在每个 accessor(`resolve_region`/`resolve_maximize_overlay`)
/// 调用时现读,与 `scaled_region` 对 `scale()` 的"结构缓存、取值现读"
/// 分工一致。
static REGIONS: LazyLock<RawRegions> = LazyLock::new(|| load(RAW));

/// 整体背景色的令牌名/十六进制字面量,只解析结构一次;真正的颜色解析在
/// `background()` 里现读,理由同 `REGIONS`。
static BACKGROUND: LazyLock<String> = LazyLock::new(|| {
    let file: RawWorkspaceFile =
        serde_json::from_str(RAW).expect("workspace.json 格式错误(解析失败,background 节点)");
    file.background
});

/// 整体背景色:窗口根容器底色 / 渲染清屏色。
pub fn background() -> Color {
    resolve_color(&BACKGROUND)
}

pub fn top_bar() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.top_bar))
}
pub fn left_icon_rail() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.left_icon_rail))
}
pub fn right_icon_rail() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.right_icon_rail))
}
pub fn project_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.project_pane))
}
pub fn preview_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.preview_pane))
}
pub fn browser_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.browser_pane))
}
pub fn agent_list_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.agent_list_pane))
}
pub fn terminal_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.terminal_pane))
}
pub fn conversation_list_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.conversation_list_pane))
}
pub fn review_content_pane() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.review_content_pane))
}
pub fn status_bar() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.status_bar))
}
pub fn maximize_overlay() -> MaximizeOverlayStyle {
    scaled_overlay(resolve_maximize_overlay(&REGIONS.maximize_overlay))
}
/// 背景取 `BG` 色 + 半透明 alpha(≈0.98,模拟原生右键菜单磨砂观感)。
/// `workspace.json` 里存的是 `"#0a0e16fa"` 这种烤死的深色字面量,不是
/// 令牌名,同 `background`(根背景)一样不会随配色方案切换——所以这里不走
/// 通用 `resolve_region`(它只按 JSON 里写的字面量/令牌名解析),显式用
/// `current().bg` 现读再自己叠 alpha,让浅色主题也能拿到对应的浅色磨砂效果。
pub fn context_menu() -> RegionStyle {
    let mut s = scaled_region(resolve_region(&REGIONS.context_menu));
    let mut bg = byteui::theme::color::current().bg;
    bg.a = 0xfa as f32 / 255.0;
    s.background = Some(bg);
    s
}
/// 弹窗(确认框/模态对话框)外壳:统一 CARD 底 + 金色描边(呼应放大态
/// 浮层同款"金色描边盒"),供 `dialog::card_style` 取用。
pub fn dialog() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.dialog))
}
/// 左面板区(项目树+预览,或单个 Web 预览)整体外边框——把"左1左2两栏"
/// 框成一个视觉整体,不是某一栏自己的边框。
pub fn left_zone() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.left_zone))
}
/// 右面板区(Agent 列表+终端,或对话列表+审阅)整体外边框,同 `left_zone`。
pub fn right_zone() -> RegionStyle {
    scaled_region(resolve_region(&REGIONS.right_zone))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `byteui::theme::color` 的当前配色方案是进程级共享 static，cargo
    /// test 默认多线程并跑；本模块几乎每个测试都直接或经 `resolve_color`
    /// 间接读 `color::current()`，默认假设 `Dark`——不加锁的话,唯一会
    /// 切换方案的 `top_bar_and_background_track_live_scheme_switch` 一旦
    /// 和它们同时跑,读到 `Light` 就会断言失败(同 `color.rs`/
    /// `term_model.rs` 测试模块的教训)。凡是依赖当前方案的测试都先拿
    /// 这把锁序列化执行。
    static SCHEME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_scheme() -> std::sync::MutexGuard<'static, ()> {
        SCHEME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 防漂移锚:关键区域的解析结果必须和改动前的字面量完全一致——纯
    /// 代码搬家,数值不该变。以后有人手滑改错配置文件,这个测试会炸。
    #[test]
    fn preview_pane_matches_pre_migration_literals() {
        let _guard = lock_scheme();
        let s = preview_pane();
        assert_eq!(s.background, Some(byteui::theme::color::current().panel));
        assert!(s.border.is_none());
        assert_eq!(s.padding, Padding::from(8.0));
        assert_eq!(s.gap, 4.0);
    }

    /// 严格还原 Figma"Dozer Phase 1 UI"node-id=87:31 的 Titlebar:与其余
    /// 区域"防漂移锚=迁移前字面量"不同,这里的锚点是设计稿数值——2026-08-05
    /// 按设计稿改过背景色/边框/内边距/间距,不再是纯代码搬家。
    #[test]
    fn top_bar_matches_figma_design() {
        let _guard = lock_scheme();
        let s = top_bar();
        assert_eq!(s.background, Some(byteui::theme::color::current().bg));
        let border = s.border.expect("top_bar 应有底部分隔线(设计稿 border-b)");
        assert_eq!(border.color, byteui::theme::color::current().border);
        assert_eq!(border.width, 1.0);
        // left=78:统一工具栏改造后,原生红黄绿交通灯叠在 top_bar 左侧,
        // 这段留白给交通灯让位,不是设计稿本身的数值(设计稿假设交通灯是
        // 画在 flex 行里的图片,现在改成系统原生绘制、不占 flex 布局位置)。
        // right=10:让设置按钮的**图标本身**(不只是按钮命中区)到窗口右边缘
        // 的距离跟 rail 按钮的图标对上。两边图标同用 `icon_size::rail()`
        // (16px),但命中区大小不同——rail 按钮 `rail_button_size()`(32px)、
        // 设置按钮 `tab_button_size()`(24px),图标在各自命中区里居中,命中区
        // 越大图标离边缘就越远。只把 `top_bar` 的 `padding.right` 设成跟
        // `right_icon_rail.padding.right`(6.0)一样只能对齐两个命中区的外
        // 边缘,图标本身仍会因为命中区差 8px 而错位 4px。真正要对齐的是
        // "命中区外边缘 + 命中区内到图标的留白"这一整段:
        // `right_icon_rail.padding.right + (rail_button_size -
        // tab_button_size) / 2 = 6.0 + (32.0 - 24.0) / 2 = 10.0`——两边图标
        // 尺寸相同,这个式子里图标尺寸本身会抵消,不用代入。
        // （2026-09-16 用户反馈两轮:先按命中区边缘对齐过一版(6.0)不够准,
        // 这版按图标本身对齐。）
        // left=78:统一工具栏改造后,原生红黄绿交通灯叠在 top_bar 左侧,
        // 这段留白给交通灯让位,不是设计稿本身的数值(设计稿假设交通灯是
        // 画在 flex 行里的图片,现在改成系统原生绘制、不占 flex 布局位置)。
        assert_eq!(
            s.padding,
            Padding {
                top: 0.0,
                right: 10.0,
                bottom: 0.0,
                left: 78.0,
            }
        );
        assert_eq!(s.gap, 8.0);
    }

    #[test]
    fn left_icon_rail_matches_pre_migration_literals() {
        let _guard = lock_scheme();
        let s = left_icon_rail();
        assert_eq!(s.background, Some(byteui::theme::color::current().panel));
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
        let _guard = lock_scheme();
        let s = status_bar();
        assert_eq!(s.background, Some(byteui::theme::color::current().panel));
        let border = s.border.expect("status_bar 应有上边线");
        assert_eq!(border.color, byteui::theme::color::current().border);
        assert_eq!(border.width, 1.0);
        assert_eq!(s.padding, Padding::from([0.0, 8.0]));
    }

    #[test]
    fn maximize_overlay_matches_pre_migration_literals() {
        let _guard = lock_scheme();
        let s = maximize_overlay();
        assert_eq!(s.scrim_background, byteui::theme::color::current().scrim);
        assert_eq!(s.scrim_padding, 40.0);
        assert_eq!(s.border.color, byteui::theme::color::current().gold);
        assert_eq!(s.border.width, 1.5);
        assert_eq!(s.border.radius, 10.0.into());
    }

    #[test]
    fn context_menu_matches_pre_migration_literals() {
        let _guard = lock_scheme();
        let s = context_menu();
        // 背景取 `BG` 色 + alpha,模拟 macOS 原生右键菜单的磨砂/半透明观感
        // (iced 无实时高斯模糊可用,退而求其次用半透明打底 + `shell()` 的
        // 软阴影一起近似"浮起且透光"的质感)。alpha 0xfa(≈0.98,2026-09-13
        // 用户反馈原 0xf0/0.94 透得太明显、菜单后面内容看着太清楚,调高压
        // 暗透光,仍留一丝透明感而非彻底不透明)。
        assert_eq!(s.background, Some(parse_hex_color("#0a0e16fa")));
        let border = s.border.expect("context_menu 应有边框");
        assert_eq!(border.color, byteui::theme::color::current().border);
        assert_eq!(border.width, 1.0);
        // 圆角对齐 macOS 原生右键菜单(比之前的 14 更扁平的 9)。
        assert_eq!(border.radius, 9.0.into());
        assert_eq!(s.padding, Padding::from(8.0));
        // 项间距压到 1,让整个菜单看起来更紧凑。
        assert_eq!(s.gap, 1.0);
    }

    #[test]
    fn left_zone_matches_config() {
        let _guard = lock_scheme();
        let s = left_zone();
        // 左右面板区整体用圆角背景浮起:背景填充 BG 色(与内层面板一致),
        // 描边宽为 0 但带圆角(用 0 宽描边保留"无描边"观感,仅让背景走圆角),
        // 四向 margin 做悬浮留白。
        assert_eq!(s.background, Some(byteui::theme::color::current().bg));
        let border = s.border.expect("left_zone 应有圆角边框(宽 0)");
        assert_eq!(border.color, byteui::theme::color::current().border);
        assert_eq!(border.width, 0.0);
        assert_eq!(border.radius, 8.0.into());
        assert_eq!(s.padding, Padding::from(1.0));
    }

    #[test]
    fn right_zone_matches_config() {
        let _guard = lock_scheme();
        let s = right_zone();
        assert_eq!(s.background, Some(byteui::theme::color::current().bg));
        let border = s.border.expect("right_zone 应有圆角边框(宽 0)");
        assert_eq!(border.color, byteui::theme::color::current().border);
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
        let _guard = lock_scheme();
        assert_eq!(resolve_color("#0a0e16"), byteui::theme::color::current().bg);
        assert_eq!(
            resolve_color("#12202a"),
            byteui::theme::color::current().card
        );
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

    /// 防回归:`REGIONS`/`BACKGROUND` 曾经是"颜色也一起烤进 `LazyLock`"的
    /// 全量缓存,首次访问后就冻结——运行时切换配色方案(`set_scheme`)对
    /// `top_bar`/`background` 这类 accessor 完全不生效(2026-09-15 用户
    /// 实测截图发现:切浅色后顶栏/图标栏/终端底色纹丝不动)。颜色解析必须
    /// 像 `scaled_region` 对 `scale()` 那样,在每次 accessor 调用时现读
    /// `byteui::theme::color::current_scheme()`,不能烤进缓存。
    #[test]
    fn top_bar_and_background_track_live_scheme_switch() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Light);
        assert_eq!(
            top_bar().background,
            Some(byteui::theme::color::current().bg)
        );
        assert_eq!(
            preview_pane().background,
            Some(byteui::theme::color::current().panel)
        );
        assert_eq!(
            background(),
            byteui::theme::color::current().tab_active_border
        );
        assert_eq!(
            context_menu().background.map(|c| (c.r, c.g, c.b)),
            Some({
                let bg = byteui::theme::color::current().bg;
                (bg.r, bg.g, bg.b)
            })
        );
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark);
        assert_eq!(
            top_bar().background,
            Some(byteui::theme::color::current().bg)
        );
        assert_eq!(
            background(),
            byteui::theme::color::current().tab_active_border
        );
    }
}
