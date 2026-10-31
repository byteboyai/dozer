# bytehost 优化 Tier2：去重与收敛 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 dozer 侧 3 处、digger 侧 3 处重复代码与样板，行为严格等价。

**Architecture:** dozer 一条分支（Task 1-3）、digger 一条分支（Task 4-6）。两仓互不依赖，可并行。

**Tech Stack:** Rust、iced、wry、cargo test。

**Spec:** [`../specs/2026-10-30-bytehost-opt-tier2-dedup-convergence-design.md`](../specs/2026-10-30-bytehost-opt-tier2-dedup-convergence-design.md)

## Global Constraints

- 依赖一律用 git tag；本地联调 `[patch]` 只放不提交的 `.cargo/config.toml`；提交前还原被它改动的 `Cargo.lock` 行。
- dozer 与 digger 同日升级 bytehost / byteui（ADR）；涉及 bytehost 公开 API 改动的梯队需协调两产品同步消费，发版与推送先问用户。
- bytehost 测试需 `--features server` / `--all-features`。
- 分支基线：各仓库的分支基于 `origin/main`（不要基于本地 main，里面可能有别的会话未推送的提交）；推送用 `git push origin <分支>:main`（快进）。
- 推送、打 tag、合并 main 每次先问用户。
- wire 形状只追加不修改；涉及 bytehost DTO 时不破坏旧客户端反序列化。
- 新 UI 复用统一组件（byteui），不在产品里复制按钮样式。

## Review Focus

1. 去重后行为等价：每个 Task 必须先有覆盖原行为的测试，抽取后测试仍绿。
2. 抽取边界合理：公共 helper/trait 放对位置（dozer-panel-host vs dozer-app internal），不引入新耦合。
3. overlay 模板化试点先行：不一次改 15 个，先 2-3 个验证模式。
4. digger subscription 合并后测试覆盖生产路径而非仅测试路径。

## 范围说明

| 项 | 处置 |
|---|---|
| bytehost 内部 wry 执行器重复 | **不在本梯队**：归 Tier3 Task 13 |
| 两产品适配器全面下沉面板库 | **不在本梯队**：归 Tier5 Task 23；本梯队只做 dozer 侧 SurfaceExecutor 上收 |
| port.rs 若有明确路线图 | 可改为标注延后而非删除，需用户裁决 |

---

## Task 1(dozer): overlay diff-webview 创建去重

**Files:** Create: `crates/dozer-app/src/platform/diff_webview.rs`（公共 helper）；Modify: `file_history_overlay.rs`、`edit_history_overlay.rs`（改为调用 helper）。

**Interfaces:**
- `pub(crate) struct DiffWebviewConfig { pub title: String, pub channel: &'static str, pub initial_html: String }`
- `pub(crate) fn create_diff_webview(cfg: DiffWebviewConfig) -> Result<wry::WebView, ...>`

- [ ] **Step 1: 核对相似度**：并排读两份代码，确认差异仅在变量名/消息名；记录差异点。
- [ ] **Step 2: 写行为快照测试**：对 file_history_overlay 与 edit_history_overlay 各写一个测试，断言 webview 创建参数（title/channel/html）与当前行为一致。运行确认通过（baseline）。
- [ ] **Step 3: 抽 helper**：创建 diff_webview.rs，把共同逻辑提取为 `create_diff_webview(cfg)`；两份 overlay 改为构造 config + 调用 helper。
- [ ] **Step 4: 运行**：Step 2 的测试仍绿；`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets -- -D warnings`。
- [ ] **Step 5: Commit**：`git commit -am "refactor(platform): extract shared diff-webview creation helper"`

---

## Task 2(dozer): SurfaceExecutor 单槽包装上收 dozer-panel-host

**Files:** Modify: `crates/dozer-panel-host/src/lib.rs`（新增 `SingleSlotSurface` 模块）；`crates/dozer-panel-usage/src/surface.rs`、`crates/dozer-app/src/extensions/codehealth/surface.rs`（改为引用 panel-host）。

**Interfaces:**
- `dozer_panel_host::SingleSlotSurface<Msg>`：固定单槽 SurfaceExecutor 包装，提供 `attach/detach/execute` 方法（签名从两份现有实现取交集）。

- [ ] **Step 1: 核对同构**：并排读两份 surface.rs，确认公开方法签名与语义一致；记录差异（若有）。
- [ ] **Step 2: 写测试**：对 usage 与 codehealth 的 Surface 行为各写测试（attach→execute→detach 序列），运行确认通过。
- [ ] **Step 3: 上收**：在 dozer-panel-host 新增 SingleSlotSurface；两份消费方改为 type alias 或直接使用。
- [ ] **Step 4: 运行**：Step 2 测试仍绿；`cargo test -p dozer-panel-host -p dozer-panel-usage -p dozer-app`。
- [ ] **Step 5: Commit**：`git commit -am "refactor(panel-host): centralize single-slot SurfaceExecutor wrapper"`

---

## Task 3(dozer): overlay 宿主模板化（试点）

**Files:** Create: `crates/dozer-app/src/platform/overlay_host.rs`（trait 或宏）；Modify: 2-3 个最相似的 overlay 宿主（试点）。

