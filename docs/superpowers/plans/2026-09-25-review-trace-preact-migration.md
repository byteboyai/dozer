# 会话审阅 Trace Host（review_trace）改造为 Preact Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/review_trace.html`（596 行手写 vanilla JS、`include_str!` 编译进二进制）换成 Preact + esbuild 离线打包，视觉与交互逐像素对齐，`data.json` 数据契约冻结不变。

**Architecture:** 新增 `crates/dozer-app/web/review-trace/`（Preact + esbuild，照抄 `web/editor` 的离线打包模式），产物提交到 `crates/dozer-app/assets/review-trace/`；`assets.rs` 的 `dozer://review-trace/` 路由从 `include_str!` 硬编码改成仿 `editor_root_for`/`serve_vendored` 的磁盘服务，`data.json` 动态注入逻辑不变。前端按现有 JS 函数 1:1 拆成 Preact 组件 + 纯函数模块，逐层 TDD 验证。

**Tech Stack:** Preact 10.29.8、esbuild 0.28.2（jsx automatic runtime）、TypeScript 5.9.3、`preact-render-to-string` 6.7.0（仅测试用，验证纯函数渲染输出）、Node 内置 `node:test`（与 `web/editor` 现有 `protocol.test.ts` 同一套跑法，无需额外测试框架）。

**Spec:** `docs/superpowers/specs/2026-09-25-review-trace-preact-migration-design.md`

## Global Constraints

- 数据契约冻结：不改 `data.json` 的 JSON 形状，也不改 Rust 侧生成 `review_data` 快照的逻辑（`ws.review`/`ReviewView`/`ReviewSource`/`review_webview_spec`/`spawn_review_load_conversation` 一律不动）。
- 不改列表侧：`extensions/conversations.rs` 保持原生 iced 不动。
- 不改 webview 创建/嵌入/z-order/焦点相关 Rust 代码；路由地址 `dozer://review-trace/host.html` 和 `?_r=1` 缓存失效参数原样保留。
- 不新增任何新交互（复制、搜索、导出等）——纯技术栈置换。
- Preact 组件只吃 `data.json` 契约，不感知 `ReviewSource::Session`/`ReviewSource::Conversation`、不实现任何插件协议或 Artifact Store 代码。
- CSP（新补，`editor`/`html_host` 都有、这个 host 目前没有）：`default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'`。
- 离线打包：无 CDN、无运行时 Node 依赖，esbuild `format:'iife'`、`sourcemap:false`、`legalComments:'none'`，与 `web/editor`/`web/json-editor` 同构。
- 视觉与交互与现状逐像素对齐：气泡样式、agent 头像/配色、markdown 渲染规则（含 GFM 表格判据、XSS 转义顺序）、trace 折叠时间线、token 统计口径，全部照抄不新增不删减。

## Review Focus

- **XSS：** 模型/工具输出里出现 `<script>`、`onerror=` 等文本不得被当成真实标签解释——Task 4 的 `markdown.test.ts` 覆盖转义路径，Task 11 再用一条含尖括号的假工具输出人工复核一遍。
- **缺省字段路径：** `summary_text` 缺失时不渲染摘要头和分割线；`agent_label` 缺失/不在已知枚举时回落通用 bot 图标；`thinking_text`/`tool_calls`/`tool_results` 全缺失时 trace 折叠区整体不渲染——Task 7、Task 8、Task 11 要覆盖这些"没数据"的路径，不能只验证"有数据"的正向情形。
- **`data.json` 404（无快照）时的错误态：** 原实现 `.catch` 分支展示"加载失败: ..."文本，不能让页面在无快照时白屏——Task 8 main.tsx 必须保留这个分支。
- **超长 `tool_results.content` 的默认折叠：** 原实现按 `content.length < 200` 决定默认展开/折叠，避免长日志撑爆页面——Task 5 迁移时要带上这个判据，Task 11 用一条超过 200 字符的结果人工验证折叠状态没丢。
- **表格误判：** 纯文本里偶然出现单个 `|`（不构成 GFM 分隔行）不应被误判成表格——Task 4 已写对应测试（`a | b` 走段落分支），在此列出是为了标明这是"已知风险点、已测试"而不是遗漏项。

---

## 文件结构总览

```text
crates/dozer-app/web/review-trace/       # 新增
├── package.json
├── tsconfig.json
├── build.mjs
└── src/
    ├── types.ts                         # TraceData/TraceEntry/ToolCall/ToolResult/AiTurnData/AgentLabel
    ├── agentIcons.ts                    # ICONS/AGENT_STYLE
    ├── traceStats.ts                    # formatTokenCount/buildTraceStatsLabel
    ├── traceStats.test.ts
    ├── markdown.ts                      # escapeHtml/renderInlineMarkdown/renderMarkdown
    ├── markdown.test.ts
    ├── host.html                        # 构建产物文件名与路由 host.html 对齐；补 CSP
    ├── styles.css                       # 从 review_trace.html <style> 块原样迁移
    ├── main.tsx                         # fetch data.json → render(<App/>)
    └── components/
        ├── AvatarIcon.tsx
        ├── ToolCallRow.tsx
        ├── ToolResultRow.tsx
        ├── TraceToggle.tsx
        ├── SummaryHeader.tsx
        └── Entry.tsx

crates/dozer-app/assets/review-trace/    # 新增，构建产物，提交到仓库
├── host.html
├── review-trace.js
└── review-trace.css

crates/dozer-app/src/assets.rs           # 修改：review-trace 路由从 include_str! 改磁盘服务
crates/dozer-app/src/review_trace.html   # 删除（内容迁到 web/review-trace/src/host.html + styles.css）
```

---

### Task 1: 脚手架 + 静态骨架（package.json / build.mjs / host.html / styles.css）

**Files:**
- Create: `crates/dozer-app/web/review-trace/package.json`
- Create: `crates/dozer-app/web/review-trace/tsconfig.json`
- Create: `crates/dozer-app/web/review-trace/build.mjs`
- Create: `crates/dozer-app/web/review-trace/src/types.ts`
- Create: `crates/dozer-app/web/review-trace/src/host.html`
- Create: `crates/dozer-app/web/review-trace/src/styles.css`
- Create: `crates/dozer-app/web/review-trace/src/main.tsx`（占位版，Task 8 替换成真实 fetch 逻辑）

**Interfaces:**
- Produces: `types.ts` 导出 `AgentLabel`、`ToolCall`、`ToolResult`、`AiTurnData`、`TraceEntry`、`TraceData`，供全部后续任务使用。
- Produces: 构建脚本产出 `crates/dozer-app/assets/review-trace/{host.html,review-trace.js,review-trace.css}`。

- [x] **Step 1: 创建 package.json**

```json
{
  "name": "dozer-review-trace",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Dozer 会话审阅 trace host（review_trace），Preact 离线打包，无 CDN/无运行时 Node。",
  "scripts": {
    "build": "node build.mjs",
    "typecheck": "tsc --noEmit",
    "test": "node --test src/*.test.ts"
  },
  "dependencies": {
    "preact": "10.29.8"
  },
  "devDependencies": {
    "@types/node": "^24",
    "esbuild": "0.28.2",
    "preact-render-to-string": "6.7.0",
    "typescript": "5.9.3"
  }
}
```

