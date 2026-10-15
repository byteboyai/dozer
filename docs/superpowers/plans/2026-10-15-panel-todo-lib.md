# S3a 把 todo 面板抽成共享库 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Dozer 里的 todo 面板(约 4,300 行 Rust + TSX 前端 + 文案)抽成共享库 `dozer-panel-todo`,并把 `panel_host` 里不依赖 `App` 的部分先抽成共享的 `dozer-panel-host`;Dozer 改用它们,**用户可见行为不变**。

**Architecture:** 两个新 crate,依赖闭包都不含 `dozer-app` / `dozer-client` / `dozer-core`。`dozer-panel-host` 装请求追踪、`PanelIo`、`ActivationCtx`、`HostOutbox` 和一个中性的 `NoticeOutbox`;`dozer-panel-todo` 装 todo 的状态 / 更新 / 协议 / 筛选 / 拖拽 / 视图 / 前端 / 文案,产品相关的东西经 `PanelDisplay` 适配 trait 和几条出口消息注入。Dozer 的 `extensions/todo/mod.rs` 缩成"重新导出 + 注册包装",所以 `app/` 里约 60 处 `todo::…` 调用点基本不动。

**Tech Stack:** Rust 2024 workspace、iced 0.14、byteui `v0.4.3`、bytehost `v0.4.0`、`dozer-model` / `dozer-services` / `dozer-i18n`;前端 Preact + esbuild(`node --test`)。

**Spec:** `docs/superpowers/specs/2026-10-15-panel-todo-lib-design.md`(含 §9 修订)。先例:`crates/dozer-panel-usage`(Task 19 的抽库)。

## Global Constraints

- **用户可见行为不变**:不改任何界面、交互、数据、文案内容、`dozerd` 与 MCP。搬走的测试**不改期望值、不删**;抽库前后测试总数必须相同(Task 0 记基线,Task 9 对比)。
- **新 crate 的依赖闭包不含 `dozer-app` / `dozer-client` / `dozer-core`**(用 `cargo tree -p <crate> | grep -E 'dozer-(app|client|core)'` 为空来验证)。允许:`dozer-model`、`dozer-services`、`dozer-i18n`、`byteui`、`bytehost-core` / `bytehost-desktop`、iced、`serde*`、`tokio`。
- **不升级任何依赖**;`Cargo.toml` 里已有的 git tag / rev 一个字符都不动。
- **不碰 Dozer 主工作区** `../dozer`(别人有未提交改动)。所有改动都在 worktree `../dozer-panel-todo`(分支 `feat/panel-todo-lib`)。
- **不推送、不合入主线**:那是用户的决定(spec §7.4)。
- 可见性规则:`pub(crate)` → `pub` 只改**编译器报告 `dozer-app` 需要**的项,每个改动处不加文档以外的内容;不顺手重命名。
- 注释风格沿用现有代码(中文,说"为什么")。门禁:`scripts/check-i18n.sh`、`scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py` 基线与终态都要通过。
- 目录约定:`ROOT=/Users/chrischiang/Projects/CoralProjects/byteboy`,工作目录 `$ROOT/dozer-panel-todo`。**第一次编译要很久**(新 target 目录);磁盘约需 20GB。

## Review Focus

spec 没细说、但最可能伤到 Dozer 真实使用者的五类问题;每一条落到具体任务的测试里:

1. **请求代数 / 过期结果**:todo 的后台结果靠 `RequestTrackers` 判断"是不是当前这一轮"。抽到新 crate 后,`next_request_owner` 的静态计数器必须仍是进程内**唯一**一份(不能 host crate 和 Dozer 各一份),否则两个面板实例可能拿到相同 owner、互相接受对方的在途结果。→ Task 1 `owners_are_unique_across_trackers`。
2. **提示去重与冷却**:`push_keyed_cooldown` 的语义("轮询每秒失败一次也只提示一次;换了一种错误要再提示;冷却过后再提示")不能在换成中性发件箱时改变。→ Task 2 的 4 条发件箱测试 + `mod.rs` 里原有的 3 条 todo 测试。
3. **悬停键映射**:`Message::Hover(HoverSlot, bool)` 映射回 `HoverId::Panel(PanelKind::Todo, slot)` 时,**行悬停(`Row(id)`)和折叠按钮悬停(`ListCollapse`)必须映射到与以前相同的 `HoverId`**,否则悬停动画的状态表会错位(卡片不再有渐变)。→ Task 4 `hover_slots_map_back_to_the_same_hover_ids`。
4. **原生 webview 让位高度**:`todo_top_row_inner_h_px()` 搬进库后,Dozer 的 `webview_geometry` 与视图必须仍读**同一个**数,否则 todo 内容区的 webview 会压住或离开顶栏。→ Task 5 `top_row_height_has_one_source`。
5. **前端资源的唯一来源**:搬走后 `crates/dozer-app/assets/todo-content` 不能再残留一份副本(Dozer 的开发态、打包态都要读库里的那一份),否则改了前端却看不到变化。→ Task 6 的 `todo_content_bundle_assets_are_present` 与 `assets/todo-content` 不存在的检查。

---

### Task 0: worktree 基线

**Files:** 无改动(基线记在草稿目录,不提交)。

- [ ] **Step 1: 确认 worktree 与起点**

```bash
export ROOT=/Users/chrischiang/Projects/CoralProjects/byteboy
cd $ROOT/dozer-panel-todo && git branch --show-current && git log -1 --format='%h %s' && git status --short | wc -l
git -C $ROOT/dozer status --short | wc -l        # 记作 N:主工作区别人的改动数,之后不得变化
```

Expected: 分支 `feat/panel-todo-lib`;worktree 干净(0);第二行起点之后是 spec 的两个提交。

- [ ] **Step 2: 记录测试基线**

```bash
cd $ROOT/dozer-panel-todo
B="${TMPDIR:-/tmp}/s3a"; mkdir -p "$B"
cargo test -p dozer-app 2>&1 | tee $B/baseline-dozer-app.log | grep -E "^test result"
cargo test -p dozer-panel-usage -p dozer-model -p dozer-services 2>&1 | grep -E "^test result" | tee $B/baseline-libs.log
cargo test -p dozer-app -- extensions::todo 2>&1 | grep -E "^test result" | tee $B/baseline-todo.log
(cd crates/dozer-app/web/todo-content && npm test 2>&1 | tail -15 | tee $B/baseline-web.log)
```

Expected: 各命令的 `test result: ok. N passed; 0 failed`。**把 `dozer-app` 总数、`extensions::todo` 的数、前端的通过数记进 `$B/baseline.txt`**,Task 9 逐项对比。若 `dozer-app` 有既有失败,先在这里记下"基线失败清单",之后只要求不新增。

- [ ] **Step 3: 记录门禁基线**

```bash
bash scripts/check-i18n.sh 2>&1 | tail -2
bash scripts/check-log-scope.sh 2>&1 | tail -1
python3 scripts/audit/check_panel_boundary.py 2>&1 | tail -3
git -C crates/dozer-app/web/todo-content status --short | head -2   # 应为空
cd crates/dozer-app/web/todo-content && npm run build 2>&1 | tail -2 && cd $ROOT/dozer-panel-todo && git status --short | head -3
```

Expected: 三个门禁都通过(记下各自的输出结尾);**前端重新构建后 `git status` 为空**,证明提交的产物与源码一致(这是 Task 6 对比产物的前提;若不为空,记下差异文件,Task 6 用它作为"允许的既有差异")。

---

