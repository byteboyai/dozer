import { useMemo, useState } from 'preact/hooks';
import { useViewState } from '../useViewState.ts';
import type { ArchitectureBody } from '../types.ts';
import { projectGraph } from '../graph/projection.ts';
import { addedNodeIds, impactNodeIds, riskForNode, riskIndex } from '../graph/linking.ts';
import { GraphCanvas } from './GraphCanvas.tsx';
import { FindingRowView } from './FindingRow.tsx';
import { t } from '../i18n.ts';

export function ArchitecturePage({ body }: { body: ArchitectureBody }) {
  const [storedLayer, setLayer] = useViewState<'crate' | 'module'>('arch.layer', 'crate');
  const [riskOnly, setRiskOnly] = useViewState('arch.riskOnly', false);
  const [expanded, setExpanded] = useViewState<ReadonlySet<string>>('arch.expanded', new Set());
  const [selectedId, setSelectedId] = useViewState<string | null>('arch.selected', null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [fitSignal, setFitSignal] = useState(0);
  const [anchorId, setAnchorId] = useState<string | null>(null);

  // 大图被裁成 crate 层(或没有模块节点)时 module 层是空图:不可选,并回落到 crate。
  const hasModules = body.nodes.some((n) => n.kind === 'module');
  const layer = hasModules ? storedLayer : 'crate';

  const idx = useMemo(() => riskIndex(body.risks), [body.risks]);
  const impact = useMemo(() => impactNodeIds(body.impact), [body.impact]);
  const added = useMemo(() => addedNodeIds(body.diff), [body.diff]);
  const graph = useMemo(
    () =>
      projectGraph({
        nodes: body.nodes,
        edges: body.edges,
        layer,
        expanded,
        riskOnly,
        riskNodeIds: idx.nodeIds,
        riskEdgeIds: idx.edgeIds,
      }),
    [body.nodes, body.edges, layer, expanded, riskOnly, idx],
  );

  const selected = graph.nodes.find((n) => n.id === selectedId) ?? null;
  const selectedRisks = selected ? riskForNode(body.risks, selected.representedIds) : [];
  const activeRiskIds = new Set(selectedRisks.map((r) => r.finding.id));

  // spec §167:优先语义 key,旧 Host 缺字段时回落预格式化文案。
  const statusNote = body.status_note_key
    ? t(body.status_note_key, body.status_note ?? '')
    : body.status_note;
  const truncatedNote =
    body.truncated_note_total !== undefined && body.truncated_note_total !== null
      ? t(
          'arch.truncated_note',
          'Module graph too large ({total} nodes); showing crate layer only. Analyses (cycles/hubs/boundaries) still use the full graph.',
          { total: String(body.truncated_note_total) },
        )
      : body.truncated_note;

  if (body.status === 'not_applicable') {
    return (
      <div>
        <h2>{t('arch.title', 'Architecture Map')}</h2>
        <div class="banner note">{statusNote}</div>
      </div>
    );
  }

  const switchLayer = (next: 'crate' | 'module') => {
    setLayer(next);
    setAnchorId(null);
    setFitSignal((n) => n + 1);
  };

  const toggleExpand = (id: string) => {
    const node = graph.nodes.find((n) => n.id === id);
    if (!node || node.childCount === 0) return;
    setAnchorId(id);
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const focusRisk = (nodeIds: string[]) => {
    // 风险涉及的节点在当前投影里可能被折叠进某个可见节点,找第一个可见代表。
    const hit = graph.nodes.find((n) => n.representedIds.some((id) => nodeIds.includes(id)));
    if (hit) {
      setSelectedId(hit.id);
      setFocusId(hit.id);
    }
  };

  return (
    <div class="arch-layout">
      <div class="arch-toolbar">
        <h2>{t('arch.title', 'Architecture Map')}</h2>
        <div class="seg">
          <button
            class={layer === 'crate' ? 'active' : ''}
            onClick={() => switchLayer('crate')}
          >
            crate
          </button>
          <button
            class={layer === 'module' ? 'active' : ''}
            disabled={!hasModules}
            title={hasModules ? undefined : t('arch.no_modules', 'No module nodes to display')}
            onClick={() => switchLayer('module')}
          >
            module
          </button>
        </div>
        <label class="switch">
          <input
            type="checkbox"
            checked={riskOnly}
            onChange={(e) => {
              setRiskOnly((e.currentTarget as HTMLInputElement).checked);
              setAnchorId(null);
              setFitSignal((n) => n + 1);
            }}
          />
          {t('arch.risk_only', 'Risks only')}
        </label>
        <button class="btn small" onClick={() => setFitSignal((n) => n + 1)}>
          {t('arch.fit', 'Fit to window')}
        </button>
        <span class="legend">
          <span>
            <i class="dot risk" />
            {t('arch.legend.risk', 'Risk')}
          </span>
          <span>
            <i class="dot impact" />
            {t('arch.legend.impact', 'Impact this scan')}
          </span>
          <span>
            <i class="dot added" />
            {t('arch.legend.added', 'Added this scan')}
          </span>
        </span>
      </div>

      {statusNote && <div class="banner note">{statusNote}</div>}
      {truncatedNote && <div class="banner note">{truncatedNote}</div>}
      {body.errors.map((e) => (
        <div class="banner error" key={e}>
          {e}
        </div>
      ))}
      {body.diff.state === 'no_baseline' && (
        <div class="dim small">
          {t('arch.diff_no_baseline', 'First scan (or legacy report); no architecture changes to compare yet.')}
        </div>
      )}
      {body.impact.truncated && (
        <div class="dim small">
          {t('arch.impact_truncated', 'Impact scope was truncated by the traversal limit.')}
        </div>
      )}

      <div class="arch-main">
        <GraphCanvas
          graph={graph}
          selectedId={selectedId}
          riskNodeIds={idx.nodeIds}
          riskEdgeIds={idx.edgeIds}
          impactNodeIds={impact}
          addedNodeIds={added}
          focusId={focusId}
          anchorId={anchorId}
          fitSignal={fitSignal}
          onSelect={setSelectedId}
          onToggleExpand={toggleExpand}
        />
        <div class="arch-side">
          {selected && (
            <div class="card detail">
              <b>{selected.label}</b>
              <dl>
                <dt>{t('arch.detail.type', 'Type')}</dt>
                <dd>{selected.kind}</dd>
                <dt>{t('arch.detail.loc', 'Lines')}</dt>
                <dd>{selected.loc}</dd>
                {selected.childCount > 0 && (
                  <>
                    <dt>{t('arch.detail.children', 'Submodules')}</dt>
                    <dd>
                      {t('arch.detail.children_count', '{count}', {
                        count: String(selected.childCount),
                      })}
                      (
                      <a
                        href="#"
                        onClick={(e) => {
                          e.preventDefault();
                          toggleExpand(selected.id);
                        }}
                      >
                        {selected.collapsed
                          ? t('arch.detail.expand', 'Expand')
                          : t('arch.detail.collapse', 'Collapse')}
                      </a>
                      )
                    </dd>
                  </>
                )}
              </dl>
            </div>
          )}
          <h3>{t('arch.risks_heading', 'Risks')}</h3>
          {body.risks.length === 0 ? (
            <div class="dim">{t('arch.risks_empty', 'No architecture risks found.')}</div>
          ) : (
            body.risks.map((r) => (
              <div
                key={r.finding.id}
                class={`risk-item ${activeRiskIds.has(r.finding.id) ? 'active' : ''}`}
              >
                <FindingRowView row={r.finding} analyze onActivate={() => focusRisk(r.node_ids)} />
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
