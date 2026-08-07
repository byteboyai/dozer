# Todo 面板扩展化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **执行前置条件:** 浏览器面板扩展化试点(`docs/superpowers/plans/2026-08-07-browser-extension-pilot.md`)的 Task 5/6 必须先落地并经审阅——两边都会大幅改 `workspace.rs`,同时改会冲突。开始执行本计划前,先确认那份计划的状态。

**Goal:** 把 Todo 面板(任务列表/筛选搜索/派发/计划时间)拆成自洽模块 `extensions::todo`(自己的 `Message`/`update`/`view`),`workspace.rs` 内核只留包装转发——阶段 1 扩展化重构的第三个试点,状态归属比前两个试点都复杂(两块状态:per-project + App 级)。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/todo.rs`:把现有 `todo.rs`(纯解析/筛选逻辑)与 `todo_meta.rs`(JSON sidecar 持久化)整体并入,新增 `WorkspaceState`(挂 `Workspace`,对应现有 8 个 `todo_*` 字段)、`AppState`(挂 `App`,对应现有 `todo_meta` 字段)、`Message`(12 个变体)、`update`(处理 10 条不涉及终端会话的消息,同时接收两块状态)、`reload_from_disk`/`record_dispatch`/`set_plan_date`/`set_completed_at`(内核直调的普通函数)、`view`。原 `todo.rs`/`todo_meta.rs` 删除。`TodoDispatchToExisting`/`TodoDispatchNew` 两条消息、以及既有的 `Message::TabAttached` 分支里"派发到新建"补记录那段,由内核直接处理(真正的终端会话写入/新建不下放进扩展模块),事后调用 `extensions::todo::record_dispatch`。`PickerLaunch` 保持在 `workspace.rs`(终端"新建 agent 会话"选择器共用,不是 Todo 专属)。

**Tech Stack:** Rust workspace;iced 0.14;同步 `std::fs` 文件 I/O(不需要 `tokio::Handle`/`emit` 这套,`.dozer/todo.md`/`todo_meta.json` 都是本机小文件同步读写,现状如此)。

## Global Constraints

- 纯重构,不改变任何用户可见行为——文件格式、`todo_meta.json` schema、冲突处理(`replace_todo_line` 返回 `None` 时静默放弃+强制重读)、三态推导规则、筛选/搜索语义,一律原样保留。
- 不建 `Extension` trait/注册表,不拆独立 crate,不引入 `iced::Task`/`Command` 风格异步。
- `PickerLaunch`(终端"新建 agent 会话"选择器共用类型)不移动,`extensions::todo::Message::DispatchNew` 只是引用它。
- `TodoDispatchToExisting`/`TodoDispatchNew` 处理时涉及的终端会话写入/新建(`ws.tabs`/`io.client.write()`/`spawn_new_tab`)一律留在 `workspace.rs`,`extensions::todo` 从头到尾不知道 `session`/`Client` 这些概念。
- `Message::TabAttached` 分支里"派发到新建的 tab 补记派发记录"那段小逻辑,同样保留在 `workspace.rs`(`TabAttached` 本身不是 Todo 消息,是终端会话生命周期的通用消息),只是改调 `extensions::todo::record_dispatch`。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app` 干净通过。

---

### Task 1: `extensions::todo` 纯逻辑与元数据类型迁入

**Files:**
- Create: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod todo;`)
- Delete: `crates/dozer-app/src/todo.rs`、`crates/dozer-app/src/todo_meta.rs`
- Modify: `crates/dozer-app/src/main.rs`(删 `mod todo;`/`mod todo_meta;`)

**Interfaces:**
- Produces:`pub struct TodoItem`、`pub fn todo_path`、`pub fn parse_todo`、
  `pub fn replace_todo_line`、`pub fn append_todo_item`、`pub fn todo_line_key`、
  `pub enum TodoState`、`pub fn todo_display_state`、`pub enum TodoFilter`、
  `pub fn filter_todos`、`pub fn completed_at_for_toggle`、`pub struct DispatchRecord`、
  `pub struct TodoTaskMeta`、`pub type TodoMetaState`、`fn meta_load()`、
  `fn meta_save(&TodoMetaState) -> io::Result<()>`(原 `todo_meta::load`/`save`,加前缀
  避免和本文件后续要加的 `AppState` 方法命名混淆,具体是否要这个前缀写代码时按实际情况
  取舍,不是硬性要求)。

- [ ] **Step 1: 创建文件,原样并入 `todo.rs` 全部内容**

创建 `crates/dozer-app/src/extensions/todo.rs`,把现有 `crates/dozer-app/src/todo.rs`
(383 行:`TodoItem`/`todo_path`/`parse_todo`/`replace_todo_line`/`append_todo_item`/
`todo_line_key`/`TodoState`/`todo_display_state`/`TodoFilter`/`filter_todos`/
`completed_at_for_toggle`,以及它们的 12 个测试)整份复制过来,内容和测试都不改一个字,
只改文件顶部的 doc comment 说明(原来说"design 2026-08-06",这次加一句已并入
`extensions::todo`)。原文件里 `use crate::todo_meta::DispatchRecord;` 这一行改成同文件内
引用(下一步会把 `DispatchRecord` 定义也搬进同一个文件,不再需要跨模块 `use`)。

- [ ] **Step 2: 并入 `todo_meta.rs` 全部内容**

紧跟 Step 1 内容之后,把现有 `crates/dozer-app/src/todo_meta.rs`(111 行:`DispatchRecord`/
`TodoTaskMeta`/`TodoMetaState`/`file_path`/`load`/`save`/`load_from`/`save_to`,以及它的
3 个测试)整份复制过来,`load`/`save` 两个 `pub fn` 改名成 `meta_load`/`meta_save`(避免
跟 Task 2 要加的 `AppState::save`/`AppState::load` 之类的方法混淆;若写代码时发现不会撞名,
不改名也可以,不是硬性要求)。两份测试模块分别是 `todo.rs`/`todo_meta.rs` 各自的
`#[cfg(test)] mod tests { ... }`,合并成一个文件后要么保留两个各自独立的 `mod tests`
块(Rust 允许同一文件多个 `mod tests`,但名字会冲突,必须重命名,比如
`mod parse_tests`/`mod meta_tests`),要么合并成一个 `mod tests` 把所有测试函数放一起
——两种做法都行,选一种保持清晰即可。

