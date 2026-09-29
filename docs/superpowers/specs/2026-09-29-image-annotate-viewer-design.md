# 图片预览换用 Annotorious + OpenSeadragon（设计稿）

> 状态：已与用户逐节确认（2026-09-28），待用户终审后转 `writing-plans` 出实现计划。
> 触发链路：用户先要求给 Flyfish 图片预览的工具条打开缩放功能 → 发现 Flyfish 工具条里大量按钮（print/download/share 等）在当前沙盒 webview 环境下不工作 → 用户改为要求直接换查看器。
> 范围来源：本文档记录的一轮 brainstorming 逐问逐答（详见下方"已决问题"）。

## 1. 背景与动机

图片预览目前经 `dozer://flyfish/` webview 渲染（`crates/dozer-app/src/preview/router.rs::classify_preview` 把 `.png/.jpg/.jpeg/.gif/.webp/.bmp/.ico/.tif/.tiff` 归为 `PreviewKind::Rendered` → `RouteReason::KnownMediaExtension`），底层是第三方 vendor 的 `@file-viewer/web`（`flyfish-file-viewer-web-full.iife.js` + `renderers/image.iife.js`）。

`host.html`（`crates/dozer-app/assets/flyfish/host.html:51`）目前显式 `el.setAttribute('toolbar', 'false')`，禁用了 Flyfish 自带工具条——起因是工具条里的 print/download/share/export-html 等按钮在当前"CSP 无网络 + 隔离 webview"环境下无法正常工作，只有 zoom-in/zoom-out/zoom-reset 是用户真正需要、且理论上能工作的功能。与其为了三个按钮打开一整条有一堆残废按钮的工具条，不如换一个更贴合需求的查看器。

同时，用户对"能否把图片局部信息精确标注出来发给 AI agent"有需求（annotorious 是为此调研的起点）。本轮范围只做**查看器替换 + 标注能力**，标注结果发给 agent 的通道**明确排除在外**，留待后续单独设计。

## 2. 范围（已决问题）

| 问题 | 决定 |
|---|---|
| 这次做到哪一步 | 换查看器 + 接上标注能力，**不接 agent** |
| 缩放/平移技术选型 | `@annotorious/openseadragon`（annotorious 官方 OpenSeadragon 集成），而非纯 `<img>` + 自研 pan/zoom |
| tif/tiff 是否要一并迁移 | **不迁移**，仍走 Flyfish 旧通道（浏览器引擎本身不能原生解码 TIFF，Flyfish 内部自带解码器，OpenSeadragon 没有） |
| 动态 gif 是否接受丢动画 | **不接受**，gif 也留在 Flyfish（OpenSeadragon 把图片画进 canvas 静态帧，会丢动画，用户判定这是不可接受的回退） |
| 标注支持的形状 | 矩形 + 点 + 任意多边形（annotorious 全形状能力） |
| 标注是否需要文字说明 | 需要（annotorious 自带的弹层文字编辑器，不必自研） |
| 标注是否要持久化 | **不需要**，本轮只放内存，随 tab/应用生命周期清空；避免过早定死格式，将来接 agent 时再决定序列化形态 |

**因此新查看器覆盖的扩展名**：`png / jpg / jpeg / webp / bmp / ico`。
**继续走 Flyfish 的扩展名**：`gif / tif / tiff`，以及所有非图片的 `pdf/doc/docx/ppt/pptx/mp3/wav/ogg/flac/m4a/aac/mp4/mov/webm/avi/mkv/woff/woff2/ttf/otf`（`router.rs::is_known_media_extension` 现有列表不变）。

## 3. 架构与路由

不新增 `PreviewKind` 变体，仍复用 `PreviewKind::Rendered`；分流发生在 webview URL 构建这一层（现有 `webview.rs::flyfish_url` 旁新增一个判定 + URL 构建函数），而不是 `classify_kind`：

