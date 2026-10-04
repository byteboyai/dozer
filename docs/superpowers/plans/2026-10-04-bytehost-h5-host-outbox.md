# bytehost H5:`HostOutbox`/`PanelCommand`——面板向 host 提需求(系统对话框与跨面板命令) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地设计文档 H5 的第一刀(`docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md` §3.2、§4):面板不再靠"host/窗口层拦截它的消息"来做只有 host 才能做的事,而是往自己 state 的 `HostOutbox` 里提**需求**(`HostRequest`),host 在每条消息之后统一排空执行。第一批迁 5 处:3 个系统"选择文件夹"对话框(`project_create` 的两处"选择根目录…"、`Files` 的"拖拽移动到目录…")和 `Files` 的 2 个跨面板消息臂(右键"搜索"、右键"查看此文件历史",经**受限的 `PanelCommand`**——用户 2026-10-04 对 P1 的裁决)。窗口层少 3 条拦截臂(−45 行),`App::update` 少 2 条 `Files` 专属臂。**不改任何用户可见行为。**

**Architecture:** 单个代码任务 + 一个门禁/文档任务。Task 1 测试先行(6 个测试,先看编译失败),再跑一个实现脚本:加 `PanelCommand`/`HostRequest<M>`/`HostOutbox<M>`(`panel_host.rs`),`Files`/`project_create` 的 state 各加一个 `host` outbox,把原来 `unreachable!`/空操作的臂改成提需求,host 加 `drain_host_requests`/`run_host_requests`/`run_panel_command`/`show_file_history`,删掉 host 与窗口层对应的拦截臂;4 个变异检验证明测试会失败。Task 2 把门禁基线 `R-HOST-PANEL-ARMS` 由 97 压到 95,回填文档。

**Tech Stack:** Rust(`dozer-app`,iced 0.14,rfd);Python 3 标准库(一次性脚本 + 审计门禁)。不碰 `Cargo.toml`/`Cargo.lock`。

**Spec:** 设计文档 §3.2(两层 Effect:host 通用 Effect 只收"只有 host 能做的事")、§4 H5、P1(已定:受限 `PanelCommand`)。依据 H0 的 E3-010/E3-011、H4 的 `PanelIo`。

## 对设计文档的两处**细化**(请评审)

1. **需求经 outbox 返回,而不是 `update` 的返回值。** 设计文档写的是"返回 `Vec<Effect>` 的纯函数"。读代码后发现各面板 `update` 的签名并不统一(`files::update` 有 8 个参数且返回 `()`,`project_create::update` 吃 `&mut Option<State>`,`search::update` 又是另一套),统一改成返回值要动十几个调用点和上百个测试。仓库里已经有现成的同款模式——`toast::Outbox`(面板往自己 state 的 outbox 里 `push`,`App::update` 的包装函数每条消息后排空)。所以 `HostOutbox` 沿用它:**面板的 `update` 签名不变**,"纯函数"的可测试性不变(测试直接看 outbox 里有什么)。
2. **本刀的词汇比设计文档小。** 设计文档 §3.2 列了 `ShowPanel`/`Emit`/`PickDirectory`/`OpenExternal`/`Toast`。本刀只落地真有第一个使用者的两个:`PickDirectory` 与 `Command(PanelCommand)`(`PanelCommand` 只有 `SearchIn`、`ShowFileHistory` 两个成员)。`ShowPanel`(群聊的 `OpenTodo`)等有使用者时再加(YAGNI,规格 §1)。

## Global Constraints

- **行为保持。** 5 处迁移的用户可见行为逐字不变:对话框的起始目录、选中后回填的消息、搜索/历史弹窗的打开方式、右键菜单先被关闭。**例外只有两处,都是退化路径、都在下面的 Review Focus 里逐条论证。**
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h5/...`),分支 `bytehost-h5`,从当前 `main` 开(H0–H4 都在 `main` 上)。主 checkout 常有并发会话与未提交改动(此刻 `crates/dozerd/src/transcripts/mod.rs` 就是别人的),不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]` 会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**;`.cargo/config.toml` 不提交;本计划不改依赖。
- **变异检验前先 `git add` 暂存当前版本**,变异后用 `git checkout -- <文件>` 还原到暂存版本。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了。**
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`**(沿用 H1 的防线)。
- **heredoc 一律用带引号的分隔符(`<<'EOF'`)写 Python 脚本,绝不用不带引号的**:shell 会展开脚本里的反引号和 `$`,把脚本和文档改坏(H4 里我犯过两次)。需要传值给脚本时用环境变量(`H5=… python3 script.py`),不要把 shell 变量拼进 heredoc。
- **clippy 诊断必须与 `main` 逐文件一致:** 草稿里第一版测试 `let mut s = AppState::default(); s.context_menu = Some(..)` 触发了 `field_reassign_with_default`,已改成结构体更新语法。
- 提交信息用 `refactor:`/`test:`/`chore(audit):`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败;`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在 linked worktree 里可能失败;`assets::tests::serves_vendored_asset_with_mime` 偶发失败。
- 草稿基线(`main` = H4 之后,linked worktree):`1750 passed; 1 failed`。草稿里 Task 1 之后 `1756 passed; 1 failed`(+6 个新测试),失败只有 `delete_confirm`。门禁基线:`R-HOST-PANEL-ARMS` 97 → 95。

## Review Focus

