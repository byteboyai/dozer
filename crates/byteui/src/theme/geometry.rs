// crates/dozer-app/src/workspace_geometry.rs
//! 外壳布局的几何常量(图标栏宽、分隔线宽、区域最小宽、窗口尺寸下限、
//! 顶栏/状态栏高、右键菜单尺寸、终端 chrome 开销估算、页签估算宽度等),
//! 编译期内嵌 `assets/theme/workspace.json` 的 `geometry` 节点,启动时
//! 解析一次。
//!
//! 与 `chrome_style.rs`(`regions` 节点,管区域外层容器样式)、
//! `workspace_font.rs`(`font_sizes` 节点,管控件内部文字字号)同一份
//! `workspace.json`——三个模块各自只解析自己关心的顶层字段,互不干扰。
//!
//! 解析失败(格式错误、缺字段)直接 panic:开发期配置错误,不是需要
//! 优雅降级的运行时数据(同 `chrome_style.rs`/`workspace_font.rs` 的定位)。
use super::icon_size;
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../../assets/theme/workspace.json");

#[derive(Deserialize)]
struct Geometry {
    icon_rail_width: f32,
    divider_width: f32,
    min_zone_width: f32,
    min_split_ratio: f32,
    max_split_ratio: f32,
    initial_window_width: f32,
    initial_window_height: f32,
    min_window_height: f32,
    top_bar_height: f32,
    /// 顶栏项目页签的"默认/合适宽"(设计基准 200,已含全局 scale)。少数页签时
    /// 每片统一用这个宽(固定,左对齐不撑爆);页签多到塞不下这个宽时,
    /// `project_tabs_row` 按可用宽均分把它收窄到低于此值。它既是默认宽也是上限宽。
    project_tab_max_width: f32,
    /// 顶栏页签行最右"＋"按钮的估算宽(设计基准 36,已含全局 scale)。`project_tabs_row`
    /// 用它在布局期从可用宽里预留出"＋"的位置,避免页签在拥挤时被压到"＋"上。
    project_tab_add_button_width: f32,
    status_bar_height: f32,
    /// 底部 footbar 系统信息条高度（设计基准 22，已含全局 scale）。与
    /// `status_bar_height` 解耦——in-pane status bar 保持 26，footbar 单独更矮更紧凑。
    footbar_height: f32,
    context_menu_width: f32,
    context_menu_height: f32,
    chrome_width_px: f32,
    chrome_height_px: f32,
    preview_chrome_top_px: f32,
    browser_chrome_top_px: f32,
    maximize_overlay_padding: f32,
    project_tab_gap: f32,
    tab_bar_avail_px: f32,
    /// 左/右图标栏按钮（rail_icon_button）的方形命中区边长（设计基准 32）。
    rail_button_size: f32,
    /// tab 栏内小方形图标按钮通用命中区边长（关闭 × / 星标 / 收藏夹，
    /// 设计基准 24）。翻页箭头走更小的 `tab_arrow_button_size`。
    tab_button_size: f32,
    /// 翻页箭头按钮专属命中区边长，小于 `tab_button_size`（设计基准 18）。
    /// `tab_button_size` 同时给关闭 ×/星标/收藏夹按钮用，不能跟着箭头一起
    /// 缩小；箭头独立一个更紧凑的方形，让 `<`/`>` 的横向留白随之变窄。
    tab_arrow_button_size: f32,
    /// 右键菜单项（menu_item）固定宽（设计基准 180）；`context_menu_width`
    /// 由它 + 菜单列表左右 padding 推导，两者需同步缩放。
    menu_item_width: f32,
    /// 菜单项内"图标↔文字"间距（设计基准 8）。
    menu_gap: f32,
    /// 菜单项上下内边距（设计基准 6）。
    menu_pad_v: f32,
    /// 菜单项左右内边距（设计基准 10）。
    menu_pad_h: f32,
    /// H0 项目中心左栏固定宽（设计基准 248，Figma 同值）。
    h0_sidebar_width: f32,
}

/// `workspace.json` 顶层结构里本模块只关心的部分——`regions`/`font_sizes`
/// 节点是另外两个模块的地盘,这里不声明,serde 默认忽略未知字段。
#[derive(Deserialize)]
struct RawWorkspaceFile {
    geometry: Geometry,
}

fn load(raw: &str) -> Geometry {
    let file: RawWorkspaceFile =
        serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败,geometry 节点)");
    file.geometry
}

static GEOMETRY: LazyLock<Geometry> = LazyLock::new(|| load(RAW));

