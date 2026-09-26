# 用量(Usage)面板内容侧改造为 Preact WebView

**状态：已批准（brainstorming 会话，2026-09-25）**

## 背景

用量面板（`extensions/usage/`）现在是左右分栏结构：右侧 agent 筛选栏
（`view.rs::list_pane`，点击设 `agent_filter`，挂 `Divider::UsageSplit`
可拖拽分栏 + 收起/展开）+ 左侧内容侧（`view.rs::content_pane`，"内容在前、
列表在后"，与 Git Log 的"列表在前、内容在后"顺序相反——见
`layout.rs:113` 注释）。内容侧现状全部手写原生 iced/`iced_widget::canvas`：
`chart.rs` 997 行手绘网格线、趋势折线、逐点圆点，柱状图每格用嵌套
`Tooltip` widget 做 hover；`view.rs` 438 行组装五态内容（统计中/空/该
agent 无数据/单 agent 趋势/全部 agent 汇总）与汇总卡片、饼图/环图等。

本仓库已有两次 WebView 前端技术选型的验证：`review_trace.html` 已实际
完成 Preact + esbuild 离线打包改造（`docs/superpowers/specs/2026-09-25-review-trace-preact-migration-design.md`，已合并 main），Todo 列表 mock
spike 也验证过同一路线的可行性（`docs/dozer-v2/dozer-v2架构分析.md`
§7.7.1）。但 review-trace 是"打开一次会话审阅"的低频交互，用的是
"整页重载 + fetch data.json 快照"模式；用量面板的 agent 筛选栏点击是
高频交互（同一次浏览可能连续点好几个 agent），照搬 review-trace 那套
会在每次点击时整页重新导航，带来可感知的白屏闪烁（本会话讨论 review-
trace 现状时已确认这个模式确实会闪，另开 loading 遮罩改造，不在本次
范围）。本次改用「长驻单槽 webview + Rust 主动推送指令更新」模式，仿
本仓库已有的 Git Log diff pane 先例（见下）。

## 目标 / 非目标

**目标**：

1. 把用量面板**内容侧整体**（`content_pane` 现有五态中的**四态**：空/该
   agent 无数据/单 agent 趋势/全部 agent 汇总，含项目汇总卡片、Agent
   份额环图/饼图、逐日行为柱状图、趋势折线图、tooltip）从手写 iced
   canvas 换成 Preact + esbuild 离线打包，新建
   `crates/dozer-app/web/usage-content/`，照抄 `web/review-trace` 已验证
   的构建模式（iife、无 sourcemap、`jsx: 'automatic'` +
   `jsxImportSource: 'preact'`、无 CDN、无运行时 Node）。**"统计中…"这一态
   不进 webview**（写实现计划时发现的修正）：它是 `byteui::feedback::
   math_curve` 动画组件（ByteBoy2077 品牌化自绘曲线，多面板共用），不值得
   为此单独重做一套等价动画；加载中时这个内容 webview 根本不挂载，原生
   iced 继续画这个动画（同 Git Log diff pane"内容不可渲染时回落原生占位"
   的既有先例）。
2. **右侧 agent 筛选栏（`list_pane`）继续留在原生 iced 不动**——点击筛选
   仍由 iced `button` 触发 `Message::AgentFilterSet`，只是筛选结果不再
   驱动 iced 内容侧重绘，而是驱动一次到 webview 的指令推送。
3. 新增一条 Usage 专属的 Rust → webview 推送协议（轻量版，不复用
   `preview::webview_protocol::EditorCommand`/`WebviewEnvelope`——那套是
   多 tab/多 document 的编辑器模型，带 `tab_id`/`document_id`/`revision`
   等 Usage 用不上的字段；Usage 是固定单槽面板，协议按需精简），但复用
   同一条 `window.__dozer.dispatch(JSON)` 注入约定与
   `preview::webview_protocol::dispatch_script`（该函数已是"任意 JSON
   字符串 → `evaluate_script` 脚本"的通用工具，与 `EditorCommand` payload
   类型无关，可直接复用）。
