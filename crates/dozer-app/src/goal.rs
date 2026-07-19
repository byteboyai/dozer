//! `.dozer/goal.md` 定标文件解析（spec P1f D2）：
//! 第一个非空行=目标（去掉行首 `#` 与空白）；`- [ ]`/`- [x]` 列表项=验收标准。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub title: String,
    pub criteria: Vec<String>,
}

pub fn goal_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("goal.md")
}

pub fn parse_goal(md: &str) -> Option<Goal> {
    let mut title: Option<String> = None;
    let mut criteria = Vec::new();
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- [ ]")
            .or_else(|| trimmed.strip_prefix("- [x]"))
        {
            criteria.push(rest.trim().to_string());
            continue;
        }
        if title.is_none() {
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        }
    }
    title.map(|title| Goal { title, criteria })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_and_criteria() {
        let md = "# 让预览支持 PDF\n\n说明文字。\n\n- [ ] cargo test 全绿\n- [x] 打开 PDF 不白屏\n- 普通列表项不算标准\n";
        let g = parse_goal(md).unwrap();
        assert_eq!(g.title, "让预览支持 PDF");
        assert_eq!(g.criteria, vec!["cargo test 全绿", "打开 PDF 不白屏"]);
    }

    #[test]
    fn title_without_hash_and_no_criteria() {
        let g = parse_goal("裸标题目标\n正文").unwrap();
        assert_eq!(g.title, "裸标题目标");
        assert!(g.criteria.is_empty());
    }

    #[test]
    fn blank_input_is_none() {
        assert!(parse_goal("").is_none());
        assert!(parse_goal("  \n\n \n").is_none());
    }

    #[test]
    fn goal_path_is_dot_dozer() {
        assert_eq!(
            goal_path(std::path::Path::new("/repo")),
            std::path::PathBuf::from("/repo/.dozer/goal.md")
        );
    }
}
