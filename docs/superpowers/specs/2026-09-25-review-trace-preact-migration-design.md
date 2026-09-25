# 会话审阅 Trace Host（review_trace）改造为 Preact

**状态：已批准（brainstorming 会话，2026-09-25）**

## 背景

对话面板（`extensions/conversations.rs`）和核心 Agent 面板的会话审阅功能，目前一分为二：

- **列表侧**（`extensions/conversations.rs`，809 行）：纯原生 iced，搜索框/agent 筛选/
  客户端分页/会话卡片列表。已按 [[dozer-extension-architecture-idea]] 阶段 1 模式拆出，
  本次不动。
- **详情/转录侧**（`crates/dozer-app/src/review_trace.html`，596 行）：**已经是 WebView**，
  但内容是手写 vanilla JS（无框架、无构建工具），`include_str!` 直接编译进
  `dozer-app` 二进制，运行时零构建产物。这是内核共享基础设施（见
  `2026-09-15-conversations-extension-pilot-design.md`「关键语义确认」一节）：
  `ws.review: Option<ReviewView>`、`ReviewSource::{Session, Conversation}`、
  `review_webview_spec`、`spawn_review_load_conversation` 全部留在 `workspace.rs`，
  同时服务核心 Agent 面板的终端会话审阅（`ReviewSource::Session`）和对话面板的历史
  会话审阅（`ReviewSource::Conversation`）——两个入口，一份实现。

本会话先做了一次 Preact + esbuild 的可行性 spike（Todo 面板 mock），确认这条技术路线
在本仓库「离线、无 CDN、CSP 严格、体积敏感」的硬约束下没有障碍（详见
`docs/dozer-v2/dozer-v2架构分析.md` §7.7.1）。本次设计把这条路线正式落到
`review_trace.html` 这个已有的、体量适中、edge case 已经跑了近半年的 webview host 上，
作为对话/Agent 面板 WebUI 化的第一块真实拼图。

## 目标 / 非目标

**目标**：

1. 把 `review_trace.html` 的渲染实现从「手写 vanilla JS」换成「Preact + esbuild 离线
   打包」，新建 `crates/dozer-app/web/review-trace/`，照抄 `web/editor`、
   `web/json-editor` 已验证的构建模式（iife、无 sourcemap、`jsx: 'automatic'` +
   `jsxImportSource: 'preact'`、无 CDN、无运行时 Node）。
2. `assets.rs` 的 `dozer://review-trace/` 路由从「`host.html` 硬编码
   `include_str!`」改成仿 `editor_root_for`/`serve_vendored` 的磁盘服务；
   `data.json` 端点保持现有的动态注入语义不变（`review_data: Option<&str>` 快照，
   无快照 404）。
3. 顺带给这个 host 补上 `editor`/`html_host` 都有、唯独它没有的严格 CSP
   （`default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline';
   connect-src 'self'`）——这是借重写顺手修的真实缺口，不是额外需求。
4. 视觉与交互与现状**逐像素对齐**：Human/AiTurn/ToolResult 气泡、agent 头像图标与
   配色、markdown（含 GFM 表格/代码块/行内代码/粗斜体）、trace 折叠时间线
   （thinking/tool_calls/tool_results）、token 统计摘要行，全部 1:1 迁移，不新增
   不删减。
5. Preact 组件写成**只吃 `data.json` 数据契约的纯渲染器**，不感知
   `ReviewSource::Session` 还是 `ReviewSource::Conversation`、不感知任一侧面板的私有
   状态——为将来 V2 把这类东西建模成 Artifact Store 的渲染器契约（见
   `docs/dozer-v2/dozer-v2架构分析.md` §16.4、§7.7）留一条不用返工的路，但**本次不
   实现任何插件协议/Artifact Store 代码**，只是约束这次写的代码边界。

**非目标**：

- 不改 `data.json` 的数据契约或 Rust 侧生成 `review_data` 快照的逻辑
  （`ws.review`/`ReviewView`/`ReviewSource`/`review_webview_spec`/
  `spawn_review_load_conversation` 均不动）。
