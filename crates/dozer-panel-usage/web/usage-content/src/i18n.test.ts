import test from 'node:test';
import assert from 'node:assert/strict';
import { t, setLocale, getLocale, USAGE_OWNER } from './i18n.ts';

test('owner id is the panel id', () => {
  assert.equal(USAGE_OWNER, 'dozer.usage');
});

test('resolves the en bundle by default', () => {
  setLocale('en');
  assert.equal(getLocale(), 'en');
  assert.equal(t('title', 'Usage'), 'Usage');
});

test('resolves zh-CN and interpolates args', () => {
  setLocale('zh-CN');
  assert.equal(t('title', 'Usage'), '用量');
  assert.equal(
    t('error.load_failed', 'Failed to load usage: {message}', { message: 'boom' }),
    '用量统计失败: boom',
  );
  setLocale('en');
});

test('falls back to the english literal when a key is missing', () => {
  setLocale('zh-CN');
  assert.equal(t('does.not.exist', 'Fallback text'), 'Fallback text');
  setLocale('en');
});
