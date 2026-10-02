# Todo 内容区迁移到 WebView Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Todo 面板右栏中**原生 tab 行以下**的内容（搜索条、状态筛选、三段任务卡片、新增框、五个弹层）换成 Preact + esbuild 离线打包的固定单槽 WebView；左栏分类树、右栏顶部 tab 行、详情窗口保持原生。1:1 保留行为，迁完删除旧 iced 列表与相关接线。

**Architecture:** 前端 `web/todo-content/` 只渲染与保存纯视图状态；Rust 在 `extensions/todo/protocol.rs` 把 `WorkspaceState` 算成 `TodoViewPayload` 整表推送（声明式比较 + 递增 `revision`）。WebView 事件携带任务 `id`，Rust 用纯函数 `route_event` 把 `id` 映射成当前下标，再**复用现有的下标式消息与更新逻辑**（`Toggle(idx)`、`StatusPick(idx, …)`、`todo_assign_agent` 等），因此现有落库、派生状态、测试基本不动。宿主接入点（内容 ID 偏移、指令队列、几何、IPC 路由、失败占位页）逐处照抄已合并的 Code Health。

**Tech Stack:** Rust 2024、iced 0.14、wry（WebView）、`serde`；前端 Preact 10 + TypeScript + esbuild，`node --test` 做单测，`preact-render-to-string` 做渲染冒烟。

**Spec:** `docs/superpowers/specs/2026-10-02-todo-webview-design.md`

**本 plan 只覆盖 spec 的 WebView 部分。** spec 里「左栏分类树拖动移动」是独立子系统，见 `docs/superpowers/plans/2026-10-02-todo-category-drag.md`，**必须在本 plan 之后执行**（本 plan 让卡片分类 chip 改走 DOM 选择器，第二份 plan 再删除 iced 的 `category_picker` 整套）。

## 对 spec 的两处细化（Rulings，执行前请知悉）

1. **乐观更新留在 Rust，不挪到前端。** spec 写的是"由前端本地先改"。实际现有 `Toggle` / 新增已经在 Rust 里先改 `items` 再落库；Rust 每帧声明式比较并推送，进程内到 WebView 只有一帧延迟，体验等价，且复用现有代码与测试。**前端只对拖拽排序做本地预览**（松手前的重排）。代价：若 Rust 的乐观状态与落库结果不一致，靠下一次权威推送覆盖，与现状相同。
2. **新增后的高亮与滚回顶部沿用 Rust 的 `Flash` / `scroll_to_top`，通过 payload 字段 `selected_id` 与 `scroll_nonce` 带给前端。** spec 写的是"取消 `Flash` 跨边界协议、前端用 `id` 差异判断"。但乐观新增用的是临时 `OPTIMISTIC_TODO_ID`，落库后换成真实 id，按 `id` 差异会闪两次、且 DOM 节点被换掉导致高亮中断。保留 Rust 现有的 2 秒计时（`start_flash` / `advance_flash` / `next_flash_wake`）最稳。代价：这三个函数及其 `main.rs` 接线不能按 spec 删除，**保留**。
3. spec 的「纯前端状态」补充：卡片**选中高亮**（点击选中）也是前端本地状态，Rust 只通过 `selected_id` 在新增后覆盖一次。

这三处与 spec 不一致，执行完成后需同步更新 spec 对应段落（Task 10 有这一步）。

## Global Constraints

- **独立 worktree 分支开发，不在 main 上直接提交**：`git worktree add .worktrees/todo-webview -b feat/todo-webview main`。主工作区常有别人未提交的改动（版本号递增、`dozerd/src/summary_pipeline.rs` 等），**不要碰、不要 stash**；每次 `git add` / `git commit` 前先 `git branch --show-current` 确认是 `feat/todo-webview`，且只 `git add` 具体路径。若用 subagent，dispatch 第一句必须是 `cd <worktree 绝对路径> && git branch --show-current`，Read / Edit 的 `file_path` 带完整 worktree 绝对路径前缀。
- **提交结尾附** `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- **GUI 只用 iced 0.14 生态；WebView 恒在 iced 之上。** 本 plan 之后 Todo 面板内不再有会与 WebView 重叠的 iced 浮层；新增浮层默认走独立原生子窗口。
- **字体**：WebView 内一律系统默认字体（`-apple-system, "PingFang SC", sans-serif`），不用等宽代码字体；中文渲染由浏览器负责。
- **颜色**：ByteBoy2077 令牌，金色只给甲方动作；状态不只靠颜色，保留文字标签。CSS 变量取值与 `web/codehealth-content/src/styles.css` 完全一致（含 `light` 方案，主题由 URL `?theme=` 注入）。
- **瞬时失败统一走 Toast**（`App::push_toast` / Outbox），不在内容区自画；`daemon_unavailable` 由顶栏徽标展示。日志来源名 `todo`，用 `dozer_core::log_*!(LOG, …)`，禁止裸 `tracing::warn!` / `eprintln!`（`scripts/check-log-scope.sh` 门禁）；**不记录任务全文或输入内容**，只记 `id` 与操作类型。
- **只读核心原则**：不新增任何编辑入口；本 plan 只搬迁现有交互。
- **不改 dozerd / dozer-core / dozer-client 任何代码**。
- **前端构建产物提交进仓库**（`crates/dozer-app/assets/todo-content/`），与其它 content webview 一致；运行时无 Node、无 CDN、无 sourcemap。
- **测试基线**：`cargo test -p dozer-app` 里 `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 main 上就失败；`cargo test --workspace` 整包运行时 dozerd 的 3 个 `summary_pipeline` 测试也失败（单独 `-p dozerd` 通过）。这些是已知基线，**不要修，也不要把它们算作本 plan 引入的失败**；每次宣称通过前要贴出实际命令输出。
- **`dead_code` 警告是路由 bug 的一手信号**：Task 8、9 删除代码后，编译器报出的每一条 `dead_code` 都要判断是"预期要删"还是"接线漏了"，不要一律 `#[allow]`。
- 可选提速：`export CARGO_TARGET_DIR=/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/target`，在新 worktree 里复用依赖缓存（别人也在用该目录时 cargo 会自动排队）。

## Review Focus

1. **过期 `id`**：用户点击时，别的 agent 经 MCP 已把该任务删掉或改了状态。`route_event` 找不到 `id` 必须返回 `None`（空操作），绝不能按旧下标改到别的行。Task 5 的 `route_event_unknown_id_is_noop` 与 Task 7 的集成断言覆盖。
2. **输入法组合输入**：中文输入法里按回车是确认候选词。新增框的 ⌘↵、内联编辑的回车在 `isComposing` 或 `keyCode === 229` 时**绝不提交**。Task 2 的 `compose.test.ts` 覆盖。
3. **单槽 WebView 跨项目**：同一个 webview 在切换项目标签后要显示新项目的数据，且旧项目的搜索词、状态筛选、新增草稿切回来还在；分类切换（`category_key` 变化）只清搜索，不清草稿。Task 2 的 `uiStore.test.ts` 覆盖。
4. **`Ready` 重发与 revision**：WebView 被重建（折叠再展开、重试）后 `set_ready(true)` 必须清空 `last_sent` 强制重发；旧 `revision` 的推送必须被前端丢弃。Task 1 的 `protocol.test.ts` 与 Task 5 的 `webview_push_state_*` 覆盖。
5. **过滤状态下拖拽排序**：搜索或状态筛选隐藏了中间的任务时，`after_id` 取放置位置上方**可见**卡片的 id；放到最上面发 `null`；放回原位不发事件。Task 2 的 `reorder.test.ts` 覆盖。
6. **看板视图与收起列表列**：`content_desired` 为假或列表列收起时矩形必须是零尺寸，WebView 不得盖住原生看板占位。Task 6 的几何测试覆盖。

---

## 文件结构总览

```
crates/dozer-app/
├── web/todo-content/                       # Task 1–3（新）
│   ├── package.json  tsconfig.json  build.mjs  render-smoke.mjs
│   └── src/
│       ├── host.html  main.tsx  ipc.ts  protocol.ts  errors.ts  types.ts  styles.css
│       ├── filter.ts  segments.ts  reorder.ts  uiStore.ts  compose.ts  popover.ts  calendar.ts  statusMeta.ts
│       ├── fixtures.ts  render-smoke.tsx  *.test.ts
│       └── components/{App,Toolbar,TodoCard,AddBox,Popovers,Segment}.tsx
├── assets/todo-content/{host.html,todo-content.js,todo-content.css}   # Task 3 构建产物
└── src/
    ├── assets.rs                           # Task 4：路由 + 测试
    ├── extensions/todo/protocol.rs         # Task 5（新）
    ├── extensions/todo/{mod,state,update,view}.rs   # Task 7–9
    ├── theme/geometry.rs                   # Task 6：todo_top_row_h_px
    ├── webview_geometry.rs                 # Task 6：todo_content_pane_bounds_for
    ├── app/{app,message,update,view,layout}.rs      # Task 7–9
    ├── runtime.rs                          # Task 7：IPC 路由
    └── platform/window_events.rs           # Task 7、9
```

---

### Task 0: 建 worktree 分支

