# Git Log 面板 Diff 视图改用 CodeMirror 设计

## 背景

Git Log 面板（`extensions/git_log.rs`）右下角 diff pane 当前由
`extensions/diff_render::colored_diff_lines` 纯 iced 渲染：整份
unified patch 文本逐行按 `+`/`-`/`@@` 前缀染色，没有真语法高亮，也没有
逐字符级别的改动高亮。数据来源是 `DiffFileEntry.patch`——`commit_detail()`
用 git2 对整个 commit 一次性算出、per-file 拆分后的 unified patch 文本，
超过 `MAX_PATCH_CHARS`（20000 字符）截断。

文件预览（`preview/`）已经用 CodeMirror 6（经 `dozer://editor/` webview
host）取代了老 iced `CodeView`，本设计把 git log 的 diff pane 也迁到同一套
CodeMirror 基础设施，换取真语法高亮 + 逐字符 diff 高亮。这是纯视觉体验升级
（用户明确判断视觉体验对 Dozer 优先级很高），不解决当前渲染器的功能缺陷——
现状可用，只是不好看/不好读。

范围只覆盖 Git Log 面板内联 diff pane。`platform/file_history_overlay.rs`
（文件历史弹窗，独立原生子窗口渲染）不在本次范围内：它是完全独立的原生
子窗口，跟本设计的 webview 宿主机制不共享任何基建，是否迁移留作后续独立
话题。

## 目标与非目标

### 目标

- Git Log 面板选中某个改动文件后，diff pane 用 CodeMirror 6 的
  `@codemirror/merge` `unifiedMergeView` 渲染：真语法高亮（按文件扩展名）+
  逐字符级 diff 高亮，单栏内联布局（不是左右并排）。
- 复用现有 `dozer://editor/` webview host 基建（协议 scheme、IPC envelope
  校验、`sync_webview_pool`、runtime 事件分派），只新增一个 `mode=diff`
  启动分支，不新起一套 URL 前缀/JS bundle。
- 二进制文件、超过大小上限的 diff、blob 读取失败：优雅降级到占位文案，
  不强行塞进 CodeMirror。
- 新增/删除文件（单侧无内容）：`unifiedMergeView` 原生支持，不需要特殊分支。

### 非目标

- 不做并排（side-by-side）双栏视图。
- 不做整文件浏览（跳出 diff 上下文看完整文件）——诉求出现时再单独评估。
- 不迁移 `file_history` 弹窗的 diff 渲染。
- 不改变 `DiffFileEntry.patch`（unified patch 文本）的既有用途——它仍是
  patch 文本的唯一真相源；CodeMirror 渲染改用另外新增的 blob 内容字段，
  两者并存，互不影响。
- 不引入编辑能力：diff pane 恒只读（历史 commit 内容本来就不可编辑，
  符合 Dozer 核心原则）。

## 架构

### 为什么不是独立 host / 独立 JS bundle

`unifiedMergeView` 需要"旧版本全文 + 新版本全文"两份完整内容自己跑 diff
算法（逐字符高亮 + 语法高亮），不是喂 unified patch 文本——这是和现有
`dozer://editor/` "按磁盘路径读单个文件"模式最大的数据来源差异，但两者
用的是同一套 CodeMirror 6 依赖、同一套协议 scheme 注册、同一套 IPC
envelope 校验机制。拆成独立 host/独立 JS bundle 只是重复造轮子；正确做法
是在现有 `editor.js` bundle 里新增一个由 URL `mode=diff` 触发的启动分支。

曾考虑过的替代方案：

1. **复用 editor host 的文件读取路径**（把旧版本内容伪装成 `__file__`
   协议约定下的虚拟文件）——否决。旧版本内容来自 git blob，不是磁盘文件，
   protocol handler 现在是无状态的纯 fs 读取函数，让它变成"git-aware"污染
   了其单一职责，且旧版本内容按 commit 变化，语义上不该有一个稳定"路径"。
