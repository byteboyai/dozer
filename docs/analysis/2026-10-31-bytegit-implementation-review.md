# bytegit 刷新优化实施审核

日期：2026-10-31。结论：**当前交付不通过审核，应先修复 R1/R2，再处理 R3/R4；整个 spec/plan 尚未完成。**

修复跟进：R1/R2 已修复到 Dozer 工作树；R3/R4 已在隔离副本修复并验证，受当前会话写权限限制以[可应用补丁](../patches/2026-10-31-bytegit-audit/README.md)交付，尚未写回 bytegit。以下保留原审核发现和失败证据，不表示修复后的测试结果。

依据：[规格](../superpowers/specs/2026-10-31-bytegit-refresh-optimization-design.md)、[实施计划](../superpowers/plans/2026-10-31-bytegit-refresh-optimization.md)。本次未修改生产代码，回归测试只在 `/private/tmp/bytegit-audit.eGgEFr` 的隔离源码副本中追加。

## 审核范围与实际交付

- bytegit：`ae60c83` 到 `4a4951b`，主要功能提交 `9d47571`；当前版本 0.9.0。
- Dozer：HEAD `f5bd3ac5`，审核未提交的 `crates/dozer-git-runtime/` 及 bytegit workspace 依赖调整。
- Digger：HEAD `2b21b0f`，当前工作树无本次产品接入变更，仍声明 bytegit v0.8.1。
- Dozer 中 agent_paths、headless_agent、summary_provider、transcripts 等其它未提交改动不属于本次审核，未评价也未修改。

计划自身仍写“实施中”：T1/T3 有明确未完成子项，T2 的 Files 映射未做；T5 新 runtime 已交付，但 T6–T13 的面板接口、两个宿主接入、统计缓存、目录/磁盘优化、存储、patch/CLI 与最终交付均未完成。检索 Dozer app、共享 Files、Digger desktop 未发现 GitRuntime 接入。因此当前最多作为底层与 runtime 的阶段性交付审核，不能作为整个方案完成验收。

## R1 · P1：退订重建后，旧查询结果会被包装成新订阅结果

位置：Dozer `crates/dozer-git-runtime/src/scheduler.rs:219`、`:510`、`:716`。

最后一个订阅移除时 WorkspaceEntry 被删除；相同 WorkspaceKey 重建后 query_seq 和 Generations 都重新初始化。旧 QueryFinished 只携带 WorkspaceKey/query_id/generations，没有工作区实例 epoch。新旧首次请求都是 query_id=1，finish_query 会拿走新实例的 inflight，用新实例的 generations 校验旧任务，然后以当前 subscribers 的新 token 投递旧结果。

可复现场景：第一次查询阻塞 → 退订 → 重订同工作区并启动第二次阻塞查询 → 仅释放旧查询。实际收到 branch="old" 的快照，event.activation_epoch=1，与新订阅 epoch=1 完全相同，宿主 token 校验无法拦截；真正新查询完成后还可能因 inflight 已被拿走而丢弃。

隔离回归：`audit_old_query_cannot_be_retagged_as_reopened_subscription` **失败**。

修复要求：为工作区实例分配不复用的 incarnation，完成消息校验 incarnation 和 query_id；或使用运行时全局唯一查询身份。不要只比对 entry 自己保存的 launched generation，必须确认完成消息属于该运行实例。WatchStarted 的 generation 当前也固定从 0 开始，应一并加实例隔离，防止旧监听创建结果挂到新实例。

对应规格 D6、A2。现有 A→B→A/退订测试均先等查询完成，再切换，没有覆盖真正的执行中重开。

## R2 · P1：退订提前释放 permit，真实并发能突破全局上限

位置：Dozer `crates/dozer-git-runtime/src/scheduler.rs:286`、`:724`。

OwnedSemaphorePermit 存在 WorkspaceEntry.inflight 中，最后一个订阅退订后删除 entry，permit 立即释放；spawn_blocking 中实际查询仍在运行。重开项目就能获得新 permit 再启动一个查询。重复关闭/重开可积累实际阻塞任务，队列指标却看不到已被删除 entry 的工作。

隔离回归：配置 max_concurrent_queries=1，阻塞第一次查询，退订重订，在释放第一次查询之前观察到实际并发=2。`audit_unsubscribe_must_not_release_a_running_query_permit` **失败**。

修复要求：permit 生命周期至少覆盖真实后台查询执行；由执行任务持有至完成，或者保留与订阅无关的 running registry。取消订阅只取消应用结果，不应释放尚在运行的资源预算。如果保留“同工作区单任务”的保证，重订时也要识别旧实例仍在运行的读取任务。

