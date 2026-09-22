// 语言注册表(前端侧)。Rust 路由传来的 language token 与
// `preview/native_editor.rs::extension_to_syntax` 保持一致;未知 token 回退
// 纯文本(无高亮),不抛错。

import type { LanguageSupport } from '@codemirror/language';
import { javascript } from '@codemirror/lang-javascript';
import { rust } from '@codemirror/lang-rust';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import { html } from '@codemirror/lang-html';
import { css } from '@codemirror/lang-css';
import { python } from '@codemirror/lang-python';

export function languageFor(token: string): LanguageSupport | null {
  switch (token) {
    case 'rust':
      return rust();
    case 'python':
      return python();
    case 'javascript':
      return javascript();
    case 'jsx':
      return javascript({ jsx: true });
    case 'typescript':
      return javascript({ typescript: true });
    case 'tsx':
      return javascript({ typescript: true, jsx: true });
    case 'json':
    case 'jsonc':
      return json();
    case 'html':
      return html();
    case 'css':
    case 'scss':
      return css();
    case 'markdown':
      return markdown();
    default:
      return null;
  }
}
