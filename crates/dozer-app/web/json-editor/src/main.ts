// Dozer JSON tree/text host(vanilla-jsoneditor)入口(Phase D Task 3)。
//
// 与 CodeMirror host 共用消息 envelope 结构(见 Rust `preview/webview_protocol.rs`),
// 只扩展各自命令/事件名。正文由本 host 从 `dozer://json-editor/__file__<path>`
// 拉取(Rust 只对白名单放行)。

import { createJSONEditor, type Content } from 'vanilla-jsoneditor';

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
const filePath = params.get('p') ?? '';
const initialReadOnly = params.get('ro') === '1';

if (scheme === 'dark') {
  document.documentElement.classList.add('jse-theme-dark');
  document.body.classList.add('jse-theme-dark');
}

let revision = 1;
let editor: ReturnType<typeof createJSONEditor> | null = null;

function post(payload: Record<string, unknown>, requestId: string | null = null): void {
  const env = {
    protocol_version: PROTOCOL_VERSION,
    project_id: binding.project_id,
    panel: binding.panel,
    tab_id: binding.tab_id,
    document_id: binding.document_id,
    revision,
    request_id: requestId,
    payload,
  };
  const ipc = (window as unknown as { ipc?: { postMessage: (s: string) => void } }).ipc;
  ipc?.postMessage(JSON.stringify(env));
}

function encodePathForFetch(path: string): string {
  return path.split('/').map((s) => encodeURIComponent(s)).join('/');
}

function onEditorChange(content: Content): void {
  revision += 1;
  // 发送当前文本(JSON 工具不逐键回传增量;文本有上限由 Rust 侧兜底)。
  const text = 'text' in content && content.text !== undefined ? content.text : '';
  post({ kind: 'document_changed', revision, text });
}

function mountEditor(content: Content): void {
  const target = document.getElementById('json-editor-root')!;
  target.innerHTML = '';
  editor = createJSONEditor({
    target,
    props: {
      content,
      mode: 'tree',
      readOnly: initialReadOnly,
      mainMenuBar: false,
      navigationBar: false,
      statusBar: true,
      onChange: onEditorChange,
      onError: (err: unknown) => post({ kind: 'failed', message: String(err), recoverable: true }),
    },
  });
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
  if (!cmd || typeof cmd.kind !== 'string' || !editor) return;
  switch (cmd.kind) {
    case 'set_document': {
      const text = String(cmd.text ?? '');
      if (typeof cmd.revision === 'number') revision = cmd.revision;
      editor.set({ text });
      break;
    }
    case 'set_read_only': {
      editor.updateProps({ readOnly: Boolean(cmd.read_only) });
      break;
    }
    case 'focus': {
      (document.querySelector('.jse-main') as HTMLElement | null)?.focus?.();
      break;
    }
    default:
      break;
  }
}

(window as unknown as { __dozer?: unknown }).__dozer = { dispatch: applyCommand };

async function boot(): Promise<void> {
  let text = '';
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
  mountEditor({ text });
  post({ kind: 'ready', read_only: initialReadOnly, language: 'json' });
}

void boot();
