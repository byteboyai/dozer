export type CategoryKey =
  | 'overview'
  | 'structure'
  | 'ui_consistency'
  | 'scan_scope'
  | 'architecture';
export type SeverityKey = 'critical' | 'watch';
export type ChangeKey = 'new' | 'worsened' | 'improved' | 'persisting' | 'resolved';
export type TierKey = 'healthy' | 'watch' | 'critical';

export type ErrorDto =
  | { kind: 'path_unavailable' }
  | { kind: 'scan_cancelled'; message: string }
  | { kind: 'scan_failed'; message: string }
  | { kind: 'save_failed'; message: string };

export interface ScanBar {
  scanning: boolean;
  scanned_at: string | null;
  scan_error: string | null;
  scan_error_key: ErrorDto | null;
  save_error: string | null;
  save_error_key: ErrorDto | null;
}

export type SkipReasonKey =
  | 'ignored'
  | 'unsupported_language'
  | 'generated'
  | 'non_utf8'
  | 'read_failed'
  | 'parse_failed';
export type ScanStatusKey = 'complete' | 'partial' | 'failed';
export type EmptyStateKey =
  | 'scanning'
  | 'never_scanned'
  | 'no_supported_code'
  | 'partial_failure'
  | 'scan_failed';

export type ReasonDto =
  | { kind: 'metric'; delta: number; unit: string }
  | { kind: 'recent_commits'; count: number };

export interface FindingRow {
  id: string;
  title: string;
  path: string;
  line: number;
  severity: SeverityKey;
  severity_label: string;
  change: ChangeKey | null;
  change_label: string | null;
  reasons: string[];
  /** 结构化成因(spec §167,可能缺省 = 旧 Host);优先用它本地化。 */
  reason_items?: ReasonDto[];
  /** 标题 key(如架构风险 `arch.risk.cycle`);优先用它本地化。 */
  title_key?: string | null;
  /** 结构复杂度行的函数名与指标值。 */
  symbol?: string | null;
  metric?: number | null;
}

export type ChangeDto =
  | { kind: 'first_scan' }
  | {
      kind: 'has_change';
      loc_delta_text: string;
      functions_delta_text: string;
      new_risks: number;
      resolved_risks: number;
      worsened: number;
      improved: number;
      loc_delta?: number;
      functions_delta?: number;
    };

export interface GitLineDto {
  branch: string | null;
  head: string | null;
  dirty: boolean;
}

export interface SummaryDto {
  loc: number;
  functions: number;
}

export interface ScopeSummaryDto {
  analyzed_files: number;
  excluded_files: number;
  skipped_files: number;
  languages: string;
}

export interface OverviewBody {
  kind: 'overview';
  git_line: string | null;
  git?: GitLineDto | null;
  tier: TierKey;
  tier_label: string;
  summary: string;
  summary_parts?: SummaryDto;
  change: ChangeDto;
  priorities: FindingRow[];
  scope_line: string;
  scope_parts?: ScopeSummaryDto | null;
  legacy_note: string | null;
}

export interface StructureBody {
  kind: 'structure';
  metric_note: string;
  findings: FindingRow[];
}

export interface UiGroup {
  title: string;
  group_key?: string;
  findings: FindingRow[];
}

export interface UiBody {
  kind: 'ui_consistency';
  applicable: boolean;
  groups: UiGroup[];
}

export interface SkippedDto {
  path: string;
  reason: string;
  reason_key?: SkipReasonKey;
}

export interface LanguageDto {
  language: string;
  files: number;
  analyzed: boolean;
}

export interface ScopeBody {
  kind: 'scan_scope';
  status_label: string;
  status?: ScanStatusKey;
  analyzed_files: number;
  excluded_files: number;
  skipped_count: number;
  languages_detail: string;
  languages?: LanguageDto[];
  duration_ms: number;
  schema_version: number;
  git_baseline: string | null;
  skipped: SkippedDto[];
}

export interface EmptyBody {
  kind: 'empty';
  message: string;
  empty_state?: EmptyStateKey;
}

export interface ArchNode {
  id: string;
  kind: 'workspace' | 'crate' | 'module' | 'external_crate';
  name: string;
  qualified_name: string;
  path: string | null;
  parent_id: string | null;
  loc: number;
  fan_in: number;
  fan_out: number;
  layer: string | null;
}

export interface ArchEdge {
  id: string;
  from: string;
  to: string;
  kind: 'cargo_dependency' | 'module_use';
  evidence_count: number;
}

export interface ArchRisk {
  kind: 'cycle' | 'hub' | 'boundary';
  finding: FindingRow;
  node_ids: string[];
  edge_ids: string[];
}

export interface ArchImpactNode {
  node_id: string;
  distance: number;
}

export interface ArchitectureBody {
  kind: 'architecture';
  status: 'complete' | 'partial' | 'not_applicable';
  status_note: string | null;
  status_note_key?: string | null;
  truncated_note: string | null;
  truncated_note_total?: number | null;
  nodes: ArchNode[];
  edges: ArchEdge[];
  cycles: { id: string; node_ids: string[]; edge_ids: string[] }[];
  risks: ArchRisk[];
  diff: {
    state: 'no_baseline' | 'unavailable' | 'compared';
    added_nodes: string[];
    removed_nodes: string[];
    added_edges: string[];
    removed_edges: string[];
    added_cycles: string[];
    resolved_cycles: string[];
  };
  impact: { direct: ArchImpactNode[]; indirect: ArchImpactNode[]; truncated: boolean };
  errors: string[];
  unresolved_edges: number;
}

export type Body =
  | EmptyBody
  | OverviewBody
  | StructureBody
  | UiBody
  | ScopeBody
  | ArchitectureBody;

export interface ViewPayload {
  scan: ScanBar;
  category: CategoryKey;
  body: Body;
}

/** 前端 → Rust(与 `CodeHealthWebviewEvent` 的 serde 形状一致)。 */
export type OutEvent =
  | { kind: 'ready' }
  | { kind: 'scan_requested' }
  | { kind: 'open_location'; path: string; line: number }
  | { kind: 'analyze_finding'; id: string }
  | { kind: 'failed'; reason: string };
