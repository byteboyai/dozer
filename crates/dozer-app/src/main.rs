mod app;
mod assets;
mod chrome;
mod code_editor;
mod conversation;
mod delivery;
mod dialog;
mod event;
mod extensions;
mod frosted;
mod git_watch;
mod keymap;
mod layout;
mod open_projects;
mod osc;
mod panel_layouts;
mod platform;
mod preview;
mod preview_state;
mod project;
mod project_meta;
mod runtime;
mod settings;
mod term;
mod theme;
mod transcript;
mod webview_geometry;
mod workspace;

use crate::platform::window_events::{FocusIntent, Runner};
use app::{Message, PanelKind};
// `with_allow_link_preview` 是 macOS 专有扩展 trait,需显式引入作用域。
// `with_titlebar_transparent`/`with_title_hidden`/`with_fullsize_content_view`
// 同样是 macOS 专有扩展 trait——去掉原生标题栏那条独立的深色条,让红黄绿
// 交通灯直接叠在 app 自己画的 top_bar 上面(统一工具栏样式,VS Code/Chrome
// 同款),project tabs 才能紧跟在交通灯右侧,不再有两条纵向堆叠的"标题栏"。
use winit::platform::macos::WindowAttributesExtMacOS;

use std::time::Duration;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::core::Renderer as _;
use iced_winit::core::alignment;
use iced_winit::core::input_method::InputMethod;
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::core::text;
use iced_winit::core::text::Renderer as _;
use iced_winit::core::time::Instant;
use iced_winit::core::window;
use iced_winit::core::{Color, Event, Font, Pixels, Point, Rectangle, Size, Theme};
use iced_winit::futures;
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::winit;

use winit::{
    dpi::LogicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::ModifiersState,
};

use std::sync::Arc;

/// Todo 面板可见时轮询 `.dozer/todo.md` 的间隔,兼顾响应与省电。
/// `pub(crate)`——`App::poll_todo_if_visible` 也要用它把自己限速到这个
/// 节奏。
pub(crate) const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// 拖拽排序(页签/Todo)进行中的重绘节奏:约 60fps,保证拖动时卡片实时
/// 跟手。拖拽本身靠 `on_move` 改状态,但本循环是事件驱动重绘,没有这个
/// 持续唤醒,拖动过程中屏幕不会更新,只有松手那一刻才重绘。
pub(crate) const DRAG_REDRAW_INTERVAL: Duration = Duration::from_millis(16);

/// 输入框右键菜单动作 → 对应的快捷键字符:返回 `Some('c')` 等,供
/// `link_command_event` 合成 ⌘/Ctrl+该键的键盘事件。非菜单动作返回 `None`。
pub(crate) fn menu_edit_key(message: &Message) -> Option<char> {
    match message {
        Message::TextInputMenuCut => Some('x'),
        Message::TextInputMenuCopy => Some('c'),
        Message::TextInputMenuPaste => Some('v'),
        Message::TextInputMenuSelectAll => Some('a'),
        _ => None,
    }
}

pub fn main() -> Result<(), winit::error::EventLoopError> {
    tracing_subscriber::fmt::init();

    // 第一次文本排版之前：先注册内嵌的 JetBrains Mono（代码/终端字体），
    // 再剔除毒化 CJK 回退的位图字体（见 fonts.rs 模块注释）。顺序很重要——
    // 注册在前，终端 `Family::Name("JetBrains Mono")` 才能解析。
    assets::fonts::load_embedded_fonts();
    assets::fonts::sanitize_font_db();

    // 先建立 token 基准值：把 workspace.json 灌进 byteui 三个 token 模块
    // （font/geometry/icon_size），取代其编译期内置默认值。
    crate::theme::init();
    // 再把上次退出前存盘的 UI scale 读回，确保首帧几何/布局按退出时的缩放排布
    // （Ctrl +/- 改过的 scale 由 `icon_size::persist_scale` 在每次缩放后落盘）。
    byteui::theme::icon_size::init_scale(&crate::theme::ui_scale_path());
    // 同理把上次选的配色方案（深色/浅色）读回，确保首帧就是退出时的主题
    // （设置弹窗选中即调 `color::persist_scheme` 落盘）。
    byteui::theme::color::init_scheme(&crate::theme::color_theme_path());

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

    let app = runtime.block_on(crate::runtime::build_app(client, handle, proxy.clone()));

    let mut runner = Runner::Loading(Some(app), proxy.clone());
    event_loop.run_app(&mut runner)

    // `runtime` 在这里才真正 drop（`main` 持有到最后一刻）：`run_app`
    // 阻塞到窗口关闭为止，期间所有 `handle.spawn` 派生的任务都能正常跑；
    // 应用退出后随 `runtime` 一起清理，不需要手动等待/取消。
}

impl winit::application::ApplicationHandler<Message> for Runner {
    /// 定时器到点（`ControlFlow::WaitUntil` 触发的
    /// `ResumeTimeReached`）：驱动周期性关注点（Todo 轮询/悬停动画）。
    fn new_events(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        if let winit::event::StartCause::ResumeTimeReached { .. } = cause
            && let Self::Ready { app, window, .. } = self
        {
            // Todo 面板可见时轮询磁盘上的 `.dozer/todo.md`,agent 或用户
            // 在编辑器中改完文件,面板能自动跟上。
            app.poll_todo_if_visible();
            // 新增任务闪光倒计时:到点且用户未手动改选就自动清除选中高亮
            // (每次都调,内部按 `until` 自己短路,不再显式判断 `flash_active`)。
            app.advance_todo_flash();
            // 按钮悬停动画:有动画进行中才逐拍推进,全部收敛后本拍不再改
            // 状态(`any_hover_anim_active` 为 false 时 `about_to_wait` 不会再
            // 排下一拍,自然停下)。
            if app.any_hover_anim_active() {
                app.advance_hover_anims();
            }
            // 拖拽悬停到折叠目录的展开计时:满 1s 才真正展开,见
            // `App::advance_drag_hover_expand` 文档。
            app.advance_drag_hover_expand();
            window.request_redraw();
        }
    }

