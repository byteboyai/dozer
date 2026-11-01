# bytegit 及其在 Dozer、Digger 中的应用：代码分析与优化方案

日期：2026-10-31。范围：三个本地工作树的生产 Rust 代码、依赖声明、锁文件与相关测试。本文只提出方案，没有修改生产代码。

实施文档：[设计规格](../superpowers/specs/2026-10-31-bytegit-refresh-optimization-design.md)与[实施计划](../superpowers/plans/2026-10-31-bytegit-refresh-optimization.md)。接口、兼容和验收决策以设计规格为准。

## 1. 结论与证据范围

bytegit 的底层边界合理，应该继续保留同步 API、独立领域类型和必要的系统 Git 调用。最值得优化的是 **监听 → 调度 → 查询 → 面板应用结果** 这条链路，其次才是历史查询算法。

建议顺序：

1. 修复监听缺口与旧结果覆盖，保证状态能够更新且不会倒退。
2. 合并重复 status，按变化类型刷新，给后台任务加并发边界。
3. 合并、缓存历史统计，再用真实大仓库数据决定是否优化 revwalk。
4. 改进 patch 的资源限制、错误可观测性和数据库锁内 Git IO。

分析基线：

| 仓库 | HEAD | 依赖现状 |
|---|---|---|
| bytegit | `ae60c83` | `0.8.1`，git2 `0.21`，可选 notify `8` |
| Dozer | `f5bd3ac5` | 多个 crate 各自声明 bytegit `v0.8.1`；本地配置 patch 到 `../bytegit` |
| Digger | `2b21b0f` | workspace 声明 bytegit `v0.8.1`；共享面板本地 patch 到当前 Dozer 工作树 |

两个消费方锁文件的 bytegit 来源均指向 `v0.8.1#ae60c831…`。本地 patch 配置和锁文件不是一回事；本报告的源码结论针对当前本地文件，发布前还需验证去掉本地 patch 后的 tag 依赖闭包。

特别纠正旧文档：`docs/superpowers/unfinished/2026-10-16-bytegit-remaining.md` 里“Digger 仍用 0.8.0”“历史类仍只认仓库根”已不符合当前代码。`diff_content::ProjectRepo` 已换算项目路径，`file_history` 的读取、patch、回滚、上一版本均已使用它。不要重复安排 D14 迁移。

## 2. bytegit 的架构与成本模型

| 模块 | 职责 | 当前实现与成本 |
|---|---|---|
| `repo.rs` | discover、open_exact、discover_workdir | 显式区分向上发现、精确根目录、非 bare 工作区；Repo 为 Send、非 Sync |
| `info.rs` | HEAD、本地分支、remote | 读取引用和配置；不同数据有不同失效条件 |
| `status.rs` | 文件状态、dirty | `status` 和 `is_dirty` 都调用 libgit2 `statuses`；后者没有提前终止扫描 |
| `history.rs` | log、提交文件、上一版本、patch | log 使用 TIME revwalk；按路径逐提交 diff；patch 限制输出但没有提前停止生成 |
| `content.rs` | blob、历史文件、磁盘对比 | 内容分类统一；磁盘读取限制为 max+1；blob 限制在 find_blob 后检查 |
| `stats.rs` | 总提交数、日统计、churn | 两种计数独立遍历；churn 对近期提交逐个 diff 并做重命名检测 |
| `watch.rs` | 文件事件分类与 debounce | notify + 标准线程；无界 mpsc；每路径 canonicalize；安静期后投递 |
| `write.rs` | init、clone、checkout | init 走 libgit2；clone/checkout 走系统 Git，保留凭据、hook、过滤器行为 |

已有的优点：公开 API 不泄漏 git2 类型；CommitId/BlobId 隔离后端；路径字面匹配已有修复和测试；内容限制、空仓库、detached HEAD、worktree、hook、过滤器等有较多语义测试。git2 0.21 的本地依赖源码确认默认 features 为空，不能把未启用 HTTPS/SSH 理解成配置遗漏。

需精确理解的成本：

