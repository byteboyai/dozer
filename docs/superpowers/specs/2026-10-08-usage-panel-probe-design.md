# Usage 面板可行性探针规格(O12)

> 日期:2026-10-08。性质:**探针规格,不是迁移规格**。只定义"一个面板 + 最小 host + fake 服务能跑"的形式、验收场景和产出物,**不改任何生产代码,不动 `PanelKind`,不做 H8**。
> 来源:`docs/dozer-v2/bytehost-H8-evaluation.md` §5 的 A+B 方案(用户 2026-10-08 裁决:停在 H7a,先做 B)。O12 的原始定义见 `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md` §O12 与 `docs/dozer-v2/bytehost-H0/00-summary.md` §4 第 5 项。
> 状态:探针已执行(2026-10-08),结论见 `docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`。
> 本文里"读代码得到"的事实带 `文件:行号`,**未经运行验证**;探针执行后以其结论为准。

## 0. 对目标的理解(待你纠正)

- **要回答的问题:** Digger 这类"第二个消费者"想复用 Usage 面板时,host 实际要提供哪些能力?
- **探针的产出是一份缺口清单,不是可运行的 host。** 清单决定 H8 的真实范围、O7(`dozer-core::protocol` 归属)是否要提前解、以及 H7b/H8 值不值得排期。
- **选 Usage 的理由(你已定):** 它是 12 个面板里数据流最单一的一个(读 dozerd 汇总 + 读本地 git 提交数 → 推给 webview 渲染),没有写操作、没有跨面板命令。
- **假设(需确认):** 探针是**无头的**(测试夹具,不开真实窗口)。真实 webview 的几何、遮挡、IPC nonce 不在探针范围内,只记录"host 为它做了什么"。

## 1. 先说读代码得到的关键事实:Usage 并不"只是一个面板"

Usage 之所以适合当探针,恰恰因为它把 host 的几种典型介入都占全了。下表是 host 里认识 Usage 的全部位置(读代码得到,未运行验证):

| 类别 | 位置 | host 在替面板做什么 |
|---|---|---|
| 状态归属 | `workspace/state.rs:458,721,1076` | `Workspace.usage` 字段(按项目);`app/app.rs:461,860` `App.usage_webview`(App 级,**不按项目**) |
| 消息路由 | `app/message.rs:64`、`app/update.rs:1621-1633` | `Loaded` 按 `project_id` 经 `with_project` 路由(**丢弃过期结果是 host 做的,不是面板**);`ToggleListCollapse`/`Hover` 被 host 拦截;其余消息发给"当前聚焦项目" |
| 切入钩子 | `app/update.rs:5251` | host 构造 `ActivationCtx` 再调 `usage::on_activate`(H6 已迁) |
| 视图布局 | `app/view.rs:965-1010` | host 决定"有无筛选栏/是否折叠/左右镜像/分栏比例",面板只给 `content_pane`/`list_pane` 两块 |
| 布局状态 | `app/layout.rs:173,191,209,224,644,759`;`dims.usage_split`/`usage_list_collapsed` | 分隔条 `Divider::UsageSplit`、折叠态、拖拽;约 60 行黄金表用例(`layout.rs:1326-1345,1416-1435`)点名 Usage |
| webview 几何 | `webview_geometry.rs:581`(`usage_content_pane_bounds_for`)及 134/267 | host 算 webview 矩形,含头部高度 `usage_content_chrome_top_px` |
| webview 身份 | `app/app.rs:659` `USAGE_CONTENT_ID_OFFSET=4_000_000`、`app.rs:3777` | **固定单槽**的 webview id;创建时机与规格由 host 写死 |
| webview 入站 | `runtime.rs:705-719`、`app/update.rs:261-267` | 按固定 webview id 识别 IPC,解析成 `UsageWebviewEvent`,`Ready` 时 host 调 `usage_webview.set_ready` |
| webview 出站 | `app/app.rs:1242-1266` `take_usage_content_script`、`platform/window_events.rs:2558` | 每帧算 desired payload、比较 `last_sent`、组 envelope、`evaluate_script`——**推送节奏由 host 的帧循环驱动** |
| 资源服务 | `assets.rs:175,293` | `dozer://usage-content` 静态资源,严格 CSP |
| 清单 | `product.rs:26,30` | 描述符与默认右栏(H7a 已清单化) |

面板自己的代码是 `extensions/usage/`(约 2100 行,4 个文件)。它对 host 的**显式**依赖已经很窄(H1/H4/H6 的成果):

