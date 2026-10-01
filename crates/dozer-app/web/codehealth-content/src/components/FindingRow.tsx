import type { FindingRow as Row } from '../types.ts';
import { send } from '../ipc.ts';

export function FindingRowView({ row, analyze }: { row: Row; analyze?: boolean }) {
  return (
    <div class="finding" onClick={() => send({ kind: 'open_location', path: row.path, line: row.line })}>
      <div class="finding-head">
        <span class={`badge ${row.severity}`}>{row.severity_label}</span>
        <span class="finding-title">{row.title}</span>
        {row.change_label && <span class="dim small">{row.change_label}</span>}
      </div>
      <div class="finding-meta">
        {row.path}:{row.line}
        {row.reasons.length > 0 && <> · {row.reasons.join(' · ')}</>}
      </div>
      {analyze && (
        <div class="finding-actions">
          <button
            class="btn small"
            onClick={(e) => {
              e.stopPropagation();
              send({ kind: 'analyze_finding', id: row.id });
            }}
          >
            交给 Agent 分析
          </button>
        </div>
      )}
    </div>
  );
}
