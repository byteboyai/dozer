# search_modal 迁独立原生窗口设计

## 背景与动机

`crates/dozer-app` 用 wry 的子 webview 承载文件/项目/会话预览与浏览器面板。wry 在
macOS 上把 webview 实现成原生子视图（`ns_view.addSubview`），而这个 `ns_view`
正是 iced/wgpu 渲染的那块 view——子视图天然合成在父视图自身内容之上，这是
AppKit 图层树的固有关系。后果是：所有由 iced 绘制、且屏幕位置可能和某个可见
webview 重叠的浮层，都会被 webview 盖住看不见。

现状的规避办法（`App::preview_desired`/`browser_desired`，`app.rs:2718-2852`）
是"检测到浮层打开就强制把对应侧 webview 的 `visible` 置 `false`"，8 类浮层共用
这一口径（详见"现状盘点"）。这个办法代价是**浮层打开/关闭时对应侧 webview 会
整体闪烁消失再恢复**，即便浮层实际位置根本没有和 webview 重叠。

其中真正的右键菜单三类（Files 树、Project 链接行、通用输入框）已经按
`docs/superpowers/specs/2026-09-16-native-context-menu-design.md` 迁到原生
`NSMenu` 解决——原生菜单运行在系统窗口服务器层级，天然盖过 webview，且弹出/
收起没有持续存在的生命周期需要管理。但 `search_modal` 不是"弹出即消失"的菜单，
是一个持续显示、可输入、可滚动结果列表的对话框，不能套用 NSMenu 的办法。

`spike/multi-window-overlay-wry`（发现记录：
`docs/superpowers/specs/2026-09-17-multi-window-overlay-spike-findings.md`，
该分支未合并进 `main`）验证了另一条路径：另开一扇原生 `winit::Window`（挂成
主窗口子窗口 + `AlwaysOnTop`）天然盖过 wry webview，不需要 `set_visible`
配合；拖动主窗口时子窗口跟随；反复开关无资源泄漏。本设计把这条路径落到
`search_modal` 这一个具体浮层上，作为该机制在生产代码里的第一个消费者。

## 现状盘点：8 类浮层共用的 hide-webview 机制

`App::preview_desired`（`app.rs:2718-2793`）与 `App::browser_desired`
（`app.rs:2798-2852`）把以下开关合并进 `app_modal_open`/`tab_overflow_open`/
`panel_popup_open` 三个布尔门，任一为真就把对应侧 webview 的
`WebviewSpec.visible` 置 `false`，经 `Runner::sync_previews()`
（`window_events.rs:912-1011`）→ `sync_webview_pool()`（`runtime.rs:168-206`）
→ `view.set_visible(...)`（`runtime.rs:205`）落到真实 wry 调用：

1. `ws.search.is_open()`——**search_modal，本设计的迁移目标**
2. `self.text_input_menu`——已迁 NSMenu（2026-09-16 设计）
3. `self.file_history`——未迁，维持现状
4. `ws.preview_tab_overflow_anchor`——未迁，维持现状
5. `ws.project_preview_tab_overflow_anchor`——未迁，维持现状
6. `self.files.context_menu_is_some()`——已迁 NSMenu
7. `self.project_link_menu`——已迁 NSMenu
8. `ws.conversations.agent_picker_open()`——未迁，维持现状（点击展开的下拉，
   不是右键菜单，2026-09-16 设计已明确排除）

本设计**只改第 1 项**。其余未迁项（3/4/5/8）继续走现有 `visible=false`
机制，不在本次范围内——`search_modal` 是这条"独立窗口"路径的第一个消费者，
不是最后一个，但下一个消费者出现前不为它们预留接口（YAGNI，见"非目标"）。

## 目标 / 非目标

**目标：**

1. `search_modal` 用独立原生 `winit::Window` 渲染，不再依赖
   `set_visible(false)` 隐藏 preview/browser webview 来避免遮挡。
2. 复用主窗口已建好的 `wgpu::Device`/`Queue`，只为这扇新窗口单独建
   `Engine`/`Renderer`（spike 已确认 `Renderer` 不可共享，`Device`/`Queue`
   可以）。
3. 视觉与交互对用户可感知的部分保持等价或更符合原生习惯：输入、结果列表
   滚动、Esc 关闭、自动聚焦查询框——除了"点击外部关闭"从"点击 scrim"改为
   "窗口失焦即关闭"（已与用户确认的设计取舍，见下）。

**非目标：**

- 不建通用的"overlay window host"抽象——本设计的实现只为 `search_modal`
  量身定做；其余 7 类浮层（现状盘点里的 2-8 项，减去已迁 NSMenu 的
  2/6/7）继续用现有机制，不在本设计范围内。第二个真实消费者出现时再回头
  从两个具体实现里提炼抽象（Rule of Three），不提前设计接口形状。
- 不改变 `extensions::search::{WorkspaceState, Message, update,
  query_field_id}` 的业务逻辑——只改渲染宿主和触发/收起的桥接代码。
