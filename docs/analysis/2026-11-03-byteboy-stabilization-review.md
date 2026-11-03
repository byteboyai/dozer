# ByteBoy 稳定化收口审核与修复结果

日期：2026-11-03。结论：**R1–R4 已修复；三个共享库标签已发布，Digger 已切换到真实 git 来源并完成 locked 验证。** GUI/真实 agent/性能验收仍按稳定化总计划保留。

## 审核对象

- 本地基线：ByteUI `eb08e57`、ByteHost `0362608`、ByteGit `5472fa5`、Dozer `6e389a30`、Digger `44b84e7`。
- Dozer 工作树保留此前的门禁、规范、文档与补丁包修改；本轮新增 CI 正式门禁调用和自检入口修正。
- Digger 已落地 `digger.patch` 的依赖门禁、实机自检入口、两组脚本测试、consumer-check workflow、正式版本清单与 git 来源 lock，并推送到 main `e82c048`。
- 远端已发布并验证 `v0.5.2`、`v0.4.22`、`libs-2026-11-03a`。

## 修复状态

| 编号 | 原优先级 | 当前状态 | 证据 |
|---|---|---|---|
| R1 | P1 | 已修复 | Dozer CI 同时运行正式门禁与 `--self-test` |
| R2 | P1 | 已修复 | Digger 单图门禁落地，15 项自检通过；失败不再被吞掉 |
| R3 | P1 | 已修复 | 三个标签已远端发布；Digger 正式 git 来源锁与 locked 构建通过 |
| R4 | P2 | 已修复 | Digger 实机自检改用本次 compiler-artifact；4 项测试通过 |

## 逐项结果

### R1 · Dozer CI 的 self-test 跳过真实门禁

- `.github/workflows/consumer-check.yml` 现在先运行 `bash scripts/check-host-deps.sh`（真实 workspace 锁定图检查），再单独运行 `bash scripts/check-host-deps.sh --self-test`（隔离回归）。
- 自检入口仍保持独立，可在带个人 path patch 的工作区运行；CI 不再只凭 13 项 fixture 单测通过。
- 现有 `test_new_business_members_are_checked` 验证普通门禁会拒绝新增业务 crate 的 `wry` 依赖；13 项自检通过。
- 本机 Dozer 直接 `bash scripts/check-host-deps.sh` 仍会因个人 bytegit/byteui path patch 与 `--locked` 冲突而失败；这是本机配置噪声，正式 CI/干净副本不受影响。该失败不应被记为门禁通过。

### R2 · Digger 门禁吞掉 cargo 失败

- 已将 `digger.patch` 的 `scripts/check-host-deps.sh` 与 `scripts/tests/test_host_deps.py` 落到 `../digger` 工作树。
- 新门禁读取单次 `cargo metadata --locked --all-features` 图，cargo 失败、坏 JSON、空成员、传递/可选/构建/跨平台依赖均失败；业务包默认纳入，`digger-paths` 的纯 bytehost-apps 身份契约是唯一窄例外。
- 复测：`python3 scripts/tests/test_host_deps.py` → **15 passed**。
- 正式锁更新后，原仓库直接运行门禁已通过（审视 3 个业务 crate），不再是旧脚本的假通过。

### R3 · Digger 正式依赖组合与锁

已修复：

- 远端已发布 ByteUI `v0.5.2`（commit `e33824578d0f6fa10f168657fe08d882ad6fadc4`）、ByteHost `v0.4.22`（commit `729fcaffe3dc71d9ab40c8b04c9cc99d2b9f8b3e`）、Dozer `libs-2026-11-03a`（commit `e019a56fb53bd9ab97eae0ac7c8152376b40b501`）。
- Digger `Cargo.toml` 已统一到上述标签；正式 `Cargo.lock` 由无个人 `.cargo` patch 的解析生成，共享包均带 git source。
- `cargo check --locked --offline --workspace --all-targets` 通过；`digger` workspace lib 测试 39/726/47/13/56 全部通过。
- Digger main 已推送到 `e82c048d8ba81ad507524335e5667bbee7472a1a`。

### R4 · Digger 实机自检使用错误缓存产物

- 已将 `digger.patch` 的 `scripts/run-editor-selftest.sh` 与 `scripts/tests/test_editor_selftest.py` 落到 `../digger` 工作树。
- 脚本现在只使用本次 Cargo compiler-artifact 的 executable，不按跨 target 路径 mtime 猜产物；构建失败/无产物明确失败，父进程超时后回收测试子进程。
- 复测：`python3 scripts/tests/test_editor_selftest.py` → **4 passed**。
- 这不等同于真实窗口/IME/焦点/绘制验收；GUI 仍按计划 P7 待执行。

## 验证与限制

| 探针 | 结果 |
|---|---|
| Dozer `--self-test` | 13 passed |
| Dozer CI 源码 | 正式门禁与自检两步均已配置 |
| Digger host-deps 自检 | 15 passed |
| Digger editor-selftest 自检 | 4 passed |
| Digger 正式 locked 构建 | 通过（git 来源锁，无个人 patch） |
| 新发布标签 | 三个远端标签均已验证 |

本轮完成了标签发布、Digger 正式锁和 locked/核心测试；完整 GUI、真实 agent 与发行性能验收仍待执行。Dozer/Digger 发布提交已推送，标签已远端验证。

## 远端 CI 与最终提交

| 对象 | 最终提交 | CI / 说明 |
|---|---|---|
| ByteUI `v0.5.2` | `e33824578d0f6fa10f168657fe08d882ad6fadc4` | CI run 41572390741：success |
| ByteHost `v0.4.22` | `729fcaffe3dc71d9ab40c8b04c9cc99d2b9f8b3e` | CI run 41572677875：success；joint-build 41573636762：success |
| Dozer `libs-2026-11-03a` | `e019a56fb53bd9ab97eae0ac7c8152376b40b501` | Consumer check 41573001561、`9da10a82` 的 41573499793、`c26b7174` 的 41574640539：success |
| Digger main | `e82c048d8ba81ad507524335e5667bbee7472a1a` | Consumer check 41573423472：success |
| ByteGit | `5472fa553ab34d97d9a1e4055c89d03b48eb4848` | 本批未改版本 |

- ByteHost tag 首次 joint-build 41572677891 失败：当时 Digger main 仍引用 ByteUI v0.5.1，出现两份 `IconKind`。Digger 更新到已发布 ByteUI/ByteHost/Dozer 标签后，手动重跑 41573636762 已成功。
- 两产品正式 lock 中的共享来源：ByteUI `v0.5.2#e3382457`、ByteHost `v0.4.22#729fcaff`、Dozer `libs-2026-11-03a#e019a56f`、ByteGit `v0.9.2#5472fa55`。
- Dozer/Digger 工作树在发布提交后均为 clean；个人 `.cargo` patch 未进入提交。

## 剩余工作

1. 真实 GUI/真实 agent 验收：Dozer 持续终端/恢复/退出、Todo 失败重试、回填状态；Digger IME/undo/焦点/保存恢复/Retry。
2. 发行性能验收：固定 fixture 下的输入到绘制、终端停消费资源、摄取期间 RPC p95/p99、首次可连接/停止耗时。
3. 完整历史二进制组合与外仓消费者升级；当前只验证了最新发布组合和契约/golden 基线。
4. 历史产品库两次启动/迁移演练与真实权限环境的 FSEvent/进程身份复验（本机已通过，仍需按实际发行环境记录）。
