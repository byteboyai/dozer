# Dozer / Digger 最新重构交付与发布就绪交接计划

> 日期：2026-11-01。状态：待其他 agent 执行。
> 用户目标：两款产品完整包含最新重构成果，并具备发布条件。
> 本任务交付计划；不执行 App 发版、升级安装、版本号递增或 GitHub Release。

## 1. 开始前必读与判断标准

1. [刷新链路规格](../specs/2026-10-31-bytegit-refresh-optimization-design.md)：接口、兼容与 A1–A9 验收的真相源。
2. [原实施计划](./2026-10-31-bytegit-refresh-optimization.md)：T0–T13 的详细操作与已有实施记录。
3. [审核报告](../../analysis/2026-10-31-bytegit-implementation-review.md)：四项缺陷和复现证据。
4. [审核修复说明](../../patches/2026-10-31-bytegit-audit/README.md)：已落地内容、待应用补丁及测试范围。

本计划安排后续工作与交接，不替代原规格，不缩减 T6–T13。只有应用真实执行路径接入、远端依赖可复现、门禁和集成验收通过，才可以写“最新重构完整交付、具备发布条件”。新增了 crate、单元测试通过、提高版本号均不足以得出该结论。

## 2. 当前事实与阻断

以下是交接时本地事实；接手 agent 必须重新核对，不能将记录当作始终有效的 HEAD。

| 范围 | 当前状态 | 还缺什么 |
|---|---|---|
| bytegit | `4a4951b`，v0.9.0 已有身份、快照、新监听 API | R3/R4 修复仅在 Dozer 的补丁中；原仓库未应用、未发布修复版 |
| Dozer | HEAD `f5bd3ac5`，App 0.5.312；新 runtime 和 R1/R2 修复在未提交工作树 | T2 Files 映射与 T6–T13；App 尚未接入 GitRuntime |
| Digger | HEAD `2b21b0f`，App 0.1.7；仍声明 bytegit v0.8.1 | runtime、面板新接口、宿主接入和依赖收口；未配置 Git remote |
| ByteUI | HEAD `541554f` 有更新，声明依赖仍是 v0.5.0 | 核对两个产品实际需要的 API 是否已包含在可获取 tag 中 |
| 依赖门禁 | 两产品 `check-shared-deps.sh --offline` 均失败 | Cargo.lock 含本地 patch 来源，尚非纯远端依赖闭包 |
| 测试 | 隔离副本 bytegit 默认 157 / all-features 223，runtime 30 项通过 | 完整产品构建、运行路径与无 patch 集成验收未完成 |

本会话只允许写 Dozer 与临时目录。下一执行者需要分别具备 bytegit、byteui、Dozer、Digger 工作目录的写权限；发布共享库还需要对应远端权限。**补丁生成或隔离副本通过，不等于原仓库已落地。**

Dozer 还存在 agent_paths、headless_agent、summary_provider、transcripts 等其它会话的未提交修改。先记录并与其执行者确认归属；不要将其一并提交或声称已覆盖测试，不要 reset/stash/覆盖这些改动。runtime 和本组文档本身也未提交，创建隔离 checkout 时需要显式带入所需文件，不能只 checkout HEAD 后误以为修复已经存在。

## 3. 执行分工与文件所有权

用户自行安排其他 agent。下列为可直接转交的任务包，并不启动代理或自动授权发布。

| Agent | 任务包 | 主要写入范围 | 前置依赖 |
|---|---|---|---|
| A | 底层与组件依赖收口 | bytegit、byteui | G0 基线 |
| B | 共享 runtime、面板、存储 | Dozer `dozer-git-runtime`、`dozer-panel-*`、`dozer-store-project` | A 的接口/修复可用 |
| C | Dozer 产品接入 | Dozer `dozer-app` 宿主适配与消息循环 | B 的数据入口稳定 |
| D | Digger 产品接入 | Digger desktop 宿主与消息循环 | B 的数据入口稳定 |
| E | 跨仓集成与最终验收 | 根 Cargo.toml/Cargo.lock、共享版本清单、验收报告 | A–D 交付 |

共享面板和 runtime 的接口由 B 维护，C/D 发现缺口交给 B 统一修改。根依赖与锁文件最终由 E 收口；其他 agent 交付具体依赖需求和修改补丁，避免并行 cargo update 相互覆盖。B 负责 Files 库的后台目录 IO；C/D 分别负责各自宿主的目录结果接收和磁盘策略。agent 没有负责目录的权限时记录阻断，不得把“做了补丁”标记为仓库交付完成。

## 4. G0：冻结基线与执行范围

