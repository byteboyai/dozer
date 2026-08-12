# `App::update` 大块内联 arm 抽方法 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `App::update`(`app.rs:2332-3847`,113 arm/1516 行)里 39 个超过 15 行内联逻辑的 match arm,按功能聚成 9 组,逐组抽成 `impl App` 上的具名私有方法,match arm 收缩成一行转发。

**Architecture:** 纯代码搬家——每个目标 arm 的函数体原样(逐字节一致)搬进一个新 `fn name(&mut self, <字段>)` 方法,arm 本身改成 `Message::X(..) => self.name(..),`。不新建模块、不改 `Message`/`App` 字段、不碰 15 行以下的 arm。

**Tech Stack:** Rust workspace;不新增依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/app-update-handler-extraction`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `7acdaa4`,已含 tab/icon-button 那份合并)分析**。这个仓库的工作目录被多个并行会话共享——开工前用本文档给出的 `grep -n` 模式核对你实际面对的代码行号,不要假设跟本文档写的完全一致(`app.rs` 的具体行号可能因其它并行分支的改动而漂移,但每个 arm 的**起始 `Message::` 行文本**是稳定的锚点)。
- **不改变任何行为**。每个任务完成后,新方法体必须与原 arm 体逐字节一致(只是从 match 里搬到了 `impl App` 的一个具名方法里,加了 `fn name(&mut self, ...) { }` 这层包装)——不得在搬家过程中"顺手"改动任何一行逻辑。
- **不动 15 行以下的 arm**、**不新建模块**、**不改 `Message` 枚举/`App` struct 字段**——这些都是设计文档明确的非目标。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check` 干净通过。`cargo test` 预期 484 passed / 2 failed(`terminal_grid_state_sizes_hidden_terminal_as_if_shown`/`terminal_pane_pixel_size_right_maximized_matches_overlay_box`,两个与本计划无关的既有失败;若数字变了先停下来查是不是本计划引入的)。
- **每个任务的 `git diff` 必须只表现为"删除一段、别处新增结构相同的一段"**——如果 diff 看起来在改逻辑而不是搬代码,回去核对到逐字节一致再提交。
- 设计文档:`docs/superpowers/specs/2026-08-12-app-update-handler-extraction-design.md`,有疑问以它为准。
- 新方法统一插入到 `impl App` 块内、`update` 方法**之后**、下一个已有私有方法(`tab_drag_move`,当前在 `update` 结束处往下不远)**之前**,按任务顺序依次追加——不打散穿插到无关方法中间,方便审阅时按任务连续定位。

---

### Task 1: 项目页签生命周期(6 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn project_select(&mut self, id: i64)`、`fn project_tab_opened(&mut self, project: Option<ProjectInfo>, recent: Vec<ProjectInfo>)`、`fn project_tab_switch(&mut self, id: i64)`、`fn project_tab_close(&mut self, id: i64)`、`fn project_slot_loaded(&mut self, id: i64, payload: RestorePayload)`、`fn project_fs_changed(&mut self, project_id: ProjectId, relevance: git_watch::Relevance)`(均 `impl App` 私有方法)。

- [ ] **Step 1: 定位 6 个 arm**

```bash
command grep -n "^            Message::ProjectSelect\|^            Message::ProjectTabOpened\|^            Message::ProjectTabSwitch\|^            Message::ProjectTabClose\|^            Message::ProjectSlotLoaded\|^            Message::ProjectFsChanged" crates/dozer-app/src/app.rs
```

预期(基于 `main` `7acdaa4`,若行号有漂移以 `grep` 实际结果为准):`ProjectSelect` L3310、`ProjectTabOpened` L3354、`ProjectTabSwitch` L3403、`ProjectTabClose` L3439、`ProjectSlotLoaded` L3472、`ProjectFsChanged` L3527(`ProjectFsChanged` 的 arm 在 L3562 `Message::GitLog(...)` 之前结束)。

- [ ] **Step 2: 把 6 个 arm 依次改成一行转发**

把每个 arm 的 `{ ...原函数体... }` 整体剪切走,arm 改成:

```rust
            Message::ProjectSelect(id) => self.project_select(id),
            Message::ProjectTabPickFolder => {} // 副作用在 main.rs(rfd 文件夹选择)
            Message::ProjectTabOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent));
                });
            }
            Message::ProjectTabOpened(project, recent) => self.project_tab_opened(project, recent),
            Message::ProjectTabSwitch(id) => self.project_tab_switch(id),
            Message::ProjectTabClose(id) => self.project_tab_close(id),
            Message::ProjectSlotLoaded(id, payload) => self.project_slot_loaded(id, payload),
            Message::ProjectFsChanged(project_id, relevance) => {
                self.project_fs_changed(project_id, relevance)
            }
```

(`ProjectTabPickFolder`/`ProjectTabOpen` 两个 arm 不足 15 行,原样保留在这里只是为了让你确认剪切边界——它们本身不改。)

- [ ] **Step 3: 在 `impl App` 里(`update` 方法之后)新增 6 个方法**

方法体分别是 Step 2 里从对应 arm 剪切走的那段原文,一字不改地贴进来,只加函数签名这层包装:

```rust
    fn project_select(&mut self, id: i64) {
        // 切项目不再通知 daemon:"活跃项目"是 GUI 侧的概念了(P2a
        // Task 1-3 删掉了 SetActiveProject)。
        //
        // 这个项目已经开着页签(`Loaded` 或还没促成的 `Stub`)时,点最近
        // 项目卡片就只是"切到那个页签",走与点页签完全相同的非破坏性
        // 路径——绝不能杀掉任何已有页签的会话(设计文档 §2)。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            self.maximized = None;
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            // 清放大态改变了终端 pane 的像素尺寸,网格必须跟着重算:
            // `terminal_grid_state` 把 `maximized` 算进去,不重算的话
            // PTY 会一直停在放大时的 cols/rows,直到某个无关的几何事件
            // 偶然触发一次重算(最终审查 Required Fix #2)。
            self.sync_terminal_grid();
            self.persist_open_projects();
            return;
        }
        // 还没开着:作为**新页签**打开(与顶栏"＋"同一条 `ProjectTabOpened`
        // 落地路径),而不是把当前页签的内容换掉——多页签下"点一张最近
        // 项目卡片"的直觉是"再开一个",不是"把手上这个换掉"。
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let recent = client.list_projects().await.unwrap_or_default();
            let opened = recent.iter().find(|p| p.id == id).cloned();
            let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent));
        });
    }

    fn project_tab_opened(&mut self, project: Option<ProjectInfo>, recent: Vec<ProjectInfo>) {
        self.recent_projects = recent.clone();
        // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
        // 要求:失败绝不能落进任何 `Workspace`,否则会留下"有界面、没
        // 归属项目"的破状态,用户一点 tab 栏的"＋"就 panic
        // (`spawn_new_tab` 的 expect)。失败文案挂到 App 级的
        // `daemon_error` 上——它不依赖任何 `Workspace` 存在,一个项目
        // 都没打开时空态视图也画得出来(Required Fix #1)。
        let Some(project) = project else {
            tracing::warn!("打开项目页签失败,页签集合保持不变");
            self.daemon_error = Some("打开项目失败,请确认 dozerd 正常后重试".to_string());
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            return;
        };
        self.daemon_error = None;
        // 放大态是外壳态,换页签后留着只会挡住新页签的界面。
        self.maximized = None;
        let id = project.id;
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            // 这个项目已经开着页签了:只前台化,绝不改写它的内容——
            // 那会把这个页签既有的终端全关掉、文件树对话列表全清空重来。
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            self.sync_terminal_grid(); // 清放大态后重算网格,理由见 `ProjectSelect`
            self.persist_open_projects();
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        // 换成新项目的面板布局(它自己没存过就退化成默认)。
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        self.sync_terminal_grid(); // 同上
        self.persist_open_projects();
    }

    fn project_tab_switch(&mut self, id: i64) {
        // 切页签只有两件事:改 `active_project_id`、必要时促成 `Stub`。
        // 没有任何内容改写,因此后台项目的终端/预览/审阅原样留着,切
        // 回来还是刚才那副样子。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if !focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            return;
        }
        self.adopt_panel_layout(id);
        self.maximized = None;
        self.current_page = AppPage::Workspace;
        self.ensure_loaded(id);
        // Git Log 面板已经开着的话,提交图缓存是 `App` 级的、不随项目
        // 页签走(见 `sync_git_log_to_active_project` 文档),不补这一
        // 下切页签会让提交图停在上一个项目,跟同一面板里已经按新项目
        // 刷新的 worktree 速览条对不上。
        if self.left_view == LeftView::GitLog {
            self.sync_git_log_to_active_project();
        }
        // 清放大态后必须重算终端网格。`PaneResized` 那条分支只在**窗口
        // 几何变化**时触发,清 `maximized` 不会自己走到那里;而
        // `terminal_grid_state` 把 `maximized` 算进公式,不重算的话
        // "在项目 A 放大终端 → 切到 B"会让 A 的 PTY 停在放大时的
        // cols/rows(最终审查 Required Fix #2)。重算是幂等的:算出来
        // 与当前 `cols/rows` 相同时 `PaneResized` 的去重会原地返回。
        self.sync_terminal_grid();
        self.persist_open_projects();
        // 按下项目页签＝选中＋准备被拖走(同终端/预览页签)。
        if let Some(idx) = self.project_order.iter().position(|p| *p == id) {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Project,
                source: idx,
            });
        }
    }

    fn project_tab_close(&mut self, id: i64) {
        // 关掉当前页签前先把它的面板布局原样存下(焦点还在它身上,
        // `stash` 会记进 `id` 那份),以后重开还能恢复。
        self.stash_active_panel_layout();
        let io = self.shell_io();
        let Some(slot) = take_project_tab(
            &mut self.projects,
            &mut self.project_order,
            &mut self.active_project_id,
            id,
        ) else {
            return;
        };
        if let WorkspaceSlot::Loaded(mut ws) = slot {
            // 关页签 = 结束该项目下所有会话(abort 转发任务 + kill
            // daemon 侧会话)。不 kill 的话会话会继续在 daemon 上跑,
            // 还会被下次 bootstrap 恢复出来。
            ws.close_all_tabs_for_switch(&io);
        }
        self.maximized = None;
        // 焦点被 `take_project_tab` 挪到了邻居页签上,而那个邻居可能还
        // 是个懒加载 `Stub`——`view()` 走的是只读的 `active_workspace()`,
        // 它**不促成** `Stub`,于是界面会画成"未打开任何项目",尽管顶栏
        // 那个页签明明高亮着。必须在这里显式促成(最终审查 Required
        // Fix #3)。
        if let Some(next) = self.active_project_id {
            // 焦点被挪到了邻居页签,把它的面板布局换上来。
            self.adopt_panel_layout(next);
            self.ensure_loaded(next);
        }
        self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
        self.persist_open_projects();
    }

    fn project_slot_loaded(&mut self, id: i64, payload: RestorePayload) {
        let Some(restore) = payload.take() else {
            return; // 信封已被取走(理论上不会发生),没有素材可落地
        };
        // 只在槽位仍是那份"加载中"占位时落地。两种落空情形:
        // - 页签在促成完成前被用户关掉了(槽位已不存在);
        // - 槽位已经被别的路径换成了真正的内容(比如
        //   `ProjectTabOpened` 的 `adopt_project`)。
        // 两种情形下这份素材都没人要了,但它已经 attach 上了该项目在
        // daemon 上的存活会话——直接 drop 只是断开事件流,daemon 侧
        // 会话仍在跑,会变成"没有任何页签持有、却还占着 PTY"的野会话。
        // 所以按关页签的语义结束掉它们(`ProjectTabClose` 同款处理)。
        // 落地的同时把占位那份 `allowed_files` 句柄接过来:main.rs 的
        // webview 池只在 `active_project_id` **变化**时才清空,它看不见
        // "同一个项目换了一份 `Workspace` 对象"。促成窗口期里用户点开
        // 的文件预览已经建出一个 id 0 的 webview,其 `dozer://` 协议
        // 闭包捕获的是**占位那一个** `Arc`;新 `Workspace` 若另起一个
        // `Arc`,`restore_preview_state` 重开的 id 0 会被
        // `sync_webview_pool` 认成"这个 id 已经有 webview 了"而只调
        // `load_url`,于是文件请求走的还是旧 `Arc` 的白名单 → 对不上
        // → 空白预览。这与 Required Fix #3 是同一个失效模式,只是触发
        // 点从"切项目"变成"促成换对象"。共用同一个 `Arc` 即可,而且
        // 不损失已经建好的 webview(比清空池更省一次导航)。
        let inherited = match self.projects.get(&id) {
            Some(WorkspaceSlot::Loaded(cur)) if cur.loading => Some(cur.allowed_files()),
            _ => None,
        };
        let landed = inherited.is_some();
        if !landed {
            let client = self.client.clone();
            let ids: Vec<String> = restore
                .sessions
                .iter()
                .map(|(info, _, _)| info.id.clone())
                .collect();
            self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        tracing::warn!("丢弃过期促成结果时结束会话失败: {e}");
                    }
                }
            });
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::from_restore(&io, *restore, inherited);
        // 重挂出来的会话,终端模型是按 `DEFAULT_COLS`×`DEFAULT_ROWS`
        // 建的,得按当前窗口几何纠正一次。这里**不能**指望
        // `sync_terminal_grid`:它算出来的网格与 `self.cols/rows` 相同
        // 时 `PaneResized` 会原地返回(去重),于是这份新装配的
        // `Workspace` 会一直停在 80×24。直接对它自己 resize 一次。
        ws.resize_all(&io, io.cols, io.rows);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
    }

    fn project_fs_changed(&mut self, project_id: ProjectId, relevance: git_watch::Relevance) {
        self.with_project(project_id, |ws, io| {
            let Some(project) = &ws.project else { return };
            spawn_project_git_refresh(project_id, PathBuf::from(&project.path), io);
        });
        // 只有 `.git` 引用类变化(分支切换/外部提交/其他 worktree
        // 提交)才值得重建 Git Log 快照——纯工作区文件编辑不影响
        // 提交历史,重算是纯浪费。`git_log_cache` 是 `App` 级、不是
        // 按项目分的(见 `sync_git_log_to_active_project`),所以这里
        // 必须先核实这条事件本来就是"当前聚焦项目"发出的
        // (`project_id == self.active_project_id`)——否则后台项目
        // 的引用变化会拿"缓存路径恰好等于前台项目路径"这个巧合当
        // 通行证,把前台正打开的详情/选中态平白清掉,而其实什么都
        // 没变。项目 id 匹配之外再核一次路径,双保险防状态漂移。
        if relevance == git_watch::Relevance::GitRefs
            && self.active_project_id == Some(project_id)
            && let Some(repo_path) = self.git_log.cache_repo_path().map(|p| p.to_path_buf())
            && self
                .active_workspace()
                .and_then(|ws| ws.active_project_path())
                .as_deref()
                == Some(repo_path.as_path())
        {
            // 引用变化只是要"内容不变、重新拉一遍",窗口大小维持原样——
            // 用 `cache_max_count()`(读当前缓存的 max_count),不是"加载
            // 更多"专用、会 `+LOAD_MORE_STEP` 的 `next_load_more_count()`。
            let max = self.git_log.cache_max_count();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::GitLog(m));
            };
            git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit);
        }
    }