/// 图标栏固定宽度(逻辑像素)，左右各一条。已含全局 scale——`rail_button_size`
/// 同步缩放，否则放大后按钮会撑破图标栏。
pub fn icon_rail_width() -> f32 {
    GEOMETRY.icon_rail_width * icon_size::scale()
}

/// 每条分隔线的命中区/渲染宽度(逻辑像素)。视觉线本身 2px,居中于此区间内。
/// 几何公式必须把每一条实际渲染出来的分隔线从可分配空间里扣掉(LeftRight
/// 一条恒在 + 当前配对视图内部一条),否则 webview bounds/IME 光标/命中
/// 测试会和 `view()` 里 `row!` 实际渲染的像素错位。
pub fn divider_width() -> f32 {
    GEOMETRY.divider_width
}

pub fn min_zone_width() -> f32 {
    GEOMETRY.min_zone_width
}

pub fn min_split_ratio() -> f32 {
    GEOMETRY.min_split_ratio
}

pub fn max_split_ratio() -> f32 {
    GEOMETRY.max_split_ratio
}

/// 左右双栏 zone 的统一"列表侧"默认占比——文件树↔文件预览、项目信息↔项目
/// 预览、Agent 列表↔终端、对话列表↔审阅四个配对共用同一个首次默认值(0.35,
/// 与文件树那份"合适"的分割一致),让所有双栏 zone 的初始宽度分配保持统一。
/// 用户手动拖拽后以各自存档的 split 为准(见 `apply_column_drag`),此处只在
/// 无存档的首次默认时生效。
pub fn default_split_ratio() -> f32 {
    0.35
}

/// 建窗时的初始窗口逻辑尺寸——仅在从未持久化过窗口尺寸(`layout.json`
/// 不存在/`window_width`/`window_height` 缺字段)时用作兜底,正常情况下
/// `main.rs` 建窗读的是 `App::window_size_pref()`(优先取上次退出前存的
/// 尺寸)。`App::window_size` 字段的初值也用它——两处必须同源,否则第一帧
/// 的几何(左面板区有效宽/终端网格)会按一个和真实窗口不同的宽度算。
pub fn initial_window_size() -> (f32, f32) {
    (
        GEOMETRY.initial_window_width,
        GEOMETRY.initial_window_height,
    )
}

/// 高度最小值只求"顶栏 + 面板 chrome + 若干行终端"能放下,不像宽度那样
/// 有严格几何推导。已含全局 scale——顶栏/状态栏等高随 `scale` 变高时,
/// 这个下限也得跟着涨,否则窗口缩不到比放大后的 chrome 更小而被钳死。
pub fn min_window_height() -> f32 {
    GEOMETRY.min_window_height * icon_size::scale()
}

/// 窗口最小内尺寸(逻辑像素,宽)。按"两条图标栏 + 那条恒在的 LeftRight
/// 分隔线 + 左右面板区各 `min_zone_width`"推出:窗口再窄下去,两个面板区
/// 就不可能同时满足最小宽,渲染只能靠 `clamp_left_width` 兜底压缩。这是
/// 双保险的外层——不能取代 `clamp_left_width`(用户持久化的 left_width
/// 可能远大于这个最小宽)。
///
/// 推导值,不是 JSON 里的独立字段——由 `icon_rail_width`/`divider_width`/
/// `min_zone_width` 三者算出,避免和它们各写各的、迟早对不上。
pub fn min_window_width() -> f32 {
    2.0 * icon_rail_width() + divider_width() + 2.0 * min_zone_width()
}

/// 顶栏固定高（逻辑像素）。与 `top_bar` 容器高度同源，勿各写各的。已含全局 scale。
pub fn top_bar_height() -> f32 {
    GEOMETRY.top_bar_height * icon_size::scale()
}

/// 顶栏项目页签的"默认/合适宽"(逻辑像素),已含全局 scale。少数页签时每片固定
/// 用这个宽;页签多到塞不下时才由 `project_tabs_row` 均分收窄。见
/// `project_tab_max_width` 字段注释。
pub fn project_tab_max_width() -> f32 {
    GEOMETRY.project_tab_max_width * icon_size::scale()
}

/// 顶栏页签行最右"＋"按钮的估算宽(逻辑像素),已含全局 scale。仅用于在布局期
/// 从页签可用宽里预留"＋"的位置,估偏只影响开始收窄的临界点,不影响正确性。
pub fn project_tab_add_button_width() -> f32 {
    GEOMETRY.project_tab_add_button_width * icon_size::scale()
}

