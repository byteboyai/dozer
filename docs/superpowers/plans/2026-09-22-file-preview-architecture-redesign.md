# Dozer 文件预览架构重构 Implementation Plan

**Goal:** 以查看、检索和 Agent 操作为第一优先级，统一 Dozer 的 CodeMirror、
Flyfish、Tabular、JSON 与大文件预览；按设备能力和全局驻留预算降级，并安全恢复
多项目 tab。

**Spec:** `docs/superpowers/specs/2026-09-22-file-preview-architecture-redesign.md`

**Executable sub-plans（按顺序执行）：**

1. `2026-09-22-file-preview-foundation.md`
2. `2026-09-22-codemirror-agent-editor.md`
3. `2026-09-22-large-file-session-resource-management.md`
4. `2026-09-22-preview-viewer-consolidation.md`

本文件是覆盖范围和完成定义的 master roadmap；子计划负责逐步实施。任何子计划
完成都不代表总重构完成，最终仍以本文“完成定义”为准。

**原则:** 每个阶段必须保持可构建、可回退；旧实现只有在替代能力通过验收后才
删除。阶段拆分是为了控制风险，不是裁剪范围。后续实现若拆成多个独立 plan，
每份 plan 必须引用本总计划并回填本清单，不能遗漏后置阶段。

> 进度(2026-09-22):子计划 1 `file-preview-foundation.md`(Phase A)已完成，
> 已回填 Phase 1–2 完成项;Phase 0 的 spike/fixture 与 Phase 3+ 未开始。

## 全局约束

- 静态资源随应用打包，无 CDN、无运行时 Node.js。
- CodeMirror/JSON WebView 与不可信 HTML/Flyfish 分安全域和 IPC。
- 所有坐标边界显式转换；外部协议统一 1-based line/column。
- 所有 Agent 写操作带 revision；失配拒绝。
- 启动恢复只加载当前项目当前文件；其他 tab 为 Suspended。
- 大文件可以降级编辑/高亮/折叠，但不能丢弃查看、搜索和跳转。
- 任何旧工具的删除 task 必须列出功能对照测试和回退提交点。
- 每阶段运行相关测试；里程碑运行 `cargo test -p dozer-app`、
  `cargo clippy --all-targets`、`cargo fmt --check`，前端运行其单测和生产构建。

## Phase 0：基线、依赖与性能夹具

**产出：** 可重复比较迁移前后的行为/性能，不改用户路径。

- [ ] 盘点 `PreviewTab`、Files/Project 两个 pane、WebView pool、Find、保存、主题、
  tab 持久化和 MCP preview context 的所有调用点，形成代码内迁移检查表。
- [ ] 建立 fixture 生成器：普通代码、中文、CRLF/BOM、10/30/64/128/500MiB 文本、
  50 万短行、1/5MiB 单行、普通/深层/压缩 JSON、JSONL、宽 CSV。
- [ ] 记录当前首屏、打开、搜索、跳转、内存和关闭/切项目行为基线。
- [ ] 确认 CodeMirror 6、语言包、bundler、`vanilla-jsoneditor` 候选版本与许可证，
  lock 版本；本阶段只做离线 spike，不接生产路由。
- [ ] spike 验证 wry 中的 IME、剪贴板、快捷键、主题、selection 回传、Rust reveal、
  NSMenu、Settings 式独立原生弹窗层级和无网络启动；不把切换 WebView
  `set_visible` 当成菜单/弹窗方案。
- [ ] 写 go/no-go 记录；若 CodeMirror 在 wry 上有阻断问题，先解决而不是绕回旧
  iced editor。

## Phase 1：全局客户端能力

**建议文件：** `crates/dozer-app/src/capabilities.rs`，App/Workspace 构造代码。

- [x] 增加 `HardwareCapabilities`、`ResourceBudgets`、`CapabilityTier`、
  `ClientCapabilities`。
- [x] `detect_hardware()` 只负责 sysinfo；`estimate_capabilities()` 是纯函数。
- [x] Dozer 启动探测一次，以 `Arc<ClientCapabilities>` 注入 App/Workspace；禁止业务
  模块再次探测。
- [x] 实现单 editor、总 preview、JSON Tree、整读、重型 WebView、后台并行预算。
- [x] 加 4/8/16/32/64GiB、低 available、CPU 0/1、上下限和单调性测试。
- [x] 增加结构化启动日志和诊断读取接口(`capabilities::current()`)。

## Phase 2：统一路由与 backend 状态机

**建议文件：** `preview/router.rs`、`preview/backend.rs`、`preview/file_profile.rs`。

- [x] 定义 `PreviewBackend`、`BackendState`、`PreviewMode`、`RouteReason`、
  `FileProfile`；给状态机写转换测试。
- [x] 实现单一 `classify_preview(path, profile, capabilities, user_mode)`。
- [x] 覆盖源码、无扩展名文本、Markdown/HTML/SVG、JSON 家族、Tabular、富媒体、
  archive、未知 UTF-8/二进制的路由测试。
- [x] 让 `PreviewTab` 先增加统一 backend，同时保留旧字段作为迁移 adapter；所有
  新代码只读 backend。
