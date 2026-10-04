# PlantUML 文件预览设计

> **状态：已批准，待实施。** 日期：2026-10-04。
>
> **产品裁决：** PlantUML 是文件预览能力，不新增 UML 面板；首版只读、本地离线
> 渲染，不要求用户安装 Java，也不把项目源码发送给公共 PlantUML Server。

## 1. 背景

Dozer 已有统一文件预览路由、Files/Project 两份预览工作区、CodeMirror 源码 host、
Flyfish/HTML/Image Annotate 等渲染 host、通用 `WebviewEnvelope`/`HostBinding`、跨项目
WebView 资源预算与文件变更刷新。PlantUML 源文件目前只会作为普通文本进入
CodeMirror，用户无法在 Dozer 内验收最终图形。

PlantUML 文件是项目产物，而不是一类需要常驻导航、聚合和独立生命周期的业务域。
因此它应与 Markdown、HTML、SVG 一样由文件预览接管：在 Files 或 Project 中打开
`.puml` 文件，默认看到图形，可切换到源码；不占用 Rail，不增加 `PanelKind`，也不
依赖 bytehost 面板迁移。

本设计扩展
[`2026-09-22-file-preview-architecture-redesign.md`](./2026-09-22-file-preview-architecture-redesign.md)
的 `Rendered` backend。若两者冲突，以本文对 PlantUML 的专项裁决为准。

## 2. 目标与非目标

### 2.1 目标

1. `.puml`、`.plantuml`、`.iuml`、`.pu`、`.wsd` 默认进入 PlantUML 图形预览，并可
   切换到 CodeMirror 源码模式。
2. 使用 PlantUML 官方 JavaScript/TeaVM 产物在本地 WebView 内生成 SVG；应用运行
   时不依赖 Java、Node、外部服务或网络。
3. 支持缩放、平移、适配窗口、100% 与重置视图；切换源码再切回时保留视口。
4. 渲染错误显示可读原因和可获得的源码行号，不白屏、不无限 loading。
5. 支持项目内受限本地 include 与随应用固定版本发布的官方 stdlib；默认禁止远程
   include。
6. 复用现有预览 tab、持久化 mode、文件监听、资源预算、统一 fallback、主题和
   WebView envelope，不建立第二套生命周期。
7. Files 与 Project 两个预览入口行为一致。

### 2.2 非目标

- 不新增 UML 独立面板、项目 UML 目录或跨图资产管理。
- 不提供图形化拖拽编辑、节点反向写回源码或完整 UML IDE。
- 不提供公共/私有 PlantUML Server 配置，也不以 Java JAR 作为首版 fallback。
- 不首发 PNG/PDF 导出、打印、版本对比、代码反向生成 UML。
- 不顺带支持 Mermaid、D2、Graphviz 等其他 diagram-as-code 语言。
- 不允许任意网络访问、`file://` 或任意文件系统读取。
- 不在本次重构通用预览架构或 bytehost。

## 3. 用户体验

### 3.1 打开与模式

用户在 Files 或 Project 中打开 PlantUML 文件后：

- 默认 mode 为 `Rendered`，内容区显示 SVG；
- 现有“预览 / 源码”切换切到 CodeMirror，语言标识为 PlantUML；
- mode 按现有 tab descriptor 持久化，未知或旧 mode 安全回退到 Rendered；
- 空文件仍由 PlantUML 专用路由认领，显示“暂无可渲染内容”，而不是普通空文本；
- 内容画像判定为二进制或有损编码时不得进入 PlantUML renderer，走现有安全
  fallback。UTF-16 首版同样不渲染，只允许既有只读源码退路。

### 3.2 查看器交互

查看器顶部使用轻量工具条，包含：

- 适配窗口；
- 100%；
- 放大、缩小；
- 重置视图。

画布支持滚轮缩放、拖拽平移。快捷键不得吞掉 Dozer 现有的全局缩放与查找路由；
查看器内 `Cmd/Ctrl+F` 没有文本搜索价值，继续由宿主按现有规则处理。

