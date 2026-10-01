import type { ArchitectureBody, ArchRisk } from '../types.ts';

export function riskIndex(risks: ArchRisk[]): { nodeIds: Set<string>; edgeIds: Set<string> } {
  const nodeIds = new Set<string>();
  const edgeIds = new Set<string>();
  for (const r of risks) {
    r.node_ids.forEach((id) => nodeIds.add(id));
    r.edge_ids.forEach((id) => edgeIds.add(id));
  }
  return { nodeIds, edgeIds };
}

/** 点画布节点 → 命中的风险(节点可能代表被折叠的后代)。 */
export function riskForNode(risks: ArchRisk[], representedIds: string[]): ArchRisk[] {
  const set = new Set(representedIds);
  return risks.filter((r) => r.node_ids.some((id) => set.has(id)));
}

export function impactNodeIds(impact: ArchitectureBody['impact']): Set<string> {
  return new Set([...impact.direct, ...impact.indirect].map((n) => n.node_id));
}

/** 只有真正有基线可比较时才返回新增节点;`no_baseline`/`unavailable`
 *  一律空集,不得把现存节点渲染成"本轮新增"。 */
export function addedNodeIds(diff: ArchitectureBody['diff']): Set<string> {
  return diff.state === 'compared' ? new Set(diff.added_nodes) : new Set();
}
