# bytehost H4:`PanelIo` 与 group_chat 执行器迁入面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地面板钩子设计的第一步(`docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md` §3.1、§4 的 H4):引入 `PanelIo<M>`(host 给面板的执行原语:`Client` + tokio `Handle` + "把结果包成面板 `Message` 投回事件循环"),并把 `group_chat` 的 Effect 执行器、命令派发、轮询判定从 host 搬进 `group_chat` 模块。host 里的 `run_group_chat_effects` 删除,`group_chat_command` 与 `poll_group_chat_if_active` 只剩构造 `PanelIo` 的胶水。**不改任何用户可见行为**,并第一次给这条链路补上单测(用指向不存在 socket 的 `Client`,不需要 dozerd)。

**Architecture:** 两个任务。Task 1 测试先行(`PanelIo` 4 个测试 + `group_chat` 9 个测试,先看它编译失败),再跑一个实现脚本一次性完成:加 `PanelIo`、`group_chat::{spawn_*(.., io), run_effect, select_group, poll_if_due}`、host 的两个 `panel_io` 构造函数与三处调用点改写;4 个变异检验证明测试真的会失败。Task 2 给门禁加两条"host 替面板做事"的棘轮规则(`R-HOST-PANEL-ARMS` 基线 97、`R-HOST-EXECUTORS` 基线 0),回填设计文档与 E3 清单。

