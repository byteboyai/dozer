import type { FindingRow } from './types.ts';

/** `new` = 本轮新增(`new`/`worsened`);`change === null`(无差异数据)不算新增。 */
export function filterStructure(findings: FindingRow[], mode: 'new' | 'all'): FindingRow[] {
  if (mode === 'all') return findings;
  return findings.filter((f) => f.change === 'new' || f.change === 'worsened');
}
