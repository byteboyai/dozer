# Markdown 图片与 Mermaid 灯箱 Implementation Plan

**Goal:** 在 Dozer 的 Markdown 渲染预览中，点击正文图片或已经渲染完成的
Mermaid 图后，在当前 Flyfish WebView 内打开可缩放、可平移、可键盘操作的灯箱；
关闭后无损恢复原文滚动位置和焦点。

**Architecture:** 能力落在 Flyfish Markdown renderer 源码中，而不是
`host.html` 注入、iced 浮层或 Rust IPC。Markdown renderer 在渲染完成后为正文
图片和 Mermaid SVG 安装统一的 `media lightbox`；灯箱只克隆/引用已经安全渲染的
DOM，不重新解析 Markdown、不重新执行 Mermaid。开发期由可复现的 Node 构建生成
`text.iife.js`，Dozer 运行期仍只加载离线静态产物，不新增 Node/Python 依赖。

**当前事实（2026-09-25）:**

- Markdown 预览走 `dozer://flyfish/host.html` 和 vendored Flyfish 2.2.2。
- `crates/dozer-app/assets/flyfish/VENDORED_VERSION` 标记为
  `2.2.2 (web-full, D5 精简)`；仓库当前只有构建产物，没有 Flyfish renderer 源码
  或可复现的 Flyfish 构建入口。
- standalone 图片 renderer 已有自己的点击灯箱和缩放能力，本计划不改变该管线。
- Markdown renderer 已把 `language-mermaid` code block 转为经过净化的 SVG，但只做
  `max-width: 100%`，没有目标级灯箱。
- Markdown viewer 已有 60%～240% 的整页缩放 provider；媒体灯箱必须维护独立缩放
  状态，不能修改或冒充整页 provider。

---

## 0. 边界与验收口径

### 包含

- Markdown 正文中的 `<img>`。
- Markdown fenced Mermaid 渲染出的 `.markdown-mermaid-svg`。
- 鼠标点击、键盘打开/关闭、按钮缩放、滚轮缩放、拖拽平移、双击切换适应/100%。
- 深色/浅色主题、窄预览栏、大图、大 SVG、GIF/SVG/位图回归。
- renderer 卸载时完整清理事件、DOM 和 object URL（若实现中产生）。

### 不包含

- standalone 图片 renderer 的交互重写。
- `.mmd`/`.mermaid` 独立 drawing renderer 的灯箱改造。
- PDF、Office、HTML iframe、代码预览中的图片。
- 独立 macOS 原生窗口、iced modal 或新的 Rust IPC 消息。
- Mermaid 源码再次执行、导出、下载、复制图片。

### 产品验收

1. 点击 Markdown 图片或 Mermaid 图，灯箱覆盖当前预览内容且不越出该预览 WebView。
2. 初次打开使用 `contain` 适应可用视口，不主动放大小于视口的位图。
3. 支持缩放范围 10%～500%；按钮、`+`/`-`、`0`/“适应”、修饰键+滚轮可用。
4. 放大后可拖拽平移；光标和边界行为明确，不允许内容永久拖丢。
5. `Escape`、关闭按钮和点击非内容背景可关闭；点击图片/SVG 本体不误关闭。
6. 打开前的焦点和 Markdown 滚动位置在关闭后恢复。
7. 键盘用户可聚焦媒体并用 `Enter`/`Space` 打开；灯箱有 dialog 语义和可读名称。
8. Mermaid 灯箱使用现有已净化 SVG 的深克隆，不重新调用 Mermaid renderer。
9. 打开/关闭灯箱不改变 Markdown 整页 zoom provider 的 scale。

---

## T1. 建立可复现的 Flyfish renderer 源码与构建门槛

**目的:** 方案 1 的前提是改源码并重建；禁止直接编辑压缩后的
`assets/flyfish/renderers/text.iife.js`。

**Files:**

- Create: `crates/dozer-app/web/flyfish/package.json`
- Create: `crates/dozer-app/web/flyfish/package-lock.json`
- Create: `crates/dozer-app/web/flyfish/README.md`
- Create: `crates/dozer-app/web/flyfish/build.mjs`
- Create: `crates/dozer-app/web/flyfish/src/`（从 Flyfish 2.2.2 对应源码取得的最小
  renderer-text 源码闭包，保留上游 LICENSE/NOTICE 和来源记录）
- Modify: `crates/dozer-app/assets/flyfish/VENDORED_VERSION`

