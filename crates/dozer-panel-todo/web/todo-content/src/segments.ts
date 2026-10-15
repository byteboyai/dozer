import type { TodoCard } from './types.ts';

export interface Segments {
  active: TodoCard[];
  paused: TodoCard[];
  done: TodoCard[];
}

export function splitSegments(items: TodoCard[]): Segments {
  const s: Segments = { active: [], paused: [], done: [] };
  for (const it of items) s[it.segment].push(it);
  return s;
}

/** `#NNN` 是"当前可见卡片的显示序号":跨三段连续递增(进行中 → 搁置 → 已完成),
 *  从 1 起,随搜索与状态筛选变化(同原生的 `shown` 计数)。 */
export function displayNumbers(s: Segments): Map<number, number> {
  const m = new Map<number, number>();
  let n = 0;
  for (const it of [...s.active, ...s.paused, ...s.done]) m.set(it.id, ++n);
  return m;
}

export function formatNumber(n: number): string {
  return `#${String(n).padStart(3, '0')}`;
}
