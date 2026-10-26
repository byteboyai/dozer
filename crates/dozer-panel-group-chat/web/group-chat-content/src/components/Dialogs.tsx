import type { ComponentChildren } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { AgentKey, Group, Member, Message } from '../types.ts';
import { send } from '../ipc.ts';
import { handleKey, suggestHandle, validateHandle } from '../handle.ts';
import { t } from '../i18n.ts';

export type DialogState =
  | null
  | { kind: 'member'; groupId: number; editing: Member | null }
  | { kind: 'push_todo'; message: Message };

function Shell({
  title, onClose, children,
}: { title: string; onClose: () => void; children: ComponentChildren }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div class="gc-modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div class="gc-modal" role="dialog" aria-label={title}>
        <h4>{title}</h4>
        {children}
      </div>
    </div>
  );
}

function MemberForm({
  group, groupId, editing, onClose,
}: { group: Group | null; groupId: number; editing: Member | null; onClose: () => void }) {
  const others = (group?.members ?? []).filter((m) => m.id !== editing?.id).map((m) => handleKey(m.handle));
  const [agent, setAgent] = useState<AgentKey>(editing?.agent ?? 'claude');
  const [handle, setHandle] = useState(editing?.handle ?? suggestHandle('claude', others));
  const [role, setRole] = useState(editing?.role_prompt ?? '');
  const err = validateHandle(handle.trim(), others);
  const submit = () => {
    if (err) return;
    if (editing) {
      send({ kind: 'update_member', member_id: editing.id, handle: handle.trim(), role_prompt: role.trim() });
    } else {
      send({ kind: 'add_member', group_id: groupId, agent, handle: handle.trim(), role_prompt: role.trim() });
    }
    onClose();
  };
  return (
    <Shell
      title={
        editing
          ? t('dialog.edit_member.title', 'Edit @{handle}', { handle: editing.handle })
          : t('dialog.add_member.title', 'Add member')
      }
      onClose={onClose}
    >
      {!editing && (
        <label class="gc-field">
          <span>{t('dialog.agent', 'Agent')}</span>
          <select
            class="gc-text" value={agent}
            onChange={(e) => {
              const next = e.currentTarget.value as AgentKey;
              setAgent(next);
              // 名称还是上一个 agent 的默认建议时,跟着换。
              if (handle === suggestHandle(agent, others)) setHandle(suggestHandle(next, others));
            }}
          >
            <option value="claude">Claude</option>
            <option value="codex">Codex</option>
          </select>
        </label>
      )}
      <label class="gc-field">
        <span>{t('dialog.handle', 'In-group name (use @ to call on it)')}</span>
        <input class="gc-text" value={handle} onInput={(e) => setHandle(e.currentTarget.value)} />
        {err && <em class="gc-err">{err}</em>}
      </label>
      <label class="gc-field">
        <span>{t('dialog.role', 'Role prompt (optional)')}</span>
        <textarea class="gc-text" rows={4} value={role} placeholder={t(
            'dialog.role_placeholder',
            "e.g. You look for holes in the plan; only raise questions, don't propose solutions.",
          )}
          onInput={(e) => setRole(e.currentTarget.value)} />
      </label>
      <div class="gc-modal-actions">
        {editing && (
          <button class="gc-danger gc-left" onClick={() => { send({ kind: 'remove_member', member_id: editing.id }); onClose(); }}>
            {t('dialog.remove_member', 'Remove from group')}
          </button>
        )}
        <button onClick={onClose}>{t('cancel.button', 'Cancel')}</button>
        <button class="gc-primary" disabled={err !== null} onClick={submit}>
          {editing ? t('dialog.save', 'Save') : t('dialog.add', 'Add')}
        </button>
      </div>
    </Shell>
  );
}

function PushTodo({ message, onClose }: { message: Message; onClose: () => void }) {
  const [text, setText] = useState(message.text.trim());
  const ok = text.trim() !== '';
  return (
    <Shell title={t('dialog.push_todo.title', 'To to-do')} onClose={onClose}>
      <p class="gc-dim">{t('dialog.push_todo.description', 'This creates a to-do in the Todo panel; assign who does it in Todo.')}</p>
      <textarea class="gc-text" rows={6} value={text} onInput={(e) => setText(e.currentTarget.value)} />
      <div class="gc-modal-actions">
        <button onClick={onClose}>{t('cancel.button', 'Cancel')}</button>
        <button class="gc-primary" disabled={!ok}
          onClick={() => { send({ kind: 'push_todo', message_id: message.id, text: text.trim() }); onClose(); }}>
          {t('dialog.push_todo.confirm', 'Create to-do')}
        </button>
      </div>
    </Shell>
  );
}

export function Dialogs({
  state, group, onClose,
}: { state: DialogState; group: Group | null; onClose: () => void }) {
  if (!state) return null;
  switch (state.kind) {
    case 'member': return <MemberForm group={group} groupId={state.groupId} editing={state.editing} onClose={onClose} />;
    case 'push_todo': return <PushTodo message={state.message} onClose={onClose} />;
  }
}