/// 统一滚动条(轨道)宽度(逻辑像素),已含全局 scale。各面板共用,收窄一致。
pub fn scrollbar_width() -> f32 {
    10.0 * icon_size::scale()
}

/// 统一滚动条滑块(thumb)宽度(逻辑像素),已含全局 scale。同时作为滑块圆角
/// 半径的基准,让滑块呈细窄的胶囊形;各面板共用,收窄一致。
pub fn scrollbar_thumb_width() -> f32 {
    4.0 * icon_size::scale()
}

/// 单条状态栏固定高（逻辑像素）。与 `status_bar_container` 同源。已含全局 scale。
pub fn status_bar_height() -> f32 {
    GEOMETRY.status_bar_height * icon_size::scale()
}

/// 底部 footbar 系统信息条高度（逻辑像素）。与 in-pane status bar 解耦，
/// 单独更矮更紧凑（见 `footbar.rs`）。已含全局 scale。
pub fn footbar_height() -> f32 {
    GEOMETRY.footbar_height * icon_size::scale()
}

/// 右键菜单浮层的最坏情形(目录:9 项)外接宽/高（逻辑像素）,main.rs 在
/// `RightClickAt` 落点处用它把坐标钳制在窗口内,避免菜单下沿/右沿超出
/// 窗口导致底部几项点不到（Important #7）。宽度取 `menu_item` 固定宽
/// 160 加列表容器左右 padding；高度按目录菜单最多 9 项估算,每项文字
/// 13 号加上下 padding 约 28px,项间 spacing 2,列表容器上下 padding 6,
/// 不必像素级精确,留够余量保证任何一项都可点即可。文件菜单项更少,用
/// 目录的最坏值同时覆盖两种情况更简单。
pub fn context_menu_width() -> f32 {
    GEOMETRY.context_menu_width * icon_size::scale()
}

pub fn context_menu_height() -> f32 {
    GEOMETRY.context_menu_height * icon_size::scale()
}

/// 终端栏内"非网格"开销的近似值：左右 padding、表头行、tab 栏行、
/// 行间 spacing。用于把窗口像素尺寸换算成终端 pane 的可用像素尺寸——
/// 这是估算值，不追求像素级精确（`term_view::grid_size` 本身就向下
/// 取整，差几像素不影响可用性，差太多也只是终端网格偏保守/偏宽松）。
/// 已含全局 scale——顶栏/状态栏等高随 `scale` 变高时,这份开销估算也要
/// 跟着涨,否则终端网格会按偏小的 chrome 估算,顶部被栏体吃掉几行。
pub fn chrome_width_px() -> f32 {
    GEOMETRY.chrome_width_px * icon_size::scale()
}

pub fn chrome_height_px() -> f32 {
    GEOMETRY.chrome_height_px * icon_size::scale()
}

/// 文件预览分支(`LeftView::Files`)内容区上方的 chrome 高度:pane 上内
/// 边距 8 + tab 栏 30。地址栏已去(文件只走项目树打开),`column` 里只剩
/// tab 栏一个子项,不再有子项间 spacing。已含全局 scale。
pub fn preview_chrome_top_px() -> f32 {
    GEOMETRY.preview_chrome_top_px * icon_size::scale()
}

/// 浏览器分支(`LeftView::Web`)内容区上方的 chrome 高度:pane 上内边距 8
/// + tab 栏 30 + 两子项间 spacing 4 + 地址栏 30(浏览器仍保留地址栏)。
///
/// 已含全局 scale。
pub fn browser_chrome_top_px() -> f32 {
    GEOMETRY.browser_chrome_top_px * icon_size::scale()
}

/// `maximize_overlay` 里 dim 背景到金色描边盒子的内边距(逻辑像素)。
/// `preview_content_bounds`/`is_in_preview_column` 换算放大态几何时必须
/// 复用这个值,不能各写各的字面量 40.0——否则两处一旦有一处改了内边距,
/// webview 摆位就会和实际渲染出的金色描边盒子错位(与本文件其它几何
/// 常量共享同一原则:渲染侧和几何公式侧不能有第二份真相)。
pub fn maximize_overlay_padding() -> f32 {
    GEOMETRY.maximize_overlay_padding
}

/// 项目页签之间的间距;`tab_window` 的宽度估算与实际渲染必须用同一个值,
/// 否则翻页边界会与眼睛看到的差一个页签。
pub fn project_tab_gap() -> f32 {
    GEOMETRY.project_tab_gap
}

