# bytehost H6:`on_activate`——面板切入钩子取代 `fire_panel_switch_in` 里的面板逻辑 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地设计文档 H6(`docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md` §3.3、§4):用户把某个面板切到前台时要做的事("刷新任务列表""重新统计用量""标记群列表过期"…),现在写死在 host 的 `fire_panel_switch_in` 的 `match` 里。本刀把其中 6 个面板的**逻辑**搬进各面板自己的 `on_activate` 钩子(Todo、Project 的记忆刷新、Usage、CodeHealth、Conversations、GroupChat),host 的每个分支只剩"造上下文 + 调钩子"。同时给这些此前**完全没有测试**的切入行为补上首批单测。**不改任何用户可见行为。**

**Architecture:** 单个代码任务 + 一个文档任务。Task 1 测试先行(10 个测试,先看编译失败),再跑实现脚本:`panel_host.rs` 加 `ActivationCtx<M>`、`PanelIo` 的 `client()/handle()/emitter()` 访问器和测试夹具 `testing::offline_ctx`;6 个面板各加 `on_activate`;host 6 个分支改调钩子;删掉 `Workspace::spawn_usage_refresh`/`spawn_codehealth_load` 两个不再有调用者的包装;5 个变异检验证明测试会失败。Task 2 回填文档(本刀不动门禁基线:host 的 `PanelKind` 分支数没变)。

**Tech Stack:** Rust(`dozer-app`,iced 0.14,tokio);Python 3 标准库(一次性脚本)。不碰 `Cargo.toml`/`Cargo.lock`。

**Spec:** 设计文档 §3.3(切入钩子)、§4 H6;依据 H4 的 `PanelIo`。

## 对设计文档的细化(请评审)

1. **钩子签名是 `on_activate(&ActivationCtx<M>)`,有状态要改的面板再多一个 `&mut` 状态参数,而不是"返回 `Vec<Effect>` 的纯函数"。** 理由同 H5:面板切入时做的事就是"往 dozerd 发请求、结果经 `emit` 回投成面板消息"——这正是 `PanelIo` 已经封装好的执行原语;让钩子返回 Effect 再由 host 解释,只是多造一层词汇。可测试性不受影响:离线夹具(`Client` 指向不存在的 socket + 记录型 `emit`)能直接断言"发了哪些请求、回了哪些消息"。
2. **`fire_panel_switch_in` 仍是一个按 `PanelKind` 的 `match`。** 它变成遍历 registry 是 H7(注册制)的事,本刀只搬**逻辑**、不动**分发**。

## Global Constraints

- **行为保持。** 6 个分支的用户可见行为逐字不变,包括几处不对称:Usage **先置 loading 再看有没有项目**;CodeHealth **只读缓存、绝不自动扫描**;Conversations/CodeHealth/Usage 要求工作区已有项目,Todo/Project 只要有活动项目 id(它们不需要项目路径)。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h6/...`),分支 `bytehost-h6`,从当前 `main` 开。主 checkout 常有并发会话与未提交改动(此刻 `crates/dozerd/src/transcripts/mod.rs` 就是别人的),不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]` 会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**;`.cargo/config.toml` 不提交;本计划不改依赖。
- **变异检验前先 `git add` 暂存当前版本**,变异后用 `git checkout -- <文件>` 还原到暂存版本。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了。**
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`**(沿用 H1 的防线)。
- **heredoc 一律用带引号的分隔符(`<<'EOF'`)写脚本,绝不用不带引号的**:shell 会展开反引号和 `$`,把脚本和文档改坏。需要传值给脚本时用环境变量(`H6=… python3 script.py`),不要把 shell 变量拼进 heredoc。
- **clippy 诊断必须与 `main` 逐文件一致。**
- 提交信息用 `refactor:`/`test:`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败;`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在 linked worktree 里可能失败;`assets::tests::serves_vendored_asset_with_mime` 偶发失败。
- 草稿基线(`main` = H5 之后,linked worktree):`1756 passed; 1 failed`。草稿里 Task 1 之后 `1766 passed; 1 failed`(+10 个新测试),失败只有 `delete_confirm`。门禁(`check_panel_boundary.py`)输出 `ok (122 refs …)`,前后不变。

## Review Focus

