# 文件预览重构 Phase D：Viewer 收敛与旧实现退役 Implementation Plan

**Goal:** 将 Flyfish、JSON、Tabular、外部应用接入统一 backend/resource/Agent 协议，
在功能与性能对照通过后删除老 iced editor 和重复普通 JSON Tree。

**Depends on:** Phase A–C。

## Task 1：Flyfish 与 Rendered/Source mode

- [ ] Flyfish 路由收窄为图片/PDF/Office/媒体/Markdown 等真正渲染格式；未知 UTF-8
  文本进入 CodeMirror/Windowed，不再用 Flyfish 文本兜底。
- [ ] Markdown、HTML、SVG 作为 `RenderedBackend`，包含可选 CodeMirror source。
- [ ] 每 tab 持久化用户 mode；Agent 行操作临时唤醒 source，不永久改偏好。
- [ ] HTML 从直接任意 `file://` 审计/迁移到隔离 host，禁止获得 editor IPC。
- [ ] Flyfish title/search 等 IPC 迁移到通用 envelope，命令域保持隔离。
- [ ] Flyfish 接入 resource manager 的 reserve/suspend/destroy 和 Failed fallback。

## Task 2：平台菜单、弹窗与外部打开

- [ ] editor/rendered/json WebView 的上下文菜单在 macOS 统一走 NSMenu。
- [ ] 需要覆盖 WebView 的弹窗复用 Settings 独立原生窗口；不通过临时
  `set_visible(false)` 解决。
- [ ] 消费其他工作完成的旧弹窗迁移，不在本计划重复改造全应用弹窗。
- [ ] 增加“使用系统默认应用打开”和已配置外部应用入口。
- [ ] 路径来自当前 tab 且再次校验存在/权限；可执行文件默认在 Finder 定位或安全
  查看，不隐式执行。
- [ ] Unsupported/损坏/加密/超预算格式都有可执行 fallback。

## Task 3：`vanilla-jsoneditor` spike 与接入

- [ ] 固定版本/许可证，构建离线 bundle；使用独立 `dozer://json-editor/` CSP。
- [ ] 验证 Tree/Text、搜索、format/repair、read-only、path select/scroll/collapse、
  主题、512MB 宣称在 Dozer WebView 中的真实边界。
- [ ] IPC 复用通用 envelope，增加 JSON path selection/context/reveal/replace。
- [ ] 严格 JSON 在预算内默认 Tree；Text 使用 CodeMirror；两种 mode 共用 revision。
- [ ] JSON Tree 预算、深度、节点数、解析错误触发 Text/Streamed 降级。
- [ ] JSONC/JSON5 至少保留 CodeMirror，不强迫转成严格 JSON 再保存。

## Task 4：Streamed JSON

- [ ] 从现有 `json_tree` 抽取 JSONL/NDJSON、超大 JSON 的虚拟化/够数即停能力。
- [ ] 接入统一 Streamed backend、Rust 搜索、稀疏定位和 Agent 上下文。
- [ ] JSONL 每行独立 root，坏行局部报错，不让整文件打不开。
- [ ] 超大单行 JSON 不全量建 DOM；提供文本窗口和可解释限制。
- [ ] 与现有 1GB fixture 做首屏、内存、展开/搜索对照。

## Task 5：删除自研普通 JSON Tree

- [ ] 建功能矩阵：Tree 展开、类型、搜索、模式切换、主题、Agent path、性能、错误
  fallback 全部由新 JSON/Streamed 覆盖。
- [ ] 将普通 `.json` 路由切到新 backend，观察期内保留 feature fallback。
- [ ] 通过验收后删除只服务普通 JSON DOM/tree 的代码；保留并重命名 streamed 部分。
- [ ] 更新旧 JSON spec 状态和依赖，删除不再使用的 parser/dependency。

## Task 6：Tabular 统一

- [ ] 现有 Tabular 包装为 backend state，接入 resource manager、Failed、Suspend。
- [ ] CSV/TSV 增加 CodeMirror 原文 mode；XLSX 保持专用 grid。
- [ ] Agent context 包含 sheet、可见行列、选中单元格；支持 reveal cell/range。
- [ ] 保持虚拟化、后台加载、行列封顶，不为统一改成 DOM 全量表格。
- [ ] 持久化 sheet 与逻辑 scroll anchor，恢复时不预加载后台项目表格。

## Task 7：删除老 iced CodeView

- [ ] 对照功能：文本显示、高亮、搜索/替换、保存、undo/redo、主题、IME、MCP context、
  大文件、find keyboard routing 全由 Phase B/C 覆盖。
- [ ] 删除 `crates/dozer-app/src/code_editor/` 和 `preview/native_editor.rs` 中旧构造路径。
- [ ] 删除 PreviewEditorEvent/Tab/Undo/Redo/Find 等仅服务 iced editor 的消息和窗口事件。
- [ ] 删除自绘 Scrollstrip、syntect editor highlighter、整文本 snapshot undo 依赖。
- [ ] 删除 PreviewTab 旧 adapter 字段与一致性断言；backend 成为唯一真相源。
- [ ] Cargo 清除只为旧 editor 存在的直接依赖；不能误删其他模块仍使用的依赖。

## Task 8：最终路由与全量验收

- [ ] 自动化覆盖规格路由矩阵每一项和大小/能力组合。
- [ ] 手工检查代码、Markdown/HTML/SVG、JSON/JSONC/JSON5/JSONL、CSV/XLSX、图片、
  PDF、Office、archive、未知文本、未知二进制。
- [ ] 检查 Files/Project 两 pane、多个项目、主题、缩放、NSMenu、Settings/其他已迁移
  弹窗、外部应用打开。
- [ ] 离线 macOS 安装包 smoke test，确认所有 bundle/字体/许可证被打包。
- [ ] 更新 CLAUDE.md、架构文档和用户帮助；旧 spec 保留历史但标明取代范围。
- [ ] cargo 全 workspace build/test/clippy/fmt + 前端 test/build 全绿。

## Phase D/总重构完成门槛

- [ ] 老 iced editor 与重复普通 JSON Tree 已删除，不是旁路闲置。
- [ ] Flyfish、CodeMirror、JSON、Tabular、Streamed、External 各自边界唯一明确。
- [ ] 所有文件都有内部查看、明确降级或外部打开。
- [ ] 查看、搜索、Agent 操作、资源预算、恢复和脏数据保护满足总规格。
- [ ] 回填 master plan Phase 7–10 和“完成定义”全部条目。

