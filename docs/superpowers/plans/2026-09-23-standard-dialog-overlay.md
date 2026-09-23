# 标准弹窗组件迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把代码库里 12 个仍用主窗口 `iced::Stack`+`scrim` 渲染的模态卡片弹窗
(确认框/表单/详情页)迁到独立原生窗口——其中 5 个结构同质的 confirm 弹窗
共用一个新的通用宿主 `ConfirmOverlay`,其余 7 个各自建小宿主——最终删除
`app/view.rs` 里对应的 `stack!` 分支与被替换的旧视图函数。

**Architecture:** 两层设计。第一层是通用 `ConfirmOverlay`(`platform/
confirm_overlay.rs`),内部持有 `dialog::ConfirmDialog<Message>` 纯数据,
`redraw` 时调用既有的 `dialog::confirm()`;一个 `sync_confirm_overlay` 按
`if/else if` 优先级链在 5 个触发条件里选出"此刻该显示哪个",而不是五个
消费方各自维护一份窗口生命周期。第二层是 7 个定制小宿主,结构与现有
`settings_overlay.rs`/`file_history_overlay.rs`/`project_create_overlay.rs`
完全同构,复用同一批共享件(`OverlayGpu`/`FocusTracker`/`open_child_window`/
`centered_overlay_bounds`)。`OverlayKind`/`close_other_overlays` 从 4 个
变体扩到 12 个,让全部 12 个新宿主与既有 4 个纳入同一个"任意时刻只开一个
模态卡片弹窗"的互斥集合。

**Tech Stack:** Rust, iced 0.14(`iced_widget`/`iced_wgpu`/`iced_winit`),
winit(独立子窗口),wgpu(每扇窗口独立 `OverlayGpu` 渲染管线)。

**Spec:** `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`

**Branch:** 整份计划在一个独立分支/worktree 上顺序执行(如
`feature/standard-dialog-overlay`),不要在 `main` 上直接跑——15 个任务
反复改同一批共享文件(`platform/window_events.rs`/`app/view.rs`),
`main` 上任何并发提交都会造成合并冲突甚至互相覆盖(本仓库过往教训:多
个 agent 同时在 `main` 上开发不同计划导致审阅时短暂混淆)。全部任务完成
且经审阅后再合并回 `main`,不要中途合并半份。

## Global Constraints

- 不改变这 12 个弹窗任何一个的业务逻辑(`State`/`Message`/`update`/校验/
  异步任务)——只改渲染宿主与触发/收起的桥接代码。
- 不改动已迁移的 `search`/`file_history`/`project_create`/`settings` 四个
  消费方自身的内部实现,只扩展它们共享的 `OverlayKind` 枚举和
  `close_other_overlays`。
- 不新增 crate 依赖。
- 任意时刻只开一个模态卡片弹窗(`OverlayKind`/`close_other_overlays` 互斥,
  12 个新变体 + 既有 4 个)。
- 需要 IME 的宿主必须显式 `window.set_ime_allowed(true)`(不会从主窗口
  继承)。
- 依赖 `chrome::native_menu` 原生右键菜单的宿主,打开时把
  `install_content_view` 挂靠指到自己的 NSView、`Drop` 时指回主窗口。
- 宿主 `handle_input` 产生的消息一律经 `self.dispatch(message)` 派发,
  不能直接 `app.update(message)`——`Runner::dispatch` 里有消息级拦截
  (如 `files::Message::MoveDirBrowse` 弹原生目录选择器),跳过它会让这
  类消息在新宿主里失效。
- 内部会触发原生模态选择器(`rfd`/系统对话框)的宿主,不能用"失焦即关闭"
  ——选择器弹出会让宿主窗口收到一次真实 `Focused(false)`,必须像
  `ProjectCreateOverlay` 那样不接失焦关闭,只认 Esc/表单自身的取消按钮。
- winit 在窗口刚创建时会无条件排一个合成的 `Focused(false)`,`FocusTracker`
  的"先真聚焦过、再失焦"状态机已经处理这个,新宿主直接复用,不要自己
  重新判断。

## Review Focus

- **互斥**:打开任意一个新宿主时,若已有另一个模态卡片弹窗开着(不论新旧
  12+4 个中的哪一个),必须先被关掉——不能同时出现两扇独立窗口。
- **webview 遮挡**(本次迁移要修的真实 bug):Files/Project/浏览器面板正
  显示 webview 内容时打开这 12 个弹窗中的任意一个,必须完全可见、不被
  wry 子视图遮挡。
- **窗口跟随**:弹窗打开期间拖动/resize 主窗口,弹窗必须跟着重新定位,
  不能残留在旧位置或被裁切。
- **连续开关**:关闭后立刻用同一个/不同类型的触发条件再打开,必须正常
  重新出现——不能因为上一次关闭遗留状态(如 `close_other_overlays` 没
  真正清空某个 `Option`)导致第二次打不开或残留旧内容。
- **IME/原生右键粘贴**(Database/SSH 表单、Project 删除输入框、Todo 详情):
  中文候选词、原生右键粘贴菜单必须在独立窗口里正常工作,不能因为迁移丢失
  ——`settings_overlay.rs` 已经是这条的正确参照实现。

---

##任务总览与执行顺序

1. `ConfirmDialog<Msg>` 加 `Clone`
2. 新建 `ConfirmOverlay` 通用宿主 + `OverlayKind::Confirm` 全套接线
3. 第一个 confirm 消费方:Todo 清空列表确认(验证宿主骨架)
4. 第二个 confirm 消费方:Files 删除确认
5. 第三个 confirm 消费方:Database 删除数据源确认
6. 第四个 confirm 消费方:SSH 删除主机确认
7. 第五个 confirm 消费方:关闭 Agent tab 确认
8. 定制宿主:Database 驱动管理(无 IME)
9. 定制宿主:Files 移动确认(无 IME,但需要"不接失焦关闭")
10. 定制宿主:Project 修复进度(无 IME,阻塞态)
11. 定制宿主:Database 数据源表单(需 IME)
12. 定制宿主:SSH 主机表单(需 IME)
13. 定制宿主:Project 删除确认(需 IME)
14. 定制宿主:Todo 详情(需 IME)
15. 最终清理:删旧 `stack!` 分支残余、`dialog::scrim`/`scrim_blocking` 判定、全量验证

任务 2-7 建立并验证通用宿主;8-14 每个独立成一次提交,互不阻塞彼此(只要
都在任务 2 之后做);15 必须最后做。

---

### Task 1: `ConfirmDialog<Msg>` 派生 `Clone`

**Files:**
- Modify: `crates/dozer-app/src/dialog.rs:130`

**Interfaces:**
- Produces: `ConfirmDialog<Msg>: Clone`(当 `Msg: Clone`,`app::Message`
  已经是 `Clone`)——后续任务的 `ConfirmOverlay::redraw` 需要
  `self.spec.clone()`。

- [ ] **Step 1: 加派生**

在 `crates/dozer-app/src/dialog.rs:130` 之前加一行:

```rust
#[derive(Clone)]
pub struct ConfirmDialog<Msg> {
```

- [ ] **Step 2: 编译验证**

Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo build -p dozer-app 2>&1 | tail -30`
Expected: 无新增错误/警告(纯派生,`Msg`/`Color`/`String`/`f32`/`Option<IconKind>`
均已是 `Clone`)。

- [ ] **Step 3: 补一个最小单测**

在 `crates/dozer-app/src/dialog.rs` 的 `mod confirm_tests` 里加:

```rust
#[test]
fn confirm_dialog_is_cloneable() {
    let spec = ConfirmDialog {
        icon: None,
        title: "标题".to_string(),
        description: "说明".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: TestMsg::Cancel,
        confirm_label: "确认".to_string(),
        confirm_msg: TestMsg::Confirm,
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    };
    let cloned = spec.clone();
    assert_eq!(cloned.title, spec.title);
    assert_eq!(cloned.confirm_msg, spec.confirm_msg);
}
```

`TestMsg` 需要加 `#[derive(Clone)]`(已有 `Debug, Clone, PartialEq` 检查
`mod confirm_tests` 现有的 `#[derive(Debug, Clone, PartialEq)] enum TestMsg`
——确认已经带 `Clone`,若没有则补上)。

- [ ] **Step 4: 跑测试**

Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo test -p dozer-app dialog:: 2>&1 | tail -20`
Expected: `confirm_dialog_is_cloneable` 通过。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/dialog.rs
git commit -m "feat(dialog): derive Clone on ConfirmDialog for overlay host reuse"
```

---

### Task 2: 新建 `ConfirmOverlay` 通用宿主 + `OverlayKind::Confirm` 接线

这一任务只搭骨架、不接任何真实消费方(消费方在 Task 3 起接入)——
`sync_confirm_overlay` 此时的"desired"永远是 `None`,验证的是"编译通过 +
互斥扩展正确"。

**Files:**
- Create: `crates/dozer-app/src/platform/confirm_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`(声明新模块,若 `platform`
  是通过 `mod x;` 列表组织——参照文件里 `mod settings_overlay;` 那一行
  加同款)
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  - `OverlayKind` 枚举(`:216-221`)
  - `close_other_overlays`(`:1119-1142`)
  - `Runner::Ready` 结构体(`:182-199` 之后追加字段)
  - 新增 `sync_confirm_overlay`(参照 `sync_settings_overlay`,
    `:1345-1405`)
  - `window_event` 顶部新增按 `WindowId` 分流早退分支(参照 settings 分支,
    `:2377-2409`)
  - `Ready` 构造处(`:2219` 附近 `settings_overlay: None,`)追加
    `confirm_overlay: None,`
  - dispatch 后调用点(`:2245`/`:2373`/`:2407`/`:3514` 附近现有的
    `self.sync_settings_overlay(event_loop);` 调用点)各追加一行
    `self.sync_confirm_overlay(event_loop);`
  - `Resized`/`CloseRequested` 主窗口处理(`:2440` 附近解构 + `:3321`/
    `:3340` 附近 reposition/释放逻辑)追加 `confirm_overlay` 的对应处理

