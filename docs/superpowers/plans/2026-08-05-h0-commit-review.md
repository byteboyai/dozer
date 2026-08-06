# H0 提交（`ac01e42`）代码审查（2026-08-05）

> 对照 `docs/superpowers/plans/2026-08-05-dozer-h0-project-center.md`（8 个任务）
> 和上游规格 `docs/superpowers/specs/2026-08-05-dozer-h0-project-center-design.md`
> 逐条核对 commit `ac01e42 feat(dozer-app): H0 项目中心落地页 + 顶栏 Dozer 页签化`。

## 结论先行

- **H0 本身的实现是对的**：`home_page`/`home_sidebar`/`home_recents_column`/
  `home_recent_files_card`/`home_recent_conversations_card`/`dozer_home_tab`/
  `load_home_recents`/`relative_time_text`/`h0_sidebar_width` 这些逻辑逐条对照
  plan 的 7 个实质任务(Task 1-7)，代码结构、字段命名、消息流转跟 plan 里写
  的几乎一致，没有发现逻辑 bug。新增的 4 个测试（`relative_time_text_boundaries`、
  `load_home_recents_empty_input_returns_empty_vecs`、
  `load_home_recents_merges_and_sorts_across_projects`、
  `load_home_recents_truncates_files_to_top_4`）全部通过。
- **但这个 commit 本身单独检出是编译不过的**——这是本轮审查最重要的发现，
  见下面"严重问题"。当前工作区看起来一切正常（`cargo build`/`test`/`clippy`/
  `fmt` 全绿），只是因为工作区里**还有一批未提交的改动**替它补上了缺口；
  一旦那批改动被提交、丢弃或这份 commit 被单独 cherry-pick/bisect，构建
  立刻失败。

## 严重问题：`ac01e42` 单独检出无法编译（已用隔离 worktree 实测验证）

用 `git worktree add --detach /tmp/h0-verify-worktree ac01e42` 单独检出这一个
commit（不碰当前工作目录，零风险）后跑 `cargo build -p dozer-app`，得到 5 个
编译错误：

```
error[E0432]: unresolved import `objc2_app_kit`
  --> crates/dozer-app/src/main.rs:52:9
error[E0432]: unresolved import `raw_window_handle`
  --> crates/dozer-app/src/main.rs:53:9
error[E0599]: no method named `reload_from_disk` found for mutable reference `&mut FileTree`
    --> crates/dozer-app/src/workspace.rs:3759:30
error[E0599]: no variant, associated function, or constant named `RefreshCw` found for enum `IconKind`
  --> crates/dozer-app/src/workspace.rs:6442:26
error[E0599]: no method named `window_handle` found for reference `&winit::window::Window`
  --> crates/dozer-app/src/main.rs:64:29
```

**根因**：这个 commit 只 stage 了 `workspace.rs`/`workspace_geometry.rs`/
`workspace.json` 三个文件，但当时 `workspace.rs` 里除了 H0 的改动之外，还**混
着另一批完全无关、当时尚未提交的改动**——图标栏 hover 态（`RailButton`/
`App.rail_hovered`/`MouseArea::on_enter/on_exit`）、项目树右键菜单"从磁盘重新
加载"（`Message::ProjectTreeReloadFromDisk` + 调 `tree.reload_from_disk()` +
菜单项用 `icons::IconKind::RefreshCw`）。这两块的"另一半"分别在 `project.rs`
（`FileTree::reload_from_disk` 方法）和 `icons.rs`（`RefreshCw` 变体）里，而这
两个文件**没有被这次提交带上**——它们连同 `main.rs` 需要的 `objc2`/
`objc2-app-kit`/`raw-window-handle` 依赖声明（`Cargo.toml`，这个是更早的
`e82a8ca` 就已经缺的老问题）目前都还躺在工作区里没提交。