1. **Usage 的"先置 loading"顺序:** 旧代码 `ws.usage.set_loading(true)` 在 `spawn_usage_refresh` 之前,且 `spawn_usage_refresh` 在工作区没有项目时直接返回——所以"没有项目也会置 loading"。新钩子保持这一点(测试 `on_activate_without_a_project_still_marks_loading_but_requests_nothing` 固定),虽然这看起来像个小缺陷(没有项目时界面会一直"统计中"),**本刀不修**,修了就是行为变更。
2. **Todo/Project 用 `active_project_id`、`project_path: None`:** 它们旧代码只依赖 `self.active_project_id`,不看工作区;新代码照旧,`ActivationCtx.project_path` 在这两处是 `None`,它们的钩子也确实不读它。Usage/CodeHealth/Conversations 经 `workspace_activation_ctx` 取 `ws.project`,没有项目时返回 `None`(Usage 仍要置 loading,CodeHealth/Conversations 什么都不做)——与旧的 `spawn_*` 包装里 `let Some(project) = &self.project else { return }` 一致。
3. **CodeHealth 切入不扫描:** 这条行为此前只写在注释里;测试 `on_activate_only_loads_the_cached_report_and_never_starts_a_scan` 现在固定"缓存加载之后 300ms 内不再有消息"。
4. **host 侧 6 个分支没有测试**(项目没有 `App` 夹具,已知缺口):Step 6 逐条对照 diff——每个分支的"造上下文"必须与被删/被改的旧代码的 project_id/路径来源一致。
5. **没有迁的(本刀明确不碰):** `GitLog`(调的是 App 级 `sync_git_log_to_active_project`,状态在 `App.git_log` 而不在面板)、`Project` 的 `ensure_project_readme_and_reveal`(host 的预览动作,面板不该有)、`Database`/`Ssh`(只是一行 `reload_from_disk()`,包一层钩子没有收益)。它们留在 `fire_panel_switch_in` 的 `match` 里,进 E3 清单说明。
6. **`workspace/state.rs` 删了两个包装**(`spawn_usage_refresh`、`spawn_codehealth_load`):编译能过就说明没有别的调用者;`usage/view.rs` 里一处引用它们的注释已订正。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/panel_host.rs` | `PanelIo::{client,handle,emitter}`、`ActivationCtx<M>`、测试夹具 `testing`;1 个测试 | 1 |
| `crates/dozer-app/src/extensions/{todo,project,usage,codehealth,conversations,group_chat}/…` | 各加 `on_activate`;共 9 个测试 | 1 |
| `crates/dozer-app/src/app/update.rs` | `workspace_activation_ctx` 辅助函数;`fire_panel_switch_in` 6 个分支改调钩子 | 1 |
| `crates/dozer-app/src/workspace/state.rs` | 删 `spawn_usage_refresh`/`spawn_codehealth_load` | 1 |
| `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 | 2 |

一次性脚本(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: `ActivationCtx` 与 6 个 `on_activate`

**Files:** 见上表(`crates/` 下 13 个文件)。

**Interfaces:**
- Produces(Task 2 与 H7 依赖):`crate::panel_host::ActivationCtx<M> { project_id: i64, project_path: Option<PathBuf>, io: PanelIo<M> }`;`PanelIo::{client() -> &Client, handle() -> &Handle, emitter() -> impl Fn(M) + Clone + Send + 'static}`;`#[cfg(test)] panel_host::testing::{TIMEOUT, runtime(), offline_ctx::<M>(&Runtime, i64, Option<PathBuf>) -> (ActivationCtx<M>, Receiver<M>)}`;`todo::on_activate(&ActivationCtx<todo::Message>)`、`project::on_activate(&ActivationCtx<project::Message>)`、`usage::on_activate(&mut WorkspaceState, Option<&ActivationCtx<usage::Message>>)`、`codehealth::on_activate(&ActivationCtx<codehealth::Message>)`、`conversations::on_activate(&ActivationCtx<conversations::Message>)`、`group_chat::on_activate(&mut WorkspaceState)`。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h6 -b bytehost-h6 main
mkdir -p ../dozer-bytehost-h6/.cargo && cp .cargo/config.toml ../dozer-bytehost-h6/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h6-scratch && mkdir -p $SCRATCH/h6
```

Expected: 干净、分支 `bytehost-h6`。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h6/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里列的;记下通过数 N(草稿:1756)。再记 clippy 基线:`RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git checkout -- Cargo.lock`(草稿:11 个文件有既有诊断)。

- [ ] **Step 3: 写失败的测试**

把下面 7 段原样写成 `$SCRATCH/h6/` 下的 7 个文件(用带引号的 heredoc 或编辑器;每段开头有一个空行,保留):

`$SCRATCH/h6/tests_panel_host.rs`(追加进 `crates/dozer-app/src/panel_host.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:PanelIo 访问器 ----

    #[test]
    fn emitter_is_a_clone_that_shares_the_sink_and_handle_is_the_hosts_runtime() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        let emit = io.emitter();
        let again = emit.clone();
        emit(1);
        again(2);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![1, 2]);
        // handle() 是 host 的 runtime:能在上面 spawn
        let (tx, done) = std::sync::mpsc::channel();
        io.handle().spawn(async move {
            let _ = tx.send(42u32);
        });
        assert_eq!(
            done.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            42
        );
    }
