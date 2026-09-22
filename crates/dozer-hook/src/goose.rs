//! Goose hook journal 写入（spec `2026-09-21-goose-agent-integration` D4）。
//!
//! Goose 的原生 hook 事件 JSON 不是 Claude/CodeBuddy 那种"扁平 transcript
//! 行"，也不该伪装成 Claude shape（否则下游会误以为数据完整——Goose hook
//! payload 没有稳定的 token usage 和完整 tool output）。所以这里把每次 hook
//! 调用的原始 stdin JSON 规范化成 schema v1 的 journal 行，追加到 Dozer 自有
//! 的 `~/.dozer/agents/goose/projects/<cwd-key>/<dozer-session-id>.jsonl`，
//! 由 dozerd 的 Goose 专用 parser 增量摄取。journal 的绝对路径由调用方补进
//! `data.transcript_path`，复用 dozerd 现有 `maybe_ingest_from_hook_data`。

use dozer_core::agent_paths::{goose_project_dir_in, home_dir};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 单行 payload 大小上限：超限保留事件元数据、把大字段替换成截断标记，
/// 防止异常工具输出把磁盘和 SQLite 拖垮（spec §5）。
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// journal 绝对路径。`cwd` 是 hook 子进程的当前工作目录——Goose 用 `sh -c`
/// 起 hook，继承会话工作目录（见 spec D3/D4，这是唯一对所有事件都稳定可用
/// 的"项目路径"来源：payload 里的 `working_dir` 只在 tool 事件上出现）。
/// `session_id` 是注入的 `DOZER_SESSION_ID`。
pub fn journal_path(cwd: &Path, session_id: &str) -> PathBuf {
    journal_path_in(&home_dir(), cwd, session_id)
}

/// `home` 显式传入版本，测试用（不碰 `HOME` 环境变量）。
pub fn journal_path_in(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    goose_project_dir_in(home, cwd).join(format!("{session_id}.jsonl"))
}

/// 把一次 hook 调用规范化成 schema v1 的 journal 行（序列化后的 JSON 字符串，
/// 不含换行）。`payload` 是 Goose 原生 hook 事件的完整 stdin JSON；
/// `goose_session_id` 取 `payload.session_id`（Goose 自己的会话 ID，只作诊断
/// 和未来 resume 映射，见 spec D3）。超过 1 MiB 时把 `payload` 替换成截断
/// 标记对象，保留事件元数据。
pub fn build_journal_line(
    event: &str,
    ts_ms: u64,
    dozer_session_id: &str,
    payload: &Value,
) -> String {
    let goose_session_id = payload
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(str::to_string);
    let mut obj = serde_json::Map::new();
    obj.insert("schema_version".into(), Value::from(1u64));
    obj.insert("type".into(), Value::from("goose_hook"));
    obj.insert("event".into(), Value::from(event));
    obj.insert("ts_ms".into(), Value::from(ts_ms));
    obj.insert("dozer_session_id".into(), Value::from(dozer_session_id));
    if let Some(id) = goose_session_id {
        obj.insert("goose_session_id".into(), Value::from(id));
    }
    obj.insert("payload".into(), payload.clone());
    let line = Value::Object(obj).to_string();
    if line.len() <= MAX_LINE_BYTES {
        return line;
    }
    // 超限：丢弃大 payload，保留事件元数据 + 截断标记。
    let mut obj = serde_json::Map::new();
    obj.insert("schema_version".into(), Value::from(1u64));
    obj.insert("type".into(), Value::from("goose_hook"));
    obj.insert("event".into(), Value::from(event));
    obj.insert("ts_ms".into(), Value::from(ts_ms));
    obj.insert("dozer_session_id".into(), Value::from(dozer_session_id));
    obj.insert("payload".into(), serde_json::json!({ "truncated": true }));
    Value::Object(obj).to_string()
}

/// 把一行 journal 追加到文件（建目录 + 逐行 append）。失败只记 `eprintln!`、
/// 不 panic——journal 写失败仍要照常转发状态事件（spec §5）。
pub fn append_journal_line(path: &Path, line: &str) {
    use std::io::Write;
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("建 journal 目录失败（已忽略）: {e}");
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        eprintln!("打开 journal 失败（已忽略）: {}", path.display());
        return;
    };
    if let Err(e) = writeln!(f, "{line}") {
        eprintln!("写 journal 失败（已忽略）: {e}");
    }
}

/// 组合式入口：把一次 Goose hook 调用落盘成 journal 行，返回 journal 的绝对
/// 路径供调用方补进 `transcript_path`。`cwd` 由调用方（hook 进程）传入。
pub fn write_journal(
    cwd: &Path,
    session_id: &str,
    event: &str,
    ts_ms: u64,
    payload: &Value,
) -> PathBuf {
    let path = journal_path(cwd, session_id);
    append_journal_line(
        &path,
        &build_journal_line(event, ts_ms, session_id, payload),
    );
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_path_in_lives_under_dozer_goose_projects_dir() {
        let p = journal_path_in(Path::new("/home/u"), Path::new("/a/b/c"), "sess-1");
        assert_eq!(
            p,
            PathBuf::from("/home/u/.dozer/agents/goose/projects/-a-b-c/sess-1.jsonl")
        );
    }

    #[test]
    fn build_journal_line_extracts_event_and_goose_session_id() {
        let payload: Value = serde_json::from_str(
            r#"{"event":"PreToolUse","session_id":"g-1","tool_name":"developer__shell","tool_input":{"command":"cargo test"}}"#,
        )
        .unwrap();
        let line = build_journal_line("PreToolUse", 1_789_950_000_000, "dozer-sess", &payload);
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["type"], "goose_hook");
        assert_eq!(v["event"], "PreToolUse");
        assert_eq!(v["dozer_session_id"], "dozer-sess");
        assert_eq!(v["goose_session_id"], "g-1");
        assert_eq!(v["payload"]["tool_name"], "developer__shell");
    }

    #[test]
    fn build_journal_line_truncates_oversized_payload() {
        let big = "x".repeat(MAX_LINE_BYTES + 1);
        let payload: Value = serde_json::json!({ "tool_input": big });
        let line = build_journal_line("PostToolUse", 1, "s", &payload);
        assert!(line.len() <= MAX_LINE_BYTES);
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["payload"]["truncated"], true);
        assert_eq!(v["event"], "PostToolUse", "截断仍保留事件元数据");
    }

    #[test]
    fn append_journal_line_creates_dir_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("s.jsonl");
        append_journal_line(&path, "{\"a\":1}");
        append_journal_line(&path, "{\"a\":2}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\"a\":1}\n{\"a\":2}\n");
    }
}