- [x] **Step 2: 创建 tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noEmit": true,
    "skipLibCheck": true,
    "allowImportingTsExtensions": true,
    "jsx": "react-jsx",
    "jsxImportSource": "preact",
    "types": ["node"],
    "lib": ["ES2020", "DOM", "DOM.Iterable"]
  },
  "include": ["src/**/*.ts", "src/**/*.tsx"]
}
```

- [x] **Step 3: 创建 build.mjs**

```js
// 生产构建:把会话审阅 trace host 打包成**离线、无 CDN、无运行时 Node** 的
// 确定性产物到 `crates/dozer-app/assets/review-trace/`。
//
// 产物:`review-trace.js`(iife bundle)、`review-trace.css`(从
// `main.tsx` 顶部 `import './styles.css'` 抽出的样式)、`host.html`(带
// 严格 CSP,原样拷贝)。
//
// 不输出 source map;minify 后去掉所有 legal comments。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/review-trace');

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
  outfile: path.join(outdir, 'review-trace.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
```

- [x] **Step 4: 创建 src/types.ts**

```ts
export type AgentLabel =
  | 'claude'
  | 'codebuddy'
  | 'opencode'
  | 'codex'
  | 'aider'
  | 'goose'
  | 'v8agent'
  | 'shell';

export interface ToolCall {
  summary: string;
  input_json?: string;
}

export interface ToolResult {
  content: string;
  is_error: boolean;
}

export interface AiTurnData {
  text?: string;
  thinking_text?: string;
  tool_calls?: ToolCall[];
  tool_results?: ToolResult[];
  tokens_in?: number;
  tokens_out?: number;
  tokens_cache_read?: number;
  tokens_cache_write?: number;
}

export type TraceEntry =
  | { Human: { text: string } }
  | { AiTurn: AiTurnData }
  | { ToolResult: ToolResult };

export interface TraceData {
  summary_title?: string;
  summary_time?: string;
  summary_text?: string;
  agent_label?: AgentLabel;
  entries: TraceEntry[];
}
```

- [x] **Step 5: 创建 src/styles.css**（从 `review_trace.html` 的 `<style>` 块原样迁移，一字不改）

```css
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
.summary-header { padding: 4px 0 8px; }
.summary-header .summary-title { font-size: 14px; font-weight: 600; color: var(--cream); margin-bottom: 4px; }
.summary-header .summary-time { font-size: 11px; color: var(--dim); margin-bottom: 8px; }
.summary-header .summary-body { color: var(--dim); }
.topic-divider {
  height: 1px; background: var(--line); border: none; margin: 14px 0;
}

.msg { margin-bottom: 20px; max-width: 100%; }
.msg-head {
  display: flex; align-items: center; gap: 6px;
  font-size: 11px; letter-spacing: .02em;
  margin-bottom: 6px;
}
.avatar-dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
.avatar-icon { width: 16px; height: 16px; flex: none; display: inline-flex; }
.avatar-icon svg { display: block; width: 100%; height: 100%; }

.msg.Human { margin-left: 12%; }
.msg.Human .msg-head { color: var(--gold); justify-content: flex-end; }
.msg.Human .avatar-dot { background: var(--gold); }
.msg.Human .bubble { border-color: var(--gold); }

.msg.AiTurn { margin: 0; }
.msg.AiTurn .msg-head { color: var(--cyan); justify-content: flex-start; }
.msg.AiTurn .avatar-dot { background: var(--cyan); }
.msg.AiTurn .bubble { border-color: var(--cyan); }

.msg.ToolResult .msg-head { color: var(--green); }
.msg.ToolResult .avatar-dot { background: var(--green); }
.msg.ToolResult .bubble { border-color: var(--green); }
.msg.ToolResult.error .msg-head { color: var(--red); }
.msg.ToolResult.error .avatar-dot { background: var(--red); }
.msg.ToolResult.error .bubble { border-color: var(--red); }

.bubble {
  border: 1px solid var(--line); border-radius: 14px; padding: 14px 16px;
}
.msg.AiTurn .bubble .text { color: #9AB4C4; }
.msg.ToolResult .bubble .text {
  color: var(--dim); font-family: ui-monospace, monospace; font-size: 12px;
  white-space: pre-wrap;
}
.msg.ToolResult.error .bubble .text { color: var(--red); }

.md-p { white-space: pre-wrap; margin: 0 0 10px; }
.md-p:last-child { margin-bottom: 0; }
.md-h { font-weight: 700; margin: 12px 0 6px; }
.md-h:first-child { margin-top: 0; }
.md-h1 { font-size: 16px; }
.md-h2 { font-size: 15px; }
.md-h3, .md-h4, .md-h5, .md-h6 { font-size: 14px; }
.md-list { margin: 0 0 10px; padding-left: 20px; }
.md-list:last-child { margin-bottom: 0; }
.md-list li { margin-bottom: 3px; }
.md-table { margin: 0 0 10px; border-collapse: collapse; max-width: 100%; font-size: 12px; }
.md-table:last-child { margin-bottom: 0; }
.md-table th, .md-table td {
  border: 1px solid var(--line); padding: 5px 10px; text-align: left; vertical-align: top;
}
.md-table th { background: #10151f; font-weight: 600; color: var(--cream); }
.md-table td { color: var(--cream); overflow-wrap: anywhere; }
.md-code-block {
  margin: 0 0 10px; padding: 8px 10px; background: #10151f;
  border: 1px solid var(--line); border-radius: 6px; overflow-x: auto;
}
.md-code-block:last-child { margin-bottom: 0; }
.md-code-block code {
  background: none; border: none; padding: 0; font-size: 12px; white-space: pre;
}
code {
  font-family: ui-monospace, monospace; font-size: 12px;
  background: #10151f; border: 1px solid var(--line); border-radius: 3px;
  padding: 1px 4px;
}

details.trace-toggle {
  margin-top: 10px; padding-top: 10px; border-top: 1px solid var(--line);
}
details.trace-toggle:first-child { margin-top: 0; padding-top: 0; border-top: none; }
details.trace-toggle > summary {
  cursor: pointer; color: var(--dim); font-size: 11px; list-style: none;
}
details.trace-toggle > summary::-webkit-details-marker { display: none; }
details.trace-toggle > summary::before { content: "轨迹 ▸ "; }
details.trace-toggle[open] > summary::before { content: "轨迹 ▾ "; }
details.trace-toggle > summary .trace-stats { margin-left: 6px; opacity: .75; }
.trace-timeline { position: relative; margin-top: 8px; padding-left: 16px; }
.trace-timeline::before {
  content: ""; position: absolute; left: 4px; top: 4px; bottom: 4px;
  width: 1px; background: var(--line);
}
.trace-item { position: relative; margin-bottom: 8px; }
.trace-item:last-child { margin-bottom: 0; }
.trace-item::before {
  content: ""; position: absolute; left: -15px; top: 4px;
  width: 7px; height: 7px; border-radius: 50%; background: var(--dim);
}
.trace-item.thinking::before { background: var(--dim); }
.trace-item.tool-call::before { background: var(--cyan); }
.trace-item.tool-result::before { background: var(--green); }
.trace-item.tool-result.error::before { background: var(--red); }
.trace-item .item-title {
  font-size: 11px; color: var(--dim); margin-bottom: 4px;
}
.trace-item .thinking-text { font-size: 12px; color: var(--dim); }
.tool-call-row summary {
  cursor: pointer; color: var(--cyan); font-size: 11px;
  background: #10151f; border: 1px solid var(--line); border-radius: 4px;
  padding: 2px 8px; display: inline-block;
}
.tool-call-row .input-json {
  white-space: pre-wrap; font-family: ui-monospace, monospace; font-size: 11px;
  color: var(--dim); margin-top: 4px; padding: 8px 10px;
  background: #10151f; border: 1px solid var(--line); border-radius: 6px;
  overflow-x: auto;
}
.tool-result-row summary {
  cursor: pointer; color: var(--dim); font-size: 11px;
}
.tool-result-row.error summary { color: var(--red); }
.tool-result-row .body {
  white-space: pre-wrap; font-family: ui-monospace, monospace; font-size: 12px;
  color: var(--dim); margin-top: 4px; padding: 8px 10px;
  background: #10151f; border: 1px solid var(--line); border-radius: 6px;
  overflow-x: auto;
}
.tool-result-row.error .body { color: var(--red); }
```

- [x] **Step 6: 创建 src/host.html**（原 `review_trace.html` 没有 CSP，这里补上）

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8" />
<meta
  http-equiv="Content-Security-Policy"
  content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'"
/>
<title>trace</title>
<link rel="stylesheet" href="review-trace.css" />
</head>
<body>
<div id="timeline"></div>
<script src="review-trace.js"></script>
</body>
</html>
```

- [x] **Step 7: 创建占位版 src/main.tsx**（证明构建管线打通；Task 8 换成真实 fetch 逻辑）

```tsx
import { render } from 'preact';
import './styles.css';

render(<div>加载中…</div>, document.getElementById('timeline')!);
```

- [x] **Step 8: 安装依赖并构建，验证产物**

```bash
cd crates/dozer-app/web/review-trace
npm install
npm run build
```

Expected: `esbuild` 输出 `review-trace.js` 体积日志，无报错；随后检查产物：

```bash
ls -la ../../assets/review-trace
grep -c "default-src 'none'" ../../assets/review-trace/host.html
```

Expected: `host.html`、`review-trace.js`、`review-trace.css` 三个文件都存在且非空；`grep` 命中 1。

- [x] **Step 9: Commit**

```bash
git add crates/dozer-app/web/review-trace crates/dozer-app/assets/review-trace
git commit -m "feat(review-trace): 脚手架 Preact + esbuild 离线打包管线"
```

---

### Task 2: agentIcons.ts（agent 图标与配色表）

**Files:**
- Create: `crates/dozer-app/web/review-trace/src/agentIcons.ts`
- Test: `crates/dozer-app/web/review-trace/src/agentIcons.test.ts`

**Interfaces:**
- Consumes: `AgentLabel` from `./types.ts`（Task 1）。
- Produces: `ICONS: Record<'user'|'bot'|'claude'|'codebuddy'|'opencode'|'codex'|'aider'|'gooseDark', string>`、`AGENT_STYLE: Record<AgentLabel, { icon: string; color: string | null }>`，供 Task 5（`AvatarIcon`）、Task 7（`Entry`）使用。

- [x] **Step 1: 写失败的测试**

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { ICONS, AGENT_STYLE } from './agentIcons.ts';

test('AGENT_STYLE covers all eight known agent labels with non-empty icon markup', () => {
  const labels = ['claude', 'codebuddy', 'opencode', 'codex', 'aider', 'goose', 'v8agent', 'shell'] as const;
  for (const label of labels) {
    const style = AGENT_STYLE[label];
    assert.ok(style, `missing AGENT_STYLE for ${label}`);
    assert.ok(style.icon.startsWith('<svg'), `${label} icon must be an <svg> string`);
  }
});

test('claude/codebuddy/opencode use a theme color; codex/aider/goose keep their own brand colors (color: null)', () => {
  assert.equal(AGENT_STYLE.claude.color, '#47DEF0');
  assert.equal(AGENT_STYLE.codebuddy.color, '#9580FF');
  assert.equal(AGENT_STYLE.opencode.color, '#1AD585');
  assert.equal(AGENT_STYLE.codex.color, null);
  assert.equal(AGENT_STYLE.aider.color, null);
  assert.equal(AGENT_STYLE.goose.color, null);
});

test('v8agent and shell fall back to the generic bot icon', () => {
  assert.equal(AGENT_STYLE.v8agent.icon, ICONS.bot);
  assert.equal(AGENT_STYLE.shell.icon, ICONS.bot);
});

test('ICONS.user is a distinct icon from ICONS.bot', () => {
  assert.notEqual(ICONS.user, ICONS.bot);
  assert.ok(ICONS.user.startsWith('<svg'));
});
```

- [x] **Step 2: 运行测试确认失败**

```bash
cd crates/dozer-app/web/review-trace
node --test src/agentIcons.test.ts
```

Expected: FAIL，`Cannot find module './agentIcons.ts'`。

- [x] **Step 3: 创建 src/agentIcons.ts**（SVG 字符串与配色表从 `review_trace.html` 逐字迁移）

```ts
import type { AgentLabel } from './types.ts';

// lucide 通用图标(circle-user-round / bot)走 currentColor,跟随
// `.msg-head` 的角色配色。claude/codebuddy/opencode 是 currentColor 单色
// 路径,通过 AGENT_STYLE 指定主题色;codex/aider/goose 使用原始品牌色
// SVG(同 `crates/byteui/assets/icons/*.svg`),不额外上色。映射逻辑与
// `workspace.rs::agent_icon` 保持一致。
export const ICONS = {
  user: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.925 20.056a6 6 0 0 0-11.851.001"/><circle cx="12" cy="11" r="4"/><circle cx="12" cy="12" r="10"/></svg>',
  bot: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/></svg>',
  claude: '<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="currentColor" d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"/></svg>',
  codebuddy: '<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="currentColor" d="M18.636.289a1 1 0 0 0-.11 0c-.18.01-.195.02-.442.24-.716.636-1.722 2.546-2.703 5.137l-.274.72-.499.16c-1.554.498-2.934 1.128-4.157 1.893-1.174.73-1.81 1.207-2.768 2.056l-.578.51-.262-.045c-2.528-.447-4.8-.612-5.843-.43-.414.077-.757.216-.862.35-.092.12-.138.263-.138.474 0 .182.034.414.098.727.265 1.236.952 2.854 2.035 4.78l.7 1.236-.023.466c-.027.499 0 1.27.06 1.793.036.319.031.327-.135.516-.565.647-.708 1.676-.408 2.84h5.364l-.33-.57c-.64-1.108-.96-1.663-1.134-2.177a5.46 5.46 0 0 1 1.564-5.84c.408-.358.962-.678 2.072-1.32l6.38-3.683c1.11-.64 1.665-.96 2.18-1.134A5.46 5.46 0 0 1 24 10.275V6.462l-.117-.06-.504-.25-.357-.662c-.924-1.702-2.41-3.696-3.477-4.666-.4-.364-.655-.517-.91-.535M11.57 17.634a1.26 1.26 0 0 1 1.722.462l1.358 2.35c.842 1.455-1.341 2.717-2.183 1.262l-1.358-2.352a1.26 1.26 0 0 1 .461-1.722m6.802-3.926a1.26 1.26 0 0 1 1.721.46l1.358 2.352c.84 1.455-1.343 2.715-2.183 1.26l-1.358-2.35a1.26 1.26 0 0 1 .462-1.722"/></svg>',
  opencode: '<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="currentColor" fill-rule="evenodd" d="M18 19.5H6V4.5H18V19.5ZM15 16.5H9V7.5H15V16.5Z"/></svg>',
  codex: '<svg version="1.2" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 250 250" width="250" height="250"><defs><linearGradient id="codex-P" gradientUnits="userSpaceOnUse"/><linearGradient id="codex-g1" x2="1" href="#codex-P" gradientTransform="matrix(0,249.335,-249.128,0,125,.332)"><stop stop-color="#b1a7ff"/><stop offset=".5" stop-color="#7a9dff"/><stop offset="1" stop-color="#3941ff"/></linearGradient></defs><path fill="url(#codex-g1)" d="m84.3 5.1q3.7-1.5 7.7-2.6 3.9-1 7.9-1.6 4-0.5 8.1-0.6 4 0 8 0.5 20.7 2.4 37.1 17.7 0.1 0.1 0.4 0.3 0.1 0 0.2 0 0 0 0.2 0 0 0 0.1 0 0 0 0.1 0 5.2-1.4 10.7-1.9 5.4-0.4 10.7 0.1 5.5 0.4 10.7 1.9 5.2 1.3 10.1 3.6l0.6 0.4 1.6 0.8q5.2 2.5 9.7 6.1 4.7 3.4 8.6 7.7 3.8 4.3 6.9 9.2 3 4.8 5.2 10.2 4.3 10.5 4.3 22.1 0.2 2.1 0 4.2-0.1 2.2-0.2 4.3-0.3 2.1-0.7 4.3-0.4 2.1-0.9 4.1 0 0.2 0 0.4 0 0.2 0 0.5 0 0.1 0.1 0.4 0.1 0.1 0.3 0.3 12.3 12.6 16.3 30 6 29.7-12.2 53.5l-1.9 2.2q-3 3.5-6.5 6.4-3.4 3.1-7.3 5.5-3.8 2.4-8.1 4.2-4.1 1.9-8.5 3.2-0.3 0-0.4 0.2-0.3 0-0.4 0.1-0.1 0.1-0.3 0.4 0 0.1-0.1 0.3c-2.7 7.7-5.3 14.2-10.2 20.7-12.5 16.5-30.8 25.5-51.5 25.5q-24.6-0.1-43.6-18.1-0.2-0.1-0.4-0.2-0.2-0.1-0.4-0.1-0.2 0-0.3 0-0.3 0-0.4 0c-5.4 1.7-10.9 1.9-16.7 1.9q-3.5 0-7-0.5-3.4-0.4-6.9-1.2-3.3-0.8-6.6-2-3.3-1.2-6.4-2.8-3.3-1.6-6.4-3.6-3-2-5.8-4.3-3-2.3-5.5-5-2.5-2.6-4.6-5.6c-2.2-2.7-4.3-5.4-5.8-8.5q-0.8-1.6-1.6-3.2-0.6-1.7-1.3-3.3-0.7-1.7-1.2-3.4-0.5-1.6-1-3.4-1.1-4-1.6-7.9-0.6-4-0.6-8 0-4 0.6-8 0.4-4 1.4-8 0 0 0-0.1 0-0.1 0-0.1 0.2-0.2 0.2-0.3 0-0.1-0.2-0.1 0-0.2 0-0.3 0-0.1-0.1-0.1 0-0.2 0-0.2-0.1-0.1-0.1-0.1-2.4-2.5-4.6-5.2-2.1-2.7-4-5.4-1.7-3-3.2-6-1.5-3.1-2.6-6.3-0.8-2-1.3-4.1-0.7-2-1.1-4-0.4-2.1-0.7-4.2-0.2-2.2-0.4-4.3-0.2-2.8-0.1-5.6 0-2.8 0.3-5.4 0.1-2.8 0.6-5.6 0.4-2.8 1.1-5.5 7-23.1 26.9-36.3 4.3-2.9 8.2-4.5 4.5-1.9 9-3.2 0.2 0 0.3-0.1 0.1-0.2 0.3-0.3 0.1 0 0.1-0.3 0.1-0.1 0.1-0.2 1-3.1 2.2-6 1-2.9 2.5-5.7 1.5-3 3.2-5.6 1.7-2.7 3.7-5.1 2.5-3.2 5.3-5.9 3-2.8 6.1-5.4 3.2-2.4 6.8-4.4 3.5-2 7.2-3.5zm48.3 146.4c-2.3 0.1-4.4 1-6 2.8-1.5 1.6-2.4 3.7-2.4 5.9 0 2.3 0.9 4.4 2.4 6.2 1.6 1.6 3.7 2.5 6 2.6h50.4c2.4 0.1 4.8-0.6 6.5-2.4 1.7-1.6 2.8-4 2.8-6.4 0-2.4-1.1-4.7-2.8-6.3-1.7-1.8-4.1-2.6-6.5-2.4zm-56.7-64.9c-1.2-1.9-3-3.4-5.3-3.9-2.2-0.5-4.5-0.3-6.5 0.9-2 1.1-3.5 3-4.1 5.2-0.7 2.2-0.4 4.6 0.6 6.5l17.7 30.9-17.5 29.5c-1.2 2-1.6 4.5-1.1 6.8 0.7 2.3 2.1 4.1 4.1 5.3 2 1.2 4.4 1.6 6.7 0.9 2.2-0.5 4.2-1.9 5.4-3.9l20.1-34.1q0.7-0.9 0.9-2.1 0.3-1.1 0.3-2.3 0-1.2-0.3-2.2-0.2-1.2-0.8-2.2z"/></svg>',
  aider: '<svg role="img" viewBox="75 8 50 50" xmlns="http://www.w3.org/2000/svg"> <title>Aider</title> <defs> <filter id="aider-glow" x="-40%" y="-30%" width="180%" height="160%"> <feGaussianBlur stdDeviation="7 1" result="blur"/> <feColorMatrix in="blur" type="matrix" values="1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 .5 0" result="lighter-blur"/> <feComposite in="SourceGraphic" in2="lighter-blur" operator="over"/> </filter> </defs> <path fill="#14b014" filter="url(#aider-glow)" d="m112.583 52.464h-4.541q-.557-1.846-.557-4.922-3.896 5.713-10.605 5.713-3.399 0-6.006-2.403-2.578-2.431-2.578-6.24 0-2.256.908-3.984.909-1.729 2.666-2.901 1.758-1.201 4.277-1.875 2.52-.703 5.977-.967l5.127-.41v-1.464q0-7.852-7.324-7.852-1.729 0-4.395.762-2.636.732-4.541 1.758l-1.201-3.282q6.24-3.193 11.602-3.193 10.429 0 10.429 11.045v13.125q0 3.984.762 7.09zm-5.332-9.053v-5.537q-6.182.557-8.496 1.055-2.285.469-3.985 1.787-1.67 1.318-1.67 3.574 0 2.051 1.436 3.31 1.436 1.26 3.662 1.26 2.432 0 4.834-1.435 2.402-1.465 4.219-4.014z"/> </svg>',
  gooseDark: '<svg role="img" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg"><title>Goose</title><g transform="translate(0 512) scale(.1 -.1)" fill="#fefefe"><path d="M1043 4212c-16-32-44-104-63-160l-33-101 28-41c16-22 84-95 153-161 68-66 122-123 118-126-8-8-95 22-213 73-56 24-106 44-111 44-17 0-22-42-22-192 0-117 3-150 15-162 19-18 134-63 292-113 68-21 123-41 123-44 0-11-48-18-218-31l-163-12 5-32c14-72 126-311 151-320 7-3 60 0 117 6 198 22 208 23 208 13 0-13-59-52-136-91-32-16-70-39-84-50l-24-20 27-39c15-22 89-105 165-184 75-80 154-164 174-187 21-23 42-42 46-42 4 0 24 16 43 36 83 87 289 236 492 355 53 32 97 57 97 55 0-1-48-43-107-92-60-48-198-179-308-289-221-220-302-324-384-490-73-147-241-572-241-609 0-40 35-76 74-76 28 0 442 160 588 228 45 21 126 68 179 104 126 86 150 98 202 98 56 0 74-14 260-202 142-145 238-228 261-228 7 0 37 45 67 100 29 55 64 110 76 122l23 21v-27c0-15-7-82-16-148-8-67-13-132-9-145 4-19 28-35 108-73 121-57 233-97 243-87 4 4 11 74 15 155 7 152 16 222 28 222 4 0 24-53 44-119 46-149 95-278 112-295 19-19 144-27 258-17 54 5 101 11 103 14 3 3-10 36-30 74-41 82-101 251-92 260 3 3 34-25 68-63 73-81 146-152 207-202l43-36 128 43c140 47 212 84 204 105-3 7-73 69-157 136-268 218-405 365-533 573-155 252-370 464-621 614-106 63-134 96-141 162-4 44-1 51 49 118 76 100 371 414 449 477 36 29 107 81 158 116 138 94 222 204 222 293 0 32-8 46-51 93l-52 54 34-6c19-4 54-15 78-25 25-11 48-16 51-12 24 29 92 148 96 171 8 37-24 67-62 58-15-4-75-37-133-74-58-36-129-74-157-83-28-9-68-26-90-37-53-27-146-127-226-242-89-128-134-177-333-359-244-222-270-241-326-241-67 0-103 29-167 135-156 262-320 429-612 623-268 179-336 242-585 545-66 81-128 147-137 147-10 0-27-22-45-58z"/></g></svg>',
};

// AgentKind::label() → {icon, color}。claude/codebuddy/opencode 使用
// currentColor 单色路径并按主题色上色;codex/aider/goose 使用原始品牌色
// SVG,颜色传 null 避免被覆盖。未知 agent 回落通用 bot 图标。
export const AGENT_STYLE: Record<AgentLabel, { icon: string; color: string | null }> = {
  claude: { icon: ICONS.claude, color: '#47DEF0' },
  codebuddy: { icon: ICONS.codebuddy, color: '#9580FF' },
  opencode: { icon: ICONS.opencode, color: '#1AD585' },
  codex: { icon: ICONS.codex, color: null },
  aider: { icon: ICONS.aider, color: null },
  goose: { icon: ICONS.gooseDark, color: null },
  v8agent: { icon: ICONS.bot, color: '#A3E635' },
  shell: { icon: ICONS.bot, color: '#6B7F8F' },
};
```

- [x] **Step 4: 运行测试确认通过**

```bash
node --test src/agentIcons.test.ts
```

Expected: 4 个测试全部 PASS。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/agentIcons.ts crates/dozer-app/web/review-trace/src/agentIcons.test.ts
git commit -m "feat(review-trace): 迁移 agent 图标与配色表"
```

---

### Task 3: traceStats.ts（token 统计与折叠摘要行）

**Files:**
- Create: `crates/dozer-app/web/review-trace/src/traceStats.ts`
- Test: `crates/dozer-app/web/review-trace/src/traceStats.test.ts`

**Interfaces:**
- Consumes: `AiTurnData` from `./types.ts`（Task 1）。
- Produces: `formatTokenCount(n: number): string`、`buildTraceStatsLabel(v: AiTurnData): string`，供 Task 6（`TraceToggle`）使用。

- [x] **Step 1: 写失败的测试**

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { formatTokenCount, buildTraceStatsLabel } from './traceStats.ts';

test('formatTokenCount below 1000 returns the plain number', () => {
  assert.equal(formatTokenCount(0), '0');
  assert.equal(formatTokenCount(999), '999');
});

test('formatTokenCount at/above 1000 uses a k-suffix and drops a trailing .0', () => {
  assert.equal(formatTokenCount(1000), '1k');
  assert.equal(formatTokenCount(1500), '1.5k');
  assert.equal(formatTokenCount(2000), '2k');
  assert.equal(formatTokenCount(12345), '12.3k');
});

test('buildTraceStatsLabel is empty with no tool calls and no tokens', () => {
  assert.equal(buildTraceStatsLabel({}), '');
});

test('buildTraceStatsLabel reports tool call count without errors', () => {
  assert.equal(
    buildTraceStatsLabel({ tool_calls: [{ summary: 'a' }, { summary: 'b' }] }),
    '2 次工具调用',
  );
});

test('buildTraceStatsLabel reports failed tool result count in parentheses', () => {
  assert.equal(
    buildTraceStatsLabel({
      tool_calls: [{ summary: 'a' }],
      tool_results: [{ content: 'x', is_error: true }],
    }),
    '1 次工具调用(1 失败)',
  );
});

test('buildTraceStatsLabel joins tool-call count and token total with a middle dot', () => {
  assert.equal(
    buildTraceStatsLabel({
      tool_calls: [{ summary: 'a' }],
      tokens_in: 500,
      tokens_out: 700,
    }),
    '1 次工具调用 · 1.2k tokens',
  );
});

test('buildTraceStatsLabel reports tokens alone when there are no tool calls', () => {
  assert.equal(buildTraceStatsLabel({ tokens_cache_read: 300 }), '300 tokens');
});
```

- [x] **Step 2: 运行测试确认失败**

```bash
node --test src/traceStats.test.ts
```

Expected: FAIL，`Cannot find module './traceStats.ts'`。

- [x] **Step 3: 创建 src/traceStats.ts**

```ts
import type { AiTurnData } from './types.ts';

// 1000 进 "k",一位小数、去掉多余的 ".0"——轨迹摘要行放不下完整数字,
// 只需要一眼看出量级。
export function formatTokenCount(n: number): string {
  if (n >= 1000) {
    return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
  }
  return String(n);
}

// 轨迹折叠行(未展开时)就能看到的统计摘要:工具调用次数(+失败数)、
// token 总量(in+out+cache_read+cache_write 相加,不细分——细分对一行
// 摘要来说信息过载,想看明细本来就有下面的工具结果/thinking 展开)。
// 两项都没有(纯 thinking、且老协议帧没有 token 字段)时返回空串,调用方
// 据此决定要不要挂这个 span。
export function buildTraceStatsLabel(v: AiTurnData): string {
  const parts: string[] = [];
  const toolCount = (v.tool_calls || []).length;
  if (toolCount > 0) {
    const errCount = (v.tool_results || []).filter((r) => r.is_error).length;
    parts.push(`${toolCount} 次工具调用${errCount > 0 ? `(${errCount} 失败)` : ''}`);
  }
  const totalTokens =
    (v.tokens_in || 0) + (v.tokens_out || 0) + (v.tokens_cache_read || 0) + (v.tokens_cache_write || 0);
  if (totalTokens > 0) {
    parts.push(`${formatTokenCount(totalTokens)} tokens`);
  }
  return parts.join(' · ');
}
```

- [x] **Step 4: 运行测试确认通过**

```bash
node --test src/traceStats.test.ts
```

Expected: 7 个测试全部 PASS。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/traceStats.ts crates/dozer-app/web/review-trace/src/traceStats.test.ts
git commit -m "feat(review-trace): 迁移 token 统计与轨迹摘要行"
```

---

### Task 4: markdown.ts（无依赖手写 markdown 渲染，风险最高的一块）

**Files:**
- Create: `crates/dozer-app/web/review-trace/src/markdown.ts`
- Test: `crates/dozer-app/web/review-trace/src/markdown.test.ts`

**Interfaces:**
- Produces: `escapeHtml(s: string): string`、`renderInlineMarkdown(s: string): string`（返回已转义、只含受控标签的 HTML 字符串）、`renderMarkdown(text: string): preact.VNode[]`，供 Task 5/6/7 的所有组件使用。

> 这是 spec 明确标出的最高风险模块——GFM 表格判据、代码块状态机、XSS 转义顺序必须逐行对照 `review_trace.html` 迁移，不要凭记忆重写。下面的实现和测试已经过独立验证（用 `preact-render-to-string` 渲染实际输出并断言字符串），照抄即可。

- [x] **Step 1: 写失败的测试**

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { h, Fragment } from 'preact';
import render from 'preact-render-to-string';
import { escapeHtml, renderInlineMarkdown, renderMarkdown } from './markdown.ts';

function html(text: string): string {
  return render(h(Fragment, null, ...renderMarkdown(text)));
}

test('escapeHtml escapes angle brackets and ampersands only', () => {
  assert.equal(escapeHtml('<script>&"x"</script>'), '&lt;script&gt;&amp;"x"&lt;/script&gt;');
});

test('renderInlineMarkdown never lets a literal angle bracket in source text become a real tag', () => {
  const out = renderInlineMarkdown('<img src=x onerror=alert(1)>');
  assert.ok(!out.includes('<img'));
  assert.ok(out.includes('&lt;img'));
});

test('renderInlineMarkdown supports bold, italic and inline code without cross-interference', () => {
  assert.equal(
    renderInlineMarkdown('**bold** *italic* `code`'),
    '<strong>bold</strong> <em>italic</em> <code>code</code>',
  );
});

test('renderInlineMarkdown does not let inline-code content be re-processed as markdown', () => {
  assert.equal(renderInlineMarkdown('`**not bold**`'), '<code>**not bold**</code>');
});

test('renderMarkdown wraps a single line of text in a paragraph', () => {
  assert.equal(html('hello world'), '<div class="md-p">hello world</div>');
});

test('renderMarkdown joins consecutive lines of one paragraph with <br>', () => {
  assert.equal(html('line one\nline two'), '<div class="md-p">line one<br>line two</div>');
});

test('renderMarkdown starts a new paragraph after a blank line', () => {
  assert.equal(
    html('first\n\nsecond'),
    '<div class="md-p">first</div><div class="md-p">second</div>',
  );
});

test('renderMarkdown renders headings with the matching level class', () => {
  assert.equal(html('## Title'), '<div class="md-h md-h2">Title</div>');
});

test('renderMarkdown renders a fenced code block verbatim, without markdown processing inside it', () => {
  assert.equal(
    html('```\nconst a = 1;\n**not bold**\n```'),
    '<pre class="md-code-block"><code>const a = 1;\n**not bold**</code></pre>',
  );
});

test('renderMarkdown renders an unordered list', () => {
  assert.equal(html('- one\n- two'), '<ul class="md-list"><li>one</li><li>two</li></ul>');
});

test('renderMarkdown renders an ordered list', () => {
  assert.equal(html('1. one\n2. two'), '<ol class="md-list"><li>one</li><li>two</li></ol>');
});

test('renderMarkdown renders a GFM table with header and body rows', () => {
  const out = html('| A | B |\n| --- | --- |\n| 1 | 2 |');
  assert.equal(
    out,
    '<table class="md-table"><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>',
  );
});

test('renderMarkdown does not treat a plain line containing one pipe as a table without a separator row', () => {
  assert.equal(html('a | b'), '<div class="md-p">a | b</div>');
});
```

- [x] **Step 2: 运行测试确认失败**

```bash
node --test src/markdown.test.ts
```

Expected: FAIL，`Cannot find module './markdown.ts'`。

- [x] **Step 3: 创建 src/markdown.ts**

```ts
import { h, type VNode } from 'preact';

// 极简 markdown → 安全 HTML(无依赖,不接 CDN——wry 自定义协议页面不
// 兜底外部脚本失败,离线优先)。文本先整体转义再拼标签,标签只由本函数
// 生成,不会把用户/模型文本里的尖括号当成真标签解释,规避 XSS。只覆盖
// agent 回复里常见的子集:标题/粗体/斜体/行内代码/代码块/有序无序列表/GFM
// 表格,不追求 CommonMark 全量兼容。与 review_trace.html 原实现逐行对照
// 迁移,详见 docs/superpowers/specs/2026-09-25-review-trace-preact-migration-design.md。
export function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function renderInlineMarkdown(s: string): string {
  const codeSpans: string[] = [];
  //  是私有区码位,正常文本几乎不可能出现,用来给行内代码占位,
  // 比原始 NUL 字节更安全。
  let out = escapeHtml(s).replace(/`([^`]+)`/g, (_match, code: string) => {
    codeSpans.push(code);
    return '' + (codeSpans.length - 1) + '';
  });
  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/__([^_]+)__/g, '<strong>$1</strong>');
  out = out.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
  out = out.replace(/_([^_\n]+)_/g, '<em>$1</em>');
  out = out.replace(/(\d+)/g, (_match, i: string) => '<code>' + codeSpans[Number(i)] + '</code>');
  return out;
}

function inlineHtml(s: string) {
  return { __html: renderInlineMarkdown(s) };
}

export function renderMarkdown(text: string): VNode[] {
  const lines = text.split('\n');
  const nodes: VNode[] = [];
  let para: string[] = [];
  let i = 0;

  const flushPara = () => {
    if (para.length === 0) return;
    nodes.push(
      h('div', {
        class: 'md-p',
        dangerouslySetInnerHTML: { __html: para.map(renderInlineMarkdown).join('<br>') },
      }),
    );
    para = [];
  };

  while (i < lines.length) {
    const line = lines[i];

    if (/^```/.test(line)) {
      flushPara();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) {
        codeLines.push(lines[i]);
        i++;
      }
      i++; // 跳过收尾的 ```
      nodes.push(h('pre', { class: 'md-code-block' }, h('code', null, codeLines.join('\n'))));
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushPara();
      nodes.push(
        h('div', {
          class: `md-h md-h${heading[1].length}`,
          dangerouslySetInnerHTML: inlineHtml(heading[2]),
        }),
      );
      i++;
      continue;
    }

    // GFM 表格:首行以 | 开头、第二行是 |---|---| 形式的分隔行才算表,
    // 避免把普通文本里的竖线误判成表格。
    if (
      /^\s*\|/.test(line) &&
      i + 1 < lines.length &&
      /^\s*\|(\s*:?-+:?\s*\|)+\s*$/.test(lines[i + 1])
    ) {
      flushPara();
      const rows: string[][] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) {
        rows.push(
          lines[i]
            .trim()
            .replace(/^\|/, '')
            .replace(/\|$/, '')
            .split('|')
            .map((c) => c.trim()),
        );
        i++;
      }
      // rows[0] = 表头,rows[1] = 分隔行(已由上面的判据保证存在),其余为正文。
      const headRow = h(
        'tr',
        null,
        rows[0].map((c) => h('th', { dangerouslySetInnerHTML: inlineHtml(c) })),
      );
      const bodyRows = rows
        .slice(2)
        .map((cells) =>
          h(
            'tr',
            null,
            cells.map((c) => h('td', { dangerouslySetInnerHTML: inlineHtml(c) })),
          ),
        );
      nodes.push(
        h('table', { class: 'md-table' }, h('thead', null, headRow), h('tbody', null, bodyRows)),
      );
      continue;
    }

    if (/^[-*]\s+/.test(line)) {
      flushPara();
      const items: VNode[] = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
        items.push(h('li', { dangerouslySetInnerHTML: inlineHtml(lines[i].replace(/^[-*]\s+/, '')) }));
        i++;
      }
      nodes.push(h('ul', { class: 'md-list' }, items));
      continue;
    }

    if (/^\d+\.\s+/.test(line)) {
      flushPara();
      const items: VNode[] = [];
      while (i < lines.length && /^\d+\.\s+/.test(lines[i])) {
        items.push(h('li', { dangerouslySetInnerHTML: inlineHtml(lines[i].replace(/^\d+\.\s+/, '')) }));
        i++;
      }
      nodes.push(h('ol', { class: 'md-list' }, items));
      continue;
    }

    if (line.trim() === '') {
      flushPara();
      i++;
      continue;
    }

    para.push(line);
    i++;
  }
  flushPara();
  return nodes;
}
```

- [x] **Step 4: 运行测试确认通过**

```bash
node --test src/markdown.test.ts
```

Expected: 12 个测试全部 PASS。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/markdown.ts crates/dozer-app/web/review-trace/src/markdown.test.ts
git commit -m "feat(review-trace): 迁移无依赖 markdown 渲染器"
```

