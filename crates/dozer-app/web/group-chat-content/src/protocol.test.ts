import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope, DEFAULT_LOCALE, type PushState } from './protocol.ts';

const payload = (project_id: number) => ({
  project_id,
  loaded: true,
  groups: [],
  selected_group_id: null,
  messages: [],
  hint: null,
  running: false,
});
const env = (revision: number, project_id: number) =>
  JSON.stringify({ protocol_version: 1, revision, payload: payload(project_id) });

const state = (over: Partial<PushState> = {}): PushState => ({
  revision: 0,
  payload: null,
  locale: DEFAULT_LOCALE,
  localeRevision: 0,
  ...over,
});

test('applies a newer revision', () => {
  const s1 = applyEnvelope(state(), env(1, 7));
  assert.equal(s1.revision, 1);
  assert.equal(s1.payload!.project_id, 7);
});

test('drops an older revision', () => {
  const s1 = applyEnvelope(state(), env(5, 1));
  const s2 = applyEnvelope(s1, env(3, 2));
  assert.equal(s2.revision, 5);
  assert.equal(s2.payload!.project_id, 1);
});

test('accepts an equal revision (idempotent re-push)', () => {
  const s1 = applyEnvelope(state(), env(4, 1));
  const s2 = applyEnvelope(s1, env(4, 1));
  assert.equal(s2.revision, 4);
});

test('ignores malformed json and missing payload', () => {
  const s1 = applyEnvelope(state({ revision: 2 }), 'not json');
  assert.equal(s1.revision, 2);
  const s2 = applyEnvelope(s1, JSON.stringify({ revision: 9 }));
  assert.equal(s2.revision, 2);
});

// spec §8:语言上下文随推送原子应用;业务 payload 不变、仅语言变化也应接受。
test('carries ui_locale and applies a locale-only change', () => {
  const s1 = applyEnvelope(state(), env(2, 5));
  const s2 = applyEnvelope(
    s1,
    JSON.stringify({
      protocol_version: 1,
      revision: 2,
      ui_locale: 'zh-CN',
      locale_revision: 1,
      payload: payload(5),
    }),
  );
  assert.equal(s2.locale, 'zh-CN');
  assert.equal(s2.localeRevision, 1);
  assert.equal(s2.payload!.project_id, 5);
});

test('keeps previous locale when the field is absent (old Host)', () => {
  const s = applyEnvelope(state({ locale: 'zh-CN', localeRevision: 3 }), env(1, 1));
  assert.equal(s.locale, 'zh-CN', '缺字段时沿用上一语言');
  assert.equal(s.localeRevision, 3);
});
