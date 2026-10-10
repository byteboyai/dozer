# Settings 对话框迁移到服务 trait(最后一个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-11-project-create-services-design.md`(同形的纯接缝迁移)、`2026-10-09` X0 接缝规格、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标:** 设置弹窗(`extensions/settings.rs`)不再直接持有 `dozer_client::Client`,可进程内测试。完成后 `extensions/**` 对 `dozer_client` 的引用整体归零。
**范围:** 该文件里 `update(.., client: &dozer_client::Client, ..)` 与 `run_apps_message(.., client, ..)` 两处,以及 host 在 `app/update.rs` 的 1 个调用点。
**明确不在范围:** host 的 `run_app_host_effects`/`window_events` 对应用宿主的调用(不在 `extensions/`,`R-CLIENT` 不计);线上协议;Git 账号连接(纯本地,不走 `Client`)。
**不变量:** 线上协议零变化;**用户可见行为零变化**——Advanced 页与「应用」页的失败本来就内联可见,所有文案与状态转移逐字不变。

**用户已确认的口径:**
- Advanced 页新增 `DaemonService`(3 个方法);其实现放在 `dozer-app`(因重启要拉起进程)。
- 「应用」页对 `bytehost_client::AppHostApi` 做泛型,不进 `Services`、不在 `dozer-services` 复制 12 个方法。

## 2. 现状

- Advanced:`AdvancedStopClicked` → `client.list()` 后经纯函数 `count_live_sessions` 统计存活会话 → `AdvancedSessionCountReady(Result<u32,String>)`;`AdvancedStopConfirm` → `client.shutdown_daemon()` → `AdvancedStopResult(Result<(),String>)`;`AdvancedRestartClicked` → `runtime::ensure_daemon(&client)`(`list` 不通则 `spawn_dozerd`,隔 1 秒重试 3 次,错误为 `String`)→ `AdvancedRestartResult(Result<(),String>)`。错误文本现为 `e.to_string()`(前两者)与 `ensure_daemon` 原样 `String`。
- 「应用」页:`run_apps_message` 把纯状态机 `s.apps.update` 吐出的 `Effect` 逐个 `handle.spawn`,调用 `AppHostApi` 的 `app_probe_runtimes/app_list/app_plan/app_install/app_stop/app_rollback/app_uninstall/app_runtime_plan/app_install_runtime/app_uninstall_runtime/app_logs`。`AppId::new` 失败走 `Failure::Transport` 且不发请求。
- `AppHostApi` 只要求实现 `request`、`subscribe`,其余方法有默认实现,返回 `impl Future + Send`,**不是 dyn 兼容**。
- `R-CLIENT` 基线:`extensions/settings.rs` 为 2。

## 3. 设计

### 3.1 `DaemonService`(`dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `list_sessions()` | `Vec<SessionInfo>` |
| `shutdown_daemon()` | `()` |
| `ensure_running()` | `()` |

返回 `BoxFuture<'_, Result<_, ServiceError>>`。`Services` 增加 `daemon: Arc<dyn DaemonService>`。`SessionInfo` 已在 `dozer-model`。

### 3.2 实现在 `dozer-app`,不在 `dozer-client`

`ensure_running` 要拉起 `dozerd` 进程(`runtime::spawn_dozerd`),`dozer-client` 够不着。故在 `dozer-app` 新增 `ClientDaemon(Client)`(`panel_host.rs` 内或同级小模块):`list_sessions`/`shutdown_daemon` 调 `Client` 的 inherent 方法并 `map_err(dozer_client::into_service_error)`;`ensure_running` 调 `crate::runtime::ensure_daemon(&self.0)`,把其 `String` 错误包成 `ServiceError::new(Unavailable, msg)`(文本不变)。`services_for` 里 `daemon: Arc::new(ClientDaemon(client.clone()))`。