/// 顶栏留给项目页签(裁剪窗口内)的估算可视宽,逻辑像素。与
/// `tab_bar_avail_px` 同性质的粗估常量:顶栏中段实际宽度随窗口宽/目标胶囊
/// tab 栏箭头翻页/tab 内容区可视宽的保守估值（逻辑像素）。`tab_bar`/
/// 预览 tab 栏都拿不到窗口尺寸（故意不引入这层依赖——见 P1L T5 brief），
/// 估偏只影响翻页边界（早一两个 tab 触发/到头），不影响正确性或崩溃。
pub fn tab_bar_avail_px() -> f32 {
    GEOMETRY.tab_bar_avail_px
}

/// 左/右图标栏按钮方形命中区边长，已含全局 scale。
pub fn rail_button_size() -> f32 {
    GEOMETRY.rail_button_size * icon_size::scale()
}

/// 顶栏页签翻页箭头按钮方形命中区边长，已含全局 scale。
pub fn tab_button_size() -> f32 {
    GEOMETRY.tab_button_size * icon_size::scale()
}

/// 翻页箭头（`tab_arrow_button`）专属方形命中区边长，已含全局 scale。比
/// `tab_button_size` 小，配合更小的 `tab_arrow` 字形让 `<`/`>` 横向留白更窄。
pub fn tab_arrow_button_size() -> f32 {
    GEOMETRY.tab_arrow_button_size * icon_size::scale()
}

/// 右键菜单项固定宽，已含全局 scale（与 `context_menu_width` 同步缩放）。
pub fn menu_item_width() -> f32 {
    GEOMETRY.menu_item_width * icon_size::scale()
}

/// 菜单项内"图标↔文字"间距，已含全局 scale。
pub fn menu_gap() -> f32 {
    GEOMETRY.menu_gap * icon_size::scale()
}

/// 菜单项上下内边距，已含全局 scale。
pub fn menu_pad_v() -> f32 {
    GEOMETRY.menu_pad_v * icon_size::scale()
}

/// 菜单项左右内边距，已含全局 scale。
pub fn menu_pad_h() -> f32 {
    GEOMETRY.menu_pad_h * icon_size::scale()
}

/// H0 项目中心左栏固定宽（逻辑像素），已含全局 scale。
pub fn h0_sidebar_width() -> f32 {
    GEOMETRY.h0_sidebar_width * icon_size::scale()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:每个字段的解析结果必须和改动前 workspace.rs 里的字面量
    /// 完全一致——纯代码搬家,数值不该变。
    #[test]
    fn values_match_pre_migration_literals() {
        assert_eq!(icon_rail_width(), 44.0);
        assert_eq!(divider_width(), 8.0);
        assert_eq!(min_zone_width(), 320.0);
        assert_eq!(min_split_ratio(), 0.2);
        assert_eq!(max_split_ratio(), 0.8);
        assert_eq!(initial_window_size(), (1440.0, 900.0));
        assert_eq!(min_window_height(), 480.0);
        assert_eq!(top_bar_height(), 40.0);
        assert_eq!(status_bar_height(), 26.0);
        assert_eq!(footbar_height(), 22.0);
        assert_eq!(context_menu_width(), 180.0);
        assert_eq!(context_menu_height(), 310.0);
        assert_eq!(chrome_width_px(), 16.0);
        assert_eq!(chrome_height_px(), 50.0);
        assert_eq!(preview_chrome_top_px(), 38.0);
        assert_eq!(browser_chrome_top_px(), 72.0);
        assert_eq!(maximize_overlay_padding(), 40.0);
        assert_eq!(project_tab_gap(), 4.0);
        assert_eq!(project_tab_max_width(), 140.0);
        assert_eq!(tab_bar_avail_px(), 360.0);
        assert_eq!(rail_button_size(), 32.0);
        assert_eq!(tab_button_size(), 24.0);
        assert_eq!(tab_arrow_button_size(), 18.0);
        assert_eq!(menu_item_width(), 160.0);
        assert_eq!(menu_gap(), 8.0);
        assert_eq!(menu_pad_v(), 6.0);
        assert_eq!(menu_pad_h(), 10.0);
        assert_eq!(h0_sidebar_width(), 248.0);
    }

    #[test]
    fn min_window_width_is_derived_not_duplicated_in_json() {
        assert_eq!(
            min_window_width(),
            2.0 * icon_rail_width() + divider_width() + 2.0 * min_zone_width()
        );
        assert_eq!(min_window_width(), 736.0);
    }

    #[test]
    #[should_panic(expected = "workspace.json 格式错误")]
    fn malformed_json_panics() {
        load(r#"{"geometry": {"icon_rail_width": 44.0}}"#);
    }
}
