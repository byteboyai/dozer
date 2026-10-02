import type {
  ViewPayload,
  FindingRow,
  ScanBar,
  ArchitectureBody,
  ArchNode,
  ArchEdge,
} from './types.ts';

export const scanIdle: ScanBar = {
  scanning: false,
  scanned_at: '2026-10-01 08:00:00 UTC',
  scan_error: null,
  save_error: null,
};

export const row = (over: Partial<FindingRow> = {}): FindingRow => ({
  id: 'f1',
  title: 'run 控制流信号 12',
  path: 'src/a.rs',
  line: 7,
  severity: 'critical',
  severity_label: '警戒',
  change: 'new',
  change_label: '本轮新增',
  reasons: ['本轮新增', '近 30 天修改 3 次'],
  ...over,
});

export const emptyFixture: ViewPayload = {
  scan: { ...scanIdle, scanned_at: null },
  category: 'overview',
  body: { kind: 'empty', message: '这个项目还没有扫描过。' },
};

export const overviewFixture: ViewPayload = {
  scan: scanIdle,
  category: 'overview',
  body: {
    kind: 'overview',
    git_line: 'main · abc123 · 有未提交改动',
    tier: 'watch',
    tier_label: '需要关注',
    summary: '核心代码 100 行 · 1 个函数存在明显结构问题',
    change: {
      kind: 'has_change',
      loc_delta_text: '+12',
      functions_delta_text: '-1',
      new_risks: 2,
      resolved_risks: 1,
      worsened: 1,
      improved: 0,
    },
    priorities: [row(), row({ id: 'f2', title: '颜色硬编码', change: null, change_label: null, reasons: [] })],
    scope_line: '已分析 3 个文件 · Rust · 排除 0 · 跳过 0',
    legacy_note: null,
  },
};

export const overviewFirstScanFixture: ViewPayload = {
  ...overviewFixture,
  body: { ...(overviewFixture.body as object), change: { kind: 'first_scan' }, priorities: [] } as unknown as ViewPayload['body'],
};

export const overviewLegacyFixture: ViewPayload = {
  ...overviewFixture,
  body: {
    ...(overviewFixture.body as object),
    legacy_note: '旧版报告,重新扫描可查看变化与范围',
  } as unknown as ViewPayload['body'],
};

export const structureFixture: ViewPayload = {
  scan: scanIdle,
  category: 'structure',
  body: {
    kind: 'structure',
    metric_note: '指标口径:控制流信号(非标准圈复杂度)',
    findings: [row(), row({ id: 'f3', title: 'old 控制流信号 9', change: 'persisting', change_label: '持续存在' })],
  },
};

export const uiFixture: ViewPayload = {
  scan: scanIdle,
  category: 'ui_consistency',
  body: {
    kind: 'ui_consistency',
    applicable: true,
    groups: [
      { title: '颜色硬编码', findings: [row({ id: 'u1', title: '#ff0000', change: null, change_label: null, reasons: [] })] },
      { title: '边距硬编码', findings: [] },
    ],
  },
};

export const uiNotApplicableFixture: ViewPayload = {
  scan: scanIdle,
  category: 'ui_consistency',
  body: { kind: 'ui_consistency', applicable: false, groups: [] },
};

export const scopeFixture: ViewPayload = {
  scan: scanIdle,
  category: 'scan_scope',
  body: {
    kind: 'scan_scope',
    status_label: '部分完成',
    analyzed_files: 3,
    excluded_files: 1,
    skipped_count: 1,
    languages_detail: 'Rust(3 个文件,结构分析)',
    duration_ms: 420,
    schema_version: 3,
    git_baseline: 'main @ abc123',
    skipped: [{ path: 'src/bad.rs', reason: '解析失败' }],
  },
};

export const scanningWithOldResultFixture: ViewPayload = {
  ...overviewFixture,
  scan: { ...scanIdle, scanning: true },
};

export const scanFailedFixture: ViewPayload = {
  ...emptyFixture,
  scan: { ...scanIdle, scanned_at: null, scan_error: 'boom' },
  body: { kind: 'empty', message: '扫描失败。' },
};

const an = (id: string, kind: ArchNode['kind'], parent: string | null): ArchNode => ({
  id,
  kind,
  name: id.replace(/^(crate|module):/, ''),
  qualified_name: id.replace(/^(crate|module):/, ''),
  path: null,
  parent_id: parent,
  loc: 10,
  fan_in: 1,
  fan_out: 1,
  layer: null,
});
const ae = (kind: ArchEdge['kind'], from: string, to: string): ArchEdge => ({
  id: `edge:${kind}:${from}:${to}`,
  from,
  to,
  kind,
  evidence_count: 1,
});

export const archBody: ArchitectureBody = {
  kind: 'architecture',
  status: 'complete',
  status_note: null,
  truncated_note: null,
  nodes: [
    an('crate:a', 'crate', null),
    an('crate:b', 'crate', null),
    an('module:a', 'module', 'crate:a'),
    an('module:a::x', 'module', 'module:a'),
    an('module:a::y', 'module', 'module:a'),
  ],
  edges: [
    ae('cargo_dependency', 'crate:a', 'crate:b'),
    ae('module_use', 'module:a::x', 'module:a::y'),
    ae('module_use', 'module:a::y', 'module:a::x'),
  ],
  cycles: [{ id: 'c1', node_ids: ['module:a::x', 'module:a::y'], edge_ids: [] }],
  risks: [
    {
      kind: 'cycle',
      finding: row({
        id: 'arch-cycle-1',
        title: '循环依赖',
        change: null,
        change_label: null,
        reasons: [],
      }),
      node_ids: ['module:a::x', 'module:a::y'],
      edge_ids: [],
    },
  ],
  diff: {
    state: 'no_baseline',
    added_nodes: [],
    removed_nodes: [],
    added_edges: [],
    removed_edges: [],
    added_cycles: [],
    resolved_cycles: [],
  },
  impact: { direct: [], indirect: [], truncated: false },
  errors: [],
  unresolved_edges: 0,
};

export const archFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: archBody,
};
export const archNotApplicableFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: {
    ...archBody,
    status: 'not_applicable',
    nodes: [],
    edges: [],
    cycles: [],
    risks: [],
    status_note: '没有可用的架构数据(旧版报告或项目内没有可分析的 Rust 代码),请重新扫描。',
  },
};
export const archPartialFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: {
    ...archBody,
    status: 'partial',
    status_note: '架构分析不完整(见扫描范围或下方错误),不能据此判断没有风险。',
    errors: ['cargo metadata 失败'],
  },
};
export const archTruncatedFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: {
    ...archBody,
    truncated_note: '模块图过大(3005 个节点),仅显示 crate 层;分析结果(循环/枢纽/越界)仍基于完整图。',
  },
};

export const archCrateOnlyFixture: ViewPayload = {
  scan: scanIdle,
  category: 'architecture',
  body: {
    ...archBody,
    nodes: archBody.nodes.filter((n) => n.kind === 'crate'),
    edges: archBody.edges.filter((e) => e.kind === 'cargo_dependency'),
    truncated_note: '模块图过大(3005 个节点),仅显示 crate 层;分析结果(循环/枢纽/越界)仍基于完整图。',
  },
};
