# 统一消息 Toast Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 Dozer 加一个全局、不占布局、自动消失的瞬时消息 Toast,并把第一批"一次性事件"提示迁移过去。

**Architecture:** 两层。`extensions/toast.rs` 是纯逻辑的 `ToastCenter`(去重/堆叠上限/按级别到期,时间由调用方注入,可单测),状态挂 `App.toast`,经 `Message::Toast` 触发。渲染宿主 `platform/toast_overlay.rs` 是**只有 Toast 堆叠大小、点击穿透、不抢焦点**的独立原生子窗口,天然叠在 webview 之上,不写任何 `preview_desired` 隐藏逻辑;生命周期由 `Runner::sync_toast_overlay` 在 `about_to_wait` 里按 `App.toast` 单向驱动。

**Tech Stack:** Rust、iced 0.14(`iced_widget`/`iced_renderer`)、winit 0.30.13、wgpu(经现有 `OverlayGpu`)。

**Spec:** `docs/superpowers/specs/2026-09-30-unified-toast-design.md`

**范围:** spec 的阶段 1 + 阶段 2(第一批迁移)。阶段 3(`daemon_error` 拆持久状态徽标)不在本计划内,单独评审。

## Global Constraints

- **分支**:必须在独立 worktree/分支 `feat/unified-toast` 上做,完成后审阅再合并;dispatch 子代理时**必须带完整 worktree 绝对路径前缀**(历史上多次漂移回 main)。worktree 从 `HEAD` 建,不带主 checkout 里别的会话的未提交改动。
- 字体:Toast 是非代码场景,一律 `Font::default()`(不写 `.font(..)`);文本一律 `Shaping::Advanced`(中文回退),不得用 `Basic`。
- 主题色只用 `byteui::theme::color::current()` 的 token(`cyan`/`green`/`gold`/`red`/`cream`/`panel`),不硬编码色值。
- **新浮层走独立原生窗口**;不得把 Toast 加进 `OverlayKind`/`close_other_overlays`(Toast 与任何模态并存,互不关闭)。
- extension 间只能消息通信:各 extension 不得直接持有/调用 `ToastCenter`;只有 `App`(内核)能 `push_toast`。
- 不引入 Swift/AppKit 专属能力;本计划只用 winit 已有 API(`with_active(false)`、`set_cursor_hittest(false)`)。
- 新增函数参数 ≥7 个且有同类型相邻时用具名参数结构体(本计划的 `ToastOverlay::open` 参数已控制在此阈值内,靠复用 `open_child_window_unfocused`)。
- 每个 Task 结束时 `cargo build -p dozer-app` 与 `cargo test -p dozer-app` 必须通过、`cargo clippy -p dozer-app --all-targets` 无新增告警、`cargo fmt` 干净。
- 提交信息末尾加:`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。commit 前先 `git status`/`git diff --cached` 看 staged 全貌(历史上并发会话的暂存区会被卷入)。

## 相对 spec 的有意偏差(Task 6 会同步回写 spec)

1. **v1 Toast 整窗点击穿透(`set_cursor_hittest(false)`),不响应悬停/点击。** 原因:macOS 上点击一个 `canBecomeKeyWindow == true` 的子窗口会让它成为 key window,抢走终端键盘焦点(winit 0.30.13 的 `WinitWindow::canBecomeKeyWindow` 恒为 `true`,无法在不引入 AppKit 专属代码的前提下改)。穿透后 Toast 在任何情况下都不可能拿到焦点,spec 列为"最大不确定项"的风险就此消除。代价:没有"悬停暂停"和"点击关闭",所有级别都自动到期(包括 Error 8s)。这两个交互留给后续。
2. **固定单条几何**(宽 380、高 56 逻辑像素,文本超出裁剪),不做按文本换行量高。窗口尺寸因此只取决于条数,纯函数可测。
3. **第一批迁移点去掉 `files/update.rs` 外部应用打开失败**:`files::update` 只有 files 自己的 `emit`,没有通往 App 级的通道,接入需要单独的设计决定。留作后续。

## Review Focus

- 主窗口被缩得比 Toast 还窄/矮 → 窗口尺寸必须钳到主窗口内且 ≥1,绝不能出现 0 或负尺寸导致 wgpu surface 配置 panic。(Task 3 `stack_bounds` 测试)
- 文本为空/纯空白 → 不产生空白 Toast;文本含换行/连续空白 → 折叠成单行,不撑坏固定几何。(Task 1 测试)
- 同一故障高频重复触发(如循环重试失败,同 key) → 只刷新计时,不新增条目、不无限增长、不反复开关窗口。(Task 1 去重测试 + Task 3 `stamps` 比较)
- Toast 出现时终端里正在打字 → 键盘焦点与输入不被打断。(Task 3 手工验收 A)
- 预览 webview 打开、或设置/确认框等模态弹窗打开时 → Toast 仍可见且不被遮罩吞掉。(Task 3 手工验收 B/C)
- 最后一条 Toast 到期 → 窗口被销毁,不留下空的透明子窗口挡住点击。(Task 3 手工验收 D;窗口本身穿透,即使残留也不挡点击,但仍要销毁)

---

### Task 1: `ToastCenter` 纯逻辑

**Files:**
- Create: `crates/dozer-app/src/extensions/toast.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod toast;`,按字母序放在 `todo` 与 `usage` 之间)

**Interfaces:**
- Consumes: 无。
- Produces(后续 Task 依赖的精确签名):
  - `pub const MAX_VISIBLE: usize = 3`
  - `pub enum Level { Info, Success, Warning, Error }`(`Debug, Clone, Copy, PartialEq, Eq`),`Level::duration(self) -> Duration`
  - `pub struct Toast { pub id: u64, pub level: Level, pub text: String, pub key: Option<String>, pub expires: Instant }`(`Debug, Clone, PartialEq`)
  - `pub struct ToastCenter`(`Debug, Default`):
    - `push(&mut self, level: Level, text: &str, key: Option<String>, now: Instant) -> Option<u64>`
    - `expire(&mut self, now: Instant) -> bool`(返回是否有条目被移除)
    - `next_wake(&self, now: Instant) -> Option<Duration>`
    - `items(&self) -> &[Toast]`
  - `pub enum Message { Push { level: Level, text: String, key: Option<String> } }`(`Debug, Clone`)
  - `pub fn update(state: &mut ToastCenter, msg: Message, now: Instant)`

- [ ] **Step 1: 建 worktree 并确认基线**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-toast -b feat/unified-toast HEAD
cd ../dozer-toast
cargo build -p dozer-app 2>&1 | tail -5
```

