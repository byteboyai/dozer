mod app;
mod assets;
mod clipboard_image;
mod conversation;
mod delivery;
mod extensions;
mod fonts;
mod git_watch;
mod goal;
mod homespace;
mod icons;
mod keymap;
mod layout;
mod open_projects;
mod osc;
mod preview;
mod preview_state;
mod project;
mod scrollbar;
mod term_model;
mod term_view;
mod theme;
mod transcript;
mod workspace;

use app::{App, LeftView, Message};
// `with_allow_link_preview` 是 macOS 专有扩展 trait,需显式引入作用域。
use wry::WebViewBuilderExtDarwin;
// `with_titlebar_transparent`/`with_title_hidden`/`with_fullsize_content_view`
// 同样是 macOS 专有扩展 trait——去掉原生标题栏那条独立的深色条,让红黄绿
// 交通灯直接叠在 app 自己画的 top_bar 上面(统一工具栏样式,VS Code/Chrome
// 同款),project tabs 才能紧跟在交通灯右侧,不再有两条纵向堆叠的"标题栏"。
use winit::platform::macos::WindowAttributesExtMacOS;

use std::process::{Command, Stdio};

/// macOS 专有：把原生红黄绿交通灯在垂直方向居中到 app 自己画的 `top_bar`
/// （默认 40pt 高）中部，而不是系统默认的 28pt 标题栏中部。去掉原生标题栏
/// 后系统仍按 28pt 旧基准排版交通灯，导致它们贴着顶栏上沿、与 40pt 顶栏里
/// 的 Dozer 字标/页签不对齐。差额对半即把三个灯整体下移、落入顶栏正中。
///
/// 放在 `WindowEvent::Resized`（含首屏显隐、全屏进出、拖拽缩放）里重设——
/// 一次拖拽缩放会连续触发几十次 `Resized`。首次成功读到的 frame 被锁成
/// 三个灯各自的原生基线，此后每次都从基线重算绝对目标 y 再整体覆盖，不对
/// "当前 frame"做相对减法：早先版本是相对减法（`origin.y -= offset`），
/// 错误地假设系统会在两次 `Resized` 之间把灯摆回默认位置——实测并不总成立，
/// 导致偏移跨调用累积，且三个灯被系统"纠正"的次数不一定相同，越拖越偏、
/// 灯与灯之间还会彼此错位。
///
/// 基线按"是否最大化"分两条（`BASELINE_NORMAL`/`BASELINE_MAXIMIZED`），各自
/// 第一次遇到该状态时读一次原生位置——不能只留一条：普通窗口态与最大化态
/// (尤其是铺满整个屏幕宽度、贴着刘海屏摄像头挖孔的情形)系统原生摆放交通灯
/// 的 y 并不是同一个值。只用一条基线时，谁先触发第一次 `Resized` 谁就把
/// 基线定死；若这次启动窗口一开始就是（上次退出前记住的）铺满屏幕尺寸，
/// 基线会按最大化态的位置锁定，之后窗口变回普通大小时又被强行按这条错的
/// 基线摆回去——反之，若基线来自普通态，窗口后来被最大化时同样会被摆错，
/// 错到超出可见/可点的标题栏范围就直接看不见了（用户反馈"最大化后交通灯
/// 消失"的根因）。仅在 `target_os = "macos"` 编译——其它平台无原生交通灯
/// 可摆。
#[cfg(target_os = "macos")]
fn center_traffic_lights(window: &winit::window::Window) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::OnceLock;

    static BASELINE_NORMAL: OnceLock<[f64; 3]> = OnceLock::new();
    static BASELINE_MAXIMIZED: OnceLock<[f64; 3]> = OnceLock::new();

    const BAND_HEIGHT: f32 = 28.0;
    let offset = (theme::geometry::top_bar_height() - BAND_HEIGHT) / 2.0;
    if offset <= 0.0 {
        return;
    }

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(ah) = handle.as_raw() else {
        return;
    };
    // 安全：窗口由 winit 主线程持有且已建好；`ns_view` 是已安装进窗口的
    // 合法 NSView 指针。只借只读引用去读/改交通灯按钮的 frame，不持有/
    // 释放 NSView 或 NSWindow。
    let ns_view: &NSView = unsafe { &*(ah.ns_view.as_ptr() as *mut NSView) };
    let Some(ns_window) = ns_view.window() else {
        return;
    };

    let buttons = [
        ns_window.standardWindowButton(NSWindowButton::CloseButton),
        ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        ns_window.standardWindowButton(NSWindowButton::ZoomButton),
    ];

    let baseline_cell = if window.is_maximized() {
        &BASELINE_MAXIMIZED
    } else {
        &BASELINE_NORMAL
    };
    let baseline = *baseline_cell.get_or_init(|| {
        let mut ys = [0.0; 3];
        for (slot, btn) in ys.iter_mut().zip(&buttons) {
            if let Some(btn) = btn {
                *slot = btn.frame().origin.y;
            }
        }
        ys
    });

    for (i, btn) in buttons.into_iter().enumerate() {
        let Some(btn) = btn else { continue };
        // `frame`/`setFrameOrigin` 在当前 objc2 版本是安全方法；只挪 y,
        // x 保留系统原生间距。
        let mut origin = btn.frame().origin;
        origin.y = baseline[i] - offset as f64;
        btn.setFrameOrigin(origin);
    }
}
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
/// 所有按钮悬停动画的帧间隔:约 60fps。配合 `App::advance_hover_anims`
/// 的指数逼近(每拍残余 75%),约 150ms 收敛,给出跟手的 ease-out 过渡。
const HOVER_ANIM_INTERVAL: Duration = Duration::from_millis(16);
/// Todo 面板可见时轮询 `.dozer/todo.md` 的间隔,兼顾响应与省电。
const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);

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
/// 会话恢复（`App::bootstrap`），失败则降级为错误态
/// （`App::with_daemon_error`）。整段在 `main()` 用
/// `runtime.block_on` 驱动，此时窗口尚未创建，不占用任何"正在跑的"
/// UI 线程。
async fn build_app(
    client: dozer_client::Client,
    handle: tokio::runtime::Handle,
    proxy: winit::event_loop::EventLoopProxy<Message>,
) -> App {
    match ensure_daemon(&client).await {
        Ok(()) => App::bootstrap(client, handle, proxy).await,
        Err(message) => App::with_daemon_error(client, handle, proxy, message),
    }
}

