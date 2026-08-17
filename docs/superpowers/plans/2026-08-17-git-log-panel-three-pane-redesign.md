# Git Log 面板三栏重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Git Log 面板从"Canvas 自绘分支拓扑图 + 固定宽详情栏"重构成三栏
可拖拽布局:左侧 commit 线性列表(+ 分支切换下拉),右上文件列表,右下选中
文件的染色 diff 内容。

**Architecture:** `git_log` 扩展模块(自己的 `Message`/`State`/`update`/`view`)
内部新增"选中文件""分支切换"两块状态,渲染从单块 Canvas 换成两条可拖拽分割线
拼出的三栏布局。左右分割复用现有 `PanelDims`/`Divider`/`apply_column_drag`
横向拖拽机制新开一个变体;右侧上下分割是仓库里第一条纵向分割线,需要平行新写
一套 `RowDivider`/`apply_row_drag`/`Message::RowDrag*` 机制,`main.rs` 的
`CursorMoved` 处理里追加对应分支。分支切换真实调用 `delivery::checkout_branch`,
UI 视觉照抄 `files.rs::branch_picker_popup` 但状态完全独立(不读 `files::
WorkspaceState`)。

**Tech Stack:** Rust, iced 0.14(`iced_widget`/`iced_widget::canvas` 移除,不再用
Canvas),`git2`,`gleisbau`(继续复用,只取部分字段)。

**Spec:** `docs/superpowers/specs/2026-08-17-git-log-panel-three-pane-redesign-design.md`

## Global Constraints

- GUI 只用 iced 0.14 生态,不引入新 crate 依赖。
- 主题色:bg `#0a0e16`、金 `#F2D94E`(**甲方动作专属**,不能用在纯展示态上,
  比如"这是普通提交/合并提交"这类分类信息不能用金色)、奶油文字
  `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。
- 新增/改造 icon 按钮、tab 类 UI 时优先复用统一组件
  (`icons::icon_button_entry`/`tabs::tab_core`);本计划里的 commit
  行/文件行是"可选中列表行"而非"icon 按钮"或"tab",不套用这两个组件,
  照抄 Todo/Files 面板现有"整行 `MouseArea` + 左侧金色竖条选中态"的既有
  范式(理由已在 spec 里说明)。
- 分支切换是真实 `git checkout`(会改写工作区文件),不是"仅查看视角切换"。
- `git_log` 模块不读 `Workspace`/`files::WorkspaceState` 的任何字段,只能
  通过自己的 `Message` 与内核(`app.rs`)交互——"扩展间不耦合,只消息通信"
  硬性原则(2026-08-07 Git Log 扩展化试点定下)。
- 每个任务完成后跑:`cargo build -p dozer-app && cargo test -p dozer-app
  --bin dozer <相关测试模块>:: -- --nocapture`,全部任务完成后跑一次完整
  `cargo build --workspace && cargo test -p dozer-app --bin dozer && cargo
  clippy -p dozer-app --all-targets && cargo fmt --check`。
- **本计划需要在独立 git worktree/分支上执行,完成后经代码审阅再合并
  main**——不要直接在 main 工作区上跑(`superpowers:using-git-worktrees`)。

---

## 文件结构总览

| 文件 | 改动 |
|---|---|
| `crates/dozer-app/src/extensions/git_log.rs` | 主战场:`CommitRow`/`State`/`Message` 扩字段,`GitLogCanvas` 删除换成新的列表/文件列表/diff/分支下拉渲染函数,`view()` 重写拼三栏布局。 |
| `crates/dozer-app/src/extensions/acceptance.rs` | `diff_view` 里的逐行染色循环抽成共享函数(Task 4),原地改成调用共享函数。 |
| `crates/dozer-app/src/theme.rs`(或新建 `crates/dozer-app/src/diff_render.rs`,Task 4 里定) | 新增共享的 diff 逐行染色渲染函数。 |
| `crates/dozer-app/src/icons.rs` | 新增 `GitCommitVertical`/`GitMerge` 两个 `IconKind` 变体。 |
| `crates/dozer-app/assets/icons/git-commit-vertical.svg`、`git-merge.svg` | 新增 vendored 图标资源(从 lucide.dev 下载)。 |
| `crates/dozer-app/src/app.rs` | `PanelDims` 加两个字段,`Divider` 加 `GitLogSplit` 变体,新增 `RowDivider` 枚举 + `Message::RowDragStart/RowDrag/RowDragEnd` + `apply_row_drag` 纯函数,`App` 加 `dragging_row: Option<RowDivider>` 字段,`Message::GitLog` 分发里新增 `BranchPickerOpen`(首次)/`BranchSwitch` 两种内核截获分支,新增纵向 `divider_bar` 姊妹函数。 |
| `crates/dozer-app/src/main.rs` | `CursorMoved` 处理里追加纵向拖拽分支(镜像现有横向 `ColumnDrag` 那段)。 |

---

## Task 1: `CommitRow` 新增 `time`/`is_merge` 字段

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs:66-81`(`CommitRow` 定义)、`137-230`(`build()`)
- Test: `crates/dozer-app/src/extensions/git_log.rs`(`#[cfg(test)] mod tests`,追加在现有 `build_*` 测试群旁边)

**Interfaces:**
- Produces: `CommitRow::time: i64`(Unix 秒,author time)、`CommitRow::is_merge: bool`。后续任务(Task 2/6)读这两个新字段。
- Consumes: `git2::Commit::time() -> git2::Time`(已有依赖,`git_commit` 变量已在 `build()` 里存在,`git2::Time::seconds() -> i64`)。

- [ ] **Step 1: 写失败的测试**

在 `git_log.rs` 测试模块里追加(紧跟 `build_marks_head_branch_and_labels` 之后):

```rust
#[test]
fn build_populates_time_and_is_merge() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/dozer-app 应有两层上级目录到仓库根");
    let snapshot = build(repo_root, DEFAULT_MAX_COMMITS).expect("gleisbau 应能解析 dozer 自己的仓库");

    // 每一行的 time 都应该是合理的正数(Unix 秒,仓库不可能早于 2020 年)。
    let epoch_2020 = 1_577_836_800_i64;
    for row in &snapshot.rows {
        assert!(row.time > epoch_2020, "commit time 应晚于 2020-01-01: {}", row.time);
    }

    // Dozer 仓库历史里确实有过 merge(如 2429d15),is_merge 应该跟
    // parents.len() >= 2 一致。
    let has_merge_row = snapshot.rows.iter().any(|r| r.is_merge);
    assert!(has_merge_row, "200 个 commit 窗口内应能看到至少一个 is_merge=true 的行");
    for row in &snapshot.rows {
        assert_eq!(row.is_merge, row.parents.len() >= 2, "is_merge 应与 parents.len()>=2 一致");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer git_log::tests::build_populates_time_and_is_merge -- --nocapture`
Expected: 编译失败(`CommitRow` 没有 `time`/`is_merge` 字段)。

- [ ] **Step 3: 实现**

`CommitRow` 定义(`git_log.rs:66-81`)追加两个字段:

```rust
#[derive(Debug, Clone)]
pub struct CommitRow {
    column: usize,
    color_idx: usize,
    short_sha: String,
    summary: String,
    parents: Vec<(usize, usize, usize)>,
    refs: Vec<RefLabel>,
    oid: git2::Oid,
    /// author time,Unix 秒——commit 列表行展示用(见 `format_commit_time`,Task 2)。
    time: i64,
    /// `parents.len() >= 2`(注意这是原始 git parent 数,不是 `parents` 字段
    /// 那个已经按 `max_count` 窗口过滤过的 `Vec`——根提交/单亲提交的行数一定
    /// 一致,只有"父提交恰好被窗口截断掉"的边界情形两者可能不同,这里用真实
    /// git parent 数,不用过滤后的 `parents.len()`,保证语义是"这个 commit
    /// 本身是不是合并提交",跟窗口大小无关)。
    is_merge: bool,
}
```

`build()`(`git_log.rs:137-230`)在 `.map(|commit| { ... })` 闭包里,`let parents = ...` 之后、`Ok(CommitRow { ... })` 之前追加:

```rust
let time = git_commit.time().seconds();
let is_merge = git_commit.parent_count() >= 2;
```

并在返回的 `CommitRow { .. }` 字面量里加上 `time, is_merge,`:

```rust
Ok(CommitRow {
    column,
    color_idx,
    short_sha,
    summary,
    parents,
    refs,
    oid: commit.oid,
    time,
    is_merge,
})
```

（测试里同时用到的 `snapshot_at()` 测试 helper 在 `git_log.rs` 底部,构造的是
`GitLogSnapshot` 不是 `CommitRow`,不受影响,不用改。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer git_log::tests:: -- --nocapture`
Expected: 全部 PASS,包括新的 `build_populates_time_and_is_merge`。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): CommitRow 新增 time/is_merge 字段"
```

---

