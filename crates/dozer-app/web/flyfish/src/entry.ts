// renderer-text 的 IIFE 入口。上游 `packages/components/web-full/scripts/
// build-iife.mjs` 为每个 renderer 生成一个临时 entry,内容等价于下面这段:
// 把 textRenderer 同时挂到 `globalThis.FlyfishFileViewerWebFullRenderers` 的
// 短名(`text`)和 renderer id 上,供 flyfish-file-viewer-web-full.iife.js 的
// lazy loader 取用。
import { textRenderer } from '../upstream/renderer-text/index.ts';

const host = globalThis as typeof globalThis & {
  FlyfishFileViewerWebFullRenderers?: Record<string, unknown>;
};
const bucket =
  host.FlyfishFileViewerWebFullRenderers ||
  (host.FlyfishFileViewerWebFullRenderers = {});
bucket['text'] = textRenderer;
bucket[textRenderer.id] = textRenderer;
