# Dozer 运行时优化实现审核与修复结果

日期：2026-11-02。审核范围：`a9e02c5d` 之上的当前工作区修改及新增文件。

依据：[设计规格](../superpowers/specs/2026-11-02-dozer-runtime-optimization-design.md)、[实施计划](../superpowers/plans/2026-11-02-dozer-runtime-optimization.md)、[实施记录](2026-11-02-dozer-runtime-optimization-implementation.md)。

结论：上一版审核提出的 9 项实现问题（R1–R9）已针对代码路径修复，并补充了对应回归；P1 项按当前可见代码路径不再成立。整套规格仍未完成：T6/T7 终端 V2 与端到端背压、T9 分批摄取、T11 完整 shutdown/BackfillState、T14/T15/A16 等仍未交付，因此不能宣布全套验收通过。R6/R7 仍缺完整 UDS 集成压力测试，下文单独标注。

## 修复状态

| 编号 | 原优先级 | 当前状态 | 主要证据 |
|---|---|---|---|
| R1 | P1 | 已修复 | 生产保存传递真实项目根；父级外链测试 |
| R2 | P1 | 已修复 | process_runner/server 等待计数改为 RAII |
| R3 | P1 | 已修复 | 后续等待阶段持续监听 cancel；leader 先退取消测试 |
| R4 | P1 | 已修复 | waiter 等 PTY 排空（至多 1s）后再广播 Exited |
| R5 | P2 | 已修复 | server/client 改为有状态 FrameReader；取消半帧测试 |
| R6 | P2 | 已修复 | 异步启动始终经过 attempts 准入，不再提前返回 latest |
| R7 | P2 | 部分修复 | 普通连接 120 + 控制连接 8 + 首帧 5s；缺完整 UDS 压力测试 |
| R8 | P2 | 已修复 | context 最多扫描 1025 个标量；长行密集命中测试 |
| R9 | P2 | 已修复 | 默认锁根移出项目；UI 接收并展示 durability unknown |

## 逐项修复

### R1 · 预览保存的父目录符号链接逃逸

- `save_text_with_context` 增加 `project_root` 参数：存在项目根时，目标必须是其词法相对路径，并由 `ProjectRoot` 逐级 `O_NOFOLLOW` 打开/写入；父目录符号链接返回错误。
- `dozer-panel-preview::Effect::SaveFile` 现在携带 `ctx.project_root`，生产保存不再从“目标父目录”重新推导授权根。
- `save_text_atomic` 仍作为兼容包装保留，但文档明确它缺少项目根上下文，不能用于生产预览保存。
- 回归：`save_with_project_root_rejects_parent_symlink_escape`，断言外部文件字节不变。

### R2 · 排队取消泄漏等待计数

- `process_runner::Limiter` 和 `server::RuntimeLimiter` 的排队计数改为 RAII `WaitingGuard`：future 被取消、abort 或 panic 时都会减计数。
- `process_runner` 的全局许可仍持有到进程组回收结束。
- 回归：`waiting_guard_releases_on_drop_and_enforces_limit`；取消排队路径由既有 cancel/timeout 测试覆盖。

### R3 · 后半段等待不再响应取消

- `process_runner` 在 leader 已退出但 IO 未排空、或 IO 已完成但 wait 未完成的第二阶段，继续在 `tokio::select!` 中监听 cancel 与 absolute deadline，走同一 `cleanup_group` 路径。
- 回归：`cancel_after_leader_exit_still_terminates_descendants`，验证 leader 先退/后代持管道时取消能及时返回 `Cancelled`。

### R4 · Exited 早于尾部输出

- PTY 读线程在 EOF 时设置 `output_drained`；独立 waiter 先记录真实退出码与退出时间，再等待读线程排空，最多 1s，随后才广播 `Exited`。超过 1s 的极端情况仍结束输出流，不无限等待。
- 迟到观察者仍可通过 `Session::exit_status()` 读取权威退出状态。
- 回归：`exited_event_arrives_after_tail_output`，断言尾部输出事件先于 `Exited`。

### R5 · 有界帧读取取消丢半帧

- server 和 client 的 `read_frame` 改为有状态 `FrameReader`：未完成字节保存在连接级 `pending`，`fill_buf` 后立即 `consume` 到 `pending`；select 取消后下一次读取从半帧继续。
- 单帧上限保持 client→server 12MiB、server→client 16MiB；无换行超限与 UTF-8 错误仍明确失败。
- 回归：server/client 各自 `frame_reader_preserves_partial_frame_when_cancelled`。

