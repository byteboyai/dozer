export type AgentKind =
  | 'unknown'
  | 'claude'
  | 'codebuddy'
  | 'opencode'
  | 'codex'
  | 'goose'
  | 'aider'
  | 'v8agent';

export interface ProjectSummary {
  conversation_count: number;
  turns: number;
  tool_calls: number;
  files_touched: number;
  tokens_in: number;
  tokens_out: number;
  tokens_cache_read: number;
  tokens_cache_write: number;
}

export type TrendChartKind = 'session_round' | 'io_tokens' | 'cache_tokens' | 'behavior';

export interface TrendDay {
  label: string;
  values: number[];
}

export interface TrendChart {
  kind: TrendChartKind;
  days: TrendDay[];
}

export interface TokenTrendSection {
  io: TrendChart | null;
  io_total: number;
  cache: TrendChart | null;
  cache_total: number;
}

export interface AgentShare {
  agent: AgentKind;
  value: number;
}

export interface AgentMetrics {
  session_share: AgentShare[];
  turn_share: AgentShare[];
  io_share: AgentShare[];
  cache_share: AgentShare[];
  total_tokens: number;
}

export interface DailyUsageDay {
  label: string;
  /** 下标与外层 `DailyUsageChart.agents` 一一对应。 */
  totals: number[];
}

export interface DailyUsageChart {
  agents: AgentKind[];
  days: DailyUsageDay[];
}

// 注意:没有 `loading` 态——"统计中…"用的是 `byteui::feedback::math_curve`
// 动画组件(ByteBoy2077 品牌化的自绘曲线,多个面板共用),不值得单独在
// webview 里重做一套等价动画。加载中时 Rust 侧根本不挂载/推送这个
// webview,原生 iced 继续显示那个动画(同 Git Log diff pane"不可渲染
// 时回落原生占位"的先例,见 Task 13)。
export type UsageViewPayload =
  | { kind: 'empty' }
  | { kind: 'agent_empty'; agent: AgentKind }
  | {
      kind: 'single_agent';
      agent: AgentKind;
      project: ProjectSummary;
      git_commits: number;
      session_trend: TrendChart | null;
      token_trend: TokenTrendSection | null;
    }
  | {
      kind: 'all_agents';
      project: ProjectSummary;
      git_commits: number;
      agent_metrics: AgentMetrics | null;
      daily_usage: DailyUsageChart | null;
      daily_behavior: TrendChart | null;
    };
