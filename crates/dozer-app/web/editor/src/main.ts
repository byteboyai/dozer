// Dozer CodeMirror 6 editor host —— 入口(Phase B)。
//
// 约定(与 Rust `preview/webview_protocol.rs` 对应):
// - 坐标对外统一 1-based line/column;本文件负责 CodeMirror offset 与
//   1-based 位置互转。
// - 编辑器栈 -> Rust 走 `window.ipc.postMessage(envelope JSON)`;
//   Rust -> 编辑器走 `window.__dozer.dispatch(envelope JSON)`。
// - 文档内容由本 host 自行从 `dozer://editor/__file__<path>` 拉取(Rust 侧
//   只对白名单内路径放行),避免启动时把全文经 IPC 推一遍。

import { EditorState, Compartment, type Extension } from '@codemirror/state';
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
  highlightSpecialChars,
} from '@codemirror/view';
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from '@codemirror/commands';
import {
  searchKeymap,
  search,
  openSearchPanel,
  closeSearchPanel,
  setSearchQuery,
  SearchQuery,
} from '@codemirror/search';
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
} from '@codemirror/language';
import {
  PROTOCOL_VERSION,
  decodeCommand,
  encodeEnvelope,
  isRange,
  type Envelope,
  type EditorEvent,
  type Position,
  type Range,
} from './protocol';
import { themeFor } from './themes';
import { languageFor } from './languages';
import './editor.css';

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
// URLSearchParams 已经完成一次百分号解码；不要再次 decodeURIComponent，
// 否则文件名含有字面量 `%` 时会抛异常。fetch URL 由分段编码重建，
// 确保 `#`、`?`、`%` 等字符不会被当作 fragment/query。
const filePath = params.get('p') ?? '';
const initialReadOnly = params.get('ro') === '1';
const languageToken = params.get('lang') ?? 'txt';
// 窗口化只读 viewer(Phase C Task 3):正文由 Rust 经 set_window 推送,不自行
// 拉取;只持有全局行区间 [windowBase, windowBase+lines-1]。
const windowed = params.get('windowed') === '1';
let windowBase = 1;
let windowTotal = 0;
let windowHeldEnd = 0;
let lastWindowRequest = 0;
let applyingWindow = false;

document.documentElement.setAttribute('data-theme', scheme);

const languageCompartment = new Compartment();
const readOnlyCompartment = new Compartment();
const lineNumberCompartment = new Compartment();

function globalLineFormatter(lineNumber: number): string {
  return String(windowBase + lineNumber - 1);
}

function toGlobalLine(localLine: number): number {
  return windowed ? windowBase + localLine - 1 : localLine;
}

function toLocalLine(globalLine: number): number {
  return windowed ? globalLine - windowBase + 1 : globalLine;
}

let revision = 1;
let view: EditorView;
let saveHandler: (() => void) | null = null;

function post(event: EditorEvent, requestId: string | null = null): void {
  const env: Envelope<EditorEvent> = {
    protocol_version: PROTOCOL_VERSION,
    project_id: binding.project_id,
    panel: binding.panel,
    tab_id: binding.tab_id,
    document_id: binding.document_id,
    revision,
    request_id: requestId,
    payload: event,
  };
  const ipc = (window as unknown as { ipc?: { postMessage: (s: string) => void } }).ipc;
  if (ipc) {
    ipc.postMessage(encodeEnvelope(env));
  }
}

function offsetToPosition(offset: number): Position {
  const line = view.state.doc.lineAt(offset);
  // 对外坐标:窗口化时换算成全局 1-based 行号。
  return { line: toGlobalLine(line.number), column: offset - line.from + 1 };
}

function positionToOffset(pos: Position): number {
  const total = view.state.doc.lines;
  const lineNo = Math.min(Math.max(toLocalLine(pos.line), 1), total);
  const line = view.state.doc.line(lineNo);
  const column = Math.min(Math.max(pos.column, 1), line.length + 1);
  return line.from + (column - 1);
}

function currentRange(): Range {
  const sel = view.state.selection.main;
  return { start: offsetToPosition(sel.anchor), end: offsetToPosition(sel.head) };
}

