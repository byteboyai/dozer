# Codex Agent 卡片模型显示 + 用量统计 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agent 面板里的 Codex 会话卡片要能显示当前模型名,「用量」面板要能统计到 Codex 的 token 用量。

**Architecture:** 分两条现有但对 Codex 关闭的管线各开一道门:1) `dozer-app` 侧对 agent 卡片做的"实时读 transcript 文件取 model/mode/activity"(`transcript::latest_model_mode_and_activity`),补上 Codex 的行形状识别;2) `dozerd` 侧的会话摄取管线(`parse_chunk`)补一个真正的 Codex 解析器,把 Codex rollout JSONL 的人类/AI 文本行 + 用量行摄取成 `ParsedTurn`,供「用量」面板的聚合查询消费。两条管线本来就互相独立(前者不碰数据库,后者不碰实时文件),这次各自新增 Codex 分支,不改动其余 agent 的既有行为。

**Tech Stack:** Rust workspace(`dozer-core`/`dozerd`/`dozer-app`),`serde_json` 手动解析(不引入新 crate),SQLite(`rusqlite`)。

**Spec:** 没有独立 spec 文档(用户明确要求跳过 brainstorming 直接写 plan)。背景依据是已有的 spike 记录 `docs/superpowers/specs/2026-08-07-codex-spike-findings.md`(确认了 transcript 落盘路径与"分层 payload"结构),以及 `docs/superpowers/plans/2026-08-20-dozer-conversation-ingestion.md` Global Constraints 里"Codex 维持解析返回空,不在本计划内补齐真实 schema"这条历史遗留的显式排除项——本计划就是补上它。本计划涉及的具体字段路径(`turn_context.payload.model`、`token_usage_record.payload.usage.*`、`response_item.payload.{role,content,id}` 等)均在 2026-09-21 通过读取本机 `~/.codex/sessions/**/rollout-*.jsonl` 的真实历史会话文件核实(只统计结构/字段名,未摘录对话原文),不是猜测。

## Global Constraints

- **不引入新依赖**(不加 `chrono`/`time` 解析 Codex 的 ISO-8601 时间戳——`dozer-app` 的 `usage/aggregate.rs::civil_from_days` 已经是"手写日期数学、不加日期库"的先例,`dozerd` 侧新增的 `days_from_civil`/时间戳解析同样手写)。
- **v1 范围只覆盖"人类/AI 文本 + 模型名 + token 用量"**,不解析 Codex 的工具调用结构(`custom_tool_call`/`custom_tool_call_output`)、思考文本(`reasoning`)。这些行在摄取时直接跳过(`_ => {}`),跟 Claude 侧对 `file-history-snapshot` 等未知行类型的既有策略一致。后果:Codex 会话在「审阅」面板里能看到人类发言和 AI 回复文本,但看不到结构化的工具调用轨迹摘要——这是可接受的范围收窄,不是遗留 bug,因为本次要解决的问题(模型显示 + 用量统计)不依赖它。
- **Codex 的 `usage`/`turn_token_usage`/`thread_token_usage` 三个用量对象语义不同**(实测核实,见 Task 2 注释):`usage` 是"这一次模型 API 调用自己的增量花费","turn_token_usage"/"thread_token_usage" 是"从会话/回合开始累计到现在的总量"。摄取时必须用 `usage`(增量),不能用后两者,否则对同一会话内的多次 API 调用重复计入指数级偏高的合计。
- **Codex 的 `input_tokens` 已经把 `cached_input_tokens` 算在内**(OpenAI Responses API 语义,与 Claude Messages API 的"`input_tokens`/`cache_read_input_tokens` 互斥、需要相加"语义相反,实测样本核实:`input_tokens - cached_input_tokens + output_tokens == total_tokens` 恒成立)。摄取到 `ParsedTurn`/`UsagePayload` 时必须做 `input_tokens - cached_input_tokens` 才符合 `dozer-app usage/aggregate.rs` 现有的"四个 token 字段直接相加"展示公式,否则用量面板的 Codex 数字会比实际偏高(缓存命中部分被重复计入)。
- **不清理"孤儿行"**,不做实时流式展示——沿用 2026-08-20 那份摄取计划的既有约束,这次新增的 Codex 分支不例外。
- 每个任务完成后运行:`cargo build -p <改动的 crate>`、`cargo test -p <改动的 crate>`、`cargo clippy -p <改动的 crate> --all-targets -- -D warnings`、`cargo fmt -- --check`。

---

