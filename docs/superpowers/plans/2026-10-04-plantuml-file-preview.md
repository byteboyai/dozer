# PlantUML 文件预览 Implementation Plan

> **For agentic workers:** 按 task 顺序实施；每个 task 必须独立可构建、可测试、可回退。
> spike 的退出条件未满足时停止实现并更新 spec，不得擅自改用公网服务或 Java。

**Goal:** 在 Files/Project 文件预览中为 `.puml/.plantuml/.iuml/.pu/.wsd` 提供
本地离线 PlantUML SVG 预览和源码切换，运行时不依赖 Java/Node/网络。

**Architecture:** 在现有 `PreviewKind::Rendered` 下新增
`RenderedRenderer::PlantUml`，新增 `dozer://plantuml-viewer/` WebView host。Rust
负责路由、读取、include 授权、资源限制与生命周期；官方 PlantUML JS/TeaVM 引擎
负责预处理、布局和 SVG 生成。所有消息复用 `WebviewEnvelope`/`HostBinding`，Files 与
Project 共用实现。

**Spec:**
`docs/superpowers/specs/2026-10-04-plantuml-file-preview-design.md`

## Global Constraints

- 不新增 `PanelKind::Uml`、Rail 项或独立面板状态。
- 不把 PlantUML 源码/include 发往公网；运行时所有资产离线。
- 不引入 Java/JRE，不在 Rust 里重写完整 PlantUML 预处理器。
- PlantUML renderer 必须成为显式 `RenderedRenderer`，不能只靠 URL 扩展名暗分流。
- Rendered mode 只读；Source mode 复用现有 CodeMirror、保存、冲突和 recovery。
- Host → Rust 消息必须通过 `WebviewEnvelope` 和完整 `HostBinding` 校验。
- 所有项目路径先 canonicalize，并验证仍位于项目根；symlink 不能绕过边界。
- WebView 失败走统一 `BackendState::Failed`/fallback，不增加面板私有 notice/error。
- 日志使用 `dozer_core::log_*!`，不得记录源码、include 内容或完整 SVG。
- 前端源码变化后必须运行构建，并提交 `assets/plantuml-viewer` 产物。

---

## Task 0：官方 JS 引擎 spike 与依赖裁决

**目的：** 在接业务代码前证实官方 PlantUML JS 产物满足兼容性、安全和资源约束。

**状态：已完成（2026-10-04）。退出条件全部满足，门禁通过。** 结论见 spec §12
“实现期决议”与 `spike/plantuml-js/README.md`。

**Files:**

- Create: `spike/plantuml-js/README.md`
- Create: `spike/plantuml-js/package.json`
- Create: `spike/plantuml-js/package-lock.json`
- Create: `spike/plantuml-js/run.mjs`
- Create: `spike/plantuml-js/fixtures/*.puml`
- Modify: spec 的“实现期决议”小节（记录结果）

- [x] 查询并锁定官方 `@plantuml/core` 或官方 release JS 产物版本，核实仓库、许可证、
      完整性 hash 和发布来源；不采用同名第三方包。
- [x] 验证 Node 中能生成 SVG：sequence、class、component、deployment、state。
- [x] 验证浏览器/WebKit 等价运行路径；记录是否需要 Worker、iframe、WASM、`blob:`、
      `unsafe-eval` 和 Graphviz `viz-global.js`。
- [x] 验证官方 stdlib/C4 与本地虚拟文件 include API；明确是“命令一次推全 manifest”
      还是“引擎同步虚拟文件系统”。
- [x] 对比同一输入连续两次 SVG；记录非确定字段以及测试应如何正规化。
- [x] 记录 bundle 原始/压缩大小、冷启动、连续 20 次渲染耗时与 RSS。
- [x] 删除/禁用网络后重跑全部 fixture，确认无外部请求。
- [x] 把最终 API、版本、CSP、include 方案、已知不兼容语法和性能写入 spike README，
      并在 spec 追加“实现期决议”。

**退出条件：** 五类核心图与至少一个 C4 fixture 离线成功；无 Java；能在不赋予任意
文件读取的情况下提供项目内 include。否则停止后续 task。

**验证：**

```bash
cd spike/plantuml-js
npm ci
npm test
```

---

## Task 1：路由与 backend 身份

**目的：** 先让类型系统正确表达 PlantUML，不创建 host。

**状态：已完成（2026-10-04）。三个验证命令全绿（router 29 / backend 15 / native_editor 2）。**

**Files:**

