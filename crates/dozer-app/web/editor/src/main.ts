// Dozer CodeMirror 6 editor host —— 入口(Phase B)。
//
// 约定(与 Rust `preview/webview_protocol.rs` 对应):
// - 坐标对外统一 1-based line/column;本文件负责 CodeMirror offset 与
//   1-based 位置互转。
// - 编辑器栈 -> Rust 走 `window.ipc.postMessage(envelope JSON)`;
//   Rust -> 编辑器走 `window.__dozer.dispatch(envelope JSON)`。
// - 文档内容由本 host 自行从 `dozer://editor/__file__<path>` 拉取(Rust 侧
//   只对白名单内路径放行),避免启动时把全文经 IPC 推一遍。

import { EditorState, Compartment, StateField, StateEffect, type Extension } from '@codemirror/state';
import {
  EditorView,
  Decoration,
  type DecorationSet,
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
  highlightSelectionMatches,
} from '@codemirror/search';
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  foldedRanges,
  foldEffect,
  unfoldEffect,
} from '@codemirror/language';
import { unifiedMergeView } from '@codemirror/merge';
import {
  PROTOCOL_VERSION,
  decodeCommand,
  encodeEnvelope,
  isRange,
  type Envelope,
  type EditorEvent,
  type FoldRange,
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
// T6:非 UTF-8 有损文本——只读展示,保存被禁用(Rust 侧也会拒绝兜底)。
const lossy = params.get('lossy') === '1';
// T6:UTF-16(读取时已转码):只读展示并说明原因。
const utf16 = params.get('enc') === 'utf16';
const languageToken = params.get('lang') ?? 'txt';
// diff 模式(Git Log 内联 diff):正文由 Rust 经 set_diff_document 推送,
// 不 fetch 文件;无编辑语义(不 save、不 snapshot、不上报 document_changed)。
const diffMode = params.get('mode') === 'diff';
// 窗口化只读 viewer(Phase C Task 3):正文由 Rust 经 set_window 推送,不自行
// 拉取;只持有全局行区间 [windowBase, windowBase+lines-1]。
const windowed = params.get('windowed') === '1';
let windowBase = 1;
let windowTotal = 0;
let windowHeldEnd = 0;
let lastWindowRequest = 0;
let applyingWindow = false;
let globalScrollbar: HTMLInputElement | null = null;
let truncationBanner: HTMLDivElement | null = null;

function updateGlobalScrollbar(): void {
  if (!globalScrollbar || windowTotal <= 0) return;
  globalScrollbar.max = String(Math.max(1, windowTotal));
  globalScrollbar.value = String(Math.min(Math.max(windowBase, 1), windowTotal));
}

/** 超长单行导致窗口被字节上限截断时,在顶部显示一条提示;否则移除。 */
function updateTruncationBanner(truncated: boolean): void {
  const editorRoot = document.getElementById('editor');
  if (!editorRoot) return;
  if (truncated) {
    if (!truncationBanner) {
      truncationBanner = document.createElement('div');
      truncationBanner.className = 'windowed-truncation-banner';
      truncationBanner.textContent = '本行过长,仅显示开头部分';
      editorRoot.appendChild(truncationBanner);
    }
  } else if (truncationBanner) {
    truncationBanner.remove();
    truncationBanner = null;
  }
}

document.documentElement.setAttribute('data-theme', scheme);

const languageCompartment = new Compartment();
const readOnlyCompartment = new Compartment();
const lineNumberCompartment = new Compartment();

// Agent 短暂高亮:纯视觉装饰,不改变选区/光标。按 range 建一条 mark
// decoration,`duration_ms` 后自动清除;新的高亮覆盖旧的并取消旧计时器。
const setHighlight = StateEffect.define<{ from: number; to: number } | null>();
let highlightTimer: ReturnType<typeof setTimeout> | null = null;

const highlightMark = Decoration.mark({ class: 'dozer-agent-highlight' });

const highlightField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(setHighlight)) {
        deco = e.value === null
          ? Decoration.none
          : Decoration.set([highlightMark.range(e.value.from, e.value.to)]);
      }
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** 下发一次短暂高亮;`duration_ms` 后自行清除。 */
function applyHighlight(start: Position, end: Position, durationMs: number): void {
  const a = positionToOffset(start);
  const b = positionToOffset(end);
  const from = Math.min(a, b);
  const to = Math.max(a, b);
  view.dispatch({ effects: setHighlight.of({ from, to }) });
  if (highlightTimer !== null) clearTimeout(highlightTimer);
  highlightTimer = setTimeout(() => {
    highlightTimer = null;
    view.dispatch({ effects: setHighlight.of(null) });
  }, Math.max(1, durationMs));
}

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
// 正文是否相对磁盘有未保存改动。载入/重载/保存后清零,用户编辑(含程序性
// 替换)置位。只有 dirty 时才写 recovery 快照——否则"打开即失焦"会为未修改的
// 文件写一份恢复快照,下次打开被恢复并误标脏(`*`)。
let dirty = false;

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

