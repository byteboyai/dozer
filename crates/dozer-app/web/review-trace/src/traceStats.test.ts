import test from 'node:test';
import assert from 'node:assert/strict';
import { formatTokenCount, buildTraceStatsLabel } from './traceStats.ts';

test('formatTokenCount below 1000 returns the plain number', () => {
  assert.equal(formatTokenCount(0), '0');
  assert.equal(formatTokenCount(999), '999');
});

test('formatTokenCount at/above 1000 uses a k-suffix and drops a trailing .0', () => {
  assert.equal(formatTokenCount(1000), '1k');
  assert.equal(formatTokenCount(1500), '1.5k');
  assert.equal(formatTokenCount(2000), '2k');
  assert.equal(formatTokenCount(12345), '12.3k');
});

test('buildTraceStatsLabel is empty with no tool calls and no tokens', () => {
  assert.equal(buildTraceStatsLabel({}), '');
});

test('buildTraceStatsLabel reports tool call count without errors', () => {
  assert.equal(
    buildTraceStatsLabel({ tool_calls: [{ summary: 'a' }, { summary: 'b' }] }),
    '2 次工具调用',
  );
});

test('buildTraceStatsLabel reports failed tool result count in parentheses', () => {
  assert.equal(
    buildTraceStatsLabel({
      tool_calls: [{ summary: 'a' }],
      tool_results: [{ content: 'x', is_error: true }],
    }),
    '1 次工具调用(1 失败)',
  );
});

test('buildTraceStatsLabel joins tool-call count and token total with a middle dot', () => {
  assert.equal(
    buildTraceStatsLabel({
      tool_calls: [{ summary: 'a' }],
      tokens_in: 500,
      tokens_out: 700,
    }),
    '1 次工具调用 · 1.2k tokens',
  );
});

test('buildTraceStatsLabel reports tokens alone when there are no tool calls', () => {
  assert.equal(buildTraceStatsLabel({ tokens_cache_read: 300 }), '300 tokens');
});
