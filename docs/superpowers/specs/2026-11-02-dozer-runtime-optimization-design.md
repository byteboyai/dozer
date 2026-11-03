# Dozer 运行时正确性与性能优化规格

> 状态：待实施；本文定义本轮优化的目标行为，不代表功能已经完成。
> 日期：2026-11-02。
> 代码分析基线：`a9e02c5d`；实施前重新记录实际基线。
> 依据：[代码分析与隔离实验](../../analysis/2026-11-02-dozer-code-optimization-review.md)。
> 实施入口：[实施计划](../plans/2026-11-02-dozer-runtime-optimization.md)。
> 本文是现有文件预览、会话、Todo、总结与服务边界规格的增量。本文明确规定的超时、恢复、预算与文件提交语义优先于旧实现中的弱保证；产品定位、权限策略和已有领域含义沿用原规格。

## 1. 目标与范围

本轮优先修复项目外文件访问、保存权限丢失、headless 失控执行及终端滞后后停更，然后改善同步阻塞、搜索复杂度、会话资源保留、启动与通信预算，最后整理职责边界。

范围包含 Dozer 仓库的 daemon、client、共享面板、预览核心、服务接缝和 CI。共享 crate 的 API 变化必须验证现有消费者兼容性；本计划不要求修改或发布 bytehost、byteui、bytegit，也不要求实施 Digger 的产品功能。

不包含：重写 UI/插件体系、替换 PTY 或数据库、迁移 Git 运行时、改变 agent 权限政策、实现远端 daemon、自动恢复执行具有外部副作用的任务，以及对整个 workspace 给出无测量依据的性能承诺。

## 2. 固定决策

| 编号 | 决策 |
|---|---|
| D1 | 延续 model → services → panel 边界；面板不直接引用 client/protocol；执行机制不进入领域模型 |
| D2 | 项目内精确搜索/编辑拒绝根目录以下任一符号链接；项目根本身可先规范化并固定身份 |
| D3 | 文件替换保持权限与受支持元数据；无法保留必要元数据时拒绝覆盖，不悄悄降级 |
| D4 | 共用执行机制，分别保留 Todo、总结、群聊的 cwd、环境、工具权限、输出解析和重试策略 |
| D5 | 进程超时覆盖解析可执行文件、stdin、输出与等待；返回前完成受管进程清理，清理期限单独计入 |
| D6 | 任务执行结果独立持久化；进程失败不伪装成普通成功 AI 回合；不自动重放可能已有副作用的任务 |
| D7 | 终端输出使用连续 offset 与明确的增量/重置语义；退出和 agent 状态可在迟到订阅时读取 |
| D8 | 按字节限制终端整条链路，UI 应用后释放预算；不能只限制 socket 到 client 的一段 |
| D9 | SQLite/文件/Git 阻塞工作经有界执行边界；摄取解析不持全局数据库锁 |
| D10 | 协议能力按新增请求协商；保留现有 JSON 形状和旧公开方法，新 trait 方法提供默认实现 |
| D11 | 正常 Shutdown、SIGINT、SIGTERM 共用幂等收尾；启动前持有原子进程锁 |
| D12 | 保留可查看的已退出会话历史，但运行期句柄与历史记录分离，历史有总预算 |
| D13 | 先补行为测试和指标，再按职责拆模块；不把本次文档交付视为生产代码实施或发布 |

## 3. 模块与所有权

