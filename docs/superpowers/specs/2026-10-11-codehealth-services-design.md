# CodeHealth 面板迁移到服务 trait(第六个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-09-dozer-services-x0-design.md` 的 Usage `LoadFailed` 先例、`2026-10-10-todo-services-design.md`(读失败分档口径)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前几个面板):** 代码健康度面板可进程内测试/复用——落盘读写只依赖小 trait,测试注入内存实现。
**范围:** 只迁 `extensions/codehealth/aggregate.rs` 里的 3 个服务调用及其调用链:
- `spawn_load_cached`(面板切入时读落盘缓存,经 `on_activate` 用 `ctx.io.client()`);
- `spawn_scan`(手动扫描:读上一份报告 → 本地扫描 → 保存),host 入口是 `Workspace::spawn_codehealth_scan`(`workspace/state.rs`,拿 `ShellIo`)。

**不变量:** 线上协议零变化;`CodeHealthReportInfo` 字段与 `report_json` 的编解码不变;**保存失败的内联提示文本逐字不变**("未保存，无法用于下次比较：{e}",面板自带的持久 `save_error`/`scan_error` 状态不动)。

**用户已确认的失败口径(用户可见的行为变化):**
- 缓存读取(`get`)失败:已有报告时**不覆盖**现有面板;从未加载过则采用"无报告"面板(与现状一致)。`Unavailable` 静默;其余弹带 key 的 Error Toast,相同文本 60 秒内只提示一次。
- 历史读取(`list`)失败(缓存读取和扫描里各一处):保持现行为(对比降级为无基线、扫描照常),但不再无声吞掉,改写一条 warn 日志;不弹 Toast。

## 2. 现状(读代码得到)

- `spawn_load_cached(project_id, project_path, client, handle, emit)`:`get_code_health_report(..).ok().flatten()` 与 `list_code_health_reports(.., 2).ok().unwrap_or_default()`——任何读失败都被当成"没有报告/没有历史";随后无条件在 `spawn_blocking` 里 `finish_panel(...)`(采集 git 元数据与热点)并发 `Message::Loaded(project_id, Box<PanelState>)`。`update` 的 `Loaded` 分支整体替换 `ws_state.panel`——**一次读失败会把已显示的报告替换成"尚未扫描"的空面板**。
- `spawn_scan`:先 `list_code_health_reports(.., 2).ok().unwrap_or_default()` 取上一份做对比(失败等于无基线,用户看不出本次为何没有对比);本地扫描后 `save_code_health_report(..)`,失败被格式化进 `panel.save_error`("未保存，无法用于下次比较：{e}"),已经可见。
- host 路由(`app/update.rs` 约 1653–1662 行):`Loaded` 与 `Scanned` 各一个专属臂,`with_project(project_id, …)`。
- `WorkspaceState` 没有 `outbox`;日志来源 `LOG`(`panel, "codehealth"`)需确认已声明(计划阶段核对)。
- `R-CLIENT` 基线:`extensions/codehealth/aggregate.rs` 为 3(`spawn_load_cached`/`spawn_scan` 的参数类型与 `on_activate` 的 `ctx.io.client()`)。

## 3. 设计

### 3.1 `CodeHealthService`(放进 `dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `get_code_health_report(project_id: i64)` | `Option<CodeHealthReportInfo>` |
| `list_code_health_reports(project_id: i64, limit: u32)` | `Vec<CodeHealthReportInfo>` |
| `save_code_health_report(project_id: i64, info: CodeHealthReportInfo)` | `()` |

参数按值传(`info` 按值,适配层再借用),返回 `BoxFuture<'_, Result<_, ServiceError>>`。`Services` 增加 `codehealth: Arc<dyn CodeHealthService>`;`Client` 实现该 trait(调用同名 inherent 方法,`map_err(into_service_error)`)。

### 3.2 面板与 host

