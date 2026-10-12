# Digger 完整接入 bytehost 实施计划

> 日期：2026-10-11。状态：待实施，所有复选框均表示尚未完成。
> 规格：[完整接入设计](../specs/2026-10-11-digger-bytehost-full-integration-design.md)。
> 目标：Dozer 与 Digger 同源使用共享桌面宿主；Digger 接通现有应用平台全部约定能力，并实际复用 Usage。
> 本文是跨仓库主计划，按 Task 执行和记录证据；不是已完成报告。新增文件和接口均为规划，不假定它们已存在。

## 0. 执行约定

- 路径前缀 `B/` 表示 bytehost 仓库，`Z/` 表示 Dozer，`D/` 表示 Digger；执行 shell 命令时使用相应仓库作为工作目录，不把前缀当真实目录。
- 每个 Task 一个可独立审查的提交组；涉及多仓库时记录各 revision 和兼容关系。提交不能混入用户既有 WIP，不能通过 reset/checkout 清理不属于任务的修改。
- 本次只写计划，不执行建仓库、依赖发布、跨仓库修改、测试或打包。实施时依照当时会话授权执行，不因本文存在而推断已有发布或合并授权。
- 跨仓库临时联调用本地未提交 patch；正式验收固定 bytehost 同一 tag、面板库固定 revision/tag，禁止依赖兄弟目录。新 tag 名以实际发布确定，不预占版本。
- 保留四个现有 crate 的框架中立性。通用 Host 不依赖任何产品 crate；产品模型与业务服务不直接搬入 Host。
- 行为复杂的迁移先写失败测试或钉住现有行为，再实施，再验证；纯文档/机械导出不写镜像实现的测试。
- 每个 Task 更新 ledger：状态、输入 revision、输出 revision、运行命令和退出码、结果、偏差、未决项。被前置条件阻塞的 Task 标 `blocked-by:<task/evidence>`，不打完成勾。
- 不将历史规格中的测试计数、已知失败或耗时当作本次基线。Task 01 重新测量；新增失败不得归为历史问题。

拟新增的记录目录：`Z/docs/dozer-v2/digger-bytehost-integration/`，含 `ledger.md`、`baseline.md`、`ownership.md`、`api-decisions.md`、`acceptance.md`。各仓库测试产物在其独立 artifact 目录，报告只引用脱敏证据。

## 1. 阶段和依赖

| 规格阶段 | Tasks | 完成闸门 |
|---|---|---|
| S0 基线/探针 | 01–03 | 当前边界与真实窗口入口已验证；关键缺口有明确归属 |
| S1 公共 core | 04–06 | ID/路由/布局受测；Dozer 服务适配不污染 Host |
| S2 公共 desktop | 07–10 | 两个不同面板接同一 Surface；共享窗口/浮层机制可用 |
| S3 Digger 正式壳 | 11 | 正式 workspace 与单面板窗口使用公共 Host |
| S4 应用服务/管理 | 12–15 | 真实接口执行器和全部管理操作接通 |
| S5 WebView/恢复 | 16–18 | 真实应用、隔离、数据清理、退出恢复闭环 |
| S6 面板与迁移收口 | 19–21 | 两产品同源 Usage；真实数据接入；通用兼容桥清零 |
| S7 发布验收 | 22–23 | A1–A18 证据齐备，固定依赖独立构建与打包 |

主路径：01 → 02 → 04 → 05 → 07 → 08 → 09 → 10 → 11 → 13 → 14 → 15 → 16 → 17 → 18 → 21 → 22 → 23。

补充依赖：03 在 01 后执行；06 依赖 04/05；12 依赖 03；13 依赖 06/11/12；19 依赖 08/09；20 依赖 03/19；21 依赖 06/10/18/19/20。11 的文件骨架可在 04 后准备，但完成验收须等 10。12 的无头实现无需等待 Digger GUI。依赖关系不要求多 agent 执行。

## 2. S0：先让设计闸门成为可验证结果

### Task 01：基线、归属清单和验收台账

**文件：**新增上述五份记录；读取三仓库 Cargo、CI、Dozer 审计脚本与 Digger ADR，不改生产代码。

- [ ] 记录三仓库 HEAD、工作区状态、平台/架构、Rust/Node/Python 版本和锁文件；隔离用户 WIP。
- [ ] 按 §4 验证命令记录现有测试、clippy、依赖门禁、Dozer 面板边界基线；Digger 仅有 spike 时明确正式 workspace 尚不存在。
- [ ] 盘点 Dozer `app/`、`workspace/`、`chrome/`、`platform/`、`panel_host.rs`、`panel_registry.rs`、`app_webview.rs`、`webview_geometry.rs`、`runtime.rs`。每项标 Host 机制 / 产品语义 / 临时适配器，并指定后续 Task。
- [ ] 为规格 A1–A18 各建未完成条目，记录验证层级、平台和所需夹具；建立兼容桥清单，删除条件必须可测。
- [ ] 记录当前 PanelKind 点名、宿主面板消息臂和业务状态字段的统计命令与数值，后续保持同口径。

