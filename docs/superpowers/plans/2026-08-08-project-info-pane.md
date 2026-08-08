# 项目信息(Project)面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增左侧 rail 可选的独立面板 `LeftView::Project`,展示项目名/git 分支+脏标/验收次数/目标(标题+标准列表,支持应用内编辑),并把现在挂在 `extensions::files::WorkspaceState` 上的 `branch`/`dirty`/`worktrees`/`project_acceptance_count`、以及顶栏(`top_bar`)的目标胶囊,统一归并到新建的 `extensions::project` 模块。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/project.rs`,拥有自己的 `Message`/`WorkspaceState`/`update`/`view`,全部同步(不接触终端会话域,不需要 `handle`/`emit`)。组合 git 刷新(分支/脏/文件状态/worktree)保留在内核,做成一个不属于任何 extension 的自由函数 `spawn_project_git_refresh`,算完一次后分发成两条独立消息:`Message::Files(files::Message::StatusesRefreshed)`(只带文件级状态)与 `Message::Project(project::Message::GitRefreshed)`(带分支/脏/worktree)。`extensions::files` 删除 `branch`/`dirty`/`worktrees`/`project_acceptance_count` 四个字段与对应两条消息,只留文件树本体。`Workspace` 新增字段 `project_panel: project::WorkspaceState`(不能叫 `project`——那个名字已经是 `Option<ProjectInfo>` 字段)。

**Tech Stack:** Rust workspace;iced 0.14;不新增依赖。

## Global Constraints

- 这是阶段 1 扩展化重构第七个试点,但夹带一次真实的产品级改动:目标从"顶栏只读
  胶囊"升级成"面板内可编辑"——已经过 brainstorming 确认,除此之外(验收/Todo/
  Files 三个面板已有的业务逻辑)一律不变。
- 状态归属判断标准是"谁展示就归谁",不是"谁先算出来的"——`branch`/`dirty`/
  `worktrees` 原本因为 Files 是唯一消费方而挂在 `files::WorkspaceState`,这次因为
  Project 面板也要展示,必须重新按展示方切分归属。
- 多消费方共享的组合数据,编排权收归内核:`spawn_project_git_refresh` 是一个不属于
  任何 extension 的自由函数,算完一次组合 git 查询后分发给两个互不知道对方存在的
  extension。
- `extensions::project` 与 `extensions::files`/`extensions::acceptance` 等其它
  extension 之间不得有任何直接类型依赖——只能通过内核转发的消息通信。
- 不做分支切换(checkout)——现状完全没有,已确认留到以后单独一轮设计,这次只把
  `branch`/`dirty` 挪个地方展示。
- 保存目标时整份重新生成 `.dozer/goal.md`,不做增量式保留其它手写内容的 patch。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过(Task 5 是内核接线的中间态,允许编译报错,见该任务说明)。
- 设计文档:`docs/superpowers/specs/2026-08-08-project-info-pane-design.md`(有疑问以它为准;下面两处地方设计文档留白,已在对应任务里补齐并标注:①`files::Message::StatusesRefreshed` 需要带 `project_id` 才能正确路由,设计文档的签名遗漏了它;②`Workspace` 存新状态的字段名设计文档未给出,`project` 已被占用,定为 `project_panel`)。

---

### Task 1: `goal::write_goal`——整份重写 `.dozer/goal.md`

**Files:**
- Modify: `crates/dozer-app/src/goal.rs`

**Interfaces:**
- Produces:`pub fn write_goal(repo: &Path, goal: &Goal) -> std::io::Result<()>`。

- [ ] **Step 1: 写失败的测试**

`goal.rs` 现有 `#[cfg(test)] mod tests`(38 行起)。加三个测试:

```rust
#[test]
fn write_goal_creates_dozer_dir_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let goal = Goal {
        title: "标题".into(),
        criteria: vec!["标准一".into()],
    };
    write_goal(dir.path(), &goal).unwrap();
    let content = std::fs::read_to_string(goal_path(dir.path())).unwrap();
    assert_eq!(content, "# 标题\n\n- [ ] 标准一\n");
}

#[test]
fn write_goal_overwrites_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let g1 = Goal {
        title: "旧标题".into(),
        criteria: vec!["旧标准".into()],
    };
    write_goal(dir.path(), &g1).unwrap();
    let g2 = Goal {
        title: "新标题".into(),
        criteria: vec![],
    };
    write_goal(dir.path(), &g2).unwrap();
    let content = std::fs::read_to_string(goal_path(dir.path())).unwrap();
    assert_eq!(content, "# 新标题\n\n");
}

#[test]
fn write_then_parse_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let goal = Goal {
        title: "圆环测试".into(),
        criteria: vec!["一".into(), "二".into()],
    };
    write_goal(dir.path(), &goal).unwrap();
    let content = std::fs::read_to_string(goal_path(dir.path())).unwrap();
    let parsed = parse_goal(&content).unwrap();
    assert_eq!(parsed, goal);
}
```

`dozer-app` 的 `Cargo.toml` 已经把 `tempfile` 列为 dev-dependency(`extensions/files.rs`/
`delivery.rs` 的测试都在用),不需要新增依赖。

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozer-app write_goal -- --nocapture
```

Expected: 编译失败(`write_goal` 未定义)。

- [ ] **Step 3: 实现 `write_goal`**

紧接着 `parse_goal` 函数之后加:

```rust
/// 按固定格式整份重写 `.dozer/goal.md`:首行 `# {title}`,空行,然后每条
/// 标准各占一行 `- [ ] {criterion}`。不保留/不合并文件里其它手写内容——
/// `parse_goal` 本来就只认标题行与 `- [ ]`/`- [x]` 列表项这两种语义,重新
/// 生成不算破坏数据(见设计文档"非目标")。`.dozer` 目录不存在则先创建。
pub fn write_goal(repo: &Path, goal: &Goal) -> std::io::Result<()> {
    let path = goal_path(repo);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut md = format!("# {}\n\n", goal.title);
    for c in &goal.criteria {
        md.push_str(&format!("- [ ] {c}\n"));
    }
    std::fs::write(path, md)
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app write_goal
cargo test -p dozer-app write_then_parse_roundtrip
```

Expected: 三个新测试 PASS。

- [ ] **Step 5: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/goal.rs
git commit -m "feat(dozer-app): add goal::write_goal to regenerate .dozer/goal.md"
```

---

### Task 2: 新图标 `IconKind::Info`

**Files:**
- Create: `crates/dozer-app/assets/icons/info.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces:`icons::IconKind::Info`,可传给现有 `icons::view(kind, size, color)`。

- [ ] **Step 1: 加 SVG 资源**

创建 `crates/dozer-app/assets/icons/info.svg`(Lucide `info`,与仓库现有图标同款
24×24 `currentColor` 描边格式):

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <circle cx="12" cy="12" r="10" />
  <path d="M12 16v-4" />
  <path d="M12 8h.01" />
</svg>
```

- [ ] **Step 2: 加枚举变体**

`crates/dozer-app/src/icons.rs` 的 `IconKind` 枚举,在 `ListChecks`(43 行)之后插入:

```rust
ListChecks,
/// 项目信息面板 rail 图标(Lucide info)。
Info,
```

`IconKind::bytes()` 的 `match` 里,在 `IconKind::ListChecks => ...`(97 行)之后加:

```rust
IconKind::ListChecks => include_bytes!("../assets/icons/list-checks.svg"),
IconKind::Info => include_bytes!("../assets/icons/info.svg"),
```

- [ ] **Step 3: 编译确认没有漏改的 match 分支**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/icons/info.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): add Info icon for the project info pane"
```

---

### Task 3: `extensions::project`——类型骨架 + `update` + `load_goal`

**Files:**
- Create: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod project;`)

