# bytehost H3:按面板展开的布局代码收口(`PanelDims` 访问器 + 分隔线拖拽合并)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消掉宿主布局代码里"每个面板写一段"的重复:(1) `PanelDims` 加 4 个按面板取字段的访问器(`split`/`split_mut`/`collapsed`/`collapsed_mut`),替换 `pair_split_ratio`、`with_pair_split_ratio`、`App::list_collapsed`、`App::toggle_panel_list_collapse` 里的 4 处 12 臂 `match`;(2) 把 `apply_column_drag` 里 10 条**除参数外逐字相同**的"面板分隔线"分支合并成一个 `apply_pair_split_drag`。生产代码约 494 行 → 约 230 行(含访问器),**不改任何用户可见行为**。这是 H0 汇总里 `PanelKind` 注册制分批的 **B2/B3 的一部分**(`docs/dozer-v2/bytehost-H0/01-panelkind.md` §6)——只做不依赖未决项 O5 的那一部分,`PanelKind` 仍是枚举,`PanelDims` 的字段名和落盘格式不变。

**Architecture:** 两个任务。Task 1 先写**特征化测试**:对 11 条分隔线(13 种面板视图组合)× 镜像/未镜像 × 是否从"已收起"出发 × 5 个光标位置共 230 个用例,黄金值由**重构前**的实现导出,在旧代码上通过后单独提交。Task 2 用 TDD 加访问器(先写访问器测试看它编译失败),再跑一个重构脚本一次性完成替换,黄金表必须一字不差;最后更新门禁基线并回填文档。

**Tech Stack:** Rust(`dozer-app`,iced 0.14);Python 3 标准库(一次性重构脚本 + 审计门禁)。不碰 `Cargo.toml`/`Cargo.lock`。

**Spec:** `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`(§5 迁移原则:先机制后面板、迁移期不改行为、先写刻画测试钉住现行口径);依据 `docs/dozer-v2/bytehost-H0/01-panelkind.md`(§2b、B2/B3)。

## Global Constraints

- **行为保持,而且是逐比特保持。** 拖拽分隔线的结果(`PanelDims` 的每个字段,含收起/展开的判定、镜像时的比例翻转)必须与重构前完全一致;特征化测试用 `f32` 的 `Debug` 输出比较,不是近似比较。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h3/...`),分支 `bytehost-h3`,从当前 `main` 开(H0–H2 都在 `main` 上)。主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]` 会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**;`.cargo/config.toml` 不提交;本计划不改依赖。
- **变异检验前先 `git add` 暂存当前版本**,变异后用 `git checkout -- <文件>` 还原到暂存版本(未暂存的已修改文件直接 `git checkout` 会把整个改动冲回原样)。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了。**
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`**(沿用 H1 的防线)。
- **不扩大范围:** 本计划**不动** `PanelDims` 的字段(落盘的 `panel_layouts.json` 靠字段名反序列化,改字段名/改成 map 是另一个有迁移成本的决定,属于 O5/B1)、**不动** `panel_meta`(图标+tooltip 表)、`fire_panel_switch_in`、`webview_geometry.rs` 的穷举 `match`(它们要面板钩子,H0 汇总 §4 候选 3/5)。
- 提交信息用 `refactor:`/`test:`/`chore(audit):`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败;`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在 linked worktree 里可能失败;`assets::tests::serves_vendored_asset_with_mime` 偶发失败。
- 草稿基线(`main` = H2 之后,linked worktree):`1732 passed; 1 failed`。草稿里 Task 1 之后 `1733 passed`(+1 个黄金表测试;导出用的那个测试带 `#[ignore]`),Task 2 之后 `1737 passed`(+4 个访问器测试),失败始终只有 `delete_confirm`。执行时以 Task 1 Step 2 实测为准。

## 分支合并后的等价关系(重构脚本依据)

10 条分支(`LeftPairSplit`/`ProjectSplit`/`SshSplit`/`DatabaseSplit`/`UsageSplit`/`CodeHealthSplit`/`GroupChatSplit`/`TodoSplit`/`GitLogSplit`/`BrowserBookmarksSplit`)逐条归一化(把面板名、`*_split`、`*_collapsed` 字段名、`list_rendered_first` 的字面量实参遮掉)后比较,**七条完全相同**,另外三条只差在:

| 分支 | 与模板的差别 | 合并后怎么表达 |
|---|---|---|
| `CodeHealthSplit`、`GitLogSplit` | 没有"拖窄就收起"那段,只写 split | `PanelDims::collapsed(kind)` 为 `None` 的面板不走收起判定 |
| `BrowserBookmarksSplit` | 没有收起;比例翻转的条件写法相反:`if list_rendered_first(false, m) { 1.0 - raw } else { raw }` | 算术上等价于模板取 `list_first = true`(`list_rendered_first(false, m)` 恒等于 `m`;模板是 `if list_rendered_first(lf, m) { raw } else { 1-raw }`,`lf = true` 时即 `if !m { raw } else { 1-raw }`),由 `default_list_first(Web) = true` 表达,**特征化测试钉住** |

每个面板的 `list_first`:`Files/Project/Ssh/Database/Todo/GitLog/Web` 为 `true`,`Agent/GroupChat/Conversations/Usage/CodeHealth` 为 `false`。`RightPairSplit` 是另一种形态(按 `state.right_view` 选 `Agent`/`Conversations`,`Usage` 原样返回 `state.dims`,其它 `unreachable!`),保留这个结构、只把 `Agent`/`Conversations` 两臂委托给同一个函数。`LeftRight` 不动。

## Review Focus

1. **黄金表没盖到的写法(草稿里真发生过)。** 第一版黄金表只从"未收起"出发:把 `*flag = false`(拖回够宽就展开)那一行删掉,黄金表**仍然通过**。所以表里加了"从已收起状态出发"这一维(230 个用例里 100 个、其中 72 个会展开)。Task 2 的变异检验第 2 条守着它。
2. **`default_list_first(Web)` 与 `Web` 的"反着命名"。** 见上表;变异检验第 1 条把它改成 `false`,黄金表必须失败。
3. **`App::list_collapsed(Files)` 必须仍是 `false`。** 旧 `match` 里 `Files` 落在 `_ => false`;新访问器 `collapsed(Files)` 返回 `Some(files_tree_collapsed)`,所以 `App::list_collapsed` 里显式写 `Files => false`,`toggle_panel_list_collapse` 里显式 `if kind == Files { return }`(`Files` 走独立的 `files_tree_collapsed` 按钮,见原注释)。**没有 `App` 构造夹具**(项目已知缺口),这两处只能靠代码审阅 + 访问器测试;审阅时对照 Step 4 的 diff 逐臂看。
4. **访问器与字段一一对应。** 12 个臂手写、最容易抄错字段(例如 `Todo` 臂写成 `project_*`)。Task 2 的访问器测试对每个面板断言"写 A 只改 A、读 A 读到 A、别的面板不受影响",变异检验第 3 条故意写错一个臂。
5. **非 macOS 代码路径从未在本机编译过。** 本计划改动的全是平台无关代码(`layout.rs` 的纯函数与 `update.rs` 的两个方法),Step 里的 `cfg` 检查应无命中。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/app/layout.rs` | 追加特征化测试模块;`PanelDims` 访问器;`default_list_first`;`apply_pair_split_drag`;`apply_column_drag` 瘦身;`pair_split_ratio`/`with_pair_split_ratio` 变一行 | 1、2 |
| `crates/dozer-app/src/app/update.rs` | `App::list_collapsed`/`toggle_panel_list_collapse` 走访问器 | 2 |
| `scripts/audit/panel-boundary.baseline.json` | `R-PANE-PICK` 的 `app/layout.rs` 条目清零(`!= PanelKind::Project.default_side()` 随分支合并消失) | 2 |
| `docs/dozer-v2/bytehost-H0/01-panelkind.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 | 2 |