**验收：**没有未归属的宿主路径；原有失败可复现或明确标记未复现；不修改门禁阈值掩盖失败。提交建议：`docs(integration): record host extraction baseline`。

### Task 02：真实窗口入口与类型契约探针

**读取：**`D/spike/editor-host/src/{main,hosts,probes}.rs`，`Z/crates/dozer-app/src/{app,platform,preview}`。
**新增：**`B/spike/desktop-contract/` 独立验证工程；`api-decisions.md` 的窗口入口、生命周期、泛型决策。

- [ ] 从实际 Cargo.lock 和 byteui 依赖确定共同 iced/wry 版本；不根据 README 猜兼容性。
- [ ] 用一个无业务模型的窗口验证 iced 事件循环、wry 子视图、主线程创建/销毁、DPI 几何和原生 overlay。使用现有平台入口，避免新建竞争事件循环。
- [ ] 用两个不同 ProductMessage/服务上下文类型编译相同 Host 回调接口；验证 iced Element 生命周期、异步 Send 和主线程非 Send 句柄边界。
- [ ] 用 Usage 与 CodeHealth 所需的不同推送形状验证 Surface 描述能表达两者；探针资源可最小化，不能伪装成已迁生产面板。
- [ ] 决定 core 放纯描述与布局/路由；desktop 放 iced view 回调和 wry 实现。记录明确签名及编译成功证据。

**验收：**真实窗口 resize、隐藏/恢复、销毁重建可观察；双产品类型通过编译。若 API 不成立，先修订决策和对应 spec，不开始大批搬文件。提交建议：`test(host): prove desktop integration contract`。

### Task 03：AppService 缺口与 Digger 数据源调查

**读取：**`B/crates/bytehost-apps/src/{service,port}.rs`、manager/process 模块、`bytehost-client/src/in_process.rs`；Digger runtime/领域文档及实际代码。

- [ ] 用隔离临时目录测试同根双启动、已保存端口冲突、损坏端口文件和 shutdown；确认可观察的错误、锁和资源释放行为。
- [ ] 列出现有 API 已支持/需补齐项；确认 shutdown 当前返回值不能表达的失败，不在适配层假装有 Result。
- [ ] 盘点真实 task/session 用量生产者、账本及查询接口；若不存在，记录 Task 20 必须补的最小垂直链路及其存储归属。
- [ ] 明确用量字段映射、时间区间、项目作用域与幂等键；缺失字段必须明确 Unsupported/Unavailable，不能编造 provider/token/cost。
- [ ] 回填 spec §11 各闸门：实测结论、仍未验证项及对应 Task。

**验收：**有可执行的 Task 12 API 补齐清单和 Task 20 数据链路方案。业务语义无法由现有架构确定时，只暂停相应依赖项，其他宿主任务继续。提交建议：`docs(integration): resolve service and usage prerequisites`。

## 3. 实施任务

### Task 04：新增 bytehost-core 和分组依赖门禁

**新增：**`B/crates/bytehost-core/{Cargo.toml,src/lib.rs,src/id.rs,src/registry.rs,src/layout.rs}`、`tests/catalog.rs`。
**修改：**`B/Cargo.toml`、`B/scripts/check-deps.sh`。

- [ ] 写测试：重复 ID 拒绝；无面板、单面板和空栏合法；未知历史 ID 可保留；应用 ID 从校验后的值映射。
- [ ] 实现稳定 PanelId、ScopeId、PanelInstanceId、SurfaceSlot 和不依赖 GUI 的注册元数据、布局 DTO。
- [ ] 对无效 ID、非法尺寸、未知 schema version 给出结构化错误；产品默认布局作为输入。
- [ ] 改依赖门禁为现有四 crate / core / desktop 三组；normal+build、所有 feature/平台均检查，禁止 `dozer-*` 和 `digger-*`。
- [ ] 加门禁变异测试：人为引入产品依赖或 core→iced 必须失败；恢复后通过。

**验证：**`cargo test -p bytehost-core`、`bash scripts/check-deps.sh`。**产出：**可供产品依赖的纯契约；提交 `feat(core): introduce product-neutral panel identities`。

### Task 05：请求身份与执行上下文

**新增：**`B/crates/bytehost-core/src/request.rs`、`tests/request_routing.rs`。
**输入：**`Z/crates/dozer-app/src/request_tracker.rs`；desktop 执行器实现随 Task 07。

- [ ] 移植现有乱序与重开实例行为测试；追加同 ID 不同 scope、不同请求 key、已排队回包后发起新请求等用例。
- [ ] 实现 latest-only 读取和 scoped-write 两种语义。写操作继续完成，实例过期时只丢弃 UI 回包；其业务结果仍由实际服务持久化。
- [ ] 用不同 generation 表达 panel 实例、服务实例和 Surface，不用全局失效使无关面板结果消失。
- [ ] 结果在 UI 派发时校验，不只在后台任务完成时校验；独立 key 不相互覆盖。
- [ ] 执行原语接收泛型服务上下文、后台执行器和 emitter；Tokio/iced 的具体实现不进入 core。