### Task 1: `dozer-panel-host` —— 抽出不依赖 `App` 的宿主件

**Files:**
- Create: `crates/dozer-panel-host/Cargo.toml`、`src/lib.rs`
- Modify: `crates/dozer-app/Cargo.toml`(加依赖)、`crates/dozer-app/src/panel_host.rs`(改为重新导出 + 留下 `PanelHost` trait 与 `impl for App`)

**Interfaces:**
- Produces(全部 `pub`):`scope_of`、`project_of`、`wrap_token`、`RequestTokenWrapExt`、`next_request_owner`、`new_request_trackers`、`RequestTrackers`、`PanelIo<M>`、`ActivationCtx<M>`、`PanelCommand`、`HostRequest<M>`、`HostOutbox<M>`。**与 `panel_host.rs` 现有定义逐字相同,只改可见性**。

- [ ] **Step 1: 先写守护测试(owner 全进程唯一)**

`crates/dozer-panel-host/src/lib.rs` 末尾(搬过去的代码之后)放:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 评审关注点 1:`next_request_owner` 的计数器是进程内唯一一份,两个 trackers 的 owner 不同,
    /// 所以重开的同 slot 面板互不接受对方的在途结果。
    #[test]
    fn owners_are_unique_across_trackers() {
        let a = RequestTrackers::default();
        let b = RequestTrackers::default();
        let token_a = a.begin(Some(1), "todos");
        let token_b = b.begin(Some(1), "todos");
        assert!(a.accepts(&token_a));
        assert!(!a.accepts(&token_b), "another instance's token is not accepted");
        assert!(!b.accepts(&token_a));
    }

    #[test]
    fn scope_round_trips_the_project_id() {
        assert_eq!(project_of(&scope_of(Some(7))), Some(7));
        assert_eq!(project_of(&scope_of(None)), None, "the global lane has no project");
    }

    #[test]
    fn a_newer_request_in_the_same_lane_invalidates_the_older_one() {
        let t = RequestTrackers::default();
        let old = t.begin(Some(1), "todos");
        let new = t.begin(Some(1), "todos");
        assert!(!t.accepts(&old));
        assert!(t.accepts(&new));
    }
}
```

- [ ] **Step 2: 建 crate,把代码搬过去**

`crates/dozer-panel-host/Cargo.toml`:

```toml
[package]
name = "dozer-panel-host"
version = "0.1.0"
edition.workspace = true
publish = false

# 面板宿主件的共享库:请求追踪、`PanelIo` / `ActivationCtx`、宿主需求发件箱、中性提示发件箱。
# 依赖闭包刻意**不含** dozer-app / dozer-client / dozer-core:Dozer 与 Digger 的共享面板库
# (`dozer-panel-todo` 起)都依赖它,而不是各自复制一份。
[dependencies]
bytehost-core = { git = "https://github.com/byteboyai/bytehost", tag = "v0.4.0" }
dozer-model = { path = "../dozer-model" }
dozer-services = { path = "../dozer-services" }
tokio.workspace = true
```

把 `crates/dozer-app/src/panel_host.rs` 里下面这些**逐字**搬进 `crates/dozer-panel-host/src/lib.rs`(顺序不变),`pub(crate)` 全部改 `pub`,`use` 行换成该 crate 自己的(`bytehost_core::id::ScopeId`、`dozer_services::Services`、`std::*`、`tokio::runtime::Handle`):

- 第 18–135 行:`scope_of`、`project_of`、`wrap_token`、`RequestTokenWrapExt`(及对 `RequestToken` 的实现)、`next_request_owner`、`new_request_trackers`、`RequestTrackers` 及其 `impl`;
- 第 136–221 行附近:`PanelIo<M>` 及其 `impl`(`new` / `services` / `handle` / `emitter` / `spawn` 等);
- 第 222–283 行:`ActivationCtx<M>`、`PanelCommand`、`HostRequest<M>`、`HostOutbox<M>` 及其 `impl`。

**不搬**:`pub use bytehost_desktop::HoverSlot`、`PanelHost` trait、`impl PanelHost for App`、`testing` 模块(用到 `crate::host_adapters`)、以及依赖 `App` 的测试。

- [ ] **Step 3: 运行,确认新 crate 单独能编译、守护测试通过**

```bash
cargo test -p dozer-panel-host 2>&1 | grep -E "^(error|warning)|test result|FAILED"
cargo tree -p dozer-panel-host | grep -E 'dozer-(app|client|core)' || echo "closure clean"
```

Expected: `test result: ok. 3 passed`,没有 warning;`closure clean`。

- [ ] **Step 4: Dozer 改为重新导出**

`crates/dozer-app/Cargo.toml` 在 `dozer-panel-usage = ...` 那行之后加 `dozer-panel-host = { path = "../dozer-panel-host" }`。

`crates/dozer-app/src/panel_host.rs`:删掉 Step 2 搬走的那些定义,在原位置放:

```rust
// 请求追踪 / `PanelIo` / `ActivationCtx` / 宿主需求发件箱已抽到共享库 `dozer-panel-host`
// (todo 起,Digger 与以后的共享面板库共用);这里重新导出,面板和 `App` 的 `crate::panel_host::…` 路径不变。
pub(crate) use dozer_panel_host::{
    ActivationCtx, HostOutbox, HostRequest, PanelCommand, PanelIo, RequestTokenWrapExt, RequestTrackers, new_request_trackers, next_request_owner, project_of, scope_of, wrap_token,
};
```

(`pub(crate) use` 只重导出 `dozer-app` 里实际用到的名字;未用到的名字编译器会报 `unused import`,删掉即可。)

- [ ] **Step 5: 全量回归**

```bash
cargo test -p dozer-app 2>&1 | grep -E "^(error|warning: unused)|^test result|FAILED"
```

Expected: 与 Task 0 基线**测试数相同、全绿**(搬走的守护测试是新增的,在新 crate 里,不计入 `dozer-app`)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-panel-host crates/dozer-app/Cargo.toml crates/dozer-app/src/panel_host.rs Cargo.lock
git commit -m "refactor: extract the App-independent host pieces into dozer-panel-host

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: 中性的提示发件箱 `NoticeOutbox`

**Files:**
- Modify: `crates/dozer-panel-host/src/lib.rs`(加 `notice` 模块)
- Modify: `crates/dozer-app/src/extensions/toast.rs`(加转换,不动现有 `Outbox`)

**Interfaces:**
- Produces: `dozer_panel_host::{NoticeLevel, Notice, NoticeOutbox}`;`NoticeOutbox::{push, push_keyed, push_keyed_cooldown(.., now: Instant), take}`;冷却时长常量 `NOTICE_COOLDOWN = 60s`(与 `toast::LOAD_FAILURE_COOLDOWN` 相同)。
- Consumes: 无。

- [ ] **Step 1: 先写测试(4 条,对应 `toast::Outbox` 现有语义)**

`crates/dozer-panel-host/src/notice.rs`:

```rust
//! 中性的"待发提示"发件箱:面板的 `update` 拿不到 `App`,失败时往自己 state 的发件箱里 `push`,
//! 产品每条消息后统一排空成自己的 Toast。与 Dozer `extensions::toast::Outbox` 同语义,但**不依赖**
//! `dozer_core::log::Scope` 与 Dozer 的 i18n 解析器,所以共享面板库可以用。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dozer_model::LocalizedText;

