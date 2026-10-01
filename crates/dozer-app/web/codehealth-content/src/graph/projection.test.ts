import test from 'node:test';
import assert from 'node:assert/strict';
import { projectGraph, type ProjectionInput } from './projection.ts';
import type { ArchNode, ArchEdge } from '../types.ts';

const n = (id: string, kind: ArchNode['kind'], parent: string | null, loc = 10): ArchNode => ({
  id,
  kind,
  name: id,
  qualified_name: id.replace(/^(module|crate):/, ''),
  path: null,
  parent_id: parent,
  loc,
  fan_in: 0,
  fan_out: 0,
  layer: null,
});
const e = (kind: ArchEdge['kind'], from: string, to: string): ArchEdge => ({
  id: `edge:${kind}:${from}:${to}`,
  from,
  to,
  kind,
  evidence_count: 1,
});

const nodes: ArchNode[] = [
  n('crate:a', 'crate', null),
  n('crate:b', 'crate', null),
  n('module:a', 'module', 'crate:a'),
  n('module:a::x', 'module', 'module:a'),
  n('module:a::x::deep', 'module', 'module:a::x'),
  n('module:a::y', 'module', 'module:a'),
  n('module:b', 'module', 'crate:b'),
];
const edges: ArchEdge[] = [
  e('cargo_dependency', 'crate:a', 'crate:b'),
  e('module_use', 'module:a::x', 'module:a::y'),
  e('module_use', 'module:a::x::deep', 'module:a::y'),
  e('module_use', 'module:a::y', 'module:b'),
];
const base: ProjectionInput = {
  nodes,
  edges,
  layer: 'crate',
  expanded: new Set(),
  riskOnly: false,
  riskNodeIds: new Set(),
  riskEdgeIds: new Set(),
};

test('crate layer shows crates and cargo edges only', () => {
  const g = projectGraph(base);
  assert.deepEqual(
    g.nodes.map((v) => v.id),
    ['crate:a', 'crate:b'],
  );
  assert.deepEqual(
    g.edges.map((v) => [v.from, v.to]),
    [['crate:a', 'crate:b']],
  );
});

test('module layer collapsed shows only root modules, with child counts', () => {
  const g = projectGraph({ ...base, layer: 'module' });
  assert.deepEqual(
    g.nodes.map((v) => v.id),
    ['module:a', 'module:b'],
  );
  const a = g.nodes.find((v) => v.id === 'module:a')!;
  assert.equal(a.collapsed, true);
  assert.equal(a.childCount, 2);
  const b = g.nodes.find((v) => v.id === 'module:b')!;
  assert.equal(b.collapsed, false);
  assert.equal(b.childCount, 0);
  // a 的内部边(x→y)映射后两端相同,被丢弃;y→b 映射成 a→b。
  assert.deepEqual(
    g.edges.map((v) => [v.from, v.to]),
    [['module:a', 'module:b']],
  );
});

test('expanding a module reveals its children and routes edges to them', () => {
  const g = projectGraph({ ...base, layer: 'module', expanded: new Set(['module:a']) });
  assert.deepEqual(
    g.nodes.map((v) => v.id),
    ['module:a', 'module:a::x', 'module:a::y', 'module:b'].sort(),
  );
  const x = g.nodes.find((v) => v.id === 'module:a::x')!;
  assert.equal(x.collapsed, true, 'x 有子模块 deep 但未展开');
  const pairs = g.edges.map((v) => `${v.from}->${v.to}`).sort();
  // deep 折叠进 x:x→y 有两条(x→y 与 deep→y)合并为 count=2。
  assert.deepEqual(pairs, ['module:a::x->module:a::y', 'module:a::y->module:b']);
  const xy = g.edges.find((v) => v.from === 'module:a::x' && v.to === 'module:a::y')!;
  assert.equal(xy.count, 2);
  assert.equal(xy.edgeIds.length, 2);
});

test('collapsed node represents its hidden descendants', () => {
  const g = projectGraph({ ...base, layer: 'module', expanded: new Set(['module:a']) });
  const x = g.nodes.find((v) => v.id === 'module:a::x')!;
  assert.deepEqual([...x.representedIds].sort(), ['module:a::x', 'module:a::x::deep']);
});

test('riskOnly keeps risk nodes (via represented ids) and edges between kept nodes', () => {
  const g = projectGraph({
    ...base,
    layer: 'module',
    expanded: new Set(['module:a']),
    riskOnly: true,
    riskNodeIds: new Set(['module:a::x::deep', 'module:a::y']),
  });
  // x 代表 deep(风险)→保留;y 风险→保留;a、b 不保留。
  assert.deepEqual(
    g.nodes.map((v) => v.id),
    ['module:a::x', 'module:a::y'],
  );
  assert.deepEqual(
    g.edges.map((v) => [v.from, v.to]),
    [['module:a::x', 'module:a::y']],
  );
});

// 布局稳定性的前提:输入顺序无关,输出按 id 升序。
test('output is independent of input order', () => {
  const g1 = projectGraph({ ...base, layer: 'module', expanded: new Set(['module:a']) });
  const g2 = projectGraph({
    ...base,
    layer: 'module',
    expanded: new Set(['module:a']),
    nodes: [...nodes].reverse(),
    edges: [...edges].reverse(),
  });
  assert.deepEqual(g1, g2);
});

test('crate layer ignores module nodes even when expanded set is non-empty', () => {
  const g = projectGraph({ ...base, expanded: new Set(['module:a']) });
  assert.deepEqual(
    g.nodes.map((v) => v.id),
    ['crate:a', 'crate:b'],
  );
});

// 修复:crate 层必须把该 crate 的所有后代模块并入 representedIds,否则模块级的
// 风险/影响范围在默认的 crate 层看不见,"仅看风险"得到空图。
test('crate layer represents all descendant modules', () => {
  const g = projectGraph(base);
  const a = g.nodes.find((v) => v.id === 'crate:a')!;
  assert.deepEqual(
    [...a.representedIds].sort(),
    ['crate:a', 'module:a', 'module:a::x', 'module:a::x::deep', 'module:a::y'],
  );
  const b = g.nodes.find((v) => v.id === 'crate:b')!;
  assert.deepEqual([...b.representedIds].sort(), ['crate:b', 'module:b']);
});

test('crate layer + riskOnly keeps crates owning a risky module', () => {
  const g = projectGraph({
    ...base,
    riskOnly: true,
    riskNodeIds: new Set(['module:a::x::deep']),
  });
  assert.deepEqual(g.nodes.map((v) => v.id), ['crate:a']);
});