Expected: `Finished`。若 HEAD 本身就编译失败,停下来向用户汇报,不要继续。后续所有命令都在 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-toast` 下执行。

- [ ] **Step 2: 写失败的测试(同时建文件骨架)**

创建 `crates/dozer-app/src/extensions/toast.rs`,先只放类型骨架 + 测试(函数体用 `todo!()` 会让测试 panic 而不是编译失败,这里直接先写测试模块并让实现缺失,以"编译失败"作为红):

```rust
//! 统一消息 Toast 的纯逻辑层:去重、堆叠上限、按级别到期。时间由调用方
//! 注入(`now`),不在内部取 `Instant::now()`,便于单测。App 级状态(挂
//! `App.toast`,不挂 `Workspace`——跨所有项目页签共享)。设计见
//! `docs/superpowers/specs/2026-09-30-unified-toast-design.md`。

use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;

    fn center() -> (ToastCenter, Instant) {
        (ToastCenter::default(), Instant::now())
    }

    #[test]
    fn level_durations_follow_spec() {
        assert_eq!(Level::Info.duration(), Duration::from_secs(3));
        assert_eq!(Level::Success.duration(), Duration::from_secs(3));
        assert_eq!(Level::Warning.duration(), Duration::from_secs(5));
        assert_eq!(Level::Error.duration(), Duration::from_secs(8));
    }

    #[test]
    fn push_assigns_increasing_ids_and_expiry() {
        let (mut c, now) = center();
        let a = c.push(Level::Info, "a", None, now).unwrap();
        let b = c.push(Level::Error, "b", None, now).unwrap();
        assert!(b > a);
        assert_eq!(c.items().len(), 2);
        assert_eq!(c.items()[0].expires, now + Duration::from_secs(3));
        assert_eq!(c.items()[1].expires, now + Duration::from_secs(8));
    }

    #[test]
    fn empty_or_whitespace_text_is_ignored() {
        let (mut c, now) = center();
        assert_eq!(c.push(Level::Info, "", None, now), None);
        assert_eq!(c.push(Level::Info, "  \n\t ", None, now), None);
        assert!(c.items().is_empty());
    }

    #[test]
    fn newlines_and_runs_of_whitespace_collapse_to_single_spaces() {
        let (mut c, now) = center();
        c.push(Level::Error, "第一行\n  第二行\t\t第三行", None, now);
        assert_eq!(c.items()[0].text, "第一行 第二行 第三行");
    }

    #[test]
    fn same_key_refreshes_instead_of_adding() {
        let (mut c, now) = center();
        let a = c.push(Level::Error, "old", Some("k".into()), now).unwrap();
        let later = now + Duration::from_secs(2);
        let b = c
            .push(Level::Warning, "new", Some("k".into()), later)
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].text, "new");
        assert_eq!(c.items()[0].level, Level::Warning);
        assert_eq!(c.items()[0].expires, later + Duration::from_secs(5));
    }

    #[test]
    fn without_key_same_level_and_text_dedups_but_different_level_does_not() {
        let (mut c, now) = center();
        c.push(Level::Error, "x", None, now);
        c.push(Level::Error, "x", None, now);
        assert_eq!(c.items().len(), 1);
        c.push(Level::Warning, "x", None, now);
        assert_eq!(c.items().len(), 2);
    }

    #[test]
    fn key_none_and_key_some_never_dedup_against_each_other() {
        let (mut c, now) = center();
        c.push(Level::Error, "x", Some("k".into()), now);
        c.push(Level::Error, "x", None, now);
        assert_eq!(c.items().len(), 2);
    }

    #[test]
    fn overflow_evicts_the_oldest() {
        let (mut c, now) = center();
        for t in ["1", "2", "3", "4"] {
            c.push(Level::Info, t, None, now);
        }
        let texts: Vec<_> = c.items().iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["2", "3", "4"]);
        assert_eq!(c.items().len(), MAX_VISIBLE);
    }

    #[test]
    fn expire_removes_due_items_and_reports_change() {
        let (mut c, now) = center();
        c.push(Level::Info, "short", None, now);
        c.push(Level::Error, "long", None, now);
        assert!(!c.expire(now + Duration::from_secs(2)));
        // 恰好到点(expires == now)也要移除,否则 next_wake 会返回 ZERO 造成空转。
        assert!(c.expire(now + Duration::from_secs(3)));
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].text, "long");
        assert!(c.expire(now + Duration::from_secs(8)));
        assert!(c.items().is_empty());
        assert!(!c.expire(now + Duration::from_secs(60)));
    }

    #[test]
    fn next_wake_is_time_to_earliest_expiry_or_none() {
        let (mut c, now) = center();
        assert_eq!(c.next_wake(now), None);
        c.push(Level::Error, "e", None, now);
        c.push(Level::Info, "i", None, now);
        assert_eq!(c.next_wake(now), Some(Duration::from_secs(3)));
        assert_eq!(
            c.next_wake(now + Duration::from_secs(1)),
            Some(Duration::from_secs(2))
        );
        // 已过期但尚未 expire():饱和为 0,而不是下溢。
        assert_eq!(
            c.next_wake(now + Duration::from_secs(10)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn update_push_message_delegates_to_push() {
        let (mut c, now) = center();
        update(
            &mut c,
            Message::Push {
                level: Level::Success,
                text: "ok".into(),
                key: None,
            },
            now,
        );
        assert_eq!(c.items().len(), 1);
        assert_eq!(c.items()[0].level, Level::Success);
    }
}
```

在 `extensions.rs` 加 `pub mod toast;`(`todo` 与 `usage` 之间)。

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p dozer-app toast:: 2>&1 | tail -15`
Expected: 编译失败,`cannot find type ToastCenter`/`Level`/`Message`/`update` 等。

- [ ] **Step 4: 写最小实现**

在 `toast.rs` 的 `use` 与 `#[cfg(test)]` 之间插入:

```rust
/// 同屏最多显示的 Toast 条数;超出时挤掉最旧的(瞬时消息过期即无意义,不排队)。
pub const MAX_VISIBLE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    /// 自动消失时长。v1 不支持悬停暂停/手动关闭(见计划「有意偏差」1),
    /// 所有级别都会到期,Error 给最长。
    pub fn duration(self) -> Duration {
        match self {
            Level::Info | Level::Success => Duration::from_secs(3),
            Level::Warning => Duration::from_secs(5),
            Level::Error => Duration::from_secs(8),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    /// 单调递增,窗口宿主用它判断"堆叠是否变了"。
    pub id: u64,
    pub level: Level,
    pub text: String,
    /// 去重键:同 key 再推 = 刷新文本/级别/计时,不新增。
    pub key: Option<String>,
    pub expires: Instant,
}

#[derive(Debug, Default)]
pub struct ToastCenter {
    items: Vec<Toast>,
    next_id: u64,
}

/// 顶层 `Message::Toast(toast::Message::..)` 的载荷。extension 想弹 Toast
/// 时向内核发这条消息,不直接持有 `ToastCenter`。
#[derive(Debug, Clone)]
pub enum Message {
    Push {
        level: Level,
        text: String,
        key: Option<String>,
    },
}

pub fn update(state: &mut ToastCenter, msg: Message, now: Instant) {
    match msg {
        Message::Push { level, text, key } => {
            state.push(level, &text, key, now);
        }
    }
}

/// 折叠所有空白(含换行)成单个空格并去首尾:Toast 是固定高度的单/双行卡片,
/// 多行文本会撑坏几何。
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl ToastCenter {
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// 推一条。返回条目 id;文本归一化后为空则忽略返回 `None`。
    pub fn push(
        &mut self,
        level: Level,
        text: &str,
        key: Option<String>,
        now: Instant,
    ) -> Option<u64> {
        let text = normalize(text);
        if text.is_empty() {
            return None;
        }
        let expires = now + level.duration();
        let existing = self.items.iter_mut().find(|t| match (&key, &t.key) {
            (Some(k), Some(tk)) => k == tk,
            (None, None) => t.level == level && t.text == text,
            _ => false,
        });
        if let Some(t) = existing {
            t.level = level;
            t.text = text;
            t.expires = expires;
            return Some(t.id);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Toast {
            id,
            level,
            text,
            key,
            expires,
        });
        if self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
        Some(id)
    }

    /// 移除到期条目(`expires <= now`)。返回是否有变化。
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.expires > now);
        self.items.len() != before
    }

    /// 距最近一条到期的剩余时间;没有条目返回 `None`(主循环不再空转)。
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        self.items
            .iter()
            .map(|t| t.expires)
            .min()
            .map(|e| e.saturating_duration_since(now))
    }
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p dozer-app toast:: 2>&1 | tail -15`
Expected: `11 passed`(上面共 11 个 `#[test]`)。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app/src/extensions/toast.rs crates/dozer-app/src/extensions.rs
git diff --cached --stat
git commit -m "feat(toast): add ToastCenter pure logic (dedup, stack cap, expiry)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: App 接入(状态、消息、到期唤醒)

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`(加 `toast` 字段、初始化、4 个方法)
- Modify: `crates/dozer-app/src/app/message.rs`(加 `Toast(toast::Message)` 变体 + import)
- Modify: `crates/dozer-app/src/app/update.rs`(加 `Message::Toast` 处理分支)
- Modify: `crates/dozer-app/src/platform/window_events.rs`(`new_events` 推进到期、`about_to_wait` 唤醒数组 6→7)

**Interfaces:**
- Consumes: Task 1 的 `ToastCenter`/`Level`/`Message`/`update`。
- Produces:
  - `App.toast: toast::ToastCenter`(`pub(crate)`)
  - `App::push_toast(&mut self, level: toast::Level, text: impl AsRef<str>)`
  - `App::push_toast_keyed(&mut self, level: toast::Level, text: impl AsRef<str>, key: &str)`
  - `App::next_toast_wake(&self) -> Option<Duration>`
  - `App::advance_toasts(&mut self) -> bool`
  - `Message::Toast(toast::Message)`

- [ ] **Step 1: `message.rs`**

在 import 的 `extensions::{...}` 列表里加 `toast`(字母序放 `ssh, todo` 之后、`usage` 之前:`ssh, todo, toast, usage`),在 `Message` 枚举里 `Footbar(...)` 变体附近加:

```rust
    /// 统一 Toast 的消息(全局,不 per-project),内核直接处理,见
    /// `extensions::toast`。extension 想弹 Toast 时构造这条消息发给内核。
    Toast(toast::Message),
```

- [ ] **Step 2: `app.rs`**

`use crate::extensions::footbar;` 旁加 `use crate::extensions::toast;`。在 `pub(crate) footbar: footbar::AppState,`(约 app.rs:470)下加:

```rust
    /// 统一 Toast 状态——App 级,跨所有项目页签共享(见 `extensions::toast`)。
    /// 渲染由 `platform::toast_overlay` 的独立原生窗口承担。
    pub(crate) toast: toast::ToastCenter,
```

在构造处 `footbar: footbar::AppState::default(),`(约 app.rs:837)下加 `toast: toast::ToastCenter::default(),`。

在 `impl App` 里 `next_todo_flash_wake` 附近加:

```rust
    /// 推一条 Toast(无去重键:同 level+文本会去重)。
    pub fn push_toast(&mut self, level: toast::Level, text: impl AsRef<str>) {
        self.toast
            .push(level, text.as_ref(), None, std::time::Instant::now());
    }

    /// 推一条带去重键的 Toast:同 key 再推只刷新文本与计时。
    pub fn push_toast_keyed(&mut self, level: toast::Level, text: impl AsRef<str>, key: &str) {
        self.toast.push(
            level,
            text.as_ref(),
            Some(key.to_string()),
            std::time::Instant::now(),
        );
    }

    /// 距最近一条 Toast 到期的剩余时间(`about_to_wait` 据此排精确唤醒,
    /// 无 Toast 返回 `None` 不空转)。
    pub fn next_toast_wake(&self) -> Option<std::time::Duration> {
        self.toast.next_wake(std::time::Instant::now())
    }

    /// 移除到期 Toast(`new_events` 的 `ResumeTimeReached` 调用)。
    pub fn advance_toasts(&mut self) -> bool {
        self.toast.expire(std::time::Instant::now())
    }
```

- [ ] **Step 3: `update.rs`**

确认文件顶部的 `use crate::extensions::{...}` 里有 `toast`(没有就加)。在 `Message::Usage(...)` 分支组附近(约 update.rs:1426)加:

```rust
            Message::Toast(msg) => {
                toast::update(&mut self.toast, msg, std::time::Instant::now());
            }
```

- [ ] **Step 4: `window_events.rs` 主循环接入**

`new_events`:把参数 `_event_loop` 保持不动(本 Task 不需要它),在 `ResumeTimeReached` 块里 `app.advance_drag_hover_expand();` 之后、`window.request_redraw();` 之前加:

```rust
            // 统一 Toast 到期清理:窗口宿主由 `about_to_wait` 里的
            // `sync_toast_overlay` 据 `app.toast` 销毁/收缩。
            app.advance_toasts();
```

`about_to_wait`:在 `let next_drag_expand = ...;` 之后加 `let next_toast = app.next_toast_wake();`,把 `wakes` 的类型改为 `[(bool, Duration); 7]` 并在数组末尾追加:

```rust
                (
                    next_toast.is_some(),
                    next_toast.unwrap_or(Duration::ZERO),
                ),
```

- [ ] **Step 5: 构建与既有测试**

Run: `cargo build -p dozer-app 2>&1 | tail -5 && cargo test -p dozer-app 2>&1 | tail -8`
Expected: `Finished` 与 `test result: ok`(既有测试不受影响;`Message` 枚举新增变体若有穷尽 `match` 会编译报错——按报错位置补 `Message::Toast(_) => ..`,通常 `update` 之外没有穷尽匹配)。

- [ ] **Step 6: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/platform/window_events.rs
git diff --cached --stat
git commit -m "feat(toast): wire ToastCenter into App state, Message and main-loop wake

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Toast 渲染宿主(独立原生窗口)

**Files:**
- Modify: `crates/dozer-app/src/platform/overlay_window.rs`(抽出私有 `build_child_window`,新增 `open_child_window_unfocused`)
- Create: `crates/dozer-app/src/platform/toast_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`(加 `pub mod toast_overlay;`)
- Modify: `crates/dozer-app/src/platform/window_events.rs`(Runner 字段、初始化、`sync_toast_overlay`、事件路由、`Resized`、`CloseRequested`、`about_to_wait` 调 sync)
- Modify: `crates/dozer-app/src/app/app.rs`(临时 `push_demo_toasts_if_requested`,Task 6 删除)

**Interfaces:**
- Consumes: Task 1 `Toast`/`Level`;Task 2 `App.toast`、`App::push_toast`;现有 `OverlayGpu::{open,redraw,reconfigure}`。
- Produces:
  - `overlay_window::open_child_window_unfocused(main_window: &Window, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>, title: &str, el: &ActiveEventLoop) -> Arc<Window>`
  - `toast_overlay::stack_bounds(main_outer_pos: PhysicalPosition<i32>, main_inner_size: PhysicalSize<u32>, scale: f64, count: usize) -> (PhysicalPosition<i32>, PhysicalSize<u32>)`
  - `toast_overlay::ToastOverlay`:`open(main_window: &Arc<Window>, adapter, device, queue, instance, stamps: Vec<Stamp>, el) -> ToastOverlay`、`window_id()`、`request_redraw()`、`stamps() -> &[Stamp]`、`update(&mut self, device, main_outer_pos, main_inner_size, scale, stamps: Vec<Stamp>)`、`reposition(&mut self, device, main_outer_pos, main_inner_size, scale)`、`redraw(&mut self, items: &[Toast])`
  - `toast_overlay::Stamp = (u64, std::time::Instant)`(条目 id + 到期时间;文本刷新会改到期时间,用它就能发现"同 key 刷新"需要重绘)
  - `toast_overlay::stamps_of(items: &[Toast]) -> Vec<Stamp>`

- [ ] **Step 1: 写失败的几何测试**

创建 `crates/dozer-app/src/platform/toast_overlay.rs`,先只放常量、`stack_bounds` 签名占位不写实现,以及测试:

```rust
//! Toast 的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-30-unified-toast-design.md` 与
//! `docs/superpowers/plans/2026-09-30-unified-toast.md`。
//!
//! 与 8 个模态卡片宿主的三处刻意不同:
//! 1. 窗口只等于"当前 Toast 堆叠的包围盒"(右下角),不覆盖整窗、没有遮罩;
//! 2. **整窗点击穿透**(`set_cursor_hittest(false)`)且建窗**不聚焦**
//!    (`with_active(false)`)——macOS 上点击 `canBecomeKeyWindow == true` 的
//!    子窗口会抢走终端键盘焦点,穿透后任何情况下都不可能拿到焦点;
//! 3. 有 Toast 才建窗,清空即销毁,不常驻空窗口。

use winit::dpi::{PhysicalPosition, PhysicalSize};

/// 单条 Toast 的逻辑像素尺寸。固定高度,文本超出裁剪(不按文本换行量高),
/// 所以窗口尺寸只取决于条数,`stack_bounds` 是纯函数。
pub(crate) const TOAST_WIDTH: f32 = 380.0;
pub(crate) const TOAST_HEIGHT: f32 = 56.0;
pub(crate) const TOAST_GAP: f32 = 8.0;
/// 距主窗口右/下边缘的逻辑像素边距。下边距留出 footbar 的位置。
pub(crate) const MARGIN_RIGHT: f32 = 16.0;
pub(crate) const MARGIN_BOTTOM: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_toast_anchors_bottom_right_of_main_window() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
            1.0,
            1,
        );
        assert_eq!(size, PhysicalSize::new(380, 56));
        assert_eq!(pos, PhysicalPosition::new(904, 746));
    }

    #[test]
    fn three_toasts_grow_upward_with_gaps() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
            1.0,
            3,
        );
        // 3*56 + 2*8 = 184
        assert_eq!(size, PhysicalSize::new(380, 184));
        assert_eq!(pos, PhysicalPosition::new(904, 618));
    }

    #[test]
    fn retina_scale_multiplies_everything() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(2400, 1600),
            2.0,
            1,
        );
        assert_eq!(size, PhysicalSize::new(760, 112));
        assert_eq!(pos, PhysicalPosition::new(1708, 1442));
    }

    #[test]
    fn tiny_main_window_clamps_size_and_never_goes_zero_or_negative() {
        let (pos, size) = stack_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(200, 30),
            1.0,
            1,
        );
        assert_eq!(size, PhysicalSize::new(200, 30));
        assert_eq!(pos, PhysicalPosition::new(100, 50));

        let (_, size) = stack_bounds(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(0, 0),
            1.0,
            1,
        );
        assert!(size.width >= 1 && size.height >= 1);
    }

    #[test]
    fn zero_count_is_treated_as_one_so_size_is_never_zero() {
        let (_, size) = stack_bounds(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1200, 800),
            1.0,
            0,
        );
        assert_eq!(size, PhysicalSize::new(380, 56));
    }
}
```

在 `platform/mod.rs` 加 `pub mod toast_overlay;`(字母序)。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-app toast_overlay:: 2>&1 | tail -10`
Expected: 编译失败,`cannot find function stack_bounds`。