- `PanelHost`(`panel_host.rs:167`):`hover_progress`/`list_collapsed`/`list_collapse_button`/`window_size`/`last_cursor`
- `ActivationCtx`/`PanelIo`:`client()`、`handle()`、`emitter()`
- 其余**隐式**依赖(未经契约):`crate::chrome::homespace::home_panel_head_with_actions`、`byteui::interaction::icons`、`crate::conversation::ConversationMeta`、`dozer_core::protocol::AgentKind`、`crate::app::{HoverId, PanelKind}`

**已有证据:** `panel_host.rs:320` 已经有一个测试,用 fake host 构造 `usage::content_pane`;`panel_host::testing::{TIMEOUT, offline_ctx, runtime}` 已经能让 `on_activate` 在假 `Client` 上跑。探针是在这两个现成零件上扩展,不是从零开始。

## 2. 探针的"形式"

### 2.1 最小 host(`ProbeHost`)

一个只在测试里存在的夹具,**不是生产 crate,不是二进制**。它必须提供且仅提供下列能力;每一项在探针结束时都要标注"面板用到了"或"面板没用到":

| # | 能力 | 探针里的实现 |
|---|---|---|
| C1 | 视图契约 `PanelHost` | 一个 `FakePanelHost`(固定窗口尺寸、可设折叠态、悬停恒为 0) |
| C2 | 激活上下文 `ActivationCtx` | `project_id` + `project_path` + 指向 fake 服务的 `PanelIo` |
| C3 | 消息投递 | `PanelIo` 的 `emit` 收进 channel;夹具把消息喂回 `usage::update`(**模拟 host 的路由职责,显式记录哪些路由是夹具替面板补的**) |
| C4 | webview 表面(`FakeSurface`) | 记录 `ready` 状态、收到的每条 envelope;可手动触发 `Ready`/断开重建 |
| C5 | 逐帧推送驱动 | 夹具里的 `tick()`,调 `usage_webview.pending_push` → `encode_usage_push` → `FakeSurface`——**照搬 `take_usage_content_script` 的逻辑,并标注哪几行是 host 独有的** |

### 2.2 fake 服务

Usage 有且仅有两个数据源:

1. **dozerd 汇总**(`client.get_usage_summary(&cwd, None)`,`dozer-client/src/lib.rs:1135`)。现状是**具体类型 `Client`**,没有 trait 接缝。
   - **探针方案(推荐):** 起一个 fake UDS 服务,讲真实协议、返回预设数据。零生产代码改动;代价是夹具要会说 `dozer-core` 的 UDS 协议。
   - **备选:** 在 `usage` 里引入 `UsageSource` trait。**这属于改生产代码,不在本探针范围**,只作为"缺口:面板的数据服务没有接缝"记入报告。
2. **项目 git 提交数**(`bytegit::Repo::discover` → `commit_count`/`commit_count_by_day`)。**不需要 fake**:用 `bytegit` 自带的 `TempRepo` 夹具造真实仓库。

### 2.3 验收场景

每个场景给出"期望"和"如果期望不成立意味着什么":

| # | 场景 | 期望 | 不成立意味着 |
|---|---|---|---|
| S1 | 切入 → fake 服务返回 3 个会话 → `Loaded` | `loading` 先真后假;`rows` 落地;下一次 `tick` 推出一条 payload | 面板的"加载→就绪"依赖了 host 未暴露的东西 |
| S2 | 设置 agent 筛选 | payload 变化,且只在变化时推送一次 | 推送去重逻辑与 host 帧循环耦合 |
| S3 | 项目无路径 | `loading=true` 且不发请求(迁移前行为) | — |
| S4 | fake 服务不可达 | `rows` 为空、不 panic、`loading` 最终为假(**注意:现状 `unwrap_or_default` 会吞掉错误**) | 记为缺口:Q6(服务缺席降级约定)在 Usage 上的具体形态 |
| S5 | webview 重建(`Ready` 再次到达) | `last_sent` 清空,同一内容**重发** | — |
| S6 | 先后发出两次刷新,较早的结果后到 | 面板自己**无法**识别过期(过期丢弃在 host 的 `with_project`) | 记为缺口:"跨项目/过期结果的路由职责"归谁 |
| S7 | `ToggleListCollapse`/`Hover` 消息 | 面板 `update` 里这两个分支是 `unreachable!`(`mod.rs:134-141`)——夹具若误投会 panic | 记为缺口:面板 `Message` 里混着"只能由 host 处理"的变体 |
| S8 | 没有 `Workspace`、没有 `App`、没有 `PanelKind::Usage` 以外的任何面板 | 夹具能编译、能跑 | 若必须引入别的面板的符号才能编译,记为 R1 违例 |

### 2.4 编译期依赖的度量(探针的客观产出之一)