**验证：**core 请求测试；必须断言两次写都完成、两次同 key 读取只应用最新一次。提交 `feat(core): preserve scoped writes and reject stale reads`。

### Task 06：拆开 Dozer Client 适配并接公共身份

**修改：**`Z/crates/dozer-app/src/{panel_host,request_tracker,product}.rs`、构造调用点；新增 `host_adapters/services.rs`（名称按现有模块约定）。

- [ ] `ClientDaemon`、`services_for` 移入产品适配层；PanelIo 构造改接 Services，fake 不先构造离线 Client 再覆盖。
- [ ] 使用公共请求身份；ProjectId 转 ScopeId 在产品适配器内完成，Host 看不到 i64 的业务含义。
- [ ] 保留各服务失败文本和既有异步行为；复跑 settings、usage、group_chat 与 request tracker 测试。
- [ ] 旧类型包装若为渐进迁移保留，写入兼容桥清单并指向 Task 21；无调用者后删除。

**验证：**Dozer 针对性测试与边界门禁，`extensions/**` 不重新出现 Client。提交 `refactor(host): isolate Dozer service construction`。

### Task 07：新增 bytehost-desktop 的窗口壳和产品回调

**新增：**`B/crates/bytehost-desktop/{Cargo.toml,src/lib.rs,src/shell.rs,src/dispatch.rs,src/panel.rs}`、`examples/minimal.rs`、`tests/product_contract.rs`。

- [ ] 按 Task 02 已编译签名实现 `HostMessage<M>`、产品更新/视图/激活/关闭回调；业务状态由产品持有。
- [ ] 接入窗口生命周期、Tokio 执行器适配、通知与通用 Host 请求；业务命令回调不进入宿主 enum。
- [ ] 最小示例注册一个 fake 面板，支持激活、异步完成、关闭和窗口退出。
- [ ] 第二份消费者夹具使用不同消息类型与服务类型，不引用 Dozer，验证 API 不是专为一个产品建立。

**验证：**desktop 单元/编译测试与真实最小窗口；依赖门禁。提交 `feat(desktop): add reusable shell and product callbacks`。

### Task 08：Surface 状态机、原生执行与 Usage 首接

**新增：**`B/crates/bytehost-core/src/surface.rs`；`B/crates/bytehost-desktop/src/surface/{mod,lifecycle,geometry,routing}.rs`、`tests/surface_lifecycle.rs`。
**修改：**`Z/crates/dozer-app/src/app/{app,update}.rs`、`runtime.rs`、`webview_geometry.rs`、`extensions/usage/`。

- [ ] 写状态测试：首次 ready 全量推送；隐藏缓存最新状态；重建需新快照；旧 ready/IPC/回包拒绝；创建失败重试。
- [ ] 用 `(instance, slot, generation)` 显式注册路由；第三方应用导航完成与内置 ready 协议分别建策略。
- [ ] 原生句柄只在 UI 线程持有；后台只能发操作意图。实例销毁即注销路由与待投递脚本。
- [ ] 把 Usage 的逐帧推送、ready 管理和 IPC 定位改经 Surface；payload 构造仍在面板。
- [ ] 删除被替代的 Usage 专属宿主逻辑，真实窗口验证项目切换和快速隐藏/恢复。

**验证：**A3/A5 初验、Usage 既有探针与新真实窗口场景。提交分 core 状态机、desktop 执行器、Dozer 接入三部分。

### Task 09：CodeHealth 第二消费者与信任边界

**修改：**Dozer `extensions/codehealth/` 和宿主专属推送/IPC 分支；desktop Surface builders。

- [ ] CodeHealth 接入相同 Surface API，保持切入不自动扫描等既有行为。
- [ ] 为 editor、preview、application 定义独立信任配置/构造路径；公共几何与生命周期共享，产品内容协议分离。
- [ ] 加测试证明 application 无法拿到 editor/preview 的自定义协议和业务 IPC；未知 Surface/消息拒绝。
- [ ] 将从第二消费者发现的通用缺口修到公共层；禁止新增 `if panel == codehealth`。

**验证：**两真实面板共享同一承载实现，Bytehost 代码不点名这两个业务模块。提交 `refactor(codehealth): adopt shared surface lifecycle`。

### Task 10：布局、焦点、浮层与旧布局兼容

**新增：**desktop `layout.rs`、`focus.rs`、`overlay.rs`、`tests/layout_compat.rs`；Dozer 产品布局映射。
**输入：**Dozer `panel_layouts.rs`、`panel_registry.rs`、`chrome/rail.rs`、`platform/overlay_window.rs`、相关 workspace/layout 代码。

