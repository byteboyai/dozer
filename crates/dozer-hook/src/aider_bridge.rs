//! Aider Markdown chat history → Dozer canonical JSONL 桥（见 spec
//! `2026-09-22-aider-agent-integration` D5）。
//!
//! 不让 dozerd 直接增量解析 `.chat.md`——一次 assistant 回复会在既有
//! `#### user` 段之后继续追加，按 byte offset 读新 chunk 会丢失"当前处于哪个
//! 角色"的上下文。这里每次 Stop/SessionEnd 读取完整 `.chat.md`，切成稳定消息，
//! 与 `.bridge.json` 已同步 ID 集合比较，只把新增的完整消息追加到 canonical
//! JSONL，再用文件锁 + 原子替换更新 bridge state。
//!
//! canonical 行形状：
//! `{"schema_version":1,"type":"aider_message","message_id":..,"role":"human"|"ai","content":..,"ts_ms":..}`

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;

const BRIDGE_SCHEMA_VERSION: u64 = 1;
/// history 最大读取量（spec §6）：超限停止同步并 warning，不截断原始 Aider 文件。
const MAX_HISTORY_BYTES: u64 = 16 * 1024 * 1024;
/// canonical 单条消息最大 1 MiB，超限截断并写 `truncated:true`（spec §6）。
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiderMessage {
    /// `"human"` 或 `"ai"`。
    pub role: String,
    pub content: String,
    /// 同 (role, content) 出现的次数（从 0 起），用于稳定 ID 去重。
    pub occurrence_index: usize,
}

/// 把完整 `.chat.md` 切成稳定消息列表。规则（实测 fixture 锁定）：
///
/// - `# aider chat started at ...`：会话边界，finalize 当前 assistant 并复位。
/// - `> ...`：blockquote（命令回显/警告/Tokens/编辑确认），跳过。
/// - `#### <content>`：用户消息；finalize 上一条 assistant。
/// - 三个反引号 ` ``` ` 切换 fence；fence 内的 `#### ` 不是新用户消息。
/// - 其余非空行：assistant 正文（只有在已见一条 user 之后）。
///
/// 不 panic、不猜测边界：只有 user 没有 assistant 时只产出 user。
pub fn parse_markdown(text: &str) -> Vec<AiderMessage> {
    parse_markdown_with_inputs(text, &[])
}

/// 借助 prompt-toolkit input history 恢复多行用户消息。Aider 的 chat
/// history 只给用户消息第一行加 `#### `，其余行没有角色标记；单独解析
/// Markdown 无法区分这些续行和紧随其后的 assistant 正文。
pub fn parse_markdown_with_inputs(text: &str, inputs: &[String]) -> Vec<AiderMessage> {
    let mut out: Vec<AiderMessage> = Vec::new();
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    let mut ai_lines: Vec<String> = Vec::new();
    let mut have_user = false;
    let mut in_fence = false;
    let lines: Vec<&str> = text.lines().collect();
    let mut line_index = 0;
    let mut input_index = 0;

    while line_index < lines.len() {
        let line = lines[line_index];
        let trimmed = line.trim();
        if trimmed.starts_with("# aider chat started at") {
            flush_ai(&mut ai_lines, &mut out, &mut counts);
            have_user = false;
            line_index += 1;
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if have_user {
                ai_lines.push(line.to_string());
            }
            line_index += 1;
            continue;
        }
        if in_fence {
            if have_user {
                ai_lines.push(line.to_string());
            }
            line_index += 1;
            continue;
        }
        if trimmed.starts_with('>') {
            line_index += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("#### ") {
            flush_ai(&mut ai_lines, &mut out, &mut counts);
            let first_line = rest.trim();
            let matched_input = inputs
                .iter()
                .enumerate()
                .skip(input_index)
                .find(|(_, input)| input.lines().next().map(str::trim) == Some(first_line));
            let content = if let Some((matched_index, input)) = matched_input {
                let continuation: Vec<&str> = input.lines().skip(1).collect();
                let matches = continuation.iter().enumerate().all(|(offset, expected)| {
                    lines.get(line_index + offset + 1).copied() == Some(*expected)
                });
                if matches {
                    line_index += continuation.len();
                    input_index = matched_index + 1;
                    input.trim().to_string()
                } else {
                    first_line.to_string()
                }
            } else {
                first_line.to_string()
            };
            let idx = counts
                .entry(("human".to_string(), content.clone()))
                .or_insert(0);
            out.push(AiderMessage {
                role: "human".into(),
                content,
                occurrence_index: *idx,
            });
            *idx += 1;
            have_user = true;
            line_index += 1;
            continue;
        }
        if have_user {
            ai_lines.push(line.to_string());
        }
        line_index += 1;
    }
    flush_ai(&mut ai_lines, &mut out, &mut counts);
    out
}

/// 解析 prompt-toolkit `FileHistory`。一个条目以 `# <timestamp>` 开始，
/// 同一条多行输入的每一行都带 `+` 前缀；空行结束该条目。
pub fn parse_input_history(content: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in content.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            if !current.is_empty() {
                let entry = current.join("\n").trim().to_string();
                if !entry.is_empty() {
                    entries.push(entry);
                }
                current.clear();
            }
        } else if let Some(rest) = line.strip_prefix('+') {
            current.push(rest.to_string());
        }
    }
    if !current.is_empty() {
        let entry = current.join("\n").trim().to_string();
        if !entry.is_empty() {
            entries.push(entry);
        }
    }
    entries
}

