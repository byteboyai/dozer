import type { EmptyBody, EmptyStateKey } from '../types.ts';
import { t } from '../i18n.ts';

const EMPTY_META: Record<EmptyStateKey, { key: string; fallback: string }> = {
  scanning: { key: 'empty.scanning', fallback: 'Scanning...' },
  never_scanned: {
    key: 'empty.never_scanned',
    fallback: 'This project has not been scanned yet.',
  },
  no_supported_code: {
    key: 'empty.no_supported_code',
    fallback: 'No supported code was found.',
  },
  partial_failure: {
    key: 'empty.partial_failure',
    fallback: 'Scan partially completed; some files failed (see Scan Scope).',
  },
  scan_failed: { key: 'empty.scan_failed', fallback: 'Scan failed.' },
};

export function EmptyState({ body }: { body: EmptyBody }) {
  // spec §167:优先用语义 key;旧 Host 缺字段时回落 payload 里的预格式化文案。
  const message =
    body.empty_state && EMPTY_META[body.empty_state]
      ? t(EMPTY_META[body.empty_state].key, EMPTY_META[body.empty_state].fallback)
      : body.message;
  return <div class="empty">{message}</div>;
}
