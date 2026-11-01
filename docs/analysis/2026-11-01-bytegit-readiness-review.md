# bytegit / Dozer / Digger 发布就绪复审

日期：2026-11-01。结论：**审核不通过，尚未具备发布条件。** 两产品确实新增了 runtime 接入，但依赖闭包未收口，接入层仍有订阅时序、目录刷新和失效分类问题。

> 修复跟进（2026-11-01）：N1–N7 已修复并推送（bytegit `v0.9.1`、ByteUI `v0.5.1`、bytehost `v0.4.21`、Dozer `libs-2026-11-01d`）；两产品无本地 patch 克隆的 check/核心测试通过。CLI 可取消进程 API 已在 bytegit `v0.9.2` 落地；剩余：产品取消 UI、release 四件套与 GUI/跨平台手工验收。详见执行记录 §3.5。

依据：[交接计划](../superpowers/plans/2026-11-01-bytegit-release-readiness-handoff.md)、[规格](../superpowers/specs/2026-10-31-bytegit-refresh-optimization-design.md)、[执行方验收记录](../superpowers/analysis/2026-11-01-bytegit-release-readiness.md)。执行方记录自身也仍标记“进行中/未就绪”，不能将它解释成最终验收通过。

## 范围与基线

- Dozer：HEAD `f18f5a91`，审核本次 Git 重构相关的工作树 diff 与未跟踪 runtime。
- Digger：HEAD `2b21b0f`，审核其工作树接入改动。
- bytegit：HEAD `6c91137`，包括此前 R3/R4 的实际修复 `8763899` 与新严格 patch API。
- ByteUI：HEAD `541554f`。未执行 App 发布、版本递增或安装；未修改生产源码。

此前审核的查询完成 ID / permit 生命周期及 rescan / 控制目录监听已有修复，本次主要审核新宿主接入和发布闭包，而非重复提出旧缺陷。

## N1 · P1：声明的远端依赖不包含当前使用的代码

位置：Dozer `Cargo.toml:64`、`crates/dozer-app/src/extensions/file_history.rs:465`；Digger `Cargo.toml:79`。

- Dozer 仍钉 bytegit v0.9.0，但已调用 workdir_patch_limited/PatchOptions。直接读取该 tag 的 src/lib.rs，确认没有这些 API；它们只存在于之后的 6c91137。
- Digger 的 dozer-git-runtime 钉在 libs-2026-10-30a。执行 `git show libs-2026-10-30a:crates/dozer-git-runtime/Cargo.toml` 明确报该路径不在 tag 中。
- 两产品的共享依赖守卫均报 `BLOCK lock-not-git`，当前成功构建依赖本地 patch。此处不是单纯“无 patch 尚未测试”，而是可定位的真实版本不匹配。

影响：去掉本地覆盖后，Dozer 无法编译新 patch 调用，Digger 无法解析新 runtime；已经发布的 tag 也不包含当前面板入口。

要求：共享库修复进入真实可获取版本后，再协调更新两个产品及锁文件，并在无本地 patch 的隔离 checkout 验证。不能通过提交当前本地来源锁文件或保留占位 tag 关闭本项。

## N2 · P1：切项目会遗留目录刷新 in-flight 状态，导致新项目不再启动刷新

位置：`crates/dozer-panel-files/src/state.rs:631`、`:868`；Digger `crates/desktop/src/adapters/files.rs:156`、`src/product.rs:3211`。

reset_for_project 替换树但未重置 tree_refresh_inflight、pending 路径或刷新身份。若 A 的后台目录读取尚未返回就切到 B，B 的 begin_tree_refresh 只积压 pending 并返回 None。A 的完成消息又被 Digger 以 project_id 不匹配丢弃，因此没有任何任务能清掉 inflight，新项目刷新永久卡住。

复现：开始 A 的全量刷新 → reset_for_project(B) → 请求 B 全量刷新，预期能启动，实际返回 None。对源码原样提取的 reset/begin/apply 方法与真实 fs 模块执行隔离测试，已失败；未用这项提取测试冒充完整面板构建。

