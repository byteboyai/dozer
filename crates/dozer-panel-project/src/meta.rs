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

    #[test]
    fn load_description_returns_none_when_dir_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_description(&dir.path().join("never-created")),
            None,
            "项目目录已被移走/删除时读描述得 None,不 panic"
        );
    }

    #[test]
    fn write_description_fails_cleanly_when_parent_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        // `.dozer` 被占成一个普通文件:`create_dir_all` 必然失败,调用方拿到 Err 而不是 panic。
        std::fs::write(dir.path().join(".dozer"), "x").unwrap();
        let result = write_description(dir.path(), "新描述");
        assert!(result.is_err());
    }

    #[test]
    fn load_description_returns_none_for_non_utf8() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(
            dir.path().join(".dozer").join("description.md"),
            [0xff, 0xfe],
        )
        .unwrap();
        assert_eq!(load_description(dir.path()), None);
    }
}