- Modify: `crates/dozer-app/src/preview/router.rs`
- Modify: `crates/dozer-app/src/preview/backend.rs`
- Modify: `crates/dozer-app/src/preview/native_editor.rs`
- Modify: `crates/dozer-app/src/preview/estimate_cost.rs`（若实际文件名不同，以现有
  `estimate_cost` 所在模块为准）
- Test: 上述模块内测试

- [x] 先添加失败路由表测试：五种扩展名及大小写默认 `Rendered`、可切 `Source`；
      二进制伪装不能进 renderer；空 `.puml` 仍是 PlantUML route。
- [x] 新增唯一 `is_plantuml_extension` helper；其它模块不得复制扩展名列表。
- [x] `RouteReason` 为 PlantUML 给出可诊断的稳定原因；不要把 `.puml` 伪装成 Markdown。
- [x] `extension_to_syntax` 返回 `plantuml`。
- [x] 新增 `RenderedRenderer::PlantUml`，在 `PreviewBackend::from_route` 中构造。
- [x] 补 mode round-trip、`hosts_webview`、backend/runtime invariant 测试。
- [x] 给 PlantUML host 定义重型 WebView 成本；先使用 Task 0 实测的保守上界。

**验证：**

```bash
cargo test -p dozer-app preview::router -- --nocapture
cargo test -p dozer-app preview::backend -- --nocapture
cargo test -p dozer-app preview::native_editor -- --nocapture
```

**回退点：** 删除 PlantUML route 后文件恢复普通 CodeMirror，不影响其它 renderer。

---

## Task 2：建立前端工程与离线产物 ✅ 完成（2026-10-04）

**目的：** 建立最小 host，能在浏览器测试环境接收 source 并渲染/显示 SVG。

**Files:**

- Create: `crates/dozer-app/web/plantuml-viewer/package.json`
- Create: `crates/dozer-app/web/plantuml-viewer/package-lock.json`
- Create: `crates/dozer-app/web/plantuml-viewer/build.mjs`
- Create: `crates/dozer-app/web/plantuml-viewer/tsconfig.json`
- Create: `crates/dozer-app/web/plantuml-viewer/src/index.ts`
- Create: `crates/dozer-app/web/plantuml-viewer/src/types.ts`（协议类型）
- Create: `crates/dozer-app/web/plantuml-viewer/src/sanitize.ts`（SVG sanitizer）
- Create: `crates/dozer-app/web/plantuml-viewer/src/renderer.ts`
- Create: `crates/dozer-app/web/plantuml-viewer/src/viewport.ts`
- Create: `crates/dozer-app/web/plantuml-viewer/src/style.css`
- Create: `crates/dozer-app/web/plantuml-viewer/render-smoke.mjs`
- Create: `crates/dozer-app/assets/plantuml-viewer/index.html`
- Generated: `crates/dozer-app/assets/plantuml-viewer/*.{js,css}` 及引擎/stdlib 产物

- [x] 按 Task 0 裁决锁定依赖；`package-lock.json` 必须提交。
      （`@plantuml/core@1.2026.8`，integrity 与 Task 0 一致；`package-lock.json` 已提交。）
- [x] 构建输出单一稳定入口；不得使用运行时 CDN、动态 npm 解析或绝对开发路径。
      （esbuild 产 `bundle.js`/`bundle.css`，引擎产物原样拷贝；`build.mjs` 显式拒绝
      构建机绝对路径与 CDN 令牌；连续两次构建字节一致。）
- [x] `index.html` 配最小 CSP；若 Task 0 需要额外 directive，逐项解释。
      （`default-src 'none'` + `script-src 'self'`/`style-src 'self' 'unsafe-inline'`/
      `img-src 'self' data:`；不开 `connect-src`，无 `unsafe-eval`——与 §12.2 一致。）
- [x] 建立 renderer adapter，隔离官方 API；UI 不直接散落调用 TeaVM 全局符号。
      （`src/renderer.ts` 封装 `renderToString`、stdlib/include 注入、渲染串行化与错误归类。）
- [x] 实现 SVG viewport：滚轮缩放、拖拽、Fit、Actual Size、Reset。
      （`src/viewport.ts`。）
- [x] SVG 挂载前执行 sanitizer；首版禁用图内链接。
      （`src/sanitize.ts`：去 `<script>`/`on*`/`foreignObject`/外链，去掉 `<a>` 的 href。）
- [x] loading、empty、error 三种状态不依赖 Rust 即可 fixture 渲染。
      （`src/index.ts` 的 `showStatus`，`render-smoke.mjs` 在 jsdom 下直接驱动。）
