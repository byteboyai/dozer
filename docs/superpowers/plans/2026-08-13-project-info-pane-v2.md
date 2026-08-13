# 项目信息(Project)面板 v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重构现有 `extensions::project` 面板:撤下"目标"UI(数据层保留)、加项目名称/描述编辑、加根目录/git remote/磁盘占用展示、加文档与 agent 记忆虚拟链接(首次自动发现 + 用户手动增删)。

**Architecture:** `dozer-core::protocol` 新增 `Request::RenameProject`,`dozerd`/`dozer-client` 各补一层薄封装,做成第一条真正需要"改 daemon 数据"的项目面板交互。`extensions::project::update` 签名从纯同步升级为 `(ws_state, msg, project_id, repo_path, client, handle, emit)`(对齐 `extensions::acceptance`/`extensions::browser` 已有形状),让改名这类需要 daemon 往返的操作能在扩展内部自己 `handle.spawn`。链接(文档/Agent 记忆)数据模型与发现算法拆进新子模块 `extensions/project/links.rs`,`extensions/project.rs` 只负责组装。磁盘占用/git remote 沿用现有"内核编排一次组合刷新、分发给多个消费者"的既有模式。

**Tech Stack:** Rust workspace;iced 0.14(新用 `iced_widget::text_editor` 做多行描述编辑);rusqlite(dozerd);无新增外部依赖(`rfd` 已是既有依赖)。

**Spec:** `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`

## Global Constraints

- **在独立分支上开发,不要直接提交到 main**:新建 `feature/project-info-pane-v2`
  分支做全部改动,完成后提请审阅,审阅通过后再合并回 `main`。
- `extensions::project` 与其它 extension(`files`/`acceptance`/`browser` 等)之间
  不得有任何直接类型依赖——只能通过内核转发的消息通信(现有硬性原则)。
- icon 按钮/tab 类 UI 优先复用 `icons::icon_button_entry`/`tabs::tab_core`(项目
  `CLAUDE.md` 关键裁决)。本计划里新增的"+"/"×"按钮沿用本文件已有的
  `goal_block`/`CriterionRemove` 那种"裸 `button`+`text` 字符按钮"风格(不是
  hover 卡片式 icon 按钮,跟现有 `目标` 区块的删除按钮同款,不套
  `icon_button_entry`——形状不同,这里显式说明理由)。
- 目标(goal)数据层(`crates/dozer-app/src/goal.rs`:`Goal`/`load_goal`/
  `write_goal`/`goal_path`)整份保留不动,任何任务都不得修改这个文件。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app &&
  cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过,**除非任务里
  显式写明允许中间态**(Task 9 是唯一例外,见该任务说明:新增字段暂时无人读取,
  `dead_code` 警告可接受,不允许报错)。涉及 `dozer-core`/`dozerd`/`dozer-client`
  的任务(Task 2-4)同理用各自 crate 名替换 `-p dozer-app`。
- 设计文档:`docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`
  (有疑问以它为准)。

---

### Task 1: 撤下"目标"UI(数据层不动)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces:`WorkspaceState` 不再有 `goal`/`title_editing`/`add_criterion_draft`
  字段,`Message` 不再有 `TitleEditStart`/`TitleEditEvent`/
  `CriterionAddInputChanged`/`CriterionAddSubmit`/`CriterionRemove` 变体。
  `WorkspaceState::new` 整个方法删除,改用 `WorkspaceState::default()` 构造。

- [ ] **Step 1: 删除 `extensions/project.rs` 里目标相关代码**

删除:`goal_block()` 函数、`Message` 枚举里 `TitleEditStart`/`TitleEditEvent`/
`CriterionAddInputChanged`/`CriterionAddSubmit`/`CriterionRemove` 五个变体、
`update()` 里对应的五个 `match` 分支、`WorkspaceState` 的 `goal`/`title_editing`/
`add_criterion_draft` 三个字段、`WorkspaceState::new`/`title_editing_is_some`/
`cancel_title_edit` 三个方法、模块顶部 `use crate::goal::{self, Goal};` 这行、
`view()` 里 `content = content.push(goal_block(ws_state));` 这一行。

`WorkspaceState` 变成:

```rust
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    error: Option<String>,
}
```

`Message` 变成:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>),
    AcceptanceCountLoaded(i64, Option<u64>),
}
```

`update()` 变成:

```rust
pub fn update(ws_state: &mut WorkspaceState, msg: Message, _project_id: i64) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, worktrees) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.worktrees = worktrees;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
    }
}
```

(`repo_path: &Path` 参数这次先一并去掉——`update` 目前不再需要写盘。后续任务会
重新加回参数,不用现在就补一个用不到的参数。)

同时删除 `#[cfg(test)] mod tests` 里所有引用被删字段/消息的测试:
`title_edit_start_prefills_existing_title`、`title_edit_start_empty_when_no_goal`、
`title_edit_submit_creates_new_goal_and_writes_disk`、
`title_edit_submit_updates_title_keeps_criteria`、
`title_edit_submit_empty_closes_without_writing`、
`criterion_add_submit_appends_and_writes_disk`、
`criterion_add_submit_empty_draft_is_noop`、`criterion_remove_deletes_and_writes_disk`、
`load_goal_reads_existing_file`、`load_goal_missing_file_is_none`、
`goal_capsule_prefixes_and_truncates`。保留 `git_refreshed_updates_three_fields`/
`acceptance_count_loaded_sets_field`/`project_card_branch_label`,并把它们的
`update(...)` 调用去掉 `repo_path` 参数、`ws_with_goal` 辅助函数改名
`new_ws()`(不再需要传 goal):

```rust
fn new_ws() -> WorkspaceState {
    WorkspaceState::default()
}

#[test]
fn git_refreshed_updates_three_fields() {
    let mut ws = new_ws();
    update(
        &mut ws,
        Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
        1,
    );
    assert_eq!(ws.branch.as_deref(), Some("main"));
    assert!(ws.dirty);
    assert_eq!(ws.worktrees().len(), 0);
}

#[test]
fn acceptance_count_loaded_sets_field() {
    let mut ws = new_ws();
    update(&mut ws, Message::AcceptanceCountLoaded(1, Some(3)), 1);
    assert_eq!(ws.project_acceptance_count, Some(3));
}

#[test]
fn project_card_branch_label() {
    assert_eq!(project_branch_label(Some("main"), false), "main");
    assert_eq!(project_branch_label(Some("main"), true), "main*");
    assert_eq!(project_branch_label(None, false), "—");
}
```

删除 `goal_capsule_text` 函数本体(不再被任何代码调用)。

- [ ] **Step 2: `view()` 收窄参数、删除目标渲染**

`view()` 目前是 `pub fn view<'a>(ws_state, project, width, outer)`,内容不变
(项目名/分支行/验收次数副行),只删除 `goal_block` 那一行 push。函数签名不变。

- [ ] **Step 3: `workspace.rs` 三处构造点改用 `default()`**

`from_restore`(约 437-439 行)、`loading_for_project`(约 562-564 行)、
`adopt_project`(约 916 行附近)里,把:

```rust
let project_panel = project::WorkspaceState::new(project::load_goal(&repo_path));
```

(或等价写法,`loading_for_project` 里是 `project::WorkspaceState::new(project::load_goal(Path::new(&project.path)))`)

统一改成:

```rust
let project_panel = project::WorkspaceState::default();
```

`empty_for_project_placeholder` 里 `project_panel: project::WorkspaceState::default(),`
不变(已经是这个写法)。

- [ ] **Step 4: `app.rs` 里 `project::update` 调用点去掉 `repo_path` 参数**

搜索 `project::update(&mut ws.project_panel, msg, project_id, &repo_path)`(目前
两处,`Message::Project` 的显式 `project_id` 路由分支与 fallback 分支),都改成:

```rust
project::update(&mut ws.project_panel, msg, project_id);
```

如果这两处上下文里 `repo_path`/`project`(`Some(project::Message::GitRefreshed(project_id, ..) | project::Message::AcceptanceCountLoaded(project_id, ..))`分支的 `let repo_path = ...; let Some(project) = ws.project.as_ref() ...` 铺垫代码)不再被用到,一并删除这几行铺垫(避免编译器报未使用变量警告)。

- [ ] **Step 5: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app extensions::project::
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全部干净通过。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): drop goal UI from project pane, keep goal.rs data layer"
```

---

### Task 2: `dozer-core::protocol::Request::RenameProject`

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces:`Request::RenameProject { id: i64, name: String }`,复用现有
  `Reply::Project { project: Option<ProjectInfo> }`。

- [ ] **Step 1: 写失败的测试**

`protocol.rs` 的 `#[cfg(test)] mod tests` 里加:

```rust
#[test]
fn rename_project_request_roundtrips() {
    let req = Request::RenameProject {
        id: 1,
        name: "新名字".into(),
    };
    assert_eq!(
        decode_line::<Request>(encode_line(&req).trim()).unwrap(),
        req
    );
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozer-core rename_project_request_roundtrips
```

Expected: 编译失败(`RenameProject` 未定义)。

- [ ] **Step 3: 加枚举变体**

