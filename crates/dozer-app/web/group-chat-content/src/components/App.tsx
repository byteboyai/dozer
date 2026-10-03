import { useRef, useState } from 'preact/hooks';
import type { ViewPayload } from '../types.ts';
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
      {payload.groups.length === 0 ? (
        <div class="gc-empty">
          <p>还没有群聊</p>
          <p class="gc-dim">在右侧「群聊」列表里新建一个群,邀请 Claude、Codex 进群,用 @ 点名让它们依次发言讨论。</p>
        </div>
      ) : group === null ? (
        <div class="gc-empty">在右侧列表里选择一个群聊</div>
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
