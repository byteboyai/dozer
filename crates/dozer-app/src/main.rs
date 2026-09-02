mod app;
mod assets;
mod clipboard_image;
mod conversation;
mod delivery;
mod diff_render;
mod extensions;
mod fonts;
mod git_watch;
mod goal;
mod homespace;
mod keymap;
mod layout;
mod menu;
mod open_projects;
mod osc;
mod panel_layouts;
mod preview;
mod preview_state;
mod project;
mod project_meta;
mod project_scaffold;
mod rail;
mod tab_widget;
mod term_model;
mod term_view;
mod terminal;
mod theme;
mod topbar;
mod transcript;
mod webview_geometry;
mod workspace;

use app::{App, Message, PanelKind};
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
    let offset = (byteui::theme::geometry::top_bar_height() - BAND_HEIGHT) / 2.0;
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

/// macOS 专有：内容视图当前帧是否悬停在某个可交互 iced 控件上——由
/// `RedrawRequested` 里算出的 `mouse_interaction`（`!= None` 即悬停中）
/// 每帧刷新，`install_topbar_drag_guard` 装的方法覆写读它决定是否放行原生
/// 拖窗。只在 macOS 下用到，其它平台不编译。
#[cfg(target_os = "macos")]
static TOPBAR_CONTROL_HOVERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// macOS 专有：给窗口内容 NSView 挂一个 `mouseDownCanMoveWindow` 覆写。
///
/// 根因：`with_titlebar_transparent`+`with_fullsize_content_view` 保留了
/// 原生标题栏固定 ~28pt 高的拖窗行为——落在这条带内的按下事件，无论下面画
/// 的是什么，AppKit 都当"标题栏背景"处理，直接原生拖窗，这一步发生在事件
/// 送到 iced 之前，`Button`/`MouseArea` 换成谁都挡不住（已用独立测试实例
/// 实测确认：拖起点在项目页签上，松手时整个窗口被拖走）。项目页签紧贴顶栏
/// 底部又几乎顶到顶栏顶（见 `top_bar_height`/`project_tab_item` 的
/// `tab_h`），因此大半个页签的可点区域落在这条 28pt 带以内。
///
/// 覆写后：当前帧鼠标悬停在任何可交互控件上（`TOPBAR_CONTROL_HOVERED`）
/// 时返回 `NO`，按下事件正常交给 iced（选中/拖拽换位照常）；悬停在真正
/// 空白顶栏时保持默认 `YES`，原生拖窗照常——这正是用户要的"没有 tab 组件
/// 的地方长按用以拖动整个窗口"。只把方法加到 winit 内容视图**自己的**
/// 运行时类上（`class_addMethod` 对已注册类添加全新 selector，不覆盖
/// `NSView` 本身继承来的实现），不影响其它 NSView 实例。
#[cfg(target_os = "macos")]
fn install_topbar_drag_guard(window: &winit::window::Window) {
    use objc2::encode::Encode;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{ffi, sel};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(ah) = handle.as_raw() else {
        return;
    };
    // 安全：同 `center_traffic_lights`——`ns_view` 是已装进窗口的合法指针，
    // 这里只借它的运行时类指针去挂方法，不持有/释放该对象。
    let ns_view: &AnyObject = unsafe { &*(ah.ns_view.as_ptr() as *mut AnyObject) };
    let class: &AnyClass = ns_view.class();

    unsafe extern "C-unwind" fn mouse_down_can_move_window(_this: &AnyObject, _cmd: Sel) -> Bool {
        let hovered = TOPBAR_CONTROL_HOVERED.load(std::sync::atomic::Ordering::Relaxed);
        Bool::new(!hovered)
    }

    // `-(BOOL)mouseDownCanMoveWindow` 无额外参数，类型串只有返回值+self+_cmd
    // 三段，手写与 objc2 内部生成的格式一致（`Encoding` 的 `Display` 即
    // Objective-C runtime 类型编码字符）。
    let types = std::ffi::CString::new(format!(
        "{}{}{}",
        Bool::ENCODING,
        <*mut AnyObject>::ENCODING,
        Sel::ENCODING
    ))
    .expect("type encoding 不含 NUL");
    let imp: Imp = unsafe {
        core::mem::transmute::<unsafe extern "C-unwind" fn(&AnyObject, Sel) -> Bool, Imp>(
            mouse_down_can_move_window,
        )
    };
    let added = unsafe {
        ffi::class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            sel!(mouseDownCanMoveWindow),
            imp,
            types.as_ptr(),
        )
    };
    if !added.as_bool() {
        tracing::warn!("挂 mouseDownCanMoveWindow 覆写失败(selector 可能已存在于该类)");
    }
}

/// macOS 专有：单个"＋"入口要能同时选择**文件**和**目录**（项目文档 /
/// Agent 记忆都能挂任意路径）。rfd 的 `pick_file`/`pick_folder` 各自只允许
/// 一种（`NSOpenPanel` 内部硬编码 `canChooseFiles`/`canChooseDirectories` 二
/// 选一），所以这里直接用 objc2 构造 `NSOpenPanel`，把两个开关都打开，让用
/// 户既能选中文件也能选中文件夹；选中结果按原路径上的 `is_dir()` 判定虚拟链
/// 接类别。只在 macOS 编译——其它平台回退到 rfd 的 `pick_file`（见调用处）。
#[cfg(target_os = "macos")]
fn pick_file_or_dir(start_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    autoreleasepool(|_| {
        // `Message::ProjectLinkPick` 在 winit 事件循环（macOS 主线程）里同步
        // 处理，此处必然持有主线程标记，可直接同步 runModal。
        let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
        let panel = NSOpenPanel::openPanel(mtm);
        // 同时放行文件和目录；保持单选的既有行为（对应 rfd 的 `pick_file`）。
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        if !start_dir.as_os_str().is_empty() {
            let dir = NSString::from_str(&start_dir.to_string_lossy());
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&dir)));
        }
        if panel.runModal() != NSModalResponseOK {
            return None;
        }
        // 单选面板：取第一个 URL 的路径。
        let url = panel.URLs().firstObject()?;
        let path = url.path()?;
        Some(std::path::PathBuf::from(path.to_string()))
    })
}

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
use iced_winit::core::{Color, Event, Font, Pixels, Point, Rectangle, Size, SmolStr, Theme};
use iced_winit::futures;
use iced_winit::runtime::task;
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::runtime::{Action, clipboard::Action as ClipboardAction};
use iced_winit::winit;

