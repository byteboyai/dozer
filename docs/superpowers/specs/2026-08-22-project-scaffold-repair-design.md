# 项目脚手架：打开即 ensure + "修复项目"落地 Design

**Status:** 已批准设计，待写实现计划。

## 背景

项目面板（`crates/dozer-app/src/extensions/project.rs`）底部的"修复项目"/"删除项目"目前都是纯 UI 占位——`Message::RepairProject => {}`、`Message::DeleteProject => {}`（`project.rs:345-346`），文档明确写着"逻辑后续接入"（`project.rs:209-212`）。用户诉求：新建项目时自动准备好 README、`.dozer/` 缓存目录、本地 git 仓库（原来没有才建）、导入原有 agent 对话数据；"修复项目"重新检查并补齐这些东西。

调研发现两点收窄了范围：

1. **"项目文档"/"Agent 记忆"已经是成熟功能**，不需要新做——`crates/dozer-app/src/extensions/project/links.rs` 的 `load_or_discover`（`links.rs:177-187`）在项目**首次没有 `links.json`** 时自动扫描 README/docs 目录、CLAUDE.md/AGENTS.md 等文件，之后落盘为可编辑列表。这个模式（"文件不存在就发现,存在就直接读"）正是本设计想给其它几项也套用的。
2. **`open_project` 是 upsert，没有干净的"这是新项目"信号**——`crates/dozerd/src/server.rs:260-263` 的 `Request::OpenProject` 处理不区分新建/重开。所以"新建时做的事"和"修复项目按钮"没必要是两套逻辑,统一成一个幂等的 ensure 操作即可(讨论中已拍板)。

## 范围边界

- 本设计只覆盖 README / `.dozer/` 目录 / git 仓库 / agent 历史数据导入这四项，以及承载它们的"可扩展 ensure/repair 步骤列表"这个机制本身。
- **不**在这次给 todo/ssh/database/links 等 extension 写具体的 ensure/repair 逻辑——它们"文件不存在=空状态"目前已经是正确行为，没什么好修的；机制留好扩展点，以后需要时各自加一行注册（讨论中已拍板：只建机制，不铺量）。
- **不**涉及"删除项目"三级方案——那是独立设计（见另一份 spec，尚待写）。
- 项目文档/Agent 记忆的自动发现逻辑（`links.rs`）不改动，继续按现状工作。

## 架构与数据流

### 1. 触发时机：打开项目即跑，不区分新建/已有

`Message::ProjectTabOpened`（`app.rs:4359` 分发到 `app.rs:4813` 的 `project_tab_opened`）成功路径（`project` 为 `Some`）触发一次 ensure；"修复项目"按钮触发同一个操作，区别只是后者要把结果聚合成一行状态文字展示、前者静默跑（讨论中已拍板：每次打开项目都检查，复用 `links.rs` 的幂等发现思路；agent 数据导入本身用增量 offset，重复跑代价可忽略）。

触发范围限定在"一个项目变成新打开的 tab"这个事件本身，不含"在已打开的 tab 之间切换"（`ProjectTabSwitch`/`ProjectSelect` 里"选中已开着的项目"这条路径不重复触发）——`ProjectSelect` 是否还有"项目未开、需要先当新 tab 打开"的分支需要在写实现计划时把调用图理清楚，不在本设计里空猜。

### 2. 可扩展的 ensure/repair 步骤机制

新增一个小类型（新文件，暂定 `crates/dozer-app/src/project_scaffold.rs`）：

```rust
/// 一次 ensure/repair 检查的结果：`AlreadyOk` 不需要做任何事，
/// `Created(desc)` 是这次新建/修复了什么(供"修复项目"状态文字展示)，
/// `Failed(msg)` 是这一步失败但不阻塞其它步骤继续跑。
pub enum ScaffoldStepResult {
    AlreadyOk,
    Created(String),
    Failed(String),
}

/// 一个同步、可幂等重跑的 ensure 步骤。`label` 是展示用短标签。
pub struct ScaffoldStep {
    pub label: &'static str,
    pub run: fn(&Path) -> ScaffoldStepResult,
}

/// 当前登记的同步步骤(README/`.dozer` 目录/git 仓库三项，均为纯函数、
/// 均对着项目根目录做存在性检查)。以后 todo/ssh/database 等 extension
/// 需要真正的 ensure/repair 逻辑时,在这里加一行调用自己模块里的函数，
/// 不需要改 `ScaffoldStep`/`ScaffoldStepResult` 这两个类型本身。
pub fn scaffold_steps() -> Vec<ScaffoldStep> {
    vec![
        ScaffoldStep { label: "缓存目录", run: ensure_dozer_dir },
        ScaffoldStep { label: "README", run: ensure_readme },
        ScaffoldStep { label: "git 仓库", run: ensure_git_repo },
    ]
}
```

