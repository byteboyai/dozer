# 右键菜单/输入框菜单原生化(NSMenu)设计

## 背景与动机

`crates/dozer-app` 用 wry 的子 webview 承载文件/项目/会话预览与浏览器面板。wry 在 macOS 上把 webview 实现成 `ns_view.addSubview(&webview)` 加进去的原生子视图(`wry-0.55.1` 源码 `wkwebview/mod.rs:660-705`),而这个 `ns_view` 正是 iced/wgpu 渲染的那块 view——子视图天然合成在父视图自身内容之上,这是 AppKit 图层树的固有关系,不是排序配置能调的。后果是:所有由 iced 绘制的右键菜单/下拉弹层,只要屏幕位置和某个可见 webview 的内容区重叠,就会被 webview 盖住看不见。

分支 `fix/webview-popup-occlusion-hide`(已合并/待合并)用"检测到浮层打开就强制把 `WebviewSpec.visible` 置 `false`"的办法缓解了这个问题,覆盖 Files 右键菜单、Project 链接右键菜单、Conversations agent 筛选下拉、输入框剪切/复制/粘贴菜单(`text_input_menu`)四类场景。这个办法成本低、复用了仓库里已有的隐藏口径,但代价是**每次弹出菜单,对应侧的 webview 会整体闪烁消失再恢复**,即便菜单实际位置根本没有和 webview 重叠(比如在文件树顶部右键,菜单离下方的预览区还很远)。

用户实测后认为这个体验不可接受,要求对"真正的右键菜单"改用**原生 `NSMenu`**——原生菜单的弹出追踪运行在系统窗口服务器层级,天然盖在任何应用内容(包括 webview)之上,不需要触碰 `visible` 开关,也就没有闪烁问题。

## 范围

**改的(3 处,均为真正由右键触发的菜单):**
1. Files 面板文件树右键菜单(`files::AppState.context_menu`,`files.rs::context_menu_popup`)
2. Project 面板链接行右键菜单(`App.project_link_menu`,`app.rs::project_link_context_menu_popup`)
3. 通用输入框右键菜单(`App.text_input_menu`,`app.rs::text_input_menu_popup`)——覆盖浏览器地址栏、Project 名称/描述编辑框、Files 搜索框等所有接了 `byteui::interaction::context_menu::wrap` 的输入控件

**不改的:**
- Conversations 面板的 agent 筛选下拉(`ws.conversations.agent_picker_open`)——这是点击展开的下拉选择器,不是右键菜单,继续用 `fix/webview-popup-occlusion-hide` 分支里的隐藏 webview 方案。
- 其余 7 处复用 `crate::menu.rs` 的弹层(ssh/sftp 文件树右键、database 数据源右键、git_log 分支下拉、todo 分类树右键/派发/日历/状态选择器、tab 溢出下拉、顶栏新增项目菜单)——这些面板不与任何 webview 同屏,原本就没有被遮挡的问题,继续用 `crate::menu.rs` 不变。混用两套菜单系统在这个仓库里没问题,因为区分标准很明确:**是否可能和 webview 同屏**。

## 架构

新增平台专属模块 `crates/dozer-app/src/native_menu.rs`,整体 `#[cfg(target_os = "macos")]`(其它平台不编译此模块,调用方在非 mac 平台走某种占位/降级,见"非 mac 平台"一节)。对外接口与 UI 框架无关:

```rust
pub enum Item<Msg> {
    Entry {
        icon: Option<byteui::interaction::icons::IconKind>,
        label: String,
        color: iced_widget::core::Color,
        enabled: bool,
        msg: Msg,
    },
    Separator,
}

/// 在给定的窗口内容视图坐标处同步弹出原生菜单,阻塞直到用户选中一项或
/// 取消。`Msg` 只需 `Clone`,不需要能在多线程间传递——整个调用发生在
/// macOS 主线程、winit 事件循环内部,和 `main.rs::pick_file_or_dir` 用
/// `NSOpenPanel::runModal()` 阻塞拿结果是同一手法。
pub fn show<Msg: Clone>(
    ns_view: &objc2_app_kit::NSView,
    items: Vec<Item<Msg>>,
    view_pos: (f32, f32),
) -> Option<Msg>;
```

### 调用方式(与现有 `pick_file_or_dir` 同一模式)

现有三处触发点(`files::Message::ContextMenuOpen`、`Message::LinkContextMenu`、`Message::TextInputMenuOpen`)目前的处理是"把 `Option<XxxMenu>` 状态写进 `App`,交给下一帧的 `view()` 渲染对应的 `xxx_menu_popup()`"。改造后,这三处处理逻辑改为:

1. 按现有逻辑组装 `Vec<native_menu::Item<Message>>`(内容照抄各自现有 `xxx_menu_popup()` 里的 `crate::menu::item`/`item_locked` 调用序列,只是从"构造 iced `Element`"改成"构造 `native_menu::Item` 数据");
2. 调 `native_menu::show(ns_view, items, view_pos)`——这一步**同步阻塞**,用户在原生菜单里选中一项或点击外部取消后才返回;
3. 若返回 `Some(msg)`,立即 `self.update(msg)` 递归分派下去(`App::update` 签名是 `fn update(&mut self, message: Message)`,本仓库没有走 iced 标准 `Task<Message>` 那套执行器,直接递归调用即可,不需要 `Task::done`/`proxy.send_event` 这类跨线程/跨帧机制)。

`ns_view` 从哪来:main.rs 已经在多处(交通灯按钮改写、拖拽覆写)用 `AppHandle`/`WindowHandle` 拿到过窗口内容 view 的裸指针(如 `ah.ns_view`),`native_menu::show` 的调用点复用同一份句柄。

`view_pos`:直接用各触发点现有的 `last_right_click()`/`last_cursor` 逻辑坐标(和现在 `crate::menu.rs` 弹层用的 `menu.x`/`menu.y` 是同一个数,不用重新算)。`NSMenu.popUpMenu(positioning:at:in:)` 的 `at:` 参数就是"目标 view 自己坐标系里的一点",不需要转换成屏幕坐标——如果实现阶段发现 Y 轴方向对不上(AppKit 默认 view 是"左下角原点、Y 向上",iced/wry 侧一直按"左上角原点、Y 向下"在用,这块内容 view 大概率已经被设成 flipped 以兼容 wry,但没有直接证据),用一次人工右键测试确认、需要的话翻一次 Y 即可,不是架构问题。

### 主题还原(自定义 NSMenuItem view)

每个 `Item::Entry` 用 `objc2` 运行时注册一个自定义 `NSView` 子类挂到对应 `NSMenuItem.setView(...)`:

- 底色/圆角:`enabled` 为真时默认透明,`NSTrackingArea` + 覆写 `mouseEntered:`/`mouseExited:` 在鼠标悬停时切到 `TAB_HOVER` 底色 + 圆角(视觉对齐现有 `menu::item`/`item_row` 的 hover 态,`MENU_HOVER_RADIUS = 6.0`)。
- 文字:`NSTextField`(non-editable, no border)或直接 `drawRect:` 里用 `NSAttributedString` 画,颜色取调用方传入的 `color`(正常项 CREAM/GOLD,锁定项 DIM,和 `crate::menu::item`/`item_locked` 的取色逻辑一致)。
- 图标:见下一节。
- `enabled = false` 的项:不挂 `NSTrackingArea` 高亮、`mouseUp:` 不触发任何动作,颜色用调用方传入的 DIM——对应现有 `menu::item_locked`(如密码框场景剪切/复制被禁用、文件树粘贴槽为空时置灰)。
- 点击派发:自定义 view 覆写 `mouseUp:`,命中时手动调 `[menuItem.menu cancelTrackingWithoutAnimation]` 结束追踪并记录"选中了哪一项"(自定义 view 持有一个指回 `show()` 调用帧里某个共享槽位的裸指针/索引,`popUpMenu` 返回后从这个槽位读出结果)——自定义 `view` 的 `NSMenuItem` 默认不会自动走标准 target-action 机制,这是已知的 AppKit 行为,`mouseUp:` 手动处理是常规解法,不是本设计新发明的技巧。
- 分隔线(`Item::Separator`):直接用 `NSMenuItem::separatorItem()`,不需要自定义 view。

### 图标栅格化

现有图标(`byteui::interaction::icons::IconKind`)是编译期内嵌的 Lucide SVG 字节,iced 侧用 `iced_widget::svg` + 运行时着色渲染。`Cargo.lock` 确认 `resvg 0.45.1`/`usvg 0.45.1`/`tiny-skia 0.11.4` 已经在依赖树里(`iced_wgpu` 的 svg feature 间接拉的)——**不需要引入新依赖**,`dozer-app` 直接把这三个 crate 加成同版本的直接依赖,在 `native_menu.rs` 里对同一份 `IconKind::bytes()` SVG 字节调用 `usvg` 解析 + `resvg` 渲染到指定颜色/尺寸的 `tiny_skia::Pixmap`,取出 RGBA 缓冲区包成 `NSBitmapImageRep` → `NSImage`,挂给自定义 view 里的一个 `NSImageView` 子视图。

按 `(IconKind, 颜色的 ARGB u32, 像素尺寸)` 做一个 `HashMap` 缓存(存在一个 `thread_local!`,因为整个调用链本就限定在主线程),避免每次弹菜单都重新栅格化——菜单项数量不多(Files 右键菜单最多约 10 项),但用户可能频繁右键,值得缓存。

### 非 mac 平台