## Task 1: Agent 卡片显示 Codex 模型名 + 当前工作内容

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs`(`latest_model_mode_and_activity`、`extract_line_activity`)
- Modify: `crates/dozer-app/src/workspace/state.rs`(`agent_card_refresh_plan`,约 2538-2575 行)
- Modify: `crates/dozer-app/src/workspace/tests.rs`(约 122-131 行,现有的 Codex 断言)

**Interfaces:**
- Consumes: 无新依赖,复用 `crates/dozer-app/src/transcript.rs` 已有的 `join_text_blocks(blocks: &[Value], kind: &str) -> Option<String>` 私有函数。
- Produces: `latest_model_mode_and_activity` 对 Codex 形状的 transcript 返回真实 `(model, None, activity)`(mode 恒 `None`,Codex 没有 permissionMode 等价字段);`agent_card_refresh_plan(AgentKind::Codex, ..)` 不再恒返回 `(false, false, ..)`。

- [ ] **Step 1: 写失败测试——`latest_model_mode_and_activity` 识别 Codex 形状**

在 `crates/dozer-app/src/transcript.rs` 的 `#[cfg(test)] mod tests` 里,紧跟在 `latest_model_and_mode_reads_codebuddy_provider_data_model` 测试之后新增:

```rust
    #[test]
    fn latest_model_and_mode_reads_codex_turn_context_model() {
        // Codex 形状:model 不在消息行上,而是独立的 `turn_context` 行,
        // 顶层 `type`,嵌套在 `payload.model`——跟 Claude(`message.model`)/
        // Codebuddy(顶层 `providerData.model`)都不同。字段路径核对自
        // 2026-09-21 实读本机 `~/.codex/sessions/**/rollout-*.jsonl`。
        // Codex 没有 permissionMode 等价字段,mode 应保持 None。
        let jsonl = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"openai\"}}\n",
            "{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-6-astra\"}}\n",
        );
        let (model, mode, _activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(mode, None);
    }

    #[test]
    fn activity_reads_codex_shaped_last_human_message_and_skips_developer_role() {
        // Codex 的 `response_item`/`payload.type:"message"` 里,`role` 除了
        // user/assistant 还有 "developer"(CLI 自己注入的系统提示片段,
        // 等价于 Claude 的 isMeta 消息)——activity 只认 role:"user",
        // "developer" 即使排在后面也不该盖掉真实用户发言。
        let jsonl = concat!(
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",",
            "\"content\":[{\"type\":\"input_text\",\"text\":\"修一下光标问题\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",",
            "\"content\":[{\"type\":\"input_text\",\"text\":\"系统提示词片段\"}]}}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(activity.as_deref(), Some("修一下光标问题"));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app --lib transcript::tests::latest_model_and_mode_reads_codex_turn_context_model transcript::tests::activity_reads_codex_shaped_last_human_message_and_skips_developer_role`
Expected: 两个测试都 FAIL(`model`/`activity` 提取到 `None`,因为还没实现 Codex 分支)。

- [ ] **Step 3: 实现 Codex 形状识别**

在 `crates/dozer-app/src/transcript.rs` 里,把 `latest_model_mode_and_activity` 函数体(158-191 行)里 model 提取那一段:

```rust
        let claude_model = v
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|s| s.as_str());
        let codebuddy_model = v
            .get("providerData")
            .and_then(|p| p.get("model"))
            .and_then(|s| s.as_str());
        if let Some(m) = claude_model.or(codebuddy_model) {
            model = Some(m.to_string());
        }
```

改成:

```rust
        let claude_model = v
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|s| s.as_str());
        let codebuddy_model = v
            .get("providerData")
            .and_then(|p| p.get("model"))
            .and_then(|s| s.as_str());
        // Codex 形状:model 不挂在消息行上,是独立的 `turn_context` 行
        // (顶层 `type:"turn_context"`),嵌套在 `payload.model`。
        let codex_model = (v.get("type").and_then(|t| t.as_str()) == Some("turn_context"))
            .then(|| v.get("payload").and_then(|p| p.get("model")))
            .flatten()
            .and_then(|s| s.as_str());
        if let Some(m) = claude_model.or(codebuddy_model).or(codex_model) {
            model = Some(m.to_string());
        }
```

再把函数顶部的文档注释(135-146 行)里"model 兼认两种互斥的行形状"改成"model 兼认三种互斥的行形状",补一句 Codex 的路径说明(照抄 Claude/Codebuddy 那两句的写法,不用逐字给出,保持文档准确即可)。

然后把 `extract_line_activity` 函数(199-216 行):

```rust
fn extract_line_activity(v: &Value) -> Option<String> {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("user") => v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Some("message") => {
            let role = v.get("role").and_then(|r| r.as_str())?;
            if role != "user" {
                return None;
            }
            let blocks = v.get("content").and_then(|c| c.as_array())?;
            join_text_blocks(blocks, "input_text")
        }
        _ => None,
    }
}
```

改成:

```rust
fn extract_line_activity(v: &Value) -> Option<String> {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("user") => v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Some("message") => {
            let role = v.get("role").and_then(|r| r.as_str())?;
            if role != "user" {
                return None;
            }
            let blocks = v.get("content").and_then(|c| c.as_array())?;
            join_text_blocks(blocks, "input_text")
        }
        // Codex 形状:`response_item`/`payload.type:"message"`,人类发言
        // 是 `payload.role:"user"`(还有 "developer"/"assistant" 两种角色,
        // "developer" 是 CLI 自己注入的系统提示片段,不算真实用户发言,
        // 不提取)。
        Some("response_item") => {
            let payload = v.get("payload")?;
            if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
                return None;
            }
            if payload.get("role").and_then(|r| r.as_str()) != Some("user") {
                return None;
            }
            let blocks = payload.get("content").and_then(|c| c.as_array())?;
            join_text_blocks(blocks, "input_text")
        }
        _ => None,
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app --lib transcript::tests`
Expected: 全部 PASS,包括新增的两个和已有的全部旧测试(不应该有回归)。