```

- [ ] **Step 4: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异(如有差异先 `cargo fmt -p dozer-app` 再重新 check)。

- [ ] **Step 5: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 484 passed / 2 failed(两个已知的既有失败)。

- [ ] **Step 6: `git diff` 自查**

```bash
git diff -- crates/dozer-app/src/app.rs
```

确认 diff 只有"某处删除一段、别处新增结构相同一段",6 个方法体与原 arm 体除首尾的 `Message::X(..) => {`/`fn name(...) {` 包装行外逐字节相同。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): 项目页签生命周期 6 个 update arm 抽成具名方法

ProjectSelect/ProjectTabOpened/ProjectTabSwitch/ProjectTabClose/
ProjectSlotLoaded/ProjectFsChanged 从 App::update 的 match 里搬进
impl App 的具名私有方法,match arm 收缩成一行转发。纯代码搬家,方法体
与原 arm 体逐字节一致,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Database 转发(4 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn database_test_connection_result(&mut self, project_id: i64, source_id: String, result: Result<(), String>)`、`fn database_tables_loaded(&mut self, project_id: i64, source_id: String, result: Result<Vec<database::TableRef>, String>)`、`fn database_columns_loaded(&mut self, project_id: i64, source_id: String, schema: Option<String>, table: String, result: Result<Vec<database::ColumnInfo>, String>)`、`fn database_message(&mut self, msg: database::Message)`。

- [ ] **Step 1: 定位 4 个 arm**

```bash
command grep -n "^            Message::Database" crates/dozer-app/src/app.rs
```

预期:`TestConnectionResult` L2735(模式跨 4 行,到 `)) => {` 才结束)、`TablesLoaded` L2766、`ColumnsLoaded` L2790(花括号模式,到 `}) => {` 结束)、`ToolbarHover` L2826(8 行,不抽)、通配 `Database(msg)` L2834。

- [ ] **Step 2: 把 4 个目标 arm 改成一行转发**

```rust
            Message::Database(database::Message::TestConnectionResult(
                project_id,
                source_id,
                result,
            )) => self.database_test_connection_result(project_id, source_id, result),
            // schema 树两个异步结果同 `TestConnectionResult` 口径:自带 project_id,
            // 按自带 id 路由,不能用当前聚焦项目。特化分支必须排在通配
            // `Message::Database(msg)` 之前,否则永远匹配不到。
            Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => {
                self.database_tables_loaded(project_id, source_id, result)
            }
            Message::Database(database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            }) => self.database_columns_loaded(project_id, source_id, schema, table, result),
            Message::Database(database::Message::ToolbarHover(target, hovered)) => {
                // 数据库面板 schema 树头部 icon 按钮的悬停:本面板不挂 App 的
                // hover 动画表,把进入/离开转发成 `HoverId` 由内核统一驱动动画。
                let id = match target {
                    database::DatabaseToolbarTarget::SchemaBack => HoverId::DatabaseSchemaBack,
                };
                self.set_hover(id, hovered);
            }
            Message::Database(msg) => self.database_message(msg),
```

(`ToolbarHover` 8 行不抽,原样列出只是确认剪切边界。)

- [ ] **Step 3: 新增 4 个方法**

```rust
    fn database_test_connection_result(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<(), String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TestConnectionResult(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_tables_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<Vec<database::TableRef>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TablesLoaded(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_columns_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<database::ColumnInfo>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            },
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_message(&mut self, msg: database::Message) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            msg,
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }
```

- [ ] **Step 4-6:** 同 Task 1 的 Step 4-6(build+fmt、test、diff 自查)。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): Database 转发 4 个 update arm 抽成具名方法

TestConnectionResult/TablesLoaded/ColumnsLoaded/通配 msg 从
App::update 搬进 impl App 具名私有方法。纯代码搬家,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: SSH 转发(4 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn ssh_test_connection_result(&mut self, project_id: i64, host_id: String, result: Result<(), String>)`、`fn ssh_unknown_key_detected(&mut self, project_id: i64, host_id: String, fingerprint: String, key_bytes: Vec<u8>)`、`fn ssh_key_changed(&mut self, project_id: i64, host_id: String, fingerprint: String)`、`fn ssh_terminal_connect_failed(&mut self, project_id: i64, host_id: String, tab_id: usize, err: String)`。

- [ ] **Step 1: 定位 4 个 arm**

```bash
command grep -n "^            Message::Ssh" crates/dozer-app/src/app.rs
```

预期:`TestConnectionResult` L3687、`UnknownKeyDetected` L3709(模式跨 6 行)、`KeyChanged` L3741、`OpenTerminal` L3768(11 行,不抽)、`TerminalConnectFailed` L3779、通配 `msg` L3803(不抽,15 行以下)。

- [ ] **Step 2: 把 4 个目标 arm 改成一行转发**

```rust
            Message::Ssh(ssh::Message::TestConnectionResult(project_id, host_id, result)) => {
                self.ssh_test_connection_result(project_id, host_id, result)
            }
            Message::Ssh(ssh::Message::UnknownKeyDetected(
                project_id,
                host_id,
                fingerprint,
                key_bytes,
            )) => self.ssh_unknown_key_detected(project_id, host_id, fingerprint, key_bytes),
            Message::Ssh(ssh::Message::KeyChanged(project_id, host_id, fingerprint)) => {
                self.ssh_key_changed(project_id, host_id, fingerprint)
            }
            // 点"终端"按钮:与既有 `TestConnection`/其它同步交互消息不同,
            // 这个消息不走 `ssh::update`(它要新建一个 tab,需要 `&mut
            // Workspace` 整体,`ssh::update` 只拿得到 `&mut ws.ssh`)——
            // 拦截在通配 `Message::Ssh(msg)` 之前,直接调 `Workspace::
            // spawn_ssh_tab`。
            Message::Ssh(ssh::Message::OpenTerminal(host_id)) => {
                self.with_focused_project(|ws, io| {
                    ws.ssh.record_reopen_after_trust(host_id.clone());
                    ws.spawn_ssh_tab(io, host_id);
                });
            }
            // 终端连接失败:先做内核层面的清理(pending/ssh_out_pending
            // 两处暂存——这次连接没能走到 `TabAttached`,不清理会一直占着
            // 这两个 map 的位置),再转给 `ssh::update` 落卡片状态(同
            // `TestConnectionResult` 的路由口径,带显式 project_id,套用
            // 一模一样的 `with_project` 外壳)。
            Message::Ssh(ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err)) => {
                self.ssh_terminal_connect_failed(project_id, host_id, tab_id, err)
            }
```

(`OpenTerminal`、通配 `msg` 两个不抽,原样保留确认边界。)

- [ ] **Step 3: 新增 4 个方法**

```rust
    fn ssh_test_connection_result(&mut self, project_id: i64, host_id: String, result: Result<(), String>) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TestConnectionResult(project_id, host_id, result),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_unknown_key_detected(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
        key_bytes: Vec<u8>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::UnknownKeyDetected(
                    project_id,
                    host_id,
                    fingerprint,
                    key_bytes,
                ),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_key_changed(&mut self, project_id: i64, host_id: String, fingerprint: String) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::KeyChanged(project_id, host_id, fingerprint),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_terminal_connect_failed(
        &mut self,
        project_id: i64,
        host_id: String,
        tab_id: usize,
        err: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            ws.pending.remove(&tab_id);
            ws.ssh_out_pending.remove(&tab_id);
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }
```

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): SSH 转发 4 个 update arm 抽成具名方法

TestConnectionResult/UnknownKeyDetected/KeyChanged/
TerminalConnectFailed 从 App::update 搬进 impl App 具名私有方法。
纯代码搬家,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Acceptance(3 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn acceptance_open(&mut self, tab_id: usize)`、`fn acceptance_reject(&mut self)`、`fn acceptance_result(&mut self, project_id: i64, msg: acceptance::Message)`。

- [ ] **Step 1: 定位 3 个 arm**

```bash
command grep -n "^            Message::Acceptance" crates/dozer-app/src/app.rs
```

预期:`Open` L2484、`Reject` L2503、复合 OR-pattern L2529(`msg @ (Loaded(project_id,..) | DiffLoaded(project_id,..) | Done(project_id,..))`,到 `) => {` 结束于 L2533)、通配 `msg` L2551(不抽)。

- [ ] **Step 2: 把 3 个目标 arm 改成一行转发**

```rust
            Message::Acceptance(acceptance::Message::Open(tab_id)) => self.acceptance_open(tab_id),
            Message::Acceptance(acceptance::Message::Reject) => self.acceptance_reject(),
            Message::Acceptance(
                msg @ (acceptance::Message::Loaded(project_id, ..)
                | acceptance::Message::DiffLoaded(project_id, ..)
                | acceptance::Message::Done(project_id, ..)),
            ) => self.acceptance_result(project_id, msg),
```

OR-pattern 整个保留在 arm 里(它是模式匹配,不是逻辑,不需要搬进方法),只把方法体挪走。

- [ ] **Step 3: 新增 3 个方法**

```rust
    fn acceptance_open(&mut self, tab_id: usize) {
        self.with_focused_project(|ws, io| {
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            let Some(project_id) = ws.project_id() else {
                return;
            };
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            tab.delivery_pending = false;
            let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Acceptance(m));
            };
            acceptance::spawn_open(project_id, tab_id, cwd, &handle, emit);
        });
    }

    fn acceptance_reject(&mut self) {
        self.with_focused_project(|ws, io| {
            let Some(session) = ws.acceptance.session() else {
                return;
            };
            let comment = session.comment().trim().to_string();
            let source = session.source_tab_id();
            let target = ws.tabs.iter().find(|t| t.tab_id == source);
            let Some(tab) = target.filter(|t| t.alive) else {
                // 会话已结束,意见无处可注——留住当前 session,不清空,让用户
                // 看到错误(现有 `acceptance_reject` 的降级路径)。
                ws.acceptance
                    .set_error("会话已结束,意见无处可注".to_string());
                return;
            };
            let id = tab.info.id.clone();
            let client = io.client.clone();
            let text_out = format!("[Dozer 验收打回] {comment}\n");
            io.handle.spawn(async move {
                if let Err(e) = client.write(&id, text_out.as_bytes()).await {
                    tracing::warn!("打回注回失败: {e}");
                }
            });
            ws.acceptance.clear_session();
        });
    }

    fn acceptance_result(&mut self, project_id: i64, msg: acceptance::Message) {
        // 判断"这次是不是通过成功"要在 `msg` 被 `move` 进闭包之前算好
        // (用 `&msg` 引用匹配,不消耗它;闭包里 `acceptance::update` 会真正
        // 拿走 `msg` 的所有权),否则会撞上"用后借用"的编译错误。
        let is_accept_ok = matches!(&msg, acceptance::Message::Done(_, Ok(_)));
        self.with_project(project_id, move |ws, io| {
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Acceptance(m));
            };
            acceptance::update(&mut ws.acceptance, msg, project_id, &client, &handle, emit);
            if is_accept_ok {
                ws.spawn_acceptance_count_refresh(io);
            }
        });
    }
```

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): Acceptance 3 个 update arm 抽成具名方法

Open/Reject/复合结果(Loaded|DiffLoaded|Done)从 App::update 搬进
impl App 具名私有方法,OR-pattern 留在 match arm 里不动。纯代码搬家,
不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Todo(3 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn todo_dispatch_to_existing(&mut self, idx: usize, session_id: String)`、`fn todo_dispatch_new(&mut self, idx: usize, launch: crate::workspace::PickerLaunch)`、`fn todo_message(&mut self, msg: todo::Message)`。

- [ ] **Step 1: 定位 3 个 arm**

```bash
command grep -n "^            Message::Todo" crates/dozer-app/src/app.rs
```

预期:`DispatchToExisting` L2698、`DispatchNew` L2715、通配 `msg` L2861(与前两个不相邻,中间隔着 Database 组的 arm)。

- [ ] **Step 2: 把 3 个 arm 改成一行转发**

```rust
            Message::Todo(todo::Message::DispatchToExisting(idx, session_id)) => {
                self.todo_dispatch_to_existing(idx, session_id)
            }
            Message::Todo(todo::Message::DispatchNew(idx, launch)) => {
                self.todo_dispatch_new(idx, launch)
            }
```

(`DispatchToExisting`/`DispatchNew` 相邻;通配 `Message::Todo(msg)` 在文件里位置不相邻,单独处理:)

```rust
            Message::Todo(msg) => self.todo_message(msg),
```

- [ ] **Step 3: 新增 3 个方法**

```rust
    fn todo_dispatch_to_existing(&mut self, idx: usize, session_id: String) {
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

    fn todo_dispatch_new(&mut self, idx: usize, launch: crate::workspace::PickerLaunch) {
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

    fn todo_message(&mut self, msg: todo::Message) {
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

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): Todo 3 个 update arm 抽成具名方法

DispatchToExisting/DispatchNew/通配 msg 从 App::update 搬进 impl App
具名私有方法。纯代码搬家,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Browser(3 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn browser_bookmarks_loaded(&mut self, pid: i64, bookmarks: Vec<browser::BookmarkInfo>)`、`fn browser_bookmarks_mutated(&mut self, pid: i64, res: Result<(), String>)`、`fn browser_message(&mut self, msg: browser::Message)`。

- [ ] **Step 1: 定位 3 个 arm**

```bash
command grep -n "^            Message::Browser" crates/dozer-app/src/app.rs
```

预期:`BookmarksLoaded` L3245、`BookmarksMutated` L3263、`DragHover` L3281(3 行,不抽)、通配 `msg` L3285。

- [ ] **Step 2: 把 3 个目标 arm 改成一行转发**

```rust
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
                self.browser_bookmarks_loaded(pid, bookmarks)
            }
            Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
                self.browser_bookmarks_mutated(pid, res)
            }
            Message::Browser(browser::Message::DragHover(idx)) => {
                // 浏览器 tab 脱的换位:光标扫过 `idx` 页签 → 走共同换位逻辑。
                self.tab_drag_move(TabGroup::Browser, idx);
            }
            Message::Browser(msg) => self.browser_message(msg),
```

(`DragHover` 不抽,原样列出确认边界。)

- [ ] **Step 3: 新增 3 个方法**

```rust
    fn browser_bookmarks_loaded(&mut self, pid: i64, bookmarks: Vec<browser::BookmarkInfo>) {
        self.with_project(pid, move |ws, io| {
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksLoaded(pid, bookmarks),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_bookmarks_mutated(&mut self, pid: i64, res: Result<(), String>) {
        self.with_project(pid, move |ws, io| {
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksMutated(pid, res),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_message(&mut self, msg: browser::Message) {
        // 按下浏览器页签＝选中＋准备被拖走(`SelectTab` 在
        // `browser::update` 里真正选中为 `active`,这里按它记下拖起源)。
        let was_select = matches!(msg, browser::Message::SelectTab(_));
        self.with_focused_project(|ws, io| {
            let project_id = ws.project.as_ref().map(|p| p.id);
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(&mut ws.browser, msg, project_id, &client, &handle, emit);
        });
        if was_select
            && let Some(ws) = self.active_workspace()
            && ws.browser.active_tab_idx() < ws.browser.tab_count()
        {
            let active = ws.browser.active_tab_idx();
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Browser,
                source: active,
            });
        }
    }
```

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): Browser 3 个 update arm 抽成具名方法

BookmarksLoaded/BookmarksMutated/通配 msg 从 App::update 搬进
impl App 具名私有方法。纯代码搬家,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: 终端 I/O(3 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn term_input(&mut self, bytes: Vec<u8>)`、`fn term_output(&mut self, project_id: ProjectId, tab_id: usize, bytes: Vec<u8>)`、`fn term_paste(&mut self, text: String)`。

- [ ] **Step 1: 定位 3 个 arm**

```bash
command grep -n "^            Message::TermInput\|^            Message::TermOutput\|^            Message::TermPaste" crates/dozer-app/src/app.rs
```

预期:`TermInput` L2334(`update` 函数的第一个 arm)、`TermOutput` L2351、`TermPaste` L3143(与前两个不相邻)。

- [ ] **Step 2: 把 3 个 arm 改成一行转发**

```rust
            Message::TermInput(bytes) => self.term_input(bytes),
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.term_output(project_id, tab_id, bytes)
            }
```

(`TermInput`/`TermOutput` 相邻;`TermPaste` 单独处理:)

```rust
            Message::TermPaste(text) => self.term_paste(text),
```

- [ ] **Step 3: 新增 3 个方法**

```rust
    fn term_input(&mut self, bytes: Vec<u8>) {
        // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
        // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
        // 会话(Fix round 2 #3)。
        if !self.terminal_visible() {
            return;
        }
        self.with_focused_project(|ws, io| {
            // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
            // 实时输出（常规终端语义），再把字节写给 daemon。
            if let Some(tab) = ws.tabs.get_mut(ws.active) {
                tab.model.scroll_to_bottom();
                tab.model.selection_clear();
            }
            ws.send_input(io, bytes);
        });
    }

    fn term_output(&mut self, project_id: ProjectId, tab_id: usize, bytes: Vec<u8>) {
        self.with_project(project_id, |ws, io| {
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            // 实时输出可能含设备查询（DSR/DA 等），应答必须写回 PTY
            // ——atuin/claude 等 TUI 依赖它（此前丢弃导致探测超时）。
            tab.ingest_osc(&bytes);
            let responses = tab.model.feed(&bytes);
            if responses.is_empty() || !tab.alive {
                return;
            }
            match &tab.backend {
                TabBackend::Daemon => {
                    let client = io.client.clone();
                    let id = tab.info.id.clone();
                    io.handle.spawn(async move {
                        if let Err(e) = client.write(&id, &responses).await {
                            tracing::warn!("回写终端查询应答失败: {e}");
                        }
                    });
                }
                TabBackend::Ssh { out } => {
                    let _ = out.send(SshOut::Data(responses));
                }
            }
        });
    }

    fn term_paste(&mut self, text: String) {
        // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
        // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
        // 执行,比单个按键更危险,必须同样拦截。
        if !self.terminal_visible() {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let Some(tab) = ws.tabs.get_mut(ws.active) else {
                return;
            };
            tab.model.scroll_to_bottom();
            let bytes = if tab.model.bracketed_paste() {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            ws.send_input(io, bytes);
        });
    }
```

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): 终端 I/O 3 个 update arm 抽成具名方法

TermInput/TermOutput/TermPaste 从 App::update 搬进 impl App 具名
私有方法。纯代码搬家,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: 预览/图标栏/导航(7 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn select_tab(&mut self, idx: usize)`、`fn pane_resized(&mut self, cols: u16, rows: u16)`、`fn left_icon_select(&mut self, v: LeftView)`、`fn right_icon_select(&mut self, v: RightView)`、`fn top_bar_home(&mut self)`、`fn preview_open_path(&mut self, path: PathBuf)`、`fn preview_select_tab(&mut self, idx: usize)`。

- [ ] **Step 1: 定位 7 个 arm**

```bash
command grep -n "^            Message::SelectTab\|^            Message::PaneResized\|^            Message::LeftIconSelect\|^            Message::RightIconSelect\|^            Message::TopBarHome\|^            Message::PreviewOpenPath\|^            Message::PreviewSelectTab" crates/dozer-app/src/app.rs
```

预期:`SelectTab` L2658、`PaneResized` L2928、`LeftIconSelect` L2968、`RightIconSelect` L3028、`TopBarHome` L3066、`PreviewOpenPath` L3166、`PreviewSelectTab` L3184。

- [ ] **Step 2: 把 7 个 arm 改成一行转发**

```rust
            Message::SelectTab(idx) => self.select_tab(idx),
```

```rust
            Message::PaneResized { cols, rows } => self.pane_resized(cols, rows),
```

```rust
            Message::LeftIconSelect(v) => self.left_icon_select(v),
```

```rust
            Message::RightIconSelect(v) => self.right_icon_select(v),
```

```rust
            Message::TopBarHome => self.top_bar_home(),
```

```rust
            Message::PreviewOpenPath(path) => self.preview_open_path(path),
```

```rust
            Message::PreviewSelectTab(idx) => self.preview_select_tab(idx),
```

(这 7 个在源文件里彼此不相邻,穿插着 `ColumnDragStart`/`Hover`/`MaximizeClose` 等不抽的小 arm——按 Step 1 的行号逐个替换,不要整段搬移周围没抽的 arm。)

- [ ] **Step 3: 新增 7 个方法**

```rust
    fn select_tab(&mut self, idx: usize) {
        // 按下页签＝选中＋准备被拖走:选中仍是唯一的语义,但顺带记下
        // "这一页签正被按住",随后鼠标划过其它页签时 `on_move` 触发
        // `TabDragMove` 完成换位;松开时 main.rs `TabDragEnd` 收尾。
        self.with_focused_project(|ws, _io| {
            if idx < ws.tabs.len() {
                ws.active = idx;
            }
        });
        if let Some(ws) = self.active_workspace()
            && idx < ws.tabs.len()
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
            });
        }
    }

    fn pane_resized(&mut self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 || (cols, rows) == (self.cols, self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let io = self.shell_io();
        // 终端网格是窗口级的:并行打开的每个项目各有一套终端 tab,
        // 但它们共用同一块终端 pane。只改当前项目的话,切回后台项目
        // 会看到一个停在旧网格、和 pane 对不上的画面,直到用户偶然
        // 再拖一次窗口才纠正——所以这里对所有已加载项目一起改
        // (`Stub` 还没有任何 tab,促成时自然按当时的 `io.cols/rows`)。
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.resize_all(&io, cols, rows);
            }
        }
    }

    fn left_icon_select(&mut self, v: LeftView) {
        if self.left_view == v {
            // 点的是已选中(激活)的图标:应退回未选中并收起左面板区。
            // 但若右面板区也已经收起了,左就是最后一个还开着的 zone,
            // 不能关——保持展开、图标维持选中态(什么都不做)。
            if !self.right_collapsed {
                self.left_collapsed = !self.left_collapsed;
            }
        } else {
            self.left_view = v;
            self.left_collapsed = false;
        }
        // 切进 Git 提交图视图时,若缓存为空或不属于当前项目,同步跑
        // 一次 `gleisbau` 布局。失败/未打开项目都落成文案,交给
        // `git_log::view` 画出来,不 panic、不静默吞掉。
        if self.left_view == LeftView::GitLog {
            self.sync_git_log_to_active_project();
        }
        // Todo 面板：切入即从磁盘重读一次 `.dozer/todo.md`，保证切进来
        // 立刻是最新内容（轮询只负责"停留期间"的同步，切换本身不算）。
        if self.left_view == LeftView::Todo {
            self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    todo::reload_from_disk(
                        &mut ws.todo,
                        std::path::Path::new(&project.path),
                    );
                }
            });
        }
        // 数据库面板：切入即从磁盘重读一次 `.dozer/database.json`，
        // 保证切进来立刻是最新内容(同 Todo 面板的切换时语义)。
        if self.left_view == LeftView::Database {
            self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    database::reload_from_disk(
                        &mut ws.database,
                        std::path::Path::new(&project.path),
                    );
                }
            });
        }
        // SSH 面板：切入即从磁盘重读一次 `.dozer/ssh_hosts.json`，语义
        // 同 Todo 面板(切换本身触发重读,停留期间的同步靠别的机制)。
        if self.left_view == LeftView::Ssh {
            self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                }
            });
        }
        // 图标栏点击一律退出放大态。放大态浮层不拦图标栏上的点击
        // (遮罩两侧垫的是无交互 Space,点击穿到下层图标按钮),所以
        // "放大左侧 → 点文件夹图标收起左侧"是可达的:不清 `maximized`
        // 就会留下一个空的金色描边浮层,只能点变暗区才能脱身
        // (Fix round 2 #2)。切换本侧显示什么内容时,放大态本也不该
        // 存活,无条件清最简单也最不容易出意外。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    fn right_icon_select(&mut self, v: RightView) {
        if self.right_view == v {
            // 同上,对称:右是最后开着的 zone 时不收起。
            if !self.left_collapsed {
                self.right_collapsed = !self.right_collapsed;
            }
        } else {
            self.right_view = v;
            self.right_collapsed = false;
            if v == RightView::Usage {
                self.with_focused_project(|ws, io| {
                    ws.usage.set_loading(true);
                    ws.spawn_usage_refresh(io);
                });
            } else if v == RightView::Acceptance {
                let tab_id = self
                    .active_workspace()
                    .and_then(|ws| ws.tabs.get(ws.active))
                    .map(|t| t.tab_id);
                if let Some(tab_id) = tab_id {
                    self.update(Message::Acceptance(acceptance::Message::Open(tab_id)));
                }
            }
        }
        // 同 LeftIconSelect(Fix round 2 #2)。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    fn top_bar_home(&mut self) {
        self.current_page = AppPage::Home;
        self.home_recents_loaded = false;
        self.home_left_view = homespace::HomeLeftView::default();
        self.home_right_view = homespace::HomeRightView::default();
        let projects: Vec<ProjectInfo> =
            self.recent_projects.iter().take(5).cloned().collect();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (files, convs) =
                tokio::task::spawn_blocking(move || load_home_recents(&projects))
                    .await
                    .unwrap_or_default();
            let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
        });
    }

    fn preview_open_path(&mut self, path: PathBuf) {
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.preview.open_path(path);
            // 新 tab 落在末尾，滚回最左让它可见（P1L T5）。
            ws.preview_tab_first = 0;
            ws.spawn_preview_state_save(io);
        });
    }

    fn preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.preview.tabs().len())
            .unwrap_or(false);
        self.with_focused_project(|ws, io| {
            ws.preview.select(idx);
            ws.spawn_preview_state_save(io);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Preview,
                source: idx,
            });
        }
    }
