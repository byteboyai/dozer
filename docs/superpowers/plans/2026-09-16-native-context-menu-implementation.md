# 右键菜单/输入框菜单原生化(NSMenu)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Files 右键菜单、Project 链接右键菜单、输入框剪切/复制/粘贴菜单(`text_input_menu`)在 macOS 上改用原生 `NSMenu` 弹出,彻底消除和 wry webview 的 z-order 冲突,不再需要隐藏 webview 让路;非 mac 平台保留改造前的行为不变。

**Architecture:** 新增 `crates/dozer-app/src/native_menu.rs`(`#[cfg(target_os = "macos")]`),提供 `Item<Msg>` 数据枚举 + 同步阻塞的 `show()`(内部用 `objc2-app-kit` 搭 `NSMenu` + 自定义 `NSMenuItem` view 还原 ByteBoy2077 主题,`popUpMenuPositioningItem_atLocation_inView` 阻塞直到用户选中/取消)。图标用已在依赖树里的 `resvg`/`usvg`/`tiny-skia`(`Cargo.lock` 确认 0.45.1/0.45.1/0.11.4)在运行时把内嵌 Lucide SVG 栅格化成指定颜色的 PNG,再用 `NSImage::initWithData` 包成图标。三处触发点(Files/Project/输入框)在各自的消息处理里同步调 `native_menu::show`,拿到结果后直接处理或递归 `update`——**不删除**现有的 `Option<XxxMenu>` 状态字段/`xxx_menu_popup` 渲染函数/`webview_hidden_by_panel_popup` 隐藏判断(spec 原计划要删,实现阶段发现 Rust 的 `#[cfg]` 不能挂在 `if/else if` 链的单个分支上,删字段会牵连一次有风险的 if-chain 重构,收益为零——不设置这些状态就等价于它们永远是 `None`,旧渲染分支天然不会触发,是更小、更安全的等价实现;需要真正删除这些 mac 侧死代码可以另开一个清理 plan)。

**Tech Stack:** Rust,`objc2`/`objc2-app-kit`/`objc2-foundation`(已有依赖),新增 `resvg`/`usvg`/`tiny-skia`(pin 到 `Cargo.lock` 现有版本 0.45.1/0.45.1/0.11.4,`tiny-skia` 开 `png` feature)。

**Spec:** `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`

## Global Constraints

- `native_menu.rs` 整体 `#[cfg(target_os = "macos")]`;三处调用点用 `#[cfg(target_os = "macos")]`/`#[cfg(not(target_os = "macos"))]` 分叉,非 mac 分支逐字保留现有代码,不改动其行为。
- 新增 `resvg`/`usvg`/`tiny-skia` 依赖必须 pin 到 `Cargo.lock` 里已有的精确版本(`resvg 0.45.1`/`usvg 0.45.1`/`tiny-skia 0.11.4`),不得引入新的次版本,避免和 `iced_wgpu` 间接依赖的版本产生重复编译。
- 不改动 Conversations agent 筛选下拉的隐藏逻辑(`webview_hidden_by_panel_popup` 的 `PanelKind::Conversations` 分支),不改动其余 7 处 `crate::menu.rs` 消费方。
- `cargo clippy --all-targets && cargo fmt` 必须通过。
- 独立分支开发,完工后经审阅再合并 main(同 `[[feedback-plans-use-worktree-branch]]`)。
- 原生 AppKit 交互(自定义 view 绘制/hover/点击追踪)没有自动化测试路径,人工在真机 `cargo run -p dozer-app` 验证,同仓库里 `NSOpenPanel`/交通灯覆写的既有先例。

---

## File Structure

- `crates/dozer-app/src/native_menu.rs`(新建,`#[cfg(target_os = "macos")]`):`Item<Msg>` 枚举、图标栅格化(纯函数,可测)、`show()`(AppKit,不可自动化测试)、`install_content_view()`(main.rs 窗口初始化时调一次)。
- `crates/dozer-app/src/main.rs`:`mod native_menu;` 声明;`install_content_view` 调用点(紧邻 `install_topbar_drag_guard`/`install_file_drag_position_tracker`);`menu_edit_key` 改 `pub(crate)`;`window_event` 里 `for message in pending_messages { self.dispatch(message); }` 之后新增一段"消费 `pending_native_menu_edit_key`、重建 `UserInterface` 补合成键盘事件"的逻辑。
- `crates/dozer-app/src/extensions/files.rs`:新增 `context_menu_items()` 纯函数;`Message::ContextMenuOpen` 分支按平台分叉。
- `crates/dozer-app/src/app.rs`:新增 `project_link_menu_items()`/`text_input_menu_items()` 纯函数;`project_link_context_menu()`/`Message::TextInputMenuOpen` 分支按平台分叉;新增 `pending_native_menu_edit_key` 字段 + `take_pending_native_menu_edit_key()`。
- `crates/dozer-app/Cargo.toml`:新增 `resvg`/`usvg`/`tiny-skia` 依赖(`target.'cfg(target_os = "macos")'.dependencies` 段,和 `objc2` 系依赖同一处理方式)。

---

## Task 1: `native_menu::Item` + 图标栅格化(纯函数,可测)

**Files:**
- Create: `crates/dozer-app/src/native_menu.rs`
- Modify: `crates/dozer-app/Cargo.toml`(加依赖)
- Modify: `crates/dozer-app/src/main.rs`(加 `mod native_menu;` 声明,放在其它 `mod xxx;` 声明旁)

**Interfaces:**
- Produces: `pub enum Item<Msg> { Entry { icon: Option<byteui::interaction::icons::IconKind>, label: String, color: iced_widget::core::Color, enabled: bool, msg: Msg }, Separator }`;`fn render_icon_pixmap(kind: IconKind, color: Color, size_px: u32) -> tiny_skia::Pixmap`(mod 内可见,Task 2 用)。

- [ ] **Step 1: 加依赖**

