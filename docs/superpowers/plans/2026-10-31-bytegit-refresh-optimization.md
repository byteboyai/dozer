# bytegit / Dozer / Digger 刷新链路优化实施计划

> 状态：实施中。2026-10-31 已完成并发布 bytegit T1–T4（`v0.9.0`，§17），并完成 T5 共享 runtime（§18）；其余 checkbox 仍表示未来工作。
> 日期：2026-10-31。
> 规格：[设计规格](../specs/2026-10-31-bytegit-refresh-optimization-design.md)。
> 证据：[代码分析与实验](../../analysis/2026-10-31-bytegit-dozer-digger-optimization.md)。

## 1. 执行规则与阶段依赖

- 本计划不授权当前文档任务发布 tag、推送或修改三个仓库的生产代码；实际实施时按用户授权推进。
- 开始每个仓库的实现前读取适用 AGENTS.md，记录 HEAD、未提交改动、依赖与现有门禁。不得覆盖其它会话文件，不复用旧计划中的历史失败列表。
- 本机 bytegit、Digger 位于 Dozer 的同级目录；以下文件路径均相对对应仓库。所有修改与命令在实施时选定的隔离 checkout 中执行，不依赖此文档中的绝对个人路径。
- 不把本地 patch、临时 vendor/target、fixture 或凭据提交进仓库。只暂存明确文件；本计划不要求自动提交、自动发版或新建代理。
- 先完成行为测试再实现有并发、资源或语义风险的部分；不为纯文档和依赖集中这类低风险机械变更添加镜像测试。
- 每个任务提供：修改文件、行为对照、验证命令及结果、已知限制；完成对应门禁才勾选，不以“代码已写”代替验证。

```text
T0 基线 → T1 身份 + T2 状态 API
T1 → T3 监听 API → T4 有界监听
T1/T2/T3/T4 → T5 runtime → T6 面板入口 → T7 Dozer → T8 Digger
T5/T7 → T9 统计缓存
T6/T7/T8 → T10 树与磁盘
T0/T2 → T11 存储与首页
T0 → T12 patch/CLI
全部必需任务 → T13 集成、发布材料与回退验证
```

依赖图表达代码顺序，不要求代理并行。可按以下交付批次拆 PR：底层 API（T1–T4）、共享运行时（T5–T6）、双产品接入（T7–T8）、计算优化（T9–T11）、资源限制与最终验证（T12–T13）。各批次都应可独立审阅；消费方切 tag 必须晚于对应库版本实际可用。

## 2. T0：建立可复用的基线

- [ ] 记录三个仓库 HEAD、分支、工作区状态、Cargo.lock、`.cargo/config.toml` 的有效覆盖与工具链。
- [ ] 在隔离构建环境复现 bytegit 默认/all-features 测试；分析中的 168 passed 只作参考，不作为永远固定的测试数。
- [ ] 建立 Dozer 与 Digger 相关 crate 的测试基线、架构边界脚本基线，记录失败名称和原因。
- [ ] 准备仓库 fixture：根项目、子目录项目、同仓多项目、两个 linked worktree、普通非 Git 目录、ignored/hidden tracked 文件、非 UTF-8 与冲突。
- [ ] 建立计数钩子或可替换 QueryBackend、Clock，以便精确断言 status 次数、任务数、调度顺序；生产逻辑不依赖真实 sleep 验证顺序。
- [ ] 记录 release 性能基线：真实 Dozer/Digger、大量文件、大量提交；冷/热分开记录 P50/P95/RSS。

交付：基线记录放到后续 `docs/superpowers/analysis/bytegit-refresh-acceptance.md`，标注环境、命令和无法验证项；基准 fixture 与脚本放 bytegit `benches/` 或 `examples/` 的专用目录，不提交巨大仓库。

## 3. T1：公开工作区身份

**bytegit 文件：** `src/repo.rs`、`src/lib.rs`，必要时新增 `src/identity.rs`；测试使用 `src/testutil.rs`。