`Request` 枚举里,在 `ListProjects,`(现有,`OpenProject { path: String }` 之后)
之后插入:

```rust
/// 项目改名。`id` 不存在或 `name` trim 后为空 → `Reply::Error`。成功复用
/// `Reply::Project { project: Some(更新后的项目) }`。
RenameProject { id: i64, name: String },
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-core rename_project_request_roundtrips
```

Expected: PASS。

- [ ] **Step 5: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-core --all-targets
cargo fmt
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): add Request::RenameProject"
```

---

### Task 3: `dozerd` 落地改名

**Files:**
- Modify: `crates/dozerd/src/projects.rs`
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes:Task 2 的 `Request::RenameProject`。
- Produces:`ProjectStore::rename(&self, id: i64, name: &str) -> Result<ProjectInfo>`。

- [ ] **Step 1: 写失败的测试**

`projects.rs` 的 `#[cfg(test)] mod tests` 里加:

```rust
#[test]
fn rename_updates_name_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
    let p = store.open("/repo/a").unwrap();

    let renamed = store.rename(p.id, "新名字").unwrap();
    assert_eq!(renamed.id, p.id);
    assert_eq!(renamed.name, "新名字");

    let list = store.list().unwrap();
    assert_eq!(list[0].name, "新名字");
}

#[test]
fn rename_missing_id_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
    assert!(store.rename(999, "x").is_err());
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozerd rename_updates_name_and_persists
```

Expected: 编译失败(`rename` 未定义)。

- [ ] **Step 3: 实现 `ProjectStore::rename`**

紧接着 `list()` 方法之后加(`impl ProjectStore` 块内):

```rust
/// 按 id 改名。`id` 不存在时返回 `Err`(不做静默 no-op)。
pub fn rename(&self, id: i64, name: &str) -> Result<ProjectInfo> {
    let conn = self.conn.lock().expect("db lock");
    let affected = conn.execute(
        "UPDATE projects SET name = ?1 WHERE id = ?2",
        rusqlite::params![name, id],
    )?;
    if affected == 0 {
        anyhow::bail!("项目 id={id} 不存在");
    }
    conn.query_row(
        "SELECT id, path, name, last_active_ms, created_ms FROM projects WHERE id = ?1",
        [id],
        row_to_project,
    )
    .map_err(Into::into)
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozerd rename_updates_name_and_persists rename_missing_id_errors
```

Expected: 两个测试 PASS。

- [ ] **Step 5: `server.rs` 接线**

`Request::ListProjects => ...`(现有分支)之后加:

```rust
Request::RenameProject { id, name } => {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Reply::Error { message: "名称不能为空".into() }
    } else {
        match projects.rename(id, trimmed) {
            Ok(p) => Reply::Project { project: Some(p) },
            Err(e) => Reply::Error { message: format!("改名失败: {e}") },
        }
    }
}
```

- [ ] **Step 6: 编译 + clippy + fmt**

```bash
cargo build -p dozerd
cargo test -p dozerd
cargo clippy -p dozerd --all-targets
cargo fmt
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/projects.rs crates/dozerd/src/server.rs
git commit -m "feat(dozerd): implement ProjectStore::rename and wire RenameProject"
```

---

### Task 4: `dozer-client::Client::rename_project`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes:Task 2 的 `Request::RenameProject`/`Reply::Project`。
- Produces:`pub async fn rename_project(&self, id: i64, name: &str) -> Result<Option<ProjectInfo>>`。

- [ ] **Step 1: 实现方法**

紧接着 `open_project` 方法之后加(对齐它的写法):

```rust
pub async fn rename_project(&self, id: i64, name: &str) -> Result<Option<ProjectInfo>> {
    match self
        .roundtrip(&Request::RenameProject {
            id,
            name: name.into(),
        })
        .await?
    {
        Reply::Project { project } => Ok(project),
        Reply::Error { message } => Err(anyhow::anyhow!(message)),
        other => bail!("意外应答: {other:?}"),
    }
}
```

- [ ] **Step 2: 编译确认**

```bash
cargo build -p dozer-client
cargo clippy -p dozer-client --all-targets
cargo fmt
```

Expected: 编译通过(这个方法目前没有调用方,`dead_code` 警告在 lib crate 里不会
触发——公开 API 不受 `dead_code` lint 约束)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): add Client::rename_project"
```

---

### Task 5: 项目改名 UI + 内核接线

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes:Task 4 的 `Client::rename_project`。
- Produces:`project::update` 新签名 `(ws_state, msg, project_id, client, handle,
  emit)`(这次不再需要 `repo_path`——改名不碰本地文件)。`Message::NameEditStart`/
  `NameEditEvent`/`NameRenamed`。

#### Part A:`extensions/project.rs`

- [ ] **Step 1: `WorkspaceState` 加编辑态字段**

```rust
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    /// 项目名称行内编辑态(None=未在编辑)。
    name_editing: Option<String>,
    error: Option<String>,
}

impl WorkspaceState {
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }

    /// 供内核 main.rs 键盘路由判断"项目名称是否在自绘编辑态"。
    pub fn name_editing_is_some(&self) -> bool {
        self.name_editing.is_some()
    }

    /// 供内核 `App::blur_inputs` 调用——失焦时取出当前编辑中的名称缓冲。
    /// 返回 `Some(raw)` 时由内核发起 daemon 改名(改动且非空才真正发请求,
    /// 见 `App::blur_inputs`);`None` 表示未处于编辑态。行为与描述字段的
    /// "失焦写盘"对齐——不再像早期版本那样直接丢弃半输入(那会导致"改名
    /// 无法保存"的观感)。
    pub fn take_name_edit(&mut self) -> Option<String> {
        self.name_editing.take()
    }
}
```

- [ ] **Step 2: `Message` 加三个变体**

```rust
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>),
    AcceptanceCountLoaded(i64, Option<u64>),
    /// daemon 改名结果。带 `project_id`,走 `with_project` 路由。
    NameRenamed(i64, Result<dozer_core::protocol::ProjectInfo, String>),
    NameEditStart,
    NameEditEvent(AddrEvent),
}
```

- [ ] **Step 3: `update` 签名升级,加改名逻辑**

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    current_name: &str,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, worktrees) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.worktrees = worktrees;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
        Message::NameEditStart => {
            ws_state.name_editing = Some(current_name.to_string());
        }
        Message::NameEditEvent(ev) => match ev {
            AddrEvent::Text(s) => {
                if let Some(buf) = &mut ws_state.name_editing {
                    buf.push_str(&s);
                }
            }
            AddrEvent::Backspace => {
                if let Some(buf) = &mut ws_state.name_editing {
                    buf.pop();
                }
            }
            AddrEvent::Cancel => ws_state.name_editing = None,
            AddrEvent::Submit => {
                let Some(raw) = ws_state.name_editing.clone() else {
                    return;
                };
                let name = raw.trim().to_string();
                if name.is_empty() || name == current_name {
                    // 空名或未改动:直接关闭编辑框,不发请求。
                    ws_state.name_editing = None;
                    return;
                }
                let client = client.clone();
                handle.spawn(async move {
                    let result = client
                        .rename_project(project_id, &name)
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
                    emit(Message::NameRenamed(project_id, result));
                });
            }
        },
        Message::NameRenamed(_, result) => match result {
            Ok(_) => {
                ws_state.name_editing = None;
                ws_state.error = None;
            }
            Err(e) => {
                ws_state.error = Some(format!("改名失败: {e}"));
                // 保留编辑态原始输入,允许重试。
            }
        },
    }
}
```

`Client` 需要实现 `Clone`(已有——`io.client.clone()` 在 `acceptance`/`browser`
既有代码里已经这么用)。文件顶部 `use` 块加 `use dozer_client::Client;`(仅类型
标注需要,`update` 签名里已经用了全限定路径 `dozer_client::Client`,这行 import
可省;若为了签名简洁改用裸 `Client`,记得加这行)。

- [ ] **Step 4: `view()` 项目名一行改成可点击**

现有:

```rust
content = content.push(
    text(p.name.clone())
        .size(theme::font::title())
        .color(theme::color::CREAM),
);
```

改成(编辑态渲染沿用本文件此前 `goal_block` 标题编辑那套自绘输入框样式):

```rust
let name_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
    if let Some(buf) = &ws_state.name_editing {
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
    } else {
        button(
            text(p.name.clone())
                .size(theme::font::title())
                .color(theme::color::CREAM),
        )
        .on_press(Message::NameEditStart)
        .style(|_t, _s| iced_widget::button::Style {
            background: None,
            text_color: theme::color::CREAM,
            ..iced_widget::button::Style::default()
        })
        .into()
    };
content = content.push(name_row);
```

`error` 渲染沿用本文件已有的 `⚠ {err}` 那一段(如果 Task 1 里连同 `goal_block`
一起删掉了,这里补回:在 `view()` 末尾、`container(content)...` 之前加):

```rust
if let Some(err) = &ws_state.error {
    content = content.push(
        text(format!("⚠ {err}"))
            .size(theme::font::label())
            .color(theme::color::RED),
    );
}
```

- [ ] **Step 5: 测试**

