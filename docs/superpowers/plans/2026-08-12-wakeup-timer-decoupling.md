# 唤醒定时器解耦 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `poll_todo_if_visible` 补上与 `toggle_blink` 同款的显式自限速;`about_to_wait` 的 interval 选择从嵌套 if/else 改成列表取最小值,让新增第 4 个周期性关注点不用碰现有分支。

**Architecture:** 两处改动都是纯调度层重构,不改变任何可观察行为。`App` 新增一个 `last_todo_poll_at: Instant` 字段(与已有的 `last_blink_at` 同构);`main.rs` 的 `TODO_POLL_INTERVAL` 加 `pub(crate)`;`about_to_wait` 内部改成 `[(bool, Duration); 3]` 列表 + `.filter().map().min()`。

**Tech Stack:** Rust workspace;不新增依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/wakeup-timer-decoupling`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `20ddafa`)分析**。工作目录被多个并行会话共享,开工前用本文档的 `grep -n` 模式核对实际行号。
- **不改变任何可观察行为**——闪烁相位、悬停动画曲线、Todo 面板刷新时机,用户侧应该感知不到任何差异。这是纯调度层重构。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check` 干净通过,`cargo test` 预期 484 passed / 2 failed(两个既有失败,与本计划无关)。
- 设计文档:`docs/superpowers/specs/2026-08-12-wakeup-timer-decoupling-design.md`,有疑问以它为准。

---

### Task 1: `poll_todo_if_visible` 补自限速

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Produces:`App` 新字段 `last_todo_poll_at: std::time::Instant`(私有);`main.rs` 的 `TODO_POLL_INTERVAL` 从私有 `const` 改为 `pub(crate) const`。
- Consumes:`crate::TODO_POLL_INTERVAL`(Task 2 也会用到同一个常量,顺序不影响——Task 2 不依赖 Task 1 的改动)。

- [ ] **Step 1: `main.rs` 给 `TODO_POLL_INTERVAL` 加 `pub(crate)`**

定位:

```bash
command grep -n "const TODO_POLL_INTERVAL" crates/dozer-app/src/main.rs
```

预期在 `main.rs:231` 附近,原文:

```rust
/// Todo 面板可见时轮询 `.dozer/todo.md` 的间隔,兼顾响应与省电。
const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);
```

改成:

```rust
/// Todo 面板可见时轮询 `.dozer/todo.md` 的间隔,兼顾响应与省电。
/// `pub(crate)`——`App::poll_todo_if_visible` 也要用它把自己限速到这个
/// 节奏(同 `BLINK_INTERVAL` 的处理,理由见该常量文档)。
pub(crate) const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);
```

- [ ] **Step 2: `app.rs` 的 `App` struct 加 `last_todo_poll_at` 字段**

定位:

```bash
command grep -n "last_blink_at: std::time::Instant," crates/dozer-app/src/app.rs
```

预期在 `app.rs:1161` 附近,紧跟在这段现有字段声明之后加一个同构字段:

```rust
    /// 上次真正翻转 `blink_on` 的时刻,`toggle_blink` 据此把自己限速到
    /// `BLINK_INTERVAL` 一拍——main.rs 的定时唤醒并不专属闪烁:悬停动画
    /// 期间(`HOVER_ANIM_INTERVAL`=16ms)会把唤醒频率提到闪烁本该的 450ms
    /// 的近 30 倍,若 `toggle_blink` 对"被叫到"照单全收,状态点就会跟着
    /// hover 的那份高频唤醒一起快速明灭,观感是"悬停 icon 按钮,别处的点
    /// 跟着闪"。
    last_blink_at: std::time::Instant,
    /// 上次真正执行 Todo 面板磁盘轮询(`poll_todo_if_visible`)的时刻,
    /// 用法与 `last_blink_at` 一致:按 `TODO_POLL_INTERVAL` 自限速,未到
    /// 点的调用直接 no-op。现在靠 mtime 检查已经安全(没变化就早退,见
    /// 该方法文档),这里补上限速是为了让"周期性函数自己对被更快唤醒
    /// 节奏带跑免疫"这条约定对全部三个周期性关注点(闪烁/悬停动画/
    /// Todo 轮询)都显式成立,不留一个"靠巧合安全"的例外。
    last_todo_poll_at: std::time::Instant,
```

