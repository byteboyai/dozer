# 会话审阅面板改用 wry webview 渲染 trace 效果 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **分支要求:** 本计划在独立 worktree 分支上开发(`superpowers:using-git-worktrees`),完成后走 code review 再合并 main——不要直接在 main 上改。main 上经常有其它并发 WIP,开工前先 `git status` 确认 worktree 干净。

**Goal:** 把 `crates/dozer-app` 的"会话审阅"面板(`review_content_pane`)从手写 iced widget 渲染,换成一个 wry webview 渲染的 trace 时间线(折叠展开、分色分级),数据经既有的 `dozer://` 自定义协议以"URL nonce 变化 → 页面重新拉取"的方式传给页面,不引入新的 IPC 握手机制。

**Architecture:** 复用 `crates/dozer-app/src/preview.rs` 的 `WebviewSpec`/`desired_webviews()` 模式与 `crates/dozer-app/src/webview_geometry.rs` 的共享 bounds 算法(Files/Project 面板已在用的同一套),把 `PanelKind::Conversations` 从"纯 iced 绘制,不挂 webview"名单里摘出来,新增一个 `dozer://review-trace/` 协议命名空间,页面加载时 `fetch` 自己的数据(`data.json`),数据来源是 `main.rs` 里一个 `static Mutex<Option<String>>` 快照,`Message::ReviewLoaded` 到达时更新快照 + 递增 nonce 逼 wry 重新导航拉取最新内容。iced 侧只保留"无内容/出错"两个原生文案分支。

**Tech Stack:** Rust workspace(`dozer-core`/`dozer-app` 两个 crate);wry 0.55(已在用);iced 0.14;serde/serde_json(已在用)。

**Spec:** `docs/superpowers/specs/2026-08-21-review-content-webview-trace-design.md`(注意:spec 里"数据流"一节写的是 `evaluate_script` 推送方案,已在写本计划过程中跟用户确认改为本文档描述的 `reload_nonce` + `dozer://` 协议方案——**以本计划为准**,spec 那一节视为过时,不要按 spec 原文实现)。

## Global Constraints

- 每个任务完成后运行:`cargo build -p dozer-app`、`cargo test -p dozer-app`、`cargo clippy -p dozer-app --all-targets`、`cargo fmt -- --check`。
- ByteBoy2077 配色:bg `#0a0e16`、金 `#F2D94E`(甲方动作专属,页面里不要挪用)、奶油 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。
- 审阅面板的"无内容"/"出错"两种原生文案分支保持不变,不纳入 webview 化范围。
- 不改动 `Message::ConversationTurnGroupOpen`/`spawn_review_load`/`conversation_list_pane` 及 dozerd 侧的摄取/查询层。

---

## Task 1: `ReviewEntry` 加 `Serialize` derive

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs:10-23`

**Interfaces:**
- Produces: `ReviewEntry` 实现 `serde::Serialize`,序列化为外部标签形式(`{"Human":{"text":"..."}}`/`{"AiTurn":{"text":"...","tools":[...],"thinking":bool}}`/`{"ToolResult":{"content":"...","is_error":bool}}`),供后续任务(main.rs 序列化推给 webview 的数据快照)使用。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/transcript.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn review_entry_serializes_with_external_tag_shape() {
        let human = ReviewEntry::Human {
            text: "你好".into(),
        };
        assert_eq!(
            serde_json::to_string(&human).unwrap(),
            r#"{"Human":{"text":"你好"}}"#
        );

        let ai = ReviewEntry::AiTurn {
            text: "回复".into(),
            tools: vec!["Edit README.md".into()],
            thinking: true,
        };
        let json = serde_json::to_string(&ai).unwrap();
        assert!(json.starts_with(r#"{"AiTurn":"#));
        assert!(json.contains(r#""thinking":true"#));

        let tool = ReviewEntry::ToolResult {
            content: "boom".into(),
            is_error: true,
        };
        assert_eq!(
            serde_json::to_string(&tool).unwrap(),
            r#"{"ToolResult":{"content":"boom","is_error":true}}"#
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app review_entry_serializes_with_external_tag_shape`
Expected: FAIL(编译错误,`ReviewEntry` 未实现 `Serialize`)

- [ ] **Step 3: 实现**

