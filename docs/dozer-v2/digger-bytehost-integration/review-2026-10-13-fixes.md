# 修复复审（2026-10-13）

结论：部分修复通过，整体仍未通过。复核 Digger `71280a7..1fd286c` 的 4 个提交、5 个变更文件，以及前次 9 项问题的当前生产调用链。bytehost 仍为 `7f25d71`。

## 本次修复中的问题

### F1 / P2：真实启动失败不会进入新增重试分支

位置：`digger/crates/desktop/src/apps/mod.rs:454`；`digger/crates/app-host/src/effects.rs:575`；`digger/crates/desktop/src/apps/management.rs:67`。

重试按钮仅在 `Startup::Unavailable` 显示，新增复位逻辑也只放在 `Outcome::ServiceFailed`。但真实 `Executor::start_service` 总是发送 `ServiceReady`：`InProcess::start` 将目录、端口、同根锁故障封装为不可用服务，而不是返回启动错误。`service_ready` 随后无条件标 Ready，列表查询才返回 Unavailable，因此故障时看不到重试按钮，opened 也没有复位。新增测试手工注入了生产者从不发送的 ServiceFailed，不能覆盖真实路径。

已用真实 Executor + InProcess，以 `/tmp` 中的普通文件作为非法服务根复现：实际收到 ServiceReady，同时 app_list 确认失败；“失败应进入 ServiceFailed”的断言失败。应在启动结果交付前判断服务可用性，或由桌面将不可用的 ServiceReady 转入失败态，确保真实错误能驱动重试界面。

### F2 / P2：显式重试把 opened 留为 false，导致成功后漏刷新及重复启动

位置：`digger/crates/desktop/src/apps/mod.rs:327`、`:561`。

RetryService 将 opened 置 false 后直接调用 start_service，没有像 Opened 分支那样置 true。成功回包到 service_ready 时，补拉探测/列表的分支要求 opened 为 true，因而被跳过；重试时提前派发的查询可能仍访问旧的不可用后端。之后切走再回应用页又满足“尚未打开”，再次启动同根服务，可能以同根锁冲突产生的不可用实例替换刚恢复的可用后端。

应统一首次启动和重试的状态迁移：标记启动进行中、禁止第二次 genesis、等待真实可用后再刷新。测试需覆盖失败→重试→成功→切走再回来，断言不再发第二次启动。此项由生产状态分支确认；不冒充真实窗口复现。

## 前次问题逐项结论

| 前次编号 | 结果 | 当前证据 |
|---|---|---|
| 1：run_id 重启覆盖 | 修复通过 | 加入 boot_id 命名空间；实际桌面回归测试保留两条记录 |
| 2：正常退出未 shutdown | 未修复，P1 | `product.rs:374` 仍直接 save + iced::exit；桌面无 begin_shutdown/await_shutdown 调用 |
| 3：应用启动入口/视口 | 未修复，P1 | actions、main 未变；surface_task/surface_open_pending 仍无生产调用者 |
| 4：Usage 不显示内容 | 未修复，P1 | adapters/usage.rs 未变，仍仅原生 content_pane，无实际 Usage WebView |
| 5：浏览器数据清理未执行 | 未修复，P1 | store_removal_task 无生产调用者；store_take_ready 只在测试中消费 |
| 6：订阅及列表轮询 | 未修复，P2 | 桌面无 subscribe/change_to_message 调用，产品 subscription 仍只监听关窗 |
| 7：服务失败不能重试 | 未修复完整，P2 | 见本次 F1/F2；新增按钮与状态测试未覆盖真实故障 |
| 8：通知丢弃 | 修复通过 | 桌面消费 Notice，管理视图显示且可关闭；定向测试通过 |
| 9：损坏账本临时兜底 | 修复通过 | 查询返回 Unavailable、不接生产者、原文件保持不变；定向测试通过 |

因此前次 9 项中关闭 3 项，6 项仍未关闭；并在第 7 项修复中发现 F2 状态回归。

## 验证

- Digger 根执行 `RUSTC_WRAPPER= CARGO_TARGET_DIR=/tmp/digger-review-20261013-target cargo test --locked --offline -p digger-store -p digger-app-host --lib`：39 + 16 = 55 项通过。
- `cargo test --locked --offline -p digger-desktop --no-run`（同样禁用 wrapper、使用临时 target）：编译成功。
- 对生成的桌面测试二进制单独执行三项：`distinct_ops_across_restarts_do_not_overwrite_each_other`、`a_notice_effect_becomes_a_visible_notice`、`a_corrupt_usage_ledger_is_unavailable_and_left_untouched`：3 项通过。
- `/tmp/digger-review-retry/src/lib.rs`：真实执行器失败启动定向测试失败，确认 F1。夹具只使用临时非法根，无 GUI、无真实用户数据操作。
- 未执行新增 `a_failed_service_start_can_retry`：该测试调用 update(Opened)/RetryService，会经 paths::bytehost_root 启动真实用户目录的服务；传入的 temp_ledger 仅隔离清理台账，没有隔离服务根。建议改为注入 fake 或独立服务根，再等待并断言实际回包及收尾。
- 编译新增 `Notice.key` 未读取警告（apps/mod.rs:165），计划要求的 `clippy -- -D warnings` 也需要处理该警告；本次未运行完整 clippy。
- 未运行 GUI、桌面全量测试、打包或签名验收；未改生产代码、锁文件及用户 WIP。本次只新增审核报告和 `/tmp` 复现夹具。
