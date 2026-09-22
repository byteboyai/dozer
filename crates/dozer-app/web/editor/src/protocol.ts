// 通用 webview 消息 envelope 的 TypeScript 端(Phase B Task 3)。
// 与 Rust `preview/webview_protocol.rs` 一一对应;CodeMirror / Flyfish /
// vanilla-jsoneditor 三种 host 共用这个结构,各自只扩展命令/事件名。
//
// 坐标协议:对外统一 1-based line/column(桥接层负责 CodeMirror offset 与
// Unicode 列 / UTF-8 byte offset 的转换)。

export const PROTOCOL_VERSION = 1;

/** 1-based 文本位置。 */
export interface Position {
  line: number;
  column: number;
}

export interface Range {
  start: Position;
  end: Position;
}

export interface FoldRange {
  from_line: number;
  to_line: number;
}

/** 窗口化 viewer 请求相邻窗口的方向。 */
export type WindowEdge = 'top' | 'bottom';

/** 编辑器栈 -> Rust 的事件。 */
export type EditorEvent =
  | { kind: 'ready'; read_only: boolean; language: string }
  | {
      kind: 'selection_changed';
      anchor: Position;
      head: Position;
      cursor: Position;
      selected_text: string | null;
    }
  | {
      kind: 'document_changed';
      revision: number;
      length: number;
      changes: { from: number; to: number; insert: string }[];
    }
  | { kind: 'save_requested'; revision: number; text: string }
  | { kind: 'snapshot'; revision: number; text: string }
  | { kind: 'focus_changed'; focused: boolean }
  | { kind: 'viewport_changed'; from_line: number; to_line: number }
  | {
      kind: 'view_state';
      cursor: Position;
      selection: Range | null;
      top_line: number;
      folds: FoldRange[];
    }
  | { kind: 'window_request'; edge: WindowEdge; anchor_line: number }
  | { kind: 'find_request' }
  | { kind: 'failed'; message: string; recoverable: boolean };

/** Rust -> 编辑器的具名命令(禁止执行任意 JS)。 */
export type EditorCommand =
  | {
      kind: 'set_document';
      text: string;
      revision: number;
      language: string;
      read_only: boolean;
    }
  | {
      kind: 'set_window';
      text: string;
      start_line: number;
      total_lines: number;
      revision: number;
    }
  | { kind: 'reveal_position'; line: number; column: number }
  | { kind: 'select_range'; start: Position; end: Position }
  | {
      kind: 'replace_range';
      start: Position;
      end: Position;
      text: string;
      revision: number;
    }
  | { kind: 'open_find'; query?: string; replace?: boolean }
  | { kind: 'set_read_only'; read_only: boolean }
  | { kind: 'focus' }
  | { kind: 'serialize_view_state'; request_id: string };

/** 三端 host 共用的 envelope。 */
export interface Envelope<P = unknown> {
  protocol_version: number;
  project_id: number;
  panel: string;
  tab_id: number;
  document_id: string;
  revision: number;
  request_id: string | null;
  payload: P;
}

export function isPosition(v: unknown): v is Position {
  if (typeof v !== 'object' || v === null) return false;
  const p = v as Record<string, unknown>;
  return (
    typeof p.line === 'number' &&
    Number.isInteger(p.line) &&
    p.line >= 1 &&
    typeof p.column === 'number' &&
    Number.isInteger(p.column) &&
    p.column >= 1
  );
}

export function isRange(v: unknown): v is Range {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Record<string, unknown>;
  return isPosition(r.start) && isPosition(r.end);
}

/** envelope 的静态形状校验(不含 project/tab 归属——那是 Rust 侧的事)。 */
export function isValidEnvelope(v: unknown): v is Envelope {
  if (typeof v !== 'object' || v === null) return false;
  const e = v as Record<string, unknown>;
  return (
    e.protocol_version === PROTOCOL_VERSION &&
    typeof e.project_id === 'number' &&
    typeof e.panel === 'string' &&
    typeof e.tab_id === 'number' &&
    Number.isInteger(e.tab_id) &&
    typeof e.document_id === 'string' &&
    typeof e.revision === 'number' &&
    (e.request_id === null || typeof e.request_id === 'string') &&
    typeof e.payload === 'object' &&
    e.payload !== null
  );
}

export function encodeEnvelope<P>(env: Envelope<P>): string {
  return JSON.stringify(env);
}

/** 解析失败(非 JSON / 形状不对)返回 `null`,调用方不 panic、静默丢弃。 */
export function decodeEnvelope(raw: string): Envelope | null {
  try {
    const parsed: unknown = JSON.parse(raw);
    return isValidEnvelope(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/** 解析 Rust 下发的命令;未知/非法一律 `null`。 */
export function decodeCommand(payload: unknown): EditorCommand | null {
  if (typeof payload !== 'object' || payload === null) return null;
  const c = payload as Record<string, unknown>;
  switch (c.kind) {
    case 'set_document':
      return typeof c.text === 'string' &&
        typeof c.revision === 'number' &&
        typeof c.language === 'string' &&
        typeof c.read_only === 'boolean'
        ? (c as unknown as EditorCommand)
        : null;
    case 'set_window':
      return typeof c.text === 'string' &&
        typeof c.start_line === 'number' &&
        typeof c.total_lines === 'number' &&
        typeof c.revision === 'number'
        ? (c as unknown as EditorCommand)
        : null;
    case 'reveal_position':
      return typeof c.line === 'number' && typeof c.column === 'number'
        ? (c as unknown as EditorCommand)
        : null;
    case 'select_range':
      return isRange(c) ? (c as unknown as EditorCommand) : null;
    case 'replace_range':
      return isRange(c) &&
        typeof c.text === 'string' &&
        typeof c.revision === 'number'
        ? (c as unknown as EditorCommand)
        : null;
    case 'open_find':
      return c as unknown as EditorCommand;
    case 'set_read_only':
      return typeof c.read_only === 'boolean'
        ? (c as unknown as EditorCommand)
        : null;
    case 'focus':
      return c as unknown as EditorCommand;
    case 'serialize_view_state':
      return typeof c.request_id === 'string'
        ? (c as unknown as EditorCommand)
        : null;
    default:
      return null;
  }
}
