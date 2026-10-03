import type { Message } from '../types.ts';
import { send } from '../ipc.ts';
import { cancelEvent } from '../events.ts';
import { renderMarkdown } from '../markdown.ts';
import { formatDuration } from '../format.ts';

const STATUS_LABEL: Record<string, string> = {
  queued: '排队中',
  running: '正在发言',
  failed: '发言失败',
  cancelled: '已取消',
};

export function MessageItem({
  m, groupId, onPushTodo,
}: { m: Message; groupId: number; onPushTodo: (m: Message) => void }) {
  const isHuman = m.author.kind === 'human';
  const pending = m.status === 'queued' || m.status === 'running';
  const canRetry = m.status === 'failed' || m.status === 'cancelled';
  const canPush = !pending && m.todo_id === null && m.text.trim() !== '';
  const colorClass = isHuman ? 'is-human' : m.author.color_slot !== null ? `gc-c${m.author.color_slot}` : 'gc-c-none';
  const dur = formatDuration(m.duration_ms);

  return (
    <div class={`gc-msg ${colorClass}`} data-seq={m.seq}>
      <div class="gc-msg-head">
        <span class="gc-author">{m.author.label}</span>
        {m.author.agent_label && <span class="gc-agent">{m.author.agent_label}</span>}
        {m.status && m.status !== 'done' && (
          <span class={`gc-status is-${m.status}`}>
            {m.status === 'running' && <span class="gc-dots" aria-hidden="true"><i /><i /><i /></span>}
            {STATUS_LABEL[m.status]}
          </span>
        )}
        {dur && m.status === 'done' && <span class="gc-dim">{dur}</span>}
      </div>
      {m.status === 'failed' && m.reason && <div class="gc-reason">{m.reason}</div>}
      {m.text !== '' && (
        <div class="gc-body" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text) }} />
      )}
      <div class="gc-msg-actions">
        {m.status === 'running' && (
          <button class="gc-link" onClick={() => send(cancelEvent(groupId, 'turn'))}>
            停止
          </button>
        )}
        {canRetry && (
          <button class="gc-link" onClick={() => send({ kind: 'retry', message_id: m.id })}>重试</button>
        )}
        {canPush && (
          <button class="gc-link" onClick={() => onPushTodo(m)}>转为待办</button>
        )}
        {m.todo_id !== null && (
          <button class="gc-link gc-badge" onClick={() => send({ kind: 'open_todo', todo_id: m.todo_id! })}>
            已转待办 ↗
          </button>
        )}
      </div>
    </div>
  );
}