```

`$SCRATCH/h6/tests_todo.rs`(追加进 `crates/dozer-app/src/extensions/todo/mod.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_requests_both_the_todo_list_and_the_category_tree() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&ctx);
        let first = rx.recv_timeout(TIMEOUT).unwrap();
        let second = rx.recv_timeout(TIMEOUT).unwrap();
        let loaded = |m: &Message| matches!(m, Message::Loaded(_));
        let categories = |m: &Message| matches!(m, Message::CategoriesLoaded(_));
        assert!(
            (loaded(&first) && categories(&second)) || (categories(&first) && loaded(&second)),
            "两个请求各回一条消息"
        );
    }
```

`$SCRATCH/h6/tests_project.rs`(追加进 `crates/dozer-app/src/extensions/project.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_requests_the_shared_memories() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::MemoriesLoaded(_)
        ));
    }
```

`$SCRATCH/h6/tests_usage.rs`(追加进 `crates/dozer-app/src/extensions/usage/mod.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_marks_loading_and_requests_a_refresh_for_that_project() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        let mut state = WorkspaceState::default();
        on_activate(&mut state, Some(&ctx));
        assert!(state.loading());
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::Loaded(7, ..)
        ));
    }

    #[test]
    fn on_activate_without_a_project_still_marks_loading_but_requests_nothing() {
        let mut state = WorkspaceState::default();
        on_activate(&mut state, None);
        assert!(state.loading(), "迁移前的行为:先置 loading,再看有没有项目");
    }
```

`$SCRATCH/h6/tests_codehealth.rs`(追加进 `crates/dozer-app/src/extensions/codehealth/mod.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_only_loads_the_cached_report_and_never_starts_a_scan() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        on_activate(&ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::Loaded(7, _)
        ));
        // 切入不自动扫描(spec:手动触发):缓存加载之后不应再有任何消息
        assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err());
    }

    #[test]
    fn on_activate_without_a_project_path_does_nothing() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&ctx);
        assert!(rx.recv_timeout(std::time::Duration::from_millis(200)).is_err());
    }
```

`$SCRATCH/h6/tests_conversations.rs`(追加进 `crates/dozer-app/src/extensions/conversations.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_refreshes_the_session_list_of_that_project() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        on_activate(&ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::SessionsRefreshed(7, Err(_))
        ));
    }

    #[test]
    fn on_activate_without_a_project_path_does_nothing() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&ctx);
        assert!(rx.recv_timeout(std::time::Duration::from_millis(200)).is_err());
    }
```

`$SCRATCH/h6/tests_group_chat.rs`(追加进 `crates/dozer-app/src/extensions/group_chat/mod.rs` 的 `mod tests { … }` 末尾):

```rust

    // ---- H6:切入钩子 ----

    #[test]
    fn on_activate_marks_the_group_list_stale_without_clearing_it() {
        let mut s = loaded(vec![group(5, "A")]);
        assert!(!s.load_pending(), "刚加载完:列表是新的");
        on_activate(&mut s);
        assert!(s.load_pending(), "切入后下次轮询要重新加载群列表");
        assert_eq!(s.groups().len(), 1, "不清已有内容,免得切入瞬间闪成没有群聊");
        assert!(s.loaded(), "ever_loaded 不回退,前端不闪回加载中");
    }
```

再写追加脚本 `$SCRATCH/h6_tests.py`(带引号 heredoc),运行并确认编译失败:

```python
#!/usr/bin/env python3
"""H6 一次性脚本:把 7 段测试追加到各文件 `mod tests` 的末尾(每个文件的 `mod tests` 都是文件最后一项)。
`SP` 是存放 tests_*.rs 的目录。**不提交。**"""
import os