一次性脚本(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: 特征化测试——在旧代码上钉住 `apply_column_drag`

**Files:**
- Modify: `crates/dozer-app/src/app/layout.rs`(文件末尾追加一个测试模块)

**Interfaces:**
- Produces(Task 2 依赖):`app::layout::drag_characterization_tests`,其中 `apply_column_drag_matches_the_golden_table` 对 230 个用例逐条比较;黄金值是**重构前**实现的输出。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h3 -b bytehost-h3 main
mkdir -p ../dozer-bytehost-h3/.cargo && cp .cargo/config.toml ../dozer-bytehost-h3/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h3-scratch && mkdir -p $SCRATCH
```

Expected: 干净、分支 `bytehost-h3`。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h3/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里列的;记下通过数 N(草稿:1732)。再记 clippy 基线:`RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git checkout -- Cargo.lock`(草稿:11 个文件有既有诊断)。

- [ ] **Step 3: 追加特征化测试模块**

把下面这段**原样追加**到 `crates/dozer-app/src/app/layout.rs` 文件末尾(前面空一行)。里面的 `GOLDEN` 是草稿里在**旧实现**上导出的 230 行黄金值;`collapse_it` 故意按字段手写,不依赖 Task 2 才有的访问器:

```rust
/// bytehost H3 特征化测试:`apply_column_drag` 对 11 条面板分隔线(13 种面板视图组合)在
/// 镜像/未镜像 × 4 个光标位置下的结果,逐条钉住(黄金值由重构前的实现导出)。
/// 重构(把每面板一段的分支收成一个函数)前后这张表必须一字不差。
#[cfg(test)]
mod drag_characterization_tests {
    use super::*;
    use crate::app::{PanelKind, ShellLayout, ShellState};

    const WINDOW_W: f32 = 1600.0;
    const XS: [f32; 5] = [0.0, 300.0, 700.0, 1000.0, 1500.0];

    /// 把 `kind` 的收起标志置位;没有收起能力的面板返回 false。**故意按字段手写**,不依赖被测的新访问器。
    fn collapse_it(dims: &mut PanelDims, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Files => dims.files_tree_collapsed = true,
            PanelKind::Project => dims.project_list_collapsed = true,
            PanelKind::Todo => dims.todo_list_collapsed = true,
            PanelKind::Database => dims.database_list_collapsed = true,
            PanelKind::Ssh => dims.ssh_list_collapsed = true,
            PanelKind::Agent => dims.agent_list_collapsed = true,
            PanelKind::Conversations => dims.conversations_list_collapsed = true,
            PanelKind::GroupChat => dims.group_chat_list_collapsed = true,
            PanelKind::Usage => dims.usage_list_collapsed = true,
            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth => return false,
        }
        true
    }

    fn cases() -> Vec<(Divider, PanelKind)> {
        vec![
            (Divider::LeftPairSplit, PanelKind::Files),
            (Divider::ProjectSplit, PanelKind::Project),
            (Divider::SshSplit, PanelKind::Ssh),
            (Divider::TodoSplit, PanelKind::Todo),
            (Divider::GitLogSplit, PanelKind::GitLog),
            (Divider::BrowserBookmarksSplit, PanelKind::Web),
            (Divider::DatabaseSplit, PanelKind::Database),
            (Divider::UsageSplit, PanelKind::Usage),
            (Divider::CodeHealthSplit, PanelKind::CodeHealth),
            (Divider::GroupChatSplit, PanelKind::GroupChat),
            (Divider::RightPairSplit, PanelKind::Agent),
            (Divider::RightPairSplit, PanelKind::Conversations),
            (Divider::RightPairSplit, PanelKind::Usage),
        ]
    }

    fn state_for(kind: PanelKind, mirrored: bool) -> ShellState {
        let mut layout = ShellLayout::default();
        if mirrored {
            // 把 kind 挪到它默认栏的对面
            let rail = &mut layout.rail_layout;
            rail.left.retain(|k| *k != kind);
            rail.right.retain(|k| *k != kind);
            match kind.default_side() {
                Side::Left => rail.right.push(kind),
                Side::Right => rail.left.push(kind),
            }
        }
        ShellState {
            layout,
            dims: PanelDims::default(),
            left_view: PanelKind::Files,
            left_collapsed: false,
            right_view: if kind.default_side() == Side::Right {
                kind
            } else {
                PanelKind::Agent
            },
            right_collapsed: false,
            browser_bookmarks_open: false,
            maximized: None,
        }
    }

    /// 与输入 `dims` 相比变了哪些字段,`name=value;` 串起来。
    fn changed(before: &PanelDims, after: &PanelDims) -> String {
        let b = format!("{before:?}");
        let a = format!("{after:?}");
        let tokens = |s: &str| -> Vec<String> {
            s.trim_start_matches("PanelDims { ")
                .trim_end_matches(" }")
                .split(", ")
                .map(str::to_string)
                .collect()
        };
        let mut out = Vec::new();
        for (x, y) in tokens(&b).into_iter().zip(tokens(&a)) {
            if x != y {
                out.push(y.replace(": ", "="));
            }
        }
        if out.is_empty() {
            "-".to_string()
        } else {
            out.join(";")
        }
    }

    fn actual() -> Vec<String> {
        let mut rows = Vec::new();
        for (divider, kind) in cases() {
            for mirrored in [false, true] {
                // 第二维:从"已收起"状态出发(只对有收起能力的面板)——钉住"拖回够宽就展开"。
                for start_collapsed in [false, true] {
                    for x in XS {
                        let mut state = state_for(kind, mirrored);
                        if start_collapsed && !collapse_it(&mut state.dims, kind) {
                            continue;
                        }
                        let before = state.dims;
                        let after = apply_column_drag(state, divider, WINDOW_W, x);
                        rows.push(format!(
                            "{divider:?}/{kind:?}|mirrored={mirrored}|start_collapsed={start_collapsed}|x={x}|{}",
                            changed(&before, &after)
                        ));
                    }
                }
            }
        }
        rows
    }

    #[test]
    #[ignore = "导出黄金值用:cargo test -p dozer-app drag_characterization_dump -- --ignored --nocapture"]
    fn drag_characterization_dump() {
        for r in actual() {
            println!("GOLDEN {r}");
        }
    }

    #[test]
    fn apply_column_drag_matches_the_golden_table() {
        let golden: Vec<&str> = GOLDEN.lines().collect();
        let got = actual();
        assert_eq!(got.len(), golden.len(), "用例数变了");
        for (g, a) in golden.iter().zip(got.iter()) {
            assert_eq!(g, a);
        }
    }

    const GOLDEN: &str = r#"LeftPairSplit/Files|mirrored=false|start_collapsed=false|x=0|files_tree_collapsed=true
LeftPairSplit/Files|mirrored=false|start_collapsed=false|x=300|files_split=0.4050633
LeftPairSplit/Files|mirrored=false|start_collapsed=false|x=700|files_split=0.8
LeftPairSplit/Files|mirrored=false|start_collapsed=false|x=1000|files_split=0.8
LeftPairSplit/Files|mirrored=false|start_collapsed=false|x=1500|files_split=0.8
LeftPairSplit/Files|mirrored=false|start_collapsed=true|x=0|-
LeftPairSplit/Files|mirrored=false|start_collapsed=true|x=300|files_split=0.4050633;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=false|start_collapsed=true|x=700|files_split=0.8;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=false|start_collapsed=true|x=1000|files_split=0.8;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=false|start_collapsed=true|x=1500|files_split=0.8;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=true|start_collapsed=false|x=0|files_split=0.8
LeftPairSplit/Files|mirrored=true|start_collapsed=false|x=300|files_split=0.8
LeftPairSplit/Files|mirrored=true|start_collapsed=false|x=700|files_split=0.8
LeftPairSplit/Files|mirrored=true|start_collapsed=false|x=1000|files_split=0.6401869
LeftPairSplit/Files|mirrored=true|start_collapsed=false|x=1500|files_tree_collapsed=true
LeftPairSplit/Files|mirrored=true|start_collapsed=true|x=0|files_split=0.8;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=true|start_collapsed=true|x=300|files_split=0.8;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=true|start_collapsed=true|x=700|files_split=0.8;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=true|start_collapsed=true|x=1000|files_split=0.6401869;files_tree_collapsed=false
LeftPairSplit/Files|mirrored=true|start_collapsed=true|x=1500|-
ProjectSplit/Project|mirrored=false|start_collapsed=false|x=0|project_list_collapsed=true
ProjectSplit/Project|mirrored=false|start_collapsed=false|x=300|project_split=0.4050633
ProjectSplit/Project|mirrored=false|start_collapsed=false|x=700|project_split=0.8
ProjectSplit/Project|mirrored=false|start_collapsed=false|x=1000|project_split=0.8
ProjectSplit/Project|mirrored=false|start_collapsed=false|x=1500|project_split=0.8
ProjectSplit/Project|mirrored=false|start_collapsed=true|x=0|-
ProjectSplit/Project|mirrored=false|start_collapsed=true|x=300|project_list_collapsed=false;project_split=0.4050633
ProjectSplit/Project|mirrored=false|start_collapsed=true|x=700|project_list_collapsed=false;project_split=0.8
ProjectSplit/Project|mirrored=false|start_collapsed=true|x=1000|project_list_collapsed=false;project_split=0.8
ProjectSplit/Project|mirrored=false|start_collapsed=true|x=1500|project_list_collapsed=false;project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=false|x=0|project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=false|x=300|project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=false|x=700|project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=false|x=1000|project_split=0.6401869
ProjectSplit/Project|mirrored=true|start_collapsed=false|x=1500|project_list_collapsed=true
ProjectSplit/Project|mirrored=true|start_collapsed=true|x=0|project_list_collapsed=false;project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=true|x=300|project_list_collapsed=false;project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=true|x=700|project_list_collapsed=false;project_split=0.8
ProjectSplit/Project|mirrored=true|start_collapsed=true|x=1000|project_list_collapsed=false;project_split=0.6401869
ProjectSplit/Project|mirrored=true|start_collapsed=true|x=1500|-
SshSplit/Ssh|mirrored=false|start_collapsed=false|x=0|ssh_list_collapsed=true
SshSplit/Ssh|mirrored=false|start_collapsed=false|x=300|ssh_split=0.4050633
SshSplit/Ssh|mirrored=false|start_collapsed=false|x=700|ssh_split=0.8
SshSplit/Ssh|mirrored=false|start_collapsed=false|x=1000|ssh_split=0.8
SshSplit/Ssh|mirrored=false|start_collapsed=false|x=1500|ssh_split=0.8
SshSplit/Ssh|mirrored=false|start_collapsed=true|x=0|-
SshSplit/Ssh|mirrored=false|start_collapsed=true|x=300|ssh_list_collapsed=false;ssh_split=0.4050633
SshSplit/Ssh|mirrored=false|start_collapsed=true|x=700|ssh_list_collapsed=false;ssh_split=0.8
SshSplit/Ssh|mirrored=false|start_collapsed=true|x=1000|ssh_list_collapsed=false;ssh_split=0.8
SshSplit/Ssh|mirrored=false|start_collapsed=true|x=1500|ssh_list_collapsed=false;ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=false|x=0|ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=false|x=300|ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=false|x=700|ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=false|x=1000|ssh_split=0.6401869
SshSplit/Ssh|mirrored=true|start_collapsed=false|x=1500|ssh_list_collapsed=true
SshSplit/Ssh|mirrored=true|start_collapsed=true|x=0|ssh_list_collapsed=false;ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=true|x=300|ssh_list_collapsed=false;ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=true|x=700|ssh_list_collapsed=false;ssh_split=0.8
SshSplit/Ssh|mirrored=true|start_collapsed=true|x=1000|ssh_list_collapsed=false;ssh_split=0.6401869
SshSplit/Ssh|mirrored=true|start_collapsed=true|x=1500|-
TodoSplit/Todo|mirrored=false|start_collapsed=false|x=0|todo_list_collapsed=true
TodoSplit/Todo|mirrored=false|start_collapsed=false|x=300|todo_split=0.4050633
TodoSplit/Todo|mirrored=false|start_collapsed=false|x=700|todo_split=0.8
TodoSplit/Todo|mirrored=false|start_collapsed=false|x=1000|todo_split=0.8
TodoSplit/Todo|mirrored=false|start_collapsed=false|x=1500|todo_split=0.8
TodoSplit/Todo|mirrored=false|start_collapsed=true|x=0|-
TodoSplit/Todo|mirrored=false|start_collapsed=true|x=300|todo_list_collapsed=false;todo_split=0.4050633
TodoSplit/Todo|mirrored=false|start_collapsed=true|x=700|todo_list_collapsed=false;todo_split=0.8
TodoSplit/Todo|mirrored=false|start_collapsed=true|x=1000|todo_list_collapsed=false;todo_split=0.8
TodoSplit/Todo|mirrored=false|start_collapsed=true|x=1500|todo_list_collapsed=false;todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=false|x=0|todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=false|x=300|todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=false|x=700|todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=false|x=1000|todo_split=0.6401869
TodoSplit/Todo|mirrored=true|start_collapsed=false|x=1500|todo_list_collapsed=true
TodoSplit/Todo|mirrored=true|start_collapsed=true|x=0|todo_list_collapsed=false;todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=true|x=300|todo_list_collapsed=false;todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=true|x=700|todo_list_collapsed=false;todo_split=0.8
TodoSplit/Todo|mirrored=true|start_collapsed=true|x=1000|todo_list_collapsed=false;todo_split=0.6401869
TodoSplit/Todo|mirrored=true|start_collapsed=true|x=1500|-
GitLogSplit/GitLog|mirrored=false|start_collapsed=false|x=0|git_log_split=0.2
GitLogSplit/GitLog|mirrored=false|start_collapsed=false|x=300|git_log_split=0.4050633
GitLogSplit/GitLog|mirrored=false|start_collapsed=false|x=700|git_log_split=0.8
GitLogSplit/GitLog|mirrored=false|start_collapsed=false|x=1000|git_log_split=0.8
GitLogSplit/GitLog|mirrored=false|start_collapsed=false|x=1500|git_log_split=0.8
GitLogSplit/GitLog|mirrored=true|start_collapsed=false|x=0|git_log_split=0.8
GitLogSplit/GitLog|mirrored=true|start_collapsed=false|x=300|git_log_split=0.8
GitLogSplit/GitLog|mirrored=true|start_collapsed=false|x=700|git_log_split=0.8
GitLogSplit/GitLog|mirrored=true|start_collapsed=false|x=1000|git_log_split=0.6401869
GitLogSplit/GitLog|mirrored=true|start_collapsed=false|x=1500|git_log_split=0.19999999
BrowserBookmarksSplit/Web|mirrored=false|start_collapsed=false|x=0|browser_bookmarks_split=0.2
BrowserBookmarksSplit/Web|mirrored=false|start_collapsed=false|x=300|browser_bookmarks_split=0.4050633
BrowserBookmarksSplit/Web|mirrored=false|start_collapsed=false|x=700|browser_bookmarks_split=0.8
BrowserBookmarksSplit/Web|mirrored=false|start_collapsed=false|x=1000|browser_bookmarks_split=0.8
BrowserBookmarksSplit/Web|mirrored=false|start_collapsed=false|x=1500|browser_bookmarks_split=0.8
BrowserBookmarksSplit/Web|mirrored=true|start_collapsed=false|x=0|browser_bookmarks_split=0.8
BrowserBookmarksSplit/Web|mirrored=true|start_collapsed=false|x=300|browser_bookmarks_split=0.8
BrowserBookmarksSplit/Web|mirrored=true|start_collapsed=false|x=700|browser_bookmarks_split=0.8
BrowserBookmarksSplit/Web|mirrored=true|start_collapsed=false|x=1000|browser_bookmarks_split=0.6401869
BrowserBookmarksSplit/Web|mirrored=true|start_collapsed=false|x=1500|browser_bookmarks_split=0.19999999
DatabaseSplit/Database|mirrored=false|start_collapsed=false|x=0|database_list_collapsed=true
DatabaseSplit/Database|mirrored=false|start_collapsed=false|x=300|database_split=0.4050633
DatabaseSplit/Database|mirrored=false|start_collapsed=false|x=700|database_split=0.8
DatabaseSplit/Database|mirrored=false|start_collapsed=false|x=1000|database_split=0.8
DatabaseSplit/Database|mirrored=false|start_collapsed=false|x=1500|database_split=0.8
DatabaseSplit/Database|mirrored=false|start_collapsed=true|x=0|-
DatabaseSplit/Database|mirrored=false|start_collapsed=true|x=300|database_list_collapsed=false;database_split=0.4050633
DatabaseSplit/Database|mirrored=false|start_collapsed=true|x=700|database_list_collapsed=false;database_split=0.8
DatabaseSplit/Database|mirrored=false|start_collapsed=true|x=1000|database_list_collapsed=false;database_split=0.8
DatabaseSplit/Database|mirrored=false|start_collapsed=true|x=1500|database_list_collapsed=false;database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=false|x=0|database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=false|x=300|database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=false|x=700|database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=false|x=1000|database_split=0.6401869
DatabaseSplit/Database|mirrored=true|start_collapsed=false|x=1500|database_list_collapsed=true
DatabaseSplit/Database|mirrored=true|start_collapsed=true|x=0|database_list_collapsed=false;database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=true|x=300|database_list_collapsed=false;database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=true|x=700|database_list_collapsed=false;database_split=0.8
DatabaseSplit/Database|mirrored=true|start_collapsed=true|x=1000|database_list_collapsed=false;database_split=0.6401869
DatabaseSplit/Database|mirrored=true|start_collapsed=true|x=1500|-
UsageSplit/Usage|mirrored=false|start_collapsed=false|x=0|usage_split=0.8
UsageSplit/Usage|mirrored=false|start_collapsed=false|x=300|usage_split=0.8
UsageSplit/Usage|mirrored=false|start_collapsed=false|x=700|usage_split=0.8
UsageSplit/Usage|mirrored=false|start_collapsed=false|x=1000|usage_split=0.6401869
UsageSplit/Usage|mirrored=false|start_collapsed=false|x=1500|usage_list_collapsed=true
UsageSplit/Usage|mirrored=false|start_collapsed=true|x=0|usage_list_collapsed=false;usage_split=0.8
UsageSplit/Usage|mirrored=false|start_collapsed=true|x=300|usage_list_collapsed=false;usage_split=0.8
UsageSplit/Usage|mirrored=false|start_collapsed=true|x=700|usage_list_collapsed=false;usage_split=0.8
UsageSplit/Usage|mirrored=false|start_collapsed=true|x=1000|usage_list_collapsed=false;usage_split=0.6401869
UsageSplit/Usage|mirrored=false|start_collapsed=true|x=1500|-
UsageSplit/Usage|mirrored=true|start_collapsed=false|x=0|usage_list_collapsed=true
UsageSplit/Usage|mirrored=true|start_collapsed=false|x=300|usage_split=0.4050633
UsageSplit/Usage|mirrored=true|start_collapsed=false|x=700|usage_split=0.8
UsageSplit/Usage|mirrored=true|start_collapsed=false|x=1000|usage_split=0.8
UsageSplit/Usage|mirrored=true|start_collapsed=false|x=1500|usage_split=0.8
UsageSplit/Usage|mirrored=true|start_collapsed=true|x=0|-
UsageSplit/Usage|mirrored=true|start_collapsed=true|x=300|usage_list_collapsed=false;usage_split=0.4050633
UsageSplit/Usage|mirrored=true|start_collapsed=true|x=700|usage_list_collapsed=false;usage_split=0.8
UsageSplit/Usage|mirrored=true|start_collapsed=true|x=1000|usage_list_collapsed=false;usage_split=0.8
UsageSplit/Usage|mirrored=true|start_collapsed=true|x=1500|usage_list_collapsed=false;usage_split=0.8
CodeHealthSplit/CodeHealth|mirrored=false|start_collapsed=false|x=0|codehealth_split=0.8
CodeHealthSplit/CodeHealth|mirrored=false|start_collapsed=false|x=300|codehealth_split=0.8
CodeHealthSplit/CodeHealth|mirrored=false|start_collapsed=false|x=700|codehealth_split=0.8
CodeHealthSplit/CodeHealth|mirrored=false|start_collapsed=false|x=1000|codehealth_split=0.6401869
CodeHealthSplit/CodeHealth|mirrored=false|start_collapsed=false|x=1500|codehealth_split=0.19999999
CodeHealthSplit/CodeHealth|mirrored=true|start_collapsed=false|x=0|codehealth_split=0.2
CodeHealthSplit/CodeHealth|mirrored=true|start_collapsed=false|x=300|codehealth_split=0.4050633
CodeHealthSplit/CodeHealth|mirrored=true|start_collapsed=false|x=700|codehealth_split=0.8
CodeHealthSplit/CodeHealth|mirrored=true|start_collapsed=false|x=1000|codehealth_split=0.8
CodeHealthSplit/CodeHealth|mirrored=true|start_collapsed=false|x=1500|codehealth_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=false|x=0|group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=false|x=300|group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=false|x=700|group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=false|x=1000|group_chat_split=0.6401869
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=false|x=1500|group_chat_list_collapsed=true
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=true|x=0|group_chat_list_collapsed=false;group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=true|x=300|group_chat_list_collapsed=false;group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=true|x=700|group_chat_list_collapsed=false;group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=true|x=1000|group_chat_list_collapsed=false;group_chat_split=0.6401869
GroupChatSplit/GroupChat|mirrored=false|start_collapsed=true|x=1500|-
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=false|x=0|group_chat_list_collapsed=true
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=false|x=300|group_chat_split=0.4050633
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=false|x=700|group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=false|x=1000|group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=false|x=1500|group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=true|x=0|-
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=true|x=300|group_chat_list_collapsed=false;group_chat_split=0.4050633
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=true|x=700|group_chat_list_collapsed=false;group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=true|x=1000|group_chat_list_collapsed=false;group_chat_split=0.8
GroupChatSplit/GroupChat|mirrored=true|start_collapsed=true|x=1500|group_chat_list_collapsed=false;group_chat_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=false|x=0|agent_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=false|x=300|agent_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=false|x=700|agent_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=false|x=1000|agent_split=0.6401869
RightPairSplit/Agent|mirrored=false|start_collapsed=false|x=1500|agent_list_collapsed=true
RightPairSplit/Agent|mirrored=false|start_collapsed=true|x=0|agent_list_collapsed=false;agent_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=true|x=300|agent_list_collapsed=false;agent_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=true|x=700|agent_list_collapsed=false;agent_split=0.8
RightPairSplit/Agent|mirrored=false|start_collapsed=true|x=1000|agent_list_collapsed=false;agent_split=0.6401869
RightPairSplit/Agent|mirrored=false|start_collapsed=true|x=1500|-
RightPairSplit/Agent|mirrored=true|start_collapsed=false|x=0|agent_list_collapsed=true
RightPairSplit/Agent|mirrored=true|start_collapsed=false|x=300|agent_split=0.4050633
RightPairSplit/Agent|mirrored=true|start_collapsed=false|x=700|agent_split=0.8
RightPairSplit/Agent|mirrored=true|start_collapsed=false|x=1000|agent_split=0.8
RightPairSplit/Agent|mirrored=true|start_collapsed=false|x=1500|agent_split=0.8
RightPairSplit/Agent|mirrored=true|start_collapsed=true|x=0|-
RightPairSplit/Agent|mirrored=true|start_collapsed=true|x=300|agent_list_collapsed=false;agent_split=0.4050633
RightPairSplit/Agent|mirrored=true|start_collapsed=true|x=700|agent_list_collapsed=false;agent_split=0.8
RightPairSplit/Agent|mirrored=true|start_collapsed=true|x=1000|agent_list_collapsed=false;agent_split=0.8
RightPairSplit/Agent|mirrored=true|start_collapsed=true|x=1500|agent_list_collapsed=false;agent_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=false|x=0|conversations_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=false|x=300|conversations_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=false|x=700|conversations_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=false|x=1000|conversations_split=0.6401869
RightPairSplit/Conversations|mirrored=false|start_collapsed=false|x=1500|conversations_list_collapsed=true
RightPairSplit/Conversations|mirrored=false|start_collapsed=true|x=0|conversations_list_collapsed=false;conversations_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=true|x=300|conversations_list_collapsed=false;conversations_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=true|x=700|conversations_list_collapsed=false;conversations_split=0.8
RightPairSplit/Conversations|mirrored=false|start_collapsed=true|x=1000|conversations_list_collapsed=false;conversations_split=0.6401869
RightPairSplit/Conversations|mirrored=false|start_collapsed=true|x=1500|-
RightPairSplit/Conversations|mirrored=true|start_collapsed=false|x=0|conversations_list_collapsed=true
RightPairSplit/Conversations|mirrored=true|start_collapsed=false|x=300|conversations_split=0.4050633
RightPairSplit/Conversations|mirrored=true|start_collapsed=false|x=700|conversations_split=0.8
RightPairSplit/Conversations|mirrored=true|start_collapsed=false|x=1000|conversations_split=0.8
RightPairSplit/Conversations|mirrored=true|start_collapsed=false|x=1500|conversations_split=0.8
RightPairSplit/Conversations|mirrored=true|start_collapsed=true|x=0|-
RightPairSplit/Conversations|mirrored=true|start_collapsed=true|x=300|conversations_list_collapsed=false;conversations_split=0.4050633
RightPairSplit/Conversations|mirrored=true|start_collapsed=true|x=700|conversations_list_collapsed=false;conversations_split=0.8
RightPairSplit/Conversations|mirrored=true|start_collapsed=true|x=1000|conversations_list_collapsed=false;conversations_split=0.8
RightPairSplit/Conversations|mirrored=true|start_collapsed=true|x=1500|conversations_list_collapsed=false;conversations_split=0.8
RightPairSplit/Usage|mirrored=false|start_collapsed=false|x=0|-
RightPairSplit/Usage|mirrored=false|start_collapsed=false|x=300|-
RightPairSplit/Usage|mirrored=false|start_collapsed=false|x=700|-
RightPairSplit/Usage|mirrored=false|start_collapsed=false|x=1000|-
RightPairSplit/Usage|mirrored=false|start_collapsed=false|x=1500|-
RightPairSplit/Usage|mirrored=false|start_collapsed=true|x=0|-
RightPairSplit/Usage|mirrored=false|start_collapsed=true|x=300|-
RightPairSplit/Usage|mirrored=false|start_collapsed=true|x=700|-
RightPairSplit/Usage|mirrored=false|start_collapsed=true|x=1000|-
RightPairSplit/Usage|mirrored=false|start_collapsed=true|x=1500|-
RightPairSplit/Usage|mirrored=true|start_collapsed=false|x=0|-
RightPairSplit/Usage|mirrored=true|start_collapsed=false|x=300|-
RightPairSplit/Usage|mirrored=true|start_collapsed=false|x=700|-
RightPairSplit/Usage|mirrored=true|start_collapsed=false|x=1000|-
RightPairSplit/Usage|mirrored=true|start_collapsed=false|x=1500|-
RightPairSplit/Usage|mirrored=true|start_collapsed=true|x=0|-
RightPairSplit/Usage|mirrored=true|start_collapsed=true|x=300|-
RightPairSplit/Usage|mirrored=true|start_collapsed=true|x=700|-
RightPairSplit/Usage|mirrored=true|start_collapsed=true|x=1000|-
RightPairSplit/Usage|mirrored=true|start_collapsed=true|x=1500|-"#;
}
```

- [ ] **Step 4: 格式化,在旧代码上跑,必须通过**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3
cargo fmt -p dozer-app && git status --short
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app drag_characterization 2>&1 | grep -E "^error|^test |test result"
git checkout -- Cargo.lock
```
Expected: `git status --short` 只有 `layout.rs`;`apply_column_drag_matches_the_golden_table ... ok`,`drag_characterization_dump ... ignored`,`test result: ok. 1 passed … 1 ignored`。**这是特征化测试:它在旧代码上必须通过**——若失败,说明 `main` 上 `apply_column_drag` 已经和草稿不同(有人改过),**停下**,用下面的命令在**旧代码上**重新导出黄金值再继续(不要改断言迎合):
`cargo test -p dozer-app drag_characterization_dump -- --ignored --nocapture 2>&1 | grep '^GOLDEN ' | sed 's/^GOLDEN //' > $SCRATCH/golden.txt`,用它替换 `GOLDEN` 常量的内容。