function postRaw(body: string): void {
  const ipc = (window as unknown as { ipc?: { postMessage: (s: string) => void } }).ipc;
  ipc?.postMessage(body);
}

// 全局 UI 缩放:Ctrl/Cmd + `=`/`+`/`-`/`1`。webview 聚焦时按键到不了 winit
// (全局缩放在那里处理),这里在捕获阶段拦下并转发宿主——与 flyfish 注入脚本
// 同一约定。否则编辑器聚焦后 Ctrl++/- 无效。
document.addEventListener(
  'keydown',
  (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.code === 'Equal' || e.key === '+' || e.key === '=') {
      e.preventDefault();
      postRaw('zoom_in');
    } else if (e.code === 'Minus' || e.key === '-') {
      e.preventDefault();
      postRaw('zoom_out');
    } else if (e.code === 'Digit1' || e.key === '1') {
      e.preventDefault();
      postRaw('zoom_reset');
    }
  },
  true,
);

// 复制 / 剪切 / 粘贴:WKWebView 成为 first responder 后键盘事件被它吃掉、到
// 不了 winit,而本项目没有原生 Edit 菜单,AppKit 因此不会把 ⌘C/⌘X/⌘V 转成
// `copy:`/`cut:`/`paste:` 原生命令——依赖这些 DOM 剪贴板事件的 CodeMirror
// (它的 defaultKeymap 不接管剪贴板,全靠 `copy`/`paste` DOM 事件)拿不到任何
// 事件,复制粘贴直接失效。这里在捕获阶段显式处理:
// - 复制/剪切用 `document.execCommand(...)`(用户按键手势,WKWebView 放行),
//   选中内容由 CodeMirror 的选区决定;
// - 粘贴读 `navigator.clipboard.readText()`,异步拿到后经一个 CodeMirror
//   transaction 替换当前选区(`replaceSelection`),保持 undo 历史与
//   `document_changed` 上报一致。
document.addEventListener(
  'keydown',
  (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.code === 'KeyC' || (e.code === 'KeyX' && !view.state.readOnly)) {
      e.preventDefault();
      try {
        document.execCommand(e.code === 'KeyC' ? 'copy' : 'cut');
      } catch {
        /* 剪贴板被拒时静默,不阻断编辑 */
      }
    } else if (e.code === 'KeyV' && !view.state.readOnly) {
      e.preventDefault();
      void pasteFromClipboard();
    }
  },
  true,
);

