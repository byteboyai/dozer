/**
 * Dozer 前端共享 i18n 解析器(spec 2026-10-12-dozer-i18n-design §5/§6)。
 *
 * 与 `dozer-i18n`(Rust)保持同规则:扁平 JSON 字符串表,仅支持 `{name}`,
 * `{{`/`}}` 表示字面花括号,单次替换,不递归、不支持 HTML。规则一致性由
 * 共享向量 `crates/dozer-i18n/vectors/interpolation.json` 保证(Rust 与 TS
 * 读同一文件)。
 *
 * 这是**中性共享模块**,各 Web 包静态 import 其相对路径即可,不复制实现,
 * 也不依赖任何 npm package。独立面板(dozer-panel-usage)的构建可从任意 cwd
 * 定位它(以本文件相对自身路径解析资源)。
 *
 * 使用:
 *   import { interpolate, normalizeLocale, makeResolver } from "<rel>/shared/i18n/i18n";
 */

export interface Bundle {
  [key: string]: string;
}

export class I18nError extends Error {}

const NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** 参数名规则:ASCII 字母/数字/下划线,不以数字开头(spec §6.2)。 */
export function isValidParamName(name: string): boolean {
  return NAME_RE.test(name);
}

/**
 * 对模板做一次插值。多余参数忽略;缺参数或非法模板抛 `I18nError`。
 */
export function interpolate(
  template: string,
  args: Record<string, string> = {},
): string {
  let out = "";
  let i = 0;
  const n = template.length;
  while (i < n) {
    const c = template[i];
    if (c === "{" && template[i + 1] === "{") {
      out += "{";
      i += 2;
      continue;
    }
    if (c === "}" && template[i + 1] === "}") {
      out += "}";
      i += 2;
      continue;
    }
    if (c === "{") {
      const end = template.indexOf("}", i + 1);
      if (end < 0) throw new I18nError("未闭合的 `{`");
      const name = template.slice(i + 1, end);
      if (!isValidParamName(name)) throw new I18nError(`非法占位符名 {${name}}`);
      if (!(name in args)) throw new I18nError(`缺少参数 ${name}`);
      out += args[name];
      i = end + 1;
      continue;
    }
    if (c === "}") throw new I18nError("未配对的 `}`");
    out += c;
    i += 1;
  }
  return out;
}

export function placeholders(template: string): string[] {
  const out: string[] = [];
  let i = 0;
  const n = template.length;
  while (i < n) {
    const c = template[i];
    if (c === "{" && template[i + 1] === "{") {
      i += 2;
      continue;
    }
    if (c === "{") {
      const end = template.indexOf("}", i + 1);
      if (end > 0) {
        const name = template.slice(i + 1, end);
        if (!out.includes(name)) out.push(name);
        i = end + 1;
        continue;
      }
    }
    i += 1;
  }
  return out;
}

/** 校验一份语言包:值非空字符串、无重复 key、模板合法。返回 key 列表。 */
export function validateBundle(raw: unknown): Bundle {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
    throw new I18nError("资源顶层必须是字符串键值对象");
  }
  const obj = raw as Record<string, unknown>;
  for (const [k, v] of Object.entries(obj)) {
    if (typeof v !== "string") throw new I18nError(`值不是字符串: ${k}`);
    if (v === "") throw new I18nError(`空翻译: ${k}`);
    // 复用插值路径做括号/占位符校验(给空参数,只关心结构错误)。
    validateTemplate(k, v);
  }
  return obj as Bundle;
}

function validateTemplate(key: string, template: string): void {
  let i = 0;
  const n = template.length;
  while (i < n) {
    const c = template[i];
    if (c === "{" && template[i + 1] === "{") {
      i += 2;
      continue;
    }
    if (c === "}" && template[i + 1] === "}") {
      i += 2;
      continue;
    }
    if (c === "{") {
      const end = template.indexOf("}", i + 1);
      if (end < 0) throw new I18nError(`模板非法(${key}): 未闭合的 \`{\``);
      const name = template.slice(i + 1, end);
      if (!isValidParamName(name)) {
        throw new I18nError(`模板非法(${key}): 非法占位符名 {${name}}`);
      }
      i = end + 1;
      continue;
    }
    if (c === "}") throw new I18nError(`模板非法(${key}): 未配对的 \`}\``);
    i += 1;
  }
}

/**
 * Locale 规范化(spec §4.1),与 Rust `normalize_locale` 同规则。
 * 无匹配返回 `null`。
 */
export function normalizeLocale(raw: string): string | null {
  const s = (raw ?? "").trim();
  if (!s) return null;
  const lower = s.replace(/_/g, "-").toLowerCase();
  const primary = lower.split("@")[0] ?? "";
  const parts = primary.split("-").filter((p) => p.length > 0);
  const lang = parts[0] ?? "";
  if (lang === "en") return "en";
  if (lang === "zh") {
    let hasHant = false;
    let regionHant = false;
    let scriptHans = false;
    for (const p of parts.slice(1)) {
      if (p === "hant" || p === "traditional") hasHant = true;
      else if (p === "hans" || p === "simplified") scriptHans = true;
      else if (p === "tw" || p === "hk" || p === "mo") regionHant = true;
    }
    if (scriptHans) return "zh-CN";
    if (hasHant || regionHant) return null;
    return "zh-CN";
  }
  return null;
}

/** 按有序候选列表解析出第一个受支持语言;全部无匹配回落 `en`。 */
export function resolveFromCandidates(candidates: Iterable<string>): string {
  for (const c of candidates) {
    const loc = normalizeLocale(c);
    if (loc) return loc;
  }
  return "en";
}

export interface LocalizedText {
  owner: string;
  key?: string;
  fallback: string;
  args?: Record<string, string>;
}

export const SAFE_FALLBACK = "Something went wrong";
const DEFAULT_OWNER_LOCALE = "en";

export interface ResourceSet {
  owner: string;
  defaultLocale?: string;
  bundles: Record<string, Bundle>;
}

/** 前端解析器:持有按 owner 组织的资源集,按 locale 查表并插值。 */
export class Resolver {
  private sets = new Map<string, ResourceSet>();

  insert(set: ResourceSet): void {
    this.sets.set(set.owner, set);
  }

  resolve(text: LocalizedText, locale: string): string {
    const { owner, key, fallback, args = {} } = text;
    if (key === undefined) {
      return fallback.trim() ? fallback : SAFE_FALLBACK;
    }
    const set = this.sets.get(owner);
    if (!set) return this.fromFallback(fallback, args);

    const defaultLoc = set.defaultLocale ?? DEFAULT_OWNER_LOCALE;
    for (const cand of [locale, defaultLoc]) {
      const bundle = set.bundles[cand];
      if (!bundle) continue;
      const template = bundle[key];
      if (template === undefined) continue;
      try {
        const rendered = interpolate(template, args);
        if (rendered.trim()) return rendered;
      } catch {
        // 该候选不可渲染,continue。
      }
    }
    return this.fromFallback(fallback, args);
  }

  private fromFallback(fallback: string, args: Record<string, string>): string {
    if (fallback.trim()) {
      try {
        const rendered = interpolate(fallback, args);
        if (rendered.trim()) return rendered;
      } catch {
        // 落到安全文案。
      }
    }
    return SAFE_FALLBACK;
  }
}