**Interfaces:**
- Consumes:`OverlayGpu`(`platform::overlay_gpu`)、`FocusTracker`
  (`platform::overlay_focus`)、`open_child_window`/`centered_overlay_bounds`
  (`platform::overlay_window`)——签名与 `settings_overlay.rs` 现有用法
  完全一致。
- Produces:
  - `pub(crate) enum ConfirmTrigger { FilesDelete, DatabaseDelete,
    SshDelete, AgentTabClose, TodoClear }`(`Clone, Copy, PartialEq, Eq`)
  - `pub(crate) struct ConfirmOverlay`,方法 `open`/`reposition`/`redraw`/
    `handle_input`/`window_id`/`request_redraw`,签名对齐
    `SettingsOverlay` 同名方法。
  - `pub(crate) fn desired_confirm(ws: Option<&Workspace>) ->
    Option<(ConfirmTrigger, dialog::ConfirmDialog<Message>)>`——Task 3-7
    逐个往这个函数的 `if/else if` 链里加一个分支,本任务先返回 `None`
    (函数体只有 `let _ = ws; None`,或者直接以 `#[allow(unused_variables)]`
    占位——**不是"以后再实现"的占位符**,是"目前没有任何已迁移的
    trigger"这一诚实状态,Task 3 起逐步替换)。

- [ ] **Step 1: 加 `platform/mod.rs` 模块声明**

在 `crates/dozer-app/src/platform/mod.rs` 里找到 `mod settings_overlay;`
这一行,紧邻加一行:

```rust
mod confirm_overlay;
```

（若 `settings_overlay` 是 `pub(crate) mod`,`confirm_overlay` 用同样的
可见性;若文件用 `pub use settings_overlay::SettingsOverlay;` 之类的
re-export,`ConfirmOverlay`/`ConfirmTrigger` 只在 `window_events.rs` 内部
用,不需要 re-export。）

- [ ] **Step 2: 写 `confirm_overlay.rs`**

```rust
//! 通用"标题+说明+取消/确认"弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`
//! 「架构」第 1 节。之所以能通用,是因为这一类弹窗的内容已经 100% 由同一个
//! 纯数据结构 `dialog::ConfirmDialog<Message>` 描述——宿主只需要存住这份
//! 数据,`redraw` 时调一次既有的 `dialog::confirm(spec.clone(), w)` 即可
//! 拿到 `Element`,不需要为"内容长什么样"引入 `Box<dyn Fn>` 或标志位。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::dialog;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// 五个 confirm 形态弹窗的判别标签——只用来在 `sync_confirm_overlay` 里
/// 判断"这次 desired 和已开的窗口是不是同一个弹窗",不需要 `Message`/
/// `ConfirmDialog` 派生 `PartialEq`(`Message` 枚举很大,不适合整体派生)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmTrigger {
    FilesDelete,
    DatabaseDelete,
    SshDelete,
    AgentTabClose,
    TodoClear,
}

/// 内容有界(1-3 行说明 + 两个按钮),固定逻辑尺寸覆盖全部 5 个用例,不需要
/// 按主窗口比例缩放(同 `settings`/`search` 的取舍,不同于 `file_history`)。
fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(420.0, 200.0)
}

pub(crate) struct ConfirmOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    trigger: ConfirmTrigger,
    spec: dialog::ConfirmDialog<Message>,
}

impl ConfirmOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn trigger(&self) -> ConfirmTrigger {
        self.trigger
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        trigger: ConfirmTrigger,
        spec: dialog::ConfirmDialog<Message>,
        el: &ActiveEventLoop,
    ) -> ConfirmOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "confirm", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        ConfirmOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            trigger,
            spec,
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    fn card(&self) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        dialog::confirm(self.spec.clone(), card_logical_size().width)
    }

    pub(crate) fn redraw(&mut self) {
        let mut interface = UserInterface::build(
            self.card(),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            mouse::Cursor::Unavailable,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            mouse::Cursor::Unavailable,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    /// Esc 与失焦统一发送 `spec.cancel_msg`——五个消费方共用同一条关闭
    /// 路径,不需要逐个判断"这是哪个弹窗、该发哪条 Cancel 消息"。
    pub(crate) fn handle_input(&mut self, event: &WindowEvent, modifiers: ModifiersState) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![self.spec.cancel_msg.clone()];
        }
        let _ = modifiers;
        Vec::new()
    }

    /// 返回 `true` 表示应该关闭——与 `search_overlay`/`file_history_overlay`
    /// 同款失焦即关闭,这五个弹窗都不涉及会弹出原生模态选择器的按钮,不需要
    /// `ProjectCreateOverlay` 那种"不接失焦关闭"的例外。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }
}
```

（`redraw`/`handle_input` 与 `SettingsOverlay` 相比少了 `app: &mut App`
参数——这是关键区别:`ConfirmOverlay` 的内容完全来自 `self.spec`,不需要
每帧重新查 `App` 状态,`cursor`/`clipboard`/`modifiers` 这些字段这五个
消费方都用不到(无文本输入、无鼠标悬停态视觉),所以比 `SettingsOverlay`
更简单,不是遗漏。）

- [ ] **Step 3: `OverlayKind` 加 `Confirm` 变体**

`crates/dozer-app/src/platform/window_events.rs:216-221`:

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    Search,
    FileHistory,
    ProjectCreate,
    Settings,
    Confirm,
}
```

- [ ] **Step 4: `close_other_overlays` 加一行**

在现有函数体(`:1119-1142`)的解构里加 `confirm_overlay,`,末尾加:

```rust
if keep != OverlayKind::Confirm {
    *confirm_overlay = None;
}
```

- [ ] **Step 5: `Runner::Ready` 加字段**

在 `settings_overlay: Option<settings_overlay::SettingsOverlay>,`
(`:199`)之后加:

```rust
/// 通用 confirm 弹窗的独立窗口宿主(五个 confirm 形态消费方共用一个)。
/// 生命周期由 `sync_confirm_overlay` 按 `confirm_overlay::desired_confirm`
/// 单向驱动开/关,与其余四类互斥。
confirm_overlay: Option<confirm_overlay::ConfirmOverlay>,
```

文件顶部 `use crate::platform::settings_overlay;` 附近加
`use crate::platform::confirm_overlay;`。

- [ ] **Step 6: 加 `sync_confirm_overlay`**

紧邻 `sync_settings_overlay`(`:1345-1405`)之后加:

```rust
/// 按 `confirm_overlay::desired_confirm` 算出的"此刻该显示哪个 confirm
/// 弹窗"(至多一个)驱动窗口开/关。与其余 `sync_*_overlay` 不同,这里比较
/// 的是 `ConfirmTrigger` 判别标签而不是内容相等——`Message` 枚举没有派生
/// `PartialEq`,也没有必要:同一个 trigger 在弹窗存活期间内容不会变
/// (如已经显示的删除确认不会因为用户操作别的东西而改文案),不需要每帧
/// 重建 spec 做深比较。
fn sync_confirm_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
    let Self::Ready { app, confirm_overlay, .. } = self else {
        return;
    };
    let desired = confirm_overlay::desired_confirm(app.active_workspace());
    let current_trigger = confirm_overlay.as_ref().map(|o| o.trigger());
    match (desired, current_trigger) {
        (Some((trigger, _)), Some(open)) if trigger == open => {
            // 同一个弹窗仍在显示,什么都不做。
        }
        (Some((trigger, spec)), _) => {
            self.close_other_overlays(OverlayKind::Confirm);
            let Self::Ready {
                window,
                instance,
                adapter,
                device,
                queue,
                confirm_overlay,
                ..
            } = self
            else {
                return;
            };
            *confirm_overlay = Some(confirm_overlay::ConfirmOverlay::open(
                window, adapter, device, queue, instance, trigger, spec, el,
            ));
        }
        (None, _) => {
            let Self::Ready { confirm_overlay, .. } = self else {
                return;
            };
            *confirm_overlay = None;
        }
    }
    let Self::Ready { confirm_overlay, .. } = self else {
        return;
    };
    if let Some(overlay) = confirm_overlay {
        overlay.request_redraw();
    }
}
```

- [ ] **Step 7: `window_event` 新增分流分支**

紧邻 settings 分支(`:2377-2409`)之前或之后加(顺序无所谓,互斥机制保证
至多一个 `Option` 是 `Some`):

```rust
if let Self::Ready {
    confirm_overlay,
    modifiers,
    ..
} = self
    && let Some(overlay) = confirm_overlay
    && window_id == overlay.window_id()
{
    if matches!(event, WindowEvent::RedrawRequested) {
        overlay.redraw();
    } else if matches!(event, WindowEvent::CloseRequested) {
        // 关闭窗口本身不代表业务状态清空——下一帧 `sync_confirm_overlay`
        // 会发现 `desired` 仍是 `Some`(业务状态没变)又把它重新建出来,
        // 所以这里必须真的发一条 cancel 消息,让触发条件本身归位。
        let cancel = overlay.trigger();
        let _ = cancel;
        if let Self::Ready { app, confirm_overlay, .. } = self
            && let Some(overlay) = confirm_overlay
        {
            let msg = overlay.handle_input(
                &WindowEvent::KeyboardInput {
                    device_id: unsafe { winit::event::DeviceId::dummy() },
                    event: winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(
                            winit::keyboard::KeyCode::Escape,
                        ),
                        logical_key: winit::keyboard::Key::Named(
                            winit::keyboard::NamedKey::Escape,
                        ),
                        text: None,
                        location: winit::keyboard::KeyLocation::Standard,
                        state: winit::event::ElementState::Pressed,
                        repeat: false,
                        ..unsafe { std::mem::zeroed() }
                    },
                    is_synthetic: false,
                },
                ModifiersState::default(),
            );
            let _ = app;
            for m in msg {
                self.dispatch(m);
            }
        }
    } else if let WindowEvent::Focused(focused) = event {
        let should_close = overlay.handle_focus(focused);
        if should_close {
            let msg = overlay.spec_cancel();
            self.dispatch(msg);
        }
    } else {
        let msg = overlay.handle_input(&event, *modifiers);
        for m in msg {
            self.dispatch(m);
        }
    }
    self.sync_confirm_overlay(event_loop);
    return;
}
```

