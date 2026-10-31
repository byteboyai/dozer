# bytehost 优化 Tier5：跨仓库治理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补齐四仓门禁与 CI 覆盖缺口、统一 pin 守卫、完成首个适配逻辑下沉试点。

**Architecture:** bytehost/dozer/digger 各一条分支；治理脚本与 CI 改动按依赖顺序推进（门禁→联合 CI→下沉试点）。byteui 作为第三同步点纳入 pin 守卫。

**Tech Stack:** bash、cargo-tree、GitHub Actions、python（dozer 现有棘轮脚本）。

**Spec:** [`../specs/2026-10-30-bytehost-opt-tier5-cross-repo-governance-design.md`](../specs/2026-10-30-bytehost-opt-tier5-cross-repo-governance-design.md)

## Global Constraints

- 依赖一律用 git tag；本地联调 `[patch]` 只放不提交的 `.cargo/config.toml`；提交前还原被它改动的 `Cargo.lock` 行。
- dozer 与 digger 同日升级 bytehost / byteui（ADR）；涉及 bytehost 公开 API 改动的梯队需协调两产品同步消费，发版与推送先问用户。
- bytehost 测试需 `--features server` / `--all-features`。
- 分支基线：各仓库的分支基于 `origin/main`（不要基于本地 main，里面可能有别的会话未推送的提交）；推送用 `git push origin <分支>:main`（快进）。
- 推送、打 tag、合并 main 每次先问用户。
- wire 形状只追加不修改；涉及 bytehost DTO 时不破坏旧客户端反序列化。
- 新 UI 复用统一组件（byteui），不在产品里复制按钮样式。

## Review Focus

1. 门禁不误拦合法依赖：白名单准确，变异自测确认"违规必拦、合法必过"。
2. 联合 CI 不太慢：试构建用缓存/增量，不阻塞每次 push。
3. pin 守卫合并不丢现有规则：shared-deps-guard 与 check-deps.sh pins 的并集完整保留。
4. 下沉试点不破坏产品边界：只下沉真正可复用的逻辑，产品特有策略留在产品侧。
5. check-shell-regions.sh 的跨仓归属核对清楚后再进 CI，不假设。

## 范围说明

| 项 | 处置 |
|---|---|
| 适配逻辑全面下沉 | **不在本梯队**：只做候选识别 + 首个试点；全面下沉另行规划 |
| dozer chrome 整体迁 bytehost-chrome | **不在本梯队**：已裁决渐进；本梯队只把区域校验进 CI |
| bytehost 内部结构/性能 | **不在本梯队**：归 Tier3/4 |

---

## Task 1(bytehost): 补 chrome/settings/settings-ui 门禁 + 默认 feature CI

**Files:** Modify: `scripts/check-deps.sh`（新增三组）、`.github/workflows/ci.yml`（加 --no-default-features job）。

**Interfaces:** check-deps.sh 新增 `chrome`/`settings`/`settings_ui` 三组校验规则；ci.yml 新增一条 `cargo check --no-default-features` step。

- [ ] **Step 1: 核对现状**：读 check-deps.sh 现有四组规则结构；读 ci.yml 现有 steps；确认 chrome/settings/settings-ui 的产品依赖边界应校验什么。
- [ ] **Step 2: 写变异自测**：故意在 chrome/settings/settings-ui 加违规依赖，确认当前脚本拦不住（红）。
- [ ] **Step 3: 实现**：补三组规则；ci.yml 加 --no-default-features job。
- [ ] **Step 4: 验证**：Step 2 变异被拦（绿）；合法依赖不被误拦；`bash scripts/check-deps.sh --self-test` 通过；ci.yml 本地 act 或 push 后 job 绿。
- [ ] **Step 5: Commit**：`git commit -am "ci(deps): add chrome/settings/settings-ui gates and no-default-features check"`

---

## Task 2(dozer): 移植 crate 级硬门禁

**Files:** Create: `scripts/check-host-deps.sh`（参照 digger）；Modify: dozer CI 配置（加 job）。

**Interfaces:** cargo-tree 扫描 dozer-model/dozer-services/dozer-store-project 等业务 crate，禁 bytehost-*/iced/wry；白名单外新增业务 crate 自动纳入；带变异自测。

- [ ] **Step 1: 核对 digger 脚本**：读 digger `scripts/check-host-deps.sh` 的扫描机制、白名单、变异自测实现。
- [ ] **Step 2: 核对 dozer 业务 crate**：确认 dozer-model/services/store-project 当前对 bytehost 的依赖纯度（应已纯净，靠约定维持）。
- [ ] **Step 3: 写变异自测**：故意在 dozer-model 加 bytehost 依赖，确认当前无门禁拦截（红）。
- [ ] **Step 4: 实现**：移植脚本、适配 dozer crate 名与白名单；CI 加 job。
- [ ] **Step 5: 验证**：变异被拦（绿）；现有合法依赖不误拦；CI job 绿。
- [ ] **Step 6: Commit**：`git commit -am "ci(deps): port crate-level host-dep gate from digger"`

