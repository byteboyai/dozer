# H0-04:根下共享模块画像与归属建议(Q7、O4、O7、O8)

> 基线提交:`108f46b`(bytehost-h0 分支,代码与 `main` 的 `4845fb85` 一致)。
> 机械数据:`data/modules.md`,生成命令 `python3 scripts/audit/report.py modules`(根模块 = `crates/dozer-app/src/*.rs`;扇入含测试与面板、host 层)。
> 回答:要求文档 Q7(`conversation`/`delivery`/`project` 三个共享模块"形似共享数据库",未审计内部)、规格 O4(`secrets`/`external_apps`/`capabilities` 归属)、O7(`conversation`/`transcript` 最终归属)、O8(`git_accounts`)。**本文给出归属建议,不关闭规格里这些未决项。** `delivery` 已随 bytegit P6 删除,不在本表。

## 1. 先纠正三个名字带来的误读

读代码后发现,规格 §3.5 与要求文档 §4.4 沿用的几个模块名**和模块实际内容不符**,这会直接影响归属判断:

| 名字 | 以为是 | 实际是 | 依据 |
|---|---|---|---|
| `project.rs`(856 行) | "Project context:项目打开/关闭/当前项目" | **文件树状态机**:`FileTree`、`TreeRow`、`PathKind`、`copy_dir_recursive`、`move_item`——Files Tree 的数据层,`files`、`ssh` 面板共用 | 文件头 `//! 文件树状态机（P1g）：懒加载单目录、展开集、可见行摊平。纯数据,不碰 iced` |
| `conversation.rs`(132 行) | "conversation 领域模型" | **展示用的中间表示**:`ConversationMeta`、`SessionRow`;领域类型 `ConversationSummary`/`TurnRecord`/`AgentKind` **已在 `dozer-core::protocol`**(与 dozerd 共享) | 文件头 `//! 历史对话展示用的中间表示(…数据来源改为查询 dozerd,本文件不再直接碰磁盘)`;`from_summary(&ConversationSummary)` |
| `transcript.rs`(828 行) | "transcript 领域模型" | **Claude Code transcript 的 app 侧适配器**:`review_entries_from_turns`(消费 dozerd 的 `TurnRecord`)+ `latest_model_mode_and_activity`(仍读 JSONL 原文,Agent 卡片指示器用) | 文件头 |

所以真正的"项目上下文"在哪?**权威数据在 dozerd**(`dozer-core::protocol::ProjectInfo`,`open_projects.rs` 注明"项目自己的路径/名字元数据不重复存,权威数据在 dozerd"),app 侧是 `Workspace.project`(来自 `ProjectInfo`)+ `open_projects.rs`(本地持久化当前开了哪些项目与顺序)+ `project_meta.rs`(`.dozer/description.md` 的读写)。这三者没有一个叫"project context"的模块。→ O2(Project context 范围)的证据见 `06-open-items-evidence.md`。

## 2. 根模块去向表

去向取值:**host**、**面板**(随某个面板走,写明哪个)、**领域库**(多面板共享的纯数据/领域模型)、**产品层**(Dozer 专属)、**byteui**、**拆分**。"Digger 复用面板"指规格 §3.4 的 Todo/Conversation/Agent/Project/Files Tree 五个。

