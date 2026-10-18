# bytehost 剩余未尽事项

整理日期：2026-10-16；**2026-10-17 重写了现状**(同日做完 GUI 手工验收与一批修复)。bytehost 仓库 `byteboyai/bytehost`,`main` 与远程同步,HEAD 即 `v0.4.7`。包含 7 个 crate（`bytehost-core`/`-chrome`/`-desktop`/`-webview`/`-apps`/`-client`/`-panel`）。dozer 已依赖 `v0.4.7`(byteui `v0.4.5`);digger 的升级由用户在 digger 仓库处理。

本文只收**尚未做完**的项。来源：bytehost 的 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`（§9 待裁决清单）、五份验收报告（A5、A6e、A6f、A6g、A6h）、dozer 的 `docs/dozer-v2/bytehost-manual-acceptance-checklist.md`、`bytehost-H8-evaluation.md`、`2026-10-04-bytehost-extraction-roadmap.md`、i18n 与 Digger 集成两条线的计划/台账。

**核对方式：** 读文档、tag、`Cargo.toml`、git 历史；§5 的“已知失败测试”已于 2026-10-16 复跑，§1 的 A 级命令行项已于 2026-10-17 执行。

## 1. 真实 GUI 手工验收:基本做完,剩几处

2026-10-17/18 在隔离环境里用真 GUI 做了手工验收,逐项结果见 `docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md`。五份验收报告(**权威且最新的副本在 bytehost 仓库** `docs/superpowers/specs/`,本仓 `specs/` 下的同名文件是 2026-10-08 的旧拷贝,已加说明)已于 2026-10-18 按结果对账:**A6f、A6g、A6h 完成;A6e 基本完成(剩一处);A5 仍部分完成。**

- **A5(部分完成):** Excalidraw 本体相关项没做(2026-10-18 决定先不做;需要联网打包字体):§3 的 1、2、3、4、6、8 与 V2 手工补项。第 7 项的行为(dozerd 停/重启、退出整个 GUI 后网关与应用仍在)已用 `py-notes`/`node-notes` 验证,但字面写的是 Excalidraw,没勾。第 5 项(`Host` 伪造)已勾。
- **A6e(基本完成):** 只剩一处未验:`py-notes` 审批卡里"全部权限强制等级为仅声明,不强制"的标注(样例没有声明任何权限,看不到这个标注)。`node-notes` 一遍、退出整个 GUI、藏起 node 的"尚未安装"提示、受管 Node 都已于 2026-10-17/18 做完;受管 Python(uv)没有对应的验收项,未验。
- **A6h(完成):** 网络来源 3 项已于 2026-10-18 做完,但 https 下载是对本机 TLS 服务器(临时 CA)做的,**未覆盖公网证书链与重定向**。输入法一项原先有缺陷(overlay 窗口里候选条出现在窗口左上角),已于 2026-10-18 修复(bytehost `v0.4.10` + dozer 3 个手写 overlay 接线),复测设置、创建项目、新建群聊三处都贴着输入框;overlay 里仍没有带下划线的组字预览,未做。
- **应用 i18n 的双窗口隔离(6.3):** 依赖分离窗口功能(已推迟),现在做不了。

验收中发现的问题大多已修(见手测文档里带"已修复"的行);已修的:WebSocket 经网关握手失败(bytehost `v0.4.5`)、断线静默(`v0.4.6` + dozer Toast)、日志文字不跟主题、设置页按钮裁切/挤压、i18n 样例被 CSP 拦、审批卡"来源渠道"标签、重订阅退避不拉长(`v0.4.7`)、压缩包 `./` 前缀(`v0.4.7`)、图标栏无区分(首字母徽标,byteui `v0.4.5`)。**撤回一条:** 验收时记的"crash 恢复后页面不重载"是我看错了,复测两条路径都会重载。

**有意不改(已裁决,写在这里避免被当成遗漏):**
- **hang 期间约 35 秒内面板仍显示"运行中"的旧页面,没有"无响应"提示。** 要显示就得新增"无响应"状态并追加 wire、改 supervisor 与面板,收益小。
- **启动即崩的应用要等约 40 秒才出现 Crashed 页。** 退避 1/2/4/8/16 秒重启 5 次才放弃是 supervisor 的设计,期间面板只显示"启动中…"。

dozer 已经有一份分级清单 `docs/dozer-v2/bytehost-manual-acceptance-checklist.md`（2026-10-16 整理）：

| 级别 | 含义 |
|---|---|
| A | 纯命令行即可（`curl`、`kill`、读文件），可直接脚本化 |
| B | 要操作 Dozer 窗口，**能否用 `agent-desktop` 读到 iced/wry 的控件还没验证**，先做探针 |
| C | 需要人判断或人的环境：视觉观感、中文输入法、真实 https 服务器、系统剪贴板 |

- **A 级已部分执行（2026-10-17）：** 纯命令行部分已在隔离 HOME 的 `dozerd` 上跑过，A5-5（伪造 Host 得 421）、A6e-3（crash 约 2 秒、hang 约 34 秒自动恢复，计数保留）、A5-7/A6e-4（停 dozerd 网关关闭，重启后端口不变、应用恢复）通过；A6f-1 只确认了网关立刻关闭。**没做：** A5-7 中"退出整个 GUI"一半、Excalidraw 打包、各项夹带的其余命令行步骤。详见清单文末「A 级执行记录」。
- **做之前要先探针：** B 级项能否自动化。
- **只能人来做：** C 级（A6h 的真实 https 下载与中文输入法、A5 的复制粘贴/导出/手写体字形）。
- **后果：** 在 B/C 项有结论前，A5、A6e、A6f、A6g、A6h 五个切片在文档里都不能标完成；spec §7 的 A5 行也明确这样写。

## 2. 文档状态与实际不一致（2026-10-16 已对账）

下列状态行已改成实际状态：

| 位置 | 原来写的 | 现在 |
|---|---|---|
| dozer `plans/2026-10-14-bytehost-i18n-integration.md` 头部 | T7/T8 “待上游发布后执行” | T7、T8 已完成（`v0.3.0` 发布、联调 patch 已删、`e5d6d585`）；仍未做的是**真实 WebView 样例冒烟** |
| dozer `specs/2026-10-14-bytehost-app-i18n-acceptance.md` | “⏳ 待 T7” | 改为“⏳ 待真实环境”，当前状态段同步 |
| dozer `plans/2026-10-11-digger-bytehost-full-integration.md` 头部 | “待实施，所有复选框均表示尚未完成” | Task 01–23 均有执行记录，但整体不标“完全接入”（见下 §6）；复选框未回写，以台账为准 |
| dozer `ledger.md` Task 08 | `IN_PROGRESS` | 保留原行并加备注：快照未回写，后续 Task 09–23 已执行；真实窗口人工确认仍未做 |
| dozer `CLAUDE.md` bytehost 一行 | 按 tag `v0.1.1` | `v0.4.1` |
| bytehost `README.md` | “四个 crate”；Digger 嵌入“尚未随 tag 发布” | 七个 crate（补 `core`/`desktop`/`chrome`，并说明前四个与后三个的依赖边界不同）；嵌入已随 tag 发布（`v0.2.0` 起，来自台账 Task 22/23） |

**crate 版本号 `0.1.0` 与 tag 不一致是有意约定**，已写在 bytehost README 的“tag 即发布”一条里(2026-10-18 核对,此前这里写“没写进 README”是过期的)。

## 3. 功能层面明确还没做的

| 项 | 内容 | 触发条件 |
|---|---|---|
| Git 仓库作为应用来源 | A6h 只做了 `AppSource::{Archive, Url}`；“Git 仓库来源”仍未做 | 有真实需求，且先定信任与依赖沙箱 |
| 网络来源放开进程型应用 | 现在**网络来源只允许静态应用**（`Untrusted`）。放开需要依赖沙箱 | 沙箱方案立项后 |
| 容器运行时 | 一直推迟。spec 验收 8 的“容器不可用提示页”现在用“安装时明确拒绝”代替；Docker/Colima 只探测、不安装 | 二期之后；见 §6.3 的提示页设计 |
| 非 macOS 平台 | 受管运行时只有 macOS 有固定版本表（pin）；非 mac 从未实际编译过 | 真要发非 mac 版本前 |
| PlantUML/UML 面板 | spec A5 倾向作为 Preview 的 `.puml` 渲染类型（只渲染不编辑），形态“待用户裁决” | 用户裁决 |
| External / Declarative / None 入口的 i18n | i18n 本期只覆盖 `EntrypointKind::Web`；`DOZER_UI_LOCALE` 留给 External 真正落地 | External 入口落地时 |
| i18n 语言族回退、简繁转换 | 明确不做，要做需单独版本化 | 出现需求时 |

## 4. 已知局限与安全边界（有意如此，写下来避免被当成遗漏）

- **出站网络强制等级仍是 `Advisory`。** 静态应用靠 CSP + 导航白名单，CSP 挡不住 WebRTC；进程型应用没有沙箱。不要宣称 `Enforced`。
- **wry 对摄像头/麦克风请求无条件批准**，没有钩子可改，目前只靠 macOS TCC 挡着。**将来 Dozer 若加语音输入等功能，必须先给应用 webview 加授权闸门（对照 manifest 的 grants）**，否则所有应用会静默拿到权限。
- **UDS 上的请求等于“同一用户”。** 同用户进程（包括 Dozer 的 PTY 里的 Agent）可以自己出计划、自己批准、自己安装并启动应用；“甲方批准”只是产品层约定，dozerd 不强制。需要强制时要由 supervisor 保存计划并只接受自己签发的。
- **`LocalDir` 安装会把整个源目录永久拷进应用目录**（含 `.env` 之类的敏感文件），即使 gateway 只提供 `runtime.source` 子目录。
- **本机压缩包享受 `Trusted`**：用户手动选中一个从网上下载的压缩包，与选目录一样受信。
- ~~**压缩包条目路径不接受开头的 `./`**（A6h 报告：可用性上偏严，不是越权）；要支持需在 `safe_segments` 里先剥掉开头的 `./` 段，属 A6h 跟进或用户裁决事项。~~ **已处理(2026-10-17):bytehost `v0.4.7` 起接受开头的 `./`,根目录条目被跳过;`a/./b`、`./../x`、`.//a`、绝对路径等仍拒绝,真实系统 `tar -czf x.tgz .` 已测。**
- **日志刷新是整段重读**（每秒读末尾 ≤256 KiB，未做偏移增量），有界但可能多读。
- **打包 Excalidraw 时联网下载 3 个字体，未校验摘要**，镜像用 `latest`，以验收报告记录的摘要为准。
- ~~**重订阅退避不会拉长(待修):** `bytehost-panel` 的 `subscribe_if_due` 把 `Backoff` 改成 `Pending`,下一次失败时档位重新从 0 算起,真实流程里一直 2 秒重连一次(dozerd 停 14 分钟共 298 次)。修法是在 `Pending` 里保留上一次的 `attempt`,并补一条经过 `subscribe_if_due` 的测试(2026-10-17 验收复测发现)。~~ **已修复(2026-10-17):bytehost `v0.4.7`,`Pending` 带着上一次的档位。**
- **PanelEffect 新增 `ConnectionLost`/`ConnectionRestored`(`v0.4.6`):** 它不是 `#[non_exhaustive]`,digger 升到 `v0.4.6` 时任何对 `PanelEffect` 的穷尽 `match` 都会编译失败,需要处理这两个变体。
- **`PanelEffect` 已在 `v0.4.9` 标 `#[non_exhaustive]`(2026-10-18):** 消费方对它的 `match` 需要兜底分支;dozer 已处理(`libs-2026-10-18b`),digger 同步时要在 `effects.rs` 加。
- **偶发测试(2026-10-18 已全部处理):** 32 线程并行下复现并修了四个同族问题——"先探测再使用"的端口被并行测试抢走(`start_uses_the_persisted_port_across_restarts`、`the_first_run_*`,改成现取端口 + 有界重试)、`Running` 事件晚于状态可见(`a_python_app_installs_starts_*`,改成有界等待事件)、`a_python_app_runs_through_the_wire_*` 查的其实是 gateway 端口而不是应用端口(改成读应用日志里的真实端口)。修后 40 轮 32 线程 0 失败(修前约 1/40 到 3/40)。**机器同时有别的重负载时**,运行时安装的"试跑超时"和进程型 e2e 的端口冲突仍可能偶发,这是负载造成的,没有处理。
- **`data_store_identifier` 算法已钉死**，改动即破坏用户的 webview 数据归属，不得改。

