import assert from 'node:assert/strict';
import { test } from 'node:test';
import { JSDOM } from 'jsdom';

import { sanitizeMermaidSvg } from '../../upstream/renderer-text/markdown.ts';

// mermaid（自 11.16 起观测到，实测 11.17.2）把 flowchart 节点 label 里的
// `\n` 转成 HTML 风格、不自闭合的 `<br>`，而不是 XML 要求的 `<br/>`。
// `sanitizeMermaidSvg` 用 `image/svg+xml`（严格 XML）重新解析 mermaid 产出
// 的 svg 字符串做安全清洗；unclosed `<br>` 会让整次解析直接抛
// "Unexpected closing tag"，导致整张图（不止那一个 label）都渲染失败。
// 复现串来自真实产物：抓取 crates/dozer-app/assets/flyfish/renderers/
// text.iife.js 对 docs/analysis/daintree静态结构图.md 第一张图的 mermaid.
// render() 原始输出。
const svgWithUnclosedBr = () => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <g>
    <foreignObject width="100" height="40">
      <div xmlns="http://www.w3.org/1999/xhtml">
        <span class="nodeLabel"><p>WebContentsView<br>React UI + Zustand stores + panels</p></span>
      </div>
    </foreignObject>
  </g>
</svg>`;

const buildDocument = () => new JSDOM('<!doctype html><html><body></body></html>').window.document;

test('sanitizeMermaidSvg 能解析 mermaid 输出的不自闭合 <br>（回归 mermaid 11.17.2 行为变化）', () => {
  const documentRef = buildDocument();
  const result = sanitizeMermaidSvg(documentRef, svgWithUnclosedBr());
  assert.equal(result.tagName.toLowerCase(), 'svg');
  const brCount = result.querySelectorAll('br').length;
  assert.equal(brCount, 1, '不自闭合的 <br> 应该被规整后正常解析成一个 br 元素，而不是解析失败');
});

test('sanitizeMermaidSvg 仍然剥离 script 标签与 on* 事件属性（安全清洗不因归一化而失效）', () => {
  const documentRef = buildDocument();
  const malicious = `<svg xmlns="http://www.w3.org/2000/svg">
    <script>alert(1)</script>
    <rect onclick="alert(1)" width="1" height="1"/>
  </svg>`;
  const result = sanitizeMermaidSvg(documentRef, malicious);
  assert.equal(result.querySelectorAll('script').length, 0);
  assert.equal(result.querySelector('rect')?.hasAttribute('onclick'), false);
});