- [ ] **Step 3: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入(在 `browser`/`git_log` 之后):

```rust
pub mod browser;
pub mod git_log;
pub mod todo;
```

`crates/dozer-app/src/main.rs` 删除 `mod todo;`/`mod todo_meta;` 两行。

- [ ] **Step 4: 删除原文件**

```bash
git rm crates/dozer-app/src/todo.rs crates/dozer-app/src/todo_meta.rs
```

- [ ] **Step 5: 编译 + 测试确认纯迁移没有破坏任何东西**

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: `extensions/todo.rs` 内容本身编译通过;`workspace.rs` 那边此刻必然报大量"找不到
`todo::`/`todo_meta::`"的错(还没接线),这些留给 Task 4 修,先用
`cargo check -p dozer-app 2>&1 | grep "extensions/todo.rs"` 单独确认这个新文件本身没有
报错。

Run: `cargo test -p dozer-app --bin dozer extensions::todo:: 2>&1 | tail -60`
Expected: 上一步单独 `cargo check` 通过后,这一步大概率仍会因为 `workspace.rs` 编译失败而
连带跑不起来——这是正常的,`extensions::todo` 自身测试要等 Task 4 内核接线完成、整个
crate 能编译过之后才能真正跑绿,本步骤只做"新文件自身语法/类型正确"这一层确认。

- [ ] **Step 6: Commit**

```bash
git add -A crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/todo.rs \
        crates/dozer-app/src/main.rs
git rm -f crates/dozer-app/src/todo.rs crates/dozer-app/src/todo_meta.rs 2>/dev/null || true
git commit -m "refactor(dozer-app): fold todo.rs and todo_meta.rs into extensions/todo.rs"
```

---

### Task 2: `WorkspaceState`/`AppState`/`Message`/`update`/内核直调函数

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Produces:
  - `pub struct WorkspaceState`(`Default`)+ `impl WorkspaceState`:
    `dispatch_popup_open(&self) -> bool`、
    `take_pending_dispatch(&mut self, tab_id: usize) -> Option<String>`
  - `pub struct AppState`(`Default`)+ `impl AppState`:
    `meta_for(&self, project_id: i64, key: u64) -> Option<&TodoTaskMeta>`、
    `record_dispatch(&mut self, project_id: i64, text: &str, session_id: String)`、
    `set_plan_date(&mut self, project_id: i64, text: &str, draft: String)`、
    `set_completed_at(&mut self, project_id: i64, text: &str, done: bool)`、
    `load() -> Self`、`fn save(&self)`(内部调 `meta_save`,失败 `tracing::warn!`,
    不返回 `Result`——内核不需要处理失败,现有 `record_todo_dispatch` 等方法就是这样,
    错误只记日志)
  - `pub enum Message`(`Debug, Clone`,12 个变体)
  - `pub fn update(ws_state: &mut WorkspaceState, app_state: &mut AppState, msg: Message, project_id: i64, project_path: &std::path::Path)`
  - `pub fn reload_from_disk(ws_state: &mut WorkspaceState, project_path: &std::path::Path)`

- [ ] **Step 1: `WorkspaceState`**

紧跟 Task 1 内容之后追加:

```rust
/// Todo 面板挂在每个 `Workspace` 上的状态。
#[derive(Default)]
pub struct WorkspaceState {
    items: Vec<TodoItem>,
    mtime: Option<std::time::SystemTime>,
    add_draft: String,
    filter: TodoFilter,
    search: String,
    dispatch_open: Option<usize>,
    pending_dispatch: std::collections::HashMap<usize, String>,
    editing_plan_date: Option<(usize, String)>,
}

impl WorkspaceState {
    /// 派发选择层是否打开(内核 `App::todo_dispatch_open` 键盘/UI 状态查询用)。
    pub fn dispatch_popup_open(&self) -> bool {
        self.dispatch_open.is_some()
    }

    /// "派发到新建"发起时记的 `tab_id → 任务文本` 映射,内核在
    /// `Message::TabAttached` 落地时用真正的 `session_id` 消费掉这条,
    /// 补记派发记录。未知 `tab_id` 返回 `None`,是 no-op。
    pub fn take_pending_dispatch(&mut self, tab_id: usize) -> Option<String> {
        self.pending_dispatch.remove(&tab_id)
    }
}
```

（`Default` derive 要求所有字段类型都实现 `Default`——`TodoFilter` 需要补
`#[derive(Default)]` 并指定默认值 `#[default] All`,写代码时在 Task 1 已并入的
`TodoFilter` 定义上补这个 derive 和标注。）

- [ ] **Step 2: `AppState`**

