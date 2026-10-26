import type { ViewPayload } from './types.ts';

/** 语言上下文(spec §8):生效语言 + GUI 本地显示代。旧 Host 无此字段时按 `en` 处理。 */
export interface LocaleStamp {
  ui_locale: string;
  locale_revision: number;
}

export interface PushState {
  revision: number;
  payload: ViewPayload | null;
  /** 当前生效语言;缺省(旧 Host)为 `en`。 */
  locale: string;
  /** 语言显示代;缺省(旧 Host)为 0。切换语言必递增。 */
  localeRevision: number;
}

export const DEFAULT_LOCALE = 'en';

/** spec §8:前端把 `document.documentElement.lang` 同步成当前生效语言。 */
export function setDocumentLang(locale: string): void {
  if (typeof document !== 'undefined' && document.documentElement) {
    document.documentElement.lang = locale;
  }
}

/** 应用一条 Rust 推送。revision 小于已应用值的推送(快速切换时晚到的旧响应)
 *  与无法解析/缺 payload 的推送一律忽略,状态原样返回。 */
export function applyEnvelope(prev: PushState, json: string): PushState {
  try {
    const env = JSON.parse(json) as {
      revision?: number;
      payload?: ViewPayload;
      ui_locale?: string;
      locale_revision?: number;
    };
    if (!env.payload || typeof env.revision !== 'number') return prev;
    if (env.revision < prev.revision) return prev;
    const locale = typeof env.ui_locale === 'string' ? env.ui_locale : prev.locale;
    const localeRevision =
      typeof env.locale_revision === 'number' ? env.locale_revision : prev.localeRevision;
    setDocumentLang(locale);
    return { revision: env.revision, payload: env.payload, locale, localeRevision };
  } catch {
    return prev;
  }
}
