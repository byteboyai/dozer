# Codex Hook 注册机制与 Transcript 落盘——Spike 验证记录

> 日期：2026-08-07
> 验证人：Codex CLI `codex-cli 0.146.0`（macOS arm64）
> 关联规格：`docs/superpowers/specs/2026-08-07-dozer-multi-agent-codex-qoder-kilo-design.md` §5
> 关联计划：`docs/superpowers/plans/2026-08-07-dozer-codex-qoder-kilo-adapters.md` Task 4
>
> **状态声明**：本记录基于官方文档核实 + 本机只读环境勘查（`codex --version`、`~/.codex/sessions/` 目录与 rollout 文件结构实读），**尚未跑过真实 hook 探针**（跑探针需要写 `~/.codex/hooks.json` 触发真实 OpenAI API 会话，属破坏性/计费操作，改由人工择机执行）。

## 结论一览

| 验证项 | 假设（spec §5 / 计划 Global Constraints） | 实测 | 一致？ |
|---|---|---|---|
| CLI 可用性 | `codex --version` 可用（`codex-cli 0.146.0`） | 命中，`0.146.0` | ✅ |
| hook 注册机制 | `~/.codex/hooks.json` 补丁，结构 `{"hooks":{"EventName":[{"hooks":[{"type":"command","command":"..."}]}]}}` | **结构经官方文档核实；本机 `~/.codex/hooks.json` 当前不存在**（30 Bytes / 无此文件），待探针实测 | 🔶 待探针 |
| hook payload 字段名 | `session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input` 与 Claude 逐字对齐 | 经官方 hooks 文档核实，字段名与 Claude Code 高度对齐；待探针实测确认 | 🔶 待探针 |
| 事件名词汇 | `SessionStart`/`UserPromptSubmit`/`PreToolUse`/`PostToolUse`/`PermissionRequest`/`Stop`/`SessionEnd` 与 Claude 同族 | 官方文档核实与 Claude 高度对齐；dozer-hook `codex.rs` 翻译表已按 spec §5 实现 | 🔶 待探针 |
| transcript 落盘路径 | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`，**不是**按 cwd 建目录 | **实读确认**：`~/.codex/sessions/2026/04/{16,17,...}/{rollout-*}.jsonl`、`2026/07/06/rollout-...jsonl` 结构存在 | ✅ |
| transcript schema | 与 Claude JSONL 不同（另有 `cwd` 内嵌在 content 里） | **实读确认**：`type` 顶层是 `session_meta`/`event_msg`/`response_item`，单行含嵌套 `payload` 对象，与 Claude 的 `type:user/assistant` 扁平结构**显著不同** | ✅（印证"transcript 解析另开计划"） |

**对计划的影响**：Task 5（Codex 事件名翻译表）+ Task 6（Codex hook 安装器：`settings_path_for("codex")` → `~/.codex/hooks.json`）按计划设计可直接落地，本记录确认的路径/版本/transcript 落盘结构与假设一致。唯一待人工跑探针确认的是 hooks.json 三层 JSON 结构在真实 CLI 里是否触发、以及 payload 字段名与假设是否逐字一致——若探针发现偏差，只需改 `codex.rs::translate_event` 与 `install.rs::settings_path_for` 的局部，不影响其余逻辑。

## 验证步骤（只读部分，已完成）

### Step 1：CLI 可用性
`codex --version` → `codex-cli 0.146.0`（与计划 Global Constraints 所述一致）。

### Step 2：~/.codex 环境勘查（只读）
- `~/.codex/hooks.json` **不存在**。据此：未来 `dozer-hook install codex` 将新建该文件而不是增量合并；且本 spike 需要探针时无需备份现有 hooks.json（Task 4 Step 2 的备份命令是幂等安全的，`cp -n` 不会误删）。
- `~/.codex/auth.json` 存在（已登录），说明跑真实探针会话是有计费风险的——这也是本记录不主动执行探针的原因。
- `~/.codex/sessions/` 目录结构实读确认：`sessions/{YYYY}/{MM}/{DD}/rollout-<uuid>.jsonl`，与计划 Global Constraints 所述完全一致。

### Step 3：transcript schema 实读（脱敏结构）
抓取 `~/.codex/sessions/2026/07/06/rollout-*.jsonl` 前三行，field 名观察：

| 行 | 顶层 `type` | 关键字段 |
|---|---|---|
| 1 | `session_meta` | `payload.id`, `payload.cwd`, `originator`, `cli_version`, `source`, `model_provider`, `base_instruction`… |
| 2 | `event_msg` | `payload.type`("task_started"), `payload.turn_id`, `started_at`, `model_context_window`, `collaboration_mode_kind`… |
| 3 | `response_item` | `payload.type`("message"), `role`("developer"), `content[{type,input_text,text}]`… |

**对比 Claude JSONL**（扁平 `{"type":"user","message":{...}}` / `{"type":"assistant","message":{...}}`）：
- Codex 是**分层 json 对象**（`type` + 嵌套 `payload`），每条 line 的语义靠 `payload.type` 区分，而非顶层 `type` 直给。
- `cwd` 出现在 `session_meta.payload.cwd`（会话级），并非像 Claude 那样按 cwd 建目录、目录即携带 cwd 信息——印证了：要按 cwd 过滤会话，必须扫全量 `sessions/` 再读每份 rollout 里的 cwd 字段，无法用目录路径简化。这彻底做实了计划"本计划不包含 Codex transcript 目录扫描"的降级决定。

## 待人工执行的探针步骤（落地 Task 4 Step 3~6）

以下步骤需真实 OpenAI 会话，由执行者择机手动跑（不改动仓库代码）：

1. 写探针 `~/.codex/hooks.json`（6 个事件：SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/Stop/SessionEnd，command 均为 `sh -c 'printf "\n===<EV>===\n" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'`）。
2. `cd /tmp && rm -f /tmp/codex-hook-probe.log && codex exec "reply with exactly one word: hello"`。
3. `cat /tmp/codex-hook-probe.log` 核对是否命中、字段名是否与假设一致。
4. 若命中且拿到 `transcript_path`，把 rollout 文件脱敏存为 `crates/dozer-hook/fixtures/codex-transcript-sample.jsonl`（作为未来"Codex transcript 解析"计划的输入）。
5. 还原（本机 hooks.json 不存在，直接 `rm -f ~/.codex/hooks.json` 即可；不要留探针）。

## 对后续计划的影响

1. **Task 5/6**：Codex 翻译表与安装器已按计划落地，本记录确认路径/版本/transcript 结构与假设一致，唯一待探针绑定的是 hooks.json 三层结构是否真触发 + payload 字段名逐字一致性。
2. **transcript 解析（未来独立计划）**：本记录实读已确认 Codex transcript 是 `session_meta`/`event_msg`/`response_item` + 嵌套 `payload` 的分层 schema，Claude 解析器（`parse_claude_shaped_jsonl`/`parse_usage`）**无法复用**；且 cwd 内嵌在 `session_meta.payload.cwd`、目录不按 cwd 建——做目录扫描必须全量扫 `sessions/` 再按内容 cwd 过滤。这些结论足够未来解析计划直接立项，不必等探针。
