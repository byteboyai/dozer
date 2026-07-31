# Dozer CodeBuddy Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 dozer-hook 能装进 CodeBuddy、把 CodeBuddy 的原生 hook 事件翻译成 dozerd 认识的 7 个规范事件名并转发，从而让 `SessionInfo.agent` 在用户跑 CodeBuddy 时正确翻转成 `AgentKind::Codebuddy`。

**Architecture:** 复用 `crates/dozer-hook` 里 Plan 1（`docs/superpowers/plans/2026-07-31-dozer-multi-agent-foundation.md`）已经搭好的 `<agent> <event>` CLI 形态和 `forward()` 转发骨架，新增一个 `codebuddy` 子模块做"原生事件名 → 规范事件名"翻译，并把 `install.rs` 的 JSON hook 合并逻辑从"只认 Claude 的 settings 路径"泛化成"认哪个 agent 就写哪个 settings 路径"。

**Tech Stack:** Rust 2024，`crates/dozer-hook`。

## Global Constraints

- **前置依赖**：本计划假定 `docs/superpowers/plans/2026-07-31-dozer-multi-agent-foundation.md` 已经合并——`AgentKind`、`dozer-hook <agent> <event>` 的 CLI 形态、`install.rs::run_at` 的 `{exe} claude {ev}` 写入格式都已存在。本计划里所有"Modify"的行号以那份计划落地后的文件状态为准。
- 本计划**不包含** CodeBuddy transcript（JSONL）的真实解析——`transcript.rs` 里 `AgentKind::Codebuddy` 分支目前返回空 `Vec`（Plan 1 已实现），这是诚实的"未支持"状态，不是本计划要修的 bug。真实解析依赖 Task 1 产出的样本文件，建议 Task 1 完成、拿到真实样本后，另开一个小计划专门写这个解析器——不在本计划范围内（详见"完成检查"一节）。
- 事件名翻译表以 `docs/superpowers/specs/2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md` §5.2 为准，不臆造。
- Task 4（安装器）假定 CodeBuddy 走"全局 settings 文件补丁"机制（与 Claude 同构，是 CodeBuddy 官方文档里"Hooks Guide/Hooks Reference"描述的默认路径，独立于更重的"Plugin 包"机制）。Task 1 的验证如果推翻这个假设，Task 4 需要按 Task 1 记录的实际机制重写——本计划把这个分支点写清楚（见 Task 4 末尾"若验证结果不同"）。

---

## Task 1: Spike——验证 CodeBuddy hook 注册方式与 transcript 落盘

这是人工验证任务，不是写代码任务；产出是一份决策记录 + 一个真实 transcript 样本文件，供 Task 4 和未来的 transcript 解析计划使用。**没有 CodeBuddy CLI 可用时，本任务无法完成，后续 Task 2-4 里凡是标注"依赖 Task 1 结论"的部分需要暂停，先跑通这一步。**

**Files:**
- Create: `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`（验证产出，若拿不到真实样本就不创建，不伪造）
- Create: `docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md`（验证结论记录）

- [ ] **Step 1: 确认 CodeBuddy CLI 可用**

Run: `codebuddy --version`
Expected: 打印版本号。若命令不存在，先按 CodeBuddy 官方安装指引装好，再继续。

- [ ] **Step 2: 验证全局 settings 补丁机制是否可行**

在一个临时目录里，手工往 `~/.codebuddy/settings.json`（若不存在则新建 `{}`）加一条 hook 注册，格式参照 CodeBuddy 的 Hooks Reference 文档，形如：

```json
{
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command", "command": "sh -c 'cat >> /tmp/codebuddy-hook-probe.log'" }] }]
  }
}
```

跑一个真实 CodeBuddy 会话，触发一次 `Stop`（正常对话结束），检查 `/tmp/codebuddy-hook-probe.log` 是否被写入、内容是不是一段 JSON。

