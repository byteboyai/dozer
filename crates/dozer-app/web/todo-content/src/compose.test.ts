import test from 'node:test';
import assert from 'node:assert/strict';
import { isAddSubmit, isInlineCommit, isCancel } from './compose.ts';

const key = (over: object) => ({
  key: 'Enter', metaKey: false, ctrlKey: false, shiftKey: false, isComposing: false, ...over,
});

// Review Focus 2:输入法组合输入期间绝不提交。
test('add box: cmd/ctrl+enter submits, plain enter does not', () => {
  assert.equal(isAddSubmit(key({ metaKey: true })), true);
  assert.equal(isAddSubmit(key({ ctrlKey: true })), true);
  assert.equal(isAddSubmit(key({})), false);
});

test('add box: never submits while composing', () => {
  assert.equal(isAddSubmit(key({ metaKey: true, isComposing: true })), false);
  assert.equal(isAddSubmit(key({ metaKey: true, keyCode: 229 })), false);
});

test('inline edit: enter commits, but not while composing', () => {
  assert.equal(isInlineCommit(key({})), true);
  assert.equal(isInlineCommit(key({ isComposing: true })), false);
  assert.equal(isInlineCommit(key({ keyCode: 229 })), false);
  assert.equal(isInlineCommit(key({ key: 'a' })), false);
});

test('escape cancels, but not while composing', () => {
  assert.equal(isCancel(key({ key: 'Escape' })), true);
  assert.equal(isCancel(key({ key: 'Escape', isComposing: true })), false);
});
