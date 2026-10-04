# H0-01:`PanelKind` 引用分类与注册制分批(Q8、O5)

> 基线提交:`61a4af8`(bytehost-h0 分支,代码与 `main` 的 `4845fb85` 一致)。
> 机械数据:`data/panelkind.md`,生成命令 `python3 scripts/audit/report.py panelkind`。
> 回答:要求文档 Q8(`PanelKind` 引用怎么分批)、规格 O5(默认栏位由谁决定,**只给倾向**)。

## 1. 总数(实测)

| 项 | 实测 | 旧文档值 |
|---|---|---|
| 变体数 | **12**(`Files, GitLog, Todo, Project, Database, Ssh, Web, Agent, GroupChat, Conversations, Usage, CodeHealth`) | 11 |
| `PanelKind` 出现的行数 | **973**(点名变体 763 + 仅类型 210) | 约 887 |
| 涉及文件数 | **31** | 约 25 |
| 遍历候选簇(15 行内 ≥3 个不同变体) | 42 | — |

复现:`grep -rn "PanelKind" crates/dozer-app/src | wc -l`(973)、`grep -rln "PanelKind" crates/dozer-app/src | wc -l`(31)。

## 2. 最重要的发现:引用量被两件事撑大,都不是"每个面板一个特判"

**(a) `Project`/`Files` 预览窗格二选一,约 82 行。** 全库 `== PanelKind::Project` / `!= PanelKind::Project` / `PanelKind::Project => …` 共 **114 行**,其中**约 82 行是窗格选择**(53 处 `==`/`!=` 判断 + 29 个取 `project_preview` 的 match 臂;`grep -rnE "(==|!=) PanelKind::Project"` 得 53,`grep -rnE -A1 "PanelKind::Project =>"` 里下一行含 `project_preview` 的有 29),**其余约 32 行是 12 臂的元数据/名称映射 match**(图标、split 比例、`"project"` 字符串、`webview_geometry` 的穷举 match 等),属 B2/B3,不是窗格选择。分布(114 行口径):(`workspace/state.rs` 35、`app/update.rs` 33、`app/app.rs` 14、`workspace/view.rs` 11、其余 21),几乎全是同一个模式:

```rust
let pane = if kind == PanelKind::Project { &mut ws.project_preview } else { &mut ws.preview };
```

(例:`workspace/state.rs:2349`、`app/app.rs:1031`、`app/update.rs:2298`)。**这不是 N 个面板的特判,而是"Preview 业务有两个实例,由 PanelKind 选择"**。Preview 业务按规格 §3.2 留在 Dozer 产品层,所以这约 82 行迁移后不进 host;它们应由一个 `preview_pane(kind)` 访问器收口——收口本身是**纯机械重构**,可以先于任何注册制工作完成,且一次能消掉全库约 8% 的引用行(评审前误写为 114 行/12%,已按 reviewer 抽查订正)。

**(b) 每面板一份的维度表,host 持有按面板展开的字段。** 例:`PanelDims` 里每个面板各有 `*_list_collapsed`、`*_split` 字段,`toggle_panel_list_collapse`(`app/update.rs:5368`)、`panel_mirrored`(`:5336`)、`with_pair_split_ratio`(`app/layout.rs:560`)、`pair_split_ratio`(`:538`)、`panel_meta`(`chrome/rail.rs:576`)都是对 12 个变体的穷举 `match`。这些是**元数据/按面板状态**:迁移后应是"以注册 id 为键的一张表",不是 12 路 match。

## 3. 逐文件分类表

分类取值:**遍历**(对所有面板一视同仁地循环/穷举)、**特判**(只对个别面板有特殊行为)、**类型传递**(把 `PanelKind` 当 id 传来传去)、**元数据**(默认栏位、图标、标题、按面板展开的状态字段)。"含测试"表示该文件的相当部分引用在 `#[cfg(test)]` 里。

