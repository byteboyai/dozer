# 群聊面板 · GUI（PanelKind::GroupChat + webview）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Dozer 里新增「群聊」面板（lucide `square-sparkles` 图标，默认右栏、排在「代理」之后）：Preact webview 渲染群切换、成员条、消息流、带 `@` 补全的输入框，以及新建群 / 成员 / 转待办对话框；Rust 侧经 `dozer-client` 驱动已合并的群聊后端。

**Architecture:** 单列整面板 webview（固定单槽，复用 Code Health / Todo 的宿主接线：ID 偏移、声明式推送 `WebviewPushState`、IPC 路由、几何、资源路由、失败占位页）。状态在 `extensions/group_chat`：`WorkspaceState` + 纯函数 `update`（返回 `Effect`，便于单测）+ `protocol.rs`（把状态算成 payload、校验 webview 事件）。后端更新靠 `about_to_wait` 定时唤醒做 `rev` 增量轮询（仿 `poll_todo_if_visible`），只在面板可见且有发言进行中时才轮询。

**Tech Stack:** Rust 2024、iced 0.14、wry；前端 Preact 10 + TypeScript + esbuild（离线打包，无 CDN，运行时无 Node），`node --test` 单测，`preact-render-to-string` 渲染冒烟。

**Spec:** `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`
**Backend plan（已合并）:** `docs/superpowers/plans/2026-10-02-group-chat-backend.md`

## 对 spec 的五处细化（Rulings，执行前请知悉）

> **2026-10-03 修订（重要，覆盖下方 Ruling 1/2）**：用户裁决群聊面板改为**两栏**——左侧聊天详情 webview + 右侧**原生**群列表（选择群、新建群=内联文本输入、删除群=内联二次确认）。切群/新建群/删除群不再走 webview 对话框；成员条、消息流、输入框、添加/编辑成员、转为待办仍留在 webview。落点：`PanelDims` 新增 `group_chat_split`/`group_chat_list_collapsed`；新增 `Divider::GroupChatSplit`；`pair_split_ratio(GroupChat)` 改返回 `Some`；`webview_geometry::group_chat_content_pane_bounds_for` 加 `list_visible`；新增 `extensions::group_chat::view::list_pane`；`webview/group-chat-content` 移除 `GroupBar` 与其 `create_group`/`select_group`/`delete_group` 出站事件。**下文 Ruling 1/2 即被本修订覆盖，仅作历史记录保留。** spec §9 已同步。

1. **对话框全部做成 webview 内的 DOM 对话框**（原含新建群、添加/编辑成员、删除群确认、转为待办，不用原生独立窗口）。spec 第 9 节写的是"走标准对话框独立窗口机制"。改因：①这些对话框全是文本输入，原生输入要接一整套焦点桥接（见 memory"原生输入框 Stage 2–6"那一串工作），webview 里天然具备；②Todo webview 已有"五个弹层做 DOM"的先例；③`app_modal_open` 时 webview 本来就会被隐藏，不存在遮挡问题。代价：提交失败时对话框已关闭，错误走 Toast（符合 spec 11 节"一次性事件走 Toast"）；成员 handle 的明显错误在前端先校验（见 Task 8）。**执行完 Task 11 同步改 spec。**（2026-10-03：新建群/删除群已改原生列表内联，仅剩添加/编辑成员、转为待办留在 webview。）
2. **（已被 2026-10-03 修订覆盖）**原设计为单列整宽 webview，没有原生列表列、没有分隔线、不新增 `PanelDims` 字段。几何复用 `PairPane`，`list_visible: false`（用量面板"无筛选栏时内容独占整宽"已有同款分支）；`pair_split_ratio(GroupChat)` 返回 `None`。**现行实现改为两栏（见上方修订）。**
3. **轮询而不是每群起循环任务。** 在 `window_events.rs` 的 `wakes` 数组里加一行（仿 Todo 轮询），`App::poll_group_chat_if_active` 自限速。理由：天然随项目切换/面板隐藏停止，无泄漏任务。
4. **Markdown 用一个自写的最小安全子集渲染器**（标题、段落、有序/无序列表、粗体、斜体、行内代码、围栏代码块、换行），**先整体转义再套样式**，原始 HTML 与链接一律以纯文本显示。agent 输出不可信，且引入 `marked` 等库需要先核实其 API 与转义行为，第一版不值得。代码块用系统等宽（`ui-monospace`）：webview host 的 CSP 没有 `font-src`，加载 JetBrains Mono 要扩 CSP 并拷 ttf，**这与 CLAUDE.md"只有 code editor 和 pty 用 JetBrains Mono"的字体裁决有出入，需要你定**：默认按"代码块算代码场景但先用系统等宽"实现，若要 JetBrains Mono 另开一个小任务。
5. **图标要先在 `byteboyai/byteui` 仓库发版。** `IconKind::SquareSparkles` 不存在，byteui 是独立仓库、dozer 按 git tag 引用（CLAUDE.md）。Task 0 里改 byteui、发 `v0.4.1`，**推送 tag 是对外动作，必须停下来由用户确认**。

## Global Constraints

- **独立 worktree 分支，不在 main 上提交**：`git worktree add .worktrees/group-chat-ui -b feat/group-chat-ui main`（旧的 `.worktrees/group-chat` 是后端分支，已合并，**不要动**）。主工作区常有他人未提交改动（版本号递增等）和并发提交，**不要碰、不要 stash**；每次 `git add`/`git commit` 前先 `git branch --show-current` 确认是 `feat/group-chat-ui`，只 `git add` 具体路径。用 subagent 时 dispatch 第一句必须是 `cd <worktree 绝对路径> && git branch --show-current`，Read/Edit 的 `file_path` 带完整 worktree 绝对路径前缀。
- **提交结尾附** `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- **GUI 只用 iced 0.14 生态；WebView 恒在 iced 之上。** 本面板内容区没有任何 iced 浮层；失败占位页是 webview 未挂载时的原生壳。
- **新增 icon/tab 类 UI 复用统一组件**（`icons::icon_button_entry`/`tabs::tab_core`）：本面板只在图标栏出现一个入口，已由 `chrome/rail.rs` 的通用渲染承担，**不手写 `MouseArea` 接线**。
- **字体**：webview 内一律系统默认字体（`-apple-system, "PingFang SC", sans-serif`）；仅代码块用等宽（见 Ruling 4）。中文渲染由浏览器负责。
- **颜色**：ByteBoy2077 令牌，CSS 变量取值与 `web/todo-content/src/styles.css` 完全一致（含 `light` 方案，主题由 URL `?theme=` 注入）。**金色只给甲方（human）动作**：human 消息、发送按钮、选中的群；agent 成员用青/绿/紫/橙区分。状态不只靠颜色，保留文字标签。
- **瞬时失败统一走 Toast**（extension 的 `update` 往自己 state 的 `outbox` 里 `push`，`App::drain_outboxes` 排空），不在内容区自画；`daemon_unavailable` 由顶栏徽标展示，不在面板里画。日志来源名 `group_chat`，`dozer_core::scope!(pub(crate) LOG, panel, "group_chat")`；禁止裸 `tracing::*!`/`eprintln!`（`scripts/check-log-scope.sh`）；**不记录消息正文、群主题、角色设定**，只记 id、状态、错误类别。
- **核心原则**：群聊消息只读展示，不提供编辑/删除单条消息的入口。
- **webview 内容不可信**：所有事件在 Rust 侧校验长度、id 存在性、agent 种类、枚举取值（`route_event`）；agent 输出走转义渲染器。
- 核心不依赖 Node/Python：前端只在**构建**时用 Node，产物（`assets/group-chat-content/`）提交进仓库，运行时不需要。
- 新增 ≥7 个同类型参数相邻的函数用具名字段结构体（CLAUDE.md）。

## Review Focus

spec 隐含、但没有任何 Task 的主测试专门覆盖的失败模式，最可能伤到真实使用，按可能性排序；每条在对应 Task 里都有钉住它的测试：

1. **升级后用户的图标栏布局被重置**：`sanitize_rail_layout` 现在要求"恰 11 个面板"，加了第 12 个后，**所有已保存过布局的老用户都会被判为坏数据而整体回落默认**，丢掉自定义的换栏。必须迁移：旧布局（11 个、不含群聊）把群聊插到「代理」之后。→ Task 1。
2. **中文输入法回车误发送**：IME 组词时按 Enter 是确认候选词，不是发送。`isComposing`/`keyCode === 229` 必须拦。→ Task 9。
3. **轮询结果乱序/重复覆盖新状态**：后到的旧 poll 结果不得把已更新的消息退回旧状态（按 `rev` 比较），**也不得用 `Posted` 的 rev 推进 `latest_rev`**（会永久漏掉 rev 更小、尚未取回的变更）。→ Task 2。
4. **切换项目页签/切群时，上一个群/项目的异步结果落到当前群**：所有异步消息带 `project_id`，群相关的再带 `group_id` 并与当前选中核对。→ Task 2。
5. **agent 输出里的 HTML/脚本**：`<img onerror=…>`、`<script>`、`javascript:` 链接必须以纯文本显示，不执行。→ Task 7。

---

## File Structure

| 文件 | 职责 |
|---|---|
| `byteui`（独立仓库）`src/interaction/icons.rs`、`assets/icons/square-sparkles.svg` | 新图标 `IconKind::SquareSparkles`（Task 0） |
| `crates/dozer-app/Cargo.toml` | byteui tag 升到 `v0.4.1` |
| `crates/dozer-app/src/app/state.rs`、`chrome/rail.rs`、`app/layout.rs`、`webview_geometry.rs`、`app/update.rs`、`app/view.rs` | `PanelKind::GroupChat` 接入（Task 1、5） |
| `crates/dozer-app/src/extensions/group_chat/mod.rs`（新） | `WorkspaceState`、`Message`、纯函数 `update`、`Effect`、异步 spawn 函数 |
| `crates/dozer-app/src/extensions/group_chat/protocol.rs`（新） | payload 类型、`current_view_payload`、webview 事件解析与 `route_event`、`WebviewPushState` |
| `crates/dozer-app/src/extensions/group_chat/view.rs`（新） | 原生壳（webview 未挂载 / 失败占位页） |
| `crates/dozer-app/src/app/app.rs`、`message.rs`、`workspace/state.rs`、`runtime.rs`、`platform/window_events.rs`、`assets.rs` | 宿主接线：ID 偏移、状态字段、消息、IPC、推送、轮询唤醒、资源路由 |
| `crates/dozer-app/web/group-chat-content/`（新） | Preact 前端源码、测试、构建脚本 |
| `crates/dozer-app/assets/group-chat-content/`（新，提交产物） | `host.html`、`group-chat-content.js`、`group-chat-content.css` |
| `.gitignore` | 忽略 `web/group-chat-content/node_modules/` |

---

### Task 0: worktree、基线、byteui 图标发版

> byteui 是独立仓库（`/Users/chrischiang/Projects/CoralProjects/byteboy/byteui`，远端 `byteboyai/byteui`）。**推送分支与 tag 是对外动作：执行到 Step 6 必须停下，把改动摘要告诉用户并等明确同意，不要自行 `git push`。**

**Files:**
- Create（byteui）: `assets/icons/square-sparkles.svg`
- Modify（byteui）: `src/interaction/icons.rs`、`CHANGELOG.md`、`Cargo.toml`
- Modify（dozer）: `crates/dozer-app/Cargo.toml`、`Cargo.lock`

- [ ] **Step 1: dozer 隔离 worktree 与基线**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add .worktrees/group-chat-ui -b feat/group-chat-ui main
cd .worktrees/group-chat-ui && git branch --show-current   # 必须是 feat/group-chat-ui
cargo build -p dozer-app 2>&1 | tail -3
cargo test -p dozer-app 2>&1 | tail -8
```

记录基线（通过/失败数）。主工作区若有与本次无关的既有失败，在汇报里对比，不要顺手修。

- [ ] **Step 2: byteui 开分支**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/byteui
git status --short            # 只应有 ?? .dozer/（已被忽略的杂项）;有别的改动先停下问用户
git checkout -b feat/square-sparkles-icon
git tag -l v0.4.1             # 必须为空;已存在则停下问用户
```

- [ ] **Step 3: 取 lucide 的 SVG（直接下载，不要手抄）**

```bash
curl -fsSL https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/square-sparkles.svg \
  -o assets/icons/square-sparkles.svg
head -c 400 assets/icons/square-sparkles.svg
```

期望：以 `<svg` 开头，含 `stroke="currentColor"` 与 `viewBox="0 0 24 24"`，与 `assets/icons/square-activity.svg` 同款描边风格。不符合就停下。

- [ ] **Step 4: 注册图标**

`src/interaction/icons.rs`：在 `SquareActivity,` 变体之后加（沿用该枚举里 Lucide 图标的注释写法）：

```rust
    /// 群聊面板 rail 图标(Lucide square-sparkles:方框 + 星芒,表多 agent 协作)。
    SquareSparkles,
```

在 `bytes()` 的 match 里 `IconKind::SquareActivity => …` 之后加：

```rust
            IconKind::SquareSparkles => include_bytes!("../../assets/icons/square-sparkles.svg"),
```

若 `preserves_original_color` 里对 Lucide 描边图标有显式列举（`SquareActivity` 出现的其他 match），把 `SquareSparkles` 一并加到同一分支。用 `grep -n "SquareActivity" src/interaction/icons.rs` 核对**每一处**，两处以外的出现也要同步。

- [ ] **Step 5: 测试 + 版本 + CHANGELOG + 提交**

在 `icons.rs` 的 `mod tests` 末尾加：

```rust
    #[test]
    fn square_sparkles_icon_has_svg_bytes() {
        let bytes = IconKind::SquareSparkles.bytes();
        let text = std::str::from_utf8(bytes).expect("svg 是 utf8");
        assert!(text.contains("<svg"), "应是 svg 文本");
        assert!(text.contains("currentColor"), "Lucide 描边图标用 currentColor");
        assert!(!IconKind::SquareSparkles.preserves_original_color());
    }
```

```bash
cargo test 2>&1 | tail -8
```

`Cargo.toml` 的 `version = "0.4.0"` 改 `"0.4.1"`；`CHANGELOG.md` 最上面加：

```markdown
## 0.4.1

- 新增 `IconKind::SquareSparkles`（Lucide square-sparkles），供 dozer 群聊面板 rail 图标使用。
```

```bash
git add assets/icons/square-sparkles.svg src/interaction/icons.rs CHANGELOG.md Cargo.toml
git commit -m "$(cat <<'EOF'
feat(icons): add SquareSparkles (0.4.1)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 6: 【停】请用户确认后推送并打 tag**

向用户汇报：byteui 本地已提交 `feat/square-sparkles-icon`，需要 `git push origin feat/square-sparkles-icon`、合并进 `main` 并 `git tag v0.4.1 && git push origin v0.4.1`。**等用户明确同意再执行**（或由用户自己推送）。tag 在远端存在之后才进入 Step 7。

- [ ] **Step 7: dozer 升级依赖**

在 dozer worktree：`crates/dozer-app/Cargo.toml` 里 `byteui = { git = "...", tag = "v0.4.0" }` 改 `v0.4.1`，然后：

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/group-chat-ui
git branch --show-current
cargo update -p byteui 2>&1 | tail -3
cargo build -p dozer-app 2>&1 | tail -3
git add crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore(deps): bump byteui to v0.4.1 (SquareSparkles icon)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

> 本地联调（byteui 还没推时）可在 worktree 里临时建 `.cargo/config.toml`：`[patch."https://github.com/byteboyai/byteui"] byteui = { path = "../../../byteui" }`，**不提交**（CLAUDE.md）。

---

### Task 1: `PanelKind::GroupChat` 与图标栏（含旧布局迁移）

**Files:**
- Modify: `crates/dozer-app/src/app/state.rs`、`chrome/rail.rs`、`app/layout.rs`、`webview_geometry.rs`、`app/update.rs`、`app/view.rs`

**Interfaces:**
- Produces: `PanelKind::GroupChat`（默认右栏，`RailLayout::default()` 右栏为 `[Agent, GroupChat, Conversations, Usage, CodeHealth]`）；日志面板名 `"group_chat"`；`rail::migrate_legacy_rail(RailLayout) -> RailLayout`（供 `sanitize_rail_layout` 调用，`pub(crate)`）。

- [ ] **Step 1: 写失败的测试（rail.rs 的 `mod tests`）**

把现有 `rail_layout_default_covers_all_panels_without_duplicates` 改为：

```rust
    #[test]
    fn rail_layout_default_covers_all_panels_without_duplicates() {
        let rail = RailLayout::default();
        assert_eq!(rail.left.len(), 7);
        assert_eq!(rail.right.len(), 5);
        let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
        all.sort_by_key(|k| format!("{k:?}"));
        all.dedup();
        assert_eq!(all.len(), 12, "12 个面板不重不漏分到左右两栏");
    }

    #[test]
    fn group_chat_sits_right_after_agent_by_default() {
        let rail = RailLayout::default();
        let i = rail.right.iter().position(|k| *k == PanelKind::Agent).unwrap();
        assert_eq!(rail.right[i + 1], PanelKind::GroupChat);
        assert_eq!(rail.side_of(PanelKind::GroupChat), Side::Right);
        assert_eq!(PanelKind::GroupChat.default_side(), Side::Right);
    }
```

并在其后加：

```rust
    /// Review Focus 1:升级前保存的 11 面板布局不得被判成坏数据整体重置。
    #[test]
    fn legacy_eleven_panel_layout_gets_group_chat_inserted_after_agent() {
        let legacy = RailLayout {
            left: vec![
                PanelKind::Project,
                PanelKind::Todo,
                PanelKind::Files,
                PanelKind::GitLog,
                PanelKind::Database,
                PanelKind::Ssh,
                PanelKind::Web,
            ],
            right: vec![
                PanelKind::Agent,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::CodeHealth,
            ],
        };
        let out = sanitize_rail_layout(legacy);
        assert_eq!(out, RailLayout::default());
    }

    #[test]
    fn legacy_customized_layout_keeps_customization() {
        // 用户把 Agent 拖到了左栏、Files 拖到了右栏。
        let legacy = RailLayout {
            left: vec![
                PanelKind::Project,
                PanelKind::Agent,
                PanelKind::Todo,
                PanelKind::GitLog,
                PanelKind::Database,
                PanelKind::Ssh,
                PanelKind::Web,
            ],
            right: vec![
                PanelKind::Files,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::CodeHealth,
            ],
        };
        let out = sanitize_rail_layout(legacy.clone());
        assert_eq!(out.left[1], PanelKind::Agent);
        assert_eq!(out.left[2], PanelKind::GroupChat, "跟在 Agent 后面,同一栏");
        assert_eq!(out.right, legacy.right, "其它栏不动");
        assert_eq!(out.left.len() + out.right.len(), 12);
    }

    #[test]
    fn already_twelve_panel_layout_is_untouched() {
        let rail = RailLayout::default();
        assert_eq!(sanitize_rail_layout(rail.clone()), rail);
    }

    #[test]
    fn eleven_panels_with_duplicate_still_falls_back_to_default() {
        let bad = RailLayout {
            left: vec![PanelKind::Files, PanelKind::Files],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(bad), RailLayout::default());
    }

    #[test]
    fn group_chat_has_rail_meta() {
        let (_icon, tip) = panel_meta(PanelKind::GroupChat);
        assert_eq!(tip, "群聊");
    }
```

同时把现有 `sanitize_rail_layout_falls_back_to_default_on_bad_data` 里注释"面板数不是 10"保持不动（逻辑未变），并在 `state.rs` 的 `log_name_tests` 里把 `ALL_KINDS: [PanelKind; 11]` 改成 `[PanelKind; 12]`、加入 `PanelKind::GroupChat`，`expected_log_name` 加 `PanelKind::GroupChat => "group_chat",`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app rail 2>&1 | tail -20`
Expected: 编译失败，`no variant named GroupChat`。

- [ ] **Step 3: 实现**

`app/state.rs`：

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
    GroupChat,
    Conversations,
    Usage,
    CodeHealth,
}
```

`default_side`：`Self::Agent | Self::GroupChat | Self::Conversations | Self::Usage | Self::CodeHealth => Side::Right,`。枚举注释里的"10 个面板"改"12 个"。

`chrome/rail.rs`：

```rust
            right: vec![
                PanelKind::Agent,
                PanelKind::GroupChat,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::CodeHealth,
            ],
```

`panel_meta` 加：`PanelKind::GroupChat => (icons::IconKind::SquareSparkles, "群聊"),`（放在 `Agent` 之后）。

`sanitize_rail_layout` 改为：

```rust
/// 旧版(没有群聊面板)落盘的布局迁移:合计恰为其余 11 个不重复面板、且不含
/// `GroupChat` 时,把 `GroupChat` 插到 `Agent` 之后(同一栏)。找不到 `Agent`
/// (理论上不会,上面的"恰 11 个不重复"已排除)就追加到右栏末尾。不满足条件
/// 的原样返回,交给后面的常规校验判定坏数据。
///
/// 不迁移的后果:升级后所有存过布局的用户都会被"恰 12 个"校验判为坏数据,
/// 整体回落默认,丢掉自定义的换栏。
pub(crate) fn migrate_legacy_rail(mut rail: RailLayout) -> RailLayout {
    let has_group_chat = rail
        .left
        .iter()
        .chain(rail.right.iter())
        .any(|k| *k == PanelKind::GroupChat);
    if has_group_chat {
        return rail;
    }
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    if all.len() != 11 || rail.left.len() + rail.right.len() != 11 {
        return rail;
    }
    for side in [Side::Left, Side::Right] {
        let panels = rail.side_mut(side);
        if let Some(i) = panels.iter().position(|k| *k == PanelKind::Agent) {
            panels.insert(i + 1, PanelKind::GroupChat);
            return rail;
        }
    }
    rail.right.push(PanelKind::GroupChat);
    rail
}

/// `RailLayout` 的消毒:先做旧布局迁移;然后任一栏为空,或两侧合计不是恰 12 个
/// 不重复的 `PanelKind`(手改/版本不一致导致的坏数据),整个回落 `default()`。
/// 不做部分修复——缺一个面板就补在默认栏这种中间态比"直接用默认值"更难排查
/// (旧版缺群聊的迁移是唯一例外,见 `migrate_legacy_rail`)。
pub(crate) fn sanitize_rail_layout(rail: RailLayout) -> RailLayout {
    let rail = migrate_legacy_rail(rail);
    if rail.left.is_empty() || rail.right.is_empty() {
        return RailLayout::default();
    }
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    if all.len() != 12 || rail.left.len() + rail.right.len() != 12 {
        return RailLayout::default();
    }
    rail
}
```

`side_mut` 若不是 `pub(crate)` 且在同模块，直接可用（上文 `rail_drag_move_into` 已在同文件使用 `rail.side_mut(side)`）。其余注释里的"11 个"改"12 个"（`rail.rs` 文件头、`RailLayout` 文档、`panel_meta` 文档）；`app/layout.rs` 的 `sanitize_shell_layout` 文档里"恰 11 个"改"恰 12 个（旧版 11 个会先迁移）"。

`app/layout.rs`：`pair_split_ratio` 加 `PanelKind::GroupChat => None,`；`with_pair_split_ratio` 加 `PanelKind::GroupChat => dims,`。