## Task 2: `format_commit_time` 时间格式化辅助函数

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`(新增函数,放在 `status_glyph` 函数旁边,`867-876` 行附近)
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `CommitRow::time: i64`(Task 1 产出)。
- Produces: `format_commit_time(unix_secs: i64) -> String`,格式 `YYYY-MM-DD HH:MM:SS`(本地时区)。Task 6(commit 列表渲染)调用它。

先确认项目现有时间格式化惯例——搜 `chrono` 是否已在 `Cargo.toml` 依赖里:

```bash
grep -n "^chrono" crates/dozer-app/Cargo.toml
```

若已有 `chrono` 依赖(项目里对话列表/Todo 面板等大概率已经用了时间戳展示),
直接用它;若没有,用标准库 `std::time::UNIX_EPOCH` + 手工换算(不新增依赖,
遵守 Global Constraints"不引入新 crate 依赖")。以下实现假设未使用 `chrono`
(标准库方案),若跑上面命令发现已有 `chrono` 依赖,改用 `chrono::
DateTime::from_timestamp(unix_secs, 0)` 换算,格式化字符串不变。

- [ ] **Step 1: 写失败的测试**

```rust
#[test]
fn format_commit_time_matches_expected_layout() {
    // 2026-08-17 10:22:31 UTC 的 Unix 秒数(用 `date -u -d
    // "2026-08-17T10:22:31" +%s` 或等价方式核算得到:1786958551)。
    // 本地时区渲染,所以这里不断言具体的年月日时分秒——只断言格式形状
    // (长度、分隔符位置),避免测试环境时区不同导致断言写死失败。
    let formatted = format_commit_time(1_786_958_551);
    assert_eq!(formatted.len(), 19, "格式应为 YYYY-MM-DD HH:MM:SS,固定 19 字符: {formatted}");
    assert_eq!(&formatted[4..5], "-");
    assert_eq!(&formatted[7..5 + 5], "-");
    assert_eq!(&formatted[10..11], " ");
    assert_eq!(&formatted[13..14], ":");
    assert_eq!(&formatted[16..17], ":");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer git_log::tests::format_commit_time_matches_expected_layout -- --nocapture`
Expected: 编译失败(函数不存在)。

- [ ] **Step 3: 实现**(标准库方案,无 `chrono` 依赖时用这版)

```rust
/// commit 时间戳格式化,`YYYY-MM-DD HH:MM:SS`(本地时区)。不引入 `chrono`
/// ——用标准库手工做民用历换算(Gregorian calendar,不处理儒略历/闰秒,
/// commit 时间戳的应用场景不需要那种精度)。
fn format_commit_time(unix_secs: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let Some(system_time) = UNIX_EPOCH.checked_add(Duration::from_secs(unix_secs.max(0) as u64))
    else {
        return "—".to_string();
    };
    // `humantime`/`time` crate 都没被本项目依赖,用系统本地时区展示时走
    // `chrono::Local`——若 Step 0 核实项目已有 `chrono` 依赖,删掉本函数体,
    // 换成:
    //   let dt = chrono::DateTime::<chrono::Local>::from(system_time);
    //   dt.format("%Y-%m-%d %H:%M:%S").to_string()
    // 下面是无 chrono 依赖时的手工 UTC 换算兜底(不做本地时区转换,直接展示
    // UTC——跟"不引入新依赖"的约束平衡;若产品上必须本地时区,则必须引入
    // chrono 或等价 crate,写计划执行者需在此处按 Step 0 的核实结果二选一,
    // 不能两边都不做)。
    let secs_since_epoch = system_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs_since_epoch.div_euclid(86_400);
    let secs_of_day = secs_since_epoch.rem_euclid(86_400);
    let (h, m, s) = (secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// Howard Hinnant 的 `civil_from_days` 算法(公开域算法,常见于无 chrono 依赖
/// 场景的日期换算实现):Unix epoch 起的天数 → (年, 月, 日)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer git_log::tests:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): 新增 format_commit_time 时间戳格式化"
```

---

## Task 3: 新增 `git-commit-vertical`/`git-merge` 图标

**Files:**
- Create: `crates/dozer-app/assets/icons/git-commit-vertical.svg`、`crates/dozer-app/assets/icons/git-merge.svg`
- Modify: `crates/dozer-app/src/icons.rs:70`(`IconKind` 枚举,`GitBranch` 变体旁边)、`:127`(同上,`GitGraph` 旁边)、`:179`(`bytes()` match,`GitBranch` 分支旁边)、`:202`(同上,`GitGraph` 分支旁边)

**Interfaces:**
- Produces: `icons::IconKind::GitCommitVertical`、`icons::IconKind::GitMerge`。Task 6(commit 列表渲染)消费。

- [ ] **Step 1: 下载图标资源**

从 Lucide 图标库(`https://lucide.dev/icons/git-commit-vertical`、
`https://lucide.dev/icons/git-merge`,均已在 brainstorming 阶段核实为真实
存在的图标)下载对应 SVG,与现有 `crates/dozer-app/assets/icons/git-branch.svg`
同规格(Lucide 默认 24x24 viewBox、`stroke="currentColor"`)。存到:

```
crates/dozer-app/assets/icons/git-commit-vertical.svg
crates/dozer-app/assets/icons/git-merge.svg
```

用 `diff crates/dozer-app/assets/icons/git-branch.svg <新文件>` 粗查一下
`<svg>` 根元素的属性形状是否一致(`viewBox`/`fill="none"`/`stroke-width`
等),避免下载到不同版本/不同参数的 SVG 导致渲染尺寸跟其它图标不一致。

- [ ] **Step 2: 注册 `IconKind` 变体**

`icons.rs:70` 附近(`GitBranch,` 那一行旁边)追加:

```rust
GitCommitVertical,
```

`icons.rs:127` 附近(`GitGraph,` 那一行旁边)追加:

```rust
GitMerge,
```

（两个变体放在各自紧邻的位置,不强求相邻——`icons.rs` 现有排布本身
`GitBranch`/`GitGraph` 就不在一起,跟随现状,不做额外整理。）

- [ ] **Step 3: 注册 `bytes()` 分支**

`icons.rs:179` 附近:

```rust
IconKind::GitCommitVertical => include_bytes!("../assets/icons/git-commit-vertical.svg"),
```

`icons.rs:202` 附近:

```rust
IconKind::GitMerge => include_bytes!("../assets/icons/git-merge.svg"),
```

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 编译通过(无 `include_bytes!` 路径错误,说明两个 svg 文件已正确
落盘且注册无误)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/assets/icons/git-commit-vertical.svg \
        crates/dozer-app/assets/icons/git-merge.svg \
        crates/dozer-app/src/icons.rs
git commit -m "feat(icons): 新增 git-commit-vertical/git-merge 图标"
```

---

## Task 4: 抽取共享的 diff 逐行染色渲染函数

**Files:**
- Create: `crates/dozer-app/src/diff_render.rs`
- Modify: `crates/dozer-app/src/main.rs`(或 `lib.rs`,加 `mod diff_render;`——先跑 `grep -n "^mod \|^pub mod " crates/dozer-app/src/main.rs` 确认现有模块声明放在 `main.rs` 里,按字母序插入)、`crates/dozer-app/src/extensions/acceptance.rs:491-529`(`diff_view` 改成调用共享函数)
- Test: `crates/dozer-app/src/diff_render.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `diff_render::colored_diff_lines(patch: &str) -> Element<'static, M, iced_widget::Theme, iced_renderer::Renderer>`(泛型消息类型 `M`,纯渲染不产生任何消息,`M` 不需要任何 trait bound——iced 的 `Element<'a, Message, ..>` 对纯展示内容不要求 `Message: Clone` 等,若编译报错要求 `M: 'static`,按报错提示加最小 bound)。Task 8(git_log 的 diff 面板)、Task（原 acceptance.rs 现有调用点)都消费它。
- Consumes: 无新依赖,纯 `patch: &str` 输入。

- [ ] **Step 1: 写失败的测试**

新建 `crates/dozer-app/src/diff_render.rs`:

```rust
//! diff 文本的逐行染色渲染——`+`/`-`/上下文三种颜色,等宽字体,`TERM_BG`
//! 背景。原先只有 `extensions/acceptance.rs::diff_view` 一份实现,Git Log
//! 面板重构(2026-08-17)需要同一套渲染,抽成共享函数避免重复(见
//! `docs/superpowers/specs/2026-08-17-git-log-panel-three-pane-redesign-design.md`
//! 第 8 节)。

use crate::theme;
use iced_widget::core::{Element, Font};
use iced_widget::{column, container, text};

/// `patch` 逐行染色:`+` 开头 GREEN、`-` 开头 RED、其余(上下文行/文件头)
/// DIM,等宽字体、`caption_sm()` 字号、`TERM_BG` 背景容器包裹。空字符串
/// 渲染成空的 `column`(不特判——调用方决定"空 patch 时是否要显示占位文案",
/// 这个函数只管染色,不管空态提示)。
pub fn colored_diff_lines<'a, M: 'a>(
    patch: &str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(0).padding([4, 12]);
    for line in patch.lines() {
        let color = if line.starts_with('+') {
            theme::color::GREEN
        } else if line.starts_with('-') {
            theme::color::RED
        } else {
            theme::color::DIM
        };
        col = col.push(
            text(line.to_string())
                .size(theme::font::caption_sm())
                .color(color)
                .font(Font::MONOSPACE)
                .line_height(iced_widget::core::text::LineHeight::Relative(1.3)),
        );
    }
    container(col)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(theme::color::TERM_BG.into()),
            ..iced_widget::container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 纯渲染函数——iced `Element` 没有公开的"读回渲染结果"API,这里只验证
    // 函数在各种输入(空串/纯 +/纯 -/混合/无换行)下不 panic,能正常构造出
    // `Element`。真正的视觉效果靠人工验收(见 spec 测试策略)。
    #[test]
    fn colored_diff_lines_does_not_panic_on_various_inputs() {
        let _: Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> = colored_diff_lines("");
        let _: Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> =
            colored_diff_lines("+added line\n-removed line\n context line\n");
        let _: Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> =
            colored_diff_lines("no trailing newline");
    }
}
```

- [ ] **Step 2: 声明模块**

`main.rs` 里找到现有 `mod` 声明群(按字母序排列),插入:

```rust
mod diff_render;
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer diff_render:: -- --nocapture`
Expected: PASS(新模块首次编译,不存在"确认失败"这一步的先决状态——这是
新增纯函数,Step 1 的测试代码本身在函数写出来之前就无法编译,等价于"失败",
这里 Step 1+Step 3 合并成一次提交前的完整验证,不需要额外的"先跑一次确认
失败"动作)。

- [ ] **Step 4: 改 `acceptance.rs::diff_view` 调用共享函数**

`acceptance.rs:491-529` 现有实现:

```rust
fn diff_view<'a>(
    diff: Option<&'a Result<String, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match diff {
        None => text("加载中…")
            .size(theme::font::caption())
            .color(theme::color::DIM)
            .into(),
        Some(Err(e)) => text(format!("⚠ {e}"))
            .size(theme::font::caption())
            .color(theme::color::RED)
            .into(),
        Some(Ok(patch)) => {
            let mut col = column![].spacing(0).padding([4, 12]);
            for line in patch.lines() {
                let color = if line.starts_with('+') {
                    theme::color::GREEN
                } else if line.starts_with('-') {
                    theme::color::RED
                } else {
                    theme::color::DIM
                };
                col = col.push(
                    text(line.to_string())
                        .size(theme::font::caption_sm())
                        .color(color)
                        .font(iced_widget::core::Font::MONOSPACE)
                        .line_height(LineHeight::Relative(1.3)),
                );
            }
            container(col)
                .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(theme::color::TERM_BG.into()),
                    ..iced_widget::container::Style::default()
                })
                .into()
        }
    }
}
```

改成:

```rust
fn diff_view<'a>(
    diff: Option<&'a Result<String, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match diff {
        None => text("加载中…")
            .size(theme::font::caption())
            .color(theme::color::DIM)
            .into(),
        Some(Err(e)) => text(format!("⚠ {e}"))
            .size(theme::font::caption())
            .color(theme::color::RED)
            .into(),
        Some(Ok(patch)) => crate::diff_render::colored_diff_lines(patch),
    }
}
```

若原文件顶部 `use` 里的 `LineHeight`/`Font` 之类 import 因此变成未使用,
按 `cargo build` 报出的 unused-import 警告删掉对应 `use` 行。

- [ ] **Step 5: 跑现有 acceptance 测试确认没有回归**

Run: `cargo test -p dozer-app --bin dozer acceptance:: -- --nocapture`
Expected: 全部 PASS(视觉渲染不由单测覆盖,这里只保证编译/现有状态机测试
不受影响)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/diff_render.rs crates/dozer-app/src/main.rs \
        crates/dozer-app/src/extensions/acceptance.rs
git commit -m "refactor: 抽取共享 diff 逐行染色渲染函数 diff_render::colored_diff_lines"
```

---

## Task 5: `git_log::State`/`Message` 新增"选中文件"

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs:260-270`(`Message` 枚举)、`:276-286`(`State` 结构体)、`:338-398`(`update()`)
- Test: 同文件测试模块

**Interfaces:**
- Produces: `Message::SelectFile(String)`、`State::selected_file: Option<String>`(私有字段,若后续任务需要从外部读取,加 `pub fn selected_file(&self) -> Option<&str>` accessor——Task 7/8 直接在同一文件内的 `view()`/`view` 辅助函数里访问私有字段即可,不需要 accessor,因为它们都在 `git_log` 模块内)。
- Consumes: 无新外部依赖。

- [ ] **Step 1: 写失败的测试**

```rust
#[tokio::test]
async fn select_file_sets_selected_file() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let result = update(
        &mut state,
        Message::SelectFile("src/main.rs".to_string()),
        &handle,
        |_| {},
    );
    assert_eq!(state.selected_file.as_deref(), Some("src/main.rs"));
    assert!(result.is_none());
}

#[tokio::test]
async fn detail_loaded_preselects_first_file() {
    let repo_path = PathBuf::from("/tmp/repo");
    let oid = git2::Oid::from_bytes(&[10; 20]).unwrap();
    let mut state = State {
        cache: Some(snapshot_at(&repo_path, 10)),
        selected: Some(oid),
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    let detail = CommitDetail {
        files: vec![
            DiffFileEntry {
                path: "a.rs".to_string(),
                status: git2::Delta::Modified,
                patch: "+x".to_string(),
                truncated: false,
            },
            DiffFileEntry {
                path: "b.rs".to_string(),
                status: git2::Delta::Added,
                patch: "+y".to_string(),
                truncated: false,
            },
        ],
    };
    update(
        &mut state,
        Message::DetailLoaded(repo_path, oid, Ok(detail)),
        &handle,
        |_| {},
    );
    assert_eq!(
        state.selected_file.as_deref(),
        Some("a.rs"),
        "detail 落地后应预选第一个改动文件"
    );
}

#[tokio::test]
async fn detail_loaded_with_no_files_clears_selected_file() {
    let repo_path = PathBuf::from("/tmp/repo");
    let oid = git2::Oid::from_bytes(&[11; 20]).unwrap();
    let mut state = State {
        cache: Some(snapshot_at(&repo_path, 10)),
        selected: Some(oid),
        selected_file: Some("stale.rs".to_string()),
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    update(
        &mut state,
        Message::DetailLoaded(repo_path, oid, Ok(CommitDetail { files: Vec::new() })),
        &handle,
        |_| {},
    );
    assert_eq!(state.selected_file, None, "无改动文件时应清空 selected_file,不留旧值");
}

#[tokio::test]
async fn select_commit_clears_selected_file() {
    let repo_path = PathBuf::from("/tmp/repo");
    let mut state = State {
        cache: Some(snapshot_at(&repo_path, 10)),
        selected_file: Some("old.rs".to_string()),
        ..State::default()
    };
    let oid = git2::Oid::from_bytes(&[12; 20]).unwrap();
    let handle = tokio::runtime::Handle::current();
    update(&mut state, Message::SelectCommit(oid), &handle, |_| {});
    assert_eq!(state.selected_file, None, "切 commit 时应先清空旧的 selected_file(等新 detail 落地才重选)");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer git_log::tests::select_file -- --nocapture`
Expected: 编译失败(`Message::SelectFile`/`State.selected_file` 不存在)。

- [ ] **Step 3: 实现**

`Message` 枚举(`git_log.rs:260-270`)追加:

```rust
SelectFile(String),
```

`State` 结构体(`git_log.rs:276-286`)追加字段:

```rust
/// 右上文件列表当前选中的文件路径(`CommitDetail.files[].path`)。切
/// commit 时先清空,新 `detail` 落地后预选第一个改动文件。
selected_file: Option<String>,
```

`update()`(`git_log.rs:338-398`)三处改动:

1. `Message::SelectCommit(oid) => { ... }` 分支开头,`state.detail = None;`
   后追加 `state.selected_file = None;`。
2. `Message::DetailLoaded(repo_path, oid, result) => { ... }` 分支里,
   `if still_current { state.detail = Some(result); }` 改成:

```rust
if still_current {
    state.selected_file = match &result {
        Ok(detail) => detail.files.first().map(|f| f.path.clone()),
        Err(_) => None,
    };
    state.detail = Some(result);
}
```

3. `match msg { ... }` 里新增一支(放在 `Message::SelectCommit` 分支之后
   即可,顺序不影响正确性):

```rust
Message::SelectFile(path) => {
    state.selected_file = Some(path);
    None
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer git_log::tests:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): State/Message 新增选中文件(SelectFile)"
```

---

## Task 6: Commit 列表——删除 Canvas,换成线性可滚动列表

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs:567-670`(`GitLogCanvas` 及其 `canvas::Program` 实现,整块删除)、`:24-33`(部分几何常量,视 Step 3 保留情况决定删留)

**Interfaces:**
- Produces: `fn commit_list_view<'a>(snapshot: &'a GitLogSnapshot, selected: Option<git2::Oid>, head_branch: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`。Task 13(`view()` 重写)调用它替代原来的 `Canvas::new(GitLogCanvas{..})`。
- Consumes: `CommitRow::{oid, short_sha, summary, refs, time, is_merge}`(`time`/`is_merge` 来自 Task 1),`format_commit_time`(Task 2),`icons::IconKind::{GitCommitVertical, GitMerge}`(Task 3),`ref_labels_text`(现有函数,`git_log.rs:676-688`,保留不改)。

- [ ] **Step 1: 删除 `GitLogCanvas`**

删除 `git_log.rs:567-670` 整块(`struct GitLogCanvas` 到
`impl canvas::Program<...> for GitLogCanvas<'_> { ... }` 结束)。

一并检查文件顶部 `use` 语句(`git_log.rs:9-15`):
`iced_widget::canvas::{self, Canvas}`、`iced_widget::core::alignment`、
`Color`(若只被 Canvas 绘制用到)、`Font`(若 `format_commit_time`/其它地方
不再用到 `Font::MONOSPACE` 就删,若 Task 6 新渲染函数里用等宽字体展示 sha
则保留)、`Point`/`Rectangle`/`Vector`——这些多数只被 `GitLogCanvas`/
`row_center`/`track_color` 用到,删除 Canvas 后大概率变成 unused import,
按 `cargo build` 报出的警告逐个清理,不要预先猜测,以编译结果为准。

`row_center`/`track_color`/`TRACK_COLORS` 三个函数/常量(`git_log.rs:37-47`、
`:573-578`)——**如果** `refs_pills` 渲染(Step 2)不需要按分支上色 refs 标签
(spec 没要求这个,简化列表阶段 refs 只是纯文字胶囊,不需要"哪个 track 用哪个
颜色"这层信息),这三个也一并删除。`DOT_RADIUS`/`ROW_HEIGHT`/`COL_WIDTH`/
`LEFT_MARGIN`/`TEXT_GAP`/`LINE_WIDTH`/`DETAIL_WIDTH` 这几个几何常量视图 6
之后重新审查一遍——`DETAIL_WIDTH` 属于旧的"固定宽详情栏"设计,这次改成三栏
FillPortion 布局后不再需要,一并删除;`ROW_HEIGHT` 等 Canvas 专属像素级常量
同理删除,新列表用 iced 原生 `column`/`row` 布局,不需要手算像素坐标。

- [ ] **Step 2: 新增 `commit_list_view`**

在删除 `GitLogCanvas` 的位置(或紧邻 `ref_labels_text` 函数之后)新增:

```rust
/// commit 线性列表(替代原 Canvas 拓扑图,2026-08-17 重构——见 spec
/// "架构与数据流"第 6 节)。每行:图标(普通/合并)+ short_sha + 时间戳 +
/// refs 标签 + summary,整行可点选中(`Message::SelectCommit`),选中态
/// 左侧金色竖条高亮(对齐 Todo/Files 面板既有选中行视觉语言)。
fn commit_list_view<'a>(
    snapshot: &'a GitLogSnapshot,
    selected: Option<git2::Oid>,
    head_branch: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut list = column![].spacing(2);
    for row in &snapshot.rows {
        let is_selected = selected == Some(row.oid);
        let icon_kind = if row.is_merge {
            crate::icons::IconKind::GitMerge
        } else {
            crate::icons::IconKind::GitCommitVertical
        };
        let refs_prefix = ref_labels_text(&row.refs, head_branch);
        let mut line = row![
            crate::icons::view(icon_kind, crate::theme::icon_size::row(), theme::color::DIM),
            text(row.short_sha.clone())
                .size(theme::font::caption())
                .color(theme::color::DIM)
                .font(Font::MONOSPACE),
            text(format_commit_time(row.time))
                .size(theme::font::caption_sm())
                .color(theme::color::DIM),
        ]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center);
        if !refs_prefix.is_empty() {
            line = line.push(
                text(refs_prefix)
                    .size(theme::font::caption_sm())
                    .color(theme::color::CYAN),
            );
        }
        line = line.push(
            text(row.summary.clone())
                .size(theme::font::caption())
                .color(theme::color::CREAM),
        );
        let accent = container(iced_widget::Space::new())
            .width(Length::Fixed(3.0))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: if is_selected { Some(theme::color::GOLD.into()) } else { None },
                ..container::Style::default()
            });
        let inner = row![accent, container(line).padding([4, 8]).width(Length::Fill)].spacing(0);
        let area = iced_widget::MouseArea::new(inner)
            .interaction(iced_widget::core::mouse::Interaction::Pointer)
            .on_press(Message::SelectCommit(row.oid));
        list = list.push(area);
    }
    scrollable(list).width(Length::Fill).height(Length::Fill).into()
}
```

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 报错(`view()` 里还在引用被删掉的 `Canvas::new(GitLogCanvas{..})`
——这是预期的中间态,`view()` 的重写留给 Task 13,这一步只确认
`commit_list_view` 本身、以及 `GitLogCanvas` 删除后其余代码(除 `view()`
外)没有编译错误)。用 `cargo build -p dozer-app 2>&1 | grep "extensions/git_log.rs"`
确认报错都集中在 `view()` 函数内、且报错原因是"用到了已删除的 `GitLogCanvas`/
`Canvas`/几何常量",不是别的意外错误。

- [ ] **Step 4: Commit**

先不追求这一步能编译通过(`view()` 还没改完,留给 Task 13)——但为了保持
"小步提交、每步都是一个独立可读的 diff"的既有习惯,这次提交允许暂时不绿
(`cargo build` 会报 `view()` 里的错),在 commit message 里注明:

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): 新增 commit_list_view,删除 GitLogCanvas(view() 暂未接线,Task 13 收尾)"
```

---

## Task 7: 文件列表面板

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`(新增函数,紧邻 `commit_list_view` 之后)

**Interfaces:**
- Produces: `fn file_list_view<'a>(detail: &'a Result<CommitDetail, String>, selected_file: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`。Task 13 调用。
- Consumes: `CommitDetail::files`(现有)、`status_glyph`(现有函数,`git_log.rs:867-876`,不改)。

- [ ] **Step 1: 实现**

（纯渲染函数,视觉逻辑照抄现有 `detail_view` 的文件行部分——这里不用
"先写测试后实现"的 TDD 流程,因为 iced `Element` 渲染函数没有可断言的
返回值,项目里现有同类渲染函数〔`detail_view`/`todo_card`/`file_list_view`
所在的既有代码〕也都没有单测,只有编译期类型检查 + 人工验收,这与 Task 1/2/5
"有可断言纯逻辑"的情况不同,不强行套 TDD 五步模板。）

```rust
/// 右上文件列表:选中 commit 改动的每个文件一行(状态字符 + 路径),点击
/// 发 `Message::SelectFile`,选中态同 `commit_list_view` 的左侧金色竖条。
fn file_list_view<'a>(
    detail: &'a Result<CommitDetail, String>,
    selected_file: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match detail {
        Err(err) => container(text(format!("详情加载失败: {err}")).color(theme::color::RED))
            .padding(8)
            .into(),
        Ok(detail) if detail.files.is_empty() => {
            container(text("无文件改动").color(theme::color::DIM))
                .padding(8)
                .into()
        }
        Ok(detail) => {
            let mut list = column![].spacing(2);
            for f in &detail.files {
                let is_selected = selected_file == Some(f.path.as_str());
                let color = match f.status {
                    git2::Delta::Added => theme::color::GREEN,
                    git2::Delta::Deleted => theme::color::RED,
                    _ => theme::color::CYAN,
                };
                let line = row![
                    text(status_glyph(f.status)).color(color).width(18),
                    text(f.path.clone())
                        .size(theme::font::caption())
                        .color(theme::color::CREAM),
                ]
                .spacing(4);
                let accent = container(iced_widget::Space::new())
                    .width(Length::Fixed(3.0))
                    .height(Length::Fill)
                    .style(move |_t: &iced_widget::Theme| container::Style {
                        background: if is_selected { Some(theme::color::GOLD.into()) } else { None },
                        ..container::Style::default()
                    });
                let inner = row![accent, container(line).padding([2, 8]).width(Length::Fill)].spacing(0);
                let area = iced_widget::MouseArea::new(inner)
                    .interaction(iced_widget::core::mouse::Interaction::Pointer)
                    .on_press(Message::SelectFile(f.path.clone()));
                list = list.push(area);
            }
            scrollable(list).width(Length::Fill).height(Length::Fill).into()
        }
    }
}
```

- [ ] **Step 2: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | grep "file_list_view"`
Expected: 无输出(`file_list_view` 本身编译无误;`view()` 还没调它,不算
"未使用函数"警告——`pub(crate)`/私有函数在同文件内暂未被调用时 Rust 会警告
`dead_code`,这是预期的中间态,留给 Task 13 消除)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): 新增 file_list_view 文件列表面板"
```

---

## Task 8: diff 内容面板

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`(新增函数,紧邻 `file_list_view` 之后;删除旧的 `detail_view` 函数,`git_log.rs:807-865`——它的职责被 `file_list_view` + 本任务的新函数拆开取代)

**Interfaces:**
- Produces: `fn diff_pane_view<'a>(detail: &'a Result<CommitDetail, String>, selected_file: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`。Task 13 调用。
- Consumes: `diff_render::colored_diff_lines`(Task 4 产出)。

- [ ] **Step 1: 删除旧 `detail_view`**

删除 `git_log.rs:807-865`(`fn detail_view<'a>(...)` 整个函数)——它的三重
职责(错误态/空态/文件列表+平铺 diff)被 `file_list_view`(Task 7,文件列表 +
选中态)和本任务的 `diff_pane_view`(单文件 diff)拆开接管。

- [ ] **Step 2: 实现**

```rust
/// 右下 diff 内容面板:`selected_file` 对应文件的 patch,逐行染色(复用
/// `diff_render::colored_diff_lines`,Task 4)。找不到该路径(比如换 commit
/// 那一瞬间 `selected_file` 还没跟上新 `detail`)或未选中任何文件时展示
/// 占位文案,不 panic。
fn diff_pane_view<'a>(
    detail: &'a Result<CommitDetail, String>,
    selected_file: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Ok(detail) = detail else {
        // 错误态已经在 file_list_view 里展示过一次,这里不重复展示错误
        // 文案,给个中性占位即可。
        return container(iced_widget::Space::new()).into();
    };
    let Some(path) = selected_file else {
        return container(text("未选中文件").color(theme::color::DIM))
            .padding(8)
            .into();
    };
    let Some(entry) = detail.files.iter().find(|f| f.path == path) else {
        return container(text("未选中文件").color(theme::color::DIM))
            .padding(8)
            .into();
    };
    let mut content = column![
        text(entry.path.clone())
            .size(theme::font::caption())
            .color(theme::color::DIM)
    ]
    .spacing(4);
    if entry.patch.is_empty() {
        content = content.push(text("(无 diff 内容)").color(theme::color::DIM));
    } else {
        content = content.push(crate::diff_render::colored_diff_lines(&entry.patch));
    }
    if entry.truncated {
        content = content.push(
            text("… diff 过长,已截断显示")
                .size(theme::font::caption_sm())
                .color(theme::color::DIM),
        );
    }
    scrollable(content).width(Length::Fill).height(Length::Fill).into()
}
```

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 除 `view()` 里对已删除函数(`GitLogCanvas`/`detail_view`)的引用外,
无其它新增编译错误。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): 新增 diff_pane_view,删除旧 detail_view"
```

---

## Task 9: `git_log::State`/`Message` 新增"分支切换"

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs:260-270`(`Message`)、`:276-286`(`State`)、`:338-398`(`update()`)
- Test: 同文件测试模块

**Interfaces:**
- Produces: `Message::{BranchPickerOpen, BranchPickerClose, BranchesLoaded(PathBuf, Vec<String>), BranchSwitch(String), BranchSwitchDone(Result<(), String>)}`;`State::{branch_picker_open: bool, branches: Vec<String>, branch_switch_pending: bool}`;`State::cache_max_count()`(**已存在**,`git_log.rs:302-307`,不用新增,Task 14 内核侧直接调用它算 `request_refresh` 的 `max_count`)。
- Consumes: 无(本任务只做状态机,不做真实 IO——`delivery::local_branches`/`checkout_branch` 的调用在 Task 14 内核侧发起)。

- [ ] **Step 1: 写失败的测试**

```rust
#[tokio::test]
async fn branch_picker_open_and_close_toggle_flag() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    update(&mut state, Message::BranchPickerOpen, &handle, |_| {});
    assert!(state.branch_picker_open);
    update(&mut state, Message::BranchPickerClose, &handle, |_| {});
    assert!(!state.branch_picker_open);
}

#[tokio::test]
async fn branches_loaded_lands_when_repo_path_matches_cache() {
    let repo_path = PathBuf::from("/tmp/repo");
    let mut state = State {
        cache: Some(snapshot_at(&repo_path, 10)),
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    update(
        &mut state,
        Message::BranchesLoaded(repo_path, vec!["main".to_string(), "dev".to_string()]),
        &handle,
        |_| {},
    );
    assert_eq!(state.branches, vec!["main".to_string(), "dev".to_string()]);
}

#[tokio::test]
async fn branches_loaded_discarded_when_repo_path_mismatches() {
    let mut state = State {
        cache: Some(snapshot_at(Path::new("/tmp/a"), 10)),
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    update(
        &mut state,
        Message::BranchesLoaded(PathBuf::from("/tmp/b"), vec!["main".to_string()]),
        &handle,
        |_| {},
    );
    assert!(state.branches.is_empty(), "仓库路径对不上,不该落地");
}

#[tokio::test]
async fn branch_switch_done_ok_clears_pending_and_closes_picker() {
    let mut state = State {
        branch_picker_open: true,
        branch_switch_pending: true,
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    update(&mut state, Message::BranchSwitchDone(Ok(())), &handle, |_| {});
    assert!(!state.branch_picker_open);
    assert!(!state.branch_switch_pending);
    assert!(state.error.is_none());
}

#[tokio::test]
async fn branch_switch_done_err_sets_error_and_clears_pending() {
    let mut state = State {
        branch_switch_pending: true,
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    update(
        &mut state,
        Message::BranchSwitchDone(Err("checkout 失败".to_string())),
        &handle,
        |_| {},
    );
    assert!(!state.branch_switch_pending);
    assert_eq!(state.error.as_deref(), Some("checkout 失败"));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer git_log::tests::branch -- --nocapture`
Expected: 编译失败。

- [ ] **Step 3: 实现**

`Message` 枚举追加:

```rust
BranchPickerOpen,
BranchPickerClose,
BranchesLoaded(PathBuf, Vec<String>),
/// 内核在 `Message::GitLog` 分发里截获处理(需要仓库路径 + 真实 IO),
/// 不会转发到这里——传进 `update` 会直接 panic(见其 `unreachable!` 分支,
/// 镜像 `LoadMore`/`ProjectTabOpen` 的既有例外模式)。
BranchSwitch(String),
BranchSwitchDone(Result<(), String>),
```

`State` 结构体追加:

```rust
/// 面板底部分支下拉是否展开。
branch_picker_open: bool,
/// 当前仓库的本地分支列表(`delivery::local_branches` 结果缓存,内核在
/// `BranchPickerOpen` 首次展开时异步查一次)。
branches: Vec<String>,
/// 分支切换请求进行中(禁用下拉交互、显示"切换中…")。
branch_switch_pending: bool,
```

`update()` 新增分支(`match msg { ... }` 里追加):

```rust
Message::BranchPickerOpen => {
    state.branch_picker_open = true;
    None
}
Message::BranchPickerClose => {
    state.branch_picker_open = false;
    None
}
Message::BranchesLoaded(repo_path, branches) => {
    let matches = state.cache.as_ref().map(|c| c.repo_path()) == Some(repo_path.as_path());
    if matches {
        state.branches = branches;
    }
    None
}
Message::BranchSwitch(_) => {
    unreachable!(
        "BranchSwitch 由内核在 Message::GitLog 分支里直接处理(需要仓库路径 + 真实 checkout IO),不会转发到这里"
    )
}
Message::BranchSwitchDone(result) => {
    state.branch_picker_open = false;
    state.branch_switch_pending = false;
    match result {
        Ok(()) => state.error = None,
        Err(e) => state.error = Some(e),
    }
    None
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer git_log::tests:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): State/Message 新增分支切换状态机"
```

---

## Task 10: 分支切换下拉(局部 overlay 渲染)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`(新增函数,紧邻 `diff_pane_view` 之后)

**Interfaces:**
- Produces: `fn branch_picker_view<'a>(state: &'a State, head_branch: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`(展开时返回覆盖层,收起时返回空元素)、`fn branch_toggle_button<'a>(state: &'a State, head_branch: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`(左侧面板底部固定展示的当前分支+展开按钮)。Task 13 调用,两者都需要 `head_branch`(来自 `GitLogSnapshot::head_branch()`,Step 1 新增的 accessor)判断"哪个分支是当前分支"。
- Consumes: `State::{branch_picker_open, branches, branch_switch_pending}`(Task 9)、`GitLogSnapshot::head_branch`(现有,私有字段,需要新增 `pub fn head_branch(&self) -> Option<&str>` accessor——见 Step 1)。

- [ ] **Step 1: 给 `GitLogSnapshot` 加 `head_branch` accessor + 补 `Border` 导入**

`git_log.rs` 顶部 `use iced_widget::core::{..}` 那一行(原为
`Color, Element, Font, Length, Pixels, Point, Rectangle, Vector`,Task 6
Step 1 已经按 `cargo build` 警告清理过 `Point`/`Rectangle`/`Vector`/`Pixels`
等 Canvas 专属导入)追加 `Border`——本任务的 `branch_toggle_button`/
`branch_picker_view` 用到 `Border { color, width, radius }` 构造边框样式,
而 `git_log.rs` 目前的导入列表里没有它(`files.rs`/`todo.rs` 都在同一行
`use iced_widget::core::{Border, ...}` 里带了这个,`git_log.rs` 之前没用到
过边框样式,没导入)。

`git_log.rs:96-104`(`impl GitLogSnapshot` 现有 `repo_path()`/`max_count()`
accessor 旁边)追加:

```rust
pub fn head_branch(&self) -> Option<&str> {
    self.head_branch.as_deref()
}
```

- [ ] **Step 2: 实现 `branch_toggle_button`(底部固定按钮)**

```rust
/// 左侧面板底部固定展示:当前分支名 + 展开箭头,点击发
/// `Message::BranchPickerOpen`/`BranchPickerClose`(按当前展开态二选一)。
fn branch_toggle_button<'a>(state: &'a State, head_branch: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let label = head_branch.unwrap_or("(无分支)");
    let msg = if state.branch_picker_open {
        Message::BranchPickerClose
    } else {
        Message::BranchPickerOpen
    };
    iced_widget::button(
        row![
            text(label).size(theme::font::body()).color(theme::color::CREAM),
            iced_widget::Space::new().width(Length::Fill),
            crate::icons::view(crate::icons::IconKind::ChevronDown, crate::theme::icon_size::row(), theme::color::DIM),
        ]
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .padding([6, 10])
    .on_press_maybe((!state.branch_switch_pending).then_some(msg))
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: None,
        text_color: theme::color::CREAM,
        border: Border { color: theme::color::BORDER, width: 1.0, radius: 6.0.into() },
        ..iced_widget::button::Style::default()
    })
    .into()
}
```

（`IconKind::ChevronDown` 已存在于 `icons.rs`——项目里 chevron 系列图标已
广泛使用,直接复用,不属于本计划新增图标范围。若命名与现有变体不完全一致,
以 `grep -n "ChevronDown" crates/dozer-app/src/icons.rs` 的实际结果为准
调整引用名。）

- [ ] **Step 3: 实现 `branch_picker_view`(局部 overlay)**

```rust
/// 分支下拉展开层:局部 `stack!`(不是 window-wide overlay,只覆盖左侧
/// Git 面板范围),视觉风格照抄 `files.rs::branch_picker_popup`(CARD 底/
/// BORDER 描边/当前分支 GOLD 高亮),但状态完全独立(不读 `files::
/// WorkspaceState`)。`branch_switch_pending` 时全部禁用并显示"切换中…"。
fn branch_picker_view<'a>(state: &'a State, head_branch: Option<&'a str>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !state.branch_picker_open {
        return iced_widget::Space::new().into();
    }
    let mut list = column![].spacing(2).width(Length::Fill);
    if state.branches.is_empty() {
        list = list.push(text("暂无本地分支").size(theme::font::body()).color(theme::color::DIM));
    }
    for name in &state.branches {
        let is_current = Some(name.as_str()) == head_branch;
        let color = if is_current { theme::color::GOLD } else { theme::color::CREAM };
        let row_btn = iced_widget::button(
            text(name.clone()).size(theme::font::body()).color(color),
        )
        .width(Length::Fill)
        .padding([6, 10])
        .style(move |_t: &iced_widget::Theme, s: iced_widget::button::Status| {
            let base = iced_widget::button::Style { background: None, text_color: color, ..iced_widget::button::Style::default() };
            match s {
                iced_widget::button::Status::Hovered => iced_widget::button::Style {
                    background: Some(theme::color::TAB_HOVER.into()),
                    ..base
                },
                _ => base,
            }
        });
        let row_btn = if state.branch_switch_pending || is_current {
            row_btn
        } else {
            row_btn.on_press(Message::BranchSwitch(name.clone()))
        };
        list = list.push(row_btn);
    }
    let panel = container(list)
        .padding(8)
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border { color: theme::color::BORDER, width: 1.0, radius: 6.0.into() },
            ..container::Style::default()
        });
    let dismiss = iced_widget::MouseArea::new(iced_widget::Space::new().width(Length::Fill).height(Length::Fill))
        .on_press(Message::BranchPickerClose);
    iced_widget::stack![dismiss, panel].width(Length::Fill).height(Length::Shrink).into()
}
```

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | grep "branch_picker_view\|branch_toggle_button"`
Expected: 无编译错误输出(未接线到 `view()` 产生的 `dead_code` 警告是预期
的中间态)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): 新增分支切换下拉渲染(branch_picker_view/branch_toggle_button)"
```

---

## Task 11: 左右分割线——`PanelDims.git_log_split` + `Divider::GitLogSplit`

**Files:**
- Modify: `crates/dozer-app/src/app.rs:322-337`(`PanelDims`)、`:343-353`(`default_panel_dims`)、`:416-440`(`sanitize_panel_dims`)、`:445-455`(`Divider`)、`:573-650`附近(`apply_column_drag`)
- Test: `crates/dozer-app/src/app.rs`(`#[cfg(test)] mod tests`,先 `grep -n "mod tests" crates/dozer-app/src/app.rs` 确认现有测试模块位置,追加在同类 `apply_column_drag`/`sanitize_panel_dims` 测试旁边)

