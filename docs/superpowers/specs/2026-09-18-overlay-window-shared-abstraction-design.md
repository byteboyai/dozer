# 弹窗独立窗口机制通用化(一):共享机制 + file_history 迁移设计

## 背景与动机

`search_modal` 已经迁到独立原生窗口(`crates/dozer-app/src/platform/search_overlay.rs`,
分支 `feature/search-modal-overlay-window` 已合并 `main`,commit `d08cbc4`)。
当时的设计文档
(`docs/superpowers/specs/2026-09-17-search-modal-overlay-window-design.md`)
明确写了"不建通用抽象,只为 `search_modal` 量身定做,第二个真实消费者出现
时再回头提炼"——本设计就是这个"回头提炼",目标是把 `search_overlay.rs`
里验证过、也踩过坑的机制通用化,应用到仍在用"`set_visible(false)` 强制
隐藏 webview"这套旧机制的其余弹窗上。

### 现状盘点复核

`search_modal` 设计文档"现状盘点"一节当时列过 8 类共用 `app_modal_open`/
`tab_overflow_open`/`panel_popup_open` 三个隐藏 webview 开关的场景。逐一
复核当前状态:

1. `ws.search.is_open()` —— 已迁独立窗口(`search_overlay.rs`)。
2. `self.text_input_menu` —— 已迁 NSMenu(`docs/superpowers/specs/
   2026-09-16-native-context-menu-design.md`)。
3. `self.file_history` —— **未迁,本设计的迁移目标**。
4. `ws.preview_tab_overflow_anchor` —— 部分迁移:`update.rs:723-758` 的
   `PreviewTabOverflowToggle` 处理在 macOS 上已经走
   `crate::chrome::native_menu::show(...)`,只有
   `#[cfg(not(target_os = "macos"))]` 分支才会真的把这个字段置
   `Some(...)`、走 iced 绘制 + 隐藏 webview 这套路径。**mac 上这条隐藏
   webview 的路径实际已经不可达**。
5. `ws.project_preview_tab_overflow_anchor` —— 同上(`update.rs` 约
   892 行起的 `ProjectPreviewTabOverflowToggle`),mac 上同样已经不可达。
6. `self.files.context_menu_is_some()` —— 已迁 NSMenu。
7. `self.project_link_menu` —— 已迁 NSMenu。
8. `ws.conversations.agent_picker_open()` —— 同 4/5:`update.rs:144-165`
   的 `AgentPickerOpen` 处理在 macOS 上已经走 `native_menu::show(...)`,
   只有 `#[cfg(not(target_os = "macos"))]` 分支才会真的置位、触发
   `webview_hidden_by_panel_popup`(`app/layout.rs:393-406`)那条隐藏
   webview 的路径。mac 上同样不可达。

即:8 类里,**只有 `file_history` 是当前真正活跃、在 mac 上仍会触发隐藏
webview 闪烁的场景**;4/5/8 三类的隐藏 webview 路径在 mac 上已经是死代码
(只服务尚未发布的非 mac 平台),NSMenu 已经把它们解决得很好——而且
wry 自己的原生右键菜单本来就是 NSMenu,Dozer 这几类弹窗改用 NSMenu 后
风格反而跟 wry 统一了,不应该动。

### 本文档的范围与后续拆分

这次的工作拆成两份 spec:

- **本文档(第一份)**:抽出 `search_overlay.rs` 里可复用的机制,把
  `file_history` 迁过去——这是唯一有当前实际收益的迁移,也是验证"抽出来
  的共享机制是否真的好用"的地方。
- **第二份(后续,不在本文档范围)**:把 4/5/8 三类的
  `#[cfg(not(target_os = "macos"))]` 回退路径换成同一套机制,顺手清理
  `app_modal_open`/`tab_overflow_open`/`panel_popup_open` 里对应的隐藏
  webview 判断。mac 上的 NSMenu 分支完全不动。这份等本文档的抽象经
  `file_history` 验证过之后再写,现在就把非 mac 细节钉死没有实际验证
  基础。