默认 SVG 背景与前景遵循当前 Dozer 主题。PlantUML 源码自身显式指定的颜色优先，
查看器不得擅自改写用户图形语义。主题变化时重新渲染或更新宿主样式，具体取决于
spike 验证结果；不得同时保留两套 renderer。

### 3.3 状态与错误

- 首次加载：显示统一 loading，而不是空白 WebView。
- 渲染成功：显示图形；可在非遮挡位置显示耗时/尺寸诊断，但首版不要求常驻状态栏。
- 语法错误：显示错误摘要、可获得的行号和“查看源码”动作。
- 引擎加载失败、超时或崩溃：进入 `BackendState::Failed`，使用统一 fallback 页面
  提供重试与源码只读退路。
- 外部 include 被拒绝：明确写“远程 include 已禁用”，不能伪装成普通语法错误。
- 文件变化：沿用当前预览 reload/revision 机制；旧渲染结果不得覆盖新文件。

## 4. 路由与 backend

### 4.1 唯一路由

`preview/router.rs::classify_preview` 仍是唯一决策点。新增集中 helper：

```rust
pub fn is_plantuml_extension(path: &Path) -> bool;
```

判定大小写不敏感，覆盖：`puml | plantuml | iuml | pu | wsd`。

路由结果：

```text
PreviewKind::Rendered
default_mode: PreviewMode::Rendered
alternate_modes: [PreviewMode::Source]
reason: RouteReason::RenderedExtension("plantuml")
```

PlantUML 判定必须位于通用 `is_editable_extension` 之前，同时保留内容安全检查：
专用扩展名不能凌驾于二进制检测。`native_editor::extension_to_syntax` 对这些扩展名
返回 `plantuml`，供源码模式高亮。

### 4.2 renderer 身份

在 `RenderedRenderer` 增加 `PlantUml`。不得只在 `preview_url(path)` 里悄悄按扩展名
分流而让 backend 仍自称 `Flyfish`；路由、预算、诊断和测试必须知道真实 renderer。

```rust
pub enum RenderedRenderer {
    Flyfish,
    IsolatedHtml,
    PlantUml,
}
```

`RenderedBackend::from_route`、`hosts_webview`、mode 切换、成本估算与一致性断言同步
覆盖该 variant。源码 mode 仍使用现有 CodeMirror host；Rendered mode 使用 PlantUML
host，同一时刻只驻留当前 mode 所需的重型 WebView。

## 5. PlantUML WebView host

新增独立命名空间与离线前端：

```text
crates/dozer-app/web/plantuml-viewer/       # TypeScript 源码与构建
crates/dozer-app/assets/plantuml-viewer/    # 提交的运行时产物
dozer://plantuml-viewer/index.html
```

不复用 Flyfish 页面：PlantUML 引擎体积、CSP、Worker/iframe 需求、错误模型和交互均与
通用媒体查看不同。新 host 仍使用现有 wry pool、几何和 `HostBinding`，不是新的窗口
或新的资源池。

构建采用仓库已有的 npm + esbuild 模式；Node 只在开发/构建前端产物时使用，不进入
Dozer 运行时。最终 JS/CSS、引擎和 stdlib 必须随应用发布，页面不得引用 CDN。

### 5.1 引擎选择

首选 PlantUML 官方 `@plantuml/core`/TeaVM 产物及其 Graphviz JS 依赖。实施前 spike
必须用锁定版本确认：

1. 实际 npm 包名、公开 API 与许可证；
2. esbuild 是否能直接打包，还是应复制官方 release 中的 `plantuml.js` 与
   `viz-global.js`；
3. CSP 下是否需要 Worker、iframe、WASM、`blob:` 或 `unsafe-eval`；
4. sequence/class/component/deployment/state/C4 的渲染一致性；
5. 本地 include 与 stdlib 的文件系统注入 API；
6. bundle 大小、首次加载、重复渲染和 RSS。

若官方 JS 产物不能在 Dozer 的安全约束内支持核心图型，计划暂停并回到设计评审，
不得静默改成公共 Server 或引入 Java 运行时。