```rust
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ReviewEntry {
    /// 人类发言（导航锚点）。
    Human { text: String },
    /// AI 一个回合：正文 + 工具一行摘要 + 是否含 thinking。
    AiTurn {
        text: String,
        tools: Vec<String>,
        thinking: bool,
    },
    /// 工具调用的返回结果(2026-08-21 补摄取——此前这类数据在
    /// dozerd 解析层被整体丢弃，见 parse.rs 的 tool_result 处理)。
    ToolResult { content: String, is_error: bool },
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app review_entry_serializes_with_external_tag_shape`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "feat(dozer-app): ReviewEntry 加 Serialize,供审阅面板 webview 化的数据快照用"
```

---

## Task 2: `Workspace`/`ReviewView` 加 `nonce`,`ReviewLoaded` 成功时递增

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:174-184`(`ReviewView` 定义)、`:479-501`(`Workspace` 字段)、`:715-729`(`Workspace::new` 默认值)
- Modify: `crates/dozer-app/src/app.rs:3693-3707`(`Message::ReviewLoaded` 处理)、`:5855-5861`(`conversation_turn_group_open` 里 `ReviewView` 构造)

**Interfaces:**
- Produces: `Workspace.review_nonce: u64`(全局单调递增,每次审阅内容成功加载 +1)、`ReviewView.nonce: u64`(加载完成时从 `Workspace.review_nonce` 拷贝一份)。供 Task 4 的 `review_webview_spec` 用来生成"内容变了就换 URL"的查询参数。

- [ ] **Step 1: 实现(无独立单测——这一步只加字段和递增逻辑,真正的行为断言留给 Task 4 的 `review_webview_spec` 测试,那里能直接构造 `ReviewView { nonce, .. }` 断言 URL 里带上了这个值,比单独测"递增"这个动作本身更有意义)**

`ReviewView` 加字段(紧跟在 `error` 后面,`expanded`/`ai_markdown` 前面):

```rust
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 审阅 webview 的重新加载水位:每次 `ReviewLoaded` 成功都从
    /// `Workspace.review_nonce` 拷一份新值,写进 `dozer://review-trace/
    /// host.html?_r=<nonce>` 的查询参数,逼 wry 在内容变化时重新导航
    /// 拉取(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。
    pub nonce: u64,
    /// 展开了过程区的 AI 回合下标（entries 中的位置）。
    pub expanded: std::collections::HashSet<usize>,
    /// 与 `entries` 等长、下标对齐的预解析 markdown——非 `AiTurn` 条目对应
    /// 位置放一份空 `Content`(构造代价可忽略)。只在 `entries` 落定时
    /// (`ReviewLoaded`)解析一次，`view()` 只管渲染，不重复 parse。
    pub ai_markdown: Vec<markdown::Content>,
}
```

`Workspace` 加字段(紧跟在 `review: Option<ReviewView>` 后面):

```rust
    /// 进行中的会话审阅（审阅 tab 内容;None=未打开;P1i）。
    pub(crate) review: Option<ReviewView>,
    /// 全局单调递增的审阅内容加载水位,每次 `Message::ReviewLoaded`
    /// 成功一次就 +1(与具体加载了哪个回合区间无关)——保证连续点开
    /// 两个不同回合、恰好都是"该 source 第一次加载"时,`ReviewView.nonce`
    /// 也不会撞成同一个值(如果各自从 0 起独立计数会撞)。见
    /// docs/superpowers/plans/2026-08-21-review-content-webview-trace.md
    /// Task 2。
    pub(crate) review_nonce: u64,
```

`Workspace::new` 默认值(紧跟在 `review: None,` 后面):

```rust
            review: None,
            review_nonce: 0,
```

`app.rs` 的 `Message::ReviewLoaded` 处理:

```rust
            Message::ReviewLoaded(project_id, source, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                rv.nonce = nonce;
                                rv.ai_markdown = crate::workspace::parse_review_markdown(&entries);
                                rv.entries = entries;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
```

`conversation_turn_group_open` 里的 `ReviewView` 构造加 `nonce: 0`:

```rust
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                nonce: 0,
                expanded: std::collections::HashSet::new(),
                ai_markdown: Vec::new(),
            });
```

- [ ] **Step 2: 编译确认无报错**

Run: `cargo build -p dozer-app`
Expected: 成功(新增字段有初始值,不影响既有逻辑)

- [ ] **Step 3: 跑既有测试确认没有回归**

Run: `cargo test -p dozer-app`
Expected: 全部 PASS

- [ ] **Step 4: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Workspace/ReviewView 加 nonce,ReviewLoaded 成功时递增"
```

---

