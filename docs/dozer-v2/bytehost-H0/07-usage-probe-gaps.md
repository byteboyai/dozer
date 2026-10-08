# Usage 面板探针:缺口报告(O12)

> 日期:2026-10-08。规格:`docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md`;计划:`docs/superpowers/plans/2026-10-08-usage-panel-probe.md`。
> 夹具:`crates/dozer-app/src/usage_probe/`(仅测试,分支 `probe/usage-o12`)。`cargo test -p dozer-app usage` 87 个全绿,其中探针 20 个(夹具自测 10 + 场景 10)。
> 本报告所有结论来自夹具实测;"是否需要真实 webview"一类标注了**未验证**。

## 1. 一句话结论

**能跑,但需要 host 提供一批面板自己做不了、且目前没有契约的能力:夹具记录了 9 条 host 职责,其中 7 条无契约且面板做不了,折算成约 4 个不同的能力域(见 §2)。** S1、S2、S3、S5、S8 全部通过,没有为绕过可见性问题改动任何生产代码(`git diff --stat main -- crates/` 只有 `main.rs` +2 行)。面板的 `content_pane`/`list_pane` 只靠 `PanelHost` 假实现就能构造,`on_activate` 只靠 `ActivationCtx`+`PanelIo` 就能跑——**面板本体已经独立;缺的是 host 围着它做的那一圈事。**

## 2. host 能力清单(`ProbeHost.duties`)

| Duty | 真实 host 位置 | 面板能自己做? | 契约现状 | 第二个消费者必须自己提供? |
|---|---|---|---|---|
| BuildActivationCtx | app/update.rs:5250 | 不能(要 client/runtime/emit) | **已有**(`ActivationCtx`/`PanelIo`) | 是,但契约已在 |
| RouteLoaded | app/update.rs:1621 | 不能(按 project_id 路由到 Workspace) | 无 | 是 |
| DropOtherProject | app/update.rs:1621 | 不能(面板 `update` 不看 project_id,见 S6) | 无 | 是 |
| InterceptCollapse | app/update.rs:1626 | 不能(变体只能 host 处理,见 S7) | 无 | 是 |
| InterceptHover | app/update.rs:1629 | 不能(同上) | 无 | 是 |
| FramePush | app/app.rs:1242 | 不能(逐帧驱动在 `App`) | 无 | 是 |
| DowngradeReady | app/app.rs:1251 | 不能(依赖 webview 池存活) | 无 | 是 |
| SkipWhileLoading | app/app.rs:1257 | **能**(`loading()` 是面板自己的访问器,可下沉进 `WebviewPushState::pending_push`) | — | 否 |
| ParseIpcAndMarkReady | runtime.rs:705 + app/update.rs:261 | 解析函数在面板;"按 webview id 识别并转发"不能 | 无 | 是 |

**记录了 9 条职责,其中 7 条"无契约且面板做不了"**(RouteLoaded、DropOtherProject、InterceptCollapse、InterceptHover、FramePush、DowngradeReady、ParseIpcAndMarkReady)。这 7 条是夹具的记账条目,不是 7 个独立能力;按代码位置与性质折算,**约 4 个能力域**:

1. **结果路由**(RouteLoaded + DropOtherProject,同一处 `with_project`,update.rs:1621)。
2. **host 专属消息拦截**(InterceptCollapse + InterceptHover,即 G5;若把这两个变体移出面板 `Message`,这两条职责**消失**而不是需要契约)。
3. **逐帧推送 + ready 生命周期**(FramePush + DowngradeReady,同一函数 `take_usage_content_script`)。
4. **IPC 识别与转发**(ParseIpcAndMarkReady)。

其中第 2 项建议直接消除而非补契约;真正要补契约的是第 1、3、4 项。
注:"面板做不了"是夹具**按构造**得出的判断(这些职责由 `ProbeHost` 代做),探针本身不能证明面板"不可能"做,只能证明它目前没有做、也没有契约让它做。

## 3. G1–G8 判定

