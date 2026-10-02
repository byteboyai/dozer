export interface RowRect {
  id: number;
  top: number;
  bottom: number;
}

function clampSlot(restLen: number, slot: number): number {
  return Math.max(0, Math.min(slot, restLen));
}

/** `slot` 是拖动项在"去掉它之后的列表"里的插入位置(0..=len)。
 *  返回应发给 Rust 的 `after_id`:放置位置上方那张可见卡片的 id,`null` = 放到最前。
 *  与 `Client::reorder_todo(id, after_id)` 语义一致。 */
export function afterIdForSlot(ids: number[], draggedId: number, slot: number): number | null {
  const rest = ids.filter((id) => id !== draggedId);
  const s = clampSlot(rest.length, slot);
  return s === 0 ? null : rest[s - 1];
}

/** 拖动期间的预览顺序。 */
export function moveToSlot(ids: number[], draggedId: number, slot: number): number[] {
  const rest = ids.filter((id) => id !== draggedId);
  const s = clampSlot(rest.length, slot);
  return [...rest.slice(0, s), draggedId, ...rest.slice(s)];
}

/** 放回原位:不发事件。 */
export function isNoopMove(ids: number[], draggedId: number, slot: number): boolean {
  const moved = moveToSlot(ids, draggedId, slot);
  return moved.length === ids.length && moved.every((id, i) => id === ids[i]);
}

/** 指针 y 对应的插入位置:越过其它行中线的行数(不含被拖动那行)。 */
export function slotFromY(rects: RowRect[], draggedId: number, y: number): number {
  let slot = 0;
  for (const r of rects) {
    if (r.id === draggedId) continue;
    if (y > (r.top + r.bottom) / 2) slot += 1;
    else break;
  }
  return slot;
}

/** 放下后保持预览顺序,直到权威推送到达(避免先弹回原位、再跳到新位置)。
 *  - 没有待定预览 → `null`;
 *  - 服务端顺序已与预览一致(已落库) → `null`(收敛,以服务端为准);
 *  - 条目集合变了(别处新增 / 删除 / 完成) → `null`(预览已过期);
 *  - 否则继续保持预览。落库被拒时由调用方的超时把预览清掉。 */
export function reconcilePendingOrder(
  pending: number[] | null,
  serverIds: number[],
): number[] | null {
  if (!pending) return null;
  if (pending.length !== serverIds.length) return null;
  if (!pending.every((id) => serverIds.includes(id))) return null;
  return pending.every((id, i) => id === serverIds[i]) ? null : pending;
}
