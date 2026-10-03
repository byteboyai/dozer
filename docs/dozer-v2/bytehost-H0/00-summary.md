# bytehost H0 汇总

> 基线提交:`6db5138`(bytehost-h0 分支;`crates/` 与 `main` 的 `4845fb85` 逐字一致,H0 未改任何产品代码)。
> 范围声明:只系统审计了 `crates/dozer-app`;`dozer-core`(Task 6 §4)、`dozerd`/`dozer-hook`/`dozer-mcp`(Task 7 的 Agent 分布)只做了局部统计。
> 子文档:`01-panelkind.md`(Q8、O5)、`02-panel-to-host.md`(Q10)、`03-host-to-panel.md`(Q13)、`04-shared-modules.md`(Q7、O4、O7、O8)、`05-platform-overlays.md`、`06-open-items-evidence.md`(O1、O2、O3、O6)、`E3-registry.tsv`(38 条)、`data/*.md`(机械快照)。
> 重跑核对:汇总前把五个报表在 `main`(`4845fb85`)上重跑,与快照 **全部 unchanged**。

## 1. 数字订正表

| 项 | 旧值(要求文档/规格) | H0 实测 | 来源 |
|---|---|---|---|
| `PanelKind` 变体数 | 11 | **12**(多 `GroupChat`) | 01 §1 |
| `PanelKind` 出现行数 | 约 887 | **973**(点名变体 763 + 仅类型 210) | 01 §1 |
| 涉及文件数 | 约 25 | **31** | 01 §1 |
| 面板代码引用 `app::App` | 暗示较多(§4.2 表列 4 个面板) | import 路径:`app::App` 8 条(7 个面板)、`workspace::Workspace` 3 条,合计 11 条;**按使用次数(含每个 `&App` 参数)共 22 处 / 9 个文件**(门禁已改按使用次数计,评审指出 import 路径口径会漏掉同文件新增的参数) | 02 §3.1、门禁基线 |
| 面板对 host 的最大依赖 | `App`、`Workspace` | **`chrome::native_menu`(34 次)、`app::HoverId`(23)、`app::TextInputTarget`(19)、`chrome::homespace`(11 次,10 个面板用)** | 02 §2 |
| `HoverId` | 未列入 | **62 个变体、188 处使用**,其中约 20 个是各面板自己的按钮——**`PanelKind` 之外的第二处"host 枚举点名面板"** | 02 §3.2 |
| host 对面板引用最重 | — | `app/update.rs`→group_chat 11、`app/app.rs`→agent_context 9(多数在测试)、`platform/window_events.rs`→files 12;`window_events.rs` 单文件 **4602 行** | 03 §2、05 §1 |
| `Project`/`Files` 预览窗格二选一 | 未列入 | **约 82 行**(53 处 `==`/`!=` + 29 个取 `project_preview` 的 match 臂;占全部 `PanelKind` 行约 8%;`Project` 相关共 114 行,另约 32 行是元数据 match)—— Preview 业务有两个实例由 `PanelKind` 选择 | 01 §2a |
| `platform/` | "约 20 个 overlay" | 22 个文件,9584 行;面板专属 overlay 宿主 11 个(其中 5 个属 Digger 复用面板) | 05 §1、§3 |
| 根下共享模块 | 3 个(conversation/delivery/project) | 21 个根模块;**3 个的名字与内容不符**:`project.rs` 实为文件树、`conversation.rs` 实为展示 IR、`transcript.rs` 实为 Claude 适配器 | 04 §1 |

## 2. 对规格三条硬边界的影响

- **E1(host 不依赖产品/面板):** 可行,但前置工作比预期大:`HoverId`/`PanelKind`/`Message` 三个 host 枚举都点名了面板(`Message` 有 20 个面板包装变体 + 5 个面板 webview 事件变体,`Workspace` 有 14 个面板状态字段,`App` 有 15 个),必须先注册制化。
- **E2(host 公共 API 无业务类型):** 需要先回答 `dozer-core::protocol`(3085 行,含全部 Dozer 领域类型)的归属,否则 host 之外的一切都依赖它(04 §4)。H0 新增的未决问题。
- **E3(特判集中登记):** 登记清单初版 **38 条**——17 条 `panel-special-case`、7 条 `parked-service`、14 条 `overlay`。"清单外无新增特判"的门禁需要稳定的识别规则(如 `PanelKind::X` 出现在 `extensions/` 之外),留 H1 设计。

## 3. 未决项进展(只写"可以收窄成什么问题",不替用户裁决)

