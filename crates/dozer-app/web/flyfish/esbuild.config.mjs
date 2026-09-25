// 共享的 renderer-text 打包配置。build.mjs(写产物)和 build-check.mjs
// (重放比对)都从这里取配置,避免两处漂移导致 stale check 失真。

import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const here = path.dirname(fileURLToPath(import.meta.url));
export const assetFile = path.resolve(
  here,
  '../../assets/flyfish/renderers/text.iife.js'
);

// 上游源码用 TS 的 ESM 风格写成 `./code.js`,实际文件是 `./code.ts`。
// Vite/tsc 会自动做这个映射,esbuild 不会,这里补一个 resolver。
const tsJsResolver = {
  name: 'ts-js-resolver',
  setup(buildApi) {
    buildApi.onResolve({ filter: /\.js$/ }, (args) => {
      if (!args.path.startsWith('.')) return null;
      const abs = path.resolve(args.resolveDir, args.path);
      const ts = abs.replace(/\.js$/, '.ts');
      if (existsSync(ts)) return { path: ts };
      return null;
    });
  },
};

/**
 * 返回 esbuild 的 renderer-text IIFE 配置。调用方按需覆盖 write/logLevel
 * 等字段。
 */
export const rendererTextBuildOptions = () => ({
  plugins: [tsJsResolver],
  entryPoints: [path.join(here, 'src/entry.ts')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  treeShaking: true,
  globalName: 'FlyfishFileViewerWebFullRendererText',
  // Vite 允许 define 里塞任意表达式,esbuild 只接受标识符/字面量。这里把
  // `import.meta.url` 映射到一个运行时横幅变量,效果等价于上游 define。
  banner: {
    js: 'const __dozerImportMetaUrl = (typeof document !== "undefined" && document.currentScript && document.currentScript.src) || (typeof location !== "undefined" ? location.href : "");',
  },
  define: {
    'process.env.NODE_ENV': JSON.stringify('production'),
    'process.env': JSON.stringify({ NODE_ENV: 'production' }),
    'import.meta.url': '__dozerImportMetaUrl',
  },
});
