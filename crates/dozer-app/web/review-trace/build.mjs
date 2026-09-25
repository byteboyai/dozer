// 生产构建:把会话审阅 trace host 打包成**离线、无 CDN、无运行时 Node** 的
// 确定性产物到 `crates/dozer-app/assets/review-trace/`。
//
// 产物:`review-trace.js`(iife bundle)、`review-trace.css`(从
// `main.tsx` 顶部 `import './styles.css'` 抽出的样式)、`host.html`(带
// 严格 CSP,原样拷贝)。
//
// 不输出 source map;minify 后去掉所有 legal comments。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/review-trace');

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

await build({
  entryPoints: [path.join(here, 'src/main.tsx')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'css' },
  outfile: path.join(outdir, 'review-trace.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
