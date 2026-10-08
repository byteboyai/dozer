# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目现实（2026-07-15 起，定位于 2026-09-24 修订）

本仓库是 **Dozer** —— 站在用户（甲方）一侧、agent 中立的 **vibe coding 全流程治理与验收层**（macOS 先发，Rust workspace）——面向更懂技术的委托人，覆盖从任务编排、多 agent 并行执行、上下文共享，到过程审计、结果验收的完整闭环。治理与验收始终是核心身份，不因覆盖全流程而降级为众多能力之一；范围从"仅验收最终产物"扩大到"全流程治理"是本次修订的实质变化，agent 中立性不变。具体落点（是否/如何插件化、Workflow Kernel 等数据模型改造）尚未立项，见 `docs/dozer-v2/dozer-v2架构分析.md`（初步分析，非已批准规格）。
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
| `bytehost`（独立仓库） | 应用宿主平台库,已拆到 `github.com/byteboyai/bytehost`,dozer 按 tag `v0.1.0` 消费(`bytehost-apps`/`bytehost-client`/`bytehost-webview`/`bytehost-panel`)。**设计/实现细节与平台规则以该仓库的 README 与 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` 为准**(应用模型/安装计划/生命周期/来源、进程型应用监管、受管运行时、origin gateway、界面框架无关的面板状态机、webview 安全策略)。**dozer 作为消费方必须遵守的边界:** `crates/bytehost-*` 已不在本仓,不要再往 dozer 里放应用宿主逻辑;GUI↔dozerd 的线上类型 `AppRequest`/`AppReply` 仍在 `bytehost-apps::proto`(`AppRequest::App`/`AppReply::App` 由 dozerd 的 `app_service.rs` 承接),**wire 形状只追加不修改**;失败带类别(`AppErrorKind`,GUI 经 `bytehost-client` 的 `AppApiError::kind()` 取回:`Host(AppFailure)` 带类别,`Transport` 无类别);应用面板挂在 `extensions/app_host.rs`(纯状态机 + `App::run_app_host_effects`),安装/卸载/停止界面在设置「应用」页(`extensions/settings_apps.rs` 现为 `bytehost-panel::install` 的薄壳 + `settings_apps_view.rs` 的 iced 视图);**应用适配宿主的严格 CSP,不放宽 CSP**;应用页面数据在 WKWebsiteDataStore 里,"含数据"卸载要清它(`app_webview::StoreRemovals`,策略实现来自 `bytehost-webview`);**导航策略与每应用存储标识只有一份**(`bytehost-webview` 的 `AppOrigin::allows_navigation` / `data_store_identifier`,dozer-app 不得再抄,`app_webview.rs` 有门禁测试);进程型应用的出站网络强制等级仍是 `Advisory`;Excalidraw 打包配方与 V2 自动验证在 `spike/v2-excalidraw`(配 `bytehost` 仓库的 `scripts/excalidraw/`)。历史 A6x 验收报告保留在本仓 `docs/superpowers/specs/*bytehost-a*-acceptance-report.md`。 |
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
- GUI 只用 iced 0.14 生态；预览 WebView 走 wry 子视图叠加，webview 恒在 GPU 内容之上。旧模式（浮层仍在主窗口内以 iced `stack!` 实现，如 tab 溢出下拉、Files/Project 右键菜单、输入框右键菜单）不能指望层级自然遮挡，必须由 `App::preview_desired` 按各自展开状态显式把 webview 矩形隐藏/下推。**新增浮层不要再走这条老路**：`search_modal`/`file_history` 已于 2026-09-18 迁移为独立原生子窗口（见 `platform/overlay_window.rs` 共享机制），作为独立 OS 窗口天然叠在 webview 之上，不需要任何显式隐藏逻辑——新的浮层/弹窗默认套这套独立窗口机制，只有明确说明理由时才退回旧的"iced 内浮层 + 显式隐藏 webview"模式。**⌘K 命令面板未实现**（规格 §3 已裁掉一期范围，顶栏曾有的纯视觉占位搜索框已于后续迭代移除，见 `app.rs` 中该处的移除说明注释）；「⌘K 打开时隐藏预览」这条早期设计陈述作废，不要再引用它。
- mac 先发但架构留门：不引入 Swift/AppKit 专属能力；核心不依赖 Node/Python。
- 一期范围以规格 §3"一期范围裁剪"为准；显式未决项（规格 §8）不得擅自定死。
- 主题 ByteBoy2077：bg `#0a0e16`、金 `#F2D94E`（甲方动作专属）、奶油文字 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。
- 新增/改造 icon 按钮、tab 类 UI 时优先复用统一组件（`icons::icon_button_entry`/`tabs::tab_core`），不要重新手写一套 `MouseArea`+`on_enter`/`on_exit` 接线；确需自定义（形状/交互模式明显不同）要在 plan 里说明理由，不是绝对禁止。
- **byteui 已迁到独立仓库 `byteboyai/byteui`（2026-10-01 起）**，dozer 与 digger 共用，通过 `byteui = { git = "...", tag = "vX.Y.Z" }` 引用，一律用 tag。改组件要去 byteui 仓库改并发版；本地联调在 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/byteui"] byteui = { path = "../byteui" }`，不提交。历史 plan/spec 里的 `crates/byteui/...` 路径指的是拆分前的位置。iced 版本必须与 byteui 一致，升级时一起升。设计见 `docs/superpowers/specs/2026-10-01-byteui-standalone-repo-design.md`。
- **bytegit 是独立仓库 `byteboyai/bytegit`（2026-10-02 起）**，dozer 与 digger 共用；消费方只引用已发布 tag，本地联调通过不提交的 `.cargo/config.toml` patch 到 `../bytegit`。Git 底层能力在 bytegit 增加并发版，dozer 只保留产品语义与呈现，不新增重复的 `git2`/git CLI 封装。迁移期旧实现按纵向切片逐步删除，见 `docs/superpowers/specs/2026-10-02-bytegit-design.md`。**新增的 git 读取一律调 `bytegit::Repo`**，不要再直接写 `Command::new("git")` 或 `git2::`。已迁移（P1，`v0.2.0`）：`delivery.rs` 的 `repo_root/is_dirty/file_statuses/branch/remote_url/local_branches/current_branch_has_commits`（保留为 bytegit 适配层，签名不变，P6 移除）、`git_hotspots` 的 `dirty_paths/head_short_sha`（非 ASCII 文件名现在返回真实 UTF-8，不再是 git 八进制转义）。**已迁移（P2，`v0.3.0`）**：`git_log`/`file_history` 的历史与 diff——`log`/`commit_files`/`previous_version`/`workdir_patch`/`file_at`/`file_bytes_at`/`blob_text` 全走 bytegit；`git_log` 的提交图布局仍用 `gleisbau`（规格 §1 非目标），只在它产出提交 id 的边界把 `git2::Oid` 转成 `CommitId`。**`git_log`/`file_history` 共用的 diff 内容类型 `DiffBlobContent` 与双侧读取（`workdir_content`/`blob_pair_content`）在 `extensions/diff_content.rs`，面板不得再互相 import。** **已迁移（P3，`v0.4.0`）**：`dozerd/projects.rs` 的项目更新时间（`git_repo_root`/`git_head_commit_ms` → `bytegit::Repo::discover` + `head_commit_time`）、`usage` 的提交计数（`commit_count`/`commit_count_by_day`）、`git_hotspots::recent_churn`（`churn`，路径相对仓库根，适配层再转相对项目根）。`dozerd` 现在也依赖 `bytegit`（同样用 tag，联调用不提交的 `[patch]`）。**已迁移（P4，`v0.5.0`）**：项目工作区监听改走 `bytegit::watch`（feature `watch`）——路径分类、`.git` 引用文件识别、debounce 都在 bytegit，忽略名单由 dozer 传 `project::HIDDEN`；`dozer-app` 不再直接依赖 `notify`，`git_watch.rs` 已删除。**`.git` 是文件的仓库（linked worktree / 子模块）的引用变化目前收不到，见规格 O3。** **已迁移（P5，`v0.6.0`）**：`delivery.rs` 的写操作 `init_repo`/`clone_repo`/`checkout_branch`/`git_available` 改成调 `bytegit::init`/`bytegit::clone`/`Repo::checkout_branch`/`bytegit::git_available`（适配函数签名不变，调用点不动，P6 移除）。**bytegit 内部为 `clone`/`checkout_branch` 仍调用 `git` 可执行文件是评估后的有意选择**（本构建 libgit2 没开 https/ssh 传输、不执行 `post-checkout` hook 与外部过滤器/Git LFS、冲突报错不点名文件；见 `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`），不要"顺手"换成 `git2`——换之前先让 `bytegit/src/write.rs` 里的钉死测试过。`checkout_branch` 走 `Repo::discover`（向上查找，子目录里切整个仓库），与 P1/P2 的 `open_exact` 相反是有意的。**已迁移（P6，`v0.7.0`，迁移完成）**：`delivery.rs` 已删除；全部生产调用点直接调 `bytegit`，`dozer-app` 不再直接依赖 `git2`（`gleisbau` 与 `bytegit` 都要求 `git2 = "0.21"` 且无默认 feature，cargo 统一成同一份——升级 `git2` 时两处一起看）。生产代码里直接调命令行 `git` 的只剩 `bytegit` 内部的 `clone`/`checkout_branch`（评估后的有意选择，见 `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`）。**按语义选构造函数**：`Repo::open_exact`（项目路径必须就是仓库根才认）、`Repo::discover`（向上查找）、`Repo::discover_workdir`（参数必须是目录、非 bare、向上查找）；各调用点用哪个见规格 §8 O9 的口径清单——**同一个"项目在仓库子目录里"的场景，不同面板口径不同是迁移前就有的，未统一（待用户裁决，见 D14）**，新代码不要想当然选一个。文件树的改动/新增/未跟踪/忽略着色语义在 `extensions/files/git_status.rs`（是 Dozer 自己的呈现语义，不是 git 查询）。
- 新增/改造函数参数 ≥7 个、且有多个同类型参数相邻（顺序传错编译器发现不了，如连续几个 `&str`/`bool`/同一消息类型）时，优先用具名字段的参数结构体替代位置参数（Rust Design Patterns: Builder），不要无脑加 `#[allow(clippy::too_many_arguments)]` 了事；结构体带闭包字段时用结构体自身的泛型参数承载（不要 `Box<dyn Fn>`），保持零成本。参数虽多但天然同质、不易传错的情况（如四个方向 padding、RGBA 四值）不受此约束。
- **字体统一：只有 code editor 和 pty 终端用 JetBrains Mono（`assets::fonts::code_font()`），其余所有场景（UI 文本、表格预览、面板等）一律用系统默认字体（`Font::default()`），不得给非代码/终端场景上等宽代码字体。** 非 ASCII 文本（如中文）的正确渲染靠 `Shaping::Advanced`（做字体回退到系统 CJK 字体），与主字体无关——`Shaping::Basic` 明确不做回退，会让中文变方块/空白，任何用 canvas `fill_text` 或自绘文本的地方都不能用 `Basic`。
- **瞬时消息统一走 Toast**（`extensions/toast.rs` + `platform/toast_overlay.rs`，见 `docs/superpowers/specs/2026-09-30-unified-toast-design.md`）：描述"刚刚发生了一件事"的一次性提示（失败/成功/警告）一律 `App::push_toast(scope, level, text)`/`push_toast_keyed`(第一个参数是调用方的日志来源 `Scope`;每条 Toast 会自动写一条日志)，不要再给某个面板加 `Option<String>` 的 notice/error 字段 + 自己画一条。描述"当前处于某状态"的持久状态（如"dozerd 不可用"、git 状态加载失败，带重试上下文）不进 Toast，留在原位。**dozerd 不可用**是 `App.daemon_unavailable`（只有三种写入：启动连不上、设置里停止、打开项目成功即清除），由顶栏徽标 `topbar::daemon_badge` 统一展示（首页/空态/工作区共用，不占布局）；"打开项目失败""新建会话失败"这类一次性事件不要写它，走 Toast。Toast 窗口是点击穿透、不聚焦的独立原生子窗口——**不要**给它加悬停/点击交互（macOS 上会让它成为 key window 抢终端焦点），要交互先重新评估焦点方案。extension 不得直接持有 `ToastCenter`，只能经内核:extension 的 `update` 拿不到 `App`,失败时往自己 state 的 `outbox`(`toast::Outbox`)里 `push`,`App::update` 的包装函数每次处理完消息后统一排空成 Toast;后台任务里则经 `proxy` 发 `Message::Toast`。文件树的文件操作失败/被拒走这条路径,唯独"移动对话框内的校验错误"留在对话框内联(`files::WorkspaceState::move_error`)。
- **日志统一走 `dozer_core::log`**（见 `docs/superpowers/specs/2026-09-30-unified-logging-design.md`）：`dozerd`/`dozer-app` 里禁止裸 `tracing::warn!` 等与 `eprintln!`（`scripts/check-log-scope.sh` 门禁；给命令行用户看的输出在行尾加 `// cli-output` 放行），一律 `dozer_core::log_*!(LOG, ...)`，`LOG` 由文件顶部 `dozer_core::scope!(LOG, panel|module, "<名>")` 声明，日志行里的 target 就是来源（`dozer::panel::files`），可用 `RUST_LOG=info,dozer::panel::files=debug` 按面板过滤。**面板日志的来源名必须是 `PanelKind` 对应的面板名**（`files`/`git_log`/`todo`/`project`/`database`/`ssh`/`web`/`agent`/`conversations`/`usage`/`code_health`/`group_chat`，`app/state.rs` 的测试会扫源码强制）；非面板代码用 `module` 来源（`shell`/`platform`/`preview`/`runtime`/`term`/弹窗类 extension 名）。共享代码里"替哪个面板工作"作为普通字段带上（如 `panel = ?panel`）。面板第一次写日志时才声明 `LOG`，不要预先声明未使用的来源。**不要用 clippy `disallowed_macros` 做门禁**：它会把 `log_warn!` 等包装宏的每个调用点都报成违规（spike 已验证）。`tracing` 的 `target:` 必须是常量，所以运行时才知道的来源（如 Toast 的 `Scope` 形参）不能当 target，放进字段 `scope`。`dozer-hook`/`dozer-mcp` 不引入 `tracing`：运行期诊断用 `dozer_core::plain_*!`，给命令行用户看的 `install`/用法输出仍是 `eprintln!`。**例外:bytehost 平台库(`bytehost-*`)是独立仓库,用裸 `tracing`,来源是 `bytehost::service` 等而非 `dozer::module::apps`;`check-log-scope.sh` 不覆盖它。** 日志文件在 `logs_dir()`，按天滚动、保留 14 天、不设大小上限。**不要把终端输入字节、口令等敏感内容写进日志**（GUI 日志现在落盘）。
- **文件预览（File Preview）路由与查看器**（见 `docs/superpowers/plans/2026-09-22-file-preview-wrap-up.md`）：
  - CodeMirror 与 vanilla-jsoneditor 常开；老 iced `CodeView`、自研普通 JSON 树与 `syntect` 已删除，永不回归。
  - `preview/router.rs::classify_preview` 是唯一路由决策点。文件名注册表（`Dockerfile`/`Makefile`/`LICENSE*`/`.env`/`.gitignore`…）优先于扩展名，但仍受内容安全检查约束；未知 UTF-8 文本进 Code，未知二进制落 `Unsupported` fallback，空文件按可编辑纯文本。
  - JSON 家族：严格 `.json` 走 vanilla-jsoneditor Tree/Text 双视图，json5/jsonc 只给 CodeMirror 文本（无树）；JSONL/NDJSON 走 `PreviewKind::Streamed`（T8，复用窗口化有界行视图，可切「原文文本」）。
  - **非 UTF-8 / UTF-16 / 二进制**：只读展示，保存恒拒绝（`PreviewTab::can_save` / `save_gate`）；`encoding_lossy` 文件顶部有只读提示（`lossy=1`）。非法编码绝不允许经 `fetch().text()` 解码后回写原文件。
  - **External/Unsupported 不 host webview**，由 `workspace/view.rs::preview_fallback_page` 统一 fallback 页承载（类型/路径/原因 + 重试/纯文本只读/外部打开，动作由 `preview::fallback_actions` 生成）。
  - **HTML/HTM 走 `dozer://html/` 隔离 host**（不经 `file://`）：绑定文件放进无脚本 sandbox iframe，CSP 无网络；相对资源只放行「已打开文件所在目录子树」（`assets::serve_html_file`）。
  - **Flyfish host 事件走通用 envelope**（T9）：URL 带 `proj/panel/tab/doc` 绑定，host 经 `window.__dozerFlyfishPost` 回传 `FlyfishEvent`（ready/failed/title/search_state），Rust 侧 `HostBinding` 校验归属；渲染失败回落 T1 页。
  - **大文件（windowed）**：窗口正文封顶 `WINDOW_MAX_BYTES`（`read_window_capped` 用 `take(max+1)`）；稀疏索引/流式搜索按固定块分段、段间重叠，超长单行**不得整行分配**（禁止 `BufRead::split`/`read_until` 整行）；`SetWindow` 派发判据是 `uses_editor_host()`，不是 `uses_codemirror()`；外部变更会失效旧索引（`apply_window_index` 校验 revision）。
  - **WebView 恒在 iced 之上**：预览内任何 iced 条（Find 条、窗口化搜索条、T10 冲突条）都必须由 `App::preview_desired` 显式把 webview 矩形下推条高，否则会被原生子视图盖住。
  - **T10 磁盘冲突**：脏 tab 遇外部修改进入显式冲突态（保留我的修改 / 重载磁盘·二次确认）；保存前 `save_gate` 再校验磁盘 mtime，避免提示后又变被静默覆盖。
- **应用 webview（`PanelKind::App`）走 `runtime.rs::build_app_webview`，不得复用预览/浏览器的构建路径**（那条路径装了 `dozer://` 协议与完整 IPC，应用代码不可信，见 `docs/superpowers/plans/2026-10-05-bytehost-a4b1-app-webview-mechanism.md`）：不注册 `dozer://`；IPC 只认 `focus`/`mouseup`/`zoom_*` 白名单且消息必须带每 webview 的随机 nonce（`AppIpc::parse`，页面伪造不了）；**wry 默认值不是安全默认**（下载放行、debug 开开发者工具、媒体权限自动批准），`build_app_webview` 必须逐项显式关闭（`build_app_webview_pins_the_restrictive_settings` 钉住）；导航只放行本应用 origin（唯一真相 `app_webview.rs::AppOrigin::allows_navigation`）、`window.open` 拒绝；每应用独立 `data_store_identifier`（算法已钉死，改动即破坏用户数据归属）。id 段 `APP_CONTENT_ID_OFFSET`，焦点经 `Message::AppWebViewFocused(webview_id)` 反查槽位。静态应用的出站网络**仍是 `Advisory`**，不要宣称 `Enforced`。A4b1 只做机制，URL 由谁供给属 A4b2。
- **群聊面板（`PanelKind::GroupChat`）**（见 `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`）：群聊只做讨论，**不分配任务**——"转为待办"只在 Todo 里新建一条，指派归 Todo；agent 发言走无头一次性调用且必须在执行层只读（Claude 工具白/黑名单、Codex `--sandbox read-only`），**严禁**给群聊调用加 `--dangerously-skip-permissions`/`-y`；`@` 只由 human 触发，agent 回复里的 `@` 不解析；群聊 webview 里所有 agent 输出必须经转义渲染器，不得 `innerHTML` 原文。前端 `types.ts` 的视图类型与后端 serde 输出同名同形，改协议两边要一起改；`assets/group-chat-content/*` 是运行时产物，每次前端源码改动后必须 `npm run build` 并一起提交。