- [ ] 先存旧布局真实样本与黄金期望；迁移保留备份，未知新版本拒绝覆盖。
- [ ] 提取 rail/pane 容器、尺寸/折叠、拖拽和焦点；产品负责枚举到 ID 的映射及默认布局。
- [ ] 提取浮层机制与通用 hover；业务菜单内容和产品首页留在产品。
- [ ] 统一 DPI、裁剪、零面积和浮层遮挡；用真实鼠标测原生浮层、in-window overlay 与焦点返回。
- [ ] Dozer 先以薄包装接入，逐项登记仍有专属状态的兼容桥，不用“文件搬了”代替行为验证。

**验证：**A4/A6；单面板和空栏例子；Dozer 现有布局回归。提交按布局迁移、焦点/浮层拆分。

### Task 11：Digger 正式 workspace 与桌面组合根

**新增：**`D/Cargo.toml`、`Cargo.lock`、`crates/desktop/{Cargo.toml,src/main.rs,src/product.rs,src/paths.rs}`、`scripts/check-host-deps.sh`。

- [x] 明确 workspace members/exclude，让已有独立 spike 不被意外纳入生产工作区。
- [x] desktop 包名定为 `digger-desktop`，使用共享 Host + byteui；创建无 Project 也能启动的产品壳。
- [x] 注册一个产品信息/状态面板和应用区域占位；占位明确标示未接服务，不虚构写作功能。
- [x] 接独立产品数据根、主题、布局保存与退出；不能复制 Dozer App/Workspace。
- [x] 建依赖门禁：产品 Domain/持久化/业务 runtime 不依赖 bytehost；对尚未存在的 crate 记录覆盖缺口，新建后必须纳入。

**验证：**`cargo check -p digger-desktop`、`cargo test --workspace`、最小窗口手动运行。提交 `feat(desktop): bootstrap Digger on bytehost shell`。

### Task 12：补齐共享应用服务的管理能力

**修改：**以 Task 03 结论为准，`B/crates/bytehost-apps/src/{service,port}.rs`、manager/process 模块；新增相应 integration tests。

- [x] 同根双服务缺少互斥时加公共锁生命周期；双启动只允许一个所有者，失败方不得停止第一方进程。
- [x] 增补受控端口重置 API（若缺失）：停止/占有权校验、原子持久化、失败保留可恢复状态；不暴露任意写内部文件能力。
- [x] 增加可观察 shutdown 结果的兼容入口（若现接口不足），保留现有调用方兼容。测试停止失败、超时与 gateway 释放，不用日志推断成功。
- [x] 验证孤儿回收同时核对进程身份，不只使用可能复用的 PID；故障场景仅针对测试创建的进程组。
- [x] 已存在能力只补缺失测试，不再写第二套 API；更新公共 API 兼容记录。

**验证：**服务单测、双进程锁/重启集成、现有 embedded_host 与 conformance。提交按互斥、管理接口、退出观测拆分。

### Task 13：Digger app-host 适配与事件闭环

**新增：**`D/crates/app-host/{Cargo.toml,src/lib.rs,src/service.rs,src/effects.rs,src/subscription.rs,src/testing.rs}`，包名 `digger-app-host`。

- [x] 写 fake AppHostApi：成功、Host/Transport 失败、断订阅、Resync 与迟到回包；未预设调用显式失败。
- [x] 实现泛型 effect 执行器，生产使用 InProcess；启动应用服务异步执行，不阻塞窗口。
- [x] 复用 bytehost-panel host/install/logs 状态；Changed/Resync 合并刷新，Disconnected 按共享退避规则重订阅与轮询。
- [x] 列表/launch URL/日志结果带 scope/request 身份；卸载和面板关闭使 UI 结果失效，写操作继续由服务完成。
- [x] 管理单一服务实例及取消令牌；退出先停止新操作、撤 Surface、停止订阅，再 await shutdown。

**验证：**`cargo test -p digger-app-host`；真实临时数据根嵌入测试。提交 `feat(app-host): wire InProcess effects and subscriptions`。

### Task 14：安装审批与管理界面

**新增：**`D/crates/desktop/src/apps/{mod,management,approval}.rs`；app-host `trust.rs`、`tests/install_flow.rs`。

- [x] 接列表、详情和本地目录/归档/URL 选择；计划生成失败、空列表和服务不可用分别显示。
- [x] 展示实际计划、来源、摘要、权限和 enforcement；LocalDir 整目录复制提示保留。
- [x] 人类确认只接受当前发出的计划身份；重复点击防重；内容或来源变化需重新计划。
- [x] 本地可信进程应用经明确批准可安装；Agent/URL Untrusted 仅 static_web，产品信任决策只在 Rust 适配层。
- [x] 安装后刷新 rail，新增应用使用稳定 ID，应用不绑定当前 Project。

**验证：**fake 测试覆盖拒绝/取消/过期/重复批准；真实静态应用从选择来源到安装显示。提交 `feat(apps): add reviewed install and management flow`。

### Task 15：运行时、日志、升级回滚和卸载操作

**新增/修改：**Digger desktop `apps/{runtime,logs,actions}.rs` 与 app-host effects tests。