- [x] 实现 RepoIdentity 及访问器，使用后端解析 gitdir/common_dir；默认编译不增加依赖。
- [x] 固定路径规范化、bare、普通仓库、linked worktree、submodule、符号链接别名的契约。
- [x] 补测试：两个子目录获得相同 WorkspaceKey；同 common_dir 的两个 worktree 获得不同 WorkspaceKey；bare 工作区订阅明确失败。
- [x] 对照现有 discover/open_exact/discover_workdir 测试，保持原语义。

**门禁：** `cargo fmt --check`、`cargo test --locked`、`cargo test --locked --all-features`；公开签名无 git2/notify 类型。对应规格 A1/A2 的身份前提。（已通过，见 §17。）

## 4. T2：一次状态扫描与纯 Files 映射

**bytegit：** `src/status.rs`、`src/lib.rs`。
**Dozer：** `crates/dozer-panel-files/src/git_status.rs`、相关 tests。

- [x] 新增 status_snapshot，以一个原始 statuses 结果计算 entries 与 dirty；旧方法保留参数行为。
- [x] 编写 ignored-only、untracked、tracked、部分暂存、冲突、非 UTF-8、rename 选项测试；明确新旧 include_ignored dirty 差异。（macOS 文件系统拒绝非 UTF-8 文件名时该用例显式跳过；Linux 会真正执行。）
- [ ] 新增 file_statuses_from_entries 纯映射入口，旧 file_statuses 仅作 discover/query/映射包装。（Dozer 侧，未开始。）
- [ ] 验证子目录状态过滤和 UI 原始路径键，仓库外 dirty 不进入子项目装饰。（Dozer 侧，未开始。）
- [ ] 用计数后端证明组合 status 只收集一次；运行与分析相同夹具的 release 对照，但不把固定耗时作为测试断言。（计数断言已完成；release 对照未做。）

**门禁：** bytegit 默认/all-features 已通过；Dozer `cargo test --locked -p dozer-panel-files` 未执行；映射与查询分离，所有现有装饰口径测试通过。对应 A3。

## 5. T3：工作区监听分类与身份重建

**bytegit：** `src/watch.rs`、必要的新事件/选项模块、`src/lib.rs`。

- [x] 新增工作区监听 API 和独立事件类型；保留旧 watch/GitChange/WatchOptions 的源码兼容性，添加旧公开构造方式的编译测试。
- [x] 监听 workdir_root、私有 gitdir、common_dir 的必要范围，去重重叠目标；支持 Index/Head/Refs/Config/HistoryBoundary/FullRescan。
- [ ] 处理普通目录新增 Git、`.git` 删除/重指向、目标监听失败的 degraded 信息及重接线。（`.git` 删除/重指向已投递 FULL_RESCAN，部分目标失败已进 degraded；普通目录新增 Git 与内部自动重接线仍由调用方用旧 watch + 重新 discover 完成，已写入模块文档。）
- [x] 根别名归一化保留逻辑路径；测试删除、symlink、重命名两侧、路径为空、rescan/错误提示。
- [x] 纯分类测试覆盖父仓库 HEAD/index/config/shallow、主工作区与 linked worktree 私有状态隔离、nested `.git` 不冒充父 refs。
- [x] 集成测试验证子目录外部空提交及 checkout；有能力的环境必须等待并断言事件，不能零事件 return 成功。（先探测 FSEvents 能力；支持时必须等到并断言，不支持则显式跳过。）

**门禁：** 默认 feature 无 notify（已用 `cargo tree` 验证）；all-features 测试通过；macOS FSEvents 实测可用。对应 A1/A2/A5。

## 6. T4：有界监听聚合与生命周期

**bytegit：** 监听聚合器、选项、WatchHandle/新句柄实现和测试。