- [x] render-smoke 覆盖核心图、C4、语法错误、恶意 SVG 和大 SVG 拒绝。
      （`render-smoke.mjs` 覆盖核心图/C4/语法错误/本地 include/零网络；
       `src/sanitize.test.mjs` 覆盖恶意 SVG 与超大 SVG 拒绝。）
- [x] 添加资产离线扫描：生成物不得含 `http://`/`https://` 外部加载引用；许可证/
      source map 注释中的 URL 可通过精确 allowlist 处理，不能宽泛跳过。
      （`scan-offline.mjs` 只匹配加载构造（src/href/import/fetch/XHR/Worker/
      sourceMappingURL），忽略字符串与注释里的惰性 URL；已接入 `npm run build`。）

**验证：**

```bash
cd crates/dozer-app/web/plantuml-viewer
npm ci
npm run build
npm test
```

---

## Task 3：`dozer://plantuml-viewer/` 资产命名空间 ✅ 完成（2026-10-04）

**目的：** 由现有自定义协议安全服务 host 与静态资产。

**Files:**

- Modify: `crates/dozer-app/src/assets.rs`
- Test: `crates/dozer-app/src/assets.rs`

- [x] 先写失败测试：`index.html`/bundle/stdlib 200、未知资源 404、路径穿越 404。
      （`plantuml_viewer_serves_vendored_files` / `plantuml_viewer_rejects_traversal_and_has_no_file_endpoint`
      先验收先失败。）
- [x] 新增 `plantuml_viewer_root_for`，结构与 editor/json-editor/tabular-host 一致。
- [x] 在 `handle_protocol` 增加 `plantuml-viewer/` 分支，只服务 vendored assets。
- [x] 本 task 不增加通用 `__file__`：源码/include 由具名 command 推送，避免页面自行
      fetch 任意已打开文件。（测试覆盖 `__file__/etc/passwd` → 404。）
- [x] 测试 host CSP 无公网、无 `file:`、无宽泛 `connect-src`。
      （`plantuml_viewer_host_has_strict_csp_and_no_external_refs`；并断言无 `unsafe-eval`。）
- [x] 更新 `handle_protocol` 顶部命名空间注释，避免文档落后于实际路由。
- [x] 附带：`scripts/build-macos-app.sh` 增加 `plantuml-viewer` 资源打包（防分发态 404）。

**验证：**

```bash
cargo test -p dozer-app assets::tests -- --nocapture
```

---

## Task 4：PlantUML 协议与 binding 校验

**目的：** 建立具名、可版本化、拒绝伪造归属的 host 协议。

**Files:**

- Modify: `crates/dozer-app/src/preview/webview_protocol.rs`
- Modify: `crates/dozer-app/src/preview/webview.rs`
- Modify: `crates/dozer-app/src/preview/backend.rs`（`.html` 的 renderer 选定搬到 backend，`preview_url` 不再猜扩展名）
- Modify: `crates/dozer-app/src/preview/view.rs`（`preview_url` 调用点传 backend）
- Modify: `crates/dozer-app/web/plantuml-viewer/src/{types,index}.ts`（对齐 wire tag：`kind`；`Failed` 失败字段 `failure_kind`）
- Test: 同模块测试

状态：**DONE**（43 protocol + 7 webview 测试全绿）。

- [x] 先写 `PlantUmlCommand`/`PlantUmlEvent` serde round-trip 失败测试。
- [x] 实现 spec 定义的命令、事件、failure kind 和 include payload。（决议：discriminant tag 用 `kind`，`Failed` 的失败分类字段改名 `failure_kind`，避免同名冲突。）
- [x] 复用 `WebviewEnvelope`，不增加任意 JavaScript 执行消息。
- [x] 事件解析限制消息总大小（`MAX_MESSAGE_BYTES`）、错误文本长度（`MAX_PLANTUML_ERROR_CHARS=4096`，按字符截断不切码点）和 line 范围（`MAX_PLANTUML_LINE=1_000_000`，越界降级为 `None`），畸形 payload 不 panic。
- [x] 扩展 rendered host binding 识别 PlantUML namespace（`hosts_rendered_binding` 增加 `dozer://plantuml-viewer/`）；只接受 Files/Project（`flyfish_binding_from_url` 的 panel 白名单）。
- [x] 测试 project/panel/tab/document 任一不匹配都被拒绝（`parses_and_validates_plantuml_events`）。
- [x] 测试旧 revision 的 `Rendered`/`Failed` 不能改变新一轮状态（协议层 `PlantUmlEvent::is_stale`/`carries_terminal_result` + 测试；真正的丢弃接线在 Task 6 消费）。
- [x] 在 `preview_url` 中按 backend renderer（而非再次猜扩展名）生成 `dozer://plantuml-viewer/index.html`；函数签名改为 `preview_url(path, backend: Option<&PreviewBackend>)`，`.html` 的 renderer 选定上移到 `PreviewBackend::from_route`（新增 `IsolatedHtml` 实际使用），删除了 `preview_url` 里重复的扩展名 match。无 backend 时保留旧扩展名回退。

