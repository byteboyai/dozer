import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldReportError } from './errors.ts';

test('benign ResizeObserver notifications are not fatal', () => {
  assert.equal(
    shouldReportError({ message: 'ResizeObserver loop completed with undelivered notifications.' }),
    false,
  );
  assert.equal(shouldReportError({ message: 'ResizeObserver loop limit exceeded' }), false);
});

test('opaque cross-origin "Script error." is not fatal', () => {
  assert.equal(shouldReportError({ message: 'Script error.' }), false);
});

test('a real Error thrown by our code is fatal', () => {
  assert.equal(shouldReportError({ message: 'x is undefined', error: new TypeError('x') }), true);
});

test('an error attributed to our bundle is fatal even without an Error object', () => {
  assert.equal(
    shouldReportError({
      message: 'boom',
      filename: 'dozer://codehealth-content/codehealth-content.js',
    }),
    true,
  );
});

test('an error with no error object and no origin is ignored', () => {
  assert.equal(shouldReportError({ message: 'something' }), false);
});