在 `crates/dozer-app/Cargo.toml` 里,找到已有的 `[target.'cfg(target_os = "macos")'.dependencies]` 段(`objc2`/`objc2-app-kit`/`objc2-foundation` 所在段),追加:

```toml
resvg = "=0.45.1"
usvg = "=0.45.1"
tiny-skia = { version = "=0.11.4", features = ["png"] }
```

- [ ] **Step 2: 验证依赖解析不引入新版本**

Run: `cargo tree -p dozer-app -i resvg -i usvg -i tiny-skia 2>&1 | head -30`
Expected: 只出现 `0.45.1`/`0.45.1`/`0.11.4` 这一组版本号(和 `iced_wgpu` 间接依赖的版本一致),没有第二个版本号出现。

- [ ] **Step 3: 写失败的测试**

创建 `crates/dozer-app/src/native_menu.rs`,先写测试(用 `#[cfg(test)] mod tests`):

```rust
//! macOS 原生右键菜单(NSMenu),替代会被 wry webview 遮挡的 iced 弹层
//! 弹层。详见 `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`。

#![cfg(target_os = "macos")]

use byteui::interaction::icons::IconKind;
use iced_widget::core::Color;

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
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib native_menu`
Expected: 编译失败,`cannot find function 'render_icon_pixmap' in this scope`。

- [ ] **Step 3: 写最小实现**

在 `Item` 枚举定义之后追加:

```rust
/// 把内嵌 Lucide SVG(`IconKind::bytes()`)按给定颜色栅格化成
/// `size_px × size_px` 的位图——渲染出来的 alpha 通道当遮罩,RGB 统一替换
/// 成 `color`(同 iced 侧 `svg::Style{color}` 的着色语义,忽略 SVG 自身
/// 颜色)。纯函数,不碰 AppKit,可在任何线程/CI 里跑。
fn render_icon_pixmap(kind: IconKind, color: Color, size_px: u32) -> tiny_skia::Pixmap {
    let opt = usvg::Options::default();
    let tree =
        usvg::Tree::from_data(kind.bytes(), &opt).expect("内嵌 Lucide SVG 资源必须能解析");
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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib native_menu`
Expected: 2 个用例 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/native_menu.rs crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): native_menu 模块骨架 + 图标栅格化

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `native_menu::show()` —— NSMenu + 自定义主题 view + 内容视图注册

**Files:**
- Modify: `crates/dozer-app/src/native_menu.rs`
- Modify: `crates/dozer-app/src/main.rs`(`install_content_view` 调用点)

**Interfaces:**
- Consumes: Task 1 的 `Item<Msg>`、`render_icon_pixmap`。
- Produces: `pub fn install_content_view(window: &winit::window::Window)`(main.rs 窗口初始化时调一次);`pub fn show<Msg: Clone>(items: Vec<Item<Msg>>, view_pos: (f32, f32)) -> Option<Msg>`(Task 4/5 用)。

> 这一步是纯 AppKit 交互代码,没有自动化测试路径(同仓库 `NSOpenPanel`/交通灯覆写的既有做法)。Step 顺序按"先能编译过、再能真的弹出菜单"分解,每步用 `cargo build` 兜底,最终手测放在 Task 6。

- [ ] **Step 1: 内容视图注册(照抄 `FILE_DRAG_CONTENT_VIEW` 的既有模式)**

在 `native_menu.rs` 顶部追加:

```rust
use objc2_app_kit::NSView;

/// 弹菜单时要挂靠的窗口内容 view——`show()` 是个不持有 `winit::window::Window`
/// 的自由函数,够不到窗口句柄,故在窗口初始化时存一份裸指针在这里(同
/// `main.rs::FILE_DRAG_CONTENT_VIEW` 的既有模式,理由一致:原生回调/独立
/// 调用点没有 `Self::Ready` 字段可用)。
static CONTENT_VIEW: std::sync::Mutex<Option<*mut NSView>> = std::sync::Mutex::new(None);

/// main.rs 窗口初始化时调一次(紧邻 `install_topbar_drag_guard`/
/// `install_file_drag_position_tracker` 的调用点),记下内容 view 供
/// `show()` 后续弹菜单用。
///
/// # Safety
/// 调用方保证传入的是已经装进窗口、生命周期覆盖整个应用运行期的窗口
/// (同 `install_topbar_drag_guard` 对 `ah.ns_view` 的既有假设)。
pub fn install_content_view(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(ah) = handle.as_raw() else {
        return;
    };
    let ptr = ah.ns_view.as_ptr() as *mut NSView;
    *CONTENT_VIEW.lock().unwrap() = Some(ptr);
}

fn content_view() -> Option<&'static NSView> {
    let ptr = (*CONTENT_VIEW.lock().unwrap())?;
    // 安全性同 `main.rs` 里其它读取该类裸指针的场景:只借引用去调只读/
    // 弹层方法,不持有/释放。
    Some(unsafe { &*ptr })
}
```

- [ ] **Step 2: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 通过(此时 `install_content_view`/`content_view` 还没有调用方,只要没有 `unused` 报错级别的问题——若 clippy 警告未使用,属预期,下一步接线后消失)。

- [ ] **Step 3: main.rs 接入 `install_content_view` 调用点**

在 `main.rs` 里 `install_topbar_drag_guard(&window);`/`install_file_drag_position_tracker(&window);` 调用之后(约第 2311-2315 行附近)追加:

```rust
#[cfg(target_os = "macos")]
native_menu::install_content_view(&window);
```

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 通过,无 unused 警告。

- [ ] **Step 5: 点击结果的共享槽位 + 图标缓存**

在 `native_menu.rs` 追加:

```rust
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSImage;
use objc2_foundation::NSData;
use std::collections::HashMap;

/// 一次 `show()` 调用期间,自定义 item view 的 `mouseUp:` 覆写把"选中了
/// 第几项"写在这里;`popUpMenuPositioningItem_atLocation_inView` 返回后
/// 读一次、立刻清空。是线程内单发的槽位(菜单是同步阻塞的模态追踪,不会
/// 有第二个 `show()` 并发进行),不需要跟 `CONTENT_VIEW` 一样长期持有。
static SELECTED_INDEX: std::sync::Mutex<Option<usize>> = std::sync::Mutex::new(None);

/// 按 `(IconKind, 颜色 ARGB, 像素尺寸)` 缓存栅格化结果,避免同一个图标
/// 每次弹菜单都重新过一遍 `resvg`。整条调用链限定在主线程(AppKit 要求),
/// 用普通 `HashMap` + 惰性 `static` 即可,不需要跨线程同步原语。
fn icon_cache() -> &'static std::sync::Mutex<HashMap<(IconKind, u32, u32), Retained<NSImage>>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<HashMap<(IconKind, u32, u32), Retained<NSImage>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
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

/// 把 `render_icon_pixmap` 的结果编码成 PNG、包成 `NSImage`(经
/// `NSData::with_bytes` + `NSImage::initWithData`,比手搭
/// `NSBitmapImageRep` 的裸像素平面初始化器简单可靠——PNG 编解码本身处理
/// 好了预乘 alpha 的语义)。命中缓存直接返回。
fn icon_image(mtm: objc2::MainThreadMarker, kind: IconKind, color: Color, size_px: u32) -> Retained<NSImage> {
    let key = (kind, color_key(color), size_px);
    if let Some(img) = icon_cache().lock().unwrap().get(&key) {
        return img.clone();
    }
    let pixmap = render_icon_pixmap(kind, color, size_px);
    let png = pixmap.encode_png().expect("tiny-skia PNG 编码不应失败");
    let data = NSData::with_bytes(&png);
    let image = unsafe { NSImage::alloc(mtm) }
        .initWithData(&data)
        .expect("PNG 编码出的数据必须能被 NSImage 解出来");
    icon_cache().lock().unwrap().insert(key, image.clone());
    image
}
```

- [ ] **Step 6: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 通过。

- [ ] **Step 7: `show()` 本体 —— 组装 `NSMenu` + 自定义主题 item view**

追加(这是整个模块里唯一直接和用户交互的入口):

```rust
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::{NSPoint, NSString};

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
                if let Some(icon) = icon {
                    ns_item.setImage(Some(&icon_image(mtm, icon, color, 16)));
                }
                let row_view = menu_item_view::new(mtm, &label, color, enabled, icon.map(|k| icon_image(mtm, k, color, 16)), idx);
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
```

> `ns_item.setImage(...)` 这一行是给"没有自定义 view 生效"时的兜底(理论上设了 `setView` 之后系统不会再画 `image`/`title`,但留着不冲突,万一 `menu_item_view` 初始化失败导致 `setView` 传 `None` 时还能看到图标+文字这条兜底路径,不是必须删除的重复代码)。

- [ ] **Step 8: 编译确认(`menu_item_view` 尚未实现,预期报错)**

Run: `cargo build -p dozer-app 2>&1 | tail -20`
Expected: 报错 `cannot find function 'new' in module 'menu_item_view'`(或 `unresolved module`)——下一步补上。

- [ ] **Step 9: 自定义 `NSMenuItem` view(hover 高亮 + 点击回填 `SELECTED_INDEX`)**

`NSMenuItem` 挂了自定义 `view` 之后,标准 target-action 点击不会自动触发——这是已知的 AppKit 行为,常规解法是在自定义 view 里覆写 `mouseUp:` 手动结束追踪并记录结果,`mouseEntered:`/`mouseExited:` 靠 `NSTrackingArea` 做 hover 高亮。用运行时动态创建一个新类(同 `main.rs::install_file_drag_position_tracker` 给 `draggingUpdated:` 挂覆写的技术,这里是**新建类**不是**给已有类挂方法**,用 `objc2::declare`/`ClassBuilder` 更直接)。

追加一个子模块:

```rust
mod menu_item_view {
    use super::{Color, SELECTED_INDEX};
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{DefinedClass, MainThreadMarker, define_class, msg_send};
    use objc2_app_kit::{
        NSColor, NSEvent, NSImage, NSImageView, NSTextField, NSTrackingArea,
        NSTrackingAreaOptions, NSView,
    };
    use objc2_foundation::{MainThreadOnly, NSObjectProtocol, NSRect};
    use std::cell::Cell;

    /// 自定义菜单项 view 的实例状态:命中的下标(点击时回填
    /// `SELECTED_INDEX` 用)+ 是否可点(锁定项不接 hover/点击)。
    pub struct Ivars {
        index: usize,
        enabled: bool,
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
                    self.setNeedsDisplay(true);
                }
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {
                self.setNeedsDisplay(true);
            }

            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, _event: &NSEvent) {
                if !self.ivars().enabled {
                    return;
                }
                *SELECTED_INDEX.lock().unwrap() = Some(self.ivars().index);
                if let Some(menu) = self.enclosingMenuItem().and_then(|item| item.menu()) {
                    unsafe { menu.cancelTrackingWithoutAnimation() };
                }
            }
        }
    );

    impl MenuItemView {
        /// 组一整行:自身画 hover 底色(`drawRect:` 走系统默认矩形填充,
        /// 靠 `wantsLayer`+`layer.backgroundColor` 更简单,这里用后者),
        /// 内部横排图标(可选)+ 文字。
        pub fn new(
            mtm: MainThreadMarker,
            label: &str,
            color: Color,
            enabled: bool,
            icon: Option<Retained<NSImage>>,
            index: usize,
        ) -> Retained<Self> {
            const ROW_HEIGHT: f64 = 22.0;
            const ROW_WIDTH: f64 = 220.0;
            let this = mtm.alloc::<Self>().set_ivars(Ivars { index, enabled });
            let this: Retained<Self> =
                unsafe { msg_send![super(this), initWithFrame: NSRect::new(objc2_foundation::NSPoint::ZERO, objc2_foundation::NSSize::new(ROW_WIDTH, ROW_HEIGHT))] };

            unsafe { this.setWantsLayer(true) };
            let tracking = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    this.bounds(),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways,
                    Some(&this),
                    None,
                )
            };
            unsafe { this.addTrackingArea(&tracking) };

            let mut x = 8.0;
            if let Some(icon) = icon {
                let image_view = unsafe {
                    NSImageView::initWithFrame(
                        NSImageView::alloc(mtm),
                        NSRect::new(objc2_foundation::NSPoint::new(x, 3.0), objc2_foundation::NSSize::new(16.0, 16.0)),
                    )
                };
                image_view.setImage(Some(&icon));
                unsafe { this.addSubview(&image_view) };
                x += 22.0;
            }
            let text = unsafe {
                NSTextField::initWithFrame(
                    NSTextField::alloc(mtm),
                    NSRect::new(
                        objc2_foundation::NSPoint::new(x, 2.0),
                        objc2_foundation::NSSize::new(ROW_WIDTH - x - 8.0, 18.0),
                    ),
                )
            };
            text.setStringValue(&objc2_foundation::NSString::from_str(label));
            text.setBezeled(false);
            text.setDrawsBackground(false);
            text.setEditable(false);
            text.setSelectable(false);
            let ns_color = unsafe {
                NSColor::colorWithRed_green_blue_alpha(
                    color.r as f64,
                    color.g as f64,
                    color.b as f64,
                    color.a as f64,
                )
            };
            text.setTextColor(Some(&ns_color));
            unsafe { this.addSubview(&text) };

            this
        }
    }
}
```