`update()` 签名从 Task 1 的 `(ws_state, msg, project_id)` 升级成这一步的
`(ws_state, msg, project_id, current_name, client, handle, emit)`,Task 1 留下
的两个既有测试调用点要跟着补齐参数(`project_card_branch_label` 不调用
`update()`,不受影响):

```rust
#[test]
fn git_refreshed_updates_three_fields() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
        1,
        "名字",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.branch.as_deref(), Some("main"));
    assert!(ws.dirty);
    assert_eq!(ws.worktrees().len(), 0);
}

#[test]
fn acceptance_count_loaded_sets_field() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::AcceptanceCountLoaded(1, Some(3)),
        1,
        "名字",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.project_acceptance_count, Some(3));
}
```

(`git_refreshed_updates_three_fields` 这个名字在 Task 6 会因为 `GitRefreshed`
多一个字段而改成 `git_refreshed_updates_four_fields`,那是 Task 6 的事,这一步
先保持三字段版本。)

`mod tests` 里 `new_ws()` 改名不变,加:

```rust
#[test]
fn name_edit_start_prefills_current_name() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::NameEditStart,
        1,
        "旧名字",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.name_editing.as_deref(), Some("旧名字"));
}

#[test]
fn name_edit_submit_same_name_closes_without_request() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(&mut ws, Message::NameEditStart, 1, "同名", &test_client(), rt.handle(), |_| {});
    update(
        &mut ws,
        Message::NameEditEvent(AddrEvent::Submit),
        1,
        "同名",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.name_editing.is_none());
}

#[test]
fn name_renamed_ok_clears_editing_state() {
    let mut ws = new_ws();
    ws.name_editing = Some("新名字".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let project = dozer_core::protocol::ProjectInfo {
        id: 1,
        path: "/repo".into(),
        name: "新名字".into(),
        last_active_ms: 0,
        created_ms: 0,
        updated_ms: 0,
    };
    update(
        &mut ws,
        Message::NameRenamed(1, Ok(project)),
        1,
        "旧名字",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.name_editing.is_none());
    assert!(ws.error.is_none());
}

#[test]
fn name_renamed_err_keeps_editing_state_and_sets_error() {
    let mut ws = new_ws();
    ws.name_editing = Some("新名字".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::NameRenamed(1, Err("连接失败".into())),
        1,
        "旧名字",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.name_editing.as_deref(), Some("新名字"));
    assert!(ws.error.is_some());
}
```

`test_client()` 辅助函数(测试专用,连一个不存在的 socket 路径——`Client::new`
只是构造句柄、不建连接,这几个测试都不会真的走网络):

```rust
fn test_client() -> dozer_client::Client {
    dozer_client::Client::new(std::path::PathBuf::from("/tmp/dozer-project-test.sock"))
}
```

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app extensions::project::
cargo clippy -p dozer-app --all-targets
cargo fmt
```

#### Part B:内核接线

- [ ] **Step 7: `app.rs` 显式 `project_id` 路由分支加 `NameRenamed`,并同步 `ws.project`**

现有:

```rust
Message::Project(
    msg @ (project::Message::GitRefreshed(project_id, ..)
    | project::Message::AcceptanceCountLoaded(project_id, ..)),
) => {
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    let Some(project) = ws.project.as_ref() else {
        return;
    };
    let repo_path = Path::new(&project.path).to_path_buf();
    project::update(&mut ws.project_panel, msg, project_id, &repo_path);
}
Message::Project(msg) => {
    let Some(project_id) = self.active_project_id else {
        return;
    };
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    let Some(project) = ws.project.as_ref() else {
        return;
    };
    let repo_path = Path::new(&project.path).to_path_buf();
    project::update(&mut ws.project_panel, msg, project_id, &repo_path);
}
```

改成(Task 1 已经把 `project::update` 的 `repo_path` 参数去掉,这次换成
`current_name`/`client`/`handle`/`emit`;`NameRenamed` 需要在调用 `project::update`
之后额外同步 `ws.project` 缓存):

```rust
Message::Project(
    msg @ (project::Message::GitRefreshed(project_id, ..)
    | project::Message::AcceptanceCountLoaded(project_id, ..)
    | project::Message::NameRenamed(project_id, ..)),
) => {
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    let Some(project) = ws.project.as_ref() else {
        return;
    };
    let current_name = project.name.clone();
    // `NameRenamed(Ok(updated))` 要把顶栏项目页签等读的 `ws.project` 缓存
    // 一并更新——这是这个面板第一次出现需要内核介入(而不是纯委托给
    // `project::update`)的消息。
    if let project::Message::NameRenamed(_, Ok(updated)) = &msg {
        ws.project = Some(updated.clone());
    }
    let client = self.client.clone();
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Project(m));
    };
    project::update(&mut ws.project_panel, msg, project_id, &current_name, &client, &handle, emit);
}
Message::Project(msg) => {
    let Some(project_id) = self.active_project_id else {
        return;
    };
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    let Some(project) = ws.project.as_ref() else {
        return;
    };
    let current_name = project.name.clone();
    let client = self.client.clone();
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Project(m));
    };
    project::update(&mut ws.project_panel, msg, project_id, &current_name, &client, &handle, emit);
}
```

(`App` 结构体上 `client: Client`/`handle: Handle`/`proxy: EventLoopProxy<Message>`
是直接字段——`Message::Project` 这两条分支是内联处理、不经过 `with_project`
辅助方法,所以直接 `self.client.clone()` 而不是 `acceptance_result` 那种
`self.with_project(project_id, move |ws, io| { let client = io.client.clone();
.. })` 的闭包写法;两种写法在这个文件里都存在,选哪种取决于该分支是否已经在
用 `with_project`——这两条 `Message::Project` 分支现状是内联的,保持内联。)

- [ ] **Step 8: `App::blur_inputs` 改成"失焦保存"名称**

名称编辑不在 `Workspace::blur_inputs` 里丢弃(早期版本调用
`cancel_name_edit()` 直接清空,导致点开别处就丢改名——已被修复)。改由
`App::blur_inputs` 取出缓冲并发起 daemon 改名:

`Workspace::blur_inputs` 里**删除** `self.project_panel.cancel_name_edit();`
这一行(名称缓冲交由上层处理);`App::blur_inputs` 改为:

```rust
pub fn blur_inputs(&mut self) {
    let Some(ws) = self.active_workspace_mut() else {
        return;
    };
    // 先取出名称编辑缓冲,再交给 `Workspace::blur_inputs` 清其它编辑态,
    // 避免顺序问题丢失半输入。
    let pending_name = ws.project_panel.take_name_edit();
    let project = ws.project.clone();
    ws.blur_inputs();
    if let (Some(p), Some(raw)) = (project, pending_name) {
        let name = raw.trim().to_string();
        let project_id = p.id;
        let current_name = p.name.clone();
        if !name.is_empty() && name != current_name {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m: project::Message| {
                let _ = proxy.send_event(Message::Project(m));
            };
            handle.spawn(async move {
                let result = client
                    .rename_project(project_id, &name)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
                emit(project::Message::NameRenamed(project_id, result));
            });
        }
    }
}
```

> 注意:`Workspace::blur_inputs` 仍保留描述字段的 `submit_description_edit_on_blur`
> (本地文件写盘、无需网络),只把名称那一行去掉——名称改名走 daemon 往返,
> 必须在持有 `client`/`handle`/`proxy` 的 `App` 层 spawn。

- [ ] **Step 9: `main.rs` 键盘路由链改名**

`app.project_title_editing()` 改成 `app.project_name_editing()`(对应
`Workspace`/`App` 上原有的 `project_title_editing` 访问器方法本体也要跟着改名,
内部只是把 `self.project_panel.title_editing_is_some()` 换成
`self.project_panel.name_editing_is_some()`)。

`to_project_title` 变量名改 `to_project_name`,后续 `if to_browser || ... ||
to_project_title || ...` 和消息构造:

```rust
} else if to_project_name {
    Message::Project(extensions::project::Message::NameEditEvent(ev))
}
```

注释"项目标题编辑"改"项目名称编辑"。

- [ ] **Step 10: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全部干净通过。

- [ ] **Step 11: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): project name inline rename via daemon"
```

---

### Task 6: Git remote + 根目录路径展示

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces:`delivery::remote_url(repo: &Path) -> Option<String>`。
  `project::Message::GitRefreshed` 增加第 5 个字段。

- [ ] **Step 1: `delivery::remote_url` 失败测试**

`delivery.rs` 的 `#[cfg(test)] mod tests` 里加(参考同文件已有的
`branch`/`is_dirty` 测试搭建临时 git 仓库的方式):

```rust
#[test]
fn remote_url_reads_origin() {
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://example.com/x.git"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        remote_url(dir.path()),
        Some("https://example.com/x.git".to_string())
    );
}

#[test]
fn remote_url_falls_back_to_first_remote_without_origin() {
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "upstream", "https://example.com/y.git"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        remote_url(dir.path()),
        Some("https://example.com/y.git".to_string())
    );
}

#[test]
fn remote_url_none_without_any_remote() {
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().unwrap();
    assert_eq!(remote_url(dir.path()), None);
}

#[test]
fn remote_url_none_for_non_git_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(remote_url(dir.path()), None);
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozer-app remote_url
```