---

### Task 5: AvatarIcon / ToolCallRow / ToolResultRow 组件

**Files:**
- Create: `crates/dozer-app/web/review-trace/src/components/AvatarIcon.tsx`
- Create: `crates/dozer-app/web/review-trace/src/components/ToolCallRow.tsx`
- Create: `crates/dozer-app/web/review-trace/src/components/ToolResultRow.tsx`

**Interfaces:**
- Consumes: `ToolCall`、`ToolResult` from `../types.ts`（Task 1）。
- Produces: `AvatarIcon({ svg, color }: { svg: string; color?: string | null })`、`ToolCallRow({ call }: { call: ToolCall })`、`ToolResultRow({ result }: { result: ToolResult })`，供 Task 6（`TraceToggle`）、Task 7（`Entry`）使用。

> 这几个是纯结构组件,没有可脱离浏览器断言的逻辑分支(唯一的逻辑分支——`content.length < 200` 决定默认展开——用 `npm run typecheck` 保证类型正确,行为正确性放在 Task 11 人工视觉核对里验证,这是本计划里 `.tsx` 文件统一的验证口径,不是本任务遗漏)。

- [x] **Step 1: 创建 src/components/AvatarIcon.tsx**

```tsx
export function AvatarIcon({ svg, color }: { svg: string; color?: string | null }) {
  return (
    <span
      class="avatar-icon"
      style={color ? { color } : undefined}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
```