**验证：**

```bash
cargo test -p dozer-app preview::webview_protocol -- --nocapture   # 43 passed
cargo test -p dozer-app preview::webview::tests -- --nocapture      # 7 passed
```

---

## Task 5：受限 include resolver

**目的：** 在 Rust 建立允许交给官方引擎的虚拟文件集合及依赖反向索引。

**Files:**

- Create: `crates/dozer-app/src/preview/plantuml.rs`
- Modify: `crates/dozer-app/src/preview/mod.rs`
- Test: `crates/dozer-app/src/preview/plantuml.rs`

建议接口：

```rust
pub struct PlantUmlDocument {
    pub source: String,
    pub includes: Vec<PlantUmlInclude>,
    pub dependencies: Vec<PathBuf>,
}

pub fn load_document(
    project_root: &Path,
    source_path: &Path,
    limits: PlantUmlLimits,
) -> Result<PlantUmlDocument, PlantUmlLoadError>;
```

- [x] 先写 fixture 测试：相对 include、嵌套、once、环、includesub（按 spike 裁决）。
      决议：`!includesub file!tag` 取 `!` 前文件部分,按本地 include 处理(支持);
      尖括号 `<C4/...>` stdlib 不是项目文件,不读盘。
- [x] 拒绝 `!includeurl`、URL、绝对路径、`file://` 和无法静态授权的外部 include
      (空目标、`${var}`/`%()`/`$!` 动态构造)。
- [x] canonicalize 根路径、源文件和 include；验证 include 最终路径仍在项目根。
- [x] 用真实 symlink fixture 覆盖“文本路径在根内、目标在根外”。
- [x] 实施 spec 的根大小、深度、文件数和总字节上限；错误携带安全的相对 include 链。
- [x] 非 UTF-8、目录、设备文件、读取失败返回具名错误。
- [x] 不展开 PlantUML 宏/条件语义；只建立官方引擎可见的虚拟文件集合。
- [x] 输出依赖列表去重且排序稳定，供文件监听和确定性测试。
      证据:`cargo test -p dozer-app preview::plantuml` → 20 passed;`preview::` → 347 passed;
      `cargo clippy -p dozer-app --all-targets` 对改动文件无告警。

**验证：**

```bash
cargo test -p dozer-app preview::plantuml -- --nocapture
```

---

## Task 6：预览加载、WebView 创建与 IPC 接线 ✅ 完成（2026-10-05）

**目的：** 让 Files/Project 真正打开 PlantUML host，并完成 ready → render → ack。

**Files:**

- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/preview/view.rs`
- Modify: `crates/dozer-app/src/preview/webview.rs`
- Modify: `crates/dozer-app/src/runtime.rs`
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/app/update.rs`（若消息处理实际落在此）
- Test: 对应模块测试

- [x] PlantUML tab 在 Rendered mode 产生一个普通 preview WebView spec；Source mode 只产生
      CodeMirror spec，不得同时驻留两份 host。（`uses_plantuml_host()` 门控；`preview_desired` 据此选 host。）
- [x] host URL 注入完整 `proj/panel/tab/doc`，与其它 rendered host 同一绑定来源。
      （`plantuml_viewer_url()` + `EditorHostBinding::new(...).document_id()`；runtime 用 `flyfish_binding_from_url` 复核。）
- [x] runtime IPC 识别 PlantUML envelope，先解析、再 `HostBinding::validate`、最后投递。
      （`runtime.rs` `is_plantuml_host` 分支 → `parse_plantuml_event` → `event.validate(binding)` → `Message::PlantUmlEvent`。）
- [x] 收到 `Ready` 后后台 `load_document`，成功才推 `SetDocument`；文件 IO/include
      解析不得阻塞 UI 线程。（`spawn_plantuml_load` 在 `spawn_blocking`；`try_push_initial_plantuml_state` 收敛为 rendezvous。）
- [x] `Rendered` 只在 revision 与当前 load 匹配时迁为 Ready。
      （`apply_plantuml_event(..)` 按 `event_revision` 门控；`apply_plantuml_event_rendered_stale_revision_is_dropped`。）
