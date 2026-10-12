import type { AgentKind, AgentShare } from './types.ts';

export const PIE_GAP_RAD = 0.035;

export interface PieSlice {
  agent: AgentKind;
  startAngle: number;
  endAngle: number;
}

// 12 点钟方向起(-PI/2),顺时针(角度递增)累加每片的角度,每片两端各扣
// 半个 PIE_GAP_RAD 留缝——逐行对照 chart.rs::PieChart::draw。
export function pieSlices(share: AgentShare[]): PieSlice[] {
  const total = share.reduce((sum, s) => sum + s.value, 0);
  if (total === 0) return [];
  let angle = -Math.PI / 2;
  const slices: PieSlice[] = [];
  for (const { agent, value } of share) {
    const sweep = (2 * Math.PI * value) / total;
    const startAngle = angle + PIE_GAP_RAD / 2;
    const endAngle = angle + sweep - PIE_GAP_RAD / 2;
    slices.push({ agent, startAngle, endAngle });
    angle += sweep;
  }
  return slices;
}

// 角度 → 圆上一点(屏幕坐标系,y 向下,角度递增=顺时针,与 chart.rs 的
// iced Radians 约定一致)。
export function pointOnCircle(cx: number, cy: number, r: number, angle: number): [number, number] {
  return [cx + r * Math.cos(angle), cy + r * Math.sin(angle)];
}

// 单个扇形切片的 SVG path `d` 属性。
export function pieSlicePath(cx: number, cy: number, r: number, slice: PieSlice): string {
  const [x1, y1] = pointOnCircle(cx, cy, r, slice.startAngle);
  const [x2, y2] = pointOnCircle(cx, cy, r, slice.endAngle);
  const largeArc = slice.endAngle - slice.startAngle > Math.PI ? 1 : 0;
  return `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2} Z`;
}
