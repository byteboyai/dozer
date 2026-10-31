# bytehost 优化 Tier3：结构性重构 规格

> 日期：2026-10-30。状态：草案，尚未实施，需评审。实施计划见 [plan](../plans/2026-10-30-bytehost-opt-tier3-structural-refactor.md)。
> 本梯队是 bytehost 内部结构性重构，影响所有消费方，需 bytehost 发版 + dozer/digger 同日升级。

## 1. 目标与边界

解决 bytehost 内部五项结构性债务：Core 全局单写者锁与抽象泄漏、desktop 四份 wry 执行器重复、纯逻辑被关在 desktop、自旋等锁、error enum 样板。

**不在本梯队**：消费侧去重（Tier2）、gateway 性能（Tier4）、跨仓治理（Tier5）。本梯队改动 bytehost 内部结构，部分涉及公开 API 或 crate 边界，需评审后实施。

## 2. 已核对的代码基线

| 位置 | 当前事实 | 重构责任 | 破坏公开 API |
| --- | --- | --- | --- |
| `bytehost-apps/src/manager.rs:181-218,444` | `Core` 用一把 `Mutex<()>` 串行化所有状态变更；`impl Deref for AppManager { type Target = Core }` 迫使 Core `pub`；5 处 `#[allow(dead_code)]` 字段（:192/200/207/210） | bytehost-apps：去 Deref、Core 收 pub(crate)、拆子锁 | 是（Deref 移除） |
| `bytehost-desktop/src/panel.rs:42`、`app_webview.rs:76`、`native_child.rs:136`、`web_child/pool.rs:105` | 四套 thread_local 句柄表 + with_handles；"关 devtools/拒下载/拒新窗口"三份独立写死 + 三份同型钉死测试 | bytehost-desktop：抽 wry_restricted_builder + 统一句柄注册表 | 否（内部） |
| `bytehost-desktop/src/panes.rs`、`hover.rs`、`layout.rs` | 只依赖 std+core+serde，却在 desktop；chrome 仅为 PaneStore 拉进 wry+winit+objc2 | bytehost：下沉至 core 或新纯 crate | 是（crate 边界） |
| `bytehost-apps/src/manager.rs:272-287` | acquire() 以 5ms thread::sleep 轮询 try_guard 规避死锁 | bytehost-apps：改 tokio::sync::Notify 或条件变量 | 否（内部） |
| 全库 27 个手写 error enum | 无 thiserror，每个手写 Display+Error+From；ManagerError 占 manager.rs:49-162 | bytehost：引入 thiserror 收敛（可选，风格取舍） | 否 |

## 3. 方案

### 3.1 Core 锁模型重构

去掉 `impl Deref for AppManager { type Target = Core }`，把 `Core` 收为 `pub(crate)`。按职责拆子锁：`install_mutex`、`start_mutex`、`runtime_mutex`，消除全局单写者瓶颈。5 处 dead_code 字段核对后删除或接入。消费方（dozer/digger）若经 Deref 访问 Core 字段，改为 AppManager 提供的显式方法。

### 3.2 wry 执行器抽公共

抽 `fn wry_restricted_builder() -> WebViewBuilder`：统一"关 devtools/拒下载/拒新窗口"策略。四份 thread_local 句柄表合并为统一注册表（`HandleRegistry`，带 slot/id 索引）。三份钉死测试合并为一份参数化测试。

### 3.3 纯逻辑下沉

`panes.rs`/`hover.rs`/`layout.rs` 下沉到 `bytehost-core`（或新建 `bytehost-layout` 纯 crate）。chrome 对 PaneStore 的依赖改为从 core 引入，不再拉 wry/winit/objc2。需核对 desktop 内部对这些模块的引用并更新 import。

### 3.4 自旋锁改 Notify

`acquire()` 的 5ms sleep 轮询改为 `tokio::sync::Notify`：持锁方释放时 notify_one，等待方 notified().await。若上下文非 async 则用 `std::sync::Condvar`。

### 3.5 thiserror 收敛（可选）

引入 thiserror，27 个手写 error enum 改为 `#[derive(thiserror::Error)]`。ManagerError 从 113 行降至约 30 行。**此项为风格取舍，需评审确认是否引入新依赖。**

## 4. 兼容与发布

- 3.1（Deref 移除）与 3.3（crate 边界）破坏公开 API/crate 结构：bytehost 发 minor 或 major tag → dozer/digger 同日升级 → 验证编译与测试。
- 3.2/3.4/3.5 为内部实现，不改 wire 或公开签名，随 bytehost 正常发版。
- 回滚策略：发版前打 tag，消费方锁旧 tag 即可回滚。
- wire 形状不受影响（本梯队不改 DTO）。

## 5. 验收与完成定义

| ID | 场景 | 必须观察到的结果 |
| --- | --- | --- |
| T3-12 | AppManager 并发 install/start/runtime | 无全局 Mutex<()> 串行化；子锁独立；dead_code 字段清零 |
| T3-13 | desktop 四执行器创建 webview | 策略一致（devtools/下载/新窗口）；句柄注册表统一；钉死测试仅一份 |
| T3-14 | chrome 依赖树 | `cargo tree -p bytehost-chrome` 不含 wry/winit/objc2（仅经 core 取 PaneStore） |
| T3-15 | acquire() 等待 | 无 5ms sleep 轮询；Notify/Condvar 唤醒延迟 <1ms |
| T3-16 | error enum 样板 | thiserror 引入后 ManagerError ≤40 行；全库手写 Display/Error impl 清零（若评审通过） |
