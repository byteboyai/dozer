# ByteBoy 五仓库下一阶段评估

> 后续进展：Digger 已有远端 main，稳定化执行见 [实施记录](2026-11-03-byteboy-stabilization-progress.md)。本文保留评估时点事实。

日期：2026-11-03。范围：ByteHost、ByteUI、ByteGit、Dozer、Digger 当前本地代码、消费关系、规范与验证。本文给出投入顺序建议，不批准新架构、不实施跨仓修改或发版。

**建议：结束大范围框架优化阶段，先做一个有明确退出条件的稳定化批次，然后以产品功能为主线推进。** 当前最有价值的工程工作是发布组合可复现、门禁可信和真实交互验收；没有足够证据支持再启动一轮通用框架重写、全量拆文件或扩写规范。

五仓库整体已具备继续迭代的基础，但不能称为全部达到发行验收。尤其 Digger 的本地联调与声明发布依赖尚未对齐，必须先收口。

## 1. 本次基线与证据等级

| 仓库 | 本地 HEAD | 当前消费/发布关系 |
|---|---|---|
| bytehost | `0362608` | HEAD 对应本地 tag v0.4.21；两产品清单均引用 v0.4.21 |
| byteui | `eb08e57` | 当前 package 0.5.1；HEAD 在 v0.5.1 后新增 NotebookTabs；两产品清单仍引用 v0.5.1 |
| bytegit | `5472fa5` | 两产品清单引用 v0.9.2，对应该提交 |
| dozer | `6e389a30` | App v0.5.315；本轮运行时优化已提交，Digger 仍引用较旧 libs 快照 |
| digger | `44b84e7` | 共享 Dozer crates 引用 libs-2026-11-01e；本地 .cargo patch 覆盖多个库；没有配置 remote |

读取了实际边界代码和 CI，不仅依赖既有报告。编译/单测使用 HEAD 归档到 `/tmp/byteboy-assessment-20261103` 的隔离副本，避免更改相邻仓库。下文明确区分本次重跑、历史验收记录和建议；未运行 GUI、真实 agent、远端 CI，也没有验证远端 tag 的可访问性。

## 2. 应先收口的事项

### 2.1 发行阻断：Digger 当前声明的依赖不足以构建当前源码

证据：

- [Digger Cargo.toml](../../../digger/Cargo.toml:45) 钉 byteui v0.5.1，Dozer 共享库钉 libs-2026-11-01e。
- [shell.rs](../../../digger/crates/desktop/src/shell.rs:78) 已使用 `IconKind::NotebookTabs`；该枚举只在 ByteUI 当前主干新增，v0.5.1 不含它。
- [preview.rs](../../../digger/crates/desktop/src/adapters/preview.rs:1141) 使用新 `SaveOutcome`，同文件测试使用 `SaveFile.project_root`；所钉 Dozer libs 快照不含这些接口。
- 当前提交的 Cargo.lock 中多个共享包没有 git source。本次 HEAD 隔离副本直接执行 `cargo check --locked --offline -j 1 --workspace --all-targets`，首先因需要更新 lock 失败。恢复先前无 patch 检查的远端依赖锁，只用于进一步诊断 API 不匹配，不能代替修复当前已提交 lock。
- 既有 [11-03 Digger 验收报告](../../../digger/docs/superpowers/reviews/2026-11-03-code-optimization-completion.md) 已记录相同 API 缺口。本次进一步检查了当前提交、实际调用点和干净锁路径，具体编译结果见末节。

处理：发布包含所需接口的 ByteUI 与统一 Dozer libs 快照；按依赖顺序更新 ByteHost 中实际需要调整的钉版和两个产品的版本/lock；无 patch 的两产品构建及必要契约测试全部通过后才视为收口。ByteGit 当前没有发现必须发新版的理由。不要为追求所有库都有新版本而空发版，也不要仅消除 lock 报错就声称 API 兼容。

验收输出应是明确的版本/SHA 组合、git 来源锁文件和构建结果，而不是“兄弟目录 patch 下能运行”。

### 2.2 应先修的工程缺陷：Digger 依赖门禁会假通过

位置：[check-host-deps.sh](../../../digger/scripts/check-host-deps.sh:64)。

`hits=$(tree_of "$pkg" | grep -E "$bytehost_banned" || true)` 将 `tree_of` 中 cargo 失败与 grep 无匹配一并吞掉。本次用隔离 fake cargo 提供两个业务包、令所有 `cargo tree` 返回 42，脚本仍输出 `ok(审视了 2 个业务 crate)`，最终退出码 0。这是可复现缺陷，不是对沙箱限制的推测。

