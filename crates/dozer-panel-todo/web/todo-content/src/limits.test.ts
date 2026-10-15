import test from 'node:test';
import assert from 'node:assert/strict';
import { MAX_TEXT_CHARS, fitsLimit, decideAdd, decideEdit } from './limits.ts';

// 必须与 Rust `extensions::todo::protocol::MAX_TEXT_CHARS` 一致(Rust 侧有同值断言)。
test('limit equals the Rust MAX_TEXT_CHARS', () => {
  assert.equal(MAX_TEXT_CHARS, 10_000);
});

// Rust 用 `chars().count()` 按码点数;emoji 在 JS 里占 2 个 UTF-16 单位,这里也按码点数。
test('fitsLimit counts code points, not UTF-16 units', () => {
  assert.equal(fitsLimit('a'.repeat(MAX_TEXT_CHARS)), true);
  assert.equal(fitsLimit('a'.repeat(MAX_TEXT_CHARS + 1)), false);
  assert.equal(fitsLimit('😀'.repeat(MAX_TEXT_CHARS)), true);
  assert.equal(fitsLimit('😀'.repeat(MAX_TEXT_CHARS + 1)), false);
});

// 审阅 Important 5:超长文本不能被静默丢弃,更不能在发送后把草稿清空。
test('add: empty draft sends nothing and keeps nothing', () => {
  assert.deepEqual(decideAdd('   \n'), { action: 'empty' });
});

test('add: over-limit draft is kept (not sent, not cleared)', () => {
  assert.deepEqual(decideAdd('字'.repeat(MAX_TEXT_CHARS + 1)), { action: 'keep' });
});

test('add: normal draft is sent trimmed', () => {
  assert.deepEqual(decideAdd('  新任务 \n'), { action: 'send', text: '新任务' });
});

test('edit: unchanged or empty text cancels without sending', () => {
  assert.deepEqual(decideEdit('原文', '原文'), { action: 'cancel' });
  assert.deepEqual(decideEdit('   ', '原文'), { action: 'cancel' });
});

test('edit: over-limit text keeps editing instead of dropping the change', () => {
  assert.deepEqual(decideEdit('字'.repeat(MAX_TEXT_CHARS + 1), '原文'), { action: 'keep' });
});

test('edit: changed text is sent trimmed', () => {
  assert.deepEqual(decideEdit(' 新文字 ', '原文'), { action: 'send', text: '新文字' });
});