- [x] 将 `desired_webviews`、native 判定、Find 能力、主题重载改读统一 backend；
  adapter 期间增加一致性断言(desired_webviews/active/主题重载已改读 backend;
  native/Find 仍读 adapter,Phase 4 收敛)。
- [x] 为 Failed/Unsupported 提供内部错误说明、重试、纯文本只读和外部打开动作
  (能力描述已提供,UI 接线留 Phase D)。

## Phase 3：CodeMirror host 与安全 IPC

**建议文件：** `assets/editor/`、`preview/code_host.rs`、wry pool/scheme handler。

- [ ] 建独立前端包，生产构建输出固定哈希或版本化静态资源到应用资源目录。
- [ ] 配置严格 CSP、禁网、禁任意脚本；Rust scheme handler 只服务白名单资源。
- [ ] 实现精简 CodeMirror：行号、高亮、折叠、搜索/替换、简单编辑、undo/redo、
  read-only、主题和 JetBrains Mono/CJK fallback。
- [ ] 建带 `protocol_version/project_id/tab_id/document_id/revision/request_id` 的消息
  envelope；解析失败不 panic。**设计为所有 webview host 通用的契约**（不是
  CodeMirror 专属），供 Phase 7 Flyfish 和 Phase 8 `vanilla-jsoneditor` 直接
  复用其结构，只扩展各自的命令/事件名。
- [ ] 接入 macOS 已验证的平台层级策略：上下文菜单走 NSMenu；需要覆盖 WebView 的
  模态弹窗复用 Settings 的独立原生窗口/独立 GPU surface。不得新增依赖
  `set_visible` 临时隐藏 WebView 的菜单/弹窗逻辑；其他弹窗的统一迁移由对应并行
  工作负责，本计划不重复实现。
- [ ] 实现 ready/set_document/change/selection/viewport/save/focus/error 与
  reveal/select/replace/open_find/set_read_only/serialize_view_state。
- [ ] 高频 selection/viewport 事件节流；正文变化发送增量，不逐键回传全文。
- [ ] 测试过期 tab、错误 revision、重复 ready、乱序响应、WebView 重建和主题切换。

## Phase 4：普通文本迁移与 Agent 操作

- [ ] 将普通源码/配置/文本路由到 CodeMirror，老 CodeView 保留 feature/fallback
  路径直至本阶段验收结束。
- [ ] 集中语言/文件名注册表；Rust 路由与前端语言加载由同一源生成。
- [ ] 实现 LF/CRLF、BOM、UTF-8 检测；非 UTF-8 只读并提供外部打开。
- [ ] 实现原子保存、dirty、undo/redo、外部文件变化和冲突状态。
- [ ] 统一 `PreviewContext`；迁移现有 MCP 选区/光标读取。字段扩展需要同步改
  `dozer-core::protocol`、`dozerd`、`dozer-client`、`dozer-mcp` 四个 crate；
  `dozer-app → dozerd`（经 UDS）的推送要单独设计节流/合并策略，不能假设复用
  editor WebView → dozer-app 那段节流。
- [ ] 实现 Agent reveal/select/replace；折叠目标自动展开，写入 revision 失配拒绝。
- [ ] ⌘F/⌘R 放入 editor WebView；⌘S 由 WebView 请求 Rust 保存；明确 Esc/Tab/
  剪贴板与 iced 全局快捷键边界。
- [ ] 验收精确行号、拖动到底、折叠、搜索、中文 IME、字符编辑、Agent 双向操作。

## Phase 5：动态大文件与流式搜索

- [ ] 用 `FileProfile + ClientCapabilities` 实现 Editable、ReadOnlyHighlighted、
  ReadOnlyPlain、Windowed 四档纯决策函数。
- [ ] 落实工作集系数、绝对护栏、超长单行规则；UI 展示降级原因和可选的单次
  强制尝试。
- [ ] 普通/只读整载 CodeMirror 性能基准覆盖阈值边界。
- [ ] 实现 Rust 流式全文搜索，限制命中条数但保留总数/截断说明。
- [ ] 实现稀疏行索引、目标附近窗口读取、全局行号基数和窗口切换。
- [ ] Agent 对未加载行 reveal 时通过索引加载窗口再选择；不得把整文件塞入上下文。
- [ ] 删除/替换旧 `native_editor.rs` 的独立大文件分档，确保全局策略是唯一真相源。

## Phase 6：会话恢复、recovery 与全局资源管理

- [ ] 扩展 `preview_state` schema：tab descriptors、active、mode、cursor、selection、
  scroll anchor、有限 fold state、file revision；旧 `paths/active_path` 可向后读取。
- [ ] 启动恢复 tab 壳为 Suspended，首屏后只排队当前项目当前文件；并发恒为 1。
- [ ] 实现 `PreviewResourceManager`：跨项目预算、估算驻留、重型 WebView 数、LRU、
  pin 原因和诊断快照。