use winit::{
    dpi::LogicalSize,
    event::{ElementState, Ime, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::ModifiersState,
};

use std::sync::Arc;

/// 所有按钮悬停动画的帧间隔:约 60fps。配合 `App::advance_hover_anims`
/// 的指数逼近(每拍残余 50%),约 80ms 收敛,给出跟手的 ease-out 过渡。
const HOVER_ANIM_INTERVAL: Duration = Duration::from_millis(16);
/// Todo 面板可见时轮询 `.dozer/todo.md` 的间隔,兼顾响应与省电。
/// `pub(crate)`——`App::poll_todo_if_visible` 也要用它把自己限速到这个
/// 节奏。
pub(crate) const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// 拖拽排序(页签/Todo)进行中的重绘节奏:约 60fps,保证拖动时卡片实时
/// 跟手。拖拽本身靠 `on_move` 改状态,但本循环是事件驱动重绘,没有这个
/// 持续唤醒,拖动过程中屏幕不会更新,只有松手那一刻才重绘。
pub(crate) const DRAG_REDRAW_INTERVAL: Duration = Duration::from_millis(16);

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

/// 输入框右键菜单动作 → 对应的快捷键字符:返回 `Some('c')` 等,供
/// `link_command_event` 合成 ⌘/Ctrl+该键的键盘事件。非菜单动作返回 `None`。
fn menu_edit_key(message: &Message) -> Option<char> {
    match message {
        Message::TextInputMenuCut => Some('x'),
        Message::TextInputMenuCopy => Some('c'),
        Message::TextInputMenuPaste => Some('v'),
        Message::TextInputMenuSelectAll => Some('a'),
        _ => None,
    }
}

/// 合成一个 `⌘/Ctrl(COMMAND)+ch` 的 `KeyPressed` iced 事件。`text_input`/
/// `text_editor` 靠 `key.to_latin(physical_key)` 把它落成 `'x'/'c'/'v'/'a'`,
/// 再检查 `modifiers.command()` 走各自的剪贴板/全选分支;`physical_key` 给
/// 出与字符一致的物理键码,保证 `to_latin` 命中。`COMMAND` 在 mac 上即 ⌘
/// (LOGO),其余平台即 Ctrl(见 iced `keyboard::Modifiers::COMMAND`)。
fn unique_command_event(ch: char) -> Event {
    use iced_winit::core::keyboard::key::Code;
    use iced_winit::core::keyboard::{self, key};
    let code = match ch {
        'a' => Code::KeyA,
        'c' => Code::KeyC,
        'v' => Code::KeyV,
        'x' => Code::KeyX,
        _ => unreachable!("menu_edit_key only maps c/x/v/a"),
    };
    let keyboard_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Character(SmolStr::new(format!("{ch}"))),
        modified_key: keyboard::Key::Character(SmolStr::new(format!("{ch}"))),
        physical_key: key::Physical::Code(code),
        modifiers: keyboard::Modifiers::COMMAND,
        location: keyboard::Location::Standard,
        text: Some(SmolStr::new(format!("{ch}"))),
        repeat: false,
    };
    Event::Keyboard(keyboard_event)
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

/// 执行一次 `operation` 遍历(程序化聚焦/滚动/每帧真实焦点镜像查询)。
///
/// 兜底修复:第三方 `iced_code_editor` 用 `iced_aw::ContextMenu` 包住代码画布,
/// 而 iced_aw 0.13.1 的 `ContextMenu::operate` 在菜单展开(`show == true`)时会
/// 把 underlay 的 layout 错当 overlay 的 layout 交给一段只含菜单浮动的 widget
/// 树,`iced_widget::Container::operate` 于是 `layout.children().next().unwrap()`
/// 到 `None` 直接 panic(整窗卡死)。Dozer 无法 patch crates.io 上的 iced_aw,故
/// 在调用侧 `catch_unwind` 兜底。这些遍历只读镜像/一次性操作,崩溃时该帧镜像
/// 留 `false`、程序化动作不生效,都是安全的;菜单一关下一帧即恢复。panic hook
/// 在遍历期间临时摘掉,避免菜单展开的每帧都打印一整屏误导性的 backtrace。
fn run_operate(
    interface: &mut UserInterface<Message, iced_widget::Theme, iced_renderer::Renderer>,
    renderer: &mut iced_renderer::Renderer,
    operation: &mut dyn iced_winit::core::widget::operation::Operation,
) {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        interface.operate(renderer, operation);
    }));
    std::panic::set_hook(prev_hook);
    if result.is_err() {
        tracing::debug!("operate 遍历被 iced_aw ContextMenu 的展开菜单 panic 吸收(安全)");
    }
}

