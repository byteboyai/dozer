# bytehost 优化五梯队 总控执行计划(Index)

> 日期：2026-10-30。状态：待实施。
> 本文是 2026-10-30 bytehost 全面分析产出的五份 spec/plan 的总控索引与执行顺序。各梯队的改动细节、验收标准以对应 spec/plan 为准，本文只定顺序、依赖与协调点。

## 文档索引

| 梯队 | Spec | Plan |
|---|---|---|
| Tier1 低风险机械改动 | [spec](../specs/2026-10-30-bytehost-opt-tier1-mechanical-cleanup-design.md) | [plan](2026-10-30-bytehost-opt-tier1-mechanical-cleanup.md) |
| Tier2 去重与收敛 | [spec](../specs/2026-10-30-bytehost-opt-tier2-dedup-convergence-design.md) | [plan](2026-10-30-bytehost-opt-tier2-dedup-convergence.md) |
| Tier3 结构性重构 | [spec](../specs/2026-10-30-bytehost-opt-tier3-structural-refactor-design.md) | [plan](2026-10-30-bytehost-opt-tier3-structural-refactor.md) |
| Tier4 运行时性能 | [spec](../specs/2026-10-30-bytehost-opt-tier4-runtime-perf-design.md) | [plan](2026-10-30-bytehost-opt-tier4-runtime-perf.md) |
| Tier5 跨仓库治理 | [spec](../specs/2026-10-30-bytehost-opt-tier5-cross-repo-governance-design.md) | [plan](2026-10-30-bytehost-opt-tier5-cross-repo-governance.md) |

## 排期原则

1. **门禁先行**：Tier5 的门禁（Task1/2）最先落地，保护后续所有改动。
2. **tag 先上收**：Tier1 Task1 必须先于任何 bytehost 发版升级，否则升 tag 要改 25 处。
3. **bytehost 改动攒一次发版**：Tier1 后半 + Tier3 + Tier4 全部进同一个 bytehost 分支、发一个新 tag，dozer/digger 只同日升级一次。
4. **消费侧去重可并行**：Tier2 的 dozer 线与 digger 线互不依赖。
5. **联合 CI 收尾**：Tier5 Task4 放最后，建成后未来的 bytehost 发版自动带两消费方试构建，人工协调消除。

## 执行顺序

### Phase 0 — 门禁先行（Tier5 前半）

| 序 | 任务 | 仓库 | 可并行 |
|---|---|---|---|
| 0.1 | T5-Task1：check-deps.sh 补 chrome/settings/settings-ui 组 + CI 加 `--no-default-features` | bytehost | 与 0.2 并行 |
| 0.2 | T5-Task2：移植 digger 硬门禁 check-host-deps.sh | dozer | 与 0.1 并行 |

### Phase 1 — dozer 机械清理（Tier1 前半）

| 序 | 任务 | 仓库 | 依赖 |
|---|---|---|---|
| 1.1 | T1-Task1：bytehost tag 上收 `[workspace.dependencies]` | dozer | **必须先于 Phase 3** |
| 1.2 | T1-Task2：删 panel-files/panel-project 未用依赖 | dozer | 顺序随意 |
| 1.3 | T1-Task3：删 layout_compat.rs | dozer | 顺序随意 |

### Phase 2 — bytehost 批量重构（Tier1 后半 + Tier3 + Tier4，一个分支一个 tag）

| 序 | 任务 | 风险 | 可并行 |
|---|---|---|---|
| 2.1 | T4-Task0：建 gateway 基准，记 baseline | 无 | — |
| 2.2 | T1-Task4：文档修正（README/lib.rs） | 无 | 与 2.3-2.5 并行 |
| 2.3 | T3-Task2：wry 执行器抽公共 | 内部 | 与 2.6-2.8 并行（双人） |
| 2.4 | T3-Task4：自旋锁改 Notify | 内部 | 同上 |
| 2.5 | T4-Task1：Arc\<Site\>；T4-Task2：spawn_blocking 调查处置 | 内部 | 依赖 2.1 |
| 2.6 | T3-Task1：Core 锁模型（破坏 Deref） | **破坏 API** | — |
| 2.7 | T3-Task3：纯逻辑下沉（改 crate 边界） | **破坏边界** | — |
| 2.8 | T1-Task5：SurfaceEvent 重命名 | **破坏 API** | — |
| 2.9 | T3-Task5：thiserror（评审批准后，可跳过） | 风格 | — |
| 2.10 | **发版：一个新 tag（先问用户）** | — | 依赖 2.1-2.9 |

