import { render } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import './styles.css';
import type { UsageViewPayload } from './types.ts';
import { App } from './components/App.tsx';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __dozer?: { dispatch(json: string): void };
  }
}

// 主题由 URL `?theme=light|dark` 注入(同 dozer://flyfish、dozer://html
// 的 `scheme_query_value()` 约定),缺省/未知值回落 dark。
function applyThemeFromUrl() {
  const theme = new URLSearchParams(location.search).get('theme');
  document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark';
}

function Root() {
  const [payload, setPayload] = useState<UsageViewPayload | null>(null);
  const [locale, setLocale] = useState<string>('en');

  useEffect(() => {
    window.__dozer = {
      dispatch(json: string) {
        try {
          const envelope = JSON.parse(json) as {
            payload?: UsageViewPayload;
            ui_locale?: string;
          };
          if (envelope.payload) setPayload(envelope.payload);
          // spec §8:语言上下文缺省(旧 Host)按 `en` 处理。locale 是响应式信号,
          // 供后续语言包接入;当前驱动 `document.documentElement.lang`。
          const uiLocale = typeof envelope.ui_locale === 'string' ? envelope.ui_locale : 'en';
          setLocale(uiLocale);
          if (typeof document !== 'undefined' && document.documentElement) {
            document.documentElement.lang = uiLocale;
          }
        } catch {
          // 忽略无法解析的推送——Rust 侧 `dispatch_script` 只在
          // `__dozer.dispatch` 已注册时才会注入,理论上不该收到坏 JSON。
        }
      },
    };
    // 页面初始化完成、`window.__dozer.dispatch` 已可用,报回 Rust——
    // Rust 侧收到后才开始推送(见 protocol.rs::UsageWebviewEvent::Ready、
    // Task 15 的 `WebviewPushState`)。
    window.ipc?.postMessage(JSON.stringify({ kind: 'ready' }));
    return () => {
      delete window.__dozer;
    };
  }, []);

  void locale;

  if (!payload) {
    return null;
  }
  return <App payload={payload} />;
}

applyThemeFromUrl();
render(<Root />, document.getElementById('root')!);