上面 `CloseRequested` 分支手搓一个合成 `Escape` 事件明显不干净——**改成
更直接的写法**:给 `ConfirmOverlay` 加一个小方法暴露 `cancel_msg`,`Focused`
分支同理复用它:

```rust
// confirm_overlay.rs 内 ConfirmOverlay impl 追加:
pub(crate) fn cancel_message(&self) -> Message {
    self.spec.cancel_msg.clone()
}
```

把上面 `window_event` 分支里 `CloseRequested`/`Focused` 两处改成:

```rust
} else if matches!(event, WindowEvent::CloseRequested) {
    self.dispatch(overlay.cancel_message());
} else if let WindowEvent::Focused(focused) = event {
    if overlay.handle_focus(focused) {
        self.dispatch(overlay.cancel_message());
    }
}
```

（这一步是对 Step 2 `ConfirmOverlay` 的小补充:加 `cancel_message`
方法,删掉本 Step 最初那段手搓合成事件的写法——按最终这版实现,不要
按中间那版。）

- [ ] **Step 8: `Ready` 构造 + dispatch 后调用点 + resize/close 处理**

- 构造处(`:2219` 附近)加:`confirm_overlay: None,`
- 四处现有 `self.sync_settings_overlay(event_loop);` 调用点
  (`:2245`/`:2373`/`:2407`/`:3514`)各追加一行
  `self.sync_confirm_overlay(event_loop);`
- 主窗口 `Resized` 处理(`:2440` 附近解构 `settings_overlay,` 那一段)
  加 `confirm_overlay,` 解构 + 对应 `if let Some(overlay) = confirm_overlay
  { overlay.reposition(device, ..); }`(参数与 `settings_overlay` 的调用
  对齐,只是 `ConfirmOverlay::reposition` 签名少了 `_window_width`/
  `_window_height` 两个未用参数,直接按 Step 2 定义的四参数版调用)
- 主窗口 `CloseRequested`(应用退出)处理(`:3321`/`:3340` 附近)加
  `if let Some(overlay) = confirm_overlay { ... } *confirm_overlay = None;`
  同款收尾

- [ ] **Step 9: 加 `desired_confirm` 占位实现**

在 `confirm_overlay.rs` 末尾加:

```rust
/// 按 `if/else if` 优先级链算出"此刻该显示哪个 confirm 弹窗"(至多一个)。
/// Task 3 起逐个把 `else if false` 换成真实触发条件——现在还没有任何
/// 消费方接入通用宿主,诚实返回 `None`,不是"以后再实现"的占位符。
pub(crate) fn desired_confirm(
    _ws: Option<&crate::workspace::Workspace>,
) -> Option<(ConfirmTrigger, dialog::ConfirmDialog<Message>)> {
    None
}
```

- [ ] **Step 10: 编译验证**

Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译通过,`unused_variables`/`dead_code` 警告若出现在
`ConfirmOverlay` 尚未使用的方法上属预期(Task 3 起会用到),不是错误。

- [ ] **Step 11: `close_other_overlays` 互斥单测扩展**

找到 `window_events.rs` 里现有的 `close_other_overlays` 穷举式单测(参照
`search_overlay`/`file_history_overlay` 那几个 `sync_action` 测试所在的
`#[cfg(test)] mod tests`),补一条:

```rust
#[test]
fn close_other_overlays_confirm_closes_the_rest() {
    // 与现有 `close_other_overlays_settings_closes_the_rest` 之类的测试
    // 同款写法:构造一个把全部 5 个 `Option` 都设成 `Some`(用最小可行的
    // 假值)的 `Runner::Ready`,调用 `close_other_overlays(OverlayKind::
    // Confirm)`,断言只有 `confirm_overlay` 保留 `Some`,其余 4 个变
    // `None`。若现有测试没有現成的"构造一个假 Ready"辅助函数,参照
    // Task 3 起真正跑通窗口后再补——本任务若辅助函数缺失,先跳过这条,
    // 在 Task 3 有真实 `ConfirmOverlay::open` 调用点之后一并补上。
}
```

（如果这一步因为"没有可用的测试期 `Runner::Ready` 构造辅助"而无法
写出真正断言,在 Task 3 完成后回来补上——不要留一个空测试体通过编译
充数,要么写出真断言,要么在本任务的提交信息里注明"互斥测试延后到
Task 3"。）

- [ ] **Step 12: 提交**

```bash
git add crates/dozer-app/src/platform/confirm_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs
git commit -m "feat(overlay): scaffold generic ConfirmOverlay host (no consumers yet)"
```

---

### Task 3: 接入 Todo 清空列表确认(验证宿主骨架)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/view.rs:438-458`
  (`clear_confirm_popup` → `clear_confirm_spec`)
- Modify: `crates/dozer-app/src/platform/confirm_overlay.rs`
  (`desired_confirm` 加第一个分支)
- Modify: `crates/dozer-app/src/app/view.rs:441-452`(删除该 `stack!` 分支)

**Interfaces:**
- Consumes:`workspace::Workspace::todo`(`ws.todo.clear_confirm_open()`,
  既有方法,不变)。
- Produces:`todo::clear_confirm_spec(&WorkspaceState) ->
  dialog::ConfirmDialog<crate::app::Message>`。

- [ ] **Step 1: 改造 `clear_confirm_popup` → `clear_confirm_spec`**

`crates/dozer-app/src/extensions/todo/view.rs:438-458` 原函数:

```rust
pub fn clear_confirm_popup(
    _ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: Some(icons::IconKind::ListTodo),
            title: "清空列表".to_string(),
            description: "这会清空当前项目的全部任务,操作不可撤销。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::ClearListCancel,
            confirm_label: "清空".to_string(),
            confirm_msg: Message::ClearListConfirm,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 12.0,
        },
        window_width,
    )
}
```

改成(删掉 `crate::dialog::confirm(...)` 包装与 `window_width` 参数,
`cancel_msg`/`confirm_msg` 换成顶层 `crate::app::Message` 包一层
`Message::Todo(...)`,`Message` 局部别名在本文件仍指 `todo::Message`,
不冲突):

```rust
pub fn clear_confirm_spec(
    _ws_state: &WorkspaceState,
) -> crate::dialog::ConfirmDialog<crate::app::Message> {
    crate::dialog::ConfirmDialog {
        icon: Some(icons::IconKind::ListTodo),
        title: "清空列表".to_string(),
        description: "这会清空当前项目的全部任务,操作不可撤销。".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: crate::app::Message::Todo(Message::ClearListCancel),
        confirm_label: "清空".to_string(),
        confirm_msg: crate::app::Message::Todo(Message::ClearListConfirm),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 12.0,
    }
}
```

- [ ] **Step 2: `desired_confirm` 加分支**

`crates/dozer-app/src/platform/confirm_overlay.rs` 的 `desired_confirm`
函数体从 `None` 改成:

```rust
pub(crate) fn desired_confirm(
    ws: Option<&crate::workspace::Workspace>,
) -> Option<(ConfirmTrigger, dialog::ConfirmDialog<Message>)> {
    let ws = ws?;
    if ws.todo.clear_confirm_open() {
        Some((
            ConfirmTrigger::TodoClear,
            crate::extensions::todo::view::clear_confirm_spec(&ws.todo),
        ))
    } else {
        None
    }
}
```

- [ ] **Step 3: 删除 `app/view.rs` 里的旧 `stack!` 分支**

`crates/dozer-app/src/app/view.rs:441-452`:

```rust
        } else if ws.todo.clear_confirm_open() {
            // Todo"清空列表"确认弹窗:窗口级 overlay,与其它 Todo 浮层同款
            // "点遮罩即收起"约定。
            let dismiss = crate::dialog::scrim(Message::Todo(todo::Message::ClearListCancel));
            stack![
                base,
                dismiss,
                todo::clear_confirm_popup(&ws.todo, self.window_size.0).map(Message::Todo)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.todo.detail_popup_open() {
```

删掉 `} else if ws.todo.clear_confirm_open() { ... }` 整段(保留前后
相邻分支的 `} else if ws.todo.detail_popup_open() {` 与其上一个分支的
结尾 `}`,让链条直接从上一个分支跳到 `detail_popup_open` 分支)。

- [ ] **Step 4: 编译 + 现有 todo 测试验证**

Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo build -p dozer-app 2>&1 | tail -40`
Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo test -p dozer-app extensions::todo 2>&1 | tail -40`
Expected: 编译通过;`clear_confirm_spec` 若有既有测试引用旧函数名
`clear_confirm_popup`,一并改名(搜索 `clear_confirm_popup` 确认没有
遗漏的调用点)。

Run: `grep -rn "clear_confirm_popup" crates/dozer-app/src/`
Expected: 无匹配(除了本次改掉的这一处历史记录外,不应该还有其它调用点)。

- [ ] **Step 5: 回补 Task 2 Step 11 的互斥测试**

现在有了真实的 `ConfirmOverlay::open` 调用路径,补上 Task 2 Step 11 留下
的 `close_other_overlays_confirm_closes_the_rest` 测试——参照文件里其它
三个既有 `close_other_overlays_*_closes_the_rest` 测试的**实际写法**
(不是本计划猜测的写法,以仓库现有代码为准)构造 `Runner::Ready` 假值,
断言 `keep = OverlayKind::Confirm` 时其余四个变 `None`。

- [ ] **Step 6: 人工验证**

```bash
cd crates/dozer-app && RUSTC_WRAPPER= cargo build -p dozer-app --bin dozer 2>&1 | tail -20
```

`cargo run -p dozer-app` 手工验证清单:
1. Todo 面板点"清空列表" → 独立窗口弹出,居中于主窗口。
2. 拖动/resize 主窗口 → 弹窗跟随重新定位。
3. Esc → 弹窗关闭,列表未被清空。
4. 再次打开、点"清空” → 弹窗关闭,列表被清空。
5. 在 Files/Project 面板打开一个文件预览(webview 可见)的情况下打开这个
   确认框 → 确认框完全可见,不被 webview 遮挡(这是本次迁移要修的真实
   bug,必须确认修复)。