1. **`OpenSearch` 现在要经过通用的 `Message::Files(msg)` 臂,而那条臂要求有活动项目且工作区已加载**(旧的专属臂不要求)。两者在实际使用里等价——"搜索"菜单项只出现在已加载工作区的文件树右键菜单里;`FileHistoryOpen` 旧代码本来就要求活动项目与已加载工作区。这是唯一一处退化路径上的行为差异,不用测试固定(没有 `App` 夹具),审阅时确认。
2. **`MoveDirBrowse` 的起始目录:** 旧代码总是 `set_directory(草稿或空路径)`;新代码只在有草稿时设置(`start: Option<PathBuf>`)。没有待确认的移动时这条消息根本不会派发(旧代码注释已说明 `unwrap_or_default` 只是防御性兜底),所以只有退化路径不同。测试固定"有草稿时起始目录就是草稿"。
3. **对话框的执行时机与线程:** 旧代码在窗口层(`Runner::dispatch`)**先于** `App::update` 拦截消息弹对话框;新代码在 `App::update` 处理完消息之后(`drain_host_requests`)弹。两者都在事件循环线程上(`App::update` 只在那里被调用),macOS 要求的主线程约束不变;选中后都是 `self.update(<回填消息>)`——排空过程里再进 `update`,`update` 末尾再排空,outbox 已被 `take` 清空,不会重复执行。
4. **右键菜单的关闭:** 旧 host 臂用 `self.files.close_context_menu()`;新代码在 `files::update` 里用 `app_state.close_context_menu()`——`App::update` 的 `Message::Files` 臂把 `&mut self.files` 作为 `app_state` 传入,是**同一个对象**。测试断言"先关菜单"。
5. **host 侧的三个新函数没有测试**(项目没有 `App` 夹具,已知缺口):`run_host_requests`、`run_panel_command`、`show_file_history`。`show_file_history` 的函数体是旧 `FileHistoryOpen` 臂**逐字搬来**(除了 `close_context_menu` 已前移到面板);`run_panel_command::SearchIn` 的 `Scope` 选择逻辑也是旧臂原文。Step 6 逐条对照 diff。
6. **没有迁的(本刀明确不碰):** `ProjectTabPickFolder`(host 自己的顶栏入口,不属于任何面板)、`ProjectLinkPick`(用 `platform::picker`,macOS/非 macOS 两条 cfg 分支,非 macOS 从未在本机编译)、`Files` 的 `CopyPath`(要窗口层的剪贴板句柄)、`FileHistoryRollbackPrevious`(结果要回投 `files::Message::FileHistoryRollbackDone`,跨面板回投需要先定 `PanelCommand` 的"回复"形态)、`Search::Pick`(改窗口层的焦点意图)、`Browser::Nav`(要原生 webview 句柄)。它们进 E3 清单的"剩余"说明,留给后续切片。
7. **host 里仍是逐面板的排空清单:** `drain_host_requests` 里 `files` 与 `project_create` 各一行——这是过渡形态(与已有的 `drain_outboxes` 里逐面板 `take_outbox` 同一个问题),注册制(H7)落地后改成遍历 registry。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/panel_host.rs` | 新增 `PanelCommand`、`HostRequest<M>`、`HostOutbox<M>` + 2 个测试 | 1 |
| `crates/dozer-app/src/extensions/files/{state,update,mod}.rs` | `host` outbox 字段;`MoveDirBrowse`/`OpenSearch`/`FileHistoryOpen` 改提需求;3 个测试;订正 3 处变体文档 | 1 |
| `crates/dozer-app/src/extensions/project_create.rs` | `host` outbox 字段;两处 `*RootDirPick` 改提需求;1 个测试 | 1 |
| `crates/dozer-app/src/app/update.rs` | `drain_host_requests`/`run_host_requests`/`run_panel_command`/`show_file_history`;删 2 条 `Files` 专属臂 | 1 |
| `crates/dozer-app/src/platform/window_events.rs` | 删 3 条系统对话框拦截臂 | 1 |
| `scripts/audit/panel-boundary.baseline.json` | `R-HOST-PANEL-ARMS` 97 → 95 | 2 |
| `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 | 2 |

