//! file_history 弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`。结构对照 `search_overlay.rs::SearchOverlay`——没有文本
//! 输入,所以没有 IME/原生右键菜单挂靠、没有查询框自动聚焦这些机制,
//! 比 `SearchOverlay` 更简单。

use std::path::PathBuf;
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
use crate::extensions::file_history;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{
    backdrop_card, centered_card_offset, full_window_overlay_bounds, open_child_window,
};

/// 卡片逻辑尺寸:宽 = 主窗口宽度的 75%,高 = 主窗口高度的 80%——同现状
/// `file_history::popup_view` 的比例。
fn card_logical_size(window_width: f32, window_height: f32) -> LogicalSize<f32> {
    LogicalSize::new(window_width * 0.75, window_height * 0.8)
}

/// diff 区域(`file_history::diff_area_view` 的 `content` 子树)在弹窗卡片
/// **自身逻辑坐标系**里的矩形——**2026-09-27 背景遮罩改造后卡片不再是
/// 这扇窗口的整个客户区**(窗口现覆盖整个主窗口,卡片由 `backdrop_card`
/// 居中画在中间),调用方必须再加上 `centered_card_offset` 算出的卡片
/// 偏移,才是这扇窗口客户区坐标系里 `view.set_bounds` 要的矩形(见
/// `sync_diff_webview` 调用点)。跟 `file_history_card` 的实际布局逐项对应:外层
/// `padding(16)`;`title` 一行(`font::subtitle()`,`spacing(12)` 在其后);
/// `body` 是 `row![list(固定 240 宽), spacing(12), diff_area]`;
/// `diff_area_view` 内部是 `column![header, content].spacing(8)`,`header`
/// 一行(`font::label()`)。跟 `git_log` 那份计划的 diff pane 几何算法同一种
/// "近似值,人工验收阶段微调"精度承诺——`rollback_error` 有值时会在
/// `content` 之后再压一行文案,那种情况下这个矩形会比实际渲染区域略高,
/// 已知的已接受偏差,不在这个函数里处理。
fn diff_area_bounds(card_logical: LogicalSize<f32>) -> (f32, f32, f32, f32) {
    let pad = 16.0;
    let title_h = byteui::theme::font::subtitle() as f32 * 1.2;
    let header_h = byteui::theme::font::label() as f32 * 1.2;
    let list_w = 240.0;
    let row_spacing = 12.0;
    let col_spacing = 8.0;

    let x = pad + list_w + row_spacing;
    let y = pad + title_h + row_spacing + header_h + col_spacing;
    let w = (card_logical.width - x - pad).max(0.0);
    let h = (card_logical.height - y - pad).max(0.0);
    (x, y, w, h)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Open,
    Close,
    Noop,
}

pub(crate) fn sync_action(open: bool, overlay_present: bool) -> SyncAction {
    match (open, overlay_present) {
        (true, false) => SyncAction::Open,
        (false, true) => SyncAction::Close,
        (true, true) | (false, false) => SyncAction::Noop,
    }
}

pub(crate) struct FileHistoryOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// diff webview 单槽位(不是池——这个弹窗任意时刻最多展示一个 diff)。
    /// `String` 是当前已加载的 URL(导航去重,同主窗口 webview 池的既有
    /// 手法)。`None` = 未挂载(未选中版本 / 内容不可渲染 / 尚未加载完)。
    diff_webview: Option<(wry::WebView, String)>,
    /// `diff_webview` 最近一次 `set_bounds`/创建时用的矩形(卡片逻辑坐标
    /// 系里的 x/y/w/h)。`sync_diff_webview` 每帧都可能被调用(键入、IPC
    /// 回包、redraw 请求都会触发),矩形没变时跳过原生 `set_bounds` 调用,
    /// 不在没必要的时候反复触发原生窗口尺寸/位置更新。
    diff_webview_bounds: Option<(f32, f32, f32, f32)>,
}