- [ ] 超预算先休眠后台项目干净 tab，再休眠当前项目非活动干净 tab；切换时懒加载。
- [ ] 实现 dirty recovery snapshot 的节流、原子写、启动恢复、保存后清理和磁盘冲突。
- [ ] 无 recovery 的 dirty、活动、保存中、Agent 写入中 tab 不可淘汰。
- [ ] 增加恢复失败计数和安全启动；问题文件保持 tab 壳并提供纯文本/外部打开。
- [ ] 测试同项目/多项目数十个大文件，总驻留受预算约束且启动只加载一个。

## Phase 7：Flyfish、HTML 与外部应用收敛

- [ ] Flyfish 收窄为富媒体/文档 renderer，未知文本不再进入 Flyfish。
- [ ] Markdown、HTML、SVG 建 Rendered/Source mode；按 tab 持久化用户选择。
- [ ] Agent 请求文本行操作时唤醒/切到 source，完成后不强制改用户持久偏好。
- [ ] 审计直接 `file://` HTML；迁移到隔离 host，禁止不可信页面获得 editor IPC。
- [ ] 统一 Flyfish 与 editor WebView 的池化、预算、tab/pane 显隐、主题和销毁规则；
  菜单/弹窗层级继续由 NSMenu 和独立原生窗口解决，不进入 WebView 显隐规则；
  Flyfish 既有的零散 IPC（title 上报、⌘F 搜索注入等）迁移到 Phase 3 定义的
  通用消息 envelope，不保留与 CodeMirror host 并存的第二套约定。
- [ ] 增加“使用系统默认应用打开”和用户配置外部应用；路径绑定当前 tab并校验。
- [ ] 对无法内嵌、损坏、加密或超预算格式始终给出可执行 fallback，而不是空白页。

## Phase 8：JSON 工具收敛

- [ ] 先让普通 JSON Text 使用 CodeMirror JSON language；JSONC/JSON5 永久至少有
  CodeMirror fallback。
- [ ] spike `vanilla-jsoneditor` 的离线 bundle、主题、Tree/Text、path select/
  scroll/collapse、revision、只读、内存和 WebView IPC；IPC 复用 Phase 3 的
  通用消息 envelope 结构，只新增 JSON 专属命令/事件，不新起一套 envelope。
- [ ] 普通严格 JSON 在预算内默认 Tree，Text 为 CodeMirror；Agent 优先使用 JSON
  path，文本模式仍提供行列。
- [ ] JSON Tree 预算不足、解析失败、深度/节点数超限时降级 Text/Streamed。
- [ ] JSONL/NDJSON 和超大 JSON 迁移为统一 Streamed JSON Viewer，保留现有虚拟化/
  够数即停能力及 Rust 搜索。
- [ ] 功能/性能对照通过后删除自研普通 JSON tree；不得删除 streamed JSON 能力。
- [ ] 更新旧 JSON spec 状态，记录哪些结论被替换、哪些算法被保留。

## Phase 9：Tabular 接入统一架构

- [ ] 将现有 CSV/TSV/XLSX viewer 包装为 `PreviewBackend::Tabular`，接入 Suspended/
  Loading/Ready/Failed 和资源预算。
- [ ] CSV/TSV 增加原文 CodeMirror mode；XLSX 保持专用 viewer。
- [ ] 定义 Agent 的当前 sheet、可见行列、选中单元格和 reveal cell 上下文。
- [ ] 大表继续虚拟化、后台加载和封顶；不得为统一而退化为 DOM 全量表格。
- [ ] 接入会话恢复和 LRU；重载时恢复 sheet 与可见锚点。

## Phase 10：旧实现删除与最终验收

- [ ] 建工具能力矩阵逐项确认 CodeMirror/Rendered/JSON/Tabular/Streamed/External
  已覆盖老路径。
- [ ] 删除 `crates/dozer-app/src/code_editor/`、老 iced editor 消息、Find/undo/
  scrollstrip、高亮器和 native load 类型。
- [ ] 清理 `PreviewTab` 旧 `editor/tabular/json_tree/loading/truncated` 组合字段及
  adapter、一致性断言和死分支。
- [ ] 删除被 `vanilla-jsoneditor` 替代的普通 JSON tree 代码，只保留明确命名的
  streamed JSON 代码。
- [ ] 更新架构注释、CLAUDE.md 关键约束、打包脚本、许可证清单和用户文档。
- [ ] 全量自动测试、前端测试、离线安装包 smoke test、深浅主题、缩放、IME、
  外部修改、崩溃恢复和多项目压力测试。
- [ ] 手工验收路由矩阵中每一类文件，以及“无法查看 -> 外部打开”的最后兜底。

## 完成定义

只有以下全部成立才算本重构完成：

1. 老 iced editor 和重复普通 JSON tree 已安全退役，而不是仅新增 CodeMirror。
2. 所有文件类型都有内部 viewer、可解释降级或外部打开路径。
3. 查看、搜索、Agent context/reveal/select 在支持的后端上一致。
4. 单文件和跨项目累计资源都受全局预算管理。
5. 启动恢复不会随历史大文件 tab 数量线性加载。
6. 脏内容、外部变更和 revision 冲突不会静默丢数据。
7. 大文件即使降级也能打开、搜索和跳转。