**Interfaces:**
- `trait OverlayHost { fn focus_tracker(&mut self) -> &mut FocusTracker; fn cursor_state(&self) -> CursorState; ... }` + 默认方法实现，或 `overlay_host!` 宏生成样板。

- [ ] **Step 1: 盘点 overlay 宿主**：列出全部 ~15 个，按相似度分组；选 2-3 个最相似的作为试点。
- [ ] **Step 2: 写试点行为测试**：对选中 overlay 的打开/关闭/焦点/光标行为写测试，运行确认通过。
- [ ] **Step 3: 抽模板**：实现 trait/宏；试点 overlay 改为使用。
- [ ] **Step 4: 运行**：试点测试仍绿；样板行数对比（记录下降比例）。
- [ ] **Step 5: Commit**：`git commit -am "refactor(platform): templatize overlay host boilerplate (pilot)"`
- [ ] **Step 6: 评估推广**：试点成功后在决策记录中写明是否推广至其余 overlay（本 Task 不强制全量）。

---

## Task 4(digger): 合并双订阅实现

**Files:** Modify: `crates/app-host/src/effects.rs`（统一订阅逻辑）；Delete or Modify: `crates/app-host/src/subscription.rs`；Modify: `tests/embedded.rs`（改测生产路径）。

**Interfaces:** 无新增公开 API。subscription.rs 的规则（Changed/Resync/退避）合并进 effects.rs 的生产实现。

- [ ] **Step 1: 核对两份编码**：并排读 subscription.rs 与 effects.rs:664-697，确认规则一致；记录差异。
- [ ] **Step 2: 写生产路径测试**：对 effects.rs 的订阅循环写 Changed/Resync/退避测试，运行确认通过。
- [ ] **Step 3: 合并**：把 subscription.rs 的规则统一到 effects.rs；subscription.rs 删除或改为 re-export；tests/embedded.rs 改测生产路径。
- [ ] **Step 4: 运行**：`cargo test -p app-host && cargo clippy -p app-host --all-targets -- -D warnings`。
- [ ] **Step 5: Commit**：`git commit -am "refactor(app-host): merge duplicate subscription logic into production path"`

---

## Task 5(digger): 处置 port.rs 超前建设

**Files:** Delete or Modify: `crates/app-host/src/port.rs`；Modify: `crates/app-host/src/lib.rs`（mod 声明）。

**Interfaces:** 无。PortResetPlan/Stage 无生产消费者。

- [ ] **Step 1: 核对零消费**：`grep -rn 'PortResetPlan\|PortResetStage\|port::' crates/` 确认仅定义无使用。
- [ ] **Step 2: 裁决**：默认删除（YAGNI）；若用户确认 Advanced 页有明确路线图则改为 `#[allow(dead_code)]` + TODO 注明预期接入时间。
- [ ] **Step 3: 执行删除或标注**。
- [ ] **Step 4: 运行**：`cargo test -p app-host && cargo clippy -p app-host --all-targets -- -D warnings`（删除后无 dead_code 警告）。
- [ ] **Step 5: Commit**：`git commit -am "refactor(app-host): remove unwired port reset scaffolding"`

---

## Task 6(digger): 抽 boot helper

**Files:** Modify: `crates/desktop/src/product.rs`（六段同型 + 8 连发收敛为 helper 调用）；Create（可选）: `crates/desktop/src/boot.rs`。

**Interfaces:**
- `fn boot_receiver_stream<T>(rx: Receiver<T>, ...) -> Task<T>`：封装 take_receiver→Task::stream(unfold) 同型模式。
- `fn attach_all_handles(shell: &mut ProductShell, ...)`：封装 8 连发 attach_handle。

- [ ] **Step 1: 核对同型段落**：读 product.rs:3952-3967 与 :3992-4067，确认六段与 8 连发的结构一致性。
- [ ] **Step 2: 写启动行为测试**：对 boot 流程的关键 observable（receiver 流建立、handle 挂载）写测试，运行确认通过。
- [ ] **Step 3: 抽 helper**：实现两个 helper；product.rs 改为调用。
- [ ] **Step 4: 运行**：`cargo test -p desktop && cargo clippy -p desktop --all-targets -- -D warnings`；Step 2 测试仍绿。
- [ ] **Step 5: Commit**：`git commit -am "refactor(desktop): extract boot receiver-stream and handle-attach helpers"`

---

## Self-Review

- **Spec 覆盖：** T2-06→Task1、T2-07→Task2、T2-08→Task3、T2-09→Task4、T2-10→Task5、T2-11→Task6。
- **行为等价保证：** 每个 Task 均先写 baseline 测试再抽取，抽取后测试仍绿。
- **风险点：** Task 1 若 bytehost web_child 能覆盖 diff-webview 场景则优先改走（需核对）；Task 3 试点先行不强制全量；Task 5 删除 vs 标注需用户裁决。

## 决策记录

（待填：Task 1 是否改走 web_child、Task 3 试点 overlay 选择与推广决定、Task 5 删除/标注裁决、提交号。）
