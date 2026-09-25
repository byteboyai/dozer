import type { TraceData } from '../types.ts';
import { renderMarkdown } from '../markdown.ts';

type SummaryHeaderData = Pick<TraceData, 'summary_title' | 'summary_time' | 'summary_text'>;

// 分割线和摘要头绑在同一个返回值里(而不是让调用方另外判断一次
// `summary_text` 是否存在):原实现靠 `renderSummaryHeader(data)` 的返回值
// 同时控制两者("有摘要头才有分割线"),这里保持同一个单一判断点,不给
// 调用方留一个可能和这里的判据走岔的重复条件。
export function SummaryHeader({ data }: { data: SummaryHeaderData }) {
  if (!data.summary_text) return null;
  return (
    <>
      <div class="summary-header">
        {data.summary_title ? <div class="summary-title">{data.summary_title}</div> : null}
        {data.summary_time ? <div class="summary-time">{data.summary_time}</div> : null}
        <div class="summary-body">{renderMarkdown(data.summary_text)}</div>
      </div>
      <hr class="topic-divider" />
    </>
  );
}
