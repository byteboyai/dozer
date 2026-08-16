# Agent 面板卡片化 + LLM/Mode/工作区展示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Agent 域面板(`crates/dozer-app/src/workspace.rs`)的紧凑列表行换成卡片,新增 LLM 型号、permission mode、工作区分支+脏标三项展示。

**Architecture:** 三个只读展示字段(`llm_model`/`permission_mode`/`workspace_override`)挂在 `SessionTab` 上,靠既有的 hook 事件推送链路(`App::agent_state_changed`)驱动刷新——每次 hook 事件都异步重读一次 transcript(仅 Claude)、按需查一次 git(仅 cwd 偏离项目根时),结果经新 `Message::AgentCardRefreshed` 写回。不新建轮询/文件监听子系统,不碰 `dozer-core`/`dozerd`/`dozer-client`。

**Tech Stack:** Rust, iced 0.14(GUI),`serde_json`(transcript 解析),tokio(`spawn_blocking` 跑 IO)。

**Spec:** `docs/superpowers/specs/2026-08-16-agent-card-workspace-design.md`

## Global Constraints

- 在独立分支(`feature/agent-card-workspace`)上开发,**不要直接提交到
  `main`**——`main` 上有另一个长驻 agent(WorkBuddy)在并行自主开发,直接在
  `main` 上改会跟它的未提交改动混在一起,审阅时无法干净区分。完成后提请
  审阅,通过后再合并回 `main`。
- `SessionTab` 已有一个字段叫 `model: TerminalModel`(终端显示缓冲区)。新
  加的 LLM 型号字段**必须**叫 `llm_model`,不能叫 `model`——同名会直接
  编译报错。
- 新增字段/函数一律 `pub(crate)`(除非本计划明确写 `pub`),不引入新的对外
  API 面。
- 每个任务结束跑一次 `cargo test -p dozer-app --bin dozer <关键字>` 确认
  新测试通过,任务全部完成后在 Task 8 跑一次全量
  `cargo build && cargo test -p dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt`。
- 中文注释风格、`pub(crate) fn xxx(&self) -> bool` 这类访问器命名,均沿用
  本文件/本 crate 现有约定,不引入新风格。

---

### Task 1: `transcript::latest_model_and_mode`

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs`(在 `parse_transcript`/
  `parse_claude_shaped_jsonl` 之后、`#[cfg(test)] mod tests {` 之前插入新
  函数,约在现有第 189 行之后)
- Test: `crates/dozer-app/src/transcript.rs` 的 `mod tests`(现有测试从第
  195 行开始,新测试加在文件最后一个 `#[test]` 之后、`mod tests` 闭合 `}`
  之前)

**Interfaces:**
- Consumes: 无(纯函数,只依赖已有的 `serde_json::Value`,该 crate 顶部已
  有 `use serde_json::Value;`)
- Produces: `pub fn latest_model_and_mode(jsonl: &str) -> (Option<String>, Option<String>)`
  ——供 Task 5 的 `Workspace::spawn_agent_card_refresh` 调用。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozer-app/src/transcript.rs` 的 `mod tests` 块内、最后一个
`#[test]` 函数之后加入:

```rust
    #[test]
    fn latest_model_and_mode_picks_last_occurrence() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"改一下 README"},"permissionMode":"plan"}
{"type":"assistant","message":{"role":"assistant","content":[],"model":"claude-sonnet-5"}}
{"type":"permission-mode","permissionMode":"auto"}
{"type":"assistant","message":{"role":"assistant","content":[],"model":"claude-opus-5"}}
"#;
        let (model, mode) = latest_model_and_mode(jsonl);
        assert_eq!(model.as_deref(), Some("claude-opus-5"));
        assert_eq!(mode.as_deref(), Some("auto"));
    }

    #[test]
    fn latest_model_and_mode_skips_noise_and_bad_lines() {
        let jsonl = "{\"type\":\"mode\",\"mode\":\"normal\"}\n不是 json 的坏行\n{\"type\":\"attachment\",\"attachment\":{}}\n";
        assert_eq!(latest_model_and_mode(jsonl), (None, None));
    }

    #[test]
    fn latest_model_and_mode_empty_input_yields_none() {
        assert_eq!(latest_model_and_mode(""), (None, None));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer transcript::tests::latest_model_and_mode -- --nocapture`
Expected: 编译失败,`latest_model_and_mode` 未定义。

- [ ] **Step 3: 写最小实现**

在 `crates/dozer-app/src/transcript.rs` 里,紧跟在 `parse_transcript`
函数(现有代码约在第 170-189 行)之后、`#[cfg(test)]` 之前插入:

```rust
/// 从 transcript 尾部提取最后一次出现的 model id / permissionMode(后
/// 出现的覆盖先出现的,只关心最新值)。`permissionMode` 在
/// `user`/`assistant`/`permission-mode` 三种行的顶层都会出现,统一按
/// 顶层键取,不区分行类型。解析失败的行跳过,不中断整体扫描(同
/// `parse_claude_shaped_jsonl` 的既有容错口径)。不像 `parse_transcript`
/// 那样建 `ReviewEntry` 列表,只回两个标量,给 Agent 卡片的实时刷新用
/// (每次 hook 事件都会重跑一次,故意做得比 `parse_transcript` 轻)。
pub fn latest_model_and_mode(jsonl: &str) -> (Option<String>, Option<String>) {
    let mut model = None;
    let mut mode = None;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(m) = v
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|s| s.as_str())
        {
            model = Some(m.to_string());
        }
        if let Some(pm) = v.get("permissionMode").and_then(|s| s.as_str()) {
            mode = Some(pm.to_string());
        }
    }
    (model, mode)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer transcript::tests::latest_model_and_mode -- --nocapture`
Expected: 3 个新测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "feat(dozer-app): 新增 transcript::latest_model_and_mode 提取 model/permissionMode"
```

---

### Task 2: `workspace::format_model_label`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(加在 `agent_state_label`/
  `dot_color` 附近,现有第 3099-3120 行一带)
- Test: `crates/dozer-app/src/workspace.rs` 的 `mod tests`(现有
  `dot_color_states`/`agent_state_label_covers_all` 测试在第 3501-3527
  行附近,新测试加在旁边)

**Interfaces:**
- Consumes: 无
- Produces: `pub(crate) fn format_model_label(raw: &str) -> String`——供
  Task 7 的 `agent_card` 视图函数调用。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozer-app/src/workspace.rs` 的 `mod tests` 块内,
`agent_state_label_covers_all` 测试函数之后加入:

```rust
    #[test]
    fn format_model_label_strips_claude_prefix_and_titlecases() {
        assert_eq!(format_model_label("claude-sonnet-5"), "Sonnet 5");
        assert_eq!(format_model_label("claude-opus-5"), "Opus 5");
        assert_eq!(format_model_label("claude-haiku-4-5"), "Haiku 4 5");
    }

    #[test]
    fn format_model_label_unknown_shape_returns_verbatim() {
        assert_eq!(format_model_label("gpt-4"), "gpt-4");
        assert_eq!(format_model_label(""), "");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::format_model_label -- --nocapture`
Expected: 编译失败,`format_model_label` 未定义。

- [ ] **Step 3: 写最小实现**

在 `crates/dozer-app/src/workspace.rs` 里,紧跟在 `dot_color` 函数(现有
代码约在第 3111-3120 行)之后插入:

```rust
/// `claude-sonnet-5` → `Sonnet 5`:去掉 `claude-` 前缀,按 `-` 分词、每
/// 词首字母大写、空格拼接。不以 `claude-` 开头的原样返回(不确定形状,
/// 不强行摘,避免拍出乱码;不维护会过期的型号对照表)。
pub(crate) fn format_model_label(raw: &str) -> String {
    match raw.strip_prefix("claude-") {
        Some(rest) => rest
            .split('-')
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => raw.to_string(),
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::format_model_label -- --nocapture`
Expected: 2 个新测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): 新增 format_model_label 美化 model id 展示"
```

---

### Task 3: `SessionTab` 新增字段 + `WorkspaceGitInfo`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:208-241`(`SessionTab`
  结构体定义)
- Modify: `crates/dozer-app/src/workspace.rs:437-452`(第一处
  `SessionTab { .. }` 构造,`from_restore` 里)
- Modify: `crates/dozer-app/src/workspace.rs:1646-1664`(第二处构造)
- Modify: `crates/dozer-app/src/workspace.rs:3530-3556`(测试辅助函数
  `make_test_tab`)

