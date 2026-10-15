import test from 'node:test';
import assert from 'node:assert/strict';
import { t, setLocale, getLocale, TODO_OWNER } from './i18n.ts';

test('owner id is the panel id', () => {
  assert.equal(TODO_OWNER, 'dozer.todo');
});

test('resolves the en bundle by default', () => {
  setLocale('en');
  assert.equal(getLocale(), 'en');
  assert.equal(t('clear_list', 'Clear list'), 'Clear list');
});

test('resolves zh-CN and interpolates args', () => {
  setLocale('zh-CN');
  assert.equal(t('clear_list', 'Clear list'), '清空列表');
  assert.equal(
    t('detail.assigned_to', 'Assigned to: {agent}', { agent: 'Claude' }),
    '指派给:Claude',
  );
  setLocale('en');
});

test('falls back to the english literal when a key is missing', () => {
  setLocale('zh-CN');
  assert.equal(t('does.not.exist', 'Fallback text'), 'Fallback text');
  setLocale('en');
});