| 层 | 职责与落点 |
|---|---|
| `dozer-file-io`（新增同步小 crate） | 目录句柄约束、文件身份、同产品写锁、原子替换和元数据；不依赖 UI、tokio、model 或产品全局路径 |
| `dozer-preview-core` | 文本编码/BOM/换行约定与保存策略；调用文件 IO 基础实现 |
| `dozerd::process_runner`（新增模块） | 受管子进程、期限、泵送、取消、进程组、回收和资源预算 |
| `headless_agent` / `summary_provider` / `group_adapter` | agent 命令和各场景政策；将执行结果映射成各自错误/输出 |
| daemon 会话层 | PTY、进程 waiter、权威 lifecycle、字节 ring、attach 与运行期资源 |
| client 会话流 | 有界接收、offset 校验、重连、快照恢复和能力降级 |
| services / model | 独立的新流接口与 DTO；保持旧接缝可编译 |
| Agent 面板 / GUI 宿主 | 按批应用终端数据、ack、tab 归属检查、明确的恢复/失败展示 |
| daemon 摄取与生命周期层 | blocking 准入、分批摄取、后台回填、统一停止与进程锁 |

现有 `summary_provider` 已有有界读取、进程组 guard 和失败分类。实施应从该实现提取并补齐公共机制，避免再保留第三份独立执行器。不得在提取时丢掉总结调用的只读能力校验、隔离目录、环境清理或模型配置；群聊的执行层只读约束保持。

## 4. 文件边界与安全提交

### 4.1 路径与文件身份

项目根先 canonicalize，再打开并固定根目录句柄。请求路径必须是项目内非空相对路径；拒绝绝对路径和 `..`，允许移除无意义的 `.`。逐级使用目录句柄相对操作和禁止跟随链接的打开方式；最终文件须为普通文件，最终打开时再次拒绝 symlink。仅校验父目录、仅 canonicalize 后按原路径重开均不满足要求。

`locate_in_file` 与精确编辑共用受控打开逻辑。预览阅读的现有链接显示策略可保留，但保存遇到上述链接应明确拒绝，不能悄悄改写链接目标或把链接替换成普通文件。项目外预览若已有合法入口，由宿主传入独立的已授权打开令牌，不能把任意文件父目录自动当作项目根绕过限制。

本保证针对链接逃逸与普通路径替换；不承诺抵抗具有同用户权限、能重挂载或把整个已打开目录移出项目的恶意进程。目录句柄固定的是授权时的对象身份，不是永远不变的绝对路径。

### 4.2 文件写入流程

1. 使用产品传入的锁目录，取得规范化目标键的跨进程 advisory 写锁。Dozer app 与 daemon 使用同一产品锁根；锁等待有期限。读到新基线之前取得锁，避免内部写入互相覆盖。
2. 受控打开目标，记录文件身份、size/mtime 与内容 revision；校验精确编辑选区，或预览传入的期望 revision。目标 `nlink > 1` 时本轮拒绝原子替换并返回明确原因，避免悄悄拆散硬链接语义。
3. 编码正文；仅有界采样原文件前 64KiB 来探测 BOM/换行。NotFound 与 PermissionDenied/IO 错误区别处理，后两者不得回落为新文件默认格式。
4. 在同目录用唯一名字和 `create_new` 创建初始权限为 0600 的临时文件，写入、同步，并恢复原文件 mode、owner/group 及存在的 ACL/xattr。平台适配器无法保留时返回 UnsupportedMetadata，原文件不变；不得恢复 setuid/setgid 等提权位。
5. 提交前在同一写锁下重新检查目标身份/revision；不匹配返回 Conflict。对已打开的父目录执行原子替换，成功后同步父目录以增强掉电持久性，并清理临时资源。

文件替换边界必须区分：`NotCommitted`、`Committed`、`CommittedDurabilityUnknown`。rename 前失败保证原内容未被本次写入替换；rename 成功后目录同步失败不能再声称原文件保持不变。上层需重读结果并提示持久性未确认，不能自动重写。

普通文件原有 `0755`、`0644`、`0600` 等访问/执行位必须保持；新文件默认 0600。必要的元数据复制失败不覆盖旧文件。已有公开 `save_text_atomic` 可保留兼容包装，新生产路径使用携带锁上下文、revision 与提交结果的新 API；文档明确旧包装的保证边界。