- [x] 读取各仓库适用 AGENTS.md；记录工具链、HEAD、dirty 状态、本地 patch、远端和现行门禁。（三仓均无 AGENTS.md；Dozer 用 CLAUDE.md。）
- [x] 盘点当前重构文件，保存与本任务相关的 diff/未跟踪文件；其它会话修改单列。
- [ ] 建立各自隔离 checkout 和可写 cargo/target 目录，保留主工作树的本地配置。（本轮仍使用主工作树 + 本地 patch；隔离 checkout 留给 E。）
- [x] 复现已通过的底层/runtime 测试；重新建立两个产品的构建、格式、clippy、测试基线，记录真实失败。（Digger 与完整 release 构建未执行。）
- [x] 建立接口/提交依赖清单与 `docs/superpowers/analysis/2026-11-01-bytegit-release-readiness.md` 验收报告。

G0 退出条件：所有执行者能明确回答“基于哪份源码、负责哪些文件、如何验证、哪些权限或外部条件尚缺”。

## 5. Agent A：bytegit 修复及共享依赖版本

- [x] 在 bytegit 检查并应用 `bytegit-fixes.patch`，复核未与其它修改冲突；不要先应用历史 regressions 补丁。（提交 `8763899`。）
- [x] 验证 R3 rescan 优先级与 R4 linked worktree 的 exclude/attributes/grafts/alternates 监听、删除/重建恢复和降级反馈。
- [x] 核对身份重新发现、普通目录新建 Git、缺失 refs 等尚未完成的 T3 子项，明确由底层还是宿主实现并提供测试。（bytegit 负责 FULL_RESCAN/degraded 与目标重建；普通目录首次 `.git` 与身份重接线由 runtime/宿主处理。缺失 refs 由 worker 定期重试并恢复。）
- [x] 完成原 T12 的严格 patch API 与可取消 Git 子进程接口，保持旧 API、hook/filter/冲突行为；向 B/C/D 交付迁移说明。（`v0.9.2` 增加 `OperationOptions`/可取消 clone/checkout：独立进程组、超时、stderr 256KiB、持续排空；旧 API 不变。产品取消 UI 尚未接。）
- [ ] 核对 ByteUI 已提交新接口与产品需求；需要新 tag 才能构建时，在 byteui 完成验证及版本收口，禁止产品靠未发布本地接口通过。（已核对：ByteUI HEAD `541554f` 无 tag，最新 tag 仍 v0.5.0。）
- [ ] 准备共享库版本发布材料：确切 commit、建议版本、API 变化、默认/all-features 测试、平台范围及 tag 可获取性证据。（bytegit 修复提交尚未发 tag。）

**库版本发布边界：** 用户当前要求是推进到发布就绪，没有要求本轮实际发版。已有授权覆盖共享库发布时按授权执行；否则把“所需共享 tag 尚不存在”记为外部阻断，并交付已验证提交与发布清单。不得引用占位 tag 或绕过依赖门禁。App 本身不在此任务包发版。

交付给 B/E：bytegit 修复实际落地提交、稳定接口、共享库依赖清单、测试结果、剩余外部阻断。没有共享库写权限或 tag 不可获取时，不得宣告 G1 完成。

## 6. Agent B：共享实现真正可供两产品消费

覆盖原 T2 剩余项、T5 修复复核、T6、T9/T10/T11 的共享库部分。

- [x] 复核当前 R1/R2 修复和执行中退订重开的测试，确保查询/监听实例隔离、真实并发许可和同工作区单任务约束有效。（runtime 14 单测/15 集成含 audit 回归通过。）
- [x] Files 增加 `file_statuses_from_entries` 纯映射与快照应用入口；应用结果不再触发隐式分支读取。（`Message::SnapshotApplied` 直接应用文件状态 + GitInfo，legacy 路径保留。）
- [x] Project/Agent 增加所需纯数据入口；Agent cwd 在另一工作区时有独立订阅。（Project `Message::SnapshotApplied`；Agent `apply_workspace_override`。独立订阅的宿主接线留给 C/D。）
- [ ] 完善 runtime 的普通非 Git 目录、身份切换、watch 降级与恢复、stale 状态和所有排队边界，不将 HIDDEN 当作 Git 忽略依据。（身份变化为 `identity_mismatch`、watch 降级/stale/排队边界已覆盖；普通非 Git 目录订阅与自动换 key 未做。）
- [x] Usage summary 单次遍历与有界缓存；保留窄 provider 接缝、NoGitStats 和现有时间/路径语义。（`summary` 默认方法 + DoGitStats 单次日桶遍历 + 64 项/64MiB LRU 缓存，键含 WorkspaceKey/HEAD/边界指纹。）
- [x] Files 目录读取移出 UI，按路径刷新并保留展开/选择/搜索语义；提供 C/D 可用的版本化结果接口。（`begin_tree_refresh`/`apply_tree_refresh` + `read_dir_snapshots`，每工作区单飞行任务、generation 校验、pending 合并。）
- [x] ProjectStore 的 Git 派生计算移出 DB 锁；加入慢查询不阻塞其它数据库访问的测试。（`RawProjectRow` + 锁外 `ProjectGitTime`，新增阻塞提供者回归。）
- [x] 发布前检查面板依赖边界，面板不反向依赖 runtime/dozer-app/client/core。（`python3 scripts/audit/check_panel_boundary.py` 通过。）