- [ ] **Step 5: 黄金表覆盖度自检**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && python3 - <<'EOF'
import re
s = open("crates/dozer-app/src/app/layout.rs").read()
g = s[s.index('const GOLDEN: &str = r#"') + len('const GOLDEN: &str = r#"'):]
g = g[:g.index('"#;')].strip().split("\n")
print("cases:", len(g))
print("start_collapsed=true:", sum("start_collapsed=true" in r for r in g))
print("…of which expand (a *_collapsed=false result):", sum("start_collapsed=true" in r and re.search(r"collapsed=false", r) is not None for r in g))
print("mirrored collapse:", sum("mirrored=true" in r and "collapsed=true" in r.split("|")[-1] for r in g))
EOF`
Expected: `cases: 230`、`start_collapsed=true: 100`、`expand: 72`、`mirrored collapse` ≥ 9(草稿值)。数字明显偏小说明用例没覆盖到收起/展开路径,先查原因。

- [ ] **Step 6: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3
git status --short
git add crates/dozer-app/src/app/layout.rs
git diff --cached --stat | tail -3
git commit -m "test(layout): characterize apply_column_drag for all pair-split dividers

230 cases (11 dividers x mirrored x started-collapsed x 5 cursor positions), golden values
exported from the pre-refactor implementation. Pins the behavior before the per-panel
branches are merged.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 只有 `layout.rs` 一个文件。

---

### Task 2: `PanelDims` 访问器 + 分隔线拖拽合并

**Files:**
- Modify: `crates/dozer-app/src/app/layout.rs`、`crates/dozer-app/src/app/update.rs`
- Modify: `scripts/audit/panel-boundary.baseline.json`、`docs/dozer-v2/bytehost-H0/01-panelkind.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1 的黄金表。
- Produces:`PanelDims::{split(&self, PanelKind) -> f32, split_mut(&mut self, PanelKind) -> &mut f32, collapsed(&self, PanelKind) -> Option<bool>, collapsed_mut(&mut self, PanelKind) -> Option<&mut bool>}`(`pub(crate)`);`fn default_list_first(PanelKind) -> bool`、`fn apply_pair_split_drag(&ShellState, PanelKind, f32, f32) -> PanelDims`(模块私有)。`pair_split_ratio`/`with_pair_split_ratio` 签名不变。