6. 打开这个确认框后再触发 Settings(⌘,)→ Todo 确认框应该被自动关闭,
   只剩 Settings 窗口(互斥验证)。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/extensions/todo/view.rs \
        crates/dozer-app/src/platform/confirm_overlay.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Todo clear-confirm to ConfirmOverlay"
```

---

### Task 4: 接入 Files 删除确认

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/view.rs:1220-1246`
- Modify: `crates/dozer-app/src/platform/confirm_overlay.rs`
- Modify: `crates/dozer-app/src/app/view.rs:116-125`

**Interfaces:**
- Produces:`files::view::delete_confirm_spec(&WorkspaceState) ->
  dialog::ConfirmDialog<crate::app::Message>`。

- [ ] **Step 1: 改造**

`crates/dozer-app/src/extensions/files/view.rs:1220-1246` 原函数改成:

```rust
pub fn delete_confirm_spec(
    ws_state: &WorkspaceState,
) -> Option<crate::dialog::ConfirmDialog<crate::app::Message>> {
    let (path, is_dir) = ws_state.tree_delete_confirm.as_ref()?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let kind = if *is_dir { "文件夹" } else { "文件" };
    Some(crate::dialog::ConfirmDialog {
        icon: None,
        title: format!("删除{kind} \"{name}\"?"),
        description: "会移入系统回收站,可从回收站找回。".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: crate::app::Message::Files(Message::DeleteCancel),
        confirm_label: "删除".to_string(),
        confirm_msg: crate::app::Message::Files(Message::DeleteConfirm),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    })
}
```

（这里返回 `Option` 而不是像 `todo::clear_confirm_spec` 那样直接返回
`ConfirmDialog`——因为 `tree_delete_confirm_is_some()` 这个判断本身就是
"内部有一个 `Option` 字段",让 `_spec` 函数自己承担"取出 `Some` 内容"
比调用方各自 `unwrap()` 更安全,`desired_confirm` 里用 `?`/`and_then`
自然衔接。`todo::clear_confirm_open()` 是布尔判断,没有内部 payload,
保持原样返回 `ConfirmDialog` 即可,两种形状都合理,不强求统一签名。）

- [ ] **Step 2: `desired_confirm` 优先级链最前面加 Files 分支**

```rust
pub(crate) fn desired_confirm(
    ws: Option<&crate::workspace::Workspace>,
) -> Option<(ConfirmTrigger, dialog::ConfirmDialog<Message>)> {
    let ws = ws?;
    if let Some(spec) = crate::extensions::files::view::delete_confirm_spec(&ws.files) {
        Some((ConfirmTrigger::FilesDelete, spec))
    } else if ws.todo.clear_confirm_open() {
        Some((
            ConfirmTrigger::TodoClear,
            crate::extensions::todo::view::clear_confirm_spec(&ws.todo),
        ))
    } else {
        None
    }
}
```

（顺序对齐现状 `app/view.rs` 的 `if/else if` 链——files-delete 排在
todo-clear 之前,与 Task 5-7 的顺序对应表见 Task 2 架构小节。）

- [ ] **Step 3: 删除 `app/view.rs:116-125` 旧分支**

```rust
        let popped = if ws.files.tree_delete_confirm_is_some() {
            let dismiss = crate::dialog::scrim(Message::Files(files::Message::DeleteCancel));
            stack![
                base,
                dismiss,
                files::delete_confirm_popup(&ws.files, self.window_size.0).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.pending_move_is_some() {
```

删掉 `let popped = if ws.files.tree_delete_confirm_is_some() { ... }`
这一整段,把 `let popped = if ws.files.pending_move_is_some() {` 顶替成
新的链首(`let popped = if ws.files.pending_move_is_some() {`,原样保留
这一分支,只是从 `else if` 降格成链首的 `if`)。

- [ ] **Step 4: 编译 + 验证**

Run: `grep -rn "delete_confirm_popup" crates/dozer-app/src/extensions/files/`
Expected: 无匹配。

Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo build -p dozer-app 2>&1 | tail -40`
Run: `cd crates/dozer-app && RUSTC_WRAPPER= cargo test -p dozer-app extensions::files 2>&1 | tail -40`

人工验证:文件树右键删除文件 → 独立窗口确认框出现;与 Todo 清空确认互斥
(开一个自动关另一个,理论上两者不会同时触发,但可以先开 Todo 确认、
再触发 Files 删除,验证前者被关掉)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/extensions/files/view.rs \
        crates/dozer-app/src/platform/confirm_overlay.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Files delete-confirm to ConfirmOverlay"
```

---

### Task 5: 接入 Database 删除数据源确认

**Files:**
- Modify: `crates/dozer-app/src/extensions/database/view.rs:580-605`
- Modify: `crates/dozer-app/src/platform/confirm_overlay.rs`
- Modify: `crates/dozer-app/src/app/view.rs:270-285`

**Interfaces:**
- Produces:`database::view::delete_confirm_spec(&WorkspaceState, &str) ->
  dialog::ConfirmDialog<crate::app::Message>`(`source_id` 是必须参数,
  不是 `Option` 内取出的——调用方已经在 `ws.database.delete_confirm()`
  拿到 `Some(id)` 才会调这个函数,签名保持既有的"外部已经判断过"这个
  约定,与 `delete_confirm_popup` 原签名一致)。

- [ ] **Step 1: 改造**

```rust
pub fn delete_confirm_spec(
    ws_state: &WorkspaceState,
    source_id: &str,
) -> crate::dialog::ConfirmDialog<crate::app::Message> {
    let name = ws_state
        .sources()
        .iter()
        .find(|s| s.id == source_id)
        .map(|s| s.name.to_string())
        .unwrap_or_else(|| source_id.to_string());
    crate::dialog::ConfirmDialog {
        icon: None,
        title: format!("删除数据源 \"{name}\"?"),
        description: "这会永久删除这条连接记录及其保存的密码。".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: crate::app::Message::Database(Message::DeleteSourceCancel),
        confirm_label: "删除".to_string(),
        confirm_msg: crate::app::Message::Database(Message::DeleteSource(
            source_id.to_string(),
        )),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    }
}
```

- [ ] **Step 2: `desired_confirm` 加分支(排在 Files 之后、SSH 之前)**

```rust
} else if let Some(source_id) = ws.database.delete_confirm() {
    Some((
        ConfirmTrigger::DatabaseDelete,
        crate::extensions::database::view::delete_confirm_spec(&ws.database, source_id),
    ))
```

插入位置在 Task 4 写的 `else if ws.todo.clear_confirm_open()` 之前
(与 `app/view.rs` 现状优先级一致:files-delete > database-delete >
ssh-delete(Task 6)> agent-tab-close(Task 7)> todo-clear)。

- [ ] **Step 3: 删除 `app/view.rs:270-285` 旧分支**

```rust
        } else if ws.database.delete_confirm().is_some() {
            // 数据库面板「删除数据源」确认框:窗口级 overlay,同上。三个
            // 数据库弹窗互斥优先级(同一时刻只显示一个):待确认删除 >
            // 新增/编辑表单 > 驱动管理。
            let source_id = ws.database.delete_confirm().unwrap();
            let dismiss =
                crate::dialog::scrim(Message::Database(database::Message::DeleteSourceCancel));
            stack![
                base,
                dismiss,
                database::delete_confirm_popup(&ws.database, source_id, self.window_size.0)
                    .map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.database.editing().is_some() {
```

删掉 `} else if ws.database.delete_confirm().is_some() { ... }` 整段。

- [ ] **Step 4: 编译 + 验证**

同 Task 4 Step 4 模式:`grep -rn "delete_confirm_popup"
crates/dozer-app/src/extensions/database/` 应无匹配;build + test +
`cargo run` 人工验证(Database 面板删除一个数据源)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/extensions/database/view.rs \
        crates/dozer-app/src/platform/confirm_overlay.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Database delete-confirm to ConfirmOverlay"
```

---

### Task 6: 接入 SSH 删除主机确认

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs:1364-1389`
- Modify: `crates/dozer-app/src/platform/confirm_overlay.rs`
- Modify: `crates/dozer-app/src/app/view.rs:316-329`

- [ ] **Step 1: 改造**

```rust
pub fn delete_confirm_spec(
    ws_state: &WorkspaceState,
    host_id: &str,
) -> crate::dialog::ConfirmDialog<crate::app::Message> {
    let name = ws_state
        .hosts()
        .iter()
        .find(|h| h.id == host_id)
        .map(|h| h.name.as_str())
        .unwrap_or(host_id)
        .to_string();
    crate::dialog::ConfirmDialog {
        icon: None,
        title: format!("删除主机 \"{name}\"?"),
        description: "这会永久删除这台主机的连接记录。".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: crate::app::Message::Ssh(Message::DeleteHostCancel),
        confirm_label: "删除".to_string(),
        confirm_msg: crate::app::Message::Ssh(Message::DeleteHost(host_id.to_string())),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    }
}
```

（`.to_string()` 提前是因为原函数借用 `ws_state` 返回 `&str`,新签名
不再返回借用类型,直接拥有权更简单。）

- [ ] **Step 2: `desired_confirm` 加分支(排在 database-delete 之后、
  agent-tab-close 之前)**

```rust
} else if let Some(host_id) = ws.ssh.delete_confirm() {
    Some((
        ConfirmTrigger::SshDelete,
        crate::extensions::ssh::delete_confirm_spec(&ws.ssh, host_id),
    ))
```

- [ ] **Step 3: 删除 `app/view.rs:316-329` 旧分支**

