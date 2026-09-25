import { Component, render, type ComponentChildren } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { TraceData } from './types.ts';
import { SummaryHeader } from './components/SummaryHeader.tsx';
import { Entry } from './components/Entry.tsx';
import './styles.css';

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