pub fn main() -> Result<(), winit::error::EventLoopError> {
    tracing_subscriber::fmt::init();

    // 第一次文本排版之前：先注册内嵌的 JetBrains Mono（代码/终端字体），
    // 再剔除毒化 CJK 回退的位图字体（见 fonts.rs 模块注释）。顺序很重要——
    // 注册在前，终端 `Family::Name("JetBrains Mono")` 才能解析。
    fonts::load_embedded_fonts();
    fonts::sanitize_font_db();

    // 把上次退出前存盘的 UI scale 读回，确保首帧几何/布局按退出时的缩放排布
    // （Ctrl +/- 改过的 scale 由 `icon_size::persist_scale` 在每次缩放后落盘）。
    crate::theme::icon_size::init_scale();

    // 注册 sqlx `Any` 驱动的具体实现(Postgres/MySQL/SQLite),必须在第一次
    // `sqlx::AnyPool::connect` 之前跑一次(数据库面板连接测试用)。
    sqlx::any::install_default_drivers();

    // Initialize winit：用户事件类型直接是 `Message`——tokio 任务经
    // `EventLoopProxy<Message>::send_event` 把事件流/daemon 状态送回 UI
    // 线程，`ApplicationHandler::user_event` 收到后转发给
    // `app.update`。
    let event_loop = EventLoop::<Message>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    // tokio Runtime 由 main 持有，跟 winit 事件循环共存一整个进程生命
    // 周期（`event_loop.run_app` 之后才会 drop）。UI 线程只允许用
    // `handle.spawn` 派发任务，绝不 `block_on` 网络 IO——启动序列的
    // 一次性 `block_on` 是唯一例外（窗口还没创建，谈不上"占用 UI 线程"）。
    let runtime = tokio::runtime::Runtime::new().expect("创建 tokio runtime");
    let handle = runtime.handle().clone();
    let client = dozer_client::Client::new(dozer_core::paths::socket_path());

    let app = runtime.block_on(build_app(client, handle, proxy.clone()));

    #[allow(clippy::large_enum_variant)]
    enum Runner {
        /// 持有启动序列已经构建好的 `App`（daemon 已连上/已降级为
        /// 错误态，视情况可能已经装好若干恢复出来的项目页签与 tab）；
        /// `resumed()` 建好窗口/wgpu 后把它 `take()` 出来转入 `Ready`。用
        /// `Option` 包一层只是为了能在 `&mut self` 上 `take`，正常情况下
        /// `resumed()` 只会被调用一次。
        Loading(Option<App>, winit::event_loop::EventLoopProxy<Message>),
        Ready {
            window: Arc<winit::window::Window>,
            queue: wgpu::Queue,
            device: wgpu::Device,
            surface: wgpu::Surface<'static>,
            format: wgpu::TextureFormat,
            renderer: Renderer,
            app: App,
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
            /// 上一次同步 webview 池时聚焦的项目 id。两个池都是**窗口级**的,
            /// key 是 `PreviewPane` 的 tab id,而那个 id 在每个项目的
            /// `Workspace` 里都从 0 独立起编——项目 A 的预览 tab 0 和项目 B 的
            /// 预览 tab 0 会撞成同一个 key。撞上时 `sync_webview_pool` 会认为
            /// "这个 id 已经有 webview 了",只对**旧** webview 调 `load_url`;
            /// 而旧 webview 的 `dozer://` 协议闭包捕获的是**项目 A** 的
            /// `allowed_files` 白名单,于是项目 B 的文件请求被那份白名单挡下,
            /// 预览一片空白(切回 A 又正常,表现成"切到 B 就坏了")。
            ///
            /// 所以聚焦项目一变就整池清空,强制每个 webview 重新创建、重新
            /// 捕获当前项目的白名单。这不损失缓存:`sync_webview_pool` 的
            /// `retain` 本来就只保留"当前项目期望清单里的 id",切走的项目的
            /// webview 无论如何都会被销毁。
            webview_project: Option<i64>,
            /// 最近一次光标物理位置(CursorMoved 更新),鼠标点击时用于命中测试。
            cursor_phys: winit::dpi::PhysicalPosition<f64>,
            /// 待应用的焦点意图(点击/消息设置,sync_previews 之后统一 apply,
            /// 确保新建 webview 已入池)。
            pending_focus: Option<FocusIntent>,
            /// 事件循环代理:webview IPC handler 用它把 `WebViewFocused` 送回
            /// UI 线程(winit 收不到子 webview 上的鼠标点击)。
            proxy: winit::event_loop::EventLoopProxy<Message>,
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
        proxy: winit::event_loop::EventLoopProxy<Message>,
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
                        // 导航会重置 WKWebView 的 pageZoom,重建后把当前
                        // 全局 UI 缩放补回去,否则预览字号会跳回 100%。
                        let _ = view.zoom(crate::theme::icon_size::scale() as f64);
                    }
                    let _ = view.set_bounds(bounds);
                    let _ = view.set_visible(spec.visible);
                }
                None => {
                    let allowed = std::sync::Arc::clone(&allowed_files);
                    let root = assets::assets_root();
                    let ipc_proxy = proxy.clone();
                    let built = wry::WebViewBuilder::new()
                        .with_url(&spec.url)
                        .with_bounds(bounds)
                        .with_visible(spec.visible)
                        // 关掉 macOS 的链接预览(force-click 弹出 peek 浮层),
                        // 否则点网页里的超链接会变成"预览"而非跳转,表现就是
                        // "能打开网页但点不了超链接"。wry 默认 allow_link_preview=true。
                        .with_allow_link_preview(false)
                        // 子 webview 上的 mousedown winit 收不到,这里注入 JS
                        // 在捕获阶段监听 mousedown,经 IPC 通知宿主调 view.focus()
                        // 让 WKWebView 成为 first responder(否则 ⌘C 选区复制
                        // 走不通:WKWebView 不是 first responder 时 keyDown 不到它)。
                        .with_initialization_script(
                            "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);"
                        )
                        .with_ipc_handler(move |_req| {
                            if _req.body() == "focus" {
                                let _ = ipc_proxy.send_event(Message::WebViewFocused);
                            }
                        })
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
                            // 新 webview 按当前全局 UI 缩放初始化,使预览字号
                            // 跟着 ⌘/Ctrl +/- 一起缩放(`WebView::zoom` 在
                            // macOS 11+ 走 WKWebView 的 pageZoom)。
                            let _ = view.zoom(crate::theme::icon_size::scale() as f64);
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
        /// `keymap` 翻译成字节，直接回灌 `App`（`Message::TermInput`），
        /// 不必改动 `winit::application::ApplicationHandler` 的实现本身。
        ///
        /// `App::update` 收到 `TermInput` 后写给 daemon（`client.write`），
        /// 不再本地 echo——回显完全走 PTY 真实回路（daemon → attach 流 →
        /// `Message::TermOutput` → `TerminalModel::feed`）。
        fn on_window_event(&mut self, event: &WindowEvent) {
            // 把 `modifiers` 和 `app`/`window` 放进同一次解构里取，
            // 避免先借一次 `self` 再调用 `&self` 方法造成的重复借用。
            let Self::Ready {
                app,
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
                    if app.dragging_divider().is_some() {
                        let scale = window.scale_factor();
                        let logical_x = (cursor_phys.x / scale) as f32;
                        let window_width = (window.inner_size().width as f64 / scale) as f32;
                        app.update(Message::ColumnDrag {
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
                    let x = x.min((window_w - theme::geometry::context_menu_width()).max(0.0));
                    let y = y.min((window_h - theme::geometry::context_menu_height()).max(0.0));
                    app.update(Message::Files(extensions::files::Message::RightClickAt {
                        x,
                        y,
                    }));
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
                    app.blur_inputs();
                    let scale = window.scale_factor();
                    let logical_x = (cursor_phys.x / scale) as f32;
                    let logical_w = (window.inner_size().width as f64 / scale) as f32;
                    // 点哪侧面板区,哪侧的 left_zone/right_zone 外边框就亮
                    // 起来(与下面窄一些的 webview 焦点路由是两回事:这个
                    // 覆盖整个面板区,不区分区内具体哪个 pane)。
                    app.set_active_zone(logical_x, logical_w);
                    let state = app.shell_state();
                    *pending_focus =
                        Some(if app::is_in_preview_column(logical_x, logical_w, &state) {
                            if state.left_view == LeftView::Web {
                                FocusIntent::Browser
                            } else {
                                FocusIntent::Preview
                            }
                        } else {
                            FocusIntent::Terminal
                        });
                    window.request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.dragging_divider().is_some() => {
                    app.update(Message::ColumnDragEnd);
                    window.request_redraw();
                }
                _ => {}
            }

            // 右键菜单打开时,Esc 优先关菜单,不进正常键盘分发(不然会被当作
            // 普通按键继续往下走,可能被地址栏/终端等其它分支消费掉)。
            if app.context_menu_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::Files(extensions::files::Message::ContextMenuClose));
                window.request_redraw();
                return;
            }

            // Agent 选择菜单打开时,Esc 优先关菜单,同右键菜单的处理口径
            // (见上一段紧邻的 context_menu_open 分支的注释)。
            if app.agent_picker_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::AgentPickerClose);
                window.request_redraw();
                return;
            }

            // Todo 派发选择层打开时,Esc 同样优先关掉弹出层,口径同上面的
            // agent 选择菜单。
            if app.todo_dispatch_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::Todo(extensions::todo::Message::DispatchClose));
                window.request_redraw();
                return;
            }

            // 预览编辑弹层打开时,Esc 优先触发关闭流程(脏则弹确认,不脏直接
            // 关),口径同上面几个弹层。
            if app.edit_session_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::PreviewEditCloseRequest);
                window.request_redraw();
                return;
            }

            // 预览编辑弹层打开时,其余按键一律不再往下走 ⌘ 快捷键/地址栏/
            // 终端转发——弹层里的 `text_editor` 走标准 iced 事件管线
            // (`.on_action(Message::PreviewEditAction)`),这里不需要也不
            // 应该手工转发。不加这道闸门的话,`terminal_visible()` 只看右侧
            // 是否展开、对弹层状态一无所知,默认布局(右侧终端可见)下弹层
            // 里打的每个字符、包括回车,都会同时写进背后那个终端/agent 会话
            // (Critical,code review 发现)。
            if app.edit_session_open() {
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
                            if let Some(text) = app.active_selection_text() {
                                clipboard.write(iced_winit::core::clipboard::Kind::Standard, text);
                            }
                        }
                        "v" => {
                            if let Some(text) =
                                clipboard.read(iced_winit::core::clipboard::Kind::Standard)
                            {
                                app.update(Message::TermPaste(text));
                                window.request_redraw();
                            } else if let Some(path) =
                                clipboard_image::read_pasteboard_image_as_temp_file()
                            {
                                // 剪贴板没有文本表示(纯截图),iced 的
                                // Clipboard::read 只认字符串,取不到图片
                                // 字节。落临时 PNG,粘贴文件路径——claude
                                // 等 CLI 会把路径识别成图片附件加载。
                                app.update(Message::TermPaste(path.to_string_lossy().into_owned()));
                                window.request_redraw();
                            }
                        }
                        _ => {}
                    }
                }
                return;
            }

            // 浏览器地址栏 / 验收意见 / 项目树行内编辑态 / 项目标题编辑:
            // 键盘直达自绘输入(不经 keymap、不进 PTY)。文件预览面板已不再
            // 有地址栏。
            let to_browser = app.browser_addr_editing();
            let to_comment = app.acceptance_comment_editing();
            let to_tree_edit = app.tree_editing();
            let to_project_title = app.project_title_editing();
            if to_browser || to_comment || to_tree_edit || to_project_title {
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
                    // 优先级:浏览器地址栏 > 验收意见 > 项目树编辑 > 项目信息
                    // 面板标题(四者同真时罕见,谁先建的编辑态谁优先没有实际
                    // 冲突场景,这个顺序只是一个确定性兜底)。
                    let message = if to_browser {
                        Message::Browser(extensions::browser::Message::AddrEvent(ev))
                    } else if to_comment {
                        Message::Acceptance(extensions::acceptance::Message::CommentEvent(ev))
                    } else if to_tree_edit {
                        Message::Files(extensions::files::Message::EditEvent(ev))
                    } else {
                        Message::Project(extensions::project::Message::TitleEditEvent(ev))
                    };
                    app.update(message);
                    window.request_redraw();
                }
                return;
            }

            // Ctrl + / Ctrl - 全局 UI 缩放:与 ⌘ 应用快捷键同级拦截,不进 PTY。
            // 同时接受 `=`/`+`(Ctrl+= 通常需 Shift,逻辑键可能是 "=" 或 "+"),
            // 覆盖不同键盘布局。
            if modifiers.control_key()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
            {
                let zoom = match &event.logical_key {
                    winit::keyboard::Key::Character(s) => match s.as_str() {
                        "+" | "=" => Some(Message::ZoomIn),
                        "-" => Some(Message::ZoomOut),
                        "1" => Some(Message::ZoomReset),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(msg) = zoom {
                    app.update(msg);
                    window.request_redraw();
                    return;
                }
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
                    app.active_app_cursor_mode(),
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
                app.update(Message::TermInput(bytes));
                window.request_redraw();
            }
        }

        /// 把 app 的 webview 期望清单同步到真实 wry 子视图:
        /// 建缺失、毁多余、对齐可见性与 bounds、URL 变更时导航。文件预览池
        /// 和浏览器池各自独立同步(`preview_desired`/`browser_desired` 已按
        /// `left_view` 互斥,同一时刻至多一个非空)。
        fn sync_previews(&mut self) {
            let Self::Ready {
                window,
                app,
                webviews,
                browser_webviews,
                webview_project,
                proxy,
                ..
            } = self
            else {
                return;
            };

            // 换了聚焦项目 → 整池清空,理由见 `webview_project` 字段文档
            // (跨项目 tab id 撞 key 会复用捕获了别的项目白名单的 webview)。
            if *webview_project != app.active_project_id() {
                webviews.clear();
                browser_webviews.clear();
                *webview_project = app.active_project_id();
            }

            let size = window.inner_size();
            let scale = window.scale_factor();
            let logical_w = size.width as f32 / scale as f32;
            let logical_h = size.height as f32 / scale as f32;
            let (x, y, w, h) =
                app::preview_content_bounds(logical_w, logical_h, &app.shell_state());
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
            };

            sync_webview_pool(
                window.as_ref(),
                webviews,
                app.preview_desired(),
                bounds,
                app.allowed_files(),
                proxy.clone(),
            );
            sync_webview_pool(
                window.as_ref(),
                browser_webviews,
                app.browser_desired(),
                bounds,
                app.allowed_files(),
                proxy.clone(),
            );
        }

        /// `ProjectTabPickFolder`/`ProjectTreeCopyPath` 等需要窗口句柄侧原生
        /// 能力(rfd 模态、系统剪贴板)的消息在此拦截,其余原样转给
        /// `app.update`。
        fn dispatch(&mut self, message: Message) {
            let Self::Ready {
                app,
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
            // 新建/切换/落成终端 tab 时把焦点交回窗口,确保光标落在输入框。
            if matches!(
                message,
                Message::PreviewOpenPath(_) | Message::PreviewSelectTab(_)
            ) {
                *pending_focus = Some(FocusIntent::Preview);
            } else if matches!(
                message,
                Message::Browser(extensions::browser::Message::OpenUrl(_))
                    | Message::Browser(extensions::browser::Message::SelectTab(_))
            ) {
                *pending_focus = Some(FocusIntent::Browser);
            } else if matches!(
                message,
                Message::SelectTab(_)
                    | Message::TabAttached(_, _, _, _)
                    | Message::AgentPickerSelect(_)
            ) {
                *pending_focus = Some(FocusIntent::Terminal);
            } else if matches!(message, Message::WebViewFocused) {
                // 子 webview 上的 mousedown winit 收不到,JS 经 IPC 发来这条
                // 消息——按当前 `left_view` 判断归预览池还是浏览器池。
                let state = app.shell_state();
                *pending_focus = Some(if state.left_view == LeftView::Web {
                    FocusIntent::Browser
                } else {
                    FocusIntent::Preview
                });
            }
            match message {
                // 顶栏"＋"与项目栏"打开项目…"共用的唯一打开入口:rfd 模态选中
                // 后一律落成**新增页签**(`ProjectTabOpen`)。此前项目栏那颗按钮
                // 另有一条 `ProjectPickFolder`→`ProjectOpen` 的就地改写路径,
                // 会杀掉当前项目的全部会话,已删除。
                Message::ProjectTabPickFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        app.update(Message::ProjectTabOpen(dir));
                    }
                }
                Message::Files(extensions::files::Message::CopyPath(path, kind)) => {
                    let root = app.active_project_path().unwrap_or_else(|| path.clone());
                    let s = crate::project::path_string(kind, &path, &root);
                    clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
                    app.update(Message::Files(extensions::files::Message::ContextMenuClose));
                    window.request_redraw();
                }
                other => app.update(other),
            }
            window.request_redraw();
        }

        /// sync_previews 之后统一应用焦点意图(此时新建 webview 已入池)。
        /// Preview/Browser → 各自当前激活 webview 拿键盘(⌘C 原生复制);
        /// Terminal/无 webview → 交回窗口(终端键盘)。
        fn apply_pending_focus(&mut self) {
            let Self::Ready {
                app,
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
                Some(FocusIntent::Preview) => match app.active_preview_webview_id() {
                    Some(id) => {
                        if let Some((view, _)) = webviews.get(&id) {
                            let _ = view.focus(); // 返回 Result,忽略
                        } else {
                            window.focus_window();
                        }
                    }
                    None => window.focus_window(),
                },
                Some(FocusIntent::Browser) => match app.active_browser_webview_id() {
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

        /// 双击顶栏空白处缩放窗口(`Message::TopBarDoubleClick` →
        /// `App::pending_zoom_toggle`)。`App` 不持有 `Window` 句柄,真正
        /// 调用 `set_maximized` 只能在这里做——与 `apply_pending_focus`
        /// 同一套"派发完消息后轮询待处理标记"节奏。
        fn apply_pending_zoom_toggle(&mut self) {
            let Self::Ready {
                app,
                window,
                webviews,
                browser_webviews,
                ..
            } = self
            else {
                return;
            };
            if app.take_pending_zoom_toggle() {
                window.set_maximized(!window.is_maximized());
            }
            // 全局 UI 缩放变更后,把同一 scale 同步给所有已存在的预览/
            // 浏览器 webview(新建的 webview 已在 `sync_webview_pool` 里按
            // 当前 scale 初始化,这里只补"已存在"这一增量)。
            if app.take_pending_preview_zoom() {
                let scale = crate::theme::icon_size::scale() as f64;
                for (view, _) in webviews.values() {
                    let _ = view.zoom(scale);
                }
                for (view, _) in browser_webviews.values() {
                    let _ = view.zoom(scale);
                }
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
                && let Self::Ready { app, window, .. } = self
            {
                app.toggle_blink();
                // Todo 面板可见时轮询磁盘上的 `.dozer/todo.md`,agent 或用户
                // 在编辑器中改完文件,面板能自动跟上。
                app.poll_todo_if_visible();
                // 按钮悬停动画:有动画进行中才逐拍推进,全部收敛后本拍不再改
                // 状态(`any_hover_anim_active` 为 false 时 `about_to_wait` 不会再
                // 排下一拍,自然停下)。
                if app.any_hover_anim_active() {
                    app.advance_hover_anims();
                }
                window.request_redraw();
            }
        }

        /// 每轮事件处理完后决定下次唤醒时机：有 tab 在工作就排下一拍闪烁
        /// 唤醒，否则回到 `Wait` 省电（不再空转重绘）。
        fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Ready { app, .. } = self {
                if app.any_blinking() || app.any_hover_anim_active() || app.todo_panel_visible() {
                    let interval = if app.any_hover_anim_active() {
                        HOVER_ANIM_INTERVAL
                    } else {
                        // Todo 面板可见时按固定的 TODO_POLL_INTERVAL 节奏轮询,
                        // 兼顾响应与省电;不需要像悬停动画那样切到更密的帧率。
                        if app.todo_panel_visible() && !app.any_blinking() {
                            TODO_POLL_INTERVAL
                        } else {
                            BLINK_INTERVAL
                        }
                    };
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        std::time::Instant::now() + interval,
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
        }

        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Loading(pending_app, proxy) = self {
                let Some(mut app) = pending_app.take() else {
                    // `resumed()` 理论上只会真正建窗口这一次；后续（若平台
                    // 又调用一次 `resumed`）直接跳过，避免重复建窗口。
                    return;
                };
                // 建窗尺寸优先用上次退出前存的偏好(`layout.json` 没有就是
                // `INITIAL_WINDOW_SIZE`),下次启动记得住用户调整过的窗口大小。
                let (init_w, init_h) = app.window_size_pref();
                let window = Arc::new(
                    event_loop
                        .create_window(
                            winit::window::WindowAttributes::default()
                                .with_title("Dozer")
                                .with_inner_size(LogicalSize::new(init_w, init_h))
                                // 双保险:窗口不许缩到"两个面板区都放不下最小宽"
                                // 以下。真正保证右半边不消失的是 app 侧的
                                // `clamp_left_width`(持久化宽可能远大于这个最小
                                // 宽),这里只是把最坏情形挡在外面。
                                .with_min_inner_size(LogicalSize::new(
                                    theme::geometry::min_window_width(),
                                    theme::geometry::min_window_height(),
                                ))
                                // 统一工具栏:标题栏背景透明 + 不画标题文字 + 内容
                                // 视图延伸到标题栏区域下面,三者必须同时打开——少
                                // 任何一个,要么标题栏留一条实色条,要么内容顶部
                                // 被裁掉一截空白。交通灯本身仍是系统原生绘制,不
                                // 受这三个开关影响,继续可点/可用。
                                .with_titlebar_transparent(true)
                                .with_title_hidden(true)
                                .with_fullsize_content_view(true),
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

                // `app` 是启动序列（daemon 连接 + 会话恢复）已经建好
                // 的状态，这里只补一次真实窗口尺寸——`set_window_size` 既把
                // 尺寸记进 `App`（`view()` 的左面板区有效宽要用），也
                // 立刻按它重算终端网格：恢复出来的 tab 之前用的是
                // `DEFAULT_COLS`/`DEFAULT_ROWS` 兜底默认值，这里纠正成实际
                // 网格（也会顺带把 resize 同步给 daemon）。网格换算细节
                // （含"上次退出时右侧停在对话视图"的情形）归 app 侧
                // 一家管，main.rs 不再自己算一份。
                let logical: LogicalSize<f32> = physical_size.to_logical(window.scale_factor());
                app.set_window_size(logical.width, logical.height);

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
                    app,
                    events: Vec::new(),
                    cursor: mouse::Cursor::Unavailable,
                    modifiers: ModifiersState::default(),
                    cache: user_interface::Cache::new(),
                    clipboard,
                    viewport,
                    resized: false,
                    webviews: std::collections::HashMap::new(),
                    browser_webviews: std::collections::HashMap::new(),
                    // 池是空的,记 `None` 让第一次 sync_previews 自然对齐到
                    // 当前项目(清空空池是 no-op)。
                    webview_project: None,
                    cursor_phys: winit::dpi::PhysicalPosition::new(0.0, 0.0),
                    pending_focus: None,
                    proxy: proxy.clone(),
                };
            }
        }

        /// tokio 任务经 `EventLoopProxy<Message>::send_event` 送回来的事件
        /// （attach 数据流的输出/退出、daemon 错误、新建会话完成……）在这里
        /// 落地：直接喂给 `App::update`，跟 `window_event` 里处理
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
            self.apply_pending_zoom_toggle();
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
                    app,
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
                            let (ix, iy, ih) = app.ime_cursor_area(lw, lh);
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
                                    // Clear the frame to the overall workspace background
                                    let _render_pass =
                                        clear(&view, &mut encoder, theme::region::background());
                                }

                                // Submit the clear pass
                                queue.submit([encoder.finish()]);

                                // Draw iced on top
                                let mut interface = UserInterface::build(
                                    app.view(),
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

                        // 窗口尺寸变了：把新的逻辑尺寸交给 `App`——它
                        // 既要用这个宽度把持久化的 `left_width` 夹进当前窗口
                        // 容得下的范围（否则窄窗下右半边整片消失），也会顺手
                        // 换算终端 pane 的新网格尺寸、套用到所有 tab 的
                        // `TerminalModel` 并同步给 daemon。
                        let logical: LogicalSize<f32> = new_size.to_logical(window.scale_factor());
                        app.set_window_size(logical.width, logical.height);
                        // 每次尺寸变化都把原生交通灯重新居中到顶栏中部——首屏
                        // 显隐、全屏进出、拖拽缩放都会经过这里，见
                        // `center_traffic_lights`（内部按基线幂等，重复调用安全）。
                        center_traffic_lights(window);
                        // bounds 同步由本函数末尾的 sync_previews 统一执行
                    }
                    WindowEvent::CloseRequested => {
                        // 同步写盘,不用 `spawn_shell_layout_save` 的异步路径——
                        // 进程马上退出,spawn 的 tokio 任务不保证跑得完。
                        app.persist_window_size_on_exit();
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
                        app.view(),
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
            // 派发（`ProjectTabPickFolder` 等在其中被拦截成 rfd 模态,
            // 其余原样转给 `app.update`）。
            for message in pending_messages {
                self.dispatch(message);
            }

            // webview 池与期望清单对齐：tab 增删、resize、地址栏导航都可能
            // 改变期望清单，统一在这里收口，不必在每个改变点各调一次。
            self.sync_previews();
            self.apply_pending_focus();
            self.apply_pending_zoom_toggle();
        }
    }

    let mut runner = Runner::Loading(Some(app), proxy.clone());
    event_loop.run_app(&mut runner)

    // `runtime` 在这里才真正 drop（`main` 持有到最后一刻）：`run_app`
    // 阻塞到窗口关闭为止，期间所有 `handle.spawn` 派生的任务都能正常跑；
    // 应用退出后随 `runtime` 一起清理，不需要手动等待/取消。
}
