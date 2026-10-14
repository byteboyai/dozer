import type { AgentKind, TrendChartKind } from './types.ts';

// 每个 agent 固定映射到一个主题 CSS 变量名(见 Task 1 styles.css 的
// :root[data-theme] 定义),不是写死的十六进制,明暗切换时自动跟随。
// Rust 侧同源映射由产品的 `PanelDisplay::agent_icon_color` 提供(Dozer 移植自
// 其 `workspace/hook.rs::agent_dot_color`),两边须一致,见 theme.test.ts。
export const AGENT_COLOR_VAR: Record<AgentKind, string> = {
  claude: '--cyan',
  codebuddy: '--purple',
  opencode: '--green',
  codex: '--orange',
  goose: '--blue',
  aider: '--magenta',
  v8agent: '--lime',
  unknown: '--dim',
};

export function agentColorVar(agent: AgentKind): string {
  return `var(${AGENT_COLOR_VAR[agent]})`;
}

export interface SeriesMeta {
  // 资源 key(查 `dozer.usage` 语言包取展示文案);渲染端经 `seriesLabel` 解析,
  // 不在模块级固定语言,见 i18n.ts。
  labelKey: string;
  colorVar: string;
}

// 移植自 chart.rs::session_trend_series/io_trend_series/cache_trend_series
// 与 view.rs 里"触达文件/Git提交"的配色(c.cream/c.green)。下标与
// Rust 侧 `TrendDay.values[i]` 一一对应,不能重排。
export const SERIES_META: Record<TrendChartKind, SeriesMeta[]> = {
  session_round: [
    { labelKey: 'series.session', colorVar: '--cream' },
    { labelKey: 'series.turns', colorVar: '--green' },
  ],
  io_tokens: [
    { labelKey: 'series.input', colorVar: '--cyan' },
    { labelKey: 'series.output', colorVar: '--purple' },
  ],
  cache_tokens: [
    { labelKey: 'series.cache_read', colorVar: '--lime' },
    { labelKey: 'series.cache_write', colorVar: '--green' },
  ],
  behavior: [
    { labelKey: 'series.files_touched', colorVar: '--cream' },
    { labelKey: 'series.git_commits', colorVar: '--green' },
  ],
};
