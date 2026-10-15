import type { TodoCard, ViewPayload } from './types.ts';

const agents = [
  { kind: 'claude', label: 'Claude', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
  { kind: 'codebuddy', label: 'CodeBuddy', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
  { kind: 'opencode', label: 'OpenCode', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
  { kind: 'v8agent', label: 'v8agent', icon_svg: '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>', preserves_color: false },
];

export const card = (id: number, text: string, over: Partial<TodoCard> = {}): TodoCard => ({
  id, text, state: 'pending', segment: 'active', plan_date: null, completed_label: null,
  category_id: null, category_name: null, assigned_agent: null, has_dispatch: false, ...over,
});

const base = (items: TodoCard[]): ViewPayload => ({
  project_id: 1,
  category_key: 'all',
  items,
  categories: [
    { id: 10, name: '后端', parent_id: null, depth: 0 },
    { id: 11, name: '接口', parent_id: 10, depth: 1 },
    { id: 12, name: '前端', parent_id: null, depth: 0 },
  ],
  agents,
  add_height_px: 60,
  today: { year: 2026, month: 10, day: 2 },
  selected_id: null,
  scroll_nonce: 0,
});

export const emptyFixture = base([]);

export const threeSegmentsFixture = base([
  card(1, '修复登录页闪烁', { plan_date: '10-05', category_id: 10, category_name: '后端' }),
  card(2, '给 claude 指派生成报告', { state: 'in_progress', has_dispatch: true, assigned_agent: 'claude' }),
  card(3, '等设计稿确认', { state: 'suspended', segment: 'paused' }),
  card(4, '补 README 安装说明', { state: 'done', segment: 'done', completed_label: '09-30' }),
]);

export const allDoneFixture = base([
  card(1, '已完成 A', { state: 'done', segment: 'done', completed_label: '10-01' }),
  card(2, '已完成 B', { state: 'done', segment: 'done', completed_label: '-' }),
]);

export const longTextFixture = base([
  card(1, '这是一段非常非常长的任务文字,'.repeat(12) + '用来确认卡片在窄宽度下会自然换行而不是撑破布局。'),
]);

export const englishFixture = base([card(1, 'Fix the flaky login test on CI')]);
