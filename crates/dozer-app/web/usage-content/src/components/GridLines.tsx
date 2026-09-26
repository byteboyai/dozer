import { gridTicks, formatCount } from '../format.ts';

export const BAR_MAX_HEIGHT = 72;
export const BAR_WIDTH = 14;
export const BAR_LABEL_GAP = 14;
export const GRID_CANVAS_HEIGHT = BAR_MAX_HEIGHT + BAR_LABEL_GAP; // 86
export const DAY_BAND_HEIGHT = GRID_CANVAS_HEIGHT + 4 + 12; // 102
export const GRID_LABEL_GUTTER = 26;

// value → 网格画布内的 y 坐标(0 基线落在画布最底部),同 chart.rs 的
// GridLines::draw / TrendLines::draw 共用换算。
export function yFor(value: number, maxTotal: number): number {
  if (maxTotal === 0) return GRID_CANVAS_HEIGHT;
  return GRID_CANVAS_HEIGHT - (value / maxTotal) * BAR_MAX_HEIGHT;
}

export function GridLines({ maxTotal }: { maxTotal: number }) {
  if (maxTotal === 0) return null;
  return (
    <>
      {gridTicks(maxTotal).map((tick) => (
        <div class="usage-grid-line" style={{ top: `${yFor(tick, maxTotal)}px` }} key={tick}>
          <span class="usage-grid-label">{formatCount(tick)}</span>
        </div>
      ))}
    </>
  );
}
