# review-trace 切换会话/审阅内容时的白屏闪烁遮罩

**状态：已批准（brainstorming 会话，2026-09-25～26）**

## 背景

`review_trace.html`（已于 2026-09-25 完成 Preact + esbuild 迁移，见
`docs/superpowers/specs/2026-09-25-review-trace-preact-migration-design.md`）
是一个 fetch 模型的 webview host：切换审阅目标（核心 Agent 面板选中新的
终端会话、或对话面板点开另一条历史会话）时，`ReviewView.nonce` 自增，
`review_webview_spec` 把新 nonce 写进 `dozer://review-trace/host.html?_r=<nonce>`
URL，wry 据此整页重新导航、Preact 重新 `fetch("dozer://review-trace/data.json")`。
每次导航都有一次浏览器原生的白屏（旧 DOM 卸载、新 HTML/JS 加载），本会话
讨论"能不能通过 loading 效果解决"时，用户提到 markdown 文件预览（Flyfish
host）切换文件时没有这个问题，要求比照它的做法处理（见
`[[dozer-review-trace-flicker-loading-mask-deferred]]` 记忆，当时决定另立
项处理，本 spec 即为该立项）。

读代码确认 Flyfish（以及 CodeMirror/JSON editor 三个 host）已经有一套完全
解决这个问题的既有机制，不需要发明新概念：

- `PreviewTab.backend_state`（`Loading`/`Ready`/`Failed`）+
  `PreviewPane::finish_load(tab_id, generation)`（世代校验，过期 ACK 不生效）。
- webview **预创建为 hidden**（`visible: tab.backend_state.is_ready() && idx
  == self.active`，见 `preview/view.rs:1426/1455/1503/1529`）——让它能在不可见
  状态下完成 boot/fetch，不会因为"不可见就不创建"而死锁在等不到 ready。
- JS 侧报回 `FlyfishEvent::DocumentLoaded`（`preview/webview_protocol.rs:428-450`）
  后，Rust 才 `finish_load` 把它切换可见；不可见期间原生渲染一个 loading 占位。

本次改造就是把 review-trace（单槽、非 tab 模型，`ws.review`/
`review_webview_spec`）接进同一套"hidden 直到 DocumentLoaded"模式，不新增
任何视觉设计，只是复用已经跑通两年的既有机制。

## 目标 / 非目标

**目标**：

1. review-trace 的 Preact bundle（`web/review-trace/src/main.tsx`）在
   `fetch("data.json")` 完成（无论成功还是走进现有的"加载失败: ..."错误
   文案分支——那个分支本身就是**有效的最终渲染结果**，不代表还要继续
   loading）后，`window.ipc.postMessage` 报一个 `document_loaded` 事件回
   Rust。协议形状比 `FlyfishEvent` 更简（不需要 `Title`/`SearchState`，
   review-trace 没有这些交互）：只有 `DocumentLoaded`（无字段）一个变体
   ——error 已经是页面里的可见内容，Rust 不需要单独知道"是不是失败了"
   才决定要不要显示。
2. `ReviewView` 新增一个"当前 nonce 是否已经 loaded"的最小状态（世代=
   已有的 `nonce` 字段，切审阅目标改 nonce 时天然重置未 loaded）。
3. `review_webview_spec` 的 `visible` 字段从恒 `true` 改成"当前 nonce 已
   loaded 才可见"——webview 仍然创建/导航（否则永远等不到 `DocumentLoaded`,
   死锁),只是暂不可见,同 Flyfish 现状。
4. `review_content_pane`（`workspace/view.rs`）在"有审阅内容但未 loaded"
   这个新状态下,原生渲染一个 loading 占位,不再渲染
   `Scrollable(review_content(...))`(那块区域此刻应该由原生占位独占,
   等 webview 切换可见后才把地方让出来)。
5. `runtime.rs` 的 `with_ipc_handler` 识别 review-trace 的 webview id
   (`CONVERSATION_REVIEW_ID_OFFSET`,现有常量),解析 `document_loaded`
   事件,派发新消息给 Rust。
6. 核心 Agent 面板(`ReviewSource::Session`)与对话面板
   (`ReviewSource::Conversation`)两个消费入口共用同一份 `ws.review`,
   自动都吃到这个修复,不需要分别改。

