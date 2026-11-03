# ByteBoy 稳定化实施记录

日期：2026-11-03。对应 [收口计划](../superpowers/plans/2026-11-03-byteboy-stabilization.md)。

**状态：已完成可写范围内的修复及跨仓候选实现/验证，尚未完成发布和实机收口。** 当前会话文件系统仅允许写 Dozer 与 /tmp；ByteUI、ByteHost、Digger 的原仓库没有被修改。它们的实现保存在隔离 git 副本和 [可应用补丁包](../patches/2026-11-03-byteboy-stabilization/README.md)，需要可写的父级工作区继续落地。

## 已实施

- Dozer：门禁一次读取完整的 `cargo metadata --locked --all-features` 图，检查所有非白名单 workspace 成员的普通/构建依赖，拒绝命令失败、坏 JSON、空成员、传递 GUI 依赖与旧 lock；不把 dev-only 依赖误计入生产图。清除 CLAUDE 重复手写的 ByteHost 版本，实际版本以清单/锁文件为准。
- Digger 候选：相同的单图门禁，消除旧脚本 cargo tree 失败仍退出 0 的问题。所有新成员默认受检查。digger-paths 通过 dozer-core 的 ProductId/ProductDirs 使用 bytehost-apps 纯契约，此处为唯一窄例外，不允许 iced/wry 或其它宿主层。
- ByteHost 候选：test-only 子进程 RAII，panic/失败路径也 kill/wait；孤儿恢复用例等待有期限，继续保留原来的身份核对和 Killed/Stale 断言。联合检查必须包含两产品，缺目录即失败，成功/失败均恢复原锁；workflow 接入 Digger 与 PR。
- Digger 实机入口候选：从本次 Cargo compiler-artifact 取得唯一 bin executable，不按缓存 mtime 选择；构建失败/无产物必须失败；Python 父进程执行和超时回收代替会遗留 sleep/管道的 shell watchdog。
- 发布候选：ByteUI 0.5.2（NotebookTabs）、ByteHost v0.4.22、统一 Dozer libs-2026-11-03a；ByteGit 继续 v0.9.2。补丁不包含未实际发布的 git SHA 或伪造发行锁。

## 已验证

| 检查 | 结果 |
|---|---|
| Dozer 门禁单测 | 13 passed：失败/坏 JSON、空成员、旧锁不变、新业务、可选/构建/跨平台/传递依赖、dev-only 与适配器 |
| Digger 门禁单测 | 15 passed；另包含路径身份契约允许与 GUI 禁止 |
| 联合检查脚本 | 3 passed：缺消费方失败，成功/失败后 lock 恢复 |
| 实机自检入口脚本 | 4 passed：只运行本次产物，无产物/构建失败拒绝，超时子进程已回收 |
| ByteUI lib | 133 passed |
| ByteHost Apps lib | 485 passed / 3 failed；新增 panic 清理用例通过；同样的受限进程身份查询用例继续失败；全批由原 306.74s 降为 8.91s，不再等 sleep 306 自然结束 |
| 候选源码组合 | 两产品 workspace all-targets 编译通过；显式更新 ByteUI 0.5.2 后再次 locked 检查，最终结果见补记 |
| 补丁可应用性 | 四份补丁均对各自原仓库基线通过 git apply --check；基线与 SHA256 见补丁包 manifest |
| Dozer panel/log 边界、diff | 通过 |

脚本自检日志随补丁包保存；编译和原始 Apps 日志在 `/tmp/byteboy-stabilization-20261103`。没有把 3 个失败改为 ignored 或放宽断言，没有声称远端 CI 已运行。

完整图检查会读取跨平台依赖的元数据；本机 Cargo 缓存只读且缺 Windows pageant 包，因此该检查使用独立可写的临时 CARGO_HOME 取得缺失公共依赖，未修改个人配置。失败必须保持失败，不能把缓存/环境错误记成门禁通过。

## 发布与实机待办

1. 将补丁落到可写的 ByteUI/ByteHost/Digger 原仓库，按依赖顺序提交/发布；Dozer 最新本地运行时提交也要实际进入发布快照。已确认远端 Digger main 是 44b84e7，但远端 ByteUI/Dozer main 尚未含所需本地提交。
2. 从真正已发布的远端 tag 生成两产品正式 git 来源锁；无个人 patch 跑 locked 全目标、共享依赖及相关回归。当前候选使用临时 path 图，不能替代这一项。
3. 在正常进程身份查询权限环境复验 ByteHost 3 项恢复测试；在正常 FSEvent 环境复验 ByteGit 监听测试。
4. computer-use 检查到已安装的 Dozer 窗口，但没有本次候选版本的独立窗口，也没有运行中的 Digger。未在用户现有窗口中修改项目或开展破坏性验收。需使用临时产品根运行候选自检、IME/undo/焦点/保存恢复和终端持续输出/恢复/退出。
5. 采集实际发行构建的输入到绘制、GUI 停消费资源、摄取期间 RPC 与关闭耗时。脚本测试/编译不是这些性能结论。

本批尚未退出，不启动新的产品功能或大范围框架重构。

## 最终补记

显式将候选 ByteUI 0.5.2 纳入临时锁后，Dozer/Digger 的 `cargo check --locked --offline -j 1 --workspace --all-targets` 均退出 0；两产品单图门禁分别审视 8/3 个业务 crate，通过并完成 13/15 项自检。保留既有 unused/dead_code 与第三方 future-incompat 警告，不声称零告警。

机器可读结果见 [verification.json](../patches/2026-11-03-byteboy-stabilization/verification.json)。候选 package/path 与真实已发布的 git 来源明确分开；本轮未创建提交、推送、发 tag 或改写相邻仓库。

## 2026-11-03 复核补充

- 五仓库当前可写；HEAD 与计划第 2 节一致（ByteUI `eb08e57`、ByteHost `0362608`、ByteGit `5472fa5`、Dozer `6e389a30`、Digger `44b84e7`）。Digger 仍未配置 remote。
- 四份补丁 SHA256 与 manifest 一致，四个仓库的 `git apply --check` 均通过。
- 修正 Dozer `check-host-deps.sh` 的自检入口顺序：`--self-test` 现在先进入隔离 workspace，不再依赖当前仓库带个人 path patch 的 `--locked` 锁；13 项自检通过。正式 locked 门禁仍需在无个人 patch 的干净副本运行。
- ByteHost Apps 两次候选记录分别为 485/3 失败与 484/3 失败，失败集合相同；复跑时以实际输出为准，不把数字当作清理保证已通过。
- 二次审核 R1/R2/R4 已修复：Dozer CI 同时运行真实门禁与自检；Digger 单图门禁与实机自检入口已落到原工作树，Digger 15 项门禁自检、4 项 editor-selftest 自检通过。R3（正式 tag/lock）仍阻断，Digger Cargo.toml/Cargo.lock 未改，未用 path 图或补丁包冒充发行验证。
