# Tabular 预览渲染层迁移到 WebView Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 tabular 预览面板(sheet 切换条 + 截断提示 + 单元格网格)从 iced canvas 手绘迁移到 webview host(ag-grid Community + Infinite Row Model),直接替换、不留对照期,同时保留现有加载性能(100k 行封顶、取消)、agent reveal 导航与会话持久化。

**Architecture:** 新增第 4 种 webview host(`dozer://tabular/`),复用 CodeMirror/vanilla-jsoneditor 已建立的 `WebviewEnvelope<T>` push 协议模式与 `EditorHostBinding`(直接扩展,不新建绑定类型)。数据加载层(`calamine`/`csv`,`tabular/mod.rs`)完全不动;Rust 侧新增"host 已就绪"与"数据已就绪"两路异步汇合逻辑,汇合后一次性推 `Init`+`SetSchema`+`SetWindow`,首帧 ACK(`WindowApplied`)后才 `finish_load`。后续滚动触发的 `WindowRequest` 是对已在内存的 `Sheet.rows` 做零 IO 切片,不是磁盘窗口化。

**Tech Stack:** Rust(iced 0.14 / wry / serde_json)+ TypeScript(esbuild IIFE 打包)+ `ag-grid-community`(MIT)。

**Spec:** `docs/superpowers/specs/2026-09-28-tabular-webview-migration-design.md`

## Global Constraints

- **只读,恒不可编辑**:ag-grid `editable: false`;不启用 `sortable`/`filter`(会打乱行序,与 agent reveal 的行列坐标假设冲突)。
- **严格 CSP,无网络**:`default-src 'none'`,JS/CSS 全部本地打包,不引 CDN(同 `web/json-editor` 现有约束)。
- **系统默认字体,禁止等宽代码字体**:tabular host 不带 `fs=`/`lh=` URL 参数,CSS 不引入 `JetBrains Mono`(对齐 CLAUDE.md"非代码/终端场景禁用等宽字体"裁决)。
- **数据加载层不动**:`tabular/mod.rs` 的 `load`/`load_cancellable`/`load_sheet*`/`Sheet`/`MAX_TABULAR_ROWS`/`TABULAR_CANCELLED` 一律不改。
- **直接切换**:不新增 feature flag 做对照期;`TabularMode::Grid` 一律走新 host(与 `codemirror_enabled()`/`json_editor_enabled()` 保持"始终开"的既有风格一致,新增 `tabular_grid_host_enabled()` 恒返回 `true`)。
- **复用而非新建绑定类型**:`dozer://tabular/` 复用 `EditorHostBinding`(project_id/panel/tab_id/path 形状与 editor/json host 完全一致),只加 `tabular_url()` 方法,不新建 `TabularHostBinding` 结构体。
- **分支要求**:本计划在独立分支上开发(建议 `feat/tabular-webview-migration`,从当前 `main` 切出),完成后按 `superpowers:finishing-a-development-branch` 走代码审阅再合并,不直接在 `main` 上开发。

## Review Focus

- **两个异步就绪源的先后顺序**:host `Ready` 可能先到或后到于后台解析线程的 `TabularLoaded`——两种顺序都必须恰好推送一次 `Init`+`SetSchema`+`SetWindow`,不多推、不漏推、不 panic。
- **空 sheet(0 行 / 0 列)**:损坏文件或空 CSV 产出 `col_count=0`/`total_rows=0` 时,JS 侧 ag-grid 初始化与 Rust 侧行切片都不能除零/越界。
- **快速连续切 sheet**:用户在上一个 `SheetSelected` 的 `WindowApplied` ACK 抵达前又切了下一个 sheet,旧 ACK 必须按 `revision`/`load_state.generation` 判过期丢弃,不能把旧 sheet 的窗口误盖到新 sheet 上。
- **持久化恢复的 sheet 下标越界**:文件被外部改过,重开 tab 时 `PersistedTabular.sheet` 可能超出新的 `sheet_names.len()`——现有 `reveal_cell`/`reveal_range`/`select_sheet` 已做钳位,迁移后必须继续钳位而不是让 JS 收到越界 `sheet_index` 崩溃。
- **agent reveal 打在尚未加载的 sheet 上**:`RevealCell` 命中未加载 sheet 时当前返回 `LoadDenied`(不自动加载),迁移后这个行为必须保持不变,不能变成静默失败或直接崩溃。

---

## File Structure

**新增:**
- `crates/dozer-app/web/tabular-host/{package.json,build.mjs,tsconfig.json,src/index.html,src/main.ts,src/theme.css}` — JS host 源码。
- `crates/dozer-app/assets/tabular-host/` — esbuild 构建产物(不手写,`npm run build` 生成)。

**修改:**
- `crates/dozer-app/src/preview/webview_protocol.rs` — 新增 `TabularCommand`/`TabularEvent`/`parse_tabular_event`/`encode_tabular_command`。
- `crates/dozer-app/src/preview/code_host.rs` — 新增 `TABULAR_URL_PREFIX`/`tabular_url()`/`is_tabular_url()`/`tabular_host_enabled()`,更新 `is_host_url()`。
- `crates/dozer-app/src/assets.rs` — 新增 `dozer://tabular/` 的 CSP + serve 分支。
- `crates/dozer-app/src/runtime.rs` — IPC handler 新增 `is_tabular_host` 分支。
- `crates/dozer-app/src/app/message.rs` — 新增 `Message::TabularHostEvent`。
- `crates/dozer-app/src/preview/state.rs` / `crates/dozer-app/src/preview/backend.rs` — 新增 `uses_tabular_grid_host()`,更新 `hosts_any_webview()`;`PreviewTab` 新增 `tabular_host_ready: bool` 字段。
- `crates/dozer-app/src/preview/view.rs` — 新增 `desired_tabular_webviews()`/`queue_tabular_command()`/`take_pending_tabular_commands_for()`/`push_sheet_schema_and_window()`;改造 `finish_tabular_load()` 延迟到 `WindowApplied` 才 `finish()`;`apply_preview_command` 的 `RevealCell` 分支补发 `RevealRange` 命令。
- `crates/dozer-app/src/app/app.rs` — 新增 `take_preview_tabular_scripts()`;`desired_webviews()` 聚合处新增 tabular 分支。
- `crates/dozer-app/src/platform/window_events.rs` — `apply_pending_editor_commands()` 新增 tabular 派发循环。
- `crates/dozer-app/src/app/update.rs` — 新增 `Message::TabularHostEvent` 处理;`TabularLoaded` 的恢复分支改调用新签名;扩展 `Message::TabularSheetLoaded` 补推 `SetSheetLoading`/`SelectSheet`/schema+window。
- `crates/dozer-app/src/workspace/state.rs` — `preview_pane_tabular_action` 签名从 `Action` 改为 `sheet: usize`,懒加载 spawn 前补发 `SetSheetLoading{loading:true}`。
- `crates/dozer-app/src/tabular/mod.rs` — `select_sheet` 转 `pub`,删除 `Action` 枚举与 `apply()`。
- `crates/dozer-app/src/workspace/view.rs` — 删除表格 tab 的 iced 原生渲染分支(1294-1334 行)。

**删除:**
- `crates/dozer-app/src/tabular/grid.rs`
- `crates/dozer-app/src/tabular/view.rs`
- `Message::TabularAction` 及其唯一处理分支(`app/update.rs:2335-2339`)

---

### Task 1: 建分支

**Files:** 无代码改动。

- [ ] **Step 1: 确认工作区干净并切分支**

```bash
git status --short   # 确认无未提交改动;若有,先处理/stash
git checkout main && git pull --ff-only
git checkout -b feat/tabular-webview-migration
```

- [ ] **Step 2: 确认 Node 工具链可用(后续 JS 任务要用)**

```bash
node --version   # 需要能跑 esbuild/tsc,参考 web/json-editor 现有要求
```

---

### Task 2: JS host 骨架(打包 + CSP + 协议 envelope 收发,不含 ag-grid)

**Files:**
- Create: `crates/dozer-app/web/tabular-host/package.json`
- Create: `crates/dozer-app/web/tabular-host/tsconfig.json`
- Create: `crates/dozer-app/web/tabular-host/build.mjs`
- Create: `crates/dozer-app/web/tabular-host/src/index.html`
- Create: `crates/dozer-app/web/tabular-host/src/theme.css`
- Create: `crates/dozer-app/web/tabular-host/src/main.ts`

**Interfaces:**
- Produces:构建产物 `crates/dozer-app/assets/tabular-host/{index.html,tabular-host.js,tabular-host.css}`,供 Task 6(`assets.rs`)serve。
- Produces:`window.__dozer.dispatch(rawJson: string): void`(供 Rust `dispatch_script` 注入调用,同 CodeMirror/JSON host 约定)。
- Produces:JS → Rust envelope 字段形状(`protocol_version/project_id/panel/tab_id/document_id/revision/request_id/payload`),`payload.kind` 取值先只有 `ready`(本任务只搭骨架,`ag-grid` 命令在 Task 3 补)。

- [ ] **Step 1: package.json**

```json
{
  "name": "dozer-tabular-host",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Dozer tabular preview host (ag-grid Community), offline bundle.",
  "scripts": {
    "build": "node build.mjs",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "ag-grid-community": "32.3.3"
  },
  "devDependencies": {
    "esbuild": "0.28.2",
    "typescript": "5.9.3",
    "@types/node": "24"
  }
}
```

- [ ] **Step 2: tsconfig.json**(照抄 `web/json-editor/tsconfig.json` 的严格档;若该文件内容与下方不同,以现有 `web/json-editor/tsconfig.json` 为准直接复制再改 `include`)

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "types": ["node"]
  },
  "include": ["src"]
}
```

- [ ] **Step 3: build.mjs**

```js
// 生产构建:把 ag-grid-community 打包成**离线、无 CDN** 的确定性产物到
// `crates/dozer-app/assets/tabular-host/`(index.html 自带严格 CSP)。
import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/tabular-host');

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

