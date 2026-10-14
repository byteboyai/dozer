import type { UiBody } from '../types.ts';
import { FindingRowView } from './FindingRow.tsx';
import { t } from '../i18n.ts';

export function UiPage({ body }: { body: UiBody }) {
  if (!body.applicable) {
    return (
      <div>
        <h2>{t('ui.title', 'UI Consistency')}</h2>
        <div class="dim">
          {t(
            'ui.not_applicable',
            'UI consistency checks apply to iced/Rust; no applicable framework detected in this project.',
          )}
        </div>
      </div>
    );
  }
  return (
    <div>
      <h2>{t('ui.title', 'UI Consistency')}</h2>
      {body.groups.map((g) => (
        <div key={g.title}>
          <h3>
            {t('ui.group_heading', '{title}: {count}', {
              title: g.group_key ? t(g.group_key, g.title) : g.title,
              count: String(g.findings.length),
            })}
          </h3>
          {g.findings.length === 0 ? (
            <div class="dim small">{t('ui.group_empty', 'No findings')}</div>
          ) : (
            g.findings.map((r) => <FindingRowView key={r.id} row={r} />)
          )}
        </div>
      ))}
    </div>
  );
}