## Task 3: 删除审阅面板的手写 iced 条目渲染与配套死代码

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:174-195`(`ReviewView` 定义 + `parse_review_markdown`)、`:2388-2452`(`review_content` 的条目渲染循环)
- Modify: `crates/dozer-app/src/app.rs:1251`(`Message::ReviewToggle` 变体)、`:3700`(`parse_review_markdown` 调用)、`:3709-3717`(`Message::ReviewToggle` 处理)、`:5855-5861`(`ReviewView` 构造)
- Modify: `crates/dozer-app/src/workspace.rs:2416`(`Message::ReviewToggle(i)` 的 `on_press`,随渲染循环一起删)

**Interfaces:**
- Consumes: 无
- Produces: `review_content` 只剩"出错"/"暂无对话"两个原生文案分支,`Human`/`AiTurn`/`ToolResult` 三种条目不再手写 iced 渲染(改由 Task 6/7 的 webview 接管)。`ReviewView.expanded`/`ai_markdown` 字段、`parse_review_markdown` 函数、`Message::ReviewToggle` 变体全部删除。

- [ ] **Step 1: 实现**

`workspace.rs` 的 `ReviewView` 去掉 `expanded`/`ai_markdown` 两个字段(`nonce` 保留,Task 2 刚加的):

```rust
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 审阅 webview 的重新加载水位:每次 `ReviewLoaded` 成功都从
    /// `Workspace.review_nonce` 拷一份新值,写进 `dozer://review-trace/
    /// host.html?_r=<nonce>` 的查询参数,逼 wry 在内容变化时重新导航
    /// 拉取(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。
    pub nonce: u64,
}
```

删除整个 `parse_review_markdown` 函数(原 `workspace.rs:186-195`)。

`review_content` 函数体删掉条目渲染循环,只留出错/空文案两支:

```rust
pub(crate) fn review_content<'a>(
    mut content: iced_widget::Column<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    ws: &'a Workspace,
) -> iced_widget::Column<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(rv) = &ws.review else {
        return content;
    };
    if let Some(err) = &rv.error {
        return content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().red),
        );
    }
    if rv.entries.is_empty() {
        content = content.push(lh(text("暂无对话")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)));
    }
    // 有内容时:真正的渲染由 review_content_pane 区域叠加的 wry webview
    // 负责(dozer://review-trace/host.html,数据经 dozer://review-trace/
    // data.json 拉取),这里不再手写 iced Column——2026-08-21 webview
    // trace 改造,见
    // docs/superpowers/plans/2026-08-21-review-content-webview-trace.md。
    content
}
```

`app.rs` 删除 `Message::ReviewToggle(usize)` 变体定义(原 `app.rs:1251` 附近)、删除其 `update()` 处理分支(原 `app.rs:3709-3717`):

```rust
            Message::ReviewToggle(i) => {
                self.with_focused_project(|ws, _io| {
                    if let Some(rv) = &mut ws.review
                        && !rv.expanded.remove(&i)
                    {
                        rv.expanded.insert(i);
                    }
                });
            }
```
(整段删除)

`app.rs` 的 `Message::ReviewLoaded` 处理里删掉 `ai_markdown` 那一行(`rv.ai_markdown = crate::workspace::parse_review_markdown(&entries);`):

```rust
            Message::ReviewLoaded(project_id, source, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                rv.nonce = nonce;
                                rv.entries = entries;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
```

`conversation_turn_group_open` 里的 `ReviewView` 构造去掉 `expanded`/`ai_markdown`:

```rust
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                nonce: 0,
            });
```

- [ ] **Step 2: 编译,清掉连带出现的 unused import**

Run: `cargo build -p dozer-app 2>&1 | grep -E "error|unused"`
Expected: 无 `error`;如果 `markdown`(`iced_widget::markdown`)、`ReviewMarkdownViewer`、`review_markdown_settings`、`ai_turn_summary`、`button` 等只被删掉的那段代码用到的 import/函数变成 unused,一并删除(`review_markdown_settings`/`ReviewMarkdownViewer`/`ai_turn_summary` 如果只在被删代码里用过,整个函数/impl 也删掉,不留 dead code)。

- [ ] **Step 3: 跑既有测试确认没有回归**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets`
Expected: 全部 PASS,clippy 无 `dead_code` 警告

- [ ] **Step 4: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): 删除审阅面板手写 iced 条目渲染(改由 webview 接管)"
```

---

## Task 4: `review_webview_spec` 纯函数 + 接入 `preview_desired`

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(新增纯函数,放在 `review_content_pane` 附近)
- Modify: `crates/dozer-app/src/app.rs:1936`(新增 ID 常量,紧邻 `PROJECT_PREVIEW_ID_OFFSET`)、`:3505-3549`(`preview_desired` 的 `match kind`)

**Interfaces:**
- Consumes: `ReviewView { entries, error, nonce }`(Task 2/3)
- Produces: `pub(crate) fn review_webview_spec(review: Option<&ReviewView>) -> Vec<preview::WebviewSpec>`;`app.rs` 新增 `pub(crate) const CONVERSATION_REVIEW_ID_OFFSET: usize = 2_000_000;`。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/workspace.rs` 的 `#[cfg(test)] mod tests` 里新增(参照文件内已有测试的 `use super::*;` 写法):