处理：先独立取得 tree，失败立即终止，再将 grep 的“无匹配”单独处理。补入 cargo 失败、空成员、违规依赖、自检恢复的变异测试；业务包默认纳入，避免仅依赖名称前缀而漏掉新 crate。Dozer 当前脚本已有分步失败处理和 8 个自测试，可参考行为，避免再复制一套逐渐漂移的实现。

依赖图检查应运行于隔离副本或严格锁定解析，不改开发者 lock。测试脚本的失败、环境受限和未执行必须分别记录。

### 2.3 发行验收缺口：真实交互与跨仓组合尚未闭环

- Dozer 运行时报告仍保留 GUI、真实 agent、完整历史二进制组合及端到端 release 性能待办。
- Digger 报告保留真实 IME/undo/焦点/页面 Retry/IPC-paint，以及受进程枚举环境限制的 app-host recovery。Rust Bridge 基准不能代表真实输入到绘制延迟。
- [ByteUI candidate workflow](../../../byteui/.github/workflows/candidate-check.yml:13) 仅手动触发；[ByteHost joint build](../../../bytehost/.github/workflows/joint-build.yml:10) 未纳入 Digger；Digger 没有 remote，存在 workflow 文件不等于远端执行过。

处理：先完成最小实机组合：保存后重开与异常恢复、编辑器 IME/undo/焦点、项目/tab 关闭重开、终端持续输出/断线恢复/resize/退出，以及该发行实际启用的应用生命周期。使用临时产品根和备份副本。FSEvent/进程枚举/窗口权限相关测试在正常 macOS 会话重跑。ByteHost 的孤儿恢复 fixture 在身份查询不可用后仍直接等待 `sleep 306` 退出；测试辅助代码也应加超时和失败清理，避免环境失败拖成数分钟等待，不应把这种等待当作生产恢复成功。

建立一次精确候选 SHA 的两产品联合检查和一次已发布 tag 的无 patch 检查。共享 API 变化时触发消费者检查，不能只等打 tag 后才发现破坏。历史二进制组合按实际支持范围保留；没有支持承诺的所有历史版本，不应成为无限扩大的验收清单。

## 3. 五个仓库分别应投入什么

| 仓库 | 代码现状判断 | 下一步投入 | 暂缓事项 |
|---|---|---|---|
| ByteHost | core/settings 纯契约与 desktop/chrome/platform 执行层分离；Apps 有单根锁、应用级锁分片、生命周期 gate、停止报告、客户端契约测试；可继续承载产品 | 稳定公开契约；补真实原生子视图/焦点/停止组合验收；共享 API 变动前做两消费方检查 | 通用插件总线、再造桌面框架、按文件行数全面拆分 |
| ByteUI | 小组件/token/共享菜单职责清楚；主题快照与缩放状态已加固；当前主要问题是新增接口未进入消费版本 | 发布所需增量；补菜单窗口、缩放、主题和键盘/焦点实机回归；按产品需求添加组件 | 全面 token 化所有常量、无产品需求的组件全集、多窗口独立主题系统 |
| ByteGit | 不向消费者泄漏 git2；读取预算、严格 patch、WorkspaceKey、合并/降级监听、取消写操作已有实现和回归 | 保持读取/监听 API 稳定；真实 FSEvent 验证；由实际产品功能推动新的 Git 操作 | 另造 Git 引擎、为假设中的 worktree/merge 场景提前铺完整框架 |
| Dozer | model/services/client/daemon/panel 已分离；运行时正确性与恢复补强有大量行为测试；组合根依然较大 | 先验收本轮稳定性，随后推进一个治理/验收的完整功能切片；修改相关路由时局部整理 | 直接实施未批准的 V2 Workflow Kernel/插件化；因文件大就重写 App/preview |
| Digger | Rust 编辑核心、TS 同步协议、增量结果、SQLite 台账/迁移和恢复路径已明显改善；发行依赖仍阻断 | 首先修版本/lock/门禁，然后完成真实编辑体验并发展写作工作流 | Rope/CRDT、通用 actor、再次更换编辑器、扩大同步协议抽象 |

说明：这不是“所有性能问题已经消失”的判断。ByteHost [service::blocking](../../../bytehost/crates/bytehost-apps/src/service.rs:632) 仍直接使用 spawn_blocking；未来扩大应用并发或提供批量入口时，执行/等待准入需要在库或明确的宿主契约上落实，不能只依赖 Dozer 外层请求限流。当前未做过载实测，不将它作为已复现的生产故障。

