import type { ArchEdge, ArchNode } from '../types.ts';

export interface ProjectionInput {
  nodes: ArchNode[];
  edges: ArchEdge[];
  layer: 'crate' | 'module';
  /** 已展开的模块 id。根模块(父为 crate)恒可见,其子级可见当且仅当父在此集合内。 */
  expanded: ReadonlySet<string>;
  riskOnly: boolean;
  riskNodeIds: ReadonlySet<string>;
  riskEdgeIds: ReadonlySet<string>;
}

export interface VNode {
  id: string;
  label: string;
  kind: string;
  loc: number;
  childCount: number;
  collapsed: boolean;
  /** 本可见节点所代表的原始节点 id(自身 + 被折叠隐藏的后代),升序。 */
  representedIds: string[];
}

export interface VEdge {
  id: string;
  from: string;
  to: string;
  count: number;
  edgeIds: string[];
}

export interface VisibleGraph {
  nodes: VNode[];
  edges: VEdge[];
}

const byId = <T extends { id: string }>(a: T, b: T) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

export function projectGraph(input: ProjectionInput): VisibleGraph {
  const g = input.layer === 'crate' ? projectCrateLayer(input) : projectModuleLayer(input);
  return input.riskOnly ? filterRisk(g, input) : g;
}

function projectCrateLayer(input: ProjectionInput): VisibleGraph {
  const crates = input.nodes.filter((n) => n.kind === 'crate').sort(byId);
  const ids = new Set(crates.map((c) => c.id));
  // 每个节点归属的 crate(沿 parent_id 链向上找到 crate 为止)。模块级的风险/影响
  // 范围要能在 crate 层显示,所以 crate 节点代表它名下的全部后代模块。
  const parentOf = new Map(input.nodes.map((n) => [n.id, n.parent_id]));
  const owner = (id: string): string | null => {
    const seen = new Set<string>();
    let cur: string | null | undefined = id;
    while (cur && !seen.has(cur)) {
      if (ids.has(cur)) return cur;
      seen.add(cur);
      cur = parentOf.get(cur);
    }
    return null;
  };
  const represented = new Map<string, string[]>(crates.map((c) => [c.id, [c.id]]));
  for (const n of input.nodes) {
    if (n.kind === 'crate') continue;
    const o = owner(n.id);
    if (o) represented.get(o)!.push(n.id);
  }
  const nodes: VNode[] = crates.map((c) => ({
    id: c.id,
    label: c.name,
    kind: c.kind,
    loc: c.loc,
    childCount: 0,
    collapsed: false,
    representedIds: represented.get(c.id)!.slice().sort(),
  }));
  const edges: VEdge[] = input.edges
    .filter((e) => e.kind === 'cargo_dependency' && ids.has(e.from) && ids.has(e.to))
    .map((e) => ({
      id: `${e.from}->${e.to}`,
      from: e.from,
      to: e.to,
      count: 1,
      edgeIds: [e.id],
    }))
    .sort(byId);
  return { nodes, edges };
}

function projectModuleLayer(input: ProjectionInput): VisibleGraph {
  const modules = input.nodes.filter((n) => n.kind === 'module');
  const modById = new Map(modules.map((m) => [m.id, m]));
  const children = new Map<string, string[]>();
  for (const m of modules) {
    if (m.parent_id && modById.has(m.parent_id)) {
      const list = children.get(m.parent_id) ?? [];
      list.push(m.id);
      children.set(m.parent_id, list);
    }
  }

  // 递归决定可见集合与"代表映射"(每个原始模块 → 最近的可见祖先或自身)。
  const rep = new Map<string, string>();
  const visible = new Set<string>();
  const roots = modules.filter((m) => !m.parent_id || !modById.has(m.parent_id));
  const walk = (id: string, visibleAncestor: string | null) => {
    const self = visibleAncestor ?? id;
    rep.set(id, self);
    if (!visibleAncestor) visible.add(id);
    const open = !visibleAncestor && input.expanded.has(id);
    for (const c of children.get(id) ?? []) {
      // 当前节点可见且已展开 → 子节点自己可见;否则子节点被折叠进当前可见节点。
      walk(c, open ? null : self);
    }
  };
  for (const r of roots) walk(r.id, null);

  const represented = new Map<string, string[]>();
  for (const [orig, v] of rep) {
    const list = represented.get(v) ?? [];
    list.push(orig);
    represented.set(v, list);
  }

  const nodes: VNode[] = [...visible]
    .map((id) => modById.get(id)!)
    .sort(byId)
    .map((m) => {
      const kids = children.get(m.id) ?? [];
      return {
        id: m.id,
        label: m.qualified_name,
        kind: m.kind,
        loc: m.loc,
        childCount: kids.length,
        collapsed: kids.length > 0 && !input.expanded.has(m.id),
        representedIds: (represented.get(m.id) ?? [m.id]).slice().sort(),
      };
    });

  const merged = new Map<string, VEdge>();
  for (const e of input.edges) {
    if (e.kind !== 'module_use') continue;
    const from = rep.get(e.from);
    const to = rep.get(e.to);
    if (!from || !to || from === to) continue;
    const key = `${from}->${to}`;
    const cur = merged.get(key);
    if (cur) {
      cur.count += 1;
      if (!cur.edgeIds.includes(e.id)) cur.edgeIds.push(e.id);
    } else {
      merged.set(key, { id: key, from, to, count: 1, edgeIds: [e.id] });
    }
  }
  const edges = [...merged.values()]
    .map((v) => ({ ...v, edgeIds: v.edgeIds.slice().sort() }))
    .sort(byId);
  return { nodes, edges };
}

function filterRisk(g: VisibleGraph, input: ProjectionInput): VisibleGraph {
  const keep = new Set(
    g.nodes
      .filter((n) => n.representedIds.some((id) => input.riskNodeIds.has(id)))
      .map((n) => n.id),
  );
  const nodes = g.nodes.filter((n) => keep.has(n.id));
  const edges = g.edges.filter((e) => keep.has(e.from) && keep.has(e.to));
  return { nodes, edges };
}