- [x] 实现 quiet_delay/max_batch_age 双期限；用可控时钟验证持续流按最大批次时长刷新。
- [x] 使用共享合并状态 + 容量 1 唤醒；实现 max_paths/max_path_bytes 与 FullRescan 折叠，所有原因位持续保留。（用 Mutex+Condvar 实现可合并唤醒，结构上不存在"清除信号与新写入交错"的丢失窗口。）
- [x] 测试唤醒清除竞态、队列拥塞、超长路径、单次 refs 夹在大量工作区事件中不丢失。
- [x] 实现停止与显式 shutdown/join；worker 内 join 返回错误，UI Drop 不等待可能阻塞的 callback。
- [x] 用 barrier 验证“已准入回调允许完成、未准入回调抑制”；修正文档，不能继续保留绝对 no-callback 承诺。
- [x] callback 慢、通知错误、部分目标失败均有观测结果；失败不演变为静默停更。

**门禁：** 纯状态机测试可重复运行、不依赖秒级 sleep（真实 FS 测试单独分组并探测平台能力）；压力测试断言待投递路径数不超过配置上限（测试用 32 条，生产默认 4096 条/1MiB）。对应 A4。

## 7. T5：共享小型 Git runtime

**Dozer 新 crate：** `crates/dozer-git-runtime/{Cargo.toml,src/lib.rs,src/identity.rs,src/scheduler.rs,src/query.rs,src/watch.rs}`，按实际规模合并文件。
**workspace：** 根 `Cargo.toml` 集中 bytegit tag 声明，逐 crate 用 workspace=true 并保留必要 features。

- [x] 实现 WorkspaceKey、SubscriptionToken、进程级 epoch 分配与各数据类别 generation，不依赖 UI/产品模型。
- [x] 实现每工作区每类 Idle/Running/pending；全局 semaphore 默认 4；等待集合按工作区合并，活动订阅优先。
- [x] 同一组合任务只 discover 一次、status 一次，按需求读元数据，返回字段级错误与采样信息。
- [x] HEAD/index 等版本变化后补跑；取消订阅清掉排队工作，执行中工作仅丢弃结果。
- [x] 桥接监听事件；输出端也要有界合并，不能把 bytegit 的有界队列转成无界 UI 快照积压。
- [x] 实现树隐藏与 Git 状态过滤的独立策略：已跟踪隐藏文件不漏，无法确认时保守扫描；分类工作在后台且同样有界。（运行时不持有 HIDDEN 名单，任何 Workdir 事件都失效 status；树投影由宿主过滤。）
- [x] 增加正常/降级补偿定时校验，仅活动项目定时重验；手动刷新绕过缓存但仍合并同类执行。（本版尚无统计缓存，手动刷新只保证合并/绕过未来的缓存接缝。）
- [x] 注入 QueryBackend、Clock 测试同项目乱序、A→B→A、关闭重开、身份重接线、同仓多项目及双 worktree。（身份变化目前检测为 `identity_mismatch` 并保留旧值，自动换 key 由宿主重新订阅完成。）
- [x] 增加阶段指标与 last-good/stale 状态；不记录含凭据 remote URL。

**门禁：** `cargo test --locked -p dozer-git-runtime` 通过（12 单测 + 13 集成 + 1 doctest）；fmt/clippy `-D warnings` 通过；`cargo tree -p dozer-git-runtime --depth 1` 只有 bytegit/tokio，完整依赖树无 iced/byteui/dozer-app/client/core。对应 A1–A5/A9。

## 8. T6：共享面板接收纯快照

**Dozer：** `crates/dozer-panel-files/src/{state,update,git_status}.rs`、`crates/dozer-panel-project/src/{state,update}.rs`、`crates/dozer-panel-agent/src/{card,tabs}.rs`，必要时 adapter 模块。

- [ ] 新增 Files/Project 应用纯数据入口，由宿主验 token 后调用；Files 应用状态不调用 spawn_git_info_load。
- [ ] 保留 legacy 消息路径；新路径显式传入分支栏信息，避免依靠隐式级联刷新。
- [ ] Agent 支持注入同一工作区快照；cwd 与项目工作区不同的 tab 独立订阅，不误用项目状态。
- [ ] 面板只消费业务所需投影；NoGitStats 和 Digger 无历史能力保持。
- [ ] 用计数假实现验证“应用结果零后台查询”，现有菜单、分支切换、装饰与卡片测试通过。