await build({
  entryPoints: [path.join(here, 'src/main.ts')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  loader: { '.css': 'css' },
  outfile: path.join(outdir, 'tabular-host.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/index.html'), path.join(outdir, 'index.html'));

console.log('built ->', outdir);
```

- [ ] **Step 4: src/index.html**

```html
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <!-- 严格 CSP:脚本/样式仅 self,禁网络/frame/任意导航。ag-grid 会注入
         内联 style(单元格宽度等),故 style-src 需 'unsafe-inline'。 -->
    <meta
      http-equiv="Content-Security-Policy"
      content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Dozer Tabular</title>
  </head>
  <body>
    <div id="tabular-root" style="width:100%;height:100%"></div>
    <script src="tabular-host.js"></script>
  </body>
</html>
```

- [ ] **Step 5: src/theme.css**(先放最小骨架,视觉细节留给 Task 3)

```css
html,
body {
  margin: 0;
  height: 100%;
  background: #0a0e16;
  overflow: hidden;
}
html[data-theme='light'],
html[data-theme='light'] body {
  background: #fef2e4;
}
#tabular-root {
  height: 100%;
}
```

- [ ] **Step 6: src/main.ts**(只做绑定解析 + envelope 收发骨架,不含 ag-grid)

```ts
// Dozer tabular 预览 host 入口。与 CodeMirror/JSON host 共用消息 envelope
// 结构(见 Rust `preview/webview_protocol.rs::TabularCommand`/`TabularEvent`)。
// 正文完全由 Rust 推送(Init/SetSchema/SetWindow),本 host 不 fetch 任何文件。
import './theme.css';

const PROTOCOL_VERSION = 1;

interface Binding {
  project_id: number;
  panel: string;
  tab_id: number;
  document_id: string;
}

const params = new URLSearchParams(location.search);
const binding: Binding = {
  project_id: Number(params.get('proj') ?? '0') || 0,
  panel: params.get('panel') ?? 'files',
  tab_id: Number(params.get('tab') ?? '0') || 0,
  document_id: params.get('doc') ?? '',
};
const scheme = params.get('theme') === 'light' ? 'light' : 'dark';
document.documentElement.dataset.theme = scheme;

let revision = 0;

function post(payload: Record<string, unknown>): void {
  const env = {
    protocol_version: PROTOCOL_VERSION,
    project_id: binding.project_id,
    panel: binding.panel,
    tab_id: binding.tab_id,
    document_id: binding.document_id,
    revision,
    request_id: null,
    payload,
  };
  const ipc = (window as unknown as { ipc?: { postMessage: (s: string) => void } }).ipc;
  ipc?.postMessage(JSON.stringify(env));
}

function applyCommand(raw: string): void {
  let env: { protocol_version?: number; payload?: { kind?: string; [k: string]: unknown } };
  try {
    env = JSON.parse(raw);
  } catch {
    return;
  }
  if (env.protocol_version !== PROTOCOL_VERSION) return;
  const cmd = env.payload;
  if (!cmd || typeof cmd.kind !== 'string') return;
  // Task 3 在这里按 cmd.kind 补 init/set_schema/set_sheet_loading/set_window/
  // reveal_range/restore_view_state 的实际处理。
  void cmd;
}

(window as unknown as { __dozer?: unknown }).__dozer = { dispatch: applyCommand };

post({ kind: 'ready' });
```

- [ ] **Step 7: 安装依赖并构建,确认产物生成**

```bash
cd crates/dozer-app/web/tabular-host
npm install
npm run typecheck
npm run build
ls ../../assets/tabular-host   # 应看到 index.html / tabular-host.js
```

Expected: `typecheck` 无错误;`build` 打印 `built -> .../assets/tabular-host`,目录下有两个文件。

- [ ] **Step 8: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git add crates/dozer-app/web/tabular-host crates/dozer-app/assets/tabular-host
git commit -m "feat(tabular): scaffold offline webview host bundle (no ag-grid yet)"
```

---

### Task 3: JS host:接入 ag-grid Infinite Row Model + 视觉

**Files:**
- Modify: `crates/dozer-app/web/tabular-host/src/main.ts`
- Modify: `crates/dozer-app/web/tabular-host/src/theme.css`

**Interfaces:**
- Consumes:无(本任务是叶子 JS 实现,不依赖其它任务产出的 Rust 类型——命令/事件的字段形状由本任务与 Task 4 共同约定,双方对齐下表)。
- Produces:JS → Rust 事件 `kind` 取值:`ready` / `sheet_selected{index}` / `window_request{sheet_index,start_row,end_row}` / `window_applied{start_row}` / `failed{message,recoverable}`。
- Produces:接受 Rust → JS 命令 `kind` 取值:`init{sheet_names,active_sheet,read_only}` / `set_schema{sheet_index,col_count,total_rows,truncated,col_widths}` / `set_sheet_loading{sheet_index,loading}` / `select_sheet{sheet_index}`(Rust 主动切 sheet,如会话恢复到非首个 sheet,区别于用户点 tab 触发的 `sheet_selected` 事件)/ `set_window{sheet_index,start_row,rows,revision}` / `reveal_range{sheet_index,r1,c1,r2,c2}` / `restore_view_state{sheet_index,start_row,start_col}`。

- [ ] **Step 1: 重写 `src/main.ts`,加入 sheet tab 栏 + ag-grid Infinite Row Model**

```ts
import { createGrid, type GridApi, type IGetRowsParams, type GridOptions } from 'ag-grid-community';
import './theme.css';

const PROTOCOL_VERSION = 1;

interface Binding {
  project_id: number;
  panel: string;
  tab_id: number;
  document_id: string;
}

const params = new URLSearchParams(location.search);
const binding: Binding = {
  project_id: Number(params.get('proj') ?? '0') || 0,
  panel: params.get('panel') ?? 'files',
  tab_id: Number(params.get('tab') ?? '0') || 0,
  document_id: params.get('doc') ?? '',
};
const scheme = params.get('theme') === 'light' ? 'light' : 'dark';
document.documentElement.dataset.theme = scheme;

let revision = 0;
let readOnly = true;
let sheetNames: string[] = [];
let activeSheet = 0;
// 每个 sheet 独立的 schema/grid 实例(切 sheet 不销毁,懒加载完成前禁用点击态由
// tab 上的 loading class 表达)。
interface SheetState {
  colCount: number;
  totalRows: number;
  truncated: boolean;
  colWidths: number[];
  loading: boolean;
  api: GridApi | null;
  container: HTMLDivElement;
}
const sheets = new Map<number, SheetState>();

function post(payload: Record<string, unknown>): void {
  const env = {
    protocol_version: PROTOCOL_VERSION,
    project_id: binding.project_id,
    panel: binding.panel,
    tab_id: binding.tab_id,
    document_id: binding.document_id,
    revision,
    request_id: null,
    payload,
  };
  const ipc = (window as unknown as { ipc?: { postMessage: (s: string) => void } }).ipc;
  ipc?.postMessage(JSON.stringify(env));
}

function excelColumnLabel(idx: number): string {
  let n = idx;
  let label = '';
  do {
    label = String.fromCharCode(65 + (n % 26)) + label;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return label;
}

const tabBar = document.getElementById('tab-bar') as HTMLDivElement;
const gridHost = document.getElementById('grid-host') as HTMLDivElement;
const banner = document.getElementById('truncated-banner') as HTMLDivElement;

function renderTabs(): void {
  tabBar.innerHTML = '';
  if (sheetNames.length <= 1) {
    tabBar.style.display = 'none';
    return;
  }
  tabBar.style.display = 'flex';
  sheetNames.forEach((name, idx) => {
    const btn = document.createElement('button');
    btn.textContent = name;
    btn.className = 'sheet-tab' + (idx === activeSheet ? ' active' : '');
    if (sheets.get(idx)?.loading) btn.classList.add('loading');
    btn.addEventListener('click', () => {
      if (idx === activeSheet) return;
      activeSheet = idx;
      post({ kind: 'sheet_selected', index: idx });
      renderTabs();
      showActiveSheet();
    });
    tabBar.appendChild(btn);
  });
}

function ensureSheetState(sheetIndex: number): SheetState {
  let state = sheets.get(sheetIndex);
  if (state) return state;
  const container = document.createElement('div');
  container.className = 'sheet-grid';
  container.style.display = 'none';
  gridHost.appendChild(container);
  state = {
    colCount: 0,
    totalRows: 0,
    truncated: false,
    colWidths: [],
    loading: false,
    api: null,
    container,
  };
  sheets.set(sheetIndex, state);
  return state;
}

function buildGrid(sheetIndex: number, state: SheetState): void {
  const rowNumberCol = {
    headerName: '',
    valueGetter: (p: { node: { rowIndex: number | null } }) =>
      p.node.rowIndex === null ? '' : p.node.rowIndex + 1,
    pinned: 'left' as const,
    width: 64,
    sortable: false,
    filter: false,
    cellClass: 'row-number-cell',
  };
  const dataCols = Array.from({ length: state.colCount }, (_, c) => ({
    headerName: excelColumnLabel(c),
    field: `c${c}`,
    sortable: false,
    filter: false,
    // 非目标:不做列宽拖拽调整(spec"非目标"一节明确排除),固定宽度。
    resizable: false,
    width: Math.max(64, Math.min(320, (state.colWidths[c] ?? 12) * 8)),
  }));
  const options: GridOptions = {
    columnDefs: [rowNumberCol, ...dataCols],
    rowModelType: 'infinite',
    cacheBlockSize: 200,
    maxBlocksInCache: 10,
    rowHeight: 26,
    headerHeight: 28,
    animateRows: false,
    suppressCellFocus: false,
    getRowId: undefined,
    datasource: {
      getRows: (p: IGetRowsParams) => {
        post({
          kind: 'window_request',
          sheet_index: sheetIndex,
          start_row: p.startRow,
          end_row: p.endRow,
        });
        // 结果由 Rust 经 `set_window` 命令异步推回(见 applySetWindow),
        // 不在这里同步 resolve/fail——ag-grid Infinite Row Model 允许块
        // 在稍后经 `api.setRowCount`/`applyRowData` 补齐(见 applySetWindow)。
        pendingGetRows.set(`${p.startRow}:${p.endRow}`, p);
      },
    },
  };
  state.api = createGrid(state.container, options);
}

const pendingGetRows = new Map<number | string, import('ag-grid-community').IGetRowsParams>();

function showActiveSheet(): void {
  for (const [idx, s] of sheets) {
    s.container.style.display = idx === activeSheet ? 'block' : 'none';
  }
  const state = sheets.get(activeSheet);
  banner.style.display = state?.truncated ? 'block' : 'none';
}

function applyInit(cmd: { sheet_names: string[]; active_sheet: number; read_only: boolean }): void {
  sheetNames = cmd.sheet_names;
  activeSheet = cmd.active_sheet;
  readOnly = cmd.read_only;
  void readOnly;
  renderTabs();
}

function applySetSchema(cmd: {
  sheet_index: number;
  col_count: number;
  total_rows: number;
  truncated: boolean;
  col_widths: number[];
}): void {
  const state = ensureSheetState(cmd.sheet_index);
  state.colCount = cmd.col_count;
  state.totalRows = cmd.total_rows;
  state.truncated = cmd.truncated;
  state.colWidths = cmd.col_widths;
  state.loading = false;
  if (!state.api) buildGrid(cmd.sheet_index, state);
  // 空表(0 行/0 列)不设 rowCount,ag-grid 直接渲染空网格,不发 getRows。
  state.api?.setGridOption('rowCount', Math.max(0, cmd.total_rows));
  renderTabs();
  showActiveSheet();
}

function applySetSheetLoading(cmd: { sheet_index: number; loading: boolean }): void {
  const state = ensureSheetState(cmd.sheet_index);
  state.loading = cmd.loading;
  renderTabs();
}

function applySetWindow(cmd: {
  sheet_index: number;
  start_row: number;
  rows: string[][];
  revision: number;
}): void {
  const state = sheets.get(cmd.sheet_index);
  if (!state?.api) return;
  const endRow = cmd.start_row + cmd.rows.length;
  for (const [key, p] of pendingGetRows) {
    if (p.startRow !== cmd.start_row) continue;
    const rowData = cmd.rows.map((row) =>
      Object.fromEntries(row.map((v, i) => [`c${i}`, v])),
    );
    p.successCallback(rowData, state.totalRows);
    pendingGetRows.delete(key);
    if (cmd.start_row === 0) {
      post({ kind: 'window_applied', start_row: cmd.start_row });
    }
    break;
  }
  void endRow;
}

function applySelectSheet(cmd: { sheet_index: number }): void {
  // Rust 主动驱动的切 sheet(会话恢复到非首个 sheet 的场景)——不是用户
  // 点了 tab,所以只更新高亮 + 显示,不回发 `sheet_selected`(避免来回)。
  if (cmd.sheet_index === activeSheet) return;
  activeSheet = cmd.sheet_index;
  renderTabs();
  showActiveSheet();
}

function applyRevealRange(cmd: {
  sheet_index: number;
  r1: number;
  c1: number;
  r2: number;
  c2: number;
}): void {
  if (cmd.sheet_index !== activeSheet) {
    activeSheet = cmd.sheet_index;
    renderTabs();
    showActiveSheet();
  }
  const state = sheets.get(cmd.sheet_index);
  state?.api?.ensureIndexVisible(cmd.r1, 'top');
  state?.api?.ensureColumnVisible(`c${cmd.c1}`);
}

function applyRestoreViewState(cmd: {
  sheet_index: number;
  start_row: number;
  start_col: number;
}): void {
  const state = sheets.get(cmd.sheet_index);
  state?.api?.ensureIndexVisible(cmd.start_row, 'top');
  state?.api?.ensureColumnVisible(`c${cmd.start_col}`);
}

function applyCommand(raw: string): void {
  let env: { protocol_version?: number; revision?: number; payload?: { kind?: string; [k: string]: unknown } };
  try {
    env = JSON.parse(raw);
  } catch {
    return;
  }
  if (env.protocol_version !== PROTOCOL_VERSION) return;
  const cmd = env.payload;
  if (!cmd || typeof cmd.kind !== 'string') return;
  if (typeof env.revision === 'number') revision = env.revision;
  switch (cmd.kind) {
    case 'init':
      applyInit(cmd as Parameters<typeof applyInit>[0]);
      break;
    case 'set_schema':
      applySetSchema(cmd as Parameters<typeof applySetSchema>[0]);
      break;
    case 'set_sheet_loading':
      applySetSheetLoading(cmd as Parameters<typeof applySetSheetLoading>[0]);
      break;
    case 'select_sheet':
      applySelectSheet(cmd as Parameters<typeof applySelectSheet>[0]);
      break;
    case 'set_window':
      applySetWindow(cmd as Parameters<typeof applySetWindow>[0]);
      break;
    case 'reveal_range':
      applyRevealRange(cmd as Parameters<typeof applyRevealRange>[0]);
      break;
    case 'restore_view_state':
      applyRestoreViewState(cmd as Parameters<typeof applyRestoreViewState>[0]);
      break;
    default:
      break;
  }
}

(window as unknown as { __dozer?: unknown }).__dozer = { dispatch: applyCommand };

post({ kind: 'ready' });
```

- [ ] **Step 2: `src/index.html` 加入骨架元素**(`#tabular-root` 内部结构,Task 2 只有一个空 div)

```html
<div id="tabular-root" style="width:100%;height:100%;display:flex;flex-direction:column">
  <div id="tab-bar"></div>
  <div id="truncated-banner">仅显示前 100,000 行(文件还有更多,未精确统计总行数)</div>
  <div id="grid-host" style="flex:1;position:relative"></div>
</div>
```

替换 Task 2 里 `<div id="tabular-root" style="width:100%;height:100%"></div>` 那一行为上面这段。

- [ ] **Step 3: `src/theme.css` 补 ByteBoy2077 配色 + 冻结/斑马纹/只读视觉**

```css
html,
body {
  margin: 0;
  height: 100%;
  background: #0a0e16;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}
html[data-theme='light'],
html[data-theme='light'] body {
  background: #fef2e4;
}
#tabular-root {
  height: 100%;
  color: #ffe5b4;
}
html[data-theme='light'] #tabular-root {
  color: #3a3226;
}
#tab-bar {
  display: flex;
  gap: 4px;
  padding: 4px 8px;
  flex-shrink: 0;
}
.sheet-tab {
  background: transparent;
  border: none;
  color: #6b7f8f;
  padding: 4px 12px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 13px;
}
.sheet-tab.active {
  background: rgba(71, 222, 240, 0.16);
  color: #ffe5b4;
}
.sheet-tab.loading {
  opacity: 0.5;
}
#truncated-banner {
  display: none;
  padding: 4px 8px;
  font-size: 12px;
  color: #6b7f8f;
  flex-shrink: 0;
}
#grid-host,
.sheet-grid {
  height: 100%;
  width: 100%;
}
/* ag-grid 主题变量覆盖(theme-quartz 内建 CSS 变量),对齐 ByteBoy2077。 */
.ag-theme-quartz-dark,
.ag-theme-quartz {
  --ag-background-color: #0a0e16;
  --ag-header-background-color: #0d1422;
  --ag-odd-row-background-color: rgba(255, 255, 255, 0.02);
  --ag-foreground-color: #ffe5b4;
  --ag-header-foreground-color: #ffe5b4;
  --ag-border-color: rgba(107, 127, 143, 0.4);
  --ag-row-hover-color: rgba(255, 229, 180, 0.06);
  --ag-selected-row-background-color: rgba(71, 222, 240, 0.22);
  --ag-font-size: 13px;
}
.row-number-cell {
  color: #6b7f8f;
  text-align: right;
}
```

`main.ts` 的 `buildGrid` 需要给 `state.container` 加上 ag-grid 主题 class,补一行:

```ts
container.className = 'sheet-grid ag-theme-quartz-dark';
```

(放进 `ensureSheetState` 里创建 `container` 之后;light 主题时改成 `ag-theme-quartz`,可在 `applyInit` 里按 `document.documentElement.dataset.theme` 统一设置一次全局 class 而不是逐个 container 判断——`document.documentElement.classList.toggle('ag-theme-quartz-dark', scheme==='dark')` 更简单,配合 CSS 选择器 `html[data-theme] .sheet-grid` 生效即可,这里给出的是最小可行版本,允许实现者选择等价写法。)

- [ ] **Step 4: 本地无 Rust 环境下的最小验证:typecheck + build**

```bash
cd crates/dozer-app/web/tabular-host
npm run typecheck
npm run build
```

Expected: 两条都无错误退出。真正的视觉/交互验证留到 Task 14 的整机 GUI 走查(此时还没有 Rust 侧数据源可推)。

- [ ] **Step 5: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git add crates/dozer-app/web/tabular-host crates/dozer-app/assets/tabular-host
git commit -m "feat(tabular): wire ag-grid Infinite Row Model into webview host"
```

---

### Task 4: Rust 协议类型(TabularCommand/TabularEvent)

**Files:**
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs`

**Interfaces:**
- Consumes:`WebviewEnvelope<T>`、`MAX_MESSAGE_BYTES`、`ProtocolError`、`PanelKind`(均已存在于本文件)。
- Produces:`TabularCommand`(6 变体,见下)、`TabularEvent`(5 变体)、`pub fn parse_tabular_event(raw: &str) -> Result<WebviewEnvelope<TabularEvent>, ProtocolError>`、`pub fn encode_tabular_command(project_id: i64, panel: PanelKind, tab_id: usize, document_id: &str, revision: u64, request_id: Option<String>, command: TabularCommand) -> String`。这四者是后续所有 Rust 任务(5、7、8、9、11)的唯一依赖来源。

- [ ] **Step 1: 在 `JsonEvent`/`parse_json_event` 之后(约第 422 行后)插入新类型,先写测试**

在文件末尾 `#[cfg(test)] mod tests` 块内(`parses_json_document_loaded` 测试之后)加入:

```rust
    #[test]
    fn parses_tabular_events() {
        let ready = parse_tabular_event(&raw(r#"{"kind":"ready"}"#)).unwrap();
        assert_eq!(ready.payload, TabularEvent::Ready);

        let sel = parse_tabular_event(&raw(r#"{"kind":"sheet_selected","index":2}"#)).unwrap();
        assert_eq!(sel.payload, TabularEvent::SheetSelected { index: 2 });

        let wr = parse_tabular_event(&raw(
            r#"{"kind":"window_request","sheet_index":0,"start_row":100,"end_row":300}"#,
        ))
        .unwrap();
        assert_eq!(
            wr.payload,
            TabularEvent::WindowRequest {
                sheet_index: 0,
                start_row: 100,
                end_row: 300
            }
        );

        let wa = parse_tabular_event(&raw(r#"{"kind":"window_applied","start_row":0}"#)).unwrap();
        assert_eq!(wa.payload, TabularEvent::WindowApplied { start_row: 0 });

        let failed = parse_tabular_event(&raw(
            r#"{"kind":"failed","message":"boom","recoverable":true}"#,
        ))
        .unwrap();
        assert_eq!(
            failed.payload,
            TabularEvent::Failed {
                message: "boom".into(),
                recoverable: true
            }
        );

        assert!(matches!(
            parse_tabular_event(&raw(r#"{"kind":"evil"}"#)),
            Err(ProtocolError::UnknownPayload(_))
        ));
    }

    #[test]
    fn rejects_oversized_tabular_message() {
        let big = "a".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(matches!(
            parse_tabular_event(&big),
            Err(ProtocolError::TooLarge { .. })
        ));
    }

    #[test]
    fn encodes_tabular_commands() {
        let s = encode_tabular_command(
            1,
            PanelKind::Files,
            2,
            "p1-t2",
            0,
            None,
            TabularCommand::Init {
                sheet_names: vec!["Sheet1".into(), "Sheet2".into()],
                active_sheet: 0,
                read_only: true,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["payload"]["kind"], "init");
        assert_eq!(v["payload"]["sheet_names"][1], "Sheet2");

        let win = encode_tabular_command(
            1,
            PanelKind::Files,
            2,
            "p1-t2",
            0,
            None,
            TabularCommand::SetWindow {
                sheet_index: 0,
                start_row: 0,
                rows: vec![vec!["a".into(), "b".into()]],
                revision: 3,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&win).unwrap();
        assert_eq!(v["payload"]["kind"], "set_window");
        assert_eq!(v["payload"]["rows"][0][1], "b");

        let sel = encode_tabular_command(
            1,
            PanelKind::Files,
            2,
            "p1-t2",
            0,
            None,
            TabularCommand::SelectSheet { sheet_index: 3 },
        );
        let v: serde_json::Value = serde_json::from_str(&sel).unwrap();
        assert_eq!(v["payload"]["kind"], "select_sheet");
        assert_eq!(v["payload"]["sheet_index"], 3);
    }

    #[test]
    fn tabular_event_validates_against_host_binding() {
        let env = parse_tabular_event(&raw(r#"{"kind":"ready"}"#)).unwrap();
        assert!(env.validate(&binding()).is_ok());
        assert!(
            env.validate(&HostBinding::new(8, PanelKind::Files, 3, "p7-t3".into()))
                .is_err()
        );
    }
```

- [ ] **Step 2: 跑测试,确认因缺类型编译失败**

```bash
cargo test -p dozer-app --lib preview::webview_protocol -- --list 2>&1 | tail -20
```

Expected: 编译错误,提示 `TabularEvent`/`parse_tabular_event`/`TabularCommand`/`encode_tabular_command` 未定义。

- [ ] **Step 3: 实现类型(插在 `JsonEvent`/`parse_json_event` 与 `FlyfishEvent` 之间,约第 423 行)**

```rust
/// Tabular 预览 host(ag-grid)的 Rust -> JS 命令。与 `EditorCommand`/`JsonEvent`
/// 共用 envelope,只扩展自己的命令/事件名。数据来源恒为已在内存的
/// `crate::tabular::Sheet`(零 IO 切片),不是磁盘窗口化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TabularCommand {
    /// host `ready` 后立即发,JS 据此画整条 sheet tab 栏(未加载的 sheet 也要
    /// 能点,保留原生网格的既有行为)。
    Init {
        sheet_names: Vec<String>,
        active_sheet: usize,
        read_only: bool,
    },
    /// 某 sheet 可显示时发(首次加载完成 / 懒加载完成),JS 用它配置 ag-grid
    /// 列定义(行号 pinned 列 + Excel 字母列头)与截断提示条。
    SetSchema {
        sheet_index: usize,
        col_count: usize,
        total_rows: usize,
        truncated: bool,
        col_widths: Vec<f32>,
    },
    /// 懒加载中,JS 把对应 tab 标 loading 态。
    SetSheetLoading { sheet_index: usize, loading: bool },
    /// Rust 主动切 sheet(不是用户点 tab 触发——例如会话恢复到非首个 sheet,
    /// 或懒加载完成后把视图切到刚加载好的目标 sheet)。JS 只更新高亮/显示,
    /// 不回发 `sheet_selected`(避免来回死循环)。
    SelectSheet { sheet_index: usize },
    /// 响应 `TabularEvent::WindowRequest`:对已在内存的 `Sheet.rows` 做零 IO
    /// 切片。`revision` 供 JS 端按 envelope 顶层 revision 丢弃过期响应
    /// (与文本窗口化的 `SetWindow` 同一丢弃手法)。
    SetWindow {
        sheet_index: usize,
        start_row: u32,
        rows: Vec<Vec<String>>,
        revision: u64,
    },
    /// agent reveal:切 sheet(未加载先走 `SheetLoadRequest`)+ 滚动到位 +
    /// 高亮范围(0-based,含端点)。
    RevealRange {
        sheet_index: usize,
        r1: u32,
        c1: u32,
        r2: u32,
        c2: u32,
    },
    /// 首个窗口应用后发,回填会话持久化的 `PersistedTabular`(active sheet +
    /// 逻辑滚动锚点)。
    RestoreViewState {
        sheet_index: usize,
        start_row: u32,
        start_col: u32,
    },
}

/// Tabular host 的 JS -> Rust 事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TabularEvent {
    /// host JS 初始化完成(空网格)。不代表任何 sheet 已可显示。
    Ready,
    /// 用户点了 sheet tab。
    SheetSelected { index: usize },
    /// ag-grid Infinite Row Model 的 `getRows` 回调发出,请求 `[start_row,
    /// end_row)` 区间的行。
    WindowRequest {
        sheet_index: usize,
        start_row: u32,
        end_row: u32,
    },
    /// 首个 `SetWindow` 真正挂上(`api.setRowCount` + 首块数据到位),回报
    /// 窗口首行全局行号。Rust 以此作为该 sheet 加载的 Ready 边界。
    WindowApplied { start_row: u32 },
    Failed { message: String, recoverable: bool },
}

/// 解析一条 tabular host 事件。与 [`parse_event`] 同规则(超大/非法/未知不 panic)。
pub fn parse_tabular_event(raw: &str) -> Result<WebviewEnvelope<TabularEvent>, ProtocolError> {
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
    let payload: TabularEvent = serde_json::from_value(env.payload)
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

/// 编码一条 Rust -> tabular host 的命令为 envelope JSON,供 `dispatch_script`
/// 注入。与 [`encode_command`] 同结构,只是 payload 类型不同(`encode_command`
/// 硬编码 `EditorCommand`,不是泛型,故需要这个平行函数)。
pub fn encode_tabular_command(
    project_id: i64,
    panel: PanelKind,
    tab_id: usize,
    document_id: &str,
    revision: u64,
    request_id: Option<String>,
    command: TabularCommand,
) -> String {
    let env = WebviewEnvelope {
        protocol_version: PROTOCOL_VERSION,
        project_id,
        panel: match panel {
            PanelKind::Project => "project".to_string(),
            PanelKind::GitLog => "gitlog".to_string(),
            _ => "files".to_string(),
        },
        tab_id,
        document_id: document_id.to_string(),
        revision,
        request_id,
        payload: command,
    };
    serde_json::to_string(&env).unwrap_or_else(|_| "{}".to_string())
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app --lib preview::webview_protocol
```

Expected: 全部通过,含新增的 5 个 tabular 测试。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview/webview_protocol.rs
git commit -m "feat(tabular): add TabularCommand/TabularEvent protocol types"
```

---

### Task 5: Host binding(`dozer://tabular/` URL + enable 开关)

**Files:**
- Modify: `crates/dozer-app/src/preview/code_host.rs`

**Interfaces:**
- Consumes:Task 4 的 `TabularCommand`/`TabularEvent`(仅类型引用,本任务不直接用到,但下游任务需要)。
- Produces:`pub const TABULAR_URL_PREFIX: &str`、`EditorHostBinding::tabular_url(&self, theme: &str) -> String`、`pub fn is_tabular_url(url: &str) -> bool`、`pub fn tabular_grid_host_enabled() -> bool`;`is_host_url()` 覆盖 tabular。

- [ ] **Step 1: 在 `tests` 模块加入新测试(`json_editor_url_is_distinct` 之后)**

```rust
    #[test]
    fn tabular_url_is_distinct_and_always_read_only() {
        let b = binding(PanelKind::Files, 3);
        let url = b.tabular_url("dark");
        assert!(is_tabular_url(&url));
        assert!(!is_editor_url(&url));
        assert!(!is_json_editor_url(&url));
        assert!(url.contains("dozer://tabular/index.html"));
        assert!(url.contains("doc=p7-t3"));
        assert!(url.contains("theme=dark"));
        // 不带 CodeMirror 专用的字号/行高参数。
        assert!(!url.contains("fs="));
        assert!(!url.contains("lh="));
    }

    #[test]
    fn is_host_url_covers_tabular() {
        let b = binding(PanelKind::Files, 3);
        assert!(is_host_url(&b.tabular_url("dark")));
    }

    #[test]
    fn tabular_grid_host_always_enabled() {
        assert!(tabular_grid_host_enabled());
    }
```

- [ ] **Step 2: 跑测试确认失败(缺方法/函数)**

```bash
cargo test -p dozer-app --lib preview::code_host -- --list 2>&1 | tail -20
```

- [ ] **Step 3: 实现**

在 `JSON_EDITOR_URL_PREFIX` 常量后加:

```rust
/// Tabular 预览 host(ag-grid)页面 URL 前缀。
pub const TABULAR_URL_PREFIX: &str = "dozer://tabular/";
```

在 `impl EditorHostBinding` 内、`json_url` 方法之后加:

```rust
    /// Tabular host(ag-grid)URL:正文完全由命令推送(`Init`/`SetSchema`/
    /// `SetWindow`),host 不 fetch 任何文件,故不带 `fs=`/`lh=`(那是
    /// CodeMirror 专用的等宽字号参数——tabular 走系统默认字体,见
    /// CLAUDE.md"非代码/终端场景禁用等宽字体"裁决)。恒只读,不带 `ro=`
    /// 参数(host 本身不支持编辑,无需首屏协商)。
    pub fn tabular_url(&self, theme: &str) -> String {
        format!(
            "{TABULAR_URL_PREFIX}index.html?p={}&theme={}&doc={}&proj={}&panel={}&tab={}",
            encode_path(&self.path),
            theme,
            super::encode_component(&self.document_id()),
            self.project_id,
            self.panel_token(),
            self.tab_id,
        )
    }
```

在 `is_json_editor_url` 之后加:

```rust
/// URL 是否是 Tabular(ag-grid)host。
pub fn is_tabular_url(url: &str) -> bool {
    url.starts_with(TABULAR_URL_PREFIX)
}
```

修改 `is_host_url`:

```rust
pub fn is_host_url(url: &str) -> bool {
    is_editor_url(url) || is_json_editor_url(url) || is_tabular_url(url)
}
```

在 `codemirror_enabled()` 之后加:

```rust
/// Tabular Grid 视图已转 webview host(ag-grid),不留对照期,恒常开
/// (与 `codemirror_enabled()`/`json_editor_enabled()` 同一"始终开"风格)。
pub fn tabular_grid_host_enabled() -> bool {
    true
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app --lib preview::code_host
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview/code_host.rs
git commit -m "feat(tabular): add dozer://tabular/ host binding URL"
```

---

### Task 6: `assets.rs` 静态资源注册 + CSP

**Files:**
- Modify: `crates/dozer-app/src/assets.rs`

**Interfaces:**
- Consumes:Task 5 的 `TABULAR_URL_PREFIX`(字符串前缀匹配,不直接导入符号也可,用字面量 `"dozer://tabular/"` 与现有 json-editor 分支写法保持一致)。
- Produces:`dozer://tabular/index.html` 与 `dozer://tabular/tabular-host.js`/`tabular-host.css` 可被 `handle_protocol` serve;后续 Task 7(`runtime.rs`)不直接依赖本任务的内部实现,只依赖"URL 能被正确 serve"这一行为。

**先读现状**:本文件里 json-editor 分支(约第 260-290 行)是唯一需要照抄的模板——执行者应先 `grep -n "json-editor" crates/dozer-app/src/assets.rs` 找到当前精确行号(本计划写作时是约 213/265/692 行,后续任务可能已推移行号),按同样结构插入 tabular 分支,不要凭空重新设计目录/CSP 结构。

- [ ] **Step 1: 加测试(照抄 `crates/dozer-app/src/assets.rs` 里 json-editor 那组 `#[cfg(test)]` 用例,函数名替换)**

在 assets.rs 的 `#[cfg(test)] mod tests` 里,json-editor 相关测试(约第 692-760 行那组)之后加:

```rust
    #[test]
    fn tabular_host_index_serves_with_strict_csp() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tabular-host"));
        let r = handle_protocol(root, &HashSet::new(), None, "dozer://tabular/index.html");
        let html = String::from_utf8_lossy(&r.body).to_string();
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("connect-src 'self'"));
    }

    #[test]
    fn tabular_host_script_is_servable() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tabular-host"));
        let r = handle_protocol(root, &HashSet::new(), None, "dozer://tabular/tabular-host.js");
        assert!(!r.body.is_empty());
        assert!(!String::from_utf8_lossy(&r.body).contains(env!("CARGO_MANIFEST_DIR")));
    }
```

(若现有 `handle_protocol` 签名与上面调用不一致——例如多一个参数或返回类型不同——以文件里 json-editor 测试的实际调用形式为准,照抄参数顺序,不要凭空猜测签名。)

- [ ] **Step 2: 跑测试确认失败(找不到 `assets/tabular-host` 分支,404 或 body 为空)**

```bash
cargo test -p dozer-app --lib assets::tests::tabular_host
```

- [ ] **Step 3: 实现**——在处理 `dozer://json-editor/` 前缀的分支旁边,加一段结构完全平行的 `dozer://tabular/` 分支(资源根 `assets/tabular-host`,CSP 与 json-editor 一致:`default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`)。注意:tabular host **不需要** `__file__` 白名单读取分支(它不 fetch 文件,所有正文都是命令推送),比 json-editor 分支更简单——不要照抄 `__file__` 那部分。

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app --lib assets::tests
```

Expected: 全部通过,含新增两个 tabular 测试,且原有 json-editor/editor 测试不受影响。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/assets.rs
git commit -m "feat(tabular): serve dozer://tabular/ host assets with strict CSP"
```

---

### Task 7: `runtime.rs` IPC 分发接线

**Files:**
- Modify: `crates/dozer-app/src/runtime.rs`
- Modify: `crates/dozer-app/src/app/message.rs`

**Interfaces:**
- Consumes:Task 4 的 `parse_tabular_event`;Task 5 的 `is_tabular_url`。
- Produces:`Message::TabularHostEvent(EditorHostBinding, WebviewEnvelope<TabularEvent>)` 变体;当 webview URL 匹配 `dozer://tabular/` 时,IPC handler 把回传 envelope 解析并经 `ipc_proxy.send_event` 送出这条消息。

- [ ] **Step 1: `message.rs` 新增变体**——在 `JsonEditorEvent` 变体之后加:

```rust
    /// Tabular host(ag-grid)发回的事件。
    TabularHostEvent(
        crate::preview::EditorHostBinding,
        crate::preview::WebviewEnvelope<crate::preview::TabularEvent>,
    ),
```

- [ ] **Step 2: `runtime.rs` 找到 `let is_json_host = crate::preview::is_json_editor_url(&spec.url);`(约第 369 行),旁边加一行**

```rust
                let is_json_host = crate::preview::is_json_editor_url(&spec.url);
                let is_tabular_host = crate::preview::is_tabular_url(&spec.url);
```

- [ ] **Step 3: 在 IPC handler 内,`editor_binding.as_ref()` 分支里,`if is_json_host { ... } else { ... }` 这段(约第 555-600 行)改成三路分派**

原代码结构:

```rust
                                } else if let Some(binding) = editor_binding.as_ref() {
                                    let expected = crate::preview::HostBinding::new(
                                        binding.project_id,
                                        binding.panel,
                                        binding.tab_id,
                                        binding.document_id(),
                                    );
                                    if is_json_host {
                                        match crate::preview::parse_json_event(body) {
                                            // ... Message::JsonEditorEvent
                                        }
                                    } else {
                                        match crate::preview::parse_event(body) {
                                            // ... Message::EditorWebviewEvent / GitLogDiffWebviewEvent
                                        }
                                    }
                                }
```

改为在 `if is_json_host { ... }` 与 `else { ... }` 之间插入一支 `else if is_tabular_host`:

```rust
                                    if is_json_host {
                                        match crate::preview::parse_json_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    tracing::warn!(%error, "拒绝无效 json-editor IPC");
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::JsonEditorEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                tracing::warn!(%error, "无法解析 json-editor IPC");
                                            }
                                        }
                                    } else if is_tabular_host {
                                        match crate::preview::parse_tabular_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    tracing::warn!(%error, "拒绝无效 tabular IPC");
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::TabularHostEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                tracing::warn!(%error, "无法解析 tabular IPC");
                                            }
                                        }
                                    } else {
                                        match crate::preview::parse_event(body) {
                                            // 保持原样不动
                                        }
                                    }
```

- [ ] **Step 4: 编译检查(尚未有 `TabularHostEvent` 的 `update()` 处理分支,先确认能编译——`match message` 是穷尽匹配,会在 Task 11 前报错缺分支;本步骤先跑 `cargo check` 确认报错位置正是 `update.rs` 的 match,而不是别处笔误)**

```bash
cargo check -p dozer-app 2>&1 | grep -A3 "non-exhaustive\|TabularHostEvent"
```

Expected: 报错指向 `app/update.rs` 的 `match message` 缺 `TabularHostEvent` 分支(下一任务补)。这是预期的中间态,不修复,直接进入 Task 8。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/runtime.rs crates/dozer-app/src/app/message.rs
git commit -m "feat(tabular): dispatch tabular host IPC events to Message::TabularHostEvent"
```

---

### Task 8: `state.rs`/`backend.rs` 谓词与 `PreviewTab` 新字段

**Files:**
- Modify: `crates/dozer-app/src/preview/state.rs`

**Interfaces:**
- Consumes:Task 5 的 `tabular_grid_host_enabled()`;已有的 `PreviewBackend::Tabular`/`TabularMode::Grid`(`backend.rs`,无需改动该文件——`TabularMode::Grid` 早已存在,只是过去指向 iced canvas)。
- Produces:`PreviewTab::uses_tabular_grid_host(&self) -> bool`;`PreviewTab.tabular_host_ready: bool` 字段(默认 `false`);更新后的 `hosts_any_webview()`。这三者是 Task 9/10/11 判断"这个 tab 该不该有 tabular webview / 该不该等 host 信号"的唯一依据。

- [ ] **Step 1: 加单测(找 `uses_json_editor` 附近的既有测试模块,或在 state.rs 的 `#[cfg(test)]` 里新增)**

```rust
    #[test]
    fn uses_tabular_grid_host_only_for_grid_mode() {
        let mut tab = make_tabular_tab(TabularMode::Grid); // 若无现成 helper,
        // 参考本文件其它测试里构造 PreviewTab 的写法(通常经 push_shell_tab
        // 或直接手写字面量),按 backend = Some(PreviewBackend::Tabular(
        // TabularBackend { format: ..., mode: TabularMode::Grid })) 构造。
        assert!(tab.uses_tabular_grid_host());
        tab.backend = Some(PreviewBackend::Tabular(TabularBackend {
            format: TabularFormat::Csv,
            mode: TabularMode::Text,
        }));
        assert!(!tab.uses_tabular_grid_host());
    }

    #[test]
    fn hosts_any_webview_covers_tabular_grid() {
        let tab = make_tabular_tab(TabularMode::Grid);
        assert!(tab.hosts_any_webview());
    }
```

（执行者需先 `grep -n "fn make_.*_tab\|PreviewTab {" crates/dozer-app/src/preview/state.rs crates/dozer-app/src/preview/view.rs` 找到本文件测试里现成的 tab 构造 helper 或字面量写法,复用它构造一个 `TabKind::File` + `PreviewBackend::Tabular` 的 tab,不要重新发明构造方式。若没有现成 helper,直接在测试里用 `PreviewTab { .. }` 结构体更新语法配合一个 `Default`/最小字面量构造,参照本文件同一测试模块里其它测试的写法。）

- [ ] **Step 2: 跑测试确认失败(缺方法)**

```bash
cargo test -p dozer-app --lib preview::state -- --list 2>&1 | tail -10
```

- [ ] **Step 3: 实现**

在 `PreviewTab` 结构体定义里,`task_cancel` 字段之前加:

```rust
    /// Tabular webview host(ag-grid)是否已报过 `ready`。与"数据是否已加载
    /// 完成"(`runtime` 是否 `TabularState::Ready`)是两个独立的异步来源,
    /// 两者都为真时才推初始 `Init`+`SetSchema`+`SetWindow`(见
    /// `PreviewPane::finish_tabular_load`/`Message::TabularHostEvent` 处理)。
    /// 新建 tab / reload 时重置为 `false`。
    pub tabular_host_ready: bool,
```

在所有构造 `PreviewTab { ... }` 字面量的地方(`view.rs` 里至少 3 处,见 Task 2 grep 到的 `pending_tabular: None,` 三处)对应加 `tabular_host_ready: false,`（这些字面量在 `view.rs`,留到 Task 9 的 Step 1 一并处理,因为那三处本来就要在 Task 9 编辑;若 Rust 编译器在本任务就因缺字段报错,说明字面量用了 `..Default::default()` 之外的显式全字段写法,此时必须在本任务立即补全,不能拖到 Task 9——以 `cargo check` 报错为准）。

在 `uses_json_editor()` 方法之后加:

```rust
    /// Tabular Grid 视图是否走 ag-grid webview host(与 `TabularMode::Text`
    /// 的 CodeMirror 原文模式互斥,同一 tab 同一时刻只有一个为真)。
    pub fn uses_tabular_grid_host(&self) -> bool {
        tabular_grid_host_enabled()
            && matches!(self.backend, Some(PreviewBackend::Tabular(t)) if t.mode == TabularMode::Grid)
    }
```

修改 `hosts_any_webview()`:

```rust
    /// 该 tab 是否由**任一** WebView host 承载(Flyfish 渲染 / CodeMirror
    /// editor / vanilla-jsoneditor Tree / Tabular ag-grid)。用于"加载是否
    /// 需要等 host 信号"的判定:凡有 host 就不该在画像后立即 finish,须等
    /// host 的 `ready`/`document_loaded`/`window_applied`。纯 iced fallback
    /// (Unsupported/External)返回 false。
    pub fn hosts_any_webview(&self) -> bool {
        self.hosts_webview()
            || self.uses_editor_host()
            || self.uses_json_editor()
            || self.uses_tabular_grid_host()
    }
```

（把文档注释里"纯 iced fallback(Unsupported/External/表格)"的"表格"二字删掉,因为表格从本任务起不再是纯 iced fallback。）

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app --lib preview::state
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview/state.rs
git commit -m "feat(tabular): add uses_tabular_grid_host predicate and host-ready flag"
```

---

### Task 9: `view.rs` — webview 期望清单 + 命令队列 + 加载生命周期改造

这是本次迁移里最核心的一个任务:把"数据加载完成就立即 Ready"改成"数据加载完成 + host 就绪 + 首窗应用完成,三者都满足才 Ready"。

**Files:**
- Modify: `crates/dozer-app/src/preview/view.rs`

**Interfaces:**
- Consumes:`EditorHostBinding::tabular_url`(Task 5)、`TabularCommand`(Task 4)、`uses_tabular_grid_host`/`tabular_host_ready`(Task 8)。
- Produces:
  - `PreviewPane::desired_tabular_webviews(&self, project_id: i64, panel: PanelKind) -> Vec<WebviewSpec>`
  - `PreviewPane::queue_tabular_command(&mut self, tab_id: usize, command: TabularCommand)`
  - `PreviewPane::take_pending_tabular_commands_for(&mut self, available_webview_ids: &HashSet<usize>, project_id: i64, panel: PanelKind) -> Vec<(usize, TabularCommand)>`
  - 改造后的 `PreviewPane::finish_tabular_load(...)`(不再立即 `Ready`,改为记录数据、尝试汇合)
  - `PreviewPane::try_push_initial_tabular_state(&mut self, tab_id: usize)`(两路汇合都满足时组好 `Init`+`SetSchema`+`SetWindow` 三条命令入队;哪个条件还没满足就是 no-op)
  - `PreviewPane::push_sheet_schema_and_window(&mut self, tab_id: usize, sheet_index: usize)`(懒加载完某个 sheet 后补推 `SetSchema`+`SetWindow`,供 Task 11 的 `TabularSheetLoaded` 处理调用)
  - `apply_preview_command` 的 `A::RevealCell` 分支补发 `TabularCommand::RevealRange`

- [ ] **Step 1: 三处 `pending_tabular: None,` 字面量旁加 `tabular_host_ready: false,`**

用编辑器在 `crates/dozer-app/src/preview/view.rs` 搜索 `pending_tabular: None,`(应有 3 处,约第 41/565/653 行),每处后面加一行 `tabular_host_ready: false,`。

- [ ] **Step 2: 加 `pending_editor_commands` 的平行字段**

在 `struct PreviewPane`(找 `pending_editor_commands: Vec<(usize, EditorCommand)>,` 所在结构体定义,约第 713 行)旁加:

```rust
    pub(crate) pending_tabular_commands: Vec<(usize, TabularCommand)>,
```

并在该结构体所有构造点(`pending_editor_commands: Vec::new(),` 所在处,约第 750 行)加:

```rust
            pending_tabular_commands: Vec::new(),
```

- [ ] **Step 3: 加命令队列方法(紧跟 `take_pending_editor_commands_for` 之后,约第 2125 行之后)**

```rust
    /// 排队一个待下发给 Tabular webview host 的命令(`tab_id`, 命令)。
    pub fn queue_tabular_command(&mut self, tab_id: usize, command: TabularCommand) {
        self.pending_tabular_commands.push((tab_id, command));
    }

    /// 取走(消费式)待下发的 tabular 命令队列。测试专用(同
    /// `take_pending_editor_commands` 的既有先例),生产代码走
    /// `take_pending_tabular_commands_for`(按可用 webview id 过滤)。
    #[cfg(test)]
    pub fn take_pending_tabular_commands(&mut self) -> Vec<(usize, TabularCommand)> {
        std::mem::take(&mut self.pending_tabular_commands)
    }

    /// 只取当前已有 WebView 句柄对应的命令;其余保留待下一帧重试(同
    /// `take_pending_editor_commands_for`)。
    pub fn take_pending_tabular_commands_for(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<(usize, TabularCommand)> {
        let pending = std::mem::take(&mut self.pending_tabular_commands);
        let mut ready = Vec::new();
        for (tab_id, command) in pending {
            let webview_id = crate::preview::EditorHostBinding::new(
                project_id,
                panel,
                tab_id,
                std::path::PathBuf::new(),
            )
            .webview_id();
            if available_webview_ids.contains(&webview_id) {
                ready.push((tab_id, command));
            } else {
                self.pending_tabular_commands.push((tab_id, command));
            }
        }
        ready
    }
```

- [ ] **Step 4: 加 `desired_tabular_webviews`(紧跟 `desired_json_webviews` 之后,约第 1557 行之后)**

```rust
    /// Tabular Grid 视图(ag-grid webview host)的期望清单。
    pub fn desired_tabular_webviews(
        &self,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<WebviewSpec> {
        if !tabular_grid_host_enabled() {
            return Vec::new();
        }
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                // 数据后台解析中(`TabularState::Loading`)也预创建 hidden
                // host,让它先完成 boot、报 `ready`,不必等数据到位——两路
                // 汇合逻辑在 `finish_tabular_load`/`try_push_initial_tabular_state`。
                let loading = matches!(tab.backend_state, BackendState::Loading);
                if !tab.uses_tabular_grid_host() || (!tab.backend_state.is_ready() && !loading) {
                    return None;
                }
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                let binding = EditorHostBinding::new(project_id, panel, tab.id, path.clone());
                Some(WebviewSpec {
                    id: tab.id,
                    url: binding.tabular_url(scheme_query_value()),
                    visible: tab.backend_state.is_ready() && idx == self.active,
                    editor_binding: Some(binding),
                    loading_generation: loading.then_some(tab.load_state.generation),
                    park_offscreen: false,
                })
            })
            .collect()
    }
```

- [ ] **Step 5: 改造 `finish_tabular_load`**

替换现有实现(约第 2026-2063 行)为:

```rust
    /// `generation` 是启动后台解析时捕获的世代;与当前 `load_state` 不匹配
    /// (tab 已关闭重开 / 重试)时丢弃旧结果,不回填(T7 取消不回填旧 sheet)。
    ///
    /// 迁移到 webview host 后不再在数据到位那一刻立即 `Ready`——还要等
    /// host `ready` + 首窗 `window_applied`(两路异步汇合,见
    /// `try_push_initial_tabular_state`)。若 host 已经先 `ready` 过,本函数
    /// 直接把初始状态推过去;若 host 还没 `ready`,由 `Message::TabularHostEvent`
    /// 的 `Ready` 分支在稍后补推。
    pub fn finish_tabular_load(
        &mut self,
        tab_id: usize,
        generation: u64,
        result: Result<crate::tabular::TabularView, String>,
    ) -> Option<usize> {
        // 借用 `self.tabs` 的部分收在这个块里,块结束后借用释放,后面才能
        // 再调用 `self.try_push_initial_tabular_state`/`self.queue_tabular_command`
        // (它们需要 `&mut self`)。这是本文件既有代码(如 `EditorWebviewEvent`
        // 的处理)反复用到的手法:先在借用块里把要用的值收进外层局部变量,
        // 块结束后再用它们调用兄弟方法。
        let accepted = {
            let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                return None;
            };
            if !tab.load_state.accepts(generation) {
                return None;
            }
            match result {
                Ok(view) => {
                    tab.runtime = PreviewRuntime::Tabular(TabularState::Ready(view));
                    true
                }
                Err(message) => {
                    tab.runtime = PreviewRuntime::None;
                    let _ = tab
                        .backend_state
                        .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                    tab.load_state.finish();
                    false
                }
            }
        };
        if !accepted {
            return None;
        }
        // 应用持久化的 sheet / 滚动锚点;若目标 sheet 不是当前已加载的,
        // 返回它让调用方触发一次懒加载。没有持久化状态(常见:新打开的
        // tab,不是从会话恢复来的)时 `pending_sheet` 是 `None`,但仍要往下
        // 走两路汇合尝试——`?` 只在这个 `and_then` 闭包内部短路,不会跳过
        // 后面的 `try_push_initial_tabular_state`。
        let pending_sheet = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| -> Option<usize> {
                let (sheet, row, col) = tab.pending_tabular.take()?;
                let active_sheet = tab.tabular_view_mut().map(|view| {
                    view.scroll_row = row;
                    view.scroll_col = col;
                    view.active_sheet
                })?;
                (active_sheet != sheet).then_some(sheet)
            });
        // 两路异步汇合:数据已加载(刚发生在上面)+ host 是否已先 `ready`
        // 过(`tabular_host_ready`)。若 host 已就绪,这里立即推初始状态;
        // 否则等 `Message::TabularHostEvent::Ready` 到达时再推
        // (见 `try_push_initial_tabular_state` 文档)。
        self.try_push_initial_tabular_state(tab_id);
        pending_sheet
    }

    /// 两路异步汇合:数据已加载(`runtime` 是 `TabularState::Ready`)且 host
    /// 已 `ready`(`tabular_host_ready`)都为真时,组好 `Init`+`SetSchema`+
    /// `SetWindow`(首 200 行,与 JS 侧 `cacheBlockSize` 对齐)三条命令入队。
    /// 两个条件哪个先满足都可能发生(小文件解析可能比 webview boot 快,
    /// 反之亦然),调用方是 `finish_tabular_load`(数据到位那一刻)与
    /// `Message::TabularHostEvent::Ready` 处理(host 到位那一刻)各调一次,
    /// 只有真正"两个都满足"的那一次会实际入队命令。用 `pub` 而非
    /// `pub(crate)`——同文件里 `queue_editor_command`/`take_pending_editor_
    /// commands_for` 等跨文件调用的兄弟方法都是 `pub fn`,保持一致。
    pub fn try_push_initial_tabular_state(&mut self, tab_id: usize) {
        let Some((sheet_names, active_sheet, ready)) =
            self.tabs.iter().find(|t| t.id == tab_id).and_then(|tab| {
                if !tab.tabular_host_ready {
                    return None;
                }
                let view = tab.tabular_view()?;
                Some((
                    view.sheet_names.clone(),
                    view.active_sheet,
                    view.active_sheet().is_some(),
                ))
            })
        else {
            return;
        };
        if !ready {
            // sheet 0 理论上在打开文件时已同步预加载(见 `tabular::load` 文档
            // "多 sheet 的 xlsx 只在打开时预加载第一个 sheet"),这个分支正常
            // 不会命中,防御性保留(数据尚未就绪时不发半成品 Init)。
            return;
        }
        self.queue_tabular_command(
            tab_id,
            TabularCommand::Init {
                sheet_names,
                active_sheet,
                read_only: true,
            },
        );
        self.push_sheet_schema_and_window(tab_id, active_sheet);
    }

    /// 把"某个 sheet 当前已加载"的 `SetSchema`+`SetWindow`(首 200 行)命令
    /// 入队。供两处共用:`try_push_initial_tabular_state`(首次打开,额外带
    /// `Init`)与 `Message::TabularSheetLoaded` 处理(懒加载完某个 sheet 后,
    /// 见 Task 11)。`sheet_index` 若已不是当前活动 sheet(用户在懒加载完成
    /// 前又切到别处)则 no-op——不推一份不会被显示的窗口。
    pub fn push_sheet_schema_and_window(&mut self, tab_id: usize, sheet_index: usize) {
        const INITIAL_WINDOW_ROWS: usize = 200;
        let Some((col_count, total_rows, truncated, col_widths, window)) = self
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|tab| tab.tabular_view())
            .and_then(|view| {
                if view.active_sheet != sheet_index {
                    return None;
                }
                let sheet = view.active_sheet()?;
                Some((
                    sheet.col_count,
                    sheet.total_rows,
                    sheet.truncated,
                    sheet.col_widths.clone(),
                    sheet
                        .rows
                        .iter()
                        .take(INITIAL_WINDOW_ROWS)
                        .cloned()
                        .collect::<Vec<Vec<String>>>(),
                ))
            })
        else {
            return;
        };
        self.queue_tabular_command(
            tab_id,
            TabularCommand::SetSchema {
                sheet_index,
                col_count,
                total_rows,
                truncated,
                col_widths,
            },
        );
        self.queue_tabular_command(
            tab_id,
            TabularCommand::SetWindow {
                sheet_index,
                start_row: 0,
                rows: window,
                revision: 0,
            },
        );
    }
