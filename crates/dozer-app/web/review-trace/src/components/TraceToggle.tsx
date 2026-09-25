import type { AiTurnData } from '../types.ts';
import { buildTraceStatsLabel } from '../traceStats.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolCallRow } from './ToolCallRow.tsx';
import { ToolResultRow } from './ToolResultRow.tsx';

export function TraceToggle({ v }: { v: AiTurnData }) {
  const hasThinking = !!v.thinking_text;
  const hasToolCalls = !!(v.tool_calls && v.tool_calls.length > 0);
  const hasToolResults = !!(v.tool_results && v.tool_results.length > 0);
  if (!hasThinking && !hasToolCalls && !hasToolResults) {
    return null;
  }

  const statsLabel = buildTraceStatsLabel(v);

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
        {hasToolCalls
          ? v.tool_calls!.map((call, i) => (
              <div class="trace-item tool-call" key={i}>
                {i === 0 ? <div class="item-title">操作过程</div> : null}
                <ToolCallRow call={call} />
              </div>
            ))
          : null}
        {hasToolResults
          ? v.tool_results!.map((result, i) => (
              <div class={`trace-item tool-result${result.is_error ? ' error' : ''}`} key={i}>
                {i === 0 ? <div class="item-title">工具结果</div> : null}
                <ToolResultRow result={result} />
              </div>
            ))
          : null}
      </div>
    </details>
  );
}