SP = os.environ["SP"]
SRC = "crates/dozer-app/src/"
PAIRS = [
    ("panel_host.rs", "tests_panel_host.rs"),
    ("extensions/todo/mod.rs", "tests_todo.rs"),
    ("extensions/project.rs", "tests_project.rs"),
    ("extensions/usage/mod.rs", "tests_usage.rs"),
    ("extensions/codehealth/mod.rs", "tests_codehealth.rs"),
    ("extensions/conversations.rs", "tests_conversations.rs"),
    ("extensions/group_chat/mod.rs", "tests_group_chat.rs"),
]
for path, tests in PAIRS:
    s = open(SRC + path).read()
    assert s.rstrip("\n").endswith("\n}"), path
    body = open(os.path.join(SP, tests)).read().rstrip("\n")
    s = s.rstrip("\n")[:-1].rstrip("\n") + "\n\n" + body + "\n}\n"
    open(SRC + path, "w").write(s)
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/h6 python3 $SCRATCH/h6_tests.py
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app --no-run 2>&1 | grep -E "^error" -A3 | head -20; git checkout -- Cargo.lock
```
Expected: 脚本输出 `ok`;编译失败,报错包含 `unresolved import crate::panel_host::testing`、`cannot find function on_activate`、`no method named emitter`(RED)。

- [ ] **Step 4: 写实现脚本并运行**

创建 `$SCRATCH/h6_impl.py`(带引号 heredoc):

```python
#!/usr/bin/env python3
"""H6 一次性实现脚本:`ActivationCtx` + 6 个面板的 `on_activate` 切入钩子;host 的 `fire_panel_switch_in`
对应分支改调钩子;删掉不再有调用者的两个 Workspace 包装。**不提交。** 在仓库根运行,每个替换点先 assert。"""
SRC = "crates/dozer-app/src/"


def read(path):
    return open(SRC + path).read()


def write(path, s):
    open(SRC + path, "w").write(s)


def edit(path, pairs):
    s = read(path)
    for a, b in pairs:
        assert a in s, (path, a[:80])
        s = s.replace(a, b, 1)
    write(path, s)


def append_after_fn(path, fn_start, text):
    """在以 `fn_start` 开头的顶层函数(以 `\\n}\\n` 结尾)之后插入 `text`。"""
    s = read(path)
    i = s.index(fn_start)
    j = s.index("\n}\n", i) + 3
    write(path, s[:j] + "\n" + text + s[j:])


# 1) panel_host.rs:PanelIo 访问器、ActivationCtx、测试辅助
edit("panel_host.rs", [
("""    /// 同步投回一条面板消息。
    pub(crate) fn emit(&self, message: M) {
        (self.emit)(message);
    }
""", """    /// 同步投回一条面板消息。
    pub(crate) fn emit(&self, message: M) {
        (self.emit)(message);
    }

    /// host 的 dozerd 连接(给仍然吃 `(client, handle, emit)` 三件套的面板 `spawn_*` 函数用)。
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// host 的 tokio runtime 句柄(给仍然吃 `(client, handle, emit)` 三件套的面板 `spawn_*` 函数用)。
    pub(crate) fn handle(&self) -> &Handle {
        &self.handle
    }

    /// 一个可 `clone`、可跨线程的 `emit` 闭包(同上,给 `spawn_*(.., emit: impl Fn(M) + Send + 'static)`)。
    pub(crate) fn emitter(&self) -> impl Fn(M) + Clone + Send + 'static {
        let emit = Arc::clone(&self.emit);
        move |message| emit(message)
    }
"""),
("/// 面板之间\"我想让另一个面板做点事\"的**受限词汇**", """/// 面板"切入"(被用户切到前台)时 host 交给它的上下文:当前项目 id、项目路径(项目还没有路径时为
/// `None`)和执行原语。面板的 `on_activate` 钩子只依赖它,不碰 `App`/`Workspace`。
pub(crate) struct ActivationCtx<M> {
    pub(crate) project_id: i64,
    pub(crate) project_path: Option<PathBuf>,
    pub(crate) io: PanelIo<M>,
}

/// 面板之间"我想让另一个面板做点事"的**受限词汇**"""),
("#[cfg(test)]\nmod tests {", """/// 给面板钩子单测用的离线夹具:`Client` 指向不存在的 socket,`emit` 收进 channel。
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

    pub(crate) fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    pub(crate) fn offline_ctx<M: Send + 'static>(
        rt: &tokio::runtime::Runtime,
        project_id: i64,
        project_path: Option<PathBuf>,
    ) -> (ActivationCtx<M>, mpsc::Receiver<M>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = Client::new(PathBuf::from("/tmp/dozer-activation-test-nonexistent.sock"));
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        (
            ActivationCtx {
                project_id,
                project_path,
                io,
            },
            rx,
        )
    }
}

#[cfg(test)]
mod tests {"""),
])

