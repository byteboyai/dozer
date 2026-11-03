# Dozer 运行时正确性与性能优化实施计划

> 状态：核心实现已补齐并完成本机自动化验证（2026-11-02）；整套最终验收尚未完成。仍保留端到端 release 性能基准、完整历史二进制组合与 macOS GUI/真实 agent 冒烟项，不标记“全部完成”。
> 最新验收：[后续补齐与验收记录](../../analysis/2026-11-02-dozer-runtime-optimization-completion.md)。
> 第一批历史记录：[运行时优化实施记录](../../analysis/2026-11-02-dozer-runtime-optimization-implementation.md)。
> 日期：2026-11-02。
> 规格：[设计规格](../specs/2026-11-02-dozer-runtime-optimization-design.md)。
> 依据：[代码分析与隔离实验](../../analysis/2026-11-02-dozer-code-optimization-review.md)。

## 1. 执行约定与交付顺序

实施时先读取适用 AGENTS.md/仓库约束，记录最新 HEAD 与未提交修改。保留现有 model/services/panel、bytehost 应用隔离、群聊只读、日志/i18n 边界。当前文档任务不执行生产代码实现、提交、推送或发版；后续按用户实施授权推进，无需为了普通可逆实现反复确认。

对并发、权限、资源和恢复改动先建立能表达目标行为的失败测试，再实现。测试应检查可观察结果和资源释放，避免只检查源码字符串。每项完成需要记录变更、验证命令/结果和实际限制后才能勾选。纯文件移动无需新增镜像测试。

本计划在单仓按依赖顺序执行，不要求创建子代理、修改相邻仓库或并行开展多代理任务。

| 批次 | 任务 | 交付目标 |
|---|---|---|
| B0 | T0 | 可重复的代码、协议、数据和性能基线 |
| B1 | T1–T2 | 文件边界、元数据和原子提交修复 |
| B2 | T3–T4 | 受管进程与诚实的 Todo 执行结果 |
| B3 | T5–T7 | lifecycle、终端恢复和端到端背压 |
| B4 | T8–T10 | 阻塞隔离、摄取与搜索性能 |
| B5 | T11–T12 | 启停、回填与 UDS 预算 |
| B6 | T13–T15 | 指标/CI、结构整理与总体验收 |

依赖：T0→T1→T2；T0→T3→T4；T0→T5→T6→T7；T0→T8→T9；T1→T10；T3/T4/T5/T8/T9→T11；T6/T8/T10/T11→T12；T13 的测试门禁随 B1 开始逐批接入；T14 在相关行为稳定后执行；T15 汇总全部必需验收。该顺序允许 P1 修复先完成，长期运行治理随后跟进。

## 2. T0：重建基线、约定兼容与测试工具

**文件/范围：** 根 Cargo 配置、`.github/workflows/consumer-check.yml`、现有 `scripts/audit/`，相关 crate 的 tests；不提交本机 patch。

- [x] 记录 HEAD/分支/工作区、工具链、锁文件、有效 Cargo 配置和本地依赖 patch。分析文档的实验数字不是新的基线结果。
- [x] 将构建输出设为当前环境可写位置；如共享 target 不可写，使用仓库内 `target/runtime-review` 或临时 target，不请求修改共享目录权限。
- [x] 对有本地 patch 的测试在可写隔离 checkout 中准备独立 lock；正式验证另跑干净 checkout 的 `--locked` 路径，不提交 patch 引起的 Cargo.lock 漂移。
- [x] 读取现有总结 provider 进程组 guard、有界读取与故障测试，列出抽取时必须保留的只读/cwd/env/model 政策。
- [x] 建立 helper 测试程序：可选择不读 stdin、同时填 stdout/stderr、非零退出、生成同组后代、leader 先退、持续 PTY 输出；只使用本地程序，不调用真实 agent。
- [x] 为受控竞态提供屏障、可注入时钟/预算、失败注入和字节/句柄计数；不以随意 sleep 证明没有竞态。
- [x] 保存旧 Request/Reply JSON golden、公开 SessionService fake 编译接缝和旧库 fixture；列出大回复接口与现有消费者。
- [ ] 在 release 下记录 locate 20k/40k/80k 高命中、多字节长行、启动慢回填、摄取期间普通 RPC 延迟、终端暂停消费资源增长基线。

