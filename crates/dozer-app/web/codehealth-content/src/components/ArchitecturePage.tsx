import { useMemo, useState } from 'preact/hooks';
import { useViewState } from '../useViewState.ts';
import type { ArchitectureBody } from '../types.ts';
import { projectGraph } from '../graph/projection.ts';
import { addedNodeIds, impactNodeIds, riskForNode, riskIndex } from '../graph/linking.ts';
import { GraphCanvas } from './GraphCanvas.tsx';
import { FindingRowView } from './FindingRow.tsx';

export function ArchitecturePage({ body }: { body: ArchitectureBody }) {
  const [layer, setLayer] = useViewState<'crate' | 'module'>('arch.layer', 'crate');
  const [riskOnly, setRiskOnly] = useViewState('arch.riskOnly', false);
  const [expanded, setExpanded] = useViewState<ReadonlySet<string>>('arch.expanded', new Set());
  const [selectedId, setSelectedId] = useViewState<string | null>('arch.selected', null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [fitSignal, setFitSignal] = useState(0);

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

  if (body.status === 'not_applicable') {
    return (
      <div>
        <h2>架构地图</h2>
        <div class="banner note">{body.status_note}</div>
      </div>
    );
  }

  const toggleExpand = (id: string) => {
    const node = graph.nodes.find((n) => n.id === id);
    if (!node || node.childCount === 0) return;
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
        <h2>架构地图</h2>
        <div class="seg">
          <button class={layer === 'crate' ? 'active' : ''} onClick={() => { setLayer('crate'); setFitSignal((n) => n + 1); }}>
            crate
          </button>
          <button class={layer === 'module' ? 'active' : ''} onClick={() => { setLayer('module'); setFitSignal((n) => n + 1); }}>
            module
          </button>
        </div>
        <label class="switch">
          <input
            type="checkbox"
            checked={riskOnly}
            onChange={(e) => {
              setRiskOnly((e.currentTarget as HTMLInputElement).checked);
              setFitSignal((n) => n + 1);
            }}
          />
          仅看风险
        </label>
        <button class="btn small" onClick={() => setFitSignal((n) => n + 1)}>
          适配窗口
        </button>
        <span class="legend">
          <span>
            <i class="dot risk" />风险
          </span>
          <span>
            <i class="dot impact" />本轮影响范围
          </span>
          <span>
            <i class="dot added" />本轮新增
          </span>
        </span>
      </div>

      {body.status_note && <div class="banner note">{body.status_note}</div>}
      {body.truncated_note && <div class="banner note">{body.truncated_note}</div>}
      {body.errors.map((e) => (
        <div class="banner error" key={e}>
          {e}
        </div>
      ))}
      {body.diff.state === 'no_baseline' && (
        <div class="dim small">首次扫描(或旧报告),暂无架构变化可比对。</div>
      )}
      {body.impact.truncated && <div class="dim small">影响范围因访问上限被截断。</div>}

      <div class="arch-main">
        <GraphCanvas
          graph={graph}
          selectedId={selectedId}
          riskNodeIds={idx.nodeIds}
          riskEdgeIds={idx.edgeIds}
          impactNodeIds={impact}
          addedNodeIds={added}
          focusId={focusId}
          fitSignal={fitSignal}
          onSelect={setSelectedId}
          onToggleExpand={toggleExpand}
        />
        <div class="arch-side">
          {selected && (
            <div class="card detail">
              <b>{selected.label}</b>
              <dl>
                <dt>类型</dt>
                <dd>{selected.kind}</dd>
                <dt>代码行</dt>
                <dd>{selected.loc}</dd>
                {selected.childCount > 0 && (
                  <>
                    <dt>子模块</dt>
                    <dd>
                      {selected.childCount} 个(
                      <a
                        href="#"
                        onClick={(e) => {
                          e.preventDefault();
                          toggleExpand(selected.id);
                        }}
                      >
                        {selected.collapsed ? '展开' : '折叠'}
                      </a>
                      )
                    </dd>
                  </>
                )}
              </dl>
            </div>
          )}
          <h3>风险</h3>
          {body.risks.length === 0 ? (
            <div class="dim">没有发现架构风险。</div>
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