`webview_geometry.rs`：两处"纯 iced / 无 webview 可摆"的零矩形分支（`PanelKind::Agent | PanelKind::Usage | PanelKind::CodeHealth` 那两处）都加上 `| PanelKind::GroupChat`（它们是"某一侧在显示的是别的面板、此函数求的是 Files/Project 预览矩形"的分支，群聊不在其列，**保持零矩形**；群聊自己的矩形在 Task 5 的专用函数里）。把 `for kind in [PanelKind::Agent, PanelKind::Usage, PanelKind::CodeHealth]` 的测试数组（约 1242 行）以及下面 `PanelKind::CodeHealth,` 的列表（约 1273 行）里补 `PanelKind::GroupChat`。

`app/update.rs` 的 `panel_select` 里 `PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}` 改为 `PanelKind::Files | PanelKind::Web | PanelKind::Agent | PanelKind::GroupChat => {}`（Task 5 会替换为真正的加载动作）。

`app/view.rs`：在面板主体的 `match`（`PanelKind::CodeHealth => {…}` 之后）加占位分支，Task 5 替换：

```rust
        PanelKind::GroupChat => {
            // Task 5 替换为 `group_chat::content_pane`。
            let c = byteui::theme::color::current();
            iced_widget::container(iced_widget::text("群聊").size(14).color(c.body))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(16)
                .style(move |_t: &iced_widget::Theme| container::Style {
                    background: Some(byteui::theme::color::current().panel.into()),
                    border: zone_pane_border(zone, lc),
                    ..container::Style::default()
                })
                .into()
        }
```

（`zone`、`lc` 是该 `match` 所在函数里已有的绑定，见 `PanelKind::CodeHealth` 分支；`container` 的引入沿用该文件已有的 `use`，若缺则补 `iced_widget::container`。）

然后由编译器带路：

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error|-->" | head -40`

每个 `non-exhaustive patterns: PanelKind::GroupChat not covered` 的 `match`，按下面原则补臂（**不要**为此改其它逻辑）：能取 `_` 的地方不变；必须穷举的地方，语义与 `Agent` 的"无专属行为"一致。已知需要处理的位置已在上文列全，编译器若报出别处，在汇报里列出来。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app rail log_name geometry 2>&1 | tail -20`
Expected: 全部 PASS（`log_names_are_unique_and_valid_scope_names` 也要过）。

- [ ] **Step 5: 手动冒烟（可选，本机有显示环境时）**

`cargo run -p dozer-app`：右栏图标栏在「代理」下方出现新图标；点击后内容区显示"群聊"占位文字；拖拽该图标换栏正常。

- [ ] **Step 6: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src
git commit -m "$(cat <<'EOF'
feat(app): add PanelKind::GroupChat with rail entry and legacy layout migration

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 扩展状态与纯函数 `update`

**Files:**
- Create: `crates/dozer-app/src/extensions/group_chat/mod.rs`
- Modify: `crates/dozer-app/src/extensions.rs`（加 `pub mod group_chat;`，按字母序放在 `footbar` 与 `git_log` 之间）

**Interfaces:**
- Consumes: `dozer_core::protocol::{AgentKind, GroupCancelScope, GroupInfo, GroupMessageInfo, GroupMessageStatus}`、`crate::extensions::toast::{Level, Outbox, Pending}`。
- Produces（Task 3、4、5 依赖，名字与签名不得改）：

```rust
pub struct WorkspaceState { .. }   // Default
impl WorkspaceState {
    pub fn groups(&self) -> &[GroupInfo];
    pub fn selected(&self) -> Option<i64>;
    pub fn messages(&self) -> &[GroupMessageInfo];
    pub fn latest_rev(&self) -> i64;
    pub fn loaded(&self) -> bool;                          // 是否曾经加载过(payload 用)
    pub fn load_pending(&self) -> bool;                    // 需要(重新)加载且没有在途
    pub fn hint(&self) -> Option<&str>;
    pub fn has_active_turn(&self) -> bool;                 // 有 Queued/Running
    pub fn select(&mut self, group_id: i64) -> Vec<Effect>;
    pub fn take_outbox(&mut self) -> Vec<Pending>;
    pub(crate) fn poll_due(&mut self, now: Instant) -> bool;   // 轮询限速 + 在途判定,置 poll_in_flight
    pub(crate) fn load_due(&mut self) -> bool;                  // `load_pending()` 为真时置 load_in_flight 并返回 true
    pub(crate) fn mark_stale(&mut self);                        // 切入面板时:下次重新加载群列表,但不清已有内容
}
pub enum Message { GroupsLoaded(i64, Result<Vec<GroupInfo>, String>), GroupCreated(i64, GroupInfo),
    GroupChanged(i64, GroupInfo), GroupDeleted(i64, i64),
    Polled { project_id: i64, group_id: i64, result: Result<(Vec<GroupMessageInfo>, i64), String> },
    Posted { project_id: i64, group_id: i64, human: GroupMessageInfo, placeholders: Vec<GroupMessageInfo>, unknown: Vec<String> },
    MessageUpdated(i64, GroupMessageInfo), TodoPushed { project_id: i64, group_id: i64, message_id: i64, todo_id: i64 },
    Failed(i64, String) }
impl Message { pub fn project_id(&self) -> i64 }
pub enum Effect { FetchMessages { group_id: i64 } }
pub fn update(state: &mut WorkspaceState, msg: Message) -> Vec<Effect>;
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
```

- [ ] **Step 1: 写失败的测试**

创建 `mod.rs`，先放文件头与测试：

```rust
//! 群聊面板扩展(spec `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`)。
//!
//! 状态按项目挂在 `Workspace.group_chat`。`update` 是**纯函数**(不碰 `Client`、
//! 不 spawn),返回 `Effect` 由 `App` 去执行——这样"乱序/重复/换群后的旧结果"
//! 这类竞态能在单测里直接覆盖。后端更新靠 `rev` 增量轮询(见 `POLL_INTERVAL`
//! 与 `App::poll_group_chat_if_active`)。

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_core::protocol::{AgentKind, GroupAuthor, GroupMemberInfo};

    fn member(id: i64, handle: &str) -> GroupMemberInfo {
        GroupMemberInfo {
            id,
            group_id: 1,
            agent: AgentKind::Claude,
            handle: handle.into(),
            role_prompt: String::new(),
        }
    }

    fn group(id: i64, topic: &str) -> GroupInfo {
        GroupInfo {
            id,
            project_id: 1,
            topic: topic.into(),
            created_ms: 0,
            members: vec![member(id * 10, "claude")],
        }
    }

    fn msg(id: i64, seq: i64, rev: i64, status: Option<GroupMessageStatus>) -> GroupMessageInfo {
        GroupMessageInfo {
            id,
            group_id: 1,
            seq,
            rev,
            author: if status.is_some() {
                GroupAuthor::Member { member_id: 10 }
            } else {
                GroupAuthor::Human
            },
            text: format!("m{id}"),
            mentions: vec![],
            status,
            duration_ms: None,
            created_ms: 0,
            todo_id: None,
        }
    }

    fn loaded(groups: Vec<GroupInfo>) -> WorkspaceState {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::GroupsLoaded(1, Ok(groups)));
        s
    }

    #[test]
    fn groups_loaded_selects_first_group_and_requests_messages() {
        let mut s = WorkspaceState::default();
        let fx = update(
            &mut s,
            Message::GroupsLoaded(1, Ok(vec![group(5, "A"), group(6, "B")])),
        );
        assert!(s.loaded());
        assert_eq!(s.selected(), Some(5));
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 5 }]);
    }

    #[test]
    fn groups_loaded_keeps_selection_when_group_still_exists_and_does_not_refetch() {
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        s.select(6);
        let fx = update(
            &mut s,
            Message::GroupsLoaded(1, Ok(vec![group(5, "A"), group(6, "B2")])),
        );
        assert_eq!(s.selected(), Some(6));
        assert!(fx.is_empty());
        assert_eq!(s.groups()[1].topic, "B2");
    }

    #[test]
    fn groups_loaded_falls_back_when_selected_group_vanished() {
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        s.select(6);
        let fx = update(&mut s, Message::GroupsLoaded(1, Ok(vec![group(5, "A")])));
        assert_eq!(s.selected(), Some(5));
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 5 }]);
    }

    #[test]
    fn groups_loaded_with_no_groups_clears_selection_and_messages() {
        let mut s = loaded(vec![group(5, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 5,
                result: Ok((vec![msg(1, 1, 1, None)], 1)),
            },
        );
        let fx = update(&mut s, Message::GroupsLoaded(1, Ok(vec![])));
        assert_eq!(s.selected(), None);
        assert!(s.messages().is_empty());
        assert!(fx.is_empty());
    }

    #[test]
    fn groups_loaded_error_goes_to_outbox_and_does_not_retry_storm() {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::GroupsLoaded(1, Err("boom".into())));
        assert!(s.loaded());
        assert!(!s.load_pending(), "失败也不立刻重试,避免每个 tick 一次重试风暴");
        let pending = s.take_outbox();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].text.contains("boom"));
    }

    #[test]
    fn select_resets_messages_and_requests_fetch() {
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 5,
                result: Ok((vec![msg(1, 1, 3, None)], 3)),
            },
        );
        let fx = s.select(6);
        assert_eq!(s.selected(), Some(6));
        assert!(s.messages().is_empty());
        assert_eq!(s.latest_rev(), 0);
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 6 }]);
        assert!(s.select(6).is_empty(), "重复选中同一个群不重复拉取");
        assert!(s.select(999).is_empty(), "不存在的群忽略");
    }

    #[test]
    fn polled_merges_by_id_and_sorts_by_seq() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((
                    vec![
                        msg(2, 2, 2, Some(GroupMessageStatus::Running)),
                        msg(1, 1, 1, None),
                    ],
                    2,
                )),
            },
        );
        let ids: Vec<_> = s.messages().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(s.latest_rev(), 2);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(2, 2, 3, Some(GroupMessageStatus::Done))], 3)),
            },
        );
        assert_eq!(s.messages().len(), 2);
        assert_eq!(s.messages()[1].status, Some(GroupMessageStatus::Done));
        assert_eq!(s.latest_rev(), 3);
    }

    /// Review Focus 3:后到的旧结果不得把消息退回旧状态。
    #[test]
    fn stale_poll_result_does_not_regress_a_newer_message() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(2, 2, 5, Some(GroupMessageStatus::Done))], 5)),
            },
        );
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(2, 2, 3, Some(GroupMessageStatus::Running))], 3)),
            },
        );
        assert_eq!(s.messages()[0].status, Some(GroupMessageStatus::Done));
        assert_eq!(s.latest_rev(), 5, "latest_rev 不回退");
    }

    /// Review Focus 3:`Posted` 只 upsert,不推进 `latest_rev`(否则会永久漏掉
    /// rev 更小、尚未取回的变更)。
    #[test]
    fn posted_upserts_messages_but_does_not_advance_latest_rev() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(1, 1, 10, None),
                placeholders: vec![msg(2, 2, 11, Some(GroupMessageStatus::Queued))],
                unknown: vec![],
            },
        );
        assert_eq!(s.messages().len(), 2);
        assert_eq!(s.latest_rev(), 0);
        assert!(s.has_active_turn());
    }

    /// Review Focus 4:换群后,上一个群的异步结果必须被丢弃。
    #[test]
    fn results_for_a_different_group_are_dropped() {
        let mut s = loaded(vec![group(1, "A"), group(2, "B")]);
        s.select(2);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(9, 1, 7, None)], 7)),
            },
        );
        assert!(s.messages().is_empty());
        assert_eq!(s.latest_rev(), 0);
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(9, 1, 7, None),
                placeholders: vec![],
                unknown: vec![],
            },
        );
        assert!(s.messages().is_empty());
    }

    #[test]
    fn poll_error_goes_to_outbox_and_backs_off() {
        let mut s = loaded(vec![group(1, "A")]);
        let t0 = Instant::now();
        assert!(s.poll_due(t0));
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Err("连不上".into()),
            },
        );
        assert_eq!(s.take_outbox().len(), 1);
        assert!(!s.poll_due(t0 + POLL_INTERVAL), "出错后退避,不是下个 tick 就重试");
        // 退避起点是出错那一刻的真实时钟(略晚于 t0),留 1s 余量避免慢机器上偶发失败。
        assert!(s.poll_due(t0 + POLL_BACKOFF + Duration::from_secs(1)));
    }

    #[test]
    fn poll_due_rate_limits_and_blocks_while_in_flight() {
        let mut s = loaded(vec![group(1, "A")]);
        let t0 = Instant::now();
        assert!(s.poll_due(t0));
        assert!(!s.poll_due(t0 + POLL_INTERVAL * 2), "上一次还在途");
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![], 0)),
            },
        );
        assert!(!s.poll_due(t0 + Duration::from_millis(10)), "未到间隔");
        assert!(s.poll_due(t0 + POLL_INTERVAL + Duration::from_millis(10)));
    }

    #[test]
    fn load_due_fires_once_until_result_arrives() {
        let mut s = WorkspaceState::default();
        assert!(s.load_pending());
        assert!(s.load_due());
        assert!(!s.load_due(), "在途期间不重复");
        update(&mut s, Message::GroupsLoaded(1, Ok(vec![])));
        assert!(!s.load_due(), "已加载不再触发");
    }

    #[test]
    fn mark_stale_makes_load_due_fire_again_without_clearing_anything() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(1, 1, 1, None)], 1)),
            },
        );
        s.mark_stale();
        assert_eq!(s.groups().len(), 1, "不清空已有的群");
        assert_eq!(s.messages().len(), 1, "不清空已有的消息");
        assert!(s.loaded(), "曾经加载过:payload 不会闪回\"加载中\"");
        assert!(s.load_pending());
        assert!(s.load_due());
    }

    #[test]
    fn group_created_is_selected_and_prepended() {
        let mut s = loaded(vec![group(1, "A")]);
        let fx = update(&mut s, Message::GroupCreated(1, group(2, "新群")));
        assert_eq!(s.selected(), Some(2));
        assert_eq!(s.groups()[0].id, 2, "新群排最前(后端 list 按 id 倒序)");
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 2 }]);
    }

    #[test]
    fn group_changed_replaces_in_place() {
        let mut s = loaded(vec![group(1, "A"), group(2, "B")]);
        let mut g = group(2, "B");
        g.members.push(member(99, "codex"));
        update(&mut s, Message::GroupChanged(1, g));
        assert_eq!(s.groups()[1].members.len(), 2);
        assert_eq!(s.groups()[0].members.len(), 1);
    }

    #[test]
    fn group_deleted_falls_back_to_next_group() {
        let mut s = loaded(vec![group(1, "A"), group(2, "B")]);
        s.select(2);
        let fx = update(&mut s, Message::GroupDeleted(1, 2));
        assert_eq!(s.groups().len(), 1);
        assert_eq!(s.selected(), Some(1));
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 1 }]);
        // 删的不是当前选中的群:选中与消息保持不动
        let mut s2 = loaded(vec![group(1, "A"), group(2, "B")]);
        let fx2 = update(&mut s2, Message::GroupDeleted(1, 2));
        assert_eq!(s2.selected(), Some(1));
        assert!(fx2.is_empty());
    }

    #[test]
    fn hint_reports_unknown_handles_and_nobody_mentioned() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(1, 1, 1, None),
                placeholders: vec![],
                unknown: vec![],
            },
        );
        assert_eq!(s.hint(), Some("无人被点名,仅作为上下文"));
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(2, 2, 2, None),
                placeholders: vec![msg(3, 3, 3, Some(GroupMessageStatus::Queued))],
                unknown: vec!["nobody".into(), "Override".into()],
            },
        );
        assert_eq!(s.hint(), Some("未找到成员 @nobody、@Override"));
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(4, 4, 4, None),
                placeholders: vec![msg(5, 5, 5, Some(GroupMessageStatus::Queued))],
                unknown: vec![],
            },
        );
        assert_eq!(s.hint(), None, "正常点名清掉上次提示");
    }

    #[test]
    fn todo_pushed_marks_the_message_locally() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(7, 1, 1, Some(GroupMessageStatus::Done))], 1)),
            },
        );
        update(
            &mut s,
            Message::TodoPushed {
                project_id: 1,
                group_id: 1,
                message_id: 7,
                todo_id: 42,
            },
        );
        assert_eq!(s.messages()[0].todo_id, Some(42));
    }

    #[test]
    fn message_updated_upserts_retry_reply() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((
                    vec![msg(2, 2, 2, Some(GroupMessageStatus::Failed { reason: "x".into() }))],
                    2,
                )),
            },
        );
        update(
            &mut s,
            Message::MessageUpdated(1, msg(2, 2, 4, Some(GroupMessageStatus::Queued))),
        );
        assert_eq!(s.messages()[0].status, Some(GroupMessageStatus::Queued));
        assert!(s.has_active_turn());
    }

    #[test]
    fn failed_message_goes_to_outbox() {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::Failed(1, "新建群聊失败: 群主题不能为空".into()));
        let p = s.take_outbox();
        assert_eq!(p.len(), 1);
        assert!(p[0].text.contains("群主题不能为空"));
        assert!(s.take_outbox().is_empty(), "取走后清空");
    }

    #[test]
    fn project_id_is_exposed_for_every_message_kind() {
        let g = group(1, "A");
        let m = msg(1, 1, 1, None);
        let all = [
            Message::GroupsLoaded(3, Ok(vec![])),
            Message::GroupCreated(3, g.clone()),
            Message::GroupChanged(3, g),
            Message::GroupDeleted(3, 1),
            Message::Polled {
                project_id: 3,
                group_id: 1,
                result: Ok((vec![], 0)),
            },
            Message::Posted {
                project_id: 3,
                group_id: 1,
                human: m.clone(),
                placeholders: vec![],
                unknown: vec![],
            },
            Message::MessageUpdated(3, m),
            Message::TodoPushed {
                project_id: 3,
                group_id: 1,
                message_id: 1,
                todo_id: 1,
            },
            Message::Failed(3, "x".into()),
        ];
        for msg in all {
            assert_eq!(msg.project_id(), 3, "{msg:?}");
        }
    }
}
```

`extensions.rs` 加 `pub mod group_chat;`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app extensions::group_chat 2>&1 | tail -15`
Expected: 编译失败，`cannot find type WorkspaceState`。

- [ ] **Step 3: 实现（放在 `mod tests` 之前）**

```rust
use crate::extensions::toast::{Level, Outbox, Pending};
use dozer_core::protocol::{GroupInfo, GroupMessageInfo, GroupMessageStatus};
use std::time::{Duration, Instant};

dozer_core::scope!(pub(crate) LOG, panel, "group_chat");

/// 有发言进行中时,两次轮询的最小间隔。本地 UDS 往返亚毫秒级,这个间隔只是为了
/// 让流式感觉足够跟手又不空转。
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 轮询出错(dozerd 不在等)后的退避。
pub const POLL_BACKOFF: Duration = Duration::from_secs(5);
/// 一次轮询最多取回的消息条数(`ListGroupMessages.limit`)。
pub const POLL_LIMIT: u32 = 200;

#[derive(Debug, Default)]
pub struct WorkspaceState {
    groups: Vec<GroupInfo>,
    selected: Option<i64>,
    /// 当前选中群的消息,按 `seq` 升序。
    messages: Vec<GroupMessageInfo>,
    /// 只由 `Polled` 推进(见 `Message::Posted` 的说明)。
    latest_rev: i64,
    /// 是否曾经成功/失败地加载过一次(payload 的 `loaded`;`mark_stale` 不会把它
    /// 改回 `false`,否则切入面板时前端会闪回"加载中")。
    ever_loaded: bool,
    /// 群列表是否是新的;`false` = 需要(重新)加载。默认 `false`(没加载过)。
    fresh: bool,
    load_in_flight: bool,
    poll_in_flight: bool,
    last_poll: Option<Instant>,
    backoff_until: Option<Instant>,
    hint: Option<String>,
    pub(crate) outbox: Outbox,
}

#[derive(Debug, Clone)]
pub enum Message {
    GroupsLoaded(i64, Result<Vec<GroupInfo>, String>),
    /// 新建群成功:排最前并自动选中。
    GroupCreated(i64, GroupInfo),
    /// 成员增删改后后端返回刷新过的整个群。
    GroupChanged(i64, GroupInfo),
    GroupDeleted(i64, i64),
    Polled {
        project_id: i64,
        group_id: i64,
        result: Result<(Vec<GroupMessageInfo>, i64), String>,
    },
    /// 发送成功:upsert human 消息与排队占位。**不推进 `latest_rev`**——占位的
    /// rev 可能大于某条尚未取回的、rev 更小的变更,推进会让它永久丢失;下一次
    /// 轮询会幂等地带回这些消息。
    Posted {
        project_id: i64,
        group_id: i64,
        human: GroupMessageInfo,
        placeholders: Vec<GroupMessageInfo>,
        unknown: Vec<String>,
    },
    /// 重试成功后后端返回的 `Queued` 消息。
    MessageUpdated(i64, GroupMessageInfo),
    TodoPushed {
        project_id: i64,
        group_id: i64,
        message_id: i64,
        todo_id: i64,
    },
    /// 一次性失败(新建群失败、发送失败…),进 Toast。
    Failed(i64, String),
}

impl Message {
    pub fn project_id(&self) -> i64 {
        match self {
            Message::GroupsLoaded(p, _)
            | Message::GroupCreated(p, _)
            | Message::GroupChanged(p, _)
            | Message::GroupDeleted(p, _)
            | Message::MessageUpdated(p, _)
            | Message::Failed(p, _) => *p,
            Message::Polled { project_id, .. }
            | Message::Posted { project_id, .. }
            | Message::TodoPushed { project_id, .. } => *project_id,
        }
    }
}

/// `update` 要 `App` 去做的异步副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// 从头拉某群的消息(`after_rev = 0`)。
    FetchMessages { group_id: i64 },
}

fn is_active(status: &Option<GroupMessageStatus>) -> bool {
    matches!(
        status,
        Some(GroupMessageStatus::Queued) | Some(GroupMessageStatus::Running)
    )
}

/// 按 id upsert:已有的只有新 `rev >= 旧 rev` 才替换(乱序到达的旧结果不退回旧
/// 状态);没有的插入,保持 `seq` 升序。
fn upsert(messages: &mut Vec<GroupMessageInfo>, incoming: GroupMessageInfo) {
    if let Some(slot) = messages.iter_mut().find(|m| m.id == incoming.id) {
        if incoming.rev >= slot.rev {
            *slot = incoming;
        }
        return;
    }
    let at = messages.partition_point(|m| m.seq < incoming.seq);
    messages.insert(at, incoming);
}

impl WorkspaceState {
    pub fn groups(&self) -> &[GroupInfo] {
        &self.groups
    }
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }
    pub fn messages(&self) -> &[GroupMessageInfo] {
        &self.messages
    }
    pub fn latest_rev(&self) -> i64 {
        self.latest_rev
    }
    pub fn loaded(&self) -> bool {
        self.ever_loaded
    }
    /// 需要(重新)加载群列表且当前没有在途加载。
    pub fn load_pending(&self) -> bool {
        !self.fresh && !self.load_in_flight
    }
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    pub fn has_active_turn(&self) -> bool {
        self.messages.iter().any(|m| is_active(&m.status))
    }

    pub fn take_outbox(&mut self) -> Vec<Pending> {
        self.outbox.take()
    }

    /// 选中某群:清空消息与 `latest_rev`、清提示,并请求从头拉取。重复选中同一个
    /// 群、选中不存在的群都是空操作。
    pub fn select(&mut self, group_id: i64) -> Vec<Effect> {
        if self.selected == Some(group_id) || !self.groups.iter().any(|g| g.id == group_id) {
            return Vec::new();
        }
        self.selected = Some(group_id);
        self.reset_messages();
        vec![Effect::FetchMessages { group_id }]
    }

    fn reset_messages(&mut self) {
        self.messages.clear();
        self.latest_rev = 0;
        self.hint = None;
        self.poll_in_flight = false;
    }

    /// 切到一个"该选的群"(首次加载 / 当前群消失 / 删除后回落),返回需要的副作用。
    fn ensure_selection(&mut self) -> Vec<Effect> {
        if let Some(id) = self.selected
            && self.groups.iter().any(|g| g.id == id)
        {
            return Vec::new();
        }
        self.reset_messages();
        match self.groups.first().map(|g| g.id) {
            Some(id) => {
                self.selected = Some(id);
                vec![Effect::FetchMessages { group_id: id }]
            }
            None => {
                self.selected = None;
                Vec::new()
            }
        }
    }

    /// `load_pending()` 为真时标记在途并返回 `true`(调用方随后去 spawn 加载)。
    pub(crate) fn load_due(&mut self) -> bool {
        if !self.load_pending() {
            return false;
        }
        self.load_in_flight = true;
        true
    }

    /// 切入面板时调用:下次 `load_due` 重新加载群列表。不清已有的群与消息,
    /// 避免切入瞬间闪成"没有群聊"。
    pub(crate) fn mark_stale(&mut self) {
        self.fresh = false;
    }

    /// 是否该发起一次轮询:没有在途、已过退避、距上次 ≥ `POLL_INTERVAL`。
    /// 返回 `true` 时已记下"在途 + 本次时间"。
    pub(crate) fn poll_due(&mut self, now: Instant) -> bool {
        if self.poll_in_flight {
            return false;
        }
        if self.backoff_until.is_some_and(|t| now < t) {
            return false;
        }
        if self
            .last_poll
            .is_some_and(|t| now.duration_since(t) < POLL_INTERVAL)
        {
            return false;
        }
        self.poll_in_flight = true;
        self.last_poll = Some(now);
        true
    }
}

pub fn update(state: &mut WorkspaceState, msg: Message) -> Vec<Effect> {
    match msg {
        Message::GroupsLoaded(_, result) => {
            state.load_in_flight = false;
            state.ever_loaded = true;
            state.fresh = true;
            match result {
                Ok(groups) => {
                    state.groups = groups;
                    state.ensure_selection()
                }
                Err(e) => {
                    state.outbox.push(LOG, Level::Error, format!("加载群聊失败: {e}"));
                    Vec::new()
                }
            }
        }
        Message::GroupCreated(_, group) => {
            let id = group.id;
            state.groups.retain(|g| g.id != id);
            state.groups.insert(0, group);
            state.selected = Some(id);
            state.reset_messages();
            vec![Effect::FetchMessages { group_id: id }]
        }
        Message::GroupChanged(_, group) => {
            if let Some(slot) = state.groups.iter_mut().find(|g| g.id == group.id) {
                *slot = group;
            }
            Vec::new()
        }
        Message::GroupDeleted(_, id) => {
            state.groups.retain(|g| g.id != id);
            if state.selected == Some(id) {
                state.selected = None;
            }
            state.ensure_selection()
        }
        Message::Polled {
            group_id, result, ..
        } => {
            state.poll_in_flight = false;
            if state.selected != Some(group_id) {
                return Vec::new();
            }
            match result {
                Ok((messages, latest)) => {
                    state.backoff_until = None;
                    for m in messages {
                        upsert(&mut state.messages, m);
                    }
                    state.latest_rev = state.latest_rev.max(latest);
                }
                Err(e) => {
                    state.backoff_until = Some(Instant::now() + POLL_BACKOFF);
                    state.outbox.push(LOG, Level::Error, format!("刷新群聊失败: {e}"));
                }
            }
            Vec::new()
        }
        Message::Posted {
            group_id,
            human,
            placeholders,
            unknown,
            ..
        } => {
            if state.selected != Some(group_id) {
                return Vec::new();
            }
            state.hint = if !unknown.is_empty() {
                let names = unknown
                    .iter()
                    .map(|n| format!("@{n}"))
                    .collect::<Vec<_>>()
                    .join("、");
                Some(format!("未找到成员 {names}"))
            } else if placeholders.is_empty() {
                Some("无人被点名,仅作为上下文".to_string())
            } else {
                None
            };
            upsert(&mut state.messages, human);
            for p in placeholders {
                upsert(&mut state.messages, p);
            }
            Vec::new()
        }
        Message::MessageUpdated(_, message) => {
            if state.selected == Some(message.group_id) {
                upsert(&mut state.messages, message);
            }
            Vec::new()
        }
        Message::TodoPushed {
            group_id,
            message_id,
            todo_id,
            ..
        } => {
            if state.selected == Some(group_id)
                && let Some(m) = state.messages.iter_mut().find(|m| m.id == message_id)
            {
                m.todo_id = Some(todo_id);
            }
            Vec::new()
        }
        Message::Failed(_, text) => {
            state.outbox.push(LOG, Level::Error, text);
            Vec::new()
        }
    }
}
```

