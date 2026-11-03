# Dozer 代码深入分析与优化建议

分析日期：2026-11-02。基线：`a9e02c5d`。本次只做分析与隔离验证，未修改业务代码。

后续文档：[优化规格](../superpowers/specs/2026-11-02-dozer-runtime-optimization-design.md)与[实施计划](../superpowers/plans/2026-11-02-dozer-runtime-optimization.md)，状态均为待实施。

当前最值得投入的是文件操作正确性、子进程生命周期和终端数据链路，其次才是模块拆分。已有的 model/services/panel 边界可以继续沿用，无需先推动一次架构重写。

## 分析范围与证据口径

本次盘点了 `crates/` 下的 24 个 crate，重点沿以下调用链阅读源码，并非逐行审计全部模块：

- Agent 面板 → `SessionService` → UDS client → daemon → PTY → 终端事件回传。
- Hook → transcript 摄取 → SQLite → Todo/headless → 总结任务。
- 精确编辑与预览保存 → 路径校验 → 冲突判断 → 写盘。
- daemon 启动、历史回填、退出，以及 CI 和 GUI 消息路由。

“复现确认”表示调用当前源文件或原样抽取的函数做了隔离执行；“静态确认”表示代码路径明确，但尚未跑完整 GUI/daemon 场景；“建议”表示需结合测量决定的优化方向。

完整 workspace 测试未完成：尝试 `cargo test -p dozerd --lib file_mutation --offline` 时，环境把构建目录指向工作区外的 `../target`，沙箱拒绝打开构建锁。随后在 `/tmp/dozer-review-20261102` 建立独立验证工程，直接引用 `file_mutation.rs`、`text_save.rs`，提供最小类型/文案接缝，并原样抽取 `run_task_and_capture`。两个源文件自带的 **20 个测试全部通过**；这不能替代完整 crate 测试，也说明现有测试没有覆盖下述边界。构建尝试引起的锁文件变动已还原。

## 优先级总览

| 编号 | 优先级 | 优化事项 | 证据 | 主要收益 |
|---|---|---|---|---|
| 1 | P1 | 修复文件末级符号链接越界 | 复现确认 | 保证项目文件边界 |
| 2 | P1 | 统一 headless 的超时、终止和失败语义 | 复现确认 | 避免超时后继续执行、失败误报成功 |
| 3 | P1 | 修复终端 Lagged 后永久停更，补上有界背压 | 静态确认 | 高吞吐下恢复输出并约束内存 |
| 4 | P1 | 保存时保留权限，统一安全写盘行为 | 权限问题复现确认 | 避免脚本失去执行位、私有文件权限放宽 |
| 5 | P2 | 隔离同步 IO，缩短 transcript 数据库锁范围 | 静态确认 | 减少 daemon 请求尾延迟 |
| 6 | P2 | 消除 locate 的平方级扫描和无界结果 | 复现确认 | 大量命中时降低 CPU 和返回体开销 |
| 7 | P2 | 分离会话退出状态、进程回收与历史保留 | 静态确认 | 避免退出竞态和长期资源增长 |
| 8 | P2 | 缩短启动关键路径，统一停止流程 | 静态确认 | 历史数据增长后仍能及时连接 |
| 9 | P2 | 为 UDS 增加帧预算、请求期限和准入控制 | 静态确认 | 约束异常客户端与卡住的请求 |
| 10 | P3 | 按职责拆热点，补行为测试门禁 | 建议 | 降低跨层修改成本 |

P1 表示应优先修复的正确性问题；P2 表示规模增长或特定时序下的可靠性/性能风险；P3 表示后续维护投入。

## 1. 精确编辑可以沿末级符号链接修改项目外文件

