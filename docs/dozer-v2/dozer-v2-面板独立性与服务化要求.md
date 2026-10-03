# Dozer V2：面板独立性与服务化要求

> 文档性质：架构要求与审计记录，**不是已批准规格**。
> 起稿日期：2026-10-02，同日按用户裁决修订一次（§2）。承接 `dozer-v2架构分析.md`。
> 本文是**活文档**：每发现一个具体问题就追加到 §6，每做出一个裁决就从 §6 挪到 §2。

## 0. 为什么单独成文

两件事推动了这份文档：

1. **Digger 复用。** Digger（自媒体、写作、内容管理）计划复用 Dozer 的 todo、file tree、agent、conversation、usage 面板，只新增几个内容类面板。复用的前提是面板能脱离 Dozer 自己的内核。Finmeter 若有同类需求同理。
2. **用户反复强调的 v2 要求：** 在一个 host 之上，每个面板都能独立运行；面板之间互相访问走 MCP 服务，设计上类似微服务。

## 1. 要求

- **R1 Host + 独立面板。** Host 负责窗口、布局、焦点、浮层、主题与生命周期，并提供统一的平台服务（如 Git）；**一个面板 + Host 就能运行**，不依赖其他面板。
- **R2 面板之间不直接引用。** 不 import 对方的类型、状态或函数；需要的共享能力由 Host 平台服务提供，状态变化经事件总线通知。
- **R3 面板能力可暴露给 agent。** 面板愿意给 agent 用的能力，经 MCP 暴露（agent 通道）。这是可选项，不是面板间通信机制。

## 2. 已定裁决（2026-10-02，用户）

1. **面板之间不走 MCP。** 用户判断"MCP 可能太理想主义"。面板间的共享能力用 **Host 统一提供的服务**（typed service），状态变化用 **事件总线**。MCP 只留给 agent 通道，与原 `dozer-v2架构分析.md` §6 的分层一致。
2. **引入事件总线。**
3. **"独立运行"的定义：一个面板 + Host 可以运行。** 不要求面板脱离 Host 单独成 App。
4. **`file_history → git_log` 也要解耦，改用 Host 统一提供的 Git 服务。** 见 §4.1、§6 Q12。
5. **Git 是 byteboy 体系的底层必备组件，与 agent 同等重要。** 它是底层，不是面板；其他面板（git_log、file_history、files、codehealth、usage、home 等）只是对 Git 底层 API 的调用与功能呈现。因此 Git 服务是**完整的底层 API**，不是"给 file_history 抽一点 diff 能力"；`git_log` 面板自身也降级为"一个视图"。Agent 与 Git 构成 Host 平台层的两根支柱（见 §5）。
6. **统一做一个独立的 `bytegit` 库。** Dozer、Digger 及后续产品共用；散落在 `dozer-app`、`dozerd` 里的约 180 个 git 调用点（§4.6）逐步迁到它之上，消除两份实现。库形态（本地同步调用），具体 API、`git2`/命令行取舍、仓库与版本方式见 Q14。

## 3. 原冲突的结论

原草案 §3 对比了方案 A（RPC + 事件总线，MCP 只给 agent）、B（调用走 MCP）、C（纯 MCP）。裁决后落在 **A**：Host 服务 + 事件总线承担面板间协作，MCP 只面向 agent。`dozer-v2架构分析.md` §6.3 的追加说明已指向本文，该说明应改为"已按本文 §2 采纳原 §6，无需调整"。

这也让 §6 的几个问题变简单：不需要进程内 MCP 传输、不需要面板间调用的权限模型。

## 4. 现状审计（2026-10-02，基于 import 与引用计数，未跑运行时）

审计范围：`crates/dozer-app/src/extensions/*`（约 3.9 万行，含测试）、host 侧对 extension 的引用、`dozer-mcp`。

### 4.1 面板之间的直接耦合：很低，只剩一处真实的

扫描所有 `use crate::extensions::*` 与 `use super::*`，extension 之间的直接引用只有：

- `file_history → git_log`（唯一一处真实的面板间耦合）
- `files → toast`（`toast` 是 host 基础设施，不算面板间耦合）
- `todo`、`project` 内部自引用

**`file_history → git_log` 的具体内容**（`file_history.rs` 1468 行，`git_log.rs` 3091 行）：
- 直接使用 `git_log::DiffBlobContent`（`file_history.rs:73、190、502`）与 `git_log::classify_diff_bytes`（`:503`）：diff 内容的数据类型与"二进制/文本/过大"分类逻辑。
- 另有多处注释写明"逻辑同 `git_log.rs::…`"（`MAX_PATCH_CHARS` 理由、根提交按空树对比的写法、`DEFAULT_MAX_COMMITS` 量级），即**两边各自实现了一份相似逻辑，靠注释保持一致**。

