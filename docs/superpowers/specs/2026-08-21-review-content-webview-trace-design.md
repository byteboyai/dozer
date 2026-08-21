# 会话审阅面板改用 wry webview 渲染 trace 效果 Design

**Status:** 已批准设计,待写实现计划。

## 背景

`crates/dozer-app/src/workspace.rs` 的 `review_content`/`review_content_pane` 是"会话"面板的右侧内容区——用户在左侧 `conversation_list_pane`(按时间倒序扁平列出的对话回合列表)点一行,通过 `Message::ConversationTurnGroupOpen` → `spawn_review_load` → `Message::ReviewLoaded` 把这一个回合区间的 `Vec<ReviewEntry>`(`transcript.rs`)加载进 `ws.review: Option<ReviewView>`,`review_content` 逐条手写 iced widget 渲染。

`ReviewEntry` 目前三种条目:`Human`(人类发言)、`AiTurn`(AI 正文 + 工具摘要 + thinking 标记,点击展开/折叠)、`ToolResult`(工具调用结果,成功用 dim 灰字、失败用红字,2026-08-21 补的摄取)。

用 spike(`spike/trace-webview/`,已验证、结论良好,详见对话记录)确认:用 wry webview + HTML/CSS/JS 渲染这条"trace 时间线"(折叠展开、分色分级)比手写 iced widget 划算得多——折叠/展开、语法高亮这些在网页里几乎是免费能力。本设计把这个方向落到 `review_content_pane` 这一个具体面板上。

**范围边界**:本设计只改"审阅内容怎么渲染"这一层。左侧 `conversation_list_pane`(列表)、`dozerd` 的摄取/分组查询层(`is_error`/`TurnGroupSummary`/`ListSessionTurnGroups`,已在 main 上线)、`Message::ConversationTurnGroupOpen`/`ReviewLoaded` 的触发链路全部不动。

## 架构

- 新增一个 wry webview,承载 `review_content_pane` 区域的全部渲染内容,替换现有的 iced `Scrollable` + 手写 `Column`。
- **实例数量**:全局唯一(同一时刻只会有一份 `ws.review` 需要展示,不是 Files/browser 那种每个 tab 一份的池)。具体用一个独立的 `Option<wry::WebView>` 字段还是照抄 `home_browser` 现有做法(在 `browser_webviews` 池里用固定 key 占一个位置)是实现阶段的选择,不影响本设计的接口——两种做法在现有代码里都有先例,写实现计划时按哪种改动更小定。
- **Bounds/可见性**:复用 `webview_geometry.rs` 现有的共享算法,新增一个函数算 `review_content_pane` 的矩形;可见性规则跟 Files/Project/browser 三个面板一致——审阅面板未展示 / 另一侧最大化 / 编辑弹窗打开 / ⌘K 面板打开时,统一"清零尺寸 + `set_visible(false)`"隐藏,不能只用遮罩盖住(wry 子视图会无视 iced 的绘制层级径直叠在最上)。
- **内容**:一份自包含的静态 HTML/CSS/JS(新文件 `crates/dozer-app/src/review_trace.html`,内容改自 `spike/trace-webview/src/trace.html` 但**去掉 tab bar**——因为一次只展示一个回合区间,不需要页面内再切换分组),`with_html` 一次性加载,暴露 `window.dozerRenderTrace(entriesJson)` 函数供 Rust 侧推数据更新。

## 数据流

