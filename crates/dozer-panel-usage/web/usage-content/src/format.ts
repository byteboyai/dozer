// 用量面板数字的统一样式,移植自 chart.rs::format_count:
// - < 1000:原样。
// - [1000, 1e6):除以 1000,1 位小数,'k' 后缀(1000 直接进 1.0k,不出现
//   1000.0k 这种中间档)。
// - >= 1e6:除以 1e6,1 位小数,'m' 后缀。
export function formatCount(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'm';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return String(n);
}

// "nice numbers" 刻度步长算法,移植自 chart.rs::nice_tick_step:按数量级
// 取 1/2/5/10 里最接近 raw_step 的一档,让刻度总落在整数上(不是简单
// max/target 等分,那样步长会是 733 这种没法一眼读的零头)。
export function niceTickStep(maxValue: number, targetTicks: number): number {
  const rawStep = maxValue / Math.max(targetTicks, 1);
  if (rawStep <= 0) return 1;
  const magnitude = Math.pow(10, Math.floor(Math.log10(rawStep)));
  const residual = rawStep / magnitude;
  let niceResidual: number;
  if (residual <= 1) niceResidual = 1;
  else if (residual <= 2) niceResidual = 2;
  else if (residual <= 5) niceResidual = 5;
  else niceResidual = 10;
  return Math.max(Math.round(niceResidual * magnitude), 1);
}

const GRID_TARGET_TICKS = 4;

// 从一个步长的整数倍往上数,数到 maxTotal 为止的刻度值(不含 0 基线),
// 移植自 chart.rs::grid_ticks。
export function gridTicks(maxTotal: number): number[] {
  if (maxTotal === 0) return [];
  const step = niceTickStep(maxTotal, GRID_TARGET_TICKS);
  const ticks: number[] = [];
  for (let v = step; v <= maxTotal; v += step) ticks.push(v);
  if (ticks.length === 0) ticks.push(maxTotal);
  return ticks;
}
