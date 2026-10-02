import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import type { AgentInfo, CategoryRow, StatusFilter, SetStatusTarget } from '../types.ts';
import { STATUS_META, STATUS_FILTER_OPTIONS, statusFilterLabel } from '../statusMeta.ts';
import { placePopover, type Rect } from '../popover.ts';
import { formatMonthDay, monthGrid, parseMonthDay, shiftMonth } from '../calendar.ts';

export type PopoverState =
  | { kind: 'status'; id: number; anchor: Rect; current: string }
  | { kind: 'dispatch'; id: number; anchor: Rect }
  | { kind: 'calendar'; id: number; anchor: Rect; planDate: string | null }
  | { kind: 'statusFilter'; anchor: Rect }
  | { kind: 'category'; id: number; anchor: Rect; currentId: number | null };

interface Props {
  state: PopoverState;
  agents: AgentInfo[];
  categories: CategoryRow[];
  today: { year: number; month: number; day: number };
  statusFilter: StatusFilter;
  onClose(): void;
  onPickStatus(id: number, target: SetStatusTarget | 'in_progress'): void;
  onPickAgent(id: number, agent: string): void;
  onPickDate(id: number, date: string): void;
  onPickStatusFilter(f: StatusFilter): void;
  onPickCategory(id: number, categoryId: number | null): void;
}

export function Popover(p: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number }>({ x: 0, y: 0 });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setPos(
      placePopover(
        p.state.anchor,
        { w: el.offsetWidth, h: el.offsetHeight },
        { w: window.innerWidth, h: window.innerHeight },
      ),
    );
  }, [p.state]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.isComposing) p.onClose();
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('resize', p.onClose);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', p.onClose);
    };
  }, [p.onClose]);

  return (
    <div class="popover-layer" onPointerDown={() => p.onClose()}>
      <div
        ref={ref}
        class={`popover popover-${p.state.kind}`}
        style={{ left: `${pos.x}px`, top: `${pos.y}px` }}
        onPointerDown={(e) => e.stopPropagation()}
      >
        {renderBody(p)}
      </div>
    </div>
  );
}

function renderBody(p: Props) {
  const s = p.state;
  switch (s.kind) {
    case 'status':
      return (['pending', 'in_progress', 'suspended', 'done'] as const).map((st) => (
        <button
          type="button"
          class="menu-item"
          style={{ color: STATUS_META[st].color }}
          onClick={() => p.onPickStatus(s.id, st)}
        >
          {STATUS_META[st].label}
        </button>
      ));
    case 'dispatch':
      return p.agents.map((a) => (
        <button type="button" class="menu-item" onClick={() => p.onPickAgent(s.id, a.kind)}>
          <span
            class={`agent-icon${a.preserves_color ? ' keep-color' : ''}`}
            dangerouslySetInnerHTML={{ __html: a.icon_svg }}
          />
          <span>{a.label}</span>
        </button>
      ));
    case 'statusFilter':
      return STATUS_FILTER_OPTIONS.map((f) => (
        <button
          type="button"
          class={`menu-item${p.statusFilter === f ? ' is-current' : ''}`}
          style={{ color: f === 'all' ? 'var(--cream)' : STATUS_META[f].color }}
          onClick={() => p.onPickStatusFilter(f)}
        >
          {statusFilterLabel(f)}
        </button>
      ));
    case 'category':
      return (
        <>
          <button type="button" class={`menu-item${s.currentId === null ? ' is-current' : ''}`} onClick={() => p.onPickCategory(s.id, null)}>
            未分类
          </button>
          {p.categories.map((c) => (
            <button
              type="button"
              class={`menu-item${s.currentId === c.id ? ' is-current' : ''}`}
              style={{ paddingLeft: `${10 + c.depth * 14}px` }}
              onClick={() => p.onPickCategory(s.id, c.id)}
            >
              {c.name}
            </button>
          ))}
        </>
      );
    case 'calendar':
      return <Calendar id={s.id} planDate={s.planDate} today={p.today} onPick={p.onPickDate} />;
  }
}

function Calendar(props: {
  id: number;
  planDate: string | null;
  today: { year: number; month: number; day: number };
  onPick(id: number, date: string): void;
}) {
  const planned = parseMonthDay(props.planDate);
  // 初始月份:有计划日期用其月份(年取今年),否则今年今月(同原生)。
  const [ym, setYm] = useState({ y: props.today.year, m: planned ? planned.m : props.today.month });
  // 高亮:有计划日期高亮它,否则高亮今天(同原生 `selected_md`)。
  const sel = planned ?? { m: props.today.month, d: props.today.day };
  const cells = monthGrid(ym.y, ym.m);
  return (
    <div class="calendar">
      <div class="calendar-head">
        <button type="button" class="icon-btn" onClick={() => setYm(shiftMonth(ym.y, ym.m, -1))}>
          ‹
        </button>
        <span class="calendar-title">{`${ym.y}-${String(ym.m).padStart(2, '0')}`}</span>
        <button type="button" class="icon-btn" onClick={() => setYm(shiftMonth(ym.y, ym.m, 1))}>
          ›
        </button>
      </div>
      <div class="calendar-grid">
        {['日', '一', '二', '三', '四', '五', '六'].map((w) => (
          <span class="calendar-weekday">{w}</span>
        ))}
        {cells.map((d) =>
          d === null ? (
            <span class="calendar-cell empty" />
          ) : (
            <button
              type="button"
              class={`calendar-cell${sel.m === ym.m && sel.d === d ? ' is-selected' : ''}`}
              onClick={() => props.onPick(props.id, formatMonthDay(ym.m, d))}
            >
              {d}
            </button>
          ),
        )}
      </div>
    </div>
  );
}
