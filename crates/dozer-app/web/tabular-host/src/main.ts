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