> `Pending`、`Outbox::push(scope, level, text)`、`take()` 的签名以 `extensions/toast.rs` 为准（上文已核对）；`LOG` 是 `dozer_core::scope!` 生成的 `Scope` 常量，用法同 `extensions/todo/update.rs:140` 的 `outbox.push_err(LOG, …)`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app extensions::group_chat 2>&1 | tail -15`
Expected: 全部 PASS。`log_name_tests::every_panel_scope_in_source_uses_a_known_panel_name` 也要过（`group_chat` 已在 Task 1 登记）。

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/group_chat/mod.rs
git commit -m "$(cat <<'EOF'
feat(group-chat): extension state and pure update with rev-based merge

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 协议——payload、webview 事件校验、声明式推送

**Files:**
- Create: `crates/dozer-app/src/extensions/group_chat/protocol.rs`
- Modify: `crates/dozer-app/src/extensions/group_chat/mod.rs`（加 `mod protocol;` 与 `pub use`）

**Interfaces:**
- Consumes: Task 2 的 `WorkspaceState` 访问器。
- Produces（Task 4、5、9 依赖）：

```rust
pub const GROUP_CHAT_PROTOCOL_VERSION: u32 = 1;
pub const READY_TIMEOUT: Duration;                    // 10s
pub struct GroupChatViewPayload { .. }                // Serialize + PartialEq + Clone
pub fn current_view_payload(state: &WorkspaceState, project_id: i64) -> GroupChatViewPayload;
pub fn encode_group_chat_push(revision: u64, payload: GroupChatViewPayload) -> String;
pub enum GroupChatWebviewEvent { .. }                 // Deserialize
pub fn parse_group_chat_event(body: &str) -> Result<GroupChatWebviewEvent, String>;
pub enum Command { CreateGroup{..}, SelectGroup{..}, DeleteGroup{..}, AddMember{..}, UpdateMember{..}, RemoveMember{..}, Post{..}, Cancel{..}, Retry{..}, PushTodo{ group_id, message_id, text }, OpenTodo{..} }
pub(crate) fn route_event(state: &WorkspaceState, event: GroupChatWebviewEvent) -> Option<Command>;
pub struct WebviewPushState { .. }                    // 同 todo::WebviewPushState
// 限制常量
pub const MAX_TOPIC_CHARS: usize = 200;  pub const MAX_ROLE_CHARS: usize = 2_000;
pub const MAX_POST_CHARS: usize = 20_000;  pub const MAX_TODO_CHARS: usize = 10_000;
```

- [ ] **Step 1: 写失败的测试**

```rust
//! 群聊面板 webview 推送协议:把 `WorkspaceState` 算成要序列化推给 webview 的
//! `GroupChatViewPayload`,并解析、校验 webview 发回的事件。前端只渲染与保存纯视图
//! 状态(草稿、对话框开关、滚动),**不做任何业务判断**——作者显示名、成员颜色
//! 槽位、状态映射、提示文案都在这里算好。形态仿 `extensions/todo/protocol.rs`
//! (声明式比较 + 固定单槽)。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::group_chat::{Message, update};
    use dozer_core::protocol::{
        AgentKind, GroupAuthor, GroupCancelScope, GroupInfo, GroupMemberInfo, GroupMessageInfo,
        GroupMessageStatus,
    };
    use serde_json::json;

    fn member(id: i64, agent: AgentKind, handle: &str) -> GroupMemberInfo {
        GroupMemberInfo {
            id,
            group_id: 1,
            agent,
            handle: handle.into(),
            role_prompt: "角色".into(),
        }
    }

    fn group() -> GroupInfo {
        GroupInfo {
            id: 1,
            project_id: 7,
            topic: "评审登录方案".into(),
            created_ms: 0,
            members: vec![
                member(10, AgentKind::Claude, "架构师"),
                member(11, AgentKind::Codex, "审阅者"),
            ],
        }
    }

    fn msg(
        id: i64,
        seq: i64,
        author: GroupAuthor,
        status: Option<GroupMessageStatus>,
    ) -> GroupMessageInfo {
        GroupMessageInfo {
            id,
            group_id: 1,
            seq,
            rev: seq,
            author,
            text: format!("t{id}"),
            mentions: vec![],
            status,
            duration_ms: Some(1500),
            created_ms: 0,
            todo_id: None,
        }
    }

    fn state_with_messages(msgs: Vec<GroupMessageInfo>) -> WorkspaceState {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::GroupsLoaded(7, Ok(vec![group()])));
        update(
            &mut s,
            Message::Polled {
                project_id: 7,
                group_id: 1,
                result: Ok((msgs, 99)),
            },
        );
        s
    }

    // ---- payload ----

    #[test]
    fn payload_for_unloaded_state_is_empty_but_marks_not_loaded() {
        let p = current_view_payload(&WorkspaceState::default(), 7);
        assert!(!p.loaded);
        assert!(p.groups.is_empty());
        assert_eq!(p.selected_group_id, None);
        assert!(!p.running);
    }

    #[test]
    fn payload_resolves_author_labels_agents_and_color_slots() {
        let s = state_with_messages(vec![
            msg(1, 1, GroupAuthor::Human, None),
            msg(2, 2, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Done)),
            msg(3, 3, GroupAuthor::Member { member_id: 11 }, Some(GroupMessageStatus::Done)),
            msg(4, 4, GroupAuthor::Member { member_id: 99 }, Some(GroupMessageStatus::Done)),
        ]);
        let p = current_view_payload(&s, 7);
        assert_eq!(p.messages[0].author.kind, AuthorKind::Human);
        assert_eq!(p.messages[0].author.label, "你");
        assert_eq!(p.messages[1].author.label, "@架构师");
        assert_eq!(p.messages[1].author.agent_label.as_deref(), Some("Claude"));
        assert_eq!(p.messages[1].author.color_slot, Some(0));
        assert_eq!(p.messages[2].author.agent_label.as_deref(), Some("Codex"));
        assert_eq!(p.messages[2].author.color_slot, Some(1));
        assert_eq!(p.messages[3].author.label, "已移除成员");
        assert_eq!(p.messages[3].author.color_slot, None);
    }

    #[test]
    fn payload_maps_statuses_and_failure_reason() {
        let s = state_with_messages(vec![
            msg(1, 1, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Queued)),
            msg(2, 2, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Running)),
            msg(
                3,
                3,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Failed { reason: "发言超时".into() }),
            ),
            msg(4, 4, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Cancelled)),
            msg(5, 5, GroupAuthor::Human, None),
        ]);
        let p = current_view_payload(&s, 7);
        let states: Vec<_> = p.messages.iter().map(|m| m.status).collect();
        assert_eq!(
            states,
            vec![
                Some(StatusKey::Queued),
                Some(StatusKey::Running),
                Some(StatusKey::Failed),
                Some(StatusKey::Cancelled),
                None
            ]
        );
        assert_eq!(p.messages[2].reason.as_deref(), Some("发言超时"));
        assert!(p.running, "有 Queued/Running → running");
    }

    #[test]
    fn payload_members_carry_agent_label_and_role() {
        let s = state_with_messages(vec![]);
        let p = current_view_payload(&s, 7);
        let g = &p.groups[0];
        assert_eq!(g.topic, "评审登录方案");
        assert_eq!(g.members[0].agent, AgentKind::Claude);
        assert_eq!(g.members[0].agent_label, "Claude");
        assert_eq!(g.members[1].agent_label, "Codex");
        assert_eq!(g.members[1].role_prompt, "角色");
        assert_eq!(p.selected_group_id, Some(1));
    }

    #[test]
    fn payload_serializes_with_snake_case_keys_and_roundtrips_equality() {
        let s = state_with_messages(vec![msg(1, 1, GroupAuthor::Human, None)]);
        let p = current_view_payload(&s, 7);
        let v: serde_json::Value = serde_json::from_str(&encode_group_chat_push(3, p.clone())).unwrap();
        assert_eq!(v["protocol_version"], 1);
        assert_eq!(v["revision"], 3);
        assert_eq!(v["payload"]["selected_group_id"], 1);
        assert_eq!(v["payload"]["groups"][0]["members"][0]["agent"], "claude");
        assert_eq!(v["payload"]["messages"][0]["author"]["kind"], "human");
        assert_eq!(current_view_payload(&s, 7), p, "同一状态两次计算相等(声明式去重依赖它)");
    }

    #[test]
    fn payload_includes_hint() {
        let mut s = state_with_messages(vec![]);
        update(
            &mut s,
            Message::Posted {
                project_id: 7,
                group_id: 1,
                human: msg(1, 1, GroupAuthor::Human, None),
                placeholders: vec![],
                unknown: vec![],
            },
        );
        assert_eq!(
            current_view_payload(&s, 7).hint.as_deref(),
            Some("无人被点名,仅作为上下文")
        );
    }

    // ---- 事件解析与校验 ----

    fn ev(v: serde_json::Value) -> GroupChatWebviewEvent {
        parse_group_chat_event(&v.to_string()).expect("parse")
    }

    #[test]
    fn parse_rejects_garbage_and_unknown_kinds() {
        assert!(parse_group_chat_event("nope").is_err());
        assert!(parse_group_chat_event(r#"{"kind":"rm_rf"}"#).is_err());
    }

    #[test]
    fn ready_and_failed_are_not_commands() {
        let s = state_with_messages(vec![]);
        assert_eq!(route_event(&s, ev(json!({"kind":"ready"}))), None);
        assert_eq!(route_event(&s, ev(json!({"kind":"failed","reason":"x"}))), None);
    }

    #[test]
    fn create_group_trims_and_validates_topic() {
        let s = WorkspaceState::default();
        assert_eq!(
            route_event(&s, ev(json!({"kind":"create_group","topic":"  评审  "}))),
            Some(Command::CreateGroup { topic: "评审".into() })
        );
        assert_eq!(route_event(&s, ev(json!({"kind":"create_group","topic":"   "}))), None);
        let long = "字".repeat(MAX_TOPIC_CHARS + 1);
        assert_eq!(route_event(&s, ev(json!({"kind":"create_group","topic":long}))), None);
    }

    #[test]
    fn select_and_delete_require_an_existing_group() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"select_group","group_id":1}))),
            Some(Command::SelectGroup { group_id: 1 })
        );
        assert_eq!(route_event(&s, ev(json!({"kind":"select_group","group_id":9}))), None);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"delete_group","group_id":1}))),
            Some(Command::DeleteGroup { group_id: 1 })
        );
        assert_eq!(route_event(&s, ev(json!({"kind":"delete_group","group_id":9}))), None);
    }

    #[test]
    fn add_member_accepts_only_claude_and_codex() {
        let s = state_with_messages(vec![]);
        let ok = ev(json!({"kind":"add_member","group_id":1,"agent":"codex","handle":" 审阅者2 ","role_prompt":"挑刺"}));
        assert_eq!(
            route_event(&s, ok),
            Some(Command::AddMember {
                group_id: 1,
                agent: AgentKind::Codex,
                handle: "审阅者2".into(),
                role_prompt: "挑刺".into()
            })
        );
        for agent in ["goose", "aider", "unknown", "codebuddy"] {
            let bad = ev(json!({"kind":"add_member","group_id":1,"agent":agent,"handle":"x","role_prompt":""}));
            assert_eq!(route_event(&s, bad), None, "{agent}");
        }
        let no_group = ev(json!({"kind":"add_member","group_id":9,"agent":"claude","handle":"x","role_prompt":""}));
        assert_eq!(route_event(&s, no_group), None);
        let empty_handle = ev(json!({"kind":"add_member","group_id":1,"agent":"claude","handle":"  ","role_prompt":""}));
        assert_eq!(route_event(&s, empty_handle), None);
    }

    #[test]
    fn role_prompt_length_is_capped() {
        let s = state_with_messages(vec![]);
        let long = "字".repeat(MAX_ROLE_CHARS + 1);
        let e = ev(json!({"kind":"add_member","group_id":1,"agent":"claude","handle":"x","role_prompt":long}));
        assert_eq!(route_event(&s, e), None);
    }

    #[test]
    fn update_and_remove_member_require_existing_member() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"update_member","member_id":10,"handle":"新名","role_prompt":""}))),
            Some(Command::UpdateMember { member_id: 10, handle: "新名".into(), role_prompt: String::new() })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"update_member","member_id":77,"handle":"x","role_prompt":""}))),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"remove_member","member_id":11}))),
            Some(Command::RemoveMember { member_id: 11 })
        );
        assert_eq!(route_event(&s, ev(json!({"kind":"remove_member","member_id":77}))), None);
    }

    #[test]
    fn post_requires_current_group_and_valid_text() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"post","group_id":1,"text":" @架构师 你好 "}))),
            Some(Command::Post { group_id: 1, text: "@架构师 你好".into() })
        );
        assert_eq!(route_event(&s, ev(json!({"kind":"post","group_id":2,"text":"hi"}))), None);
        assert_eq!(route_event(&s, ev(json!({"kind":"post","group_id":1,"text":"  "}))), None);
        let long = "字".repeat(MAX_POST_CHARS + 1);
        assert_eq!(route_event(&s, ev(json!({"kind":"post","group_id":1,"text":long}))), None);
    }

    #[test]
    fn cancel_requires_current_group_and_valid_scope() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"cancel","group_id":1,"scope":"round"}))),
            Some(Command::Cancel { group_id: 1, scope: GroupCancelScope::Round })
        );
        assert!(parse_group_chat_event(r#"{"kind":"cancel","group_id":1,"scope":"everything"}"#).is_err());
        assert_eq!(route_event(&s, ev(json!({"kind":"cancel","group_id":2,"scope":"turn"}))), None);
    }

    #[test]
    fn retry_only_for_failed_or_cancelled_messages_in_view() {
        let s = state_with_messages(vec![
            msg(1, 1, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Failed { reason: "x".into() })),
            msg(2, 2, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Done)),
            msg(3, 3, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Cancelled)),
            msg(4, 4, GroupAuthor::Human, None),
        ]);
        assert_eq!(route_event(&s, ev(json!({"kind":"retry","message_id":1}))), Some(Command::Retry { message_id: 1 }));
        assert_eq!(route_event(&s, ev(json!({"kind":"retry","message_id":3}))), Some(Command::Retry { message_id: 3 }));
        assert_eq!(route_event(&s, ev(json!({"kind":"retry","message_id":2}))), None);
        assert_eq!(route_event(&s, ev(json!({"kind":"retry","message_id":4}))), None);
        assert_eq!(route_event(&s, ev(json!({"kind":"retry","message_id":99}))), None);
    }

    #[test]
    fn push_todo_requires_message_in_view_without_existing_todo_and_valid_text() {
        let mut linked = msg(2, 2, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Done));
        linked.todo_id = Some(5);
        let s = state_with_messages(vec![
            msg(1, 1, GroupAuthor::Human, None),
            linked,
            msg(3, 3, GroupAuthor::Member { member_id: 10 }, Some(GroupMessageStatus::Running)),
        ]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"push_todo","message_id":1,"text":" 加验证码 "}))),
            Some(Command::PushTodo { group_id: 1, message_id: 1, text: "加验证码".into() })
        );
        assert_eq!(route_event(&s, ev(json!({"kind":"push_todo","message_id":2,"text":"x"}))), None, "已转过");
        assert_eq!(route_event(&s, ev(json!({"kind":"push_todo","message_id":3,"text":"x"}))), None, "进行中的消息没有正文可转");
        assert_eq!(route_event(&s, ev(json!({"kind":"push_todo","message_id":1,"text":" "}))), None);
        assert_eq!(route_event(&s, ev(json!({"kind":"push_todo","message_id":99,"text":"x"}))), None);
        let long = "字".repeat(MAX_TODO_CHARS + 1);
        assert_eq!(route_event(&s, ev(json!({"kind":"push_todo","message_id":1,"text":long}))), None);
    }

    #[test]
    fn open_todo_needs_positive_id() {
        let s = WorkspaceState::default();
        assert_eq!(route_event(&s, ev(json!({"kind":"open_todo","todo_id":5}))), Some(Command::OpenTodo { todo_id: 5 }));
        assert_eq!(route_event(&s, ev(json!({"kind":"open_todo","todo_id":0}))), None);
        assert_eq!(route_event(&s, ev(json!({"kind":"open_todo","todo_id":-3}))), None);
    }

    // ---- 声明式推送状态 ----

    #[test]
    fn push_state_only_pushes_when_ready_and_changed() {
        let mut ps = WebviewPushState::default();
        let a = current_view_payload(&WorkspaceState::default(), 7);
        assert!(ps.pending_push(&a).is_none(), "未 ready 不推");
        ps.set_ready(true);
        assert!(ps.pending_push(&a).is_some());
        assert_eq!(ps.mark_sent(a.clone()), 1);
        assert!(ps.pending_push(&a).is_none(), "未变化不重推");
        let b = current_view_payload(&state_with_messages(vec![]), 7);
        assert!(ps.pending_push(&b).is_some());
        ps.set_ready(true);
        assert!(ps.pending_push(&b).is_some(), "重新 ready 强制重发");
    }

    #[test]
    fn push_state_times_out_when_never_ready_and_recovers_on_clear() {
        let mut ps = WebviewPushState::default();
        let t0 = Instant::now();
        ps.observe_availability(true, t0);
        assert!(ps.failed().is_none());
        ps.observe_availability(true, t0 + READY_TIMEOUT + Duration::from_secs(1));
        assert!(ps.failed().is_some());
        ps.clear_failed();
        assert!(ps.failed().is_none());
    }

    #[test]
    fn push_state_failed_event_blocks_pushes() {
        let mut ps = WebviewPushState::default();
        ps.set_ready(true);
        ps.set_failed("渲染异常".into());
        let a = current_view_payload(&WorkspaceState::default(), 7);
        assert!(ps.pending_push(&a).is_none());
        assert_eq!(ps.failed(), Some("渲染异常"));
    }
}
```

`mod.rs` 顶部加：

```rust
mod protocol;
pub use protocol::{
    Command, GroupChatViewPayload, GroupChatWebviewEvent, WebviewPushState,
    current_view_payload, encode_group_chat_push, parse_group_chat_event,
};
pub(crate) use protocol::route_event;
```

（`mod view;` 与 `pub use view::{ShellMessage, content_pane};` 在 Task 5 创建 `view.rs` 时一起加。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app group_chat::protocol 2>&1 | tail -15`
Expected: 编译失败，`cannot find type GroupChatWebviewEvent`。

- [ ] **Step 3: 实现（放在 `mod tests` 之前）**

