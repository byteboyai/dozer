import type { Member } from '../types.ts';
import { t } from '../i18n.ts';

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
          title={m.role_prompt || t('member_bar.no_role', 'No role set')}
          onClick={() => onEdit(m)}
        >
          @{m.handle}<span class="gc-chip-agent">{m.agent_label}</span>
        </button>
      ))}
      <button class="gc-chip gc-chip-add" onClick={onAdd}>
        {t('member_bar.add', '＋ Add member')}
      </button>
    </div>
  );
}