> `objc2` 0.6 的 `define_class!` 宏语法/`Ivars`/`DefinedClass` trait 的确切写法请对照 `Cargo.lock` 里 pin 的 `objc2 = "0.6"` 版本文档核对(`cargo doc -p objc2 --open`)——这是这份代码库第一次用 `define_class!` 新建一个 AppKit 子类(此前的 `mouse_down_can_move_window`/`draggingUpdated:` 都是"给已有类挂一个方法",不是新建类),宏的具体字段/属性名如果和这里写的不完全一致,以编译器报错为准逐字段修正,不是架构问题。`NSTrackingArea`/`NSImageView`/`NSTextField` 的 `initWithFrame`/`setXxx` 方法名同理,以 `objc2-app-kit 0.3.2` 生成的绑定为准。

- [ ] **Step 10: 编译,按报错逐个修正 API 名**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 第一轮大概率会有几处 `objc2-app-kit`/`objc2` 方法名或 `define_class!` 语法对不上,逐条按报错信息修正(改方法名/加 `unsafe`/调整 `Ivars` 写法),直到编译通过。这一步是本 plan 里唯一允许"实现和这里写的代码不完全一致"的地方——修正后如果调用点(`Item`/`show` 的对外签名)没变,不影响 Task 3-5。

- [ ] **Step 11: `cargo clippy`**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -40`
Expected: 无新增警告(`unsafe` 块较多是预期的,不是警告)。

- [ ] **Step 12: Commit**

```bash
git add crates/dozer-app/src/native_menu.rs crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): native_menu::show —— NSMenu + 自定义主题菜单项 view

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 三处调用点的纯 "items 组装" 函数 + 单测

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`(新增 `context_menu_items`)
- Modify: `crates/dozer-app/src/app.rs`(新增 `project_link_menu_items`/`text_input_menu_items`)

**Interfaces:**
- Consumes: Task 1 的 `native_menu::Item<Msg>`。
- Produces: `files::context_menu_items(target: &Path, is_dir: bool, is_root: bool, has_clipboard: bool) -> Vec<native_menu::Item<files::Message>>`;`app::project_link_menu_items(target: project::links::LinkTarget, index: usize) -> Vec<native_menu::Item<Message>>`;`app::text_input_menu_items(target: &TextInputTarget) -> Vec<native_menu::Item<Message>>`。Task 4/5 直接调用这三个函数。

- [ ] **Step 1: 写失败的测试(files.rs)**

在 `extensions/files.rs` 的 `#[cfg(test)] mod tests` 里追加:

```rust
    #[test]
    fn context_menu_items_hides_delete_rename_for_root() {
        let items = context_menu_items(Path::new("/proj"), true, true, false);
        let has_delete = items.iter().any(|i| matches!(
            i,
            crate::native_menu::Item::Entry { msg: Message::DeleteRequest(..), .. }
        ));
        assert!(!has_delete, "项目根目录不该出现删除项");
    }

    #[test]
    fn context_menu_items_shows_delete_rename_for_non_root() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, false);
        let has_delete = items.iter().any(|i| matches!(
            i,
            crate::native_menu::Item::Entry { msg: Message::DeleteRequest(..), .. }
        ));
        assert!(has_delete, "非根目录该有删除项");
    }

    #[test]
    fn context_menu_items_paste_locked_when_clipboard_empty() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, false);
        let paste = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry { msg: Message::Paste(_), enabled, .. } => Some(*enabled),
            _ => None,
        });
        assert_eq!(paste, Some(false), "剪贴槽为空时粘贴该锁定");
    }

    #[test]
    fn context_menu_items_paste_enabled_when_clipboard_has_content() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, true);
        let paste = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry { msg: Message::Paste(_), enabled, .. } => Some(*enabled),
            _ => None,
        });
        assert_eq!(paste, Some(true));
    }

    #[test]
    fn context_menu_items_file_target_has_no_new_file_or_paste() {
        let items = context_menu_items(Path::new("/proj/src/main.rs"), false, false, false);
        let has_new_file = items.iter().any(|i| matches!(
            i,
            crate::native_menu::Item::Entry { msg: Message::NewFile(_), .. }
        ));
        let has_paste = items.iter().any(|i| matches!(
            i,
            crate::native_menu::Item::Entry { msg: Message::Paste(_), .. }
        ));
        assert!(!has_new_file && !has_paste, "非目录不该有新建文件/粘贴");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib files::tests::context_menu_items`