**Interfaces:**
- Consumes: 无
- Produces: `SessionTab.llm_model: Option<String>`、
  `SessionTab.permission_mode: Option<String>`、
  `SessionTab.workspace_override: Option<WorkspaceGitInfo>`,
  `pub(crate) struct WorkspaceGitInfo { pub branch: Option<String>, pub dirty: bool }`
  ——供 Task 5(写入)、Task 6(路由写入)、Task 7(视图读取)使用。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozer-app/src/workspace.rs` 的 `mod tests` 块内,紧跟
`make_test_tab` 函数之后加入(用 `make_test_tab` 构造一个 tab,断言三个
新字段默认值为 `None`——这个测试目前会因为字段不存在而编译失败,是本
task 的失败态):

```rust
    #[test]
    fn new_session_tab_fields_default_to_none() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tab = make_test_tab(&rt, "s1", dozer_core::protocol::AgentKind::Claude);
        assert_eq!(tab.llm_model, None);
        assert_eq!(tab.permission_mode, None);
        assert_eq!(tab.workspace_override, None);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::new_session_tab_fields_default_to_none -- --nocapture`
Expected: 编译失败,`SessionTab` 没有 `llm_model`/`permission_mode`/
`workspace_override` 字段。

- [ ] **Step 3: 加字段 + 改三处构造**

在 `crates/dozer-app/src/workspace.rs:208-241` 的 `SessionTab` 定义里,
`last_turn_head: Option<String>,` 字段之后、`tab_id` 字段之前插入:

```rust
    /// 最近一次 hook 事件后从 transcript 尾部提取的模型 id(原始,未美化;
    /// 渲染时经 `format_model_label`)。仅 `agent == AgentKind::Claude` 会
    /// 被填充,其余 agent 恒 `None`。不叫 `model`——上面已有一个
    /// `model: TerminalModel` 字段是终端显示缓冲区,同名会编译报错。
    pub llm_model: Option<String>,
    /// 同上,来自 transcript 顶层 `permissionMode`(如 `auto`/`plan`)。
    pub permission_mode: Option<String>,
    /// 工作区分支/脏标覆盖:仅当这个会话的 `effective_cwd()` 偏离项目根
    /// 目录时才会被填充;为 `None` 时渲染层直接读 `ws.project_panel` 的
    /// 项目级缓存(见 `Workspace::spawn_agent_card_refresh`)。
    pub workspace_override: Option<WorkspaceGitInfo>,
```

在 `SessionTab` 结构体定义**之前**(紧挨着,同一处)加入新类型:

```rust
/// 单个会话的工作区展示态,来自 `delivery::branch`/`delivery::is_dirty`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceGitInfo {
    pub branch: Option<String>,
    pub dirty: bool,
}
```

第一处构造(`crates/dozer-app/src/workspace.rs:437-452`),
`last_turn_head: None,` 之后加三行:

```rust
                last_turn_head: None,
                llm_model: None,
                permission_mode: None,
                workspace_override: None,
                backend: TabBackend::Daemon,
```

第二处构造(`crates/dozer-app/src/workspace.rs:1646-1664`),同样在
`last_turn_head: None,` 之后加:

```rust
            last_turn_head: None,
            llm_model: None,
            permission_mode: None,
            workspace_override: None,
            backend: match ssh_backend {
                Some(out) => TabBackend::Ssh { out },
                None => TabBackend::Daemon,
            },
```

测试辅助函数(`crates/dozer-app/src/workspace.rs:3530-3556` 的
`make_test_tab`),同样在 `last_turn_head: None,` 之后加:

```rust
            last_turn_head: None,
            llm_model: None,
            permission_mode: None,
            workspace_override: None,
            tab_id: 0,
            forwarder: rt.spawn(async {}),
            backend: TabBackend::Daemon,
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo build -p dozer-app 2>&1 | tail -30`(先确认整个 crate 能编译
——改结构体字段会波及所有构造点)
Expected: 编译成功,无 "missing field" 报错。

Run: `cargo test -p dozer-app --bin dozer workspace::tests::new_session_tab_fields_default_to_none -- --nocapture`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): SessionTab 新增 llm_model/permission_mode/workspace_override 字段"
```

---

### Task 4: `extensions::project::WorkspaceState` 新增 `branch()`/`dirty()` 访问器

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`(`WorkspaceState`
  的 `impl` 块,紧邻 `branch: Option<String>`/`dirty: bool` 字段定义
  ——现有第 17-18 行一带,`impl WorkspaceState` 块参考现有 `worktrees()`
  访问器,现有第 60-62 行附近)

**Interfaces:**
- Consumes: 无
- Produces: `pub(crate) fn branch(&self) -> Option<&str>`、
  `pub(crate) fn dirty(&self) -> bool`——供 Task 7 的 `agent_card` 视图
  函数在 `workspace_override` 为 `None` 时读取项目级缓存。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozer-app/src/extensions/project.rs` 的 `mod tests` 块内,紧
跟既有的 `git_refreshed_updates_four_fields` 测试(现有第 833-857 行)
之后加入,复用同一批测试 helper(`new_ws`/`test_repo_path`/
`test_client`,均已在 `mod tests` 顶部定义):

```rust
    #[test]
    fn branch_and_dirty_accessors_read_current_state() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(1, Some("main".to_string()), true, vec![], vec![]),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.branch(), Some("main"));
        assert!(ws.dirty());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer extensions::project::tests::branch_and_dirty_accessors -- --nocapture`
Expected: 编译失败,`branch()`/`dirty()` 方法不存在。

