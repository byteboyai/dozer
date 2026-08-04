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
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/theme/workspace.json");

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
    status_bar_height: f32,
    context_menu_width: f32,
    context_menu_height: f32,
    chrome_width_px: f32,
    chrome_height_px: f32,
    preview_chrome_top_px: f32,
    browser_chrome_top_px: f32,
    maximize_overlay_padding: f32,
    project_tab_gap: f32,
    project_tab_avail_px: f32,
    tab_bar_avail_px: f32,
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

/// 图标栏固定宽度(逻辑像素)，左右各一条。
pub fn icon_rail_width() -> f32 {
    GEOMETRY.icon_rail_width
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
/// 有严格几何推导。
pub fn min_window_height() -> f32 {
    GEOMETRY.min_window_height
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

/// 顶栏固定高（逻辑像素）。与 `top_bar` 容器高度同源，勿各写各的。
pub fn top_bar_height() -> f32 {
    GEOMETRY.top_bar_height
}

/// 单条状态栏固定高（逻辑像素）。与 `status_bar_container` 同源。
pub fn status_bar_height() -> f32 {
    GEOMETRY.status_bar_height
}

/// 右键菜单浮层的最坏情形(目录:8 项)外接宽/高（逻辑像素）,main.rs 在
/// `RightClickAt` 落点处用它把坐标钳制在窗口内,避免菜单下沿/右沿超出
/// 窗口导致底部几项点不到（Important #7）。宽度取 `menu_item` 固定宽
/// 180 加列表容器左右 padding；高度按目录菜单最多 8 项估算,每项文字
/// 13 号加上下 padding 约 28px,项间 spacing 2,列表容器上下 padding 6,
/// 不必像素级精确,留够余量保证任何一项都可点即可。文件菜单项更少,用
/// 目录的最坏值同时覆盖两种情况更简单。
pub fn context_menu_width() -> f32 {
    GEOMETRY.context_menu_width
}

pub fn context_menu_height() -> f32 {
    GEOMETRY.context_menu_height
}

/// 终端栏内"非网格"开销的近似值：左右 padding、表头行、tab 栏行、
/// 行间 spacing。用于把窗口像素尺寸换算成终端 pane 的可用像素尺寸——
/// 这是估算值，不追求像素级精确（`term_view::grid_size` 本身就向下
/// 取整，差几像素不影响可用性，差太多也只是终端网格偏保守/偏宽松）。
pub fn chrome_width_px() -> f32 {
    GEOMETRY.chrome_width_px
}

pub fn chrome_height_px() -> f32 {
    GEOMETRY.chrome_height_px
}

/// 文件预览分支(`LeftView::Files`)内容区上方的 chrome 高度:pane 上内
/// 边距 8 + tab 栏 30。地址栏已去(文件只走项目树打开),`column` 里只剩
/// tab 栏一个子项,不再有子项间 spacing。
pub fn preview_chrome_top_px() -> f32 {
    GEOMETRY.preview_chrome_top_px
}

/// 浏览器分支(`LeftView::Web`)内容区上方的 chrome 高度:pane 上内边距 8
/// + tab 栏 30 + 两子项间 spacing 4 + 地址栏 30(浏览器仍保留地址栏)。
pub fn browser_chrome_top_px() -> f32 {
    GEOMETRY.browser_chrome_top_px
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
/// 长短浮动,iced 立即模式在构造阶段拿不到真实分配宽,估偏只会让翻页边界差
/// 一个页签(与终端 tab 栏同一套取舍)。
pub fn project_tab_avail_px() -> f32 {
    GEOMETRY.project_tab_avail_px
}

/// tab 栏箭头翻页/tab 内容区可视宽的保守估值（逻辑像素）。`tab_bar`/
/// 预览 tab 栏都拿不到窗口尺寸（故意不引入这层依赖——见 P1L T5 brief），
/// 估偏只影响翻页边界（早一两个 tab 触发/到头），不影响正确性或崩溃。
pub fn tab_bar_avail_px() -> f32 {
    GEOMETRY.tab_bar_avail_px
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
        assert_eq!(top_bar_height(), 44.0);
        assert_eq!(status_bar_height(), 26.0);
        assert_eq!(context_menu_width(), 200.0);
        assert_eq!(context_menu_height(), 280.0);
        assert_eq!(chrome_width_px(), 16.0);
        assert_eq!(chrome_height_px(), 50.0);
        assert_eq!(preview_chrome_top_px(), 38.0);
        assert_eq!(browser_chrome_top_px(), 72.0);
        assert_eq!(maximize_overlay_padding(), 40.0);
        assert_eq!(project_tab_gap(), 4.0);
        assert_eq!(project_tab_avail_px(), 420.0);
        assert_eq!(tab_bar_avail_px(), 360.0);
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