### R6 · 异步执行快速路径绕过幂等检查

- `start_todo_execution` 不再在 `in_flight` 已占用时直接返回 latest attempt；所有请求先经过 `TaskExecutionStore::admit`。
- 相同 `idempotency_key` + 相同输入指纹返回原 attempt；同 key 不同输入返回 `IdempotencyConflict`；不同 key 且已有 Running attempt 返回 InFlight。
- 若 attempts 准入成功但内存 `in_flight` 已被旧同步路径占用，新 attempt 会 Drop 为 Interrupted，而不是并发启动第二条执行。
- 尚未补 server handler 级并发 UDS 集成测试；当前由 store 单元测试与 handler 代码路径静态复核覆盖。

### R7 · 连接耗尽时控制入口不可达

- 普通连接名额改为 120，额外保留 8 个控制连接名额；普通名额耗尽时，新连接使用控制名额并标记 `control_only`。
- `control_only` 连接的首条请求必须是 `Shutdown`，否则返回 Busy 并关闭；首帧读取增加 5s 期限，防止只占连接不发请求。
- 工作 semaphore 之外仍有 4 个控制请求名额。
- 回归：`control_connection_reserve_survives_normal_connection_exhaustion`。
- 尚未补“占满普通连接后仍能建立真实 UDS 控制请求并完成停止”的集成压力测试。

### R8 · locate 每次命中仍扫描整条 context 长行

- `bounded_context` 只遍历到第 1025 个 Unicode 标量即可判定截断，不再对整条长行做 `chars().count()`。
- 回归：`locate_truncates_context_without_rescanning_whole_long_line`，20k 字符长行、带截断标记且 context 长度受预算约束。

### R9 · 产品保存仍用兼容包装、锁文件污染项目、持久性警告丢失

- `dozer-file-io` 的默认锁根改为当前用户隔离的临时目录（`dozer-write-locks-<uid>`），app/daemon 同一用户共享，不再在用户项目写 `.dozer-write-*.lock`。
- 生产预览保存走 `save_text_with_context(..., project_root, lock_root)`；`SaveFinished` 携带 `SaveOutcome`。
- `CommittedDurabilityUnknown` 仍视为“内容已提交”（清脏标），同时在页签显示 `text_save.durability_unknown` 提示；不再静默丢弃。
- 回归：`save_with_context_does_not_create_lock_files_in_project`、`save_finished_durability_unknown_marks_saved_with_warning`。

## 验证结果

本机使用 `CARGO_TARGET_DIR=$PWD/target/runtime-review`、`--offline`：

| 命令 | 结果 |
|---|---|
| `cargo test -p dozerd --lib` | 515 passed, 3 ignored |
| `cargo test -p dozer-client --lib` | 63 passed |
| `cargo test -p dozer-panel-preview --lib` | 410 passed |
| `cargo test -p dozer-preview-core` | 166 passed |
| `cargo test -p dozer-file-io` | 22 passed |
| `cargo check -p dozerd -p dozer-client -p dozer-mcp -p dozer-app -p dozer-panel-preview --all-targets` | 通过 |
| `cargo fmt --all -- --check` / `git diff --check` | 通过 |
| 面板边界 / 日志范围脚本 | 通过 |

## 仍未完成

- **T6/T7**：终端 V2 offset/epoch/Resume/Reset、恢复状态机、GUI ack 与端到端字节背压未落地；旧 `Lagged` 后停更问题仍在。
- **T4 UI**：Todo 面板未展示 Failed/Interrupted 与显式重试；能力协商未落地。
- **T9**：transcript 分批摄取、锁外解析、源 generation/rescan-needed、prompt/argv 预算未完成。
- **T11**：可取消有界扫描、BackfillState、涵盖 attempt/bridge/worker/socket 的完整 shutdown coordinator 未完成。
- **T12**：Busy/Limit/Timeout 稳定类型映射、server 侧操作期限、真实 128 连接/慢客户端压力测试未完成。
- **T14/T15/A16**：结构整理、完整兼容矩阵、release 基准与最终验收未完成。
- R6/R7 的 handler 级 UDS 集成测试仍需补齐；本次不再以 store 单测代替完整集成验证。

plan 顶部状态、checkbox 与实施记录已按当前实际完成度同步；未完成项仍保持未勾选。本次验证未修改 Cargo.lock 的本机 path patch 漂移（验证后已还原）。