- [ ] **Step 1: 写失败的访问器测试**

把下面这段**原样追加**到 `crates/dozer-app/src/app/layout.rs` 末尾(前面空一行):

```rust
/// bytehost H3:`PanelDims` 按面板取字段的访问器。
#[cfg(test)]
mod panel_dims_accessor_tests {
    use super::*;
    use crate::app::PanelKind;

    const ALL: [PanelKind; 12] = [
        PanelKind::Files,
        PanelKind::GitLog,
        PanelKind::Todo,
        PanelKind::Project,
        PanelKind::Database,
        PanelKind::Ssh,
        PanelKind::Web,
        PanelKind::Agent,
        PanelKind::GroupChat,
        PanelKind::Conversations,
        PanelKind::Usage,
        PanelKind::CodeHealth,
    ];

    #[test]
    fn split_mut_writes_exactly_the_field_split_reads() {
        for (i, kind) in ALL.into_iter().enumerate() {
            let mut dims = PanelDims::default();
            let marker = 0.123 + i as f32 * 0.01;
            *dims.split_mut(kind) = marker;
            assert_eq!(dims.split(kind), marker, "{kind:?}");
            // 别的面板的 split 一个都不能被碰
            for other in ALL.into_iter().filter(|k| *k != kind) {
                assert_ne!(dims.split(other), marker, "{kind:?} 写进了 {other:?}");
            }
        }
    }

    #[test]
    fn exactly_nine_panels_can_collapse_and_three_cannot() {
        let can: Vec<_> = ALL
            .into_iter()
            .filter(|k| PanelDims::default().collapsed(*k).is_some())
            .collect();
        assert_eq!(can.len(), 9);
        for kind in [PanelKind::GitLog, PanelKind::Web, PanelKind::CodeHealth] {
            assert_eq!(PanelDims::default().collapsed(kind), None, "{kind:?}");
            assert!(
                PanelDims::default().collapsed_mut(kind).is_none(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn collapsed_mut_writes_exactly_the_field_collapsed_reads() {
        for kind in ALL {
            let mut dims = PanelDims::default();
            let Some(flag) = dims.collapsed_mut(kind) else {
                continue;
            };
            *flag = true;
            assert_eq!(dims.collapsed(kind), Some(true), "{kind:?}");
            for other in ALL.into_iter().filter(|k| *k != kind) {
                assert_ne!(
                    dims.collapsed(other),
                    Some(true),
                    "{kind:?} 写进了 {other:?}"
                );
            }
        }
    }

    #[test]
    fn files_collapse_flag_is_the_tree_collapsed_field() {
        let mut dims = PanelDims::default();
        *dims.collapsed_mut(PanelKind::Files).unwrap() = true;
        assert!(dims.files_tree_collapsed);
    }
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app panel_dims_accessor 2>&1 | grep -E "^error" -A3 | head -8; git checkout -- Cargo.lock`
Expected: 编译失败,`no method named \`split_mut\` found for struct \`PanelDims\``(`split`、`collapsed`、`collapsed_mut` 同)。