要求：项目切换时撤销旧任务的应用资格、清空 pending/inflight，并分配不复用的生命周期身份；迟到结果同时校验项目生命周期和刷新 generation。仅把数值 generation 归零还会引入 A→B→A 的旧结果碰撞。

## N3 · P1：异步 discover/订阅确认没有请求意图版本，旧订阅能覆盖新订阅

位置：`crates/dozer-git-runtime/src/lib.rs:451`、`scheduler.rs::subscribe`；Dozer `app/update.rs:2741`；Digger `product.rs:3198`。

订阅 epoch 在 discover 完成后、调度器接受 Subscribe 时才分配。两次同 project_id 请求若较早一次 discover 更慢，旧意图会在最后进入调度器，撤销较新的有效订阅并获得更大的 completion epoch。Dozer 的 ready 分支只检查 project_id，随后直接 replace token；Digger 也只检查当前项目 id。宿主没有启动订阅前的 request generation，也没有核对确认消息属于哪次项目生命周期。

此外，即使 discover 顺序正常，两个异步 ready 消息也可能乱序处理：旧 ready 替换当前 token 后，Dozer 还会 unsubscribe 被替换的新 token，使宿主保存一个 runtime 已经不承认的旧 token。

隔离探针：阻塞旧订阅的 discover，先完成同 id 的新订阅，再放行旧 discover。旧工作区快照实际以 activation_epoch=1 投递，而期望的新工作区 token 为 epoch=0，断言失败。该探针验证 runtime 的乱序语义；宿主误接受的结论来自对应 ready 分支的源码检查，不是完整 GUI 场景重放。

要求：订阅意图在 discover 之前分配版本；runtime 与宿主都拒绝已过期的意图/项目生命周期。不能只对 ready 消息比较“哪个 token 数字更大”，因为当前数字反映完成顺序，不代表用户意图顺序。

## N4 · P1：首个快照可能先于 token 确认被丢弃，随后不会补发

位置：Dozer `app/app.rs:979`、`app/update.rs:2673,2765`、`workspace/state.rs:514`；Digger `product.rs:961,982,3198`。

runtime 事件转发与 subscribe 完成消息是两条独立任务路径。首次 Snapshot 在宿主安装 token 之前到达时，会被 token_current/Option 比较直接拒绝；ready 处理只调用 set_active(true)。订阅本来已 active=true，因此 set_active 不触发查询，也不重放最后快照。

影响：无后续文件事件时，前台新开项目的分支、dirty、文件着色可能保持空态，直到约 60s 定时重验。新订阅复用同仓已有快照的路径甚至会在 Subscribe reply 前发布事件，同样需要明确握手。

隔离时序探针：消费并模拟宿主丢弃初始快照 → 安装 token/调用 set_active(true) → 等待重放，实际无事件，断言失败。该测试固定的是允许出现的消息顺序，不是声称 runtime 的 set_active API 单独违反了其契约；缺陷在两者组合没有建立可靠交接。

要求：订阅确认携带可应用快照、宿主确认前缓存并校验快照，或显式握手后再发布/重放；至少保证 ready 后一定有一次有效结果。不要仅依赖两个任务“通常按预期调度”。

## N5 · P2：Git Log 刷新漏掉 HEAD 事件

位置：Dozer `crates/dozer-app/src/app/update.rs:2649`。

注释写“引用/HEAD 变化”，实际条件只有 REFS || FULL_RESCAN。外部切换两个已有分支、detach 到另一提交可能只改私有 HEAD/index 与工作区，不改 refs；新 watcher 恰好已将 HEAD 与 REFS 区分开。这时 Project/Files 分支更新，但已打开的 Git Log 仍展示旧分支提交图。

要求：HEAD 纳入图失效，继续保持 Index 不重建图；增加外部 checkout 已有分支、detached HEAD、纯 git add 的对照回归。

