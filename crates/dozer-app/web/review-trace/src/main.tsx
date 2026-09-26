import { Component, render, type ComponentChildren } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { TraceData } from './types.ts';
import { SummaryHeader } from './components/SummaryHeader.tsx';
import { Entry } from './components/Entry.tsx';
import './styles.css';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
  }
}

// review-trace 切换审阅目标时整页重新导航,期间原生 iced 显示 loading
// 占位、webview 保持隐藏(见 workspace/view.rs::review_webview_spec)。
// 页面达到任一终态(数据渲染成功 / fetch 失败 / 渲染期异常)都要报这一条,
// 让 Rust 侧切换可见——不区分成功或失败,失败态本身就是页面里可见的
// "加载失败: ..."文案,同样算"已经可以显示了"。
function reportDocumentLoaded() {
  window.ipc?.postMessage(
    JSON.stringify({ protocol_version: 1, payload: { kind: 'document_loaded' } }),
  );
}

// fetch/JSON 解析阶段的异常由下面 App 里的 `.catch` 兜底;但 entries 的
// 实际渲染发生在 Preact 调度的异步阶段(不在那个 `.then` 的同步调用栈里),
// 那里抛出的异常不会被那个 `.catch` 捕获,页面会停在渲染前的空白态而不是
// 显示"加载失败"。Preact 的渲染期异常会沿组件树向上找最近的
// `getDerivedStateFromError`/`componentDidCatch`,这里补一个错误边界兜底,
// 覆盖 `.catch` 覆盖不到的这一段。
class RenderErrorBoundary extends Component<{ children: ComponentChildren }, { error: string | null }> {
  state = { error: null as string | null };

  static getDerivedStateFromError(error: unknown) {
    return { error: String(error) };
  }

  render() {
    if (this.state.error) return <>{'加载失败: ' + this.state.error}</>;
    return this.props.children;
  }
}

function App() {
  const [data, setData] = useState<TraceData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch('dozer://review-trace/data.json')
      .then((r) => r.json())
      .then((d: TraceData) => setData(d))
      .catch((err) => setError(String(err)));
  }, []);

  // data/error 任一个从初始的 null 变为非 null,都代表这次导航已经走到
  // "可以展示内容"的终态(内容本身或错误文案),此时报 document_loaded。
  // 挂在 App 自身而不是 fetch 回调上,覆盖三种终态:fetch 成功(data 被设)、
  // fetch 失败(error 被设)、以及渲染期异常——渲染异常发生在 data 已被设置
  // 之后(拿到数据才会往下渲染 Entry),这次提交的 useEffect 已经在异常发生
  // 前触发,不受 RenderErrorBoundary 截获影响。
  useEffect(() => {
    if (data !== null || error !== null) {
      reportDocumentLoaded();
    }
  }, [data, error]);

  if (error) return <>{'加载失败: ' + error}</>;
  if (!data) return null;

  return (
    <RenderErrorBoundary>
      <SummaryHeader data={data} />
      {data.entries.map((entry, i) => (
        <Entry entry={entry} agentLabel={data.agent_label} key={i} />
      ))}
    </RenderErrorBoundary>
  );
}

render(<App />, document.getElementById('timeline')!);
