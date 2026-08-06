# Todo 面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 Dozer 加一个跟随项目仓库的 Todo 面板——`.dozer/todo.md` 两态 checklist,agent 可直接
读写;GUI 侧新增 `LeftView::Todo` 面板,支持按状态/关键字筛选、把任务派发给指定 agent tab、
勾选完成、新增任务、以及 GUI 本地记的计划时间/完成时间。

**Architecture:** 纯函数核心(`todo.rs` 的解析/写入/三态推导/筛选,`todo_meta.rs` 的本地
sidecar load/save)+ 薄 GUI 胶水层(`workspace.rs` 里的 `Workspace`/`App` 字段、`Message`
变体、`todo_pane` 渲染、`main.rs` 的轮询唤醒)。文件本身(`.dozer/todo.md`)是唯一的 git 追踪
真相源,agent 与 GUI 都直接读写它;"进行中"状态、派发记录、计划/完成时间都是 GUI 本地派生态/
缓存,不进文件、不进协议。

**Tech Stack:** Rust、iced 0.14(`crates/dozer-app`)、`serde`/`serde_json`(JSON sidecar)、
`std::fs`(同步文件 I/O,文件极小,同 `goal.rs`/`open_projects.rs` 既有惯例)。

## Global Constraints

- 规格来源:`docs/superpowers/specs/2026-08-06-dozer-todo-panel-design.md`(已批准)。本计划
  与 spec 有出入的地方(主要是把 spec 里偏示意性的类型换成代码库里真实存在的类型/签名)会在
  对应任务里注明原因。
- **`crates/dozer-app/src/workspace.rs` 当前 9100+ 行,且这几天有另一个并行会话在持续往
  里加东西**(Agent 面板/Git Shell/图标相关的改动)。本计划里给出的行号只是撰写时刻的参考
  定位,**不保证到执行时还准确**——每个任务落地前先用函数名/字段名/枚举名(而不是行号)
  搜索确认插入点,行号对不上时以搜索结果为准。
- 不引入新依赖(不引入 `notify` 文件监听,不引入 markdown 解析库)——全部复用
  `serde`/`serde_json`/`std::fs`,已经是 `crates/dozer-app` 的既有依赖。
- 中文注释、`cargo fmt`/`cargo clippy --all-targets -- -D warnings` 干净、新增纯函数逻辑
  headless 单测覆盖、GUI 交互效果人工用 `cargo run -p dozer-app` 验收——这四条是本仓库
  贯穿全部既有 P1x/P2x 任务的硬性惯例,本计划每个任务都照做,不再逐条重复。
- 甲方动作专属色 `theme::GOLD`(`#F2D94E`)不能被状态语义(存活/进行中等)借用——这条
  CLAUDE.md 明文规定,`agent_dot_color`/`dot_color` 已经在遵守,Todo 面板的"进行中"用
  `theme::GREEN`(呼应 `dot_color` 里"存活=绿"的既有语义),不用金色。

---

## File Structure

**新建：**
- `crates/dozer-app/src/todo.rs` — `.dozer/todo.md` 解析/写入辅助/三态推导/筛选,纯函数,
  结构镜像 `crates/dozer-app/src/goal.rs`。
- `crates/dozer-app/src/todo_meta.rs` — GUI 本地任务元数据(派发记录/计划时间/完成时间)
  sidecar,结构镜像 `crates/dozer-app/src/open_projects.rs`。
- `crates/dozer-app/assets/icons/list-checks.svg` — 左图标栏 Todo 图标(Lucide `list-checks`,
  与既有图标同款 24×24 / `stroke="currentColor"` / `stroke-width="2"` 规范)。

**修改：**
- `crates/dozer-app/src/icons.rs` — `IconKind` 加 `ListChecks` 变体。
- `crates/dozer-app/src/workspace.rs` — 目前唯一的 GUI 逻辑/渲染大文件,本功能延续现状把
  新代码加进去(不做拆分——这是这个代码库对 `workspace.rs` 的既有做法,H0/Agent 域面板等
  之前的功能都是这样落地的,不在本计划里单独提"该拆文件了")。具体改动点见各任务。
- `crates/dozer-app/src/main.rs` — `about_to_wait`/`new_events` 加一个独立的 Todo 轮询
  唤醒分支,不影响现有闪烁/悬停动画分支。

---

### Task 1: `todo.rs` — `TodoItem` 与 `parse_todo`

**Files:**
- Create: `crates/dozer-app/src/todo.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod todo;`,紧挨着现有 `mod goal;`一类的模块
  声明旁边——用 `grep -n "^mod goal;" crates/dozer-app/src/main.rs` 定位)

**Interfaces:**
- Produces: `pub struct TodoItem { pub text: String, pub done: bool }`、
  `pub fn todo_path(repo: &std::path::Path) -> std::path::PathBuf`、
  `pub fn parse_todo(md: &str) -> Vec<TodoItem>`

- [ ] **Step 1: 写 `todo.rs` 文件头 + 失败的测试**

创建 `crates/dozer-app/src/todo.rs`:

```rust
//! `.dozer/todo.md` 任务列表解析（Todo 面板 design，2026-08-06）：
//! 标准两态 checkbox（`- [ ]`/`- [x]`），git 可追踪，agent 可直接读写。
//! 解析风格镜像 `goal.rs`——手写、宽松，不引入 markdown 库；格式意外
//! （多级缩进、非 checkbox 正文）一律忽略，不因为文件"长得不标准"而失败。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

pub fn todo_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("todo.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_path_is_dot_dozer() {
        assert_eq!(
            todo_path(Path::new("/repo")),
            PathBuf::from("/repo/.dozer/todo.md")
        );
    }

    #[test]
    fn parses_pending_and_done_items() {
        let md = "# Todo\n\n- [ ] 修复登录页闪烁\n- [x] 补 README 安装说明\n";
        let items = parse_todo(md);
        assert_eq!(
            items,
            vec![
                TodoItem { text: "修复登录页闪烁".to_string(), done: false },
                TodoItem { text: "补 README 安装说明".to_string(), done: true },
            ]
        );
    }

    #[test]
    fn ignores_non_checkbox_lines_and_blank_file() {
        let md = "# Todo\n\n正文说明，不是任务。\n- 普通列表项也不算\n  - [ ] 缩进的不算一级\n";
        assert_eq!(parse_todo(md), Vec::new());
        assert_eq!(parse_todo(""), Vec::new());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo::tests -- --nocapture`
Expected: 编译失败，`parse_todo` 未定义（`ignores_non_checkbox_lines_and_blank_file` /
`parses_pending_and_done_items` 用到了它）。

- [ ] **Step 3: 实现 `parse_todo`**

在 `todo_path` 之后加：

