import type { ScopeBody } from '../types.ts';

export function ScopePage({ body }: { body: ScopeBody }) {
  return (
    <div>
      <h2>扫描范围</h2>
      <div class="card">
        <div>扫描状态:{body.status_label}</div>
        <div>
          已分析 {body.analyzed_files} 个文件 · 排除 {body.excluded_files} 个 · 跳过{' '}
          {body.skipped_count} 个
        </div>
        <div class="dim">已发现语言:{body.languages_detail}</div>
        <div class="dim">扫描耗时:{body.duration_ms} ms</div>
        <div class="dim">报告版本:schema v{body.schema_version}</div>
        {body.git_baseline && <div class="dim">Git 基准:{body.git_baseline}</div>}
      </div>
      {body.skipped.length > 0 && (
        <>
          <h3>跳过/排除明细:</h3>
          <ul class="skip-list">
            {body.skipped.map((s) => (
              <li key={s.path}>
                {s.path}({s.reason})
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
