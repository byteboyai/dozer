import test from 'node:test';
import assert from 'node:assert/strict';
import { applyEnvelope } from './protocol.ts';

const env = (revision: number, message: string) =>
  JSON.stringify({
    protocol_version: 1,
    revision,
    payload: {
      scan: { scanning: false, scanned_at: null, scan_error: null, save_error: null },
      category: 'overview',
      body: { kind: 'empty', message },
    },
  });

test('applies a newer revision', () => {
  const s0 = { revision: 0, payload: null };
  const s1 = applyEnvelope(s0, env(1, 'a'));
  assert.equal(s1.revision, 1);
  assert.equal((s1.payload!.body as { message: string }).message, 'a');
});

// Review Focus 3:旧推送晚到不得覆盖新状态。
test('drops an older revision', () => {
  const s1 = applyEnvelope({ revision: 0, payload: null }, env(5, 'new'));
  const s2 = applyEnvelope(s1, env(3, 'old'));
  assert.equal(s2.revision, 5);
  assert.equal((s2.payload!.body as { message: string }).message, 'new');
});

test('ignores malformed json and keeps state', () => {
  const s1 = applyEnvelope({ revision: 2, payload: null }, 'not json');
  assert.equal(s1.revision, 2);
  const s2 = applyEnvelope(s1, JSON.stringify({ revision: 9 }));
  assert.equal(s2.revision, 2, '缺 payload 的推送忽略');
});