| 文件 | 点名 | 仅类型 | 簇 | 人工分类 | 依据 |
|---|---|---|---|---|---|
| `app/app.rs` | 100 | 29 | 1 | 特判 + 类型传递 | Project/Files 窗格选择 7 处(`:1031,1085,1325,1351,1381,1524-1552`);`:3419-3544` 对 `GitLog/Usage/CodeHealth/Todo/GroupChat` 逐面板各写一段 webview 期望几何(**特判,迁移后应是面板钩子 `desired_webviews`**);`:3575-3616` 按 `Files/Project/Conversations` 取各类 webview(Preview 业务);`:2381-3213` 十余个 `fn xxx(&self, kind: PanelKind)` 是类型传递 |
| `webview_geometry.rs` | 114 | 3 | 11 | 遍历(穷举 match)+ 特判 | `:75-133`、`:162-269` 两处对 12 个变体的穷举 `match`,多数臂是 `(0,0,0,0)`——**真正有几何的只有 `Files/Web/Project/Conversations`** 四个;`:311`、`:410` 特判 `Files`/`GitLog`;`:898` 起是测试(约 55% 的引用行在测试里,含测试) |
| `chrome/rail.rs` | 92 | 22 | 12 | 元数据 + 遍历 | `RailLayout::default()`(`:165-186`)按顺序列 12 个变体 = 默认栏位与顺序;`panel_meta`(`:576`)穷举图标与 tooltip;`:211-216` 的 `migrate_legacy_rail` 写死"其余 11 个、插到 Agent 之后";`:734+` 测试 |
| `app/update.rs` | 78 | 9 | 6 | 特判 + 元数据 | Project/Files 窗格选择 33 处(同 §2a);`fire_panel_switch_in`(`:5152-5215`)对 9 个面板各写一段"切入时的刷新"(**特判,应成面板的 `on_activate` 钩子**,`Files/Web/Agent` 为空);`:5336-5377` 两个按面板展开的 `match`(元数据) |
| `workspace/state.rs` | 42 | 39 | 0 | 特判 + 类型传递 | 35 处 Project/Files 窗格选择(`:2349-2872` 一长串 `if kind == PanelKind::Project`);`:2247-2339` 类型传递 |
| `app/layout.rs` | 60 | 7 | 6 | 元数据 + 特判 | `:538-606` 两个 12 臂 `match`(split 比例取/设);`:681-990` 十次 `side != PanelKind::X.default_side()` 重复——**每个面板一段结构相同的布局代码**(遍历,应是对注册表的一个 for);`:1146-1198` 特判 `Agent` 右栏 |
| `platform/window_events.rs` | 45 | 4 | 0 | 特判 | 窗口事件里按面板命中判断(Task 5 逐条登记) |
| `workspace/view.rs` | 42 | 4 | 0 | 特判(Preview) | 11 处 Project/Files 窗格选择,其余是预览消息的 `PanelKind::Files` 实参 |
| `app/message.rs` | 0 | 45 | 0 | 类型传递 | 45 个 `Message` 变体携带 `PanelKind`,机械替换 |
| `preview/view.rs` | 30 | 8 | 0 | 特判(Preview) + 含测试 | 窗格选择;`:5835` 起在 `mod tests`(`:2698` 之后) |
| `app/view.rs` | 35 | 1 | 0 | 特判 | `app.panel_mirrored(PanelKind::Usage)` 一类逐面板分支(`:992`) |
| `app/state.rs` | 25 | 10 | 3 | 元数据 | `PanelKind` 定义、`default_side()`(`:22-60`) |
| `preview/code_host.rs` | 22 | 7 | 2 | 特判(Preview) | `:150` `(PanelKind::Project, id - PROJECT_PREVIEW_ID_OFFSET)`:用 id 偏移区分 Project/Files 预览——**host 层的 webview id 编码里藏着面板区分** |
| `preview/webview_protocol.rs` | 22 | 5 | 0 | 特判(Preview) | 同上 |
| `workspace/tests.rs` | 17 | 1 | 0 | 测试 | 全部在测试文件里 |
| `preview/resources.rs` | 4 | 5 | 0 | 类型传递 | `ViewerKey = (i64, PanelKind, usize)`(`:22`)把面板放进资源键;其余在测试 |
| `panel_layouts.rs` | 6 | 1 | 1 | 元数据 | 布局落盘 |
| `term/terminal.rs` | 4 | 2 | 0 | 特判 | `:46` 特判 `Ssh`;`:188` 特判 `Agent` 的列表折叠 |
| `extensions/usage/mod.rs` | 5 | 0 | 0 | 注释 | 5 处都是文档注释里写"见 app.rs `PanelKind::Usage` 分支"——**面板文档在指回 host 的硬编码分支** |
| `preview/webview.rs` | 3 | 1 | 0 | 特判(Preview) | `:150` 从 URL 的 `"project"/"files"` 解出 `PanelKind` |
| `extensions/files/state.rs` | 0 | 3 | 0 | 类型传递 | 面板自己持有 `kind: PanelKind` 字段(`:182`) |
| `extensions/files/view.rs` | 1 | 2 | 0 | 类型传递 | 同上 |
| `extensions/project/view.rs` | 3 | 0 | 0 | 注释 | 同 usage |
| `extensions/todo/view.rs` | 3 | 0 | 0 | 类型传递 + 注释 | `:73-74` 向 host 传 `PanelKind::Todo` 取折叠态——**面板为取自己的折叠状态而 import host 的 `PanelKind` + `app.list_collapsed`** |
| `extensions/usage/view.rs` | 3 | 0 | 0 | 类型传递 | 同 todo |
| `extensions/database/view.rs` | 2 | 0 | 0 | 类型传递 | `:781-782` 同 todo |
| `platform/edit_history_overlay.rs` | 1 | 1 | 0 | 特判 | overlay 里点名面板 |
| `platform/file_history_overlay.rs` | 1 | 1 | 0 | 特判 | 同上 |
| `extensions/database/state.rs` | 1 | 0 | 0 | 注释 | |
| `extensions/todo/state.rs` | 1 | 0 | 0 | 注释 | |
| `runtime.rs` | 1 | 0 | 0 | 特判 | `:680` 特判 `GitLog` |