```rust
    #[test]
    fn review_webview_spec_empty_when_no_review() {
        assert_eq!(review_webview_spec(None), Vec::new());
    }

    #[test]
    fn review_webview_spec_empty_on_error_or_empty_entries() {
        let with_error = ReviewView {
            source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: Some("boom".into()),
            nonce: 3,
        };
        assert_eq!(review_webview_spec(Some(&with_error)), Vec::new());

        let empty_entries = ReviewView {
            source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),
            entries: Vec::new(),
            error: None,
            nonce: 3,
        };
        assert_eq!(review_webview_spec(Some(&empty_entries)), Vec::new());
    }

    #[test]
    fn review_webview_spec_url_carries_nonce_and_is_visible() {
        let rv = ReviewView {
            source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            nonce: 7,
        };
        let specs = review_webview_spec(Some(&rv));
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].url, "dozer://review-trace/host.html?_r=7");
        assert!(specs[0].visible);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app review_webview_spec`
Expected: FAIL(编译错误,函数不存在)

- [ ] **Step 3: 实现**

在 `workspace.rs`(`review_content_pane` 函数附近)新增:

```rust
/// 审阅内容的 webview 期望清单(`preview::desired_webviews` 同款语义)。
/// 没有审阅内容 / 出错 / 空回合区间时返回空清单——`sync_webview_pool`
/// 的 `retain` 会据此销毁 webview,不需要额外的隐藏逻辑。有内容时返回
/// 唯一一条,URL 带 `rv.nonce` 当查询参数,内容变化(`Message::ReviewLoaded`
/// 落地新 entries)时 nonce 递增、URL 变化,逼 `sync_webview_pool` 重新
/// `load_url`(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。`id`
/// 固定填 0,真正的池 key 由调用方(`App::preview_desired`)加
/// `CONVERSATION_REVIEW_ID_OFFSET` 决定——这个面板任意时刻只有一份内容,
/// 不需要 Files/Project 那种按 tab id 分池的能力。
pub(crate) fn review_webview_spec(review: Option<&ReviewView>) -> Vec<preview::WebviewSpec> {
    let Some(rv) = review else {
        return Vec::new();
    };
    if rv.error.is_some() || rv.entries.is_empty() {
        return Vec::new();
    }
    vec![preview::WebviewSpec {
        id: 0,
        url: format!("dozer://review-trace/host.html?_r={}", rv.nonce),
        visible: true,
    }]
}
```

`app.rs` 新增 ID 常量(紧邻 `PROJECT_PREVIEW_ID_OFFSET` 定义):