```rust
        } else if ws.ssh.delete_confirm().is_some() {
            // 主机面板「删除主机」确认框:窗口级 overlay,同上。两个主机
            // 弹窗互斥优先级(同一时刻只显示一个):待确认删除 > 新增/编辑
            // 表单。
            let host_id = ws.ssh.delete_confirm().unwrap();
            let dismiss = crate::dialog::scrim(Message::Ssh(ssh::Message::DeleteHostCancel));
            stack![
                base,
                dismiss,
                ssh::delete_confirm_popup(&ws.ssh, host_id, self.window_size.0).map(Message::Ssh)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.ssh.editing().is_some() {
```

- [ ] **Step 4: 编译 + 验证**(同 Task 4/5 模式,针对 SSH 面板)

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/extensions/ssh.rs \
        crates/dozer-app/src/platform/confirm_overlay.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate SSH delete-confirm to ConfirmOverlay"
```

---

### Task 7: 接入关闭 Agent tab 确认

**Files:**
- Modify: `crates/dozer-app/src/workspace/view.rs:364-390`
- Modify: `crates/dozer-app/src/platform/confirm_overlay.rs`
- Modify: `crates/dozer-app/src/app/view.rs:347-359`

- [ ] **Step 1: 改造**

`workspace/view.rs:364-390` 原函数(见前文引用,完整内容已在 brainstorming
阶段确认过)改成:

```rust
pub(crate) fn agent_close_confirm_spec(
    ws: &Workspace,
) -> Option<crate::dialog::ConfirmDialog<crate::app::Message>> {
    let idx = ws.pending_close_tab?;
    let title = ws
        .tabs
        .get(idx)
        .map(|t| crate::workspace::hook::tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
        .unwrap_or_else(|| "会话".to_string());
    Some(crate::dialog::ConfirmDialog {
        icon: None,
        title: format!("关闭 \"{title}\"?"),
        description: "该会话仍在运行 / 等待输入,关闭会结束此会话。".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: Message::TermTabCloseCancel,
        confirm_label: "关闭".to_string(),
        confirm_msg: Message::TermTabCloseConfirm,
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    })
}
```

（这里 `Message` 就是顶层 `crate::app::Message`——`workspace/view.rs`
本来就 `use crate::app::Message;` 直接引用顶层类型,不像 `files/view.rs`
那样有一个局部 `files::Message` 别名,不需要 `crate::app::Message::
Files(...)` 那层包装。以 `workspace/view.rs` 文件顶部实际的
`use` 语句为准确认这一点,若发现该文件其实也有局部 `Message` 别名,则
按 Task 4-6 的写法加 `crate::app::Message` 全限定。）

- [ ] **Step 2: `desired_confirm` 加分支(排在 ssh-delete 之后、
  todo-clear 之前——最后一个)**

```rust
} else if let Some(spec) = crate::workspace::view::agent_close_confirm_spec(ws) {
    Some((ConfirmTrigger::AgentTabClose, spec))
```

- [ ] **Step 3: 删除 `app/view.rs:347-359` 旧分支**

```rust
        } else if ws.pending_close_tab.is_some() {
            // 关 Agent 面板 tab 确认框:目标会话 Running/AwaitingInput 时才进
            // 这里(分流见 `app::update` `Message::CloseTab`)。窗口级 overlay,
            // 点遮罩=取消。
            let dismiss = crate::dialog::scrim(Message::TermTabCloseCancel);
            stack![
                base,
                dismiss,
                agent_close_confirm_popup(ws, self.window_size.0)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.agent_picker_open {
```

- [ ] **Step 4: 编译 + 验证**

Run: `grep -rn "agent_close_confirm_popup" crates/dozer-app/src/`
Expected: 无匹配。

人工验证:Agent 面板关闭一个 Running/AwaitingInput 的 tab → 独立窗口
确认框出现。至此 5 个 confirm 形态弹窗全部迁完,`ConfirmOverlay` 通用
宿主验证完成。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/workspace/view.rs \
        crates/dozer-app/src/platform/confirm_overlay.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate agent-tab-close-confirm to ConfirmOverlay"
```

---

### Task 8: 定制宿主——Database 驱动管理(无 IME)

从这里起,每个定制宿主结构与 `settings_overlay.rs` 同构,本任务给出完整
模板,后续 6 个任务照此结构替换类型/字段名。

**Files:**
- Create: `crates/dozer-app/src/platform/database_drivers_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind`/`close_other_overlays`/`Ready`/`sync_*`/`window_event`/
  `Resized`/`CloseRequested`,同 Task 2 的接线模式,变体名
  `OverlayKind::DatabaseDrivers`)
- Modify: `crates/dozer-app/src/extensions/database/view.rs:16-`
  (`drivers_popup` 函数体不变,只改外层容器 `Length::Fixed`→`Length::Fill`
  并改名 `database_drivers_card`)
- Modify: `crates/dozer-app/src/app/view.rs:304-315`(删旧分支)

**Interfaces:**
- Produces:`pub(crate) struct DatabaseDriversOverlay`,方法
  `open`/`reposition`/`redraw`/`handle_input`/`handle_focus`/`window_id`/
  `request_redraw`,签名对齐 `SettingsOverlay`。

- [ ] **Step 1: 改造视图函数**

`database/view.rs` 里的 `drivers_popup`(`:16-`)函数签名从

```rust
pub fn drivers_popup<'a>(
    app_state: &'a AppState,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

改成

```rust
pub fn database_drivers_card<'a>(
    app_state: &'a AppState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

函数体内部找到最外层 `container(...)` 的 `.width(dialog::width(window_width))`
(或等价的 `Length::Fixed`)调用,改成 `.width(Length::Fill).height(Length::Fill)`
——具体位置以函数体实际最外层容器为准,原则同 `settings_card`/
`file_history_card` 当初的调整:独立窗口本身已经是量好的画布。

- [ ] **Step 2: 写 `database_drivers_overlay.rs`**

```rust
//! 数据库「管理驱动」弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`
//! 「架构」第 2 节。结构与 `settings_overlay.rs` 同构,无 IME/原生右键
//! 菜单需求(纯列表展示,无文本输入)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::database;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 420.0)
}

pub(crate) struct DatabaseDriversOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl DatabaseDriversOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> DatabaseDriversOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "database-drivers", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        DatabaseDriversOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let mut interface = UserInterface::build(
            database::view::database_drivers_card(&app.database).map(Message::Database),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Database(
                crate::extensions::database::Message::DriversPopupToggle,
            )];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let mut interface = UserInterface::build(
            database::view::database_drivers_card(&app.database).map(Message::Database),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}
```

（`app.database` 已确认是 `App` 结构体上的普通字段
`pub(crate) database: database::AppState`——`app/app.rs:450`——不是
`Option`,也不需要额外访问器,`platform/` 下的文件与 `App` 同 crate,
`pub(crate)` 可直接访问,`&app.database` 直接用即可。）

- [ ] **Step 3: `OverlayKind`/`close_other_overlays`/`Ready`/`sync_*`/
  `window_event`/`Resized`/`CloseRequested` 接线**

完全照抄 Task 2 Step 3-8 的模式,把 `Confirm`/`confirm_overlay`/
`ConfirmOverlay` 替换成 `DatabaseDrivers`/`database_drivers_overlay`/
`DatabaseDriversOverlay`,`sync_confirm_overlay` 的开关条件替换成
`app.database.drivers_popup_open()`(布尔,不像 `ConfirmOverlay` 需要
判别哪一个 trigger,直接照抄 `sync_settings_overlay` 的三段 `match
SyncAction` 模式,不是 `ConfirmOverlay` 那种"5 选 1"模式)。

- [ ] **Step 4: 删除 `app/view.rs:304-315` 旧分支**

```rust
        } else if self.database.drivers_popup_open() {
            // 数据库面板「管理驱动」弹窗:窗口级 overlay,同上。
            let dismiss =
                crate::dialog::scrim(Message::Database(database::Message::DriversPopupToggle));
            stack![
                base,
                dismiss,
                database::drivers_popup(&self.database, self.window_size.0).map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.ssh.delete_confirm().is_some() {
```

- [ ] **Step 5: 编译 + 人工验证**

Run: `grep -rn "fn drivers_popup\b" crates/dozer-app/src/` → 应无匹配
(已改名 `database_drivers_card`)。

`cargo build` + `cargo run` 验证:Database 面板"管理驱动"按钮 → 独立
窗口弹出;webview 显示中打开不被遮挡;Esc 关闭。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/platform/database_drivers_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/database/view.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Database drivers panel to independent window"
```

---

### Task 9: 定制宿主——Files 移动确认(无 IME,但不接失焦关闭)

**特殊之处**(与 Task 8 模板的关键差异,不能照抄):"到目录"旁的
`...` 按钮(`Message::MoveDirBrowse`)会在 `Runner::dispatch`
(`window_events.rs:1566`)里同步弹出 `rfd::FileDialog::pick_folder()`
原生模态选择器,这会让本宿主窗口收到一次真实 `Focused(false)`——必须像
`ProjectCreateOverlay` 那样**不接失焦关闭**(`handle_focus` 不调用,
`WindowEvent::Focused` 分支留空/不处理),只认 Esc 与表单自身的
取消按钮。且"新名称"输入框支持中文文件名,需要 IME。

**Files:**
- Create: `crates/dozer-app/src/platform/files_move_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind::FilesMove`,同 Task 2/8 接线模式,**`window_event`
  分支里不处理 `WindowEvent::Focused`**,照抄 `project_create_overlay`
  在 `window_events.rs:2355-2361` 的写法与注释)
- Modify: `crates/dozer-app/src/extensions/files/view.rs:1253-`
  (`move_confirm_popup` → `files_move_card`,签名去掉 `window_width`,
  外层容器改 `Length::Fill`)
- Modify: `crates/dozer-app/src/app/view.rs:126-135`(删旧分支)

- [ ] **Step 1: 改造视图函数**

`files/view.rs` 的 `move_confirm_popup`(`:1253-`)改名
`files_move_card`,签名去掉 `window_width: f32` 参数,函数体内部
`column![...]`/`container(...)` 最外层的宽度约束(若原来靠调用方套了
`Length::Fixed`,以实际代码为准)统一改 `Length::Fill`。

- [ ] **Step 2: 写 `files_move_overlay.rs`**

结构同 Task 8 的 `DatabaseDriversOverlay`,但:
- 需要 `window.set_ime_allowed(true)`(`open` 里,`OverlayGpu::open`
  之后)。
- **不实现失焦关闭**:`handle_focus` 方法不要——`window_event` 里对应
  分支直接不处理 `WindowEvent::Focused`(照抄
  `project_create_overlay` 在 `window_events.rs:2355-2361` 那段的
  写法与注释原因)。
- `handle_input` 的 Esc 分支发 `Message::Files(files::Message::
  MoveCancel)`。
- 依赖原生右键粘贴菜单(文本输入框):打开时
  `#[cfg(target_os = "macos")] crate::chrome::native_menu::
  install_content_view(&window);`,`Drop` 时指回主窗口(照抄
  `SettingsOverlay` 的 `Drop` 实现)。

```rust
//! 文件树"拖拽移动"确认弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! **不接失焦关闭**:"到目录"旁的浏览按钮(`MoveDirBrowse`)会同步弹出
//! 原生 `rfd` 目录选择器,那会让本窗口收到一次真实 `Focused(false)`,
//! 若照常触发失焦即关闭会把正在填的移动表单整个关掉——同
//! `ProjectCreateOverlay` 的既有考量。需要 IME(新文件名可能是中文)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::files;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(460.0, 220.0)
}

pub(crate) struct FilesMoveOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl FilesMoveOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> FilesMoveOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "files-move", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        FilesMoveOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let mut interface = UserInterface::build(
            files::view::files_move_card(&ws.files).map(Message::Files),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Files(files::Message::MoveCancel)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            files::view::files_move_card(&ws.files).map(Message::Files),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

impl Drop for FilesMoveOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}
```

- [ ] **Step 3: `window_event` 分流分支不处理 `Focused`**

照抄 `window_events.rs:2355-2361` 里 `project_create_overlay` 分支
对 `WindowEvent::Focused` 的处理方式(不匹配这个变体,落进 `else` 分支
一起交给 `handle_input` 的普通消息路径,注释说明原因同该处现有注释)。

- [ ] **Step 4: 删除 `app/view.rs:126-135` 旧分支**

```rust
        } else if ws.files.pending_move_is_some() {
            let dismiss = crate::dialog::scrim(Message::Files(files::Message::MoveCancel));
            stack![
                base,
                dismiss,
                files::move_confirm_popup(&ws.files, self.window_size.0).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.files.context_menu_is_some() {
```

- [ ] **Step 5: 编译 + 人工验证**

`cargo build` + `cargo run`:文件树拖拽移动一个文件 → 独立窗口弹出;
点"…"浏览按钮 → 原生目录选择器弹出且**不关闭移动表单窗口**;选完目录
后表单窗口仍在,回填目录;中文文件名输入正常;Esc 关闭。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/platform/files_move_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/files/view.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Files move-confirm to independent window"
```

---

### Task 10: 定制宿主——Project 修复进度(无 IME,阻塞态)

**特殊之处**:`scrim_blocking` 语义(不可通过点遮罩/Esc/失焦关闭)——
`handle_input` 不响应 Esc,`window_event` 分支不处理 `Focused`,
`WindowEvent::CloseRequested`(用户点原生窗口的关闭按钮)也不能直接关,
只有 `run.all_done()` 为真时表单自身的"关闭"按钮点击才产生
`Message::Project(project::Message::...)` 关闭消息。

**Files:**
- Create: `crates/dozer-app/src/platform/project_scaffold_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind::ProjectScaffold`)
- Modify: `crates/dozer-app/src/extensions/project/view.rs:516-`
  (`scaffold_progress_popup` → `project_scaffold_card`)
- Modify: `crates/dozer-app/src/app/view.rs:256-269`(删旧分支)

- [ ] **Step 1: 改造视图函数**

`scaffold_progress_popup`(`:516-`)改名 `project_scaffold_card`,签名去
`window_width`,外层容器改 `Length::Fill`。内部"关闭"按钮的
`on_press_maybe`(`done` 时才激活)逻辑不变。

- [ ] **Step 2: 写 `project_scaffold_overlay.rs`**

```rust
//! Project「修复项目」进度弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! `scrim_blocking` 语义原样保留:进行中不可通过 Esc/失焦/原生窗口关闭
//! 按钮关闭,只有全部步骤完成(`run.all_done()`)后内容里的"关闭"按钮
//! 才能真正关闭——`handle_input` 不判断 Esc,`window_event` 分流分支
//! 不处理 `WindowEvent::Focused`/`CloseRequested`,与 `ConfirmOverlay`/
//! 其余定制宿主的默认行为都不同,不要在整理代码时"顺手"补上。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::project;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 360.0)
}

pub(crate) struct ProjectScaffoldOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl ProjectScaffoldOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> ProjectScaffoldOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "project-scaffold", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        ProjectScaffoldOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    // 故意不提供 `handle_focus`——本宿主不接失焦关闭,`window_event`
    // 分流分支直接不匹配 `WindowEvent::Focused`,交给下面 `handle_input`
    // 的普通事件路径(iced 内部会忽略它,不产生任何消息)。

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let mut interface = UserInterface::build(
            project::view::project_scaffold_card(&ws.project_panel).map(Message::Project),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    /// 不判断 Esc——这是本宿主与其余全部定制宿主的关键差异,进行中唯一
    /// 能产生关闭消息的方式是点内容里的"关闭"按钮(`done` 时才可点)。
    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            project::view::project_scaffold_card(&ws.project_panel).map(Message::Project),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}
```

`window_events.rs` 里本宿主对应的 `window_event` 分流分支(照 Task 2
Step 7 的模式写)**不要**加 `WindowEvent::Focused`/`CloseRequested` 的
特殊处理分支——两者都落进最后的 `else { self.dispatch(overlay.
handle_input(...)); }` 兜底,`handle_input` 对这两类事件本就不产生任何
消息(`conversion::window_event` 会把它们转换成 iced 事件喂给
`interface.update`,但没有任何 widget 会响应,不会产生消息),效果就是
"什么都不做",不需要额外拦截。

- [ ] **Step 3: 删除 `app/view.rs:256-269` 旧分支**

```rust
        } else if ws.project_panel.scaffold_run.is_some() {
            // 项目面板「修复项目」进度弹窗:窗口级 overlay。进行中不可通过
            // 点遮罩关闭(`scrim_blocking` 不挂 `on_press`),同 panel-level
            // 版本的既有约定(spec"弹窗可取消性"一节)。
            let scrim = crate::dialog::scrim_blocking();
            stack![
                base,
                scrim,
                project::scaffold_progress_popup(&ws.project_panel, self.window_size.0)
                    .map(Message::Project)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.database.delete_confirm().is_some() {
```

- [ ] **Step 4: 编译 + 人工验证**

`cargo build` + `cargo run`:触发"修复项目" → 独立窗口弹出,进行中
Esc/点原生关闭按钮均无效,全部步骤完成后"关闭"按钮可点、点击后窗口
关闭。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/project_scaffold_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/project/view.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Project scaffold-progress to independent window"
```

---

### Task 11: 定制宿主——Database 数据源表单(需 IME)

**Files:**
- Create: `crates/dozer-app/src/platform/database_source_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind::DatabaseSource`)
- Modify: `crates/dozer-app/src/extensions/database/view.rs:242-`
  (`source_form` → `database_source_card`)
- Modify: `crates/dozer-app/src/app/view.rs:286-303`(删旧分支)

结构同 Task 9(`FilesMoveOverlay`)——需要 IME + 原生右键粘贴菜单挂靠
(数据源名称/连接串字段是文本输入)。**先确认**表单内是否也有类似
`MoveDirBrowse` 的"浏览本地文件"按钮(`grep -n "rfd::" crates/dozer-app/
src/extensions/database/`——brainstorming 阶段已核实**没有**,本表单
可以用简单失焦即关闭,不需要 Task 9 那种"不接失焦"的例外)。

- [ ] **Step 1: 改造视图函数**

`source_form`(`:242-`)改名 `database_source_card`,签名去
`window_width`,外层 `Length::Fill`。签名保留 `draft`/`app_state`/
`test_status` 三个既有参数(不含 `window_width`)。

- [ ] **Step 2: 写 `database_source_overlay.rs`**

```rust
//! Database「新增/编辑数据源」表单的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 简单失焦即关闭 + IME + 原生右键粘贴菜单——结构同 `settings_overlay.rs`,
//! 无需 `suppress_next_blur`(表单内无会拉起系统浏览器/原生选择器的按钮,
//! brainstorming 阶段已用 `grep -rn "rfd::" extensions/database/` 核实)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::database;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 480.0)
}

pub(crate) struct DatabaseSourceOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl DatabaseSourceOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> DatabaseSourceOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "database-source", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        DatabaseSourceOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    /// `draft` 从 `ws.database.editing()` 取——`redraw`/`handle_input` 顶部
    /// 短路,`None` 表示表单已经在别处被关掉(过期一帧),直接不画。
    fn card<'a>(
        ws: &'a crate::workspace::Workspace,
        app_state: &'a database::AppState,
    ) -> Option<iced_widget::core::Element<'a, database::Message, iced_widget::Theme, iced_renderer::Renderer>> {
        let draft = ws.database.editing()?;
        Some(database::view::database_source_card(
            draft,
            app_state,
            ws.database.draft_test_status(),
        ))
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let Some(card) = Self::card(ws, &app.database) else {
            return;
        };
        let mut interface = UserInterface::build(
            card.map(Message::Database),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Database(database::Message::DraftCancel)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let Some(card) = Self::card(ws, &app.database) else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            card.map(Message::Database),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

impl Drop for DatabaseSourceOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}
```

（`app.database` 是 `pub(crate) database: database::AppState` 普通字段
——见 Task 8 的确认,与 `DatabaseDriversOverlay` 用同一种直接字段访问,
不需要额外访问器。）

- [ ] **Step 3: 删除 `app/view.rs:286-303` 旧分支**

```rust
        } else if ws.database.editing().is_some() {
            // 数据库面板「新增/编辑数据源」表单:窗口级 overlay,同上。
            let draft = ws.database.editing().unwrap();
            let dismiss = crate::dialog::scrim(Message::Database(database::Message::DraftCancel));
            stack![
                base,
                dismiss,
                database::source_form(
                    draft,
                    &self.database,
                    ws.database.draft_test_status(),
                    self.window_size.0,
                )
                .map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.database.drivers_popup_open() {
```

- [ ] **Step 4: 编译 + 人工验证**

`cargo build` + `cargo run`:新增数据源 → 独立窗口表单弹出;中文名称
输入 + 候选词正常;右键粘贴正常;Esc 关闭;webview 显示中打开不被遮挡。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/database_source_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/database/view.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Database source form to independent window"
```

---

### Task 12: 定制宿主——SSH 主机表单(需 IME)

**Files:**
- Create: `crates/dozer-app/src/platform/ssh_host_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind::SshHost`)
- Modify: `crates/dozer-app/src/extensions/ssh.rs:1022-`
  (`host_form` → `ssh_host_card`)
- Modify: `crates/dozer-app/src/app/view.rs:330-346`(删旧分支)

结构与 Task 11(`DatabaseSourceOverlay`)同款:简单失焦关闭 + IME + 原生
菜单挂靠(brainstorming 已核实 SSH 表单无 `rfd::` 调用,不需要 Task 9
那种"不接失焦"的例外)。

- [ ] **Step 1: 改造视图函数**

`ssh.rs` 里的 `host_form`(`:1022-`)函数签名从

```rust
pub fn host_form<'a>(
    draft: &'a SshHostDraft,
    status: &'a TestStatus,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

改成

```rust
pub fn ssh_host_card<'a>(
    draft: &'a SshHostDraft,
    status: &'a TestStatus,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

函数体内部最外层容器的宽度约束改 `Length::Fill`(具体位置以函数体实际
最外层 `container(...)`/`column![...]` 为准)。

- [ ] **Step 2: 写 `ssh_host_overlay.rs`**

```rust
//! SSH「添加/编辑主机」表单的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 简单失焦即关闭 + IME + 原生右键粘贴菜单——结构同 `settings_overlay.rs`,
//! 无需 `suppress_next_blur`(表单内无会拉起系统浏览器/原生选择器的按钮)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::ssh;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 480.0)
}

pub(crate) struct SshHostOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl SshHostOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> SshHostOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "ssh-host", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        SshHostOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    fn card<'a>(ws: &'a crate::workspace::Workspace) -> Option<iced_widget::core::Element<'a, ssh::Message, iced_widget::Theme, iced_renderer::Renderer>> {
        let draft = ws.ssh.editing()?;
        let status = draft
            .id
            .as_deref()
            .map(|id| ws.ssh.test_status(id))
            .unwrap_or(&ssh::TestStatus::Idle);
        Some(ssh::ssh_host_card(draft, status))
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let Some(card) = Self::card(ws) else {
            return;
        };
        let mut interface = UserInterface::build(
            card.map(Message::Ssh),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Ssh(ssh::Message::DraftCancel)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let Some(card) = Self::card(ws) else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            card.map(Message::Ssh),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

impl Drop for SshHostOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}
```

- [ ] **Step 3: 删除 `app/view.rs:330-346` 旧分支**

```rust
        } else if ws.ssh.editing().is_some() {
            // 主机面板「添加/编辑主机」表单:窗口级 overlay,同上。
            let draft = ws.ssh.editing().unwrap();
            let status = draft
                .id
                .as_deref()
                .map(|id| ws.ssh.test_status(id))
                .unwrap_or(&ssh::TestStatus::Idle);
            let dismiss = crate::dialog::scrim(Message::Ssh(ssh::Message::DraftCancel));
            stack![
                base,
                dismiss,
                ssh::host_form(draft, status, self.window_size.0).map(Message::Ssh)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.pending_close_tab.is_some() {
```

- [ ] **Step 4: 编译 + 人工验证**

`cargo build` + `cargo run`:SSH 面板新增/编辑主机 → 独立窗口表单弹出;
中文主机名称输入 + 候选词正常;右键粘贴正常;Esc 关闭;webview 显示中
打开不被遮挡。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/ssh_host_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/ssh.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate SSH host form to independent window"
```

---

### Task 13: 定制宿主——Project 删除确认(无 IME)

**已核实**(写计划阶段读过完整函数体,不是 brainstorming 阶段的推测):
`project_delete_confirm_popup`(`extensions/project/view.rs:592-`)只有
一个 `radio_row` 三选一单选控件(`delete::DeleteScope`)+ 取消/确认按钮,
**没有任何 `text_input`**。spec 早前的"输入项目名确认"这个描述不准确,
已在 spec 文档更正。本任务用 Task 8(`DatabaseDriversOverlay`)那种更
简单的无 IME/无原生菜单模板。

**Files:**
- Create: `crates/dozer-app/src/platform/project_delete_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind::ProjectDelete`)
- Modify: `crates/dozer-app/src/extensions/project/view.rs:592-`
  (`project_delete_confirm_popup` → `project_delete_card`)
- Modify: `crates/dozer-app/src/app/view.rs:241-255`(删旧分支)

- [ ] **Step 1: 改造视图函数**

`project/view.rs` 的 `project_delete_confirm_popup`(`:592-`)函数签名从

```rust
pub fn project_delete_confirm_popup(
    ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

改成

```rust
pub fn project_delete_card(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

函数体内部最外层容器宽度约束改 `Length::Fill`。

- [ ] **Step 2: 写 `project_delete_overlay.rs`**

```rust
//! Project「删除项目」三选一确认弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 纯单选(`delete::DeleteScope`)+ 取消/确认按钮,无文本输入,不需要
//! IME/原生右键菜单——结构同 `settings_overlay.rs` 去掉 IME 那两行。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::project;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(440.0, 260.0)
}

pub(crate) struct ProjectDeleteOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl ProjectDeleteOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> ProjectDeleteOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "project-delete", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        ProjectDeleteOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let mut interface = UserInterface::build(
            project::view::project_delete_card(&ws.project_panel).map(Message::Project),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Project(project::Message::DeleteProjectCancel)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            project::view::project_delete_card(&ws.project_panel).map(Message::Project),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}
```

- [ ] **Step 3: 删除 `app/view.rs:241-255` 旧分支**

```rust
        } else if ws.project_panel.delete_pending.is_some() {
            // 项目面板「删除项目」确认框:窗口级 overlay,同其它面板弹窗
            // 的既有口径(2026-09-15 起——此前是 panel-level `stack!`,只在
            // 本面板宽度范围内居中,不是整个软件窗体)。
            let dismiss =
                crate::dialog::scrim(Message::Project(project::Message::DeleteProjectCancel));
            stack![
                base,
                dismiss,
                project::project_delete_confirm_popup(&ws.project_panel, self.window_size.0)
                    .map(Message::Project)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.project_panel.scaffold_run.is_some() {
```

- [ ] **Step 4: 编译 + 人工验证**

`cargo build` + `cargo run`:项目面板"删除项目" → 独立窗口三选一确认框
弹出;三个单选项可正常切换;Esc 关闭;webview 显示中打开不被遮挡。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/project_delete_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/project/view.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Project delete-confirm to independent window"
```

---

### Task 14: 定制宿主——Todo 详情(需 IME)

**Files:**
- Create: `crates/dozer-app/src/platform/todo_detail_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
  (`OverlayKind::TodoDetail`)
- Modify: `crates/dozer-app/src/app/update.rs:4706-`
  (`App::todo_detail_popup` 方法 → 拆成自由函数
  `crate::extensions::todo::view::todo_detail_card(&Workspace)`)
- Modify: `crates/dozer-app/src/app/view.rs:453-460`(删旧分支)

**特殊之处**:`App::todo_detail_popup` 目前是 `App` 的方法(`&self`,
内部自己调 `self.active_workspace()`),不是接收 `&WorkspaceState`
的自由函数——迁移时要把它变成自由函数 `todo_detail_card(ws: &Workspace)
-> Element<...>`,调用方(`TodoDetailOverlay::redraw`/`handle_input`)
自己先取 `app.active_workspace()` 再传进去,与其余 6 个定制宿主的既有
写法(`redraw` 内部 `let Some(ws) = app.active_workspace() else {
return; }` 那一步)保持一致,不要把这个方法留在 `App` 上单独特殊处理。

结构同 Task 11(`DatabaseSourceOverlay`):简单失焦关闭 + IME + 原生右键
菜单挂靠(任务详情含标题/备注文本编辑,brainstorming 已核实 Todo 扩展
无 `rfd::` 调用)。

- [ ] **Step 1: 拆出自由函数**

`app/update.rs:4706-` 的 `pub(crate) fn todo_detail_popup<'a>(&self)
-> Element<'a, Message, ...>` 函数体里,把 `let Some(ws) =
self.active_workspace() else { return column![].into(); };` 这一步
挪到调用方(`TodoDetailOverlay`),函数本身改成:

```rust
// 从 `app/update.rs` 挪到 `extensions/todo/view.rs`,改名并接收
// `&Workspace` 而不是 `&App`。
pub fn todo_detail_card(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(idx) = ws.todo.detail_open_idx() else {
        return column![].into();
    };
    let Some(item) = ws.todo.items().get(idx) else {
        return column![].into();
    };
    // ...原函数体其余部分原样保留（把 `self.xxx` 替换成 `ws.xxx`，
    // 原函数除了开头的 `active_workspace()` 之外不应该再引用 `App`
    // 的其它字段——若发现确实引用了，先在本步骤记录下来，不要静默
    // 丢弃那部分逻辑）。
}
```

搬到 `crates/dozer-app/src/extensions/todo/view.rs`(该文件已有
`use` 了本函数体所需的大部分类型;若缺失,按报错逐个补 `use`)。
`app/update.rs` 原方法整个删除。

- [ ] **Step 2: 写 `todo_detail_overlay.rs`**

```rust
//! Todo 任务详情弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 简单失焦即关闭 + IME + 原生右键粘贴菜单——结构同 `settings_overlay.rs`,
//! 无需 `suppress_next_blur`。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::todo;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(520.0, 480.0)
}

pub(crate) struct TodoDetailOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl TodoDetailOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> TodoDetailOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "todo-detail", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        TodoDetailOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let mut interface = UserInterface::build(
            todo::view::todo_detail_card(ws).map(Message::Todo),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Todo(todo::Message::DetailClose)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            todo::view::todo_detail_card(ws).map(Message::Todo),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

impl Drop for TodoDetailOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}
```

- [ ] **Step 3: 删除 `app/view.rs:453-460` 旧分支**

```rust
        } else if ws.todo.detail_popup_open() {
            // 任务详情弹窗:窗口级 overlay,原生渲染(不走 wry webview)。
            // 点弹层外任意处经 dismiss 收起,与其它 Todo 浮层同款约定。
            let dismiss = crate::dialog::scrim(Message::Todo(todo::Message::DetailClose));
            stack![base, dismiss, self.todo_detail_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_filter_popup_open() {
```

- [ ] **Step 4: 编译 + 人工验证**

Run: `grep -rn "fn todo_detail_popup" crates/dozer-app/src/`
Expected: 无匹配。

`cargo build` + `cargo run`:点开一个任务详情 → 独立窗口弹出;中文
备注编辑 + IME 正常;Esc 关闭;webview 显示中打开不被遮挡。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/todo_detail_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/extensions/todo/view.rs \
        crates/dozer-app/src/app/update.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(overlay): migrate Todo detail popup to independent window"
```

---

### Task 15: 最终清理

**Files:**
- Modify: `crates/dozer-app/src/app/view.rs`(确认 `if/else if` 链只剩
  范围外的 13 处菜单/下拉与最终 `else` 兜底)
- Modify: `crates/dozer-app/src/dialog.rs`(视 grep 结果决定是否删
  `scrim`/`scrim_blocking`)
- Modify: `crates/dozer-app/src/app/app.rs`(核对 `app_modal_open` 两处
  不需要改动,补注释说明"12 个模态卡片弹窗已迁独立窗口,不参与这里的
  隐藏判断"防止未来误加)

- [ ] **Step 1: 确认 12 个旧分支已全部清空**

```bash
grep -n "crate::dialog::scrim(" crates/dozer-app/src/app/view.rs
```

Expected: 0 处匹配(12 个全部迁完;若仍有匹配,说明前面某个 Task 漏做,
回去补上,不要在这里绕过)。

- [ ] **Step 2: 检查 `dialog::scrim`/`scrim_blocking` 是否还有调用点**

```bash
grep -rn "dialog::scrim\b\|dialog::scrim_blocking\b" crates/dozer-app/src/
```

若 0 处匹配,删除 `crates/dozer-app/src/dialog.rs` 里的 `scrim`/
`scrim_blocking`/`scrim_layer` 三个函数(`:58-125` 附近,以实际内容为准)
及其文档注释里对这两者的介绍段落。若仍有匹配(某个范围外的锚点菜单/
下拉其实也在用 `scrim` 而不是 `MouseArea`,盘点阶段的启发式判断有漏网),
保留这两个函数,在提交信息里注明具体是哪个调用点导致保留。

- [ ] **Step 3: 全量验证**

```bash
cd crates/dozer-app && RUSTC_WRAPPER= cargo build -p dozer-app 2>&1 | tail -60
cd crates/dozer-app && RUSTC_WRAPPER= cargo clippy -p dozer-app --all-targets 2>&1 | tail -100
cd crates/dozer-app && RUSTC_WRAPPER= cargo test -p dozer-app 2>&1 | tail -80
cargo fmt -p dozer-app
```

Expected:build/clippy 无新增警告或错误(与迁移前基线比对,只允许"个别
函数因为签名改变而产生的、已经在对应 Task 里处理过的"变化);全部既有
测试通过。

- [ ] **Step 4: 完整人工回归清单**

`cargo run -p dozer-app` 逐一走一遍全部 12 个弹窗(每个:打开、内容正确、
webview 显示中打开不被遮挡、Esc 或对应关闭方式生效、与至少另一个弹窗
互斥),外加:
- 连续快速切换触发不同的 5 个 confirm 形态弹窗,确认 `ConfirmOverlay`
  每次都显示正确的文案(没有"上一个弹窗的内容还留在窗口里"的残留)。
- 拖动/resize 主窗口时任意打开一个弹窗,确认跟随。
- 应用退出(`CloseRequested`)时若某个弹窗开着,确认不会遗留孤儿窗口/
  崩溃。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/dialog.rs crates/dozer-app/src/app/app.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "chore(overlay): finish modal-card dialog migration, drop dead Stack+scrim path"
```

---

## Self-Review Notes

（写计划人自查记录,不是执行者要做的事——列在这里方便审阅者核对。这是
真实自查后修正过的版本,不是初稿的自我感觉良好总结。）

- **占位符扫描,发现并修正了 4 处真实违规**:初稿里 Task 10("结构同
  Task 8 模板,关键差异…")、Task 11("结构同 `SettingsOverlay`…")、
  Task 12("结构与 Task 11 完全同构")、Task 14("结构同 Task 11 模板")
  都只写了文字描述 + 差异点,没有重复代码——正是"No Placeholders"一节
  明确禁止的"similar to Task N"写法。已经逐个补全整份宿主代码(Task 10
  的 `ProjectScaffoldOverlay`、Task 11 的 `DatabaseSourceOverlay`、
  Task 12 的 `SshHostOverlay`、Task 14 的 `TodoDetailOverlay` 均为完整
  可读的独立代码块,不是对 Task 8/11 的引用)。
- **发现并修正了一处事实错误**:初稿 Task 13(Project 删除确认)标题写
  "需要 IME",沿用的是 spec 盘点阶段"输入项目名确认"这个未经验证的
  描述。写计划阶段实际读了 `project_delete_confirm_popup`
  (`extensions/project/view.rs:592-`)完整函数体,确认只有
  `radio_row` 三选一单选,没有任何 `text_input`——改用 Task 8 那种更
  简单的无 IME 模板,并回头更正了 spec 文档「现状盘点」与「七个定制
  宿主」两处表格里同样的错误描述(spec 是活文档,发现引用它的计划有
  更准确的信息时应该回头修正,不是留着两份文档互相矛盾)。
- **发现并修正了一处遗漏的特殊行为**:初稿 spec 没有识别出 Files 移动
  确认(Task 9)需要"不接失焦关闭"这个例外——写计划阶段追查
  `Message::MoveDirBrowse`(`files/view.rs:1303` 的"…"浏览按钮)发现它
  在 `Runner::dispatch`(`window_events.rs:1566`)里同步弹出原生 `rfd`
  目录选择器,与 `ProjectCreateOverlay` 需要同样例外的原因完全一致。
  已回头把这条也补进 spec 文档的定制宿主表格("失焦关闭"新增一列)。
  同时确认该表单其实也需要 IME(新文件名可能是中文)——spec 初稿这一项
  写的是"否",也已更正。
- **发现并消除了一处未验证的假设**:初稿 Task 8 假设存在
  `app.database_state()` 访问器并注明"以实际字段可见性为准"——实际
  `grep` 后确认 `app.database` 就是 `App` 结构体上的普通
  `pub(crate)` 字段(`app/app.rs:450`),不需要任何访问器,已在 Task 8/
  Task 11 两处改成直接 `&app.database`,不留悬而未决的实现细节。
- **Spec 覆盖**:spec「架构」1/2/3 节 → Task 1-2(通用宿主打底)、
  Task 3-7(5 个 confirm 消费方)、Task 8-14(7 个定制宿主)全部对应;
  spec「排期备注」四阶段 → 与本计划任务顺序一一对应(含上面提到的
  Files 移动确认顺序调整);spec「测试策略」的四类测试
  (`*_confirm_spec` 单测、互斥单测、`Clone` 派生、人工验证清单)→
  分别落在 Task 1 Step 3、Task 2 Step 11/Task 3 Step 5、Task 1 Step 1、
  每个任务的"人工验证"步骤 + Task 15 Step 4 汇总清单。
- **类型一致性**:`ConfirmTrigger`(Task 2)在 Task 3-7 里被同一个
  `desired_confirm` 函数使用,变体名(`FilesDelete`/`DatabaseDelete`/
  `SshDelete`/`AgentTabClose`/`TodoClear`)贯穿一致;`OverlayKind` 新增
  变体名在「架构」小节与各任务「Files」小节的引用一致
  (`Confirm`/`FilesMove`/`ProjectDelete`/`ProjectScaffold`/
  `DatabaseSource`/`DatabaseDrivers`/`SshHost`/`TodoDetail`)。
- **Review Focus 覆盖**:互斥→每个任务的"人工验证"都含一条跨弹窗互斥
  检查,Task 15 Step 4 汇总;webview 遮挡→ Task 3 Step 6 第 5 条起,
  每个任务人工验证清单都包含,Task 15 Step 4 汇总;窗口跟随→ 各任务
  `reposition` 实现 + Task 15 Step 4;连续开关→ Task 3 Step 6 第 6 条 +
  Task 15 Step 4"连续快速切换"一条;IME/原生粘贴→ Task 11/12/13/14
  各自人工验证清单(Task 13 最终确认不适用,已相应更正)。
- **仍然留给实现阶段核实、但已给出明确验证命令的细节**:Task 8 Step 1
  里 `drivers_popup` 最外层容器改 `Length::Fill` 的具体位置("以函数体
  实际最外层容器为准")、Task 2 Step 11 的互斥单测若现有测试没有可复用
  的 `Runner::Ready` 构造辅助时延后到 Task 3 补——这两处是"要读当时的
  实际代码才能落笔的具体行号/写法",不是可以想清楚就写死的设计决策,
  留給实现阶段是恰当的,不属于占位符。
