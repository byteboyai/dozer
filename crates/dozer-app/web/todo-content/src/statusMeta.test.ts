import test from 'node:test';
import assert from 'node:assert/strict';
import { STATUS_META, STATUS_FILTER_OPTIONS } from './statusMeta.ts';

test('labels and colors match the native implementation', () => {
  assert.deepEqual(STATUS_META.pending, { label: '待办', color: 'var(--cyan)' });
  assert.deepEqual(STATUS_META.in_progress, { label: '进行中', color: 'var(--gold)' });
  assert.deepEqual(STATUS_META.suspended, { label: '搁置', color: 'var(--cream)' });
  assert.deepEqual(STATUS_META.done, { label: '已完成', color: 'var(--dim)' });
});

test('status filter offers all plus the four states in order', () => {
  assert.deepEqual(STATUS_FILTER_OPTIONS, ['all', 'pending', 'in_progress', 'suspended', 'done']);
});