- [ ] **Step 3: 构造点初始化新字段**

定位:

```bash
command grep -n "last_blink_at: std::time::Instant::now()," crates/dozer-app/src/app.rs
```

预期在 `app.rs:1470` 附近,紧跟着加:

```rust
            last_blink_at: std::time::Instant::now(),
            last_todo_poll_at: std::time::Instant::now(),
```

- [ ] **Step 4: `poll_todo_if_visible` 加限速判断**

定位:

```bash
command grep -n "pub fn poll_todo_if_visible" crates/dozer-app/src/app.rs
```

预期在 `app.rs:1652` 附近,原文:

```rust
    /// `main.rs` 定时唤醒调用：只在 `todo_panel_visible()` 时才真的
    /// `stat` 一下 `.dozer/todo.md` 的 mtime；没变就是一次系统调用，
    /// 变了才重读+reparse（`todo::reload_from_disk` 内部也会再 stat 一次
    /// mtime，多一次系统调用换取它保持独立可复用）。
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let current = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if current != ws.todo.mtime() {
            todo::reload_from_disk(&mut ws.todo, std::path::Path::new(&project.path));
        }
    }
```

改成:

```rust
    /// `main.rs` 定时唤醒调用：只在 `todo_panel_visible()` 时才真的
    /// `stat` 一下 `.dozer/todo.md` 的 mtime；没变就是一次系统调用，
    /// 变了才重读+reparse（`todo::reload_from_disk` 内部也会再 stat 一次
    /// mtime，多一次系统调用换取它保持独立可复用）。按 `last_todo_poll_at`
    /// 自限速到 `TODO_POLL_INTERVAL`——悬停动画等更快节奏把唤醒带密时
    /// 不会跟着高频重复 `stat`(2026-08-12 解耦重构,同 `toggle_blink` 的
    /// 处理)。
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.last_todo_poll_at) < crate::TODO_POLL_INTERVAL {
            return;
        }
        self.last_todo_poll_at = now;
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let current = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if current != ws.todo.mtime() {
            todo::reload_from_disk(&mut ws.todo, std::path::Path::new(&project.path));
        }
    }
```

- [ ] **Step 5: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异。

- [ ] **Step 6: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 484 passed / 2 failed(两个已知的既有失败)。

- [ ] **Step 7: 人工验证**

按既有的独立命名临时二进制方法(绝不碰用户正式 app)起一个测试实例:
打开一个项目、左栏切到 Todo 面板并保持可见,在 rail 图标上来回悬停制造
密集唤醒(复现之前 blink bug 的手法),观察 Todo 面板内容/列表没有任何
肉眼可见的异常刷新或闪烁——这是这次改动唯一有外部可观察副作用风险的
路径。验证完关闭该临时实例、删除临时二进制。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
refactor(app): poll_todo_if_visible 补上与 toggle_blink 同款自限速

现在靠 mtime 检查已经安全(没变化就早退),但架构层面不强制这一点,
悬停动画等更快唤醒节奏把它带到远高于 TODO_POLL_INTERVAL 的频率时,
只是多了几次廉价 stat() 调用而非真正的 bug。这次补上显式自限速,让
"周期性函数自己对被更快节奏带跑免疫"这条约定对闪烁/Todo 轮询两者都
显式成立,不留一个靠巧合安全的例外。纯调度层改动,不改变可观察行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `about_to_wait` 改成列表取最小值

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes:`HOVER_ANIM_INTERVAL`/`BLINK_INTERVAL`/`TODO_POLL_INTERVAL`(均已在 `main.rs` 模块作用域内,`about_to_wait` 原本就直接引用,本任务不改它们的定义——`TODO_POLL_INTERVAL` 的 `pub(crate)` 化是 Task 1 做的,Task 2 不依赖 Task 1 是否已完成,两者可任意顺序执行)。