Expected: 编译失败(`remote_url` 未定义)。

- [ ] **Step 3: 实现**

紧接着 `pub fn branch(repo: &Path) -> Option<String>` 之后加:

```rust
/// `git remote get-url origin`;没有 origin 时退化取 `git remote -v` 第一条
/// 记录的 fetch URL;完全没有 remote 或非 git 目录 → `None`。
pub fn remote_url(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo)
        .output()
        .ok()?;
    if out.status.success() {
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !url.is_empty() {
            return Some(url);
        }
    }
    let out = Command::new("git")
        .args(["remote", "-v"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(|s| s.to_string())
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app remote_url
```

Expected: 四个测试 PASS。

- [ ] **Step 5: `extensions/project.rs`——`GitRefreshed` 加字段,`view` 渲染根目录/remote**

`Message::GitRefreshed` 签名改:

```rust
GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>, Option<String>),
```

`WorkspaceState` 加字段 `remote_url: Option<String>`。`update` 对应分支:

```rust
Message::GitRefreshed(_, branch, dirty, worktrees, remote_url) => {
    ws_state.branch = branch;
    ws_state.dirty = dirty;
    ws_state.worktrees = worktrees;
    ws_state.remote_url = remote_url;
}
```

`view()` 在验收次数副行之后加一个小节(`p.path` 是 `ProjectInfo` 自带字段,不需要
新状态):

```rust
content = content.push(
    text("根目录").size(theme::font::label()).color(theme::color::DIM),
);
content = content.push(text(p.path.clone()).size(theme::font::caption()).color(theme::color::BODY));
if let Some(url) = &ws_state.remote_url {
    content = content.push(
        text("Git 仓库").size(theme::font::label()).color(theme::color::DIM),
    );
    content = content.push(text(url.clone()).size(theme::font::caption()).color(theme::color::BODY));
}
```

- [ ] **Step 6: 测试更新**

`git_refreshed_updates_three_fields` 改名 `git_refreshed_updates_four_fields`。
注意 Task 5 已经把 `update()` 签名升级成 `(ws_state, msg, project_id,
current_name, client, handle, emit)`——这个测试从 Task 1 遗留下来时还是旧的
3 参数调用,这次要一并补齐,不是只改 `GitRefreshed` 的参数列表:

```rust
#[test]
fn git_refreshed_updates_four_fields() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::GitRefreshed(
            1,
            Some("main".to_string()),
            true,
            vec![],
            Some("https://x.git".into()),
        ),
        1,
        "名字",
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.branch.as_deref(), Some("main"));
    assert!(ws.dirty);
    assert_eq!(ws.worktrees().len(), 0);
    assert_eq!(ws.remote_url.as_deref(), Some("https://x.git"));
}
```

(`acceptance_count_loaded_sets_field`/`project_card_branch_label` 两个测试——
`project_card_branch_label` 不调用 `update()`,不受影响;
`acceptance_count_loaded_sets_field` 从 Task 5 起已经是 7 参数调用,这次不用
再改。)

- [ ] **Step 7: `workspace.rs`——`spawn_project_git_refresh` 多取一个字段**

```rust
fn spawn_project_git_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let (b, d, s, w, r) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || {
                (
                    delivery::branch(&repo_path),
                    delivery::is_dirty(&repo_path),
                    delivery::file_statuses(&repo_path),
                    delivery::worktrees(&repo_path),
                    delivery::remote_url(&repo_path),
                )
            }
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new(), None));
        let _ = proxy.send_event(Message::Files(files::Message::StatusesRefreshed(
            project_id, s,
        )));
        let _ = proxy.send_event(Message::Project(project::Message::GitRefreshed(
            project_id, b, d, w, r,
        )));
    });
}
```

- [ ] **Step 8: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/delivery.rs crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): show repo root path and git remote in project pane"
```

---

### Task 7: 磁盘占用统计

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`extensions::project::DISK_USAGE_EXCLUDE`、
  `extensions::project::dir_size_excluding(root: &Path, exclude: &[&str]) -> u64`、
  `Message::DiskUsageLoaded(i64, u64)`。

- [ ] **Step 1: `dir_size_excluding` 失败测试**

`extensions/project.rs` 的 `mod tests` 里加:

```rust
#[test]
fn dir_size_excluding_sums_files_and_skips_excluded_dirs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "12345").unwrap(); // 5 bytes
    std::fs::create_dir(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join("target").join("big.bin"), vec![0u8; 1000]).unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join("b.txt"), "12").unwrap(); // 2 bytes
    let size = dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE);
    assert_eq!(size, 7); // 5 + 2,target 整个跳过
}

#[test]
fn dir_size_excluding_empty_dir_is_zero() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE), 0);
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozer-app dir_size_excluding
```

Expected: 编译失败。

- [ ] **Step 3: 实现**

文件顶部(`view` 函数之前任意位置)加:

```rust
/// 磁盘占用统计的排除名单——跟 `crates/dozer-app/src/project.rs::HIDDEN`
/// (文件树"要不要显示这一行")语义不同,这里是"算不算项目真实内容",不复用
/// 那份常量。
pub const DISK_USAGE_EXCLUDE: [&str; 7] =
    [".git", "target", "node_modules", "dist", "build", ".venv", "__pycache__"];

/// 递归求和 `root` 下所有文件大小,跳过名字命中 `exclude` 的目录(整个子树
/// 跳过,不下钻)。读不到的条目(权限/符号链接死链)跳过不计入,不中断整体
/// 计算。
pub fn dir_size_excluding(root: &Path, exclude: &[&str]) -> u64 {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if exclude.contains(&name.as_str()) {
                continue;
            }
            total += dir_size_excluding(&entry.path(), exclude);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app dir_size_excluding
```

Expected: PASS。

- [ ] **Step 5: `Message`/`WorkspaceState`/`update`/`view`**

`WorkspaceState` 加 `disk_usage_bytes: Option<u64>`。`Message` 加
`DiskUsageLoaded(i64, u64)`。`update` 加分支:

```rust
Message::DiskUsageLoaded(_, bytes) => {
    ws_state.disk_usage_bytes = Some(bytes);
}
```

`view()` 在 Task 6 加的"根目录/Git 仓库"小节标题上方补一个占用数字(单位换算成
MB,保留整数,跟草图"文件存储 (450MB)"的展示口径一致):

```rust
let usage_label = ws_state
    .disk_usage_bytes
    .map(|b| format!("文件存储 ({} MB)", b / 1_000_000))
    .unwrap_or_else(|| "文件存储".to_string());
content = content.push(
    text(usage_label).size(theme::font::label()).color(theme::color::DIM),
);
```

（插入位置:紧接在验收次数副行之后、Task 6 那段"根目录"小节之前——"文件存储"是
这一整块小节的标题,根目录路径/Git 仓库 remote 是这个标题下的两行内容,视觉层级
对齐草图。）

- [ ] **Step 6: 测试**

```rust
#[test]
fn disk_usage_loaded_sets_field() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(&mut ws, Message::DiskUsageLoaded(1, 12345), 1, "名字", &test_client(), rt.handle(), |_| {});
    assert_eq!(ws.disk_usage_bytes, Some(12345));
}
```

- [ ] **Step 7: `workspace.rs`——`spawn_disk_usage_refresh` + 4 个调用点**

紧接着 `spawn_project_git_refresh` 之后加:

```rust
/// 磁盘占用是独立于组合 git 刷新的异步任务——避免大仓库的目录遍历拖慢
/// 分支/脏标显示。触发点与 `spawn_project_git_refresh` 相同。
fn spawn_disk_usage_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let bytes = tokio::task::spawn_blocking(move || {
            project::dir_size_excluding(&repo_path, &project::DISK_USAGE_EXCLUDE)
        })
        .await
        .unwrap_or(0);
        let _ = proxy.send_event(Message::Project(project::Message::DiskUsageLoaded(
            project_id, bytes,
        )));
    });
}
```

在 `spawn_project_git_refresh(project_id, repo_path, io);` 的全部 4 个既有调用点
(`workspace.rs` 两处、`app.rs` 两处)紧接着各加一行:

```rust
spawn_disk_usage_refresh(project_id, repo_path.clone(), io);
```

(`app.rs` 两处调用点原本传的是 `PathBuf::from(&project.path)` 而不是复用某个
`repo_path` 局部变量——那两处要么先存一个局部变量再两次调用都用它,要么各自
`PathBuf::from(&project.path)` 调用两次,任选其一,不要求两处写法一致。)

- [ ] **Step 8: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): compute and show disk usage excluding build artifacts"
```

---

### Task 8: 项目描述——本地文件读写

**Files:**
- Create: `crates/dozer-app/src/project_meta.rs`
- Modify: `crates/dozer-app/src/main.rs`(声明模块)

**Interfaces:**
- Produces:`pub fn load_description(repo: &Path) -> Option<String>`、
  `pub fn write_description(repo: &Path, text: &str) -> std::io::Result<()>`。

- [ ] **Step 1: 写失败的测试**

创建 `crates/dozer-app/src/project_meta.rs`:

```rust
//! 项目描述:`.dozer/description.md`,纯文本,无格式约束。跟 `goal.rs` 平级
//! ——目标和描述是两种独立数据,只是都挂在 `.dozer/` 下。

use std::path::{Path, PathBuf};

fn description_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("description.md")
}

/// 文件不存在或内容 trim 后为空 → `None`。
pub fn load_description(repo: &Path) -> Option<String> {
    let text = std::fs::read_to_string(description_path(repo)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 整份重写。空字符串仍然写入一个空文件(不是删除文件)——跟
/// `load_description` 的"trim 后为空 → None"配合,行为等价于清空。
pub fn write_description(repo: &Path, text: &str) -> std::io::Result<()> {
    let path = description_path(repo);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "这是一段描述").unwrap();
        assert_eq!(
            load_description(dir.path()),
            Some("这是一段描述".to_string())
        );
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_description(dir.path()), None);
    }

    #[test]
    fn load_whitespace_only_is_none() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "   \n  ").unwrap();
        assert_eq!(load_description(dir.path()), None);
    }

    #[test]
    fn write_creates_dozer_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "x").unwrap();
        assert!(description_path(dir.path()).exists());
    }

    #[test]
    fn write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "旧").unwrap();
        write_description(dir.path(), "新").unwrap();
        assert_eq!(load_description(dir.path()), Some("新".to_string()));
    }
}
```

- [ ] **Step 2: 声明模块**

`main.rs` 顶部 `mod` 声明列表(按字母序,`preview_state`/`project` 之间)加:

```rust
mod project_meta;
```

- [ ] **Step 3: 跑测试**

```bash
cargo test -p dozer-app project_meta::
```

Expected: 五个测试 PASS。

- [ ] **Step 4: clippy + fmt**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/project_meta.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): add project_meta for .dozer/description.md"
```

---

### Task 9: `extensions/project/links.rs`——数据模型 + 发现算法

**Files:**
- Create: `crates/dozer-app/src/extensions/project/links.rs`
- Modify: `crates/dozer-app/src/extensions/project.rs`(声明子模块)

**Interfaces:**
- Produces:`LinkKind`、`LinkEntry`、`LinksState`、`LinkTarget`、`DirRow`、
  `load`/`save`/`discover_docs`/`discover_memory`/`load_or_discover`/
  `read_dir_row`。

- [ ] **Step 1: 类型 + `load`/`save` 失败测试**

创建 `crates/dozer-app/src/extensions/project/links.rs`:

```rust
//! 文档/Agent 记忆虚拟链接:数据模型、发现算法、`.dozer/links.json` 存取。
//! 见设计文档"虚拟链接(文档 + Agent 记忆)"一节。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkTarget {
    Docs,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LinkKind {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEntry {
    pub path: PathBuf,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LinksState {
    pub docs: Vec<LinkEntry>,
    pub memory: Vec<LinkEntry>,
}

impl LinksState {
    pub fn list(&self, target: LinkTarget) -> &[LinkEntry] {
        match target {
            LinkTarget::Docs => &self.docs,
            LinkTarget::Memory => &self.memory,
        }
    }

    pub fn list_mut(&mut self, target: LinkTarget) -> &mut Vec<LinkEntry> {
        match target {
            LinkTarget::Docs => &mut self.docs,
            LinkTarget::Memory => &mut self.memory,
        }
    }
}

fn links_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("links.json")
}

/// 文件不存在 → `None`(调用方据此判断"要不要跑首次自动发现")。文件存在但
/// 损坏(反序列化失败)→ `Some(LinksState::default())`,不当作"文件不存在"
/// ——损坏就是空,不会触发重新自动发现覆盖用户已有的手动改动假象。
pub fn load(repo: &Path) -> Option<LinksState> {
    let text = std::fs::read_to_string(links_path(repo)).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save(repo: &Path, state: &LinksState) -> std::io::Result<()> {
    let path = links_path(repo);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(state).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let state = LinksState {
            docs: vec![LinkEntry {
                path: PathBuf::from("/repo/README.md"),
                kind: LinkKind::File,
            }],
            memory: vec![],
        };
        save(dir.path(), &state).unwrap();
        assert_eq!(load(dir.path()), Some(state));
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);
    }

    #[test]
    fn load_corrupt_file_returns_default_not_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(dir.path().join(".dozer").join("links.json"), "not json").unwrap();
        assert_eq!(load(dir.path()), Some(LinksState::default()));
    }
}
```

- [ ] **Step 2: 声明子模块**

`extensions/project.rs` 顶部加:

```rust
pub mod links;
```

- [ ] **Step 3: 跑测试**

```bash
cargo test -p dozer-app extensions::project::links::
```

Expected: 三个测试 PASS(`cargo build -p dozer-app` 此时应保持干净——这一步只加
了纯数据模块,没有任何未使用的公开项:`LinkTarget`/`LinkKind`/`LinkEntry` 都在
`LinksState`/测试里被用到)。

- [ ] **Step 4: `discover_docs` 失败测试**

`links.rs` 的 `mod tests` 里加:

```rust
#[test]
fn discover_docs_finds_readme_and_docs_dir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();
    std::fs::write(dir.path().join("CHANGELOG.md"), "").unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap(); // 不匹配,应忽略
    let found = discover_docs(dir.path());
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].path, dir.path().join("CHANGELOG.md")); // 文件按名排序
    assert_eq!(found[0].kind, LinkKind::File);
    assert_eq!(found[1].path, dir.path().join("README.md"));
    assert_eq!(found[2].path, dir.path().join("docs")); // 目录排在文件之后
    assert_eq!(found[2].kind, LinkKind::Dir);
}

#[test]
fn discover_docs_matches_claude_and_agents_md() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("CLAUDE.md"), "").unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "").unwrap();
    let found = discover_docs(dir.path());
    assert_eq!(found.len(), 2);
}

#[test]
fn discover_docs_case_insensitive_and_no_recursion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("readme.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs").join("README.md"), "").unwrap(); // 不该被发现,非根目录
    let found = discover_docs(dir.path());
    assert_eq!(found.len(), 2); // readme.txt + docs 目录本身
}

#[test]
fn discover_docs_empty_dir_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(discover_docs(dir.path()).is_empty());
}
```

- [ ] **Step 5: 实现 `discover_docs`**

```rust
const DOC_FILE_PREFIXES: [&str; 4] = ["readme", "changelog", "contributing", "license"];
const DOC_EXACT_FILES: [&str; 2] = ["claude.md", "agents.md"];
const DOC_DIR_NAMES: [&str; 4] = ["docs", "doc", "design", "documentation"];

/// 首次发现:根目录直接子项(不递归)。文件名(忽略大小写)以
/// `readme`/`changelog`/`contributing`/`license` 开头,或精确匹配
/// `claude.md`/`agents.md`(指令文件,归到文档而非"记忆"),或目录名(忽略
/// 大小写)精确匹配 `docs`/`doc`/`design`/`documentation`。结果顺序:文件在
/// 前、目录在后,组内按名排序。
pub fn discover_docs(repo: &Path) -> Vec<LinkEntry> {
    let Ok(rd) = std::fs::read_dir(repo) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if DOC_DIR_NAMES.contains(&lower.as_str()) {
                dirs.push((name, entry.path()));
            }
        } else if DOC_FILE_PREFIXES.iter().any(|p| lower.starts_with(p))
            || DOC_EXACT_FILES.contains(&lower.as_str())
        {
            files.push((name, entry.path()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    files
        .into_iter()
        .map(|(_, path)| LinkEntry { path, kind: LinkKind::File })
        .chain(
            dirs.into_iter()
                .map(|(_, path)| LinkEntry { path, kind: LinkKind::Dir }),
        )
        .collect()
}
```

- [ ] **Step 6: 跑测试确认通过**

```bash
cargo test -p dozer-app extensions::project::links::discover_docs
```

Expected: 四个测试 PASS。

- [ ] **Step 7: `discover_memory` 失败测试**

```rust
#[test]
fn discover_memory_finds_existing_agent_dirs() {
    let home = tempfile::tempdir().unwrap();
    let repo = PathBuf::from("/repo/x");
    let claude_memory = crate::conversation::claude_project_dir_in(home.path(), &repo).join("memory");
    std::fs::create_dir_all(&claude_memory).unwrap();
    let found = discover_memory_in(home.path(), &repo);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, claude_memory);
    assert_eq!(found[0].kind, LinkKind::Dir);
}

#[test]
fn discover_memory_none_when_nothing_exists() {
    let home = tempfile::tempdir().unwrap();
    let repo = PathBuf::from("/repo/y");
    assert!(discover_memory_in(home.path(), &repo).is_empty());
}
```

**先检查 `crates/dozer-app/src/conversation.rs` 是否已有 `_in` 变体(显式传
`home: &Path` 而不是内部读 `HOME` 环境变量的版本)**——该文件里
`project_dir_in(home, agent_root, cwd)` 是私有辅助函数,`claude_project_dir`/
`codebuddy_project_dir`/`opencode_project_dir` 三个公开函数目前都是内部读
`home_dir()`(即读 `HOME` 环境变量)、没有对外暴露 `_in` 版本。测试要避免
mutate 全局 `HOME`(该文件顶部注释已经解释过这个坑),所以这一步要先给
`conversation.rs` 补三个公开的 `_in` 变体:

```rust
pub fn claude_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".claude", cwd)
}