- 新增一个扩展名子集判定（暂定 `prefers_image_annotate_viewer(path)`），命中 → 构建 `dozer://image-annotate/host.html?p=...&theme=...` URL。
- 未命中（含 gif/tif/tiff 及所有非图片媒体）→ 沿用现有 `flyfish_url`。

新增独立协议命名空间 `dozer://image-annotate/`，完整照抄 `dozer://html/` 的隔离模式（`crates/dozer-app/src/assets.rs` 里 `dozer://html/` 分支是模板）：
- 自己的 `host.html`（新写，不复用 Flyfish 的 `host.html`）。
- 图片字节读取复用现有 `serve_allowlisted_file`/`serve_html_file` 的路径穿越防护逻辑（`..`、跨目录校验）。
- **不**把新渲染器塞进 `crates/dozer-app/assets/flyfish/` 目录——那棵目录整体是 vendor 产物（`flyfish-viewer-manifest.json`/`VENDORED_VERSION`），混入手写代码会和以后升级 Flyfish 打架。新目录建议 `crates/dozer-app/assets/image-annotate/`。

`webview.rs` 新增一个 URL 构建函数（对称于 `flyfish_url`/`html_url`），带上与现有一致的查询串约定：`p`（路径）、`theme`、以及 T9 envelope 需要的 `proj/panel/tab/doc`。

## 4. Webview host 与消息通道

新 host 不复用 `FlyfishEvent`/`__dozerFlyfishPost`，单独起一套（沿用 `webview_protocol.rs::WebviewEnvelope<T>` 泛型 + `HostBinding` 校验机制，只是 payload 类型不同）：

- `host.html` 里设 `window.__dozerImageAnnotatePost`，行为对齐现有 `__dozerFlyfishPost`：`post(payload)` 把 envelope 通过 `window.ipc.postMessage` 送出；project/panel/tab/doc 由 URL 查询串带、Rust 侧用 `HostBinding` 校验归属，JS 不能自报路径。
- 新事件枚举 `ImageAnnotateEvent`，变体：
  - `Ready`：host 脚本初始化完成（对齐 Flyfish 的 `ready`）。
  - `DocumentLoaded`：图片解码完成、OpenSeadragon 已挂载（对齐 Flyfish 的 `document_loaded`，Rust 侧据此才把原生子视图设为可见）。
  - `Failed { message, recoverable }`：渲染失败 → 走 T1 `preview_fallback_page` 统一 fallback（对齐 Flyfish 现有失败处理）。
  - 不设 `Title` 事件：图片类 tab 标题取自文件名，不像 Markdown 那样依赖 `document.title`，与 Flyfish 的 title 通道无对应需求。
  - `AnnotationsChanged { annotations: Vec<...> }`：标注增删改时把当前全部标注序列化回传。本轮不落盘，但 Rust 侧仍需接住这个事件——不然标注数据只活在 JS 里，切 tab/关 tab 就彻底丢失且 Rust 侧毫无感知，将来"是否提示未处理标注"之类的功能也无从谈起。
- Rust 侧新增 `parse_image_annotate_event`（对齐 `parse_flyfish_event` 的解析/容错方式），在 `runtime.rs` 现有统一 IPC handler 里新增一个并列分支，校验 `HostBinding` 后转发 `Message::ImageAnnotateEvent(binding, event)` 进 iced 事件循环。

## 5. 标注 UI 与缩放控制