/// 读失败类提示的冷却:轮询会每秒重试一次,持续性失败若每次都弹,Toast 会一直被续期;
/// 相同 key 下相同文本在这段时间内只提示一次。
pub const NOTICE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    /// 只写日志、不弹 Toast 的诊断(例如 dozerd 不可用时的静默读取失败)。
    Debug,
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: LocalizedText,
    pub key: Option<String>,
}

#[derive(Debug, Default)]
pub struct NoticeOutbox {
    items: Vec<Notice>,
    /// 每个 key 上一次经 `push_keyed_cooldown` 推出的文本与时刻。排空(`take`)不清它,
    /// 否则每帧排空一次就等于没有冷却。比较用 `LocalizedText` 的结构相等(key + 参数),
    /// 与语言无关,locale 切换不影响冷却。
    cooldowns: HashMap<String, (LocalizedText, Instant)>,
}

impl NoticeOutbox {
    pub fn push(&mut self, level: NoticeLevel, text: impl Into<LocalizedText>) {
        self.items.push(Notice { level, text: text.into(), key: None });
    }

    /// 带去重键的版本:同 key 再推只刷新文本与计时(语义同 `App::push_toast_keyed`)。
    pub fn push_keyed(&mut self, level: NoticeLevel, text: impl Into<LocalizedText>, key: impl Into<String>) {
        self.items.push(Notice { level, text: text.into(), key: Some(key.into()) });
    }

    /// 带冷却的 `push_keyed`:同一 key 下,与上一次推出的文本相同且距今不足 [`NOTICE_COOLDOWN`] 则不推;
    /// 文本变了或已过冷却则推并更新记录。`now` 由调用方注入。
    pub fn push_keyed_cooldown(&mut self, level: NoticeLevel, text: impl Into<LocalizedText>, key: impl Into<String>, now: Instant) {
        let text = text.into();
        let key = key.into();
        let repeat = matches!(self.cooldowns.get(&key), Some((last, at)) if *last == text && now.duration_since(*at) < NOTICE_COOLDOWN);
        if repeat {
            return;
        }
        self.cooldowns.insert(key.clone(), (text.clone(), now));
        self.push_keyed(level, text, key);
    }

    pub fn take(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str) -> LocalizedText {
        LocalizedText::keyed("dozer.todo", key, "fallback")
    }

    #[test]
    fn push_and_take_preserve_order_and_empty_the_box() {
        let mut o = NoticeOutbox::default();
        o.push(NoticeLevel::Error, text("a"));
        o.push_keyed(NoticeLevel::Warning, text("b"), "k");
        let items = o.take();
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].level, items[0].key.as_deref()), (NoticeLevel::Error, None));
        assert_eq!((items[1].level, items[1].key.as_deref()), (NoticeLevel::Warning, Some("k")));
        assert!(o.take().is_empty());
    }

    /// 评审关注点 2:轮询每秒失败一次,相同文本在冷却内只提示一次。
    #[test]
    fn the_same_text_inside_the_cooldown_is_pushed_once() {
        let mut o = NoticeOutbox::default();
        let t0 = Instant::now();
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0);
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0 + Duration::from_secs(1));
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0 + Duration::from_secs(59));
        assert_eq!(o.take().len(), 1);
    }

    #[test]
    fn a_different_text_or_an_expired_cooldown_pushes_again() {
        let mut o = NoticeOutbox::default();
        let t0 = Instant::now();
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0);
        o.push_keyed_cooldown(NoticeLevel::Error, text("other"), "poll", t0 + Duration::from_secs(1));
        o.push_keyed_cooldown(NoticeLevel::Error, text("other"), "poll", t0 + Duration::from_secs(61));
        assert_eq!(o.take().len(), 3, "a changed error shows again, and so does the same one after 60s");
    }

    #[test]
    fn draining_does_not_reset_the_cooldown() {
        let mut o = NoticeOutbox::default();
        let t0 = Instant::now();
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0);
        assert_eq!(o.take().len(), 1);
        o.push_keyed_cooldown(NoticeLevel::Error, text("load"), "poll", t0 + Duration::from_secs(1));
        assert!(o.take().is_empty(), "draining every frame must not defeat the cooldown");
    }
}
```

`src/lib.rs` 顶部加 `mod notice; pub use notice::{NOTICE_COOLDOWN, Notice, NoticeLevel, NoticeOutbox};`。

- [ ] **Step 2: 运行**

```bash
cargo test -p dozer-panel-host 2>&1 | grep -E "^(error|warning)|test result|FAILED"
```

Expected: `test result: ok. 7 passed`(3 + 4),没有 warning。

- [ ] **Step 3: Dozer 一侧的转换(`toast.rs`,不改现有 `Outbox`)**

在 `crates/dozer-app/src/extensions/toast.rs` 的 `Outbox` 定义之后加:

```rust
/// 共享面板库的中性提示 → Dozer 的待发提示:补上日志来源 `scope`,级别一一对应。
/// `Debug` 级只写日志、不弹 Toast,所以返回 `None` 并在这里落日志。
pub fn pending_from_notice(scope: Scope, n: dozer_panel_host::Notice) -> Option<Pending> {
    use dozer_panel_host::NoticeLevel;
    let level = match n.level {
        NoticeLevel::Debug => {
            dozer_core::log_debug!(scope, "{}", crate::i18n::host_render_stable(&n.text));
            return None;
        }
        NoticeLevel::Info => Level::Info,
        NoticeLevel::Success => Level::Success,
        NoticeLevel::Warning => Level::Warning,
        NoticeLevel::Error => Level::Error,
    };
    Some(Pending { scope, level, text: n.text, key: n.key })
}
```

并在 `toast.rs` 的 `mod tests` 里加:

```rust
    #[test]
    fn a_neutral_notice_becomes_a_pending_toast_with_the_scope_added_and_debug_never_toasts() {
        use dozer_panel_host::{Notice, NoticeLevel};
        let text = dozer_model::LocalizedText::keyed("dozer.todo", "k", "fallback");
        let n = |level| Notice { level, text: text.clone(), key: Some("x".into()) };
        let p = pending_from_notice(crate::extensions::todo::LOG, n(NoticeLevel::Error)).unwrap();
        assert_eq!((p.level, p.key.as_deref()), (Level::Error, Some("x")));
        assert_eq!(pending_from_notice(crate::extensions::todo::LOG, n(NoticeLevel::Warning)).unwrap().level, Level::Warning);
        assert!(pending_from_notice(crate::extensions::todo::LOG, n(NoticeLevel::Debug)).is_none());
    }
```

```bash
cargo test -p dozer-app toast 2>&1 | grep -E "^(error|warning: unused)|^test result|FAILED"
```

Expected: 通过,数量比基线的 `toast` 测试多 1。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-panel-host crates/dozer-app/src/extensions/toast.rs
git commit -m "refactor: a neutral NoticeOutbox in dozer-panel-host, with the toast conversion

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `dozer-panel-todo` 骨架,搬纯逻辑(筛选、拖拽、协议编解码)

**Files:**
- Create: `crates/dozer-panel-todo/Cargo.toml`、`src/lib.rs`
- Move(`git mv`): `crates/dozer-app/src/extensions/todo/{filter,category_drag}.rs` → `crates/dozer-panel-todo/src/`
- Modify: `crates/dozer-app/Cargo.toml`、`crates/dozer-app/src/extensions/todo/mod.rs`、`crates/dozer-app/src/app/mod.rs`(或定义 `tree_drag_*` 的文件)

**Interfaces:**
- Produces: `dozer_panel_todo::{filter, category_drag}`;`category_drag::{tree_drag_past_threshold, tree_drag_held_long_enough}`(从 `app` 搬来的两个纯函数,逐字)。
- Consumes: Task 1/2 的 `dozer-panel-host`(本任务暂不用,先把依赖声明好)。

- [ ] **Step 1: 建 crate 骨架**

`crates/dozer-panel-todo/Cargo.toml`:

```toml
[package]
name = "dozer-panel-todo"
version = "0.1.0"
edition.workspace = true
publish = false