- [ ] **Step 1: 建分支并确认**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add .worktrees/todo-webview -b feat/todo-webview main
cd .worktrees/todo-webview && git branch --show-current && git log --oneline -1 | cat
```
Expected: 输出 `feat/todo-webview` 与当前 main 的最近提交。此后所有命令都在 `.worktrees/todo-webview` 里执行。

---

### Task 1: 前端脚手架与桥接（可构建的空 App）

**Files:**
- Create: `crates/dozer-app/web/todo-content/{package.json,tsconfig.json,build.mjs,render-smoke.mjs}`
- Create: `crates/dozer-app/web/todo-content/src/{host.html,main.tsx,ipc.ts,protocol.ts,errors.ts,types.ts,styles.css}`
- Create: `crates/dozer-app/web/todo-content/src/components/App.tsx`
- Test: `src/protocol.test.ts`、`src/errors.test.ts`

**Interfaces:**
- Produces（供后续前端任务使用）：`types.ts` 的 `ViewPayload`、`TodoCard`、`CategoryRow`、`AgentInfo`、`TodayInfo`、`TodoState`、`SegmentKey`、`StatusFilter`、`OutEvent`；`ipc.ts` 的 `send(event: OutEvent)`；`protocol.ts` 的 `PushState`、`applyEnvelope(prev, json)`。

- [ ] **Step 1: 写 `package.json`、`tsconfig.json`**

`crates/dozer-app/web/todo-content/package.json`：

```json
{
  "name": "dozer-todo-content",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Dozer Todo 面板内容区(todo-content),Preact 离线打包,无 CDN/无运行时 Node。",
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

`tsconfig.json`：与 `web/codehealth-content/tsconfig.json` 完全相同（复制该文件）：

```bash
cp ../codehealth-content/tsconfig.json tsconfig.json
```
（在 `crates/dozer-app/web/todo-content` 目录下执行。）

- [ ] **Step 2: 写 `build.mjs`、`render-smoke.mjs`**

`build.mjs`（照抄 `codehealth-content/build.mjs`，只换目录名与产物名）：

```js
// 生产构建:把 Todo 内容区打包成离线、无 CDN、无运行时 Node 的确定性产物到
// `crates/dozer-app/assets/todo-content/`。不输出 source map;minify 后去掉
// 所有 legal comments。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/todo-content');

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
  outfile: path.join(outdir, 'todo-content.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
```

`render-smoke.mjs`（照抄 `codehealth-content/render-smoke.mjs`，只换临时目录前缀）：

```js
// 渲染冒烟:用 esbuild 把 `src/render-smoke.tsx` 打成 node 可执行的 ESM
// (JSX 不能直接被 `node --test` 的类型剥离跑),再 import 执行。
import { build } from 'esbuild';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = await mkdtemp(path.join(tmpdir(), 'todo-smoke-'));
const outfile = path.join(dir, 'smoke.mjs');

await build({
  entryPoints: [path.join(here, 'src/render-smoke.tsx')],
  bundle: true,
  format: 'esm',
  platform: 'node',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'empty' },
  outfile,
  logLevel: 'silent',
});

await import(pathToFileURL(outfile).href);
```

- [ ] **Step 3: 写 `types.ts`、`ipc.ts`**

`src/types.ts`：

```ts
export type TodoState = 'pending' | 'in_progress' | 'suspended' | 'done';
export type SegmentKey = 'active' | 'paused' | 'done';
export type StatusFilter = 'all' | TodoState;

export interface TodoCard {
  id: number;
  text: string;
  state: TodoState;
  segment: SegmentKey;
  /** "MM-DD" 或 null */
  plan_date: string | null;
  /** 已完成时的完成日期("MM-DD"),否则 null */
  completed_label: string | null;
  category_id: number | null;
  /** null = 未分类 */
  category_name: string | null;
  /** AgentKind 的 snake_case 字符串,如 "claude" */
  assigned_agent: string | null;
  has_dispatch: boolean;
}

export interface CategoryRow {
  id: number;
  name: string;
  parent_id: number | null;
  depth: number;
}

export interface AgentInfo {
  kind: string;
  label: string;
  /** Rust 内嵌的 SVG 原文(可信来源) */
  icon_svg: string;
  preserves_color: boolean;
}

export interface TodayInfo {
  year: number;
  month: number;
  day: number;
}

export interface ViewPayload {
  project_id: number;
  /** all | uncategorized | node:<id> */
  category_key: string;
  items: TodoCard[];
  categories: CategoryRow[];
  agents: AgentInfo[];
  add_height_px: number;
  today: TodayInfo;
  /** Rust 在新增后高亮的卡片 id(沿用 Flash 计时),否则 null */
  selected_id: number | null;
  /** 每次 Rust 要求"滚回顶部"时递增 */
  scroll_nonce: number;
}

export type SetStatusTarget = 'pending' | 'suspended' | 'done';

export type OutEvent =
  | { kind: 'ready' }
  | { kind: 'failed'; reason: string }
  | { kind: 'add'; text: string }
  | { kind: 'toggle'; id: number }
  | { kind: 'edit_text'; id: number; text: string }
  | { kind: 'reorder'; id: number; after_id: number | null }
  | { kind: 'set_status'; id: number; state: SetStatusTarget }
  | { kind: 'set_plan_date'; id: number; date: string }
  | { kind: 'assign_agent'; id: number; agent: string }
  | { kind: 'set_category'; id: number; category_id: number | null }
  | { kind: 'open_detail'; id: number }
  | { kind: 'add_height'; px: number };
```

`src/ipc.ts`（与 `codehealth-content/src/ipc.ts` 相同）：

```ts
import type { OutEvent } from './types.ts';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __dozer?: { dispatch(json: string): void };
  }
}

export function send(event: OutEvent): void {
  window.ipc?.postMessage(JSON.stringify(event));
}
```

- [ ] **Step 4: 写失败的测试 `protocol.test.ts`、`errors.test.ts`**

`src/protocol.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope } from './protocol.ts';

const payload = (project_id: number) => ({
  project_id,
  category_key: 'all',
  items: [],
  categories: [],
  agents: [],
  add_height_px: 60,
  today: { year: 2026, month: 10, day: 2 },
  selected_id: null,
  scroll_nonce: 0,
});
const env = (revision: number, project_id: number) =>
  JSON.stringify({ protocol_version: 1, revision, payload: payload(project_id) });

test('applies a newer revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(1, 7));
  assert.equal(s1.revision, 1);
  assert.equal(s1.payload!.project_id, 7);
});

// Review Focus 4:旧推送晚到不得覆盖新状态。
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
  assert.equal(s2.payload, null);
});
```

`src/errors.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldReportError } from './errors.ts';

test('benign ResizeObserver notice is not fatal', () => {
  assert.equal(shouldReportError({ message: 'ResizeObserver loop limit exceeded' }), false);
});

test('opaque cross-origin Script error is not fatal', () => {
  assert.equal(shouldReportError({ message: 'Script error.' }), false);
});

test('an Error thrown by our code is fatal', () => {
  assert.equal(shouldReportError({ message: 'boom', error: new Error('boom') }), true);
});

test('error attributed to our bundle is fatal', () => {
  assert.equal(
    shouldReportError({ message: 'x', filename: 'dozer://todo-content/todo-content.js' }),
    true,
  );
});

test('error from elsewhere without an Error object is ignored', () => {
  assert.equal(shouldReportError({ message: 'x', filename: 'https://example.com/a.js' }), false);
});
```

- [ ] **Step 5: 安装依赖并确认测试失败**

```bash
cd crates/dozer-app/web/todo-content && npm install 2>&1 | tail -3 && npm test 2>&1 | tail -15
```
Expected: `protocol.test.ts` 与 `errors.test.ts` 因 `Cannot find module './protocol.ts'` / `'./errors.ts'` 失败（RED）。

- [ ] **Step 6: 实现 `protocol.ts`、`errors.ts`**

`src/protocol.ts`：

```ts
import type { ViewPayload } from './types.ts';

export interface PushState {
  revision: number;
  payload: ViewPayload | null;
}

/** 应用一条 Rust 推送。revision 小于已应用值的推送(快速切换时晚到的旧响应)
 *  与无法解析/缺 payload 的推送一律忽略,状态原样返回。 */
export function applyEnvelope(prev: PushState, json: string): PushState {
  try {
    const env = JSON.parse(json) as { revision?: number; payload?: ViewPayload };
    if (!env.payload || typeof env.revision !== 'number') return prev;
    if (env.revision < prev.revision) return prev;
    return { revision: env.revision, payload: env.payload };
  } catch {
    return prev;
  }
}
```

`src/errors.ts`：

```ts
export interface ErrorLike {
  message?: string;
  error?: unknown;
  filename?: string;
}

/** 只有"确实是我们的代码抛的错"才算致命(Rust 侧会据此回落原生占位页)。
 *  浏览器的良性通知(ResizeObserver)和拿不到来源的跨域 "Script error." 不算。 */
export function shouldReportError(e: ErrorLike): boolean {
  const msg = e.message ?? '';
  if (/ResizeObserver loop/i.test(msg)) return false;
  if (msg === 'Script error.') return false;
  if (e.error instanceof Error) return true;
  return typeof e.filename === 'string' && e.filename.includes('todo-content');
}
```

- [ ] **Step 7: 写 `host.html`、`styles.css`（令牌）、`main.tsx`、空 `App.tsx`**

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
<title>todo</title>
<link rel="stylesheet" href="todo-content.css" />
</head>
<body>
<div id="root"></div>
<script src="todo-content.js"></script>
</body>
</html>
```

`src/styles.css`（令牌与 `codehealth-content` 逐值一致，本任务只放令牌与基础重置，组件样式在 Task 3 追加）：

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
  --red: #FF5C5C;
  --tab-hover: #277a85;
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
  --red: #c0392b;
  --tab-hover: #cfe9ec;
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

`src/main.tsx`：

```tsx
import { render } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import './styles.css';
import { App } from './components/App.tsx';
import { applyEnvelope, type PushState } from './protocol.ts';
import { send } from './ipc.ts';
import { shouldReportError } from './errors.ts';

// 主题由 URL `?theme=light|dark` 注入(同 dozer://codehealth-content),缺省回落 dark。
function applyThemeFromUrl() {
  const theme = new URLSearchParams(location.search).get('theme');
  document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark';
}

function Root() {
  const [state, setState] = useState<PushState>({ revision: 0, payload: null });

  useEffect(() => {
    window.__dozer = {
      dispatch(json: string) {
        setState((prev) => applyEnvelope(prev, json));
      },
    };
    const onError = (e: ErrorEvent) => {
      if (shouldReportError(e)) send({ kind: 'failed', reason: String(e.message || e.error) });
    };
    window.addEventListener('error', onError);
    // `__dozer.dispatch` 已可用,报回 Rust;Rust 收到后才开始推送。
    send({ kind: 'ready' });
    return () => {
      window.removeEventListener('error', onError);
      delete window.__dozer;
    };
  }, []);

  if (!state.payload) return null;
  return <App payload={state.payload} />;
}

applyThemeFromUrl();
render(<Root />, document.getElementById('root')!);
```

`src/components/App.tsx`（占位，Task 3 替换）：

```tsx
import type { ViewPayload } from '../types.ts';

export function App({ payload }: { payload: ViewPayload }) {
  return <div class="todo-root">{payload.items.length} 项</div>;
}
```

- [ ] **Step 8: 跑测试与类型检查确认通过**

```bash
npm test 2>&1 | tail -15
npm run typecheck 2>&1 | tail -5
```
Expected: `node --test` 全 PASS（protocol 4 个、errors 5 个）；`render-smoke.mjs` 此时会因缺 `src/render-smoke.tsx` 失败——**先创建一个最小的** `src/render-smoke.tsx`：

```tsx
import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import type { ViewPayload } from './types.ts';

const empty: ViewPayload = {
  project_id: 1,
  category_key: 'all',
  items: [],
  categories: [],
  agents: [],
  add_height_px: 60,
  today: { year: 2026, month: 10, day: 2 },
  selected_id: null,
  scroll_nonce: 0,
};

test('scaffold renders', () => {
  assert.match(render(<App payload={empty} />), /todo-root/);
});
```
然后再跑 `npm test`，Expected: 全部 PASS；`npm run typecheck` 无输出。

- [ ] **Step 9: 确认可构建并提交**

```bash
npm run build 2>&1 | tail -4; ls ../../assets/todo-content
```
Expected: 构建成功，目录里有 `host.html`、`todo-content.js`、`todo-content.css`（此处**不提交产物**，Task 3 末尾统一提交；先 `git status --short` 确认 `assets/todo-content/` 是未跟踪的）。

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/todo-webview
git branch --show-current
git add crates/dozer-app/web/todo-content
git commit -m "feat(todo): scaffold todo-content webview frontend" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
注意 `node_modules/` 必须被忽略：先 `git check-ignore crates/dozer-app/web/todo-content/node_modules && echo ignored`；若未忽略，检查其它 web 项目的 `.gitignore` 做法并照做（`git status --short` 不应出现 `node_modules`）。

---

### Task 2: 前端纯逻辑模块（TDD）

**Files（均在 `crates/dozer-app/web/todo-content/src/`）:**
- Create: `filter.ts`、`segments.ts`、`reorder.ts`、`uiStore.ts`、`compose.ts`、`popover.ts`、`calendar.ts`、`statusMeta.ts`
- Test: 同名 `*.test.ts`

**Interfaces:**
- Consumes: Task 1 的 `types.ts`。
- Produces：`visibleItems(items, search, status)`；`splitSegments`、`displayNumbers`、`formatNumber`；`afterIdForSlot`、`moveToSlot`、`slotFromY`、`isNoopMove`；`UiStore`（`get(pid)`、`onPayload(p) → { scrollToTop }`）、`ProjectUi`；`isAddSubmit`、`isInlineCommit`、`isCancel`；`placePopover`；`monthGrid`、`daysInMonth`、`firstWeekday`、`shiftMonth`、`pad2`、`formatMonthDay`、`parseMonthDay`；`STATUS_META`、`STATUS_FILTER_OPTIONS`。

- [ ] **Step 1: 写全部失败测试**

`src/filter.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { matchesSearch, visibleItems } from './filter.ts';
import type { TodoCard } from './types.ts';

const card = (id: number, text: string, state: TodoCard['state'] = 'pending'): TodoCard => ({
  id, text, state, segment: state === 'done' ? 'done' : state === 'suspended' ? 'paused' : 'active',
  plan_date: null, completed_label: null, category_id: null, category_name: null,
  assigned_agent: null, has_dispatch: false,
});

test('empty query keeps everything', () => {
  assert.equal(matchesSearch('任何文字', ''), true);
  assert.equal(matchesSearch('任何文字', '   '), true);
});

test('substring match is case-insensitive', () => {
  assert.equal(matchesSearch('给 Claude 指派生成报告', 'claude'), true);
  assert.equal(matchesSearch('修复登录页闪烁', '登录'), true);
  assert.equal(matchesSearch('修复登录页闪烁', '不存在'), false);
});

test('visibleItems intersects search and status, keeping order', () => {
  const items = [card(1, '修复登录', 'pending'), card(2, '补 README', 'done'), card(3, '修复注册', 'done')];
  assert.deepEqual(visibleItems(items, '修复', 'all').map((i) => i.id), [1, 3]);
  assert.deepEqual(visibleItems(items, '修复', 'done').map((i) => i.id), [3]);
  assert.deepEqual(visibleItems(items, '', 'pending').map((i) => i.id), [1]);
});
```

`src/segments.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { splitSegments, displayNumbers, formatNumber } from './segments.ts';
import type { TodoCard } from './types.ts';

const c = (id: number, segment: TodoCard['segment']): TodoCard => ({
  id, text: `t${id}`, state: segment === 'done' ? 'done' : segment === 'paused' ? 'suspended' : 'pending',
  segment, plan_date: null, completed_label: null, category_id: null, category_name: null,
  assigned_agent: null, has_dispatch: false,
});

test('splitSegments groups by segment and keeps relative order', () => {
  const s = splitSegments([c(1, 'done'), c(2, 'active'), c(3, 'paused'), c(4, 'active'), c(5, 'done')]);
  assert.deepEqual(s.active.map((i) => i.id), [2, 4]);
  assert.deepEqual(s.paused.map((i) => i.id), [3]);
  assert.deepEqual(s.done.map((i) => i.id), [1, 5]);
});

test('numbers run continuously active -> paused -> done, 1-based', () => {
  const s = splitSegments([c(1, 'done'), c(2, 'active'), c(3, 'paused'), c(4, 'active')]);
  const n = displayNumbers(s);
  assert.equal(n.get(2), 1);
  assert.equal(n.get(4), 2);
  assert.equal(n.get(3), 3);
  assert.equal(n.get(1), 4);
});

test('formatNumber pads to three digits', () => {
  assert.equal(formatNumber(1), '#001');
  assert.equal(formatNumber(42), '#042');
  assert.equal(formatNumber(1234), '#1234');
});
```

`src/reorder.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { afterIdForSlot, moveToSlot, slotFromY, isNoopMove } from './reorder.ts';

// Review Focus 5
test('slot 0 means before everything -> after_id null', () => {
  assert.equal(afterIdForSlot([1, 2, 3], 3, 0), null);
});

test('after_id is the visible predecessor in the list without the dragged item', () => {
  // 拖 1 到 slot 2:剩余 [2,3],插在 3 之后?slot=2 → 前驱是 rest[1]=3
  assert.equal(afterIdForSlot([1, 2, 3], 1, 2), 3);
  assert.equal(afterIdForSlot([1, 2, 3], 1, 1), 2);
  assert.equal(afterIdForSlot([1, 2, 3], 3, 1), 1);
});

test('slot is clamped into range', () => {
  assert.equal(afterIdForSlot([1, 2, 3], 1, 99), 3);
  assert.equal(afterIdForSlot([1, 2, 3], 1, -5), null);
});

test('moveToSlot gives the preview order', () => {
  assert.deepEqual(moveToSlot([1, 2, 3], 3, 0), [3, 1, 2]);
  assert.deepEqual(moveToSlot([1, 2, 3], 1, 2), [2, 3, 1]);
});

test('dropping back at the original position is a no-op', () => {
  assert.equal(isNoopMove([1, 2, 3], 2, 1), true);
  assert.equal(isNoopMove([1, 2, 3], 2, 0), false);
});

test('slotFromY counts rows whose midpoint is above the pointer, excluding the dragged one', () => {
  const rects = [
    { id: 1, top: 0, bottom: 40 },
    { id: 2, top: 50, bottom: 90 },
    { id: 3, top: 100, bottom: 140 },
  ];
  assert.equal(slotFromY(rects, 1, 5), 0); // 在最上方
  assert.equal(slotFromY(rects, 1, 100), 1); // 越过 2 的中线(70)但没越过 3 的中线(120)
  assert.equal(slotFromY(rects, 1, 500), 2); // 越过所有
});
```

`src/uiStore.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { UiStore } from './uiStore.ts';
import type { ViewPayload } from './types.ts';

const p = (over: Partial<ViewPayload>): ViewPayload => ({
  project_id: 1, category_key: 'all', items: [], categories: [], agents: [],
  add_height_px: 60, today: { year: 2026, month: 10, day: 2 },
  selected_id: null, scroll_nonce: 0, ...over,
});

// Review Focus 3
test('state is kept per project across switches', () => {
  const s = new UiStore();
  s.onPayload(p({ project_id: 1 }));
  s.get(1).addDraft = '写到一半的任务';
  s.get(1).search = '登录';
  s.onPayload(p({ project_id: 2 }));
  assert.equal(s.get(2).addDraft, '');
  s.onPayload(p({ project_id: 1 }));
  assert.equal(s.get(1).addDraft, '写到一半的任务');
  assert.equal(s.get(1).search, '登录');
});

test('category change clears search but keeps the add draft', () => {
  const s = new UiStore();
  s.onPayload(p({ category_key: 'all' }));
  s.get(1).search = 'x';
  s.get(1).searchDraft = 'x';
  s.get(1).addDraft = '草稿';
  s.onPayload(p({ category_key: 'node:7' }));
  assert.equal(s.get(1).search, '');
  assert.equal(s.get(1).searchDraft, '');
  assert.equal(s.get(1).addDraft, '草稿');
});

test('first payload for a project does not clear search', () => {
  const s = new UiStore();
  s.get(1).search = 'keep';
  s.onPayload(p({ category_key: 'node:3' }));
  assert.equal(s.get(1).search, 'keep');
});

test('selected_id from Rust overrides local selection only when it changes', () => {
  const s = new UiStore();
  s.onPayload(p({ selected_id: 5 }));
  assert.equal(s.get(1).selectedId, 5);
  s.get(1).selectedId = 9; // 用户点了别的卡片
  s.onPayload(p({ selected_id: 5 })); // 同一个值再次推送,不应覆盖用户的选择
  assert.equal(s.get(1).selectedId, 9);
  s.onPayload(p({ selected_id: null })); // Rust 的 2 秒高亮到点清除
  assert.equal(s.get(1).selectedId, null);
});

test('scroll_nonce change requests scroll-to-top exactly once', () => {
  const s = new UiStore();
  assert.equal(s.onPayload(p({ scroll_nonce: 0 })).scrollToTop, false);
  assert.equal(s.onPayload(p({ scroll_nonce: 1 })).scrollToTop, true);
  assert.equal(s.onPayload(p({ scroll_nonce: 1 })).scrollToTop, false);
});
```

`src/compose.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { isAddSubmit, isInlineCommit, isCancel } from './compose.ts';

const key = (over: object) => ({
  key: 'Enter', metaKey: false, ctrlKey: false, shiftKey: false, isComposing: false, ...over,
});

// Review Focus 2:输入法组合输入期间绝不提交。
test('add box: cmd/ctrl+enter submits, plain enter does not', () => {
  assert.equal(isAddSubmit(key({ metaKey: true })), true);
  assert.equal(isAddSubmit(key({ ctrlKey: true })), true);
  assert.equal(isAddSubmit(key({})), false);
});

test('add box: never submits while composing', () => {
  assert.equal(isAddSubmit(key({ metaKey: true, isComposing: true })), false);
  assert.equal(isAddSubmit(key({ metaKey: true, keyCode: 229 })), false);
});

test('inline edit: enter commits, but not while composing', () => {
  assert.equal(isInlineCommit(key({})), true);
  assert.equal(isInlineCommit(key({ isComposing: true })), false);
  assert.equal(isInlineCommit(key({ keyCode: 229 })), false);
  assert.equal(isInlineCommit(key({ key: 'a' })), false);
});

test('escape cancels, but not while composing', () => {
  assert.equal(isCancel(key({ key: 'Escape' })), true);
  assert.equal(isCancel(key({ key: 'Escape', isComposing: true })), false);
});
```

`src/popover.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { placePopover } from './popover.ts';

const vp = { w: 800, h: 600 };

test('opens below the anchor when there is room', () => {
  const p = placePopover({ left: 100, top: 100, right: 160, bottom: 124 }, { w: 120, h: 80 }, vp);
  assert.deepEqual(p, { x: 100, y: 128 });
});

test('flips above when there is no room below', () => {
  const p = placePopover({ left: 100, top: 560, right: 160, bottom: 584 }, { w: 120, h: 80 }, vp);
  assert.equal(p.y, 560 - 80 - 4);
});

test('is clamped horizontally into the viewport (never leaves the webview rect)', () => {
  const p = placePopover({ left: 780, top: 100, right: 800, bottom: 124 }, { w: 200, h: 80 }, vp);
  assert.equal(p.x, 800 - 200 - 4);
  const q = placePopover({ left: -30, top: 100, right: 0, bottom: 124 }, { w: 200, h: 80 }, vp);
  assert.equal(q.x, 4);
});
```

`src/calendar.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { daysInMonth, firstWeekday, monthGrid, shiftMonth, formatMonthDay, parseMonthDay } from './calendar.ts';

test('daysInMonth handles leap years', () => {
  assert.equal(daysInMonth(2024, 2), 29);
  assert.equal(daysInMonth(2023, 2), 28);
  assert.equal(daysInMonth(2100, 2), 28);
  assert.equal(daysInMonth(2000, 2), 29);
  assert.equal(daysInMonth(2026, 10), 31);
  assert.equal(daysInMonth(2026, 11), 30);
});

test('firstWeekday is Sunday-based (1970-01-01 was a Thursday)', () => {
  assert.equal(firstWeekday(1970, 1), 4);
  assert.equal(firstWeekday(2026, 10), 4);
});

test('monthGrid pads leading blanks and lists every day', () => {
  const g = monthGrid(2026, 10);
  assert.equal(g.slice(0, 4).every((c) => c === null), true);
  assert.equal(g[4], 1);
  assert.equal(g.filter((c) => c !== null).length, 31);
  assert.equal(g.length % 7, 0);
});

test('shiftMonth wraps the year', () => {
  assert.deepEqual(shiftMonth(2026, 1, -1), { y: 2025, m: 12 });
  assert.deepEqual(shiftMonth(2026, 12, 1), { y: 2027, m: 1 });
  assert.deepEqual(shiftMonth(2026, 5, 1), { y: 2026, m: 6 });
});

test('MM-DD formatting and parsing round trip', () => {
  assert.equal(formatMonthDay(9, 5), '09-05');
  assert.deepEqual(parseMonthDay('09-05'), { m: 9, d: 5 });
  assert.equal(parseMonthDay('13-01'), null);
  assert.equal(parseMonthDay('abc'), null);
  assert.equal(parseMonthDay(null), null);
});
```

`src/statusMeta.test.ts`：

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { STATUS_META, STATUS_FILTER_OPTIONS } from './statusMeta.ts';

test('labels and colors match the native implementation', () => {
  assert.deepEqual(STATUS_META.pending, { label: '待办', color: 'var(--cyan)' });
  assert.deepEqual(STATUS_META.in_progress, { label: '进行中', color: 'var(--gold)' });
  assert.deepEqual(STATUS_META.suspended, { label: '搁置', color: 'var(--cream)' });
  assert.deepEqual(STATUS_META.done, { label: '已完成', color: 'var(--dim)' });
});

test('status filter offers all plus the four states in order', () => {
  assert.deepEqual(STATUS_FILTER_OPTIONS, ['all', 'pending', 'in_progress', 'suspended', 'done']);
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd crates/dozer-app/web/todo-content && node --test src/*.test.ts 2>&1 | tail -20`
Expected: 8 个新测试文件均因 `Cannot find module` 失败（RED）；`protocol.test.ts`、`errors.test.ts` 仍通过。

- [ ] **Step 3: 实现全部模块**

`src/filter.ts`：

```ts
import type { StatusFilter, TodoCard } from './types.ts';

/** 大小写不敏感的子串匹配;空/全空白查询匹配一切(同 Rust `filter_todos`)。 */
export function matchesSearch(text: string, query: string): boolean {
  const q = query.trim().toLowerCase();
  return q === '' || text.toLowerCase().includes(q);
}

/** 关键词 ∩ 状态,保持原顺序。分类过滤已由 Rust 在推送前完成。 */
export function visibleItems(items: TodoCard[], search: string, status: StatusFilter): TodoCard[] {
  return items.filter(
    (it) => matchesSearch(it.text, search) && (status === 'all' || it.state === status),
  );
}
```

`src/segments.ts`：

```ts
import type { TodoCard } from './types.ts';

export interface Segments {
  active: TodoCard[];
  paused: TodoCard[];
  done: TodoCard[];
}

export function splitSegments(items: TodoCard[]): Segments {
  const s: Segments = { active: [], paused: [], done: [] };
  for (const it of items) s[it.segment].push(it);
  return s;
}

/** `#NNN` 是"当前可见卡片的显示序号":跨三段连续递增(进行中 → 搁置 → 已完成),
 *  从 1 起,随搜索与状态筛选变化(同原生的 `shown` 计数)。 */
export function displayNumbers(s: Segments): Map<number, number> {
  const m = new Map<number, number>();
  let n = 0;
  for (const it of [...s.active, ...s.paused, ...s.done]) m.set(it.id, ++n);
  return m;
}

export function formatNumber(n: number): string {
  return `#${String(n).padStart(3, '0')}`;
}
```

`src/reorder.ts`：

```ts
export interface RowRect {
  id: number;
  top: number;
  bottom: number;
}

function clampSlot(restLen: number, slot: number): number {
  return Math.max(0, Math.min(slot, restLen));
}

/** `slot` 是拖动项在"去掉它之后的列表"里的插入位置(0..=len)。
 *  返回应发给 Rust 的 `after_id`:放置位置上方那张可见卡片的 id,`null` = 放到最前。
 *  与 `Client::reorder_todo(id, after_id)` 语义一致。 */
export function afterIdForSlot(ids: number[], draggedId: number, slot: number): number | null {
  const rest = ids.filter((id) => id !== draggedId);
  const s = clampSlot(rest.length, slot);
  return s === 0 ? null : rest[s - 1];
}

/** 拖动期间的预览顺序。 */
export function moveToSlot(ids: number[], draggedId: number, slot: number): number[] {
  const rest = ids.filter((id) => id !== draggedId);
  const s = clampSlot(rest.length, slot);
  return [...rest.slice(0, s), draggedId, ...rest.slice(s)];
}

/** 放回原位:不发事件。 */
export function isNoopMove(ids: number[], draggedId: number, slot: number): boolean {
  const moved = moveToSlot(ids, draggedId, slot);
  return moved.length === ids.length && moved.every((id, i) => id === ids[i]);
}

/** 指针 y 对应的插入位置:越过其它行中线的行数(不含被拖动那行)。 */
export function slotFromY(rects: RowRect[], draggedId: number, y: number): number {
  let slot = 0;
  for (const r of rects) {
    if (r.id === draggedId) continue;
    if (y > (r.top + r.bottom) / 2) slot += 1;
    else break;
  }
  return slot;
}
```

`src/uiStore.ts`：

```ts
import type { StatusFilter, ViewPayload } from './types.ts';

/** 每个项目各自保留的纯前端视图状态(不回传 Rust)。 */
export interface ProjectUi {
  /** 已生效的搜索词(回车 / 点按钮 / 失焦提交后) */
  search: string;
  /** 搜索框草稿 */
  searchDraft: string;
  status: StatusFilter;
  addDraft: string;
  /** 新增框当前高度(px);null = 用 payload.add_height_px */
  addHeight: number | null;
  /** 上一次见到的 category_key;null = 还没收到过推送 */
  categoryKey: string | null;
  selectedId: number | null;
  /** 上一次从 Rust 收到的 selected_id,用来判断"是否变化" */
  lastRustSelected: number | null;
  scrollNonce: number;
  scrollTop: number;
}

function fresh(): ProjectUi {
  return {
    search: '',
    searchDraft: '',
    status: 'all',
    addDraft: '',
    addHeight: null,
    categoryKey: null,
    selectedId: null,
    lastRustSelected: null,
    scrollNonce: 0,
    scrollTop: 0,
  };
}

export class UiStore {
  private m = new Map<number, ProjectUi>();

  get(projectId: number): ProjectUi {
    let ui = this.m.get(projectId);
    if (!ui) {
      ui = fresh();
      this.m.set(projectId, ui);
    }
    return ui;
  }

  /** 应用一条推送带来的、需要前端状态响应的变化。返回是否应滚回顶部。 */
  onPayload(p: ViewPayload): { scrollToTop: boolean } {
    const ui = this.get(p.project_id);
    // 分类切换:清搜索(草稿与生效词),沿用原生 `clear_search` 语义;不动新增草稿。
    if (ui.categoryKey !== null && ui.categoryKey !== p.category_key) {
      ui.search = '';
      ui.searchDraft = '';
    }
    ui.categoryKey = p.category_key;
    // Rust 的 selected_id 只在它"变化"时覆盖本地选择(新增后 2 秒高亮)。
    if (p.selected_id !== ui.lastRustSelected) {
      ui.selectedId = p.selected_id;
      ui.lastRustSelected = p.selected_id;
    }
    let scrollToTop = false;
    if (p.scroll_nonce !== ui.scrollNonce) {
      ui.scrollNonce = p.scroll_nonce;
      scrollToTop = true;
    }
    return { scrollToTop };
  }
}
```

`src/compose.ts`：

```ts
export interface KeyLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
  keyCode?: number;
}