```rust
/// 宽松解析：只认一级 `- [ ]`/`- [x]` 列表项（`trim` 后必须以这两个前缀
/// 之一开头——多级缩进的子项 `trim` 后前导空格会被吃掉，但因为前面还有
/// `- [ ]` 的兄弟节点占了行首，不会被误判成一级项，见 `parses_pending_
/// and_done_items` 与 `ignores_non_checkbox_lines_and_blank_file` 两个
/// 测试）；其余行（标题、正文）一律忽略，不因为格式意外而失败。文件不
/// 存在/为空 → 空列表，不是 `Option`（跟 `goal.rs::parse_goal` 不同——
/// todo 没有"整份文件代表一个目标"这种要么有要么没有的语义）。
pub fn parse_todo(md: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- [ ]") {
            items.push(TodoItem { text: rest.trim().to_string(), done: false });
        } else if let Some(rest) = trimmed.strip_prefix("- [x]") {
            items.push(TodoItem { text: rest.trim().to_string(), done: true });
        }
    }
    items
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app todo::tests -- --nocapture`
Expected: 3 个测试全部 PASS。

- [ ] **Step 5: 在 `main.rs` 注册模块**

找到 `main.rs` 里现有的 `mod goal;`（`grep -n "^mod " crates/dozer-app/src/main.rs`），紧
挨着加一行 `mod todo;`。

- [ ] **Step 6: 全量构建确认没有破坏其它东西**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets -- -D warnings`
Expected: 干净，无警告无错误（`todo.rs` 目前还没被任何地方引用，`#![allow(dead_code)]`
不需要现在加——等 Task 8 接入 `workspace.rs` 后自然消解；如果这一步 clippy 因为"从未使用"
报错，在 `todo.rs` 顶部临时加 `#![allow(dead_code)]` 并在 Task 8 完成后删掉这行）。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/todo.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): Todo 面板——todo.rs 文件格式解析(TodoItem/parse_todo)"
```

---

### Task 2: `todo.rs` — `replace_todo_line` 与 `append_todo_item`

**Files:**
- Modify: `crates/dozer-app/src/todo.rs`

**Interfaces:**
- Consumes: 无（纯字符串操作，不依赖 Task 1 之外的任何东西）
- Produces: `pub fn replace_todo_line(content: &str, old_line: &str, new_line: &str) -> Option<String>`、
  `pub fn append_todo_item(content: &str, text: &str) -> String`

- [ ] **Step 1: 写失败的测试**

在 `todo.rs` 的 `mod tests` 里加：

```rust
    #[test]
    fn replace_todo_line_hits_and_replaces() {
        let content = "# Todo\n\n- [ ] 任务A\n- [ ] 任务B\n";
        let out = replace_todo_line(content, "- [ ] 任务A", "- [x] 任务A").unwrap();
        assert_eq!(out, "# Todo\n\n- [x] 任务A\n- [ ] 任务B\n");
    }

    #[test]
    fn replace_todo_line_misses_returns_none() {
        let content = "# Todo\n\n- [ ] 任务A\n";
        assert_eq!(replace_todo_line(content, "- [ ] 不存在的行", "x"), None);
    }

    #[test]
    fn replace_todo_line_only_replaces_first_match() {
        // 已知限制：文件里有多行完全相同的文本时，只替换第一次出现。
        let content = "- [ ] 重复\n- [ ] 重复\n";
        let out = replace_todo_line(content, "- [ ] 重复", "- [x] 重复").unwrap();
        assert_eq!(out, "- [x] 重复\n- [ ] 重复\n");
    }

    #[test]
    fn append_todo_item_to_empty_list() {
        let content = "# Todo\n";
        assert_eq!(append_todo_item(content, "新任务"), "# Todo\n- [ ] 新任务\n");
    }

    #[test]
    fn append_todo_item_after_last_existing_item() {
        let content = "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n";
        assert_eq!(
            append_todo_item(content, "任务C"),
            "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n- [ ] 任务C\n"
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo::tests -- --nocapture`
Expected: 编译失败（两个函数未定义）。

- [ ] **Step 3: 实现两个函数**

```rust
/// 在 `content` 里找到与 `old_line` 逐字节相同的一行（第一次出现），
/// 替换成 `new_line`。找不到（文件已被 agent 并发改过）返回 `None`，
/// 调用方按"冲突，放弃这次写入，强制重读"处理，不是错误（见 design
/// 第 6 节）。
pub fn replace_todo_line(content: &str, old_line: &str, new_line: &str) -> Option<String> {
    let mut found = false;
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        if !found && line == old_line {
            out.push_str(new_line);
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    found.then_some(out)
}

/// 在最后一个 `- [ ]`/`- [x]` 行之后追加一条新任务；纯追加不依赖"找到
/// 匹配行"，冲突面比 `replace_todo_line` 小。文件里一条任务都没有时，
/// 追加在文件末尾（保留原有内容，末尾补一个换行再接新行，避免跟最后
/// 一行内容粘连）。
pub fn append_todo_item(content: &str, text: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let last_item_idx = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("- [ ]") || l.trim_start().starts_with("- [x]"));
    let insert_at = last_item_idx.map(|i| i + 1).unwrap_or(lines.len());
    let mut out = String::with_capacity(content.len() + text.len() + 8);
    for (i, line) in lines.iter().enumerate() {
        if i == insert_at {
            out.push_str("- [ ] ");
            out.push_str(text);
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    if insert_at == lines.len() {
        out.push_str("- [ ] ");
        out.push_str(text);
        out.push('\n');
    }
    out
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app todo::tests -- --nocapture`
Expected: 全部 PASS（含 Task 1 的 3 个）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/todo.rs
git commit -m "feat(dozer-app): Todo 面板——单行定点替换/追加写入辅助"
```

---

### Task 3: `todo.rs` — `todo_line_key`

**Files:**
- Modify: `crates/dozer-app/src/todo.rs`

**Interfaces:**
- Produces: `pub fn todo_line_key(text: &str) -> u64`

- [ ] **Step 1: 写失败的测试**

```rust
    #[test]
    fn todo_line_key_ignores_surrounding_whitespace_but_not_content() {
        assert_eq!(todo_line_key("  任务A  "), todo_line_key("任务A"));
        assert_ne!(todo_line_key("任务A"), todo_line_key("任务B"));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo::tests::todo_line_key -- --nocapture`
Expected: 编译失败。

- [ ] **Step 3: 实现**

文件顶部 `use` 里加 `use std::hash::{Hash, Hasher};`，函数体：

```rust
/// 派发记录/计划时间/完成时间在 GUI 本地 sidecar 里用这个 key 关联到
/// 具体某条任务——不给 markdown 行发明稳定 id（那需要往文件里塞隐藏
/// 标记，agent 编辑时容易破坏），代价是"改了任务文字会跟丢这条的全部
/// 本地元数据"，v1 接受（design 非目标）。用文本 `trim` 后算哈希，不
/// 要求无碰撞，只要求"实践中够用"，同 `AgentKind` 分组等既有哈希用途
/// 的验收标准。
pub fn todo_line_key(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app todo:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/todo.rs
git commit -m "feat(dozer-app): Todo 面板——todo_line_key 稳定哈希"
```

---

### Task 4: `todo_meta.rs` — 本地元数据 sidecar

**Files:**
- Create: `crates/dozer-app/src/todo_meta.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod todo_meta;`，紧挨 `mod open_projects;`）

**Interfaces:**
- Consumes: 无
- Produces: `pub struct DispatchRecord { pub session_id: String, pub dispatched_at: SystemTime }`、
  `pub struct TodoTaskMeta { pub dispatch: Option<DispatchRecord>, pub plan_date: Option<String>,
  pub completed_at: Option<SystemTime> }`、
  `pub type TodoMetaState = HashMap<i64, HashMap<u64, TodoTaskMeta>>`（外层 key 是
  `ProjectId`——`workspace.rs` 里 `pub type ProjectId = i64;` 的别名，这个 crate 内部模块间
  不循环依赖 `workspace` 类型，直接用 `i64`）、`pub fn load() -> TodoMetaState`、
  `pub fn save(state: &TodoMetaState) -> std::io::Result<()>`

**注意（相对 spec 的修正）：** design 文档里 `DispatchRecord` 写的是 `tab_id: usize`。核对
`crates/dozer-app/src/workspace.rs` 实际代码后发现 `Workspace.tabs` 是 `Vec<SessionTab>`，
下标（`usize`）会随关闭/重排其它 tab 而漂移，不是稳定引用；真正稳定的是
`SessionInfo.id: String`（daemon 侧会话 id，见 `crates/dozer-core/src/protocol.rs`）。本任务
改用 `session_id: String`，后面 Task 13 判断"目标 tab 是否还存活"时用
`ws.tabs.iter().any(|t| t.info.id == session_id && t.alive)`，比按下标查更可靠。

- [ ] **Step 1: 写文件 + 失败的测试（直接照抄 `open_projects.rs` 的测试骨架，换成新类型）**

创建 `crates/dozer-app/src/todo_meta.rs`：

```rust
//! Todo 面板 GUI 本地任务元数据（Todo 面板 design 第 3/8 节）：派发
//! 记录 + 计划时间 + 完成时间。跟 `open_projects.rs` 同一挂靠模式——
//! 纯运行时缓存，不进 git，不影响 `.dozer/todo.md` 本身的格式，读失败
//! （不存在/损坏）一律回落空 map，不 panic。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchRecord {
    pub session_id: String,
    pub dispatched_at: SystemTime,
}

/// 三个字段互相独立——只设 `plan_date` 不影响 `dispatch`，反之亦然。
/// `#[serde(default)]` 让老文件缺字段时补 `None` 而不是整份反序列化
/// 失败（同 `ShellLayout` 的既有惯例）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TodoTaskMeta {
    #[serde(default)]
    pub dispatch: Option<DispatchRecord>,
    #[serde(default)]
    pub plan_date: Option<String>,
    #[serde(default)]
    pub completed_at: Option<SystemTime>,
}

pub type TodoMetaState = HashMap<i64, HashMap<u64, TodoTaskMeta>>;

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("todo_meta.json")
}

pub fn load() -> TodoMetaState {
    load_from(&file_path())
}

pub fn save(state: &TodoMetaState) -> io::Result<()> {
    save_to(&file_path(), state)
}

fn load_from(path: &Path) -> TodoMetaState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &TodoMetaState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("TodoMetaState 总能序列化");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), TodoMetaState::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), TodoMetaState::default());
    }

    #[test]
    fn save_then_load_round_trips_and_fields_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo_meta.json");
        let mut state = TodoMetaState::new();
        let mut per_project = HashMap::new();
        // 一条只设了 plan_date，一条只设了 dispatch——验证字段互相独立。
        per_project.insert(
            1,
            TodoTaskMeta {
                plan_date: Some("2026-08-10".to_string()),
                ..Default::default()
            },
        );
        per_project.insert(
            2,
            TodoTaskMeta {
                dispatch: Some(DispatchRecord {
                    session_id: "sess-abc".to_string(),
                    dispatched_at: SystemTime::UNIX_EPOCH,
                }),
                ..Default::default()
            },
        );
        state.insert(42, per_project);
        save_to(&path, &state).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, state);
        assert!(loaded[&42][&1].dispatch.is_none());
        assert!(loaded[&42][&2].plan_date.is_none());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo_meta:: -- --nocapture`
Expected: 编译失败——`tempfile` 是否已是 dev-dependency 需要先确认。

Run: `grep -n "tempfile" crates/dozer-app/Cargo.toml`
如果没有输出（`open_projects.rs` 的测试已经在用 `tempfile::tempdir()`，大概率已经是
`[dev-dependencies]` 里的一员；如果确实没有，加一行 `tempfile = "3"` 到
`crates/dozer-app/Cargo.toml` 的 `[dev-dependencies]` 段，再重新跑）。

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test -p dozer-app todo_meta:: -- --nocapture`
Expected: 3 个测试全部 PASS。

- [ ] **Step 4: 在 `main.rs` 注册模块**