## 5. 验收报告里记录的“已知无关失败”（2026-10-16 已复核）

验收报告（A6f、A6g、A6h，以及 Digger 集成的 `acceptance.md`）当时记录了几个“与本切片无关、clean tree 也失败/偶发”的测试。2026-10-16 在 dozer `main`（`812ac3e6`）上各跑了一次：

- `cargo test -p dozer-app`：**1963 passed, 0 failed, 2 ignored**。`extensions::files::tests::delete_confirm_spec_reflects_pending_target` 与 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name` 都没有失败。
- `cargo test -p dozerd`：**lib 461 passed, 0 failed, 1 ignored**，集成测试全绿。`memory::tests::list_orders_by_updated_ms_desc`、`app_service::tests::the_first_run_is_unavailable_when_the_port_cannot_be_sa…` 都没有失败。

**结论：** 这些失败现在没有复现，可以认为已被后续修复或已消失。**限制：** 每个只跑了一次；其中 `memory` 与 `app_service` 那两条当时标的是“偶发/并行偶发”，一次通过不能证明偶发问题已修好。若它们再出现，按“偶发”处理。验收报告里的旧记录保留原样（报告是当时的快照，不回写）。

## 6. 架构线：H8 与 Digger 一并推进

- **H8（面板状态与消息信封从“点名 12 个面板”改为按面板 id 取的容器）。** 评估报告 `bytehost-H8-evaluation.md`：Q-H8-1 已按 A+B 裁决（停在 H7a，先做 O12 探针）；Usage 探针（2026-10-08）按原判据只命中 2 条，未达阈值，结论是 H8 的合理范围不是全量动态化，而是补齐几块契约。O7（`protocol` 归属）已解（`dozer-model`），X0 试点（Usage 经 `dozer-services` 取数）已落地，其余面板在按纵向切片迁移（CLAUDE.md 称全部面板已迁）。**报告里 Q-H8-2（Digger 复用 Dozer 面板是近期已排期还是“将来可能”）、Q-H8-3（首要目标是“host 可复用”还是“`update.rs` 变小”）我没有找到明确的裁决记录**；Digger 现在已经在复用 Usage 面板（`libs-2026-10-16c`），Q-H8-2 事实上已回答；**该事实结论已于 2026-10-17 写回 `bytehost-H8-evaluation.md`**(裁决仍属用户,Q-H8-3 仍未裁决)。路线图 X4 只做 host 骨架需要的最小部分；每一步开工前先复测三个度量（`PanelKind` 引用、`update.rs` 面板臂数、`Workspace` 面板字段数），确认数字确实下降。**不要自动排全量 H8。**
- **A8（Digger 的 supervisor 怎么实现）：** spec 写“待 Digger 启动时裁决”。实际 Digger 已选**进程内嵌入**（`bytehost-client/in-process` 的 `InProcess`，无守护进程），可以把这条标成已定。
- **Digger 完整接入的硬缺口**（台账 Task 23 标 DONE，但 `docs/dozer-v2/digger-bytehost-integration/acceptance.md` §4.5 的最终判定是 **pass 7 · partial 11，整体不标“完全接入”**）：
  - **A17 真实 Usage 生产端：** 真实生产事件 → 账本 → UI → 跨重启没有实跑；应用运行携带的 token 诚实为 0（S6/S7 不完整）。
  - **A18 发布包签名：** 机器上没有 Developer ID Application 证书（只有过期的 Apple Development 证书和中间 CA），用户已裁决不申请，**永久阻塞**，只能出 ad-hoc 签名候选包。
  - **复审登记的后续计划清单 #2–#6**（`review-2026-10-13-full.md`），其中 #3 无应用启动/视口（真机 `LaunchUrl` 未接）、#4 Usage webview 未挂载属 A17 缺口；#1、#7、#8、#9 已修。
  - **Digger ADR-001 / ADR-011 保持 `Proposed`**，A18 不过不升 Accepted。
  - 另有多项 L3/L4 真机验证因缺屏幕录制/辅助功能权限、无 GUI runner 而 not-run，并入人工项。

## 7. 跨仓库流程

- **同日一起升。** bytehost/byteui 升级时 dozer 与 digger 必须同一天一起升（否则 `PanelId` 出现两份类型、byteui 全局主题状态分裂）。`v0.4.2`–`v0.4.4` 只是 `shared-deps-guard` 工具提交（相对 `v0.4.1` 没有库代码变化），所以 dozer/digger 停在 `v0.4.1` 功能上不缺东西；但 guard 的 `behind` 规则可能提示落后。**要不要升到 `v0.4.4`、什么时候升，需要定。**
- **bytehost 的 CI** 有 fmt、clippy（`-D warnings`）、test、`check-deps.sh` 及其变异测试，**没有**跨仓库的“用 dozer/digger 联合构建”检查，也没有 `check-shell-regions` 之类的一致性检查（后者在 digger 里，见 byteui 文档 §1）。
- **bytehost-chrome 固定 byteui `v0.4.4`**，是 byteui 升级时第三个要同步的消费方。

## 8. 已确认不做 / 已定

- 自定义协议不作应用 gateway；gateway 用 `<app-id>.localhost` + 单一固定端口 + Host 头路由。
- 进程所有权跟随 dozerd，dozerd 停止则应用停止；gateway 放进 dozerd。
- Docker/Colima 只探测与指引，不由 bytehost 安装。
- 应用入口形态是 rail 图标 → 该应用自己的面板；独立窗口留作以后的可选能力。
- 静态应用的 CSP 不放宽。