/** 输入法组合输入期间(含 Safari 的 keyCode 229)的按键一律不当作提交/取消。 */
function composing(e: KeyLike): boolean {
  return e.isComposing || e.keyCode === 229;
}

/** 新增框:⌘↵ / Ctrl+↵ 提交;普通回车是换行,不提交。 */
export function isAddSubmit(e: KeyLike): boolean {
  return e.key === 'Enter' && (e.metaKey || e.ctrlKey) && !composing(e);
}

/** 内联编辑:回车提交(现有行为),组合输入期间不提交。 */
export function isInlineCommit(e: KeyLike): boolean {
  return e.key === 'Enter' && !composing(e);
}

export function isCancel(e: KeyLike): boolean {
  return e.key === 'Escape' && !composing(e);
}
```

`src/popover.ts`：

```ts
export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

const GAP = 4;

/** 弹层定位:默认在锚点下方左对齐;下方放不下就翻到上方;横向夹在视口内,
 *  保证弹层不超出 webview 矩形。 */
export function placePopover(
  anchor: Rect,
  size: { w: number; h: number },
  viewport: { w: number; h: number },
): { x: number; y: number } {
  let y = anchor.bottom + GAP;
  if (y + size.h > viewport.h - GAP) {
    const above = anchor.top - size.h - GAP;
    y = above >= GAP ? above : Math.max(GAP, viewport.h - size.h - GAP);
  }
  let x = anchor.left;
  if (x + size.w > viewport.w - GAP) x = viewport.w - size.w - GAP;
  if (x < GAP) x = GAP;
  return { x, y };
}
```

`src/calendar.ts`：

```ts
export const pad2 = (n: number): string => String(n).padStart(2, '0');

export function daysInMonth(y: number, m: number): number {
  return new Date(y, m, 0).getDate();
}

/** 周日为 0(同原生 `first_weekday_of_month`)。 */
export function firstWeekday(y: number, m: number): number {
  return new Date(y, m - 1, 1).getDay();
}

/** 拍平的月历格:前导 null 补到该月 1 日的星期,末尾补满整周。 */
export function monthGrid(y: number, m: number): (number | null)[] {
  const cells: (number | null)[] = Array(firstWeekday(y, m)).fill(null);
  for (let d = 1; d <= daysInMonth(y, m); d++) cells.push(d);
  while (cells.length % 7 !== 0) cells.push(null);
  return cells;
}

export function shiftMonth(y: number, m: number, delta: number): { y: number; m: number } {
  const idx = y * 12 + (m - 1) + delta;
  return { y: Math.floor(idx / 12), m: (idx % 12) + 1 };
}

export function formatMonthDay(m: number, d: number): string {
  return `${pad2(m)}-${pad2(d)}`;
}

export function parseMonthDay(s: string | null): { m: number; d: number } | null {
  if (!s) return null;
  const parts = s.split('-');
  if (parts.length !== 2) return null;
  const m = Number(parts[0]);
  const d = Number(parts[1]);
  if (!Number.isInteger(m) || !Number.isInteger(d) || m < 1 || m > 12 || d < 1 || d > 31) return null;
  return { m, d };
}
```

`src/statusMeta.ts`：

```ts
import type { StatusFilter, TodoState } from './types.ts';

export const STATUS_META: Record<TodoState, { label: string; color: string }> = {
  pending: { label: '待办', color: 'var(--cyan)' },
  in_progress: { label: '进行中', color: 'var(--gold)' },
  suspended: { label: '搁置', color: 'var(--cream)' },
  done: { label: '已完成', color: 'var(--dim)' },
};

export const STATUS_FILTER_OPTIONS: StatusFilter[] = [
  'all',
  'pending',
  'in_progress',
  'suspended',
  'done',
];

export function statusFilterLabel(f: StatusFilter): string {
  return f === 'all' ? '全部' : STATUS_META[f].label;
}
```

- [ ] **Step 4: 跑测试与类型检查确认通过**

```bash
node --test src/*.test.ts 2>&1 | tail -12
npm run typecheck 2>&1 | tail -5
```
Expected: 全部 PASS，无类型错误。若 `slotFromY` 用例 `slotFromY(rects, 1, 100)` 失败，检查中线：行 2 中线 70、行 3 中线 120，y=100 越过行 2（slot=1）不越过行 3，应为 1。

- [ ] **Step 5: 提交**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/todo-webview
git branch --show-current
git add crates/dozer-app/web/todo-content/src
git commit -m "feat(todo): add pure logic modules for todo-content (search, segments, reorder, per-project ui state)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 前端组件、样式、渲染冒烟，并构建提交产物

**Files（均在 `crates/dozer-app/web/todo-content/src/`）:**
- Modify: `components/App.tsx`、`styles.css`、`render-smoke.tsx`
- Create: `components/{Toolbar,TodoCard,AddBox,Popovers,Segment}.tsx`、`fixtures.ts`
- Modify（构建产物）: `crates/dozer-app/assets/todo-content/*`

**Interfaces:**
- Consumes: Task 1、2 的全部导出。
- Produces: `App({ payload })`（根组件，通过 `send` 发 `OutEvent`）。

- [ ] **Step 1: 写 `fixtures.ts`（渲染冒烟与后续手工对照共用）**

```ts
import type { TodoCard, ViewPayload } from './types.ts';

const agents = [
  { kind: 'claude', label: 'Claude', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
  { kind: 'codebuddy', label: 'CodeBuddy', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
  { kind: 'opencode', label: 'OpenCode', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
  { kind: 'v8agent', label: 'v8agent', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
];

export const card = (id: number, text: string, over: Partial<TodoCard> = {}): TodoCard => ({
  id, text, state: 'pending', segment: 'active', plan_date: null, completed_label: null,
  category_id: null, category_name: null, assigned_agent: null, has_dispatch: false, ...over,
});

const base = (items: TodoCard[]): ViewPayload => ({
  project_id: 1,
  category_key: 'all',
  items,
  categories: [
    { id: 10, name: '后端', parent_id: null, depth: 0 },
    { id: 11, name: '接口', parent_id: 10, depth: 1 },
    { id: 12, name: '前端', parent_id: null, depth: 0 },
  ],
  agents,
  add_height_px: 60,
  today: { year: 2026, month: 10, day: 2 },
  selected_id: null,
  scroll_nonce: 0,
});

export const emptyFixture = base([]);

export const threeSegmentsFixture = base([
  card(1, '修复登录页闪烁', { plan_date: '10-05', category_id: 10, category_name: '后端' }),
  card(2, '给 claude 指派生成报告', { state: 'in_progress', has_dispatch: true, assigned_agent: 'claude' }),
  card(3, '等设计稿确认', { state: 'suspended', segment: 'paused' }),
  card(4, '补 README 安装说明', { state: 'done', segment: 'done', completed_label: '09-30' }),
]);

export const allDoneFixture = base([
  card(1, '已完成 A', { state: 'done', segment: 'done', completed_label: '10-01' }),
  card(2, '已完成 B', { state: 'done', segment: 'done', completed_label: '-' }),
]);

export const longTextFixture = base([
  card(1, '这是一段非常非常长的任务文字,'.repeat(12) + '用来确认卡片在窄宽度下会自然换行而不是撑破布局。'),
]);

export const englishFixture = base([card(1, 'Fix the flaky login test on CI')]);
```

- [ ] **Step 2: 写失败的渲染冒烟 `render-smoke.tsx`**

```tsx
import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import {
  emptyFixture,
  threeSegmentsFixture,
  allDoneFixture,
  longTextFixture,
  englishFixture,
} from './fixtures.ts';
import type { ViewPayload } from './types.ts';

const html = (p: ViewPayload) => render(<App payload={p} />);

test('empty list shows the no-match hint, toolbar and add box', () => {
  const out = html(emptyFixture);
  assert.match(out, /没有匹配的任务/);
  assert.match(out, /搜索任务…/);
  assert.match(out, /添加新任务/);
});

test('three segments show dividers, numbers and status labels', () => {
  const out = html(threeSegmentsFixture);
  assert.match(out, /#001/);
  assert.match(out, /#004/);
  assert.match(out, /搁置/);
  assert.match(out, /已完成/);
  assert.match(out, /进行中/);
  assert.match(out, /修复登录页闪烁/);
});

test('uncategorized cards show 未分类, categorized show their name', () => {
  const out = html(threeSegmentsFixture);
  assert.match(out, /未分类/);
  assert.match(out, /后端/);
});

test('detail button appears only with an assignment or dispatch; assign button only for idle pending', () => {
  const out = html(threeSegmentsFixture);
  assert.match(out, /详情/); // 任务 2 已派发
  assert.match(out, /指派/); // 任务 1 是待办且无派发
});

test('done cards show completion date and strikethrough class', () => {
  const out = html(allDoneFixture);
  assert.match(out, /10-01/);
  assert.match(out, /is-done/);
});

test('long and english text render', () => {
  assert.match(html(longTextFixture), /非常非常长/);
  assert.match(html(englishFixture), /Fix the flaky login test/);
});

test('no popover is rendered initially', () => {
  assert.doesNotMatch(html(threeSegmentsFixture), /class="popover/);
});
```

Run: `node render-smoke.mjs 2>&1 | tail -15`
Expected: 失败（RED）：`App` 还是占位实现，找不到 `没有匹配的任务` 等。

- [ ] **Step 3: 实现组件**

`src/components/Segment.tsx`：

```tsx
import type { ComponentChildren } from 'preact';

export function SegmentDivider({ label }: { label: string }) {
  return (
    <div class="segment-divider">
      <span class="segment-divider-label">{label}</span>
      <span class="segment-divider-line" />
    </div>
  );
}

export function EmptyHint({ children }: { children: ComponentChildren }) {
  return <div class="empty-hint">{children}</div>;
}
```

`src/components/Toolbar.tsx`（搜索条 + 状态筛选；回车 / 点按钮 / 失焦提交搜索）：

```tsx
import type { StatusFilter } from '../types.ts';
import { statusFilterLabel, STATUS_META } from '../statusMeta.ts';
import { isInlineCommit } from '../compose.ts';

interface Props {
  searchDraft: string;
  highlight: boolean;
  status: StatusFilter;
  onDraft(v: string): void;
  onSubmit(): void;
  onOpenStatusFilter(anchor: DOMRect): void;
}

export function Toolbar(p: Props) {
  const color = p.status === 'all' ? 'var(--cream)' : STATUS_META[p.status].color;
  return (
    <div class={`toolbar${p.highlight ? ' is-highlight' : ''}`}>
      <button
        type="button"
        class="status-segment"
        style={{ color }}
        onClick={(e) => p.onOpenStatusFilter((e.currentTarget as HTMLElement).getBoundingClientRect())}
      >
        <span>{statusFilterLabel(p.status)}</span>
        <span class="chevron" aria-hidden="true">▾</span>
      </button>
      <input
        class="search-input"
        type="text"
        placeholder="搜索任务…"
        value={p.searchDraft}
        onInput={(e) => p.onDraft((e.currentTarget as HTMLInputElement).value)}
        onKeyDown={(e) => {
          if (isInlineCommit(e)) {
            e.preventDefault();
            p.onSubmit();
          }
        }}
        onBlur={() => p.onSubmit()}
      />
      <button type="button" class="icon-btn" aria-label="搜索" onClick={() => p.onSubmit()}>
        ⌕
      </button>
    </div>
  );
}
```

`src/components/TodoCard.tsx`（单张卡片）：

```tsx
import { useEffect, useRef } from 'preact/hooks';
import type { TodoCard as Card } from '../types.ts';
import { STATUS_META } from '../statusMeta.ts';
import { formatNumber } from '../segments.ts';
import { isCancel, isInlineCommit } from '../compose.ts';

export interface CardProps {
  item: Card;
  number: number;
  selected: boolean;
  draggable: boolean;
  dragging: boolean;
  editing: boolean;
  onToggle(): void;
  onSelect(): void;
  onBeginEdit(): void;
  onCommitEdit(text: string): void;
  onCancelEdit(): void;
  onOpenCategory(anchor: DOMRect): void;
  onOpenCalendar(anchor: DOMRect): void;
  onOpenStatus(anchor: DOMRect): void;
  onOpenDispatch(anchor: DOMRect): void;
  onOpenDetail(): void;
  onPointerDown(e: PointerEvent): void;
}

const rectOf = (e: Event) => (e.currentTarget as HTMLElement).getBoundingClientRect();

export function TodoCard(p: CardProps) {
  const { item } = p;
  const done = item.state === 'done';
  const meta = STATUS_META[item.state];
  const dateLabel = done ? item.completed_label ?? '-' : item.plan_date ?? '-';
  const showDetail = item.assigned_agent !== null || item.has_dispatch;
  const showAssign = item.state === 'pending' && !item.has_dispatch;
  const taRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (p.editing && taRef.current) {
      const el = taRef.current;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    }
  }, [p.editing]);

  return (
    <div
      class={`todo-card${done ? ' is-done' : ''}${p.selected ? ' is-selected' : ''}${p.dragging ? ' is-dragging' : ''}${p.draggable ? ' is-draggable' : ''}`}
      data-id={item.id}
      onPointerDown={(e) => {
        p.onSelect();
        p.onPointerDown(e as unknown as PointerEvent);
      }}
    >
      <div class="card-top">
        <span class="card-number">{formatNumber(p.number)}</span>
        <button type="button" class="chip" onClick={(e) => p.onOpenCategory(rectOf(e))}>
          {item.category_name ?? '未分类'}
        </button>
        <span class="spacer" />
        <button type="button" class="date-badge" onClick={(e) => p.onOpenCalendar(rectOf(e))}>
          <span aria-hidden="true">▦</span>
          <span>{dateLabel}</span>
        </button>
      </div>
      <div class="card-body">
        <button
          type="button"
          class={`checkbox${done ? ' is-checked' : ''}`}
          aria-label="切换完成"
          onClick={() => p.onToggle()}
        >
          {done ? '✓' : ''}
        </button>
        {p.editing ? (
          <textarea
            ref={taRef}
            class="edit-area"
            placeholder="任务内容…"
            defaultValue={item.text}
            rows={1}
            onKeyDown={(e) => {
              if (isInlineCommit(e)) {
                e.preventDefault();
                p.onCommitEdit((e.currentTarget as HTMLTextAreaElement).value);
              } else if (isCancel(e)) {
                e.preventDefault();
                p.onCancelEdit();
              }
            }}
            onBlur={(e) => p.onCommitEdit((e.currentTarget as HTMLTextAreaElement).value)}
          />
        ) : (
          <div class="card-text" onClick={() => p.onBeginEdit()}>
            {item.text}
          </div>
        )}
      </div>
      <div class="card-bottom">
        <button
          type="button"
          class="status-btn"
          style={{ color: meta.color }}
          onClick={(e) => p.onOpenStatus(rectOf(e))}
        >
          <span>{meta.label}</span>
          <span class="chevron" aria-hidden="true">▾</span>
        </button>
        {showDetail ? (
          <button type="button" class="detail-btn" onClick={() => p.onOpenDetail()}>
            详情
          </button>
        ) : null}
        {showAssign ? (
          <button type="button" class="assign-btn" onClick={(e) => p.onOpenDispatch(rectOf(e))}>
            <span>指派</span>
            <span aria-hidden="true">›</span>
          </button>
        ) : null}
      </div>
    </div>
  );
}
```

`src/components/AddBox.tsx`（新增框：Enter 换行、⌘↵ 提交、「↑」提交、可拖拽高度）：

```tsx
import { useRef } from 'preact/hooks';
import { isAddSubmit } from '../compose.ts';

const MIN_H = 44;
const MAX_H = 360;

interface Props {
  draft: string;
  height: number;
  onDraft(v: string): void;
  onSubmit(): void;
  onHeight(px: number, commit: boolean): void;
}

export function AddBox(p: Props) {
  const startRef = useRef<{ y: number; h: number } | null>(null);
  const clamp = (h: number) => Math.max(MIN_H, Math.min(MAX_H, h));

  const onHandleDown = (e: PointerEvent) => {
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    startRef.current = { y: e.clientY, h: p.height };
  };
  const onHandleMove = (e: PointerEvent) => {
    const s = startRef.current;
    if (!s) return;
    // 向上拉增高(同原生:框顶的拖拽手柄向上拉)
    p.onHeight(clamp(s.h + (s.y - e.clientY)), false);
  };
  const onHandleUp = (e: PointerEvent) => {
    const s = startRef.current;
    startRef.current = null;
    if (s) p.onHeight(clamp(s.h + (s.y - e.clientY)), true);
  };

  return (
    <div class="add-box">
      <div
        class="add-resize-handle"
        onPointerDown={onHandleDown}
        onPointerMove={onHandleMove}
        onPointerUp={onHandleUp}
      />
      <div class="add-row">
        <textarea
          class="add-input"
          style={{ height: `${p.height}px` }}
          placeholder="添加新任务"
          value={p.draft}
          onInput={(e) => p.onDraft((e.currentTarget as HTMLTextAreaElement).value)}
          onKeyDown={(e) => {
            if (isAddSubmit(e)) {
              e.preventDefault();
              p.onSubmit();
            }
          }}
        />
        <button type="button" class="add-submit" aria-label="添加" onClick={() => p.onSubmit()}>
          ↑
        </button>
      </div>
    </div>
  );
}
```

`src/components/Popovers.tsx`（五个弹层共用一个浮层容器）：

```tsx
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import type { AgentInfo, CategoryRow, StatusFilter, SetStatusTarget } from '../types.ts';
import { STATUS_META, STATUS_FILTER_OPTIONS, statusFilterLabel } from '../statusMeta.ts';
import { placePopover, type Rect } from '../popover.ts';
import { formatMonthDay, monthGrid, parseMonthDay, shiftMonth } from '../calendar.ts';

export type PopoverState =
  | { kind: 'status'; id: number; anchor: Rect; current: string }
  | { kind: 'dispatch'; id: number; anchor: Rect }
  | { kind: 'calendar'; id: number; anchor: Rect; planDate: string | null }
  | { kind: 'statusFilter'; anchor: Rect }
  | { kind: 'category'; id: number; anchor: Rect; currentId: number | null };

interface Props {
  state: PopoverState;
  agents: AgentInfo[];
  categories: CategoryRow[];
  today: { year: number; month: number; day: number };
  statusFilter: StatusFilter;
  onClose(): void;
  onPickStatus(id: number, target: SetStatusTarget | 'in_progress'): void;
  onPickAgent(id: number, agent: string): void;
  onPickDate(id: number, date: string): void;
  onPickStatusFilter(f: StatusFilter): void;
  onPickCategory(id: number, categoryId: number | null): void;
}

export function Popover(p: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number }>({ x: 0, y: 0 });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setPos(
      placePopover(
        p.state.anchor,
        { w: el.offsetWidth, h: el.offsetHeight },
        { w: window.innerWidth, h: window.innerHeight },
      ),
    );
  }, [p.state]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.isComposing) p.onClose();
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('resize', p.onClose);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', p.onClose);
    };
  }, [p.onClose]);

  return (
    <div class="popover-layer" onPointerDown={() => p.onClose()}>
      <div
        ref={ref}
        class={`popover popover-${p.state.kind}`}
        style={{ left: `${pos.x}px`, top: `${pos.y}px` }}
        onPointerDown={(e) => e.stopPropagation()}
      >
        {renderBody(p)}
      </div>
    </div>
  );
}

function renderBody(p: Props) {
  const s = p.state;
  switch (s.kind) {
    case 'status':
      return (['pending', 'in_progress', 'suspended', 'done'] as const).map((st) => (
        <button
          type="button"
          class="menu-item"
          style={{ color: STATUS_META[st].color }}
          onClick={() => p.onPickStatus(s.id, st)}
        >
          {STATUS_META[st].label}
        </button>
      ));
    case 'dispatch':
      return p.agents.map((a) => (
        <button type="button" class="menu-item" onClick={() => p.onPickAgent(s.id, a.kind)}>
          <span
            class={`agent-icon${a.preserves_color ? ' keep-color' : ''}`}
            dangerouslySetInnerHTML={{ __html: a.icon_svg }}
          />
          <span>{a.label}</span>
        </button>
      ));
    case 'statusFilter':
      return STATUS_FILTER_OPTIONS.map((f) => (
        <button
          type="button"
          class={`menu-item${p.statusFilter === f ? ' is-current' : ''}`}
          style={{ color: f === 'all' ? 'var(--cream)' : STATUS_META[f].color }}
          onClick={() => p.onPickStatusFilter(f)}
        >
          {statusFilterLabel(f)}
        </button>
      ));
    case 'category':
      return (
        <>
          <button type="button" class={`menu-item${s.currentId === null ? ' is-current' : ''}`} onClick={() => p.onPickCategory(s.id, null)}>
            未分类
          </button>
          {p.categories.map((c) => (
            <button
              type="button"
              class={`menu-item${s.currentId === c.id ? ' is-current' : ''}`}
              style={{ paddingLeft: `${10 + c.depth * 14}px` }}
              onClick={() => p.onPickCategory(s.id, c.id)}
            >
              {c.name}
            </button>
          ))}
        </>
      );
    case 'calendar':
      return <Calendar id={s.id} planDate={s.planDate} today={p.today} onPick={p.onPickDate} />;
  }
}

function Calendar(props: {
  id: number;
  planDate: string | null;
  today: { year: number; month: number; day: number };
  onPick(id: number, date: string): void;
}) {
  const planned = parseMonthDay(props.planDate);
  // 初始月份:有计划日期用其月份(年取今年),否则今年今月(同原生)。
  const [ym, setYm] = useState({ y: props.today.year, m: planned ? planned.m : props.today.month });
  // 高亮:有计划日期高亮它,否则高亮今天(同原生 `selected_md`)。
  const sel = planned ?? { m: props.today.month, d: props.today.day };
  const cells = monthGrid(ym.y, ym.m);
  return (
    <div class="calendar">
      <div class="calendar-head">
        <button type="button" class="icon-btn" onClick={() => setYm(shiftMonth(ym.y, ym.m, -1))}>
          ‹
        </button>
        <span class="calendar-title">{`${ym.y}-${String(ym.m).padStart(2, '0')}`}</span>
        <button type="button" class="icon-btn" onClick={() => setYm(shiftMonth(ym.y, ym.m, 1))}>
          ›
        </button>
      </div>
      <div class="calendar-grid">
        {['日', '一', '二', '三', '四', '五', '六'].map((w) => (
          <span class="calendar-weekday">{w}</span>
        ))}
        {cells.map((d) =>
          d === null ? (
            <span class="calendar-cell empty" />
          ) : (
            <button
              type="button"
              class={`calendar-cell${sel.m === ym.m && sel.d === d ? ' is-selected' : ''}`}
              onClick={() => props.onPick(props.id, formatMonthDay(ym.m, d))}
            >
              {d}
            </button>
          ),
        )}
      </div>
    </div>
  );
}
```

`src/components/App.tsx`（组装；拖拽排序用指针事件）：

```tsx
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ViewPayload, StatusFilter, SetStatusTarget } from '../types.ts';
import { send } from '../ipc.ts';
import { UiStore } from '../uiStore.ts';
import { visibleItems } from '../filter.ts';
import { splitSegments, displayNumbers } from '../segments.ts';
import { afterIdForSlot, isNoopMove, moveToSlot, slotFromY } from '../reorder.ts';
import { Toolbar } from './Toolbar.tsx';
import { TodoCard } from './TodoCard.tsx';
import { AddBox } from './AddBox.tsx';
import { Popover, type PopoverState } from './Popovers.tsx';
import { SegmentDivider, EmptyHint } from './Segment.tsx';

const DRAG_THRESHOLD = 4;

interface DragState {
  id: number;
  startY: number;
  active: boolean;
  slot: number;
}

export function App({ payload }: { payload: ViewPayload }) {
  const store = useRef(new UiStore()).current;
  const [, bump] = useState(0);
  const rerender = () => bump((n) => n + 1);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [popover, setPopover] = useState<PopoverState | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);

  const { scrollToTop } = store.onPayload(payload);
  const ui = store.get(payload.project_id);

  useEffect(() => {
    if (scrollToTop && scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [scrollToTop, payload.scroll_nonce]);

  // 切换项目:收起弹层、取消编辑与拖拽(各项目的草稿与搜索由 UiStore 保留)。
  useEffect(() => {
    setPopover(null);
    setEditingId(null);
    setDrag(null);
  }, [payload.project_id]);

  const visible = useMemo(
    () => visibleItems(payload.items, ui.search, ui.status),
    [payload.items, ui.search, ui.status],
  );
  const segs = splitSegments(visible);
  const numbers = displayNumbers(segs);

  // 拖动期间的预览顺序(只在进行中段)
  const activeIds = segs.active.map((i) => i.id);
  const previewIds = drag && drag.active ? moveToSlot(activeIds, drag.id, drag.slot) : activeIds;
  const activeById = new Map(segs.active.map((i) => [i.id, i]));
  const activeOrdered = previewIds.map((id) => activeById.get(id)!).filter(Boolean);

  const submitSearch = () => {
    ui.search = ui.searchDraft;
    rerender();
  };

  const submitAdd = () => {
    const text = ui.addDraft.trim();
    if (!text) return;
    send({ kind: 'add', text });
    ui.addDraft = '';
    rerender();
  };

  const onCardPointerDown = (id: number) => (e: PointerEvent) => {
    if (!activeById.has(id) || editingId !== null || e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest('button, textarea, input')) return; // 点按钮/输入不触发拖拽
    setDrag({ id, startY: e.clientY, active: false, slot: activeIds.indexOf(id) });
  };

  useEffect(() => {
    if (!drag) return;
    const onMove = (e: PointerEvent) => {
      setDrag((d) => {
        if (!d) return d;
        const moved = d.active || Math.abs(e.clientY - d.startY) > DRAG_THRESHOLD;
        if (!moved) return d;
        const rows = Array.from(document.querySelectorAll<HTMLElement>('.segment-active .todo-card')).map((el) => {
          const r = el.getBoundingClientRect();
          return { id: Number(el.dataset.id), top: r.top, bottom: r.bottom };
        });
        return { ...d, active: true, slot: slotFromY(rows, d.id, e.clientY) };
      });
    };
    const finish = (commit: boolean) => {
      setDrag((d) => {
        if (d && d.active && commit && !isNoopMove(activeIds, d.id, d.slot)) {
          send({ kind: 'reorder', id: d.id, after_id: afterIdForSlot(activeIds, d.id, d.slot) });
        }
        return null;
      });
    };
    const onUp = () => finish(true);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') finish(false);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('keydown', onKey);
    };
  }, [drag?.id, activeIds.join(',')]);

  const openPopover = (p: PopoverState) => {
    setEditingId(null);
    setPopover(p);
  };
  const rect = (r: DOMRect) => ({ left: r.left, top: r.top, right: r.right, bottom: r.bottom });

  const renderCard = (it: (typeof payload.items)[number], draggable: boolean) => (
    <TodoCard
      key={it.id}
      item={it}
      number={numbers.get(it.id) ?? 0}
      selected={ui.selectedId === it.id}
      draggable={draggable}
      dragging={!!drag && drag.active && drag.id === it.id}
      editing={editingId === it.id}
      onToggle={() => send({ kind: 'toggle', id: it.id })}
      onSelect={() => {
        ui.selectedId = it.id;
        rerender();
      }}
      onBeginEdit={() => {
        setPopover(null);
        setEditingId(it.id);
      }}
      onCommitEdit={(text) => {
        setEditingId(null);
        const t = text.trim();
        if (t && t !== it.text) send({ kind: 'edit_text', id: it.id, text: t });
      }}
      onCancelEdit={() => setEditingId(null)}
      onOpenCategory={(a) => openPopover({ kind: 'category', id: it.id, anchor: rect(a), currentId: it.category_id })}
      onOpenCalendar={(a) => openPopover({ kind: 'calendar', id: it.id, anchor: rect(a), planDate: it.plan_date })}
      onOpenStatus={(a) => openPopover({ kind: 'status', id: it.id, anchor: rect(a), current: it.state })}
      onOpenDispatch={(a) => openPopover({ kind: 'dispatch', id: it.id, anchor: rect(a) })}
      onOpenDetail={() => send({ kind: 'open_detail', id: it.id })}
      onPointerDown={onCardPointerDown(it.id)}
    />
  );

  const pickStatus = (id: number, target: SetStatusTarget | 'in_progress') => {
    const card = payload.items.find((i) => i.id === id);
    if (target === 'in_progress') {
      // 「进行中」不可直接写入:搁置 → 恢复为待办;否则打开派发弹层。
      if (card?.state === 'suspended') {
        send({ kind: 'set_status', id, state: 'pending' });
        setPopover(null);
      } else {
        setPopover((cur) => (cur ? { kind: 'dispatch', id, anchor: cur.anchor } : cur));
      }
      return;
    }
    send({ kind: 'set_status', id, state: target });
    setPopover(null);
  };

  const hasAny = segs.active.length + segs.paused.length + segs.done.length > 0;
  const addHeight = ui.addHeight ?? payload.add_height_px;

  return (
    <div class="todo-root">
      <Toolbar
        searchDraft={ui.searchDraft}
        highlight={ui.searchDraft !== '' || ui.search !== ''}
        status={ui.status}
        onDraft={(v) => {
          ui.searchDraft = v;
          rerender();
        }}
        onSubmit={submitSearch}
        onOpenStatusFilter={(a) => openPopover({ kind: 'statusFilter', anchor: rect(a) })}
      />
      <div class="todo-scroll" ref={scrollRef} onScroll={(e) => (ui.scrollTop = (e.currentTarget as HTMLElement).scrollTop)}>
        {!hasAny ? (
          <EmptyHint>没有匹配的任务</EmptyHint>
        ) : (
          <>
            <div class="segment segment-active">{activeOrdered.map((it) => renderCard(it, true))}</div>
            {segs.paused.length > 0 ? (
              <div class="segment segment-paused">
                <SegmentDivider label="搁置" />
                {segs.paused.map((it) => renderCard(it, false))}
              </div>
            ) : null}
            {segs.done.length > 0 ? (
              <div class="segment segment-done">
                <SegmentDivider label="已完成" />
                {segs.done.map((it) => renderCard(it, false))}
              </div>
            ) : null}
          </>
        )}
      </div>
      <AddBox
        draft={ui.addDraft}
        height={addHeight}
        onDraft={(v) => {
          ui.addDraft = v;
          rerender();
        }}
        onSubmit={submitAdd}
        onHeight={(px, commit) => {
          ui.addHeight = px;
          rerender();
          if (commit) send({ kind: 'add_height', px });
        }}
      />
      {popover ? (
        <Popover
          state={popover}
          agents={payload.agents}
          categories={payload.categories}
          today={payload.today}
          statusFilter={ui.status as StatusFilter}
          onClose={() => setPopover(null)}
          onPickStatus={pickStatus}
          onPickAgent={(id, agent) => {
            send({ kind: 'assign_agent', id, agent });
            setPopover(null);
          }}
          onPickDate={(id, date) => {
            send({ kind: 'set_plan_date', id, date });
            setPopover(null);
          }}
          onPickStatusFilter={(f) => {
            ui.status = f;
            setPopover(null);
            rerender();
          }}
          onPickCategory={(id, categoryId) => {
            send({ kind: 'set_category', id, category_id: categoryId });
            setPopover(null);
          }}
        />
      ) : null}
    </div>
  );
}
```

注意 `App.tsx` 里 `payload.items` 的类型引用用了 `(typeof payload.items)[number]`；若类型检查不接受，改为从 `../types.ts` 导入 `TodoCard as CardData` 并使用。

- [ ] **Step 4: 追加组件样式到 `styles.css`**

在 `src/styles.css` 末尾追加：

```css
.todo-root { display: flex; flex-direction: column; height: 100%; position: relative; }
.todo-scroll { flex: 1; overflow-y: auto; padding: 0 20px 8px; }

/* 搜索条 */
.toolbar { display: flex; align-items: center; gap: 8px; margin: 8px 20px; padding: 4px 8px;
  border: 1px solid var(--border); border-radius: 8px; background: var(--bg); }