```rust
/// Todo 面板挂在 `App` 上的元数据(派发记录/计划时间/完成时间),按
/// `project_id` 分桶,整体持久化到 `todo_meta.json`。
#[derive(Default)]
pub struct AppState {
    meta: TodoMetaState,
}

impl AppState {
    pub fn load() -> Self {
        Self { meta: meta_load() }
    }

    fn save(&self) {
        if let Err(e) = meta_save(&self.meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }

    /// 按 `project_id`+`todo_line_key` 查这条任务的元数据(派发记录/计划
    /// 时间/完成时间),渲染层(`view`)和三态推导都用这个。
    pub fn meta_for(&self, project_id: i64, key: u64) -> Option<&TodoTaskMeta> {
        self.meta.get(&project_id)?.get(&key)
    }

    /// 把一条派发记录写进去并落盘。`text` 用来算 `todo_line_key`——跟
    /// 查询用的 key 必须是同一套算法,否则写进去的记录永远查不到。
    pub fn record_dispatch(&mut self, project_id: i64, text: &str, session_id: String) {
        let key = todo_line_key(text);
        let entry = self.meta.entry(project_id).or_default();
        entry.insert(
            key,
            TodoTaskMeta {
                dispatch: Some(DispatchRecord {
                    session_id,
                    dispatched_at: std::time::SystemTime::now(),
                }),
                ..entry.get(&key).cloned().unwrap_or_default()
            },
        );
        self.save();
    }

    /// 写/清计划时间:`draft` 为空字符串时存 `None`。
    pub fn set_plan_date(&mut self, project_id: i64, text: &str, draft: String) {
        let key = todo_line_key(text);
        let entry = self.meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.plan_date = if draft.trim().is_empty() {
            None
        } else {
            Some(draft.trim().to_string())
        };
        self.save();
    }

    /// 勾选变完成 → 盖章当前时间;取消勾选 → 清空。
    pub fn set_completed_at(&mut self, project_id: i64, text: &str, done: bool) {
        let key = todo_line_key(text);
        let entry = self.meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.completed_at = completed_at_for_toggle(done, std::time::SystemTime::now());
        self.save();
    }
}
```

- [ ] **Step 3: `Message`**

```rust
/// Todo 面板自己的消息类型。`DispatchToExisting`/`DispatchNew` 涉及终端
/// 会话读写,内核在到达 `update` 之前就会拦截处理,不会真的传进
/// `update`——传进来会 `unreachable!`(同 Git Log 试点 `LoadMore` 的
/// 处理方式)。
#[derive(Debug, Clone)]
pub enum Message {
    Toggle(usize),
    AddInputChanged(String),
    AddSubmit,
    FilterSet(TodoFilter),
    SearchChanged(String),
    DispatchOpen(usize),
    DispatchClose,
    DispatchToExisting(usize, String),
    DispatchNew(usize, crate::workspace::PickerLaunch),
    PlanDateEditStart(usize),
    PlanDateChanged(String),
    PlanDateSubmit,
}
```

- [ ] **Step 4: `reload_from_disk`**

```rust
/// 重读 `.dozer/todo.md`,刷新 `items`/`mtime`。文件不存在/读失败按空
/// 列表处理,不 panic。现有 `Workspace::reload_todo_from_disk` 的搬家
/// 版本。
pub fn reload_from_disk(ws_state: &mut WorkspaceState, project_path: &std::path::Path) {
    let path = todo_path(project_path);
    ws_state.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let md = std::fs::read_to_string(&path).unwrap_or_default();
    ws_state.items = parse_todo(&md);
}
```

- [ ] **Step 5: `update`**

```rust
/// 处理除 `DispatchToExisting`/`DispatchNew` 之外的 10 条消息,统一接收
/// 两块状态——`Toggle`/`PlanDateEditStart`/`PlanDateSubmit` 需要读写
/// `AppState`(不只是 Git Log/浏览器试点里"只有派发类消息碰跨领域状态"
/// 那么简单,写计划前重新核对现有代码才发现这点)。
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    project_path: &std::path::Path,
) {
    match msg {
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
            let new_line = format!("- [{}] {}", if item.done { " " } else { "x" }, item.text);
            let before_text = item.text.clone();
            let path = todo_path(project_path);
            let Ok(content) = std::fs::read_to_string(&path) else {
                return;
            };
            match replace_todo_line(&content, &old_line, &new_line) {
                Some(new_content) => {
                    if let Err(e) = std::fs::write(&path, &new_content) {
                        tracing::warn!("写入 todo.md 失败: {e}");
                        return;
                    }
                    reload_from_disk(ws_state, project_path);
                }
                None => {
                    // 冲突:文件已经变了,放弃这次写入,直接重读展示最新状态。
                    reload_from_disk(ws_state, project_path);
                    return;
                }
            }
            // 文本没变(正常勾选场景)才更新 completed_at;如果文本变了
            // (文件可能在重读期间被 agent 并发改过),跳过,避免把完成
            // 时间错记到另一条任务上。
            if let Some(after) = ws_state.items.get(idx)
                && after.text == before_text
            {
                app_state.set_completed_at(project_id, &after.text, after.done);
            }
        }
        Message::AddInputChanged(s) => ws_state.add_draft = s,
        Message::AddSubmit => {
            let text = ws_state.add_draft.trim().to_string();
            if text.is_empty() {
                return;
            }
            let path = todo_path(project_path);
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let new_content = append_todo_item(&content, &text);
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                tracing::warn!("创建 .dozer 目录失败: {e}");
                return;
            }
            if let Err(e) = std::fs::write(&path, &new_content) {
                tracing::warn!("写入 todo.md 失败: {e}");
                return;
            }
            ws_state.add_draft.clear();
            reload_from_disk(ws_state, project_path);
        }
        Message::FilterSet(f) => ws_state.filter = f,
        Message::SearchChanged(s) => ws_state.search = s,
        Message::DispatchOpen(idx) => ws_state.dispatch_open = Some(idx),
        Message::DispatchClose => ws_state.dispatch_open = None,
        Message::PlanDateEditStart(idx) => {
            let existing = ws_state
                .items
                .get(idx)
                .map(|item| todo_line_key(&item.text))
                .and_then(|key| app_state.meta_for(project_id, key))
                .and_then(|m| m.plan_date.clone())
                .unwrap_or_default();
            ws_state.editing_plan_date = Some((idx, existing));
        }
        Message::PlanDateChanged(s) => {
            if let Some((_, draft)) = ws_state.editing_plan_date.as_mut() {
                *draft = s;
            }
        }
        Message::PlanDateSubmit => {
            if let Some((idx, draft)) = ws_state.editing_plan_date.clone()
                && let Some(text) = ws_state.items.get(idx).map(|item| item.text.clone())
            {
                app_state.set_plan_date(project_id, &text, draft);
            }
            ws_state.editing_plan_date = None;
        }
        Message::DispatchToExisting(..) | Message::DispatchNew(..) => {
            unreachable!(
                "DispatchToExisting/DispatchNew 由内核在 Message::Todo 分支里直接处理\
                 (需要终端会话读写能力),不会转发到这里"
            )
        }
    }
}
```