- [x] **Step 2: 创建 src/components/ToolCallRow.tsx**

```tsx
import type { ToolCall } from '../types.ts';

export function ToolCallRow({ call }: { call: ToolCall }) {
  return (
    <details class="tool-call-row">
      <summary>{call.summary}</summary>
      {call.input_json ? <div class="input-json">{call.input_json}</div> : null}
    </details>
  );
}
```

- [x] **Step 3: 创建 src/components/ToolResultRow.tsx**

```tsx
import type { ToolResult } from '../types.ts';

export function ToolResultRow({ result }: { result: ToolResult }) {
  const firstLine = result.content.split('\n')[0].slice(0, 80);
  return (
    <details
      class={`tool-result-row${result.is_error ? ' error' : ''}`}
      open={result.content.length < 200}
    >
      <summary>{(result.is_error ? '⚠ 失败 · ' : '→ ') + firstLine}</summary>
      <div class="body">{result.content}</div>
    </details>
  );
}
```

- [x] **Step 4: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/components/AvatarIcon.tsx \
  crates/dozer-app/web/review-trace/src/components/ToolCallRow.tsx \
  crates/dozer-app/web/review-trace/src/components/ToolResultRow.tsx
git commit -m "feat(review-trace): 迁移头像图标与工具调用/结果行组件"
```

---

### Task 6: TraceToggle 组件（thinking/tool_calls/tool_results 折叠时间线）

**Files:**
- Create: `crates/dozer-app/web/review-trace/src/components/TraceToggle.tsx`

**Interfaces:**
- Consumes: `AiTurnData` from `../types.ts`（Task 1）、`buildTraceStatsLabel` from `../traceStats.ts`（Task 3）、`renderMarkdown` from `../markdown.ts`（Task 4）、`ToolCallRow`/`ToolResultRow` from `./ToolCallRow.tsx`/`./ToolResultRow.tsx`（Task 5）。
- Produces: `TraceToggle({ v }: { v: AiTurnData })`，供 Task 7（`Entry`）使用；`v.thinking_text`/`tool_calls`/`tool_results` 全缺失时返回 `null`（对应 Review Focus「缺省字段路径」）。

- [x] **Step 1: 创建 src/components/TraceToggle.tsx**

```tsx
import type { AiTurnData } from '../types.ts';
import { buildTraceStatsLabel } from '../traceStats.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolCallRow } from './ToolCallRow.tsx';
import { ToolResultRow } from './ToolResultRow.tsx';