pub fn main() -> Result<(), winit::error::EventLoopError> {
    tracing_subscriber::fmt::init();

    // 第一次文本排版之前：先注册内嵌的 JetBrains Mono（代码/终端字体），
    // 再剔除毒化 CJK 回退的位图字体（见 fonts.rs 模块注释）。顺序很重要——
    // 注册在前，终端 `Family::Name("JetBrains Mono")` 才能解析。
    fonts::load_embedded_fonts();
    fonts::sanitize_font_db();

    // 先建立 token 基准值：把 workspace.json 灌进 byteui 三个 token 模块
    // （font/geometry/icon_size），取代其编译期内置默认值。
    crate::theme::init();
    // 再把上次退出前存盘的 UI scale 读回，确保首帧几何/布局按退出时的缩放排布
    // （Ctrl +/- 改过的 scale 由 `icon_size::persist_scale` 在每次缩放后落盘）。
    byteui::theme::icon_size::init_scale(&crate::theme::ui_scale_path());

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
            renderer: iced_renderer::Renderer,
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
            /// 最近一次同步后所有**可见** wry 子视图的逻辑矩形(窗口逻辑坐标
            /// `(x, y, w, h)`),两个池(预览 + 浏览器)合并。每帧在
            /// `sync_previews` 里重算,`RedrawRequested` 的 cursor 更新用它做
            /// 命中测试:光标落在任一 webview 内就**不**调 `window.set_cursor`,
            /// 把光标控制权让给 WKWebView——否则 iced 每帧把窗口光标强制刷成
            /// 箭头,会盖掉 WKWebView 自己在超链接上显示的握手光标。
            webview_rects: Vec<(f32, f32, f32, f32)>,
            /// 最近一次光标物理位置(CursorMoved 更新),鼠标点击时用于命中测试。
            cursor_phys: winit::dpi::PhysicalPosition<f64>,
            /// 是否有外部 OS 文件正处于拖拽过程(winit `HoveredFile` 置
            /// true、`HoveredFileCancelled`/`DroppedFile` 置 false)。置 true
            /// 期间每个 `CursorMoved` 都会 re-hit-test 文件树并刷新 `FileDragHover`
            /// 高亮;置 false 时收起高亮并把拖入交回终端现状行为。
            files_dragging: bool,
            /// 待应用的焦点意图(点击/消息设置,sync_previews 之后统一 apply,
            /// 确保新建 webview 已入池)。一次性:apply 完就被 `.take()` 走。
            pending_focus: Option<FocusIntent>,
            /// 当前键盘焦点归属,跟 `pending_focus` 同一批地方一起设,但常驻
            /// 不被消费——原生预览 tab(`iced-code-editor` 直接画在窗口里,
            /// 不像旧 wry 预览那样有 OS 级 webview 抢走键盘)要靠这个字段
            /// 才能在 `window_event` 的按键分发链里判断"键盘现在真的该给
            /// 预览列,还是该给终端",见键盘路由那段注释。
            current_focus: FocusIntent,
            /// 事件循环代理:webview IPC handler 用它把 `WebViewFocused` 送回
            /// UI 线程(winit 收不到子 webview 上的鼠标点击)。
            proxy: winit::event_loop::EventLoopProxy<Message>,
        },
    }

    /// 点击/消息后决定键盘焦点归谁:预览 webview、浏览器 webview(各自
    /// ⌘C 走原生复制)或窗口(终端)。
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum FocusIntent {
        Preview(PanelKind),
        Browser,
        Terminal,
    }

    /// `sync_previews` 的差集同步逻辑,预览池/浏览器池共用同一套算法,
    /// 各自传各自的 `pool`/`specs`,互不干扰。每条 spec 自带各自的矩形
    /// (不再是整批共用一个)——支持两个不同的 webview 面板(如 `Files` 在
    /// 左栏、`Project` 在右栏)同时出现在同一个池里,各自摆在各自的位置。
    /// 见 spec "webview 面板的镜像 bounds(2026-08-19 Stage 4a 审阅后修订)"
    /// 一节。
    fn sync_webview_pool(
        window: &winit::window::Window,
        pool: &mut std::collections::HashMap<usize, (wry::WebView, String)>,
        specs: Vec<(preview::WebviewSpec, wry::Rect)>,
        allowed_files: std::sync::Arc<
            std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
        >,
        review_snapshot: std::sync::Arc<std::sync::Mutex<Option<String>>>,
        proxy: winit::event_loop::EventLoopProxy<Message>,
        report_title: bool,
    ) {
        let desired_ids: std::collections::HashSet<usize> =
            specs.iter().map(|(s, _)| s.id).collect();
        pool.retain(|id, _| desired_ids.contains(id));

        for (spec, bounds) in specs {
            match pool.get_mut(&spec.id) {
                Some((view, loaded_url)) => {
                    // 去重判断要同时看两个来源,缺一个都会闪:
                    // - `view.url()`(webview 实际地址):网页内超链接让 webview
                    //   自行导航后,这里才反映新地址。只信它会导致地址栏驱动
                    //   的导航`load_url` 后、真提交前那几帧里 `view.url()`
                    //   仍是旧值,每一帧都重发一次 `load_url`(反复重启导航,
                    //   页面不停闪烁)。
                    // - `loaded_url`(我们曾下达的命令地址):只信它会在网页内
                    //   跳转完成后把已加载好的目标页误判成"要加载"再整页重载
                    //   一次。`view.url()` 失败(罕见)时退回缓存的 `loaded_url`。
                    // 合起来:只有**既没到过、也没下达过**这个地址时才真正加载,
                    // 既挡住地址栏提交后的重复导航,又不打扰网页内跳转。
                    let current = view.url().unwrap_or_else(|_| loaded_url.clone());
                    if current != spec.url && *loaded_url != spec.url {
                        if let Err(e) = view.load_url(&spec.url) {
                            tracing::warn!("预览导航失败: {e}");
                        }
                        *loaded_url = spec.url.clone();
                        // 导航会重置 WKWebView 的 pageZoom,重建后把当前
                        // 全局 UI 缩放补回去,否则预览字号会跳回 100%。
                        let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                    }
                    let _ = view.set_bounds(bounds);
                    let _ = view.set_visible(spec.visible);
                }
                None => {
                    let allowed = std::sync::Arc::clone(&allowed_files);
                    let review_snapshot = std::sync::Arc::clone(&review_snapshot);
                    let root = assets::assets_root();
                    let ipc_proxy = proxy.clone();
                    let nav_proxy = proxy.clone();
                    let webview_id = spec.id;
                    // 常驻注入脚本:焦点/拖拽/缩放三件套(所有 webview);浏览器
                    // 面板(`report_title`)额外附一段"页面标题回报":把
                    // `window.__dozer_webview` 记成本 webview 的 id,页面
                    // DOMContentLoaded 与 `document.title` 变化(MutationObserver)
                    // 时经 IPC 发 `title:<id>:<title>`,让 tab 标题在页面加载
                    // 完成后从 URL 切换到 HTML `<title>`。预览 webview 不掺和。
                    let mut script = String::from(
                        "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);document.addEventListener('mouseup',function(){window.ipc.postMessage('mouseup')},true);document.addEventListener('keydown',function(e){if(e.ctrlKey){var c=e.code,k=e.key;if(c==='Equal'||k==='+'||k==='='){e.preventDefault();window.ipc.postMessage('zoom_in');}else if(c==='Minus'||k==='-'){e.preventDefault();window.ipc.postMessage('zoom_out');}else if(c==='Digit1'||k==='1'){e.preventDefault();window.ipc.postMessage('zoom_reset');}}},true);",
                    );
                    // 文本光标 + Ctrl/Cmd+C 复制(所有 webview):
                    // - 默认让普通文字显示 I 形文本光标;链接/按钮等可交互元素
                    //   仍是手形(继承自 `cursor: text` 时会被 `cursor: pointer`
                    //   覆盖,因为后写、同为 `!important`)。
                    // - Ctrl+C / Cmd+C 都拦截复制当前选区:webview 成为 first
                    //   responder 后 keydown 被它吃掉、到不了 winit,得在这段
                    //   注入 JS 里自己拦。用 `document.execCommand('copy')`
                    //   (用户手势触发,WKWebView 会放行);个别方案拿不到选区
                    //   或失败时再退回 `navigator.clipboard`。`Cmd+Meta` 双键
                    //   都按,保证两套快捷键一致触发。用 `e.code` 判断,避免
                    //   键盘布局差异影响 `e.key`。
                    script.push_str(
                        "(function(){var s=document.createElement('style');s.textContent=\"html,body,body *{cursor:text!important}a,a *,button,*[role='link'],*[role='button'],summary,*[onclick],label[for]{cursor:pointer!important}\";(document.head||document.documentElement).appendChild(s);document.addEventListener('keydown',function(e){if((e.ctrlKey||e.metaKey)&&e.code==='KeyC'){e.preventDefault();var ok=false;try{ok=document.execCommand('copy')}catch(_){}if(!ok){var g=window.getSelection&&window.getSelection();if(g&&g.toString()){try{navigator.clipboard.writeText(g.toString()).then(function(){},function(){})}catch(_){}}}}});})();",
                    );
                    if report_title {
                        script.push_str("window.__dozer_webview=");
                        script.push_str(webview_id.to_string().as_str());
                        script.push_str(
                            ";function _dt(){window.ipc.postMessage('title:'+String(__dozer_webview)+':'+document.title)}if(document.readyState==='complete'){_dt()}else{document.addEventListener('DOMContentLoaded',_dt)}new MutationObserver(_dt).observe(document.documentElement||document,{childList:true,subtree:true,attributeFilter:['title']});",
                        );
                    }
                    let mut builder = wry::WebViewBuilder::new()
                        .with_url(&spec.url)
                        .with_bounds(bounds)
                        .with_visible(spec.visible)
                        // 关掉 macOS 的链接预览 force-click/long-press peek 浮层。
                        // (这只影响长按/重压预览,不影响普通单击跳转;普通单击
                        // 跳转真正缺的那块是下方的 `on_page_load`/`new_window_req`
                        // 回报钩子。)
                        .with_allow_link_preview(false)
                        // 子 webview 上的 mousedown winit 收不到,这里注入 JS
                        // 在捕获阶段监听 mousedown,经 IPC 通知宿主调 view.focus()
                        // 让 WKWebView 成为 first responder(否则 ⌘C 选区复制
                        // 走不通:WKWebView 不是 first responder 时 keyDown 不到它)。
                        // 顺带监听 mouseup:页签拖拽拖进 webview 后在这里松开,
                        // winit 收不到 `Released`,靠这条 IPC 结束拖拽
                        // (`WebViewMouseUp`)。
                        .with_initialization_script(script)
                        .with_ipc_handler(move |_req| {
                            let body = _req.body().as_str();
                            match body {
                                "mouseup" => {
                                    let _ = ipc_proxy.send_event(Message::WebViewMouseUp);
                                }
                                // 子 webview 抢到键盘焦点(首次点击后 WKWebView
                                // 成为 first responder)时,Ctrl++/-/1 这类全局缩放
                                // 快捷键不会再回到 winit,会被 webview 自己吃掉。
                                // 这里在 keydown 捕获阶段拦下这几个组合键、转发给
                                // 宿主走 `icon_size::zoom_by`,使首页/预览/浏览器
                                // 各种 webview 聚焦态下缩放都和终端/自绘面板一致。
                                "zoom_in" => {
                                    let _ = ipc_proxy.send_event(Message::ZoomIn);
                                }
                                "zoom_out" => {
                                    let _ = ipc_proxy.send_event(Message::ZoomOut);
                                }
                                "zoom_reset" => {
                                    let _ = ipc_proxy.send_event(Message::ZoomReset);
                                }
                                // 浏览器面板报回页面 HTML 标题:`title:<id>:<title>`。
                                // `splitn(2, ':')` 只拆第一个冒号,标题里再带冒号
                                // 也不被误拆。空标题页面仍发,由 `browser::update`
                                // 的空标题守卫决定不覆盖 URL 标题。
                                title_msg if title_msg.starts_with("title:") => {
                                    let rest = &title_msg["title:".len()..];
                                    let mut it = rest.splitn(2, ':');
                                    if let (Some(id_s), Some(title)) = (it.next(), it.next())
                                        && let Ok(id) = id_s.parse::<usize>()
                                    {
                                        let _ = ipc_proxy.send_event(Message::BrowserTitle(
                                            id,
                                            title.to_string(),
                                        ));
                                    }
                                }
                                _ => {
                                    let _ = ipc_proxy.send_event(Message::WebViewFocused);
                                }
                            }
                        });
                    // 浏览器面板 webview 额外挂两个导航回报钩子(预览 webview
                    // 不需要):
                    // 1) `on_page_load`:网页内超链接让 webview 自行导航后,把
                    //    真实目标 URL 报回宿主(`Message::BrowserNavigated`),让
                    //    地址栏/页签跟随页面跳转——此前没有任何机制告诉浏览器
                    //    面板"页面自己跳走了",点链接后面板仍停在旧 URL;
                    //    2) `new_window_req`:`target="_blank"`/`window.open`
                    //    请求新窗口,wry 默认 `createWebViewWith:` 返回 nil 会
                    //    静默丢弃这类点击。这里拒绝 OS 新窗口、改在浏览器面板
                    //    里新开一个 tab。
                    if report_title {
                        // `on_page_load` 与 `new_window_req` 是两个 `move` 闭包,
                        // 不能共用同一个 `nav_proxy`(第一个构造时就把原始值移走),
                        // 这里各留一份 clone。
                        let page_proxy = nav_proxy.clone();
                        let win_proxy = nav_proxy;
                        builder = builder
                            .with_on_page_load_handler(move |event, url| {
                                // 只在整页**加载完成**时回报一次(`Finished` 对应
                                // WKWebView `didFinishNavigation`,第二个参数就是
                                // 主 frame 的当前地址,不会被子 frame/资源加载
                                // 刷屏)。`Started` 也会触发主 frame 提交,但完成态
                                // 更稳,避免回报过早。
                                if matches!(event, wry::PageLoadEvent::Finished) {
                                    let _ = page_proxy
                                        .send_event(Message::BrowserNavigated(webview_id, url));
                                }
                            })
                            .with_new_window_req_handler(move |url, _features| {
                                let _ = win_proxy.send_event(Message::BrowserNewWindow(url));
                                wry::NewWindowResponse::Deny
                            });
                    }
                    let built = builder
                        .with_custom_protocol("dozer".into(), move |_id, request| {
                            let allowed = allowed.lock().expect("allowed_files 锁");
                            let review_data = review_snapshot.lock().expect("review snapshot 锁");
                            let reply = assets::handle_protocol(
                                &root,
                                &allowed,
                                review_data.as_deref(),
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
                            let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
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
        /// 清除文件树拖拽高亮(`drag_hover` 置空)。拖拽取消/落下但不在树上时
        /// 调用,让上一帧金框高亮立刻消失(否则树会一直亮着直到下一次 hover)。
        fn clear_file_drag_hover(app: &mut App) {
            app.update(Message::Files(extensions::files::Message::FileDragHover(
                std::collections::HashSet::new(),
            )));
        }

        fn on_window_event(&mut self, event: &WindowEvent) {
            // 把 `modifiers` 和 `app`/`window` 放进同一次解构里取，
            // 避免先借一次 `self` 再调用 `&self` 方法造成的重复借用。
            let Self::Ready {
                app,
                window,
                modifiers,
                clipboard,
                cursor_phys,
                files_dragging,
                pending_focus,
                current_focus,
                ..
            } = self
            else {
                return;
            };

            // 光标位置跟踪 + 点击焦点路由（验收反馈 2/失焦回正常态）:
            // 任一左键点击先退出剩余自绘输入编辑态;再按落点决定键盘归谁。
            match event {
                WindowEvent::CursorMoved { position, .. } => {
                    *cursor_phys = *position;
                    // 记录光标逻辑坐标供 Todo 日历浮层当弹出锚点(点日历按钮
                    // 时光标正好在按钮上,等价"按钮旁边"),镜像右键菜单用的
                    // `last_right_click` 套路。
                    let scale = window.scale_factor();
                    app.last_cursor = (
                        (cursor_phys.x / scale) as f32,
                        (cursor_phys.y / scale) as f32,
                    );
                    // 外部文件拖拽悬停:实时 re-hit-test 文件树目录行,把
                    // 命中结果作为 `FileDragHover` 刷给 `drag_hover`,驱动
                    // 目录行整行金色高亮(用户要求的"拖拽时实时高亮")。命中
                    // 不到目录就清空高亮。拖拽在树上移动时 iced 不重绘
                    // MouseArea,必须靠这里主动请求重绘。
                    if *files_dragging {
                        let scale = window.scale_factor();
                        let logical_x = (cursor_phys.x / scale) as f32;
                        let logical_y = (cursor_phys.y / scale) as f32;
                        let window_width = (window.inner_size().width as f64 / scale) as f32;
                        let window_height = (window.inner_size().height as f64 / scale) as f32;
                        let target = app.files_drop_target(
                            window_width,
                            window_height,
                            logical_x,
                            logical_y,
                        );
                        let hover = target
                            .into_iter()
                            .collect::<std::collections::HashSet<std::path::PathBuf>>();
                        app.update(Message::Files(extensions::files::Message::FileDragHover(
                            hover,
                        )));
                        window.request_redraw();
                    }
                    if app.dragging_divider().is_some() {
                        let scale = window.scale_factor();
                        let logical_x = (cursor_phys.x / scale) as f32;
                        let window_width = (window.inner_size().width as f64 / scale) as f32;
                        app.update(Message::ColumnDrag {
                            window_width,
                            logical_x,
                        });
                    }
                    // 纵向(上下)拖拽同理:按窗口逻辑高换算 cursor y。
                    if app.dragging_row().is_some() {
                        let scale = window.scale_factor();
                        let logical_y = (cursor_phys.y / scale) as f32;
                        let window_height = (window.inner_size().height as f64 / scale) as f32;
                        app.update(Message::RowDrag {
                            window_height,
                            logical_y,
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
                    let x =
                        x.min((window_w - byteui::theme::geometry::context_menu_width()).max(0.0));
                    let y =
                        y.min((window_h - byteui::theme::geometry::context_menu_height()).max(0.0));
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
                    // 文件预览与浏览器分别挂在 `Files`/`Project`/`Web` 面板上,
                    // 可能已拖到任一栏:`is_in_preview_column` 返回命中的面板,
                    // 据此区分交给哪个 webview 池(浏览器池 vs 预览池)。
                    let intent = match webview_geometry::is_in_preview_column(
                        logical_x, logical_w, &state,
                    ) {
                        Some(PanelKind::Web) => FocusIntent::Browser,
                        Some(kind) => FocusIntent::Preview(kind),
                        None => FocusIntent::Terminal,
                    };
                    *pending_focus = Some(intent);
                    *current_focus = intent;
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
                // 纵向拖拽松开左键同理。
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.dragging_row().is_some() => {
                    app.update(Message::RowDragEnd);
                    window.request_redraw();
                }
                // 页签拖拽换位同理:左键松开即结束(不需要位置续传,CursorMoved
                // 里没有对应分支——换位是靠被拖过 tab 的 on_move 驱动的)。
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.dragging_tab().is_some() => {
                    app.update(Message::TabDragEnd);
                    window.request_redraw();
                }
                // 图标栏拖拽换栏同理:左键松开即结束,换栏/换位是靠被拖
                // 过按钮的 on_move 驱动的,这里只负责收尾。
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.dragging_rail() => {
                    app.update(Message::RailDragEnd);
                    window.request_redraw();
                }
                // Todo 面板拖拽排序同理:左键松开即结束并把新顺序写盘(换位
                // 是靠被拖过卡片的 `on_move` 驱动的,这里只负责收尾)。
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.todo_dragging() => {
                    app.update(Message::TodoDragEnd);
                    window.request_redraw();
                }
                // 外部 OS 文件拖拽进入窗口:进入即置拖拽标记,之后每个
                // `CursorMoved` 都会 re-hit-test 树并刷新高亮(见上面
                // CursorMoved 分支);离开窗口/取消时清标记并收起高亮。
                WindowEvent::HoveredFile(_) => {
                    *files_dragging = true;
                }
                WindowEvent::HoveredFileCancelled => {
                    *files_dragging = false;
                    Self::clear_file_drag_hover(app);
                    window.request_redraw();
                    return;
                }
                // 外部 OS 文件拖拽落下:优先落到文件树目录行 → 触发移动
                // (吸收掉,不再进后面的终端字节分发);否则落给终端现状行为
                // (不 return,继续走 bytes 匹配的 `DroppedFile` 分支)。
                WindowEvent::DroppedFile(path) if *files_dragging => {
                    let scale = window.scale_factor();
                    let logical_x = (cursor_phys.x / scale) as f32;
                    let logical_y = (cursor_phys.y / scale) as f32;
                    let window_w = (window.inner_size().width as f64 / scale) as f32;
                    let window_h = (window.inner_size().height as f64 / scale) as f32;
                    let target = app.files_drop_target(window_w, window_h, logical_x, logical_y);
                    *files_dragging = false;
                    if let Some(target) = target {
                        app.update(Message::Files(extensions::files::Message::FileDrop {
                            paths: vec![path.clone()],
                            target,
                        }));
                        window.request_redraw();
                        return;
                    }
                    Self::clear_file_drag_hover(app);
                }
                _ => {}
            }
            // 右键"搜索"弹窗打开时,Esc 优先:任何时候直接关整个弹窗(迁移到
            // 原生 text_input 后查询框聚焦态由 iced 自己管,不再有"编辑态"
            // 这个中间态,不再区分先退编辑态再关弹窗的两级行为)。不放靠后
            // 位置以免被终端当普通按键消费掉。
            if app.search_popup_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::Search(extensions::search::Message::SearchClose));
                window.request_redraw();
                return;
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
                // 预览 tab 右键菜单与文件树右键菜单互斥——关掉当前开着的那个。
                if app.preview_tab_context_menu_open() {
                    app.update(Message::PreviewTabContextMenuClose);
                } else if app.project_preview_tab_context_menu_open() {
                    app.update(Message::ProjectPreviewTabContextMenuClose);
                } else if app.project_link_context_menu_open() {
                    app.update(Message::ProjectLinkContextMenuClose);
                } else if app.category_context_menu_open() {
                    app.update(Message::CategoryContextMenuClose);
                } else if app.category_picker_open() {
                    app.update(Message::CategoryPickerClose);
                } else if app.text_input_menu_open() {
                    app.update(Message::TextInputMenuClose);
                } else if app.database_source_context_menu_open() {
                    app.update(Message::DatabaseSourceContextMenuClose);
                } else {
                    app.update(Message::Files(extensions::files::Message::ContextMenuClose));
                }
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

            // 顶栏新增项目菜单打开时,Esc 同样优先关菜单,口径同上面的
            // agent 选择菜单。
            if app.project_add_menu_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::ProjectAddMenuClose);
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

            // Todo 状态(待办/进行中/搁置/已完成)下拉选择层打开时,Esc 优先
            // 关掉弹出层,口径同上面 agent 选择菜单。
            if app.todo_status_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::Todo(extensions::todo::Message::StatusClose));
                window.request_redraw();
                return;
            }

            // Todo 日历日期选择器打开时,Esc 同样优先关掉弹出层,口径同上面
            // 的 Todo 派发选择层。
            if app.todo_calendar_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::Todo(extensions::todo::Message::CalendarClose));
                window.request_redraw();
                return;
            }

            // Todo 搜索框左前"状态"筛选浮层打开时,Esc 同样优先关掉弹出层,
            // 口径同上面 Todo 日历选择器。
            if app.todo_status_filter_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::Todo(extensions::todo::Message::StatusFilterClose));
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
            // 终端转发——弹层里的 `iced-code-editor` 走标准 iced 事件管线
            // (键盘事件经 `Canvas` widget 的 `on_event` 自己消化),这里不需要
            // 也不应该手工转发。不加这道闸门的话,`terminal_visible()` 只看右侧
            // 是否展开、对弹层状态一无所知,默认布局(右侧终端可见)下弹层
            // 里打的每个字符、包括回车,都会同时写进背后那个终端/agent 会话
            // (Critical,code review 发现)。
            if app.edit_session_open() {
                return;
            }

            // 原生预览 tab(白名单扩展名,`preview.rs` 直接画 `CodeEditor`,
            // 不再是旧版 wry 预览那种能抢走 OS 级键盘焦点的子视图)打开且
            // 键盘焦点确实在预览列时,同上一道闸门的道理放行——键盘事件走
            // 标准 iced 管线直达 `CodeEditor`,不再往下落进 ⌘ 快捷键/终端
            // 转发分支。多了 `current_focus == Preview` 这层判断是因为原生
            // 预览不是模态弹层:用户切去终端敲字时,背景里开着的原生预览
            // tab 不该继续偷键盘(不加这层判断会把这类按键错误地拦在这里,
            // 而不是送进终端)。`current_focus` 携带的 `PanelKind` 决定查
            // `Files` 还是 `Project` 那个 `PreviewPane`——此前恒查 Files,
            // Project 预览面板里的原生编辑器 tab 一直收不到键盘。
            if let FocusIntent::Preview(kind) = *current_focus
                && app.active_preview_tab_has_native_editor(kind)
            {
                return;
            }

            // Files 搜索框(Stage 2)/ 浏览器地址栏(Stage 3)/ Todo 搜索框、
            // 新增任务框、首页项目搜索框(Stage 4),都已迁移到 iced 原生
            // text_input/text_editor 且每帧查真实焦点 / SSH·Database 连接
            // 表单(字段本来就是真 `text_input`,只是补上一直缺失的路由放行
            // 判断):命中就直接放行给标准 iced 事件转换管线,交真正的
            // text_input/text_editor 自己处理光标/选区/IME(同上面 Preview
            // 原生编辑器那道闸门的手法)。SSH/Database 用"表单是否打开"这个
            // 粗粒度信号(不像其它几个原生字段要每帧查真实焦点),表单打开时
            // 整体放行,不区分表单内具体哪个字段聚焦。必须放在 ⌘ 组合键
            // 判断(下方 `modifiers.super_key()` 分支)之前,否则 ⌘V 粘贴会
            // 被错误地转发进终端而不是交给 text_input 自己内置的粘贴处理。
            if app.files_search_focused()
                || app.browser_addr_focused()
                || app.todo_search_focused()
                || app.todo_add_focused()
                || app.todo_content_focused()
                || app.category_rename_focused()
                || app.tree_edit_focused()
                || app.home_project_search_focused()
                || app.conversation_search_focused()
                || app.git_log_search_focused()
                || app.ssh_form_open()
                || app.database_form_open()
                || app.comment_focused()
                || app.project_name_focused()
                || app.query_focused()
            {
                return;
            }

            // Todo/浏览器/文件树/项目树等面板输入均已迁移 iced 原生
            // text_input/text_editor,走上面那道独立的原生放行闸门,不再在此列。
            // 提到 ⌘ 组合键判断之前,因为 ⌘V 粘贴也要认这套聚焦态(见下方 fix)。

            // ⌘ 组合键是应用级快捷键，一律不进 PTY(此前 ⌘C 会把裸 "c"
            // 漏写进终端)。⌘C 复制当前选区；⌘V 粘贴剪贴板——粘贴落进正在
            // 聚焦的 iced 原生输入(经原生放行闸门),未聚焦则走 `TermPaste`。
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
                                let target = app.keyboard_term_target();
                                app.update(Message::TermPaste(target, text));
                                window.request_redraw();
                            } else if let Some(path) =
                                clipboard_image::read_pasteboard_image_as_temp_file()
                            {
                                // 剪贴板没有文本表示(纯截图),iced 的
                                // Clipboard::read 只认字符串,取不到图片
                                // 字节。落临时 PNG,粘贴文件路径——claude
                                // 等 CLI 会把路径识别成图片附件加载。
                                let target = app.keyboard_term_target();
                                app.update(Message::TermPaste(
                                    target,
                                    path.to_string_lossy().into_owned(),
                                ));
                                window.request_redraw();
                            }
                        }
                        _ => {}
                    }
                }
                return;
            }

            // IME 组字预览(尚未提交):不发字节给 PTY,只更新一份渲染态给
            // `term_view` 在光标处画预览文字(见 `Message::TermImePreedit`
            // 文档)。空字符串表示组字结束/取消,清空预览。上面的原生
            // text_input 放行闸门已经把这类字段的 Ime 事件挡在了本函数
            // 之外(它们各自走 iced 标准事件管线自己处理 IME),这里能
            // 到达的都是终端目标。
            if let WindowEvent::Ime(Ime::Preedit(text, _range)) = event {
                let target = app.keyboard_term_target();
                let preedit = if text.is_empty() {
                    None
                } else {
                    Some(text.clone())
                };
                app.update(Message::TermImePreedit(target, preedit));
                window.request_redraw();
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
                let target = app.keyboard_term_target();
                tracing::warn!(?target, ?bytes, "DEBUG term input fallback fired");
                app.update(Message::TermInput(target, bytes));
                // 提交即组字结束:清掉预览态,避免上一段预览文字残留在
                // 光标位置(下一次 `Ime::Preedit` 到来前的空窗期)。
                if matches!(event, WindowEvent::Ime(Ime::Commit(_))) {
                    app.update(Message::TermImePreedit(target, None));
                }
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
                webview_rects,
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

            // 每帧重算可见 webview 矩形,供 cursor 更新做命中测试
            // (参见 `webview_rects` 字段文档)。
            webview_rects.clear();

            let size = window.inner_size();
            let scale = window.scale_factor();
            let logical_w = size.width as f32 / scale as f32;
            let logical_h = size.height as f32 / scale as f32;
            // 预览 webview 是原生子视图,无视 iced 绘制顺序径直叠在最上——
            // 曾经试过"菜单盖到预览区就把 webview 藏成零矩形"讨好右键菜单,
            // 但预览区窄、菜单定宽,几乎任何右键都会让菜单探进预览区,
            // 表现成"预览要么整体消失要么必须在最顶层"。已按要求退回原状:
            // webview 照常显示,右键菜单会被它盖住。
            // `preview_desired` 已经按左右两侧各自算好矩形,直接喂给
            // `sync_webview_pool`(`Files`/`Project` 可分居两侧同时活跃)。
            sync_webview_pool(
                window.as_ref(),
                webviews,
                app.preview_desired(logical_w, logical_h)
                    .into_iter()
                    .map(|(s, (x, y, w, h))| {
                        webview_rects.push((x, y, w, h));
                        (
                            s,
                            wry::Rect {
                                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
                app.allowed_files(),
                app.review_snapshot(),
                proxy.clone(),
                // 预览 webview 不需要回报页面标题,只有浏览器面板要。
                false,
            );
            // 首页右栏浏览器(`home_browser`)占的是右面板区,不是工作区的左
            // 面板预览区,所以单独算一套边界(见 `home_browser_bounds`);工作区
            // 浏览器 2026-08-11 曾短暂迁到右面板,同日已按用户要求移回左面板区
            // (`PanelKind::Web`)。`browser_desired` 非首页时已按 `Web` 所在侧
            // 算好矩形;首页分支带的是占位 `(0,0,0,0)`,这里用 `home_browser_bounds`
            // 整体覆盖。
            let browser_specs: Vec<(preview::WebviewSpec, wry::Rect)> = if app.is_home() {
                let (r, bounds) = Self::home_browser_bounds(logical_w, logical_h);
                webview_rects.push(r);
                app.browser_desired(logical_w, logical_h)
                    .into_iter()
                    .map(|(s, _)| (s, bounds))
                    .collect()
            } else {
                app.browser_desired(logical_w, logical_h)
                    .into_iter()
                    .map(|(s, (x, y, w, h))| {
                        webview_rects.push((x, y, w, h));
                        (
                            s,
                            wry::Rect {
                                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
                            },
                        )
                    })
                    .collect()
            };
            sync_webview_pool(
                window.as_ref(),
                browser_webviews,
                browser_specs,
                app.allowed_files(),
                app.review_snapshot(),
                proxy.clone(),
                // 浏览器面板 webview 注入页面标题回报,tab 加载完成后标题
                // 从 URL 切到 HTML `<title>`。
                true,
            );
        }

        /// 首页右栏全局浏览器(`home_browser`)的 webview 像素边界:占满右侧
        /// 面板区(左图标栏 + 首页侧栏 + 分隔线之后,到右侧图标栏之前),扣掉
        /// right_zone 的 margin、顶栏/footbar 高度与浏览器地址栏高度。原生
        /// 子视图不听 iced 布局,逐像素算(同 `preview_content_bounds` 的思路)。
        /// 返回逻辑 `(x, y, w, h)`,并顺带给出 `wry::Rect` 供摆放。
        fn home_browser_bounds(
            window_width: f32,
            window_height: f32,
        ) -> ((f32, f32, f32, f32), wry::Rect) {
            let rail = byteui::theme::geometry::icon_rail_width();
            let sidebar = byteui::theme::geometry::h0_sidebar_width();
            let divider = byteui::theme::geometry::divider_width();
            let chrome = byteui::theme::geometry::browser_chrome_top_px();
            let m = theme::region::right_zone().margin;
            let x = rail + sidebar + divider + m.left + 8.0;
            let w =
                (window_width - rail - sidebar - divider - rail - m.left - m.right - 16.0).max(0.0);
            let y = byteui::theme::geometry::top_bar_height() + m.top + chrome;
            let h = (window_height
                - byteui::theme::geometry::top_bar_height()
                - byteui::theme::geometry::status_bar_height()
                - m.top
                - m.bottom
                - chrome
                - 8.0)
                .max(0.0);
            (
                (x, y, w, h),
                wry::Rect {
                    position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                    size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
                },
            )
        }

        /// 两个 `(x, y, w, h)` 逻辑矩形是否重叠(含恰好边贴边的情况)。用于
        /// 判断右键菜单是否盖到预览 webview——重叠才把 webview 藏零,避免
        /// 菜单没伸进预览区时把整片预览误藏。
        /// `ProjectTabPickFolder`/`ProjectTreeCopyPath` 等需要窗口句柄侧原生
        /// 能力(rfd 模态、系统剪贴板)的消息在此拦截,其余原样转给
        /// `app.update`。
        fn dispatch(&mut self, message: Message) {
            let Self::Ready {
                app,
                window,
                cursor_phys: _,
                pending_focus,
                current_focus,
                clipboard,
                browser_webviews,
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
                *pending_focus = Some(FocusIntent::Preview(PanelKind::Files));
                *current_focus = FocusIntent::Preview(PanelKind::Files);
            } else if matches!(
                message,
                Message::ProjectPreviewOpenPath(_) | Message::ProjectPreviewSelectTab(_)
            ) {
                *pending_focus = Some(FocusIntent::Preview(PanelKind::Project));
                *current_focus = FocusIntent::Preview(PanelKind::Project);
            } else if matches!(
                message,
                Message::Browser(extensions::browser::Message::OpenUrl(_))
                    | Message::Browser(extensions::browser::Message::SelectTab(_))
            ) {
                *pending_focus = Some(FocusIntent::Browser);
                *current_focus = FocusIntent::Browser;
                // 没经过鼠标点击的 `blur_inputs()`,原生预览编辑器不会自己
                // 让出焦点——见 `App::blur_preview_editors` 的说明。
                app.blur_preview_editors();
            } else if matches!(
                message,
                Message::SelectTab(_)
                    | Message::SelectTabNoDrag(_)
                    | Message::TabAttached(_, _, _, _)
                    | Message::AgentPickerSelect(_)
            ) {
                *pending_focus = Some(FocusIntent::Terminal);
                *current_focus = FocusIntent::Terminal;
                app.blur_preview_editors();
            } else if matches!(message, Message::WebViewFocused) {
                // 子 webview 上的 mousedown winit 收不到,JS 经 IPC 发来这条
                // 消息。它不携带面板信息(预览池/浏览器池共用同一 IPC 代理),
                // 只能从状态反推:先看左/右栏哪个面板是 `Web`(→ 浏览器池),
                // 否则落到预览池——`Files` 预览 webview 在左栏、`Project`
                // 预览 webview 在右栏可各自独立存在,需要 `is_in_preview_column`
                // 那样按侧判断,但这里没有鼠标坐标,只能用
                // `active_preview_panel_kind()`(见其注释:双面板同时活跃时
                // 优先 `Files`)。
                let state = app.shell_state();
                let intent =
                    if state.left_view == PanelKind::Web || state.right_view == PanelKind::Web {
                        FocusIntent::Browser
                    } else if let Some(kind) = app.active_preview_panel_kind() {
                        FocusIntent::Preview(kind)
                    } else {
                        FocusIntent::Terminal
                    };
                *pending_focus = Some(intent);
                *current_focus = intent;
                // `WebViewFocused` 恒是某个 wry webview 收到了焦点(原生
                // `iced-code-editor` tab 走的是另一条渲染路径,不会发这条
                // 消息)——不管 `intent` 落在 Preview 还是 Browser,收到焦点
                // 的都不是原生编辑器,它该让出焦点。
                app.blur_preview_editors();
            }
            match message {
                // `iced-code-editor` 的内部消息:编辑器产生的 `iced::Task`(剪贴板
                // 读写/搜索框聚焦)需要在持有 `Clipboard` 句柄的这里执行,不能走
                // `app.update`(其返回 `()`,无运行时)。桥接器把 Task 里的
                // 副作用(剪贴板)落到系统剪贴板,把 `Output` 子消息递归回灌编辑器。
                Message::EditorEvent(event) => {
                    Self::run_editor_task(app, clipboard, event);
                    window.request_redraw();
                }
                Message::PreviewEditorEvent(tab_id, event) => {
                    Self::run_preview_tab_editor_task(app, clipboard, tab_id, event, false);
                    window.request_redraw();
                }
                Message::ProjectPreviewEditorEvent(tab_id, event) => {
                    Self::run_preview_tab_editor_task(app, clipboard, tab_id, event, true);
                    window.request_redraw();
                }
                // 顶栏"＋"与项目栏"打开项目…"共用的唯一打开入口:rfd 模态选中
                // 后一律落成**新增页签**(`ProjectTabOpen`)。此前项目栏那颗按钮
                // 另有一条 `ProjectPickFolder`→`ProjectOpen` 的就地改写路径,
                // 会杀掉当前项目的全部会话,已删除。
                Message::ProjectTabPickFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        app.update(Message::ProjectTabOpen(dir));
                    }
                }
                Message::ProjectLinkPick(target) => {
                    // 单颗"＋"入口:打开根目录在项目根的文件浏览器,选中后按
                    // 实际类型(`is_dir()`)判定虚拟链接是该当文件还是目录,再回
                    // 送 `LinkAdd` 落盘。macOS 用原生 `NSOpenPanel` 同时放行
                    // 文件与目录;其它平台 rfd 无"文件+文件夹"双兼容开关,回退
                    // 到 rfd 的 `pick_file`(仍有 `path.is_dir()` 分支兜底)。
                    let start_dir = app
                        .active_project_path()
                        .or_else(|| std::env::current_dir().ok())
                        .unwrap_or_default();

                    #[cfg(target_os = "macos")]
                    let picked = pick_file_or_dir(&start_dir);
                    #[cfg(not(target_os = "macos"))]
                    let picked = rfd::FileDialog::new().set_directory(&start_dir).pick_file();

                    if let Some(path) = picked {
                        let kind = if path.is_dir() {
                            extensions::project::links::LinkKind::Dir
                        } else {
                            extensions::project::links::LinkKind::File
                        };
                        app.update(Message::Project(extensions::project::Message::LinkAdd {
                            target,
                            path,
                            kind,
                        }));
                        window.request_redraw();
                    }
                }
                Message::Files(extensions::files::Message::CopyPath(path, kind)) => {
                    let root = app.active_project_path().unwrap_or_else(|| path.clone());
                    let s = crate::project::path_string(kind, &path, &root);
                    clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
                    app.update(Message::Files(extensions::files::Message::ContextMenuClose));
                    window.request_redraw();
                }
                // 点击搜索弹窗里的命中行:由窗口句柄侧拦截,映射回预览域打开
                // (文件预览需要 `allowed_files` 白名单与 tree 高亮,走
                // `App::update` 的 `PreviewOpenPath` 最合适)。权限/焦点一并
                // 处理,并关掉搜索弹窗——"挑中即落地预览",不留浮层悬浮(同
                // 其它弹层互斥清理口径)。
                Message::Search(extensions::search::Message::Pick(hit)) => {
                    app.update(Message::Search(extensions::search::Message::SearchClose));
                    *pending_focus = Some(FocusIntent::Preview(PanelKind::Files));
                    *current_focus = FocusIntent::Preview(PanelKind::Files);
                    app.update(Message::PreviewOpenPath(hit.path));
                    window.request_redraw();
                }
                // 浏览器后退/前进/刷新:webview 句柄只在 main.rs 的浏览器池里,
                // 浏览器 `State` 摸不到——在这里对激活 webview 直接执行历史
                // 导航/刷新。wry 0.55 没暴露 `go_back`/`go_forward`,后退/前进
                // 用 `window.history` JS 兜底;刷新走原生 `reload()`。
                Message::Browser(extensions::browser::Message::Nav(action)) => {
                    if let Some(id) = app.active_browser_webview_id()
                        && let Some((view, _)) = browser_webviews.get_mut(&id)
                    {
                        match action {
                            extensions::browser::NavAction::Back => {
                                let _ = view.evaluate_script("window.history.back()");
                            }
                            extensions::browser::NavAction::Forward => {
                                let _ = view.evaluate_script("window.history.forward()");
                            }
                            extensions::browser::NavAction::Refresh => {
                                let _ = view.reload();
                            }
                        }
                    }
                }
                // 新增任务框已迁 iced 原生 `text_editor`(Stage 4),点击命中
                // 区域内由组件自身接管聚焦与光标定位,不再有 `AddEditStart`/
                // `AddCursorAt` 自绘换算分支。任务内容编辑框已迁 iced 原生
                // `text_input`(Stage 5):进入编辑态由卡片内 `MouseArea` 直接
                // 派发 `ContentEditStart`,编辑态内的点击/光标定位/caret 全由
                // 原生控件接管,不再有 `ContentCursorAt`/bounds 换算分支。
                other => app.update(other),
            }
            window.request_redraw();
        }

        /// 执行 `iced-code-editor` 产生的 `iced::Task`。dozer 的 `App::update`
        /// 返回 `()`、没有 iced 运行时,编辑器把剪贴板读写等副作用包成
        /// `Task` 委托给宿主——这里手动把 Task 拆成 `Action` 流,把剪贴板
        /// 动作落到系统剪贴板(持有 `Clipboard` 句柄的只有 `Runner`),把
        /// `Output` 子消息(如剪贴板读到的 `Paste(text)`)递归回灌编辑器。
        ///
        /// 普通编辑路径(打字/删改/移动)产生的 Task 恒为 `Task::none()`,这里
        /// 直接跳过;只有复制/剪切/粘贴会真正走剪贴板分支。
        fn run_editor_task(
            app: &mut App,
            clipboard: &mut Clipboard,
            event: iced_code_editor::Message,
        ) {
            use iced_winit::futures::futures::stream::StreamExt;
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(event);
            while let Some(ev) = queue.pop_front() {
                let t = app.preview_edit_event(ev);
                let Some(stream) = task::into_stream(t) else {
                    continue;
                };
                // 把这一轮 Task 流里的 `Output` 子消息收集起来,剪贴板动作
                // 同步落到系统剪贴板。流里可能有 `yield_now` 占位项,被
                // `filter_map` 跳过,不影响。
                let mut outputs: Vec<iced_code_editor::Message> = Vec::new();
                futures::futures::executor::block_on(async {
                    let mut stream = stream;
                    while let Some(action) = stream.next().await {
                        match action {
                            Action::Output(m) => outputs.push(m),
                            Action::Clipboard(cb) => match cb {
                                ClipboardAction::Read { target, channel } => {
                                    let text = clipboard.read(target);
                                    let _ = channel.send(text);
                                }
                                ClipboardAction::Write { target, contents } => {
                                    clipboard.write(target, contents);
                                }
                            },
                            _ => {}
                        }
                    }
                });
                for m in outputs {
                    queue.push_back(m);
                }
            }
        }

        /// 同 `run_editor_task`,但把消息转发给某个原生预览 tab(按 `tab_id`)而不是
        /// 编辑弹层的单一 `edit_session`。两个函数体基本重复——保持"预览/编辑分层"
        /// 这条既定决策(见设计文档),不引入一个把两种目标都塞进同一签名的抽象。
        /// `project` 为 true 时路由到 Project 面板右配对预览
        /// (`ProjectPreviewEditOpenByTab`/`app.project_preview_tab_editor_event`),
        /// 否则走 Files 预览那套。
        fn run_preview_tab_editor_task(
            app: &mut App,
            clipboard: &mut Clipboard,
            tab_id: usize,
            event: iced_code_editor::Message,
            project: bool,
        ) {
            use iced_winit::futures::futures::stream::StreamExt;
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(event);
            while let Some(ev) = queue.pop_front() {
                // 只读预览的右键"编辑"项:不入编辑器(编辑器是只读的,内部也没有
                // 对应处理),直接转成本体消息打开该 tab 的编辑浮层。
                if let iced_code_editor::Message::OpenInEditor = ev {
                    if project {
                        app.update(Message::ProjectPreviewEditOpenByTab(tab_id));
                    } else {
                        app.update(Message::PreviewEditOpenByTab(tab_id));
                    }
                    continue;
                }
                let t = if project {
                    app.project_preview_tab_editor_event(tab_id, ev)
                } else {
                    app.preview_tab_editor_event(tab_id, ev)
                };
                let Some(stream) = task::into_stream(t) else {
                    continue;
                };
                let mut outputs: Vec<iced_code_editor::Message> = Vec::new();
                futures::futures::executor::block_on(async {
                    let mut stream = stream;
                    while let Some(action) = stream.next().await {
                        match action {
                            Action::Output(m) => outputs.push(m),
                            Action::Clipboard(cb) => match cb {
                                ClipboardAction::Read { target, channel } => {
                                    let text = clipboard.read(target);
                                    let _ = channel.send(text);
                                }
                                ClipboardAction::Write { target, contents } => {
                                    clipboard.write(target, contents);
                                }
                            },
                            _ => {}
                        }
                    }
                });
                for m in outputs {
                    queue.push_back(m);
                }
            }
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
                Some(FocusIntent::Preview(kind)) => match app.active_preview_webview_id(kind) {
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
                let scale = byteui::theme::icon_size::scale() as f64;
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
                let wakes: [(bool, Duration); 5] = [
                    (app.any_hover_anim_active(), HOVER_ANIM_INTERVAL),
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
                ];
                if let Some(interval) = wakes
                    .into_iter()
                    .filter(|(active, _)| *active)
                    .map(|(_, interval)| interval)
                    .min()
                {
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
                install_topbar_drag_guard(&window);

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
                    webview_rects: Vec::new(),
                    cursor_phys: winit::dpi::PhysicalPosition::new(0.0, 0.0),
                    files_dragging: false,
                    pending_focus: None,
                    // 默认终端拿键盘,跟现状(启动时终端可打字、没有任何
                    // 预览/编辑弹层抢焦点)一致。
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
                                    run_operate(&mut interface, renderer, &mut op);
                                }

                                // 项目树行内编辑刚触发时程序化聚焦真正的
                                // `text_input`(一次性位,消费即复位)。
                                if tree_edit_focus_pending {
                                    let mut op =
                                        iced_widget::core::widget::operation::focusable::focus::<()>(
                                            extensions::files::tree_edit_field_id(),
                                        );
                                    run_operate(&mut interface, renderer, &mut op);
                                }

                                // Todo 任务内容编辑刚触发时程序化聚焦真正的
                                // `text_input`(一次性位,消费即复位)。
                                if content_edit_focus_pending {
                                    let mut op =
                                        iced_widget::core::widget::operation::focusable::focus::<()>(
                                            extensions::todo::content_field_id(),
                                        );
                                    run_operate(&mut interface, renderer, &mut op);
                                }

                                // Todo 分类树行内改名刚触发时程序化聚焦真正的
                                // `text_input`(一次性位,消费即复位)。
                                if category_edit_focus_pending {
                                    let mut op =
                                        iced_widget::core::widget::operation::focusable::focus::<()>(
                                            extensions::todo::category_rename_field_id(),
                                        );
                                    run_operate(&mut interface, renderer, &mut op);
                                }

                                // 项目名称编辑 / 右键搜索弹窗查询框刚触发时程序化聚焦
                                // 真正的 `text_input`(一次性位,消费即复位)。
                                if name_edit_focus_pending {
                                    let mut op =
                                        iced_widget::core::widget::operation::focusable::focus::<()>(
                                            extensions::project::name_field_id(),
                                        );
                                    run_operate(&mut interface, renderer, &mut op);
                                }
                                if query_focus_pending {
                                    let mut op =
                                        iced_widget::core::widget::operation::focusable::focus::<()>(
                                            extensions::search::query_field_id(),
                                        );
                                    run_operate(&mut interface, renderer, &mut op);
                                }

                                // 浏览器地址栏刚获得焦点时,全选当前网址(
                                // 单击即选中整条,方便直接覆盖输入)。一次性位,
                                // 消费即复位(取位在 `interface` 构建前完成)。
                                if addr_select_all_pending {
                                    let mut op = iced_widget::core::widget::operation::text_input::select_all::<()>(
                                        extensions::browser::addr_field_id(),
                                    );
                                    run_operate(&mut interface, renderer, &mut op);
                                }

                                // 右键输入框弹菜单后,把焦点移到被右键的输入(
                                // 使菜单的复制/粘贴作用于它)。一次性位,消费即复位。
                                if let Some(id) = input_menu_focus {
                                    let mut op =
                                        iced_widget::core::widget::operation::focusable::focus::<()>(
                                            id,
                                        );
                                    run_operate(&mut interface, renderer, &mut op);
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
                                let files_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Files) {
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::files::CaptureSearchFocus,
                                        );
                                        extensions::files::take_search_focused()
                                    } else {
                                        false
                                    };

                                // 项目树行内编辑框(Stage 5)同款每帧真实焦点查询:
                                // 与 Files 搜索框完全同构。只在 Files 左栏可见时跑。
                                let tree_edit_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Files) {
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::files::CaptureTreeEditFocus,
                                        );
                                        extensions::files::take_tree_edit_focused()
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
                                        run_operate(
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
                                        run_operate(
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
                                        run_operate(
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
                                        run_operate(
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
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::todo::CaptureCategoryRenameFocus,
                                        );
                                        extensions::todo::take_category_rename_focused()
                                    } else {
                                        false
                                    };

                                // 首页项目搜索框(Stage 4):同款每帧查真实焦点态。
                                let home_search_focused = if app.is_home() {
                                    run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut homespace::CaptureHomeSearchFocus,
                                    );
                                    homespace::take_home_search_focused()
                                } else {
                                    false
                                };

                                // 会话列表搜索框:同款每帧查真实焦点态。该面板
                                // (`PanelKind::Conversations`)默认挂右栏,但跟其它
                                // 面板一样可以被拖到左栏(见 `App::rail_cross_apply`),
                                // 所以两侧都要查,不能只看 `left_view()`(同浏览器
                                // 地址栏 `browser_addr_focused` 两侧都查的既有处理)。
                                let conversation_search_focused = if matches!(
                                    app.left_view(),
                                    crate::app::PanelKind::Conversations
                                ) || app.right_view
                                    == crate::app::PanelKind::Conversations
                                {
                                    run_operate(
                                        &mut interface,
                                        renderer,
                                        &mut workspace::CaptureConversationSearchFocus,
                                    );
                                    workspace::take_conversation_search_focused()
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
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::git_log::CaptureSearchFocus,
                                        );
                                        extensions::git_log::take_search_focused()
                                    } else {
                                        false
                                    };

                                // 验收意见框(Stage 6):同款每帧查真实焦点态。
                                let comment_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Acceptance)
                                    {
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::acceptance::CaptureCommentFocus,
                                        );
                                        extensions::acceptance::take_comment_focused()
                                    } else {
                                        false
                                    };

                                // 项目名称编辑框(Stage 6):渲染在 `PanelKind::
                                // Project`,同款每帧查真实焦点态。
                                let name_edit_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Project) {
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::project::CaptureNameEditFocus,
                                        );
                                        extensions::project::take_name_edit_focused()
                                    } else {
                                        false
                                    };

                                // 右键搜索弹窗查询框(Stage 6):全局浮层,不挂靠
                                // 任何 `left_view`,gating 条件用 `search_popup_open`。
                                let query_focused = if app.search_popup_open() {
                                    run_operate(
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
                                    if let Some(icon) =
                                        conversion::mouse_interaction(mouse_interaction)
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
                                    TOPBAR_CONTROL_HOVERED.store(
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
                                                winit::dpi::LogicalPosition::new(
                                                    cursor.x, cursor.y,
                                                ),
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
                                app.set_comment_focused(comment_focused);
                                app.set_project_name_focused(name_edit_focused);
                                app.set_query_focused(query_focused);
                                app.set_conversation_search_focused(conversation_search_focused);
                                app.set_git_log_search_focused(git_log_search_focused);

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
                            run_operate(&mut interface, renderer, &mut op);
                        }
                        let synth = [unique_command_event(ch)];
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
