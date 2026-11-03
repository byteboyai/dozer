# Dozer 运行时优化后续补齐与验收记录

日期：2026-11-02。对应 [实施计划](../superpowers/plans/2026-11-02-dozer-runtime-optimization.md) 与 [设计规格](../superpowers/specs/2026-11-02-dozer-runtime-optimization-design.md)。

结论：本轮已补齐核心实现与下列本机自动化验证；**整份计划仍未全部验收**。GUI/真实 agent、历史二进制完整组合、端到端 release 性能和完整慢连接故障矩阵仍待验证。第一批 implementation/review 文档是历史快照，以本文作为最新完成情况，不将历史缺口重复计作当前已知缺陷。

## 1. 实现范围

- 文件打开与保存共用目录句柄边界、跨进程锁、revision 校验、元数据复制和原子替换。补齐 rename 前后故障注入、两个独立进程争写、末级路径替换和文件增长限制。修正 macOS ACL API 返回值判断，真实扩展 ACL 在保存后保持；最后一次 fsync 包含元数据变更，提交后不再因重新打开文件失败而误报未提交。
- Todo、总结、群聊共用有界进程执行器，期限覆盖解析、等待、stdin/stdout/stderr 与回收。取消排队任务不会 spawn；保留场景权限、cwd、环境、只读策略。Todo attempts 持久化执行状态与幂等键，失败/中断等待显式操作，UI 展示失败和重试并拒绝迟到结果。
- 会话退出与输出排空分离，真实退出状态通过持久 lifecycle 发布。PTY 非阻塞读写可停止，退出释放运行句柄。历史缓存、活会话创建与 Draining 登记均受预算和单一所有者管理。排空超时通过 `output_truncated` 明确披露。
- V2 终端流包含 epoch、Resume/Reset、状态 revision 和恢复；client 从 GUI 已应用水位恢复。正文许可覆盖共享交付句柄，最后一个 clone 释放才回收字节额度。大快照拆成至多 64KiB 批次，保留快照尾部的预算；replay 数据不会回应历史终端查询。旧 daemon 通过有界、明确 degraded 的适配器接入。
- 面板采用新流，Reset 清理解析器、屏幕、选择和 cwd/OSC 状态；关闭与切换取消旧流，拒绝旧 owner/epoch。恢复、失败、降级和输出不完整有持续状态提示。
- 同步服务使用有界共享 IO 池，future 构造不执行 IO；server 将领域同步 handler 移出异步 executor。PTY 输入有字节预算，kill 独立于满输入队列。
- transcript 两个 worker、公平分批、按源合并和源集合上限；锁外读取/解析，事务校验 source generation/offset。半行不推进 offset，替换、截断、追加竞态均有回归。独立持久解析序号与分页序号，避免替换后新消息复用旧索引导致分页漏行。Todo 使用轻量最新回合判定及有预算的 prompt。
- session 收尾先持久化 finalization intent，再摄取和入 summary job。失败保留 intent 并退避恢复；重启复用已提交 job，包括 job 已落库但收尾标记尚未提交的窗口；并发自然退出/关闭共用每会话准入。
- 单实例锁先于有副作用初始化。bind 成功后才放行历史扫描、summary worker 与任务轮询。后台发现迭代有界、可取消，发现失败明确计入状态；取消状态不会被迟到更新覆盖。历史 UI 的状态查询错误显示失败，不再吞掉错误后显示确认无历史。RPC、SIGTERM、SIGINT 共用停止流程，Shutdown 回复写失败仍执行清理。
- UDS 帧读取、编码、连接、准入、写入与操作期限有界；普通请求耗尽不能误触发 Shutdown。JSON 编码按转义后字节限制。项目、Todo、memory、书签、群聊和 transcript 等查询在物化期间检查行数/字节预算，超限返回 Limit，不伪装完整列表。
- locate 单向游标保留 Unicode 坐标。V2 声明 complete/limit_reason/revision，旧接口超限报错；context JSON 转义计入完整回复预算。client transport/services、daemon domain handler、app Todo 执行路由拆为独立模块，公开兼容入口保留。
- CI 纳入核心行为测试；真实产品 helper 路径来自本次 Cargo compiler-artifact，避免缓存旧二进制被误用。

