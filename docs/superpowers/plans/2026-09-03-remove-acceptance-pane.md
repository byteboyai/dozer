# 删除验收面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把验收(Acceptance)面板从 Dozer 里完全删除——dozer-app 的 UI/状态/路由、dozerd 的存储/协议、dozer-client 的客户端方法、用户文档——不迁移任何具体能力到 Todo 面板，Todo 面板本身不做任何改动。

**Architecture:** `PanelKind::Acceptance` 是这次删除的"keystone"：它被 `dozer-app` 里六个文件的非穷尽 `match` 消费，且被序列化进两份磁盘配置文件。删除顺序必须先摘掉这个枚举变体触发编译器报错，跟着编译器逐个修完全部消费点（这比手动 grep 排查更可靠——`match` 非穷尽性检查会精确列出每一处遗漏），再依次往下删除 `extensions::acceptance`/`goal.rs`/`delivery.rs` 的验收专属部分、dozer-client 的方法、dozer-core 的协议变体、dozerd 的存储与 handler，最后是全部相关文档。dozerd::server::serve() 签名改动会连带影响 6 个测试文件（dozer-client/dozer-mcp/dozerd 三个 crate 下）的测试夹具，这部分同样靠编译器报错定位，不手动枚举每个调用点。

**Tech Stack:** Rust workspace（dozer-app: iced 0.14 GUI；dozerd: tokio + rusqlite；dozer-core: serde 协议；dozer-client: tokio UDS 客户端）。

**Spec:** `docs/superpowers/specs/2026-09-03-remove-acceptance-pane-design.md`

## Global Constraints

- 不迁移任何验收专属能力到 Todo 面板（checklist/diff 预览/通过-打回/git ref）——spec 已确认，Todo 面板代码本次不改一行。
- dozerd 后端（`acceptances` 表、协议、client 方法）彻底删除，不保留哑接口。
- 不写 sqlite `DROP TABLE` 迁移——已存量用户盘上的 `acceptances` 表原样留着，代码不再引用即可。
- 不删除历史设计/计划文档（`docs/superpowers/specs/2026-08-08-acceptance-pane-design.md` 等 P1c-P1l 系列）。
- 文档改写只删失效的具体机制描述（图标/checklist/diff/按钮/链接），不重写产品定位或发明新的 Todo 驱动流程叙事——已跟用户确认。
- 本计划必须在独立的 git worktree/分支上执行，不得直接在 `main` 上开发；分支名建议 `remove-acceptance-pane`。分派给 subagent 时必须显式带上完整 worktree 绝对路径前缀，防止 agent 漂移回 `main`（历史上出现过多次此类事故）。
- 每个任务结束前必须跑 `cargo build --workspace` 且不报错（deletion 类改动的主要正确性信号就是编译器的非穷尽 match/多余实参报错）。

---

### Task 1: dozer-app — 删除 `PanelKind::Acceptance` 及其全部消费侧接线

这是最大也是最关键的一个任务：`PanelKind::Acceptance` 被 app.rs/main.rs/workspace.rs/rail.rs/webview_geometry.rs/panel_layouts.rs 六个文件消费，且这次不删除 `extensions::acceptance`/`goal.rs`/`delivery.rs` 本身（那是 Task 2/3），只摘掉指向它们的接线。

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/rail.rs`
- Modify: `crates/dozer-app/src/webview_geometry.rs`
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/panel_layouts.rs`（仅测试夹具）

**Interfaces:**
- 本任务结束后：`extensions::acceptance`/`goal` 模块仍存在但不再被 app.rs/workspace.rs 引用；`delivery.rs` 的验收专属函数仍存在但不再被 app.rs 调用（Task 2/3 处理）。
- `PanelKind` 枚举从 11 个变体减到 10 个（`Files/GitLog/Todo/Project/Database/Ssh/Web/Agent/Conversations/Usage`）。

- [ ] **Step 1: 删除 `PanelKind::Acceptance` 枚举变体**

`crates/dozer-app/src/app.rs:109-121`，删除 `Acceptance,` 一行：

```rust
pub enum PanelKind {
    Files,
    GitLog,
    Todo,
    Project,
    Database,
    Ssh,
    Web,
    Agent,
    Conversations,
    Usage,
}
```

同时 `crates/dozer-app/src/app.rs:127-138` 的 `default_side()`：

```rust
    pub fn default_side(self) -> Side {
        match self {
            Self::Files
            | Self::GitLog
            | Self::Todo
            | Self::Project
            | Self::Database
            | Self::Ssh
            | Self::Web => Side::Left,
            Self::Agent | Self::Conversations | Self::Usage => Side::Right,
        }
    }
```

- [ ] **Step 2: 跑一次编译，收集全部报错位置**

```bash
cargo build -p dozer-app 2>&1 | grep -E "^error" -A 3
```

预期会在下面 Step 3-9 列出的每个位置报"no variant named `Acceptance`"或"non-exhaustive match"。按报错逐个修（下面已经把已知位置和修法列全，报错列表应与之一一对应；如果编译器报出下面没列到的新位置，同样按"删掉这个 match 分支/这行引用"的原则处理，不要用 `_ =>` 通配符掩盖，除非该 match 本来就有通配符分支）。

- [ ] **Step 3: `app.rs` — `pair_split_ratio`/`with_pair_split_ratio`**

`crates/dozer-app/src/app.rs:903-917`（`pair_split_ratio`），删除：
```rust
        PanelKind::Acceptance => None,
```
且把函数前的文档注释里"`None` 表示这个面板是单栏(Acceptance)，没有分割比例"改成"`None` 表示这个面板是单栏，没有分割比例"（去掉"(Acceptance)"）。

`crates/dozer-app/src/app.rs:920-964`（`with_pair_split_ratio`），删除：
```rust
        PanelKind::Acceptance => dims,
```

- [ ] **Step 4: `app.rs` — `RightPairSplit` 拖拽换算**

`crates/dozer-app/src/app.rs:1221-1238`，原：
```rust
            match kind {
                PanelKind::Agent => PanelDims {
                    agent_split: ratio,
                    ..state.dims
                },
                PanelKind::Conversations => PanelDims {
                    conversations_split: ratio,
                    ..state.dims
                },
                // 用量统计是单栏（不分割）,没有自己的 split 权重。
                PanelKind::Usage => state.dims,
                // 验收面板同用量统计是单栏,不分割。
                PanelKind::Acceptance => state.dims,
                _ => unreachable!(
                    "RightPairSplit 只会在 state.right_view 是 Agent/Conversations/\
                     Usage/Acceptance 之一时出现——Stage 1 遗留的兜底,这里维持"
                ),
            }
```
改为：
```rust
            match kind {
                PanelKind::Agent => PanelDims {
                    agent_split: ratio,
                    ..state.dims
                },
                PanelKind::Conversations => PanelDims {
                    conversations_split: ratio,
                    ..state.dims
                },
                // 用量统计是单栏（不分割）,没有自己的 split 权重。
                PanelKind::Usage => state.dims,
                _ => unreachable!(
                    "RightPairSplit 只会在 state.right_view 是 Agent/Conversations/\
                     Usage 之一时出现——Stage 1 遗留的兜底,这里维持"
                ),
            }
```

- [ ] **Step 5: `app.rs` — `Message::Acceptance`/`DeliveryChecked` 变体与路由**

`crates/dozer-app/src/app.rs:1514-1515` 删除：
```rust
    /// TurnEnded 触发的交付检测结果（tab_id, 是否有待验收交付）。
    DeliveryChecked(ProjectId, usize, bool),
```

`crates/dozer-app/src/app.rs:1529-1531` 删除：
```rust
    /// 验收面板的全部消息,内核只转发不解读——见
    /// `extensions::acceptance::Message`。
    Acceptance(acceptance::Message),
```

`crates/dozer-app/src/app.rs:4356-4358`（`Message::DeliveryChecked` 路由）删除：
```rust
            Message::DeliveryChecked(project_id, tab_id, pending) => {
                self.delivery_checked(project_id, tab_id, pending)
            }
```

