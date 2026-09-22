# 文件预览重构 Phase A：能力估算、路由与状态机 Implementation Plan

**Goal:** 在不改变用户当前预览行为的前提下，建立全局客户端能力、文件画像、
唯一预览路由和可迁移的 backend 状态机，为 CodeMirror 与资源管理提供稳定基础。

**Depends on:** 无。

**Spec:** `docs/superpowers/specs/2026-09-22-file-preview-architecture-redesign.md`

**Master plan:** `docs/superpowers/plans/2026-09-22-file-preview-architecture-redesign.md`

> 状态(2026-09-22):Phase A 已实现。现状冻结矩阵见
> `docs/superpowers/analysis/file-preview-current-matrix.md`。

## Task 1：冻结现状与迁移矩阵

**Files:**

- Create: `docs/superpowers/analysis/file-preview-current-matrix.md`
- Test fixtures: `crates/dozer-app/tests/fixtures/preview/` 或测试运行时生成器

- [x] 列出 Files/Project 两个 `PreviewPane` 从打开路径到渲染、搜索、保存、关闭、
  主题、WebView pool、MCP context 的完整调用链。
- [x] 为 `editor/tabular/json_tree/loading/truncated` 所有组合列出现状合法状态和
  消费方；标记将由统一 backend 替换的位置。
- [x] 建路由 fixture：源码、无扩展名文本、Markdown、HTML、SVG、严格 JSON、
  JSONC/JSON5、JSONL、CSV/TSV/XLSX、图片、PDF、压缩包、未知 UTF-8、未知二进制。
- [x] 建性能 fixture 生成器，不提交数百 MB 二进制 fixture(画像类 fixture 已在
  `file_profile` 测试内联生成；>64MiB 大文件生成器随 Phase C 的
  `file_policy`/`large_text` 落地)。
- [x] 记录当前测试命令和已知失败，避免迁移把旧失败误判为新回归
  (`workspace::tests::agent_icon_maps_each_kind_to_brand_icon`，改动前即失败)。

## Task 2：全局客户端能力快照

**Files:**

- Create: `crates/dozer-app/src/capabilities.rs`
- Modify: `crates/dozer-app/src/app/state.rs`
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`
- Modify: `crates/dozer-app/src/main.rs` 或实际 App 初始化入口

**Interfaces:**

```rust
pub struct HardwareCapabilities {
    pub total_memory_bytes: u64,
    pub available_memory_at_start_bytes: u64,
    pub physical_cpu_count: usize,
    pub logical_cpu_count: usize,
}

pub struct ResourceBudgets {
    pub single_editor_bytes: u64,
    pub total_preview_bytes: u64,
    pub json_tree_bytes: u64,
    pub full_file_load_bytes: u64,
    pub max_heavy_webviews: usize,
    pub background_parallelism: usize,
}

pub fn detect_hardware() -> HardwareCapabilities;
pub fn estimate_capabilities(raw: HardwareCapabilities) -> ClientCapabilities;
```

- [x] 先写 4/8/16/32/64GiB、低 available、CPU 0/1、clamp、单调性测试。
- [x] 实现纯预算函数；不得读取环境或全局状态。
- [x] 实现 `detect_hardware`，失败使用保守默认值且记录 warning。
- [x] 启动只探测一次，构造 `Arc<ClientCapabilities>` 注入 App/Workspace
  (`runtime::build_app` 探测 + `install`，App/`ShellIo`/`PreviewPane` 注入)。
- [x] 删除或改接 `native_editor::full_load_max_bytes()` 的重复 sysinfo 探测，但暂不
  改变旧 editor 分档行为；用 adapter 从新预算提供旧值。
- [x] 输出一次结构化启动日志，敏感信息只含硬件容量，不含路径/文件名。

## Task 3：文件画像

**Files:**

- Create: `crates/dozer-app/src/preview/file_profile.rs`
- Modify: `crates/dozer-app/src/preview/mod.rs`

**Interfaces:**

```rust
pub struct FileProfile {
    pub size_bytes: u64,
    pub sampled_line_count: Option<u64>,
    pub sampled_max_line_bytes: usize,
    pub utf8: Utf8Status,
    pub content_kind: ContentKind,
    pub modified: Option<SystemTime>,
}