G1 退出条件：B 交付 C/D 共用的确定接口、可编译样例、库级测试和依赖需求。后续接口变化必须同时通知 C/D，不能让两个宿主各补一套底层实现。

## 7. Agent C：Dozer 的真实运行路径接入

覆盖原 T7，以及 T9–T12 的 Dozer 宿主部分。

- [x] 用进程内共享 GitRuntime 取代旧的每项目独立查询链；覆盖恢复、收养、激活、关闭、路径变化与回合结束。（非 Git 目录保留旧 watch 的树/预览刷新兜底。）
- [x] 令牌与分类 generation 在宿主应用结果前校验；同一快照投影到 Files/Project/Agent，清除重复 discover/status/分支二次读取。（Files 用 `SnapshotApplied`，Project 用纯数据入口；Agent 通过 Project 状态/override 投影。）
- [x] Git Log 使用唯一请求版本；普通文件保存不读 remote，git add 不重建图，其它分支 refs 变化仍能更新图。（`SnapshotLoaded` 带唯一 generation；Workdir 不触发、REFS/FullRescan 触发。）
- [x] 目录刷新结果异步合并，磁盘统计独立合并/节流；首页最近文件只取项目子树、排除 ignored/非普通文件。
- [ ] 接入 Usage summary/cache、Codehealth 必要复用及严格 patch/取消入口，保留既有语义。（Usage summary + 有界缓存、Codehealth 单次 HEAD 读取、file_history 严格 patch 已完成；CLI 取消回收未做。）
- [x] 错误显示保留 last-good/stale；关闭项目的迟到任务不能污染新项目；隐藏路径里的 tracked 改动不漏 dirty。（runtime token/epoch + 退订；树跳过隐藏路径但 Git status 仍失效。）
- [x] 清理不再使用的旧 watcher/组合快照函数，并迁移对应测试，避免两条链同时运行。（`start_git_watch`/`spawn_project_git_refresh`/`ProjectFsChanged` 已删；旧快照函数仅 `cfg(test)`。）

G2-C 退出条件：实际应用路径测试和手工操作证据确认新 runtime 被调用；仅在 Cargo.toml 增加依赖不算接入。（代码路径已接入，`cargo test -p dozer-app` 1133 passed；尚未做 GUI 手工录像/日志证据。）

## 8. Agent D：Digger 的真实运行路径接入

覆盖原 T8、T10/T12 的 Digger 宿主部分。

- [x] 接入 B 的同源接口和 runtime；保持 Digger 的 NoGitStats 与关闭的 Git 历史能力。
- [x] 完整保留变更原因/路径；同版本快照同时更新 Files 与 Project，修复 Project 长期保留旧分支/dirty/remote 的问题。
- [x] 处理项目切换、A→B→A、关闭重开、普通非 Git 目录与身份重接线；在入口拒绝旧 token。（普通非 Git 目录保留旧 watch 只刷树；身份 mismatch 触发重新订阅。）
- [x] 接入后台目录结果、按需刷新及对应写操作反馈，检查 Agent 不额外重复查询同工作区。（目录走 runtime watch + 异步 `TreeRefreshRequest`；分支切换等写操作仍走 Files 面板既有路径。）
- [x] 维持预览、编辑器、菜单和 Agent 操作能力，增加集成回归。（Digger lib 717 passed。）
- [x] 将共享依赖需求交给 E 收口，不以本地 Dozer/ByteUI patch 作为最终验证环境。（Digger 已切 `libs-2026-11-01b`、bytegit v0.9.1、byteui v0.5.1；无 patch 克隆 check/test 通过。）

G2-D 退出条件：Digger 真实产品路径已调用新实现，两个面板数据同步，相关集成回归通过；没有 remote 不影响本地集成测试，但会影响未来正式发布方式，须单独记录。（代码路径已接入，`cargo test -p digger-desktop --lib` 717 passed；尚无 GUI 手工证据，依赖仍为本地 patch。）

## 9. Agent E：纯远端依赖闭包和集成验收

