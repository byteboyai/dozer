# review-trace 白屏闪烁 loading 遮罩 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 切换审阅目标(核心 Agent 面板选中新终端会话 / 对话面板点开另一条历史会话)时,`review-trace` webview 整页重新导航造成的白屏闪烁,换成"原生 loading 占位 → 内容"的观感,不消除重载本身。

**Architecture:** 复用本仓库 CodeMirror/JSON/Flyfish 三个 host 已验证的"webview 预创建为 hidden,JS 报 `document_loaded` 后才切可见"模式,接进 `review-trace` 的单槽非 tab 模型:`ReviewView` 新增 `loaded_nonce: Option<u64>`,`review_webview_spec` 的 `visible` 从恒真改成 `loaded_nonce == Some(nonce)`,`review_content_pane` 在未 loaded 时渲染原生占位;前端 `main.tsx` 在 fetch 成功/失败/渲染异常三种终态都 `postMessage` 报回 `document_loaded`(复用既有 `WebviewEnvelope<T>` 通用信封,新增最小的 `ReviewTraceEvent` payload);`runtime.rs` 按固定 `CONVERSATION_REVIEW_ID_OFFSET` webview id 识别并派发。

**Tech Stack:** 与既有 `preview::webview_protocol` 一致(serde,`WebviewEnvelope<T>` 泛型信封);前端沿用已有 `web/review-trace` 的 Preact + esbuild 管线,不新增依赖。

**Spec:** `docs/superpowers/specs/2026-09-26-review-trace-loaded-mask-design.md`

## Global Constraints

- 不改 `data.json` 契约、不改 review-trace 任何视觉渲染组件(`Entry`/`SummaryHeader`/`markdown.ts` 等全部不动)。
- 不引入 `PreviewTab`/`backend_state`/`finish_load` 这套 tab 模型——只借鉴思路,不复用代码(review-trace 是单槽非 tab,状态形状不同)。
- 不做 `FlyfishEvent::Failed`/`recoverable` 那一层重试 UI——review-trace 的失败已经是页面内可见的错误文案,`document_loaded` 在成功/失败/渲染异常三种终态都要触发,让原生占位及时让位。
- `review_webview_spec` 里 `rv.error.is_some() || rv.entries.is_empty()` 时返回空清单(销毁 webview)的既有逻辑不动——那是"根本没有数据"，跟本次"有数据、正在加载"是两回事。
- `nonce` 递增/URL 重新导航机制(`Message::ReviewLoaded` 落地新内容触发)不动,这次只加一层可见性遮罩。
- 两个消费入口(核心 Agent 面板 `ReviewSource::Session`、对话面板 `ReviewSource::Conversation`)共用同一份 `ws.review`,任何改动自动对两者生效,不需要分别处理。

## Review Focus

- **过期 `document_loaded` 事件把 `loaded_nonce` 设错**:用户快速连续切换两次审阅目标,第一次的 `document_loaded` 事件在第二次 `nonce` 已经自增之后才姗姗来迟——Task 3 的测试要显式覆盖"事件到达时事件所属的 nonce 已经不是当前 nonce"这个时序,不能假设消息一定按发出顺序处理。
- **JS 渲染期异常(不是 fetch 失败)导致 `document_loaded` 永远发不出去**:`main.tsx` 已有的 `RenderErrorBoundary` 捕获子组件渲染异常,但如果 `postMessage` 只挂在 fetch 的 `.then`/`.catch` 上,渲染异常这条路径完全绕过它——Task 6 要把上报逻辑挂在"data 或 error 任一个终态被设置"这个更上层的时机,覆盖三种终态,不能只挂 fetch 回调。
- **`ws.review` 在 `document_loaded`事件到达前被切换成 `None`**(用户切走审阅面板 / 关闭会话):`update.rs` 处理事件时如果无脑假设 `ws.review` 一定是 `Some`会 panic——Task 3 要用 `if let Some(rv) = &mut ws.review` 守卫,并写测试覆盖"事件到达时 `ws.review` 已经是 `None`"这个边界。
- **`review_webview_spec` 现有的空清单分支(无数据)与新的"未 loaded"分支职责重叠**:`rv.entries.is_empty()` 为真时应该走既有的"销毁 webview"分支,不能被新加的可见性判断抢先命中——Task 2 的测试要覆盖"entries 为空 + loaded_nonce 恰好等于 nonce"这种理论上不该出现但要防呆的组合,确认空清单分支优先。
- **`review_content_pane` 的三态渲染顺序错误**:未 loaded 时如果误判成"有内容"分支(渲染 `Scrollable`)而不是"loading 占位"分支,webview 隐藏期间用户会看到一个空的可滚动区域而不是有意义的 loading 提示——Task 4 要写测试锁定"有内容但未 loaded"必须命中占位分支,不是既有的两个分支之一。

