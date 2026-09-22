// 语言注册表(前端侧)。Rust 路由传来的 language token 与
// `preview/native_editor.rs::extension_to_syntax` **一一对应**;未知 token 回退
// 纯文本(无高亮),不抛错。
//
// 覆盖策略:官方 `@codemirror/lang-*` 优先;官方没有的用社区包(zig/makefile/
// elixir/graphql);其余用 `@codemirror/legacy-modes` 的 StreamLanguage。

import type { Extension } from '@codemirror/state';
import { StreamLanguage } from '@codemirror/language';

import { rust } from '@codemirror/lang-rust';
import { python } from '@codemirror/lang-python';
import { javascript } from '@codemirror/lang-javascript';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import { html } from '@codemirror/lang-html';
import { css } from '@codemirror/lang-css';
import { xml } from '@codemirror/lang-xml';
import { sql } from '@codemirror/lang-sql';
import { yaml } from '@codemirror/lang-yaml';
import { php } from '@codemirror/lang-php';
import { sass } from '@codemirror/lang-sass';

import { zig } from 'codemirror-lang-zig';
import { makefile } from 'codemirror-lang-makefile';
import { elixir } from 'codemirror-lang-elixir';
import { graphql } from 'cm6-graphql';

import { shell } from '@codemirror/legacy-modes/mode/shell';
import { toml } from '@codemirror/legacy-modes/mode/toml';
import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile';
import { diff } from '@codemirror/legacy-modes/mode/diff';
import { lua } from '@codemirror/legacy-modes/mode/lua';
import { r } from '@codemirror/legacy-modes/mode/r';
import { swift } from '@codemirror/legacy-modes/mode/swift';
import { haskell } from '@codemirror/legacy-modes/mode/haskell';
import { protobuf } from '@codemirror/legacy-modes/mode/protobuf';
import { go } from '@codemirror/legacy-modes/mode/go';
import { ruby } from '@codemirror/legacy-modes/mode/ruby';
import { c, cpp, java, kotlin, scala } from '@codemirror/legacy-modes/mode/clike';

function legacy(mode: Parameters<typeof StreamLanguage.define>[0]): Extension {
  return StreamLanguage.define(mode);
}

/// language token → CodeMirror 语言扩展。未知/纯文本返回 `null`。
export function languageFor(token: string): Extension | null {
  switch (token) {
    // 官方语言包
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
    case 'markdown':
      return markdown();
    case 'html':
      return html();
    case 'css':
      return css();
    case 'scss':
      // `sass()` 默认即 SCSS 语法。
      return sass();
    case 'xml':
      return xml();
    case 'sql':
      return sql();
    case 'yaml':
      return yaml();
    case 'php':
      return php();

    // 社区语言包
    case 'zig':
      return zig();
    case 'makefile':
      return makefile();
    case 'elixir':
      return elixir();
    case 'graphql':
      return graphql();

    // legacy StreamLanguage
    case 'bash':
      return legacy(shell);
    case 'toml':
      return legacy(toml);
    case 'dockerfile':
      return legacy(dockerFile);
    case 'diff':
      return legacy(diff);
    case 'lua':
      return legacy(lua);
    case 'r':
      return legacy(r);
    case 'swift':
      return legacy(swift);
    case 'haskell':
      return legacy(haskell);
    case 'protobuf':
      return legacy(protobuf);
    case 'go':
      return legacy(go);
    case 'ruby':
      return legacy(ruby);
    case 'c':
      return legacy(c);
    case 'cpp':
      return legacy(cpp);
    case 'java':
      return legacy(java);
    case 'kotlin':
      return legacy(kotlin);
    case 'scala':
      return legacy(scala);

    default:
      return null;
  }
}
