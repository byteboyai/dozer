# bytegit / Dozer / Digger 刷新链路优化规格

> 状态：待实施；本次交付为规格与计划，不代表功能已完成。
> 日期：2026-10-31。
> 依据：[代码分析](../../analysis/2026-10-31-bytegit-dozer-digger-optimization.md)。
> 实施入口：[实施计划](../plans/2026-10-31-bytegit-refresh-optimization.md)。
> 适用：bytegit `v0.8.1` 与分析文档所列的 Dozer/Digger 本地基线。本文是原 bytegit 迁移规格的增量规格；与旧文档冲突的刷新、生命周期和优化语义以本文为准。

## 1. 目标、边界与固定决策

本次解决工作区监听缺失、异步结果倒退、重复 Git 扫描及无界后台工作；保持两个产品现有的 Git 数据口径。

| 编号 | 固定决策 |
|---|---|
| D1 | bytegit 继续同步，不依赖 tokio、iced、产品模型或数据库，不公开 git2/notify 类型 |
| D2 | branch/dirty 属于整个工作区；Files 的路径与装饰只属于项目子树 |
| D3 | linked worktree 的 HEAD/index 独立；不能按 common_dir 合并工作区 |
| D4 | 一次组合刷新至多进行一次 libgit2 statuses 收集，dirty 在项目过滤和 UTF-8 过滤前计算 |
| D5 | 宿主统一调度并按数据类别合并任务；面板应用已计算的数据，不因应用数据再次发起相同查询 |
| D6 | 所有后台读取结果经身份、激活 epoch、数据类别 generation 校验后应用 |
| D7 | 旧 API 保留；通过新 API 表达新的监听事件和硬资源限制，避免给可直接构造的旧公开结构体追加必填字段 |
| D8 | 共享异步逻辑放在小型 `dozer-git-runtime` crate，仅依赖 bytegit、tokio；不加入 dozer-services 的面板服务 trait 层 |
| D9 | 文件树 HIDDEN 只直接控制树/预览事件投影；Git 状态失效单独判定，不以隐藏目录名直接丢弃已跟踪改动 |
| D10 | clone/checkout 保留系统 Git；提交图布局保留 gleisbau；Digger 继续使用 NoGitStats |

非目标：新增 commit/stage/push/pull/merge/worktree 管理；更换 libgit2；把托管平台账户或提交图放进 bytegit；全历史增量索引；大文件流式回滚；统一所有非 Git 文件操作的异步架构。现有历史子目录路径换算已完成，本次只增加回归验证。

## 2. 分层与职责

```text
bytegit
  工作区身份 / 单次状态扫描 / 引用与历史查询 / 监听分类 / CLI 写操作
       ↓
dozer-git-runtime
  订阅 / 有界合并 / generation / 查询调度 / 内存缓存 / 失败与降级状态
       ↓
Dozer、Digger 的宿主适配层
  项目生命周期 / Files 映射 / Project、Agent 投影 / UI 消息路由
       ↓
共享面板
  应用结果 / 展示 / 发出用户意图
```

`dozer-git-runtime` 首版不做 daemon、IPC、通用事件总线或独立持久化；产品在一个进程内共享一个实例。Digger 以同一 Dozer libs tag 引用它。Files/Project/Agent 面板不反向依赖 runtime，避免将宿主执行器带入面板。

## 3. 工作区身份与订阅

bytegit 新增 `Repo::identity() -> Result<RepoIdentity, GitError>`。以下为语义签名，公开结构体以构造器/访问器和 non_exhaustive 保留扩展空间：

```rust
RepoIdentity {
    workdir_root: Option<PathBuf>, // bare 为 None
    git_dir: PathBuf,              // 当前工作区私有 gitdir
    common_dir: PathBuf,           // 共享 refs/objects 所在目录
}
WorkspaceKey { workdir_root: PathBuf, git_dir: PathBuf }
```