- [ ] **Step 3: 写最小实现**

在 `crates/dozer-app/src/extensions/project.rs` 的 `impl WorkspaceState`
块内(参考现有 `pub fn worktrees(&self) -> &[WorktreeInfo]` 访问器旁边)
加入:

```rust
    /// 项目级分支名(Project 面板/Agent 卡片工作区行共用读口)。
    pub(crate) fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// 项目级脏标(同上)。
    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer extensions::project::tests::branch_and_dirty_accessors -- --nocapture`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs
git commit -m "feat(dozer-app): project::WorkspaceState 新增 branch()/dirty() 只读访问器"
```

---

### Task 5: `Workspace::spawn_agent_card_refresh`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(新增两个函数,放在
  `spawn_review_load` 附近,现有第 969-990 行一带——`agent_card_refresh_plan`
  是 `impl Workspace` 块外的模块级自由函数,`spawn_agent_card_refresh` 是
  `impl Workspace` 块内的方法,紧跟 `spawn_review_load` 之后)

**Interfaces:**
- Consumes: `transcript::latest_model_and_mode`(Task 1)、
  `WorkspaceGitInfo`(Task 3)、`delivery::repo_root`/`delivery::branch`/
  `delivery::is_dirty`(已存在,`crates/dozer-app/src/delivery.rs:31,374,41`)、
  `Message::AgentCardRefreshed`(Task 6 定义,本 task 先引用其构造,Task 6
  再真正加这个变体——两个 task 顺序因此不能颠倒,必须先做本 task 里的纯
  决策函数,再在 Task 6 补 `Message` 变体后才能让整个文件重新编译通过;
  若按顺序执行到本 task 结尾时 `cargo build` 会因为 `Message::
  AgentCardRefreshed` 还不存在而失败,这是预期的中间态,Task 6 结束后才
  会恢复绿)。
- Produces: `pub(crate) fn agent_card_refresh_plan(agent: AgentKind, cwd: &Path, project_root: Option<&Path>) -> (bool, bool)`
  (纯函数,供本 task 自测)、`pub(crate) fn spawn_agent_card_refresh(&self, io: &ShellIo, tab_id: usize, agent: AgentKind, transcript_path: Option<String>, cwd: PathBuf, project_root: Option<PathBuf>)`
  ——供 Task 6 在 `agent_state_changed` 里调用。

- [ ] **Step 1: 写失败的测试(纯决策函数)**

在 `crates/dozer-app/src/workspace.rs` 的 `mod tests` 块内加入:

```rust
    #[test]
    fn agent_card_refresh_plan_decides_by_agent_and_cwd() {
        let root = PathBuf::from("/repo");
        let elsewhere = PathBuf::from("/elsewhere");

        assert_eq!(
            agent_card_refresh_plan(dozer_core::protocol::AgentKind::Claude, &root, Some(&root)),
            (true, false),
            "Claude + cwd 等于项目根:只做 model/mode"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Codebuddy,
                &root,
                Some(&root)
            ),
            (false, false),
            "非 Claude + cwd 等于项目根:两者都不做"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Codebuddy,
                &elsewhere,
                Some(&root)
            ),
            (false, true),
            "非 Claude + cwd 偏离项目根:只做工作区"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Claude,
                &elsewhere,
                Some(&root)
            ),
            (true, true),
            "Claude + cwd 偏离项目根:两者都做"
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::agent_card_refresh_plan -- --nocapture`
Expected: 编译失败,`agent_card_refresh_plan` 未定义。

- [ ] **Step 3: 写最小实现**

先在 `crates/dozer-app/src/workspace.rs` 里的 `impl Workspace` 块**外**
(模块级自由函数,不是 `Workspace` 方法——同 `group_tabs_by_agent` 的既有
写法,这样测试才能不经 `Workspace::` 前缀直接调用)加入:

```rust
/// 决定这次 hook 事件驱动的卡片刷新要不要做 model/mode 提取、要不要
/// 单独查一次工作区 git 信息。抽成纯函数只为可测——
/// `Workspace::spawn_agent_card_refresh` 里直接调用,不重复判断逻辑。
pub(crate) fn agent_card_refresh_plan(
    agent: AgentKind,
    cwd: &Path,
    project_root: Option<&Path>,
) -> (bool, bool) {
    let needs_model_mode = agent == AgentKind::Claude;
    let needs_workspace = project_root != Some(cwd);
    (needs_model_mode, needs_workspace)
}
```

再紧跟在 `spawn_review_load` 方法(现有第 969-990 行,仍在 `impl
Workspace` 块**内**)之后插入:

```rust
    /// hook 事件驱动的"卡片元信息"刷新(仿 `spawn_review_load` 的写法):
    /// model/permissionMode(仅 Claude,从 transcript 尾部轻量提取,见
    /// `transcript::latest_model_and_mode`)+ 工作区覆盖(仅当该 session
    /// 的 cwd 偏离项目根目录时才查;常见情形直接复用 `project_panel` 的
    /// 项目级缓存,这里不产生任何 IO)。两者都不需要时直接返回,不起
    /// 异步任务。
    pub(crate) fn spawn_agent_card_refresh(
        &self,
        io: &ShellIo,
        tab_id: usize,
        agent: AgentKind,
        transcript_path: Option<String>,
        cwd: PathBuf,
        project_root: Option<PathBuf>,
    ) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let (needs_model_mode, needs_workspace) =
            agent_card_refresh_plan(agent, &cwd, project_root.as_deref());
        if !needs_model_mode && !needs_workspace {
            return;
        }
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let (llm_model, mode, workspace) = tokio::task::spawn_blocking(move || {
                let (llm_model, mode) = if needs_model_mode {
                    transcript_path
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .map(|s| transcript::latest_model_and_mode(&s))
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };
                let workspace = if needs_workspace {
                    delivery::repo_root(&cwd).map(|repo| WorkspaceGitInfo {
                        branch: delivery::branch(&repo),
                        dirty: delivery::is_dirty(&repo),
                    })
                } else {
                    None
                };
                (llm_model, mode, workspace)
            })
            .await
            .unwrap_or((None, None, None));
            let _ = proxy.send_event(Message::AgentCardRefreshed(
                project_id, tab_id, llm_model, mode, workspace,
            ));
        });
    }