- [ ] **Step 3: 重构脚本**

创建 `$SCRATCH/h3_refactor.py`:

```python
#!/usr/bin/env python3
"""H3 一次性重构脚本:把 `apply_column_drag` 里 10 条几乎一样的"面板分割线"分支收成一个函数,
并给 `PanelDims` 加按面板取字段的访问器(替换 4 处 12 臂 match)。**不提交。** 在仓库根运行。"""
import re

SRC = "crates/dozer-app/src/"
p = SRC + "app/layout.rs"
s = open(p).read()

# 1) PanelDims 访问器(插在 `impl Default for PanelDims` 之前)
accessors = '''impl PanelDims {
    /// `kind` 面板配对的分割比例(统一口径是"pair 内第一个 slot 的占比",见 `pair_split_ratio`)。
    pub(crate) fn split(&self, kind: PanelKind) -> f32 {
        match kind {
            PanelKind::Files => self.files_split,
            PanelKind::Project => self.project_split,
            PanelKind::Ssh => self.ssh_split,
            PanelKind::Database => self.database_split,
            PanelKind::Todo => self.todo_split,
            PanelKind::GitLog => self.git_log_split,
            PanelKind::Web => self.browser_bookmarks_split,
            PanelKind::Agent => self.agent_split,
            PanelKind::GroupChat => self.group_chat_split,
            PanelKind::Conversations => self.conversations_split,
            PanelKind::Usage => self.usage_split,
            PanelKind::CodeHealth => self.codehealth_split,
        }
    }

    pub(crate) fn split_mut(&mut self, kind: PanelKind) -> &mut f32 {
        match kind {
            PanelKind::Files => &mut self.files_split,
            PanelKind::Project => &mut self.project_split,
            PanelKind::Ssh => &mut self.ssh_split,
            PanelKind::Database => &mut self.database_split,
            PanelKind::Todo => &mut self.todo_split,
            PanelKind::GitLog => &mut self.git_log_split,
            PanelKind::Web => &mut self.browser_bookmarks_split,
            PanelKind::Agent => &mut self.agent_split,
            PanelKind::GroupChat => &mut self.group_chat_split,
            PanelKind::Conversations => &mut self.conversations_split,
            PanelKind::Usage => &mut self.usage_split,
            PanelKind::CodeHealth => &mut self.codehealth_split,
        }
    }

    /// `kind` 面板列表列的"是否收起"标志;没有收起能力的面板(`GitLog`/`Web`/`CodeHealth`)是 `None`。
    /// `Files` 的标志是 `files_tree_collapsed`(语义同其余面板的 `*_list_collapsed`)。
    pub(crate) fn collapsed(&self, kind: PanelKind) -> Option<bool> {
        match kind {
            PanelKind::Files => Some(self.files_tree_collapsed),
            PanelKind::Project => Some(self.project_list_collapsed),
            PanelKind::Todo => Some(self.todo_list_collapsed),
            PanelKind::Database => Some(self.database_list_collapsed),
            PanelKind::Ssh => Some(self.ssh_list_collapsed),
            PanelKind::Agent => Some(self.agent_list_collapsed),
            PanelKind::Conversations => Some(self.conversations_list_collapsed),
            PanelKind::GroupChat => Some(self.group_chat_list_collapsed),
            PanelKind::Usage => Some(self.usage_list_collapsed),
            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth => None,
        }
    }

    pub(crate) fn collapsed_mut(&mut self, kind: PanelKind) -> Option<&mut bool> {
        match kind {
            PanelKind::Files => Some(&mut self.files_tree_collapsed),
            PanelKind::Project => Some(&mut self.project_list_collapsed),
            PanelKind::Todo => Some(&mut self.todo_list_collapsed),
            PanelKind::Database => Some(&mut self.database_list_collapsed),
            PanelKind::Ssh => Some(&mut self.ssh_list_collapsed),
            PanelKind::Agent => Some(&mut self.agent_list_collapsed),
            PanelKind::Conversations => Some(&mut self.conversations_list_collapsed),
            PanelKind::GroupChat => Some(&mut self.group_chat_list_collapsed),
            PanelKind::Usage => Some(&mut self.usage_list_collapsed),
            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth => None,
        }
    }
}

'''
a = "impl Default for PanelDims {"
assert a in s
s = s.replace(a, accessors + a, 1)

# 2) pair_split_ratio / with_pair_split_ratio 改走访问器
i = s.index("pub(crate) fn pair_split_ratio(")
j = s.index("/// 拖拽某条分隔线到窗口逻辑 x 坐标")
s = s[:i] + '''pub(crate) fn pair_split_ratio(dims: &PanelDims, kind: PanelKind) -> Option<f32> {
    Some(dims.split(kind))
}

/// `pair_split_ratio` 的写入侧。
pub(crate) fn with_pair_split_ratio(dims: PanelDims, kind: PanelKind, ratio: f32) -> PanelDims {
    let mut dims = dims;
    *dims.split_mut(kind) = ratio;
    dims
}

/// 每个面板默认(未镜像)态下"列表侧是否渲染在前(pair 内第一个元素)"。`Web` 的
/// `browser_bookmarks_split` 虽然语义反着命名(内容占比),算法上它的 pair 第一个元素
/// 就是"split 字段那一侧",所以按 `true` 处理——由特征化测试钉住。
fn default_list_first(kind: PanelKind) -> bool {
    match kind {
        PanelKind::Files
        | PanelKind::Project
        | PanelKind::Ssh
        | PanelKind::Database
        | PanelKind::Todo
        | PanelKind::GitLog
        | PanelKind::Web => true,
        PanelKind::Agent
        | PanelKind::GroupChat
        | PanelKind::Conversations
        | PanelKind::Usage
        | PanelKind::CodeHealth => false,
    }
}

/// 拖拽 `kind` 面板的配对分割线到窗口逻辑 x 坐标 `logical_x` 后的新 `PanelDims`
/// (`apply_column_drag` 里所有"面板分割线"分支的共用实现)。
///
/// 有收起能力的面板(`PanelDims::collapsed(kind)` 为 `Some`):拖窄到列表列宽度小于
/// `project::footer_min_width`(所有可收纳面板统一走这一份估算,不各面板各算一套)时直接收起
/// ——冻结 `split` 里上一次仍够宽的比例,不让它被拖成挤爆按钮的小数值;拖回超过阈值时用当前
/// 光标位置连续算出新比例并展开,对称、可逆(拖拽是逐帧调用本函数,不是一次性判定)。
/// 没有收起能力的面板只写 `split`。
fn apply_pair_split_drag(
    state: &ShellState,
    kind: PanelKind,
    window_width: f32,
    logical_x: f32,
) -> PanelDims {
    let side = state.layout.rail_layout.side_of(kind);
    let (x0, pair_w) = pair_x0_and_width(side, window_width, state);
    if pair_w <= 0.0 {
        return state.dims;
    }
    let raw_ratio = ((logical_x - x0) / pair_w).clamp(
        byteui::theme::geometry::min_split_ratio(),
        byteui::theme::geometry::max_split_ratio(),
    );
    let mirrored = side != kind.default_side();
    let ratio = if list_rendered_first(default_list_first(kind), mirrored) {
        raw_ratio
    } else {
        1.0 - raw_ratio
    };
    let mut dims = state.dims;
    if let Some(flag) = dims.collapsed_mut(kind) {
        let (list_w, _) = pair_list_content_width(pair_w, ratio);
        if list_w < project::footer_min_width() {
            *flag = true;
            return dims;
        }
        *flag = false;
    }
    *dims.split_mut(kind) = ratio;
    dims
}

''' + s[j:]

# 3) apply_column_drag:10 条分支 -> 调用共用函数
i = s.index("        Divider::LeftPairSplit => {")
tail = s.index("\n    }\n}\n", s.index("        Divider::RightPairSplit => {"))
new_arms = '''        Divider::LeftPairSplit => apply_pair_split_drag(&state, PanelKind::Files, window_width, logical_x),
        Divider::ProjectSplit => apply_pair_split_drag(&state, PanelKind::Project, window_width, logical_x),
        Divider::SshSplit => apply_pair_split_drag(&state, PanelKind::Ssh, window_width, logical_x),
        Divider::DatabaseSplit => apply_pair_split_drag(&state, PanelKind::Database, window_width, logical_x),
        Divider::UsageSplit => apply_pair_split_drag(&state, PanelKind::Usage, window_width, logical_x),
        Divider::CodeHealthSplit => apply_pair_split_drag(&state, PanelKind::CodeHealth, window_width, logical_x),
        Divider::GroupChatSplit => apply_pair_split_drag(&state, PanelKind::GroupChat, window_width, logical_x),
        Divider::TodoSplit => apply_pair_split_drag(&state, PanelKind::Todo, window_width, logical_x),
        Divider::GitLogSplit => apply_pair_split_drag(&state, PanelKind::GitLog, window_width, logical_x),
        Divider::BrowserBookmarksSplit => apply_pair_split_drag(&state, PanelKind::Web, window_width, logical_x),
        // 右栏配对:写哪个 split 取决于右侧当前视图(Agent / Conversations;Usage 有自己的
        // `UsageSplit` 分割线,这里不动它)。
        Divider::RightPairSplit => match state.right_view {
            kind @ (PanelKind::Agent | PanelKind::Conversations) => {
                apply_pair_split_drag(&state, kind, window_width, logical_x)
            }
            PanelKind::Usage => state.dims,
            _ => unreachable!(
                "RightPairSplit 只会在 state.right_view 是 Agent/Conversations/\\
                 Usage 之一时出现——Stage 1 遗留的兜底,这里维持"
            ),
        },'''
s = s[:i] + new_arms + s[tail:]
open(p, "w").write(s)

# 4) App::list_collapsed / toggle_panel_list_collapse 走访问器(Files 仍排除:它走独立的 files_tree_collapsed 按钮)
p = SRC + "app/update.rs"
s = open(p).read()
i = s.index("    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {")
j = s.index("    /// 翻转某两栏面板列表列的展开/收起")
s = s[:i] + '''    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Files => false,
            _ => self.dims.collapsed(kind).unwrap_or(false),
        }
    }

''' + s[j:]
i = s.index("        let flag = match kind {\n            PanelKind::Project => &mut self.dims.project_list_collapsed,")
j = s.index("        *flag = !*flag;", i)
s = s[:i] + '''        if kind == PanelKind::Files {
            return;
        }
        let Some(flag) = self.dims.collapsed_mut(kind) else {
            return;
        };
''' + s[j:]
open(p, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3
python3 $SCRATCH/h3_refactor.py && cargo fmt -p dozer-app && git status --short
```
Expected: `git status --short` 只有 `app/layout.rs`、`app/update.rs`。脚本里每个替换点都先 `assert` 原文存在,原文不在会直接报错而不是静默跳过——若报 `AssertionError`,说明 `main` 上这几处已被改动,按报错点对照 Task 1 前后的 `git diff main` 手工对齐后再继续,**不要**改断言。