# 2) 六个面板的 on_activate
append_after_fn("extensions/todo/state.rs", "pub fn request_categories_refresh(", '''/// 面板切入:刷新任务列表与分类树(两个请求各回一条消息)。
pub fn on_activate(ctx: &crate::panel_host::ActivationCtx<Message>) {
    let emit = ctx.io.emitter();
    request_todos_refresh(ctx.project_id, ctx.io.client(), ctx.io.handle(), emit.clone());
    request_categories_refresh(ctx.project_id, ctx.io.client(), ctx.io.handle(), emit);
}
''')
append_after_fn("extensions/project/update.rs", "pub fn request_memories_refresh(", '''/// 面板切入:刷新共享记忆列表(README 的展开/定位是 host 的预览动作,不在这里)。
pub fn on_activate(ctx: &crate::panel_host::ActivationCtx<Message>) {
    request_memories_refresh(ctx.project_id, ctx.io.client(), ctx.io.handle(), ctx.io.emitter());
}
''')
append_after_fn("extensions/usage/mod.rs", "pub fn spawn_refresh(", '''/// 面板切入:先置"统计中",有项目就发起一次用量统计(没有项目时只置 loading——迁移前的行为)。
pub fn on_activate(
    state: &mut WorkspaceState,
    ctx: Option<&crate::panel_host::ActivationCtx<Message>>,
) {
    state.set_loading(true);
    let Some(ctx) = ctx else {
        return;
    };
    let Some(path) = ctx.project_path.clone() else {
        return;
    };
    spawn_refresh(
        ctx.project_id,
        path,
        ctx.io.client(),
        ctx.io.handle(),
        ctx.io.emitter(),
    );
}
''')
append_after_fn("extensions/codehealth/aggregate.rs", "pub fn spawn_load_cached(", '''/// 面板切入:只读上次落盘的扫描结果,**不**自动扫描(spec:手动触发,与 Usage 的"打开即自动扫"
/// 是明确的行为差异)。没有项目路径时什么都不做。
pub fn on_activate(ctx: &crate::panel_host::ActivationCtx<Message>) {
    let Some(path) = ctx.project_path.clone() else {
        return;
    };
    spawn_load_cached(
        ctx.project_id,
        path,
        ctx.io.client(),
        ctx.io.handle(),
        ctx.io.emitter(),
    );
}
''')
append_after_fn("extensions/conversations.rs", "pub fn spawn_refresh(", '''/// 面板切入:刷新会话列表(离开一段时间再切回来不会看到陈旧快照,同 Usage 的既有口径)。
/// 没有项目路径时什么都不做。
pub fn on_activate(ctx: &crate::panel_host::ActivationCtx<Message>) {
    let Some(cwd) = ctx.project_path.clone() else {
        return;
    };
    spawn_refresh(
        ctx.project_id,
        cwd,
        ctx.io.client(),
        ctx.io.handle(),
        ctx.io.emitter(),
    );
}
''')
append_after_fn("extensions/group_chat/mod.rs", "pub fn poll_if_due(", '''/// 面板切入:把群列表标成过期(下次轮询重新加载)。`mark_stale` 不清已有内容,切入瞬间不会闪成
/// "没有群聊";实际加载由 host 的轮询(`poll_if_due`)发起。
pub fn on_activate(state: &mut WorkspaceState) {
    state.mark_stale();
}
''')

