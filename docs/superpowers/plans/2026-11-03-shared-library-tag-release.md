# 共享库标签缺失解阻计划

日期：2026-11-03。状态：**待执行；T0 基线与补丁核对已完成，T3–T6 的推送/tag/正式锁仍待授权执行。** 目标是实际发布 `byteui/v0.5.2`、`bytehost/v0.4.22`、`dozer/libs-2026-11-03a`，并让 Digger 使用真实已发布依赖通过干净 locked 验证。

本计划只处理标签、对应源码、消费版本和正式锁文件。整体稳定化的 GUI、真实 agent、性能和其它验收继续由 [稳定化总计划](2026-11-03-byteboy-stabilization.md) 跟踪，不因三个标签出现而自动完成。

交接依据：[审核记录](../../analysis/2026-11-03-byteboy-stabilization-review.md)、[补丁说明](../../patches/2026-11-03-byteboy-stabilization/README.md)、[补丁基线与 SHA256](../../patches/2026-11-03-byteboy-stabilization/manifest.json)。补丁是可审阅的实现起点，执行前重新检查是否已有等价修改，不能重复覆盖。

## 1. 当前事实与最终版本矩阵

上次远端检查三个标签均不存在。ByteUI 本地 `eb08e57` 包含 NotebookTabs，但远端 main 仍为 `2b3973c`；ByteHost 本地/远端为 `0362608`；Dozer 本地 `6e389a30` 包含新运行时，远端 main 仍为 `a9e02c5`。这些是检查时点事实，执行前须刷新。

| 发布者 | 目标标签 | 必须包含/消费的内容 | 完成后消费者 |
|---|---|---|---|
| ByteUI | v0.5.2 | NotebookTabs/SVG；package version 0.5.2；CHANGELOG；必要候选映射 | ByteHost、Dozer、Digger |
| ByteHost | v0.4.22 | 所有 ByteUI 消费声明统一 v0.5.2；本批选入的修复与 CI | Dozer、Digger |
| Dozer | libs-2026-11-03a | 最新共享 API/运行时；ByteUI v0.5.2、ByteHost v0.4.22；正式 git 来源 lock | Digger 的全部同源 Dozer crates |
| Digger | 正常修复提交 | 上述三个实际 tag；统一的 Dozer libs 快照；正式 git 来源 lock | 产品构建与验收 |

ByteGit 保持 v0.9.2。ByteHost 的 crate package version 沿用仓库规则，不为 tag v0.4.22 把所有 crate 强改版本；Dozer libs tag 与 GUI App 发布号分开。

### 2026-11-03 复核补充

- 远端事实未变：ByteUI main `2b3973c`、ByteHost main `0362608`、Dozer main `a9e02c5`、ByteGit main `5472fa5`；三个预留标签仍不存在。Digger 仍无 remote。
- 四份补丁 SHA256 与 manifest 一致，ByteUI/ByteHost/Digger/Dozer 的 `git apply --check` 均通过。
- 二次审核 R1/R2/R4 已修复：Dozer CI 同时运行真实门禁与自检；Digger 的单图门禁与实机自检入口已落到 `../digger` 工作树，15 项门禁自检、4 项 editor-selftest 自检通过。Digger `Cargo.toml`/`Cargo.lock` 保持旧版本，未用 path 图或假锁冒充发布。
- R3 仍阻断：三个上游 tag 未发布，Digger 正式 locked 构建仍因 lock 需要更新而失败。T3–T6 必须在获得推送/tag 授权后执行。

**依赖顺序：T0 核对 → T1 源码落地 → T2 统一候选验证 → T3 ByteUI 发布 → T4 ByteHost 发布 → T5 Dozer libs 发布 → T6 Digger/两产品正式验证 → T7 发布记录。**

## 2. T0：核对权限、基线与已有发布

- [ ] 确认相关仓库及 git 元数据可写；保留全部原有修改和个人 Cargo 配置，不 reset/clean。
- [ ] 记录各仓库 HEAD、分支、status、remote 和实际发布规则；核对本地未推送提交是否属于本批必要前置。
- [x] 用 `git ls-remote` 重新核对三个远端标签及 main。Digger 远端为 `https://github.com/byteboyai/digger.git`，本地 remote 缺失时核对后补配，不覆盖已有正确 remote。
- [x] 校验补丁 hash，按当前源码重新 dry-run；若修改已落地则审阅等价实现，不再次 apply。
- [ ] 若标签已存在，先获取并检查其剥离后的 commit、内容和消费者状态：内容正确则接续验证；内容错误则停止并报告，选择新标签并统一修改引用，不移动/强推旧标签。

完成产物：实际基线、远端标签状态与确定的目标发布分支。此阶段不创建标签。

## 3. T1：将所需源码落到发布提交