- [ ] **Step 5: 打开 `agent_card_refresh_plan` 对 Codex 的门禁**

在 `crates/dozer-app/src/workspace/state.rs` 里,把 `agent_card_refresh_plan` 函数(2538-2575 行)的这部分:

```rust
    let needs_model_mode = matches!(
        agent,
        AgentKind::Claude | AgentKind::Unknown | AgentKind::Codebuddy | AgentKind::Opencode
    );
```

改成:

```rust
    let needs_model_mode = matches!(
        agent,
        AgentKind::Claude
            | AgentKind::Unknown
            | AgentKind::Codebuddy
            | AgentKind::Opencode
            | AgentKind::Codex
    );
```

以及:

```rust
    let needs_activity = !matches!(agent, AgentKind::Codex);
```

改成:

```rust
    let needs_activity = true;
```

同时更新这两行上方那段大注释(2543-2555 行、2560-2566 行)——把"Codex 目前 `parse_transcript` 恒回空,读了也提取不出东西,不值得为它打开这道门"这句删掉或改成"Codex 现在也能从 `turn_context`/`response_item` 行里提取到 model/activity,见 `transcript::latest_model_mode_and_activity`",避免注释和代码脱节误导下一个读者。

- [ ] **Step 6: 更新 `workspace/tests.rs` 里过期的 Codex 断言**

在 `crates/dozer-app/src/workspace/tests.rs` 里,把:

```rust
    assert_eq!(
        agent_card_refresh_plan(dozer_core::protocol::AgentKind::Codex, &root, Some(&root)),
        (false, false, false),
        "Codex 的 transcript 恒解不出内容(parse_transcript 空 Vec),\
             model/mode/activity 都不值得读"
    );
```

改成:

```rust
    assert_eq!(
        agent_card_refresh_plan(dozer_core::protocol::AgentKind::Codex, &root, Some(&root)),
        (true, true, false),
        "Codex 现在也能从 turn_context/response_item 行提取 model/activity\
             (mode 恒 None——Codex 没有 permissionMode 等价字段)"
    );
```

- [ ] **Step 7: 运行 workspace 测试确认通过**

Run: `cargo test -p dozer-app --lib workspace::tests`
Expected: 全部 PASS。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/transcript.rs crates/dozer-app/src/workspace/state.rs crates/dozer-app/src/workspace/tests.rs
git commit -m "$(cat <<'EOF'
feat(agent-card): extract Codex model/activity from turn_context and response_item lines

Codex 的 transcript 是分层 payload 结构,跟 Claude/CodeBuddy 都不同——model 在
独立的 turn_context 行里,人类发言在 response_item/payload.type:message 里,
之前 agent_card_refresh_plan 直接把 Codex 排除在门禁外,导致 agent 卡片永远
显示不出 Codex 的模型名和当前工作内容。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: dozerd 摄取 Codex transcript(人类/AI 文本 + token 用量)

**Files:**
- Modify: `crates/dozerd/src/transcripts/parse.rs`
- Modify: `crates/dozer-hook/fixtures/codex-transcript-sample.jsonl`(把 spike 时代的 3 行占位样本换成能覆盖 model/developer 跳过/usage 的完整样本)

**Interfaces:**
- Consumes: `dozer_core::protocol::AgentKind::Codex`,已有的 `ParsedTurn`/`fallback_key`/`last_complete_line_boundary`。
- Produces: `parse_chunk(AgentKind::Codex, ..)` 不再恒回空 `Vec`,返回 role 为 `"human"`/`"ai"`/`"token_usage"` 的 `ParsedTurn`(`"token_usage"` 是本计划新引入的第三种角色,专门承载 Codex 用量行,不含正文——Task 3 会在审阅时间线渲染那边补一道"跳过这个角色"的处理,避免空气泡)。

- [ ] **Step 1: 用真实字段路径重写 fixture**

用 Write 工具把 `crates/dozer-hook/fixtures/codex-transcript-sample.jsonl` 整个替换成(每行一个 JSON 对象,`demo-`/`msg-demo-`/`resp-demo-` 前缀延续原文件"手写合成样本,不是真实抓取"的既有约定;token 数字取自 2026-09-21 实读本机真实 rollout 文件的样本值):

