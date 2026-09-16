//! macOS 原生右键菜单(NSMenu),替代会被 wry webview 遮挡的 iced 弹层
//! 弹层。详见 `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`。

#![cfg(target_os = "macos")]

use byteui::interaction::icons::IconKind;
use iced_widget::core::Color;

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadOnly};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSData, NSPoint, NSString};

/// 一条原生菜单描述——调用方只管拼数据,不碰 AppKit。
pub enum Item<Msg> {
    Entry {
        icon: Option<IconKind>,
        label: String,
        color: Color,
        enabled: bool,
        msg: Msg,
    },
    Separator,
}

/// 把内嵌 Lucide SVG(`IconKind::bytes()`)按给定颜色栅格化成
/// `size_px × size_px` 的位图——渲染出来的 alpha 通道当遮罩,RGB 统一替换
/// 成 `color`(同 iced 侧 `svg::Style{color}` 的着色语义,忽略 SVG 自身
/// 颜色)。纯函数,不碰 AppKit,可在任何线程/CI 里跑。
fn render_icon_pixmap(kind: IconKind, color: Color, size_px: u32) -> tiny_skia::Pixmap {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(kind.bytes(), &opt).expect("内嵌 Lucide SVG 资源必须能解析");
    let native_size = tree.size().to_int_size();
    let scale = size_px as f32 / native_size.width().max(1) as f32;
    let mut pixmap = tiny_skia::Pixmap::new(size_px, size_px).expect("size_px 非零");
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let (r, g, b) = (
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8,
    );
    for pixel in pixmap.pixels_mut() {
        let a = pixel.alpha();
        *pixel = tiny_skia::PremultipliedColorU8::from_rgba(
            (r as u16 * a as u16 / 255) as u8,
            (g as u16 * a as u16 / 255) as u8,
            (b as u16 * a as u16 / 255) as u8,
            a,
        )
        .expect("premultiply 计算结果 r/g/b <= a 恒成立");
    }
    pixmap
}

// 弹菜单时要挂靠的窗口内容 view——`show()` 是个不持有 `winit::window::Window`
// 的自由函数,够不到窗口句柄,故在窗口初始化时存一份裸指针在这里(同
// `main.rs::FILE_DRAG_CONTENT_VIEW` 的既有模式,理由一致:原生回调/独立
// 调用点没有 `Self::Ready` 字段可用)。只在主线程读写(`install_content_view`
// 和 `show()` 都在 macOS 主线程),用 `thread_local!` + `Cell` 免加锁。
// 存活期与窗口本身相同,不需要释放。
thread_local! {
    static CONTENT_VIEW: std::cell::Cell<*mut NSView> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

// main.rs 窗口初始化时调一次(紧邻 `install_topbar_drag_guard`/
// `install_file_drag_position_tracker` 的调用点),记下内容 view 供
// `show()` 后续弹菜单用。
pub fn install_content_view(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(ah) = handle.as_raw() else {
        return;
    };
    let ptr = ah.ns_view.as_ptr() as *mut NSView;
    CONTENT_VIEW.with(|v| v.set(ptr));
}

fn content_view() -> Option<&'static NSView> {
    let ptr = CONTENT_VIEW.with(|v| v.get());
    if ptr.is_null() {
        return None;
    }
    // 安全性同 `main.rs` 里其它读取该类裸指针的场景:只借引用去调只读/
    // 弹层方法,不持有/释放。
    Some(unsafe { &*ptr })
}

/// 一次 `show()` 调用期间,自定义 item view 的 `mouseUp:` 覆写把"选中了
/// 第几项"写在这里;`popUpMenuPositioningItem_atLocation_inView` 返回后
/// 读一次、立刻清空。是线程内单发的槽位(菜单是同步阻塞的模态追踪,不会
/// 有第二个 `show()` 并发进行)。
static SELECTED_INDEX: std::sync::Mutex<Option<usize>> = std::sync::Mutex::new(None);