`crates/dozer-app/src/app.rs:4378-4401`（`Message::Acceptance(...)` 全部路由分支）整段删除：
```rust
            Message::Acceptance(acceptance::Message::Open(tab_id)) => self.acceptance_open(tab_id),
            Message::Acceptance(acceptance::Message::Reject) => self.acceptance_reject(),
            Message::Acceptance(
                msg @ (acceptance::Message::Loaded(project_id, ..)
                | acceptance::Message::DiffLoaded(project_id, ..)
                | acceptance::Message::Done(project_id, ..)),
            ) => self.acceptance_result(project_id, msg),
            Message::Acceptance(acceptance::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Acceptance(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    let client = io.client.clone();
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Acceptance(m));
                    };
                    acceptance::update(&mut ws.acceptance, msg, project_id, &client, &handle, emit);
                });
            }
```

`crates/dozer-app/src/app.rs:5310-5319`（`project::Message` 的 or-pattern），删除其中一行：
```rust
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)),
            ) => {
```
（删掉的是原来夹在 `GitRefreshed` 和 `NameRenamed` 之间的 `| project::Message::AcceptanceCountLoaded(project_id, ..)` 一行；`project::Message::AcceptanceCountLoaded` 变体本身在本任务 Step 8 随 `project.rs` 一起删除。）

- [ ] **Step 6: `app.rs` — `acceptance_open`/`acceptance_reject`/`acceptance_result`/`comment_focused`/`set_comment_focused` 方法**

`crates/dozer-app/src/app.rs:6238-6302` 整段删除（`acceptance_open`/`acceptance_reject`/`acceptance_result` 三个方法）。

`crates/dozer-app/src/app.rs:6806-6814`（`RightIconSelect`/面板切入刷新的 match），删除：
```rust
                PanelKind::Acceptance => {
                    let tab_id = self
                        .active_workspace()
                        .and_then(|ws| ws.tabs.get(ws.active))
                        .map(|t| t.tab_id);
                    if let Some(tab_id) = tab_id {
                        self.update(Message::Acceptance(acceptance::Message::Open(tab_id)));
                    }
                }
```

`crates/dozer-app/src/app.rs:8807-8810`（`view` 渲染 match 最后一个分支），删除：
```rust
        PanelKind::Acceptance => {
            acceptance::view(&ws.acceptance, Length::Fill, zone_pane_border(zone, ac))
                .map(Message::Acceptance)
        }
```
（这是这个大 match 的最后一个分支，删除后确认上一个分支 `PanelKind::Usage => { ... }` 后紧跟 `}` 收尾即可，语法上无需补逗号。）

`crates/dozer-app/src/app.rs:3073-3085` 整段删除（`comment_focused`/`set_comment_focused` 两个方法）：
```rust
    /// 验收意见框是否持有 iced 真实焦点(main.rs 原生放行闸门用)。
    pub fn comment_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.comment_focused())
    }

    /// 每帧渲染循环读走 `CaptureCommentFocus` 查到的真实焦点态后写进当前
    /// 工作区(main.rs 键盘路由随后读 `comment_focused` 消费)。
    pub fn set_comment_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.acceptance.set_comment_focused(focused);
        }
    }
```

删除 `crates/dozer-app/src/app.rs:16` 的 `use crate::extensions::acceptance;`（本文件已无其他 `acceptance::` 引用）。

- [ ] **Step 7: `app.rs` — `agent_state_changed`/`delivery_checked` 重构**

这一步不是纯删除：`delivery_checked` 方法里"回合结束刷新项目 git 状态/磁盘占用,文件树装饰随之更新（P1h）"这个副作用不是验收专属的，必须原地保留，只是改成跟"回合结束刷新会话列表"一样无条件触发（`agent_state_changed` 里已经有一次类似的历史修复,同样的理由这次再补一次）。

`crates/dozer-app/src/app.rs` 里 `fn agent_state_changed(...)` 方法（约 7045-7153 行）的 `self.with_project(project_id, |ws, io| { ... })` 闭包体，原内容：

```rust
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            let mut card_refresh_args: Option<(Option<String>, PathBuf)> = None;
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                card_refresh_args = Some((tab.transcript_path.clone(), tab.effective_cwd()));
                if state == AgentState::TurnEnded {
                    // git 检测不许在 UI 线程跑：丢 tokio,结果经 proxy 回来
                    let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                    let last_turn = tab.last_turn_head.clone();
                    let proxy = io.proxy.clone();
                    tracing::info!(tab_id, cwd = %cwd.display(), "回合结束,开始交付检测");
                    io.handle.spawn(async move {
                        let pending = tokio::task::spawn_blocking(move || {
                            let Some(repo) = delivery::repo_root(&cwd) else {
                                tracing::info!(cwd = %cwd.display(), "非 git 仓库,不参与闭环");
                                return None;
                            };
                            let dirty = delivery::is_dirty(&repo);
                            let head = delivery::head_commit(&repo);
                            let accepted = delivery::last_accepted(&repo).map(|(_, c)| c);
                            let pending = delivery::delivery_pending(
                                dirty,
                                head.as_deref(),
                                accepted.as_deref(),
                                last_turn.as_deref(),
                            );
                            tracing::info!(
                                repo = %repo.display(),
                                dirty,
                                has_accepted = accepted.is_some(),
                                pending,
                                "交付检测完成"
                            );
                            Some(pending)
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some(pending) = pending {
                            let _ = proxy
                                .send_event(Message::DeliveryChecked(project_id, tab_id, pending));
                        }
                    });
                }
            }
            if let Some((transcript_path, cwd)) = card_refresh_args {
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    transcript_path,
                    cwd,
                    active_repo.clone(),
                );
            }
            // 回合结束后刷新会话列表(transcript 增长/新增；P1j)。此前这个
            // 刷新只挂在 `delivery_checked`(交付检测异步任务成功回来才发的
            // 消息)上——非 git 仓库、`repo_root` 解析失败、`spawn_blocking`
            // 出错都会让那条异步任务直接 `return None` 而不发
            // `DeliveryChecked`,会话列表就此静默再也不刷新,且没有任何
            // 手动刷新入口能补救(用户反馈"看不到新会话",根因就是这个耦合)。
            // 会话列表新鲜度跟交付检测是否成功完全是两件事,不该耦合在一起,
            // 这里改成回合结束就无条件刷新,不等交付检测。
            if state == AgentState::TurnEnded {
                ws.spawn_conversations_refresh(io);
            }
            // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
            if state == AgentState::TurnEnded
                && let Some(rv) = &ws.review
                && review_should_refresh_on_turn(&rv.source, tab_id)
                && let Some((path, tab_agent)) = ws
                    .tabs
                    .iter()
                    .find(|t| t.tab_id == tab_id)
                    .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
            {
                ws.spawn_review_load(
                    io,
                    ReviewSource::Session(tab_id),
                    path,
                    tab_agent,
                    -1,
                    10_000,
                );
            }
```

替换为：

```rust
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            let mut card_refresh_args: Option<(Option<String>, PathBuf)> = None;
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                card_refresh_args = Some((tab.transcript_path.clone(), tab.effective_cwd()));
            }
            if let Some((transcript_path, cwd)) = card_refresh_args {
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    transcript_path,
                    cwd,
                    active_repo.clone(),
                );
            }
            // 回合结束后刷新会话列表(transcript 增长/新增；P1j)与项目 git/
            // 磁盘占用状态(文件树装饰随之更新；P1h)。这两项刷新原先分别挂在
            // 会话列表自己的耦合链路、以及已删除的验收检测异步回调
            // (`delivery_checked`)上——验收闭环删除后,后者连带的刷新触发点
            // 也没了,这里改成回合结束就无条件触发,不再依赖任何验收检测结果
            // (前半"会话列表"这条 P1j 当年就已经这样修过一次,这次是把后半
            // "git/磁盘占用"也补齐同样的处理)。
            if state == AgentState::TurnEnded {
                ws.spawn_conversations_refresh(io);
                if let Some(project) = &ws.project {
                    let repo_path = PathBuf::from(&project.path);
                    spawn_project_git_refresh(project_id, repo_path.clone(), io);
                    spawn_disk_usage_refresh(project_id, repo_path, io);
                }
            }
            // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
            if state == AgentState::TurnEnded
                && let Some(rv) = &ws.review
                && review_should_refresh_on_turn(&rv.source, tab_id)
                && let Some((path, tab_agent)) = ws
                    .tabs
                    .iter()
                    .find(|t| t.tab_id == tab_id)
                    .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
            {
                ws.spawn_review_load(
                    io,
                    ReviewSource::Session(tab_id),
                    path,
                    tab_agent,
                    -1,
                    10_000,
                );
            }
```

