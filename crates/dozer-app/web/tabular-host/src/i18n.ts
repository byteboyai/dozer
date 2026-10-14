/**
 * Tabular (ag-grid) host 前端 i18n(spec 2026-10-12-dozer-i18n-design §5/§6)。
 *
 * 资源是构建期静态打包的 `dozer.preview` 语言包(与 Rust 侧读同一份 JSON)。
 * 解析规则复用中性共享模块 `web/shared/i18n`,不复制实现。
 *
 * locale 由 Rust 经 URL `&locale=` 传入(与 `&theme=` 同一机制)。
 */

import {
  Resolver,
  type Bundle,
  type ResourceSet,
} from '../../shared/i18n/i18n.ts';
import en from '../../../locales/panels/preview/en.json' with { type: 'json' };
import zhCN from '../../../locales/panels/preview/zh-CN.json' with { type: 'json' };

export const PREVIEW_OWNER = 'dozer.preview';
const DEFAULT_LOCALE = 'en';

const bundles: Record<string, Bundle> = {
  en: en as Bundle,
  'zh-CN': zhCN as Bundle,
};

const set: ResourceSet = {
  owner: PREVIEW_OWNER,
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
  return resolver.resolve({ owner: PREVIEW_OWNER, key, fallback, args }, currentLocale);
}
