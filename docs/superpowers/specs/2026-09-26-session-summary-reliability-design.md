# 会话总结可靠性与完整性修复 Spec

状态：待实施。2026-09-26 用户要求编写修复 spec 和 plan；本文不代表已实现或已通过验收。

配套计划：[Implementation Plan](../plans/2026-09-26-session-summary-reliability.md)。

## 1. 问题与证据

用户实际观察到会话面板没有总结，或仅拼接人类发言；手动“修复项目”也无法有效改善。

当前代码的可复核事实：

| 位置 | 当前行为 | 后果 |
| --- | --- | --- |
| `crates/dozerd/src/server.rs`：`CloseWithSummary`、`finalize_session_summary` | 向原 PTY 注入 prompt，等 MCP 回交 60 秒，超时调用启发式函数 | 关闭路径没有独立 LLM 补救；依赖原 agent 状态及 MCP |
| `crates/dozerd/src/session_summary.rs`：`heuristic_from_turns` | 只取 human，摘要为 `human.join("\n")` | 没有总结 AI 做了什么 |
| `crates/dozerd/src/session_summary_backfill.rs`：`missing_summary_conversations` | 只选没有总结行的 conversation | 降级结果、空摘要、过期摘要不能被修复 |
| `crates/dozerd/src/headless_agent.rs` | 单回合截断至 4,000 字符，总预算 16,000 字符，丢最早回合 | 长 session 只总结尾部 |
| 同上：`run_and_extract` | 丢弃 stderr，不检查退出码 | 无法定位认证、配置及 CLI 失败 |
| `crates/dozerd/src/default_agent_config.rs` | 缺配置默认为 Claude | 总结器未必是用户可用的 agent |
| `crates/dozer-app/src/extensions/project/update.rs` | 请求失败发出 0/0 进度；降级也计为完成 | UI 容易把失败表现为完成 |
| `crates/dozerd/src/task_processor.rs` | 创建空摘要却标记 `AiGenerated` | Todo 会话也会被误判为已有总结 |

2026-09-26 只读检查本机 `dozer.db`：556 条 conversation，其中 180 条无关联总结；总结表有 379 条 `heuristic_fallback`、22 条 `ai_generated`。按来源区分，backfill 有 171 条降级、21 条 AI 标记；普通 session 有 208 条降级、1 条 AI 标记。总结行与 conversation 不是一对一，不能直接相加计算覆盖率；这些是存量记录分布，不是调用成功率。日志确认存在等待 60 秒后降级，不能据此推断所有失败的具体 CLI 原因。

本文取代旧 spec 中“失败写启发式总结”“只补不存在的行”“仅内存进度”“关闭时依赖 PTY 回交”“默认静默选择 Claude”的约定。历史文档保留作为背景。

## 2. 目标与边界

目标：

1. 基于已摄取的完整 session 记录独立调用 LLM，说明目标、实际工作、结果与验证、未完成事项。
2. 自动关闭、项目修复、单会话生成/重做使用同一任务管线；可重试失败、降级和过期结果。
3. 长会话按时间顺序分段归纳再汇总，不静默丢弃早期记录或长回合后半部分。
4. 提供明确的总结器选择、错误原因、进度与来源；只有真实成功才标记成功。
5. 不因总结延迟阻塞 tab 关闭；daemon 重启后任务可恢复；重做失败保留旧成功摘要。

范围：Rust daemon、协议/client、iced 会话面板/修复项目 UI、现有 CLI 总结适配器。需要支持 Codex 作为总结器，并允许任意已摄取 agent 的记录交给所选总结器。总结器与 transcript 来源相互独立。

非目标：新增云端服务、训练模型、每轮对话自动调用 LLM、恢复未被 transcript 记录的信息、修改外部 agent 仓库、重构通用任务执行器。打开项目仍只补 transcript，不自动批量消费模型额度。

## 3. 统一任务与触发规则

