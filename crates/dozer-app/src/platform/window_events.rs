//! 窗口事件状态机:`Runner` 状态(Loading/Ready)与 `impl Runner`(窗口事件
//! 分发、焦点路由、webview 池同步)。Phase 1 结构重组时从 `main.rs` 抽出,
//! 逻辑保持原样。`impl winit::application::ApplicationHandler<Message>
//! for Runner`(`resumed`/`user_event`/`window_event`)本应同批搬走,当时
//! 漏了——`main.rs` 因此仍有 1498 行,跟 Phase 1"main.rs 收敛"的目标不符,
//! 现在补搬过来,跟同一个 `Runner` 类型的另一半 `impl` 合并到一起。

use std::sync::Arc;
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

use winit::platform::macos::WindowAttributesExtMacOS;
use winit::{
    dpi::LogicalSize,
    event::{ElementState, Ime, WindowEvent},
    event_loop::ControlFlow,
    keyboard::ModifiersState,
};

use crate::app::{App, Message, PanelKind};
use crate::extensions;
use crate::platform::file_history_overlay;
use crate::platform::project_create_overlay;
use crate::platform::search_overlay;
use crate::platform::settings_overlay;
use crate::preview;
use crate::theme;

/// 按 iced 本帧算出的 `mouse_interaction` 刷新窗口光标。
///
/// 这套事件循环是手写的(不是 `iced_winit::program::run`),光标刷新必须
/// **两条路径都走**:`RedrawRequested` 分支画完帧后算一次首屏/存量态,
/// 事件分支(尤其 `CursorMoved`)也必须立刻用 `interface.update` 返回的
/// state 更新一次。少了后者就会出一个很隐蔽的 bug:纯 hover 移动鼠标
/// (不点击、不产生消息、不触发重绘)时 iced 根本不会重算 interaction
/// ——光标形状会停在**进入前**那一片区域的形状上。代码编辑器尤其明显:
/// `text_editor` 悬停时只返回 `Interaction::Text`、不做任何会引发重绘的
/// 动画/hover 消息,于是即使它正确地报了 `Text`,窗口光标也永远刷不出
/// I 形;而页签/按钮有 hover 动画、会触发重绘,看上去"正常",掩盖了缺口。
///
/// 光标落进任一**可见** wry 子视图(预览/浏览器面板)时把控制权让给
/// WKWebView:窗口级 NSCursor 全局唯一、谁最后写谁赢,iced 每帧强刷会把
/// webview 自己的握手/文本光标盖成箭头(见 `webview_rects` 字段文档)。
fn apply_mouse_cursor(
    window: &winit::window::Window,
    app: &App,
    webview_rects: &[(f32, f32, f32, f32)],
    mouse_interaction: mouse::Interaction,
) {
    if let Some(icon) = conversion::mouse_interaction(mouse_interaction) {
        let cursor_over_webview = webview_rects.iter().any(|&(wx, wy, ww, wh)| {
            let (cx, cy) = app.last_cursor;
            ww > 0.0 && wh > 0.0 && cx >= wx && cx <= wx + ww && cy >= wy && cy <= wy + wh
        });
        if !cursor_over_webview {
            window.set_cursor(icon);
            window.set_cursor_visible(true);
        }
    } else {
        window.set_cursor_visible(false);
    }

    // 顶栏原生拖窗守卫复用这同一个每帧算出的 interaction——非 `None`
    // 即悬停在某个可交互控件上(页签/关闭按钮/…),见
    // `install_topbar_drag_guard` 文档。
    #[cfg(target_os = "macos")]
    crate::platform::window::TOPBAR_CONTROL_HOVERED.store(
        mouse_interaction != mouse::Interaction::None,
        std::sync::atomic::Ordering::Relaxed,
    );
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum Runner {
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
        /// 建主窗口 wgpu 资源时用的 `Instance`/`Adapter`,原本只是
        /// `resumed()` 里的局部变量、用完就扔——search overlay 窗口
        /// (`platform/search_overlay.rs`)要另开一个 `Surface`+`Engine`,
        /// 必须用同一个 `Instance` 建 surface、同一个 `Adapter` 建
        /// `Engine`(不能用一个新建的、跟当前 `device`/`queue` 没有血缘
        /// 关系的 `Instance`/`Adapter`),所以补存下来。
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
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
        /// 左键当前是否物理按住(`MouseInput{Pressed/Released, Left}`
        /// 各自置 true/false)。树内拖拽的 `Pending → Dragging` 确认
        /// (`App::maybe_confirm_tree_drag`)额外拿它当硬性前提——纯靠
        /// 存好的按下坐标/时间戳和当前光标比对,万一那次按下对应的
        /// `MouseInput::Released` 因为某种原因没能触发树内拖拽的收尾
        /// (`TreeDragRelease`),`tree_drag` 就会一直卡在 `Pending`,
        /// 之后任何单纯的鼠标悬停(不按键)只要位移/时长凑够阈值,都会
        /// 被误判成"确认了一次拖拽"——2026-09 用户实测反馈并截图:点击
        /// 展开箭头、松开左键后,仅仅轻微移动鼠标(未按住任何键)就冒出
        /// 了跟随光标的幽灵胶囊,该文件"隔空"就能被挪到任意目录。这个
        /// 标志位是自愈:没有物理按住左键,`maybe_confirm_tree_drag`
        /// 直接拒绝确认并顺手清掉任何残留的 `tree_drag`。
        left_mouse_down: bool,
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
        /// 独立原生窗口宿主——`None` 表示当前没开。生命周期由
        /// `sync_search_overlay` 按 `ws.search.is_open()` 单向驱动开/关。
        search_overlay: Option<search_overlay::SearchOverlay>,
        /// 同 `search_overlay`,`file_history` 弹窗的独立窗口宿主。生命
        /// 周期由 `sync_file_history_overlay` 按 `app.file_history.
        /// is_some()` 单向驱动开/关,且与 `search_overlay` 互斥(见
        /// `OverlayKind`/`close_other_overlays`)。
        file_history_overlay: Option<file_history_overlay::FileHistoryOverlay>,
        /// 同 `search_overlay`/`file_history_overlay`,"创建项目"弹窗的
        /// 独立窗口宿主。**不接入失焦关闭**(见 `ProjectCreateOverlay` 文档
        /// 注释),生命周期只由 `sync_project_create_overlay` 按
        /// `app.project_create.is_some()` 驱动。
        project_create_overlay: Option<project_create_overlay::ProjectCreateOverlay>,
        /// 同 `search_overlay`/`file_history_overlay`/`project_create_
        /// overlay`,设置弹窗的独立窗口宿主。生命周期由
        /// `sync_settings_overlay` 按 `app.settings.is_some()` 单向驱动
        /// 开/关,与其余三类互斥。
        settings_overlay: Option<settings_overlay::SettingsOverlay>,
    },
}

