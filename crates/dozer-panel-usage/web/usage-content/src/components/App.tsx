import type { UsageViewPayload, ProjectSummary } from '../types.ts';
import { ProjectSummaryBoxes } from './ProjectSummaryBoxes.tsx';
import { AgentMetricsSection } from './AgentMetricsSection.tsx';
import { DailyUsageChart } from './DailyUsageChart.tsx';
import { TrendSection, TokenTrendSectionView } from './TrendSection.tsx';
import { EmptyRows, EmptyAgentRows } from './EmptyStates.tsx';
import { t } from '../i18n.ts';

function ProjectSection({ project, gitCommits }: { project: ProjectSummary; gitCommits: number }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head">{t('section.project', 'Project usage')}</div>
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
          {payload.session_trend && (
            <TrendSection
              title={t('section.session_trend', 'Session trend')}
              chart={payload.session_trend}
            />
          )}
          {payload.token_trend && <TokenTrendSectionView section={payload.token_trend} />}
        </>
      );
    case 'all_agents':
      return (
        <>
          <ProjectSection project={payload.project} gitCommits={payload.git_commits} />
          {payload.agent_metrics && (
            <div class="usage-section">
              <div class="usage-section-head">{t('section.agent_metrics', 'Agent usage')}</div>
              <AgentMetricsSection metrics={payload.agent_metrics} />
            </div>
          )}
          {payload.daily_usage && (
            <div class="usage-section">
              <div class="usage-section-head">{t('section.daily_usage', 'Daily usage')}</div>
              <DailyUsageChart chart={payload.daily_usage} />
            </div>
          )}
          {payload.daily_behavior && (
            <TrendSection
              title={t('section.daily_behavior', 'Daily activity')}
              chart={payload.daily_behavior}
            />
          )}
        </>
      );
  }
}
