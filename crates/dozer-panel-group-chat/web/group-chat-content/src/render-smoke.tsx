import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import { setLocale } from './i18n.ts';
import {
  unloadedFixture, noGroupsFixture,
  conversationFixture, runningFixture, failedFixture, maliciousFixture, hintFixture,
  noMembersFixture, noSelectionFixture,
} from './fixtures.ts';

setLocale('en');

test('unloaded shows loading, not the empty-state', () => {
  const out = render(<App payload={unloadedFixture} />);
  assert.match(out, /Loading/);
  assert.doesNotMatch(out, /No group chats yet/);
});

test('loaded with no groups shows the empty-state', () => {
  assert.match(render(<App payload={noGroupsFixture} />), /No group chats yet/);
});

test('member bar lists handles with agent labels and an add button', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /@architect/);
  assert.match(out, /Claude/);
  assert.match(out, /@reviewer/);
  assert.match(out, /Codex/);
  assert.match(out, /Add member/);
});

test('messages render authors, markdown and durations', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, />You</);
  assert.match(out, /<strong>Reasons<\/strong>/);
  assert.match(out, /<li>Blocks credential stuffing<\/li>/);
  assert.match(out, /<code>rate limiting<\/code>/);
  assert.match(out, /2\.3 s/);
});

test('todo badge appears only on messages already pushed to a todo; push button on the others', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /Added to to-do/);
  assert.match(out, /To to-do/);
});

test('running and queued messages show status labels and a stop control', () => {
  const out = render(<App payload={runningFixture} />);
  assert.match(out, /Speaking/);
  assert.match(out, /Queued/);
  assert.match(out, /Stop this round/);
});

test('failed message shows reason and retry; cancelled shows retry too', () => {
  const out = render(<App payload={failedFixture} />);
  assert.match(out, /Speaking timed out/);
  assert.match(out, /Cancelled/);
  assert.equal((out.match(/Retry/g) ?? []).length, 2);
});

// Review Focus 5
test('hostile content is escaped in the rendered page', () => {
  const out = render(<App payload={maliciousFixture} />);
  assert.doesNotMatch(out, /<script/i);
  assert.doesNotMatch(out, /<img/i);
  assert.doesNotMatch(out, /href="javascript/i);
  assert.match(out, /&lt;script&gt;/);
});

test('hint is shown near the composer', () => {
  assert.match(render(<App payload={hintFixture} />), /Members not found: @nobody/);
});

test('empty group shows a hint to add members and disables nothing silently', () => {
  const out = render(<App payload={noMembersFixture} />);
  assert.match(out, /no members yet/);
});

test('no selected group but groups exist prompts to pick one', () => {
  assert.match(render(<App payload={noSelectionFixture} />), /Select a group chat/);
});

test('composer placeholder explains @ and keys', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /use @ to call on members/);
});