.toolbar.is-highlight { border-color: var(--gold); }
.search-input { flex: 1; min-width: 0; border: 0; outline: 0; background: transparent; color: var(--cream); font: inherit; }
.search-input::placeholder { color: var(--dim); }
.status-segment, .icon-btn, .chip, .date-badge, .status-btn, .detail-btn, .assign-btn, .menu-item, .add-submit, .checkbox {
  appearance: none; font: inherit; cursor: pointer; background: transparent; border: 0; color: inherit;
}
.status-segment { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border: 1px solid var(--border);
  border-radius: 4px; background: var(--bg); }
.status-segment:hover, .status-btn:hover, .detail-btn:hover, .assign-btn:hover { background: var(--card); border-color: var(--gold); }
.icon-btn { padding: 2px 6px; color: var(--dim); border-radius: 4px; }
.icon-btn:hover { color: var(--gold); }
.chevron { color: var(--dim); font-size: 10px; }

/* 分段 */
.segment-divider { display: flex; align-items: center; gap: 10px; margin: 14px 0 8px; color: var(--dim); font-size: 12px; }
.segment-divider-line { flex: 1; height: 1px; background: var(--border); }
.empty-hint { padding: 20px; color: var(--dim); }

/* 卡片 */
.todo-card { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 10px 12px;
  margin-bottom: 8px; box-shadow: var(--shadow); user-select: none; }
.todo-card.is-draggable { cursor: grab; }
.todo-card.is-selected { border-color: var(--gold); }
.todo-card.is-dragging { opacity: .55; cursor: grabbing; }
.card-top { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--dim); }
.spacer { flex: 1; }
.chip { padding: 1px 8px; border: 1px solid var(--border); border-radius: 10px; background: var(--card); color: var(--dim); }
.chip:hover { border-color: var(--gold); }
.date-badge { display: inline-flex; align-items: center; gap: 4px; color: var(--dim); }
.card-body { display: flex; gap: 10px; align-items: flex-start; margin: 8px 0; }
.checkbox { width: 18px; height: 18px; flex: none; border: 1.5px solid var(--dim); border-radius: 4px; color: var(--dim);
  display: inline-flex; align-items: center; justify-content: center; font-size: 12px; line-height: 1; padding: 0; margin-top: 2px; }
.checkbox.is-checked { background: var(--border); border-color: var(--border); }
.card-text { flex: 1; min-width: 0; color: var(--body); cursor: text; white-space: pre-wrap; word-break: break-word; user-select: text; }
.is-done .card-text { color: var(--dim); text-decoration: line-through; }
.edit-area { flex: 1; min-width: 0; resize: none; padding: 6px 8px; font: inherit; color: var(--cream); background: transparent;
  border: 1px solid var(--border); border-radius: 4px; outline: 0; field-sizing: content; min-height: 2.4em; }
.card-bottom { display: flex; align-items: center; gap: 12px; }
.status-btn, .detail-btn, .assign-btn { display: inline-flex; align-items: center; gap: 4px; padding: 4px 8px;
  border: 1px solid var(--border); border-radius: 4px; background: var(--bg); }
.detail-btn { color: var(--gold); }
.assign-btn { color: var(--cream); }

/* 新增框 */
.add-box { padding: 4px 20px 12px; border-top: 1px solid var(--border); }
.add-resize-handle { height: 6px; margin: 0 auto 4px; width: 48px; border-radius: 3px; background: var(--border); cursor: ns-resize; }
.add-row { position: relative; }
.add-input { width: 100%; resize: none; padding: 8px 40px 8px 10px; font: inherit; color: var(--cream); background: var(--bg);
  border: 1px solid var(--border); border-radius: 8px; outline: 0; }
.add-input:hover, .add-input:focus { border-color: var(--gold); }
.add-submit { position: absolute; right: 8px; bottom: 8px; width: 24px; height: 24px; border-radius: 12px; color: var(--dim); font-size: 14px; }
.add-submit:hover { color: var(--gold); }

/* 弹层 */
.popover-layer { position: fixed; inset: 0; z-index: 10; }
.popover { position: fixed; min-width: 120px; max-height: 70vh; overflow: auto; padding: 4px;
  background: var(--card); border: 1px solid var(--border); border-radius: 6px; box-shadow: var(--shadow); }
.menu-item { display: flex; align-items: center; gap: 8px; width: 100%; padding: 6px 10px; text-align: left; border-radius: 4px; color: var(--cream); }
.menu-item:hover { background: var(--card-hover); }
.menu-item.is-current { background: var(--tab-hover); }
.agent-icon { width: 14px; height: 14px; display: inline-flex; color: var(--cream); }
.agent-icon svg { width: 100%; height: 100%; fill: currentColor; }
.agent-icon.keep-color svg { fill: revert; }
.popover-category { min-width: 180px; }