Expected: 编译失败,`context_menu_items` 未定义。

- [ ] **Step 3: 写最小实现(files.rs)**

在 `context_menu_popup` 函数**之前**插入(照抄其条件分支,只是从"构造 `Element`"改成"构造 `native_menu::Item`"):

```rust
/// `context_menu_popup` 的原生菜单版本——纯数据组装,不碰渲染/AppKit,和
/// 旧版共用完全相同的条件分支(是否目录/是否根/是否有剪贴内容),方便
/// 单测覆盖,行为上二者应保持一致。
pub fn context_menu_items(
    target: &std::path::Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> Vec<crate::native_menu::Item<Message>> {
    use crate::native_menu::Item;
    let body = byteui::theme::color::current().body;
    let dim = byteui::theme::color::current().dim;
    let target = target.to_path_buf();
    let mut items = vec![
        Item::Entry {
            icon: Some(icons::IconKind::Search),
            label: "搜索".into(),
            color: body,
            enabled: true,
            msg: Message::OpenSearch(target.clone(), is_dir),
        },
        Item::Separator,
    ];
    if is_dir {
        items.push(Item::Entry {
            icon: Some(icons::IconKind::FilePlus),
            label: "新建文件".into(),
            color: body,
            enabled: true,
            msg: Message::NewFile(target.clone()),
        });
        items.push(Item::Entry {
            icon: Some(icons::IconKind::FolderPlus),
            label: "新建文件夹".into(),
            color: body,
            enabled: true,
            msg: Message::NewFolder(target.clone()),
        });
        items.push(Item::Separator);
    }
    items.push(Item::Entry {
        icon: Some(icons::IconKind::Copy),
        label: "复制".into(),
        color: body,
        enabled: true,
        msg: Message::Copy(target.clone(), is_dir),
    });
    if is_dir {
        items.push(Item::Entry {
            icon: Some(icons::IconKind::ClipboardPaste),
            label: "粘贴".into(),
            color: if has_clipboard { body } else { dim },
            enabled: has_clipboard,
            msg: Message::Paste(target.clone()),
        });
    }
    if !is_root {
        items.push(Item::Entry {
            icon: Some(icons::IconKind::Trash),
            label: "删除".into(),
            color: body,
            enabled: true,
            msg: Message::DeleteRequest(target.clone(), is_dir),
        });
        items.push(Item::Entry {
            icon: Some(icons::IconKind::Rename),
            label: "重命名".into(),
            color: body,
            enabled: true,
            msg: Message::RenameStart(target.clone()),
        });
    }
    items.push(Item::Separator);
    items.push(Item::Entry {
        icon: None,
        label: "复制绝对路径".into(),
        color: body,
        enabled: true,
        msg: Message::CopyPath(target.clone(), crate::project::PathKind::Absolute),
    });
    items.push(Item::Entry {
        icon: None,
        label: "复制相对路径".into(),
        color: body,
        enabled: true,
        msg: Message::CopyPath(target.clone(), crate::project::PathKind::Relative),
    });
    items.push(Item::Entry {
        icon: Some(icons::IconKind::FolderOpen),
        label: "在 Finder 中打开".into(),
        color: body,
        enabled: true,
        msg: Message::RevealInFinder(target.clone()),
    });
    items.push(Item::Entry {
        icon: Some(icons::IconKind::RefreshCw),
        label: "从磁盘重新加载".into(),
        color: body,
        enabled: true,
        msg: Message::ReloadFromDisk,
    });
    items
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib files::tests::context_menu_items`
Expected: 5 个用例 PASS。

- [ ] **Step 5: 写失败的测试(app.rs —— project_link_menu_items / text_input_menu_items)**

在 `app.rs` 的 `mod tests` 里追加:

```rust
    #[test]
    fn project_link_menu_items_has_single_delete_entry() {
        let items = project_link_menu_items(project::links::LinkTarget::Docs, 2);
        assert_eq!(items.len(), 1);
        assert!(matches!(
            items[0],
            crate::native_menu::Item::Entry {
                msg: Message::Project(project::Message::LinkRemove { index: 2, .. }),
                ..
            }
        ));
    }

    #[test]
    fn text_input_menu_items_locks_cut_copy_for_secure_field() {
        let target = TextInputTarget {
            id: iced_widget::core::widget::Id::new("x"),
            secure: true,
        };
        let items = text_input_menu_items(&target);
        let cut_enabled = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry { msg: Message::TextInputMenuCut, enabled, .. } => {
                Some(*enabled)
            }
            _ => None,
        });
        assert_eq!(cut_enabled, Some(false));
    }

    #[test]
    fn text_input_menu_items_enables_cut_copy_for_normal_field() {
        let target = TextInputTarget {
            id: iced_widget::core::widget::Id::new("x"),
            secure: false,
        };
        let items = text_input_menu_items(&target);
        let cut_enabled = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry { msg: Message::TextInputMenuCut, enabled, .. } => {
                Some(*enabled)
            }
            _ => None,
        });
        assert_eq!(cut_enabled, Some(true));
    }

    #[test]
    fn text_input_menu_items_always_has_four_entries() {
        let target = TextInputTarget {
            id: iced_widget::core::widget::Id::new("x"),
            secure: false,
        };
        assert_eq!(text_input_menu_items(&target).len(), 4);
    }
```

(检查 `project::links::LinkTarget` 是否有一个类似 `Docs` 的简单变体可直接构造;如果它的变体都带字段/不可枚举出一个零参数值,改用该类型现成的测试用构造方式——具体看 `project.rs` 里 `LinkTarget` 的定义,选一个能直接写字面量的变体。)

