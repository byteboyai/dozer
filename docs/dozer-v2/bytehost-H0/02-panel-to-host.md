# H0-02:面板对 host 的符号依赖与去向(Q10)

> 基线提交:`bb29096`(bytehost-h0 分支,代码与 `main` 的 `4845fb85` 一致)。
> 机械数据:`data/symbols.md`,生成命令 `python3 scripts/audit/report.py symbols`(面板 = `crates/dozer-app/src/extensions/**`,统计 `crate::<模块>::<符号>` 的引用,**含测试**)。
> 回答:要求文档 Q10(面板对 host 的 SDK 契约清单:哪些进 byteui、哪些进 host SDK、哪些留产品、哪些靠重构消除)。
> 范围:只列"被 ≥2 个面板使用"的符号(32 个);单面板符号是该面板的私有依赖,不构成跨面板契约,留给后续各面板迁出时逐个处理。

## 1. 生产引用 vs 仅测试引用

机械报表不区分测试。下表的"生产/测试"由脚本按"文件里第一个 `#[cfg(test)]` 之前/之后"分段、对符号词做 `\bsym\b` 计数得出(**含词边界误报**:`app::Message`、`secrets::remove`、`secrets::save` 的词在代码里太常见,计数偏高,表中标 *)。复现脚本见本文末 §6。

**只在测试里被引用的:** `secrets::fake`(6 次,全在测试)——测试专用,不计入依赖。**测试占比很高的:** `chrome::native_menu`(生产 10 / 测试 24,面板里的 `native_menu` 引用多数在测试)。

## 2. 符号去向表

去向取值:**byteui**(通用 UI 组件,应进 byteui 仓库)、**host SDK**(宿主应向面板提供的稳定契约)、**留产品**(Dozer 专属,Digger 不需要)、**重构消除**(面板不该依赖它)。

| 符号 | 面板数 | 生产/测试 | 去向 | 依据 |
|---|---|---|---|---|
| `app::App` | 7 | 18 / 0 | **重构消除** | 面板拿 `&App` 是为了读 host 状态(`app.list_collapsed(kind)`、`app.hover_progress`、`app.panel_mirrored`),这些应改成显式参数或 view 上下文结构;见 §3 |
| `workspace::Workspace` | 2 | 5 / 1 | **重构消除** | 仅 `ssh`、`todo` 用;面板不该碰整个 `Workspace`(它持有所有面板状态) |
| `app::PanelKind` | 4 | 11 / 1 | **重构消除** | 面板传 `PanelKind::Todo` 等给 host 取折叠态(`todo/view.rs:73-74`、`usage/view.rs`、`database/view.rs:781`);改注册 id 后由 host 在调用面板 view 时注入,面板不再自报身份(见 `01-panelkind.md` B1/B2) |
| `app::HoverId` | 6 | 45 / 1 | **重构消除 + 拆分** | 见 §3.2:`HoverId` 是 host 的单个大枚举,**62 个变体里相当一部分是各面板自己的按钮**(`TodoListCollapse`、`FilesSearchSubmit`、`DatabaseTabItem`…);面板需要"悬停动画键",host 应提供通用键(带命名空间的 id),面板按钮不应写进 host 枚举 |
| `app::TextInputTarget` | 9 | 18 / 3 | **host SDK** | `{ id: widget::Id, secure: bool }`,是"右键菜单要作用于哪个输入框"的通用契约(`Message::TextInputMenuOpen`);无业务类型,9 个面板共用,是 host SDK 的典型候选 |
| `app::Message` | 2 | * | **重构消除** | `files`、`footbar` 直接构造 host 的 `Message`;`Message` 是 host 的总消息枚举,面板应只产出自己的消息并由 host 包装 |
| `app::Divider` / `app::divider_bar` / `app::tab_divider` | 2/2/3 | 1/1/3 | **byteui** | 分隔条/页签分隔线,纯 UI 元件;`Divider` 枚举里带各面板的 split 名,拆开后通用部分进 byteui |
| `chrome::homespace::home_panel_head*` | 10 | 11 / 0 | **byteui** | 面板标题头(图标+标题+可选动作),10 个面板都用;文件里是 `home_panel_head`/`home_panel_head_with_actions`,和 `homespace.rs` 的首页逻辑混在同一个文件,**需先把这两个函数从首页模块里拆出来** |
| `chrome::menu::*`(`item_row_fill`、`shell_frosted`…) | 6 | 35 / 6 | **byteui** | 菜单行/毛玻璃壳,纯 UI;`menu_spec` 同属此类 |
| `chrome::native_menu` | 4 | 10 / 24 | **host SDK** | macOS 原生右键菜单;依赖平台,应是 host 提供的服务(面板给出菜单规格、host 负责呈现);测试占比高说明面板测试里在构造它,迁移时测试要同步改 |
| `chrome::tab_widget` | 3 | 10 / 0 | **byteui** | 页签组件(`tab_label`、`tab_container_style`);项目 CLAUDE.md 要求"优先复用统一组件 `tab_core`",它本来就该在 byteui |
| `theme`(`theme::color` 等) | 6 | — | **byteui** | 主题色;byteui 已有 `byteui::theme::color`,面板里的 `crate::theme` 是产品主题映射,需核实与 byteui 的分工 |
| `theme::region` | 5 | 14 / 3 | **留产品** | `region::agent_list_pane()`、`preview_pane()`、`browser_pane()` 等按**区域名**取样式——区域名是 Dozer 的布局语汇;通用部分(区域样式的结构)进 byteui,取值留产品 |
| `workspace::agent_icon` / `agent_dot_color` | 3 / 2 | 7 / 0、5 / 0 | **留产品** | 定义在 `workspace/hook.rs`,把 `AgentKind` 映射成图标/颜色;Agent 是 Dozer 的领域(Digger 若复用 Agent 面板则随 Agent 领域走,见 O1) |
| `workspace::relative_time_text` | 2 | 5 / 0 | **byteui** | `fn(modified_ms, now_ms) -> String` 纯函数,定义却在 `workspace/view.rs:1916` |
| `workspace::split_portions` / `lh` / `tree_row_font_size` | 2 / 2 / 2 | 3/1、6/6、9/0 | **byteui** | 纯布局/字号助手,定义在 `workspace/view.rs`(`:615,624,631`);`lh` 测试占比高 |
| `workspace::ShellIo` | 2 | 3 / 0 | **host SDK** | `{ client, handle, proxy, capabilities, … }`(`workspace/state.rs:311`):面板异步任务需要的运行时句柄;应作为 host 给面板的稳定上下文(只暴露面板真的用的字段) |
| `project::TreeRow` / `FileTree` / `PathKind` | 2 / 2 / 1 | 7/4、6/3、— | **领域库** | 文件树数据模型(`files`、`ssh` 共用);属于"Files Tree 共享面板"的领域层,见 `04-shared-modules.md` |
| `secrets::{SecretStore, SecretRef, KeyringStore, save, remove}` | 2 | 4,4,4,2,13* / 0 | **host SDK(服务)** | `database`、`ssh` 存连接口令;**不带 Dozer 语义**,任何有"保存凭据"的面板都要;归属见 O4 |
| `secrets::fake` | 2 | 0 / 6 | 测试专用 | 测试替身,随 `secrets` 走 |
| `git_accounts::{GitProvider, self}` | 2 | 32 / 17、8 / 0 | **待定(O8)** | 托管平台账户(GitHub/GitLab…),`project_create`、`settings` 用;bytegit 规格 B3 把它排除出 bytegit,归属仍未定 |
| `project_meta::write_description` | 2 | 4 / 1 | **留产品**(待核) | 把项目描述写进仓库内文件;`project`、`project_create` 用,属 Project 面板领域 |

