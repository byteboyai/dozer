import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import type { ViewPayload } from './types.ts';

const empty: ViewPayload = {
  project_id: 1,
  category_key: 'all',
  items: [],
  categories: [],
  agents: [],
  add_height_px: 60,
  today: { year: 2026, month: 10, day: 2 },
  selected_id: null,
  scroll_nonce: 0,
};

test('scaffold renders', () => {
  assert.match(render(<App payload={empty} />), /todo-root/);
});
