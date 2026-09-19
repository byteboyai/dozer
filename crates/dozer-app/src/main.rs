mod app;
mod assets;
mod chrome;
mod code_editor;
mod conversation;
mod delivery;
mod dialog;
mod event;
mod extensions;
mod external_apps;
mod frosted;
mod git_accounts;
mod git_watch;
mod keymap;
mod layout;
mod menu_spec;
mod open_projects;
mod osc;
mod panel_layouts;
mod platform;
mod preview;
mod preview_state;
mod project;
mod project_meta;
mod runtime;
mod tabular;
mod term;
mod theme;
mod transcript;
mod webview_geometry;
mod workspace;

use crate::platform::window_events::Runner;
use app::Message;

use iced_winit::winit;
use winit::event_loop::EventLoop;

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
