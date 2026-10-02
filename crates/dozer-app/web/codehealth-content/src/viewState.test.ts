import test from 'node:test';
import assert from 'node:assert/strict';
import { readViewState, writeViewState, resetViewState } from './viewState.ts';

test('returns the initial value when nothing was stored', () => {
  resetViewState();
  assert.equal(readViewState('structure.mode', 'new'), 'new');
});

// 修复:分类切换会卸载页面组件,视图状态必须存在组件之外。
test('stored value survives and is returned on the next read', () => {
  resetViewState();
  writeViewState('structure.mode', 'all');
  assert.equal(readViewState('structure.mode', 'new'), 'all');
});

test('keys are independent', () => {
  resetViewState();
  writeViewState('a', 1);
  assert.equal(readViewState('b', 2), 2);
});