- [ ] **Step 4: 逐臂审阅 `App` 两个方法的 diff(没有夹具可测)**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && git diff -- crates/dozer-app/src/app/update.rs`
Expected: `list_collapsed`:`Files => false`,其余 `self.dims.collapsed(kind).unwrap_or(false)`——对照旧 `match`:`Project/Todo/Database/Ssh/Agent/Conversations/GroupChat/Usage` 读各自的 `*_list_collapsed`,`_ => false`(含 `Files/GitLog/Web/CodeHealth`);`toggle_panel_list_collapse`:`Files` 直接 `return`,没有收起能力的面板(`collapsed_mut` 为 `None`)也 `return`,其余翻转对应标志,**后面的 `self.maximized = None; self.on_shell_layout_changed();` 保持不变**。逐条核对 `collapsed()` 的 9 个臂与旧 `match` 的字段对应关系(`Files` 臂是 `files_tree_collapsed`,在 `App` 层被显式排除)。

- [ ] **Step 5: 编译、全量测试、clippy**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A8 | head -30
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
```
Expected: build 无输出;**`apply_column_drag_matches_the_golden_table` 仍然通过**(它是唯一的行为裁判);测试通过数 = Task 1 之后的数 + 4,失败只有已知项;`clippy: no new diagnostics`。