对 `extensions/usage/` 用 `scripts/audit/` 现有的依赖边提取器出一份"Usage → 其余 crate 模块"的边清单,按三类标注:

- **A. 已有契约覆盖**(`PanelHost`/`PanelIo`/`ActivationCtx`)
- **B. 通用 UI 件,应进 SDK 或 byteui**(`home_panel_head_with_actions`、`icons`、`HoverId`、`theme`)
- **C. 领域类型,归属待定**(`ConversationMeta`、`AgentKind`、`dozer-core::protocol::*`)——这一类直接对应 O7

## 3. 探针的产出物

1. **缺口报告** `docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`(随探针执行产出):每个缺口一行——现象、证据(场景编号 + 文件行)、归类(见 §4)、是否阻塞"第二个消费者"。
2. **依赖边清单**(§2.4 的 A/B/C 三类)。
3. **一句话结论**:"Usage + 最小 host 能跑吗?"——能 / 能但需要列表中的 N 项 host 能力 / 不能,因为 X。
4. **对 Q-H8 的回填建议**:H8 的真实范围(哪几类 host 介入需要动态化,哪几类只是需要更好的契约)。

## 4. 读代码预判的缺口(待探针证实或证伪)

这些是**假设**,不是结论;探针的价值就在于验证它们:

| 编号 | 假设 | 归类 |
|---|---|---|
| G1 | webview 的"身份、几何、入站 IPC、出站推送"全部是 host 里按 `PanelKind::Usage` 写死的一整套(上表 6 行),面板只提供 payload 与解析函数。第二个消费者要用 Usage 的 webview 内容,就必须有一个**通用的"面板 webview 表面"契约**。这很可能是 H8 之外最大的一块。 | 缺契约 |
| G2 | 推送是**每帧声明式比较**(`take_usage_content_script`),逻辑在 `App` 上,不在面板里。 | 职责错位 |
| G3 | 过期结果的丢弃(S6)与"刷新只对当前项目生效"在 host。面板单独跑时不具备。 | 职责错位 |
| G4 | 数据服务是具体 `Client`,没有接缝;错误被 `unwrap_or_default` 吞掉。 | 缺接缝 / 缺降级约定(Q6) |
| G5 | `Message` 里有只能由 host 处理的变体(S7)。 | 契约污染 |
| G6 | `ConversationMeta`/`AgentKind` 属于 O7 范围,Usage 离开 `dozer-app` 就必须解决。 | O7 |
| G7 | 分栏(`usage_split`、折叠、镜像)是 host 的布局状态,面板只有"两块 pane"的约定,没有"我是两栏面板"的声明。 | 缺声明 |
| G8 | Usage 视图借用 `workspace::agent_icon`/`agent_dot_color`(规格 §1 漏列) | 缺契约(通用 UI helper 住在 Workspace 模块) |

若 G1–G7 里有 ≥3 条被证实且归类为"缺契约/缺接缝",则 H8 的范围应当**缩小为"补齐这几块契约"**而不是全量动态化。这是对 H8 评估 §3 "收益为零"判断的直接检验。

## 5. 非目标

- 不迁移 Usage 的任何代码,不抽 crate,不新增 trait(`UsageSource` 等只作为缺口记录)。
- 不碰 `PanelKind`、`Message`、`PanelDims`、布局黄金表。
- 不验证真实 webview(几何、遮挡、nonce、CSP)。
- 不解 O7;只标注 Usage 触到了它。
- 不评估 Digger 的具体需求(O11)。

## 6. 待你裁决

- **D1(推荐:无头夹具):** 探针是只在 `#[cfg(test)]` 里的夹具,还是另做一个 `examples/` 小二进制开真实窗口?后者能看到 G1 的真实代价,但要把 webview 池、窗口循环一起拉进来,几乎等于重做一个 host,与"最小"相悖。
- **D2(推荐:fake UDS):** 数据服务的 fake 方式。fake UDS 服务不改生产代码;`UsageSource` trait 要改。
- **D3:** 缺口报告放 `docs/dozer-v2/bytehost-H0/`(与 H0 系列同处)还是新开 `bytehost-probe/`?我默认前者。

## 7. 执行估算与后续

- 探针执行(夹具 + 8 个场景 + 依赖边 + 缺口报告)约 1–2 天量级。
- 完成后,按报告结论回到 H8 评估的 Q-H8-2/3:若 G1/G3/G4 成立,优先补契约(webview 表面、过期结果路由、服务接缝),而不是做全量动态化。
- 本规格批准后用 `writing-plans` 出执行计划。**建议在独立分支做**(项目惯例:plan 要求独立分支 + 审阅后合并)。
