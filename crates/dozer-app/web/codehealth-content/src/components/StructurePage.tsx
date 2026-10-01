import { useState } from 'preact/hooks';
import type { StructureBody } from '../types.ts';
import { filterStructure } from '../structureFilter.ts';
import { FindingRowView } from './FindingRow.tsx';

export function StructurePage({ body }: { body: StructureBody }) {
  const [mode, setMode] = useState<'new' | 'all'>('new');
  const shown = filterStructure(body.findings, mode);
  return (
    <div>
      <div class="row">
        <h2>结构复杂度</h2>
        <span class="dim small">{body.metric_note}</span>
      </div>
      <div class="seg">
        <button class={mode === 'new' ? 'active' : ''} onClick={() => setMode('new')}>
          本轮新增
        </button>
        <button class={mode === 'all' ? 'active' : ''} onClick={() => setMode('all')}>
          全部
        </button>
      </div>
      {shown.length === 0 ? (
        <div class="dim">
          {mode === 'new' ? '本轮没有新增的结构复杂度发现。' : '没有发现结构复杂的函数。'}
        </div>
      ) : (
        shown.map((r) => <FindingRowView key={r.id} row={r} />)
      )}
    </div>
  );
}
