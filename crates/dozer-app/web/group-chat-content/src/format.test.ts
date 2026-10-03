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

import { runningPhase, formatElapsed } from './format.ts';

test('runningPhase advances connecting → thinking → long wait', () => {
  assert.equal(runningPhase(0), '正在连接模型');
  assert.equal(runningPhase(3999), '正在连接模型');
  assert.equal(runningPhase(4000), '思考中');
  assert.equal(runningPhase(30_000), '仍在思考,内容较多');
  assert.equal(runningPhase(90_000), '耗时较长,仍在等待模型返回');
});

test('formatElapsed', () => {
  assert.equal(formatElapsed(-5), '0 秒');
  assert.equal(formatElapsed(12_900), '12 秒');
  assert.equal(formatElapsed(65_000), '1 分 05 秒');
});
