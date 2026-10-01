import test from 'node:test';
import assert from 'node:assert/strict';
import { riskIndex, riskForNode, impactNodeIds, addedNodeIds } from './linking.ts';
import type { ArchRisk, ArchitectureBody } from '../types.ts';
import { row } from '../fixtures.ts';

const risks: ArchRisk[] = [
  { kind: 'cycle', finding: row({ id: 'r1' }), node_ids: ['m:x', 'm:y'], edge_ids: ['e1', 'e2'] },
  { kind: 'hub', finding: row({ id: 'r2' }), node_ids: ['m:x'], edge_ids: [] },
];

test('riskIndex unions node and edge ids', () => {
  const idx = riskIndex(risks);
  assert.deepEqual([...idx.nodeIds].sort(), ['m:x', 'm:y']);
  assert.deepEqual([...idx.edgeIds].sort(), ['e1', 'e2']);
});

test('riskForNode matches by any represented id', () => {
  assert.deepEqual(
    riskForNode(risks, ['m:y']).map((r) => r.finding.id),
    ['r1'],
  );
  assert.deepEqual(
    riskForNode(risks, ['m:x']).map((r) => r.finding.id),
    ['r1', 'r2'],
  );
  assert.deepEqual(riskForNode(risks, ['m:z']), []);
});

test('impactNodeIds merges direct and indirect', () => {
  const s = impactNodeIds({
    direct: [{ node_id: 'a', distance: 1 }],
    indirect: [{ node_id: 'b', distance: 2 }],
    truncated: false,
  });
  assert.deepEqual([...s].sort(), ['a', 'b']);
});

type Diff = ArchitectureBody['diff'];
const diff = (state: Diff['state']): Diff => ({
  state,
  added_nodes: ['n1'],
  removed_nodes: [],
  added_edges: [],
  removed_edges: [],
  added_cycles: [],
  resolved_cycles: [],
});

// Review Focus 4:没有基线/不可用时不得把现存节点标成"本轮新增"。
test('added nodes only exist when there is a real baseline', () => {
  assert.deepEqual([...addedNodeIds(diff('compared'))], ['n1']);
  assert.equal(addedNodeIds(diff('no_baseline')).size, 0);
  assert.equal(addedNodeIds(diff('unavailable')).size, 0);
});
