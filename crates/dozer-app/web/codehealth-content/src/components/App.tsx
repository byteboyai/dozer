import type { ViewPayload } from '../types.ts';
import { ScanHeader } from './ScanHeader.tsx';
import { EmptyState } from './EmptyState.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const { scan, body } = payload;
  return (
    <div>
      <ScanHeader scan={scan} hasReport={body.kind !== 'empty'} />
      {body.kind === 'empty' ? <EmptyState message={body.message} /> : null}
    </div>
  );
}