```

- [ ] **Step 4: 跑测试确认通过(纯函数部分)**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::agent_card_refresh_plan -- --nocapture`
Expected: PASS(该测试只依赖纯函数,不依赖 `Message::AgentCardRefreshed`
是否已定义;但整个 crate 的 `cargo build` 此时会因为
`Message::AgentCardRefreshed` 还不存在而失败——这是预期状态,`cargo
test` 单跑这个测试目标本身若因整包编译失败也跑不起来,所以这一步如果
连编译都过不了属正常,直接进入 Task 6 补上 `Message` 变体后再统一验证
一次即可,不要在本 task 卡住去单独修补编译)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Workspace::spawn_agent_card_refresh 提取 llm_model/mode/工作区"
```

---

### Task 6: `Message::AgentCardRefreshed` + 路由 + 触发调用

设计上把"落地逻辑"和"App 派发"拆开:落地逻辑(`apply_agent_card_refresh`)
是个只操作 `&mut [SessionTab]` 的自由函数,放在 `workspace.rs`,不需要
构造完整 `App`/`Workspace` 就能单测(同 `group_tabs_by_agent(tabs: &[SessionTab])`
的既有写法);`App::update` 里的分支只是一行转发,不单独写测试——同现有
`Message::DeliveryChecked(project_id, tab_id, pending) => self.delivery_checked(project_id, tab_id, pending)`
这类一行转发分支的既有惯例(转发本身没有分支逻辑,不值得为它单独搭
`App` 级测试夹具)。

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(新增
  `apply_agent_card_refresh` 自由函数,放在 Task 5 的
  `spawn_agent_card_refresh` 附近,同一个 `impl Workspace` 块外、模块级)
- Modify: `crates/dozer-app/src/app.rs:1201`(`Message` 枚举,紧邻
  `DeliveryChecked` 变体)
- Modify: `crates/dozer-app/src/app.rs:3023-3025`(`App::update` 里
  `DeliveryChecked` 分支旁边加新分支)
- Modify: `crates/dozer-app/src/app.rs:4836-4907`(`agent_state_changed`
  函数体内,`tab.transcript_path` 更新之后)

**Interfaces:**
- Consumes: `Workspace::spawn_agent_card_refresh`(Task 5)、
  `WorkspaceGitInfo`/`SessionTab`(Task 3)。
- Produces: `pub(crate) fn apply_agent_card_refresh(tabs: &mut [SessionTab], tab_id: usize, llm_model: Option<String>, mode: Option<String>, workspace: Option<WorkspaceGitInfo>)`
  (workspace.rs,供 app.rs 调用)、
  `Message::AgentCardRefreshed(ProjectId, usize, Option<String>, Option<String>, Option<WorkspaceGitInfo>)`
  ——完整数据闭环,供 Task 7 视图层间接消费(通过读 `SessionTab` 字段)。

- [ ] **Step 1: 写失败的测试(`apply_agent_card_refresh`)**

在 `crates/dozer-app/src/workspace.rs` 的 `mod tests` 块内,紧跟 Task 5
加的 `agent_card_refresh_plan_decides_by_agent_and_cwd` 测试之后加入:

```rust
    #[test]
    fn apply_agent_card_refresh_sets_fields_only_when_some() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![make_test_tab(&rt, "a", AgentKind::Claude)];
        tabs[0].tab_id = 7;

        apply_agent_card_refresh(
            &mut tabs,
            7,
            Some("claude-sonnet-5".to_string()),
            Some("auto".to_string()),
            None,
        );
        assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(tabs[0].permission_mode.as_deref(), Some("auto"));
        assert_eq!(tabs[0].workspace_override, None);

        // 第二次刷新 model/mode 都是 None(比如那次 transcript 读取
        // 失败):不应该把已经拿到的值抹掉。
        apply_agent_card_refresh(&mut tabs, 7, None, None, None);
        assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(tabs[0].permission_mode.as_deref(), Some("auto"));

        // 未知 tab_id:整体 no-op,不 panic。
        apply_agent_card_refresh(&mut tabs, 999, Some("x".to_string()), None, None);
        assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::apply_agent_card_refresh -- --nocapture`
Expected: 编译失败,`apply_agent_card_refresh` 未定义。

- [ ] **Step 3: 写最小实现(`workspace.rs`)**

紧跟在 Task 5 加的 `spawn_agent_card_refresh` 方法之后(仍在 `impl
Workspace` 块内的话要挪到块外,写成模块级自由函数——不是 `Workspace`
方法,因为它只需要 `&mut [SessionTab]`,不需要整个 `Workspace`)插入:

```rust
/// `Message::AgentCardRefreshed` 落地:在 `tabs` 里找 `tab_id`,`None`
/// 字段表示这次没有新值,不覆盖已有值(每次刷新只重新扫描"当前"
/// transcript 内容,理论上不会无中生有变回 `None`,这里的保护针对
/// transcript 读取失败等异常情形,不让卡片从"有值"闪回"无值")。tab
/// 不存在(已关闭)时整体 no-op,不 panic。只依赖 `&mut [SessionTab]`
/// 不依赖整个 `Workspace`,同 `group_tabs_by_agent` 的既有写法,方便
/// 直接单测。
pub(crate) fn apply_agent_card_refresh(
    tabs: &mut [SessionTab],
    tab_id: usize,
    llm_model: Option<String>,
    mode: Option<String>,
    workspace: Option<WorkspaceGitInfo>,
) {
    let Some(tab) = tabs.iter_mut().find(|t| t.tab_id == tab_id) else {
        return;
    };
    if llm_model.is_some() {
        tab.llm_model = llm_model;
    }
    if mode.is_some() {
        tab.permission_mode = mode;
    }
    if workspace.is_some() {
        tab.workspace_override = workspace;
    }
}
```

- [ ] **Step 4: 跑测试确认通过(`apply_agent_card_refresh`)**

Run: `cargo test -p dozer-app --bin dozer workspace::tests::apply_agent_card_refresh -- --nocapture`
Expected: PASS。

- [ ] **Step 5: 加 `Message` 变体 + 路由 + 触发调用(`app.rs`)**

在 `crates/dozer-app/src/app.rs:1201` 的 `DeliveryChecked(ProjectId,
usize, bool),` 变体之后插入:

```rust
    /// hook 事件驱动的 Agent 卡片元信息刷新结果(tab_id, LLM 型号,
    /// permission mode,工作区分支/脏标覆盖)。`None` 字段表示这次没有
    /// 新值,落地时不覆盖已有值(见 `workspace::apply_agent_card_refresh`)。
    AgentCardRefreshed(
        ProjectId,
        usize,
        Option<String>,
        Option<String>,
        Option<crate::workspace::WorkspaceGitInfo>,
    ),