## 3. 重点:"重构消除"的符号怎么消

这一节不是设计,只写"面板用它是为了什么,怎样可以不需要它"。

### 3.1 `app::App`(18 处生产引用 / 7 个面板)

面板文件里 `App` 的 import 点有 8 个(见 `grep -rn "crate::app::App" extensions`),按名字使用共 22 处(含 Workspace),此后对它调用的方法,统计(`grep -rnE "app\.[a-z_]+\(" crates/dozer-app/src/extensions`)只有这几个:`app.hover_progress`(13 次)、`app.list_collapsed`(3)、`app.list_collapse_button`(3)、`app.hover_tooltip_ready`(1)、`app.active_workspace`(1);另有 `app.take_outbox`、`app.save_to` 各 1 次,经读代码属于**别的叫 `app` 的局部变量**(不是 `&App`),不计。归成三类:
1. **悬停动画:** `hover_progress`、`hover_tooltip_ready`(14 次)——由 host 提供只读的 `HoverQuery` 句柄。
2. **折叠状态与按钮:** `list_collapsed`、`list_collapse_button`(6 次)——应改成 host 调用面板 view 时把 `collapsed: bool` 传入;折叠按钮本身是通用 UI,可由 host 渲染。
3. **取当前工作区:** `active_workspace`(1 次,在 `extensions/` 的某个面板里,Task 5 记入 E3 的"越界"类,逐处确认)。

结论:面板对 `App` 的依赖**形态只有这三种**,每一种都可以由"调用方传参/注入句柄"替代;这是 H1 门禁基线(`scripts/audit/panel-boundary.baseline.json`,22 处 `App`/`Workspace` 使用 / 9 文件)有希望降到 0 的依据。注意**门禁基线数的是 `App`/`Workspace` 的"使用次数"**(import 行 + 每个 `&App` 参数/限定路径,共 22 处,评审后由 import 路径数 11 改为此口径),与上面"方法调用"次数(`app.hover_progress(…)` 等)是两种口径,不可相加。