这说明要抽出的不只是类型，而是一块"diff 内容与 blob 读取"的共享能力，应进 Host Git 服务（§6 Q12）。

### 4.2 面板对 Host 的耦合：这才是主要问题

面板依赖 host 内部符号（按 import 统计）：

| 依赖 | 使用方 |
|------|--------|
| `crate::app::{App, HoverId, ProjectId, TextInputTarget}` | todo、conversations、git_log、ssh |
| `crate::workspace::*`（`Workspace`、`agent_icon`、`lh`、`split_portions`、`agent_dot_color`…） | todo、conversations、browser、footbar |
| `crate::chrome::*`（`tab_widget`、`homespace`、`native_menu`） | browser、git_log、conversations、usage |
| `crate::preview::WebviewSpec` | browser |
| `crate::theme`、`crate::menu_spec` | 多数面板 |

这些符号就是 v2 的"host 契约"要收敛成的东西：`App`、`Workspace` 必须出局，`HoverId`/`theme`/图标/菜单规格这类通用能力变成稳定的 SDK 接口。

### 4.3 Host 对面板的硬编码：数量大

- `PanelKind` 是封闭枚举（11 个变体：Files、GitLog、Todo、Project、Database、Ssh、Web、Agent、Conversations、Usage、CodeHealth），默认左右栏归属也写死在 `default_side()` 里。`PanelKind` 在 src 内约 887 处引用，分布在 `workspace`、`preview`、`platform`、`term`、`panel_layouts`、`webview_geometry` 等约 25 个文件。
- host 侧（`app/`、`workspace/`、`chrome/`、`platform/`、`preview/`）对 18 个 extension 模块有直接引用：files 35、project 27、todo 26、browser 20、database 19、agent_context 18、project_create/git_log 各 15、usage/ssh/search/conversations 各 14、toast 13、codehealth 11、edit_history 10、footbar 7、file_history 7、settings 6。
- `app/` 目录合计约 1.7 万行，是 host 的主要体量。

即：**Digger 想"只换面板清单"，现在做不到**，因为面板清单散落在上述全部位置，不是一个注册点。

### 4.4 共享领域模块放在 app 根，形似"共享数据库"

`crate::conversation`（usage、conversations 使用）、`crate::delivery`（files、codehealth 使用）、`crate::project`（files 使用）是 `dozer-app` 根下的模块，被多个面板共用。微服务语境下这是"多个服务共享同一份内部模型"。它们是留在 host、下沉到 `dozerd`，还是各自成为服务，**未审计**，见 §6 Q7。

### 4.6 Git 现状：散落在 GUI 进程里，没有底层

按 `Command::new("git")` / `git2::` / `gix::` 的调用点统计：

| 位置 | 调用点 | 说明 |
|------|--------|------|
| `dozer-app/extensions/git_log.rs` | 94 | 提交图、diff、分支 |
| `dozer-app/extensions/file_history.rs` | 40 | 单文件历史，与上面部分重复实现 |
| `dozer-app/delivery.rs` | 36 | 文件 git 状态、worktree 列表（被 files、codehealth 使用） |
| `dozer-app/extensions/codehealth/git_hotspots.rs` | 5 | 热点统计 |
| `dozerd/projects.rs` | 2 | **另一份实现**：守护进程自己取项目 branch/head/dirty，经协议字段 `git_head`/`git_branch`/`git_dirty` 带给 GUI |
| `dozer-app/extensions/usage`、`chrome/homespace.rs`、`platform/file_history_overlay.rs` | 各 1–2 | 零散使用 |

另有 `git_watch.rs`、`git_accounts.rs` 在 `dozer-app` 根。`git2` 依赖**只在 `dozer-app/Cargo.toml`**。

观察：
- Git 能力约 180 个调用点分布在至少 7 处，**没有任何一个 crate 是"Git 底层"**；`dozerd` 与 `dozer-app` 各有一份自己的 git 读取。
- Digger 若复用面板，就会连带 `dozer-app` 里这堆散落代码；若不复用就得再写一遍。
- 这与"Git 是底层必备组件"的定位正好相反，是本次最大的结构性缺口。

### 4.5 现有 MCP 面（`dozer-mcp`，11 个 tool）

