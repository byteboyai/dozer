# ByteBoy 稳定化剩余工作与交接计划

日期：2026-11-03。状态：**共享库标签、Digger 正式 git 来源锁和两产品 locked/核心测试已完成；GUI、真实 agent、远端 CI 与发行性能验收仍未完成。** 本文用于继续收口实机与验收，不要求重做已发布实现。

目标：用首次 2–3 个工作日时间盒收口发布可复现、门禁可信和关键使用路径；达标后转回功能开发。时间盒是首次复盘节点，不是通过跳过失败来按期完成的理由。本批不启动插件化、通用 actor、编辑器底层替换或大范围结构重写。

依据与交付物：

- [五仓库评估](../../analysis/2026-11-03-byteboy-next-stage-assessment.md)
- [最新实施记录](../../analysis/2026-11-03-byteboy-stabilization-progress.md)
- [补丁包与落地说明](../../patches/2026-11-03-byteboy-stabilization/README.md)
- [补丁基线和 SHA256](../../patches/2026-11-03-byteboy-stabilization/manifest.json)
- [候选验证结果](../../patches/2026-11-03-byteboy-stabilization/verification.json)

## 1. 已完成内容与当前工作树

| 范围 | 已完成 | 当前存放位置 |
|---|---|---|
| Dozer 门禁/规范 | 单次完整锁定依赖图检查；13 项自检；规范版本事实改为引用清单/lock | Dozer 当前未提交修改：CLAUDE.md、scripts/check-host-deps.sh、scripts/tests/test_host_deps.py |
| Digger 门禁 | 修复 cargo 失败仍成功；默认覆盖新业务；普通/构建/跨平台/传递依赖检查；15 项自检 | digger.patch，尚未落原仓库 |
| ByteHost 测试与联合检查 | helper RAII、有限等待；两消费方必查；缺目录失败及恢复 lock，3 项脚本自检 | bytehost.patch，尚未落原仓库 |
| Digger 实机入口 | 只运行本次 compiler-artifact；父进程超时回收；4 项脚本测试 | digger.patch，尚未落原仓库 |
| ByteUI 增量/候选映射 | v0.5.2 版本准备；候选映射补齐 file-io/git-runtime；133 项单测通过 | byteui.patch，尚未落原仓库；NotebookTabs 已在本地基线提交内 |
| 组合检查 | 两产品候选 locked 全目标编译和依赖图检查通过；四份补丁 apply --check 通过 | 补丁包日志/verification.json |

这些通过结果包含临时 path patch，不代表正式发布组合已通过。ByteHost Apps 候选记录为 485 passed / 3 failed；评估末次日志为 484 passed / 3 failed，两次失败集合相同，具体总数取决于当次纳入的测试集合，复跑时以实际输出为准。3 个进程身份/孤儿恢复用例仍待正常权限环境复验。ByteGit 上次有 9 个真实监听测试因无法启动 FSEvent 失败，不能记为通过。

Dozer 内已有上述修改和未跟踪文档/补丁包，必须保留；不要再次覆盖成旧实现。原会话只可写 Dozer 与 /tmp，因此其它三仓库仅交付补丁。`/tmp/byteboy-stabilization-20261103` 是可选调试缓存，不是必需交付；丢失后应能仅凭仓库内补丁和记录继续。

### 2026-11-03 当前核对