一次性脚本(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: `HostOutbox`/`PanelCommand` 与 5 处迁移

**Files:** 见上表(`crates/` 下 7 个文件)。

**Interfaces:**
- Produces(Task 2 与后续切片依赖):`crate::panel_host::{PanelCommand, HostRequest<M>, HostOutbox<M>}`——`PanelCommand::{SearchIn { path: PathBuf, is_dir: bool }, ShowFileHistory { path: PathBuf }}`;`HostRequest::{PickDirectory { start: Option<PathBuf>, on_picked: fn(PathBuf) -> M }, Command(PanelCommand)}`;`HostOutbox<M>::{default(), push(HostRequest<M>), take() -> Vec<HostRequest<M>>}`(均 `pub(crate)`);`files::WorkspaceState.host`、`project_create::State.host`(`pub(crate)`);`App::{drain_host_requests, run_host_requests<M>(Vec<HostRequest<M>>, fn(M) -> Message), run_panel_command(PanelCommand)}`(私有)。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h5 -b bytehost-h5 main
mkdir -p ../dozer-bytehost-h5/.cargo && cp .cargo/config.toml ../dozer-bytehost-h5/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h5-scratch && mkdir -p $SCRATCH
```

Expected: 干净、分支 `bytehost-h5`。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h5/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里列的;记下通过数 N(草稿:1750)。再记 clippy 基线:`RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git checkout -- Cargo.lock`(草稿:11 个文件有既有诊断)。

- [ ] **Step 3: 写失败的测试**

创建三个文件:

`$SCRATCH/tests_panel_host.rs`(**追加进 `panel_host.rs` 的 `mod tests { … }` 末尾**):

```rust

    // ---- H5:HostOutbox / HostRequest / PanelCommand ----

    #[test]
    fn host_outbox_returns_requests_in_push_order_and_empties_itself() {
        let mut out: HostOutbox<u32> = HostOutbox::default();
        out.push(HostRequest::Command(PanelCommand::SearchIn {
            path: "/a".into(),
            is_dir: true,
        }));
        out.push(HostRequest::PickDirectory {
            start: None,
            on_picked: |p| p.as_os_str().len() as u32,
        });
        let reqs = out.take();
        assert_eq!(reqs.len(), 2);
        assert!(matches!(
            &reqs[0],
            HostRequest::Command(PanelCommand::SearchIn { is_dir: true, .. })
        ));
        assert!(matches!(
            &reqs[1],
            HostRequest::PickDirectory { start: None, .. }
        ));
        assert!(out.take().is_empty());
    }

    #[test]
    fn pick_directory_reply_maps_the_picked_path_into_the_panels_message() {
        let req: HostRequest<String> = HostRequest::PickDirectory {
            start: Some("/s".into()),
            on_picked: |p| p.display().to_string(),
        };
        let HostRequest::PickDirectory { start, on_picked } = req else {
            panic!("expected PickDirectory");
        };
        assert_eq!(start, Some(std::path::PathBuf::from("/s")));
        assert_eq!(on_picked("/x/y".into()), "/x/y");
    }
```

`$SCRATCH/tests_files.rs`(**追加进 `extensions/files/mod.rs` 的 `mod tests { … }` 末尾**):

```rust

    // ---- H5:需要 host 才能做的动作,经 HostOutbox 提给 host ----

    use crate::panel_host::{HostRequest, PanelCommand};

    async fn dispatch(ws_state: &mut WorkspaceState, app_state: &mut AppState, msg: Message) {
        let handle = tokio::runtime::Handle::current();
        update(
            ws_state,
            app_state,
            msg,
            1,
            &handle,
            |_| {},
            &crate::external_apps::ExternalAppsConfig::default(),
            true,
        );
    }

    fn open_context_menu() -> AppState {
        AppState {
            context_menu: Some(ContextMenu {
                x: 1.0,
                y: 2.0,
                target: PathBuf::from("/proj/a.txt"),
                is_dir: false,
                agent_terminal_visible: true,
            }),
            ..AppState::default()
        }
    }

    #[tokio::test]
    async fn move_dir_browse_asks_the_host_for_a_directory_starting_at_the_draft() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        ws_state.pending_move = Some(PendingMove {
            source: PathBuf::from("/a/b.txt"),
            source_is_dir: false,
            name_draft: "b.txt".into(),
            dir_draft: "/tmp/x".into(),
        });
        dispatch(&mut ws_state, &mut AppState::default(), Message::MoveDirBrowse).await;
        let mut reqs = ws_state.host.take();
        assert_eq!(reqs.len(), 1);
        match reqs.remove(0) {
            HostRequest::PickDirectory { start, on_picked } => {
                assert_eq!(start, Some(PathBuf::from("/tmp/x")));
                assert!(matches!(
                    on_picked(PathBuf::from("/tmp/y")),
                    Message::MoveDirInput(ref s) if s == "/tmp/y"
                ));
            }
            _ => panic!("expected PickDirectory"),
        }
    }

    #[tokio::test]
    async fn open_search_closes_the_context_menu_and_asks_the_host_to_search_there() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = open_context_menu();
        dispatch(
            &mut ws_state,
            &mut app_state,
            Message::OpenSearch(PathBuf::from("/proj/dir"), true),
        )
        .await;
        assert!(app_state.context_menu.is_none(), "先关右键菜单");
        let reqs = ws_state.host.take();
        assert_eq!(reqs.len(), 1);
        assert!(matches!(
            &reqs[0],
            HostRequest::Command(PanelCommand::SearchIn { path, is_dir: true })
                if path == &PathBuf::from("/proj/dir")
        ));
    }

    #[tokio::test]
    async fn file_history_open_closes_the_context_menu_and_asks_the_host_to_show_history() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = open_context_menu();
        dispatch(
            &mut ws_state,
            &mut app_state,
            Message::FileHistoryOpen(PathBuf::from("/proj/a.txt")),
        )
        .await;
        assert!(app_state.context_menu.is_none(), "先关右键菜单");
        let reqs = ws_state.host.take();
        assert_eq!(reqs.len(), 1);
        assert!(matches!(
            &reqs[0],
            HostRequest::Command(PanelCommand::ShowFileHistory { path })
                if path == &PathBuf::from("/proj/a.txt")
        ));
    }
```

`$SCRATCH/tests_project_create.rs`(**追加进 `extensions/project_create.rs` 的 `mod tests { … }` 末尾**):

```rust

    // ---- H5:两处"选择根目录…"经 HostOutbox 提给 host ----

    use crate::panel_host::HostRequest;

    #[test]
    fn root_dir_pick_buttons_ask_the_host_for_a_directory_and_map_the_reply_back() {
        for (pick, expect_local) in [
            (Message::LocalRootDirPick, true),
            (Message::CloneRootDirPick, false),
        ] {
            let mut s = State::default();
            assert!(apply_field_message(&mut s, &pick));
            let mut reqs = s.host.take();
            assert_eq!(reqs.len(), 1);
            match reqs.remove(0) {
                HostRequest::PickDirectory { start, on_picked } => {
                    assert_eq!(start, None);
                    match (on_picked(std::path::PathBuf::from("/r")), expect_local) {
                        (Message::LocalRootDirPicked(p), true) | (Message::CloneRootDirPicked(p), false) => {
                            assert_eq!(p, "/r");
                        }
                        (other, _) => panic!("unexpected reply: {other:?}"),
                    }
                }
                _ => panic!("expected PickDirectory"),
            }
        }
    }