function encodePathForFetch(path: string): string {
  return path.split('/').map((segment) => encodeURIComponent(segment)).join('/');
}

function throttle<T extends (...args: never[]) => void>(fn: T, ms: number): T {
  let last = 0;
  let timer: number | undefined;
  let pending: unknown[] | null = null;
  return ((...args: unknown[]) => {
    const now = Date.now();
    pending = args;
    if (now - last >= ms) {
      last = now;
      const a = pending;
      pending = null;
      fn(...(a as never[]));
    } else if (timer === undefined) {
      timer = window.setTimeout(() => {
        timer = undefined;
        if (pending) {
          last = Date.now();
          const a = pending;
          pending = null;
          fn(...(a as never[]));
        }
      }, ms - (now - last));
    }
  }) as unknown as T;
}

const emitSelection = throttle(() => {
  const sel = view.state.selection.main;
  post({
    kind: 'selection_changed',
    anchor: offsetToPosition(sel.anchor),
    head: offsetToPosition(sel.head),
    cursor: offsetToPosition(sel.head),
    selected_text: sel.empty ? null : view.state.sliceDoc(sel.from, sel.to),
  });
}, 80);

const emitViewport = throttle(() => {
  const { from, to } = view.viewport;
  post({
    kind: 'viewport_changed',
    from_line: toGlobalLine(view.state.doc.lineAt(from).number),
    to_line: toGlobalLine(view.state.doc.lineAt(to).number),
  });
}, 120);

/** 窗口化时滚到持有窗口边界 → 请求相邻窗口(节流 + 去重)。 */
function maybeRequestWindow(): void {
  if (!windowed || applyingWindow || windowTotal === 0) return;
  const now = Date.now();
  if (now - lastWindowRequest < 400) return;
  const { from, to } = view.viewport;
  const firstLocal = view.state.doc.lineAt(from).number;
  const lastLocal = view.state.doc.lineAt(to).number;
  const lines = view.state.doc.lines;
  if (lastLocal >= lines - 1 && windowHeldEnd < windowTotal) {
    lastWindowRequest = now;
    post({ kind: 'window_request', edge: 'bottom', anchor_line: toGlobalLine(lastLocal) });
  } else if (firstLocal <= 1 && windowBase > 1) {
    lastWindowRequest = now;
    post({ kind: 'window_request', edge: 'top', anchor_line: toGlobalLine(firstLocal) });
  }
}

function buildExtensions(): Extension[] {
  return [
    lineNumberCompartment.of(windowed ? lineNumbers({ formatNumber: globalLineFormatter }) : lineNumbers()),
    highlightActiveLineGutter(),
    highlightSpecialChars(),
    history(),
    foldGutter(),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    bracketMatching(),
    rectangularSelection(),
    crosshairCursor(),
    highlightActiveLine(),
    search({ top: true }),
    keymap.of([
      {
        key: 'Mod-s',
        preventDefault: true,
        run: () => {
          saveHandler?.();
          return true;
        },
      },
      {
        key: 'Mod-f',
        preventDefault: true,
        run: (v) => {
          openSearchPanel(v);
          return true;
        },
      },
      {
        key: 'Mod-r',
        preventDefault: true,
        run: (v) => {
          openSearchPanel(v);
          return true;
        },
      },
      indentWithTab,
      ...defaultKeymap,
      ...historyKeymap,
      ...searchKeymap,
      ...foldKeymap,
    ]),
    readOnlyCompartment.of(readOnlyExtensions(initialReadOnly)),
    languageCompartment.of(languageFor(languageToken) ?? []),
    themeFor(scheme),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        revision += 1;
        const changes: { from: number; to: number; insert: string }[] = [];
        update.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
          changes.push({ from: fromA, to: toA, insert: inserted.toString() });
        });
        post({
          kind: 'document_changed',
          revision,
          length: update.state.doc.length,
          changes,
        });
      }
      if (update.selectionSet) emitSelection();
      if (update.viewportChanged || update.geometryChanged) {
        emitViewport();
        maybeRequestWindow();
      }
      if (update.focusChanged) {
        post({ kind: 'focus_changed', focused: update.view.hasFocus });
      }
    }),
  ];
}

