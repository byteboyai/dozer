# Dozer 文件预览架构重构设计

## 背景

Dozer 当前文件预览由五套能力并存：Flyfish/wry、`iced_widget::text_editor`
包装的 `CodeView`、iced 原生 Tabular Viewer、iced 原生 JSON Tree Viewer，以及
HTML 的直接 `file://` 页面。`PreviewTab` 通过 `editor`、`tabular`、
`json_tree` 多个 `Option` 的组合表达后端；JSON 甚至同时持有 editor 与 tree。
文件路由、搜索、保存、WebView 池判定和主题重载分别重复判断这些字段。

这个结构已经产生三类根本问题：

1. 老 `CodeView`（`iced_widget::text_editor`）**没有代码折叠 API，也没有
   公开的真实滚动偏移读取接口**，可拖拽滚动条只能靠 `Action::Scroll{lines}`
   计数近似，这两项是 iced 官方 widget 的硬约束，不是可调参数能修的。另有一个
   独立的性能 bug：`cosmic_text::Buffer::set_text` 在视口高度未设置
   （`height_opt == None`）时会把 shaping 窗口退化成 `[0, ∞)`，同步 shape
   整份文档；这个 bug 本身有更便宜的候选修法（先声明视口尺寸再灌文本，见
   `2026-09-19-large-file-editor-performance-design.md`），**不是必须靠整体
   换编辑器才能解决**——真正逼迫换编辑器的是折叠能力的缺失，性能问题只是
   一并被解决的副产品，不应该被当作同等权重的更换理由。
2. 单文件大小分档没有解决多项目、多 tab 恢复时的累计内存和启动阻塞。
3. 后端能力没有统一协议，用户选区、Agent 跳转、搜索、保存和休眠恢复需要按
   每个 viewer 单独接线。

本设计取代并统筹以下既有设计中与文件预览架构冲突的部分：

- `2026-09-19-json-preview-design.md`
- `2026-09-19-large-file-editor-performance-design.md`
- `2026-09-19-tabular-viewer-design.md` 中的路由/生命周期部分

既有实现和测试在对应迁移阶段前继续有效；不是立即删除。

## 产品原则

优先级固定为：

1. **能查看**：尽可能让 Dozer 内部查看所有文件；不支持或不适合内嵌的格式提供
   “使用系统默认应用打开”。
2. **能检索**：文本、代码和大型文件始终尽力提供文件内搜索与结果跳转。
3. **Agent 可感知、可导航**：Agent 能获得当前文件、revision、光标、选区和可见
   范围，并能让 viewer 跳转/选中目标。
4. **编辑从简**：普通代码只支持字符级修改、粘贴、局部替换、undo/redo、保存；
   不建设 IDE。
5. **性能优先**：文件越大，依次关闭编辑、全文语法树、高亮和整文件加载；降级
   后仍保留查看、搜索和 Agent 跳转。
6. **恢复 tab 不等于恢复内容**：启动只恢复 tab 描述；最多立即加载当前项目的
   当前文件。

## 目标与非目标

### 目标

- 用 CodeMirror 6 取代老 iced `CodeView`，解决精确行号、滚动、折叠、搜索、
  选区和程序化跳转。
- 建立唯一 `PreviewBackend` 路由，消除 `editor/tabular/json_tree` 组合判断。
- 启动时检测一次客户端能力，形成全局不可变资源预算。
- 建立跨项目的预览资源管理器，按预算加载、休眠和淘汰 viewer。
- 支持干净 tab 懒恢复和脏 tab recovery snapshot。
- 保留有明确价值的 Flyfish 和 Tabular；分阶段替换普通 JSON Tree。
- 大文件使用 Rust 流式搜索、稀疏行索引和窗口化查看。
- 未知/外部格式提供系统默认应用打开入口。

### 非目标

- LSP、自动补全、重构、诊断、hover、minimap、调试、多光标。
- 把 Dozer 变成通用 IDE。
- 为了统一技术栈而强行把 PDF、Office、表格或媒体变成文本。
- 在迁移验收前删除旧 viewer 或丢失既有能力。

## 总体架构