**Interfaces:**
- Produces: `PanelDims::git_log_split: f32`、`Divider::GitLogSplit`。Task 13(`view()`)、Task 14(内核 `Message::GitLog` 分发里读 `self.dims.git_log_split`)消费。
- Consumes: 现有 `theme::geometry::default_split_ratio()`/`min_split_ratio()`/`max_split_ratio()`、`pair_content_width`/`left_zone_width`(均已存在,不改)。

- [ ] **Step 1: 写失败的测试**

先 `grep -n "fn apply_column_drag_todo_split\|TodoSplit" crates/dozer-app/src/app.rs`
找到现有 `TodoSplit` 分支的测试用例作为参照格式,在其旁边追加(具体测试
函数名以现有命名习惯为准,以下示例假设现有测试形如
`apply_column_drag_updates_todo_split_ratio`,若实际命名不同按实际调整
新测试的命名前缀,保持一致性):

```rust
#[test]
fn apply_column_drag_updates_git_log_split_ratio() {
    let state = ShellState {
        dims: PanelDims::default(),
        ..test_shell_state() // 若现有测试文件里有类似的 test_shell_state() helper 就复用;
                              // 若没有,按现有 TodoSplit 测试构造 ShellState 的方式原样抄一份
    };
    let window_width = 1600.0;
    let result = apply_column_drag(state, Divider::GitLogSplit, window_width, 300.0);
    assert!(result.git_log_split >= theme::geometry::min_split_ratio());
    assert!(result.git_log_split <= theme::geometry::max_split_ratio());
}

#[test]
fn sanitize_panel_dims_clamps_git_log_split() {
    let dims = PanelDims { git_log_split: 5.0, ..PanelDims::default() };
    let sanitized = sanitize_panel_dims(dims);
    assert!(sanitized.git_log_split <= theme::geometry::max_split_ratio());

    let dims = PanelDims { git_log_split: f32::NAN, ..PanelDims::default() };
    let sanitized = sanitize_panel_dims(dims);
    assert_eq!(sanitized.git_log_split, PanelDims::default().files_split);
}
```