/// 点击/消息后决定键盘焦点归谁:预览 webview、浏览器 webview(各自
/// ⌘C 走原生复制)或窗口(终端)。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusIntent {
    Preview(PanelKind),
    Browser,
    Terminal,
}

/// 独立窗口弹窗的种类,给"开一个就关掉其它已开的"这条互斥规则用(见
/// `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
/// design.md`「架构」第 2 节)。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    Search,
    FileHistory,
    ProjectCreate,
    Settings,
}

impl Runner {
    /// 键盘/IME 输入拦截处：终端聚焦时把原始 `WindowEvent` 经
    /// `keymap` 翻译成字节，直接回灌 `App`（`Message::TermInput`），
    /// 不必改动 `winit::application::ApplicationHandler` 的实现本身。
    ///
    /// `App::update` 收到 `TermInput` 后写给 daemon（`client.write`），
    /// 不再本地 echo——回显完全走 PTY 真实回路（daemon → attach 流 →
    /// `Message::TermOutput` → `TerminalModel::feed`）。
    /// 清除文件树拖拽高亮(`drag_hover` 置空)+ 展开计时。拖拽取消/落下
    /// 但不在树上时调用,让上一帧金框高亮立刻消失(否则树会一直亮着
    /// 直到下一次 hover),顺带清掉可能残留的展开计时,避免下一场全新
    /// 拖拽被上一场的残留计时提前触发展开。
    fn clear_file_drag_hover(app: &mut App) {
        app.update(Message::Files(
            crate::extensions::files::Message::FileDragHover(std::collections::HashSet::new()),
        ));
        app.clear_drag_hover_expand();
    }

    /// 返回 `true` 表示这次事件已被"应用级快捷键"完整接管
    /// （已经 `app.update(...)` 落地了对应动作）——调用方
    /// `window_event` 据此跳过随后把**同一个**原始事件再转换喂给
    /// iced 标准管线那一步。不加这道闸门的后果就是本函数"拦下"的
    /// ⌘S/⌘F/⌘R/⌘G 这类组合键会被双重处理:这里已经正确触发了
    /// `PreviewSaveActive` 等 Message,但原始按键事件转换出的 iced
    /// 事件依然会喂给下面的 `interface.update`——官方 `text_editor`
    /// 的默认 `Binding::from_key_press`（`iced_widget` 源码,
    /// `text_editor.rs`）只把 `c`/`x`/`v`/`a` 四个字母认成
    /// Copy/Cut/Paste/SelectAll,其余任何字母只要 `KeyPress.text`
    /// 非空就会落进兜底的 `Some(Self::Insert(c))`,**不检查是否按着
    /// Command**——于是 ⌘S 在真正落盘的同时,那个裸 "s" 字符也被当成
    /// 普通输入插进了正文（2026-09-14 用户反馈"⌘S 会在文件里敲出一个
    /// s"，实测确认）。本函数其它分支(Esc 关各类浮层、Ctrl 缩放、
    /// ⌘C/⌘V、IME 预组字转发终端、外部拖拽文件……)都不吃"字母被当成
    /// 普通字符插入"这个坑(要么不是文本编辑控件的场景,要么 iced 自己
    /// 的 Binding 已经正确特判),继续按原样返回 `false`(不吞、照常
    /// 双喂),只有这条 Preview 原生编辑器闸门真正命中动作时才返回
    /// `true`。
    pub(crate) fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        // 把 `modifiers` 和 `app`/`window` 放进同一次解构里取，
        // 避免先借一次 `self` 再调用 `&self` 方法造成的重复借用。
        let Self::Ready {
            app,
            window,
            modifiers,
            clipboard,
            cursor_phys,
            files_dragging,
            left_mouse_down,
            pending_focus,
            current_focus,
            ..
        } = self
        else {
            return false;
        };

