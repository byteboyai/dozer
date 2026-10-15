import test from 'node:test';
import assert from 'node:assert/strict';
import {
  STATUS_META,
  STATUS_FILTER_OPTIONS,
  statusLabel,
  statusFilterLabel,
} from './statusMeta.ts';
import { setLocale } from './i18n.ts';

test('labels and colors match the native implementation', () => {
  assert.deepEqual(STATUS_META.pending, { labelKey: 'status.pending', color: 'var(--cyan)' });
  assert.deepEqual(STATUS_META.in_progress, {
    labelKey: 'status.in_progress',
    color: 'var(--gold)',
  });
  assert.deepEqual(STATUS_META.suspended, {
    labelKey: 'status.suspended',
    color: 'var(--cream)',
  });
  assert.deepEqual(STATUS_META.done, { labelKey: 'status.done', color: 'var(--dim)' });
});

test('status filter offers all plus the four states in order', () => {
  assert.deepEqual(STATUS_FILTER_OPTIONS, ['all', 'pending', 'in_progress', 'suspended', 'done']);
});

test('status labels follow the active locale', () => {
  setLocale('en');
  assert.equal(statusLabel('pending'), 'To do');
  assert.equal(statusFilterLabel('all'), 'All');
  setLocale('zh-CN');
  assert.equal(statusLabel('pending'), '待办');
  assert.equal(statusLabel('done'), '已完成');
  assert.equal(statusFilterLabel('all'), '全部');
  // 复位,避免影响其它用例。
  setLocale('en');
});
