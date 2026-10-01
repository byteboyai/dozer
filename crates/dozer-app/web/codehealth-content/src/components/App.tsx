import type { ViewPayload } from '../types.ts';
import { ScanHeader } from './ScanHeader.tsx';
import { EmptyState } from './EmptyState.tsx';
import { OverviewPage } from './OverviewPage.tsx';
import { StructurePage } from './StructurePage.tsx';
import { UiPage } from './UiPage.tsx';
import { ScopePage } from './ScopePage.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const { scan, body } = payload;
  return (
    <div>
      <ScanHeader scan={scan} hasReport={body.kind !== 'empty'} />
      {body.kind === 'empty' && <EmptyState message={body.message} />}
      {body.kind === 'overview' && <OverviewPage body={body} />}
      {body.kind === 'structure' && <StructurePage body={body} />}
      {body.kind === 'ui_consistency' && <UiPage body={body} />}
      {body.kind === 'scan_scope' && <ScopePage body={body} />}
    </div>
  );
}
