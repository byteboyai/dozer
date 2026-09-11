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
    /// 顶栏项目页签的"默认/合适宽"(已含全局 scale)。少数页签时
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
    /// 菜单项上下内边距（设计基准 5，比早期的 7 更紧凑，压缩行高留白）。
    pub menu_pad_v: f32,
    /// 菜单项左右内边距（设计基准 14，对齐 macOS 原生右键菜单的横向留白）。
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
            project_tab_max_width: 160.0,
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
            menu_pad_v: 5.0,
            menu_pad_h: 14.0,
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
    (
        current().initial_window_width,
        current().initial_window_height,
    )
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
        assert_eq!(t.project_tab_max_width, 160.0);
        assert_eq!(t.tab_bar_avail_px, 360.0);
        assert_eq!(t.rail_button_size, 32.0);
        assert_eq!(t.tab_button_size, 24.0);
        assert_eq!(t.tab_arrow_button_size, 18.0);
        assert_eq!(t.menu_item_width, 160.0);
        assert_eq!(t.menu_gap, 8.0);
        assert_eq!(t.menu_pad_v, 5.0);
        assert_eq!(t.menu_pad_h, 14.0);
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
        assert_eq!(
            c.icon_rail_width,
            GeometryTokens::byteboy2077().icon_rail_width
        );
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
