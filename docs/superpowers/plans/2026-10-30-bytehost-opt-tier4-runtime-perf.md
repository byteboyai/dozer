# bytehost 优化 Tier4：运行时性能 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 bytehost gateway 每请求的 Site clone 分配；评估并按结论处置 spawn_blocking。所有改动以基准测量佐证。

**Architecture:** bytehost 一条分支；发版后 dozer/digger 同日升级。

**Tech Stack:** Rust、tokio、gateway HTTP 库（待核对：hyper/axum/其它）、criterion 或 wrk 压测。

**Spec:** [`../specs/2026-10-30-bytehost-opt-tier4-runtime-perf-design.md`](../specs/2026-10-30-bytehost-opt-tier4-runtime-perf-design.md)

## Global Constraints

- 依赖一律用 git tag；本地联调 `[patch]` 只放不提交的 `.cargo/config.toml`；提交前还原被它改动的 `Cargo.lock` 行。
- dozer 与 digger 同日升级 bytehost / byteui（ADR）；涉及 bytehost 公开 API 改动的梯队需协调两产品同步消费，发版与推送先问用户。
- bytehost 测试需 `--features server` / `--all-features`。
- 分支基线：各仓库的分支基于 `origin/main`（不要基于本地 main，里面可能有别的会话未推送的提交）；推送用 `git push origin <分支>:main`（快进）。
- 推送、打 tag、合并 main 每次先问用户。
- wire 形状只追加不修改；涉及 bytehost DTO 时不破坏旧客户端反序列化。
- 新 UI 复用统一组件（byteui），不在产品里复制按钮样式。

## Review Focus

1. 基准测量方法可信：压测脚本/criterion bench 可复现，baseline 与改后在同一环境对比。
2. Arc\<Site\> 不引入新锁竞争：若 Site 需更新，ArcSwap/RwLock 方案不劣化读路径。
3. spawn_blocking 结论诚实：若保留则记录理由，不为改而改；若去除则确认无真阻塞操作遗留导致 executor 卡顿。
4. 性能改动不改行为：静态资源响应内容、状态码、头部与改前一致。

## 范围说明

| 项 | 处置 |
|---|---|
| gateway 路由锁模型 | **不改**：已用独立 RwLock，设计正确 |
| 非 gateway 路径的性能 | **不在本梯队** |
| spawn_blocking 若核对后全部合理 | 标为"调查结论：保留"，仅做 Task 1 分配优化 |

---

## Task 0(bytehost): 建立 gateway 基准

**Files:** Create: `crates/bytehost-apps/benches/gateway.rs`（criterion）或 `scripts/bench-gateway.sh`（wrk 压测脚本）。

**Interfaces:** 可复现的基准工具，输出 p50/p99 延迟、吞吐、每请求分配次数（若用 criterion + dhat/heaptrack）。

- [ ] **Step 1: 核对 gateway 栈**：读 `crates/bytehost-apps/src/gateway/` 确认 HTTP 库、Site 定义、serve_static 与 spawn_blocking 闭包内容、每 app gateway 拓扑。
- [ ] **Step 2: 建基准**：criterion bench 或 wrk 脚本，模拟静态资源请求负载（并发 50-100，持续 10s）。
- [ ] **Step 3: 记录 baseline**：跑 3 次取中位数，记录 p50/p99/吞吐/分配次数到决策记录。
- [ ] **Step 4: Commit**：`git commit -am "bench(gateway): add reproducible baseline benchmark"`

---

## Task 1(bytehost): Arc\<Site\> 替代每请求 clone

**Files:** Modify: `crates/bytehost-apps/src/gateway/mod.rs`（serve_static 签名、路由表 Site 存储）；Site 定义处（若需 Arc 包装）。

**Interfaces:**
- 路由表存 `Arc<Site>`（或 `ArcSwap<Site>` 若需热更新）。
- `serve_static(site: Arc<Site>, ...)` 替代 `serve_static(site: &Site, ...)` + 内部 clone。

