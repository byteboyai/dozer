import type { Group } from '../types.ts';
import { send } from '../ipc.ts';

export function GroupBar({
  groups, selectedId, onNew, onDelete,
}: {
  groups: Group[];
  selectedId: number | null;
  onNew: () => void;
  onDelete: (id: number) => void;
}) {
  return (
    <div class="gc-groupbar">
      <div class="gc-tabs">
        {groups.map((g) => (
          <div class={`gc-tab${g.id === selectedId ? ' is-selected' : ''}`} key={g.id}>
            <button
              class="gc-tab-main"
              title={g.topic}
              onClick={() => send({ kind: 'select_group', group_id: g.id })}
            >
              {g.topic}
            </button>
            {g.id === selectedId && (
              <button class="gc-icon" title="删除群聊" onClick={() => onDelete(g.id)}>×</button>
            )}
          </div>
        ))}
      </div>
      <button class="gc-icon gc-new" title="新建群聊" aria-label="新建群聊" onClick={onNew}>＋</button>
    </div>
  );
}