2. **不读 blob，直接用已有的 unified patch 文本 + 自定义 diff 染色**——否决。
   `unifiedMergeView` 本质要两份完整文档才能自己算 diff；喂 patch 文本这条
   路线和"单栏内联、逐字符高亮"的既定视图形式技术上矛盾，做不到。

### 新增/修改组件

| 文件 | 改动 |
|------|------|
| `crates/dozer-app/src/preview/git_diff_host.rs`（新增） | `GitDiffHostBinding`：固定单槽位绑定（见下），`url()` 构造 `dozer://editor/index.html?mode=diff&...`。 |
| `crates/dozer-app/src/preview/webview_protocol.rs` | `EditorCommand` 新增 `SetDiffDocument { old_text, new_text, language, revision, read_only }` 变体。 |
| `crates/dozer-app/src/app/app.rs` | 新增 `GIT_LOG_DIFF_WEBVIEW_ID` 常量；`App::preview_desired` 新增 `PanelKind::GitLog` 分支；新增算 git log 面板"右下 diff pane"矩形的几何函数。 |
| `crates/dozer-app/src/extensions/git_log.rs` | `DiffFileEntry` 新增 `old_blob: Option<git2::Oid>` / `new_blob: Option<git2::Oid>`；`Message` 新增 `DiffContentLoaded`；`SelectFile` 处理里按需派发异步 blob 读取 Task；`diff_pane_view` 按"是否有已加载且可渲染的 diff 内容"决定渲染 webview 占位区域还是走现有 iced 占位文案分支。 |
| `crates/dozer-app/web/editor/package.json` | 新增 `@codemirror/merge` 依赖。 |
| `crates/dozer-app/web/editor/src/main.ts` | 按 `mode=diff` URL 参数挂载 `unifiedMergeView`；监听 `SetDiffDocument` 命令。 |
| `crates/dozer-app/web/editor/src/protocol.ts` | 新增 `SetDiffDocument` 命令的 TS 类型与解析（镜像 Rust `EditorCommand` 的 serde 契约）。 |

### 单槽位绑定（不按 tab 池化）

现有 `EditorHostBinding`/`PROJECT_PREVIEW_ID_OFFSET` 是"每个 tab 一个
webview"的池化设计，服务 Files/Project 面板可以同时开多个预览 tab 的场景。
Git Log 面板任意时刻只有一个"选中文件"，没有 tab 概念，所以：

- `GIT_LOG_DIFF_WEBVIEW_ID` 是一个固定常量（不是偏移池的起点），
  `desired_webviews()` 最多返回一个 `WebviewSpec`。
- `GitDiffHostBinding` 的绑定信息固定（不含 commit oid / 文件路径）：
  `document_id` 恒为 `"gitlog-diff"`，`panel` 为 `PanelKind::GitLog`，
  `tab_id` 固定 `0`。绑定只用来证明"这条消息确实来自 git log 的 diff
  webview"，不需要证明"当前具体是哪个文件"——具体内容通过命令推送，
  内容对不对由 Rust 侧的 `revision` 递增和 `SelectFile` 触发的 stale-guard
  保证（见下）。
- 切换选中文件时 webview 实例**不销毁重建**，只是再发一次
  `SetDiffDocument`（`revision` 递增）换内容，减少 remount 闪烁。
- 面板不可见、未选中文件、或选中文件不可渲染（二进制/超限/读取失败）时，
  `desired_webviews()` 返回空，`sync_webview_pool` 据此销毁 webview 实例，
  面板退回现有 iced 占位文案渲染。

Git Log 面板状态本身不按项目分（既有非目标，本次不改），diff webview 也
因此是全局单例，不需要项目维度的 id 偏移。

## 数据流

1. 用户点文件列表某一行 → `git_log::Message::SelectFile(path)`。
2. git_log 模块记下 `selected_file`；若该文件在当前 `CommitDetail` 里存在
   对应 `DiffFileEntry` 且判定可渲染（blob 非二进制、大小在上限内——判定
   逻辑见下），派发异步 Task：用 git2 按 `DiffFileEntry.old_blob`/
   `new_blob`（可能为 `None`，对应新增/删除文件）读取 blob 内容。
