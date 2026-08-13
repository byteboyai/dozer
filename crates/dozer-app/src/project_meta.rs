//! 项目描述:`.dozer/description.md`,纯文本,无格式约束。跟 `goal.rs` 平级
//! ——目标和描述是两种独立数据,只是都挂在 `.dozer/` 下。

use std::path::{Path, PathBuf};

fn description_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("description.md")
}

/// 文件不存在或内容 trim 后为空 → `None`。
pub fn load_description(repo: &Path) -> Option<String> {
    let text = std::fs::read_to_string(description_path(repo)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 整份重写。空字符串仍然写入一个空文件(不是删除文件)——跟
/// `load_description` 的"trim 后为空 → None"配合,行为等价于清空。
pub fn write_description(repo: &Path, text: &str) -> std::io::Result<()> {
    let path = description_path(repo);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "这是一段描述").unwrap();
        assert_eq!(
            load_description(dir.path()),
            Some("这是一段描述".to_string())
        );
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_description(dir.path()), None);
    }

    #[test]
    fn load_whitespace_only_is_none() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "   \n  ").unwrap();
        assert_eq!(load_description(dir.path()), None);
    }

    #[test]
    fn write_creates_dozer_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "x").unwrap();
        assert!(description_path(dir.path()).exists());
    }

    #[test]
    fn write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "旧").unwrap();
        write_description(dir.path(), "新").unwrap();
        assert_eq!(load_description(dir.path()), Some("新".to_string()));
    }
}
