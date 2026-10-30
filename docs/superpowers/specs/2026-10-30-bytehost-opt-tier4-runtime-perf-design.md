# bytehost 优化 Tier4：运行时性能 规格

> 日期：2026-10-30。状态：草案，尚未实施。实施计划见 [plan](../plans/2026-10-30-bytehost-opt-tier4-runtime-perf.md)。
> 本梯队聚焦 bytehost gateway 热路径的运行时性能，改动必须有基准测量佐证。

## 1. 目标与边界

消除 gateway 每请求的不必要内存分配与调度开销：`Site` 的每请求 clone、`spawn_blocking` 的必要性评估。

**不在本梯队**：结构性重构（Tier3）、去重（Tier2）、治理（Tier5）。性能改动为内部实现，不改公开 API 或 wire。

## 2. 已核对的代码基线

| 位置 | 当前事实 | 性能责任 |
| --- | --- | --- |
| `bytehost-apps/src/gateway/mod.rs:95-97`（serve_static） | 每 HTTP 请求 `.cloned()` 整个 `Site`（含 PathBuf+String 两次分配） | bytehost-apps：改 `Arc<Site>` |
| `bytehost-apps/src/gateway/mod.rs:330` | 每请求 `spawn_blocking`；需核对闭包内实际阻塞操作（文件 I/O？纯 CPU？） | bytehost-apps：评估后保留或去除 |
| gateway 路由 | 已用独立 `RwLock` 而非全局锁（设计正确，不改） | — |

**待核对项**（实施前必须确认）：`Site` 结构体完整定义与字段成本；`spawn_blocking` 闭包内容；gateway HTTP 库（hyper/axum/其它）；每 app 一个 gateway 还是共享；是否已有 benchmark。

## 3. 方案

### 3.1 Arc\<Site\> 替代每请求 clone

路由表存 `Arc<Site>` 而非 `Site`；`serve_static` 接收 `Arc<Site>` 参数，消除每请求 PathBuf+String 分配。若 `Site` 需运行期更新（如配置热重载），用 `arc-swap::ArcSwap<Site>` 或 `RwLock<Arc<Site>>` 保证读写不阻塞。核对 `Site` 的可变性需求后选定方案。

### 3.2 spawn_blocking 必要性评估

读 `mod.rs:330` 闭包内容：
- 若全部为文件 I/O（读静态资源）：`spawn_blocking` 合理，保留；仅做 3.1 的分配优化。
- 若含纯 CPU 或误判为阻塞的 async 操作：去除 `spawn_blocking`，改直接 async 处理，降低调度开销。
- 若混合：拆分，仅文件 I/O 部分走 blocking pool。

**诚实结论优先**：若核对后确认 spawn_blocking 全部合理，spec/plan 写明"调查结论：保留"，不为改而改。

## 4. 兼容与发布

- 两项均为 bytehost-apps 内部实现，不改公开 API、wire、DTO。
- bytehost 发版 + dozer/digger 同日升级；回滚锁旧 tag。
- 性能验收需基准对比，不接受"应该更快"的定性声明。

## 5. 验收与完成定义

| ID | 场景 | 必须观察到的结果 |
| --- | --- | --- |
| T4-17 | gateway 静态资源压测（wrk/criterion） | 每请求堆分配次数下降（Site clone 消除）；p50/p99 延迟不劣化；吞吐 ≥ baseline |
| T4-18 | spawn_blocking 调查 | 书面结论：保留/去除/拆分；若去除，executor 无阻塞卡顿；若保留，记录理由 |
| T4-B | 基准工具 | 实施前建立可复现的 gateway benchmark（criterion bench 或 wrk 脚本），baseline 数据记录在决策记录 |