pub fn codebuddy_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".codebuddy", cwd)
}

pub fn opencode_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".dozer/agents/opencode", cwd)
}
```

(紧接在对应的 `claude_project_dir`/`codebuddy_project_dir`/`opencode_project_dir`
各自定义之后加,原函数保持不变、内部改成调用新的 `_in` 变体避免重复:

```rust
pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    claude_project_dir_in(&home_dir(), cwd)
}
```

三个都这样改。)

- [ ] **Step 8: 跑测试确认失败**

```bash
cargo test -p dozer-app discover_memory
```

Expected: 编译失败(`discover_memory_in` 未定义)。

- [ ] **Step 9: 实现 `discover_memory`/`discover_memory_in`**

`links.rs` 加:

```rust
/// 首次发现:`claude_project_dir/memory`、`codebuddy_project_dir`、
/// `opencode_project_dir` 三个目录,存在的才收进结果(不存在的静默跳过,不
/// 算错误)。Claude 是唯一有"结构化记忆子目录"这个明确约定的(跟本仓
/// auto-memory 系统同款),Codebuddy/Opencode 用各自的项目存储根目录代替
/// ——语义上更接近"历史会话"而非严格"记忆",但这是目前唯一已知的路径规则。
pub fn discover_memory(repo: &Path) -> Vec<LinkEntry> {
    discover_memory_in(&crate::conversation::home_dir(), repo)
}

fn discover_memory_in(home: &Path, repo: &Path) -> Vec<LinkEntry> {
    let candidates = [
        crate::conversation::claude_project_dir_in(home, repo).join("memory"),
        crate::conversation::codebuddy_project_dir_in(home, repo),
        crate::conversation::opencode_project_dir_in(home, repo),
    ];
    candidates
        .into_iter()
        .filter(|p| p.is_dir())
        .map(|path| LinkEntry { path, kind: LinkKind::Dir })
        .collect()
}
```

`conversation.rs` 里 `home_dir()` 目前是私有函数(`fn home_dir() -> PathBuf`)。
`links.rs` 要用它,改成 `pub(crate) fn home_dir() -> PathBuf`(不用整个
`pub`,`links.rs` 在同一个 crate 内就够)。

- [ ] **Step 10: 跑测试确认通过**

```bash
cargo test -p dozer-app extensions::project::links::
cargo test -p dozer-app conversation::
```

Expected: 全部 PASS,包括 `conversation.rs` 原有测试不受影响。

- [ ] **Step 11: `load_or_discover` + `read_dir_row`(供后续任务用,先实现+测试)**

```rust
/// `load` 返回 `None`(文件不存在,首次打开)时跑两个 `discover_*` 拼出初始
/// `LinksState` 并立即 `save`;返回 `Some(state)` 直接用,不再跑发现。
pub fn load_or_discover(repo: &Path) -> LinksState {
    if let Some(state) = load(repo) {
        return state;
    }
    let state = LinksState {
        docs: discover_docs(repo),
        memory: discover_memory(repo),
    };
    let _ = save(repo, &state);
    state
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// 目录类型链接就地展开用:单层 `read_dir`,目录在前、按名排序。读不到
/// (权限/不存在)返回空 vec,不报错。
pub fn read_dir_row(path: &Path) -> Vec<DirRow> {
    let Ok(rd) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut rows: Vec<DirRow> = rd
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            DirRow { path: e.path(), name, is_dir }
        })
        .collect();
    rows.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    rows
}
```

测试:

```rust
#[test]
fn load_or_discover_runs_discovery_when_no_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();
    let state = load_or_discover(dir.path());
    assert_eq!(state.docs.len(), 1);
    // 第二次调用不再重新发现——已经落盘,即使根目录多了一个新文件也不会
    // 被捡进来。
    std::fs::write(dir.path().join("CHANGELOG.md"), "").unwrap();
    let state2 = load_or_discover(dir.path());
    assert_eq!(state2.docs.len(), 1);
}

#[test]
fn load_or_discover_uses_existing_file_without_rediscovering() {
    let dir = tempfile::tempdir().unwrap();
    let manual = LinksState {
        docs: vec![],
        memory: vec![],
    };
    save(dir.path(), &manual).unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();
    let state = load_or_discover(dir.path());
    assert!(state.docs.is_empty()); // 不会因为磁盘上有 README 就补进来
}

#[test]
fn read_dir_row_lists_dirs_before_files_sorted_by_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("a_dir")).unwrap();
    let rows = read_dir_row(dir.path());
    assert_eq!(rows.len(), 2);
    assert!(rows[0].is_dir);
    assert_eq!(rows[0].name, "a_dir");
    assert_eq!(rows[1].name, "b.txt");
}

#[test]
fn read_dir_row_missing_dir_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_dir_row(&dir.path().join("nope")).is_empty());
}
```

- [ ] **Step 12: 跑测试**

```bash
cargo test -p dozer-app extensions::project::links::
```

Expected: 全部 PASS。

- [ ] **Step 13: 编译整个 crate 确认无报错(允许 `dead_code` 警告)**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`links::load_or_discover`/`links::read_dir_row`/
`LinksState::list`/`LinkTarget` 目前没有任何调用方(下一个任务才会接进
`WorkspaceState`),`dead_code` 警告可接受,**不允许报错**——这是本计划唯一
允许中间态警告的任务(跟原 Project 面板计划 Task 3 同款допущение)。

```bash
cargo fmt
```

- [ ] **Step 14: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project/links.rs crates/dozer-app/src/conversation.rs
git commit -m "feat(dozer-app): add links data model, discovery, and .dozer/links.json storage"
```

---

### Task 10: 描述 + 链接接入 `WorkspaceState` 构造

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 8 的 `project_meta::load_description`,Task 9 的
  `links::load_or_discover`。
- Produces:`WorkspaceState::new(description: Option<String>, links:
  links::LinksState) -> Self`。

- [ ] **Step 1: `WorkspaceState` 加字段 + 重新引入 `new`**

```rust
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    remote_url: Option<String>,
    disk_usage_bytes: Option<u64>,
    project_acceptance_count: Option<u64>,
    description: Option<String>,
    name_editing: Option<String>,
    links: links::LinksState,
    error: Option<String>,
}

impl WorkspaceState {
    /// 打开一个新项目时构造。`description`/`links` 由调用方在构造之前分别
    /// 调 `project_meta::load_description`/`links::load_or_discover` 拿到
    /// (同现有 `files::WorkspaceState::new(FileTree::new(..))` 那种"调用方
    /// 先算好再传入"的既有模式)。
    pub fn new(description: Option<String>, links: links::LinksState) -> Self {
        Self {
            description,
            links,
            ..Self::default()
        }
    }

    // worktrees/name_editing_is_some/take_name_edit 三个既有访问器不变
    // (take_name_edit 取代早期的 cancel_name_edit:失焦时取缓冲交 App 层保存,
    // 不再丢弃半输入)。
}
```

- [ ] **Step 2: `workspace.rs` 三处构造点**

`from_restore`/`loading_for_project`/`adopt_project`(Task 1 Step 3 里刚改成
`project::WorkspaceState::default()` 的三处)改成:

```rust
let project_panel = project::WorkspaceState::new(
    crate::project_meta::load_description(&repo_path),
    project::links::load_or_discover(&repo_path),
);
```

(`repo_path`/等价的 `Path::new(&project.path)` 局部变量在这三处此前构造
`files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)))` 时已经
现成可用,直接复用,不需要新取。)

- [ ] **Step 3: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全部干净通过——`description`/`links` 字段现在被构造点写入,但还没有
任何 `view()`/其它逻辑读取,理论上仍会有"never read"警告;**这一步同样允许该
警告**,下一个任务会消费掉。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): load description and links at project-open time"
```

---

### Task 11: 项目描述 UI(多行原生输入)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/workspace.rs`(改名成功后写盘调用点如需要)

**Interfaces:**
- Produces:`Message::DescriptionEditStart`/`DescriptionEditAction`/
  `DescriptionEditSubmit`。

- [ ] **Step 1: `WorkspaceState` 加编辑态字段**

```rust
description_editing: Option<iced_widget::text_editor::Content>,
```

- [ ] **Step 2: `Message` 加三个变体**

```rust
DescriptionEditStart,
DescriptionEditAction(iced_widget::text_editor::Action),
DescriptionEditSubmit,
```

- [ ] **Step 3: `update` 加三个分支**

