//! 项目描述:`.dozer/description.md`。读写实现已搬进 `dozer-panel-project::meta`,
//! 这里只保留原路径转调(避免改所有调用点)。

pub use dozer_panel_project::meta::{load_description, write_description};

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
        assert!(dir.path().join(".dozer").join("description.md").exists());
    }

    #[test]
    fn write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        write_description(dir.path(), "旧").unwrap();
        write_description(dir.path(), "新").unwrap();
        assert_eq!(load_description(dir.path()), Some("新".to_string()));
    }
}