### 3.2 `app::HoverId`(45 处生产引用,面板是 6 个)

`HoverId` 是 host 的单个枚举(`app/state.rs:93`),共 **62 个变体、全库 188 处使用(extensions 里 40 处)**。变体里混着三类:host 自己的(`Topbar`、`Rail`、`ProjectTabClose`、`HomeTab`…)、预览的(`PreviewTabItem`、`ProjectPreviewFindPrev`…共约 14 个,留产品)、**各面板自己的按钮**(`TodoListCollapse`、`TodoCategoryRow(i64)`、`DatabaseListCollapse`、`DatabaseTabItem`、`SshListCollapse`、`SshTabItem`、`ConversationsListCollapse`、`UsageListCollapse`、`FilesSearchSubmit`、`FilesDotfiles`、`FilesBranchSwitch`、`FileTreeCollapse`、`ProjectDocsAdd`、`ProjectRemoteAdd`、`ProjectMemoryAdd`、`ProjectListCollapse`…)。

这是与 `PanelKind` 并行的**第二处"host 枚举里点名面板"**:Digger 想去掉某个面板,`HoverId` 里的变体就成了死代码;想加新面板,必须改 host 枚举。注册制需要把它改成"带命名空间的 id"(例如 `(panel_id, local_key)`),或让面板自己持有悬停状态——**这条在原 v2 文档里没被列出,是 H0 的新发现**,H1 候选切片应单独评估。

## 4. 对 byteui 的影响(**本任务不改 byteui**)

被判为 byteui 的符号清单,供另开 byteui 仓库的计划使用:`homespace::home_panel_head*`(需先从首页模块拆出)、`chrome::menu::*` 与 `menu_spec`、`chrome::tab_widget`、`app::divider_bar`/`tab_divider`(及 `Divider` 中的通用部分)、`workspace::{relative_time_text, split_portions, lh, tree_row_font_size}`(纯助手,目前放在 `workspace/view.rs`,搬家前要去掉对 `Workspace` 的间接依赖)、`theme::region` 的结构部分。**byteui 现为独立仓库且 dozer 与 digger 共用(项目 CLAUDE.md),发版流程是 byteui 仓库改并发 tag**;这些搬迁属于独立的 byteui 计划,不在 bytehost H0/H1 范围。

## 5. 抽查结果(机械表前 10 行)

对 `symbols` 表前 10 行(`chrome::homespace`、`TextInputTarget`、`App`、`HoverId`、`chrome::menu`、`theme`、`theme::region`、`chrome::native_menu`、`PanelKind`、`tab_widget`),各取一个使用面板用 `grep -n` 打开源码确认:

| 符号 | 抽查文件 | 结果 |
|---|---|---|
| `chrome::homespace` | `extensions/todo/view.rs:34` | 确认:`crate::chrome::homespace::home_panel_head(` |
| `app::TextInputTarget` | `extensions/ssh.rs:1023` | 确认:`crate::app::TextInputTarget {` |
| `app::App` | `extensions/todo/view.rs` | 确认(`&App` 参数) |
| `app::HoverId` | `extensions/git_log.rs`、`database/view.rs` | 确认 |
| `chrome::menu` | `extensions/conversations.rs:403` | 确认:`crate::chrome::menu::item_row_fill(` |
| `theme` | `extensions/browser.rs` | 确认 |
| `theme::region` | `extensions/group_chat/view.rs` | 确认 |
| `chrome::native_menu` | `extensions/files/mod.rs`、`files/view.rs`、`conversations.rs`、`database/view.rs`、`ssh/sftp.rs` | 确认,**且多数命中在测试里**(与 §1 一致) |
| `app::PanelKind` | `extensions/todo/view.rs:73` | 确认 |
| `chrome::tab_widget` | `extensions/browser.rs` | 确认 |

10/10 确认。**机械表的已知盲区:** 提取器看不到 `use super::*` 带进来的符号、看不到宏内的路径;如果某个面板通过 `mod.rs` 的 `pub use` 重导出间接依赖 host,会记在 `mod.rs` 名下,不会被算成"面板→host"边。目前 `extensions/*/mod.rs` 里没有 `pub use crate::` 形式的重导出(`grep -rn "pub use crate::" crates/dozer-app/src/extensions` 无输出),所以此盲区在当前代码上没有实际影响。

## 6. 复现

```bash
python3 scripts/audit/report.py symbols            # 机械表
grep -rn "pub use crate::" crates/dozer-app/src/extensions   # 重导出盲区检查(应无输出)
```

生产/测试分段统计用的是一次性脚本(按文件第一个 `#[cfg(test)]` 分段、`\bsym\b` 计词),口径粗糙,仅用于判断"是否主要在测试里";需要精确数字的符号请逐处读代码。
