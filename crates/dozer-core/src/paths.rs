use directories::ProjectDirs;
use std::path::PathBuf;

fn dirs() -> ProjectDirs {
    ProjectDirs::from("ai", "byteboy", "dozer").expect("home directory must exist")
}

/// ~/Library/Application Support 或 XDG config 下的 dozer 配置目录
pub fn config_dir() -> PathBuf {
    dirs().config_dir().to_path_buf()
}

/// 运行态（PID、socket、滚屏缓存）目录
pub fn state_dir() -> PathBuf {
    dirs()
        .state_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dirs().data_local_dir().to_path_buf())
}

/// dozerd 的 Unix Domain Socket 路径
pub fn socket_path() -> PathBuf {
    state_dir().join("dozerd.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_dozer() {
        assert!(config_dir().to_string_lossy().contains("dozer"));
    }

    #[test]
    fn state_dir_ends_with_dozer() {
        assert!(state_dir().to_string_lossy().contains("dozer"));
    }

    #[test]
    fn socket_path_is_under_state_dir() {
        let s = socket_path();
        assert!(s.starts_with(state_dir()));
        assert_eq!(s.file_name().unwrap(), "dozerd.sock");
    }
}
