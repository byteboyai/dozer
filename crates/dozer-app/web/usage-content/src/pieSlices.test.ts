import test from 'node:test';
import assert from 'node:assert/strict';
import { pieSlices, PIE_GAP_RAD } from './pieSlices.ts';

test('empty share list produces no slices', () => {
  assert.deepEqual(pieSlices([]), []);
});

test('all-zero share (total 0) produces no slices', () => {
  assert.deepEqual(pieSlices([{ agent: 'claude', value: 0 }]), []);
});

test('a single 100% slice starts at -PI/2 and sweeps nearly a full circle minus one gap', () => {
  const [s] = pieSlices([{ agent: 'claude', value: 10 }]);
  const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;
  assert.ok(near(s.startAngle, -Math.PI / 2 + PIE_GAP_RAD / 2));
  assert.ok(near(s.endAngle, -Math.PI / 2 + 2 * Math.PI - PIE_GAP_RAD / 2));
});

// 对应本计划 Review Focus:多切片累积误差不应产生可见缝隙(gap 之外的
// 累加误差)——全部切片角宽之和必须精确等于 2π - n*PIE_GAP_RAD。
test('slice angular widths sum to 2*PI minus one gap per slice, regardless of share count', () => {
  const share = [
    { agent: 'claude' as const, value: 7 },
    { agent: 'codebuddy' as const, value: 13 },
    { agent: 'opencode' as const, value: 5 },
    { agent: 'v8agent' as const, value: 41 },
  ];
  const slices = pieSlices(share);
  const totalWidth = slices.reduce((sum, s) => sum + (s.endAngle - s.startAngle), 0);
  const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;
  assert.ok(near(totalWidth, 2 * Math.PI - slices.length * PIE_GAP_RAD));
});

test('slices appear in the same order as the input share list', () => {
  const share = [
    { agent: 'claude' as const, value: 1 },
    { agent: 'codebuddy' as const, value: 1 },
  ];
  const slices = pieSlices(share);
  assert.deepEqual(
    slices.map((s) => s.agent),
    ['claude', 'codebuddy'],
  );
});
