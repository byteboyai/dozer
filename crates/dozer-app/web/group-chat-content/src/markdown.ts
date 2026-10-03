// 群聊消息的最小 Markdown 渲染器。agent 输出不可信,所以这里的原则是:
//   1. **先整体转义**,再在转义后的文本上套固定的标签——输出里不可能出现输入带来的标签;
//   2. 不支持原始 HTML、链接、图片(一律以纯文本显示);
//   3. 只做:标题、段落、有序/无序列表、粗体、斜体、行内代码、围栏代码块、换行。
// 不引入第三方库(见 plan Ruling 4)。

const ESC: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ESC[c]);
}

/** 行内:行内代码先抽走占位(内容不再解析),其余先转义再套粗体/斜体。 */
function renderInline(text: string): string {
  const codes: string[] = [];
  const withoutCode = text.replace(/`([^`\n]+)`/g, (_m, c: string) => {
    codes.push(`<code>${escapeHtml(c)}</code>`);
    return `\u0000${codes.length - 1}\u0000`;
  });
  let out = escapeHtml(withoutCode);
  // 粗体先于斜体;要求标记内侧不是空白,避免 `2 * 3 * 4`。
  out = out.replace(/\*\*(?=\S)([^*\n]*?\S)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/(^|[^*\w])\*(?=\S)([^*\n]*?\S)\*(?!\*)/g, '$1<em>$2</em>');
  return out.replace(/\u0000(\d+)\u0000/g, (_m, i: string) => codes[Number(i)]);
}

const FENCE = /^```/;
const BULLET = /^\s*[-*]\s+(.*)$/;
const ORDERED = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;

export function renderMarkdown(src: string): string {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const html: string[] = [];
  let i = 0;

  const flushParagraph = (buf: string[]) => {
    if (buf.length === 0) return;
    html.push(`<p>${buf.map(renderInline).join('<br>')}</p>`);
    buf.length = 0;
  };

  const para: string[] = [];
  while (i < lines.length) {
    const line = lines[i];

    if (FENCE.test(line)) {
      flushParagraph(para);
      const code: string[] = [];
      i++;
      // 没有收尾围栏时(流式输出中途)一直读到结尾。
      while (i < lines.length && !FENCE.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      i++; // 跳过收尾围栏(若有)
      html.push(`<pre><code>${escapeHtml(code.join('\n'))}</code></pre>`);
      continue;
    }

    if (line.trim() === '') {
      flushParagraph(para);
      i++;
      continue;
    }

    const h = HEADING.exec(line);
    if (h) {
      flushParagraph(para);
      const level = Math.min(5, Math.max(3, h[1].length + 2));
      html.push(`<h${level}>${renderInline(h[2])}</h${level}>`);
      i++;
      continue;
    }

    const isBullet = BULLET.test(line);
    const isOrdered = !isBullet && ORDERED.test(line);
    if (isBullet || isOrdered) {
      flushParagraph(para);
      const re = isBullet ? BULLET : ORDERED;
      const items: string[] = [];
      while (i < lines.length) {
        const m = re.exec(lines[i]);
        if (!m) break;
        items.push(`<li>${renderInline(m[1])}</li>`);
        i++;
      }
      html.push(isBullet ? `<ul>${items.join('')}</ul>` : `<ol>${items.join('')}</ol>`);
      continue;
    }

    para.push(line);
    i++;
  }
  flushParagraph(para);
  return html.join('');
}
