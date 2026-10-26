// 渲染冒烟:用 esbuild 把 `src/render-smoke.tsx` 打成 node 可执行的 ESM
// (JSX 不能直接被 `node --test` 的类型剥离跑),再 import 执行。
import { build } from 'esbuild';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = await mkdtemp(path.join(tmpdir(), 'group-chat-smoke-'));
const outfile = path.join(dir, 'smoke.mjs');

await build({
  entryPoints: [path.join(here, 'src/render-smoke.tsx')],
  bundle: true,
  format: 'esm',
  platform: 'node',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'empty' },
  outfile,
  logLevel: 'silent',
});

await import(pathToFileURL(outfile).href);