**门禁：** `cargo test --locked -p dozer-panel-files -p dozer-panel-project -p dozer-panel-agent`；`python3 scripts/audit/check_panel_boundary.py`（执行前读取脚本参数与现行门禁用法）；依赖边界无 runtime 倒灌。对应 A3。

## 9. T7：Dozer 接入与 Git Log 版本

**Dozer：** `crates/dozer-app/src/workspace/state.rs`、`app/{message,update}.rs`、`extensions/git_log.rs`、相关宿主适配模块；新增一个集中 git runtime 适配模块。

- [ ] Workspace 注册/恢复/收养/关闭/改路径使用 runtime 订阅，移除旧 watcher 与重复快照路径。
- [ ] 激活与回合结束转成失效请求；结果统一验证 token，随后分发 Files/Project/Agent 投影。
- [ ] 落实失效矩阵：Index 不重建图，Refs 更新分支/图，Workdir 不重读 remote。
- [ ] git_log pending 使用唯一 generation；测试同路径同 max_count 两次请求，旧结果先到不能清掉最新 pending。
- [ ] 替换 Codehealth 的重复 HEAD discover；保留 dirty_paths/churn 各自项目/仓库路径口径，禁止顺手改变分析语义。
- [ ] 删除/收拢不再使用的 unique_remote_urls、project_git_snapshot、spawn_project_git_refresh 实现，保留仍需的测试公共入口或迁移测试。

**门禁：** `cargo test --locked -p dozer-app` 与相关新调度测试；手工验证根/子目录/双项目/双 worktree、空提交、git add、checkout、remote config、错误 stale。逐事件计数确认 A1–A5/A9。

## 10. T8：Digger 接入并补齐 Project 刷新

**Digger：** `Cargo.toml`、`crates/desktop/Cargo.toml`、`src/adapters/{files,project,agent}.rs`、`src/product.rs` 及集成测试。

- [ ] 通过真实可用的 Dozer libs tag 引入 runtime 和面板新接口；本地联调 patch 仅在隔离目录使用。
- [ ] FilesFsChanged 不再丢弃事件原因/路径；用 shared runtime 取代私有 watcher 和独立 refresh_git。
- [ ] 同一版本结果同时刷新 Files 分支栏和 Project branch/dirty/remotes；产品路由明确拒绝旧项目 token。
- [ ] 树事件使用共享 HIDDEN，Git 过滤遵循 runtime tracked/ignore 策略，不直接照抄旧的整目录丢弃实现。
- [ ] 合并 Project git_snapshot 查询；确保 Agent 不产生同仓重复 status。
- [ ] 添加根/子目录、A→B→A、Project 已打开时外部修改、隐藏 tracked 文件的集成回归。
- [ ] 验证 NoGitStats、关闭 Git 历史能力、编辑器和预览能力均保持。

**门禁：** Digger `cargo test --locked -p digger-desktop`，现行产品/依赖边界检查及上述集成测试；无需调用外部网络仓库即可测试。对应 A1–A5/A9。

## 11. T9：统计合并与有界缓存

**Dozer：** `crates/dozer-panel-usage/src/{adapter,state,lib_tests}.rs`、`crates/dozer-app/src/host_adapters/usage_panel.rs`、`crates/dozer-git-runtime/src/` 的统计/缓存模块；Codehealth churn 适配。

- [ ] 新增 summary 默认方法及接缝自有错误类型；保留旧方法和第三方 provider 默认实现兼容。
- [ ] DoGitStats 一次日桶遍历后求和，NoGitStats 零遍历；统计与 usage_summary 并发启动。
- [ ] 缓存键加入 HEAD、WorkspaceKey、历史边界/相关配置版本、选项；churn 键包含 since/窗口与 rename 参数。
- [ ] 容量 64 项、64MiB 预算、超大结果不缓存、LRU 淘汰；读前小型版本校验，手动刷新跳过缓存。
- [ ] 验证 merge/reset/rebase/shallow deepen/时间倒挂，不用简单提交数增量替代完整可达计数。
- [ ] 测试一次请求一次遍历、相同有效键零历史遍历、键失效重算；Usage generation 与其它数据加载错误语义不退化。

