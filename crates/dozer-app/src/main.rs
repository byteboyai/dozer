mod assets;
mod chrome_style;
mod conversation;
mod delivery;
mod fonts;
mod goal;
mod icons;
mod keymap;
mod layout;
mod osc;
mod preview;
mod preview_state;
mod project;
mod term_model;
mod term_view;
mod theme;
mod transcript;
mod workspace;

use workspace::{Message, Workspace};

use std::process::{Command, Stdio};
use std::time::Duration;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::core::time::Instant;
use iced_winit::core::window;
use iced_winit::core::{Event, Font, Pixels, Size, Theme};
use iced_winit::futures;
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::winit;

use winit::{
    dpi::LogicalSize,
    event::{ElementState, Ime, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::ModifiersState,
};

use std::sync::Arc;

/// tab 状态点闪烁的半周期：每 450ms 翻一次相位（≈1.1Hz 一明一暗）。
/// 仅当有 tab 处于工作态时才据此定时唤醒，空闲仍是 `ControlFlow::Wait`。
const BLINK_INTERVAL: Duration = Duration::from_millis(450);

/// 清空一帧到给定背景色，不再绘制 spike 阶段的示例三角形
/// （spike B 的 `scene.rs`/wgsl shader 已随本任务删除）。
fn clear<'a>(
    target: &'a wgpu::TextureView,
    encoder: &'a mut wgpu::CommandEncoder,
    background_color: iced_winit::core::Color,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear({
                    let [r, g, b, a] = background_color.into_linear();

                    wgpu::Color {
                        r: r as f64,
                        g: g as f64,
                        b: b as f64,
                        a: a as f64,
                    }
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

/// daemon 连不上时的自动拉起：优先用 `current_exe` 同目录下的 `dozerd`
/// 二进制（cargo workspace 构建后与 `dozer` 落在同一个 target 目录），
/// 找不到就退化到 PATH 查找（`Command::new("dozerd")` 交给 shell/PATH 解析）。
fn spawn_dozerd() {
    let same_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("dozerd")));

    let mut command = match same_dir {
        Some(path) if path.exists() => Command::new(path),
        _ => Command::new("dozerd"),
    };

    match command.stdin(Stdio::null()).spawn() {
        Ok(_child) => tracing::info!("已自动拉起 dozerd"),
        Err(e) => tracing::error!("自动拉起 dozerd 失败: {e}"),
    }
}

/// 启动序列第一步：探测 daemon 是否可用（一次 `list()` 往返）。连不上
/// 就自动 spawn `dozerd`，隔 1 秒重试，最多 3 次；全部失败则返回错误
/// 文案，交给调用方决定如何展示（不阻塞窗口创建本身）。
async fn ensure_daemon(client: &dozer_client::Client) -> Result<(), String> {
    if client.list().await.is_ok() {
        return Ok(());
    }

    tracing::warn!("daemon 未响应，尝试自动拉起 dozerd");
    spawn_dozerd();

    let mut last_err = String::from("daemon 未响应");
    for attempt in 1..=3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        match client.list().await {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                tracing::warn!(attempt, "重试连接 dozerd 仍失败: {last_err}");
            }
        }
    }
    Err(format!("无法连接 dozerd（已重试 3 次）：{last_err}"))
}

/// 启动序列：连 daemon（失败则 spawn dozerd 重试）→ 成功则做 GUI 级
/// 会话恢复（`Workspace::bootstrap`），失败则降级为错误态
/// （`Workspace::with_daemon_error`）。整段在 `main()` 用
/// `runtime.block_on` 驱动，此时窗口尚未创建，不占用任何"正在跑的"
/// UI 线程。
async fn build_workspace(
    client: dozer_client::Client,
    handle: tokio::runtime::Handle,
    proxy: winit::event_loop::EventLoopProxy<Message>,
) -> Workspace {
    match ensure_daemon(&client).await {
        Ok(()) => Workspace::bootstrap(client, handle, proxy).await,
        Err(message) => Workspace::with_daemon_error(client, handle, proxy, message),
    }
}