**验证：** 能在隔离环境运行既有核心单元测试；实际失败逐项记录，不能把上一轮 20 个隔离测试写成 workspace 已通过。输出基线记录，作为 A14/A16 的起点。

## 3. T1：项目文件受控打开

**文件：** 新增 `crates/dozer-file-io/{Cargo.toml,src/lib.rs,src/path.rs}`；`crates/dozerd/src/file_mutation.rs`、`crates/dozer-preview-core/Cargo.toml`，必要的 preview 保存策略适配。

- [x] 先增加末级外链、父级外链、内部链接、绝对路径、`..`、根路径别名、非普通文件测试；外部目标字节必须保持。
- [x] 固定规范化根目录身份；逐级目录句柄打开并禁止跟随链接，读/改共享同一解析入口。
- [x] 通过屏障替换末级路径，检查打开的对象；不使用“canonicalize 后按旧路径再次打开”实现强边界。
- [x] 定义 PathEscape / SymlinkRejected / NotFound / UnsupportedFile / Io 错误；产品层映射现有错误形状和本地化文案。
- [x] 精确搜索和编辑接入；预览阅读策略保持，保存明确拒绝链接；为合法的外部预览打开令牌保留独立入口。
- [x] 文档/测试明确目录对象身份保证及同用户恶意目录搬移不在保证内。

**门禁：** `cargo test --locked -p dozer-file-io` 和 `cargo test --locked -p dozerd --lib file_mutation`；相关预览策略测试。对应 A1。

## 4. T2：带元数据与提交结果的文件写入

**文件：** `dozer-file-io/src/{write,metadata,lock}.rs`；`dozer-preview-core/src/text_save.rs`；`dozer-panel-preview/src/{effects,pane,update}.rs`；`dozerd/src/file_mutation.rs` 及服务端结果适配。

- [x] 测试 0755/0600、BOM/CRLF、ACL/xattr、hardlink、多写入者、读格式失败及写盘故障；故障注入区分 rename 前/后。
- [x] 实现产品传入锁根的跨进程写锁和目标键；app/daemon 使用同一根，锁在读基线前取得，等待受预算约束。
- [x] 实现受控临时文件写入、元数据复制、revision 重验、原子替换、目录同步与清理；拒绝不支持的必要元数据/hardlink，清除提权位。
- [x] 引入 NotCommitted / Committed / CommittedDurabilityUnknown；上层对最后一种重读状态，不自动重写。
- [x] 格式探测仅采样 64KiB，保留正文编码语义；PermissionDenied 等错误不能当新文件默认格式。
- [x] daemon 精确编辑与预览保存迁到同一基础实现，旧 API 保留兼容包装；UI 的已保存状态只在实际提交结果确认后更新。
- [x] 更新关于“任何一步失败原文件不变”的过强注释，明确外部非合作写入的最终检查竞态。

**门禁：** file-io、preview-core 和 panel-preview 相关测试；两个进程写入 fixture；dozerd mutation 回归。对应 A2，并重新确认 A1。

## 5. T3：抽取受管进程执行器

**文件：** 新增 `crates/dozerd/src/process_runner.rs`；`lib.rs`、`headless_agent.rs`、`summary_provider.rs`、`group_adapter.rs`。

- [x] 先把分析中的超时存活、stdin 不读、exit 42 转为仓库内行为回归；增加双管道饱和、超大输出、leader 退出后后代持管道、cancel/abort 测试。
- [x] 从 summary provider 提取已有进程组和读取机制，设计具名 ExecutionSpec/ExecutionOutcome/ExecutionFailure；场景政策仍由原适配器决定。
- [x] 让程序解析、stdin、输出和 wait 共用绝对期限；并行泵送且关闭 stdin，错误不吞掉。
- [x] 实现 stdout 失败上限、stderr 有界保留/继续排空、退出码检查和允许列表诊断；原始 stderr/prompt 不持久化。
- [x] 实现独立进程组、TERM→KILL、显式 wait/reap、取消 guard 和清理失败分类，校验 leader 已退出路径；全局执行/等待预算覆盖所有场景，许可持有到实际回收。
- [x] 将旧总结、Todo、群聊与现有 provider 全部接入，保留其权限参数、隔离 cwd、环境与输出解析；删除迁移后的重复执行代码。
- [x] 保持总结 transient 分类、群聊只读金丝雀契约及各场景默认调用时长，超限不能返回普通成功。