- [ ] **Step 6: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && git add crates
# 变异 1: Web 的 list_first 改成 false —— 黄金表必须失败
python3 - <<'EOF'
p = "crates/dozer-app/src/app/layout.rs"
s = open(p).read()
a = "        | PanelKind::GitLog\n        | PanelKind::Web => true,"
assert a in s
open(p, "w").write(s.replace(a, "        | PanelKind::GitLog => true,\n        PanelKind::Web => false,", 1))
EOF
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app apply_column_drag_matches_the_golden_table 2>&1 | grep -E "^error|^test .*golden|test result"
git checkout -- crates/dozer-app/src/app/layout.rs Cargo.lock

# 变异 2: 删掉"拖回够宽就展开"那一行 —— 黄金表必须失败
python3 - <<'EOF'
p = "crates/dozer-app/src/app/layout.rs"
s = open(p).read()
a = "        *flag = false;\n    }\n    *dims.split_mut(kind) = ratio;"
assert a in s
open(p, "w").write(s.replace(a, "    }\n    *dims.split_mut(kind) = ratio;", 1))
EOF
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app apply_column_drag_matches_the_golden_table 2>&1 | grep -E "^test .*golden|test result"
git checkout -- crates/dozer-app/src/app/layout.rs Cargo.lock

# 变异 3: Todo 的收起标志误读成 Project 的 —— 访问器测试必须失败
python3 - <<'EOF'
p = "crates/dozer-app/src/app/layout.rs"
s = open(p).read()
a = "            PanelKind::Todo => Some(self.todo_list_collapsed),"
assert a in s
open(p, "w").write(s.replace(a, "            PanelKind::Todo => Some(self.project_list_collapsed),", 1))
EOF
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app panel_dims_accessor 2>&1 | grep -E "^test .*FAILED|test result"
git checkout -- crates/dozer-app/src/app/layout.rs Cargo.lock; git status --short crates | wc -l; git diff --stat | wc -l
```
Expected: 三次变异都以 `FAILED` 结束(若变异 1 的 `python3` 报 `AssertionError`,说明 `default_list_first` 的排版与草稿不同,手工把 `Web` 那一臂改成 `false` 再测);每次还原后 `git diff --stat | wc -l` 为 `0`(暂存的重构版本被恢复)。

- [ ] **Step 7: 更新门禁基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && export PYTHONDONTWRITEBYTECODE=1 && python3 scripts/audit/check_panel_boundary.py; python3 scripts/audit/check_panel_boundary.py --update; git diff scripts/audit/panel-boundary.baseline.json | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)"; python3 scripts/audit/check_panel_boundary.py`
Expected: 第一次检查对着旧基线仍 `ok`(`app/layout.rs` 的 `R-PANE-PICK` 从 1 降到 0 是"下降",允许);`--update` 后 diff 只有 `app/layout.rs` 那 3 行被删(`"app/layout.rs": { "R-PANE-PICK": 1 },`);最后一次检查 `ok (27 refs in 5 files)`。

