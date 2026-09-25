import { Fragment } from 'preact';
import type { AiTurnData } from '../types.ts';
import { buildTraceStatsLabel } from '../traceStats.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolCallRow } from './ToolCallRow.tsx';
import { ToolResultRow } from './ToolResultRow.tsx';

export function TraceToggle({ v }: { v: AiTurnData }) {
  const hasThinking = !!v.thinking_text;
  const toolCalls = v.tool_calls ?? [];
  const toolResults = v.tool_results ?? [];
  const hasToolCalls = toolCalls.length > 0;
  const hasToolResults = toolResults.length > 0;
  if (!hasThinking && !hasToolCalls && !hasToolResults) {
    return null;
  }

  const statsLabel = buildTraceStatsLabel(v);
  // 一次工具调用对应一个工具结果:按下标把调用[i]和结果[i]相邻渲染,而不是
  // 全部调用堆一起、全部结果堆一起。协议目前没有调用 id 做精确配对
  // (ToolCallInfo/ToolResultEntry 都没有 tool_use_id 字段),下标顺序是
  // 目前能拿到的最佳近似;数量不一致时缺的那一侧对应位置就不渲染。用
  // Fragment 分组而不是外包一层 div——`.trace-item` 必须保持是
  // `.trace-timeline` 的直接子节点,`:last-child` 消隐最后一项的
  // margin-bottom 和连接线的定位都依赖这一点。
  const pairCount = Math.max(toolCalls.length, toolResults.length);

  return (
    <details class="trace-toggle">
      <summary>{statsLabel ? <span class="trace-stats">{statsLabel}</span> : null}</summary>
      <div class="trace-timeline">
        {hasThinking ? (
          <div class="trace-item thinking">
            <div class="item-title">思考过程</div>
            <div class="thinking-text">{renderMarkdown(v.thinking_text as string)}</div>
          </div>
        ) : null}
        {Array.from({ length: pairCount }, (_, i) => {
          const call = toolCalls[i];
          const result = toolResults[i];
          return (
            <Fragment key={i}>
              {call ? (
                <div class="trace-item tool-call">
                  {i === 0 ? <div class="item-title">操作过程</div> : null}
                  <ToolCallRow call={call} />
                </div>
              ) : null}
              {result ? (
                <div class={`trace-item tool-result${result.is_error ? ' error' : ''}`}>
                  {i === 0 ? <div class="item-title">工具结果</div> : null}
                  <ToolResultRow result={result} />
                </div>
              ) : null}
            </Fragment>
          );
        })}
      </div>
    </details>
  );
}
