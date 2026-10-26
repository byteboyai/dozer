import test from 'node:test';
import assert from 'node:assert/strict';
import { h, Fragment } from 'preact';
import render from 'preact-render-to-string';
import { escapeHtml, renderInlineMarkdown, renderMarkdown } from './markdown.ts';

function html(text: string): string {
  return render(h(Fragment, null, ...renderMarkdown(text)));
}

test('escapeHtml escapes angle brackets and ampersands only', () => {
  assert.equal(escapeHtml('<script>&"x"</script>'), '&lt;script&gt;&amp;"x"&lt;/script&gt;');
});

test('renderInlineMarkdown never lets a literal angle bracket in source text become a real tag', () => {
  const out = renderInlineMarkdown('<img src=x onerror=alert(1)>');
  assert.ok(!out.includes('<img'));
  assert.ok(out.includes('&lt;img'));
});

test('renderInlineMarkdown supports bold, italic and inline code without cross-interference', () => {
  assert.equal(
    renderInlineMarkdown('**bold** *italic* `code`'),
    '<strong>bold</strong> <em>italic</em> <code>code</code>',
  );
});

test('renderInlineMarkdown does not let inline-code content be re-processed as markdown', () => {
  assert.equal(renderInlineMarkdown('`**not bold**`'), '<code>**not bold**</code>');
});

test('renderMarkdown wraps a single line of text in a paragraph', () => {
  assert.equal(html('hello world'), '<div class="md-p">hello world</div>');
});

test('renderMarkdown joins consecutive lines of one paragraph with <br>', () => {
  assert.equal(html('line one\nline two'), '<div class="md-p">line one<br>line two</div>');
});

test('renderMarkdown starts a new paragraph after a blank line', () => {
  assert.equal(
    html('first\n\nsecond'),
    '<div class="md-p">first</div><div class="md-p">second</div>',
  );
});

test('renderMarkdown renders headings with the matching level class', () => {
  assert.equal(html('## Title'), '<div class="md-h md-h2">Title</div>');
});

test('renderMarkdown renders a fenced code block verbatim, without markdown processing inside it', () => {
  assert.equal(
    html('```\nconst a = 1;\n**not bold**\n```'),
    '<pre class="md-code-block"><code>const a = 1;\n**not bold**</code></pre>',
  );
});

test('renderMarkdown renders an unordered list', () => {
  assert.equal(html('- one\n- two'), '<ul class="md-list"><li>one</li><li>two</li></ul>');
});

test('renderMarkdown renders an ordered list', () => {
  assert.equal(html('1. one\n2. two'), '<ol class="md-list"><li>one</li><li>two</li></ol>');
});

test('renderMarkdown renders a GFM table with header and body rows', () => {
  const out = html('| A | B |\n| --- | --- |\n| 1 | 2 |');
  assert.equal(
    out,
    '<table class="md-table"><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>',
  );
});

test('renderMarkdown does not treat a plain line containing one pipe as a table without a separator row', () => {
  assert.equal(html('a | b'), '<div class="md-p">a | b</div>');
});
