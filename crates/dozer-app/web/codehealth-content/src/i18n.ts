/**
 * Code Health 内容区前端 i18n(spec 2026-10-12-dozer-i18n-design §5/§6/§8)。
 *
 * 资源是**构建期静态打包**的 `dozer.code_health` 语言包(与 Rust 侧读同一份
 * JSON,见 `crates/dozer-app/locales/panels/codehealth/`),离线、无 CDN、无
 * 运行时 fetch。解析规则复用中性共享模块 `web/shared/i18n`(按 spec §5 "各 Web
 * 包静态 import 其相对路径"),不复制实现。
 *
 * spec §167:面板业务文案优先由**前端本地化**——Rust 侧 payload 追加了语义字段
 * (enum key / 原始计数 / 描述 key),前端据此按 locale 渲染;旧 Host 的预格式化
 * 字段保留为兜底。
 */

import {
  Resolver,
  type Bundle,
  type ResourceSet,
} from '../../shared/i18n/i18n.ts';
import en from '../../../locales/panels/codehealth/en.json' with { type: 'json' };
import zhCN from '../../../locales/panels/codehealth/zh-CN.json' with {
  type: 'json',
};
import type { ErrorDto } from './types.ts';

export const CODE_HEALTH_OWNER = 'dozer.code_health';
const DEFAULT_LOCALE = 'en';

const bundles: Record<string, Bundle> = {
  en: en as Bundle,
  'zh-CN': zhCN as Bundle,
};

const set: ResourceSet = {
  owner: CODE_HEALTH_OWNER,
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
 * 解析一条 `dozer.code_health` 文案。`fallback` 是英文(资源缺失/不可渲染时的
 * 兜底),与 Rust 侧 `panel_text` 的约定一致。
 */
export function t(
  key: string,
  fallback: string,
  args?: Record<string, string>,
): string {
  return resolver.resolve({ owner: CODE_HEALTH_OWNER, key, fallback, args }, currentLocale);
}

/**
 * 把 Rust payload 的稳定错误 key(`ErrorDto`,spec §10)渲染成本地化文案。
 * `{message}` 一律为服务/扫描器的原始消息,不翻译。
 */
export function errorText(dto: ErrorDto): string {
  switch (dto.kind) {
    case 'path_unavailable':
      return t('error.path_unavailable', 'Scan failed: project path unavailable');
    case 'scan_cancelled':
      return t(
        'error.scan_cancelled',
        'Scan task cancelled: {message}',
        { message: dto.message },
      );
    case 'scan_failed':
      return t(
        'error.scan_failed',
        'Scan failed: {message}',
        { message: dto.message },
      );
    case 'save_failed':
      return t(
        'error.save_failed',
        'Not saved; cannot be used for the next comparison: {message}',
        { message: dto.message },
      );
  }
}