- [ ] **Step 6: 编译确认**

Run: `cargo check -p dozer-app 2>&1 | grep "extensions/todo.rs"`
Expected: 无输出(本文件本身无报错);整个 crate 此刻仍因 `workspace.rs` 未接线而编译不过,
留给 Task 4。

- [ ] **Step 7: 新增单测**

在文件的测试模块里追加(参照 Task 1 Step 2 选定的测试模块组织方式放进去):

```rust
    fn ws_with_item(text: &str, done: bool) -> WorkspaceState {
        WorkspaceState {
            items: vec![TodoItem {
                text: text.to_string(),
                done,
            }],
            ..WorkspaceState::default()
        }
    }

    fn project_dir_with_todo(md: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = todo_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, md).unwrap();
        let root = dir.path().to_path_buf();
        (dir, root)
    }

    #[test]
    fn reload_from_disk_populates_items_and_mtime() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, &root);
        assert_eq!(ws_state.items.len(), 1);
        assert!(ws_state.mtime.is_some());
    }

    #[test]
    fn reload_from_disk_missing_file_yields_empty_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.items.is_empty());
        assert!(ws_state.mtime.is_none());
    }

    #[test]
    fn update_toggle_flips_line_on_disk_and_sets_completed_at() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::Toggle(0), 1, &root);
        assert!(ws_state.items[0].done, "内存态应反映勾选后的完成态");
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).unwrap().completed_at.is_some());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [x] 任务A"));
    }

    #[test]
    fn update_toggle_missing_original_line_reloads_without_setting_completed_at() {
        // 文件内容跟内存态对不上(模拟并发冲突):old_line 找不到。
        let (_dir, root) = project_dir_with_todo("- [x] 任务A(已经被改过)\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::Toggle(0), 1, &root);
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).is_none(), "冲突时不该记完成时间");
    }

    #[test]
    fn update_add_submit_appends_and_clears_draft() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState {
            add_draft: "新任务".to_string(),
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::AddSubmit, 1, &root);
        assert!(ws_state.add_draft.is_empty());
        assert_eq!(ws_state.items.len(), 1);
        assert_eq!(ws_state.items[0].text, "新任务");
    }

    #[test]
    fn update_add_submit_empty_draft_is_noop() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddSubmit,
            1,
            &root,
        );
        assert!(ws_state.items.is_empty());
    }

    #[test]
    fn update_filter_and_search_set_fields() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::FilterSet(TodoFilter::Done),
            1,
            &root,
        );
        assert_eq!(ws_state.filter, TodoFilter::Done);
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchChanged("关键字".to_string()),
            1,
            &root,
        );
        assert_eq!(ws_state.search, "关键字");
    }

    #[test]
    fn update_dispatch_open_and_close_toggle_popup() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::DispatchOpen(2), 1, &root);
        assert!(ws_state.dispatch_popup_open());
        update(&mut ws_state, &mut app_state, Message::DispatchClose, 1, &root);
        assert!(!ws_state.dispatch_popup_open());
    }

    #[test]
    fn update_plan_date_edit_start_prefills_from_app_state() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        app_state.set_plan_date(1, "任务A", "08-10".to_string());
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEditStart(0),
            1,
            &root,
        );
        assert_eq!(
            ws_state.editing_plan_date,
            Some((0, "08-10".to_string()))
        );
    }

    #[test]
    fn update_plan_date_edit_start_no_existing_value_prefills_empty() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEditStart(0),
            1,
            &root,
        );
        assert_eq!(ws_state.editing_plan_date, Some((0, String::new())));
    }

    #[test]
    fn update_plan_date_submit_writes_app_state_and_clears_editing() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        ws_state.editing_plan_date = Some((0, "08-10".to_string()));
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateSubmit,
            1,
            &root,
        );
        assert!(ws_state.editing_plan_date.is_none());
        let key = todo_line_key("任务A");
        assert_eq!(
            app_state.meta_for(1, key).unwrap().plan_date.as_deref(),
            Some("08-10")
        );
    }

    #[test]
    fn app_state_record_dispatch_then_meta_for_finds_it() {
        let mut app_state = AppState::default();
        app_state.record_dispatch(1, "任务A", "sess-1".to_string());
        let key = todo_line_key("任务A");
        let meta = app_state.meta_for(1, key).unwrap();
        assert_eq!(meta.dispatch.as_ref().unwrap().session_id, "sess-1");
    }

    #[test]
    fn take_pending_dispatch_removes_and_returns_once() {
        let mut ws_state = WorkspaceState::default();
        ws_state
            .pending_dispatch
            .insert(7, "任务A".to_string());
        assert_eq!(
            ws_state.take_pending_dispatch(7),
            Some("任务A".to_string())
        );
        assert_eq!(ws_state.take_pending_dispatch(7), None, "取过一次就没了");
    }
```

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "feat(dozer-app): add todo WorkspaceState/AppState/Message/update"
```

---

### Task 3: `extensions::todo::view` 与渲染辅助函数

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Produces: `pub fn view(app_state: &AppState, ws_state: &WorkspaceState, project_id: i64, tabs: &[SessionTabSummary], width: Length, outer: Border) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>`
  (`SessionTabSummary` 是新引入的小结构体,见 Step 1 说明——`todo_pane` 现有实现需要读
  `ws.tabs`(终端会话列表)算存活 agent tab 和三态推导用的"目标是否存活",但 `WorkspaceState`
  不持有终端会话数据,所以由内核把需要的那一小份摘要传进来。)

- [ ] **Step 1: 为什么 `view` 需要一个额外的 `tabs` 参数**

现有 `todo_pane` 里两处直接读 `ws.tabs`(终端会话 tab 列表,`Workspace` 的核心字段,不属于
Todo 面板状态):
1. 算三态时:`ws.tabs.iter().any(|t| t.info.id == d.session_id && t.alive)` 判定派发目标
   是否存活。
2. 派发选择层列出的"存活 agent tab":`ws.tabs.iter().filter(|t| t.alive).map(|t| (t.info.id.as_str(), tab_title(...)))`。

`extensions::todo` 不该知道 `SessionTab`(终端领域的类型)。内核在调用 `view` 之前先把这
两处需要的信息摘成一份精简清单传进去:

```rust
/// `view` 渲染派发相关 UI 需要的终端会话摘要,由内核从 `ws.tabs` 摘出来
/// 传入——`extensions::todo` 不知道 `SessionTab` 这个终端领域的类型。
pub struct SessionTabSummary {
    pub session_id: String,
    pub title: String,
    pub alive: bool,
}
```

插在 `Message` 定义之前(或紧跟 `WorkspaceState`/`AppState` 之后,写代码时选一个逻辑顺序
清楚的位置)。

- [ ] **Step 2: `view` 主体**

紧跟 `update` 之后:

```rust
pub fn view(
    app_state: &AppState,
    ws_state: &WorkspaceState,
    project_id: i64,
    tabs: &[SessionTabSummary],
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = column![
        text("Todo")
            .size(workspace_font::title())
            .color(theme::CREAM),
        text(format!("{} 条任务 · .dozer/todo.md", ws_state.items.len()))
            .size(workspace_font::caption())
            .color(theme::DIM),
    ]
    .spacing(4)
    .padding([20, 20]);

    let states: Vec<TodoState> = ws_state
        .items
        .iter()
        .map(|item| {
            let key = todo_line_key(&item.text);
            let dispatch = app_state
                .meta_for(project_id, key)
                .and_then(|m| m.dispatch.as_ref());
            let target_alive = dispatch
                .map(|d| tabs.iter().any(|t| t.session_id == d.session_id && t.alive))
                .unwrap_or(false);
            todo_display_state(item, dispatch, target_alive)
        })
        .collect();
    let visible_idx = filter_todos(&ws_state.items, &states, ws_state.filter, &ws_state.search);

    let existing_tabs: Vec<(&str, String)> = tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| (t.session_id.as_str(), t.title.clone()))
        .collect();

    let toolbar = row![
        todo_filter_segment("全部", TodoFilter::All, ws_state.filter),
        todo_filter_segment("待办", TodoFilter::Pending, ws_state.filter),
        todo_filter_segment("进行中", TodoFilter::InProgress, ws_state.filter),
        todo_filter_segment("完成", TodoFilter::Done, ws_state.filter),
        text_input("搜索任务关键字…", &ws_state.search)
            .on_input(Message::SearchChanged)
            .size(workspace_font::body())
            .width(Length::Fill)
            .style(
                |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                    background: theme::BG.into(),
                    border: Border::default(),
                    icon: theme::DIM,
                    placeholder: theme::DIM,
                    value: theme::CREAM,
                    selection: theme::GOLD,
                }
            ),
    ]
    .spacing(8)
    .padding([12, 20])
    .align_y(iced_widget::core::alignment::Vertical::Center);

    let mut list = column![].spacing(2);
    if visible_idx.is_empty() {
        list = list.push(
            container(
                text("没有匹配的任务")
                    .size(workspace_font::body())
                    .color(theme::DIM),
            )
            .padding([20, 20]),
        );
    } else {
        for &idx in &visible_idx {
            let item = &ws_state.items[idx];
            let key = todo_line_key(&item.text);
            let meta = app_state.meta_for(project_id, key);
            let mut row = None;
            if let Some((editing_idx, draft)) = &ws_state.editing_plan_date
                && *editing_idx == idx
            {
                row = Some(todo_plan_date_edit_row(item, draft));
            }
            match row {
                Some(r) => list = list.push(r),
                None => {
                    list = list.push(todo_row(
                        idx,
                        item,
                        states[idx],
                        meta,
                        ws_state.dispatch_open == Some(idx),
                        &existing_tabs,
                    ));
                }
            }
        }
    }

    let add_row = text_input("＋新增任务…", &ws_state.add_draft)
        .on_input(Message::AddInputChanged)
        .on_submit(Message::AddSubmit)
        .size(workspace_font::body())
        .padding([10, 20])
        .style(
            |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                background: theme::BG.into(),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                icon: theme::DIM,
                placeholder: theme::DIM,
                value: theme::CREAM,
                selection: theme::GOLD,
            },
        );

    let divider = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        });

    let content = column![
        header,
        toolbar,
        scrollable(list).height(Length::Fill),
        divider,
        add_row,
    ]
    .height(Length::Fill);

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BG.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 3: 渲染辅助函数(照抄现有实现,类型/参数按本模块调整)**