Expected 两种可能之一：
- **命中**：`/tmp/codebuddy-hook-probe.log` 有内容，说明全局 settings 补丁机制可行，走 Task 4 现有设计。
- **未命中**：文件为空/不存在，说明 CodeBuddy 不认全局 settings 里的 `hooks` 段（需要走 plugin 包机制），记录这个结论，Task 4 需要重新设计（不在本计划范围，见 Task 4 末尾）。

- [ ] **Step 3: 检查探测日志里的字段**

若 Step 2 命中，用 `cat /tmp/codebuddy-hook-probe.log` 查看那段 JSON，确认是否含 `session_id`/`transcript_path`/`cwd`/`hook_event_name` 字段（spec §1 依据公开文档做的假设）。记下实际字段名——如果跟假设不一致，Task 2/3 的翻译表和字段读取逻辑要跟着改。

- [ ] **Step 4: 抓一份真实 transcript 样本**

从 Step 2 抓到的 `transcript_path` 指向的文件复制一份出来（脱敏：删掉真实项目路径/文件内容，只留结构），存到 `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`。这份 fixture 是未来"CodeBuddy transcript 解析"计划的输入，不是本计划要解析的对象。

- [ ] **Step 5: 写决策记录**

创建 `docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md`，内容至少包含：hook 注册机制是全局补丁还是 plugin 包、探测日志里的实际字段名（跟 spec §1 假设的差异，如果有）、transcript 样本是否拿到。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md
git commit -m "docs: record CodeBuddy hook registration spike findings"
```

（若 Step 4 拿不到可脱敏的样本，跳过 fixture 文件，只提交决策记录，commit message 改成 "docs: record CodeBuddy hook registration spike findings (no transcript sample)"。）

---

## Task 2: CodeBuddy 事件名翻译表

**Files:**
- Create: `crates/dozer-hook/src/codebuddy.rs`
- Modify: `crates/dozer-hook/src/main.rs`（顶部加 `mod codebuddy;`）

**Interfaces:**
- Produces: `pub fn translate_event(raw: &str) -> Option<String>`——`None` 表示这个事件不转发（子 agent 生命周期事件）；`Some(name)` 是翻译后的规范事件名。

- [ ] **Step 1: 写失败的测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_events_map_to_same_name() {
        assert_eq!(translate_event("SessionStart"), Some("SessionStart".to_string()));
        assert_eq!(translate_event("UserPromptSubmit"), Some("UserPromptSubmit".to_string()));
        assert_eq!(translate_event("PreToolUse"), Some("PreToolUse".to_string()));
        assert_eq!(translate_event("PostToolUse"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("Notification"), Some("Notification".to_string()));
        assert_eq!(translate_event("SessionEnd"), Some("SessionEnd".to_string()));
    }

    #[test]
    fn failure_variants_merge_into_success_variant() {
        assert_eq!(translate_event("PostToolUseFailure"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("StopFailure"), Some("Stop".to_string()));
        assert_eq!(translate_event("Stop"), Some("Stop".to_string()));
    }

    #[test]
    fn subagent_events_are_dropped() {
        assert_eq!(translate_event("SubagentStart"), None);
        assert_eq!(translate_event("SubagentStop"), None);
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        // 未知事件原样透传给 dozerd 的 agent_state_for，那边自己会因为
        // 认不出而不改状态（spec §3：翻译层对未知事件不 panic、不拦截）。
        assert_eq!(translate_event("SomeFutureEvent"), Some("SomeFutureEvent".to_string()));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook codebuddy:: -- --nocapture`
Expected: FAIL，`crates/dozer-hook/src/codebuddy.rs` 不存在，编译错误。

- [ ] **Step 3: 实现**

创建 `crates/dozer-hook/src/codebuddy.rs`：