## 2. 实际资源预算

| 层次 | 当前限制与含义 |
|---|---|
| 终端正文 | 每流 1MiB、client 共享 16MiB；单批 64KiB，GUI 应用后 ack |
| 终端编码帧 | 流帧最大 5MiB，共享准入预留 64MiB；安静连接不预占额度 |
| 会话历史 | ring 每会话 1MiB，总正文 64MiB；最多 64 个活会话、128 个退出会话、1024 条元信息，退出历史 TTL 24h |
| PTY | 每会话输入 64KiB；输出 broadcast 128 槽；退出排空至多 1s |
| 同步 IO 池 | 4 个执行线程、64 个等待工作；取消未开始的工作不执行 |
| 摄取 | 2 worker、256 个不同源；常规读取块 1MiB、单记录/批次正文上限 8MiB、JSONL 每批至多 500 条 |
| 任务上下文 | 64KiB prompt，保留完整回合边界并声明省略；必要输入不能静默截断 |
| 查询 | 通用物化上限 10,000 行 / 8MiB 估算内存；transcript 原始行上限 24MiB；编码帧仍另行限流 |
| LocateV2 | 文件 16MiB、1000 命中、context 1024 Unicode 标量、完整 JSON 回复 1MiB |
| UDS | 请求 12MiB / 回复 16MiB；连接和写 3s，普通 RPC 10s，Attach 握手和首帧 5s，普通空闲 30s |
| 停止 | 总预算 10s，受管进程清理子预算 3s；超时记录未完成资源并保留恢复状态 |

正文额度与帧准入是不同计量，不能相加后声称等于进程 RSS。解码对象、allocator、终端模型、内核 socket 缓冲等仍占内存；本轮没有测量真实 GUI RSS。Codex SQLite 兼容导入按线程限制 8MiB/100k 原始项，不等同于 JSONL 的 500 条事务策略。

## 3. 构建与测试证据

基线 HEAD：`a9e02c5d6c55abddb543d15245535f462cd4466b`，分支 `main`；macOS aarch64，rustc/cargo 1.98.1。工作区原有未提交改动保留，没有创建提交、推送或发版。

本机 `.cargo/config.toml` 的 bytegit/byteui 相邻目录 patch 会让 Cargo 临时去掉两条 git source。行为测试通过临时 wrapper 执行，运行后恢复原 lock；**这些测试不能称为根目录 `--locked` 测试**。正式锁文件检查在 `/tmp/dozer-runtime-locked-verification` 副本内执行，排除 `.cargo` 本机 patch，使用官方锁定 git 依赖和独立 target，从副本 cwd 运行 `cargo check --locked --offline -j 1 --workspace --all-targets`。

| 验证 | 结果 / 本机证据 |
|---|---|
| dozerd lib | 547 passed，3 ignored（需真实已登录 agent 的只读金丝雀）；`/tmp/dozer-runtime-daemon-last-lib.log` |
| daemon 请求/生命周期集成 | 44 passed，覆盖 agent context、app、文件写、群聊、hook、locale、运行时回归、session survival、shutdown；`/tmp/dozer-runtime-daemon-acceptance-last.log` |
| client lib | 70 passed |
| core lib | 116 passed |
| file-io lib | 29 passed，1 ignored helper；跨进程父测试实际启动该 helper，非漏测 |
| agent / conversations / preview / Todo 面板 | 分别 83 / 86 / 410 / 69 passed |
| preview-core / services / store-project lib | 分别 166 / 29 / 33 passed |
| 上述消费者日志 | `/tmp/dozer-runtime-final-consumer-evidence.log` |
| 真实 client/daemon、MCP、产品进程与最终构建 | 见本文末尾最终验证补记；保留各自命令和结果，不用单元测试代替 |

可复现核心命令：

```sh
cargo test --locked -p dozer-file-io -p dozer-preview-core -p dozer-services -p dozer-store-project --lib
cargo test --locked -p dozerd --lib
cargo test --locked -p dozer-client -p dozer-core --lib
cargo test --locked -p dozer-panel-agent -p dozer-panel-conversations -p dozer-panel-preview -p dozer-panel-todo --lib
cargo test --locked -p dozer-client --test against_real_daemon --test summary_reliability --test group_chat_client
cargo test --locked -p dozer-mcp --test todo_tools --test file_mutation_tools --test features_gating --test default_product_golden
```

