import type { ToolResult } from '../types.ts';
import { t } from '../i18n.ts';

export function ToolResultRow({ result }: { result: ToolResult }) {
  const firstLine = result.content.split('\n')[0].slice(0, 80);
  return (
    <details
      class={`tool-result-row${result.is_error ? ' error' : ''}`}
      open={result.content.length < 200}
    >
      <summary>
        {(result.is_error ? t('trace.failed_prefix', '⚠ Failed · ') : '→ ') + firstLine}
      </summary>
      <div class="body">{result.content}</div>
    </details>
  );
}