- [x] 从 `@file-viewer/web@2.2.2` / 对应上游 tag 获取与现有 bundle 一致的源码；
      记录包名、版本、上游 commit/tag、许可证和原始 tarball integrity。
- [x] 若发布包不含可构建源码，固定上游 git commit 后导入 renderer-text 的最小源码
      闭包；不得根据压缩 bundle 反向手抄一份不可追溯实现。
- [x] 锁定 Node 依赖版本并提交 lockfile；开发构建允许 Node，Dozer 运行时不得依赖它。
- [x] `build.mjs` 输出到临时目录，成功后原子替换
      `assets/flyfish/renderers/text.iife.js`；构建失败不得破坏已提交产物。
- [x] 构建只更新 text renderer；不得把已按 D5 裁掉的 CAD/Office/工程资产重新带回。
- [x] README 写明：安装、测试、构建、同步产物、上游升级、许可证核对和预期输出路径。
- [x] 在 `VENDORED_VERSION` 中补充 `renderer-text` 的源码 provenance 和本地 patch 标识。
- [x] 建立 reproducibility 检查：clean checkout 连续构建两次的 SHA-256 一致；若 bundler
      含时间戳，先消除时间戳而不是在校验中忽略整个文件。

**硬门槛:** 无法取得许可证兼容且与 2.2.2 对应的 renderer 源码时停止实施并回报，
不得退化为修改 minified IIFE 或偷偷采用 `host.html` 注入方案。

**自动化:** `npm ci && npm test && npm run build && npm run build:check`。

**回退点:** 删除新增 `web/flyfish` 工程并恢复原 `text.iife.js` 与
`VENDORED_VERSION`；Rust/host 无变化。

---

## T2. 抽出媒体识别与纯缩放几何

**目的:** 先把最容易出错的媒体边界、适应比例和缩放锚点做成无 DOM 的纯逻辑。

**Files:**

- Create: `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/geometry.ts`
- Create: `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/geometry.test.ts`
- Create: `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/types.ts`

- [x] 定义 `MediaDescriptor`：`kind: "image" | "mermaid"`、可访问名称、固有宽高、
      源节点；位图在 `naturalWidth/naturalHeight` 可用前不得错误计算为 0%。
- [x] 定义 `ViewportState`：scale、translation、viewport/content 尺寸及缩放锚点。
- [x] 实现 `fitScale`：上限 100%、下限 10%，在扣除工具栏和安全边距后 contain。
- [x] 实现 `zoomAtPoint`：10%～500% clamp，缩放前后指针下的内容坐标保持不变。
- [x] 实现 pan clamp：内容小于视口时居中；内容大于视口时至少保留可恢复边缘，
      resize 后重新 clamp。
- [x] 为零尺寸、极端长宽比、1px 图片、超大 SVG、视口 resize、连续浮点缩放写表驱动
      测试；禁止出现 `NaN`/`Infinity`。

**自动化:** Node 单测只测纯函数，不依赖 WebKit 或截图。

---

## T3. 在 Flyfish Markdown renderer 内实现通用灯箱

**目的:** 建立单实例、可销毁、无全局泄漏的 DOM 控制器。

**Files:**

- Create: `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/index.ts`
- Create: `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/styles.ts`
- Create: `crates/dozer-app/web/flyfish/src/markdown/media-lightbox/dom.test.ts`
- Modify: Flyfish Markdown renderer 对应源码文件（实际路径以 T1 导入后的上游结构为准，
  不在计划中伪造文件名）

- [x] renderer 完成 Markdown HTML 和 Mermaid 转换后，调用
      `installMarkdownMediaLightbox({ viewer, article, theme })`，返回 `destroy()`。
- [x] 仅匹配 `article` 内成功加载的图片和
      `.markdown-mermaid[data-markdown-mermaid="rendered"] > svg`；错误态 Mermaid 源码块
      不可打开。
- [x] 普通图片复用已解析 URL；Mermaid 使用 `cloneNode(true)` 深克隆已经净化的 SVG。
      克隆前移除重复 `id` 风险或将所有 SVG 内部 ID/引用加灯箱实例前缀，确保
      marker、clipPath、gradient 不失效也不污染正文 DOM。
- [x] 创建一个 `role="dialog"`、`aria-modal="true"` 的 overlay；工具栏包含关闭、
      缩小、当前百分比、放大、100%、适应窗口。图标按钮均有 `aria-label` 和 tooltip。