- 不改变打开 search 的触发方式（快捷键/点击入口不变）。
- 不处理"整窗覆盖 + 窗口内自绘 scrim"这个候选方案（详见"考虑过的方案"）。

## 考虑过的方案

**方案 A（采用）：卡片大小窗口 + 失焦即关闭。** 新窗口只有对话框卡片那么大
（类似 spike 里的 overlay），不画 scrim；点击主窗口（=新窗口失去 OS
焦点）就关闭，贴近原生 popover/下拉菜单的习惯，主窗口本身不需要变暗。

**方案 B：整窗覆盖 + 窗口内自绘 scrim。** 新窗口铺满整个主窗口区域，自己在
里面画半透明 scrim + 居中卡片，视觉效果和现状 1:1 一致。代价是这扇窗口自己
要处理"点 scrim 区域关、点卡片区域不关"的命中测试，且窗口尺寸要跟主窗口
resize 同步铺满，比方案 A 多一层复杂度，视觉收益（保留变暗效果）不足以
抵消。

**结论：采用方案 A**（已与用户确认）。

## 架构

### 1. 安放位置：挂在现有 `Runner::Ready` 上，不引入新事件循环

winit 原生支持一个 `ApplicationHandler` 处理多扇窗口的事件（按 `WindowId`
分发），不需要为 overlay 开独立线程或独立事件循环。`Runner::Ready`
（`window_events.rs:42-125`）新增字段：

```rust
search_overlay: Option<SearchOverlay>,
```

新增同步步骤 `sync_search_overlay()`，调用时机与写法对齐现有
`sync_previews()`（`window_events.rs:912-1011`，负责按 `preview_desired`/
`browser_desired` 重算 webview 池的同名模式）——每次分发完一批消息后调用，
按 `ws.search.is_open()` 与 `self.search_overlay.is_some()` 是否一致来
开窗/关窗：

```rust
fn sync_search_overlay(&mut self, app: &App, el: &ActiveEventLoop) {
    match (app.workspace().search.is_open(), &self.search_overlay) {
        (true, None) => self.search_overlay = Some(SearchOverlay::open(&self.window, el)),
        (false, Some(_)) => self.search_overlay = None, // Drop 负责释放窗口/surface/renderer
        _ => {}
    }
}
```

`extensions::search::{WorkspaceState, Message, update, query_field_id}`
不变——共享同一个 `App`/`WorkspaceState`，只是多了一条"谁来 build/draw
这个 `Element`"的路径。

### 2. `SearchOverlay` 结构体与生命周期

```rust
struct SearchOverlay {
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    renderer: iced_wgpu::Renderer,
    cache: iced_winit::runtime::user_interface::Cache,
    viewport: iced_wgpu::graphics::Viewport,
}
```

不持有独立的 `Device`/`Queue`——`SearchOverlay::open` 接收主窗口 `Ready`
里已经建好的 `&wgpu::Device`/`&wgpu::Queue`/`&wgpu::Adapter`（`clone()`
共享句柄），只为这扇窗口单独 `Engine::new(...)` + `Renderer::new(...)`
（spike Task 3 已证实这条路径可行）。

**开(`open`)：** `with_parent_window` + `WindowLevel::AlwaysOnTop`，无
装饰、透明背景，尺寸取对话框卡片尺寸（复用现状 `dialog::width(...)` 的
计算），位置居中于主窗口（复用现状 `align_x(Center).align_y(Center)`
的视觉效果，换算成屏幕坐标）；创建后立即请求 OS 焦点，随后对这扇窗口自己
的 `UserInterface` 跑一次 `operation::focusable::focus`，让查询框自动
获得输入焦点（等价于现状 `query_focus_pending` 一次性标记的效果，只是
作用域从"主窗口下一帧"变成"这扇新窗口的第一次 build"）。

**跟随主窗口移动：** 免费——spike 已证实 `addChildWindow` 语义下位置
自动跟随，不需要代码。

**跟随主窗口 resize：** **不**免费——`addChildWindow` 只管位置不管尺寸/
布局联动。`sync_search_overlay()` 之外，主窗口 `WindowEvent::Resized`
处理里追加一步：若 `search_overlay` 存在，重算居中位置并
`window.set_outer_position(...)`（尺寸本身不随主窗口变，卡片是固定尺寸）。

**关(`close`)：** 三个触发点收敛到同一条路径——都是先经
`app.update(Message::Search(Message::SearchClose))` 把 `ws.search.open`
置 `false`，再由下一次 `sync_search_overlay()` 观察到状态翻转后
`self.search_overlay = None`（`Drop` 释放窗口/surface/renderer/cache）：

1. 现状已有的关闭路径（× 按钮、结果被选中后的副作用）——不变。
2. Esc——在 overlay 自己的 `KeyboardInput` 分支本地处理，不需要像现状
   `window_events.rs:500-514` 那样抢在主窗口终端转发逻辑之前特殊处理
   （overlay 窗口里没有终端要竞争）。
