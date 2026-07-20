//! ClaudeCode 对话来源（P1j）：扫描 `~/.claude/projects/<cwd换->` 列出该项目
//! 的历史对话(JSONL),取首句人类发言当标题。纯 IO + 纯解析;dozerd 不参与。
//! `agent` 字段为多 agent 留维度(现恒 "claude");核心列表只吃 ConversationMeta。
// 过渡期:T3/T4 接线前无调用方（沿用先例）。
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMeta {
    pub path: PathBuf,
    pub title: String,
    pub modified_ms: u64,
    pub size_bytes: u64,
    pub agent: String,
}

/// cwd → Claude 存储目录：`~/.claude/projects/<cwd 中 '/' 换 '-'>`。
pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let key = cwd.to_string_lossy().replace('/', "-");
    PathBuf::from(home)
        .join(".claude")
        .join("projects")
        .join(key)
}

/// transcript 首段 → 首句人类发言(首个字符串型 user content)。纯函数。
pub fn conversation_title(jsonl_head: &str) -> Option<String> {
    for line in jsonl_head.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("user")
            && let Some(text) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
        {
            return Some(text.to_string());
        }
    }
    None
}

/// 该对话是否是当前活会话(其路径在打开着的 transcript 集合中)。纯函数。
pub fn is_current_conversation(meta_path: &Path, open_transcripts: &[String]) -> bool {
    let p = meta_path.to_string_lossy();
    open_transcripts.iter().any(|o| o.as_str() == p)
}

/// 列出目录下所有 .jsonl 为对话(mtime 倒序)。读失败/非目录返回空。
/// 标题只读文件前若干字节以省 IO。
pub fn list_conversations(dir: &Path) -> Vec<ConversationMeta> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<ConversationMeta> = rd
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                return None;
            }
            let meta = e.metadata().ok()?;
            let size_bytes = meta.len();
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let head = read_head(&path, 16 * 1024);
            let title = conversation_title(&head).unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "(无标题对话)".into())
            });
            Some(ConversationMeta {
                path,
                title,
                modified_ms,
                size_bytes,
                agent: "claude".into(),
            })
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.modified_ms));
    out
}

/// 读文件前 n 字节为 String(lossy)。
fn read_head(path: &Path, n: usize) -> String {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..read]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn title_from_first_string_user_turn() {
        let head = concat!(
            "{\"type\":\"mode\",\"mode\":\"x\"}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n"
        );
        assert_eq!(conversation_title(head).as_deref(), Some("改一下 README"));
        assert_eq!(conversation_title("{\"type\":\"mode\"}\nbad\n"), None);
    }

    #[test]
    fn is_current_matches_open_transcript() {
        use std::path::Path;
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation(Path::new("/t/a.jsonl"), &opens));
        assert!(!is_current_conversation(Path::new("/t/c.jsonl"), &opens));
    }

    #[test]
    fn list_sorts_by_mtime_desc_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("one.jsonl");
        let p2 = dir.path().join("two.jsonl");
        std::fs::write(
            &p1,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第一个\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            &p2,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二个\"}}\n",
        )
        .unwrap();
        let list = list_conversations(dir.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "第二个", "mtime 倒序:后写的在前");
        assert_eq!(list[0].agent, "claude");
        assert!(list[0].size_bytes > 0);
        assert!(list_conversations(std::path::Path::new("/no/such/dir")).is_empty());
    }
}