fn flush_ai(
    ai_lines: &mut Vec<String>,
    out: &mut Vec<AiderMessage>,
    counts: &mut HashMap<(String, String), usize>,
) {
    let content = ai_lines.join("\n").trim().to_string();
    ai_lines.clear();
    if content.is_empty() {
        return;
    }
    let idx = counts
        .entry(("ai".to_string(), content.clone()))
        .or_insert(0);
    out.push(AiderMessage {
        role: "ai".into(),
        content,
        occurrence_index: *idx,
    });
    *idx += 1;
}

/// 稳定消息 ID：FNV-1a 64 位（非加密，仅用于会话内去重）。格式
/// `role + "\n" + trimmed(content) + "\n" + occurrence_index`。
pub fn stable_message_id(role: &str, content: &str, occurrence_index: usize) -> String {
    let input = format!("{role}\n{}\n{occurrence_index}", content.trim());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Default)]
struct BridgeState {
    synced_ids: HashSet<String>,
}

fn load_state(path: &Path) -> BridgeState {
    let Ok(text) = fs::read_to_string(path) else {
        return BridgeState::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BridgeState::default();
    };
    // 损坏或旧版本 schema：从空集合重新开始（会重放既有消息，但不丢数据——
    // dozerd 侧按 message_key 幂等）。
    if v.get("schema_version").and_then(|n| n.as_u64()) != Some(BRIDGE_SCHEMA_VERSION) {
        return BridgeState::default();
    }
    let synced = v
        .get("synced_ids")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    BridgeState { synced_ids: synced }
}

fn save_state(path: &Path, state: &BridgeState) -> Result<(), String> {
    let mut ids: Vec<&String> = state.synced_ids.iter().collect();
    ids.sort();
    let v = serde_json::json!({
        "schema_version": BRIDGE_SCHEMA_VERSION,
        "synced_ids": ids,
    });
    atomic_write(path, &v.to_string())
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("建目录失败: {e}"))?;
    }
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&tmp, content).map_err(|e| format!("写临时文件失败: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename 失败: {e}"))
}

fn read_history(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("读 history metadata 失败: {e}"))?;
    if meta.len() > MAX_HISTORY_BYTES {
        return Err("chat history 超过 16 MiB，停止同步".to_string());
    }
    fs::read_to_string(path).map_err(|e| format!("读 history 失败: {e}"))
}

fn truncate_content(content: &str) -> (String, bool) {
    if content.len() <= MAX_MESSAGE_BYTES {
        return (content.to_string(), false);
    }
    let mut end = MAX_MESSAGE_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = content[..end].to_string();
    truncated.push_str("…[truncated]");
    (truncated, true)
}

fn build_canonical_line(
    id: &str,
    role: &str,
    content: &str,
    ts_ms: u64,
    truncated: bool,
) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("schema_version".into(), serde_json::Value::from(1u64));
    obj.insert("type".into(), serde_json::Value::from("aider_message"));
    obj.insert("message_id".into(), serde_json::Value::from(id));
    obj.insert("role".into(), serde_json::Value::from(role));
    obj.insert("content".into(), serde_json::Value::from(content));
    obj.insert("ts_ms".into(), serde_json::Value::from(ts_ms));
    if truncated {
        obj.insert("truncated".into(), serde_json::Value::from(true));
    }
    serde_json::Value::Object(obj).to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 同步主入口：读取 `.chat.md`、解析、与 bridge state 比对、只追加新增完整