## 目标 / 非目标

**目标:**

1. 把 `search_overlay.rs` 里通用、非 `search` 专属的机制(wgpu 窗口/
   surface/renderer 建立、焦点转移判定、建子窗口的样板代码)抽成可复用
   的小型组件,消费方各自组合使用(不是一个包办一切的泛型巨结构,见
   "架构"一节的取舍说明)。
2. 抽取时**顺带修掉** `search_overlay.rs` 上线后经两轮代码审阅揪出的
   五个问题(见下方"抽取时一并修正的教训"),让后续消费方(`file_history`
   以及第二份 spec 的三类)不需要重新踩同样的坑。
3. `file_history` 弹窗改用独立原生窗口渲染,不再需要
   `set_visible(false)` 强制隐藏 preview/browser webview。
4. 这次通用化催生的"多个弹窗类型互斥"需求(见下)一并落地,顺带堵上
   现状代码里"旧弹窗状态没真正清空、被高优先级弹窗遮住之后又冒出来"
   这个已确认的漂移点(见"互斥机制"一节)。

**非目标:**

- 不在本文档范围内迁移 tab-overflow 下拉/agent picker(见上文"范围与
  后续拆分")。
- 不新增依赖。
- 不改变 `file_history` 的业务逻辑(`State`/`Message`/`update`/git2 查询/
  diff 渲染)——只改渲染宿主和触发/收起的桥接代码,同 `search_modal`
  迁移时的既有原则。
- 不做"一个泛型 `OverlayWindow<Msg>` 包办四类弹窗"的设计(brainstorming
  过程中与用户讨论过、明确否决,见"架构"一节)。

## 抽取时一并修正的教训

`search_overlay.rs` 上线后两轮代码审阅揪出的五个问题,均已在
`feature/search-modal-overlay-window` 分支修复(commit `dfc11ae`/
`02263e5`),抽取共享机制时要把修复后的版本作为基线,不是原始版本:

1. winit 在窗口刚创建时会无条件排一个合成的 `Focused(false)`,必须用
   "先真聚焦过、再失焦"这个状态机判定,不能直接拿它当失焦关闭信号
   (`focus_transition`/`handle_focus`,`search_overlay.rs:71-82`)。
2. 异步消息落地(状态变了但没有开/关窗口)不会自动触发重绘,必须在
   "这次分发的消息可能改了状态"之后无条件补一次 `request_redraw()`,
   不用精确判断脏没脏(照搬主窗口 `dispatch()` 的尺度)。
3. 需要 IME 的消费方(有文本输入的弹窗)必须显式 `set_ime_allowed(true)`
   ——不会从主窗口继承。
4. 独立窗口自己的 `dispatch` 路径如果会触发预览 webview 相关的副作用
   (比如打开新预览),必须补跑 `sync_previews()`/`apply_pending_focus()`
   ——不能假设主窗口哪次事件会顺便跑到。`file_history` 目前没有这类副
   作用(点击提交只是切换右侧 diff,不开新预览/tab),但共享机制要把这个
   调用点留出来,供后续消费方按需接入。
5. 依赖 `chrome::native_menu` 原生右键菜单的消费方,要在打开时把
   `install_content_view` 的挂靠目标指到自己的 NSView、关闭(`Drop`)时
   指回主窗口——`file_history` 没有文本输入、不需要这条,但共享机制的
   设计要让"要不要接入原生菜单"是每个消费方自己决定的可选项,不是
   全体消费方都要素做的必答题。

## 架构

### 1. 共享机制:抽小块,不抽大一统泛型

与用户在 brainstorming 中讨论并确认的取舍:不做一个参数化闭包/泛型的
`OverlayWindow<Msg>` 包办四类弹窗(会引入 `Box<dyn Fn>` 与一堆标志位
配置,覆盖 `file_history` 这种双栏异步内容和简单点击列表这种本质不同
的场景反而别扭)。改为抽出真正共享、与"具体画什么"无关的底层机制,
每个消费方(`SearchOverlay`、新的 `FileHistoryOverlay`,以及后续第二份
spec 的两类)各自持有一个小的宿主结构体,内部组合这些共享件——延续
`chrome::tab_widget::tab_overflow_menu` 现在"共享纯视图函数、状态/消息
各管各的"这个仓库里已有的风格,而不是引入一套新的泛型抽象范式。

新增 `crates/dozer-app/src/platform/overlay_gpu.rs`:

```rust
/// 独立原生窗口的 wgpu 渲染管线——不持有独立的 Device/Queue/Adapter/
/// Instance(全部从主窗口 `Ready` 借来的共享句柄),只有 Surface/
/// Renderer/Cache/Viewport/Clipboard 是这扇窗口自己的一份(`search_overlay
/// .rs` 已验证 `Renderer` 不可跨窗口共享)。
pub(crate) struct OverlayGpu {
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    renderer: iced_wgpu::Renderer,
    cache: iced_winit::runtime::user_interface::Cache,
    viewport: iced_wgpu::graphics::Viewport,
    clipboard: iced_winit::Clipboard,
}

impl OverlayGpu {
    /// 从共享的 `Instance`/`Adapter`/`Device`/`Queue` 为 `window` 建一份
    /// 独立渲染管线。`size` 是这扇窗口的物理尺寸。
    pub(crate) fn open(
        window: &std::sync::Arc<winit::window::Window>,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) -> OverlayGpu { /* 搬 search_overlay.rs::open 里 surface/format/
                          engine/renderer/viewport/clipboard 那段,原样迁移 */ }

    /// resize 时重配置 surface + viewport(不重建 Engine/Renderer)。
    pub(crate) fn reconfigure(
        &mut self,
        device: &wgpu::Device,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) { /* 搬 SearchOverlay::reposition 里 surface.configure 那段 */ }
}
```

新增 `crates/dozer-app/src/platform/overlay_focus.rs`:

```rust
/// winit 在窗口刚创建时会无条件排一个合成 `Focused(false)`,必须先收到
/// 过真 `Focused(true)`、再收到 `Focused(false)` 才算真正失焦。
#[derive(Default)]
pub(crate) struct FocusTracker {
    focused: bool,
}

impl FocusTracker {
    /// 记录一次焦点事件,返回"是否该因失焦而关闭"。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        /* 原样迁移 search_overlay.rs::focus_transition 的判定逻辑 */
    }
}
```

新增 `crates/dozer-app/src/platform/overlay_window.rs`:

```rust
/// 挂成主窗口子窗口 + `AlwaysOnTop` 的无装饰透明窗口——四类消费方共用
/// 的建窗样板(`search_overlay.rs::open` 里 `Window::default_attributes()`
/// 到 `create_window` 那段)。`pos`/`size` 由各消费方自己的几何计算给出
/// (居中/锚定按钮/贴面板底部,策略不同,不抽象成共享代码,见
/// "各消费方的定位策略互不相同"一节)。
pub(crate) fn open_child_window(
    main_window: &winit::window::Window,
    pos: winit::dpi::PhysicalPosition<i32>,
    size: winit::dpi::PhysicalSize<u32>,
    title: &str,
    el: &winit::event_loop::ActiveEventLoop,
) -> std::sync::Arc<winit::window::Window> { /* 原样迁移 */ }
```

`redraw`/`handle_input` **不抽象**,每个消费方的宿主结构体自己写(10-20
行级别,build 各自的 `Element`、跑各自的 `interface.update`)——这是
Approach B 与"一个大一统类型"的关键区别,也是共享机制该止步的地方。

### 2. 互斥机制:开一个自动关掉其他已开的

brainstorming 中确认:这四类弹窗(`search`/`file_history`,以及第二份
spec 的 tab-overflow/agent picker)在新机制下要彼此强制互斥。原因不只是
"看起来更克制"——现状代码里已经存在真实的状态漂移问题(见
`search_modal` 设计文档撰写前的调研:`ContextMenuOpen`/`FileHistoryOpen`
等触发点只清掉互斥的旧弹窗状态,不清与自己不同类的其它弹窗状态,靠
`App::view()` 单条 `if/else if` 链的渲染优先级掩盖,右键这类不经这条链
拦截的操作能绕过遮罩,让底下弹窗的状态保持 `Some` 却被临时盖住,回头
优先级更高的弹窗一关,旧弹窗又冒出来)。新机制天然没有"渲染优先级"这个
概念(每个都是独立窗口),必须显式做互斥,顺带堵上这个漂移点。

`Runner::Ready` 每类弹窗一个 `Option<_>` 字段(本期只加
`file_history_overlay: Option<FileHistoryOverlay>`,`search_overlay` 已有,
tab-overflow/agent picker 的字段留给第二份 spec)。新增一个小助手,任一
类别即将从 `None → Some` 时,先把其余几个置 `None`:

```rust
/// 打开任意一类独立窗口弹窗前,先关掉其余已开的——四类弹窗互斥,见
/// spec"互斥机制"一节。
fn close_other_overlays(&mut self, keep: OverlayKind) {
    let Self::Ready { search_overlay, file_history_overlay, .. } = self else { return };
    if keep != OverlayKind::Search { *search_overlay = None; }
    if keep != OverlayKind::FileHistory { *file_history_overlay = None; }
    // 第二份 spec 加 TabOverflow/AgentPicker 变体时在这里补对应分支。
}
```

`OverlayKind` 是本期新增的小枚举(`Search`/`FileHistory`,第二份 spec
再加两个变体)。`sync_search_overlay`/新增的 `sync_file_history_overlay`
在各自要 `Open` 时先调 `close_other_overlays(...)`。

### 3. `file_history` overlay 具体设计

**视图拆分**(同 `search_modal` 拆 `search_card` 的手法):现状
`popup_view(state, window_width, window_height)`(`file_history.rs:376-427`)
把"标题+双栏内容"和"套 `Length::Fixed(window_width*0.75/window_height*0.8)`
尺寸 + 全窗居中容器"糅在一个函数里。拆成:

- `file_history_card(state: &State) -> Element<...>`——标题 + 双栏内容
  (`file_history.rs:381-413` 那部分),外层容器改 `Length::Fill`(填满
  调用方给的画布,不是 `Length::Fixed` 算好的像素值——道理同
  `search_card` 当初的调整:独立窗口本身已经是量好的画布,不需要
  "在更大画布里收缩/居中适配"这层语义)。
- `popup_view(...)` 保留(旧路径仍在用,直到本设计上线那一刻整体替换,
  过渡期同 `search_modal`/`search_card` 当初的两步走)。

**几何**:居中于主窗口(同 `search`),但尺寸是
`(window_width * 0.75, window_height * 0.8)`——随主窗口宽高变化,不是
`search` 那种固定高度。复用 `search_overlay.rs::centered_overlay_bounds`
的算法本体(输入换成这个尺寸表达式),但这次的"卡片逻辑尺寸"计算函数
要接收 `window_width`**和** `window_height` 两个参数(`search` 当初只需要
`window_width`)。

**宿主结构体** `FileHistoryOverlay`(`platform/file_history_overlay.rs`):
持有 `window: Arc<Window>`、`gpu: OverlayGpu`、`focus: FocusTracker`,
`open`/`reposition`/`redraw`/`handle_input` 四个方法,内部调用 1 节的
共享件 + 自己的 `file_history_card` 视图函数。不需要 IME、不需要原生
菜单挂靠(无文本输入)。`handle_input` 里 Esc 走 `Message::FileHistory
(file_history::Message::Close)`,失焦走 `FocusTracker::handle_focus` 判定
后同样关闭——两条路径与 `search_overlay.rs` 完全一致的做法。

### 4. 改动清单

- `app/view.rs:560-573`:删除 `file_history` 分支从主窗口 `stack!` 的
  组合。
- `extensions/file_history.rs`:拆出 `file_history_card`;旧 `popup_view`
  连同这次要删的调用点一并删除(同 `search_modal()` 当初的两步走,不
  过渡期保留)。
- `app.rs:2721`/`app.rs:2792`(`preview_desired`/`browser_desired` 的
  `app_modal_open`):去掉 `self.file_history.is_some()`。
- `platform/window_events.rs`:`Ready` 新增 `file_history_overlay` 字段;
  新增 `sync_file_history_overlay()`(镜像 `sync_search_overlay`);
  `window_event` 顶部再加一个按 `WindowId` 分流到 `FileHistoryOverlay`
  的早退分支(不合并进 `search` 那个分支——两类弹窗各自独立判断
  `window_id`,顺序无所谓,见"错误处理"一节的补充说明);主窗口
  `Resized`/`CloseRequested` 处理各自追加对 `file_history_overlay` 的
  reposition/释放,同 `search_overlay` 现有写法。
- 新建 `platform/overlay_gpu.rs`/`overlay_focus.rs`/`overlay_window.rs`
  (1 节)、`platform/file_history_overlay.rs`(3 节)。
- `platform/search_overlay.rs`:改造为消费 1 节的共享件而不是自己内联
  等价代码(`OverlayGpu`/`FocusTracker`/`open_child_window` 替换掉
  `SearchOverlay` 内部对应的重复实现),`SearchOverlay` 自身的公开接口
  (`open`/`redraw`/`handle_input`/`reposition`/`window_id`/
  `apply_pending_native_menu_edit_key`)不变,调用方
  (`window_events.rs` 里已有的 `sync_search_overlay`/`window_event`
  分支)不用跟着改。

## 错误处理

同 `search_overlay.rs` 现状:创建失败(`.expect(...)`)、资源释放靠
`Drop`,不为 `file_history_overlay` 单独发明一套优雅降级——两个消费方
共用同一个 `OverlayGpu::open` 内部的失败处理,行为天然一致。

`window_event` 里两类弹窗(`search`/`file_history`)各自一个独立的按
`WindowId` 分流早退分支,顺序谁先谁后无所谓——**至多一个 `Option` 是
`Some`**(互斥机制保证),所以运行时不会有两个分支同时命中同一个
`window_id` 的歧义;写成两个独立分支而不是一个统一分支里 `match` 枚举,
是因为 2 节的 `OverlayKind` 只用于"打开时关别的"这一个场景,`redraw`/
`handle_input` 本身仍是各消费方自己的方法,没有共同的 trait/枚举分发
必要性(呼应 1 节"不抽大一统类型"的取舍)。

## 测试策略

- `centered_overlay_bounds` 的双参数(width+height)版本、`FocusTracker`
  的状态机判定、`close_other_overlays`/`OverlayKind` 的互斥逻辑,均为
  纯函数/纯状态机,按 `search_overlay.rs` 现有测试的写法补单测。
- `extensions::file_history::{State, Message, update}` 不受影响,维持
  现有覆盖。
- 窗口生命周期(开/关/拖动跟随/resize 跟随/失焦关闭/连续开关无泄漏/
  与 `search` 互斥)没有自动化测试手段,`cargo run` 人工验证清单,覆盖
  `search_overlay.rs` 原有清单 + 新增的"开 file_history 时 search 若
  开着会先被关掉"这一项。

## 排期备注

本文档验证通过、`file_history` 迁移落地后,再写第二份 spec 覆盖
tab-overflow(Files/Project)与 agent picker 的非 mac 回退路径迁移——
到时 `OverlayKind`/`close_other_overlays` 各加一个变体,复用本文档
1 节的共享件,mac 上的 NSMenu 分支全程不动。
