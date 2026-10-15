export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

const GAP = 4;

/** 弹层定位:默认在锚点下方左对齐;下方放不下就翻到上方;横向夹在视口内,
 *  保证弹层不超出 webview 矩形。 */
export function placePopover(
  anchor: Rect,
  size: { w: number; h: number },
  viewport: { w: number; h: number },
): { x: number; y: number } {
  let y = anchor.bottom + GAP;
  if (y + size.h > viewport.h - GAP) {
    const above = anchor.top - size.h - GAP;
    y = above >= GAP ? above : Math.max(GAP, viewport.h - size.h - GAP);
  }
  let x = anchor.left;
  if (x + size.w > viewport.w - GAP) x = viewport.w - size.w - GAP;
  if (x < GAP) x = GAP;
  return { x, y };
}
