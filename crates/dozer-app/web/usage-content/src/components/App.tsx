import type { UsageViewPayload, ProjectSummary } from '../types.ts';
import { ProjectSummaryBoxes } from './ProjectSummaryBoxes.tsx';
import { AgentMetricsSection } from './AgentMetricsSection.tsx';
import { DailyUsageChart } from './DailyUsageChart.tsx';
import { TrendSection, TokenTrendSectionView } from './TrendSection.tsx';
import { EmptyRows, EmptyAgentRows } from './EmptyStates.tsx';

function ProjectSection({ project, gitCommits }: { project: ProjectSummary; gitCommits: number }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head">项目用量统计</div>
      <ProjectSummaryBoxes project={project} gitCommits={gitCommits} />
    </div>
  );
}

export function App({ payload }: { payload: UsageViewPayload }) {
  switch (payload.kind) {
    case 'empty':
      return <EmptyRows />;
    case 'agent_empty':
      return <EmptyAgentRows />;
    case 'single_agent':
      return (
        <>
          <ProjectSection project={payload.project} gitCommits={payload.git_commits} />
          {payload.session_trend && <TrendSection title="Session 趋势" chart={payload.session_trend} />}
          {payload.token_trend && <TokenTrendSectionView section={payload.token_trend} />}
        </>
      );
    case 'all_agents':
      return (
        <>
          <ProjectSection project={payload.project} gitCommits={payload.git_commits} />
          {payload.agent_metrics && (
            <div class="usage-section">
              <div class="usage-section-head">Agent 用量统计</div>
              <AgentMetricsSection metrics={payload.agent_metrics} />
            </div>
          )}
          {payload.daily_usage && (
            <div class="usage-section">
              <div class="usage-section-head">每日用量统计</div>
              <DailyUsageChart chart={payload.daily_usage} />
            </div>
          )}
          {payload.daily_behavior && <TrendSection title="每日行为统计" chart={payload.daily_behavior} />}
        </>
      );
  }
}
