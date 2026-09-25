import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { TraceData } from './types.ts';
import { SummaryHeader } from './components/SummaryHeader.tsx';
import { Entry } from './components/Entry.tsx';
import './styles.css';

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
    <>
      <SummaryHeader data={data} />
      {data.summary_text ? <hr class="topic-divider" /> : null}
      {data.entries.map((entry, i) => (
        <Entry entry={entry} agentLabel={data.agent_label} key={i} />
      ))}
    </>
  );
}

render(<App />, document.getElementById('timeline')!);