```

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): 预览/图标栏/导航 7 个 update arm 抽成具名方法

SelectTab/PaneResized/LeftIconSelect/RightIconSelect/TopBarHome/
PreviewOpenPath/PreviewSelectTab 从 App::update 搬进 impl App 具名
私有方法。纯代码搬家,不改行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: 杂项核心(6 个 arm)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces:`fn agent_state_changed(&mut self, project_id: ProjectId, tab_id: usize, agent: AgentKind, state: AgentState, transcript_path: Option<String>)`、`fn delivery_checked(&mut self, project_id: ProjectId, tab_id: usize, pending: bool)`、`fn conversation_open(&mut self, path: PathBuf)`、`fn search_results(&mut self, project_id: i64, result: Result<Vec<(String, Vec<search::SearchHit>)>, String>)`、`fn git_log_load_more(&mut self)`、`fn files_project_message(&mut self, project_id: i64, msg: files::Message)`。

- [ ] **Step 1: 定位 6 个 arm**

```bash
command grep -n "^            Message::AgentStateChanged\|^            Message::DeliveryChecked\|^            Message::ConversationOpen\|^            Message::Search(search::Message::SearchResults\|^            Message::GitLog(git_log::Message::LoadMore\|^            Message::Files($" crates/dozer-app/src/app.rs
```

预期:`AgentStateChanged` L2388、`DeliveryChecked` L2456、`ConversationOpen` L2625、`Search(SearchResults)` L2884、`GitLog(LoadMore)` L3566、`Files(project-scoped OR-pattern)` L3620。

- [ ] **Step 2: 把 6 个 arm 改成一行转发**

```rust
            Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
                self.agent_state_changed(project_id, tab_id, agent, state, transcript_path)
            }