/* 日历 */
.calendar { width: 224px; padding: 6px; }
.calendar-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 4px; }
.calendar-title { color: var(--cream); }
.calendar-grid { display: grid; grid-template-columns: repeat(7, 1fr); gap: 2px; text-align: center; }
.calendar-weekday { color: var(--dim); font-size: 11px; padding: 2px 0; }
.calendar-cell { padding: 4px 0; border-radius: 4px; color: var(--body); }
.calendar-cell:not(.empty):hover { background: var(--card-hover); }
.calendar-cell.is-selected { background: var(--tab-hover); color: var(--cream); }
```

- [ ] **Step 5: 跑渲染冒烟、单测、类型检查**

```bash
cd crates/dozer-app/web/todo-content
node render-smoke.mjs 2>&1 | tail -15
node --test src/*.test.ts 2>&1 | tail -6
npm run typecheck 2>&1 | tail -10
```
Expected: 渲染冒烟 7 个用例 PASS；单测全 PASS；类型检查无输出。类型错误按提示修（常见：`onKeyDown` 事件类型与 `KeyLike` 不兼容时，在调用处用 `e as unknown as KeyboardEvent`；`defaultValue` 在 textarea 上用 `value` + `onInput` 亦可，行为需保持"进入编辑时填入原文、可编辑、提交时读 `value`"）。

- [ ] **Step 6: 构建并提交产物**

```bash
npm run build 2>&1 | tail -4
ls -la ../../assets/todo-content
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/todo-webview
git branch --show-current
git add crates/dozer-app/web/todo-content crates/dozer-app/assets/todo-content
git commit -m "feat(todo): implement todo-content components, styles and committed bundle" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `assets/todo-content/` 有三个非空文件（`todo-content.js` 数十 KB 量级）；`git status --short` 干净。


---

### Task 4: `assets.rs` 资源路由与产物测试

**Files:**
- Modify: `crates/dozer-app/src/assets.rs`（`todo_content_root_for`、路由、4 个测试）

**Interfaces:**
- Consumes: Task 3 提交的 `assets/todo-content/{host.html,todo-content.js,todo-content.css}`。
- Produces: `dozer://todo-content/<path>` 路由。

- [ ] **Step 1: 写失败的测试**

在 `assets.rs` 的 `mod tests` 里、`codehealth_content_host_has_strict_csp_and_no_external_refs` 之后追加（`scratch()`、`handle_protocol` 沿用同模块现有写法）：

```rust
    #[test]
    fn todo_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("todo-content")).unwrap();
        std::fs::write(
            root.with_file_name("todo-content").join("host.html"),
            b"<html>t</html>",
        )
        .unwrap();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://todo-content/host.html");
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>t</html>");
    }

    #[test]
    fn todo_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("todo-content")).unwrap();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://todo-content/nope");
        assert_eq!(r.status, 404);
    }

    /// 路径穿越不得逃出 todo-content 根。
    #[test]
    fn todo_content_rejects_path_traversal() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("todo-content")).unwrap();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://todo-content/../usage-content/host.html",
        );
        assert_eq!(r.status, 404);
    }

    /// 提交的 todo-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn todo_content_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/todo-content"));
        for f in ["host.html", "todo-content.js", "todo-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 todo-content 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "todo-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src、无网络引用。
    #[test]
    fn todo_content_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/todo-content"));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("connect-src"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("todo-content.js") && html.contains("todo-content.css"));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo_content 2>&1 | tail -15`
Expected: `todo_content_serves_vendored_files` 等路由相关测试 FAIL（404，路由还没加）；`todo_content_bundle_assets_are_present`、`..._strict_csp_...` 应 PASS（产物已在 Task 3 提交）。

- [ ] **Step 3: 实现路由**

在 `codehealth_content_root_for` 之后加：

```rust
/// todo-content host(Todo 面板内容区)静态资源根 = flyfish 根的兄弟目录
/// `todo-content`。同 `codehealth_content_root_for`。
fn todo_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("todo-content")
}
```

在 `if let Some(path) = rest.strip_prefix("codehealth-content/") { ... }` 之后加：

```rust
    // todo-content host(Todo 面板内容区):同 codehealth-content,没有 data.json
    // 特判,数据全靠 evaluate_script 推送。
    if let Some(path) = rest.strip_prefix("todo-content/") {
        return serve_vendored(&todo_content_root_for(assets_root), path);
    }
```

并把路由注释里列举命名空间的那行（`// html/usage-content/codehealth-content 这些命名空间。`）补上 `todo-content`。

- [ ] **Step 4: 跑测试确认通过并提交**

Run: `cargo test -p dozer-app todo_content 2>&1 | tail -10`
Expected: 5 个 PASS。

```bash
git branch --show-current
git add crates/dozer-app/src/assets.rs
git commit -m "feat(todo): serve dozer://todo-content assets" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Rust 协议模块 `extensions/todo/protocol.rs`

**Files:**
- Create: `crates/dozer-app/src/extensions/todo/protocol.rs`
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`（`mod protocol; pub(crate) use protocol::*;`）
- Modify: `crates/dozer-app/src/extensions/todo/state.rs`（`scroll_nonce` 字段、`selected_id()`）

**Interfaces:**
- Consumes: 现有 `WorkspaceState` 的 `items`/`categories`/`category_selected`/`selected_row`/`add_input_height()`；`todo_display_state`、`filter_todos_by_category`、`visible_category_rows`、`format_todo_month_day`；`crate::workspace::{Workspace, agent_icon}`；`dozer_core::protocol::{AgentKind, TodoInfo}`。
- Produces（Task 6、7 使用）：
  - `TODO_PROTOCOL_VERSION: u32`、`READY_TIMEOUT: Duration`、`MAX_TEXT_CHARS: usize`
  - `TodoViewPayload`、`TodoCardVm`、`CategoryRowVm`、`AgentVm`、`TodayVm`（均 `Serialize + PartialEq + Clone + Debug`）
  - `pub(crate) fn derive_states(ws: &Workspace) -> Vec<TodoState>`
  - `pub(crate) fn current_view_payload(ws: &Workspace, project_id: i64, today: (i32, u32, u32)) -> TodoViewPayload`
  - `pub(crate) fn build_payload(todo: &WorkspaceState, states: &[TodoState], project_id: i64, today: (i32, u32, u32), agents: Vec<AgentVm>) -> TodoViewPayload`
  - `pub enum TodoWebviewEvent`、`pub enum SetStatusTarget`、`pub fn parse_todo_event(body: &str) -> Result<TodoWebviewEvent, String>`
  - `pub enum Routed { Message(Message), AssignAgent { idx: usize, agent: AgentKind }, OpenDetail { idx: usize } }`、`pub(crate) fn route_event(items: &[TodoInfo], event: TodoWebviewEvent) -> Option<Routed>`
  - `pub struct WebviewPushState`（`set_ready`、`observe_availability`、`pending_push`、`mark_sent`、`failed`、`set_failed`、`clear_failed`）
  - `pub fn encode_todo_push(revision: u64, payload: TodoViewPayload) -> String`
  - `WorkspaceState::selected_id() -> Option<i64>`、字段 `scroll_nonce: u32`

> `route_event` 返回的 `Message` 变体里有 Task 7 才新增的 `AddText` / `EditText` / `ReorderTo` / `SetCategory` / `AddHeight`。**本任务先在 `state.rs` 的 `Message` 枚举里把这 5 个变体与 `ContentRetry` 加上**（只加变体，处理逻辑在 Task 7；`update` 的 `match` 需要对它们给空分支 `=> {}` 先保证编译，Task 7 再替换）。

- [ ] **Step 1: 在 `state.rs` 里加状态字段、访问器与新 `Message` 变体**

`WorkspaceState` 增加字段（放在 `scroll_to_top` 之后）：

```rust
    /// 每次 `start_flash`(新增任务后要滚回顶部)时递增;随推送带给 webview,
    /// 前端据此滚回顶部。取代 iced `scrollable::scroll_to` 的一次性标记。
    pub(crate) scroll_nonce: u32,
```

`start_flash` 末尾加一行 `self.scroll_nonce = self.scroll_nonce.wrapping_add(1);`。

加访问器（`items()` 附近）：

```rust
    /// 当前"选中高亮"的任务 id(新增后 2 秒高亮用),供推送给 webview。
    pub fn selected_id(&self) -> Option<i64> {
        self.selected_row
            .and_then(|i| self.items.get(i))
            .map(|it| it.id)
    }
```

`Message` 枚举末尾追加：

```rust
    /// webview `Add`:新增任务文本(已 trim、非空、未超长)。
    AddText(String),
    /// webview `EditText`:按 id 改文字。
    EditText(i64, String),
    /// webview `Reorder`:把 `id` 挪到 `after_id` 之后(`None` = 进行中段最前)。
    ReorderTo { id: i64, after_id: Option<i64> },
    /// webview `SetCategory`:`None` = 未分类。
    SetCategory(i64, Option<i64>),
    /// webview `AddHeight`:新增框高度(px),由 `set_add_input_height` 钳制。
    AddHeight(f32),
    /// webview 加载失败后原生占位页的「重试」。
    ContentRetry,
```

`update.rs` 的 `update` 里先给这 6 个变体加空分支（Task 7 替换）：

```rust
        Message::AddText(_)
        | Message::EditText(_, _)
        | Message::ReorderTo { .. }
        | Message::SetCategory(_, _)
        | Message::AddHeight(_)
        | Message::ContentRetry => {}
```

- [ ] **Step 2: 写失败的测试**

`protocol.rs` 末尾写完整的 `mod tests`（先只写测试，实现在 Step 4）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::todo::{CategoryFilter, Message};
    use dozer_core::protocol::CategoryInfo;
    use serde_json::json;

    fn todo(id: i64, text: &str) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: text.to_string(),
            done: false,
            paused: false,
            rank: id,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        }
    }

    fn category(id: i64, parent: Option<i64>, name: &str) -> CategoryInfo {
        serde_json::from_value(json!({
            "id": id, "project_id": 1, "parent_id": parent, "name": name,
            "rank": id, "created_ms": 0
        }))
        .expect("CategoryInfo")
    }

    fn state_with(items: Vec<TodoInfo>) -> WorkspaceState {
        WorkspaceState {
            items,
            ..WorkspaceState::default()
        }
    }

    // ---- payload ----

    #[test]
    fn payload_filters_by_selected_category_in_original_order() {
        let mut a = todo(1, "A");
        a.category_id = Some(10);
        let b = todo(2, "B");
        let mut c = todo(3, "C");
        c.category_id = Some(11); // 10 的子分类
        let mut ws = state_with(vec![a, b, c]);
        ws.categories = vec![category(10, None, "后端"), category(11, Some(10), "接口")];
        ws.category_selected = CategoryFilter::Node(10);
        let states = vec![TodoState::Pending; 3];
        let p = build_payload(&ws, &states, 7, (2026, 10, 2), vec![]);
        assert_eq!(p.project_id, 7);
        assert_eq!(p.category_key, "node:10");
        assert_eq!(p.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(p.items[0].category_name.as_deref(), Some("后端"));
    }

    #[test]
    fn payload_marks_segments_and_uses_given_states() {
        let mut paused = todo(2, "搁置");
        paused.paused = true;
        let mut done = todo(3, "完成");
        done.done = true;
        done.completed_at_ms = Some(0);
        let ws = state_with(vec![todo(1, "进行"), paused, done]);
        let states = vec![TodoState::InProgress, TodoState::Suspended, TodoState::Done];
        let p = build_payload(&ws, &states, 1, (2026, 10, 2), vec![]);
        assert_eq!(p.items[0].segment, SegmentKey::Active);
        assert_eq!(p.items[1].segment, SegmentKey::Paused);
        assert_eq!(p.items[2].segment, SegmentKey::Done);
        assert_eq!(p.items[0].state, StateKey::InProgress);
        assert!(p.items[2].completed_label.is_some());
        assert!(p.items[0].completed_label.is_none());
    }

    #[test]
    fn payload_category_key_covers_all_filters() {
        let mut ws = state_with(vec![]);
        let key = |ws: &WorkspaceState| build_payload(ws, &[], 1, (2026, 10, 2), vec![]).category_key;
        ws.category_selected = CategoryFilter::All;
        assert_eq!(key(&ws), "all");
        ws.category_selected = CategoryFilter::Uncategorized;
        assert_eq!(key(&ws), "uncategorized");
        ws.category_selected = CategoryFilter::Node(42);
        assert_eq!(key(&ws), "node:42");
    }

    #[test]
    fn payload_flattens_categories_with_depth_and_parent() {
        let mut ws = state_with(vec![]);
        ws.categories = vec![category(10, None, "后端"), category(11, Some(10), "接口")];
        let p = build_payload(&ws, &[], 1, (2026, 10, 2), vec![]);
        assert_eq!(p.categories.len(), 2);
        assert_eq!((p.categories[1].id, p.categories[1].depth, p.categories[1].parent_id), (11, 1, Some(10)));
    }

    #[test]
    fn payload_carries_selected_id_scroll_nonce_and_height() {
        let mut ws = state_with(vec![todo(5, "新"), todo(6, "旧")]);
        ws.start_flash(0);
        let p = build_payload(&ws, &[TodoState::Pending; 2], 1, (2026, 10, 2), vec![]);
        assert_eq!(p.selected_id, Some(5));
        assert_eq!(p.scroll_nonce, 1);
        assert!(p.add_height_px > 0.0);
    }

    /// 字段名一旦改动就会破坏前端,用快照钉住形状。
    #[test]
    fn payload_serializes_stable_shape() {
        let mut item = todo(1, "写文档");
        item.plan_date = Some("10-05".into());
        let ws = state_with(vec![item]);
        let p = build_payload(&ws, &[TodoState::Pending], 3, (2026, 10, 2), vec![]);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["project_id"], 3);
        assert_eq!(v["category_key"], "all");
        assert_eq!(v["today"], json!({"year": 2026, "month": 10, "day": 2}));
        assert_eq!(
            v["items"][0],
            json!({
                "id": 1, "text": "写文档", "state": "pending", "segment": "active",
                "plan_date": "10-05", "completed_label": null, "category_id": null,
                "category_name": null, "assigned_agent": null, "has_dispatch": false
            })
        );
        for key in ["categories", "agents", "add_height_px", "selected_id", "scroll_nonce"] {
            assert!(v.get(key).is_some(), "缺字段 {key}");
        }
    }

    #[test]
    fn agents_are_the_four_headless_kinds_with_inline_svg() {
        let agents = agent_vms();
        let kinds: Vec<_> = agents.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![AgentKind::Claude, AgentKind::Codebuddy, AgentKind::Opencode, AgentKind::V8agent]
        );
        assert!(agents.iter().all(|a| a.icon_svg.contains("<svg")));
    }

    // ---- events ----

    #[test]
    fn parses_every_event_kind() {
        let cases = [
            (r#"{"kind":"ready"}"#, TodoWebviewEvent::Ready),
            (r#"{"kind":"failed","reason":"x"}"#, TodoWebviewEvent::Failed { reason: "x".into() }),
            (r#"{"kind":"add","text":"a"}"#, TodoWebviewEvent::Add { text: "a".into() }),
            (r#"{"kind":"toggle","id":3}"#, TodoWebviewEvent::Toggle { id: 3 }),
            (
                r#"{"kind":"edit_text","id":3,"text":"b"}"#,
                TodoWebviewEvent::EditText { id: 3, text: "b".into() },
            ),
            (
                r#"{"kind":"reorder","id":3,"after_id":null}"#,
                TodoWebviewEvent::Reorder { id: 3, after_id: None },
            ),
            (
                r#"{"kind":"reorder","id":3,"after_id":2}"#,
                TodoWebviewEvent::Reorder { id: 3, after_id: Some(2) },
            ),
            (
                r#"{"kind":"set_status","id":3,"state":"suspended"}"#,
                TodoWebviewEvent::SetStatus { id: 3, state: SetStatusTarget::Suspended },
            ),
            (
                r#"{"kind":"set_plan_date","id":3,"date":"10-05"}"#,
                TodoWebviewEvent::SetPlanDate { id: 3, date: "10-05".into() },
            ),
            (
                r#"{"kind":"assign_agent","id":3,"agent":"claude"}"#,
                TodoWebviewEvent::AssignAgent { id: 3, agent: AgentKind::Claude },
            ),
            (
                r#"{"kind":"set_category","id":3,"category_id":null}"#,
                TodoWebviewEvent::SetCategory { id: 3, category_id: None },
            ),
            (r#"{"kind":"open_detail","id":3}"#, TodoWebviewEvent::OpenDetail { id: 3 }),
            (r#"{"kind":"add_height","px":120.0}"#, TodoWebviewEvent::AddHeight { px: 120.0 }),
        ];
        for (body, expected) in cases {
            assert_eq!(parse_todo_event(body).unwrap(), expected, "{body}");
        }
    }

    #[test]
    fn rejects_unknown_kind_and_garbage() {
        assert!(parse_todo_event(r#"{"kind":"nope"}"#).is_err());
        assert!(parse_todo_event("not json").is_err());
    }

    // ---- routing ----

    fn items3() -> Vec<TodoInfo> {
        let mut paused = todo(2, "搁置");
        paused.paused = true;
        let mut done = todo(3, "完成");
        done.done = true;
        vec![todo(1, "进行"), paused, done]
    }

    // Review Focus 1:过期 id 必须空操作,绝不能落到别的行。
    #[test]
    fn route_event_unknown_id_is_noop() {
        let items = items3();
        for ev in [
            TodoWebviewEvent::Toggle { id: 999 },
            TodoWebviewEvent::EditText { id: 999, text: "x".into() },
            TodoWebviewEvent::Reorder { id: 999, after_id: None },
            TodoWebviewEvent::SetStatus { id: 999, state: SetStatusTarget::Done },
            TodoWebviewEvent::SetPlanDate { id: 999, date: "10-05".into() },
            TodoWebviewEvent::AssignAgent { id: 999, agent: AgentKind::Claude },
            TodoWebviewEvent::SetCategory { id: 999, category_id: None },
            TodoWebviewEvent::OpenDetail { id: 999 },
        ] {
            assert!(route_event(&items, ev.clone()).is_none(), "{ev:?}");
        }
    }

    #[test]
    fn route_event_maps_id_to_current_index() {
        let items = items3();
        let r = route_event(&items, TodoWebviewEvent::Toggle { id: 3 }).unwrap();
        assert!(matches!(r, Routed::Message(Message::Toggle(2))));
        let r = route_event(&items, TodoWebviewEvent::OpenDetail { id: 2 }).unwrap();
        assert!(matches!(r, Routed::OpenDetail { idx: 1 }));
        let r = route_event(
            &items,
            TodoWebviewEvent::SetStatus { id: 1, state: SetStatusTarget::Done },
        )
        .unwrap();
        assert!(matches!(r, Routed::Message(Message::StatusPick(0, TodoState::Done))));
    }

    #[test]
    fn route_add_trims_and_rejects_empty_and_oversized() {
        let items = items3();
        let r = route_event(&items, TodoWebviewEvent::Add { text: "  新任务\n".into() }).unwrap();
        assert!(matches!(r, Routed::Message(Message::AddText(ref t)) if t == "新任务"));
        assert!(route_event(&items, TodoWebviewEvent::Add { text: "   ".into() }).is_none());
        let huge = "字".repeat(MAX_TEXT_CHARS + 1);
        assert!(route_event(&items, TodoWebviewEvent::Add { text: huge.clone() }).is_none());
        assert!(
            route_event(&items, TodoWebviewEvent::EditText { id: 1, text: huge }).is_none()
        );
    }

    #[test]
    fn route_set_plan_date_validates_month_day() {
        let items = items3();
        let ok = route_event(&items, TodoWebviewEvent::SetPlanDate { id: 1, date: "9-5".into() });
        // 规范化成 MM-DD
        assert!(matches!(ok, Some(Routed::Message(Message::CalendarPick(0, ref d))) if d == "09-05"));
        for bad in ["13-01", "00-10", "10-32", "abc", "10-00", ""] {
            assert!(
                route_event(&items, TodoWebviewEvent::SetPlanDate { id: 1, date: bad.into() }).is_none(),
                "{bad}"
            );
        }
    }

    #[test]
    fn route_assign_agent_only_accepts_the_four_headless_kinds() {
        let items = items3();
        let ok = route_event(&items, TodoWebviewEvent::AssignAgent { id: 1, agent: AgentKind::V8agent });
        assert!(matches!(ok, Some(Routed::AssignAgent { idx: 0, agent: AgentKind::V8agent })));
        for bad in [AgentKind::Codex, AgentKind::Goose, AgentKind::Aider, AgentKind::Unknown] {
            assert!(route_event(&items, TodoWebviewEvent::AssignAgent { id: 1, agent: bad }).is_none());
        }
    }

    #[test]
    fn route_reorder_only_for_active_items_and_not_onto_itself() {
        let items = items3();
        let ok = route_event(&items, TodoWebviewEvent::Reorder { id: 1, after_id: None });
        assert!(matches!(ok, Some(Routed::Message(Message::ReorderTo { id: 1, after_id: None }))));
        // 搁置、已完成不可拖
        assert!(route_event(&items, TodoWebviewEvent::Reorder { id: 2, after_id: None }).is_none());
        assert!(route_event(&items, TodoWebviewEvent::Reorder { id: 3, after_id: None }).is_none());
        // 挪到自己之后无意义
        assert!(route_event(&items, TodoWebviewEvent::Reorder { id: 1, after_id: Some(1) }).is_none());
    }

    #[test]
    fn route_add_height_requires_finite() {
        let items = items3();
        assert!(matches!(
            route_event(&items, TodoWebviewEvent::AddHeight { px: 90.0 }),
            Some(Routed::Message(Message::AddHeight(_)))
        ));
        assert!(route_event(&items, TodoWebviewEvent::AddHeight { px: f32::NAN }).is_none());
        assert!(route_event(&items, TodoWebviewEvent::AddHeight { px: f32::INFINITY }).is_none());
    }

    #[test]
    fn route_ready_and_failed_are_handled_by_the_app_not_routed() {
        let items = items3();
        assert!(route_event(&items, TodoWebviewEvent::Ready).is_none());
        assert!(route_event(&items, TodoWebviewEvent::Failed { reason: "x".into() }).is_none());
    }

    // ---- push state ----

    fn payload(project_id: i64) -> TodoViewPayload {
        build_payload(&state_with(vec![]), &[], project_id, (2026, 10, 2), vec![])
    }

    #[test]
    fn push_state_waits_for_ready_then_sends_once() {
        let mut s = WebviewPushState::default();
        let p = payload(1);
        assert!(s.pending_push(&p).is_none(), "未 ready 不推送");
        s.set_ready(true);
        let to_send = s.pending_push(&p).expect("ready 后应推送");
        assert_eq!(s.mark_sent(to_send), 1);
        assert!(s.pending_push(&p).is_none(), "内容没变不重复推送");
        assert!(s.pending_push(&payload(2)).is_some(), "内容变了(换项目)要推送");
    }

    // Review Focus 4:webview 被重建后 ready 会强制重发当前内容。
    #[test]
    fn push_state_set_ready_forces_resend() {
        let mut s = WebviewPushState::default();
        let p = payload(1);
        s.set_ready(true);
        let first = s.pending_push(&p).unwrap();
        s.mark_sent(first);
        s.set_ready(true);
        assert!(s.pending_push(&p).is_some());
        assert_eq!(s.mark_sent(p.clone()), 2, "revision 单调递增");
    }

    #[test]
    fn push_state_times_out_when_never_ready_and_blocks_push() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, t0);
        assert!(s.failed().is_none());
        s.observe_availability(true, t0 + READY_TIMEOUT + Duration::from_secs(1));
        assert!(s.failed().is_some());
        s.set_ready(true);
        s.clear_failed();
        assert!(s.failed().is_none());
    }

    #[test]
    fn push_state_unavailable_resets_ready() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        s.observe_availability(false, Instant::now());
        assert!(s.pending_push(&payload(1)).is_none());
    }

    #[test]
    fn encode_push_wraps_revision_and_version() {
        let out = encode_todo_push(5, payload(9));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["protocol_version"], TODO_PROTOCOL_VERSION);
        assert_eq!(v["revision"], 5);
        assert_eq!(v["payload"]["project_id"], 9);
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p dozer-app extensions::todo::protocol 2>&1 | tail -15`
Expected: 编译失败，`cannot find ...` 一批（`build_payload`、`TodoWebviewEvent`、`route_event`、`WebviewPushState` 等都未定义）。

- [ ] **Step 4: 实现协议模块**

在 `protocol.rs` 里、`mod tests` 之前写（先在 `mod.rs` 加 `mod protocol;` 与 `pub(crate) use protocol::*;`）：

```rust
//! Todo 内容区 webview 推送协议:把 `WorkspaceState` 算成要序列化推给 webview
//! 的 `TodoViewPayload`,并解析 webview 发回的事件。前端只渲染与保存纯视图
//! 状态,**不做过滤(分类)、状态派生、排序**——这些全在 Rust。
//! 形态仿 `extensions/codehealth/protocol.rs`(声明式比较 + 固定单槽)。

use super::{
    CategoryFilter, Message, TodoState, WorkspaceState, filter_todos_by_category,
    format_todo_month_day, is_active_todo, parse_month_day, todo_display_state,
    visible_category_rows,
};
use crate::workspace::{Workspace, agent_icon};
use dozer_core::protocol::{AgentKind, TodoInfo};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const TODO_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 来自 webview 的任务文本长度上限(字符数)。webview 内容不可信,防止异常
/// 超长文本落库。
pub const MAX_TEXT_CHARS: usize = 10_000;

// ---------------------------------------------------------------------------
// Rust → webview
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StateKey {
    Pending,
    InProgress,
    Suspended,
    Done,
}

impl From<TodoState> for StateKey {
    fn from(s: TodoState) -> Self {
        match s {
            TodoState::Pending => StateKey::Pending,
            TodoState::InProgress => StateKey::InProgress,
            TodoState::Suspended => StateKey::Suspended,
            TodoState::Done => StateKey::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKey {
    Active,
    Paused,
    Done,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TodoCardVm {
    pub id: i64,
    pub text: String,
    pub state: StateKey,
    pub segment: SegmentKey,
    /// "MM-DD"
    pub plan_date: Option<String>,
    /// 已完成时的完成日期("MM-DD"或 "-"),否则 `None`。
    pub completed_label: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub assigned_agent: Option<AgentKind>,
    pub has_dispatch: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CategoryRowVm {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentVm {
    pub kind: AgentKind,
    pub label: &'static str,
    /// Rust 内嵌的 SVG 原文(可信来源,前端用 innerHTML 渲染)。
    pub icon_svg: String,
    pub preserves_color: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct TodayVm {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TodoViewPayload {
    pub project_id: i64,
    /// `all` / `uncategorized` / `node:<id>`;前端据此在分类切换时重置搜索。
    pub category_key: String,
    /// 已按左栏选中的分类过滤,保持 dozerd 给出的顺序。
    pub items: Vec<TodoCardVm>,
    /// 全展开拍平,供分类选择器按 `depth` 缩进。
    pub categories: Vec<CategoryRowVm>,
    pub agents: Vec<AgentVm>,
    pub add_height_px: f32,
    pub today: TodayVm,
    /// 新增后 2 秒高亮的任务 id(沿用 `Flash` 计时)。
    pub selected_id: Option<i64>,
    /// 每次要求"滚回顶部"时递增。
    pub scroll_nonce: u32,
}

/// 指派选择器里的四个 headless agent(与 `dispatch_items` 一致)。
pub(crate) const DISPATCH_AGENTS: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::V8agent,
];

pub(crate) fn agent_vms() -> Vec<AgentVm> {
    DISPATCH_AGENTS
        .into_iter()
        .map(|agent| {
            let icon = agent_icon(agent);
            AgentVm {
                kind: agent,
                label: agent.display_label(),
                icon_svg: String::from_utf8_lossy(icon.bytes()).into_owned(),
                preserves_color: icon.preserves_original_color(),
            }
        })
        .collect()
}

/// 每张任务卡片的派生状态(需要看派发的目标 session 是否存活,所以要
/// `Workspace` 而不只是 `WorkspaceState`)。顺序与 `ws.todo.items` 一一对应。
pub(crate) fn derive_states(ws: &Workspace) -> Vec<TodoState> {
    ws.todo
        .items()
        .iter()
        .map(|item| {
            let target_alive = item
                .dispatch_session_id
                .as_ref()
                .map(|sid| {
                    ws.tabs
                        .iter()
                        .chain(ws.ssh_tabs.iter())
                        .any(|t| t.alive && t.info.id == *sid)
                })
                .unwrap_or(false);
            todo_display_state(item, target_alive)
        })
        .collect()
}

pub(crate) fn current_view_payload(
    ws: &Workspace,
    project_id: i64,
    today: (i32, u32, u32),
) -> TodoViewPayload {
    build_payload(&ws.todo, &derive_states(ws), project_id, today, agent_vms())
}

/// 纯函数,便于不构造 `Workspace` 地测试。`states` 与 `todo.items` 等长。
pub(crate) fn build_payload(
    todo: &WorkspaceState,
    states: &[TodoState],
    project_id: i64,
    today: (i32, u32, u32),
    agents: Vec<AgentVm>,
) -> TodoViewPayload {
    let allowed = filter_todos_by_category(&todo.items, &todo.categories, todo.category_selected);
    let category_name = |id: Option<i64>| {
        id.and_then(|cid| todo.categories.iter().find(|c| c.id == cid))
            .map(|c| c.name.clone())
    };
    let items = todo
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| allowed.contains(&item.id))
        .map(|(i, item)| {
            let segment = if item.done {
                SegmentKey::Done
            } else if item.paused {
                SegmentKey::Paused
            } else {
                SegmentKey::Active
            };
            TodoCardVm {
                id: item.id,
                text: item.text.clone(),
                state: states.get(i).copied().unwrap_or(TodoState::Pending).into(),
                segment,
                plan_date: item.plan_date.clone(),
                completed_label: item.done.then(|| {
                    item.completed_at_ms
                        .map(format_todo_month_day)
                        .unwrap_or_else(|| "-".to_string())
                }),
                category_id: item.category_id,
                category_name: category_name(item.category_id),
                assigned_agent: item.assigned_agent,
                has_dispatch: item.dispatch_session_id.is_some(),
            }
        })
        .collect();

    let all_expanded: std::collections::HashSet<i64> =
        todo.categories.iter().map(|c| c.id).collect();
    let categories = visible_category_rows(&todo.categories, &all_expanded)
        .into_iter()
        .map(|row| CategoryRowVm {
            parent_id: todo
                .categories
                .iter()
                .find(|c| c.id == row.id)
                .and_then(|c| c.parent_id),
            id: row.id,
            name: row.name,
            depth: row.depth,
        })
        .collect();

    TodoViewPayload {
        project_id,
        category_key: match todo.category_selected {
            CategoryFilter::All => "all".to_string(),
            CategoryFilter::Uncategorized => "uncategorized".to_string(),
            CategoryFilter::Node(id) => format!("node:{id}"),
        },
        items,
        categories,
        agents,
        add_height_px: todo.add_input_height(),
        today: TodayVm {
            year: today.0,
            month: today.1,
            day: today.2,
        },
        selected_id: todo.selected_id(),
        scroll_nonce: todo.scroll_nonce,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TodoPushEnvelope {
    pub protocol_version: u32,
    /// 单调递增;前端丢弃小于已应用值的推送。
    pub revision: u64,
    pub payload: TodoViewPayload,
}

pub fn encode_todo_push(revision: u64, payload: TodoViewPayload) -> String {
    serde_json::to_string(&TodoPushEnvelope {
        protocol_version: TODO_PROTOCOL_VERSION,
        revision,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

// ---------------------------------------------------------------------------
// webview → Rust
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetStatusTarget {
    Pending,
    Suspended,
    Done,
}

impl From<SetStatusTarget> for TodoState {
    fn from(t: SetStatusTarget) -> Self {
        match t {
            SetStatusTarget::Pending => TodoState::Pending,
            SetStatusTarget::Suspended => TodoState::Suspended,
            SetStatusTarget::Done => TodoState::Done,
        }
    }
}

/// webview 发回的事件。webview 内容不可信:文本长度、日期格式、agent 种类、
/// 浮点有限性都在 `route_event` 里校验。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TodoWebviewEvent {
    Ready,
    Failed { reason: String },
    Add { text: String },
    Toggle { id: i64 },
    EditText { id: i64, text: String },
    Reorder { id: i64, after_id: Option<i64> },
    SetStatus { id: i64, state: SetStatusTarget },
    SetPlanDate { id: i64, date: String },
    AssignAgent { id: i64, agent: AgentKind },
    SetCategory { id: i64, category_id: Option<i64> },
    OpenDetail { id: i64 },
    AddHeight { px: f32 },
}

pub fn parse_todo_event(body: &str) -> Result<TodoWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// `route_event` 的结果:交给 `App::todo_message` 的消息,或 `App` 上现有的
/// 下标式入口。
#[derive(Debug, Clone)]
pub enum Routed {
    Message(Message),
    AssignAgent { idx: usize, agent: AgentKind },
    OpenDetail { idx: usize },
}

fn index_of(items: &[TodoInfo], id: i64) -> Option<usize> {
    items.iter().position(|i| i.id == id)
}

fn clean_text(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && t.chars().count() <= MAX_TEXT_CHARS).then(|| t.to_string())
}

/// 把 webview 事件的 `id` 映射成**当前**下标,并校验入参,再复用现有的下标式
/// 消息与更新逻辑。`id` 已不存在(任务被别的 agent 删了)或入参非法一律返回
/// `None`(空操作),绝不能落到别的行。`Ready`/`Failed` 由 `App` 自己处理,这里
/// 返回 `None`。
pub(crate) fn route_event(items: &[TodoInfo], event: TodoWebviewEvent) -> Option<Routed> {
    use TodoWebviewEvent as Ev;
    match event {
        Ev::Ready | Ev::Failed { .. } => None,
        Ev::Add { text } => Some(Routed::Message(Message::AddText(clean_text(&text)?))),
        Ev::Toggle { id } => Some(Routed::Message(Message::Toggle(index_of(items, id)?))),
        Ev::EditText { id, text } => {
            index_of(items, id)?;
            Some(Routed::Message(Message::EditText(id, clean_text(&text)?)))
        }
        Ev::Reorder { id, after_id } => {
            let idx = index_of(items, id)?;
            if !is_active_todo(&items[idx]) || after_id == Some(id) {
                return None;
            }
            Some(Routed::Message(Message::ReorderTo { id, after_id }))
        }
        Ev::SetStatus { id, state } => {
            let idx = index_of(items, id)?;
            Some(Routed::Message(Message::StatusPick(idx, state.into())))
        }
        Ev::SetPlanDate { id, date } => {
            let idx = index_of(items, id)?;
            let (m, d) = parse_month_day(&date)?;
            if !(1..=31).contains(&d) {
                return None;
            }
            Some(Routed::Message(Message::CalendarPick(
                idx,
                format!("{m:02}-{d:02}"),
            )))
        }
        Ev::AssignAgent { id, agent } => {
            let idx = index_of(items, id)?;
            DISPATCH_AGENTS
                .contains(&agent)
                .then_some(Routed::AssignAgent { idx, agent })
        }
        Ev::SetCategory { id, category_id } => {
            index_of(items, id)?;
            Some(Routed::Message(Message::SetCategory(id, category_id)))
        }
        Ev::OpenDetail { id } => Some(Routed::OpenDetail {
            idx: index_of(items, id)?,
        }),
        Ev::AddHeight { px } => px
            .is_finite()
            .then_some(Routed::Message(Message::AddHeight(px))),
    }
}

// ---------------------------------------------------------------------------
// 声明式推送状态(同 codehealth::WebviewPushState)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<TodoViewPayload>,
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

    pub fn pending_push(&self, desired: &TodoViewPayload) -> Option<TodoViewPayload> {
        if !self.ready || self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    /// 记录已送达并返回本次 revision(从 1 起)。
    pub fn mark_sent(&mut self, payload: TodoViewPayload) -> u64 {
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

`parse_month_day` 现在是 `update.rs` 里的 `pub(crate) fn`，通过 `super::` 可达（`mod.rs` 的 `pub(crate) use update::*`）。`is_active_todo`、`format_todo_month_day` 同理（分别在 `state.rs`、`view.rs`）。`IconKind::bytes()` / `preserves_original_color()` 是 byteui 公开接口。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p dozer-app extensions::todo 2>&1 | tail -30`
Expected: 全部 PASS（含新增约 20 个与既有 todo 测试）。常见需调整点：`CategoryInfo` 若有额外必填字段，用 `serde_json::json!` 补齐；`WorkspaceState` 的字段可见性若是私有则改用 `pub(crate)`（本 plan 假定与 `mod.rs` 现有测试一致，已是 `pub(crate)`）。

- [ ] **Step 6: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo/protocol.rs crates/dozer-app/src/extensions/todo/mod.rs crates/dozer-app/src/extensions/todo/state.rs crates/dozer-app/src/extensions/todo/update.rs
git commit -m "feat(todo): add webview protocol (payload, events, routing, push state)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 几何：`todo_top_row_h_px` 与 `todo_content_pane_bounds_for`

**Files:**
- Modify: `crates/dozer-app/src/theme/geometry.rs`（`todo_top_row_h_px`、测试）
- Modify: `crates/dozer-app/src/webview_geometry.rs`（`PairPane.content_first`、`todo_content_pane_bounds_for`、测试）

**Interfaces:**
- Produces:
  - `theme::geometry::todo_top_row_h_px() -> f32`（= 原生 tab 行固定高 + 1px 分割线，已含全局 scale）
  - `theme::geometry::todo_top_row_inner_h_px() -> f32`（tab 行本身的固定高，Task 8 渲染侧用）
  - `webview_geometry::todo_content_pane_bounds_for(side, window_width, window_height, state, content_desired) -> (f32, f32, f32, f32)`

- [ ] **Step 1: 写失败的测试**

`theme/geometry.rs` 的 `mod tests` 追加：

```rust
    /// Todo 右栏 tab 行 + 分割线的总高:渲染侧(`todo::view`)固定 tab 行高、
    /// 几何侧(`todo_content_pane_bounds_for`)据此给 webview 让出头部。两侧
    /// 同一真相,漂移会让 webview 盖住 tab 行或留空。
    #[test]
    fn todo_top_row_is_tab_height_plus_padding_plus_divider() {
        let inner = todo_top_row_inner_h_px();
        assert_eq!(inner, byteui::theme::geometry::tab_button_size() + 8.0);
        assert_eq!(todo_top_row_h_px(), inner + 1.0);
    }
```

`webview_geometry.rs` 的 `mod tests` 末尾追加（`test_state()`、`ShellState`、`maximized_box_*` 沿用该模块现有写法，与 `codehealth_content_*` 测试同构）：

```rust
    #[test]
    fn todo_content_zero_when_not_desired() {
        let state = ShellState { left_view: PanelKind::Todo, ..test_state() };
        let (_, _, w, h) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, false);
        assert_eq!((w, h), (0.0, 0.0));
    }

    #[test]
    fn todo_content_zero_when_side_collapsed() {
        let state = ShellState {
            left_view: PanelKind::Todo,
            left_collapsed: true,
            ..test_state()
        };
        let (_, _, w, h) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert_eq!((w, h), (0.0, 0.0));
    }

    #[test]
    fn todo_content_zero_when_panel_kind_is_not_todo() {
        let state = test_state(); // left_view: PanelKind::Files
        let (_, _, w, h) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert_eq!((w, h), (0.0, 0.0));
    }

    /// Todo 是"列表在前、内容在后":内容列的起点在分栏线右侧,且随 `todo_split` 变化。
    #[test]
    fn todo_content_is_to_the_right_of_the_list_and_follows_split() {
        let state = ShellState { left_view: PanelKind::Todo, ..test_state() };
        let (x, _, w, h) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert!(w > 100.0 && h > 100.0, "w={w} h={h}");
        let zone_x0 = byteui::theme::geometry::icon_rail_width();
        assert!(x > zone_x0 + 50.0, "内容列应在列表列右侧: x={x} zone_x0={zone_x0}");
        let mut wider_list = state.clone();
        wider_list.dims.todo_split = (state.dims.todo_split + 0.2).min(0.9);
        let (x2, _, w2, _) =
            todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &wider_list, true);
        assert!(x2 > x && w2 < w, "列表变宽,内容列右移且变窄: {x}->{x2}, {w}->{w2}");
    }

    /// Review Focus 6:列表列收起后内容拿满整个配对宽度。
    #[test]
    fn todo_content_fills_when_list_collapsed() {
        let state = ShellState { left_view: PanelKind::Todo, ..test_state() };
        let (_, _, w, _) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        let mut collapsed = state.clone();
        collapsed.dims.todo_list_collapsed = true;
        let (_, _, wc, _) =
            todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &collapsed, true);
        assert!(wc > w, "收起列表列后内容更宽: {w} -> {wc}");
    }

    /// webview 只覆盖原生 tab 行与分割线**以下**。
    #[test]
    fn todo_content_y_starts_below_the_native_top_row() {
        let state = ShellState { left_view: PanelKind::Todo, ..test_state() };
        let (_, y, _, _) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        let zone_top =
            byteui::theme::geometry::top_bar_height() + theme::region::left_zone().margin.top;
        let expected = zone_top + theme::geometry::todo_top_row_h_px();
        assert!((y - expected).abs() < 1.0, "y={y} expected={expected}");
    }

    #[test]
    fn todo_content_maximized_stays_inside_box() {
        let state = ShellState {
            left_view: PanelKind::Todo,
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, y, w, h) = todo_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true);
        assert!(w > 100.0 && h > 100.0, "w={w} h={h}");
        let (x0, avail_w) = maximized_box_x_range(1600.0);
        assert!(x >= x0 && x + w <= x0 + avail_w, "x={x} w={w}");
        assert!(y + h <= maximized_box_height(900.0) + 200.0);
    }
```

**镜像（`mirrored`）说明**：列表在右、内容在左的镜像分支走共享的 `pair_columns(…, mirrored)`，与 Project 同一写法；该文件现有测试里没有可复用的镜像布局构造（Usage / Code Health 同样没有镜像几何测试），本 plan 不为它另造构造机制，改为列入人工验收（Task 10 的第 7 项）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app todo_ 2>&1 | grep -E "^error|todo_content|todo_top_row" | head`
Expected: 编译失败，`cannot find function todo_content_pane_bounds_for` / `todo_top_row_h_px`。

- [ ] **Step 3: 实现**

`theme/geometry.rs`（放在 `usage_content_chrome_top_px` 之后）：

```rust
/// Todo 右栏顶部原生 tab 行(「列表视图 / 看板视图」+「收起」按钮)本身的固定
/// 高度(逻辑像素,已含全局 scale)。渲染侧 `todo::view` 把 tab 行钉成这个高度
/// (tab 按钮同样钉 `tab_button_size()` 高),不靠内容自然撑高——几何侧才能
/// 精确地给 webview 让出这段。
pub fn todo_top_row_inner_h_px() -> f32 {
    byteui::theme::geometry::tab_button_size() + 8.0
}

/// tab 行 + 其下 1px 分割线(`byteui::layout::divider::horizontal`)的总高。
/// webview 只覆盖这段**以下**的内容区,不能把 tab 行也盖住。
pub fn todo_top_row_h_px() -> f32 {
    todo_top_row_inner_h_px() + 1.0
}
```

`webview_geometry.rs`：

1. `PairPane` 增加字段 `content_first: bool`（文档：`true` = "内容在前、列表在后"，如 Usage / Code Health；`false` = "列表在前、内容在后"，如 Todo）。
2. `usage_content_pane_bounds_for` 与 `codehealth_content_pane_bounds_for` 的 `PairPane { … }` 里各加 `content_first: true,`。
3. `pair_content_pane_bounds_for` 的 `compute` 闭包里，把

```rust
            let cols = pair_columns(pair_w, pane.split, !mirrored);
```
改为
```rust
            let cols = pair_columns(
                pair_w,
                pane.split,
                if pane.content_first { !mirrored } else { mirrored },
            );
```
4. 新增（放在 `codehealth_content_pane_bounds_for` 之后）：

```rust
/// Todo 面板内容区 webview 矩形。与用量 / 代码健康度同走 `pair_content_pane_bounds_for`,
/// 差别:Todo 是"**列表在前、内容在后**"(`content_first = false`);列表列可收起
/// (`dims.todo_list_collapsed`,收起后内容独占整条配对宽);内容列顶部让出原生
/// tab 行与分割线的固定高度(`theme::geometry::todo_top_row_h_px`,渲染侧同源)。
/// 不可摆放(`!content_desired` / 该侧收起 / 不是 Todo / 放大的是另一侧)时返回零
/// 尺寸矩形;看板视图由调用方(`preview_desired`)令 `content_desired = false`。
pub fn todo_content_pane_bounds_for(
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
            kind: PanelKind::Todo,
            split: state.dims.todo_split,
            chrome_top: theme::geometry::todo_top_row_h_px(),
            content_desired,
            list_visible: !state.dims.todo_list_collapsed,
            content_first: false,
        },
    )
}
```

同时把 `PairPane` 与共享函数文档里"(Usage、CodeHealth)"改成"(Usage、CodeHealth、Todo)"。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app todo_ 2>&1 | tail -20; cargo test -p dozer-app codehealth_content usage_content 2>&1 | tail -6`
Expected: 新增几何测试 PASS；**既有** `codehealth_content_*`、`usage_content_*` 测试仍全 PASS（`content_first: true` 保持原行为）。

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/geometry.rs crates/dozer-app/src/webview_geometry.rs
git commit -m "feat(todo): geometry for the todo content webview (list-first pair, native top row inset)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: 宿主接入与事件处理（WebView 挂载但原生列表尚在）

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`（`TODO_CONTENT_ID_OFFSET`、`todo_webview` 字段与初始化、`take_todo_content_script`、`preview_desired` 的 Todo 分支）
- Modify: `crates/dozer-app/src/app/message.rs`（`TodoContentWebviewEvent`）
- Modify: `crates/dozer-app/src/app/update.rs`（事件处理、`ContentRetry`）
- Modify: `crates/dozer-app/src/runtime.rs`（IPC 路由）
- Modify: `crates/dozer-app/src/platform/window_events.rs`（脚本注入）
- Modify: `crates/dozer-app/src/extensions/todo/update.rs`（新 `Message` 变体的处理逻辑）
- Test: `extensions/todo/mod.rs` 或 `protocol.rs`（状态与新增逻辑）

**Interfaces:**
- Consumes: Task 5 的 `TodoWebviewEvent`、`route_event`、`Routed`、`WebviewPushState`、`current_view_payload`、`encode_todo_push`、`parse_todo_event`；Task 6 的 `todo_content_pane_bounds_for`。
- Produces: `crate::app::TODO_CONTENT_ID_OFFSET`；`App.todo_webview`；`App::take_todo_content_script`；`App::todo_content_event`；`Message::TodoContentWebviewEvent`。

> **本任务结束时**：webview 会被挂载并收到推送，但此时原生 iced 列表还画在下面（Task 8 才替换）。目的是先让宿主链路可单独验证，再做破坏性的删除。

- [ ] **Step 1: 写失败的测试（新增逻辑与状态）**

在 `extensions/todo/mod.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn start_flash_bumps_scroll_nonce_and_sets_selected_id() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(5, "新"), todo_info(6, "旧")],
            ..WorkspaceState::default()
        };
        assert_eq!(ws_state.scroll_nonce, 0);
        assert_eq!(ws_state.selected_id(), None);
        ws_state.start_flash(0);
        assert_eq!(ws_state.scroll_nonce, 1);
        assert_eq!(ws_state.selected_id(), Some(5));
        ws_state.start_flash(1);
        assert_eq!(ws_state.scroll_nonce, 2);
        assert_eq!(ws_state.selected_id(), Some(6));
    }

    #[test]
    fn set_add_input_height_clamps_to_bounds() {
        let mut ws_state = WorkspaceState::default();
        ws_state.set_add_input_height(10_000.0);
        assert!(ws_state.add_input_height() <= ADD_INPUT_MAX_HEIGHT);
        ws_state.set_add_input_height(1.0);
        assert!(ws_state.add_input_height() >= ADD_INPUT_MIN_HEIGHT);
    }
```

（`ADD_INPUT_MAX_HEIGHT` / `ADD_INPUT_MIN_HEIGHT` 在 `state.rs` 里已有；若不是 `pub(crate)` 则放宽可见性。）

Run: `cargo test -p dozer-app extensions::todo 2>&1 | tail -8`
Expected: 这两个测试应在 Task 5 已加的 `scroll_nonce` / `selected_id` 基础上 PASS；若 `set_add_input_height` 测试暴露常量不可见，修可见性后重跑。（它们是对 Task 5 改动的回归保护，不是 RED→GREEN 的新行为。）

- [ ] **Step 2: 实现 `extensions/todo/update.rs` 里新 `Message` 变体的处理**

先抽出新增任务的共享逻辑，让 `AddSubmit` 与 `AddText` 共用（把 `AddSubmit` 里"插入乐观项 + `start_flash(0)` + spawn `add_todo`"那一段挪进下面的函数，`AddSubmit` 只负责读草稿、trim、清草稿后调用它）：

```rust
/// 新增任务的共享逻辑:插入乐观项(置顶)、起闪光、异步落库。`AddSubmit`(原生
/// 草稿)与 `AddText`(webview 文本)共用。
fn submit_new_todo(
    ws_state: &mut WorkspaceState,
    text: String,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
) {
    ws_state.items.insert(
        0,
        TodoInfo {
            id: OPTIMISTIC_TODO_ID,
            project_id,
            text: text.clone(),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        },
    );
    ws_state.start_flash(0);
    let client = client.clone();
    handle.spawn(async move {
        let res = client
            .add_todo(project_id, &text)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string());
        emit(Message::Mutated(res));
    });
}
```

`AddSubmit` 分支改为：

```rust
        Message::AddSubmit => {
            let text = ws_state.add_draft.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            ws_state.add_draft = iced_widget::text_editor::Content::new();
            submit_new_todo(ws_state, text, project_id, client, handle, emit);
        }
```

把 Task 5 里给的 6 个空分支替换为：

```rust
        Message::AddText(text) => {
            submit_new_todo(ws_state, text, project_id, client, handle, emit);
        }
        Message::EditText(id, text) => {
            let text = text.trim().to_string();
            let Some(item) = ws_state.items.iter().find(|i| i.id == id) else {
                return;
            };
            if text.is_empty() || item.text == text {
                return;
            }
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .edit_todo_text(id, &text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::ReorderTo { id, after_id } => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .reorder_todo(id, after_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::SetCategory(id, category_id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .set_todo_category(id, category_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                // 与原分类选择器同一条刷新路径:分类与任务列表一并重拉。
                emit(Message::CategoryMutated(res));
            });
        }
        Message::AddHeight(px) => ws_state.set_add_input_height(px),
        Message::ContentRetry => {}
```

- [ ] **Step 3: 宿主接入——`app.rs`**

1. 紧接 `CODEHEALTH_CONTENT_ID_OFFSET` 常量（`app.rs` 约 656 行）：

```rust
/// Todo 面板内容区 webview 的固定槽位 ID(接在 `CODEHEALTH_CONTENT_ID_OFFSET` 之后,
/// 避免与其它偏移冲突)。
pub(crate) const TODO_CONTENT_ID_OFFSET: usize = 6_000_000;
```

2. `App` 结构体里 `codehealth_webview` 字段（约 470 行）之后加：

```rust
    pub(crate) todo_webview: crate::extensions::todo::WebviewPushState,
```
构造处（约 849 行 `codehealth_webview: …default(),` 之后）加：
```rust
            todo_webview: crate::extensions::todo::WebviewPushState::default(),
```

3. `take_codehealth_content_script` 之后加：

```rust
    /// Todo 内容区待下发推送。声明式:每帧比较"当前该显示什么"
    /// (`current_view_payload`)与"上次送达的"(`todo_webview.pending_push`),
    /// 同时驱动加载超时判定。已失败时不推送——原生占位页接管。
    pub fn take_todo_content_script(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
        now: std::time::Instant,
    ) -> Vec<(usize, String)> {
        let webview_id = TODO_CONTENT_ID_OFFSET;
        self.todo_webview
            .observe_availability(available_webview_ids.contains(&webview_id), now);
        if !available_webview_ids.contains(&webview_id) || self.todo_webview.failed().is_some() {
            return Vec::new();
        }
        let Some(project_id) = self.active_project_id else {
            return Vec::new();
        };
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let desired = crate::extensions::todo::current_view_payload(
            ws,
            project_id,
            crate::extensions::todo::today_ymd(),
        );
        let Some(payload) = self.todo_webview.pending_push(&desired) else {
            return Vec::new();
        };
        let revision = self.todo_webview.mark_sent(payload.clone());
        let envelope = crate::extensions::todo::encode_todo_push(revision, payload);
        vec![(webview_id, crate::preview::dispatch_script(&envelope))]
    }
```

4. `preview_desired` 里，紧接 `if kind == PanelKind::CodeHealth { … continue; }` 之后加：

```rust
            if kind == PanelKind::Todo {
                // 已失败(加载超时/渲染异常)时不挂载,原生占位页接管;看板视图是
                // 原生占位,同样不挂载。列表列收起时矩形由几何函数按
                // `todo_list_collapsed` 处理(内容独占整条配对宽),不在这里隐藏。
                let content_desired = self.todo_webview.failed().is_none()
                    && ws.todo.view() == crate::extensions::todo::TodoView::List;
                let bounds = crate::webview_geometry::todo_content_pane_bounds_for(
                    side,
                    window_width,
                    window_height,
                    &self.shell_state(),
                    content_desired,
                );
                if bounds.2 > 0.0 && bounds.3 > 0.0 {
                    let spec = WebviewSpec {
                        id: TODO_CONTENT_ID_OFFSET,
                        url: format!(
                            "dozer://todo-content/host.html?theme={}",
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

- [ ] **Step 4: `message.rs`、`runtime.rs`、`window_events.rs`**

`app/message.rs`，紧接 `CodeHealthContentWebviewEvent(...)`（约 629 行）加：

```rust
    /// Todo 内容区 webview 发回的事件(固定单槽,按固定 webview id 识别)。
    TodoContentWebviewEvent(crate::extensions::todo::TodoWebviewEvent),
```

`runtime.rs`，在 `webview_id == crate::app::CODEHEALTH_CONTENT_ID_OFFSET` 分支之后、`CONVERSATION_REVIEW_ID_OFFSET` 分支之前加：

```rust
                                } else if webview_id == crate::app::TODO_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // todo-content 同 codehealth-content:固定单槽,按固定
                                    // webview id 识别,不复用任何 binding。
                                    match crate::extensions::todo::parse_todo_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::TodoContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 todo-content IPC");
                                        }
                                    }
```

`platform/window_events.rs`，紧接代码健康度那段 `for … take_codehealth_content_script …` 之后加：

```rust
        // Todo 内容区 webview(单固定槽):声明式推送,同代码健康度节奏。
        for (webview_id, js) in
            app.take_todo_content_script(&available_webview_ids, std::time::Instant::now())
        {
            if let Some((view, _)) = webviews.get(&webview_id) {
                let _ = view.evaluate_script(&js);
            }
        }
```

- [ ] **Step 5: `app/update.rs`——事件处理与重试**

1. 在 `Message::CodeHealthContentWebviewEvent(event) => { … }` 之后加：

```rust
            Message::TodoContentWebviewEvent(event) => self.todo_content_event(event),
```

2. 在 `todo_message` 之后加：

```rust
    /// Todo 内容区 webview 事件入口。`Ready`/`Failed` 在这里直接处理;其余事件交给
    /// 纯函数 `todo::route_event` 把 `id` 映射成当前下标并校验入参,再复用现有的
    /// 下标式入口(`todo_message` / `todo_assign_agent` / `todo_detail_open`)。
    pub(crate) fn todo_content_event(&mut self, event: todo::TodoWebviewEvent) {
        match &event {
            todo::TodoWebviewEvent::Ready => {
                self.todo_webview.set_ready(true);
                return;
            }
            todo::TodoWebviewEvent::Failed { reason } => {
                dozer_core::log_warn!(
                    LOG,
                    panel = "todo",
                    %reason,
                    "Todo 内容页渲染失败,回落原生占位"
                );
                self.todo_webview.set_failed(reason.clone());
                return;
            }
            _ => {}
        }
        let routed = self
            .active_workspace()
            .and_then(|ws| todo::route_event(ws.todo.items(), event));
        match routed {
            Some(todo::Routed::Message(msg)) => self.todo_message(msg),
            Some(todo::Routed::AssignAgent { idx, agent }) => self.todo_assign_agent(idx, agent),
            Some(todo::Routed::OpenDetail { idx }) => self.todo_detail_open(idx),
            None => {}
        }
    }
```

3. 在 `Message::Todo(msg) => match msg { … }` 的分支里（紧接 `todo::Message::CategoryContextMenuOpen(id) => …` 之前）加：

```rust
                todo::Message::ContentRetry => self.todo_webview.clear_failed(),
```

- [ ] **Step 6: 编译并检查**

```bash
cargo build 2>&1 | grep -E "^(error|warning: unused)|Finished" -A8 | head -30
cargo test -p dozer-app extensions::todo 2>&1 | tail -6
cargo test -p dozer-app webview_geometry 2>&1 | tail -4
```
Expected: 编译通过（此时 `ContentRetry` 的发出方还不存在，可能有 `dead_code` 提示，Task 8 会用到，**暂不处理**）；todo、几何测试全 PASS。

- [ ] **Step 7: 冒烟——webview 能挂载并发出 `Ready`（人工 + 日志）**

```bash
RUST_LOG=info,dozer::module::runtime=debug cargo run -p dozer-app 2>&1 | grep -i "todo-content" | head
```
手工：打开一个项目，切到 Todo 面板；此时原生 iced 列表仍画着，webview 应叠在其上（内容区会出现 webview 的页面），不应出现 `无法解析 todo-content IPC` 或 `Todo 内容页渲染失败` 日志。若 webview 没出现，检查 `preview_desired` 分支与几何矩形（`bounds` 是否非零）。**这一步在无显示环境下无法执行时，在 ledger 里明确写"未做 GUI 冒烟"，不要宣称已验证。**

- [ ] **Step 8: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/app crates/dozer-app/src/runtime.rs crates/dozer-app/src/platform/window_events.rs crates/dozer-app/src/extensions/todo
git commit -m "feat(todo): host the todo content webview (slot, push, IPC routing, event handling)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```


---

### Task 8: 切换——原生 tab 行钉高 + WebView 槽位 + 删除旧 iced 列表与弹层挂载

> 这一步是**破坏性切换**：旧的 iced 列表、搜索条、新增框、四个弹层的渲染与挂载一并删除，Todo 内容区从此只有 WebView。完成后 `cargo build` 必须通过；残余的状态字段、消息变体、焦点接线留给 Task 9，所以本任务结束时会有一批 `dead_code` 警告，**这是预期**，Task 9 逐条消除。

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/view.rs`
- Modify: `crates/dozer-app/src/app/view.rs`（删四个浮层挂载；`todo::view` 的调用点）
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`（删随函数一起失效的测试）

**Interfaces:**
- Consumes: Task 6 的 `theme::geometry::todo_top_row_inner_h_px()`；Task 7 的 `App.todo_webview`、`Message::ContentRetry`。
- Produces: `todo::view` 右栏 = 钉高的原生 tab 行 + 分割线 + `todo_content_slot`。

- [x] **Step 1: 钉 tab 按钮高度，写右栏新结构**

`todo_view_tab` 里，在 `.padding([5, 14])` 之后加一行（让 tab 行高度确定，与几何同源）：

```rust
        .height(Length::Fixed(byteui::theme::geometry::tab_button_size()))
```

在 `view.rs` 里 `kanban_placeholder` 之后新增：

```rust
/// 右栏内容区的占位槽:列表视图下 webview 作为原生子视图叠在这块区域之上,所以
/// 这里只放一个撑满的透明占位;webview 加载失败时改显示原因与「重试」
/// (不保留旧 iced 列表作回退)。
pub(crate) fn todo_content_slot<'a>(
    failed: Option<&str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    match failed {
        Some(reason) => container(
            column![
                text("Todo 页面加载失败")
                    .size(byteui::theme::font::body())
                    .color(tokens.body),
                text(reason.to_string())
                    .size(byteui::theme::font::caption())
                    .color(tokens.dim),
                button(text("重试").size(byteui::theme::font::label()))
                    .on_press(Message::ContentRetry),
            ]
            .spacing(10)
            .padding(16),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        None => container(space::Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    }
}
```

把 `view()` 里从 `// ---- 状态推导` 到 `content_pane` 组装结束的部分改为下面这样（**左栏 `sidebar_pane` 一字不改**）：

1. 删除整段 `let states: Vec<TodoState> = …;`。
2. `top_row` 与 `body` 与 `content_pane` 改为：

```rust
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Todo,
        app.list_collapsed(crate::app::PanelKind::Todo),
        crate::app::HoverId::TodoListCollapse,
        "收起",
        "展开",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::TodoListCollapse, hovered),
    );
    // 右区顶栏(原生,保留):左侧「列表视图 / 看板视图」切换 tab,右端收起/展开
    // 列表列按钮。**钉成固定高度**(`todo_top_row_inner_h_px`,tab 按钮同样钉
    // `tab_button_size()` 高),不靠内容自然撑高——几何侧
    // (`webview_geometry::todo_content_pane_bounds_for`)据 `todo_top_row_h_px`
    // 给 webview 让出这段,两侧必须同源。
    let top_row = container(
        row![
            todo_view_tab(
                "列表视图",
                TodoView::List,
                ws_state.view() == TodoView::List
            ),
            todo_view_tab(
                "看板视图",
                TodoView::Kanban,
                ws_state.view() == TodoView::Kanban
            ),
            space::Space::new().width(Length::Fill),
            collapse,
        ]
        .width(Length::Fill)
        .align_y(iced_widget::core::Alignment::Center)
        .spacing(4)
        .padding([0, 20]),
    )
    .width(Length::Fill)
    .height(Length::Fixed(theme::geometry::todo_top_row_inner_h_px()))
    .align_y(iced_widget::core::alignment::Vertical::Center);
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view() {
            TodoView::List => todo_content_slot(app.todo_webview.failed()),
            // 看板视图一期仅占位:此时 webview 不挂载(`preview_desired` 的
            // `content_desired` 为假),原生占位可见。
            TodoView::Kanban => kanban_placeholder(),
        };
    let content_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![top_row, crate::app::tab_divider(), body].height(Length::Fill))
            .width(content_width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: content_outer,
                ..container::Style::default()
            })
            .into();
```

3. `view()` 的 `ws: &Workspace` 参数不再使用：从签名删掉，并同步改 `app/view.rs` 里两处调用（`todo::view(app, &ws.todo, ws, …)` → `todo::view(app, &ws.todo, …)`）。

- [x] **Step 2: 删除 `view.rs` 里不再被引用的函数**

**保留**（左栏、tab 行、详情窗口、通用工具仍在用）：`view`、`todo_view_tab`、`kanban_placeholder`、`todo_content_slot`、`todo_clear_footer_bar`、`clear_confirm_spec`、`category_tree_nav`、`category_rename_row`、`category_pseudo_row`、`todo_detail_card`、`format_todo_month_day`、`civil_from_days`，以及这些函数直接依赖的辅助（如 `status_meta`，若详情窗口在用则保留）。

**删除**：`todo_resize_handle`、`todo_footer_bar`、`todo_search_bar`、`status_filter_label`、`status_filter_segment`、`todo_list_view`、`todo_list_row`、`drag_insert_indicator`、`todo_segment_divider`、`TodoCardArgs` 与 `todo_card`、`dispatch_items`、`status_items`、`todo_dispatch_overlay`、`todo_status_button`、`todo_status_overlay`、`todo_status_filter_overlay`、`todo_calendar_popup`、`todo_calendar_overlay`。

做法：先删上面这批，再 `cargo build`，编译器会报出被它们独占使用的 `use`、常量和辅助函数——**逐条判断**：确属只服务已删函数的就删；若某条是被保留代码引用（报"未找到"），说明保留清单漏了，恢复它并在 ledger 记一条 `Ruling`。

- [x] **Step 3: 删除 `app/view.rs` 里四个浮层挂载**

删除 `else if ws.todo.status_popup_open() { … }`、`else if ws.todo.calendar_popup_open() { … }`、`else if ws.todo.dispatch_popup_open() { … }`、`else if ws.todo.status_filter_popup_open() { … }` 四个完整分支（各自含一个 `dismiss` 与一个 `match todo::todo_*_overlay(ws, self.window_size)`），其余 `else if` 链保持。

- [x] **Step 4: 删除随函数失效的测试**

`extensions/todo/mod.rs` 的 `mod tests` 里删除 `dispatch_items_covers_the_four_headless_agents`、`status_items_covers_all_four_states_for_given_index`（它们测的是已删的 `dispatch_items` / `status_items`）。

- [x] **Step 5: 编译与回归**

```bash
cargo build 2>&1 | grep -E "^error" -A8 | head -40
cargo test -p dozer-app extensions::todo 2>&1 | tail -6
cargo test -p dozer-app webview_geometry 2>&1 | tail -4
```
Expected: 编译通过；todo 与几何测试 PASS。此时 `cargo build` 会出现一批 `dead_code` / `unused` 警告（`Message` 变体、状态字段、焦点相关函数等），**记录下来**作为 Task 9 的清单，**不要**现在 `#[allow]`。

- [x] **Step 6: GUI 冒烟（需要显示环境）** — 无显示环境,未做 GUI 冒烟

```bash
cargo run -p dozer-app
```
手工：打开项目 → Todo 面板。预期：左栏分类树与「清空列表」不变；右栏顶部是「列表视图 / 看板视图」tab 和「收起」按钮，其下是 WebView（搜索条、卡片、新增框）；切到「看板视图」tab，WebView 消失、显示看板占位；切回列表视图 WebView 恢复且数据在。若 tab 行与 WebView 之间出现缝隙或被盖住，检查 `todo_top_row_inner_h_px` 与 tab 按钮 `Fixed` 高度是否一致。**无显示环境无法执行时，在 ledger 明确写"未做 GUI 冒烟"。**

- [x] **Step 7: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src
git commit -m "refactor(todo): replace the iced list with the webview slot; drop iced popup mounts" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 9: 删除残余原生接线、状态与消息

**Files:**
- Modify: `crates/dozer-app/src/platform/window_events.rs`
- Modify: `crates/dozer-app/src/app/{app,update,state,layout}.rs`
- Modify: `crates/dozer-app/src/extensions/todo/{state,update,mod}.rs`
- Modify: `crates/dozer-app/src/main.rs`（若有对已删函数的引用，编译会指出）

**Interfaces:**
- Consumes: Task 8 结束时编译器报出的 `dead_code` 清单。

**保留清单（不要删）**：`start_flash` / `advance_flash` / `next_flash_wake` / `Flash` / `selected_row` / `scroll_nonce`；`add_input_height` / `set_add_input_height` / `ADD_INPUT_*`；左栏分类树、改名（`category_renaming`、`CaptureCategoryRenameFocus`、`category_rename_*`）；详情窗口全部（`detail_*`、`CaptureDetailReplyFocus`、`Detail*` 消息）；`Toggle(idx)`、`StatusPick(idx, TodoState)`、`CalendarPick(idx, String)`（`route_event` 在用）；`Hover(HoverId, bool)`（收起按钮在用）；`AddText`、`EditText`、`ReorderTo`、`SetCategory`、`AddHeight`、`ContentRetry`；`filter_todos_by_category` 及其测试；`parse_month_day`、`today_ymd`。

- [ ] **Step 1: 删 `platform/window_events.rs` 里 Todo 的接线**

- 删除四段 `if app.todo_dispatch_open() … / app.todo_status_open() … / app.todo_calendar_open() … / app.todo_status_filter_open() …`（各自是一个 Esc 关弹层的 `if` 块，约 742–800 行）。
- 从 `if app.files_search_focused() || … ` 的 `||` 链里删除 `app.todo_search_focused()`、`app.todo_add_focused()`、`app.todo_content_focused()` 三行。
- 删除 `let scroll_pending = app.take_todo_scroll_to_top();` 及其后对 `scroll_pending` 的使用（`scrollable::scroll_to(…TODO_LIST_SCROLL_ID…)` 那一块）。
- 删除 `let content_edit_focus_pending = app.active_workspace_mut().is_some_and(|ws| ws.take_content_edit_focus_pending());` 及 `if content_edit_focus_pending { … content_field_id() … }` 块。
- 删除 Todo 卡片拖拽的**释放路由**：`WindowEvent::MouseInput { state: Released, button: Left, .. } if app.todo_dragging() => { app.update(Message::TodoDragEnd); … }` 整个分支（约 565–572 行）；把 `app.todo_dragging() || app.dragging_tab().is_some()`（约 2760 行的重绘条件）改为 `app.dragging_tab().is_some()`。
- 删除 `todo_search_focused` / `todo_add_focused` / `content_edit_focused` 三个"每帧查真实焦点"的 `let` 块（各自调用 `CaptureTodoSearchFocus` / `CaptureAddFocus` / `CaptureContentEditFocus`），以及末尾 `app.set_todo_search_focused(…)`、`app.set_todo_add_focused(…)`、`app.set_todo_content_focused(…)` 三行。**不要**动 `category_rename_focused` 与 `detail_reply_focused` 的对应代码。

- [ ] **Step 2: 删 `app/app.rs` 里 Todo 的访问器与焦点方法**

删除：`todo_dragging`（约 2257 行）、`todo_search_focused` / `set_todo_search_focused`、`todo_add_focused` / `set_todo_add_focused`、`todo_content_focused` / `set_todo_content_focused`（及其"失焦即落盘"逻辑）、`todo_dispatch_open` / `todo_calendar_open` / `todo_status_open` / `todo_status_filter_open`（约 3033–3089 行一带）。`advance_flash` 调用点（`App` 的 `ws.todo.advance_flash()`）、`next_flash_wake`、**保留**。`take_todo_scroll_to_top` 删除。

- [ ] **Step 3: 删 `app/update.rs`、`app/layout.rs` 里的 `TodoAddGrow` 与已死的 Todo 分支**

- `app/layout.rs`：删除 `RowDivider::TodoAddGrow` 变体与其 `state.dims` 分支（约 300、1022–1025 行）。
- `app/update.rs`：删除 `RowDivider::TodoAddGrow => { … }`（约 1896 行）；`todo_message` 里删除 `AddResizeStart` 特判（`self.dragging_row = Some(RowDivider::TodoAddGrow)`）以及 `CalendarOpen` / `DispatchOpen` / `StatusOpen` / `StatusFilterOpen` 设置锚点的四个 `if matches!` 块；删除 `Message::Todo(todo::Message::AssignAgent(idx, agent)) => …` 与 `Message::Todo(todo::Message::DetailOpen(idx)) => …` 两条分支（`route_event` 直接调用 `todo_assign_agent` / `todo_detail_open`）；删除 `todo::Message::DispatchOpen(idx)` 与 `todo::Message::StatusOpen(idx)` 的 macOS 原生菜单分支。
- `app/message.rs` 删除 `TodoDragEnd` 变体，`app/update.rs` 删除 `Message::TodoDragEnd => { … }` 处理（约 1926 行）。
- `workspace/state.rs`（约 3036 行）删除 `self.todo.cancel_drag();` 一行（`cancel_drag` 随 `drag` 字段一起删除）。
- `app/state.rs`：删除只服务已删视图的 `HoverId::Todo*`（`TodoAddSubmit`、`TodoSearchSubmit` 等）；**保留** `TodoListCollapse`。以编译器 `dead_code` 为准。

- [ ] **Step 4: 删 `extensions/todo/state.rs` 里的状态字段、消息变体与无用函数**

`WorkspaceState` 删除字段：`add_draft`、`add_focused`、`scroll_to_top`、`search`、`search_draft`、`search_focused`、`dispatch_open`、`dispatch_anchor`、`status_open`、`status_anchor`、`calendar_open`、`calendar_view`、`calendar_anchor`、`editing_content`、`content_edit_focused`、`content_edit_focus_pending`、`drag`、`status_filter`、`status_filter_open`、`status_filter_anchor`。

`Message` 删除变体：`AddEdit`、`AddSubmit`、`AddResizeStart`、`RowSelect`、`SearchInput`、`SearchSubmit`、`DragMove`、`DragEnd`、`DispatchOpen`、`DispatchClose`、`AssignAgent`、`DetailOpen`、`StatusOpen`、`StatusClose`、`StatusFilterOpen`、`StatusFilterClose`、`StatusFilterPick`、`CalendarOpen`、`CalendarClose`、`CalendarPrevMonth`、`CalendarNextMonth`、`ContentEditStart`、`ContentEdit`、`CategoryPickerOpenForTodo`；同时删除 `app/update.rs` 里对 `CategoryPickerOpenForTodo` 的分支，以及 `CategoryPickerTarget::Todo(i64)` 变体与 `CategoryPickerSelect` 里的 `Todo` 分支（只剩 `Category(i64)`，整套选择器由第二份 plan 删除）。

删除类型与函数：`TodoDrag`、`StatusFilter`、`CaptureAddFocus`、`CaptureTodoSearchFocus`、`CaptureContentEditFocus`、`add_field_id`、`content_field_id`、`todo_search_field_id`、`take_add_focused`、`take_todo_search_focused`、`take_content_edit_focused`、`TODO_LIST_SCROLL_ID`、`take_scroll_to_top`、`commit_content_edit`、`set_search_focused` / `set_add_focused` / `set_content_edit_focused_flag`、`commit_search` / `clear_search`、各弹层的 `open_*` / `close_*` / `*_popup_open` / `set_*_anchor` 访问器、`drag_active` / `cancel_drag`，以及 `update.rs` 里对应已删变体的处理分支（`Message::CategorySelect` 里的 `ws_state.clear_search()` 一并去掉，搜索清空现在由前端按 `category_key` 变化完成）。

做法：先删字段与变体，`cargo build`，按编译错误逐处清理引用；最后跑下面的 grep 门禁。

- [ ] **Step 5: 迁移与清理工具函数、测试**

- 把 `format_todo_month_day` 与 `civil_from_days`（目前在 `view.rs`）移到 `update.rs`，紧挨 `today_ymd`（它们是日期工具，不再属于视图）；`mod.rs` 里 `format_month_day_from_ms` 测试保持不动（通过 `super::*` 引用）。
- 删除不再被调用的 `filter_todos`（`filter.rs`）、`first_weekday_of_month`、`days_in_month`、`days_from_civil`（若编译器报 `dead_code` 且无其它引用）。
- `mod.rs` 的 `mod tests` 删除对应测试：`filter_all_with_empty_query_keeps_everything`、`filter_by_keyword_case_insensitive_substring`、`filter_keyword_is_substring_match_on_text`、`commit_content_edit_*` 三个、`calendar_days_in_month_handles_leap_years`、`calendar_first_weekday_of_1970_jan_is_thursday`、`calendar_days_from_civil_round_trips`。**保留** `calendar_parse_month_day_accepts_mm_dd_and_rejects_bad_input`、`completed_at_for_toggle_reflects_done`、`display_state_*`、`filter_todos_by_category_*`、`visible_category_rows_*`、`category_descendants_*`、`task_title_for_session_*`、`format_month_day_from_ms`。
- `sample_states()` 若只被已删测试用则一并删。

- [ ] **Step 6: 编译、门禁、测试**

```bash
cargo build 2>&1 | grep -E "^(warning: unused|warning: .*never|error)" -A5 | head -40
grep -rnE "CaptureAddFocus|CaptureContentEditFocus|CaptureTodoSearchFocus|take_content_edit_focused|add_field_id|content_field_id|todo_search_field_id|TodoDrag|TodoAddGrow|todo_dispatch_open|todo_status_open|todo_calendar_open|todo_status_filter_open|set_todo_search_focused|set_todo_add_focused|set_todo_content_focused|take_todo_scroll_to_top|TODO_LIST_SCROLL_ID|StatusFilterPick|CategoryPickerOpenForTodo|DispatchOpen|StatusOpen|CalendarOpen|todo_dragging|TodoDragEnd|cancel_drag" crates --include='*.rs' | grep -v "^[^:]*:[0-9]*:\s*//"
cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | head
bash scripts/check-log-scope.sh
```
Expected: 编译无 `dead_code` / `unused` 警告（有则逐条判断：要么删、要么说明接线漏了并修）；grep **无输出**（注释行不计）；`dozer-app` 测试只剩基线那 1 个失败（`delete_confirm_spec_reflects_pending_target`）；门禁 `log scope check: ok`。

- [ ] **Step 7: 统计与提交**

```bash
git diff --stat main..HEAD -- crates/dozer-app/src/extensions/todo/view.rs | tail -1
git branch --show-current
git add crates/dozer-app/src
git commit -m "refactor(todo): remove native input/popup plumbing and dead state after the webview switch" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 10: 最终验证、spec 同步与收尾