```

**关于"只应推送一次"**:上面这版每次数据/host 就绪都会被调用,但只要 `tabular_host_ready` 尚未置真或数据尚未 `Ready`,`and_then` 链就在对应环节返回 `None`,天然不会重复入队——真正会入队的时刻只有"两个条件都满足"的那一次调用,不需要额外的"已推送过"标记。若某个 tab 因为懒加载完*另一个* sheet(非首个)而再次触发 `TabularSheetLoaded`,那条路径走的是 Task 11 的 `TabularSheetLoaded` 处理(未改动,不经过本方法),不会重复推 `Init`。

- [ ] **Step 6: `apply_preview_command` 的 `A::RevealCell` 分支补发 webview 命令**

在 `O::Accepted { request_id: rid }` 之前(约第 1966 行,原 `if request.is_some() { return O::LoadDenied ... }` 之后)加:

```rust
                self.queue_tabular_command(
                    tab_id,
                    TabularCommand::RevealRange {
                        sheet_index: *sheet,
                        r1: *row,
                        c1: *col,
                        r2: *row,
                        c2: *col,
                    },
                );
```

- [ ] **Step 7: 编译检查**

```bash
cargo check -p dozer-app 2>&1 | tail -40
```

Expected: 本任务范围内的新代码应能编译(仍会有 Task 10/11 未完成导致的 `update.rs`/`app.rs` 相关报错,属预期中间态)。

- [ ] **Step 8: 修复被本任务改动打破的既有测试,并加两条新测试覆盖"两路汇合"**

`tabular_state_transitions_from_loading_to_ready_via_load_result`(约第 3009-3031 行)断言"`finish_tabular_load` 之后立即 `BackendState::Ready`"——这条断言现在不成立了(要等 host `ready` + `window_applied`)。把断言改成反映新行为:

```rust
    #[test]
    fn tabular_state_transitions_from_loading_to_ready_via_load_result() {
        let p = std::env::temp_dir().join(format!("tabular_ready_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        assert!(pane.tabular_mut(id).is_none(), "加载完成前 apply 应 no-op");
        assert!(matches!(
            pane.tabs()[pane.active_idx()].backend_state,
            BackendState::Loading
        ));
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        let generation = pane.load_generation(id);
        pane.finish_tabular_load(id, generation, Ok(loaded));
        assert!(
            pane.tabular_mut(id).is_some(),
            "数据到位后 tabular_mut 应能拿到可变引用(即便还没 Ready)"
        );
        // 数据到位了,但 host 还没报 `ready`——两路汇合尚未完成,仍是
        // Loading,不应有任何命令被推给还不存在的 webview。
        assert!(matches!(
            pane.tabs()[pane.active_idx()].backend_state,
            BackendState::Loading
        ));
        assert!(pane.take_pending_tabular_commands().is_empty());
        std::fs::remove_file(p).ok();
    }
```

再加两条新测试,覆盖"数据先到 / host 先到"两种顺序都恰好推送一次 `Init`+`SetSchema`+`SetWindow`(Review Focus 第一条):

```rust
    /// 数据先于 host 就绪(小文件解析比 webview boot 快的常见情形)。
    #[test]
    fn initial_tabular_state_pushes_once_when_data_ready_first() {
        let p = std::env::temp_dir()
            .join(format!("tabular_rendezvous_a_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        let generation = pane.load_generation(id);
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        pane.finish_tabular_load(id, generation, Ok(loaded));
        assert!(
            pane.take_pending_tabular_commands().is_empty(),
            "host 还没 ready,不该推任何命令"
        );
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .tabular_host_ready = true;
        pane.try_push_initial_tabular_state(id);
        let cmds = pane.take_pending_tabular_commands();
        assert_eq!(cmds.len(), 3, "应恰好推 Init+SetSchema+SetWindow 三条");
        assert!(matches!(cmds[0].1, TabularCommand::Init { .. }));
        assert!(matches!(cmds[1].1, TabularCommand::SetSchema { .. }));
        assert!(matches!(cmds[2].1, TabularCommand::SetWindow { .. }));
        std::fs::remove_file(p).ok();
    }

    /// host 先于数据就绪(webview boot 比后台解析快的情形)。
    #[test]
    fn initial_tabular_state_pushes_once_when_host_ready_first() {
        let p = std::env::temp_dir()
            .join(format!("tabular_rendezvous_b_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .tabular_host_ready = true;
        pane.try_push_initial_tabular_state(id);
        assert!(
            pane.take_pending_tabular_commands().is_empty(),
            "数据还没到位,不该推任何命令"
        );
        let generation = pane.load_generation(id);
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        pane.finish_tabular_load(id, generation, Ok(loaded));
        let cmds = pane.take_pending_tabular_commands();
        assert_eq!(cmds.len(), 3, "应恰好推 Init+SetSchema+SetWindow 三条");
        std::fs::remove_file(p).ok();
    }
```

跑测试:

```bash
cargo test -p dozer-app --lib preview::view -- tabular
```

Expected: 全部通过,含改过的既有测试与两条新增测试。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/preview/view.rs
git commit -m "feat(tabular): defer Ready until webview host applies first window"
```

---

### Task 10: `app.rs` + `window_events.rs` — 每帧派发接线

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes:Task 9 的 `desired_tabular_webviews`/`take_pending_tabular_commands_for`;Task 4 的 `encode_tabular_command`。
- Produces:`App::take_preview_tabular_scripts(&mut self, kind: PanelKind, available_webview_ids: &HashSet<usize>) -> Vec<(usize, String)>`;`desired_webviews()` 聚合结果里包含 tabular webview;`apply_pending_editor_commands()` 每帧把 tabular 命令注入对应 webview。

- [ ] **Step 1: `app.rs` 新增方法(紧跟 `take_preview_editor_scripts` 之后,约第 1037 行之后)**

```rust
    /// 构建本帧待注入 Tabular webview host 的脚本清单,同
    /// `take_preview_editor_scripts` 的节奏。
    pub fn take_preview_tabular_scripts(
        &mut self,
        kind: PanelKind,
        available_webview_ids: &std::collections::HashSet<usize>,
    ) -> Vec<(usize, String)> {
        let Some(ws) = self.active_workspace_mut() else {
            return Vec::new();
        };
        let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
            return Vec::new();
        };
        let pane = match kind {
            PanelKind::Project => &mut ws.project_preview,
            _ => &mut ws.preview,
        };
        let pending =
            pane.take_pending_tabular_commands_for(available_webview_ids, project_id, kind);
        let mut out = Vec::new();
        for (tab_id, command) in pending {
            let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
                continue;
            };
            if !tab.uses_tabular_grid_host() {
                continue;
            }
            let crate::preview::TabKind::File(path) = &tab.kind else {
                continue;
            };
            let binding =
                crate::preview::EditorHostBinding::new(project_id, kind, tab_id, path.clone());
            let envelope = crate::preview::encode_tabular_command(
                project_id,
                kind,
                tab_id,
                &binding.document_id(),
                tab.web_revision,
                None,
                command,
            );
            out.push((
                binding.webview_id(),
                crate::preview::dispatch_script(&envelope),
            ));
        }
        out
    }
```

- [ ] **Step 2: `app.rs` 里 `desired_webviews()` 聚合处(约第 3305-3316 行,`json` 那两个 `match kind { ... }` 块之后)加一段平行的 tabular 聚合**

```rust
                // Tabular Grid 视图的 ag-grid webview host。
                match kind {
                    PanelKind::Files => specs.extend(
                        ws.preview
                            .desired_tabular_webviews(project.id, PanelKind::Files),
                    ),
                    PanelKind::Project => specs.extend(
                        ws.project_preview
                            .desired_tabular_webviews(project.id, PanelKind::Project),
                    ),
                    _ => {}
                }
```

- [ ] **Step 3: `window_events.rs` 的 `apply_pending_editor_commands`(约第 2412-2440 行)加一段派发循环**,紧跟 editor 那个 `for kind in [PanelKind::Files, PanelKind::Project] { ... }` 循环之后:

```rust
        for kind in [PanelKind::Files, PanelKind::Project] {
            for (webview_id, js) in app.take_preview_tabular_scripts(kind, &available_webview_ids) {
                if let Some((view, _)) = webviews.get(&webview_id) {
                    let _ = view.evaluate_script(&js);
                }
            }
        }
```

- [ ] **Step 4: 编译检查**

```bash
cargo check -p dozer-app 2>&1 | tail -40
```

Expected: 仍会有 Task 11(`update.rs` 缺 `TabularHostEvent` 分支)导致的报错,属预期。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app/app.rs crates/dozer-app/src/platform/window_events.rs
git commit -m "feat(tabular): dispatch tabular webview specs and pending commands each frame"
```

---

### Task 11: `update.rs` — `Message::TabularHostEvent` 处理

**Files:**
- Modify: `crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Consumes:Task 9 的 `try_push_initial_tabular_state`/`push_sheet_schema_and_window`/`queue_tabular_command`;Task 8 的 `tabular_host_ready`;Task 4 的 `TabularCommand::SelectSheet`/`SetSheetLoading`;`crate::tabular::TabularView`/`Sheet`(已存在,不改)。
- Produces:`Message::TabularHostEvent` 的完整状态机处理(数据 ⇄ host 两路汇合、滚动窗口请求零 IO 切片);扩展后的 `Message::TabularSheetLoaded` 处理(懒加载完成后补推 schema/window 并切 JS 活动 sheet)。

- [ ] **Step 1: 在 `Message::JsonEditorEvent` 处理块之后(约第 650 行之后)加新分支**

借用规则同 Task 9 的 `finish_tabular_load`:所有读 `tab` 字段的逻辑收在一个借用块里,把要执行的动作(是否推初始状态 / 推哪条命令 / 切哪个 sheet)收集进外层局部变量,借用块结束后再调用需要 `&mut pane`/`&mut ws` 的兄弟方法。

```rust
            Message::TabularHostEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, io| {
                    let panel = binding.panel;
                    let mut push_initial = false;
                    let mut restore_view: Option<(usize, u32, u32)> = None;
                    let mut window_push: Option<(usize, u32, Vec<Vec<String>>, u64)> = None;
                    let mut sheet_to_select: Option<usize> = None;
                    {
                        let pane = if panel == PanelKind::Project {
                            &mut ws.project_preview
                        } else {
                            &mut ws.preview
                        };
                        let Some(tab) =
                            pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id)
                        else {
                            return;
                        };
                        use crate::preview::TabularEvent;
                        match event.payload {
                            TabularEvent::Ready => {
                                tab.web_error = None;
                                tab.tabular_host_ready = true;
                                push_initial = true;
                            }
                            TabularEvent::WindowApplied { start_row: _ } => {
                                // 首窗真正挂上才 finish,避免"空网格+行号1"
                                // 的中间态露出。非在途(迟到 ACK)不改终态。
                                if tab.load_state.is_active() {
                                    let _ = tab.backend_state.try_transition(
                                        crate::preview::BackendState::Ready,
                                    );
                                    tab.load_state.finish();
                                }
                                if let Some(view) = tab.tabular_view() {
                                    let (sheet_index, start_row, start_col) = (
                                        view.active_sheet,
                                        view.scroll_row as u32,
                                        view.scroll_col as u32,
                                    );
                                    if start_row != 0 || start_col != 0 {
                                        restore_view = Some((sheet_index, start_row, start_col));
                                    }
                                }
                            }
                            TabularEvent::WindowRequest {
                                sheet_index,
                                start_row,
                                end_row,
                            } => {
                                // 零 IO:对已在内存的 Sheet.rows 切片。
                                // sheet_index 与当前活动 sheet 不一致(用户
                                // 已经切走)的迟到请求直接丢弃,不回窗口。
                                let revision = tab.web_revision;
                                if let Some(view) = tab.tabular_view()
                                    && view.active_sheet == sheet_index
                                    && let Some(sheet) = view.active_sheet()
                                {
                                    let start = (start_row as usize).min(sheet.rows.len());
                                    let end = (end_row as usize).min(sheet.rows.len());
                                    let rows = sheet.rows[start..end].to_vec();
                                    window_push = Some((sheet_index, start as u32, rows, revision));
                                }
                            }
                            TabularEvent::SheetSelected { index } => {
                                sheet_to_select = Some(index);
                            }
                            TabularEvent::Failed {
                                message,
                                recoverable,
                            } => {
                                tab.web_error = Some(message.clone());
                                let _ = tab.backend_state.try_transition(
                                    crate::preview::BackendState::Failed(
                                        crate::preview::PreviewError::new(message, recoverable),
                                    ),
                                );
                                if tab.load_state.is_active() {
                                    tab.load_state.finish();
                                }
                            }
                        }
                    }
                    let pane = if panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    if push_initial {
                        pane.try_push_initial_tabular_state(binding.tab_id);
                    }
                    if let Some((sheet_index, start_row, start_col)) = restore_view {
                        pane.queue_tabular_command(
                            binding.tab_id,
                            crate::preview::TabularCommand::RestoreViewState {
                                sheet_index,
                                start_row,
                                start_col,
                            },
                        );
                    }
                    if let Some((sheet_index, start_row, rows, revision)) = window_push {
                        pane.queue_tabular_command(
                            binding.tab_id,
                            crate::preview::TabularCommand::SetWindow {
                                sheet_index,
                                start_row,
                                rows,
                                revision,
                            },
                        );
                    }
                    if let Some(index) = sheet_to_select {
                        ws.preview_pane_tabular_action(panel, binding.tab_id, index, io);
                    }
                });
            }
```

- [ ] **Step 2: `TabularLoaded` 处理里的恢复分支改调用新签名**(约第 2369-2376 行)

```rust
                    // 恢复的 active sheet 不是首个 → 触发一次懒加载。
                    if let Some(sheet) = sheet_to_select {
                        ws.preview_pane_tabular_action(kind, tab_id, sheet, io);
                    }
```

（原来传 `crate::tabular::Action::SelectSheet(sheet)`,现在直接传 `sheet: usize`——签名在 Task 12 改。）

- [ ] **Step 3: 扩展 `Message::TabularSheetLoaded` 处理,补推 webview 命令**(约第 2379-2392 行)

现状只把结果回填进 `TabularView`(`view.apply_sheet_loaded(...)`)供 iced 重绘用;迁移后还要:加载成功时把 webview 上对应 tab 的 loading 态清掉、把这个 sheet 的 `SetSchema`+`SetWindow` 推过去,并且**主动切换 JS 的活动 sheet**(这次懒加载可能是 Task 12 里"会话恢复到非首个 sheet"触发的,JS 侧此时还停在 `Init` 给的 sheet 0,不会自己知道要切过来——`push_sheet_schema_and_window` 内部已经用"`sheet_index` 是否仍是当前活动 sheet"做了保护,但"当前活动 sheet"这件事本身要先靠 `SelectSheet` 命令告诉 JS)。加载失败(`result` 是 `Err`)时只清 loading 态,不推 schema/window(该 sheet 保持"未加载"的原始占位,用户可以再点一次重试,同原生实现的既有退路)。

```rust
            Message::TabularSheetLoaded(project_id, kind, tab_id, sheet_index, result) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = if kind == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    let loaded_ok = result.is_ok();
                    if let Some(crate::preview::TabularState::Ready(view)) =
                        pane.tabular_state_mut(tab_id)
                    {
                        view.apply_sheet_loaded(sheet_index, result);
                    }
                    pane.queue_tabular_command(
                        tab_id,
                        crate::preview::TabularCommand::SetSheetLoading {
                            sheet_index,
                            loading: false,
                        },
                    );
                    if loaded_ok {
                        pane.queue_tabular_command(
                            tab_id,
                            crate::preview::TabularCommand::SelectSheet { sheet_index },
                        );
                        pane.push_sheet_schema_and_window(tab_id, sheet_index);
                    }
                });
            }
```

- [ ] **Step 4: 编译检查**

```bash
cargo check -p dozer-app 2>&1 | tail -60
```

Expected: 剩余报错应只指向 Task 12(`preview_pane_tabular_action` 签名不匹配)和 Task 13/14(`Message::TabularAction`/`crate::tabular::Action` 仍被引用)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app/update.rs
git commit -m "feat(tabular): handle TabularHostEvent rendezvous and window slicing"
```

---

### Task 12: `workspace/state.rs` — `preview_pane_tabular_action` 签名简化

**Files:**
- Modify: `crates/dozer-app/src/workspace/state.rs`

**Interfaces:**
- Consumes:`TabularView::select_sheet`(Task 13 把它从 `fn` 私有 + `#[allow(dead_code)]` 改成 `pub fn`——本任务先按"已是 pub"来写调用代码,Task 13 补齐定义;若严格 TDD 顺序要求先看到编译错误,可以先做 Task 13 再回来做本任务,两者顺序可互换,不影响最终状态)。
- Produces:`Workspace::preview_pane_tabular_action(&mut self, kind: PanelKind, tab_id: usize, sheet: usize, io: &ShellIo)`(不再接受 `crate::tabular::Action`)。

- [ ] **Step 1: 修改方法签名与实现**(约第 2519-2535 行)

```rust
    /// 表格预览 tab 的 sheet 切换(来自 Tabular webview host 的
    /// `sheet_selected` 事件,或启动恢复流程里"目标 sheet 非首个"的补触发)。
    /// tab 不存在 / 该 tab 不是表格 / 还在加载中都 no-op。切到一个还没加载过
    /// 的 sheet 时会 spawn 后台加载,完成后经 `Message::TabularSheetLoaded`
    /// 回填(按 `project_id` 而非"当前聚焦项目"路由,见该消息文档)。
    pub fn preview_pane_tabular_action(
        &mut self,
        kind: PanelKind,
        tab_id: usize,
        sheet: usize,
        io: &ShellIo,
    ) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let pane = if kind == PanelKind::Project {
            &mut self.project_preview
        } else {
            &mut self.preview
        };
        let Some(request) = pane
            .tabular_mut(tab_id)
            .and_then(|view| view.select_sheet(sheet))
        else {
            return;
        };
        // 该 sheet 还没加载过,才会走到这里:让 webview 把对应 tab 标 loading
        // 态(见 Task 4 的 `TabularCommand::SetSheetLoading`)。
        pane.queue_tabular_command(
            tab_id,
            crate::preview::TabularCommand::SetSheetLoading {
                sheet_index: request.index,
                loading: true,
            },
        );
        let cancel = pane
            .tabs()
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.task_cancel_token())
            .unwrap_or_else(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let proxy = io.proxy.clone();
        io.handle.spawn_blocking(move || {
            let cancelled = move || cancel.load(std::sync::atomic::Ordering::Relaxed);
            let result =
                crate::tabular::load_sheet_cancellable(&request.path, &request.name, cancelled);
            let _ = proxy.send_event(Message::TabularSheetLoaded(
                project_id,
                kind,
                tab_id,
                request.index,
                result,
            ));
        });
    }
```

- [ ] **Step 2: 编译检查**

```bash
cargo check -p dozer-app 2>&1 | tail -40
```

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/workspace/state.rs
git commit -m "refactor(tabular): simplify preview_pane_tabular_action to take sheet index"
```

---

### Task 13: `tabular/mod.rs` — 清理 `Action`/`apply`,`select_sheet` 转 `pub`

**Files:**
- Modify: `crates/dozer-app/src/tabular/mod.rs`

**Interfaces:**
- Consumes:无新依赖。
- Produces:`pub fn select_sheet(&mut self, sheet: usize) -> Option<SheetLoadRequest>`(原私有 `fn`,签名不变,只改可见性 + 去掉 `#[allow(dead_code)]`);删除 `pub enum Action`(`Scroll`/`SelectSheet` 两个变体)与 `TabularView::apply`。

- [ ] **Step 1: 找到 `select_sheet` 定义,去掉 `#[allow(dead_code)]` 注解与"内部原语(暂由测试使用)"这句过时注释,`fn` 改 `pub fn`**

```rust
    /// T12:切到 `sheet`(越界钳到合法),未加载则返回后台加载请求(调用方
    /// 物化后重试一次)。`sheet` 数为 0 时 no-op。来自 Tabular webview host
    /// 的 `sheet_selected` 事件,或启动恢复的"目标 sheet 非首个"补触发。
    pub fn select_sheet(&mut self, sheet: usize) -> Option<SheetLoadRequest> {
        if self.sheets.is_empty() {
            return None;
        }
        let idx = sheet.min(self.sheets.len() - 1);
        if idx == self.active_sheet {
            return None;
        }
        self.apply(Action::SelectSheet(idx))
    }
```

先只做可见性/注释这一步改动,**暂时保留** `self.apply(Action::SelectSheet(idx))` 这行调用(下一步再内联替换),避免中间态编译失败范围过大。

- [ ] **Step 2: 内联 `apply` 逻辑到 `select_sheet`,删除 `Action` 枚举与 `apply` 方法**

把 `select_sheet` 方法体里的 `self.apply(Action::SelectSheet(idx))` 替换为 `apply` 方法里 `Action::SelectSheet` 分支的原始逻辑(不含 `Action::Scroll` 分支——那个已经没有调用方,随 `apply` 一起删):

```rust
    pub fn select_sheet(&mut self, sheet: usize) -> Option<SheetLoadRequest> {
        if self.sheets.is_empty() {
            return None;
        }
        let idx = sheet.min(self.sheets.len() - 1);
        if idx == self.active_sheet {
            return None;
        }
        self.active_sheet = idx;
        self.scroll_row = 0;
        self.scroll_col = 0;
        // 已经有一次加载在飞就不再重复 spawn(2026-09 code review 发现:来回
        // 切走再切回同一个未加载 sheet,原来会对着同一份大文件重复触发后台
        // 加载)。
        if self.sheets[idx].is_none() && self.loading_sheets.insert(idx) {
            Some(SheetLoadRequest {
                index: idx,
                path: self.path.clone(),
                name: self.sheet_names[idx].clone(),
            })
        } else {
            None
        }
    }
```

删除整个 `pub fn apply(&mut self, action: Action) -> Option<SheetLoadRequest> { ... }` 方法与文件顶部/中部的 `pub enum Action { Scroll { dx: i32, dy: i32 }, SelectSheet(usize) }` 定义。

`reveal_cell`/`reveal_range` 方法体里调用的是私有方法 `self.select_sheet(sheet)`——现在它是 `pub`,调用点不用改,但这两个方法自己头上的 `#[allow(dead_code)] // T12:Agent reveal 内部原语(暂由测试使用)。` 也应删掉(它们经 Task 9 的 `apply_preview_command` 早已是真实调用路径,只是注解没跟上)。

- [ ] **Step 2: 全仓搜索确认没有残留引用**

```bash
grep -rn "tabular::Action\|crate::tabular::Action" crates/dozer-app/src
```

Expected: 无匹配(Task 11/12 已经把两处调用点改掉)。

- [ ] **Step 3: 编译 + 跑 `tabular` 模块测试**

```bash
cargo check -p dozer-app 2>&1 | tail -40
cargo test -p dozer-app --lib tabular::
```

Expected: `tabular` 模块自身单测(`select_sheet`/`reveal_cell`/`reveal_range` 相关)全部通过;若有测试直接构造 `Action::Scroll`/`Action::SelectSheet` 并调用 `.apply(...)`,按新 `select_sheet` 签名改写。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/tabular/mod.rs
git commit -m "refactor(tabular): drop Action/apply, promote select_sheet to pub"
```

---

### Task 14: 删除旧 iced 渲染实现 + 全量验证

**Files:**
- Delete: `crates/dozer-app/src/tabular/grid.rs`
- Delete: `crates/dozer-app/src/tabular/view.rs`
- Modify: `crates/dozer-app/src/tabular/mod.rs`(去掉 `pub mod grid;`/`pub mod view;`/`pub use grid::Action;`)
- Modify: `crates/dozer-app/src/workspace/view.rs`(删渲染分支)
- Modify: `crates/dozer-app/src/app/message.rs`(删 `TabularAction` 变体)
- Modify: `crates/dozer-app/src/app/update.rs`(删 `Message::TabularAction` 处理分支)

**Interfaces:** 无新增,纯删除 + 收尾验证。

- [ ] **Step 1: `tabular/mod.rs` 顶部去掉对已删模块的引用**

```rust
pub mod view; // 删除这行
```

原文件顶部是:

```rust
pub mod grid;
pub mod view;

pub use grid::Action;
```

全部删掉(三行都删,包括 `pub use grid::Action;`——`Action` 已在 Task 13 移除)。

- [ ] **Step 2: 删除文件**

```bash
git rm crates/dozer-app/src/tabular/grid.rs crates/dozer-app/src/tabular/view.rs
```

- [ ] **Step 3: `workspace/view.rs` 删除表格 tab 的 iced 原生渲染分支**

删除约第 1294-1334 行整个 `if let Some(tabular) = active_tab.tabular_state() && !active_tab.uses_editor_host() { ... }` 块(含 `Ready`/`Loading` 两个内部分支)。删除后,`else if let Some(page) = preview_fallback_page(...)` 这行(原第 1335 行)前面不应再有悬空的 `if`——把它改成整段逻辑的第一个 `if`:

```rust
        if let Some(page) = preview_fallback_page(kind, active_tab) {
            content = content.push(page);
        } else if let Some(loading) =
            preview_loading_view(&active_tab.load_state, active_tab.load_observe.as_ref())
        {
            content = content.push(loading);
        } else if active_tab.kind == TabKind::Blank {
            content = content.push(preview_blank_info_card(ws, preview));
        }
```

（Tabular tab 在 `Loading` 阶段会落进第二个分支 `preview_loading_view(...)`,复用统一 loading 动画,不再需要表格专属的"正在打开表格…"占位文案——这是符合预期的行为收敛,不是遗漏。）

- [ ] **Step 4: `message.rs` 删除 `TabularAction` 变体**

```rust
    TabularAction(PanelKind, usize, crate::tabular::Action),  // 删除这一整段(含上面的文档注释)
```

- [ ] **Step 5: `update.rs` 删除处理分支**

```rust
            Message::TabularAction(kind, tab_id, action) => {
                self.with_focused_project(move |ws, io| {
                    ws.preview_pane_tabular_action(kind, tab_id, action, io);
                });
            }
```

整段删除。

- [ ] **Step 6: 全仓搜索确认无残留引用**

```bash
grep -rn "TabularAction\|tabular::grid\|tabular::view\b" crates/dozer-app/src
```

Expected: 无匹配。

- [ ] **Step 7: 全量构建 + lint + 测试**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo test -p dozer-app
cargo fmt --check -p dozer-app
```

Expected: 全部通过,`clippy` 无新增警告(尤其留意 dead_code——如果 `select_sheet`/`reveal_cell`/`reveal_range` 仍报 dead_code,说明某处调用链没接上,回查 Task 9/11/12)。

- [ ] **Step 8: JS 侧最终构建校验(确保产物是最新的)**

```bash
cd crates/dozer-app/web/tabular-host && npm run build && cd -
git status --short crates/dozer-app/assets/tabular-host
```

Expected: 若产物有变化,一并加入本次 commit。

- [ ] **Step 9: 人工 GUI 走查(遵循 CLAUDE.md"UI 改动要在真实 GUI 里跑一遍"的要求)**

```bash
cargo run -p dozer-app
```

按 spec 的测试策略逐条过一遍,记录任何偏差:
1. 打开一个多 sheet 的 `.xlsx`:sheet tab 栏在 webview 里正确显示、点击切换、未加载的 sheet 显示 loading 态。
2. 打开一个超过 10 万行的大 `.csv`:截断提示条正确显示,滚动到底部/顶部时窗口正确续加载(不卡顿、不空白)。
3. 通过 agent(MCP `preview_navigate`)对一个已打开的表格 tab 发 reveal 请求:正确切 sheet、滚动到位、高亮目标单元格。
4. 关闭该 tab 重新打开(或重启 Dozer):sheet/滚动位置正确从持久化状态恢复。
5. 冻结表头行 + 首列(行号) + 斑马纹的视觉效果符合预期,配色跟随深/浅主题切换。
6. 尝试在网格里编辑/双击单元格:确认不可编辑(只读)。
7. 打开一个空 `.csv`(0 字节)或只有表头没有数据行的 `.csv`:不崩溃,网格区正常显示空态(Review Focus"空 sheet"一条)。

- [ ] **Step 10: 最终 commit**

```bash
git add -A
git commit -m "refactor(tabular): remove legacy iced canvas grid, cut over to ag-grid webview host"
```

- [ ] **Step 11: 收尾**

按 `superpowers:finishing-a-development-branch` 走完整分支收尾流程(代码审阅、合并到 `main`)。