async function pasteFromClipboard(): Promise<void> {
  try {
    const text = await navigator.clipboard.readText();
    if (!text) return;
    view.dispatch(view.state.replaceSelection(text));
  } catch {
    /* 无剪贴板权限/无文本表示(如图片)时静默 */
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

/** T11:导出当前折叠区(行范围)。 */
function currentFolds(): FoldRange[] {
  const out: FoldRange[] = [];
  foldedRanges(view.state).between(0, view.state.doc.length, (from, to) => {
    out.push({
      from_line: view.state.doc.lineAt(from).number,
      to_line: view.state.doc.lineAt(to).number,
    });
  });
  return out;
}

/** T11:按固定顺序恢复视图状态——folds → selection/cursor → scroll。 */
function restoreViewState(cmd: {
  cursor: Position | null;
  selection: Range | null;
  top_line: number | null;
  folds: FoldRange[];
}): void {
  const clampLine = (n: number) => Math.min(Math.max(n, 1), view.state.doc.lines);
  // 1) folds:先全展开,再按保存范围折叠。
  const unfold: StateEffect<unknown>[] = [];
  foldedRanges(view.state).between(0, view.state.doc.length, (from, to) => {
    unfold.push(unfoldEffect.of({ from, to }));
  });
  if (unfold.length > 0) view.dispatch({ effects: unfold });
  if (cmd.folds.length > 0) {
    const refold: StateEffect<unknown>[] = [];
    for (const f of cmd.folds) {
      const from = view.state.doc.line(clampLine(f.from_line)).from;
      const to = view.state.doc.line(clampLine(f.to_line)).to;
      if (to > from) refold.push(foldEffect.of({ from, to }));
    }
    if (refold.length > 0) view.dispatch({ effects: refold });
  }
  // 2) selection / cursor。
  if (cmd.selection) {
    const anchor = positionToOffset(cmd.selection.start);
    const head = positionToOffset(cmd.selection.end);
    view.dispatch({ selection: { anchor, head } });
  } else if (cmd.cursor) {
    const offset = positionToOffset(cmd.cursor);
    view.dispatch({ selection: { anchor: offset } });
  }
  // 3) scroll(最后;窗口化时按全局行号换算)。
  if (cmd.top_line !== null) {
    const offset = view.state.doc.line(clampLine(toLocalLine(cmd.top_line))).from;
    view.dispatch({ effects: EditorView.scrollIntoView(offset, { y: 'start' }) });
  }
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
    // 选中一个词(如双击)时,高亮同一文件里其他位置的相同文字。
    highlightSelectionMatches(),
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
          if (windowed) {
            // 窗口化:只搜持有窗口没意义,交给 Rust 在整文件上流式搜索。
            post({ kind: 'find_request' });
            return true;
          }
          openSearchPanel(v);
          return true;
        },
      },
      {
        key: 'Mod-r',
        preventDefault: true,
        run: (v) => {
          if (windowed) {
            post({ kind: 'find_request' });
            return true;
          }
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
    highlightField,
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
        dirty = true;
        scheduleSnapshot();
      }
      if (update.selectionSet) emitSelection();
      if (update.viewportChanged || update.geometryChanged) {
        emitViewport();
        maybeRequestWindow();
      }
      if (update.focusChanged) {
        post({ kind: 'focus_changed', focused: update.view.hasFocus });
        if (!update.view.hasFocus) sendSnapshot();
      }
    }),
  ];
}

function readOnlyExtensions(readOnly: boolean): Extension {
  return readOnly ? [EditorState.readOnly.of(true), EditorView.editable.of(false)] : [];
}

/** diff 模式下 `unifiedMergeView` 的扩展集合:doc 为"新"文本,original 为
 *  "旧"文本;关掉 accept/reject 控件(只读,不给用户改 diff 的入口)与
 *  "未变区段折叠"(Git Log 要看整份 diff),保留变更高亮与 gutter 标记。 */
function diffMergeExtension(original: string): Extension {
  return unifiedMergeView({
    original,
    highlightChanges: true,
    gutter: true,
    mergeControls: false,
    collapseUnchanged: undefined,
    syntaxHighlightDeletions: true,
  });
}

/** diff 模式的扩展栈:只读、无 history(无编辑)、语言走后续 reconfigure。
 *  与普通编辑器共用行号/gutter/滚动基础,但不接保存、快照、窗口化找回。 */
function buildDiffExtensions(oldText: string, language: string, readOnly: boolean): Extension[] {
  return [
    lineNumbers(),
    highlightSpecialChars(),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    foldGutter(),
    keymap.of([...defaultKeymap, ...foldKeymap]),
    diffMergeExtension(oldText),
    readOnlyCompartment.of(readOnlyExtensions(readOnly)),
    languageCompartment.of(languageFor(language) ?? []),
    highlightField,
    themeFor(scheme),
    EditorView.updateListener.of((update) => {
      // 只读 diff:不做 document_changed / snapshot 上报;仅同步 viewport
      // (Rust 侧不一定用,但保持与普通 host 一致的最小事件面)。
      if (update.viewportChanged || update.geometryChanged) {
        const { from, to } = update.view.viewport;
        post({
          kind: 'viewport_changed',
          from_line: update.view.state.doc.lineAt(from).number,
          to_line: update.view.state.doc.lineAt(to).number,
        });
      }
    }),
  ];
}

saveHandler = () => {
  if (diffMode || windowed || lossy || view.state.readOnly) return; // 只读:不落盘
  post({ kind: 'save_requested', revision, text: view.state.doc.toString() });
  // 保存请求已发出:视作不再有本地未存改动(若 Rust 侧因磁盘冲突/只读回退拒绝,
  // 会经 reload/SetDocument 重新同步并按需再标脏)。
  dirty = false;
  lastSnapshotRevision = revision;
};