```rust
/// `Conversations` 面板的审阅 webview 只有唯一一份内容,不需要 Files/
/// Project 那种按 tab id 分池——固定用这一个 id(经 `review_webview_spec`
/// 的 `id: 0` 加这个偏移得到),与另两个偏移空间(`0` 起、`PROJECT_
/// PREVIEW_ID_OFFSET` 起)互不相撞。
pub(crate) const CONVERSATION_REVIEW_ID_OFFSET: usize = 2_000_000;
```

`preview_desired` 的 `match kind` 加一支(在 `PanelKind::Project => (...)` 后面):

```rust
            let (specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {
                PanelKind::Files => (ws.preview.desired_webviews(), 0),
                PanelKind::Project => (
                    ws.project_preview.desired_webviews(),
                    PROJECT_PREVIEW_ID_OFFSET,
                ),
                PanelKind::Conversations => (
                    crate::workspace::review_webview_spec(ws.review.as_ref()),
                    CONVERSATION_REVIEW_ID_OFFSET,
                ),
                _ => continue,
            };
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app review_webview_spec`
Expected: PASS

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): review_webview_spec 接入 preview_desired,Conversations 面板加入 webview 期望清单"
```

---

## Task 5: `webview_geometry::preview_content_bounds_for` 补 `Conversations` 真实 bounds

**Files:**
- Modify: `crates/dozer-app/src/webview_geometry.rs:106-115`(放大态 `match kind` 里的 `Conversations` 分支)、`:202-206`(非放大态 `match kind` 里的 `Conversations` 分支)

**Interfaces:**
- Consumes: `PanelKind::Conversations`(已有)、`state.dims.conversations_split`(已有,`app.rs:379`)、`pair_columns`(已有)
- Produces: `preview_content_bounds_for` 对 `side` 是 `PanelKind::Conversations` 时返回真实矩形(不再是占位 `(0,0,0,0)`)。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/webview_geometry.rs` 的 `#[cfg(test)] mod tests` 里新增(用文件里已有的 `test_state()` 辅助函数):

```rust
    #[test]
    fn preview_content_bounds_conversations_review_content_is_first_when_not_mirrored() {
        // app.rs 的 PanelKind::Conversations 分支(zone 渲染,`else` 臂):
        // 未镜像时渲染顺序是 [review, divider, list]——跟 pair_columns 的
        // 内建默认("mirrored=false → list 先")相反,所以必须传 `!mirrored`
        // (同 Web 收藏夹那处的手法),否则 webview 会摆到列表底下而不是
        // 列表前面。
        let state = ShellState {
            left_view: PanelKind::Conversations,
            ..test_state()
        };
        let (x, _y, w, _h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        let m = theme::region::left_zone().margin;
        let col_start = byteui::theme::geometry::icon_rail_width() + m.left;
        assert!(
            x >= col_start && x < col_start + 16.0,
            "非镜像态 review 内容应紧贴面板区左边界(pair 里第一个元素): x={x}"
        );
        assert!(w > 200.0, "w={w}");
    }

    #[test]
    fn preview_content_bounds_conversations_zero_when_left_collapsed() {
        let state = ShellState {
            left_view: PanelKind::Conversations,
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app preview_content_bounds_conversations`
Expected: 第一条 FAIL(当前 `Conversations` 分支恒返回 `(0,0,0,0)`,`w=0` 不满足 `w > 200.0`);第二条本来就该 PASS(占位分支恒零)。

- [ ] **Step 3: 实现**

非放大态(`webview_geometry.rs:202-206`)原文:

```rust
        // Stage 4a 跨栏拖拽:该侧视图可为另一栏面板,纯 iced 绘制、该侧
        // 无 webview 可摆,装空矩形。
        PanelKind::Agent | PanelKind::Conversations | PanelKind::Usage | PanelKind::Acceptance => {
            (0.0, 0.0, 0.0, 0.0)
        }
```

改成:

```rust
        // Stage 4a 跨栏拖拽:该侧视图可为另一栏面板,纯 iced 绘制、该侧
        // 无 webview 可摆,装空矩形。
        PanelKind::Agent | PanelKind::Usage | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
        // 审阅内容(2026-08-21 webview trace 改造):跟 Files/Project 同款
        // "配对列宽 + preview chrome 高度"算法,但 `mirrored` 要取反——
        // app.rs 的 `PanelKind::Conversations` 分支未镜像时渲染顺序是
        // `[review, divider, list]`(内容先),跟 `pair_columns` 的内建
        // 默认("mirrored=false → list 先")相反,同 Web 收藏夹那处的手法。
        PanelKind::Conversations => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.conversations_split, !mirrored);
            let x = zone_x0 + cols.content_x + 8.0 + m.left;
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
```

放大态(`webview_geometry.rs:106-115`)原文:

```rust
            // Database/Ssh/Todo/GitLog 纯 iced 绘制,不挂 webview 子视图;
            // Agent/Conversations/Usage/Acceptance 同理——任一侧放大只要
            // 显示的是这几种,都没有 webview 可摆。
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Conversations
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
```

改成:

```rust
            // Database/Ssh/Todo/GitLog 纯 iced 绘制,不挂 webview 子视图;
            // Agent/Usage/Acceptance 同理——任一侧放大只要显示的是这几种,
            // 都没有 webview 可摆。
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
            // 审阅内容放大态:跟非放大态同一份 `!mirrored` 理由,只是
            // x0/avail_w/avail_h 换成放大盒子的换算(同 Files/Project 放大
            // 态分支)。
            PanelKind::Conversations => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.conversations_split, !mirrored);
                let x = x0 + cols.content_x + 8.0;
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app preview_content_bounds_conversations`
Expected: 两条都 PASS

Run: `cargo test -p dozer-app`
Expected: 全部 PASS(既有 Files/Project/Web 的 bounds 测试不受影响)

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/webview_geometry.rs
git commit -m "feat(dozer-app): preview_content_bounds_for 补 Conversations 面板真实 bounds"
```

---

## Task 6: `review_trace.html` 页面 + `dozer://review-trace/` 协议命名空间

**Files:**
- Create: `crates/dozer-app/src/review_trace.html`
- Modify: `crates/dozer-app/src/assets.rs`(`handle_protocol` 签名与实现、`#[cfg(test)] mod tests` 全部调用点)

**Interfaces:**
- Consumes: 无(纯字符串路由 + 注入的 `review_data` 参数)
- Produces: `handle_protocol` 签名变为 `pub fn handle_protocol(assets_root: &Path, allowed: &HashSet<PathBuf>, review_data: Option<&str>, uri: &str) -> ProtocolReply`;新增 `dozer://review-trace/host.html`(内嵌静态页面)与 `dozer://review-trace/data.json`(回显 `review_data`,`None` 时 404)两个端点。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/assets.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn review_trace_host_html_serves_embedded_page_regardless_of_review_data() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://review-trace/host.html?_r=1");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(!r.body.is_empty());
    }

    #[test]
    fn review_trace_data_json_echoes_injected_snapshot() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            Some(r#"[{"Human":{"text":"hi"}}]"#),
            "dozer://review-trace/data.json",
        );
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "application/json");
        assert_eq!(r.body, br#"[{"Human":{"text":"hi"}}]"#);
    }

    #[test]
    fn review_trace_data_json_404_when_no_snapshot() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://review-trace/data.json");
        assert_eq!(r.status, 404);
    }

    #[test]
    fn review_trace_unknown_subpath_404() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://review-trace/nope");
        assert_eq!(r.status, 404);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app review_trace_`
Expected: FAIL(编译错误——`handle_protocol` 还是 3 个参数,这几条测试传了 4 个)

- [ ] **Step 3: 实现**

先创建 `crates/dozer-app/src/review_trace.html`:

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8" />
<title>trace</title>
<style>
  :root {
    --bg: #0a0e16;
    --gold: #F2D94E;
    --cream: #FFE5B4;
    --cyan: #47DEF0;
    --green: #1AD585;
    --red: #ff5c5c;
    --dim: #6b7280;
    --line: #232b3a;
  }
  * { box-sizing: border-box; }
  html, body {
    margin: 0; height: 100%;
    background: var(--bg); color: var(--cream);
    font: 13px/1.5 -apple-system, "PingFang SC", sans-serif;
  }
  #timeline { height: 100%; overflow-y: auto; padding: 16px 20px; }
  .entry { position: relative; padding-left: 22px; margin-bottom: 14px; }
  .entry::before {
    content: ""; position: absolute; left: 6px; top: 4px; bottom: -14px;
    width: 1px; background: var(--line);
  }
  .entry:last-child::before { display: none; }
  .entry::after {
    content: ""; position: absolute; left: 2px; top: 4px;
    width: 9px; height: 9px; border-radius: 50%;
  }
  .entry.Human::after { background: var(--gold); }
  .entry.AiTurn::after { background: var(--cyan); }
  .entry.ToolResult::after { background: var(--green); }
  .entry.ToolResult.error::after { background: var(--red); }
  .entry .role {
    font-size: 11px; text-transform: uppercase; letter-spacing: .04em;
    margin-bottom: 3px;
  }
  .entry.Human .role { color: var(--gold); }
  .entry.AiTurn .role { color: var(--cyan); }
  .entry.ToolResult .role { color: var(--green); }
  .entry.ToolResult.error .role { color: var(--red); }
  .entry .text { white-space: pre-wrap; }
  .entry.ToolResult .text {
    color: var(--dim); font-family: ui-monospace, monospace; font-size: 12px;
  }
  .entry.ToolResult.error .text { color: var(--red); }
  .tools { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
  .tool-pill {
    background: #10151f; border: 1px solid var(--line);
    color: var(--cyan); border-radius: 4px; padding: 2px 8px; font-size: 11px;
  }
  .thinking-badge {
    display: inline-block; margin-top: 4px; font-size: 11px; color: var(--dim);
    border: 1px dashed var(--line); border-radius: 4px; padding: 1px 6px;
  }
  details.tool-result-body summary {
    cursor: pointer; color: var(--dim); font-size: 11px; margin-top: 2px;
  }
</style>
</head>
<body>
<div id="timeline"></div>
<script>
const ROLE_LABEL = { Human: "甲方", AiTurn: "AI", ToolResult: "工具结果" };

function renderEntry(entry) {
  const kind = Object.keys(entry)[0];
  const v = entry[kind];
  const el = document.createElement("div");
  el.className = "entry " + kind + (kind === "ToolResult" && v.is_error ? " error" : "");

  const role = document.createElement("div");
  role.className = "role";
  role.textContent = ROLE_LABEL[kind] || kind;
  el.appendChild(role);

  if (kind === "ToolResult") {
    const details = document.createElement("details");
    details.className = "tool-result-body";
    details.open = v.content.length < 200;
    const summary = document.createElement("summary");
    summary.textContent = (v.is_error ? "⚠ 失败 · " : "→ ") + v.content.split("\n")[0].slice(0, 80);
    const body = document.createElement("div");
    body.className = "text";
    body.textContent = v.content;
    details.appendChild(summary);
    details.appendChild(body);
    el.appendChild(details);
    return el;
  }

  if (kind === "Human") {
    const text = document.createElement("div");
    text.className = "text";
    text.textContent = v.text;
    el.appendChild(text);
    return el;
  }

  if (v.text) {
    const text = document.createElement("div");
    text.className = "text";
    text.textContent = v.text;
    el.appendChild(text);
  }
  if (v.thinking) {
    const badge = document.createElement("span");
    badge.className = "thinking-badge";
    badge.textContent = "思考过程(略)";
    el.appendChild(badge);
  }
  if (v.tools && v.tools.length) {
    const tools = document.createElement("div");
    tools.className = "tools";
    v.tools.forEach(function (s) {
      const pill = document.createElement("span");
      pill.className = "tool-pill";
      pill.textContent = s;
      tools.appendChild(pill);
    });
    el.appendChild(tools);
  }
  return el;
}

fetch("dozer://review-trace/data.json")
  .then(function (r) { return r.json(); })
  .then(function (entries) {
    const el = document.getElementById("timeline");
    el.innerHTML = "";
    entries.forEach(function (entry) { el.appendChild(renderEntry(entry)); });
  })
  .catch(function (err) {
    document.getElementById("timeline").textContent = "加载失败: " + err;
  });
</script>
</body>
</html>
```