- [x] 接运行时 probe/plan/install/uninstall；显示依赖应用、失败原因和持续问题页。
- [x] 接 start/stop、upgrade plan、rollback；升级失败后再次拉取权威状态，不由 UI 猜运行版本。
- [x] 复用 LogsState：跟随/取消跟随、首次失败、保留旧文本、迟到回包丢弃。
- [x] 卸载 UI 明确保留数据与删除数据两种行为；先调用共享业务流程，浏览器清理接 Task 17。
- [x] 为能力矩阵每一行记录 UI 入口、effect、API、成功和失败测试名称；无入口的 API 不算接通。

**验证：**管理界面状态测试、进程应用真实安装/启停；A8/A9/A13 的服务与 UI 部分。提交按运行时、日志、版本/卸载三组。

### Task 16：真实应用 Surface 与 builder 强制策略

**修改：**`B/crates/bytehost-desktop/src/surface/` 的 application builder；Digger `apps/surface.rs`；移植并更新 spike 策略探针。

- [x] builder 接 bytehost-webview AppOrigin、AppIpc、data_store_identifier；明确关闭下载、新窗口、devtools。（共享 `bytehost-desktop::app_webview::build_app_child`，`79e7d8d`）
- [x] 将 launch URL 从 effect 直接送内存 Surface，不进持久状态；销毁重开重新取地址。（`apps/surface.rs` `set_pending_url`/`pending`；`reopen` = close 再 open、代数 +1；地址不入日志/状态）
- [x] 表格测试逐项检查 builder 决策，真实恶意页面检查跨 origin、伪造 nonce、自定义协议与合成事件结果。（表格：`build_app_child_pins_the_restrictive_settings` + Spike 同源测试；**真实恶意页面探针受 Task 08 OS 权限限制，与人工项合并**）
- [x] 剪贴板按授权和实测能力保守处理，媒体保持无用途声明；记录无法强制的项，不虚报策略。（`.with_clipboard(false)`；无媒体用途声明；无法强制项记于 ledger）
- [~] 静态、py-notes、node-notes 各运行到真实可交互内容；验证同 origin 正常跳转与焦点。（**与 Task 08 人工项合并**：Digger 一期无应用视口，真实运行待人工确认）

**验证：**A7/A10/A11；探针结果与实际 builder 同源，不能只测复制的配置。（Spike `spike_app_pins_match_the_shared_builder_verbatim`：`include_str!` 读共享 builder 生产段逐项比对）提交 `feat(desktop): enforce shared application surface policy`。

### Task 17：浏览器存储、卸载清理与端口恢复

**修改：**desktop 存储清理适配、Digger app-host 管理操作及设置页；必要时补 bytehost-webview 公共策略测试。

- [x] 先销毁所有相关 app Surface，再执行共享 StoreRemovals；清理失败显示持续待重试状态。
- [x] 清理待办在退出重启后可恢复，防止重新安装复活被用户要求删除的数据；服务数据与浏览器数据分别记录结果。
- [~] 用两个 app 验证存储隔离、同 app 重启连续性和保留/删除数据后的重装差异。（机制与表格测试已做；两 app 真实浏览器存储隔离/重启连续性依赖真实窗口（真人验收，并入 Task 08 人工作业项）。）
- [x] 设置接端口冲突重试与显式重置：展示 origin 变化影响、停服务并撤 Surface、调 Task 12 API、恢复订阅。
- [x] 测失败恢复和重复操作幂等；不在 Digger 直接编辑 gateway.json。

**验证：**A8/A12，端口冲突不损坏已有注册表和应用数据。提交 `feat(apps): recover store cleanup and gateway settings`。

### Task 18：正常退出、崩溃恢复与双产品并行运行

**新增：**`D/crates/app-host/tests/recovery.rs`、desktop 真实窗口恢复夹具；bytehost 对应故障注入测试。

- [x] 在独立根启动应用，正常关闭窗口后证明 gateway 端口关闭、子进程停止，desired 状态仍按库约定恢复。
- [~] 强杀测试专用 Digger 进程，重启同根并检查孤儿回收；测试无关进程绝不被回收。（孤儿回收按记录核对身份的机制与表格测试已做（`tests/recovery.rs` + bytehost `reconcile_reaps_a_leftover_process_recorded_in_the_run_dir`）；真实"强杀子进程 → 重启"需真实进程夹具，并入 Task 08 人工项，不以"丢弃句柄"冒充。）
- [x] 同根双启动拒绝；不同根的 Dozer/Digger 同时运行，不串端口、应用、浏览器数据。
- [~] shutdown 故障应有 UI/报告可见结果；测超时与用户选择强退，不静默等待无限时间。（报告/超时/不静默的机制与表格测试已做（`tests/recovery.rs` + bytehost `a_shutdown_that_overruns_the_timeout_is_reported_not_swallowed`）；UI 呈现强退选择并入 Task 08 人工项。）
- [~] 无 Project 启动应用；Project 导出/移动若已有实现则做真实验证，未有实现必须在 Task 23 标为未验收，不能以路径检查代替功能测试。（无 Project 启动已验证；Project 导出/移动产品层尚无实现，按计划在 Task 23 标未验收。）

