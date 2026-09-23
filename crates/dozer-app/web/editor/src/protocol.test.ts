import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PROTOCOL_VERSION,
  decodeCommand,
  decodeEnvelope,
  encodeEnvelope,
  isPosition,
  isRange,
  isValidEnvelope,
  type Envelope,
} from './protocol.ts';

function envelope(payload: unknown): Envelope {
  return {
    protocol_version: PROTOCOL_VERSION,
    project_id: 7,
    panel: 'files',
    tab_id: 3,
    document_id: 'doc-3',
    revision: 2,
    request_id: null,
    payload,
  };
}

test('envelope round-trips and validates', () => {
  const raw = encodeEnvelope(envelope({ kind: 'ready', read_only: false, language: 'rust' }));
  const decoded = decodeEnvelope(raw);
  assert.ok(decoded);
  assert.equal(decoded?.tab_id, 3);
});

test('decodeEnvelope rejects non-json, wrong version and bad shape', () => {
  assert.equal(decodeEnvelope('not json'), null);
  assert.equal(decodeEnvelope('{}'), null);
  const wrongVersion = { ...envelope({}), protocol_version: 999 };
  assert.equal(decodeEnvelope(JSON.stringify(wrongVersion)), null);
  assert.equal(isValidEnvelope({ protocol_version: PROTOCOL_VERSION }), false);
});

test('decodeEnvelope rejects missing request_id type', () => {
  const bad = { ...envelope({}), request_id: 5 };
  assert.equal(decodeEnvelope(JSON.stringify(bad)), null);
});

test('position helpers are 1-based ints', () => {
  assert.ok(isPosition({ line: 1, column: 1 }));
  assert.ok(!isPosition({ line: 0, column: 1 }));
  assert.ok(!isPosition({ line: 1.5, column: 1 }));
  assert.ok(!isPosition({ line: 1 }));
  assert.ok(isRange({ start: { line: 1, column: 2 }, end: { line: 3, column: 4 } }));
});

test('decodeCommand accepts known commands', () => {
  const setDoc = decodeCommand({
    kind: 'set_document',
    text: 'hi',
    revision: 1,
    language: 'rust',
    read_only: false,
  });
  assert.equal(setDoc?.kind, 'set_document');

  const reveal = decodeCommand({ kind: 'reveal_position', line: 10, column: 2 });
  assert.equal(reveal?.kind, 'reveal_position');

  const replace = decodeCommand({
    kind: 'replace_range',
    start: { line: 1, column: 1 },
    end: { line: 1, column: 3 },
    text: 'x',
    revision: 4,
  });
  assert.equal(replace?.kind, 'replace_range');

  const find = decodeCommand({ kind: 'open_find', query: 'foo', replace: true });
  assert.equal(find?.kind, 'open_find');

  const save = decodeCommand({ kind: 'save_document' });
  assert.equal(save?.kind, 'save_document');
});

test('decodeCommand accepts set_diff_document with all fields', () => {
  const decoded = decodeCommand({
    kind: 'set_diff_document',
    old_text: 'old\n',
    new_text: 'new\n',
    language: 'rust',
    revision: 5,
    read_only: true,
  });
  assert.deepStrictEqual(decoded, {
    kind: 'set_diff_document',
    old_text: 'old\n',
    new_text: 'new\n',
    language: 'rust',
    revision: 5,
    read_only: true,
  });
});

test('decodeCommand rejects set_diff_document missing a required field', () => {
  const decoded = decodeCommand({
    kind: 'set_diff_document',
    old_text: 'old\n',
    new_text: 'new\n',
    language: 'rust',
    revision: 5,
    // read_only intentionally missing
  });
  assert.strictEqual(decoded, null);
});

test('decodeCommand rejects malformed and unknown commands', () => {
  assert.equal(decodeCommand(null), null);
  assert.equal(decodeCommand({ kind: 'evil' }), null);
  assert.equal(decodeCommand({ kind: 'set_document', text: 'x' }), null);
  assert.equal(
    decodeCommand({ kind: 'replace_range', start: { line: 1, column: 1 }, text: 'x', revision: 1 }),
    null,
  );
  assert.equal(decodeCommand({ kind: 'set_read_only', read_only: 'yes' }), null);
});
