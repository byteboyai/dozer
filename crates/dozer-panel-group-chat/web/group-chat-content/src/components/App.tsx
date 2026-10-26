import { useRef, useState } from 'preact/hooks';
import type { ViewPayload } from '../types.ts';
import { t } from '../i18n.ts';
import { MemberBar } from './MemberBar.tsx';
import { MessageList } from './MessageList.tsx';
import { Composer } from './Composer.tsx';
import { Dialogs, type DialogState } from './Dialogs.tsx';

export function App({ payload }: { payload: ViewPayload }) {
  const [dialog, setDialog] = useState<DialogState>(null);
  // 草稿按群保存,切群不丢(纯前端状态,不进 Rust)。
  const drafts = useRef<Record<number, string>>({});

  if (!payload.loaded) {
    return <div class="gc-empty">{t('list.loading', 'Loading…')}</div>;
  }

  const group = payload.groups.find((g) => g.id === payload.selected_group_id) ?? null;

  return (
    <div class="gc-root">
      {payload.groups.length === 0 ? (
        <div class="gc-empty">
          <p>{t('list.empty_title', 'No group chats yet')}</p>
          <p class="gc-dim">
            {t(
              'list.empty_hint',
              'Create a group in the "Group chat" list on the right, invite Claude and Codex, then use @ to call on them to discuss in turn.',
            )}
          </p>
        </div>
      ) : group === null ? (
        <div class="gc-empty">
          {t('list.select_hint', 'Select a group chat in the list on the right')}
        </div>
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