- `is_dirty` 省掉的是领域条目转换，当前并没有省掉 libgit2 的全量状态收集。
- TIME revwalk 即使只取少量 log，也可能先承担可达历史的解析排序成本；`max_count` 不是 CPU 工作量上限。
- `churn` 的时间排序是为了包含时间戳异常的祖先提交，不能直接改成“遇到旧时间就停止的任意遍历”。
- `ContentLimits` 不等于所有后端内存分配都被限制：`find_blob` 后才检查 blob 大小，不能据此保证超大 blob 从未解压或加载。

## 3. 在两个产品中的实际应用

### 3.1 Dozer

主链路：`Workspace::start_git_watch` → `ProjectFsChanged` → `project_fs_changed` → `spawn_project_git_refresh` → Files/Project 两条结果消息。

一次 `project_git_snapshot` 会：

1. `discover_workdir`，读取 HEAD，再 `is_dirty(default)` 扫描一次。
2. `file_statuses(project)` 内部重新 discover，使用 include_ignored=true 再扫描一次，并过滤到项目子树。
3. `unique_remote_urls` 再 discover 一次。
4. Files 收到 `StatusesRefreshed` 后又启动 `spawn_git_info_load`，重新 discover、读取 HEAD 和本地分支。

因此，一次刷新本身就包含 **2 次 status 和至少 4 次仓库发现**；不是单纯的“几个函数重复命名”。期间工作区可能继续变化，多个面板还可能观察到不同瞬间的数据。

另外，`project_fs_changed` 对每批事件都会启动磁盘占用刷新；引用变化也会同步重读缓存目录。保存一个已有文件、git add、分支切换被绑定到了近似相同的刷新路径。`index` 被归入 refs_changed，因此 git add 也可能触发 Git Log 重建。

其他调用：

| 场景 | 使用点 | 优化含义 |
|---|---|---|
| Usage | `host_adapters/usage_panel.rs`、`dozer-panel-usage/src/state.rs` | 总数与日统计分别遍历，且在 usage_summary await 后才开始 Git 统计 |
| Agent 卡片 | `dozer-panel-agent/src/card.rs` | cwd 所属仓库的分支/dirty；同仓库多 tab 可共享快照 |
| 历史、diff、回滚 | `extensions/{git_log,file_history,diff_content}.rs` | 内容读取使用 bytegit；提交图布局仍由 gleisbau 提供 |
| Codehealth | `extensions/codehealth/git_hotspots.rs` | HEAD/dirty/churn；git_snapshot 内又单独 discover 获取短 SHA |
| 首页最近文件 | `chrome/homespace.rs::load_home_recents` | discover 后又调用 file_statuses；使用仓库根而非项目子树，且没有先排除 ignored 条目 |
| 项目排序 | `dozer-store-project/src/projects.rs` | row_to_project 内计算 Git 时间，发生在数据库连接锁持有期间 |
| 创建、初始化、切分支 | project_create、scaffold、Files/Git Log | 保留现有写操作语义，优化重点是生命周期与结果反馈 |

### 3.2 Digger

Digger 通过共享 Dozer crate 复用 Files、Project、Agent、项目存储等能力，也在 desktop 适配层直接使用 bytegit。它并不是只通过 Dozer 间接依赖。

- Files：`adapters/files.rs` 自己持有 watcher；变更后刷新树及 file_statuses，随后共享 Files 更新器再读取分支信息。
- Project：`adapters/project.rs::git_snapshot` 自己读取 branch/dirty/remotes；同一次快照两次 discover。
- Agent：共享 `dozer-panel-agent` 的 cwd Git 信息。
- 项目排序：共享 `dozer-store-project`。
- Usage：明确注入 `NoGitStats`，当前没有 Git 统计需求。
- Files 的 Git 历史能力未启用，不应把 Dozer 的提交图或历史重型能力强加给 Digger。

关键差异：Digger 的 `FsChange` 只保留 project_id 和 workdir_changed，丢弃 refs_changed 与 paths。`product.rs::FilesFsChanged` 只转交 Files，不触发 Project 面板 Git 刷新。因此 Files 分支栏可能更新，Project 分支/dirty/remote 仍保持旧快照，直到另一次激活或显式刷新。

## 4. 已确认的问题与优先级

### P1：监听覆盖与刷新一致性

**A. 子目录项目查询与监听范围不一致。**

