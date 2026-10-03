import type { CancelScope, OutEvent } from './types.ts';

export const cancelEvent = (groupId: number, scope: CancelScope): OutEvent => ({
  kind: 'cancel', group_id: groupId, scope,
});