紧接着，删除整个 `fn delivery_checked(&mut self, project_id: ProjectId, tab_id: usize, pending: bool) { ... }` 方法（原内容如下，整段删除）：

```rust
    fn delivery_checked(&mut self, project_id: ProjectId, tab_id: usize, pending: bool) {
        self.with_project(project_id, |ws, io| {
            let active_id = ws.tabs.get(ws.active).map(|t| t.tab_id);
            let is_active = active_id == Some(tab_id);
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            tracing::info!(
                tab_id,
                pending,
                is_active,
                "交付检测结果落地(pending 写入该 tab;仅当前激活 tab 显示横幅)"
            );
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.delivery_pending = pending;
                // 记录本回合 HEAD 供下回合比对（同步读一次可容忍:仅 rev-parse）
                let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                if let Some(repo) = delivery::repo_root(&cwd) {
                    tab.last_turn_head = delivery::head_commit(&repo);
                }
            }
            // 回合结束后刷新项目 git 状态,文件树装饰随之更新（P1h）。
            if let Some(project) = &ws.project {
                let repo_path = PathBuf::from(&project.path);
                spawn_project_git_refresh(project_id, repo_path.clone(), io);
                spawn_disk_usage_refresh(project_id, repo_path, io);
            }
            // 回合结束后刷新会话列表已经在 `agent_state_changed` 里 `TurnEnded` 时无条件
            // 触发过一次(不再等这条交付检测异步消息回来才刷),这里不再重复。
        });
    }
```

- [ ] **Step 8: `extensions/project.rs` — `project_acceptance_count`/`AcceptanceCountLoaded`**

`crates/dozer-app/src/extensions/project.rs:80` 删除字段：
```rust
    project_acceptance_count: Option<u64>,
```

`crates/dozer-app/src/extensions/project.rs:219-226` 文档注释与变体，原：
```rust
/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`remote_url`)、
/// 验收次数、daemon 改名结果。`GitRefreshed`/`AcceptanceCountLoaded`/
/// `NameRenamed` 由内核分发,带 `project_id`,走 `with_project`;其余是用户
/// 交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<String>),
    AcceptanceCountLoaded(i64, Option<u64>),
```
改为：
```rust
/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`remote_url`)、
/// daemon 改名结果。`GitRefreshed`/`NameRenamed` 由内核分发,带
/// `project_id`,走 `with_project`;其余是用户交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<String>),
```

`crates/dozer-app/src/extensions/project.rs:337-339` 删除：
```rust
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
```

`crates/dozer-app/src/extensions/project.rs:317-319` 文档注释，把"(不像 Files/Acceptance)"改成"(不像 Files)"：
```rust
/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files);改名需要 daemon 往返,走
/// `handle`/`emit`。
```

`crates/dozer-app/src/extensions/project.rs:814-820` 删除：
```rust
    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        content = content.push(
            text(format!("{n} 次验收"))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().gold),
        );
    }
```

`crates/dozer-app/src/extensions/project.rs:1549-1565` 整段删除测试：
```rust
    #[test]
    fn acceptance_count_loaded_sets_field() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::AcceptanceCountLoaded(1, Some(3)),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.project_acceptance_count, Some(3));
    }
```

- [ ] **Step 9: `workspace.rs` — `acceptance`/`delivery_pending`/`last_turn_head`/`spawn_acceptance_count_refresh`/`comment_focused`/`effective_project_repo`**

删除 `crates/dozer-app/src/workspace.rs:39` 的 `use crate::extensions::acceptance;`。

删除字段 `pub(crate) acceptance: acceptance::WorkspaceState,`（约 392-393 行，含其上方文档注释一行）。

删除字段初始化 `acceptance: acceptance::WorkspaceState::default(),`（约 661 行）。

删除 `self.acceptance = acceptance::WorkspaceState::default();`（约 1212 行，在 `close_all_tabs_for_switch` 或类似的项目切换重置逻辑里）。

删除 `ws.spawn_acceptance_count_refresh(io);` 的两处调用（约 595 行、1266 行）。

删除整个 `spawn_acceptance_count_refresh` 方法（约 1029-1052 行）：
```rust
    /// 异步取当前项目验收次数 → AcceptanceCountLoaded（项目卡副行）。
    /// 查询键走 `acceptance_query_repo`（= 落库侧 `delivery::repo_root`），
    /// 而非原始 `p.path`，否则子目录/符号链接路径撞不到库、副行静默空白。
    pub(crate) fn spawn_acceptance_count_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else { return };
        let project_id = p.id;
        let project_path = p.path.clone();
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            // repo_root 是阻塞 git 调用，隔离到 spawn_blocking。
            let repo = tokio::task::spawn_blocking(move || acceptance_query_repo(&project_path))
                .await
                .ok()
                .flatten();
            let n = match repo {
                Some(repo) => client.acceptance_count(&repo).await.ok(),
                None => None,
            };
            let _ = proxy.send_event(Message::Project(project::Message::AcceptanceCountLoaded(
                project_id, n,
            )));
        });
    }
```

删除 `acceptance_query_repo` 自由函数及其文档注释（约 3769-3775 行）：
```rust
/// 从项目路径求"验收查询键"：与落库侧同款 `delivery::repo_root`（git
/// toplevel，解析符号链接/子目录），保证 `count_for_repo` 精确匹配命中。
/// 非 git 路径返回 `None`——验收依赖 git ref 沉淀，非 git 仓库不可能有记录，
/// 直接不查，别拿未规范化的原始路径去撞库（会静默查不到→副行空白）。
pub(crate) fn acceptance_query_repo(project_path: &str) -> Option<String> {
    delivery::repo_root(Path::new(project_path)).map(|p| p.to_string_lossy().into_owned())
}
```
及其测试（约 4528-4539 行，函数名 `acceptance_query_repo_none_for_non_git_path`，整个 `#[test] fn` 删除）。

删除 `comment_focused` 委托方法（约 2012-2015 行）：
```rust
    /// 验收意见框是否持有 iced 真实焦点(main.rs 原生放行闸门用)。
    pub fn comment_focused(&self) -> bool {
        self.acceptance.comment_focused()
    }
```

删除 `SessionTab.delivery_pending: bool` 字段（约 246 行，含其文档注释行），及其三处初始化 `delivery_pending: false,`（约 546、1917、4729 行）。

删除 `SessionTab.last_turn_head: Option<String>` 字段（约 248 行），及其三处初始化 `last_turn_head: None,`（约 547、1918、4730 行，紧邻 `delivery_pending` 初始化的下一行）。

删除 `effective_project_repo` 自由函数及其文档注释（约 3762-3767 行）：
```rust
/// 交付/验收使用的仓库：当前项目优先，无则回落会话 cwd（P1f 现状；P1g D4）。
pub(crate) fn effective_project_repo(active: Option<&Path>, session_cwd: &Path) -> PathBuf {
    active
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| session_cwd.to_path_buf())
}
```
及其测试 `effective_project_repo_prefers_active`（约 4541-4550 行，整个 `#[test] fn` 删除）。

删除 `crates/dozer-app/src/app.rs:44` 附近 `use crate::workspace::{ ... effective_project_repo, ... }` 列表里的 `effective_project_repo,` 一项（本任务 Step 7 之后 app.rs 已不再调用它）。

