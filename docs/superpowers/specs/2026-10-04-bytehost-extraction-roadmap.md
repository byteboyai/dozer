# bytehost 拆分路线图:先应用宿主,再把 iced 宿主移出 dozer-app

> 状态:**路线图(用户 2026-10-04 已裁决三项形态问题),不是实施计划。** 每个切片开工前仍要单独写 plan、单独评审、单独合并。
> 目标(用户原话):"先处理好 bytehost,然后把 host 部分从 dozer 中去掉;UML 面板暂时不处理。"
> 依据:`2026-10-03-bytehost-boundary-design.md`(E1/E2/E3、归属)、`2026-10-04-bytehost-panel-hooks-and-registry-design.md`(H1–H7a 已完成)、`docs/dozer-v2/bytehost-H8-evaluation.md`、`2026-10-04-bytehost-app-host-design.md`(应用宿主,A0–A5)。

## 1. 已定裁决(2026-10-04,用户)

1. **形态:先在 dozer 仓库的 workspace 里建 crate,边界稳定后再迁独立仓库**(`byteboyai/bytehost`,届时像 byteui/bytegit 一样按 tag 引用)。理由:host 与 Dozer 目前耦合很深(协议类型、`Client`、Preview),跨仓库每步都要双向发版联调,迁移期成本过高。
2. **顺序:先做应用宿主的无界面部分(A0–A2),再做 iced 宿主的结构拆分。** A0–A2 是新建 crate、不碰现有 host 代码,风险低、尽早有产出;其中 A3(rail 动态条目 = H7b 最小版)恰好是结构拆分的第一步,之后沿用同一个 crate 边界。
3. **协议归属(O7):host 只依赖抽象的服务接口,Dozer 提供实现。** bytehost 定义面板需要的服务 trait(按需最小化),`dozer-app` 用 `dozer-client` 实现;`dozer-core::protocol` 留在 Dozer,host 不依赖它。
4. UML 面板暂不处理。

## 2. "把 host 从 dozer 里去掉"到底有多大(实测,`crates/dozer-app/src`)

| 目录/文件 | 行数 | 属于 host? |
|---|---|---|
| `app/`(update 6.4k、app 6.2k、view/layout/state/message) | 17.4k | 大部分是 host,但里面有 153 条面板专属消息臂(§3 的 H8 对象) |
| `platform/`(22 个文件) | 9.5k | 通用 overlay 机制归 host;约 11 个是面板专属 overlay(产品层) |
| `chrome/`(rail、顶栏、首页等) | 5.1k | host(首页内容偏产品) |
| `workspace/`(Workspace、tabs、view) | 7.6k | host 骨架 + 44 个面板状态字段(H8 对象) |
| `term/` | 2.1k | 待 O1/O6 裁决(终端是否随 host) |
| 根下:`webview_geometry` 2.1k、`runtime` 0.8k、`panel_host`、`panel_registry`、`keymap`、`menu_spec`、`theme`、`layout`、`panel_layouts`、`capabilities`… | 约 6k | 多数是 host |
| `preview/`(Preview 业务) | 14.1k | **留 Dozer**(边界规格 §3.2) |
| `extensions/`(各面板) | 43.6k | **留 Dozer**(面板是 host 的消费者) |

**host 相关合计约 4–5 万行,与面板、Preview、协议类型交织。** 这不是一次迁移能完成的事;下面按"边界先于搬家"的顺序分阶段,每一步行为不变。

## 3. 阶段

### 阶段 1:应用宿主的无界面部分(A0–A2,已有设计 `bytehost-app-host-design.md`)

- **A0** 新建 `crates/bytehost-apps`(无 feature:类型与纯逻辑;`digest` feature:摘要)。 **已完成(`bytehost-a0`):`6ebec5fe`。**
- **A1** `server` feature:`AppManager`、`static_web` runtime、gateway(Host 校验、固定端口)、各 runtime 的 `probe`。前置验证 V1。 **已完成(`bytehost-a1`):`283ff056`。**
- **A2** 接入 dozerd:`dozer-core::protocol` 加 `App(..)` 变体、`dozerd/server.rs` 转发、`dozer-client` 加 `app_*`、启动对账/退出清理。
- 这一阶段结束时:Dozer 里能装静态 Web 应用并由 dozerd 提供服务(GUI 入口在阶段 2 的 A3/A4)。

### 阶段 2:iced 宿主的结构拆分(每一步都是单独的 plan)

顺序是**先让边界成立,再搬代码**:

| 步 | 内容 | 为什么在这个位置 |
|---|---|---|
| X0 | **服务接口盘点与定义(O7):** 逐个列出 host(`app/`、`workspace/`、`platform/`、`chrome/`)里对 `dozer_client::Client`、`dozer_core::protocol::*` 的使用,把"面板需要的服务"收敛成最小 trait 集;只定义接口与适配层,不搬代码 | 这是搬家的硬前置;不先做,bytehost crate 会反向依赖 dozer-core |
| X1 | **A3 / H7b-min:** rail/布局条目 id 能表达 `app:<id>`,按应用 id 存独立 WebView 状态,落盘兼容 | 动态条目是应用面板的前提,也是 H8 的最小试点 |
| X2 | **新建 `crates/bytehost`(iced 宿主,有界面)骨架,并迁入最底层、已解耦的叶子模块**:`panel_host`、`panel_registry`、`HoverId`/`HoverSlot`、`PanelDims` 访问器、`Toast` 机制、`overlay_window` 机制 | 这些在 H1–H7a 里已经与产品解耦,最容易搬,能先验证 crate 边界与门禁 |
| X3 | **Preview 解耦(S4/S5):** webview id/URL 里编码面板名的地方改成通用的"表面(Surface)"标识;Files Tree 只发"打开目标"通用命令(O3) | Preview 留 Dozer,但 host 的 webview 机制必须不认识 Project/Files |
| X4 | **H8 的最小必要部分:** 面板状态与消息信封从"点名 12 个面板"改为按面板 id 取的容器——**只做 host 骨架需要的部分**,不一次动全量(范围由 X1 的试点数据决定) | 评估(`bytehost-H8-evaluation.md`)已说明这是最大风险项,放在边界与试点之后 |
| X5 | **搬 host 骨架:** `app/` 的壳(窗口、布局、消息分发)、`workspace/` 的 tab/栏位容器、`chrome/` 的 rail/顶栏、`webview_geometry`、`window_events` 里的通用部分 | 此时依赖都已是 trait/id,搬家是机械的 |
| X6 | **Dozer 变成消费者:** `dozer-app` 只剩 composition root(`product.rs`)、各面板(`extensions/`)、Preview、产品设置与首页;`Cargo` 里依赖 `bytehost` | 达成"host 部分从 dozer 中去掉" |
| X7 | **(可选)迁独立仓库** `byteboyai/bytehost`,按 tag 引用;同时 `bytehost-apps` 一并迁出 | 边界稳定、Digger 真要用时再做 |

### 阶段间的闸门

- 每个切片:全量测试通过数 = 基线 + 新增;clippy 诊断逐文件与基线一致;门禁(`scripts/audit/check_panel_boundary.py`)只降不升。
- 阶段 2 每一步开工前,先复测 H8 评估里的度量(`PanelKind` 引用、`update.rs` 面板臂数、`Workspace` 面板字段数),确认该步确实让数字下降,而不是只增加新边界。
- X4 之前不承诺"host 变小":H1–H7a 的实测是 `app/update.rs` 只少了 1.5%。

## 4. 未决项(阶段 2 之前必须回答)

| # | 问题 | 何时必须定 |
|---|---|---|
| T1 | `term/`(2.1k)与 Agent/Ssh 面板不可分(O1/O6):终端区随 host 走,还是留 Dozer? | X5 之前;Digger 是否需要终端是产品问题 |
| T2 | 首页(`chrome/homespace`)与项目页签属于 host 还是产品? | X5 之前 |
| T3 | `dozer-core::log`、`paths` 等"产品无关机制"的归属(留 dozer-core,还是 bytehost 自带) | X2 之前 |
| T4 | `secrets`/`external_apps`/`capabilities`/`git_accounts`(边界规格 §3.5 寄存项)的最终归属 | X5 之前 |
| T5 | host 对面板的服务 trait 的粒度:一个大 `HostServices` 还是按能力拆小 trait | X0 内定 |

## 5. 对时间的诚实预期

- 阶段 1(A0–A2)是几个中等切片的量级。
- 阶段 2 里 X0、X2、X3 是中等,**X4 与 X5 是大块**(涉及 `update.rs` 6.4k、`app.rs` 6.2k、`window_events.rs` 4.5k)。H1–H7a 一共做了 7 个小切片,才把"契约"立起来;搬家本身的工作量与那 7 个切片的总和同量级或更大。
- 建议在阶段 1 结束后,用阶段 2 的度量复核一次是否继续按此顺序推进。