// 脏正文 recovery 快照:编辑后 1.5s 防抖上报一次;失焦时立即补一次。窗口化
// 只读、无脏内容,不参与。**只有 dirty 时才写**——否则刚打开、未做任何修改的
// 文件在失焦时也会写一份等于磁盘内容的快照,下次打开被当"恢复"而误标脏。
let snapshotTimer: number | undefined;
let lastSnapshotRevision = 0;
function sendSnapshot(): void {
  if (diffMode || windowed) return;
  snapshotTimer = undefined;
  if (!dirty) return;
  if (revision === lastSnapshotRevision) return;
  lastSnapshotRevision = revision;
  post({ kind: 'snapshot', revision, text: view.state.doc.toString() });
}
function scheduleSnapshot(): void {
  if (diffMode || windowed) return;
  if (snapshotTimer !== undefined) window.clearTimeout(snapshotTimer);
  snapshotTimer = window.setTimeout(sendSnapshot, 1500);
}

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
      dirty = false;
      lastSnapshotRevision = revision;
      break;
    }
    case 'set_diff_document': {
      revision = cmd.revision;
      // 每次换文件都是"新 doc + 新 original",整份重建最干净(不留旧 diff
      // 装饰);webview 实例本身不重建,由 Rust 侧的固定单槽负责。
      view.setState(
        EditorState.create({
          doc: cmd.new_text,
          extensions: buildDiffExtensions(cmd.old_text, cmd.language, cmd.read_only),
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
      updateGlobalScrollbar();
      updateTruncationBanner(cmd.truncated === true);
      emitViewport();
      // T4:正文已真正挂上 → 回报 Rust 作为窗口化 Ready 边界(收到前 host 保
      // 持 hidden/loading,不会先露出空编辑器)。
      post({ kind: 'window_applied', start_line: cmd.start_line });
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
    case 'highlight_range': {
      if (!isRange(cmd)) break;
      applyHighlight(cmd.start, cmd.end, cmd.duration_ms);
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
          folds: currentFolds(),
        },
        cmd.request_id,
      );
      break;
    }
    case 'restore_view_state': {
      restoreViewState({
        cursor: cmd.cursor ?? null,
        selection: cmd.selection ?? null,
        top_line: cmd.top_line ?? null,
        folds: cmd.folds ?? [],
      });
      break;
    }
    case 'save_document': {
      // Rust 侧关闭 dirty tab 前下发:复用 ⌘S 的落盘链路,回
      // `save_requested`。窗口化只读无脏内容,忽略。
      saveHandler?.();
      break;
    }
    case 'reload_document': {
      // T5:磁盘外部变更后**就地**重载(重新 fetch,不重新导航)。旧正文一直
      // 可见,直到新内容挂上才回 `document_loaded`。
      if (!windowed) void reloadDocument();
      break;
    }
  }
}

/** 载入磁盘正文:整体替换 doc 并把 undo 历史重置为空。
 *
 *  不能走 `view.dispatch({ changes })`——`history()` 从建 view 起就生效,把
 *  初始(空)→全文这次程序性替换记进历史后,刚打开文件按 ⌘Z 会把整个文档
 *  撤销清空。改为重建 EditorState(等价于 `set_document`),history 从零开始,
 *  ⌘Z 在没有任何用户编辑前无事可撤。 */
function loadDocumentText(text: string): void {
  const extensions = windowed
    ? buildExtensions()
    : diffMode
      ? buildDiffExtensions('', languageToken, initialReadOnly)
      : buildExtensions();
  view.setState(EditorState.create({ doc: text, extensions }));
  // 正文刚与磁盘对齐:清 dirty 并让下次失焦不再补写"未修改"快照。
  dirty = false;
  lastSnapshotRevision = revision;
}

/** T5:重新拉取磁盘内容并就地替换 doc(保留滚动/焦点尽量不动)。 */
async function reloadDocument(): Promise<void> {
  let text = '';
  let bytes = 0;
  let error: string | null = null;
  try {
    const res = await fetch('__file__' + encodePathForFetch(filePath));
    if (res.ok) {
      text = await res.text();
      bytes = new TextEncoder().encode(text).length;
    } else {
      error = `读取文件失败: ${res.status}`;
    }
  } catch (err) {
    error = `读取文件异常: ${String(err)}`;
  }
  if (error === null) {
    revision += 1;
    loadDocumentText(text);
  }
  post({ kind: 'document_loaded', revision, bytes, error });
}

(window as unknown as { __dozer?: unknown }).__dozer = { dispatch: applyCommand };