- [ ] **Step 6: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib app::tests::project_link_menu_items app::tests::text_input_menu_items`
Expected: 编译失败,函数未定义。

- [ ] **Step 7: 写最小实现(app.rs)**

在 `project_link_context_menu_popup` 方法**之前**插入:

```rust
/// `project_link_context_menu_popup` 的原生菜单版本,纯数据组装。
fn project_link_menu_items(
    target: project::links::LinkTarget,
    index: usize,
) -> Vec<native_menu::Item<Message>> {
    vec![native_menu::Item::Entry {
        icon: Some(icons::IconKind::Trash),
        label: "删除".into(),
        color: byteui::theme::color::current().body,
        enabled: true,
        msg: Message::Project(project::Message::LinkRemove { target, index }),
    }]
}
```

在 `text_input_menu_popup` 方法**之前**插入:

```rust
/// `text_input_menu_popup` 的原生菜单版本,纯数据组装——密码框场景锁定
/// 剪切/复制,同旧版语义。
fn text_input_menu_items(target: &TextInputTarget) -> Vec<native_menu::Item<Message>> {
    use native_menu::Item;
    let body = byteui::theme::color::current().body;
    let dim = byteui::theme::color::current().dim;
    let (cut_copy_color, cut_copy_enabled) = if target.secure {
        (dim, false)
    } else {
        (body, true)
    };
    vec![
        Item::Entry {
            icon: Some(icons::IconKind::Scissors),
            label: "剪切".into(),
            color: cut_copy_color,
            enabled: cut_copy_enabled,
            msg: Message::TextInputMenuCut,
        },
        Item::Entry {
            icon: Some(icons::IconKind::Copy),
            label: "复制".into(),
            color: cut_copy_color,
            enabled: cut_copy_enabled,
            msg: Message::TextInputMenuCopy,
        },
        Item::Entry {
            icon: Some(icons::IconKind::ClipboardPaste),
            label: "粘贴".into(),
            color: body,
            enabled: true,
            msg: Message::TextInputMenuPaste,
        },
        Item::Entry {
            icon: Some(icons::IconKind::SelectAll),
            label: "全选".into(),
            color: body,
            enabled: true,
            msg: Message::TextInputMenuSelectAll,
        },
    ]
}
```

- [ ] **Step 8: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib`
Expected: 全量测试 PASS(含本任务新增的 8 条)。

- [ ] **Step 9: `clippy`/`fmt`**

Run: `cargo clippy --all-targets && cargo fmt`

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions/files.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
test(dozer-app): 三处原生菜单的纯 items 组装函数 + 单测

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: 接入 Files 右键菜单 + Project 链接右键菜单(mac 走原生,非 mac 不变)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs:953-964`(`Message::ContextMenuOpen` 分支)
- Modify: `crates/dozer-app/src/app.rs:4340-4359`(`project_link_context_menu` 方法)

**Interfaces:**
- Consumes: Task 3 的 `context_menu_items`/`project_link_menu_items`;Task 2 的 `native_menu::show`。

- [ ] **Step 1: 修改 `files.rs` 的 `ContextMenuOpen` 分支**

把:

```rust
        Message::ContextMenuOpen { path, is_dir } => {
            let (x, y) = app_state.last_right_click;
            ws_state.tree_selected = Some(path.clone());
            // 与分支切换弹层互斥:开右键菜单时收起分支弹层,避免两个浮层
            // 同时挂着(同 `PreviewTabContextMenu` 关文件树右键菜单的约定)。
            ws_state.branch_picker_open = false;
            app_state.context_menu = Some(ContextMenu {
                x,
                y,
                target: path,
                is_dir,
            });
        }