ByteUI 的主题/缩放仍是进程级状态；单产品统一主题符合当前用法，只有出现同进程不同窗口独立主题的实际需求才值得扩展。ByteGit 保留的旧同步 API 不自动拥有所有新 API 的预算与取消保证，消费方新增慢操作应明确选择适当接口。

大型源文件不是独立的延期理由。本次统计包含测试的 Rust 行数约为：ByteHost 5.1 万、ByteUI 0.88 万、ByteGit 0.69 万、Dozer 22.1 万、Digger 5.6 万。ByteHost manager 约 6337 行，但主要测试从 2262 行开始；不能把这些数字当作“生产逻辑都堆在一起”的证明。Dozer/Digger 的组合根仍值得在相关功能改动时按所有权拆解，保持单一状态源、类型化路由和生命周期，不引入动态总线来隐藏依赖。

已有 Git 复审的实例 owner、Files generation 和嵌套工作区问题，在当前代码可见 owner 与 WorkspaceKey 校验；不把旧报告的失败当成当前仍未修复的问题。

## 4. 规范只补能执行的共同约束

现有文档数量足够，缺口主要是当前状态和事实容易漂移。例如 Dozer CLAUDE 的 ByteHost 版本描述仍有 v0.4.10，而当前 Cargo 清单已是 v0.4.21；ByteHost manager 文件头仍描述单一全局写锁，实际已有应用分片锁。继续追加长篇规范会扩大维护面。

建议把以下五条作为简短的跨仓共同规则，并对应到已有脚本/行为测试：

1. **依赖与发布：** 支持组合记录精确 tag/SHA；候选 patch 验证与发行 locked 验证分开；一库一来源；依赖检查失败必须失败。版本事实由清单/lock 生成，文档不重复手抄。
2. **异步所有权：** 工作有 owner、取消、执行与等待预算；结果携带实例/请求身份；关闭/重建不能接受旧回包。取消已经产生副作用的操作，状态须明确。
3. **落盘语义：** 唯一权威数据源、版本迁移、revision/事务边界、提交结果与恢复路径；错误不能冒充成功，重试不能重复不可逆副作用。
4. **UI/信任边界：** 契约层不依赖平台；原生句柄由宿主拥有；可信预览与不可信页面分开；持续失败原位展示，瞬时事件使用已有通知机制。
5. **验证与完成定义：** 行为变化有能表达用户结果的测试；实机/端到端/性能证据不能互相替代；未执行、环境失败和代码失败分别记录。

改变公开行为、数据协议、所有权或权限时写 spec/ADR；纯内部可逆整理用简短变更说明和相关测试即可。现有历史报告保留，增加当前状态索引；不再为同一事实维护多份“最终”文件。

## 5. 推荐推进顺序与停止条件

### 批次一：短期稳定化，建议以 2–3 个工作日为首次时间盒

这是投入上限建议，不是工期承诺；若实机发现数据损坏或失控进程等问题，单独立项修复并重新评估。

- 修 Digger 假通过门禁并验证故障注入。
- 完成 ByteUI/Dozer libs 与消费者的版本/lock 收口，两产品无 patch、locked 构建和相关契约测试通过。
- 跑最小实机关键路径，保留问题、样本和支持组合；核心权限/恢复金丝雀有明确结果。
- 记录真实发行构建下的几个关键基线：编辑输入到绘制、终端停消费后的资源增长、摄取期间普通 RPC 延迟和关闭耗时。先测量，再决定是否需要专项优化，不将所有模块都纳入一次性能工程。

达到上述退出条件后，停止以“框架还不够完美”为理由扩大重构范围。架构美观、剩余无关 warning、所有历史版本组合和未测出的假设性能问题，不应继续阻塞普通功能迭代；发行所承诺的兼容、数据安全和进程清理必须保持门槛。

### 批次二：连续两个功能迭代，以功能为主

建议先按约 **70% 产品功能、20% 相关质量与回归、10% 工具/规范维护** 分配，之后依据故障和交付数据调整，不同时打开五个仓库的重构计划。

- **Dozer 候选方向：** 沿已有任务、attempt、会话、Git 变化与预览，交付一个从任务执行到证据查看、验收结论的完整切片。优先验证用户是否更容易判断 agent 的产物，而不是扩展更多互不关联的面板。具体范围另行产品立项；不预设必须引入 Workflow Kernel 或插件协议。
- **Digger 候选方向：** 选择一个来源/笔记到写作产物的真实工作流，把编辑、保存恢复和 Agent 辅助连成可完成的用户任务。先用真实文档和输入法找摩擦，不先更换数据结构。
- **共享库：** 只处理这些切片明确需要的能力与真实重复；至少两个实际使用点、相同语义与稳定变化轴，再考虑上提。两份相似代码不必立即合并，特别是保存/迁移等有不同权限和恢复语义的逻辑。

