import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import {
  emptyFixture,
  threeSegmentsFixture,
  allDoneFixture,
  longTextFixture,
  englishFixture,
} from './fixtures.ts';
import type { ViewPayload } from './types.ts';
import { setLocale } from './i18n.ts';

const html = (p: ViewPayload) => render(<App payload={p} />);

test('empty list shows the no-match hint, toolbar and add box', () => {
  setLocale('en');
  const out = html(emptyFixture);
  assert.match(out, /No matching tasks/);
  assert.match(out, /Search tasks\.\.\./);
  assert.match(out, /Add a new task/);
});

test('three segments show dividers, numbers and status labels', () => {
  setLocale('en');
  const out = html(threeSegmentsFixture);
  assert.match(out, /#001/);
  assert.match(out, /#004/);
  assert.match(out, /Suspended/);
  assert.match(out, /Done/);
  assert.match(out, /In progress/);
  assert.match(out, /修复登录页闪烁/);
});

test('uncategorized cards show the placeholder, categorized show their name', () => {
  setLocale('en');
  const out = html(threeSegmentsFixture);
  assert.match(out, /Uncategorized/);
  assert.match(out, /后端/);
});

test('detail button appears only with an assignment or dispatch; assign button only for idle pending', () => {
  setLocale('en');
  const out = html(threeSegmentsFixture);
  assert.match(out, /Details/); // 任务 2 已派发
  assert.match(out, /Assign/); // 任务 1 是待办且无派发
});

test('done cards show completion date and strikethrough class', () => {
  setLocale('en');
  const out = html(allDoneFixture);
  assert.match(out, /10-01/);
  assert.match(out, /is-done/);
});

test('long and english text render', () => {
  setLocale('en');
  assert.match(html(longTextFixture), /非常非常长/);
  assert.match(html(englishFixture), /Fix the flaky login test/);
});

test('no popover is rendered initially', () => {
  setLocale('en');
  assert.doesNotMatch(html(threeSegmentsFixture), /class="popover/);
});

// 审阅 Important 5:输入框带与 Rust 一致的长度上限,超限根本输入不进去。
test('add box limits input length to the shared maximum', () => {
  setLocale('en');
  assert.match(html(emptyFixture), /maxlength="10000"/);
});

// spec §4.3:切换语言后重渲染即换文案(用户任务正文保持原文)。
test('chrome switches to zh-CN while user text stays verbatim', () => {
  setLocale('zh-CN');
  const out = html(threeSegmentsFixture);
  assert.match(out, /搁置/);
  assert.match(out, /已完成/);
  assert.match(out, /未分类/);
  assert.match(out, /详情/);
  assert.match(out, /指派/);
  assert.match(out, /修复登录页闪烁/); // 用户任务正文不变
  setLocale('en');
});