impl FileHistoryOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> FileHistoryOverlay {
        let scale = main_window.scale_factor();
        let (pos, size) = full_window_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
        );
        let window = open_child_window(main_window, pos, size, "file-history", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        FileHistoryOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            diff_webview: None,
            diff_webview_bounds: None,
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
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
        let Some(state) = app.file_history.as_ref() else {
            return;
        };
        let logical_size: LogicalSize<f32> = self
            .window
            .inner_size()
            .to_logical(self.window.scale_factor());
        let card_logical = card_logical_size(logical_size.width, logical_size.height);
        let mut interface = UserInterface::build(
            backdrop_card(
                file_history::file_history_card(state).map(Message::FileHistory),
                card_logical,
            ),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
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
        self.gpu.cache = interface.into_cache();

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

    /// 每帧调用:按 `app.file_history` 当前状态决定 diff webview 的存在/
    /// URL/矩形,并在 webview 已确认 ready 且有未送达内容时推一次
    /// `SetDiffDocument`。不复用主窗口 `sync_webview_pool`(见本计划顶部
    /// "Architecture"——那套的 IPC 路由按 `binding.panel` 分支,且池 key
    /// 空间是主窗口专属的,生搬到这扇独立窗口上要么错路由要么要新增
    /// `PanelKind` 变体,两者都不值当,这个槽位本来就只服务一个 webview)。
    pub(crate) fn sync_diff_webview(
        &mut self,
        app: &mut App,
        allowed_files: Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
        proxy: winit::event_loop::EventLoopProxy<Message>,
    ) {
        let desired = app.file_history.as_ref().and_then(|s| {
            let loaded = s.loaded_diff()?;
            let crate::extensions::git_log::DiffBlobContent::Text { .. } = &loaded.content else {
                return None;
            };
            let target = s.target()?;
            Some((target.file_path.clone(), loaded.oid))
        });

        let Some((file_path, _oid)) = desired else {
            self.diff_webview = None;
            self.diff_webview_bounds = None;
            return;
        };

        let binding =
            crate::preview::EditorHostBinding::new(0, crate::app::PanelKind::Files, 0, file_path);
        let url = binding.diff_url(crate::preview::scheme_query_value());
        let logical_size: LogicalSize<f32> = self
            .window
            .inner_size()
            .to_logical(self.window.scale_factor());
        let card_logical = card_logical_size(logical_size.width, logical_size.height);
        let card_offset = centered_card_offset(logical_size, card_logical);
        let (x, y, w, h) = diff_area_bounds(card_logical);
        let (x, y) = (x + card_offset.x, y + card_offset.y);
        let bounds = wry::Rect {
            position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
            size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
        };

        match &mut self.diff_webview {
            Some((view, loaded_url)) => {
                if *loaded_url != url {
                    let _ = view.load_url(&url);
                    *loaded_url = url;
                }
                let current_bounds = (x, y, w, h);
                if self.diff_webview_bounds != Some(current_bounds) {
                    let _ = view.set_bounds(bounds);
                    self.diff_webview_bounds = Some(current_bounds);
                }
            }
            None => {
                let root = crate::assets::assets_root();
                let ipc_proxy = proxy;
                let expected_binding = binding.clone();
                let built = wry::WebViewBuilder::new()
                    .with_url(&url)
                    .with_bounds(bounds)
                    .with_visible(true)
                    .with_custom_protocol("dozer".into(), move |_id, request| {
                        let allowed = allowed_files.lock().expect("allowed_files 锁");
                        let reply = crate::assets::handle_protocol(
                            &root,
                            &allowed,
                            None,
                            &request.uri().to_string(),
                        );
                        wry::http::Response::builder()
                            .status(reply.status)
                            .header("Content-Type", reply.mime)
                            .body(std::borrow::Cow::Owned(reply.body))
                            .unwrap()
                    })
                    .with_ipc_handler(move |req| {
                        let body = req.body().as_str();
                        let expected = crate::preview::HostBinding::new(
                            expected_binding.project_id,
                            expected_binding.panel,
                            expected_binding.tab_id,
                            expected_binding.document_id(),
                        );
                        match crate::preview::parse_event(body) {
                            Ok(event) => {
                                if let Err(error) = event.validate(&expected) {
                                    tracing::warn!(%error, "拒绝无效 file-history diff IPC");
                                } else {
                                    let _ = ipc_proxy.send_event(
                                        Message::FileHistoryDiffWebviewEvent(expected, event),
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, "无法解析 file-history diff IPC");
                            }
                        }
                    })
                    .build_as_child(&self.window);
                match built {
                    Ok(view) => {
                        self.diff_webview = Some((view, url));
                        self.diff_webview_bounds = Some((x, y, w, h));
                    }
                    Err(e) => tracing::warn!("文件历史 diff webview 创建失败: {e}"),
                }
            }
        }

        // 内容推送:webview 已 ready 且当前内容还没送达才推。包一层闭包,让
        // 内部的早退 `return None` 只跳出这段计算,不会意外跳出整个
        // `sync_diff_webview`(万一以后有人在这段之后追加清理/日志代码)。
        let push: Option<(git2::Oid, String)> = (|| {
            let s = app.file_history.as_ref()?;
            if !s.diff_webview_ready() {
                return None;
            }
            let loaded = s.loaded_diff()?;
            if s.diff_sent_for() == Some(loaded.oid) {
                return None;
            }
            let crate::extensions::git_log::DiffBlobContent::Text { old_text, new_text } =
                &loaded.content
            else {
                return None;
            };
            let language = crate::preview::extension_to_syntax(std::path::Path::new(&binding.path));
            let cmd = crate::preview::EditorCommand::SetDiffDocument {
                old_text: old_text.clone(),
                new_text: new_text.clone(),
                language: language.to_string(),
                revision: 0,
                read_only: true,
            };
            let script = crate::preview::dispatch_script(&crate::preview::encode_command(
                binding.project_id,
                binding.panel,
                binding.tab_id,
                &binding.document_id(),
                0,
                None,
                cmd,
            ));
            Some((loaded.oid, script))
        })();
        if let Some((oid, script)) = push
            && let Some((view, _)) = &self.diff_webview
        {
            let _ = view.evaluate_script(&script);
            if let Some(s) = app.file_history.as_mut() {
                s.set_diff_sent_for(oid);
            }
        }
    }

    /// 喂一个原始 winit 事件进这扇窗口自己的 iced 管线。同
    /// `SearchOverlay::handle_input`,逐事件即时重建 `UserInterface`(这棵
    /// 视图树重建成本可忽略,换取不用维护独立的事件缓冲)。
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
            return vec![Message::FileHistory(file_history::Message::Close)];
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
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(state) = app.file_history.as_ref() else {
            return Vec::new();
        };
        let logical_size: LogicalSize<f32> = self
            .window
            .inner_size()
            .to_logical(self.window.scale_factor());
        let card_logical = card_logical_size(logical_size.width, logical_size.height);
        let mut interface = UserInterface::build(
            backdrop_card(
                file_history::file_history_card(state).map(Message::FileHistory),
                card_logical,
            ),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_opens_when_open_and_no_overlay() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
    }

    #[test]
    fn sync_action_closes_when_closed_but_overlay_present() {
        assert_eq!(sync_action(false, true), SyncAction::Close);
    }

    #[test]
    fn sync_action_noop_when_states_already_match() {
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }
}