- 用 libgit2 已解析的 gitdir/common_dir，不能只手工读取项目下 `.git` 文件。路径规范化后比较，保留项目输入路径用于 UI 文件键。
- 工作区查询拒绝 bare；历史只读 API 的 bare 兼容性不变。
- 子目录项目首先 discover_workdir，随后订阅整个工作区；同工作区的多个项目共享 watcher 和查询，各自持有 project_prefix。
- 未初始化 Git 的普通目录仍保留文件树监听；检测 `.git` 创建、删除或重新指向时重新 discover 并替换订阅身份。身份重建失败进入降级状态，不能继续把旧仓库结果当作新目录状态。
- 同一 common_dir 下不同 git_dir 分开调度状态；共享 refs 变动通知各工作区，不能把主工作区 HEAD/index 当作其它 worktree 的私有变化。
- 嵌套仓库/子模块不自动建立额外产品项目；其工作区文件变动可以使外层 status 失效，嵌套 `.git` 内部不被误认成外层 refs。

## 4. 单次查询与数据语义

### 4.1 状态快照

新增 `Repo::status_snapshot(StatusOptions) -> Result<StatusSnapshot, GitError>`：

```rust
StatusSnapshot {
    entries: Vec<StatusEntry>,
    dirty: bool,
}
```

要求：

1. 一次调用只执行一次后端 statuses 收集。
2. entries 与现有 status 相同选项下的输出语义一致；include_ignored=true 仍能给树提供 ignored 装饰。
3. 新快照 dirty 表示所请求范围内的真实改动，排除纯 ignored；包含冲突、非 UTF-8 路径及其它后端非空改动标志，不能从 entries 是否非空推导。
4. 组合刷新使用 include_untracked=true、include_ignored=true、detect_renames=false；dirty 是全仓库，Files 再投影到项目。
5. 旧 `status`、`is_dirty` 保留其参数语义。尤其不悄悄改变旧 `is_dirty(include_ignored=true)` 的行为；文档明确它与新快照 dirty 的差别。

Files 新增纯映射入口 `file_statuses_from_entries(project, identity, entries)`，执行规范化前缀过滤和路径重建。保留旧 file_statuses 为兼容包装。冲突的现有显示方式本次保持不变，但 dirty 必须正确。

### 4.2 组合查询

runtime 每个查询任务打开一次 Repo，在同一任务内按需读取 HEAD、status、branches、remotes。数据字段分别使用 Result；remotes 失败不应清空有效 status。快照携带 WorkspaceKey、各数据类别版本、采样时间、失败信息。

它不是 Git 原子事务。任务读取前后检查 HEAD 与可获取的 index/配置版本指纹；确认变化时保留失效标记并补跑。即使文件时间戳检测未捕获外部写入，监听后续失效、激活重验仍须保证最终收敛。

宿主把一个结果投影给 Files/Project/Agent。新增面板 `apply_*` 数据入口；legacy `StatusesRefreshed` 可保持原行为，但新路径不能再进入它触发分支二次读取。跨面板的版本接受由宿主统一决定。

## 5. 监听事件、忽略策略与资源边界

### 5.1 新 API

保留 `watch(root, WatchOptions, FnMut(GitChange))`。新增基于 RepoIdentity 的工作区监听入口与独立 WorkspaceWatchOptions/WorkspaceChange 类型，产品迁移到新入口。

事件至少表达：Workdir、Index、Head、Refs、Config、HistoryBoundary、FullRescan；同时携带有上限的逻辑变更路径、序号和降级信息。FullRescan 表示路径不足以做局部刷新，不能解释成“没有文件变化”。

分类范围：私有 gitdir 的 HEAD/index/config.worktree、common_dir 的 refs/packed-refs/config、shallow 等历史边界，以及 gitdir 重接线。不要把 index.lock/HEAD.lock 的临时存在误当成最终状态；最终替换和删除必须失效。refs 下临时锁允许保守失效。

