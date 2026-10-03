import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import { unloadedFixture, noGroupsFixture } from './fixtures.ts';

test('unloaded shows loading, not the empty-state', () => {
  const out = render(<App payload={unloadedFixture} />);
  assert.match(out, /加载中/);
  assert.doesNotMatch(out, /还没有群聊/);
});

test('loaded with no groups shows the empty-state', () => {
  assert.match(render(<App payload={noGroupsFixture} />), /还没有群聊/);
});
