# 会话总结可靠性与完整性修复 Implementation Plan

状态：待实施；本文件只规划工作，复选框不代表完成。2026-09-26。

Spec：[会话总结可靠性与完整性修复](../specs/2026-09-26-session-summary-reliability-design.md)。

## 实施原则

- 按 Task 1 → 8 推进；每个任务包含实现、针对性验证及验收证据。不要求额外子代理或外部技能。
- 仅修改本仓库；保留用户未提交的无关改动。当前 review-trace 与 docs/analysis 改动不属于本计划。
- 总结执行与 Todo 执行隔离：共享 headless 文件时不改变任务处理的编辑权限、超时或输出契约。
- 优先迁移与独立服务，再接协议/UI，最后切换关闭入口；不提前删除旧读取路径。
- 常规单元/集成测试使用 fake runner，不调用付费模型；最后单独进行真实 CLI 和 UI 验收。不要对用户全库批量重算来验证实现。
- CLI 参数及模型能力在实施时核验；文档现有注释不能代替当前版本验证。能力无法落实时如实记录阻塞，不静默降级。

## Task 1：冻结输入契约与存储迁移

文件：`crates/dozerd/src/session_summary.rs`、`transcripts/mod.rs`、新 `summary_jobs.rs` / `summary_snapshot.rs`、`lib.rs`、`main.rs`。

- [ ] 定义规范快照、revision 哈希、pipeline_version、结果覆盖元数据与任务错误分类。
- [ ] 新建 results/jobs/batches/关联/中间产物表，事务迁移；明确索引、generation 分配和唯一活动任务约束。
- [ ] 实现稳定的旧结果选择与 legacy 映射；保留旧表、NULL 关联和 task_id，不迁移为伪 current。
- [ ] 增加分页读取完整 transcript 的接口，固定读取快照；大输入不在 handler 中一次无限加载。
- [ ] 实现结果与任务状态分离的查询及 CAS 发布；新增内容产生 stale。
- [ ] 验证：空库、旧库、重复迁移、同 conversation 多行、空 AI/heuristic、NULL 关联、同回合数正文变化及旧 generation 完成。

验收：A1/A8 的存储部分；数据库中失败尝试不改变有效摘要。

## Task 2：可诊断、可隔离的总结调用器

文件：`crates/dozerd/src/headless_agent.rs`、新 `summary_provider.rs` / `summary_config.rs`、相关测试。

- [ ] 核验各适配器当前 CLI 帮助/官方契约，形成能力表和所测版本记录；补 Codex 总结适配器。
- [ ] 实现专属 summary 配置和配置优先级，保留 Todo 的 default_agent 行为；无配置返回明确错误。
- [ ] 总结器禁工具/只读隔离、独立临时 cwd、清除 session/MCP 关联；不能提供隔离时报告不支持。
- [ ] 实现整个调用 deadline、有界并发读取 stdout/stderr、stdin 错误、退出码、脱敏日志、kill/wait 和进程组清理。
- [ ] 按 CLI 解析最终模型输出，再校验结构化结果；记录请求模型与可取得的实际模型，未知值不伪造。
- [ ] 实现受预算约束的瞬态重试/格式纠正及 provider 认证错误分类。
- [ ] 用 fake CLI 验证 spawn 失败、非零退出、大 stderr、stdin 提前关闭、超时带子进程、回显 prompt、无效/空结果以及取消清理。

验收：A5/A9；测试确认 Todo 原有执行能力未被总结隔离策略改变。

## Task 3：完整会话分块与递归归并

文件：新 `crates/dozerd/src/summary_pipeline.rs`、`summary_snapshot.rs`、测试 fixtures。

- [ ] 定义结构化 chunk facts 与最终 title/summary schema，附 turn 范围及证据类别。
- [ ] 保留全部有效 human/AI/tool 内容，过滤隐藏 thinking；超长单回合拆段，建立无缺口的输入覆盖区间。
- [ ] 实现预算切块、逐块抽取、递归归并；不继续使用丢头的 `build_transcript_text` 作为新版入口。
- [ ] 处理后续纠正、未验证和未完成事项；只有所有阶段成功才产出完整结果。
- [ ] 持久化 chunk 输出与阶段进度，验证缓存 revision/version/provider 一致才能复用。
- [ ] 落实 64 调用/30 分钟初始预算、预算耗尽错误与重试提高预算入口所需参数。
- [ ] 建短会话、早中晚事实、超长单回合、矛盾纠正、失败验证、只有用户要求、工具缺失等固定 fixtures；fake runner 验证输入覆盖和控制流，人工事实清单留给真实验收。

验收：A3 的覆盖机制；不能把 fake 输出作为模型语义质量通过的证据。

## Task 4：持久调度与修复策略

文件：新 `crates/dozerd/src/summary_service.rs`、`summary_jobs.rs`、`session_summary_backfill.rs`、`main.rs`。

- [ ] 实现单条提交、批次筛选、默认全局并发 1、公平队列、活动任务复用。
- [ ] 默认选择缺失/legacy/heuristic/空白/stale/无有效结果的失败项；支持单条 force。
- [ ] 实现 generation/CAS、输入更新 stale、重启恢复及已成功 chunk 重用。
- [ ] 实现共享 job 的订阅/取消语义；认证等共同错误使余下任务明确失败，不启动大量必败 CLI。
- [ ] 持久化真实批次计数，保证计数恒等式；存储失败不可计 succeeded。
- [ ] 实现中间产物保留清理，不删除有效摘要或诊断元数据。
- [ ] 验证重复提交、批次重叠、共享任务取消、更新中快照、重启恢复、失败后重试以及结果落库失败。

