import type { TraceData } from '../types.ts';
import { renderMarkdown } from '../markdown.ts';

type SummaryHeaderData = Pick<TraceData, 'summary_title' | 'summary_time' | 'summary_text'>;

export function SummaryHeader({ data }: { data: SummaryHeaderData }) {
  if (!data.summary_text) return null;
  return (
    <div class="summary-header">
      {data.summary_title ? <div class="summary-title">{data.summary_title}</div> : null}
      {data.summary_time ? <div class="summary-time">{data.summary_time}</div> : null}
      <div class="summary-body">{renderMarkdown(data.summary_text)}</div>
    </div>
  );
}