# 3) host:fire_panel_switch_in 的 6 个分支
edit("app/update.rs", [
("""            PanelKind::Todo => {
                if let Some(project_id) = self.active_project_id {
                    let client = self.client.clone();
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    let emit = move |m: todo::Message| {
                        let _ = proxy.send_event(Message::Todo(m));
                    };
                    let emit_todos = emit.clone();
                    todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
                    todo::request_categories_refresh(project_id, &client, &handle, emit);
                }
            }""", """            PanelKind::Todo => {
                if let Some(project_id) = self.active_project_id {
                    todo::on_activate(&ActivationCtx {
                        project_id,
                        project_path: None,
                        io: self.panel_io(Message::Todo),
                    });
                }
            }"""),
("""                self.ensure_project_readme_and_reveal();
                if let Some(project_id) = self.active_project_id {
                    let client = self.client.clone();
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    let emit = move |m: project::Message| {
                        let _ = proxy.send_event(Message::Project(m));
                    };
                    project::request_memories_refresh(project_id, &client, &handle, emit);
                }""", """                self.ensure_project_readme_and_reveal();
                if let Some(project_id) = self.active_project_id {
                    project::on_activate(&ActivationCtx {
                        project_id,
                        project_path: None,
                        io: self.panel_io(Message::Project),
                    });
                }"""),
("""            PanelKind::Usage => self.with_focused_project(|ws, io| {
                ws.usage.set_loading(true);
                ws.spawn_usage_refresh(io);
            }),""", """            PanelKind::Usage => self.with_focused_project(|ws, io| {
                let ctx = workspace_activation_ctx(ws, io, Message::Usage);
                usage::on_activate(&mut ws.usage, ctx.as_ref());
            }),"""),
("""            PanelKind::CodeHealth => self.with_focused_project(|ws, io| {
                ws.spawn_codehealth_load(io);
            }),""", """            PanelKind::CodeHealth => self.with_focused_project(|ws, io| {
                if let Some(ctx) = workspace_activation_ctx(ws, io, Message::CodeHealth) {
                    codehealth::on_activate(&ctx);
                }
            }),"""),
("""            PanelKind::Conversations => self.with_focused_project(|ws, io| {
                ws.spawn_conversations_refresh(io);
            }),""", """            PanelKind::Conversations => self.with_focused_project(|ws, io| {
                if let Some(ctx) = workspace_activation_ctx(ws, io, Message::Conversations) {
                    conversations::on_activate(&ctx);
                }
            }),"""),
("""            PanelKind::GroupChat => self.with_focused_project(|ws, _io| {
                ws.group_chat.mark_stale();
            }),""", """            PanelKind::GroupChat => self.with_focused_project(|ws, _io| {
                crate::extensions::group_chat::on_activate(&mut ws.group_chat);
            }),"""),
])
p = SRC + "app/update.rs"
s = open(p).read()
a = "impl App {\n    pub fn update(&mut self, message: Message) {"
assert a in s
s = s.replace(a, '''/// 由工作区的项目信息造切入上下文;工作区还没有项目时返回 `None`。
fn workspace_activation_ctx<M: Send + 'static>(
    ws: &Workspace,
    io: &crate::workspace::ShellIo,
    wrap: fn(M) -> Message,
) -> Option<ActivationCtx<M>> {
    let project = ws.project.as_ref()?;
    Some(ActivationCtx {
        project_id: project.id,
        project_path: Some(PathBuf::from(&project.path)),
        io: io.panel_io(wrap),
    })
}

''' + a, 1)
first_use = s.index("\nuse ")
s = s[:first_use] + "\nuse crate::panel_host::ActivationCtx;" + s[first_use:]
open(p, "w").write(s)

# 4) 删不再有调用者的两个 Workspace 包装 + 订正注释
p = SRC + "workspace/state.rs"
s = open(p).read()
for name in ("spawn_usage_refresh", "spawn_codehealth_load"):
    i = s.index(f"    pub(crate) fn {name}(&self, io: &ShellIo) {{")
    # 向上吞掉紧邻的 /// 文档注释
    k = s.rfind("\n\n", 0, i) + 2
    j = s.index("\n    }\n", i) + len("\n    }\n")
    s = s[:k] + s[j:].lstrip("\n")
open(p, "w").write(s)
edit("extensions/usage/view.rs", [("`Workspace::spawn_usage_refresh` 自动触发(见 `panel_select`)", "`usage::on_activate` 自动触发(见 `fire_panel_switch_in`)")])
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && export PYTHONDONTWRITEBYTECODE=1
python3 $SCRATCH/h6_impl.py && cargo fmt -p dozer-app && git status --short
```
Expected: `git status --short` 是 13 个文件:`app/update.rs`、`extensions/{codehealth/aggregate,codehealth/mod,conversations,group_chat/mod,project,project/update,todo/mod,todo/state,usage/mod,usage/view}.rs`、`panel_host.rs`、`workspace/state.rs`(和 `Cargo.lock`,马上还原)。脚本里每个替换点都先 `assert` 原文存在;若报 `AssertionError`,说明 `main` 上这几处已被改动,按报错点对照 `git diff main` 手工对齐后再继续,**不要**改断言。

- [ ] **Step 5: 编译、全量测试、clippy、门禁**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A9 | head -40
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
python3 scripts/audit/check_panel_boundary.py
```
Expected: build 无输出;测试通过数 = N + 10,失败只有已知项;`clippy: no new diagnostics`;门禁 `ok (122 refs …)`。

