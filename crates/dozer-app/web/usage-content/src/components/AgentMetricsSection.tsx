import type { AgentMetrics, AgentShare } from '../types.ts';
import { formatCount } from '../format.ts';
import { PieChart } from './PieChart.tsx';
import { ChartStatList } from './ChartStatList.tsx';

function shareTotal(share: AgentShare[]): number {
  return share.reduce((sum, s) => sum + s.value, 0);
}

function MetricBanner({ label, total }: { label: string; total: number }) {
  return (
    <div class="usage-metric-banner">
      <span class="label">{label}</span>
      <span class="total">{formatCount(total)} total</span>
    </div>
  );
}

function MetricGroup({ title, share }: { title: string; share: AgentShare[] }) {
  return (
    <div class="usage-metric-group">
      <PieChart share={share} />
      <ChartStatList title={title} share={share} />
    </div>
  );
}

// 某一侧没有数据时只放有数据那一节、让它吃满整行——对应
// chart.rs::pair_metric_cells 的 (Some,None)/(None,Some) 分支。
function PairCells({
  left,
  right,
}: {
  left: [string, AgentShare[]] | null;
  right: [string, AgentShare[]] | null;
}) {
  if (left && right) {
    return (
      <div class="usage-pair">
        <MetricGroup title={left[0]} share={left[1]} />
        <MetricGroup title={right[0]} share={right[1]} />
      </div>
    );
  }
  const only = left ?? right;
  if (!only) return null;
  return (
    <div class="usage-pair">
      <MetricGroup title={only[0]} share={only[1]} />
    </div>
  );
}

export function AgentMetricsSection({ metrics }: { metrics: AgentMetrics }) {
  const sessTotal = shareTotal(metrics.session_share);
  const hasSessionGroup = metrics.session_share.length > 0 || metrics.turn_share.length > 0;
  const hasTokenGroup = metrics.io_share.length > 0 || metrics.cache_share.length > 0;
  return (
    <>
      {hasSessionGroup && (
        <>
          <MetricBanner label="Session" total={sessTotal} />
          <PairCells
            left={metrics.session_share.length > 0 ? ['Session', metrics.session_share] : null}
            right={metrics.turn_share.length > 0 ? ['Round', metrics.turn_share] : null}
          />
        </>
      )}
      {hasTokenGroup && (
        <>
          <MetricBanner label="Tokens" total={metrics.total_tokens} />
          <PairCells
            left={metrics.io_share.length > 0 ? ['Input/Output', metrics.io_share] : null}
            right={metrics.cache_share.length > 0 ? ['Cache Read/Write', metrics.cache_share] : null}
          />
        </>
      )}
    </>
  );
}