| # | H0 证据 | 可以收窄成 |
|---|---|---|
| O1 | 06:机制已集中在 dozerd(25 个文件),散落的是 `workspace/hook.rs`(hook/MCP 安装)与 UI 纯函数;"Agent 面板"在代码里 = 终端区 + 卡片,与 Terminal 不可分 | "是否把 hook/MCP 安装划到 dozerd?"以及与 O6 合并裁决 |
| O2 | 06:权威数据在 dozerd;面板只需 `project_id` + 路径;`project.rs` 不是 project context | "host 只提供 打开集合 + 当前 id + 路径 是否够用"(证据说够) |
| O3 | 06:边界已存在于 `App::update` 的分派(`TreeRowDoubleClick`→`PreviewOpenPath`) | 三种协议形态 A/B/C 的取舍,倾向 B,A 作过渡 |
| O4 | 04 §5:三者不是同一类——`secrets`(host 服务候选)、`external_apps`(Files 私有)、`capabilities`(host) | 拆成三个独立小问题 |
| O5 | 01 §5:默认栏位连同顺序;落盘布局用 serde 枚举名且"恰 12 个"校验写死 | 注册信息 vs composition root;倾向 composition root |
| O6 | 06:`term/` 2053 行,与 O1 同一刀 | Digger 是否需要终端(产品问题) |
| O7 | 04 §3:领域类型已在 `dozer-core::protocol`;app 根下是投影 | `protocol` 本身的产品归属 |
| O8 | 04 §3:钥匙串命名空间写死 `dozer-git`、面向项目创建 | 倾向留产品 |
| O9 | 03 §4、06 O3:多处越界点都指向同一个 **Effect** 机制需求 | Effect 机制的最小形态(与事件总线 Q9 的关系) |
| O10 | 02 §3.1:`App`/`Workspace` 使用共 22 处(9 个文件),门禁基线很小 | "先模块边界 + 门禁"可行性已有数据支持 |
| O11 | 未触及 | 仍需产品确认 |
| O12 | 未触及 | H1 候选切片 5(见下)里给出形式定义 |

## 4. H1 候选范围(**不是 H1 计划**)

按"风险最低、收益最高"排序。**H1 plan 需用户在评审本文后决定做哪个再写。**

| # | 候选切片 | 涉及(H0 数据) | 前置未决项 | 风险 |
|---|---|---|---|---|
| 1 | **`preview_pane(kind)` 收口**:消掉约 82 行 Project/Files 窗格二选一(01 B0) | 13 个文件 | 无 | 低:纯机械、不改行为,只动产品层 Preview 业务 |
| 2 | **面板引用 `App`/`Workspace` 降到 0**:22 处使用改为传参/注入句柄,基线降 0 | 9 个文件 | 无 | 低–中:`hover_progress` 13 次、`list_collapsed` 6 次要引入 `HoverQuery`/参数 |
| 3 | **`window_events.rs` 内联的面板事件声明化**(E3-010/011/038):面板声明自己关心的窗口事件 | 4602 行文件中的约 80 个引用点 | 无,但要新增事件钩子接口 | 中:最大单点,是 overlay 迁移的前置(05 §4) |
| 4 | **`HoverId` 命名空间化**:把面板按钮移出 host 枚举(E3-016) | 62 变体/188 处使用 | 无 | 中:动画键与 tooltip 机制被牵动 |
| 5 | **"一个面板 + host 可运行"夹具(O12)的形式定义**:选一个面板(建议 Todo 或 Usage——纯展示、依赖少)做最小 host + fake 服务的**可行性探针**,只写形式不迁移 | 一个面板 | 依赖 2、4 的接口 | 中:验证性质,产出是 spec 不是代码 |

建议从 1 和 2 开始(都无前置、都不改行为),3 与 4 排在 Effect 机制(O9)有最小形态之后。

## 5. 门禁说明

- **怎么跑:** `python3 scripts/audit/check_panel_boundary.py`(检查,有回退退出 1)、`--update`(重写基线)。测试:`python3 scripts/audit/test_check_panel_boundary.py`、`python3 scripts/audit/test_edges.py`。
- **基线含义:** `scripts/audit/panel-boundary.baseline.json`,**`extensions/**` 里对 `App` 与 `Workspace` 的"使用次数"(import 行 + 每个 `&App` 参数/限定路径;注释与字符串字面量不计),按文件×规则记数,只许减不许增**;当前 22 处 / 9 个文件(import 路径 11 条,其余是同文件里的参数与限定路径)。测试代码里的使用同样计入。识别的写法:按名字 import(含 `{…}` 嵌套、`as` 别名、`use super::super::…`)、只 import 模块后的 `app::App`/别名模块、不 import 的 `crate::app::App` 全路径。**已知漏洞:** 通过 `use crate::app::*` glob 带入 `App`、通过类型别名(`type H = App`)间接使用、宏内路径;**棘轮不会自动收紧**——使用次数下降后,在有人运行 `--update` 之前计数可以涨回旧基线。
- **何时可 `--update`:** 引用数下降后固化新基线;或经评审的迁移引入新的合理引用。不得为"让门禁过"而 `--update` 抬高基线。
- **尚未接入 CI:** 仓库当前没有 CI 配置,门禁**手动运行**;接入前建议加进 `scripts/` 旁的现有检查(`scripts/check-log-scope.sh` 同类)的调用处。
- **变异检验已做:** 在 `extensions/toast.rs` 首行加 `use crate::app::App;`,门禁报 `R-APP 0 -> 1` 且退出 1;还原后退出 0。评审后又做一次:在 `extensions/git_log.rs` 里追加一个 `fn f(a: &App) {}`(不新增 import),门禁报 `R-APP 6 -> 7`——旧的"数 import 路径"口径抓不到这种回退。
- **E3 门禁留 H1:** "清单外无新增特判"需要先有稳定的识别规则;H0 的 E3 清单是人工审计产物,不能直接当门禁输入。
- **已知局限:** 提取器只认 `crate::` 路径,看不到 `use super::*` 带入的符号、宏内路径、`pub use` 重导出(当前 `extensions/` 里没有 `pub use crate::`,无实际影响);计数不区分生产与测试(02 §1 对此有人工校验)。
