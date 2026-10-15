import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope, DEFAULT_LOCALE, type PushState } from './protocol.ts';

const payload = (project_id: number) => ({
  project_id,
  category_key: 'all',
  items: [],
  categories: [],
  agents: [],
  add_height_px: 60,
  today: { year: 2026, month: 10, day: 2 },
  selected_id: null,
  scroll_nonce: 0,
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

// Review Focus 4:旧推送晚到不得覆盖新状态。
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
  assert.equal(s2.payload, null);
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
      payload: payload(7),
    }),
  );
  assert.equal(s.locale, 'zh-CN');
  assert.equal(s.localeRevision, 3);
  assert.equal(s.payload!.project_id, 7);
});

test('defaults locale to en when the field is absent (old Host)', () => {
  const s = applyEnvelope(
    state({ locale: 'zh-CN', localeRevision: 3 }),
    env(1, 1),
  );
  assert.equal(s.locale, 'zh-CN', '缺字段时沿用上一语言');
  assert.equal(s.localeRevision, 3);
});

// spec §8:业务 payload 不变、仅语言变化的推送(同 revision 也应被接受)。
test('applies a locale change even when the payload is unchanged', () => {
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
  assert.equal(s2.revision, 2);
  assert.equal(s2.payload!.project_id, 5);
});