- `spawn_load_cached`/`spawn_scan` 改吃 `Arc<dyn CodeHealthService>`;`on_activate` 用 `ctx.io.services().codehealth.clone()`;`Workspace::spawn_codehealth_scan` 用 `services_for(&io.client).codehealth`。
- **缓存读取失败:** `get` 返回 `Err(e)` 时,仍照旧用"无报告"计算一份 `PanelState`(含 git 数据),但发新消息 `Message::LoadFailed(project_id, ServiceErrorKind, String, Box<PanelState>)`(同 Usage 的 `LoadFailed` 携带 git 数据的先例);`get` 成功则发 `Loaded`(包括 `list` 失败时:`previous = None`,写 warn 日志)。
- `update` 的 `LoadFailed` 分支:若 `ws_state.panel.report` 为 `None`(从未成功加载过),采用携带的 panel(与今天一致);否则**不动现有面板**。再按 `kind` 分档:`Unavailable` 只写 debug 日志;其余 `outbox.push_keyed(LOG, Error, "代码健康度加载失败: {message}", "codehealth-load")`,用 `last_load_failure: Option<(String, Instant)>` 做 60 秒同文本去重(`LOAD_FAILURE_TOAST_COOLDOWN`,与 Todo/Project/Conversations 同值,各模块一份,见 §6)。
- `WorkspaceState` 增加 `outbox: toast::Outbox`、`last_load_failure` 与 `take_outbox`;`App::drain_outboxes` 加一行排空。
- host 路由:`LoadFailed` 并入已有 `Loaded` 的臂(`msg @ (codehealth::Message::Loaded(project_id, ..) | codehealth::Message::LoadFailed(project_id, ..))`),**不新增面板专属臂**(`R-HOST-PANEL-ARMS` 不上升)。
- **保存失败**:`save_code_health_report` 的 `Err(e)` 仍格式化为 `"未保存，无法用于下次比较：{e}"` 进 `panel.save_error`(`e.message` 与原 `anyhow` 文本相同)。
- **`list` 失败**(扫描里):`dozer_core::log_warn!(LOG, error = %e, "读取上一份健康度报告失败,本次扫描不含对比")`,`previous = None`,继续扫描。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **H0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `codehealth/aggregate.rs`(3)、门禁输出。
- **H1 `CodeHealthService` trait:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与"没有报告"区分)。
- **H2 `Client` 实现:** 回放服务端适配测试(`get` 成功且 `Option` 原样带回、`list` 成功、`save` 返回 `()`、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`)。
- **H3 `Services.codehealth` + 假实现:** `services_for` 与 `base_services()` 补字段;进程内 `FakeCodeHealth`(预设 get/list 结果、可让 save/get/list 分别失败、记录调用)。
- **H4 面板与 host 切换:** 面板层测试(`FakeCodeHealth`,临时目录当项目路径):
  - 缓存读取:成功得到 `Loaded`;`get` 失败得到 `LoadFailed`(携带 panel);`list` 失败仍得到 `Loaded`。
  - `update`:首次失败采用携带的 panel;已有报告时失败保留旧 panel;`Unavailable` 不弹 Toast;`Rejected` 弹一条带 key `codehealth-load` 且 60 秒内同文本不重复、换文本再弹。
  - 扫描:`save` 失败时 `panel.save_error` 文本逐字为 "未保存，无法用于下次比较：{服务文本}";`list` 失败不阻断扫描(仍得到 `Scanned(Ok(..))`)。
  - 探针 `host_site` 行号因 `drain_outboxes` 多一行整体 +1,只改行号。
- **H5 门禁与验收:** `R-CLIENT` 基线中 `codehealth/aggregate.rs` 清零;回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project、GroupChat、Conversations、CodeHealth 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/codehealth/**` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在这些文件为 0,基线只降。
3. 缓存读取三条路径、`update` 的失败分档、扫描的保存失败文本与 `list` 失败不阻断,均有测试。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、间歇的 `git_log::build_*`、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不改 `scan_error`/`save_error` 这类持久状态字段与其展示;不改本地扫描/差异/热点/Git 采集逻辑。
- 不给 `Loaded`/`Scanned` 补代数保护(慢的旧结果可能覆盖新结果,同其它面板的既有问题,登记为后续项)。
- 不把冷却逻辑抽到 `toast::Outbox`(Todo、Project、Conversations、CodeHealth 各一份,抽取另立项)。
- 不对齐 Usage 的失败口径;不改线上协议;不动 `PanelIo::client()`。

## 7. 风险与已知问题

- **用户可见行为变化:** 已有报告时缓存读取失败不再被"尚未扫描"覆盖(原来会覆盖)。这是修缺陷,用户已确认。
- **`PanelState` 较大**:`LoadFailed` 与 `Loaded` 一样装箱携带,避免撑大 `Message` 枚举。
- **探针行号漂移:** `drain_outboxes` 多一行,`usage_probe/host.rs` 的 `host_site` 引用整体 +1,只改行号。
- **测试涉及真实文件系统/Git:** `finish_panel` 会采集项目路径的 git 元数据;面板层测试用临时目录(非 git 仓库也应能得到 `PanelState`,`git_snapshot` 对非仓库返回空/None),计划阶段先核对该行为再写断言。
- **WIP 文件:** 用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动,`Cargo.lock` 剔除 bytegit/byteui 的 `source =` 行。

## 8. 分支与执行

规格分支 `spec/codehealth-services`(worktree `../dozer-codehealth-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