`assets.rs` 的 `handle_protocol` 改签名 + 加分支:

```rust
pub fn handle_protocol(
    assets_root: &Path,
    allowed: &HashSet<PathBuf>,
    review_data: Option<&str>,
    uri: &str,
) -> ProtocolReply {
    // 剥离 scheme 与 query;只服务 flyfish/review-trace 两个命名空间。
    let Some(rest) = uri.strip_prefix("dozer://") else {
        return not_found();
    };
    let rest = rest.split('?').next().unwrap_or(rest);

    // 审阅面板 trace 页面(2026-08-21):页面本身是编译期内嵌的静态资源,
    // 不走磁盘;数据端点回显调用方注入的当前审阅内容快照——没有快照
    // (还没加载过审阅内容)时 404。
    if let Some(path) = rest.strip_prefix("review-trace/") {
        return match path {
            "host.html" => ProtocolReply {
                status: 200,
                mime: "text/html",
                body: include_str!("review_trace.html").as_bytes().to_vec(),
            },
            "data.json" => match review_data {
                Some(json) => ProtocolReply {
                    status: 200,
                    mime: "application/json",
                    body: json.as_bytes().to_vec(),
                },
                None => not_found(),
            },
            _ => not_found(),
        };
    }

    let Some(path) = rest.strip_prefix("flyfish/") else {
        return not_found();
    };

    // 本地文件端点:__file__/<百分号编码的绝对路径>(编码保留 '/')。
    if let Some(encoded) = path.strip_prefix("__file__") {
        let Some(decoded) = percent_decode(encoded) else {
            return not_found();
        };
        let file = PathBuf::from(decoded);
        if !allowed.contains(&file) {
            return not_found();
        }
        return match std::fs::read(&file) {
            Ok(body) => ProtocolReply {
                status: 200,
                mime: mime_for(&file),
                body,
            },
            Err(_) => not_found(),
        };
    }

    // vendored 资产:先逐段解码,再拒绝路径穿越——编码形态也拦得住:
    // %2e%2e 解码成 '..' 后才比较;%2F 解码出的 '/' 直接判拒。
    let mut full = assets_root.to_path_buf();
    for seg in path.split('/') {
        let Some(seg) = percent_decode(seg) else {
            return not_found();
        };
        if seg.is_empty() || seg == ".." || seg == "." || seg.contains('/') || seg.contains('\0') {
            return not_found();
        }
        full.push(seg);
    }
    match std::fs::read(&full) {
        Ok(body) => ProtocolReply {
            status: 200,
            mime: mime_for(&full),
            body,
        },
        Err(_) => not_found(),
    }
}
```

