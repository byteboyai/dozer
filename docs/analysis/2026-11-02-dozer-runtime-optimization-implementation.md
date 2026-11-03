# Dozer 运行时优化实施记录（第一批）

> 日期：2026-11-02。
> 此文件保留第一批历史快照；当前状态与新增验证以[后续补齐与验收记录](2026-11-02-dozer-runtime-optimization-completion.md)为准。
> 计划：[2026-11-02-dozer-runtime-optimization.md](../superpowers/plans/2026-11-02-dozer-runtime-optimization.md)。
> 规格：[2026-11-02-dozer-runtime-optimization-design.md](../superpowers/specs/2026-11-02-dozer-runtime-optimization-design.md)。

本轮按计划先落地 P1 正确性修复：受控文件 IO、原子保存与元数据保持、统一受管进程执行器；同时完成 locate 旧接口的线性扫描/显式预算和核心 CI job。T4–T9、T10 V2、T11–T12、T14–T15 尚未实施，下文“未完成”部分逐项列出，不能用本轮结果声称整套计划完成。

## 基线

最新修复补充：第二轮审核 N1–N4 已关闭，详见[第二轮审核的修复与复测](2026-11-02-dozer-runtime-optimization-rereview.md#n1n4-修复与复测)。新增真实 UDS 回归已加入 CI；这不表示原计划全部完成。

- 代码基线：`a9e02c5d`（`chore(release): 0.5.314`）。
- 工具链：`cargo 1.98.1` / `rustc 1.98.1`（aarch64-apple-darwin）。
- 构建目录：本机验证使用仓库内 `target/runtime-review`，避免共享 target 写锁影响；未修改共享目录权限。
- `.cargo/config.toml` 有本机 `byteui` / `bytegit` path patch。验证时 Cargo 会尝试从 `Cargo.lock` 去掉对应 `source` 行；这些 patch 漂移已手工还原，提交的锁文件只保留新增 `dozer-file-io` 与依赖边。带本地 patch 的机器不适合直接跑 `--locked`；干净 checkout/CI 使用 `--locked`。

## 已完成

### T1 项目文件受控打开（新增 `dozer-file-io`）

文件：`crates/dozer-file-io/{Cargo.toml,src/lib.rs,src/path.rs,src/metadata.rs,src/lock.rs,src/write.rs}`；`crates/dozerd/src/file_mutation.rs`；`crates/dozer-preview-core/src/text_save.rs`。

- 项目根先 `canonicalize` 并固定目录句柄；路径必须是项目内非空相对路径，拒绝绝对路径、`..` 和根/前缀，跳过无意义的 `.`。
- 逐级 `openat(O_DIRECTORY|O_NOFOLLOW)`、末级 `openat(O_NOFOLLOW)`，并检查是普通文件。读、locate、精确编辑、预览保存共用同一套入口；末级和父级符号链接都返回 `SymlinkRejected`。
- 错误类型：`PathEscape` / `SymlinkRejected` / `NotFound` / `UnsupportedFile` / `UnsupportedMetadata` / `Conflict` / `LockTimeout` / `Io`。`file_mutation::LocateError` 映射到现有 wire 形状。
- 文档明确：目录句柄固定授权时的对象身份；同用户恶意进程重挂载/搬移整个已打开目录不在保证内。

### T2 带元数据与提交结果的原子写入

- `write_atomic` 在目标父目录（或调用方传入的产品锁根）取得跨进程 `flock` 写锁；锁文件名由目标键的稳定 FNV 哈希生成，锁文件不 unlink。
- 读取基线 `dev/ino/size/mtime/mode/uid/gid/nlink` 形成 revision；提交前在锁内重新打开并复核身份与 revision。
- 同目录 `create_new` 临时文件（初始 0600）→ 写入/fsync → 恢复 mode（保留 `0755/0644/0600`，清除 setuid/setgid）→ 恢复 owner/group → 复制 xattr（macOS 额外通过原生 ACL API 复制扩展 ACL）→ `renameat` → 父目录 fsync → 清理临时文件。
- 硬链接目标（`nlink > 1`）明确拒绝；必要元数据无法保留时在 rename 前失败。
- 提交结果区分 `Committed` / `CommittedDurabilityUnknown` / `NotCommitted`。目录 fsync 失败的已提交内容不会被上层自动重写。
- 预览保存只采样前 64KiB 探测 BOM/换行；只有 `NotFound` 才回落默认格式，权限/IO 错误不会当成新文件。
- 兼容包装 `save_text_atomic` 保留；新增 `save_text_with_context` 暴露锁根与提交结果。daemon 精确编辑与预览保存共用 `dozer-file-io`。

### T3 受管进程执行器（新增 `process_runner`）

文件：`crates/dozerd/src/process_runner.rs`；接入 `summary_provider.rs`、`headless_agent.rs`、`group_adapter.rs`。

- 具名 `ExecutionSpec` / `ExecutionOutcome` / `ExecutionFailure`；absolute deadline 覆盖 spawn 后的 stdin、stdout、stderr 与 wait。
- stdin/stdout/stderr 并行泵送；stdin 写完显式关闭发送 EOF。stdout 超预算返回 `OutputLimit`，不把截断结果当成功；stderr 有界保留但继续排空。
- Unix 独立进程组；取消/超时/异常统一 TERM（最多 500ms）→ KILL → 显式 wait/reap；清理失败返回 `CleanupFailed`。`ProcessGroup` Drop 作为 task abort 兜底。
- 全局并发 4、等待 32；排队时间计入原 deadline；许可持有到进程组回收结束，超载返回 `Busy`。
- 总结、Todo（`process_task_headless`）、群聊已迁移；原有隔离 cwd、环境清理、权限参数、输出解析、总结 transient 分类与场景时长保持不变。

### T4 持久化 Todo 执行结果（后端，UI 待接入）

文件：新增 `crates/dozerd/src/task_execution.rs`；`task_processor.rs`、`task_poller.rs`、`daemon.rs`、`server.rs`、`transcripts/mod.rs`；`dozer-model/src/todo.rs`、`dozer-core/src/protocol.rs`、`dozer-services/src/lib.rs`、`dozer-client/src/lib.rs`。

- 新增独立 `task_execution_attempts` 表：attempt_id / todo_id / input_generation / input_fingerprint / idempotency_key / status / error_kind / error_message / started/finished / conversation_id / created_ms。独立 `CREATE TABLE/INDEX IF NOT EXISTS` 幂等迁移，不使用全局 `user_version`，不向 `TodoInfo` 加必填字段。
- 同一任务用 SQLite partial unique index 约束最多一个 `queued/running` attempt；另有内存 `InFlightGuard` RAII 占用，panic/取消/提前返回都会释放。`RunningAttempt` Drop 时未正常完成则持久化为 Interrupted。
- 执行前先写人类回合；`mark_input_persisted` 把 `input_generation` 更新为最新 human 回合索引。成功时在同一 SQLite 事务里写 AI 回合 + Succeeded；失败时写 `is_error=1` 的 AI 错误回合 + Failed，不会伪造成普通成功。
- 重启时 `recover_running()` 把遗留 Queued/Running 收敛为 Interrupted（error_kind=restart）。`auto_process_allowed` 阻止自动轮询重跑 Failed/Interrupted，除非出现比 attempt 记录更新的人类输入；旧历史没有 attempt 时沿用旧规则。
- 新 DTO `TaskExecutionInfo` / `TaskExecutionStatus`；协议新增 `StartTodoExecution`、`GetTodoExecution`、`Reply::TodoExecution`；`TodoService` 新方法提供保守默认实现；Client 提供对应异步/查询入口。旧 `ProcessTodoNow` 保留并生成新 attempt，不自动重试。
- idempotency_key 重复且输入指纹相同返回原 attempt；输入不同则拒绝。Todo UI 的失败/中断展示与显式重试按钮尚未接入（见未完成）。

### T9 Transcript 轻量轮询与路径查询（部分）

- `TranscriptStore::latest_turn_role` 只查最新一条 role；轮询器改用 `needs_processing_role`，不再为每个候选任务拉取完整轨迹并解析。
- `task_processor` 用 `ProjectStore::path_of(id)` 直接取项目路径，不再列出全部项目再筛选。
- 仍未实现：分批 IO/解析/提交、1MiB/500 条/8MiB 预算、源身份与 generation、满载 rescan-needed、半行/截断/替换回归、prompt 64KiB 预算与 argv 可用预算检查。

### T8 有界阻塞与 PTY 输入所有者（部分）

- `Session` 的 `write`/`resize` 不再在 async handler 里直接做阻塞 PTY IO；改为每会话单一 writer 线程 + 有界命令队列。
- 待写正文预算 64KiB、消息上限 256；满载/单次超限返回可读错误，不静默丢键。队列按到达顺序执行 write/resize。
- `kill` 直接操作 child，不排在输入队列后面，满载时仍可停止。
- 新增队列满拒绝测试。
- 仍未实现：request blocking 并发/等待准入、慢文件/Git/SQLite 移出 async worker、进程内服务 future 构造期慢 IO 隔离、不可中断 IO 的分段取消与停止预算。

### T5 会话真实退出码与迟到观察（部分）

- `Session` 增加独立 child waiter 线程：通过 `try_wait` 回收子进程、记录真实退出码，并且只广播一次 `Exited { code }`。PTY EOF 不再等价于“进程退出且 code=None”。
- 新增 `Session::exit_status()`：迟到 attach 可以在广播已错过时读取权威退出状态；`server` 在 `Attached` 应答后补发一次 `Exited`。
- `kill` 对已自然退出（ESRCH）的会话幂等返回成功。
- 退出历史预算：registry 按总正文 64MiB、最多 128 个退出会话、24h TTL 逐出**已退出且无活动订阅**的正文；常驻元信息上限 1024 条，超限时移除最老已退出记录。活动会话永不因历史预算被杀；attach 到已逐出历史返回明确 `HistoryEvicted`。
- 仍未实现：lifecycle revision、bridge/自然退出总结的单一收尾所有者与租约、退出后运行期句柄释放（master/writer/child）。

### T10 旧接口 locate 线性扫描与预算（部分）

- locate 改为单向字节/行/Unicode 标量游标；context 行边界在同一条长行上复用，不再为每个命中从文件头重扫。
- 保留 1-based 行列与半开区间 `[start,end)`；新增 emoji、中文、多行 query、无换行长单行密集命中测试。
- 旧接口显式预算：文件 16MiB、命中 1000 条、单条 context 1024 标量（带截断标记）、回复约 1MiB。超限返回 `LocateError::Limit`，daemon 回 `Reply::Error`，不伪装成“结果为空/全部命中”。
- 新增 `LocateInFileV2` / `LocateReport`：返回已知前缀、`complete`、`limit_reason` 和文件 revision；命中/回复预算耗尽时保留已扫描结果并声明不完整。MCP 使用 V2 并输出 `complete/limit_reason/revision`，旧 daemon 回退旧接口时按完整列表处理。
- 修正协议注释与 MCP 工具描述中“含端点”的歧义，明确实际语义是 1-based 半开区间 `[start,end)`（end 是最后一个字符之后）。
- 未实现 release 基准与同构建前后对照（见未完成）。

### T11 原子单实例锁（部分）

- 新增 `crates/dozerd/src/process_lock.rs`：在日志初始化、DB、apps 和 socket 清理之前对 `<socket>.lock` 取得 `flock` 排他锁，持有到进程退出；锁文件不 unlink。
- 第二实例在 `daemon::run` 早期直接失败，不会触碰 DB/apps/socket；`ensure_single_instance` 的 connect 探测保留用于陈旧 socket 清理，但不再单独裁决单实例。
- 后台回填状态机、统一 shutdown coordinator、SIGTERM/SIGINT 共用路径和双实例信号测试尚未实施。
- 历史 transcript 发现/摄取/Codex SQLite 回填改为 socket 绑定前调度的后台 blocking 任务,不再阻塞启动;扫描仍一次性收集,尚未实现有界迭代/可取消与 BackfillState 能力。
- SIGINT/SIGTERM 现在调用与 Shutdown RPC 相同的 `shutdown_stores`（提交会话总结 + kill 存活会话 + 停止 apps）；worker 标志与 socket/进程锁仍由 daemon 收尾。

### T12 有界 UDS 帧（部分）

- `dozerd::server` 与 `dozer-client` 生产路径用有界 `read_frame` 替代 `BufReader::lines()`：按 `fill_buf/consume` 分片读取，client→server 上限 12MiB、server→client 上限 16MiB；无换行超限立即返回 `InvalidData` 并关连接，不继续积累。
- UTF-8 解码失败和超限都分类为协议错误；客户端请求编码后超过 12MiB 拒绝发送。
- 主请求应答发送前检查编码后大小，超过 16MiB 改回可读 `Reply::Error`，不发送半个 JSON 帧。
- client 增加 connect 3s、write 3s、普通 RPC 10s、Attach/订阅首帧 5s 的期限；旧同步 `ProcessTodoNow` 保留 600s 独立政策；长期订阅本身不设总超时。
- 应答读取超时返回 ServiceError(Unavailable)，文案明确“请求可能已执行、结果未知”，客户端不自动重发。
- server 增加连接上限 128、普通工作 16、控制预留 4、等待上限 64；长订阅只占连接名额，初始请求结束后释放普通工作名额；`Shutdown` 走控制名额。过载返回稳定 Busy 文案。
- 尚未实现：稳定 Busy/Limit/Timeout 类型映射、server 侧操作期限、128 连接/慢读写压力测试。

### T13 核心 CI 测试 job（部分）

`.github/workflows/consumer-check.yml` 新增 `runtime-tests` job：

- `cargo test --locked -p dozer-file-io -p dozer-preview-core`
- `cargo test --locked -p dozerd --lib`
- `cargo test --locked -p dozer-client --lib`
- `cargo test --locked -p dozer-panel-preview -p dozer-panel-todo -p dozer-panel-agent --lib`
- `bash scripts/check-log-scope.sh`

这些测试不依赖 agent 登录、网络模型或 GUI。

## 验证结果

本机使用 `CARGO_TARGET_DIR=$PWD/target/runtime-review`、`--offline` 执行：

| 命令 | 结果 |
|---|---|
| `cargo test -p dozer-file-io` | 22 passed |
| `cargo test -p dozer-preview-core` | 166 passed |
| `cargo test -p dozerd --lib` | 519 passed, 3 ignored（含 N1–N4 修复回归） |
| `cargo test -p dozerd --test runtime_regressions` | 3 passed：控制连接、任务幂等、迟到 Attach |
| `cargo test -p dozer-client --lib` | 63 passed |
| `cargo test -p dozerd --test file_mutation_requests --test agent_context_requests` | 4 passed |
| `cargo test -p dozerd --test session_survival` | 6 passed |
| `cargo test -p dozerd --test shutdown` | 4 passed |
| `cargo test -p dozer-mcp --test todo_tools --test file_mutation_tools` | 9 passed |
| `cargo test -p dozer-panel-agent -p dozer-panel-preview -p dozer-panel-todo --lib` | 82 / 410 / 69 passed |
| `cargo test -p dozer-mcp --test features_gating --test default_product_golden` | 6 / 4 passed |
| `cargo fmt --all -- --check` | 通过 |
| `python3 scripts/audit/check_panel_boundary.py` | ok |
| `bash scripts/check-log-scope.sh` | ok |

新增回归覆盖的重点行为：

- 外链/父链/内部链接/绝对路径/`..`/非普通文件；外部目标字节不变；
- 0755/0600 保持、setuid/setgid 清除、xattr 复制、硬链接拒绝、revision 冲突、临时文件不残留；
- BOM/CRLF 保持、只采样 64KiB、目标为符号链接时拒绝保存；
- stdin 不读仍受 deadline、exit 42 非成功、stdout 超限、stderr 饱和排空、leader 先退出但后代持管道、cancel、双管道饱和、进程组后代回收；
- locate 的 Unicode 标量坐标、多行 query、长单行密集命中、超限显式失败；
- attempt 的单任务准入、RAII 中断、幂等 key 返回/拒绝、成功/失败回合与状态同事务、重启 Running→Interrupted、失败后自动轮询不重放；
- 原子进程锁第二实例拒绝、锁文件不 unlink；
- UDS 无换行超大帧拒绝、分片读取与 EOF；
- 会话真实退出码、迟到 attach 读取退出状态、PTY 输入队列满载拒绝；
- LocateV2 的 `complete/limit_reason/revision` 与超限前缀结果。

## 验收矩阵状态

| 编号 | 状态 | 本轮证据 / 缺口 |
|---|---|---|
| A1 | 通过 | `dozer-file-io` 路径/符号链接/外部字节测试；`file_mutation` 外链/父链测试 |
| A2 | 部分 | 0755/0600/setuid/xattr/hardlink/revision/临时文件测试；缺 rename 前后故障注入与完整 ACL fixture |
| A3 | 通过 | `process_runner` 的 timeout/stdin/exit 42/双管道/leader 先退/cancel/后代回收测试 |
| A4 | 部分 | attempt 准入/幂等/中断/重启/成功失败事务测试；缺 Todo UI 与取消/DB 提交失败完整矩阵 |
| A5 | 部分 | 真实退出码、迟到 attach 补发 Exited；终端 V2 Resume/Reset/epoch 未实现 |
| A6 | 未完成 | GUI 端到端背压、暂停 ack 预算与 tab 关闭释放未实现 |
| A7 | 部分 | 独立 waiter、真实退出码、历史预算/逐出；缺运行期句柄释放与总结至多一次 |
| A8 | 未完成 | 分批摄取、锁外解析、源 generation/rescan-needed 未实现 |
| A9 | 部分 | 轻量最新回合轮询、`path_of`；缺 blocking 准入与进程内服务隔离 |
| A10 | 部分 | 线性 locate、显式预算、LocateV2；缺 release 基准/扫描计数曲线 |
| A11 | 部分 | 原子进程锁、后台回填调度；缺 BackfillState、可取消有界扫描、双实例测试 |
| A12 | 部分 | SIGINT/SIGTERM 复用 `shutdown_stores`；缺 attempt/bridge/worker/socket 单一 coordinator 与信号集成测试 |
| A13 | 部分 | 有界帧、client 期限、连接/工作/控制准入；缺 Busy/Limit/Timeout 稳定映射与压力测试 |
| A14 | 部分 | 新协议均为新增请求/默认 trait 方法，golden/fake 编译通过；缺四种 client/daemon 组合实测 |
| A15 | 部分 | 新增 `runtime-tests` CI job 与本地门禁；CI 尚未实际跑过 |
| A16 | 未完成 | 尚无 release 性能基准与同 fixture 前后对照 |

## 未完成 / 已知限制

- **T4**：后端 attempt 表、准入、幂等、失败回合、重启恢复已实施；Todo UI 仍未展示 Failed/Interrupted 与提供显式重试按钮，`RuntimeCapabilities` 能力声明、取消/DB 提交失败/新输入轮询的完整测试矩阵也尚未补齐。旧 `ProcessTodoNow` 前端路径仍按同步 RPC 使用。
- **T5–T7**：真实退出码、独立 waiter、迟到 attach 补发 Exited、退出历史预算与元信息上限已实施；lifecycle revision、bridge/自然退出总结单一收尾、运行期句柄释放、终端 V2（offset/epoch/Resume/Reset）、client 恢复、面板到 GUI 的端到端背压尚未实施；旧 `Lagged` 路径仍是原行为。
- **T8**：每会话 PTY 单写队列/64KiB 输入预算已实施；有界 request blocking 准入、慢文件/Git/SQLite 移出 async worker、进程内服务构造期慢 IO 隔离尚未实施。
- **T9**：轻量最新回合轮询与 `path_of` 已实施；transcript 分批摄取、锁外解析、源 generation/满载 rescan-needed、prompt 64KiB 预算尚未实施。
- **T10**：旧接口线性扫描/预算与 LocateV2 的 `complete/limit_reason/revision` 已实施；release 性能基准与扫描计数曲线未采集。
- **T11–T12**：原子进程锁、有界帧读取/发送前大小校验、client connect/write/operation 期限与超时不重发、SIGINT/SIGTERM 复用 Shutdown 的会话/apps 收尾、连接/工作/控制准入已实施；后台回填状态、完整 shutdown coordinator（attempt/bridge/worker/socket 单一所有者）、Busy/Limit/Timeout 稳定类型映射、server 侧操作期限尚未实施。
- **T14–T15**：结构整理、完整兼容矩阵、release 基准、macOS 冒烟与最终验收记录未完成。
- 程序路径 `resolve_binary_path` 仍发生在 `ExecutionSpec` deadline 之外；spawn 之后的各阶段已受统一期限约束。
- macOS ACL 通过原生 `acl_get_fd_np/acl_set_fd_np` 复制；Linux POSIX ACL 依赖 `system.posix_acl_*` xattr。其他 Unix 平台只复制 xattr；非 Unix 回退实现弱于目录句柄实现。
- advisory 写锁只串行化遵守同一锁根的 Dozer 写入；外部非合作编辑器在最终检查与 rename 之间的写入不在保证内。revision 是元数据 revision，不是文件系统内容 CAS。
- release 性能基准、locate 扫描计数曲线、RPC p95/p99、终端内存上界等指标未采集；本轮不据此宣称整体性能提升。
- 工作区里 `crates/dozer-hook/src/opencode.rs` 的并发追加改动不是本轮产生，未改动/未回滚。
