import dagre from 'dagre';
import type { VisibleGraph } from './projection.ts';

const NODE_W = 160;
const NODE_H = 36;

/** 用 dagre 做从左到右的分层布局,返回每个节点中心坐标。纯计算、不依赖 DOM;
 *  输入已按 id 升序(见 `projectGraph`),dagre 对相同输入给出相同输出,
 *  这是"重新扫描后布局不随机跳动"的保证。 */
export function computeLayout(graph: VisibleGraph): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>();
  if (graph.nodes.length === 0) return out;
  const g = new dagre.graphlib.Graph({ multigraph: false });
  g.setGraph({ rankdir: 'LR', nodesep: 24, ranksep: 70, marginx: 20, marginy: 20 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const n of graph.nodes) g.setNode(n.id, { width: NODE_W, height: NODE_H });
  for (const e of graph.edges) g.setEdge(e.from, e.to);
  dagre.layout(g);
  for (const n of graph.nodes) {
    const p = g.node(n.id);
    out.set(n.id, { x: p.x, y: p.y });
  }
  return out;
}

export const NODE_SIZE = { w: NODE_W, h: NODE_H };

type Positions = Map<string, { x: number; y: number }>;

/** 展开/折叠后整图被 dagre 重排;把整张新布局平移,使被点击的锚点节点留在原位
 *  (用户的方位感),其余节点相对锚点的位置不变。锚点不在新旧布局里时原样返回。
 *  不修改入参。 */
export function alignToAnchor(next: Positions, prev: Positions, anchorId: string | null): Positions {
  if (!anchorId) return next;
  const before = prev.get(anchorId);
  const after = next.get(anchorId);
  if (!before || !after) return next;
  const dx = before.x - after.x;
  const dy = before.y - after.y;
  const out: Positions = new Map();
  for (const [id, p] of next) out.set(id, { x: p.x + dx, y: p.y + dy });
  return out;
}
