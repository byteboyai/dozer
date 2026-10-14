import type { FindingRow as Row, ReasonDto } from '../types.ts';
import { send } from '../ipc.ts';
import { t } from '../i18n.ts';

const SEVERITY_META: Record<string, { key: string; fallback: string }> = {
  critical: { key: 'severity.critical', fallback: 'Critical' },
  watch: { key: 'severity.watch', fallback: 'Watch' },
};

const CHANGE_META: Record<string, { key: string; fallback: string }> = {
  new: { key: 'change.new', fallback: 'New this scan' },
  worsened: { key: 'change.worsened', fallback: 'Worsened this scan' },
  improved: { key: 'change.improved', fallback: 'Improved this scan' },
  persisting: { key: 'change.persisting', fallback: 'Persisting' },
  resolved: { key: 'change.resolved', fallback: 'Resolved' },
};

function reasonText(r: ReasonDto): string {
  if (r.kind === 'metric') {
    return t('finding.reason.metric', 'Metric {delta}{unit}', {
      delta: r.delta > 0 ? `+${r.delta}` : String(r.delta),
      unit: r.unit,
    });
  }
  return t('finding.reason.recent_commits', 'Modified {count} times in the last 30 days', {
    count: String(r.count),
  });
}

export function FindingRowView({
  row,
  analyze,
  onActivate,
}: {
  row: Row;
  analyze?: boolean;
  onActivate?: () => void;
}) {
  // spec §167:优先语义字段(severity/change/title_key/reason_items),旧 Host
  // 缺字段时回落预格式化文案。
  const severityLabel = SEVERITY_META[row.severity]
    ? t(SEVERITY_META[row.severity].key, SEVERITY_META[row.severity].fallback)
    : row.severity_label;
  const changeLabel =
    row.change && CHANGE_META[row.change]
      ? t(CHANGE_META[row.change].key, CHANGE_META[row.change].fallback)
      : row.change_label;
  const title = row.title_key
    ? t(row.title_key, row.title)
    : row.title;
  const reasons = row.reason_items
    ? row.reason_items.map(reasonText)
    : row.reasons;
  return (
    <div
      class="finding"
      onClick={onActivate ?? (() => send({ kind: 'open_location', path: row.path, line: row.line }))}
    >
      <div class="finding-head">
        <span class={`badge ${row.severity}`}>{severityLabel}</span>
        <span class="finding-title">{title}</span>
        {changeLabel && <span class="dim small">{changeLabel}</span>}
      </div>
      <div class="finding-meta">
        {row.path}:{row.line}
        {reasons.length > 0 && <> · {reasons.join(' · ')}</>}
      </div>
      {analyze && (
        <div class="finding-actions">
          <button
            class="btn small"
            onClick={(e) => {
              e.stopPropagation();
              send({ kind: 'analyze_finding', id: row.id });
            }}
          >
            {t('finding.analyze', 'Ask Agent to analyze')}
          </button>
        </div>
      )}
    </div>
  );
}