### 5.2 命令与事件

Host ready 后由 Rust 推送源码和已解析的 include 文件，不让页面自行读取任意路径。
沿用 `WebviewEnvelope<T>`：

```rust
pub enum PlantUmlCommand {
    SetDocument {
        revision: u64,
        path: String,
        source: String,
        includes: Vec<PlantUmlInclude>,
        theme: PlantUmlTheme,
    },
    FitView,
    ActualSize,
    ResetView,
}

pub enum PlantUmlEvent {
    Ready,
    Rendered {
        revision: u64,
        width: u32,
        height: u32,
        duration_ms: u64,
    },
    Failed {
        revision: u64,
        kind: PlantUmlFailureKind,
        message: String,
        line: Option<u32>,
    },
    OpenSource {
        line: Option<u32>,
    },
}
```

`includes` 使用项目相对规范化路径与 UTF-8 内容。若 spike 证明官方引擎能安全地通过
同步虚拟文件系统按需索引，可把载荷改成显式 allowlist manifest + 具名读取命令；
不能给 JS 任意路径 fetch 权限。协议调整必须在 plan 的 spike 决议中记录。

每次文档或 include 改变都递增 revision。JS 和 Rust 两侧均丢弃旧 revision 的
`Rendered`/`Failed`，防止快速切 tab、连续保存或后台渲染乱序。

## 6. include 与安全边界

### 6.1 允许

- 当前 PlantUML 文件；
- 项目根目录内、经规范化后仍在项目根内的相对本地 include；
- 随 Dozer 固定版本发布的 PlantUML 官方 stdlib，例如 `<C4/C4_Context>`；
- `!include_once`；
- 若实现成本可控，支持本地 `!includesub`；否则首版明确报“不支持”，不得错误展开。

### 6.2 拒绝

- `!includeurl`、HTTP/HTTPS 与其他远程读取；
- `file://`；
- 绝对路径；
- 规范化后越出项目根的 `..`；
- symlink 解析后越出项目根；
- 非 UTF-8、本地设备文件及非普通文件；
- 动态构造后无法静态确定目标的外部 include。

### 6.3 资源上限

初始护栏由常量集中定义，并用测试钉死：

| 维度 | 初值 | 超出行为 |
|---|---:|---|
| 根源码大小 | 4 MiB | 不渲染，保留源码只读退路 |
| include 深度 | 16 | 失败并报告 include 链 |
| include 文件数 | 128 | 失败 |
| include 总字节 | 16 MiB | 失败 |
| 单次渲染时间 | 10 s | 失败，可重试 |
| SVG 输出字节 | 32 MiB | 拒绝挂载 |

这些是安全上限，不是资源预算替代品；性能基准后可调整。解析器必须检测 include 环。
不要尝试在 Rust 中完整实现 PlantUML 预处理器；Rust 只负责授权、解析明确的 include
引用并建立受限虚拟文件集合，最终预处理语义仍由官方引擎执行。

### 6.4 SVG

引擎输出按不可信内容处理。挂载前至少拒绝/清理：`script`、事件属性、`foreignObject`、
外部 URL、`javascript:`、非本地资源引用。SVG 内链接若要启用，只允许经过 Rust
再次校验的项目相对路径；首版可以全部禁用链接，不影响验收。

CSP 默认 `default-src 'none'`，按 spike 的最小需求逐项开放本地 `script-src`、
`style-src`、`worker-src`、`img-src`；不得为了省事使用宽泛 `connect-src *`。

## 7. 状态、资源与文件变化

- PlantUML host 计入 `max_heavy_webviews` 和 preview 总预算；成本模型包含固定引擎
  成本、源码/include 字节和 SVG 上限。
- viewer 走现有 `(project_id, panel, tab_id)` 资源键，Files/Project 同名 tab 不冲突。
- inactive/suspended tab 不渲染；重新激活后按现有 reserve → load → ready 流程恢复。
- 视口属于纯前端 view state；正常 mode 切换可在 tab runtime 保存有界的
  `scale/x/y`，资源淘汰时允许回到 Fit，不持久化 SVG。