本机实际命令使用前述 patch wrapper 与 `--offline -j 1` 替代 `--locked`；正式 CI 无本机 patch，保留 `--locked`。产品隔离测试先构建 `product_daemon` example，再从 compiler-artifact 读取本次 executable，以 `CARGO_BIN_EXE_product_daemon` 传给测试。

## 4. A1–A16 验收映射

“自动化通过”限于列出的行为与环境；不代表所有 GUI/模型/性能验收均已完成。

| 项目 | 已有自动化证据 | 完成范围 |
|---|---|---|
| A1 | file-io path 用例、打开 FD 后替换链接、daemon file mutation requests | 自动化通过 |
| A2 | `extended_acl_survives_atomic_replacement`、`cross_process_revision_conflict`、metadata/before_rename/directory_sync 注入、preview 保存回归 | 自动化通过 |
| A3 | process_runner 的不读 stdin、双管道、exit42、leader 后退、取消/abort、排队取消；group_adapter/summary_provider/headless_agent | 自动化通过；真实 agent 金丝雀未跑 |
| A4 | task_execution/processor/poller 的幂等、失败/中断恢复和提交故障；Todo 面板状态测试 | 自动化通过；完整历史产品库启动演练未跑 |
| A5 | `half_frame_disconnect_reconnects_from_ui_applied_watermark`、非法 epoch/gap/duplicate、V2 Resume/Reset UDS 回归、面板 owner/reset 测试 | 自动化通过；GUI 手动冒烟未跑 |
| A6 | `delivery_waits_for_ui_ack_and_last_clone_keeps_credit`、`large_snapshot_is_applied_in_bounded_replay_batches`、unacked drop、关闭清理 | 额度测试通过；GUI RSS 未测 |
| A7 | 短命会话/late attach、`exiting_child_releases_runtime_handles_even_with_unread_input`、历史逐出、finalization recovery | 自动化通过 |
| A8 | `large_source_commits_at_most_500_records_per_batch`、追加/替换解析屏障、独立分页索引、源冲突、半行/截断、有界源队列 | 自动化通过 |
| A9 | 最新回合轻量判定、prompt 边界、services lazy/panic/blocked IO 用例 | 自动化通过 |
| A10 | Unicode/多行/密集命中坐标、JSON 转义预算、扫描计数和下面的优化构建内核基准 | 正确性通过；完整 locate RPC release 基准未跑 |
| A11 | process lock、第二产品实例无副作用、BackfillState、目录发现失败/取消、监听就绪门控 | 部分；假慢扫描与双实例同时启动完整矩阵未跑 |
| A12 | `draining_during_spawn_rejects_late_registration_and_reaps_child`、重复 Shutdown、断开回复端、实际 SIGTERM/SIGINT 产品测试 | 部分；综合慢/失败资源停止矩阵未跑 |
| A13 | 超长无换行、取消保留半帧、有界 JSON 编码、写期限/连接过载、普通控制预留回归 | 部分；慢读写/长期空闲完整矩阵未跑 |
| A14 | JSON golden、默认 trait fake、legacy adapter、App envelope、迁移/重启和真实 client/daemon 请求 | 部分；完整历史二进制组合与外仓消费者未跑 |
| A15 | workspace all-targets check、fmt、host/panel/log/shared-deps 门禁；CI job 源码已更新 | 本机门禁结果见补记；未声称远端 CI 已执行成功 |
| A16 | 参数、计数器、固定 fixture 基准脚本/CSV 与限制说明 | 部分；端到端 release 延迟与 RSS 未测 |

兼容组合：新 client/新 daemon 使用真实服务集成；新 client/旧 daemon 以模拟旧协议服务验证 Reset+degraded；旧 client/新 daemon 以真实 daemon 的 legacy JSON/API 测试；旧 client/旧 daemon 仅保留 golden/契约基线。后两项不能等同于启动了历史发布版的全部四种二进制组合。

## 5. Locate 算法基准

