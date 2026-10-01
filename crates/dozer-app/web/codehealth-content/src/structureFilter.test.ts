import test from 'node:test';
import assert from 'node:assert/strict';
import { filterStructure } from './structureFilter.ts';
import { row } from './fixtures.ts';

const rows = [
  row({ id: 'a', change: 'new' }),
  row({ id: 'b', change: 'worsened' }),
  row({ id: 'c', change: 'improved' }),
  row({ id: 'd', change: 'persisting' }),
  row({ id: 'e', change: null }),
];

// 与旧 view.rs 的 `is_new` 语义逐条对应:新增或恶化算"本轮新增",无差异数据不算。
test('new mode keeps only new and worsened', () => {
  assert.deepEqual(filterStructure(rows, 'new').map((r) => r.id), ['a', 'b']);
});

test('all mode keeps everything in order', () => {
  assert.deepEqual(filterStructure(rows, 'all').map((r) => r.id), ['a', 'b', 'c', 'd', 'e']);
});
