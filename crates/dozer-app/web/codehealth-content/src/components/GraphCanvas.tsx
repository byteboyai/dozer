import { useEffect, useRef } from 'preact/hooks';
import cytoscape, { type Core, type ElementDefinition } from 'cytoscape';
import type { VisibleGraph } from '../graph/projection.ts';
import { alignToAnchor, computeLayout, NODE_SIZE } from '../graph/layout.ts';

export interface GraphCanvasProps {
  graph: VisibleGraph;
  selectedId: string | null;
  riskNodeIds: ReadonlySet<string>;
  riskEdgeIds: ReadonlySet<string>;
  impactNodeIds: ReadonlySet<string>;
  addedNodeIds: ReadonlySet<string>;
  /** 变化时把该节点居中(列表点击定位)。 */
  focusId: string | null;
  /** 值变化时适配窗口(「适配窗口 / 重置视图」按钮)。 */
  fitSignal: number;
  /** 刚被展开/折叠的节点:重排后它留在原位,其余节点平滑过渡。 */
  anchorId: string | null;
  onSelect(id: string | null): void;
  onToggleExpand(id: string): void;
}

function css(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function styleSheet(): cytoscape.StylesheetJson {
  return [
    {
      selector: 'node',
      style: {
        shape: 'round-rectangle',
        width: NODE_SIZE.w,
        height: NODE_SIZE.h,
        label: 'data(label)',
        'font-size': 11,
        'font-family': '-apple-system, "PingFang SC", sans-serif',
        color: css('--cream'),
        'text-valign': 'center',
        'text-halign': 'center',
        'text-wrap': 'ellipsis',
        'text-max-width': `${NODE_SIZE.w - 16}px`,
        'background-color': css('--card'),
        'border-width': 1,
        'border-color': css('--border'),
      },
    },
    { selector: 'node[?collapsed]', style: { 'border-style': 'double', 'border-width': 3 } },
    { selector: 'node.impact', style: { 'border-color': css('--cyan'), 'border-width': 2 } },
    { selector: 'node.added', style: { 'border-color': css('--green'), 'border-width': 2 } },
    { selector: 'node.risk', style: { 'border-color': css('--red'), 'border-width': 3 } },
    { selector: 'node:selected', style: { 'border-color': css('--gold'), 'border-width': 3 } },
    {
      selector: 'edge',
      style: {
        width: 1.2,
        'curve-style': 'bezier',
        'line-color': css('--dim'),
        'target-arrow-color': css('--dim'),
        'target-arrow-shape': 'triangle',
        'arrow-scale': 0.9,
      },
    },
    {
      selector: 'edge.risk',
      style: { 'line-color': css('--red'), 'target-arrow-color': css('--red'), width: 2.2 },
    },
  ] as cytoscape.StylesheetJson;
}

type Positions = Map<string, { x: number; y: number }>;

/** `start`:上一次的位置。幸存节点先放在旧位置,随后动画到 `pos`,避免展开时整图跳变。 */
function elements(p: GraphCanvasProps, pos: Positions, start: Positions): ElementDefinition[] {
  const nodes: ElementDefinition[] = p.graph.nodes.map((n) => ({
    group: 'nodes',
    data: {
      id: n.id,
      label: n.collapsed ? `${n.label} (+${n.childCount})` : n.label,
      collapsed: n.collapsed,
    },
    position: start.get(n.id) ?? pos.get(n.id)!,
    classes: [
      n.representedIds.some((id) => p.riskNodeIds.has(id)) ? 'risk' : '',
      n.representedIds.some((id) => p.impactNodeIds.has(id)) ? 'impact' : '',
      n.representedIds.some((id) => p.addedNodeIds.has(id)) ? 'added' : '',
    ]
      .filter(Boolean)
      .join(' '),
  }));
  const edges: ElementDefinition[] = p.graph.edges.map((e) => ({
    group: 'edges',
    data: { id: e.id, source: e.from, target: e.to },
    classes: e.edgeIds.some((id) => p.riskEdgeIds.has(id)) ? 'risk' : '',
  }));
  return [...nodes, ...edges];
}

export function GraphCanvas(props: GraphCanvasProps) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const handlers = useRef(props);
  handlers.current = props;
  const lastPos = useRef<Positions>(new Map());

  // 创建一次。
  useEffect(() => {
    if (!host.current) return;
    const core = cytoscape({
      container: host.current,
      elements: [],
      style: styleSheet(),
      layout: { name: 'preset' },
      wheelSensitivity: 0.3,
      minZoom: 0.1,
      maxZoom: 3,
      boxSelectionEnabled: false,
      autounselectify: false,
    });
    core.on('tap', 'node', (ev) => handlers.current.onSelect(ev.target.id()));
    core.on('tap', (ev) => {
      if (ev.target === core) handlers.current.onSelect(null);
    });
    core.on('dbltap', 'node', (ev) => handlers.current.onToggleExpand(ev.target.id()));
    cy.current = core;
    return () => {
      core.destroy();
      cy.current = null;
    };
  }, []);

  // 图或高亮集合变化 → 重建元素并 preset 布局(坐标来自 dagre,确定性)。
  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    const hadNodes = core.nodes().length > 0;
    const prev = lastPos.current;
    const next = alignToAnchor(computeLayout(props.graph), prev, props.anchorId);
    lastPos.current = next;
    core.batch(() => {
      core.elements().remove();
      core.add(elements(props, next, prev));
    });
    core.layout({ name: 'preset' }).run();
    core.nodes().forEach((n) => {
      const from = prev.get(n.id());
      const to = next.get(n.id());
      if (from && to && (from.x !== to.x || from.y !== to.y)) {
        n.animate({ position: to }, { duration: 200 });
      }
    });
    if (!hadNodes) core.fit(undefined, 24);
    if (props.selectedId) core.getElementById(props.selectedId).select();
  }, [
    props.graph,
    props.riskNodeIds,
    props.riskEdgeIds,
    props.impactNodeIds,
    props.addedNodeIds,
  ]);

  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    core.elements().unselect();
    if (props.selectedId) core.getElementById(props.selectedId).select();
  }, [props.selectedId]);

  useEffect(() => {
    const core = cy.current;
    if (!core || !props.focusId) return;
    const n = core.getElementById(props.focusId);
    if (n.nonempty())
      core.animate(
        { center: { eles: n }, zoom: Math.max(core.zoom(), 1) },
        { duration: 200 },
      );
  }, [props.focusId]);

  useEffect(() => {
    cy.current?.fit(undefined, 24);
  }, [props.fitSignal]);

  return <div class="graph-canvas" ref={host} />;
}
