# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目现实（2026-07-15 起）

本仓库是 **Dozer** —— 站在用户（甲方）一侧的、agent 中立的 AI 治理与验收层（macOS 先发，Rust workspace）。
权威文档：

- 规格（唯一需求真相源）：`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`
- 实现计划系列：`docs/superpowers/plans/2026-07-15-dozer-p1a-*.md` 起
- UI 设计：Figma "Dozer Phase 1 UI"（12 帧）；参考截图 `design/参考/`

## Workspace 布局

| crate | 职责 |
|-------|------|
| `crates/dozer-core` | 共享类型、路径、UDS 协议 |
| `crates/dozerd` | session daemon：PTY 池、会话存活（bin: `dozerd`） |
| `crates/dozer-app` | iced 0.14 GUI（bin: `dozer`） |
| `crates/dozer-hook` | 被 agent hooks 调用的零依赖小二进制（bin: `dozer-hook`） |
| `crates/dozer-client` | dozer-app/dozer-mcp 共用的 UDS 客户端库（`Client`） |
| `crates/dozer-mcp` | 面向外部 CLI agent 的只读 MCP stdio server（bin: `dozer-mcp`） |
| `spike/*` | 一次性技术验证，随时可删 |

## 构建与测试

```bash
cargo build                    # 全 workspace
cargo test -p dozerd           # 单 crate 测试
cargo run -p dozer-app         # 跑 GUI
cargo clippy --all-targets && cargo fmt
```

## 关键裁决（违反即错）

- **核心原则：Dozer 方便用户预览 AI 的工作结果,用户应尽可能通过 AI 修改产物,而不是自己直接改产物。** 预览类功能(文件预览、Preview WebView 等)优先做"渲染/查看"而非"编辑";如果确实需要提供直接编辑入口,要能说明为什么这个场景绕不开用户亲自动手,不能默认给。
- boy CLI 已废弃，永不回归；agent 启动/模型托管/doctor 全归 dozerd。`crates/legacy-boy` 已删除（2026-08-12）——删除时 dozerd 尚未实际迁入 `config`/`process`/`doctor` 这三块，旧实现只留在 git 历史（删除前的最后一次提交）里，之后要做这几块时得从那份历史重新参考，不是已经迁完。
- GUI 只用 iced 0.14 生态；预览 WebView 走 wry 子视图叠加，webview 恒在 GPU 内容之上——凡是需要盖住它的原生浮层（如 Usage 面板全屏态）都要显式隐藏 webview，不能指望层级自然遮挡。**⌘K 命令面板未实现**（规格 §3 已裁掉一期范围，顶栏曾有的纯视觉占位搜索框已于后续迭代移除，见 `app.rs` 中该处的移除说明注释）；「⌘K 打开时隐藏预览」这条早期设计陈述作废，不要再引用它。
- mac 先发但架构留门：不引入 Swift/AppKit 专属能力；核心不依赖 Node/Python。
- 一期范围以规格 §3"一期范围裁剪"为准；显式未决项（规格 §8）不得擅自定死。
- 主题 ByteBoy2077：bg `#0a0e16`、金 `#F2D94E`（甲方动作专属）、奶油文字 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。
- 新增/改造 icon 按钮、tab 类 UI 时优先复用统一组件（`icons::icon_button_entry`/`tabs::tab_core`），不要重新手写一套 `MouseArea`+`on_enter`/`on_exit` 接线；确需自定义（形状/交互模式明显不同）要在 plan 里说明理由，不是绝对禁止。
- 新增/改造函数参数 ≥7 个、且有多个同类型参数相邻（顺序传错编译器发现不了，如连续几个 `&str`/`bool`/同一消息类型）时，优先用具名字段的参数结构体替代位置参数（Rust Design Patterns: Builder），不要无脑加 `#[allow(clippy::too_many_arguments)]` 了事；结构体带闭包字段时用结构体自身的泛型参数承载（不要 `Box<dyn Fn>`），保持零成本。参数虽多但天然同质、不易传错的情况（如四个方向 padding、RGBA 四值）不受此约束。
- **字体统一：只有 code editor 和 pty 终端用 JetBrains Mono（`assets::fonts::code_font()`），其余所有场景（UI 文本、表格预览、面板等）一律用系统默认字体（`Font::default()`），不得给非代码/终端场景上等宽代码字体。** 非 ASCII 文本（如中文）的正确渲染靠 `Shaping::Advanced`（做字体回退到系统 CJK 字体），与主字体无关——`Shaping::Basic` 明确不做回退，会让中文变方块/空白，任何用 canvas `fill_text` 或自绘文本的地方都不能用 `Basic`。
- **文件预览（File Preview）路由与查看器**（见 `docs/superpowers/plans/2026-09-22-file-preview-wrap-up.md`）：
  - CodeMirror 与 vanilla-jsoneditor 常开；老 iced `CodeView`、自研普通 JSON 树与 `syntect` 已删除，永不回归。
  - `preview/router.rs::classify_preview` 是唯一路由决策点。文件名注册表（`Dockerfile`/`Makefile`/`LICENSE*`/`.env`/`.gitignore`…）优先于扩展名，但仍受内容安全检查约束；未知 UTF-8 文本进 Code，未知二进制落 `Unsupported` fallback，空文件按可编辑纯文本。
  - JSON 家族统一 `PreviewKind::Json`：严格 `.json` 走 vanilla-jsoneditor Tree/Text 双视图，json5/jsonc/jsonl/ndjson 只给 CodeMirror 文本（无树）。
  - **非 UTF-8 / UTF-16 / 二进制**：只读展示，保存恒拒绝（`PreviewTab::can_save` / `save_gate`）；`encoding_lossy` 文件顶部有只读提示（`lossy=1`）。非法编码绝不允许经 `fetch().text()` 解码后回写原文件。
  - **External/Unsupported 不 host webview**，由 `workspace/view.rs::preview_fallback_page` 统一 fallback 页承载（类型/路径/原因 + 重试/纯文本只读/外部打开，动作由 `preview::fallback_actions` 生成）。
  - **大文件（windowed）**：窗口正文封顶 `WINDOW_MAX_BYTES`（`read_window_capped` 用 `take(max+1)`）；稀疏索引/流式搜索按固定块分段、段间重叠，超长单行**不得整行分配**（禁止 `BufRead::split`/`read_until` 整行）；`SetWindow` 派发判据是 `uses_editor_host()`，不是 `uses_codemirror()`；外部变更会失效旧索引（`apply_window_index` 校验 revision）。
  - **WebView 恒在 iced 之上**：预览内任何 iced 条（Find 条、窗口化搜索条、T10 冲突条）都必须由 `App::preview_desired` 显式把 webview 矩形下推条高，否则会被原生子视图盖住。
  - **T10 磁盘冲突**：脏 tab 遇外部修改进入显式冲突态（保留我的修改 / 重载磁盘·二次确认）；保存前 `save_gate` 再校验磁盘 mtime，避免提示后又变被静默覆盖。
