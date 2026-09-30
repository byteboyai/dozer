//! macOS 专有窗口平台胶水:原生交通灯垂直居中 + 顶栏原生拖窗守卫。

dozer_core::scope!(LOG, module, "platform");

/// macOS 专有：把原生红黄绿交通灯在垂直方向居中到 app 自己画的 `top_bar`
/// （默认 40pt 高）中部，而不是系统默认的 28pt 标题栏中部。去掉原生标题栏
/// 后系统仍按 28pt 旧基准排版交通灯，导致它们贴着顶栏上沿、与 40pt 顶栏里
/// 的 Dozer 字标/页签不对齐。差额对半即把三个灯整体下移、落入顶栏正中。
///
/// 放在 `WindowEvent::Resized`（含首屏显隐、全屏进出、拖拽缩放）里重设。
/// 目标 y 直接由按钮父视图的当前 bounds 计算，不缓存首次看到的 frame：窗口
/// 从最大化恢复时，`Resized` 可能早于 AppKit 对标题栏按钮的重新布局；若在这
/// 个时机把旧 frame 记成普通窗口基线，之后会再下移一次，交通灯便会永久偏低。
/// 父视图 bounds 是当前窗口状态的绝对几何，因而不依赖事件先后且重复调用幂等。
/// 仅在 `target_os = "macos"` 编译——其它平台无原生交通灯可摆。
fn traffic_light_origin_y(
    bounds_origin_y: f64,
    bounds_height: f64,
    button_height: f64,
    top_bar_height: f64,
    flipped: bool,
) -> f64 {
    if flipped {
        bounds_origin_y + (top_bar_height - button_height) / 2.0
    } else {
        bounds_origin_y + bounds_height - (top_bar_height + button_height) / 2.0
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn center_traffic_lights(window: &winit::window::Window) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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

    for btn in buttons.into_iter() {
        let Some(btn) = btn else { continue };
        let Some(superview) = (unsafe { btn.superview() }) else {
            continue;
        };
        let frame = btn.frame();
        let bounds = superview.bounds();
        let mut origin = frame.origin;
        origin.y = traffic_light_origin_y(
            bounds.origin.y,
            bounds.size.height,
            frame.size.height,
            byteui::theme::geometry::top_bar_height() as f64,
            superview.isFlipped(),
        );
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
        dozer_core::log_warn!(
            LOG,
            "挂 mouseDownCanMoveWindow 覆写失败(selector 可能已存在于该类)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::traffic_light_origin_y;

    #[test]
    fn traffic_light_y_uses_current_unflipped_parent_bounds() {
        assert_eq!(traffic_light_origin_y(0.0, 28.0, 16.0, 40.0, false), 0.0);
        assert_eq!(traffic_light_origin_y(4.0, 32.0, 16.0, 40.0, false), 8.0);
    }

    #[test]
    fn traffic_light_y_supports_flipped_parent_coordinates() {
        assert_eq!(traffic_light_origin_y(3.0, 28.0, 16.0, 40.0, true), 15.0);
    }
}
