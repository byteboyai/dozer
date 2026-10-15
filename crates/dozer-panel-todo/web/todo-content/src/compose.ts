export interface KeyLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
  keyCode?: number;
}

/** 输入法组合输入期间(含 Safari 的 keyCode 229)的按键一律不当作提交/取消。 */
function composing(e: KeyLike): boolean {
  return e.isComposing || e.keyCode === 229;
}

/** 新增框:⌘↵ / Ctrl+↵ 提交;普通回车是换行,不提交。 */
export function isAddSubmit(e: KeyLike): boolean {
  return e.key === 'Enter' && (e.metaKey || e.ctrlKey) && !composing(e);
}

/** 内联编辑:回车提交(现有行为),组合输入期间不提交。 */
export function isInlineCommit(e: KeyLike): boolean {
  return e.key === 'Enter' && !composing(e);
}

export function isCancel(e: KeyLike): boolean {
  return e.key === 'Escape' && !composing(e);
}