同 Task 1 Step 5 的方式，加 `mod todo_meta;`。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/todo_meta.rs crates/dozer-app/src/main.rs crates/dozer-app/Cargo.toml
git commit -m "feat(dozer-app): Todo 面板——todo_meta.rs 本地元数据 sidecar"
```

---

### Task 5: `todo.rs` — 三态推导

**Files:**
- Modify: `crates/dozer-app/src/todo.rs`

**Interfaces:**
- Consumes: `todo_meta::DispatchRecord`（Task 4）——`todo.rs` 加
  `use crate::todo_meta::DispatchRecord;`
- Produces: `pub enum TodoState { Pending, InProgress, Done }`、
  ```rust
  pub fn todo_display_state(
      item: &TodoItem,
      dispatch: Option<&DispatchRecord>,
      target_alive: bool,
  ) -> TodoState
  ```

- [ ] **Step 1: 写失败的测试**

```rust
    use crate::todo_meta::DispatchRecord;

    fn item(done: bool) -> TodoItem {
        TodoItem { text: "任务".to_string(), done }
    }

    fn record() -> DispatchRecord {
        DispatchRecord {
            session_id: "sess".to_string(),
            dispatched_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn done_item_is_always_done_regardless_of_dispatch() {
        assert_eq!(todo_display_state(&item(true), None, false), TodoState::Done);
        assert_eq!(todo_display_state(&item(true), Some(&record()), true), TodoState::Done);
    }

    #[test]
    fn pending_without_dispatch_is_pending() {
        assert_eq!(todo_display_state(&item(false), None, false), TodoState::Pending);
    }

    #[test]
    fn pending_with_live_dispatch_is_in_progress() {
        assert_eq!(
            todo_display_state(&item(false), Some(&record()), true),
            TodoState::InProgress
        );
    }

    #[test]
    fn pending_with_dead_dispatch_falls_back_to_pending() {
        assert_eq!(
            todo_display_state(&item(false), Some(&record()), false),
            TodoState::Pending
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo::tests -- --nocapture`
Expected: 编译失败。

- [ ] **Step 3: 实现**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    Pending,
    InProgress,
    Done,
}

/// `done` 为真直接 `Done`（不管有没有派发记录——已完成的任务不需要
/// 再关心是谁做的）；否则看有没有派发记录，记录存在且目标 session
/// 仍存活（`target_alive`，调用方传 `ws.tabs.iter().any(|t| t.info.id
/// == dispatch.session_id && t.alive)`）→ `InProgress`；否则（没派发
/// 过，或派发目标已经退出）→ `Pending`。`plan_date`/`completed_at`
/// 不参与这个推导，跟三态是两件事（design 第 4/8 节）。
pub fn todo_display_state(
    item: &TodoItem,
    dispatch: Option<&DispatchRecord>,
    target_alive: bool,
) -> TodoState {
    if item.done {
        return TodoState::Done;
    }
    if dispatch.is_some() && target_alive {
        TodoState::InProgress
    } else {
        TodoState::Pending
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app todo:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/todo.rs
git commit -m "feat(dozer-app): Todo 面板——待办/进行中/完成三态推导"
```

---

### Task 6: `todo.rs` — 筛选与搜索

**Files:**
- Modify: `crates/dozer-app/src/todo.rs`

**Interfaces:**
- Consumes: `TodoItem`、`TodoState`、`todo_display_state`（本文件内 Task 1/5）
- Produces: `pub enum TodoFilter { All, Pending, InProgress, Done }`、
  ```rust
  pub fn filter_todos<'a>(
      items: &'a [TodoItem],
      states: &[TodoState],
      filter: TodoFilter,
      query: &str,
  ) -> Vec<usize>
  ```
  返回筛完之后保留的下标列表（不是拷贝内容本身——调用方渲染时按下标回查 `items`/`states`，
  避免多一份数据的生命周期纠缠）。`states[i]` 必须与 `items[i]` 一一对应，调用方负责保证
  （`workspace.rs` 那边会先对完整列表统一跑一遍 `todo_display_state` 得到 `states`，再传
  进来）。

- [ ] **Step 1: 写失败的测试**

```rust
    fn sample() -> (Vec<TodoItem>, Vec<TodoState>) {
        let items = vec![
            TodoItem { text: "修复登录页闪烁".to_string(), done: false },
            TodoItem { text: "补 README 安装说明".to_string(), done: false },
            TodoItem { text: "移除死代码".to_string(), done: true },
        ];
        let states = vec![TodoState::Pending, TodoState::InProgress, TodoState::Done];
        (items, states)
    }

    #[test]
    fn filter_all_with_empty_query_keeps_everything() {
        let (items, states) = sample();
        assert_eq!(filter_todos(&items, &states, TodoFilter::All, ""), vec![0, 1, 2]);
    }

    #[test]
    fn filter_by_state() {
        let (items, states) = sample();
        assert_eq!(filter_todos(&items, &states, TodoFilter::Done, ""), vec![2]);
        assert_eq!(filter_todos(&items, &states, TodoFilter::InProgress, ""), vec![1]);
    }

    #[test]
    fn filter_by_keyword_case_insensitive_substring() {
        let (items, states) = sample();
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, "readme"),
            vec![1]
        );
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, "登录"),
            vec![0]
        );
    }

    #[test]
    fn filter_combines_state_and_keyword() {
        let (items, states) = sample();
        // "README" 只在下标 1，且下标 1 是 InProgress——命中；换成 Done 就不命中了。
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::InProgress, "readme"),
            vec![1]
        );
        assert!(filter_todos(&items, &states, TodoFilter::Done, "readme").is_empty());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo::tests -- --nocapture`
Expected: 编译失败。

- [ ] **Step 3: 实现**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoFilter {
    All,
    Pending,
    InProgress,
    Done,
}

/// 纯前端过滤：状态相等匹配 + 关键字对 `TodoItem.text` 做大小写不敏感
/// 的子串匹配（空 `query` 不过滤）。作用在"已经解析+推导好状态"的
/// 内存列表上，不碰文件、不碰 sidecar（design 第 7 节）。
pub fn filter_todos<'a>(
    items: &'a [TodoItem],
    states: &[TodoState],
    filter: TodoFilter,
    query: &str,
) -> Vec<usize> {
    let query_lower = query.trim().to_lowercase();
    items
        .iter()
        .zip(states.iter())
        .enumerate()
        .filter(|(_, (_, state))| match filter {
            TodoFilter::All => true,
            TodoFilter::Pending => **state == TodoState::Pending,
            TodoFilter::InProgress => **state == TodoState::InProgress,
            TodoFilter::Done => **state == TodoState::Done,
        })
        .filter(|(_, (item, _))| {
            query_lower.is_empty() || item.text.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app todo:: -- --nocapture`
Expected: 全部 PASS（Task 1-6 累计约 16 个测试）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/todo.rs
git commit -m "feat(dozer-app): Todo 面板——状态筛选 + 关键字搜索纯函数"
```

---

### Task 7: 新增 `IconKind::ListChecks` 图标

**Files:**
- Create: `crates/dozer-app/assets/icons/list-checks.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces: `IconKind::ListChecks` 变体，`icons::view(IconKind::ListChecks, size, color)` 可用

- [ ] **Step 1: 创建 SVG 资源**

创建 `crates/dozer-app/assets/icons/list-checks.svg`（Lucide `list-checks`，与既有图标
逐字节同规范——24×24 viewBox、`stroke="currentColor"`、`stroke-width="2"`、圆头圆角连接）：

```xml
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
  <path d="M16 5H3" />
  <path d="M16 12H3" />
  <path d="M16 19H3" />
  <path d="m21 5-2 2-1-1" />
  <path d="m21 12-2 2-1-1" />
  <path d="m21 19-2 2-1-1" />
</svg>
```

- [ ] **Step 2: `IconKind` 加变体**

在 `crates/dozer-app/src/icons.rs` 的 `pub enum IconKind` 里，`GitBranch,` 那一行之后加：

```rust
    ListChecks,
```

在 `fn bytes(self)` 的 `match` 里，`IconKind::GitBranch => include_bytes!("../assets/icons/git-branch.svg"),`
那一行之后加：

```rust
            IconKind::ListChecks => include_bytes!("../assets/icons/list-checks.svg"),
```

- [ ] **Step 3: 构建确认新变体没漏 match 分支**

Run: `cargo build -p dozer-app`
Expected: 干净——如果 `bytes()` 的 `match` 是穷举式（没有 `_ =>` 兜底分支，看现有代码风格
应该是），漏加会直接编译报错 `non-exhaustive patterns`，能立刻发现。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/icons/list-checks.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): 新增 list-checks 图标(Todo 面板用)"
```

---

### Task 8: `LeftView::Todo` + 左图标栏第三个图标 + 空面板占位

先把"点图标能切到一个空 Todo 面板"这条端到端链路跑通，再在后续任务里往面板里填内容——
避免一次性改动过大、调试困难。

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `RailButton`、`HoverId::Rail`、`rail_icon_button`、`left_icon_rail`、
  `left_panel_area`（均为 `workspace.rs` 既有代码，本任务只加一个变体+一个渲染分支）
- Produces: `LeftView::Todo` 变体、`RailButton::LeftTodo` 变体、
  `fn todo_pane(app: &App, ws: &Workspace, width: Length, border: Border) -> Element<...>`
  （本任务先给占位实现，Task 9 起真正填内容）

- [ ] **Step 1: `LeftView` 加 `Todo` 变体**

搜索 `pub enum LeftView {`（`workspace.rs` 顶部附近，撰写时是第 77 行），改成：

```rust
pub enum LeftView {
    Files,
    Web,
    Todo,
}
```

- [ ] **Step 2: `RailButton` 加 `LeftTodo` 变体**

搜索 `pub enum RailButton {`，改成：

```rust
pub enum RailButton {
    LeftFiles,
    LeftWeb,
    LeftTodo,
    RightAgent,
    RightConversations,
}
```

- [ ] **Step 3: 编译，确认所有既有 `match app.left_view` / `match ... RailButton` 穷举处
  报错**

Run: `cargo build -p dozer-app 2>&1 | grep "non-exhaustive\|workspace.rs"`

Expected: 至少两处报错——`left_panel_area` 里 `match app.left_view { LeftView::Files =>
..., LeftView::Web => ... }`（缺 `Todo` 分支）,以及任何按 `RailButton` 做穷举 `match` 的
地方（目前代码只用 `RailButton` 当 `HashMap` key，大概率没有穷举 `match`，这一步只是
确认——如果编译没报 `RailButton` 相关错，跳过不用管）。

- [ ] **Step 4: `left_panel_area` 补 `LeftView::Todo` 分支（占位实现）**

搜索 `fn left_panel_area`，在其内部 `match app.left_view { LeftView::Files => {...}
LeftView::Web => browser_pane(ws, Length::Fill, zone_pane_border(zone, ac)), }` 的
`LeftView::Web` 分支之后加：

```rust
        LeftView::Todo => todo_pane(app, ws, Length::Fill, zone_pane_border(zone, ac)),
```

在文件里找一处合适的位置（比如 `browser_pane` 函数定义之前）新增占位函数：

```rust
/// 左面板区 Todo 视图：`.dozer/todo.md` 任务列表 + 筛选/搜索 + 派发。
/// Task 8 先占位（空面板，验证图标栏切换链路通），Task 9 起逐步填内容。
fn todo_pane<'a>(
    _app: &'a App,
    _ws: &'a Workspace,
    width: Length,
    border: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    container(text("Todo 面板占位").size(workspace_font::body()).color(theme::DIM))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BG.into()),
            border,
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 5: `left_icon_rail` 加第三个图标**

搜索 `fn left_icon_rail`，在其 `column![ ... ]` 里，`LeftWeb` 那个 `MouseArea::new(...)`
之后（`column!` 宏的下一个元素位置）加：

```rust
        MouseArea::new(rail_icon_button(
            icons::IconKind::ListChecks,
            app.left_view == LeftView::Todo && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftTodo)),
            Message::LeftIconSelect(LeftView::Todo),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftTodo), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftTodo), false)),
```

- [ ] **Step 6: 编译确认干净**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets -- -D warnings
&& cargo fmt -p dozer-app -- --check`
Expected: 全部干净。`Message::LeftIconSelect(v)` 的既有 `update()` 处理逻辑是对
`LeftView` 泛化的（不需要改），点新图标应该已经能切视图——不需要新增 `Message` 变体。

- [ ] **Step 7: 人工验收**

Run: `cargo run -p dozer-app`
Expected: 左图标栏出现第三个图标（list-checks 线框图），点击后左面板区显示"Todo 面板
占位"灰字，再点一次 Files/Web 图标能正常切回去；再点一次 Todo 图标（已选中态）能收起
左面板区，语义与 Files/Web 完全一致。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Todo 面板——LeftView::Todo 挂载 + 左图标栏第三个图标(占位面板)"
```

---

### Task 9: 加载 + 渲染任务列表（无派发/无日期，先只读）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `todo::{TodoItem, parse_todo, todo_path}`（Task 1）
- Produces: `Workspace.todo_items: Vec<todo::TodoItem>`、
  `Workspace.todo_mtime: Option<std::time::SystemTime>`、
  `Workspace::reload_todo_from_disk(&mut self)`、`todo_pane` 真正渲染列表（勾选框+文本，
  勾选交互留到 Task 10）

- [ ] **Step 1: `Workspace` 结构体加两个字段**

搜索 `agent_picker_open: bool,`（`Workspace` 结构体定义里，紧跟其后有一段注释"这份
`Workspace` 是否只是..."），在它之前或之后加：

```rust
    /// `.dozer/todo.md` 解析后的内存缓存，`reload_todo_from_disk` 刷新。
    todo_items: Vec<todo::TodoItem>,
    /// 上一次成功读取时 `.dozer/todo.md` 的 mtime，轮询靠比较它决定要不要
    /// 重读（`App::poll_todo_if_visible`，Task 11）。`None` = 还没读过，
    /// 或者文件不存在。
    todo_mtime: Option<std::time::SystemTime>,
```

同时找到 `Workspace` 的构造处（`grep -n "agent_picker_open: false" crates/dozer-app/src/
workspace.rs` 定位所有构造 `Workspace` 字面量的地方——多项目并行功能下 `Workspace` 可能
不止一处构造，例如新建项目、`from_restore` 促成等，每一处都要补上新字段），各处加：

```rust
            todo_items: Vec::new(),
            todo_mtime: None,
```

- [ ] **Step 2: 编译，用报错列出所有需要补字段的构造点**

Run: `cargo build -p dozer-app 2>&1 | grep "missing field"`
Expected: 列出所有遗漏的 `Workspace { ... }` 字面量位置，逐一按 Step 1 的方式补上，直到
这条命令没有输出。

- [ ] **Step 3: 实现 `reload_todo_from_disk`**

在 `impl Workspace` 块里（挨着 `spawn_new_tab` 之类的方法）加：

```rust
    /// 从磁盘重新读取并解析 `.dozer/todo.md`，刷新 `todo_items`/
    /// `todo_mtime`。文件不存在/读失败按"空列表"处理，不 panic、不报
    /// 错——同 `load_project_goal` 的既有惯例（P1f D2："目标文件极小，
    /// 可容忍同步读"，Todo 面板同理）。
    fn reload_todo_from_disk(&mut self) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        self.todo_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let md = std::fs::read_to_string(&path).unwrap_or_default();
        self.todo_items = todo::parse_todo(&md);
    }
```

- [ ] **Step 4: 切到 Todo 视图时触发加载**

搜索 `Message::LeftIconSelect(v) => {`，在 `self.left_view = v; self.left_collapsed =
false;` 那个 `else` 分支里加一行：

```rust
                    self.left_view = v;
                    self.left_collapsed = false;
                    if v == LeftView::Todo {
                        self.with_focused_project(|ws, _io| ws.reload_todo_from_disk());
                    }
```

- [ ] **Step 5: `todo_pane` 真正渲染列表**

`todo_row` 用到 `rich_text!`/`span`（完成态删除线）——搜索 `workspace.rs` 顶部现有的
`use iced_widget::{...}`（`grep -n "^use iced_widget" crates/dozer-app/src/workspace.rs`），
补上 `rich_text` 和 `span`（跟 `text`/`row`/`column` 等既有导入放一起）。

把 Task 8 写的占位实现整个替换成：

```rust
/// 左面板区 Todo 视图：`.dozer/todo.md` 任务列表 + 筛选/搜索（Task 12）
/// + 派发（Task 13）+ 计划/完成时间（Task 14）。本任务只做只读列表
/// 渲染 + 勾选框点击。
fn todo_pane<'a>(
    _app: &'a App,
    ws: &'a Workspace,
    width: Length,
    border: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = column![
        text("Todo").size(workspace_font::title()).color(theme::CREAM),
        text(format!("{} 条任务 · .dozer/todo.md", ws.todo_items.len()))
            .size(workspace_font::caption()).color(theme::DIM),
    ]
    .spacing(4)
    .padding([20, 20]);

    let mut list = column![].spacing(2);
    if ws.todo_items.is_empty() {
        list = list.push(
            container(text("还没有 todo").size(workspace_font::body()).color(theme::DIM))
                .padding([20, 20]),
        );
    } else {
        for (idx, item) in ws.todo_items.iter().enumerate() {
            list = list.push(todo_row(idx, item));
        }
    }

    let content = column![header, scrollable(list).height(Length::Fill)].height(Length::Fill);

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BG.into()),
            border,
            ..container::Style::default()
        })
        .into()
}

/// 单条任务行：勾选框（完成态实心打勾/未完成态空心框）+ 任务文本
/// （完成态删除线+暗色）。派发按钮/计划时间/完成时间留给后续任务
/// 往这里加（Task 13/14 会重写这个函数加更多参数，但勾选框/文本这
/// 两块视觉在本任务里就是最终实现，不是占位——没有"留给下个任务
/// 补"这回事，避免自我循环引用）。
///
/// 完成态删除线用 `rich_text!`/`span`（`iced_widget::text::Text` 的
/// 简单 `text()` 构造器不支持 `.strikethrough()`，这个方法只存在于
/// `iced_core::text::Span`，见 `iced_core-0.14.0/src/text.rs`）。
fn todo_row(idx: usize, item: &todo::TodoItem) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let box_color = if item.done { theme::BORDER } else { theme::DIM };
    let checkbox = container(if item.done {
        text("✓").size(workspace_font::caption()).color(theme::DIM).into()
    } else {
        Element::from(iced_widget::space::Space::new())
    })
    .width(Length::Fixed(18.0))
    .height(Length::Fixed(18.0))
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: if item.done { Some(theme::BORDER.into()) } else { None },
        border: Border { color: box_color, width: 1.5, radius: 4.0.into() },
        ..container::Style::default()
    });

    let label_color = if item.done { theme::DIM } else { theme::CREAM };
    let label: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = if item.done {
        rich_text![
            span(item.text.clone())
                .size(workspace_font::body())
                .color(label_color)
                .strikethrough(true)
        ]
        .into()
    } else {
        text(item.text.clone()).size(workspace_font::body()).color(label_color).into()
    };

    button(
        row![checkbox, label]
            .spacing(10)
            .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::TodoToggle(idx))
    .width(Length::Fill)
    .padding([10, 20])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::CREAM,
        ..button::Style::default()
    })
    .into()
}
```

注意这一步引用了 `Message::TodoToggle`，但还没定义——这一步先不用管编译报错，下一步
（Step 6）会把 `Message` 变体、`update()` 处理分支、真正的写入逻辑一次性补齐，两步
合起来才是可编译状态。

- [ ] **Step 6: `Message` 加 `TodoToggle` 变体 + `update()` 里加处理分支**

搜索 `pub enum Message {`，找一处相邻语义的位置（比如 `AgentPickerSelect` 附近）加：

```rust
    /// 点击任务行的勾选框：`usize` 是 `Workspace.todo_items` 里的下标。
    TodoToggle(usize),
```

在 `impl App { fn update(...) }` 的 `match message` 里加处理分支：

```rust
            Message::TodoToggle(idx) => {
                self.with_focused_project(|ws, _io| ws.toggle_todo_item(idx));
            }
```

在 `impl Workspace` 里加：

```rust
    /// 勾选/取消勾选第 `idx` 条任务：算出新行文本、用
    /// `todo::replace_todo_line` 定点替换、写回磁盘、重新解析刷新内存
    /// 态。找不到要替换的原始行（文件已被 agent 并发改过）时静默放弃
    /// 这次操作、强制走一次 `reload_todo_from_disk`（design 第 6 节，
    /// 冲突不是错误）。
    fn toggle_todo_item(&mut self, idx: usize) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let Some(item) = self.todo_items.get(idx) else {
            return;
        };
        let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
        let new_line = format!("- [{}] {}", if item.done { " " } else { "x" }, item.text);
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        match todo::replace_todo_line(&content, &old_line, &new_line) {
            Some(new_content) => {
                if let Err(e) = std::fs::write(&path, &new_content) {
                    tracing::warn!("写入 todo.md 失败: {e}");
                    return;
                }
                self.reload_todo_from_disk();
            }
            None => {
                // 冲突：文件已经变了，放弃这次写入，直接重读展示最新状态。
                self.reload_todo_from_disk();
            }
        }
    }
```

- [ ] **Step 7: 编译 + 跑既有测试确认没有回归**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全部干净、全部 PASS（这一步没加新的纯函数单测——`toggle_todo_item`/
`reload_todo_from_disk` 涉及真实文件 I/O 和 `Workspace` 状态，按这个代码库的既有惯例
留给人工验收，纯逻辑已经在 Task 1-2 的 `replace_todo_line`/`parse_todo` 测过）。

- [ ] **Step 8: 人工验收**

在任意一个已经 `git init` 过的目录手动建一个 `.dozer/todo.md`：

```bash
mkdir -p /tmp/todo-smoke/.dozer
cat > /tmp/todo-smoke/.dozer/todo.md <<'EOF'
# Todo

- [ ] 任务一
- [x] 任务二
EOF
```

在 Dozer 里打开 `/tmp/todo-smoke` 作为项目，点左图标栏 Todo 图标，确认能看到两行任务
（"任务一"未勾选、"任务二"已勾选，颜色有区分）；点"任务一"整行，确认它变成勾选态且
`cat /tmp/todo-smoke/.dozer/todo.md` 显示 `- [x] 任务一`；再点一次确认能勾回去。手动
在文件里改动（比如加一行新任务）、切到别的图标栏视图再切回 Todo，确认改动被读到。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Todo 面板——加载+渲染任务列表,勾选框读写打通"
```

---

### Task 10: 新增任务输入行

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `todo::append_todo_item`（Task 2）
- Produces: `Workspace.todo_add_draft: String`、`Message::TodoAddInputChanged(String)`、
  `Message::TodoAddSubmit`

- [ ] **Step 1: `Workspace` 加草稿字段**

同 Task 9 Step 1 的方式，加字段 + 所有构造点补默认值：

```rust
    /// "＋新增任务"输入框当前内容（未提交）。
    todo_add_draft: String,
```

构造点补 `todo_add_draft: String::new(),`。

- [ ] **Step 2: `Message` 加两个变体**

```rust
    /// "＋新增任务"输入框内容变化。
    TodoAddInputChanged(String),
    /// 提交新增（回车）。
    TodoAddSubmit,
```

- [ ] **Step 3: `update()` 加处理分支**

```rust
            Message::TodoAddInputChanged(s) => {
                self.with_focused_project(|ws, _io| ws.todo_add_draft = s);
            }
            Message::TodoAddSubmit => {
                self.with_focused_project(|ws, _io| ws.submit_todo_add());
            }
```

`impl Workspace` 加：

```rust
    /// 提交"＋新增任务"输入框：草稿为空/全空白时不动作（不追加空任务）。
    /// 用 `todo::append_todo_item` 纯追加，冲突面比 `replace_todo_line`
    /// 小——不需要处理"找不到原始行"的分支。
    fn submit_todo_add(&mut self) {
        let text = self.todo_add_draft.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let new_content = todo::append_todo_item(&content, &text);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!("创建 .dozer 目录失败: {e}");
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, &new_content) {
            tracing::warn!("写入 todo.md 失败: {e}");
            return;
        }
        self.todo_add_draft.clear();
        self.reload_todo_from_disk();
    }
