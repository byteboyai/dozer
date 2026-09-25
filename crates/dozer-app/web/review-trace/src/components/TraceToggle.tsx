import { Fragment } from 'preact';
import type { AiTurnData } from '../types.ts';
import { buildTraceStatsLabel } from '../traceStats.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolCallRow } from './ToolCallRow.tsx';
import { ToolResultRow } from './ToolResultRow.tsx';
import { pairCallsAndResults } from '../pairing.ts';

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
  // 每个 `.trace-item` 必须保持是 `.trace-timeline` 的直接子节点，
  // `:last-child` 消隐最后一项的 margin-bottom 和连接线定位都依赖这一点，
  // 所以用 Fragment 分组而不是外包一层 div。配对规则见 pairing.ts。
  const pairs = pairCallsAndResults(toolCalls, toolResults);
  const firstResultPairIndex = pairs.findIndex((p) => p.results.length > 0);

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
        {pairs.map((pair, i) => (
          <Fragment key={i}>
            {pair.call ? (
              <div class="trace-item tool-call">
                {i === 0 ? <div class="item-title">操作过程</div> : null}
                <ToolCallRow call={pair.call} />
              </div>
            ) : null}
            {pair.results.map((result, j) => (
              <div class={`trace-item tool-result${result.is_error ? ' error' : ''}`} key={j}>
                {i === firstResultPairIndex && j === 0 ? (
                  <div class="item-title">工具结果</div>
                ) : null}
                <ToolResultRow result={result} />
              </div>
            ))}
          </Fragment>
        ))}
      </div>
    </details>
  );
}
