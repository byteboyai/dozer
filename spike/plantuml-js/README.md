# Task 0 spike — offline PlantUML JS engine

一次性技术验证，随时可删（不留进产品代码）。目的：在实现 Dozer PlantUML
文件预览前，确认官方 `@plantuml/core`（PlantUML 作者 Arnaud Roques 发布的
TeaVM→JS 编译版）能**完全离线**在浏览器 DOM 里把 `.puml` 渲染成 SVG，并能
通过注入把标准库（如 C4）和本地 `!include` 喂给引擎。

对应计划：`docs/superpowers/plans/2026-10-04-plantuml-file-preview.md` 的 Task 0
（硬门禁）。结论：**门禁通过**，可以进入 Task 1。

## 怎么跑

```bash
cd spike/plantuml-js
npm install          # 只装 @plantuml/core 与 jsdom
npm test             # = node run.mjs && node run-c4.mjs
```

- `run.mjs`：五种核心图（sequence/class/component/deployment/state）离线渲染 +
  确定性 + 本地 `!include` 走 stdlib 虚拟文件系统。
- `run-c4.mjs`：额外加载 vendored C4 标准库包，渲染真实 C4 图；全程**断网**
  （XHR/fetch 被替换成抛错），断言零网络请求。

## 结论（全部验证通过）

| 项 | 结果 |
|----|------|
| sequence / class / component / deployment / state | 全部渲染出 `<svg>`，19–105ms |
| C4 标准库图（`!include <C4/C4_Context>`） | 渲染成功，371ms，5894 bytes |
| 确定性 | 同一输入两次渲染 **逐字节相同**（去掉易变的 `plantuml-src` 属性后也相同） |
| 网络 | **零** XHR/fetch 尝试（含 C4 场景） |
| 本地 `!include` 文件注入 | 通过 `PLANTUML_STDLIB` 命名空间生效 |
| 标准库注入 | 通过 `PLANTUML_STDLIB_LOADER` 覆盖默认网络加载器生效 |

## 关键 API 与契约（实现期照此做）

### 引擎入口

`@plantuml/core@1.2026.8`（`type: module`，`main: plantuml.js`）：

```js
import { render, renderToString } from './plantuml.js';
// render(lines: string[], targetId: string, opts?: {dark?: boolean}): void
//   直接渲染进 DOM 里 id=targetId 的元素
// renderToString(lines: string[], onSuccess(svg), onError(err)): void
//   拿到 SVG 字符串（Dozer 用这个，把结果经 envelope 回传 Rust）
```

- `lines` 是**已按 `\r\n|\r|\n` 拆分的字符串数组**（见上游 `index-basic.html`）。
- 渲染是**异步**的（TeaVM 协程 + `render` 内部回调）。
- **加载顺序**：`viz-global.js`（Graphviz/Viz.js，classic script，**必须先加载**）
  → 再 `import plantuml.js`。`package.json` 的 `sideEffects` 也标了
  `viz-global.js`。`viz-global.js` 会挂 `window`/`globalThis` 上的 Graphviz 全局。
- 画布文本度量走 `HTMLCanvasElement.getContext('2d').measureText()`，并可能
  `createImageData`/`getImageData`/`toDataURL`（viz 路径）。真实 WKWebView 原生支持，
  无需 shim；jsdom 里必须 mock，见 `harness.mjs`。
- `globalThis` vs `window`：引擎读全局时用 `globalThis ?? self ?? window`
  （见下方 loader）。**浏览器里 `globalThis === window`，无需特殊处理**；
  仅 Node/jsdom 测试环境需要把命名空间同时挂到两处。

### 标准库 / 本地文件虚拟文件系统

引擎不访问本地文件系统；`!include` 的解析全部经由注入的全局命名空间：

- `PLANTUML_STDLIB[libBase][path] = string[]`（**文件内容行数组**，CBf 读取）。
  例：`PLANTUML_STDLIB.c4['c4_context'] = [...]`。
- `PLANTUML_STDLIB_JSON[libBase][path] = string`（JSON 形式，Er5 读取）。
- `PLANTUML_STDLIB_INFO[libBase] = { name, display_name, description, version, license, ... }`
  （DWS 读取，用于 `!include <lib/...>` 暴露元数据）。**info 存在与否决定走
  “已注册库”分支还是“脚本加载”分支**。
- `PLANTUML_STDLIB_LOADER(name, ok, fail)`：覆盖默认加载器。默认实现会
  `document.createElement('script').src = PLANTUML_STDLIB_BASE + name` 网络注入。
  覆盖后**同步**从内存满足加载：找到命名空间就 `ok()` 并 `return true`（非 false 即接管），
  找不到就 `return false` 回落默认（我们会阻断）路径。
  - **`name` 是脚本文件名**，如 `"c4.min.js"`（`PLANTUML_STDLIB_BASE` 默认是
    `https://plantuml.github.io/plantuml/js-plantuml/`）；命名空间的 key 是去掉
    `.min.js`/`.js` 的 base，如 `"c4"`。生产 loader 必须做这个后缀剥离。
  - 引擎读该全局同样是 `globalThis ?? self ?? window`。
