//! 多 agent 对话来源（P1j 起步，P2b 扩展到 CodeBuddy/OpenCode）：扫描每个
//! agent 各自的落盘目录，列出该项目的历史对话（JSONL），取首句人类发言
//! 当标题。纯 IO + 纯解析；dozerd 不参与。

use dozer_core::protocol::AgentKind;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMeta {
    pub path: PathBuf,
    pub title: String,
    pub modified_ms: u64,
    pub size_bytes: u64,
    pub agent: AgentKind,
}

fn project_key(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// cwd → Claude 存储目录：`~/.claude/projects/<cwd 中 '/' 换 '-'>`。
pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    home_dir().join(".claude").join("projects").join(project_key(cwd))
}

/// cwd → CodeBuddy 存储目录：`~/.codebuddy/projects/<cwd 中 '/' 换 '-'>`
/// （spec §1：与 Claude 的目录结构平行）。
pub fn codebuddy_project_dir(cwd: &Path) -> PathBuf {
    home_dir().join(".codebuddy").join("projects").join(project_key(cwd))
}

/// cwd → dozer 自己为 OpenCode 代写的 transcript 目录（OpenCode 本身没有
/// JSONL 落盘，dozer-hook 按 Claude 格式代写；spec §5.3）。
pub fn opencode_project_dir(cwd: &Path) -> PathBuf {
    home_dir()
        .join(".dozer")
        .join("agents")
        .join("opencode")
        .join("projects")
        .join(project_key(cwd))
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
/// 标题只读文件前若干字节以省 IO。`agent` 用于给每条记录打标签，不影响
/// 扫描逻辑本身。
pub fn list_conversations(agent: AgentKind, dir: &Path) -> Vec<ConversationMeta> {
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
                agent,
            })
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.modified_ms));
    out
}

/// 合并 Claude/CodeBuddy/OpenCode 三个目录下同一个 cwd 的历史对话，按
/// mtime 统一倒序（spec §7：历史侧栏展示"这个项目下所有对话"，不管当年
/// 用哪个 agent 跑的）。
pub fn list_all_conversations(cwd: &Path) -> Vec<ConversationMeta> {
    let mut out = list_conversations(AgentKind::Claude, &claude_project_dir(cwd));
    out.extend(list_conversations(
        AgentKind::Codebuddy,
        &codebuddy_project_dir(cwd),
    ));
    out.extend(list_conversations(
        AgentKind::Opencode,
        &opencode_project_dir(cwd),
    ));
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
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn codebuddy_dir_uses_codebuddy_root() {
        let d = codebuddy_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.codebuddy/projects/-a-b-c"));
    }

    #[test]
    fn opencode_dir_lives_under_dozer_data_dir() {
        let d = opencode_project_dir(std::path::Path::new("/a/b/c"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.dozer/agents/opencode/projects/-a-b-c")
        );
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
        let list = list_conversations(AgentKind::Claude, dir.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "第二个", "mtime 倒序:后写的在前");
        assert_eq!(list[0].agent, AgentKind::Claude);
        assert!(list[0].size_bytes > 0);
        assert!(
            list_conversations(AgentKind::Claude, std::path::Path::new("/no/such/dir")).is_empty()
        );
    }

    #[test]
    fn list_conversations_tags_requested_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();
        let list = list_conversations(AgentKind::Codebuddy, dir.path());
        assert_eq!(list[0].agent, AgentKind::Codebuddy);
    }

    #[test]
    fn list_all_conversations_merges_three_dirs_sorted_by_mtime() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: 测试串行执行，临时改 HOME 后立即恢复；claude/codebuddy/opencode
        // 三个目录解析函数都读 HOME 环境变量。
        let prev_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", home.path()) };

        let cwd = std::path::Path::new("/proj");
        let claude_dir = claude_project_dir(cwd);
        let codebuddy_dir = codebuddy_project_dir(cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codebuddy_dir).unwrap();
        std::fs::write(
            claude_dir.join("a.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"claude 对话\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            codebuddy_dir.join("b.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"codebuddy 对话\"}}\n",
        )
        .unwrap();

        let list = list_all_conversations(cwd);

        match prev_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].agent, AgentKind::Codebuddy, "后写入的在前");
        assert_eq!(list[1].agent, AgentKind::Claude);
    }
}
