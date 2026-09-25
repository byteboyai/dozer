// 深克隆 Mermaid SVG 时,把其内部所有 id 及引用加上实例前缀。
//
// 目的:
// - 正文 DOM 与灯箱克隆同时存在,若不改 id,`url(#x)` / `<use href="#x">`
//   会解析到正文那一份(或反向),marker/clipPath/gradient 失效或互相污染。
// - 前缀只需在当前文档内唯一;同一灯箱实例可多次克隆,因此前缀含一个序号。
//
// 策略:两遍。第一遍收集所有 id 并建立 old->new 映射;第二遍改写 id 以及所有
// 「值里出现 #oldId」的属性(marker-*、clip-path、mask、fill、stroke、filter、
// href、xlink:href、style 等)。按 id 长度倒序替换,避免 `a` 命中 `ab`。

const REFERENCE_ATTRIBUTES = [
  'href',
  'xlink:href',
  'marker-start',
  'marker-mid',
  'marker-end',
  'clip-path',
  'mask',
  'fill',
  'stroke',
  'filter',
  'style',
];

export interface SvgIdRewriteResult {
  /** 被改写的 id 数量(便于断言)。 */
  rewritten: number;
}

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * 就地改写 `root` 子树内所有 SVG 元素 id 及其引用,加上 `prefix`。
 * 返回改写的 id 数量。幂等性不保证,调用方对每个克隆只调用一次。
 */
export const rewriteSvgIds = (root: Element, prefix: string): SvgIdRewriteResult => {
  const idMap = new Map<string, string>();
  const elements = [root, ...Array.from(root.querySelectorAll('*'))];

  for (const el of elements) {
    const id = el.getAttribute('id');
    if (id) {
      idMap.set(id, `${prefix}${id}`);
    }
  }

  if (idMap.size === 0) {
    return { rewritten: 0 };
  }

  // 长 id 优先替换,避免前缀包含关系导致误替换。
  const ordered = Array.from(idMap.entries()).sort((a, b) => b[0].length - a[0].length);

  for (const el of elements) {
    const id = el.getAttribute('id');
    if (id && idMap.has(id)) {
      el.setAttribute('id', idMap.get(id)!);
    }
    for (const attr of REFERENCE_ATTRIBUTES) {
      if (!el.hasAttribute(attr)) continue;
      const value = el.getAttribute(attr) ?? '';
      if (!value.includes('#')) continue;
      let next = value;
      for (const [oldId, newId] of ordered) {
        // 只替换 `#oldId` 作为完整引用标识符出现的位置,避免 `#a` 命中 `#ab`。
        next = next.replace(new RegExp(`#${escapeRegExp(oldId)}(?![\\w.-])`, 'g'), `#${newId}`);
      }
      if (next !== value) {
        el.setAttribute(attr, next);
      }
    }
    // style 属性里也可能有 url(#id),已含在 REFERENCE_ATTRIBUTES。
  }

  return { rewritten: idMap.size };
};