**Interfaces:**
- Produces:`pub struct WorkspaceState`(`new`/`worktrees`/`title_editing_is_some`/
  `cancel_title_edit`)、`pub enum Message`、`pub fn update(..)`、
  `pub fn load_goal(repo_path: &Path) -> Option<Goal>`。

- [ ] **Step 1: 类型定义**

创建 `crates/dozer-app/src/extensions/project.rs`:

```rust
//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数、目标(标题+
//! 标准列表,支持应用内编辑)。阶段 1 扩展化重构第七个试点,设计见
//! `docs/superpowers/specs/2026-08-08-project-info-pane-design.md`。
use crate::delivery::WorktreeInfo;
use crate::goal::{self, Goal};
use crate::workspace::AddrEvent;
use std::path::Path;

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    goal: Option<Goal>,
    /// 目标标题的行内编辑态(None=未在编辑;创建新目标时也复用这个字段,
    /// `goal` 为 `None` 且 `title_editing` 有值就是"正在设置首个目标")。
    title_editing: Option<String>,
    /// 新增标准的输入草稿。
    add_criterion_draft: String,
    error: Option<String>,
}

impl WorkspaceState {
    /// 打开一个新项目时构造(`goal` 由调用方通过 `load_goal` 同步读一次
    /// `.dozer/goal.md` 拿到,传进来)。
    pub fn new(goal: Option<Goal>) -> Self {
        Self {
            goal,
            ..Self::default()
        }
    }

    /// 供内核 `worktree_strip`(Git Log 视图外层装饰,不属于
    /// `extensions::git_log`)读取——`worktrees` 数据来自组合 git 刷新,但
    /// 消费方是 Git Log 视图,归属判断见设计文档"关键语义确认"。
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }

    /// 供内核 main.rs 键盘路由判断"标题是否在自绘编辑态"。
    pub fn title_editing_is_some(&self) -> bool {
        self.title_editing.is_some()
    }

    /// 供内核 `Workspace::blur_inputs` 调用——点击输入框外时取消标题编辑
    /// (不保存半输入,同 Files 试点项目树编辑取消的处理口径)。
    pub fn cancel_title_edit(&mut self) {
        self.title_editing = None;
    }
}

/// 同步读一次 `.dozer/goal.md`(文件极小,可容忍同步读)。现有
/// `workspace.rs::load_project_goal` 的搬家版本,仅参数类型从 `&str` 改成
/// `&Path`。供内核在项目打开(`from_restore`/`loading_for_project`)/
/// `adopt_project` 时调用,结果传给 `WorkspaceState::new`。
pub fn load_goal(repo_path: &Path) -> Option<Goal> {
    let md = std::fs::read_to_string(goal::goal_path(repo_path)).ok()?;
    goal::parse_goal(&md)
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`worktrees`)、
/// 验收次数、标题/标准列表的应用内编辑。`GitRefreshed`/`AcceptanceCountLoaded`
/// 由内核分发,带 `project_id`,走 `with_project`;其余是用户交互消息,走
/// `with_focused_project`。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>),
    AcceptanceCountLoaded(i64, Option<u64>),
    TitleEditStart,
    TitleEditEvent(AddrEvent),
    CriterionAddInputChanged(String),
    CriterionAddSubmit,
    CriterionRemove(usize),
}

/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files/Acceptance),全部同步完成,不需要
/// `handle`/`emit`。
pub fn update(ws_state: &mut WorkspaceState, msg: Message, _project_id: i64, repo_path: &Path) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, worktrees) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.worktrees = worktrees;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
        Message::TitleEditStart => {
            ws_state.title_editing = Some(
                ws_state
                    .goal
                    .as_ref()
                    .map(|g| g.title.clone())
                    .unwrap_or_default(),
            );
        }
        Message::TitleEditEvent(ev) => match ev {
            AddrEvent::Text(s) => {
                if let Some(buf) = &mut ws_state.title_editing {
                    buf.push_str(&s);
                }
            }
            AddrEvent::Backspace => {
                if let Some(buf) = &mut ws_state.title_editing {
                    buf.pop();
                }
            }
            AddrEvent::Cancel => ws_state.title_editing = None,
            AddrEvent::Submit => {
                let Some(raw) = ws_state.title_editing.take() else {
                    return;
                };
                let title = raw.trim().to_string();
                if title.is_empty() {
                    // `take()` 已经关闭编辑框——空标题视为取消,同 Files
                    // 试点"提交空名字关闭编辑框而非保留"的既有处理口径。
                    return;
                }
                let goal = match &ws_state.goal {
                    Some(g) => Goal {
                        title,
                        criteria: g.criteria.clone(),
                    },
                    None => Goal {
                        title,
                        criteria: Vec::new(),
                    },
                };
                match goal::write_goal(repo_path, &goal) {
                    Ok(()) => {
                        ws_state.error = None;
                        ws_state.goal = Some(goal);
                    }
                    Err(e) => {
                        ws_state.error = Some(format!("保存失败: {e}"));
                        // 写盘失败,保留原始输入(未 trim)允许重试。
                        ws_state.title_editing = Some(raw);
                    }
                }
            }
        },
        Message::CriterionAddInputChanged(s) => ws_state.add_criterion_draft = s,
        Message::CriterionAddSubmit => {
            let text = ws_state.add_criterion_draft.trim().to_string();
            if text.is_empty() {
                ws_state.add_criterion_draft = String::new();
                return;
            }
            let Some(goal) = &mut ws_state.goal else {
                return;
            };
            goal.criteria.push(text);
            match goal::write_goal(repo_path, goal) {
                Ok(()) => {
                    ws_state.error = None;
                    ws_state.add_criterion_draft = String::new();
                }
                Err(e) => {
                    goal.criteria.pop();
                    ws_state.error = Some(format!("保存失败: {e}"));
                    // `add_criterion_draft` 不清空,允许重试。
                }
            }
        }
        Message::CriterionRemove(i) => {
            let Some(goal) = &mut ws_state.goal else {
                return;
            };
            if i >= goal.criteria.len() {
                return;
            }
            let removed = goal.criteria.remove(i);
            if let Err(e) = goal::write_goal(repo_path, goal) {
                goal.criteria.insert(i, removed);
                ws_state.error = Some(format!("保存失败: {e}"));
            } else {
                ws_state.error = None;
            }
        }
    }
}
```

- [ ] **Step 2: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入(在 `git_log`/`todo` 之间):

```rust
pub mod acceptance;
pub mod browser;
pub mod files;
pub mod git_log;
pub mod project;
pub mod todo;
pub mod usage;
```

- [ ] **Step 3: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`update`/`load_goal`/`WorkspaceState` 目前未被内核调用,
`dead_code` 警告可接受,不允许报错。

- [ ] **Step 4: 新增单测**