---

## 文件结构总览

```text
crates/dozer-app/src/preview/webview_protocol.rs   # 修改:新增 ReviewTraceEvent + parse_review_trace_event
crates/dozer-app/src/workspace/state.rs            # 修改:ReviewView 新增 loaded_nonce 字段
crates/dozer-app/src/workspace/view.rs             # 修改:review_webview_spec 可见性、review_content_pane 新分支
crates/dozer-app/src/app/message.rs                # 修改:新增 Message::ReviewTraceWebviewEvent
crates/dozer-app/src/app/update.rs                 # 修改:处理该消息 + ReviewView 构造点补字段
crates/dozer-app/src/runtime.rs                    # 修改:ipc_handler 识别 review-trace webview id
crates/dozer-app/web/review-trace/src/main.tsx     # 修改:三种终态上报 document_loaded
```

---

### Task 1: `ReviewTraceEvent` 协议类型 + 解析函数

**Files:**
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs`

**Interfaces:**
- Consumes: 既有 `WebviewEnvelope<T>`/`PROTOCOL_VERSION`/`MAX_MESSAGE_BYTES`/`ProtocolError`(同文件已有)。
- Produces: `ReviewTraceEvent`(`pub`)、`parse_review_trace_event(raw: &str) -> Result<WebviewEnvelope<ReviewTraceEvent>, ProtocolError>`,供 Task 5(`runtime.rs`)使用。

- [ ] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/preview/webview_protocol.rs 的
// `#[cfg(test)] mod tests` 块内,紧邻既有 `parse_flyfish_event` 测试
#[test]
fn parse_review_trace_event_accepts_minimal_envelope() {
    let raw = r#"{"protocol_version":1,"payload":{"kind":"document_loaded"}}"#;
    let env = parse_review_trace_event(raw).expect("应解析成功");
    assert_eq!(env.payload, ReviewTraceEvent::DocumentLoaded);
    // 省略的信封字段(project_id/panel/tab_id/document_id/revision/
    // request_id)全部落到 `#[serde(default)]`,不要求 JS 侧提供——
    // review-trace 单槽非 tab,没有这些身份需要携带。
    assert_eq!(env.project_id, 0);
    assert_eq!(env.tab_id, 0);
}

#[test]
fn parse_review_trace_event_rejects_unknown_kind() {
    let raw = r#"{"protocol_version":1,"payload":{"kind":"bogus"}}"#;
    assert!(parse_review_trace_event(raw).is_err());
}