`native_menu.rs` 整体 `#[cfg(target_os = "macos")]`。仓库当前是"mac 先发但架构留门"(`CLAUDE.md`),三处调用点在非 mac 平台编译时退回**本次改造前的行为**(即 `fix/webview-popup-occlusion-hide` 分支那套"写状态 + 下一帧渲染 iced 弹层 + 隐藏 webview"逻辑,用 `#[cfg(not(target_os = "macos"))]` 保留)——这样不会在未来做 Windows/Linux 移植前引入一个"其它平台完全没有右键菜单"的空白区。

## 要清理的旧代码(mac 平台路径)

- `files::AppState.context_menu` 字段、`ContextMenu` 结构体、`files.rs::context_menu_popup` 渲染函数——改成直接在 `ContextMenuOpen` 处理里组装 `Vec<native_menu::Item<_>>` 并同步调用 `native_menu::show`,不再需要"状态 + 下一帧渲染"这层间接。
- `App.project_link_menu` 字段、`ProjectLinkMenu` 结构体、`app.rs::project_link_context_menu_popup`。
- `App.text_input_menu` 字段、`TextInputMenu` 结构体、`app.rs::text_input_menu_popup`。
- `fix/webview-popup-occlusion-hide` 分支里给这三者加的隐藏逻辑:`app.rs::webview_hidden_by_panel_popup` 里 `PanelKind::Files`/`PanelKind::Project` 两个分支(连带对应的两个单测),以及 `preview_desired`/`browser_desired` 里并入 `app_modal_open` 的 `self.text_input_menu.is_some()` 判断——这些 mac 平台不再需要,因为原生菜单不会被 webview 盖住;`webview_hidden_by_panel_popup` 保留 `PanelKind::Conversations` 分支(agent 筛选下拉仍需要)。非 mac 平台的 `#[cfg(not(target_os = "macos"))]` 分支里,这套隐藏逻辑原样保留(因为非 mac 还是走旧的 iced 弹层)。
- main.rs/其它模块里如果还有直接引用上述被删字段/函数的地方(如 Esc 键路由 `App::context_menu_open`/`project_link_context_menu_open`/`text_input_menu_open` 等既有 accessor),需要同步改成查询"原生菜单当前是否正在追踪"——但原生菜单是同步阻塞调用,`show()` 还没返回之前,`update()`/整个事件循环都停在这一帧里,Esc 键路由这类"菜单是否打开"的旁路查询在原生菜单场景下**没有存在的必要**(用户按 Esc,`NSMenu` 自己的追踪循环会处理,不会传导到 winit 的 `KeyDown` 事件)——具体哪些 accessor 需要删、哪些调用点需要跟着改,留给实现计划逐个盘点。

## 测试策略

原生 AppKit 交互(自定义 view 绘制、hover、点击追踪、`popUpMenu` 阻塞行为)和仓库里已有的 `NSOpenPanel`/交通灯按钮改写/拖拽覆写一样,**没有自动化测试**,只能人工在真机上跑 `cargo run -p dozer-app` 验证——这是已知、可接受的仓库既有约定(参考 `[[feedback-verify-plan-completion-independently]]`/App fixture 缺口的既有记录)。

可以自动化测试的部分:
- "现有 `xxx_menu_popup()` 里的条件分支(是否目录/是否项目根/剪贴槽是否有内容/是否密码框)"改造成"组装 `Vec<native_menu::Item<Message>>`"之后,这段**纯数据组装逻辑**(不碰 AppKit)可以抽成独立函数单测,和菜单具体怎么画解耦——比如 `files_context_menu_items(is_dir, is_root, has_clipboard, target) -> Vec<native_menu::Item<files::Message>>`,断言各种组合下该出现哪些项、哪些项 `enabled=false`。三处触发点都可以照此模式各抽一个纯函数 + 单测,覆盖率不因为"UI 变成原生的"而降低。
- 图标栅格化缓存的 key 计算(`(IconKind, 颜色, 尺寸)` → 缓存键)如果有非平凡的量化/取整逻辑,值得单测;`resvg`/`tiny-skia` 的渲染结果本身不值得测(信任库的正确性)。

## 自查

- **占位符扫描**:无 TBD/待定字样。
- **内部一致性**:三处调用点(Files/Project/输入框)统一走 `native_menu::show` 同一接口;非 mac 平台明确保留旧路径,不产生"该平台完全没有右键菜单"的缺口;`webview_hidden_by_panel_popup` 明确只保留 Conversations 分支,Files/Project 分支的删除范围与"要清理的旧代码"一节对应一致。
- **范围检查**:聚焦 3 个触发点 + 1 个新模块,不牵动其余 7 处 `crate::menu.rs` 消费方,足够作为单个实现计划的输入,不需要再拆子项目。
- **歧义检查**:`view_pos` 的 Y 轴方向留了"实现阶段人工验证确认,必要时翻转"的开放项,已明确标注为非架构性风险,不是遗漏。
