//! 独立原生窗口共用的建窗样板 + 居中定位算法——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。`open_overlay`/`reposition_overlay` 是 `2026-09-23-standard-
//! dialog-overlay-design.md` 迁移的 8 个模态卡片宿主共用的建窗口+建 GPU
//! 管线、以及 resize/主窗口移动跟随两段胶水(这两段在那 8 个消费方的
//! `open`/`reposition` 方法体里逐字重复,唯一变量是各自的
//! `card_logical_size()`/窗口 tag)。IME/原生右键菜单挂靠仍由各消费方
//! 自己在拿到 `open_overlay` 返回的 `window` 后按需调用——不是每个消费方
//! 都需要,不塞进这个共用函数。
//!
//! **2026-09-27 背景遮罩改造**:overlay 窗口本身原来只等于卡片大小,四周
//! 没有任何东西,点击卡片外的区域会直接穿透打到主窗体上(可操作主窗体的
//! 按钮/标签页等,与"弹窗打开时不能操作主窗体"的预期相悖)。改造后 overlay
//! 窗口覆盖整个主窗口客户区(`full_window_overlay_bounds`),卡片仍是原来
//! 的大小,由 `backdrop_card` 套一层"整窗口居中 + 半透明遮罩"的外壳——
//! 窗口本身天然挡住对主窗体的点击(遮罩区域没有 `on_press`,点了没反应,
//! 只是穿不透,不是"点外部关闭")。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_widget::container;
use iced_widget::core::{Element, Length, alignment};
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::platform::overlay_gpu::OverlayGpu;

/// 主窗口外框物理位置 + 物理尺寸 → overlay 窗口应放的物理位置与物理尺寸——
/// 覆盖整个主窗口客户区(不再只等于卡片大小,见上面模块文档的改造说明)。
pub(crate) fn full_window_overlay_bounds(
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    (main_outer_pos, main_inner_size)
}

/// 卡片在 `backdrop_card` 居中布局后,相对 overlay 窗口左上角的逻辑偏移。
/// `file_history_overlay` 定位内嵌 diff webview 时需要这个偏移——它的原生
/// 子视图坐标系是"整扇窗口",不是"卡片";卡片本身在 iced 视图树里的居中
/// 交给 `backdrop_card` 的布局自动处理,不需要这个偏移。
pub(crate) fn centered_card_offset(
    window_logical_size: LogicalSize<f32>,
    card_logical_size: LogicalSize<f32>,
) -> LogicalPosition<f32> {
    LogicalPosition::new(
        (window_logical_size.width - card_logical_size.width) / 2.0,
        (window_logical_size.height - card_logical_size.height) / 2.0,
    )
}

/// overlay 窗口本身是 `with_decorations(false)` 的 borderless 窗口,macOS
/// 不会像主窗口(保留 `.titled` style mask,见 `window.rs` 顶部注释)那样自动
/// 套系统原生圆角——遮罩若填满整扇 overlay 窗口的直角矩形,四角会略微戳出
/// 主窗体实际可见的圆角之外。这里给遮罩本身画成圆角矩形来补偿,而不是去
/// 挂原生 CALayer 圆角:overlay 窗口已 `with_transparent(true)` 逐像素透明,
/// 遮罩四角留空即可透出下层。
///
/// 取值不经 `theme::region` 的 `scaled_*` 折算——那条链路乘的是 Dozer 自己
/// 的内容缩放(Cmd +/-,`icon_size::scale()`),而这里要匹配的是 macOS 窗口
/// 服务端渲染的物理圆角,与 app 内容缩放无关,必须是与 UI 缩放脱钩的固定值。
/// 10.0 是肉眼比对当前系统窗口圆角选的近似值(macOS 用连续曲率的"squircle"
/// 描边,不是纯圆弧,像素级完全重合做不到,但目测已经贴合)。
const BACKDROP_CORNER_RADIUS: f32 = 10.0;