- [ ] ByteUI：落地 byteui.patch，确保本地 NotebookTabs 提交及资源也进入待发布分支；版本、CHANGELOG 与待发布功能一致。
- [ ] ByteHost：落地 bytehost.patch，审阅测试 helper 清理、联合检查及 ByteUI 钉版；保留生产进程身份/权限校验。
- [ ] Dozer：保留最新运行时及当前门禁修复；审阅 dozer-release.patch 的 ByteUI/ByteHost 版本升级。发布目标必须包含 Digger 所需的 SaveOutcome、SaveFile.project_root 等共享 API。
- [ ] Digger：落地 digger.patch，准备统一版本清单与门禁/自检修复；正式 Cargo.lock 等上游实际发布后生成。
- [x] 检查 Dozer CI 的真实依赖门禁和隔离自检分别执行，不能因 `--self-test` 提前返回而跳过真实图；以当前实际文件为准，不覆盖其它会话已完成的修复。
- [ ] 将行为修复与版本/lock 更新组织成可审阅提交。不得包含临时 path patch、个人绝对路径、候选 lock、凭据或无关修改。

完成产物：四仓库待发布/消费提交，列明每个提交的源码差异及必要前置。原有本地 HEAD 只是基线，不能直接给它补目标 tag 当作完成。

## 4. T2：发布前统一候选验证

这一阶段证明源码组合兼容，不能替代发布后的正式 git 来源检查。

- [ ] 固定 ByteUI、ByteHost、Dozer、Digger 的候选精确 SHA；存在未提交候选修改时先形成可复核提交。
- [ ] 在临时副本构建统一候选图：所有相关共享 crate 使用对应候选源码，特别包含 dozer-file-io/dozer-git-runtime；显式确认 ByteUI 选中 0.5.2，不能忽略 patch 未使用警告而实际测试旧版。
- [ ] 为尚不存在的 tag 使用明确记录的候选解析方案：临时沿用已存在 git tag 作为解析起点并同源 patch，或等上游实际发布后再验证对应声明。临时清单和 path lock 不进入发行提交。
- [ ] 两产品候选 workspace all-targets 与必要契约测试通过；各共享库仅一个来源，测试旧 trait fake、线上 JSON 和新共享保存接口兼容性。
- [ ] ByteUI 按仓库要求执行 fmt、clippy、单测和示例检查；ByteHost 执行默认/all-features、客户端契约、边界和故障测试。正常系统权限环境的进程恢复失败须先查清，不能改 ignored 或放宽断言。
- [ ] 两产品门禁/入口脚本回归通过，并以非法真实依赖验证实际 CI 命令确实失败；测试数量以最终源码为准。
- [ ] 修正联合 workflow 的组合输入：发布前候选检查使用本次固定的四仓快照；仅 patch ByteHost、其余仍拉旧 main，不能证明新组合兼容。缺任一消费者即失败，不跳过 Digger、不屏蔽错误。
- [ ] 将旧消费版本兼容检查与本次候选检查分别标识。tag 自动联合检查若仍拉旧消费者，须改为使用已记录的验证快照；最终正式消费者检查在 T6 执行，避免缺未来 tag 形成发布循环。

完成产物：候选源码矩阵、解析方式、实际测试结果和 CI run URL/SHA。任一发布提交在验证后改变内容，需要重跑受影响验证并更新 tested SHA。

## 5. T3：发布 ByteUI v0.5.2

- [ ] 确认最终提交含 NotebookTabs、SVG、version 0.5.2 和发布说明，发布前验证通过。
- [ ] 将实际测试的提交纳入目标发布分支；若通过 PR 合并改变了 commit SHA，重新确认/验证最终内容。
- [ ] 在该精确提交创建 v0.5.2，并将必要分支提交与该标签推送到正确远端。
- [ ] 重新从远端获取标签，核对剥离后的 commit 等于 tested SHA；记录远端 URL/tag/SHA。
- [ ] 在无个人 patch 的临时消费者中确认能通过 git tag 解析到 ByteUI 0.5.2。

完成标准：远端标签存在、指向正确源码、消费方真实解析可用；仅本地有 tag 不算完成。

## 6. T4：发布 ByteHost v0.4.22

前置：T3 完成。

- [ ] ByteHost 所有实际消费 ByteUI 的清单统一使用已发布 v0.5.2，检查 chrome/settings-ui 等容易遗漏的 crate。
- [ ] 使用实际已发布 ByteUI 验证 ByteHost，不能继续靠本地 ByteUI path patch。
- [ ] 按仓库规则验证库本身、公开契约与候选消费者，记录正常权限环境的孤儿恢复结果；若仍失败，先修再发布。
- [ ] 创建并推送指向最终 tested SHA 的 v0.4.22；从远端重新获取并核对 commit。
- [ ] 确认消费者能解析已发布的九个 ByteHost crate，同一仓库所有 crate 使用同一 tag/SHA。

完成标准：远端 v0.4.22 可用，内部 ByteUI 依赖正确，实际源码/测试和 tag 对应。

## 7. T5：发布 Dozer libs-2026-11-03a

前置：T3、T4 完成。