```

- [ ] **Step 4: `todo_pane` 底部加输入行**

在 `todo_pane` 的 `content` 组装里，`scrollable(list)` 之后加一行输入框：

```rust
    let add_row = text_input("＋新增任务…", &ws.todo_add_draft)
        .on_input(Message::TodoAddInputChanged)
        .on_submit(Message::TodoAddSubmit)
        .size(workspace_font::body())
        .padding([10, 20])
        .style(|_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
            background: theme::BG.into(),
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() },
            icon: theme::DIM,
            placeholder: theme::DIM,
            value: theme::CREAM,
            selection: theme::GOLD,
        });

    let content = column![
        header,
        scrollable(list).height(Length::Fill),
        rule::horizontal(1).style(|_t: &iced_widget::Theme| rule::Style {
            color: theme::BORDER,
            width: 1,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
        }),
        add_row,
    ]
    .height(Length::Fill);
```

需要确认 `text_input`/`rule` 是否已经在文件顶部 `use iced_widget::{...}` 里引入——
`grep -n "^use iced_widget" crates/dozer-app/src/workspace.rs` 看现有 `use` 块，没有的话
补上 `text_input` 和 `rule`（这个代码库项目树"新建文件"那处内联编辑框已经在用
`text_input`，`grep -n "text_input(" crates/dozer-app/src/workspace.rs` 能找到一个可以
照抄样式的现成例子，风格对齐那里而不是本步骤给的示例——示例是保证类型/字段名正确的
最小可用版本，视觉细节以项目树那个为准）。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全部干净、全部 PASS。

- [ ] **Step 6: 人工验收**

沿用 Task 9 Step 8 的 `/tmp/todo-smoke` 项目，在输入框里打字、回车，确认新任务出现在
列表末尾且 `.dozer/todo.md` 文件末尾真的多了一行 `- [ ] ...`；输入框回车后应清空。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Todo 面板——新增任务输入行"
```

---

### Task 11: mtime 轮询同步

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `Workspace::reload_todo_from_disk`（Task 9，改成 `pub(crate)` 或加一个
  `pub(crate)` 包装，main.rs 需要跨模块调用 `App` 上的方法）
- Produces: `App::todo_panel_visible(&self) -> bool`、`App::poll_todo_if_visible(&mut self)`、
  `main.rs` 里的 `TODO_POLL_INTERVAL` 常量 + `about_to_wait`/`new_events` 新分支

- [ ] **Step 1: `App` 加两个方法**

在 `impl App` 里，挨着 `pub fn active_workspace_mut` 加：

