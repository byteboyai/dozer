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

- [ ] 先写 fixture 测试：相对 include、嵌套、once、环、includesub（按 spike 裁决）。
- [ ] 拒绝 `!includeurl`、URL、绝对路径、`file://` 和无法静态授权的外部 include。
- [ ] canonicalize 根路径、源文件和 include；验证 include 最终路径仍在项目根。
- [ ] 用真实 symlink fixture 覆盖“文本路径在根内、目标在根外”。
- [ ] 实施 spec 的根大小、深度、文件数和总字节上限；错误携带安全的相对 include 链。
- [ ] 非 UTF-8、目录、设备文件、读取失败返回具名错误。
- [ ] 不展开 PlantUML 宏/条件语义；只建立官方引擎可见的虚拟文件集合。
- [ ] 输出依赖列表去重且排序稳定，供文件监听和确定性测试。

**验证：**

```bash
cargo test -p dozer-app preview::plantuml -- --nocapture
```

---

## Task 6：预览加载、WebView 创建与 IPC 接线

**目的：** 让 Files/Project 真正打开 PlantUML host，并完成 ready → render → ack。

**Files:**

- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/preview/view.rs`
- Modify: `crates/dozer-app/src/preview/webview.rs`
- Modify: `crates/dozer-app/src/runtime.rs`
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/app/update.rs`（若消息处理实际落在此）
- Test: 对应模块测试

- [ ] PlantUML tab 在 Rendered mode 产生一个普通 preview WebView spec；Source mode 只产生
      CodeMirror spec，不得同时驻留两份 host。
- [ ] host URL 注入完整 `proj/panel/tab/doc`，与其它 rendered host 同一绑定来源。
- [ ] runtime IPC 识别 PlantUML envelope，先解析、再 `HostBinding::validate`、最后投递。
- [ ] 收到 `Ready` 后后台 `load_document`，成功才推 `SetDocument`；文件 IO/include
      解析不得阻塞 UI 线程。
- [ ] `Rendered` 只在 revision 与当前 load 匹配时迁为 Ready。
- [ ] `Failed` 迁入统一 `BackendState::Failed`；可重试错误显示 Retry，策略拒绝不可
      伪装成 retryable 引擎错误。
- [ ] `OpenSource { line }` 切换当前 tab 到 Source mode，并复用现有 reveal 命令定位。
- [ ] tab/项目关闭后的迟到事件静默丢弃；记录 debug 日志但不 Toast 骚扰用户。
- [ ] Files 与 Project 相同 tab id 的测试确保资源键/事件不串线。

**验证：**

```bash
cargo test -p dozer-app preview -- --nocapture
```

---

## Task 7：文件变化、include 依赖与 revision

**目的：** 根文件或任一 include 变化后只刷新受影响的已打开图。

**Files:**

- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`
- Modify: 当前工作区文件事件分发所在模块
- Test: 对应模块测试

- [ ] 在 preview workspace state 维护 `dependency -> ViewerKey` 反向索引；路径使用
      canonical form，UI/错误只显示项目相对路径。
- [ ] 每次成功 load 原子替换该 viewer 的依赖集；失败/关闭/suspend 清理旧索引。
- [ ] 根文件变化沿用现有 reload；include 变化对所有引用它的打开 tab 发失效。
- [ ] 连续变更经现有 debounce 合并；每次有效失效只递增一次 revision。
- [ ] watcher 在一次渲染期间再次变更时，旧结果丢弃并安排新一轮，不进入永久 loading。
- [ ] 分支切换/批量变更只刷新当前驻留 viewer；Suspended tab 激活时读取最新磁盘。
- [ ] 测试两个根图共享 include、依赖删除/重命名、关闭一个 tab 后索引清理。

**验证：**

```bash
cargo test -p dozer-app preview -- --nocapture
cargo test -p dozer-app workspace -- --nocapture
```

---

## Task 8：资源预算、主题与 view state

**目的：** 把重型引擎纳入现有资源闭环，并完成产品级交互。

**Files:**

- Modify: PlantUML viewer 前端源码与生成资产
- Modify: preview resource manager/estimate 模块
- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: 主题重载与 pending command 相关模块
- Test: Rust 与前端测试

- [ ] reserve/register/touch/active/suspend/release 全部复用当前 resource manager。
- [ ] 不把 SVG 或 JS 引擎对象放入 Rust tab 状态；WebView 淘汰后允许重建。
- [ ] 给有界 viewport state 定义 `scale/x/y`；普通 mode 切换保留，进程重启不要求恢复。
- [ ] 主题切换不创建第二 renderer；验证显式 PlantUML 颜色不被覆盖。
- [ ] 工具条完成 Fit/Actual Size/Zoom/Reset；按钮语义和颜色遵循 ByteBoy2077，金色
      只用于用户动作。
- [ ] `Cmd/Ctrl +/-/0` 与 Dozer 全局缩放不冲突；焦点路由复用现有 WebView 逻辑。
- [ ] 连续多帧不得对被预算拒绝的 viewer 反复销毁/重建。
- [ ] 资源诊断能区分 PlantUML host，并显示估算成本而不泄露源码。

**验证：**

```bash
cargo test -p dozer-app capabilities -- --nocapture
cargo test -p dozer-app preview -- --nocapture
cd crates/dozer-app/web/plantuml-viewer && npm test
```

---

## Task 9：安全回归与端到端验收

**目的：** 在发布配置、断网环境和真实 WKWebView 中关闭剩余风险。

**Files:**

- Create: `docs/superpowers/analysis/plantuml-preview-acceptance.md`
- Modify: 本 plan checkbox 与 spec 实现期决议
- Modify: `CLAUDE.md`（仅在形成需要长期遵守的新裁决时）

- [ ] 运行 spec §9 全部人工验收，逐项记录结果、版本、机器和截图/日志位置。
- [ ] 使用代理/抓包或网络禁用环境验证零网络请求，包括 `!includeurl` fixture。
- [ ] 验证无 Java、无 Node 的打包应用可渲染；Node 只用于仓库构建。
- [ ] 恶意 SVG、畸形 envelope、超大错误字符串、include bomb、循环和 symlink 穿越均
      不能执行脚本、越界读取、OOM 或 panic。
- [ ] 快速切 tab/保存/include 变化，验证 revision 不乱序。
- [ ] 打开足够多重型 preview 触发预算，验证 suspend/resume 与 RSS 回落。
- [ ] 深浅主题、窗口缩放、左右栏拖动、面板最大化、Files/Project 镜像布局均正常。
- [ ] 更新文件预览 current matrix，把五种扩展名标为 PlantUML Rendered + Source。
- [ ] 全量门禁通过，确认没有新增裸 `tracing`/`eprintln!` 或面板边界违规。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets -- -D warnings
scripts/check-log-scope.sh
python3 scripts/audit/check_panel_boundary.py
```

---

## Task 10：清理 spike 与交付

**目的：** 收口临时代码与文档，使源码、生成资产和决策一致。

- [ ] 若 Task 0 spike 不属于长期回归资产，删除其可执行临时代码；保留 README、fixture
      来源与结论。若其 fixture 被正式前端测试复用，则迁移后再删，不能复制两份。
- [ ] `rg` 检查五种扩展名只由 `is_plantuml_extension` 定义；测试字符串除外。
- [ ] `rg` 检查 PlantUML host 不含公网 URL、`file://`、任意 `eval` 入口。
- [ ] 确认 `package-lock.json`、生成 assets、许可证/版本记录均已提交。
- [ ] 更新 spec 状态为“已实现”，记录最终引擎版本、bundle 大小和偏离设计之处。
- [ ] 最后一次运行 Task 9 的全量门禁。

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
