// Dozer tabular 预览 host 入口。与 CodeMirror/JSON host 共用消息 envelope
// 结构(见 Rust `preview/webview_protocol.rs::TabularCommand`/`TabularEvent`)。
// 正文完全由 Rust 推送(Init/SetSchema/SetWindow),本 host 不 fetch 任何文件。
import {
  createGrid,
  type ColDef,
  type GridApi,
  type IGetRowsParams,
  type GridOptions,
  type ValueGetterParams,
} from 'ag-grid-community';
import 'ag-grid-community/styles/ag-theme-quartz.css';
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
document.documentElement.classList.toggle('ag-theme-quartz-dark', scheme === 'dark');
document.documentElement.classList.toggle('ag-theme-quartz', scheme !== 'dark');

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

// 每个被 ag-grid `getRows` 请求的块,按 `${sheetIndex}:${startRow}:${endRow}`
// 记录待回填的回调。Rust 经 `set_window` 推回结果后按 start_row 匹配 success。
const pendingGetRows = new Map<string, { sheetIndex: number; params: IGetRowsParams }>();
const pendingBySheet = new Map<number, Set<string>>();

function buildGrid(sheetIndex: number, state: SheetState): void {
  const rowNumberCol: ColDef = {
    headerName: '',
    valueGetter: (p: ValueGetterParams) =>
      p.node?.rowIndex == null ? '' : p.node.rowIndex + 1,
    pinned: 'left',
    width: 64,
    sortable: false,
    filter: false,
    cellClass: 'row-number-cell',
  };
  const dataCols: ColDef[] = Array.from({ length: state.colCount }, (_, c) => ({
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
    datasource: {
      getRows: (p: IGetRowsParams) => {
        // 结果由 Rust 经 `set_window` 命令异步推回(见 applySetWindow)。
        const key = `${sheetIndex}:${p.startRow}`;
        pendingGetRows.set(key, { sheetIndex, params: p });
        let keys = pendingBySheet.get(sheetIndex);
        if (!keys) {
          keys = new Set();
          pendingBySheet.set(sheetIndex, keys);
        }
        keys.add(key);
        post({
          kind: 'window_request',
          sheet_index: sheetIndex,
          start_row: p.startRow,
          end_row: p.endRow,
        });
      },
    },
  };
  state.api = createGrid(state.container, options);
}

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
  // 空表(0 行/0 列):ag-grid 不会发起 getRows,直接显示空态覆盖层,
  // 避免无限 loading spinner(Review Focus"空 sheet"一条)。
  if (state.totalRows === 0) {
    state.api?.showNoRowsOverlay();
  } else {
    state.api?.hideOverlay();
  }
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
  const key = `${cmd.sheet_index}:${cmd.start_row}`;
  const pending = pendingGetRows.get(key);
  const rowData = cmd.rows.map((row) => Object.fromEntries(row.map((v, i) => [`c${i}`, v])));
  if (pending) {
    pending.params.successCallback(rowData, state.totalRows);
    pendingGetRows.delete(key);
    pendingBySheet.get(cmd.sheet_index)?.delete(key);
  }
  if (cmd.start_row === 0) {
    post({ kind: 'window_applied', start_row: cmd.start_row });
  }
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
  let env: {
    protocol_version?: number;
    revision?: number;
    payload?: { kind?: string; [k: string]: unknown };
  };
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