- [ ] **Step 10: `rail.rs` — 图标/tooltip/金点徽标/默认布局/11→10 计数**

`crates/dozer-app/src/rail.rs:2` 模块文档注释，"11 个面板" 改成 "10 个面板"。

`crates/dozer-app/src/rail.rs:87` 附近文档注释里的"保证 11 个面板不重不漏"改成"保证 10 个面板不重不漏"。

`crates/dozer-app/src/rail.rs:125-130`（`RailLayout::default()` 的 `right` 数组），删除 `PanelKind::Acceptance,`：
```rust
            right: vec![
                PanelKind::Agent,
                PanelKind::Conversations,
                PanelKind::Usage,
            ],
```

`crates/dozer-app/src/rail.rs:135-138` 文档注释"两侧合计不是恰 11 个不重复"改成"恰 10 个"。

`crates/dozer-app/src/rail.rs:146` 逻辑：
```rust
    if all.len() != 11 || rail.left.len() + rail.right.len() != 11 {
```
改为：
```rust
    if all.len() != 10 || rail.left.len() + rail.right.len() != 10 {
```

`crates/dozer-app/src/rail.rs:453-455` 文档注释"11 个 `PanelKind` variant"改成"10 个"。

`crates/dozer-app/src/rail.rs:456-470`（`panel_meta`），删除：
```rust
        PanelKind::Acceptance => (icons::IconKind::BadgeCheck, "验收"),
```

删除整个 `panel_badge` 函数（约 472-503 行）：
```rust
/// 面板专属的按钮徽标装饰(目前只有验收面板有:当前激活 tab 有待处理
/// 交付时,右上角叠一个金色小圆点)。其余 10 个面板返回 `None`。
fn panel_badge(
    app: &App,
    kind: PanelKind,
) -> Option<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if kind != PanelKind::Acceptance {
        return None;
    }
    let pending = app
        .active_workspace()
        .and_then(|ws| ws.tabs.get(ws.active))
        .map(|t| t.delivery_pending)
        .unwrap_or(false);
    if !pending {
        return None;
    }
    Some(
        container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().gold.into()),
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    )
}
```

其调用点（约 367-370 行）：
```rust
        let entry = match panel_badge(app, kind) {
            Some(badge) => stack![base, badge].into(),
            None => base,
        };
```
改为：
```rust
        let entry = base;
```
（如果 `entry` 后续用法要求它是 `Element<...>` 而不是需要 `.into()` 的具体类型，直接用 `base` 即可，因为 `base` 本来就已经声明为 `Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>` 类型。）

`crates/dozer-app/src/rail.rs:674-685`（测试），原：
```rust
    /// `RailLayout::default()` 把 11 个面板不重不漏分到左右两栏,
    /// 与现状 7/4 分组逐一对应(防漂移锚)。
    #[test]
    fn rail_layout_default_covers_all_panels_without_duplicates() {
        let rail = RailLayout::default();
        assert_eq!(rail.left.len(), 7);
        assert_eq!(rail.right.len(), 4);
        let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
        all.sort_by_key(|k| format!("{k:?}"));
        all.dedup();
        assert_eq!(all.len(), 11, "11 个面板不重不漏分到左右两栏");
    }
```
改为：
```rust
    /// `RailLayout::default()` 把 10 个面板不重不漏分到左右两栏,
    /// 与现状 7/3 分组逐一对应(防漂移锚)。
    #[test]
    fn rail_layout_default_covers_all_panels_without_duplicates() {
        let rail = RailLayout::default();
        assert_eq!(rail.left.len(), 7);
        assert_eq!(rail.right.len(), 3);
        let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
        all.sort_by_key(|k| format!("{k:?}"));
        all.dedup();
        assert_eq!(all.len(), 10, "10 个面板不重不漏分到左右两栏");
    }
```

`crates/dozer-app/src/rail.rs:694-701`（测试），删除最后一行断言：
```rust
    #[test]
    fn side_of_finds_every_default_panel() {
        let rail = RailLayout::default();
        assert_eq!(rail.side_of(PanelKind::Files), Side::Left);
        assert_eq!(rail.side_of(PanelKind::Web), Side::Left);
        assert_eq!(rail.side_of(PanelKind::Agent), Side::Right);
    }
```

`crates/dozer-app/src/rail.rs:703-730`（`sanitize_rail_layout_falls_back_to_default_on_bad_data`），原：
```rust
    /// `sanitize_rail_layout` 对坏数据回落默认:任一栏为空、面板重复、
    /// 面板数不是 11——任一情形都不做部分修复。
    #[test]
    fn sanitize_rail_layout_falls_back_to_default_on_bad_data() {
        // 左侧为空。
        let empty_left = RailLayout {
            left: vec![],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(empty_left), RailLayout::default());

        // 面板数不是 11。
        let too_few = RailLayout {
            left: vec![PanelKind::Files],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(too_few), RailLayout::default());

        // 面板重复(缺一个面板 + 重复另一个,合计仍 11 但去重后不足)。
        let dup = RailLayout {
            left: vec![PanelKind::Files; 7],
            right: vec![PanelKind::Agent; 4],
        };
        assert_eq!(sanitize_rail_layout(dup), RailLayout::default());
```
改为：
```rust
    /// `sanitize_rail_layout` 对坏数据回落默认:任一栏为空、面板重复、
    /// 面板数不是 10——任一情形都不做部分修复。
    #[test]
    fn sanitize_rail_layout_falls_back_to_default_on_bad_data() {
        // 左侧为空。
        let empty_left = RailLayout {
            left: vec![],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(empty_left), RailLayout::default());

        // 面板数不是 10。
        let too_few = RailLayout {
            left: vec![PanelKind::Files],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(too_few), RailLayout::default());

        // 面板重复(缺一个面板 + 重复另一个,合计仍 10 但去重后不足)。
        let dup = RailLayout {
            left: vec![PanelKind::Files; 7],
            right: vec![PanelKind::Agent; 3],
        };
        assert_eq!(sanitize_rail_layout(dup), RailLayout::default());
```
（该测试函数后续还有"合法数据原样保留"的部分，保持不动。）

- [ ] **Step 11: `webview_geometry.rs` — 两处 match 分支 + 三处测试**

`crates/dozer-app/src/webview_geometry.rs:24` 文档注释"Conversations/Usage/Acceptance)"改成"Conversations/Usage)"。

`crates/dozer-app/src/webview_geometry.rs:113-120`，原：
```rust
            // Database/Ssh/Todo/GitLog 纯 iced 绘制,不挂 webview 子视图;
            // Agent/Usage/Acceptance 同理——任一侧放大只要显示的是这几种,
            // 都没有 webview 可摆。
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
```
改为：
```rust
            // Database/Ssh/Todo/GitLog 纯 iced 绘制,不挂 webview 子视图;
            // Agent/Usage 同理——任一侧放大只要显示的是这几种,
            // 都没有 webview 可摆。
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Usage => (0.0, 0.0, 0.0, 0.0),
```

`crates/dozer-app/src/webview_geometry.rs:239`：
```rust
        PanelKind::Agent | PanelKind::Usage | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
```
改为：
```rust
        PanelKind::Agent | PanelKind::Usage => (0.0, 0.0, 0.0, 0.0),
```

`crates/dozer-app/src/webview_geometry.rs:735-741` 文档注释里去掉 `PanelKind::Acceptance` 的提及（若整段注释仍通顺，只删变体名即可，不用改写句子结构）。

`crates/dozer-app/src/webview_geometry.rs:744`：
```rust
        for kind in [PanelKind::Agent, PanelKind::Usage, PanelKind::Acceptance] {
```
改为：
```rust
        for kind in [PanelKind::Agent, PanelKind::Usage] {
```

`crates/dozer-app/src/webview_geometry.rs:771-776`：
```rust
        for kind in [
            PanelKind::Agent,
            PanelKind::Conversations,
            PanelKind::Usage,
            PanelKind::Acceptance,
        ] {
```
改为：
```rust
        for kind in [
            PanelKind::Agent,
            PanelKind::Conversations,
            PanelKind::Usage,
        ] {
```