- [ ] **Step 1: 核对 Site 可变性**：确认 Site 是否运行期更新；若是，选 ArcSwap/RwLock\<Arc\<Site\>\>；若否，直接 Arc。
- [ ] **Step 2: 写行为等价测试**：静态资源响应内容/状态码/头部与改前一致（复用现有 gateway 测试或新增）。
- [ ] **Step 3: 实现**：路由表改存 Arc；serve_static 接收 Arc；消除 .cloned()。
- [ ] **Step 4: 重测基准**：跑 Task 0 基准，对比 baseline；断言分配次数下降、p50/p99 不劣化。
- [ ] **Step 5: 运行**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check && bash scripts/check-deps.sh`。
- [ ] **Step 6: Commit**：`git commit -am "perf(gateway): use Arc<Site> to eliminate per-request clone allocations"`

---

## Task 2(bytehost): spawn_blocking 调查与处置

**Files:** Modify（若需）: `crates/bytehost-apps/src/gateway/mod.rs:330` 附近。

**Interfaces:** 无公开 API 变化。内部调度策略调整。

- [ ] **Step 1: 核对闭包内容**：读 spawn_blocking 包裹的闭包，分类每个操作：文件 I/O / 纯 CPU / async 误判。
- [ ] **Step 2: 书面结论**：
  - 全部文件 I/O → 保留，记录理由，本 Task 结束（仅 Commit 调查结论到决策记录）。
  - 含非阻塞操作 → 去除 spawn_blocking 或拆分，仅真阻塞部分走 blocking pool。
- [ ] **Step 3: 若改动，写行为等价测试**：响应内容/延迟/executor 无卡顿。
- [ ] **Step 4: 若改动，重测基准**：对比 baseline，确认吞吐/延迟改善且无回归。
- [ ] **Step 5: 运行**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`。
- [ ] **Step 6: Commit**：`git commit -am "perf(gateway): [retain|remove|split] spawn_blocking based on I/O analysis"`（按实际结论选词）

---

## Task 3(bytehost): 发版 + 消费方同日升级（先问用户）

- [ ] **Step 1:** bytehost 全量验证（跑两遍确认无偶发）。
- [ ] **Step 2:** 给用户审阅基准对比数据；批准后合并 main、打新 tag、推送。
- [ ] **Step 3:** dozer/digger 升 tag、`cargo test` 全绿；推送先问用户。

---

## Self-Review

- **Spec 覆盖：** T4-B→Task0、T4-17→Task1、T4-18→Task2。
- **测量佐证：** 每个性能 Task 均先建/复用基准，改后对比，不接受定性声明。
- **风险点：** Task 2 可能结论为"保留"，这是合法结果；Task 1 若 Site 需热更新则 ArcSwap 引入新依赖，需评审。

## 决策记录

- gateway 栈:hyper 1 + hyper-util,`serve_static` 每请求 `Site` clone。
- **baseline(v0.4.18 前,64 并发 / 5000 请求,3 次中位数)**:28491 req/s,p50 2.12ms,
  p99 6.17ms,77.8 allocs/req,36485 B/req。
- **改后(`Arc<Site>`)**:29487 req/s,p50 2.06ms,p99 5.03ms,76.7 allocs/req,36410 B/req。
  每请求少 1 次分配(PathBuf clone),吞吐/延迟不劣化。分配口径是**整进程(客户端+服务端)**,
  见 `gateway_bench` 注释。
- **spawn_blocking(Task2)结论:保留**。`route → serve_static` 里的 `std::fs::read` 是真阻塞
  文件 I/O;其余步骤是微秒级纯 CPU,拆两段调度只会多一次开销。结论写在代码注释里。
- 基准工具:`crates/bytehost-apps/examples/gateway_bench.rs` + `scripts/bench-gateway.sh`。
- 提交:bytehost `3f76601`/`2bb8f66`/`f660900`。