```

```rust
            Message::DeliveryChecked(project_id, tab_id, pending) => {
                self.delivery_checked(project_id, tab_id, pending)
            }
```

```rust
            Message::ConversationOpen(path) => self.conversation_open(path),
```

```rust
            Message::Search(search::Message::SearchResults(project_id, result)) => {
                self.search_results(project_id, result)
            }
```

```rust
            Message::GitLog(git_log::Message::LoadMore) => self.git_log_load_more(),
```

```rust
            Message::Files(
                msg @ (files::Message::StatusesRefreshed(project_id, ..)
                | files::Message::PasteDone(project_id, ..)
                | files::Message::OpDone { project_id, .. }),
            ) => self.files_project_message(project_id, msg),
```

(6 个在源文件里彼此不相邻——按 Step 1 的行号逐个替换,`Files` 那个 OR-pattern 模式整个保留在 arm 里不动,同 Task 4 的 Acceptance 处理方式。)

- [ ] **Step 3: 新增 6 个方法**

```rust
    fn agent_state_changed(
        &mut self,
        project_id: ProjectId,
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    ) {
        self.with_project(project_id, |ws, io| {
            // 当前项目路径先取出（下面要 &mut 借 tab，冲突）；重锚:项目优先。
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                if state == AgentState::TurnEnded {
                    // git 检测不许在 UI 线程跑：丢 tokio,结果经 proxy 回来
                    let cwd =
                        effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
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
                            let _ =
                                proxy.send_event(Message::DeliveryChecked(
                                    project_id, tab_id, pending,
                                ));
                        }
                    });
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
                ws.spawn_review_load(io, ReviewSource::Session(tab_id), path, tab_agent);
            }
        });
    }

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
                let cwd =
                    effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                if let Some(repo) = delivery::repo_root(&cwd) {
                    tab.last_turn_head = delivery::head_commit(&repo);
                }
            }
            // 回合结束后刷新项目 git 状态,文件树装饰随之更新（P1h）。
            if let Some(project) = &ws.project {
                spawn_project_git_refresh(project_id, PathBuf::from(&project.path), io);
            }
            // 回合结束后刷新对话列表(transcript 增长/新增；P1j)。
            ws.spawn_conversations_refresh(io);
        });
    }

    fn conversation_open(&mut self, path: PathBuf) {
        self.with_focused_project(move |ws, io| {
            // 若点开的是某活会话的当前对话 → Session 源(回合结束刷新);否则 File 快照。
            let path_s = path.to_string_lossy().into_owned();
            let session_tab = ws
                .tabs
                .iter()
                .find(|t| t.transcript_path.as_deref() == Some(path_s.as_str()));
            // 活会话 tab 的 agent 若还是 Unknown（hook 事件还没到，或
            // 老装的 hook 一直上报 Unknown）不该盖掉从对话历史扫描
            // 位置推断出的已知 agent——优先取“已知”的那个。
            let agent = session_tab
                .map(|t| t.agent)
                .filter(|a| *a != AgentKind::Unknown)
                .or_else(|| {
                    ws.conversations
                        .iter()
                        .find(|c| c.path == path)
                        .map(|c| c.agent)
                })
                .unwrap_or_default();
            let source = session_tab
                .map(|t| ReviewSource::Session(t.tab_id))
                .unwrap_or_else(|| ReviewSource::File(path.clone()));
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                expanded: std::collections::HashSet::new(),
            });
            ws.spawn_review_load(io, source, path_s, agent);
        });
    }

    fn search_results(
        &mut self,
        project_id: i64,
        result: Result<Vec<(String, Vec<search::SearchHit>)>, String>,
    ) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        self.with_project(project_id, move |ws, _io| {
            let emit = move |m| {
                let _ = proxy.send_event(Message::Search(m));
            };
            search::update(
                &mut ws.search,
                search::Message::SearchResults(project_id, result),
                project_id,
                &handle,
                emit,
            );
        });
    }

    fn git_log_load_more(&mut self) {
        let Some(path) = self
            .active_workspace()
            .and_then(|ws| ws.active_project_path())
        else {
            return;
        };
        let next = self.git_log.next_load_more_count();
        // `request_refresh` 内部会把 `selected` 清空,所以必须在调用它之前
        // 先读出来,落地新快照后(`update()` 处理 `SnapshotLoaded` 那支)才能
        // 据此还原选中态——镜像现有 `Message::GitLogLoadMore` 分支"先记
        // selected,刷新,再把 restore_after_load 设回去"的顺序。
        let selected = self.git_log.selected();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::GitLog(m));
        };
        git_log::request_refresh(&mut self.git_log, path, next, &handle, emit);
        self.git_log.set_restore_after_load(selected);
    }

    fn files_project_message(&mut self, project_id: i64, msg: files::Message) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Files(m));
        };
        let app_files = &mut self.files;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
    }
```

- [ ] **Step 4-6:** 同 Task 1。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): 杂项核心 6 个 update arm 抽成具名方法

AgentStateChanged/DeliveryChecked/ConversationOpen/
Search(SearchResults)/GitLog(LoadMore)/Files(项目路由复合模式)从
App::update 搬进 impl App 具名私有方法。纯代码搬家,不改行为。至此
设计文档里 39 个超过 15 行的 arm 全部抽完。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

9 个任务都完成后:

- [ ] `cargo build -p dozer-app --bin dozer` 全量编译无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 仍是 484 passed / 2 failed(与开工前一致)。
- [ ] `cargo fmt -p dozer-app -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets` 没有新增 lint。
- [ ] `command grep -c "^            Message::" crates/dozer-app/src/app.rs` 统计的 match arm 总行数应该明显下降(39 个 arm 每个从"多行块"变成 1-3 行转发,`update` 函数体总行数预计从 1516 行降到 400 行上下——不用精确验证这个数字,只是给个直觉预期)。
- [ ] 提请审阅,通过后合并回 `main`。
