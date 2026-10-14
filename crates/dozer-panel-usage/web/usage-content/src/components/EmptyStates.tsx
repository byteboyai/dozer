import { t } from '../i18n.ts';

export function EmptyRows() {
  return (
    <div class="usage-empty">
      {t('empty.no_conversations', 'This project has no agent conversations yet')}
    </div>
  );
}

export function EmptyAgentRows() {
  return (
    <div class="usage-empty">
      {t('empty.no_agent_usage', 'This agent has no usage data in the current project yet')}
    </div>
  );
}
