/** 耗时的简短中文显示。 */
export function formatDuration(ms: number | null): string {
  if (ms === null) return '';
  if (ms < 1000) return '<1 秒';
  const s = ms / 1000;
  if (s < 10) return `${Math.round(s * 10) / 10} 秒`;
  if (s < 60) return `${Math.round(s)} 秒`;
  const m = Math.floor(s / 60);
  const rest = Math.round(s - m * 60);
  return rest === 0 ? `${m} 分` : `${m} 分 ${rest} 秒`;
}