```rust
    /// 当前是否"正看着"某个项目的 Todo 面板——轮询是否要继续排下一拍
    /// 唤醒的判断条件（`main.rs::about_to_wait`），跟 `any_blinking`/
    /// `any_hover_anim_active` 同一层级。
    pub fn todo_panel_visible(&self) -> bool {
        self.left_view == LeftView::Todo && self.active_workspace().is_some()
    }

    /// `main.rs` 定时唤醒调用：只在 `todo_panel_visible()` 时才真的
    /// `stat` 一下 `.dozer/todo.md` 的 mtime；没变就是一次系统调用，
    /// 变了才重读+reparse（`reload_todo_from_disk` 内部也会再 stat 一次
    /// mtime，这里的 stat 只用来判断"要不要触发重读"，多一次系统调用
    /// 换取 `reload_todo_from_disk` 保持独立可复用，成本可忽略）。
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
        if current != ws.todo_mtime {
            ws.reload_todo_from_disk();
        }
    }
```

`reload_todo_from_disk` 在 Task 9 里定义成了私有方法（同一个 `impl Workspace` 块内被
`poll_todo_if_visible` 调用没问题，两者都在 `workspace.rs` 同一个 crate 内——不需要改
可见性，`pub fn poll_todo_if_visible` 在 `workspace.rs` 内部调用 `Workspace` 的私有方法
是允许的，只有 `main.rs` 那边调用 `App::poll_todo_if_visible` 才需要 `App` 这一层是
`pub`）。

- [ ] **Step 2: `main.rs` 加常量 + 两个新分支**

搜索 `const HOVER_ANIM_INTERVAL: Duration = Duration::from_millis(16);`，之后加：

```rust
/// Todo 面板 mtime 轮询间隔（design 第 5 节："约 1s"）。只在
/// `App::todo_panel_visible()` 时才排这一档唤醒，没人看这个面板时不
/// 产生任何后台开销。
const TODO_POLL_INTERVAL: Duration = Duration::from_millis(1000);
```

搜索 `fn new_events`，在 `app.toggle_blink();` 之后、`window.request_redraw();` 之前加：

```rust
                app.poll_todo_if_visible();
```

搜索 `fn about_to_wait`，把：

```rust
                if app.any_blinking() || app.any_hover_anim_active() {
                    let interval = if app.any_hover_anim_active() {
                        HOVER_ANIM_INTERVAL
                    } else {
                        BLINK_INTERVAL
                    };
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        std::time::Instant::now() + interval,
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
```

改成：

```rust
                if app.any_blinking() || app.any_hover_anim_active() || app.todo_panel_visible() {
                    // 三档定时唤醒共用同一个 `ControlFlow::WaitUntil`：悬停动画
                    // 最密（跟手），闪烁其次，Todo 轮询最松（design 第 5 节
                    // "约 1s"，不需要跟手）。优先级只影响挑哪个 interval，
                    // `new_events` 里三件事都会做，不冲突。
                    let interval = if app.any_hover_anim_active() {
                        HOVER_ANIM_INTERVAL
                    } else if app.any_blinking() {
                        BLINK_INTERVAL
                    } else {
                        TODO_POLL_INTERVAL
                    };
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        std::time::Instant::now() + interval,
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
```

- [ ] **Step 3: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全部干净、全部 PASS。

- [ ] **Step 4: 人工验收**

用 Task 9 的 `/tmp/todo-smoke` 项目，打开 Todo 面板，**不切走**，用另一个终端/编辑器
直接改 `/tmp/todo-smoke/.dozer/todo.md`（模拟 agent 编辑）——比如加一行新任务、把某行
`[ ]` 改成 `[x]`——大约 1 秒内面板应该自动刷新显示最新内容，不需要手动切走再切回来。
切到 Files/Web 视图后再改文件，确认面板不可见时不会（也不需要）自动刷新；切回 Todo
应该立刻显示最新内容（Task 9 Step 4 的"切入即重读"逻辑兜底）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): Todo 面板——mtime 轮询同步,面板可见时才排唤醒"
```

---

### Task 12: 筛选 + 搜索工具栏

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `todo::{TodoFilter, TodoState, todo_display_state, filter_todos}`（Task 5/6）
- Produces: `Workspace.todo_filter: todo::TodoFilter`、`Workspace.todo_search: String`、
  `Message::TodoFilterSet(todo::TodoFilter)`、`Message::TodoSearchChanged(String)`

- [ ] **Step 1: `Workspace` 加两个字段**

同前面任务的模式，加字段 + 补所有构造点：

```rust
    /// 当前状态筛选（全部/待办/进行中/完成），纯前端状态，不持久化。
    todo_filter: todo::TodoFilter,
    /// 搜索框当前关键字，纯前端状态，不持久化。
    todo_search: String,
```

`todo_filter: todo::TodoFilter::All,`、`todo_search: String::new(),`。

- [ ] **Step 2: `Message` 加两个变体 + `update()` 处理**

```rust
    /// 点击筛选分段（全部/待办/进行中/完成）。
    TodoFilterSet(todo::TodoFilter),
    /// 搜索框内容变化。
    TodoSearchChanged(String),
```

```rust
            Message::TodoFilterSet(f) => {
                self.with_focused_project(|ws, _io| ws.todo_filter = f);
            }
            Message::TodoSearchChanged(s) => {
                self.with_focused_project(|ws, _io| ws.todo_search = s);
            }
```

- [ ] **Step 3: `todo_pane` 接入筛选**

在 `todo_pane` 里，`header` 之后、渲染 `list` 之前，插入工具栏并把渲染循环换成基于
`filter_todos` 的下标列表：

```rust
    let states: Vec<todo::TodoState> = ws
        .todo_items
        .iter()
        .map(|item| {
            let key = todo::todo_line_key(&item.text);
            let dispatch = app_todo_dispatch_for(ws, key); // Task 13 起才有真正的派发数据；
                                                             // 在那之前先传 None，见下方说明。
            let target_alive = dispatch
                .map(|d| ws.tabs.iter().any(|t| t.info.id == d.session_id && t.alive))
                .unwrap_or(false);
            todo::todo_display_state(item, dispatch, target_alive)
        })
        .collect();
    let visible_idx = todo::filter_todos(&ws.todo_items, &states, ws.todo_filter, &ws.todo_search);

    let toolbar = row![
        todo_filter_segment("全部", todo::TodoFilter::All, ws.todo_filter),
        todo_filter_segment("待办", todo::TodoFilter::Pending, ws.todo_filter),
        todo_filter_segment("进行中", todo::TodoFilter::InProgress, ws.todo_filter),
        todo_filter_segment("完成", todo::TodoFilter::Done, ws.todo_filter),
        text_input("搜索任务关键字…", &ws.todo_search)
            .on_input(Message::TodoSearchChanged)
            .size(workspace_font::body())
            .width(Length::Fill),
    ]
    .spacing(8)
    .padding([12, 20])
    .align_y(iced_widget::core::Alignment::Center);

    let mut list = column![].spacing(2);
    if visible_idx.is_empty() {
        list = list.push(
            container(text("没有匹配的任务").size(workspace_font::body()).color(theme::DIM))
                .padding([20, 20]),
        );
    } else {
        for &idx in &visible_idx {
            list = list.push(todo_row(idx, &ws.todo_items[idx]));
        }
    }
```

**关于 `app_todo_dispatch_for` 的说明：** 这个函数在本任务里还不存在——Task 13 才会
真正实现"按 `todo_line_key` 查 `App.todo_meta` 拿 `DispatchRecord`"。本任务先写一个
最小占位，让上面这段代码能编译过、筛选/搜索链路能跑通（此时"进行中"筛选永远筛不出
东西，这是预期的，Task 13 之后才会有真数据）：

```rust
/// Task 13 会把这个换成真正查 `App.todo_meta` 的实现；本任务先占位
/// 返回 `None`，让筛选/搜索的编译+交互链路能先跑通。
fn app_todo_dispatch_for<'a>(
    _ws: &'a Workspace,
    _key: u64,
) -> Option<&'a todo_meta::DispatchRecord> {
    None
}
```

`fn todo_filter_segment` 辅助函数（渲染一个分段按钮，选中态高亮）：

```rust
fn todo_filter_segment<'a>(
    label: &'a str,
    value: todo::TodoFilter,
    current: todo::TodoFilter,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let active = value == current;
    button(text(label).size(workspace_font::caption()).color(if active {
        theme::CREAM
    } else {
        theme::DIM
    }))
    .on_press(Message::TodoFilterSet(value))
    .padding([4, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active { Some(theme::CARD.into()) } else { None },
        text_color: if active { theme::CREAM } else { theme::DIM },
        border: Border { radius: 5.0.into(), ..Border::default() },
        ..button::Style::default()
    })
    .into()
}
```

`content` 组装里把 `header` 之后加 `toolbar`：

```rust
    let content = column![header, toolbar, scrollable(list).height(Length::Fill), /* ...原有 rule/add_row */]
        .height(Length::Fill);
```

`todo::TodoFilter` 需要能 `Copy`（上面 `todo_filter_segment(..., current: todo::
TodoFilter)` 按值传）——Task 6 定义时已经 `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`，
不需要改。

- [ ] **Step 4: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全部干净、全部 PASS。

- [ ] **Step 5: 人工验收**

`/tmp/todo-smoke/.dozer/todo.md` 加几条不同状态/不同文字的任务，验证：点"待办"/"完成"
分段只显示对应状态的任务；在搜索框打字，列表实时按关键字收窄；筛选+搜索组合使用效果
符合预期；清空搜索框恢复全部（在当前筛选范围内）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Todo 面板——状态筛选 + 搜索工具栏接入 UI"
```

---

### Task 13: 派发到 agent tab

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `todo_meta::{TodoTaskMeta, DispatchRecord, TodoMetaState, load, save}`（Task 4）、
  `PickerLaunch`、`picker_launch_command`、`spawn_new_tab`（既有，本任务给
  `spawn_new_tab` 加一个参数）
- Produces: `App.todo_meta: todo_meta::TodoMetaState`、`Workspace.todo_dispatch_open:
  Option<usize>`、`Message::{TodoDispatchOpen(usize), TodoDispatchClose,
  TodoDispatchToExisting(usize, String), TodoDispatchNew(usize, PickerLaunch)}`

**注意（相对 spec 的修正）：** design 文档第 3 节写"launch 参数已经支持自定义初始命令"——
核对当前 `spawn_new_tab`/`picker_launch_command` 实际代码后，`PickerLaunch` 目前只有
`Agent(Option<AgentKind>)`/`Git` 两种，不支持任意文本。本任务给 `spawn_new_tab` 加一个
独立的 `follow_up: Option<String>` 参数（而不是往 `PickerLaunch` 里塞一个 `Custom(String)`
变体）：`PickerLaunch` 语义上是"选了哪个 agent/shell"，跟"打开后再追加键入一段任务文本"
是两件事，混在一个枚举里会让 `picker_launch_command` 这个纯函数背上"要不要考虑任务文本"
的额外分支。`spawn_new_tab` 目前只有一个调用点（`Message::AgentPickerSelect`），改动面
很小。**这个函数最近正被另一个并行会话频繁触碰（Agent 面板/Git Shell 相关工作），落地
前务必先重新 `grep -n "fn spawn_new_tab" crates/dozer-app/src/workspace.rs` 确认最新签名
和调用点数量，如果已经不是本计划撰写时的样子，按实际签名调整这里的改法，思路不变。**

- [ ] **Step 1: `spawn_new_tab` 加 `follow_up` 参数**

搜索 `fn spawn_new_tab(&mut self, io: &ShellIo, launch: PickerLaunch) {`，改签名为：