**门禁：** Usage/runtime/Codehealth 相关测试；记录全历史扫描次数与热缓存性能。Digger 仍不启动统计任务。对应 A6/A9。

## 12. T10：后台目录刷新与磁盘节流

**Dozer 共享 Files：** `src/{fs,state,update}.rs`；两产品宿主事件适配与 Dozer 磁盘刷新路径。

- [ ] 将事件驱动的目录读取拆为后台读取结果与 UI 合并，利用 paths 只重读受影响父目录。
- [ ] 处理 rename 两侧、父目录删除、目录替换、FullRescan；使用独立目录 generation，保留现有展开/选择/搜索语义。
- [ ] 并发读取与消息载荷有界；目录结果版本落后时不覆盖新创建/删除产生的新状态。
- [ ] 磁盘任务独立合并，每项目一个 in-flight，10s 最短启动间隔、激活 30s 新鲜度策略；Refs/Index 不触发它。
- [ ] 补假慢文件系统/目录计数测试，断言事件处理不执行同步目录 IO；构建产物批量写入不重读全部树。

**门禁：** Files/runtime 与两个宿主相关测试；大量展开目录下的 UI 事件处理耗时及后台读取数报告。对应 A4/A5/A7。

## 13. T11：锁外 Git 时间与首页范围

**Dozer：** `crates/dozer-store-project/src/projects.rs`、`crates/dozer-app/src/chrome/homespace.rs`。

- [ ] 分离 RawProjectRow 与 Git 派生 ProjectInfo；list/open/rename 在锁内完成数据库步骤，释放锁后补 updated_ms。
- [ ] 同一次列表按工作区复用 HEAD 时间；维持 max(HEAD时间,last_active_ms)、单调活跃戳及既有 SQL 排序语义。
- [ ] 用可阻塞的时间提供者测试慢 Git 期间其它数据库访问能继续，不以易抖动毫秒阈值断言锁释放。
- [ ] 首页按项目子树投影、排除 ignored 与非普通文件；同项目去重，多项目保留各自关联。
- [ ] 加入子目录项目和 ignored 目录 mtime 回归，验证原“最近文件”普通场景。

**门禁：** `cargo test --locked -p dozer-store-project`、Dozer 首页相关测试；Digger 共享存储集成验证。对应 A7。

## 14. T12：严格 patch 与 Git 子进程控制

**bytegit：** `src/{history,write,error,lib}.rs`、必要进程控制模块；Dozer `file_history`/`project_create` 与 Files 分支切换适配。

- [ ] 新增严格 patch API；UTF-8 边界、零预算、超长单行、输入预算、主动终止和真实错误区分均有测试。
- [ ] 保留旧 patch API；产品迁入新入口，受限结果显示可解释占位，不把所有限制变成 Backend 错误。
- [ ] 新增独立进程操作选项/取消令牌与有界进度回调，旧同步入口兼容；stderr tail 256KiB、持续排空输出。
- [ ] 平台实现回收受控子进程，默认无总超时；测试取消真实进程树、超时、失败清理边界及输出截断。
- [ ] 继续运行现有 clone/checkout hook、外部过滤器、脏工作区冲突、参数注入防护测试。
- [ ] 不修改 file_bytes_at/回滚的文件模式和 symlink 行为；流式回滚另行设计。

**门禁：** bytegit 默认/all-features、Dozer 调用方相关测试；macOS/Linux/Windows 可用环境的进程回收验证。对应 A8。

## 15. T13：跨仓集成、依赖闭包与交付材料

