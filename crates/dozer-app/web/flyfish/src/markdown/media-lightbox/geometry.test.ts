import assert from 'node:assert/strict';
import { describe, test } from 'node:test';

import {
  GeometryError,
  centeredState,
  clampTranslation,
  contentPointAt,
  fitScale,
  reconcileOnResize,
  stageTransform,
  zoomAtPoint,
} from './geometry.ts';
import { DEFAULT_FIT_INSETS, SCALE_MAX, SCALE_MIN, type ViewportState } from './types.ts';

const close = (actual: number, expected: number, eps = 1e-9) => {
  assert.ok(
    Math.abs(actual - expected) <= eps,
    `expected ${actual} ≈ ${expected} (eps ${eps})`
  );
};

describe('fitScale', () => {
  const cases: Array<{
    name: string;
    content: { width: number; height: number };
    viewport: { width: number; height: number };
    expected: number;
  }> = [
    { name: 'wide image fits by width', content: { width: 2000, height: 1000 }, viewport: { width: 1000, height: 1000 }, expected: 0.5 },
    { name: 'tall image fits by height', content: { width: 1000, height: 2000 }, viewport: { width: 1000, height: 1000 }, expected: 0.5 },
    { name: 'exact fit is 1', content: { width: 800, height: 600 }, viewport: { width: 800, height: 600 }, expected: 1 },
    { name: 'small image never upscale', content: { width: 100, height: 100 }, viewport: { width: 1000, height: 1000 }, expected: 1 },
    { name: '1px image stays at 1', content: { width: 1, height: 1 }, viewport: { width: 1000, height: 1000 }, expected: 1 },
    { name: 'extreme aspect very wide clamped to min', content: { width: 100000, height: 1 }, viewport: { width: 1000, height: 1000 }, expected: 0.1 },
    { name: 'extreme aspect very tall clamped to min', content: { width: 1, height: 100000 }, viewport: { width: 1000, height: 1000 }, expected: 0.1 },
    { name: 'huge svg downscales, clamped to min', content: { width: 20000, height: 20000 }, viewport: { width: 500, height: 500 }, expected: 0.1 },
    { name: 'tiny viewport clamps to min', content: { width: 10000, height: 10000 }, viewport: { width: 10, height: 10 }, expected: 0.1 },
  ];

  for (const c of cases) {
    test(c.name, () => {
      const scale = fitScale(c.content, c.viewport);
      close(scale, c.expected);
      assert.ok(Number.isFinite(scale));
      assert.ok(scale >= SCALE_MIN && scale <= 1);
    });
  }

  test('zero content size returns 100% not NaN', () => {
    const scale = fitScale({ width: 0, height: 0 }, { width: 1000, height: 1000 });
    assert.equal(scale, 1);
  });

  test('zero viewport returns 100% not NaN', () => {
    const scale = fitScale({ width: 100, height: 100 }, { width: 0, height: 0 });
    assert.equal(scale, 1);
  });

  test('insets reduce available area', () => {
    const scale = fitScale(
      { width: 1000, height: 1000 },
      { width: 1000, height: 1000 },
      { top: 100, right: 0, bottom: 100, left: 0 }
    );
    close(scale, 0.8);
  });

  test('non-finite input throws instead of producing NaN', () => {
    assert.throws(() => fitScale({ width: Number.NaN, height: 1 }, { width: 1, height: 1 }), GeometryError);
    assert.throws(() => fitScale({ width: 1, height: 1 }, { width: Infinity, height: 1 }), GeometryError);
  });
});

