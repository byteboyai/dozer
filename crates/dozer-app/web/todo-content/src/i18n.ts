/**
 * Todo 内容区前端 i18n(spec 2026-10-12-dozer-i18n-design §5/§6/§8)。
 *
 * 资源是**构建期静态打包**的 `dozer.todo` 语言包(与 Rust 侧读同一份 JSON),
 * 离线、无 CDN、无运行时 fetch。解析规则复用中性共享模块 `web/shared/i18n`,
 * 不复制实现。
 *
 * locale 来自 Host 随 envelope 推送的 `ui_locale`(spec §8):`main.tsx` 每次
 * 收到新 envelope 后调用 `setLocale`,组件在重渲染时读到新值。
 */

import {
  Resolver,
  type Bundle,
  type ResourceSet,
} from '../../shared/i18n/i18n.ts';
import en from '../../../locales/panels/todo/en.json' with { type: 'json' };
import zhCN from '../../../locales/panels/todo/zh-CN.json' with { type: 'json' };

export const TODO_OWNER = 'dozer.todo';
const DEFAULT_LOCALE = 'en';

const bundles: Record<string, Bundle> = {
  en: en as Bundle,
  'zh-CN': zhCN as Bundle,
};

const set: ResourceSet = {
  owner: TODO_OWNER,
  defaultLocale: DEFAULT_LOCALE,
  bundles,
};

const resolver = new Resolver();
resolver.insert(set);

let currentLocale = DEFAULT_LOCALE;

/** 由 `main.tsx` 在每次收到 envelope 后注入当前生效语言。 */
export function setLocale(locale: string): void {
  currentLocale = locale;
}

export function getLocale(): string {
  return currentLocale;
}

/**
 * 解析一条 `dozer.todo` 文案。`key` 已在语言包里;`fallback` 是英文(资源
 * 缺失/不可渲染时的兜底),与 Rust 侧 `panel_text` 的约定一致。
 */
export function t(
  key: string,
  fallback: string,
  args?: Record<string, string>,
): string {
  return resolver.resolve({ owner: TODO_OWNER, key, fallback, args }, currentLocale);
}