- 五仓库当前均可写；HEAD 与第 2 节基线一致：ByteUI `eb08e57`、ByteHost `0362608`、ByteGit `5472fa5`、Dozer `6e389a30`、Digger `44b84e7`。Digger 仍没有配置 remote，补配前需先确认目标分支。
- 四份补丁的 SHA256 与 manifest 一致；ByteUI/ByteHost/Digger/Dozer 的 `git apply --check` 均通过。Dozer `dozer-release.patch` 仍可在当前 HEAD 上 dry-run。
- `bash scripts/check-host-deps.sh --self-test` 已调整为先进入隔离自检，不再被当前工作区的个人 path patch / `--locked` 锁文件阻断；13 项自检通过。正式 locked 门禁仍必须在无个人 patch 的干净副本执行。
- Dozer 最新运行时提交 `6e389a30`（App 0.5.315）已包含在本地基线；尚未推送远端，因此不能作为已发布 libs 快照引用。
- 二次审核后：Dozer CI 已同时运行真实依赖门禁与 `--self-test`；Digger 的门禁/自检修复、正式版本清单与 git 来源 lock 已提交并推送到 main `e82c048`。
- 已发布 ByteUI `v0.5.2`（`e338245`）、ByteHost `v0.4.22`（`729fcaf`）、Dozer `libs-2026-11-03a`（`e019a56`）；Digger locked 全目标与 workspace lib 测试通过。

## 2. 基线与依赖顺序

| 仓库 | 补丁/工作树基线 | 预备发布版本 |
|---|---|---|
| ByteUI | eb08e57c5e489f905ccc0336107ad4c3b4291e79 | v0.5.2 |
| ByteHost | 036260882682ae61f193a49470a33da1e6d8fc31 | v0.4.22 |
| Dozer | 6e389a300795d6295bdc3431223e5f84e08e1c2a | libs-2026-11-03a |
| Digger | 44b84e78dbb81fd0df1a14606d8ac3b3df9f6402 | 本批正常修复提交，不预设产品版本号 |
| ByteGit | 5472fa553ab34d97d9a1e4055c89d03b48eb4848 | 继续 v0.9.2，不为空发版 |

以上是交接时点快照，执行前重新核对本地/远端。ByteUI 的 NotebookTabs 和 Dozer 的最新运行时提交当时尚未进入远端 main；补丁基于本地新 HEAD，不是对远端旧 main 的全部差异。

顺序：**P0 基线/权限 → P1 补丁落地 → P2 发布前验证 → P3 ByteUI → P4 ByteHost → P5 Dozer libs → P6 Digger 正式锁/两产品发行检查 → P7 实机/性能 → P8 收口记录。** 不应要求 Digger 在上游标签尚未存在时先通过无 patch 发行构建。

标签名只是预留。若已被其它工作使用，核对内容后选择新版本并统一调整清单、补丁说明和验证记录；不移动已发布标签。

## 3. P0：确认可写范围、分支和远端

- [ ] 在有权限的会话中实际创建/删除探针，确认五仓库可写；文件访问失败不得解释成代码失败。
- [ ] 读取各仓库适用 AGENTS/开发约束；记录 HEAD、分支、status、remote、有效 Cargo 配置和工具链。
- [ ] 保留全部原有修改，包括 Dozer 的未提交门禁、规范、分析文档和补丁包；不 reset/clean 或删除个人 .cargo 配置。
- [ ] 重新检查 Digger 远端 `https://github.com/byteboyai/digger.git` 与目标分支。本地若未配置 origin，核对后补配；已有 origin 不盲目覆盖。
- [x] 从 manifest 核对四份补丁 SHA256，并重新 dry-run。基线有变化时审阅差异后适配，不强行重复应用。

示例（在正常终端执行；按实际工作区调整根目录）：

```bash
BYTEBOY_ROOT="/Users/chrischiang/Projects/CoralProjects/byteboy"
PATCH_DIR="$BYTEBOY_ROOT/dozer/docs/patches/2026-11-03-byteboy-stabilization"
git -C "$BYTEBOY_ROOT/byteui" apply --check "$PATCH_DIR/byteui.patch"
git -C "$BYTEBOY_ROOT/bytehost" apply --check "$PATCH_DIR/bytehost.patch"
git -C "$BYTEBOY_ROOT/digger" apply --check "$PATCH_DIR/digger.patch"
git -C "$BYTEBOY_ROOT/dozer" apply --check "$PATCH_DIR/dozer-release.patch"
```

完成证据：环境/分支/差异清单、补丁校验结果和实际目标分支。此阶段不发 tag。

## 4. P1：落地现成修复，区分行为与发布变更