**验证：**A14/A15/A16；所有 kill 目标由夹具创建并记录身份。提交 `test(apps): verify shutdown and crash recovery`。

### Task 19：提取同源 Usage 面板库

**新增：**`Z/crates/dozer-panel-usage/{Cargo.toml,src/,web/}`。
**迁移输入：**`Z/crates/dozer-app/src/extensions/usage/`、`web/usage-content/`、对应资源服务与构建逻辑。

- [x] 先列明现有行为/资源契约，连同现有测试迁入库；不能在两处各留完整实现。（阶段 1+2 已完成，`dozer-panel-usage` 52 测试迁移；旧副本已删）
- [x] 将 workspace/chrome UI helper 依赖替换为公共 UI 或注入的产品展示适配。（`PanelDisplay<M>` 接缝）
- [x] 审计本地 Git 统计路径：直接 bytegit Repo 读取改为面板所需的窄数据提供接口，Dozer 实现保持现状，Digger 明确其可提供的语义，不把 Git 实现塞进 Host。（`GitStatsProvider` / `NoGitStats`；Dozer 侧包 bytegit 待阶段 3）
- [x] 迁移离线 Web bundle、构建脚本、许可证和资源版本；运行原 npm typecheck/test/build，实际使用迁后 bundle。（阶段 2 完成，产物逐字节一致）
- [x] Dozer 仅保留重新导出/注册包装；库依赖闭包无 dozer-app、dozer-client、dozer-core，允许领域 model/services 和公共 UI。（闭包已满足；Dozer 接入完成——`extensions/usage.rs` 纯重导出，产品展示适配在 host 侧 `host_adapters/usage_panel.rs`）
- [ ] 以可固定 revision 提供 Digger 引用，验证依赖中的领域类型不出现两份不兼容来源。（阶段 3 后 + Task 20）

**验证：**Usage 旧测试、fake 独立面板真窗口、Dozer 真数据行为对照；A2。提交按 Rust 提取、资源提取、Dozer 接入拆分。

### Task 20：Digger 真实 Usage 数据闭环

**文件：**Task 03 指定的 Digger 用量账本/生产者模块；`D/crates/desktop/src/adapters/usage.rs`；独立集成测试。

- [x] 若生产者/账本已有，接查询；若不存在，先补最小真实任务完成事件→幂等持久记录→区间查询链路，按 Digger runtime 架构归属，不写进 Project 内容模型或 Host。（新业务 crate `digger-store`：`UsageLedger`/`LedgerUsageService`/`TaskRunner`+`RunningTasks`；`digger/crates/store/src/{ledger,service,runner}.rs`）
- [x] 为该链路在 `api-decisions.md` 写清文件、事件来源、去重键、单位/时区、错误与恢复；若需要新业务 ADR，先落决定再实施存储。（`api-decisions.md` §14）
- [x] 实现 UsageService 与 Task 19 其他必要数据接口；不存在的能力明确呈现，不以假数或恒空掩盖。（`LedgerUsageService` 实现 `UsageService`；读失败→`Unavailable`、确实为空→`Ok(空)`；无 agent 运行时→token 如实 0，不伪造）
- [x] 真任务结束后生成可查记录，重启后仍在；重复事件不翻倍；两个 scope 不串用量，读取失败与成功空集合区分。（`apps::tests` 五个集成测试 + `digger-store` 15 单测）
- [x] 将同一个 Usage 库注册进 Digger，用真实服务展示；开发 fake 仅在测试/演示配置启用。（`digger.usage` 面板 + `adapters/usage.rs`；注入真实 `LedgerUsageService`，无 fake）

**验证：**A17 必须从真实生产事件驱动；只插测试记录可证明查询，但不能完成 A17。若任务运行/账本尚不具备，记录具体缺口，S6/S7 保持未完成。提交 `feat(usage): connect Digger task usage to shared panel`。