`crates/dozer-app/src/webview_geometry.rs:843`注释"Project 从默认左栏挪到右栏(11 个面板不重不漏)"改成"(10 个面板不重不漏)"，`crates/dozer-app/src/webview_geometry.rs:853-859`：
```rust
                    right: vec![
                        PanelKind::Project,
                        PanelKind::Agent,
                        PanelKind::Conversations,
                        PanelKind::Usage,
                        PanelKind::Acceptance,
                    ],
```
改为：
```rust
                    right: vec![
                        PanelKind::Project,
                        PanelKind::Agent,
                        PanelKind::Conversations,
                        PanelKind::Usage,
                    ],
```

- [ ] **Step 12: `panel_layouts.rs` — 测试夹具**

`crates/dozer-app/src/panel_layouts.rs:151` 与 `:181` 两处 `right_view: PanelKind::Acceptance,` 都改成 `right_view: PanelKind::Usage,`。

- [ ] **Step 13: `main.rs` — 原生输入焦点闸门 + comment_focused 每帧查询**

`crates/dozer-app/src/main.rs:1234` 删除一行：
```rust
                || app.comment_focused()
```
（它是一个更长的 `if ... || ... { return; }` 判断链中的一行，删除后确认上下相邻的 `||` 语法仍然连贯。）

`crates/dozer-app/src/main.rs` 里"验收意见框(Stage 6)"那段（约 2565-2577 行）整段删除：
```rust
                                // 验收意见框(Stage 6):同款每帧查真实焦点态。
                                let comment_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Acceptance)
                                    {
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::acceptance::CaptureCommentFocus,
                                        );
                                        extensions::acceptance::take_comment_focused()
                                    } else {
                                        false
                                    };

```

`crates/dozer-app/src/main.rs` 约 2787 行删除：
```rust
                                app.set_comment_focused(comment_focused);
```

- [ ] **Step 14: 编译验证**

```bash
cargo build -p dozer-app 2>&1 | tail -100
```
预期：无 `PanelKind::Acceptance`/`Message::Acceptance`/`comment_focused`/`delivery_pending`/`effective_project_repo` 相关报错。如果还有报错，重复 Step 2 的排查方式，直到干净通过。

- [ ] **Step 15: 测试验证**

```bash
cargo test -p dozer-app 2>&1 | tail -60
```
预期全部通过，尤其确认 Step 10/11/12 改过的测试（`rail_layout_default_covers_all_panels_without_duplicates`/`side_of_finds_every_default_panel`/`sanitize_rail_layout_falls_back_to_default_on_bad_data`/`preview_content_bounds_bare_for_right_panel_on_left`/`is_in_preview_column_false_for_right_panel_on_left`/`is_in_preview_column_returns_project_when_project_on_right`/`save_then_load_round_trips_per_project`/`per_project_layouts_are_independent`）都在绿。

- [ ] **Step 16: clippy + fmt**

```bash
cargo clippy -p dozer-app --all-targets 2>&1 | tail -60
cargo fmt -p dozer-app
```

- [ ] **Step 17: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/rail.rs crates/dozer-app/src/webview_geometry.rs crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/panel_layouts.rs
git commit -m "feat(dozer-app): 摘除 PanelKind::Acceptance 消费侧接线，为整体删除验收面板铺路"
```

---

### Task 2: 删除 `extensions/acceptance.rs` 与 `goal.rs`

**Files:**
- Delete: `crates/dozer-app/src/extensions/acceptance.rs`
- Delete: `crates/dozer-app/src/goal.rs`
- Modify: `crates/dozer-app/src/extensions.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Task 1 已经清空了对这两个模块的全部外部引用，本任务是纯粹的文件/模块声明删除。

- [ ] **Step 1: 删除文件**

```bash
rm crates/dozer-app/src/extensions/acceptance.rs
rm crates/dozer-app/src/goal.rs
```

- [ ] **Step 2: 删除模块声明**

`crates/dozer-app/src/extensions.rs:6` 删除：
```rust
pub mod acceptance;
```

`crates/dozer-app/src/main.rs:10` 删除：
```rust
mod goal;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p dozer-app 2>&1 | tail -60
```
预期干净通过（Task 1 已确保没有任何代码引用这两个模块）。

- [ ] **Step 4: 测试验证**

```bash
cargo test -p dozer-app 2>&1 | tail -30
```

- [ ] **Step 5: Commit**

```bash
git add -A crates/dozer-app/src/extensions.rs crates/dozer-app/src/main.rs
git status  # 确认 acceptance.rs/goal.rs 显示为 deleted
git add crates/dozer-app/src/extensions/acceptance.rs crates/dozer-app/src/goal.rs
git commit -m "feat(dozer-app): 删除 extensions::acceptance 与 goal.rs"
```

---

### Task 3: `delivery.rs` — 摘除验收专属函数

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`

**Interfaces:**
- 保留不动：`repo_root`/`is_dirty`/`branch`/`local_branches`/`checkout_branch`/`init_repo`/`file_statuses`/`ChangeKind`/`FileGitStatus`/`dir_status`/`remote_url` 等——仍被 `files.rs`/分支 UI/`workspace.rs`/`git_log.rs`/`app.rs` 使用。
- 删除后 `head_commit`/`last_accepted`/`accept`/`changes`/`file_diff`/`FileChange`/`ACCEPTED_REF_PREFIX`/`delivery_pending` 在整个 crate 内不再有任何引用（已用 grep 核实：这些符号仅被本文件内部或已删除的 `extensions/acceptance.rs`/app.rs 的已删代码引用）。

- [ ] **Step 1: 删除常量与类型**

`crates/dozer-app/src/delivery.rs:9` 删除：
```rust
pub const ACCEPTED_REF_PREFIX: &str = "refs/dozer/accepted/";
```

`crates/dozer-app/src/delivery.rs:11-17` 删除：
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    pub path: String,
    /// None = 二进制或未跟踪（numstat 给不出行数）
    pub added: Option<u32>,
    pub removed: Option<u32>,
}
```

- [ ] **Step 2: 删除函数**

`crates/dozer-app/src/delivery.rs:53-57` 删除 `head_commit`：
```rust
pub fn head_commit(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}
```

`crates/dozer-app/src/delivery.rs:59-76` 删除 `last_accepted`（含文档注释）：
```rust
/// 现存最大号 accepted ref：`(n, commit)`。
pub fn last_accepted(repo: &Path) -> Option<(u32, String)> {
    let out = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            ACCEPTED_REF_PREFIX,
        ],
    )?;
    out.lines()
        .filter_map(|l| {
            let (name, commit) = l.split_once(' ')?;
            let n: u32 = name.strip_prefix(ACCEPTED_REF_PREFIX)?.parse().ok()?;
            Some((n, commit.to_string()))
        })
        .max_by_key(|(n, _)| *n)
}
```

`crates/dozer-app/src/delivery.rs:78-111` 删除 `changes`（含文档注释）。

`crates/dozer-app/src/delivery.rs:113-169` 删除 `file_diff` 及其常量 `FILE_DIFF_MAX_CHARS`（含文档注释）。

`crates/dozer-app/src/delivery.rs:171-189` 删除 `accept`（含文档注释）。

删除后 `Context`/`bail`（来自 `crates/dozer-app/src/delivery.rs:4` 的 `use anyhow::{Context, Result, bail};`）在本文件里只剩 `accept`/`file_diff` 用到——两者都已删除，`.context(...)` 与 `bail!(...)` 在文件其余部分（`checkout_branch`/`init_repo` 等仍保留的函数用的是 `Result<(), String>`，不调用这两个宏/方法）没有其他调用点。把该行改成：
```rust
use anyhow::Result;
```