```rust
use super::WorkspaceState;
use dozer_core::protocol::{
    AgentKind, GroupAuthor, GroupCancelScope, GroupInfo, GroupMessageStatus,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const GROUP_CHAT_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 来自 webview 的文本长度上限(字符数)。webview 内容不可信,防止异常超长文本
/// 落库/进提示词。`MAX_POST_CHARS` 与后端 `group_service::MAX_POST_CHARS` 对齐。
pub const MAX_TOPIC_CHARS: usize = 200;
pub const MAX_ROLE_CHARS: usize = 2_000;
pub const MAX_POST_CHARS: usize = 20_000;
pub const MAX_TODO_CHARS: usize = 10_000;
const MAX_HANDLE_CHARS: usize = 32;

/// 成员颜色槽位数(前端映射到 4 个 CSS 变量)。
const COLOR_SLOTS: usize = 4;

// ---------------------------------------------------------------------------
// Rust → webview
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusKey {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Human,
    Member,
    System,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AuthorVm {
    pub kind: AuthorKind,
    /// 显示名:"你" / "@handle" / "已移除成员" / "系统"。
    pub label: String,
    /// "Claude"/"Codex",成员消息才有。
    pub agent_label: Option<String>,
    /// 成员颜色槽位(0..COLOR_SLOTS),已移除成员/非成员为 `None`。
    pub color_slot: Option<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemberVm {
    pub id: i64,
    pub agent: AgentKind,
    pub agent_label: &'static str,
    pub handle: String,
    pub role_prompt: String,
    pub color_slot: u8,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupVm {
    pub id: i64,
    pub topic: String,
    pub members: Vec<MemberVm>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MessageVm {
    pub id: i64,
    pub seq: i64,
    pub author: AuthorVm,
    pub text: String,
    pub status: Option<StatusKey>,
    /// 失败原因(`status == Failed` 时)。
    pub reason: Option<String>,
    pub duration_ms: Option<u64>,
    pub todo_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupChatViewPayload {
    pub project_id: i64,
    /// `false` = 还没从 dozerd 加载过(前端显示"加载中"而不是"还没有群聊")。
    pub loaded: bool,
    pub groups: Vec<GroupVm>,
    pub selected_group_id: Option<i64>,
    /// 当前选中群的消息,按 `seq` 升序。
    pub messages: Vec<MessageVm>,
    /// 一次性发送提示("无人被点名…"/"未找到成员 @x"),下次发送/切群时清除。
    pub hint: Option<String>,
    /// 本群有 `Queued`/`Running` 的发言(输入框旁显示"停止本轮")。
    pub running: bool,
}

fn color_slot_of(group: &GroupInfo, member_id: i64) -> Option<u8> {
    group
        .members
        .iter()
        .position(|m| m.id == member_id)
        .map(|i| (i % COLOR_SLOTS) as u8)
}

fn status_of(status: &GroupMessageStatus) -> StatusKey {
    match status {
        GroupMessageStatus::Queued => StatusKey::Queued,
        GroupMessageStatus::Running => StatusKey::Running,
        GroupMessageStatus::Done => StatusKey::Done,
        GroupMessageStatus::Failed { .. } => StatusKey::Failed,
        GroupMessageStatus::Cancelled => StatusKey::Cancelled,
    }
}

pub fn current_view_payload(state: &WorkspaceState, project_id: i64) -> GroupChatViewPayload {
    let groups: Vec<GroupVm> = state
        .groups()
        .iter()
        .map(|g| GroupVm {
            id: g.id,
            topic: g.topic.clone(),
            members: g
                .members
                .iter()
                .enumerate()
                .map(|(i, m)| MemberVm {
                    id: m.id,
                    agent: m.agent,
                    agent_label: m.agent.display_label(),
                    handle: m.handle.clone(),
                    role_prompt: m.role_prompt.clone(),
                    color_slot: (i % COLOR_SLOTS) as u8,
                })
                .collect(),
        })
        .collect();

    let selected_group = state
        .selected()
        .and_then(|id| state.groups().iter().find(|g| g.id == id));

    let messages = state
        .messages()
        .iter()
        .map(|m| {
            let author = match &m.author {
                GroupAuthor::Human => AuthorVm {
                    kind: AuthorKind::Human,
                    label: "你".to_string(),
                    agent_label: None,
                    color_slot: None,
                },
                GroupAuthor::System => AuthorVm {
                    kind: AuthorKind::System,
                    label: "系统".to_string(),
                    agent_label: None,
                    color_slot: None,
                },
                GroupAuthor::Member { member_id } => {
                    let found =
                        selected_group.and_then(|g| g.members.iter().find(|x| x.id == *member_id));
                    match found {
                        Some(mem) => AuthorVm {
                            kind: AuthorKind::Member,
                            label: format!("@{}", mem.handle),
                            agent_label: Some(mem.agent.display_label().to_string()),
                            color_slot: selected_group.and_then(|g| color_slot_of(g, mem.id)),
                        },
                        None => AuthorVm {
                            kind: AuthorKind::Member,
                            label: "已移除成员".to_string(),
                            agent_label: None,
                            color_slot: None,
                        },
                    }
                }
            };
            MessageVm {
                id: m.id,
                seq: m.seq,
                author,
                text: m.text.clone(),
                status: m.status.as_ref().map(status_of),
                reason: match &m.status {
                    Some(GroupMessageStatus::Failed { reason }) => Some(reason.clone()),
                    _ => None,
                },
                duration_ms: m.duration_ms,
                todo_id: m.todo_id,
            }
        })
        .collect();

    GroupChatViewPayload {
        project_id,
        loaded: state.loaded(),
        groups,
        selected_group_id: state.selected(),
        messages,
        hint: state.hint().map(str::to_string),
        running: state.has_active_turn(),
    }
}

#[derive(Debug, Serialize)]
struct GroupChatPushEnvelope {
    protocol_version: u32,
    revision: u64,
    payload: GroupChatViewPayload,
}

pub fn encode_group_chat_push(revision: u64, payload: GroupChatViewPayload) -> String {
    serde_json::to_string(&GroupChatPushEnvelope {
        protocol_version: GROUP_CHAT_PROTOCOL_VERSION,
        revision,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

// ---------------------------------------------------------------------------
// webview → Rust
// ---------------------------------------------------------------------------

/// webview 发回的事件。内容不可信:长度、id 存在性、agent 种类都在 `route_event`
/// 里校验。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GroupChatWebviewEvent {
    Ready,
    Failed { reason: String },
    CreateGroup { topic: String },
    SelectGroup { group_id: i64 },
    DeleteGroup { group_id: i64 },
    AddMember {
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    },
    UpdateMember {
        member_id: i64,
        handle: String,
        role_prompt: String,
    },
    RemoveMember { member_id: i64 },
    Post { group_id: i64, text: String },
    Cancel { group_id: i64, scope: GroupCancelScope },
    Retry { message_id: i64 },
    PushTodo { message_id: i64, text: String },
    OpenTodo { todo_id: i64 },
}

pub fn parse_group_chat_event(body: &str) -> Result<GroupChatWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// 校验后交给 `App` 执行的命令。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    CreateGroup { topic: String },
    SelectGroup { group_id: i64 },
    DeleteGroup { group_id: i64 },
    AddMember {
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    },
    UpdateMember {
        member_id: i64,
        handle: String,
        role_prompt: String,
    },
    RemoveMember { member_id: i64 },
    Post { group_id: i64, text: String },
    Cancel { group_id: i64, scope: GroupCancelScope },
    Retry { message_id: i64 },
    PushTodo {
        group_id: i64,
        message_id: i64,
        text: String,
    },
    OpenTodo { todo_id: i64 },
}

fn clean(text: &str, max_chars: usize) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && t.chars().count() <= max_chars).then(|| t.to_string())
}

fn group_exists(state: &WorkspaceState, id: i64) -> bool {
    state.groups().iter().any(|g| g.id == id)
}

fn member_exists(state: &WorkspaceState, id: i64) -> bool {
    state
        .groups()
        .iter()
        .any(|g| g.members.iter().any(|m| m.id == id))
}

/// 校验 webview 事件并映射成命令。`id` 已不存在、入参非法一律返回 `None`
/// (空操作),绝不能落到别的对象上。`Ready`/`Failed` 由 `App` 自己处理,这里
/// 返回 `None`。
pub(crate) fn route_event(
    state: &WorkspaceState,
    event: GroupChatWebviewEvent,
) -> Option<Command> {
    use GroupChatWebviewEvent as Ev;
    match event {
        Ev::Ready | Ev::Failed { .. } => None,
        Ev::CreateGroup { topic } => Some(Command::CreateGroup {
            topic: clean(&topic, MAX_TOPIC_CHARS)?,
        }),
        Ev::SelectGroup { group_id } => group_exists(state, group_id)
            .then_some(Command::SelectGroup { group_id }),
        Ev::DeleteGroup { group_id } => group_exists(state, group_id)
            .then_some(Command::DeleteGroup { group_id }),
        Ev::AddMember {
            group_id,
            agent,
            handle,
            role_prompt,
        } => {
            if !group_exists(state, group_id)
                || !matches!(agent, AgentKind::Claude | AgentKind::Codex)
            {
                return None;
            }
            Some(Command::AddMember {
                group_id,
                agent,
                handle: clean(&handle, MAX_HANDLE_CHARS)?,
                role_prompt: role_prompt_of(&role_prompt)?,
            })
        }
        Ev::UpdateMember {
            member_id,
            handle,
            role_prompt,
        } => {
            if !member_exists(state, member_id) {
                return None;
            }
            Some(Command::UpdateMember {
                member_id,
                handle: clean(&handle, MAX_HANDLE_CHARS)?,
                role_prompt: role_prompt_of(&role_prompt)?,
            })
        }
        Ev::RemoveMember { member_id } => member_exists(state, member_id)
            .then_some(Command::RemoveMember { member_id }),
        Ev::Post { group_id, text } => {
            if state.selected() != Some(group_id) {
                return None;
            }
            Some(Command::Post {
                group_id,
                text: clean(&text, MAX_POST_CHARS)?,
            })
        }
        Ev::Cancel { group_id, scope } => {
            (state.selected() == Some(group_id)).then_some(Command::Cancel { group_id, scope })
        }
        Ev::Retry { message_id } => {
            let m = state.messages().iter().find(|m| m.id == message_id)?;
            matches!(
                m.status,
                Some(GroupMessageStatus::Failed { .. }) | Some(GroupMessageStatus::Cancelled)
            )
            .then_some(Command::Retry { message_id })
        }
        Ev::PushTodo { message_id, text } => {
            let m = state.messages().iter().find(|m| m.id == message_id)?;
            // 一条消息只转一次;进行中/排队中的 agent 消息还没有正文。
            if m.todo_id.is_some()
                || matches!(
                    m.status,
                    Some(GroupMessageStatus::Queued) | Some(GroupMessageStatus::Running)
                )
            {
                return None;
            }
            Some(Command::PushTodo {
                // 消息列表只含当前选中群的消息,所以所属群就是当前选中群。
                group_id: state.selected()?,
                message_id,
                text: clean(&text, MAX_TODO_CHARS)?,
            })
        }
        Ev::OpenTodo { todo_id } => (todo_id > 0).then_some(Command::OpenTodo { todo_id }),
    }
}

/// 角色设定允许为空(默认无角色);非空则按长度上限截检,超限整个事件作废。
fn role_prompt_of(text: &str) -> Option<String> {
    let t = text.trim();
    (t.chars().count() <= MAX_ROLE_CHARS).then(|| t.to_string())
}

// ---------------------------------------------------------------------------
// 声明式推送状态(同 todo::WebviewPushState)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<GroupChatViewPayload>,
    revision: u64,
    /// webview 已进池但尚未 ready 的起点,用于判定加载超时。
    waiting_since: Option<Instant>,
    /// 加载失败/超时原因;`Some` 时内容区回落原生占位页(重试清除)。
    failed: Option<String>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发当前内容。
            self.last_sent = None;
            self.waiting_since = None;
        }
    }

    /// 每帧由消费点调用:`available` = webview 当前在池里。
    pub fn observe_availability(&mut self, available: bool, now: Instant) {
        if !available {
            self.ready = false;
            self.waiting_since = None;
            return;
        }
        if self.ready {
            self.waiting_since = None;
            return;
        }
        let since = *self.waiting_since.get_or_insert(now);
        if self.failed.is_none() && now.duration_since(since) > READY_TIMEOUT {
            self.failed = Some("面板页面加载超时".to_string());
        }
    }

    pub fn pending_push(&self, desired: &GroupChatViewPayload) -> Option<GroupChatViewPayload> {
        if !self.ready || self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    /// 记录已送达并返回本次 revision(从 1 起)。
    pub fn mark_sent(&mut self, payload: GroupChatViewPayload) -> u64 {
        self.revision += 1;
        self.last_sent = Some(payload);
        self.revision
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn set_failed(&mut self, reason: String) {
        self.failed = Some(reason);
        self.ready = false;
        self.last_sent = None;
        self.waiting_since = None;
    }

    pub fn clear_failed(&mut self) {
        self.failed = None;
        self.waiting_since = None;
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app group_chat 2>&1 | tail -15`
Expected: Task 2 与 Task 3 的测试全部 PASS。

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/group_chat
git commit -m "$(cat <<'EOF'
feat(group-chat): webview protocol, payload and event validation

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 异步 spawn 函数与 `Command` 执行

**Files:**
- Modify: `crates/dozer-app/src/extensions/group_chat/mod.rs`（加 spawn 函数）

**Interfaces:**
- Consumes: `dozer_client::Client` 的群聊方法（后端 plan 产出，已合并）、Task 2/3。
- Produces（Task 5 依赖）：

```rust
pub fn spawn_load_groups(project_id: i64, client: &Client, handle: &Handle, emit: impl Fn(Message) + Send + 'static);
pub fn spawn_fetch_messages(project_id: i64, group_id: i64, after_rev: i64, client: &Client, handle: &Handle, emit: impl Fn(Message) + Send + 'static);
pub fn spawn_command(project_id: i64, cmd: Command, client: &Client, handle: &Handle, emit: impl Fn(Message) + Send + 'static);
```

> 这些函数只是"调 `Client` → 把结果包成 `Message` 经 `emit` 发回"，逻辑薄。**可测部分**是"每个 `Command` 对应哪个 `Message`/失败文案"，把它抽成纯函数 `outcome_message`（输入 `Command` 与后端结果的抽象）会过度设计；这里用一个针对真实 daemon 的集成测试覆盖（`dozerd` 已有 `Client` 的真实 daemon 测试范式）。

- [ ] **Step 1: 写契约测试**

放在 `crates/dozer-client/tests/group_chat_client.rs`（`dozer-client` 的 dev-dependencies 里已有 `dozerd`，现有 `against_real_daemon.rs` 就是这么用的；**不要**为此给 `dozer-app` 加 `dozerd` 依赖）。它验证后端 plan 产出的 `Client` 方法端到端可用，并作为 GUI 层假设的契约测试：

```rust
use dozer_client::Client;
use dozer_core::protocol::{AgentKind, GroupCancelScope, GroupMessageStatus};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

struct EchoRunner;
impl dozerd::group_adapter::GroupAgentRunner for EchoRunner {
    fn run<'a>(
        &'a self,
        _req: dozerd::group_adapter::TurnRequest,
        _cancel: watch::Receiver<bool>,
    ) -> dozerd::group_adapter::BoxFuture<'a, Result<String, dozerd::group_adapter::TurnError>> {
        Box::pin(async { Ok("echo".to_string()) })
    }
}

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/tmp/dz-group-client-{}.sock", uuid::Uuid::new_v4()))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (std::path::PathBuf, CleanupGuard, Arc<dozerd::projects::ProjectStore>) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-group-client-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let dir_lookup = projects.clone();
    let groups = dozerd::group_service::GroupService::new(
        Arc::new(dozerd::group_store::GroupStore::new(&db).unwrap()),
        Arc::new(EchoRunner),
        Arc::new(move |id| dir_lookup.path_of(id).ok().flatten().map(std::path::PathBuf::from)),
    );
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: projects.clone(),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
        groups,
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

#[tokio::test]
async fn gui_contract_create_post_poll_cancel_push_todo() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    let group = client.create_group(project.id, "契约").await.unwrap();
    client
        .add_group_member(group.id, AgentKind::Claude, "claude", "")
        .await
        .unwrap();
    let (human, ph, unknown) = client
        .post_group_message(group.id, "@claude 你好 @ghost")
        .await
        .unwrap();
    assert_eq!(ph.len(), 1);
    assert_eq!(unknown, vec!["ghost".to_string()]);

    // GUI 的轮询语义:after_rev=0 取全部,之后只取变更。
    let mut latest = 0;
    let mut all = Vec::new();
    for _ in 0..100 {
        let (msgs, l) = client.list_group_messages(group.id, latest, 200).await.unwrap();
        latest = l;
        all.extend(msgs);
        if all
            .iter()
            .any(|m| m.status == Some(GroupMessageStatus::Done))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(all.iter().any(|m| m.id == human.id));
    let done = all
        .iter()
        .find(|m| m.status == Some(GroupMessageStatus::Done))
        .expect("agent 回复完成");
    let todo = client
        .push_group_message_to_todo(done.id, "跟进")
        .await
        .unwrap();
    assert_eq!(todo.text, "跟进");
    client
        .cancel_group(group.id, GroupCancelScope::Round)
        .await
        .unwrap();
}
```

- [ ] **Step 2: 跑测试（预期已通过，因为后端已合并）**

Run: `cargo test -p dozer-client --test group_chat_client 2>&1 | tail -10`
Expected: PASS——这一步是**确认后端契约与本计划假设一致**（`list_group_messages(after_rev, limit)`、`post_group_message` 返回三元组等）。若失败，说明后端实际签名与计划假设不同：**停下**，把差异汇报，不要在 GUI 层打补丁绕过。

- [ ] **Step 3: 实现 spawn 函数**

`mod.rs` 追加：

```rust
use dozer_client::Client;
use tokio::runtime::Handle;

/// 加载某项目的群聊列表。
pub fn spawn_load_groups(
    project_id: i64,
    client: &Client,
    handle: &Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_groups(project_id)
            .await
            .map_err(|e| e.to_string());
        emit(Message::GroupsLoaded(project_id, result));
    });
}

/// 取某群 `rev > after_rev` 的消息(`after_rev = 0` 即全量)。
pub fn spawn_fetch_messages(
    project_id: i64,
    group_id: i64,
    after_rev: i64,
    client: &Client,
    handle: &Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_group_messages(group_id, after_rev, POLL_LIMIT)
            .await
            .map_err(|e| e.to_string());
        emit(Message::Polled {
            project_id,
            group_id,
            result,
        });
    });
}

/// 执行一条经 `route_event` 校验过的命令。`SelectGroup`/`OpenTodo` 不走这里
/// (前者是同步状态变更,后者是切面板),由 `App` 直接处理。
pub fn spawn_command(
    project_id: i64,
    cmd: Command,
    client: &Client,
    handle: &Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let fail = |what: &str, e: anyhow::Error| Message::Failed(project_id, format!("{what}: {e}"));
        match cmd {
            Command::CreateGroup { topic } => match client.create_group(project_id, &topic).await {
                Ok(group) => emit(Message::GroupCreated(project_id, group)),
                Err(e) => emit(fail("新建群聊失败", e)),
            },
            Command::DeleteGroup { group_id } => match client.delete_group(group_id).await {
                Ok(()) => emit(Message::GroupDeleted(project_id, group_id)),
                Err(e) => emit(fail("删除群聊失败", e)),
            },
            Command::AddMember {
                group_id,
                agent,
                handle,
                role_prompt,
            } => match client
                .add_group_member(group_id, agent, &handle, &role_prompt)
                .await
            {
                Ok(group) => emit(Message::GroupChanged(project_id, group)),
                Err(e) => emit(fail("添加成员失败", e)),
            },
            Command::UpdateMember {
                member_id,
                handle,
                role_prompt,
            } => match client
                .update_group_member(member_id, &handle, &role_prompt)
                .await
            {
                Ok(group) => emit(Message::GroupChanged(project_id, group)),
                Err(e) => emit(fail("修改成员失败", e)),
            },
            Command::RemoveMember { member_id } => {
                match client.remove_group_member(member_id).await {
                    Ok(group) => emit(Message::GroupChanged(project_id, group)),
                    Err(e) => emit(fail("移除成员失败", e)),
                }
            }
            Command::Post { group_id, text } => {
                match client.post_group_message(group_id, &text).await {
                    Ok((human, placeholders, unknown)) => emit(Message::Posted {
                        project_id,
                        group_id,
                        human,
                        placeholders,
                        unknown,
                    }),
                    Err(e) => emit(fail("发送失败", e)),
                }
            }
            Command::Cancel { group_id, scope } => {
                if let Err(e) = client.cancel_group(group_id, scope).await {
                    emit(fail("停止失败", e));
                }
            }
            Command::Retry { message_id } => match client.retry_group_message(message_id).await {
                Ok(message) => emit(Message::MessageUpdated(project_id, message)),
                Err(e) => emit(fail("重试失败", e)),
            },
            Command::PushTodo {
                group_id,
                message_id,
                text,
            } => {
                match client.push_group_message_to_todo(message_id, &text).await {
                    Ok(todo) => emit(Message::TodoPushed {
                        project_id,
                        group_id,
                        message_id,
                        todo_id: todo.id,
                    }),
                    Err(e) => emit(fail("转为待办失败", e)),
                }
            }
            Command::SelectGroup { .. } | Command::OpenTodo { .. } => {}
        }
    });
}
```


- [ ] **Step 4: 编译并回归 Task 3 测试**

Run: `cargo test -p dozer-app group_chat 2>&1 | tail -15`
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/group_chat crates/dozer-client/tests/group_chat_client.rs
git commit -m "$(cat <<'EOF'
feat(group-chat): async spawn helpers and client contract test

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: 宿主接线（几何、资源路由、App 状态、IPC、推送、轮询、原生壳）

> 这一步把 Task 2–4 接到应用上，**仍然没有前端**：webview 指向 `dozer://group-chat-content/host.html`，此时资源不存在会 404，面板在 10 秒后回落失败占位页——这正是失败占位页的第一次真实验证。Task 6 之后才有页面。

**Files:**
- Modify: `crates/dozer-app/src/webview_geometry.rs`、`assets.rs`、`app/app.rs`、`app/message.rs`、`app/update.rs`、`app/view.rs`、`workspace/state.rs`、`runtime.rs`、`platform/window_events.rs`
- Create: `crates/dozer-app/src/extensions/group_chat/view.rs`
- Modify: `crates/dozer-app/src/extensions/group_chat/mod.rs`（`mod view;`、`pub use view::content_pane;`）

**Interfaces:**
- Consumes: Task 2–4 的全部。
- Produces: `App::group_chat_webview: group_chat::WebviewPushState`、`GROUP_CHAT_CONTENT_ID_OFFSET: usize = 7_000_000`、`Workspace.group_chat: group_chat::WorkspaceState`、`Message::GroupChat(group_chat::Message)`、`Message::GroupChatContentWebviewEvent(GroupChatWebviewEvent)`、`App::poll_group_chat_if_active`、`App::group_chat_panel_visible`。

- [ ] **Step 1: 几何——写失败的测试**

在 `webview_geometry.rs` 的 `mod tests` 末尾加（`test_state()`、`ShellState`、`MaximizedPane`、`maximized_box_x_range`、`maximized_box_height` 都已存在，见相邻的 `codehealth_content_*` 测试）：

