# bytegit 发布就绪复核：实例 owner 修复与 Agent 异步投影

日期：2026-11-01。Dozer 基线 `6296b6e4`（功能修复 `734f2800`），Digger `88fcf4e`。下文复核阶段未改生产代码；末尾记录后续生产修复与验证。未发布或安装。

## 上轮 Q1–Q3

- Q1：Workspace 的 git_owner 来自进程级原子分配，随 ready 消息返回；宿主同时校验 owner/request。销毁重开不再仅凭重复的 request=1 接受旧确认。
- Q2：Files 的 RefreshOwner 随新实例创建；目录请求、两个宿主回包和 apply_tree_refresh 均携带/校验 owner 与 generation。同实例 reset 与销毁后 new 两类边界已有隔离。
- Q3：Agent 候选 tab 在后台解析真实 WorkspaceKey，嵌套 linked worktree 不再只凭路径前缀被判定为父工作区。路径前缀仍作候选筛选，但最终匹配以真实身份为准。

以上源码修复成立，不再重复报告上轮原缺陷。两个项目 offline 依赖守卫均通过。不过 Q3 新增的异步投影链还有下列时序问题。

## A1 · P2：AgentGitProjection 仅校验订阅 token，异步结果未绑定快照版本与 tab cwd

位置：Dozer `crates/dozer-app/src/app/update.rs:1041,2795`、`app/message.rs:333`。

每个工作区快照独立启动一个后台任务，按捕获的 tab_id/cwd 解析 WorkspaceKey。任务完成后发送 `(project_id, token, tab_ids, branch, dirty)`。接收端只检查 git_token_current，然后按 tab_id 直接覆盖 workspace_override。

订阅 token 在正常文件修改、HEAD 切换以及 tab 内 cd 时都可以保持不变，因此无法辨别：

1. **旧快照晚完成。** 快照 S1 的身份解析慢于 S2，S2 先更新卡片；S1 后到时仍通过 token 检查，把较新的 branch/dirty 回滚为旧值。即使 runtime 本身按 generation 丢弃过期查询，这一层在收到快照后新增的异步任务也没有延续该版本约束。
2. **cwd 在解析期间改变。** 任务根据原 cwd 判断 tab 属于父工作区，返回前用户已 cd 到另一 worktree，独立卡片信息也已更新。回包仅带 tab_id，没有解析时 cwd/会话身份；旧父工作区信息仍会覆盖该 tab。前置 WorkspaceKey 比较只证明捕获的旧 cwd，不证明应用结果时 tab 的归属。

这不会修改 Git 文件，但会使 Agent 卡片显示错误的分支和脏标，因此在本次刷新一致性任务收口前应修复。

建议：为 Agent 投影保留快照的 HEAD/status generations 或专属投影请求序号；回包携带每个 tab 的会话身份、捕获 cwd 或 cwd 版本。应用时同时验证当前工作区 token、最新投影版本和 tab 当前归属。不能只给每个后台任务分配一个新序号却不在应用端与最新请求比较。

## 本次验证证据

隔离目录：`/private/tmp/bytegit-owner-review.hSQrmo`。

| 检查 | 结果与范围 |
|---|---|
| Files 同实例 reset 与新实例 owner 隔离 | 2 项通过；原样提取生产 state 方法和 RefreshOwner 定义，导入实际 fs.rs |
| Files 原 fs 模块 | 34 项通过 |
| Dozer 旧 Workspace ready 拒绝 | 1 项通过；原样提取 git_subscription_ready、next_git_owner 与静态分配器，配真实 runtime/token，替换无关 App/UI 结构 |
| Agent 旧投影覆盖新投影 | 1 项失败；原样提取生产 AgentGitProjection 消息处理分支及 apply_workspace_override，模拟 S2→S1 完成顺序 |
| Agent cwd 改变后旧投影仍应用 | 1 项失败；相同提取代码，模拟 tab 已进入另一 worktree |
| 两产品 check-shared-deps.sh --offline | 均通过 |

这些是定向源码提取探针，不是完整 iced App/GUI 的端到端测试。本次未重跑完整 App 测试、release 构建或人工 GUI 验收；不把执行方的已有结果充作本次亲自执行。

失败反例保存在 [reproduce.rs](../patches/2026-11-01-agent-git-projection-review/reproduce.rs)，只依赖标准库：

```sh
rustc --edition 2024 --test docs/patches/2026-11-01-agent-git-projection-review/reproduce.rs -o /private/tmp/agent-projection-review
/private/tmp/agent-projection-review --nocapture
```

其中 token 与 App/SessionTab 的无关结构使用最小替身，保留修复前的生产分支和覆盖函数原文。两个断言在该历史反例中预期失败，不代表下方修复后的生产行为。

复核阶段结论：上轮三个问题已修复，依赖门禁通过；发现一项新增的 P2 异步投影问题，修复情况如下。

## A1 修复记录

生产实现位于 `crates/dozer-app/src/host_adapters/agent_git_projection.rs`，由真实 App 消息处理分支调用：

- 每个 Workspace 保存投影版本；每次接受快照都推进版本，即使没有候选 tab 也使上次后台结果失效。应用端只接受最新版本。
- 回包携带 tab ID、session ID 与捕获时的 effective cwd；应用时逐项与当前 tab 比较，拒绝已关闭、已换会话或已切换 cwd 的结果。
- 保留原订阅 token 校验与后台 WorkspaceKey 解析，不增加 UI 线程 Git I/O，也不改变共享面板 API 或依赖版本。

隔离验证目录：`/private/tmp/dozer-agent-projection-fix.dYcwFa/source`。从基线归档覆盖本次五个生产源码文件，使用锁定的远程依赖，不加载开发目录的本地依赖覆盖；Cargo cache 与 target 均隔离。

| 检查 | 结果与范围 |
|---|---|
| `cargo test --offline --locked -p dozer-app agent_git_projection -- --nocapture` | 5 项通过；直接编译真实 App、使用真实 SessionTab 和生产投影实现 |
| `cargo test --offline --locked -p dozer-app` | 1146 项通过，2 项忽略；包括上述 5 项 |
| `cargo clippy --offline --locked -p dozer-app --all-targets --no-deps` | 退出 0；仍报告既有代码警告，本次新增投影模块无诊断；未使用 `-D warnings` |
| `scripts/check-shared-deps.sh --offline` | 通过 |
| 面板边界棘轮 | 通过 |
| 本次源码 rustfmt 与 `git diff --check` | 通过 |

首轮完整测试中的 3 个 git_log 用例因源码归档缺少 `.git` 失败；复制基线仓库元数据后完整重跑通过。新增用例覆盖 S2 先完成、S1 后完成，cwd 改变，会话重开，tab 重排/关闭，以及无候选快照撤销旧任务。它们测试真实生产投影状态机，未模拟整个窗口消息循环或人工 GUI 操作。

A1 已在 Dozer 源码修复。以上结论不替代产品的 release 构建、人工 GUI 验收或 Digger 的发布门禁。