```text
FileDescriptor + ClientCapabilities + persisted user mode
                         |
                  PreviewRouter
                         |
      +------------------+-------------------+
      |                  |                   |
 CodeMirrorHost      FlyfishHost       Native/Specialized
 code/plain text     rendered media    tabular / streamed
      |                  |                   |
      +------------------+-------------------+
                         |
              PreviewResourceManager
                         |
       selection / search / reveal / save / suspend
```

### 单一后端状态

`PreviewTab` 不再用多个 `Option` 猜测当前后端：

```rust
pub enum PreviewBackend {
    Code(CodeBackend),
    Rendered(RenderedBackend),
    Json(JsonBackend),
    Tabular(TabularBackend),
    Streamed(StreamedBackend),
    External(ExternalBackend),
    Unsupported(UnsupportedBackend),
}

pub enum BackendState<T> {
    Suspended,
    Queued,
    Loading,
    Ready(T),
    Failed(PreviewError),
}
```

可切换视图是一个 backend 内的 mode，不再通过同时持有两个完整 viewer 表达。例如
Markdown 是 `Rendered { source: Some(CodeLanguage::Markdown) }`；普通 JSON 是
`Json { mode: Tree|Text }`。

## 文件路由

路由优先级：用户显式 mode > 安全/资源策略 > 专用 viewer > 文件名/扩展名 >
内容探测 > External/Unsupported。

| 类型 | 默认后端 | 可切换 |
|---|---|---|
| 源码、配置、纯文本、未知 UTF-8 文本 | CodeMirror | — |
| Markdown | Flyfish 渲染 | CodeMirror 源码 |
| HTML/HTM | 隔离渲染 | CodeMirror 源码 |
| SVG | Flyfish 图像 | CodeMirror XML |
| 严格 JSON | JSON Tree（迁移前可先 CodeMirror） | CodeMirror Text |
| JSONC/JSON5 | CodeMirror | 可在后续加容错树，不作为迁移门槛 |
| JSONL/NDJSON | Streamed JSON Viewer | 当前窗口原文 |
| CSV/TSV/XLSX | Tabular | CSV/TSV 可切原文 |
| 图片/PDF/Office/媒体 | Flyfish | — |
| 压缩包 | 专用/外部打开 | — |
| 未知二进制 | External/Unsupported | 系统默认应用 |

无扩展名文件通过文件名注册表和内容探测识别，包括 `Dockerfile`、`Makefile`、
`LICENSE`、`.gitignore`、`.env`。

路由结果必须携带 reason，供诊断和 UI 解释降级原因。

## CodeMirror 6

CodeMirror 运行在独立 `dozer://editor/index.html` wry 页面中，静态 JS/CSS/字体
随应用打包，不使用 CDN，运行时不需要 Node.js。它与 Flyfish（以及 Phase 8 的
`vanilla-jsoneditor`）共享 WebView 池、主题变量、矩形同步、资源预算和 tab
激活/休眠生命周期，但拥有独立 IPC 命令域与 CSP。

macOS 的 WebView 层级问题已经由平台原生 UI 路径解决，不需要为了菜单/弹窗增加
一套“打开浮层前 `set_visible(false)`、关闭后恢复”的跨 host 显隐协调器：菜单
使用 NSMenu；模态弹窗使用 Settings 已验证的独立原生窗口/独立 GPU surface 方案，
天然位于主窗口 WebView 之上。其他旧弹窗向该方案迁移由独立工作负责，本重构只
消费统一后的菜单/弹窗基础设施，不重复实现。`set_visible` 仍用于 tab 激活、pane
显隐、休眠和销毁等正常生命周期，不承担弹窗遮挡职责。非 macOS 尚未发布，其
fallback 继续遵循 overlay-window 既有设计，不阻塞本次 macOS 重构。

Flyfish 现有 IPC（title 上报、⌘F 搜索注入）仍是零散约定；本设计要建立三种
webview host 共用的消息 envelope 和资源生命周期契约。Flyfish 与
`vanilla-jsoneditor` 迁移到同一 envelope 结构，各自保留独立命令/事件名。

第一版功能：语言高亮、行号、语法折叠、搜索/替换、括号匹配、选区、undo/redo、
简单输入和保存。禁用补全、LSP、lint、格式化、多光标和网络访问。

### 文档与 revision

已加载时 CodeMirror document 是编辑缓冲权威；磁盘是最后保存版本；休眠脏 tab
的 recovery snapshot 是恢复权威。所有变更递增 revision。Agent 的写命令必须
携带读取时 revision，不匹配即拒绝，不得覆盖用户后续输入。