```rust
//! CodeBuddy 原生 hook 事件名 → dozerd 规范事件名翻译（spec §5.2）。
//! 规范词汇表就是 dozerd::agent_state_for 认识的 7 个 Claude 事件名，
//! 这里只做翻译，不引入新词汇。

/// `None` = 这个事件不转发给 dozerd（子 agent 生命周期，不影响顶层四态机）。
/// 未知事件原样透传——翻译层不对"没见过的事件"做任何假设，交给 dozerd
/// 那边的 `agent_state_for` 自己因为认不出而不改状态。
pub fn translate_event(raw: &str) -> Option<String> {
    match raw {
        "SessionStart" => Some("SessionStart"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" | "PostToolUseFailure" => Some("PostToolUse"),
        "Notification" => Some("Notification"),
        "Stop" | "StopFailure" => Some("Stop"),
        "SessionEnd" => Some("SessionEnd"),
        "SubagentStart" | "SubagentStop" => return None,
        other => other,
    }
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_events_map_to_same_name() {
        assert_eq!(translate_event("SessionStart"), Some("SessionStart".to_string()));
        assert_eq!(translate_event("UserPromptSubmit"), Some("UserPromptSubmit".to_string()));
        assert_eq!(translate_event("PreToolUse"), Some("PreToolUse".to_string()));
        assert_eq!(translate_event("PostToolUse"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("Notification"), Some("Notification".to_string()));
        assert_eq!(translate_event("SessionEnd"), Some("SessionEnd".to_string()));
    }

    #[test]
    fn failure_variants_merge_into_success_variant() {
        assert_eq!(translate_event("PostToolUseFailure"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("StopFailure"), Some("Stop".to_string()));
        assert_eq!(translate_event("Stop"), Some("Stop".to_string()));
    }

    #[test]
    fn subagent_events_are_dropped() {
        assert_eq!(translate_event("SubagentStart"), None);
        assert_eq!(translate_event("SubagentStop"), None);
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        assert_eq!(translate_event("SomeFutureEvent"), Some("SomeFutureEvent".to_string()));
    }
}
```

`main.rs` 顶部（`mod install;` 之后）加：

```rust
mod codebuddy;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook codebuddy:: -- --nocapture`
Expected: PASS（4 个测试）

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/codebuddy.rs crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): CodeBuddy native event name translation table"
```

---

## Task 3: `forward()` 接入翻译表

**Files:**
- Modify: `crates/dozer-hook/src/main.rs`（`forward` 函数体，Plan 1 落地后的版本）

**Interfaces:**
- Consumes: `codebuddy::translate_event`（Task 2）。
- Produces: `agent == AgentKind::Codebuddy` 时，`forward()` 转发前先翻译事件名；翻译结果是 `None` 时整次调用直接返回、不发任何东西给 dozerd。

- [ ] **Step 1: 写失败的测试**

`main.rs` 现有 `#[cfg(test)] mod tests`（Plan 1 Task 7 加的）里新增：

```rust
    #[test]
    fn resolve_event_translates_codebuddy_failure_variants() {
        assert_eq!(
            resolve_event(AgentKind::Codebuddy, Some("PostToolUseFailure")),
            Some("PostToolUse".to_string())
        );
        assert_eq!(
            resolve_event(AgentKind::Codebuddy, Some("StopFailure")),
            Some("Stop".to_string())
        );
    }

    #[test]
    fn resolve_event_drops_codebuddy_subagent_events() {
        assert_eq!(resolve_event(AgentKind::Codebuddy, Some("SubagentStart")), None);
    }

    #[test]
    fn resolve_event_passes_claude_events_through_untranslated() {
        assert_eq!(
            resolve_event(AgentKind::Claude, Some("PostToolUseFailure")),
            Some("PostToolUseFailure".to_string()),
            "Claude 分支不套用 CodeBuddy 的翻译表"
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook resolve_event -- --nocapture`
Expected: FAIL，`resolve_event` 函数不存在，编译错误。

- [ ] **Step 3: 实现**

把 `forward()` 里"决定 event 字符串"那一段逻辑抽成独立函数（Plan 1 里 `forward` 的这段）：

```rust
    let event = match event_arg {
        Some(e) => e.to_string(),
        None => data
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
    };
```

改成：

```rust
    let Some(event) = resolve_event(agent, event_arg) else {
        return; // 该事件按翻译表规则被丢弃（如 CodeBuddy 的 Subagent* 事件）
    };
```

并在 `forward` 函数上方新增：