紧跟 `view` 之后,把现有 `todo_row`/`todo_dispatch_popup`/`todo_plan_date_edit_row`/
`todo_filter_segment`/`format_todo_time`/`civil_from_days` 六个函数整段搬过来:

- `todo_row`:签名里 `meta: Option<&'a todo_meta::TodoTaskMeta>` 改成
  `meta: Option<&'a TodoTaskMeta>`(同文件内类型,去模块前缀),`Message::TodoToggle`/
  `Message::TodoPlanDateEditStart`/`Message::TodoDispatchOpen` 分别改成
  `Message::Toggle`/`Message::PlanDateEditStart`/`Message::DispatchOpen`,函数体其余部分
  (勾选框/文本删除线/日期标签/派发按钮/进行中标签)原样不动。
- `todo_dispatch_popup`:`Message::TodoDispatchToExisting`/`Message::TodoDispatchNew`
  改成 `Message::DispatchToExisting`/`Message::DispatchNew`,`PickerLaunch` 引用改成
  `crate::workspace::PickerLaunch`,其余不动。
- `todo_plan_date_edit_row`:`Message::TodoPlanDateChanged`/`Message::TodoPlanDateSubmit`
  改成 `Message::PlanDateChanged`/`Message::PlanDateSubmit`,其余不动。