### 何时重新切回专项框架优化

出现数据丢失、权限越界、无法回收的进程、无界增长，立即优先修。或者同一框架缺陷反复造成产品故障、两个产品为同一语义反复修改不同适配、实测关键路径超出明确预算、普通切片持续被同一组合根阻挡时，再做有指标和退出条件的框架任务。

没有这些证据时，继续功能开发，通过真实使用决定下一次抽象边界。

## 6. 本次验证结果

验证日志位于 `/tmp/byteboy-assessment-20261103`。库仓库不跟踪 Cargo.lock 时，仅在隔离副本生成临时锁运行 offline 测试，不将它称为发布 locked 验证。下列“历史”项目未在本轮全部重跑。

| 验证 | 本次结果 |
|---|---|
| ByteUI HEAD lib | 133 passed；`cargo test --offline -j 1 --lib` |
| ByteGit HEAD all-features | 222 passed、9 failed；失败全部为真实监听用例无法启动 FSEvent stream，未修改或跳过断言；本轮不能标 all-features 全通过 |
| ByteHost 依赖门禁与变异自检 | 隔离副本通过；包括纯契约/产品依赖/桌面层依赖与钉版变异 |
| Dozer panel boundary / log scope | 通过 |
| Dozer host-deps | 通过，8 个自测试通过 |
| Dozer shared-deps `--offline --rev HEAD` | 0 阻断；本地追踪提示 main 领先 origin/main 3 提交，不以此代替实时远端核验 |
| Digger shared-deps offline | 失败：24 个共享包缺 git source，符合当前 lock 事实 |
| Digger HEAD 无 patch locked 检查 | 失败：当前 lock 需要更新 |
| Digger host-deps 故障注入 | cargo tree 故意返回 42，脚本退出 0，假通过已复现 |
| ByteHost 其它 8 crate lib | 382 passed（client 的 lib 为 0；客户端主要契约集成测试未在本轮重跑） |
| ByteHost Apps lib | 484 passed、3 failed；进程身份/孤儿恢复相关用例，详见最终补记 |
| Digger 补充 API 编译诊断 | 失败：NotebookTabs 缺失，详见最终补记 |
| Dozer 最新完整运行时验证（历史记录） | daemon 547；client 集成 16；MCP 19；产品隔离 5，详见 [11-02 完成记录](2026-11-02-dozer-runtime-optimization-completion.md) |
| Digger 最新完整验证（历史记录） | editor/属性/协议、store/paths、desktop 726、TS 30 及 release Bridge/SQLite 样本，详见其 11-03 完成记录；该记录已明确 GUI/干净依赖未通过 |

本次不作“全仓零缺陷”认证。代码检查覆盖关键边界与近期变化，重点是判断下一阶段投入方向；没有修改产品或共享库实现、没有发布 tag、没有推送。只新增本评估及必要证据文件，临时检查导致的 Dozer lock source 变化已按检查前内容恢复。

## 7. 最终补记

Digger 在仅修复隔离副本依赖锁、保持所声明远端版本不变的诊断构建中，再次报 `E0599: IconKind::NotebookTabs` 不存在，位置 `shell.rs:78`，退出码 101。这不是网络/缓存缺失导致的失败。旧 Dozer libs 的 effects 源码还可直接确认 `SaveFile` 不含 project_root，且没有 SaveOutcome；本次编译首先停在产品 lib 的图标错误，未将后续测试编译错误声称为本轮再次全部执行所得。

ByteHost 其它 8 crate 的 lib 结果：chrome 70、client 0、core 56、desktop 95、panel 102、settings 22、settings-ui 7、webview 30，共 382 passed。构造/状态机/纯策略测试通过不等于真实 GUI 窗口已验收。

ByteHost Apps 最终结果为 484 passed / 3 failed。两条 supervise 测试期望 Killed 实得 Stale，manager 孤儿恢复测试的 `sleep 306` 自然退出后触发“应被信号终止”的断言失败，总运行 306.74s。结合当前环境的进程查询限制，这些结果不能被记作已通过的清理保证；需要在具备进程身份查询能力的正常 macOS 环境复验。没有为了取得绿灯跳过断言或修改生产安全检查。
