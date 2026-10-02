import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ViewPayload, StatusFilter, SetStatusTarget, TodoCard as CardData } from '../types.ts';
import { send } from '../ipc.ts';
import { UiStore } from '../uiStore.ts';
import { visibleItems } from '../filter.ts';
import { splitSegments, displayNumbers } from '../segments.ts';
import { afterIdForSlot, isNoopMove, moveToSlot, reconcilePendingOrder, slotFromY } from '../reorder.ts';
import { decideAdd, decideEdit } from '../limits.ts';
import { Toolbar } from './Toolbar.tsx';
import { TodoCard } from './TodoCard.tsx';
import { AddBox } from './AddBox.tsx';
import { Popover, type PopoverState } from './Popovers.tsx';
import { SegmentDivider, EmptyHint } from './Segment.tsx';

const DRAG_THRESHOLD = 4;

interface DragState {
  id: number;
  startY: number;
  active: boolean;
  slot: number;
}

export function App({ payload }: { payload: ViewPayload }) {
  const store = useRef(new UiStore()).current;
  const [, bump] = useState(0);
  const rerender = () => bump((n) => n + 1);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [popover, setPopover] = useState<PopoverState | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  // 放下后保持的预览顺序,直到权威推送到达(见 `reconcilePendingOrder`)。
  const [pendingOrder, setPendingOrder] = useState<number[] | null>(null);
  const dragRef = useRef<DragState | null>(null);
  dragRef.current = drag;

  const { scrollToTop } = store.onPayload(payload);
  const ui = store.get(payload.project_id);

  useEffect(() => {
    if (scrollToTop && scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [scrollToTop, payload.scroll_nonce]);

  // 切换项目:收起弹层、取消编辑与拖拽(各项目的草稿与搜索由 UiStore 保留)。
  useEffect(() => {
    setPopover(null);
    setEditingId(null);
    setDrag(null);
    setPendingOrder(null);
  }, [payload.project_id]);

  const visible = useMemo(
    () => visibleItems(payload.items, ui.search, ui.status),
    [payload.items, ui.search, ui.status],
  );
  const segs = splitSegments(visible);
  const numbers = displayNumbers(segs);

  // 拖动期间的预览顺序(只在进行中段)
  const activeIds = segs.active.map((i) => i.id);
  const settledPending = reconcilePendingOrder(pendingOrder, activeIds);
  const previewIds =
    drag && drag.active
      ? moveToSlot(activeIds, drag.id, drag.slot)
      : settledPending ?? activeIds;
  const activeById = new Map(segs.active.map((i) => [i.id, i]));
  const activeOrdered = previewIds.map((id) => activeById.get(id)!).filter(Boolean);

  // 服务端顺序追上预览(或条目集合变了)后清掉待定预览。
  useEffect(() => {
    if (pendingOrder !== null && settledPending === null) setPendingOrder(null);
  }, [pendingOrder, settledPending]);
  // 落库被拒时服务端顺序永远追不上:超时后回退到权威顺序(Rust 侧已发 Toast)。
  useEffect(() => {
    if (pendingOrder === null) return;
    const t = setTimeout(() => setPendingOrder(null), 2000);
    return () => clearTimeout(t);
  }, [pendingOrder]);

  const submitSearch = () => {
    ui.search = ui.searchDraft;
    rerender();
  };

  const submitAdd = () => {
    const d = decideAdd(ui.addDraft);
    if (d.action !== 'send') return; // 空草稿不发;超长保留草稿(不静默丢字)
    send({ kind: 'add', text: d.text });
    ui.addDraft = '';
    rerender();
  };

  const onCardPointerDown = (id: number) => (e: PointerEvent) => {
    if (!activeById.has(id) || editingId !== null || e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest('button, textarea, input')) return; // 点按钮/输入不触发拖拽
    setDrag({ id, startY: e.clientY, active: false, slot: activeIds.indexOf(id) });
  };

  useEffect(() => {
    if (!drag) return;
    const onMove = (e: PointerEvent) => {
      setDrag((d) => {
        if (!d) return d;
        const moved = d.active || Math.abs(e.clientY - d.startY) > DRAG_THRESHOLD;
        if (!moved) return d;
        const rows = Array.from(document.querySelectorAll<HTMLElement>('.segment-active .todo-card')).map((el) => {
          const r = el.getBoundingClientRect();
          return { id: Number(el.dataset.id), top: r.top, bottom: r.bottom };
        });
        return { ...d, active: true, slot: slotFromY(rows, d.id, e.clientY) };
      });
    };
    const finish = (commit: boolean) => {
      const d = dragRef.current;
      if (d && d.active && commit && !isNoopMove(activeIds, d.id, d.slot)) {
        setPendingOrder(moveToSlot(activeIds, d.id, d.slot));
        send({ kind: 'reorder', id: d.id, after_id: afterIdForSlot(activeIds, d.id, d.slot) });
      }
      setDrag(null);
    };
    const onUp = () => finish(true);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') finish(false);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('keydown', onKey);
    };
  }, [drag?.id, activeIds.join(',')]);

  const openPopover = (p: PopoverState) => {
    setEditingId(null);
    setPopover(p);
  };
  const rect = (r: DOMRect) => ({ left: r.left, top: r.top, right: r.right, bottom: r.bottom });

  const renderCard = (it: CardData, draggable: boolean) => (
    <TodoCard
      key={it.id}
      item={it}
      number={numbers.get(it.id) ?? 0}
      selected={ui.selectedId === it.id}
      draggable={draggable}
      dragging={!!drag && drag.active && drag.id === it.id}
      editing={editingId === it.id}
      onToggle={() => send({ kind: 'toggle', id: it.id })}
      onSelect={() => {
        ui.selectedId = it.id;
        rerender();
      }}
      onBeginEdit={() => {
        setPopover(null);
        setEditingId(it.id);
      }}
      onCommitEdit={(text) => {
        const d = decideEdit(text, it.text);
        if (d.action === 'keep') return; // 超长:继续编辑,不丢改动
        setEditingId(null);
        if (d.action === 'send') send({ kind: 'edit_text', id: it.id, text: d.text });
      }}
      onCancelEdit={() => setEditingId(null)}
      onOpenCategory={(a) => openPopover({ kind: 'category', id: it.id, anchor: rect(a), currentId: it.category_id })}
      onOpenCalendar={(a) => openPopover({ kind: 'calendar', id: it.id, anchor: rect(a), planDate: it.plan_date })}
      onOpenStatus={(a) => openPopover({ kind: 'status', id: it.id, anchor: rect(a), current: it.state })}
      onOpenDispatch={(a) => openPopover({ kind: 'dispatch', id: it.id, anchor: rect(a) })}
      onOpenDetail={() => send({ kind: 'open_detail', id: it.id })}
      onPointerDown={onCardPointerDown(it.id)}
    />
  );

  const pickStatus = (id: number, target: SetStatusTarget | 'in_progress') => {
    const card = payload.items.find((i) => i.id === id);
    if (target === 'in_progress') {
      // 「进行中」不可直接写入:搁置 → 恢复为待办;否则打开派发弹层。
      if (card?.state === 'suspended') {
        send({ kind: 'set_status', id, state: 'pending' });
        setPopover(null);
      } else {
        setPopover((cur) => (cur ? { kind: 'dispatch', id, anchor: cur.anchor } : cur));
      }
      return;
    }
    send({ kind: 'set_status', id, state: target });
    setPopover(null);
  };

  const hasAny = segs.active.length + segs.paused.length + segs.done.length > 0;
  const addHeight = ui.addHeight ?? payload.add_height_px;

  return (
    <div class="todo-root">
      <Toolbar
        searchDraft={ui.searchDraft}
        highlight={ui.searchDraft !== '' || ui.search !== ''}
        status={ui.status}
        onDraft={(v) => {
          ui.searchDraft = v;
          rerender();
        }}
        onSubmit={submitSearch}
        onOpenStatusFilter={(a) => openPopover({ kind: 'statusFilter', anchor: rect(a) })}
      />
      <div class="todo-scroll" ref={scrollRef} onScroll={(e) => (ui.scrollTop = (e.currentTarget as HTMLElement).scrollTop)}>
        {!hasAny ? (
          <EmptyHint>没有匹配的任务</EmptyHint>
        ) : (
          <>
            <div class="segment segment-active">{activeOrdered.map((it) => renderCard(it, true))}</div>
            {segs.paused.length > 0 ? (
              <div class="segment segment-paused">
                <SegmentDivider label="搁置" />
                {segs.paused.map((it) => renderCard(it, false))}
              </div>
            ) : null}
            {segs.done.length > 0 ? (
              <div class="segment segment-done">
                <SegmentDivider label="已完成" />
                {segs.done.map((it) => renderCard(it, false))}
              </div>
            ) : null}
          </>
        )}
      </div>
      <AddBox
        draft={ui.addDraft}
        height={addHeight}
        onDraft={(v) => {
          ui.addDraft = v;
          rerender();
        }}
        onSubmit={submitAdd}
        onHeight={(px, commit) => {
          ui.addHeight = px;
          rerender();
          if (commit) send({ kind: 'add_height', px });
        }}
      />
      {popover ? (
        <Popover
          state={popover}
          agents={payload.agents}
          categories={payload.categories}
          today={payload.today}
          statusFilter={ui.status as StatusFilter}
          onClose={() => setPopover(null)}
          onPickStatus={pickStatus}
          onPickAgent={(id, agent) => {
            send({ kind: 'assign_agent', id, agent });
            setPopover(null);
          }}
          onPickDate={(id, date) => {
            send({ kind: 'set_plan_date', id, date });
            setPopover(null);
          }}
          onPickStatusFilter={(f) => {
            ui.status = f;
            setPopover(null);
            rerender();
          }}
          onPickCategory={(id, categoryId) => {
            send({ kind: 'set_category', id, category_id: categoryId });
            setPopover(null);
          }}
        />
      ) : null}
    </div>
  );
}