Advisory 锁仅串行化遵守该锁的 Dozer 写入。外部编辑器不受该锁约束；提交前检查缩小丢失更新窗口，但不是文件系统内容 CAS。本轮不宣称阻止任意外部进程在最终检查和 rename 之间写入。

## 5. 受管进程与任务结果

### 5.1 共用执行机制

执行输入使用具名结构：command、stdin、absolute deadline、cancel、stdout/stderr budget、termination policy。程序路径解析使用同一个 deadline，spawn 前检查剩余时间。同步 spawn 本身不能由 async timeout 强制抢占，结束后必须立即检查期限并收尾，不能在超时后继续进入正常执行。

spawn 后立即并行启动 stdin 写入、stdout 和 stderr 读取；stdin 完成关闭以发送 EOF。stdin 错误、读取错误、输出超限、非零退出分别分类，不静默忽略。stdout 超限视为 OutputLimit，不把截断结果当成成功；stderr 超限继续丢弃并排空，只保留预算内诊断数据。

Unix 下在独立进程组启动，取消/超时/异常返回均请求终止该组，必要时升级 SIGKILL 并等待直接 child 回收。保留 leader 的原始 pgid，覆盖 leader 已退出但后代仍持管道的情况。`kill_on_drop` 和 guard 作为 task abort 的兜底；正常错误路径显式 await 回收。主动脱离进程组的后代不在普通进程组保证内，不宣称强沙箱。

执行期限到达后最多再使用 3s 清理预算（TERM 宽限最多 500ms，随后 KILL/reap）；清理异常返回 CleanupFailed 并进入受管残留诊断，不能宣称已清理完成。退出状态以成功码判定；信号退出保留终止原因。原始输入、token、完整 stderr 不进入日志，面向用户的诊断使用允许列表类别与短文案。

初始预算：普通任务 stdout 8MiB；总结 stdout 256KiB；群聊 stdout 1MiB；各场景 stderr 保留 64KiB，用户诊断至多 2KiB 且经允许列表处理。保留既有调用时长：旧总结 90s、Todo 600s、群聊 300s，已配置的总结 provider 时长继续从配置读取。预算可注入测试，不能由不可信请求无上限扩大。

执行器初始全局并发上限 4、等待上限 32，排队时间计入原 absolute deadline；许可持有到回收结束，不能在返回 Timeout 时提前归还。场景已有更小的并发约束继续生效，超载返回 Busy，不通过额外无界 task 绕过排队上限。

### 5.2 Todo 执行状态与重试

新增 `task_execution_attempts` 表，至少记录 attempt_id、todo_id、input_generation、可选 idempotency_key、status、error_kind、started/finished 时间及关联 conversation。status 为 Queued / Running / Succeeded / Failed / Interrupted；失败结果不能代替 Todo 的人工完成/搁置状态。

- 同一任务同时只准入一个 attempt；使用持久化状态和 RAII 运行令牌，取消或 panic 不得永久留在内存 in_flight。
- 先持久化本次人类输入与 Running，再执行；成功时事务写 AI 回合并更新 Succeeded，失败记录 error 语义及 Failed，不能写成 `is_error=0` 的普通成功回合。
- 重启后 Running 置 Interrupted，保留“可能已有副作用”的状态；Failed/Interrupted 需要用户显式重试或新的人类输入，轮询不因缺少成功 AI 回合而自动重复执行。
- 旧历史只有最后一条 AI、没有 attempt 记录时保持旧的已处理判定，不据不可靠文本回溯猜测旧失败。
- 新执行请求可携带 idempotency_key，重复 key 返回原 attempt，输入不同但 key 相同则拒绝。旧 `ProcessTodoNow` 保留，生成新的 attempt；客户端不自动重试它。

新增独立 ExecutionState DTO 与查询/执行方法，旧 TodoInfo 不追加 Rust 必填字段。新 trait 方法有保守默认实现，旧服务仍可编译；Dozer 面板对支持新能力的服务展示状态、失败原因和显式重试。总结重试继续遵循既有只读总结政策，不能把它应用到有副作用的 Todo。