```rust
    fn spawn_new_tab(&mut self, io: &ShellIo, launch: PickerLaunch, follow_up: Option<String>) {
```

函数体内部，搜索：

```rust
                    if let Some(cmd) = picker_launch_command(launch) {
                        let bytes = format!("{cmd}\n").into_bytes();
                        if let Err(e) = client.write(&session_id, &bytes).await {
                            tracing::warn!("自动键入初始命令失败: {e}");
                        }
                    }
                    forward_events(project_id, tab_id, rx, proxy).await;
```

改成：

```rust
                    if let Some(cmd) = picker_launch_command(launch) {
                        let bytes = format!("{cmd}\n").into_bytes();
                        if let Err(e) = client.write(&session_id, &bytes).await {
                            tracing::warn!("自动键入初始命令失败: {e}");
                        }
                    }
                    if let Some(text) = follow_up {
                        let bytes = format!("{text}\n").into_bytes();
                        if let Err(e) = client.write(&session_id, &bytes).await {
                            tracing::warn!("派发任务文本失败: {e}");
                        }
                    }
                    forward_events(project_id, tab_id, rx, proxy).await;
```

`follow_up` 需要 `move` 进这个 `async move` 块——它已经是按值传入 `spawn_new_tab` 的，
闭包捕获链路不用额外处理（`launch`/`follow_up` 都在同一个 `async move { ... }` 之前的
作用域里按值使用）。

搜索唯一的既有调用点 `Message::AgentPickerSelect(agent) => { ... ws.spawn_new_tab(io,
agent); }`，改成：

```rust
            Message::AgentPickerSelect(agent) => {
                self.with_focused_project(|ws, io| {
                    ws.agent_picker_open = false;
                    ws.spawn_new_tab(io, agent, None);
                });
            }
```

- [ ] **Step 2: 编译确认签名改动闭环**

Run: `cargo build -p dozer-app 2>&1 | grep "spawn_new_tab\|error"`
Expected: 干净（如果这时候发现除了 `Message::AgentPickerSelect` 之外还有别的调用点——
大概率是另一个并行会话新加的，比如某个"重试"按钮——按同样的方式补上第三个参数，多数
情况传 `None`）。

- [ ] **Step 3: `App` 加 `todo_meta` 字段 + 启动时加载**

搜索 `App` 结构体里 `hover_anims: std::collections::HashMap<HoverId, HoverAnim>,` 那一行，
之后加：

```rust
    /// Todo 面板本地元数据（派发记录/计划时间/完成时间），启动时
    /// `todo_meta::load()` 读盘，每次变更后 `todo_meta::save` 落盘
    /// （design 第 3/8 节）。
    todo_meta: todo_meta::TodoMetaState,
```

搜索 `hover_anims: std::collections::HashMap::new(),`（`App` 构造处），之后加：

```rust
            todo_meta: todo_meta::load(),
```

- [ ] **Step 4: `Workspace` 加 `todo_dispatch_open` 字段**

同前面任务模式：

```rust
    /// 当前打开着派发选择层的任务下标（`None` = 未打开任何派发层）。
    todo_dispatch_open: Option<usize>,
```

构造点补 `todo_dispatch_open: None,`。

- [ ] **Step 5: `Message` 加四个变体**

```rust
    /// 点击某条任务的"派发"按钮：打开派发选择层。
    TodoDispatchOpen(usize),
    /// 点击选择层外/Esc：关闭不派发。
    TodoDispatchClose,
    /// 选中一个已存活的 agent tab 派发：(任务下标, 目标 session id)。
    TodoDispatchToExisting(usize, String),
    /// 选"新建"派发：(任务下标, 要新建的 agent/shell 选项)。
    TodoDispatchNew(usize, PickerLaunch),
```

- [ ] **Step 6: `update()` 加四个处理分支**

```rust
            Message::TodoDispatchOpen(idx) => {
                self.with_focused_project(|ws, _io| ws.todo_dispatch_open = Some(idx));
            }
            Message::TodoDispatchClose => {
                self.with_focused_project(|ws, _io| ws.todo_dispatch_open = None);
            }
            Message::TodoDispatchToExisting(idx, session_id) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo_dispatch_open = None;
                    ws.dispatch_todo_to_existing(io, &session_id, &text);
                });
                self.record_todo_dispatch(project_id, &text, session_id);
            }
            Message::TodoDispatchNew(idx, launch) => {
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo_dispatch_open = None;
                    ws.spawn_new_tab(io, launch, Some(text));
                });
                // 新建 tab 的 session id 要等异步 attach 完成才知道
                // （`Message::TabAttached`），派发记录延后到那时候补记，
                // 见 Step 7。
            }
```

`self.active_project_id`/`self.active_workspace()` 是 `App` 上既有方法/字段
（`active_workspace` 前面 Task 11 已经用过）。

在 `impl Workspace` 里加：

```rust
    /// 把 `text` 当输入写进已存活的 `session_id` 对应 tab——跟
    /// `send_input`（写给当前激活 tab）同一条通路，区别是目标 session
    /// 由参数指定而不是 `self.active`。派发目标可能在选择弹层打开期间
    /// 被用户关掉（tab 已经不在 `self.tabs` 里了）——静默跳过，同
    /// `send_input` 对"tab 已消失"的既有降级路径。
    fn dispatch_todo_to_existing(&self, io: &ShellIo, session_id: &str, text: &str) {
        let Some(tab) = self.tabs.iter().find(|t| t.info.id == session_id) else {
            return;
        };
        if !tab.alive {
            return;
        }
        let client = io.client.clone();
        let id = session_id.to_string();
        let bytes = format!("{text}\n").into_bytes();
        io.handle.spawn(async move {
            if let Err(e) = client.write(&id, &bytes).await {
                tracing::warn!("派发任务文本失败: {e}");
            }
        });
    }
```

`io.client`/`io.handle` 是 `ShellIo` 的私有字段，`Workspace` 的方法在同一个模块
（`workspace.rs`）内可以直接访问——跟 `send_input` 现有代码的访问方式一致，不需要改
`ShellIo` 的可见性。

在 `impl App` 里加：

```rust
    /// 把一条派发记录写进 `todo_meta` 并落盘。`text` 用来算
    /// `todo::todo_line_key`——跟渲染时查询用的 key 必须是同一套算法，
    /// 否则写进去的记录永远查不到。
    fn record_todo_dispatch(&mut self, project_id: ProjectId, text: &str, session_id: String) {
        let key = todo::todo_line_key(text);
        let entry = self.todo_meta.entry(project_id).or_default();
        entry.insert(
            key,
            todo_meta::TodoTaskMeta {
                dispatch: Some(todo_meta::DispatchRecord {
                    session_id,
                    dispatched_at: std::time::SystemTime::now(),
                }),
                ..entry.get(&key).cloned().unwrap_or_default()
            },
        );
        if let Err(e) = todo_meta::save(&self.todo_meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }
```

- [ ] **Step 7: "新建"派发的 session id 延后补记**

搜索 `fn on_tab_attached`（`Message::TabAttached` 的处理逻辑，Task 11 之前读到过它的
函数签名），这个函数目前不知道"这个新 tab 是不是从 Todo 派发新建过来的、对应哪条任务
文本"。给 `Workspace` 加一个字段记住"待认领"的派发意图：

```rust
    /// "派发到新建"发起时记一笔：`tab_id`（`spawn_new_tab` 内部生成、
    /// 后面 `on_tab_attached` 能拿到的那个）→ 任务文本。`on_tab_attached`
    /// 里查到就消费掉、往 `App.todo_meta` 补一条派发记录（这时才第一次
    /// 知道真正的 `session_id`）；查不到说明这个 tab 不是 Todo 派发建的，
    /// 什么都不做。
    todo_pending_dispatch: HashMap<usize, String>,
```

构造点补 `todo_pending_dispatch: HashMap::new(),`。

`spawn_new_tab` 返回 `tab_id`（目前函数签名是 `fn spawn_new_tab(...)`，没有返回值——
搜索函数体最后 `self.pending.insert(tab_id, jh);` 那一行，之后加 `tab_id`，把函数签名
的返回类型从隐式 `()` 改成 `-> usize`）：

```rust
    fn spawn_new_tab(&mut self, io: &ShellIo, launch: PickerLaunch, follow_up: Option<String>) -> usize {
        // ...函数体不变...
        self.pending.insert(tab_id, jh);
        tab_id
    }
```

（`if self.loading { return; }` 那个早退分支现在也要有返回值——改成
`if self.loading { return usize::MAX; }` 并在调用方按"`usize::MAX` = 没真正建成"过滤，
或者更简单：把 `spawn_new_tab` 的返回类型改成 `Option<usize>`，`loading` 分支返回
`None`，正常路径返回 `Some(tab_id)`——选后者，跟这个代码库大量用 `Option` 表达"可能
不存在"的风格更一致）。

调整后：

```rust
    fn spawn_new_tab(
        &mut self,
        io: &ShellIo,
        launch: PickerLaunch,
        follow_up: Option<String>,
    ) -> Option<usize> {
        if self.loading {
            return None;
        }
        // ...中间不变...
        self.pending.insert(tab_id, jh);
        Some(tab_id)
    }
```

`Message::AgentPickerSelect` 调用点（Step 1 已改）不需要再改——忽略返回值即可，
`ws.spawn_new_tab(io, agent, None);` 依然合法（返回值没用上，`Option<usize>` 没有
`#[must_use]`，不会报警告；如果 clippy 报 `unused_must_use` 类警告，在这行前面加
`let _ =`）。

回到 `Message::TodoDispatchNew` 分支（Step 6），改成：

```rust
            Message::TodoDispatchNew(idx, launch) => {
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo_dispatch_open = None;
                    if let Some(tab_id) = ws.spawn_new_tab(io, launch, Some(text.clone())) {
                        ws.todo_pending_dispatch.insert(tab_id, text);
                    }
                });
            }
```

搜索 `fn on_tab_attached`，在函数体（`pending` 条目被取出、`SessionTab` 被真正插入
`self.tabs` 之后的位置——具体插入点看现有实现，找 `self.tabs.push(...)` 或类似语句）
之后加一段"认领待处理派发"的逻辑。因为这个函数是 `Workspace` 的方法、不直接持有
`App.todo_meta`，改成让调用方（`App` 处理 `Message::TabAttached` 的地方）在
`on_tab_attached` 返回后再检查一次：

搜索 `Message::TabAttached(project_id, tab_id, info, snapshot) => {`，在现有处理逻辑
（多半是 `self.dispatch_to_project(project_id, |ws, io| ws.on_tab_attached(io, tab_id,
info, snapshot));` 这样的形状，具体按 `dispatch_to_project`/等价方法的既有代码为准）
之后，补一段：

```rust
            Message::TabAttached(project_id, tab_id, info, snapshot) => {
                let session_id = info.id.clone();
                self.dispatch_to_project(project_id, |ws, io| {
                    ws.on_tab_attached(io, tab_id, info, snapshot);
                });
                if let Some(text) = self
                    .active_workspace_mut()
                    .and_then(|ws| ws.todo_pending_dispatch.remove(&tab_id))
                {
                    self.record_todo_dispatch(project_id, &text, session_id);
                }
            }
```

