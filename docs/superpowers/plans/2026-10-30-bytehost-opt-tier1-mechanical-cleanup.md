# bytehost 优化 Tier1：低风险机械改动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成分析后最低风险的 5 项机械改动：dozer bytehost tag 上收 workspace、删未用依赖、删死代码；bytehost 文档修正、SurfaceEvent 重命名。

**Architecture:** 两个仓库各一条分支：`dozer`（Task 1-3）、`bytehost`（Task 4-5）。Task 5 是破坏性重命名，需 bytehost 发版后 dozer/digger 同日升级。

**Tech Stack:** Rust、cargo、git tag 依赖。

**Spec:** [`../specs/2026-10-30-bytehost-opt-tier1-mechanical-cleanup-design.md`](../specs/2026-10-30-bytehost-opt-tier1-mechanical-cleanup-design.md)

## Global Constraints

- 依赖一律用 git tag；本地联调 `[patch]` 只放不提交的 `.cargo/config.toml`；提交前还原被它改动的 `Cargo.lock` 行。
- dozer 与 digger 同日升级 bytehost / byteui（ADR）；涉及 bytehost 公开 API 改动的梯队需协调两产品同步消费，发版与推送先问用户。
- bytehost 测试需 `--features server` / `--all-features`。
- 分支基线：各仓库的分支基于 `origin/main`（不要基于本地 main，里面可能有别的会话未推送的提交）；推送用 `git push origin <分支>:main`（快进）。
- 推送、打 tag、合并 main 每次先问用户。
- wire 形状只追加不修改；涉及 bytehost DTO 时不破坏旧客户端反序列化。
- 新 UI 复用统一组件（byteui），不在产品里复制按钮样式。

## Review Focus

1. tag 上收后 spike 独立 workspace 的旧 tag 是否被正确隔离或清理。
2. 删除未用依赖后编译确实通过（间接消费经 dozer-panel-host，不是遗漏）。
3. layout_compat.rs 确实无运行时引用（不只是 allow(dead_code)）。
4. SurfaceEvent 重命名的消费点是否全部覆盖（dozer/digger grep 零残留）。

## 范围说明

| 项 | 处置 |
|---|---|
| bytehost Core 锁模型 / wry 执行器重构 | **不在本梯队**：归 Tier3 |
| dozer overlay/surface 去重 | **不在本梯队**：归 Tier2 |
| gateway 性能 | **不在本梯队**：归 Tier4 |

---

## Task 1(dozer): bytehost tag 上收 workspace.dependencies

**Files:** Modify: 根 `Cargo.toml`（`[workspace.dependencies]` 新增 9 个 bytehost crate）；13 个消费 crate 的 `Cargo.toml`（改为 `.workspace = true`）；`spike/v2-excalidraw/Cargo.toml`、`spike/browser-probe/Cargo.toml`（清理旧 tag 或加注）；`Cargo.lock`。

**Interfaces:** 无公开 API 变化。依赖声明方式变更，版本语义不变。

- [ ] **Step 1: 核对现状**：`grep -rn 'bytehost' crates/*/Cargo.toml spike/*/Cargo.toml` 列出全部声明点；确认 digger 根 Cargo.toml 的 workspace 写法作为参照。
- [ ] **Step 2: 改根 Cargo.toml**：在 `[workspace.dependencies]` 添加 9 个 bytehost crate（tag v0.4.17），feature 按消费方需求保留（apps 的 server、desktop 的 overlay、client 的 conformance/in-process 由各消费 crate 自行追加 features，workspace 只声明基础版本）。
- [ ] **Step 3: 改各消费 crate**：把 `bytehost-xxx = { git = "...", tag = "v0.4.17" }` 替换为 `bytehost-xxx.workspace = true`；需要额外 feature 的保留 `{ workspace = true, features = [...] }`。
- [ ] **Step 4: 处理 spike**：spike 是独立 workspace，其 Cargo.toml 不继承主 workspace；把 v0.2.0/v0.4.16 旧 tag 升到 v0.4.17 或加注释说明"spike 独立锁版，不随主仓升级"。
- [ ] **Step 5: 验证**：`cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` 全绿；`grep -rn 'tag = "v0.4.17"' crates/` 应零结果（仅根 Cargo.toml 有）。
- [ ] **Step 6: Commit**：`git commit -am "chore(deps): centralize bytehost tag in workspace.dependencies"`

---

## Task 2(dozer): 删除未用依赖

