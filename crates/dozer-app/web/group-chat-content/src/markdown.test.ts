import test from 'node:test';
import assert from 'node:assert/strict';
import { renderMarkdown } from './markdown.ts';

// Review Focus 5:agent 输出里的 HTML/脚本必须以纯文本显示。
test('raw html is escaped, never emitted as tags', () => {
  const out = renderMarkdown('<script>alert(1)</script><img src=x onerror=alert(1)>');
  assert.doesNotMatch(out, /<script/i);
  assert.doesNotMatch(out, /<img/i);
  assert.match(out, /&lt;script&gt;/);
  assert.match(out, /&lt;img/);
});

test('javascript: and other links are plain text, not anchors', () => {
  const out = renderMarkdown('[点我](javascript:alert(1)) 和 <a href="x">y</a> 和 https://example.com');
  assert.doesNotMatch(out, /<a[\s>]/i);
  assert.doesNotMatch(out, /href="/i); // 转义后只会剩 `href=&quot;`,不会有真正的属性
  assert.match(out, /javascript:alert\(1\)/);
});

test('attribute-breaking quotes in text are escaped', () => {
  const out = renderMarkdown('"><svg onload=alert(1)>');
  assert.doesNotMatch(out, /<svg/i);
  assert.match(out, /&quot;/);
});

test('paragraphs and line breaks', () => {
  assert.equal(renderMarkdown('第一段\n\n第二段'), '<p>第一段</p><p>第二段</p>');
  assert.equal(renderMarkdown('甲\n乙'), '<p>甲<br>乙</p>');
});

test('bold, italic and inline code', () => {
  assert.equal(renderMarkdown('**粗** 和 *斜* 和 `code`'), '<p><strong>粗</strong> 和 <em>斜</em> 和 <code>code</code></p>');
});

test('inline code content is escaped and not re-parsed as markdown', () => {
  assert.equal(renderMarkdown('`<b>**x**</b>`'), '<p><code>&lt;b&gt;**x**&lt;/b&gt;</code></p>');
});

test('fenced code block keeps content verbatim (escaped) and ignores language tag injection', () => {
  const out = renderMarkdown('```js"><x>\nconst a = "<b>";\n**not bold**\n```');
  assert.match(out, /<pre><code>/);
  assert.match(out, /const a = &quot;&lt;b&gt;&quot;;/);
  assert.match(out, /\*\*not bold\*\*/);
  assert.doesNotMatch(out, /<x>/);
});

test('unterminated fence is rendered as code to the end (streaming-friendly)', () => {
  const out = renderMarkdown('说明\n```\nlet x = 1;');
  assert.match(out, /<p>说明<\/p>/);
  assert.match(out, /<pre><code>let x = 1;<\/code><\/pre>/);
});

test('unordered and ordered lists', () => {
  assert.equal(renderMarkdown('- 甲\n- 乙'), '<ul><li>甲</li><li>乙</li></ul>');
  assert.equal(renderMarkdown('1. 一\n2. 二'), '<ol><li>一</li><li>二</li></ol>');
  assert.equal(renderMarkdown('* 甲\n* 乙'), '<ul><li>甲</li><li>乙</li></ul>');
});

test('list items support inline formatting', () => {
  assert.equal(renderMarkdown('- **重点** 与 `x`'), '<ul><li><strong>重点</strong> 与 <code>x</code></li></ul>');
});

test('headings map to h3-h5 (panel-sized) regardless of level', () => {
  assert.match(renderMarkdown('# 大'), /^<h3>大<\/h3>$/);
  assert.match(renderMarkdown('### 小'), /^<h5>小<\/h5>$/);
  assert.match(renderMarkdown('###### 最小'), /^<h5>最小<\/h5>$/);
});

test('hash without space is not a heading', () => {
  assert.equal(renderMarkdown('#标签'), '<p>#标签</p>');
});

test('emphasis markers inside words with underscores are left alone', () => {
  assert.equal(renderMarkdown('snake_case_name'), '<p>snake_case_name</p>');
});

test('unmatched emphasis markers stay literal', () => {
  assert.equal(renderMarkdown('2 * 3 = 6 and **未闭合'), '<p>2 * 3 = 6 and **未闭合</p>');
});

test('empty and whitespace-only input renders nothing', () => {
  assert.equal(renderMarkdown(''), '');
  assert.equal(renderMarkdown('  \n\n  '), '');
});

test('very long single line does not blow up', () => {
  const out = renderMarkdown('字'.repeat(50_000));
  assert.ok(out.startsWith('<p>') && out.endsWith('</p>'));
});

test('crlf line endings are normalized', () => {
  assert.equal(renderMarkdown('甲\r\n\r\n乙'), '<p>甲</p><p>乙</p>');
});
