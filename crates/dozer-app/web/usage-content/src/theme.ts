import type { AgentKind, TrendChartKind } from './types.ts';

// 移植自 workspace/hook.rs::agent_dot_color——每个 agent 固定映射到一个
// 主题 CSS 变量名(见 Task 1 styles.css 的 :root[data-theme] 定义),不是
// 写死的十六进制,明暗切换时自动跟随。
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
  label: string;
  colorVar: string;
}

// 移植自 chart.rs::session_trend_series/io_trend_series/cache_trend_series
// 与 view.rs 里"触达文件/Git提交"的配色(c.cream/c.green)。下标与
// Rust 侧 `TrendDay.values[i]` 一一对应,不能重排。
export const SERIES_META: Record<TrendChartKind, SeriesMeta[]> = {
  session_round: [
    { label: '会话', colorVar: '--cream' },
    { label: '回合', colorVar: '--green' },
  ],
  io_tokens: [
    { label: 'Input', colorVar: '--cyan' },
    { label: 'Output', colorVar: '--purple' },
  ],
  cache_tokens: [
    { label: '读', colorVar: '--lime' },
    { label: '写', colorVar: '--green' },
  ],
  behavior: [
    { label: '触达文件', colorVar: '--cream' },
    { label: 'Git提交', colorVar: '--green' },
  ],
};