#[test]
fn parse_review_trace_event_rejects_oversized_message() {
    let huge = "x".repeat(MAX_MESSAGE_BYTES + 1);
    let raw = format!(r#"{{"protocol_version":1,"payload":{{"kind":"document_loaded","pad":"{huge}"}}}}"#);
    assert!(matches!(
        parse_review_trace_event(&raw),
        Err(ProtocolError::TooLarge { .. })
    ));
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app webview_protocol::tests::parse_review_trace_event
```

Expected: FAIL,`ReviewTraceEvent`/`parse_review_trace_event` 未定义。

- [ ] **Step 3: 实现**(紧接着 `parse_flyfish_event` 之后追加,逐行对照它的结构——size 检查/`kind` 预取用于错误信息/`serde_json::from_value` 转最终类型)

```rust
/// review-trace(单槽、非 tab)host 报回的事件。只有一个变体——
/// review-trace 的失败已经是页面内可见的错误文案(见
/// `web/review-trace/src/main.tsx` 的 `RenderErrorBoundary`/`.catch`),
/// Rust 不需要单独知道"是不是失败了"才决定要不要显示,成功/失败/渲染
/// 异常三种终态都报同一个事件。不复用 `FlyfishEvent`(那个带
/// `Title`/`SearchState`/`Failed{recoverable}`,review-trace 没有这些
/// 交互,复用只会引入不必要的字段)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewTraceEvent {
    DocumentLoaded,
}

/// 解析一条 review-trace host 事件。与 [`parse_flyfish_event`] 同规则
/// (超大/非法/未知不 panic)。
pub fn parse_review_trace_event(raw: &str) -> Result<WebviewEnvelope<ReviewTraceEvent>, ProtocolError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { bytes: raw.len() });
    }
    let env: WebviewEnvelope<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| ProtocolError::BadJson(e.to_string()))?;
    let kind = env
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let payload: ReviewTraceEvent = serde_json::from_value(env.payload)
        .map_err(|_| ProtocolError::UnknownPayload(kind.clone()))?;
    Ok(WebviewEnvelope {
        protocol_version: env.protocol_version,
        project_id: env.project_id,
        panel: env.panel,
        tab_id: env.tab_id,
        document_id: env.document_id,
        revision: env.revision,
        request_id: env.request_id,
        payload,
    })
}
```

> 若 `ProtocolError::TooLarge`/`ProtocolError::UnknownPayload`/`ProtocolError::BadJson` 的具体字段名与既有 `parse_flyfish_event` 引用的不完全一致,以 `parse_flyfish_event` 现有实现(紧邻上方)实际引用的变体名为准照抄,不要凭空猜字段名。

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app webview_protocol::tests::parse_review_trace_event
```

Expected: 3 个测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview/webview_protocol.rs
git commit -m "feat(review-trace): 新增 loaded 遮罩事件协议 ReviewTraceEvent"
```

---

### Task 2: `ReviewView.loaded_nonce` + `review_webview_spec` 可见性

**Files:**
- Modify: `crates/dozer-app/src/workspace/state.rs`
- Modify: `crates/dozer-app/src/workspace/view.rs`
- Modify: `crates/dozer-app/src/app/update.rs`(`ReviewView` 唯一构造点补字段)

**Interfaces:**
- Produces: `ReviewView::loaded_nonce: Option<u64>` 字段,`review_webview_spec` 新可见性判据,供 Task 3(事件落地)、Task 4(原生占位)使用。

- [ ] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/workspace/view.rs 的 `#[cfg(test)] mod tests`
// 块内,紧邻既有 `review_webview_spec_*` 测试
#[test]
fn review_webview_spec_hidden_until_loaded_nonce_matches() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: vec![ReviewEntry::Human { text: "hi".into() }],
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: None,
    };
    let specs = review_webview_spec(Some(&rv));
    assert_eq!(specs.len(), 1, "有数据时仍应创建 webview(否则永远等不到 loaded 事件)");
    assert!(!specs[0].visible, "loaded_nonce 未追上 nonce 时不可见");
}

#[test]
fn review_webview_spec_visible_when_loaded_nonce_matches() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: vec![ReviewEntry::Human { text: "hi".into() }],
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: Some(3),
    };
    let specs = review_webview_spec(Some(&rv));
    assert!(specs[0].visible);
}

