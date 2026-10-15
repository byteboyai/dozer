import test from 'node:test';
import assert from 'node:assert/strict';
import { splitSegments, displayNumbers, formatNumber } from './segments.ts';
import type { TodoCard } from './types.ts';

const c = (id: number, segment: TodoCard['segment']): TodoCard => ({
  id, text: `t${id}`, state: segment === 'done' ? 'done' : segment === 'paused' ? 'suspended' : 'pending',
  segment, plan_date: null, completed_label: null, category_id: null, category_name: null,
  assigned_agent: null, has_dispatch: false,
});

test('splitSegments groups by segment and keeps relative order', () => {
  const s = splitSegments([c(1, 'done'), c(2, 'active'), c(3, 'paused'), c(4, 'active'), c(5, 'done')]);
  assert.deepEqual(s.active.map((i) => i.id), [2, 4]);
  assert.deepEqual(s.paused.map((i) => i.id), [3]);
  assert.deepEqual(s.done.map((i) => i.id), [1, 5]);
});

test('numbers run continuously active -> paused -> done, 1-based', () => {
  const s = splitSegments([c(1, 'done'), c(2, 'active'), c(3, 'paused'), c(4, 'active')]);
  const n = displayNumbers(s);
  assert.equal(n.get(2), 1);
  assert.equal(n.get(4), 2);
  assert.equal(n.get(3), 3);
  assert.equal(n.get(1), 4);
});

test('formatNumber pads to three digits', () => {
  assert.equal(formatNumber(1), '#001');
  assert.equal(formatNumber(42), '#042');
  assert.equal(formatNumber(1234), '#1234');
});
