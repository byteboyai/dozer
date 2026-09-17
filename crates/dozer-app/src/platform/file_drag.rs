//! macOS 专有外部文件拖拽悬停位置追踪:补 winit 没实现的 `draggingUpdated:`
//! 回调,让外部 OS 文件拖拽悬停期间文件树能实时高亮命中目录。

/// macOS 专有：外部 OS 文件拖拽悬停在窗口内移动期间的最新光标位置（逻辑
/// 坐标，原点左上，Y 向下——与 `cursor_phys`/`files_drop_target` 同源）。
///
/// 由 [`install_file_drag_position_tracker`] 装的 `draggingUpdated:` 覆写
/// 每次回调都写一遍；`main.rs` 的 `on_window_event` 每帧 `RedrawRequested`
/// 读一遍去重算文件树命中/高亮（见该处调用）。winit 的 macOS 后端只实现了
/// `draggingEntered:`/`performDragOperation:`/`draggingExited:` 三个
/// `NSDraggingDestination` 方法，唯独没有 `draggingUpdated:`——已用诊断日志
/// 实测确认：一次完整的外部文件拖拽过程里，`WindowEvent::CursorMoved` 一次
/// 都不会触发，`HoveredFile` 只在进入窗口那一刻来一次。这不是这份实现的
/// bug，是 winit 本身没有为原生拖拽悬停过程暴露任何带位置的事件。
#[cfg(target_os = "macos")]
static FILE_DRAG_POSITION: std::sync::Mutex<Option<(f32, f32)>> = std::sync::Mutex::new(None);

