import test from 'node:test';
import assert from 'node:assert/strict';
import { placePopover } from './popover.ts';

const vp = { w: 800, h: 600 };

test('opens below the anchor when there is room', () => {
  const p = placePopover({ left: 100, top: 100, right: 160, bottom: 124 }, { w: 120, h: 80 }, vp);
  assert.deepEqual(p, { x: 100, y: 128 });
});

test('flips above when there is no room below', () => {
  const p = placePopover({ left: 100, top: 560, right: 160, bottom: 584 }, { w: 120, h: 80 }, vp);
  assert.equal(p.y, 560 - 80 - 4);
});

test('is clamped horizontally into the viewport (never leaves the webview rect)', () => {
  const p = placePopover({ left: 780, top: 100, right: 800, bottom: 124 }, { w: 200, h: 80 }, vp);
  assert.equal(p.x, 800 - 200 - 4);
  const q = placePopover({ left: -30, top: 100, right: 0, bottom: 124 }, { w: 200, h: 80 }, vp);
  assert.equal(q.x, 4);
});