# 同源 Todo 面板库(S3a):Dozer 与 Digger 共用的状态 / 更新 / 协议 / 视图 / web 资源。
# 依赖闭包刻意**不含** dozer-app / dozer-client / dozer-core(见 spec §4.1):领域 model/services、
# 共享宿主件与公共 UI 允许,产品专属语义经注入的适配器提供。
[dependencies]
dozer-model = { path = "../dozer-model" }
dozer-services = { path = "../dozer-services" }
dozer-i18n = { path = "../dozer-i18n" }
dozer-panel-host = { path = "../dozer-panel-host" }
byteui = { git = "https://github.com/byteboyai/byteui", tag = "v0.4.3" }
bytehost-core = { git = "https://github.com/byteboyai/bytehost", tag = "v0.4.0" }
bytehost-desktop = { git = "https://github.com/byteboyai/bytehost", tag = "v0.4.0" }
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
iced_widget = { version = "0.14", features = ["canvas", "svg"] }
iced_renderer = "0.14"

[dev-dependencies]
tempfile = "3"
```

(`byteui` / `bytehost-*` / `iced_*` 的版本与 `crates/dozer-panel-usage/Cargo.toml` 保持完全一致;若该文件版本更新过,以它为准。)

`src/lib.rs`:

```rust
//! 同源 Todo 面板库(S3a):Dozer 与 Digger 共用的 Todo 面板数据层、视图层与 web 资源。
//!
//! 依赖闭包**不含** `dozer-app` / `dozer-client` / `dozer-core`。产品专属语义经注入提供,见 `adapter`。

pub mod category_drag;
pub mod filter;
```

- [ ] **Step 2: 搬 `filter.rs`、`category_drag.rs`**

```bash
git mv crates/dozer-app/src/extensions/todo/filter.rs crates/dozer-panel-todo/src/filter.rs
git mv crates/dozer-app/src/extensions/todo/category_drag.rs crates/dozer-panel-todo/src/category_drag.rs
```

找到 `crate::app::tree_drag_past_threshold` / `tree_drag_held_long_enough` 的定义(`grep -rn "fn tree_drag_past_threshold\|fn tree_drag_held_long_enough" crates/dozer-app/src`),把这两个**纯函数逐字**搬到 `category_drag.rs` 顶部并设为 `pub`;原位置改成 `pub(crate) use dozer_panel_todo::category_drag::{tree_drag_past_threshold, tree_drag_held_long_enough};`(文件树等其它面板仍在用)。把 `category_drag.rs` 里 `crate::app::tree_drag_…` 的三处改成本模块的同名函数。

`filter.rs` / `category_drag.rs` 里 `use super::*;` 之类依赖旧 `mod.rs` 的 `use` 行,改成显式 `use`(`dozer_model::…`)。

- [ ] **Step 3: Dozer 侧引用**

`crates/dozer-app/Cargo.toml` 加 `dozer-panel-todo = { path = "../dozer-panel-todo" }`。`extensions/todo/mod.rs` 里原来的 `mod filter; mod category_drag;` 及其 `pub use` 换成 `pub use dozer_panel_todo::{category_drag::{…}, filter::{…}};`(保持原来导出的名字不变)。

- [ ] **Step 4: 运行**

```bash
cargo test -p dozer-panel-todo 2>&1 | grep -E "^(error|warning)|test result|FAILED"
cargo test -p dozer-app 2>&1 | grep -E "^(error|warning: unused)|^test result|FAILED"
cargo tree -p dozer-panel-todo | grep -E 'dozer-(app|client|core)' || echo "closure clean"
```

Expected: 库里 `filter` / `category_drag` 的测试随文件一起搬来并通过;`dozer-app` 的测试数 = 基线 − 库里搬走的数(记下差值,Task 9 用它对账);`closure clean`。

- [ ] **Step 5: Commit**

```bash
git add -A crates Cargo.lock
git commit -m "refactor: dozer-panel-todo skeleton with the filter and category-drag modules

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 搬状态、更新、协议;换掉内核耦合

**Files:**
- Move: `extensions/todo/{state,update,protocol}.rs` → `crates/dozer-panel-todo/src/`
- Create: `crates/dozer-panel-todo/src/adapter.rs`(`TextInputTarget`、出口消息)
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`、`app/update.rs`(约 1945–1975 行的 `Hover` / `TextInputMenuOpen` 处理)、`app/app.rs`

**Interfaces:**
- Consumes: Task 1 的 `RequestTrackers` / `RequestTokenWrapExt` / `ActivationCtx` / `project_of` / `scope_of`;Task 2 的 `NoticeOutbox`。
- Produces(`dozer_panel_todo`):`WorkspaceState`、`Message`(其中 `Hover(HoverSlot, bool)`、`TextInputMenuOpen(adapter::TextInputTarget)`)、`update`、`on_activate`、`request_todos_refresh`、`request_categories_refresh`、`parse_todo_event`、`route_event`、`Routed`、`TodoWebviewEvent`、`WebviewPushState`、`current_view_payload`、`encode_todo_push`、`today_ymd`、`tick`、`take_outbox() -> Vec<Notice>`、`TODO_OWNER`。**`LOG` 不进库**(见下)。

- [ ] **Step 1: 先写守护测试(悬停键映射,评审关注点 3)**

在 `crates/dozer-app/src/extensions/todo/mod.rs`(包装层)的测试里加(此时 `Message::Hover` 已是新形状,所以先让它编译失败):

```rust
    /// 评审关注点 3:库里的 `HoverSlot` 映射回的 `HoverId`,必须与抽库前 `HoverId::row(..)` / `HoverId::list_collapse(..)` 构造的完全相同。
    #[test]
    fn hover_slots_map_back_to_the_same_hover_ids() {
        use crate::app::{HoverId, PanelKind};
        use bytehost_desktop::HoverSlot;
        assert_eq!(hover_id_of(HoverSlot::Row(42)), HoverId::row(PanelKind::Todo, 42));
        assert_eq!(hover_id_of(HoverSlot::ListCollapse), HoverId::list_collapse(PanelKind::Todo));
        assert_eq!(hover_id_of(HoverSlot::Choice(3)), HoverId::choice(PanelKind::Todo, 3));
    }