`crates/dozer-app/src/delivery.rs` 中 `delivery_pending` 函数（含文档注释，原约 473-487 行）删除：
```rust
/// spec D3 的"有变更"精确定义（纯函数，方便矩阵测试）。
pub fn delivery_pending(
    dirty: bool,
    head: Option<&str>,
    accepted: Option<&str>,
    last_turn_head: Option<&str>,
) -> bool {
    if dirty {
        return true;
    }
    match accepted {
        Some(a) => head != Some(a),
        None => head != last_turn_head,
    }
}
```

- [ ] **Step 3: 修剪/删除测试**

`repo_root_and_head_and_dirty` 测试（原约 517-531 行）保留，但删除其中依赖 `head_commit` 的一行断言：
```rust
        assert!(head_commit(&repo).is_some());
```
（保留测试其余部分——它主要测的是 `repo_root`/`is_dirty`，这两个函数不删。）

整段删除以下测试函数（均只测已删除的函数）：
- `accept_writes_sequential_refs_and_refuses_dirty`
- `changes_lists_modified_and_untracked_against_baseline`
- `file_diff_new_file_shows_all_added_lines`
- `file_diff_modified_file_shows_plus_minus_lines`
- `file_diff_truncates_when_too_long`
- `delivery_pending_matrix`

`branch_none_when_head_detached` 测试保留（测的是仍保留的 `branch()`），但把依赖已删 `head_commit` 的这一行：
```rust
        let head = head_commit(&repo).unwrap();
```
改写成直接用测试自己的 git 子进程调用取 HEAD：
```rust
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
```

- [ ] **Step 4: 编译验证**

```bash
cargo build -p dozer-app 2>&1 | tail -60
```

- [ ] **Step 5: 测试验证**

```bash
cargo test -p dozer-app delivery:: 2>&1 | tail -60
```
预期：`repo_root_and_head_and_dirty`/`branch_of_repo`/`branch_none_when_head_detached`/`file_statuses_*`/`dir_status_*`/`remote_url_reads_origin` 等仍保留的测试全部通过；已删除的测试不再出现在列表里。

- [ ] **Step 6: clippy + fmt**

```bash
cargo clippy -p dozer-app --all-targets 2>&1 | tail -40
cargo fmt -p dozer-app
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "feat(dozer-app): delivery.rs 摘除验收专属函数(accept/changes/file_diff/delivery_pending 等)"
```

---

### Task 4: `dozer-client` — 摘除验收相关客户端方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- 本任务完成后 `dozer-client` 不再有任何符号引用 `Request::RecordAcceptance`/`Request::GetAcceptanceCount`/`Reply::AcceptanceCount`——这些协议变体本身要等 Task 5 才删（顺序上先删客户端调用方，再删协议定义，中间态是"协议里还有这几个变体但没人构造"，编译不受影响）。

- [ ] **Step 1: 删除 `RecordAcceptanceParams`**

`crates/dozer-client/src/lib.rs:33-44` 删除：
```rust
/// `Client::record_acceptance` 的参数对象:原先 7 个位置参数里 `repo`/
/// `goal`/`verdict`/`comment`/`ref_name` 五个都是 `&str`,顺序传错编译器
/// 发现不了(Rust Design Patterns:Builder,用具名字段替代同类型位置参数)。
pub struct RecordAcceptanceParams<'a> {
    pub repo: &'a str,
    pub goal: &'a str,
    pub criteria_checked: &'a [String],
    pub verdict: &'a str,
    pub comment: &'a str,
    pub ref_name: &'a str,
    pub ts_ms: u64,
}
```

- [ ] **Step 2: 删除 `record_acceptance`/`acceptance_count` 方法**

`crates/dozer-client/src/lib.rs:178-194` 删除 `record_acceptance`：
```rust
    pub async fn record_acceptance(&self, params: RecordAcceptanceParams<'_>) -> Result<()> {
        match self
            .roundtrip(&Request::RecordAcceptance {
                repo: params.repo.into(),
                goal: params.goal.into(),
                criteria_checked: params.criteria_checked.to_vec(),
                verdict: params.verdict.into(),
                comment: params.comment.into(),
                ref_name: params.ref_name.into(),
                ts_ms: params.ts_ms,
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

`crates/dozer-client/src/lib.rs:227-236` 删除 `acceptance_count`：
```rust
    pub async fn acceptance_count(&self, repo: &str) -> Result<u64> {
        match self
            .roundtrip(&Request::GetAcceptanceCount { repo: repo.into() })
            .await?
        {
            Reply::AcceptanceCount { count } => Ok(count),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => Err(anyhow::anyhow!("非预期应答: {other:?}")),
        }
    }
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p dozer-client 2>&1 | tail -40
```

- [ ] **Step 4: 测试验证**

```bash
cargo test -p dozer-client 2>&1 | tail -40
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): 摘除 record_acceptance/acceptance_count 客户端方法"
```

---

### Task 5: `dozer-core` + `dozerd` — 协议变体、存储、handler、全部 ripple 测试

这是第二大任务：`dozerd::server::serve()` 签名去掉 `store` 参数后，18 处调用点（6 个文件）里的 12 处（`dozerd/tests/session_survival.rs` 6 处 + `dozerd/tests/hook_events.rs` 6 处）都要跟着改，此外 `dozer-client`/`dozer-mcp` 的 4 个集成测试文件各有 1 处。这部分体量大但机械（少字段、参数数量对不上，编译器会精确报出每个调用点），不逐个抄录到本步骤里，按 Step 4 的方法批量修。

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`
- Modify: `crates/dozerd/src/lib.rs`
- Delete: `crates/dozerd/src/acceptance.rs`
- Modify: `crates/dozerd/tests/session_survival.rs`
- Modify: `crates/dozerd/tests/hook_events.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`
- Modify: `crates/dozer-mcp/tests/submit_session_summary.rs`
- Modify: `crates/dozer-mcp/tests/get_preview_context.rs`
- Modify: `crates/dozer-mcp/tests/todo_tools.rs`
- (可选轻量清理) `crates/dozerd/src/bookmarks.rs`/`projects.rs`/`session_summary.rs`/`todo.rs` 里提到 "AcceptanceStore" 的注释

**Interfaces:**
- Task 4 已确保 `dozer-client` 不再构造 `Request::RecordAcceptance`/`Request::GetAcceptanceCount`，本任务可以安全删除协议定义本身。

- [ ] **Step 1: 删除协议变体**

`crates/dozer-core/src/protocol.rs:374-383` 删除（含文档注释）：
```rust
    /// 验收通过的结构性记录（spec P1f D5）；acceptor 由 daemon 侧补 "user"。
    RecordAcceptance {
        repo: String,
        goal: String,
        criteria_checked: Vec<String>,
        verdict: String,
        comment: String,
        ref_name: String,
        ts_ms: u64,
    },
```

`crates/dozer-core/src/protocol.rs:416-419` 删除（含文档注释）：
```rust
    /// 取某仓库的验收次数（项目卡"N 次验收"用）。
    GetAcceptanceCount {
        repo: String,
    },
```

`crates/dozer-core/src/protocol.rs:637-640` 删除（含文档注释）：
```rust
    /// 验收次数。
    AcceptanceCount {
        count: u64,
    },
```

删除测试 `record_acceptance_roundtrips`（约 1032-1045 行）与 `acceptance_count_request_roundtrips`（约 1079-1087 行），整个 `#[test] fn` 各自删除。

- [ ] **Step 2: `dozerd/src/acceptance.rs` 删除 + `lib.rs` 模块声明删除**

```bash
rm crates/dozerd/src/acceptance.rs
```

`crates/dozerd/src/lib.rs:1` 删除：
```rust
pub mod acceptance;
```

- [ ] **Step 3: `server.rs` — 签名与 handler**

`crates/dozerd/src/server.rs:67-80`（`pub async fn serve`），删除参数 `store: Arc<crate::acceptance::AcceptanceStore>,`（该行位于 `registry` 之后、`projects` 之前）。

`crates/dozerd/src/server.rs` 内 `serve` 函数体里 `let store = store.clone();`（accept 循环里，紧邻 `let registry = registry.clone();` 之后）一行删除。

`crates/dozerd/src/server.rs` 内 `handle_conn(...)` 调用处，删除传入的 `store,` 实参。

