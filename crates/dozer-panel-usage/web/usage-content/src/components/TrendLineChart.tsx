import type { TrendChart } from '../types.ts';
import { formatCount } from '../format.ts';
import { SERIES_META } from '../theme.ts';
import { GridLines, yFor, GRID_CANVAS_HEIGHT, GRID_LABEL_GUTTER } from './GridLines.tsx';

export function TrendLineChart({ chart }: { chart: TrendChart }) {
  const days = chart.days;
  const n = Math.max(days.length, 1);
  const series = SERIES_META[chart.kind];
  const maxTotal = Math.max(1, ...days.flatMap((d) => d.values));

  return (
    <div class="usage-chart-wrap" style={{ height: '86px' }}>
      <GridLines maxTotal={maxTotal} />
      <div class="usage-trend-svg-wrap" style={{ marginLeft: `${GRID_LABEL_GUTTER}px` }}>
        <svg
          width="100%"
          height={GRID_CANVAS_HEIGHT}
          viewBox={`0 0 100 ${GRID_CANVAS_HEIGHT}`}
          preserveAspectRatio="none"
        >
          {series.map((s, idx) => (
            <polyline
              key={s.label}
              points={days
                .map((d, i) => `${(100 * (i + 0.5)) / n},${yFor(d.values[idx], maxTotal)}`)
                .join(' ')}
              fill="none"
              stroke={`var(${s.colorVar})`}
              stroke-width="2"
              vector-effect="non-scaling-stroke"
            />
          ))}
        </svg>
        {series.flatMap((s, idx) =>
          days.map((d, i) => (
            <div
              key={`${s.label}-${i}`}
              class="usage-trend-dot"
              style={{
                left: `${((i + 0.5) / n) * 100}%`,
                top: `${yFor(d.values[idx], maxTotal)}px`,
                background: `var(${s.colorVar})`,
              }}
            />
          )),
        )}
        <div class="usage-trend-cells">
          {days.map((d, i) => (
            <div class="usage-trend-cell" key={i}>
              <div class="usage-hover-line" />
              <div class="usage-tooltip">
                <div class="day-label">{d.label}</div>
                {series.map(
                  (s, idx) =>
                    d.values[idx] > 0 && (
                      <div class="usage-legend-row" key={s.label}>
                        <span class="usage-legend-dot" style={{ background: `var(${s.colorVar})` }} />
                        <span class="usage-legend-text">
                          {s.label}: {formatCount(d.values[idx])}
                        </span>
                      </div>
                    ),
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
      <div class="usage-trend-day-labels" style={{ marginLeft: `${GRID_LABEL_GUTTER}px` }}>
        {days.map((d) => (
          <span class="usage-day-label" key={d.label}>
            {d.label}
          </span>
        ))}
      </div>
    </div>
  );
}
