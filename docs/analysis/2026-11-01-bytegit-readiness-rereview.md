# bytegit / Dozer / Digger 发布就绪第二次复审

日期：2026-11-01。结论：**尚不通过。**
> 修复跟进：Q1（Workspace owner）、Q2（Files 实例 owner）、Q3（Agent 按 WorkspaceKey）已修复并提交；两产品无 patch 克隆 check/测试通过。剩余仅 GUI/跨平台人工验证。 上轮修复已消除依赖阻断并覆盖普通项目切换，但“销毁对象再重建”的生命周期隔离仍有两个缺口，新增 Agent 投影也存在工作区身份误判。

本次基线：Dozer `18818ee5`、Digger `38575d4`、bytegit `5472fa5`（v0.9.2）、ByteUI `2b3973c`（v0.5.1）。审核开始时两个产品工作树干净。未修改生产代码，未执行发布或安装。

依据：[上轮复审](./2026-11-01-bytegit-readiness-review.md)、[规格](../superpowers/specs/2026-10-31-bytegit-refresh-optimization-design.md)、[验收记录](../superpowers/analysis/2026-11-01-bytegit-release-readiness.md)。

## 上轮修复核对

| 项 | 本次核对 |
|---|---|
| N1 依赖闭包 | 两产品已钉 bytegit v0.9.2、ByteUI v0.5.1；Digger 钉 libs-2026-11-01d；两个 offline 依赖守卫通过，不再重复报告旧版本缺包/API 问题 |
| N2 树刷新切项目 | reset_for_project 已清 inflight/pending 并递增 generation；同实例复用问题修复，但全新实例仍复用编号，见 Q2 |
| N3 订阅意图乱序 | runtime 在 discover 前分配 intent，宿主增加 request；同对象内可拒绝旧确认，但 Dozer 重建 Workspace 后 request 重用，见 Q1 |
| N4 首快照交接 | 两宿主 ready 后主动 refresh_manual，已补可靠的后续有效结果路径 |
| N5 HEAD 失效 | Git Log 条件已有 HEAD/REFS/FullRescan，Index 仍不重建 |
| N6 普通目录升格 | 已有 reconcile_git_mode 与 refs_changed 传递和重新订阅路径 |
| N7 缓存生命周期 | provider/cache 已提升至共享 OnceLock，不再每次激活丢弃 |

发布依赖与正常接入已有实质进展；下面只报告本次仍能定位的问题。

## Q1 · P1：Dozer 重开 Workspace 后复用订阅请求编号，旧 ready 会撤销新订阅

位置：`crates/dozer-app/src/workspace/state.rs:522,592`；`crates/dozer-app/src/app/update.rs:2798`。

Workspace 构造时 git_subscription_request=0，第一次 start_git_subscription 变成 1。关闭项目丢弃 Workspace 后，再打开同一 project_id，新对象第一次请求又是 1。旧异步任务的 ready 消息也带 project_id/1，因此会通过当前 request 校验。

具体顺序：

1. 旧 Workspace 的订阅已被 runtime 接受，token_old 的 ready 尚未被 UI 消费。
2. 关闭再打开相同项目；新对象安装 token_new，request=1。
3. 迟到 ready(project_id,1,token_old) 到达，replace 接受旧 token。
4. ready 分支 unsubscribe 被替换的 token_new，撤销了真正有效的 runtime 订阅。
5. 后续 refresh_manual(token_old) 被 runtime 拒绝，项目失去 Git 更新来源。

runtime 的全局 intent 和查询 ID 无法弥补这里的 UI 对象身份碰撞：旧确认不是一份新提交给 runtime 的 Subscribe，而是已完成操作的迟到 UI 消息。

隔离探针使用真实 runtime 创建旧/新 token，再按当前 ready 分支的 request=1/replace/unsubscribe/refresh 规则交错消息，确认有效订阅被撤销、后续没有事件，断言失败。它验证真实 token 语义与宿主规则组合，不是完整 GUI 生命周期自动化测试。

要求：每个 Workspace 生命周期有不可复用的 owner/epoch，并写入订阅请求和 ready 消息；或者请求 ID 使用进程级唯一分配。关闭、占位替换、销毁重建都必须拒绝旧 owner。仅在对象内部递增计数不够。

## Q2 · P1：Files 新实例复用目录刷新 generation，接受旧实例快照

位置：`crates/dozer-panel-files/src/state.rs:622,895,911`；Dozer `app/update.rs:1047`；Digger `product.rs:3250`、`adapters/files.rs:148`。

