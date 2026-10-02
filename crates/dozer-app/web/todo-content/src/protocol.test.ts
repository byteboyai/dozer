import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope } from './protocol.ts';

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

test('applies a newer revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(1, 7));
  assert.equal(s1.revision, 1);
  assert.equal(s1.payload!.project_id, 7);
});

// Review Focus 4:旧推送晚到不得覆盖新状态。
test('drops an older revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(5, 1));
  const s2 = applyEnvelope(s1, env(3, 2));
  assert.equal(s2.revision, 5);
  assert.equal(s2.payload!.project_id, 1);
});

test('accepts an equal revision (idempotent re-push)', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(4, 1));
  const s2 = applyEnvelope(s1, env(4, 1));
  assert.equal(s2.revision, 4);
});

test('ignores malformed json and missing payload', () => {
  const s1 = applyEnvelope({ revision: 2, payload: null }, 'not json');
  assert.equal(s1.revision, 2);
  const s2 = applyEnvelope(s1, JSON.stringify({ revision: 9 }));
  assert.equal(s2.revision, 2);
  assert.equal(s2.payload, null);
});
