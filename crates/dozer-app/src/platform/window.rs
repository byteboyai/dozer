//! macOS 专有窗口平台胶水:原生交通灯垂直居中 + 顶栏原生拖窗守卫。

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
pub(crate) fn center_traffic_lights(window: &winit::window::Window) {
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
pub(crate) static TOPBAR_CONTROL_HOVERED: std::sync::atomic::AtomicBool =
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
pub(crate) fn install_topbar_drag_guard(window: &winit::window::Window) {
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
