export const pad2 = (n: number): string => String(n).padStart(2, '0');

export function daysInMonth(y: number, m: number): number {
  return new Date(y, m, 0).getDate();
}

/** 周日为 0(同原生 `first_weekday_of_month`)。 */
export function firstWeekday(y: number, m: number): number {
  return new Date(y, m - 1, 1).getDay();
}

/** 拍平的月历格:前导 null 补到该月 1 日的星期,末尾补满整周。 */
export function monthGrid(y: number, m: number): (number | null)[] {
  const cells: (number | null)[] = Array(firstWeekday(y, m)).fill(null);
  for (let d = 1; d <= daysInMonth(y, m); d++) cells.push(d);
  while (cells.length % 7 !== 0) cells.push(null);
  return cells;
}

export function shiftMonth(y: number, m: number, delta: number): { y: number; m: number } {
  const idx = y * 12 + (m - 1) + delta;
  return { y: Math.floor(idx / 12), m: (idx % 12) + 1 };
}

export function formatMonthDay(m: number, d: number): string {
  return `${pad2(m)}-${pad2(d)}`;
}

export function parseMonthDay(s: string | null): { m: number; d: number } | null {
  if (!s) return null;
  const parts = s.split('-');
  if (parts.length !== 2) return null;
  const m = Number(parts[0]);
  const d = Number(parts[1]);
  if (!Number.isInteger(m) || !Number.isInteger(d) || m < 1 || m > 12 || d < 1 || d > 31) return null;
  return { m, d };
}