（构造 `ShellState`/`test_shell_state()` 的具体写法必须照抄本文件里已有的
`TodoSplit`/`ProjectSplit` 拖拽测试——执行者在写 Step 1 之前先跑
`grep -n "fn.*apply_column_drag" crates/dozer-app/src/app.rs` 读一遍现有
测试的完整代码,不要凭空猜测 `ShellState` 的构造字段。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer app::tests::apply_column_drag_updates_git_log_split_ratio -- --nocapture`
Expected: 编译失败(`Divider::GitLogSplit`/`PanelDims.git_log_split` 不存在)。

- [ ] **Step 3: 实现**

`PanelDims`(`app.rs:322-337`)追加字段:

```rust
/// Git Log 面板配对:commit 列表占左面板区宽度的比例,右侧(文件列表+diff)
/// 拿剩下的。
pub git_log_split: f32,
```

`default_panel_dims()`(`app.rs:343-353`)追加:

```rust
git_log_split: theme::geometry::default_split_ratio(),
```

`sanitize_panel_dims()`(`app.rs:416-440`)在 `PanelDims { .. }` 字面量里追加:

```rust
git_log_split: clamp_split(d.git_log_split),
```

`Divider`(`app.rs:445-455`)追加变体:

```rust
/// Git Log 面板内部左右分隔线:左边 commit 列表,右边文件列表+diff。
GitLogSplit,
```

`apply_column_drag`(`app.rs:573` 起的 `match divider { ... }`)追加一支,
逻辑完全镜像 `TodoSplit`:

```rust
Divider::GitLogSplit => {
    let pair_w = pair_content_width(left_zone_width(window_width, &state));
    if pair_w <= 0.0 {
        return state.dims;
    }
    let ratio = ((logical_x - theme::geometry::icon_rail_width()) / pair_w).clamp(
        theme::geometry::min_split_ratio(),
        theme::geometry::max_split_ratio(),
    );
    PanelDims {
        git_log_split: ratio,
        ..state.dims
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer app:: -- --nocapture`
Expected: 全部 PASS(含现有的 `PanelDims`/`Divider`/`apply_column_drag`
既有测试,确认没有回归)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(app): PanelDims/Divider 新增 Git Log 左右分割线(GitLogSplit)"
```

---

## Task 12: 右侧上下分割线——`RowDivider`/`apply_row_drag`/纵向拖拽消息

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(`PanelDims` 再加一字段;新增 `RowDivider` 枚举、`apply_row_drag` 纯函数、`Message::{RowDragStart, RowDrag, RowDragEnd}`、`App.dragging_row: Option<RowDivider>` 字段及 `dragging_row()`/相关 accessor、`update()` 里新增三支处理、新增纵向 `divider_bar` 姊妹函数)、`crates/dozer-app/src/main.rs`(`CursorMoved` 处理追加纵向拖拽分支)
- Test: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces: `PanelDims::git_log_file_diff_split: f32`、`RowDivider::GitLogFileDiffSplit`、`Message::{RowDragStart(RowDivider), RowDrag{window_height: f32, logical_y: f32}, RowDragEnd}`、`fn apply_row_drag(state: ShellState, divider: RowDivider, window_height: f32, logical_y: f32) -> PanelDims`、`fn horizontal_divider_bar<'a>(divider: RowDivider, top_bg: Color, bottom_bg: Color) -> Element<'a, Message, ..>`。Task 13(`view()`)、Task 14(内核 `git_log_file_diff_split` 读取)消费。
- Consumes: `theme::geometry::{min_split_ratio, max_split_ratio, divider_width}`(横向的 `divider_width()` 是宽度,纵向新函数需要一个"高度"版本——先检查
  `theme::geometry` 是否已有通用的粗细常量可复用,若只有 `divider_width()`
  这一个,直接复用同一个数值当"分割线厚度",不新增常量,横竖两个方向共用
  同一粗细,视觉一致)。

**已知无先例,几何计算需要新写**(spec 明确标注这是仓库第一条纵向拖拽线):

- [ ] **Step 1: 写 `apply_row_drag` 的失败测试**

先确定"右侧区域可用高度"怎么算。`git_log::view()` 右侧区域(文件列表+diff)
的可用高度 = 面板总高度 - 标题栏高度 - worktree 条高度 - 内边距。这些具体
数值在 Task 13 写 `view()` 时才最终确定实际像素——这里先用一个**内核不关心
面板内部细节、只关心"总可用高度"**的设计:`apply_row_drag` 的调用方
(`App::update`)传入的 `window_height` 就是整个窗口高度,函数内部按窗口
高度减去一个**固定估算值**(顶栏 + footbar + 面板 padding,参考现有
`maximized_box_height`/`status_bar_height` 一类"已经在算窗口内可用高度"
的既有函数,具体做法是复用 `theme::geometry::status_bar_height()`
这类现成的窗口级几何函数,不要在 `apply_row_drag` 里重新定义一套"Git Log
面板专属"的偏移量估算——若现有窗口级几何函数不足以精确定位到"Git Log
面板右侧区域顶部在屏幕上的 y 坐标",这一版先用**近似值**(比如"窗口高度
的 15% 处大致是面板内容区顶部"这类粗略估算),测试只验证"比例落在合法
区间"而不验证"像素级精确对齐",人工验收阶段(Task 15)再肉眼调整。

```rust
#[test]
fn apply_row_drag_clamps_ratio_within_valid_range() {
    let state = ShellState { dims: PanelDims::default(), ..test_shell_state() };
    let window_height = 1000.0;

    // 光标在窗口中间——应该落在合法比例区间内。
    let mid = apply_row_drag(state.clone(), RowDivider::GitLogFileDiffSplit, window_height, 500.0);
    assert!(mid.git_log_file_diff_split >= theme::geometry::min_split_ratio());
    assert!(mid.git_log_file_diff_split <= theme::geometry::max_split_ratio());

    // 光标远超窗口顶部/底部——应该被 clamp,不产生非法比例(NaN/负数/>1)。
    let top = apply_row_drag(state.clone(), RowDivider::GitLogFileDiffSplit, window_height, -500.0);
    assert_eq!(top.git_log_file_diff_split, theme::geometry::min_split_ratio());
    let bottom = apply_row_drag(state, RowDivider::GitLogFileDiffSplit, window_height, 5000.0);
    assert_eq!(bottom.git_log_file_diff_split, theme::geometry::max_split_ratio());
}

#[test]
fn sanitize_panel_dims_clamps_git_log_file_diff_split() {
    let dims = PanelDims { git_log_file_diff_split: -1.0, ..PanelDims::default() };
    let sanitized = sanitize_panel_dims(dims);
    assert!(sanitized.git_log_file_diff_split >= theme::geometry::min_split_ratio());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer app::tests::apply_row_drag -- --nocapture`
Expected: 编译失败(`apply_row_drag`/`RowDivider`/`PanelDims.git_log_file_diff_split` 均不存在)。

- [ ] **Step 3: 实现**

`PanelDims` 追加字段:

```rust
/// Git Log 面板右侧配对:文件列表占右侧区域高度的比例,diff 内容拿剩下的。
pub git_log_file_diff_split: f32,
```

`default_panel_dims()` 追加 `git_log_file_diff_split:
theme::geometry::default_split_ratio(),`。

`sanitize_panel_dims()` 追加 `git_log_file_diff_split:
clamp_split(d.git_log_file_diff_split),`。

新增枚举(放在 `Divider` 定义之后):

```rust
/// 纵向(上下)可拖拽分割线——目前只有 Git Log 面板右侧"文件列表 | diff
/// 内容"这一条,单独开一个枚举而不是塞进 `Divider`(横向语义不同,`Divider`
/// 现有六个变体全部是左右分割,`apply_column_drag`/`Message::ColumnDrag`
/// 的几何计算全部基于 `logical_x`,混进去会让那个函数的语义变得模糊)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowDivider {
    GitLogFileDiffSplit,
}
```

新增纯函数(放在 `apply_column_drag` 之后):

```rust
/// `apply_column_drag` 的纵向镜像:按 `logical_y`/`window_height` 算比例。
/// "可用高度"用近似估算(粗略减去顶栏/footbar 这类固定装饰高度)——Git Log
/// 面板内部标题/worktree 条的精确高度不在这里计算,内核不关心面板内部布局
/// 细节,只提供窗口级的粗略换算;像素级对齐精度不足时人工验收阶段允许
/// 后续单独调整这个估算值,不阻塞本任务。
pub(crate) fn apply_row_drag(
    state: ShellState,
    divider: RowDivider,
    window_height: f32,
    logical_y: f32,
) -> PanelDims {
    match divider {
        RowDivider::GitLogFileDiffSplit => {
            let usable_height = (window_height - theme::geometry::status_bar_height() * 2.0).max(1.0);
            let ratio = (logical_y / usable_height).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            PanelDims {
                git_log_file_diff_split: ratio,
                ..state.dims
            }
        }
    }
}
```

新增消息变体(`Message` 枚举,`ColumnDragStart`/`ColumnDrag`/`ColumnDragEnd`
旁边,`app.rs:1285-1292`):

```rust
RowDragStart(RowDivider),
RowDrag { window_height: f32, logical_y: f32 },
RowDragEnd,
```

`App` 结构体新增字段(紧邻现有 `dragging: Option<Divider>` 字段——先
`grep -n "dragging: Option<Divider>" crates/dozer-app/src/app.rs` 定位):

```rust
dragging_row: Option<RowDivider>,
```

对应初始化处(`App::new`/构造函数里 `dragging: None,` 旁边)追加
`dragging_row: None,`。

新增 accessor(`dragging_divider()` 旁边,`app.rs:2794` 附近):

```rust
pub fn dragging_row(&self) -> Option<RowDivider> {
    self.dragging_row
}
```

`update()` 里 `Message::ColumnDragEnd => { .. }` 分支之后追加三支:

```rust
Message::RowDragStart(divider) => {
    self.dragging_row = Some(divider);
}
Message::RowDrag { window_height, logical_y } => {
    if let Some(divider) = self.dragging_row {
        let state = self.shell_state();
        self.dims = apply_row_drag(state, divider, window_height, logical_y);
    }
}
Message::RowDragEnd => {
    self.dragging_row = None;
    self.on_shell_layout_changed();
}
```

新增纵向 `divider_bar` 姊妹函数(紧邻现有 `divider_bar`,`app.rs:7119` 附近,
结构镜像,`row!`→`column!`,`width`↔`height` 互换,`ResizingColumn`→
`ResizingRow`——若 iced 0.14 的 `mouse::Interaction` 没有 `ResizingRow`
变体,`grep -n "Resizing" ~/.cargo/registry/src/*/iced_core-0.14*/src/mouse/interaction.rs`
核实实际枚举名,常见的是 `ResizingHorizontally`/`ResizingVertically`,若
现有横向 `divider_bar` 用的是 `ResizingColumn`,说明命名不是"方向"而是
"哪个轴向的容器在缩放",纵向姊妹函数按同一命名思路推类比命名,以 iced 0.14
实际枚举为准调整):

```rust
fn horizontal_divider_bar<'a>(
    divider: RowDivider,
    top_bg: Color,
    bottom_bg: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let line_h = 2.0_f32;
    let side_h = (theme::geometry::divider_width() - line_h) / 2.0;
    let top_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(top_bg.into()),
            ..container::Style::default()
        });
    let bottom_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(bottom_bg.into()),
            ..container::Style::default()
        });
    let line = container(iced_widget::Space::new())
        .height(Length::Fixed(line_h))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BORDER.into()),
            ..container::Style::default()
        });
    let col = column![top_side, line, bottom_side]
        .height(Length::Fixed(theme::geometry::divider_width()))
        .width(Length::Fill);
    MouseArea::new(col)
        .interaction(mouse::Interaction::ResizingRow)
        .on_press(Message::RowDragStart(divider))
        .into()
}
```

（`ResizingRow` 已核实存在于 `iced_core::mouse::Interaction`(与横向
`divider_bar` 用的 `ResizingColumn` 是同一枚举里的姊妹变体,`~/.cargo/
registry/src/*/iced_core-0.14*/src/mouse/interaction.rs` 可查),直接使用,
不需要额外核实。）

`main.rs` 的 `CursorMoved` 处理(`main.rs:628-637` 附近,`if app.
dragging_divider().is_some() { .. }` 那段)之后追加镜像分支:

```rust
if app.dragging_row().is_some() {
    let scale = window.scale_factor();
    let logical_y = (cursor_phys.y / scale) as f32;
    let window_height = (window.inner_size().height as f64 / scale) as f32;
    app.update(Message::RowDrag { window_height, logical_y });
}
```

`WindowEvent::MouseInput { state: ElementState::Released, .. }` 处理里,
找到现有 `app.update(Message::ColumnDragEnd);`(`main.rs:715` 附近)那一行,
其后追加:

```rust
app.update(Message::RowDragEnd);
```

（`ColumnDragEnd`/`RowDragEnd` 两条都无条件发也没关系——`update()` 里
`self.dragging_row = None` 对"本来就是 `None`"是幂等操作,不需要额外判断
"这次抬起到底是横向还是纵向拖拽结束",跟现有 `ColumnDragEnd` 的既有写法
风格一致,不新增判断分支。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer app:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 5: 全量编译确认**

Run: `cargo build -p dozer-app`
Expected: 编译通过(`horizontal_divider_bar`/`RowDivider` 等新代码此时
还未被 `git_log::view()` 使用,允许 `dead_code` 警告,不允许编译错误)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs
git commit -m "feat(app): 新增纵向拖拽机制(RowDivider/apply_row_drag/RowDrag*),Git Log 右侧上下分割线用"
```

---

## Task 13: `git_log::view()` 重写,拼三栏布局

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs:693-802`(`view()` 整个函数重写)

**Interfaces:**
- Consumes: Task 6(`commit_list_view`)、Task 7(`file_list_view`)、Task 8(`diff_pane_view`)、Task 9/10(分支切换状态+渲染)、Task 11(`Divider::GitLogSplit`)、Task 12(`RowDivider::GitLogFileDiffSplit`/`horizontal_divider_bar`)。
- Produces: `git_log::view(state: &State, worktrees: &[WorktreeInfo], git_log_split: f32, git_log_file_diff_split: f32) -> Element<'_, Message, ..>`——**签名变化**:新增两个 `f32` 参数(内核持有的 split 比例,`git_log` 模块自己不存布局比例,跟 `Todo`/`Project` 面板现有"内核传 split 值进来"的既有模式一致)。Task 14(内核调用点)要跟着改调用参数。

- [ ] **Step 1: 检查现有 Todo/Project 面板 `view()` 怎么接收 split 参数**

Run: `grep -n "pub fn view" crates/dozer-app/src/extensions/todo.rs`

读一下 Todo 面板 `view()` 的参数列表(是不是也是"内核传 split 比例进来"
这个模式),确认本任务的新 `git_log::view()` 签名跟现有同类面板保持一致的
参数顺序/命名习惯。

- [ ] **Step 2: 重写 `view()`**

```rust
pub fn view<'a>(
    state: &'a State,
    worktrees: &'a [WorktreeInfo],
    git_log_split: f32,
    git_log_file_diff_split: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let error = state.error.as_deref();
    let head = crate::homespace::home_panel_head(crate::icons::IconKind::GitGraph, "Git");

    let loading = state.pending.is_some();
    let Some(snapshot) = state.cache.as_ref() else {
        let text_content = if loading { "加载中…" } else { "未打开项目" };
        return container(column![head, text(text_content).color(theme::color::DIM)].spacing(8).padding(12)).into();
    };
    if snapshot.rows.is_empty() {
        return container(column![head, text("没有可显示的提交").color(theme::color::DIM)].spacing(8).padding(12)).into();
    }

    let head_branch = snapshot.head_branch();
    let mut left = column![head, worktree_strip(worktrees)].spacing(8);
    if let Some(err) = error {
        left = left.push(text(format!("git log 读取失败: {err}")).size(theme::font::caption()).color(theme::color::RED));
    }
    left = left.push(commit_list_view(snapshot, state.selected, head_branch));
    let load_more = iced_widget::button(
        text("加载更多提交 (+200)").size(theme::font::caption()).color(theme::color::CREAM),
    )
    .on_press_maybe((!loading).then_some(Message::LoadMore))
    .padding([4, 12]);
    left = left.push(load_more);
    left = left.push(branch_toggle_button(state, head_branch));
    let left_with_picker = iced_widget::stack![
        container(left).width(Length::Fill).height(Length::Fill),
        branch_picker_view(state, head_branch),
    ];

    let (list_portion, content_portion) = crate::workspace::split_portions(git_log_split);
    let right: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(detail) = state.detail.as_ref() {
            let (top_portion, bottom_portion) = crate::workspace::split_portions(git_log_file_diff_split);
            column![
                container(file_list_view(detail, state.selected_file.as_deref()))
                    .height(Length::FillPortion(top_portion)),
                horizontal_divider_bar(RowDivider::GitLogFileDiffSplit, theme::color::BG, theme::color::BG),
                container(diff_pane_view(detail, state.selected_file.as_deref()))
                    .height(Length::FillPortion(bottom_portion)),
            ]
            .height(Length::Fill)
            .into()
        } else {
            container(text("选择一个提交查看改动").color(theme::color::DIM))
                .padding(12)
                .into()
        };

    row![
        container(left_with_picker).width(Length::FillPortion(list_portion)),
        crate::app::divider_bar(Divider::GitLogSplit, theme::color::BG, theme::color::BG),
        container(right).width(Length::FillPortion(content_portion)),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
```

（`crate::app::divider_bar`/`Divider` 目前是 `app.rs` 内的私有/`pub(crate)`
项——若编译报可见性错误,把 `app.rs` 里的 `fn divider_bar` 与 `enum Divider`
可见性从当前修饰符改成 `pub(crate)`,这是本任务允许的最小可见性放宽,不
需要额外任务。背景色用 `theme::color::BG`——已核实存在,且与现有
`divider_bar(Divider::LeftRight, theme::color::BG, theme::color::BG)`
调用点(`app.rs:5386`)同款;Todo/Project 面板那两处用的是
`theme::region::project_pane().background.unwrap_or(theme::color::BG)`
这种"读区域配色、无配置时落回 BG"的写法,Git Log 面板目前没有专属的
`theme::region::git_log_pane()` 配置项,直接用 `theme::color::BG` 更简单,
不需要为了统一写法而新增一个区域配置项。）

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app`
Expected: 编译通过,无 `dead_code` 警告残留(`commit_list_view`/
`file_list_view`/`diff_pane_view`/`branch_toggle_button`/`branch_picker_view`
全部被 `view()` 使用到)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(git-log): view() 重写为三栏可拖拽布局"
```

---

## Task 14: 内核(`app.rs`)接线——`BranchPickerOpen`/`BranchSwitch` 截获 + `view()` 调用点更新 + 分支切换后刷新

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(找到现有 `Message::GitLog` 分发的 `match` 块,参照 `docs/superpowers/specs/2026-08-07-git-log-extension-pilot-design.md` 第 6 节的既有结构追加两支;找到 `LeftView::GitLog => extensions::git_log::view(&app.git_log, ..)` 调用点,补上新参数)

**Interfaces:**
- Consumes: Task 9(`Message::{BranchPickerOpen, BranchSwitch}`)、Task 11(`self.dims.git_log_split`)、Task 12(`self.dims.git_log_file_diff_split`)、Task 13(`git_log::view` 新签名)、`delivery::{local_branches, checkout_branch}`(现有函数,不改)。

- [ ] **Step 1: 定位现有 `Message::GitLog` 分发代码**

Run: `grep -n "Message::GitLog" crates/dozer-app/src/app.rs`

找到类似(依 2026-08-07 spec 第 6 节)这样的结构:

```rust
Message::GitLog(extensions::git_log::Message::LoadMore) => { .. }
Message::GitLog(extensions::git_log::Message::ProjectTabOpen(path)) => { .. }
Message::GitLog(msg) => { .. 转发给 extensions::git_log::update .. }
```

- [ ] **Step 2: 追加 `BranchPickerOpen`(首次加载分支列表)分支**

在 `Message::GitLog(extensions::git_log::Message::LoadMore) => { .. }` 之后
插入:

```rust
Message::GitLog(extensions::git_log::Message::BranchPickerOpen) => {
    // 先把"展开"这个状态位落地(纯状态机部分仍走 update,不跳过),再判断
    // 要不要顺带发一次异步查询。
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::GitLog(m));
    };
    let needs_fetch = self.git_log.branches_is_empty(); // 见 Step 2b:State 需要新增这个只读判断
    extensions::git_log::update(
        &mut self.git_log,
        extensions::git_log::Message::BranchPickerOpen,
        &handle,
        emit.clone(),
    );
    if needs_fetch && let Some(repo_path) = self
        .active_workspace()
        .and_then(|ws| ws.active_project_path())
    {
        self.handle.spawn(async move {
            let repo_path2 = repo_path.clone();
            let branches = tokio::task::spawn_blocking(move || {
                crate::delivery::local_branches(&repo_path2).unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            emit(extensions::git_log::Message::BranchesLoaded(repo_path, branches));
        });
    }
}
```

`extensions::git_log::State` 需要新增一个只读判断方法(`git_log.rs`,
`impl State` 块里,`selected()` accessor 旁边):

```rust
/// 分支列表是否还没查过(`BranchPickerOpen` 首次展开时,内核据此判断要不要
/// 发起异步查询——避免每次展开都重新查一遍)。
pub fn branches_is_empty(&self) -> bool {
    self.branches.is_empty()
}
```

- [ ] **Step 3: 追加 `BranchSwitch` 分支**

紧邻上一步插入:

```rust
Message::GitLog(extensions::git_log::Message::BranchSwitch(name)) => {
    let Some(repo_path) = self.active_workspace().and_then(|ws| ws.active_project_path()) else {
        return;
    };
    self.git_log.set_branch_switch_pending(true); // 见下方 State 新增方法
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    self.handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            crate::delivery::checkout_branch(&repo_path2, &name)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        let _ = proxy.send_event(Message::GitLog(extensions::git_log::Message::BranchSwitchDone(result)));
    });
}
```

`extensions::git_log::State` 新增写方法(`branches_is_empty` 旁边):

```rust
pub(crate) fn set_branch_switch_pending(&mut self, pending: bool) {
    self.branch_switch_pending = pending;
}
```

- [ ] **Step 4: `Message::GitLog(msg) => { .. }` 通用转发分支追加"切换成功后重新拉 commit 列表"**

现有通用转发分支(2026-08-07 spec 第 6 节示例):

```rust
Message::GitLog(msg) => {
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::GitLog(m));
    };
    if let Some(next) = extensions::git_log::update(&mut self.git_log, msg, &handle, emit) {
        self.update(Message::GitLog(next));
    }
}
```

改成:检测这次处理的是不是 `BranchSwitchDone(Ok(()))`,是则额外触发一次
`request_refresh`:

```rust
Message::GitLog(msg) => {
    let is_branch_switch_success = matches!(
        &msg,
        extensions::git_log::Message::BranchSwitchDone(Ok(()))
    );
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::GitLog(m));
    };
    if let Some(next) = extensions::git_log::update(&mut self.git_log, msg, &handle, emit.clone()) {
        self.update(Message::GitLog(next));
    }
    if is_branch_switch_success && let Some(repo_path) = self
        .active_workspace()
        .and_then(|ws| ws.active_project_path())
    {
        let max_count = self.git_log.cache_max_count();
        extensions::git_log::request_refresh(&mut self.git_log, repo_path, max_count, &handle, emit);
    }
}
```

- [ ] **Step 5: 更新 `LeftView::GitLog` 的 `view()` 调用点**

找到(2026-08-07 spec 第 6 节):

```rust
LeftView::GitLog => extensions::git_log::view(&app.git_log).map(Message::GitLog),
```

改成:

```rust
LeftView::GitLog => extensions::git_log::view(
    &app.git_log,
    ws.project_panel.worktrees(),
    app.dims.git_log_split,
    app.dims.git_log_file_diff_split,
).map(Message::GitLog),
```

（若实际现有调用点参数顺序/`ws.project_panel.worktrees()` 写法与此不完全
一致,以 `grep -n "extensions::git_log::view" crates/dozer-app/src/app.rs`
查到的真实代码为准调整,只新增两个 split 参数,不改动其它既有参数传递
方式。）

- [ ] **Step 6: 全量编译确认**

Run: `cargo build --workspace`
Expected: 编译通过,无残留 `dead_code` 警告(全部新函数/字段都已被消费)。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/extensions/git_log.rs
git commit -m "feat(app): Git Log 分支切换/首次加载分支列表的内核截获接线 + view() 调用点更新"
```

---

## Task 15: 完整回归测试 + 人工验收

**Files:** 无代码改动,只跑验证。

- [ ] **Step 1: 完整自动化测试**

```bash
cargo build --workspace
cargo test -p dozer-app --bin dozer
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

Expected: 全部通过。若 `cargo fmt --check` 报格式问题,跑 `cargo fmt -p
dozer-app` 后重新 `git add`+追加一次 commit(`style: cargo fmt`)。

- [ ] **Step 2: 人工验收清单**(`cargo run -p dozer-app`,对照 spec"测试策略"
  第五段的验收清单逐条过)

1. 打开一个有 merge commit 历史的项目(Dozer 仓库自身即可,历史上已确认
   有 merge,如 `2429d15`),进 Git Log 面板,确认 commit 列表正常倒序展示、
   merge commit 图标(`git-merge`)跟普通提交图标(`git-commit-vertical`)
   视觉区分明显。
2. 点一个 commit → 右上文件列表刷新;点某个文件 → 右下 diff 内容刷新,
   染色正确(增删行颜色对,`+` 绿 `-` 红)。
3. 拖拽左右分割线(commit 列表宽度)、右侧上下分割线(文件列表/diff 高度),
   比例正常响应,不产生跳变/卡死;重开 app 后比例保持(持久化生效)。
4. 点分支下拉,切一个不同的分支:确认真的执行了 `git checkout`(用
   `git status`/`git branch --show-current` 在终端核对工作区确实切过去了)、
   commit 列表刷新成新分支的历史、当前有未提交改动时切换是否符合预期
   (人工制造一个 dirty 状态,确认非当前分支被禁用/无法点击)。
5. worktree 速览条位置(现在应该在标题下方、commit 列表上方)/点击切换到
   其它 worktree 的行为跟改造前一致。
6. 切项目/切 tab 后 Git Log 面板内容跟着对,不残留上一个项目的状态。

- [ ] **Step 3: 记录验收结果,若发现问题回退到对应 Task 修复**

若人工验收发现问题(比如纵向拖拽的"可用高度"估算不准导致光标跟手位置
明显偏移),回到 Task 12 的 `apply_row_drag` 调整估算公式,不需要重开新
Task——这是"待写计划时确认的实现细节"里已经预告过的已知风险点。

---

## 完成检查

- [ ] `CommitRow` 有 `time`/`is_merge` 字段,`build()` 正确填充,测试覆盖。
- [ ] `format_commit_time` 输出格式正确。
- [ ] `git-commit-vertical`/`git-merge` 图标已注册并可渲染。
- [ ] `diff_render::colored_diff_lines` 被 `acceptance.rs`/`git_log.rs` 共用,
  无重复实现。
- [ ] `git_log::State`/`Message` 新增"选中文件"状态机,测试覆盖切 commit 时
  的预选/清空逻辑。
- [ ] `GitLogCanvas` 已删除,commit 列表改用线性可滚动列表。
- [ ] 文件列表/diff 内容两个新面板正确渲染,选中态视觉一致。
- [ ] 分支切换状态机 + 局部 overlay 渲染完成,是真实 `git checkout`。
- [ ] 左右分割线(`Divider::GitLogSplit`)+ 右侧上下分割线(`RowDivider::
  GitLogFileDiffSplit`,仓库首条纵向拖拽)均可拖拽且持久化。
- [ ] 内核截获 `BranchPickerOpen`(首次)/`BranchSwitch`,切换成功后触发
  commit 列表刷新。
- [ ] `cargo build --workspace && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿。
- [ ] 人工验收清单(Task 15 Step 2)六条全部过。
- [ ] 全程在独立 worktree/分支上完成,经代码审阅后再合并 main。
</content>
