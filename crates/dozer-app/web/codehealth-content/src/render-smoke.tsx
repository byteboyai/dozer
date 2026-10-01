import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import { emptyFixture, scanFailedFixture, scanningWithOldResultFixture } from './fixtures.ts';
import type { ViewPayload } from './types.ts';

const html = (p: ViewPayload) => render(<App payload={p} />);

test('empty: never scanned shows message and a scan button', () => {
  const out = html(emptyFixture);
  assert.match(out, /这个项目还没有扫描过。/);
  assert.match(out, /扫描/);
});

test('empty: scan error shows banner', () => {
  const out = html(scanFailedFixture);
  assert.match(out, /扫描失败,请重试:boom/);
});

test('scanning disables the button', () => {
  const out = html(scanningWithOldResultFixture);
  assert.match(out, /disabled/);
  assert.match(out, /扫描中…/);
});