    /// 每轮事件处理完后决定下次唤醒时机:各个周期性关注点(按钮悬停
    /// 动画/Todo 面板轮询/拖拽重绘/页签 tooltip 计时)各自的"是否需要
    /// 唤醒"+"需要多快"列在一起,取激活项里最小的 interval——新增周期性
    /// 关注点只需要在这个列表里加一行,不用碰其它分支(2026-08-12
    /// 解耦重构:每个关注点自己的函数各自按自己的 `last_*_at` 限速,
    /// 这里只负责"下次什么时候唤醒",不负责"唤醒后该不该真的做事")。
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Self::Ready { app, .. } = self {
            // 页签标题 tooltip 的 3s 悬停计时:还没满 3s 的页签需要继续排
            // 唤醒,满 3s 那一刻靠 `next_tooltip_wake` 算出的剩余时间精确
            // 重绘出气泡;满 3s 后 `next_tooltip_wake` 返回 None,不再空转。
            let next_tip = app.next_tooltip_wake();
            // 新增任务闪光计时:按剩余时间精确排一次"恰好 2s 才清除高亮"
            // 的唤醒,到期后 `next_todo_flash_wake` 返回 None 自然停下。
            let next_flash = app.next_todo_flash_wake();
            // 拖拽悬停展开计时:同上,满 1s 那一刻精确唤醒一次让
            // `advance_drag_hover_expand` 真正展开,没有悬停中的目录时
            // 返回 `None` 不再空转。
            let next_drag_expand = app.next_drag_hover_expand_wake();
            let wakes: [(bool, Duration); 6] = [
                (
                    app.any_hover_anim_active(),
                    crate::event::HOVER_ANIM_INTERVAL,
                ),
                (app.todo_panel_visible(), TODO_POLL_INTERVAL),
                (
                    app.todo_dragging() || app.dragging_tab().is_some(),
                    DRAG_REDRAW_INTERVAL,
                ),
                (
                    next_tip.is_some(),
                    next_tip.unwrap_or(crate::app::HOVER_TOOLTIP_DELAY),
                ),
                (
                    next_flash.is_some(),
                    next_flash.unwrap_or(crate::extensions::todo::ADD_SELECT_HIGHLIGHT),
                ),
                (
                    next_drag_expand.is_some(),
                    next_drag_expand.unwrap_or(extensions::files::DRAG_HOVER_EXPAND_DELAY),
                ),
            ];
            if let Some(interval) = wakes
                .into_iter()
                .filter(|(active, _)| *active)
                .map(|(_, interval)| interval)
                .min()
            {
                event_loop
                    .set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + interval));
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
                                byteui::theme::geometry::min_window_width(),
                                byteui::theme::geometry::min_window_height(),
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
            // 顶栏原生拖窗守卫：只需装一次方法覆写，见函数文档。
            #[cfg(target_os = "macos")]
            crate::platform::window::install_topbar_drag_guard(&window);
            // 外部文件拖拽悬停位置追踪：补 winit 没实现的 `draggingUpdated:`，
            // 见函数文档。
            #[cfg(target_os = "macos")]
            crate::platform::file_drag::install_file_drag_position_tracker(&window);
            // 原生右键菜单(NSMenu)弹层挂靠的内容 view:记一份裸指针供
            // `native_menu::show` 后续弹菜单用,见 `native_menu.rs`。
            #[cfg(target_os = "macos")]
            crate::chrome::native_menu::install_content_view(&window);

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

            let (format, adapter, device, queue) = futures::futures::executor::block_on(async {
                let adapter =
                    wgpu::util::initialize_adapter_from_env_or_default(&instance, Some(&surface))
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
                webview_rects: Vec::new(),
                cursor_phys: winit::dpi::PhysicalPosition::new(0.0, 0.0),
                files_dragging: false,
                left_mouse_down: false,
                pending_focus: None,
                // 默认终端拿键盘,跟现状(启动时终端可打字)一致。
                current_focus: FocusIntent::Terminal,
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
        // `consumed == true`:已经被应用级快捷键接管(见
        // `on_window_event` 顶部文档),下面不能再把同一个原始事件转换
        // 喂给 iced 标准管线,否则会重复处理(⌘S 这类字母快捷键会在
        // 落盘的同时把字母本身当普通字符插进 `text_editor` 正文)。
        let consumed = self.on_window_event(&event);

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
                webview_rects,
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
                    // IME 候选窗定位 + 组字预览浮层的绘制,挪到本帧
                    // `interface.update()`(下面)产出最新 `InputMethod`
                    // 之后处理——见该处理块的注释。
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

                            let mut encoder =
                                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: None,
                                });

                            {
                                // Clear the frame to the overall workspace background
                                let _render_pass = crate::event::clear(
                                    &view,
                                    &mut encoder,
                                    theme::region::background(),
                                );
                            }

