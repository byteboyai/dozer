# bytehost 优化 Tier3：结构性重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 bytehost 内部五项结构性重构：Core 锁模型、wry 执行器抽公共、纯逻辑下沉、自旋锁改 Notify、thiserror 收敛（可选）。

**Architecture:** bytehost 一条分支；Task 1/3 破坏公开 API 或 crate 边界，发版后 dozer/digger 各一条分支同日升级。

**Tech Stack:** Rust、tokio、wry、iced、cargo test/clippy。

**Spec:** [`../specs/2026-10-30-bytehost-opt-tier3-structural-refactor-design.md`](../specs/2026-10-30-bytehost-opt-tier3-structural-refactor-design.md)

## Global Constraints

- 依赖一律用 git tag；本地联调 `[patch]` 只放不提交的 `.cargo/config.toml`；提交前还原被它改动的 `Cargo.lock` 行。
- dozer 与 digger 同日升级 bytehost / byteui（ADR）；涉及 bytehost 公开 API 改动的梯队需协调两产品同步消费，发版与推送先问用户。
- bytehost 测试需 `--features server` / `--all-features`。
- 分支基线：各仓库的分支基于 `origin/main`（不要基于本地 main，里面可能有别的会话未推送的提交）；推送用 `git push origin <分支>:main`（快进）。
- 推送、打 tag、合并 main 每次先问用户。
- wire 形状只追加不修改；涉及 bytehost DTO 时不破坏旧客户端反序列化。
- 新 UI 复用统一组件（byteui），不在产品里复制按钮样式。

## Review Focus

1. Core 锁拆分后无死锁/竞态：子锁粒度正确，不引入新的锁序问题。
2. wry 行为等价：抽公共 builder 后 devtools/下载/新窗口策略与四份原实现逐条一致。
3. crate 边界不引入循环依赖：panes/hover/layout 下沉后 core 不反向依赖 desktop。
4. 公开 API 破坏面：Deref 移除与 crate 边界变更的消费方引用全部覆盖（dozer/digger grep 零残留）。
5. thiserror 是否引入：评审裁决后再执行 Task 5。

## 范围说明

| 项 | 处置 |
|---|---|
| gateway 性能（Arc\<Site\>/spawn_blocking） | **不在本梯队**：归 Tier4 |
| 消费侧 adapter 去重 | **不在本梯队**：归 Tier2 |
| thiserror 引入 | **需评审裁决**：Task 5 默认跳过，批准后执行 |

---

## Task 1(bytehost): Core 锁模型重构

**Files:** Modify: `crates/bytehost-apps/src/manager.rs`（Core 定义、Deref impl、acquire、5 处 dead_code 字段）；消费方引用点（dozer/digger，Grep 确定）。

**Interfaces:**
- 移除 `impl Deref for AppManager { type Target = Core }`。
- `Core` 收为 `pub(crate)`；拆为 `install_state: Mutex<InstallState>`、`start_state: Mutex<StartState>`、`runtime_state: Mutex<RuntimeState>`（具体子锁边界按 manager.rs 实际字段分组）。
- AppManager 对外提供显式方法替代 Deref 访问（如 `fn install_state(&self) -> ...`）。