**门禁：** dozerd 的 process_runner/headless_agent/summary_provider/group_adapter 单元测试；检查 helper 子进程结束及组内后代消失。对应 A3；已有环境限制应写明，不能只检查函数返回 Timeout。

## 6. T4：持久化 Todo 执行结果与显式重试

**文件：** 新增 `dozerd/src/task_execution.rs`；`todo.rs`、`task_poller.rs`、`task_processor.rs`、`server.rs`、`daemon.rs`；model/services/core/client 与 Todo 面板接口、状态和视图。

- [x] 增加独立 attempts 表和幂等迁移，包含规格要求的身份/状态字段；不使用全库 user_version，不增加 TodoInfo 必填字段。
- [x] 通过新 DTO、独立服务默认方法和新增请求支持执行状态、异步启动和 idempotency_key；能力声明与实际支持一同交付。
- [x] 实现每任务单次准入、持久化 Running 与 RAII 运行令牌；阻止自动轮询和人工按钮并发双跑。
- [x] 人类输入在执行前持久化；成功回合与 Succeeded 原子落库；失败记录明确 error/status，不伪成功。
- [x] 重启 Running→Interrupted；Failed/Interrupted 等待显式重试或新输入；旧历史按旧规则保留，不猜测失败文本。
- [x] 幂等 key 重复返回原 attempt，不同输入复用 key 拒绝；旧 ProcessTodoNow 不自动重试。
- [x] Todo UI 显示执行失败/中断与显式重试；不擅自切换人工 Done/Suspended，也不把持久失败状态只放进 Toast。
- [x] 测试取消、panic、DB 提交失败、进程成功但落库中断、重启、重复请求和新输入版本；未知外部副作用需人工决定。

**门禁：** attempts 迁移与状态机、poller/processor、服务契约、Todo 面板测试；JSON golden/旧 fake 编译。对应 A4/A14。

## 7. T5：权威会话生命周期与运行期资源释放

**文件：** `dozerd/src/{session,registry,server,ide_bridge,ring}.rs`，新增 lifecycle/retention 模块；会话相关 tests。

- [x] 为短命进程、订阅晚于退出、bridge 启动晚于退出、退出与最后输出交错建立确定性测试。
- [x] 引入 Running/Exited 权威状态和 revision，独立 child waiter，区分 PTY EOF 与退出码；退出后按预算排空输出。
- [x] 将自然退出总结与 bridge 收尾归到单一所有者和租约；使用现有总结去重键，不依赖输出 broadcast 的单条 Exited。
- [x] 活会话运行句柄与退出历史记录分离，及时释放 master/writer/child；历史记录实现字节/条数/TTL 预算及 HistoryEvicted。
- [x] 保留活动会话与未完成持久化收尾，历史预算不杀活进程；常驻元信息也有上限。
- [x] 测试真实退出码、late attach、历史逐出、运行句柄归零和总结至多准入一次。

**门禁：** session/registry/bridge/retention 单元测试，现有 session survival/自然退出总结相关集成测试。对应 A7，并为 A5 建立权威状态基础。

## 8. T6：终端 V2 协议、受限流与恢复

**文件：** `dozer-model/src/agent.rs`、`dozer-services/src/lib.rs`、`dozer-core/src/protocol.rs`；client 新会话流模块；daemon attach 与能力处理；共享 fakes。

- [x] 定义独立 SessionStream DTO/接口和新增能力请求，不改旧 SessionEvent/SessionAttach 的 Rust 形状；新 trait 默认返回不支持。
- [x] daemon 实现带 epoch/Resume/Reset/状态 revision 的 attach；正确处理先订阅后快照、重叠去重和已退出会话。
- [x] client 验证 offset、长度和 epoch，保留 UI 已应用水位；单一恢复任务、有限退避、明确失败状态。
- [x] 将旧 daemon 接入受限 legacy adapter，重连后统一 Reset、状态标记 degraded；旧 client 继续走旧帧。
- [x] 对快照和输出创建字节许可，并计入跨流全局预算；能力不支持不得接一个无界桥假装满足新保证。
- [x] 测试窗口内续接、窗口外重置、乱序/重复帧、半帧断线、重连期间退出及旧 epoch 迟到。