全局 Git 配置、include/includeIf、外置 ignore/attributes 不承诺逐个建 watcher；激活、手动刷新和定时重验覆盖这些变化。相关版本进入缓存失效边界，不能宣称仅凭 HEAD 就永远有效。

### 5.2 路径与过滤

- 根路径建立已知别名映射；保留逻辑路径，不通过逐文件 canonicalize 把 symlink 文件事件变成仓库外目标路径。
- 删除路径按已知根映射归一化；重命名保留旧/新两侧；无法确定路径、后端 rescan 标志或通知错误触发 FullRescan。
- 树/预览使用共享 HIDDEN 过滤。Git 状态判断不能直接复用该名单。
- 隐藏/ignored 目录的事件在后台按缓存的已跟踪路径集合及 Git ignore 判定过滤；已跟踪文件永不因为 HIDDEN 名称被静默忽略。新路径、ignore 规则变化、集合失效或分类不确定时保守请求 status。
- Git ignore 判定也不得逐事件无限排队；在有界批次内后台处理，超限改为一次 status。构建产物噪声应合并，不能让 UI 同步读取 ignore/index。

### 5.3 有界聚合与停止

新 API 初始参数如下，均可配置，测试注入时钟：

| 参数 | 初始值 | 约束 |
|---|---:|---|
| quiet_delay | 300ms | 安静期 |
| max_batch_age | 1s | 从首个相关事件计时，后续事件不延长上限 |
| max_paths | 4096 | 超限清空明细并置 FullRescan，分类位不丢失 |
| max_path_bytes | 1MiB | 路径累计估算字节上限，与条数同时限制 |
| wake_capacity | 1 | 共用有界合并状态；唤醒信号合并，不按事件堆积消息 |

两端交换批次时必须在同一同步协议下清除唤醒状态，避免“清除信号与新写入交错导致永久不唤醒”。callback 不在后端通知线程执行；文档要求它仅转发。

生命周期明确为：Drop 请求停止并抑制尚未准入的回调，已准入回调可能完成；新增显式 shutdown/join 用于需要确认线程退出的调用方。从 worker 自身调用 join 返回明确错误，不自等待。修正旧文档不成立的绝对承诺，宿主必须校验 epoch；无需让 UI 等待 worker join。

启动时部分监听目标失败要返回/发出 degraded 信息；仍可用的监听继续工作。降级时活动项目每 30s 合并重验，正常状态每 60s 补偿校验；后台未激活项目仅监听并在激活时重验。不把定时重验变成全项目高频轮询。

## 6. 调度、结果版本与缓存

### 6.1 令牌与状态机

订阅使用 `SubscriptionToken { project_id, activation_epoch, workspace_key }`，请求另带按数据类别的 generation。epoch 来自进程级单调计数器，关闭重开或 A→B→A 都不能复用旧值；项目路径/身份改变同时替换 token。

每 WorkspaceKey、每查询类别维护 Idle / Running 与 pending invalidation。请求状态变化时只推进受影响类别版本，合并原因；Running 期间不再并发启动同类任务，完成后至多立即补跑一次，新的事件仍继续合并。全进程默认最多 4 个 Git 读取任务，后台优先级低于活动项目，等待集合按工作区合并而非按事件入队。

结果只在 token 仍存在且对应 generation 当前有效时应用。新 HEAD 事件不应使无关目录查询一律作废；不同类别独立校验。持续写入可能使结果长期保持 stale，本规格保证任务/内存有界以及写入停止后的最终收敛，不保证任意外部持续写入下的原子新鲜视图。

显式用户写操作不共用只读去重队列；其结果通知必须送达，完成后使相应只读数据失效。spawn_blocking 的已运行任务不能依赖 abort 停止；关闭订阅后取消排队任务，执行中结果丢弃。

### 6.2 失效矩阵