文件末尾加 `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ws_with_goal(goal: Option<Goal>) -> WorkspaceState {
        WorkspaceState::new(goal)
    }

    #[test]
    fn title_edit_start_prefills_existing_title() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "旧标题".into(),
            criteria: vec![],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        assert_eq!(ws.title_editing.as_deref(), Some("旧标题"));
    }

    #[test]
    fn title_edit_start_empty_when_no_goal() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        assert_eq!(ws.title_editing.as_deref(), Some(""));
    }

    #[test]
    fn title_edit_submit_creates_new_goal_and_writes_disk() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Text("新目标".into())),
            1,
            repo.path(),
        );
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Submit),
            1,
            repo.path(),
        );
        assert!(ws.title_editing.is_none());
        assert_eq!(ws.goal.as_ref().unwrap().title, "新目标");
        let content = std::fs::read_to_string(goal::goal_path(repo.path())).unwrap();
        assert!(content.starts_with("# 新目标"));
    }

    #[test]
    fn title_edit_submit_updates_title_keeps_criteria() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "旧".into(),
            criteria: vec!["标准".into()],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Backspace),
            1,
            repo.path(),
        );
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Text("新".into())),
            1,
            repo.path(),
        );
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Submit),
            1,
            repo.path(),
        );
        let g = ws.goal.as_ref().unwrap();
        assert_eq!(g.title, "新");
        assert_eq!(g.criteria, vec!["标准".to_string()]);
    }

    #[test]
    fn title_edit_submit_empty_closes_without_writing() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Submit),
            1,
            repo.path(),
        );
        assert!(ws.title_editing.is_none());
        assert!(ws.goal.is_none());
        assert!(!goal::goal_path(repo.path()).exists());
    }

    #[test]
    fn criterion_add_submit_appends_and_writes_disk() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "标题".into(),
            criteria: vec![],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(
            &mut ws,
            Message::CriterionAddInputChanged("新标准".into()),
            1,
            repo.path(),
        );
        update(&mut ws, Message::CriterionAddSubmit, 1, repo.path());
        assert_eq!(
            ws.goal.as_ref().unwrap().criteria,
            vec!["新标准".to_string()]
        );
        assert_eq!(ws.add_criterion_draft, "");
        let content = std::fs::read_to_string(goal::goal_path(repo.path())).unwrap();
        assert!(content.contains("- [ ] 新标准"));
    }

    #[test]
    fn criterion_add_submit_empty_draft_is_noop() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "标题".into(),
            criteria: vec![],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::CriterionAddSubmit, 1, repo.path());
        assert!(ws.goal.as_ref().unwrap().criteria.is_empty());
    }

    #[test]
    fn criterion_remove_deletes_and_writes_disk() {
        let g = Goal {
            title: "标题".into(),
            criteria: vec!["a".into(), "b".into()],
        };
        let repo = tempfile::tempdir().unwrap();
        goal::write_goal(repo.path(), &g).unwrap();
        let mut ws = ws_with_goal(Some(g));
        update(&mut ws, Message::CriterionRemove(0), 1, repo.path());
        assert_eq!(ws.goal.as_ref().unwrap().criteria, vec!["b".to_string()]);
        let content = std::fs::read_to_string(goal::goal_path(repo.path())).unwrap();
        assert!(!content.contains("- [ ] a"));
        assert!(content.contains("- [ ] b"));
    }

    #[test]
    fn git_refreshed_updates_three_fields() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
            1,
            repo.path(),
        );
        assert_eq!(ws.branch.as_deref(), Some("main"));
        assert!(ws.dirty);
        assert_eq!(ws.worktrees().len(), 0);
    }

    #[test]
    fn acceptance_count_loaded_sets_field() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(
            &mut ws,
            Message::AcceptanceCountLoaded(1, Some(3)),
            1,
            repo.path(),
        );
        assert_eq!(ws.project_acceptance_count, Some(3));
    }

    #[test]
    fn load_goal_reads_existing_file() {
        let repo = tempfile::tempdir().unwrap();
        let g = Goal {
            title: "读回".into(),
            criteria: vec!["一".into()],
        };
        goal::write_goal(repo.path(), &g).unwrap();
        assert_eq!(load_goal(repo.path()), Some(g));
    }

    #[test]
    fn load_goal_missing_file_is_none() {
        let repo = tempfile::tempdir().unwrap();
        assert_eq!(load_goal(repo.path()), None);
    }
}
```

- [ ] **Step 5: 跑测试**

```bash
cargo test -p dozer-app extensions::project::
```

Expected: 全部新增测试 PASS。

- [ ] **Step 6: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add project Message/WorkspaceState/update/load_goal"
```

---

### Task 4: `extensions::project::view`

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`

**Interfaces:**
- Consumes:Task 3 的 `WorkspaceState`/`Message`。
- Produces:`pub fn view<'a>(ws_state: &'a WorkspaceState, project: Option<&'a
  ProjectInfo>, width: Length, outer: Border) -> Element<'a, Message,
  iced_widget::Theme, iced_widget::Renderer>`。

- [ ] **Step 1: 文件顶部补齐 import**

在现有 `use` 块基础上加:

```rust
use crate::{icons, theme};
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, text, text_input};
```

- [ ] **Step 2: 搬 `project_branch_label`/`goal_capsule_text` 两个纯函数**

从 `crates/dozer-app/src/workspace.rs` 的 7059/7069 行搬过来(定义与断言原样不变,
只搬文件位置——这两个函数目前只有 `extensions/files.rs`(即将在 Task 6 删除对应
调用点)与顶栏胶囊(即将在 Task 6 删除)在用,搬完之后本文件是唯一归属地):

```rust
/// 项目卡分支标签:`分支` / `分支*`(脏)/ `—`(非 git)。
fn project_branch_label(branch: Option<&str>, dirty: bool) -> String {
    match branch {
        Some(b) if dirty => format!("{b}*"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    }
}

/// 目标标题文案:`目标：{标题}`;标题过长按字符截断加省略号。无 goal 或空
/// 标题 → None。`max_chars` 含省略号占位。现有 `workspace.rs` 顶栏胶囊同名
/// 函数的搬家版本,断言不变——原顶栏用途已删除(设计文档目标 #6),这次复用
/// 给面板内标题按钮展示。
fn goal_capsule_text(goal: Option<&Goal>, max_chars: usize) -> Option<String> {
    let title = goal?.title.trim();
    if title.is_empty() {
        return None;
    }
    let shown = if title.chars().count() > max_chars {
        let mut s: String = title.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        title.to_string()
    };
    Some(format!("目标：{shown}"))
}
```

- [ ] **Step 3: `view` 主体**

```rust
/// 面板主入口(单栏,不与任何其它面板配对——同 GitLog/Usage)。`project` 为
/// `None` 时内核不会真正走到这里(`left_panel_area` 对 `LeftView::Project`
/// 无条件调用本函数,但 `App::view()` 顶层只在有聚焦项目时才会渲染到这个
/// 分支),这里仍保留一次防御性判断,风格对齐 Files 试点。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project: Option<&'a ProjectInfo>,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(p) = project else {
        return container(iced_widget::Space::new())
            .width(width)
            .height(Length::Fill)
            .into();
    };

    let mut content = column![].spacing(12).padding(14);

    content = content.push(
        text(p.name.clone())
            .size(theme::font::title())
            .color(theme::color::CREAM),
    );

    let label = project_branch_label(ws_state.branch.as_deref(), ws_state.dirty);
    let bcolor = if ws_state.dirty {
        theme::color::GOLD
    } else {
        theme::color::BODY
    };
    content = content.push(
        row![
            icons::view(
                icons::IconKind::GitBranch,
                crate::theme::icon_size::row(),
                bcolor
            ),
            text(label).size(theme::font::label()).color(bcolor),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        content = content.push(
            text(format!("{n} 次验收"))
                .size(theme::font::caption())
                .color(theme::color::GOLD),
        );
    }

    content = content.push(goal_block(ws_state));

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(theme::color::PANEL.into()),
            border: outer,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// 目标区块:标题行(点击进入编辑,编辑态下换成自绘输入框)+ 有目标时逐条
/// 列出标准(各带删除按钮)+ "＋新增标准"输入框(复用 Todo 面板"加一条"的
/// 既有交互形状,原生 `text_input`);没有目标时只显示"未定标"入口,不渲染
/// 标准列表/输入框(点击进入同一个标题编辑态,创建首个目标)。
fn goal_block(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![].spacing(8);

    let title_row: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
        if let Some(buf) = &ws_state.title_editing {
            container(
                text(format!("{buf}▏"))
                    .size(theme::font::title())
                    .color(theme::color::CREAM),
            )
            .padding([2, 4])
            .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::CARD.into()),
                border: Border {
                    color: theme::color::CREAM,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..iced_widget::container::Style::default()
            })
            .into()
        } else if let Some(g) = &ws_state.goal {
            let title_text =
                goal_capsule_text(Some(g), 60).unwrap_or_else(|| "（未命名目标）".to_string());
            button(
                text(title_text)
                    .size(theme::font::title())
                    .color(theme::color::CREAM),
            )
            .on_press(Message::TitleEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: theme::color::CREAM,
                ..iced_widget::button::Style::default()
            })
            .into()
        } else {
            button(
                text("未定标 · 点击设置目标")
                    .size(theme::font::body())
                    .color(theme::color::DIM),
            )
            .on_press(Message::TitleEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: theme::color::DIM,
                ..iced_widget::button::Style::default()
            })
            .into()
        };
    col = col.push(title_row);

    if let Some(g) = &ws_state.goal {
        for (i, c) in g.criteria.iter().enumerate() {
            col = col.push(
                row![
                    text(format!("· {c}"))
                        .size(theme::font::body())
                        .color(theme::color::BODY),
                    iced_widget::space::horizontal(),
                    button(text("×").size(theme::font::body()).color(theme::color::DIM))
                        .on_press(Message::CriterionRemove(i))
                        .style(|_t, _s| iced_widget::button::Style {
                            background: None,
                            text_color: theme::color::DIM,
                            ..iced_widget::button::Style::default()
                        }),
                ]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center),
            );
        }
        col = col.push(
            text_input("＋新增标准…", &ws_state.add_criterion_draft)
                .on_input(Message::CriterionAddInputChanged)
                .on_submit(Message::CriterionAddSubmit)
                .size(theme::font::body())
                .padding([8, 10])
                .style(
                    |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                        background: theme::color::BG.into(),
                        border: Border {
                            color: theme::color::BORDER,
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        icon: theme::color::DIM,
                        placeholder: theme::color::DIM,
                        value: theme::color::CREAM,
                        selection: theme::color::GOLD,
                    },
                ),
        );
    }

    if let Some(err) = &ws_state.error {
        col = col.push(
            text(format!("⚠ {err}"))
                .size(theme::font::label())
                .color(theme::color::RED),
        );
    }

    col.into()
}
```