```

把三段追加进去并确认编译失败:

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && export PYTHONDONTWRITEBYTECODE=1
python3 - <<'EOF'
import os
S = os.path.expanduser("~/Projects/CoralProjects/byteboy/dozer-bytehost-h5-scratch")
for path, name in (("crates/dozer-app/src/panel_host.rs", "tests_panel_host.rs"),
                   ("crates/dozer-app/src/extensions/files/mod.rs", "tests_files.rs"),
                   ("crates/dozer-app/src/extensions/project_create.rs", "tests_project_create.rs")):
    s = open(path).read()
    i = s.rindex("\n}\n")  # 文件最后一个 `}` 是 `mod tests` 的结尾
    open(path, "w").write(s[:i] + "\n" + open(os.path.join(S, name)).read().rstrip("\n") + "\n}\n")
EOF
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app host_outbox 2>&1 | grep -E "^error" | head -4; git checkout -- Cargo.lock
```
Expected: 编译失败,`unresolved imports \`crate::panel_host::HostRequest\`, \`crate::panel_host::PanelCommand\``、`cannot find type \`HostOutbox\` in this scope`。

- [ ] **Step 4: 实现脚本**

创建 `$SCRATCH/h5_impl.py`:

```python
#!/usr/bin/env python3
"""H5 一次性实现脚本:HostOutbox/HostRequest/PanelCommand;Files 与 project_create 把"需要 host 才能做"的
动作经 outbox 提给 host;host 执行器 run_host_requests/run_panel_command;删掉 host/窗口层对应的拦截臂。
**不提交。** 在仓库根运行。每个替换点先 assert 原文存在。"""
SRC = "crates/dozer-app/src/"


def edit(path, pairs):
    p = SRC + path
    s = open(p).read()
    for a, b in pairs:
        assert a in s, (path, a[:80])
        s = s.replace(a, b, 1)
    open(p, "w").write(s)


# 1) panel_host.rs:词汇与 outbox(放在 PanelIo 的 impl 之后、HoverSlot 之前)
edit("panel_host.rs", [
("/// 面板内一个可悬停元素的**槽位**", '''/// 面板之间"我想让另一个面板做点事"的**受限词汇**(设计文档 P1,用户 2026-10-04 裁决:面板不依赖
/// host 的总 `Message`,也不互相引用)。载荷只用通用类型(路径、布尔),不出现任何面板的业务类型;
/// 由 host 把命令翻译成对目标面板/弹窗的具体动作(`App::run_panel_command`)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PanelCommand {
    /// 在某个路径下发起搜索(`is_dir` 决定按目录递归还是只搜单文件)。
    SearchIn { path: PathBuf, is_dir: bool },
    /// 打开某个文件(绝对路径)的 git 历史。
    ShowFileHistory { path: PathBuf },
}

/// 面板向 host 提的需求——只有 host 才能做的事(系统对话框、跨面板动作)。`M` 是提需求的那个面板
/// 自己的 `Message` 类型:需要回复的需求带一个 `fn(..) -> M`,host 执行后把回复包成该面板的消息
/// 投回(包装函数由 host 在排空时给,面板不知道自己在 host 里叫什么)。
pub(crate) enum HostRequest<M> {
    /// 弹系统"选择文件夹"对话框;选中后用 `on_picked` 造出面板消息。`start` 是起始目录。
    PickDirectory {
        start: Option<PathBuf>,
        on_picked: fn(PathBuf) -> M,
    },
    /// 让 host 把一个跨面板命令派发给目标。
    Command(PanelCommand),
}

/// 面板 state 里待交给 host 的需求队列。与 `toast::Outbox`(待发提示)同一个模式:面板的 `update`
/// 签名各不相同(返回 `()`/`Option`/`Vec<Effect>`),拿不到 `App`;往自己 state 的 outbox 里 `push`,
/// `App::update` 的包装函数每条消息后统一排空执行。纯数据,可单测。
pub(crate) struct HostOutbox<M> {
    items: Vec<HostRequest<M>>,
}

impl<M> Default for HostOutbox<M> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<M> HostOutbox<M> {
    pub(crate) fn push(&mut self, request: HostRequest<M>) {
        self.items.push(request);
    }

    pub(crate) fn take(&mut self) -> Vec<HostRequest<M>> {
        std::mem::take(&mut self.items)
    }
}

/// 面板内一个可悬停元素的**槽位**'''),
("use std::future::Future;\n", "use std::future::Future;\nuse std::path::PathBuf;\n"),
])

# 2) Files:state 字段 + update 里三条臂
edit("extensions/files/state.rs", [
("    pub(crate) outbox: crate::extensions::toast::Outbox,\n    pub(crate) file_tree: Option<FileTree>,",
 "    pub(crate) outbox: crate::extensions::toast::Outbox,\n    /// 待交给 host 的需求(系统对话框、跨面板命令);`App::update` 每条消息后排空执行。\n    pub(crate) host: crate::panel_host::HostOutbox<Message>,\n    pub(crate) file_tree: Option<FileTree>,"),
])
edit("extensions/files/update.rs", [
("""        Message::MoveDirBrowse => {
            unreachable!(
                "由 main.rs Runner::dispatch 拦截处理,见 files::Message::MoveDirBrowse 文档"
            )
        }""", """        // 拖拽移动确认框"到目录"旁的浏览按钮:本模块不认识系统对话框,交给 host 弹,起始目录用当前草稿。
        Message::MoveDirBrowse => {
            let start = ws_state.move_dir_draft().map(PathBuf::from);
            ws_state.host.push(HostRequest::PickDirectory {
                start,
                on_picked: |dir| Message::MoveDirInput(dir.display().to_string()),
            });
        }"""),
("""        Message::OpenSearch(..) => {
            unreachable!("由内核拦截处理,映射成 search::Message::SearchOpen")
        }
        Message::FileHistoryOpen(_) => {
            unreachable!("由内核拦截处理,见 files::Message::FileHistoryOpen 文档")
        }""", """        // 右键菜单"搜索":先关右键菜单(否则搜索弹窗 dismiss 一关,旧菜单又冒回来),再请 host 在该路径下
        // 开搜索——目录按目录递归搜,文件只搜单文件(由 host 的 `SearchIn` 处理)。
        Message::OpenSearch(path, is_dir) => {
            app_state.close_context_menu();
            ws_state
                .host
                .push(HostRequest::Command(PanelCommand::SearchIn { path, is_dir }));
        }
        // 右键"查看此文件历史":先收起右键菜单(同 OpenSearch 的既有约定),再请 host 打开历史。
        Message::FileHistoryOpen(path) => {
            app_state.close_context_menu();
            ws_state
                .host
                .push(HostRequest::Command(PanelCommand::ShowFileHistory { path }));
        }"""),
])
p = SRC + "extensions/files/update.rs"
s = open(p).read()
i = s.index("pub fn update(")
s = s[:i] + s[i:]  # 保持原样;import 加在文件顶部第一个 `use` 之前
first_use = s.index("\nuse ")
s = s[:first_use] + "\nuse crate::panel_host::{HostRequest, PanelCommand};" + s[first_use:]
open(p, "w").write(s)

# 3) project_create:state 字段 + 两条臂
edit("extensions/project_create.rs", [
("#[derive(Default)]\npub struct State {\n    pub tab: Tab,",
 "#[derive(Default)]\npub struct State {\n    /// 待交给 host 的需求(两处\"选择根目录…\"弹系统对话框);`App::update` 每条消息后排空执行。\n    pub(crate) host: crate::panel_host::HostOutbox<Message>,\n    pub tab: Tab,"),
("""        Message::LocalRootDirPick | Message::CloneRootDirPick => {
            // 弹 rfd 文件夹选择器是内核(`Runner::dispatch`)的职责,这里
            // 收到说明路由出了问题,当 no-op 处理,不 panic。
            true
        }""", """        // 弹系统文件夹选择器是 host 的职责(本模块保持可在单测里构造):提一个 `PickDirectory` 需求,
        // host 选完后用 `on_picked` 把结果回填成 `*RootDirPicked`。
        Message::LocalRootDirPick => {
            state.host.push(HostRequest::PickDirectory {
                start: None,
                on_picked: |dir| Message::LocalRootDirPicked(dir.display().to_string()),
            });
            true
        }
        Message::CloneRootDirPick => {
            state.host.push(HostRequest::PickDirectory {
                start: None,
                on_picked: |dir| Message::CloneRootDirPicked(dir.display().to_string()),
            });
            true
        }"""),
])
p = SRC + "extensions/project_create.rs"
s = open(p).read()
first_use = s.index("\nuse ")
s = s[:first_use] + "\nuse crate::panel_host::HostRequest;" + s[first_use:]
open(p, "w").write(s)

# 4) host:排空与执行
edit("app/update.rs", [
("""        self.update_inner(message);
        // 放在包装里而不是 `match` 末尾:`update_inner` 的许多分支会提前 `return`,
        // 放在 `match` 里会漏排空。
        self.drain_outboxes();
    }
""", """        self.update_inner(message);
        // 放在包装里而不是 `match` 末尾:`update_inner` 的许多分支会提前 `return`,
        // 放在 `match` 里会漏排空。
        self.drain_outboxes();
        self.drain_host_requests();
    }

    /// 排空各面板 state 的 `HostOutbox` 并执行(系统对话框、跨面板命令)。先把所有需求收出来再执行:
    /// 执行过程会再进 `self.update`(回投面板消息),不能在持有 workspace 借用时做。
    fn drain_host_requests(&mut self) {
        let mut files_requests = Vec::new();
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                files_requests.extend(ws.files.host.take());
            }
        }
        let create_requests = self
            .project_create
            .as_mut()
            .map(|s| s.host.take())
            .unwrap_or_default();
        self.run_host_requests(files_requests, Message::Files);
        self.run_host_requests(create_requests, Message::ProjectCreate);
    }

    /// 执行一批面板需求;`wrap` 把提需求的面板自己的 `Message` 包成宿主 `Message`。
    fn run_host_requests<M: Send + 'static>(
        &mut self,
        requests: Vec<crate::panel_host::HostRequest<M>>,
        wrap: fn(M) -> Message,
    ) {
        use crate::panel_host::HostRequest;
        for request in requests {
            match request {
                HostRequest::PickDirectory { start, on_picked } => {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(start) = start {
                        dialog = dialog.set_directory(start);
                    }
                    if let Some(dir) = dialog.pick_folder() {
                        self.update(wrap(on_picked(dir)));
                    }
                }
                HostRequest::Command(command) => self.run_panel_command(command),
            }
        }
    }

    /// 跨面板命令的唯一派发点:面板只说"想让别人做什么",由这里决定谁来做。
    fn run_panel_command(&mut self, command: crate::panel_host::PanelCommand) {
        use crate::panel_host::PanelCommand;
        match command {
            PanelCommand::SearchIn { path, is_dir } => {
                let scope = if is_dir {
                    search::Scope::Dir(path)
                } else {
                    search::Scope::File(path)
                };
                self.update(Message::Search(search::Message::SearchOpen(scope)));
            }
            PanelCommand::ShowFileHistory { path } => self.show_file_history(path),
        }
    }

    /// 解析出仓库相对路径、组出 `FileHistoryTarget`、异步跑一次 `build()`。
    fn show_file_history(&mut self, path: PathBuf) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = PathBuf::from(&project.path);
        let Ok(file_path) = path.strip_prefix(&repo_path).map(|p| p.to_path_buf()) else {
            return;
        };
        let target = file_history::FileHistoryTarget {
            project_id,
            repo_path: repo_path.clone(),
            file_path: file_path.clone(),
        };
        self.file_history = Some(file_history::State::new(target));
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let repo_path2 = repo_path.clone();
            let file_path2 = file_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                file_history::build(&repo_path2, &file_path2, file_history::DEFAULT_MAX_COUNT)
            })
            .await
            .unwrap_or_else(|e| Err(format!("加载失败: {e}")));
            let _ = proxy.send_event(Message::FileHistory(file_history::Message::SnapshotLoaded(
                repo_path, file_path, result,
            )));
        });
    }
"""),
])
# 删 host 的两条 Files 拦截臂
p = SRC + "app/update.rs"
s = open(p).read()
i = s.index("            Message::Files(files::Message::OpenSearch(path, is_dir)) => {")
j = s.index("            Message::Files(files::Message::FileHistoryRollbackPrevious(path)) => {")
s = s[:i] + s[j:]
open(p, "w").write(s)

# 5) 窗口层:删三条系统对话框拦截臂
p = SRC + "platform/window_events.rs"
s = open(p).read()
i = s.index("            // \"创建项目\"弹窗里两处\"选择根目录…\"")
j = s.index("            Message::ProjectLinkPick(target) => {")
s = s[:i] + s[j:]
open(p, "w").write(s)

# 6) 订正三处变体文档(它们不再是"内核拦截,不进 update")
edit("extensions/files/state.rs", [
("""    /// 右键"查看此文件历史":内核拦截,不进 `update`——由内核解析出仓库
    /// 相对路径、组出 `file_history::FileHistoryTarget`,写入
    /// `App::file_history` 并异步跑 `file_history::build`(见""",
 """    /// 右键"查看此文件历史":`files::update` 先收起右键菜单,再往 `ws_state.host` 提一个
    /// `PanelCommand::ShowFileHistory`;host 的 `run_panel_command` 解析出仓库
    /// 相对路径、组出 `file_history::FileHistoryTarget`,写入
    /// `App::file_history` 并异步跑 `file_history::build`(见"""),
("""    /// 右键菜单"搜索":内核拦截,不进 `update`——由内核映射成
    /// `search::Message::SearchOpen` 打开文件树右键作用域的搜索弹窗。""",
 """    /// 右键菜单"搜索":`files::update` 先收起右键菜单,再往 `ws_state.host` 提一个
    /// `PanelCommand::SearchIn`;host 把它映射成 `search::Message::SearchOpen`
    /// 打开文件树右键作用域的搜索弹窗。"""),
("""    /// 拖拽移动确认框"到目录"字段旁边的"..."浏览按钮:要弹原生目录选择器
    /// (`rfd::FileDialog`),`files::update()`(纯状态转换,拿不到原生
    /// 对话框能力)处理不了——由 main.rs 的 `Runner::dispatch` 拦截(同
    /// `Message::ProjectTabPickFolder` 的既有套路,注意这**不是**
    /// `App::update` 那层拦截,是更外层 main.rs 自己的 match),选完后转发
    /// 一条 `MoveDirInput` 回填草稿。""",
 """    /// 拖拽移动确认框"到目录"字段旁边的"..."浏览按钮:要弹原生目录选择器,
    /// `files::update()`(纯状态转换,拿不到原生对话框能力)自己做不了——它往
    /// `ws_state.host` 提一个 `HostRequest::PickDirectory`(起始目录 = 当前草稿),
    /// host 弹完对话框后用 `on_picked` 转发一条 `MoveDirInput` 回填草稿。"""),
])
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && export PYTHONDONTWRITEBYTECODE=1
python3 $SCRATCH/h5_impl.py && cargo fmt -p dozer-app && git status --short
```
Expected: `git status --short` 是 `app/update.rs`、`extensions/files/{mod,state,update}.rs`、`extensions/project_create.rs`、`panel_host.rs`、`platform/window_events.rs`(和 `Cargo.lock`,马上还原)。脚本里每个替换点都先 `assert` 原文存在;若报 `AssertionError`,说明 `main` 上这几处已被改动,按报错点对照 `git diff main` 手工对齐后再继续,**不要**改断言。

