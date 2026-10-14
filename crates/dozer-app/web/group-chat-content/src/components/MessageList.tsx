import { useEffect, useRef } from 'preact/hooks';
import type { Message } from '../types.ts';
import { MessageItem } from './MessageItem.tsx';
import { t } from '../i18n.ts';

const NEAR_BOTTOM_PX = 80;

export function MessageList({
  messages, groupId, hasMembers, onPushTodo,
}: {
  messages: Message[];
  groupId: number;
  hasMembers: boolean;
  onPushTodo: (m: Message) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  // 新消息到来:仅当用户本来就在底部附近才跟随滚动,不打断回看。
  useEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages]);

  return (
    <div
      class="gc-messages"
      ref={box}
      onScroll={() => {
        const el = box.current;
        if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
      }}
    >
      {messages.length === 0 ? (
        <div class="gc-empty-inline">
          {hasMembers
            ? t('messages.empty_with_members', 'Send a message and use @ to call on members.')
            : t(
                'messages.empty_no_members',
                'This group has no members yet; click "Add member" above first.',
              )}
        </div>
      ) : (
        messages.map((m) => (
          <MessageItem key={m.id} m={m} groupId={groupId} onPushTodo={onPushTodo} />
        ))
      )}
    </div>
  );
}