- [ ] **Step 6: 逐条审阅 host 侧 diff(没有 `App` 夹具可测)**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && git diff -- crates/dozer-app/src/app/update.rs crates/dozer-app/src/workspace/state.rs`
Expected(逐条对照,对应 Review Focus 1、2、4、6):
1. 新函数 `workspace_activation_ctx`:`ws.project?` → `project_id = project.id`、`project_path = Some(PathBuf::from(&project.path))`、`io = io.panel_io(wrap)`;没有项目返回 `None`;
2. Todo/Project 分支:仍以 `self.active_project_id` 为条件,`project_path: None`,`io: self.panel_io(Message::Todo/Project)`;Project 分支开头的 `ensure_project_readme_and_reveal()` **保留在 host**;
3. Usage 分支:`usage::on_activate(&mut ws.usage, ctx.as_ref())` ——无项目时 `ctx` 为 `None`,钩子仍置 loading;
4. CodeHealth/Conversations 分支:无项目则什么都不做;GroupChat 分支只调 `mark_stale` 钩子;
5. `state.rs` 里被删的恰好是 `spawn_usage_refresh`、`spawn_codehealth_load` 两个函数(连同其文档注释),没有别的改动;`GitLog`/`Database`/`Ssh` 分支**没有改**。

- [ ] **Step 7: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && export PYTHONDONTWRITEBYTECODE=1
git add crates
cat > $SCRATCH/h6_mutate.py <<'EOF'
import sys
w = sys.argv[1]
S = "crates/dozer-app/src/extensions/"
if w == "1":   # Usage 切入不再先置 loading
    p, a, b = S + "usage/mod.rs", "    state.set_loading(true);\n    let Some(ctx) = ctx else {", "    let Some(ctx) = ctx else {"
elif w == "2":   # CodeHealth 切入误触发扫描(再多加载一次缓存)
    p, a = S + "codehealth/aggregate.rs", "    spawn_load_cached(\n        ctx.project_id,\n        path,\n        ctx.io.client(),\n        ctx.io.handle(),\n        ctx.io.emitter(),\n    );\n}\n"
    b = "    for _ in 0..2 {\n        spawn_load_cached(\n            ctx.project_id,\n            path.clone(),\n            ctx.io.client(),\n            ctx.io.handle(),\n            ctx.io.emitter(),\n        );\n    }\n}\n"
elif w == "3":   # Todo 切入漏刷分类树
    p, a = S + "todo/state.rs", "    request_categories_refresh(ctx.project_id, ctx.io.client(), ctx.io.handle(), emit);\n"
    b = "    let _ = emit;\n"
elif w == "4":   # GroupChat 切入什么都不做
    p, a, b = S + "group_chat/mod.rs", "    state.mark_stale();\n}", "    let _ = state;\n}"
elif w == "5":   # Conversations 刷了别的项目
    p, a = S + "conversations.rs", "    spawn_refresh(\n        ctx.project_id,\n        cwd,"
    b = "    spawn_refresh(\n        ctx.project_id + 1,\n        cwd,"
s = open(p).read()
assert a in s, w
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3 4 5; do echo "M$m:"; python3 $SCRATCH/h6_mutate.py $m && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app on_activate 2>&1 | grep -E "^error|FAILED$|test result" | head -4; git checkout -- crates Cargo.lock; done
git diff --stat | wc -l; git status --short
```
Expected: 五次变异各自让**对应**的测试失败:M1(Usage 不置 loading):`usage::…::on_activate_marks_loading_and_requests_a_refresh_for_that_project` 与 `…without_a_project_still_marks_loading…`;M2(CodeHealth 双加载):`on_activate_only_loads_the_cached_report_and_never_starts_a_scan`;M3(Todo 漏分类树):`on_activate_requests_both_the_todo_list_and_the_category_tree`;M4(GroupChat 空操作):`on_activate_marks_the_group_list_stale_without_clearing_it`;M5(Conversations 项目 id 错):`on_activate_refreshes_the_session_list_of_that_project`;每次还原后 `git diff --stat | wc -l` 为 `0`。

- [ ] **Step 8: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6
git status --short
git add crates
git diff --cached --stat | tail -15
git commit -m "refactor(panels): on_activate hooks — panels own what happens when they are switched in

Todo, Project (memories), Usage, CodeHealth, Conversations and GroupChat now carry their own
switch-in logic behind an ActivationCtx; fire_panel_switch_in only builds the context and calls
the hook. Adds the first tests for this previously untested behaviour (incl. CodeHealth never
auto-scans). Removes the two Workspace wrappers that lost their last caller.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `git diff --cached --stat` 里只有 13 个 `crates/` 文件(没有 `Cargo.lock`、`docs/`)。

---

### Task 2: 文档回填

**Files:**
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1 的提交。

- [ ] **Step 1: 回填并校验**

用带引号的 heredoc 创建 `$SCRATCH/h6_docs.py`(提交短 id 经环境变量传入):

