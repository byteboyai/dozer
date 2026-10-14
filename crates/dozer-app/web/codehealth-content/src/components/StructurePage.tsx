import { useViewState } from '../useViewState.ts';
import type { StructureBody } from '../types.ts';
import { filterStructure } from '../structureFilter.ts';
import { FindingRowView } from './FindingRow.tsx';
import { t } from '../i18n.ts';

export function StructurePage({ body }: { body: StructureBody }) {
  const [mode, setMode] = useViewState<'new' | 'all'>('structure.mode', 'new');
  const shown = filterStructure(body.findings, mode);
  return (
    <div>
      <div class="row">
        <h2>{t('structure.title', 'Structure Complexity')}</h2>
        <span class="dim small">
          {t(
            'structure.metric_note',
            'Metric definition: control-flow signal (not standard cyclomatic complexity)',
          )}
        </span>
      </div>
      <div class="seg">
        <button class={mode === 'new' ? 'active' : ''} onClick={() => setMode('new')}>
          {t('structure.filter.new', 'New this scan')}
        </button>
        <button class={mode === 'all' ? 'active' : ''} onClick={() => setMode('all')}>
          {t('structure.filter.all', 'All')}
        </button>
      </div>
      {shown.length === 0 ? (
        <div class="dim">
          {mode === 'new'
            ? t('structure.empty.new', 'No new structure complexity findings this scan.')
            : t('structure.empty.all', 'No structurally complex functions found.')}
        </div>
      ) : (
        shown.map((r) => <FindingRowView key={r.id} row={r} />)
      )}
    </div>
  );
}