- `PLANTUML_THEMES[themeId] = ...`：`!theme` 定义，引擎按需从页面目录 fetch
  （BR$/CC3）。离线要预置或改走 loader。
- **远端 `!includeurl` / 带 URL 的 include** 走 `XMLHttpRequest`（plantuml.js:4761
  附近）。这是唯一的网络出口——生产环境必须阻断（识别到外链 include 直接报错，
  **不得**把源码/include 发给公共服务器）。

### 官方标准库包（C4 为例）

`javascript:fetch` 到的 `c4.min.js`（180KB）头部即：
```js
window.PLANTUML_STDLIB = window.PLANTUML_STDLIB || {};
window.PLANTUML_STDLIB.c4 = window.PLANTUML_STDLIB.c4 || {};
window.PLANTUML_STDLIB.c4.c4 = ["","!global NEW_C4_STYLE ?= 0", ...];
window.PLANTUML_STDLIB_INFO.c4 = { name:"C4", version:"2.13.0", license:"MIT", ... };
```
同时写 `PLANTUML_STDLIB_JSON.c4`。包内 37 个文件（`c4`、`c4_component`、
`c4_context`、`c4_sequence`、`_examples_/...` 等）。C4 图需要 Graphviz（viz-global），
已随引擎加载。

## 资源体积（实现期资源上限的输入）

| 文件 | 原始 | gzip |
|------|------|------|
| `plantuml.js`（引擎） | 3.9 MB | 1.05 MB |
| `viz-global.js`（Graphviz/Viz.js） | 1.4 MB | 590 KB |
| `themes.js`（内置主题） | 320 KB | 27 KB |
| `emoji.js` | 1.8 MB | — |
| `openiconic.js` | 52 KB | — |
| `c4.min.js`（可选，vendored） | 180 KB | 24 KB |

单个 WebView 会话加载 ~5.3MB 引擎（gzip ~1.6MB）+ 可选 stdlib，属一次性成本。

### 性能（jsdom + Node，非 WKWebView 绝对值，仅供量级参考）

- 冷启动首图（sequence）：~100ms。
- 预热后连续 20 次 sequence 渲染：**总计 130ms，min 5ms / max 8ms / avg 6.5ms**。
- 进程 RSS（含 Node + jsdom 开销）：~387 MiB；`heapUsed` ~106 MiB。
  真实 WKWebView 的 JSC 与内存曲线不同，实现期需在 WebKit 侧复测。
- 引擎异步且共享全局状态，**同一 JS context 内多次渲染必须串行化**（生产端排队）。

## 版本 / 许可 / 完整性（供实现期锁版本）

- 包：`@plantuml/core@1.2026.8`
- 作者：Arnaud Roques（PlantUML 作者），**MIT**，`repository git+https://github.com/plantuml/plantuml.git`
- npm dist-tags: latest 指向 `1.2026.8`
- integrity `sha512-md2wGuaIJnAq1yKkedSpQE68cLsvZvctwzWHkDAnC3DTVLe/MSd/2vr4ETwr6jtc1av8cdxCohFuW4VGejMHyw==`
- dist.shasum `1a9dbacf1411927c17f4ce5dab57ed986da109f5`
- 无 Java、无服务端、无运行时网络（除主动 `!includeurl`）。官方 README 明确
  “entirely in the browser, with no server and no Java”。

## 未在本次 spike 覆盖（实现期自行处理）

- 多张图在同一 JS context 连续渲染的并发/串行（引擎异步且共享全局，需串行化；
  计划里已列为约束）。
- `!theme` / emoji / openiconic 的离线预置策略（本次只证明 stdlib 机制，
  themes 走的是另一条 BR$/fetch 路径）。
- 非 C4 的其他内置 stdlib（AWS/azure 等）——同一 loader 机制，按需 vendored。
- 真实 WKWebView 下的性能与 `toDataURL`（jsdom 无 canvas，属测试环境限制）。
- 超大 `.puml` 的资源上限（计划里由 Rust 侧读取/行数上限控制）。

## 文件

- `harness.mjs` — jsdom 环境 + 全局镜像 + canvas/SVG mock + 命名空间/loader 注册。
- `run.mjs` — 五核心图 + 确定性 + 本地 include。
- `run-c4.mjs` — 断网 + C4 stdlib 全链路。
- `fixtures/*.puml` — 六种图的样例（含 `with_include.puml`/`included.puml`）。
- `stdlib/c4.min.js` — vendored C4 标准库包（spike 期抓取，随 spike 提交）。
- `node_modules/` — 不提交（见仓库 `.gitignore`）。
