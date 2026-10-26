import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDuration } from './format.ts';
import { setLocale } from './i18n.ts';

setLocale('en');

test('formatDuration', () => {
  assert.equal(formatDuration(null), '');
  assert.equal(formatDuration(400), '<1 s');
  assert.equal(formatDuration(1500), '1.5 s');
  assert.equal(formatDuration(12_000), '12 s');
  assert.equal(formatDuration(75_000), '1 min 15 s');
  assert.equal(formatDuration(120_000), '2 min');
});

import { runningPhase, formatElapsed } from './format.ts';

test('runningPhase advances connecting → thinking → long wait', () => {
  assert.equal(runningPhase(0), 'Connecting to the model');
  assert.equal(runningPhase(3999), 'Connecting to the model');
  assert.equal(runningPhase(4000), 'Thinking');
  assert.equal(runningPhase(30_000), 'Still thinking; the content is lengthy');
  assert.equal(runningPhase(90_000), 'Taking a while; still waiting for the model');
});

test('formatElapsed', () => {
  assert.equal(formatElapsed(-5), '0 s');
  assert.equal(formatElapsed(12_900), '12 s');
  assert.equal(formatElapsed(65_000), '1 min 05 s');
});