- [ ] **Step 1: 核对现状**：读 manager.rs:181-218（Core 字段与 Mutex）、:444（Deref）、:192/200/207/210（dead_code）；Grep dozer/digger 中经 Deref 访问 Core 的引用点。
- [ ] **Step 2: 写失败测试**：对 AppManager 的 install/start/runtime 并发操作写测试，断言无全局串行化（如两个不同职责操作可并行）。运行确认当前因单锁而串行/或测试设计能暴露问题。
- [ ] **Step 3: 实现**：去 Deref、Core 收 pub(crate)、拆子锁、删/接 dead_code 字段、补显式方法。
- [ ] **Step 4: 运行**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check && bash scripts/check-deps.sh` 全绿。
- [ ] **Step 5: 消费方同步**：dozer/digger 分支更新 Deref 引用为显式方法；`cargo test` 全绿。
- [ ] **Step 6: Commit**（bytehost）：`git commit -am "refactor(apps)!: remove Deref to Core, split global mutex into sub-locks"`

---

## Task 2(bytehost): wry 执行器抽公共

**Files:** Create: `crates/bytehost-desktop/src/wry_common.rs`（restricted builder + HandleRegistry）；Modify: `panel.rs`、`app_webview.rs`、`native_child.rs`、`web_child/pool.rs`（改用公共模块）；合并三份钉死测试。

**Interfaces:**
- `pub(crate) fn wry_restricted_builder() -> WebViewBuilder`：统一关 devtools/拒下载/拒新窗口。
- `pub(crate) struct HandleRegistry`：统一 thread_local 句柄表，带 slot/id 索引；`with_handles` 访问器收敛为一份。

- [ ] **Step 1: 核对四份实现**：并排读 panel.rs:42、app_webview.rs:76、native_child.rs:136、web_child/pool.rs:105 的句柄表与策略代码；确认三份钉死测试（app_webview.rs:254、native_child_tests.rs:176、pool.rs:644）的断言一致。
- [ ] **Step 2: 写行为等价测试**：对四个执行器各写 webview 创建测试，断言策略（devtools/下载/新窗口）与当前一致。运行确认通过（baseline）。
- [ ] **Step 3: 实现**：抽 wry_common.rs；四处改用；三份钉死测试合并为一份参数化测试。
- [ ] **Step 4: 运行**：Step 2 测试仍绿；`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`。
- [ ] **Step 5: Commit**：`git commit -am "refactor(desktop): extract shared wry restricted builder and handle registry"`

---

## Task 3(bytehost): 纯逻辑下沉出 desktop

**Files:** Modify: `crates/bytehost-desktop/src/panes.rs`、`hover.rs`、`layout.rs`（移至 core 或新 crate）；`crates/bytehost-core/src/lib.rs`（新增模块）；`crates/bytehost-chrome/Cargo.toml`（去 wry/winit/objc2 直接依赖，改经 core）；desktop 内部引用点。

**Interfaces:**
- `bytehost_core::panes`、`bytehost_core::hover`、`bytehost_core::layout`（或新 crate `bytehost-layout`）。
- chrome 的 `PaneStore` 引用从 `bytehost_desktop::panes` 改为 `bytehost_core::panes`。

- [ ] **Step 1: 核对依赖**：读 panes.rs/hover.rs/layout.rs 的 use 语句，确认仅 std+core+serde；Grep desktop 内部与 chrome 对这些模块的引用。
- [ ] **Step 2: 写失败测试**：chrome 的 PaneStore 测试（若已有则复用），断言其不依赖 wry/winit/objc2。
- [ ] **Step 3: 实现**：移动三个模块到 core；更新 desktop/chrome 的 import 与 Cargo.toml；chrome 去掉 wry/winit/objc2 直接依赖。
- [ ] **Step 4: 运行**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && bash scripts/check-deps.sh`；`cargo tree -p bytehost-chrome | grep -E 'wry|winit|objc2'` 应零结果。
- [ ] **Step 5: 消费方同步**：dozer/digger 若直接引用 desktop::panes/hover/layout，改为 core 路径；`cargo test` 全绿。
- [ ] **Step 6: Commit**：`git commit -am "refactor(core)!: sink panes/hover/layout from desktop to core; chrome no longer pulls wry/winit/objc2"`

---

## Task 4(bytehost): 自旋锁改 Notify

**Files:** Modify: `crates/bytehost-apps/src/manager.rs`（acquire 方法，约 :272-287）。

**Interfaces:** 无公开 API 变化。内部等待机制从 sleep 轮询改为 Notify/Condvar。

- [ ] **Step 1: 核对现状**：读 acquire() 的 try_guard + 5ms sleep 循环；确认调用上下文（async 还是 sync）。
- [ ] **Step 2: 写失败测试**：测量 acquire 等待延迟（如持锁方释放后等待方获得锁的时间），断言 <1ms。运行确认当前因 5ms 轮询而失败。
- [ ] **Step 3: 实现**：async 上下文用 `tokio::sync::Notify`（释放时 notify_one，等待方 notified().await）；sync 上下文用 `std::sync::Condvar`。
- [ ] **Step 4: 运行**：Step 2 测试通过；`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`。
- [ ] **Step 5: Commit**：`git commit -am "perf(apps): replace 5ms spin-wait with Notify/Condvar in acquire()"`

---

## Task 5(bytehost): thiserror 收敛（需评审批准）