/// 给卡片元素套一层"整窗口居中 + 半透明背景遮罩"的外壳——遮罩色复用
/// `theme::region::maximize_overlay().scrim_background`(放大态浮层同款
/// token,不是另起一个硬编码颜色)。遮罩容器没有 `on_press`:点击穿不透
/// 到主窗体,但也不产生任何消息去关弹窗(已与用户确认:点遮罩=无反应,
/// 不是"点外部关闭")。遮罩圆角见 `BACKDROP_CORNER_RADIUS` 注释。
pub(crate) fn backdrop_card<'a, Message: 'a>(
    card: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    card_logical: LogicalSize<f32>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let scrim = crate::theme::region::maximize_overlay().scrim_background;
    container(
        container(card)
            .width(Length::Fixed(card_logical.width))
            .height(Length::Fixed(card_logical.height)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(move |_theme: &iced_widget::Theme| container::Style {
        background: Some(scrim.into()),
        border: iced_widget::core::Border {
            radius: BACKDROP_CORNER_RADIUS.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// 挂成主窗口子窗口的无装饰透明窗口。只建窗口本身,不建 wgpu 渲染管线(见
/// `OverlayGpu`)、不设 IME/原生菜单挂靠(消费方按需自己调用,大多数消费方
/// 不需要——见 spec「架构」第 1 节)。
///
/// **2026-09-27 去掉 `WindowLevel::AlwaysOnTop`**:macOS 上它映射到
/// `kCGFloatingWindowLevel`,是跨 app 的全局悬浮层级——会导致这扇窗口飘在
/// 其他 app(如浏览器)窗口之上,即便 Dozer 本身已切到后台。真正需要的"盖过
/// 主窗口自己的 wry webview 子视图"效果,靠 `with_parent_window` 触发的
/// `addChildWindow_ordered(NSWindowAbove)`(winit 侧无条件执行,见
/// `window_delegate.rs`)已经保证——它是"这扇窗口相对父窗口的层级"，不依赖
/// 全局窗口层级,默认 `WindowLevel::Normal` 即可,不再单独设置。
pub(crate) fn open_child_window(
    main_window: &Window,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    title: &str,
    el: &ActiveEventLoop,
) -> Arc<Window> {
    use winit::raw_window_handle::HasWindowHandle;

    let parent_handle = main_window
        .window_handle()
        .expect("main window handle")
        .as_raw();
    let attrs = Window::default_attributes()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_position(pos)
        .with_inner_size(size);
    // Safety: `parent_handle` 取自仍存活的主窗口(`Ready` 持有的
    // `Arc<Window>`),本函数返回前主窗口不会被 drop。
    let attrs = unsafe { attrs.with_parent_window(Some(parent_handle)) };
    let window = Arc::new(el.create_window(attrs).expect("create overlay window"));
    window.focus_window();
    window
}

/// `open` 方法体里"建覆盖主窗口的子窗口 + 建这扇窗口自己的 wgpu 渲染管线"
/// 那两步在 8 个模态卡片宿主之间逐字重复的部分。调用方自己的 `open` 只需要
/// 传窗口 tag,再按需对返回的 `window` 调 `set_ime_allowed`/
/// `install_content_view`、拼自己结构体里其余字段;卡片大小由调用方自己在
/// `redraw`/`handle_input` 里通过 `backdrop_card` 套壳时决定,不影响这里的
/// 窗口本身几何(窗口现在恒等于主窗口客户区大小)。
pub(crate) fn open_overlay(
    main_window: &Arc<Window>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    instance: &wgpu::Instance,
    tag: &str,
    el: &ActiveEventLoop,
) -> (Arc<Window>, OverlayGpu) {
    let scale = main_window.scale_factor();
    let (pos, size) = full_window_overlay_bounds(
        main_window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0)),
        main_window.inner_size(),
    );
    let window = open_child_window(main_window, pos, size, tag, el);
    let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
    (window, gpu)
}

/// `reposition` 方法体在 8 个模态卡片宿主之间逐字重复的部分——主窗口移动
/// /resize 后跟着重新覆盖 + 重配置 surface,不再需要各自的
/// `card_logical_size()`(窗口大小只取决于主窗口,同 `open_overlay`)。
pub(crate) fn reposition_overlay(
    window: &Window,
    gpu: &mut OverlayGpu,
    device: &wgpu::Device,
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
) {
    let (pos, size) = full_window_overlay_bounds(main_outer_pos, main_inner_size);
    window.set_outer_position(pos);
    if window.inner_size() != size {
        let _ = window.request_inner_size(size);
        gpu.reconfigure(device, size, scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_window_overlay_bounds_matches_main_window_exactly() {
        let (pos, size) = full_window_overlay_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
        );
        assert_eq!(pos, PhysicalPosition::new(100, 50));
        assert_eq!(size, PhysicalSize::new(1200, 800));
    }

    #[test]
    fn centered_card_offset_centers_within_window() {
        let offset = centered_card_offset(
            LogicalSize::new(1200.0, 800.0),
            LogicalSize::new(400.0, 640.0),
        );
        assert_eq!(offset.x, 400.0);
        assert_eq!(offset.y, 80.0);
    }

    #[test]
    fn centered_card_offset_zero_when_card_fills_window() {
        let offset = centered_card_offset(
            LogicalSize::new(600.0, 640.0),
            LogicalSize::new(600.0, 640.0),
        );
        assert_eq!(offset.x, 0.0);
        assert_eq!(offset.y, 0.0);
    }
}