## 6. 终端恢复、背压与会话状态

### 6.1 新流协议

新增 RuntimeCapabilities 请求/应答，能力至少包括 terminal_stream_v2、task_execution_state、locate_v2、backfill_state。旧 daemon 对未知请求返回现有 Error 时识别为能力不支持，不循环探测。Hello v1 形状和全局协议版本不因本轮新增请求而强制升级。

新增 AttachV2 与专用初始/推送应答，领域语义如下：

| 数据 | 含义 |
|---|---|
| AttachSnapshot | stream_epoch、mode=Resume/Reset、start_offset、next_offset、正文及权威 lifecycle/agent 快照 |
| OutputChunk | stream_epoch、start_offset、end_offset、正文；长度须等于 end-start |
| StateSnapshot | 最新 lifecycle_revision 与 agent_revision 及对应状态，可覆盖过期状态 |
| ResyncRequired | 当前流无法无损续接；停止接收该 epoch 的增量，发起一次恢复 |

offset 是原始终端字节坐标，不是字符数；`start <= end`，同流接受的增量必须连续。Attach 先登记输出订阅再取得带水位快照，重叠输出按水位去重；订阅过程遇到状态变化要读到最新 revision。epoch 由 client 每次创建 attach 意图时生成、请求携带且服务端回显，恢复前即失效旧 epoch；不能等新连接返回后才知道哪些旧结果应该丢弃。

Resume 表示请求 offset 仍在 ring 内，正文只补未消费区间；Reset 表示明确替换当前终端模型，不能直接追加快照。Reset 会丢掉 ring 之外的历史和部分交互状态，UI 应给出一次有标识的历史不完整提示，不声称完整恢复。offset 溢出、倒退、未知 epoch 视为协议错误。

### 6.2 恢复与兼容

支持新能力时，client 遇到 Lagged、offset 缺口或连接中断进入单一 Recovering 状态，关闭旧订阅，以最后一个 **UI 已应用** 的 offset 重连。恢复期间的新数据受同一预算限制；同一流最多一个恢复任务。最多连续重试 5 次，退避 100/200/400/800/1600ms；失败转可重试错误，用户重试重置次数，完成一次正常数据应用后清除连续失败计数。

应用 Reset 和新 epoch 要作为一个操作完成；ack 必须带 tab 归属和 epoch。终端历史重置时一并清理旧解析器状态、选择和不再有效的屏幕缓存，窗口尺寸随后重发。attach 已退出会话时也必须获得确定的退出状态，不能等一条已经错过的广播。

旧 attach/事件枚举和服务方法继续保留。增加独立 `SessionStream` 接口与 `SessionStreamEvent`，使用有界接收器或自有受限流对象，不能直接把旧 UnboundedReceiver 改成另一类型。新 trait 方法默认返回 Rejected/能力不支持，不通过无界桥接假装支持。

新 client 对旧 daemon 使用受限的 legacy 适配：重新 attach 后统一 Reset，不假设旧回复能证明 Resume；Lagged 必须关闭旧连接并恢复，不能只打印日志继续等待。旧协议不能提供的 lifecycle/退出保证在状态中标记 degraded。旧 client 对新 daemon 继续使用旧 JSON 路径；该旧路径不承诺新增的端到端背压保证，兼容矩阵必须单独验证。

### 6.3 字节预算与 UI 交付

初始每条终端流排队正文上限 1MiB，client 进程共享上限 16MiB；恢复快照也计入预算。单批至多 64KiB，正常 UI 唤醒间隔约 16ms；每个 tab 最多一条待处理数据唤醒，状态变化合并为最新快照。

