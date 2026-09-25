# Dozer Flyfish renderer-text build

Dozer 运行期只加载离线静态产物,不依赖 Node/Python。这个目录是
**开发期**构建入口:从 Flyfish 上游源码重新生成 Dozer 使用的 vendored
renderer bundle,并在其上叠加 Dozer 自己的 `markdown-media-lightbox` patch。

## 背景与来源

- Dozer 的 Markdown/Code 预览由 Flyfish `renderer-text` 渲染器承担。
- 上游包 `@file-viewer/web@2.2.2` / `@file-viewer/core@2.2.2` **不包含**
  renderer 源码,只发布编译后的 dist 与 viewer 资产。renderer 的 IIFE
  (`renderers/text.iife.js`)由上游 monorepo 的
  `packages/components/web-full/scripts/build-iife.mjs` 用 Vite 产出。
- 因此本目录把上游 renderer-text 源码**按文件**vendore 到
  `upstream/renderer-text/`,并固定到与 Dozer 现有产物对应的上游 tag。

| 项 | 值 |
|----|----|
| 上游仓库 | https://github.com/flyfish-dev/file-viewer |
| 上游 tag | `v2.2.2` |
| 上游 commit | `af2080ca10a2d415bd6d6877c36db5cb6f9d5567` |
| 源目录 | `packages/renderers/text/src/` |
| 许可证 | Apache-2.0,见 `upstream/renderer-text/LICENSE` |
| 依赖 | `@file-viewer/core@2.2.2`(npm 发布版)+ `marked` / `mermaid` / `highlight.js` / `diff2html` / `pako` |

`upstream/renderer-text/` 下的文件**逐字节来自上游 tag**,除新增的
`markdown-media-lightbox/` 子目录与对 `markdown.ts` 的 Dozer patch 外不应手改。
license 与来源记录必须随源码一起更新。

## 安装 / 测试 / 构建

```bash
cd crates/dozer-app/web/flyfish
npm ci
npm test          # 纯函数 + DOM 单测(Node test runner)
npm run typecheck
npm run build     # 生成并原子替换 ../../assets/flyfish/renderers/text.iife.js
npm run build:check   # 重放构建并与已提交产物比对;stale 则退出码非 0
```

`npm run build` 只更新 `assets/flyfish/renderers/text.iife.js`。它**不会**
重新引入 D5 已裁掉的 CAD/Office/工程资产,也**不**改动
`flyfish-file-viewer-web-full.iife.js`、`vendor/`、`wasm/` 或 `host.html`。

## 可复现性

- 产物用 esbuild 打包,关闭 source map 与 legal comments;`import.meta.url`
  在构建期替换为运行时 `document.currentScript.src`,产物中不含构建机路径、
  CDN host 或 `sourceMappingURL`。
- `npm run build:check` 重放构建并与提交产物逐字节比较,同时验证「可复现」
  和「已同步」。CI 应在源码或 lockfile 变更后运行它。

## 上游升级

1. 选一个上游 tag(优先与 `@file-viewer/web` 发布版本对应),记录其 commit。
2. 用 `git show <tag>:packages/renderers/text/src/<file>` 覆盖
   `upstream/renderer-text/` 下对应文件;同步 LICENSE。
3. 若上游依赖版本变化,更新 `package.json` 中对应 semver 并 `npm install`
   重建 `package-lock.json`。
4. `npm run build`,确认变更只影响预期范围,再更新 `VENDORED_VERSION` 的
   provenance 文本。
5. 跑 `cargo test -p dozer-app`,确认自定义协议仍能服务更新后的 bundle。

## 预期输出

- `crates/dozer-app/assets/flyfish/renderers/text.iife.js`
- 该文件的 SHA-256 会打印在构建日志里;连续两次 clean build 必须一致。