这几块无关改动的具体内容、各自的实现质量（`reload_from_disk` 本身写得对、菜单
项挂错了地方、hover 态逻辑没抽成可测函数等）已经在
`docs/superpowers/plans/2026-08-05-pending-worktree-code-review.md` 里详细记
过，这里不重复；这份文档只记新发现的事实：**它们现在不是"顺手一起审"的旁支
了，而是让 `main` 当前 HEAD 保持可构建的硬依赖。**

**建议的修复顺序**（不需要改写/amend `ac01e42`，也不需要动 H0 的代码本身）：

1. 先把 `docs/superpowers/plans/2026-08-05-pending-worktree-code-review.md`
   里"严重问题 1"那 3 行 `Cargo.toml` 依赖单独提交。
2. 再把 `project.rs`（`reload_from_disk` 及其测试）单独提交。
3. 再把 `icons.rs` + `refresh-cw.svg`（`RefreshCw` 变体）单独提交——注意这一步
   完成后 `ac01e42` 引用的 `IconKind::RefreshCw` 才有对应定义。
4. 三步做完后用 `git log --oneline` 走一遍确认每一个 commit 单独检出都能
   `cargo build` 过（尤其是 `ac01e42` 本身，现在它排在这三个新提交**之前**，
   如果不想让历史上出现"这个 commit 单独编译不过"的空洞，需要考虑把这三个
   补丁 commit 变基到 `ac01e42` 之前，或者接受"只保证 HEAD 可构建，中间某个
   历史点编译不过"——这个取舍需要你来定，我不会擅自 rebase）。

## Task 逐条核对（对照 plan 的 8 个任务）

| Task | Plan 要求 | 实现情况 |
|---|---|---|
| 1 | `h0_sidebar_width` 几何 token | 完全一致（`workspace.json` +1 字段 +`workspace_geometry.rs` 访问函数 + 防漂移锚测试） |
| 2 | 抽出 `relative_time_text` | 完全一致，边界测试 7 个断言都在，跟 plan 给的代码逐字符一致 |
| 3 | `App.recent_projects` 字段 + 同步 | 完全一致（`bootstrap()`/`ProjectTabOpened` 两处同步点都有） |
| 4 | `HomeRecentFile`/`HomeRecentConversation` + `load_home_recents` | 逻辑一致；可见性从 plan 写的"私有"改成了 `pub(crate)`（见下"好的偏差"） |
| 5 | `Message::HomeRecentsLoaded` + `TopBarHome` 异步刷新 | 完全一致 |
| 6 | `dozer_home_tab` + 接入 `top_bar()` | 完全一致 |
| 7 | `home_page` 两栏重写 | 完全一致，四个辅助渲染函数（`home_sidebar`/`home_recents_column`/两张卡）拆分方式和 plan 给的代码结构一致 |
| 8 | 人工验收 + 全量校验 | **未见证据**——commit message/diff 里没有任何迹象表明跑过规格 §5 的人工验收清单；建议在宣布 H0 完成前实际跑一遍(尤其是"关掉所有项目页签回到空状态"这一条,纯读代码不容易看出是否真的不会 panic) |

## 好的偏差（不是问题，值得认可）

`HomeRecentFile`/`HomeRecentConversation` 声明成了 `pub(crate)` 而不是 plan
里写的纯私有 `struct`。原因是它们被塞进了 `pub enum Message` 的一个变体
（`HomeRecentsLoaded`），实现者在类型上加了注释"因此至少是 `pub(crate)`(见
`private_interfaces`)"——这是对 Rust 可见性规则比 plan 本身想得更周到的地方，
不是 bug，不需要改回去。

## 规格里已知的取舍（非新问题，只是确认实现忠实执行了决定）

- 侧栏项目卡不显示 git 分支——这是 plan 的 Global Constraints 里明确记过的
  "已知与人工验收草案的差异"，代码确实只画了名称/相对时间/路径三行，跟这条
  决定一致，不是遗漏。

## 尚未验证的部分

- 没有实际跑起 GUI 去看视觉效果（这次审查全靠读 diff + `cargo build/test/
  clippy/fmt`，没有截图验证）。H0 是纯 UI 落地页，建议至少手动过一遍规格 §5
  的 6 条人工验收清单再收尾。