| 编号 | 假设 | 证据 | 结论 |
|---|---|---|---|
| G1 | webview 的身份/几何/入站/出站全是 host 里按 `PanelKind::Usage` 写死的一整套,缺通用"面板 webview 表面"契约 | `FramePush`、`DowngradeReady`、`ParseIpcAndMarkReady` 三项均"面板不能自己做"(§2);`FakeSurface` 只需 attach/detach/receive 三个动作即可模拟 | **CONFIRMED**(夹具只证明了契约的形状;几何、nonce、CSP 的真实代价**未验证**) |
| G2 | 推送是每帧声明式比较,逻辑在 `App` 上 | `drift_guard_tick_mirrors_take_usage_content_script` 通过:`ProbeHost::tick` 必须手抄 8 步顺序;面板只提供 `WebviewPushState` 的去重 | **CONFIRMED** |
| G3a | 跨项目路由("只对对应项目生效")在 host | `s6_characterizes_g3_*`:面板 `update` 收下 project_id=99 的结果;夹具里是 `ProbeHost::deliver` 丢弃;真实 `with_project`(app/app.rs:1626)按 project_id 找 Workspace | **CONFIRMED** |
| G3b | 过期结果(同一项目、较早发起的刷新后到)由 host 丢弃 | 同一测试:后到的旧 `Loaded` 覆盖新结果。**读 `with_project`:它只按 project_id 找 Workspace 再应用,没有序号/revision 检查——host 也不丢弃** | **REFUTED(原假设错)——这是一个真实的生产缺陷,不是"职责放错了地方"**:快速切入 Usage、离开、再切入会发起两次 `spawn_refresh`,较慢的第一次后到时会覆盖较新的数据。**已修复(分支 `fix/usage-stale-refresh`):`Loaded` 带刷新代数,面板 `update` 丢弃代数低于最新的结果;S6 已翻转为 `s6_stale_refresh_is_dropped_by_the_panel_itself`,跨项目部分保留为 `s6_characterizes_g3a_*`** |
| G4 | 数据服务是具体 `Client`,无接缝;错误被吞 | `s4_characterizes_g4_service_failure_looks_like_no_data`:`HangUp` 与 `ErrorReply` 都得到 `empty` payload;fake UDS 是不改生产代码的唯一手段(`FakeDozerd` 讲真实协议,零改动可用) | **CONFIRMED** |
| G5 | `Message` 混有只能由 host 处理的变体 | 两个 `s7_characterizes_g5_*` 的 `should_panic("由内核拦截处理")` 均成立 | **CONFIRMED** |
| G6 | `ConversationMeta`/`AgentKind` 属 O7 | 依赖边表有 `C` 类 2 项:`conversation::ConversationMeta`(6 次)、`dozer_core::protocol`(7 次) | **CONFIRMED** |
| G7 | 分栏(`usage_split`、折叠、镜像)是 host 的布局状态 | 夹具没实现任何分栏/折叠状态,S1–S5 照样跑通 → 面板逻辑不依赖它;但"布局由 host 持有"这一点来自读代码 | **PARTIAL**(未由探针直接验证) |
| G8 | (规格漏列)`view.rs` 借用 `workspace::agent_icon`/`agent_dot_color` | 依赖边表 `B` 类有这两项(各 1 次,均在 view.rs) | **CONFIRMED** |

## 4. 意外发现

- **无场景级意外:** S1–S8 首次运行全部通过,三个特征化缺口(G3/G4/G5)按预判成立,没有为绕过可见性问题改动生产代码。
- **一个计划缺陷(已处理):** 计划里夹具自测把 `tokio::time::timeout` 构造在 `block_on` 之外,无 reactor 而 panic;改为 `async` 块包裹(仅测试代码)。
- **构建副产物:** 在 worktree 里构建会让 `Cargo.lock` 多出 `bytegit`/`byteui` 的 `source` 行(被跟踪的 `.cargo/config.toml` 用 `../byteui` 路径 patch 在 worktree 里解析不同所致),与本探针无关,未提交。
- **`list_pane` 与 `content_pane` 不对称:** `content_pane` 吃 `&impl PanelHost`,`list_pane` 不吃任何 host——后者已经是"纯视图"。

## 5. 依赖边清单(`scripts/audit/unit_edges.py ext:usage`)