`ensure_dozer_dir`/`ensure_readme`/`ensure_git_repo` 三个函数的幂等判据：

- `ensure_dozer_dir`：`repo.join(".dozer")` 不存在就 `create_dir_all`；`create_dir_all` 本身对已存在目录是 no-op，这一步严格意义上不需要"先检查"，直接建、按建之前是否已存在决定返回 `AlreadyOk`/`Created`。
- `ensure_readme`：复用 `links::discover_docs(repo)` 的结果——如果里面已经有任何文件（`discover_docs` 只在根目录找 `readme`/`changelog`/`contributing`/`license` 前缀的文件，`links.rs:68,83-114`），说明已经有能当"项目说明"的文件，跳过；否则在根目录写 `README.md`，内容是 `# {repo 目录名}\n`（最简模板，不做更多内容生成——如果以后要接 AI 生成简介，是另一个独立的设计）。
- `ensure_git_repo`：`repo.join(".git")` 不存在时调用已有的 `crate::delivery::init_repo(repo)`（`delivery.rs:459-469`，同步 shell 出 `git init`）；已存在则 `AlreadyOk`。

这三步都是同步、可能阻塞的文件系统/子进程调用（`init_repo` 尤其如此），因此**都不在 `update()` 里直接跑**，而是打包进一次 `tokio::task::spawn_blocking`——这正是 `files.rs` 现有 `Message::GitInit` 处理里已经用过的手法（`files.rs:754-759`：`handle.spawn(async move { tokio::task::spawn_blocking(...).await; emit(...) })`），这次把三步都塞进同一个 `spawn_blocking` 闭包里顺序跑完，不需要三条并行的完成消息。

### 3. Agent 历史数据导入：新增一个 dozerd 请求

前一轮讨论已经确认：`ingest_session` 对同一个 `conversation_id` 是从已记录的 `parsed_offset` 增量读，重复调用代价小、且能顺带补上尚未被 hook 实时摄取的旧内容。新增（`dozer-core::protocol`）：

```rust
/// 补录一个项目目录下、目前 dozerd 还没摄取过的 agent transcript 历史
/// (以及追平任何已摄取文件里新增的尾部内容)。跟 `OpenProject` 分开——
/// 那个只管项目登记，这个专门负责"扫这个项目的三家 agent 目录，该导入
/// 的都导入"。
Request::BackfillProjectTranscripts { cwd: String },
Reply::BackfillDone { imported_files: u32 },
```

`dozerd` 处理：用 `dozer_core::agent_paths::{claude,codebuddy,opencode}_project_dir_in(home, cwd)`（已有函数，`get_conversation_turns` JOIN 那次调研里确认过同一套用于 DB 查询）列出这个项目在三个 agent 各自目录下的 `.jsonl` 文件，对每个文件调用 `TranscriptStore::ingest_session`（`mod.rs:117-258`，已有函数，全量/增量兼容）。这是 `crates/dozerd/src/backfill.rs` 里已有的 `backfill_all(store, files)` 的**按项目收窄版**，不是全新逻辑——`backfill_all` 现在是"扫全部项目、daemon 启动时跑一次"，新函数是"扫一个项目、任意时刻可以按需跑"，两者可以共享同一个"对一批 `(AgentKind, PathBuf)` 逐个 `ingest_session`"的内部循环。

`dozer-app` 侧：`spawn_review_load` 一类的既有 async-spawn 模式，调用 `client.backfill_project_transcripts(cwd)`（新增的 `dozer-client` 包装方法，照抄 `get_conversation_turns` 那种"包一层 `roundtrip`"的写法）。

### 4. 聚合结果 + "修复项目"状态文字

项目面板状态（`project.rs` 里 `WorkspaceState`/项目面板自己的状态结构，具体挂载点留给实现计划确认）新增：