| 模块 | 行 | 扇入单元 | Digger 复用面板用它? | iced | 去向 | 依据 |
|---|---|---|---|---|---|---|
| `theme` | 98 | 20 | 全部 | 否 | **产品层 + byteui** | 文件头:颜色/字号/图标尺寸/基础几何 token 由 `byteui::theme` 持有,本模块只做 `init()` 把产品主题灌进去、再有 `region`/`terminal_font`/`geometry` 三个子模块读 `assets/theme/workspace.json`;`region::agent_list_pane()` 等按**区域名**取样式是 Dozer 布局语汇 |
| `assets` | 1392 | 5 | — | 否 | **拆分** | `dozer://` 协议应答:机制(协议分发、白名单、MIME)归 host,内容(vendored Flyfish dist、`dozer://html/`、`dozer://flyfish/…`)归产品;`handle_protocol` 目前把两者写在一起 |
| `project`(`FileTree` 等) | 856 | 5(files, ssh, app, platform, workspace) | Files Tree ✓ | 否 | **领域库 / 随 Files Tree 面板** | 纯数据、不碰 iced、`files` 与 `ssh` 共用;应随 Files Tree 面板一起成为共享面板的数据层(`ssh` 的 SFTP 树复用它,说明它不止服务本地文件) |
| `capabilities` | 301 | 4(app, preview, workspace, runtime) | 无面板直接用 | 否 | **host** | 文件头:启动时探测一次硬件、换算应用级预算,"业务模块一律通过 `current()`/`Arc` 注入读取";预览预算与 Surface 机制相关,是产品无关的宿主能力 |
| `conversation`(展示 IR) | 132 | 4(conversations, usage, app, chrome) | Conversation ✓ | 否 | **面板(Conversation)** | 是 `ConversationSummary` 的 UI 投影;`usage` 面板也用它(`ConversationMeta` 6 次),所以不能只算 Conversation 面板私有——**usage 对它的依赖是 usage 复用 Conversation 数据的体现**,抽出面板时要为 usage 保留只读视图类型 |
| `project_meta` | 73 | 4(project, project_create, app, workspace) | Project ✓ | 否 | **面板(Project)** | 仅两个函数 `load_description`/`write_description`,读写 `.dozer/description.md`;路径里的 `.dozer/` 是产品目录名 |
| `runtime` | 772 | 4(settings, app, platform, main) | — | 是 | **拆分** | `spawn_dozerd`(启动 dozerd 子进程,**产品**)与 `run_operate`/`sync_webview_pool`(operation 遍历、webview 池同步,**host 机制**)混在一个文件 |
| `menu_spec` | 200 | 3(files, chrome, workspace) | Files ✓ | 是 | **byteui** | 平台无关的菜单 IR(`MenuSpec`、`to_native`、`to_iced`),无业务类型;与 `chrome::menu`、`native_menu` 同属一族(见 `02-panel-to-host.md`) |
| `external_apps` | 216 | 2(files, app) | Files ✓ | 否 | **面板(Files)** | 文件头:"用外部软件打开"配置表,供**文件树右键菜单**;`files` 面板引用 74 次 |
| `git_accounts` | 388 | 2(project_create, settings) | Project(project_create)部分 | 否 | **产品层(Project 面板的服务)** | 文件头:Git 托管账户的本地元数据,Token 存系统钥匙串命名空间 `"dozer-git"`(**产品名写死**);`RemoteRepo`/`extract_username` 面向 GitHub/GitLab 之类托管平台——bytegit 规格已将它排除(非目标,B3),也不应进 host |
| `layout` | 147 | 2(app, panel_layouts) | — | 否 | **host** | 外壳布局持久化(左面板宽度、分割比例、选择与收起态),"应用级偏好" |
| `panel_layouts` | 203 | 1(app) | — | 否 | **host** | 每个项目各自的面板布局持久化;内容含 `PanelDims`(按面板展开的字段,见 `01-panelkind.md` §2b),注册制后要改成按 id 键 |
| `open_projects` | 72 | 1(app) | — | 否 | **host** | 并行项目页签集合的持久化(当前开了哪些项目、顺序、前台是谁) |
| `secrets` | 196 | 2(database, ssh) | 无 | 否 | **host(服务)** | `SecretStore` 抽象 + `KeyringStore` + `FakeStore`,无 Dozer 语义,任何要存凭据的面板都需要;目前只有两个**非**复用清单里的面板使用 |
| `transcript` | 828 | 2(app, workspace) | Agent ✓(卡片/回合明细) | 否 | **面板(Agent)领域层** | Claude Code 专属解析(JSONL);**可能与 `dozerd/src/transcripts/{parse,scan}.rs` 重复**——dozerd 侧是摄取,app 侧是 Agent 卡片的"最新 model/mode/activity",二者是否同一份解析需另核 |
| `preview_state` | 295 | 1(workspace) | — | 否 | **产品层(Preview)** | 预览 tab 的持久化,Preview 业务留产品(规格 §3.2) |
| `webview_geometry` | 2055 | 2(app, platform) | — | 否 | **拆分** | 通用 Surface 几何(预览矩形、命中判断)归 host;同文件里有 `git_log_diff_pane_bounds_for`、`usage_content_pane_bounds_for`、`codehealth_content_pane_bounds_for`、`todo_content_pane_bounds_for`、`group_chat_content_pane_bounds_for` 五个**每面板一个**的几何函数——应变成面板钩子(`01-panelkind.md` S2/S3) |
| `event` | 73 | 1(platform) | — | 是 | **host** | 悬停动画帧间隔、清帧、合成命令键事件 |
| `frosted` | 35 | 1(chrome) | — | 是 | **byteui** | 磨砂噪点贴图原语,纯 UI |
| `keymap` | 294 | 1(platform) | — | 否 | **面板(Terminal),待 O6** | 键盘/IME → 终端字节序列的纯函数 |
| `osc` | 170 | 1(workspace) | — | 否 | **面板(Terminal),待 O6** | OSC 7/133 扫描器,终端专属 |