脚本：[benchmark_runtime.py](../../scripts/audit/benchmark_runtime.py)；原始结果：[CSV](2026-11-02-dozer-runtime-locate-benchmark.csv)。脚本抽取当前保留的旧 prefix helper 和新 LineCursor，以 `rustc -O` 构建同一个独立程序，同一 `"中x\n".repeat(N)` fixture、全部 N 个坐标、各 5 次。旧 helper 与基线 HEAD 实现一致，两算法 checksum 相等。

| 命中数 | prefix 中位 ms | cursor 中位 ms | prefix 扫描字节 | cursor 扫描字节 |
|---|---:|---:|---:|---:|
| 20,000 | 280.496375 | 0.119625 | 1,000,010,000 | 99,998 |
| 40,000 | 1109.729833 | 0.222208 | 4,000,020,000 | 199,998 |
| 80,000 | 4437.448333 | 0.439375 | 16,000,040,000 | 399,998 |

这是坐标扫描内核实验，特意不应用生产 1000 命中上限以比较相同工作量；不是 Cargo release 端到端 RPC、文件 IO/context/JSON 或 GUI 指标。数据支持扫描由平方增长改为线性，不支持整个产品提高相同比例的结论。

## 6. 保证边界与仍待验收

目录句柄保证打开对象身份，不保证对抗同用户恶意搬移整个目录对象；合作写锁与 revision 不消除非合作写入最后检查后的竞态。transcript 检查观察到的替换/截断，不承诺识别对抗性的同 inode 改写再增长。

进程组清理只覆盖受管组；主动脱离进程组的后代不在此保证内。不可中断文件系统/系统调用无法被安全强制抢占，停止期限外的工作记录为未完成；已提交 offset、attempt、finalization intent 支持后续恢复，不能将超时说成完全清理。

旧终端协议明确降级，旧无界 API 不获得新流的完整背压保证。summary/provider、群聊权限仍沿用原场景政策，本轮不调用真实模型验证权限金丝雀。

仍待执行：完整 macOS GUI 产品冒烟、真实 agent 只读金丝雀、完整历史二进制组合/外仓消费者、历史产品库两次启动演练、慢首帧/读写/长期空闲与启停故障矩阵，以及相同 fixture 下完整 release locate、摄取期间 RPC p95/p99、GUI 停顿 RSS、首次可连接/停止耗时。计划对应待办保持未勾选。

## 7. 最终验证补记

- 真实 client/daemon 集成：`against_real_daemon` 14、`group_chat_client` 1、`summary_reliability` 1，共 16 passed；日志 `/tmp/dozer-runtime-client-final-integrations.log`。首次执行暴露旧测试仍假设 bridge/summary 占用两个输出订阅；现已按持久 lifecycle 更新为零，20 轮 attach/drop 的连接资源释放断言完整保留，重跑通过。
- MCP：`default_product_golden` 4、`features_gating` 6、`file_mutation_tools` 2、`todo_tools` 7，共 19 passed；日志 `/tmp/dozer-runtime-mcp-final-integrations.log`。
- 新构建的产品 helper：`product_isolation` 5 passed，包含两个隔离产品、同根第二实例无副作用、SIGTERM/SIGINT 实际回收和 socket/锁释放；日志 `/tmp/dozer-runtime-product-last.log`。
- 最新历史面板：86 passed；日志 `/tmp/dozer-runtime-conversations-last.log`。
- 不带本机 patch 的隔离副本：`cargo check --locked --offline -j 1 --workspace --all-targets` 通过；日志 `/tmp/dozer-runtime-locked-final.log`。保留既有 unused/dead_code 及第三方 future-incompat 警告，不声称零告警。
- `cargo fmt --all --check`、panel boundary、host-deps（含 8 个自测试）、log-scope、shared-deps offline、`git diff --check` 均通过。fmt 调用 Cargo metadata 也可能触发本机 patch 的 source 漂移，最终锁文件复核恢复原 git source，仅保留 file-io 新包和必要依赖边。
- 以上证据是本机运行结果；没有声称 GitHub Actions 远端 job 已通过。临时 target、wrapper、产品 fixture 和原始日志均位于 `/tmp`，未加入交付文件；固定基准脚本与 CSV 在仓库内。