```
{"timestamp":"2026-09-21T02:50:44.000Z","type":"session_meta","payload":{"id":"demo-0000-0000-0000-000000000001","timestamp":"2026-09-21T02:50:44.000Z","cwd":"/private/tmp","originator":"codex_cli","cli_version":"0.146.0","source":"exec","thread_source":"user","model_provider":"openai"}}
{"timestamp":"2026-09-21T02:50:44.010Z","type":"event_msg","payload":{"type":"task_started","turn_id":"demo-0000-0000-0000-000000000002","started_at":1789959044,"model_context_window":258400,"collaboration_mode_kind":"default"}}
{"timestamp":"2026-09-21T02:50:44.020Z","type":"response_item","payload":{"type":"message","id":"msg-demo-0000-0000-0000-000000000003","role":"developer","content":[{"type":"input_text","text":"system prompt boilerplate"}]}}
{"timestamp":"2026-09-21T02:50:44.030Z","type":"response_item","payload":{"type":"message","id":"msg-demo-0000-0000-0000-000000000004","role":"user","content":[{"type":"input_text","text":"reply with exactly one word: hello"}]}}
{"timestamp":"2026-09-21T02:50:44.040Z","type":"turn_context","payload":{"turn_id":"demo-0000-0000-0000-000000000002","cwd":"/private/tmp","model":"gpt-6-astra","collaboration_mode":{"mode":"default","settings":{"model":"gpt-6-astra"}}}}
{"timestamp":"2026-09-21T02:50:48.000Z","type":"response_item","payload":{"type":"message","id":"msg-demo-0000-0000-0000-000000000005","role":"assistant","content":[{"type":"output_text","text":"hello"}]}}
{"timestamp":"2026-09-21T02:50:48.010Z","type":"token_usage_record","payload":{"response_id":"resp-demo-0000-0000-0000-000000000006","session_id":"demo-0000-0000-0000-000000000001","turn_id":"demo-0000-0000-0000-000000000002","usage":{"input_tokens":14768,"cached_input_tokens":12160,"cache_write_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":0,"total_tokens":14773},"turn_token_usage":{"input_tokens":14768,"cached_input_tokens":12160,"cache_write_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":0,"total_tokens":14773},"thread_token_usage":{"input_tokens":14768,"cached_input_tokens":12160,"cache_write_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":0,"total_tokens":14773}}}
{"timestamp":"2026-09-21T02:50:48.020Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"demo-0000-0000-0000-000000000002","started_at":1789959044,"completed_at":1789959048,"duration_ms":4000,"time_to_first_token_ms":1200,"last_agent_message":"hello"}}
```

- [ ] **Step 2: 写失败测试——主流程 + 用量减法 + developer 跳过**

在 `crates/dozerd/src/transcripts/parse.rs` 的 `#[cfg(test)] mod tests` 里,紧跟在 `codebuddy_parses_real_fixture_sample_with_usage` 测试之后新增:

```rust
    #[test]
    fn codex_parses_real_shaped_fixture_sample_with_usage() {
        let text = include_str!("../../../dozer-hook/fixtures/codex-transcript-sample.jsonl");
        let turns = parse_chunk(AgentKind::Codex, text, "conv1", 0);
        assert_eq!(
            turns.len(),
            3,
            "1 用户消息 + 1 assistant 回复 + 1 用量行;developer/session_meta/\
             turn_context/event_msg 等不摄取"
        );
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "reply with exactly one word: hello");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "hello");
        assert_eq!(turns[2].role, "token_usage");
        assert_eq!(turns[2].content, "");
        // input_tokens(14768)已经把 cached_input_tokens(12160)算在内
        // (OpenAI Responses API 语义,跟 Claude 相反),摄取时要减掉缓存
        // 命中部分,不然会跟 tokens_cache_read 重复计入。
        assert_eq!(turns[2].tokens_in, 14768 - 12160);
        assert_eq!(turns[2].tokens_out, 5);
        assert_eq!(turns[2].tokens_cache_read, 12160);
        assert_eq!(turns[2].tokens_cache_write, 0);
        assert!(turns[0].ts.is_some(), "timestamp 字符串应该被解析成毫秒");
        assert_ne!(turns[0].message_key, "conv1:0", "应该取 payload.id,不退化成 fallback_key");
    }

    #[test]
    fn codex_sums_usage_across_multiple_api_calls_within_one_turn() {
        // 一次逻辑回合内可能有好几次模型 API 调用(工具调用往返),每次都
        // 各自产出一条 token_usage_record;`usage` 字段是每次调用自己的
        // 增量花费,不是累计值——两条用量行应该各自摘出一条 ParsedTurn,
        // 求和交给 dozerd 的 SUM 查询,这里只验证"没有被错误合并/覆盖"。
        let text = concat!(
            "{\"timestamp\":\"2026-09-21T00:00:00.000Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"id\":\"m1\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"第一步\"}]}}\n",
            "{\"timestamp\":\"2026-09-21T00:00:01.000Z\",\"type\":\"token_usage_record\",",
            "\"payload\":{\"response_id\":\"r1\",\"usage\":{\"input_tokens\":100,",
            "\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":10,",
            "\"reasoning_output_tokens\":0,\"total_tokens\":110}}}\n",
            "{\"timestamp\":\"2026-09-21T00:00:02.000Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"id\":\"m2\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"第二步\"}]}}\n",
            "{\"timestamp\":\"2026-09-21T00:00:03.000Z\",\"type\":\"token_usage_record\",",
            "\"payload\":{\"response_id\":\"r2\",\"usage\":{\"input_tokens\":50,",
            "\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":5,",
            "\"reasoning_output_tokens\":0,\"total_tokens\":55}}}\n",
        );
        let turns = parse_chunk(AgentKind::Codex, text, "conv1", 0);
        assert_eq!(turns.len(), 4);
        let usage_turns: Vec<_> = turns.iter().filter(|t| t.role == "token_usage").collect();
        assert_eq!(usage_turns.len(), 2);
        assert_eq!(usage_turns[0].tokens_in, 100);
        assert_eq!(usage_turns[1].tokens_in, 50);
        assert_ne!(
            usage_turns[0].message_key, usage_turns[1].message_key,
            "两次调用各有独立 response_id,不能共用 message_key 互相覆盖"
        );
    }

    #[test]
    fn codex_timestamp_parses_iso8601_to_epoch_millis() {
        let text = concat!(
            "{\"timestamp\":\"1970-01-01T00:00:00.000Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"id\":\"m1\",\"role\":\"user\",",
            "\"content\":[{\"type\":\"input_text\",\"text\":\"hi\"}]}}\n",
        );
        let turns = parse_chunk(AgentKind::Codex, text, "conv1", 0);
        assert_eq!(turns[0].ts, Some(0));
    }
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p dozerd --lib transcripts::parse::tests::codex_`
Expected: 三个新测试全部 FAIL(`parse_chunk(AgentKind::Codex, ..)` 现在还是恒返回空 `Vec`)。

