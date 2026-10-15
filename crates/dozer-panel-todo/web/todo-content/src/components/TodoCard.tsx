import { useEffect, useRef } from 'preact/hooks';
import type { TodoCard as Card } from '../types.ts';
import { STATUS_META, statusLabel } from '../statusMeta.ts';
import { formatNumber } from '../segments.ts';
import { isCancel, isInlineCommit } from '../compose.ts';
import { MAX_TEXT_CHARS } from '../limits.ts';
import { t } from '../i18n.ts';

export interface CardProps {
  item: Card;
  number: number;
  selected: boolean;
  draggable: boolean;
  dragging: boolean;
  editing: boolean;
  onToggle(): void;
  onSelect(): void;
  onBeginEdit(): void;
  onCommitEdit(text: string): void;
  onCancelEdit(): void;
  onOpenCategory(anchor: DOMRect): void;
  onOpenCalendar(anchor: DOMRect): void;
  onOpenStatus(anchor: DOMRect): void;
  onOpenDispatch(anchor: DOMRect): void;
  onOpenDetail(): void;
  onPointerDown(e: PointerEvent): void;
}

const rectOf = (e: Event) => (e.currentTarget as HTMLElement).getBoundingClientRect();

export function TodoCard(p: CardProps) {
  const { item } = p;
  const done = item.state === 'done';
  const meta = STATUS_META[item.state];
  const dateLabel = done ? item.completed_label ?? '-' : item.plan_date ?? '-';
  const showDetail = item.assigned_agent !== null || item.has_dispatch;
  const showAssign = item.state === 'pending' && !item.has_dispatch;
  const taRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (p.editing && taRef.current) {
      const el = taRef.current;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    }
  }, [p.editing]);

  return (
    <div
      class={`todo-card${done ? ' is-done' : ''}${p.selected ? ' is-selected' : ''}${p.dragging ? ' is-dragging' : ''}${p.draggable ? ' is-draggable' : ''}`}
      data-id={item.id}
      onPointerDown={(e) => {
        p.onSelect();
        p.onPointerDown(e as unknown as PointerEvent);
      }}
    >
      <div class="card-top">
        <span class="card-number">{formatNumber(p.number)}</span>
        <button type="button" class="chip" onClick={(e) => p.onOpenCategory(rectOf(e))}>
          {item.category_name ?? t('category.uncategorized', 'Uncategorized')}
        </button>
        <span class="spacer" />
        <button type="button" class="date-badge" onClick={(e) => p.onOpenCalendar(rectOf(e))}>
          <span aria-hidden="true">▦</span>
          <span>{dateLabel}</span>
        </button>
      </div>
      <div class="card-body">
        <button
          type="button"
          class={`checkbox${done ? ' is-checked' : ''}`}
          aria-label={t('card.toggle_done', 'Toggle done')}
          onClick={() => p.onToggle()}
        >
          {done ? '✓' : ''}
        </button>
        {p.editing ? (
          <textarea
            ref={taRef}
            class="edit-area"
            placeholder={t('card.text_placeholder', 'Task content...')}
            maxLength={MAX_TEXT_CHARS}
            defaultValue={item.text}
            rows={1}
            onKeyDown={(e) => {
              if (isInlineCommit(e)) {
                e.preventDefault();
                p.onCommitEdit((e.currentTarget as HTMLTextAreaElement).value);
              } else if (isCancel(e)) {
                e.preventDefault();
                p.onCancelEdit();
              }
            }}
            onBlur={(e) => p.onCommitEdit((e.currentTarget as HTMLTextAreaElement).value)}
          />
        ) : (
          <div class="card-text" onClick={() => p.onBeginEdit()}>
            {item.text}
          </div>
        )}
      </div>
      <div class="card-bottom">
        <button
          type="button"
          class="status-btn"
          style={{ color: meta.color }}
          onClick={(e) => p.onOpenStatus(rectOf(e))}
        >
          <span>{statusLabel(item.state)}</span>
          <span class="chevron" aria-hidden="true">▾</span>
        </button>
        {showDetail ? (
          <button type="button" class="detail-btn" onClick={() => p.onOpenDetail()}>
            {t('card.detail', 'Details')}
          </button>
        ) : null}
        {showAssign ? (
          <button type="button" class="assign-btn" onClick={(e) => p.onOpenDispatch(rectOf(e))}>
            <span>{t('card.assign', 'Assign')}</span>
            <span aria-hidden="true">›</span>
          </button>
        ) : null}
      </div>
    </div>
  );
}