export function TraceToggle({ v }: { v: AiTurnData }) {
  const hasThinking = !!v.thinking_text;
  const hasToolCalls = !!(v.tool_calls && v.tool_calls.length > 0);
  const hasToolResults = !!(v.tool_results && v.tool_results.length > 0);
  if (!hasThinking && !hasToolCalls && !hasToolResults) {
    return null;
  }

  const statsLabel = buildTraceStatsLabel(v);

  return (
    <details class="trace-toggle">
      <summary>{statsLabel ? <span class="trace-stats">{statsLabel}</span> : null}</summary>
      <div class="trace-timeline">
        {hasThinking ? (
          <div class="trace-item thinking">
            <div class="item-title">思考过程</div>
            <div class="thinking-text">{renderMarkdown(v.thinking_text as string)}</div>
          </div>
        ) : null}
        {hasToolCalls
          ? v.tool_calls!.map((call, i) => (
              <div class="trace-item tool-call" key={i}>
                {i === 0 ? <div class="item-title">操作过程</div> : null}
                <ToolCallRow call={call} />
              </div>
            ))
          : null}
        {hasToolResults
          ? v.tool_results!.map((result, i) => (
              <div class={`trace-item tool-result${result.is_error ? ' error' : ''}`} key={i}>
                {i === 0 ? <div class="item-title">工具结果</div> : null}
                <ToolResultRow result={result} />
              </div>
            ))
          : null}
      </div>
    </details>
  );
}
```

- [x] **Step 2: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [x] **Step 3: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/components/TraceToggle.tsx
git commit -m "feat(review-trace): 迁移轨迹折叠时间线组件"
```

