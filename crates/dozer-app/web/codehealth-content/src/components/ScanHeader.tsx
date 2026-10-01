import type { ScanBar } from '../types.ts';
import { send } from '../ipc.ts';

export function ScanHeader({ scan, hasReport }: { scan: ScanBar; hasReport: boolean }) {
  const when = scan.scanned_at ? `上次扫描:${scan.scanned_at}` : '尚未扫描';
  return (
    <div>
      <div class="scan-header">
        {hasReport && <span class="dim small">{when}</span>}
        <button
          class="btn"
          disabled={scan.scanning}
          onClick={() => send({ kind: 'scan_requested' })}
        >
          {scan.scanning ? '扫描中…' : '扫描'}
        </button>
        {scan.scanning && hasReport && scan.scanned_at && (
          <span class="dim small">当前展示的是 {scan.scanned_at} 的旧结果</span>
        )}
      </div>
      {scan.scan_error && <div class="banner error">扫描失败,请重试:{scan.scan_error}</div>}
      {scan.save_error && <div class="banner note">{scan.save_error}</div>}
    </div>
  );
}
