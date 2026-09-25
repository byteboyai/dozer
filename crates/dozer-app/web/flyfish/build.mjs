// 生产构建:把 Flyfish renderer-text(code + markdown)源码打包成**离线、无
// CDN、无运行时 Node** 的确定性 IIFE 产物到
// `crates/dozer-app/assets/flyfish/renderers/text.iife.js`。
//
// 上游 flyfish-dev/file-viewer 用 Vite 产出同名 `text.iife.js`;这里用
// esbuild + 与上游 `packages/components/web-full/scripts/build-iife.mjs`
// 等价的 define/resolve,保证可复现且产物语义一致。
//
// - 不输出 source map(避免把本机绝对路径写进产物);去掉 legal comments
//   (许可证清单在 upstream/renderer-text/LICENSE 与 VENDORED_VERSION 维护)。
// - 先写临时文件,校验通过后再原子替换目标,构建失败不破坏已提交产物。
//
// 产物全局名必须与 flyfish-file-viewer-web-full.iife.js 里 lazy loader 期望
// 的 `FlyfishFileViewerWebFullRendererText` 一致。

import { build } from 'esbuild';
import { mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { assetFile, rendererTextBuildOptions } from './esbuild.config.mjs';

const result = await build({
  ...rendererTextBuildOptions(),
  write: false,
  logLevel: 'info',
});

if (result.outputFiles.length !== 1) {
  throw new Error(`expected exactly one output file, got ${result.outputFiles.length}`);
}

const bytes = result.outputFiles[0].contents;
const text = Buffer.from(bytes).toString('utf8');

// 硬性校验:产物不得引用 CDN / 构建机绝对路径 / source map。
// 注意 `http://www.w3.org/2000/svg` 之类的 XML 命名空间是合法且必要的,
// 这里只拦真正会发起网络请求的 CDN host 和构建机路径。
for (const banned of ['sourceMappingURL', assetFile, 'cdn.jsdelivr.net', 'unpkg.com', 'cdnjs.cloudflare.com', 'registry.npm']) {
  if (text.includes(banned)) {
    throw new Error(`refusing to write bundle: found banned token ${JSON.stringify(banned)}`);
  }
}
if (!text.includes('markdown-mermaid')) {
  throw new Error('refusing to write bundle: markdown-mermaid marker missing (wrong entry?)');
}

const dir = await mkdtemp(path.join(tmpdir(), 'dozer-flyfish-'));
const tmpFile = path.join(dir, 'text.iife.js');
await writeFile(tmpFile, bytes);

const sha = createHash('sha256').update(bytes).digest('hex');
const prev = await readFile(assetFile).catch(() => null);
const prevSha = prev ? createHash('sha256').update(prev).digest('hex') : null;

await rename(tmpFile, assetFile);
await rm(dir, { recursive: true, force: true });

console.log(`[flyfish] built -> ${assetFile}`);
console.log(`[flyfish] sha256 ${sha}${sha === prevSha ? ' (unchanged)' : ''}`);