        // 左键物理按住状态,独立于下面按具体拖拽类型分派的 match——见
        // `left_mouse_down` 字段文档:不管后面哪个分支(甚至没有任何
        // 分支)处理这次事件,这个状态都必须先如实同步。
        if let WindowEvent::MouseInput {
            state,
            button: winit::event::MouseButton::Left,
            ..
        } = event
        {
            *left_mouse_down = *state == ElementState::Pressed;
        }

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
                // 树内拖拽 `Pending → Dragging` 确认:每次光标真的移动
                // 都判断一次是否已越过距离+时长两道阈值,且左键必须
                // 真的物理按住(见 `left_mouse_down`/`App::
                // maybe_confirm_tree_drag` 文档——没有这道硬性前提,
                // 万一某次按下对应的松开没能触发树内拖拽收尾,残留的
                // `Pending` 会被之后任何不按键的悬停误判成"确认拖拽")。
                // 转换发生时才重绘——那之后行才会挂 `on_move`/换抓取
                // 光标/画幽灵胶囊,不重绘看不出来;多数时候这个调用是
                // 纯粹的早退,开销可忽略。
                if app.maybe_confirm_tree_drag(*left_mouse_down) {
                    window.request_redraw();
                }
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
                    let hit =
                        app.files_drop_target(window_width, window_height, logical_x, logical_y);
                    // 命中折叠目录时武装展开计时(满 1s 才真正展开,见
                    // `App::arm_drag_hover_expand` 文档);没命中就清空,
                    // 不留残留计时。
                    if let Some(hit) = &hit {
                        app.arm_drag_hover_expand(&hit.target);
                    } else {
                        app.clear_drag_hover_expand();
                    }
                    let hover = hit
                        .into_iter()
                        .map(|h| h.highlight)
                        .collect::<std::collections::HashSet<std::path::PathBuf>>();
                    app.update(Message::Files(
                        crate::extensions::files::Message::FileDragHover(hover),
                    ));
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
                let x = x.min((window_w - byteui::theme::geometry::context_menu_width()).max(0.0));
                let y = y.min((window_h - byteui::theme::geometry::context_menu_height()).max(0.0));
                app.update(Message::Files(
                    crate::extensions::files::Message::RightClickAt { x, y },
                ));
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
                let intent = match crate::webview_geometry::is_in_preview_column(
                    logical_x, logical_w, &state,
                ) {
                    Some(PanelKind::Web) => FocusIntent::Browser,
                    Some(kind) => FocusIntent::Preview(kind),
                    None => FocusIntent::Terminal,
                };
                // 左键按下统一 blur 输入框。但若落点是"原生可编辑预览"
                // 列,它的 `text_editor` 自己这一帧会 self-focus 出光标
                // (官方 `text_editor` 点击即聚焦,无需 main.rs 参与),这里
                // 若再把预览编辑器一起 blur 掉,就会同一帧把它刚自聚焦出
                // 的光标抬掉——表现为"点了代码预览却拿不到光标"(2026-09-06
                // 修复)。是该列仍是原生编辑时才保留;点预览列里其它非编辑器
                // 区/点在其它列时维持原行为,把预览编辑器照常 blur。
                let keep_native_preview_editor = matches!(
                    intent,
                    FocusIntent::Preview(kind) if app.preview_active_tab_is_native(kind)
                );
                app.blur_inputs(keep_native_preview_editor);
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
            // 文件树内拖拽移动同理:左键松开即结束,悬停命中/合法性校验
            // 是靠被拖过目录行的 `on_move` 驱动的(`TreeDragOver`),这里
            // 只负责收尾——有合法待定目标就提交移动,否则原地清空。
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } if app.dragging_tree_item() => {
                app.update(Message::Files(
                    crate::extensions::files::Message::TreeDragRelease,
                ));
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
            // 外部 OS 文件拖拽悬停期间的实时高亮/自动展开:macOS 上原生
            // 拖拽悬停不产生 `CursorMoved`(见 `FILE_DRAG_POSITION` 文档),
            // 改由 `install_file_drag_position_tracker` 装的原生覆写驱动
            // 的 `RedrawRequested` 重新命中并刷新;命中到仍折叠的目录顺带
            // 自动展开,让拖拽能继续往深一层落。非 macOS 平台
            // `file_drag_position()` 恒返回 `None`,退回 `cursor_phys`
            // (若该平台的 winit 后端确实在拖拽悬停时发 `CursorMoved`,
            // 下面 CursorMoved 分支的现状逻辑仍会正常接手)。
            WindowEvent::RedrawRequested if *files_dragging => {
                let scale = window.scale_factor();
                let (logical_x, logical_y) = crate::platform::file_drag::file_drag_position()
                    .unwrap_or_else(|| {
                        (
                            (cursor_phys.x / scale) as f32,
                            (cursor_phys.y / scale) as f32,
                        )
                    });
                let window_width = (window.inner_size().width as f64 / scale) as f32;
                let window_height = (window.inner_size().height as f64 / scale) as f32;
                let hit = app.files_drop_target(window_width, window_height, logical_x, logical_y);
                if let Some(hit) = &hit {
                    app.arm_drag_hover_expand(&hit.target);
                } else {
                    app.clear_drag_hover_expand();
                }
                let hover = hit
                    .into_iter()
                    .map(|h| h.highlight)
                    .collect::<std::collections::HashSet<std::path::PathBuf>>();
                app.update(Message::Files(
                    crate::extensions::files::Message::FileDragHover(hover),
                ));
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
                return false;
            }
            // 外部 OS 文件拖拽落下:优先落到文件树目录行 → 触发移动
            // (吸收掉,不再进后面的终端字节分发);否则落给终端现状行为
            // (不 return,继续走 bytes 匹配的 `DroppedFile` 分支)。命中
            // 位置优先用 `file_drag_position()`(见上面 RedrawRequested
            // 分支的说明),`cursor_phys` 只作非 macOS/未及时收到过原生
            // 覆写回调时的退路。
            WindowEvent::DroppedFile(path) if *files_dragging => {
                let scale = window.scale_factor();
                let (logical_x, logical_y) = crate::platform::file_drag::file_drag_position()
                    .unwrap_or_else(|| {
                        (
                            (cursor_phys.x / scale) as f32,
                            (cursor_phys.y / scale) as f32,
                        )
                    });
                let window_w = (window.inner_size().width as f64 / scale) as f32;
                let window_h = (window.inner_size().height as f64 / scale) as f32;
                let hit = app.files_drop_target(window_w, window_h, logical_x, logical_y);
                *files_dragging = false;
                if let Some(hit) = hit {
                    app.update(Message::Files(
                        crate::extensions::files::Message::FileDrop {
                            paths: vec![path.clone()],
                            target: hit.target,
                        },
                    ));
                    window.request_redraw();
                    return false;
                }
                Self::clear_file_drag_hover(app);
            }
            _ => {}
        }

        // 兜底:search overlay 打开时,主窗口这边收到的 Esc 也关掉它。
        // 正常路径是 overlay 自己的 `SearchOverlay::handle_input`(它有
        // 独立的 `WindowId`,这个按键根本不会落到这里)——这里纯粹是万一
        // overlay 没能真的拿到 OS 键盘焦点(比如某个平台/窗口管理器边界
        // 情形导致按键被系统转投回了主窗口)时的安全阀,不是主路径,不
        // 要求它一直有效。跟下面这段"原生放行闸门"里补的
        // `app.search_popup_open()` 是同一个防御性考虑,理由见那处注释。
        if app.search_popup_open()
            && let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = event
            && event.state == ElementState::Pressed
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::Search(
                crate::extensions::search::Message::SearchClose,
            ));
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            // 右键菜单互斥——关掉当前开着的那个。
            if app.project_link_context_menu_open() {
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
                app.update(Message::Files(
                    crate::extensions::files::Message::ContextMenuClose,
                ));
            }
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::AgentPickerClose);
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::ProjectAddMenuClose);
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::Todo(
                crate::extensions::todo::Message::DispatchClose,
            ));
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::Todo(crate::extensions::todo::Message::StatusClose));
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::Todo(
                crate::extensions::todo::Message::CalendarClose,
            ));
            window.request_redraw();
            return false;
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
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::Todo(
                crate::extensions::todo::Message::StatusFilterClose,
            ));
            window.request_redraw();
            return false;
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
            // 原生预览闸门内的应用级快捷键:原生编辑器/Find 输入框都是真 iced
            // widget,普通打字与剪切复制由 iced 消化;但 ⌘S · ⌘F · ⌘G · ⌘↑ ·
            // Esc 属于"编辑器自己也当普通键吞掉"的组合键,必须先在这里拦下,再
            // 把没命中的键放行给 iced。这里只把命中映射一个 `Message` 交给 gate
            // 末尾统一 update——避免为每个组合键重复 update+redraw、也避开闭包
            // 捕捉 `app` 造成借用冲突。
            let kind_root = kind;
            let WindowEvent::KeyboardInput {
                event: kev,
                is_synthetic: false,
                ..
            } = event
            else {
                return false;
            };
            if kev.state != ElementState::Pressed {
                return false;
            }
            let normal_char =
                |c: &str| kev.logical_key == winit::keyboard::Key::Character(c.into());
            let named =
                |n: winit::keyboard::NamedKey| kev.logical_key == winit::keyboard::Key::Named(n);
            // 命中一个当前动作;无 Find 会话时 Esc/⌘G/⌘↑ 不抢(没条就别误会要开)。
            let bar_open = app.preview_find_bar_open(kind_root);
            let action: Option<Message> = if !modifiers.super_key()
                && !modifiers.control_key()
                && !bar_open
                && named(winit::keyboard::NamedKey::Tab)
            {
                // 裸 Tab(非 ⌘/⌃ 组合,Find 条关着):iced 官方 text_editor
                // 默认 Binding 不把 Tab 落成任何动作,这里让它变成"向前聚焦
                // 编辑器插一个制表符"的编辑动作(见 `PreviewTabInsertTab`)。
                Some(Message::PreviewTabInsertTab(kind_root))
            } else if modifiers.super_key() {
                if normal_char("s") {
                    Some(Message::PreviewSaveActive(kind_root))
                } else if normal_char("z") {
                    // ⌘Z 撤销 / ⌘⇧Z 重做:官方 text_editor 无 undo API,
                    // 走应用层快照栈(见 code_editor 模块"已知取舍")。
                    // shift 分支必须先判,否则 ⌘⇧Z 会被 ⌘Z 抢走。
                    if modifiers.shift_key() {
                        Some(Message::PreviewRedoActive(kind_root))
                    } else {
                        Some(Message::PreviewUndoActive(kind_root))
                    }
                } else if normal_char("f") {
                    // 第二趟 ⌘F 仍是开/聚焦(消息贴合 request_find_focus)。
                    Some(Message::PreviewFindOpen(kind_root))
                } else if normal_char("r") {
                    // ⌘R:同 ⌘F 但替换行默认展开(用户需求:F 收起/R 展开,
                    // 查询框前圆盘箭头再手动切换)。
                    Some(Message::PreviewFindOpenWithReplace(kind_root))
                } else if bar_open && normal_char("g") {
                    // 下一个命中(文件内循环)。无条时空放给 iced 无副作用。
                    Some(Message::PreviewFindGo(kind_root, true))
                } else if bar_open && named(winit::keyboard::NamedKey::ArrowUp) {
                    Some(Message::PreviewFindGo(kind_root, false))
                } else {
                    None
                }
            } else if bar_open && named(winit::keyboard::NamedKey::Escape) {
                // Esc 关条并把焦点归还编辑器(`PreviewFindClose` 内层触发)。
                Some(Message::PreviewFindClose(kind_root))
            } else {
                None
            };
            if let Some(msg) = action {
                app.update(msg);
                window.request_redraw();
                // 已完整接管:不能再把同一个按键喂给 iced,否则官方
                // `text_editor` 默认 Binding 会把 s/f/r/g 这些字母当
                // 普通字符插进正文(见本函数顶部文档)。
                return true;
            }
            // 没命中任何应用级快捷键(比如普通打字):照常放行给 iced,
            // 不然连基本输入都会被这里吞掉。
            return false;
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
            || app.detail_reply_focused()
            || app.tree_edit_focused()
            || app.home_project_search_focused()
            || app.conversation_search_focused()
            || app.git_log_search_focused()
            || app.ssh_form_open()
            || app.database_form_open()
            || app.files_move_confirm_open()
            || app.project_name_focused()
            || app.project_description_focused()
            // search overlay 打开时的防御性兜底:查询框已经不在主窗口的
            // `UserInterface` 里了,正常情况下这个窗口的按键事件根本不会
            // 落到这条主窗口路径(overlay 是独立 `WindowId`,自己的
            // `SearchOverlay::handle_input` 处理)。这里加上纯粹是为了
            // "万一 overlay 没能真的拿到 OS 键盘焦点"这种边界情形兜底——
            // 至少不让按键被当成 ⌘ 组合键/终端输入误处理(即便它们也到
            // 不了查询框,`Esc` 靠上面那条独立的兜底块能关掉弹窗)。
            || app.search_popup_open()
        {
            return false;
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
                            crate::assets::clipboard_image::read_pasteboard_image_as_temp_file()
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
            return false;
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
            return false;
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
                return false;
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
            } if event.state == ElementState::Pressed => crate::keymap::key_to_bytes(
                &event.logical_key,
                modifiers,
                app.active_app_cursor_mode(),
            ),
            WindowEvent::Ime(Ime::Commit(text)) => Some(crate::keymap::ime_commit_to_bytes(text)),
            // 拖文件进终端：转成 shell 转义的完整路径写入会话
            // （Terminal.app 同款行为）。
            WindowEvent::DroppedFile(path) => Some(crate::keymap::dropped_path_to_bytes(
                &path.to_string_lossy(),
            )),
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
        // 终端字节转发路径不接管事件:iced 没有任何原生文本控件会
        // 因为这个按键做出不该有的反应(终端渲染是自绘 canvas,不是
        // `text_editor`),继续照常双喂给标准管线。
        false
    }

    /// 把 app 的 webview 期望清单同步到真实 wry 子视图:
    /// 建缺失、毁多余、对齐可见性与 bounds、URL 变更时导航。文件预览池
    /// 和浏览器池各自独立同步(`preview_desired`/`browser_desired` 已按
    /// `left_view` 互斥,同一时刻至多一个非空)。
    pub(crate) fn sync_previews(&mut self) {
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
        crate::runtime::sync_webview_pool(
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
        let browser_specs: Vec<(crate::preview::WebviewSpec, wry::Rect)> = if app.is_home() {
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
        crate::runtime::sync_webview_pool(
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

    /// 打开任意一类独立窗口弹窗前,先关掉其余已开的——见 `OverlayKind`
    /// 文档。现状代码里"旧弹窗状态没真正清空、被高优先级弹窗遮住之后又
    /// 冒出来"是已确认的真实漂移(见 spec「架构」第 2 节),独立窗口没有
    /// `App::view()` 那种渲染优先级兜底,必须显式互斥。
    fn close_other_overlays(&mut self, keep: OverlayKind) {
        let Self::Ready {
            search_overlay,
            file_history_overlay,
            project_create_overlay,
            settings_overlay,
            ..
        } = self
        else {
            return;
        };
        if keep != OverlayKind::Search {
            *search_overlay = None;
        }
        if keep != OverlayKind::FileHistory {
            *file_history_overlay = None;
        }
        if keep != OverlayKind::ProjectCreate {
            *project_create_overlay = None;
        }
        if keep != OverlayKind::Settings {
            *settings_overlay = None;
        }
    }

    /// 独立按 `ws.search.is_open()` 开/关 search overlay 窗口,跟
    /// `sync_previews` 同款"每次分发完消息就跑一遍"模式,但各管各的
    /// (webview 池同步跟 overlay 窗口生命周期没有交集)。
    fn sync_search_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                search_overlay,
                ..
            } = self
            else {
                return;
            };
            search_overlay::sync_action(app.search_popup_open(), search_overlay.is_some())
        };
        match action {
            search_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::Search);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    search_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let window_width = app.window_size.0;
                *search_overlay = Some(search_overlay::SearchOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    window_width,
                    el,
                ));
            }
            search_overlay::SyncAction::Close => {
                let Self::Ready { search_overlay, .. } = self else {
                    return;
                };
                *search_overlay = None;
            }
            search_overlay::SyncAction::Noop => {}
        }
        // 这次分发的消息可能只是改了 `ws.search` 的内容(典型例子:异步
        // `SearchResults` 从 `user_event` 落地),不涉及开/关窗口,上面的
        // `match` 不会碰它——但内容变了就得让这扇窗口重绘,不然会一直停在
        // "搜索中…"直到用户碰巧在这扇窗口里移动一下鼠标。不判断"到底是
        // 不是真的有变化",跟主窗口 `dispatch()` 对几乎每条消息都无条件
        // `request_redraw()` 的既有尺度一致。
        let Self::Ready { search_overlay, .. } = self else {
            return;
        };
        if let Some(overlay) = search_overlay {
            overlay.request_redraw();
        }
    }

    /// 同 `sync_search_overlay`,按 `app.file_history.is_some()` 开/关
    /// file_history overlay 窗口。
    fn sync_file_history_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                file_history_overlay,
                ..
            } = self
            else {
                return;
            };
            file_history_overlay::sync_action(
                app.file_history.is_some(),
                file_history_overlay.is_some(),
            )
        };
        match action {
            file_history_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::FileHistory);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    file_history_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let main_window_size = LogicalSize::new(app.window_size.0, app.window_size.1);
                *file_history_overlay = Some(file_history_overlay::FileHistoryOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    main_window_size,
                    el,
                ));
            }
            file_history_overlay::SyncAction::Close => {
                let Self::Ready {
                    file_history_overlay,
                    ..
                } = self
                else {
                    return;
                };
                *file_history_overlay = None;
            }
            file_history_overlay::SyncAction::Noop => {}
        }
        let Self::Ready {
            file_history_overlay,
            ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = file_history_overlay {
            overlay.request_redraw();
        }
    }

    fn sync_project_create_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                project_create_overlay,
                ..
            } = self
            else {
                return;
            };
            project_create_overlay::sync_action(
                app.project_create.is_some(),
                project_create_overlay.is_some(),
            )
        };
        match action {
            project_create_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::ProjectCreate);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    project_create_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let main_window_size =
                    winit::dpi::LogicalSize::new(app.window_size.0, app.window_size.1);
                *project_create_overlay = Some(project_create_overlay::ProjectCreateOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    main_window_size,
                    el,
                ));
            }
            project_create_overlay::SyncAction::Close => {
                let Self::Ready {
                    project_create_overlay,
                    ..
                } = self
                else {
                    return;
                };
                *project_create_overlay = None;
            }
            project_create_overlay::SyncAction::Noop => {}
        }
        let Self::Ready {
            project_create_overlay,
            ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = project_create_overlay {
            overlay.request_redraw();
        }
    }

    /// 同 `sync_file_history_overlay`,按 `app.settings.is_some()` 开/关
    /// settings overlay 窗口。
    fn sync_settings_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                settings_overlay,
                ..
            } = self
            else {
                return;
            };
            settings_overlay::sync_action(app.settings.is_some(), settings_overlay.is_some())
        };
        match action {
            settings_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::Settings);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    settings_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let main_window_size =
                    winit::dpi::LogicalSize::new(app.window_size.0, app.window_size.1);
                *settings_overlay = Some(settings_overlay::SettingsOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    main_window_size,
                    el,
                ));
            }
            settings_overlay::SyncAction::Close => {
                let Self::Ready {
                    settings_overlay, ..
                } = self
                else {
                    return;
                };
                *settings_overlay = None;
            }
            settings_overlay::SyncAction::Noop => {}
        }
        let Self::Ready {
            settings_overlay, ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = settings_overlay {
            overlay.request_redraw();
        }
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
        let m = crate::theme::region::right_zone().margin;
        let x = rail + sidebar + divider + m.left + 8.0;
        let w = (window_width - rail - sidebar - divider - rail - m.left - m.right - 16.0).max(0.0);
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
    pub(crate) fn dispatch(&mut self, message: Message) {
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
            Message::Browser(crate::extensions::browser::Message::OpenUrl(_))
                | Message::Browser(crate::extensions::browser::Message::SelectTab(_))
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
                | Message::TabAttached(_, _, _, _, _)
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
            let intent = if state.left_view == PanelKind::Web || state.right_view == PanelKind::Web
            {
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
            // 原生预览 tab 的官方 `text_editor` `Action`:剪贴板读写由 iced
            // 运行时经 `Widget::update` 拿到的 `Clipboard` 直接处理,不需要
            // 像 vendored `iced-code-editor` 那样拆 `Task` 桥接。`Files`/
            // `Project` 两个预览面板分两套消息,分别直呼对应转发方法。
            Message::PreviewEditorEvent(tab_id, action) => {
                app.preview_tab_editor_event(tab_id, action);
                window.request_redraw();
            }
            Message::ProjectPreviewEditorEvent(tab_id, action) => {
                app.project_preview_tab_editor_event(tab_id, action);
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
            // "创建项目"弹窗里两处"选择根目录…":同 `ProjectTabPickFolder`
            // 的套路,原生模态选中后回填 `*RootDirPicked`。`project_create`
            // 模块自己不认识 `rfd`(保持可在单测里构造),原生选择器必须
            // 在这层窗口句柄侧拦截。本窗口不做失焦关闭(见
            // `ProjectCreateOverlay`),即为了此模态弹起时表单不被误关。
            Message::ProjectCreate(
                crate::extensions::project_create::Message::LocalRootDirPick,
            ) => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.update(Message::ProjectCreate(
                        crate::extensions::project_create::Message::LocalRootDirPicked(
                            dir.display().to_string(),
                        ),
                    ));
                }
            }
            Message::ProjectCreate(
                crate::extensions::project_create::Message::CloneRootDirPick,
            ) => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.update(Message::ProjectCreate(
                        crate::extensions::project_create::Message::CloneRootDirPicked(
                            dir.display().to_string(),
                        ),
                    ));
                }
            }
            // 拖拽移动确认框"到目录"旁边的"..."浏览按钮:同上一条
            // `ProjectTabPickFolder` 的套路,原生模态选中后回填
            // `MoveDirInput`(`files::update()` 自己不认识 `rfd`,见
            // `files::Message::MoveDirBrowse` 文档)。起始目录用当前
            // 草稿(没有待确认的移动时这条消息本就不会被派发,`unwrap_or_
            // default` 只是防御性兜底)。
            Message::Files(crate::extensions::files::Message::MoveDirBrowse) => {
                let start = app
                    .active_workspace()
                    .and_then(|ws| ws.files.move_dir_draft())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default();
                if let Some(dir) = rfd::FileDialog::new().set_directory(&start).pick_folder() {
                    app.update(Message::Files(
                        crate::extensions::files::Message::MoveDirInput(dir.display().to_string()),
                    ));
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
                let picked = crate::platform::picker::pick_file_or_dir(&start_dir);
                #[cfg(not(target_os = "macos"))]
                let picked = rfd::FileDialog::new().set_directory(&start_dir).pick_file();

                if let Some(path) = picked {
                    let kind = if path.is_dir() {
                        crate::extensions::project::links::LinkKind::Dir
                    } else {
                        crate::extensions::project::links::LinkKind::File
                    };
                    app.update(Message::Project(
                        crate::extensions::project::Message::LinkAdd { target, path, kind },
                    ));
                    window.request_redraw();
                }
            }
            Message::Files(crate::extensions::files::Message::CopyPath(path, kind)) => {
                let root = app.active_project_path().unwrap_or_else(|| path.clone());
                let s = crate::project::path_string(kind, &path, &root);
                clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
                app.update(Message::Files(
                    crate::extensions::files::Message::ContextMenuClose,
                ));
                window.request_redraw();
            }
            // 点击搜索弹窗里的命中行:由窗口句柄侧拦截,映射回预览域打开
            // (文件预览需要 `allowed_files` 白名单与 tree 高亮,走
            // `App::update` 的 `PreviewOpenPath` 最合适)。权限/焦点一并
            // 处理,并关掉搜索弹窗——"挑中即落地预览",不留浮层悬浮(同
            // 其它弹层互斥清理口径)。
            Message::Search(crate::extensions::search::Message::Pick(hit)) => {
                app.update(Message::Search(
                    crate::extensions::search::Message::SearchClose,
                ));
                *pending_focus = Some(FocusIntent::Preview(PanelKind::Files));
                *current_focus = FocusIntent::Preview(PanelKind::Files);
                app.update(Message::PreviewOpenPath(hit.path));
                window.request_redraw();
            }
            // 浏览器后退/前进/刷新:webview 句柄只在 main.rs 的浏览器池里,
            // 浏览器 `State` 摸不到——在这里对激活 webview 直接执行历史
            // 导航/刷新。wry 0.55 没暴露 `go_back`/`go_forward`,后退/前进
            // 用 `window.history` JS 兜底;刷新走原生 `reload()`。
            Message::Browser(crate::extensions::browser::Message::Nav(action)) => {
                if let Some(id) = app.active_browser_webview_id()
                    && let Some((view, _)) = browser_webviews.get_mut(&id)
                {
                    match action {
                        crate::extensions::browser::NavAction::Back => {
                            let _ = view.evaluate_script("window.history.back()");
                        }
                        crate::extensions::browser::NavAction::Forward => {
                            let _ = view.evaluate_script("window.history.forward()");
                        }
                        crate::extensions::browser::NavAction::Refresh => {
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

    /// sync_previews 之后统一应用焦点意图(此时新建 webview 已入池)。
    /// Preview/Browser → 各自当前激活 webview 拿键盘(⌘C 原生复制);
    /// Terminal/无 webview → 交回窗口(终端键盘)。
    pub(crate) fn apply_pending_focus(&mut self) {
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
    pub(crate) fn apply_pending_zoom_toggle(&mut self) {
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

/// 从 main.rs 迁移(main.rs 瘦身补做,原 impl ApplicationHandler 依赖它们)。
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
                instance,
                adapter,
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
                search_overlay: None,
                file_history_overlay: None,
                project_create_overlay: None,
                settings_overlay: None,
            };
        }
    }

    /// tokio 任务经 `EventLoopProxy<Message>::send_event` 送回来的事件
    /// （attach 数据流的输出/退出、daemon 错误、新建会话完成……）在这里
    /// 落地：直接喂给 `App::update`，跟 `window_event` 里处理
    /// iced 消息走的是同一条 `update` 逻辑，只是消息来源不同。
    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: Message) {
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
        self.sync_search_overlay(event_loop);
        self.sync_file_history_overlay(event_loop);
        self.sync_project_create_overlay(event_loop);
        self.sync_settings_overlay(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // search overlay 窗口自己那份 `WindowId` 的事件,整段独立处理,
        // 早退保证不动下面那 ~1000 行主窗口的既有 `match`。
        if let Self::Ready {
            app,
            search_overlay,
            ..
        } = self
            && let Some(overlay) = search_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::Search(extensions::search::Message::SearchClose));
            } else if let WindowEvent::Focused(focused) = event {
                // 合成 Focused(false) 不计为失焦(见 SearchOverlay::handle_focus)。
                if overlay.handle_focus(focused) {
                    self.dispatch(Message::Search(extensions::search::Message::SearchClose));
                }
            } else {
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            // 查询框原生右键菜单(剪切/复制/粘贴)选中一项后的合成按键收尾——
            // 主窗口那份等价逻辑(window_event 尾部)建的是 `app.view()`,
            // 查询框已经不在那棵树里,够不着,这扇窗口自己补一遍。上面的
            // `self.dispatch(...)` 需要整个 `self` 的可变借用,跟这里已经
            // 借出去的 `app`/`overlay` 冲突不了(NLL 允许 `app`/`overlay`
            // 在各自分支内"最后一次用"之后释放),但这一步要在 dispatch 循环
            // *之后* 再用一次 `app`,所以在这里重新单独借一次,而不是复用
            // 上面那次借用。
            if let Self::Ready {
                app,
                search_overlay,
                ..
            } = self
                && let Some(overlay) = search_overlay
            {
                overlay.apply_pending_native_menu_edit_key(app);
            }
            // 挑中一条结果(`Message::Search(Pick(hit))`)在 `dispatch` 里
            // 被内核拦截成 `PreviewOpenPath` + `SearchClose`,会新开/切换
            // 预览 webview、设置待聚焦意图——这两件事平时都要靠这三个调用
            // 落地(`user_event`/主窗口 `window_event` 尾部都是这个顺序),
            // overlay 这条分支之前漏调了,新开的预览要等主窗口凑巧被别的
            // 事件触发到这段收尾才会真的显示出来/拿到焦点。
            self.sync_previews();
            self.apply_pending_focus();
            self.apply_pending_zoom_toggle();
            self.sync_search_overlay(event_loop);
            return;
        }

        // file_history overlay 窗口自己那份 `WindowId` 的事件,同 search
        // overlay 早退分支的手法,互相独立。
        if let Self::Ready {
            app,
            file_history_overlay,
            ..
        } = self
            && let Some(overlay) = file_history_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::FileHistory(
                    extensions::file_history::Message::Close,
                ));
            } else if let WindowEvent::Focused(focused) = event {
                if overlay.handle_focus(focused) {
                    self.dispatch(Message::FileHistory(
                        extensions::file_history::Message::Close,
                    ));
                }
            } else {
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            self.sync_file_history_overlay(event_loop);
            return;
        }

        if let Self::Ready {
            app,
            project_create_overlay,
            ..
        } = self
            && let Some(overlay) = project_create_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::ProjectCreate(
                    extensions::project_create::Message::Close,
                ));
            } else {
                // 故意不处理 `WindowEvent::Focused`——本窗口不做失焦关闭
                // (见 `ProjectCreateOverlay` 文档注释),根目录字段要弹
                // 嵌套的 rfd 选择器,那会让本窗口瞬间失焦,若照搬 search/
                // file_history 的失焦关闭逻辑会在用户选目录过程中把整个
                // 表单误关掉。Esc 键的关闭由 `handle_input` 内部拦截,
                // 走的是普通消息返回路径,不需要这里特殊处理。
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            // `handle_input` 上面这一圈 `dispatch` 里可能出现
            // `GoToSettings`(设 `app.settings = Some(...)`)——那扇窗口的
            // 生命周期由 `sync_settings_overlay` 单独驱动,这个分支只调
            // 自己的 `sync_project_create_overlay` 的话,新窗口不保证在
            // 这一帧就被建出来(代码评审 finding:GoToSettings doesn't
            // sync new overlay same tick),两个都要跟着 dispatch 后调。
            self.sync_project_create_overlay(event_loop);
            self.sync_settings_overlay(event_loop);
            return;
        }

        if let Self::Ready {
            app,
            settings_overlay,
            ..
        } = self
            && let Some(overlay) = settings_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::Settings(extensions::settings::Message::Close));
            } else if let WindowEvent::Focused(focused) = event {
                // 本弹窗接入失焦即关闭,但"没有 PAT?点此生成"会拉起系统
                // 浏览器,那次真实失焦要靠 `State::suppress_next_blur`
                // 吞掉(见 `SettingsOverlay::handle_focus` 文档注释)。
                let mut fallback = false;
                let suppress = app
                    .settings
                    .as_mut()
                    .map(|s| &mut s.suppress_next_blur)
                    .unwrap_or(&mut fallback);
                if overlay.handle_focus(focused, suppress) {
                    self.dispatch(Message::Settings(extensions::settings::Message::Close));
                }
            } else {
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            self.sync_settings_overlay(event_loop);
            return;
        }

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
                search_overlay,
                file_history_overlay,
                project_create_overlay,
                settings_overlay,
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

                            // 同理,消费 项目名称编辑 的一次性聚焦位(触发
                            // 点击落在旧的 button/MouseArea 上,真
                            // `text_input` 本帧才出现、不会自己拿焦点;验收
                            // 意见框常驻可见、点击即原生聚焦,不需要这机制)。
                            let name_edit_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_name_edit_focus_pending());

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

                            // IME 组字预览浮层的落点 + 内容,`State::Updated`
                            // 分支下面填充,画在 `interface.draw()` 之后
                            // (见下方"画 IME 组字预览浮层"注释)。
                            let mut ime_overlay: Option<(Rectangle, String)> = None;

                            // Update the mouse cursor(见 `apply_mouse_cursor`
                            // 文档:事件路径也调它,两条路径都要刷)。
                            if let user_interface::State::Updated {
                                mouse_interaction,
                                input_method,
                                ..
                            } = state
                            {
                                apply_mouse_cursor(window, app, webview_rects, mouse_interaction);

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
                    // search overlay 是一扇尺寸跟随主窗口宽度的子窗口,主窗口
                    // resize 后要重新居中 + 重配 surface(`with_parent_window`
                    // 只管"跟着移动",不管尺寸/布局联动)。
                    if let Some(overlay) = search_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                        );
                    }
                    if let Some(overlay) = file_history_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                            app.window_size.1,
                        );
                    }
                    if let Some(overlay) = project_create_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                            app.window_size.1,
                        );
                    }
                    if let Some(overlay) = settings_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                            app.window_size.1,
                        );
                    }
                    // bounds 同步由本函数末尾的 sync_previews 统一执行
                }
                WindowEvent::CloseRequested => {
                    // 图干净,不是正确性要求——Drop 本身就会释放。
                    *search_overlay = None;
                    *file_history_overlay = None; // 同上,图干净。
                    *project_create_overlay = None; // 图干净,Drop 本身就会释放。
                    *settings_overlay = None; // 图干净,Drop 本身就会释放。
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

                let (state, _) =
                    interface.update(events, *cursor, renderer, clipboard, &mut messages);

                // 事件路径也必须立刻刷新窗口光标,不能只等 `RedrawRequested`
                // ——纯 hover 移动鼠标不产生消息、不触发重绘,不在这里刷就会
                // 漏掉光标形状变化(代码编辑器的 I 形即由此丢失)。见
                // `apply_mouse_cursor` 文档。
                if let user_interface::State::Updated {
                    mouse_interaction, ..
                } = state
                {
                    apply_mouse_cursor(window, app, webview_rects, mouse_interaction);
                }

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
        self.sync_search_overlay(event_loop);
        self.sync_file_history_overlay(event_loop);
        self.sync_project_create_overlay(event_loop);
        self.sync_settings_overlay(event_loop);
    }
}