- [ ] **Step 5: 编译、全量测试、clippy**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A9 | head -40
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
```
Expected: build 无输出;测试通过数 = N + 6,失败只有已知项;`clippy: no new diagnostics`。

- [ ] **Step 6: 逐条审阅 host 侧 diff(没有 `App` 夹具可测)**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && git diff -- crates/dozer-app/src/app/update.rs crates/dozer-app/src/platform/window_events.rs`
Expected(逐条对照,对应 Review Focus 1–5):
1. `update()` 末尾多一行 `self.drain_host_requests();`,在 `drain_outboxes()` 之后;
2. `drain_host_requests` 先把 `files`(遍历已加载工作区)与 `project_create` 的需求**全部收出来**,再调用 `run_host_requests`(此时不再持有 workspace 借用);
3. `run_host_requests::PickDirectory`:`rfd::FileDialog::new()`,有 `start` 才 `set_directory`,选中则 `self.update(wrap(on_picked(dir)))`——对照窗口层被删的三条臂,都是"`pick_folder()` 选中后回填一条面板消息";
4. `run_panel_command::SearchIn`:`Scope::Dir`/`Scope::File` 的选择、`SearchOpen` 的派发与旧 `OpenSearch` 臂一致;`ShowFileHistory` → `show_file_history`,其函数体与被删的旧 `FileHistoryOpen` 臂逐行相同(**唯一差别:旧臂开头的 `self.files.close_context_menu()` 已前移到 `files::update`**);
5. 被删的 `App::update` 臂只有 `Message::Files(OpenSearch)` 与 `Message::Files(FileHistoryOpen)` 两条;被删的窗口层臂只有 `ProjectCreate(LocalRootDirPick)`、`ProjectCreate(CloneRootDirPick)`、`Files(MoveDirBrowse)` 三条(`ProjectTabPickFolder`、`ProjectLinkPick` 等**保留**)。