/// 读一次 [`FILE_DRAG_POSITION`]。非 macOS 平台恒 `None`(该平台的 winit
/// 后端若确实在拖拽悬停时发 `CursorMoved`,调用方自会退回 `cursor_phys`,
/// 见调用处)。
#[cfg(target_os = "macos")]
pub(crate) fn file_drag_position() -> Option<(f32, f32)> {
    FILE_DRAG_POSITION.lock().ok().and_then(|p| *p)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn file_drag_position() -> Option<(f32, f32)> {
    None
}

/// macOS 专有：[`install_file_drag_position_tracker`] 装的原生覆写要在
/// 拖拽悬停期间反复请求重绘,但它是个不持有 `Self::Ready` 任何字段的
/// `extern "C-unwind" fn`,够不到 `main.rs` 事件循环里的 `window` 变量——
/// 装覆写时把 `Arc<Window>` 存一份在这里,让覆写体能调 winit 官方的
/// `request_redraw()`(会自己去抖合并,不会比原生 `setNeedsDisplay:` 更
/// 激进,且走的是 winit 预期的重绘请求路径,不绕过它的内部记账)。
#[cfg(target_os = "macos")]
static FILE_DRAG_WINDOW: std::sync::OnceLock<std::sync::Arc<winit::window::Window>> =
    std::sync::OnceLock::new();

// macOS 专有：装覆写时顺手存一份内容 NSView 的裸指针,供覆写体调
// `convertPoint:fromView:` 把窗口坐标换算成这块 view 自己的本地坐标系
// （见 `install_file_drag_position_tracker` 文档"坐标换算"一段）。只在
// 主线程读写(AppKit 回调/`install_*` 安装都在主线程),用 `Cell` 免加锁。
// 存活期与窗口本身相同,不需要释放。
#[cfg(target_os = "macos")]
thread_local! {
    static FILE_DRAG_CONTENT_VIEW: std::cell::Cell<*mut objc2_app_kit::NSView> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// macOS 专有：给内容 NSView 的运行时类新增 `draggingUpdated:` 覆写，补上
/// winit 没实现的这一环（见 [`FILE_DRAG_POSITION`] 文档）。同 `install_
/// topbar_drag_guard` 的手法——`class_addMethod` 只新增全新 selector，不碰
/// winit 自己已经实现的 `draggingEntered:`/`performDragOperation:`。
///
/// 坐标换算：`[sender draggingLocation]` 给的是窗口 base 坐标系的点，直接
/// 拿窗口高度做算术翻转一度踩了坑——`fullSizeContentView` 窗口的标题栏/
/// 内容视图边界关系没有一份公开、稳定的算术公式（`NSWindow.
/// contentRectForFrameRect:`、`inner_size()` 各种推导都实测跟
/// `draggingLocation` 对不上，偏差还不是个简单常数）。改用 AppKit 自己的
/// `[contentView convertPoint:loc fromView:nil]`——这是官方指定的"从窗口
/// 坐标转某个 view 本地坐标"的转换,不管标题栏怎么算都会给对的答案;若这块
/// view `isFlipped`(iced/wgpu 内容视图通常是),转换结果已经是原点左上、Y
/// 向下,直接就是 `files_drop_target` 要的逻辑坐标,不需要再手动翻转。
///
/// 挂载对象是**窗口的 delegate**,不是内容 NSView——读 winit 0.30.13 源码
/// (`window_delegate.rs`)确认 `draggingEntered:`/`performDragOperation:`/
/// `draggingExited:` 三个方法和 `registerForDraggedTypes:` 调用都在
/// `WindowDelegate` 类上,不在内容 view 上;挂错对象时 `class_addMethod`
/// 照样报成功(纯运行时类操作,不检查这个类是不是真的会收到拖拽回调),
/// 但 AppKit 实际派发拖拽事件时根本不会问这个 view,已用日志实测确认
/// (`class_addMethod` 成功但覆写体从未被调用)。
#[cfg(target_os = "macos")]
pub(crate) fn install_file_drag_position_tracker(window: &std::sync::Arc<winit::window::Window>) {
    use objc2::encode::Encode;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{ffi, sel};
    use objc2_app_kit::{NSDragOperation, NSView};
    use objc2_foundation::NSPoint;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let _ = FILE_DRAG_WINDOW.set(window.clone());

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(ah) = handle.as_raw() else {
        return;
    };
    // 安全：同 `install_topbar_drag_guard`——`ns_view` 是已装进窗口的合法
    // 指针，这里只借它去拿窗口/delegate 对象，不持有/释放它们。
    let ns_view: &NSView = unsafe { &*(ah.ns_view.as_ptr() as *mut NSView) };
    FILE_DRAG_CONTENT_VIEW.with(|v| v.set(ns_view as *const NSView as *mut NSView));
    let Some(ns_window) = ns_view.window() else {
        return;
    };
    let Some(delegate) = ns_window.delegate() else {
        return;
    };
    let delegate_obj: &AnyObject = unsafe { &*(Retained::as_ptr(&delegate) as *const AnyObject) };
    let class: &AnyClass = delegate_obj.class();

    unsafe extern "C-unwind" fn dragging_updated(
        _this: &AnyObject,
        _cmd: Sel,
        sender: *mut AnyObject,
    ) -> NSDragOperation {
        let loc: NSPoint = unsafe { objc2::msg_send![sender, draggingLocation] };
        let view_ptr = FILE_DRAG_CONTENT_VIEW.with(|v| v.get());
        if !view_ptr.is_null()
            && let Some(window) = FILE_DRAG_WINDOW.get()
        {
            let view: &NSView = unsafe { &*view_ptr };
            let local: NSPoint = view.convertPoint_fromView(loc, None);
            if let Ok(mut pos) = FILE_DRAG_POSITION.lock() {
                *pos = Some((local.x as f32, local.y as f32));
            }
            window.request_redraw();
        }
        NSDragOperation::Generic
    }

    // `-(NSDragOperation)draggingUpdated:(id)sender` 的类型串：返回值 + self
    // + _cmd + 一个 id 参数,四段。
    let types = std::ffi::CString::new(format!(
        "{}{}{}{}",
        NSDragOperation::ENCODING,
        <*mut AnyObject>::ENCODING,
        Sel::ENCODING,
        <*mut AnyObject>::ENCODING,
    ))
    .expect("type encoding 不含 NUL");
    let imp: Imp = unsafe {
        core::mem::transmute::<
            unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> NSDragOperation,
            Imp,
        >(dragging_updated)
    };
    let added = unsafe {
        ffi::class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            sel!(draggingUpdated:),
            imp,
            types.as_ptr(),
        )
    };
    if !added.as_bool() {
        tracing::warn!("挂 draggingUpdated: 覆写失败(selector 可能已存在于该类)");
    }
}