describe('zoomAtPoint', () => {
  test('keeps content point under cursor stable', () => {
    const state: ViewportState = { scale: 1, translation: { x: 0, y: 0 }, mode: 'manual' };
    const cursor = { x: 200, y: 150 };
    const before = contentPointAt(cursor, state);
    const next = zoomAtPoint(state, 2, cursor);
    const after = contentPointAt(cursor, next);
    close(after.x, before.x);
    close(after.y, before.y);
  });

  test('clamps to 10%..500%', () => {
    const state: ViewportState = { scale: 1, translation: { x: 0, y: 0 }, mode: 'manual' };
    assert.equal(zoomAtPoint(state, 100, { x: 0, y: 0 }).scale, SCALE_MAX);
    assert.equal(zoomAtPoint(state, 0.001, { x: 0, y: 0 }).scale, SCALE_MIN);
  });

  test('uses provided anchor contentPoint over derived', () => {
    const state: ViewportState = { scale: 2, translation: { x: 10, y: 20 }, mode: 'manual' };
    const next = zoomAtPoint(state, 1, { x: 50, y: 60 }, {
      viewportPoint: { x: 50, y: 60 },
      contentPoint: { x: 0, y: 0 },
    });
    close(next.translation.x, 50);
    close(next.translation.y, 60);
  });

  test('result is always finite', () => {
    const state: ViewportState = { scale: 1, translation: { x: 0, y: 0 }, mode: 'manual' };
    const next = zoomAtPoint(state, 3, { x: 0, y: 0 });
    assert.ok(Number.isFinite(next.translation.x));
    assert.ok(Number.isFinite(next.translation.y));
    assert.ok(Number.isFinite(next.scale));
  });

  test('repeated floating zoom never yields NaN', () => {
    const content = { width: 1000, height: 800 };
    const viewport = { width: 500, height: 400 };
    let state: ViewportState = centeredState(1, content, viewport);
    for (let i = 0; i < 200; i += 1) {
      const zoomed = zoomAtPoint(state, state.scale * 1.1, { x: 100, y: 100 });
      const translation = clampTranslation(zoomed.translation, content, viewport, zoomed.scale);
      state = { scale: zoomed.scale, translation, mode: 'manual' };
      assert.ok(Number.isFinite(state.scale));
      assert.ok(Number.isFinite(state.translation.x));
      assert.ok(Number.isFinite(state.translation.y));
      assert.ok(state.scale <= SCALE_MAX && state.scale >= SCALE_MIN);
    }
  });
});

describe('clampTranslation', () => {
  const content = { width: 1000, height: 500 };

  test('content smaller than viewport is centered', () => {
    const t = clampTranslation({ x: 999, y: -999 }, content, { width: 2000, height: 1000 }, 1);
    close(t.x, 500);
    close(t.y, 250);
  });

  test('content larger than viewport keeps recoverable edges', () => {
    const viewport = { width: 500, height: 250 };
    const t = clampTranslation({ x: 9999, y: 9999 }, content, viewport, 2);
    // scaled content = 2000x1000; x in [500-2000, 0] = [-1500, 0]; y in [-750, 0]
    close(t.x, 0);
    close(t.y, 0);
    const t2 = clampTranslation({ x: -9999, y: -9999 }, content, viewport, 2);
    close(t2.x, -1500);
    close(t2.y, -750);
  });

  test('non-finite translation input throws', () => {
    assert.throws(() => clampTranslation({ x: Number.NaN, y: 0 }, content, { width: 1, height: 1 }, 1), GeometryError);
  });
});

describe('reconcileOnResize', () => {
  test('fit mode re-fits to new viewport', () => {
    const content = { width: 1000, height: 1000 };
    const state = centeredState(1, content, { width: 500, height: 500 }, 'fit');
    const next = reconcileOnResize(state, content, { width: 250, height: 250 });
    close(next.scale, 0.25);
    assert.equal(next.mode, 'fit');
  });

  test('manual mode preserves scale', () => {
    const content = { width: 1000, height: 1000 };
    const state: ViewportState = { scale: 2, translation: { x: 9999, y: 9999 }, mode: 'manual' };
    const next = reconcileOnResize(state, content, { width: 300, height: 300 });
    close(next.scale, 2);
    close(next.translation.x, 0);
    close(next.translation.y, 0);
    assert.equal(next.mode, 'manual');
  });

  test('zoom out below viewport after resize re-centers', () => {
    const content = { width: 100, height: 100 };
    const state: ViewportState = { scale: 1, translation: { x: -50, y: -50 }, mode: 'manual' };
    const next = reconcileOnResize(state, content, { width: 400, height: 400 });
    close(next.translation.x, 150);
    close(next.translation.y, 150);
  });
});

describe('stageTransform', () => {
  test('emits translate3d + scale', () => {
    assert.equal(
      stageTransform({ scale: 2, translation: { x: 10, y: -5 } }),
      'translate3d(10px, -5px, 0) scale(2)'
    );
  });

  test('throws on NaN to avoid emitting invalid CSS', () => {
    assert.throws(() => stageTransform({ scale: Number.NaN, translation: { x: 0, y: 0 } }), GeometryError);
  });
});

describe('contentPointAt', () => {
  test('inverts translate then scale', () => {
    const p = contentPointAt({ x: 60, y: 40 }, { scale: 2, translation: { x: 10, y: 0 } });
    close(p.x, 25);
    close(p.y, 20);
  });

  test('zero scale throws', () => {
    assert.throws(() => contentPointAt({ x: 0, y: 0 }, { scale: 0, translation: { x: 0, y: 0 } }), GeometryError);
  });
});

describe('insets default', () => {
  test('default insets are all zero', () => {
    assert.deepEqual(DEFAULT_FIT_INSETS, { top: 0, right: 0, bottom: 0, left: 0 });
  });
});