1. `Message::ReviewLoaded(project_id, source, Ok(entries))` 到达,`ws.review = Some(ReviewView { entries, .. })`(既有逻辑不变)。
2. 新增一步:把 `&[ReviewEntry]` 序列化成 JSON,调用 `webview.evaluate_script("window.dozerRenderTrace(<json>)")` 推给页面重绘。`ReviewEntry` 需要补 `#[derive(Serialize)]`(目前只有 `Debug, Clone, PartialEq`)。
3. `evaluate_script` 只有在页面完成首次加载后调用才可靠——页面 `DOMContentLoaded` 后通过既有的 IPC 事件桥(`main.rs` 已经在用的 postMessage/`EventLoopProxy` 往返模式)向 Rust 侧 ping 一次"就绪",Rust 侧在收到就绪信号前把待推送的 JSON 缓存住,就绪后立即补推一次。
4. `ws.review = None`(未选中任何回合)和 `rv.error`(加载失败)两种状态**仍然走原生 iced** 渲染("暂无审阅内容"提示文字 / 红色错误文字),webview 在这两种状态下隐藏。
5. `Message::ReviewToggle(usize)` 和 `ReviewView.expanded: HashSet<usize>` **整体删除**——AI 回合的展开/折叠不再需要跟 Rust 侧同步状态,变成页面内部纯 DOM/JS(参考 spike 里的 `<details>` 元素),这是净删除,不是保留兼容。

## 组件改动清单

- `crates/dozer-app/src/transcript.rs`:`ReviewEntry` 加 `Serialize` derive。
- `crates/dozer-app/src/workspace.rs`:
  - `review_content` 函数体清空,原来 `Human`/`AiTurn`/`ToolResult` 三个分支的手写 iced 渲染整块删除。
  - `review_content_pane` 保留"无内容/出错"两个原生分支(逻辑基本不变),`ws.review.is_some()` 且无 `error` 时,不再走 `Scrollable`,改成给 webview 预留一块占位容器(bounds 计算会以这块区域为准,同 Files/Project 面板"iced 只占位、webview 叠加"的既有手法)。
  - `ReviewView.expanded` 字段删除。
- `crates/dozer-app/src/app.rs`:
  - `Message::ReviewToggle` 变体删除,`app.rs:3709` 对应的 match 分支删除。
  - `Message::ReviewLoaded` 的处理里新增"序列化 + evaluate_script"这一步(或就绪缓存逻辑)。
- `crates/dozer-app/src/webview_geometry.rs`:新增 `review_content_bounds_for(...)` 一类函数,签名/风格参照现有 `preview_content_bounds_for`/`left_files_tree_bounds_for`。
- `crates/dozer-app/src/main.rs`:新增 webview 实例的创建/销毁/可见性同步,接入现有逐帧 `sync_previews`/`sync_webview_pool` 同款例程(具体是否复用 `browser_webviews` 那个 `HashMap` 还是新开一个字段,实现阶段定)。
- 新文件 `crates/dozer-app/src/review_trace.html`。

## 错误处理

- webview 创建失败(WebKit 不可用等极端情况):记日志,面板永久落回 `review_content_pane` 的原生占位文案分支,不 panic——跟 Files/Project/browser 三个面板现有的失败处理方式一致。
- 页面就绪握手超时/丢失:不额外做超时兜底——页面是本地静态 HTML,`DOMContentLoaded` 在正常情况下是毫秒级的,握手丢失意味着 webview 创建本身就失败了,已经被上一条覆盖。

## 测试

- `ReviewEntry` → JSON 序列化的字段完整性可以直接单测(纯函数,不需要真起 wry)。
- `webview_geometry.rs` 新增的 bounds 函数沿用现有"纯函数、可单测"的模式(同 `preview_content_bounds_for` 等既有测试写法)。
- 页面内的 JS 渲染逻辑(折叠展开、分色)本身没法在 CI 里断言,跟 Files/Project/browser 三个面板一样,最终靠人工 GUI 验收确认观感和交互。

## 已知取舍(不在本设计范围内,记录下来避免以后重新讨论)

- webview 实例是复用现有 pool(固定 key)还是新开一个 `Option<WebView>` 字段:留给实现计划按改动量决定,两者行为等价。
- 是否要给 `review_trace.html` 接入 `dozer://` 自定义协议(像 `assets.rs` 那样按需加载资源):本设计判定不需要——页面完全自包含(CSS/JS 内联,不需要外部字体/图片),`include_str!` + `with_html` 足够,减一层间接。