**门禁：** model/core/client 单元与服务契约，UDS attach 恢复集成，四种新旧 client/daemon 配对。对应 A5/A14；端到端内存保证要等 T7，不在此提前宣称完成。

## 9. T7：面板到 GUI 的完整背压与终端重置

**文件：** `dozer-panel-agent/src/{lib,state,update,restore}.rs`；`dozer-app/src/{app,host_adapters,term}` 适配；`dozer-panel-term/src/model.rs`。

- [x] 使用新流替换生产 attach forwarder，旧能力通过 client 受限适配进入同一上层状态机。
- [x] 每 tab 保留最多一个待处理数据唤醒；数据通过有预算的共享 pending batch 提供，GUI 消息携带共享交付句柄（Arc），不复制整批正文。
- [x] 终端模型应用数据后 ack offset/epoch 并释放许可；旧 tab/epoch 消息丢弃时也释放资源。
- [x] Reset 清理旧解析器/屏幕/选择状态并替换快照，重发 resize；发一次历史不完整提示，避免不断 Toast。
- [x] 控制状态合并且退出必达；关闭 tab、取消新建、项目 A→B→A 结束旧流和恢复任务。
- [x] 暂停 GUI ack 做满预算测试，检查不向无界 iced 消息转移大字节；恢复后 marker 继续，tab 关闭释放连接和许可。

**门禁：** panel-agent/term 与 app 宿主测试，假服务慢消费集成；macOS 手动验证连续输出、选择/resize、重置及退出。对应 A5/A6。

## 10. T8：有界阻塞与 PTY 输入所有者

**文件：** 新增 dozerd blocking 准入模块；`server.rs`、`session.rs`；`dozer-store-project/src/service.rs` 及其它查出的进程内服务适配。

- [x] 按规格配置执行/等待预算，先取得准入再启动 blocking；等待上限和关闭状态明确，Shutdown 保留通道。
- [x] 将慢文件/Git/SQLite 操作移出 async worker；进程内服务只在 future 被 poll 后调度后台工作，不在构造时执行。
- [x] 每会话单一 PTY 写入队列，普通 write/resize 保持准入顺序，输入队列初始 64KiB；kill/停止有独立取消通道，不能被满载/阻塞 write 挡住；满载明确失败，禁止静默丢键。
- [x] 活动 blocking 无法被 abort 的情况使用分段取消/结果丢弃及停止预算，错误和 panic 释放准入。
- [x] 用可阻塞接缝证明慢查询不会占用普通 executor 线程，验证排队、拒绝和许可释放；不只断言调用了 spawn_blocking。

**门禁：** blocking、PTY 输入、进程内服务契约与 daemon request 测试。对应 A9/A13 的执行准入部分。

## 11. T9：Transcript 分批摄取与轻量任务判定

**文件：** `dozerd/src/transcripts/{mod,parse,scan}.rs`、`backfill.rs`、hook handler、task_poller/processor；必要的 worker 模块。

- [x] 建立同源合并和有界源集合，源身份/generation 与会话 ID 空间明确；满载进入 rescan-needed，不静默漏最终 hook。
- [x] 按规格读取批次/记录限制，解析在 DB 锁外完成；事务校验源版本与 offset，成功后推进。
- [x] 对半行、超长记录、非 UTF-8、截断、替换、解析时外部追加、并发重复触发建立回归；保持现有消息键去重政策。
- [x] 对查询原始行先复制必要字段再锁外解析轨迹；慢解析期间其它查询可继续。
- [x] 增加最新回合/attempt 轻量查询，poller 不取全量轨迹；processor 使用 path_of 和有预算的 prompt 组装。
- [x] 取消/重启只从已提交 offset 恢复；收尾等待最后批次或保留明确的待摄取状态。

**门禁：** transcripts 解析/摄取/回填、task 判定测试；慢解析屏障与多源并发测试；记录锁持有和提交次数。对应 A8/A9。