既有测试模块里所有 `handle_protocol(...)` 调用点,在 `allowed` 参数后面插入一个 `None,`(新增第 3 个参数),例如:

```rust
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://flyfish/host.html");
```

按同样方式改掉 `serves_vendored_asset_with_mime`、`rejects_traversal_and_unknown`(3 处 `handle_protocol` 调用)、`file_endpoint_requires_allowlist`(2 处)、`rejects_percent_encoded_traversal`(循环体里的 1 处)——原文件一共 9 处调用,全部加上 `None,` 这一个参数,不改变其它参数顺序/取值。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app review_trace_`
Expected: PASS

Run: `cargo test -p dozer-app assets::`
Expected: 既有 `serves_vendored_asset_with_mime`/`rejects_traversal_and_unknown`/`file_endpoint_requires_allowlist`/`rejects_percent_encoded_traversal`/`percent_decode_roundtrip` 全部 PASS(签名加参数后行为不变)

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/assets.rs crates/dozer-app/src/review_trace.html
git commit -m "feat(dozer-app): 新增 dozer://review-trace 协议命名空间 + trace 页面"
```

---

## Task 7: main.rs 接线——快照更新 + 协议闭包传参

**Files:**
- Modify: `crates/dozer-app/src/main.rs`(新增 module 级 `static`、`dispatch()` 新增拦截分支、`sync_webview_pool` 内的协议闭包)

**Interfaces:**
- Consumes: `Message::ReviewLoaded`(已有)、`assets::handle_protocol` 新签名(Task 6)
- Produces: 审阅内容成功加载时,`REVIEW_SNAPSHOT` 快照同步更新,供后续任意 webview(不只 Conversations 面板新建的那个,所有走 `dozer://` 协议的 webview 共用同一个协议闭包)在收到 `dozer://review-trace/data.json` 请求时读到最新内容。

