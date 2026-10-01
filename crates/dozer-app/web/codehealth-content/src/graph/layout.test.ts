import test from 'node:test';
import assert from 'node:assert/strict';
import { computeLayout } from './layout.ts';
import type { VisibleGraph } from './projection.ts';

const vn = (id: string) => ({
  id,
  label: id,
  kind: 'module',
  loc: 10,
  childCount: 0,
  collapsed: false,
  representedIds: [id],
});
const graph: VisibleGraph = {
  nodes: ['a', 'b', 'c', 'd'].map(vn),
  edges: [
    { id: 'a->b', from: 'a', to: 'b', count: 1, edgeIds: ['e1'] },
    { id: 'a->c', from: 'a', to: 'c', count: 1, edgeIds: ['e2'] },
    { id: 'b->d', from: 'b', to: 'd', count: 1, edgeIds: ['e3'] },
    { id: 'c->d', from: 'c', to: 'd', count: 1, edgeIds: ['e4'] },
  ],
};

test('every node gets finite coordinates', () => {
  const pos = computeLayout(graph);
  assert.equal(pos.size, 4);
  for (const { x, y } of pos.values()) {
    assert.ok(Number.isFinite(x) && Number.isFinite(y));
  }
});

// 原 spec 验收项:同一份图连续两次布局节点坐标逐点相同。
test('layout is deterministic across runs', () => {
  const a = [...computeLayout(graph).entries()];
  const b = [...computeLayout(graph).entries()];
  assert.deepEqual(a, b);
});

test('left-to-right: a source is left of its dependents', () => {
  const pos = computeLayout(graph);
  assert.ok(pos.get('a')!.x < pos.get('b')!.x);
  assert.ok(pos.get('b')!.x < pos.get('d')!.x);
});

test('cycles do not throw and still place every node', () => {
  const cyc: VisibleGraph = {
    nodes: ['x', 'y'].map(vn),
    edges: [
      { id: 'x->y', from: 'x', to: 'y', count: 1, edgeIds: ['1'] },
      { id: 'y->x', from: 'y', to: 'x', count: 1, edgeIds: ['2'] },
    ],
  };
  assert.equal(computeLayout(cyc).size, 2);
});

test('empty graph yields empty layout', () => {
  assert.equal(computeLayout({ nodes: [], edges: [] }).size, 0);
});

import { alignToAnchor } from './layout.ts';

type Pos = Map<string, { x: number; y: number }>;
const P = (o: Record<string, [number, number]>): Pos =>
  new Map(Object.entries(o).map(([k, [x, y]]) => [k, { x, y }]));

// 展开/折叠后整图被 dagre 重排,但用户点的那个节点要留在原位,否则会失去方位感。
test('alignToAnchor keeps the anchor where it was and shifts everything by the same delta', () => {
  const prev = P({ a: [100, 50], b: [300, 50] });
  const next = P({ a: [20, 20], b: [220, 20], c: [220, 80] });
  const out = alignToAnchor(next, prev, 'a');
  assert.deepEqual(out.get('a'), { x: 100, y: 50 });
  assert.deepEqual(out.get('b'), { x: 300, y: 50 });
  assert.deepEqual(out.get('c'), { x: 300, y: 110 });
});

test('alignToAnchor is a no-op without an anchor or when the anchor is new', () => {
  const next = P({ a: [20, 20] });
  assert.deepEqual(alignToAnchor(next, P({ z: [1, 1] }), null), next);
  assert.deepEqual(alignToAnchor(next, P({ z: [1, 1] }), 'a'), next);
  assert.deepEqual(alignToAnchor(next, P({ z: [1, 1] }), 'missing'), next);
});

test('alignToAnchor does not mutate its inputs', () => {
  const prev = P({ a: [100, 50] });
  const next = P({ a: [20, 20] });
  alignToAnchor(next, prev, 'a');
  assert.deepEqual(next.get('a'), { x: 20, y: 20 });
});