验收：A1/A6；修复一次失败后仍可再次修复，不写 heuristic。

## Task 5：协议与客户端完整接线

文件：`crates/dozer-core/src/protocol.rs`、`crates/dozer-client/src/lib.rs`、`crates/dozerd/src/server.rs`、对应集成测试。

- [ ] 新增 V2 单条/批次提交、进度查询、失败重试、取消及 provider 配置/能力查询接口。
- [ ] 请求携带策略、可选 provider/model/预算；响应返回稳定 ID、阶段、计数、失败原因和结果状态。
- [ ] 同次改动实现 server 全部 match 分支与 client 方法，避免留下可被调用的成功占位回复。
- [ ] 保留旧请求包装与 MCP 兼容，规定旧客户端可见结果；新版 client 识别旧 daemon 不支持。
- [ ] 列表联查新版结果/任务状态，保留 Todo 关联与历史读取。
- [ ] 验证序列化默认值、真实 daemon/client 请求往返、请求失败、批次不存在及 daemon 重连。

验收：A7/A8 协议部分；不通过 0/0 掩盖错误。

## Task 6：关闭、退出、Shutdown 与 Todo 接入

文件：`crates/dozerd/src/server.rs`、`session.rs`（如需退出协调）、`task_processor.rs`、`crates/dozer-app/src/workspace/state.rs`、`workspace/hook.rs`、关闭/Shutdown 测试。

- [ ] 用统一服务替换 PTY SUMMARY_PROMPT + 60 秒轮询；关闭前保存意图，退出后最终摄取并冻结快照。
- [ ] 自然退出在 daemon 生命周期统一处理，不依赖 UI 是否订阅退出事件；主动关闭和自然退出去重。
- [ ] Shutdown 持久化所有候选任务并停止 worker/子进程，不等待模型；启动时恢复。
- [ ] 按 transcript 可用性而非旧 agent 白名单判断可总结，覆盖 Codex；缺 transcript 有可见原因。
- [ ] Todo 不再创建空的 AiGenerated；保留任务关联，实际回合完成后入队。
- [ ] 验证最终一轮落盘、晚到 transcript、关闭立即返回、自然退出无 UI、Shutdown 中重启、无记录 shell 与 Todo 失败记录。

验收：A2/A8；不再有自动产生人类拼接摘要的入口。

## Task 7：会话面板与项目修复 UI

文件：`crates/dozer-app/src/conversation.rs`、`extensions/conversations.rs`、`extensions/project.rs`、`extensions/project/update.rs`、`extensions/project/view.rs`、`app/update.rs`、实际总结详情渲染组件。

- [ ] 从当前渲染调用链定位详情组件，接入独立摘要/任务状态，不只新增后端不可达接口。
- [ ] 增加生成、重试、重新生成；当前摘要在重做期间与失败后保留，历史拼接标为摘录。
- [ ] 提供共享 provider/model 配置组件，无配置明确提示；显示 CLI 默认模型或已知模型。
- [ ] 修复进度展示成功/失败/跳过与分块阶段，失败可展开；提交/轮询失败独立报错。
- [ ] 支持后台继续、显式取消、失败重试及预算提高；重复点击禁用或复用任务。
- [ ] 项目切换/重连按稳定 ID 恢复状态；结果发布刷新列表与已打开详情。
- [ ] 验证 UI 状态映射、错误不变成功、旧 daemon 提示；实际点击完成一次生成、失败重试、隐藏后恢复与取消。

验收：A7；用户可从界面解释“为什么没有总结”，并直接采取修复动作。

## Task 8：回归与真实质量验收

- [ ] 针对改动运行 `cargo test -p dozerd -p dozer-core -p dozer-client -p dozer-mcp -p dozer-app`。
- [ ] 运行 `cargo build --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`；记录已有无关失败，不擅自扩展修复范围。
- [ ] 使用临时数据库与受控 transcript 跑真实 CLI：至少一个实际可用 provider 及新增 Codex 适配器；若同为 Codex，可合并。其他标记可用的适配器也需真实 smoke。
- [ ] 长会话事实清单：最早目标、中部关键修改/决策、最后验证结果、仍未完成事项全部核对；检查没有把请求当成果。
- [ ] 验证模型无法执行夹在 transcript 中的编辑/命令指令；临时项目内容和文件哈希不变。
- [ ] 从“修复项目”真实点击开始，核对数据库结果、provider、revision、UI 成功计数，并用可控失败核对失败计数与重试。
- [ ] 新建 `docs/analysis/2026-09-26-session-summary-reliability-validation.md`，记录版本、命令、A1–A9 结果、受控输入/输出与遗留限制，不包含用户私密 transcript。
- [ ] 仅在代码检查、真实 LLM、真实 UI、迁移回归均有证据时标记完成；环境阻塞项逐条标未验收。

## 交付与回退

交付包含代码、迁移、测试、CLI 能力/版本记录及验收报告。新表不删除旧表；需要回退时停用新 worker 并保留数据库，新失败任务不破坏旧结果。回退到旧二进制只能看到旧表内容，无法展示新版状态/结果，需明确说明这一限制，不自动做破坏性反向迁移。