4. Webview 集成仿 Git Log diff pane 先例（`app.rs:3171` 附近
   `PanelKind::GitLog` 分支、`webview_geometry::git_log_diff_pane_bounds_for`、
   `GIT_LOG_DIFF_ID_OFFSET`）：`preview_desired` 新增 `PanelKind::Usage`
   分支，固定单槽（新增 `USAGE_CONTENT_ID_OFFSET = 4_000_000`，延续现有
   1_000_000 递增序列），矩形由新增
   `webview_geometry::usage_content_pane_bounds_for` 计算（复用
   `dims.usage_split`、collapse 状态、"内容在前"的列顺序，与
   `git_log_diff_pane_bounds_for` 同构但列顺序相反）。
5. Rust 侧待推送指令走队列模式（仿 `App::take_git_log_diff_script`），
   新增 `App::take_usage_content_script`，在
   `window_events.rs::apply_pending_editor_commands` 同一节奏里
   `evaluate_script` 注入（该函数已服务 Files/Project 编辑器命令 + Git
   Log diff 命令，属于"待推送命令的统一消费点"，不新开一条独立的每帧
   轮询路径）。
6. `content_pane` 现有五态直接建模为一个 Rust enum（`UsageViewPayload`
   或类似命名，具体在实现计划阶段定名），`#[derive(Serialize)]`；
   `ConversationUsage`/`ProjectUsageTotals`/`DayAgentTotals`/`DaySeries`/
   各类 `Vec<(AgentKind, u64)>` 份额直接加 `Serialize` 复用，不新设计一套
   JSON 形状——前端只管照着 enum variant 渲染对应五态，不做任何聚合
   计算（与 review-trace"纯渲染器"原则一致）。
7. 视觉与交互**逐像素对齐**：汇总卡片、环图/饼图配色与图例、逐日柱状图、
   趋势折线、hover tooltip 内容与触发方式,全部 1:1 迁移,不新增不删减。

**非目标**：

- 不改右侧 agent 筛选栏（`list_pane`）——继续原生 iced，`Message::AgentFilterSet`
  语义不变，只是它现在除了改 `WorkspaceState.agent_filter` 之外，还要
  触发一次新指令推送。
- 不改 `aggregate.rs` 里任何聚合计算逻辑——这次只做「计算结果从喂给
  iced view 函数改成序列化推给 webview」的输送方式置换，聚合口径（含
  `agent_metric_share_drops_agents_under_one_percent` 等已有业务规则）
  一律不动。
- 不改 `Divider::UsageSplit` 拖拽/收起机制本身，只是新增的 bounds 函数
  要读它现有的值。
- 不做 review-trace 白屏闪烁的 loading 遮罩改造（另立项，见
  `[[dozer-review-trace-flicker-loading-mask-deferred]]` 记忆）。
- 不新增任何新交互（导出图表、更换配色方案等）——纯技术栈置换 + 输送
  方式改造。
- 不复用/改造 `preview::webview_protocol::EditorCommand`/`WebviewEnvelope`
  本身的结构定义,只复用其中与 payload 类型无关的 `dispatch_script`
  工具函数。

## 架构与构建

```text
crates/dozer-app/web/usage-content/      # 新增，仿 web/review-trace 结构
├── package.json                         # preact 依赖 + esbuild devDependency
├── build.mjs                            # esbuild：jsx automatic/preact，iife，无 sourcemap
└── src/
    ├── host.html                        # 构建产物文件名与 dozer://usage-content/host.html
                                          # 路由保持一致；严格 CSP（比 review-trace
                                          # 更紧，见下）+ <div id="root">
    ├── main.tsx                         # 监听 window.__dozer.dispatch 推送的
                                          # UsageViewPayload → setState → render(<App/>)
    ├── components/
    │   ├── ProjectSummaryBoxes.tsx      # 项目用量统计（含 Git 提交格）
    │   ├── AgentShareSection.tsx        # 全部 agent 态：Session/Round/IO/Cache 环图+图例
    │   ├── DailyBehaviorChart.tsx       # 逐日行为柱状图（按 agent 分组）
    │   ├── TrendChart.tsx               # 单 agent 态：Session/Token 趋势折线
    │   ├── Tooltip.tsx                  # 柱/点 hover 提示气泡
    │   └── EmptyStates.tsx              # loading/空/该 agent 无数据 三态文案
    └── format.ts                        # format_count 等纯函数原样迁移（k/m 三档
                                          # 取整规则、逐行对照 usage/mod.rs 现有实现）

crates/dozer-app/assets/usage-content/   # 构建产物提交到仓库，供 serve_vendored 服务
├── host.html
├── usage-content.js
└── usage-content.css
```

