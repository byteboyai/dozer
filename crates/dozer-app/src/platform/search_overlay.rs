//! search_modal 的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-17-search-modal-overlay-window-design.md`、
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`(共享机制抽取)。`SearchOverlay` 挂在主窗口 `Runner::Ready`
//! 上,不是独立事件循环——winit 原生按 `WindowId` 把多扇窗口的事件分发
//! 进同一个 `ApplicationHandler`,这扇窗口只是 `window_event` 顶部多出的
//! 一个分支。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::search;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// overlay 卡片的固定逻辑高度,对应现状 `search_modal` 的
/// `max_height(640.0)`。宽度不固定,随主窗口宽度变化(见 `card_logical_size`
/// 调用点,复用 `crate::dialog::width` 的"整窗 1/3"口径)。
const CARD_HEIGHT: f32 = 640.0;

/// 卡片逻辑尺寸:宽度与 `dialog::width(window_width)`(`Length::Fixed(
/// window_width / 3.0)`)保持一致,高度固定 `CARD_HEIGHT`。这里直接算成
/// `f32`,因为 `LogicalSize::new` 要数值而 `dialog::width` 返回 `Length`。
fn card_logical_size(window_width: f32) -> LogicalSize<f32> {
    LogicalSize::new(window_width / 3.0, CARD_HEIGHT)
}

/// `sync_search_overlay` 要不要开/关 overlay 的纯判定,跟真正建/毁窗口的
/// 副作用(`SearchOverlay::open`/`Drop`)分开,方便单测穷举四种组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Open,
    Close,
    Noop,
}

pub(crate) fn sync_action(search_open: bool, overlay_present: bool) -> SyncAction {
    match (search_open, overlay_present) {
        (true, false) => SyncAction::Open,
        (false, true) => SyncAction::Close,
        (true, true) | (false, false) => SyncAction::Noop,
    }
}

/// 独立原生窗口宿主——`search_card()` 的独立渲染管线。`gpu`/`focus` 组合
/// 了 `overlay_gpu`/`overlay_focus` 两个共享件(见
/// `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
/// design.md`「架构」第 1 节的取舍:不做一个泛型大一统类型包办一切)。
pub(crate) struct SearchOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// 这扇窗口里最近一次右键按下的逻辑坐标——`TextInputMenuOpen` 的原生
    /// 菜单定位复用 `app.files.last_right_click`(全应用共享的坐标缓存,
    /// 不是 Files 专属),在这扇窗口里右键时必须先把它覆写成这扇窗口自己
    /// 的坐标,否则菜单会弹在主窗口上次右键的旧位置(见 `handle_input`)。
    last_right_click: (f32, f32),
    /// 主窗口句柄——`open()` 时把 `chrome::native_menu` 的挂靠目标临时
    /// 指向这扇窗口自己的 NSView,`Drop` 时得指回来,不然这扇窗口关掉后
    /// Files/Project 等主窗口里其它输入框的右键菜单会继续错误地尝试挂在
    /// 一个已经销毁的 NSView 上。
    main_window: Arc<Window>,
}