- [ ] 更新 bytegit README/API 文档与 CI 平台矩阵；Windows/Linux 未执行结果明确列为未验证，不能写通过。
- [ ] 在所有变更后的库上执行 fmt、clippy 与默认/all-features 测试；两个产品执行相关 crate 及 workspace 必需门禁，对比 T0 基线。
- [ ] 在隔离 checkout 去掉本地 patch，使用实际发布 tag 重新解析锁文件；检查 cargo tree/metadata 的 bytegit/git2 来源和版本。
- [ ] 验证新 runtime 随 Dozer libs tag 可用，Digger 所有共享 Dozer 依赖对齐同一 libs tag；不保留本地路径或占位 tag。
- [ ] 完成 A1–A9 验收矩阵，附命令、环境、结果、失败解释、手工录像/日志位置；没有执行的项写未验证。
- [ ] 复测 release 冷/热性能矩阵：P50/P95、RSS、status/discover 次数、in-flight、队列与缓存峰值、目录重读数；不引用 debug 夹具作为产品提速结论。
- [ ] 准备协调版本清单、发布顺序及回退提交/tag 组合；在本地模拟回退，确认不需要数据库迁移或缓存清理脚本。
- [ ] 更新旧 remaining 文档中已过时的 Digger 版本和 D14 历史类描述，链接新规格，保留其历史记录性质。

### 建议验证命令

以下在各自隔离仓库根运行，工具链/平台开关按 T0 的现行配置补全；这只是计划，尚未执行：

```sh
# bytegit
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --all-features

# Dozer（runtime 创建后）
cargo test --locked -p dozer-git-runtime -p dozer-panel-files -p dozer-panel-project -p dozer-panel-agent -p dozer-panel-usage -p dozer-store-project
cargo test --locked -p dozer-app
cargo tree --locked -i bytegit
cargo tree --locked -i git2

# Digger
cargo test --locked -p digger-desktop
cargo test --locked --workspace
cargo tree --locked -i bytegit
cargo tree --locked -i git2
```

若新依赖需修改锁文件，先在允许解析的本地联调阶段更新并检查差异，再执行 --locked 门禁。不可通过删除锁文件、批量升级依赖或隐藏错误来使门禁通过。所有必须门禁通过后，只有新增代码、失败或未解决疑点才扩大/重复测试。

## 16. 完成定义与交付记录

| 交付 | 完成条件 |
|---|---|
| 底层 | T1–T4 通过，旧 API 编译兼容、新 API 边界与生命周期明确 |
| 产品正确性 | T5–T8 通过，根/子目录/多项目/worktree/迟到结果收敛正确 |
| 计算优化 | T9–T11 通过，单次状态扫描、统计单遍历、UI/DB 锁外 IO 有证据 |
| 资源与发布 | T12–T13 通过，限制/取消有效，无本地 patch 的版本组合可复现 |

预估总量约 15–25 人日，包含共享 runtime、三平台资源控制和集成验证；真实历史算法重写、流式回滚和额外产品功能不计入。先完成 T0 后按基线修正估算，不以时间到期宣告完成。

最终记录必须区分“实现完成”“测试通过”“版本已发布”“消费方已切换”。本文的 checkbox 不承担自动发布授权；没有用户要求的发布动作不因计划写明发布顺序而自动执行。

## 17. 实施记录（2026-10-31，bytegit T1–T4）

本次范围仅为 bytegit 底层 T1–T4；Dozer/Digger 接入与发布均未发生。

**基线（改动前）**

- 仓库：`bytegit`，HEAD `ae60c83` = tag `v0.8.1`；工作区只有未跟踪的 `.dozer/`。
- 环境：macOS aarch64，`rustc 1.98.1`、`cargo 1.98.1`。
- `cargo test --locked`：139 passed。分析文档里的 168 是另一时点/特性组合的参考值，未作为固定目标。

**改动**

