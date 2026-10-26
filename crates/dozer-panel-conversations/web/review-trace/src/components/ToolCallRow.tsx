import type { ToolCall } from '../types.ts';

export function ToolCallRow({ call }: { call: ToolCall }) {
  return (
    <details class="tool-call-row">
      <summary>{call.summary}</summary>
      {call.input_json ? <div class="input-json">{call.input_json}</div> : null}
    </details>
  );
}