## 4. 特判清单(Task 5 的 E3 登记来源)

按**迁移后落点**归类,每条给出位置与被特判的面板:

| # | 位置 | 被特判的面板 | 行为 | 迁移后落点 |
|---|---|---|---|---|
| S1 | `app/update.rs:5152-5215` `fire_panel_switch_in` | Todo、Database、Project、Ssh、Usage、CodeHealth、Conversations、GitLog、GroupChat(Files/Web/Agent 为空) | 切入面板时的刷新动作 | 面板钩子 `on_activate`,host 只调用 |
| S2 | `app/app.rs:3419-3544` | GitLog、Usage、CodeHealth、Todo、GroupChat | 每面板一段 webview 期望几何 | 面板钩子 `desired_webviews` |
| S3 | `webview_geometry.rs:75-269` 两个穷举 `match` | Files、Web、Project、Conversations 有几何,其余 `(0,0,0,0)` | 预览列几何 | 面板声明自己是否有预览列;几何计算留 host(Surface 机制) |
| S4 | 约 82 处 Project/Files 窗格选择(§2a;114 行口径里另约 32 行是元数据 match,归 B2/B3) | Project、Files | 选 `project_preview` 还是 `preview` | **留产品层**(Preview 业务),先用 `preview_pane(kind)` 收口 |
| S5 | `preview/code_host.rs:150`、`preview/webview.rs:150`、`PROJECT_PREVIEW_ID_OFFSET` | Project、Files | webview id/URL 里编码面板 | 留产品层;host 的 webview 注册不应认识这两个名字 |
| S6 | `app/layout.rs:1146-1198`、`app/app.rs:3248-3375` | Agent | 右栏"Agent 未镜像"时的渲染顺序与尺寸 | 待 O1(Agent 怎么切)裁决后再定 |
| S7 | `term/terminal.rs:46`、`:188` | Ssh、Agent | 终端区对这两个面板的特殊布局 | 待 O6(Terminal 是否共享) |
| S8 | `app/app.rs:1634-1635` | GroupChat | 左右栏 GroupChat 占位 | 面板钩子 |
| S9 | `runtime.rs:680` | GitLog | 运行时对 GitLog 的特判 | 需读上下文确认(Task 5) |

