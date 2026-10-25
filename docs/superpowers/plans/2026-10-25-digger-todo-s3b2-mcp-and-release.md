# S3b-2 Digger MCP 待办工具(S6e-3b)与发布 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Digger 里的 agent 能通过 MCP 读写当前项目的待办(`list_todos` / `add_todo` / `toggle_todo` / `edit_todo_text`),改动在待办面板里 1 秒内可见;随后发布 Digger 的新版本。

**Architecture:** 工具代码已在 `dozer-mcp`(Dozer 与 Digger 共用),只需把 `digger-mcp` 入口的 `McpFeatures.todos` 从 `false` 改 `true`,并改对应测试。**前置:S3b-1 已合并**(面板存在且有 1 秒轮询),否则 agent 改了数据而用户看不到。

**Tech Stack:** Rust、`dozer-mcp`(rmcp stdio)、`dozer-client`

**Spec:** `docs/superpowers/specs/2026-10-25-digger-todo-panel-s3b-design.md`(§7 Q5)、`2026-10-18-agent-mcp-s6e-design.md` §6.3

**工作位置:** Digger 仓库,独立分支(如 `feat/mcp-todos`),建在 S3b-1 合并后的 `main` 上。

## Global Constraints

- 只改 `digger-mcp` 入口的特性开关与测试;**不改 `dozer-mcp`**(若发现必须改,停下来单独列任务并走 Dozer 发新 libs tag,新 tag 的依赖钉必须与 Digger 的 byteui / bytehost / bytegit 钉一致)。
- `summary` 仍为 `false`(`submit_session_summary` 依赖 `diggerd` 的总结流水线,默认关,见 S6a D7)。
- **对外动作(版本号提交、打 tag、推送、安装到 `/Applications`)每一步都先问用户**,不凭本计划自行执行。
- 不碰真实数据与用户常驻 `diggerd`:测试一律独立 `DIGGER_DATA_DIR`。
- 提交信息末尾带:`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`

## Review Focus

1. **Digger 会话里同时配了 `dozer` 与 `digger` 两条 MCP**(S6e 设计 §2 的陷阱):`dozer-mcp serve` 缺 `DOZER_SESSION_ID` 会报错退出,agent 的 MCP 状态里 `dozer` 一直显示失败。要确认 `digger-mcp` 的待办工具**不受影响**,并在验收清单里把这条已知现象写明(Task 3)。
2. **会话所在目录不属于任何已登记项目**(Task 1):`list_todos` / `add_todo` 返回人能读懂的错误,不 panic、不写到别的项目。
3. **`add_todo` 传空串或只有空白**(Task 1):**2026-10-25 核实,现状不拒绝**——`dozer-mcp` 的 `add_todo` 直接转给 `Client::add_todo`,`dozerd` 的 `TodoStore::add` 也不校验,Dozer 的 agent 同样能建出空任务。本期**钉住现状**(测试名 `blank_todo_text_is_accepted_today_like_dozer`,注释写明"改成拒绝要动 dozer-mcp / dozerd,走 Dozer 发新 libs tag"),并在验收清单与报告里列为已知行为;是否收紧由用户另议,不在本任务里改。
4. **`toggle_todo` / `edit_todo_text` 的 `id` 不限定在本会话的项目内**(与 Dozer 现状一致,见 `dozer-mcp/src/server.rs`):agent 能改别的项目的任务。本期保持与 Dozer 一致,不单独收紧;在验收清单里列为已知行为,是否收紧另议。
5. **已经开着的 Agent 会话看不到新工具**:agent 只在会话启动时读一次工具清单,升级后要重开会话(Task 3 的真机清单要写明,免得把它当成 bug)。

---

### Task 1: 打开待办工具并更新测试

**Files:**
- Modify: `crates/digger-mcp/src/main.rs`(`todos: true`,更新文件头注释"记忆 + 待办 + 预览桥接 + 精确编辑")
- Modify: `crates/digger-mcp/tests/tool_set.rs`、`crates/digger-mcp/tests/memory_roundtrip.rs`(其中复制了一份 `McpFeatures`,与入口保持一致)
- Create: `crates/digger-mcp/tests/todo_roundtrip.rs`

**Interfaces:**
- Consumes: `dozer_mcp::McpFeatures`、`McpFeatures::tool_names()` / `all_tool_names()`、`memory_roundtrip.rs` 里已有的"起隔离 `diggerd` 并直接调工具函数"的做法
- Produces: 无新公开接口

- [ ] **Step 1: 改测试(先红)**
  - `tool_set.rs`:`tools_list_is_memory_plus_todos_plus_the_preview_and_edit_bridge`:`digger_features().tool_names()` 等于**以 `tool_names()` 的实际分组顺序**排出的 `write_memory, list_memories, get_memory, list_todos, add_todo, toggle_todo, edit_todo_text, get_preview_context, preview_navigate, locate_in_file, apply_precise_edit`(顺序以 `dozer-mcp` 的分组顺序为准,先在 Dozer 的同名测试里看一眼,不要猜);保留"同名项落在 `all_tool_names()` 同一份路由表"的断言。
  - 把 `todo_and_summary_tools_stay_off` 改成 `only_the_summary_tool_stays_off`:`submit_session_summary` 不在清单里,四个待办工具在。
  - `todo_roundtrip.rs`:
    - `add_then_list_then_toggle_then_edit`:起隔离 `diggerd`、登记一个项目、造一个属于该项目的会话身份(同 `memory_roundtrip.rs`),依次 `add_todo("买牛奶")` → `list_todos` 含该条且 `done=false` → `toggle_todo(id, true)` → 再 `list_todos` 为 `done=true` → `edit_todo_text(id, "买燕麦奶")` → 再 `list_todos` 文字已变;最后经**独立的 `Client`** 直接 `list_todos(project_id)` 读到同样结果(证明落进 `digger.db`)。
    - `a_session_outside_any_registered_project_gets_a_readable_error`(Review Focus 2):会话 cwd 不属于任何项目时 `add_todo` 返回 `Err`,错误文本非空且不含 `panic`。
    - `blank_todo_text_is_accepted_today_like_dozer`(Review Focus 3,**钉住现状**,见上):`add_todo("")` 成功并产生一条空任务;测试注释写明这是与 Dozer 一致的现状、收紧另议。
    - `todo_ids_are_not_scoped_to_the_session_project`(Review Focus 4,**钉住现状**):项目 A 的会话对项目 B 的任务 id `toggle_todo` 成功;测试名与注释写明"与 Dozer 现状一致,收紧另议",以后收紧时这条测试会提醒同步改。
