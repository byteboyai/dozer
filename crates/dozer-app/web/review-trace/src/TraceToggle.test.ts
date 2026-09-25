import test from 'node:test';
import assert from 'node:assert/strict';
import { pairCallsAndResults } from './pairing.ts';
import type { ToolCall, ToolResult } from './types.ts';

// 把配对结果摊平成 [call 或 result] 顺序列表，断言渲染顺序——等价于
// 组件里 `.trace-timeline` 下的实际 `trace-item` 序列。
function sequence(calls: ToolCall[], results: ToolResult[]): string[] {
  const out: string[] = [];
  for (const pair of pairCallsAndResults(calls, results)) {
    if (pair.call) out.push(pair.call.summary);
    for (const r of pair.results) out.push(r.content);
  }
  return out;
}

test('all calls have id: pairs each call with its matching result, in call order', () => {
  const calls: ToolCall[] = [
    { summary: 'call A', id: 'a' },
    { summary: 'call B', id: 'b' },
  ];
  const results: ToolResult[] = [
    { content: 'result B', is_error: false, call_id: 'b' },
    { content: 'result A', is_error: false, call_id: 'a' },
  ];
  assert.deepEqual(sequence(calls, results), ['call A', 'result A', 'call B', 'result B']);
});

test('one call id maps to multiple results: stacked in encounter order after that call', () => {
  const calls: ToolCall[] = [{ summary: 'call A', id: 'a' }];
  const results: ToolResult[] = [
    { content: 'first', is_error: false, call_id: 'a' },
    { content: 'second', is_error: false, call_id: 'a' },
  ];
  assert.deepEqual(sequence(calls, results), ['call A', 'first', 'second']);
});

test('result with no matching call_id is appended at the end, not dropped', () => {
  const calls: ToolCall[] = [{ summary: 'call A', id: 'a' }];
  const results: ToolResult[] = [
    { content: 'orphan', is_error: false, call_id: 'unknown' },
    { content: 'matched', is_error: false, call_id: 'a' },
  ];
  assert.deepEqual(sequence(calls, results), ['call A', 'matched', 'orphan']);
});

test('any call missing id falls back to index pairing for the whole turn', () => {
  const calls: ToolCall[] = [{ summary: 'call A', id: 'a' }, { summary: 'call B' }];
  const results: ToolResult[] = [
    { content: 'result 1', is_error: false, call_id: 'a' },
    { content: 'result 2', is_error: false },
  ];
  assert.deepEqual(sequence(calls, results), ['call A', 'result 1', 'call B', 'result 2']);
});

test('no ids anywhere: behaves exactly like the existing index-based pairing', () => {
  const calls: ToolCall[] = [{ summary: 'call A' }, { summary: 'call B' }];
  const results: ToolResult[] = [
    { content: 'result A', is_error: false },
    { content: 'result B', is_error: false },
  ];
  assert.deepEqual(sequence(calls, results), ['call A', 'result A', 'call B', 'result B']);
});