`update` 目前签名是 Task 5 定的 `(ws_state, msg, project_id, current_name,
client, handle, emit)`,这次再加一个 `repo_path: &Path` 参数(描述要写本地
文件,跟改名不同,需要仓库路径):

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    current_name: &str,
    repo_path: &Path,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        // .. 既有分支不变 ..
        Message::DescriptionEditStart => {
            let initial = ws_state.description.clone().unwrap_or_default();
            ws_state.description_editing =
                Some(iced_widget::text_editor::Content::with_text(&initial));
        }
        Message::DescriptionEditAction(action) => {
            if let Some(content) = &mut ws_state.description_editing {
                content.perform(action);
            }
        }
        Message::DescriptionEditSubmit => {
            let Some(content) = ws_state.description_editing.take() else {
                return;
            };
            let text = content.text().trim().to_string();
            match crate::project_meta::write_description(repo_path, &text) {
                Ok(()) => {
                    ws_state.description = if text.is_empty() { None } else { Some(text) };
                    ws_state.error = None;
                }
                Err(e) => {
                    ws_state.error = Some(format!("保存失败: {e}"));
                    ws_state.description_editing = Some(content); // 保留编辑态允许重试
                }
            }
        }
    }
}
```

（Task 5 里加进 `update` 签名的 `current_name`/`client`/`handle`/`emit` 参数继续
保留,只是这次描述分支不用它们——Rust 不要求每个分支都用完所有参数,不影响
编译,只是这几个参数在描述相关分支里用不上。）

所有调用 `project::update(..)` 的地方(`app.rs` 两处、Task 5 新增的测试
`test_client()` 调用等)都要在参数列表里插入 `repo_path`(位置在
`current_name` 之后、`client` 之前)。

- [ ] **Step 4: `view()` 描述区块**

在项目名一行之后加(点击进入编辑,编辑态下换成多行 `text_editor`;无描述时
显示占位文案,呼应草图虚线框观感——这里用普通文本按钮模拟"点击开始编辑"的
入口,不画真的虚线边框,保持跟本文件其它区块一致的极简风格):

```rust
let description_block: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
    if let Some(editing) = &ws_state.description_editing {
        iced_widget::text_editor(editing)
            .placeholder("项目描述信息…")
            .on_action(Message::DescriptionEditAction)
            .height(Length::Fixed(72.0))
            .into()
    } else {
        let label = ws_state
            .description
            .clone()
            .unwrap_or_else(|| "点击添加项目描述…".to_string());
        let color = if ws_state.description.is_some() {
            theme::color::BODY
        } else {
            theme::color::DIM
        };
        button(text(label).size(theme::font::body()).color(color))
            .on_press(Message::DescriptionEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: theme::color::BODY,
                ..iced_widget::button::Style::default()
            })
            .into()
    };
content = content.push(description_block);
```

**失焦保存**:描述编辑态跟"名称编辑"不同,不走 `main.rs` 自绘键盘拦截链
(Task 5 里名称编辑走的是那条链),原生 `text_editor` 靠 iced 自身焦点系统收
字符,没有一个天然的"提交"按键(多行文本里 Enter 是换行,不是提交)。这次先
用最简单的方案:`Workspace::blur_inputs`(点击面板外任意位置触发)里加一行,
如果 `description_editing.is_some()` 就自动触发一次 `DescriptionEditSubmit`
等价逻辑。因为 `blur_inputs` 是纯同步方法、没有 `emit`/`client` 可用,不能直接
调 `project::update`,改成给 `WorkspaceState` 加一个不需要外部依赖的辅助方法:

```rust
impl WorkspaceState {
    /// 供内核 `Workspace::blur_inputs` 调用——失焦时把当前编辑态直接写盘
    /// (描述保存不需要网络往返,不用等 `Message` 走一圈)。
    pub fn submit_description_edit_on_blur(&mut self, repo_path: &Path) {
        let Some(content) = self.description_editing.take() else {
            return;
        };
        let text = content.text().trim().to_string();
        if crate::project_meta::write_description(repo_path, &text).is_ok() {
            self.description = if text.is_empty() { None } else { Some(text) };
        }
        // 写失败这里不重试(失焦场景不适合弹错误态阻塞用户),下次进入面板
        // 仍能看到 `description` 字段的旧值,不会丢用户输入太久——这是已知
        // 的简化,写盘失败几率很低(权限问题会在其它写操作里更早暴露)。
    }
}
```

`Workspace::blur_inputs` 里加(需要拿到 `repo_path`——`self.project.as_ref()`
取):

```rust
if let Some(p) = &self.project {
    let repo_path = Path::new(&p.path);
    self.project_panel.submit_description_edit_on_blur(repo_path);
}
```

`Message::DescriptionEditSubmit`/对应 `update` 分支保留(给以后可能加的显式
"保存"按钮用),即使暂时没有 UI 触发它也不算 dead code——`update` 里的
`match` 分支只要 `Message` 变体存在就会被编译器要求穷举,不会因为"没有 view
按钮发它"而警告。

- [ ] **Step 5: 测试**

```rust
#[test]
fn description_edit_start_prefills_existing_text() {
    let mut ws = new_ws();
    ws.description = Some("旧描述".into());
    let repo = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::DescriptionEditStart,
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.description_editing.is_some());
}

#[test]
fn description_edit_submit_writes_disk_and_updates_field() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(&mut ws, Message::DescriptionEditStart, 1, "名字", repo.path(), &test_client(), rt.handle(), |_| {});
    update(
        &mut ws,
        Message::DescriptionEditAction(iced_widget::text_editor::Action::Edit(
            iced_widget::text_editor::Edit::Paste(std::sync::Arc::new("新描述".to_string())),
        )),
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    update(&mut ws, Message::DescriptionEditSubmit, 1, "名字", repo.path(), &test_client(), rt.handle(), |_| {});
    assert_eq!(ws.description.as_deref(), Some("新描述"));
    assert_eq!(
        crate::project_meta::load_description(repo.path()).as_deref(),
        Some("新描述")
    );
}

#[test]
fn submit_description_edit_on_blur_writes_and_clears_editing_state() {
    let mut ws = new_ws();
    ws.description_editing = Some(iced_widget::text_editor::Content::with_text("失焦保存"));
    let repo = tempfile::tempdir().unwrap();
    ws.submit_description_edit_on_blur(repo.path());
    assert!(ws.description_editing.is_none());
    assert_eq!(ws.description.as_deref(), Some("失焦保存"));
}
```

（`Action::Edit(Edit::Paste(Arc<String>))` 是 `iced_core::text::editor` 里
`Action`/`Edit` 两个枚举的真实签名——`Edit::Paste` 接一整段文字直接塞进
`Content`,测试用它模拟"用户输入了这段文字"最省事。）

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app extensions::project::
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): editable project description with native text_editor"
```

---

### Task 12: 链接 UI + 交互(增/删/展开/打开)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Produces:`Message::LinkAdd`/`LinkRemove`/`LinkDirToggle`/`OpenLink`,顶层
  `app::Message::ProjectLinkPickFile`/`ProjectLinkPickDir`。

#### Part A:`extensions/project.rs`

- [ ] **Step 1: `WorkspaceState` 加展开集字段**

```rust
expanded_link_dirs: std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
```

- [ ] **Step 2: `Message` 加四个变体**

```rust
LinkAdd {
    target: links::LinkTarget,
    path: PathBuf,
    kind: links::LinkKind,
},
LinkRemove {
    target: links::LinkTarget,
    index: usize,
},
LinkDirToggle {
    target: links::LinkTarget,
    path: PathBuf,
},
/// 内核拦截处理,见 `files::Message::OpenFile` 文档同款写法。
OpenLink(PathBuf),
```

- [ ] **Step 3: `update` 加四个分支(`OpenLink` 是 `unreachable!`)**

```rust
Message::LinkAdd { target, path, kind } => {
    let already_present = ws_state.links.list(target).iter().any(|e| e.path == path);
    if !already_present {
        ws_state.links.list_mut(target).push(links::LinkEntry { path, kind });
        match links::save(repo_path, &ws_state.links) {
            Ok(()) => ws_state.error = None,
            Err(e) => {
                ws_state.links.list_mut(target).pop();
                ws_state.error = Some(format!("保存失败: {e}"));
            }
        }
    }
}
Message::LinkRemove { target, index } => {
    let list = ws_state.links.list_mut(target);
    if index >= list.len() {
        return;
    }
    let removed = list.remove(index);
    if let Err(e) = links::save(repo_path, &ws_state.links) {
        ws_state.links.list_mut(target).insert(index, removed);
        ws_state.error = Some(format!("保存失败: {e}"));
    } else {
        ws_state.error = None;
    }
}
Message::LinkDirToggle { path, .. } => {
    if ws_state.expanded_link_dirs.remove(&path).is_none() {
        let rows = links::read_dir_row(&path);
        ws_state.expanded_link_dirs.insert(path, rows);
    }
}
Message::OpenLink(_) => {
    unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
}
```

- [ ] **Step 4: `view()`——"项目文档"/"Agent 记忆"两个小节**

在磁盘占用/根目录/git remote 小节之后加:

```rust
content = content.push(links_section(
    "项目文档",
    links::LinkTarget::Docs,
    &ws_state.links,
    &ws_state.expanded_link_dirs,
));
content = content.push(links_section(
    "Agent 记忆",
    links::LinkTarget::Memory,
    &ws_state.links,
    &ws_state.expanded_link_dirs,
));
```

`view()` 函数末尾之前(或作为独立私有函数)加:

```rust
fn links_section<'a>(
    title: &'static str,
    target: links::LinkTarget,
    links_state: &'a links::LinksState,
    expanded: &'a std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(6);
    col = col.push(
        row![
            text(title).size(theme::font::label()).color(theme::color::DIM),
            iced_widget::space::horizontal(),
            button(text("+文件").size(theme::font::caption()).color(theme::color::DIM))
                .on_press(Message::PickFile(target))
                .style(|_t, _s| iced_widget::button::Style {
                    background: None,
                    text_color: theme::color::DIM,
                    ..iced_widget::button::Style::default()
                }),
            button(text("+目录").size(theme::font::caption()).color(theme::color::DIM))
                .on_press(Message::PickDir(target))
                .style(|_t, _s| iced_widget::button::Style {
                    background: None,
                    text_color: theme::color::DIM,
                    ..iced_widget::button::Style::default()
                }),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    for (i, entry) in links_state.list(target).iter().enumerate() {
        let name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.path.to_string_lossy().into_owned());
        let row_icon = if entry.kind == links::LinkKind::Dir {
            icons::IconKind::Folder
        } else {
            icons::icon_for_file(&name)
        };
        let click_msg = if entry.kind == links::LinkKind::Dir {
            Message::LinkDirToggle { target, path: entry.path.clone() }
        } else {
            Message::OpenLink(entry.path.clone())
        };
        col = col.push(
            row![
                button(
                    row![
                        icons::view(row_icon, crate::theme::icon_size::row(), theme::color::DIM),
                        text(name).size(theme::font::body()).color(theme::color::BODY),
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::Alignment::Center)
                )
                .on_press(click_msg)
                .style(|_t, _s| iced_widget::button::Style {
                    background: None,
                    text_color: theme::color::BODY,
                    ..iced_widget::button::Style::default()
                }),
                iced_widget::space::horizontal(),
                button(text("×").size(theme::font::body()).color(theme::color::DIM))
                    .on_press(Message::LinkRemove { target, index: i })
                    .style(|_t, _s| iced_widget::button::Style {
                        background: None,
                        text_color: theme::color::DIM,
                        ..iced_widget::button::Style::default()
                    }),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        );
        if entry.kind == links::LinkKind::Dir {
            if let Some(rows) = expanded.get(&entry.path) {
                for row_entry in rows {
                    col = col.push(
                        row![
                            iced_widget::space::Space::new().width(Length::Fixed(20.0)),
                            icons::view(
                                if row_entry.is_dir {
                                    icons::IconKind::Folder
                                } else {
                                    icons::icon_for_file(&row_entry.name)
                                },
                                crate::theme::icon_size::row(),
                                theme::color::DIM
                            ),
                            text(row_entry.name.clone())
                                .size(theme::font::caption())
                                .color(theme::color::DIM),
                        ]
                        .spacing(6)
                        .align_y(iced_widget::core::Alignment::Center),
                    );
                }
            }
        }
    }
    col.into()
}
```

`Message` 补两个变体(picker 触发,内核拦截转发到顶层 `Message::ProjectLinkPickFile`/`ProjectLinkPickDir`,见 Part B):

```rust
PickFile(links::LinkTarget),
PickDir(links::LinkTarget),
```

`update` 里对应加(内核拦截,同 `OpenLink`):

```rust
Message::PickFile(_) | Message::PickDir(_) => {
    unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
}
```

- [ ] **Step 5: 测试**

```rust
#[test]
fn link_add_appends_and_writes_disk() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkAdd {
            target: links::LinkTarget::Docs,
            path: PathBuf::from("/repo/README.md"),
            kind: links::LinkKind::File,
        },
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.links.docs.len(), 1);
    let loaded = links::load(repo.path()).unwrap();
    assert_eq!(loaded.docs.len(), 1);
}

#[test]
fn link_add_dedupes_existing_path() {
    let mut ws = new_ws();
    ws.links.docs.push(links::LinkEntry {
        path: PathBuf::from("/repo/README.md"),
        kind: links::LinkKind::File,
    });
    let repo = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkAdd {
            target: links::LinkTarget::Docs,
            path: PathBuf::from("/repo/README.md"),
            kind: links::LinkKind::File,
        },
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.links.docs.len(), 1);
}

#[test]
fn link_remove_deletes_and_writes_disk() {
    let mut ws = new_ws();
    ws.links.memory.push(links::LinkEntry {
        path: PathBuf::from("/home/.claude/memory"),
        kind: links::LinkKind::Dir,
    });
    let repo = tempfile::tempdir().unwrap();
    links::save(repo.path(), &ws.links).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkRemove { target: links::LinkTarget::Memory, index: 0 },
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.links.memory.is_empty());
    assert!(links::load(repo.path()).unwrap().memory.is_empty());
}

#[test]
fn link_dir_toggle_expands_then_collapses() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join("docs")).unwrap();
    std::fs::write(repo.path().join("docs").join("a.md"), "").unwrap();
    let docs_path = repo.path().join("docs");
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkDirToggle { target: links::LinkTarget::Docs, path: docs_path.clone() },
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.expanded_link_dirs.get(&docs_path).map(|r| r.len()), Some(1));
    update(
        &mut ws,
        Message::LinkDirToggle { target: links::LinkTarget::Docs, path: docs_path.clone() },
        1,
        "名字",
        repo.path(),
        &test_client(),
        rt.handle(),
        |_| {},
    );
    assert!(!ws.expanded_link_dirs.contains_key(&docs_path));
}
```

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app extensions::project::
cargo clippy -p dozer-app --all-targets
cargo fmt
```

#### Part B:内核接线

- [ ] **Step 7: `app.rs`——`OpenLink`/`PickFile`/`PickDir` 拦截**

`Message::Project` 的 fallback 分支之前加(顺序在两条"显式 `project_id`"分支
之后):

```rust
Message::Project(project::Message::OpenLink(path)) => {
    self.update(Message::PreviewOpenPath(path));
}
Message::Project(project::Message::PickFile(target)) => {
    self.update(Message::ProjectLinkPickFile(target));
}
Message::Project(project::Message::PickDir(target)) => {
    self.update(Message::ProjectLinkPickDir(target));
}
```

顶层 `pub enum Message`(`app.rs` 里,`ProjectTabPickFolder,` 那一行之后)加:

```rust
/// "+文件"/"+目录":main.rs 弹 rfd 模态选完后回送
/// `project::Message::LinkAdd`。
ProjectLinkPickFile(project::links::LinkTarget),
ProjectLinkPickDir(project::links::LinkTarget),
```

`Message::ProjectLinkPickFile`/`ProjectLinkPickDir` 这两条顶层消息本身不需要
`app.rs` 里再写处理分支(main.rs 会直接拦截它们,类似 `ProjectTabPickFolder`
——如果 `app.update` 里对未识别的顶层消息有穷举 `match` 要求,加两个 no-op
分支占位):

```rust
Message::ProjectLinkPickFile(_) | Message::ProjectLinkPickDir(_) => {
    // rfd 弹窗在 main.rs 里同步处理,选中后转成 project::Message::LinkAdd
    // 再回送到这里;这两条顶层消息本身不需要 App::update 处理任何东西。
}
```

- [ ] **Step 8: `main.rs`——rfd 处理 + `OpenLink`/`PickFile`/`PickDir` 消息构造**

紧挨着现有 `Message::ProjectTabPickFolder => { .. }` 分支之后加:

```rust
Message::ProjectLinkPickFile(target) => {
    if let Some(path) = rfd::FileDialog::new().pick_file() {
        app.update(Message::Project(extensions::project::Message::LinkAdd {
            target,
            path,
            kind: extensions::project::links::LinkKind::File,
        }));
        window.request_redraw();
    }
}
Message::ProjectLinkPickDir(target) => {
    if let Some(path) = rfd::FileDialog::new().pick_folder() {
        app.update(Message::Project(extensions::project::Message::LinkAdd {
            target,
            path,
            kind: extensions::project::links::LinkKind::Dir,
        }));
        window.request_redraw();
    }
}
```

- [ ] **Step 9: 编译 + 测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全部干净通过。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): doc and agent-memory link add/remove/expand/open"
```

---

## 人工验收(全部任务合并后)

- 项目名称:点标题进入编辑、改名提交、顶栏项目页签同步显示新名字;改名失败
  (比如把 daemon 关掉再试)时面板显示错误、编辑框保留输入。
- 项目描述:点击占位文案进入多行编辑、输入多行文字、点击面板外任意位置
  (失焦)后重开项目确认内容还在。
- 根目录路径 + Git 仓库 remote 正确显示;磁盘占用数字在 dozer 自己这个仓库
  (`target/` 目录很大)上明显小于"不排除"时的数字。
- 首次打开一个新项目:根目录有 `README.md`/`docs/` 时自动出现在"项目文档",
  Claude 记忆目录存在时自动出现在"Agent 记忆"。
- 删除一条自动发现的链接、重新打开(或切换再切回)面板,确认不会自动复活。
- 点文件类型链接在右侧预览 tab 打开;点目录类型链接在面板内就地展开/收起。
- 手动"+文件"/"+目录"各加一条链接;重复加同一路径确认不会出现两条。
- `目标`功能确认已从面板消失(不再有任何目标相关 UI 入口)。
