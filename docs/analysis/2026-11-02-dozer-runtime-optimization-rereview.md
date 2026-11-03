# Dozer 运行时优化第二轮审核

> 修复状态：N1–N4 已修复，仓库内回归与原审核定向测试均通过。以下保留修复前证据；修复实现与复测结果见文末。本次只关闭这四项问题，整套优化计划仍为部分实施。

日期：2026-11-02。范围：`a9e02c5d` 之上的当前工作区，重点复核上一轮 R1–R9 的修复。

依据：[规格](../superpowers/specs/2026-11-02-dozer-runtime-optimization-design.md)、[计划](../superpowers/plans/2026-11-02-dozer-runtime-optimization.md)、[上一轮审核及开发方修复记录](2026-11-02-dozer-runtime-optimization-review.md)。

**原审核结论（修复前）：本轮修复仍不能验收通过。** 确认 3 项 P1、1 项 P2；其中包括控制连接补丁引入的新回归，以及原取消、终端输出和幂等问题未覆盖的路径。原审核阶段只新增本文，没有修改业务代码；后续修复记录见文末。

## N1 · P1：满载时普通请求会触发整个 daemon 退出

位置：`crates/dozerd/src/server.rs:761–765`、`server.rs:1935–1937`。

控制连接补丁在普通连接满额后，用预留名额接入新连接并标记 `control_only`。该连接收到非 Shutdown 请求时，设置 `should_exit_after_reply = true`，意图是返回 Busy 并关闭连接。

但是回复后的共用分支把这个标志解释为“通知整个服务停止”，调用 `shutdown_signal.notify_one()`；serve 因而退出。这条路径没有执行真正 Shutdown 分支的会话/apps 收尾。

通过直接依赖当前 dozerd 源码的隔离工程，启动真实 `daemon::run`（数据和 socket 均在临时目录、后台功能关闭），保留 120 个普通连接，再在第 121 个连接发送 Hello：

```text
121st ordinary request reply = Error("服务繁忙（Busy），请稍后重试")
daemon stopped after ordinary request = true
daemon result = Ok(Ok(()))
```

这是实际 UDS 复现，不是 semaphore 单元测试。普通请求不应具有停止服务的副作用。

修复要求：区分“关闭本连接”和“停止 daemon”；只有真正的 Shutdown 收尾路径才通知停止。回归应先占满普通连接，确认额外 Hello 仅被拒绝、服务仍运行，再确认真实 Shutdown 可以通过预留入口停止。对应上一轮 R7、规格 A13。

## N2 · P1：进程排队阶段仍不响应取消

位置：`crates/dozerd/src/process_runner.rs:140–145`。

后半段 IO/wait 的取消处理已经补齐，但 `run` 仍先单独 await `limiter().acquire(remaining)`；cancel watch 要在取得许可并 spawn 后才被轮询。执行名额满时，取消的群聊/任务会继续等待原 deadline，或等到空位后先 spawn 再处理取消。

直接调用当前 `dozerd::process_runner`：占满 4 个执行名额，提交一个 400ms deadline 的排队任务，在 50ms 时发送 cancel：

```text
cancel queued after 50ms:
elapsed ≈ 402–404ms
result = Err(Timeout)
```

预期应及时得到 Cancelled，不应等待原执行期限；实际场景的期限可达数分钟。仅修复第一轮 select 之后的分支不能覆盖排队取消。

修复要求：准入等待也同时监听 cancel 和 absolute deadline，并在 spawn 前再次拒绝已经取消/过期的调用；等待占用继续使用 RAII。新增满载排队取消、进入队列前已取消、取消与释放许可交错时不启动命令的回归。对应上一轮 R3、规格 A3。

## N3 · P1：迟到 Attach 仍会在输出排空之前收到 Exited

位置：`crates/dozerd/src/session.rs:237–244`、`crates/dozerd/src/server.rs:878`、`server.rs:1925–1933`。

waiter 现在先记录 `exit_code`，等待 PTY 排空后再广播 Exited，改善了已有订阅的顺序。但 Attach 仍将 `s.exit_status()` 直接放进 `late_exit`，在 Attached 回复之后立即补发 Exited。这个查询表示进程已退出，并不表示最后的输出已经进入快照或发给客户端。

受控调度验证使用当前 session 源码的临时副本，仅在 PTY reader 已读到字节、尚未放入 ring 时增加测试屏障，模拟线程被暂停；未修改仓库源文件，也没有更改状态机。子进程写 FINAL 并退出，在 1s 排空期限内执行与 Attach 相同的 subscribe/snapshot/exit_status 读取：

```text
snapshot = "", next_offset = 0, immediate late_exit = Some(Some(0))
释放 reader 屏障后：Output("FINAL", offset=5)
随后：Exited(code=0)
```

session 广播顺序本身已正确；问题是 server 的“迟到补发”旁路仍会把进程退出码提前作为终端流结束发送。当前 client/面板在收到 Exited 时结束转发，因此最后的 FINAL 仍可能丢失。

修复要求：保留“进程已退出”和“终端输出流已完成”两种状态；迟到补发须与快照及最终输出水位一致，不能仅换成另一个非原子读取的 bool。若快照未覆盖最终水位，应先转发剩余输出再发 Exited。增加 child 已退出、reader 尚未提交尾部、此时重新 Attach 的 server 集成测试。对应上一轮 R4、规格 A5/A7。

本项证据为受控 session 时序加实际 server 分支复核，没有宣称已跑完整 GUI 或实际 UDS 的这个调度场景。

## N4 · P2：不同幂等键的并发请求仍被错误当作已接收

位置：`crates/dozerd/src/server.rs:300–303`。