- 根文件变化触发现有 reload。include 文件变化也必须使所有依赖它的已打开
  PlantUML tab 失效并重渲染，因此 Rust 维护 `include_path -> viewer key` 反向索引。
- 项目关闭、tab 关闭或 revision 更新后到达的事件静默丢弃，不 panic。

## 8. Agent 与编辑边界

PlantUML Rendered mode 只读。用户修改路径是：切到源码 mode 使用现有 CodeMirror，
或让 Agent 修改文件。首版不新增专用“Ask Agent”协议；现有 PreviewContext 已能让
Agent 获知当前文件，渲染错误的行号通过“查看源码”定位。

`Cmd/Ctrl+S`、undo/redo、replace 只在 Source mode 走现有 CodeMirror 能力。
Rendered host 不得实现保存，也不得成为文档内容权威。

## 9. 测试与验收

### 9.1 自动化

- router 表：五种扩展名、大小写、空文件、二进制伪装、持久化 mode；
- backend：`RenderedRenderer::PlantUml`、Source/Rendered 切换和一致性；
- protocol：完整/缺字段/畸形/超大消息、binding 不匹配、旧 revision；
- include：正常、嵌套、once、环、穿越、绝对路径、symlink 越界、远程、深度/数量/
  总字节上限；
- assets：host/bundle/stdlib 存在、CSP 无网络引用、未知路径 404；
- lifecycle：Files/Project、tab 关闭、项目关闭、reload、suspend/resume、迟到事件；
- 前端 fixtures：sequence/class/component/deployment/state、C4、语法错误、超大 SVG；
- sanitizer：恶意 SVG fixture 不留下脚本、事件或外链。

### 9.2 人工验收

1. Files 与 Project 分别打开同一 `.puml`，均默认显示图形。
2. 图形/源码往返，源码可编辑保存，保存后图形刷新且没有旧 revision 闪回。
3. 缩放、平移、适配窗口、100%、重置在深浅主题下正常。
4. 打开五种核心图型与 C4 fixture，断网环境下全部渲染。
5. 语法错误显示原因；点击查看源码定位到错误行（引擎提供行号时）。
6. 项目内多层 include 正常；修改被 include 文件后打开的图自动更新。
7. `!includeurl`、目录穿越和 symlink 越界被明确拒绝，网络抓包无请求。
8. 连续快速切换三个大图，最终只显示当前文件，UI 不冻结。
9. 达到 WebView 预算后能 suspend/resume，不反复销毁重建。
10. 无 Java、无 Node、断网的发布包可以完成上述操作。

## 10. 发布与回退

按“spike → 路由/backend → host/protocol → include → 生命周期/验收”切片提交。上线前
保留源码 mode 作为稳定退路。若 PlantUML host 失败，单个 tab 进入统一 Failed 页面；
不得把所有 `.puml` 静默重新分类为 Flyfish 或向公网发送内容。

## 11. 延后项

以下需求成立时再单独立项：项目 UML 总览面板、跨图导航、架构文档覆盖率、导出、
图表 diff、公共/私有 Server、自定义 PlantUML JAR、其他 diagram-as-code 语言。

## 12. 实现期决议（Task 0 spike）

> 日期：2026-10-04。spike 见 `spike/plantuml-js/`（README 有完整数据）。
> **退出条件全部满足，门禁通过**：五类核心图 + C4 离线渲染成功；无 Java；能在不给
> JS 任意文件读取权限的前提下提供项目内 include。

### 12.1 引擎与版本

- 采用官方 `@plantuml/core@1.2026.8`（PlantUML 作者 Arnaud Roques，MIT，
  `repository git+https://github.com/plantuml/plantuml.git`）。不用同名第三方包。
- 完整性：`sha512-md2wGuaIJnAq1yKkedSpQE68cLsvZvctwzWHkDAnC3DTVLe/MSd/2vr4ETwr6jtc1av8cdxCohFuW4VGejMHyw==`，
  `dist.shasum 1a9dbacf1411927c17f4ce5dab57ed986da109f5`。`package-lock.json` 必须提交。