```rust
    #[test]
    fn group_chat_content_zero_when_not_desired() {
        let state = ShellState {
            right_view: PanelKind::GroupChat,
            ..test_state()
        };
        let (_, _, w, h) = group_chat_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &state, false);
        assert_eq!((w, h), (0.0, 0.0));
    }

    #[test]
    fn group_chat_content_zero_when_side_collapsed() {
        let state = ShellState {
            right_view: PanelKind::GroupChat,
            right_collapsed: true,
            ..test_state()
        };
        let (_, _, w, h) = group_chat_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &state, true);
        assert_eq!((w, h), (0.0, 0.0));
    }

    #[test]
    fn group_chat_content_zero_when_panel_kind_is_not_group_chat() {
        let state = test_state();
        let (_, _, w, h) = group_chat_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &state, true);
        assert_eq!((w, h), (0.0, 0.0));
    }

    /// 单列整宽:与"用量面板无筛选栏时内容独占整宽"同一个分支,横向位置与宽度必须一致
    /// (用量面板顶部多一个原生头所以 y/h 不同,只比 x/w)。
    #[test]
    fn group_chat_content_fills_the_whole_zone_width_like_usage_without_list() {
        let gc = ShellState {
            right_view: PanelKind::GroupChat,
            ..test_state()
        };
        let us = ShellState {
            right_view: PanelKind::Usage,
            ..test_state()
        };
        let (gx, _, gw, gh) = group_chat_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &gc, true);
        let (ux, _, uw, _) = usage_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &us, true, false);
        assert!(gw > 100.0 && gh > 100.0, "w={gw} h={gh}");
        assert!((gx - ux).abs() < 0.1 && (gw - uw).abs() < 0.1, "gx={gx} gw={gw} ux={ux} uw={uw}");
    }

    #[test]
    fn group_chat_content_y_starts_at_zone_top() {
        let state = ShellState {
            right_view: PanelKind::GroupChat,
            ..test_state()
        };
        let (_, y, _, _) = group_chat_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &state, true);
        let zone_top = byteui::theme::geometry::top_bar_height() + theme::region::right_zone().margin.top;
        assert!((y - zone_top).abs() < 1.0, "y={y} zone_top={zone_top}");
    }

    #[test]
    fn group_chat_content_maximized_stays_inside_box() {
        let state = ShellState {
            right_view: PanelKind::GroupChat,
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let (x, y, w, h) = group_chat_content_pane_bounds_for(Side::Right, 1600.0, 900.0, &state, true);
        assert!(w > 100.0 && h > 100.0, "w={w} h={h}");
        let (x0, avail_w) = maximized_box_x_range(1600.0);
        assert!(x >= x0 && x + w <= x0 + avail_w, "x={x} w={w}");
        assert!(y + h <= maximized_box_height(900.0) + 200.0);
    }
```

Run: `cargo test -p dozer-app group_chat_content 2>&1 | tail -8` → 编译失败（无该函数）。

- [ ] **Step 2: 几何实现**

`webview_geometry.rs`，紧跟 `todo_content_pane_bounds_for` 之后加：

```rust
/// 群聊面板 webview 矩形:单列整宽(没有原生列表列、没有分隔线),顶部无原生头
/// (`chrome_top = 0`,群切换/成员条都在 webview 里)。与用量面板"无筛选栏时内容
/// 独占整宽"同一个分支(`list_visible = false`)。不可摆放(`!content_desired` /
/// 该侧收起 / 不是 GroupChat / 放大的是另一侧)时返回零尺寸矩形。
pub fn group_chat_content_pane_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
    content_desired: bool,
) -> (f32, f32, f32, f32) {
    pair_content_pane_bounds_for(
        side,
        window_width,
        window_height,
        state,
        PairPane {
            kind: PanelKind::GroupChat,
            // `list_visible = false` 时不参与分栏,取值无意义。
            split: byteui::theme::geometry::default_split_ratio(),
            chrome_top: 0.0,
            content_desired,
            list_visible: false,
            content_first: true,
        },
    )
}
```

并把 `PairPane` 与 `pair_content_pane_bounds_for` 的文档里"两栏面板(Usage、CodeHealth、Todo)"补一句"以及 GroupChat（单列）"。

Run: `cargo test -p dozer-app group_chat_content 2>&1 | tail -8` → PASS。

- [ ] **Step 3: 资源路由——写失败的测试并实现**

`assets.rs` 的 `mod tests` 末尾加（`scratch()` 已存在）：

```rust
    #[test]
    fn group_chat_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("group-chat-content")).unwrap();
        std::fs::write(
            root.with_file_name("group-chat-content").join("host.html"),
            b"<html>g</html>",
        )
        .unwrap();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://group-chat-content/host.html");
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>g</html>");
    }

    #[test]
    fn group_chat_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("group-chat-content")).unwrap();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://group-chat-content/nope");
        assert_eq!(r.status, 404);
    }

    /// 路径穿越不得逃出 group-chat-content 根。
    #[test]
    fn group_chat_content_rejects_path_traversal() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("group-chat-content")).unwrap();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://group-chat-content/../usage-content/host.html",
        );
        assert_eq!(r.status, 404);
    }
```

（"产物齐全 / 严格 CSP"两个测试放到 Task 6，那时才有产物。）

实现：`assets.rs` 在 `todo_content_root_for` 之后加：

```rust
/// group-chat-content host(群聊面板)静态资源根 = flyfish 根的兄弟目录
/// `group-chat-content`。同 `todo_content_root_for`。
fn group_chat_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("group-chat-content")
}
```

`handle_protocol` 里 `todo-content/` 分支之后加：

```rust
    // group-chat-content host(群聊面板):同 todo-content,没有 data.json 特判,
    // 数据全靠 evaluate_script 推送。
    if let Some(path) = rest.strip_prefix("group-chat-content/") {
        return serve_vendored(&group_chat_content_root_for(assets_root), path);
    }
```

并把注释"只服务 …/todo-content 这些命名空间"补上 `group-chat-content`。

Run: `cargo test -p dozer-app assets 2>&1 | tail -10` → PASS。

- [ ] **Step 4: 原生壳 `view.rs`**

创建 `extensions/group_chat/view.rs`（仿 `codehealth::content_pane`，但只有内容壳）：

```rust
//! 群聊面板原生壳:webview 盖在这块 `container` 之上,它只负责面板背景/边框;
//! `failed` 为 `Some` 时 webview 不挂载,这里显示失败原因与"重试"。内容渲染
//! 全部在 `dozer://group-chat-content` webview 里(见 `web/group-chat-content/`)。

use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, text};

/// 原生壳发出的消息:只有"重试加载"。
#[derive(Debug, Clone)]
pub enum ShellMessage {
    Retry,
}

pub fn content_pane(
    failed: Option<&str>,
    outer: Border,
) -> Element<'static, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> =
        match failed {
            Some(reason) => column![
                text("群聊页面加载失败").size(14).color(tokens.body),
                text(reason.to_string()).size(12).color(tokens.dim),
                button(text("重试").size(13)).on_press(ShellMessage::Retry),
            ]
            .spacing(10)
            .padding(16)
            .into(),
            None => column![].into(),
        };
    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().panel.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

`mod.rs` 加 `mod view;` 与 `pub use view::{ShellMessage, content_pane};`。

- [ ] **Step 5: App 状态、消息、IPC、推送、挂载**

**`app/app.rs`**：

1. 常量（接在 `TODO_CONTENT_ID_OFFSET` 之后）：

```rust
/// 群聊面板 webview 的固定单槽位 ID(接在 `TODO_CONTENT_ID_OFFSET` 之后,避免与其它
/// 偏移冲突)。
pub(crate) const GROUP_CHAT_CONTENT_ID_OFFSET: usize = 7_000_000;
```

2. `App` 结构体字段（与 `codehealth_webview` 并列，`todo_webview` 附近）：

```rust
    pub(crate) group_chat_webview: crate::extensions::group_chat::WebviewPushState,
```

构造处（`codehealth_webview: …default()` 附近）加 `group_chat_webview: crate::extensions::group_chat::WebviewPushState::default(),`。

3. 推送消费点（紧跟 `take_todo_content_script`）：

```rust
    /// 群聊面板待下发推送。声明式:每帧比较"当前该显示什么"
    /// (`current_view_payload`)与"上次送达的"(`group_chat_webview.pending_push`),
    /// 同时驱动加载超时判定。已失败时不推送——原生占位页接管。
    pub fn take_group_chat_content_script(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
        now: std::time::Instant,
    ) -> Vec<(usize, String)> {
        let webview_id = GROUP_CHAT_CONTENT_ID_OFFSET;
        self.group_chat_webview
            .observe_availability(available_webview_ids.contains(&webview_id), now);
        if !available_webview_ids.contains(&webview_id)
            || self.group_chat_webview.failed().is_some()
        {
            return Vec::new();
        }
        let Some(project_id) = self.active_project_id else {
            return Vec::new();
        };
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let desired = crate::extensions::group_chat::current_view_payload(&ws.group_chat, project_id);
        let Some(payload) = self.group_chat_webview.pending_push(&desired) else {
            return Vec::new();
        };
        let revision = self.group_chat_webview.mark_sent(payload.clone());
        let envelope = crate::extensions::group_chat::encode_group_chat_push(revision, payload);
        vec![(webview_id, crate::preview::dispatch_script(&envelope))]
    }
```

4. 挂载（`desired_webviews` 里 `if kind == PanelKind::Todo {…}` 之后、`let (mut specs, id_offset)` 之前）：

```rust
            if kind == PanelKind::GroupChat {
                // 已失败(加载超时/渲染异常)时不挂载,原生占位页接管。
                let content_desired = self.group_chat_webview.failed().is_none();
                let bounds = crate::webview_geometry::group_chat_content_pane_bounds_for(
                    side,
                    window_width,
                    window_height,
                    &self.shell_state(),
                    content_desired,
                );
                if bounds.2 > 0.0 && bounds.3 > 0.0 {
                    let spec = WebviewSpec {
                        id: GROUP_CHAT_CONTENT_ID_OFFSET,
                        url: format!(
                            "dozer://group-chat-content/host.html?theme={}",
                            crate::preview::scheme_query_value()
                        ),
                        visible: !app_modal_open,
                        editor_binding: None,
                        loading_generation: None,
                        park_offscreen: false,
                    };
                    out.push((spec, bounds));
                }
                continue;
            }
```

5. 面板可见性与轮询（紧跟 `poll_todo_if_visible`）：

```rust
    /// 群聊面板当前是否"正被看着":在某一侧显示、该侧未收起,且没有被另一侧的放大态盖住。
    pub fn group_chat_panel_visible(&self) -> bool {
        if self.active_workspace().is_none() {
            return false;
        }
        let left = self.left_view == PanelKind::GroupChat && !self.left_collapsed;
        let right = self.right_view == PanelKind::GroupChat && !self.right_collapsed;
        match self.maximized {
            Some(MaximizedPane::Left) => left,
            Some(MaximizedPane::Right) => right,
            None => left || right,
        }
    }

    /// `about_to_wait` 是否需要为群聊排下一拍唤醒:面板可见,且(需要加载群列表 或
    /// 本群有发言进行中)。没有发言时不轮询,不空转。
    pub fn group_chat_poll_wanted(&self) -> bool {
        self.group_chat_panel_visible()
            && self
                .active_workspace()
                .is_some_and(|ws| ws.group_chat.load_pending() || ws.group_chat.has_active_turn())
    }

    /// `ResumeTimeReached` 时调用:该加载就加载,该轮询就轮询(限速、在途、退避都在
    /// `WorkspaceState` 里)。
    pub fn poll_group_chat_if_active(&mut self) {
        if !self.group_chat_panel_visible() {
            return;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let now = std::time::Instant::now();
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
        let (Some(group_id), true) = (
            ws.group_chat.selected(),
            ws.group_chat.has_active_turn(),
        ) else {
            return;
        };
        if ws.group_chat.poll_due(now) {
            let after_rev = ws.group_chat.latest_rev();
            crate::extensions::group_chat::spawn_fetch_messages(
                project_id, group_id, after_rev, &client, &handle, emit,
            );
        }
    }
```

（`MaximizedPane` 已在 `app.rs` 里可用，见 `maximized: Option<MaximizedPane>`。`active_workspace_mut` 名称以代码里实际为准——`with_focused_project` 里用的就是 `self.active_workspace_mut()`。）

**`app/message.rs`**：`use crate::extensions::{…, group_chat, …};`；在 `CodeHealth(codehealth::Message),` 之后加：

```rust
    /// 群聊面板的全部消息,内核只转发不解读——见 `extensions::group_chat::Message`。
    GroupChat(group_chat::Message),
```

并在 `CodeHealthContentWebviewEvent(…)` 之后加：

```rust
    GroupChatContentWebviewEvent(crate::extensions::group_chat::GroupChatWebviewEvent),
    /// 群聊原生壳(失败占位页)的"重试"。
    GroupChatShell(crate::extensions::group_chat::ShellMessage),
```

**`workspace/state.rs`**：字段（`codehealth` 附近）`pub(crate) group_chat: group_chat::WorkspaceState,`；构造处 `group_chat: group_chat::WorkspaceState::default(),`；项目重置处（`self.codehealth = codehealth::WorkspaceState::default();` 附近，约 1102 行）加 `self.group_chat = group_chat::WorkspaceState::default();`；`use` 里补 `group_chat`。

**`app/update.rs`**：

1. 事件分发（紧跟 `Message::TodoContentWebviewEvent(event) => …`）：

```rust
            Message::GroupChatContentWebviewEvent(event) => self.group_chat_content_event(event),
            Message::GroupChatShell(crate::extensions::group_chat::ShellMessage::Retry) => {
                self.group_chat_webview.clear_failed();
            }
            Message::GroupChat(msg) => {
                let project_id = msg.project_id();
                let mut effects = Vec::new();
                self.with_project(project_id, |ws, _io| {
                    effects = crate::extensions::group_chat::update(&mut ws.group_chat, msg);
                });
                self.run_group_chat_effects(project_id, effects);
            }
```

2. 处理函数（紧跟 `todo_content_event`）：

```rust
    pub(crate) fn group_chat_content_event(
        &mut self,
        event: crate::extensions::group_chat::GroupChatWebviewEvent,
    ) {
        use crate::extensions::group_chat::{self as gc, GroupChatWebviewEvent as Ev};
        match &event {
            Ev::Ready => {
                self.group_chat_webview.set_ready(true);
                return;
            }
            Ev::Failed { reason } => {
                dozer_core::log_warn!(
                    gc::LOG,
                    panel = "group_chat",
                    %reason,
                    "群聊内容页渲染失败,回落原生占位"
                );
                self.group_chat_webview.set_failed(reason.clone());
                return;
            }
            _ => {}
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let cmd = self
            .active_workspace()
            .and_then(|ws| gc::route_event(&ws.group_chat, event));
        if let Some(cmd) = cmd {
            self.group_chat_command(project_id, cmd);
        }
    }

    fn group_chat_command(&mut self, project_id: i64, cmd: crate::extensions::group_chat::Command) {
        use crate::extensions::group_chat::{self as gc, Command};
        match cmd {
            Command::SelectGroup { group_id } => {
                let mut effects = Vec::new();
                self.with_project(project_id, |ws, _io| {
                    effects = ws.group_chat.select(group_id);
                });
                self.run_group_chat_effects(project_id, effects);
            }
            Command::OpenTodo { .. } => {
                // 只确保 Todo 面板看得见;不做"定位到某条待办"(Todo 面板没有这个入口,
                // 且不在本功能范围)。**不能用 `panel_select`**:它是图标栏点击的处理器,
                // 对已激活的面板是"收起",还会武装图标栏拖拽(审阅发现的缺陷)。
                self.show_panel(PanelKind::Todo);
            }
            other => {
                let proxy = self.proxy.clone();
                gc::spawn_command(
                    project_id,
                    other,
                    &self.client.clone(),
                    &self.handle.clone(),
                    move |m| {
                        let _ = proxy.send_event(Message::GroupChat(m));
                    },
                );
            }
        }
    }

    fn run_group_chat_effects(
        &mut self,
        project_id: i64,
        effects: Vec<crate::extensions::group_chat::Effect>,
    ) {
        use crate::extensions::group_chat::{self as gc, Effect};
        for effect in effects {
            match effect {
                Effect::FetchMessages { group_id } => {
                    let proxy = self.proxy.clone();
                    gc::spawn_fetch_messages(
                        project_id,
                        group_id,
                        0,
                        &self.client.clone(),
                        &self.handle.clone(),
                        move |m| {
                            let _ = proxy.send_event(Message::GroupChat(m));
                        },
                    );
                }
            }
        }
    }
```

3. 面板切入动作：`panel_select` 里把 Task 1 的 `| PanelKind::GroupChat => {}` 拆出来：

```rust
                // 切入时刷新群列表(别的入口可能新增过群);`mark_stale` 不清已有内容,
                // 切入瞬间不会闪成"没有群聊"。实际加载由 `poll_group_chat_if_active` 发起。
                PanelKind::GroupChat => self.with_focused_project(|ws, _io| {
                    ws.group_chat.mark_stale();
                }),
                PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
```


**`runtime.rs`**：IPC 分发里，在 `todo-content` 分支之后加：

```rust
                                } else if webview_id == crate::app::GROUP_CHAT_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // group-chat-content 同 todo-content:固定单槽,按固定
                                    // webview id 识别,不复用任何 binding。
                                    match crate::extensions::group_chat::parse_group_chat_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::GroupChatContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 group-chat-content IPC");
                                        }
                                    }
```

**`platform/window_events.rs`**：

1. 推送（紧跟 todo 那段）：

```rust
        // 群聊面板 webview(单固定槽):声明式推送,同 Todo 节奏。
        for (webview_id, js) in
            app.take_group_chat_content_script(&available_webview_ids, std::time::Instant::now())
        {
            if let Some((view, _)) = webviews.get(&webview_id) {
                let _ = view.evaluate_script(&js);
            }
        }
```

2. `new_events` 里 `app.poll_todo_if_visible();` 之后加 `app.poll_group_chat_if_active();`。
3. `about_to_wait` 的 `wakes` 数组长度 7 → 8，加一项：

```rust
                (app.group_chat_poll_wanted(), crate::extensions::group_chat::POLL_INTERVAL),
```

**`app/update.rs` 的 `drain_outboxes`**：在 `pending.extend(ws.todo.take_outbox());` 之后加 `pending.extend(ws.group_chat.take_outbox());`。

**`app/view.rs`**：把 Task 1 的占位分支替换为：

```rust
        PanelKind::GroupChat => crate::extensions::group_chat::content_pane(
            app.group_chat_webview.failed(),
            zone_pane_border(zone, lc),
        )
        .map(Message::GroupChatShell),
```

**`show_panel`（`OpenTodo` 用）**：在 `chrome/rail.rs` 加纯函数 `show_panel_in(side, kind, view, collapsed, maximized) -> ShowPanel`（字段 `view/collapsed/maximized/switched/changed`）：已看得见（且没被对侧放大态盖住）→ `changed = false` 什么都不做；被收起 → 展开；显示的是别的 → 切过去；被对侧放大盖住 → 退出放大态；凡有改动一律清 `maximized`（同 `panel_select` 口径）。单测覆盖这几种情形，并有回归测试"已展开的面板绝不被收起"。`App::show_panel(kind)` 调它，应用结果后调 `fire_panel_switch_in(kind)` 与 `on_shell_layout_changed()`。为此把 `panel_select` 里的"切入时动作"`match` 抽成 `fn fire_panel_switch_in(&mut self, kind)`，图标栏点击与程序化显示共用，`panel_select` 行为不变。

- [ ] **Step 6: 编译与测试**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: unused)" -A6 | head -40`，逐个修到零错误；Run: `cargo test -p dozer-app 2>&1 | tail -15`。
Expected: 全 PASS，与 Task 0 基线相比无新增失败。再跑 `scripts/check-log-scope.sh`、`cargo clippy -p dozer-app --all-targets 2>&1 | tail -20`、`cargo fmt`。

- [ ] **Step 7: 手动冒烟（本机有显示环境时）**

`cargo run -p dozer-app`，打开一个项目，点右栏群聊图标：面板区是空的深色底；约 10 秒后出现"群聊页面加载失败 / 面板页面加载超时 / 重试"——**这是预期**（前端尚未构建）。点"重试"再等 10 秒又回落。切到别的面板再切回来无崩溃。

- [ ] **Step 8: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src
git commit -m "$(cat <<'EOF'
feat(group-chat): wire panel host (geometry, assets route, IPC, push, polling)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 前端脚手架、构建与产物

**Files:**
- Create: `crates/dozer-app/web/group-chat-content/{package.json,build.mjs,tsconfig.json,render-smoke.mjs}`
- Create: `crates/dozer-app/web/group-chat-content/src/{host.html,ipc.ts,types.ts,protocol.ts,protocol.test.ts,errors.ts,errors.test.ts,styles.css,main.tsx,fixtures.ts,render-smoke.tsx}`
- Create: `crates/dozer-app/web/group-chat-content/src/components/App.tsx`（先放最小壳，Task 7–10 逐步充实）
- Create（构建产物，提交）: `crates/dozer-app/assets/group-chat-content/{host.html,group-chat-content.js,group-chat-content.css}`
- Modify: `.gitignore`、`crates/dozer-app/src/assets.rs`（加产物齐全 / CSP 测试）

**Interfaces:**
- Produces: `src/types.ts` 里与 Task 3 payload 一一对应的类型 `ViewPayload`、`OutEvent`（后续所有前端 Task 依赖，字段名与 Rust `serde` 输出完全一致）。

- [ ] **Step 1: 搭脚手架**

```bash
cd crates/dozer-app/web && mkdir -p group-chat-content/src/components && cd group-chat-content
```

`package.json`：

```json
{
  "name": "dozer-group-chat-content",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Dozer 群聊面板(group-chat-content),Preact 离线打包,无 CDN/无运行时 Node。",
  "scripts": {
    "build": "node build.mjs",
    "typecheck": "tsc --noEmit",
    "test": "node --test src/*.test.ts && node render-smoke.mjs"
  },
  "dependencies": {
    "preact": "10.29.8"
  },
  "devDependencies": {
    "@types/node": "^24",
    "esbuild": "0.28.2",
    "preact-render-to-string": "6.6.3",
    "typescript": "5.9.3"
  }
}
```

`tsconfig.json`：与 `web/todo-content/tsconfig.json` 完全相同（原样复制）。

`build.mjs`（把 `todo-content` 换成 `group-chat-content`）：

```js
// 生产构建:把群聊面板打包成离线、无 CDN、无运行时 Node 的确定性产物到
// `crates/dozer-app/assets/group-chat-content/`。不输出 source map;minify 后去掉
// 所有 legal comments。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/group-chat-content');

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

await build({
  entryPoints: [path.join(here, 'src/main.tsx')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'css' },
  outfile: path.join(outdir, 'group-chat-content.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
```

`render-smoke.mjs`：与 `web/todo-content/render-smoke.mjs` 相同，仅把临时目录前缀 `todo-smoke-` 改 `group-chat-smoke-`。

`src/host.html`：

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8" />
<meta
  http-equiv="Content-Security-Policy"
  content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'"
/>
<title>group-chat</title>
<link rel="stylesheet" href="group-chat-content.css" />
</head>
<body>
<div id="root"></div>
<script src="group-chat-content.js"></script>
</body>
</html>
```

`src/ipc.ts`：与 todo 版相同（`send(event: OutEvent)`，`window.ipc?.postMessage(JSON.stringify(event))`）。

`src/errors.ts` 与 `errors.test.ts`：与 todo 版相同，仅把 `filename.includes('todo-content')` 改 `'group-chat-content'`，测试里的 URL 同步改 `dozer://group-chat-content/group-chat-content.js`。

`.gitignore` 在 `todo-content/node_modules/` 那组之后加：

