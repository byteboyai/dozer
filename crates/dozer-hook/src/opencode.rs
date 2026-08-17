//! OpenCode 没有自己的 JSONL transcript 落盘（数据在它自己的 SQLite
//! 里），dozer-hook 代它按 Claude 的字段形状写一份，好让
//! `dozer-app/src/transcript.rs` 的 Claude 分支零改动直接复用（spec
//! §5.3）。目录布局跟 `dozer-app/src/conversation.rs::opencode_project_dir`
//! 保持逐字节一致——两边各自独立实现是因为分属不同 crate（`dozer-hook`
//! 不依赖 `dozer-app`），布局约定写在 spec 里、不是从代码互相 import 来的。
//!
//! `home` 走 `_in` 变体显式传入（而不是函数内部读 `HOME` 环境变量），
//! 跟 `conversation.rs::project_dir_in` 同一套手法——`HOME` 是进程级
//! 跨线程共享状态，cargo test 默认多线程跑时 mutate 它会让并发测试
//! 互相踩坏（见 commit fb975fd 在 conversation.rs 里的同源修复）。
//! 公开的 `transcript_path`/`append_transcript_line` 读 `HOME` 给生产
//! 调用用；测试走 `_in` 变体传临时目录，不碰全局环境。

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

fn project_key(cwd: &str) -> String {
    cwd.replace('/', "-")
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// cwd + dozer session id → 这次会话代写的 transcript 文件路径。
/// `main.rs::forward` 用它把这个路径写回 hook 上报给 dozerd 的
/// `data.transcript_path` 字段——OpenCode 插件自己发来的 payload 里没有
/// 这个字段(2026-08-17 之前的缺口,见该调用点注释),没有这次调用会一直
/// 是死代码。
pub fn transcript_path(cwd: &str, session_id: &str) -> PathBuf {
    transcript_path_in(&home_dir(), cwd, session_id)
}

/// 同上，`home` 显式传入——测试用，避免读全局 `HOME`。
fn transcript_path_in(home: &Path, cwd: &str, session_id: &str) -> PathBuf {
    home.join(".dozer")
        .join("agents")
        .join("opencode")
        .join("projects")
        .join(project_key(cwd))
        .join(format!("{session_id}.jsonl"))
}

/// 追加一行（每次事件只 append，不做 read-modify-write，避免并发写坏文件；
/// 失败由调用方决定是否吞掉，本函数如实返回 `io::Result`）。
pub fn append_transcript_line(cwd: &str, session_id: &str, line: &Value) -> std::io::Result<()> {
    append_transcript_line_in(&home_dir(), cwd, session_id, line)
}

fn append_transcript_line_in(
    home: &Path,
    cwd: &str,
    session_id: &str,
    line: &Value,
) -> std::io::Result<()> {
    let path = transcript_path_in(home, cwd, session_id);
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

    #[test]
    fn transcript_path_mirrors_conversation_rs_layout() {
        let home = tempfile::tempdir().unwrap();
        let p = transcript_path_in(home.path(), "/a/b/c", "sess-1");
        assert_eq!(
            p,
            home.path()
                .join(".dozer")
                .join("agents")
                .join("opencode")
                .join("projects")
                .join("-a-b-c")
                .join("sess-1.jsonl")
        );
    }

    #[test]
    fn append_creates_parent_dirs_and_writes_one_json_line() {
        let home = tempfile::tempdir().unwrap();
        let line = json!({"type": "user", "message": {"role": "user", "content": "hi"}});
        append_transcript_line_in(home.path(), "/proj", "sess-2", &line).unwrap();
        let path = transcript_path_in(home.path(), "/proj", "sess-2");
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 1);
        let parsed: Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed, line);
    }

    #[test]
    fn append_twice_produces_two_lines_in_order() {
        let home = tempfile::tempdir().unwrap();
        let l1 = json!({"type": "user", "message": {"role": "user", "content": "第一句"}});
        let l2 = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "回复"}]}});
        append_transcript_line_in(home.path(), "/proj2", "sess-3", &l1).unwrap();
        append_transcript_line_in(home.path(), "/proj2", "sess-3", &l2).unwrap();
        let content =
            std::fs::read_to_string(transcript_path_in(home.path(), "/proj2", "sess-3")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(lines[0]).unwrap()["message"]["content"],
            "第一句"
        );
    }
}