function readOnlyExtensions(readOnly: boolean): Extension {
  return readOnly ? [EditorState.readOnly.of(true), EditorView.editable.of(false)] : [];
}

saveHandler = () => {
  if (windowed) return; // 窗口化只读
  post({ kind: 'save_requested', revision, text: view.state.doc.toString() });
};

function applyCommand(raw: string): void {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return;
  }
  const env = parsed as Envelope;
  if (env.protocol_version !== PROTOCOL_VERSION) return;
  const cmd = decodeCommand(env.payload);
  if (!cmd) return;
  switch (cmd.kind) {
    case 'set_document': {
      revision = cmd.revision;
      view.setState(
        EditorState.create({
          doc: cmd.text,
          extensions: [
            ...buildExtensions(),
            languageCompartment.of(languageFor(cmd.language) ?? []),
            readOnlyCompartment.of(readOnlyExtensions(cmd.read_only)),
          ],
        }),
      );
      break;
    }
    case 'set_window': {
      revision = cmd.revision;
      windowBase = cmd.start_line;
      windowTotal = cmd.total_lines;
      applyingWindow = true;
      view.setState(
        EditorState.create({ doc: cmd.text, extensions: buildExtensions() }),
      );
      windowHeldEnd = windowBase + view.state.doc.lines - 1;
      applyingWindow = false;
      emitViewport();
      break;
    }
    case 'reveal_position': {
      const offset = positionToOffset({ line: cmd.line, column: cmd.column });
      view.dispatch({
        selection: { anchor: offset },
        effects: EditorView.scrollIntoView(offset, { y: 'center' }),
      });
      break;
    }
    case 'select_range': {
      if (!isRange(cmd)) break;
      const anchor = positionToOffset(cmd.start);
      const head = positionToOffset(cmd.end);
      view.dispatch({
        selection: { anchor, head },
        effects: EditorView.scrollIntoView(Math.min(anchor, head), { y: 'center' }),
      });
      break;
    }
    case 'replace_range': {
      // 窗口化恒只读,拒绝任何写入。
      if (windowed || !isRange(cmd) || cmd.revision !== revision) break;
      const from = positionToOffset(cmd.start);
      const to = positionToOffset(cmd.end);
      view.dispatch({ changes: { from, to, insert: cmd.text } });
      break;
    }
    case 'open_find': {
      if (cmd.query !== undefined && cmd.query !== '') {
        view.dispatch({ effects: setSearchQuery.of(new SearchQuery({ search: cmd.query })) });
      }
      openSearchPanel(view);
      break;
    }
    case 'set_read_only': {
      view.dispatch({
        effects: readOnlyCompartment.reconfigure(readOnlyExtensions(cmd.read_only)),
      });
      break;
    }
    case 'focus': {
      view.focus();
      break;
    }
    case 'serialize_view_state': {
      const sel = view.state.selection.main;
      const topLine = view.state.doc.lineAt(view.viewport.from).number;
      post(
        {
          kind: 'view_state',
          cursor: offsetToPosition(sel.head),
          selection: sel.empty ? null : currentRange(),
          top_line: topLine,
          folds: [],
        },
        cmd.request_id,
      );
      break;
    }
  }
}

(window as unknown as { __dozer?: unknown }).__dozer = { dispatch: applyCommand };

async function boot(): Promise<void> {
  let text = '';
  if (!windowed) {
    try {
      const res = await fetch('__file__' + encodePathForFetch(filePath));
      if (res.ok) {
        text = await res.text();
      } else {
        post({ kind: 'failed', message: `读取文件失败: ${res.status}`, recoverable: true });
      }
    } catch (err) {
      post({ kind: 'failed', message: `读取文件异常: ${String(err)}`, recoverable: true });
    }
  }

  view = new EditorView({
    state: EditorState.create({ doc: text, extensions: buildExtensions() }),
    parent: document.getElementById('editor')!,
  });

  post({
    kind: 'ready',
    read_only: initialReadOnly,
    language: languageToken,
  });
  emitViewport();
  view.focus();
}

void boot();

// 供 Rust 侧 `evaluate_script` 之外的关闭/清理留口(不导出到全局用户态)。
export { closeSearchPanel };