两产品都把 project.path 直接传给 watch。bytegit 只看 `root/.git`，没有先 discover 父仓库；项目为 `/repo/sub` 时，真实 `/repo/.git/HEAD` 不在监听树内，ExternalGit 也无法解析。外部 checkout 或空提交可以不改动子目录文件，界面不会必然收到刷新信号。

修复：解析工作区身份，分别保存 project_root、workdir_root、git_dir、common_dir。项目文件树按项目订阅；仓库 branch/dirty 按整个工作区订阅。当前 dirty 的产品语义是整个仓库，所以仅补监听父 `.git` 还不够：项目外文件变动也应使仓库 dirty 失效。多个项目指向同一工作区时共享底层监听并分别投影文件事件。linked worktree 的 index/HEAD 是私有的，不能只按 common_dir 合并。

**B. Digger 的忽略名单与注释不一致。**

`start_watch` 注释说与 Dozer 相同，实际只调用 `WatchOptions::new(300ms)`，默认规则为空。target/node_modules/__pycache__ 的事件都可进入刷新链。

修复：首批沿用共享 `dozer_panel_files::fs::HIDDEN` 配置产品策略；之后明确区分“树上隐藏”与“不会影响 Git 状态”。HIDDEN 不应无条件作为 Git 正确性的依据：被隐藏目录也可能包含已跟踪文件。若采用忽略策略，应保留激活时重验或定期校验的补偿路径。

**C. 同一项目旧任务可覆盖新任务。**

`StatusesRefreshed`、`GitRefreshed`、`GitInfoLoaded` 缺少刷新版本。project_id 路由只能解决部分跨项目问题，不能解决同项目 A 慢、B 快、最后 A 覆盖 B。Git Log 的 pending=(path,max_count) 同样区分不了重复请求；先到的旧结果可能被接受，并清掉 pending，使新结果被丢弃。

修复：使用 `(project_id, activation_epoch, refresh_generation)` 或等效令牌；仓库快照可有自己的 generation。结果应用前校验身份与版本，关闭项目或重新激活递增 epoch。参考 Usage 和 file_history diff 已存在的 generation 模式。Digger 的 Files 迟到结果过滤也应在宿主入口明确执行，共享更新器本身不验证当前项目。

**D. Digger 没有把仓库变更广播到 Project。**

修复：FsChange 保留完整分类和必要路径；由宿主协调器将同一快照投递给 Files 与 Project。不要在两个面板各自产生一条重复的全量查询链。

### P1：调度缺少边界

**E. debounce 可以无限等待，队列和路径集合没有上限。**

`debounce_loop` 每收到一条都重新等待整个安静期，持续事件下没有最大投递延迟；mpsc 与 BTreeSet 都无界。慢 callback 又会阻塞后续批处理。

修复：同时设置 quiet_delay 和 max_batch_age，例如先用 300ms/1s 再实测调整；路径超过阈值后折叠为 full_rescan 标记，保留失效分类。用有界合并状态加唤醒信号，避免满队列时静默丢失唯一的一次引用变化。notify 后端错误、rescan 提示、空路径事件需要触发保守重验或报告降级。

`WatchHandle::Drop` 目前只设 AtomicBool，没有等待 worker；worker 在检查标志之后、调用 callback 之前存在竞态，因此文档“Drop 后不会再有回调”并非严格保证。应明确生命周期契约，使用停止信号与可回收线程；若承诺同步停止，还要处理 callback 内停止自身时的死锁。宿主 epoch 校验仍然必要。

**F. 每批事件都启动新任务，没有合并正在运行的刷新。**

修复：每工作区每类查询最多一个 in-flight；运行期间的新变化只更新 generation 和 pending 标志，完成后最多补跑一次。已开始的 spawn_blocking 不能靠 abort 保证停止；丢弃过期结果、合并后续请求才是基础机制。

### P2：重复计算与失效范围过大

**G. 状态查询合并。**

在 bytegit 增加 `status_snapshot(options)`，一次 libgit2 statuses 遍历同时输出 entries 和 dirty。dirty 必须在跳过非 UTF-8 路径之前，根据原始状态计算，并排除纯 ignored 条目、保留冲突。不能直接用现有 `status(include_ignored=true)` 的 Vec 是否非空，也不能从仅项目子树的映射推导整个仓库 dirty。