- [ ] **Step 3: 实现 `stack_bounds`**

在常量之后、`#[cfg(test)]` 之前:

```rust
/// 主窗口外框位置 + 内区尺寸 + 缩放 + 条数 → Toast 窗口的物理位置与尺寸。
/// 窗口尺寸钳到主窗口内且 ≥1(主窗口被缩得比 Toast 还小也不出 0/负尺寸,
/// 否则 wgpu surface 配置会失败)。`count == 0` 按 1 处理。定位约定与
/// `overlay_window::full_window_overlay_bounds` 一致:以 `outer_position`
/// 为原点、`inner_size` 为范围。
pub(crate) fn stack_bounds(
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
    count: usize,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let n = count.max(1);
    let px = |logical: f32| (f64::from(logical) * scale).round() as i64;
    let main_w = i64::from(main_inner_size.width);
    let main_h = i64::from(main_inner_size.height);
    let w = px(TOAST_WIDTH).min(main_w).max(1);
    let h = px(TOAST_HEIGHT * n as f32 + TOAST_GAP * (n - 1) as f32)
        .min(main_h)
        .max(1);
    let x = i64::from(main_outer_pos.x) + (main_w - w - px(MARGIN_RIGHT)).max(0);
    let y = i64::from(main_outer_pos.y) + (main_h - h - px(MARGIN_BOTTOM)).max(0);
    (
        PhysicalPosition::new(x as i32, y as i32),
        PhysicalSize::new(w as u32, h as u32),
    )
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p dozer-app toast_overlay:: 2>&1 | tail -10`
Expected: `5 passed`。