**这一步的具体写法高度依赖 `Message::TabAttached` 现有的真实处理代码（本计划撰写时
没有把它的完整实现摘出来核对）——落地前先 `grep -n "Message::TabAttached" crates/
dozer-app/src/workspace.rs` 读一遍现有分支的真实结构，把"认领 todo_pending_dispatch"
这段逻辑接在合适的位置，原则不变：`on_tab_attached` 完成之后、拿到真正的
`session_id`（来自 `info.id`）时，查一次 `todo_pending_dispatch`，命中就调
`record_todo_dispatch` 补记派发记录、并把这条从 `todo_pending_dispatch` 里删掉。**

- [ ] **Step 8: `todo_pane`/`todo_row` 接入派发按钮 + 选择层 + 让 `app_todo_dispatch_for`
  查真实数据**

把 Task 12 的占位 `app_todo_dispatch_for` 换成真实实现：

```rust
/// 按 `todo_line_key` 查 `App.todo_meta` 拿这条任务的派发记录（如果
/// 有）。`ws` 参数在这个签名里其实用不上项目 id——项目 id 从 `app` 侧
/// 传，改成直接接收 `app: &App, ws: &Workspace, key: u64`。
fn app_todo_dispatch_for<'a>(
    app: &'a App,
    ws: &Workspace,
    key: u64,
) -> Option<&'a todo_meta::DispatchRecord> {
    let project_id = ws.project.as_ref()?.id;
    app.todo_meta.get(&project_id)?.get(&key)?.dispatch.as_ref()
}
```

`todo_pane` 里调用处相应改成 `app_todo_dispatch_for(app, ws, key)`（`todo_pane` 签名里
已经有 `app: &'a App` 参数，Task 8 起就有，之前几个任务没用上它——现在用上了；如果
中途因为"参数未使用"被 clippy 警告过要求加 `_app`，这一步改回 `app`）。

`todo_row` 加派发按钮，并且改造成"选中态才显示派发按钮/在进行中显示存活标签"：

```rust
/// 单条任务行：勾选框 + 任务文本 + 右侧派发按钮（待办态）或"进行中"
/// 标签（进行中态）——完成态两者都不显示。
fn todo_row<'a>(
    idx: usize,
    item: &'a todo::TodoItem,
    state: todo::TodoState,
    dispatch_open: bool,
    existing_tabs: &'a [(&'a str, String)], // (session_id, 展示用标题)，派发选择层列表用；
                                             // 标题是 `tab_title` 返回的 owned String，见下方
                                             // `todo_pane` 组装处
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // ...勾选框/文本部分沿用 Task 9/10 已有实现...

    let trailing: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = match state {
        todo::TodoState::Pending => button(
            row![
                icons::view(icons::IconKind::SquarePlus, crate::icon_size::row(), theme::GOLD),
                text("派发").size(workspace_font::caption()).color(theme::GOLD),
            ]
            .spacing(4)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::TodoDispatchOpen(idx))
        .padding([5, 10])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::GOLD,
            border: Border { color: theme::BORDER, width: 1.0, radius: 6.0.into() },
            ..button::Style::default()
        })
        .into(),
        todo::TodoState::InProgress => row![
            text("●").size(workspace_font::dot_sm()).color(theme::GREEN),
            text("进行中").size(workspace_font::caption()).color(theme::GREEN),
        ]
        .spacing(5)
        .into(),
        todo::TodoState::Done => iced_widget::space::Space::new().into(),
    };

    let row_content = row![/* checkbox, text(FILL 宽) */, trailing]
        .spacing(10)
        .align_y(iced_widget::core::Alignment::Center)
        .padding([10, 20]);

    let base: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = row_content.into();
    if dispatch_open {
        // 选择层叠在这一行下方，见下面 todo_dispatch_popup。
        column![base, todo_dispatch_popup(idx, existing_tabs)].into()
    } else {
        base
    }
}
```

新增 `todo_dispatch_popup`（样式对齐 `agent_picker_popup`——`theme::CARD` 底、
`theme::BORDER` 描边，列表按钮）：

```rust
/// Todo 派发选择层：列出当前项目存活的 agent tab + 一个"新建"入口，
/// 样式对齐 `agent_picker_popup`（同款 CARD 底 + BORDER 描边）。跟
/// `agent_picker_popup` 挂在窗口右上角固定坐标不同，这个层挂在触发它
/// 的那一行下方（`todo_row` 直接把它塞进同一个 `column!`），不需要
/// 额外的坐标计算。
fn todo_dispatch_popup<'a>(
    idx: usize,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![].spacing(2);
    // 不能写 `for &(session_id, title) in existing_tabs`——`title` 是
    // `String`，不是 `Copy`，模式前面那个 `&` 会尝试把它从引用里移出来，
    // 编译不过。match ergonomics 下 `(session_id, title)` 直接对
    // `&(&str, String)` 解构会自动按引用绑定，`session_id: &&str`、
    // `title: &String`，两者都够用（`.to_string()`/传给 `text()` 都支持
    // 自动解引用）。
    for (session_id, title) in existing_tabs {
        col = col.push(
            button(text(title.clone()).size(workspace_font::body()).color(theme::CREAM))
                .on_press(Message::TodoDispatchToExisting(idx, session_id.to_string()))
                .width(Length::Fill)
                .padding([6, 12])
                .style(|_t: &iced_widget::Theme, _s| button::Style {
                    background: None,
                    text_color: theme::CREAM,
                    ..button::Style::default()
                }),
        );
    }
    col = col.push(
        button(text("新建 agent 会话…").size(workspace_font::body()).color(theme::GOLD))
            .on_press(Message::TodoDispatchNew(idx, PickerLaunch::Agent(None)))
            .width(Length::Fill)
            .padding([6, 12])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::GOLD,
                ..button::Style::default()
            }),
    );
    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border { color: theme::BORDER, width: 1.0, radius: 6.0.into() },
            ..container::Style::default()
        })
        .into()
}
```

`todo_pane` 里组装 `existing_tabs`（当前项目存活 tab 的 `(session_id, 展示标题)` 列表，
标题复用现成的 `tab_title` 函数——`grep -n "fn tab_title" crates/dozer-app/src/
workspace.rs` 确认签名后按其参数传）并把它和 `ws.todo_dispatch_open` 传给每个
`todo_row`：

```rust
    let existing_tabs: Vec<(&str, String)> = ws
        .tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| (t.info.id.as_str(), tab_title(t.agent, t.info.cwd.as_deref(), &t.info.name)))
        .collect();
```

（`tab_title` 的确切参数类型如果跟这里假设的不一致——落地前 `grep` 确认一下，这是
既有函数，本计划不重新定义它，按它真实的签名调整调用方式即可。）

同时把 Task 12 写的 `states` 计算循环里的占位调用换成真实调用，把渲染循环里
`todo_row(idx, &ws.todo_items[idx])`（Task 9/12 的 2 参数旧签名）换成本任务
`todo_row` 新签名对应的调用：

```rust
    let states: Vec<todo::TodoState> = ws
        .todo_items
        .iter()
        .map(|item| {
            let key = todo::todo_line_key(&item.text);
            let dispatch = app_todo_dispatch_for(app, ws, key); // 换成真实实现，不再是 Task 12 的占位
            let target_alive = dispatch
                .map(|d| ws.tabs.iter().any(|t| t.info.id == d.session_id && t.alive))
                .unwrap_or(false);
            todo::todo_display_state(item, dispatch, target_alive)
        })
        .collect();
    // ...visible_idx/toolbar 计算不变（Task 12 已经写好）...
    for &idx in &visible_idx {
        let item = &ws.todo_items[idx];
        list = list.push(todo_row(
            idx,
            item,
            states[idx],
            ws.todo_dispatch_open == Some(idx),
            &existing_tabs,
        ));
    }
```

- [ ] **Step 9: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全部干净、全部 PASS。这一步大概率需要几轮"编译报错 → 按报错信息核对现有
代码真实签名 → 调整"的来回，尤其是 `Message::TabAttached`/`tab_title`/
`dispatch_to_project` 这几处——都是本计划里标注过"依赖现有代码真实结构，落地前重新
确认"的地方。

- [ ] **Step 10: 人工验收**

在 `/tmp/todo-smoke` 项目里开一个 agent tab（比如纯 Shell），回到 Todo 面板，点某条
待办任务的"派发"按钮：确认弹出选择层、列出刚才那个存活 tab；选中它，确认任务文本被
当输入打进那个 tab 的终端里，且这条任务在 Todo 面板里状态变成"进行中"（绿点+文字）；
把那个 tab 关掉，回到 Todo 面板（或等 1 秒轮询），确认这条任务状态回落成"待办"。再
测一次"新建 agent 会话…"选项：确认新开一个 tab、任务文本被打进去、`todo_meta.json`
（`~/Library/Application Support/dozer/`或等价 `config_dir()` 路径下）里能看到对应的
派发记录。

- [ ] **Step 11: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Todo 面板——派发到 agent tab(已有/新建),进行中态接入真实数据"
```

---

### Task 14: 计划时间 / 完成时间

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `todo_meta::TodoTaskMeta`（Task 4）、`Message::TodoToggle`（Task 9，本任务
  扩展其处理逻辑）
- Produces: `Workspace.todo_editing_plan_date: Option<(usize, String)>`（`Some((idx,
  draft))` = 正在给第 `idx` 条编辑计划时间，`draft` 是输入框当前内容）、
  `Message::{TodoPlanDateEditStart(usize), TodoPlanDateChanged(String), TodoPlanDateSubmit}`

- [ ] **Step 1: `Workspace` 加字段**

```rust
    /// 正在内联编辑计划时间的任务下标 + 输入框草稿；`None` = 当前没有
    /// 任何一条在编辑计划时间。
    todo_editing_plan_date: Option<(usize, String)>,
```

构造点补 `todo_editing_plan_date: None,`。

- [ ] **Step 2: `Message` 加三个变体 + `update()` 处理**

```rust
    /// 点击某条任务旁的"计划 X"/空白处，进入内联编辑。
    TodoPlanDateEditStart(usize),
    /// 编辑框内容变化。
    TodoPlanDateChanged(String),
    /// 提交（回车/失焦）。
    TodoPlanDateSubmit,
```

```rust
            Message::TodoPlanDateEditStart(idx) => {
                let existing = self
                    .active_workspace()
                    .and_then(|ws| ws.project.as_ref().map(|p| p.id))
                    .and_then(|pid| {
                        let key = self
                            .active_workspace()
                            .and_then(|ws| ws.todo_items.get(idx))
                            .map(|item| todo::todo_line_key(&item.text))?;
                        self.todo_meta.get(&pid)?.get(&key)?.plan_date.clone()
                    })
                    .unwrap_or_default();
                self.with_focused_project(|ws, _io| {
                    ws.todo_editing_plan_date = Some((idx, existing));
                });
            }
            Message::TodoPlanDateChanged(s) => {
                self.with_focused_project(|ws, _io| {
                    if let Some((_, draft)) = ws.todo_editing_plan_date.as_mut() {
                        *draft = s;
                    }
                });
            }
            Message::TodoPlanDateSubmit => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let entry = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_editing_plan_date.clone())
                    .and_then(|(idx, draft)| {
                        let text = self
                            .active_workspace()
                            .and_then(|ws| ws.todo_items.get(idx))
                            .map(|item| item.text.clone())?;
                        Some((text, draft))
                    });
                if let Some((text, draft)) = entry {
                    self.set_todo_plan_date(project_id, &text, draft);
                }
                self.with_focused_project(|ws, _io| ws.todo_editing_plan_date = None);
            }