`assets.rs` 改动（仿 review-trace）：

- 新增 `usage_content_root_for(assets_root: &Path) -> PathBuf`，仿
  `review_trace_root_for`。
- `dozer://usage-content/` 命名空间：**没有 `data.json` 端点**——所有数据
  经 `evaluate_script` 推送，不走 fetch，`host.html`/js/css 一律走
  `serve_vendored(&usage_content_root_for(assets_root), path)`。因为没有
  数据获取通道，CSP 可以比 review-trace 更紧：
  `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'`
  （不需要 `connect-src`）。
- `__file__` 前缀不需要（同 review-trace）。

`webview_geometry.rs` 改动：

- 新增 `usage_content_pane_bounds_for(side, window_width, window_height,
  state: &ShellState) -> (f32, f32, f32, f32)`，结构同
  `git_log_diff_pane_bounds_for`：`collapsed || kind != PanelKind::Usage`
  时返回零矩形；否则用 `pair_content_width` + `pair_columns(pair_w,
  state.dims.usage_split, mirrored)` 算列宽，但取"内容"列（第一列，序
  与 Git Log 相反）。**不像 Git Log 那样额外判断"是否有可渲染内容"才产出
  非零矩形**——用量面板内容侧五态（含 loading/空态）都在 webview 内部
  渲染，只要 `PanelKind::Usage` 且该侧未收起，矩形就非零；agent 筛选栏
  是否显示（`ws_state.has_agent_filter()`）只影响内容列宽度是否占满
  整个面板（无筛选栏时不参与 `pair_columns` 分栏，内容独占全宽）。

`app.rs`/`window_events.rs` 改动：

- `preview_desired`（app.rs:3141 起）新增 `PanelKind::Usage` 分支，产出
  固定单槽 `WebviewSpec`（`id: USAGE_CONTENT_ID_OFFSET`，`url:
  "dozer://usage-content/host.html?theme=..."`，`visible:
  !app_modal_open`），矩形来自 `usage_content_pane_bounds_for`。
- 新增 `App::take_usage_content_script(&available_webview_ids) ->
  Vec<(usize, String)>`，仿 `take_git_log_diff_script`：`Message::Loaded`/
  `Message::AgentFilterSet` 落地时把新 `UsageViewPayload` 序列化、包一层
  `dispatch_script`，推入待发队列；`window_events.rs::apply_pending_editor_commands`
  末尾加一段消费循环（同 Git Log diff 那段的写法）。

## 组件拆分对照表

| 现有 iced 函数/类型 | 迁移去向 |
|---|---|
| `view.rs::content_pane` 五态分支 | `main.tsx` 顶层 `switch(payload.kind)` |
| `project_summary_boxes` | `ProjectSummaryBoxes.tsx` |
| 全部 agent 态环图/图例（`agent_session_share` 等四个份额 + 对应渲染） | `AgentShareSection.tsx` |
| 逐日行为柱状图（`behavior_series`/`GridLines` canvas.Program） | `DailyBehaviorChart.tsx`（SVG/CSS 画柱，替代手写 `canvas::Frame`） |
| 单 agent 态趋势折线（`session_round_trend`/`io_trend`/`cache_trend`/`TrendLines` canvas.Program） | `TrendChart.tsx`（SVG `<path>` 折线，替代手写 `canvas::Path`） |
| `day_tooltip_bubble`/`trend_tooltip_bubble`（iced `Tooltip` widget） | `Tooltip.tsx`（CSS `:hover` + 定位，行为对齐现状触发时机） |
| `format_count`（k/m 三档） | `format.ts`，纯函数原样迁移，`nice_tick_step`/`grid_ticks` 同一批迁移 |
| 三个空态文案（"统计中…"/"还没有对话记录"/"该 agent 还没有用量数据"） | `EmptyStates.tsx` |
| `ConversationUsage`/`ProjectUsageTotals`/`DayAgentTotals`/`DaySeries` | 加 `#[derive(Serialize)]`，作为 `UsageViewPayload` 内部字段直接复用，不新建 TS 对应类型之外的转换层 |