- [ ] ByteUI：应用 byteui.patch；保留基线中的 NotebookTabs 与 SVG，更新版本/CHANGELOG 和候选 workflow。
- [ ] ByteHost：应用 bytehost.patch；核对只改变测试所有权、联合检查/CI及共享依赖，不放宽生产进程身份规则。
- [ ] Digger：应用 digger.patch；保留 digger-paths 的窄契约例外，仍禁止 iced/wry/其它宿主依赖；普通业务默认纳入。
- [ ] Dozer：保留当前门禁/测试/规范修改；依赖升级来自 dozer-release.patch，在对应标签实际存在后用于正式解析。
- [ ] 将行为修复与版本/lock 更新分为可审阅提交；补丁内的未来版本清单可暂存，不能据此声称已经可发行。
- [ ] 检查补丁没有包含个人 .cargo 配置、临时绝对路径、候选 path lock 或测试用户数据。

关键文件：

| 仓库 | 文件 |
|---|---|
| Dozer | scripts/check-host-deps.sh、scripts/tests/test_host_deps.py、CLAUDE.md、Cargo.toml/Cargo.lock |
| ByteUI | Cargo.toml、CHANGELOG.md、.github/workflows/candidate-check.yml |
| ByteHost | apps 的 testutil/manager/process::supervise、scripts/joint-build.sh、scripts/tests/test_joint_build.py、两个 workflow、消费 ByteUI 的清单 |
| Digger | scripts/check-host-deps.sh、scripts/run-editor-selftest.sh、scripts/tests 两文件、consumer-check.yml、Cargo.toml/Cargo.lock |

完成证据：原仓库中的实际 diff 与提交 SHA；不得只引用隔离副本或补丁文件代替落地。

## 5. P2：发布前验证与 CI 组合检查

- [ ] 在实际落地源码中运行 Dozer 13 项、Digger 15 项门禁自检，以及联合脚本 3 项、自检入口 4 项测试；数量以最终源码为准，不删测试凑历史数字。
- [ ] ByteUI：fmt、clippy、lib/公开示例相关检查通过。
- [ ] ByteHost：fmt、clippy、默认 feature 检查、all-features 与客户端 conformance/产品嵌入契约；正常权限环境下重跑 3 个孤儿恢复用例。
- [ ] ByteGit：正常 FSEvent 环境重跑 all-features，尤其上次失败的 9 个真实监听用例；保留 linked-worktree、rescan/degraded、取消与清理回归。
- [ ] Digger：验证实机脚本确实拒绝旧缓存产物，超时能回收其测试进程；运行相关预览保存、Agent/Todo/项目隔离契约测试。
- [ ] 重新构建统一候选组合，必要时显式 cargo update 选中候选 ByteUI 0.5.2；不能容忍“patch 未使用”后实际仍在测试旧 ByteUI。
- [ ] 检查两产品候选依赖图各共享库仅一个来源，随后用候选 lock 跑 workspace all-targets；记录精确源码 SHA和 patch 清单。
- [ ] 验证联合 workflow 的真实组合输入。ByteHost 只 patch 自己、而消费者仍引用旧 ByteUI/Dozer 快照时，可能形成重复来源或缺 API：先用统一候选 workflow/明确快照验证，修正实际配置后再要求 PR/tag 检查通过，不能跳过 Digger 或屏蔽错误。

脚本级命令（各自仓库根目录）：

```bash
# Dozer 与 Digger
python3 scripts/tests/test_host_deps.py
# ByteHost
python3 scripts/tests/test_joint_build.py
# Digger
python3 scripts/tests/test_editor_selftest.py
```

库仓库若不跟踪 Cargo.lock，在隔离副本生成临时锁运行测试；不要将其描述成消费方正式 locked 验证。记录源码改动后的实际结果，不把本计划中的历史通过数直接复用。

环境限制处理：记录具体失败、环境和所需重跑命令；不改 ignored、不删断言、不用纯状态机测试代替真实 FSEvent/进程身份检查。若正常权限环境仍失败，先修复并补行为回归，再继续相应发布。