快速路径虽已删除，但 `start_task` 返回 `AdmissionError::InFlight` 后，handler 仍查询 latest 并返回 `Reply::TodoExecution`。同 key 不同输入会被 store 拦住，但**新 key、新输入**与已有执行冲突时，仍会收到原 attempt，第二次输入没有被执行。

真实 UDS 验证中，用 Running attempt fixture 保持第一次执行（避免调用真实 agent）：第一次 key 为 key-a；第二次请求携带 key-b 与不同 human_reply。当前回复为：

```json
{
  "type": "todo_execution",
  "execution": {
    "idempotency_key": "key-a",
    "status": "running"
  }
}
```

返回的 attempt_id 也与第一次完全相同。该结果不是 key-b 请求的幂等命中，而是请求被静默替换。

修复要求：只有 store 返回 `Admission::Existing` 时才返回原 attempt；InFlight 直接返回 Busy/InFlight 错误。server 级测试同时覆盖同 key 同输入、同 key 不同输入、不同 key 与无 key 的并发请求。对应上一轮 R6、规格 A4/A14。

## 本轮通过的复核

在同一隔离工程中，使用当前实际 crate/source 重跑下列五组反例，断言均通过：

1. 生产新保存入口传入真实项目根后，拒绝父目录外链，项目外字节保持原样。
2. 普通保存不再在用户项目创建 `.dozer-write-*` 锁文件。
3. IO 先完成、leader 先退出这两种运行阶段的取消，均返回 Cancelled。
4. 取消 32 个排队 future、释放执行名额后，可以继续成功执行命令，等待计数未永久泄漏。
5. 当前有状态 FrameReader 被取消后保留半帧，后续字节能重组为完整 JSON。

这些结论仅针对列出的反例；例如“排队计数不泄漏”不等于“排队期间监听取消”，不能混用验收证据。

## 验证记录与范围

隔离工程：`/tmp/dozer-runtime-rereview-20261102`。复制当前 Cargo.lock 作为依赖版本起点，仅在该临时工程内解析依赖，构建输出使用 `target/runtime-review`；未修改真实产品数据目录。真实 daemon 使用 `DaemonFeatures::minimal()` 与临时 ProductDirs。

定向验收测试：

```text
previous_fixed_regressions_pass                    PASS（内部五组断言）
ordinary_request_must_not_shutdown_daemon           FAIL
different_key_while_running_must_be_rejected        FAIL
queued_cancellation_must_be_observed                FAIL
```

命令：

```bash
cargo test --offline --manifest-path /tmp/dozer-runtime-rereview-20261102/Cargo.toml --test re_review
```

结果：1 passed、3 failed。N3 的受控调度通过另一个 `late-attach-barrier` 运行模式验证。上述失败是预期行为断言被实际产品行为违反，不是编译/依赖或真实 agent 登录问题。

本轮未重新运行整个 workspace，也不把开发方记录的 515/63 等通过数当作独立复测结果。`git diff --check` 通过。

原计划未完成的终端 V2/背压、Todo UI、分批摄取、完整 shutdown coordinator、兼容矩阵与 release 基准，本轮未新增验收证据，仍不能标记整套规格完成。建议先修复以上四条具体路径，再补相应 server/准入/迟到 Attach 回归，更新修复记录中的“已修复”结论。

## N1–N4 修复与复测

用户授权直接修复后，完成以下变更：

- **N1**：分离关闭本连接与停止 daemon 的标志。控制预留连接收到普通请求或错误 JSON 后只关闭本连接；真正的 Shutdown 仍执行原收尾并通知停止。
- **N2**：取消监听覆盖执行许可等待，排队取消时 RAII 释放等待计数；取得许可后、spawn 前重验取消与期限。已取消命令不启动。
- **N3**：正文、最终水位和流完成状态归入同一个 `OutputState` 锁；reader 的 Output 与 waiter 的 Exited 也在该锁内发布。Attach 使用同一锁读取完整快照，不以进程退出码替代流完成。已封闭的流不再发布迟到正文；补发 Exited 后取消该连接订阅，避免重复 Exited。
- **N4**：仅 `Admission::Existing` 返回既有 attempt。`AdmissionError::InFlight` 明确返回错误，不查询 latest 冒充本次请求的执行结果。

新增持久回归：

| 位置 | 覆盖 |
|---|---|
| `process_runner` 单元测试 | 真正排队后取消且命令未启动、等待计数释放；进入执行器前已取消不启动 |
| `session` 单元测试 | reader 提交受阻、进程已退出时快照仍不宣称流结束；排空后的快照/续接水位；结束后不再发布正文或重复结束 |
| `tests/runtime_regressions.rs` | 120 个普通连接满载后拒绝 Hello/错误 JSON 只关闭连接，真实 Shutdown 可达；四种幂等/冲突组合；迟到 Attach 正文完整且仅一个 Exited |

CI 的 runtime-tests job 已加入 `--test runtime_regressions`，不依赖真实 agent 或 GUI。

修复后验证：

```text
cargo test --offline -p dozerd --lib
  519 passed, 3 ignored
cargo test --offline -p dozerd --test runtime_regressions --test session_survival --test shutdown --test hook_events
  19 passed（3 + 6 + 4 + 6）
原审核临时工程 --test re_review
  4 passed, 0 failed（此前 1 passed, 3 failed）
cargo check --offline -p dozerd -p dozer-client -p dozer-mcp -p dozer-app --all-targets
  通过，保留 app 原有 unused/dead_code 告警
cargo fmt --all -- --check / git diff --check
  通过
面板边界 / 日志范围门禁
  通过
```

验证时仅临时解析本机 bytegit/byteui path patch 所需的 Cargo.lock source 行，结束后恢复原字节；没有改动用户已有锁文件内容，也没有推送或发布。