- [ ] 更新 Dozer 的 workspace 依赖到真实 v0.5.2/v0.4.22，确认必要本地运行时/API 提交和门禁修复已包含。
- [ ] 在不含个人 .cargo patch 的源码副本中解析已发布依赖，按共享库变更定向更新正式 Cargo.lock；审阅差异，避免无关 registry 依赖漂移。
- [ ] 将正式锁带回对应工作树，提交后从该实际提交再做干净验证；不能手填未来 git SHA 或把 path lock 作为正式锁。
- [ ] 运行 locked workspace all-targets、运行时/client/panel/文件 IO 相关回归与真实边界门禁，记录实际结果。
- [ ] 创建并发布统一 libs-2026-11-03a，目标为完整 tested SHA；共享 crates 来自同一提交，不分别引用不同快照。
- [ ] 从远端重新取 tag，确认 Digger 所需的新类型/字段和必要新 crate 实际在该提交中。

完成标准：远端 libs tag 的源码与依赖图均正确；不能只因为名字存在就认为缺 API 已解决。

## 8. T6：Digger 消费与两产品正式发行验证

前置：三个上游 tag 均已远端验证。

- [ ] Digger 的 ByteUI/ByteHost/全部同源 Dozer crates 统一引用目标标签；ByteGit 保持已验证版本。
- [ ] 从无个人 patch 的源码副本生成正式 git 来源 lock，修复共享包缺 source 的问题；不删除开发者的个人联调配置。
- [ ] 将 lock 纳入实际提交，核对 source 中的 tag 和 commit 与发布矩阵一致。
- [ ] 从两产品实际待交付提交重新创建干净 checkout；不复制个人 .cargo、不用 URL rewrite/本地仓库替代官方 git 来源。
- [ ] 两产品运行下列公共门禁，另补各自核心回归。库仓库不跟踪锁时可生成临时锁测试，但不能把它算作产品正式 locked 验证。

```bash
cargo fmt --all --check
cargo check --locked --workspace --all-targets
bash scripts/check-host-deps.sh
bash scripts/check-host-deps.sh --self-test
bash scripts/check-shared-deps.sh --offline
```

- [ ] Dozer 增加 panel boundary/log scope 与相关运行时/客户端/面板测试；Digger 增加 editor/store/paths/app-host、前端检查/测试、parser fixtures 和嵌入产物一致性。
- [ ] 两产品依赖图没有重复共享库来源，真实 CI 在本次最终提交上通过并保留链接。网络/缓存/权限失败单独列出，不能改用个人 patch 后记为通过。
- [ ] 推送实际消费提交及正式锁，核对远端目标分支包含它们。

完成标准：远端不再缺标签、产品不再缺接口/正式锁，干净 locked 构建和相应契约均通过。

## 9. 推送操作模板与失败处理

以下只是实施模板，本计划编写时不执行发布。执行者先完成该阶段全部验证，再填入实际 remote/分支/tested SHA/tag；不能直接把当前 HEAD 当作 tested SHA。

```bash
# 在对应发布者仓库执行，使用实际已验证参数。
RELEASE_REMOTE="origin"
RELEASE_BRANCH="main"
RELEASE_SHA="<完整已验证提交SHA>"
RELEASE_TAG="<本阶段标签>"

git tag -a "$RELEASE_TAG" "$RELEASE_SHA" -m "Release $RELEASE_TAG"
git push --atomic "$RELEASE_REMOTE" \
  "${RELEASE_SHA}:refs/heads/${RELEASE_BRANCH}" \
  "refs/tags/${RELEASE_TAG}:refs/tags/${RELEASE_TAG}"
git ls-remote "$RELEASE_REMOTE" \
  "refs/tags/${RELEASE_TAG}" "refs/tags/${RELEASE_TAG}^{}"
```

- 先检查远端和本地同名标签；已存在时核对，不用 force 覆盖。
- 受保护分支按仓库要求走 PR，合并后以实际合并提交为验证对象；不绕过保护。
- atomic 不支持、认证失败、非 fast-forward 或 CI 失败时停止并报告，不静默退化为部分发布，不推送无关标签。
- annotated tag 的对象 SHA 与剥离后的 commit SHA 分别记录；tested SHA 应与 commit 比较。
- 一旦标签已发布不得移动；内容错误时通过修复提交和新版本更正，更新消费者引用，保留历史。

## 10. T7：交付证据与完成定义

- [ ] 提交发布记录，至少包含仓库 URL、标签、tag 对象/commit SHA、所含接口、上游版本、实际测试命令/结果和 CI链接。
- [ ] 记录两产品最终提交、正式锁中的各 git 来源 SHA及干净构建日志。
- [ ] 更新 [稳定化总计划](2026-11-03-byteboy-stabilization.md) 的发布/正式锁任务；保持尚未完成的 GUI、真实 agent 与性能待办。
- [ ] 重新从远端查询三个标签，并从实际 git 来源跑最终消费检查，确认问题确实解阻。

只有三个标签的正确源码已远端可取、两产品消费与正式锁/契约通过，才标记本专项完成。单独创建标签、推送本地分支、候选 patch 编译通过，都不足以完成本专项。