- 不改列表侧（`extensions/conversations.rs` 保持原生 iced 不动）。
- 不改 webview 创建/嵌入/z-order/焦点相关 Rust 代码；`dozer://review-trace/host.html`
  这个路由地址本身、`?_r=1` 缓存失效参数、webview 生命周期均不变。
- 不新增任何新交互（复制按钮、搜索、导出等）——纯技术栈置换。
- 不现在就实现插件协议或 Artifact Store——目标 5 只约束代码组织方式，不代表本次要
  交付插件化基础设施。
- 首页「最近对话」等其他消费方（如有）不在本次范围内确认；如迁移过程中发现还有其他
  地方引用 `review_trace.html`，按「保持行为不变」原则原样跟随迁移，不额外扩展范围。

## 现状数据契约（冻结，原样保留）

`fetch("dozer://review-trace/data.json")` 返回的 JSON 形状：

```text
{
  summary_title?: string,
  summary_time?: string,
  summary_text?: string,
  agent_label?: "claude" | "codebuddy" | "opencode" | "codex" | "aider" | "goose"
              | "v8agent" | "shell",
  entries: Array<
    | { Human: { text: string } }
    | { AiTurn: {
          text?: string,
          thinking_text?: string,
          tool_calls?: Array<{ summary: string, input_json?: string }>,
          tool_results?: Array<{ content: string, is_error: bool }>,
          tokens_in?: number, tokens_out?: number,
          tokens_cache_read?: number, tokens_cache_write?: number,
        } }
    | { ToolResult: { content: string, is_error: bool } }
  >
}
```

`entries` 里每个元素是单键对象（键名即 variant tag：`Human`/`AiTurn`/`ToolResult`，对应
Rust 侧 serde 默认的 externally-tagged 枚举序列化）。`agent_label` 缺失或不在上述枚举中
时，回落到通用 bot 图标（现状 `renderEntry` 的 `AGENT_STYLE[agentLabel] || { icon:
ICONS.bot, color: null }`）。

## 架构与构建

```text
crates/dozer-app/web/review-trace/     # 新增，仿 web/editor 结构
├── package.json                       # preact 依赖 + esbuild devDependency
├── build.mjs                          # esbuild：jsx automatic/preact，iife，无 sourcemap
└── src/
    ├── host.html                      # 构建产物文件名与 dozer://review-trace/host.html
                                        # 路由保持一致（build.mjs 直接产出同名文件，不
                                        # 引入「源文件名≠路由路径」的额外心智负担）；
                                        # 补上现有 CSP 缺口后的骨架 + <div id="timeline">
    ├── main.tsx                       # fetch data.json → render(<App data={...}/>)
    ├── components/
    │   ├── SummaryHeader.tsx          # summary_title/summary_time/summary_text
    │   ├── Entry.tsx                  # 分发 Human/AiTurn/ToolResult
    │   ├── ToolCallRow.tsx            # <details> 折叠，input_json
    │   ├── ToolResultRow.tsx          # <details> 折叠，is_error 高亮，短内容默认展开
    │   └── TraceToggle.tsx            # thinking/tool_calls/tool_results 折叠时间线
    ├── markdown.ts                    # 现有 escapeHtml/renderInlineMarkdown/renderMarkdown
    │                                  # 原样迁移（无依赖手写实现，不引入 markdown 库）
    ├── agentIcons.ts                  # ICONS/AGENT_STYLE 表，原样迁移
    └── traceStats.ts                  # formatTokenCount/buildTraceStatsLabel 纯函数

crates/dozer-app/assets/review-trace/  # 构建产物提交到仓库，供 serve_vendored 服务
├── host.html
├── review-trace.js
└── review-trace.css                   # 如样式拆得出独立 CSS；否则内联在 host.html，
                                        # 与 editor 的做法二选一，构建时定
```

`assets.rs` 改动：

