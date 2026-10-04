# bytehost 边界与拆分规格

> 状态：**草案，待用户审阅**。日期：2026-10-03。
> 依据：`docs/group_chat/关于bytehost的讨论总结.md`（本规格的直接来源）、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`（R1–R3、§4 审计、§6 的 Q7/Q8/Q9/Q10/Q13）、`docs/superpowers/specs/2026-10-02-bytegit-design.md`（先例：独立库、tag 引用、纵向切片迁移）。
> 前置：bytegit 的 P1–P6 已合并 `main`（`delivery.rs` 已删除，`dozer-app` 不再直接依赖 `git2`）。
> 本规格只定义 `bytehost` 的**边界、依赖规则、验收标准与未决项**，不含具体 API 设计、不含 H1 以后的实施计划。H1 以后的计划必须等 H0 审计给出真实依赖数据后再写（见 §8）。
> 本文中的"现状"数字取自 v2 要求文档 §4 的 2026-10-02 审计（只做 import 与引用计数，未跑运行时），H0 会重新核实，不得把它们当作已验证事实引用。

## 1. 目标与非目标

**目标**

1. 从 `dozer-app` 里抽出一个产品无关的 iced 桌面宿主：窗口、布局、焦点、浮层、主题承载、面板生命周期，让"一个面板 + host 就能运行"（要求文档 R1）。
2. 让 Digger 能只换面板清单、复用 Todo / Conversation / Agent / Project / Files Tree 面板，而不带走 Dozer 的治理、交付、验收语义。
3. 把"面板清单散落在 `PanelKind` 的 973 处引用(H0 实测,原审计为约 887)"收敛成一个注册点（要求文档 §4.3）。
4. 在不改变用户可见行为的前提下完成，沿用 bytegit 的纵向切片方式：每一步都能单独编译、通过测试、合并。

**非目标**

- 不是重写：不换 GUI 框架（iced 0.14，GPUI 评估已裁决不换）、不换 CodeMirror/wry 的预览架构。
- 不拆独立面板仓库：本规格只确定 host 的边界；面板（Todo 等）是否各自成 crate/仓库是另一个决策，前提是 host 边界先稳定。
- 不设计事件总线、服务注册的细节（要求文档 Q9）：本规格只规定它们**属于 host 机制**，设计另议。
- 不引入 Swift/AppKit 专属能力，核心不依赖 Node/Python（沿用项目 CLAUDE.md 的关键裁决）。
- 不做 Preview 业务的共享化（见 §3.2）。
- 一期不拆 `bytehost-sdk` / `bytehost-iced` / `bytehost-testing` 多个 crate，也不为未来需求预先设计大量 trait。

## 2. 定义与三条硬边界

**阶段性定义：**

> `bytehost` 是 iced 桌面产品中，**产品无关**、且**不属于** byteui、bytegit、独立面板或明确领域库的公共宿主能力。

采用"剩余即 host"的排除法：凡不是 byteui（通用 UI 组件）、bytegit（Git 底层）、某个面板、某个领域库、或 Dozer 产品层的东西，暂归 host。允许初期内部不够整洁，但必须守住：

| # | 规则 | 如何检查（见 §6） |
|---|------|-------------------|
| E1 | **依赖方向固定：** 产品组合面板，面板依赖 host；host 不依赖 Dozer 产品代码，也不依赖任何具体面板。 | `bytehost` 的 `Cargo.toml` 不含 dozer-app / 面板 crate；`cargo tree` 校验 |
| E2 | **host 公共 API 不出现业务类型：** `TodoItem`、`Conversation`、`DeliveryStatus`、`PanelKind` 的具体变体等一律不得出现在 host 的公开签名里。 | 公开 API 扫描 + 评审 |
| E3 | **面板特判集中登记：** 迁移期 host 内确实残留的"认识某个面板"的分支，必须登记在一个清单文件里（位置、原因、移除条件），不得散落。登记清单的条目数只减不增。 | H0 产出清单；门禁脚本检查清单外无新特判 |

"归属按**机制**判断，不按目录判断"：不得把现有 `app/`、`platform/`、`extensions/` 整个目录搬进 host。同一个目录里既有 host 机制、也有面板专属代码，逐文件、逐函数裁决。

## 3. 归属裁决

### 3.1 属于 host 的机制

- 应用与面板生命周期。
- Workspace、Tab、栏位与布局容器。
- Panel Registry（面板注册：标题 i18n key + fallback、图标、默认栏位——默认栏位由谁决定见 O5）。
- 窗口、焦点、导航、快捷键与菜单分发。
- 通用事件与 Toast 机制（`toast` 已是 host 基础设施，不算面板间耦合，要求文档 §4.1）。
- 通用 Overlay / Surface 承载与几何同步，包括 `platform/overlay_window.rs` 这套独立原生子窗口机制，以及 WebView 的生命周期、几何、焦点、层级。
- 文件选择、拖放等系统接入。
- 设置的**注册、持久化与展示机制**（设置项内容属于产品或面板，见 §3.3）。
- 日志机制：`dozer_core::log` 的 scope 约定已是产品无关机制，其归属（留在 `dozer-core` 还是并入 host）在 H0 中随 `dozer-core` 拆分一起核实。

### 3.2 暂不共享：Preview 与 Digger Writing

保守方案，沿用讨论结论：

- Dozer 现有 Preview **不**作为共享面板拆出。CodeMirror、文件类型路由（`preview/router.rs::classify_preview`）、HTML/JSON 等查看器留在 Dozer 产品层。
- 通用 WebView/Surface 机制（见 §3.1）进 host。
- Digger 新建独立 Writing 面板（富文本创作、文档结构、内容组织），**不**把它设计成 Preview 的变体。
- 与此相关，**Files Tree 必须与 Preview 解耦**：Files Tree 只发出"打开目标"的通用命令，由产品决定处理者（Dozer → Preview，Digger → Writing）。协议未定，见 O3。

### 3.3 留在 Dozer 产品层

- 治理、交付、验收语义（`delivery` 的治理部分已随 bytegit P6 拆干净，剩余的 Dozer 呈现语义，如 `files/git_status.rs` 的着色档位，同样属于产品层或面板）。
- 默认面板组合与默认布局。
- Dozer 品牌、ByteBoy2077 主题**内容**（主题机制归 byteui/host）、产品设置项。
- 具体面板的分支及其专属 overlay（`platform/*_overlay.rs` 里大多数是某个面板的专属 overlay，迁移清单由 H0 产出）。
- Dozer composition root（`main.rs` 一类，负责把面板清单装配给 host）。
- 目前的 Preview 业务（§3.2）。

### 3.4 第一批共享面板（Digger 已确认复用）

Todo、Conversation、Agent、Project 面板、Files Tree。共享的是**面板主体与通用领域能力**，Dozer 的治理语义不随面板进入 host。

两点说明，避免误读：
- 面板"共享"不等于"进入 host"。它们是消费 host 的面板，host 不得依赖它们（E1）。
- Agent 面板主体与会话运行服务如何切开，尚未裁决（O1）。

### 3.5 暂存或待审计的公共服务

下列条目**可以暂时寄存在 host**，但这不意味着永久属于 host 核心。每一项在 H0 里都要给出依赖统计与最终归属建议：

- Project context（项目打开、关闭、当前项目）。
- `secrets`、`external_apps`、`capabilities`。
- 跨面板协调状态。
- `conversation`、`transcript` 等领域模型（要求文档 Q7：`conversation`/`delivery`/`project` 三个共享模块"形似共享数据库"，**未审计其内部**）。
- Git watch、Git accounts：`git_watch` 已进 bytegit（P4）；`git_accounts`（凭据，bytegit 规格 B3 非目标）归属仍未定。

寄存规则：寄存在 host 的条目必须登记（同 E3 的清单），并标注"候选最终去向"，以免"暂存"变成默认永久。

## 4. 现状与差距（待 H0 核实）

下表是 v2 审计的摘要，**仅作 H0 的起点**。

| 差距 | 现状（2026-10-02 审计） | host 化需要解决什么 |
|------|-------------------------|---------------------|
| 面板依赖 host 内部符号 | 面板 import `crate::app::{App, HoverId, ProjectId, TextInputTarget}`、`crate::workspace::*`、`crate::chrome::*`、`crate::preview::WebviewSpec`、`crate::theme`、`crate::menu_spec`（todo/conversations/git_log/ssh/browser/footbar/usage 等） | `App`、`Workspace` 必须从面板代码里出局；`HoverId`/`theme`/图标/菜单规格这类通用能力要么进 byteui，要么成为 host 的稳定接口（Q10） |
| host 对面板硬编码 | `PanelKind` 是封闭枚举（12 变体；原审计为 11，H0 实测多 `GroupChat`，引用 973 行/31 文件，见 `docs/dozer-v2/bytehost-H0/00-summary.md`）、默认栏位写死在 `default_side()`；host 侧（`app/`、`workspace/`、`chrome/`、`platform/`、`preview/`）对 18 个 extension 模块有直接引用；`app/` 约 1.7 万行 | 注册制替代封闭枚举；host 通过 registry 遍历面板而不是点名 |
| 共享领域模块在 app 根 | `conversation`、`project`（`delivery` 已拆除）被多个面板共用 | 逐个裁决：留 host / 下沉 `dozerd` / 独立领域库（Q7） |
| 面板专属 overlay 在 `platform/` | `platform/` 22 个文件、9584 行；面板专属 overlay 宿主 11 个（5 个属 Digger 复用面板）；`window_events.rs` 单文件 4602 行内联多个面板的事件处理（H0 实测，见 `bytehost-H0/05-platform-overlays.md`） | 机制归 host，具体 overlay 随面板走（H0 出迁移清单） |
| 面板间耦合 | 只剩 `file_history → git_log` 一处，已于 bytegit P2 解除 | 无（门禁防止回归） |

## 5. 迁移原则

1. **纵向切片，每步可合并。** 不做"先搬一大半再说"的长分支；沿用 bytegit 的 P0…Pn 方式，每个切片有计划、有验收、单独合并。
2. **先机制后面板。** 先让 host 边界在 `dozer-app` 内部以模块边界的形式成立并有门禁，再决定物理拆成 crate；不一上来就建空 crate。
3. **先审计后固化。** 任何归属假设在 H0 拿到依赖数据前都是假设；H0 之前不动代码。
4. **迁移期不改变用户可见行为。** 与 bytegit P6 相同的纪律：一致性有疑问时先写刻画测试把现行口径钉住，统一口径是产品决策，不在迁移里顺手做。
5. **兼容债务显式登记。** 迁移期的适配层（类似 bytegit 过渡期的 `delivery.rs`）必须在登记清单里写明"何时删"，并在对应切片的验收里要求删除。
6. **沿用既有裁决：** 新浮层默认走独立原生子窗口机制（不走 iced `stack!` + 显式隐藏 webview）；瞬时消息统一 Toast；日志统一 `dozer_core::log`；icon 按钮/tab 复用统一组件。host 化不得回退这些。

## 6. 验收标准

host 边界"成立"的判据（每一条都要有可执行的检查，不接受"看起来是"）：

1. **依赖方向：** `bytehost` 不依赖 dozer 产品代码与任何具体面板（E1，`cargo tree` + CI）。
2. **公共 API 无业务类型：** E2，评审清单 + 脚本扫描公开签名里的业务类型名。
3. **特判清单：** E3 的登记清单存在，门禁脚本保证清单外无新增"点名某面板"的 host 代码，条目数只减不增。
4. **面板边界门禁：** 面板代码里不得出现 `App`、`Workspace`（H0 设计门禁的形式，参照 `scripts/check-log-scope.sh`；不使用 clippy `disallowed_*`，该路线在日志门禁上已验证不可行）。
5. **一个面板 + host 可运行：** 最小 host + 一个面板 + fake 服务能跑起来（要求文档 §5 要点 4）。这条在 H0 里只定义形式，真正达成在后续切片。
6. **不回退：** 全量测试通过、clippy 与基线逐文件一致、`cargo machete` 干净；用户可见行为不变。
7. **兼容债务清零：** 登记的适配层按计划删除。

## 7. 与相邻文档的关系

- **要求文档 R1/R2/R3：** 本规格是 R1（Host + 独立面板）的边界定义；R2（面板之间不直接引用）靠 §6 第 4 条门禁；R3（面板能力经 MCP 暴露给 agent）与 host 解耦，不在本规格范围。
- **要求文档 §6 的开放问题：** 本规格**不**关闭 Q7/Q8/Q9/Q10/Q13；H0 的审计目标之一就是为它们提供数据。
- **bytegit：** 已完成。host 与 bytegit 的关系是 host 平台服务的一个提供者；Git 状态变化是事件总线的第一个用例（要求文档 §7.3），但总线设计不在本规格内。
- **byteui：** host 的 UI 通用组件依赖 byteui；`HoverId`/`theme`/`menu_spec`/`tab_widget` 这类符号归 byteui 还是 host（Q10）在 H0 里逐个标注。iced 版本与 byteui 一致，升级时一起升。

## 8. 未决项（不得擅自定死）

| # | 问题 | 现状 / 倾向 |
|---|------|-------------|
| O1 | **Agent** 的面板主体与会话运行服务具体怎么切；要求文档 Q15 提出 Agent 与 Git 对称为底层支柱，是否做一次对称的分布审计 | 未审计；H0 先出 Agent 相关代码在 `dozerd` 与 `dozer-app` 的分布，再决定；证据:`bytehost-H0/06-open-items-evidence.md` §O1(机制已集中在 dozerd;Agent 面板 = 终端区,与 O6 同一刀) |
| O2 | **Project context** 进入 host 的范围 | 倾向最小化（项目打开/关闭/当前项目），具体待 H0；证据:`bytehost-H0/06-open-items-evidence.md` §O2、`04-shared-modules.md` §1(`project.rs` 实为文件树,不是 project context) |
| O3 | **Files Tree "打开目标"协议**（Files Tree → 通用打开命令/目标注册 → 产品决定处理者） | 协议形态未定；H0 只统计 Files 对 Preview 的现有引用；证据:`bytehost-H0/06-open-items-evidence.md` §O3(边界已存在于 `App::update` 分派) |
| O4 | `secrets`、`external_apps`、`capabilities` 的归属 | 暂存 host，登记候选去向；证据:`bytehost-H0/04-shared-modules.md` §5(三者不是同一类) |
| O5 | 移除 `PanelKind` 后，面板**默认栏位**由注册信息还是产品 composition root 决定 | 未决；影响 registry 的数据形状；证据:`bytehost-H0/01-panelkind.md` §5 |
| O6 | **Terminal** 是否纳入当前共享范围 | 架构上更像独立面板，但 Digger 真实需求未确认；不确认前不进共享清单；证据:`bytehost-H0/06-open-items-evidence.md` §O6 |
| O7 | `conversation`/`transcript` 等领域模型最终归属 | 要求文档 Q7，H0 审计；证据:`bytehost-H0/04-shared-modules.md` §3(领域类型已在 `dozer-core::protocol`) |
| O8 | `git_accounts` 归 bytegit 还是独立服务 | 未定；证据:`bytehost-H0/04-shared-modules.md` §3(倾向留产品) |
| O9 | 事件总线承载（复用 `dozerd` UDS 还是进程内 channel）与降级约定 | 要求文档 Q6、Q9，另行设计 |
| O10 | host 物理形态：先在 `dozer-app` 内以模块边界成立，还是直接建 `bytehost` crate；以及独立仓库（按 bytegit/byteui 的 tag 方式）的时机 | 倾向"先模块边界 + 门禁，后物理拆分"；时机待 H0 |
| O11 | Digger 的 Todo 语义（选题 vs 验收项）与对非编码 agent 的摄取支持 | 要求文档 Q11，待产品确认；影响 Todo 是否能原样共享 |
| O12 | 一个面板 + host 可运行的验收夹具（最小 host + fake 服务）具体形态 | H0 只定义形式 |

## 9. 风险

- **"剩余即 host"会让 host 变成新的垃圾场。** 缓解：E3 登记清单 + 寄存条目标注候选去向，只减不增。
- **按目录而不是按机制搬运。** `app/`（约 1.7 万行）和 `platform/` 里 host 机制与面板专属代码混杂；整目录搬运会把面板特判一并带进 host，违反 E1/E2。缓解：H0 逐文件裁决。
- **归属假设未经验证就固化。** 缓解：§5.3，H0 之前不写 H1 以后的计划。
- **`PanelKind` 973 处引用的迁移成本**（其中约 82 行是 Project/Files 预览窗格二选一，可先机械收口，见 `bytehost-H0/01-panelkind.md` B0）。缓解：Q8 的分类（"遍历全部面板" / "特判某面板" / "仅类型传递"）作为 H0 的审计产出，分批迁移。
- **预览 WebView 恒在 GPU 内容之上**这一约束使"通用 Surface 几何同步"成为 host 的硬需求：旧模式浮层需要 `App::preview_desired` 显式隐藏，host 化时这个耦合点要么随旧浮层迁走，要么进 host 的几何同步机制，不能被遗漏。
- **Digger 尚未接入：** host 的抽象全部来自 Dozer 的单一消费者，容易过拟合。缓解：一期不预设 trait；等 Digger 接入后按真实差异再抽象。

## 10. 下一步

1. 评审本规格，确认 §2 的三条硬边界、§3 的归属裁决与 §8 的未决项。
2. **H0（只读审计与迁移清单）已完成（2026-10-03）**，计划 `docs/superpowers/plans/2026-10-03-bytehost-h0-audit.md`，汇总 `docs/dozer-v2/bytehost-H0/00-summary.md`。H0 新发现需在评审时一并确认：(a) `dozer-core::protocol`（3085 行 Dozer 领域类型）的归属；(b) §3.3 应改为"overlay 宿主壳随面板走，窗口机制归 host"（`05-platform-overlays.md` §3）；(c) `HoverId` 是 `PanelKind` 之外第二处点名面板的 host 枚举。H0 的范围原为：
   - 各候选模块对 `App`、`Workspace` 与其他面板的依赖统计；
   - 顶层共享模型（`conversation`/`project`/`transcript` 等）及 host 反向调用面板的审计，覆盖要求文档 Q7、Q8、Q13；
   - 面板边界门禁设计（§6 第 4 条）；
   - `platform/` 下各面板专属 overlay 的迁移清单；
   - E3 登记清单的初版。
3. 评审 H0 汇总 → 从 `00-summary.md` §4 的候选切片里选定 H1 → 写 H1 plan（候选 1、2 无前置未决项、不改行为）。
