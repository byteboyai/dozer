import type { DailyUsageChart as DailyUsageChartType } from '../types.ts';
import { formatCount } from '../format.ts';
import { agentColorVar } from '../theme.ts';
import { GridLines, BAR_MAX_HEIGHT, GRID_LABEL_GUTTER } from './GridLines.tsx';

// agent 展示名,同 ChartStatList.tsx 的 AGENT_LABEL(逐一对应
// AgentKind::label())。
const AGENT_LABEL: Record<string, string> = {
  claude: 'claude',
  codebuddy: 'codebuddy',
  opencode: 'opencode',
  codex: 'codex',
  goose: 'goose',
  aider: 'aider',
  v8agent: 'v8agent',
  unknown: 'unknown',
};

export function DailyUsageChart({ chart }: { chart: DailyUsageChartType }) {
  const maxTotal = Math.max(1, ...chart.days.flatMap((d) => d.totals));
  const n = chart.days.length;
  return (
    <div class="usage-chart-wrap" style={{ height: '102px', marginLeft: `${GRID_LABEL_GUTTER}px` }}>
      <div class="usage-grid-overlay" style={{ marginLeft: `-${GRID_LABEL_GUTTER}px` }}>
        <GridLines maxTotal={maxTotal} />
      </div>
      <div class={`usage-bar-groups ${n === 1 ? 'usage-bar-groups-center' : ''}`}>
        {chart.days.map((day, i) => (
          <div
            class="usage-day-band"
            style={{ background: i % 2 === 0 ? 'var(--card)' : 'transparent' }}
            key={day.label}
          >
            <div class="usage-tooltip">
              <div class="day-label">{day.label}</div>
              {chart.agents.map(
                (agent, ai) =>
                  day.totals[ai] > 0 && (
                    <div class="usage-legend-row" key={agent}>
                      <span class="usage-legend-dot" style={{ background: agentColorVar(agent) }} />
                      <span class="usage-legend-text">
                        {AGENT_LABEL[agent]} {formatCount(day.totals[ai])}
                      </span>
                    </div>
                  ),
              )}
            </div>
            <div class="usage-bar-row">
              {chart.agents.map((agent, ai) => {
                const value = day.totals[ai];
                const height = Math.max((value / maxTotal) * BAR_MAX_HEIGHT, 1);
                return (
                  <div class="usage-bar-col" key={agent}>
                    <span class="usage-bar-value">{formatCount(value)}</span>
                    <div
                      class="usage-bar"
                      style={{ height: `${height}px`, background: agentColorVar(agent) }}
                    />
                  </div>
                );
              })}
            </div>
            <span class="usage-day-label">{day.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