预算从解码前的帧准入开始，经 client、panel forwarder、GUI pending batch 一直持有到终端模型实际应用后释放；ack 丢弃旧 epoch 时释放对应预算。不能把字节复制进无界 iced 消息后就释放许可。协议帧编码/解码和单个已应用模型的常驻内存单列测量，不能用“排队 1MiB”声称整个终端仅占 1MiB。

大快照先预留完整正文额度，拆为至多 64KiB 的 GUI 交付；拆分尾部带 replay 标记，不回应历史终端查询。最后状态在尾部应用后发布，旧 epoch 的未 ack 数据不推进恢复水位。编码帧使用独立 64MiB 共享准入，流帧上限 5MiB；安静连接不预占帧额度。

预算满时暂停读取，允许 daemon 的 ring/broadcast 触发重同步；不无声丢弃任意字节。控制状态独立保存最新值、退出状态不可被数据流挤掉；控制快照的序列化预算为 16KiB，事件名最多 256 字节、路径最多 4KiB。超限字段明确报 Limit，不能把任意原文塞进控制快照。关闭 tab 或取消 pending attach 必须结束 receiver/forwarder、释放预算与 socket。

### 6.4 会话生命周期与历史

daemon 权威状态为 Running → Exited(exit_status)，附单调 revision；PTY EOF 作为独立数据面状态。专用 waiter 回收 child 并记录真实退出码；退出后继续排空 PTY，至多 1s，超时关闭剩余读取并标记输出不完整，然后结束输出流。V2 状态通过 `output_truncated` 披露排空超时，不能仅以 `output_complete` 隐含表示。不得把 EOF 永远等价为 code=None 的进程退出。

自然退出总结与 bridge 清理由 registry 级的单一收尾所有者触发，迟到观察者读取当前状态。保证每个会话每种收尾副作用至多准入一次；总结先持久化 finalization intent，再执行最后摄取和 job 入队；失败保留待恢复意图，重启按 source session 复用已提交 job。总结队列去重负责重启一致性。bridge 使用可释放租约，退出与 session_started 交错时最终释放，不能依赖一次性输出 broadcast。

退出后释放 master、writer、child 等运行期资源，保留独立历史记录。初始正文缓存总预算 64MiB、最多 128 个退出会话、无活动订阅记录 TTL 24h；任一约束达到时按最近访问淘汰无活动订阅正文，保留精简元信息最多 1024 条。活动订阅计入总预算但可将旧正文标记已逐出，后续查询明确返回 HistoryEvicted；不能因预算杀活跃进程，也不能移除未完成持久化收尾所需记录。

## 7. 阻塞执行、摄取与搜索

### 7.1 有界阻塞工作

为 request IO、transcript 摄取和回填分别限制准入；初始 request blocking 并发 4、等待 64；transcript worker 并发 2、待处理源 256；启动 backfill worker 1。按源合并重复摄取信号，合并唤醒而非按每个 hook 堆积任务。昂贵工作不得在 async worker 或进程内服务 future 构造时同步执行。

数据库操作可继续使用现有 SQLite 连接与事务，通过显式 blocking 边界运行；本轮不要求引入连接池或全库 WAL。PTY 输入改为每会话单写入所有者，待写正文预算 64KiB；普通 write/resize 保持准入顺序，满队列明确拒绝，不能丢用户按键。kill/停止走独立取消通道并可越过被阻塞的输入，不能排在永远写不完的数据后面。已运行的 blocking 工作不能假设 abort 就能停止；分段检查取消，关停等待按期限处理。

### 7.2 Transcript 增量摄取

每个 source/conversation 串行摄取，源身份包含 agent、规范化路径与文件身份；不能只按裸文件名串行化不同来源。先短锁读取已提交 offset/version，锁外打开、限量读取和解析，再短事务校验版本并写入回合及新 offset。版本变化则丢弃候选、重新读取，不能提交陈旧解析结果。

