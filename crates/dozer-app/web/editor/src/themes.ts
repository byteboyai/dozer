// ByteBoy2077 深浅主题。背景/文本/选区 chrome 走 EditorView.theme,语法
// token 色走 HighlightStyle(锚定终端 16 色面板的同一套角色色,与老 iced
// 编辑器的 syntect 主题观感一致)。

import { EditorView } from '@codemirror/view';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import type { Extension } from '@codemirror/state';

interface Palette {
  bg: string;
  fg: string;
  gutterBg: string;
  gutterFg: string;
  activeLine: string;
  selection: string;
  cursor: string;
  comment: string;
  green: string;
  cyan: string;
  purple: string;
  red: string;
  orange: string;
  fn: string;
  panel: string;
}

const DARK: Palette = {
  bg: '#0a0e16',
  fg: '#FFE5B4',
  gutterBg: '#0a0e16',
  gutterFg: '#6B7F8F',
  activeLine: 'rgba(255,229,180,0.05)',
  selection: 'rgba(71,222,240,0.22)',
  cursor: '#F2D94E',
  comment: '#6B7F8F',
  green: '#1AD585',
  cyan: '#47DEF0',
  purple: '#9580FF',
  red: '#FF6E6E',
  orange: '#FFF3B0',
  fn: '#B5A5FF',
  panel: '#0a0e16',
};

const LIGHT: Palette = {
  bg: '#fef2e4',
  fg: '#3a3226',
  gutterBg: '#fef2e4',
  gutterFg: '#9a8f7d',
  activeLine: 'rgba(0,0,0,0.04)',
  selection: 'rgba(0,120,150,0.18)',
  cursor: '#8a6d00',
  comment: '#9a8f7d',
  green: '#0f8a52',
  cyan: '#0b7f92',
  purple: '#6a4fd0',
  red: '#c23b3b',
  orange: '#9a6b00',
  fn: '#5a49c0',
  panel: '#fef2e4',
};

function highlight(p: Palette): HighlightStyle {
  return HighlightStyle.define([
    { tag: [t.comment, t.lineComment, t.blockComment], color: p.comment },
    { tag: [t.string, t.special(t.string)], color: p.green },
    { tag: [t.regexp], color: p.orange },
    { tag: [t.number, t.bool, t.null], color: p.orange },
    { tag: [t.keyword, t.controlKeyword, t.moduleKeyword], color: p.cyan },
    { tag: [t.operator], color: p.fg },
    { tag: [t.typeName, t.className, t.namespace], color: p.purple },
    { tag: [t.function(t.variableName), t.labelName], color: p.fn },
    {
      tag: [t.definition(t.variableName), t.definition(t.propertyName)],
      color: p.fg,
    },
    { tag: [t.propertyName, t.attributeName], color: p.orange },
    { tag: [t.variableName], color: p.fg },
    { tag: [t.tagName], color: p.cyan },
    { tag: [t.attributeValue], color: p.green },
    { tag: [t.heading, t.heading1, t.heading2], color: p.cyan },
    { tag: [t.link, t.url], color: p.fn, textDecoration: 'underline' },
    { tag: [t.invalid], color: p.red },
    { tag: [t.meta, t.processingInstruction], color: p.comment },
  ]);
}

function baseTheme(p: Palette): Extension {
  // 与 PTY 终端一致的字号/行高(由 Rust 经 URL `fs`/`lh` 传入,基准值不含
  // 全局 scale——WebView pageZoom 负责缩放)。
  const params = new URLSearchParams(location.search);
  const fontSize = Number(params.get('fs')) || 14;
  const lineHeight = Number(params.get('lh')) || 1.2;
  return EditorView.theme(
    {
      '&': {
        color: p.fg,
        backgroundColor: p.bg,
        height: '100%',
      },
      '.cm-scroller': {
        fontFamily: 'JetBrains Mono, ui-monospace, SFMono-Regular, monospace',
        fontSize: `${fontSize}px`,
        lineHeight: String(lineHeight),
      },
      '.cm-content': { caretColor: p.cursor },
      '.cm-cursor, .cm-dropCursor': { borderLeftColor: p.cursor },
      '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
        { backgroundColor: p.selection },
      '.cm-gutters': {
        backgroundColor: p.gutterBg,
        color: p.gutterFg,
        border: 'none',
      },
      '.cm-activeLine': { backgroundColor: p.activeLine },
      '.cm-activeLineGutter': { backgroundColor: p.activeLine, color: p.fg },
      '.cm-foldPlaceholder': {
        backgroundColor: 'transparent',
        border: 'none',
        color: p.comment,
      },
      '.cm-panels': { backgroundColor: p.panel, color: p.fg },
      '.cm-panel.cm-search input, .cm-panel.cm-search button': {
        backgroundColor: p.bg,
        color: p.fg,
        border: `1px solid ${p.comment}`,
      },
    },
    { dark: p === DARK },
  );
}

export function themeFor(scheme: string): Extension[] {
  const p = scheme === 'light' ? LIGHT : DARK;
  return [baseTheme(p), syntaxHighlighting(highlight(p))];
}
