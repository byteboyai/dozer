// handle 校验(镜像 Rust `dozerd::group_mentions::validate_handle`,Rust 为权威)与
// `@` 补全的纯逻辑。**终止标点集合必须与 Rust `TERMINATORS` 保持一致。**

const TERMINATORS = new Set([
  ',', '.', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '<', '>', '"', "'", '`', '，',
  '。', '；', '：', '！', '？', '（', '）', '、', '「', '」',
]);
const HANDLE_MAX_CHARS = 32;

export function isHandleChar(c: string): boolean {
  return !/\s/.test(c) && c !== '@' && !TERMINATORS.has(c);
}

export function handleKey(handle: string): string {
  return handle.toLowerCase();
}

/** 返回给用户看的中文错误,合法返回 null。`takenKeys` 是本群已有 handle 的 `handleKey`。 */
export function validateHandle(handle: string, takenKeys: string[]): string | null {
  if (handle.length === 0) return '名称不能为空';
  if ([...handle].length > HANDLE_MAX_CHARS) return `名称不能超过 ${HANDLE_MAX_CHARS} 个字符`;
  const bad = [...handle].find((c) => !isHandleChar(c));
  if (bad !== undefined) return `名称不能包含 ${JSON.stringify(bad)}`;
  if (takenKeys.includes(handleKey(handle))) return `群里已有名为 ${handle} 的成员`;
  return null;
}

export function suggestHandle(agent: 'claude' | 'codex', takenKeys: string[]): string {
  if (!takenKeys.includes(agent)) return agent;
  for (let n = 2; ; n++) {
    const cand = `${agent}${n}`;
    if (!takenKeys.includes(cand)) return cand;
  }
}

// ---- @ 补全 ----

export interface MentionContext {
  /** `@` 在文本中的下标(按 UTF-16 code unit,与 textarea 的 selectionStart 一致) */
  start: number;
  query: string;
}

/** 光标紧跟在一个未完成的 `@query` 之后才返回上下文。`@` 前一个字符不能是 ASCII
 *  字母数字/`_`/`.`/`-`(排除邮箱),与 Rust `parse_mentions` 一致。 */
export function mentionContext(text: string, rawCaret: number): MentionContext | null {
  const caret = Math.max(0, Math.min(rawCaret, text.length));
  let i = caret;
  while (i > 0 && isHandleChar(text[i - 1])) i--;
  if (i === 0 || text[i - 1] !== '@') return null;
  const at = i - 1;
  const prev = at === 0 ? '' : text[at - 1];
  if (prev !== '' && /[A-Za-z0-9_.\-]/.test(prev)) return null;
  return { start: at, query: text.slice(at + 1, caret) };
}

export function filterMembers<T extends { handle: string }>(members: T[], query: string): T[] {
  const q = query.toLowerCase();
  if (q === '') return members.slice();
  const prefix = members.filter((m) => m.handle.toLowerCase().startsWith(q));
  const rest = members.filter(
    (m) => !m.handle.toLowerCase().startsWith(q) && m.handle.toLowerCase().includes(q),
  );
  return [...prefix, ...rest];
}

/** 把光标前的 `@partial` 替换成 `@handle `(带一个空格),并返回新光标位置。 */
export function applyMention(
  text: string,
  ctx: MentionContext,
  caret: number,
  handle: string,
): { text: string; caret: number } {
  const insert = `@${handle} `;
  const next = text.slice(0, ctx.start) + insert + text.slice(caret);
  return { text: next, caret: ctx.start + insert.length };
}

/** 补全菜单是否该显示:有上下文,且用户没有在**这个 `@`** 上按过 Esc。
 *  "已关闭"按 `@` 的位置记,而不是靠改光标——否则 Esc 的 keydown 关掉菜单后,紧接着的
 *  keyup 会按真实光标重算上下文,菜单立刻又打开。 */
export function mentionMenuOpen(ctx: MentionContext | null, dismissedStart: number | null): boolean {
  return ctx !== null && ctx.start !== dismissedStart;
}

/** 光标离开任何 `@` 提及后忘掉"已关闭"标记,之后在同一下标再敲 `@` 能正常打开。 */
export function nextDismissed(ctx: MentionContext | null, dismissedStart: number | null): number | null {
  return ctx === null ? null : dismissedStart;
}