pub fn main() -> Result<(), winit::error::EventLoopError> {
    tracing_subscriber::fmt::init();

    // 第一次文本排版之前剔除毒化 CJK 回退的位图字体（见 fonts.rs 模块注释）。
    fonts::sanitize_font_db();

    // Initialize winit：用户事件类型直接是 `Message`——tokio 任务经
    // `EventLoopProxy<Message>::send_event` 把事件流/daemon 状态送回 UI
    // 线程，`ApplicationHandler::user_event` 收到后转发给
    // `workspace.update`。
    let event_loop = EventLoop::<Message>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    // tokio Runtime 由 main 持有，跟 winit 事件循环共存一整个进程生命
    // 周期（`event_loop.run_app` 之后才会 drop）。UI 线程只允许用
    // `handle.spawn` 派发任务，绝不 `block_on` 网络 IO——启动序列的
    // 一次性 `block_on` 是唯一例外（窗口还没创建，谈不上"占用 UI 线程"）。
    let runtime = tokio::runtime::Runtime::new().expect("创建 tokio runtime");
    let handle = runtime.handle().clone();
    let client = dozer_client::Client::new(dozer_core::paths::socket_path());

    let workspace = runtime.block_on(build_workspace(client, handle, proxy.clone()));

    #[allow(clippy::large_enum_variant)]
    enum Runner {
        /// 持有启动序列已经构建好的 `Workspace`（daemon 已连上/已降级为
        /// 错误态，视情况可能已经装好若干恢复出来的 tab）；`resumed()`
        /// 建好窗口/wgpu 后把它 `take()` 出来转入 `Ready`。用
        /// `Option` 包一层只是为了能在 `&mut self` 上 `take`，正常情况下
        /// `resumed()` 只会被调用一次。
        Loading(Option<Workspace>),
        Ready {
            window: Arc<winit::window::Window>,
            queue: wgpu::Queue,
            device: wgpu::Device,
            surface: wgpu::Surface<'static>,
            format: wgpu::TextureFormat,
            renderer: Renderer,
            workspace: Workspace,
            events: Vec<Event>,
            cursor: mouse::Cursor,
            cache: user_interface::Cache,
            clipboard: Clipboard,
            viewport: Viewport,
            modifiers: ModifiersState,
            resized: bool,
            /// 预览 webview 池:tab id → (句柄, 当前已加载 URL)。句柄只在
            /// 本事件环存取(spike 约束 2);URL 缓存用于导航去重。
            webviews: std::collections::HashMap<usize, (wry::WebView, String)>,
            /// 浏览器 webview 池,语义/生命周期同 `webviews`,但服务独立的
            /// "地球图标"浏览器视图——两个池各自独立增删,tab id 空间即使
            /// 撞了也不会互相覆盖(不同 HashMap)。
            browser_webviews: std::collections::HashMap<usize, (wry::WebView, String)>,
            /// 最近一次光标物理位置(CursorMoved 更新),鼠标点击时用于命中测试。
            cursor_phys: winit::dpi::PhysicalPosition<f64>,
            /// 待应用的焦点意图(点击/消息设置,sync_previews 之后统一 apply,
            /// 确保新建 webview 已入池)。
            pending_focus: Option<FocusIntent>,
        },
    }

    /// 点击/消息后决定键盘焦点归谁:预览 webview、浏览器 webview(各自
    /// ⌘C 走原生复制)或窗口(终端)。
    #[derive(Clone, Copy)]
    enum FocusIntent {
        Preview,
        Browser,
        Terminal,
    }

    /// `sync_previews` 的差集同步逻辑,预览池/浏览器池共用同一套算法,
    /// 各自传各自的 `pool`/`specs`,互不干扰。
    fn sync_webview_pool(
        window: &winit::window::Window,
        pool: &mut std::collections::HashMap<usize, (wry::WebView, String)>,
        specs: Vec<preview::WebviewSpec>,
        bounds: wry::Rect,
        allowed_files: std::sync::Arc<
            std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
        >,
    ) {
        let desired_ids: std::collections::HashSet<usize> = specs.iter().map(|s| s.id).collect();
        pool.retain(|id, _| desired_ids.contains(id));

        for spec in specs {
            match pool.get_mut(&spec.id) {
                Some((view, loaded_url)) => {
                    if *loaded_url != spec.url {
                        if let Err(e) = view.load_url(&spec.url) {
                            tracing::warn!("预览导航失败: {e}");
                        }
                        *loaded_url = spec.url.clone();
                    }
                    let _ = view.set_bounds(bounds);
                    let _ = view.set_visible(spec.visible);
                }
                None => {
                    let allowed = std::sync::Arc::clone(&allowed_files);
                    let root = assets::assets_root();
                    let built = wry::WebViewBuilder::new()
                        .with_url(&spec.url)
                        .with_bounds(bounds)
                        .with_visible(spec.visible)
                        .with_custom_protocol("dozer".into(), move |_id, request| {
                            let allowed = allowed.lock().expect("allowed_files 锁");
                            let reply = assets::handle_protocol(
                                &root,
                                &allowed,
                                &request.uri().to_string(),
                            );
                            wry::http::Response::builder()
                                .status(reply.status)
                                .header("Content-Type", reply.mime)
                                .body(std::borrow::Cow::Owned(reply.body))
                                .expect("构造协议应答")
                        })
                        .build_as_child(window);
                    match built {
                        Ok(view) => {
                            pool.insert(spec.id, (view, spec.url.clone()));
                        }
                        Err(e) => tracing::error!("创建预览 webview 失败: {e}"),
                    }
                }
            }
        }
    }

    impl Runner {
        /// 键盘/IME 输入拦截处：终端聚焦时把原始 `WindowEvent` 经
        /// `keymap` 翻译成字节，直接回灌 `Workspace`（`Message::TermInput`），
        /// 不必改动 `winit::application::ApplicationHandler` 的实现本身。
        ///
        /// `Workspace::update` 收到 `TermInput` 后写给 daemon（`client.write`），
        /// 不再本地 echo——回显完全走 PTY 真实回路（daemon → attach 流 →
        /// `Message::TermOutput` → `TerminalModel::feed`）。
        fn on_window_event(&mut self, event: &WindowEvent) {
            // 把 `modifiers` 和 `workspace`/`window` 放进同一次解构里取，
            // 避免先借一次 `self` 再调用 `&self` 方法造成的重复借用。
            let Self::Ready {
                workspace,
                window,
                modifiers,
                clipboard,
                cursor_phys,
                pending_focus,
                ..
            } = self
            else {
                return;
            };

            // 光标位置跟踪 + 点击焦点路由（验收反馈 2/失焦回正常态）:
            // 任一左键点击先退出所有自绘输入编辑态(点回输入框会被 iced
            // 随后的 AddrClick/CommentClick 重新进入);再按落点决定键盘归谁。
            match event {
                WindowEvent::CursorMoved { position, .. } => {
                    *cursor_phys = *position;
                    if workspace.dragging_divider().is_some() {
                        let scale = window.scale_factor();
                        let logical_x = (cursor_phys.x / scale) as f32;
                        let window_width = (window.inner_size().width as f64 / scale) as f32;
                        workspace.update(Message::ColumnDrag {
                            window_width,
                            logical_x,
                        });
                    }
                    // 悬停(未拖拽)也要请求重绘:分隔线的 resize 光标走
                    // MouseArea::interaction → mouse_interaction() → RedrawRequested
                    // 里的 window.set_cursor(icon) 这条既有管线(main.rs:808-816
                    // 一带),只在真的重绘发生时才会重算光标——不主动重绘的话,
                    // 悬停不会变光标,得等下一次因别的原因触发的重绘(比如点击)
                    // 才会"追上",观感上就是"划过没反应、点一下才变"。
                    window.request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button,
                    ..
                } if *button == winit::event::MouseButton::Right
                    || (*button == winit::event::MouseButton::Left && modifiers.control_key()) =>
                {
                    // 右键、或 Control+左键——macOS 系统级"次级点击"约定
                    // (RustRover 等原生 app 都认这个),但 winit 的 macOS 后端
                    // 不会自动做这个转换(已核实其 mouseDown:/rightMouseDown:
                    // 直接按 AppKit 实际调用的 responder 方法映射按钮,不看
                    // 修饰键),这里手动补上,两者统一走同一支。
                    let scale = window.scale_factor();
                    let x = (cursor_phys.x / scale) as f32;
                    let y = (cursor_phys.y / scale) as f32;
                    // 菜单是手算像素定位、不自带边界检测的浮层——右键点在
                    // 窗口下/右 250px 内时,原样使用点击坐标会把菜单下沿/
                    // 右沿画出窗口外,底部几项(删除/重命名等)点不到。钳制
                    // 到"窗口尺寸 - 菜单最坏尺寸"内(Important #7)。
                    let logical_size = window.inner_size();
                    let window_w = (logical_size.width as f64 / scale) as f32;
                    let window_h = (logical_size.height as f64 / scale) as f32;
                    let x = x.min((window_w - workspace::CONTEXT_MENU_WIDTH).max(0.0));
                    let y = y.min((window_h - workspace::CONTEXT_MENU_HEIGHT).max(0.0));
                    workspace.update(Message::RightClickAt { x, y });
                    // 不在这里 request_redraw——右键若真的命中某行,该行的
                    // `MouseArea::on_right_press` 随本轮事件走 iced 正常分发,
                    // 那条路径自会触发重绘;若点在空白处,菜单本就不该开,不必
                    // 额外重绘。
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: winit::event::MouseButton::Left,
                    ..
                } => {
                    workspace.blur_inputs();
                    let scale = window.scale_factor();
                    let logical_x = (cursor_phys.x / scale) as f32;
                    let logical_w = (window.inner_size().width as f64 / scale) as f32;
                    let state = workspace.shell_state();
                    *pending_focus = Some(
                        if workspace::is_in_preview_column(logical_x, logical_w, &state) {
                            if state.left_view == workspace::LeftView::Web {
                                FocusIntent::Browser
                            } else {
                                FocusIntent::Preview
                            }
                        } else {
                            FocusIntent::Terminal
                        },
                    );
                    window.request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if workspace.dragging_divider().is_some() => {
                    workspace.update(Message::ColumnDragEnd);
                    window.request_redraw();
                }
                _ => {}
            }

            // 右键菜单打开时,Esc 优先关菜单,不进正常键盘分发(不然会被当作
            // 普通按键继续往下走,可能被地址栏/终端等其它分支消费掉)。
            if workspace.context_menu_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                workspace.update(Message::ProjectTreeContextMenuClose);
                window.request_redraw();
                return;
            }

            // ⌘ 组合键是应用级快捷键，一律不进 PTY（此前 ⌘C 会把裸 "c"
            // 漏写进终端）。⌘C 复制当前选区；⌘V 粘贴剪贴板。
            if modifiers.super_key() {
                if let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                    && event.state == ElementState::Pressed
                    && let winit::keyboard::Key::Character(s) = &event.logical_key
                {
                    match s.as_str() {
                        "c" => {
                            if let Some(text) = workspace.active_selection_text() {
                                clipboard.write(iced_winit::core::clipboard::Kind::Standard, text);
                            }
                        }
                        "v" => {
                            if let Some(text) =
                                clipboard.read(iced_winit::core::clipboard::Kind::Standard)
                            {
                                workspace.update(Message::TermPaste(text));
                                window.request_redraw();
                            }
                        }
                        _ => {}
                    }
                }
                return;
            }

            // 浏览器地址栏 / 验收意见 / 项目树行内编辑态:键盘直达自绘输入
            // (不经 keymap、不进 PTY)。文件预览面板已不再有地址栏。
            let to_browser = workspace.browser_addr_editing();
            let to_comment = workspace.acceptance_comment_editing();
            let to_tree_edit = workspace.tree_editing();
            if to_browser || to_comment || to_tree_edit {
                let addr_event = match event {
                    WindowEvent::KeyboardInput {
                        event,
                        is_synthetic: false,
                        ..
                    } if event.state == ElementState::Pressed => {
                        use winit::keyboard::{Key, NamedKey};
                        match &event.logical_key {
                            Key::Character(s) => Some(workspace::AddrEvent::Text(s.to_string())),
                            Key::Named(NamedKey::Space) => {
                                Some(workspace::AddrEvent::Text(" ".into()))
                            }
                            Key::Named(NamedKey::Backspace) => {
                                Some(workspace::AddrEvent::Backspace)
                            }
                            Key::Named(NamedKey::Enter) => Some(workspace::AddrEvent::Submit),
                            Key::Named(NamedKey::Escape) => Some(workspace::AddrEvent::Cancel),
                            _ => None,
                        }
                    }
                    WindowEvent::Ime(Ime::Commit(text)) => {
                        Some(workspace::AddrEvent::Text(text.clone()))
                    }
                    _ => None,
                };
                if let Some(ev) = addr_event {
                    // 优先级:浏览器地址栏 > 验收意见 > 项目树编辑(三者同真时
                    // 罕见,谁先建的编辑态谁优先没有实际冲突场景,这个顺序只是
                    // 一个确定性兜底)。
                    let message = if to_browser {
                        Message::BrowserAddrEvent(ev)
                    } else if to_comment {
                        Message::AcceptanceCommentEvent(ev)
                    } else {
                        Message::ProjectTreeEditEvent(ev)
                    };
                    workspace.update(message);
                    window.request_redraw();
                }
                return;
            }

            let bytes = match event {
                // 只处理真实按键（忽略窗口获得焦点时 winit 补发的
                // synthetic 事件）与按下沿；`modifiers` 由外层
                // `window_event` 在 `ModifiersChanged` 时更新，这里读到的
                // 始终是按键发生时刻的最新状态。
                WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } if event.state == ElementState::Pressed => keymap::key_to_bytes(
                    &event.logical_key,
                    modifiers,
                    workspace.active_app_cursor_mode(),
                ),
                WindowEvent::Ime(Ime::Commit(text)) => Some(keymap::ime_commit_to_bytes(text)),
                // 拖文件进终端：转成 shell 转义的完整路径写入会话
                // （Terminal.app 同款行为）。
                WindowEvent::DroppedFile(path) => {
                    Some(keymap::dropped_path_to_bytes(&path.to_string_lossy()))
                }
                _ => None,
            };

            if let Some(bytes) = bytes {
                workspace.update(Message::TermInput(bytes));
                window.request_redraw();
            }
        }

        /// 把 workspace 的 webview 期望清单同步到真实 wry 子视图:
        /// 建缺失、毁多余、对齐可见性与 bounds、URL 变更时导航。文件预览池
        /// 和浏览器池各自独立同步(`preview_desired`/`browser_desired` 已按
        /// `left_view` 互斥,同一时刻至多一个非空)。
        fn sync_previews(&mut self) {
            let Self::Ready {
                window,
                workspace,
                webviews,
                browser_webviews,
                ..
            } = self
            else {
                return;
            };

            let size = window.inner_size();
            let scale = window.scale_factor();
            let logical_w = size.width as f32 / scale as f32;
            let logical_h = size.height as f32 / scale as f32;
            let (x, y, w, h) =
                workspace::preview_content_bounds(logical_w, logical_h, &workspace.shell_state());
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
            };

            sync_webview_pool(
                window.as_ref(),
                webviews,
                workspace.preview_desired(),
                bounds,
                workspace.allowed_files(),
            );
            sync_webview_pool(
                window.as_ref(),
                browser_webviews,
                workspace.browser_desired(),
                bounds,
                workspace.allowed_files(),
            );
        }

        /// `ProjectPickFolder`/`ProjectTreeCopyPath` 等需要窗口句柄侧原生
        /// 能力(rfd 模态、系统剪贴板)的消息在此拦截,其余原样转给
        /// `workspace.update`。
        fn dispatch(&mut self, message: Message) {
            let Self::Ready {
                workspace,
                window,
                pending_focus,
                clipboard,
                ..
            } = self
            else {
                return;
            };
            // 打开/切到预览 tab → 键盘焦点跟去预览(否则 ⌘C 复制的是终端选区)。
            // 浏览器 tab 同理归浏览器(各自独立的 webview 池,焦点不能混)。
            if matches!(
                message,
                Message::PreviewOpenPath(_) | Message::PreviewSelectTab(_)
            ) {
                *pending_focus = Some(FocusIntent::Preview);
            } else if matches!(
                message,
                Message::BrowserOpenUrl(_) | Message::BrowserSelectTab(_)
            ) {
                *pending_focus = Some(FocusIntent::Browser);
            }
            match message {
                Message::ProjectPickFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        workspace.update(Message::ProjectOpen(dir));
                    }
                }
                Message::ProjectTreeCopyPath(path, kind) => {
                    let root = workspace
                        .active_project_path()
                        .unwrap_or_else(|| path.clone());
                    let s = crate::project::path_string(kind, &path, &root);
                    clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
                    workspace.update(Message::ProjectTreeContextMenuClose);
                    window.request_redraw();
                }
                other => workspace.update(other),
            }
            window.request_redraw();
        }

        /// sync_previews 之后统一应用焦点意图(此时新建 webview 已入池)。
        /// Preview/Browser → 各自当前激活 webview 拿键盘(⌘C 原生复制);
        /// Terminal/无 webview → 交回窗口(终端键盘)。
        fn apply_pending_focus(&mut self) {
            let Self::Ready {
                workspace,
                window,
                webviews,
                browser_webviews,
                pending_focus,
                ..
            } = self
            else {
                return;
            };
            match pending_focus.take() {
                Some(FocusIntent::Preview) => match workspace.active_preview_webview_id() {
                    Some(id) => {
                        if let Some((view, _)) = webviews.get(&id) {
                            let _ = view.focus(); // 返回 Result,忽略
                        } else {
                            window.focus_window();
                        }
                    }
                    None => window.focus_window(),
                },
                Some(FocusIntent::Browser) => match workspace.active_browser_webview_id() {
                    Some(id) => {
                        if let Some((view, _)) = browser_webviews.get(&id) {
                            let _ = view.focus();
                        } else {
                            window.focus_window();
                        }
                    }
                    None => window.focus_window(),
                },
                Some(FocusIntent::Terminal) => window.focus_window(),
                None => {}
            }
        }
    }

    impl winit::application::ApplicationHandler<Message> for Runner {
        /// 闪烁定时器到点（`ControlFlow::WaitUntil` 触发的
        /// `ResumeTimeReached`）：翻转全局闪烁相位并请求重绘。相位是全局
        /// 的，所有工作态 tab（含失焦的）在同一帧一起明灭。
        fn new_events(
            &mut self,
            _event_loop: &winit::event_loop::ActiveEventLoop,
            cause: winit::event::StartCause,
        ) {
            if let winit::event::StartCause::ResumeTimeReached { .. } = cause
                && let Self::Ready {
                    workspace, window, ..
                } = self
            {
                workspace.toggle_blink();
                window.request_redraw();
            }
        }

        /// 每轮事件处理完后决定下次唤醒时机：有 tab 在工作就排下一拍闪烁
        /// 唤醒，否则回到 `Wait` 省电（不再空转重绘）。
        fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Ready { workspace, .. } = self {
                if workspace.any_blinking() {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        std::time::Instant::now() + BLINK_INTERVAL,
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
        }

        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Loading(pending_workspace) = self {
                let Some(mut workspace) = pending_workspace.take() else {
                    // `resumed()` 理论上只会真正建窗口这一次；后续（若平台
                    // 又调用一次 `resumed`）直接跳过，避免重复建窗口。
                    return;
                };
                let window = Arc::new(
                    event_loop
                        .create_window(
                            winit::window::WindowAttributes::default()
                                .with_title("Dozer")
                                .with_inner_size(LogicalSize::new(
                                    workspace::INITIAL_WINDOW_SIZE.0,
                                    workspace::INITIAL_WINDOW_SIZE.1,
                                ))
                                // 双保险:窗口不许缩到"两个面板区都放不下最小宽"
                                // 以下。真正保证右半边不消失的是 workspace 侧的
                                // `clamp_left_width`(持久化宽可能远大于这个最小
                                // 宽),这里只是把最坏情形挡在外面。
                                .with_min_inner_size(LogicalSize::new(
                                    workspace::MIN_WINDOW_WIDTH,
                                    workspace::MIN_WINDOW_HEIGHT,
                                )),
                        )
                        .expect("Create window"),
                );
                // 打开 IME：CJK 等组合输入法要靠 `WindowEvent::Ime(Commit)`
                // 才能拿到最终提交文本（默认关闭，见 winit 文档）。
                window.set_ime_allowed(true);

                let physical_size = window.inner_size();
                let viewport = Viewport::with_physical_size(
                    Size::new(physical_size.width, physical_size.height),
                    window.scale_factor() as f32,
                );
                let clipboard = Clipboard::connect(window.clone());

                let backend = wgpu::Backends::from_env().unwrap_or_default();

                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: backend,
                    ..Default::default()
                });
                let surface = instance
                    .create_surface(window.clone())
                    .expect("Create window surface");

                let (format, adapter, device, queue) =
                    futures::futures::executor::block_on(async {
                        let adapter = wgpu::util::initialize_adapter_from_env_or_default(
                            &instance,
                            Some(&surface),
                        )
                        .await
                        .expect("Create adapter");

                        let adapter_features = adapter.features();

                        let capabilities = surface.get_capabilities(&adapter);

                        let (device, queue) = adapter
                            .request_device(&wgpu::DeviceDescriptor {
                                label: None,
                                required_features: adapter_features & wgpu::Features::default(),
                                required_limits: wgpu::Limits::default(),
                                memory_hints: wgpu::MemoryHints::MemoryUsage,
                                trace: wgpu::Trace::Off,
                                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                            })
                            .await
                            .expect("Request device");

                        (
                            capabilities
                                .formats
                                .iter()
                                .copied()
                                .find(wgpu::TextureFormat::is_srgb)
                                .or_else(|| capabilities.formats.first().copied())
                                .expect("Get preferred format"),
                            adapter,
                            device,
                            queue,
                        )
                    });

                surface.configure(
                    &device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format,
                        width: physical_size.width,
                        height: physical_size.height,
                        present_mode: wgpu::PresentMode::AutoVsync,
                        alpha_mode: wgpu::CompositeAlphaMode::Auto,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 2,
                    },
                );

                // `workspace` 是启动序列（daemon 连接 + 会话恢复）已经建好
                // 的状态，这里只补一次真实窗口尺寸——`set_window_size` 既把
                // 尺寸记进 `Workspace`（`view()` 的左面板区有效宽要用），也
                // 立刻按它重算终端网格：恢复出来的 tab 之前用的是
                // `DEFAULT_COLS`/`DEFAULT_ROWS` 兜底默认值，这里纠正成实际
                // 网格（也会顺带把 resize 同步给 daemon）。网格换算细节
                // （含"上次退出时右侧停在对话视图"的情形）归 workspace 侧
                // 一家管，main.rs 不再自己算一份。
                let logical: LogicalSize<f32> = physical_size.to_logical(window.scale_factor());
                workspace.set_window_size(logical.width, logical.height);

                // Initialize iced

                let renderer = {
                    let engine = Engine::new(
                        &adapter,
                        device.clone(),
                        queue.clone(),
                        format,
                        None,
                        Shell::headless(),
                    );

                    Renderer::new(engine, Font::default(), Pixels::from(16))
                };

                // You should change this if you want to render continuously
                event_loop.set_control_flow(ControlFlow::Wait);

                *self = Self::Ready {
                    window,
                    device,
                    queue,
                    renderer,
                    surface,
                    format,
                    workspace,
                    events: Vec::new(),
                    cursor: mouse::Cursor::Unavailable,
                    modifiers: ModifiersState::default(),
                    cache: user_interface::Cache::new(),
                    clipboard,
                    viewport,
                    resized: false,
                    webviews: std::collections::HashMap::new(),
                    browser_webviews: std::collections::HashMap::new(),
                    cursor_phys: winit::dpi::PhysicalPosition::new(0.0, 0.0),
                    pending_focus: None,
                };
            }
        }

        /// tokio 任务经 `EventLoopProxy<Message>::send_event` 送回来的事件
        /// （attach 数据流的输出/退出、daemon 错误、新建会话完成……）在这里
        /// 落地：直接喂给 `Workspace::update`，跟 `window_event` 里处理
        /// iced 消息走的是同一条 `update` 逻辑，只是消息来源不同。
        fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: Message) {
            if !matches!(self, Self::Ready { .. }) {
                return;
            }
            // `dispatch` 是 `&mut self` 方法，必须先确认 `Ready` 态、再
            // 结束上面那次只读匹配的借用，才能在这里调用（避免与
            // `Self::Ready { .. }` 解构借用冲突）。
            self.dispatch(event);
            self.sync_previews();
            self.apply_pending_focus();
        }

        fn window_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            _window_id: winit::window::WindowId,
            event: WindowEvent,
        ) {
            self.on_window_event(&event);

            // 解构借用限定在这个块内：块尾产出待派发的 `messages`，块结束后
            // 那些字段借用随之释放，才能在块外调用 `self.dispatch`/
            // `self.sync_previews`（它们要 `&mut self` 整体）。
            let pending_messages: Vec<Message> = {
                let Self::Ready {
                    window,
                    device,
                    queue,
                    surface,
                    format,
                    renderer,
                    workspace,
                    events,
                    viewport,
                    cursor,
                    modifiers,
                    clipboard,
                    cache,
                    resized,
                    ..
                } = self
                else {
                    return;
                };

                match event {
                    WindowEvent::RedrawRequested => {
                        // IME 候选窗跟随文本光标(否则默认落窗口左上角)。每帧
                        // 更新,始终反映当前输入上下文(终端/地址栏/意见框)。
                        {
                            let scale = window.scale_factor();
                            let size = window.inner_size();
                            let (lw, lh) = (
                                size.width as f32 / scale as f32,
                                size.height as f32 / scale as f32,
                            );
                            let (ix, iy, ih) = workspace.ime_cursor_area(lw, lh);
                            window.set_ime_cursor_area(
                                winit::dpi::LogicalPosition::new(ix, iy),
                                winit::dpi::LogicalSize::new(1.0, ih),
                            );
                        }
                        if *resized {
                            let size = window.inner_size();

                            *viewport = Viewport::with_physical_size(
                                Size::new(size.width, size.height),
                                window.scale_factor() as f32,
                            );

                            surface.configure(
                                device,
                                &wgpu::SurfaceConfiguration {
                                    format: *format,
                                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                    width: size.width,
                                    height: size.height,
                                    present_mode: wgpu::PresentMode::AutoVsync,
                                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                                    view_formats: vec![],
                                    desired_maximum_frame_latency: 2,
                                },
                            );

                            *resized = false;
                        }

                        match surface.get_current_texture() {
                            Ok(frame) => {
                                let view = frame
                                    .texture
                                    .create_view(&wgpu::TextureViewDescriptor::default());

                                let mut encoder = device.create_command_encoder(
                                    &wgpu::CommandEncoderDescriptor { label: None },
                                );

                                {
                                    // Clear the frame to the ByteBoy2077 background
                                    let _render_pass = clear(&view, &mut encoder, theme::BG);
                                }

                                // Submit the clear pass
                                queue.submit([encoder.finish()]);

                                // Draw iced on top
                                let mut interface = UserInterface::build(
                                    workspace.view(),
                                    viewport.logical_size(),
                                    std::mem::take(cache),
                                    renderer,
                                );

                                let (state, _) = interface.update(
                                    &[Event::Window(
                                        window::Event::RedrawRequested(Instant::now()),
                                    )],
                                    *cursor,
                                    renderer,
                                    clipboard,
                                    &mut Vec::new(),
                                );

                                // Update the mouse cursor
                                if let user_interface::State::Updated {
                                    mouse_interaction, ..
                                } = state
                                {
                                    // Update the mouse cursor
                                    if let Some(icon) =
                                        conversion::mouse_interaction(mouse_interaction)
                                    {
                                        window.set_cursor(icon);
                                        window.set_cursor_visible(true);
                                    } else {
                                        window.set_cursor_visible(false);
                                    }
                                }

                                // Draw the interface
                                interface.draw(
                                    renderer,
                                    &Theme::Dark,
                                    &renderer::Style::default(),
                                    *cursor,
                                );
                                *cache = interface.into_cache();

                                renderer.present(None, frame.texture.format(), &view, viewport);

                                // Present the frame
                                frame.present();
                            }
                            Err(error) => match error {
                                wgpu::SurfaceError::OutOfMemory => {
                                    panic!(
                                        "Swapchain error: {error}. \
                                        Rendering cannot continue."
                                    )
                                }
                                _ => {
                                    // Try rendering again next frame.
                                    window.request_redraw();
                                }
                            },
                        }
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        *cursor = mouse::Cursor::Available(conversion::cursor_position(
                            position,
                            viewport.scale_factor(),
                        ));
                    }
                    WindowEvent::ModifiersChanged(new_modifiers) => {
                        *modifiers = new_modifiers.state();
                    }
                    WindowEvent::Resized(new_size) => {
                        *resized = true;

                        // 窗口尺寸变了：把新的逻辑尺寸交给 `Workspace`——它
                        // 既要用这个宽度把持久化的 `left_width` 夹进当前窗口
                        // 容得下的范围（否则窄窗下右半边整片消失），也会顺手
                        // 换算终端 pane 的新网格尺寸、套用到所有 tab 的
                        // `TerminalModel` 并同步给 daemon。
                        let logical: LogicalSize<f32> = new_size.to_logical(window.scale_factor());
                        workspace.set_window_size(logical.width, logical.height);
                        // bounds 同步由本函数末尾的 sync_previews 统一执行
                    }
                    WindowEvent::CloseRequested => {
                        event_loop.exit();
                    }
                    _ => {}
                }

                // Control+左键在上面已经当右键处理(RightClickAt/坐标钳制),
                // 但那只是 main.rs 原始事件层的记账——iced 自己的 MouseArea::
                // on_right_press 是靠这里转换出的 iced 事件类型来触发的,不
                // 单独拦一次的话 iced 只会看到一次普通左键按下,行会被当成
                // 正常单击处理(打开文件/展开目录)而不是弹菜单。判定要在
                // `conversion::window_event` 消费掉 `event` 之前借用完。
                let is_control_left_press = matches!(
                    &event,
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: winit::event::MouseButton::Left,
                        ..
                    }
                ) && modifiers.control_key();

                // Map window event to iced event
                if let Some(event) =
                    conversion::window_event(event, window.scale_factor() as f32, *modifiers)
                {
                    let event = if is_control_left_press
                        && matches!(
                            event,
                            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                        ) {
                        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
                    } else {
                        event
                    };
                    events.push(event);
                }

                // If there are events pending
                if !events.is_empty() {
                    // We process them
                    let mut interface = UserInterface::build(
                        workspace.view(),
                        viewport.logical_size(),
                        std::mem::take(cache),
                        renderer,
                    );

                    let mut messages: Vec<Message> = Vec::new();

                    let _ = interface.update(events, *cursor, renderer, clipboard, &mut messages);

                    events.clear();
                    *cache = interface.into_cache();

                    messages
                } else {
                    Vec::new()
                }
            };

            // 借用已随上面的块结束释放；这里逐条经 `dispatch`
            // 派发（`ProjectPickFolder` 等在其中被拦截成 rfd 模态,
            // 其余原样转给 `workspace.update`）。
            for message in pending_messages {
                self.dispatch(message);
            }

            // webview 池与期望清单对齐：tab 增删、resize、地址栏导航都可能
            // 改变期望清单，统一在这里收口，不必在每个改变点各调一次。
            self.sync_previews();
            self.apply_pending_focus();
        }
    }

    let mut runner = Runner::Loading(Some(workspace));
    event_loop.run_app(&mut runner)

    // `runtime` 在这里才真正 drop（`main` 持有到最后一刻）：`run_app`
    // 阻塞到窗口关闭为止，期间所有 `handle.spawn` 派生的任务都能正常跑；
    // 应用退出后随 `runtime` 一起清理，不需要手动等待/取消。
}
