# ByteBoy 稳定化收口审核与修复结果

日期：2026-11-03。结论：**R1、R2、R4 已修复并复测；R3 仍被未发布的 ByteUI/ByteHost/Dozer 标签阻断，不能通过收口审核。** 本轮修改了 Dozer CI 与 Digger 工作树，未创建提交、推送或发布标签。

## 审核对象

- 本地基线：ByteUI `eb08e57`、ByteHost `0362608`、ByteGit `5472fa5`、Dozer `6e389a30`、Digger `44b84e7`。
- Dozer 工作树保留此前的门禁、规范、文档与补丁包修改；本轮新增 CI 正式门禁调用和自检入口修正。
- Digger 工作树已应用 `digger.patch` 中选择的文件：依赖门禁、实机自检入口、两组脚本测试、consumer-check workflow 与稳定化 review 记录；**Cargo.toml/Cargo.lock 未修改**。
- 远端仍没有 `v0.5.2`、`v0.4.22`、`libs-2026-11-03a` 这些预留标签。

## 修复状态

| 编号 | 原优先级 | 当前状态 | 证据 |
|---|---|---|---|
| R1 | P1 | 已修复 | Dozer CI 同时运行正式门禁与 `--self-test` |
| R2 | P1 | 已修复 | Digger 单图门禁落地，15 项自检通过；失败不再被吞掉 |
| R3 | P1 | 仍阻断 | Digger 正式 Cargo.toml/lock 仍指向旧版本；新标签未发布 |
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
- 当前原仓库直接运行门禁会以“Cargo.lock 需要更新”失败，而不是旧的假通过；这对应未解决的 R3。

### R3 · Digger 正式依赖组合与锁仍不可发行

仍阻断，未用本地 patch 或伪造锁掩盖：

- `../digger/Cargo.toml` 仍引用 ByteUI `v0.5.1`、ByteHost `v0.4.21`、Dozer `libs-2026-11-01e`，而源码已使用 NotebookTabs、`SaveOutcome`、`SaveFile.project_root` 等新接口。
- 远端尚未发布包含这些接口的 ByteUI/ByteHost/Dozer 标签；`Cargo.lock` 仍缺共享包 git source。
- `cargo check --locked --offline -j 1 --workspace --all-targets` 仍因 lock 需要更新而失败。修复必须按 P3–P6 顺序发布真实标签、更新消费方清单并重新生成正式 git 来源锁；不能把候选 path 图或补丁包通过当作发行证据。

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
| Digger 正式 locked 构建 | 仍失败：Cargo.lock 需要更新（R3） |
| 新发布标签 | 远端仍未出现 |

本轮没有重新执行完整 Rust 编译、完整 GUI 或真实性能验收；R3 未解决前这些发行验收不应被宣称通过。Dozer 与 Digger 的改动仍在各自工作树中，未提交、未推送、未发 tag。

## 剩余阻断

1. 发布包含所需 API 的 ByteUI、ByteHost 与统一 Dozer libs 标签，并在消费方清单中统一版本。
2. 在无个人 `.cargo` patch 的干净副本生成正式 git 来源 lock，运行两产品 locked 全目标、契约与边界门禁。
3. 完成 P7 的真实 GUI/真实 agent/性能验收；脚本自检和候选 path 编译不能替代。

## 后续复核：脚本修复通过，标签发布仍阻断

重新检查当前工作树：R1 的 Dozer CI 已分别调用正式门禁和隔离自检；R2/R4 的 Digger 单图门禁、compiler-artifact 选择和父进程超时回收已落到实际工作树。实际重跑 Dozer 13 项、Digger 门禁 15 项、实机入口 4 项测试，全部通过。上述三项不再列作当前未修复问题。

R3 仍未解阻。重新查询三个官方远端的精确 tag（含 annotated tag 的剥离引用），v0.5.2、v0.4.22、libs-2026-11-03a 均无结果；ByteUI 首次查询遇到 SSL 错误，单独重试成功后仍无该 tag。两产品清单仍消费 ByteUI v0.5.1、ByteHost v0.4.21，Digger 同源 Dozer crates 仍为 libs-2026-11-01e。ByteUI package version 也仍是 0.5.1。

因此本次可以确认脚本修复完成，不能确认共享库标签发布专项完成；仍需实际发布正确提交、更新消费者和正式锁，并完成干净 locked 验证。未重复运行未变化的全量构建，也没有将缺失标签状态标为通过。
