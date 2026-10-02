import test from 'node:test';
import assert from 'node:assert/strict';
import { UiStore } from './uiStore.ts';
import type { ViewPayload } from './types.ts';

const p = (over: Partial<ViewPayload>): ViewPayload => ({
  project_id: 1, category_key: 'all', items: [], categories: [], agents: [],
  add_height_px: 60, today: { year: 2026, month: 10, day: 2 },
  selected_id: null, scroll_nonce: 0, ...over,
});

// Review Focus 3
test('state is kept per project across switches', () => {
  const s = new UiStore();
  s.onPayload(p({ project_id: 1 }));
  s.get(1).addDraft = '写到一半的任务';
  s.get(1).search = '登录';
  s.onPayload(p({ project_id: 2 }));
  assert.equal(s.get(2).addDraft, '');
  s.onPayload(p({ project_id: 1 }));
  assert.equal(s.get(1).addDraft, '写到一半的任务');
  assert.equal(s.get(1).search, '登录');
});

test('category change clears search but keeps the add draft', () => {
  const s = new UiStore();
  s.onPayload(p({ category_key: 'all' }));
  s.get(1).search = 'x';
  s.get(1).searchDraft = 'x';
  s.get(1).addDraft = '草稿';
  s.onPayload(p({ category_key: 'node:7' }));
  assert.equal(s.get(1).search, '');
  assert.equal(s.get(1).searchDraft, '');
  assert.equal(s.get(1).addDraft, '草稿');
});

test('first payload for a project does not clear search', () => {
  const s = new UiStore();
  s.get(1).search = 'keep';
  s.onPayload(p({ category_key: 'node:3' }));
  assert.equal(s.get(1).search, 'keep');
});

test('selected_id from Rust overrides local selection only when it changes', () => {
  const s = new UiStore();
  s.onPayload(p({ selected_id: 5 }));
  assert.equal(s.get(1).selectedId, 5);
  s.get(1).selectedId = 9; // 用户点了别的卡片
  s.onPayload(p({ selected_id: 5 })); // 同一个值再次推送,不应覆盖用户的选择
  assert.equal(s.get(1).selectedId, 9);
  s.onPayload(p({ selected_id: null })); // Rust 的 2 秒高亮到点清除
  assert.equal(s.get(1).selectedId, null);
});

test('scroll_nonce change requests scroll-to-top exactly once', () => {
  const s = new UiStore();
  assert.equal(s.onPayload(p({ scroll_nonce: 0 })).scrollToTop, false);
  assert.equal(s.onPayload(p({ scroll_nonce: 1 })).scrollToTop, true);
  assert.equal(s.onPayload(p({ scroll_nonce: 1 })).scrollToTop, false);
});