每次 IO 读取预算 1MiB，每批解析/提交最多 500 条记录、累计原文上限 8MiB，单 JSONL 记录上限 8MiB。跨读取块的单条记录通过有界残片缓冲处理，1MiB IO 分段不意味着拒绝合法的 2MiB 记录；缓冲和候选解析计入 worker 的资源测量。半行不推进 offset；超过单记录上限标记该源失败并保留旧 offset，不能无限追加或静默跳行。文件截断/替换重置源 generation，并遵循既有消息键去重与历史保留政策，不能因锁外解析新增重复回合。

源合并集合满时发出有界的 rescan-needed 标记，后台恢复扫描；hook 轻量确认可以先返回，但最后一次状态变化后须最终摄取，无需用户再触发 hook。查询的 JSON 轨迹解析也移出 DB 锁。

Todo 轮询以单条最新 role/is_error/attempt 查询判定，不加载完整轨迹；处理任务通过 `path_of` 取项目路径。prompt 初始预算 64KiB，保留任务正文、最新输入及完整回合边界；历史不足以全纳时明确写入省略标记，不截断 UTF-8 或伪造已见完整历史。单条必要输入超过预算则返回 InputLimit。对通过 argv 传 prompt 的 provider，还需按平台实际 argv/env 可用预算提前检查；不能把 JSON 请求上限当作操作系统命令行预算，也不能未经 provider 支持就切换到 stdin。

### 7.3 Locate

按有序匹配单向维护字节/行/Unicode 标量列游标，复用行边界；扫描工作应接近 `O(N + 返回内容)`，超长单行不能为每个命中重复从行首扫描。保持现有 1-based 行列和半开替换范围 `[start,end)`；修正协议注释中“含端点”的歧义，不改变现有实际坐标含义，也不引入 UTF-16 列换算迁移。

新增 LocateV2：初始输入文件上限 16MiB、命中上限 1000、单条 context 上限 1024 个 Unicode 标量、序列化回复上限 1MiB。结果携带 `complete`、`limit_reason`、文件 revision；已知前缀结果可返回，但必须声明不完整，不编造总命中数。context 裁剪有显式标记，不能改变真实命中坐标。

旧 LocateInFile 保持完整列表或错误的语义；超预算返回可读 Limit 错误，不能把截断列表伪装成“全部”。MCP 和新服务消费 V2 并向 agent 说明限制；本轮不承诺跨多次请求的搜索分页快照。

## 8. 启动、停止和通信

### 8.1 启动与后台回填

在日志初始化、数据库迁移、apps 对账和 socket 删除之前，取得产品 socket 对应锁文件的排他进程锁并持有到完全退出；锁文件不在退出时 unlink，防止不同 inode 上同时加锁。取得锁后才判断并清理陈旧 socket，不能把 connect 超时当作死亡证明；若锁已占用直接拒绝第二实例。

数据库必要迁移和轻量恢复完成后绑定监听；全量 transcript 发现、文件回填、Codex SQLite 摄取改为低优先级分批后台任务。发现阶段也须有界，不先收集全部文件名到 Vec。按需打开会话可提升该源优先级。

新增 BackfillState（NotStarted / Discovering / Ingesting / Complete / Failed / Cancelled，附计数、revision、rescan_needed 和允许列表错误）。目录不存在视为未安装/无数据；其它发现错误须计入失败，不得报告空目录成功。历史查询在回填中可返回已知数据，但支持该能力的 UI 必须展示加载状态；不能把空结果当成“确认无历史”。Complete 定义为本轮扫描发现的源均已处理或记录错误，不保证持续写入下的全局静态快照。

### 8.2 统一停止

Shutdown、SIGINT、SIGTERM、启动失败后的清理调用同一 shutdown coordinator：