**实施结果（2026-10-12）：**A17 由**真实用户发起的应用操作**经真实 `Executor::run_install_traced` 驱动（非测试插入）；token 如实为 0、代表应用操作而非 agent 会话——无真实 token 运行，**S6/S7 保持未完成**（计划 `:284`）。`cargo test -p digger-desktop`（41）/-p digger-store`（15）全绿；`check-host-deps.sh` EXIT=0；`cargo clippy --all-targets` 无警告；`cargo build --workspace` 全绿。dozer 依赖仍走本地 uncommitted patch，正式 pin 见 Task 22。

### Task 21：迁移剩余通用 Host 路径并删除兼容桥

**修改：**Task 01 ownership 清单列出的 Dozer app/workspace/chrome/platform/runtime 路径与产品组合根；公共 desktop。

- [ ] 按应用 WebView → Todo/GroupChat → editor/preview 承载 → 通用窗口事件顺序逐组切到共享实现；每组独立提交和行为回归。
- [ ] 产品仍可持有业务 enum/state，但宿主不能保存产品专属字段、解析业务协议或 match 产品面板名。
- [ ] Project/Files 的预览业务与终端 PTY 留在产品，以 Surface/内容槽和通用焦点接入；删除重复几何/生命周期调度。
- [ ] 清除兼容桥台账，每项以无调用者、行为测试和单一实现证据关闭；不靠移动文件或放宽扫描目录降低门禁数字。
- [ ] 对 Dozer 新旧布局、面板激活、搜索/历史等产品命令、原生菜单与窗口退出做回归。

**验证：**同口径结构度量；公共 Host 反向依赖为零；兼容桥零；两产品使用同一 Host crate。提交按表面/容器切片，最后一次仅删除已无调用代码。

**实施进度（2026-10-12，切片 1–8）：** 切片 1:应用面板 Effect 执行器(`App::run_app_host_effects`)已外移出 `app/` 到 `crates/dozer-app/src/host_adapters/app_host.rs`,`R-HOST-EXECUTORS` 回退归零(基线未改)。切片 2:预览窗格选择统一——新增 `App::panel_in_either_view`,调用点改走 `Workspace::preview_pane[_mut]`,`R-PANE-PICK` 由 6 降至 2(仅访问器本体),基线只降(附带修正陈旧 `R-HOST-PANEL-ARMS` 95→93)。切片 3:**C8 收口**——删除产品 `crates/dozer-app/src/request_tracker.rs`(`RequestToken` 包装),面板请求身份改挂 core;bytehost 侧加中立 `ScopedTrackers`(`f50ad77`),dozer 侧新增 `panel_host::{scope_of,project_of,RequestTrackers,RequestTokenWrapExt}` 作唯一收口,7 个面板 `AsyncResult` 载荷改 `bytehost_core::request::RequestToken`,守卫改比 scope,`project_id` 不再搭在 token 上(`300c7dab`)。切片 4:**C12 收口**——bytehost 侧加中立的 `FocusRouter<K>`/`FocusTarget<K>`(`c80d443`),dozer 侧 `window_events.rs` 的 `pending_focus`/`current_focus` 两字段合并为 `FocusRouter<FocusIntent>`,`apply_pending_focus` 改为解算器+回落窗口,产品只留 `FocusIntent` 业务键与 `target()` 映射;新增门禁 `R-FOCUS-PRODUCT-KEYS`(基线 25,只降不升,`fa9f37a2`)。切片 5:**C11 收口**——bytehost 侧加中立的 `PanelId` 键控 rail/pane 容器 `bytehost-desktop::panes`(`PaneDims`/`PaneSpec`/`PaneStore`/`RailItems`/`apply_pane_split_drag`,`3dad53e`),dozer 侧新增 `PanelKind::stable_id()`(`dozer.files`…)/`panel_registry::panel_id`,把配对分割拖拽算术与 rail 栏位反查改挂宿主容器(`PanelDims::to_pane_store`/`apply_pane_store` 投影,命名字段保留为落盘兼容面),新增门禁 `R-PANE-KEYED-FIELDS`(基线 81,只降不升,`b50b5ca0`)。切片 6:**C9 收口**——面板内槽位词汇移入宿主,bytehost 侧加中立的 `bytehost-desktop::hover::HoverSlot`(`17c0736`),dozer 侧 `panel_host.rs` 删除本地 `HoverSlot` 改重导出该宿主类型(调用点零改动),退役 `R-HOVERSLOT-VARIANTS` 门禁规则并按棘轮下降基线(222/6→213/5,`5669776c`)。切片 7:**C7 收口(命名中立半边)**——`webview_geometry.rs` 三个几何函数改中立名(`content_pane_bounds_for`/`is_in_content_column`/`left_list_pane_bounds_for`,`0f9ecc44`),新增 `R-PREVIEW-NAMING` 门禁防旧名回退(`6ecadf22`);几何"上收宿主"依 Task 08 裁决不做。切片 8:**C10 原生浮层建窗机制上收设计成稿**(`1b1f858f`),含三方案对比(A feature-gated 整搬 / B 只搬无渲染器依赖的一半 / C B 起步续 A)与 `R-OVERLAY-DUP` 门禁形状,**推荐方案 B 先行,待用户裁决**(裁决点:是否要求门禁完全归零)。详见 ledger Task 21 切片 1–8。**C10 实现后 Task 21 全部桥收口;之后进 Task 22(依赖钉版 + 独立构建/发布打包)。**

### Task 22：固定依赖、独立构建与发布包

**修改：**三仓库 Cargo/锁文件、CI；Digger `packaging/macos/`、`scripts/package-macos.sh`（新增）。

- [ ] bytehost 完成测试后形成可发布提交，按会话授权发布新 tag；两个产品的所有 bytehost crate 使用同一 tag。
- [ ] Usage 库与 model/services 固定同一兼容来源；干净 clone 不依赖本地 patch、兄弟目录或未提交资源。
- [ ] 为 Digger 添加 macOS bundle、资源嵌入、产品标识与最小系统版本；构建 release，实际安装并冷启动。
- [ ] 记录实际支持架构；签名/公证依环境凭据落实，未完成不能写成已分发发行版。
- [ ] 用包含旧布局和旧应用数据的测试副本做升级/降级演练；未知格式拒绝覆盖，回滚不自动删除用户数据。

**验证：**A1/A18；发布包可显示 Web 内容与运行应用，退出无残留。未经实际发布的结果称候选包。提交 `build(digger): pin shared host and package desktop`。

### Task 23：最终验收与文档收口

**修改：**`acceptance.md`；Digger ADR-001/011、技术架构；bytehost README；Dozer 提取路线图。

- [ ] 按 §5 覆盖表跑 A1–A18，记录 pass/fail/not-run，附三仓库 revision/tag、命令和证据路径。
- [ ] GUI 人工项记录实际鼠标、焦点、浮层、IME、DPI 与截图；测试中直接发送消息不能代替真实点击。
- [ ] Project 导出/移动、真实 Usage 生产者或系统权限行为有未验项时明确列出，整体不得标“完全接入”。
- [ ] 修正 README 的 Tauri 等旧描述；ADR 仅在其全部必需验证通过后标 Accepted。
- [ ] 总结新增公共 API、剩余产品代码、兼容格式和回滚方式。关闭全部兼容桥与未决验收项后，主计划方可完成。

**产出：**可审查的最终验收报告。提交 `docs(integration): record Digger bytehost acceptance`。

## 4. 验证命令与运行边界

以下为实施时命令，不是本次已运行结果；在对应仓库执行。新增包的命令在创建包之后才可运行。

**bytehost 常规检查：**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
bash scripts/check-deps.sh
```