完成证据：候选组合矩阵、脚本/库/契约测试结果，以及仍失败项的明确处置。

## 6. P3–P5：发布上游与统一 Dozer libs 快照

三个缺失标签的具体解阻顺序、发布门槛与正式消费验证见 [共享库标签发布专项计划](2026-11-03-shared-library-tag-release.md)；该专项完成不替代本计划的实机与性能验收。

### P3：ByteUI

- [ ] 核对包含 NotebookTabs 与版本变更的最终提交；完成必要测试。
- [ ] 将实际测试的提交推到目标分支并发布新的不可变 tag；记录远端 tag 解析出的 commit。
- [ ] 从远端重新取该 tag，确认不是只在本地存在。

### P4：ByteHost

- [ ] 全部实际消费 ByteUI 的 crate 使用同一已发布版本；重查依赖图与锁，避免两份 ByteUI/iced 类型。
- [ ] 测试辅助清理和两消费方联合检查落到实际发布提交；生产权限/导航/生命周期政策保持。
- [ ] 在已发布 ByteUI 上验证，再推送/发布 ByteHost tag并核对远端 SHA。

### P5：Dozer libs

- [ ] 包含已有最新运行时实现、当前门禁/规范修复及两项共享依赖升级，保留旧 API/trait fake/线上 JSON 兼容契约。
- [ ] 在没有个人 path patch 的源码副本中，从实际远端 tag 解析并生成正式 Cargo.lock；将 git 来源锁带回对应工作树。
- [ ] 运行正式 locked 全目标、核心运行时/client/panel/文件 IO 回归和边界门禁。
- [ ] 发统一 libs tag，使 Digger 的全部同源共享 crate 能引用一个真实快照；用远端 tag SHA 验证。

发布前重新读取适用仓库发布规则。推送或 tag 不得夹带无关修改；不能将“分支已推送”记成“标签已发布”，也不能将 tag 已存在记成该版本已经通过验收。

## 7. P6：生成 Digger 正式锁，验证两产品发行组合

- [ ] Digger 的全部同源 Dozer crates 引用实际发布的统一 libs tag；统一 ByteUI/ByteHost，ByteGit 保持已验证版本。
- [ ] 无个人 patch 的隔离副本生成正式 lock，修复原有 24 个共享包缺少 git source 的问题；核对所有共享包的实际来源与 SHA。
- [ ] 将生成的正式 lock 带回工作树；不把候选 path 锁复制成发行锁，不删除个人联调配置。
- [ ] 从两产品准备发布的实际提交分别创建干净 checkout，不含个人 .cargo patch；运行下列命令。
- [ ] Digger 核心 editor/store/paths/app-host、前端类型/状态机与嵌入产物一致性检查通过；Dozer 核心运行时与客户端/面板回归通过。
- [ ] 联合检查覆盖两个产品，远端 CI 有实际 run URL/SHA/结果；没有远端执行证据时只记录本地等价验证。

两产品干净 checkout 的最低公共门禁：

```bash
cargo fmt --all --check
cargo check --locked --workspace --all-targets
bash scripts/check-host-deps.sh --self-test
bash scripts/check-shared-deps.sh --offline
```

Dozer 另跑 panel boundary/log scope；Digger 另跑 web check/test、parser fixtures 和 `build-editor-web.sh --check`。实际完整命令参考各仓库 workflow，helper 必须来自本次构建。

本阶段失败时按依赖组合/API/环境分类定位；不降级到个人 patch 后继续宣称发行通过。

完成证据：两产品正式 lock、已发布版本/SHA矩阵、无 patch locked 日志、契约/产物检查和远端 CI 结果。

## 8. P7：实机与发行性能验收

使用独立临时产品根和可丢弃项目；不迁移或修改正在使用的真实项目。确认屏幕解锁、图形会话、进程身份与 FSEvent 权限可用，并记录实际二进制来源。已安装但无法对应源码 SHA 的旧窗口不能计作候选验收。

