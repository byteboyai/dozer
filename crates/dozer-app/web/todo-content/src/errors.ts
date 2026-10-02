export interface ErrorLike {
  message?: string;
  error?: unknown;
  filename?: string;
}

/** 只有"确实是我们的代码抛的错"才算致命(Rust 侧会据此回落原生占位页)。
 *  浏览器的良性通知(ResizeObserver)和拿不到来源的跨域 "Script error." 不算。 */
export function shouldReportError(e: ErrorLike): boolean {
  const msg = e.message ?? '';
  if (/ResizeObserver loop/i.test(msg)) return false;
  if (msg === 'Script error.') return false;
  if (e.error instanceof Error) return true;
  return typeof e.filename === 'string' && e.filename.includes('todo-content');
}