```

- [ ] **Step 2: 搬文件,改内核耦合点**

```bash
git mv crates/dozer-app/src/extensions/todo/state.rs crates/dozer-panel-todo/src/state.rs
git mv crates/dozer-app/src/extensions/todo/update.rs crates/dozer-panel-todo/src/update.rs
git mv crates/dozer-app/src/extensions/todo/protocol.rs crates/dozer-panel-todo/src/protocol.rs
```

逐项替换(按编译器指出的位置):

| 原来 | 改成 |
|---|---|
| `use crate::app::HoverId;`、`Message::Hover(HoverId, bool)` | `use bytehost_desktop::HoverSlot;`、`Message::Hover(HoverSlot, bool)` |
| `crate::app::TextInputTarget` | `crate::adapter::TextInputTarget` |
| `use crate::panel_host::RequestTokenWrapExt;`、`crate::panel_host::{RequestTrackers, ActivationCtx, project_of, scope_of}` | `dozer_panel_host::{…}` 同名 |
| `crate::extensions::toast::Outbox` / `Pending` / `Level` | `dozer_panel_host::{NoticeOutbox, Notice, NoticeLevel}`;`take_outbox() -> Vec<Notice>` |
| `ws_state.outbox.push(LOG, Level::Error, text)` | `ws_state.outbox.push(NoticeLevel::Error, text)`(去掉 `LOG` 实参) |
| `push_keyed_cooldown(LOG, Level::Error, text, key, now)` | `push_keyed_cooldown(NoticeLevel::Error, text, key, now)` |
| `dozer_core::log_debug!(LOG, "任务/分类读取失败(dozerd 不可用): {message}")` | `ws_state.outbox.push(NoticeLevel::Debug, LocalizedText::literal(TODO_OWNER, format!("任务/分类读取失败(dozerd 不可用): {message}")))` |
| `dozer_core::scope!(pub(crate) LOG, panel, "todo");` | **删除**(库不依赖 `dozer-core`);`LOG` 在 Dozer 包装层定义 |
| `crate::i18n::HOST_OWNER`(`protocol.rs:452`) | `TODO_OWNER` 或库自己的 owner 常量(按原语义取用) |
| `crate::workspace::agent_icon(agent)`(`protocol.rs:127`) | 经 `ProtocolHost` 参数传入:`agent_vms(icon_of: &dyn Fn(AgentKind) -> AgentIconData)`,其中 `AgentIconData { bytes: Vec<u8>, preserves_color: bool }`(见 Step 3) |

`src/adapter.rs`:

```rust
//! 产品注入的适配类型。

/// 文本框的右键菜单目标:库只发出这个不透明目标,由产品转成自己的菜单请求。
/// 形状与 Dozer `app::TextInputTarget` 相同。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}

/// agent 图标的数据(原来 `protocol.rs` 直接调 `workspace::agent_icon` 取字节与"保留原色"标记)。
#[derive(Debug, Clone)]
pub struct AgentIconData {
    pub bytes: Vec<u8>,
    pub preserves_color: bool,
}
```

`lib.rs` 加 `pub mod adapter; pub mod protocol; pub mod state; pub mod update;` 及 `pub use` 重新导出(照 `extensions/todo/mod.rs` 原来对外导出的名字,逐个照搬)。`TODO_OWNER` 常量(原在 `mod.rs:16`)搬进库的 `lib.rs`:`pub const TODO_OWNER: &str = "dozer.todo";`。

- [ ] **Step 3: Dozer 包装层(`extensions/todo/mod.rs` 缩成这样)**

```rust
//! Todo 面板的 Dozer 侧包装:面板本体已抽到共享库 `dozer-panel-todo`(S3a),这里只留
//! 重新导出和产品专属的注册 / 适配。

pub use dozer_panel_todo::*;

dozer_core::scope!(pub(crate) LOG, panel, "todo");

/// 库的悬停槽位 → Dozer 悬停动画表的键。
pub(crate) fn hover_id_of(slot: bytehost_desktop::HoverSlot) -> crate::app::HoverId {
    crate::app::HoverId::Panel(crate::app::PanelKind::Todo, slot)
}

/// 库的文本框目标 → Dozer 的。
pub(crate) fn text_target_of(t: dozer_panel_todo::adapter::TextInputTarget) -> crate::app::TextInputTarget {
    crate::app::TextInputTarget { id: t.id, secure: t.secure }
}