IPC 只暴露具名命令，禁止执行任意 JS。这里定义的消息 envelope
（`protocol_version/project_id/tab_id/document_id/revision/request_id`）是
**所有 webview host 的通用契约**，不是 CodeMirror 专属——Flyfish 收敛
（工具收敛结论）和 `vanilla-jsoneditor` 接入都必须复用同一 envelope 结构，
各自只扩展自己的命令/事件名：

```text
Editor -> Rust: ready, selection_changed, document_changed,
                save_requested, focus_changed, viewport_changed, failed
Rust/Agent -> Editor: set_document, reveal_position, select_range,
                       replace_range, open_find, set_read_only, focus,
                       serialize_view_state
```

坐标协议对外统一为 1-based line/column；桥接层负责 CodeMirror offset、Unicode
列和必要的 UTF-8 byte offset 转换。

目标行位于折叠区时必须展开最小包含区域再 reveal/select。

## Agent 能力

统一 `PreviewContext`：

```rust
pub struct PreviewContext {
    pub project_id: i64,
    pub tab_id: usize,
    pub path: PathBuf,
    pub revision: u64,
    pub mode: PreviewMode,
    pub read_only: bool,
    pub cursor: Option<TextPosition>,
    pub selection: Option<TextRange>,
    pub selected_text: Option<String>,
    pub visible_lines: Option<LineRange>,
}
```

Agent 可请求 reveal/select；局部 replace 仅对可写 CodeMirror 且 revision 匹配时
允许。休眠 tab 被 Agent 操作时先进入加载队列；操作在 ready 后执行。上下文有
大小上限，不默认发送整文件。

现状核实：MCP 的 `get_preview_context` 查询的是 `dozerd`（session daemon）
持有的 `dozer_core::protocol::PreviewContext`（当前只有
`path/start_line/start_col/end_line/end_col/has_selection/updated_at_ms`），
不是直接查 `dozer-app`。这意味着上面的高频 selection/viewport 节流只覆盖了
"editor WebView → dozer-app"这一段；`dozer-app → dozerd`（经 UDS）的推送是
另一段独立链路，必须有自己的节流/合并策略，不能假设已经被前一段的节流覆盖。
扩展后的 `PreviewContext` 字段需要同步落到 `dozer-core::protocol`、
`dozerd`、`dozer-client`、`dozer-mcp` 四个 crate。

## 客户端能力估算

Dozer 每次启动只探测一次，生成不可变、应用级 `Arc<ClientCapabilities>`：

```rust
pub struct ClientCapabilities {
    pub hardware: HardwareCapabilities,
    pub budgets: ResourceBudgets,
    pub tier: CapabilityTier,
}
```

保留总内存、启动时可用内存、物理/逻辑 CPU；预算至少包括单 editor、全部 preview、
JSON Tree、整文件读取、重型 WebView 数和后台并行度。检测与纯估算函数分离，业务
模块不得重复调用 sysinfo。

初始公式：

```text
single_editor = clamp(min(total * 8%, available * 20%), 64MiB, 512MiB)
total_preview = clamp(min(total * 15%, available * 35%), 128MiB, 1.5GiB)
json_tree     = clamp(single_editor / 2, 32MiB, 256MiB)
```

公式是可测试默认值，不是永恒常量；性能基准可调整。高配机器仍受每种 viewer 的
绝对护栏限制。

## 文件画像与能力降级

`FileProfile` 包含字节数、采样行数、最大行长度、UTF-8 状态和类型。初始策略：

| 条件 | 策略 |
|---|---|
| 估算可编辑工作集在预算内，且文件 <=30MiB | 完整 CodeMirror |
| 可编辑超预算、只读高亮在预算内，且 <=64MiB | CodeMirror 只读高亮 |
| 高亮超预算、纯文本在预算内，且 <=128MiB | CodeMirror 只读纯文本 |
| 更大 | Rust 窗口化/流式 Viewer |

估算系数初值：可编辑 9x、只读高亮 6x、纯文本 4x、JSON Tree 15x。单行超过
100KiB 默认关闭换行；超过 1MiB 关闭高亮/折叠；超过 5MiB 强制窗口化。