**非目标**：

- 不改 `data.json` 契约、不改 review-trace 的视觉渲染逻辑(Entry/
  markdown/trace 折叠等 Preact 组件全部不动)。
- 不引入 `PreviewTab`/`backend_state`/`finish_load` 这套 tab 模型本身
  ——review-trace 继续是单槽、非 tab,只是**照抄同一个"hidden 直到
  loaded"思路**,不做代码复用(状态形状不同,勉强复用会引入不必要的
  耦合)。
- 不做"failed 态原生重试 UI"(`FlyfishEvent::Failed`/`recoverable` 那一套)
  ——review-trace 的失败已经是页面内可见的错误文案,不需要 Rust 侧再管。
- 不消除 nonce 变化时的整页重新导航本身(那是"根治",见
  `[[dozer-review-trace-flicker-loading-mask-deferred]]` 记忆里提到的
  "换成 Git Log diff pane 式长驻单槽 + 推送指令"路线)——这次只做遮罩,
  观感从"白屏→内容"变成"loading 占位→内容"。

## 现状（冻结，原样保留的部分）

- `ReviewView`(`workspace/state.rs:177-197`):`source`/`entries`/`error`/
  `agent`/`nonce`/`summary_*` 六组字段不动,新增字段单独列在下面。
- `review_webview_spec`(`workspace/view.rs:391-414`):`rv.error.is_some()
  || rv.entries.is_empty()` 时返回空清单(销毁 webview)的逻辑不动——这是
  "根本没有审阅数据"的情形,跟本次要解决的"有数据、webview 正在加载"是
  两回事,不冲突。
- `nonce` 递增/URL 重新导航机制(`Message::ReviewLoaded` 落地新内容时)
  不动。

## 设计

### 事件协议

```rust
// preview/webview_protocol.rs 或就近的 review-trace 专属位置(实现计划
// 阶段定,两者都行——不是核心决策点)
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewTraceEvent {
    DocumentLoaded,
}
```

前端(`main.tsx`)在 `fetch` 的 `.then`/`.catch` **两条分支都会走到的收尾
处**(而不是只在成功分支)`postMessage`,保证失败态也会让 webview 切换
可见(显示页面里的错误文案,而不是永远卡在原生 loading 占位)。

### `ReviewView` 状态

```rust
pub struct ReviewView {
    // ...既有字段不动...
    /// 当前 `nonce` 对应的内容是否已经 `DocumentLoaded`——`None`/不等于
    /// `nonce` 都算"未 loaded"。换审阅目标时 `nonce` 自增,这个字段不用
    /// 显式清空也会因为不等于新 nonce 而自动回到"未 loaded"。
    pub loaded_nonce: Option<u64>,
}
```

### `review_webview_spec` 可见性

```rust
visible: rv.loaded_nonce == Some(rv.nonce),
```

其余字段(`id`/`url`/`editor_binding`/`loading_generation`)不动——webview
依然会被创建、依然会导航到带新 nonce 的 URL,只是刚创建那一刻 `visible`
为假,原生 loading 占位顶着这块区域,直到 `DocumentLoaded` 事件把
`loaded_nonce` 追上 `nonce`。

### `review_content_pane` 原生占位

`ws.review.is_some()` 分支里,再细分一层:

- `rv.loaded_nonce != Some(rv.nonce)`:渲染原生 loading 占位(复用现成的
  加载态视觉组件,不是"暂无审阅内容"那条空态文案——两者语义不同,不能
  共用同一段文案,只能共用"独占内容区、不渲染 Scrollable"的结构)。
- `rv.loaded_nonce == Some(rv.nonce)`:现状的 `Scrollable(review_content(...))`
  不变。

### IPC 路由(`runtime.rs`)

`review-trace` 现在落在 `with_ipc_handler` 的通用兜底分支(`else {
WebViewFocused }`)——它至今没有专属的 binding/事件识别,因为迁移前后都
是"只读渲染,不发事件回 Rust"。这次要新增一条按 `webview_id ==
CONVERSATION_REVIEW_ID_OFFSET` 识别的分支,解析 `document_loaded`,派发
新消息(`Message::ReviewTraceWebviewEvent` 或类似命名,实现计划阶段定)
给 `update.rs`,落地为 `ws.review` 的 `loaded_nonce = Some(rv.nonce)`。