## 12. T10：Locate 线性扫描与显式预算

**文件：** `dozerd/src/file_mutation.rs`；model preview DTO、core protocol、services/client 精确搜索接缝、`dozer-mcp/src/` locate 工具。

- [x] 保留 Unicode 标量列、1-based 行列与 `[start,end)`；增加 emoji、中文、多行 query、长单行密集命中、无换行 fixture。
- [x] 用单向游标/行边界复用消除重复前缀扫描；context 仅生成预算内正文，记录扫描计数。
- [x] 新 LocateV2 返回 complete/limit_reason/revision；按输入、命中、context 和序列化回复四种预算限制。
- [x] 旧接口超限返回错误，MCP 显式告知结果不完整；未新增隐含分页或错误总数。
- [ ] 同构建/fixture 跑至少 5 次 release 前后基准；提交算法扫描计数和规模曲线，不用 debug 秒数作 release 承诺。

**门禁：** mutation/locate、JSON、MCP 工具测试及 release 基准。对应 A10/A14。

## 13. T11：原子单实例、后台回填与统一停止

**文件：** `dozerd/src/{daemon,server,registry,backfill,ide_bridge}.rs`；新增 shutdown/startup lock/backfill 状态模块；model/core/client 与历史面板状态适配。

- [x] 在任何有副作用初始化前取得并持有独立进程锁；锁文件不 unlink；有锁时第二实例不得动 DB/apps/socket。
- [x] 将必要迁移与历史回填分离，监听先于全量发现/摄取；扫描迭代有界、可取消，项目按需回填可提高优先级。
- [x] 暴露 BackfillState 能力/查询，历史 UI 展示加载和失败状态，不把部分空结果当确认无历史。
- [x] 引入唯一 shutdown coordinator 与 Draining 准入；跟踪已准入 CreateSession/派发，保证取会话快照前完成登记或清理。
- [x] 三种停止入口复用持久化总结、attempt 状态、进程/会话/bridge、apps/worker 与 socket/锁清理；回复发送失败也继续停止。
- [x] 保存/停止失败不挂死；超时记录未完成资源，并保留重启恢复所需状态。
- [ ] 补全双实例同时启动、陈旧 socket、假慢扫描可连接的故障矩阵；当前已覆盖同根第二实例无副作用、创建中停止、重复 Shutdown、实际 SIGTERM/SIGINT、断开回复端继续停止。

**门禁：** daemon 生命周期、产品隔离、backfill 状态与实际信号子进程集成。对应 A11/A12，并重跑 A4/A7 的重启场景。

## 14. T12：有界 UDS 帧、期限和准入

**文件：** `dozer-client` transport 模块；dozerd server transport；core 编解码辅助及 MCP 使用方；相关服务错误适配。

- [x] 按 T0 大回复清单落实各操作预算；超限前不分配完整 base64/JSON，发出回复前校验完整帧。
- [x] 替换无界 lines 读取，限制无换行输入、分片读取和解码；保留合法 UTF-8/大快照路径。
- [x] 实现连接/普通工作准入和控制预留名额，普通排队与长期订阅分别计数。
- [x] 实现 connect/write/operation deadline，区分 Attach 握手、长期订阅和旧长操作；未来不向所有调用套同一个短超时。
- [x] 稳定映射 Busy/Limit/Timeout；不得为了新分类直接破坏旧 ServiceErrorKind 的穷尽匹配，必要时使用新 transport 错误类型映射回旧接缝。
- [x] 写请求超时返回结果未知，禁止自动重发；新幂等任务请求只查询原 attempt。
- [ ] 补全慢首帧、慢读写与长期空闲的完整故障矩阵；当前已覆盖无换行超限、帧取消、编码转义预算、写期限、连接/控制过载、断开 Shutdown 回复端。

**门禁：** client/core/server transport 测试；真实 UDS 假服务集成；新旧协议 golden。对应 A13/A14。

## 15. T13：随批次接入指标与 CI

**文件：** `.github/workflows/consumer-check.yml` 或新增 runtime-tests job；相关模块的内部指标；`docs/analysis/` 验收记录。

