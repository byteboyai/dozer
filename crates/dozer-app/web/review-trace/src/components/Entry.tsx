import type { AgentLabel, AiTurnData, ToolResult, TraceEntry } from '../types.ts';
import { AGENT_STYLE, ICONS } from '../agentIcons.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolResultRow } from './ToolResultRow.tsx';
import { TraceToggle } from './TraceToggle.tsx';
import { AvatarIcon } from './AvatarIcon.tsx';

const ROLE_LABEL: Record<string, string> = { ToolResult: '工具结果' };

export function Entry({ entry, agentLabel }: { entry: TraceEntry; agentLabel?: AgentLabel }) {
  const kind = Object.keys(entry)[0] as 'Human' | 'AiTurn' | 'ToolResult';
  const isErrorResult = kind === 'ToolResult' && (entry as { ToolResult: ToolResult }).ToolResult.is_error;

  let head;
  if (kind === 'Human') {
    head = (
      <div class="msg-head">
        <AvatarIcon svg={ICONS.user} />
        <span>you</span>
      </div>
    );
  } else if (kind === 'AiTurn') {
    const style = (agentLabel && AGENT_STYLE[agentLabel]) || { icon: ICONS.bot, color: null };
    head = (
      <div class="msg-head">
        <AvatarIcon svg={style.icon} color={style.color} />
        <span>{agentLabel || 'AI'}</span>
      </div>
    );
  } else {
    head = (
      <div class="msg-head">
        <span class="avatar-dot" />
        <span>{ROLE_LABEL[kind] || kind}</span>
      </div>
    );
  }

  return (
    <div class={`msg ${kind}${isErrorResult ? ' error' : ''}`}>
      {head}
      <div class="bubble">
        {kind === 'ToolResult' ? (
          <ToolResultRow result={(entry as { ToolResult: ToolResult }).ToolResult} />
        ) : null}
        {kind === 'Human' ? (
          <div class="text">{renderMarkdown((entry as { Human: { text: string } }).Human.text)}</div>
        ) : null}
        {kind === 'AiTurn' ? (
          <>
            {(entry as { AiTurn: { text?: string } }).AiTurn.text ? (
              <div class="text">
                {renderMarkdown((entry as { AiTurn: { text: string } }).AiTurn.text)}
              </div>
            ) : null}
            <TraceToggle v={(entry as { AiTurn: AiTurnData }).AiTurn} />
          </>
        ) : null}
      </div>
    </div>
  );
}