对应规格 §6.1、A4。R1/R2 需分别修复，给查询加唯一 ID 并不能解决真实并发超限。

## R3 · P2：未处理 notify 的 rescan 标志

位置：bytegit `src/workspace_watch.rs:258`、`:429`。

classify_event 只对 paths 为空、路径无法解释或根目录结构事件设置 FullRescan，没有检查 Event::need_rescan()。后端若提示事件流已不完整，且事件附有 `.git/objects/...` 等被过滤路径，本批次直接被丢弃；即使附带普通文件路径，也只刷新该路径，无法补回此前丢失的变化。Access 的提前过滤也不应优先于 rescan 信号。

隔离回归：构造携带 Flag::Rescan、路径 `/r/.git/objects/x` 的事件，确认 need_rescan() 为 true，分类输出却没有 FULL_RESCAN。`audit_rescan_flag_must_override_an_irrelevant_path` **失败**。

修复要求：在 Access/路径过滤前检查 rescan，强制设置 FULL_RESCAN 并清空不完整明细；添加带相关路径、无关路径和 Access 类型的回归。后续定时校验不能替代通知层的此项正确性。

对应规格 §5.2、A4/A5。

## R4 · P2：linked worktree 未实际监听部分共享配置/历史边界

位置：bytegit `src/workspace_watch.rs:650`；分类分支见 `:574`。

extra_targets 给工作区外的私有 gitdir 加递归监听，给 common_dir 只加非递归监听，再额外递归监听 refs。它没有覆盖 common_dir/info/exclude、info/attributes、info/grafts、objects/info/alternates。classify_git 虽然给这些路径返回 CONFIG/HISTORY_BOUNDARY，后端没有对应监听目标就无法可靠送达。

示例：workdir=/w，gitdir=/main/.git/worktrees/w，common=/main/.git。在要求非递归监听只覆盖直接子项的平台上，修改 `/main/.git/info/exclude` 不会触发工作区状态失效，dirty/ignored 装饰保留旧值直到其它刷新。

隔离回归：检查 extra_targets 返回的路径和递归模式，未找到覆盖 common info/exclude 的目标；`audit_linked_worktree_must_watch_common_exclude` **失败**。这是监听目标覆盖测试，不是对 Linux/Windows 的真实事件投递测试。

修复要求：增加共享 info、objects/info 的必要监听，处理目录不存在/重建并在失败时暴露 degraded。不要通过递归监听整个 objects 大目录弥补，避免引入对象写入风暴。除现有 refs 缺失限制外，此处也是独立的监听覆盖缺口。

对应规格 §5.1、A5/A6。

## 测试证据与交付限制

从当前源码复制 bytegit/runtime，使用当前 Dozer Cargo.lock 中的缓存依赖构造隔离离线 workspace，禁止写入用户全局 cargo 缓存，未构建完整 GUI：

| 测试 | 结果 |
|---|---|
| 原 bytegit all-features | 219 passed |
| 原 runtime 单元测试 | 12 passed |
| 原 runtime 集成测试 | 13 passed |
| 原 runtime doctest | 1 passed |
| 审核追加的 runtime 回归 | 2 failed，分别确认 R1/R2 |
| 审核追加的 bytegit 回归 | 2 failed，分别确认 R3/R4 |

回归测试补丁已保存，均不含生产实现修改：

- [Dozer 回归补丁](../patches/2026-10-31-bytegit-audit/dozer-regressions.patch)：从 Dozer 根应用到 runtime 集成测试。
- [bytegit 回归补丁](../patches/2026-10-31-bytegit-audit/bytegit-regressions.patch)：从 bytegit 根应用到 workspace_watch 内部测试。

在各自隔离仓库运行 `cargo test -p dozer-git-runtime --test runtime audit_` 与 `cargo test --all-features audit_` 可复核；当前版本预期失败。补丁只记录本次审核反例，正式修复还应补全新结果成功送达、进程退出/错误路径、三平台事件投递等回归。

另外，Dozer 当前 Cargo.lock 已移除 bytegit/byteui 的 Git source，反映本地 patch 联调；T13 要求的无本地 patch 锁文件和跨产品依赖闭包尚未验收。此次没有运行完整 Dozer/Digger workspace 门禁，也不把这些未完成事项宣称为已验证。

建议处理顺序：先修复 R1/R2 并以执行中切换测试替换仅检查 token 数字变化的覆盖，再补 R3/R4；通过阶段审核后继续 T6–T13，最后执行整体 A1–A9 验收。