// 按 `(IconKind, 颜色 ARGB, 像素尺寸)` 缓存栅格化结果,避免同一个图标
// 每次弹菜单都重新过一遍 `resvg`。整条调用链限定在主线程(AppKit 要求),
// `Retained<NSImage>` 是 `MainThreadOnly`(非 `Send`),故用 `thread_local!`
// 而非 `OnceLock`(后者要求 `Sync`)。
thread_local! {
    static ICON_CACHE: std::cell::RefCell<std::collections::HashMap<(IconKind, u32, u32), Retained<NSImage>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn color_key(color: Color) -> u32 {
    let (r, g, b, a) = (
        (color.r * 255.0).round() as u32,
        (color.g * 255.0).round() as u32,
        (color.b * 255.0).round() as u32,
        (color.a * 255.0).round() as u32,
    );
    (r << 24) | (g << 16) | (b << 8) | a
}

/// 菜单项图标像素尺寸——同 `crate::menu.rs::icon_leading` 用的
/// `icon_size::row()`(已含全局 scale),原生菜单和 iced 菜单在同一次
/// 缩放调整下应保持一致大小。
fn icon_size_px() -> u32 {
    byteui::theme::icon_size::row().round() as u32
}

/// 把 `render_icon_pixmap` 的结果编码成 PNG、包成 `NSImage`(经
/// `NSData::with_bytes` + `NSImage::initWithData`,比手搭
/// `NSBitmapImageRep` 的裸像素平面初始化器简单可靠——PNG 编解码本身处理
/// 好了预乘 alpha 的语义)。命中缓存直接返回。
fn icon_image(kind: IconKind, color: Color, size_px: u32) -> Retained<NSImage> {
    let key = (kind, color_key(color), size_px);
    if let Some(img) = ICON_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return img;
    }
    let pixmap = render_icon_pixmap(kind, color, size_px);
    let png = pixmap.encode_png().expect("tiny-skia PNG 编码不应失败");
    let data = NSData::with_bytes(&png);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .expect("PNG 编码出的数据必须能被 NSImage 解出来");
    ICON_CACHE.with(|c| c.borrow_mut().insert(key, image.clone()));
    image
}

/// 同步弹出原生菜单,阻塞到用户选中一项或点外部/按 Esc 取消。`items` 为
/// 空时直接返回 `None`,不弹菜单。`view_pos` 是内容 view 自己坐标系里的
/// 一点——和现有 `crate::menu.rs` 弹层用的 `last_right_click()` 是同一份
/// 逻辑坐标,不需要转换成屏幕坐标(`popUpMenuPositioningItem:atLocation:inView:`
/// 的 `atLocation:` 就是"目标 view 自己坐标系里的一点")。
pub fn show<Msg: Clone>(items: Vec<Item<Msg>>, view_pos: (f32, f32)) -> Option<Msg> {
    if items.is_empty() {
        return None;
    }
    let Some(view) = content_view() else {
        tracing::warn!("native_menu::show: 内容 view 未注册,跳过弹菜单");
        return None;
    };
    let mtm = objc2::MainThreadMarker::new().expect("show() 只能在主线程调用");
    *SELECTED_INDEX.lock().unwrap() = None;

    let menu = NSMenu::new(mtm);
    // 索引 → 消息的映射,`SELECTED_INDEX` 写回的下标据此取出对应 `Msg`。
    let mut msgs: Vec<Option<Msg>> = Vec::with_capacity(items.len());

    for (idx, item) in items.into_iter().enumerate() {
        match item {
            Item::Separator => {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
                msgs.push(None);
            }
            Item::Entry {
                icon,
                label,
                color,
                enabled,
                msg,
            } => {
                let ns_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(&label),
                        None,
                        &NSString::from_str(""),
                    )
                };
                ns_item.setEnabled(enabled);
                let icon_px = icon_size_px();
                if let Some(icon) = icon {
                    ns_item.setImage(Some(&icon_image(icon, color, icon_px)));
                }
                let row_view = menu_item_view::MenuItemView::new(
                    mtm,
                    &label,
                    color,
                    enabled,
                    icon.map(|k| icon_image(k, color, icon_px)),
                    idx,
                );
                ns_item.setView(Some(&row_view));
                menu.addItem(&ns_item);
                msgs.push(Some(msg));
            }
        }
    }

    let _ = menu.popUpMenuPositioningItem_atLocation_inView(
        None,
        NSPoint::new(view_pos.0 as f64, view_pos.1 as f64),
        Some(view),
    );

    SELECTED_INDEX
        .lock()
        .unwrap()
        .take()
        .and_then(|idx| msgs.get(idx).cloned().flatten())
}