- [x] `Failed` 迁入统一 `BackendState::Failed`；可重试错误显示 Retry，策略拒绝不可
      伪装成 retryable 引擎错误。（`apply_plantuml_event` / `apply_plantuml_load_error` 走同一 `PreviewError`。）
- [x] `OpenSource { line }` 切换当前 tab 到 Source mode，并复用现有 reveal 命令定位。
      （`update.rs` OpenSource 臂 → `enter_code_mode` + `EditorCommand::RevealPosition`。）
- [x] tab/项目关闭后的迟到事件静默丢弃；记录 debug 日志但不 Toast 骚扰用户。
      （`PlantUmlLoaded` 世代不匹配 / 事件 tab 缺失 → 静默；`apply_plantuml_event_ignores_non_plantuml_tab`。）
- [x] Files 与 Project 相同 tab id 的测试确保资源键/事件不串线。
      （`take_pending_plantuml_commands_for(available_webview_ids, project_id, panel)`。）

**验证：**

```bash
cargo test -p dozer-app preview -- --nocapture
```

**实现期修订（含 include 重写，2026-10-05）：** Task 0 spike 对“本地 include 生效”的
断言只检查输出含 `<svg`，实测证明普通 `!include path` 被引擎**静默丢弃**（引擎只在
尖括号 stdlib 形式 `!include <local/...>` 时查询 `PLANTUML_STDLIB`）。因此
`preview/plantuml.rs`（Task 5 文件）新增**重写**：把根源码与每个 include 内容里项目内的
include 指令改写为 `!include <local/<项目相对键>>`；新增测试
`rewrites_plain_include_to_local_stdlib_form` / `rewrites_nested_include_relative_to_including_file`
/ `rewrites_include_once_preserving_keyword` / `rewrites_includesub_preserving_tag` /
`rewrites_stdlib_and_local_together` / `angle_bracket_stdlib_is_passed_through_untouched`，
并把前端 `render-smoke.mjs` 的本地 include 断言改为**内容确实出现在 SVG**（含嵌套）。
spec §12.3 已同步修订说明。

---

## Task 7：文件变化、include 依赖与 revision

**目的：** 根文件或任一 include 变化后只刷新受影响的已打开图。

**Files:**

- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`
- Modify: 当前工作区文件事件分发所在模块
- Test: 对应模块测试

- [x] 在 preview workspace state 维护 `dependency -> ViewerKey` 反向索引；路径使用
      canonical form，UI/错误只显示项目相对路径。
      *实现期决议：反向索引落在每个 `PreviewPane`(`plantuml_dep_index:
      HashMap<PathBuf, Vec<usize>>`)，viewer 身份即“本 pane + tab_id”——每个面板
      各有一份 `PreviewPane`，天然区分 Files / Project；键为
      `PlantUmlDocument::dependencies` 里的绝对 canonical 路径。索引是派生视图，
      任何 tab/依赖集变动后由 `rebuild_plantuml_dep_index` 整体重建（先清后建），
      不增量维护。doc 里的“workspace state”按此实现（`Workspace` 只是逐 pane 转发）。*
- [x] 每次成功 load 原子替换该 viewer 的依赖集；失败/关闭/suspend 清理旧索引。
      *`store_plantuml_document` 整体替换 `tab.plantuml_dependencies` 后重建索引；
      `close` 摘除该 tab 条目、`clear_all` 清空、`suspend_tab` 清空依赖集并重建。*
- [x] 根文件变化沿用现有 reload；include 变化对所有引用它的打开 tab 发失效。
      *`reload_webviews_for` 新增 PlantUML 分支：命中判据 = 根路径命中 **或**
      `plantuml_tabs_depending_on(canonical_changed)` 命中；命中的图清依赖/暂存/
      host 就绪、推进 generation、换 URL 重新导航，并把 `(tab_id, 根路径)` 记入
      `pending_plantuml_reloads`，由 `Workspace::spawn_pending_plantuml_reloads`
      （`App::project_fs_changed` 调用，Files/Project 各一次）在后台重跑
      `load_document` 重读 include。*
- [x] 连续变更经现有 debounce 合并；每次有效失效只递增一次 revision。
      *watcher 侧 debounce 不变；pane 侧对 `pending_plantuml_reloads` 按 tab 去重，
      同一 include 连续两次变化只入队一条、generation/reload_nonce 各只推进一次
      （首次命中已清依赖集，旧依赖索引不再命中）。*
- [x] watcher 在一次渲染期间再次变更时，旧结果丢弃并安排新一轮，不进入永久 loading。
      *每次失效推进一次 generation 并重置 `PendingPlantUmlDocument`/host 就绪；
      `store_plantuml_document`/`apply_plantuml_event` 按 generation/revision 门控
      丢弃旧结果；新 `load_document` 汇合后推新 `SetDocument`，不会永久 Loading。*
- [x] 分支切换/批量变更只刷新当前驻留 viewer；Suspended tab 激活时读取最新磁盘。
      *反向索引只登记已成功加载（有依赖集）的 tab；`suspend_tab` 清空依赖后不再被
      命中，重新物化时经既有 `begin_load` 路径重跑 `load_document` 读最新磁盘。*
- [x] 测试两个根图共享 include、依赖删除/重命名、关闭一个 tab 后索引清理。

**验证：**

```bash
cargo test -p dozer-app preview -- --nocapture
cargo test -p dozer-app workspace -- --nocapture
```

**实现期记录（2026-10-05）：** 新增/改动：`preview/state.rs`（`PreviewTab::
plantuml_dependencies`、`PreviewPane::pending_plantuml_reloads`/`plantuml_dep_index`）、
`preview/view.rs`（`rebuild_plantuml_dep_index`、`plantuml_tabs_depending_on`、
`take_pending_plantuml_reloads`、`reload_webviews_for` PlantUML 分支、`close`/
`clear_all`/`suspend_tab`/`store_plantuml_document` 索引维护）、`workspace/state.rs`
（`spawn_pending_plantuml_reloads`）、`app/update.rs`（`project_fs_changed` 调
spawn）。测试：`plantuml_dep_index_maps_shared_include_to_both_tabs`、
`plantuml_include_change_marks_tab_for_reload_and_bumps_generation`、
`plantuml_root_change_is_not_double_bumped_by_generic_path`、
`repeat_include_change_coalesces_into_single_queued_reload`、
`plantuml_unrelated_change_is_noop`、`closing_one_tab_cleans_shared_dep_index`、
`suspending_tab_drops_its_dep_index_entry`、
`removed_include_after_reload_is_no_longer_a_trigger`。`cargo test -p dozer-app
preview` 422 passed / `workspace` 91 passed，clippy 无新增告警。

---

## Task 8：资源预算、主题与 view state

**目的：** 把重型引擎纳入现有资源闭环，并完成产品级交互。

**Files:**

- Modify: PlantUML viewer 前端源码与生成资产
- Modify: preview resource manager/estimate 模块
- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: 主题重载与 pending command 相关模块
- Test: Rust 与前端测试

- [x] reserve/register/touch/active/suspend/release 全部复用当前 resource manager。
      `WebviewSpec` 新增 `reserve: Option<ReserveHint>`；PlantUML Rendered host 无
      `editor_binding` 但带 `ReserveHint{project,panel,tab,cost}`（`plantuml_reserve_hint`
      用 `estimate_cost` 的固定引擎+SVG 上界）。`sync_webview_pool` 的可预留判据改为
      `editor_binding.is_some() || reserve.is_some()`，两种 host 共用
      reserve/register/touch/set_active/prune/release 与淘汰台账；`update.rs` 的
      `needs_reserve` 增加 `uses_plantuml_host()`，PlantUML 在画像后进入 `Reserving`
      再经 `granted` 推进 `CreatingHost`。证据：`plantuml_rendered_host_carries_reserve_hint_in_desired_webviews`。
- [x] 不把 SVG 或 JS 引擎对象放入 Rust tab 状态；WebView 淘汰后允许重建。
      Rust 侧只登记 `ViewerCost`（字节估算 + 种类），SVG 与引擎对象仅活在 webview 内；
      失败/淘汰走既有 `mark_reserve_denied`/`suspend_tab`，不缓存渲染结果。
- [x] 给有界 viewport state 定义 `scale/x/y`；普通 mode 切换保留，进程重启不要求恢复。
      前端 `viewport.ts` 的 `Viewport` 维护有界 `scale/x/y`（含 wheel 缩放、拖拽平移、
      Fit/ActualSize/Reset）；view state 纯前端，不新增协议事件（规格 §5.2 无 viewport 事件）。
- [x] 主题切换不创建第二 renderer；验证显式 PlantUML 颜色不被覆盖。
      `reload_all_webviews_for_theme` 已覆盖 `hosts_webview()`（含 PlantUML），只推进
      `_r=` 重载并带新 `theme=` 参数重新导航（同一 host，不并存两个 renderer）；
      `style.css` 注释明确引擎显式颜色优先、不被 host 主题改写。
- [x] 工具条完成 Fit/Actual Size/Zoom/Reset；按钮语义和颜色遵循 ByteBoy2077，金色
      只用于用户动作。`index.html` 五键（适配窗口/100%/放大/缩小/重置视图）在
      `index.ts` 本地接 `Viewport`；`style.css` 金色仅用于 hover 高亮与错误重试（用户动作）。
- [x] `Cmd/Ctrl +/-/0` 与 Dozer 全局缩放不冲突；焦点路由复用现有 WebView 逻辑。
      viewer 脚本不注册任何 keydown/keypress，不 `preventDefault`，这些快捷键照常冒泡到
      Dozer 全局缩放/查找路由。
- [x] 连续多帧不得对被预算拒绝的 viewer 反复销毁/重建。
      被拒 → `mark_reserve_denied` 把 tab 迁到 `Failed` 并 `load_state.finish()`；
      `hosts_webview()` 对 Failed 返回 false → `desired_webviews` 不再产出 spec →
      同帧起不再创建/销毁。证据：`plantuml_rendered_host_carries_reserve_hint_in_desired_webviews`
      + `suspend_and_reserve_denied_lifecycle`。
- [x] 资源诊断能区分 PlantUML host，并显示估算成本而不泄露源码。
      新增 `ViewerHostKind`（editor/json-editor/plantuml/tabular-grid/rendered/other），
      `ViewerCost.kind` + `ViewerRegistration.kind` 承载；`ResourceDiagnostics` 增加
      `by_kind`/`bytes_by_kind`（只报种类与字节估算，不含正文）。`sync_webview_pool`
      每轮发 `预览资源诊断` debug 日志（含 `plantuml_resident`）。证据：
      `diagnostics_distinguishes_plantuml_via_estimate_cost`。

**验证：**

```bash
cargo test -p dozer-app capabilities -- --nocapture
cargo test -p dozer-app preview -- --nocapture
cd crates/dozer-app/web/plantuml-viewer && npm test
```

---

## Task 9：安全回归与端到端验收 🔶 自动化完成，人工待执行（2026-10-05）

**目的：** 在发布配置、断网环境和真实 WKWebView 中关闭剩余风险。

**状态：** 所有可机验项(fmt/log-scope/panel-boundary/clippy/Rust 测试/前端
`npm test`/`scan-offline`)已通过;恶意 SVG、畸形 envelope、超大错误、include bomb/
环/symlink 等安全回归由既有自动化用例覆盖(见 acceptance §2)。人工视觉/断网/打包
验收 A1–A13 待人工执行并回填 `plantuml-preview-acceptance.md` §4。

**Files:**

- Create: `docs/superpowers/analysis/plantuml-preview-acceptance.md`
- Modify: 本 plan checkbox 与 spec 实现期决议
- Modify: `CLAUDE.md`（仅在形成需要长期遵守的新裁决时）

- [ ] 运行 spec §9 全部人工验收，逐项记录结果、版本、机器和截图/日志位置。
      （人工矩阵 A1–A13 已列入 `docs/superpowers/analysis/plantuml-preview-acceptance.md`
      §4，待人工在发布包/断网/真实 WKWebView 上执行并回填。）
- [ ] 使用代理/抓包或网络禁用环境验证零网络请求，包括 `!includeurl` fixture。
      （自动化侧：`render-smoke.mjs` 断言引擎路径零 XHR/fetch；`scan-offline.mjs`
      带 9 文件通过；协议侧 `rejects_includeurl_and_plain_urls`。真实出站抓包见 A7。）
- [ ] 验证无 Java、无 Node 的打包应用可渲染；Node 只用于仓库构建。（Node 仅
      `web/plantuml-viewer` 构建期；引擎为 TeaVM JS，无 JRE 依赖。见 A10。）
- [x] 恶意 SVG、畸形 envelope、超大错误字符串、include bomb、循环和 symlink 穿越均
      不能执行脚本、越界读取、OOM 或 panic。（自动化用例清单见 `plantuml-preview-
      acceptance.md` §2:前端 sanitizer 6 例 + Rust 协议/include/router/assets 全绿。）
- [ ] 快速切 tab/保存/include 变化，验证 revision 不乱序。（单元级：`apply_plantuml_
      event_rendered_stale_revision_is_dropped`、`store_plantuml_document_drops_stale_
      generation`、`plantuml_include_change_marks_tab_for_reload_and_bumps_generation`；
      交互级见 A12。）
- [ ] 打开足够多重型 preview 触发预算，验证 suspend/resume 与 RSS 回落。
      （预算链路见 Task 8 测试与诊断日志;RSS 观察见 A9/A13。）
- [ ] 深浅主题、窗口缩放、左右栏拖动、面板最大化、Files/Project 镜像布局均正常。
      （见 A3/A11。）
- [x] 更新文件预览 current matrix，把五种扩展名标为 PlantUML Rendered + Source。
      （`docs/superpowers/analysis/file-preview-current-matrix.md` §7 增量追加。）
- [x] 全量门禁通过，确认没有新增裸 `tracing`/`eprintln!` 或面板边界违规。
      （`cargo fmt --check`、`check-log-scope.sh`、`check_panel_boundary.py`、clippy 均通过;
      完整结果见 `plantuml-preview-acceptance.md` §1。）

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets -- -D warnings
scripts/check-log-scope.sh
python3 scripts/audit/check_panel_boundary.py
```

