import type { Group, Message, ViewPayload } from './types.ts';

export const unloadedFixture: ViewPayload = {
  project_id: 1, loaded: false, groups: [], selected_group_id: null, messages: [], hint: null, running: false,
};
export const noGroupsFixture: ViewPayload = { ...unloadedFixture, loaded: true };

const members = [
  { id: 10, agent: 'claude' as const, agent_label: 'Claude', handle: 'architect', role_prompt: 'Owns the overall plan', color_slot: 0 },
  { id: 11, agent: 'codex' as const, agent_label: 'Codex', handle: 'reviewer', role_prompt: '', color_slot: 1 },
];
const group: Group = { id: 1, topic: 'Review the login plan', members };

const human = (id: number, seq: number, text: string): Message => ({
  id, seq, text, status: null, reason: null, duration_ms: null, todo_id: null,
  author: { kind: 'human', label: 'You', agent_label: null, color_slot: null },
});
const agent = (
  id: number, seq: number, memberIdx: number, text: string, status: Message['status'],
  extra: Partial<Message> = {},
): Message => ({
  id, seq, text, status, reason: null, duration_ms: 2300, todo_id: null,
  author: {
    kind: 'member', label: `@${members[memberIdx].handle}`,
    agent_label: members[memberIdx].agent_label, color_slot: members[memberIdx].color_slot,
  },
  ...extra,
});

export const conversationFixture: ViewPayload = {
  project_id: 1, loaded: true, groups: [group], selected_group_id: 1, hint: null, running: false,
  messages: [
    human(1, 1, '@architect @reviewer Should login require a captcha?'),
    agent(2, 2, 0, 'Recommend yes. **Reasons**:\n- Blocks credential stuffing\n- Low cost', 'done'),
    agent(3, 3, 1, 'Agreed, but add `rate limiting` as a fallback.', 'done', { todo_id: 9 }),
  ],
};

export const runningFixture: ViewPayload = {
  ...conversationFixture, running: true,
  messages: [
    human(1, 1, '@architect @reviewer take a look'),
    agent(2, 2, 0, '', 'running', { duration_ms: null }),
    agent(3, 3, 1, '', 'queued', { duration_ms: null }),
  ],
};

export const failedFixture: ViewPayload = {
  ...conversationFixture,
  messages: [
    human(1, 1, '@architect take a look'),
    agent(2, 2, 0, '', 'failed', { reason: 'Speaking timed out' }),
    agent(3, 3, 1, '', 'cancelled'),
  ],
};

export const maliciousFixture: ViewPayload = {
  ...conversationFixture,
  messages: [
    human(1, 1, '<img src=x onerror=alert(1)>'),
    agent(2, 2, 0, '<script>alert(1)</script> [x](javascript:alert(1))', 'done'),
  ],
};

export const hintFixture: ViewPayload = { ...conversationFixture, hint: 'Members not found: @nobody' };

export const noMembersFixture: ViewPayload = {
  ...conversationFixture, messages: [],
  groups: [{ id: 1, topic: 'Empty group', members: [] }],
};

export const noSelectionFixture: ViewPayload = {
  ...conversationFixture, selected_group_id: null, messages: [],
};
