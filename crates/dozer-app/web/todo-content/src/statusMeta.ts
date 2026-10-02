import type { StatusFilter, TodoState } from './types.ts';

export const STATUS_META: Record<TodoState, { label: string; color: string }> = {
  pending: { label: '待办', color: 'var(--cyan)' },
  in_progress: { label: '进行中', color: 'var(--gold)' },
  suspended: { label: '搁置', color: 'var(--cream)' },
  done: { label: '已完成', color: 'var(--dim)' },
};

export const STATUS_FILTER_OPTIONS: StatusFilter[] = [
  'all',
  'pending',
  'in_progress',
  'suspended',
  'done',
];

export function statusFilterLabel(f: StatusFilter): string {
  return f === 'all' ? '全部' : STATUS_META[f].label;
}