**Files:**
- Modify: `docs/superpowers/specs/2026-10-02-todo-webview-design.md`（同步三处细化）
- 无新代码

- [ ] **Step 1: 全量检查**

```bash
cd crates/dozer-app/web/todo-content && npm test 2>&1 | tail -6 && npm run typecheck 2>&1 | tail -3 && cd -
cargo fmt --check && echo fmt-ok
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "generated [0-9]+ warning" | head -3
cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result|FAILED" | grep -E "FAILED"
bash scripts/check-log-scope.sh
```
Expected: 前端测试与类型检查全过；fmt ok；clippy 警告数不高于 main 基线（改动前在 main 上先记一次基线）；整包测试的失败**只有**已知基线：`delete_confirm_spec_reflects_pending_target` 与 dozerd 的 3 个 `summary_pipeline`；门禁 ok。

- [ ] **Step 2: 确认前端产物与源码一致**

```bash
cd crates/dozer-app/web/todo-content && npm run build 2>&1 | tail -2 && cd - && git status --short crates/dozer-app/assets/todo-content
```
Expected: `git status` 在该目录下**无改动**（提交的产物就是当前源码的确定性构建结果）。若有改动，说明忘了提交最新产物，提交之。

- [ ] **Step 3: 人工验收清单（需要显示环境，逐项执行并把结果写进 ledger）**