- `todo_filter_segment`:`Message::TodoFilterSet` 改成 `Message::FilterSet`,参数类型
  `todo::TodoFilter` 改成同文件内的 `TodoFilter`,其余不动。
- `format_todo_time`/`civil_from_days`:原样搬,不改一个字(纯日期格式化,不涉及
  `Message`/状态)。

（`app_todo_dispatch_for` 这个现有函数**不用搬**——它的职责已经被 `AppState::meta_for`
+ `view` 里内联的 `.and_then(|m| m.dispatch.as_ref())` 取代,不需要单独保留。）

- [ ] **Step 4: 文件顶部补齐 import**

在文件顶部(Task 1 已有的基础上)追加:

```rust
use crate::{chrome_style, icons, theme, workspace_font};
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{
    button, column, container, row, rich_text, scrollable, span, text, text_input,
};
```

（`chrome_style` 这次 `view` 是否要用它取 `region.padding`/`gap` 之类,对照现有 `todo_pane`
——现有实现里没有用 `chrome_style::todo_pane()` 这种 region 辅助,是手写的 `padding`/
`spacing` 字面量,所以这里大概率不需要 `chrome_style`,写代码时按实际编译报错增删这一行。）

- [ ] **Step 5: 编译,逐条修正**

Run: `cargo check -p dozer-app 2>&1 | grep -A 20 "extensions/todo.rs"`
Expected: 逐条修正类型/import 问题,直到本文件不再报错。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "feat(dozer-app): add todo::view and render helpers"
```

---

### Task 4: 内核接线

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `extensions::todo::{Message, WorkspaceState, AppState, SessionTabSummary, update, reload_from_disk, record_dispatch}`(Task 1-3)

- [x] **Step 1: `use` 与字段合并**

文件顶部加 `use crate::extensions::todo;`(紧跟 `use crate::extensions::browser;`/
`use crate::extensions::git_log;` 之后)。

`App` 结构体(约第 1423-1425 行)把:

```rust
    /// Todo 面板本地元数据（派发记录/计划时间/完成时间），启动时
    /// `todo_meta::load()` 读盘，每次变更后 `todo_meta::save` 落盘。
    todo_meta: todo_meta::TodoMetaState,
```

改成:

```rust
    /// Todo 面板 App 级状态(派发记录/计划时间/完成时间,按项目分桶,
    /// 启动时读盘、每次变更落盘)——见 `extensions::todo::AppState`。
    todo: todo::AppState,
```

`App::bootstrap` 里的 `todo_meta: todo_meta::load(),`(约第 2924 行)改成
`todo: todo::AppState::load(),`。

`Workspace` 结构体把这 8 个字段(约第 1501-1519 行,连同各自文档注释)：

```rust
    todo_items: Vec<todo::TodoItem>,
    todo_mtime: Option<std::time::SystemTime>,
    todo_add_draft: String,
    todo_filter: todo::TodoFilter,
    todo_search: String,
    todo_dispatch_open: Option<usize>,
    todo_pending_dispatch: HashMap<usize, String>,
    todo_editing_plan_date: Option<(usize, String)>,