- [ ] **Step 5: `overlay_window.rs` 抽建窗函数并新增不聚焦版**

把 `open_child_window` 里从 `use winit::raw_window_handle::HasWindowHandle;` 到 `create_window` 的部分抽成私有函数(`active` 控制初始是否聚焦),`open_child_window` 行为**保持不变**(仍 `enable_overlay_mouse_moved_events` + `focus_window`):

```rust
fn build_child_window(
    main_window: &Window,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    title: &str,
    el: &ActiveEventLoop,
    active: bool,
) -> Arc<Window> {
    use winit::raw_window_handle::HasWindowHandle;

    let parent_handle = main_window
        .window_handle()
        .expect("main window handle")
        .as_raw();
    let attrs = Window::default_attributes()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_active(active)
        .with_position(pos)
        .with_inner_size(size);
    // Safety: `parent_handle` 取自仍存活的主窗口(`Ready` 持有的
    // `Arc<Window>`),本函数返回前主窗口不会被 drop。
    let attrs = unsafe { attrs.with_parent_window(Some(parent_handle)) };
    Arc::new(el.create_window(attrs).expect("create overlay window"))
}

pub(crate) fn open_child_window(
    main_window: &Window,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    title: &str,
    el: &ActiveEventLoop,
) -> Arc<Window> {
    let window = build_child_window(main_window, pos, size, title, el, true);
    enable_overlay_mouse_moved_events(&window);
    window.focus_window();
    window
}

/// 非模态提示窗口(Toast)用:建窗**不聚焦**(winit macOS 上 `with_active(false)`
/// 走 `orderFront` 而非 `makeKeyAndOrderFront`)且整窗**点击穿透**——点击
/// 落到下面的主窗口,这扇窗口永远不会成为 key window,不会抢终端键盘焦点。
/// 不调 `enable_overlay_mouse_moved_events`(穿透窗口收不到鼠标事件)。
pub(crate) fn open_child_window_unfocused(
    main_window: &Window,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    title: &str,
    el: &ActiveEventLoop,
) -> Arc<Window> {
    let window = build_child_window(main_window, pos, size, title, el, false);
    let _ = window.set_cursor_hittest(false);
    window
}
```