/// agent 图标:库不认识 `workspace::agent_icon`,在这里取字节交给它。
pub(crate) fn agent_icon_data(agent: dozer_model::AgentKind) -> dozer_panel_todo::adapter::AgentIconData {
    let icon = crate::workspace::agent_icon(agent);
    dozer_panel_todo::adapter::AgentIconData { bytes: icon.bytes().to_vec(), preserves_color: icon.preserves_original_color() }
}
```

`app/update.rs`(约 1945–1947 行)改为:

```rust
                todo::Message::Hover(slot, h) => self.set_hover(todo::hover_id_of(slot), h),
                todo::Message::TextInputMenuOpen(target) => {
                    // 原有处理不变,只把目标转成 Dozer 的类型
                    let target = todo::text_target_of(target);
                    …
```

其余 `todo::Message::…` 调用点保持原样(包装层 `pub use dozer_panel_todo::*` 让它们照常编译)。所有 `ws.todo.take_outbox()` 的调用点(`app/update.rs:215`)改为:

```rust
                pending.extend(ws.todo.take_outbox().into_iter().filter_map(|n| crate::extensions::toast::pending_from_notice(todo::LOG, n)));
```

调用 `todo::agent_vms()` 的地方(`protocol.rs` 内部与 `app/app.rs` 的推送构造)传入 `todo::agent_icon_data`。

- [ ] **Step 4: 运行**

```bash
cargo test -p dozer-panel-todo 2>&1 | grep -E "^(error|warning)|test result|FAILED"
cargo test -p dozer-app 2>&1 | grep -E "^(error|warning: unused)|^test result|FAILED"
bash scripts/check-log-scope.sh 2>&1 | tail -1
```

Expected: 先看到 Step 1 的测试因 `Message::Hover` 形状而编译失败(RED),改完后全绿;`dozer-app` 与库的测试数之和 = 基线(扣掉 Task 3 已对账的部分,再加上 Task 2 的 1 条和 Task 4 的 1 条新增);`log scope check: ok`。`mod.rs` 里依赖 `crate::service_fakes::FakeTodo` / `crate::panel_host::testing` 的原有测试**留在 `dozer-app`**(它们只用库的公开 API;需要的 `pub(crate)` 项改 `pub` 并加 `#[doc(hidden)]`),其余随文件走。

- [ ] **Step 5: Commit**

```bash
git add -A crates Cargo.lock
git commit -m "refactor: move todo state, update and protocol into dozer-panel-todo

Hover keys, text-input targets, notices, logging and agent icons now cross
the crate boundary through neutral types.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `PanelDisplay` 与视图

**Files:**
- Modify: `crates/dozer-panel-todo/src/adapter.rs`(加 `PanelDisplay`)
- Move: `extensions/todo/view.rs` → `crates/dozer-panel-todo/src/view.rs`
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`(实现 `PanelDisplay`)、`app/view.rs`(约 635、648 行调用 `todo::view`)、`theme/geometry.rs`

**Interfaces:**
- Produces: `dozer_panel_todo::adapter::PanelDisplay`,`dozer_panel_todo::view::view`(参数从 `&impl PanelHost` 换成 `&impl PanelDisplay`),`dozer_panel_todo::top_row_inner_h_px()`。

`PanelDisplay` 只含 `view.rs` 现在向 `PanelHost` / Dozer 内核要的东西:

```rust
use bytehost_desktop::HoverSlot;
use iced_widget::core::{Element, Padding};

/// 产品给 Todo 视图的展示能力(同 `dozer-panel-usage` 的同名 trait,形状沿用,只加 todo 确实需要的)。
pub trait PanelDisplay {
    /// 某个按钮 / 卡片的悬停动画进度 0.0..=1.0。
    fn hover_progress(&self, slot: HoverSlot) -> f32;
    /// 列表列当前是否收起。
    fn list_collapsed(&self) -> bool;
    /// 列表列的折叠 / 展开按钮;`on_select` 与 `on_hover` 由库给出。
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        collapsed: bool,
        tooltip_collapse: String,
        tooltip_expand: String,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板头(图标 + 标题 + 分割线)。
    fn panel_head<'a, M: 'a>(&self, icon: byteui::interaction::icons::IconKind, title: String) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板头外侧的内边距(Dozer 取自 `workspace.json` 的 `project_pane` 区域)。
    fn header_padding(&self) -> Padding;
    /// 两个 pane 之间的水平分割线。
    fn tab_divider<'a, M: 'a>(&self) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 当前生效语言(BCP-47)。
    fn locale(&self) -> &str {
        "en"
    }
}

/// 右区顶栏行的内高:`tab_button_size + 8`。**唯一来源**:视图和产品的 webview 让位几何都读它。
pub fn top_row_inner_h_px() -> f32 {
    byteui::theme::geometry::tab_button_size() + 8.0
}
```

- [ ] **Step 1: 先写守护测试(评审关注点 4)**

库 `view.rs` 末尾:

```rust
#[cfg(test)]
mod tests {
    /// 评审关注点 4:原生 webview 让位的高度与视图用的是同一个数。
    #[test]
    fn top_row_height_has_one_source() {
        assert_eq!(crate::adapter::top_row_inner_h_px(), byteui::theme::geometry::tab_button_size() + 8.0);
    }
}
```

Dozer 侧 `crates/dozer-app/src/theme/geometry.rs` 的测试里加:

```rust
    #[test]
    fn todo_top_row_height_is_the_librarys() {
        assert_eq!(todo_top_row_inner_h_px(), dozer_panel_todo::adapter::top_row_inner_h_px());
        assert_eq!(todo_top_row_h_px(), dozer_panel_todo::adapter::top_row_inner_h_px() + 1.0);
    }
```

- [ ] **Step 2: 搬视图,改调用**

```bash
git mv crates/dozer-app/src/extensions/todo/view.rs crates/dozer-panel-todo/src/view.rs
```

按下表替换(`view.rs` 内):

| 原来 | 改成 |
|---|---|
| `app: &impl PanelHost` | `app: &impl PanelDisplay` |
| `crate::chrome::homespace::home_panel_head(icon, title)` | `app.panel_head(icon, title)` |
| `theme::region::project_pane().padding`(2 处) | `app.header_padding()` |
| `app.list_collapse_button(PanelKind::Todo, collapsed, HoverId::list_collapse(..), tc, te, on_select, on_hover)` | `app.list_collapse_button(collapsed, tc, te, on_select, on_hover)`;`on_hover` 里发 `Message::Hover(HoverSlot::ListCollapse, hovered)` |
| `app.list_collapsed(PanelKind::Todo)` | `app.list_collapsed()` |
| `app.hover_progress(HoverId::row(PanelKind::Todo, id))`、`Message::Hover(HoverId::row(..), h)` | `app.hover_progress(HoverSlot::Row(id))`、`Message::Hover(HoverSlot::Row(id), h)` |
| `crate::app::tab_divider()` | `app.tab_divider()` |
| `theme::geometry::todo_top_row_inner_h_px()` | `crate::adapter::top_row_inner_h_px()` |
| `crate::i18n::panel_text(TODO_OWNER, k, fb, locale)` / `panel_text_args` | `crate::i18n::t(k, fb, locale)` / `t_args`(Task 6 建 `i18n` 模块;**本任务先在 `lib.rs` 里放一个转发到 `dozer_i18n` 的最小 `i18n` 模块**,Task 6 再换成带资源的完整版) |
| `crate::i18n::host_render(text, locale)`(渲染 `webview_failed`) | 参数改为 `webview_failed: Option<String>`,由产品先渲染好再传入 |
| `TextInputMenuOpen(crate::app::TextInputTarget{..})` | `TextInputMenuOpen(crate::adapter::TextInputTarget{..})` |

Dozer 侧:`impl PanelDisplay for App`(放 `extensions/todo/mod.rs`,各方法转调 `PanelHost` 与现有 chrome 函数,`hover_progress(slot)` 先 `hover_id_of(slot)`),`app/view.rs` 的两处 `todo::view(app, …)` 把 `webview_failed` 先 `map(|t| crate::i18n::host_render(t, app.locale()))`。`theme/geometry.rs` 里 `todo_top_row_inner_h_px()` 改为 `dozer_panel_todo::adapter::top_row_inner_h_px()`。

- [ ] **Step 3: 运行**

```bash
cargo test -p dozer-panel-todo 2>&1 | grep -E "^(error|warning)|test result|FAILED"
cargo test -p dozer-app 2>&1 | grep -E "^(error|warning: unused)|^test result|FAILED"
```

Expected: 守护测试先因 `adapter::top_row_inner_h_px` 不存在而失败(RED),再绿;测试数对账同 Task 4。

- [ ] **Step 4: Commit**

```bash
git add -A crates
git commit -m "refactor: move the todo view into dozer-panel-todo behind PanelDisplay

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 前端应用、文案、资源路径

**Files:**
- Move: `crates/dozer-app/web/todo-content` → `crates/dozer-panel-todo/web/todo-content`;`crates/dozer-app/assets/todo-content` → `crates/dozer-panel-todo/assets/todo-content`;`crates/dozer-app/locales/panels/todo` → `crates/dozer-panel-todo/locales`
- Modify: `crates/dozer-panel-todo/web/todo-content/build.mjs`(输出路径)、`src/i18n.rs`(完整版)、`lib.rs`(`web_assets_root`)、`crates/dozer-app/src/assets.rs`、`scripts/build-macos-app.sh`

**Interfaces:**
- Produces: `dozer_panel_todo::web_assets_root() -> PathBuf`;`dozer_panel_todo::i18n::{TODO_OWNER, t, t_args}`。

- [ ] **Step 1: 先写守护测试(评审关注点 5)**

`crates/dozer-app/src/assets.rs` 的 `mod tests`(紧挨 `usage_content_bundle_assets_are_present`)加:

```rust
    /// 评审关注点 5:todo-content 产物的**唯一来源**在 `dozer-panel-todo`;`dozer-app` 里不再留副本。
    #[test]
    fn todo_content_bundle_assets_are_present_and_have_one_source() {
        let root = dozer_panel_todo::web_assets_root();
        for f in ["host.html", "todo-content.js", "todo-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 todo-content 产物 {f}: {}", p.display());
            assert!(std::fs::metadata(&p).unwrap().len() > 0, "todo-content 产物为空: {f}");
        }
        let old = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/todo-content");
        assert!(!old.exists(), "dozer-app 里不应再留 todo-content 副本: {}", old.display());
    }