```

替换成一个:

```rust
    /// Todo 面板 per-project 状态——见 `extensions::todo::WorkspaceState`。
    todo: todo::WorkspaceState,
```

`Workspace` 的 `empty_for_project_placeholder()` 里把这 8 行:

```rust
            todo_items: Vec::new(),
            todo_mtime: None,
            todo_add_draft: String::new(),
            todo_filter: todo::TodoFilter::All,
            todo_search: String::new(),
            todo_dispatch_open: None,
            todo_pending_dispatch: HashMap::new(),
            todo_editing_plan_date: None,
```

改成一行:

```rust
            todo: todo::WorkspaceState::default(),
```

- [x] **Step 2: 顶层 `Message` 枚举**

把 12 个 `Todo*` 变体(约第 974-997 行,连同文档注释)整段删除,换成一个(位置放在原来
`TodoToggle` 所在处):

```rust
    /// Todo 面板的全部消息(派发到已有/新建会话除外——那两条内核直接
    /// 拦截处理,见 `update()` 对应分支),内核只转发不解读——见
    /// `extensions::todo::Message`。
    Todo(todo::Message),
```

- [x] **Step 3: `update()` 分支**

删除现有 `Message::TodoToggle` 到 `Message::TodoPlanDateSubmit` 这 12 段(内容见 Task
之前勘探时读到的原文,整段删掉),换成三支(位置放在原 `Message::TodoToggle` 所在处):

```rust
            Message::Todo(todo::Message::DispatchToExisting(idx, session_id)) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo.items().get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo.close_dispatch_popup();
                    ws.dispatch_todo_to_existing(io, &session_id, &text);
                });
                self.todo.record_dispatch(project_id, &text, session_id);
            }
            Message::Todo(todo::Message::DispatchNew(idx, launch)) => {
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo.items().get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo.close_dispatch_popup();
                    if let Some(tab_id) = ws.spawn_new_tab(io, launch, Some(text.clone())) {
                        ws.todo.insert_pending_dispatch(tab_id, text);
                    }
                });
            }
            Message::Todo(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                // `self.todo`(App 级)和某个 `Workspace` 要同时可变借用,
                // `todo::update` 才能一次处理完两块状态——不能套用
                // `with_focused_project(|ws, _io| ..)` 那种单参数闭包(它只
                // 借出 `ws`,拿不到 `self.todo`)。改用 `loaded_workspace_mut`
                // 直接从 `self.projects` 借 `&mut Workspace`,跟 `&mut self.todo`
                // 是结构体的两个不同字段,互不冲突,Rust 借用检查器允许分别
                // 借用。
                let app_todo = &mut self.todo;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let project_path = std::path::PathBuf::from(&project.path);
                todo::update(&mut ws.todo, app_todo, msg, project_id, &project_path);
            }
```

（`loaded_workspace_mut` 是文件里已有的私有辅助函数,`with_project`/`ensure_loaded` 等
方法内部就是这样从 `self.projects` 按 `project_id` 取 `&mut Workspace` 的。上面
`DispatchToExisting`/`DispatchNew` 两支同理受这条约束——它们调用
`self.todo.record_dispatch(..)` 是在 `self.with_focused_project(..)` **闭包结束之后**
才做,这样两次借用不重叠,能编译过;这次三支修改都要保持"闭包/借用 `ws` 的那段代码块先
结束,再单独借用 `self.todo`"这个顺序,不能在同一个语句里同时借。)

- [x] **Step 4: `Workspace::todo` 需要的新增小方法**

回到 `extensions/todo.rs`(本步骤补 Task 2 遗漏的两个方法,因为 Step 3 的内核代码用到了
它们),`impl WorkspaceState` 追加:

```rust
    pub fn items(&self) -> &[TodoItem] {
        &self.items
    }

    pub fn close_dispatch_popup(&mut self) {
        self.dispatch_open = None;
    }

    pub fn insert_pending_dispatch(&mut self, tab_id: usize, text: String) {
        self.pending_dispatch.insert(tab_id, text);
    }
```

- [x] **Step 5: `Message::TabAttached` 分支的小修改**

现有分支(约第 3983-3997 行)：

```rust
            Message::TabAttached(project_id, tab_id, info, snapshot) => {
                let session_id = info.id.clone();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(io, tab_id, info, snapshot)
                });
                if let Some(text) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.todo_pending_dispatch.remove(&tab_id))
                {
                    self.record_todo_dispatch(project_id, &text, session_id);
                }
            }
```

改成:

```rust
            Message::TabAttached(project_id, tab_id, info, snapshot) => {
                let session_id = info.id.clone();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(io, tab_id, info, snapshot)
                });
                if let Some(text) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.todo.take_pending_dispatch(tab_id))
                {
                    self.todo.record_dispatch(project_id, &text, session_id);
                }
            }
```

(只换了 `ws.todo_pending_dispatch.remove(&tab_id)` → `ws.todo.take_pending_dispatch(tab_id)`、
`self.record_todo_dispatch(..)` → `self.todo.record_dispatch(..)` 两处,其余不动。)

- [x] **Step 6: 删除旧的 `App`/`Workspace` 方法,访问器改用新接口**

删除这些已被 `extensions::todo` 取代的方法整段:`Workspace::reload_todo_from_disk`、
`Workspace::toggle_todo_item`、`Workspace::submit_todo_add`、`App::record_todo_dispatch`、
`App::set_todo_plan_date`、`App::set_todo_completed_at`。

`App::poll_todo_if_visible`(约第 3071-3086 行)改成:

```rust
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let current = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if current != ws.todo.mtime() {
            todo::reload_from_disk(&mut ws.todo, std::path::Path::new(&project.path));
        }
    }