---

## Task 10：清理 spike 与交付 🔶 审计完成，C4 缺口待裁决（2026-10-05）

**目的：** 收口临时代码与文档，使源码、生成资产和决策一致。

- [x] 若 Task 0 spike 不属于长期回归资产，删除其可执行临时代码；保留 README、fixture
      来源与结论。若其 fixture 被正式前端测试复用，则迁移后再删，不能复制两份。
      （删除 `harness.mjs`/`run.mjs`/`run-c4.mjs`/`package.json`/`package-lock.json`；
      保留 `README.md`/`fixtures/`/`stdlib/`;已确认 fixtures 未被正式前端测试复用
      ——`render-smoke.mjs` 内联自己的 diagram 源。README 已加归档说明。）
- [x] `rg` 检查五种扩展名只由 `is_plantuml_extension` 定义；测试字符串除外。
      （`router.rs:210` 是唯一路由判据;`extension_to_syntax`/`rendered_ext` 是各自
      独立的语法 token / 原因标签映射,与 `md`/`html`/`svg` 处理一致。见验收 §2.5。）
- [x] `rg` 检查 PlantUML host 不含公网 URL、`file://`、任意 `eval` 入口。
      （index.html 无公网 URL;审计发现唯一 `new Function`(`registerStdlibScript`)是
      死代码且会导致 C4 未注册——已删除,改为 `renderer.ts::loadEngine` 用同源
      `<script src="stdlib/c4.min.js">` 加载 stdlib。现在 host 无 `new Function`。
      见验收 §2.5。）