- [ ] **Step 8: 回填文档**

每条先 `grep -n` 找到原文再改,改完 `git diff` 看一遍:
1. `docs/dozer-v2/bytehost-H0/01-panelkind.md` §6 表里 B2 一行与 B3 一行末尾各追加:"**部分完成(H3):** `PanelDims` 的 4 处 12 臂 `match`(`pair_split_ratio`/`with_pair_split_ratio`/`App::list_collapsed`/`toggle_panel_list_collapse`)走 `PanelDims::{split,split_mut,collapsed,collapsed_mut}`;`apply_column_drag` 里 10 条面板分隔线分支合并为 `apply_pair_split_drag`(约 494 → 约 230 行,含访问器);`panel_meta`、`fire_panel_switch_in`、`webview_geometry.rs` 的穷举 match 仍在(需面板钩子)。"
2. `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7:在 H2 那条之后追加"**bytehost H3 已完成(2026-10-04):** 宿主布局里按面板展开的重复收口——`PanelDims` 访问器替换 4 处 12 臂 match,`apply_column_drag` 的 10 条面板分隔线分支合并为一个函数(230 个用例的特征化测试钉住行为);`PanelDims` 字段与落盘格式未动,`PanelKind` 注册制(B1)与面板钩子(B4)留给后续切片。"

- [ ] **Step 9: 最终校验并提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h3 && export PYTHONDONTWRITEBYTECODE=1
python3 scripts/audit/test_edges.py 2>&1 | tail -1; python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1; python3 scripts/audit/check_panel_boundary.py
git diff -U10 -- crates | grep -nE "cfg\((not\()?target_os" | head -3
git status --short
git add crates/dozer-app/src/app/layout.rs crates/dozer-app/src/app/update.rs scripts/audit/panel-boundary.baseline.json docs/dozer-v2
git diff --cached --stat | tail -8
git commit -m "refactor(layout): PanelDims accessors and one shared pair-split drag

Four 12-arm per-panel matches become PanelDims::{split,split_mut,collapsed,collapsed_mut};
the ten near-identical divider branches in apply_column_drag collapse into
apply_pair_split_drag (behavior pinned by the 230-case characterization table).

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 两组脚本测试 `OK`、门禁 `ok`;`cfg` 检查无输出;`git diff --cached --stat` 里只有 `layout.rs`、`update.rs`、基线 json、两份文档(没有 `Cargo.lock`、`.cargo/`)。

---

## Self-Review

**1. 覆盖:** B2/B3 里不依赖未决项的部分——`PanelDims` 的按面板字段访问(4 处 match)与 `apply_column_drag` 的 10 条重复分支——全部有对应步骤;`panel_meta`/`fire_panel_switch_in`/`webview_geometry` 明确列为不在范围(需面板钩子)。

**2. 占位符扫描:** 测试、重构脚本、黄金值均为草稿里跑通的完整版本;文档回填给出要写入的文字。

**3. 一致性:** 访问器名(`split`/`split_mut`/`collapsed`/`collapsed_mut`)在测试、重构脚本、`update.rs` 改写里一致;`default_list_first` 的取值表与"分支合并后的等价关系"一节一致;黄金表 230 = 13 种组合 × 镜像 2 × 起始收起(2,无收起能力的面板只有 1)× 5 个位置的实际展开。

**4. Review Focus:** 5 条各有归属(1→Task 1 Step 5 与 Task 2 变异 2、2→变异 1、3→Task 2 Step 4、4→变异 3 与访问器测试、5→Step 9 的 `cfg` 检查)。