保留原 `open_child_window` 上方那段"去掉 `WindowLevel::AlwaysOnTop`"的文档注释(挪到 `open_child_window` 上不变)。

Run: `cargo build -p dozer-app 2>&1 | tail -5`
Expected: `Finished`(`open_child_window_unfocused` 暂未被调用会有 dead_code 警告,Step 7 后消失)。

- [ ] **Step 6: 实现 `toast_overlay.rs` 宿主与视图**

在 `stack_bounds` 之后追加(`use` 放文件顶部):

```rust
use std::sync::Arc;
use std::time::Instant;

use iced_wgpu::wgpu;
use iced_widget::core::{Alignment, Border, Element, Length, Padding};
use iced_widget::{column, container, row, text};
use iced_winit::core::mouse;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::extensions::toast::{Level, Toast};
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::open_child_window_unfocused;

/// 堆叠指纹:条目 id + 到期时间。同 key 刷新会改到期时间(文本可能同时变),
/// 用它比较就能发现"内容变了要重绘",而只比 id 会漏。
pub(crate) type Stamp = (u64, Instant);

pub(crate) fn stamps_of(items: &[Toast]) -> Vec<Stamp> {
    items.iter().map(|t| (t.id, t.expires)).collect()
}

/// 纯视图:一列固定尺寸卡片(左侧色条 + 文本)。窗口背景透明,圆角与边框
/// 由 iced 自己画。文本用系统默认字体 + `Shaping::Advanced`(中文回退)。
pub(crate) fn view(items: &[Toast]) -> Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let mut col = column![]
        .spacing(TOAST_GAP)
        .width(Length::Fixed(TOAST_WIDTH));
    for t in items {
        let accent = match t.level {
            Level::Info => colors.cyan,
            Level::Success => colors.green,
            Level::Warning => colors.gold,
            Level::Error => colors.red,
        };
        let bar = container(iced_widget::space::horizontal())
            .width(Length::Fixed(4.0))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(accent.into()),
                ..container::Style::default()
            });
        let body = container(
            text(t.text.as_str())
                .size(byteui::theme::font::body())
                .color(colors.cream)
                .shaping(text::Shaping::Advanced),
        )
        .padding(Padding {
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
            left: 12.0,
        })
        .width(Length::Fill)
        .align_y(Alignment::Center);
        let panel = colors.panel;
        col = col.push(
            container(row![bar, body])
                .width(Length::Fixed(TOAST_WIDTH))
                .height(Length::Fixed(TOAST_HEIGHT))
                .clip(true)
                .style(move |_t: &iced_widget::Theme| container::Style {
                    background: Some(panel.into()),
                    border: Border {
                        color: accent,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                }),
        );
    }
    col.into()
}

pub(crate) struct ToastOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    stamps: Vec<Stamp>,
}

impl ToastOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn stamps(&self) -> &[Stamp] {
        &self.stamps
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        stamps: Vec<Stamp>,
        el: &ActiveEventLoop,
    ) -> ToastOverlay {
        let scale = main_window.scale_factor();
        let (pos, size) = stack_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            stamps.len(),
        );
        let window = open_child_window_unfocused(main_window, pos, size, "toast", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        ToastOverlay { window, gpu, stamps }
    }

    /// 堆叠变了:换指纹并按新条数重定位/缩放窗口。
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        stamps: Vec<Stamp>,
    ) {
        self.stamps = stamps;
        self.reposition(device, main_outer_pos, main_inner_size, scale);
    }

    /// 主窗口 resize 后重新贴右下角(条数不变)。
    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let (pos, size) = stack_bounds(main_outer_pos, main_inner_size, scale, self.stamps.len());
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, items: &[Toast]) {
        self.gpu
            .redraw(&self.window, mouse::Cursor::Unavailable, view(items));
    }
}
```