/// 消息到 canonical、原子更新 state。全程持 `<canonical>.lock` 文件锁串行化。
/// 返回本次新追加的行数。失败返回诊断字符串但不修改 Aider 原始 history。
pub fn sync(
    chat_md: &Path,
    input_history: &Path,
    canonical: &Path,
    bridge_state: &Path,
) -> Result<usize, String> {
    let lock_path = canonical.with_extension("lock");
    with_file_lock(&lock_path, || {
        let text = read_history(chat_md)?;
        let inputs = fs::read_to_string(input_history)
            .map(|text| parse_input_history(&text))
            .unwrap_or_default();
        let messages = parse_markdown_with_inputs(&text, &inputs);
        let mut state = load_state(bridge_state);

        let base_ts = now_ms();
        let mut new_lines: Vec<String> = Vec::new();
        let mut new_count = 0u64;
        for msg in &messages {
            let id = stable_message_id(&msg.role, &msg.content, msg.occurrence_index);
            if state.synced_ids.contains(&id) {
                continue;
            }
            let (content, truncated) = truncate_content(&msg.content);
            let ts_ms = base_ts + new_count;
            new_lines.push(build_canonical_line(
                &id, &msg.role, &content, ts_ms, truncated,
            ));
            state.synced_ids.insert(id);
            new_count += 1;
        }
        if new_lines.is_empty() {
            return Ok(0);
        }

        if let Some(parent) = canonical.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("建 canonical 目录失败: {e}"))?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(canonical)
            .map_err(|e| format!("打开 canonical 失败: {e}"))?;
        for line in &new_lines {
            writeln!(f, "{line}").map_err(|e| format!("写 canonical 失败: {e}"))?;
        }
        f.flush()
            .map_err(|e| format!("flush canonical 失败: {e}"))?;

        save_state(bridge_state, &state)?;
        Ok(new_lines.len())
    })
}