新增 `summary_service` 编排层，旧关闭/补录 handler 只负责提交任务。任务输入以 `conversation_id` 为主键关联，PTY 的 `session_id` 仅保留来源信息。

| 入口 | 行为 |
| --- | --- |
| 关闭有 transcript 的 daemon tab | 最终摄取、持久化总结任务后正常关闭 PTY；不注入 prompt、不等 LLM |
| 自然退出 | 在 daemon 统一退出处理处做最终摄取并入队，与主动关闭去重 |
| daemon Shutdown | 终止会话并完成可用记录的最终摄取、持久化待处理任务；不等待模型完成；下次启动恢复 |
| 修复项目 | transcript 补录完成后，处理缺失、旧版、降级、空白、失败及过期摘要 |
| 会话详情“生成总结/重试/重新生成” | 明确针对单条 conversation；允许替换当前有效摘要 |
| 打开项目/浏览列表 | 只读状态，不隐式生成 |

关闭流程先持久化意图，PTY 结束后再摄取最终 transcript 并冻结输入。进程退出早于 transcript 最后落盘时做有界补读；无法可靠读取时记录摄取错误或覆盖不足，不声称完整。没有 conversation 的纯 shell/SSH 不创建虚假占位总结；有 agent 但缺 transcript 时显示原因。

记录仍在更新的会话允许手动总结当前快照；此后新内容使结果变为“有新内容，待更新”。不在生成过程中无限追逐活跃会话。

## 4. 存储、状态与并发约束

保留 `session_summaries` 历史表及 MCP 接口，不破坏已有 task/session 关联。新增规范结果与持久任务，避免旧表同一 conversation 多行的不确定覆盖。

- `conversation_summary_results`：`conversation_id` 唯一；title、summary、结构化 facts、source_revision、pipeline_version、provider、requested_model、reported_model（可空）、coverage、generated_at、source_job_id。
- `summary_jobs`：job_id、conversation_id、source_session_id、trigger、provider/model 快照、source_revision、pipeline_version、generation、状态、attempt、错误分类/脱敏描述、时间戳、阶段与分块进度。
- `summary_batches` 和 batch/job 关联表：batch_id、cwd、选择策略、计数及完成状态。同一 job 可以被批次复用，不重复启动。
- 快照/分块中间产物持久化：以 job_id 关联规范输入及 chunk 结果，重启重用已成功分块。终态后按明确保留策略清理中间产物，保留结果与诊断元数据；默认保留中间产物 7 天。

任务状态为 `queued → running → succeeded | failed`；取消为 `cancelled`。重试创建新 attempt，保留失败记录。摘要展示状态单独计算为 missing、legacy、current、stale；旧成功摘要与最新失败任务可同时存在，不以一个 status 混合二者。

`source_revision` 为规范化输入内容哈希，包含角色、顺序、正文、工具记录及影响语义的字段，不仅用回合数或时间。相同 revision + pipeline_version + provider/model 的活动任务去重；强制重做也不能并行覆盖同一 conversation，活动时返回原 job。

每条 conversation 使用单调 generation；完成时以事务/CAS 发布，迟到旧 job 不覆盖新 generation。source 已变化时可保留该快照结果但展示 stale，不能标记当前完整。

全局默认并发 1；不同批次公平推进，关闭事件与批量修复复用同一队列。重启时失去 worker 的 running 恢复 queued，并从成功 chunk 后续执行。daemon 停止及取消均终止并回收子进程及其子进程组。

## 5. 总结器选择与 CLI 契约

引入独立 `summary` 配置，至少含 provider、可选 model、单调用超时、重试上限、输入预算。不要改变 Todo 的 `default_agent` 行为。

选择优先级：本次 UI 明确选择 → 已保存 summary 配置 → 有效的旧 default_agent 配置（UI 明示为兼容来源）。均未配置时，UI 提示选择已安装总结器；自动任务记录 `configuration_required`，不静默选择 Claude，也不逐条启动必失败进程。项目修复其他步骤仍可完成，总结步骤明确失败。