/// 对应本计划 Review Focus"空清单分支与未 loaded 分支职责重叠":即便
/// `loaded_nonce == nonce`(理论上不该跟"无数据"同时成立,但要防呆),
/// `entries.is_empty()` 仍必须优先命中既有的"销毁 webview"分支。
#[test]
fn review_webview_spec_empty_entries_still_wins_over_loaded_nonce() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: Vec::new(),
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: Some(3),
    };
    assert_eq!(review_webview_spec(Some(&rv)), Vec::new());
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app workspace::view::tests::review_webview_spec
```

Expected: FAIL,`ReviewView` 缺 `loaded_nonce` 字段(既有测试构造点因为字段不匹配也会一并编译失败,属预期,Step 3 一起修)。

- [ ] **Step 3: 实现**

```rust
// crates/dozer-app/src/workspace/state.rs::ReviewView,summary_time 字段
// 之后新增
/// 当前 `nonce` 对应的内容是否已经被 review-trace webview 报回
/// `ReviewTraceEvent::DocumentLoaded`——`None`/不等于 `nonce` 都算"未
/// loaded"。换审阅目标时 `nonce` 自增,这个字段不需要显式清空,天然因
/// 为不等于新 `nonce` 而回到"未 loaded"(见 `review_webview_spec`)。
pub loaded_nonce: Option<u64>,
```

```rust
// crates/dozer-app/src/workspace/view.rs::review_webview_spec,把
// `visible: true,` 改成:
visible: rv.loaded_nonce == Some(rv.nonce),
```

```rust
// crates/dozer-app/src/app/update.rs:5126 附近,ReviewView 唯一构造点
// 补上新字段(初始未 loaded)
ws.review = Some(ReviewView {
    source: source.clone(),
    entries: Vec::new(),
    error: None,
    agent,
    nonce: 0,
    summary_title: Some(summary_title),
    summary_text,
    summary_time: Some(relative_time_text(last_ts, now_ms)),
    loaded_nonce: None,
});
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app workspace
```

Expected: 全部 PASS(含既有 `review_webview_spec_empty_when_no_review`/`review_webview_spec_empty_on_error_or_empty_entries`/`review_webview_spec_url_carries_nonce_and_is_visible`——最后这个既有测试断言"恒可见",本任务改了这个语义,需要同步更新它的断言而不是删掉,见下一步)。

- [ ] **Step 5: 更新一个因语义变化而需要调整断言的既有测试**

```rust
// crates/dozer-app/src/workspace/tests.rs(或既有测试所在文件)里的
// review_webview_spec_url_carries_nonce_and_is_visible——原断言"恒可见"
// 已不成立,补上 loaded_nonce 匹配的前提,函数名可以保留(它验证的核心
// ——URL 带 nonce——没变,只是"可见"这半句需要加前提)
#[test]
fn review_webview_spec_url_carries_nonce_and_is_visible_when_loaded() {
    let rv = ReviewView {
        // ...既有字段照旧...
        loaded_nonce: Some(/* 与既有测试里的 nonce 值一致 */ 0),
    };
    let specs = review_webview_spec(Some(&rv));
    assert!(specs[0].url.contains(&format!("_r={}", rv.nonce)));
    assert!(specs[0].visible);
}
```

- [ ] **Step 6: 运行全部 workspace 测试确认通过**

```bash
cargo test -p dozer-app workspace
```

Expected: 全部 PASS。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace/state.rs crates/dozer-app/src/workspace/view.rs crates/dozer-app/src/workspace/tests.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(review-trace): ReviewView 新增 loaded_nonce,webview 默认隐藏直到 loaded"
```

---

### Task 3: `Message::ReviewTraceWebviewEvent` + update.rs 落地

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Consumes: `ReviewTraceEvent`(Task 1)。
- Produces: `Message::ReviewTraceWebviewEvent(ReviewTraceEvent)`,供 Task 5(`runtime.rs` 派发)使用;`update.rs` 处理后落地 `ws.review.loaded_nonce`。

> 不带 binding、不带 project_id 定位——`ws.review` 是"当前聚焦项目"的字段(同 `ReviewLoaded` 现状用 `self.with_project(project_id, ...)` 定位的方式不同,`ReviewView` 本身不按 project_id 路由,是 `Workspace` 的直接字段),消息落地时操作 `self.active_workspace_mut()`(或既有等价方法)拿到的那个 workspace,不需要额外携带项目身份。

