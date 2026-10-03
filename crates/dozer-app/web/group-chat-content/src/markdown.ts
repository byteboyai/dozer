// 群聊消息的最小 Markdown 渲染器。agent 输出不可信,所以这里的原则是:
//   1. **先整体转义**,再在转义后的文本上套固定的标签——输出里不可能出现输入带来的标签;
//   2. 不支持原始 HTML、链接、图片(一律以纯文本显示);
//   3. 只做:标题、段落、有序/无序列表、粗体、斜体、行内代码、围栏代码块、换行。
// 另支持 GFM 表格(含对齐);不引入第三方库(见 plan Ruling 4)。

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

/** 把一行表格按未转义的 `|` 切成单元格,去掉首尾竖线;`\\|` 还原为字面 `|`。 */
function splitRow(line: string): string[] {
  let t = line.trim();
  if (t.startsWith('|')) t = t.slice(1);
  if (t.endsWith('|') && !t.endsWith('\\|')) t = t.slice(0, -1);
  const cells: string[] = [];
  let cur = '';
  for (let k = 0; k < t.length; k++) {
    if (t[k] === '\\' && t[k + 1] === '|') {
      cur += '|';
      k++;
    } else if (t[k] === '|') {
      cells.push(cur.trim());
      cur = '';
    } else {
      cur += t[k];
    }
  }
  cells.push(cur.trim());
  return cells;
}

const DELIM_CELL = /^:?-+:?$/;

/** 分隔行(`---|:--:|--:`):至少含一个 `|` 或首尾竖线,且每格都是横线。 */
function parseDelimiter(line: string): string[] | null {
  if (!line.includes('|') || !line.includes('-')) return null;
  const cells = splitRow(line);
  if (!cells.every((c) => DELIM_CELL.test(c))) return null;
  return cells.map((c) => {
    const l = c.startsWith(':');
    const r = c.endsWith(':');
    return l && r ? 'center' : r ? 'right' : l ? 'left' : '';
  });
}

function renderRow(tag: 'th' | 'td', cells: string[], aligns: string[]): string {
  const tds = aligns.map((a, c) => {
    const style = a ? ` style="text-align:${a}"` : '';
    return `<${tag}${style}>${renderInline(cells[c] ?? '')}</${tag}>`;
  });
  return `<tr>${tds.join('')}</tr>`;
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

    // 表格:表头行 + 分隔行,列数一致;后续连续含 `|` 的行都是表体。
    if (line.includes('|') && i + 1 < lines.length) {
      const aligns = parseDelimiter(lines[i + 1]);
      const head = aligns && splitRow(line);
      if (aligns && head && head.length === aligns.length) {
        flushParagraph(para);
        i += 2;
        const body: string[] = [];
        while (i < lines.length && lines[i].trim() !== '' && lines[i].includes('|')) {
          body.push(renderRow('td', splitRow(lines[i]), aligns));
          i++;
        }
        html.push(
          `<div class="gc-table-wrap"><table><thead>${renderRow('th', head, aligns)}</thead>` +
            `<tbody>${body.join('')}</tbody></table></div>`,
        );
        continue;
      }
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