## 测试与验证

**Rust 侧**：

- 仿 review-trace 现有 `review_trace_host_html_serves_embedded_page_regardless_of_review_data`
  写法：`usage_content_host_html_serves_bundle_files`，断言 bundle 文件存在
  且非空。
- `usage_content_pane_bounds_for` 的分栏比例/collapse/镜像 geometry 单测，
  仿 `git_log_diff_pane_bounds_for` 现有测试套路（collapsed→零矩形、
  `usage_split` 极值 clamp 后的列宽换算、mirrored 时列序翻转）。
- `UsageViewPayload`（及内部各聚合类型）序列化字段快照测试，锁定字段名
  与 variant tag，防止后续悄悄改字段名导致前端解析失效而无编译期报错。
- `App::take_usage_content_script` 队列消费测试：多次 `AgentFilterSet`
  只保留最新一条待推送（不堆积陈旧指令）。仿 `take_git_log_diff_script`
  的既有结构——它把"内容/webview 是否已挂载/去重"三道判定收在
  `State::pending_diff_push` 这个纯状态里（app.rs:1050-1055），可独立
  单测；`take_usage_content_script` 应同样把去重判定收进一个纯状态方法
  （而不是散在 `take_*` 函数体内），照此结构写等价单测。

**前端侧**（无 Rust 测试能覆盖的部分，人工视觉核对，同 review-trace 的
验证方式）：

- 准备脱敏后的真实用量数据样本，覆盖：loading 态、空态（无对话记录）、
  该 agent 无数据态、单 agent 趋势态（Session/Token 两组折线）、全部
  agent 汇总态（四组环图 + 逐日柱状图），新旧实现并排截图比对。
- agent 筛选栏连续点击多个 agent，确认每次切换都是"webview 内容原地
  更新"而非整页闪烁（这是本次改造相对 review-trace 模式的核心验证点）。
- 收起/展开右侧筛选栏，确认内容侧宽度跟随 `usage_split`/collapse 状态
  正确联动，不出现 webview 矩形滞后一帧或残留旧宽度的情况。
- 控制台零报错、零 CSP 违规。

## 风险与边界

- **图表从 canvas 手绘换成 SVG/CSS 声明式渲染，不是逐行翻译**：
  `chart.rs` 里 `GridLines`/`TrendLines` 两个 `canvas::Program` 的绘制
  逻辑（网格线间距算法 `nice_tick_step`/`grid_ticks`、趋势线坐标映射）
  要先吃透产出规律再用 SVG 表达等价效果，不能像 markdown 渲染器那样
  逐行照抄——这是这次改造里最有真实复杂度的部分,视觉走查要覆盖边界
  值（数据量很小时的兜底刻度、超过 7 天/15 天窗口截断等 `aggregate.rs`
  测试里已锁定的业务规则）。
- **推送指令的"最新覆盖旧"语义要显式保证**：agent 筛选栏可能被快速
  连续点击，若队列不去重、旧指令还没来得及注入就被新指令追加，可能
  出现"webview 短暂显示上一个 agent 数据"的可感知错位；`take_usage_content_script`
  设计上应该是"覆盖式"（新指令替换队列里同槽位的旧指令）而非纯追加,
  实现计划阶段要显式确认这一点。
- **两处布局细节容易在实现时被忽略**：内容/列表顺序与 Git Log 相反
  （"内容在前"），以及"无筛选栏时内容独占全宽"这个条件（`has_agent_filter()`
  为假时不参与分栏）——`usage_content_pane_bounds_for` 若照抄
  `git_log_diff_pane_bounds_for` 但漏掉这两点会导致收起筛选栏后 webview
  矩形没跟着放大占满,是这次几何计算里最容易复制黏贴出错的地方。
- **CSP 更紧不代表没有回归风险**：现有 `chart.rs` 的 hover tooltip 依赖
  iced `Tooltip` widget 的即时定位，SVG/CSS 版本要确认触发时机（进入/
  离开判定、边界溢出时的气泡翻转方向）与现状一致，不能只做"看起来差不多"
  的近似实现。
