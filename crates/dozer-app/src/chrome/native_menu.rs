//! macOS 原生右键菜单(NSMenu),替代会被 wry webview 遮挡的 iced 弹层
//! 弹层。详见 `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`。

#![cfg(target_os = "macos")]

use byteui::interaction::icons::IconKind;
use iced_widget::core::Color;

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadOnly};
use objc2_app_kit::{NSColor, NSImage, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};

/// 一条原生菜单描述——调用方只管拼数据,不碰 AppKit。
///
/// `color` 是**文字**颜色;`icon_color` 是**图标**独立着色(`None` = 跟随
/// `color`),给"图标和文字不同色"的菜单(如 agent 选择器:图标用 agent
/// 专属色、文字用 BODY)用。`enabled = false` 的项置灰且不接 hover/点击。
pub enum Item<Msg> {
    Entry {
        icon: Option<IconKind>,
        icon_color: Option<Color>,
        label: String,
        color: Color,
        enabled: bool,
        msg: Msg,
    },
    Separator,
}

impl<Msg> Item<Msg> {
    /// 常规可点项:文字与图标都用主题 BODY 色,图标可无。
    pub fn entry(icon: Option<IconKind>, label: impl Into<String>, msg: Msg) -> Self {
        let body = byteui::theme::color::current().body;
        Item::Entry {
            icon,
            icon_color: None,
            label: label.into(),
            color: body,
            enabled: true,
            msg,
        }
    }
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

/// 菜单项图标像素尺寸——同 `crate::chrome::menu.rs::icon_leading` 用的
/// `icon_size::row()`(已含全局 scale),原生菜单和 iced 菜单在同一次
/// 缩放调整下应保持一致大小。
fn icon_size_px() -> u32 {
    byteui::theme::icon_size::row().round() as u32
}

/// 菜单整行宽 + 左右 margin——`menu_item_view::MenuItemView` 的 hover 高亮
/// 和分隔线共用同一份数字,保证两者左右留白严格一致(而不是分隔线走
/// AppKit 系统默认的 `separatorItem` 内边距、高亮走另一个数,两条各自
/// 独立算出来的宽度对不上)。
fn row_geometry() -> (f64, f64) {
    let row_width = byteui::theme::geometry::menu_item_width() as f64;
    let margin = byteui::theme::geometry::menu_pad_h() as f64;
    (row_width, margin)
}

/// 分隔线整行高度(像素)——`show()` 估算菜单总高时也要用到,提到模块级
/// 常量避免和 `separator_view` 各算一份、数字漂移。
const SEP_ROW_HEIGHT: f64 = 9.0;

/// 常规项整行高度:图标/文字谁高就跟谁,再加上下 padding——和
/// `menu_item_view::MenuItemView::new` 实际起 frame 用的公式必须是同一份
/// (`show()` 估算菜单总高、动画外的那个函数造真实行高,两处算出来的数字
/// 对不上,`show()` 钳位时就会算少,菜单还是会超出窗口)。
fn entry_row_height() -> f64 {
    let font_size = byteui::theme::font::label() as f64;
    let pad_v = byteui::theme::geometry::menu_pad_v() as f64;
    let icon_px = icon_size_px() as f64;
    let text_height = font_size + 4.0;
    let content_height = icon_px.max(text_height);
    content_height + pad_v * 2.0
}

/// 分隔线:1px `BORDER` 色横线,左右各让开 `row_geometry` 的 margin
/// (`menu_pad_h`)——比 `menu_item_view::MenuItemView` 的 hover 高亮(让开
/// `menu_hover_inset`)更短一截,于是高亮矩形比分隔线略长(2026-09 用户
/// 口径)。不用 `NSMenuItem::separatorItem()` 的系统默认样式(那条线的
/// 内边距是系统控制的,跟自定义高亮的留白量对不上)。
fn separator_view(mtm: objc2::MainThreadMarker) -> Retained<NSView> {
    let (row_width, margin) = row_geometry();
    let container = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(row_width, SEP_ROW_HEIGHT),
        ),
    );
    let line = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(margin, (SEP_ROW_HEIGHT - 1.0) / 2.0),
            NSSize::new((row_width - margin * 2.0).max(0.0), 1.0),
        ),
    );
    line.setWantsLayer(true);
    if let Some(layer) = line.layer() {
        let border = byteui::theme::color::current().border;
        let ns_color = NSColor::colorWithRed_green_blue_alpha(
            border.r as f64,
            border.g as f64,
            border.b as f64,
            border.a as f64,
        );
        layer.setBackgroundColor(Some(&ns_color.CGColor()));
    }
    container.addSubview(&line);
    container
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
/// 一点——和现有 `crate::chrome::menu.rs` 弹层用的 `last_right_click()` 是同一份
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

    // 菜单预估尺寸(宽固定、高按行类型累加),用来把锚点钳制在内容 view
    // 范围内——`popUpMenuPositioningItem:atLocation:inView:` 只保证菜单
    // 不超出*屏幕*,不保证不超出*本应用窗口*;`view_pos`(`self.last_cursor`)
    // 离窗口右/下边缘较近时(如本例"新建 Agent"按钮就贴着 Agent 面板顶部),
    // 菜单会越过窗口边界悬在桌面上,观感上像是"跑出了窗体"。钳位公式与
    // `topbar.rs::project_add_menu_popup`(iced 弹层同类问题的既有解法)
    // 一致:锚点不超过"窗口尺寸 - 菜单尺寸"。
    let (pop_w, _margin) = row_geometry();
    let pop_h: f64 = items
        .iter()
        .map(|item| match item {
            Item::Separator => SEP_ROW_HEIGHT,
            Item::Entry { .. } => entry_row_height(),
        })
        .sum();
    let frame = view.frame();
    let clamped_x = (view_pos.0 as f64).clamp(0.0, (frame.size.width - pop_w).max(0.0));
    let clamped_y = (view_pos.1 as f64).clamp(0.0, (frame.size.height - pop_h).max(0.0));

    let menu = NSMenu::new(mtm);
    // 索引 → 消息的映射,`SELECTED_INDEX` 写回的下标据此取出对应 `Msg`。
    let mut msgs: Vec<Option<Msg>> = Vec::with_capacity(items.len());

    for (idx, item) in items.into_iter().enumerate() {
        match item {
            Item::Separator => {
                let sep_item = NSMenuItem::new(mtm);
                sep_item.setEnabled(false);
                sep_item.setView(Some(&separator_view(mtm)));
                menu.addItem(&sep_item);
                msgs.push(None);
            }
            Item::Entry {
                icon,
                icon_color,
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
                let icon_color = icon_color.unwrap_or(color);
                if let Some(icon) = icon {
                    ns_item.setImage(Some(&icon_image(icon, icon_color, icon_px)));
                }
                let row_view = menu_item_view::MenuItemView::new(
                    mtm,
                    &label,
                    color,
                    enabled,
                    icon.map(|k| icon_image(k, icon_color, icon_px)),
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
        NSPoint::new(clamped_x, clamped_y),
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
    use objc2_quartz_core::CALayer;
    use std::cell::Cell;

    /// hover 高亮的圆角半径,对齐 `crate::chrome::menu.rs` 里 iced 版菜单项的
    /// `MENU_HOVER_RADIUS`。
    const HOVER_RADIUS: f64 = 6.0;

    /// 自定义菜单项 view 的实例状态:命中的下标(点击时回填
    /// `SELECTED_INDEX` 用)+ 是否可点(锁定项不接 hover/点击)+ 当前是否
    /// 悬停(`mouseEntered:`/`mouseExited:` 翻转,驱动 `highlight`
    /// 高亮——`NSMenuItem` 挂了自定义 `view` 后 AppKit 不会自动画选中态,
    /// 必须自己维护这个状态并据此改层背景色,不能只靠 `setNeedsDisplay`
    /// (没有 `drawRect:` 覆写,那样什么也不会变)。
    /// `highlight` 是整行自身图层之上的一块独立子图层,左右各留出
    /// `menu_hover_inset` 的 margin 再画背景色——不能直接用整行自身的根图层
    /// (那样高亮会顶到行的左右边缘,贴着菜单外框,跟 iced 版菜单/系统原生
    /// 菜单的"高亮块比整行窄一圈"观感不一致)。这块 margin 比文字/分隔线的
    /// `menu_pad_h` 小,高亮矩形因此比分隔线略长、且与文字之间留出一圈左右
    /// 边距(见 `new` 里的说明)。
    pub struct Ivars {
        index: usize,
        enabled: bool,
        hovered: Cell<bool>,
        highlight: Retained<CALayer>,
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
        /// 按 `hovered` 当前值把 `highlight` 子图层背景设成 `TAB_HOVER` 色
        /// 或透明——`mouseEntered:`/`mouseExited:` 都调这个,保持单一落笔点。
        fn apply_hover_background(&self) {
            let highlight = &self.ivars().highlight;
            if self.ivars().hovered.get() {
                let hover = byteui::theme::color::current().tab_hover;
                let ns_color = NSColor::colorWithRed_green_blue_alpha(
                    hover.r as f64,
                    hover.g as f64,
                    hover.b as f64,
                    hover.a as f64,
                );
                highlight.setBackgroundColor(Some(&ns_color.CGColor()));
            } else {
                highlight.setBackgroundColor(None);
            }
        }

        /// 组一整行:自身画 hover 底色(靠 `wantsLayer`+`layer.backgroundColor`
        /// 更简单),内部横排图标(可选)+ 文字。宽/内边距/图标↔文字间距/
        /// 字号全部读 `byteui::theme` token(均已含全局 scale),和
        /// `crate::chrome::menu.rs` 的 iced 版菜单项共用同一套尺寸口径,放大/缩小
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
            let (row_width, pad_h) = super::row_geometry();
            let gap = byteui::theme::geometry::menu_gap() as f64;
            let icon_px = super::icon_size_px() as f64;
            // 文字行高留一点余量(字号本身只是字形高度,行框要比它高一圈
            // 才不会顶到边)。行高公式提到 `super::entry_row_height()`——
            // `show()` 估算菜单总高要用同一份公式,不然算少了钳位就防不住
            // 菜单超出窗口。
            let text_height = font_size + 4.0;
            let row_height = super::entry_row_height();
            // hover 高亮块左右各让开 `menu_hover_inset`——比文字/分隔线的
            // `menu_pad_h` 小,于是高亮矩形比分隔线略长,且高亮左/右缘与文字
            // 之间留出 `menu_pad_h - menu_hover_inset` 的边距(2026-09 用户
            // 反馈:原先高亮与分隔线同宽、左缘又和文字起点重合,文字紧贴高亮
            // 边缘、没有左右边距)。
            let highlight_margin = byteui::theme::geometry::menu_hover_inset() as f64;
            let highlight = CALayer::new();
            highlight.setFrame(NSRect::new(
                NSPoint::new(highlight_margin, 0.0),
                NSSize::new((row_width - highlight_margin * 2.0).max(0.0), row_height),
            ));
            highlight.setCornerRadius(HOVER_RADIUS);

            let this = mtm.alloc::<Self>().set_ivars(Ivars {
                index,
                enabled,
                hovered: Cell::new(false),
                highlight,
            });
            let this: Retained<Self> = unsafe {
                msg_send![
                    super(this),
                    initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(row_width, row_height))
                ]
            };

            this.setWantsLayer(true);
            if let Some(layer) = this.layer() {
                layer.addSublayer(&this.ivars().highlight);
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
            // 无论该行是否真的有图标,都给图标列预留 `icon_px + gap` 宽位,
            // 让文字起点恒等于 `pad_h + icon_px + gap`(macOS 原生菜单同款:
            // 图标是固定左列,无图标的项文字也对齐到同一列)。tab 溢出菜单
            // 借此把"选中项前置 `>`"落进这个固定列,未选中行不画图标、文字
            // 仍与选中行对齐,不会出现选中行被箭头右推错位。
            x += icon_px + gap;
            if let Some(icon) = icon {
                let icon_y = (row_height - icon_px) / 2.0;
                let image_view = NSImageView::initWithFrame(
                    NSImageView::alloc(mtm),
                    NSRect::new(NSPoint::new(pad_h, icon_y), NSSize::new(icon_px, icon_px)),
                );
                image_view.setImage(Some(&icon));
                this.addSubview(&image_view);
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
