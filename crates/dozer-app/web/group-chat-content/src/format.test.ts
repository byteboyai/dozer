import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDuration } from './format.ts';

test('formatDuration', () => {
  assert.equal(formatDuration(null), '');
  assert.equal(formatDuration(400), '<1 秒');
  assert.equal(formatDuration(1500), '1.5 秒');
  assert.equal(formatDuration(12_000), '12 秒');
  assert.equal(formatDuration(75_000), '1 分 15 秒');
  assert.equal(formatDuration(120_000), '2 分');
});