---

## Task 3(dozer/digger): check-shell-regions.sh 进 CI

**Files:** Modify: dozer 或 digger CI 配置（视脚本归属）；可能 Move/Copy: `check-shell-regions.sh`。

**Interfaces:** CI job 自动校验 dozer chrome 区域数值 vs byteui 导出，漂移即失败。

- [ ] **Step 1: 核对脚本归属与机制**：确认 check-shell-regions.sh 在哪个仓库、校验什么、如何跨仓获取 byteui 导出。
- [ ] **Step 2: 设计 CI 形态**：若脚本在 digger 但校验 dozer，决定是挪到 dozer、还是建跨仓 workflow、还是把校验逻辑内联到 dozer CI。
- [ ] **Step 3: 实现**：按 Step 2 结论把校验进 CI；消除人工比对。
- [ ] **Step 4: 验证**：故意改一个区域数值，CI 失败（红→绿）；恢复后 CI 绿。
- [ ] **Step 5: Commit**：`git commit -am "ci(chrome): automate shell-region check against byteui exports"`

---

## Task 4(跨仓): 联合构建 CI + 统一 pin 守卫

**Files:** Modify: bytehost `.github/workflows/`（加联合构建触发）；bytehost `scripts/check-deps.sh` + `tools/shared-deps-guard`（合并）；digger `shared-deps.toml`（填充）；bytehost-chrome Cargo.toml（byteui 钉版纳入守卫）。

**Interfaces:** bytehost 发版前触发 dozer/digger 试构建；单一 pin 守卫覆盖 bytehost/byteui/两消费方的 tag 一致性；shared-deps.toml 登记实际钉版。

- [ ] **Step 1: 核对现有守卫**：读 shared-deps-guard 与 check-deps.sh pins 组，列出规则并集；读 digger shared-deps.toml 确认空登记；核对 chrome→byteui 钉版方式。
- [ ] **Step 2: 设计联合 CI**：bytehost push/tag 时触发 dozer/digger 试构建（用 repository_dispatch 或 workflow_call）；缓存优化避免太慢。
- [ ] **Step 3: 实现 pin 守卫合并**：shared-deps-guard 与 check-deps.sh pins 合并为单一脚本，规则并集完整保留；填充 shared-deps.toml；chrome→byteui 纳入。
- [ ] **Step 4: 实现联合 CI**：配置 workflow；试跑确认 dozer/digger 能被触发且构建结果回传。
- [ ] **Step 5: 验证**：故意制造 tag 不一致，守卫拦截；联合 CI 在 bytehost 改动时触发两消费方构建。
- [ ] **Step 6: Commit**（各仓）：`git commit -am "ci: add cross-repo joint build and unify pin guards"`

---

## Task 5(跨仓): 适配逻辑下沉首个试点

**Files:** Modify: dozer-panel-host 或 bytehost-panel（下沉目标）；digger adapters / dozer host_adapters（消费方改引用）。

**Interfaces:** 下沉的适配器 trait/函数签名；两产品消费方改为引用共享实现。

- [ ] **Step 1: 盘点候选**：列出 digger adapters（~10.5k 行）与 dozer host_adapters（~1.9k 行）中 todo/browser/agent/preview 的可复用逻辑；选最相似的一个作为试点。
- [ ] **Step 2: 写行为等价测试**：对试点适配器在两产品的行为写测试，运行确认通过（baseline）。
- [ ] **Step 3: 下沉实现**：把可复用逻辑提到 dozer-panel-host 或 bytehost-panel；两产品消费方改为引用。
- [ ] **Step 4: 验证**：Step 2 测试仍绿；两产品 `cargo test` 全绿；适配器行数下降记录。
- [ ] **Step 5: Commit**：`git commit -am "refactor(panel): sink shared adapter logic (pilot: <name>)"`
- [ ] **Step 6: 评估推广**：试点成功后在决策记录写明全面下沉的候选清单与优先级（本 Task 不强制全量）。

---

## Self-Review

- **Spec 覆盖：** T5-19→Task1、T5-20→Task2、T5-21→Task3、T5-22→Task4、T5-23→Task5。
- **顺序依赖：** Task1/2（门禁）→ Task3/4（CI）→ Task5（下沉）；门禁先行确保后续改动有防回归保护。
- **风险点：** Task3 需核对脚本跨仓归属，不假设；Task4 联合 CI 性能需缓存优化；Task5 试点选择影响推广可行性。

## 决策记录

（待填：check-shell-regions.sh 归属与 CI 形态、联合 CI 触发机制、pin 守卫合并后规则清单、下沉试点选择与推广清单、提交号。）
