import type { ViewPayload } from './types.ts';

export const unloadedFixture: ViewPayload = {
  project_id: 1, loaded: false, groups: [], selected_group_id: null, messages: [], hint: null, running: false,
};
export const noGroupsFixture: ViewPayload = { ...unloadedFixture, loaded: true };