面板侧所有错误统一 `.map_err(|e| e.message)`(`Display` 即 message,与原 `e.to_string()` 一致;重启文本与原 `String` 相同)。

### 3.3 `settings::update` 与「应用」页

```rust
pub fn update<A>(
    state: &mut Option<State>, msg: Message,
    daemon: Arc<dyn DaemonService>, apps: A,
    handle: &Handle, emit: impl Fn(Message) + Clone + Send + 'static,
) where A: AppHostApi + Clone + Send + Sync + 'static
```

`run_apps_message<A>` 同样泛型,`client.clone()` 换成 `apps.clone()`。host 调用点传 `services_for(&self.client).daemon` 与 `self.client.clone()`。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **S0 基线:** 记录测试数、clippy 数、`R-CLIENT` 中 `settings.rs`(2)、门禁输出。
- **S1 `DaemonService` trait + `Services.daemon`:** 契约测试(内存实现、`tokio::spawn` Send、失败可与空会话区分)。
- **S2 `ClientDaemon` 适配:** `list_sessions`/`shutdown_daemon` 的回放服务端测试(成功、断开 → `Unavailable`、`ErrorReply` → `Rejected`);`list_sessions` 对不存在的 socket 得 `Unavailable`;**不测 `ensure_running`**(会真拉起 dozerd);`services_for` 补字段。
- **S3 假实现:** `FakeDaemon`(预设 `list_sessions`/`shutdown`/`ensure` 结果,记录调用,`base_services` 补 `daemon`);`FakeApps`(实现 `request`/`subscribe`,按 `AppRequest` 回预设 `AppReply`,未预设的变体返回 `AppApiError::Transport("未预设")`,记录请求)。
- **S4 切换:** Advanced 三条路径经 `FakeDaemon` 的测试(只数 `alive`;停止确认调 `shutdown_daemon`;重启调 `ensure_running`;三条失败文本逐字不变;状态前置条件不满足时不调服务);「应用」页经 `FakeApps` 的测试(`Opened` 发 `ProbeRuntimes` 与 `List`;`Stop` 非法 id 得 `Failure::Transport` 且不发请求;`Install` 把批准计划与来源原样送入请求;结果消息送达)。host 调用点切换;现有纯状态测试不动;探针 `host_site` 行号如漂移只改数字。
- **S5 门禁与验收:** `R-CLIENT` 中 `settings.rs` 清零(此后 `extensions/**` 全部为 0);回填 `08-client-use-inventory.md`;`CLAUDE.md` 的 `dozer-services` 行改为"全部面板已迁"。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/**` 不再含 `dozer_client`;`R-CLIENT` 全为 0,基线只降。
3. Advanced 三路径与「应用」页关键路径有测试(S4)。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、并行负载间歇失败、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh`、`model_golden` 通过。

## 6. 明确不做

- 不改 host 的应用宿主调用、`project_tab_opened`、线上协议;不动 Git 账号连接;不把 `ensure_daemon` 搬进 `dozer-client`。
- 不在 `dozer-services` 里镜像 `AppHostApi`。

## 7. 风险

- **泛型贯穿 `settings::update`:** host 只有一处调用,编译器会指出全部位置;`A` 的默认方法 future 带 `Send` 约束,可进 `handle.spawn`(计划阶段 S4 首次编译确认,不成立则记 Ruling 并改约束)。
- **假 `AppHostApi` 变体多:** 未预设变体返回显式错误,避免静默通过。
- **`ensure_running` 会拉起进程:** 所有测试用 `FakeDaemon`;`ClientDaemon::ensure_running` 不测。
- **`Cargo.lock` 噪声 / WIP 文件:** 同前几次,提交前 `git checkout Cargo.lock`,不提交 `Info.plist`/版本号改动,在独立 worktree 实施。

## 8. 分支与执行

规格分支 `spec/settings-services`(worktree `../dozer-settings-spec`);实施另开 `feat/settings-services`。合并进 main 前需询问用户。