**Tech Stack:** Rust(`dozer-app`,iced 0.14,tokio);Python 3 标准库(一次性脚本 + 审计门禁)。不碰 `Cargo.toml`/`Cargo.lock`。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`(§1 已定裁决、§3.1 `PanelIo`、§3.2 Effect 两层、§4 H4)。依据 H0 的 E3-012/E3-014、H1 的 `PanelHost`。

## Global Constraints

- **行为保持。** 迁移前后的每条后台任务(请求哪个 `Client` 方法、成功/失败各发哪条 `Message`、失败文案)逐字不变;spawn 的时机只允许与迁移前同样"在 host 的事件处理里同步发起"。**尤其注意:** 迁移前除 `SelectGroup` 外的命令是**直接 spawn,不要求项目工作区已加载**——不能把它们挪进 `with_project(..)`(那样工作区没加载时命令会被吞掉;草稿里我第一版就这么写了,自查时发现)。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h4/...`),分支 `bytehost-h4`,从当前 `main` 开(H0–H3 与设计文档都在 `main` 上)。主 checkout 常有并发会话与未提交改动(此刻 `crates/dozerd/src/transcripts/mod.rs` 就是别人的未提交改动),不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]` 会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**;`.cargo/config.toml` 不提交;本计划不改依赖。
- **变异检验前先 `git add` 暂存当前版本**,变异后用 `git checkout -- <文件>` 还原到暂存版本(未暂存的已修改文件直接 `git checkout` 会把整个改动冲回原样)。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了。**
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`**(沿用 H1 的防线)。
- **heredoc 一律用带引号的分隔符(`<<'EOF'`)写 Python 脚本**:不带引号时 shell 会展开脚本里的反引号和 `$`(草稿里踩过,把脚本改坏了)。
- `PanelIo` 是具体结构体,不是 trait;**不为 `spawn_blocking` 等暂时没人用的方法预先加接口**(YAGNI,按真实需求增加)。设计文档 §3.1 的草图里有 `spawn(fut)`/`spawn_blocking`,实现取了更贴合现状的形态——任务拿到 `(Client, PanelIo<M>)`,想发几条消息(含零条)由任务自己决定;Task 2 会把设计文档对齐到实现。
- 提交信息用 `refactor:`/`test:`/`chore(audit):`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败;`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在 linked worktree 里可能失败;`assets::tests::serves_vendored_asset_with_mime` 偶发失败。
- 草稿基线(`main` = H3 + 设计文档之后,linked worktree):`1737 passed; 1 failed`。草稿里 Task 1 之后 `1750 passed; 1 failed`(+13 个新测试),失败只有 `delete_confirm`。执行时以 Task 1 Step 2 实测为准。

## Review Focus

1. **host 侧行为差异(无夹具可测,只能审阅 + 注释钉住)。** 项目没有可构造的 `App` 测试夹具(已知缺口),host 三处调用点的语义只能靠 diff 审阅:`Message::GroupChat(msg)` 臂(`update` 的 Effect 只会在工作区存在时产生,仍然只在 `with_project` 里产生,执行时机从"闭包外"变成"闭包内",都是同步发起的 `handle.spawn`,不阻塞)、`SelectGroup`(仍在 `with_project` 里)、其余命令(**不在** `with_project` 里,与迁移前一致)。Task 1 Step 6 逐条核对。
2. **假 `Client` 测试的前提。** `dozer_client::Client::new(path)` 是惰性的——每次请求才 `UnixStream::connect`,路径不存在则快速失败。测试用 `/tmp/dozer-*-nonexistent.sock`;若该路径碰巧存在(极不可能)测试会变得不确定。负向断言("没有消息")用 200ms 窗口,只会假通过、不会假失败;正向断言用 5s 超时。
3. **`PanelIo::emit` 要求 `Send + Sync`。** host 的 `EventLoopProxy<Message>` 满足(草稿里编译通过);若将来换事件循环实现不满足,这里会是编译错误而非运行时问题。
4. **删掉的 host 代码里有没有别的调用者。** `run_group_chat_effects` 与 `group_chat_command` 的 spawn 分支只有本计划里改的三处调用点;`spawn_fetch_messages`/`spawn_load_groups`/`spawn_command` 的签名变了,Step 5 的编译就是调用点清单(草稿里 `app.rs` 的轮询是唯一直接调用者)。
5. **门禁的臂计数是粗指标。** `R-HOST-PANEL-ARMS` 数的是 `app/update.rs` 里 12 空格缩进的 `Message::<面板>(` 臂(含纯转发臂),只看趋势;不要拿它当"专属处理体"的精确计数。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/panel_host.rs` | 新增 `PanelIo<M>` + 4 个测试 | 1 |
| `crates/dozer-app/src/extensions/group_chat/mod.rs` | `spawn_*` 改吃 `&PanelIo<Message>`;新增 `run_effect`/`select_group`/`poll_if_due`;9 个测试 | 1 |
| `crates/dozer-app/src/workspace/state.rs` | `ShellIo::panel_io` | 1 |
| `crates/dozer-app/src/app/app.rs` | `App::panel_io`;`poll_group_chat_if_active` 瘦身 | 1 |
| `crates/dozer-app/src/app/update.rs` | `Message::GroupChat` 臂、`group_chat_command` 改写;删 `run_group_chat_effects` | 1 |
| `scripts/audit/check_panel_boundary.py`、`test_check_panel_boundary.py`、`panel-boundary.baseline.json` | 两条新规则,基线更新 | 2 |
| `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 | 2 |

一次性脚本(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: `PanelIo` 与 group_chat 执行器迁入面板

**Files:**
- Modify: `crates/dozer-app/src/panel_host.rs`、`crates/dozer-app/src/extensions/group_chat/mod.rs`、`crates/dozer-app/src/workspace/state.rs`、`crates/dozer-app/src/app/app.rs`、`crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Produces(Task 2 与后续切片依赖):`crate::panel_host::PanelIo<M>`——`PanelIo::new(client: Client, handle: Handle, emit: impl Fn(M) + Send + Sync + 'static)`、`.emit(&self, M)`、`.spawn(&self, task: impl FnOnce(Client, PanelIo<M>) -> impl Future<Output = ()> + Send + 'static)`,实现 `Clone`(均 `pub(crate)`);`ShellIo::panel_io::<M>(&self, wrap: fn(M) -> Message) -> PanelIo<M>` 与 `App::panel_io`;`group_chat::{spawn_load_groups(project_id, &PanelIo<Message>), spawn_fetch_messages(project_id, group_id, after_rev, &PanelIo<Message>), spawn_command(project_id, Command, &PanelIo<Message>), run_effect(Effect, project_id, &PanelIo<Message>), select_group(&mut WorkspaceState, project_id, group_id, &PanelIo<Message>), poll_if_due(&mut WorkspaceState, project_id, Instant, &PanelIo<Message>)}`。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h4 -b bytehost-h4 main
mkdir -p ../dozer-bytehost-h4/.cargo && cp .cargo/config.toml ../dozer-bytehost-h4/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h4-scratch && mkdir -p $SCRATCH
```

Expected: 干净、分支 `bytehost-h4`。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h4/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里列的;记下通过数 N(草稿:1737)。再记 clippy 基线:`RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git checkout -- Cargo.lock`(草稿:11 个文件有既有诊断)。

- [ ] **Step 3: 写失败的测试**

创建 `$SCRATCH/tests_panel_io.rs`(**要追加进 `panel_host.rs` 的 `mod tests { … }` 末尾**):

```rust

    // ---- H4:PanelIo ----

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    /// 一个把 `emit` 都收进 channel 的 `PanelIo<u32>`(`Client` 指向不存在的 socket,本组测试不用它)。
    fn recording_io(
        rt: &tokio::runtime::Runtime,
    ) -> (PanelIo<u32>, std::sync::mpsc::Receiver<u32>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let client = dozer_client::Client::new(std::path::PathBuf::from(
            "/tmp/dozer-panel-io-test-nonexistent.sock",
        ));
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        (io, rx)
    }

    #[test]
    fn emit_delivers_the_message_synchronously() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.emit(7);
        assert_eq!(rx.try_recv().unwrap(), 7);
    }

    #[test]
    fn clones_share_one_sink() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        let other = io.clone();
        io.emit(1);
        other.emit(2);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn spawn_runs_on_the_runtime_and_the_task_may_emit_any_number_of_times() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.spawn(|_client, io| async move {
            io.emit(10);
            io.emit(11);
        });
        io.spawn(|_client, _io| async move { /* 零条也合法(例如只在失败时才发消息的命令) */ });
        let got = [
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        ];
        assert_eq!(got, [10, 11]);
        assert!(rx.recv_timeout(std::time::Duration::from_millis(200)).is_err());
    }

    #[test]
    fn spawn_hands_the_task_the_hosts_client() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.spawn(|client, io| async move {
            // 指向不存在 socket 的 Client:真的发请求必然失败——证明拿到的是 host 的那个 Client。
            let failed = client.list_groups(1).await.is_err();
            io.emit(u32::from(failed));
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            1
        );
    }
