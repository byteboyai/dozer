export type AgentLabel =
  | 'claude'
  | 'codebuddy'
  | 'opencode'
  | 'codex'
  | 'aider'
  | 'goose'
  | 'v8agent'
  | 'shell';

export interface ToolCall {
  summary: string;
  input_json?: string;
}

export interface ToolResult {
  content: string;
  is_error: boolean;
}

export interface AiTurnData {
  text?: string;
  thinking_text?: string;
  tool_calls?: ToolCall[];
  tool_results?: ToolResult[];
  tokens_in?: number;
  tokens_out?: number;
  tokens_cache_read?: number;
  tokens_cache_write?: number;
}

export type TraceEntry =
  | { Human: { text: string } }
  | { AiTurn: AiTurnData }
  | { ToolResult: ToolResult };

export interface TraceData {
  summary_title?: string;
  summary_time?: string;
  summary_text?: string;
  agent_label?: AgentLabel;
  entries: TraceEntry[];
}