WorkspaceState 通过 Default 创建时 tree_refresh_generation=0，首次目录读取为 1。现有修复只在 reset_for_project 上递增；若 state 被销毁再 new，新实例又从 1 开始。Digger set_project(None) 明确将 state 设为 None，之后重开会走新建状态；Dozer 关闭重开同 project_id 也会构建新 Files 状态。

两个宿主的目录回包仅携带 project_id/generation。新实例已有 generation=1 的任务执行时，旧实例 generation=1 的回包会通过 apply_tree_refresh 校验，覆盖新树并清除新实例 inflight；真正的新回包随后可能被丢弃。

隔离反例：旧状态读取含 old.txt 的快照 → drop 旧状态 → 磁盘改为 new.txt → 创建新状态并开始首次刷新 → 应用旧快照。实际可见行被替换回 old.txt，断言失败。

该测试导入实际 fs.rs，并原样提取 state 的 reset/begin/apply 方法，替换无关 UI 字段以避免构建整个 iced 依赖；正式完整面板回归补丁已另存。本次不把提取 harness 当作完整面板测试通过。

要求：目录请求/回包携带实例 owner，宿主同时验证项目生命周期和刷新请求；或使用跨实例唯一的请求身份。既有“同一 state 调用 reset”用例不能替代“drop 后 new”的测试。

## Q3 · P2：Agent 用路径前缀冒充工作区身份，嵌套 worktree 被父快照覆盖

位置：Dozer `crates/dozer-app/src/app/update.rs:2768`。

宿主选择 Agent tab 的条件是 effective_cwd().starts_with(workdir)。但“位于某个目录下面”不等于“同一个 Git 工作区”。例如把 linked worktree 放在 `/repo/.worktrees/task`，或项目下存在独立嵌套仓库，其 cwd 满足前缀条件，实际 WorkspaceKey/HEAD/index 均与父工作区不同。

父工作区任一 status 快照到来后，apply_workspace_override 会把父 branch/dirty 写入这些 tab，覆盖 tab 自己的工作区信息，违背“跨工作区 tab 不误用项目状态”的代码注释和规格。

隔离探针实际执行 `git worktree add --detach <outer>/.worktrees/task`：两个 RepoIdentity.workspace_key 不同，当前宿主前缀过滤却返回 true，断言失败。探针核对真实工作区身份与原过滤条件，未创建 Agent UI。

要求：按每个 tab 已解析/订阅的 WorkspaceKey 匹配快照，不按 cwd 路径包含关系判断。身份解析在后台随 cwd 变化更新，避免为修复此项在 UI 线程逐 tab discover。

## 本次验证

隔离副本：`/private/tmp/bytegit-final-review.SXXE3Q`，使用本机缓存依赖；原生产仓库保持未修改。

| 验证 | 结果 |
|---|---|
| 原 bytegit all-features | 231 passed |
| 原 runtime 单测 / 集成 / doctest | 18 / 15 / 1 passed |
| 原 Files fs 模块 | 34 passed |
| Q1 宿主 ready 规则 + 真实 runtime 探针 | failed |
| Q2 原样提取方法 + 真实 fs 模块探针 | failed，显示了已删除的 old.txt |
| Q3 真实嵌套 worktree 身份/前缀探针 | failed |
| Git checkout 后台 hook 的 100ms 超时探针 | passed，未发现此路径的超时问题，不作为缺陷报告 |
| Dozer/Digger offline 共享依赖守卫 | 均通过 |

可应用的回归补丁：

- [runtime/UI 规则与 worktree 探针](../patches/2026-11-01-bytegit-final-review/runtime-ui-regressions.patch)。
- [正式 Files 销毁重建回归](../patches/2026-11-01-bytegit-final-review/files-lifecycle-regression.patch)，等价于本次提取 harness 的用例，未在完整面板 crate 执行。

未重新构建完整 App、未运行 GUI 手工验收、未复核发布包全部哈希；执行方已有 release 构建证据不等于本次审核重跑。验收记录 §3.6 已有构建/打包说明，但 §4 仍写“未执行”，相关最终结论也需在收口时更新，避免下游依赖过时状态。

建议先补跨对象生命周期 owner，联动修 Q1/Q2，再按 WorkspaceKey 修 Q3；随后验证两个宿主关闭重开和嵌套 worktree 的端到端操作。现阶段不建议以“全部完成”关闭发布就绪任务。