```rust
/// 决定最终转发给 dozerd 的事件名：先取原始事件名（CLI 参数缺失时退回
/// hook JSON 里的 `hook_event_name` 字段），再按 agent 过一遍翻译表。
/// `None` 表示这次调用不该转发（spec §5.2：子 agent 生命周期事件丢弃）。
fn resolve_event(agent: AgentKind, event_arg: Option<&str>) -> Option<String> {
    let raw = event_arg.map(str::to_string);
    let raw = raw.unwrap_or_else(|| "unknown".to_string());
    match agent {
        AgentKind::Codebuddy => codebuddy::translate_event(&raw),
        _ => Some(raw),
    }
}
```

> 注意：这个改写丢掉了原来"CLI 参数缺失时读 hook JSON 里的 `hook_event_name`"这条回退逻辑里对 `data` 的依赖——`resolve_event` 现在不读 `data`，只读 CLI 传入的 `event_arg`。需要把 `data` 的读取挪到 `resolve_event` 调用之前完成（`forward` 函数里 `let data = ...` 那行已经在 `resolve_event` 调用之前，只是原来的 `hook_event_name` 兜底要一并搬过来）。完整替换 `forward` 函数体：

```rust
fn forward(agent: AgentKind, event_arg: Option<&str>) {
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        return;
    };
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let data = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let event_arg = event_arg.or_else(|| data.get("hook_event_name").and_then(|v| v.as_str()));
    let Some(event) = resolve_event(agent, event_arg) else {
        return;
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let req = Request::HookEvent {
        session_id,
        agent,
        event,
        ts_ms,
        data,
    };

    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(dozer_core::paths::socket_path())
    else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.write_all(encode_line(&req).as_bytes());
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS（`resolve_event` 新测试 + Plan 1 遗留的 `parse_agent` 测试全绿）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): apply CodeBuddy event translation before forwarding"
```

---

## Task 4: CodeBuddy hook 安装器

假定 Task 1 确认 CodeBuddy 走"全局 settings 文件补丁"机制（与 Claude 同构）。**若 Task 1 的结论是 plugin 包机制，本任务的 Step 3 需要重写成"生成 plugin 目录 + `hooks/hooks.json` manifest"，`run_at` 这套"读 JSON、增量合并、写回"的函数不适用——这种情况下把本任务标记为跳过，另开一个任务写 plugin 包安装器，Task 2/3 产出的翻译表和转发逻辑不受影响，照常复用。**

**Files:**
- Modify: `crates/dozer-hook/src/install.rs:18-24`（`settings_path`，泛化成按 agent 取路径）
- Modify: `crates/dozer-hook/src/install.rs:41-82`（`run_at`，命令串里的 agent 名参数化）
- Modify: `crates/dozer-hook/src/main.rs`（`install`/`uninstall` 分派加 agent 参数）
- Modify: `crates/dozer-hook/src/install.rs`（既有 4 个测试 + 新增 CodeBuddy 用例）

**Interfaces:**
- Consumes: Plan 1 Task 7 产出的 `run_at`/`settings_path`/`EVENTS`。
- Produces: `pub fn settings_path_for(agent: &str) -> PathBuf`；`run_at(path: &Path, agent: &str, install: bool) -> i32`（签名新增 `agent` 参数）；CLI `dozer-hook install codebuddy` / `dozer-hook uninstall codebuddy` 能装/卸 `~/.codebuddy/settings.json`。

- [ ] **Step 1: 写失败的测试**

`install.rs` 现有测试模块，把 `install_creates_settings_and_registers_all_events` 改成参数化（新增一个 CodeBuddy 版本，不改原有 Claude 版本的断言内容，只改调用点传参）：

