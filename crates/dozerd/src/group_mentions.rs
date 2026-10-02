//! 群聊 `@` 解析(纯函数,spec 2026-10-02-group-chat-panel-design §7)。
//!
//! 规则:`@` 前一个字符不能是 ASCII 字母数字/`_`/`.`/`-`(排除 `a@b.com`);
//! handle 由"非空白、非 `@`、非终止标点"的字符组成,遇到终止标点(含全角)
//! 即结束;匹配忽略大小写;同一成员多次 `@` 只记第一次的位置;不认识的
//! handle 收进 `unknown`(只提示,不触发任何人)。

const HANDLE_MAX_CHARS: usize = 32;

/// 终止 handle 的标点(含全角)。`validate_handle` 同样禁用它们,保证
/// "能存进去的 handle 一定能被解析出来"。
const TERMINATORS: &[char] = &[
    ',', '.', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '<', '>', '"', '\'', '`', '，',
    '。', '；', '：', '！', '？', '（', '）', '、', '「', '」',
];

fn is_handle_char(c: char) -> bool {
    !c.is_whitespace() && c != '@' && !TERMINATORS.contains(&c)
}

/// 群内唯一性与匹配用的键(忽略大小写)。
pub fn handle_key(handle: &str) -> String {
    handle.to_lowercase()
}

/// 校验成员 handle。`Err` 是可直接展示给用户的中文原因。
pub fn validate_handle(handle: &str) -> Result<(), String> {
    if handle.is_empty() {
        return Err("名称不能为空".into());
    }
    if handle.chars().count() > HANDLE_MAX_CHARS {
        return Err(format!("名称不能超过 {HANDLE_MAX_CHARS} 个字符"));
    }
    if let Some(bad) = handle.chars().find(|c| !is_handle_char(*c)) {
        return Err(format!("名称不能包含 {bad:?}"));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq)]
pub struct MentionParse {
    /// 被点名成员 id,按首次出现顺序,已去重。
    pub members: Vec<i64>,
    /// 没有对应成员的 handle(原文写法,已去重)。
    pub unknown: Vec<String>,
}

/// `roster` 是 `(member_id, handle)`。
pub fn parse_mentions(text: &str, roster: &[(i64, &str)]) -> MentionParse {
    let mut out = MentionParse::default();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        let prev_ok = i == 0
            || !(chars[i - 1].is_ascii_alphanumeric() || matches!(chars[i - 1], '_' | '.' | '-'));
        let mut j = i + 1;
        while j < chars.len() && is_handle_char(chars[j]) {
            j += 1;
        }
        if prev_ok && j > i + 1 {
            let token: String = chars[i + 1..j].iter().collect();
            let key = handle_key(&token);
            match roster.iter().find(|(_, h)| handle_key(h) == key) {
                Some((id, _)) => {
                    if !out.members.contains(id) {
                        out.members.push(*id);
                    }
                }
                None => {
                    if !out.unknown.contains(&token) {
                        out.unknown.push(token);
                    }
                }
            }
        }
        i = j.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> Vec<(i64, &'static str)> {
        vec![(1, "claude"), (2, "Codex"), (3, "架构师")]
    }

    #[test]
    fn mentions_follow_first_occurrence_order_and_dedupe() {
        let p = parse_mentions("@codex 先说，然后 @claude，最后 @Codex 补充", &roster());
        assert_eq!(p.members, vec![2, 1]);
        assert!(p.unknown.is_empty());
    }

    #[test]
    fn matching_ignores_case() {
        let p = parse_mentions("@CLAUDE 你好", &roster());
        assert_eq!(p.members, vec![1]);
    }

    #[test]
    fn full_width_punctuation_terminates_handle() {
        let p = parse_mentions("@架构师，看下；@claude。", &roster());
        assert_eq!(p.members, vec![3, 1]);
    }

    #[test]
    fn email_is_not_a_mention() {
        let p = parse_mentions("联系 foo@claude.com 或 a_b@codex", &roster());
        assert!(p.members.is_empty());
        assert!(p.unknown.is_empty());
    }

    #[test]
    fn unknown_handle_is_reported_once_and_does_not_trigger() {
        let p = parse_mentions("@Override 和 @Override 还有 @nobody", &roster());
        assert!(p.members.is_empty());
        assert_eq!(
            p.unknown,
            vec!["Override".to_string(), "nobody".to_string()]
        );
    }

    #[test]
    fn mention_right_after_chinese_char_counts() {
        let p = parse_mentions("你好@claude", &roster());
        assert_eq!(p.members, vec![1]);
    }

    #[test]
    fn bare_at_sign_is_ignored() {
        let p = parse_mentions("@ 单独的 @ 和 @@claude", &roster());
        assert_eq!(p.members, vec![1]);
        assert!(p.unknown.is_empty());
    }

    #[test]
    fn validate_handle_rules() {
        assert!(validate_handle("claude").is_ok());
        assert!(validate_handle("架构师").is_ok());
        assert!(validate_handle("").is_err());
        assert!(validate_handle("a b").is_err());
        assert!(validate_handle("a@b").is_err());
        assert!(validate_handle("评审，员").is_err());
        assert!(validate_handle(&"x".repeat(33)).is_err());
    }
}
