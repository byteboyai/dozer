//! 设置弹窗(主题 + Git 账户连接)的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 结构对照 `ProjectCreateOverlay`:接入 IME 与原生右键菜单(PAT 输入框的
//! 粘贴、可能的 CJK 输入),**故意不接入 `FocusTracker`**——2026-09-27 用户
//! 要求"点击窗口以外的地方不需要关闭窗口",与"新建项目"弹窗一致,只认 Esc
//! 键 / 显式"关闭"按钮。既然不再有失焦即关闭,`State::suppress_next_blur`
//! 那套吞失焦机制也随之作废("没有 PAT?点此生成"拉起浏览器导致的失焦本
//! 就不会再关窗)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::settings;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{backdrop_card, full_window_overlay_bounds};

/// 卡片逻辑尺寸——与"新建项目"弹窗(`project_create::card_logical_size`)
/// 完全一致:宽度 = 主窗口宽度 40%(下限 560),高度 = 主窗口高度 75%(下限
/// 520),保证两类弹窗窗口大小统一(2026-09-27 用户要求样式对齐创建项目弹窗)。
fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    let logical: LogicalSize<f32> = window.inner_size().to_logical(window.scale_factor());
    let size = crate::extensions::project_create::card_logical_size(logical.width, logical.height);
    LogicalSize::new(size.width, size.height)
}

pub(crate) struct SettingsOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// 创建这扇窗口时的共享对话框身份。销毁 / 回包按它定位实例。
    key: bytehost_settings::dialog::DialogKey,
}

impl SettingsOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn key(&self) -> bytehost_settings::dialog::DialogKey {
        self.key
    }

    pub(crate) fn focus(&self) {
        self.window.focus_window();
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn window_arc(&self) -> Arc<Window> {
        self.window.clone()
    }

    pub(crate) fn try_open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
        key: bytehost_settings::dialog::DialogKey,
    ) -> Result<SettingsOverlay, String> {
        let scale = main_window.scale_factor();
        let (pos, size) = full_window_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
        );
        let (window, gpu) = bytehost_desktop::dialog::try_create_dialog_resources(
            || {
                let window = bytehost_desktop::overlay::try_open_child_window(
                    main_window,
                    pos,
                    size,
                    "settings",
                    el,
                )?;
                window.set_ime_allowed(true);
                Ok(window)
            },
            |window| OverlayGpu::try_open(window, instance, adapter, device, queue, size, scale),
        )?;

        Ok(SettingsOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            key,
        })
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let (pos, size) = full_window_overlay_bounds(main_outer_pos, main_inner_size);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(state) = app.settings.as_ref() else {
            return;
        };
        let content = backdrop_card(
            settings::settings_card(state, app.locale.active_locale()).map(Message::Settings),
            card_logical_size(&self.window),
        );
        // 建界面 / 空事件布局 / draw / present 都在共享 `OverlayGpu` 里,
        // 产品侧不再维护第二份渲染与输入策略。
        self.gpu.redraw(&self.window, self.cursor, content);
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        let was_composing = self.gpu.is_composing();
        let escape = matches!(event, WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        }
            if key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        );
        // modifiers / cursor / winit→iced 转换只有共享实现一份。
        let Some(iced_event) =
            self.gpu
                .track_and_convert(&mut self.cursor, &mut self.modifiers, event)
        else {
            return Vec::new();
        };
        let Some(state) = app.settings.as_ref() else {
            return Vec::new();
        };
        let content = backdrop_card(
            settings::settings_card(state, app.locale.active_locale()).map(Message::Settings),
            card_logical_size(&self.window),
        );
        let (mut messages, status) =
            self.gpu
                .dispatch_with_status(&self.window, self.cursor, content, iced_event);
        if escape
            && !was_composing
            && status == iced_winit::core::event::Status::Ignored
            && messages.is_empty()
        {
            let action = settings::escape_message(state).unwrap_or(settings::Message::Close);
            messages.push(Message::Settings(action));
        }
        messages
    }
}