1. **拖拽排序**：只有「进行中」段可拖；放置指示线所见即所得；放回原位不触发落库；`Esc` 取消。
2. **内联编辑**：点文字进入编辑；回车提交、失焦提交、空文本丢弃；**中文输入法组合期间回车不误提交**。
3. **五个弹层**：状态、派发、状态筛选、日历、分类选择器；点空白或 `Esc` 关闭；靠近右缘 / 底部时翻转定位，不超出 WebView。选「进行中」：搁置的任务直接恢复为待办，其它的打开派发弹层。
4. **新增框**：Enter 换行、⌘↵（Ctrl+↵）提交、「↑」提交；拖拽手柄调高度，**重启后高度保持**；新增后新卡片置顶、高亮约 2 秒并滚回顶部。
5. **切换项目标签再切回来**：新增草稿、搜索词、状态筛选、滚动位置还在；切到另一个项目时数据换成该项目的。
6. **快速切换左栏分类**：无白屏闪烁；搜索被清空；新增草稿保留。
7. **收起 / 展开列表列**：内容区跟随；切到「看板视图」WebView 隐藏、显示看板占位；**面板被镜像到另一侧**时内容区位置仍与原生布局对齐（Task 6 说明的镜像分支人工验收）。
8. **键盘与剪贴板**：在搜索框、新增框、内联编辑里直接打字、⌘V 粘贴、⌘C 复制；⌘ 快捷键**不会**被转发进终端；点回 iced 区域后键盘焦点恢复。
9. **外部变更**：让 agent 通过 MCP 新增 / 删除一条任务，列表自动更新；删除一张正被点击的卡片，操作空操作且无崩溃。
10. **「详情」窗口**：点「详情」打开原生详情窗口，回复框、处理按钮照常。
11. **左栏**：分类树选择、右键菜单、「清空列表」确认框都照常。

