# 文件预览 Phase B 进度(CodeMirror host)

> 对应计划:`docs/superpowers/plans/2026-09-22-codemirror-agent-editor.md`。
> 本文件记录本轮已落地的部分、刻意未接线的部分,以及后续接线的具体步骤。
> 2026-09-22。

## 已完成

### Task 1:离线前端工程(主体)
- `crates/dozer-app/web/editor/` —— TypeScript + CodeMirror 6 前端包:
  - 依赖锁定(见 `package.json` / `package-lock.json`):`@codemirror/{state,
    view,commands,search,language}` + 首批语言包(rust/python/js/ts/json/
    markdown/html/css)+ `@lezer/highlight`。
  - 构建 `build.mjs`(esbuild):输出 `editor.js`(iife、minify、无 source
    map、无 legal comments)、`editor.css`、`index.html`、`fonts/JetBrainsMono.ttf`
    到 `crates/dozer-app/assets/editor/`(已提交)。产物无 CDN、无运行时
    Node、无本机绝对路径。
  - 主题 `themes.ts`:ByteBoy2077 深/浅两套,语法色锚定终端 16 色角色;
    字体 JetBrains Mono(打包 ttf)+ 系统 CJK fallback。
  - 基础编辑器:行号、折叠 gutter、搜索/替换、选区、history、read-only、
    括号匹配;**未开启**补全/LSP/lint/multicursor/minimap。
  - 单测 `protocol.test.ts`(`node --test`,Node 24 类型擦除)+ `tsc --noEmit`
    类型检查,均通过。
- `.gitignore` 忽略 `web/editor/node_modules/`(产物提交,依赖不提交)。

### Task 3:通用消息 envelope(主体)
- `crates/dozer-app/src/preview/webview_protocol.rs` —— 所有 webview host 共用
  的 `WebviewEnvelope<T>` + `EditorEvent`(ready/selection_changed/
  document_changed/save_requested/focus_changed/viewport_changed/view_state/
  failed)+ `EditorCommand`(set_document/reveal_position/select_range/
  replace_range/open_find/set_read_only/focus/serialize_view_state)。
  - `parse_event` 过大/非法 JSON/未知 kind 一律返回错误,不 panic。
  - `HostBinding::validate` 校验 version/project/panel/tab/document(JS 不能
    自报归属)。
  - `encode_command` 供 Rust 经 `evaluate_script` 下发。
  - 前端 `src/protocol.ts` 是同一契约的 TS 端,含镜像单测。

### Task 2:scheme / CSP / host 描述(部分)
- `crates/dozer-app/src/assets.rs`:`dozer://editor/` 命名空间——`index.html`/
  `editor.js`/`editor.css`/`fonts/*` 从 editor 根(与 flyfish 根同级的
  `editor/`,dev 与打包态同构)服务;`dozer://editor/__file__<abs>` 复用
  `allowed_files` 白名单(**沿用旧的 `assets_root` 参数,editor 根由其兄弟
  目录派生,未改 `handle_protocol` 签名**)。路径穿越(含百分号编码形态)拒绝。
- `crates/dozer-app/web/editor/src/index.html`:严格 CSP(`default-src 'none'`,
  脚本/字体仅 self,`connect-src 'self'` 供 `__file__` fetch,`style-src`
  因 CodeMirror 注入内联样式需 `'unsafe-inline'`)。
- `crates/dozer-app/src/preview/code_host.rs`:`EditorHostBinding`
  `(project_id, panel, tab_id, path)` → 稳定 `document_id` / webview 池 key
  (project 面板用 `PROJECT_PREVIEW_ID_OFFSET`)/ editor URL(逐段编码路径 +
  theme/ro/lang);`panel_and_tab_from_webview_id` 反解;`is_editor_url`;
  `codemirror_enabled()`(Cargo feature `codemirror`,默认关闭)。
- `crates/dozer-app/Cargo.toml`:`[features] codemirror = []`(开发开关,
  默认关闭 → 用户可见行为不变)。

## 已接通的运行时垂直切片

- feature 开启时 `PreviewKind::Code` 不再进入异步 iced editor load；同步创建
  轻量 tab，正文由 editor host 经白名单 scheme 读取。JSON/Streamed 仍保留
  原路径，feature 关闭行为不变。
- App 按 project/panel 为 Code tab 生成 `EditorHostBinding` 与 editor
  `WebviewSpec`；Files/Project 继续使用现有 pool id 偏移。
- runtime 创建 WebView 时捕获 Rust 可信 binding，解析、校验 envelope 后才派发
  `EditorWebviewEvent`；同 pool key 的 host 类型变化会重建 WebView，避免复用旧
  IPC closure。editor 不注入会抢占 CodeMirror Cmd/Ctrl-C、Cmd/Ctrl-F 的通用脚本。
- `PreviewTab` 镜像 revision、selection、viewport 与错误；App 处理 ready、change、
  save、view state 与 failed，并拒绝 binding/path 不匹配、revision 回退或 payload/
  envelope revision 不一致的事件。
- iced 渲染层不再重复绘制 CodeMirror tab。
- **原子保存**(`preview/text_save.rs`):`save_text_atomic` 读原文件约定 → 编码
  → 同目录临时文件 → `flush`+`sync_all` → `rename` 覆盖;保持 UTF-8 BOM 与原
  换行约定(原文件以 CRLF 为主则写回 CRLF;`fetch().text()` 会丢 BOM,保存时
  按原件补回)。`SaveRequested` 已改走它,失败只置 `web_error`、保留 dirty。
