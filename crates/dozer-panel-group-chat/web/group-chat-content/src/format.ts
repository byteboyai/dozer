import { t } from './i18n.ts';

/** 耗时的简短显示。 */
export function formatDuration(ms: number | null): string {
  if (ms === null) return '';
  if (ms < 1000) return t('duration.lt_second', '<1 s');
  const s = ms / 1000;
  if (s < 10) {
    return t('duration.seconds_precise', '{seconds} s', {
      seconds: String(Math.round(s * 10) / 10),
    });
  }
  if (s < 60) {
    return t('duration.seconds', '{seconds} s', {
      seconds: String(Math.round(s)),
    });
  }
  const m = Math.floor(s / 60);
  const rest = Math.round(s - m * 60);
  if (rest === 0) return t('duration.minutes', '{minutes} min', { minutes: String(m) });
  return t('duration.minutes_seconds', '{minutes} min {seconds} s', {
    minutes: String(m),
    seconds: String(rest),
  });
}

/**
 * 发言中的阶段文案。后端是一次性无头调用、中途没有进度事件,所以这里按"已等待时长"
 * 推断阶段(不是真实事件),目的是让几十秒的等待有反馈。
 */
export function runningPhase(elapsedMs: number): string {
  const s = elapsedMs / 1000;
  if (s < 4) return t('phase.connecting', 'Connecting to the model');
  if (s < 25) return t('phase.thinking', 'Thinking');
  if (s < 60) return t('phase.thinking_long', 'Still thinking; the content is lengthy');
  return t('phase.waiting', 'Taking a while; still waiting for the model');
}

/** 等待秒数,如 `12 s`、`1 min 05 s`。 */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) {
    return t('duration.seconds', '{seconds} s', { seconds: String(s) });
  }
  return t('elapsed.minutes_seconds', '{minutes} min {seconds} s', {
    minutes: String(Math.floor(s / 60)),
    seconds: String(s % 60).padStart(2, '0'),
  });
}