- [x] 打开时记录 activeElement、正文滚动位置和整页 zoom scale；关闭时恢复焦点与滚动，
      并断言整页 scale 未被修改。
- [x] 灯箱打开时锁定 Markdown viewer 自身滚动，只让灯箱消费拖拽/缩放；关闭时恢复原
      `overflow`，不得硬编码成与打开前不同的值。
- [x] 点击背景关闭只认 `event.target === overlay`；媒体本体、工具栏和空白画布中的拖拽
      均不误关。
- [x] `Escape` 关闭；`+`/`=` 放大、`-` 缩小、`0` 回 100%、`F` 适应；输入框或其他
      editable target 上不截获快捷键。
- [x] 滚轮仅在 `Meta`/`Ctrl` 修饰时缩放，普通滚轮保留为画布滚动/平移，避免用户浏览
      Markdown 时误触放大。
- [x] Pointer Events 实现拖拽，并处理 pointer capture、cancel、窗口失焦和多指场景；
      一期不实现 pinch，但第二指针不得破坏状态。
- [x] ResizeObserver 在预览栏尺寸变化时重新 clamp；处于 fit 模式则重新 fit，手动 scale
      模式保留 scale。
- [x] renderer `unmount()` 调用 lightbox `destroy()`，移除 observer、document listener、
      pointer capture、临时 DOM 与可能的 URL。

**样式约束:**

- overlay 使用 fixed inset 0，z-index 只在 Flyfish 渲染 surface 内竞争，不引入 iced
  遮挡处理或 `App::preview_desired` 分支。
- 深色背景对齐 ByteBoy2077 `#0a0e16`，控件不使用甲方动作专属金色作为普通状态；
  浅色主题保持足够对比度。
- 原文图片和 Mermaid 增加 `cursor: zoom-in`、可见 focus ring；灯箱内可拖拽状态使用
  `grab`/`grabbing`。
- transform 只落在单一 stage 元素，使用 `translate3d + scale`；缩放/拖拽期间不重新执行
  Mermaid，不反复改 SVG width/height 引发布局抖动。

**自动化:** DOM 测试覆盖打开、两类媒体、关闭方式、焦点恢复、键盘、背景点击、
destroy 幂等、重复打开不积累监听器、SVG ID 重写。

---

## T4. 生成并同步 Dozer vendored 资产

**目的:** 让源码变更真实进入 Dozer 使用的离线 bundle，并防止只改源码没更新产物。

**Files:**

- Modify: `crates/dozer-app/assets/flyfish/renderers/text.iife.js`（构建生成，禁止手改）
- Modify: `crates/dozer-app/assets/flyfish/VENDORED_VERSION`
- Modify: `crates/dozer-app/assets/flyfish/flyfish-viewer-assets.json`（仅当现有 manifest
  记录校验值/尺寸且构建确实要求）

- [x] clean build 生成 `text.iife.js`，核对 bundle 中同时保留 Markdown、Mermaid、搜索、
      整页 zoom provider 与新增 lightbox；不得意外引入网络请求。
- [x] 比较构建前后资产清单和总体积；只允许预期的 text renderer 增量。
- [x] 更新版本/provenance 文本，明确这是 2.2.2 + Dozer markdown-media-lightbox patch，
      不冒充上游新版本。
- [x] 增加生成物 stale check：源码/lockfile 改动而 bundle 未同步时 CI/本地检查失败。
- [x] `host.html` 不增加媒体选择器、事件监听器或灯箱 CSS；它仍只负责 Dozer host、主题、
      load 生命周期和 IPC bridge。

**自动化:** 构建检查 + `cargo test -p dozer-app assets`，确认自定义协议仍能服务更新后的
bundle，且不放宽 `__file__` 白名单。

**实施记录（T1–T4）:**

- 提交边界调整（经确认）：T3 把重建后的 `text.iife.js` 一并提交，使该提交自身即可通过
  `build:check`；T4 不再产生新文件，只保留「产物同步 + provenance + stale check」的验证
  语义。VENDORED_VERSION 的 provenance 文本在 T1 已落地。
- 供应链硬门槛：上游 npm 包 `@file-viewer/web@2.2.2` / `@file-viewer/core@2.2.2` 不含
  renderer 源码；源码取自 `github.com/flyfish-dev/file-viewer` tag `v2.2.2`
  （commit `af2080ca10a2d415bd6d6877c36db5cb6f9d5567`，Apache-2.0）。
