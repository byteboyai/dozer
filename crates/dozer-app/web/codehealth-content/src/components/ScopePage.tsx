import type { ScopeBody, SkipReasonKey } from '../types.ts';
import { t } from '../i18n.ts';

const STATUS_META: Record<string, { key: string; fallback: string }> = {
  complete: { key: 'scope.status.complete', fallback: 'Complete' },
  partial: { key: 'scope.status.partial', fallback: 'Partially complete' },
  failed: { key: 'scope.status.failed', fallback: 'Failed' },
};

const SKIP_META: Record<SkipReasonKey, { key: string; fallback: string }> = {
  ignored: { key: 'scope.skip_reason.ignored', fallback: 'Ignore rule' },
  unsupported_language: {
    key: 'scope.skip_reason.unsupported_language',
    fallback: 'Unsupported language',
  },
  generated: { key: 'scope.skip_reason.generated', fallback: 'Generated file' },
  non_utf8: { key: 'scope.skip_reason.non_utf8', fallback: 'Non-UTF-8' },
  read_failed: { key: 'scope.skip_reason.read_failed', fallback: 'Read failed' },
  parse_failed: { key: 'scope.skip_reason.parse_failed', fallback: 'Parse failed' },
};

export function ScopePage({ body }: { body: ScopeBody }) {
  const status = body.status
    ? t(STATUS_META[body.status].key, STATUS_META[body.status].fallback)
    : body.status_label;
  const languages = body.languages
    ? body.languages
        .map((l) =>
          l.analyzed
            ? t('scope.language_entry_analyzed', '{language} ({files} files, structurally analyzed)', {
                language: l.language,
                files: String(l.files),
              })
            : t('scope.language_entry_counted', '{language} ({files} files, counted only)', {
                language: l.language,
                files: String(l.files),
              }),
        )
        .join(' · ')
    : body.languages_detail;
  return (
    <div>
      <h2>{t('scope.title', 'Scan Scope')}</h2>
      <div class="card">
        <div>{t('scope.status_line', 'Scan status: {status}', { status })}</div>
        <div>
          {t('scope.counts_line', 'Analyzed {analyzed} files · Excluded {excluded} · Skipped {skipped}', {
            analyzed: String(body.analyzed_files),
            excluded: String(body.excluded_files),
            skipped: String(body.skipped_count),
          })}
        </div>
        <div class="dim">
          {t('scope.languages_line', 'Languages found: {languages}', { languages })}
        </div>
        <div class="dim">
          {t('scope.duration_line', 'Scan duration: {ms} ms', { ms: String(body.duration_ms) })}
        </div>
        <div class="dim">
          {t('scope.schema_line', 'Report version: schema v{version}', {
            version: String(body.schema_version),
          })}
        </div>
        {body.git_baseline && (
          <div class="dim">
            {t('scope.git_baseline', 'Git baseline: {baseline}', { baseline: body.git_baseline })}
          </div>
        )}
      </div>
      {body.skipped.length > 0 && (
        <>
          <h3>{t('scope.skipped_heading', 'Skipped/excluded details:')}</h3>
          <ul class="skip-list">
            {body.skipped.map((s) => (
              <li key={s.path}>
                {s.path}(
                {s.reason_key
                  ? t(SKIP_META[s.reason_key].key, SKIP_META[s.reason_key].fallback)
                  : s.reason}
                )
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