```

（用到的 `WorkspaceState::mtime(&self) -> Option<std::time::SystemTime>` 访问器,回到
`extensions/todo.rs` 的 `impl WorkspaceState` 里再补一个:`pub fn mtime(&self) ->
Option<std::time::SystemTime> { self.mtime }`。）

`App::todo_dispatch_open`(约第 3371-3375 行)改成:

```rust
    pub fn todo_dispatch_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.dispatch_popup_open())
            .unwrap_or(false)
    }
```

- [x] **Step 7: `App::view()` 的 `LeftView::Todo` 分支**

约第 6821 行:

```rust
        LeftView::Todo => todo_pane(app, ws, Length::Fill, zone_pane_border(zone, ac)),
```

改成:

```rust
        LeftView::Todo => {
            let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
                return column![].into();
            };
            let tabs: Vec<todo::SessionTabSummary> = ws
                .tabs
                .iter()
                .map(|t| todo::SessionTabSummary {
                    session_id: t.info.id.clone(),
                    title: tab_title(t.agent, t.cwd.as_deref(), &t.info.name),
                    alive: t.alive,
                })
                .collect();
            todo::view(
                &app.todo,
                &ws.todo,
                project_id,
                &tabs,
                Length::Fill,
                zone_pane_border(zone, ac),
            )
            .map(Message::Todo)
        }
```

**注意**:原 `todo_pane` 在没有打开项目时会怎么表现?现有实现里 `todo_pane` 直接吃
`ws: &Workspace`(不是 `Option`),而这个函数只在真的有 `Workspace` 时才会被调用到——
`ws.project` 理论上恒为 `Some`(见前两个试点计划里已经确认过的"正常促成过的 `Workspace`
恒有 `project: Some(_)`"这条不变式)。上面 `let Some(project_id) = ... else { return
column![].into(); }` 是防御性写法,不应该真的触发;如果写代码时发现这个分支确实从不可能
是 `None`,可以把这层 `Option` 判断去掉、直接 `.expect(...)`,两种写法都符合"不改变可见
行为"的约束,选哪种由实现者根据代码整洁度判断。

- [x] **Step 8: 编译,逐条修正**

Run: `cargo build -p dozer-app 2>&1 | head -200`
Expected: 逐条修正——重点关注 Step 3 里提到的"`self.todo` 与 `ws` 借用冲突"这类问题,
按 Step 3 给出的 `loaded_workspace_mut` 方案处理,**不要**为了绕过借用检查而把 `AppState`
克隆一份再写回去这种取巧办法(那样会在两次读写之间产生数据竞争窗口,即使单线程 UI
更新循环里实际不会触发,也不是干净的解法)。

- [x] **Step 9: 全量测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt --check`
Expected: 全绿,包括 `extensions::todo::` 下 Task 1(原 `todo.rs`/`todo_meta.rs` 的全部
既有测试)+ Task 2(新增单测)全部通过。

- [x] **Step 10: Commit**

```bash
git add -A crates/dozer-app
git commit -m "refactor(dozer-app): route Todo messages through extensions::todo"
```

---

### Task 5: 全量校验与人工验收

**Files:** 无新增/修改(纯校验任务)

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

Run: `cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check`
Expected: 全部 crate 编译通过、测试全绿、无警告、无格式差异。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

打开一个项目,点左图标栏进入 Todo 面板:
1. "＋新增任务"输入文字回车——新任务出现在列表底部,`.dozer/todo.md` 里能看到新行。
2. 勾选一条任务——删除线+暗色,`.dozer/todo.md` 对应行变成 `- [x]`;取消勾选恢复。
3. 筛选(全部/待办/进行中/完成)、搜索框关键字过滤——列表按预期缩小。
4. 待办任务点"派发"→ 选一个已存活的 agent tab——任务文本应该被写进那个终端会话;再点该
   任务的派发按钮,应该看到"进行中"标签(绿点)。
5. 待办任务点"派发"→"新建 agent 会话…"——应新建一个终端 tab 并自动键入任务文本;
   等新会话真正 attach 后,回到 Todo 面板确认该任务也变成"进行中"(验证
   `TabAttached`→`take_pending_dispatch`→`record_dispatch` 这条链路)。
6. 待办/进行中任务点"计划"文字——进入内联编辑,输入日期回车,行内应显示"计划 <日期>";
   重新点开应该预填之前存的值。
7. 勾选任务变完成——该行显示"完成于 <时间>";取消勾选,"完成于"文案消失。
8. 在 `.dozer/todo.md` 里手动(用编辑器,不通过 GUI)加一行 `- [ ] 外部添加的任务`,等
   轮询周期过去(Todo 面板保持打开状态)——新任务应该自动出现,不用手动刷新。
9. 重启 `cargo run -p dozer-app`——之前的派发记录/计划时间/完成时间应该还在(验证
   `todo_meta.json` 真的落了盘)。

若上述任一步与预期不符,对照 Task 2(状态转换逻辑)/Task 4(内核路由,尤其
`self.todo`/`ws` 借用分离那部分)重新核对。

- [ ] **Step 3: 确认没有遗留未提交的改动**

Run: `git status`
Expected: 干净(`.dozer/` 目录下的运行期产物如果被 git 追踪不到就不用管,那是本地测试项目
自己的 todo.md,不属于这次改动)。
