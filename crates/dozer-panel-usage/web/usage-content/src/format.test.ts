import test from 'node:test';
import assert from 'node:assert/strict';
import { formatCount, niceTickStep, gridTicks } from './format.ts';

test('formatCount below 1000 returns the plain integer', () => {
  assert.equal(formatCount(0), '0');
  assert.equal(formatCount(999), '999');
});

test('formatCount uses k-suffix with one decimal in the [1000, 1e6) range', () => {
  assert.equal(formatCount(1000), '1.0k');
  assert.equal(formatCount(1234), '1.2k');
  assert.equal(formatCount(123_456), '123.5k');
  assert.equal(formatCount(999_999), '1000.0k');
});

test('formatCount uses m-suffix with one decimal at/above 1e6', () => {
  assert.equal(formatCount(1_000_000), '1.0m');
  assert.equal(formatCount(1_234_567), '1.2m');
});

// 与 chart.rs::tests::nice_tick_step_rounds_to_1_2_5_family 逐条对应。
test('niceTickStep rounds the raw step to the 1/2/5 family', () => {
  assert.equal(niceTickStep(7, 4), 2);
  assert.equal(niceTickStep(42, 4), 20);
  assert.equal(niceTickStep(1, 4), 1);
});

// 与 chart.rs::tests::grid_ticks_stops_at_max_and_never_empty 逐条对应。
test('gridTicks stops at max and never returns empty for a positive total', () => {
  assert.deepEqual(gridTicks(0), []);
  assert.deepEqual(gridTicks(7), [2, 4, 6]);
  assert.deepEqual(gridTicks(1), [1]);
});