```
# group-chat-content host deps (built assets under assets/group-chat-content are committed)
crates/dozer-app/web/group-chat-content/node_modules/
```

- [ ] **Step 2: 类型（与 Rust payload 对齐）**

`src/types.ts`：

```ts
export type AgentKey = 'claude' | 'codex';
export type StatusKey = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';
export type AuthorKind = 'human' | 'member' | 'system';

export interface Author {
  kind: AuthorKind;
  /** "你" / "@handle" / "已移除成员" / "系统" */
  label: string;
  /** "Claude" / "Codex",成员消息才有 */
  agent_label: string | null;
  /** 0..3,已移除成员/非成员为 null */
  color_slot: number | null;
}

export interface Member {
  id: number;
  agent: AgentKey;
  agent_label: string;
  handle: string;
  role_prompt: string;
  color_slot: number;
}

export interface Group {
  id: number;
  topic: string;
  members: Member[];
}

export interface Message {
  id: number;
  seq: number;
  author: Author;
  text: string;
  /** human/系统消息为 null */
  status: StatusKey | null;
  reason: string | null;
  duration_ms: number | null;
  todo_id: number | null;
}

export interface ViewPayload {
  project_id: number;
  loaded: boolean;
  groups: Group[];
  selected_group_id: number | null;
  messages: Message[];
  hint: string | null;
  running: boolean;
}

export type CancelScope = 'turn' | 'round';

export type OutEvent =
  | { kind: 'ready' }
  | { kind: 'failed'; reason: string }
  | { kind: 'create_group'; topic: string }
  | { kind: 'select_group'; group_id: number }
  | { kind: 'delete_group'; group_id: number }
  | { kind: 'add_member'; group_id: number; agent: AgentKey; handle: string; role_prompt: string }
  | { kind: 'update_member'; member_id: number; handle: string; role_prompt: string }
  | { kind: 'remove_member'; member_id: number }
  | { kind: 'post'; group_id: number; text: string }
  | { kind: 'cancel'; group_id: number; scope: CancelScope }
  | { kind: 'retry'; message_id: number }
  | { kind: 'push_todo'; message_id: number; text: string }
  | { kind: 'open_todo'; todo_id: number };
```

`src/protocol.ts`：与 todo 版相同（`PushState`、`applyEnvelope`，`ViewPayload` 从 `./types.ts` 引）。`src/protocol.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope } from './protocol.ts';

const payload = (project_id: number) => ({
  project_id,
  loaded: true,
  groups: [],
  selected_group_id: null,
  messages: [],
  hint: null,
  running: false,
});
const env = (revision: number, project_id: number) =>
  JSON.stringify({ protocol_version: 1, revision, payload: payload(project_id) });

test('applies a newer revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(1, 7));
  assert.equal(s1.revision, 1);
  assert.equal(s1.payload!.project_id, 7);
});

test('drops an older revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(5, 1));
  const s2 = applyEnvelope(s1, env(3, 2));
  assert.equal(s2.revision, 5);
  assert.equal(s2.payload!.project_id, 1);
});

test('accepts an equal revision (idempotent re-push)', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(4, 1));
  const s2 = applyEnvelope(s1, env(4, 1));
  assert.equal(s2.revision, 4);
});

test('ignores malformed json and missing payload', () => {
  const s1 = applyEnvelope({ revision: 2, payload: null }, 'not json');
  assert.equal(s1.revision, 2);
  const s2 = applyEnvelope(s1, JSON.stringify({ revision: 9 }));
  assert.equal(s2.revision, 2);
});
```

`src/main.tsx`：与 todo 版相同（`Root`、`applyThemeFromUrl`、`send({kind:'ready'})`），只把 `import { App } from './components/App.tsx'` 保持、注释里 `dozer://todo-content`/`codehealth-content` 换成 `group-chat-content`。

`src/styles.css`：先放主题变量与基础（后续 Task 追加样式）：

```css
:root[data-theme="dark"] {
  --bg: #0d131c;
  --panel: #0a0e16;
  --card: #12202a;
  --card-hover: #162a36;
  --border: #1c3440;
  --cream: #FFE5B4;
  --body: #c9d4dc;
  --dim: #6B7F8F;
  --gold: #F2D94E;
  --cyan: #47DEF0;
  --green: #1AD585;
  --purple: #B79CFF;
  --orange: #FFA94D;
  --red: #FF5C5C;
  --shadow: 0 1px 2px rgba(0, 0, 0, 0.45), 0 4px 14px rgba(0, 0, 0, 0.25);
}
:root[data-theme="light"] {
  --bg: #fffdf6;
  --panel: #fef2e4;
  --card: #fefdfb;
  --card-hover: #fff8ec;
  --border: #d7dfe5;
  --cream: #16232e;
  --body: #2b3a46;
  --dim: #4c5c68;
  --gold: #118b96;
  --cyan: #0e8a9e;
  --green: #128f5a;
  --purple: #6b4fd1;
  --orange: #b45f06;
  --red: #c0392b;
  --shadow: 0 1px 2px rgba(22, 35, 46, 0.12), 0 4px 14px rgba(22, 35, 46, 0.08);
}
* { box-sizing: border-box; }
html, body {
  margin: 0; height: 100%;
  background: var(--bg); color: var(--body);
  font: 13px/1.5 -apple-system, "PingFang SC", sans-serif;
}
#root { height: 100%; }
```

`src/components/App.tsx`（最小壳）：

```tsx
import type { ViewPayload } from '../types.ts';

export function App({ payload }: { payload: ViewPayload }) {
  if (!payload.loaded) return <div class="gc-empty">加载中…</div>;
  if (payload.groups.length === 0) return <div class="gc-empty">还没有群聊</div>;
  return <div class="gc-root">{payload.groups.length} 个群聊</div>;
}
```

`src/fixtures.ts`（后续 Task 扩充）：

```ts
import type { ViewPayload } from './types.ts';

export const unloadedFixture: ViewPayload = {
  project_id: 1, loaded: false, groups: [], selected_group_id: null, messages: [], hint: null, running: false,
};
export const noGroupsFixture: ViewPayload = { ...unloadedFixture, loaded: true };
```

`src/render-smoke.tsx`：

```tsx
import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import { unloadedFixture, noGroupsFixture } from './fixtures.ts';

test('unloaded shows loading, not the empty-state', () => {
  const out = render(<App payload={unloadedFixture} />);
  assert.match(out, /加载中/);
  assert.doesNotMatch(out, /还没有群聊/);
});

test('loaded with no groups shows the empty-state', () => {
  assert.match(render(<App payload={noGroupsFixture} />), /还没有群聊/);
});
```

- [ ] **Step 3: 安装、测试、构建**

```bash
npm install
npm run typecheck
npm test
npm run build
ls -la ../../assets/group-chat-content
```

Expected：类型检查与测试全过；`assets/group-chat-content/` 下有 `host.html`、`group-chat-content.js`、`group-chat-content.css`（CSS 由 esbuild 从 `main.tsx` 里 `import './styles.css'` 抽出）。`package-lock.json` 生成并提交。

- [ ] **Step 4: Rust 侧产物齐全 / CSP 测试**

`assets.rs` 的 `mod tests` 末尾加（仿 `codehealth_content_*`）：

```rust
    /// 提交的 group-chat-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn group_chat_content_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/group-chat-content"));
        for f in ["host.html", "group-chat-content.js", "group-chat-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 group-chat-content 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "group-chat-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src、无网络引用。
    #[test]
    fn group_chat_content_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/group-chat-content"));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("connect-src"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("group-chat-content.js") && html.contains("group-chat-content.css"));
    }
```

Run: `cargo test -p dozer-app group_chat_content 2>&1 | tail -10` → PASS。

- [ ] **Step 5: 手动冒烟**

`cargo run -p dozer-app`，打开项目，点群聊图标：不再回落失败页，显示"加载中…"随后"还没有群聊"（dozerd 在跑且项目下没有群时）。

- [ ] **Step 6: 提交**

```bash
git branch --show-current
git add .gitignore crates/dozer-app/web/group-chat-content crates/dozer-app/assets/group-chat-content crates/dozer-app/src/assets.rs
git commit -m "$(cat <<'EOF'
feat(group-chat): preact frontend scaffold, build and committed bundle

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

> 此后每个前端 Task 末尾都要 `npm run build` 并把更新后的 `assets/group-chat-content/*` 一并提交（产物是运行时用的，不是副产品）。

---

### Task 7: 安全 Markdown 渲染器（纯函数）

**Files:**
- Create: `crates/dozer-app/web/group-chat-content/src/markdown.ts`、`markdown.test.ts`

**Interfaces:**
- Produces: `export function renderMarkdown(src: string): string`（返回可直接 `dangerouslySetInnerHTML` 的 HTML 字符串；**输入里的任何 `<`、`>`、`&`、引号在输出里都被转义**）。

- [ ] **Step 1: 写失败的测试**

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { renderMarkdown } from './markdown.ts';

// Review Focus 5:agent 输出里的 HTML/脚本必须以纯文本显示。
test('raw html is escaped, never emitted as tags', () => {
  const out = renderMarkdown('<script>alert(1)</script><img src=x onerror=alert(1)>');
  assert.doesNotMatch(out, /<script/i);
  assert.doesNotMatch(out, /<img/i);
  assert.match(out, /&lt;script&gt;/);
  assert.match(out, /&lt;img/);
});

test('javascript: and other links are plain text, not anchors', () => {
  const out = renderMarkdown('[点我](javascript:alert(1)) 和 <a href="x">y</a> 和 https://example.com');
  assert.doesNotMatch(out, /<a[\s>]/i);
  assert.doesNotMatch(out, /href="/i); // 转义后只会剩 `href=&quot;`,不会有真正的属性
  assert.match(out, /javascript:alert\(1\)/);
});

test('attribute-breaking quotes in text are escaped', () => {
  const out = renderMarkdown('"><svg onload=alert(1)>');
  assert.doesNotMatch(out, /<svg/i);
  assert.match(out, /&quot;/);
});

test('paragraphs and line breaks', () => {
  assert.equal(renderMarkdown('第一段\n\n第二段'), '<p>第一段</p><p>第二段</p>');
  assert.equal(renderMarkdown('甲\n乙'), '<p>甲<br>乙</p>');
});

test('bold, italic and inline code', () => {
  assert.equal(renderMarkdown('**粗** 和 *斜* 和 `code`'), '<p><strong>粗</strong> 和 <em>斜</em> 和 <code>code</code></p>');
});

test('inline code content is escaped and not re-parsed as markdown', () => {
  assert.equal(renderMarkdown('`<b>**x**</b>`'), '<p><code>&lt;b&gt;**x**&lt;/b&gt;</code></p>');
});

test('fenced code block keeps content verbatim (escaped) and ignores language tag injection', () => {
  const out = renderMarkdown('```js"><x>\nconst a = "<b>";\n**not bold**\n```');
  assert.match(out, /<pre><code>/);
  assert.match(out, /const a = &quot;&lt;b&gt;&quot;;/);
  assert.match(out, /\*\*not bold\*\*/);
  assert.doesNotMatch(out, /<x>/);
});

test('unterminated fence is rendered as code to the end (streaming-friendly)', () => {
  const out = renderMarkdown('说明\n```\nlet x = 1;');
  assert.match(out, /<p>说明<\/p>/);
  assert.match(out, /<pre><code>let x = 1;<\/code><\/pre>/);
});

test('unordered and ordered lists', () => {
  assert.equal(renderMarkdown('- 甲\n- 乙'), '<ul><li>甲</li><li>乙</li></ul>');
  assert.equal(renderMarkdown('1. 一\n2. 二'), '<ol><li>一</li><li>二</li></ol>');
  assert.equal(renderMarkdown('* 甲\n* 乙'), '<ul><li>甲</li><li>乙</li></ul>');
});

test('list items support inline formatting', () => {
  assert.equal(renderMarkdown('- **重点** 与 `x`'), '<ul><li><strong>重点</strong> 与 <code>x</code></li></ul>');
});

test('headings map to h3-h5 (panel-sized) regardless of level', () => {
  assert.match(renderMarkdown('# 大'), /^<h3>大<\/h3>$/);
  assert.match(renderMarkdown('### 小'), /^<h5>小<\/h5>$/);
  assert.match(renderMarkdown('###### 最小'), /^<h5>最小<\/h5>$/);
});

test('hash without space is not a heading', () => {
  assert.equal(renderMarkdown('#标签'), '<p>#标签</p>');
});

test('emphasis markers inside words with underscores are left alone', () => {
  assert.equal(renderMarkdown('snake_case_name'), '<p>snake_case_name</p>');
});

test('unmatched emphasis markers stay literal', () => {
  assert.equal(renderMarkdown('2 * 3 = 6 and **未闭合'), '<p>2 * 3 = 6 and **未闭合</p>');
});

test('empty and whitespace-only input renders nothing', () => {
  assert.equal(renderMarkdown(''), '');
  assert.equal(renderMarkdown('  \n\n  '), '');
});

test('very long single line does not blow up', () => {
  const out = renderMarkdown('字'.repeat(50_000));
  assert.ok(out.startsWith('<p>') && out.endsWith('</p>'));
});

test('crlf line endings are normalized', () => {
  assert.equal(renderMarkdown('甲\r\n\r\n乙'), '<p>甲</p><p>乙</p>');
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd crates/dozer-app/web/group-chat-content && node --test src/markdown.test.ts`
Expected: FAIL（找不到 `./markdown.ts`）。

- [ ] **Step 3: 实现**

```ts
// 群聊消息的最小 Markdown 渲染器。agent 输出不可信,所以这里的原则是:
//   1. **先整体转义**,再在转义后的文本上套固定的标签——输出里不可能出现输入带来的标签;
//   2. 不支持原始 HTML、链接、图片(一律以纯文本显示);
//   3. 只做:标题、段落、有序/无序列表、粗体、斜体、行内代码、围栏代码块、换行。
// 不引入第三方库(见 plan Ruling 4)。

const ESC: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ESC[c]);
}

/** 行内:行内代码先抽走占位(内容不再解析),其余先转义再套粗体/斜体。 */
function renderInline(text: string): string {
  const codes: string[] = [];
  const withoutCode = text.replace(/`([^`\n]+)`/g, (_m, c: string) => {
    codes.push(`<code>${escapeHtml(c)}</code>`);
    return `\u0000${codes.length - 1}\u0000`;
  });
  let out = escapeHtml(withoutCode);
  // 粗体先于斜体;要求标记内侧不是空白,避免 `2 * 3 * 4`。
  out = out.replace(/\*\*(?=\S)([^*\n]*?\S)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/(^|[^*\w])\*(?=\S)([^*\n]*?\S)\*(?!\*)/g, '$1<em>$2</em>');
  return out.replace(/\u0000(\d+)\u0000/g, (_m, i: string) => codes[Number(i)]);
}

const FENCE = /^```/;
const BULLET = /^\s*[-*]\s+(.*)$/;
const ORDERED = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;