- [ ] **Step 7: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && export PYTHONDONTWRITEBYTECODE=1
git add crates
cat > $SCRATCH/h5_mutate.py <<'EOF'
import sys
w = sys.argv[1]
if w == "1":   # MoveDirBrowse 不再带草稿作为起始目录
    p, a, b = "crates/dozer-app/src/extensions/files/update.rs", "let start = ws_state.move_dir_draft().map(PathBuf::from);", "let start: Option<PathBuf> = None;"
elif w == "2":   # OpenSearch 不再先关右键菜单
    p, a = "crates/dozer-app/src/extensions/files/update.rs", "        Message::OpenSearch(path, is_dir) => {\n            app_state.close_context_menu();\n"
    b = "        Message::OpenSearch(path, is_dir) => {\n"
elif w == "3":   # FileHistoryOpen 误提成搜索命令
    p, a = "crates/dozer-app/src/extensions/files/update.rs", "                .push(HostRequest::Command(PanelCommand::ShowFileHistory { path }));"
    b = "                .push(HostRequest::Command(PanelCommand::SearchIn { path, is_dir: false }));"
elif w == "4":   # "克隆"的选目录回填成了"本地"的消息
    p, a = "crates/dozer-app/src/extensions/project_create.rs", "on_picked: |dir| Message::CloneRootDirPicked(dir.display().to_string()),"
    b = "on_picked: |dir| Message::LocalRootDirPicked(dir.display().to_string()),"