```python
import os
H6 = os.environ["H6"]
sp = "docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md"
s = open(sp).read()
a = "### 3.3 切入钩子"
assert a in s
i = s.index(a)
j = s.index("\n## ", i)
s = s[:j] + "\n\n(H6 实现取舍:钩子签名是 `on_activate(ctx: &ActivationCtx<Message>)`,有状态要改的面板再加 `&mut WorkspaceState`;`ActivationCtx { project_id, project_path: Option<PathBuf>, io: PanelIo<M> }`。host 的 `fire_panel_switch_in` 仍是一个按 `PanelKind` 的 `match`——它到 H7 注册制落地才会变成遍历 registry;H6 只把每个分支的**逻辑**搬进面板、让它们可单测。)" + s[j:]
lines = s.split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("| **H6** |"):
        lines[k] = l[:-1].rstrip() + " **部分完成(H6,`bytehost-h6`):`" + H6 + "`**——6 个面板的切入钩子(Todo、Project 记忆刷新、Usage、CodeHealth、Conversations、GroupChat)迁入面板模块并补测;GitLog(host 的 `App.git_log` 状态)、Project 的 README 展开(host 预览动作)、Database/Ssh(只是一行 `reload_from_disk`)留在 host |"
        hit = True
assert hit
open(sp, "w").write("\n".join(lines))
tp = "docs/dozer-v2/bytehost-H0/E3-registry.tsv"
rows = open(tp).read().split("\n")
hit = False
for k, r in enumerate(rows):
    if r.startswith("E3-001\t"):
        f = r.split("\t")
        f[4] = "H6 已把 Todo/Project 记忆/Usage/CodeHealth/Conversations/GroupChat 六个分支的切入动作迁进各面板的 on_activate;fire_panel_switch_in 仍是按 PanelKind 的 match(GitLog、Project 的 README 展开、Database、Ssh 四处仍在 host)"
        f[5] = "注册制(H7)后改成遍历 registry 调用面板钩子;GitLog 的 App 级状态与 README 展开随之评估"
        rows[k] = "\t".join(f)
        hit = True
assert hit
open(tp, "w").write("\n".join(rows))
rp = "docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md"
s = open(rp).read().rstrip("\n")
s += "\n\n11. **bytehost H6 已完成（2026-10-04）：** 引入 `ActivationCtx` 与 `PanelIo` 的 `client()`/`handle()`/`emitter()`；Todo、Project 记忆刷新、Usage、CodeHealth、Conversations、GroupChat 六个面板的“切入时动作”迁进各自的 `on_activate` 并补上首批单测（含“CodeHealth 切入不自动扫描”这条此前只写在注释里的 spec 差异）；`Workspace` 少了两个无人再调用的包装。`fire_panel_switch_in` 仍是按 `PanelKind` 的 `match`，等 H7 注册制。\n"
open(rp, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6 && export PYTHONDONTWRITEBYTECODE=1
H6=$(git log --format=%h -1 --grep="on_activate hooks") python3 $SCRATCH/h6_docs.py
awk -F'\t' 'NR>1 && (NF!=6 || $6=="")' docs/dozer-v2/bytehost-H0/E3-registry.tsv | wc -l
git diff -- docs | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | cut -c1-170
python3 scripts/audit/check_panel_boundary.py
```
Expected: `E3-registry.tsv` 校验输出 `0`;`git diff` 里只有设计文档 §3.3 末尾补注与 H6 行、E3-001 一行、要求文档第 11 条,**反引号内容完整**(若有被吞掉的痕迹,说明用了不带引号的 heredoc——`git checkout -- docs` 后重做);门禁 `ok`。

- [ ] **Step 2: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h6
git status --short
git add docs
git diff --cached --stat | tail -5
git commit -m "docs(bytehost): backfill H6 (on_activate hooks)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 只有 `docs/` 下 3 个文件。

---

## Self-Review

**1. 覆盖:** 设计文档 §3.3 的"切入钩子"——6/10 个分支迁走(Todo、Project 记忆、Usage、CodeHealth、Conversations、GroupChat);`GitLog`/Project README/`Database`/`Ssh` 的理由见 Review Focus 5。**坦白收益:** 本刀不减少 host 的分支数(分发仍是 `match`,要到 H7),主要收益是 ① 面板切入行为第一次有了单测(此前为零),② `ActivationCtx` 形状在 H7 注册制下可直接复用。

**2. 占位符扫描:** 测试、实现脚本、变异脚本、文档脚本均为草稿里跑通的完整版本(草稿里用同一套脚本从 `main` 重放,diff 与原型逐字一致)。

**3. 一致性:** `ActivationCtx` 字段、`on_activate` 签名、`panel_host::testing::offline_ctx` 在测试、实现脚本、Interfaces、文档里一致。

**4. Review Focus:** 6 条各有归属(1→测试 + Step 6;2→Step 6;3→测试;4→Step 6;5→范围说明;6→Step 5 编译)。