| 文件 | 内容 |
|---|---|
| `src/identity.rs`（新） | `RepoIdentity`、`WorkspaceKey`、`Repo::identity`；按 libgit2 解析的 workdir/gitdir/commondir canonicalize，bare 的 `workspace_key()` 返回 `NotARepo`。 |
| `src/status.rs` | `StatusSnapshot`、`Repo::status_snapshot`；唯一 `collect_statuses` 入口；snapshot `dirty` 排除纯 ignored，旧 `status`/`is_dirty` 参数语义不变。 |
| `src/workspace_watch.rs`（新） | `watch_workspace`、`WorkspaceChange`/`WorkspaceChangeKind`、`WorkspaceWatchOptions`、`WorkspaceWatchHandle`、`WatchDegraded`；workdir/私有 gitdir/common dir 分类，quiet_delay + max_batch_age，路径条数/字节上限，FullRescan 折叠，Drop 抑制未准入回调，显式 `shutdown`。 |
| `src/repo.rs` | 测试计数钩子（`status_collections`），生产 API 不变。 |
| `src/lib.rs` | 导出上述新类型；默认 feature 仍只依赖 git2/serde。 |
| `src/watch.rs` | 仅新增旧公开构造方式的兼容测试，行为未改。 |

**验证（bytegit 根目录）**

```text
cargo fmt --check                                     通过
cargo clippy --locked --all-targets --all-features -- -D warnings   通过
cargo test --locked                                   157 passed
cargo test --locked --all-features                    219 passed
cargo tree --locked -e normal | rg 'notify|tokio|iced'  无匹配(默认 feature)
```

真实文件系统用例先探测 FSEvents：支持时必须等到并断言事件，不支持则显式跳过。本次 macOS 上全部真实 FS 用例执行通过。

**发布记录**

- 版本：`Cargo.toml` `0.8.1` → `0.9.0`（新增 API，向后兼容的 minor）。
- 提交：`9d47571 feat: add workspace identity, status snapshots and bounded workspace watching`；`4a4951b chore: release 0.9.0`。
- tag：`v0.9.0` → `4a4951b`，已推送 `origin`；`origin/main` 同步到 `4a4951b`。
- 发布前在版本号变更后重跑：fmt、默认/all-features clippy 与测试，均通过（157 / 219 passed）。

**已知限制 / 未完成**

- 普通非 Git 目录新增 `.git`：新 API 需要 `RepoIdentity`，调用方仍用旧 `watch`；`.git` 删除/重指向只投递 `FULL_RESCAN`，重新 discover 与重建监听由调用方完成（见 `workspace_watch` 模块文档）。
- 非 UTF-8 路径计数测试在 macOS APFS 上无法创建夹具（EILSEQ），显式跳过；Linux 等平台会真正执行。
- 启动时若 `common_dir/refs` 尚不存在，只监听 common dir 本层；之后的 `refs` 目录创建会上报 REFS，但不会自动补递归监听。正常初始化/克隆的仓库都有 `refs/`。
- 未做与分析文档相同夹具的 release 耗时对照；未跑 Linux/Windows 门禁。
- 已升版本并发 tag，但尚未修改 Dozer/Digger 的 bytegit 依赖（留给 T5/T7/T8）。

## 18. 实施记录（2026-10-31，T5 共享 runtime）

**新增 crate：** `crates/dozer-git-runtime`，依赖只有 `bytegit`（watch feature）与 `tokio`。

| 文件 | 内容 |
|---|---|
| `src/lib.rs` | `GitRuntime`/`EventStream`/`RuntimeConfig`/`RuntimeEvent`/`WatchedPaths`；按 `project_id` 合并的有界事件队列；指标快照。 |
| `src/identity.rs` | `SubscriptionToken`、`Generations`、`Categories`、`RefreshReason`；监听原因位 → 失效类别映射。 |
| `src/query.rs` | `QueryBackend` 接缝、`BytegitBackend`、字段级 `Field<T>`、`WorkspaceSnapshot`、last-good/stale 合并。 |
| `src/scheduler.rs` | 单任务调度循环:每工作区单飞行任务、全局 semaphore、pending 合并、generation 校验、补偿校验、监听/查询结果回传。 |
| `src/watch.rs` | 监听线程 → 调度器的有界合并入口(原因位 + 路径上限 + FullRescan 折叠)。 |