- **不打包进 esbuild bundle，改为复制官方 release 产物**：`plantuml.js`(3.9MB) 与
  `viz-global.js`(1.4MB) 作为静态资源随应用发布。原因是引擎是 TeaVM 大体积产物、
  ESM 顶层依赖浏览器全局，直接打包无收益且增加 sourcemap/transform 风险。
  前端 `web/plantuml-viewer` 只写薄胶水（加载、消息、缩放、错误映射）。

### 12.2 运行路径（无 Worker/WASM/unsafe-eval）

- 加载顺序固定：先 classic `<script src="viz-global.js">`（Graphviz/Viz.js，挂
  `window`/`globalThis` 全局），再 `import { renderToString } from 'plantuml.js'`。
- 主线程运行，**不需要 Worker、iframe、WASM、`blob:` 或 `unsafe-eval`**。
- 文本度量走 `HTMLCanvasElement.getContext('2d')`（`measureText`，C4/viz 路径还会
  `createImageData`/`getImageData`/`toDataURL`），WKWebView 原生支持，无需额外 CSP 放行。
- CSP 仍从 `default-src 'none'` 起步，逐项只放开本地 `script-src`（含 `'wasm-unsafe-eval'`
  仅在实测需要时）、`style-src`、`img-src data:`；**不得**开 `connect-src`。

### 12.3 include / stdlib 注入（采纳“引擎同步虚拟文件系统”，非 manifest 命令）

spike 证实引擎不访问文件系统，全部经注入全局：

- `PLANTUML_STDLIB[libBase][relPath] = string[]`（行数组）——引擎 `CBf` 读取。
- `PLANTUML_STDLIB_JSON[libBase][relPath] = string`——`Er5` 备选读取。
- `PLANTUML_STDLIB_INFO[libBase] = {name,version,license,...}`——`DWS` 读取，
  **决定走“已注册库”分支还是网络脚本加载分支**。
- `PLANTUML_STDLIB_LOADER(name, ok, fail)`：覆盖默认加载器（默认会
  `script.src = PLANTUML_STDLIB_BASE + name` 网络注入）。**`name` 是脚本文件名**
  （如 `"c4.min.js"`），命名空间 key 是去 `.min.js`/`.js` 后的 base（如 `"c4"`），
  loader 必须做后缀剥离；命中即 `ok()` 并 `return true`，否则 `return false`。
- `!theme` 走 `PLANTUML_THEMES`/页面目录 fetch（另一条路径），离线需单独预置。
- **远端 `!includeurl`/URL include 走 `XMLHttpRequest`——唯一网络出口**。生产必须
  阻断并在 Rust 侧前置识别外链 include，报“远程 include 已禁用”。

因此 §5.2 的 `SetDocument.includes: Vec<PlantUmlInclude>` 保持不变：Rust 授权并读取
项目内 include 后，把文件内容以 `PLANTUML_STDLIB` 虚拟文件形式推给页面；不需要、
也不得给 JS 任意路径 fetch 权限。C4 等官方 stdlib 以固定版本 vendored 静态资源随应用
发布（`c4.min.js` 180KB/gzip 24KB），页面引导时预置命名空间。

### 12.4 确定性与错误

- 同一输入两次渲染**逐字节相同**；唯一易变字段是 SVG 上的 `plantuml-src` 数据属性，
  测试正规化时去掉该属性即可。前端无需额外去抖。
- 引擎异步、共享全局，**同一 JS context 必须串行化渲染**（生产端排队，一次一张）。
- 语法错误经 `renderToString` 的 `onError` 回调返回字符串；行号提取按引擎消息解析，
  解析不到则 `line = None`（UI 仍显示错误摘要 + 查看源码）。

### 12.5 性能量级（jsdom/Node，非 WKWebView 绝对值）

冷启动首图 ~100ms；预热后连续 20 次 sequence 渲染 130ms（avg 6.5ms）。实现期需在真实
WKWebView 复测冷启动与内存，作为 §7 成本模型的输入。§6.3 资源上限初值维持不变。