- [ ] **Step 4: 把 `join_codebuddy_text_blocks` 改名成通用名字**

Codex 的 `content` 数组形状(`[{"type":"input_text"/"output_text","text":...}]`)跟 Codebuddy 完全一样,这个函数本来就没有 Codebuddy 专属逻辑,改名复用,不重复写一份。在 `crates/dozerd/src/transcripts/parse.rs` 里把:

```rust
fn join_codebuddy_text_blocks(blocks: &[Value], kind: &str) -> String {
```

改成:

```rust
fn join_text_blocks_by_kind(blocks: &[Value], kind: &str) -> String {
```

同时把文件里唯一的调用点(`parse_codebuddy_shaped_chunk` 内部)从 `join_codebuddy_text_blocks(blocks, "input_text")`/`join_codebuddy_text_blocks(blocks, "output_text")` 改成 `join_text_blocks_by_kind(...)`(函数体不变)。

- [ ] **Step 5: 新增时间戳解析 + Codex 解析函数**

在 `parse_codebuddy_shaped_chunk` 函数结束之后、`fn parse_chunk` 之前,新增:

```rust
/// Howard Hinnant 的公开 days_from_civil 算法——`dozer-app` 的
/// `usage/aggregate.rs::civil_from_days` 是它的逆运算(那边是"天数→年月日"
/// 给图表标签用,这边是"年月日→天数"给时间戳解析用),两边各自私有实现,
/// `dozerd`/`dozer-app` 之间没有共享的日期工具模块,不做跨 crate 复用。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = ((m as i64 + 9) % 12) as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Codex 的 timestamp 字段(如 `"2026-09-21T02:50:44.008Z"`)→ epoch 毫秒。
/// 定长解析,不引入 chrono/time(见 Global Constraints)。格式跟实测样本
/// (`~/.codex/sessions/**/rollout-*.jsonl`)完全一致:固定 UTC、毫秒精度、
/// `Z` 结尾;只要有一处不匹配就整体返回 `None`,不做宽松容错——时间戳解析
/// 失败只影响这一行的排序展示,不值得为极端形状维护正则级别的解析器。
fn parse_codex_timestamp_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 24
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[23] != b'Z'
    {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: u64 = s.get(11..13)?.parse().ok()?;
    let minute: u64 = s.get(14..16)?.parse().ok()?;
    let second: u64 = s.get(17..19)?.parse().ok()?;
    let millis: u64 = s.get(20..23)?.parse().ok()?;
    let days = days_from_civil(year, month, day);
    let day_ms = u64::try_from(days).ok()?.checked_mul(86_400_000)?;
    let time_ms = hour * 3_600_000 + minute * 60_000 + second * 1_000 + millis;
    Some(day_ms + time_ms)
}

/// Codex rollout transcript(`session_meta`/`event_msg`/`response_item`/
/// `turn_context`/`token_usage_record`,顶层 `type` + 嵌套 `payload`,跟
/// Claude/Codebuddy 的扁平结构完全不同,见 spike 记录
/// `docs/superpowers/specs/2026-08-07-codex-spike-findings.md`)→
/// `ParsedTurn`。v1 范围只摘"人类/AI 文本"(`response_item`/
/// `payload.type:"message"`,role user/assistant,developer 角色是 CLI 注入
/// 的系统提示片段,跳过)和"token 用量"(`token_usage_record`,产出一条独立
/// 的 `role:"token_usage"` 行,不含正文)。`reasoning`/`custom_tool_call`/
/// `custom_tool_call_output`/`session_meta`/`event_msg`/`world_state`/
/// `turn_context` 等行本计划不解析,原样跳过(同 Claude 侧对未知行类型的
/// 既有策略)。
fn parse_codex_shaped_chunk(
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    let mut out = Vec::new();
    let mut turn_index = starting_turn_index;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let top_type = v.get("type").and_then(|t| t.as_str());
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_codex_timestamp_ms);
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match top_type {
            Some("response_item")
                if payload.get("type").and_then(|t| t.as_str()) == Some("message") =>
            {
                let role = payload.get("role").and_then(|r| r.as_str());
                let Some(blocks) = payload.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                let message_key = payload
                    .get("id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
                match role {
                    Some("user") => {
                        let content = join_text_blocks_by_kind(blocks, "input_text");
                        if content.is_empty() {
                            continue;
                        }
                        out.push(ParsedTurn {
                            message_key,
                            role: "human".into(),
                            content,
                            ts,
                            raw_json: line.to_string(),
                            ..Default::default()
                        });
                        turn_index += 1;
                    }
                    Some("assistant") => {
                        out.push(ParsedTurn {
                            message_key,
                            role: "ai".into(),
                            content: join_text_blocks_by_kind(blocks, "output_text"),
                            ts,
                            raw_json: line.to_string(),
                            ..Default::default()
                        });
                        turn_index += 1;
                    }
                    // "developer" 是 Codex CLI 自己注入的系统提示片段(等价
                    // 于 Claude 的 isMeta 消息),不是真实用户发言,不摄取。
                    _ => {}
                }
            }
            Some("token_usage_record") => {
                let Some(usage) = payload.get("usage") else {
                    continue;
                };
                let field = |k: &str| usage.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
                let input_tokens = field("input_tokens");
                let cached_input_tokens = field("cached_input_tokens");
                let message_key = payload
                    .get("response_id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
                out.push(ParsedTurn {
                    message_key,
                    role: "token_usage".into(),
                    // input_tokens 已经把 cached_input_tokens 算在内(见
                    // Global Constraints),减掉才符合 tokens_in/
                    // tokens_cache_read 互不重叠、直接相加的既有展示公式。
                    tokens_in: input_tokens.saturating_sub(cached_input_tokens),
                    tokens_out: field("output_tokens"),
                    tokens_cache_read: cached_input_tokens,
                    tokens_cache_write: field("cache_write_input_tokens"),
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            _ => {}
        }
    }
    out
}
```

