import test from 'node:test';
import assert from 'node:assert/strict';
import { matchesSearch, visibleItems } from './filter.ts';
import type { TodoCard } from './types.ts';

const card = (id: number, text: string, state: TodoCard['state'] = 'pending'): TodoCard => ({
  id, text, state, segment: state === 'done' ? 'done' : state === 'suspended' ? 'paused' : 'active',
  plan_date: null, completed_label: null, category_id: null, category_name: null,
  assigned_agent: null, has_dispatch: false,
});

test('empty query keeps everything', () => {
  assert.equal(matchesSearch('任何文字', ''), true);
  assert.equal(matchesSearch('任何文字', '   '), true);
});

test('substring match is case-insensitive', () => {
  assert.equal(matchesSearch('给 Claude 指派生成报告', 'claude'), true);
  assert.equal(matchesSearch('修复登录页闪烁', '登录'), true);
  assert.equal(matchesSearch('修复登录页闪烁', '不存在'), false);
});

test('visibleItems intersects search and status, keeping order', () => {
  const items = [card(1, '修复登录', 'pending'), card(2, '补 README', 'done'), card(3, '修复注册', 'done')];
  assert.deepEqual(visibleItems(items, '修复', 'all').map((i) => i.id), [1, 3]);
  assert.deepEqual(visibleItems(items, '修复', 'done').map((i) => i.id), [3]);
  assert.deepEqual(visibleItems(items, '', 'pending').map((i) => i.id), [1]);
});
