# Dozer OpenCode Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `dozer-hook` 能接住 OpenCode 侧转发来的规范化事件，并把其中携带的内容代写成 Claude 格式的 transcript JSONL，使 `dozer-app` 现有（Plan 1 已实现）的 `AgentKind::Opencode` 解析路径有真实数据可读。

**Architecture:** OpenCode 没有"CLI 调用 hook 脚本"的机制，翻译成规范事件名和识别边沿这部分逻辑必须活在 opencode 进程内部的插件里（spec §5.3）；插件把翻译好的规范事件 + 一个可选的、已经拍成 Claude JSONL 行形状的 `transcript_line` 对象，通过 `dozer-hook opencode <event>`（复用 Plan 1 搭好的 CLI 骨架）转发给 dozerd。dozer-hook 收到后除了照常转发 `Request::HookEvent`，还要把 `transcript_line`（如果有）追加写进 `~/.dozer/agents/opencode/projects/<cwd-key>/<session-id>.jsonl`。**这个 `transcript_line` 该长什么样，由插件自己按 Claude 的字段形状构造好再传过来——dozer-hook 的 Rust side 完全不需要认识 OpenCode 内部的 `session.*`/`message.*`/`part.*` 事件结构**，只做"有就追加写"，翻译复杂度被完全封在插件里，不泄漏到 Rust 侧。

**Tech Stack:** Rust 2024（`crates/dozer-hook`）+ TypeScript（OpenCode 插件，独立于 cargo workspace）。

## Global Constraints

- **前置依赖**：本计划假定 `docs/superpowers/plans/2026-07-31-dozer-multi-agent-foundation.md` 已合并（`AgentKind`、`dozer-hook <agent> <event>` CLI 形态、`opencode_project_dir`/`parse_transcript` 的 Opencode 分支都已经在 Plan 1 里实现）。
- **本计划把工作拆成两半，只交付前一半**：Task 1（Rust 侧：`dozer-hook opencode` 接住 `transcript_line` 并落盘）是可以现在就写死代码、有单元测试保障的，本计划覆盖到底。**真正的 TypeScript 插件（Task 2 是它的 spike）本计划不交付最终实现**——OpenCode 插件的确切 API 形状（怎么注册、hook 函数签名、怎么订阅 `session.*`/`part.*` 事件总线）在公开文档里没有给到函数级别的确定性，硬写等于在猜語法。Task 2 的产出是一份验证清单 + 决策记录，真正的插件代码是这份 spike 之后的独立小计划——这不是偷懒少写一个任务，是"没有可验证的 API 事实就不该把猜测当代码写进 no-placeholder 的计划里"（跟 spec §6 对 CodeBuddy 的处理是同一个原则）。
- 只要 Task 1 落地，`~/.dozer/agents/opencode/projects/...` 这条路径和"代写 Claude 格式行"这个契约就是稳定的、可以先测的——不依赖插件是否已经存在。
- `transcript_line` 的 JSON 形状必须能被 `dozer-app/src/transcript.rs::parse_transcript(AgentKind::Opencode, ...)`（Plan 1 已实现，复用 Claude 分支）正确解析，也就是必须是 `{"type":"user","message":{"role":"user","content":"..."}}` 或 `{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}, {"type":"tool_use","name":"...","input":{...}}, ...]}}` 这两种形状之一。

---

## Task 1: `dozer-hook opencode` 落盘 transcript 行

**Files:**
- Create: `crates/dozer-hook/src/opencode.rs`
- Modify: `crates/dozer-hook/src/main.rs`（顶部加 `mod opencode;`；`forward` 函数体，Plan 1 落地后的版本，转发成功后调用落盘）

**Interfaces:**
- Produces: `pub fn transcript_path(cwd: &str, session_id: &str) -> PathBuf`；`pub fn append_transcript_line(cwd: &str, session_id: &str, line: &serde_json::Value) -> std::io::Result<()>`。

