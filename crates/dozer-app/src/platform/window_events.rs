//! 窗口事件状态机:`Runner` 状态(Loading/Ready)与 `impl Runner`(窗口事件
//! 分发、焦点路由、webview 池同步)。Phase 1 结构重组时从 `main.rs` 抽出,
//! 逻辑保持原样。

use std::sync::Arc;

use iced_wgpu::graphics::Viewport;
use iced_wgpu::wgpu;
use iced_winit::Clipboard;
use iced_winit::core::Event;
use iced_winit::core::mouse;
use iced_winit::runtime::user_interface;

use winit::{
    event::{ElementState, Ime, WindowEvent},
    keyboard::ModifiersState,
};

use crate::app::{App, Message, PanelKind};

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
            || app.query_focused()
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