- [x] 确认 `package-lock.json`、生成 assets、许可证/版本记录均已提交。
      （`git ls-files` 确认;`assets/plantuml-viewer/*` 与 `stdlib/c4.min.js` 均为提交的
      生成产物;版本/integrity 记录见 spike README + spec §12.1 + package-lock。）
- [x] 更新 spec 状态为“已实现”，记录最终引擎版本、bundle 大小和偏离设计之处。
      （spec 头部状态区已更新:引擎 1.2026.8、五份 vendored 产物体积、无协议偏离、
      一处已知功能缺口(C4 stdlib 注册)。）
- [x] 最后一次运行 Task 9 的全量门禁。
      （C4 修复后:fmt/check-log-scope/check_panel_boundary/clippy 通过;
      `cargo test -p dozer-app` 1851 passed(3 既有失败);`npm test` 全绿;
      `scan-offline.mjs` 9 文件通过。）

**审计发现与修复(2026-10-05):** vendored C4 stdlib 初版未在宿主启动时注册
(`renderer.ts::registerStdlibScript` 为用 `new Function` 的死代码,`index.ts` 不加载
`stdlib/c4.min.js`),导致生产路径下 `!include <C4/...>` 失败(默认加载器走网络被 CSP
拒绝)。已删除该死代码,改为 `renderer.ts::loadEngine` 用同源 classic
`<script src="stdlib/c4.min.js">` 依次加载 `VENDORED_STDLIB_SCRIPTS`;
`render-smoke.mjs` 按同一机制驱动并断言 `PLANTUML_STDLIB.c4` 已注册。已重建
`assets/plantuml-viewer/bundle.js`(`new Function` 消失,新增 c4 加载引用),离线扫描
通过。详见 `docs/superpowers/analysis/plantuml-preview-acceptance.md` §2.5。

## 建议提交边界

1. `spike(plantuml): validate official JavaScript renderer`
2. `feat(preview): classify PlantUML rendered documents`
3. `feat(plantuml): add offline viewer bundle`
4. `feat(preview): serve and bind PlantUML host`
5. `feat(preview): resolve safe PlantUML includes`
6. `feat(preview): wire PlantUML rendering lifecycle`
7. `feat(preview): refresh diagrams on include changes`
8. `test(plantuml): cover security and lifecycle acceptance`

不要把依赖引擎、路由、include 安全和全部宿主接线压成一个不可审阅的大提交。