- [ ] **Step 1: 写失败的测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let home = tempfile::tempdir().unwrap();
        let prev = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", home.path()) };
        let result = f(home.path());
        match prev {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        result
    }

    #[test]
    fn transcript_path_mirrors_conversation_rs_layout() {
        with_home(|home| {
            let p = transcript_path("/a/b/c", "sess-1");
            assert_eq!(
                p,
                home.join(".dozer")
                    .join("agents")
                    .join("opencode")
                    .join("projects")
                    .join("-a-b-c")
                    .join("sess-1.jsonl")
            );
        });
    }

    #[test]
    fn append_creates_parent_dirs_and_writes_one_json_line() {
        with_home(|_home| {
            let line = json!({"type": "user", "message": {"role": "user", "content": "hi"}});
            append_transcript_line("/proj", "sess-2", &line).unwrap();
            let path = transcript_path("/proj", "sess-2");
            let content = std::fs::read_to_string(&path).unwrap();
            assert_eq!(content.lines().count(), 1);
            let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(parsed, line);
        });
    }

    #[test]
    fn append_twice_produces_two_lines_in_order() {
        with_home(|_home| {
            let l1 = json!({"type": "user", "message": {"role": "user", "content": "第一句"}});
            let l2 = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "回复"}]}});
            append_transcript_line("/proj2", "sess-3", &l1).unwrap();
            append_transcript_line("/proj2", "sess-3", &l2).unwrap();
            let content = std::fs::read_to_string(transcript_path("/proj2", "sess-3")).unwrap();
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(lines.len(), 2);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(lines[0]).unwrap()["message"]["content"],
                "第一句"
            );
        });
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook opencode:: -- --nocapture`
Expected: FAIL，`crates/dozer-hook/src/opencode.rs` 不存在，编译错误。

- [ ] **Step 3: 实现**

创建 `crates/dozer-hook/src/opencode.rs`：

```rust
//! OpenCode 没有自己的 JSONL transcript 落盘（数据在它自己的 SQLite
//! 里），dozer-hook 代它按 Claude 的字段形状写一份，好让
//! `dozer-app/src/transcript.rs` 的 Claude 分支零改动直接复用（spec
//! §5.3）。目录布局跟 `dozer-app/src/conversation.rs::opencode_project_dir`
//! 保持逐字节一致——两边各自独立实现是因为分属不同 crate（`dozer-hook`
//! 不依赖 `dozer-app`），布局约定写在 spec 里、不是从代码互相 import 来的。

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

fn project_key(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// cwd + dozer session id → 这次会话代写的 transcript 文件路径。
pub fn transcript_path(cwd: &str, session_id: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home)
        .join(".dozer")
        .join("agents")
        .join("opencode")
        .join("projects")
        .join(project_key(cwd))
        .join(format!("{session_id}.jsonl"))
}

/// 追加一行（每次事件只 append，不做 read-modify-write，避免并发写坏文件；
/// 失败由调用方决定是否吞掉，本函数如实返回 `io::Result`）。
pub fn append_transcript_line(cwd: &str, session_id: &str, line: &Value) -> std::io::Result<()> {
    let path = transcript_path(cwd, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with_home<T>(f: impl FnOnce(&Path) -> T) -> T {
        let home = tempfile::tempdir().unwrap();
        let prev = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", home.path()) };
        let result = f(home.path());
        match prev {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        result
    }

    #[test]
    fn transcript_path_mirrors_conversation_rs_layout() {
        with_home(|home| {
            let p = transcript_path("/a/b/c", "sess-1");
            assert_eq!(
                p,
                home.join(".dozer")
                    .join("agents")
                    .join("opencode")
                    .join("projects")
                    .join("-a-b-c")
                    .join("sess-1.jsonl")
            );
        });
    }

    #[test]
    fn append_creates_parent_dirs_and_writes_one_json_line() {
        with_home(|_home| {
            let line = json!({"type": "user", "message": {"role": "user", "content": "hi"}});
            append_transcript_line("/proj", "sess-2", &line).unwrap();
            let path = transcript_path("/proj", "sess-2");
            let content = std::fs::read_to_string(&path).unwrap();
            assert_eq!(content.lines().count(), 1);
            let parsed: Value = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(parsed, line);
        });
    }

    #[test]
    fn append_twice_produces_two_lines_in_order() {
        with_home(|_home| {
            let l1 = json!({"type": "user", "message": {"role": "user", "content": "第一句"}});
            let l2 = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "回复"}]}});
            append_transcript_line("/proj2", "sess-3", &l1).unwrap();
            append_transcript_line("/proj2", "sess-3", &l2).unwrap();
            let content = std::fs::read_to_string(transcript_path("/proj2", "sess-3")).unwrap();
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(lines.len(), 2);
            assert_eq!(
                serde_json::from_str::<Value>(lines[0]).unwrap()["message"]["content"],
                "第一句"
            );
        });
    }
}
```

`main.rs` 顶部（`mod codebuddy;` 之后，若 CodeBuddy 计划尚未落地就跟在 `mod install;` 之后）加：

```rust
mod opencode;
```

`forward()` 函数体里，`let req = Request::HookEvent { ... };` 这行**之前**插入（必须在这之前：`session_id` 和 `data` 都会被这次构造按字段简写移动进 `req`，之后就不能再借用了）：

```rust
    if agent == AgentKind::Opencode
        && let Some(line) = data.get("transcript_line").filter(|v| !v.is_null())
    {
        let cwd = data
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        if let Err(e) = opencode::append_transcript_line(cwd, &session_id, line) {
            eprintln!("opencode transcript 落盘失败（已忽略，不影响转发）: {e}");
        }
    }
