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

const URL_REFERENCE_ATTRIBUTES = [
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
const FRAGMENT_REFERENCE_ATTRIBUTES = ['href', 'xlink:href'];
const ID_LIST_ATTRIBUTES = ['aria-labelledby', 'aria-describedby'];

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
    for (const attr of URL_REFERENCE_ATTRIBUTES) {
      if (!el.hasAttribute(attr)) continue;
      const value = el.getAttribute(attr) ?? '';
      let next = value;
      for (const [oldId, newId] of ordered) {
        // 这里只接受 url(#id)。不能泛化成任意 #id：fill="#fff"、stroke="#000"
        // 是颜色，不是 fragment 引用；误改会让 Mermaid 大片回退成黑色。
        next = next.replace(
          new RegExp(`url\\(\\s*(["']?)#${escapeRegExp(oldId)}\\1\\s*\\)`, 'g'),
          `url(#${newId})`
        );
      }
      if (next !== value) {
        el.setAttribute(attr, next);
      }
    }
    for (const attr of FRAGMENT_REFERENCE_ATTRIBUTES) {
      const value = el.getAttribute(attr);
      if (!value) continue;
      const mapped = value.startsWith('#') ? idMap.get(value.slice(1)) : undefined;
      if (mapped) el.setAttribute(attr, `#${mapped}`);
    }
    for (const attr of ID_LIST_ATTRIBUTES) {
      const value = el.getAttribute(attr);
      if (!value) continue;
      el.setAttribute(
        attr,
        value.split(/\s+/).map(id => idMap.get(id) ?? id).join(' ')
      );
    }
  }

  // Mermaid 会在 SVG 内嵌 <style>。同步改写 url(#id) 与明确的 ID 选择器，
  // 但绝不能把声明值里的十六进制颜色当成 ID。
  for (const style of root.querySelectorAll('style')) {
    let css = style.textContent ?? '';
    for (const [oldId, newId] of ordered) {
      css = css.replace(
        new RegExp(`url\\(\\s*(["']?)#${escapeRegExp(oldId)}\\1\\s*\\)`, 'g'),
        `url(#${newId})`
      );
      css = css.replace(
        new RegExp(`(^|[},\\s])#${escapeRegExp(oldId)}(?=[\\s.{:[>+~,#])`, 'gm'),
        `$1#${newId}`
      );
    }
    style.textContent = css;
  }

  return { rewritten: idMap.size };
};