**bytehost 现有真实进程用例：**

```bash
cargo test -p bytehost-apps --all-features --test process_apps_live -- --ignored
cargo test -p bytehost-client --all-features --test embedded_host -- --ignored
```

**Dozer 基线/最终回归：**

```bash
cargo fmt --all --check
cargo test --workspace --no-fail-fast
cargo clippy --all-targets
python3 scripts/audit/check_panel_boundary.py
python3 -m unittest discover -s scripts/audit -p 'test_*.py'
bash scripts/check-log-scope.sh
```

中间切片只跑受影响测试与必需门禁；例如 `cargo test -p dozer-app usage`、`cargo test -p dozer-app request_tracker`，不在没有新风险时重复全量测试。Usage 迁库后改为对应新包测试，并记录旧测试名称的迁移。

**Digger 正式 workspace 创建后：**

```bash
cargo fmt --all --check
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check-host-deps.sh
cargo build -p digger-desktop --release
```

**Usage Web 工程在实际资源目录执行：**

```bash
npm ci
npm run typecheck
npm test
npm run build
```

窗口/进程夹具须设超时、隔离数据根与可追踪进程身份，报告中剔除 launch token。无 GUI runner 时明确 not-run，移交可用 macOS 环境执行，不以编译成功代替。网络依赖下载、仓库写入和 GUI 启动遵守执行环境权限，不绕过限制。

## 5. 规格验收与任务覆盖

| 规格项 | 负责 Task | 最终证据 |
|---|---|---|
| A1 独立构建 | 04/11/22 | 干净 clone 构建与依赖闭包 |
| A2 单面板运行 | 07/19 | fake 单面板真实窗口 |
| A3 请求身份 | 05/06/08/13 | 乱序、scope、实例重建测试 |
| A4 布局 | 04/10 | 黄金文件与空栏真窗口 |
| A5 Surface 生命周期 | 08/09 | 原生实例/快照/旧 IPC 测试 |
| A6 几何与浮层 | 02/10/16 | resize/DPI/点击/焦点记录 |
| A7 三种运行时应用 | 14/16 | 安装到交互再重启的真实流程 |
| A8 版本与卸载 | 15/17 | 升级故障、回滚与两种重装 |
| A9 运行时问题 | 15 | 失败与恢复持久状态页 |
| A10 鉴权/导航 | 16 | Gateway + WKWebView 行为探针 |
| A11 IPC/权限 | 16 | 对抗页面与脱敏检查 |
| A12 存储隔离 | 17 | 两 app、跨重启、删除数据 |
| A13 订阅与迟到结果 | 13/15 | 断线重订阅、Resync、乱序测试 |
| A14 退出/恢复 | 12/18 | 正常/强杀/双启动进程证据 |
| A15 两产品并行 | 18 | 独立根/端口/存储观察 |
| A16 Project 隔离 | 11/18 | 无 Project 窗口和真实导出/移动 |
| A17 真实 Usage | 20 | 真生产事件→账本→UI，跨重启 |
| A18 发布包 | 22 | 实际包安装、冷启动、资源和退出 |

## 6. 首批执行范围

先执行 Tasks 01–03，交付当前基线、可编译的双产品契约探针、真实窗口证据及 AppService/Usage 缺口决策。它们通过后进入 Tasks 04–06；此时不必等待 Digger 全部业务具备，也不提前宣称共享桌面宿主已完成。