```rust
    #[test]
    fn install_creates_settings_and_registers_all_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, "claude", true), 0);
        let root = read(&path);
        for ev in EVENTS {
            let arr = root["hooks"][ev].as_array().expect(ev);
            assert_eq!(arr.len(), 1, "{ev}");
            let cmd = arr[0]["hooks"][0]["command"].as_str().unwrap();
            assert!(
                cmd.contains("dozer-hook") || cmd.contains("dozer_hook"),
                "{cmd}"
            );
            assert!(cmd.contains(" claude "), "{cmd}: 应携带 agent 标识");
            assert!(cmd.ends_with(ev), "{cmd}");
        }
    }

    #[test]
    fn install_writes_agent_specific_command_for_codebuddy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, "codebuddy", true), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" codebuddy "), "{cmd}");
    }

    #[test]
    fn settings_path_for_codebuddy_points_at_codebuddy_dir() {
        // DOZER_CODEBUDDY_SETTINGS 覆盖，跟既有 DOZER_CLAUDE_SETTINGS 同一套测试手法。
        unsafe { std::env::set_var("DOZER_CODEBUDDY_SETTINGS", "/tmp/probe-codebuddy.json") };
        assert_eq!(
            settings_path_for("codebuddy"),
            std::path::PathBuf::from("/tmp/probe-codebuddy.json")
        );
        unsafe { std::env::remove_var("DOZER_CODEBUDDY_SETTINGS") };
    }
```

其余既有测试（`install_is_idempotent_and_preserves_foreign_hooks`/`uninstall_removes_only_ours`/`malformed_settings_refused_without_write`）调用 `run_at(&path, true)` 的地方全部改成 `run_at(&path, "claude", true)`（保持原有断言不变，只是签名多一个参数）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook install:: -- --nocapture`
Expected: FAIL，`run_at`/`settings_path_for` 签名不匹配，编译错误。

- [ ] **Step 3: 实现**

`install.rs` 第 18-24 行 `settings_path`：

```rust
/// agent 名 → 该 agent 的 hook 配置文件路径。CodeBuddy 走
/// `~/.codebuddy/settings.json`（Task 1 spike 确认的机制），环境变量
/// 覆盖用于测试，跟既有 Claude 路径同一套手法。
pub fn settings_path_for(agent: &str) -> PathBuf {
    match agent {
        "codebuddy" => {
            if let Ok(p) = std::env::var("DOZER_CODEBUDDY_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".codebuddy").join("settings.json")
        }
        _ => {
            if let Ok(p) = std::env::var("DOZER_CLAUDE_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".claude").join("settings.json")
        }
    }
}
```

（原来的 `settings_path()` 函数删掉，`main.rs` 里的调用点改成 `settings_path_for("claude")`——见本任务下面 `main.rs` 的改动。）

第 41 行 `run_at` 签名与命令串写入（第 77-81 行）：

```rust
pub fn run_at(path: &Path, agent: &str, install: bool) -> i32 {
```

```rust
        if install {
            arr.push(json!({
                "hooks": [{ "type": "command", "command": format!("{exe} {agent} {ev}") }]
            }));
        }
```

`main.rs` 里 `install`/`uninstall` 分派（Plan 1 Task 7 落地的版本）改成接受可选 agent 参数，默认 `claude`：

```rust
        Some("install") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(install::run_at(&install::settings_path_for(&agent), &agent, true))
        }
        Some("uninstall") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(install::run_at(&install::settings_path_for(&agent), &agent, false))
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS，全量 `dozer-hook` 测试绿（含 Task 2/3 遗留的测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/install.rs crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): CodeBuddy hook installer (settings.json patch)"
```

---

## 完成检查

- [ ] Task 1 的决策记录已写清楚：CodeBuddy 到底走哪种 hook 注册机制、字段名跟假设是否一致。
- [ ] `dozer-hook install codebuddy` / `dozer-hook uninstall codebuddy` 能正确装/卸 `~/.codebuddy/settings.json`（若 Task 1 结论是 plugin 包机制，这一项按该机制重新验收）。
- [ ] CodeBuddy 的 `PostToolUseFailure`/`StopFailure` 正确归并、`Subagent*` 正确丢弃，未知事件透传不 panic。
- [ ] `cargo test -p dozer-hook` 全绿。
- [ ] **明确排除在本计划外**：`transcript.rs` 里 `AgentKind::Codebuddy` 分支仍然返回空（Plan 1 已实现的诚实降级），真实解析依赖 Task 1 产出的 fixture，是一个独立的后续小计划，不在本计划验收范围内。
