// Dozer JSON tree/text host(vanilla-jsoneditor)入口(Phase D Task 3)。
//
// 与 CodeMirror host 共用消息 envelope 结构(见 Rust `preview/webview_protocol.rs`),
// 只扩展各自命令/事件名。正文由本 host 从 `dozer://json-editor/__file__<path>`
// 拉取(Rust 只对白名单放行)。

import { createJSONEditor, type Content } from 'vanilla-jsoneditor';
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
const filePath = params.get('p') ?? '';
const initialReadOnly = params.get('ro') === '1';

document.documentElement.dataset.theme = scheme;
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
      // 导航栏承载搜索框(Ctrl+F)与当前路径;关闭它搜索功能会一并消失。
      navigationBar: true,
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
  // 先宣告 host JS 就绪(T6:`ready` 只代表脚本初始化,不代表正文可见)。
  post({ kind: 'ready', read_only: initialReadOnly, language: 'json' });
  let text: string | null = null;
  let error: string | null = null;
  try {
    const res = await fetch('__file__' + encodePathForFetch(filePath));
    if (res.ok) {
      text = await res.text();
    } else {
      error = `读取文件失败: ${res.status}`;
    }
  } catch (err) {
    error = `读取文件异常: ${String(err)}`;
  }
  if (error === null) {
    // Tree 视图要求合法 JSON:先显式解析,把非法 JSON 作为 `document_loaded.error`
    // 上报(而不是挂载后由 vanilla-jsoneditor 同步 `onError`)——否则 `failed`
    // 之后紧跟的成功 `document_loaded` 会覆盖错误态。
    try {
      JSON.parse(text ?? '');
    } catch (err) {
      error = `JSON 解析失败: ${String(err)}`;
    }
  }
  if (error === null) {
    mountEditor({ text: text ?? '' });
    post({ kind: 'document_loaded', revision, bytes: (text ?? '').length, error: null });
  } else {
    post({ kind: 'document_loaded', revision, bytes: 0, error });
  }
}

void boot();