```

改成:

```rust
        Message::ContextMenuOpen { path, is_dir } => {
            ws_state.tree_selected = Some(path.clone());
            // 与分支切换弹层互斥:开右键菜单时收起分支弹层,避免两个浮层
            // 同时挂着(同 `PreviewTabContextMenu` 关文件树右键菜单的约定)。
            ws_state.branch_picker_open = false;
            #[cfg(target_os = "macos")]
            {
                let (x, y) = app_state.last_right_click;
                let is_root = ws_state
                    .file_tree
                    .as_ref()
                    .map(|t| t.root() == path.as_path())
                    .unwrap_or(false);
                let has_clipboard = ws_state.tree_clipboard.is_some();
                let items = context_menu_items(&path, is_dir, is_root, has_clipboard);
                if let Some(msg) = crate::native_menu::show(items, (x, y)) {
                    update(ws_state, app_state, msg, project_id, handle, emit);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let (x, y) = app_state.last_right_click;
                app_state.context_menu = Some(ContextMenu {
                    x,
                    y,
                    target: path,
                    is_dir,
                });
            }
        }
```

- [ ] **Step 2: 修改 `app.rs` 的 `project_link_context_menu`**

把:

```rust
    fn project_link_context_menu(&mut self, target: project::links::LinkTarget, index: usize) {
        let (x, y) = self.files.last_right_click();
        self.files.close_context_menu();
        self.project_link_menu = Some(ProjectLinkMenu {
            x,
            y,
            target,
            index,
        });
        // 右击即选中该行:从对应链接列表取下标项路径,标记到
        // `project_panel.selected_link`(参考文件树 `ContextMenuOpen` 同时选中)。
        if let Some(path) = self
            .active_workspace()
            .and_then(|ws| ws.project_panel.link_path_at(target, index))
        {
            self.with_focused_project(|ws, _io| {
                ws.project_panel.set_selected_link(path);
            });
        }
    }
```

改成:

```rust
    fn project_link_context_menu(&mut self, target: project::links::LinkTarget, index: usize) {
        self.files.close_context_menu();
        // 右击即选中该行:从对应链接列表取下标项路径,标记到
        // `project_panel.selected_link`(参考文件树 `ContextMenuOpen` 同时选中)。
        if let Some(path) = self
            .active_workspace()
            .and_then(|ws| ws.project_panel.link_path_at(target, index))
        {
            self.with_focused_project(|ws, _io| {
                ws.project_panel.set_selected_link(path);
            });
        }
        #[cfg(target_os = "macos")]
        {
            let (x, y) = self.files.last_right_click();
            let items = project_link_menu_items(target, index);
            if let Some(msg) = native_menu::show(items, (x, y)) {
                self.update(msg);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (x, y) = self.files.last_right_click();
            self.project_link_menu = Some(ProjectLinkMenu {
                x,
                y,
                target,
                index,
            });
        }
    }
```

- [ ] **Step 3: 编译 + 跑全量测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: 编译通过,全量测试 PASS(注意:非 mac 分支这台开发机上编不了/跑不了,`#[cfg(not(target_os = "macos"))]` 分支只能靠代码审阅确认逐字未改,不必也无法在本机验证)。

- [ ] **Step 4: `clippy`/`fmt`**

Run: `cargo clippy --all-targets && cargo fmt`

- [ ] **Step 5: 人工验证**

`cargo run -p dozer-app`:
1. Files 面板打开一个文件预览(webview 可见),右键文件树的一个非根文件/目录——菜单应完整浮在最上层,不被下方 webview 挡住;webview 全程保持可见、不闪烁。分别测试根目录(无删除/重命名项)、有/无剪贴内容时"粘贴"项的置灰态。
2. Project 面板预览一个链接目标(webview 可见),右键一条链接行——"删除"菜单完整可见,点击后该链接确实被移除。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/files.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): Files/Project 右键菜单接入原生 NSMenu(mac)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: 接入输入框剪切/复制/粘贴菜单(含 main.rs 合成键盘事件的时序调整)

这是三处里最复杂的一处:菜单项本身不直接执行剪切/复制,而是合成一个
`⌘/Ctrl+x/c/v/a` 键盘事件重新喂给 iced,靠聚焦中的 `text_input`/
`text_editor` 内置剪贴板逻辑执行(`main.rs:3462-3483` 的既有机制)。原生
菜单同步阻塞在 `App::update()` 内部返回时,那一帧驱动该机制的 `interface`
已经不在了,需要在 `dispatch` 循环之后另起一次。

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(新增 `pending_native_menu_edit_key` 字段 + 访问器;`Message::TextInputMenuOpen` 分支)
- Modify: `crates/dozer-app/src/main.rs`(`menu_edit_key` 改 `pub(crate)`;`window_event` 里 dispatch 循环之后新增一段)

**Interfaces:**
- Consumes: Task 3 的 `text_input_menu_items`;Task 2 的 `native_menu::show`;main.rs 既有的 `menu_edit_key`/`unique_command_event`/`run_operate`。
- Produces: `App::take_pending_native_menu_edit_key(&mut self) -> Option<(char, iced_widget::core::widget::Id)>`。

- [ ] **Step 1: `app.rs` 加字段 + 访问器**

在 `text_input_menu: Option<TextInputMenu>,` 字段(约第 2360 行)旁边追加:

```rust
    /// mac 原生菜单选中剪切/复制/粘贴/全选后,要合成的 `⌘+x/c/v/a` 字符 +
    /// 目标输入 id——`native_menu::show` 同步阻塞返回时那一帧的
    /// `UserInterface` 已经不在了,main.rs 在 `dispatch` 循环之后另起一次
    /// 补上(见 `main.rs::window_event` 对应处)。非 mac 平台恒 `None`。
    pending_native_menu_edit_key: Option<(char, iced_widget::core::widget::Id)>,
```

在字段默认值初始化处(`text_input_menu: None,` 旁,约第 2731 行)追加 `pending_native_menu_edit_key: None,`。

在 `text_input_menu_open(&self)` 方法(约第 4281 行)附近追加:

```rust
    /// main.rs 在 `dispatch` 循环之后取走一次,取到即消费——见
    /// `pending_native_menu_edit_key` 字段文档。
    pub(crate) fn take_pending_native_menu_edit_key(
        &mut self,
    ) -> Option<(char, iced_widget::core::widget::Id)> {
        self.pending_native_menu_edit_key.take()
    }
```

- [ ] **Step 2: `app.rs` 改 `Message::TextInputMenuOpen` 分支**

把:

```rust
            Message::TextInputMenuOpen(target) => {
                // 与其它右键菜单互斥——关掉别的,只留本菜单(同时避免互相顶)。
                self.files.close_context_menu();
                self.project_link_menu = None;
                let (x, y) = self.files.last_right_click();
                self.text_input_menu = Some(TextInputMenu {
                    x,
                    y,
                    target: target.clone(),
                });
                // 右键不聚焦 iced 输入框(只有左键会),菜单的复制/粘贴需要通过
                // `interface.operate` 把焦点移到目标输入,否则合成回的 ⌘+c/v
                // 事件作用不到它。记录待聚焦 id,本帧后由 `apply_pending_focus`
                // 应用(main.rs)。
                self.pending_text_input_focus = Some(target.id);
            }
```

改成:

```rust
            Message::TextInputMenuOpen(target) => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.files.last_right_click();
                    let items = text_input_menu_items(&target);
                    if let Some(msg) = native_menu::show(items, (x, y)) {
                        if let Some(ch) = crate::menu_edit_key(&msg) {
                            self.pending_native_menu_edit_key = Some((ch, target.id.clone()));
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // 与其它右键菜单互斥——关掉别的,只留本菜单(同时避免互相顶)。
                    self.files.close_context_menu();
                    self.project_link_menu = None;
                    let (x, y) = self.files.last_right_click();
                    self.text_input_menu = Some(TextInputMenu {
                        x,
                        y,
                        target: target.clone(),
                    });
                    // 右键不聚焦 iced 输入框(只有左键会),菜单的复制/粘贴需要通过
                    // `interface.operate` 把焦点移到目标输入,否则合成回的 ⌘+c/v
                    // 事件作用不到它。记录待聚焦 id,本帧后由 `apply_pending_focus`
                    // 应用(main.rs)。
                    self.pending_text_input_focus = Some(target.id);
                }
            }
```

- [ ] **Step 3: `main.rs` 把 `menu_edit_key` 改成 `pub(crate)`**

```rust
pub(crate) fn menu_edit_key(message: &Message) -> Option<char> {
```

(原本是 `fn menu_edit_key`,只加 `pub(crate)`,函数体不变。)

- [ ] **Step 4: 编译确认(还没接 main.rs 消费端,预期 `pending_native_menu_edit_key` 有"从未读取"的死代码警告是正常的,下一步补上)**

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 通过,可能有 `field is never read`(仅当 `take_pending_native_menu_edit_key` 还没被调用时)——下一步接线后消失。

- [ ] **Step 5: `main.rs` 在 dispatch 循环之后补合成键盘事件**

找到 `window_event` 里这一段(约第 3493-3498 行):

```rust
            // 借用已随上面的块结束释放；这里逐条经 `dispatch`
            // 派发（`ProjectTabPickFolder` 等在其中被拦截成 rfd 模态,
            // 其余原样转给 `app.update`）。
            for message in pending_messages {
                self.dispatch(message);
            }
```

在这段**之后**(`self.sync_previews();` 之前)插入:

```rust
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
                                iced_winit::core::widget::operation::focusable::focus::<()>(
                                    target_id,
                                );
                            run_operate(&mut interface, renderer, &mut op);
                            let synth = [unique_command_event(ch)];
                            let mut second: Vec<Message> = Vec::new();
                            let _ =
                                interface.update(&synth, *cursor, renderer, clipboard, &mut second);
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
```

- [ ] **Step 6: 编译 + 跑全量测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: 通过。

- [ ] **Step 7: `clippy`/`fmt`**

Run: `cargo clippy --all-targets && cargo fmt`

- [ ] **Step 8: 人工验证**

`cargo run -p dozer-app`:
1. 打开浏览器面板(webview 可见),右键地址栏——菜单完整可见,不被下方网页盖住;选"复制"后 ⌘V 粘贴到系统其它地方能粘出地址栏原内容,验证合成键盘事件确实生效。
2. 同一输入框测试"剪切"(内容真的从输入框消失并进了剪贴板)、"粘贴"(剪贴板内容真的插入光标处)、"全选"(输入框内容全选高亮)。
3. 打开 Project 面板名称/描述编辑框,重复上述四个动作,确认非浏览器场景下同样生效。
4. 密码框场景(如有,检查代码里 `secure: true` 的实际输入点,通常是 SSH/Database 表单的密码字段)确认剪切/复制置灰不可点,粘贴/全选仍可用。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 输入框剪切/复制/粘贴菜单接入原生 NSMenu(mac)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: 收尾——全量验证 + 已知遗留

**Files:** 无新改动,验证 + 记录。

- [ ] **Step 1: 全量检查**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy --all-targets && cargo fmt --check`
Expected: 全部通过,`fmt --check` 无 diff。

- [ ] **Step 2: 回归检查 —— Conversations agent 筛选下拉不受影响**

`cargo run -p dozer-app`:打开 Conversations 面板(webview 可见),点开底部 agent 筛选下拉——确认它仍然是原来的"隐藏 webview 让路"行为(闪一下但正确显示),没有被本次改动误伤(它不在本次原生化范围内)。

- [ ] **Step 3: 记录已知遗留**

在 `docs/superpowers/specs/2026-09-16-native-context-menu-design.md` 末尾追加一段(不改动已写的设计内容,只追加):

```markdown
## 实现阶段的偏差记录(2026-09-16)

- "要清理的旧代码"一节描述的字段/函数删除**未执行**:Rust 的 `#[cfg]`
  不能挂在 `if/else if` 链的单个分支上,物理删除这些字段需要先把
  `app.rs` 顶层浮层的 if-chain 重构成 `#[cfg]` 友好的结构(如 `match`),
  对本次目标(消除 z-order 冲突)没有必要的收益,风险却不小。实际做法是
  "mac 平台永远不再把这些 `Option` 字段设成 `Some`",等价于它们被删除的
  运行时效果,旧渲染分支/`webview_hidden_by_panel_popup` 判断在 mac 上
  天然不会触发,保留在源码里但是死代码。真正物理删除可以作为一次独立、
  低优先级的清理 plan 另开。
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-09-16-native-context-menu-design.md
git commit -m "$(cat <<'EOF'
docs(dozer-app): 记录原生菜单实现阶段对 spec 清理范围的偏差

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage:** spec 的"范围"一节三处触发点(Files/Project/输入框)分别对应 Task 4/4/5;"架构"一节的 `Item`/`show`/图标栅格化对应 Task 1/2;"测试策略"一节"纯数据组装可测"对应 Task 3;"要清理的旧代码"一节的实际执行方式在 Task 6 里作为偏差记录写回 spec,不是遗漏而是显式记录的范围调整。

**Placeholder scan:** 除 Task 2 Step 9/10 明确标注"`objc2 0.6`/`objc2-app-kit 0.3.2` 具体方法名以编译器报错为准逐字段修正"外(这是第一次在此仓库使用 `define_class!` 新建 AppKit 子类,原因已在 Architecture 一节说明),其余步骤均为可直接编译的完整代码,无 TBD。

**Type consistency:** `native_menu::Item<Msg>`/`native_menu::show<Msg: Clone>` 在 Task 1/2 定义、Task 3/4/5 调用处签名一致;`pending_native_menu_edit_key: Option<(char, iced_widget::core::widget::Id)>` 在 Task 5 Step 1 定义、Step 2/5 使用处类型一致;三个 `xxx_menu_items` 函数名/签名在 Task 3 定义、Task 4/5 调用处一致。