`crates/dozer-app/src/server.rs`（笔误更正：是 `crates/dozerd/src/server.rs`）里 `async fn handle_conn(...)`（约 293-307 行）签名删除参数 `store: Arc<crate::acceptance::AcceptanceStore>,`（位于 `registry` 之后、`projects` 之前，与 `serve` 签名对称）。

`crates/dozerd/src/server.rs:403-428` 删除整个 `Request::RecordAcceptance { ... } => { ... }` 分支。

`crates/dozerd/src/server.rs:448-451` 删除整个 `Request::GetAcceptanceCount { repo } => match store.count_for_repo(&repo) { ... }` 分支。

`crates/dozerd/src/server.rs` 里测试辅助函数 `_assert_signature`（约 896-926 行）同步删除 `store` 参数与传参：
```rust
        fn _assert_signature(
            socket: &std::path::Path,
            registry: std::sync::Arc<crate::registry::SessionRegistry>,
            projects: std::sync::Arc<crate::projects::ProjectStore>,
            bookmarks: std::sync::Arc<crate::bookmarks::BookmarkStore>,
            transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
            session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
            todos: std::sync::Arc<crate::todo::TodoStore>,
            categories: std::sync::Arc<crate::todo_category::CategoryStore>,
        ) {
            let fut = crate::server::serve(
                socket,
                registry,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                std::sync::Arc::new(crate::session_summary_backfill::BackfillRegistry::new()),
                todos,
                categories,
                crate::task_poller::new_in_flight(),
            );
            std::mem::drop(fut);
        }
```
（去掉 `store` 参数声明和传参这两处，其余保持不变。）

- [ ] **Step 4: `main.rs` — 移除 store 构造与传参**

`crates/dozerd/src/main.rs:82-84` 删除：
```rust
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

`crates/dozerd/src/main.rs` 里 `dozerd::server::serve(...)` 调用（约 115-127 行）删除 `store,` 这一行实参。

- [ ] **Step 5: 编译，收集全部 ripple 报错**

```bash
cargo build --workspace --tests 2>&1 | grep -E "^error" -A 5
```

预期报错集中在两类：
1. "no field `store`"/"this function takes N arguments but M were supplied" —— 全部出现在 `crates/dozerd/tests/session_survival.rs`（6 处 `serve(...)` 调用，均通过内联 `test_store(),` 传参）与 `crates/dozerd/tests/hook_events.rs`（6 处，2 处内联 `test_store(),`、2 处 `let store = test_store();` 具名绑定后传参、2 处 `let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());` 具名绑定后传参）。
2. 同类报错出现在 `crates/dozer-client/tests/against_real_daemon.rs`（1 处）、`crates/dozer-mcp/tests/submit_session_summary.rs`（1 处）、`crates/dozer-mcp/tests/get_preview_context.rs`（1 处）、`crates/dozer-mcp/tests/todo_tools.rs`（1 处）——这 4 个文件的模式完全一致：先有一行 `let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());`，后面 `dozerd::server::serve(...)` 调用里传了 `store,`。

对每一处报错：删掉该调用点里传给 `serve(...)` 的 `store`/`test_store()` 实参那一行，以及为它准备的 `let store = ...;`/内联 `test_store(),` 绑定（如果是内联在参数列表里的 `test_store(),`，直接删那一行即可；如果是先 `let store = ...;` 再在参数列表里传 `store,`，两处都要删）。

- [ ] **Step 6: 单独处理 `hook_events.rs` 里的 `record_acceptance_persists` 测试**

这一个不是机械 ripple，是在测验收功能本身，整个删除。`crates/dozerd/tests/hook_events.rs` 里的 `#[tokio::test] async fn record_acceptance_persists() { ... }`（约 163-242 行，含它上面独立定义的 `fn test_store() -> std::sync::Arc<dozerd::acceptance::AcceptanceStore> { ... }` 辅助函数，约 157-161 行）整段删除。

删除后确认 `test_store()` 在 `hook_events.rs` 里再无其他调用点（Step 5 已经把其余调用点的 `test_store()` 引用一起删掉了）；如果确认无调用，本函数已随上面一起删除，无需额外操作。同理确认 `crates/dozerd/tests/session_survival.rs` 里的 `fn test_store() -> std::sync::Arc<dozerd::acceptance::AcceptanceStore> { ... }`（约 600-602 行）在 Step 5 清完 6 处调用点后也不再被引用，一并删除这个辅助函数本身。

- [ ] **Step 7: 反复编译直到干净**

```bash
cargo build --workspace --tests 2>&1 | tail -100
```
重复 Step 5/6 的方法直到没有报错。

- [ ] **Step 8: 最终 grep 验证——防止死代码/未用 helper 残留**

编译干净不代表没有残留的未使用 helper（比如 `test_store()` 若不小心漏删一个调用点会变成"仍被引用"从而不报错；反过来若某个 `test_store()` 定义变成零调用点，只是 warning 不是 error，不会被 Step 7 的 build 逼出来）。跑：

```bash
grep -rn "AcceptanceStore\|AcceptanceRecord\|RecordAcceptance\|GetAcceptanceCount\|AcceptanceCount\|test_store\b" --include="*.rs" .
```

预期：零命中。如果有命中，逐个确认是否为遗留死代码并删除。

- [ ] **Step 9: (可选) 清理 dozerd 内注释里对 AcceptanceStore 的类比措辞**

`crates/dozerd/src/bookmarks.rs:1`、`crates/dozerd/src/projects.rs:3`、`crates/dozerd/src/session_summary.rs:2`、`crates/dozerd/src/todo.rs:1` 四处模块文档注释里提到"与 AcceptanceStore 共享同一个/各持一个连接"之类的类比措辞——`AcceptanceStore` 已不存在，改成类比另一个仍存在的 store（如 `ProjectStore`/`BookmarkStore` 互相类比）或直接删掉这半句类比，不强制要求，但建议顺手清理，避免注释引用不存在的类型。

- [ ] **Step 10: 测试验证**

```bash
cargo test -p dozer-core -p dozerd -p dozer-client -p dozer-mcp 2>&1 | tail -100
```
预期全部通过；`record_acceptance_persists`/`record_acceptance_roundtrips`/`acceptance_count_request_roundtrips`/`AcceptanceStore` 自己的两个单测（`open_record_count_roundtrip`/`count_for_repo_filters_by_repo`，随整个文件删除）均不再出现在测试列表里。

- [ ] **Step 11: clippy + fmt**

```bash
cargo clippy --workspace --all-targets 2>&1 | tail -100
cargo fmt --all
```

- [ ] **Step 12: Commit**

```bash
git add -A crates/dozer-core crates/dozerd crates/dozer-client crates/dozer-mcp
git commit -m "feat(dozerd): 删除验收存储/协议/handler，修复全部 ripple 测试夹具"
```

---

### Task 6: 文档 — CLAUDE.md + `docs/user_guide/*`

**Files:**
- Modify: `CLAUDE.md`
- Delete: `docs/user_guide/acceptance.md`
- Modify: `docs/user_guide/README.md`
- Modify: `docs/user_guide/getting-started.md`
- Modify: `docs/user_guide/panels.md`
- Modify: `docs/user_guide/configuration.md`
- Modify: `docs/user_guide/agents.md`

**Interfaces:** 无（纯文档改动，不影响编译/测试）。

- [ ] **Step 1: `CLAUDE.md`**

第 19 行，原：
```
| `crates/dozerd` | session daemon：PTY 池、会话存活、验收闭环存储（bin: `dozerd`） |
```
改为：
```
| `crates/dozerd` | session daemon：PTY 池、会话存活（bin: `dozerd`） |
```