**Files:** Modify: `Cargo.toml`（workspace 加 thiserror）；27 个 error enum 所在文件（重点 `manager.rs:49-162`）。

**Interfaces:** error enum 改为 `#[derive(thiserror::Error)]`，手写 Display/Error/From impl 删除。公开 error 类型名与变体不变。

- [ ] **Step 1: 评审裁决**：确认是否引入 thiserror 新依赖。未批准则跳过本 Task。
- [ ] **Step 2: 盘点**：Grep 全库手写 `impl.*Display.*for.*Error` 与 `impl std::error::Error`，列出 27 个 enum。
- [ ] **Step 3: 逐 crate 改造**：从 ManagerError 开始，改为 derive；运行 `cargo test --all-features` 确认错误消息与 From 转换不变。
- [ ] **Step 4: 运行**：全库 `cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check`。
- [ ] **Step 5: Commit**：`git commit -am "refactor: adopt thiserror for error enums, remove hand-written boilerplate"`

---

## Task 6(bytehost): 发版 + 消费方同日升级（先问用户）

- [ ] **Step 1:** bytehost 全量验证：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check && bash scripts/check-deps.sh`（跑两遍确认无偶发）。
- [ ] **Step 2:** 给用户审阅；批准后合并 main、打新 tag、推送；`git ls-remote --tags origin <新tag>` 核对。
- [ ] **Step 3:** dozer/digger 分支升 tag、更新 Task 1/3 的消费点、`cargo test` 全绿；推送先问用户。

---

## Self-Review

- **Spec 覆盖：** T3-12→Task1、T3-13→Task2、T3-14→Task3、T3-15→Task4、T3-16→Task5。
- **破坏面：** Task 1（Deref 移除）与 Task 3（crate 边界）破坏公开 API，消费方同步步骤已内置；Task 2/4/5 为内部改动。
- **风险点：** Task 1 子锁拆分需确认无新锁序死锁；Task 3 需确认 core 不反向依赖 desktop；Task 5 需评审批准。

## 决策记录

- **Task1**:`impl Deref for AppManager` 已移除,`Core` 收为 `pub(crate)`;消费方无引用
  (Core 的方法本来就是 `pub(crate)`,Deref 对外等于没暴露任何可用 API)。
  **锁拆成"per-app 写锁(64 分片)+ 全局生命周期 RwLock"**:per-app 操作持全局读锁 +
  该应用分片锁,同一应用串行、不同应用并行;`reconcile`/`suspend_all` 持全局写锁,
  与所有 per-app 操作互斥。监管线程仍是"try 锁 + 查 cancel + 等通知",所以 `stop`
  (持有该应用锁)与 `suspend_all`(持有全局写锁)`join` 它们不会死锁。
  新增测试:不同应用不互相阻塞、同一应用仍串行、全局操作排除 per-app 操作、
  释放后 <1ms 拿到锁、取消立刻放弃。4 个字段上陈旧的 `#[allow(dead_code)]` 已删除
  (全部实际在用)。
- **Task2**:`wry_common::{apply_restricted, HandleRegistry}`。受限策略在调用方 `configure` **之后**
  应用(产品不能把 devtools/下载/新窗口放开);`panel` 是可信内部 HTML,刻意保留 wry 默认。
  三份源码钉死测试合并为 `wry_common` 的一份参数化测试。
- **Task3**:`panes`/`hover`/`layout_migrate` 下沉 `bytehost-core`。core 已有 `layout`(DTO)模块,
  故迁移逻辑改名 `layout_migrate`;`bytehost-desktop` 保留同名转发(`layout` 模块转发到
  `layout_migrate`),既有消费方路径不变。`bytehost-chrome` 不再依赖 desktop,
  `cargo tree -p bytehost-chrome | grep -E 'wry|winit'` 为空;objc2 仍直接使用(交通灯按钮)。
- **Task4**:`Mutex + Condvar` + `WriteGuard`(Drop 时 notify_all);取消时 `wake_lock_waiters`
  主动唤醒,避免"持锁 join"死锁。新增两个测试:释放后 <1ms 拿到锁、取消立即放弃。
- **Task5**:跳过(thiserror 未引入)。
- 提交:bytehost `2242b1e`/`35aec3b`/`a2059b6`/`e30eeba`/`8cb8ed1`;
  锁拆分 `fad7715`(tag `v0.4.19`)。