- [ ] **Step 2: 跑测试确认失败**:`cargo test -p digger-mcp`,预期 `tool_set` 与 `todo_roundtrip` 失败(开关还是 `false`)。
- [ ] **Step 3: 实现**:`main.rs` 里 `todos: true`;`memory_roundtrip.rs` 里的特性副本同步改(或改成引用一个共享的 `digger_features()` 辅助,二选一,选改动小的)。
- [ ] **Step 4: 跑测试确认通过**:`cargo test -p digger-mcp`;`cargo clippy -p digger-mcp --all-targets -- -D warnings`
- [ ] **Step 5: Commit** `feat(mcp): digger-mcp 开启待办工具(S6e-3b)`

### Task 2: agent 改动在面板里可见

**Files:**
- Create: `crates/desktop/tests/todo_mcp_visible_in_panel.rs`

**Interfaces:**
- Consumes: S3b-1 的 `TodoPanel`(`set_project` / `activate` / `poll` / `update` / `items`)与 Task 1 的工具调用方式

- [ ] **Step 1: 写测试** `agent_added_todo_shows_up_within_one_poll_interval`:起隔离 `diggerd`、登记项目、`TodoPanel` `set_project` + `activate` 并等首轮加载完成;经 MCP 工具函数 `add_todo("由 agent 新增")`;推进时间 `POLL_INTERVAL` 后 `poll(now)`,把 `rx` 里的回流消息喂进 `update`;断言 `items()` 含该条。再对 `toggle_todo` 同样断言列表里 `done` 变化。
- [ ] **Step 2: 跑测试**:`cargo test -p digger-desktop --test todo_mcp_visible_in_panel`,预期通过(轮询在 S3b-1 已实现;若失败说明 S3b-1 的轮询有缺口,回 S3b-1 修,不要在这里绕)。
- [ ] **Step 3: Commit** `test(todo): agent 经 MCP 改的任务在面板里 1 个轮询周期内可见`

### Task 3: 真机验收、文档状态与发布

**Files:**
- Create: `docs/superpowers/reviews/2026-10-25-todo-s3b2-acceptance.md`
- Modify: `2026-10-18-agent-mcp-s6e-design.md` 与 `2026-10-18-agent-panel-s6-design.md` 的状态行(S6e-3b 已完成、M1 的剩余项);`2026-10-25-digger-todo-panel-s3b-design.md` 状态行

- [ ] **Step 1: 写真机清单**(用户在沙箱 Digger 里勾;沙箱 = 独立数据目录 + 临时 HOME,**新装的版本**,Agent 会话**重开**——Review Focus 5):
  1. Agent 面板新开一个 Claude 会话,让它"列出我的待办":返回与待办面板一致的列表。
  2. 让它"新增一条待办:xxx":待办面板约 1 秒内出现该条;面板收起时新增,再展开后也在。
  3. 让它"把 xxx 标记完成":面板里该条变为已完成。
  4. 让它改一条任务的文字:面板里文字更新。
  5. 对 Codex、其它已接入的 agent 各重复 1–4 中的任意两项。
  6. Agent 的 MCP 状态页:`digger` 显示可用、工具数比升级前多 4;`dozer` 一项若显示启动失败,属于 S6e 设计 §2 的已知并存现象(Review Focus 1),确认与待办工具无关。
  7. 会话目录是一个没登记的项目:让 agent 调 `list_todos`,得到一条可读的错误而不是卡住。
  8. 已知行为(不算 bug):agent 用别的项目的任务 id 能改成功(Review Focus 4);升级前已开着的 Agent 会话看不到新工具。
- [ ] **Step 2: 门禁**:`bash scripts/release.sh --dry-run`(格式 / clippy / 依赖门禁 / 全量测试,不改文件不打包),预期全过;失败要贴输出,偶发失败必须单独重跑量化(重复次数与失败次数写进报告),不能重跑到绿就算。
- [ ] **Step 3: 等用户确认真机清单通过**,再进入下一步。
- [ ] **Step 4(对外动作,逐项问用户)**:`bash scripts/release.sh --allow-dirty`(若工作区有用户的未提交改动)→ 审阅 `git diff`(版本号、`Cargo.lock`)→ 提交 `chore(release): <版本>` → 安装到 `/Applications/Digger.app`(先把旧版挪到备份位置,不删除)→ 签名校验 `codesign --verify --deep --strict` 与 `CFBundleShortVersionString` 核对。Digger 无远端,不推送;若这期同时改了 Dozer 库(本计划预期**不改**),Dozer 另走自己的发版流程。
- [ ] **Step 5: 更新状态行并提交文档**;在 S6 总设计稿里把 M1 的剩余项更新到当时的真实状态。