- 新增 `review_trace_root_for(assets_root: &Path) -> PathBuf`，仿 `editor_root_for`。
- `dozer://review-trace/` 命名空间下：
  - `data.json` 继续特判（`review_data` 注入/404），逻辑不动，且判断顺序在
    `serve_vendored` 分支之前，不受影响。
  - 其余路径（`host.html`、js、css）一律 → `serve_vendored(&review_trace_root_for(assets_root), path)`，
    不再对 `host.html` 单独 `include_str!`。
  - `__file__` 前缀（allowlist 文件读取）：review-trace 不需要，不新增。

## 组件拆分对照表

| 现有 JS 函数 | 迁移去向 |
|---|---|
| `renderSummaryHeader` | `SummaryHeader.tsx` |
| `renderEntry` | `Entry.tsx` |
| `renderToolCallRow` | `ToolCallRow.tsx` |
| `renderToolResultRow` | `ToolResultRow.tsx`（`Entry` 里 `ToolResult` 分支和 `TraceToggle` 内部两处复用同一组件，与现状一致） |
| `renderTraceToggle` | `TraceToggle.tsx` |
| `renderMarkdown`/`renderInlineMarkdown`/`escapeHtml`/`renderMarkdownInto` | `markdown.ts`，接口改成返回 Preact 节点而非拼 DOM/innerHTML，但转义规则、GFM 表格判据、代码块/列表/标题解析逻辑逐行对照原样迁移 |
| `formatTokenCount`/`buildTraceStatsLabel` | `traceStats.ts`，纯函数原样迁移，可直接搬运单测（如果之后要补） |
| `ICONS`/`AGENT_STYLE`/`renderAvatarIcon` | `agentIcons.ts` + `AvatarIcon.tsx` |

## 测试与验证

**Rust 侧**（`crates/dozer-app/src/assets.rs` 现有 4 个 `review_trace_*` 测试）：

- `review_trace_host_html_serves_embedded_page_regardless_of_review_data` → 改造成
  断言 bundle 文件存在 + 非空（仿现有 `editor_bundle_assets_are_present`/
  `json_editor_bundle_assets_are_present` 写法），不再断言「编译期内嵌」这个已经不
  成立的前提。
- `review_trace_data_json_echoes_injected_snapshot`、
  `review_trace_data_json_404_when_no_snapshot`、`review_trace_unknown_subpath_404`
  三个测的是路由/注入逻辑，与前端实现无关，原样保留。

**前端侧**（无 Rust 测试能覆盖的部分，走人工视觉核对，同本会话 Todo spike 的验证方式）：

- 准备一份脱敏后的真实 `data.json` 样本（覆盖：`summary_header` 有/无、
  `Human`、`AiTurn` 含 `text`+`thinking_text`+`tool_calls`+`tool_results`、纯
  `ToolResult`、markdown 表格/代码块/行内代码混排、`is_error` 高亮），新旧实现各跑
  一遍，浏览器里并排截图比对，控制台零报错、零 CSP 违规。
- 两个真实入口（核心 Agent 面板会话审阅、对话面板历史会话审阅）都手动打开一遍，
  确认共用同一份新实现、行为没有分叉。

## 风险与边界

- **`markdown.ts` 是这次迁移里唯一有真实复杂度的部分**：GFM 表格判据、代码块状态机、
  XSS 安全转义顺序（先整体转义、标签只由渲染函数自己生成，不解释用户/模型文本里的
  尖括号）必须逐行对照迁移，不能凭记忆重写，否则容易引入 XSS 回归或表格误判。
- **`data.json` 契约冻结是这次改造成立的前提**：如果实现过程中发现 Rust 侧
  `review_data` 生成代码有隐藏字段/边界情况没有体现在上面的契约里，先补全契约文档，
  不要边做边改契约（否则失去「纯技术栈置换、行为不变」这个验证基准）。
- **两个消费入口共享同一份实现**：改动前确认没有第三个隐藏消费方（已在「非目标」里
  注明按需跟随，不主动扩大范围）。