---

### Task 7: SummaryHeader / Entry 组件

**Files:**
- Create: `crates/dozer-app/web/review-trace/src/components/SummaryHeader.tsx`
- Create: `crates/dozer-app/web/review-trace/src/components/Entry.tsx`

**Interfaces:**
- Consumes: `TraceData`、`TraceEntry`、`AgentLabel` from `../types.ts`（Task 1）、`renderMarkdown` from `../markdown.ts`（Task 4）、`ICONS`/`AGENT_STYLE` from `../agentIcons.ts`（Task 2）、`AvatarIcon`/`ToolResultRow` from `./AvatarIcon.tsx`/`./ToolResultRow.tsx`（Task 5）、`TraceToggle` from `./TraceToggle.tsx`（Task 6）。
- Produces: `SummaryHeader({ data }: { data: Pick<TraceData, 'summary_title'|'summary_time'|'summary_text'> })`、`Entry({ entry, agentLabel }: { entry: TraceEntry; agentLabel?: AgentLabel })`，供 Task 8（`main.tsx`）使用。`SummaryHeader` 在 `summary_text` 缺失时返回 `null`（Review Focus「缺省字段路径」）。

- [x] **Step 1: 创建 src/components/SummaryHeader.tsx**

```tsx
import type { TraceData } from '../types.ts';
import { renderMarkdown } from '../markdown.ts';

type SummaryHeaderData = Pick<TraceData, 'summary_title' | 'summary_time' | 'summary_text'>;

export function SummaryHeader({ data }: { data: SummaryHeaderData }) {
  if (!data.summary_text) return null;
  return (
    <div class="summary-header">
      {data.summary_title ? <div class="summary-title">{data.summary_title}</div> : null}
      {data.summary_time ? <div class="summary-time">{data.summary_time}</div> : null}
      <div class="summary-body">{renderMarkdown(data.summary_text)}</div>
    </div>
  );
}
```

- [x] **Step 2: 创建 src/components/Entry.tsx**

```tsx
import type { AgentLabel, AiTurnData, ToolResult, TraceEntry } from '../types.ts';
import { AGENT_STYLE, ICONS } from '../agentIcons.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolResultRow } from './ToolResultRow.tsx';
import { TraceToggle } from './TraceToggle.tsx';
import { AvatarIcon } from './AvatarIcon.tsx';

const ROLE_LABEL: Record<string, string> = { ToolResult: '工具结果' };

export function Entry({ entry, agentLabel }: { entry: TraceEntry; agentLabel?: AgentLabel }) {
  const kind = Object.keys(entry)[0] as 'Human' | 'AiTurn' | 'ToolResult';
  const isErrorResult = kind === 'ToolResult' && (entry as { ToolResult: ToolResult }).ToolResult.is_error;

  let head;
  if (kind === 'Human') {
    head = (
      <div class="msg-head">
        <AvatarIcon svg={ICONS.user} />
        <span>you</span>
      </div>
    );
  } else if (kind === 'AiTurn') {
    const style = (agentLabel && AGENT_STYLE[agentLabel]) || { icon: ICONS.bot, color: null };
    head = (
      <div class="msg-head">
        <AvatarIcon svg={style.icon} color={style.color} />
        <span>{agentLabel || 'AI'}</span>
      </div>
    );
  } else {
    head = (
      <div class="msg-head">
        <span class="avatar-dot" />
        <span>{ROLE_LABEL[kind] || kind}</span>
      </div>
    );
  }

  return (
    <div class={`msg ${kind}${isErrorResult ? ' error' : ''}`}>
      {head}
      <div class="bubble">
        {kind === 'ToolResult' ? (
          <ToolResultRow result={(entry as { ToolResult: ToolResult }).ToolResult} />
        ) : null}
        {kind === 'Human' ? (
          <div class="text">{renderMarkdown((entry as { Human: { text: string } }).Human.text)}</div>
        ) : null}
        {kind === 'AiTurn' ? (
          <>
            {(entry as { AiTurn: { text?: string } }).AiTurn.text ? (
              <div class="text">
                {renderMarkdown((entry as { AiTurn: { text: string } }).AiTurn.text)}
              </div>
            ) : null}
            <TraceToggle v={(entry as { AiTurn: AiTurnData }).AiTurn} />
          </>
        ) : null}
      </div>
    </div>
  );
}
```

- [x] **Step 3: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/components/SummaryHeader.tsx \
  crates/dozer-app/web/review-trace/src/components/Entry.tsx
git commit -m "feat(review-trace): 迁移摘要头与消息条目分发组件"
```

---

### Task 8: main.tsx 接上真实 fetch 逻辑

**Files:**
- Modify: `crates/dozer-app/web/review-trace/src/main.tsx`（替换 Task 1 的占位版）

**Interfaces:**
- Consumes: `TraceData` from `./types.ts`（Task 1）、`SummaryHeader` from `./components/SummaryHeader.tsx`（Task 7）、`Entry` from `./components/Entry.tsx`（Task 7）。
- Produces: 挂载到 `#timeline` 的完整 App；保留原实现的 `.catch` 错误态（Review Focus「`data.json` 404 时的错误态」）。

- [x] **Step 1: 替换 src/main.tsx**