/// 对 `lock_path` 取 flock 排它锁执行 `f`，进程退出自动释放（无 stale lock）。
fn with_file_lock<T>(lock_path: &Path, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use std::os::unix::io::AsRawFd;
    if let Some(parent) = lock_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("打开锁文件失败: {e}"))?;
    let fd = file.as_raw_fd();
    if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
        return Err("获取文件锁失败".to_string());
    }
    let result = f();
    unsafe { libc::flock(fd, libc::LOCK_UN) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_reply_fixture() {
        let text = include_str!("../fixtures/aider/simple-reply.chat.md");
        let msgs = parse_markdown(text);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "human");
        assert_eq!(msgs[0].content, "Reply with exactly: hello world");
        assert_eq!(msgs[1].role, "ai");
        assert_eq!(msgs[1].content, "hello world");
    }

    #[test]
    fn parses_code_fence_with_inner_hashes_without_splitting() {
        let text = include_str!("../fixtures/aider/code-fence.chat.md");
        let msgs = parse_markdown(text);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "human");
        assert!(msgs[1].content.contains("#### this is not a user message"));
        assert!(!msgs[1].content.contains("Tokens:"), "blockquote 应被跳过");
    }

    #[test]
    fn skips_blockquotes_and_session_boundary() {
        let text = "# aider chat started at 2026-09-22 09:37:03\n\
                    > Aider v0.86.2\n\
                    > Model: openai/deepseek-v3\n\
                    #### 问一个问题\n\
                    回答内容\n\
                    > Tokens: 10 sent, 5 received.\n";
        let msgs = parse_markdown(text);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "human");
        assert_eq!(msgs[1].content, "回答内容");
    }

    #[test]
    fn multiple_chat_start_resets_context() {
        let text = "# aider chat started at 09:00\n\
                    #### 第一条\n\
                    回复一\n\
                    # aider chat started at 09:01\n\
                    #### 第二条\n\
                    回复二\n";
        let msgs = parse_markdown(text);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2].content, "第二条");
    }

    #[test]
    fn user_without_reply_produces_only_user() {
        let msgs = parse_markdown("#### 只有我\n");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "human");
    }

    #[test]
    fn restores_multiline_user_message_from_input_history() {
        let chat = "# aider chat started at 09:00\n\
                    #### 请完成以下工作\n\
                    1. 修改登录页\n\
                    2. 添加测试\n\
                    \n\
                    已完成。\n";
        let inputs = parse_input_history(
            "# 2026-09-22 09:00:00\n+请完成以下工作\n+1. 修改登录页\n+2. 添加测试\n",
        );
        let msgs = parse_markdown_with_inputs(chat, &inputs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "human");
        assert_eq!(
            msgs[0].content,
            "请完成以下工作\n1. 修改登录页\n2. 添加测试"
        );
        assert_eq!(msgs[1].role, "ai");
        assert_eq!(msgs[1].content, "已完成。");
    }

    #[test]
    fn input_history_groups_consecutive_plus_lines_into_one_entry() {
        let entries = parse_input_history(
            "\n# 2026-09-22 09:00:00\n+第一行\n+第二行\n\n# 2026-09-22 09:01:00\n+/help\n",
        );
        assert_eq!(entries, vec!["第一行\n第二行", "/help"]);
    }

    #[test]
    fn stable_message_id_is_deterministic_and_distinct_per_occurrence() {
        let a = stable_message_id("human", "hello", 0);
        let b = stable_message_id("human", "hello", 0);
        let c = stable_message_id("human", "hello", 1);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(
            stable_message_id("human", "hello", 0),
            stable_message_id("ai", "hello", 0)
        );
    }

    #[test]
    fn sync_appends_only_new_messages_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("s.chat.md");
        let canonical = dir.path().join("s.jsonl");
        let state = dir.path().join("s.bridge.json");
        let input = dir.path().join("s.input.history");
        fs::write(&chat, "# aider chat started at 09:00\n#### 问\n答\n").unwrap();

        assert_eq!(sync(&chat, &input, &canonical, &state).unwrap(), 2);
        // 重复同步不追加。
        assert_eq!(sync(&chat, &input, &canonical, &state).unwrap(), 0);

        let lines: Vec<String> = fs::read_to_string(&canonical)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(lines.len(), 2);
        let v0: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v0["type"], "aider_message");
        assert_eq!(v0["role"], "human");
    }

    #[test]
    fn sync_appends_only_one_round_of_new_messages() {
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("s.chat.md");
        let canonical = dir.path().join("s.jsonl");
        let state = dir.path().join("s.bridge.json");
        let input = dir.path().join("s.input.history");
        fs::write(&chat, "# aider chat started at 09:00\n#### 问\n答\n").unwrap();
        assert_eq!(sync(&chat, &input, &canonical, &state).unwrap(), 2);

        // 追加第二轮：只新增两行。
        fs::write(
            &chat,
            "# aider chat started at 09:00\n#### 问\n答\n#### 再问\n再答\n",
        )
        .unwrap();
        assert_eq!(sync(&chat, &input, &canonical, &state).unwrap(), 2);
        assert_eq!(fs::read_to_string(&canonical).unwrap().lines().count(), 4);
    }

    #[test]
    fn sync_truncates_oversized_message() {
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("s.chat.md");
        let canonical = dir.path().join("s.jsonl");
        let state = dir.path().join("s.bridge.json");
        let input = dir.path().join("s.input.history");
        let big = "x".repeat(MAX_MESSAGE_BYTES + 100);
        fs::write(
            &chat,
            format!("# aider chat started at 09:00\n#### q\n{big}\n"),
        )
        .unwrap();
        sync(&chat, &input, &canonical, &state).unwrap();
        let lines: Vec<String> = fs::read_to_string(&canonical)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let last: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(last["truncated"], true);
        assert!(
            last["content"]
                .as_str()
                .unwrap()
                .is_char_boundary(last["content"].as_str().unwrap().len())
        );
    }

    #[test]
    fn truncates_non_ascii_content_on_utf8_boundary() {
        let content = "你".repeat(MAX_MESSAGE_BYTES / 3 + 10);
        let (truncated, did_truncate) = truncate_content(&content);
        assert!(did_truncate);
        assert!(truncated.ends_with("…[truncated]"));
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn sync_rejects_oversized_history() {
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("s.chat.md");
        let canonical = dir.path().join("s.jsonl");
        let state = dir.path().join("s.bridge.json");
        let input = dir.path().join("s.input.history");
        let big = "x".repeat((MAX_HISTORY_BYTES + 1) as usize);
        fs::write(&chat, big).unwrap();
        assert!(sync(&chat, &input, &canonical, &state).is_err());
    }

    #[test]
    fn concurrent_sync_does_not_duplicate() {
        // 两个"进程"串行 sync 同一份 history,结果仍是 2 行(flock 串行化)。
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("s.chat.md");
        let canonical = dir.path().join("s.jsonl");
        let state = dir.path().join("s.bridge.json");
        let input = dir.path().join("s.input.history");
        fs::write(&chat, "# aider chat started at 09:00\n#### 问\n答\n").unwrap();
        sync(&chat, &input, &canonical, &state).unwrap();
        sync(&chat, &input, &canonical, &state).unwrap();
        sync(&chat, &input, &canonical, &state).unwrap();
        assert_eq!(fs::read_to_string(&canonical).unwrap().lines().count(), 2);
    }
}
