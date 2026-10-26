import type { ToolCall, ToolResult } from './types.ts';

// 每个 call 的展示单元：这条调用本身(可能没有，见"未配对结果"那一组) +
// 配对上的结果列表(可能 0/1/多个)。
export interface Pairing {
  call?: ToolCall;
  results: ToolResult[];
}

// 下标近似(现状，2026-09-25 之前唯一实现):call[i]/result[i] 相邻渲染。
export function pairByIndex(toolCalls: ToolCall[], toolResults: ToolResult[]): Pairing[] {
  const pairCount = Math.max(toolCalls.length, toolResults.length);
  return Array.from({ length: pairCount }, (_, i) => ({
    call: toolCalls[i],
    results: toolResults[i] ? [toolResults[i]] : [],
  }));
}

// 精确配对：按 call.id 把 result 分组挂到对应的 call 后面，一个 id 对应
// 多个 result 时按遇到顺序堆叠;call_id 对不上任何 call(或没有 call_id)的
// result 单独追加一组，放在最后，不能丢。
export function pairById(toolCalls: ToolCall[], toolResults: ToolResult[]): Pairing[] {
  const callIds = new Set(toolCalls.map((c) => c.id as string));
  const resultsByCallId = new Map<string, ToolResult[]>();
  const unmatched: ToolResult[] = [];
  for (const result of toolResults) {
    if (result.call_id && callIds.has(result.call_id)) {
      const list = resultsByCallId.get(result.call_id) ?? [];
      list.push(result);
      resultsByCallId.set(result.call_id, list);
    } else {
      unmatched.push(result);
    }
  }
  // id 理论上应唯一;若上游数据异常出现重复 id，只让第一个同 id 的
  // call 领取结果，后续重名 call 给空列表——避免同一批结果被渲染多次。
  const consumed = new Set<string>();
  const pairs: Pairing[] = toolCalls.map((call) => {
    const id = call.id as string;
    if (consumed.has(id)) {
      return { call, results: [] };
    }
    consumed.add(id);
    return { call, results: resultsByCallId.get(id) ?? [] };
  });
  if (unmatched.length > 0) {
    pairs.push({ results: unmatched });
  }
  return pairs;
}

// 只有全部 tool_calls 都带 id 才进精确配对，只要缺一个就整体回落下标
// 近似——不做"部分 id、部分下标"的混合模式，行为不可预期。见
// docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md。
export function pairCallsAndResults(
  toolCalls: ToolCall[],
  toolResults: ToolResult[],
): Pairing[] {
  const allCallsHaveId = toolCalls.length > 0 && toolCalls.every((c) => !!c.id);
  return allCallsHaveId ? pairById(toolCalls, toolResults) : pairByIndex(toolCalls, toolResults);
}