## N6 · P2：普通目录初始化 Git 后没有升格为 runtime 订阅

位置：Dozer `workspace/state.rs:497`、`app/update.rs:2566`；Digger `adapters/files.rs:159,178`、`product.rs:965`。

Dozer 初次发现不是仓库就只建立 plain_watch，PlainFsChanged 只刷树/预览/磁盘，不重做 discover；Digger 的 runtime.subscribe 失败只打印错误，plain watch 还把 refs_changed 丢掉。目录后来 git init 或 `.git` 被创建后，两条路径均没有“重新 discover → 停旧 watcher → 建 runtime 订阅”的晋升流程。

影响：从普通目录开始的项目不能可靠获得实时 Git 状态，直到重开项目；已有仓库 `.git` 消失后重建也缺少完整恢复链路。此问题在执行记录中被描述为“宿主兜底”，但当前兜底只覆盖文件树。

要求：保留身份相关事件和项目生命周期版本，在控制目录创建/删除及补偿重验时重新判断身份；分别测试普通目录初始化、仓库暂时失效后恢复。

## N7 · P2：Usage 的统计缓存随每次激活重建，实际调用没有跨刷新命中

位置：Dozer `crates/dozer-app/src/host_adapters/usage_panel.rs:236`。

activate 每次 Arc::new(DoGitStats::new())，StatsCache 随 provider 新建。后台任务只调用一次 summary，结束后 provider 被释放。故同一 HEAD 连续切入 Usage 仍每次扫描历史；测试直接复用同一个 DoGitStats，无法覆盖真实宿主生命周期。

要求：在 App/共享服务或稳定的项目状态中保留 provider/cache；手动刷新通过明确的失效机制绕过缓存。增加实际 activation 连续两次、同 HEAD 零新增历史遍历的回归。

## 验证记录与未完成范围

在 `/private/tmp/bytegit-readiness-review.dQPAe5` 使用当前 bytegit/runtime 源码与本地缓存依赖构造隔离 workspace，未改变生产实现。

| 验证 | 实际结果 |
|---|---|
| 当前 bytegit all-features | 227 passed |
| 当前 runtime 原单测 / 原集成 / doctest | 17 / 15 / 1 passed |
| 原始 Files fs 模块 | 34 passed（隔离导入实际 fs.rs） |
| 新目录切换反例 | 1 failed（提取实际 state 方法、替换无关 UI 字段） |
| 新订阅乱序 / 首快照交接反例 | 2 failed（隔离 runtime 集成探针） |
| 两产品共享依赖守卫 --offline | 均失败，BLOCK lock-not-git |
| v0.9.0 API / libs-2026-10-30a runtime 路径核对 | 确认所需 API/包缺失 |

反例补丁：

- [runtime 时序反例](../patches/2026-11-01-bytegit-readiness-review/runtime-regressions.patch)。
- [Files 项目切换回归](../patches/2026-11-01-bytegit-readiness-review/files-reset-regression.patch)。后者为正式面板测试的等价用例，当前仅在提取方法 harness 中执行，未在完整面板 crate 运行。

本次未重新执行完整 Dozer/Digger workspace、GUI 或 release 构建，不能把执行方既有测试数字当作此次复审亲自验证的结果。Linux/Windows 事件及进程树回收也未验证。

原计划仍有明确缺项：CLI 可取消进程回收未实现；Agent 的 apply_workspace_override 新入口只见库内测试，宿主快照处理未调用；宿主快照投影未传递 stale/error 状态；工作区事件的磁盘任务仅限流启动时间，没有完整单飞行/补跑策略；目录结果应用仍调用 is_dir。它们需要继续逐项对照规格，不能将“App 已引用 runtime”写成 A1–A9 全部完成。

建议顺序：先修 N2–N4 的生命周期问题，再修 N5/N6 并补宿主集成测试，落实 N7 缓存生命周期与剩余规格项；最后收口 N1 的真实依赖版本和无 patch 验收。仍不建议递增 App 版本或宣告发布就绪。