| 原因 | status | HEAD | branches/graph | remotes/config | 历史统计 | 树/预览 | 磁盘占用 |
|---|---|---|---|---|---|---|---|
| Workdir | 是 | 否 | 否 | 否 | 否 | 命中路径 | 独立节流 |
| Index | 是 | 否 | 否 | 否 | 否 | 必要装饰 | 否 |
| Head | 是 | 是 | 是 | 否 | 验证缓存键 | 若文件事件或 FullRescan | 否 |
| Refs | 保守验证 HEAD 后决定 | 是 | 是 | 否 | HEAD/边界变化时 | 通常否 | 否 |
| Config | 保守是 | 必要时 | 必要时 | 是 | 相关配置变化时 | 必要时 | 否 |
| HistoryBoundary | 必要时 | 是 | 是 | 否 | 是 | 否 | 否 |
| FullRescan/激活/手动 | 是 | 是 | 是 | 是 | 验证缓存键 | 全量缓存目录 | 激活/手动按策略 |

普通文件保存不重算 remotes/branches；纯 git add 不重建 gleisbau 图；其它分支 refs 变化可以重建图，即使当前 HEAD 未变。git_log 的重复请求也使用唯一 generation，不再仅比较 path/max_count。

### 6.3 统计与缓存边界

GitStatsProvider 新增有默认实现的 `summary`，返回总数和日统计；旧方法保留。默认实现可调用两次旧方法，DoGitStats 覆盖为一次 commit_count_by_day 后求和，NoGitStats 覆盖为零/空。新增接口的错误类型属于面板接缝，不暴露 bytegit；旧 provider 的默认实现无法恢复其已吞掉的错误，这个限制必须写进文档。

Git 统计与 usage_summary 并行启动，不改变其它用量加载的成功/失败语义。现有 Usage generation 继续有效。

runtime 中统计缓存键至少包括 WorkspaceKey、HEAD commit、历史边界指纹、查询选项与相关配置版本。日桶保留当前 committer time/UTC 整数除法语义。churn 加 since 的精确值/窗口标识和重命名参数；本次只做按需完整计算与缓存，不做增量累计。

所有缓存有容量及估算字节上限，默认 64 个统计项、总预算 64MiB，LRU 淘汰；超大结果直接不缓存。缓存命中前通过小型版本读取校验 HEAD/shallow；objects 缺失、仓库修复或无法校验时不复用。直接原始字段修改由补偿校验/显式刷新失效，不能承诺任意磁盘篡改下永久正确。

## 7. 文件树、磁盘、存储与首页

- 事件刷新只重读受影响父目录；缓存目录扫描在后台完成，UI 仅合并结果。重命名、目录删除和 FullRescan 有回退路径，保留展开态、选中态、搜索状态的已有产品语义。
- 磁盘占用独立：每项目至多一个任务，连续变更最短 10s 启动间隔；激活超过 30s 未采样则刷新，后台不因 refs/index 重扫。显示采样时间并允许旧值，明确是估算值。
- ProjectStore 先在连接锁内读写原始行并提交，再释放锁读取 Git 时间。保持 updated_ms=max(HEAD提交时间,last_active_ms)，Git 失败回落 last_active_ms；每次列表按工作区复用派生值。
- 首页最近文件固定为项目子树的非 ignored 普通文件。相同物理文件归属多个项目时保留各自项目关联，但同一项目内去重；不把忽略目录的 mtime 作为最近文件。

## 8. 错误与资源限制

后台数据使用 Ready / Empty / Stale / Unavailable 等效状态；无仓库、无提交是可解释空态，IO/backend/任务失败保留最后成功值并标记 stale。有新 token 时旧项目数据不能作为新项目的 last-good 显示。分支等可空字段不可再把查询失败悄悄映射成“干净”。用户错误提示按工作区/错误类别限频，详细错误写日志，URL 不含凭据。

新增严格 patch API（例如 `workdir_patch_limited`），旧 workdir_patch 语义保留：

