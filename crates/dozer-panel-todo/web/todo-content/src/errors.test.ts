import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldReportError } from './errors.ts';

test('benign ResizeObserver notice is not fatal', () => {
  assert.equal(shouldReportError({ message: 'ResizeObserver loop limit exceeded' }), false);
});

test('opaque cross-origin Script error is not fatal', () => {
  assert.equal(shouldReportError({ message: 'Script error.' }), false);
});

test('an Error thrown by our code is fatal', () => {
  assert.equal(shouldReportError({ message: 'boom', error: new Error('boom') }), true);
});

test('error attributed to our bundle is fatal', () => {
  assert.equal(
    shouldReportError({ message: 'x', filename: 'dozer://todo-content/todo-content.js' }),
    true,
  );
});

test('error from elsewhere without an Error object is ignored', () => {
  assert.equal(shouldReportError({ message: 'x', filename: 'https://example.com/a.js' }), false);
});
