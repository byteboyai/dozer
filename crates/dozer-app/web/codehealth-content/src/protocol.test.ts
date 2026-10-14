import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope, DEFAULT_LOCALE, type PushState } from './protocol.ts';

const env = (revision: number, message: string) =>
  JSON.stringify({
    protocol_version: 1,
    revision,
    payload: {
      scan: { scanning: false, scanned_at: null, scan_error: null, scan_error_key: null, save_error: null, save_error_key: null },
      category: 'overview',
      body: { kind: 'empty', message },
    },
  });

const state = (over: Partial<PushState> = {}): PushState => ({
  revision: 0,
  payload: null,
  locale: DEFAULT_LOCALE,
  localeRevision: 0,
  ...over,
});

test('applies a newer revision', () => {
  const s0 = state();
  const s1 = applyEnvelope(s0, env(1, 'a'));
  assert.equal(s1.revision, 1);
  assert.equal((s1.payload!.body as { message: string }).message, 'a');
});

// Review Focus 3:旧推送晚到不得覆盖新状态。
test('drops an older revision', () => {
  const s1 = applyEnvelope(state(), env(5, 'new'));
  const s2 = applyEnvelope(s1, env(3, 'old'));
  assert.equal(s2.revision, 5);
  assert.equal((s2.payload!.body as { message: string }).message, 'new');
});

test('ignores malformed json and keeps state', () => {
  const s1 = applyEnvelope(state({ revision: 2 }), 'not json');
  assert.equal(s1.revision, 2);
  const s2 = applyEnvelope(s1, JSON.stringify({ revision: 9 }));
  assert.equal(s2.revision, 2, '缺 payload 的推送忽略');
});

// spec §8:语言上下文随推送原子应用;旧 Host 无字段时保留上一语言。
test('carries ui_locale and locale_revision into the state', () => {
  const s = applyEnvelope(
    state(),
    JSON.stringify({
      protocol_version: 1,
      revision: 1,
      ui_locale: 'zh-CN',
      locale_revision: 3,
      payload: {
        scan: { scanning: false, scanned_at: null, scan_error: null, scan_error_key: null, save_error: null, save_error_key: null },
        category: 'overview',
        body: { kind: 'empty', message: 'x' },
      },
    }),
  );
  assert.equal(s.locale, 'zh-CN');
  assert.equal(s.localeRevision, 3);
});

test('keeps previous locale when the field is absent (old Host)', () => {
  const s = applyEnvelope(state({ locale: 'zh-CN', localeRevision: 3 }), env(1, 'a'));
  assert.equal(s.locale, 'zh-CN', '缺字段时沿用上一语言');
  assert.equal(s.localeRevision, 3);
});