- [x] 每个行为修复批次加入不依赖 agent 登录、网络模型和 GUI 的单元测试集合，避免到最后才让 CI 开始跑测试。
- [x] 默认核心集合包含 file-io、preview-core，以及 dozerd 的文件、runner、attempt、lifecycle、摄取、transport；client 与 panel-agent 的恢复/背压测试。
- [x] 需要真实 daemon 的测试显式先 build 测试二进制并传入路径；不依赖目标目录已有旧程序。
- [x] 保留 fmt/check、共享依赖、面板边界与日志/i18n 门禁；识别现有 clippy 基线，禁止将 unrelated 历史告警混进本轮修复范围。
- [x] 增加规格指标，优先内部快照/测试钩子和 scoped 日志，不引入通用遥测平台，不输出敏感正文。
- [ ] 补全端到端 release 性能脚本；已有固定 fixture、优化构建、5 次重复的 locate 坐标内核脚本与 CSV，不能代替全部性能验收。CI 单元测试使用计数/屏障，不以机器时间阈值替代正确性。

**门禁：** CI 核心 job 成功，源码边界与敏感信息检查通过；指标可测预算与收尾结果。对应 A15/A16。

## 16. T14：行为稳定后的结构整理

**文件：** `dozerd/src/server.rs` 与新增 handlers/transport；`dozer-client/src/lib.rs` 与服务模块；必要的 app update/window_events 路由模块。

- [x] 将 transport、领域 handler 与运行时所有者分开，显式 ServerContext 收敛依赖；不重新引入面板到 client/protocol 的边。
- [x] client 保留公开重导出，把公共 transport 与各服务实现分离；避免为拆文件复制 roundtrip。
- [x] app 只移动已有明确所有权的路由/原生资源代码；焦点、关闭、epoch 校验和 WebView 隔离不变。
- [x] 不按总行数拆分测试较多的 pane.rs，不在重构中夹带新的产品行为。
- [x] 重跑相关行为测试与边界门禁，保持可独立审阅的机械 diff。

**门禁：** 相关 crate 测试与 `cargo check --locked --workspace --all-targets`、面板/host/日志门禁。对应 A15；重构本身不能作为前面行为修复的替代证据。

## 17. T15：集成验收、兼容和交付记录

- [x] 对规格 A1–A16 逐项给出测试名称、命令、结果与证据路径；部分验收和未验证项明确保留，不以“实现完成”代替。
- [ ] 完成旧库两次启动/迁移、四种 client/daemon 组合、旧 trait fake 编译和共享库消费者兼容检查；无法访问的消费者明确记录，不能伪称验证。
- [ ] macOS 冒烟：脚本/私有文件保存、Todo 失败/取消/显式重试、快速退出 agent、持续终端输出/恢复/resize、回填中历史界面、两种信号退出。
- [ ] release 基准对照：locate 规模曲线、摄取时 RPC p95/p99、GUI 停顿终端内存、退出历史保留、首次可连接与完整停止耗时。
- [x] 报告实际资源上界和保证边界：非合作文件写入、脱离进程组后代、旧客户端流、不可中断 IO、降级旧轮询。
- [x] 文档同步新能力、默认预算和错误含义；确认 Cargo.lock 没有本机 path patch、临时 target/fixture/日志未入库。
- [x] 更新本计划状态与验收链接；当前保留最终验收待办，未标记全部完成。未创建 release/tag、推送或修改外仓依赖。

建议最终通用验证命令（以实施时真实 crate/测试名补齐，不要求反复重跑已通过且未受影响的检查）：

```bash
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo test --locked -p dozer-file-io -p dozer-preview-core
cargo test --locked -p dozerd --lib
cargo test --locked -p dozer-client --lib
cargo test --locked -p dozer-panel-agent --lib
cargo test --locked -p dozer-panel-preview --lib
cargo test --locked -p dozer-panel-todo --lib
python3 scripts/audit/check_panel_boundary.py
bash scripts/check-host-deps.sh --self-test
bash scripts/check-log-scope.sh
bash scripts/check-shared-deps.sh --offline
```

集成测试在显式构建所需 daemon/helper 后按名称执行；若有前端源码变更，运行对应前端 build 并提交必要运行时产物。测试受环境限制时保留准确原因和未验证范围，不通过修改外部权限或隐藏失败继续声称全部通过。
