/**
 * Usage 内容区前端 i18n(spec 2026-10-12-dozer-i18n-design §5/§6/§8)。
 *
 * 资源是**构建期静态打包**的 `dozer.usage` 语言包(与 Rust 侧读同一份 JSON,
 * 见 `crates/dozer-panel-usage/locales/`),离线、无 CDN、无运行时 fetch。
 * 解析规则复用中性共享模块 `web/shared/i18n`(位于 `dozer-app/web/shared`,
 * 按 spec §5 "各 Web 包静态 import 其相对路径"),不复制实现。
 *
 * 本包属独立 crate `dozer-panel-usage`,因此从任意 cwd 构建都能定位到这一共享
 * 模块与自身资源,不依赖兄弟工作区运行路径。
 */

import {
  Resolver,
  type Bundle,
  type ResourceSet,
} from '../../../../dozer-app/web/shared/i18n/i18n.ts';
import en from '../../../locales/en.json' with { type: 'json' };
import zhCN from '../../../locales/zh-CN.json' with { type: 'json' };

export const USAGE_OWNER = 'dozer.usage';
const DEFAULT_LOCALE = 'en';

const bundles: Record<string, Bundle> = {
  en: en as Bundle,
  'zh-CN': zhCN as Bundle,
};

const set: ResourceSet = {
  owner: USAGE_OWNER,
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
 * 解析一条 `dozer.usage` 文案。`fallback` 是英文(资源缺失/不可渲染时的兜底),
 * 与 Rust 侧 `panel_text` 的约定一致。
 */
export function t(
  key: string,
  fallback: string,
  args?: Record<string, string>,
): string {
  return resolver.resolve({ owner: USAGE_OWNER, key, fallback, args }, currentLocale);
}
