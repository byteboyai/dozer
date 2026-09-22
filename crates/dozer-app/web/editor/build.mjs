// 生产构建:把 CodeMirror 编辑器打包成**离线、无 CDN、无运行时 Node** 的
// 确定性产物到 `crates/dozer-app/assets/editor/`。
//
// 产物:`editor.js`(iife bundle)、`editor.css`(bundle 出来的样式)、
// `index.html`(带严格 CSP)、`fonts/JetBrainsMono.ttf`。
//
// 不输出 source map(避免把本机绝对路径写进产物);minify 后去掉所有
// legal comments(许可证清单在仓库 `THIRD_PARTY`/计划文档里维护)。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/editor');
const fontSrc = path.resolve(here, '../../assets/fonts/JetBrainsMono[wght].ttf');

await rm(outdir, { recursive: true, force: true });
await mkdir(path.join(outdir, 'fonts'), { recursive: true });

await build({
  entryPoints: [path.join(here, 'src/main.ts')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  // 字体由构建脚本单独拷到 outdir/fonts;CSS 里的 url('fonts/...') 保持原样,
  // 不当作 bundle 输入去解析。
  external: ['fonts/*'],
  outfile: path.join(outdir, 'editor.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/index.html'), path.join(outdir, 'index.html'));
await cp(fontSrc, path.join(outdir, 'fonts/JetBrainsMono.ttf'));

console.log('built ->', outdir);