3. **新增：`WindowEvent::Focused(false)`**（overlay 窗口失去 OS 焦点，
   即用户点击了主窗口或切到别的 App）。

**已知风险，实现前第一步验证：** `with_parent_window` 挂载的无装饰
`AlwaysOnTop` 子窗口在 macOS 上是否真的会独立收到 `Focused(false)`
（而不是焦点事件只报给父窗口，或者压根不触发），spike 的发现记录明确
把"失焦行为"列为未验证项。这是"失焦即关闭"整个设计的地基假设，实现计划
的第一个任务应该是一个几行代码的最小验证（不是先搭完整个 overlay
再发现假设不成立）。

### 3. 渲染与输入

**视图：** 拆分现状 `search_modal()`（`search.rs:340-444`）——保留内部
`dialog` 构建那部分（`search.rs:360-424`）抽成新函数 `search_card(&ws.search,
project_root) -> Element<...>`，不再包 `crate::dialog::scrim(...)` +
`stack![...]`（`search.rs:426-443`）。旧的 `search_modal()`
整个函数连同 `app/view.rs:131-143` 里把它塞进主窗口 `stack![...]` 的那段
一起删除——不保留作为回退选项（现状盘点里已确认的取舍）。

**每帧渲染：** overlay 自己的 `RedrawRequested` 分支跑与主窗口相同的手工
`UserInterface::build → update → draw → present` 循环（主窗口这套写法
本身就是因为要手动控制 winit 事件循环以配合 wry/IME 才没用
`iced_winit::program::run`，overlay 作为同一个 `ApplicationHandler` 的
第二扇窗口，自然延续同一种手法，不引入 iced 自带的多窗口 daemon 模式），
只是 `Element` 换成 `search_card(...)`，`Cache`/`Renderer` 换成 overlay
自己的。

**输入路由：** overlay 的 `window_event` 分支把落在它 `WindowId` 上的
`KeyboardInput`/`CursorMoved`/`MouseInput`/`MouseWheel` 喂给它自己的
`UserInterface::update`，产出的 `Message`（`QueryInput`/`QuerySubmit`/
`Pick`/`SearchClose`/...）照样经 `app.update(Message::Search(...))`
派发——`extensions::search::update()` 完全不用改，因为它本来就不关心
调用方是主窗口的 `UserInterface` 还是 overlay 的。

### 4. 现有代码改动清单

- `app/view.rs:131-143`：删除 `search::search_modal(...)` 从主窗口
  `stack![...]` 的组合。
- `app.rs:2737-2738`（`preview_desired` 的 `app_modal_open`）与
  `app.rs:2809`（`browser_desired` 的 `app_modal_open`）：去掉
  `ws.search.is_open()` 这一项——这正是本设计要达成的效果：search 打开
  不再需要强制隐藏 preview/browser webview。
- `extensions/search.rs`：`search_modal()` 及其 scrim/stack 包装删除；
  新增 `search_card()`（从 `search_modal()` 内部拆出，逻辑不变）。
- `platform/window_events.rs`：`Ready` 新增 `search_overlay` 字段；新增
  `sync_search_overlay()`；`window_event` 新增 overlay 窗口 id 分支；
  `Resized` 处理追加 overlay 重新定位。
- `extensions::search::{WorkspaceState, Message, update, query_field_id}`：
  不变。

## 错误处理

- **overlay 创建失败**（adapter/device/窗口创建失败，低概率）：记录日志，
  `ws.search.open` 保持/回退为 `false`——退化成"没打开"，不维护第二套
  实现作为回退（现状 `search_modal()` 已删除）。
- **资源释放**：依赖普通 `Drop`（spike 已证实反复开关 20 次无泄漏），
  不需要额外的显式清理记账。主窗口 `WindowEvent::CloseRequested`
  处理里在 `event_loop.exit()` 前显式丢弃 `search_overlay`，图干净，
  不是正确性要求。

## 测试策略

- 纯逻辑（居中位置/resize 联动的坐标计算）按仓库惯例写单元测试。
- `extensions::search::{update, view}` 不受影响，维持现有覆盖不变。
- 窗口生命周期本身（开/关/拖动跟随/resize 跟随/失焦关闭/连续开关无泄漏）
  没有自动化测试手段，与 `native_menu.rs`、spike 本身同一惯例：`cargo run`
  人工验证，验收清单見实现计划（覆盖 spike 原有 Task 2 Step 4 的三项 +
  本设计新增的 resize 跟随、失焦关闭两项 + 连续开关内存粗测）。

## 排期备注

第一步必须是最小化验证"`Focused(false)` 在 `with_parent_window` 子窗口
上是否真的独立触发"这个地基假设（几行代码级别，不是先搭完整 overlay），
验证通过再继续搭剩余部分；若不成立，需要回来改"关闭"触发方式的设计
（比如退回方案 B，或改成主窗口点击时显式转发关闭消息），不属于本设计
文档当前范围。