大文件搜索由 Rust 流式执行，命中结果封顶并携带行列与摘要。后台建立稀疏行索引
（如每 1000 行一个 byte offset），支持 Agent 任意行跳转。窗口化 CodeMirror
仅持有目标附近内容，并显示全局行号基数。

## 多项目、恢复与资源管理

持久化从 `paths + active_path` 扩展为 tab descriptor：path、mode、cursor、selection、
scroll anchor、fold state（有界）、active、file revision。不得持久化 WebView、全文、
语法树或搜索结果。

启动顺序：恢复项目/tab 壳 -> 首屏可交互 -> 只加载当前项目当前文件。其他 tab
全部 `Suspended`；启动加载并发固定为 1，不后台预热大文件。

`PreviewResourceManager` 在所有项目间维护总预算、估算驻留量、重型 WebView 数、
last_accessed、dirty/active 状态。超预算按 LRU 依次休眠后台项目干净 tab、当前
项目非活动干净 tab；当前 tab、保存中、Agent 写入中和无 recovery 的脏 tab不可
淘汰。

脏 tab 定期原子写 recovery snapshot；正常保存删除 snapshot。只有 snapshot 成功
后才可休眠。磁盘外部变化：干净 tab 自动重载；脏 tab进入冲突状态，不静默覆盖。

连续恢复失败或上次启动未完成时，保留 tab 壳但不自动加载问题文件，提供纯文本
只读重试和外部打开，防止启动死循环。

## 工具收敛结论

- **老 iced text editor：删除。** CodeMirror 完成同等功能、Agent API、大文件
  fallback 和迁移验收后移除 `code_editor/` 及旧消息链。
- **Flyfish：保留并收窄。** 负责富媒体/文档渲染，不再作为未知文本兜底；文本
  统一 CodeMirror/Streamed。
- **Tabular：保留。** 它对 CSV/XLSX 的虚拟化网格价值明确；接入统一 backend、
  资源和 Agent 协议。CSV/TSV 增加原文模式。
- **JSON Tree：分阶段替换。** 普通严格 JSON 迁移到 `vanilla-jsoneditor` 后删除
  自研普通 JSON tree；JSONL/NDJSON 和超大 JSON 的流式能力保留并重构为
  Streamed JSON Viewer，直到替代品通过同等性能验收。
- **CodeMirror：新增并成为所有代码/文本的唯一正常后端。**
- **外部应用：新增正式 fallback。** 通过既有 external-app 配置或系统默认应用
  打开，路径必须来自当前 tab，UI 明示这是离开 Dozer 的操作。

## 安全与可靠性

- editor 资源本地打包，严格 CSP、无网络、无任意 JS 执行。
- JS 不能提交任意路径；所有读写绑定 project/tab/path 并由 Rust 校验。
- HTML 渲染与 editor 使用不同安全域/策略，禁止不可信 HTML 获得 editor IPC。
- 保存保持原 BOM/换行约定；非 UTF-8 默认只读；用同目录临时文件原子替换。
- macOS 菜单统一使用 NSMenu，模态弹窗复用 Settings 的独立原生窗口方案；禁止
  在主 iced surface 内新增会被 WebView 遮挡的菜单/弹窗，也不为它们切换 WebView
  `set_visible`。`set_visible` 只服务 tab/pane 生命周期。非 macOS fallback 复用
  overlay-window 既有抽象。
- “外部打开”不隐式执行文件本身；可执行文件只定位到文件或使用安全查看器。

## 可观测性与验收

启动日志记录能力原始值、预算和 tier；每次路由/降级记录 backend、reason、估算成本。
诊断页可显示当前驻留 viewer 和预算使用。

必须通过：

- 代码行号、滚动到底、折叠、搜索准确；中文 IME 正常。
- Agent 获取选区、跳转折叠内行、revision 冲突拒绝正确。
- 多项目恢复数十个大文件 tab 时只加载一个，首屏不随 tab 数线性变慢。
- 内存超预算会休眠正确 tab，脏内容不丢失。
- 大文件可搜索、跳转、复制；功能降级有解释。
- 普通 JSON、JSONL、CSV/XLSX、Markdown/HTML、图片/PDF、未知文本和未知二进制
  均走预期路由并有 fallback。
- 无网络环境下所有 editor/viewer 正常工作。