- [ ] **Step 4: 同步 spec**

在 `docs/superpowers/specs/2026-10-02-todo-webview-design.md` 里按本 plan 开头的三处细化修改：
- 「协议 → revision 与一致性」：乐观更新改为"沿用现有 Rust 侧乐观更新（`Toggle` / 新增），前端只对拖拽排序做本地预览"；删除"取消 `Flash` / `FlashItem` 跨边界协议"的说法，改为"保留 Rust 的 `Flash` 计时，通过 payload 的 `selected_id` 与 `scroll_nonce` 带给前端"。
- 「协议 → 纯前端视图状态」：补"卡片选中高亮是前端本地状态，Rust 仅在新增后通过 `selected_id` 覆盖一次"。
- 「迁完要删除的原生代码」：把 `Flash` / `advance_flash` / `next_flash_wake` 从删除清单移出，写明保留。

```bash
git branch --show-current
git add docs/superpowers/specs/2026-10-02-todo-webview-design.md
git commit -m "docs: sync todo webview spec with the implemented flash/optimistic-update decisions" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 收尾**

按 `superpowers:finishing-a-development-branch` 处理：向用户汇报验证结果与 ledger 里的 Rulings，**合并到 main 由用户决定**。合并前检查主工作区是否有别人未提交的改动与本分支文件重叠（`comm -12 <(git diff --name-only main..feat/todo-webview | sort) <(git status --short | awk '{print $2}' | sort)`），有重叠按"用唯一标签 stash → 快进合并 → `stash apply` → 按标签 drop"的流程处理，不要碰别人的 stash。

---

## 自检记录

- **Spec 覆盖**（WebView 部分）：目标 1–3、5–7 → Task 1–9；弹层五个 → Task 3（`Popovers.tsx`）；协议与 `id` 路由 → Task 5；`revision` / `Ready` 重发 → Task 1、5；几何与原生 tab 行 → Task 6、8；宿主接入（ID 偏移、指令队列、IPC、失败占位页）→ Task 7、8；要删除的原生代码 → Task 8、9；错误与降级 → Task 7（`Failed` / 超时）、8（占位页 + 重试）；测试与验收 → 各任务 + Task 10。**目标 4（左栏拖动）不在本 plan**，见第二份 plan。
- **与 spec 的差异**已在文首列出并在 Task 10 同步 spec。
- **类型一致**：`TodoViewPayload` 字段在 Rust（Task 5）与 TS（`types.ts`，Task 1）逐字段对应（`project_id`、`category_key`、`items`、`categories`、`agents`、`add_height_px`、`today{year,month,day}`、`selected_id`、`scroll_nonce`）；事件 `kind` 取值（`ready`/`failed`/`add`/`toggle`/`edit_text`/`reorder`/`set_status`/`set_plan_date`/`assign_agent`/`set_category`/`open_detail`/`add_height`）在 TS `OutEvent` 与 Rust `TodoWebviewEvent` 一致；`Routed` 与 `Message` 新变体在 Task 5 定义、Task 7 实现。
- **已知需要以编译器为准的点**：`WorkspaceState` 字段可见性、`ADD_INPUT_*` 常量可见性、`CategoryInfo` 构造所需字段、`App.tsx` 里 `typeof payload.items` 的类型引用、`todo::view` 删参后的调用点——均在对应步骤标明，行为保持不变。