                            // Submit the clear pass
                            queue.submit([encoder.finish()]);

                            // 消费"Todo 列表滚回顶部"一次性位(必须在
                            // `UserInterface::build` 之前取走,因为构建会借走
                            // `app` 的不可变引用,后面就不能再可变借用了)。
                            let scroll_pending = app.take_todo_scroll_to_top();

                            // 同理,消费"项目树行内编辑刚触发、需要程序化聚焦"
                            // 一次性位(右键菜单点"重命名"/"新建文件"这类触发
                            // 点击落在别的控件上,真 `text_input` 下一帧才出现、
                            // 不会自己拿焦点,必须在这里强制 `focus` 一下)。
                            let tree_edit_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_tree_edit_focus_pending());

                            // 同理,消费"拖拽移动确认框刚弹出、需要程序化聚焦
                            // 新名称输入框"一次性位(拖放/双击落点触发,真
                            // `text_input` 下一帧才出现、不会自己拿焦点)。
                            let move_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_move_focus_pending());

                            // 同理,消费"Todo 任务内容编辑刚触发、需要程序化
                            // 聚焦"一次性位(点卡片文字进入编辑态,真 `text_input`
                            // 下一帧才出现、不会自己拿焦点)。
                            let content_edit_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_content_edit_focus_pending());

                            // 同理,消费"Todo 分类树行内改名刚触发、需要程序化
                            // 聚焦"一次性位(右键"重命名"/新建后自动进入改名态,
                            // 真 `text_input` 下一帧才出现、不会自己拿焦点)。
                            let category_edit_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.todo.take_category_rename_focus_pending());

                            // 同理,消费 项目名称编辑/右键搜索 弹窗查询框两个
                            // 一次性聚焦位(触发点击落在旧的 button/MouseArea
                            // 上,真 `text_input` 本帧才出现、不会自己拿焦点;
                            // 验收意见框常驻可见、点击即原生聚焦,不需要这机制)。
                            let name_edit_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_name_edit_focus_pending());
                            let query_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_query_focus_pending());

                            // 同理,消费"浏览器地址栏刚获得焦点、需要全选
                            // 当前网址"的一次性位(单击即选中整条,见
                            // `App::take_addr_select_all_pending`)。
                            let addr_select_all_pending = app.take_addr_select_all_pending();

                            // 同理,右键输入框弹菜单时把焦点移到被右键的输入
                            // (右键不聚焦 iced 输入框,菜单复制/粘贴要作用到它,
                            // 必须显式 focus;一次性位,消费即复位)。
                            let input_menu_focus = app.take_pending_text_input_focus();

                            // 同理,消费"新建原生预览编辑器 tab 需要程序化聚焦"
                            // 三个一次性位——官方 `text_editor` 的焦点
                            // 是真实 iced 焦点树的一部分,构造时拿不到,要等下一帧
                            // 用 `operation::focusable::focus` 强制聚焦(同项目树
                            // 行内编辑/Todo 内容编辑的既有手法)。
                            let preview_editor_focus_id =
                                app.active_workspace_mut().and_then(|ws| {
                                    ws.preview
                                        .take_pending_editor_focus()
                                        .then(|| ws.preview.active_editor_focus_id())
                                        .flatten()
                                });
                            let project_preview_editor_focus_id =
                                app.active_workspace_mut().and_then(|ws| {
                                    ws.project_preview
                                        .take_pending_editor_focus()
                                        .then(|| ws.project_preview.active_editor_focus_id())
                                        .flatten()
                                });
                            // ⌘F 打开 Find 条后把焦点给输入框(一次性位,消费即复
                            // 位)。取的是 `find_field_id(kind)` 这个稳定静态 id——它
                            // 只出现在有条的那一帧,位与条同帧建立、同帧被这里消费。
                            let preview_find_focus_id = app.active_workspace_mut().and_then(|ws| {
                                ws.preview
                                    .take_pending_find_focus()
                                    .then(|| crate::preview::find_field_id(PanelKind::Files))
                            });
                            let project_preview_find_focus_id =
                                app.active_workspace_mut().and_then(|ws| {
                                    ws.project_preview
                                        .take_pending_find_focus()
                                        .then(|| crate::preview::find_field_id(PanelKind::Project))
                                });
                            // 查找跳到某个命中后,编辑器需要补聚焦才能画出选区高亮
                            // (一次性位,消费即复位;不撵走 Find 输入框的真焦点,
                            // 见 `FocusAlso` 文档)。
                            let preview_reveal_focus_id =
                                app.active_workspace_mut().and_then(|ws| {
                                    ws.preview
                                        .take_pending_editor_reveal_focus()
                                        .then(|| ws.preview.find_editor_focus_id())
                                        .flatten()
                                });
                            let project_preview_reveal_focus_id =
                                app.active_workspace_mut().and_then(|ws| {
                                    ws.project_preview
                                        .take_pending_editor_reveal_focus()
                                        .then(|| ws.project_preview.find_editor_focus_id())
                                        .flatten()
                                });
                            // 同理,消费"消息驱动把焦点拨离预览编辑器"一次性位
                            // (见 `Workspace::blur_preview_editors` 的说明)。
                            let editor_unfocus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_editor_unfocus_pending());
                            // 两个预览面板各自的原生 editor id(若当前 tab 是原生态)——
                            // 只有这两个 id 会被下面的 `UnfocusTargets` 摘掉焦点,不会
                            // 误伤同一次点击刚刚聚焦的其它 widget(见 `UnfocusTargets` 文档)。
                            let editor_unfocus_targets: Vec<iced_winit::core::widget::Id> = app
                                .active_workspace()
                                .map(|ws| {
                                    [
                                        ws.preview.active_editor_focus_id(),
                                        ws.project_preview.active_editor_focus_id(),
                                    ]
                                    .into_iter()
                                    .flatten()
                                    .collect()
                                })
                                .unwrap_or_default();

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

                            // 新增任务置顶后把 Todo 列表滚回顶部,让新任务
                            // 可见(一次性位,消费即复位)。
                            if scroll_pending {
                                let mut op = iced_winit::core::widget::operation::scrollable::scroll_to::<()>(
                                    iced_winit::core::widget::Id::new(
                                        crate::extensions::todo::TODO_LIST_SCROLL_ID,
                                    ),
                                    iced_winit::core::widget::operation::scrollable::AbsoluteOffset::<Option<f32>> {
                                        x: Some(0.0),
                                        y: Some(0.0),
                                    },
                                );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 项目树行内编辑刚触发时程序化聚焦真正的
                            // `text_input`(一次性位,消费即复位)。
                            if tree_edit_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::files::tree_edit_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 拖拽移动确认框刚弹出时程序化聚焦"新名称"输入框
                            // (一次性位,消费即复位)。
                            if move_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::files::move_name_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // Todo 任务内容编辑刚触发时程序化聚焦真正的
                            // `text_input`(一次性位,消费即复位)。
                            if content_edit_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::todo::content_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // Todo 分类树行内改名刚触发时程序化聚焦真正的
                            // `text_input`(一次性位,消费即复位)。
                            if category_edit_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::todo::category_rename_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 项目名称编辑 / 右键搜索弹窗查询框刚触发时程序化聚焦
                            // 真正的 `text_input`(一次性位,消费即复位)。
                            if name_edit_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::project::name_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }
                            if query_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::search::query_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 浏览器地址栏刚获得焦点时,全选当前网址(
                            // 单击即选中整条,方便直接覆盖输入)。一次性位,
                            // 消费即复位(取位在 `interface` 构建前完成)。
                            if addr_select_all_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::text_input::select_all::<()>(
                                        extensions::browser::addr_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 右键输入框弹菜单后,把焦点移到被右键的输入(
                            // 使菜单的复制/粘贴作用于它)。一次性位,消费即复位。
                            if let Some(id) = input_menu_focus {
                                let mut op = iced_widget::core::widget::operation::focusable::focus::<
                                    (),
                                >(id);
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 新建原生预览编辑器 tab 时程序化聚焦真正的
                            // `text_editor`(一次性位,消费即复位)。
                            for id in [preview_editor_focus_id, project_preview_editor_focus_id]
                                .into_iter()
                                .flatten()
                            {
                                let mut op = iced_widget::core::widget::operation::focusable::focus::<
                                    (),
                                >(id);
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // ⌘F 打开 Find 条时把焦点程序化拨到输入框(`text_input`,
                            // 一次性位如上)。放在上一段编辑器聚焦之后——同帧不会同时
                            // 开条又想聚焦编辑器,顺序无冲突。
                            for id in [preview_find_focus_id, project_preview_find_focus_id]
                                .into_iter()
                                .flatten()
                            {
                                let mut op = iced_widget::core::widget::operation::focusable::focus::<
                                    (),
                                >(id);
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 查找跳到某个命中后补聚焦编辑器,让原生 `text_editor`
                            // 画出选区高亮——用不 unfocus 别人的 `FocusAlso`(标准
                            // `operation::focusable::focus` 会把 Find 输入框刚拿到
                            // 的焦点撵掉,见其文档),放在上面 Find 输入框聚焦之后,
                            // 确保输入框的真焦点(接收键盘)不被这一步覆盖。
                            for id in [preview_reveal_focus_id, project_preview_reveal_focus_id]
                                .into_iter()
                                .flatten()
                            {
                                let mut op = crate::runtime::FocusAlso { target: id };
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // 消息驱动(非鼠标点击)把焦点拨离预览编辑器——只摘
                            // `editor_unfocus_targets`(两个预览面板各自的原生
                            // editor id)自己的焦点,不用无目标版本的
                            // `operation::focusable::unfocus()`(那个会把**当前
                            // 持有焦点的任意 widget**都摘掉,同一次点击如果恰好
                            // 正在把焦点交给别的原生输入框——比如常驻搜索框——
                            // 会被这一下连带打掉,见 `UnfocusTargets` 文档)。
                            if editor_unfocus_pending && !editor_unfocus_targets.is_empty() {
                                let mut op = crate::runtime::UnfocusTargets {
                                    targets: editor_unfocus_targets.clone(),
                                };
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }

                            // Files 搜索框(Stage 2,唯一已迁移到 iced
                            // 原生 text_input 的字段)每帧查一遍真实
                            // 焦点态——旧版靠 `SearchEditStart` 手动
                            // 置位的 bool 已随迁移废弃,main.rs 键盘
                            // 路由改成"每帧问 iced 真相"而不是自己维护
                            // 一份可能脱节的镜像。切走 Files 左栏时显式
                            // 记 false,避免残留上一次的 true(Files 不
                            // 可见时 view() 里没有这个 text_input,
                            // CaptureSearchFocus 找不到匹配 id,不会自己
                            // 覆盖成 false)。只把结果存进局部量,真正写
                            // 回 `app` 要等 `interface` 释放对 `app` 的
                            // 不可变借用之后(见下方 `into_cache` 之后),
                            // 否则这里借用 `app.view()` 建出的 `interface`
                            // 还活着,不能同时再可变借用 `app`。
                            // Files 面板可以被挪到右栏(`relocate_to_right`),
                            // 只查 `left_view` 会在这种布局下让搜索框永远
                            // 捕不到真实焦点(2026-09 用户反馈:文件树搜索框
                            // 点了也打不进字——鼠标点击本身走 iced 正常
                            // widget 树、能拿到真焦点,但这里的每帧焦点捕获
                            // 一直没跑,`files_search_focused()` 恒 false,
                            // 键盘路由 OR 链永远不放行,字符全被拦下)。
                            let files_in_either_view =
                                matches!(app.left_view(), crate::app::PanelKind::Files)
                                    || matches!(app.right_view(), crate::app::PanelKind::Files);
                            let files_focused = if files_in_either_view {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::files::CaptureSearchFocus,
                                );
                                extensions::files::take_search_focused()
                            } else {
                                false
                            };

                            // 项目树行内编辑框(Stage 5)同款每帧真实焦点查询:
                            // 与 Files 搜索框完全同构,同样要两栏都查。
                            let tree_edit_focused = if files_in_either_view {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::files::CaptureTreeEditFocus,
                                );
                                extensions::files::take_tree_edit_focused()
                            } else {
                                false
                            };

                            // 原生预览"文件内搜索"(⌘F/⌘R)查询输入框:同款
                            // 每帧真实焦点查询。Files/Project 各自的 Find 条
                            // 分别渲染在 `PanelKind::Files`/`PanelKind::Project`
                            // 对应的预览面板里(`app.rs::panel_body` 的
                            // `preview_pane`/`project_preview_pane` 分支),
                            // 同样可能被拖到左右任一栏,两侧都要查——同
                            // `git_log_search_focused` 的既有处理。两条 Find
                            // 条可能同帧都存在,`CaptureFindFocus` 一次遍历按
                            // id 分别写回两个面板各自的 static,这里一次取走
                            // 两份结果。
                            let find_panel_visible = files_in_either_view
                                || matches!(app.left_view(), crate::app::PanelKind::Project)
                                || app.right_view == crate::app::PanelKind::Project;
                            let (files_find_focused, project_find_focused) = if find_panel_visible {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut preview::CaptureFindFocus,
                                );
                                (
                                    preview::take_find_focused(crate::app::PanelKind::Files),
                                    preview::take_find_focused(crate::app::PanelKind::Project),
                                )
                            } else {
                                (false, false)
                            };

                            // SSH/Database 新增/编辑表单:同款每帧真实焦点
                            // 查询,替换掉旧版"表单是否打开"的粗粒度信号
                            // (2026-09 用户反馈两次根因:切走面板不关表单、
                            // 面板与 Agent 终端分栏同屏时表单开着但焦点其实
                            // 在终端——面板"是否可见"不等于字段"是否真聚焦",
                            // 只有每帧查真实焦点才两种场景都对)。只在对应
                            // 面板可能可见时才跑遍历,省一次无谓的树遍历
                            // (面板都不可见,表单也不可能被渲染更不可能持有
                            // 真焦点)。
                            let ssh_panel_visible =
                                matches!(app.left_view(), crate::app::PanelKind::Ssh)
                                    || app.right_view == crate::app::PanelKind::Ssh;
                            let ssh_form_focused = if ssh_panel_visible {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::ssh::CaptureFormFocus,
                                );
                                extensions::ssh::take_form_focused()
                            } else {
                                false
                            };
                            let database_panel_visible =
                                matches!(app.left_view(), crate::app::PanelKind::Database)
                                    || app.right_view == crate::app::PanelKind::Database;
                            let database_form_focused = if database_panel_visible {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::database::CaptureFormFocus,
                                );
                                extensions::database::take_form_focused()
                            } else {
                                false
                            };

                            // 浏览器地址栏(Stage 3)同款每帧真实焦点查询:
                            // 与 Files 搜索框完全同构。`PanelKind::Web` 是浏览器
                            // 面板对应的 left_view 取值(同 FocusIntent::Browser
                            // 分支查 PanelKind::Web 的既有用法)。首页
                            // (`is_home()`)恒渲染全局浏览器 `home_browser`,是跟
                            // 工作区 `ws.browser` 独立的另一个实例,`active_
                            // workspace()` 在首页上恒为 `None`——两种情况都要跑
                            // 这个查询,否则首页地址栏永远探测不到焦点(代码审阅
                            // 时发现的既有缺口,读写目标的分流在 `App::
                            // browser_addr_focused`/`set_browser_addr_focused`
                            // 里做,这里只负责查询)。同样只把结果存进局部量,
                            // 写回 app 要等 `interface` 释放借用之后。
                            let browser_addr_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Web)
                                    || app.is_home()
                                {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::browser::CaptureAddrFocus,
                                    );
                                    extensions::browser::take_addr_focused()
                                } else {
                                    false
                                };

                            // Todo 搜索框(Stage 4)同款:每帧查真实焦点态。
                            // 只在 Todo 左栏可见时跑,不必要时不做无谓遍历。
                            let todo_search_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::todo::CaptureTodoSearchFocus,
                                    );
                                    extensions::todo::take_todo_search_focused()
                                } else {
                                    false
                                };

                            // Todo 添加框(Stage 4):同款每帧查真实焦点态。
                            let todo_add_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::todo::CaptureAddFocus,
                                    );
                                    extensions::todo::take_add_focused()
                                } else {
                                    false
                                };

                            // Todo 任务内容编辑框(Stage 5):同款每帧查真实焦点
                            // 态。只在 Todo 左栏可见时跑;光标/焦点在编辑态内由原生
                            // `text_input` 自己维护,这里只要真/假,供
                            // `set_todo_content_focused` 做"失焦即落盘"边缘触发。
                            let content_edit_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::todo::CaptureContentEditFocus,
                                    );
                                    extensions::todo::take_content_edit_focused()
                                } else {
                                    false
                                };

                            // Todo 分类树行内改名框:同款每帧查真实
                            // 焦点态,只在 Todo 左栏可见时跑。
                            let category_rename_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::todo::CaptureCategoryRenameFocus,
                                    );
                                    extensions::todo::take_category_rename_focused()
                                } else {
                                    false
                                };

                            // Todo 任务详情弹窗的回复框:同款每帧查真实
                            // 焦点态(Todo 浮层都长在当前总布局里)。
                            let detail_reply_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::todo::CaptureDetailReplyFocus,
                                    );
                                    extensions::todo::take_detail_reply_focused()
                                } else {
                                    false
                                };

                            // 首页项目搜索框(Stage 4):同款每帧查真实焦点态。
                            let home_search_focused = if app.is_home() {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut crate::chrome::homespace::CaptureHomeSearchFocus,
                                );
                                crate::chrome::homespace::take_home_search_focused()
                            } else {
                                false
                            };

                            // 会话列表搜索框:同款每帧查真实焦点态。该面板
                            // (`PanelKind::Conversations`)默认挂右栏,但跟其它
                            // 面板一样可以被拖到左栏(见 `App::rail_cross_apply`),
                            // 所以两侧都要查,不能只看 `left_view()`(同浏览器
                            // 地址栏 `browser_addr_focused` 两侧都查的既有处理)。
                            let conversation_search_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Conversations)
                                    || app.right_view == crate::app::PanelKind::Conversations
                                {
                                    crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::conversations::CaptureConversationSearchFocus,
                                );
                                    extensions::conversations::take_conversation_search_focused()
                                } else {
                                    false
                                };

                            // Git Log commit 搜索框:同款每帧查真实焦点态。该面板
                            // 默认挂左栏,但同样可以被拖到右栏(见
                            // `App::rail_cross_apply`),两侧都要查,同
                            // `conversation_search_focused` 的既有处理。
                            let git_log_search_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::GitLog)
                                    || app.right_view == crate::app::PanelKind::GitLog
                                {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::git_log::CaptureSearchFocus,
                                    );
                                    extensions::git_log::take_search_focused()
                                } else {
                                    false
                                };

                            // 项目名称编辑框(Stage 6):渲染在 `PanelKind::
                            // Project`,同款每帧查真实焦点态。
                            let name_edit_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Project) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::project::CaptureNameEditFocus,
                                    );
                                    extensions::project::take_name_edit_focused()
                                } else {
                                    false
                                };

                            // 项目描述编辑框:同 `name_edit_focused`,渲染在
                            // `PanelKind::Project`,每帧查真实焦点态。
                            let description_edit_focused =
                                if matches!(app.left_view(), crate::app::PanelKind::Project) {
                                    crate::runtime::run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut extensions::project::CaptureDescriptionEditFocus,
                                    );
                                    extensions::project::take_description_edit_focused()
                                } else {
                                    false
                                };

                            // 右键搜索弹窗查询框(Stage 6):全局浮层,不挂靠
                            // 任何 `left_view`,gating 条件用 `search_popup_open`。
                            let query_focused = if app.search_popup_open() {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::search::CaptureQueryFocus,
                                );
                                extensions::search::take_query_focused()
                            } else {
                                false
                            };

                            // IME 组字预览浮层的落点 + 内容,`State::Updated`
                            // 分支下面填充,画在 `interface.draw()` 之后
                            // (见下方"画 IME 组字预览浮层"注释)。
                            let mut ime_overlay: Option<(Rectangle, String)> = None;

                            // Update the mouse cursor
                            if let user_interface::State::Updated {
                                mouse_interaction,
                                input_method,
                                ..
                            } = state
                            {
                                // Update the mouse cursor
                                if let Some(icon) = conversion::mouse_interaction(mouse_interaction)
                                {
                                    // 光标落进任一**可见** wry 子视图(预览面板
                                    // / 浏览器面板)时,把光标控制权让给 WKWebView:
                                    // 否则 iced 每帧 `window.set_cursor` 会把窗口
                                    // 光标强制刷成箭头,盖掉 WKWebView 自己在超链接
                                    // 上显示的握手光标(窗口级 NSCursor 是全局唯一
                                    // 的,谁最后写谁赢)。webview 自身会管理光标
                                    // (箭头/握手/文本),无需我们隐藏或覆写。
                                    let cursor_over_webview =
                                        webview_rects.iter().any(|&(wx, wy, ww, wh)| {
                                            let (cx, cy) = app.last_cursor;
                                            ww > 0.0
                                                && wh > 0.0
                                                && cx >= wx
                                                && cx <= wx + ww
                                                && cy >= wy
                                                && cy <= wy + wh
                                        });
                                    if !cursor_over_webview {
                                        window.set_cursor(icon);
                                        window.set_cursor_visible(true);
                                    }
                                } else {
                                    window.set_cursor_visible(false);
                                }
                                // 顶栏原生拖窗守卫复用这同一个每帧算出的
                                // interaction——非 `None` 即悬停在某个可
                                // 交互控件上（页签/关闭按钮/…），见
                                // `install_topbar_drag_guard` 文档。
                                #[cfg(target_os = "macos")]
                                crate::platform::window::TOPBAR_CONTROL_HOVERED.store(
                                    mouse_interaction != mouse::Interaction::None,
                                    std::sync::atomic::Ordering::Relaxed,
                                );

                                // IME 候选窗跟随文本光标:优先用 iced 自己
                                // 算出的 `input_method.cursor`(任何原生
                                // text_input 聚焦且要 IME 时都会给出精确
                                // 位置——这套手写的低层事件循环不像标准
                                // `iced_winit::program::run` 那样自动替我们
                                // 调 `set_ime_cursor_area`/画组字预览浮层,
                                // 这两件事以前都没人接,候选窗与组字文字
                                // 因此一律钉在终端光标位置或者干脆画不出来
                                // (2026-08-21 修复)。没有任何原生控件要 IME
                                // 时(`Disabled`,包括终端聚焦的情况——终端
                                // 不是 iced 控件)才退回终端光标的手写算法。
                                match input_method {
                                    InputMethod::Enabled {
                                        cursor, preedit, ..
                                    } => {
                                        window.set_ime_cursor_area(
                                            winit::dpi::LogicalPosition::new(cursor.x, cursor.y),
                                            winit::dpi::LogicalSize::new(
                                                cursor.width.max(1.0),
                                                cursor.height,
                                            ),
                                        );
                                        if let Some(p) = preedit
                                            && !p.content.is_empty()
                                        {
                                            ime_overlay = Some((cursor, p.content));
                                        }
                                    }
                                    InputMethod::Disabled => {
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
                                }
                            }

                            // Draw the interface
                            interface.draw(
                                renderer,
                                &Theme::Dark,
                                &renderer::Style::default(),
                                *cursor,
                            );

                            // 画 IME 组字预览浮层:iced 的 `text_input` 自己
                            // 不画组字中的文字(只上报 `InputMethod`,标准
                            // `iced_winit::program::run` 才会把它画成一层
                            // "over-the-spot" 浮层),这套手写事件循环没有
                            // 这层运行时,只能自己补——半透明底 + 描边下划线,
                            // 视觉上照抄终端自己那份组字预览
                            // (`term_view.rs` 的 `preedit` 绘制)。粗略估算
                            // 宽度(字符数 × 近似字宽),不做精确度量/换行,
                            // 同终端那份一贯的"先简单实现"取舍。
                            if let Some((cursor, content)) = ime_overlay {
                                let font_size = byteui::theme::font::body() as f32;
                                let char_w = font_size * 0.6;
                                let width =
                                    (unicode_width::UnicodeWidthStr::width(content.as_str())
                                        as f32
                                        * char_w)
                                        .max(char_w);
                                let height = cursor.height.max(font_size * 1.4);
                                let bounds = Rectangle::new(
                                    Point::new(cursor.x, cursor.y),
                                    Size::new(width, height),
                                );
                                renderer.with_layer(Rectangle::INFINITE, |renderer| {
                                    renderer.fill_quad(
                                        renderer::Quad {
                                            bounds,
                                            ..renderer::Quad::default()
                                        },
                                        Color {
                                            a: 0.25,
                                            ..byteui::theme::color::current().cream
                                        },
                                    );
                                    renderer.fill_text(
                                        text::Text {
                                            content,
                                            bounds: Size::new(width, height),
                                            size: Pixels(font_size),
                                            line_height: text::LineHeight::default(),
                                            font: renderer.default_font(),
                                            align_x: text::Alignment::Left,
                                            align_y: alignment::Vertical::Top,
                                            shaping: text::Shaping::Advanced,
                                            wrapping: text::Wrapping::None,
                                        },
                                        Point::new(cursor.x, cursor.y),
                                        byteui::theme::color::current().cream,
                                        bounds,
                                    );
                                    renderer.fill_quad(
                                        renderer::Quad {
                                            bounds: Rectangle::new(
                                                Point::new(cursor.x, cursor.y + height - 2.0),
                                                Size::new(width, 2.0),
                                            ),
                                            ..renderer::Quad::default()
                                        },
                                        byteui::theme::color::current().cream,
                                    );
                                });
                            }

                            *cache = interface.into_cache();

                            // `interface` 已被消费,对 `app` 的不可变借用
                            // 随之释放——之前算好的 Files 搜索框真实焦点
                            // 现在才写回工作区(供下一帧键盘路由
                            // `files_search_focused` 消费)。每帧都重写,
                            // 即使值没变也幂等,无副作用。
                            app.set_files_search_focused(files_focused);
                            app.set_todo_search_focused(todo_search_focused);
                            app.set_todo_add_focused(todo_add_focused);
                            app.set_home_project_search_focused(home_search_focused);
                            app.set_tree_edit_focused(tree_edit_focused);
                            app.set_todo_content_focused(content_edit_focused);
                            app.set_category_rename_focused(category_rename_focused);
                            app.set_detail_reply_focused(detail_reply_focused);
                            app.set_project_name_focused(name_edit_focused);
                            app.set_project_description_focused(description_edit_focused);
                            app.set_query_focused(query_focused);
                            app.set_conversation_search_focused(conversation_search_focused);
                            app.set_git_log_search_focused(git_log_search_focused);
                            app.set_find_query_focused(
                                crate::app::PanelKind::Files,
                                files_find_focused,
                            );
                            app.set_find_query_focused(
                                crate::app::PanelKind::Project,
                                project_find_focused,
                            );
                            app.set_ssh_form_focused(ssh_form_focused);
                            app.set_database_form_focused(database_form_focused);

                            // 同上,浏览器地址栏的真实焦点态现在才写回工作区
                            // (供下一帧键盘路由 `browser_addr_focused` 消费)。
                            app.set_browser_addr_focused(browser_addr_focused);

                            // 关闭掉 `iced_graphics` 的 `web-colors` 后(见根
                            // Cargo.toml 的 `[patch]`: 阻断 umbrella `iced`
                            // default 特性里的 `web-colors`, 恢复 sRGB 伽马校正),
                            // `iced_renderer::Renderer` 不再是 `wgpu+tiny-skia`
                            // 的 fallback 枚举, 而直接就是 `iced_wgpu::Renderer`,
                            // 因此这里不再分支, 直接 present。
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
                    crate::platform::window::center_traffic_lights(window);
                    // bounds 同步由本函数末尾的 sync_previews 统一执行
                }
                WindowEvent::CloseRequested => {
                    // 同步写盘,不用 `spawn_shell_layout_save` 的异步路径——
                    // 进程马上退出,spawn 的 tokio 任务不保证跑得完。
                    app.persist_window_size_on_exit();
                    // 关 tab 时发往 daemon 的 kill/总结请求同理不保证跑完
                    // ——这里有限等待,避免用户关 tab 后立刻退出导致请求
                    // 半路被丢弃、daemon 侧会话仍 alive、重启后还魂。
                    app.wait_for_pending_exit_tasks();
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

            // Map window event to iced event。`consumed` 时跳过:这个
            // 事件已经被 `on_window_event` 当应用级快捷键接管过了,不能
            // 再喂给 iced 标准管线重复处理(见该函数顶部文档)。
            if !consumed
                && let Some(event) =
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

                // 输入框右键菜单动作:把 "Cut/Copy/Paste/Select All" 合成回
                // 对应的 ⌘/Ctrl+`x`/`c`/`v`/`a` 键盘事件,喂给同一个
                // `interface` 再跑一遍——复用 iced 原生 text_input /
                // text_editor 的剪贴板与光标插入逻辑(它们靠
                // `key.to_latin(physical_key)+command` 识别快捷键;
                // text_input 还会自行跳过密码框的复制/剪切)。
                //
                // 帧序:菜单是上一帧右键弹出的(那时已把焦点经
                // `focusable::focus` 移到被右键的输入),这里点菜单项本身
                // 不抢走输入框焦点;合成前再用 `focusable::focus` 补一次,
                // 双保险保证作用于被右键的那个输入。
                if let Some(ch) = messages.iter().find_map(menu_edit_key) {
                    if let Some(id) = app.text_input_menu_target_id() {
                        let mut op =
                            iced_winit::core::widget::operation::focusable::focus::<()>(id);
                        crate::runtime::run_operate(&mut interface, renderer, &mut op);
                    }
                    let synth = [crate::event::unique_command_event(ch)];
                    let mut second: Vec<Message> = Vec::new();
                    let _ = interface.update(&synth, *cursor, renderer, clipboard, &mut second);
                    messages.append(&mut second);
                }

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

        // mac 原生右键菜单选中了剪切/复制/粘贴/全选:上面 `dispatch` 已经
        // 跑完 `TextInputMenuOpen` 的处理、写好了
        // `pending_native_menu_edit_key`,但驱动合成键盘事件所需的
        // `UserInterface` 只在上面那个块的作用域内活着,已经被
        // `interface.into_cache()` 收掉——这里独立重新借一次
        // `Self::Ready` 的字段、建一个新 `UserInterface`,补聚焦 + 把
        // `⌘/Ctrl+x/c/v/a` 事件喂给它,和 `pending_messages` 完全同一套
        // 手法(见上面那段注释里链接的 `menu_edit_key` 原始用法)。
        #[cfg(target_os = "macos")]
        {
            let synth_messages: Vec<Message> = {
                let Self::Ready {
                    app,
                    renderer,
                    viewport,
                    cursor,
                    clipboard,
                    cache,
                    ..
                } = self
                else {
                    return;
                };
                match app.take_pending_native_menu_edit_key() {
                    Some((ch, target_id)) => {
                        let mut interface = UserInterface::build(
                            app.view(),
                            viewport.logical_size(),
                            std::mem::take(cache),
                            renderer,
                        );
                        let mut op =
                            iced_winit::core::widget::operation::focusable::focus::<()>(target_id);
                        crate::runtime::run_operate(&mut interface, renderer, &mut op);
                        let synth = [crate::event::unique_command_event(ch)];
                        let mut second: Vec<Message> = Vec::new();
                        let _ = interface.update(&synth, *cursor, renderer, clipboard, &mut second);
                        *cache = interface.into_cache();
                        second
                    }
                    None => Vec::new(),
                }
            };
            for message in synth_messages {
                self.dispatch(message);
            }
        }

        // webview 池与期望清单对齐：tab 增删、resize、地址栏导航都可能
        // 改变期望清单，统一在这里收口，不必在每个改变点各调一次。
        self.sync_previews();
        self.apply_pending_focus();
        self.apply_pending_zoom_toggle();
    }
}
