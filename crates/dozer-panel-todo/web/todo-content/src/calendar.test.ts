import test from 'node:test';
import assert from 'node:assert/strict';
import { daysInMonth, firstWeekday, monthGrid, shiftMonth, formatMonthDay, parseMonthDay } from './calendar.ts';

test('daysInMonth handles leap years', () => {
  assert.equal(daysInMonth(2024, 2), 29);
  assert.equal(daysInMonth(2023, 2), 28);
  assert.equal(daysInMonth(2100, 2), 28);
  assert.equal(daysInMonth(2000, 2), 29);
  assert.equal(daysInMonth(2026, 10), 31);
  assert.equal(daysInMonth(2026, 11), 30);
});

test('firstWeekday is Sunday-based (1970-01-01 was a Thursday)', () => {
  assert.equal(firstWeekday(1970, 1), 4);
  assert.equal(firstWeekday(2026, 10), 4);
});

test('monthGrid pads leading blanks and lists every day', () => {
  const g = monthGrid(2026, 10);
  assert.equal(g.slice(0, 4).every((c) => c === null), true);
  assert.equal(g[4], 1);
  assert.equal(g.filter((c) => c !== null).length, 31);
  assert.equal(g.length % 7, 0);
});

test('shiftMonth wraps the year', () => {
  assert.deepEqual(shiftMonth(2026, 1, -1), { y: 2025, m: 12 });
  assert.deepEqual(shiftMonth(2026, 12, 1), { y: 2027, m: 1 });
  assert.deepEqual(shiftMonth(2026, 5, 1), { y: 2026, m: 6 });
});

test('MM-DD formatting and parsing round trip', () => {
  assert.equal(formatMonthDay(9, 5), '09-05');
  assert.deepEqual(parseMonthDay('09-05'), { m: 9, d: 5 });
  assert.equal(parseMonthDay('13-01'), null);
  assert.equal(parseMonthDay('abc'), null);
  assert.equal(parseMonthDay(null), null);
});
