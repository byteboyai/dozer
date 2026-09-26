//! 总结管线的输入契约与 revision 哈希(spec 2026-09-26 第 4、6 节)。
//!
//! 规范快照是从 `TurnRecord` 生成的稳定文本,`revision_hash` 是对它的确定
//! 性哈希——同一份 transcript 内容(含角色、顺序、正文、工具记录)在任意
//! 进程、任意时刻算出相同的 revision,daemon 重启也不会让结果凭空变 stale。

use dozer_core::protocol::TurnRecord;

/// 总结管线版本。改变分块/归并/抽取 prompt 等"影响产出版本"的算法时 +1,
/// 旧 pipeline_version 的结果在下次修复时会被重新生成(计入"stale")。
pub const PIPELINE_VERSION: &str = "v1";

/// 把一段回合记录规范化成稳定文本:每行一个回合,字段用制表符分隔,顺序
/// 固定(role → thinking → is_error → 工具调用摘要 → 正文)。隐藏 thinking
/// 的正文不进哈希(那部分在抽取时被过滤,不影响产出的语义输入),但 thinking
/// 标志本身要进哈希,让"有没有 thinking"影响 revision。
pub fn canonicalize_turns(turns: &[TurnRecord]) -> String {
    let mut out = String::new();
    for t in turns {
        let tools = t
            .tool_calls
            .iter()
            .map(|c| {
                c.input_json
                    .as_deref()
                    .map(|j| format!("{}:{}", c.summary, j))
                    .unwrap_or_else(|| c.summary.clone())
            })
            .collect::<Vec<_>>()
            .join(";");
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            t.role,
            if t.thinking { "1" } else { "0" },
            if t.is_error { "1" } else { "0" },
            tools,
            t.content,
        ));
    }
    out
}

/// FNV-1a 64 位确定性哈希,输出 16 位小写 hex。不引入 sha2 依赖——revision
/// 只要求"同内容同哈希、不同内容几乎不碰撞",不需要密码学强度;跨进程稳定
/// 是硬要求,`std::hash::DefaultHasher`(SipHash 带随机 seed)不满足。
pub fn revision_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 从回合记录直接算 `source_revision`(规范快照 → 哈希两步合一)。
pub fn revision_of(turns: &[TurnRecord]) -> String {
    revision_hash(&canonicalize_turns(turns))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, content: &str) -> TurnRecord {
        TurnRecord {
            turn_index: 0,
            role: role.into(),
            content: content.into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(1),
            is_error: false,
            tool_result_call_id: None,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        }
    }

    #[test]
    fn revision_is_deterministic() {
        let turns = vec![turn("human", "改一下 README"), turn("ai", "已改")];
        assert_eq!(revision_of(&turns), revision_of(&turns));
    }

    #[test]
    fn revision_changes_with_content() {
        let a = vec![turn("human", "改一下 README")];
        let b = vec![turn("human", "改一下 README 加安装说明")];
        assert_ne!(revision_of(&a), revision_of(&b));
    }

    #[test]
    fn revision_changes_with_role_order() {
        let a = vec![turn("human", "x"), turn("ai", "y")];
        let b = vec![turn("ai", "y"), turn("human", "x")];
        assert_ne!(revision_of(&a), revision_of(&b));
    }

    #[test]
    fn thinking_flag_affects_revision_but_not_content() {
        let mut t = turn("ai", "思考内容");
        t.thinking = false;
        let a = revision_of(&[t.clone()]);
        t.thinking = true;
        let b = revision_of(&[t.clone()]);
        assert_ne!(a, b);
    }

    #[test]
    fn revision_hash_is_stable_hex() {
        let h = revision_hash("hello");
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
