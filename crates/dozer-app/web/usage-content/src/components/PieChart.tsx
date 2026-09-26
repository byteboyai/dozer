import type { AgentShare } from '../types.ts';
import { pieSlices, pieSlicePath } from '../pieSlices.ts';
import { agentColorVar } from '../theme.ts';

const PIE_RADIUS = 52;

export function PieChart({ share }: { share: AgentShare[] }) {
  const size = PIE_RADIUS * 2 + 8;
  const c = size / 2;
  const slices = pieSlices(share);
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} class="usage-pie">
      {slices.map((s) => (
        <path key={s.agent} d={pieSlicePath(c, c, PIE_RADIUS, s)} fill={agentColorVar(s.agent)} />
      ))}
    </svg>
  );
}