```rust
pub struct ScaffoldReport {
    pub steps: Vec<(String, ScaffoldStepResult)>, // 三个同步步骤 + "agent 历史" 一项
}
```

流程：`RepairProject`（或 `ProjectTabOpened` 成功路径）触发时，`handle.spawn` 一个任务：先 `spawn_blocking` 跑完三个同步步骤拿到结果，再 `await` 一次 `backfill_project_transcripts`（成功即 `Created("导入 N 个文件")`/`imported_files == 0` 时 `AlreadyOk`，失败 `Failed(msg)`），四项结果拼进一个 `ScaffoldReport`，用一条新消息（如 `Message::ProjectScaffoldDone(project_id, ScaffoldReport)`）送回。

- `ProjectTabOpened` 触发的这次：收到 `ScaffoldReport` 后**不展示**，只是让副作用（README/`.dozer`/git/历史数据）落地。
- "修复项目"按钮触发的这次：收到后把 `ScaffoldReport` 存到项目面板状态，渲染成一行内联状态文字（如"已修复：git 仓库、README；已是最新：缓存目录；agent 历史导入失败：xxx"），不新建 toast/通知系统（讨论中已拍板）。

两条触发路径调用同一个"跑四步骤"函数，只是外层调用方决定要不要把结果落进"要展示"的状态字段——机制本身不重复。

## 错误处理

- 四步里任意一步失败（如系统没装 git、README 写入权限不足、`BackfillProjectTranscripts` 请求失败），不阻塞其它三步继续执行——每一步独立捕获错误，汇总进 `ScaffoldReport`。
- `ProjectTabOpened` 触发的静默 ensure，失败只记 `tracing::warn!`，不打断"项目已打开"这个既成事实（跟现有"项目文档发现"失败时的降级方式一致——`discover_docs`/`discover_memory` 读目录失败就返回空列表,不 panic）。
- `ensure_readme` 的"已存在判据"复用 `discover_docs`，如果那次目录读取失败（权限问题等），保守起见**不**创建 README（避免在读不到目录真实状态时误判"没有 README"而覆盖式创建，宁可这一步报 `Failed`，用户可以再点一次"修复项目"重试）。

## 测试

- `ensure_dozer_dir`/`ensure_readme`/`ensure_git_repo` 三个纯函数：tempdir 场景覆盖"缺失→创建"“已存在→`AlreadyOk`”两种，`ensure_readme` 额外覆盖"根目录已有 `CHANGELOG.md`(非 README)→ 判定已有文档、跳过创建"这种边界。
- `dozerd` 新增的按项目收窄扫描函数：沿用 `get_conversation_turns_paginates_by_keyset`/`ingest_session` 现有测试用的 `fixture()`/`tempdir` 模式，构造一个项目目录下两个 agent 各一份 transcript，断言两份都被 `ingest_session` 且能在 DB 里查到。
- `ScaffoldReport` → 状态文字的格式化函数：纯函数，给定几种 `steps` 组合断言输出字符串。
- 触发时机（`ProjectTabOpened` 是否真的只在"新开 tab"而不在"tab 间切换"时触发）：写实现计划阶段先把 `ProjectSelect`/`ProjectTabSwitch`/`ProjectTabOpened` 的调用图核实清楚，再决定这一条要不要写成单测还是靠代码结构本身保证。

## 已知取舍（不在本设计范围内，记录避免以后重新讨论）

- todo/ssh/database/links 等 extension 目前不接 `scaffold_steps()`——它们"文件缺失=空状态"已经正确，没有当下就要修的坏状态。机制留了扩展点，真的出现"某个 extension 的 `.dozer/*.json` 损坏需要探测并恢复"的需求时再补，不预先猜需求形状。
- README 内容目前只给最简一行标题模板，不接 AI 生成的项目简介——如果以后要做，是构建在这个"README 缺失就创建"判据之上的独立功能，不是这次的一部分。
- `BackfillProjectTranscripts` 和 daemon 启动时的全局 `backfill_all` 共享"对一批文件逐个 `ingest_session`"的内部循环，但不做成"daemon 启动就不用再跑这个了"这种互斥优化——两条路径独立触发、独立幂等，没有必要互相感知对方跑没跑过。