- 构建器替换：Dozer 侧用 esbuild 复刻上游 Vite 的 IIFE 语义（global 名、`ts-js-resolver`、
  `import.meta.url` 映射），关闭 sourcemap/legalComments，禁止 CDN/绝对路径。
- 体积：`text.iife.js` 原 3,568,420 B → esbuild baseline 3,841,580 B（换 bundler）
  → 灯箱后 3,855,783 B（+14,203 B，唯一增量）。`renderers/` 其他文件、manifest、
  `host.html` 均未改动。
- 可复现：连续 clean build SHA-256 一致（`a2a177c9…`）；`npm test` 44 项全过；
  `npm run typecheck`、`npm run build:check`、`cargo test -p dozer-app assets`（30 项）全过。
- 未决：`crates/dozer-app/web/flyfish/node_modules/` 已加入 `.gitignore`；`package-lock.json`
  已提交，`npm ci` 可复现。

---

## T5. WebKit 真机回归与发布验收

**目的:** Node DOM 测试不能替代 WKWebView 的 pointer、SVG 和焦点行为，最后必须走真实
Dozer 子 WebView。

**Fixture:**

- Create: `crates/dozer-app/fixtures/preview/markdown-media-lightbox.md`
- Create: 小 PNG、透明 PNG、动画 GIF、带 viewBox 的 SVG、超宽/超高图片 fixture；优先
  生成或使用仓库自有素材，禁止引入来源不明资产。

- [ ] fixture 同时包含相对路径图片、带 alt/无 alt 图片、链接包裹图片、两个 Mermaid、
      Mermaid gradient/marker/长图、故意失败的 Mermaid 源码块。
- [ ] 明确链接包裹图片优先级：点击图片开灯箱，点击链接的非图片区域仍导航；键盘聚焦
      不产生嵌套交互元素的非法语义（必要时只复用外层链接焦点，不给 img 加 tabindex）。
- [ ] 深色、浅色主题分别验证打开、按钮、focus ring、SVG 文字和遮罩对比度。
- [ ] 窄栏、宽栏、窗口 resize、切 tab、切 Rendered/Source、刷新文件、关闭 tab 时验证
      无残留 overlay、无崩溃、无滚动跳跃。
- [ ] 验证 Markdown 整页缩放先设为非 100%，再开/关媒体灯箱后原 scale 不变。
- [ ] 验证同文档连续打开不同媒体、快速开关 50 次，监听器和 DOM 节点数不增长。
- [ ] 对大 Mermaid 和大位图观察交互期间 CPU/RSS；拖拽和缩放不触发 Mermaid 重绘。
- [ ] standalone PNG/JPG 预览原有灯箱、PDF、CodeMirror、JSON viewer 全部 smoke test，
      确认 text renderer patch 未影响其他管线。
- [ ] macOS 打包后从 `.app/Contents/Resources/flyfish` 再验一次，确认构建脚本携带新产物，
      全程无 CDN/网络依赖。

**最终命令:**

```bash
cd crates/dozer-app/web/flyfish
npm ci
npm test
npm run typecheck
npm run build:check

cd ../../../..
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
scripts/build-macos-app.sh debug
```

---

## 实施顺序与提交边界

1. **T1 单独提交:** 可复现源码、许可证、构建与未改变行为的 baseline 产物。
2. **T2 单独提交:** 纯几何和单测，不接 UI。
3. **T3 单独提交:** renderer 源码灯箱和 DOM 测试。
4. **T4 单独提交:** 生成后的 vendored bundle、版本/provenance 和 stale check。
5. **T5 单独提交:** fixtures、真机回归修正和验收记录。

每个提交都必须可构建、可单独回退。不得把 T1 的供应链变化和 T3 的交互实现压进同一
提交，否则出现 bundle 回归时无法区分是上游源码校准问题还是灯箱逻辑问题。

## 完成定义

- 图片和 Mermaid 均满足 §0 产品验收 1～9。
- 灯箱实现只存在于 Flyfish Markdown renderer 源码，不在 `host.html`、Rust 注入脚本或
  minified bundle 中维护第二份逻辑。
- Flyfish renderer 从 clean checkout 可复现构建，源码 provenance、许可证、lockfile、
  生成物和版本标识齐全。
- 自动化、Dozer Rust 回归、真实 WKWebView 与打包态验收全部通过。
