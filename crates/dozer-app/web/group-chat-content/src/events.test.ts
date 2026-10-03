import test from 'node:test';
import assert from 'node:assert/strict';
import { cancelEvent } from './events.ts';

test('cancel event carries the group id (not a message id) and the scope', () => {
  assert.deepEqual(cancelEvent(7, 'turn'), { kind: 'cancel', group_id: 7, scope: 'turn' });
  assert.deepEqual(cancelEvent(7, 'round'), { kind: 'cancel', group_id: 7, scope: 'round' });
});