`preview_navigate`、`submit_session_summary`、`list_todos`、`add_todo`、`toggle_todo`、`edit_todo_text`、`write_memory`、`list_memories`、`get_memory`、`locate_in_file`、`apply_precise_edit`。

观察：
- 只覆盖 Todo、Memory、Preview/Files 定位与编辑、会话摘要；**Usage、Conversations、Git、Database、SSH、Code Health 没有任何 MCP 接口**，面板之间互访在这些域目前无路可走。
- 是**单一集中式** stdio server，不是"每个面板自己的服务"。R2 的微服务形态要求改成注册制 + 聚合。
- 项目 `CLAUDE.md` 的 crate 表把它描述为"只读 MCP stdio server"，与 `add_todo`、`write_memory`、`apply_precise_edit` 等写工具不符，文档需订正（与本文无关，顺手记一笔）。

## 5. 目标形态（草案）

```text
Host（窗口 / 布局 / 焦点 / 浮层 / 主题 / 面板 Registry）
  ├── 底层支柱：Git 服务、Agent 服务（完整 API，所有面板与产品共用）
  ├── 其他平台服务（typed service）：Project、Conversation 数据…
  ├── 事件总线：状态变更通知（带 revision）
  │
  ├── Panel A ─┐   面板只依赖 Host SDK 契约：
  ├── Panel B ─┼── 服务调用 + 事件订阅 + UI 契约
  └── Panel C ─┘   不 import 其他面板，不碰 App/Workspace

  dozer-mcp / MCP Gateway：agent 通道，面板可选择暴露自己的 tools
```

要点：
1. 面板在 Registry 注册：标题（i18n key + fallback）、默认栏、图标。`PanelKind` 由封闭枚举改为注册制 ID。
2. 面板对 Host 只依赖 SDK 契约；`App`/`Workspace` 不得出现在面板代码里。
3. 面板之间共享的能力（如 diff 读取）沉到 Host 服务，不留在某个面板里让别的面板借用。
4. 面板对 Host 服务的依赖要能被测试替身（fake service）替换，这也是"一个面板 + Host 可运行"的验收手段：最小 Host + 一个面板 + fake 服务能跑起来。

## 6. 待解决问题（持续追加）

| # | 问题 | 现状 / 倾向 | 状态 |
|---|------|-------------|------|
| Q1 | 面板间互访是否走 MCP | 不走，见 §2.1 | **已决** |
| Q2 | "独立运行"的含义 | 一个面板 + Host，见 §2.3 | **已决** |
| Q3 | 进程内 MCP 传输 | 随 Q1 取消 | 作废 |
| Q4 | Git 服务放哪 | 已定性为底层（§2.5）；**具体承载（独立 crate / 独立仓库 / 并入 `dozerd`）待定**，见 Q14 | 部分已决 |
| Q5 | 面板调用 Host 服务的权限 | 本地可信程序，第一阶段不做面板级权限 | 暂缓 |
| Q6 | 事件总线上，订阅的面板缺席/Host 服务不可用时的降级约定（例：Usage 没有 Conversation 数据） | 需统一约定而非各面板自定 | 待设计 |
| Q7 | `conversation`/`delivery`/`project` 三个共享模块的归属：本质都是"Host 服务"候选 | §4.4，**未审计其内部**，不知道哪些是纯数据哪些带 UI | 待审计 |
| Q8 | `PanelKind` → 注册制：887 处引用怎么分批 | 先分类："遍历全部面板" vs "特判某个面板" vs "仅类型传递" | 待审计 |
| Q9 | 事件总线的承载：复用 `dozerd` UDS 协议，还是 Host 进程内 channel | 需要定事件模型（序号、revision、订阅粒度、背压） | 待设计 |
| Q10 | 面板对 Host 的 SDK 契约清单：`HoverId`、`theme`、`menu_spec`、`tab_widget` 哪些进 byteui、哪些进 Host SDK | §4.2 表是起点 | 待设计 |
| Q11 | Digger 的 Todo 语义（选题 vs 验收项）与 agent/conversation 摄取对非编码 agent 的支持 | 复用前需确认 | 待产品确认 |
| Q12 | Git 服务的边界 | 已定：完整底层 API，不是 diff 读取子集（§2.5）；**现状盘点已完成，见 `bytegit-调用点盘点.md`**：生产代码几乎只读，仅 4 个写操作，commit/worktree/merge 等现状为零；边界问题 B1–B6 已裁决；**设计规格草案：`docs/superpowers/specs/2026-10-02-bytegit-design.md`** | 部分已决 |
| Q14 | `bytegit` 的设计：①API 清单（Q12）；②`git2` 还是命令行 git（现状并存，需统一）；③同步还是提供异步封装（GUI 里现在如何包 blocking 调用）；④`git_watch`（变更监听）属于 `bytegit` 还是 Host 事件总线的生产者；⑤`git_accounts`（凭据）归属；⑥独立仓库按 tag 引用，沿用 byteui 的 patch 联调流程 | 形态已定：独立库；其余待设计 | 部分已决 |
| Q15 | Agent 作为另一根支柱：现有 agent 能力（启动、会话、hook、PTY）在 `dozerd` 与 `dozer-app` 中的分布是否也像 Git 一样散落 | 需同样做一次分布审计；是否抽成与 Git 对称的底层 | 待审计 |
| Q13 | Host 自身对 `git_log` 的直接引用（`app/update.rs` 里分支选择器直接调 `git_log::update`、`app/state.rs` 引用 `FileFilter`）是否也要经 Registry | Host 对面板的硬编码属 Q8 范畴 | 待审计 |