- [ ] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/app/update.rs 的 `#[cfg(test)] mod tests`
// 块内(或既有测试模块,视 update.rs 现有测试组织方式而定)
#[test]
fn review_trace_webview_event_sets_loaded_nonce_to_current_nonce() {
    let mut app = test_app_with_project(); // 复用既有测试辅助函数,构造一个有活跃项目的 App
    app.with_focused_project(|ws, _io| {
        ws.review = Some(ReviewView {
            source: ReviewSource::Conversation("c1".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce: 5,
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce: None,
        });
    });
    app.update(Message::ReviewTraceWebviewEvent(ReviewTraceEvent::DocumentLoaded));
    app.with_focused_project(|ws, _io| {
        assert_eq!(ws.review.as_ref().unwrap().loaded_nonce, Some(5));
    });
}

/// 对应本计划 Review Focus"过期事件":事件到达时 nonce 已经因为又切了
/// 一次审阅目标而自增,`loaded_nonce` 应该被设成事件到达那一刻的**当前**
/// nonce(6),不是事件本该对应的旧 nonce——因为事件本身不携带"我是为哪个
/// nonce 报的"这个信息(简化设计,见 spec),Rust 侧只能假设"最近一次收到
/// 的 document_loaded 对应当前 nonce"。这意味着:如果旧页面的事件在新
/// 页面已经导航之后才姗姗来迟,会错误地让 loaded_nonce 提前追上新
/// nonce(新页面其实还没真正 loaded)。
///
/// **已知限制,不在本任务修复范围**(见 spec"风险与边界"——协议刻意从简,
/// 不携带 nonce);这条测试的目的是记录这个已知行为,不是断言"正确"结果,
/// 避免未来有人以为这是未测试的疏漏。
#[test]
fn review_trace_webview_event_stale_event_still_advances_to_current_nonce_known_limitation() {
    let mut app = test_app_with_project();
    app.with_focused_project(|ws, _io| {
        ws.review = Some(ReviewView {
            source: ReviewSource::Conversation("c1".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce: 6, // 已经因为切了下一个审阅目标而自增
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce: None,
        });
    });
    // 这是"为 nonce 5 发的"事件,姗姗来迟,此时 ws.review.nonce 已经是 6。
    app.update(Message::ReviewTraceWebviewEvent(ReviewTraceEvent::DocumentLoaded));
    app.with_focused_project(|ws, _io| {
        assert_eq!(
            ws.review.as_ref().unwrap().loaded_nonce,
            Some(6),
            "已知限制:事件不携带 nonce,只能追到当前值"
        );
    });
}

#[test]
fn review_trace_webview_event_no_op_when_review_is_none() {
    let mut app = test_app_with_project();
    app.with_focused_project(|ws, _io| ws.review = None);
    // 不应 panic。
    app.update(Message::ReviewTraceWebviewEvent(ReviewTraceEvent::DocumentLoaded));
}
```

> `test_app_with_project()` 若不存在,改用 `update.rs`/`workspace/tests.rs` 里既有的"构造一个带活跃项目的 App"测试辅助函数(本 crate 测试里普遍存在类似辅助函数,实现时按实际命名替换,不要凭空发明一个新的构造路径)。

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app update::tests::review_trace_webview_event
```

Expected: FAIL,`Message::ReviewTraceWebviewEvent` 未定义。

- [ ] **Step 3: 新增 Message 变体**

```rust
// crates/dozer-app/src/app/message.rs,`GitLogDiffWebviewEvent`/
// `FileHistoryDiffWebviewEvent`/`FlyfishEvent` 变体附近新增。不带
// binding——同 spec"设计"一节的理由,review-trace 不按 project_id/tab_id
// 路由。
ReviewTraceWebviewEvent(crate::preview::ReviewTraceEvent),
```

- [ ] **Step 4: update.rs 处理**

```rust
// crates/dozer-app/src/app/update.rs,`Message::GitLogDiffWebviewEvent`
// 分支附近新增
Message::ReviewTraceWebviewEvent(event) => {
    let crate::preview::ReviewTraceEvent::DocumentLoaded = event;
    self.with_focused_project(|ws, _io| {
        if let Some(rv) = &mut ws.review {
            rv.loaded_nonce = Some(rv.nonce);
        }
    });
}
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cargo test -p dozer-app update::tests::review_trace_webview_event
```

Expected: 3 个测试全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(review-trace): 新增 ReviewTraceWebviewEvent 消息与落地逻辑"
```

---

### Task 4: `review_content_pane` 原生 loading 占位

**Files:**
- Modify: `crates/dozer-app/src/workspace/view.rs`

**Interfaces:**
- Consumes: `ReviewView::loaded_nonce`(Task 2)。

- [ ] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/workspace/view.rs 的 `#[cfg(test)] mod tests`
// 块内。测试策略:`review_content_pane` 返回 `Element`,不方便直接断言
// 内部结构;抽一个纯函数 `review_pane_state(rv: Option<&ReviewView>) ->
// ReviewPaneState` 承载"该渲染哪一态"的判定(空/loading/内容三态),
// `review_content_pane` 内部调用它来分支,视图函数本身只管把结果画出来
// ——同 `conversation_visible_count`/`filter_sessions` 这类"抽纯函数供
// headless 单测"的既有写法。
#[derive(Debug, PartialEq)]
pub(crate) enum ReviewPaneState {
    Empty,
    Loading,
    Ready,
}

pub(crate) fn review_pane_state(rv: Option<&ReviewView>) -> ReviewPaneState {
    let Some(rv) = rv else {
        return ReviewPaneState::Empty;
    };
    if rv.loaded_nonce == Some(rv.nonce) {
        ReviewPaneState::Ready
    } else {
        ReviewPaneState::Loading
    }
}

#[cfg(test)]
mod review_pane_state_tests {
    use super::*;

    fn rv(nonce: u64, loaded_nonce: Option<u64>) -> ReviewView {
        ReviewView {
            source: ReviewSource::Conversation("c1".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce,
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce,
        }
    }

    #[test]
    fn none_is_empty() {
        assert_eq!(review_pane_state(None), ReviewPaneState::Empty);
    }

    #[test]
    fn unloaded_nonce_is_loading() {
        assert_eq!(review_pane_state(Some(&rv(3, None))), ReviewPaneState::Loading);
        assert_eq!(review_pane_state(Some(&rv(3, Some(2)))), ReviewPaneState::Loading);
    }

    #[test]
    fn matching_nonce_is_ready() {
        assert_eq!(review_pane_state(Some(&rv(3, Some(3)))), ReviewPaneState::Ready);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app workspace::view::review_pane_state_tests
```

Expected: FAIL,`review_pane_state`/`ReviewPaneState` 未定义。

- [ ] **Step 3: 实现纯函数 + 接入 `review_content_pane`**

```rust
// crates/dozer-app/src/workspace/view.rs,review_content_pane 函数体内,
// 把现有的:
//   if ws.review.is_some() {
//       let body = review_content(column![].spacing(region.gap), ws);
//       content = content.push(Scrollable::new(body)...);
//   } else {
//       content = content.push(container(lh(text("暂无审阅内容——...")...)));
//   }
// 改成三态分支:
match review_pane_state(ws.review.as_ref()) {
    ReviewPaneState::Empty => {
        content = content.push(
            container(lh(text("暂无审阅内容——点击左侧对话列表中的对话开始审阅")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }
    ReviewPaneState::Loading => {
        // 与 Empty 分支同结构(独占内容区、不渲染 Scrollable),只是文案
        // 换成"加载中",让 webview 隐藏期间这块区域有意义的提示而不是
        // 空的可滚动区域。复用 `byteui::feedback::math_curve` 同款加载态
        // 组件(同用量面板"统计中…"分支的写法,视觉基调统一)。
        content = content.push(
            container(byteui::feedback::math_curve::loading_hint(
                byteui::feedback::math_curve::Curve::RoseThree,
                "加载中…",
                64.0,
            ))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }
    ReviewPaneState::Ready => {
        let body = review_content(column![].spacing(region.gap), ws);
        content = content.push(
            Scrollable::new(body)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        );
    }
}
```

> `review_pane_state`/`ReviewPaneState` 定义本身放在 `#[cfg(test)]` 之外的正常模块作用域(上面 Step 1 示例把它跟测试放在一起只是为了展示,实现时把 `enum`/`fn` 挪到 `review_content_pane` 函数之前的正常位置,测试模块只保留 `#[cfg(test)] mod review_pane_state_tests`)。

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app workspace::view
```

Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace/view.rs
git commit -m "feat(review-trace): review_content_pane 增加原生 loading 占位态"
```

---

### Task 5: `runtime.rs` IPC 路由

**Files:**
- Modify: `crates/dozer-app/src/runtime.rs`

**Interfaces:**
- Consumes: `preview::parse_review_trace_event`(Task 1)、`Message::ReviewTraceWebviewEvent`(Task 3)、既有 `CONVERSATION_REVIEW_ID_OFFSET` 常量(`app.rs:630`)。

- [ ] **Step 1: 实现**(`with_ipc_handler` 闭包内 `_ => { ... }` 分支,`if let Some(binding) = flyfish_binding.as_ref() && looks_like_envelope { ... }` 之后、`else if let Some(binding) = editor_binding.as_ref() { ... }` 之前插入一个 `else if`。review-trace 至今没有专属 binding——它落在这段代码现有的兜底 `else { WebViewFocused }` 里,因为迁移前后都是"只读渲染,不发事件回 Rust";这次要在兜底之前插一段专属识别,按固定 webview id 判断,不新增 binding 类型)

```rust
} else if webview_id == crate::app::CONVERSATION_REVIEW_ID_OFFSET && looks_like_envelope {
    match crate::preview::parse_review_trace_event(body) {
        Ok(env) => {
            let _ = ipc_proxy.send_event(Message::ReviewTraceWebviewEvent(env.payload));
        }
        Err(error) => {
            tracing::warn!(%error, "无法解析 review-trace IPC");
        }
    }
} else if let Some(binding) = editor_binding.as_ref() {
```

> 插入点提醒(同 Usage 面板那份计划的教训,再强调一遍):原有代码是
> `if let Some(binding) = flyfish_binding.as_ref() && looks_like_envelope { ... }
> else if let Some(binding) = editor_binding.as_ref() { ... }
> else { ... 兜底 WebViewFocused }`——上面这段只是在第一个 `else if`
> 前面**再插一个 `else if`**,不改动前后两段既有分支的内容;`webview_id`
> 变量在这个闭包作用域内已经存在(`find_page`/`find_native` 分支已经在
> 用它),不需要额外捕获。

- [ ] **Step 2: 编译确认**

```bash
cargo check -p dozer-app
```

Expected: 通过。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/runtime.rs
git commit -m "feat(review-trace): runtime.rs 接入 loaded 事件 IPC 路由"
```

---

### Task 6: 前端 main.tsx——三种终态上报 `document_loaded`

**Files:**
- Modify: `crates/dozer-app/web/review-trace/src/main.tsx`

**Interfaces:**
- Produces: 页面达到"data 已设置"/"error 已设置"任一终态时,`window.ipc.postMessage` 报回 `{"protocol_version":1,"payload":{"kind":"document_loaded"}}`。

> 对应本计划 Review Focus 第二条:上报时机必须覆盖 fetch 成功、fetch 失败、Preact 渲染期异常(`RenderErrorBoundary` 捕获)三种情形,不能只挂在 fetch 的 `.then`/`.catch` 上——`RenderErrorBoundary` 捕获的是 `<Entry>`/`<SummaryHeader>` 渲染期抛出的异常,这条路径不经过那两个 fetch 回调。用一个 `useEffect` 挂在 `App` 组件里"data 或 error 任一个被设置"这个更上层的时机,能统一覆盖前两种;渲染异常发生在 `data` 已经被设置**之后**(拿到数据才会往下渲染 `Entry`),`useEffect([data, error])` 在 `data` 从 `null` 变为非 `null` 的那次提交仍会触发(异常发生在子组件渲染阶段,被 `RenderErrorBoundary` 挡在 `App` 自身提交完成之前,`App` 的 `useEffect` 不受影响),已经覆盖第三种情形,不需要再额外挂什么。

- [ ] **Step 1: 修改 src/main.tsx**

```tsx
import { Component, render, type ComponentChildren } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { TraceData } from './types.ts';
import { SummaryHeader } from './components/SummaryHeader.tsx';
import { Entry } from './components/Entry.tsx';
import './styles.css';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
  }
}

// review-trace 切换审阅目标时整页重新导航,期间原生 iced 显示 loading
// 占位、webview 保持隐藏(见 workspace/view.rs::review_webview_spec)。
// 页面达到任一终态(数据渲染成功 / fetch 失败 / 渲染期异常)都要报这一条,
// 让 Rust 侧切换可见——不区分成功或失败,失败态本身就是页面里可见的
// "加载失败: ..."文案,同样算"已经可以显示了"。
function reportDocumentLoaded() {
  window.ipc?.postMessage(
    JSON.stringify({ protocol_version: 1, payload: { kind: 'document_loaded' } }),
  );
}

class RenderErrorBoundary extends Component<{ children: ComponentChildren }, { error: string | null }> {
  state = { error: null as string | null };

  static getDerivedStateFromError(error: unknown) {
    return { error: String(error) };
  }

  render() {
    if (this.state.error) return <>{'加载失败: ' + this.state.error}</>;
    return this.props.children;
  }
}

function App() {
  const [data, setData] = useState<TraceData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch('dozer://review-trace/data.json')
      .then((r) => r.json())
      .then((d: TraceData) => setData(d))
      .catch((err) => setError(String(err)));
  }, []);

  // data/error 任一个从初始的 null 变为非 null,都代表这次导航已经走到
  // "可以展示内容"的终态(内容本身或错误文案),此时报 document_loaded。
  useEffect(() => {
    if (data !== null || error !== null) {
      reportDocumentLoaded();
    }
  }, [data, error]);

  if (error) return <>{'加载失败: ' + error}</>;
  if (!data) return null;

  return (
    <RenderErrorBoundary>
      <SummaryHeader data={data} />
      {data.entries.map((entry, i) => (
        <Entry entry={entry} agentLabel={data.agent_label} key={i} />
      ))}
    </RenderErrorBoundary>
  );
}

render(<App />, document.getElementById('timeline')!);
```

- [ ] **Step 2: 构建**

```bash
cd crates/dozer-app/web/review-trace
npm run typecheck
npm run build
```

Expected: 无报错,产物更新。

- [ ] **Step 3: 人工冒烟(本地起服务打开产物,控制台手动验证三种终态都触发上报)**

```bash
python3 -m http.server 8899 --directory ../../assets/review-trace
```

浏览器打开 `http://localhost:8899/host.html`(注意:本地 http 打开时
`dozer://` scheme 不可用,`fetch` 会失败——这正好顺手验证"fetch 失败"
这条路径);控制台应能看到 `window.ipc` 未定义导致 `postMessage` 调用
被 `?.` 短路、不报错,页面显示"加载失败: ..."文案。若要验证"成功"路径,
需要在真机(`cargo run -p dozer-app`)里打开一个真实审阅会话核对(见
Task 7)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/main.tsx
git commit -m "feat(review-trace): main.tsx 三种终态上报 document_loaded"
```

---

### Task 7: 全量验证 + 人工视觉核对

**Files:** 无新增/修改,纯验证。

- [ ] **Step 1: 全 workspace 构建 + 测试 + lint**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部通过。

- [ ] **Step 2: 真机人工核对(`cargo run -p dozer-app`)**

- [ ] 核心 Agent 面板:选中一个正在运行/已结束的终端会话,首次打开审阅时看到原生"加载中…"占位,随后切换到真实内容,没有白屏。
- [ ] 对话面板:点开一条历史会话的详情,同上。
- [ ] 连续快速切换 3-4 个不同的审阅目标(核心 Agent 面板或对话面板皆可),确认每次都是"加载中… → 内容",没有可感知的白屏闪烁。
- [ ] 制造一次无快照场景(如有便捷方式)确认原有"暂无审阅内容"空态文案不受影响,不会被误判成"加载中"。
- [ ] 控制台(如可开发者工具核查)零报错、零 CSP 违规。

- [ ] **Step 3: 最终 Commit(若 Step 2 发现问题已在前面任务修复,这里只是收尾确认)**

```bash
git status
# 若有未提交的小修改:
git add -A
git commit -m "fix(review-trace): 人工视觉核对后的收尾修正"
```