```

(`assets/todo-content` 里实际有 `host.html`、`todo-content.css`、`todo-content.js` 三个文件。)

- [ ] **Step 2: 搬目录,改构建输出**

```bash
git mv crates/dozer-app/web/todo-content crates/dozer-panel-todo/web/todo-content
git mv crates/dozer-app/assets/todo-content crates/dozer-panel-todo/assets/todo-content
mkdir -p crates/dozer-panel-todo/locales
git mv crates/dozer-app/locales/panels/todo/en.json crates/dozer-panel-todo/locales/en.json
git mv crates/dozer-app/locales/panels/todo/zh-CN.json crates/dozer-panel-todo/locales/zh-CN.json
```

`web/todo-content/build.mjs` 里 `const outdir = path.resolve(here, '../../assets/todo-content');` 的相对层级随目录一起变:`web/todo-content` → 库根是 `../..`,所以**这行不用改**(`crates/dozer-panel-todo/web/todo-content/../../assets/todo-content`)。注释里的 `crates/dozer-app/assets/todo-content/` 改成 `crates/dozer-panel-todo/assets/todo-content/`。若前端 import 了 `../shared/…`(`crates/dozer-app/web/shared`),按 `grep -rn "shared" crates/dozer-panel-todo/web/todo-content/src crates/dozer-panel-todo/web/todo-content/*.mjs` 的结果处理:共享的前端文件**复制进** `crates/dozer-panel-todo/web/shared/`(与 usage 库的做法一致,不跨 crate 引用)。

- [ ] **Step 3: 完整的 `i18n` 模块与 `web_assets_root`**

`crates/dozer-panel-todo/src/i18n.rs`(照 `crates/dozer-panel-usage/src/i18n.rs` 的写法,owner 换成 `dozer.todo`,资源 `locales/{en,zh-CN}.json`):

```rust
//! Todo 面板自带的文案解析:资源随库(`locales/{en,zh-CN}.json`,owner `dozer.todo`),locale 由宿主注入。
//! 解析复用中性纯库 `dozer-i18n`,不依赖 `dozer-app`。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

pub use crate::TODO_OWNER;

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(TODO_OWNER);
        set.insert_bundle("en", ResourceBundle::from_json(EN_JSON).expect("todo en.json 必须合法"));
        set.insert_bundle("zh-CN", ResourceBundle::from_json(ZH_CN_JSON).expect("todo zh-CN.json 必须合法"));
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

/// 解析一条 `dozer.todo` 文案。`fallback` 是英文兜底。
pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    resolver().resolve(&LocalizedText::keyed(TODO_OWNER, key, fallback), locale).text
}

/// 同 [`t`],带具名参数。
pub fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut text = LocalizedText::keyed(TODO_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}
```

`lib.rs` 加:

```rust
pub mod i18n;

/// 本 crate 自带的 todo-content web 资源根(`assets/todo-content/`,由 `web/todo-content/build.mjs` 构建)。
/// **唯一来源**:消费方(Dozer / Digger)从这条路径取 `host.html` / `todo-content.js` / …,不各自留副本。
/// 返回 `PathBuf` 而非 `&'static Path`,便于消费方在打包态改为指向 `Contents/Resources/todo-content`。
pub fn web_assets_root() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/todo-content"))
}
```

`dozer-app` 里 `i18n.rs` 的 `PANEL_OWNERS` 若登记了 `dozer.todo`(`grep -n "dozer.todo" crates/dozer-app/src/i18n.rs`),按 `dozer.usage` 的先例删除或改为"独立面板自带资源"。

- [ ] **Step 4: Dozer 的资源服务与打包**

`crates/dozer-app/src/assets.rs`:`todo_content_root_for`(约 193–196 行)改成与 `usage_content_root_for`(约 173–186 行)同型:打包态(`flyfish_root` 的兄弟目录 `todo-content` 存在)优先,否则回退 `dozer_panel_todo::web_assets_root()`。`scripts/build-macos-app.sh:137` 改为:

```bash
cp -R "$ROOT_DIR/crates/dozer-panel-todo/assets/todo-content" "$APP_DIR/Contents/Resources/todo-content"
```

(并把上方说明注释里"从 dozer-app 下取"同步成"从 dozer-panel-todo 取",照第 126 行 usage 的写法。)

- [ ] **Step 5: 运行 + 前端回归 + 产物对比**

```bash
cargo test -p dozer-panel-todo -p dozer-app 2>&1 | grep -E "^(error|warning: unused)|^test result|FAILED"
(cd crates/dozer-panel-todo/web/todo-content && npm test 2>&1 | tail -8 && npm run build 2>&1 | tail -2)
git status --short crates/dozer-panel-todo/assets | head       # 重新构建后应与提交的产物一致
diff -r <(git show HEAD:crates/dozer-app/assets/todo-content/todo-content.js) crates/dozer-panel-todo/assets/todo-content/todo-content.js && echo "bundle identical"
```

Expected: Rust 测试同上;前端测试通过数 = Task 0 基线;**重新构建后 `git status` 为空、`bundle identical`**(若 Task 0 Step 3 记了既有差异,允许同样的差异)。

- [ ] **Step 6: Commit**

```bash
git add -A crates scripts
git commit -m "refactor: the todo web app, locales and asset root now live in dozer-panel-todo

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: 登记新 crate 到 Dozer 的门禁

**Files:**(照 `dozer-panel-usage` 的每一处登记)
- Modify: `scripts/i18n/scan_sources.py`(约 54–55、73 行)、`scripts/i18n/check_migration.py`(约 61–64、104、126、378 行)、`scripts/i18n/build_inventory.py`(约 66 行)、`scripts/audit/panel-boundary.baseline.json`
- Regenerate: `docs/dozer-v2/i18n/inventory.json`

- [ ] **Step 1: 看门禁现在怎么失败**

```bash
bash scripts/check-i18n.sh 2>&1 | tail -15
python3 scripts/audit/check_panel_boundary.py 2>&1 | tail -15
```

Expected: 失败,指出 `extensions/todo` 的路径不存在、`crates/dozer-panel-todo/locales` 未登记 owner 等 —— 这是红。

- [ ] **Step 2: 逐处登记**

用 `grep -n "dozer-panel-usage\|dozer.usage" scripts/i18n/*.py scripts/audit/*` 找到每一处,为 `dozer-panel-todo` / `dozer.todo` 加同型的条目:
- `scan_sources.py`:扫描根加 `crates/dozer-panel-todo/src`、`crates/dozer-panel-todo/web`;路径前缀 → owner 映射加 `("crates/dozer-panel-todo/", "dozer.todo")`(并去掉指向 `crates/dozer-app/locales/panels/todo` 的旧映射);
- `check_migration.py`:独立面板 `locales/` 约定加上 `dozer.todo`;扫描根与 `--assets` 检查里的 `crates/dozer-panel-usage/assets` 旁加 `crates/dozer-panel-todo/assets`;
- `build_inventory.py`:分组表(约 64–66 行,"First panels" 一组,编号 `"09"`)把 `("crates/dozer-app/src/extensions/todo/", "09")` 和 `("crates/dozer-app/web/todo-content/", "09")` 换成 `("crates/dozer-panel-todo/", "09")`;**还有按路径写死的例外**要跟着改路径:约 114–115 行的排除项 `crates/dozer-app/web/todo-content/src/{fixtures.ts,render-smoke.tsx}` → `crates/dozer-panel-todo/web/todo-content/src/…`,约 141 行的 `("crates/dozer-app/src/extensions/todo/update.rs", "新分类")` → `("crates/dozer-panel-todo/src/update.rs", "新分类")`;用 `grep -n "todo" scripts/i18n/build_inventory.py scripts/i18n/scan_sources.py scripts/i18n/check_migration.py` 把剩下的路径写死处找全;
- `panel-boundary.baseline.json`:为 `dozer-panel-todo` 登记"依赖闭包不含 dozer-app/client/core"的基线(照 `dozer-panel-usage` 的条目复制并改名);
- 重新生成清单:`python3 scripts/i18n/build_inventory.py`(提交 `docs/dozer-v2/i18n/inventory.json` 的变化,只应是 `extensions/todo` → `dozer-panel-todo` 的路径迁移,条目数不变)。

- [ ] **Step 3: 运行(绿)**

```bash
bash scripts/check-i18n.sh 2>&1 | tail -3
bash scripts/check-log-scope.sh 2>&1 | tail -1
python3 scripts/audit/check_panel_boundary.py 2>&1 | tail -3
python3 -c "import json;d=json.load(open('docs/dozer-v2/i18n/inventory.json'));print(len(d.get('entries',d)))"
```

Expected: 三个门禁都通过;清单条目总数与 Task 0 时相同(迁移路径不增不减)。

- [ ] **Step 4: Commit**

```bash
git add scripts docs
git commit -m "chore: register dozer-panel-todo and dozer-panel-host with the i18n and boundary gates

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 清理 Dozer 里的旧路径与重复实现

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`、`crates/dozer-app/src/panel_host.rs`、`crates/dozer-app/src/extensions.rs`

- [ ] **Step 1: 找残留**

```bash
cd $ROOT/dozer-panel-todo
ls crates/dozer-app/src/extensions/todo/                 # 只应剩 mod.rs(包装层 + 留在 Dozer 的测试)
grep -rn "fn request_todos_refresh\|fn request_categories_refresh\|fn route_event\|struct WorkspaceState" crates/dozer-app/src/extensions/todo/ | head
ls crates/dozer-app/assets | grep -c todo-content        # 0
ls crates/dozer-app/web | grep -c todo-content           # 0
ls crates/dozer-app/locales/panels | grep -c "^todo$"    # 0
grep -rn "panel_host::\(scope_of\|project_of\|RequestTrackers\)" crates/dozer-panel-todo | head -2    # 空
cargo build -p dozer-app 2>&1 | grep -E "^warning: (unused|dead)" | head
```

Expected: `todo/` 下只有 `mod.rs`;后面几条的计数 / 输出为 0 或空;**没有 unused / dead_code 警告**。有就删(只删确认无调用者的东西)。

- [ ] **Step 2: 依赖闭包与重复实现的最终检查**

```bash
for c in dozer-panel-host dozer-panel-todo; do cargo tree -p $c | grep -E 'dozer-(app|client|core)' && echo "LEAK in $c" || echo "$c closure clean"; done
grep -rn "mod filter\|mod category_drag\|mod protocol\|mod state;" crates/dozer-app/src/extensions/todo/ | head -2    # 空
```

Expected: 两个 `closure clean`;后一条为空。

- [ ] **Step 3: Commit**

```bash
git add -A crates
git commit -m "chore: remove the leftovers of the todo panel from dozer-app

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 9: 最终验证与交接

**Files:**
- Modify: `docs/superpowers/specs/2026-10-15-panel-todo-lib-design.md`(状态行)

- [ ] **Step 1: 测试数对账**

```bash
B="${TMPDIR:-/tmp}/s3a"
cargo test -p dozer-app -p dozer-panel-todo -p dozer-panel-host -p dozer-panel-usage -p dozer-model -p dozer-services 2>&1 | grep -E "^test result"
(cd crates/dozer-panel-todo/web/todo-content && npm test 2>&1 | tail -6)
cat $B/baseline.txt
```

Expected: **Rust 总数 = 基线总数 + 新增(Task 1 的 3 条、Task 2 的 4+1 条、Task 4 的 1 条、Task 5 的 2 条、Task 6 的 1 条)**,0 失败;前端通过数 = 基线。逐项把"基线数 → 终态数"写进交接说明;不一致就回到对应任务查,不要靠改测试对齐。

- [ ] **Step 2: 全部门禁与脚本**

```bash
bash scripts/check-i18n.sh 2>&1 | tail -1
bash scripts/check-log-scope.sh 2>&1 | tail -1
python3 scripts/audit/check_panel_boundary.py 2>&1 | tail -1
bash -n scripts/build-macos-app.sh && echo syntax-ok
git -C $ROOT/dozer status --short | wc -l        # 仍是 Task 0 记下的 N
git status --short | wc -l                        # 0
```

Expected: 全部通过;主工作区文件数不变;worktree 干净。

- [ ] **Step 3: spec 状态行**

`docs/superpowers/specs/2026-10-15-panel-todo-lib-design.md` 的 `- 状态:…` 改为 `- 状态:已实现(待用户在 Dozer 上目测验收;未合入主线、未推送)`。提交:

```bash
git add docs
git commit -m "docs(spec): S3a implemented, awaiting acceptance

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 4: 用户验收清单(交给用户,执行者不要替用户勾选)**

在 Dozer 上 `cargo run -p dozer-app`,逐项对照抽库前:
1. todo 面板:新增、勾选、编辑文本、拖拽排序、改状态、改计划日期、日历弹层。
2. 分类树:新建 / 重命名 / 删除 / 拖动移动 / 右键菜单。
3. 详情浮层:打开、回复、"处理"按钮。
4. 列表列折叠 / 展开按钮;悬停卡片时的渐变;窗口缩放时内容区 webview 紧贴顶栏下沿。
5. 失败提示:停掉 `dozerd` 后的读取失败提示(应只弹一次,60 秒内不重复)。
6. 打包:`bash scripts/build-macos-app.sh` 后 `Contents/Resources/todo-content` 存在且面板能打开。
7. 中英文切换后面板文案正确。

- [ ] **Step 5: 交接给 3b**

在交接说明里写清:合入主线与推送由用户决定;3b 需要 3a 的一个**已推送**提交作为 `dozer-panel-todo` / `dozer-panel-host` 的 git `rev`;库的公开面(`WorkspaceState`、`Message`、`update`、`on_activate`、`view::view`、`adapter::PanelDisplay`、`web_assets_root()`、`NoticeOutbox`)即 3b 的全部输入。

---

## Self-Review(写完计划后的核对)

- **Spec 覆盖**:§4.1 crate 边界 → Task 3–6;§4.2 接缝(§9 修订后)→ Task 1(host 件)、2(发件箱)、4(悬停 / 文本框 / 日志 / agent 图标)、5(`PanelDisplay`)、6(i18n / 资源);§4.3 Dozer 包装层 → Task 4 Step 3 与 Task 8;§5 分步 → Task 1–8;§6 验证 → Task 0 / 9;§7 风险 → Review Focus 与 Task 9;§9.8 门禁登记 → Task 7。
- **有意与 spec 不同的一处**:spec §5 的 6 步在计划里是 9 个任务(多了 `dozer-panel-host` 抽出、中性发件箱、门禁登记),均来自 §9 修订。
- **计划里没有逐行验证过的部分**(诚实标注):这是一次搬迁式重构,不像 S1/S2 那样在草稿副本里整体编译验证过。Task 1–6 给出的**接缝代码**(host 件的搬迁清单、`NoticeOutbox`、`PanelDisplay`、`i18n` 模块、悬停 / 文本框映射)已对照源码核过签名与行号;各文件内部的 `use` 行、`pub(crate)`→`pub` 的具体处则只能按编译器报告逐个处理(Global Constraints 里写明了规则),这不是遗留项,而是搬迁的机械部分。每个任务的"运行"步骤都有明确的 Expected,编译失败时按 Expected 回查。
- **类型一致性**:`HoverSlot`(Task 4 的 `Message::Hover`、Task 5 的 `PanelDisplay::hover_progress`)、`TextInputTarget` / `AgentIconData`(Task 4 `adapter.rs`)、`NoticeOutbox` / `Notice` / `NoticeLevel`(Task 2,Task 4 使用)、`top_row_inner_h_px`(Task 5 定义,Dozer `theme/geometry.rs` 委托)、`web_assets_root` / `i18n::{t, t_args}`(Task 6 定义,Task 5 的占位版与 Task 6 的完整版同签名)在各任务间名字与签名一致。
- **占位符**:没有 TBD。门禁脚本里按路径写死的例外(Task 7)已列出已知的几处,其余由 `grep` 命令找全。