```

在 `crates/dozer-app/src/app.rs:3023-3025` 的 `Message::DeliveryChecked`
分支之后插入:

```rust
            Message::AgentCardRefreshed(project_id, tab_id, llm_model, mode, workspace) => {
                self.with_project(project_id, |ws, _io| {
                    workspace::apply_agent_card_refresh(
                        &mut ws.tabs,
                        tab_id,
                        llm_model,
                        mode,
                        workspace,
                    );
                });
            }
```

在 `crates/dozer-app/src/app.rs` 的 `agent_state_changed` 函数
(第 4836-4907 行)内,`if let Some(tab) = ws.tab_by_id_mut(tab_id) { ... }`
块内部、`tracing::info!(tab_id, ?state, "agent 状态变更");` 这行之后
(不进任何 `if state == ...` 条件分支里,保证每次调用都会触发)插入:

```rust
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    tab.transcript_path.clone(),
                    tab.effective_cwd(),
                    active_repo.clone(),
                );
```

> 注:`tab`/`io`/`active_repo` 都是 `agent_state_changed` 函数体内已有
> 的绑定(`tab` 来自这个 `if let Some(tab) = ws.tab_by_id_mut(tab_id)`
> 块本身,`active_repo` 是函数开头 `let active_repo = ws.project.
> as_ref().map(...)`,`io` 是 `with_project` 闭包的第二个参数)——插入
> 位置必须在这个 `if let` 块内部,不能挪到外面(外面拿不到 `tab`)。

- [ ] **Step 6: 跑测试确认通过(整体)**

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 编译成功(Task 5 结尾时因 `Message::AgentCardRefreshed` 还不
存在导致的编译失败,到这里应该恢复)。

Run: `cargo test -p dozer-app --bin dozer workspace:: transcript:: -- --nocapture 2>&1 | tail -60`
Expected: `workspace`/`transcript` 模块下全部测试(含 Task 1-6 新增的)
PASS。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Message::AgentCardRefreshed 路由 + agent_state_changed 触发卡片刷新"
```