- [ ] **Step 6: 接入 `parse_chunk` 分派**

把:

```rust
        AgentKind::Codex => Vec::new(),
```

改成:

```rust
        AgentKind::Codex => {
            parse_codex_shaped_chunk(text, conversation_id, starting_turn_index)
        }
```

同时把 `parse_chunk` 函数上方的文档注释里跟"Codex 恒返回空"相关的过期表述删掉/更新(如果有的话;当前版本这段注释只讲分派逻辑本身,不涉及 Codex,可以不改)。

- [ ] **Step 7: 更新 `extract_turn_trace_detail` 里过期的 Codex 注释**

把:

```rust
        // Codex 目前完全不摄取(parse_chunk 分派到空 Vec),没有 raw_json
        // 可读。
        AgentKind::Codex => TurnTraceDetail::default(),
```

改成:

```rust
        // Codex 现在摄取人类/AI 文本 + 用量(见 parse_codex_shaped_chunk),
        // 但结构化工具调用/思考文本不在 v1 范围内,读时补全维持全空
        // ——跟"有 raw_json 但选择不解析"是两回事,不是没有数据可读。
        AgentKind::Codex => TurnTraceDetail::default(),
```

- [ ] **Step 8: 运行测试确认通过**

Run: `cargo test -p dozerd --lib transcripts::parse`
Expected: 全部 PASS,包括新增的三个测试和已有的全部旧测试(Codebuddy 相关测试因为函数改名要保证还能编译通过、断言不变)。

- [ ] **Step 9: 跑 clippy + fmt**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 无警告、无格式差异。

- [ ] **Step 10: Commit**

```bash
git add crates/dozerd/src/transcripts/parse.rs crates/dozer-hook/fixtures/codex-transcript-sample.jsonl
git commit -m "$(cat <<'EOF'
feat(dozerd): ingest Codex transcript turns and token usage

Codex 的 rollout JSONL 是分层 payload 结构(session_meta/event_msg/
response_item/turn_context/token_usage_record),跟 Claude/CodeBuddy 的扁平
结构完全不同,之前 parse_chunk 对 Codex 恒返回空 Vec,用量面板永远统计不到
Codex 的数据。这次只摄取 v1 需要的三样:人类文本、AI 文本、token 用量
(用量行独立成 role:"token_usage",不含正文,配合减去 OpenAI Responses API
里已经被 cached_input_tokens 重复计入 input_tokens 的那部分)。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 用量面板收录 Codex + 审阅时间线跳过用量行

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage/aggregate.rs`(`AGENT_ORDER`)
- Modify: `crates/dozer-app/src/extensions/usage/mod.rs`(两个断言"Codex 应该被忽略"的旧测试)
- Modify: `crates/dozer-app/src/transcript.rs`(`review_entries_from_turns`,跳过 `role:"token_usage"` 的行)

