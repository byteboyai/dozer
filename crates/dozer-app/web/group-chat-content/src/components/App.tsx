import { useRef, useState } from 'preact/hooks';
import type { ViewPayload } from '../types.ts';
import { GroupBar } from './GroupBar.tsx';
import { MemberBar } from './MemberBar.tsx';
import { MessageList } from './MessageList.tsx';
import { Composer } from './Composer.tsx';
import { Dialogs, type DialogState } from './Dialogs.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const [dialog, setDialog] = useState<DialogState>(null);
  // 草稿按群保存,切群不丢(纯前端状态,不进 Rust)。
  const drafts = useRef<Record<number, string>>({});

  if (!payload.loaded) return <div class="gc-empty">加载中…</div>;

  const group = payload.groups.find((g) => g.id === payload.selected_group_id) ?? null;

  return (
    <div class="gc-root">
      <GroupBar
        groups={payload.groups}
        selectedId={payload.selected_group_id}
        onNew={() => setDialog({ kind: 'new_group' })}
        onDelete={(id) => setDialog({ kind: 'delete_group', groupId: id })}
      />
      {payload.groups.length === 0 ? (
        <div class="gc-empty">
          <p>还没有群聊</p>
          <p class="gc-dim">邀请 Claude、Codex 进群,用 @ 点名让它们依次发言讨论。</p>
          <button class="gc-primary" onClick={() => setDialog({ kind: 'new_group' })}>新建群聊</button>
        </div>
      ) : group === null ? (
        <div class="gc-empty">选择一个群聊</div>
      ) : (
        <>
          <MemberBar
            members={group.members}
            onAdd={() => setDialog({ kind: 'member', groupId: group.id, editing: null })}
            onEdit={(m) => setDialog({ kind: 'member', groupId: group.id, editing: m })}
          />
          <MessageList
            messages={payload.messages}
            groupId={group.id}
            hasMembers={group.members.length > 0}
            onPushTodo={(m) => setDialog({ kind: 'push_todo', message: m })}
          />
          <Composer
            groupId={group.id}
            members={group.members}
            running={payload.running}
            hint={payload.hint}
            draftOf={(id) => drafts.current[id] ?? ''}
            onDraft={(id, text) => { drafts.current[id] = text; }}
          />
        </>
      )}
      <Dialogs
        state={dialog}
        group={group}
        onClose={() => setDialog(null)}
      />
    </div>
  );
}