---

### Task 7: 视图:`agent_list_row` → `agent_card`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:2201-2236`(`agent_list_row`
  函数,改名/重写为 `agent_card`)
- Modify: `crates/dozer-app/src/workspace.rs:2176-2183`
  (`agent_list_pane` 内调用点)

**Interfaces:**
- Consumes: `format_model_label`(Task 2)、`SessionTab.llm_model`/
  `.permission_mode`/`.workspace_override`(Task 3)、
  `extensions::project::WorkspaceState::branch()`/`dirty()`(Task 4)、
  既有 `tab_title`/`dot_color`/`agent_state_label`
  (workspace.rs:3059,3099,3111)。
- Produces: `pub(crate) fn agent_card(ws: &Workspace, idx: usize) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>`
  ——替换 `agent_list_pane` 里原来的 `agent_list_row` 调用。视图函数本身
  不写单测(iced 视图函数在本 crate 里都是人工验收,不做快照测试),本
  task 靠 Step 1-2 的编译检查 + Task 8 的人工验收覆盖。

- [ ] **Step 1: 替换 `agent_list_row` 为 `agent_card`**

把 `crates/dozer-app/src/workspace.rs:2197-2236` 的整个 `agent_list_row`
函数(含其上的文档注释)替换成:

```rust
/// Agent 面板里单条会话卡片:agent 名 → LLM 行(仅 Claude 且已解析到值
/// 时显示)→ Mode 行(同上条件)→ 工作区行(分支名 + 脏标,所有 agent
/// 都显示)→ 状态点 + 状态文字。整卡可点选中该 tab(`idx == ws.active`
/// 时 `theme::color::CARD` 背景高亮,同项目树选中行的手法)。
pub(crate) fn agent_card(
    ws: &Workspace,
    idx: usize,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;

    let mut lines = column![
        text(tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name))
            .size(theme::font::body())
            .color(theme::color::CREAM),
    ]
    .spacing(4);

    if let Some(llm_model) = &tab.llm_model {
        lines = lines.push(labeled_row("LLM", &format_model_label(llm_model)));
    }
    if let Some(mode) = &tab.permission_mode {
        lines = lines.push(labeled_row("Mode", mode));
    }

    let (branch, dirty) = match &tab.workspace_override {
        Some(w) => (w.branch.as_deref(), w.dirty),
        None => (ws.project_panel.branch(), ws.project_panel.dirty()),
    };
    lines = lines.push(workspace_row(branch, dirty));

    lines = lines.push(
        row![
            text("●")
                .size(theme::font::caption())
                .color(dot_color(tab.agent_state, tab.alive)),
            text(agent_state_label(tab.agent_state))
                .size(theme::font::caption_sm())
                .color(theme::color::DIM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    button(container(lines).padding(10))
        .on_press(Message::SelectTab(idx))
        .width(Length::Fill)
        .style(move |_t, _s| button::Style {
            background: if active {
                Some(theme::color::CARD.into())
            } else {
                None
            },
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: theme::color::CREAM,
            ..button::Style::default()
        })
        .into()
}

/// `label: value` 一行 caption 文字,LLM/Mode/工作区三行共用。
fn labeled_row(
    label: &str,
    value: &str,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    text(format!("{label}: {value}"))
        .size(theme::font::caption())
        .color(theme::color::DIM)
        .into()
}

/// 工作区行:无分支(非 git 项目)显示 `—`;有未提交改动时分支名后缀
/// `(Uncommitted)`——跟 `extensions/files.rs` 里分支切换菜单当前分支带
/// 脏标时的既有文案(`n.push_str("(Uncommitted)")`,见该文件约第 1409
/// 行)保持同一措辞,不新造一套脏标文案。
fn workspace_row(
    branch: Option<&str>,
    dirty: bool,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let value = match branch {
        Some(b) if dirty => format!("{b}(Uncommitted)"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    };
    labeled_row("工作区", &value)
}
```