1. 原子进入 Draining，关闭工作准入，取消排队创建/派发/回填；登记中的 CreateSession 持有活动令牌。
2. 等待已准入创建完成登记或自行清理，之后再枚举会话；不得出现最后一次扫描后又插入会话。
3. 将有内容的总结需求提交到持久队列，attempt 落为相应中断状态；本轮不等待新的 LLM 总结完成。
4. 结束会话/受管 headless、等待 child 回收及 bridge 租约释放；停止 apps 和 worker，刷新必要状态。
5. 移除自己持有的 socket，最后释放进程锁。Shutdown RPC 成功回复尝试发送；对端已断开也必须继续停止，不以回复写成功作为清理触发条件。

初始总停止预算 10s，子进程清理仍服从其 3s 子预算；超时记录具体未完成资源并强制结束可终止的执行。不可中断系统调用不承诺被安全抢占。重复停止请求复用同一过程；SIGKILL/断电依赖下次启动恢复，不能声称完成上述收尾。

### 8.3 UDS 预算与期限

使用有最大长度、支持取消的帧读取，按编码后的 JSON 字节计量，读取超过上限时不继续积累到换行。初始 client→server 帧上限 12MiB、server→client 16MiB；终端快照原始正文仍上限 1MiB。字段和操作另有更小预算，限制须在 base64 解码/大对象分配前尽可能检查。发出回复前也校验上限，不能发送半个 JSON 帧。

连接总上限 128，其中普通请求工作并发 16、等待 64；终端长期订阅不占普通请求执行名额，但占连接名额。过载使用稳定 Busy/Limit 诊断；若连错误帧也无法安全发送则关闭连接。控制/Shutdown 请求保留独立名额，避免普通负载导致无法停止。

初始 connect/write 上限各 3s、普通 RPC 总期限 10s、Attach 握手 5s、首次请求读取 5s；分阶段预算不能突破操作总 deadline。Todo 等长操作不直接等待其全部执行：新请求返回 attempt 并查询状态；旧长操作路径保持最长 600s 加清理余量的独立政策。长期订阅不设置普通总超时，但写阻塞超过 3s 关闭该流，交由 client 恢复；普通未订阅连接空闲 30s 关闭。

查询物化阶段另设初始通用 10,000 行/8MiB 估算内存预算，transcript 原始行预算 24MiB；超限返回明确 Limit，不返回伪完整前缀。编码器按转义后 JSON 字节提前失败，避免先构造完整超大编码帧。

帧上限可能拒绝过去无界的大查询，属于显式预算变更。新增分页读取或明确 Limit 错误，不静默截断已声明完整的数据。已有限制的总结/用量快照接口逐项盘点，不能用一句“统一 16MiB”忽略现有合法输入兼容性。

读取超时不能证明写请求未执行。现有写 RPC 不自动重试；新增任务幂等键仅适用于该新接口，不能据此声称 CreateSession、保存和所有写请求都已实现幂等。

## 9. 兼容、数据迁移与发布边界

- 新 DTO 位于 `dozer-model`，协议 envelope 位于 `dozer-core::protocol`，trait 位于 `dozer-services`；不修改 bytehost 的 AppRequest/AppReply 形状。
- 新能力用独立请求/回复表达；不向现有可直接构造的公开结构体添加必填字段，不向旧 SessionEvent 添加导致穷尽匹配破坏的变体。
- 新 trait 方法提供默认“不支持”或无状态结果；默认行为不能产生伪成功。新 Dozer 实现与 fakes 同步更新，现有消费者应可继续编译。
- task attempts 使用独立幂等迁移/表检测，不抢占 transcript 已使用的全局 `PRAGMA user_version`。不重写旧 Todo 完成状态、conversation ID 或既有总结记录。
- 文件权限/内容修复不回溯修改已有文件；迁移测试覆盖旧库和二次启动。降级旧二进制能忽略新增表，但无法识别新 attempt，降级前必须停清执行，不能保证旧轮询遵守新状态。
- Dozer 库 tag、外部消费者升级、推送和发布另行按实际授权执行；本次仅形成规格和计划。

## 10. 验收矩阵

