import type { ScanBar } from '../types.ts';
import { send } from '../ipc.ts';
import { errorText, t } from '../i18n.ts';

export function ScanHeader({ scan, hasReport }: { scan: ScanBar; hasReport: boolean }) {
  const when = scan.scanned_at
    ? t('scan.last_at', 'Last scan: {at}', { at: scan.scanned_at })
    : t('scan.never', 'Not scanned yet');
  const scanError = scan.scan_error_key
    ? errorText(scan.scan_error_key)
    : (scan.scan_error ?? '');
  const saveError = scan.save_error_key
    ? errorText(scan.save_error_key)
    : (scan.save_error ?? '');
  return (
    <div>
      <div class="scan-header">
        {hasReport && <span class="dim small">{when}</span>}
        <button
          class="btn"
          disabled={scan.scanning}
          onClick={() => send({ kind: 'scan_requested' })}
        >
          {scan.scanning ? t('scan.scanning', 'Scanning...') : t('scan.button', 'Scan')}
        </button>
        {scan.scanning && hasReport && scan.scanned_at && (
          <span class="dim small">
            {t('scan.stale', 'Showing stale results from {at}', { at: scan.scanned_at })}
          </span>
        )}
      </div>
      {scan.scan_error && (
        <div class="banner error">
          {t('scan.failed', 'Scan failed, please retry: {reason}', { reason: scanError })}
        </div>
      )}
      {scan.save_error && <div class="banner note">{saveError}</div>}
    </div>
  );
}