## 7. 下一步

1. **Git 底层化作为第一个纵向切片，`file_history`/`git_log` 是它的第一批消费者。** 先为 `bytegit` 定 API 清单与设计（Q12、Q14），再把 `file_history`、`git_log`、`delivery`、`git_hotspots`、`dozerd/projects.rs` 的调用点逐步迁到它之上。这同时是"一个面板 + Host + fake Git 服务能跑"的第一次验证。
   - **P0 已完成（2026-10-02）：** `byteboyai/bytegit` `v0.1.0` 已发布（`Repo::discover`、`TempRepo` 夹具、基础类型，31 个测试）；P1 计划待写。计划：`docs/superpowers/plans/2026-10-02-bytegit-p0-skeleton.md`。
   - **P1 已完成（2026-10-02）：** `v0.2.0` 已发布（HEAD/分支/远程、工作区状态）；`delivery.rs` 的 `repo_root/is_dirty/file_statuses/branch/remote_url/local_branches/current_branch_has_commits` 与 `git_hotspots::dirty_paths/head_short_sha` 已迁为 bytegit 适配层。计划：`docs/superpowers/plans/2026-10-02-bytegit-p1-queries.md`。
   - **P2 已完成（2026-10-03）：** `v0.3.0` 已发布（历史/diff、blob 读取与分类、`workdir_patch`/`file_bytes_at`）；`git_log`/`file_history` 已迁到 bytegit，两面板共用的 diff 内容层抽到 `extensions/diff_content.rs`，`file_history → git_log` 的面板间耦合已消失。计划：`docs/superpowers/plans/2026-10-03-bytegit-p2-history-diff.md`。
   - **P3 已完成（2026-10-03）：** `v0.4.0` 已发布（`head_commit_time`/`commit_count`/`commit_count_by_day`/`churn`）；`usage` 的提交计数、`git_hotspots::recent_churn`、`dozerd/projects.rs` 的项目更新时间已迁到 bytegit，**`dozerd` 现在也依赖 `bytegit`**。计划：`docs/superpowers/plans/2026-10-03-bytegit-p3-stats-churn.md`。
   - **P4 已完成（2026-10-03）：** `v0.5.0` 已发布（`watch` feature）；项目工作区监听已进 `bytegit`（`bytegit::watch`，忽略名单由 dozer 经 `IgnoreRules` 传入），`dozer-app` 不再直接依赖 `notify`，`git_watch.rs` 已删除；Host 只负责把 `GitChange` 转成消息。计划：`docs/superpowers/plans/2026-10-03-bytegit-p4-watch.md`。
   - **P5 已完成（2026-10-03）：** `v0.6.0` 已发布（写操作 `init`/`clone`/`Repo::checkout_branch`/`git_available`）；`delivery.rs` 的四个函数改为 bytegit 适配层。写操作评估：`init` 用 `git2`，`clone`/`checkout` 留在命令行（本构建 libgit2 无 https/ssh、不执行 hook 与外部过滤器、报错退化），理由见评估存档 `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`。计划：`docs/superpowers/plans/2026-10-03-bytegit-p5-write-ops.md`。

2. 审计 Q7、Q8、Q13。
3. 确定事件总线的最小模型（Q9），先给 Git 状态变化（工作区变更、分支切换）这一条事件用起来。
4. 面板扩展化的整体顺序待定，原文 §11 的试点顺序（Code Health → Todo → SSH）是按"进程外插件"定的，与本文"一个面板 + Host"的目标不同，是否沿用待议。