修复弹窗提供总结器/model 选择与保存入口；会话详情使用相同组件。未明确指定 model 时展示“CLI 默认模型”，实际模型未知就记录未知，不猜测。配置非法必须报告错误。

适配器覆盖现有 Claude、CodeBuddy、OpenCode、V8agent、Goose、Aider，并补 Codex。实施时核对安装版本帮助及官方文档，记录所测版本；本文不预设尚未验证的 CLI 参数。只有验证过一次性输出与无写工具能力的适配器才可在 UI 标记可用。

总结是纯文本处理：输入 transcript 被当作数据，不执行其中指令，不恢复原会话，不挂可写 MCP，不启用编辑/命令工具。优先采用 CLI 支持的禁工具或只读能力并在隔离临时目录运行；仅提示词不足以保证隔离。不具备可验证隔离能力时明确 `unsupported_capability`，不以跳过权限模式代替。

预检包括可执行文件解析、受支持参数/能力及可确定的本地配置；不能只凭二进制存在声称认证成功。认证失败通过真实调用识别，按 provider/model 熔断本批剩余任务，提供一次明确原因，修复配置后可重试。

执行器必须：

- 从路径解析、spawn、写 stdin 到等待退出均有 deadline；初始单调用 120 秒，可配置。
- 并行排空 stdout/stderr，设置有界输出，处理 stdin 错误及非零退出；超时 kill + wait，不留后台进程。
- 按适配器先提取最终模型文本，再解析结构化 JSON；不从工具 trace 或回显 prompt 误取结果。
- 区分 configuration_required、unsupported_capability、spawn、authentication、rate_limit、timeout、nonzero_exit、invalid_output、input_unavailable、storage_error。
- 瞬态错误最多自动重试 2 次，指数退避；认证/配置/不支持不自动重试。格式错误最多一次纠正调用，受同一预算约束。
- 错误持久化退出码、耗时及脱敏 stderr 尾部；日志不写完整 transcript、认证值或原始环境变量。

## 6. 完整记录与分段汇总

“完整”指覆盖当前可读取 transcript 快照中的全部有效对话内容，不宣称覆盖未记录的操作或模型隐藏思考。

1. 先补齐该 conversation 的摄取，按 turn_index 固定顺序生成不可变快照。保留 human、可见 AI、工具调用/结果及错误；排除隐藏 thinking 和明确的协议噪声。工具信息缺失时标注限制。
2. 按总结器预算切块，预留指令、输出及汇总空间；优先回合边界，超长单回合继续拆段。使用保守 token 估算或对应 tokenizer，不按 16,000 字符直接砍头。
3. 每块提取结构化事实：用户目标、实际行动、决策、结果/验证、未完成项；事实附来源 turn 范围，区分用户要求、AI 自述和工具可验证证据。
4. 所有块成功后按时间合并；后续撤销/失败修正早期结论。中间结果仍超预算则递归归并，记录层级及进度，不丢最早块。
5. 输出简短标题（最多 60 字）及可读摘要（目标最多约 1,200 中文字），重点说明实际结果；结构化 facts 保留证据范围。不得把“用户要求完成”改写成“已完成”，没有验证就写未验证。
6. 校验 title/summary 非空、JSON 字段及证据范围有效。只有所有块和最终合并成功才发布 current。部分失败保留中间结果用于重试，不能发布为完整总结。

每 job 初始最多 64 次模型调用（包括重试/纠正），累计调用时间预算 30 分钟；超过预算显式失败 `budget_exceeded`，保留可重用块，UI 可在用户主动重试时提高预算。不能为了预算静默删除输入。

## 7. 历史兼容与修复选择

默认项目修复处理：无结果、旧 heuristic、空白/占位、旧 pipeline_version、revision 不匹配、最近失败且无当前有效结果。显式单条重新生成始终可用；本次不增加整项目强制覆盖按钮。

