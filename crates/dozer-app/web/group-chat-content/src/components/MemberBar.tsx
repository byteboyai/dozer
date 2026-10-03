import type { Member } from '../types.ts';

export function MemberBar({
  members, onAdd, onEdit,
}: {
  members: Member[];
  onAdd: () => void;
  onEdit: (m: Member) => void;
}) {
  return (
    <div class="gc-memberbar">
      {members.map((m) => (
        <button
          class={`gc-chip gc-c${m.color_slot}`}
          key={m.id}
          title={m.role_prompt || '没有角色设定'}
          onClick={() => onEdit(m)}
        >
          @{m.handle}<span class="gc-chip-agent">{m.agent_label}</span>
        </button>
      ))}
      <button class="gc-chip gc-chip-add" onClick={onAdd}>＋ 添加成员</button>
    </div>
  );
}
