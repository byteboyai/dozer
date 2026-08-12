# 唤醒定时器解耦设计

**状态:已批准(brainstorming 会话,2026-08-12)**

## 背景

`main.rs` 的 `Runner::about_to_wait`(`main.rs:1266`)/`Runner::new_events`
(`main.rs:1242`)是一对共享的定时唤醒机制:三件不相关的周期性工作——
状态点闪烁(`toggle_blink`)、按钮悬停动画推进(`advance_hover_anims`)、
Todo 面板轮询磁盘(`poll_todo_if_visible`)——共用同一次
`ControlFlow::WaitUntil` 决策。`about_to_wait` 用嵌套 if/else 在三者里
手动挑"当前该用哪个 interval"(悬停动画进行中用最快的 16ms,否则 Todo
面板可见用 1000ms,否则用闪烁的 450ms),`new_events` 的 `ResumeTimeReached`
分支则**无条件**把三件事都做一遍,不管这次唤醒本来是为哪个 interval 排的。

这个耦合已经导致过一次真实 bug(2026-08-12 修复,见
`git log --oneline --grep=悬停 icon`):悬停 icon 按钮期间,唤醒频率被
`HOVER_ANIM_INTERVAL`(16ms)提到闪烁本该的 450ms 的近 30 倍,而
`toggle_blink` 当时对每次被叫到都照单全收直接翻转相位,状态点因此跟着
高频唤醒一起快速闪烁。已经修成 `toggle_blink` 自己按 `last_blink_at`
限速(`app.rs:1682`),问题当时已解决。

复查另外两个:

- `advance_hover_anims`——它**就是** `HOVER_ANIM_INTERVAL` 这个最快节奏
  的所有者,不存在"被更快的节奏带跑"的风险,不需要改。
- `poll_todo_if_visible`(`app.rs:1652`)——被更快的唤醒带到远高于
  1000ms 的频率调用时,不会有可见 bug:它内部先 `stat()` 一下
  `.dozer/todo.md` 的 mtime,没变化就直接返回,只有真的改了才重读+重解析
  (`app.rs:1663-1666`)。多余的调用只是多了几次廉价的 `stat()` 系统调用,
  不改变任何可观察行为。**这次审查确认它现在没有存活 bug,是运气好而非
  设计使然**——它的安全性完全靠自己"没变化就早退"这个巧合，架构层面
  并没有要求或强制这一点。

结论:**当前没有存活 bug**,但 `about_to_wait` 这套"嵌套 if/else 手动挑
最快 interval"+"每个周期性函数各自决定要不要自限速"的架构,对
**未来新增的第 4 个周期性关注点**没有任何约束——新功能的作者(人或
agent)完全可能像 `toggle_blink` 修复前那样,忘记自己的函数需要对"被
更快节奏带跑"这件事免疫,重蹈同一个 bug。这次是防御性重构,目的是让
"新增一个周期性关注点"这件事在架构上就是安全的,不用开发者自己想起来
要加限速。

## 目标 / 非目标

**目标**:

1. `about_to_wait` 的 interval 选择从嵌套 if/else 改成一个 `[(bool,
   Duration); 3]` 列表 + 取激活项里最小的 interval——以后加第 4 个周期性
   关注点,只需要在列表里加一行,不用碰现有分支。
2. `poll_todo_if_visible` 补上与 `toggle_blink` 同款的自限速(按
   `last_todo_poll_at` 判断是否已过 `TODO_POLL_INTERVAL`,未到点直接
   return,不做 `stat()`)——即使现在靠 mtime 检查已经安全,补上限速让
   "每个周期性函数自己对被更快节奏带跑免疫"这条约定对三者都成立、都
   显式可见,而不是"两个真的限速了、一个靠巧合安全"这种不对称状态。
3. `TODO_POLL_INTERVAL` 加 `pub(crate)`(比照 `BLINK_INTERVAL` 已有的
   处理),`app.rs` 才能在 `poll_todo_if_visible` 里引用它。

**非目标**:

- **不引入 trait/注册机制**(brainstorming 已确认——当前只有 3 个具体
  实例,`PeriodicWake` 之类的抽象过度设计,YAGNI)。
- **不改变任何可观察行为**。闪烁相位、悬停动画曲线、Todo 面板刷新时机,
  用户侧感知不到任何差异——这是纯调度层重构。
- **不改 `advance_hover_anims`**——它是最快节奏的所有者,不存在需要
  限速的场景。
- **不改 `new_events` 无条件调用三个函数这个模式本身**——保留"每次
  `ResumeTimeReached` 都把三个函数各调一次,各自决定要不要真正做事"
  这个设计,只是让"要不要真正做事"这个判断对三者都是显式、一致的自
  限速,而不是隐式判断。

## 架构

### 1. `poll_todo_if_visible` 补自限速(`app.rs`)

`App` struct 里 `last_blink_at` 字段(`app.rs:1161`)旁边加一个同类字段:

```rust
    /// 上次真正执行 Todo 面板磁盘轮询(`poll_todo_if_visible`)的时刻,
    /// 用法与 `last_blink_at` 一致:按 `TODO_POLL_INTERVAL` 自限速,未到
    /// 点的调用直接 no-op。现在靠 mtime 检查已经安全(没变化就早退,见
    /// 该方法文档),这里补上限速是为了让"周期性函数自己对被更快唤醒
    /// 节奏带跑免疫"这条约定对全部三个周期性关注点(闪烁/悬停动画/
    /// Todo 轮询)都显式成立,不留一个"靠巧合安全"的例外。
    last_todo_poll_at: std::time::Instant,
```

构造点(`app.rs:1470` `last_blink_at: std::time::Instant::now(),` 那一行
旁边)加:

```rust
            last_todo_poll_at: std::time::Instant::now(),
```

`poll_todo_if_visible`(`app.rs:1652`)本体加限速判断,插在现有
`if !self.todo_panel_visible() { return; }` 之后、真正 `stat()` 之前:

```rust
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

### 2. `TODO_POLL_INTERVAL` 加 `pub(crate)`(`main.rs:231`)

```rust
/// Todo 面板可见时轮询 `.dozer/todo.md` 的间隔,兼顾响应与省电。
/// `pub(crate)`——`App::poll_todo_if_visible` 也要用它把自己限速到这个
/// 节奏(同 `BLINK_INTERVAL` 的处理,理由见该常量文档)。
pub(crate) const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);
```

### 3. `about_to_wait` 改成列表+取最小值(`main.rs:1266`)

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

`new_events`(`main.rs:1242`)本体不改——仍然是"`ResumeTimeReached` 时
三个函数各调一次,各自决定要不要真正做事"这个既有模式,只是现在三个
函数都显式自限速了。

## 错误处理

不适用——`Duration`/`Instant` 比较不会失败,`std::fs::metadata` 已有的
`.ok()` 降级处理不变。

## 测试策略

- `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin
  dozer && cargo fmt -p dozer-app -- --check` 干净通过,`cargo test` 数字
  与开工前一致(484 passed / 2 failed,两个既有失败)。
- 人工验证(沿用当天已用过多次、独立命名临时二进制的方法,不碰用户
  正式 app):打开一个项目、切到 Todo 面板并保持可见,同时用鼠标在
  icon 按钮上来回悬停制造密集唤醒,观察 Todo 面板不应该有任何肉眼可见
  的异常刷新/闪烁——这是这次重构唯一有"外部可观察副作用风险"的路径
  (`poll_todo_if_visible` 新增了一个提前 return 分支),其余改动
  (`about_to_wait` 的调度逻辑、`toggle_blink` 未改动)没有新的可观察面。

## 排期备注

这次改动只碰 `app.rs`(`App` struct 一个新字段 + `poll_todo_if_visible`
一处改动)和 `main.rs`(一个常量加 `pub(crate)` + `about_to_wait` 重写),
与"App::update 抽方法"那份计划(碰的是 `App::update` 内部)、"tab/icon
按钮共享组件"(已合并)touch 的都是不同函数,不冲突,可以独立开分支
并行做。
