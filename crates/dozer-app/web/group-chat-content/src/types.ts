export type AgentKey = 'claude' | 'codex';
export type StatusKey = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';
export type AuthorKind = 'human' | 'member' | 'system';

export interface Author {
  kind: AuthorKind;
  /** "你" / "@handle" / "已移除成员" / "系统" */
  label: string;
  /** "Claude" / "Codex",成员消息才有 */
  agent_label: string | null;
  /** 0..3,已移除成员/非成员为 null */
  color_slot: number | null;
}

export interface Member {
  id: number;
  agent: AgentKey;
  agent_label: string;
  handle: string;
  role_prompt: string;
  color_slot: number;
}

export interface Group {
  id: number;
  topic: string;
  members: Member[];
}

export interface Message {
  id: number;
  seq: number;
  author: Author;
  text: string;
  /** human/系统消息为 null */
  status: StatusKey | null;
  reason: string | null;
  duration_ms: number | null;
  todo_id: number | null;
}

export interface ViewPayload {
  project_id: number;
  loaded: boolean;
  groups: Group[];
  selected_group_id: number | null;
  messages: Message[];
  hint: string | null;
  running: boolean;
}

export type CancelScope = 'turn' | 'round';

export type OutEvent =
  | { kind: 'ready' }
  | { kind: 'failed'; reason: string }
  | { kind: 'add_member'; group_id: number; agent: AgentKey; handle: string; role_prompt: string }
  | { kind: 'update_member'; member_id: number; handle: string; role_prompt: string }
  | { kind: 'remove_member'; member_id: number }
  | { kind: 'post'; group_id: number; text: string }
  | { kind: 'cancel'; group_id: number; scope: CancelScope }
  | { kind: 'retry'; message_id: number }
  | { kind: 'push_todo'; message_id: number; text: string }
  | { kind: 'open_todo'; todo_id: number };