```

创建 `$SCRATCH/tests_group_chat.rs`(**要追加进 `group_chat/mod.rs` 的 `mod tests { … }` 末尾**):

```rust

    // ---- H4:Effect/Command 的执行器随面板走(host 只给 PanelIo) ----

    use crate::panel_host::PanelIo;
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// `Client` 指向不存在的 socket:所有请求都会快速失败,于是每个任务都恰好发出一条"失败"
    /// 形态的消息——不需要 dozerd 就能断言"哪个 Effect/Command 触发了哪个任务、报什么错"。
    fn offline_io(rt: &tokio::runtime::Runtime) -> (PanelIo<Message>, Receiver<Message>) {
        let (tx, rx) = channel();
        let tx = Arc::new(Mutex::new(tx));
        let client = dozer_client::Client::new(std::path::PathBuf::from(
            "/tmp/dozer-group-chat-test-nonexistent.sock",
        ));
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        (io, rx)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn load_groups_against_an_unreachable_daemon_reports_an_error_for_that_project() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        spawn_load_groups(7, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::GroupsLoaded(7, Err(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn fetch_messages_against_an_unreachable_daemon_reports_an_error_poll_for_that_group() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        spawn_fetch_messages(7, 5, 3, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Polled {
                project_id: 7,
                group_id: 5,
                result: Err(_),
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn every_async_command_reports_failure_with_its_own_wording() {
        let cases: Vec<(Command, &str)> = vec![
            (Command::CreateGroup { topic: "t".into() }, "新建群聊失败"),
            (Command::DeleteGroup { group_id: 5 }, "删除群聊失败"),
            (
                Command::AddMember {
                    group_id: 5,
                    agent: dozer_core::protocol::AgentKind::Claude,
                    handle: "h".into(),
                    role_prompt: String::new(),
                },
                "添加成员失败",
            ),
            (
                Command::UpdateMember {
                    member_id: 1,
                    handle: "h".into(),
                    role_prompt: String::new(),
                },
                "修改成员失败",
            ),
            (Command::RemoveMember { member_id: 1 }, "移除成员失败"),
            (
                Command::Post {
                    group_id: 5,
                    text: "hi".into(),
                },
                "发送失败",
            ),
            (
                Command::Cancel {
                    group_id: 5,
                    scope: dozer_core::protocol::GroupCancelScope::Turn,
                },
                "停止失败",
            ),
            (Command::Retry { message_id: 9 }, "重试失败"),
            (
                Command::PushTodo {
                    group_id: 5,
                    message_id: 9,
                    text: "x".into(),
                },
                "转为待办失败",
            ),
        ];
        let rt = runtime();
        for (cmd, wording) in cases {
            let (io, rx) = offline_io(&rt);
            spawn_command(7, cmd.clone(), &io);
            match rx.recv_timeout(TIMEOUT).unwrap() {
                Message::Failed(7, text) => {
                    assert!(text.starts_with(wording), "{cmd:?}: {text}");
                }
                other => panic!("{cmd:?}: unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn select_and_open_todo_commands_do_not_spawn_anything() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        spawn_command(7, Command::SelectGroup { group_id: 5 }, &io);
        spawn_command(7, Command::OpenTodo { todo_id: 1 }, &io);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn run_effect_maps_each_effect_to_its_task() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        run_effect(Effect::FetchMessages { group_id: 5 }, 7, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Polled {
                project_id: 7,
                group_id: 5,
                result: Err(_),
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
        run_effect(Effect::CreateGroup { topic: "t".into() }, 7, &io);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::Failed(7, ref t) if t.starts_with("新建群聊失败")
        ));
        run_effect(Effect::DeleteGroup { group_id: 5 }, 7, &io);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::Failed(7, ref t) if t.starts_with("删除群聊失败")
        ));
    }

    #[test]
    fn select_group_changes_selection_and_fetches_its_messages() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        select_group(&mut s, 7, 6, &io);
        assert_eq!(s.selected(), Some(6));
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Polled {
                project_id: 7,
                group_id: 6,
                ..
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn select_of_the_current_or_an_unknown_group_does_nothing() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        let mut s = loaded(vec![group(5, "A")]);
        select_group(&mut s, 7, 5, &io);
        select_group(&mut s, 7, 999, &io);
        assert_eq!(s.selected(), Some(5));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn poll_if_due_loads_the_group_list_first() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        let mut s = WorkspaceState::default(); // 从未加载:load_due
        poll_if_due(&mut s, 7, Instant::now(), &io);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::GroupsLoaded(7, Err(_))
        ));
        // 在途期间不重复发起
        poll_if_due(&mut s, 7, Instant::now(), &io);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn poll_if_due_polls_only_a_selected_group_with_an_active_turn() {
        let rt = runtime();
        let (io, rx) = offline_io(&rt);
        let mut s = loaded(vec![group(5, "A")]);
        // 没有进行中的发言:不轮询
        poll_if_due(&mut s, 7, Instant::now(), &io);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        // 有一条 Running 的消息:轮询,after_rev 取当前 latest_rev
        update(
            &mut s,
            Message::Polled {
                project_id: 7,
                group_id: 5,
                result: Ok((vec![msg(1, 1, 4, Some(GroupMessageStatus::Running))], 4)),
            },
        );
        poll_if_due(&mut s, 7, Instant::now(), &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Polled {
                project_id: 7,
                group_id: 5,
                result: Err(_),
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
```

把两段追加进去,并确认它们编译失败:

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1
python3 - <<'EOF'
import os
S = os.path.expanduser("~/Projects/CoralProjects/byteboy/dozer-bytehost-h4-scratch")
for path, name in (("crates/dozer-app/src/panel_host.rs", "tests_panel_io.rs"),
                   ("crates/dozer-app/src/extensions/group_chat/mod.rs", "tests_group_chat.rs")):
    s = open(path).read()
    i = s.rindex("\n}\n")  # 文件最后一个 `}` 是 `mod tests` 的结尾
    open(path, "w").write(s[:i] + "\n" + open(os.path.join(S, name)).read().rstrip("\n") + "\n}\n")
EOF
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app group_chat 2>&1 | grep -E "^error" | head -4; git checkout -- Cargo.lock
```
Expected: 编译失败,`unresolved import \`crate::panel_host::PanelIo\``、`cannot find type \`PanelIo\` in this scope`、`this function takes 4 arguments but 2 arguments were supplied`(`spawn_fetch_messages` 还是旧签名)。

- [ ] **Step 4: 实现脚本**

创建 `$SCRATCH/h4_impl.py`:

```python
#!/usr/bin/env python3
"""H4 一次性实现脚本:引入 PanelIo,把 group_chat 的 Effect/Command 执行器与轮询判定搬进面板模块,
host 只剩构造 PanelIo 的胶水。**不提交。** 在仓库根运行。每个替换点先 assert 原文存在。"""
SRC = "crates/dozer-app/src/"


def edit(path, pairs):
    p = SRC + path
    s = open(p).read()
    for a, b in pairs:
        assert a in s, (path, a[:70])
        s = s.replace(a, b, 1)
    open(p, "w").write(s)


# 1) PanelIo(panel_host.rs,放在 HoverSlot 之前)
edit("panel_host.rs", [
("use crate::app::{App, HoverId, PanelKind};\nuse iced_widget::core::Element;\n",
 '''use crate::app::{App, HoverId, PanelKind};
use dozer_client::Client;
use iced_widget::core::Element;
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::Handle;

/// host 给面板的**执行原语**:面板要在后台跑任务,需要三样东西——`Client`(dozerd 连接)、
/// `Handle`(tokio runtime)、"把结果包成该面板的 `Message` 投回事件循环"。收成一个值,由 host
/// 构造、传给面板模块里的执行器(`group_chat::run_effect` 等);host 不再认识面板的具体 Effect。
///
/// 与 [`PanelHost`](只读视图契约)是一对:`PanelHost` 管"读 host 状态来画 view",`PanelIo` 管
/// "让 host 替我跑东西"。是具体结构体而不是 trait(规格 §1:不为未来的第二个宿主预先抽象);
/// 测试里用 `PanelIo::new` 配一个收进 channel 的 `emit` 即可。
pub(crate) struct PanelIo<M> {
    client: Client,
    handle: Handle,
    emit: Arc<dyn Fn(M) + Send + Sync + 'static>,
}

impl<M> Clone for PanelIo<M> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            handle: self.handle.clone(),
            emit: Arc::clone(&self.emit),
        }
    }
}

impl<M: Send + 'static> PanelIo<M> {
    pub(crate) fn new(
        client: Client,
        handle: Handle,
        emit: impl Fn(M) + Send + Sync + 'static,
    ) -> Self {
        Self {
            client,
            handle,
            emit: Arc::new(emit),
        }
    }

    /// 同步投回一条面板消息。
    pub(crate) fn emit(&self, message: M) {
        (self.emit)(message);
    }

    /// 在 host 的 runtime 上跑一个后台任务。任务拿到 host 的 `Client` 和一份 `PanelIo` 副本,
    /// 想发多少条消息(包括零条:只在失败时才发的命令)由任务自己决定。
    pub(crate) fn spawn<F, Fut>(&self, task: F)
    where
        F: FnOnce(Client, PanelIo<M>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let client = self.client.clone();
        let io = self.clone();
        self.handle.spawn(async move { task(client, io).await });
    }
}
'''),
])

# 2) group_chat:spawn_* 改吃 PanelIo,新增 run_effect / select_group / poll_if_due
p = SRC + "extensions/group_chat/mod.rs"
s = open(p).read()
i = s.index("/// 加载某项目的群聊列表。\npub fn spawn_load_groups(")
j = s.index("#[cfg(test)]\nmod tests {")
old = s[i:j]
new = '''/// 加载某项目的群聊列表。
pub fn spawn_load_groups(project_id: i64, io: &PanelIo<Message>) {
    io.spawn(move |client, io| async move {
        let result = client
            .list_groups(project_id)
            .await
            .map_err(|e| e.to_string());
        io.emit(Message::GroupsLoaded(project_id, result));
    });
}

/// 取某群 `rev > after_rev` 的消息(`after_rev = 0` 即全量)。
pub fn spawn_fetch_messages(
    project_id: i64,
    group_id: i64,
    after_rev: i64,
    io: &PanelIo<Message>,
) {
    io.spawn(move |client, io| async move {
        let result = client
            .list_group_messages(group_id, after_rev, POLL_LIMIT)
            .await
            .map_err(|e| e.to_string());
        io.emit(Message::Polled {
            project_id,
            group_id,
            result,
        });
    });
}

/// 执行一条经 `route_event` 校验过的命令。`SelectGroup`/`OpenTodo` 不走这里
/// (前者是同步状态变更,见 [`select_group`];后者是切面板,由 host 处理)。
pub fn spawn_command(project_id: i64, cmd: Command, io: &PanelIo<Message>) {
    io.spawn(move |client, io| async move {
        let fail =
            |what: &str, e: anyhow::Error| Message::Failed(project_id, format!("{what}: {e}"));
        match cmd {
'''
# 复用原 match 体:从 "Command::CreateGroup" 起到函数结尾,把 `emit(` 换成 `io.emit(`
k = old.index("            Command::CreateGroup { topic } =>")
body = old[k:]
body = body.replace("emit(", "io.emit(")
# 函数结尾仍是 `        }\n    });\n}\n\n`
new += body
new += '''
/// 面板 `Effect` 的执行器:host 不认识这些变体,只负责给 `PanelIo`。
pub fn run_effect(effect: Effect, project_id: i64, io: &PanelIo<Message>) {
    match effect {
        Effect::FetchMessages { group_id } => spawn_fetch_messages(project_id, group_id, 0, io),
        Effect::CreateGroup { topic } => {
            spawn_command(project_id, Command::CreateGroup { topic }, io)
        }
        Effect::DeleteGroup { group_id } => {
            spawn_command(project_id, Command::DeleteGroup { group_id }, io)
        }
    }
}

/// 选中某群(`Command::SelectGroup`:同步状态变更),再执行它产生的 Effect(清空后从头拉消息)。
/// 其余命令走异步 [`spawn_command`];`OpenTodo`(切面板)是跨面板动作,由 host 处理。
pub fn select_group(
    state: &mut WorkspaceState,
    project_id: i64,
    group_id: i64,
    io: &PanelIo<Message>,
) {
    for effect in state.select(group_id) {
        run_effect(effect, project_id, io);
    }
}

/// `ResumeTimeReached` 时调用:该加载群列表就加载,该轮询当前群的消息就轮询
/// (限速、在途、退避都在 `WorkspaceState` 里)。"面板是否可见"由 host 在调用前判断。
pub fn poll_if_due(
    state: &mut WorkspaceState,
    project_id: i64,
    now: Instant,
    io: &PanelIo<Message>,
) {
    if state.load_due() {
        spawn_load_groups(project_id, io);
        return;
    }
    let (Some(group_id), true) = (state.selected(), state.has_active_turn()) else {
        return;
    };
    if state.poll_due(now) {
        let after_rev = state.latest_rev();
        spawn_fetch_messages(project_id, group_id, after_rev, io);
    }
}

'''
s = s[:i] + new + s[j:]
s = s.replace("use dozer_client::Client;\n", "use crate::panel_host::PanelIo;\n", 1) if "use dozer_client::Client;\n" in s else s
s = s.replace("use tokio::runtime::Handle;\n", "", 1)
open(p, "w").write(s)

# 3) host 胶水:ShellIo / App 各一个 panel_io 构造
edit("workspace/state.rs", [
("impl ShellIo {\n", '''impl ShellIo {
    /// 给面板模块的执行原语:`wrap` 把面板自己的 `Message` 包成宿主 `Message`(通常是 `Message::<面板>`)。
    pub(crate) fn panel_io<M: Send + 'static>(
        &self,
        wrap: fn(M) -> Message,
    ) -> crate::panel_host::PanelIo<M> {
        let proxy = self.proxy.clone();
        crate::panel_host::PanelIo::new(self.client.clone(), self.handle.clone(), move |m| {
            let _ = proxy.send_event(wrap(m));
        })
    }

'''),
])
edit("app/app.rs", [
("    pub(crate) fn shell_io(&self) -> ShellIo {",
 '''    /// 同 [`ShellIo::panel_io`],直接从 `App` 取。
    pub(crate) fn panel_io<M: Send + 'static>(
        &self,
        wrap: fn(M) -> Message,
    ) -> crate::panel_host::PanelIo<M> {
        self.shell_io().panel_io(wrap)
    }

    pub(crate) fn shell_io(&self) -> ShellIo {'''),
("""        let now = std::time::Instant::now();
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m: crate::extensions::group_chat::Message| {
            let _ = proxy.send_event(Message::GroupChat(m));
        };
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        if ws.group_chat.load_due() {
            crate::extensions::group_chat::spawn_load_groups(project_id, &client, &handle, emit);
            return;
        }
        let (Some(group_id), true) = (ws.group_chat.selected(), ws.group_chat.has_active_turn())
        else {
            return;
        };
        if ws.group_chat.poll_due(now) {
            let after_rev = ws.group_chat.latest_rev();
            crate::extensions::group_chat::spawn_fetch_messages(
                project_id, group_id, after_rev, &client, &handle, emit,
            );
        }
""", """        let now = std::time::Instant::now();
        let io = self.panel_io(Message::GroupChat);
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        crate::extensions::group_chat::poll_if_due(&mut ws.group_chat, project_id, now, &io);
"""),
])

# 4) host:Message::GroupChat 臂、group_chat_command、删 run_group_chat_effects
edit("app/update.rs", [
("""            Message::GroupChat(msg) => {
                let project_id = msg.project_id();
                let mut effects = Vec::new();
                self.with_project(project_id, |ws, _io| {
                    effects = crate::extensions::group_chat::update(&mut ws.group_chat, msg);
                });
                self.run_group_chat_effects(project_id, effects);
            }""", """            Message::GroupChat(msg) => {
                let project_id = msg.project_id();
                self.with_project(project_id, |ws, io| {
                    let pio = io.panel_io(Message::GroupChat);
                    for effect in crate::extensions::group_chat::update(&mut ws.group_chat, msg) {
                        crate::extensions::group_chat::run_effect(effect, project_id, &pio);
                    }
                });
            }"""),
])
p = SRC + "app/update.rs"
s = open(p).read()
i = s.index("    fn group_chat_command(&mut self, project_id: i64, cmd: crate::extensions::group_chat::Command) {")
j = s.index("    pub(crate) fn browser_bookmarks_loaded(")
new_cmd = '''    fn group_chat_command(&mut self, project_id: i64, cmd: crate::extensions::group_chat::Command) {
        use crate::extensions::group_chat::{self as gc, Command};
        match cmd {
            Command::SelectGroup { group_id } => {
                self.with_project(project_id, |ws, io| {
                    let pio = io.panel_io(Message::GroupChat);
                    gc::select_group(&mut ws.group_chat, project_id, group_id, &pio);
                });
            }
            Command::OpenTodo { .. } => {
                // 只确保 Todo 面板看得见;不做"定位到某条待办"(Todo 面板没有这个入口,
                // 且不在本功能范围)。不能用 `panel_select`:它对已激活的面板是"收起",
                // 还会武装图标栏拖拽。(跨面板动作,留在 host。)
                self.show_panel(PanelKind::Todo);
            }
            // 其余命令不依赖工作区状态,直接 spawn(与迁移前一致:项目工作区没加载也照常执行)。
            other => gc::spawn_command(project_id, other, &self.panel_io(Message::GroupChat)),
        }
    }

'''
s = s[:i] + new_cmd + s[j:]
open(p, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1
python3 $SCRATCH/h4_impl.py && cargo fmt -p dozer-app && git status --short
```
Expected: `git status --short` 只有 `app/app.rs`、`app/update.rs`、`extensions/group_chat/mod.rs`、`panel_host.rs`、`workspace/state.rs`(和 `Cargo.lock`,马上还原)。脚本里每个替换点都先 `assert` 原文存在;若报 `AssertionError`,说明 `main` 上这几处已被改动,按报错点对照 `git diff main` 手工对齐后再继续,**不要**改断言。

- [ ] **Step 5: 编译、全量测试、clippy**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A9 | head -40
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
```
Expected: build 无输出;测试通过数 = N + 13,失败只有已知项;`clippy: no new diagnostics`。

- [ ] **Step 6: 逐条审阅 host 侧 diff(没有 `App` 夹具可测)**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && git diff -- crates/dozer-app/src/app crates/dozer-app/src/workspace | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)"`
Expected(逐条对照,草稿里 host 侧净减约 59 行:删 89 行、加 30 行):
1. `App::panel_io` / `ShellIo::panel_io` 是新增的构造胶水,`wrap: fn(M) -> Message`,内部 `proxy.send_event(wrap(m))`;
2. `poll_group_chat_if_active`:仍先判 `group_chat_panel_visible()`、仍取 `active_project_id`、仍要 `active_workspace_mut()` 才继续;循环体搬进 `group_chat::poll_if_due`,判定顺序(先 `load_due`、再 `selected && has_active_turn`、再 `poll_due`)逐句对应;
3. `Message::GroupChat(msg)` 臂:`update` 产生的 Effect 逐个交给 `run_effect`,仍然只在 `with_project` 找到工作区时才会有 Effect;
4. `group_chat_command`:`SelectGroup` 仍在 `with_project` 里(它要改状态);`OpenTodo` 仍是 `show_panel(PanelKind::Todo)`(跨面板动作,留在 host);**其余命令直接 `gc::spawn_command(project_id, other, &self.panel_io(Message::GroupChat))`,不在 `with_project` 里**(与迁移前一致);
5. `run_group_chat_effects` 整个被删,工程里不再有对它的引用(`grep -rn run_group_chat_effects crates/` 无输出)。

- [ ] **Step 7: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1
git add crates
cat > $SCRATCH/h4_mutate.py <<'EOF'
import sys
which = sys.argv[1]
p = "crates/dozer-app/src/extensions/group_chat/mod.rs"
s = open(p).read()
if which == "1":   # run_effect 把 CreateGroup 误映射成删除
    a = "        Effect::CreateGroup { topic } => {\n            spawn_command(project_id, Command::CreateGroup { topic }, io)\n        }"
    b = "        Effect::CreateGroup { topic } => {\n            let _ = topic;\n            spawn_command(project_id, Command::DeleteGroup { group_id: 0 }, io)\n        }"
elif which == "2":  # 轮询不再要求"有进行中的发言"
    a = "    let (Some(group_id), true) = (state.selected(), state.has_active_turn()) else {"
    b = "    let (Some(group_id), true) = (state.selected(), true) else {"
elif which == "3":  # select_group 只改状态、不再拉消息
    a = "    for effect in state.select(group_id) {\n        run_effect(effect, project_id, io);\n    }"
    b = "    let _ = state.select(group_id);\n    let _ = (project_id, io);"
assert a in s, which
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3; do echo "M$m:"; python3 $SCRATCH/h4_mutate.py $m && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app extensions::group_chat 2>&1 | grep -E "^error|FAILED$|test result" | head -3; git checkout -- crates/dozer-app/src/extensions/group_chat/mod.rs Cargo.lock; done
git diff --stat | wc -l; git status --short
```
Expected: 三次变异各自让**对应**的测试失败——M1:`run_effect_maps_each_effect_to_its_task`;M2:`poll_if_due_polls_only_a_selected_group_with_an_active_turn`;M3:`select_group_changes_selection_and_fetches_its_messages`;每次还原后 `git diff --stat | wc -l` 为 `0`(暂存的实现版本被恢复)。

- [ ] **Step 8: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4
git status --short
git add crates
git diff --cached --stat | tail -8
git commit -m "refactor(group_chat): PanelIo; Effect executor, command dispatch and poll decision live in the panel

host no longer knows group_chat's Effect variants (run_group_chat_effects is gone): it only
builds a PanelIo (client + runtime handle + message wrapper) and hands it to the panel.
First unit tests for this path, using a Client pointed at a nonexistent socket.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `git diff --cached --stat` 里只有 5 个 `crates/` 文件(没有 `Cargo.lock`、`scripts/`、`docs/`)。

---

### Task 2: 门禁(host 替面板做事的棘轮)与文档回填

**Files:**
- Modify: `scripts/audit/test_check_panel_boundary.py`、`scripts/audit/check_panel_boundary.py`、`scripts/audit/panel-boundary.baseline.json`
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1 的提交。
- Produces:门禁规则 `R-HOST-PANEL-ARMS`(`app/update.rs` 里 12 空格缩进的 `Message::<面板>(` 臂条数,基线 97)、`R-HOST-EXECUTORS`(`app/` 下 `fn run_<面板>_effects` 个数,基线 0 即不出现在基线里),均只许减不许增。

- [ ] **Step 1: 写失败的测试**

创建 `$SCRATCH/test_gate_h4.py` 并把它**插入** `scripts/audit/test_check_panel_boundary.py` 里 `class Compare(unittest.TestCase):` 之前:

```python
class HostPanelCoupling(unittest.TestCase):
    """R-HOST-PANEL-ARMS / R-HOST-EXECUTORS:host 里"替面板做事"的代码只许减不许增(H4)。"""
    def scan1(self, rel, text):
        return g.scan({rel: text}).get(rel, {})
    def test_panel_specific_arms_in_app_update_are_counted(self):
        text = (
            "            Message::Files(files::Message::CopyPath(p, k)) => {}\n"
            "            Message::GitLog(msg) => {}\n"
            "            Message::TermInput(x) => {}\n"          # 非面板消息
            "                Message::Files(inner) => {}\n"       # 缩进更深:不是顶层臂
        )
        self.assertEqual(self.scan1("app/update.rs", text), {"R-HOST-PANEL-ARMS": 2})
    def test_arms_in_other_files_are_not_counted(self):
        text = "            Message::Files(files::Message::CopyPath(p, k)) => {}\n"
        self.assertEqual(self.scan1("app/view.rs", text), {})
    def test_commented_out_arms_are_not_counted(self):
        text = "            // Message::Files(x) => {}\n"
        self.assertEqual(self.scan1("app/update.rs", text), {})
    def test_per_panel_effect_executors_in_host_are_counted(self):
        text = "    fn run_group_chat_effects(&mut self) {}\n    fn run_other_effects(&mut self) {}\n    fn run_effect(&mut self) {}\n"
        self.assertEqual(self.scan1("app/update.rs", text), {"R-HOST-EXECUTORS": 2})
    def test_executors_outside_app_are_not_counted(self):
        text = "pub fn run_group_chat_effects() {}\n"
        self.assertEqual(self.scan1("extensions/group_chat/mod.rs", text), {})
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1
python3 - <<'EOF'
import os
S = os.path.expanduser("~/Projects/CoralProjects/byteboy/dozer-bytehost-h4-scratch")
p = "scripts/audit/test_check_panel_boundary.py"
s = open(p).read()
a = "class Compare(unittest.TestCase):"
assert a in s
open(p, "w").write(s.replace(a, open(os.path.join(S, "test_gate_h4.py")).read() + a, 1))
EOF
python3 scripts/audit/test_check_panel_boundary.py 2>&1 | grep -E "^(FAIL|ERROR)|^Ran|^FAILED|^OK"
```
Expected: `FAIL: test_panel_specific_arms_in_app_update_are_counted`、`FAIL: test_per_panel_effect_executors_in_host_are_counted`(其余 3 个本来就该过),`FAILED (failures=2)`。

- [ ] **Step 2: 实现规则**

创建 `$SCRATCH/gate_patch.py` 并运行:

```python
p = "scripts/audit/check_panel_boundary.py"
s = open(p).read()
a = "def scan(files):"
assert a in s
s = s.replace(a, '''# host 里"替面板做事"的两种形态(H4 起,规格 §6 验收 2):
#  - R-HOST-PANEL-ARMS:`app/update.rs` 里顶层(12 空格缩进)的"某个面板的消息"臂条数——host 对该面板
#    有专属处理体的入口(含纯转发臂,所以这是个粗指标,只看趋势:只许减不许增);
#  - R-HOST-EXECUTORS:`app/` 下名为 `run_<面板>_effects` 的 host 侧面板 Effect 执行器(应随面板走,
#    host 只给 `PanelIo`)。
HOST_PANEL_ARM_RE = re.compile(
    r"^ {12}Message::(?:Files|GitLog|Todo|Project|Database|Ssh|Browser|HomeBrowser|Conversations|Usage"
    r"|CodeHealth|GroupChat|AgentContext|Search|Settings|ProjectCreate|FileHistory|EditHistory|Footbar)\\(",
    re.M,
)
HOST_EXECUTOR_RE = re.compile(r"\\bfn run_[a-z_]+_effects\\b")


def scan(files):''', 1)
a = "        n = len(PANE_PICK_RE.findall(edges.strip_comments(text)))"
assert a in s
s = s.replace(a, '''        code = edges.strip_comments(text)
        if rel == "app/update.rs":
            k = len(HOST_PANEL_ARM_RE.findall(code))
            if k:
                hit["R-HOST-PANEL-ARMS"] = k
        if rel.startswith("app/"):
            k = len(HOST_EXECUTOR_RE.findall(code))
            if k:
                hit["R-HOST-EXECUTORS"] = k
''' + a, 1)
a = '规则 R-HOVERID-VARIANTS / R-HOVERSLOT-VARIANTS: `HoverId`(app/state.rs)与 `HoverSlot`(panel_host.rs)的变体数'
assert a in s
s = s.replace(a, a + '\n规则 R-HOST-PANEL-ARMS / R-HOST-EXECUTORS: `app/update.rs` 里面板专属消息臂条数 / `app/` 下 `run_<面板>_effects` 执行器个数', 1)
open(p, "w").write(s)
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1 && python3 $SCRATCH/gate_patch.py && python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -3`
Expected: `Ran 23 tests` … `OK`。

- [ ] **Step 3: 更新基线并核对**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1 && python3 scripts/audit/check_panel_boundary.py; python3 scripts/audit/check_panel_boundary.py --update && tr -d '\n ' < scripts/audit/panel-boundary.baseline.json; echo; python3 scripts/audit/check_panel_boundary.py`
Expected: 第一次检查对着旧基线报 `app/update.rs: R-HOST-PANEL-ARMS 0 -> 97`(新规则的现存值,预期);`--update` 后基线为 `{"app/state.rs":{"R-HOVERID-VARIANTS":12},"app/update.rs":{"R-HOST-PANEL-ARMS":97,"R-PANE-PICK":1},"panel_host.rs":{"R-HOVERSLOT-VARIANTS":9},"platform/window_events.rs":{"R-PANE-PICK":3},"workspace/state.rs":{"R-PANE-PICK":2}}`(**没有 `R-HOST-EXECUTORS`**:基线 0 不记录,任何新出现的执行器都会是 `0 -> n`);最后一次检查 `ok`。若 `R-HOST-PANEL-ARMS` 不是 97,说明 `main` 上这个文件在草稿之后有改动——以实测为准写进基线,并在回填文档时用实测值。

- [ ] **Step 4: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1
git add scripts/audit
printf '\n    fn run_foo_effects(&mut self) {}\n' >> crates/dozer-app/src/app/update.rs
python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"
git checkout -- crates/dozer-app/src/app/update.rs; python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"; git status --short crates | wc -l
```
Expected: 变异后 `app/update.rs: R-HOST-EXECUTORS 0 -> 1` 且 `exit=1`;还原后 `ok`、`exit=0`、最后输出 `0`(`crates/` 回到 Task 1 提交的版本)。

- [ ] **Step 5: 回填文档**

每条先 `grep -n` 找到原文再改,改完 `git diff` 看一遍:
1. `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md` §3.1 的 `PanelIo` 代码草图,把 `spawn`/`spawn_blocking` 两行改成实现的形态:`pub fn spawn<F, Fut>(&self, task: F) where F: FnOnce(Client, PanelIo<M>) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static; // 任务拿到 host 的 Client 与一份 PanelIo 副本,想发几条消息(含零条)自己定`,并在草图后加一句"`spawn_blocking` 暂时没人用,按需再加(H4 实现取舍)";§4 表里 H4 一行末尾追加"**已完成(H4,`bytehost-h4`):`<Task 1 提交短 id>`**"。
2. `docs/dozer-v2/bytehost-H0/E3-registry.tsv` 的 `E3-014` 行:把"原因"列改成"`group_chat` 的 Effect 执行器、命令派发、轮询判定已于 H4 迁入 `group_chat` 模块(host 只构造 `PanelIo`);host 里仍剩 `Command::OpenTodo` → `show_panel(Todo)`(跨面板动作)与 `group_chat_content_event` 对 webview 事件的 Ready/Failed 处理(host 持有 `group_chat_webview`)","移除条件"列改成"跨面板动作经 `HostEffect::ShowPanel`(H5);webview 推送状态随面板自持(H7/H8)"。改完运行 `awk -F'\t' 'NR>1 && (NF!=6 || $6=="")' docs/dozer-v2/bytehost-H0/E3-registry.tsv | wc -l`,Expected: `0`。
3. `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7:在 H3 那条之后追加"**bytehost H4 已完成(2026-10-04):** 引入 `PanelIo`(host 给面板的执行原语),`group_chat` 的 Effect 执行器/命令派发/轮询判定迁入面板模块并补上首批单测(假 `Client` 指向不存在的 socket);host 里 `run_group_chat_effects` 删除;门禁新增 `R-HOST-PANEL-ARMS`(基线 97)与 `R-HOST-EXECUTORS`。H5(`HostEffect`:`ShowPanel`/`Emit`/`PickDirectory`)是下一步。"

- [ ] **Step 6: 最终校验并提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h4 && export PYTHONDONTWRITEBYTECODE=1
python3 scripts/audit/test_edges.py 2>&1 | tail -1; python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1; python3 scripts/audit/check_panel_boundary.py
git status --short
git add scripts/audit/check_panel_boundary.py scripts/audit/test_check_panel_boundary.py scripts/audit/panel-boundary.baseline.json docs
git diff --cached --stat | tail -8
git commit -m "chore(audit): ratchet host-side panel arms and per-panel effect executors; backfill H4 docs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 两组脚本测试 `OK`、门禁 `ok`;`git diff --cached --stat` 里只有 `scripts/audit/` 与 `docs/`(没有 `crates/`、`Cargo.lock`)。

---

## Self-Review

**1. 覆盖:** 设计文档 §4 的 H4(`PanelIo` + group_chat Effect 执行器/命令的 spawn 分支迁入 + effect 单测)→ Task 1;§6 验收 2(新增门禁)→ Task 2。设计文档里 H4 提到"删 `group_chat_command` 的 spawn 分支":实现里 `group_chat_command` 保留(`OpenTodo` 跨面板动作与 `SelectGroup` 的 `with_project` 仍在 host),但 spawn 逻辑全部在面板模块里。

**2. 占位符扫描:** 测试、实现脚本、门禁补丁均为草稿里跑通的完整版本;文档回填给出要写入的文字。

**3. 一致性:** `PanelIo` 的方法签名在实现脚本、测试、Interfaces 里一致;`select_group`(取代草稿第一版的 `dispatch_command`)在测试、实现脚本、host 调用点里一致;门禁规则名与基线一致。

**4. Review Focus:** 5 条各有归属(1→Step 6、2→Step 3 测试说明、3→Step 5 编译、4→Step 5/6 的 grep、5→Task 2 Step 3 与门禁注释)。
