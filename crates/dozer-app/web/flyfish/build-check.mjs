// stale check:源码/lockfile 改动但提交的 text.iife.js 未同步时失败。
//
// 做法:构建到内存(不写文件),与 `assets/flyfish/renderers/text.iife.js`
// 逐字节比较。一致则通过,不一致说明产物需要 `npm run build` 重新同步。
//
// 这是「重放构建 + 比对」,比单独存一份 hash 更强:它同时验证构建可复现
// 和产物已同步。

import { build } from 'esbuild';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';

import { assetFile, rendererTextBuildOptions } from './esbuild.config.mjs';

const result = await build({
  ...rendererTextBuildOptions(),
  write: false,
  logLevel: 'silent',
});

const fresh = Buffer.from(result.outputFiles[0].contents);
const committed = await readFile(assetFile);
const freshSha = createHash('sha256').update(fresh).digest('hex');
const committedSha = createHash('sha256').update(committed).digest('hex');

if (freshSha !== committedSha) {
  console.error('[flyfish] STALE: committed text.iife.js does not match a fresh build.');
  console.error(`  committed: ${committedSha}`);
  console.error(`  fresh:     ${freshSha}`);
  console.error('  run `npm run build` and commit the regenerated bundle.');
  process.exit(1);
}

console.log(`[flyfish] up to date (${freshSha})`);