mod menu_item_view {
    use super::{Color, SELECTED_INDEX};
    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSColor, NSEvent, NSFont, NSImage, NSImageView, NSTextField, NSTrackingArea,
        NSTrackingAreaOptions, NSView,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::cell::Cell;

    /// hover 高亮的圆角半径,对齐 `crate::menu.rs` 里 iced 版菜单项的
    /// `MENU_HOVER_RADIUS`。
    const HOVER_RADIUS: f64 = 6.0;

    /// 自定义菜单项 view 的实例状态:命中的下标(点击时回填
    /// `SELECTED_INDEX` 用)+ 是否可点(锁定项不接 hover/点击)+ 当前是否
    /// 悬停(`mouseEntered:`/`mouseExited:` 翻转,驱动 `layer.backgroundColor`
    /// 高亮——`NSMenuItem` 挂了自定义 `view` 后 AppKit 不会自动画选中态,
    /// 必须自己维护这个状态并据此改层背景色,不能只靠 `setNeedsDisplay`
    /// (没有 `drawRect:` 覆写,那样什么也不会变)。
    pub struct Ivars {
        index: usize,
        enabled: bool,
        hovered: Cell<bool>,
    }

    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[ivars = Ivars]
        pub struct MenuItemView;

        unsafe impl NSObjectProtocol for MenuItemView {}