impl SearchOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    /// 供 `sync_search_overlay` 在"这次分发的消息可能只是改了 `ws.search`
    /// 内容"时补一次重绘(见调用点注释)。
    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 开一扇挂成主窗口子窗口的独立窗口,复用主窗口的 `Device`/`Queue`/
    /// `Adapter`/`Instance`,只为这扇窗口单独建 `Surface`/`Engine`/
    /// `Renderer`(spike `spike/multi-window-overlay-wry` 已验证这条路径
    /// 可行:见 `docs/superpowers/specs/2026-09-17-multi-window-overlay-
    /// spike-findings.md`)。
    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        window_width: f32,
        el: &ActiveEventLoop,
    ) -> SearchOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size(window_width);
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "search", el);
        // CJK 组字要靠这个才能拿到候选窗——主窗口在 `resumed()` 里也调了
        // 同一个方法(`window_events.rs:1497`),winit 对新窗口默认关闭 IME,
        // 这扇窗口是独立创建的,不会继承主窗口那次调用的效果。
        window.set_ime_allowed(true);
        // 原生右键菜单(`chrome::native_menu`)靠一个进程级 thread_local
        // 记"该往哪个 NSView 上弹",只在主窗口初始化时装过一次——这扇窗口
        // 打开期间,查询框的剪切/复制/粘贴菜单也要经这条路径,得先把挂靠
        // 目标指过来,`Drop` 里再指回主窗口(见该字段与 `impl Drop` 的文档)。
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);

        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        SearchOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            last_right_click: (0.0, 0.0),
            main_window: main_window.clone(),
        }
    }

    /// 记录焦点事件,返回"是否该因失焦而关闭"。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    /// 主窗口 resize 后重新居中 + 重配置 surface(`with_parent_window` 的
    /// `addChildWindow` 语义只让位置跟随移动,不管尺寸/布局联动)。
    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        window_width: f32,
    ) {
        let card_logical = card_logical_size(window_width);
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    /// 每帧渲染:走标准 `UserInterface::build → draw → present`。打开后
    /// 第一帧顺带消费"查询框待自动聚焦"一次性位(复用
    /// `extensions::search::open()` 早就在设的那个标记)。
    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace_mut() else {
            return;
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());
        // 一次性聚焦位要在 `UserInterface` 建立之前取走:接口借住 `ws`
        // (视图树借 `&ws.search`),期间不能再 `&mut ws`。
        let focus_pending = ws.take_query_focus_pending();

        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );

        if focus_pending {
            let mut op = iced_winit::core::widget::operation::focusable::focus::<()>(
                search::query_field_id(),
            );
            crate::runtime::run_operate(&mut interface, &mut self.gpu.renderer, &mut op);
        }
        crate::runtime::run_operate(
            &mut interface,
            &mut self.gpu.renderer,
            &mut search::CaptureQueryFocus,
        );
        let query_focused = search::take_query_focused();

        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        // `into_cache` 消费掉 `interface`,释放它对 `ws` 的借用,下面才能再
        // 可变借用 `ws` 写回焦点态。
        self.gpu.cache = interface.into_cache();
        ws.search.set_query_focused(query_focused);

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    /// 喂一个原始 winit 事件进这扇窗口自己的 iced 管线。Esc 在这里本地
    /// 处理直接产出 `SearchClose`,不需要主窗口那套"抢在终端转发前特殊
    /// 处理"的手法——这扇窗口里没有终端要竞争按键。逐事件即时重建一次
    /// `UserInterface` 而不是像主窗口那样攒一批再统一处理:这棵视图树
    /// 很小(一个对话框),重建成本可忽略,换来的是不用再维护一份独立的
    /// 事件缓冲/两阶段处理逻辑。
    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Search(search::Message::SearchClose)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        // 查询框右键(剪切/复制/粘贴菜单,`extensions::search::Message::
        // TextInputMenuOpen`)复用 `app.files.last_right_click` 这个全应用
        // 共享的坐标缓存定位原生菜单——不先在这里覆写,菜单会弹在主窗口
        // 上一次右键的旧位置(那个字段只有主窗口自己的右键处理会写)。
        if let WindowEvent::MouseInput {
            state: winit::event::ElementState::Pressed,
            button: winit::event::MouseButton::Right,
            ..
        } = event
            && let mouse::Cursor::Available(point) = self.cursor
        {
            self.last_right_click = (point.x, point.y);
            app.files.last_right_click = self.last_right_click;
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace_mut() else {
            return Vec::new();
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());
        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }

    /// 原生右键菜单(查询框的剪切/复制/粘贴/全选)选中一项后,`App::update`
    /// 已经同步弹完 `NSMenu`、把待合成的按键写进
    /// `app.pending_native_menu_edit_key`——主窗口那份等价收尾逻辑
    /// (`window_events.rs` 尾部)建的是 `app.view()`,查询框已经不在那棵
    /// 树里,够不着,所以这扇窗口自己的事件分支里要单独收一遍。只在目标
    /// 确实是查询框时才 `take()`:这个字段是全应用共享的单槽位,`take()`
    /// 会连同"根本不是查询框"的情形一起清空,那样会偷走本该留给主窗口
    /// 其它输入框的合成按键。调用点:`window_event` 的 overlay 分支,
    /// 在 `handle_input` 派发完消息之后、早退之前。
    pub(crate) fn apply_pending_native_menu_edit_key(&mut self, app: &mut App) {
        let is_query_field = app
            .pending_native_menu_edit_key
            .as_ref()
            .is_some_and(|(_, id)| *id == search::query_field_id());
        if !is_query_field {
            return;
        }
        let Some((ch, target_id)) = app.take_pending_native_menu_edit_key() else {
            return;
        };
        let Some(ws) = app.active_workspace_mut() else {
            return;
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());
        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut op = iced_winit::core::widget::operation::focusable::focus::<()>(target_id);
        crate::runtime::run_operate(&mut interface, &mut self.gpu.renderer, &mut op);
        let synth = [crate::event::unique_command_event(ch)];
        // 剪切/复制/粘贴/全选作用于纯文本编辑,不会产出需要二次处理的业务
        // 消息(跟主窗口同款收尾逻辑一样直接丢弃产出);置信度来自两边走的
        // 是同一个查询框 `text_input`,行为不会因为宿主窗口不同而分叉。
        let mut ignored = Vec::new();
        let _ = interface.update(
            &synth,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut ignored,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
    }
}

impl Drop for SearchOverlay {
    /// 把 `chrome::native_menu` 的挂靠目标指回主窗口——`open()` 打开时
    /// 临时指到了这扇窗口自己的 NSView(见那里的调用点注释),这扇窗口
    /// 消失后,Files/Project 等主窗口里其它输入框的右键菜单还得继续正常
    /// 弹在主窗口上。
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_opens_when_search_open_and_no_overlay() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
    }

    #[test]
    fn sync_action_closes_when_search_closed_but_overlay_present() {
        assert_eq!(sync_action(false, true), SyncAction::Close);
    }

    #[test]
    fn sync_action_noop_when_states_already_match() {
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }
}