async function boot(): Promise<void> {
  // T5:先以**空正文**建 view 并 immediate 上报 `ready`——`ready` 只表示 host JS
  // 初始化完成,正文是否可见由随后的 `document_loaded` 表达。这样 Rust 侧可以
  // 用 `document_loaded`(带 revision/bytes/error)作为非窗口化 tab 的 Ready 边界,
  // 而不会把"host 起了但正文还没到/读取失败"误判为已就绪(旧实现 fetch 失败时
  // 仍会补一个 `ready`,把 Failed 态拉回)。
  //
  // diff 模式(Git Log 内联 diff)正文由 Rust 经 set_diff_document 推送,既不在
  // 这里 fetch,也不发 `document_loaded`(URL 亦不带 `p=`);窗口化同理。
  view = new EditorView({
    state: EditorState.create({
      extensions: diffMode
        ? buildDiffExtensions('', languageToken, initialReadOnly)
        : buildExtensions(),
    }),
    parent: document.getElementById('editor')!,
  });

  // Phase 2:预览内选区右键"发送给 Agent"。选区为空时完全不拦截——右键
  // 照常弹浏览器原生菜单;不做"发送整篇文件"这类超出范围的功能。选区非空
  // 时现场取值上报,不依赖 `emitSelection` 的节流缓存(避免与最后一次
  // selection_changed 之间的时序竞态)。
  view.contentDOM.addEventListener(
    'contextmenu',
    (e) => {
      const sel = view.state.selection.main;
      if (sel.empty) return;
      e.preventDefault();
      // `currentRange()` 按 anchor/head 顺序给(反向选区时 start 在 end 之后),
      // 但 `selected_text` 用 `sliceDoc(sel.from, sel.to)` 恒正向取值——这里
      // 同样按 `from`/`to` 算 range,保证两者方向一致,不给 agent 一个头尾
      // 颠倒的坐标区间。
      post({
        kind: 'context_menu_requested',
        x: e.clientX,
        y: e.clientY,
        range: { start: offsetToPosition(sel.from), end: offsetToPosition(sel.to) },
        selected_text: view.state.sliceDoc(sel.from, sel.to),
      });
    },
    true,
  );

  // T6:非 UTF-8 / UTF-16 只读:顶部常驻提示,说明只读原因(保存由
  // saveHandler / Rust 双重拒绝)。
  if (lossy || utf16) {
    const editorRoot = document.getElementById('editor')!;
    const banner = document.createElement('div');
    banner.className = 'windowed-truncation-banner';
    banner.textContent = lossy
      ? '该文件不是有效的 UTF-8 文本,仅只读显示;保存已禁用以免破坏原文件'
      : '该文件是 UTF-16 编码,已转码为只读文本;保存已禁用以免改变原编码';
    editorRoot.appendChild(banner);
  }

  if (windowed) {
    const editorRoot = document.getElementById('editor')!;
    globalScrollbar = document.createElement('input');
    globalScrollbar.type = 'range';
    globalScrollbar.className = 'windowed-global-scrollbar';
    globalScrollbar.min = '1';
    globalScrollbar.step = '1';
    globalScrollbar.setAttribute('aria-label', '全局文件滚动位置');
    globalScrollbar.addEventListener('input', () => {
      const line = Number(globalScrollbar?.value ?? 1);
      if (Number.isFinite(line)) {
        lastWindowRequest = Date.now();
        post({ kind: 'window_request', edge: 'bottom', anchor_line: Math.max(1, Math.round(line)) });
      }
    });
    editorRoot.appendChild(globalScrollbar);
    updateGlobalScrollbar();
  }

  post({
    kind: 'ready',
    read_only: initialReadOnly,
    language: languageToken,
  });

  if (!windowed && !diffMode) {
    let text = '';
    let bytes = 0;
    let error: string | null = null;
    try {
      const res = await fetch('__file__' + encodePathForFetch(filePath));
      if (res.ok) {
        text = await res.text();
        bytes = new TextEncoder().encode(text).length;
      } else {
        error = `读取文件失败: ${res.status}`;
      }
    } catch (err) {
      error = `读取文件异常: ${String(err)}`;
    }
    if (error === null) {
      // 正文真正挂上后再上报 `document_loaded`(Rust 以此 finish 加载并显示)。
      // 用整体重建而非 dispatch change,避免初始载入被记进 undo 历史。
      loadDocumentText(text);
    }
    post({ kind: 'document_loaded', revision, bytes, error });
  }

  emitViewport();
  view.focus();
}

void boot();

// 供 Rust 侧 `evaluate_script` 之外的关闭/清理留口(不导出到全局用户态)。
export { closeSearchPanel };