**Files:** Modify: `crates/dozer-panel-files/Cargo.toml`（删 bytehost-core、bytehost-desktop）、`crates/dozer-panel-project/Cargo.toml`（删 bytehost-desktop）；`Cargo.lock`。

**Interfaces:** 无。这两个 crate 经 dozer-panel-host 间接消费 bytehost，无直接 use。

- [ ] **Step 1: 核对零引用**：`grep -rn 'bytehost_core\|bytehost_desktop' crates/dozer-panel-files/src/ crates/dozer-panel-project/src/` 应零结果。
- [ ] **Step 2: 删依赖行**。
- [ ] **Step 3: 验证**：`cargo build -p dozer-panel-files -p dozer-panel-project && cargo test -p dozer-panel-files -p dozer-panel-project` 全绿。
- [ ] **Step 4: Commit**：`git commit -am "chore(deps): remove unused bytehost deps from panel-files and panel-project"`

---

## Task 3(dozer): 删除 layout_compat.rs 死代码

**Files:** Delete: `crates/dozer-app/src/app/layout_compat.rs`；Modify: `crates/dozer-app/src/app/mod.rs`（删 `mod layout_compat;` 声明）。

**Interfaces:** 无。模块未接线，无外部消费者。

- [ ] **Step 1: 核对未接线**：`grep -rn 'layout_compat' crates/dozer-app/src/` 除 mod 声明与文件自身外零引用；确认注释中"尚未接入"属实。
- [ ] **Step 2: 删文件与 mod 声明**。
- [ ] **Step 3: 验证**：`cargo build -p dozer-app && cargo test -p dozer-app` 全绿。
- [ ] **Step 4: Commit**：`git commit -am "refactor(app): remove unwired layout_compat module"`

---

## Task 4(bytehost): 文档修正

**Files:** Modify: `README.md`（crate 数量"七个"→9，表格补 settings/settings-ui）、`crates/bytehost-panel/src/lib.rs`（顶部注释去 Tauri）、`crates/bytehost-webview/src/lib.rs`（同上）。

**Interfaces:** 无。纯文档。

- [ ] **Step 1: 核对现状**：读 README crate 表格、两个 lib.rs 顶部注释，确认待修正文字。
- [ ] **Step 2: 修正**：README 数量与表格补全；lib.rs 注释改为反映 iced+wry 当前事实，去除"Tauri 宿主（Digger）"。
- [ ] **Step 3: 验证**：`cargo fmt --all --check`（文档不影响编译，但保持流程一致）。
- [ ] **Step 4: Commit**：`git commit -am "docs: fix crate count and remove stale Tauri references"`

---

## Task 5(bytehost): SurfaceEvent 重命名（破坏性，需发版协调）

**Files:** Modify: `crates/bytehost-desktop/src/shell.rs`（`SurfaceEvent` → `HostSurfaceEvent`）、`crates/bytehost-desktop/src/lib.rs`（更新 re-export）；dozer/digger 所有消费点（Grep 确定）。

**Interfaces:** `desktop::shell::SurfaceEvent` 重命名为 `HostSurfaceEvent`。`core::SurfaceEvent` 不变。消费方 import 路径更新。

- [ ] **Step 1: 核对消费点**：在 bytehost/dozer/digger 三仓 `grep -rn 'shell::SurfaceEvent\|desktop::SurfaceEvent'`，列出全部引用。
- [ ] **Step 2: 重命名**：shell.rs 定义 + lib.rs re-export + 所有消费点。
- [ ] **Step 3: 验证 bytehost**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check && bash scripts/check-deps.sh` 全绿。
- [ ] **Step 4: 发版（先问用户）**：批准后打新 tag、推送；`git ls-remote --tags origin <新tag>` 核对。
- [ ] **Step 5: dozer/digger 同日升级**：两仓分支升 tag、更新消费点、`cargo test` 全绿。推送先问用户。
- [ ] **Step 6: Commit**（bytehost）：`git commit -am "refactor(desktop)!: rename shell::SurfaceEvent to HostSurfaceEvent to avoid ambiguity with core::SurfaceEvent"`

---

## Self-Review

- **Spec 覆盖：** T1-01→Task1、T1-02→Task1+2、T1-03→Task3、T1-04→Task4、T1-05→Task5。
- **风险点：** Task 5 是破坏性改动，发版与升级需用户批准且 dozer/digger 必须同日；Task 1 的 spike 独立 workspace 需确认不被主 workspace 改动波及。
- **类型一致：** 无新增类型；Task 5 仅重命名。

## 决策记录

（待填：新 tag 号、spike 旧 tag 处置方式、提交号。）