### Digger

- [ ] 运行 `scripts/run-editor-selftest.sh`、`lifecycle`、`usage`，检查真实 SUMMARY 与退出码。
- [ ] 使用 `DIGGER_EDITOR_FAIL_FIRST_BUILD=1` 验证实际 Retry 按钮，确认新 generation 后能完成加载。
- [ ] 真实中文 IME 组合输入、撤销/重做、焦点、主题和项目/tab 切换；保存后重开内容一致，冲突/失败保留草稿并能重试。
- [ ] 仅验收当前产品实际启用的应用生命周期；被移除或禁用的面板不作为虚构产品流程要求。

### Dozer

- [ ] 持续终端输出、暂停消费后恢复 marker、断线重连、Resize/选择、退出早于 attach、关闭再开和项目切换，检查旧 owner/epoch 不污染新状态。
- [ ] Todo 失败/中断持续可见，重试是显式操作；任务启动/超时不自动重复产生副作用。
- [ ] 回填期间可连接且历史状态诚实；实际 SIGTERM/SIGINT/RPC 停止后进程、锁/socket 与持久恢复状态符合预期。
- [ ] 真实 agent 的只读/权限金丝雀使用隔离目录验证，不在用户项目放宽权限或调用危险参数。

### 性能

- [ ] 固定 fixture、构建 profile、工具链、硬件和基线 SHA，预先写清测量范围/预算/判定；保留原始样本，不只留一张汇总表。
- [ ] Digger：生产 IPC/paint 与输入到绘制；沿用现有 100/1000/5000 块 fixture及可重复基准，注明 UI 与 Rust Bridge 的边界。
- [ ] Dozer：真实发行构建的 locate 请求、摄取期间普通 RPC p95/p99、GUI 停消费后的资源增长、首次可连接与停止耗时。
- [ ] 观测排队正文额度、编码帧、终端模型/进程 RSS 和运行句柄；不将逻辑字节许可等同于整进程内存。
- [ ] 与同 fixture/构建方式的基线对比；若无可用历史基线，明确建立当前基线，不编造优化倍数。对实际超预算项单独立项，不扩大成全框架重写。

完成证据：实际可执行文件 SHA/版本、实机检查表、失败复现、性能原始样本及可重复命令。

## 9. P8：完成定义、回退与交付

- [ ] 所有代码落到实际仓库，修复/发布提交与测试记录一致；没有只留在 /tmp 的必要修改。
- [ ] 远端依赖标签存在，两产品正式 git 来源锁与无 patch locked 检查通过，联合/CI 有真实结果。
- [ ] 进程恢复和文件监听的既有环境失败已在适当环境复验；关键实机与性能结论有证据。
- [ ] 更新实施记录与本计划，只勾选实际完成项；将候选记录保留为历史证据，注明最终来源。
- [ ] 交付版本矩阵、提交/tag/CI链接、验证命令、已知限制和回退步骤。

回退原则：不移动远端标签；源码/依赖回退使用新提交。涉及持久数据时先停止产品并备份数据根、SQLite/WAL/SHM及旧格式文件；按现有迁移/导出政策恢复，不能仅降级二进制后猜测数据库仍兼容。不要把旧 JSON 覆盖到已有新写入的权威数据库。

若首次时间盒结束仍有验收缺口，列明阻断、已完成内容、下一次验证条件；不宣布全部收口，也不自动继续无上限的框架优化。达到退出条件后，按约 70% 产品功能、20% 相关质量、10% 工具/规范维护进入下一阶段，具体新功能另行立项。

## 10. 新会话交接提示

> 继续 ByteBoy 稳定化收口，读取本计划、实施记录与补丁包 manifest。先确认五仓库写权限和当前实际分支/远端状态，保留 Dozer 已有未提交修复，不重复实现。按 P0–P8 推进；候选 patch 验证不能代替已发布 git 来源的正式 lock 和实机验收。不要移动现有标签，不隐藏环境失败，不启动额外大重构。
