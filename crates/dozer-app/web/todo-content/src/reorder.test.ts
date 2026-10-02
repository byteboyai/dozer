import test from 'node:test';
import assert from 'node:assert/strict';
import { afterIdForSlot, moveToSlot, slotFromY, isNoopMove, reconcilePendingOrder } from './reorder.ts';

// Review Focus 5
test('slot 0 means before everything -> after_id null', () => {
  assert.equal(afterIdForSlot([1, 2, 3], 3, 0), null);
});

test('after_id is the visible predecessor in the list without the dragged item', () => {
  // 拖 1 到 slot 2:剩余 [2,3],插在 3 之后?slot=2 → 前驱是 rest[1]=3
  assert.equal(afterIdForSlot([1, 2, 3], 1, 2), 3);
  assert.equal(afterIdForSlot([1, 2, 3], 1, 1), 2);
  assert.equal(afterIdForSlot([1, 2, 3], 3, 1), 1);
});

test('slot is clamped into range', () => {
  assert.equal(afterIdForSlot([1, 2, 3], 1, 99), 3);
  assert.equal(afterIdForSlot([1, 2, 3], 1, -5), null);
});

test('moveToSlot gives the preview order', () => {
  assert.deepEqual(moveToSlot([1, 2, 3], 3, 0), [3, 1, 2]);
  assert.deepEqual(moveToSlot([1, 2, 3], 1, 2), [2, 3, 1]);
});

test('dropping back at the original position is a no-op', () => {
  assert.equal(isNoopMove([1, 2, 3], 2, 1), true);
  assert.equal(isNoopMove([1, 2, 3], 2, 0), false);
});

test('slotFromY counts rows whose midpoint is above the pointer, excluding the dragged one', () => {
  const rects = [
    { id: 1, top: 0, bottom: 40 },
    { id: 2, top: 50, bottom: 90 },
    { id: 3, top: 100, bottom: 140 },
  ];
  assert.equal(slotFromY(rects, 1, 5), 0); // 在最上方
  assert.equal(slotFromY(rects, 1, 100), 1); // 越过 2 的中线(70)但没越过 3 的中线(120)
  assert.equal(slotFromY(rects, 1, 500), 2); // 越过所有
});

// 审阅 Important 2:放下后要保持预览顺序,直到权威推送到达,不能先弹回原位。
test('pending preview is kept while the server still reports the old order', () => {
  assert.deepEqual(reconcilePendingOrder([3, 1, 2], [1, 2, 3]), [3, 1, 2]);
});

test('pending preview settles (is dropped) once the server order matches it', () => {
  assert.equal(reconcilePendingOrder([3, 1, 2], [3, 1, 2]), null);
});

test('pending preview is dropped when the set of items changed (add/delete/complete)', () => {
  assert.equal(reconcilePendingOrder([3, 1, 2], [1, 2]), null);
  assert.equal(reconcilePendingOrder([3, 1, 2], [1, 2, 3, 4]), null);
});

test('no pending preview stays none', () => {
  assert.equal(reconcilePendingOrder(null, [1, 2, 3]), null);
});