Files 增加 `file_statuses_from_entries(project, repo_root, entries)` 纯映射函数，保留旧包装函数兼容调用方。一次打开的 Repo 供 HEAD/status/remotes/branches 共用。快照只能提供“同一刷新批次”的一致性，不是 Git 事务快照；读取前后验证 HEAD/index 等版本并按需要重试，不能宣称对外部写入原子。

**H. 按变更原因失效。**

当前 refs_changed 把 HEAD、index、refs、packed-refs 混在一起，且遗漏 `.git/config`。remote 配置修改不一定触发事件，纯 index 更新又可能重建提交图。

| 变化 | 刷新数据 | 通常无需刷新 |
|---|---|---|
| 工作区文件 | status、相关目录、命中路径的预览 | remote、本地分支、历史图 |
| index | status | 历史总数、历史图 |
| HEAD | HEAD/status；提交变化时刷新历史 | 无关 remote |
| refs/packed-refs | 分支与图；按当前 HEAD 是否改变判断统计 | 无关磁盘占用 |
| config | remote/配置；相关配置变动时保守刷新 status | 仅凭 config 事件就全量重建历史 |
| ignore/attributes/全局配置 | 保守重验其影响的数据 | 不能只当普通单文件内容修改 |

GitLog 实际由 gleisbau 布局；修复 bytegit::log 的耗时不自动改善提交图性能。图缓存和失效要在对应调用链测量。

**I. 树与磁盘占用刷新过重。**

两产品的事件处理会同步调用 reload_tree_from_disk，它逐个重读全部缓存目录。在文件多或慢文件系统上可能阻塞 UI。先利用 paths 只刷新受影响父目录；再把目录读取移出 UI 线程，携带 generation 合并结果，保留展开/选择态。重命名同时处理新旧父目录；事件不完整时回退全量。

Dozer 的磁盘占用改成独立、低频且合并的任务；引用事件不触发磁盘遍历。忽略目录也可能增长，因此磁盘用量不能仅依赖已过滤的 Git/文件树事件保持精确；按产品允许的新鲜度在激活或定时重算。

### P2：历史统计与内容处理

**J. Usage 两次全历史遍历。**

一次 commit_count_by_day 的各桶之和就是同口径的提交总数。优先给 GitStatsProvider 增加 `summary` 返回 total/by_day，提供默认实现兼容现有 provider；DoGitStats 覆盖为单次扫描，NoGitStats 返回中性值。按 `(工作区身份, HEAD commit, 统计选项)` 缓存，切分支或 force reset 后重新验证。shallow 边界变化也会改变可达历史，即使 HEAD 不变，仍需失效。

Git 与 usage_summary 独立，可并行启动；不要用纯 Git 错误阻止其它用量结果展示。

churn 单独缓存，键加入 since/时间窗口、路径与重命名策略。仅新 HEAD 相对旧 HEAD 可证明线性快进时考虑增量；merge、rewrite、浅克隆变化先全量回算。复用按 CommitId 的不可变父信息/时间/文件 delta 比直接累加总数更稳妥，可避免合并分支重复计数。

**K. patch 限制没有限制后续计算。**

workdir_patch 达到 max_bytes 后仍向 diff.print 回调返回 true，后续 diff 行继续遍历；一行也可能任意长，“最多超出最后一行”不是严格内存上限。

修复分两步：先截断到 UTF-8 边界上的硬字节限制，再通过后端支持的终止信号提前退出，只把本次主动终止映射为成功且 truncated=true，保留真实后端错误。另加单文件输入体积阈值和取消检查；回调终止也不保证省掉前置 diff 计算。保留旧宽松语义的兼容入口或明确标注 API 行为变化。

读取/回滚方面：read_disk 的 max+1 已有限流；file_bytes_at 的不限大小是为原样回滚设计。大文件回滚可另加流式导出 API；写回建议使用同目录临时文件与替换，明确符号链接、权限、文件模式语义后落地，避免半写文件。该项不能以修改“文本读取限制”替代。

### P2/P3：错误、存储与维护

**L. 查询失败被当成干净/空数据。**

