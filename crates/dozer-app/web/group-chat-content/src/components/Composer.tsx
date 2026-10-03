import { useEffect, useRef, useState } from 'preact/hooks';
import type { Member } from '../types.ts';
import { send } from '../ipc.ts';
import { cancelEvent } from '../events.ts';
import {
  applyMention, filterMembers, mentionContext, mentionMenuOpen, nextDismissed,
} from '../handle.ts';

export function Composer({
  groupId, members, running, hint, draftOf, onDraft,
}: {
  groupId: number;
  members: Member[];
  running: boolean;
  hint: string | null;
  draftOf: (groupId: number) => string;
  onDraft: (groupId: number, text: string) => void;
}) {
  const [text, setText] = useState(draftOf(groupId));
  const [caret, setCaret] = useState(0);
  const [pick, setPick] = useState(0);
  // 用户在哪个 `@`(下标)上按过 Esc;见 `mentionMenuOpen`。
  const [dismissed, setDismissed] = useState<number | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);

  // 切群:载入该群草稿。
  useEffect(() => { setText(draftOf(groupId)); setPick(0); }, [groupId]);

  const ctx = mentionContext(text, caret);
  const ctxStart = ctx ? ctx.start : -1;
  // 光标离开提及后忘掉"已关闭"标记。
  useEffect(() => {
    setDismissed((d) => nextDismissed(ctx, d));
  }, [ctxStart]);
  const options = ctx && mentionMenuOpen(ctx, dismissed) ? filterMembers(members, ctx.query) : [];
  const menuOpen = options.length > 0;

  const update = (next: string, c: number) => {
    setText(next);
    setCaret(c);
    onDraft(groupId, next);
    setPick(0);
  };

  const choose = (handle: string) => {
    if (!ctx) return;
    const r = applyMention(text, ctx, caret, handle);
    update(r.text, r.caret);
    queueMicrotask(() => area.current?.setSelectionRange(r.caret, r.caret));
  };

  const submit = () => {
    const t = text.trim();
    if (t === '') return;
    send({ kind: 'post', group_id: groupId, text: t });
    update('', 0);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // Review Focus 2:中文输入法组词时 Enter 是确认候选词,不是发送。
    if (e.isComposing || e.keyCode === 229) return;
    if (menuOpen) {
      if (e.key === 'ArrowDown') { e.preventDefault(); setPick((p) => (p + 1) % options.length); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); setPick((p) => (p - 1 + options.length) % options.length); return; }
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); choose(options[pick].handle); return; }
      if (e.key === 'Escape') { e.preventDefault(); if (ctx) setDismissed(ctx.start); return; }
    }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit(); }
  };

  return (
    <div class="gc-composer">
      {hint && <div class="gc-hint">{hint}</div>}
      {menuOpen && (
        <div class="gc-mention-menu" role="listbox">
          {options.map((m, i) => (
            <button
              class={`gc-mention-item gc-c${m.color_slot}${i === pick ? ' is-picked' : ''}`}
              key={m.id}
              onMouseDown={(e) => { e.preventDefault(); choose(m.handle); }}
            >
              @{m.handle}<span class="gc-chip-agent">{m.agent_label}</span>
            </button>
          ))}
        </div>
      )}
      <div class="gc-composer-row">
        <textarea
          ref={area}
          class="gc-input"
          rows={2}
          value={text}
          placeholder="输入消息,@ 点名成员发言;Enter 发送,Shift+Enter 换行"
          onInput={(e) => {
            const el = e.currentTarget;
            update(el.value, el.selectionStart ?? el.value.length);
          }}
          onClick={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
          onKeyUp={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
          onKeyDown={onKeyDown}
        />
        <div class="gc-composer-actions">
          {running && (
            <button class="gc-stop" onClick={() => send(cancelEvent(groupId, 'round'))}>
              停止本轮
            </button>
          )}
          <button class="gc-send" disabled={text.trim() === ''} onClick={submit}>发送</button>
        </div>
      </div>
    </div>
  );
}
