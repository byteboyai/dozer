//! Toast 的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-30-unified-toast-design.md` 与
//! `docs/superpowers/plans/2026-09-30-unified-toast.md`。
//!
//! 与 8 个模态卡片宿主的三处刻意不同:
//! 1. 窗口只等于"当前 Toast 堆叠的包围盒"(右下角),不覆盖整窗、没有遮罩;
//! 2. **整窗点击穿透**(`set_cursor_hittest(false)`)且建窗**不聚焦**
//!    (`with_active(false)`)——macOS 上点击 `canBecomeKeyWindow == true` 的
//!    子窗口会抢走终端键盘焦点,穿透后任何情况下都不可能拿到焦点;
//! 3. 有 Toast 才建窗,清空即销毁,不常驻空窗口。

use std::sync::Arc;
use std::time::Instant;

use iced_wgpu::wgpu;
use iced_widget::core::{Alignment, Border, Element, Length, Padding};
use iced_widget::{column, container, row, text};
use iced_winit::core::mouse;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::extensions::toast::{Level, Toast};
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::open_child_window_unfocused;

/// 单条 Toast 的逻辑像素尺寸。固定高度,文本超出裁剪(不按文本换行量高),
/// 所以窗口尺寸只取决于条数,`stack_bounds` 是纯函数。
pub(crate) const TOAST_WIDTH: f32 = 380.0;
pub(crate) const TOAST_HEIGHT: f32 = 56.0;
pub(crate) const TOAST_GAP: f32 = 8.0;
/// 距主窗口右/下边缘的逻辑像素边距。下边距留出 footbar 的位置。
pub(crate) const MARGIN_RIGHT: f32 = 16.0;
pub(crate) const MARGIN_BOTTOM: f32 = 48.0;

/// 堆叠指纹:条目 id + 到期时间。同 key 刷新会改到期时间(文本可能同时变),
/// 用它比较就能发现"内容变了要重绘",而只比 id 会漏。
pub(crate) type Stamp = (u64, Instant);

pub(crate) fn stamps_of(items: &[Toast]) -> Vec<Stamp> {
    items.iter().map(|t| (t.id, t.expires)).collect()
}

/// 主窗口外框位置 + 内区尺寸 + 缩放 + 条数 → Toast 窗口的物理位置与尺寸。
/// 窗口尺寸钳到主窗口内且 ≥1(主窗口被缩得比 Toast 还小也不出 0/负尺寸,
/// 否则 wgpu surface 配置会失败)。`count == 0` 按 1 处理。定位约定与
/// `overlay_window::full_window_overlay_bounds` 一致:以 `outer_position`
/// 为原点、`inner_size` 为范围。
pub(crate) fn stack_bounds(
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
    count: usize,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let n = count.max(1);
    let px = |logical: f32| (f64::from(logical) * scale).round() as i64;
    let main_w = i64::from(main_inner_size.width);
    let main_h = i64::from(main_inner_size.height);
    let w = px(TOAST_WIDTH).min(main_w).max(1);
    let h = px(TOAST_HEIGHT * n as f32 + TOAST_GAP * (n - 1) as f32)
        .min(main_h)
        .max(1);
    let x = i64::from(main_outer_pos.x) + (main_w - w - px(MARGIN_RIGHT)).max(0);
    let y = i64::from(main_outer_pos.y) + (main_h - h - px(MARGIN_BOTTOM)).max(0);
    (
        PhysicalPosition::new(x as i32, y as i32),
        PhysicalSize::new(w as u32, h as u32),
    )
}

/// 纯视图:一列固定尺寸卡片(左侧色条 + 文本)。窗口背景透明,圆角与边框
/// 由 iced 自己画。文本用系统默认字体 + `Shaping::Advanced`(中文回退)。
pub(crate) fn view(
    items: &[Toast],
) -> Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let mut col = column![]
        .spacing(TOAST_GAP)
        .width(Length::Fixed(TOAST_WIDTH));
    for t in items {
        let accent = match t.level {
            Level::Info => colors.cyan,
            Level::Success => colors.green,
            Level::Warning => colors.gold,
            Level::Error => colors.red,
        };
        let bar = container(iced_widget::space::horizontal())
            .width(Length::Fixed(4.0))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(accent.into()),
                ..container::Style::default()
            });
        let body = container(
            text(t.text.as_str())
                .size(byteui::theme::font::body())
                .color(colors.cream)
                .shaping(text::Shaping::Advanced),
        )
        .padding(Padding {
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
            left: 12.0,
        })
        .width(Length::Fill)
        .align_y(Alignment::Center);
        let panel = colors.panel;
        col = col.push(
            container(row![bar, body])
                .width(Length::Fixed(TOAST_WIDTH))
                .height(Length::Fixed(TOAST_HEIGHT))
                .clip(true)
                .style(move |_t: &iced_widget::Theme| container::Style {
                    background: Some(panel.into()),
                    border: Border {
                        color: accent,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                }),
        );
    }
    col.into()
}

pub(crate) struct ToastOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    stamps: Vec<Stamp>,
}

impl ToastOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn stamps(&self) -> &[Stamp] {
        &self.stamps
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        stamps: Vec<Stamp>,
        el: &ActiveEventLoop,
    ) -> ToastOverlay {
        let scale = main_window.scale_factor();
        let (pos, size) = stack_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            stamps.len(),
        );
        let window = open_child_window_unfocused(main_window, pos, size, "toast", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        ToastOverlay {
            window,
            gpu,
            stamps,
        }
    }

    /// 堆叠变了:换指纹并按新条数重定位/缩放窗口。
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        stamps: Vec<Stamp>,
    ) {
        self.stamps = stamps;
        self.reposition(device, main_outer_pos, main_inner_size, scale);
    }

    /// 主窗口 resize 后重新贴右下角(条数不变)。
    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let (pos, size) = stack_bounds(main_outer_pos, main_inner_size, scale, self.stamps.len());
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, items: &[Toast]) {
        self.gpu
            .redraw(&self.window, mouse::Cursor::Unavailable, view(items));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_toast_anchors_bottom_right_of_main_window() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
            1.0,
            1,
        );
        assert_eq!(size, PhysicalSize::new(380, 56));
        assert_eq!(pos, PhysicalPosition::new(904, 746));
    }

    #[test]
    fn three_toasts_grow_upward_with_gaps() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
            1.0,
            3,
        );
        // 3*56 + 2*8 = 184
        assert_eq!(size, PhysicalSize::new(380, 184));
        assert_eq!(pos, PhysicalPosition::new(904, 618));
    }

    #[test]
    fn retina_scale_multiplies_everything() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(2400, 1600),
            2.0,
            1,
        );
        assert_eq!(size, PhysicalSize::new(760, 112));
        assert_eq!(pos, PhysicalPosition::new(1708, 1442));
    }

    #[test]
    fn tiny_main_window_clamps_size_and_never_goes_zero_or_negative() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(200, 30),
            1.0,
            1,
        );
        assert_eq!(size, PhysicalSize::new(200, 30));
        assert_eq!(pos, PhysicalPosition::new(100, 50));

        let (_, size) = stack_bounds(PhysicalPosition::new(0, 0), PhysicalSize::new(0, 0), 1.0, 1);
        assert!(size.width >= 1 && size.height >= 1);
    }

    #[test]
    fn zero_count_is_treated_as_one_so_size_is_never_zero() {
        let (_, size) = stack_bounds(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1200, 800),
            1.0,
            0,
        );
        assert_eq!(size, PhysicalSize::new(380, 56));
    }
}