s = open(p).read()
assert a in s, w
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3 4; do echo "M$m:"; python3 $SCRATCH/h5_mutate.py $m && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app extensions 2>&1 | grep -E "^error|FAILED$|test result" | head -4; git checkout -- crates Cargo.lock; done
git diff --stat | wc -l; git status --short
```
Expected: 四次变异各自让**对应**的测试失败(除了永远失败的 `delete_confirm`):M1:`move_dir_browse_asks_the_host_for_a_directory_starting_at_the_draft`;M2:`open_search_closes_the_context_menu_and_asks_the_host_to_search_there`;M3:`file_history_open_closes_the_context_menu_and_asks_the_host_to_show_history`;M4:`root_dir_pick_buttons_ask_the_host_for_a_directory_and_map_the_reply_back`;每次还原后 `git diff --stat | wc -l` 为 `0`(暂存的实现版本被恢复)。

- [ ] **Step 8: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5
git status --short
git add crates
git diff --cached --stat | tail -9
git commit -m "refactor(panels): HostOutbox/PanelCommand — panels ask the host for dialogs and cross-panel actions

Files (move-dir browse, search-here, file history) and project_create (two root-dir pickers)
no longer rely on host/window-layer interception: they push HostRequests into their own state's
outbox and the host drains and executes them after every message. PanelCommand is the restricted
cross-panel vocabulary (SearchIn, ShowFileHistory). Removes 3 window-layer arms and 2 App::update
Files arms.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `git diff --cached --stat` 里只有 7 个 `crates/` 文件(没有 `Cargo.lock`、`scripts/`、`docs/`)。

---

### Task 2: 门禁基线与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1 的提交。
- Produces:门禁基线 `R-HOST-PANEL-ARMS` 95(门禁规则本身不变,H4 已建)。

- [ ] **Step 1: 门禁应当"下降"并压基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && export PYTHONDONTWRITEBYTECODE=1 && python3 scripts/audit/check_panel_boundary.py; python3 scripts/audit/check_panel_boundary.py --update; git diff scripts/audit/panel-boundary.baseline.json | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)"; python3 scripts/audit/check_panel_boundary.py`
Expected: 第一次检查对着旧基线 `ok`(95 低于 97 是"下降",允许);`--update` 后 diff 只有 `-  "R-HOST-PANEL-ARMS": 97,` 与 `+  "R-HOST-PANEL-ARMS": 95,`;最后一次 `ok (122 refs in 5 files)`。若不是 95,说明 `main` 上这个文件在草稿之后有改动——以实测为准,回填文档时用实测值。

- [ ] **Step 2: 回填文档**

用带引号的 heredoc 创建 `$SCRATCH/h5_docs.py` 并运行(提交短 id 经环境变量传入):

