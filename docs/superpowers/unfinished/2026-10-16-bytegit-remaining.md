# bytegit 剩余未尽事项

整理日期：2026-10-16。bytegit 当前 `v0.8.1`，dozer 已升到该版本；digger 仍锁在 `v0.8.0`（经 dozer libs tag `libs-2026-10-16c`）。

权威来源：`docs/superpowers/specs/2026-10-02-bytegit-design.md` §8（O1–O16）与 O9 口径清单。本文只收**尚未做完**的项，已完成的见该规格。

迁移 P0–P6 已全部完成；`v0.8.0` 补了 O3（`.git` 为文件的监听）、O15（过滤 Access 事件）、O16①②（checkout 参数/clone 不卡凭据提示）；`v0.8.1` 修了 O11（路径字面匹配）；D14 状态类已统一。

## 1. 值得排期但暂不紧急

### D14 历史类：统一「只认仓库根 vs 向上查找」
- **现状：** 状态类（`project_git_snapshot`、分支选择器的“是否有改动”、`git_snapshot`/`head_short_sha`、`files::git_status::file_statuses`）已改为向上查找。历史类仍用 `Repo::open_exact`：`file_history`、`git_log`、`diff_content`（经 `diff_content::open_exact_repo`）的历史、blob、工作区内容、回滚、上一版本。
- **后果：** 项目目录在仓库子目录里时，状态面板能取到数据，历史面板仍报错。测试 `build_in_a_repo_subdirectory_is_an_error_not_a_wrong_history`、`commit_detail_in_a_repo_subdirectory_is_an_error` 固定着当前行为。
- **要做的事：** 把 `file_path` 换算成相对仓库根（`file_statuses` 里的 `project_prefix` 可复用），再把 `open_exact_repo` 换成 `Repo::discover`；`bytegit` 与 API 不用动。风险点：`log` 的 pathspec 相对仓库根，换算漏了会悄悄给出错误历史；回滚是写操作，路径错了后果比读取严重。
- **触发条件：** 项目在仓库子目录里的用法出现真实反馈，或 Digger / v2 引入 worktree 与子目录项目。
- **注意：** “是否有改动”现在是整个仓库的，不只项目子树；若产品上要“仅项目子树”，需要另给 `StatusOptions` 加路径过滤。

## 2. 等触发条件再做

| 项 | 内容 | 触发条件 |
|---|---|---|
| O14 / O5 | `churn`、`commit_count*` 全历史扫描，超大仓库上比命令行 `git log --since` 慢（libgit2 时间排序遍历先解析整段可达历史）；`log` 的 `max_count` 同类 | 有实测的慢仓库。优化前先量，不要凭感觉 |
| O7 | `gleisbau` 与 `bytegit` 都要求 `git2 = "0.21"` 且无默认 feature，cargo 统一成同一份；升级 `git2` 要两处一起看。若 `gleisbau` 成阻碍，再评估自写提交图（规格 B1） | 升级 `git2` 时 |
| O8 | Digger 是否需要提交图；需要则单独设计 `graph` feature | Digger 提出需求时 |
| O3 余项 | `watch` 的 gitdir 解析只在启动时做一次，`.git` 文件被改成指向别处不会重新接线；`.git` 文件本身的变化不上报 | 出现“切 worktree 指向”的真实场景时 |
| O15 余项 | 只过滤了 `Access` 事件；非 macOS 平台从未实际编译过 | 非 mac 平台发布前，整体验一遍 `watch` |

## 3. 已评估，决定不做（记录原因，避免重复讨论）

| 项 | 结论 |
|---|---|
| O12 | P2 里三处错误文案由英文改中文，视为已接受的用户可见变化；要改回英文另议 |
| O13 | `churn` 对提交时间做全局排序，比旧的 `git log --since`（遇到第一个过旧提交就停）结果多，属有意差异，更符合“近 30 天”字面含义 |
| O16③ | `init` 不再生成 14 个 `.sample` 示例 hook（只剩 `README.sample`），纯外观差异，保持原样 |
| `clone`/`checkout_branch` 换 `git2` | 评估后的有意选择，见 `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`；换之前先让 `bytegit/src/write.rs` 里钉死的测试（post-checkout hook、外部过滤器）通过 |

## 4. 需要顺手处理的小事

- **digger 同步：** digger 仍用 bytegit `v0.8.0`。同步步骤：dozer 打新 libs tag（如 `libs-2026-10-16d`），digger 把四处 dozer 依赖指过去，`cargo update`，`cargo test --workspace`。digger 工作树在分支 `chore/bytegit-0.8.0`（基于 `chore/align-deps-2026-10`，提交 `eb858d0`），**digger 仓库没有配置任何远程**，该分支未推送；需先确定 digger 的远程地址。
- **规格 §4 / §6 的 O9 口径清单：** 状态类已迁到 `discover`，清单里“`Repo::discover`”一行尚未补全这几个调用点（`open_exact` 一行已注明）。下次改 D14 历史类时一并整理。
- **plans 目录里 bytegit P0–P6 的 checkbox 全是未勾选**，但对应工作都已完成，不能当进度指标。