旧 AI 摘要没有覆盖版本信息，保留展示但标记 legacy，第一次修复生成新版。多条历史结果优先选择非空 AI，再按 created_ts_ms 和 session_id 稳定排序，不任意覆盖。NULL conversation_id 保留历史诊断，不猜测关联。

失败不写新的 heuristic，不覆盖旧成功摘要。旧拼接仍可作为折叠的“历史用户发言摘录”查看，不标记 LLM 总结。迁移只建表/复制有效旧展示数据，不在启动时自动触发全库付费重算。

MCP `submit_session_summary` 继续接收，但未携带新版覆盖证据时仅作为 legacy，不能越过新任务的版本与 generation 判定。Todo 创建时移除“空摘要=AiGenerated”语义；task 关联独立保留，实际记录完成后入统一队列。

## 8. 协议与 UI

新增 V2 请求/响应，旧客户端接口保留兼容包装：提交单条/批次返回 job_id/batch_id；按 ID 查询进度；重试失败项；取消批次。旧轮询的 0/0 不再用于新版 UI 判断成功。

批次响应至少含 total、queued、running、succeeded、failed、cancelled、skipped 与 phase；保持 `total = queued + running + succeeded + failed + cancelled + skipped`。completed 仅表示终态数量，不能等同 succeeded。批次有失败时状态为 completed_with_errors；请求、数据库和轮询错误单独展示。预检失败的已选任务计 failed，并共享明确错误原因。

会话列表/详情展示：未总结、排队中、正在总结（分块进度）、AI 总结、历史摘录、待更新、生成失败；提供生成/重试/重新生成，展示所用总结器与时间。重做期间继续显示旧摘要及生成状态。

修复项目显示“成功 X / 失败 Y / 跳过 Z”，可展开失败项，重试失败或取消。关闭进度弹窗只隐藏 UI，任务继续；取消必须调用显式取消操作。共享 job 仅在没有其他活动订阅者或自动触发需求时终止，取消某批不会破坏其他批次。

项目切换、UI 重连按 project_id + batch_id/job_id 路由，daemon 重启可查询恢复后的任务。成功发布后刷新对应会话，不能必须再次修复项目才看到结果。新版 app 连接旧 daemon 时显示需要升级，不伪装为已成功。

## 9. 验收标准

- A1：缺失/降级/空白/旧版/过期记录均会被修复选中；当前有效结果跳过；单条强制重做生效。
- A2：关闭、自然退出、Shutdown 均不依赖 PTY prompt/MCP；关闭不等模型，重启可恢复，最终可用 transcript 被摄取。
- A3：受控短会话及超过旧 16,000 字符预算、单条超过 4,000 字符的长会话均覆盖开头/中部/结尾事实；撤销和失败不被误写为完成。
- A4：真实可用 CLI 生成非空总结，记录 provider/版本/覆盖信息；Codex 适配器单独通过真实 smoke。仅 fake CLI 通过不等于 LLM 链路验收。
- A5：认证失败、非零退出、无效 JSON、超时、stdin 失败均可见；无 heuristic 新写入，无旧摘要丢失，无孤儿子进程。
- A6：重复点击、并发关闭/修复、迟到旧结果、运行中 transcript 更新、daemon 中途重启均符合去重和版本规则。
- A7：UI 区分成功与完成，轮询/提交失败不显示 0/0 成功；可重试、取消、关闭后恢复查询。
- A8：迁移保留历史记录及 Todo 关联；多历史行选择确定；MCP 兼容写入不能覆盖新版结果。
- A9：日志无原始敏感内容；模型不能通过总结任务执行 transcript 中的命令或编辑项目。

真实质量验收使用受控 transcript 与事实清单，记录输入 revision、输出及人工判定；不以“含分隔符/能解析 JSON”代替总结质量。若真实 CLI 环境不可用，应明确标记 A4 未完成，不宣称端到端完成。