```python
import os
H5 = os.environ["H5"]
sp = "docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md"
s = open(sp).read()
a = "### 3.3 切入钩子"
assert a in s
s = s.replace(a, "(H5 实现取舍:需求经面板 state 里的 `HostOutbox` 返回,而不是 `update` 的返回值——沿用 `toast::Outbox` 的现有模式,各面板 `update` 的签名不变;本刀词汇只落地 `PickDirectory` 与 `Command(PanelCommand)`,`ShowPanel` 等有使用者时再加。)\n\n" + a, 1)
lines = s.split("\n")
hit = False
for i, l in enumerate(lines):
    if l.startswith("| **H5** |"):
        lines[i] = l[:-1].rstrip() + " **部分完成(H5a,`bytehost-h5`):`" + H5 + "`**——3 个选目录对话框(project_create 两处 + Files 的移动到目录)与 Files 的搜索/查看历史两条跨面板臂;`PanelCommand` 目前只有 `SearchIn`/`ShowFileHistory`;余下见 E3-010/E3-011 |"
        hit = True
assert hit
open(sp, "w").write("\n".join(lines))
tp = "docs/dozer-v2/bytehost-H0/E3-registry.tsv"
rows = open(tp).read().split("\n")
hit = False
for i, r in enumerate(rows):
    if r.startswith("E3-011\t"):
        f = r.split("\t")
        f[4] = "H5a 已迁:project_create 两处与 Files 移动到目录的 rfd 对话框、Files 的 OpenSearch/FileHistoryOpen(经 HostOutbox/PanelCommand);仍剩 ProjectLinkPick(platform::picker,cfg 分支)、Files 的 CopyPath(剪贴板句柄)与 FileHistoryRollbackPrevious(结果回投 Files 消息)、Search::Pick(窗口层焦点意图)、Browser::Nav(原生 webview 句柄)"
        f[5] = "HostRequest 词汇扩展(ShowPanel/带回复的命令)与窗口层上下文(焦点意图/剪贴板/webview 池)可注入(H5b/H8)"
        rows[i] = "\t".join(f)
        hit = True
assert hit
open(tp, "w").write("\n".join(rows))
rp = "docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md"
s = open(rp).read().rstrip("\n")
s += "\n\n10. **bytehost H5a 已完成（2026-10-04）：** 引入 `HostOutbox`/`HostRequest`/`PanelCommand`（P1 已定为受限命令类型）；`project_create` 的两处“选择根目录…”、`Files` 的“移动到目录…”与右键“搜索”“查看此文件历史”不再靠 host/窗口层拦截，改为面板往自己 state 的 outbox 提需求、host 每条消息后统一执行；窗口层少 3 条拦截臂，`App::update` 少 2 条 `Files` 专属臂，门禁 `R-HOST-PANEL-ARMS` 基线 97 → 95。余下的窗口层拦截（剪贴板、焦点意图、webview 句柄、`ProjectLinkPick`）需要窗口层上下文可注入，留给后续切片。\n"
open(rp, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5
H5=$(git log --format=%h -1 --grep="HostOutbox/PanelCommand") python3 $SCRATCH/h5_docs.py
awk -F'\t' 'NR>1 && (NF!=6 || $6=="")' docs/dozer-v2/bytehost-H0/E3-registry.tsv | wc -l
git diff -- docs | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | cut -c1-170
```
Expected: `E3-registry.tsv` 校验输出 `0`;`git diff` 里只有设计文档的 H5 行与 §3.2 补注、E3-010/E3-011 两行、要求文档第 10 条,**反引号内容完整**(若出现反引号被吞掉的痕迹,说明用了不带引号的 heredoc——`git checkout -- docs` 后重做)。

- [ ] **Step 3: 最终校验并提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h5 && export PYTHONDONTWRITEBYTECODE=1
python3 scripts/audit/test_edges.py 2>&1 | tail -1; python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1; python3 scripts/audit/check_panel_boundary.py
git status --short
git add scripts/audit/panel-boundary.baseline.json docs
git diff --cached --stat | tail -6
git commit -m "chore(audit): ratchet host-side panel arms to 95; backfill H5 docs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 两组脚本测试 `OK`、门禁 `ok`;`git diff --cached --stat` 里只有 `scripts/audit/` 与 `docs/`(没有 `crates/`、`Cargo.lock`)。

---

## Self-Review

**1. 覆盖:** 设计文档 H5 的"迁走 `window_events.rs` 的 rfd 对话框拦截与 `Files` 的跨面板消息臂"——对话框 3/5 条迁走(余下 `ProjectTabPickFolder`/`ProjectLinkPick` 的理由见 Review Focus 6),跨面板消息臂 2/3 条迁走(`RollbackPrevious` 留后)。**范围比设计文档的"约 −300 行"小**:实测窗口层对面板的拦截大头是拖拽落点转发、剪贴板、原生 webview 句柄操作,它们需要的不是 `PickDirectory` 这类词汇,而是窗口层状态(焦点意图、剪贴板、webview 池)进入某种可注入的上下文——那是 H5b/H8 的问题,本刀不假装解决。

**2. 占位符扫描:** 测试、实现脚本、文档脚本均为草稿里跑通的完整版本。

**3. 一致性:** `HostRequest::PickDirectory { start, on_picked }`、`PanelCommand::{SearchIn, ShowFileHistory}`、`HostOutbox::{push, take}` 的名字与形状在测试、实现脚本、Interfaces、文档里一致。

**4. Review Focus:** 7 条各有归属(1、2→Task 1 Step 6 审阅,2 另有测试;3→Step 6;4→测试 + Step 6;5→Step 6;6→范围说明;7→过渡说明)。