3. Task 结果回落 `git_log::Message::DiffContentLoaded(commit_oid, path,
   Result<(String, String, &'static str), String>)`（old_text, new_text,
   language token）。回落时校验 `commit_oid` 与 `path` 仍与当前
   `state.detail_oid` / `state.selected_file` 一致，不一致则丢弃（防止
   用户手快切换 commit/文件后旧结果覆盖新选择，同 `DetailLoaded` 现有的
   stale-guard 手法）。
4. `App::preview_desired` 的 `PanelKind::GitLog` 分支：git log 面板当前
   `side` 可见、且 git_log state 里有已加载的 diff 内容时，产出一个
   `WebviewSpec`（URL 来自 `GitDiffHostBinding::url()`，矩形来自新增的
   git log 面板专用几何函数）；否则产出空 Vec。
5. `sync_webview_pool` 按 `desired_webviews()` 的差集创建/销毁 webview 实例
   （复用现有机制，无需改动）。
6. webview 首次挂载只加载空壳页面（URL 不带 diff 正文，避免几十 KB 文本
   塞进 URL）；JS 端 CodeMirror 挂完后回 `ready` envelope（复用现有 editor
   host 的 ready 事件）。
7. Rust 收到 `ready`（经 `HostBinding` 校验绑定）后，发送
   `EditorCommand::SetDiffDocument { old_text, new_text, language, revision,
   read_only: true }`。
8. 用户切换选中文件：重复步骤 1-3，得到新内容后 Rust 直接再发一次
   `SetDiffDocument`（`revision` 递增），webview 实例保持挂载不重建。

## 二进制 / 大小判定

- 二进制判定：blob 内容含 NUL 字节，或整体不是合法 UTF-8（不做 lossy
  容忍——diff 场景下半个字符乱码比不显示更糟）。命中则该文件判定"不可
  渲染"，回落现有 iced 占位文案（"(二进制文件，不支持预览)"类文案，具体
  措辞实现时定）。
- 大小上限：新增常量 `MAX_DIFF_BLOB_BYTES = 512 * 1024`（512KB，old/new 各自
  判定，任一侧超限即判定该文件"不可渲染"），回落占位文案（不做部分截断
  渲染——截断后的 `unifiedMergeView` diff 结果没有意义，比不显示更容易
  误导）。
- 新增/删除文件：`old_blob`/`new_blob` 一侧为 `None` 时该侧内容按空字符串
  处理，`unifiedMergeView` 原生渲染成整片新增/删除，不需要特殊分支。
- blob 读取失败（如并发操作导致仓库状态变化）：`DiffContentLoaded` 携带
  `Err(String)`，回落占位文案并展示错误信息，不 panic。

## 测试

- `git_diff_host.rs`：绑定/URL 构造单测，镜像 `code_host.rs` 现有测试风格
  （URL 含 `mode=diff`、`panel=gitlog`、`ro=1` 等断言）。
- `git_log.rs`：在现有 test-repo fixture 基础上，为 blob 内容提取新增用例
  ——改动文件、新增文件（`old_blob = None`）、删除文件（`new_blob = None`）、
  二进制文件短路（不尝试解码）、超限文件短路。
- `app.rs`：`preview_desired` 的 `PanelKind::GitLog` 分支单测——该出
  `WebviewSpec` 就出、不该出（未选中/不可渲染/面板不可见）就空。
- `webview_protocol.rs`：`SetDiffDocument` 的序列化/反序列化往返测试。
- `web/editor/src/protocol.test.ts`：镜像最近落地的 "structured
  backend/runtime invariant test" 套路（commit `88889e5b`），补
  `SetDiffDocument` 的 Rust↔TS 序列化契约测试，防止两侧 tag/字段漂移。
- UI 手动验证：正常改动文件、新增文件、删除文件、二进制文件、超大 diff、
  在 diff pane 展开时来回切换选中文件（验证 revision 递增换内容不闪烁/
  不串内容）。
