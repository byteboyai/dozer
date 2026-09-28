// 生产构建:把 ag-grid-community 打包成**离线、无 CDN** 的确定性产物到
// `crates/dozer-app/assets/tabular-host/`(index.html 自带严格 CSP)。
import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/tabular-host');

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

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
  loader: { '.css': 'css' },
  outfile: path.join(outdir, 'tabular-host.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/index.html'), path.join(outdir, 'index.html'));

console.log('built ->', outdir);