| 类 | 模块 | 符号 | 出现文件 | 次数 |
|---|---|---|---|---|
| A | `panel_host` | `ActivationCtx` | mod.rs | 1 |
| A | `panel_host` | `PanelHost` | view.rs | 1 |
| B | `app` | `HoverId` | mod.rs,view.rs | 3 |
| B | `app` | `PanelKind` | view.rs | 4 |
| B | `byteui` | `feedback` | view.rs | 2 |
| B | `byteui` | `interaction` | view.rs | 1 |
| B | `byteui` | `theme` | view.rs | 13 |
| B | `chrome` | `homespace` | view.rs | 2 |
| B | `workspace` | `agent_dot_color` | view.rs | 1 |
| B | `workspace` | `agent_icon` | view.rs | 1 |
| C | `conversation` | `ConversationMeta` | aggregate.rs,mod.rs,protocol.rs,view.rs | 6 |
| C | `dozer_core` | `protocol` | aggregate.rs,mod.rs,protocol.rs,view.rs | 7 |
| S | `bytegit` | `Repo` | mod.rs | 2 |
| S | `dozer_client` | `Client` | mod.rs | 1 |
| T | `panel_host` | `testing` | mod.rs | 3 |

计数:A 2 项,B 8 项,C 2 项,S 2 项,T 1 项。注意:该提取器只看 `use crate::…`/`crate::…` 与四个外部 crate 前缀,`ctx.io`(`PanelIo`)这类经参数传入的依赖不会出现。

- **A**(已有契约)只有 `PanelHost`、`ActivationCtx`。
- **B**(通用 UI 件,应进 SDK 或 byteui)占多数:`HoverId`/`PanelKind`、`chrome::homespace`、`workspace::agent_icon`/`agent_dot_color`、`byteui`。
- **C**(领域类型)2 项,对应 O7。
- **S**(数据服务)2 项:`dozer_client::Client`、`bytegit::Repo`,都是具体类型。

## 6. 对 Q-H8 的回填建议

规格 §4 的判据写的是"**G1–G7** 里有 ≥3 条被证实且归类为缺契约/缺接缝"。**按规格原文,实测只有 2 条(G1 缺契约、G4 缺接缝),未达阈值。** 若把规格之外新增的 G8(通用 UI helper 住在 `workspace` 模块,性质更接近"归位"而非"缺契约")计入则为 3 条,但那是对判据的放宽。另外 G4 的"无接缝"也被探针削弱了:`Client::new(socket)` + `FakeDozerd` 本身就是一条可用的进程边界接缝,真正缺的是进程内 trait 接缝,以及"无数据 / 取数失败"的区分。

因此下面的建议是**判断,不是阈值推出的结论**:证据支持"缺口集中在 host 的 webview 与路由围栏,而不是 2.7 万行文件的状态结构",所以 H8 若要做,范围应缩到补契约;但是否值得排期仍取决于 Q-H8-2(Digger 复用的时间点)。

建议:

1. **H8 不做全量动态化**,缩小为补齐下列四块契约(按收益排序):
   - **面板 webview 表面契约**(G1/G2):身份、几何、入站事件、出站推送。对应上面"约 4 个能力域"里的第 3、4 项,也是最大的一块。
   - **面板结果路由契约**(G3a/G3b):给面板的异步结果带"项目 + 序号",让过期/跨项目判断可以在契约层完成。**其中 G3b 是现存的生产缺陷,已单独修复(见 G3b 行);G3a 仍需契约。**
   - **数据服务接缝与降级约定**(G4,即 Q6):区分"没有数据"与"取数失败";引入接缝**需改生产代码,探针未做**。
   - **通用 UI helper 归位**(G8/B 类):`agent_icon`/`agent_dot_color` 等从 `workspace` 挪到 SDK/byteui。
   - 另:把 host 专属变体(`ToggleListCollapse`/`Hover`)从面板 `Message` 里分离(G5)。
2. **O7 应提前解**(G6):Usage 一离开 `dozer-app` 就同时被 `conversation::ConversationMeta` 和 `dozer_core::protocol` 卡住。若 Digger 复用面板是近期要事,O7 比 H8 更先。
3. **仍未回答的:** 真实 webview 下 G1 的代价(几何、遮挡、nonce、CSP)。探针是无头的,这部分需要第二个探针或在补契约时验证。
4. 与 H8 评估 §3 的关系:该评估判断"收益为零";探针表明**面板本体已独立、缺口集中在 host 的 webview 与路由围栏上**,所以收益并非为零,但**对应的是几块契约而不是 2.7 万行文件的重写**。
