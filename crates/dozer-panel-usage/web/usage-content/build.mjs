// 生产构建:把用量面板内容侧打包成离线、无 CDN、无运行时 Node 的确定性
// 产物到同 crate 的 `crates/dozer-panel-usage/assets/usage-content/`。不输出
// source map;minify 后去掉所有 legal comments。
//
// 产物全是本仓(firet-party)代码,不含第三方 legal comment(见同目录
// `NOTICE.md`)。消费方(Dozer / Digger)从本 crate 的 `assets/usage-content/`
// 单一来源服务该 host,不各自留副本。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/usage-content');

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
  outfile: path.join(outdir, 'usage-content.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