```tsx
import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { TraceData } from './types.ts';
import { SummaryHeader } from './components/SummaryHeader.tsx';
import { Entry } from './components/Entry.tsx';
import './styles.css';

function App() {
  const [data, setData] = useState<TraceData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch('dozer://review-trace/data.json')
      .then((r) => r.json())
      .then((d: TraceData) => setData(d))
      .catch((err) => setError(String(err)));
  }, []);

  if (error) return <>{'加载失败: ' + error}</>;
  if (!data) return null;

  return (
    <>
      <SummaryHeader data={data} />
      {data.summary_text ? <hr class="topic-divider" /> : null}
      {data.entries.map((entry, i) => (
        <Entry entry={entry} agentLabel={data.agent_label} key={i} />
      ))}
    </>
  );
}

render(<App />, document.getElementById('timeline')!);
```

- [x] **Step 2: 类型检查 + 构建**

```bash
npm run typecheck
npm run build
```

Expected: 两条命令都退出码 0；`npm run build` 打印 `built -> .../assets/review-trace`。

- [x] **Step 3: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/main.tsx crates/dozer-app/assets/review-trace
git commit -m "feat(review-trace): 接上真实 data.json fetch 逻辑"
```

---

### Task 9: Rust 侧路由改造（`assets.rs`）+ 删除旧文件

**Files:**
- Modify: `crates/dozer-app/src/assets.rs:157-162`（`editor_root_for` 之后新增 `review_trace_root_for`）
- Modify: `crates/dozer-app/src/assets.rs:219-239`（`review-trace/` 命名空间路由）
- Modify: `crates/dozer-app/src/assets.rs`（`review_trace_*` 测试，见 Task 10 一并改）
- Delete: `crates/dozer-app/src/review_trace.html`

**Interfaces:**
- Consumes: `serve_vendored`、`editor_root_for` 已有实现（同文件内）。
- Produces: `review_trace_root_for(flyfish_root: &Path) -> PathBuf`；`dozer://review-trace/` 路由除 `data.json` 外全部落到 `serve_vendored`。

- [x] **Step 1: 在 `editor_root_for` 之后新增 `review_trace_root_for`**

在 `crates/dozer-app/src/assets.rs` 第 162 行（`editor_root_for` 函数结束的 `}` 之后）插入：

```rust
/// review-trace host(会话审阅 trace 时间线)静态资源根 = flyfish 根的
/// 兄弟目录 `review-trace`。同 `editor_root_for`,dev 与打包态同构。
fn review_trace_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("review-trace")
}
```

- [x] **Step 2: 把 `review-trace/` 命名空间路由改成磁盘服务**

把现有这一段（第 219-239 行）：

```rust
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
```

替换成：

```rust
    // 审阅面板 trace 页面(2026-09-25 起 Preact 离线打包产物,从磁盘服务,
    // 同 editor/json-editor):数据端点回显调用方注入的当前审阅内容快照
    // ——没有快照(还没加载过审阅内容)时 404,判断顺序在 serve_vendored
    // 之前,不受影响。
    if let Some(path) = rest.strip_prefix("review-trace/") {
        if path == "data.json" {
            return match review_data {
                Some(json) => ProtocolReply {
                    status: 200,
                    mime: "application/json",
                    body: json.as_bytes().to_vec(),
                },
                None => not_found(),
            };
        }
        return serve_vendored(&review_trace_root_for(assets_root), path);
    }
```

- [x] **Step 3: 删除旧文件**

```bash
git rm crates/dozer-app/src/review_trace.html
```

- [x] **Step 4: 编译确认没有遗留引用**

```bash
cargo check -p dozer-app
```