Run: `cargo build -p dozer-app 2>&1 | tail -20`
Expected: 若 iced 0.14 的个别 API 名(如 `.clip(true)`、`Padding` 字段、`text::Shaping` 路径)与上面不符,**只按编译器提示调整这些 API 名**,不改布局意图。`Finished` 之前 `ToastOverlay` 的 dead_code 警告可忽略。

- [ ] **Step 7: `window_events.rs` 接入 Runner**

1. `Runner::Ready` 变体的字段声明里(`confirm_overlay` 约 window_events.rs:233 附近)加:

```rust
        /// Toast 的独立原生窗口。**不参与** `OverlayKind`/`close_other_overlays`
        /// (非模态,与任何模态弹窗并存)。生命周期由 `sync_toast_overlay` 按
        /// `app.toast` 单向驱动:有 Toast 才建窗,清空即销毁。
        toast_overlay: Option<toast_overlay::ToastOverlay>,
```

并 `use crate::platform::toast_overlay;`(与 `confirm_overlay` 的 use 并列)。

2. 构造 `Ready` 处(约 window_events.rs:2923,`confirm_overlay: None,` 旁)加 `toast_overlay: None,`。

3. 新增方法(放在 `sync_confirm_overlay` 之后):

```rust
    /// 按 `app.toast` 驱动 Toast 窗口:空 → 销毁;有 → 开窗或(指纹变了时)
    /// 重定位并重绘。指纹见 `toast_overlay::Stamp`。`about_to_wait` 每轮
    /// 调一次,所以 overlay 窗口自己事件里派发出的 Toast 也会在同一轮之后
    /// 显示,不依赖某个特定的 dispatch 尾部。
    fn sync_toast_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let Self::Ready {
            app,
            window,
            instance,
            adapter,
            device,
            queue,
            toast_overlay,
            ..
        } = self
        else {
            return;
        };
        let stamps = toast_overlay::stamps_of(app.toast.items());
        if stamps.is_empty() {
            *toast_overlay = None;
            return;
        }
        let outer = window
            .outer_position()
            .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0));
        let inner = window.inner_size();
        let scale = window.scale_factor();
        match toast_overlay {
            Some(o) if o.stamps() == stamps.as_slice() => {}
            Some(o) => {
                o.update(device, outer, inner, scale, stamps);
                o.request_redraw();
            }
            None => {
                let o = toast_overlay::ToastOverlay::open(
                    window, adapter, device, queue, instance, stamps, el,
                );
                o.request_redraw();
                *toast_overlay = Some(o);
            }
        }
    }
```

4. `about_to_wait` 开头(在 `if let Self::Ready { app, .. } = self {` 之前)加 `self.sync_toast_overlay(event_loop);`。

5. 窗口事件路由:在 confirm overlay 的早退分支之前加(同 edit_history 的手法):

```rust
        // Toast 窗口自己那份 `WindowId` 的事件:整窗点击穿透、不聚焦,只需要
        // 响应重绘;其余事件(包括合成的 Focused)一律忽略。
        if let Self::Ready {
            app, toast_overlay, ..
        } = self
            && let Some(overlay) = toast_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app.toast.items());
            }
            return;
        }
```

6. 主窗口 `WindowEvent::Resized`(约 window_events.rs:4254)所在的字段解构列表里加 `toast_overlay,`,并在其他 overlay 的 `reposition` 之后加:

```rust
                    if let Some(overlay) = toast_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                        );
                    }
```

7. 主窗口 `CloseRequested` 分支的 overlay 清空列表里加 `*toast_overlay = None; // 图干净,Drop 本身就会释放。`。

- [ ] **Step 8: 临时 demo 触发器(Task 6 会删)**

`app.rs` 的 `impl App` 里加:

```rust
    /// **临时**(Task 6 删除):`DOZER_TOAST_DEMO=1` 时一次推四条不同级别的
    /// Toast,用于手工验收窗口宿主。
    pub fn push_demo_toasts_if_requested(&mut self) {
        if std::env::var_os("DOZER_TOAST_DEMO").is_none() {
            return;
        }
        self.push_toast(toast::Level::Info, "信息:这是一条 Info Toast");
        self.push_toast(toast::Level::Success, "成功:已保存");
        self.push_toast(toast::Level::Warning, "警告:这是一条 Warning Toast");
        self.push_toast(
            toast::Level::Error,
            "错误:打开项目失败,请确认 dozerd 正常后重试——这条文本故意写得很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长,用来验证固定高度下的裁剪",
        );
    }
```

在 `Runner` 从建窗走到 `Self::Ready { app, .. }` 的位置(`toast_overlay: None,` 所在的构造语句紧前面,`app` 刚构造完、尚未 move 进 `Ready`)调用一次 `app.push_demo_toasts_if_requested();`(需要 `let mut app`)。

Run: `cargo build -p dozer-app 2>&1 | tail -5`
Expected: `Finished`,无 dead_code 警告(`open_child_window_unfocused`/`ToastOverlay` 都已被使用)。

- [ ] **Step 9: 手工验收(必须人工,无法自动化)**