- [ ] **Step 4: 补两个搬过来的测试**

在 Task 3 已建好的 `mod tests` 里加(断言与 `workspace.rs` 现有版本逐字一致):

```rust
#[test]
fn project_card_branch_label() {
    assert_eq!(project_branch_label(Some("main"), false), "main");
    assert_eq!(project_branch_label(Some("main"), true), "main*");
    assert_eq!(project_branch_label(None, false), "—");
}

#[test]
fn goal_capsule_prefixes_and_truncates() {
    let g = Goal {
        title: "会话存活 daemon 雏形".into(),
        criteria: vec![],
    };
    assert_eq!(
        goal_capsule_text(Some(&g), 100).as_deref(),
        Some("目标：会话存活 daemon 雏形")
    );
    let long = Goal {
        title: "一二三四五六七八九十".into(),
        criteria: vec![],
    };
    assert_eq!(
        goal_capsule_text(Some(&long), 5).as_deref(),
        Some("目标：一二三四…")
    );
    assert_eq!(goal_capsule_text(None, 10), None);
    let empty = Goal {
        title: "   ".into(),
        criteria: vec![],
    };
    assert_eq!(goal_capsule_text(Some(&empty), 10), None);
}
```

- [ ] **Step 5: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 逐条修正类型/字段名不一致的地方(如 `theme::color`/`theme::font` 的具体
常量名要跟 `crates/dozer-app/src/theme` 模块实际导出的对上)。

- [ ] **Step 6: 跑测试 + clippy + fmt**

```bash
cargo test -p dozer-app extensions::project::
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs
git commit -m "feat(dozer-app): add project::view with inline goal editing"
```

---

### Task 5: 内核接线 A——枚举/字段/模块引入/rail 图标(允许中间态编译失败)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 2 的 `icons::IconKind::Info`,Task 3 的 `project::WorkspaceState`/
  `project::Message`。

- [ ] **Step 1: `use` 引入**

`workspace.rs` 顶部按字母序加(在 `git_log`/`todo` 之间):

```rust
use crate::extensions::git_log;
use crate::extensions::project;
use crate::extensions::todo;
```

- [ ] **Step 2: `LeftView`/`RailButton` 新增变体**

`LeftView` 枚举(现约 84 行)加 `Project`:

```rust
pub enum LeftView {
    Files,
    Web,
    GitLog,
    Todo,
    Project,
}
```

`RailButton` 枚举(现约 104 行)加 `LeftProject`:

```rust
pub enum RailButton {
    LeftFiles,
    LeftWeb,
    LeftGit,
    LeftTodo,
    LeftProject,
    RightAgent,
    RightConversations,
    RightUsage,
    RightAcceptance,
}
```

- [ ] **Step 3: `Workspace` 字段替换**

`pub struct Workspace { .. }` 里(现约 1325-1326 行)删除:

```rust
/// 顶栏胶囊用的项目级目标（打开项目时同步读 .dozer/goal.md）。
project_goal: Option<Goal>,
```

加(**不能叫 `project`**——那个名字已经是 `project: Option<ProjectInfo>` 字段,设计
文档没给出这个字段名,这里定为 `project_panel`):

```rust
/// Project 信息面板 per-project 状态——见 `extensions::project::WorkspaceState`。
project_panel: project::WorkspaceState,
```

- [ ] **Step 4: 顶层 `Message` 枚举新增变体**

在 `Files(files::Message),`(现约 1054 行)之后加:

```rust
/// Files 面板的全部消息,内核只转发不解读——见 `extensions::files::Message`。
Files(files::Message),
/// Project 信息面板的全部消息,内核只转发不解读——见
/// `extensions::project::Message`。
Project(project::Message),
```

`ProjectFsChanged` 的文档注释(现约 1044-1048 行)提到"这条消息同时喂给
Files...和 Git Log...两个独立扩展",实际上 Task 6 会让它同时喂给 Files/Project/
Git Log 三个,顺手更新一下措辞:

```rust
/// 项目:`git_watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新
/// 了(D4)。`Relevance` 决定这次触发要不要顺带做 Plan 2 的 Git Log 快照
/// 重建。这条消息同时喂给 Files(刷新 git_statuses)、Project(刷新
/// branch/dirty/worktrees)和 Git Log(条件触发快照重建)三个独立扩展,
/// 内核继续拦截、分别转发,不包进任何一个 extension 的 `Message`。
ProjectFsChanged(ProjectId, git_watch::Relevance),
```

- [ ] **Step 5: `left_icon_rail` 新增第 5 个按钮**

现有 4 个按钮(约 5630-5665 行)之后加第 5 个:

```rust
MouseArea::new(rail_icon_button(
    icons::IconKind::Info,
    app.left_view == LeftView::Project && left_open,
    app.hover_progress(HoverId::Rail(RailButton::LeftProject)),
    Message::LeftIconSelect(LeftView::Project),
))
.on_enter(Message::Hover(HoverId::Rail(RailButton::LeftProject), true))
.on_exit(Message::Hover(HoverId::Rail(RailButton::LeftProject), false)),
```

（插入位置在 `column![ .. ]` 宏内 `LeftTodo` 那个 `MouseArea` 之后,别忘了给前一项
补逗号。）

- [ ] **Step 6: 编译确认(允许报错)**

```bash
cargo build -p dozer-app
```

Expected: 报一堆错误——`LeftView`/`RailButton` 新增变体后,所有穷举 `match` 的地方
缺分支;`ws.project_goal`/`self.project_goal` 的引用点找不到字段;`Workspace` 的
几处构造点(`empty_for_project_placeholder`/`from_restore`/`loading_for_project`/
`adopt_project`)缺 `project_panel` 字段初始化。**这些全部留到 Task 6 逐条修正,
本任务先确认到这一步即可**——中间状态提交。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): add LeftView::Project rail icon and Message::Project wrapper"
```

（这一步允许 `cargo build` 仍然报错——中间状态提交,Task 6 会继续改到编译通过。
如果你的工作流不允许中间状态提交,合并 Task 5/6 一次性做完再提交。）

---