多处 `.ok()`、unwrap_or(false/default) 混淆无仓库、无提交、IO 失败和后台任务失败。保留 NotARepo/NoCommits 的友好空态；其它错误记录结构化日志，UI 保留上次成功值并标记 stale。分别记录 discover/status/branches/history/树扫描/磁盘扫描的时间与失败次数，日志避免输出含凭据的 remote URL。

**M. DB 锁内执行 Git IO。**

ProjectStore::list/open/rename 通过 row_to_project 计算 Git updated_ms，连接 Mutex 尚未释放。大量项目或慢盘会延长其它存储操作的等待。先读取纯数据库行并释放锁，再补 Git 派生字段；同一次列表按工作区复用 HEAD 时间，保持 updated_ms=max(HEAD时间,last_active_ms) 的现有排序语义。

**N. 首页最近文件的投影值得修正。**

load_home_recents 调 `file_statuses(repo.root())`，可能给子目录项目挂上整个仓库的文件；它也忽略返回值中的 ignored 标志，忽略目录的 mtime 可能混入“最近文件”。先确定产品口径，再按项目范围过滤、排除 ignored/非普通文件，并对同一物理文件的多项目归属制定去重规则。

**O. 写进程与工程维护。**

系统 Git 的选择继续保留。clone 当前使用 Command::output，没有进度、取消、超时和 stderr 上限；增加带取消令牌/事件回调的选项，保留原同步简易入口。超时策略由产品决定，处理退出码、子进程回收和失败目标目录归属，不自动删用户已有目录。checkout 仍需通过已有 hook/过滤器/冲突语义测试。

Dozer 将 bytegit tag 收到 workspace.dependencies，消费方用 workspace=true 并在各自 crate 声明必要 features。README 示例还写 v0.1.0，更新到实际版本。不要扩大 bytegit 到托管平台账号、提交图布局、宿主调度器或数据库。

## 5. 推荐实现结构与交付拆分

bytegit 提供同步查询、Git 身份解析、底层监听分类；宿主持有异步调度、订阅者、缓存、令牌；共享面板消费纯数据。两产品先复用小型查询/调度辅助模块，等接口稳定再决定独立 crate，避免为去重先搭庞大的新服务。

建议请求流：

```text
notify 事件 / 显式刷新 / 面板激活
  → 按工作区合并失效原因（quiet_delay + max_batch_age）
  → 每类最多一个查询在运行
  → 单次 status + 必要元数据，生成带 generation 的结果
  → 宿主验证 activation_epoch/generation
  → 向 Files、Project、Agent 分发所需投影
```

工作区身份包含规范化 workdir_root 与 git_dir，common_dir 用于共享对象/引用知识，不代替工作区身份。历史只读缓存可进一步按对象库和 commit 共享。Repo 不放进跨线程共享的 Arc<Repo>；优先每个合并任务打开一次，只有基准证明打开句柄显著时再引入专属 worker 长持句柄，并处理 index/config 缓存刷新。

| 批次 | 修改边界 | 可验收结果 | 粗估工作量 |
|---|---|---|---|
| 1 | Digger 事件载荷/分发；两产品刷新令牌；bytegit/宿主监听身份 | 父仓库切分支、同项目乱序、A→B→A 均正确；构建产物不造成刷新风暴 | 3–5 人日 |
| 2 | status_snapshot、Files 纯映射、宿主查询合并 | Dozer 每刷新 status 从 2 次降为 1 次；各类 in-flight≤1 | 2–4 人日 |
| 3 | 事件细分、最大批次时长、目录刷新、磁盘节流 | git add 不重建历史；持续变更可见；队列内存有界 | 3–5 人日 |
| 4 | Usage 汇总与缓存、项目存储锁外 IO、错误遥测 | 一次统计一遍历史；相同 HEAD 命中缓存；慢 Git 不持有 DB 锁 | 2–4 人日 |
| 5 | patch/进程生命周期、真实大仓库基准、文档与发布 | 资源上限明确；CLI 写操作语义保持；无本地 patch 的两产品门禁通过 | 2–4 人日 |

估算是熟悉代码的一名开发者的实施量级，不含跨平台问题修复和产品决策等待；批次 1 的身份解析可以与批次 3 的监听 API 一起设计、分开交付。不要一次性重写三个仓库。

## 6. 本次验证及局限

### 已执行