## 5. 默认栏位(O5):现状与倾向(**非裁决**)

现状:`PanelKind::default_side()`(`app/state.rs:40-58`)写死;`RailLayout::default()`(`chrome/rail.rs:165-186`)另写一份**顺序**(顺序比 `default_side` 多一层信息);二者靠测试(`rail.rs:716-725`)保持一致。`default_side()` 在 `app/layout.rs` 里被调用 11 次、`webview_geometry.rs` 7 次、`app/app.rs` 1 次,几乎都是"是否偏离默认栏(镜像)"判断。**落盘的布局用 serde 的枚举名**(`PanelKind` 派生 `Serialize/Deserialize`),且消毒逻辑写死"恰 12 个不重复"(`rail.rs:222` 起,迁移旧布局时写死"其余 11 个")。

倾向:默认栏位**连同默认顺序**一起由产品组合根(composition root)给出,注册信息里只放面板自身的属性(id、标题、图标、是否需要预览列)。理由:同一个面板在 Dozer 与 Digger 里默认位置本来就不同(Digger 没有 Database/Ssh 等面板,"恰 12 个"校验必须由产品给出面板清单),放进注册信息会让面板带着产品布局偏好。反方理由:放进注册信息可以让"一个面板 + host 就能运行"在没有产品组合根时仍有合理默认。**这一点留给规格 O5 裁决**。

## 6. 注册制迁移批次建议

按风险递增、每批可独立合并:

| 批 | 内容 | 涉及(行数,来自 §2–§3) | 前置 |
|---|---|---|---|
| B0 | **`preview_pane(kind)` 收口**:消掉约 82 处 Project/Files 窗格选择(留产品层的纯机械重构) | 约 82 行(114 行口径的子集)/ 13 个文件 | 无 | **已完成(H1):75 处机械重写 + 4 个访问器 + 4 处手工编辑;剩余 7 处是合理写法(见基线)** |
| B1 | **类型传递**:`PanelKind` 换成注册 id 类型(`app/message.rs` 45 个变体、`preview/resources.rs`、面板里的 `kind` 字段),保持 serde 兼容 | 约 210 行(仅类型) | O5 |
| B2 | **元数据表**:`panel_meta`、`default_side`、按面板的 `*_list_collapsed`/`*_split` 字段变成按 id 取的表,消掉 `app/layout.rs` 的 12 臂 `match` 与 10 段重复布局 | `app/layout.rs` 60、`chrome/rail.rs` 92、`app/update.rs` 约 20 | B1;O5 |
| B3 | **遍历**:`webview_geometry.rs`、`app/layout.rs:681-990` 的"每面板一段"改成对注册表的 for | 约 150 行 | B2 |
| B4 | **特判变钩子**:`fire_panel_switch_in`(S1)、`desired_webviews`(S2) | 约 90 行 | B3;面板接口设计 |
| B5 | 剩余特判(S6–S9) | — | O1、O6 |

**建议第一批:B0。** 它不需要任何未决项,不改变行为,只动产品层的 Preview 业务,并一次消掉全库约 8% 的 `PanelKind` 引用行。

## 7. 抽样校验

随机抽样 30 行(`grep -rn PanelKind crates/dozer-app/src`,固定种子 7)逐行人工判读:

- **"点名变体"/"仅类型"两分法与人工判读 30/30 一致**(6 行仅类型:`rail.rs:359`、`files/state.rs:182`、`preview/view.rs:1630`、`app.rs:2296`、`:3176`、`:3147`;`rail.rs:716` 的文档注释因写 `PanelKind::default_side` 小写不匹配变体正则,同归仅类型;其余 23 行点名)。
- **但"点名"行里有噪声:** 抽样中 4 行在测试代码里(`rail.rs:780`、`workspace/tests.rs:1157`、`webview_geometry.rs:1385`、`preview/view.rs:5835`),约占点名行的 17%;另有若干是文档注释。**所以 763 这个"点名变体"数字高估了真实的生产特判量,约 15–20%**,迁移估算应打折。
- 结论:启发式作"分类线索"可靠(100%),作"生产特判数量"需打折;本文的分类以上面的人工逐文件分析为准。
