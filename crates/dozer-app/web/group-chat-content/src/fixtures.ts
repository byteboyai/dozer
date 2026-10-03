import type { Group, Message, ViewPayload } from './types.ts';

export const unloadedFixture: ViewPayload = {
  project_id: 1, loaded: false, groups: [], selected_group_id: null, messages: [], hint: null, running: false,
};
export const noGroupsFixture: ViewPayload = { ...unloadedFixture, loaded: true };

const members = [
  { id: 10, agent: 'claude' as const, agent_label: 'Claude', handle: '架构师', role_prompt: '负责整体方案', color_slot: 0 },
  { id: 11, agent: 'codex' as const, agent_label: 'Codex', handle: '审阅者', role_prompt: '', color_slot: 1 },
];
const group: Group = { id: 1, topic: '评审登录方案', members };

const human = (id: number, seq: number, text: string): Message => ({
  id, seq, text, status: null, reason: null, duration_ms: null, todo_id: null,
  author: { kind: 'human', label: '你', agent_label: null, color_slot: null },
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
    human(1, 1, '@架构师 @审阅者 登录要不要加验证码？'),
    agent(2, 2, 0, '建议加。**理由**:\n- 防撞库\n- 成本低', 'done'),
    agent(3, 3, 1, '同意,但要 `限流` 兜底。', 'done', { todo_id: 9 }),
  ],
};

export const runningFixture: ViewPayload = {
  ...conversationFixture, running: true,
  messages: [
    human(1, 1, '@架构师 @审阅者 看下'),
    agent(2, 2, 0, '', 'running', { duration_ms: null }),
    agent(3, 3, 1, '', 'queued', { duration_ms: null }),
  ],
};

export const failedFixture: ViewPayload = {
  ...conversationFixture,
  messages: [
    human(1, 1, '@架构师 看下'),
    agent(2, 2, 0, '', 'failed', { reason: '发言超时' }),
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

export const hintFixture: ViewPayload = { ...conversationFixture, hint: '未找到成员 @nobody' };

export const noMembersFixture: ViewPayload = {
  ...conversationFixture, messages: [],
  groups: [{ id: 1, topic: '空群', members: [] }],
};

export const noSelectionFixture: ViewPayload = {
  ...conversationFixture, selected_group_id: null, messages: [],
};
