import type { StatusFilter, TodoCard } from './types.ts';

/** 大小写不敏感的子串匹配;空/全空白查询匹配一切(同 Rust `filter_todos`)。 */
export function matchesSearch(text: string, query: string): boolean {
  const q = query.trim().toLowerCase();
  return q === '' || text.toLowerCase().includes(q);
}

/** 关键词 ∩ 状态,保持原顺序。分类过滤已由 Rust 在推送前完成。 */
export function visibleItems(items: TodoCard[], search: string, status: StatusFilter): TodoCard[] {
  return items.filter(
    (it) => matchesSearch(it.text, search) && (status === 'all' || it.state === status),
  );
}
