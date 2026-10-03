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

/**
 * 发言中的阶段文案。后端是一次性无头调用、中途没有进度事件,所以这里按"已等待时长"
 * 推断阶段(不是真实事件),目的是让几十秒的等待有反馈。
 */
export function runningPhase(elapsedMs: number): string {
  const s = elapsedMs / 1000;
  if (s < 4) return '正在连接模型';
  if (s < 25) return '思考中';
  if (s < 60) return '仍在思考,内容较多';
  return '耗时较长,仍在等待模型返回';
}

/** 等待秒数,如 `12 秒`、`1 分 05 秒`。 */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} 秒`;
  return `${Math.floor(s / 60)} 分 ${String(s % 60).padStart(2, '0')} 秒`;
}
