import type { StatusFilter, TodoState } from './types.ts';
import { t } from './i18n.ts';

export const STATUS_META: Record<TodoState, { labelKey: string; color: string }> = {
  pending: { labelKey: 'status.pending', color: 'var(--cyan)' },
  in_progress: { labelKey: 'status.in_progress', color: 'var(--gold)' },
  suspended: { labelKey: 'status.suspended', color: 'var(--cream)' },
  done: { labelKey: 'status.done', color: 'var(--dim)' },
};

const STATUS_FALLBACK: Record<TodoState, string> = {
  pending: 'To do',
  in_progress: 'In progress',
  suspended: 'Suspended',
  done: 'Done',
};

export const STATUS_FILTER_OPTIONS: StatusFilter[] = [
  'all',
  'pending',
  'in_progress',
  'suspended',
  'done',
];

/** 状态标签(随当前 locale 解析)。 */
export function statusLabel(s: TodoState): string {
  return t(STATUS_META[s].labelKey, STATUS_FALLBACK[s]);
}

export function statusFilterLabel(f: StatusFilter): string {
  return f === 'all' ? t('status.all', 'All') : statusLabel(f);
}
