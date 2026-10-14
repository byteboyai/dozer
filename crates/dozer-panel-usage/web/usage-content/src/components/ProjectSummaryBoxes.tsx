import type { ProjectSummary } from '../types.ts';
import { formatCount } from '../format.ts';
import { StatBox } from './StatBox.tsx';
import { t } from '../i18n.ts';

export function ProjectSummaryBoxes({
  project,
  gitCommits,
}: {
  project: ProjectSummary;
  gitCommits: number;
}) {
  return (
    <div class="usage-stat-row">
      <StatBox
        stats={[
          { label: t('stat.conversations', 'Sessions'), value: formatCount(project.conversation_count), colorVar: '--cream' },
          { label: t('stat.turns', 'Turns'), value: formatCount(project.turns), colorVar: '--cream' },
          { label: t('stat.tool_calls', 'Tool calls'), value: formatCount(project.tool_calls), colorVar: '--cream' },
          { label: t('stat.files_touched', 'Files touched'), value: formatCount(project.files_touched), colorVar: '--cream' },
          { label: t('stat.git_commits', 'Git commits'), value: formatCount(gitCommits), colorVar: '--cream' },
        ]}
      />
      <StatBox
        stats={[
          { label: 'Input', value: formatCount(project.tokens_in), colorVar: '--cyan' },
          { label: 'Output', value: formatCount(project.tokens_out), colorVar: '--cyan' },
          { label: t('stat.cache_read', 'cache read'), value: formatCount(project.tokens_cache_read), colorVar: '--cyan' },
          { label: t('stat.cache_write', 'cache write'), value: formatCount(project.tokens_cache_write), colorVar: '--cyan' },
        ]}
      />
    </div>
  );
}
