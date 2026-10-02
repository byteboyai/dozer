/** 任务文本长度上限(按**码点数**计,与 Rust `protocol::MAX_TEXT_CHARS` 的
 *  `chars().count()` 一致)。输入框的 `maxLength` 按 UTF-16 单位计,只会比它更严,
 *  所以能输入进去的文本一定不会被 Rust 拒绝。两边改动必须同步。 */
export const MAX_TEXT_CHARS = 10_000;

export function fitsLimit(text: string): boolean {
  return [...text].length <= MAX_TEXT_CHARS;
}

export type AddDecision = { action: 'empty' } | { action: 'keep' } | { action: 'send'; text: string };

/** 新增:空草稿什么都不做;超长**保留草稿**(不发送、不清空,避免静默丢字);否则发送。 */
export function decideAdd(draft: string): AddDecision {
  const text = draft.trim();
  if (!text) return { action: 'empty' };
  if (!fitsLimit(text)) return { action: 'keep' };
  return { action: 'send', text };
}

export type EditDecision = { action: 'cancel' } | { action: 'keep' } | { action: 'send'; text: string };

/** 内联编辑:空或没改动 → 取消;超长 → 继续编辑(不丢改动);否则提交。 */
export function decideEdit(next: string, original: string): EditDecision {
  const text = next.trim();
  if (!text || text === original) return { action: 'cancel' };
  if (!fitsLimit(text)) return { action: 'keep' };
  return { action: 'send', text };
}