第 39 行，原：
```
- GUI 只用 iced 0.14 生态；预览 WebView 走 wry 子视图叠加，webview 恒在 GPU 内容之上——凡是需要盖住它的原生浮层（如验收 tab 全屏态）都要显式隐藏 webview，不能指望层级自然遮挡。**⌘K 命令面板未实现**（规格 §3 已裁掉一期范围，顶栏曾有的纯视觉占位搜索框已于后续迭代移除，见 `app.rs` 中该处的移除说明注释）；「⌘K 打开时隐藏预览」这条早期设计陈述作废，不要再引用它。
```
改为：
```
- GUI 只用 iced 0.14 生态；预览 WebView 走 wry 子视图叠加，webview 恒在 GPU 内容之上——凡是需要盖住它的原生浮层（如 Usage 面板全屏态）都要显式隐藏 webview，不能指望层级自然遮挡。**⌘K 命令面板未实现**（规格 §3 已裁掉一期范围，顶栏曾有的纯视觉占位搜索框已于后续迭代移除，见 `app.rs` 中该处的移除说明注释）；「⌘K 打开时隐藏预览」这条早期设计陈述作废，不要再引用它。
```

- [ ] **Step 2: 删除 `docs/user_guide/acceptance.md`**

```bash
rm docs/user_guide/acceptance.md
```

- [ ] **Step 3: `docs/user_guide/README.md`**

删除第 13 行整条 TOC 项：
```
- **[验收闭环](acceptance.md)** —— `.dozer/goal.md` 格式、验收标准、diff 查看、通过("沉淀")与打回、演进史。
```

第 22 行，原：
```
- **项目是资产,agent 是可更换的乙方。** 目标、验收标准、验收记录、对话历史都挂在项目上,不挂在某个 agent 上——今天用 Claude、明天换 Codex,历史不丢。
```
改为：
```
- **项目是资产,agent 是可更换的乙方。** 对话历史都挂在项目上,不挂在某个 agent 上——今天用 Claude、明天换 Codex,历史不丢。
```

第 23 行，原：
```
- **验收权不属于被验收方。** 通过/打回是你按的按钮,不是 agent 自己说"我做完了"就算数。
```
改为：
```
- **验收权不属于被验收方。**
```

- [ ] **Step 4: `docs/user_guide/getting-started.md`**

第 63 行删除整条 TOC 项：
```
- [验收闭环](acceptance.md) —— goal.md 格式、验收标准、通过与打回。
```

第 49-56 行"一轮最简闭环"整节，原：
```
## 一轮最简闭环

1. 在项目根目录手写一个 `.dozer/goal.md`:第一行是目标,后面若干 `- [ ]` 是验收标准(Dozer 目前没有图形化的创建入口,纯文本即可,agent 也可以帮你写)。
2. 让 agent 干活。
3. 一轮结束(agent 报告"我做完了"或你觉得该看看了),点右侧 **验收** 图标——如果有改动待审,图标上会有一个金色小红点。
4. 在验收视图里逐个勾标准、看 diff、写意见,然后"通过·沉淀"(打一个 `refs/dozer/accepted/N` 的 git ref)或"打回并注回"(把你的意见当作输入直接发回那个 agent 的终端)。

这个循环是 Dozer 的核心,详见 [验收闭环](acceptance.md)。
```
改为：
```
## 一轮最简闭环

1. 让 agent 干活。
2. 一轮结束后,自己判断这轮做得怎么样、要不要继续。
```

- [ ] **Step 5: `docs/user_guide/panels.md`**

第 3 行，原：
```
Dozer 一共有 11 种面板,挂在左右两条图标栏上(见 [工作区布局](workspace.md))。**Project**、**Acceptance**、**Agent**、**Conversations** 分别在 [项目管理](projects.md)、[验收闭环](acceptance.md)、[Agent 会话](agents.md) 里单独讲过,本页覆盖剩下的七个。
```
改为：
```
Dozer 一共有 10 种面板,挂在左右两条图标栏上(见 [工作区布局](workspace.md))。**Project**、**Agent**、**Conversations** 分别在 [项目管理](projects.md)、[Agent 会话](agents.md) 里单独讲过,本页覆盖剩下的七个。
```

- [ ] **Step 6: `docs/user_guide/configuration.md`**

第 27 行，原：
```
| `dozer.db` | `dozerd` 的 SQLite 库:项目列表、验收记录、浏览器书签、会话摘要、已摄取的对话/transcript 索引 |
```
改为：
```
| `dozer.db` | `dozerd` 的 SQLite 库:项目列表、浏览器书签、会话摘要、已摄取的对话/transcript 索引 |
```

第 33 行整行删除（`.dozer/goal.md` 这一行；Dozer 已不再读取该文件）：
```
| `.dozer/goal.md` | 当前目标 + 验收标准(纯手写 Markdown,见 [验收闭环](acceptance.md)) |
```

- [ ] **Step 7: `docs/user_guide/agents.md`**

第 47 行整行删除：
```
`TurnEnded` 是触发 [验收](acceptance.md) 检查的信号——一轮结束后 Dozer 会去看工作区有没有改动、HEAD 有没有前移,决定要不要在验收图标上点一个金色小红点提醒你。
```
（删除后确认上下文空行不重复即可，无需补充新句子。）

- [ ] **Step 8: 检查是否还有遗漏的 acceptance.md 链接**

```bash
grep -rn "acceptance\.md" docs/user_guide/
```
预期：零命中。

- [ ] **Step 9: Commit**

```bash
git add CLAUDE.md docs/user_guide/
git commit -m "docs: 删除验收面板相关文档，更新 CLAUDE.md 措辞"
```

---

### Task 7: 全 workspace 最终验证

**Files:** 无新改动，纯验证。

- [ ] **Step 1: 全量 build**

```bash
cargo build --workspace 2>&1 | tail -60
```
预期无 error。已知会有 1 条既存 linker warning（`__eh_frame section too large`）与既存 `block v0.1.6` future-incompat 提示，与本次改动无关，可忽略。

- [ ] **Step 2: 全量 clippy**

```bash
cargo clippy --workspace --all-targets 2>&1 | tail -100
```
预期无 warning（除上述两条既存提示）。

- [ ] **Step 3: 全量 fmt 检查**

```bash
cargo fmt --all -- --check
```
如有格式问题跑 `cargo fmt --all` 后重新 `git add`。

- [ ] **Step 4: 全量测试**

```bash
cargo test --workspace 2>&1 | tail -150
```
预期全部通过。

- [ ] **Step 5: 最终确认符号清零**

```bash
grep -rln "PanelKind::Acceptance\|extensions::acceptance\|acceptance::Message\|AcceptanceStore\|RecordAcceptance\|GetAcceptanceCount\|AcceptanceCount\|delivery_pending\|ACCEPTED_REF_PREFIX\|project_acceptance_count" --include="*.rs" .
```
预期：零命中。（`docs/superpowers/specs/`、`docs/superpowers/plans/` 下的历史文档不受此 grep 约束，本条只查 `.rs` 源码。）

- [ ] **Step 6: 手动 GUI 走查**

```bash
cargo run -p dozer-app
```
走查：
- 打开一个已存在的项目（此项目此前若用过验收面板，右侧图标栏应已不再出现验收图标——首次启动时 `layout.json`/`panel_layouts.json` 若曾含 `"Acceptance"`，会因反序列化失败整体回落默认布局，这是本次改动已知且接受的一次性副作用，属正常现象，不是 bug）。
- 确认右侧图标栏只剩 Agent/Conversations/Usage 三个图标（加上左侧七个，共十个）。
- 确认项目卡片（首页项目列表）不再显示"N 次验收"这行。
- 开一个 agent 会话，触发一轮 `TurnEnded`（比如跟 Claude Code 说完一句话等它回合结束），确认文件树的 git 改动装饰仍会刷新（验证 Task 1 Step 7 的重构没有丢失这个副作用）。

- [ ] **Step 7: 向用户报告完成**

汇总本次删除涉及的文件数/行数变化，并明确提示：已存量用户下次启动 Dozer 时会因为 `layout.json`/`panel_layouts.json` 反序列化失败而重置窗口尺寸与图标栏排布（一次性、无数据丢失），dozerd 侧 sqlite 库里的旧 `acceptances` 表原样留存但不再被任何代码引用。