位置：[file_mutation.rs:30](../../crates/dozerd/src/file_mutation.rs#L30)。

`resolve_project_path` canonicalize 的是父目录，之后拼回文件名，并通过 `is_file()` 判断。`is_file` 和后续读写会跟随文件本身的符号链接，因此父目录在项目内并不保证最终目标在项目内。

隔离复现：项目内 `link.txt` 指向项目外的 `outside.txt`，调用 `apply_precise_edit` 后返回成功，外部文件从 `old text` 变为 `NEW text`。同一解析函数也用于 `locate_in_file`，所以读取边界同样受影响。

建议先明确末级符号链接策略：若允许，需校验最终目标 canonicalize 后仍在根目录内；若不允许，则显式拒绝。该补丁能修复本次静态链接复现，但仅增加 canonicalize 仍不能解决检查与打开之间的替换竞态。需要强边界时，应通过目录句柄和受控打开操作约束路径遍历，并基于已打开文件校验身份。

验收至少覆盖：普通文件、项目内链接、项目外链接、父目录链接、绝对路径、`..`、检查后链接被替换。不要把“多做一次字符串前缀检查”当作完整修复。

## 2. Headless 超时没有结束实际执行，退出失败也可能被当成成功

位置：[headless_agent.rs:472](../../crates/dozerd/src/headless_agent.rs#L472)、[headless_agent.rs:560](../../crates/dozerd/src/headless_agent.rs#L560)。

两条执行路径都先写 stdin，再对 `wait_with_output` 包 timeout；未设置 `kill_on_drop(true)`，也没有在超时分支显式 kill/wait。任务处理路径还直接返回 stdout，没有检查 `output.status.success()`。

原样抽取 `run_task_and_capture` 后确认：

- `sleep 30` 在 100ms 后返回 `Timeout`，此时 `kill -0` 仍确认子进程存活，验证程序随后主动终止了它。
- 对不读 stdin 的进程写入 1MiB 数据，设置 50ms 超时，实际约 513ms 后才返回成功。stdin 写入阶段未被期限覆盖；若对端长期不读，等待可以更久。
- `exit 42` 返回 `Ok("")`。结合 [task_processor.rs:104](../../crates/dozerd/src/task_processor.rs#L104) 的回合落库逻辑，失败可能变成普通 AI 回合；`needs_processing` 又只看最后一条是否为 AI，后续自动轮询会跳过它。

建议在 dozerd 内建立一个小型进程执行模块，统一绝对 deadline、stdin/stdout 并行泵送、输出字节上限、取消、退出码和 stderr 摘要。超时覆盖 spawn 后全部阶段；失败后终止并等待回收。`kill_on_drop` 可作兜底，但对子进程派生的进程树，还需按平台实现进程组终止，不能据此宣称已清理所有后代。

已有 [group_adapter.rs:201](../../crates/dozerd/src/group_adapter.rs#L201) 设置了 `kill_on_drop(true)`，可以参考其取消处理；复用时仍需补齐 stdin 和输出预算。任务执行失败应有明确状态，不应只编码进普通 AI 文本。

编写实施规格时补查确认：`summary_pipeline` 已调用更完善的 `summary_provider`，后者已有有界读取和进程组清理；本节复现针对 Todo 捕获函数及旧 headless 执行路径，不能据此判定所有总结调用都有同一缺陷。统一执行机制时应提取并保留现有 provider 的保护，而不是重新实现一份较弱执行器。

## 3. 终端 Lagged 后会停止推送，而上层没有恢复

位置：[server.rs:1594](../../crates/dozerd/src/server.rs#L1594)、[client/lib.rs:1515](../../crates/dozer-client/src/lib.rs#L1515)、[panel-agent/update.rs:535](../../crates/dozer-panel-agent/src/update.rs#L535)。

代码链路如下：daemon 的 broadcast 接收者落后 → 写出 `lagged; reattach` → `sub = None`；client 转成 `TermEvent::Lagged`；面板只记录警告并 `continue`。连接仍在，但 daemon 已不再为它转发终端事件，因此结果可能是终端持续停更，而不只是丢掉一部分历史输出。

另一处问题是 client 把终端字节放进 `unbounded_channel`。daemon 的 1MiB ring 和 1024 个 broadcast 事件并不能限制这个下游队列。下游消费停顿时，字节积压仍然可能持续增长；面板转发之后的 GUI 队列也应一起检查，避免只把积压换个位置。

建议把终端链路作为一个完整改动处理：

1. 保留已消费 offset；目前 client 的 Output 转换丢弃了协议中的 offset，面板也丢弃了 attach 返回的 next_offset。
2. Lagged 后自动重连/重新 attach；offset 超出 ring 范围时发送明确的“替换快照”语义，避免把快照当增量追加导致重复渲染。
3. 按字节预算限制队列，合并连续输出，按帧批量交付 UI。仅按消息条数限制无法约束可变长度数据。
4. Agent 状态和退出状态使用独立的可恢复状态通道，避免被高频输出挤掉。

验收：持续输出时人为暂停消费者，恢复后仍能看到新增 marker；内存受预算限制；快照重同步不重复追加；退出状态最终送达。本项为静态路径确认，尚未跑 GUI 压测。

## 4. 原子保存会改变文件权限，两套写盘实现也不一致

位置：[text_save.rs:61](../../crates/dozer-preview-core/src/text_save.rs#L61)、[file_mutation.rs:195](../../crates/dozerd/src/file_mutation.rs#L195)。

预览保存创建临时文件后 rename，没有把原文件权限复制到新文件。本机隔离验证中：

- 原权限 `0755` → 保存后 `0644`，脚本失去执行位。
- 原权限 `0600` → 保存后 `0644`，私有文件的访问权限被放宽。

另外，daemon 精确编辑使用 `std::fs::write` 原地覆盖。写入中途失败可能留下截断内容；它和预览层的原子替换行为不同。精确编辑的 `expected_text` 只检查选区内容，也不能保证读文件到写回之间没有其他修改。仅靠 rename 可以避免半文件，不能解决丢失更新。

建议提炼一个不依赖 UI 的文件写入基础实现，调用方继续负责各自产品策略。替换前保留适当的原权限；明确符号链接、ACL/xattr 的支持策略；对读取原格式失败区分 NotFound 与其他错误，避免一律回落默认格式。格式探测声明只需要前 64KiB，但当前 `std::fs::read` 仍先读完整文件，也可以改为有界采样。

并发方面，应串行化 Dozer 自己对同一文件的写入，并在提交前检查文件身份/revision。对外部编辑器，普通文件系统缺乏通用的内容 CAS；需要明确冲突检测的实际保证范围，不能把一次 mtime 检查描述成严格原子条件写。

## 5. 同步 IO 和解析占用异步 worker，且 transcript 全程持数据库锁

位置：[server.rs:622](../../crates/dozerd/src/server.rs#L622)、[transcripts/mod.rs:495](../../crates/dozerd/src/transcripts/mod.rs#L495)、[store-project/service.rs:70](../../crates/dozer-store-project/src/service.rs#L70)。

daemon 请求处理直接执行 PTY write/flush、SQLite、文件读写和 transcript 摄取。`ingest_session` 在取得数据库 Mutex 后，打开文件、读取全部未摄取内容、解析 JSON，再写入事务；同一 `TranscriptStore` 的读取会被整个过程阻塞。首次摄取或大段新增内容不能套用注释中“读几行，毫秒级”的假设。

进程内服务还有相似误区：`let r = self.store.list(); Box::pin(async move { r })` 把同步工作放在 future 构造前执行，并不会避免阻塞调用线程；`ProjectStore::list` 还会做 Git 探测。将同步工作移入 `async {}` 本身也不会解决此问题。

建议分阶段处理：

- 用带并发预算的 blocking 执行边界隔离文件/Git/SQLite 工作，避免无上限地堆积 `spawn_blocking`。
- 摄取采用分批读取和提交。读文件与解析移出全局 DB 锁，但必须增加同会话串行化或 offset/revision 再校验，避免并发摄取重复提交。
- 数据库 actor 或专用线程是可选实现，重点是明确队列、事务和锁范围。不要先盲目引入连接池或只打开 WAL；它们不能消除同一个 Rust Mutex 带来的串行化。
- [task_poller.rs:81](../../crates/dozerd/src/task_poller.rs#L81) 仅为判断最后一条角色，却拉取全部回合并解析轨迹；应增加轻量的最新回合查询。`task_processor` 再拉取全部历史、拼接全部 prompt，也应有上下文预算，并使用 `path_of` 代替列出全部项目再筛选。

先测量摄取耗时、锁等待、阻塞任务排队及 PTY write 的 p95/p99，再决定是否拆读写连接。

## 6. Locate 对高命中文件呈平方级增长

位置：[file_mutation.rs:61](../../crates/dozerd/src/file_mutation.rs#L61)、[file_mutation.rs:85](../../crates/dozerd/src/file_mutation.rs#L85)。

每次命中，起止坐标都从文件头重新扫描；context 又单独查找行边界。命中数随文件长度增长时，坐标计算接近 `O(N²)`。结果数量和每条 context 长度也没有预算，超长单行可能被重复复制多次。

直接调用当前源码，输入为重复的 `x\n`、query 为 `x`，debug 构建单次测量：

| 文件字节 | 命中数 | 耗时 |
|---:|---:|---:|
| 20,000 | 10,000 | 0.951s |
| 40,000 | 20,000 | 3.675s |
| 80,000 | 40,000 | 15.135s |

这是构造的高命中场景，绝对耗时不代表 release 产品性能；翻倍输入约四倍耗时，与源码复杂度一致。

建议沿匹配位置单向维护行列游标，或建立行起点索引并复用同一行的字符映射。避免只优化行号后，长行列号仍反复从行首扫描。补充命中条数、context 字符数、文件字节数和总返回字节预算；超限要让调用方知道结果不完整，不能静默截断。保留中文、emoji、多行 query 的坐标约定测试。

## 7. 会话退出事件不持久，历史保留同时保留了运行期资源

位置：[session.rs:134](../../crates/dozerd/src/session.rs#L134)、[server.rs:515](../../crates/dozerd/src/server.rs#L515)、[registry.rs:17](../../crates/dozerd/src/registry.rs#L17)。

PTY 读线程在 EOF 时广播一次 Exited；自然退出总结的订阅在 spawn 后创建，IDE bridge 的退出订阅还要等异步 `session_started` 完成。短命进程可以先退出、后订阅，broadcast 不回放旧事件，而 `Session` 自己仍持有 sender，接收者未必会通过 Closed 结束。由此可能遗漏自然退出总结和 bridge 清理。这一竞态来自代码时序，尚未动态复现。

Registry 有意保留已死会话以支持 scrollback，这个产品行为本身合理；问题是它保留整个 `Session`，包含 master、writer、child 和输出缓冲。没有 TTL、条数或总字节预算。若保留 1000 个都写满 ring 的已退出会话，仅正文就约 1GiB，尚未计入句柄和其他开销。

建议将可回放的 `SessionLifecycle` 与输出数据分离；观察者订阅后读取当前退出状态，并保证收尾只执行一次。单独负责 child wait/reap，区分进程退出与 PTY EOF，获取真实退出码；当前发送的 code 恒为 None。退出后释放运行期句柄，把元信息和有预算的历史缓冲作为另一种记录保留，而非直接删除用户所需历史。

## 8. 全量历史回填阻塞监听启动，停止路径的收尾行为不一致

位置：[daemon.rs:146](../../crates/dozerd/src/daemon.rs#L146)、[daemon.rs:222](../../crates/dozerd/src/daemon.rs#L222)、[server.rs:622](../../crates/dozerd/src/server.rs#L622)。

daemon 在进入 `serve`、绑定 socket 前同步发现和回填所有 transcript，并摄取 Codex SQLite。历史增长会直接拉长“可以接受连接”的时间，而不仅是历史面板加载时间。

建议数据库迁移和必要状态恢复完成后尽早监听；历史摄取转成有界后台任务，分批提交并显式报告加载状态。读取历史功能要明确回填中与完整结果的区别。

停止方面，`Request::Shutdown` 会提交会话总结并 kill 存活会话；SIGINT/SIGTERM 分支只显式停止 apps、设置总结 worker 标志并移除 socket，没有复用该会话收尾流程。应统一幂等的停止过程：停止接收新工作、持久化必要任务、按期限结束子进程和 bridge、停止 worker、清理 socket。需用“排空新建会话任务”的同步机制覆盖创建/停止竞态，不能只靠再扫一遍 registry。

`ensure_single_instance` 是 connect 探测，并非原子的进程互斥。两个实例同时启动仍存在检查窗口；可在任何副作用前取得并持有进程锁。超时探测也不宜直接等价于“旧实例已经死了”。这是独立于本次回填性能问题的启动可靠性建议。

## 9. UDS 缺少统一的帧大小、请求期限和并发预算

位置：[server.rs:457](../../crates/dozerd/src/server.rs#L457)、[client/lib.rs:214](../../crates/dozer-client/src/lib.rs#L214)。

两端都使用 `BufReader::lines()` 接收 JSON；通用 client roundtrip 没有内置 deadline；server accept 后直接 spawn，每个连接均可一直等待下一行。没有换行的长输入可以持续占用缓冲，接受连接但不应答的服务可以让请求一直等待。

建议引入有最大帧长的读取层；限制活跃连接、普通请求和昂贵操作的并发数；为 connect、write、普通 reply、attach 握手设置清晰的期限。长期终端订阅无需沿用普通 RPC 总超时。大快照和完整会话查询应有单独预算，或改为分页/分块，不能用一个过小的全局限制破坏正常功能。

请求超时不代表服务端没执行，尤其是 CreateSession、写文件、派发任务；新增自动重试前必须考虑请求幂等标识或结果查询，不能无条件重发写请求。

## 10. 在修复行为后，按职责拆热点并建立测试门禁

值得保留的基础：`dozer-model` 统一领域形状，`dozer-services` 提供可替换接口，面板经服务取数；预览核心已独立；`dozer-git-runtime` 已有 generation 校验、订阅身份和并发调度。这些都是后续优化的支点。

热点拆分建议：

| 位置 | 当前耦合 | 建议拆分边界 |
|---|---|---|
| `dozerd/server.rs` | transport、领域请求、会话收尾和 hook 摄取混在一个 handler | transport 与按领域分组的 handler；共享显式 ServerContext |
| `dozer-client/src/lib.rs` | RPC transport、协议适配、各服务实现集中 | 公共 transport + session/project/summary 等服务模块 |
| `dozer-app/src/app/update.rs` | 大量面板路由及窗口状态分支 | 按领域组合路由，保留消息归属、revision 和失效规则 |
| `dozer-app/src/platform/window_events.rs` | 焦点、WebView、窗口生命周期集中 | 按原生资源所有权拆分，先测焦点/关闭行为再移动 |

不要按文件总行数机械拆分：例如 `panel-preview/pane.rs` 虽有 7549 行，首个测试模块前约 1825 行，剩余很大部分是测试。先识别真正的生产职责和状态所有权。

当前 [consumer-check.yml](../../.github/workflows/consumer-check.yml) 执行格式、编译与依赖/边界门禁，没有运行 Rust 行为测试。建议先增加与本报告问题对应的、无需真实 agent 或 GUI 的测试：文件边界和权限、进程超时后消失、非零退出、终端滞后恢复、会话退出状态回放。再逐步加入纯逻辑 crate 和现有稳定单元测试；需要示例二进制或桌面的集成测试单列准备步骤。

## 建议落地顺序与验收

1. **文件操作修复**：末级链接边界、权限保持、精确编辑安全写盘。验收外部目标不变、0755/0600 不变、写失败原文件不被截断。
2. **Headless 执行修复**：deadline 覆盖 stdin、超时取消、kill/reap、非零退出和输出预算。验收失败不会形成普通成功回合，超时后执行进程确实结束。
3. **终端链路修复**：offset 贯通、有界队列、Lagged 重订阅和快照重置。验收暂停/恢复消费者后输出继续，内存有界。
4. **性能优化**：locate 单向扫描、轻量 Todo 判定、摄取批处理和锁范围。验收通过输入规模曲线和并发请求尾延迟，而非只看小样本均值。
5. **长期运行优化**：生命周期回放、运行期句柄释放、历史预算、后台启动回填和统一停止。
6. **最后做结构整理**：围绕已明确的职责移动代码，并把上述行为测试加入 CI。

建议记录六类指标：RPC p95/p99、transcript 摄取/锁等待、终端排队字节/重同步次数、活跃与保留会话的句柄和内存、启动可连接耗时、进程超时后的残留数。只有真实产品负载数据才能进一步确定优化幅度；本报告不据静态阅读估算整体性能提升百分比。