## 数据流(一次完整的切换审阅目标)

1. 用户点开新会话 → `Message::ReviewLoaded` 落地,`nonce` 自增,
   `loaded_nonce` 因为不等于新 `nonce` 而自动变成"未 loaded"。
2. `preview_desired` 下一帧读到新 `review_webview_spec`:URL 带新 nonce,
   `visible: false`。wry 据此重新导航(webview 实例不变,同现状,只是
   URL 变了触发内部重载)。
3. `review_content_pane` 同一帧渲染原生 loading 占位(取代 Scrollable)。
4. 新页面 boot,`fetch("data.json")` 拿到新快照,渲染完(或渲染错误文案)
   → `postMessage(document_loaded)`。
5. `runtime.rs` 解析、派发消息 → `update.rs` 把 `loaded_nonce` 设成新
   `nonce`。
6. 下一帧 `review_webview_spec.visible` 变真,`review_content_pane` 切回
   `Scrollable`——原生占位与 webview 内容在同一帧完成互换,不会有一帧
   两者都不可见的"更白"的瞬间(iced 一帧内完成新占位判定,webview 的
   `visible` 也在同一帧 `preview_desired` 里翻转)。

## 测试与验证

**Rust 侧**：

- `review_webview_spec`:新增用例断言 `loaded_nonce != Some(nonce)` 时
  `visible: false`,`loaded_nonce == Some(nonce)` 时 `visible: true`;
  `rv.error.is_some()`/`entries.is_empty()` 仍然返回空清单(既有断言不
  应受影响,回归验证)。
- `Message::ReviewTraceWebviewEvent` 处理:落地后 `loaded_nonce` 正确
  更新;世代/nonce 不匹配(事件到达时 `nonce` 已经因为又切了一次目标而
  再次自增)的"过期事件"不应该把 `loaded_nonce` 错误地设成旧值——用整数
  相等比较天然规避,但要写测试锁定这一点(同 Review Focus)。
- `review_content_pane`:新状态分支(有内容但未 loaded)渲染的是占位而
  不是 `Scrollable`。

**前端侧**(人工核对,同前两次 review-trace/usage-content 迁移的验证
方式):

- 连续快速切换好几个不同的历史会话/终端会话,确认每次都是"loading 占位
  → 内容",没有可感知的白屏。
- 制造一次 `data.json` 404(无快照)场景,确认走的是"空清单直接不挂载"
  这条既有分支(不进入本次新状态机),行为与现状一致。
- 制造一次"页面能 fetch 到但内容本身导致 JS 抛异常"的情形(如果能构造),
  确认没有异常也会卡在原生 loading 占位——这是本次设计**明确不处理**的
  边界(见风险与边界),验证是为了确认这个已知限制的表现符合预期,不是
  要求修复它。

## 风险与边界

- **JS 异常导致 `document_loaded` 永远发不出去**:如果 Preact 渲染过程
  本身抛未捕获异常(不是 fetch 失败,是渲染错误),`postMessage` 调用点
  可能根本执行不到,原生 loading 占位会永久卡住,不会自动降级成错误态
  ——这是本次设计的已知限制(对应 Flyfish 的 `Failed`/超时重试没有等价
  实现,`FlyfishEvent::Failed` 那一层复杂度被明确排除在非目标之外)。
  如果这个边界在真实使用中被触发过,后续应该补一个"若干秒未收到
  `document_loaded` 就强制显示 webview(哪怕它是错的)"的超时兜底,而不是
  现在就加——YAGNI,先看会不会真的发生。
- **`loaded_nonce` 的过期事件判定完全依赖整数相等,不做时间戳/序号校验**
  ——`nonce` 本身已经是单调递增的唯一标识,足够;但要在测试里显式验证
  "旧事件到达时 nonce 已经变了"这个时序,不能假设消息队列一定按发出顺序
  送达导致巧合正确。
- **两个消费入口(核心 Agent 面板/对话面板)共用同一份状态机**:改的是
  `ws.review` 这个共享字段,理论上两个入口的行为应该完全一致;实现计划
  执行完后建议都手动点一遍确认,不能只测一个入口就认为两个都好了。
