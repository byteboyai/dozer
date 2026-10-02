import type { StatusFilter } from '../types.ts';
import { statusFilterLabel, STATUS_META } from '../statusMeta.ts';
import { isInlineCommit } from '../compose.ts';

interface Props {
  searchDraft: string;
  highlight: boolean;
  status: StatusFilter;
  onDraft(v: string): void;
  onSubmit(): void;
  onOpenStatusFilter(anchor: DOMRect): void;
}

export function Toolbar(p: Props) {
  const color = p.status === 'all' ? 'var(--cream)' : STATUS_META[p.status].color;
  return (
    <div class={`toolbar${p.highlight ? ' is-highlight' : ''}`}>
      <button
        type="button"
        class="status-segment"
        style={{ color }}
        onClick={(e) => p.onOpenStatusFilter((e.currentTarget as HTMLElement).getBoundingClientRect())}
      >
        <span>{statusFilterLabel(p.status)}</span>
        <span class="chevron" aria-hidden="true">▾</span>
      </button>
      <input
        class="search-input"
        type="text"
        placeholder="搜索任务…"
        value={p.searchDraft}
        onInput={(e) => p.onDraft((e.currentTarget as HTMLInputElement).value)}
        onKeyDown={(e) => {
          if (isInlineCommit(e)) {
            e.preventDefault();
            p.onSubmit();
          }
        }}
        onBlur={() => p.onSubmit()}
      />
      <button type="button" class="icon-btn" aria-label="搜索" onClick={() => p.onSubmit()}>
        ⌕
      </button>
    </div>
  );
}