export function renderMarkdown(src: string): string {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const html: string[] = [];
  let i = 0;

  const flushParagraph = (buf: string[]) => {
    if (buf.length === 0) return;
    html.push(`<p>${buf.map(renderInline).join('<br>')}</p>`);
    buf.length = 0;
  };

  let para: string[] = [];
  while (i < lines.length) {
    const line = lines[i];

    if (FENCE.test(line)) {
      flushParagraph(para);
      const code: string[] = [];
      i++;
      // 没有收尾围栏时(流式输出中途)一直读到结尾。
      while (i < lines.length && !FENCE.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      i++; // 跳过收尾围栏(若有)
      html.push(`<pre><code>${escapeHtml(code.join('\n'))}</code></pre>`);
      continue;
    }

    if (line.trim() === '') {
      flushParagraph(para);
      i++;
      continue;
    }

    const h = HEADING.exec(line);
    if (h) {
      flushParagraph(para);
      const level = Math.min(5, Math.max(3, h[1].length + 2));
      html.push(`<h${level}>${renderInline(h[2])}</h${level}>`);
      i++;
      continue;
    }

    const isBullet = BULLET.test(line);
    const isOrdered = !isBullet && ORDERED.test(line);
    if (isBullet || isOrdered) {
      flushParagraph(para);
      const re = isBullet ? BULLET : ORDERED;
      const items: string[] = [];
      while (i < lines.length) {
        const m = re.exec(lines[i]);
        if (!m) break;
        items.push(`<li>${renderInline(m[1])}</li>`);
        i++;
      }
      html.push(isBullet ? `<ul>${items.join('')}</ul>` : `<ol>${items.join('')}</ol>`);
      continue;
    }

    para.push(line);
    i++;
  }
  flushParagraph(para);
  return html.join('');
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `node --test src/markdown.test.ts`
Expected: 全部 PASS。若 `snake_case_name` 或 `2 * 3 = 6 and **未闭合` 用例失败，说明斜体正则误吞，**收紧正则而不是改测试**。

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add crates/dozer-app/web/group-chat-content/src/markdown.ts crates/dozer-app/web/group-chat-content/src/markdown.test.ts
git commit -m "$(cat <<'EOF'
feat(group-chat): safe minimal markdown renderer for agent output

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: handle 校验与 `@` 补全（纯函数）

**Files:**
- Create: `web/group-chat-content/src/handle.ts`、`handle.test.ts`、`format.ts`、`format.test.ts`

**Interfaces:**
- Produces:

```ts
export function validateHandle(handle: string, takenKeys: string[]): string | null;   // 中文错误或 null
export function handleKey(handle: string): string;                                     // 小写化
export function suggestHandle(agent: 'claude' | 'codex', takenKeys: string[]): string;
export interface MentionContext { start: number; query: string }
export function mentionContext(text: string, caret: number): MentionContext | null;
export function filterMembers<T extends { handle: string }>(members: T[], query: string): T[];
export function applyMention(text: string, ctx: MentionContext, caret: number, handle: string): { text: string; caret: number };
export function mentionMenuOpen(ctx: MentionContext | null, dismissedStart: number | null): boolean;  // Esc 关闭后按 `@` 位置记住
export function nextDismissed(ctx: MentionContext | null, dismissedStart: number | null): number | null;
export function formatDuration(ms: number | null): string;
```

> `validateHandle` 镜像 Rust `dozerd::group_mentions::validate_handle`（空、>32 字符、含空白/`@`/终止标点），**两边必须同步**；Rust 侧是权威，这里只为"提交前给出即时提示"。终止标点集合从 `crates/dozerd/src/group_mentions.rs` 的 `TERMINATORS` 原样抄。

- [ ] **Step 1: 写失败的测试**

`handle.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  validateHandle, handleKey, suggestHandle, mentionContext, filterMembers, applyMention,
  mentionMenuOpen, nextDismissed,
} from './handle.ts';

test('validateHandle accepts normal handles incl. chinese', () => {
  assert.equal(validateHandle('claude', []), null);
  assert.equal(validateHandle('架构师', []), null);
});

test('validateHandle rejects empty, too long, whitespace, @, terminator punctuation', () => {
  assert.match(validateHandle('', [])!, /不能为空/);
  assert.match(validateHandle('x'.repeat(33), [])!, /32/);
  assert.ok(validateHandle('a b', []));
  assert.ok(validateHandle('a@b', []));
  assert.ok(validateHandle('评审，员', []));
  assert.ok(validateHandle('a.b', []));
});

test('validateHandle rejects duplicates ignoring case', () => {
  assert.match(validateHandle('Claude', ['claude'])!, /已有/);
  assert.equal(validateHandle('claude2', ['claude']), null);
});

test('handleKey lowercases', () => {
  assert.equal(handleKey('ClAuDe'), 'claude');
});

test('suggestHandle prefers the agent name, then numbers', () => {
  assert.equal(suggestHandle('claude', []), 'claude');
  assert.equal(suggestHandle('claude', ['claude']), 'claude2');
  assert.equal(suggestHandle('codex', ['codex', 'codex2']), 'codex3');
});

// ---- @ 补全 ----

test('mentionContext finds the @query before the caret', () => {
  assert.deepEqual(mentionContext('你好 @cla', 7), { start: 3, query: 'cla' });
  assert.deepEqual(mentionContext('@', 1), { start: 0, query: '' });
  assert.deepEqual(mentionContext('看下@架构', 5), { start: 2, query: '架构' });
});

test('mentionContext is null when caret is not inside a mention', () => {
  assert.equal(mentionContext('你好 world', 8), null);
  assert.equal(mentionContext('@claude 你好', 10), null); // 光标在空格之后的普通文字里
  assert.equal(mentionContext('foo@bar.com', 7), null);    // 邮箱不算
  assert.deepEqual(mentionContext('@abc', 99), { start: 0, query: 'abc' }); // 越界光标按末尾处理
});

test('mentionContext stops at terminator punctuation', () => {
  assert.equal(mentionContext('@claude，然后', 10), null);
});

test('filterMembers matches by case-insensitive prefix then substring', () => {
  const ms = [{ handle: 'Claude' }, { handle: 'codex' }, { handle: '审阅者' }];
  assert.deepEqual(filterMembers(ms, 'c').map((m) => m.handle), ['Claude', 'codex']);
  assert.deepEqual(filterMembers(ms, 'ode').map((m) => m.handle), ['codex']);
  assert.deepEqual(filterMembers(ms, '').map((m) => m.handle), ['Claude', 'codex', '审阅者']);
  assert.deepEqual(filterMembers(ms, 'zzz'), []);
});

test('applyMention replaces the partial mention and adds a trailing space', () => {
  const ctx = { start: 3, query: 'cla' };
  assert.deepEqual(applyMention('你好 @cla', ctx, 7, 'claude'), { text: '你好 @claude ', caret: 11 });
  // 光标后还有文字时不吞掉后面的内容
  assert.deepEqual(applyMention('你好 @cla 你呢', ctx, 7, 'claude'), { text: '你好 @claude  你呢', caret: 11 });
});

// ---- Esc 关闭补全菜单 ----
// 回归:Esc 的 keydown 关掉菜单后,紧接着的 keyup 会按真实光标重算上下文,菜单立刻重开。
// 所以"已被用户关闭"要按 `@` 的位置记住,而不是靠改光标。

test('menu is open for a live mention context and closed without one', () => {
  assert.equal(mentionMenuOpen({ start: 3, query: 'c' }, null), true);
  assert.equal(mentionMenuOpen(null, null), false);
});

test('a dismissed mention stays closed across keyup recomputation of the same context', () => {
  assert.equal(mentionMenuOpen({ start: 3, query: 'cl' }, 3), false);
  assert.equal(mentionMenuOpen({ start: 3, query: 'cla' }, 3), false); // 继续输入也不重开
});

test('a different mention (new @ elsewhere) opens normally', () => {
  assert.equal(mentionMenuOpen({ start: 10, query: '' }, 3), true);
});

test('dismissal is forgotten once the caret leaves any mention', () => {
  assert.equal(nextDismissed(null, 3), null);
  assert.equal(nextDismissed({ start: 3, query: 'x' }, 3), 3);
  assert.equal(nextDismissed({ start: 3, query: 'x' }, null), null);
  // 忘掉之后,同一下标再敲 `@` 能正常打开
  assert.equal(mentionMenuOpen({ start: 3, query: '' }, nextDismissed(null, 3)), true);
});
```

`format.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDuration } from './format.ts';

test('formatDuration', () => {
  assert.equal(formatDuration(null), '');
  assert.equal(formatDuration(400), '<1 秒');
  assert.equal(formatDuration(1500), '1.5 秒');
  assert.equal(formatDuration(12_000), '12 秒');
  assert.equal(formatDuration(75_000), '1 分 15 秒');
  assert.equal(formatDuration(120_000), '2 分');
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `node --test src/handle.test.ts src/format.test.ts` → FAIL。

- [ ] **Step 3: 实现**

`handle.ts`：

```ts
// handle 校验(镜像 Rust `dozerd::group_mentions::validate_handle`,Rust 为权威)与
// `@` 补全的纯逻辑。**终止标点集合必须与 Rust `TERMINATORS` 保持一致。**

const TERMINATORS = new Set([
  ',', '.', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '<', '>', '"', "'", '`', '，',
  '。', '；', '：', '！', '？', '（', '）', '、', '「', '」',
]);
const HANDLE_MAX_CHARS = 32;

export function isHandleChar(c: string): boolean {
  return !/\s/.test(c) && c !== '@' && !TERMINATORS.has(c);
}

export function handleKey(handle: string): string {
  return handle.toLowerCase();
}

/** 返回给用户看的中文错误,合法返回 null。`takenKeys` 是本群已有 handle 的 `handleKey`。 */
export function validateHandle(handle: string, takenKeys: string[]): string | null {
  if (handle.length === 0) return '名称不能为空';
  if ([...handle].length > HANDLE_MAX_CHARS) return `名称不能超过 ${HANDLE_MAX_CHARS} 个字符`;
  const bad = [...handle].find((c) => !isHandleChar(c));
  if (bad !== undefined) return `名称不能包含 ${JSON.stringify(bad)}`;
  if (takenKeys.includes(handleKey(handle))) return `群里已有名为 ${handle} 的成员`;
  return null;
}

export function suggestHandle(agent: 'claude' | 'codex', takenKeys: string[]): string {
  if (!takenKeys.includes(agent)) return agent;
  for (let n = 2; ; n++) {
    const cand = `${agent}${n}`;
    if (!takenKeys.includes(cand)) return cand;
  }
}

// ---- @ 补全 ----

export interface MentionContext {
  /** `@` 在文本中的下标(按 UTF-16 code unit,与 textarea 的 selectionStart 一致) */
  start: number;
  query: string;
}

/** 光标紧跟在一个未完成的 `@query` 之后才返回上下文。`@` 前一个字符不能是 ASCII
 *  字母数字/`_`/`.`/`-`(排除邮箱),与 Rust `parse_mentions` 一致。 */
export function mentionContext(text: string, rawCaret: number): MentionContext | null {
  const caret = Math.max(0, Math.min(rawCaret, text.length));
  let i = caret;
  while (i > 0 && isHandleChar(text[i - 1])) i--;
  if (i === 0 || text[i - 1] !== '@') return null;
  const at = i - 1;
  const prev = at === 0 ? '' : text[at - 1];
  if (prev !== '' && /[A-Za-z0-9_.\-]/.test(prev)) return null;
  return { start: at, query: text.slice(at + 1, caret) };
}

export function filterMembers<T extends { handle: string }>(members: T[], query: string): T[] {
  const q = query.toLowerCase();
  if (q === '') return members.slice();
  const prefix = members.filter((m) => m.handle.toLowerCase().startsWith(q));
  const rest = members.filter(
    (m) => !m.handle.toLowerCase().startsWith(q) && m.handle.toLowerCase().includes(q),
  );
  return [...prefix, ...rest];
}

/** 把光标前的 `@partial` 替换成 `@handle `(带一个空格),并返回新光标位置。 */
export function applyMention(
  text: string,
  ctx: MentionContext,
  caret: number,
  handle: string,
): { text: string; caret: number } {
  const insert = `@${handle} `;
  const next = text.slice(0, ctx.start) + insert + text.slice(caret);
  return { text: next, caret: ctx.start + insert.length };
}

/** 补全菜单是否该显示:有上下文,且用户没有在**这个 `@`** 上按过 Esc。 */
export function mentionMenuOpen(ctx: MentionContext | null, dismissedStart: number | null): boolean {
  return ctx !== null && ctx.start !== dismissedStart;
}

/** 光标离开任何 `@` 提及后忘掉"已关闭"标记,之后在同一下标再敲 `@` 能正常打开。 */
export function nextDismissed(ctx: MentionContext | null, dismissedStart: number | null): number | null {
  return ctx === null ? null : dismissedStart;
}
```

`format.ts`：

```ts
/** 耗时的简短中文显示。 */
export function formatDuration(ms: number | null): string {
  if (ms === null) return '';
  if (ms < 1000) return '<1 秒';
  const s = ms / 1000;
  if (s < 10) return `${Math.round(s * 10) / 10} 秒`;
  if (s < 60) return `${Math.round(s)} 秒`;
  const m = Math.floor(s / 60);
  const rest = Math.round(s - m * 60);
  return rest === 0 ? `${m} 分` : `${m} 分 ${rest} 秒`;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `node --test src/handle.test.ts src/format.test.ts` → 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add crates/dozer-app/web/group-chat-content/src/handle.ts crates/dozer-app/web/group-chat-content/src/handle.test.ts crates/dozer-app/web/group-chat-content/src/format.ts crates/dozer-app/web/group-chat-content/src/format.test.ts
git commit -m "$(cat <<'EOF'
feat(group-chat): handle validation, @ completion and duration formatting

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: 界面组件（群切换、成员条、消息流、输入框、对话框）

**Files:**
- Create: `web/group-chat-content/src/components/{GroupBar,MemberBar,MessageItem,MessageList,Composer,Dialogs}.tsx`
- Modify: `web/group-chat-content/src/components/App.tsx`、`src/styles.css`、`src/fixtures.ts`、`src/render-smoke.tsx`
- Rebuild & commit: `crates/dozer-app/assets/group-chat-content/*`

**Interfaces:**
- Consumes: Task 6 的 `types.ts`、`ipc.ts`；Task 7 `renderMarkdown`；Task 8 的 `handle.ts`、`format.ts`。
- 组件约定：全部 props 驱动，`send(OutEvent)` 只在叶子交互处调用；**草稿与对话框开关是纯前端状态**，不进 Rust。

- [ ] **Step 1: 扩充 fixtures 与渲染冒烟（先写失败的测试）**

`src/fixtures.ts` 追加：

```ts
import type { Group, Message } from './types.ts';

const members = [
  { id: 10, agent: 'claude' as const, agent_label: 'Claude', handle: '架构师', role_prompt: '负责整体方案', color_slot: 0 },
  { id: 11, agent: 'codex' as const, agent_label: 'Codex', handle: '审阅者', role_prompt: '', color_slot: 1 },
];
const group: Group = { id: 1, topic: '评审登录方案', members };

const human = (id: number, seq: number, text: string): Message => ({
  id, seq, text, status: null, reason: null, duration_ms: null, todo_id: null,
  author: { kind: 'human', label: '你', agent_label: null, color_slot: null },
});
const agent = (
  id: number, seq: number, memberIdx: number, text: string, status: Message['status'],
  extra: Partial<Message> = {},
): Message => ({
  id, seq, text, status, reason: null, duration_ms: 2300, todo_id: null,
  author: {
    kind: 'member', label: `@${members[memberIdx].handle}`,
    agent_label: members[memberIdx].agent_label, color_slot: members[memberIdx].color_slot,
  },
  ...extra,
});

export const conversationFixture: ViewPayload = {
  project_id: 1, loaded: true, groups: [group], selected_group_id: 1, hint: null, running: false,
  messages: [
    human(1, 1, '@架构师 @审阅者 登录要不要加验证码？'),
    agent(2, 2, 0, '建议加。**理由**:\n- 防撞库\n- 成本低', 'done'),
    agent(3, 3, 1, '同意,但要 `限流` 兜底。', 'done', { todo_id: 9 }),
  ],
};

export const runningFixture: ViewPayload = {
  ...conversationFixture, running: true,
  messages: [
    human(1, 1, '@架构师 @审阅者 看下'),
    agent(2, 2, 0, '', 'running', { duration_ms: null }),
    agent(3, 3, 1, '', 'queued', { duration_ms: null }),
  ],
};

export const failedFixture: ViewPayload = {
  ...conversationFixture,
  messages: [
    human(1, 1, '@架构师 看下'),
    agent(2, 2, 0, '', 'failed', { reason: '发言超时' }),
    agent(3, 3, 1, '', 'cancelled'),
  ],
};

export const maliciousFixture: ViewPayload = {
  ...conversationFixture,
  messages: [
    human(1, 1, '<img src=x onerror=alert(1)>'),
    agent(2, 2, 0, '<script>alert(1)</script> [x](javascript:alert(1))', 'done'),
  ],
};

export const hintFixture: ViewPayload = { ...conversationFixture, hint: '未找到成员 @nobody' };

export const noMembersFixture: ViewPayload = {
  ...conversationFixture, messages: [],
  groups: [{ id: 1, topic: '空群', members: [] }],
};

export const noSelectionFixture: ViewPayload = {
  ...conversationFixture, selected_group_id: null, messages: [],
};
```

`src/render-smoke.tsx` 追加：

```tsx
import {
  conversationFixture, runningFixture, failedFixture, maliciousFixture, hintFixture,
  noMembersFixture, noSelectionFixture,
} from './fixtures.ts';

test('group bar shows topics and marks the selected one', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /评审登录方案/);
  assert.match(out, /is-selected/);
  assert.match(out, /新建群聊/);
});

test('member bar lists handles with agent labels and an add button', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /@架构师/);
  assert.match(out, /Claude/);
  assert.match(out, /@审阅者/);
  assert.match(out, /Codex/);
  assert.match(out, /添加成员/);
});

test('messages render authors, markdown and durations', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /你/);
  assert.match(out, /<strong>理由<\/strong>/);
  assert.match(out, /<li>防撞库<\/li>/);
  assert.match(out, /<code>限流<\/code>/);
  assert.match(out, /2\.3 秒/);
});

test('todo badge appears only on messages already pushed to a todo; push button on the others', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /已转待办/);
  assert.match(out, /转为待办/);
});

test('running and queued messages show status labels and a stop control', () => {
  const out = render(<App payload={runningFixture} />);
  assert.match(out, /正在发言/);
  assert.match(out, /排队中/);
  assert.match(out, /停止本轮/);
});

test('failed message shows reason and retry; cancelled shows retry too', () => {
  const out = render(<App payload={failedFixture} />);
  assert.match(out, /发言超时/);
  assert.match(out, /已取消/);
  assert.equal((out.match(/重试/g) ?? []).length, 2);
});

// Review Focus 5
test('hostile content is escaped in the rendered page', () => {
  const out = render(<App payload={maliciousFixture} />);
  assert.doesNotMatch(out, /<script/i);
  assert.doesNotMatch(out, /<img/i);
  assert.doesNotMatch(out, /href="javascript/i);
  assert.match(out, /&lt;script&gt;/);
});

test('hint is shown near the composer', () => {
  assert.match(render(<App payload={hintFixture} />), /未找到成员 @nobody/);
});

test('empty group shows a hint to add members and disables nothing silently', () => {
  const out = render(<App payload={noMembersFixture} />);
  assert.match(out, /还没有成员/);
});

test('no selected group but groups exist prompts to pick one', () => {
  assert.match(render(<App payload={noSelectionFixture} />), /选择一个群聊/);
});

test('composer placeholder explains @ and keys', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /@ 点名成员发言/);
});
```

Run: `npm test` → 新用例 FAIL。

- [ ] **Step 2: 实现组件**

`components/App.tsx`：

```tsx
import { useRef, useState } from 'preact/hooks';
import type { ViewPayload } from '../types.ts';
import { GroupBar } from './GroupBar.tsx';
import { MemberBar } from './MemberBar.tsx';
import { MessageList } from './MessageList.tsx';
import { Composer } from './Composer.tsx';
import { Dialogs, type DialogState } from './Dialogs.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const [dialog, setDialog] = useState<DialogState>(null);
  // 草稿按群保存,切群不丢(纯前端状态,不进 Rust)。
  const drafts = useRef<Record<number, string>>({});

  if (!payload.loaded) return <div class="gc-empty">加载中…</div>;

  const group = payload.groups.find((g) => g.id === payload.selected_group_id) ?? null;

  return (
    <div class="gc-root">
      <GroupBar
        groups={payload.groups}
        selectedId={payload.selected_group_id}
        onNew={() => setDialog({ kind: 'new_group' })}
        onDelete={(id) => setDialog({ kind: 'delete_group', groupId: id })}
      />
      {payload.groups.length === 0 ? (
        <div class="gc-empty">
          <p>还没有群聊</p>
          <p class="gc-dim">邀请 Claude、Codex 进群,用 @ 点名让它们依次发言讨论。</p>
          <button class="gc-primary" onClick={() => setDialog({ kind: 'new_group' })}>新建群聊</button>
        </div>
      ) : group === null ? (
        <div class="gc-empty">选择一个群聊</div>
      ) : (
        <>
          <MemberBar
            members={group.members}
            onAdd={() => setDialog({ kind: 'member', groupId: group.id, editing: null })}
            onEdit={(m) => setDialog({ kind: 'member', groupId: group.id, editing: m })}
          />
          <MessageList
            messages={payload.messages}
            groupId={group.id}
            hasMembers={group.members.length > 0}
            onPushTodo={(m) => setDialog({ kind: 'push_todo', message: m })}
          />
          <Composer
            groupId={group.id}
            members={group.members}
            running={payload.running}
            hint={payload.hint}
            draftOf={(id) => drafts.current[id] ?? ''}
            onDraft={(id, text) => { drafts.current[id] = text; }}
          />
        </>
      )}
      <Dialogs
        state={dialog}
        group={group}
        onClose={() => setDialog(null)}
      />
    </div>
  );
}
```

`components/GroupBar.tsx`：

```tsx
import type { Group } from '../types.ts';
import { send } from '../ipc.ts';

export function GroupBar({
  groups, selectedId, onNew, onDelete,
}: {
  groups: Group[];
  selectedId: number | null;
  onNew: () => void;
  onDelete: (id: number) => void;
}) {
  return (
    <div class="gc-groupbar">
      <div class="gc-tabs">
        {groups.map((g) => (
          <div class={`gc-tab${g.id === selectedId ? ' is-selected' : ''}`} key={g.id}>
            <button
              class="gc-tab-main"
              title={g.topic}
              onClick={() => send({ kind: 'select_group', group_id: g.id })}
            >
              {g.topic}
            </button>
            {g.id === selectedId && (
              <button class="gc-icon" title="删除群聊" onClick={() => onDelete(g.id)}>×</button>
            )}
          </div>
        ))}
      </div>
      <button class="gc-icon gc-new" title="新建群聊" aria-label="新建群聊" onClick={onNew}>＋</button>
    </div>
  );
}
```

`components/MemberBar.tsx`：

```tsx
import type { Member } from '../types.ts';

export function MemberBar({
  members, onAdd, onEdit,
}: {
  members: Member[];
  onAdd: () => void;
  onEdit: (m: Member) => void;
}) {
  return (
    <div class="gc-memberbar">
      {members.map((m) => (
        <button
          class={`gc-chip gc-c${m.color_slot}`}
          key={m.id}
          title={m.role_prompt || '没有角色设定'}
          onClick={() => onEdit(m)}
        >
          @{m.handle}<span class="gc-chip-agent">{m.agent_label}</span>
        </button>
      ))}
      <button class="gc-chip gc-chip-add" onClick={onAdd}>＋ 添加成员</button>
    </div>
  );
}
```

`components/MessageItem.tsx`：

```tsx
import type { Message } from '../types.ts';
import { send } from '../ipc.ts';
import { cancelEvent } from '../events.ts';
import { renderMarkdown } from '../markdown.ts';
import { formatDuration } from '../format.ts';

const STATUS_LABEL: Record<string, string> = {
  queued: '排队中',
  running: '正在发言',
  failed: '发言失败',
  cancelled: '已取消',
};

export function MessageItem({
  m, groupId, onPushTodo,
}: { m: Message; groupId: number; onPushTodo: (m: Message) => void }) {
  const isHuman = m.author.kind === 'human';
  const pending = m.status === 'queued' || m.status === 'running';
  const canRetry = m.status === 'failed' || m.status === 'cancelled';
  const canPush = !pending && m.todo_id === null && m.text.trim() !== '';
  const colorClass = isHuman ? 'is-human' : m.author.color_slot !== null ? `gc-c${m.author.color_slot}` : 'gc-c-none';
  const dur = formatDuration(m.duration_ms);

  return (
    <div class={`gc-msg ${colorClass}`} data-seq={m.seq}>
      <div class="gc-msg-head">
        <span class="gc-author">{m.author.label}</span>
        {m.author.agent_label && <span class="gc-agent">{m.author.agent_label}</span>}
        {m.status && m.status !== 'done' && (
          <span class={`gc-status is-${m.status}`}>
            {m.status === 'running' && <span class="gc-dots" aria-hidden="true"><i /><i /><i /></span>}
            {STATUS_LABEL[m.status]}
          </span>
        )}
        {dur && m.status === 'done' && <span class="gc-dim">{dur}</span>}
      </div>
      {m.status === 'failed' && m.reason && <div class="gc-reason">{m.reason}</div>}
      {m.text !== '' && (
        <div class="gc-body" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text) }} />
      )}
      <div class="gc-msg-actions">
        {m.status === 'running' && (
          <button class="gc-link" onClick={() => send(cancelEvent(groupId, 'turn'))}>
            停止
          </button>
        )}
        {canRetry && (
          <button class="gc-link" onClick={() => send({ kind: 'retry', message_id: m.id })}>重试</button>
        )}
        {canPush && (
          <button class="gc-link" onClick={() => onPushTodo(m)}>转为待办</button>
        )}
        {m.todo_id !== null && (
          <button class="gc-link gc-badge" onClick={() => send({ kind: 'open_todo', todo_id: m.todo_id! })}>
            已转待办 ↗
          </button>
        )}
      </div>
    </div>
  );
}
```


`components/MessageList.tsx`：

```tsx
import { useEffect, useRef } from 'preact/hooks';
import type { Message } from '../types.ts';
import { MessageItem } from './MessageItem.tsx';

const NEAR_BOTTOM_PX = 80;

export function MessageList({
  messages, groupId, hasMembers, onPushTodo,
}: {
  messages: Message[];
  groupId: number;
  hasMembers: boolean;
  onPushTodo: (m: Message) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  // 新消息到来:仅当用户本来就在底部附近才跟随滚动,不打断回看。
  useEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages]);

  return (
    <div
      class="gc-messages"
      ref={box}
      onScroll={() => {
        const el = box.current;
        if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
      }}
    >
      {messages.length === 0 ? (
        <div class="gc-empty-inline">
          {hasMembers ? '发一条消息,用 @ 点名成员发言。' : '这个群还没有成员,先点上方「添加成员」。'}
        </div>
      ) : (
        messages.map((m) => (
          <MessageItem key={m.id} m={m} groupId={groupId} onPushTodo={onPushTodo} />
        ))
      )}
    </div>
  );
}
```

（`hasMembers=false` 的文案"这个群还没有成员"与渲染冒烟里的 `/还没有成员/` 匹配。）

`components/Composer.tsx`：

```tsx
import { useEffect, useRef, useState } from 'preact/hooks';
import type { Member } from '../types.ts';
import { send } from '../ipc.ts';
import { cancelEvent } from '../events.ts';
import {
  applyMention, filterMembers, mentionContext, mentionMenuOpen, nextDismissed,
} from '../handle.ts';

export function Composer({
  groupId, members, running, hint, draftOf, onDraft,
}: {
  groupId: number;
  members: Member[];
  running: boolean;
  hint: string | null;
  draftOf: (groupId: number) => string;
  onDraft: (groupId: number, text: string) => void;
}) {
  const [text, setText] = useState(draftOf(groupId));
  const [caret, setCaret] = useState(0);
  const [pick, setPick] = useState(0);
  // 用户在哪个 `@`(下标)上按过 Esc;见 `mentionMenuOpen`。
  const [dismissed, setDismissed] = useState<number | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);

  // 切群:载入该群草稿。
  useEffect(() => { setText(draftOf(groupId)); setPick(0); }, [groupId]);

  const ctx = mentionContext(text, caret);
  const ctxStart = ctx ? ctx.start : -1;
  // 光标离开提及后忘掉"已关闭"标记。
  useEffect(() => {
    setDismissed((d) => nextDismissed(ctx, d));
  }, [ctxStart]);
  const options = ctx && mentionMenuOpen(ctx, dismissed) ? filterMembers(members, ctx.query) : [];
  const menuOpen = options.length > 0;

  const update = (next: string, c: number) => {
    setText(next);
    setCaret(c);
    onDraft(groupId, next);
    setPick(0);
  };

  const choose = (handle: string) => {
    if (!ctx) return;
    const r = applyMention(text, ctx, caret, handle);
    update(r.text, r.caret);
    queueMicrotask(() => area.current?.setSelectionRange(r.caret, r.caret));
  };

  const submit = () => {
    const t = text.trim();
    if (t === '') return;
    send({ kind: 'post', group_id: groupId, text: t });
    update('', 0);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // Review Focus 2:中文输入法组词时 Enter 是确认候选词,不是发送。
    if (e.isComposing || e.keyCode === 229) return;
    if (menuOpen) {
      if (e.key === 'ArrowDown') { e.preventDefault(); setPick((p) => (p + 1) % options.length); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); setPick((p) => (p - 1 + options.length) % options.length); return; }
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); choose(options[pick].handle); return; }
      if (e.key === 'Escape') { e.preventDefault(); if (ctx) setDismissed(ctx.start); return; }
    }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit(); }
  };

  return (
    <div class="gc-composer">
      {hint && <div class="gc-hint">{hint}</div>}
      {menuOpen && (
        <div class="gc-mention-menu" role="listbox">
          {options.map((m, i) => (
            <button
              class={`gc-mention-item gc-c${m.color_slot}${i === pick ? ' is-picked' : ''}`}
              key={m.id}
              onMouseDown={(e) => { e.preventDefault(); choose(m.handle); }}
            >
              @{m.handle}<span class="gc-chip-agent">{m.agent_label}</span>
            </button>
          ))}
        </div>
      )}
      <div class="gc-composer-row">
        <textarea
          ref={area}
          class="gc-input"
          rows={2}
          value={text}
          placeholder="输入消息,@ 点名成员发言;Enter 发送,Shift+Enter 换行"
          onInput={(e) => {
            const el = e.currentTarget;
            update(el.value, el.selectionStart ?? el.value.length);
          }}
          onClick={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
          onKeyUp={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
          onKeyDown={onKeyDown}
        />
        <div class="gc-composer-actions">
          {running && (
            <button class="gc-stop" onClick={() => send(cancelEvent(groupId, 'round'))}>
              停止本轮
            </button>
          )}
          <button class="gc-send" disabled={text.trim() === ''} onClick={submit}>发送</button>
        </div>
      </div>
    </div>
  );
}
```

> 渲染冒烟里 `placeholder` 含"@ 点名成员发言"，对应测试断言 `/@ 点名成员发言/`。

`components/Dialogs.tsx`（新建群、添加/编辑成员、删除群确认、转待办；全部 DOM 对话框，Esc 关闭，提交后立即关闭，失败由 Rust 走 Toast）：

```tsx
import type { ComponentChildren } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { AgentKey, Group, Member, Message } from '../types.ts';
import { send } from '../ipc.ts';
import { handleKey, suggestHandle, validateHandle } from '../handle.ts';

export type DialogState =
  | null
  | { kind: 'new_group' }
  | { kind: 'delete_group'; groupId: number }
  | { kind: 'member'; groupId: number; editing: Member | null }
  | { kind: 'push_todo'; message: Message };

function Shell({
  title, onClose, children,
}: { title: string; onClose: () => void; children: ComponentChildren }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div class="gc-modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div class="gc-modal" role="dialog" aria-label={title}>
        <h4>{title}</h4>
        {children}
      </div>
    </div>
  );
}

function NewGroup({ onClose }: { onClose: () => void }) {
  const [topic, setTopic] = useState('');
  const ok = topic.trim() !== '';
  const submit = () => { if (!ok) return; send({ kind: 'create_group', topic: topic.trim() }); onClose(); };
  return (
    <Shell title="新建群聊" onClose={onClose}>
      <label class="gc-field">
        <span>讨论主题</span>
        <input
          class="gc-text" value={topic} autoFocus placeholder="例如:评审登录方案"
          onInput={(e) => setTopic(e.currentTarget.value)}
          onKeyDown={(e) => { if (e.isComposing || e.keyCode === 229) return; if (e.key === 'Enter') submit(); }}
        />
      </label>
      <div class="gc-modal-actions">
        <button onClick={onClose}>取消</button>
        <button class="gc-primary" disabled={!ok} onClick={submit}>创建</button>
      </div>
    </Shell>
  );
}

function DeleteGroup({ groupId, onClose }: { groupId: number; onClose: () => void }) {
  return (
    <Shell title="删除群聊" onClose={onClose}>
      <p>删除后群聊记录无法恢复,进行中的发言会被停止。</p>
      <div class="gc-modal-actions">
        <button onClick={onClose}>取消</button>
        <button class="gc-danger" onClick={() => { send({ kind: 'delete_group', group_id: groupId }); onClose(); }}>
          删除
        </button>
      </div>
    </Shell>
  );
}

function MemberForm({
  group, groupId, editing, onClose,
}: { group: Group | null; groupId: number; editing: Member | null; onClose: () => void }) {
  const others = (group?.members ?? []).filter((m) => m.id !== editing?.id).map((m) => handleKey(m.handle));
  const [agent, setAgent] = useState<AgentKey>(editing?.agent ?? 'claude');
  const [handle, setHandle] = useState(editing?.handle ?? suggestHandle('claude', others));
  const [role, setRole] = useState(editing?.role_prompt ?? '');
  const err = validateHandle(handle.trim(), others);
  const submit = () => {
    if (err) return;
    if (editing) {
      send({ kind: 'update_member', member_id: editing.id, handle: handle.trim(), role_prompt: role.trim() });
    } else {
      send({ kind: 'add_member', group_id: groupId, agent, handle: handle.trim(), role_prompt: role.trim() });
    }
    onClose();
  };
  return (
    <Shell title={editing ? `编辑 @${editing.handle}` : '添加成员'} onClose={onClose}>
      {!editing && (
        <label class="gc-field">
          <span>Agent</span>
          <select
            class="gc-text" value={agent}
            onChange={(e) => {
              const next = e.currentTarget.value as AgentKey;
              setAgent(next);
              // 名称还是上一个 agent 的默认建议时,跟着换。
              if (handle === suggestHandle(agent, others)) setHandle(suggestHandle(next, others));
            }}
          >
            <option value="claude">Claude</option>
            <option value="codex">Codex</option>
          </select>
        </label>
      )}
      <label class="gc-field">
        <span>群内名称(用 @ 点名)</span>
        <input class="gc-text" value={handle} onInput={(e) => setHandle(e.currentTarget.value)} />
        {err && <em class="gc-err">{err}</em>}
      </label>
      <label class="gc-field">
        <span>角色设定(可选)</span>
        <textarea class="gc-text" rows={4} value={role} placeholder="例如:你负责挑方案的漏洞,只提问题不提方案。"
          onInput={(e) => setRole(e.currentTarget.value)} />
      </label>
      <div class="gc-modal-actions">
        {editing && (
          <button class="gc-danger gc-left" onClick={() => { send({ kind: 'remove_member', member_id: editing.id }); onClose(); }}>
            移出群聊
          </button>
        )}
        <button onClick={onClose}>取消</button>
        <button class="gc-primary" disabled={err !== null} onClick={submit}>{editing ? '保存' : '添加'}</button>
      </div>
    </Shell>
  );
}

function PushTodo({ message, onClose }: { message: Message; onClose: () => void }) {
  const [text, setText] = useState(message.text.trim());
  const ok = text.trim() !== '';
  return (
    <Shell title="转为待办" onClose={onClose}>
      <p class="gc-dim">会在 Todo 面板新建一条待办;由哪个 agent 来做,在 Todo 里指派。</p>
      <textarea class="gc-text" rows={6} value={text} onInput={(e) => setText(e.currentTarget.value)} />
      <div class="gc-modal-actions">
        <button onClick={onClose}>取消</button>
        <button class="gc-primary" disabled={!ok}
          onClick={() => { send({ kind: 'push_todo', message_id: message.id, text: text.trim() }); onClose(); }}>
          创建待办
        </button>
      </div>
    </Shell>
  );
}

export function Dialogs({
  state, group, onClose,
}: { state: DialogState; group: Group | null; onClose: () => void }) {
  if (!state) return null;
  switch (state.kind) {
    case 'new_group': return <NewGroup onClose={onClose} />;
    case 'delete_group': return <DeleteGroup groupId={state.groupId} onClose={onClose} />;
    case 'member': return <MemberForm group={group} groupId={state.groupId} editing={state.editing} onClose={onClose} />;
    case 'push_todo': return <PushTodo message={state.message} onClose={onClose} />;
  }
}
```

`src/styles.css` 追加（要点：群切换条横向滚动、成员色块四色槽、human 消息金色左边线、agent 用对应色槽；等宽字体仅 `pre code`；对话框遮罩；运行中三点动画）：

```css
.gc-root { display: flex; flex-direction: column; height: 100%; position: relative; }
.gc-empty, .gc-empty-inline { color: var(--dim); text-align: center; padding: 32px 20px; }
.gc-empty p { margin: 6px 0; }
.gc-dim { color: var(--dim); font-size: 12px; }

button { appearance: none; font: inherit; cursor: pointer; color: inherit; }
.gc-primary { background: var(--gold); color: var(--panel); border: 0; border-radius: 6px; padding: 5px 14px; font-weight: 600; }
.gc-primary:disabled { opacity: .45; cursor: default; }
.gc-danger { background: transparent; color: var(--red); border: 1px solid var(--red); border-radius: 6px; padding: 5px 12px; }
.gc-link { background: transparent; border: 0; color: var(--dim); padding: 0 4px; font-size: 12px; }
.gc-link:hover { color: var(--gold); }
.gc-badge { color: var(--green); }
.gc-icon { background: transparent; border: 0; color: var(--dim); padding: 2px 6px; border-radius: 4px; }
.gc-icon:hover { color: var(--gold); background: var(--card); }

/* 群切换条 */
.gc-groupbar { display: flex; align-items: center; gap: 4px; padding: 8px 12px 0; border-bottom: 1px solid var(--border); }
.gc-tabs { display: flex; gap: 4px; overflow-x: auto; flex: 1; min-width: 0; scrollbar-width: thin; }
.gc-tab { display: inline-flex; align-items: center; flex: none; max-width: 200px; border: 1px solid transparent;
  border-bottom: 0; border-radius: 8px 8px 0 0; padding: 0 2px 0 8px; }
.gc-tab.is-selected { background: var(--panel); border-color: var(--border); }
.gc-tab.is-selected .gc-tab-main { color: var(--gold); }
.gc-tab-main { background: transparent; border: 0; padding: 6px 4px; color: var(--body);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 150px; }
.gc-new { flex: none; }

/* 成员条 */
.gc-memberbar { display: flex; flex-wrap: wrap; gap: 6px; padding: 8px 12px; border-bottom: 1px solid var(--border); }
.gc-chip { display: inline-flex; align-items: center; gap: 6px; background: var(--card); border: 1px solid var(--border);
  border-radius: 999px; padding: 2px 10px; }
.gc-chip:hover { background: var(--card-hover); }
.gc-chip-agent { color: var(--dim); font-size: 11px; }
.gc-chip-add { color: var(--dim); border-style: dashed; }
.gc-c0 { --accent: var(--cyan); }
.gc-c1 { --accent: var(--green); }
.gc-c2 { --accent: var(--purple); }
.gc-c3 { --accent: var(--orange); }
.gc-c-none { --accent: var(--dim); }
.gc-chip[class*="gc-c"] { border-color: var(--accent); color: var(--accent); }
.gc-chip-add { color: var(--dim); border-color: var(--border); }

/* 消息流 */
.gc-messages { flex: 1; overflow-y: auto; padding: 12px 16px; }
.gc-msg { border-left: 3px solid var(--accent, var(--border)); padding: 4px 0 4px 12px; margin: 0 0 14px; }
.gc-msg.is-human { --accent: var(--gold); }
.gc-msg-head { display: flex; align-items: center; gap: 8px; margin-bottom: 2px; }
.gc-author { font-weight: 600; color: var(--accent, var(--cream)); }
.gc-agent { color: var(--dim); font-size: 11px; }
.gc-status { font-size: 11px; padding: 0 8px; border-radius: 999px; border: 1px solid var(--border); color: var(--dim); }
.gc-status.is-running { color: var(--cyan); border-color: var(--cyan); }
.gc-status.is-failed { color: var(--red); border-color: var(--red); }
.gc-reason { color: var(--red); font-size: 12px; margin: 2px 0; }
.gc-body { color: var(--cream); overflow-wrap: anywhere; }
.gc-body p { margin: 4px 0; }
.gc-body ul, .gc-body ol { margin: 4px 0; padding-left: 22px; }
.gc-body h3, .gc-body h4, .gc-body h5 { margin: 8px 0 4px; color: var(--cream); }
.gc-body code { background: var(--card); border-radius: 4px; padding: 0 4px; font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12px; }
.gc-body pre { background: var(--card); border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px; overflow-x: auto; }
.gc-body pre code { background: transparent; padding: 0; }
.gc-msg-actions { display: flex; gap: 4px; margin-top: 2px; min-height: 18px; }
.gc-dots i { display: inline-block; width: 4px; height: 4px; margin: 0 1px; border-radius: 50%; background: currentColor;
  animation: gc-pulse 1.2s infinite ease-in-out; }
.gc-dots i:nth-child(2) { animation-delay: .2s; }
.gc-dots i:nth-child(3) { animation-delay: .4s; }
@keyframes gc-pulse { 0%, 80%, 100% { opacity: .25; } 40% { opacity: 1; } }

/* 输入框 */
.gc-composer { position: relative; border-top: 1px solid var(--border); padding: 8px 12px 12px; }
.gc-hint { color: var(--orange); font-size: 12px; margin-bottom: 4px; }
.gc-composer-row { display: flex; gap: 8px; align-items: flex-end; }
.gc-input { flex: 1; resize: none; background: var(--bg); color: var(--cream); border: 1px solid var(--border);
  border-radius: 8px; padding: 6px 8px; font: inherit; outline: 0; }
.gc-input:focus { border-color: var(--gold); }
.gc-composer-actions { display: flex; flex-direction: column; gap: 6px; }
.gc-send { background: var(--gold); color: var(--panel); border: 0; border-radius: 6px; padding: 5px 14px; font-weight: 600; }
.gc-send:disabled { opacity: .45; cursor: default; }
.gc-stop { background: transparent; color: var(--red); border: 1px solid var(--red); border-radius: 6px; padding: 4px 10px; }
.gc-mention-menu { position: absolute; left: 12px; bottom: calc(100% - 4px); background: var(--card); border: 1px solid var(--border);
  border-radius: 8px; box-shadow: var(--shadow); padding: 4px; display: flex; flex-direction: column; min-width: 160px; z-index: 5; }
.gc-mention-item { display: flex; justify-content: space-between; gap: 12px; background: transparent; border: 0;
  border-radius: 4px; padding: 4px 8px; color: var(--accent); text-align: left; }
.gc-mention-item.is-picked, .gc-mention-item:hover { background: var(--card-hover); }

/* 对话框 */
.gc-modal-backdrop { position: absolute; inset: 0; background: rgba(0, 0, 0, .45); display: flex; align-items: center; justify-content: center; z-index: 10; }
.gc-modal { width: min(420px, 90%); background: var(--panel); border: 1px solid var(--gold); border-radius: 10px; padding: 14px 16px; box-shadow: var(--shadow); }
.gc-modal h4 { margin: 0 0 10px; color: var(--cream); }
.gc-field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; font-size: 12px; color: var(--dim); }
.gc-text { background: var(--bg); color: var(--cream); border: 1px solid var(--border); border-radius: 6px; padding: 5px 8px; font: inherit; outline: 0; resize: vertical; }
.gc-text:focus { border-color: var(--gold); }
.gc-err { color: var(--red); font-style: normal; }
.gc-modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
.gc-modal-actions button { border-radius: 6px; padding: 5px 12px; background: var(--card); border: 1px solid var(--border); }
.gc-modal-actions .gc-primary { background: var(--gold); border-color: var(--gold); }
.gc-modal-actions .gc-danger { background: transparent; border-color: var(--red); }
.gc-left { margin-right: auto; }
```

- [ ] **Step 3: 事件构造纯函数（钉住"停止发的是群 id，不是消息 id"）**

渲染冒烟发现不了"调用处把消息 id 当群 id 传"这类错。把 `cancel` 事件的构造抽成纯函数，单测钉住字段含义；`MessageItem`/`Composer` 都只经它发 `cancel`，且 `MessageItem` 的 `groupId` 是**必填 prop**，缺了 `tsc --noEmit` 会报错（Step 4 的 `typecheck` 是第二道闸）。

`src/events.ts`：

```ts
import type { CancelScope, OutEvent } from './types.ts';

export const cancelEvent = (groupId: number, scope: CancelScope): OutEvent => ({
  kind: 'cancel', group_id: groupId, scope,
});
```

`src/events.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { cancelEvent } from './events.ts';

test('cancel event carries the group id (not a message id) and the scope', () => {
  assert.deepEqual(cancelEvent(7, 'turn'), { kind: 'cancel', group_id: 7, scope: 'turn' });
  assert.deepEqual(cancelEvent(7, 'round'), { kind: 'cancel', group_id: 7, scope: 'round' });
});
```


- [ ] **Step 4: 跑全部前端检查并构建**

```bash
cd crates/dozer-app/web/group-chat-content
npm run typecheck
npm test
npm run build
```

Expected：类型检查零错误；`node --test` 与渲染冒烟全 PASS。

- [ ] **Step 5: 手动冒烟（本机有显示环境时）**

`cargo run -p dozer-app`，打开项目 → 群聊面板：
1. 新建群 → 群切换条出现并选中；
2. 添加成员 Claude、Codex → 成员条出现两个色块；
3. 输入 `@` 出现补全菜单，↑↓ Enter 选择；用**中文输入法**输入"你好"按 Enter 确认候选词时**不会发送**；
4. 发送 `@claude 简单回答 1+1`，消息流出现"正在发言/排队中"，完成后出现回复；
5. 对 agent 回复点"转为待办" → 对话框 → 创建 → 消息出现"已转待办 ↗"，点击跳到 Todo 面板；
6. 点"×"删除群 → 确认。

- [ ] **Step 6: 提交（含构建产物）**

```bash
git branch --show-current
git add crates/dozer-app/web/group-chat-content crates/dozer-app/assets/group-chat-content
git commit -m "$(cat <<'EOF'
feat(group-chat): group bar, member bar, message list, composer and dialogs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: 文档、全量验证、同步 spec、人工验收

**Files:**
- Modify: `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`
- Modify: `CLAUDE.md`（只在确有出入时：见 Step 2）

- [ ] **Step 1: 全量验证**

```bash
cargo fmt
cargo clippy --workspace --all-targets 2>&1 | tail -20
scripts/check-log-scope.sh
cargo test --workspace 2>&1 | tail -30
(cd crates/dozer-app/web/group-chat-content && npm run typecheck && npm test)
```

Expected：fmt 无 diff；clippy 无新增；门禁 `ok`；Rust 与前端测试全绿。已知与本次无关的既有失败（如 dozerd `summary_pipeline`）与基线对比确认不是本分支引入，如实说明，不要顺手修。

- [ ] **Step 2: 检查 CLAUDE.md 是否需要补充**

CLAUDE.md 目前写"工作区 10/11 个面板"之类的数量描述的地方（`grep -n "11 个\|10 个" CLAUDE.md`）若存在就改成 12；"关键裁决"里加一条（只加**不能从代码推出**的约束）：

```markdown
- **群聊面板（`PanelKind::GroupChat`）**（见 `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`）：群聊只做讨论，**不分配任务**——"转为待办"只在 Todo 里新建一条，指派归 Todo；agent 发言走无头一次性调用且必须在执行层只读（Claude 工具白/黑名单、Codex `--sandbox read-only`），**严禁**给群聊调用加 `--dangerously-skip-permissions`/`-y`；`@` 只由 human 触发，agent 回复里的 `@` 不解析；群聊 webview 里所有 agent 输出必须经转义渲染器，不得 `innerHTML` 原文。
```

- [ ] **Step 3: 同步 spec**

对 `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md` 做：
1. 第 9 节：对话框改为"webview 内 DOM 对话框（新建群、添加/编辑成员、删除确认、转为待办）"，删掉"走标准对话框独立窗口机制"；补"面板为单列整宽 webview，无原生列表列"；Markdown 一句"最小安全子集，原始 HTML 与链接以纯文本显示"。
2. 第 11 节：补"提交失败对话框已关闭、错误走 Toast；成员名称在前端先做即时校验（镜像后端规则）"。
3. 第 4.1 节 `dozer-app` 一行补"轮询由 `about_to_wait` 定时唤醒驱动，仅在面板可见且有发言进行中时运行"。
4. 第 13 节未决项里把"字体"补一条："群聊代码块目前用系统等宽；是否改用 JetBrains Mono 待用户决定（需扩 host CSP 的 `font-src` 并拷字体）"。

- [ ] **Step 4: 人工验收清单（在汇报里逐项写结果）**

- 右栏图标栏：群聊图标在「代理」下方，tooltip "群聊"；拖拽换栏、收起/展开、放大态都正常，放大态下 webview 不溢出描边盒。
- **升级路径**：把仓库里一份旧版 `layout.json`（11 个面板）放进状态目录再启动，图标栏保持原自定义顺序并多出群聊图标，没有被重置。
- 切换项目页签：各项目的群聊独立；上一个项目的发言结果不会出现在当前项目。
- dozerd 未启动：面板不崩，顶栏徽标提示；恢复后可用。
- 发言中关闭 Dozer 再打开：遗留的排队/运行中消息显示"发言失败（dozerd 重启…）"并可重试。
- 深色/浅色主题各看一遍；窗口缩到最小宽度时群切换条横向滚动、成员条换行、输入框不溢出。
- 对话框打开时 webview 不被原生遮罩遮住（`app_modal_open` 与对话框无关，这里只是确认 DOM 对话框自身遮罩可点击关闭）。

- [ ] **Step 5: 提交并汇报**

```bash
git branch --show-current
git add docs/superpowers/specs/2026-10-02-group-chat-panel-design.md CLAUDE.md
git commit -m "$(cat <<'EOF'
docs(group-chat): sync spec and CLAUDE.md with GUI plan rulings

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
EOF
)"
```

向用户汇报：完成了什么、跑过哪些验证（含基线对比）、人工验收清单的结果、两个需要用户拍板的点（Ruling 4 的字体；byteui v0.4.1 的发版是否已完成）。

---

## Self-Review（对照 spec 逐条）

| spec 条目 | 落点 |
|---|---|
| §1 面板：新 `PanelKind::GroupChat`、`square-sparkles`、右栏排 Agent 之后、webview | Task 0（图标）、Task 1（枚举/栏/迁移）、Task 5（webview 宿主） |
| §1 `@` 触发、多个串行 | 后端已覆盖；GUI 在 Task 9 Composer 的 `@` 补全 + `post` 事件 |
| §1 转待办单独操作、不分配 | Task 3 `Command::PushTodo`（无 assignee）、Task 9 `PushTodo` 对话框文案"在 Todo 里指派" |
| §4.1 `dozer-app` 只做展示与输入、不持有 agent 进程 | Task 2–5：全部走 `Client`；失败经 `outbox` → Toast |
| §5 数据模型在 GUI 的映射 | Task 3 payload（作者标签、颜色槽、状态映射） |
| §9 界面：群切换条、成员条、消息流、`@` 补全输入框、状态与重试 | Task 9 |
| §9 视觉：金色=human、agent 青/绿等、系统字体、Markdown、loading 动画 | Task 6 `styles.css`、Task 7、Task 9 CSS（`gc-dots`） |
| §9 对话框 | Ruling 1：DOM 对话框；Task 9 `Dialogs.tsx` |
| §10 转为待办：预填可编辑、不指派、徽标、跳转 Todo | Task 9 `PushTodo` + `MessageItem` 徽标 + `OpenTodo`→`panel_select(Todo)` |
| §11 错误：dozerd 不可用走徽标、一次性事件走 Toast、无成员引导、无 `@` 轻提示、日志来源 `group_chat` | Task 2（outbox、hint）、Task 1（日志名表）、Task 9（空态文案） |
| §12 测试 | 每个 Task TDD；Rust 纯函数 + 前端 `node --test` + 渲染冒烟；`HostBinding` 式归属校验在本面板不适用（固定单槽、按 webview id 路由，同 Todo/Code Health） |
| §13 未决 | 字体（Ruling 4）；轮询间隔 `POLL_INTERVAL = 500ms` 带"待实测"性质，已写在常量文档里 |

**占位符扫描**：无 TBD/TODO。三处容易被遗漏的联动（`Command::PushTodo` 带 `group_id`、`loaded`/`fresh` 拆分与 `mark_stale`、`MessageItem` 的必填 `groupId`）都已直接写进对应 Task 的代码与测试，没有"稍后再改"。

**类型一致性**：`Message`/`Effect`/`Command`/`GroupChatViewPayload` 在 Task 2–3 定义，Task 4 的 `spawn_command`、Task 5 的 `group_chat_command`/`run_group_chat_effects`、Task 5 的 `current_view_payload` 调用全部沿用同名；前端 `types.ts` 的字段名与 Task 3 的 `serde` 输出逐一对应（`selected_group_id`、`agent_label`、`color_slot`、`reason`、`duration_ms`、`todo_id`、`hint`、`running`、`loaded`）；`OutEvent` 的 `kind` 取值与 `GroupChatWebviewEvent` 的 `snake_case` 变体一一对应。

**已知需执行者留意的点**：
1. Task 5 触及约 10 个文件的接线，改动机械但量大，编译器会逐个指路；`active_workspace_mut`/`with_project` 的确切名称以代码为准。
2. Task 1 的 `view.rs` 占位分支要用到该 `match` 所在函数里已有的 `zone`/`lc` 绑定，先看 `PanelKind::CodeHealth` 分支的写法再抄。
3. 前端 `npm install` 需要联网；`package-lock.json` 必须提交，构建产物 `assets/group-chat-content/*` 每次前端改动后都要重新 `npm run build` 并一起提交（`assets.rs` 的"产物齐全"测试只挡"忘记构建"，挡不住"产物过期"，所以评审时看 diff 里产物是否随源码更新）。
4. 本计划没有给 `HostBinding`/ID 偏移之外的"跨项目串扰"加额外保护：固定单槽 webview 在切项目时由 `take_group_chat_content_script` 用当前激活项目的 payload 声明式覆盖，旧项目的事件在 `route_event` 里按当前 `WorkspaceState` 校验 id 存在性，不存在即丢弃（Review Focus 4 的测试在 Task 2 的 `results_for_a_different_group_are_dropped`）。
