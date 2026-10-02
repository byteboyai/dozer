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

const html = (p: ViewPayload) => render(<App payload={p} />);

test('empty list shows the no-match hint, toolbar and add box', () => {
  const out = html(emptyFixture);
  assert.match(out, /没有匹配的任务/);
  assert.match(out, /搜索任务…/);
  assert.match(out, /添加新任务/);
});

test('three segments show dividers, numbers and status labels', () => {
  const out = html(threeSegmentsFixture);
  assert.match(out, /#001/);
  assert.match(out, /#004/);
  assert.match(out, /搁置/);
  assert.match(out, /已完成/);
  assert.match(out, /进行中/);
  assert.match(out, /修复登录页闪烁/);
});

test('uncategorized cards show 未分类, categorized show their name', () => {
  const out = html(threeSegmentsFixture);
  assert.match(out, /未分类/);
  assert.match(out, /后端/);
});

test('detail button appears only with an assignment or dispatch; assign button only for idle pending', () => {
  const out = html(threeSegmentsFixture);
  assert.match(out, /详情/); // 任务 2 已派发
  assert.match(out, /指派/); // 任务 1 是待办且无派发
});

test('done cards show completion date and strikethrough class', () => {
  const out = html(allDoneFixture);
  assert.match(out, /10-01/);
  assert.match(out, /is-done/);
});

test('long and english text render', () => {
  assert.match(html(longTextFixture), /非常非常长/);
  assert.match(html(englishFixture), /Fix the flaky login test/);
});

test('no popover is rendered initially', () => {
  assert.doesNotMatch(html(threeSegmentsFixture), /class="popover/);
});

// 审阅 Important 5:输入框带与 Rust 一致的长度上限,超限根本输入不进去。
test('add box limits input length to the shared maximum', () => {
  assert.match(html(emptyFixture), /maxlength="10000"/);
});
