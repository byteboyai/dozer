import type { TrendChart, TokenTrendSection as TokenTrendSectionType } from '../types.ts';
import { formatCount } from '../format.ts';
import { SERIES_META } from '../theme.ts';
import { TrendLineChart } from './TrendLineChart.tsx';

function InlineLegend({ chartKind }: { chartKind: TrendChart['kind'] }) {
  return (
    <div class="usage-inline-legend">
      {SERIES_META[chartKind].map((s) => (
        <div class="usage-legend-row" key={s.label}>
          <span class="usage-legend-dot" style={{ background: `var(${s.colorVar})` }} />
          <span class="usage-legend-text">{s.label}</span>
        </div>
      ))}
    </div>
  );
}

export function TrendSection({ title, chart }: { title: string; chart: TrendChart }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head-row">
        <div class="usage-section-head">{title}</div>
        <InlineLegend chartKind={chart.kind} />
      </div>
      <TrendLineChart chart={chart} />
    </div>
  );
}

// 对应 chart.rs::token_trend_section:一个"Token 趋势"标题下,IO 与 Cache
// 两张独立子图各自独立纵轴 max,任一缺失只画另一张,都缺失时上层
// (App.tsx)不渲染这个 section。
export function TokenTrendSectionView({ section }: { section: TokenTrendSectionType }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head">Token 趋势</div>
      {section.io && (
        <>
          <div class="usage-section-head-row">
            <span class="usage-trend-window-tag">
              Input/Output({formatCount(section.io_total)}/{section.io.days.length}days)
            </span>
            <InlineLegend chartKind="io_tokens" />
          </div>
          <TrendLineChart chart={section.io} />
        </>
      )}
      {section.cache && (
        <div class={section.io ? 'usage-subsection-gap' : ''}>
          <div class="usage-section-head-row">
            <span class="usage-trend-window-tag">
              Cache read/write({formatCount(section.cache_total)}/{section.cache.days.length}days)
            </span>
            <InlineLegend chartKind="cache_tokens" />
          </div>
          <TrendLineChart chart={section.cache} />
        </div>
      )}
    </div>
  );
}