        impl MenuItemView {
            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _event: &NSEvent) {
                if self.ivars().enabled {
                    self.ivars().hovered.set(true);
                    self.apply_hover_background();
                }
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {
                self.ivars().hovered.set(false);
                self.apply_hover_background();
            }

            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, _event: &NSEvent) {
                if !self.ivars().enabled {
                    return;
                }
                *SELECTED_INDEX.lock().unwrap() = Some(self.ivars().index);
                let menu = self.enclosingMenuItem().and_then(|item| unsafe { item.menu() });
                if let Some(menu) = menu {
                    menu.cancelTrackingWithoutAnimation();
                }
            }
        }
    );

    impl MenuItemView {
        /// 按 `hovered` 当前值把层背景设成 `TAB_HOVER` 色或透明——
        /// `mouseEntered:`/`mouseExited:`/`new()` 初始化都调这个,保持单一
        /// 落笔点。
        fn apply_hover_background(&self) {
            let Some(layer) = self.layer() else {
                return;
            };
            if self.ivars().hovered.get() {
                let hover = byteui::theme::color::current().tab_hover;
                let ns_color = NSColor::colorWithRed_green_blue_alpha(
                    hover.r as f64,
                    hover.g as f64,
                    hover.b as f64,
                    hover.a as f64,
                );
                layer.setBackgroundColor(Some(&ns_color.CGColor()));
            } else {
                layer.setBackgroundColor(None);
            }
        }

        /// 组一整行:自身画 hover 底色(靠 `wantsLayer`+`layer.backgroundColor`
        /// 更简单),内部横排图标(可选)+ 文字。宽/内边距/图标↔文字间距/
        /// 字号全部读 `byteui::theme` token(均已含全局 scale),和
        /// `crate::menu.rs` 的 iced 版菜单项共用同一套尺寸口径,放大/缩小
        /// UI 时原生菜单跟着一起变,不会停在编译期写死的固定像素。
        pub fn new(
            mtm: MainThreadMarker,
            label: &str,
            color: Color,
            enabled: bool,
            icon: Option<Retained<NSImage>>,
            index: usize,
        ) -> Retained<Self> {
            let font_size = byteui::theme::font::label() as f64;
            let pad_h = byteui::theme::geometry::menu_pad_h() as f64;
            let pad_v = byteui::theme::geometry::menu_pad_v() as f64;
            let gap = byteui::theme::geometry::menu_gap() as f64;
            let icon_px = super::icon_size_px() as f64;
            let row_width = byteui::theme::geometry::menu_item_width() as f64;
            // 文字行高留一点余量(字号本身只是字形高度,行框要比它高一圈
            // 才不会顶到边),行高再取"图标/文字谁高就跟谁"+ 上下内边距。
            let text_height = font_size + 4.0;
            let content_height = icon_px.max(text_height);
            let row_height = content_height + pad_v * 2.0;

            let this = mtm.alloc::<Self>().set_ivars(Ivars {
                index,
                enabled,
                hovered: Cell::new(false),
            });
            let this: Retained<Self> = unsafe {
                msg_send![
                    super(this),
                    initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(row_width, row_height))
                ]
            };

            this.setWantsLayer(true);
            if let Some(layer) = this.layer() {
                layer.setCornerRadius(HOVER_RADIUS);
            }
            let owner: &objc2::runtime::AnyObject = &this;
            let tracking = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    mtm.alloc::<NSTrackingArea>(),
                    this.bounds(),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways,
                    Some(owner),
                    None,
                )
            };
            this.addTrackingArea(&tracking);

            let mut x = pad_h;
            if let Some(icon) = icon {
                let icon_y = (row_height - icon_px) / 2.0;
                let image_view = NSImageView::initWithFrame(
                    NSImageView::alloc(mtm),
                    NSRect::new(NSPoint::new(x, icon_y), NSSize::new(icon_px, icon_px)),
                );
                image_view.setImage(Some(&icon));
                this.addSubview(&image_view);
                x += icon_px + gap;
            }
            let text_y = (row_height - text_height) / 2.0;
            let text = NSTextField::initWithFrame(
                NSTextField::alloc(mtm),
                NSRect::new(
                    NSPoint::new(x, text_y),
                    NSSize::new(row_width - x - pad_h, text_height),
                ),
            );
            text.setStringValue(&NSString::from_str(label));
            text.setBezeled(false);
            text.setDrawsBackground(false);
            text.setEditable(false);
            text.setSelectable(false);
            text.setFont(Some(&NSFont::systemFontOfSize(font_size)));
            let ns_color = NSColor::colorWithRed_green_blue_alpha(
                color.r as f64,
                color.g as f64,
                color.b as f64,
                color.a as f64,
            );
            text.setTextColor(Some(&ns_color));
            this.addSubview(&text);

            this
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files 右键菜单用到的图标都能正常栅格化成非空、非全透明的位图——
    /// 不要求逐像素比对(信任 resvg 渲染正确性),只验证"栅格化流程本身
    /// 没有 panic/返回空图",这是把 `render_icon_pixmap` 接进真正
    /// AppKit 代码之前最基本的保障。
    #[test]
    fn render_icon_pixmap_produces_nonempty_bitmap_for_menu_icons() {
        for kind in [
            IconKind::Search,
            IconKind::Trash,
            IconKind::Copy,
            IconKind::ClipboardPaste,
            IconKind::Scissors,
            IconKind::SelectAll,
        ] {
            let pixmap = render_icon_pixmap(kind, Color::WHITE, 16);
            assert_eq!(pixmap.width(), 16);
            assert_eq!(pixmap.height(), 16);
            assert!(
                pixmap.pixels().iter().any(|p| p.alpha() > 0),
                "{kind:?} 栅格化结果不该是全透明的空图"
            );
        }
    }

    /// 着色遮罩语义:同一个图标用两种颜色栅格化,凡是不透明的像素,RGB
    /// 必须跟随传入颜色变化(忽略 SVG 自带颜色,只用其覆盖度当遮罩)——
    /// 对应 `iced` 侧 `svg::Style{color}` 的既有着色语义,原生菜单图标要
    /// 和 iced 里同一批图标看起来一致。
    #[test]
    fn render_icon_pixmap_recolors_by_alpha_mask() {
        let red = render_icon_pixmap(IconKind::Trash, Color::from_rgb(1.0, 0.0, 0.0), 16);
        let blue = render_icon_pixmap(IconKind::Trash, Color::from_rgb(0.0, 0.0, 1.0), 16);
        let idx = red
            .pixels()
            .iter()
            .position(|p| p.alpha() > 200)
            .expect("图标至少有一个接近不透明的像素");
        let red_px = red.pixels()[idx];
        let blue_px = blue.pixels()[idx];
        assert!(red_px.red() > red_px.blue(), "红色着色后该像素应偏红");
        assert!(blue_px.blue() > blue_px.red(), "蓝色着色后该像素应偏蓝");
    }
}