### Task 6: 内核接线 B——组合 git 刷新分发 + Files 瘦身 + 顶栏胶囊删除

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes:Task 3 的 `project::WorkspaceState`/`project::Message`/`project::update`/
  `project::load_goal`,Task 4 的 `project::view`。
- Produces:`files::Message::StatusesRefreshed(i64, HashMap<PathBuf,
  FileGitStatus>)`——**注意这里带了 `project_id`,设计文档给的签名
  `StatusesRefreshed(HashMap<..>)` 漏了它**:这条消息是组合 git 刷新的异步结果,
  必须能按 `project_id` 走 `with_project` 路由,不能走 `with_focused_project`
  (同现有 `PasteDone`/`OpDone` 的路由方式),否则用户切换项目页签期间刷新结果
  会串到错误的项目上。

#### Part A:`extensions/files.rs` 瘦身

- [ ] **Step 1: `WorkspaceState` 删字段**

删除 `branch`/`dirty`/`worktrees`/`project_acceptance_count` 四个字段,只留:

```rust
#[derive(Default)]
pub struct WorkspaceState {
    file_tree: Option<FileTree>,
    git_statuses: HashMap<PathBuf, FileGitStatus>,
    tree_selected: Option<PathBuf>,
    tree_clipboard: Option<(PathBuf, bool)>,
    tree_error: Option<String>,
    tree_delete_confirm: Option<(PathBuf, bool)>,
    tree_edit: Option<TreeEdit>,
}
```

删除 `worktrees()` 访问器(整个方法)。

`reset_for_project` 删掉 `branch`/`dirty`/`project_acceptance_count` 三行赋值:

```rust
pub fn reset_for_project(&mut self, file_tree: FileTree) {
    self.file_tree = Some(file_tree);
    self.tree_selected = None;
    self.git_statuses = HashMap::new();
}
```

- [ ] **Step 2: `Message` 枚举替换两个变体**

删除 `GitRefreshed(..)`/`AcceptanceCountLoaded(..)`,加:

```rust
/// 组合 git 刷新结果里跟 Files 有关的部分(只剩文件级状态,`branch`/
/// `dirty`/`worktrees` 已经归 `extensions::project`)。带 `project_id`,
/// 走 `with_project` 路由(异步结果消息的既有约束)。
StatusesRefreshed(i64, HashMap<PathBuf, FileGitStatus>),
```

- [ ] **Step 3: `update` 对应分支替换**

```rust
Message::StatusesRefreshed(_, statuses) => {
    ws_state.git_statuses = statuses;
}
```

（原来的 `Message::GitRefreshed`/`Message::AcceptanceCountLoaded` 两个分支删掉。）

- [ ] **Step 4: 删除 `spawn_git_refresh`**

整个 `pub fn spawn_git_refresh(..)` 函数删除——组合刷新的查询逻辑搬进 Task 6 Part B
新增的内核自由函数,不再委托给这个模块。

- [ ] **Step 5: `view` 签名瘦身,删除项目信息卡与底部状态条**

新签名:

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let mut tree_col = column![].spacing(region.gap);

    if let Some(err) = &ws_state.tree_error {
        header = header.push(
            text(format!("⚠ {err}"))
                .size(theme::font::label())
                .color(theme::color::RED),
        );
    }
    if let Some(tree) = &ws_state.file_tree {
        for row in tree.visible_rows() {
            let is_renaming = matches!(
                &ws_state.tree_edit,
                Some(TreeEdit { mode: TreeEditMode::Rename(p), .. }) if *p == row.path
            );
            if is_renaming {
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth, buffer));
                continue;
            }
            let indent = "  ".repeat(row.depth);
            let status: Option<(delivery::ChangeKind, bool)> = if row.is_dir {
                delivery::dir_status(&row.path, &ws_state.git_statuses)
                    .map(|d| (d.kind, d.unstaged))
            } else {
                ws_state
                    .git_statuses
                    .get(&row.path)
                    .map(|s| (s.kind, s.unstaged))
            };
            let name_color = theme::color::BODY;
            let row_icon: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
                if row.is_dir {
                    let chevron = if row.expanded {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    };
                    let folder = if row.expanded {
                        icons::IconKind::FolderOpen
                    } else {
                        icons::IconKind::Folder
                    };
                    row![
                        icons::view(
                            chevron,
                            crate::theme::icon_size::chevron(),
                            theme::color::DIM
                        ),
                        icons::view(folder, crate::theme::icon_size::row(), theme::color::DIM),
                    ]
                    .spacing(crate::theme::icon_size::tree_row_gap())
                    .align_y(iced_widget::core::Alignment::Center)
                    .into()
                } else {
                    row![
                        iced_widget::space::Space::new()
                            .width(Length::Fixed(
                                crate::theme::icon_size::chevron()
                                    + crate::theme::icon_size::tree_row_gap(),
                            ))
                            .height(Length::Shrink),
                        icons::view(
                            icons::icon_for_file(&row.name),
                            crate::theme::icon_size::row(),
                            theme::color::DIM
                        ),
                    ]
                    .spacing(0)
                    .align_y(iced_widget::core::Alignment::Center)
                    .into()
                };
            let mut line = row![
                text(indent)
                    .size(crate::workspace::tree_row_font_size())
                    .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                    .color(name_color),
                row_icon,
                text(row.name.clone())
                    .size(crate::workspace::tree_row_font_size())
                    .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                    .color(name_color),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center);
            if let Some((kind, unstaged)) = status {
                line = line.push(iced_widget::space::horizontal());
                line = line.push(
                    text(tree_row_dot_glyph(unstaged))
                        .size(theme::font::dot_xs())
                        .color(tree_row_dot_color(kind)),
                );
            }
            let msg = if row.is_dir {
                Message::Toggle(row.path.clone())
            } else {
                Message::OpenFile(row.path.clone())
            };
            let is_selected = ws_state.tree_selected.as_deref() == Some(row.path.as_path());
            let row_btn: iced_widget::Button<
                '_,
                Message,
                iced_widget::Theme,
                iced_widget::Renderer,
            > = button(line)
                .on_press(msg)
                .width(Length::Fill)
                .style(move |_t, _s| button::Style {
                    background: if is_selected {
                        Some(theme::color::CARD.into())
                    } else {
                        None
                    },
                    text_color: theme::color::BODY,
                    ..button::Style::default()
                });
            tree_col = tree_col.push(MouseArea::new(row_btn).on_right_press(
                Message::ContextMenuOpen {
                    path: row.path.clone(),
                    is_dir: row.is_dir,
                },
            ));
            let is_new_target = matches!(
                &ws_state.tree_edit,
                Some(TreeEdit {
                    mode: TreeEditMode::NewFile | TreeEditMode::NewFolder,
                    parent_dir,
                    ..
                }) if *parent_dir == row.path
            );
            if is_new_target && row.expanded {
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth + 1, buffer));
            }
        }
    }

    let body = container(
        column![
            header,
            Scrollable::new(tree_col)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    crate::scrollbar::scrollbar()
                ))
                .style(|_t, _s| crate::scrollbar::scrollbar_style()),
        ]
        .spacing(region.gap),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    });

    container(body).width(width).height(Length::Fill).into()
}
```

（"项目信息卡"那一段——`let label = ..`/`card_col`/`card` 的构造——整段删除;
文件树行渲染循环的内容原样保留,只是不再嵌在项目卡下面;函数末尾不再包一层
`column![body, project_status_bar(..)]`,直接 `container(body)`。）

删除 `use dozer_core::protocol::ProjectInfo;` 这行 import(不再需要)。

- [ ] **Step 6: 删除 `project_status_bar` 函数**

整个 `pub(crate) fn project_status_bar(..)` 函数删除。

- [ ] **Step 7: 测试调整**

删除 `git_refreshed_updates_four_fields` 测试,替换成:

```rust
#[tokio::test]
async fn statuses_refreshed_updates_git_statuses() {
    let mut ws_state = ws_with_tree(std::env::temp_dir());
    let mut app_state = AppState::default();
    let mut statuses = HashMap::new();
    statuses.insert(
        PathBuf::from("/x"),
        crate::delivery::FileGitStatus {
            kind: crate::delivery::ChangeKind::Modified,
            staged: false,
            unstaged: true,
        },
    );
    let handle = tokio::runtime::Handle::current();
    update(
        &mut ws_state,
        &mut app_state,
        Message::StatusesRefreshed(1, statuses.clone()),
        1,
        &handle,
        |_| {},
    );
    assert_eq!(ws_state.git_statuses.len(), 1);
}
```

（`git_statuses` 字段私有,测试在同一个 `mod tests` 里,`use super::*;` 已经能直接
访问,不需要新增访问器。）

#### Part B:`workspace.rs` 组合刷新分发 + 4 个调用点 + 路由

- [ ] **Step 8: 删除 `Workspace::spawn_project_git_refresh` 方法**

删除现有(约 1896-1910 行):

```rust
fn spawn_project_git_refresh(&self, io: &ShellIo) {
    let Some(p) = &self.project else {
        return;
    };
    let project_id = p.id;
    let repo_path = PathBuf::from(&p.path);
    let proxy = io.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Files(m));
    };
    files::spawn_git_refresh(project_id, repo_path, &io.handle, emit);
}
```

- [ ] **Step 9: 新增内核自由函数**

在 `impl Workspace { .. }` 块结束、`impl App {` 开始之前(现约 2276/2277 行之间)
插入:

```rust
/// 异步跑一次组合 git 查询(分支/脏/文件状态/worktree),完成后分发成两条
/// 独立消息:`Files(StatusesRefreshed)` 只带文件级状态,`Project(GitRefreshed)`
/// 带分支/脏/worktree。两个 extension 互不知道对方存在,内核是唯一知道
/// "这两份数据同源"的地方(设计文档"关键语义确认")。4 个既有调用点:
/// `Workspace::from_restore`/`adopt_project`/回合结束(`DeliveryChecked`)/
/// `Message::ProjectFsChanged`——因为要同时认识 `files::Message`/
/// `project::Message` 两个类型,不适合作为任何一个 extension 的自由函数,
/// 也不需要 `&self`,做成纯自由函数、参数显式传入。
fn spawn_project_git_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let (b, d, s, w) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || {
                (
                    delivery::branch(&repo_path),
                    delivery::is_dirty(&repo_path),
                    delivery::file_statuses(&repo_path),
                    delivery::worktrees(&repo_path),
                )
            }
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new()));
        let _ = proxy.send_event(Message::Files(files::Message::StatusesRefreshed(
            project_id, s,
        )));
        let _ = proxy.send_event(Message::Project(project::Message::GitRefreshed(
            project_id, b, d, w,
        )));
    });
}
```

- [ ] **Step 10: `empty_for_project_placeholder` 字段替换**

`project_goal: None,` 改成:

```rust
project_panel: project::WorkspaceState::default(),
```

- [ ] **Step 11: `from_restore` 调用点调整**

现有(约 1526-1536, 1552 行):

```rust
let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
let project_goal = load_project_goal(&project.path);

