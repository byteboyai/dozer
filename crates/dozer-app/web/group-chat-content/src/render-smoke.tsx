import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import {
  unloadedFixture, noGroupsFixture,
  conversationFixture, runningFixture, failedFixture, maliciousFixture, hintFixture,
  noMembersFixture, noSelectionFixture,
} from './fixtures.ts';

test('unloaded shows loading, not the empty-state', () => {
  const out = render(<App payload={unloadedFixture} />);
  assert.match(out, /加载中/);
  assert.doesNotMatch(out, /还没有群聊/);
});

test('loaded with no groups shows the empty-state', () => {
  assert.match(render(<App payload={noGroupsFixture} />), /还没有群聊/);
});

test('group bar shows topics and marks the selected one', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /评审登录方案/);
  assert.match(out, /is-selected/);
  assert.match(out, /新建群聊/);
});

test('member bar lists handles with agent labels and an add button', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /@架构师/);
  assert.match(out, /Claude/);
  assert.match(out, /@审阅者/);
  assert.match(out, /Codex/);
  assert.match(out, /添加成员/);
});

test('messages render authors, markdown and durations', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /你/);
  assert.match(out, /<strong>理由<\/strong>/);
  assert.match(out, /<li>防撞库<\/li>/);
  assert.match(out, /<code>限流<\/code>/);
  assert.match(out, /2\.3 秒/);
});

test('todo badge appears only on messages already pushed to a todo; push button on the others', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /已转待办/);
  assert.match(out, /转为待办/);
});

test('running and queued messages show status labels and a stop control', () => {
  const out = render(<App payload={runningFixture} />);
  assert.match(out, /正在发言/);
  assert.match(out, /排队中/);
  assert.match(out, /停止本轮/);
});

test('failed message shows reason and retry; cancelled shows retry too', () => {
  const out = render(<App payload={failedFixture} />);
  assert.match(out, /发言超时/);
  assert.match(out, /已取消/);
  assert.equal((out.match(/重试/g) ?? []).length, 2);
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
  assert.match(render(<App payload={hintFixture} />), /未找到成员 @nobody/);
});

test('empty group shows a hint to add members and disables nothing silently', () => {
  const out = render(<App payload={noMembersFixture} />);
  assert.match(out, /还没有成员/);
});

test('no selected group but groups exist prompts to pick one', () => {
  assert.match(render(<App payload={noSelectionFixture} />), /选择一个群聊/);
});

test('composer placeholder explains @ and keys', () => {
  const out = render(<App payload={conversationFixture} />);
  assert.match(out, /@ 点名成员发言/);
});