## 3. 规格 §3.5 点名模块的专节

- **Project context(O2):** 见 §1。它不是一个模块,是三件事:权威数据在 dozerd(`ProjectInfo`)、app 侧的 `Workspace.project`、`open_projects` 持久化。host 最小范围倾向于只含"打开的项目集合 + 当前项目 id + 路径"(`open_projects.rs` 已是这个形状),不含 `project_meta`、`project`(文件树)。详见 `06-open-items-evidence.md` §O2。
- **`secrets`:** 无 Dozer 语义,**建议 host 提供**;没有复用面板依赖它,所以**不紧迫**,触发条件是出现第三个需存凭据的面板(`E3-021`)。
- **`external_apps`:** 只服务 Files Tree 右键菜单,**随 Files Tree 面板走**,不是 host 服务;规格 O4 把它与 `secrets`/`capabilities` 并列为"同一类待决"不准确。
- **`capabilities`:** 与 Surface/预览预算耦合,**建议 host**;`runtime`/`preview`/`workspace`/`app` 使用,面板不直接用。
- **`git_accounts`(O8):** 证据指向"产品层 + Project 面板的服务",不进 bytegit(账户/远程仓库列表在 bytegit 规格里已是非目标)、也不进 host:钥匙串命名空间 `"dozer-git"` 写死了产品名。**留产品**,Digger 若需要托管平台账户时再抽(`E3-024`)。
- **`conversation`/`transcript`(O7):** 领域类型已在 `dozer-core::protocol`(与 dozerd 共享),app 根下两个模块只是 **UI 投影与 Claude 专属适配器**。结论:**领域模型的归属问题已经被 `dozer-core::protocol` 回答了一半**(下沉到共享协议层);待办是 `conversation.rs`(随 Conversation 面板)、`transcript.rs`(随 Agent 领域,先核实与 `dozerd/transcripts` 的重复)。

## 4. `dozer-core` 公开模块清单

`crates/dozer-core/src/lib.rs` 只有四个公开模块:

| 模块 | 行 | 产品无关? | 说明 |
|---|---|---|---|
| `protocol` | 3085 | **否(Dozer 专属)** | dozerd↔GUI 的线协议,含 `TodoInfo`、`MemoryInfo`、`GroupInfo`、`CodeHealthReportInfo`、`PreviewContext`、`ProjectInfo`、`TurnRecord`、`AgentKind` 等**全部 Dozer 领域类型**。Digger 复用面板就要依赖它——要么 Digger 沿用同一份 protocol,要么 protocol 按领域拆分;这是比 host 更大的问题,**H0 只记录,不裁决** |
| `log` | 649 | **机制是,字符串不是** | 两层设计(std 层 + `tracing` feature 层)产品无关,**但 `Component::App => "dozer-app.log"`、`Component::Daemon => "dozerd.log"`、`Component::Hook => "dozer-hook.log"` 写死了产品名**。**规格 §3.1 的"日志机制归属"裁决:机制归 host 一侧(`dozer_core::log` 的 scope/门禁约定本身产品无关),但抽出前必须把产品名参数化;H1 不阻塞,先在 E3 外登记为兼容债务** |
| `paths` | 58 | **否** | `ProjectDirs::from("ai", "byteboy", "dozer")` 写死;`dozerd.sock` 路径 |
| `agent_paths` | 254 | **是(Agent 领域)** | 各 agent 的会话存储目录路径计算,dozerd 与 dozer-app 共用;随 Agent 领域(O1) |

## 5. 对 O4、O7、O8 的证据摘要(不下裁决)

- **O4:** `secrets` 无 Dozer 语义、单一抽象(host 服务候选);`external_apps` 只服务 Files(不是 host 服务,应面板私有);`capabilities` 与 Surface 耦合(host)。规格里把三者并列为同一类不成立,建议拆开。
- **O7:** 领域类型已在 `dozer-core::protocol`;app 根下的 `conversation`/`transcript` 是投影与适配器。真正的未决是 `protocol` 本身的产品归属。
- **O8:** 证据指向留产品(钥匙串命名空间写死、托管平台 API 面向 Project 创建流程)。
