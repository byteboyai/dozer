import type { OverviewBody } from '../types.ts';
import { FindingRowView } from './FindingRow.tsx';
import { t } from '../i18n.ts';

const TIER_META: Record<string, { key: string; fallback: string }> = {
  healthy: { key: 'tier.healthy', fallback: 'Healthy' },
  watch: { key: 'tier.watch', fallback: 'Needs attention' },
  critical: { key: 'tier.critical', fallback: 'Critical' },
};

function signed(n: number): string {
  return n > 0 ? `+${n}` : String(n);
}

export function OverviewPage({ body }: { body: OverviewBody }) {
  // spec §167:优先语义字段,旧 Host 缺字段时回落预格式化文案。
  const tier = TIER_META[body.tier]
    ? t(TIER_META[body.tier].key, TIER_META[body.tier].fallback)
    : body.tier_label;
  const gitLine = body.git
    ? [
        body.git.branch ?? t('overview.git_detached', 'detached'),
        body.git.head ?? '',
        body.git.dirty ? t('overview.git_dirty', 'uncommitted changes') : '',
      ]
        .filter((s) => s !== '')
        .join(' · ')
    : body.git_line;
  const summary = body.summary_parts
    ? t('overview.summary', 'Core code {loc} lines · {functions} functions with obvious structural issues', {
        loc: String(body.summary_parts.loc),
        functions: String(body.summary_parts.functions),
      })
    : body.summary;
  const scopeLine = body.scope_parts
    ? t(
        'overview.scope_line_detail',
        'Analyzed {analyzed} files · {languages} · Excluded {excluded} · Skipped {skipped}',
        {
          analyzed: String(body.scope_parts.analyzed_files),
          languages: body.scope_parts.languages,
          excluded: String(body.scope_parts.excluded_files),
          skipped: String(body.scope_parts.skipped_files),
        },
      )
    : body.scope_line;
  return (
    <div>
      <div class="row">
        <h2>{t('overview.title', 'Code Health Overview')}</h2>
        {gitLine && <span class="dim small">{gitLine}</span>}
      </div>

      <div class="card">
        <div class={`tier ${body.tier}`}>{tier}</div>
        <div>{summary}</div>
      </div>

      <div class="card">
        <div>{t('overview.change_heading', 'Changes this scan')}</div>
        {body.change.kind === 'first_scan' ? (
          <div class="dim">
            {t('overview.change_first_scan', 'First scan; no history to compare yet')}
          </div>
        ) : (
          <>
            <div class="kv">
              <span>
                {t('overview.loc_delta', 'Lines {delta}', {
                  delta:
                    body.change.loc_delta !== undefined
                      ? signed(body.change.loc_delta)
                      : body.change.loc_delta_text,
                })}
              </span>
              <span>
                {t('overview.functions_delta', 'Functions {delta}', {
                  delta:
                    body.change.functions_delta !== undefined
                      ? signed(body.change.functions_delta)
                      : body.change.functions_delta_text,
                })}
              </span>
            </div>
            <div class="kv">
              <span class="neg">
                {t('overview.new_risks', 'New risks +{count}', {
                  count: String(body.change.new_risks),
                })}
              </span>
              <span class="pos">
                {t('overview.resolved_risks', 'Resolved risks {count}', {
                  count: String(body.change.resolved_risks),
                })}
              </span>
            </div>
          </>
        )}
      </div>

      <h3>{t('overview.priorities_heading', 'Prioritized')}</h3>
      {body.priorities.length === 0 ? (
        <div class="dim">{t('overview.priorities_empty', 'No prioritized items.')}</div>
      ) : (
        body.priorities.map((r) => <FindingRowView key={r.id} row={r} analyze />)
      )}

      <div class="dim small scope-line">
        {t('overview.scope_line', 'Scan confidence: {scope}', { scope: scopeLine })}
      </div>
      {body.legacy_note && (
        <div class="banner note">
          {t(body.legacy_note, 'Legacy report; rescan to see changes and scope')}
        </div>
      )}
    </div>
  );
}