每项独立 commit，可单独 revert；低风险在前、破坏性在后。

### Phase 3 — 消费方同日升级

| 序 | 任务 | 依赖 |
|---|---|---|
| 3.1 | dozer 升新 tag + 更新 Deref/SurfaceEvent/panes 引用点 | 1.1、2.10 |
| 3.2 | digger 同日升级（ADR 约束） | 2.10 |

两仓可并行，但必须同日推送（先问用户）。

### Phase 4 — 消费侧去重（Tier2，可与 Phase 2/3 并行开工，升级后合入）

| 序 | 任务 | 仓库 |
|---|---|---|
| 4.1 | T2-Task1：overlay diff-webview 去重 | dozer |
| 4.2 | T2-Task2：SurfaceExecutor 上收 panel-host | dozer |
| 4.3 | T2-Task3：overlay 模板化试点 | dozer |
| 4.4 | T2-Task4：订阅合并 | digger |
| 4.5 | T2-Task5：port.rs 处置 | digger |
| 4.6 | T2-Task6：boot helper | digger |

dozer 线（4.1-4.3）与 digger 线（4.4-4.6）完全独立。若在 Phase 3 前开工，基于旧 tag 写、升级后 rebase。

### Phase 5 — 治理收尾（Tier5 后半）

| 序 | 任务 | 依赖 |
|---|---|---|
| 5.1 | T5-Task3：check-shell-regions.sh 进 CI | 核对脚本归属后定 |
| 5.2 | T5-Task4：联合构建 CI + 统一 pin 守卫 | Phase 0-3 各仓 CI 稳定 |
| 5.3 | T5-Task5：适配下沉首个试点 | 4.2（panel-host 上收模式已验证） |

## 关键路径

```
0.1/0.2 → 1.1 → 2.6-2.8（破坏性项）→ 2.10 发版 → 3.1/3.2 同日升级 → 5.2 联合 CI
```

## 必须问用户的节点

- 2.10：bytehost 发版（打 tag、推送）
- 3.1/3.2：dozer/digger 推送
- 5.2：改 CI 配置的合入
- 2.9：thiserror 是否引入（评审裁决）
- 4.5：port.rs 删除 vs 标注延后（裁决）

## 决策记录

2026-10-30 实施完成(Phase 0–4 + 5.1,5.2 本地形态,5.3 仅候选识别)。

- **bytehost 新 tag:`v0.4.18`(提交 `8cb8ed1`)**,已推送 main 与 tag。它同时带上了此前的
  `feat/shared-settings` + `feat/web-find` 5 个提交(dozer 已依赖 settings crate,v0.4.17 里没有)。
- **dozer**:分支 `feat/bytehost-opt`,基线 `origin/main`(`08b63c99`),已升 `v0.4.18`。
- **bytehost**:分支 `feat/bytehost-opt`,基线 `feat/shared-settings`(`bd68451`)——不是 `origin/main`:
  dozer 已消费的 settings crate 只在该分支上。
- **digger**:本地分支 `feat/bytehost-opt`,基线 `main`(`91a2a80`)。该仓没配 remote,只本地提交。
- **thiserror(2.9)**:跳过(未批准引入)。
- **port.rs(4.5)**:删除(YAGNI;设置页 Advanced 尚无端口重置入口)。
- **联合 CI(5.2)**:两消费方都没有 GitHub Actions,先落本地 `scripts/joint-build.sh`(bytehost);
  跨仓 workflow 等两仓有可复用 workflow + 跨仓 token 再接。
- **适配下沉(5.3)**:只出候选清单(见 Tier5 决策记录),不做具体下沉。
- **偏差**:Tier3 Task1 只移除 `Deref` 并把 `Core` 收为 `pub(crate)`,**未拆子锁**——install/start/
  runtime 都写同一批 app 记录与 gateway,按职责拆锁会让同一 app 的 install 与 start 并发写记录;
  安全粒度应是"按 app 锁",需单独立项。理由已记入 Tier3。
- **验证口径**:bytehost `cargo test --all-features`/clippy/fmt/`check-deps --self-test` 全绿;
  dozer 全仓测试除 `dozerd::product_isolation` 4 项(worktree 下找不到 `product_daemon` 示例二进制,
  环境限制)外全绿;digger `cargo test --workspace` 全绿(用本地 patch)。