Run: `DOZER_TOAST_DEMO=1 cargo run -p dozer-app`

依次核对,**任何一项失败都要停下汇报,不要继续 Task 4**:

- **A 焦点**:打开一个项目,在终端里持续输入字符;启动后 Toast 出现的瞬间及其存在期间,输入不中断、光标仍在终端(不是 Toast 窗口拿走了键盘)。
- **B 叠在 webview 之上**:打开一个含预览 webview 的文件(如 HTML/JSON),再触发 Toast(重启带 demo 变量,或等下次),Toast 完整可见、没被 webview 盖住。
- **C 与模态并存**:打开设置弹窗或任一确认框时 Toast 仍可见,且不会关闭该弹窗。
- **D 到期销毁**:Info/Success 3s、Warning 5s、Error 8s 依次消失;全部消失后窗口被销毁(可用「活动监视器/窗口列表」或点击原 Toast 位置确认点击能落到下面的主窗口——本来就穿透,主要看 Toast 视觉消失)。
- **E 缩放**:拖动缩小主窗口,Toast 始终贴右下角、不越界;缩到比 Toast 还窄时不崩溃。
- **F 长文本**:第四条超长文本被裁剪在固定高度内,不撑破卡片。
- **G 字体/中文**:中文正常渲染(无方块),字体是系统默认。
- **H 上限**:demo 一次推 4 条,只显示最新 3 条(最早的 Info 被挤掉)。

- [ ] **Step 10: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app/src/platform/ crates/dozer-app/src/app/app.rs
git diff --cached --stat
git commit -m "feat(toast): add click-through non-focusing overlay window host

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 迁移 `daemon_error` 的两处一次性事件

**Files:**
- Modify: `crates/dozer-app/src/app/update.rs`(约 :3922 打开项目失败、约 :1411 删除项目未完全成功)
- Modify: `crates/dozer-app/src/term/terminal.rs`(若 worktree 里仍有终端栏渲染 `daemon_error` 的那段则删除)

**Interfaces:**
- Consumes: Task 2 的 `App::push_toast`/`push_toast_keyed`、Task 1 的 `Level`。
- Produces: 无新接口;行为变化:这两处不再写 `daemon_error`。

- [ ] **Step 1: 确认终端栏那处是否已删**

Run: `rg -n "daemon_error" crates/dozer-app/src/term/terminal.rs`
若有命中(说明主 checkout 里的最小修复尚未提交、worktree 不带它),删除 `terminal_pane` 里整段:

```rust
    if let Some(err) = &app.daemon_error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }
```

若无命中则跳过。合并时这一段与主 checkout 的同一改动相同,冲突可直接取任一侧。

- [ ] **Step 2: 打开项目失败改推 Toast**

`project_tab_opened` 里(约 update.rs:3922)把

```rust
            self.daemon_error = Some("打开项目失败,请确认 dozerd 正常后重试".to_string());
```

替换为

```rust
            self.push_toast_keyed(
                toast::Level::Error,
                "打开项目失败,请确认 dozerd 正常后重试",
                "open-project-failed",
            );
```

同时更新其上方注释:失败文案现在走 Toast(不依赖任何 `Workspace`,一个项目都没打开时 Toast 也显示),不再挂 `daemon_error`。**保留** 下方成功路径的 `self.daemon_error = None;`(它仍用于清除"dozerd 已停止"这类持久状态)。

- [ ] **Step 3: 删除项目未完全成功改推 Toast**

`Message::ProjectDeleteDone` 分支里(约 update.rs:1411)把

```rust
                    self.daemon_error = Some(format!("删除项目未完全成功: {}", errors.join("; ")));
```

替换为

```rust
                    self.push_toast(
                        toast::Level::Warning,
                        format!("删除项目未完全成功: {}", errors.join("; ")),
                    );
```

读一遍该分支其余代码,确认没有别处依赖"errors 非空 ⇒ daemon_error 已被写"。

- [ ] **Step 4: 构建、测试、回归确认**

Run: `cargo build -p dozer-app 2>&1 | tail -5 && cargo test -p dozer-app 2>&1 | tail -8`
Expected: 通过。再 `rg -n "daemon_error" crates/dozer-app/src/app/update.rs` 确认剩余写入点只有:`Message::DaemonError`、`project_tab_opened` 成功路径的 `= None`、dozerd 停止/重启(`"dozerd 已停止…"`/`None`)——这些是持久状态,保持不动。

- [ ] **Step 5: 手工验收**

停掉 dozerd(或用无效环境)后在 App 里点顶栏 ＋ 打开项目:右下角出现红色 Toast"打开项目失败,请确认 dozerd 正常后重试",8s 后消失;连点两次只刷新不叠两条;终端栏没有多出任何一行、终端网格高度不变。

- [ ] **Step 6: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/app/update.rs crates/dozer-app/src/term/terminal.rs
git diff --cached --stat
git commit -m "refactor(toast): report open-project/delete-project failures via Toast

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: 迁移 `agent_context` 的 `notice`

**Files:**
- Modify: `crates/dozer-app/src/extensions/agent_context.rs`(`notice()`→`take_notice()`,删 `DismissNotice` 与视图里的条,改测试)
- Modify: `crates/dozer-app/src/app/update.rs`(`agent_context_message` 的 `other` 分支,约 :5303)

**Interfaces:**
- Consumes: Task 2 `App::push_toast`。
- Produces: `agent_context::State::take_notice(&mut self) -> Option<String>`(取代 `notice(&self) -> Option<&str>`)。`Message::DismissNotice` 被删除。

`apply` 仍然把失败原因写进 `state.notice`(它是可单测的纯状态转移);变化是"App 在每次处理完消息后把 notice 取走并转成 Toast",不再由视图画一条红字。extension 没有直接依赖 Toast(保持"extension 间只能消息通信"),级别统一按 `Error`(包括"已发送到终端,但未能记录"这条;要区分级别需要给 notice 带级别,留作后续)。

- [ ] **Step 1: 改失败测试(先红)**

`agent_context.rs` 测试模块里,把所有 `s.notice()` 改为 `s.take_notice()`(`s` 需要 `let mut`),并调整这几个测试:

- `loaded_ok_replaces_items_and_err_sets_notice`:`assert!(s.take_notice().unwrap().contains("boom"));`
- 约 :371 与 :385、:390 处对 `notice` 的断言同理改为 `take_notice()`(逐个检查——同一个测试里不要对同一状态连续调两次 `take_notice()` 去断言两次都有值,第二次会是 `None`;需要多次读的地方先 `let n = s.take_notice();`)。
- `open_messages_are_not_handled_here_and_dismiss_clears_notice`:重命名为 `open_messages_are_not_handled_here_and_leave_notice_untouched`,删掉 `apply(&mut s, Message::DismissNotice)` 及其后 `is_none` 断言;改为在 `Open`/`OpenHistory` 后断言 `take_notice()` 仍为原值。
- 新增:

```rust
    #[test]
    fn take_notice_returns_once_then_none() {
        let mut s = State::default();
        apply(&mut s, Message::Loaded(Err("boom".into())));
        assert!(s.take_notice().unwrap().contains("boom"));
        assert!(s.take_notice().is_none());
    }
```

Run: `cargo test -p dozer-app agent_context:: 2>&1 | tail -10`
Expected: 编译失败(`take_notice` 不存在 / `DismissNotice` 仍在)。

- [ ] **Step 2: 实现**

`agent_context.rs`:
- 把 `pub fn notice(&self) -> Option<&str>` 换成:

```rust
    /// 取走待展示的失败提示(App 每次处理完 `AgentContext` 消息后调用一次,
    /// 转成 Toast)。取走后为 `None`,同一条不会重复弹。
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }
```

- 删掉 `Message::DismissNotice` 变体、`apply` 里对应分支。
- 删掉 `strip` 视图里 `if let Some(n) = state.notice() { bar = bar.push(button(text(n.to_string())...)) }` 那整段;`bar` 不再需要 `mut` 的地方按编译器提示改。
- 该文件里若还有别处调用 `notice()`(`rg -n "notice\(\)" crates/dozer-app/src`),一并按上面替换或删除。

- [ ] **Step 3: `update.rs` 取走并推 Toast**

`agent_context_message` 的 `other =>` 分支改为:

```rust
            other => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::AgentContext(project_id, m));
                };
                let notice = {
                    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                        return;
                    };
                    ctx::update(
                        &mut ws.agent_context,
                        project_id,
                        root,
                        other,
                        &client,
                        &handle,
                        emit,
                    );
                    ws.agent_context.take_notice()
                };
                if let Some(n) = notice {
                    self.push_toast(toast::Level::Error, n);
                }
            }
```

(先把 `ws` 借用限制在块内再 `self.push_toast`,避免可变借用冲突。)

- [ ] **Step 4: 运行测试与构建**

Run: `cargo test -p dozer-app agent_context:: 2>&1 | tail -10 && cargo build -p dozer-app 2>&1 | tail -5`
Expected: 全部通过,无 dead_code 警告(注意最近一次提交 `adf3026f` 专门为消除 agent_context 访问器的 dead_code 改过视图——确认这次没有引入新的死代码)。

- [ ] **Step 5: 手工验收**

让上下文列表加载失败或移除失败(如停掉 dozerd 后在终端下方条里点移除):出现红色 Toast,几秒后消失;终端下方那一条不再出现红字,条的高度不变(`STRIP_HEIGHT` 固定,PTY 网格不重算)。

- [ ] **Step 6: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
git add crates/dozer-app/src/extensions/agent_context.rs crates/dozer-app/src/app/update.rs
git diff --cached --stat
git commit -m "refactor(toast): report agent-context failures via Toast instead of the strip notice

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 收尾(删 demo、文档、总验证)

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`、`crates/dozer-app/src/platform/window_events.rs`(删临时 demo)
- Modify: `CLAUDE.md`(关键裁决新增一条)
- Modify: `docs/superpowers/specs/2026-09-30-unified-toast-design.md`(回写偏差)

**Interfaces:** Consumes 全部前序 Task。Produces:无。

- [ ] **Step 1: 删临时 demo**

删除 `App::push_demo_toasts_if_requested` 及 `window_events.rs` 里对它的调用。`rg -n "DOZER_TOAST_DEMO|push_demo_toasts" crates/` 应无命中。

- [ ] **Step 2: `CLAUDE.md` 关键裁决新增一条**

在「关键裁决」里"新增/改造 icon 按钮…"那条之后加:

```markdown
- **瞬时消息统一走 Toast**(`extensions/toast.rs` + `platform/toast_overlay.rs`,见 `docs/superpowers/specs/2026-09-30-unified-toast-design.md`):描述"刚刚发生了一件事"的一次性提示(失败/成功/警告)一律 `App::push_toast`/`push_toast_keyed`,不要再给某个面板加 `Option<String>` 的 notice/error 字段 + 自己画一条。描述"当前处于某状态"的持久状态(如"dozerd 已停止"、文件树加载失败,带重试上下文)不进 Toast,留在原位。Toast 窗口是点击穿透、不聚焦的独立原生子窗口——**不要**给它加悬停/点击交互(macOS 上会让它成为 key window 抢终端焦点),要交互先重新评估焦点方案。extension 不得直接持有 `ToastCenter`,只能经内核。
```

- [ ] **Step 3: 回写 spec 偏差**

在 spec 文档顶部状态行改为"已按 `2026-09-30-unified-toast.md` 实施阶段 1-2",并在「架构·层 2」「第一批迁移点」「未决项」里同步:v1 点击穿透且无悬停暂停/点击关闭(所有级别自动到期);固定几何;`files/update.rs` 外部应用打开失败移出第一批(原因:`files::update` 无 App 级通道);`agent_context` 的 notice 一律 Error 级。「风险」里的"macOS 焦点行为"改写为"已通过穿透+不聚焦规避,若日后要加交互需重新评估"。

- [ ] **Step 4: 总验证**

```bash
cargo build 2>&1 | tail -3
cargo test -p dozer-app 2>&1 | tail -5
cargo clippy --all-targets 2>&1 | tail -5
cargo fmt --check && echo fmt-ok
```

Expected: 全部通过。再完整手工过一遍 Task 3 Step 9 的 A–H 中 A/B/C/D(demo 已删,用 Task 4/5 的真实触发路径复验一次 A/B/C/D)。

- [ ] **Step 5: 提交并交接审阅**

```bash
git add -A crates/dozer-app/src CLAUDE.md docs/superpowers/specs/2026-09-30-unified-toast-design.md
git diff --cached --stat
git commit -m "chore(toast): drop demo trigger, document Toast convention, sync spec

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

不要自行合并 `feat/unified-toast`;交给用户审阅后再合并(合并前先看主 checkout 是否有别的会话的未提交改动/并发提交)。