```

改完之后 `forward()` 函数体的完整顺序应该是：算出 `session_id`/`data`/`event`/`ts_ms` → 上面这段 OpenCode 落盘（用 `&session_id`、`&data` 借用，不移动）→ `let req = Request::HookEvent { session_id, agent, event, ts_ms, data };`（这里才真正移动）→ 连 socket、发送。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS，全量 `dozer-hook` 测试绿（含本任务 3 个新测试 + Plan 1/2 遗留测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/opencode.rs crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): append plugin-supplied transcript lines for OpenCode sessions"
```

---

## Task 2: Spike——确认 OpenCode 插件 API 形状

人工验证任务，产出决策记录，供后续"写真正的 TS 插件"这份独立计划使用。**本计划不包含插件本体的实现**（见 Global Constraints）。

**Files:**
- Create: `docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md`

- [ ] **Step 1: 确认 OpenCode CLI 可用**

Run: `opencode --version`
Expected: 打印版本号。不存在就先按官方指引装好。

- [ ] **Step 2: 找到插件加载的确切位置和 manifest 格式**

查 OpenCode 当前版本的插件文档（`opencode.ai/docs/plugins/` 或对应版本的文档站），确认：插件文件放在哪个目录（全局 `~/.config/opencode/plugin/` 还是项目级 `.opencode/plugin/`，还是 package.json 里声明）、入口文件的导出形状（默认导出一个函数？具名导出一个对象？）。写一个最小的"hello world"插件（比如在 `SessionStart` 时 `console.error` 一行），跑一个真实会话，确认它确实被加载、确实执行了。

- [ ] **Step 3: 确认能订阅到哪些事件、事件对象长什么样**

在 Step 2 的插件里打印所有能拿到的事件（`session.*`/`message.*`/`part.*`），跑一个真实对话（发一条消息、触发一次工具调用），把控制台输出记下来——重点确认：`part.updated` 里 `type: "tool"` 的事件对象具体字段名是什么（工具名、输入参数、执行状态分别在哪个字段）、`message.updated` 里怎么区分 `role: "user"` 和 `role: "assistant"`、有没有一个明确的"这条消息/这个工具调用彻底结束了"的信号。

- [ ] **Step 4: 确认环境变量继承**

在插件里打印 `process.env.DOZER_SESSION_ID`（需要先手动 `export DOZER_SESSION_ID=probe-123` 再启动 opencode，模拟 dozerd spawn PTY 时的注入），确认插件进程内能读到。若读不到，说明 spec §5.3 "插件天然继承父进程环境变量"这条假设不成立，需要另想办法传递会话 id——这种情况下 Task 1 已经交付的 Rust 侧代码不受影响（`append_transcript_line` 只认调用方传的 `session_id` 参数，不关心它是怎么来的），受影响的只是插件那份后续计划。

- [ ] **Step 5: 确认能否从插件里 spawn 子进程**

在插件里试着 spawn 一次 `echo hello > /tmp/opencode-plugin-probe.log`，确认 OpenCode 的插件沙箱（如果有的话）允许执行子进程——这是 spec §5.3"插件 spawn `dozer-hook opencode <event>`"这个转发方式成立的前提。若不允许，需要改用"插件直接连 dozerd 的 Unix socket"这条路（spec 方案 B 提到过的备选），Task 1 的 `append_transcript_line`/`transcript_path` 仍然可以复用，只是调用方从"dozer-hook 转发时顺带调用"变成需要一个新的直连入口——这个改动不在本计划范围内，记录进决策文档，留给插件计划处理。

- [ ] **Step 6: 写决策记录**

创建 `docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md`，至少包含：插件文件放哪、manifest/导出形状、能拿到的事件字段清单（附一份真实事件对象的 JSON 转储）、`DOZER_SESSION_ID` 是否可读、能否 spawn 子进程。这份文档是插件实现计划的直接输入。

- [ ] **Step 7: 提交**

```bash
git add docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md
git commit -m "docs: record OpenCode plugin API spike findings"
```

---

## 完成检查

- [ ] `dozer-hook opencode <event>` 在收到带 `transcript_line` 字段的 hook JSON 时，能正确把这一行代写进 `~/.dozer/agents/opencode/projects/<cwd-key>/<session-id>.jsonl`，多次调用按顺序追加，不覆盖。
- [ ] 落盘失败（权限/磁盘满）不影响 `Request::HookEvent` 照常转发给 dozerd——两件事互相独立，一个失败不拖累另一个。
- [ ] `cargo test -p dozer-hook` 全绿。
- [ ] Task 2 的决策记录写清楚了插件到底怎么装、事件字段长什么样、能不能 spawn 子进程。
- [ ] **明确排除在本计划外**：OpenCode 插件本体（TypeScript 代码）不在本计划交付范围内，是 Task 2 决策记录之后的独立后续计划——这是因为插件要调用的具体 API（订阅哪个事件、字段怎么读）在写这份计划时没有可验证的事实依据，写死等于猜语法，不满足"计划里的每一步都要是可执行的真实内容"这条要求。
