import { useRef } from 'preact/hooks';
import { isAddSubmit } from '../compose.ts';

const MIN_H = 44;
const MAX_H = 360;

interface Props {
  draft: string;
  height: number;
  onDraft(v: string): void;
  onSubmit(): void;
  onHeight(px: number, commit: boolean): void;
}

export function AddBox(p: Props) {
  const startRef = useRef<{ y: number; h: number } | null>(null);
  const clamp = (h: number) => Math.max(MIN_H, Math.min(MAX_H, h));

  const onHandleDown = (e: PointerEvent) => {
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    startRef.current = { y: e.clientY, h: p.height };
  };
  const onHandleMove = (e: PointerEvent) => {
    const s = startRef.current;
    if (!s) return;
    // 向上拉增高(同原生:框顶的拖拽手柄向上拉)
    p.onHeight(clamp(s.h + (s.y - e.clientY)), false);
  };
  const onHandleUp = (e: PointerEvent) => {
    const s = startRef.current;
    startRef.current = null;
    if (s) p.onHeight(clamp(s.h + (s.y - e.clientY)), true);
  };

  return (
    <div class="add-box">
      <div
        class="add-resize-handle"
        onPointerDown={onHandleDown}
        onPointerMove={onHandleMove}
        onPointerUp={onHandleUp}
      />
      <div class="add-row">
        <textarea
          class="add-input"
          style={{ height: `${p.height}px` }}
          placeholder="添加新任务"
          value={p.draft}
          onInput={(e) => p.onDraft((e.currentTarget as HTMLTextAreaElement).value)}
          onKeyDown={(e) => {
            if (isAddSubmit(e)) {
              e.preventDefault();
              p.onSubmit();
            }
          }}
        />
        <button type="button" class="add-submit" aria-label="添加" onClick={() => p.onSubmit()}>
          ↑
        </button>
      </div>
    </div>
  );
}
