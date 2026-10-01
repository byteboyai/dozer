export type CategoryKey =
  | 'overview'
  | 'structure'
  | 'ui_consistency'
  | 'scan_scope'
  | 'architecture';
export type SeverityKey = 'critical' | 'watch';
export type ChangeKey = 'new' | 'worsened' | 'improved' | 'persisting' | 'resolved';
export type TierKey = 'healthy' | 'watch' | 'critical';

export interface ScanBar {
  scanning: boolean;
  scanned_at: string | null;
  scan_error: string | null;
  save_error: string | null;
}

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
    };

export interface OverviewBody {
  kind: 'overview';
  git_line: string | null;
  tier: TierKey;
  tier_label: string;
  summary: string;
  change: ChangeDto;
  priorities: FindingRow[];
  scope_line: string;
  legacy_note: string | null;
}

export interface StructureBody {
  kind: 'structure';
  metric_note: string;
  findings: FindingRow[];
}

export interface UiGroup {
  title: string;
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
}

export interface ScopeBody {
  kind: 'scan_scope';
  status_label: string;
  analyzed_files: number;
  excluded_files: number;
  skipped_count: number;
  languages_detail: string;
  duration_ms: number;
  schema_version: number;
  git_baseline: string | null;
  skipped: SkippedDto[];
}

export interface EmptyBody {
  kind: 'empty';
  message: string;
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
  truncated_note: string | null;
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