- UTF-8 有效输出不超过 max_output_bytes；超长单行也服从上限；0 上限仅在存在可输出内容时标 truncated。
- 到达输出限制后主动终止回调，仅将已知主动终止转成成功截断；真实后端错误照常返回。
- 明确输入字节预算与取消检查。输入超限返回类型化的受限结果，由 UI 给占位；不能把回调提前退出宣传为限制了 libgit2 所有前置分配。
- 新读取 API 不改变 file_bytes_at 的原样回滚语义；本次不改符号链接/权限/原子替换的回滚产品行为。

CLI 新增可取消操作入口/独立选项，旧 clone/checkout 兼容包装：进度事件不得阻塞进程 IO 排空；stderr 保留最后 256KiB 并标记截断；stdout/stderr 持续排空。取消或调用方明确设置的超时需回收 Git 进程及其受控子进程；不能全局按名字杀 git。默认不设 clone 总时长上限。失败不删除预存目标目录，不输出凭据。平台进程组/job object 行为需集成测试，不能只测试取消标志。

## 9. 可观测性与验收

记录每阶段耗时、status 次数、合并事件数、in-flight/等待工作区数、缓存命中、过期结果丢弃、FullRescan、监听降级和目录重读数。耗时按 P50/P95 展示，调试日志不逐文件打印路径内容。

| ID | 验收条件 |
|---|---|
| A1 | 子目录项目外部空提交/checkout 可更新 HEAD/分支；项目外改动更新全仓 dirty，Files 不显示项目外路径 |
| A2 | 同项目乱序、A→B→A、关闭再打开均不应用旧结果；两 worktree 的状态互不串用 |
| A3 | 一次组合刷新 status 收集≤1；应用 Files 快照不触发重复 discover/分支读取 |
| A4 | 每工作区同类查询并发≤1，全进程读取≤配置值，事件/路径/缓存空间有界；持续事件按 max_batch_age 投递 |
| A5 | Index 不重建历史图；Workdir 不读取 remotes；config 更新 remote；ignored 构建产物不造成树重读风暴，隐藏路径内 tracked 改动不漏 dirty |
| A6 | Usage 一次全历史遍历产生 total/by_day；相同有效键命中缓存；merge/reset/shallow 变化不复用错误结果 |
| A7 | 树事件处理不在 UI 线程执行目录 IO；纯 refs 不触发磁盘遍历；Git 时间不在数据库锁内计算 |
| A8 | 严格 patch 满足字节上限；CLI 取消实际回收进程；原 hook/filter/冲突测试继续通过 |
| A9 | 错误不清空最后有效状态；降级明确可见；无本地 patch 的两产品 tag 依赖闭包验证通过 |

性能验收不预设“产品提速 50%”：分析中的 49.5% 是 debug 夹具局部结果。提交 release、冷/热缓存分组的真实仓库对照；以查询次数、并发边界和正确性作为硬门禁，P50/P95/RSS 为实测报告。无能力的 OS watcher 环境显式报告不可验证，不能静默通过集成断言。

## 10. 兼容、交付与回退

bytegit 新 API 以新增版本发布，建议下一可用 minor（实施时核对 tag，不预占版本）。旧 GitChange/WatchOptions 构造和旧方法编译兼容。新功能关闭时 bytegit 默认依赖图不引入 notify/tokio/iced。

Dozer 所有 bytegit 声明集中到 workspace；Digger 和共享面板使用同一 tag，验证没有多份 bytegit/git2。共享面板新数据入口、runtime 与宿主同一个 Dozer libs tag 交付。最终验证在隔离 checkout 中进行，不能改动用户主工作树的本地 patch 来制造发布结果。

回退单位是 bytegit tag + Dozer libs tag + Digger 对应适配提交的组合，不能只回退共享消息类型一端。缓存仅内存、无数据库迁移；失败可清空重建。先交付 bytegit API，再迁移共享库和两个产品；不得在 tag 尚不存在时提交消费方的占位版本。