- [ ] **Step 1: 实现(main.rs 这部分是事件循环接线,没有独立单测——正确性靠 Task 6 的 `handle_protocol` 单测 + Task 4 的 `review_webview_spec` 单测覆盖,这里只负责把两者接起来;验收方式见 Step 2 的编译检查 + Step 3 的人工 GUI 验收)**

在 `main.rs` 里 `enum Runner { ... }` 定义之前(紧邻 `let app = runtime.block_on(...)` 之后,任何函数体顶层 item 都可以,放在 `Runner` 定义前最贴近使用处)新增:

```rust
    /// 审阅 webview 的当前内容快照——`Message::ReviewLoaded` 成功时在
    /// `dispatch()` 里写入(JSON 字符串),`dozer://review-trace/data.json`
    /// 协议端点读取。用 `static` 而不是 `Runner::Ready` 的字段,是因为
    /// `sync_webview_pool` 的协议闭包在 `Files`/`Project`/`browser_webviews`
    /// 三个池之间是同一份代码、各自独立捕获——塞进某个池的结构体字段够不
    /// 到另一个池的闭包,`static` 是所有闭包都能直接引用的最简单办法。
    /// 见 docs/superpowers/plans/2026-08-21-review-content-webview-trace.md
    /// Task 7。
    static REVIEW_SNAPSHOT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
```

`sync_webview_pool` 内新建 webview 时的协议闭包(原 `main.rs:594-606`)加一行读快照、多传一个参数:

```rust
                        .with_custom_protocol("dozer".into(), move |_id, request| {
                            let allowed = allowed.lock().expect("allowed_files 锁");
                            let review_data = REVIEW_SNAPSHOT.lock().expect("review snapshot 锁");
                            let reply = assets::handle_protocol(
                                &root,
                                &allowed,
                                review_data.as_deref(),
                                &request.uri().to_string(),
                            );
                            wry::http::Response::builder()
                                .status(reply.status)
                                .header("Content-Type", reply.mime)
                                .body(std::borrow::Cow::Owned(reply.body))
                                .expect("构造协议应答")
                        })
```

`dispatch()`(`main.rs:1377` 起的 `fn dispatch(&mut self, message: Message)`)在现有 `Message::Browser(extensions::browser::Message::Nav(action)) => { ... }` 分支后面、`other => app.update(other)` 前面新增一支:

```rust
                // 审阅内容加载成功时,把 entries 序列化进快照,供
                // `dozer://review-trace/data.json` 协议端点读取——webview
                // 句柄摸不到(spike 约束 2),不能直接 evaluate_script 推,
                // 靠 `review_webview_spec` 的 nonce 变化逼 wry 重新导航、
                // 页面自己 fetch 拉取最新快照(Task 4/6)。
                Message::ReviewLoaded(project_id, source, result) => {
                    if let Ok(entries) = &result {
                        let json = serde_json::to_string(entries).unwrap_or_default();
                        *REVIEW_SNAPSHOT.lock().expect("review snapshot 锁") = Some(json);
                    }
                    app.update(Message::ReviewLoaded(project_id, source, result));
                }
```

- [ ] **Step 2: 编译确认无报错**

Run: `cargo build -p dozer-app`
Expected: 成功

- [ ] **Step 3: 跑全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt -- --check`
Expected: 全部 PASS,clippy 无警告

- [ ] **Step 4: 人工 GUI 验收**

Run: `cargo run -p dozer-app`

验收清单:
1. 打开任意有历史会话的项目,切到"会话"面板(`PanelKind::Conversations`),点左侧列表里任意一条 → 右侧应出现 wry webview 渲染的 trace 时间线(金色圆点=甲方发言、青色=AI、绿色/红色=工具结果成功/失败),不是原来的纯文字列表。
2. 折叠展开工具结果(`<details>`)的交互能正常点开/收起。
3. 连续点击列表里不同的回合行,右侧内容应跟着切换到新回合的 trace(验证 nonce 递增确实触发了 `load_url` 重新加载,不是停在第一次加载的内容不动)。
4. 把"会话"面板拖到镜像态(拖拽切到默认栏另一侧,或用命令面板切换面板位置)、缩放窗口、放大/还原该侧面板,webview 位置应跟着正确跟随,不越界、不跟另一侧内容重叠。
5. 编辑弹层打开时(如打开验收面板的编辑态)、⌘K 命令面板打开时,审阅 webview 应正确隐藏,不残留在最上层。
6. 会话为空或加载失败的回合,应显示原生"暂无对话"/红色错误文案,不应有 webview 空白闪烁。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): 接线审阅内容快照到 dozer://review-trace 协议,Conversations 面板 webview 化完工"
```