- `hosts_webview` 增加旧行为兜底子句(非 CodeMirror tab 在读盘失败等情况下仍
  退回 Flyfish),避免 feature 关闭时同步 push_tab 失败路径出现空白页。
- **Rust→编辑器命令派发(骨架)**:`PreviewPane` 增待下发命令队列
  (`queue_editor_command`/`take_pending_editor_commands`);`App::
  take_preview_editor_scripts(kind)` 按 Rust 可信 binding + tab 镜像 revision
  组装 `encode_command` envelope 并包成 `window.__dozer.dispatch(...)` 注入脚本
  (`webview_protocol::dispatch_script`,防御 `__dozer` 未就绪);`window_events::
  apply_pending_editor_commands` 在每帧(同 `apply_pending_preview_find` 节奏)
  经持有句柄的 `evaluate_script` 下发。**Agent 入口(Task 6)与 jump-to-line
  自动接线随后接入**。
- `scripts/build-macos-app.sh` 增拷 `assets/editor` → `Contents/Resources/editor`
  (与 `assets_root` 的兄弟目录约定一致),否则打包态 editor 预览 404。
- **代码健康度 jump-to-line**:`pending_jump_line` 在 editor `ready` 后经命令队列
  自动 `reveal_position`。
- **Task 6 — PreviewContext 全链路(主体)**:
  - `dozer_core::protocol::PreviewContext` 扩展 `revision` / `mode` / `read_only`
    / `selected_text`(上限 `PREVIEW_SELECTED_TEXT_MAX_CHARS=4096`, `set_selected_text`
    截断)/ `visible_start_line` / `visible_end_line`;**全部带 serde 默认值**,
    旧 JSON/旧客户端仍可解码(有回归测试)。
  - `dozer-app` 的 `spawn_preview_context_push` 现同时覆盖老 iced editor 与
    CodeMirror tab(`preview_context_from_web_state` 把镜像的 1-based 选区/可见行
    转成上下文),并填 `mode`/`read_only`。CodeMirror 的 selection/viewport/ready
    事件会触发这段推送(经既有 250ms trailing 防抖)。
  - `dozer-mcp` 的 `get_preview_context` 输出新增这些字段(有预览/无预览两条
    JSON 形状都补齐,`null` 占位)。
  - `dozerd`/`dozer-client` 无需改动(上下文不透明透传)。
  - **App 侧 Agent 命令入口**:`preview_editor_reveal/select/replace`;`replace`
    要求 `expected_revision == tab.web_revision`,失配拒绝(不排队)。
- **上下文即时 flush**:把 push 拆成 `enqueue_preview_context_push(io, delay)`;
  `spawn_preview_context_push`=250ms 防抖,新增 `flush_preview_context_push`=0。
  tab 切换(`preview_select_tab`)、关闭(`PreviewCloseTab`)、失焦
  (`blur_preview_editors`)改走 flush,不再等防抖窗口。
- **非 UTF-8/二进制内容强制只读**:`route_and_backend` 的只读判据加入
  `profile.utf8 == Invalid || content_kind == Binary` —— CodeMirror host 的
  `fetch().text()` 会丢字节,置只读避免编辑后保存损坏原文(backend Code mode
  → ReadOnly → editor URL `ro=1`)。

## 仍未完成(后续 Task 4–7)

1. **非 macOS 无内置查找条/剪贴板差异**、以及 editor 的"外部打开"入口(当前
   非 UTF-8 只做只读,未给外部打开按钮)。
2. 大文件预算/休眠唤醒与失败后外部打开 fallback。
3. Task 5/7 的完整验收:拖动到底/折叠/搜索(In-editor 已具备,需人工验收)、
   IME、路由矩阵;旧 iced editor 删除属 Phase D。

## NSMenu 上下文菜单(有意不实现)

计划 Task 2 要求 editor webview 的上下文菜单走 NSMenu。实为**不必**:WKWebView
的右键菜单本身就是 macOS 原生菜单、天然渲染在 webview 之上(不受"webview 盖住
iced 浮层"问题影响),且提供可用的剪切/复制/粘贴。反过来,自定义 NSMenu 的
粘贴只能走 JS `document.execCommand('paste')`,而 WKWebView **禁止**无用户手势
的 JS 粘贴 —— 自建菜单会把原本可用的粘贴弄坏。结论:editor webview 保留系统
默认原生菜单;若将来要加"在系统应用中打开"等自定义项,再单独做该平台任务。

## 验证

- `cargo check --workspace --all-targets` 默认与 `--features codemirror`:通过。
- `cargo test -p dozer-app` 默认 **1197 passed**、`--features codemirror` **1180
  passed**;两者都只剩改动前已存在的 `agent_icon_maps_each_kind_to_brand_icon`
  一项失败(另有 1 ignored)。`dozer-core` 71 passed、`dozer-mcp` 14 passed。
- `cargo fmt --check` 通过;`cargo clippy` 仅剩既有 `file_history.rs` / `spike/*`
  warning。
- 前端:`npm run typecheck`、`npm test`(6 passed)、`npm run build`:通过。