| 编号 | 必须满足的行为 | 主要验证方式 |
|---|---|---|
| A1 | 普通项目文件可操作；末级/父级外链、绝对路径、`..`、替换链接不能导致项目外读写 | 临时目录、目录句柄与受控竞态屏障 |
| A2 | 保存保持 0755/0600；写入/复制元数据失败原内容不变；rename 后同步失败报告已提交但持久性未知 | 权限/ACL/xattr fixture 与故障注入 |
| A3 | stdin 不读、stdout 饱和、leader 先退、超时/取消/abort 均按政策清理；exit 42 不能成功 | 本地 helper 子进程和进程组观察，无真实 agent |
| A4 | attempt 并发去重、崩溃恢复、显式重试和 key 去重；失败不触发自动副作用重放 | 旧库迁移和状态机测试 |
| A5 | Lagged/断线恢复后仍见新 marker；Resume 不重放、Reset 不追加；A→B→A 的旧 epoch 被拒绝 | 小 ring、慢消费者、UDS 集成和面板 fake |
| A6 | client→GUI 未应用字节受每流/全局预算限制；tab 关闭释放队列、许可和连接 | 计数许可、暂停 UI ack 的确定性测试 |
| A7 | spawn 后立即退出、退出早于 bridge 启动均最终收尾；真实退出码、wait/reap、历史逐出可观测 | 生命周期屏障、短命 helper、句柄检查 |
| A8 | 大摄取解析不持全局 DB 锁；同源多触发不重复入库、半行/替换/截断行为正确；积压有界 | 注入慢 IO/解析器及提交屏障 |
| A9 | Todo 待处理判定不取完整轨迹，进程内服务不在 future 构造时做慢 IO | 查询/调用计数和阻塞接缝 |
| A10 | locate 坐标正确，高命中/长单行不平方级增长，V2 超预算明确不完整，旧接口不伪完整 | Unicode fixture、扫描计数与 release 基准 |
| A11 | 慢历史扫描不阻塞首次监听；回填状态诚实；两个实例并发启动只有一方获锁且另一方无副作用 | 假慢 backfill 与双进程测试 |
| A12 | 三种停止入口走同一路径，创建/停止竞态不留会话，回复失败也继续停止 | coordinator 与实际信号子进程测试 |
| A13 | 无换行超大帧、慢读写、并发过载受预算约束；订阅不因普通 RPC 总期限退出；写超时不自动重放 | UDS 假服务、短期限和字节上限 |
| A14 | 新/旧 client、daemon、服务实现组合行为明确；旧库幂等迁移；App envelope 不变 | JSON golden、能力协商与编译契约测试 |
| A15 | 现有面板/依赖/日志边界通过；CI 执行核心行为测试，不依赖真实 agent 登录 | 既有脚本与测试 job |
| A16 | 性能报告有相同 fixture/构建模式的前后结果和资源上界，未验证项明确列出 | 验收记录、计数器与基准产物 |

性能基准使用 release 构建、固定输入、至少 5 次样本和中位数/p95。定位算法优先以扫描次数证明近线性；同机 2× 输入的中位耗时目标不超过 2.5×，仅作基准告警，不作为易抖动单元测试断言。未知真实负载的 p99 绝对值不预先编造；T0 记录基线后在验收报告给出实测变化。

必须记录的指标：RPC 延迟/拒绝数、blocking 排队与执行数、摄取锁等待/字节/提交数、终端待应用字节/Resync 次数、会话历史正文与运行句柄、启动可连接时间、停止耗时、超时后残留/清理失败数。日志不包含终端输入、完整 prompt 或凭据。

## 11. 当前实现与验收状态

本机已实现的预算、测试证据和明确未验收项见 [后续补齐与验收记录](../../analysis/2026-11-02-dozer-runtime-optimization-completion.md)。实现完成不替代第 10 节最终验收；GUI、完整历史二进制矩阵和端到端 release 性能项目继续保留。
