import type { UiBody } from '../types.ts';
import { FindingRowView } from './FindingRow.tsx';

export function UiPage({ body }: { body: UiBody }) {
  if (!body.applicable) {
    return (
      <div>
        <h2>UI 一致性</h2>
        <div class="dim">UI 一致性检测适用于 iced/Rust,当前项目未识别到适用框架。</div>
      </div>
    );
  }
  return (
    <div>
      <h2>UI 一致性</h2>
      {body.groups.map((g) => (
        <div key={g.title}>
          <h3>
            {g.title}:{g.findings.length} 处
          </h3>
          {g.findings.length === 0 ? (
            <div class="dim small">无发现</div>
          ) : (
            g.findings.map((r) => <FindingRowView key={r.id} row={r} />)
          )}
        </div>
      ))}
    </div>
  );
}