```

`impl App` 加：

```rust
    /// 写/清计划时间：`draft` 为空字符串时存 `None`（清空这个字段，
    /// design 第 8 节"不留空白占位"——空草稿等价于用户想清空这条）。
    fn set_todo_plan_date(&mut self, project_id: ProjectId, text: &str, draft: String) {
        let key = todo::todo_line_key(text);
        let entry = self.todo_meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.plan_date = if draft.trim().is_empty() {
            None
        } else {
            Some(draft.trim().to_string())
        };
        if let Err(e) = todo_meta::save(&self.todo_meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }
```

- [ ] **Step 3: 勾选完成时自动盖章完成时间**

回到 Task 9 写的 `toggle_todo_item`（`Workspace` 方法）——它目前只管 `.dozer/todo.md`
本身的读写，不碰 `todo_meta`（`todo_meta` 现在挂在 `App` 上，`Workspace` 方法拿不到）。
把"翻转 `completed_at`"这段逻辑挪到 `App::update` 的 `Message::TodoToggle` 分支里，
在调用 `toggle_todo_item` 前后各查一次 `item.done`：

搜索 `Message::TodoToggle(idx) => {`（Task 9 Step 6 加的），改成：

```rust
            Message::TodoToggle(idx) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let before = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .cloned();
                self.with_focused_project(|ws, _io| ws.toggle_todo_item(idx));
                let after = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .cloned();
                if let (Some(before), Some(after)) = (before, after) {
                    // 文本没变(正常勾选场景)才更新 completed_at；如果文本变了
                    // (理论上不该发生在同一次 toggle 里，但 `reload_todo_from_
                    // disk` 期间文件可能被 agent 并发改过)，跳过——避免把
                    // 完成时间错记到另一条任务上。
                    if before.text == after.text {
                        self.set_todo_completed_at(project_id, &after.text, after.done);
                    }
                }
            }
```

`impl App` 加：

```rust
    /// 勾选变完成 → 盖章当前时间；取消勾选 → 清空（design 第 8 节:
    /// "避免一个待办任务身上挂着一个陈旧的完成于时间戳"）。
    fn set_todo_completed_at(&mut self, project_id: ProjectId, text: &str, done: bool) {
        let key = todo::todo_line_key(text);
        let entry = self.todo_meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.completed_at = done.then(std::time::SystemTime::now);
        if let Err(e) = todo_meta::save(&self.todo_meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }
```

`TodoItem` 需要 `Clone`——Task 1 定义时已经 `#[derive(Debug, Clone, PartialEq)]`，不用改。

- [ ] **Step 4: 纯函数单测——完成时间转换逻辑**

这条转换（`done: bool → completed_at: Option<SystemTime>`）本身逻辑简单到不值得从
`set_todo_completed_at`（带 I/O 副作用）里再拆一个纯函数出来单测——按 design 测试策略
"拆成纯函数单测"这条的精神，在 `todo.rs` 加一个真正纯粹、不碰 `App`/`todo_meta` 的
辅助函数，`set_todo_completed_at` 内部调用它：

在 `todo.rs`（不是 `workspace.rs`）加：

```rust
/// `done` 翻转成 `completed_at` 该有的值：完成 → `Some(now)`，取消
/// 完成 → `None`。抽成纯函数是为了能不起 GUI/不碰文件单测这条转换
/// 规则本身。
pub fn completed_at_for_toggle(done: bool, now: std::time::SystemTime) -> Option<std::time::SystemTime> {
    done.then_some(now)
}

#[cfg(test)]
mod completed_at_tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn done_gets_timestamp_undone_gets_none() {
        let now = SystemTime::now();
        assert_eq!(completed_at_for_toggle(true, now), Some(now));
        assert_eq!(completed_at_for_toggle(false, now), None);
    }
}
```

`workspace.rs` 的 `set_todo_completed_at` 改成调用它：

```rust
        meta.completed_at = todo::completed_at_for_toggle(done, std::time::SystemTime::now());
```

- [ ] **Step 5: `todo_row` 渲染计划时间/完成时间**

在 `todo_row` 里（Task 13 已经改造成按 `state` 分支渲染 `trailing`），文本和 `trailing`
之间插入一段可选的日期文字。函数签名加一个 `meta: Option<&todo_meta::TodoTaskMeta>`
参数：

```rust
fn todo_row<'a>(
    idx: usize,
    item: &'a todo::TodoItem,
    state: todo::TodoState,
    meta: Option<&'a todo_meta::TodoTaskMeta>,
    dispatch_open: bool,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // ...勾选框/文本/trailing 沿用 Task 9/13...

    let date_label: Option<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
        match state {
            todo::TodoState::Done => meta.and_then(|m| m.completed_at).map(|t| {
                text(format!("完成于 {}", format_todo_time(t)))
                    .size(workspace_font::caption())
                    .color(theme::DIM)
                    .into()
            }),
            _ => meta.and_then(|m| m.plan_date.as_deref()).map(|d| {
                button(text(format!("计划 {d}")).size(workspace_font::caption()).color(theme::DIM))
                    .on_press(Message::TodoPlanDateEditStart(idx))
                    .padding(0)
                    .style(|_t: &iced_widget::Theme, _s| button::Style {
                        background: None,
                        text_color: theme::DIM,
                        ..button::Style::default()
                    })
                    .into()
            }),
        };

    let mut middle = row![/* checkbox, text */].spacing(10).align_y(iced_widget::core::Alignment::Center);
    if let Some(label) = date_label {
        middle = middle.push(label);
    }
    let row_content = row![middle, trailing] /* 原有布局微调，date_label 放在 text 和 trailing 之间 */
        .spacing(10)
        .align_y(iced_widget::core::Alignment::Center)
        .padding([10, 20]);
```

`format_todo_time` 辅助函数（`SystemTime` 格式化成"MM-DD HH:MM"——这个代码库有没有
现成的时间格式化辅助，先 `grep -n "fn format.*time\|chrono\|time::" crates/dozer-app/
src/workspace.rs` 看一眼；如果已经有类似 `relative_time_text` 这样的函数可以参考风格，
没有现成依赖就用标准库手写一个足够用的版本）：

```rust
/// `SystemTime` → "MM-DD HH:MM"，本地时区。足够用于"完成于"这种粗粒度
/// 提示，不追求时区/夏令时严格正确性（同 design 里"计划时间不做格式
/// 校验"一个量级的从简）。
fn format_todo_time(t: std::time::SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = t.into();
    datetime.format("%m-%d %H:%M").to_string()
}
```

**如果 `crates/dozer-app/Cargo.toml` 里没有 `chrono` 依赖**（`grep -n "chrono"
crates/dozer-app/Cargo.toml` 确认），这里新增一个依赖需要停下来——跟 Global Constraints
"不引入新依赖"的原则有轻微冲突（那条约束针对的是 `notify`/markdown 解析库这种"为这个
功能专门引入"的依赖，`chrono` 属于通用时间处理，如果 workspace 内其它 crate 已经在用
就不算新增；`grep -rn "chrono" Cargo.lock | head -1` 确认一下 lockfile 里是不是已经
因为别的依赖间接锁定了 chrono）。如果确认全仓库都没有 chrono，退回用标准库
`SystemTime` 配合 `humantime` 或者干脆存 `Instant`/`Duration` 算相对时间文案（比如
"3 分钟前"，参考 `relative_time_text` 如果存在的话）——具体取舍在这一步执行时按实际
情况决定，不在计划里预先锁死格式化方案，只锁死行为约定："完成于"要能让用户看出大致
什么时候完成的，不要求精确到秒。

- [ ] **Step 6: `todo_pane` 组装 `meta` 参数并渲染内联编辑输入框**

`todo_pane` 循环渲染 `todo_row` 时，按 `todo_line_key` 查 `app.todo_meta` 拿 `meta`
传进去；如果 `ws.todo_editing_plan_date == Some((idx, draft))`，这一行额外渲染一个
`text_input` 覆盖/替代 `date_label` 的位置：

```rust
    for &idx in &visible_idx {
        let item = &ws.todo_items[idx];
        let key = todo::todo_line_key(&item.text);
        let project_id = ws.project.as_ref().map(|p| p.id);
        let meta = project_id.and_then(|pid| app.todo_meta.get(&pid)).and_then(|m| m.get(&key));
        if let Some((editing_idx, draft)) = &ws.todo_editing_plan_date {
            if *editing_idx == idx {
                list = list.push(todo_plan_date_edit_row(idx, item, draft));
                continue;
            }
        }
        list = list.push(todo_row(idx, item, states[idx], meta, ws.todo_dispatch_open == Some(idx), &existing_tabs));
    }
```

```rust
/// 计划时间内联编辑态：任务文本 + 一个 `text_input`，回车/失焦提交。
fn todo_plan_date_edit_row<'a>(
    idx: usize,
    item: &'a todo::TodoItem,
    draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    row![
        text(item.text.clone()).size(workspace_font::body()).color(theme::CREAM),
        text_input("计划时间，如 08-10", draft)
            .on_input(Message::TodoPlanDateChanged)
            .on_submit(Message::TodoPlanDateSubmit)
            .size(workspace_font::caption())
            .width(Length::Fixed(140.0)),
    ]
    .spacing(10)
    .align_y(iced_widget::core::Alignment::Center)
    .padding([10, 20])
    .into()
}
```

失焦提交（点击别处关闭编辑框）如果这个版本的 iced 没有现成的"失焦"事件可用，先只做
"回车提交"，人工验收时确认这条可接受——不是本任务的阻断项（design 也只写了"内联编辑
框模式"，没有强定"必须支持失焦提交"）。

- [ ] **Step 7: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 全部干净、全部 PASS（累计新增纯函数测试到这一步大约 18-19 个）。

- [ ] **Step 8: 人工验收**

`/tmp/todo-smoke` 项目里：点一条待办任务旁边的空白/"计划"文字，出现输入框，输入
"08-20"回车，确认那一行显示"计划 08-20"；再次点击能改成别的值；清空后回车，确认这段
文字消失（不留空白占位）。勾选一条任务完成，确认它显示"完成于 <当前时间>"；取消勾选，
确认这段文字消失。重启 `dozer` 确认计划时间/完成时间在 `todo_meta.json` 里持久化、
重开后还在。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/todo.rs crates/dozer-app/src/workspace.rs crates/dozer-app/Cargo.toml
git commit -m "feat(dozer-app): Todo 面板——计划时间内联编辑 + 完成时间自动盖章"
```

---

## Self-Review Notes（写完计划后的自查，供执行者参考，不是待办事项）

- **Spec 覆盖**：spec 目标 1-7 逐条对应 Task 1-2(目标1)、Task 8(目标2)、Task 13(目标3)、
  Task 5+13(目标4)、Task 11(目标5)、Task 12(目标6)、Task 14(目标7)。design 里
  "GUI 写入路径"(第6节)对应 Task 9/10 的 `toggle_todo_item`/`submit_todo_add`。
- **两处相对 spec 的技术修正**（已在对应任务里注明原因，不是遗漏）：
  `DispatchRecord.tab_id: usize` → `session_id: String`（Task 4）；
  `PickerLaunch` 承载自定义命令 → `spawn_new_tab` 独立 `follow_up` 参数（Task 13）。
- **已知不确定性**（标注在 Task 13/14 里，不是缺陷）：`Message::TabAttached`/
  `tab_title`/`format_todo_time` 的具体接线方式依赖当时的真实代码结构，且
  `workspace.rs` 正被并行工作持续修改——这几步执行时都要求先重新读一遍现有代码再
  动手，而不是死抄计划里的示例代码。
