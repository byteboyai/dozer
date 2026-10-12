import type { ProjectSummary } from '../types.ts';
import { formatCount } from '../format.ts';
import { StatBox } from './StatBox.tsx';

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
          { label: '会话', value: formatCount(project.conversation_count), colorVar: '--cream' },
          { label: '回合', value: formatCount(project.turns), colorVar: '--cream' },
          { label: '工具调用', value: formatCount(project.tool_calls), colorVar: '--cream' },
          { label: '触达文件', value: formatCount(project.files_touched), colorVar: '--cream' },
          { label: 'Git提交', value: formatCount(gitCommits), colorVar: '--cream' },
        ]}
      />
      <StatBox
        stats={[
          { label: 'Input', value: formatCount(project.tokens_in), colorVar: '--cyan' },
          { label: 'Output', value: formatCount(project.tokens_out), colorVar: '--cyan' },
          { label: 'cache 读', value: formatCount(project.tokens_cache_read), colorVar: '--cyan' },
          { label: 'cache 写', value: formatCount(project.tokens_cache_write), colorVar: '--cyan' },
        ]}
      />
    </div>
  );
}