pub fn profile_file(path: &Path) -> io::Result<FileProfile>;
```

- [x] 测试空文件、短文本、BOM、CRLF、非法 UTF-8、NUL、超长单行、扩展名伪装。
- [x] 只采样文件头尾的有界字节，不因画像扫描整个大文件(头/尾各至多 64KiB)。
- [x] 路由只用画像结果，不在多个模块重复读 metadata/猜二进制。
- [x] 保留原始 encoding/line-ending 信息供后续保存策略使用
  (`TextEncoding`/`LineEnding`/`has_bom`)。

## Task 4：单一 PreviewRouter

**Files:**

- Create: `crates/dozer-app/src/preview/router.rs`
- Modify: `crates/dozer-app/src/preview/native_editor.rs`（转 adapter）
- Modify: `crates/dozer-app/src/preview/webview.rs`（转 adapter）
- Modify: `crates/dozer-app/src/tabular/mod.rs`
- Modify: `crates/dozer-app/src/json_tree/mod.rs`

**Interfaces:**

```rust
pub enum PreviewKind {
    Code,
    Rendered,
    Json,
    Tabular,
    Streamed,
    External,
    Unsupported,
}

pub struct PreviewRoute {
    pub kind: PreviewKind,
    pub default_mode: PreviewMode,
    pub alternate_modes: Vec<PreviewMode>,
    pub reason: RouteReason,
}

pub fn classify_preview(
    path: &Path,
    profile: &FileProfile,
    capabilities: &ClientCapabilities,
    persisted_mode: Option<PreviewMode>,
) -> PreviewRoute;
```

- [x] 先为规格路由矩阵逐项写表驱动测试。
- [ ] 文件名规则优先于 extension fallback，覆盖 Makefile/Dockerfile/LICENSE/dotfile
  ——**刻意推迟**:改动前这些名字走 Flyfish、测试断言如此,Phase A 契约是行为
  不变,留待 Phase 4 真实迁移(见矩阵 §6)。
- [x] 用户 mode 仅在该 route 支持时覆盖默认值；失效旧 mode 安全回默认。
- [x] 路由结果带可展示 reason；不得返回无法解释的 bool 组合。
- [x] 打开路径、native/tabular/json viewer 选择与 WebView 生命周期统一消费
  router/backend；旧扩展名谓词仅保留在 router 内部作为兼容分类实现，后续阶段
  替换规则时不再需要修改各消费方。

## Task 5：BackendState 与 PreviewTab adapter

**Files:**

- Create: `crates/dozer-app/src/preview/backend.rs`
- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/preview/view.rs`

- [x] 定义 `BackendState::{Suspended,Queued,Loading,Ready,Failed}` 和合法转换。
- [x] 定义 `PreviewBackend` 描述；runtime handle 不要求 `Clone/Serialize`。
- [x] `PreviewTab` 新增 route/backend 字段，旧字段暂保留为 adapter。
- [x] 新开 tab 先建 descriptor/backend，再由 adapter 创建现有 viewer。
- [x] `desired_webviews`、active、外部变更与主题重载改读 backend
  (`hosts_webview`)；Rendered/Source 切换同步更新 backend，native 判定由 router
  给出；Find 能力仍读运行时 adapter，留待 Phase 4 收敛。
- [x] debug/test 下断言 backend 与旧字段一致，发现迁移漏点立即失败。
- [x] Failed 状态提供 retry/plain-text/external-open 能力描述，不再无限 Loading。

## Task 6：持久化 schema 前向兼容骨架

**Files:**

- Modify: `crates/dozer-app/src/preview_state.rs`

- [x] 新增 version 与 `PersistedPreviewTab`，字段先允许缺省。
- [x] 旧 `{paths,active_path}` 能迁移为新 descriptor；保存后只写新版。
- [x] 本阶段仍按现有方式加载内容，不提前引入 Suspended 行为。
- [x] 测试旧版、损坏文件、缺字段、新版 round-trip 和未知 mode fallback。

## Phase A 验收

- [x] 用户可见行为与迁移前一致(路由逐项对齐旧行为，见矩阵 §6 的推迟清单)。
- [x] 所有新打开的 tab 都有唯一 route/backend/reason。
- [x] 硬件只探测一次，路由测试不依赖真实机器(纯函数 + 构造 `ClientCapabilities`)。
- [x] Phase A 定向测试 105/105 通过，fmt 与 `git diff --check` 通过；完整测试
  1167 通过、2 个非本阶段失败：既有的
  `agent_icon_maps_each_kind_to_brand_icon`，以及独立运行仍失败的 git watcher
  debounce 测试。`cargo clippy -p dozer-app --all-targets` 成功，保留 4 个非本
  阶段 warning；`-D warnings` 因这些存量 warning 失败。
- [x] 回填 master plan Phase 0–2 完成项。