- **缩放/平移**：使用 OpenSeadragon 自带的 `showNavigationControl`（放大/缩小/回到原图/全屏 + 滚轮 + 拖拽平移），不自研工具条。控件样式对齐 `host.html` 现有的 `--preview-bg` 等 CSS 变量，深浅色跟随 ByteBoy2077 主题（金 `#F2D94E` 可用作标注高亮色，与"甲方动作专属"语义一致——标注是用户对 AI 产物的介入动作）。
- **标注形状**：矩形 + 点 + 任意多边形，用 `@annotorious/openseadragon` 自带绘制工具；需要一个小工具条切换"当前画哪种形状 / 回到查看模式"——这是该库集成本身预期要接的部件，不是自研发明。
- **文字说明**：每个标注用 annotorious 自带的弹层编辑器输入文字备注（标准能力，不必自研编辑 UI）。
- **模式边界**：默认停留在查看/平移模式，拖拽 = 平移图片；只有显式选中某个绘制工具后拖拽才变成画形状，避免和 OSD 的平移手势冲突。
- **数据生命周期**：标注状态挂在对应 PreviewTab 的运行时状态里，随 tab 关闭 / 应用重启清空，不落盘、不定义任何持久化格式（W3C Web Annotation JSON 等留到接 agent 时再决定，避免被仓促定的格式绑死）。

## 6. 第三方库引入与 CSP

- 新增 vendor 依赖：`openseadragon`（UMD/ESM 预构建包）+ `@annotorious/openseadragon`（含其依赖的 `@annotorious/annotorious` 核心）。
- 遵循"核心不依赖 Node/Python"的约束：不在 `cargo build` 流程里跑 Node 构建，而是把预构建好的 bundle 文件直接下载、原样存进 `crates/dozer-app/assets/image-annotate/vendor/`，配一个类似 Flyfish `VENDORED_VERSION` 的版本记录文件，方便以后升级/审计。
- CSP 延续 `dozer://html/` 的"无网络"策略：所有 JS/CSS 必须是本地 vendor 文件，不能有 CDN `<script src="https://...">`；图标字体等资源内嵌或本地化。

**已知风险（不阻塞设计，实现阶段验证，写入验收清单）**：`.ico`/`.bmp` 在 WebKit（wry 用的引擎）里通过 `<img>`/canvas 解码的兼容性没有 Flyfish 稳，需要实测；某个格式解码失败时，行为上要能优雅回退（`Failed` 事件 → fallback 页），不能白屏。

## 7. 测试与验收标准

**自动化测试**：
- `assets.rs`：新 `dozer://image-annotate/` 协议 handler 的路径穿越防护测试，复用 `serve_allowlisted_file` 已有测试模式。
- `router.rs`/`webview.rs`：扩展名分流的表驱动测试——覆盖 png/jpg/jpeg/webp/bmp/ico 走新 host，gif/tif/tiff 走 Flyfish 不变。
- `webview_protocol.rs`（或新模块）：`parse_image_annotate_event` 对齐 `parse_flyfish_event` 的测试套路——合法 JSON → 正确枚举；`HostBinding` 不匹配 → 拒绝；非法/半截 JSON → 不 panic。

**人工验收清单**（JS/webview 侧无自动化测试基础设施）：
- [ ] 打开 png/jpg/jpeg/webp/bmp/ico 各一张，确认走新查看器且能正常显示
- [ ] 打开 gif/tif/tiff 各一张，确认仍走 Flyfish（gif 动画不丢）
- [ ] 缩放（滚轮 + 导航控件按钮）、平移（拖拽）正常
- [ ] 画矩形 / 点 / 多边形标注，能加文字备注
- [ ] 关闭 tab 后重新打开同一图片，确认标注已清空（符合"不持久化"设计）
- [ ] 深色/浅色主题切换，查看器与标注高亮色跟随
- [ ] 故意打开一个损坏的图片文件，确认走 `Failed` → fallback 页而不是白屏或卡死

## 8. 明确排除（本轮不做）

- 标注结果发给 agent 的任何通道（文本描述 / 裁剪图 / 结构化 payload）——留待后续单独设计，届时需要重新审视本文档"不持久化"的决定是否要改。
- 标注数据持久化（sidecar 文件 / 项目级存储）。
- tif/tiff/gif 迁移到新查看器。
- Flyfish 工具条的其他功能（print/download/share/export-html）在新查看器里的等价替代——用户明确只要放大缩小 + 标注。