- [ ] **Step 1: 定位 `about_to_wait`**

```bash
command grep -n "fn about_to_wait" crates/dozer-app/src/main.rs
```

预期在 `main.rs:1266` 附近,原文:

```rust
        /// 每轮事件处理完后决定下次唤醒时机：有 tab 在工作就排下一拍闪烁
        /// 唤醒，否则回到 `Wait` 省电（不再空转重绘）。
        fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Ready { app, .. } = self {
                if app.any_blinking() || app.any_hover_anim_active() || app.todo_panel_visible() {
                    let interval = if app.any_hover_anim_active() {
                        HOVER_ANIM_INTERVAL
                    } else {
                        // Todo 面板可见时按固定的 TODO_POLL_INTERVAL 节奏轮询,
                        // 兼顾响应与省电;不需要像悬停动画那样切到更密的帧率。
                        if app.todo_panel_visible() && !app.any_blinking() {
                            TODO_POLL_INTERVAL
                        } else {
                            BLINK_INTERVAL
                        }
                    };
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        std::time::Instant::now() + interval,
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
        }
```

- [ ] **Step 2: 替换成列表 + 取最小值**

```rust
        /// 每轮事件处理完后决定下次唤醒时机:三个周期性关注点(状态点
        /// 闪烁/按钮悬停动画/Todo 面板轮询)各自的"是否需要唤醒"+"需要
        /// 多快"列在一起,取激活项里最小的 interval——新增第 4 个周期性
        /// 关注点只需要在这个列表里加一行,不用碰其它分支(2026-08-12
        /// 解耦重构:每个关注点自己的函数各自按自己的 `last_*_at` 限速,
        /// 这里只负责"下次什么时候唤醒",不负责"唤醒后该不该真的做事")。
        fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Ready { app, .. } = self {
                let wakes: [(bool, Duration); 3] = [
                    (app.any_hover_anim_active(), HOVER_ANIM_INTERVAL),
                    (app.any_blinking(), BLINK_INTERVAL),
                    (app.todo_panel_visible(), TODO_POLL_INTERVAL),
                ];
                if let Some(interval) = wakes
                    .into_iter()
                    .filter(|(active, _)| *active)
                    .map(|(_, interval)| interval)
                    .min()
                {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        std::time::Instant::now() + interval,
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
        }
```

`new_events`(`main.rs:1242` 附近)不改——仍然是"`ResumeTimeReached` 时
三个函数各调一次,各自决定要不要真正做事"的既有模式。

- [ ] **Step 3: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异。

- [ ] **Step 4: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 484 passed / 2 failed(两个已知的既有失败)。

- [ ] **Step 5: 人工验证**

同 Task 1 的验证方法:起一个独立命名临时二进制,确认闪烁(需要一个
"工作中"状态的 agent tab,若手头没有可跳过这一项单独验证)、悬停动画、
Todo 轮询三者各自的节奏观感与改动前一致——这个任务纯粹重排了"什么时候
唤醒"的判断逻辑,三个函数本身没有任何改动,预期完全无感知差异。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
refactor(app): about_to_wait 唤醒 interval 选择改成列表取最小值

嵌套 if/else 手动挑"当前该用哪个 interval",新增一个周期性关注点得
手动改这段分支。改成 [(bool, Duration); 3] 列表 + 取激活项最小值,
以后加第 4 个关注点只需要在列表里加一行。纯调度层重构,不改变可观察
行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

两个任务都完成后:

- [ ] `cargo build -p dozer-app --bin dozer` 全量编译无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 仍是 484 passed / 2 failed(与开工前一致)。
- [ ] `cargo fmt -p dozer-app -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets` 没有新增 lint。
- [ ] 提请审阅,通过后合并回 `main`。