Expected: 编译通过（此时 4 个 `review_trace_*` 测试会失败，Task 10 里修；先只确认非测试代码能编译）。若报 `include_str!` 找不到文件或未使用的 import，回去检查 Step 2 是否完整替换。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/assets.rs
git commit -m "feat(review-trace): assets.rs 路由改成磁盘服务，删除旧 include_str! host"
```

---

### Task 10: 更新 Rust 测试 + 全量构建产物 + 跑通测试套件

**Files:**
- Modify: `crates/dozer-app/src/assets.rs`（测试模块，`review_trace_host_html_serves_embedded_page_regardless_of_review_data` 及新增 CSP/无泄漏测试）

**Interfaces:**
- Consumes: Task 9 的路由改动；Task 1-8 产出的 `crates/dozer-app/assets/review-trace/*` 构建产物。

- [x] **Step 1: 把 `review_trace_host_html_serves_embedded_page_regardless_of_review_data` 改成断言构建产物存在**

找到这个测试（原本断言「编译期内嵌」，这个前提已经不成立）：

```rust
    #[test]
    fn review_trace_host_html_serves_embedded_page_regardless_of_review_data() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://review-trace/host.html?_r=1",
        );
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(!r.body.is_empty());
    }
```

替换成：

```rust
    /// 提交的 review-trace 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn review_trace_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/review-trace"));
        for f in ["host.html", "review-trace.js", "review-trace.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 review-trace 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "review-trace 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP + 无网络:host.html 不得引用任何外部 URL,且带 `default-src 'none'`。
    #[test]
    fn review_trace_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/review-trace"));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("Content-Security-Policy"), "host.html 必须声明 CSP");
        assert!(
            html.contains("default-src 'none'"),
            "CSP 必须以 default-src 'none' 起步"
        );
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(html.contains("connect-src 'self'"), "仅允许同源 fetch");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "review-trace host.html 不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("review-trace.js") && html.contains("review-trace.css"));
    }

    /// 打包产物不得泄漏本机绝对路径或 sourcemap 引用。
    #[test]
    fn review_trace_js_has_no_absolute_paths_or_sourcemap() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/review-trace"));
        let js = std::fs::read_to_string(root.join("review-trace.js")).expect("读 review-trace.js");
        assert!(!js.contains("sourceMappingURL"), "不应有 sourcemap 引用");
        assert!(
            !js.contains(env!("CARGO_MANIFEST_DIR")),
            "不应含源码树绝对路径"
        );
    }
```

`review_trace_data_json_echoes_injected_snapshot`、`review_trace_data_json_404_when_no_snapshot`、`review_trace_unknown_subpath_404` 三个测试原样保留，不用改。

- [x] **Step 2: 确认 `crates/dozer-app/web/review-trace` 构建产物是最新的**

```bash
cd crates/dozer-app/web/review-trace
npm run typecheck
npm run test
npm run build
cd ../../../..
```

Expected: 三条命令全部退出码 0（`npm run test` 应显示 Task 2/3/4 的全部测试通过）。

- [x] **Step 3: 跑 Rust 测试套件**

```bash
cargo test -p dozer-app
```

Expected: 全部通过，包括 Step 1 新增/改造的 4 个 `review_trace_*` 测试。若 `review_trace_bundle_assets_are_present` 失败，先确认 Step 2 的 `npm run build` 真的跑过。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/src/assets.rs crates/dozer-app/assets/review-trace
git commit -m "test(review-trace): 更新 assets.rs 测试适配磁盘服务的 Preact 产物"
```

---

### Task 11: 人工视觉核对（新旧实现并排比对）

**Files:**
- Create（临时，仅用于本任务核对，不提交到仓库）: 一份脱敏 `data.json` 样本

**Interfaces:**
- Consumes: Task 8 的完整产物（`crates/dozer-app/assets/review-trace/`）。

> 这一步覆盖 Review Focus 里"缺省字段路径""XSS""超长内容默认折叠"三条——这些是 `.tsx` 组件唯一的行为验证点(本计划里 `.tsx` 文件统一走 typecheck + 视觉核对,不引入 jsdom/组件测试框架,理由见 spec「测试与验证」一节)。
>
> **补记(2026-09-25，code-review 后)：** 首次落地时这一整个任务被跳过——落地
> commit 自己写了"Task 11 待人工核对"，六步全部是 `- [ ]`。独立 code-review
> fork 指出后在本轮补做，用浏览器实测发现并当场验证修复了两个真实 bug：
> `main.tsx` 的错误处理只覆盖 fetch/JSON 阶段，entries 渲染期抛出的异常不会
> 触发"加载失败"提示而是白屏；`scripts/build-macos-app.sh` 从未加上
> `assets/review-trace` 的拷贝，导致打包后的 .app 里会话审阅整个 404（两个
> 问题都不在 Task 11 原定步骤范围内，是核对真实数据时顺带发现的）。修复见
> commit `44363dc2`。

- [x] **Step 1: 准备覆盖全部形态的样本 `data.json`**

在 `crates/dozer-app/assets/review-trace/` 旁边创建一个临时 `fixture.json`（不要 `git add`，验证完删除）：

```json
{
  "summary_title": "示例会话摘要",
  "summary_time": "2026-09-25 10:00",
  "summary_text": "这是一段**加粗**的摘要,包含一个链接式强调和 `inline code`。",
  "agent_label": "claude",
  "entries": [
    { "Human": { "text": "帮我看看 <script>alert(1)</script> 这段文本会不会被当成标签" } },
    {
      "AiTurn": {
        "text": "不会,会被转义成纯文本显示。\n\n| 列 A | 列 B |\n| --- | --- |\n| 1 | 2 |",
        "thinking_text": "先确认转义逻辑,再检查 XSS 测试用例。",
        "tool_calls": [{ "summary": "grep escapeHtml", "input_json": "{\"pattern\":\"escapeHtml\"}" }],
        "tool_results": [
          { "content": "match: crates/dozer-app/web/review-trace/src/markdown.ts", "is_error": false },
          {
            "content": "一段超过 200 字符的日志用来验证默认折叠:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "is_error": true
          }
        ],
        "tokens_in": 500,
        "tokens_out": 700
      }
    },
    { "ToolResult": { "content": "独立工具结果消息,非 error", "is_error": false } }
  ]
}
```

- [x] **Step 2: 起本地静态服务器，用假的 `dozer://` fetch 目标测（用普通 http 服务器 + 浏览器直接打开 host.html，`fetch` 路径手工改指向本地 `fixture.json`）**

```bash
cd crates/dozer-app/assets/review-trace
python3 -m http.server 8935 &
```

临时把 `review-trace.js` 里的 `dozer://review-trace/data.json` 换成 `data.json`（相对路径）只是本地核对用的 hack，**验证完必须 `git checkout` 撤销**，不能带进构建产物:

```bash
sed -i '' 's#dozer://review-trace/data.json#data.json#' review-trace.js
```

这时 `data.json` 还不存在——先用浏览器打开一次 `http://127.0.0.1:8935/host.html`，确认页面显示"加载失败: ..."文本而不是空白（Review Focus 第 3 条：`data.json` 404 时的错误态，`fetch(...).json()` 收到 `not found` 纯文本会因 JSON 解析失败进入 `.catch` 分支，和原实现行为一致，这里手工确认一遍）。确认后再放真实样本:

```bash
cp fixture.json data.json
```

刷新页面，继续下面的核对。

- [x] **Step 3: 用浏览器打开并核对**

用 claude-in-chrome（或任意浏览器）打开 `http://127.0.0.1:8935/host.html`，核对：

- Human/AiTurn/ToolResult 三种气泡的颜色、对齐、边框色与 `review_trace.html` 原版一致（金色人类气泡右对齐、青色 AI 气泡左对齐、绿色工具结果）。
- `<script>alert(1)</script>` 那句 Human 消息里的尖括号原样显示为文本，不触发弹窗、控制台无脚本执行痕迹。
- AiTurn 里的 GFM 表格正确渲染成 `<table>`,加粗/行内代码正确。
- 轨迹折叠行(`详情`)展开后能看到"思考过程"/"操作过程"/"工具结果"三段,工具调用次数和 token 统计文本与原版口径一致。
- 超过 200 字符的那条 `tool_results` 默认是折叠状态(收起),另一条(is_error, 短内容)默认展开。
- 控制台(`read_console_messages`)零报错、零 CSP violation。
- **缺省字段路径**(Review Focus 第 2 条):把 `data.json` 里的 `summary_title`/`summary_time`/`summary_text` 三个字段整体删掉刷新一次,确认摘要头和分割线完全不渲染(不是渲染成空 div);再把某条 `AiTurn` 条目的 `thinking_text`/`tool_calls`/`tool_results` 三个字段都删掉,确认那条消息没有"轨迹 ▸"折叠行(`TraceToggle` 返回 `null` 生效,不是渲染出一个空的 `<details>`)。

- [x] **Step 4: 撤销临时改动，清理**

```bash
git checkout -- review-trace.js
rm data.json fixture.json
kill %1  # 关掉 python3 http.server
cd ../../../..
```

- [ ] **Step 5: 真实入口手动过一遍（仍未做，如实记录）**

`cargo run -p dozer-app` 启动应用，分别打开：
1. 核心 Agent 面板的终端会话审阅(`ReviewSource::Session`)。
2. 对话面板历史会话审阅(`ReviewSource::Conversation`)。

确认两个入口渲染出的 trace 时间线视觉一致、都是新 Preact 实现(可通过 DevTools 检查 `review-trace.js` 是否被加载,或确认布局与 Step 3 核对结果一致)。

> **2026-09-25 状态：Step 1-4/6 已用本地 fixture harness 实测完成（含新发现并修复的两个 bug，见上方补记）；Step 5 没有做**——需要一个跑起来的 `dozer-app` 会话加真实的 Agent/Conversation 数据，在本轮修复里没有起这个环境。`data.json` 的响应体形状在 Rust 侧和前端侧都没变（数据契约冻结，Step 1-4 已经拿真实形状的 fixture 测过两条消费路径共用的同一份渲染代码），所以剩余风险主要是"webview 挂载/embedding 这层 Rust 胶水代码是否接对了新路由"，不是渲染逻辑本身——这部分仍需要在真实环境里补一次。

- [x] **Step 6: 无需 commit**（本任务不产出提交内容，只是验证）

---

### Task 12: 全量构建 + lint + 最终提交

**Files:**
- 无新增/修改文件；本任务是收尾质量门。

**Interfaces:**
- Consumes: Task 1-11 全部产出。

- [x] **Step 1: 全 workspace 构建**

```bash
cargo build
```

Expected: 编译通过，无警告新增。

- [x] **Step 2: clippy + fmt**

```bash
cargo clippy --all-targets
cargo fmt --check
```

Expected: `clippy` 无新增警告；`fmt --check` 无差异(若有差异，跑 `cargo fmt` 后重新 `git add`)。

- [x] **Step 3: 完整测试套件**

```bash
cargo test -p dozer-app
```

Expected: 全部通过。

- [x] **Step 4: 确认 `review_trace.html` 及其所有引用已清除**

```bash
grep -rn "review_trace.html" crates/dozer-app/src || echo "clean"
```

Expected: 输出 `clean`（只应该在这条 grep 命令本身之外找不到任何引用；如果 `assets.rs` 里的注释仍提到旧文件名作为历史说明，属于正常，不是代码引用）。

- [x] **Step 5: 最终收尾 commit（如果前面步骤有 fmt/clippy 修复产生改动）**

```bash
git status --short
# 如果有改动:
git add -A
git commit -m "chore(review-trace): cargo fmt/clippy 收尾"
```

Expected: 工作区干净，`git log --oneline -12` 能看到本计划 Task 1-10 的全部提交。

---

## 实施记录（Task 1–10、12 已落地，Task 11 待人工核对）

**提交序列（本计划）：**

- `b9309c12` 脚手架 Preact + esbuild 离线打包管线（Task 1）
- `bf003375` 迁移 agent 图标与配色表（Task 2）
- `85a9ae05` 迁移 token 统计与轨迹摘要行（Task 3）
- `8cca0518` 迁移无依赖 markdown 渲染器（Task 4）
- `2d59e174` 迁移头像图标与工具调用/结果行组件（Task 5）
- `56a54338` 迁移轨迹折叠时间线组件（Task 6）
- `869b471f` 迁移摘要头与消息条目分发组件（Task 7）
- `0ce3adeb` 接上真实 data.json fetch 逻辑（Task 8）
- `cf3bb7be` assets.rs 路由改成磁盘服务，删除旧 include_str! host（Task 9）
- `dea4430a` 更新 assets.rs 测试适配磁盘服务的 Preact 产物（Task 10，含 cargo fmt 收尾）

**与计划文本的偏差（均为修正而非改需求）：**

- `markdown.ts` 里 `nodes`/`items` 数组类型改为 `VNode<any>[]`：计划原文 `VNode[]`（默认 `P = {}`）在本仓 Preact 10.29.8 类型下无法容纳带 `class`/`dangerouslySetInnerHTML` 的 VNode 入栈（`npm run typecheck` 报 TS2345）。运行时行为与导出签名 `renderMarkdown(text: string): VNode[]` 不变。
- 行内代码占位符实现按原 `review_trace.html` 的 `\uE000`（计划文档里该私有区码位被渲染丢失为空串），语义与旧实现一致。
- `agentIcons.ts` 的 SVG 字符串从 `review_trace.html` 机械抽取（计划文档里 codex 等长 SVG 被截断），保证逐字节一致。

**验证门禁（本地全绿）：**

- `npm run typecheck`（review-trace）：退出码 0。
- `npm run test`（review-trace）：24 项通过（agentIcons 4 + traceStats 7 + markdown 13）。
- `npm run build`：产出 `assets/review-trace/{host.html,review-trace.js,review-trace.css}`，`review-trace.js` SHA-256 `c17619ce…`。
- 行为核对（临时 esbuild bundle + `preact-render-to-string`，未提交）：缺省字段路径（`summary_text` 缺失不渲染摘要头、`thinking_text`/`tool_calls`/`tool_results` 全缺失不渲染 `TraceToggle`）、Human 气泡 `<script>` 转义、超长 `tool_results`（>200 字符）默认折叠/短内容默认展开、未知 `agent_label` 回落 bot 图标、GFM 表格渲染——9 项断言全部 PASS。
- `cargo test -p dozer-app`：1349 项通过（含 6 个 `review_trace_*`）。
- `cargo clippy -p dozer-app --all-targets`：无新增警告（现有警告均在 codehealth/file_history/confirm_overlay 等无关模块）。
- `grep review_trace.html crates/dozer-app/src`：仅剩描述性注释引用（`app.rs` 数据契约说明、`markdown.ts` 迁移说明），无代码引用。

**Task 11（人工视觉核对）尚未执行**：需要在浏览器里并排比对新旧渲染（气泡配色/对齐、XSS、表格、轨迹三段、`>200` 折叠、控制台零 CSP violation），以及 `cargo run -p dozer-app` 走 `ReviewSource::Session` / `ReviewSource::Conversation` 两个真实入口。该步骤依赖人工观察，保留为未勾选。

**工作树注意**：本会话期间有并行改动落在 `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/{dom.test.ts,svgIds.ts}` 与 `crates/dozer-app/assets/flyfish/renderers/text.iife.js`，非本计划产出，未纳入本计划的任何提交。
