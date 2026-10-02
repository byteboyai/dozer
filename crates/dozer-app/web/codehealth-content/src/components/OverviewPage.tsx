import type { OverviewBody } from '../types.ts';
import { FindingRowView } from './FindingRow.tsx';

export function OverviewPage({ body }: { body: OverviewBody }) {
  return (
    <div>
      <div class="row">
        <h2>代码健康度总览</h2>
        {body.git_line && <span class="dim small">{body.git_line}</span>}
      </div>

      <div class="card">
        <div class={`tier ${body.tier}`}>{body.tier_label}</div>
        <div>{body.summary}</div>
      </div>

      <div class="card">
        <div>本次变化</div>
        {body.change.kind === 'first_scan' ? (
          <div class="dim">首次扫描,暂无历史可比对</div>
        ) : (
          <>
            <div class="kv">
              <span>代码行 {body.change.loc_delta_text}</span>
              <span>函数数 {body.change.functions_delta_text}</span>
            </div>
            <div class="kv">
              <span class="neg">新增风险 +{body.change.new_risks}</span>
              <span class="pos">已解决风险 {body.change.resolved_risks}</span>
            </div>
          </>
        )}
      </div>

      <h3>优先处理</h3>
      {body.priorities.length === 0 ? (
        <div class="dim">暂无优先处理项。</div>
      ) : (
        body.priorities.map((r) => <FindingRowView key={r.id} row={r} analyze />)
      )}

      <div class="dim small scope-line">扫描可信度:{body.scope_line}</div>
      {body.legacy_note && <div class="banner note">{body.legacy_note}</div>}
    </div>
  );
}
