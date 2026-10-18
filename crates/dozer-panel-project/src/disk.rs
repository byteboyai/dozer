//! 项目根目录的尺寸统计:用量徽标与预览空白页共用同一口径。

use std::path::Path;

/// 磁盘占用统计的排除名单——跟文件树的"要不要显示这一行"语义不同,这里是
/// "算不算项目真实内容",不复用文件树那份常量。
pub const DISK_USAGE_EXCLUDE: [&str; 7] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "__pycache__",
];

/// 递归求和 `root` 下所有文件大小,跳过名字命中 `exclude` 的目录(整个子树
/// 跳过,不下钻)。读不到的条目(权限/符号链接死链)跳过不计入,不中断整体
/// 计算。
pub fn dir_size_excluding(root: &Path, exclude: &[&str]) -> u64 {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if exclude.contains(&name.as_str()) {
                continue;
            }
            total += dir_size_excluding(&entry.path(), exclude);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// 项目根目录简介(预览空白页用):总字节数 + 文件计数 + `fs::metadata` 拿的
/// ctime/mtime。字段与原 Dozer `preview::BlankPaneInfo` 一一对应;这里是库自己的
/// 小结构,不让预览模块的私有类型穿越库边界。
#[derive(Debug, Clone, PartialEq)]
pub struct BlankInfo {
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub file_count: u64,
    pub created: Option<std::time::SystemTime>,
    pub modified: Option<std::time::SystemTime>,
}

/// 递归求总字节数 + 文件计数。`dir_size_excluding` 的近亲,但要同时给出两份数。
fn dir_size_and_count(root: &Path, exclude: &[&str]) -> (u64, u64) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return (0, 0);
    };
    let mut total_size = 0u64;
    let mut total_count = 0u64;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if exclude.contains(&name.as_str()) {
                continue;
            }
            let (s, c) = dir_size_and_count(&entry.path(), exclude);
            total_size += s;
            total_count += c;
        } else if let Ok(meta) = entry.metadata() {
            total_size += meta.len();
            total_count += 1;
        }
    }
    (total_size, total_count)
}

/// 空白页(`PreviewPane::blank_info`)用的项目根目录简介:递归求总字节数
/// + 文件计数 + `fs::metadata(root)` 拿 ctime/mtime。
///
/// 复用 [`DISK_USAGE_EXCLUDE`],与"用量徽章"统计口径一致——`target/`、
/// `.git/`、`node_modules` 等不计入。
///
/// 读不到的子项静默跳过,不破坏外层结果:大仓库上难免有 `EACCES` / 死链,
/// 中断会让卡片永久卡在"计算中"。
///
/// `created` / `modified` 是平台 syscall,macOS 全支持;某些 sandboxed
/// 环境会回 `Err`,此时对应字段为 `None`,view 用 `—` 占位。
pub fn compute_blank_info(root: &Path) -> BlankInfo {
    let (size_bytes, file_count) = dir_size_and_count(root, &DISK_USAGE_EXCLUDE);
    let (created, modified) = match std::fs::metadata(root) {
        Ok(md) => (md.created().ok(), md.modified().ok()),
        Err(_) => (None, None),
    };
    BlankInfo {
        path: root.to_path_buf(),
        size_bytes,
        file_count,
        created,
        modified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_size_excluding_sums_files_and_skips_excluded_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "12345").unwrap(); // 5 bytes
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("big.bin"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("b.txt"), "12").unwrap(); // 2 bytes
        assert_eq!(dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE), 7);
    }

    #[test]
    fn dir_size_excluding_empty_dir_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE), 0);
    }

    #[test]
    fn compute_blank_info_excludes_git_target_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "123").unwrap();
        for excluded in [".git", "target", "node_modules"] {
            std::fs::create_dir(dir.path().join(excluded)).unwrap();
            std::fs::write(dir.path().join(excluded).join("x"), "0000000").unwrap();
        }
        let info = compute_blank_info(dir.path());
        assert_eq!(info.path, dir.path());
        assert_eq!(info.size_bytes, 3);
        assert_eq!(info.file_count, 1);
    }

    #[test]
    fn compute_blank_info_missing_root_returns_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let info = compute_blank_info(&dir.path().join("nope"));
        assert_eq!(info.size_bytes, 0);
        assert_eq!(info.file_count, 0);
        assert!(info.created.is_none());
        assert!(info.modified.is_none());
    }
}
