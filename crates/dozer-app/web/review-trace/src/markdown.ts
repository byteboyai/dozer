import { h, type VNode } from 'preact';

// 极简 markdown → 安全 HTML(无依赖,不接 CDN——wry 自定义协议页面不
// 兜底外部脚本失败,离线优先)。文本先整体转义再拼标签,标签只由本函数
// 生成,不会把用户/模型文本里的尖括号当成真标签解释,规避 XSS。只覆盖
// agent 回复里常见的子集:标题/粗体/斜体/行内代码/代码块/有序无序列表/GFM
// 表格,不追求 CommonMark 全量兼容。与 review_trace.html 原实现逐行对照
// 迁移,详见 docs/superpowers/specs/2026-09-25-review-trace-preact-migration-design.md。
export function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function renderInlineMarkdown(s: string): string {
  const codeSpans: string[] = [];
  // \uE000 是私有区码位,正常文本几乎不可能出现,用来给行内代码占位,
  // 比原始 NUL 字节更安全——HTML5 输入预处理会把 U+0000 统一替换成
  // U+FFFD,直接嵌 NUL 字节可行但脆弱、不可移植。
  let out = escapeHtml(s).replace(/`([^`]+)`/g, (_match, code: string) => {
    codeSpans.push(code);
    return '\uE000' + (codeSpans.length - 1) + '\uE000';
  });
  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/__([^_]+)__/g, '<strong>$1</strong>');
  out = out.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
  out = out.replace(/_([^_\n]+)_/g, '<em>$1</em>');
  out = out.replace(/\uE000(\d+)\uE000/g, (_match, i: string) => '<code>' + codeSpans[Number(i)] + '</code>');
  return out;
}

function inlineHtml(s: string) {
  return { __html: renderInlineMarkdown(s) };
}

export function renderMarkdown(text: string): VNode[] {
  const lines = text.split('\n');
  const nodes: VNode<any>[] = [];
  let para: string[] = [];
  let i = 0;

  const flushPara = () => {
    if (para.length === 0) return;
    nodes.push(
      h('div', {
        class: 'md-p',
        dangerouslySetInnerHTML: { __html: para.map(renderInlineMarkdown).join('<br>') },
      }),
    );
    para = [];
  };

  while (i < lines.length) {
    const line = lines[i];

    if (/^```/.test(line)) {
      flushPara();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) {
        codeLines.push(lines[i]);
        i++;
      }
      i++; // 跳过收尾的 ```
      nodes.push(h('pre', { class: 'md-code-block' }, h('code', null, codeLines.join('\n'))));
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushPara();
      nodes.push(
        h('div', {
          class: `md-h md-h${heading[1].length}`,
          dangerouslySetInnerHTML: inlineHtml(heading[2]),
        }),
      );
      i++;
      continue;
    }

    // GFM 表格:首行以 | 开头、第二行是 |---|---| 形式的分隔行才算表,
    // 避免把普通文本里的竖线误判成表格。行内单元格复用行内渲染(粗体/
    // 行内代码等照常生效),全部经 escapeHtml 转义,无 XSS 面。
    if (
      /^\s*\|/.test(line) &&
      i + 1 < lines.length &&
      /^\s*\|(\s*:?-+:?\s*\|)+\s*$/.test(lines[i + 1])
    ) {
      flushPara();
      const rows: string[][] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) {
        rows.push(
          lines[i]
            .trim()
            .replace(/^\|/, '')
            .replace(/\|$/, '')
            .split('|')
            .map((c) => c.trim()),
        );
        i++;
      }
      // rows[0] = 表头,rows[1] = 分隔行(已由上面的判据保证存在),其余为正文。
      const headRow = h(
        'tr',
        null,
        rows[0].map((c) => h('th', { dangerouslySetInnerHTML: inlineHtml(c) })),
      );
      const bodyRows = rows
        .slice(2)
        .map((cells) =>
          h(
            'tr',
            null,
            cells.map((c) => h('td', { dangerouslySetInnerHTML: inlineHtml(c) })),
          ),
        );
      nodes.push(
        h('table', { class: 'md-table' }, h('thead', null, headRow), h('tbody', null, bodyRows)),
      );
      continue;
    }

    if (/^[-*]\s+/.test(line)) {
      flushPara();
      const items: VNode<any>[] = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
        items.push(h('li', { dangerouslySetInnerHTML: inlineHtml(lines[i].replace(/^[-*]\s+/, '')) }));
        i++;
      }
      nodes.push(h('ul', { class: 'md-list' }, items));
      continue;
    }

    if (/^\d+\.\s+/.test(line)) {
      flushPara();
      const items: VNode<any>[] = [];
      while (i < lines.length && /^\d+\.\s+/.test(lines[i])) {
        items.push(h('li', { dangerouslySetInnerHTML: inlineHtml(lines[i].replace(/^\d+\.\s+/, '')) }));
        i++;
      }
      nodes.push(h('ol', { class: 'md-list' }, items));
      continue;
    }

    if (line.trim() === '') {
      flushPara();
      i++;
      continue;
    }

    para.push(line);
    i++;
  }
  flushPara();
  return nodes;
}
