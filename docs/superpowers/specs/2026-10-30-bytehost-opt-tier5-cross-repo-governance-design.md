# bytehost 优化 Tier5：跨仓库治理 规格

> 日期：2026-10-30。状态：草案，尚未实施。实施计划见 [plan](../plans/2026-10-30-bytehost-opt-tier5-cross-repo-governance.md)。
> 本梯队聚焦 bytehost/dozer/digger + byteui 四仓的门禁、CI、版本同步与适配逻辑下沉，是防漂移的治理层。

## 1. 目标与边界

补齐门禁覆盖缺口、建立跨仓联合 CI、统一 pin 守卫、把可复用适配逻辑下沉面板库。治理层改动本身不改产品行为，但影响开发流程与防回归能力。

**不在本梯队**：具体代码重构（Tier2/3）、性能（Tier4）。

## 2. 已核对的代码基线

| 位置 | 当前事实 | 治理责任 |
| --- | --- | --- |
| bytehost `scripts/check-deps.sh` | 只校验 apps/core/desktop/pins 四组；chrome/settings/settings-ui 无产品依赖门禁 | bytehost：补三组门禁 |
| bytehost `.github/workflows/ci.yml:30-33` | 只跑 `--all-features`；默认 feature 构建从不验证 | bytehost：加 `cargo check --no-default-features` |
| bytehost `tools/shared-deps-guard` | ref-mismatch/lock-duplicate/behind 规则与 check-deps.sh pins 组（:106-140）重复 | bytehost：合并/统一 |
| digger `scripts/check-host-deps.sh` | cargo-tree 扫描 + 变异自测，强制 store/editor 禁 bytehost-*/iced/wry | digger：维持；dozer 移植 |
| dozer `check_panel_boundary.py` | 棘轮只管 dozer-app 内部引用数；管不住 dozer-model/services/store-project 纯度 | dozer：移植 digger 硬门禁 |
| dozer chrome 区域校验 | 靠 digger 的 `check-shell-regions.sh` 人工比对 byteui 导出，未进 CI | dozer/digger：进 CI |
| bytehost CI | 无 dozer/digger 联合构建检查 | 跨仓：建联合 CI |
| digger `shared-deps.toml` | 钉版登记目前为空、仅提醒 | digger：填充 |
| bytehost-chrome → byteui | chrome 钉 byteui 版本，是第三个同步点 | 跨仓：纳入 pin 守卫 |
| dozer-panel-host | 已共享 PanelIo/NoticeOutbox | 跨仓：继续下沉适配逻辑 |
| digger adapters（~10.5k 行）vs dozer host_adapters（~1.9k 行） | 两产品各养一套 todo/browser/agent/preview 适配器 | 跨仓：识别可下沉候选 |

## 3. 方案

### 3.1 bytehost 补门禁 + 默认 feature CI

check-deps.sh 新增 chrome/settings/settings-ui 三组，校验产品依赖边界（参照现有 apps/core/desktop 组规则）。ci.yml 加一条 `cargo check --no-default-features`（apps 的 server/digest/manifest-toml feature 分层最依赖默认构建正确）。

### 3.2 dozer 移植 crate 级硬门禁

参照 digger check-host-deps.sh：cargo-tree 扫描 dozer-model/dozer-services/dozer-store-project 等业务 crate，强制禁 bytehost-*/iced/wry；白名单外新增业务 crate 自动纳入；带变异自测（故意加违规依赖确认被拦）。

### 3.3 check-shell-regions.sh 进 CI

核对该脚本当前所在仓库与跨仓运行方式。若逻辑校验 dozer chrome 区域 vs byteui 导出，则把校验挪到 dozer CI 或建跨仓 workflow；消除人工比对。

### 3.4 联合构建 CI + 统一 pin 守卫

建 bytehost+dozer+digger 联合构建检查（bytehost 发版前触发两消费方试构建，或各自 CI 加交叉检查 job）。合并 shared-deps-guard 与 check-deps.sh pins 组为单一守卫；填充 digger shared-deps.toml；把 bytehost-chrome→byteui 钉版纳入守卫范围。

### 3.5 适配逻辑下沉面板库

盘点 digger adapters（~10.5k 行）与 dozer host_adapters（~1.9k 行）中 todo/browser/agent/preview 的可复用逻辑。本梯队只做候选识别与首个试点（选一个最相似的适配器下沉到 dozer-panel-host 或 bytehost-panel）；全面下沉另行规划。

## 4. 兼容与发布

- 治理改动多为脚本/CI，不改产品代码，发布影响小。
- 3.4 联合 CI 与 3.5 下沉涉及多仓库协调，需说明顺序：先补门禁（3.1/3.2）→ 再建联合 CI（3.4）→ 最后试点下沉（3.5）。
- 回滚：脚本/CI 改动 revert 即可；下沉试点若破坏行为则 revert 消费方改动。

## 5. 验收与完成定义

| ID | 场景 | 必须观察到的结果 |
| --- | --- | --- |
| T5-19 | bytehost check-deps.sh 跑 chrome/settings/settings-ui 组 | 违规依赖被拦；ci.yml 含 --no-default-features job 且默认构建失败能被捕获 |
| T5-20 | dozer 业务 crate 加 bytehost 依赖 | check-host-deps.sh 拦截；变异自测通过 |
| T5-21 | dozer chrome 区域数值漂移 | CI 自动失败，无需人工比对 |
| T5-22 | bytehost 发版前 | 联合构建 CI 跑 dozer/digger 试构建；pin 守卫无重复规则；shared-deps.toml 非空 |
| T5-23 | 首个适配下沉试点 | 两产品该适配器行数下降；行为等价测试通过 |