在 `/private/tmp/bytegit-review.flSVAh` 创建 bytegit 源码与锁文件副本，从本机 crate 缓存构造隔离依赖源，离线运行。没有改动 bytegit/Digger 工作树，也没有改动生产实现。

- 原始代码 `cargo test --offline --locked --all-features`：**168 passed，0 failed**。
- 在副本中追加三项独立行为探针：**3 passed**，确认子目录根无法分类父仓库 HEAD、`.git/config` 被丢弃、20ms 持续事件流在 80ms debounce 下约 300ms 内不投递。它们验证当前缺口，不是修复后的回归测试，也没有把纯分类探针当作 OS 事件集成证明。
- 构造 2,000 个未跟踪文件和 500 个 ignored 文件，对照“discover + is_dirty + discover + status”与“discover + status + 派生 dirty”。debug 构建，预热 4 次，随后 40 次顺序采样：

| 查询方式 | 中位耗时 | P95（nearest-rank） |
|---|---:|---:|
| 两次 status 扫描 | 19.691 ms | 21.144 ms |
| 一次扫描 | 9.943 ms | 10.339 ms |

该夹具中中位耗时减少约 49.5%，两组 entries 与 dirty 一致。它只证明重复扫描可消除，不代表完整 GUI、冷缓存、已跟踪大仓库或发布构建的收益；本探针也没有覆盖非 UTF-8 路径、冲突等新 API 必须处理的边界。

### 后续验收

1. 确定性调度测试：同项目旧/新结果乱序，切项目再切回，关闭后迟到，多个项目共享同仓库，两 linked worktree 私有 index 隔离。
2. 监听测试：子目录项目下外部空提交/checkout、项目外 dirty、git add、remote 配置、持续事件、overflow、删除路径和符号链接别名。逐路径 canonicalize 会跟随 symlink，删除后又只能回退原路径，需同时保留逻辑路径与可靠的根映射。
3. 状态语义测试：ignored 不计 dirty，冲突计 dirty，非 UTF-8 不漏 dirty，子树过滤不改变全仓 dirty，已有 rename/部分暂存口径不变。
4. 历史缓存测试：merge、force reset、rebase、时间戳倒挂、浅克隆 deepen；同一 commit 的不可变结果可复用，窗口和选项变化必须失效。
5. patch 测试：超长单行、UTF-8 边界、零上限、二进制、主动终止和真实后端错误区分；记录峰值 RSS 与生成耗时。
6. 基准矩阵：真实 Dozer/Digger 仓库、至少一个大量文件仓库和一个大量提交仓库；release 构建，冷/热缓存分组，记录 P50/P95、查询次数、in-flight 峰值、路径积压与 UI 停顿。
7. bytegit CI 增加 Linux/Windows 覆盖。现有真实 watcher 测试在零事件时直接返回成功，不能将 168 项全绿视作 FSEvents、inotify 或 Windows 通知端到端均可靠；有能力的集成环境应先验证 watcher 可投递，再严格断言业务事件。
8. 两产品最终以已发布 tag、无本地 patch 的依赖闭包构建并运行相关测试；本次没有执行完整 Dozer/Digger workspace 测试，因此跨产品结论中未动态验证的部分均为源码调用链分析。

## 7. 主要源码入口

- bytegit：`../bytegit/src/{repo,status,watch,history,stats,content,write,info}.rs`。
- Dozer 项目查询与监听：`crates/dozer-app/src/workspace/state.rs:463,1696,1718`。
- Dozer 事件调度：`crates/dozer-app/src/app/update.rs:2542`。
- Files 映射与分支二次加载：`crates/dozer-panel-files/src/git_status.rs:47`、`src/update.rs:59,833`。
- Digger 监听与查询：`../digger/crates/desktop/src/adapters/files.rs:152,197,215`。
- Digger Project：`../digger/crates/desktop/src/adapters/project.rs:687,985`；事件分发 `src/product.rs:3014`。
- Usage：`crates/dozer-panel-usage/src/state.rs:164`、`crates/dozer-app/src/host_adapters/usage_panel.rs:99`。
- 历史路径与回滚：`crates/dozer-app/src/extensions/diff_content.rs:47`、`file_history.rs:478`。
- 项目存储：`crates/dozer-store-project/src/projects.rs:138,181`。