**Interfaces:**
- Consumes: Task 2 产出的 `role:"token_usage"` 的 `TurnRecord`/`ParsedTurn`(字段形状不变,`tokens_in`/`tokens_out`/`tokens_cache_read`/`tokens_cache_write` 四个已有字段)。
- Produces: `agents_present`/`agent_token_share`/`daily_totals_by_agent` 等所有基于 `AGENT_ORDER` 的函数开始把 Codex 纳入统计;`review_entries_from_turns` 不再把 `role:"token_usage"` 的行渲染成空的 `AiTurn` 气泡。

- [ ] **Step 1: 写失败测试——审阅时间线跳过用量行**

在 `crates/dozer-app/src/transcript.rs` 的 `#[cfg(test)] mod tests` 里,紧跟在 `review_entries_from_turns_keeps_orphan_tool_result_at_top_level` 测试之后新增:

```rust
    #[test]
    fn review_entries_from_turns_skips_token_usage_rows() {
        use dozer_core::protocol::TurnRecord;
        // Codex 的用量行(见 dozerd parse.rs::parse_codex_shaped_chunk)不是
        // 给人看的内容,只用于 dozerd 那边的 token 求和——审阅时间线必须
        // 跳过它,否则会插入一堆没有正文的空 AiTurn 气泡(role 既不是
        // "human" 也不是 "tool_result" 时,既有的兜底分支会把任何角色都
        // 当成 ai 文本处理)。
        let turns = vec![
            TurnRecord {
                turn_index: 0,
                role: "human".into(),
                content: "你好".into(),
                ..Default::default()
            },
            TurnRecord {
                turn_index: 1,
                role: "ai".into(),
                content: "回复".into(),
                ..Default::default()
            },
            TurnRecord {
                turn_index: 2,
                role: "token_usage".into(),
                content: String::new(),
                tokens_in: 100,
                tokens_out: 20,
                ..Default::default()
            },
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(
            entries.len(),
            2,
            "用量行不应该产生第三个条目(也不该被折叠进 AiTurn 覆盖掉正文)"
        );
        match &entries[1] {
            ReviewEntry::AiTurn { text, .. } => assert_eq!(text, "回复"),
            other => panic!("{other:?}"),
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app --lib transcript::tests::review_entries_from_turns_skips_token_usage_rows`
Expected: FAIL(`entries.len()` 会是 3,因为 `token_usage` 行会走兜底分支被当成一个新的空 `AiTurn`)。

- [ ] **Step 3: 在 `review_entries_from_turns` 里加跳过分支**

在 `crates/dozer-app/src/transcript.rs` 里,把 `review_entries_from_turns` 函数(62-133 行)里的:

```rust
    for t in turns {
        match t.role.as_str() {
            "human" => out.push(ReviewEntry::Human {
                text: t.content.clone(),
            }),
            "tool_result" => {
```

改成:

```rust
    for t in turns {
        match t.role.as_str() {
            "human" => out.push(ReviewEntry::Human {
                text: t.content.clone(),
            }),
            // Codex 的用量记录(见 dozerd parse.rs::parse_codex_shaped_chunk)
            // 没有正文,只用于用量求和,不该出现在时间线里。
            "token_usage" => {}
            "tool_result" => {
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app --lib transcript::tests`
Expected: 全部 PASS。

- [ ] **Step 5: 写失败测试——`AGENT_ORDER` 收录 Codex**

在 `crates/dozer-app/src/extensions/usage/mod.rs` 里,先把这两个"Codex 应该被忽略"的旧测试改掉(它们的前提在这个任务之后不再成立,改成用 `Kilo` 验证"确实还没接入的 agent 仍然被忽略"——`Kilo` 不在本计划范围内,继续保持既有的"解析返回空"降级行为)。

把:

```rust
    #[test]
    fn daily_totals_ignores_agents_without_dedicated_bucket() {
        // Codex/Kilo 目前不产出可统计的用量数据（见计划 Global
        // Constraints），跟 Unknown 一样被忽略，不能 panic,也不产生
        // 任何一天的记录(没有任何可展示的 agent,图表应该整体不渲染)。
        let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(99))];
        let days = daily_totals_by_agent(&rows);
        assert!(days.is_empty(), "没有任何已知 agent 有数据时不该产出天记录");
    }
```

改成:

```rust
    #[test]
    fn daily_totals_ignores_agents_without_dedicated_bucket() {
        // Kilo 目前仍不产出可统计的用量数据(2026-09-21 起 Codex 已经被
        // AGENT_ORDER 收录,不再属于这一类——见 daily_totals_by_agent_
        // includes_codex),跟 Unknown 一样被忽略,不能 panic,也不产生
        // 任何一天的记录(没有任何可展示的 agent,图表应该整体不渲染)。
        let rows = vec![(meta_at(AgentKind::Kilo, 0), usage_with_tokens(99))];
        let days = daily_totals_by_agent(&rows);
        assert!(days.is_empty(), "没有任何已知 agent 有数据时不该产出天记录");
    }

    #[test]
    fn daily_totals_by_agent_includes_codex() {
        let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(3))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(total_for(&days[0], AgentKind::Codex), 3);
    }
```

再把:

```rust
    #[test]
    fn agents_present_ignores_agents_without_dedicated_bucket() {
        let rows = vec![(meta(AgentKind::Codex, "a"), usage_with_tokens(1))];
        assert_eq!(agents_present(&rows), Vec::new());
    }
```

改成:

```rust
    #[test]
    fn agents_present_ignores_agents_without_dedicated_bucket() {
        let rows = vec![(meta(AgentKind::Kilo, "a"), usage_with_tokens(1))];
        assert_eq!(agents_present(&rows), Vec::new());
    }

    #[test]
    fn agents_present_includes_codex() {
        let rows = vec![(meta(AgentKind::Codex, "a"), usage_with_tokens(1))];
        assert_eq!(agents_present(&rows), vec![AgentKind::Codex]);
    }
```

- [ ] **Step 6: 运行测试确认失败**

Run: `cargo test -p dozer-app --lib extensions::usage::mod::tests::daily_totals_by_agent_includes_codex extensions::usage::mod::tests::agents_present_includes_codex`
Expected: 两个新测试 FAIL(`AGENT_ORDER` 还没收录 `AgentKind::Codex`,`agents_present`/`daily_totals_by_agent` 对 Codex 恒空)。

- [ ] **Step 7: 把 Codex 加进 `AGENT_ORDER`**

在 `crates/dozer-app/src/extensions/usage/aggregate.rs` 里把:

```rust
pub(crate) const AGENT_ORDER: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::V8agent,
];
```

改成:

```rust
pub(crate) const AGENT_ORDER: [AgentKind; 5] = [
    AgentKind::Claude,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::V8agent,
    AgentKind::Codex,
];
```

同时把这个常量上方的文档注释(267-269 行)"不能像 Codex/Kilo 那样被漏掉"改成"不能像 Kilo 那样被漏掉"(Codex 从这次改动起不再是反面例子),`aggregate.rs` 里另一处同款措辞(380 行附近 `agents_present` 上方注释)一并改掉。

- [ ] **Step 8: 运行测试确认通过**

Run: `cargo test -p dozer-app --lib extensions::usage`
Expected: 全部 PASS,包括改掉的两个旧测试和新增的两个测试。

- [ ] **Step 9: 全量跑一遍 dozer-app 测试 + clippy + fmt**

Run: `cargo test -p dozer-app --lib && cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全部 PASS,无警告,无格式差异。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions/usage/aggregate.rs crates/dozer-app/src/extensions/usage/mod.rs crates/dozer-app/src/transcript.rs
git commit -m "$(cat <<'EOF'
feat(usage): include Codex in usage panel aggregation

AGENT_ORDER 之前只有 Claude/Codebuddy/Opencode/V8agent 四家,Codex 即使摄取
到了用量数据(Task 2)也不会出现在任何饼图/柱状图/筛选栏里。同时给审阅时间
线加一道跳过 role:"token_usage" 行的处理,避免 Codex 的用量记录被渲染成没
有正文的空 AI 气泡。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec 覆盖检查**(对照本计划自己的 Goal,没有独立 spec 文档):
- "Codex 卡片显示模型名" → Task 1 覆盖(`latest_model_mode_and_activity` 的 `turn_context` 分支 + `agent_card_refresh_plan` 门禁)。
- "Codex 用量面板能统计到数据" → Task 2(摄取)+ Task 3(收录进 `AGENT_ORDER`)共同覆盖。
- "不能因为这次改动污染其他 agent 的既有行为" → Task 2 Step 4 的函数改名只涉及一个私有函数和它在文件内的唯一调用点,不改变 Codebuddy 的解析逻辑本身;Task 3 的 `AGENT_ORDER` 改动是纯增量(新增一个数组元素),不改变其余四家的顺序/口径。
- "不能让 Codex 的用量行污染审阅时间线" → Task 3 Step 1-4 显式测试 + 修复。

**占位符检查**:全部 Step 都给了完整可编译的 Rust 代码/JSONL 内容,没有"TBD"/"参考 Task N 实现"这类占位表述。

**类型一致性检查**:`parse_codex_shaped_chunk`/`parse_iso8601`(实为 `parse_codex_timestamp_ms`)/`days_from_civil`/`join_text_blocks_by_kind` 四个新函数名在 Task 2 的 Step 5/6 里前后一致;`role: "token_usage"` 这个新角色字符串在 Task 2(产出)和 Task 3(消费/跳过)两处的拼写完全一致。
