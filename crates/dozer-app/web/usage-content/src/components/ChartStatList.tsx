import type { AgentShare } from '../types.ts';
import { formatCount } from '../format.ts';
import { agentColorVar } from '../theme.ts';

// agent 展示名——同 `AgentKind::label()`(dozer_core::protocol,
// crates/dozer-core/src/protocol.rs),与 Rust 侧逐一对应。
const AGENT_LABEL: Record<string, string> = {
  claude: 'claude',
  codebuddy: 'codebuddy',
  opencode: 'opencode',
  codex: 'codex',
  goose: 'goose',
  aider: 'aider',
  v8agent: 'v8agent',
  unknown: 'unknown',
};

export function ChartStatList({ title, share }: { title: string; share: AgentShare[] }) {
  const total = share.reduce((sum, s) => sum + s.value, 0);
  return (
    <div>
      <div class="usage-legend-title">
        {title}({formatCount(total)})
      </div>
      {share.map((s) => {
        const pct = total > 0 ? Math.floor((s.value * 100) / total) : 0;
        return (
          <div class="usage-legend-row" key={s.agent}>
            <span class="usage-legend-dot" style={{ background: agentColorVar(s.agent) }} />
            <span class="usage-legend-text">
              {AGENT_LABEL[s.agent]} - {formatCount(s.value)}({pct}%)
            </span>
          </div>
        );
      })}
    </div>
  );
}