- [ ] **Step 2: 改调用点**

在 `crates/dozer-app/src/workspace.rs:2176-2183` 的 `agent_list_pane`
函数内,把:

```rust
            for idx in idxs {
                content = content.push(agent_list_row(ws, idx));
            }
```

改成:

```rust
            for idx in idxs {
                content = content.push(agent_card(ws, idx));
            }
```

同时更新 `agent_list_pane` 函数上方的文档注释(现有第 2149-2151 行,提到
"点击一行 = `Message::SelectTab`"的措辞不用改,但如果注释里出现
"agent_list_row"字样,同步改成"agent_card")。

- [ ] **Step 3: 跑编译 + 已有单测确认没有回归**

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 编译成功,无 `agent_list_row` 未定义/未使用的报错。

Run: `cargo test -p dozer-app --bin dozer workspace:: -- --nocapture 2>&1 | tail -60`
Expected: `workspace` 模块下全部既有测试(含 Task 1-6 新增的)PASS,无
新增失败。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Agent 面板紧凑列表行改卡片,展示 LLM/Mode/工作区"
```

---

### Task 8: 全量验证 + 人工验收

**Files:** 无新增/修改文件,纯验证。

**Interfaces:** 无。

- [ ] **Step 1: 全量构建**

Run: `cargo build 2>&1 | tail -40`
Expected: 整个 workspace 编译成功,无新增 warning(已知的两个既有
warning——`format_todo_time`/`write_goal` 未使用——不算新增,若看到别的
新 warning 要处理掉)。

- [ ] **Step 2: 全量测试**

Run: `cargo test -p dozer-app --bin dozer 2>&1 | tail -60`
Expected: 全部测试 PASS,数量比改动前多至少 10 个(Task 1: 3,Task 2: 2,
Task 3: 1,Task 4: 1,Task 5: 1,Task 6: 1 —— 至少这 9 个新测试,外加可能
在核实 helper 签名过程中补的测试)。

- [ ] **Step 3: clippy + fmt**

Run: `cargo clippy --all-targets 2>&1 | grep -E "^error"`
Expected: 空输出(无新增 error;既有 warning 允许保留,但不能有本次改动
引入的新 warning——若 clippy 提示新代码里有 `dead_code`/`unused` 之类,
必须处理,不能留着)。

Run: `cargo fmt -p dozer-app && git diff --stat`
Expected: `cargo fmt` 后若有额外 diff,检查是不是本次改动的代码被重新
格式化(是的话属正常,`git add` 进最后一次 commit 或单独提一次格式化
commit)。

- [ ] **Step 4: 启动 app 做人工验收**

Run: `cargo run -p dozer-app`(或用 `run` 技能,若项目里配置了专门的启动
方式)

人工验收清单(逐项确认):

1. 打开一个 Claude 会话,首轮回复完成后,Agent 面板对应卡片出现 LLM 行
   (如 `LLM: Sonnet 5`)和 Mode 行(如 `Mode: auto`)。
2. 中途用 Shift+Tab(或 Claude Code 对应快捷键)切换 permission mode,
   再来一轮对话后,卡片上的 Mode 值跟着变。
3. 在该会话的 shell 里 `cd` 到另一个 git 仓库,发一条消息触发 hook 事件
   后,该卡片的工作区行分支名跟着变,且这个变化不影响同一项目下其它
   agent 卡片的工作区行(它们仍显示项目根的分支)。
4. 在项目根仓库里造一处未提交改动(比如 `echo x >> README.md`),确认
   工作区行分支名后缀出现 `(Uncommitted)`;`git checkout -- README.md`
   撤销后(或提交掉)确认后缀消失(等下一次 hook 事件触发刷新,或手动
   触发一次交付检测)。
5. 打开一个 Codebuddy 或 Opencode 会话,确认卡片**没有** LLM/Mode 行,
   但工作区行正常显示。
6. 在一个非 git 目录开的项目(如果测试环境允许)里打开 agent,确认工作区
   行显示 `—`,不报错、不崩溃。
7. 分组标题(如 `claude(1)`)依然正确显示分组内数量,点击卡片依然能
   切换到对应终端 tab(`Message::SelectTab` 行为不变)。

- [ ] **Step 5: 记录人工验收结果,准备提审**

若第 4 步清单全部通过,在 PR/审阅说明里注明"人工验收清单 1-7 项已过";
若某项不通过,回到对应 Task 定位问题、修复、重新走一遍该 Task 的测试
循环,不要跳过失败项直接进入审阅。
