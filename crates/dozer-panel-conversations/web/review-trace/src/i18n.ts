/**
 * 会话审阅 trace host 前端 i18n(spec 2026-10-12-dozer-i18n-design §5/§6)。
 *
 * 资源是构建期静态打包的 `dozer.agent` 语言包(与 Rust 侧读同一份 JSON)。
 * locale 由 Rust 经 URL `&locale=` 传入(与 `&theme=` 同一机制),宿主页
 * 启动时 `setLocale(location.search 的 locale)`。
 */

import {
  Resolver,
  type Bundle,
  type ResourceSet,
} from '../../../../dozer-app/web/shared/i18n/i18n.ts';
import en from '../../../../dozer-app/locales/panels/agent/en.json' with { type: 'json' };
import zhCN from '../../../../dozer-app/locales/panels/agent/zh-CN.json' with { type: 'json' };

export const AGENT_OWNER = 'dozer.agent';
const DEFAULT_LOCALE = 'en';

const bundles: Record<string, Bundle> = {
  en: en as Bundle,
  'zh-CN': zhCN as Bundle,
};

const set: ResourceSet = {
  owner: AGENT_OWNER,
  defaultLocale: DEFAULT_LOCALE,
  bundles,
};

const resolver = new Resolver();
resolver.insert(set);

let currentLocale = DEFAULT_LOCALE;

export function setLocale(locale: string): void {
  currentLocale = locale;
}

export function getLocale(): string {
  return currentLocale;
}

export function t(
  key: string,
  fallback: string,
  args?: Record<string, string>,
): string {
  return resolver.resolve({ owner: AGENT_OWNER, key, fallback, args }, currentLocale);
}