先合入 A/B/C/D 的实际交付，再冻结版本。

- [x] 建立依赖顺序：bytegit/ByteUI 的可获取版本 → Dozer 共享 crates/libs 快照 → Dozer/Digger 产品依赖。（bytegit v0.9.1、byteui v0.5.1、bytehost v0.4.21、Dozer libs-2026-11-01b。）
- [x] 核对共享 tag 是否真实包含目标 commit；版本号一致不等于代码一致，禁止用旧 tag 配本地新代码过验收。（`cargo tree` 显示真实 commit；无 patch 克隆直接编译。）
- [x] 在独立 checkout 清除本地 patch，以远端 Git tag 重建锁文件；不修改主工作树配置。（Dozer/Digger 各建无 `.cargo/config.toml` 克隆并重新生成锁文件。）
- [x] Cargo.lock 的 bytegit、byteui、dozer-* 等共享来源全部为真实 git source；不存在本地路径、占位 tag 或同库多份版本。（两产品锁文件已替换为无 patch 生成版本。）
- [ ] 运行两个项目的依赖守卫、宿主/面板边界检查与现行格式、clippy、构建、测试门禁；完整结果对照 G0 基线，既有失败不能静默忽略。（无 patch 克隆已跑 `cargo check`、核心测试与两产品 `check-shared-deps.sh --offline`；完整 workspace/release 门禁尚未重跑。）
- [x] 以 release profile 构建两款 App 的各自 GUI/daemon/hook/MCP 四件套；不得复用旧版本二进制冒充本次构建。（无 patch 隔离克隆构建成功，大小/hash 见验收报告 §3.6。）
- [x] 打包资源与 plist/二进制版本一致性可在隔离目录验证；不调用会自动升版本的发布入口，不覆盖已安装应用。（两产品 `.app` 在临时克隆打包；版本/build、四件套、ad-hoc 签名核对通过。）
- [ ] 形成协调版本/提交清单、性能报告、已知限制和回退组合，不发布 App tag/Release。

若共享版本尚未发布、远端访问缺失或门禁失败，最终结论只能是“未就绪”，并列出剩余项。不能改成 path 依赖、关闭门禁或只提升产品版本来制造通过。

## 10. 最终必须覆盖的场景

| 场景 | 两产品验收要点 |
|---|---|
| 根项目 / 子目录项目 | 父仓库空提交、checkout 更新；项目外 dirty 更新全仓标记，Files 不显示项目外文件 |
| 多项目 / 多 worktree | 同工作区共享查询；不同 worktree 的 HEAD/index 不串用 |
| 执行中切换 | A→B→A、退订重开、关闭后的迟到结果不污染；全局和同工作区并发约束成立 |
| 大量文件事件 | quiet/max_age 有效，队列/路径/缓存有界，构建产物不造成树重读风暴，tracked 隐藏文件不漏 |
| Git 控制文件 | add、refs、config、shallow、exclude/attributes/alternates 与目录重建失效正确 |
| 历史 / 统计 | merge、reset、rebase、时间倒挂、shallow deepen 结果和缓存正确；Digger 不意外启动统计 |
| IO 与错误 | 目录 IO 不在 UI，Git IO 不持 DB 锁；失败/stale/恢复可解释 |
| 内容 / 子进程 | patch 硬字节限制与取消生效；Git hook/filter/冲突行为不变，取消实际回收进程 |
| 无本地 patch | 两款 App 的实际编译及现行门禁通过，所有共享源码能由所钉 tag 获取 |

验收同时满足原规格 A1–A9，不能只检查本表。平台事件不可用时显式记为未验证；当前 macOS 发布目标必须完成真实通知和真实工作区集成验证。

## 11. 每个 agent 的交付格式

1. 基于的仓库/HEAD、修改文件、交付 commit 或可应用 patch，明确是否已写回原仓库。
2. 实现的任务编号、数据接口、调用方迁移要求和仍未完成项。
3. 实际执行命令、环境、通过/失败/未验证结果，以及相对基线的变化。
4. 是否存在本地 patch、未发布共享版本、目录写权限或远端权限阻断。
5. 下一位执行者可以直接复现的验证步骤。不要交付无法访问的临时目录作为唯一证据。

E 的最终报告必须分别回答：

- Dozer 的实际运行路径是否完整包含本次重构？
- Digger 的实际运行路径是否完整包含本次重构？
- 两者是否能在无本地 patch 的 checkout 构建并通过现行门禁？
- 是否仍有阻止发布的代码、依赖、测试或外部条件？

上述问题都有可复核证据，且无剩余发布阻断，才标记“发布就绪”。App 版本号、安装及实际发布由用户另行安排。
