import { render } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import './styles.css';
import { App } from './components/App.tsx';
import { applyEnvelope, DEFAULT_LOCALE, type PushState } from './protocol.ts';
import { send } from './ipc.ts';
import { shouldReportError } from './errors.ts';

// 主题由 URL `?theme=light|dark` 注入(同 dozer://group-chat-content),缺省回落 dark。
function applyThemeFromUrl() {
  const theme = new URLSearchParams(location.search).get('theme');
  document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark';
}

function Root() {
  const [state, setState] = useState<PushState>({
    revision: 0,
    payload: null,
    locale: DEFAULT_LOCALE,
    localeRevision: 0,
  });

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