**workspace 依赖集中：**

- 根 `Cargo.toml` 新增 `bytegit = { ..., tag = "v0.9.0" }`。
- `dozer-app`/`dozer-panel-files`/`dozer-panel-project`/`dozer-panel-agent`/`dozer-store-project` 全部改为 `workspace = true`，需要 watch/testutil 的保留 features。
- 本地仍通过未提交的 `.cargo/config.toml` patch 指向 `../bytegit`；crate 实际检查的是 `v0.9.0`。

**行为要点：**

- 订阅时解析一次 `RepoIdentity`；查询任务内再 discover 一次并只读请求类别。`Workdir/Index` 只失效 status，`Refs` 先验证 HEAD/branches，HEAD 真变才补 status。
- 飞行期间的新失效只推进受影响 generation；返回时按类别丢弃过期字段并补跑一次。退订会删掉排队工作、事件与工作区；执行中结果回来找不到工作区即丢弃。
- 同仓库多个项目共享 watcher/快照/查询；新项目优先复用已有快照，不重复拉取。
- 全进程默认 4 个读取任务；等待集合按工作区合并，活动订阅优先拿名额。输出按项目合并为"最新快照 + 合并后的监听路径"，不会按事件堆积。
- 正常 60s / 降级 30s 补偿校验仅针对活动项目；监听失败或快照 stale 进入降级。
- 运行时不应用树 HIDDEN 名单；任何 Workdir 事件都保守失效 Git status。

**验证：**

```text
cargo test -p dozer-git-runtime              12 单测 + 13 集成 + 1 doctest 通过
cargo fmt -p dozer-git-runtime --check       通过
cargo clippy -p dozer-git-runtime --all-targets -- -D warnings   通过
cargo tree -p dozer-git-runtime --depth 1    仅 bytegit / tokio
```

覆盖:一次查询单次 discover/status、同仓多项目共享、全局并发上限、飞行结果作废补跑、A→B→A、退订重开、字段失败保留 last-good、身份 mismatch、整次失败、双 worktree、手动刷新合并、事件队列/路径上限。

**已知限制 / 下一步：**

- 每工作区当前只有一个飞行任务（比"每类各一个"更保守）；全局并发与合并语义不受影响。
- 身份变化只检测为 `identity_mismatch`，不自动换 key；T7 宿主应据事件重新订阅。
- 补偿校验的 ticker 用真实 500ms 唤醒、注入时钟判断期限；测试通过 `validate_now()` 精确推进。
- 尚未接入 Dozer/Digger 宿主与面板；T6 起做纯快照入口，T7 做项目生命周期与投影。

## 19. 审核缺陷修复（2026-10-31）

对应[审核报告](../../analysis/2026-10-31-bytegit-implementation-review.md) R1–R4，详情及 bytegit 应用步骤见[修复交付说明](../../patches/2026-10-31-bytegit-audit/README.md)。

- R1：Dozer 使用运行时全局查询 ID，查询完成消息不能与重建工作区的首个查询混淆；watch 启动结果及回调使用不复用的订阅 epoch 隔离。
- R2：running 注册表独立于订阅存活，semaphore permit 由真实执行任务持有；退订重开不绕过全局或同工作区并发限制。
- R3：bytegit 隔离副本在 Access/路径过滤前检查 rescan 标志。
- R4：bytegit 隔离副本补共享控制目录监听与缺失/重建后的恢复、补扫和降级信息。
- 验证：bytegit 默认 157 / all-features 223；runtime 14 单测 + 15 集成 + 1 doctest；格式及两库 all-features clippy `-D warnings` 通过。
- 落地状态：Dozer 修复已在工作树；bytegit 原目录在当前会话只读，修复以 `bytegit-fixes.patch` 交付，未写回原目录、未发布版本；T6–T13 仍未实施。