let mut ws = Self {
    tabs,
    active: 0,
    next_tab_id,
    pending: HashMap::new(),
    project: Some(project),
    files,
    project_goal,
    recent_projects,
    allowed_files: allowed_files.unwrap_or_else(|| Arc::new(Mutex::new(HashSet::new()))),
    ..Self::empty_for_project_placeholder()
};
ws.ensure_project_terminal(io);
ws.restore_preview_state();
ws.spawn_project_git_refresh(io);
```

改成(`repo_path` 必须在 `project` 被 move 进 `Self { project: Some(project), .. }`
**之前**取出来,拿到 `ws` 之后就晚了):

```rust
let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
let repo_path = PathBuf::from(&project.path);
let project_panel = project::WorkspaceState::new(project::load_goal(&repo_path));

let mut ws = Self {
    tabs,
    active: 0,
    next_tab_id,
    pending: HashMap::new(),
    project: Some(project),
    files,
    project_panel,
    recent_projects,
    allowed_files: allowed_files.unwrap_or_else(|| Arc::new(Mutex::new(HashSet::new()))),
    ..Self::empty_for_project_placeholder()
};
ws.ensure_project_terminal(io);
ws.restore_preview_state();
spawn_project_git_refresh(project_id, repo_path, io);
```

（`project_id` 这个局部变量在 `from_restore` 更早处已经算好——`let project_id =
project.id;`,在 `project` 被 move 之前,直接复用,不用重新取。）

- [ ] **Step 12: `loading_for_project` 调用点调整**

现有:

```rust
fn loading_for_project(project: ProjectInfo) -> Self {
    let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
    let project_goal = load_project_goal(&project.path);
    Self {
        project: Some(project),
        files,
        project_goal,
        loading: true,
        ..Self::empty_for_project_placeholder()
    }
}
```

改成:

```rust
fn loading_for_project(project: ProjectInfo) -> Self {
    let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
    let project_panel =
        project::WorkspaceState::new(project::load_goal(Path::new(&project.path)));
    Self {
        project: Some(project),
        files,
        project_panel,
        loading: true,
        ..Self::empty_for_project_placeholder()
    }
}
```

- [ ] **Step 13: `adopt_project` 调用点调整**

现有:

```rust
fn adopt_project(&mut self, io: &ShellIo, project: ProjectInfo) {
    self.loading = false;
    self.files
        .reset_for_project(FileTree::new(PathBuf::from(&project.path)));
    self.git_watch = None;
    self.conversations = Vec::new();
    self.usage = usage::WorkspaceState::default();
    self.project_goal = load_project_goal(&project.path);
    self.project = Some(project);
    self.ensure_project_terminal(io);
    self.restore_preview_state();
    self.spawn_project_git_refresh(io);
    self.spawn_conversations_refresh(io);
    self.spawn_acceptance_count_refresh(io);
    browser::request_bookmarks_refresh(
        self.project.as_ref().map(|p| p.id),
        &io.client,
        &io.handle,
        {
            let proxy = io.proxy.clone();
            move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            }
        },
    );
    self.start_git_watch(io);
}
```

改成:

```rust
fn adopt_project(&mut self, io: &ShellIo, project: ProjectInfo) {
    self.loading = false;
    self.files
        .reset_for_project(FileTree::new(PathBuf::from(&project.path)));
    self.git_watch = None;
    self.conversations = Vec::new();
    self.usage = usage::WorkspaceState::default();
    let project_id = project.id;
    let repo_path = PathBuf::from(&project.path);
    self.project_panel = project::WorkspaceState::new(project::load_goal(&repo_path));
    self.project = Some(project);
    self.ensure_project_terminal(io);
    self.restore_preview_state();
    spawn_project_git_refresh(project_id, repo_path, io);
    self.spawn_conversations_refresh(io);
    self.spawn_acceptance_count_refresh(io);
    browser::request_bookmarks_refresh(
        self.project.as_ref().map(|p| p.id),
        &io.client,
        &io.handle,
        {
            let proxy = io.proxy.clone();
            move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            }
        },
    );
    self.start_git_watch(io);
}
```

（`browser::request_bookmarks_refresh(..)` 调用本身原样不动,只是上下文里其它行变了。）

- [ ] **Step 14: `spawn_acceptance_count_refresh` 改发到 `Message::Project`**

现有(约 1834-1854 行)结尾:

```rust
let _ = proxy.send_event(Message::Files(files::Message::AcceptanceCountLoaded(
    project_id, n,
)));
```

改成:

```rust
let _ = proxy.send_event(Message::Project(project::Message::AcceptanceCountLoaded(
    project_id, n,
)));
```

（函数其余部分——`self.project`/`acceptance_query_repo`/`client.acceptance_count`
——不变,这个方法本来就不需要认识 `files::Message`,继续留作 `Workspace` 方法。）

- [ ] **Step 15: `DeliveryChecked` 处理器调用点调整**

现有(约 3041-3066 行)里的:

```rust
// 回合结束后刷新项目 git 状态,文件树装饰随之更新（P1h）。
ws.spawn_project_git_refresh(io);
```

改成:

```rust
// 回合结束后刷新项目 git 状态,文件树装饰随之更新（P1h）。
if let Some(project) = &ws.project {
    spawn_project_git_refresh(project_id, PathBuf::from(&project.path), io);
}
```

（`project_id` 是这个 `Message::DeliveryChecked(project_id, tab_id, pending)` 分支
自带的参数,`with_project(project_id, |ws, io| { .. })` 闭包内直接可用,不用重新取。）

- [ ] **Step 16: `ProjectFsChanged` 处理器调用点调整**

现有(约 3818-3827 行):

```rust
Message::ProjectFsChanged(project_id, relevance) => {
    self.with_project(project_id, |ws, io| {
        let Some(project) = &ws.project else { return };
        let repo_path = PathBuf::from(&project.path);
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Files(m));
        };
        files::spawn_git_refresh(project_id, repo_path, &io.handle, emit);
    });
    // ...(Relevance::GitRefs 触发 Git Log 快照重建那一段,原样不动)
}
```

改成:

```rust
Message::ProjectFsChanged(project_id, relevance) => {
    self.with_project(project_id, |ws, io| {
        let Some(project) = &ws.project else { return };
        spawn_project_git_refresh(project_id, PathBuf::from(&project.path), io);
    });
    // ...(Relevance::GitRefs 触发 Git Log 快照重建那一段,原样不动)
}
```

- [ ] **Step 17: `Message::Files(..)` 分组路由分支调整**

现有(约 3899-3903 行):

```rust
Message::Files(
    msg @ (files::Message::GitRefreshed(project_id, ..)
    | files::Message::PasteDone(project_id, ..)
    | files::Message::OpDone { project_id, .. }
    | files::Message::AcceptanceCountLoaded(project_id, ..)),
) => {
```

改成:

```rust
Message::Files(
    msg @ (files::Message::StatusesRefreshed(project_id, ..)
    | files::Message::PasteDone(project_id, ..)
    | files::Message::OpDone { project_id, .. }),
) => {
```

（分支体内部——`files::update(&mut ws.files, app_files, msg, project_id, &handle,
emit);`——不变。）

- [ ] **Step 18: 新增 `Message::Project(..)` 路由分支**

紧接在上面 `Message::Files(msg) => { .. }` 兜底分支(约 3916-3930 行)之后加:

```rust
Message::Project(
    msg @ (project::Message::GitRefreshed(project_id, ..)
    | project::Message::AcceptanceCountLoaded(project_id, ..)),
) => {
    self.with_project(project_id, move |ws, _io| {
        let Some(project) = &ws.project else { return };
        let repo_path = PathBuf::from(&project.path);
        project::update(&mut ws.project_panel, msg, project_id, &repo_path);
    });
}
Message::Project(msg) => {
    self.with_focused_project(|ws, _io| {
        let Some(project) = &ws.project else { return };
        let project_id = project.id;
        let repo_path = PathBuf::from(&project.path);
        project::update(&mut ws.project_panel, msg, project_id, &repo_path);
    });
}
```

#### Part C:`left_panel_area`/`no_project_placeholder`/`worktree_strip`/`top_bar`

- [ ] **Step 19: `left_panel_area` 里 `LeftView::Files` 调用点简化**

现有(约 5943-5962 行):

```rust
LeftView::Files => {
    let (list_portion, content_portion) = split_portions(app.shell_layout.files_split);
    let list_pane: Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> =
        if ws.project.is_some() {
            files::view(
                &ws.files,
                ws.project.as_ref(),
                app.daemon_error.is_none(),
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Files)
        } else {
            no_project_placeholder(
                app,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
        };
    ...
```

改成:

```rust
LeftView::Files => {
    let (list_portion, content_portion) = split_portions(app.shell_layout.files_split);
    let list_pane: Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> =
        if ws.project.is_some() {
            files::view(
                &ws.files,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Files)
        } else {
            no_project_placeholder(
                app,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
        };
    ...
```

- [ ] **Step 20: `left_panel_area` 新增 `LeftView::Project` 分支**

在 `LeftView::Todo => { .. }` 分支(现约 5991-6013 行)之后加:

```rust
LeftView::Project => project::view(
    &ws.project_panel,
    ws.project.as_ref(),
    Length::Fill,
    zone_pane_border(zone, ac),
)
.map(Message::Project),
```

- [ ] **Step 21: `no_project_placeholder` 删掉底部状态条**

现有结尾(约 6336-6342 行):

```rust
container(column![
    body,
    files::project_status_bar(app.daemon_error.is_none(), &ws.files, outer)
])
.width(width)
.height(Length::Fill)
.into()
```

改成:

```rust
container(body).width(width).height(Length::Fill).into()
```

（`files::project_status_bar` 已在 Part A Step 6 删除,这里是唯一残留调用点。函数
头部文档注释提到"含底部状态条"的措辞也顺手删掉。）

- [ ] **Step 22: 4 处穷举 `LeftView` 匹配补 `Project` 分支**

`preview_content_bounds` 函数里两处(现约 619-641/650-675 行),各加:

```rust
LeftView::GitLog => (0.0, 0.0, 0.0, 0.0),
LeftView::Todo => (0.0, 0.0, 0.0, 0.0),
// Project 面板纯 iced 绘制,不挂 webview 子视图。
LeftView::Project => (0.0, 0.0, 0.0, 0.0),
```

`is_in_preview_column` 函数里两处(现约 692-707/710-726 行),各加:

```rust
LeftView::GitLog => false,
// Todo 面板纯 iced 绘制,无 webview,永不落在预览列。
LeftView::Todo => false,
// Project 面板同 Todo,纯 iced 绘制,无 webview。
LeftView::Project => false,
```

- [ ] **Step 23: `worktree_strip` 调用点数据源切换**

现有(约 6023 行):

```rust
Some(worktree_strip(ws.files.worktrees()))
```

改成:

```rust
Some(worktree_strip(ws.project_panel.worktrees()))
```

- [ ] **Step 24: `top_bar` 删除目标胶囊**

现有(约 4335-4362 行):

```rust
let mut right = row![].spacing(10);
if let Some(cap) = app
    .active_workspace()
    .and_then(|ws| goal_capsule_text(ws.project_goal.as_ref(), 28))
{
    let capsule = container(
        row![
            text("●").size(theme::font::dot_sm()).color(theme::color::GOLD),
            text(cap).size(theme::font::body()).color(theme::color::CREAM)
        ].spacing(6),
    )
    .padding([5, 10])
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::color::CARD.into()),
        border: Border { color: theme::color::GOLD, width: 1.0, radius: 6.0.into() },
        ..container::Style::default()
    });
    right = right.push(capsule);
}
```

改成:

```rust
let mut right = row![].spacing(10);
```

（后面紧接着的"设置"按钮 `right = right.push(MouseArea::new(..));` 原样不动。）

- [ ] **Step 25: 删除 `goal_capsule_text`/`project_branch_label`/`load_project_goal`**

从 `workspace.rs` 删除这三个函数(现约 7059-7088 行)——前两个已在 Task 4 搬进
`extensions/project.rs`,`load_project_goal` 已被 `project::load_goal` 取代。

删除 `use crate::goal::{self, Goal};` 这行 import(现约 41 行)——搬完之后
`workspace.rs` 不再有任何 `Goal`/`goal::` 引用。

删除对应的两个旧测试:`project_card_branch_label`(现约 8315-8319 行)、
`goal_capsule_prefixes_and_truncates`(现约 8682-8702 行,含开头的
`use crate::goal::Goal;`)——断言已经原样搬进 Task 4。

- [ ] **Step 26: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 剩余报错基本是"字段/方法改名后的残留引用"——逐条按上面 Step 10-25
的模式修正,不应该再出现"缺 match 分支"类的错误(Step 22 已经补齐 4 处)。

- [ ] **Step 27: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 28: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/extensions/files.rs
git commit -m "refactor(dozer-app): route combined git refresh through kernel fan-out to Files/Project"
```

---

### Task 7: main.rs 键盘路由——标题自绘编辑第 4 个目标

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes:Task 3 的 `project::WorkspaceState::title_editing_is_some`/
  `cancel_title_edit`。

- [ ] **Step 1: `Workspace`/`App` 新增键盘路由访问器**

`workspace.rs` 里 `Workspace::tree_editing`(现约 2237-2239 行)之后加:

```rust
/// 项目信息面板标题是否处于自绘编辑态(main.rs 键盘路由用)。
pub fn project_title_editing(&self) -> bool {
    self.project_panel.title_editing_is_some()
}
```

`App::tree_editing`(现约 2611-2614 行)之后加:

```rust
/// 项目信息面板标题是否处于自绘编辑态(main.rs 键盘路由用)。
pub fn project_title_editing(&self) -> bool {
    self.active_workspace()
        .is_some_and(|ws| ws.project_title_editing())
}
```

- [ ] **Step 2: `blur_inputs` 加一行**

`Workspace::blur_inputs`(现约 2252-2258 行):

```rust
pub fn blur_inputs(&mut self) {
    if self.browser.addr_editing() {
        self.browser.addr_cancel();
    }
    self.acceptance.clear_comment_editing();
    self.files.cancel_tree_edit();
    self.project_panel.cancel_title_edit();
}
```

- [ ] **Step 3: main.rs 键盘拦截优先级链加第 4 个目标**

现有(约 657-703 行):

```rust
let to_browser = app.browser_addr_editing();
let to_comment = app.acceptance_comment_editing();
let to_tree_edit = app.tree_editing();
if to_browser || to_comment || to_tree_edit {
    let addr_event = match event {
        WindowEvent::KeyboardInput {
            event,
            is_synthetic: false,
            ..
        } if event.state == ElementState::Pressed => {
            use winit::keyboard::{Key, NamedKey};
            match &event.logical_key {
                Key::Character(s) => Some(workspace::AddrEvent::Text(s.to_string())),
                Key::Named(NamedKey::Space) => {
                    Some(workspace::AddrEvent::Text(" ".into()))
                }
                Key::Named(NamedKey::Backspace) => {
                    Some(workspace::AddrEvent::Backspace)
                }
                Key::Named(NamedKey::Enter) => Some(workspace::AddrEvent::Submit),
                Key::Named(NamedKey::Escape) => Some(workspace::AddrEvent::Cancel),
                _ => None,
            }
        }
        WindowEvent::Ime(Ime::Commit(text)) => {
            Some(workspace::AddrEvent::Text(text.clone()))
        }
        _ => None,
    };
    if let Some(ev) = addr_event {
        let message = if to_browser {
            Message::Browser(extensions::browser::Message::AddrEvent(ev))
        } else if to_comment {
            Message::Acceptance(extensions::acceptance::Message::CommentEvent(ev))
        } else {
            Message::Files(extensions::files::Message::EditEvent(ev))
        };
        app.update(message);
        window.request_redraw();
    }
    return;
}
```

改成:

```rust
let to_browser = app.browser_addr_editing();
let to_comment = app.acceptance_comment_editing();
let to_tree_edit = app.tree_editing();
let to_project_title = app.project_title_editing();
if to_browser || to_comment || to_tree_edit || to_project_title {
    let addr_event = match event {
        WindowEvent::KeyboardInput {
            event,
            is_synthetic: false,
            ..
        } if event.state == ElementState::Pressed => {
            use winit::keyboard::{Key, NamedKey};
            match &event.logical_key {
                Key::Character(s) => Some(workspace::AddrEvent::Text(s.to_string())),
                Key::Named(NamedKey::Space) => {
                    Some(workspace::AddrEvent::Text(" ".into()))
                }
                Key::Named(NamedKey::Backspace) => {
                    Some(workspace::AddrEvent::Backspace)
                }
                Key::Named(NamedKey::Enter) => Some(workspace::AddrEvent::Submit),
                Key::Named(NamedKey::Escape) => Some(workspace::AddrEvent::Cancel),
                _ => None,
            }
        }
        WindowEvent::Ime(Ime::Commit(text)) => {
            Some(workspace::AddrEvent::Text(text.clone()))
        }
        _ => None,
    };
    if let Some(ev) = addr_event {
        // 优先级:浏览器地址栏 > 验收意见 > 项目树编辑 > 项目信息面板标题
        // (四者同真时罕见,谁先建的编辑态谁优先没有实际冲突场景,这个
        // 顺序只是一个确定性兜底)。
        let message = if to_browser {
            Message::Browser(extensions::browser::Message::AddrEvent(ev))
        } else if to_comment {
            Message::Acceptance(extensions::acceptance::Message::CommentEvent(ev))
        } else if to_tree_edit {
            Message::Files(extensions::files::Message::EditEvent(ev))
        } else {
            Message::Project(extensions::project::Message::TitleEditEvent(ev))
        };
        app.update(message);
        window.request_redraw();
    }
    return;
}
```

（构造 `addr_event` 的 `match event { .. }` 逻辑原样不动,只是外层条件和最终
`message` 的选择链多了一支。）

- [ ] **Step 4: 编译 + 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): route project title AddrEvent through main.rs keyboard interception"
```

---

### Task 8: 全量校验与人工验收

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全绿。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

对照设计文档逐项走一遍:

- 左图标栏新增第 5 个图标(Info),点击进入 Project 面板,再点一次收起(同其它
  4 个图标的交互)。
- Project 面板展示:项目名、git 分支(带图标)+ 脏标(有未提交改动时变金)、
  "N 次验收"副行(0 次时不显示,同现有 Files 卡逻辑)。
- 有 `.dozer/goal.md`:显示标题 + 标准列表,标题可点击进入编辑,标准逐条可删除,
  底部有"＋新增标准"输入框。
- 没有 `.dozer/goal.md`:显示"未定标 · 点击设置目标",点击进入标题编辑态,输入
  标题回车后创建文件、显示新目标(初始无标准)。
- 编辑标题:点击进入自绘输入框(尾缀"▏"光标),输入文字、回车提交,标题更新且
  `.dozer/goal.md` 文件内容同步更新(用编辑器打开确认);按 Esc 取消编辑,标题
  还原。
- 增删标准:输入框回车新增一条,立即出现在列表里且写盘;点某条标准的"×"删除,
  立即从列表消失且写盘;重启 app 后改动仍在(读盘验证持久化)。
- Files 面板(点第 1 个图标):确认不再显示项目信息卡(项目名/分支/验收次数),
  不再显示底部状态条("文件·git ...·组件"那条),只剩文件树本体+右键菜单/
  删除确认等既有交互不受影响。
- 顶栏:确认不再出现目标胶囊(金色描边小圆点+"目标：..."文案)。
- Git Log 面板(点第 3 个图标)的 worktree 速览条(图上方的"本工作区:{分支}"
  一行)不受影响,切换 worktree 仍能正常跳转。
- 切换项目页签:Project 面板的分支/脏标/验收次数/目标各自独立,不串项目
  (跟其它 6 个试点一致的多项目并行不变式)。
- 回合结束(agent 会话完成一次交付)后,Project 面板的分支/脏标/验收次数(若
  发生过验收)与 Files 面板的文件树彩色点都能各自正确刷新,不需要手动切换
  面板触发。

- [ ] **Step 3: 确认没有遗留未提交的改动**

```bash
git status
```

Expected: 干净,无未跟踪/未暂存改动(`.dozer/` 目录等项目运行时产物除外)。
