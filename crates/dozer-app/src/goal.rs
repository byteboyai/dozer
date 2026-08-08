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

/// 按固定格式整份重写 `.dozer/goal.md`:首行 `# {title}`,空行,然后每条
/// 标准各占一行 `- [ ] {criterion}`。不保留/不合并文件里其它手写内容——
/// `parse_goal` 本来就只认标题行与 `- [ ]`/`- [x]` 列表项这两种语义,重新
/// 生成不算破坏数据(见设计文档"非目标")。`.dozer` 目录不存在则先创建。
pub fn write_goal(repo: &Path, goal: &Goal) -> std::io::Result<()> {
    let path = goal_path(repo);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut md = format!("# {}\n\n", goal.title);
    for c in &goal.criteria {
        md.push_str(&format!("- [ ] {c}\n"));
    }
    std::fs::write(path, md)
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

    #[test]
    fn write_goal_creates_dozer_dir_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let goal = Goal {
            title: "标题".into(),
            criteria: vec!["标准一".into()],
        };
        write_goal(dir.path(), &goal).unwrap();
        let content = std::fs::read_to_string(goal_path(dir.path())).unwrap();
        assert_eq!(content, "# 标题\n\n- [ ] 标准一\n");
    }

    #[test]
    fn write_goal_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let g1 = Goal {
            title: "旧标题".into(),
            criteria: vec!["旧标准".into()],
        };
        write_goal(dir.path(), &g1).unwrap();
        let g2 = Goal {
            title: "新标题".into(),
            criteria: vec![],
        };
        write_goal(dir.path(), &g2).unwrap();
        let content = std::fs::read_to_string(goal_path(dir.path())).unwrap();
        assert_eq!(content, "# 新标题\n\n");
    }

    #[test]
    fn write_then_parse_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let goal = Goal {
            title: "圆环测试".into(),
            criteria: vec!["一".into(), "二".into()],
        };
        write_goal(dir.path(), &goal).unwrap();
        let content = std::fs::read_to_string(goal_path(dir.path())).unwrap();
        let parsed = parse_goal(&content).unwrap();
        assert_eq!(parsed, goal);
    }
}
