//! 文本保存(文件预览重构 Phase B Task 4):同目录临时文件 + rename 原子替换,
//! 并保持原文件的 BOM / 换行约定。
//!
//! 编辑器(CodeMirror)内部正文统一用 `\n` 分隔、且 `fetch(...).text()` 会
//! 丢掉 UTF-8 BOM。保存时按**磁盘原件**的约定复原:原文件有 BOM 就补回,
//! 原来以 CRLF 为主就转回 CRLF。

use std::path::Path;

use dozer_file_io::{
    AtomicWriteError, AtomicWriteOutcome, ProjectRoot, Revision, WriteOptions, write_atomic,
};

/// 预览保存的提交结果。`CommittedDurabilityUnknown` 表示内容已经替换,只是
/// 目录同步失败;UI 可以标记为已保存,但需要提示持久性未确认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Committed,
    CommittedDurabilityUnknown(String),
}

/// 从原文件字节采样出的文本约定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileTextFormat {
    pub bom: bool,
    pub crlf: bool,
}

/// 检测原文件的 BOM / 换行约定(只扫前 64KiB,足够;大文件也不会因此变慢)。
pub fn detect_format(original: &[u8]) -> FileTextFormat {
    let bom = original.starts_with(&[0xEF, 0xBB, 0xBF]);
    let sample = &original[..original.len().min(64 * 1024)];
    let mut crlf = 0usize;
    let mut lf_only = 0usize;
    let mut i = 0;
    while i < sample.len() {
        if sample[i] == b'\r' && sample.get(i + 1) == Some(&b'\n') {
            crlf += 1;
            i += 2;
            continue;
        }
        if sample[i] == b'\n' {
            lf_only += 1;
        }
        i += 1;
    }
    FileTextFormat {
        bom,
        crlf: crlf > lf_only,
    }
}

/// 把编辑器正文(以 `\n` 分隔)按 `fmt` 编码成待写盘字节。
pub fn encode_for_save(text: &str, fmt: FileTextFormat) -> Vec<u8> {
    // 先归一化任何残留 CRLF,避免重复转换;再按需转回 CRLF。
    let normalized = text.replace("\r\n", "\n");
    let body = if fmt.crlf {
        normalized.replace('\n', "\r\n")
    } else {
        normalized
    };
    let mut out = Vec::with_capacity(body.len() + 3);
    if fmt.bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// 兼容包装:没有项目根上下文,只能把目标父目录当作根。生产预览保存必须
/// 使用 [`save_text_with_context`] 并传真实项目根,否则父目录符号链接会把
/// 外部目录变成新的授权根。
pub fn save_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    save_text_with_context(path, text, None, None).map(|_| ())
}

/// 带项目根/锁根/提交结果的新保存入口。`project_root` 存在时,目标必须
/// 是它的词法相对路径,并由 `ProjectRoot` 逐级拒绝符号链接;`lock_root`
/// 为 `None` 时使用用户隔离的临时锁根。
pub fn save_text_with_context(
    path: &Path,
    text: &str,
    project_root: Option<&Path>,
    lock_root: Option<&Path>,
) -> std::io::Result<SaveOutcome> {
    let (root, relative) = match project_root {
        Some(project_root) => {
            let root = ProjectRoot::open(project_root).map_err(file_error_to_io)?;
            let relative = path.strip_prefix(project_root).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "target is outside the project root",
                )
            })?;
            let relative = relative.to_str().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "target path is not UTF-8")
            })?;
            (root, relative.to_string())
        }
        None => {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let root = ProjectRoot::open(parent).map_err(file_error_to_io)?;
            let file_name = path.file_name().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "target path has no file name",
                )
            })?;
            let file_name = file_name.to_str().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "target file name is not UTF-8",
                )
            })?;
            (root, file_name.to_string())
        }
    };

    // 只采样前 64KiB;大文件不因为探测格式而整读一遍。NotFound 与权限/IO
    // 错误区别处理:后者不能回落成新文件默认格式。
    let (format, expected_revision) = match root.open_file(&relative) {
        Ok(opened) => {
            let revision = Revision::of(&opened.metadata);
            let sample = opened.read_prefix(64 * 1024).map_err(file_error_to_io)?;
            (detect_format(&sample), Some(revision))
        }
        Err(dozer_file_io::FileError::NotFound) => (FileTextFormat::default(), None),
        Err(error) => return Err(file_error_to_io(error)),
    };
    let bytes = encode_for_save(text, format);
    let options = WriteOptions {
        lock_root,
        expected_revision,
        create: true,
        ..WriteOptions::default()
    };
    match write_atomic(&root, &relative, &bytes, &options) {
        Ok(AtomicWriteOutcome::Committed { .. }) => Ok(SaveOutcome::Committed),
        Ok(AtomicWriteOutcome::CommittedDurabilityUnknown { warning, .. }) => {
            Ok(SaveOutcome::CommittedDurabilityUnknown(warning))
        }
        Err(AtomicWriteError::NotCommitted(error)) => Err(file_error_to_io(error)),
        Err(AtomicWriteError::CommittedDurabilityUnknown { warning, .. }) => {
            Ok(SaveOutcome::CommittedDurabilityUnknown(warning))
        }
    }
}

fn file_error_to_io(error: dozer_file_io::FileError) -> std::io::Error {
    match error {
        dozer_file_io::FileError::NotFound => {
            std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string())
        }
        dozer_file_io::FileError::Conflict => {
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, error.to_string())
        }
        dozer_file_io::FileError::Io(io) => io,
        other => std::io::Error::other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_bom_and_crlf() {
        assert_eq!(
            detect_format(b"\xEF\xBB\xBFa\r\nb\r\n"),
            FileTextFormat {
                bom: true,
                crlf: true
            }
        );
        assert_eq!(
            detect_format(b"a\nb\n"),
            FileTextFormat {
                bom: false,
                crlf: false
            }
        );
    }

    #[test]
    fn mixed_endings_prefer_majority() {
        // 2 个 CRLF vs 1 个 LF → CRLF。
        assert!(detect_format(b"a\r\nb\r\nc\n").crlf);
        // 2 个 LF vs 1 个 CRLF → LF。
        assert!(!detect_format(b"a\nb\nc\r\n").crlf);
    }

    #[test]
    fn encode_round_trips_bom_and_crlf() {
        let fm = FileTextFormat {
            bom: true,
            crlf: true,
        };
        let bytes = encode_for_save("a\nb\n", fm);
        assert_eq!(bytes, b"\xEF\xBB\xBFa\r\nb\r\n");
        // 已是 CRLF 的输入不会变成 CRCRLF。
        let bytes = encode_for_save("a\r\nb\r\n", fm);
        assert_eq!(bytes, b"\xEF\xBB\xBFa\r\nb\r\n");
    }

    #[test]
    fn encode_plain_lf() {
        assert_eq!(
            encode_for_save("x\ny", FileTextFormat::default()),
            b"x\ny".to_vec()
        );
    }

    #[test]
    fn atomic_save_preserves_original_format_and_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"\xEF\xBB\xBFfn main() {}\r\n").unwrap();
        }
        save_text_atomic(&path, "fn main() {\n    let x = 1;\n}\n").unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM 保留");
        let text = String::from_utf8(saved).unwrap();
        assert!(text.contains("let x = 1;"));
        assert!(!text.contains("\n\n"), "换行应为 CRLF");
        assert!(text.matches("\r\n").count() >= 3);
        // 临时文件不残留。
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("dozer-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件应已 rename 走");
    }

    #[test]
    fn atomic_save_creates_when_missing_with_lf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        save_text_atomic(&path, "hello\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello\n");
    }

    /// 目标所在目录不可写:原子保存(先写同目录临时文件)失败,原文件字节不变。
    #[cfg(unix)]
    #[test]
    fn save_in_a_read_only_directory_fails_and_leaves_the_original_bytes_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"original\n").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = save_text_atomic(&path, "changed\n");
        // 还原权限,免得临时目录清理失败。
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        // 以 root 运行时目录权限不起作用;只在确实被拒绝时断言「原文件不变」。
        if result.is_err() {
            assert_eq!(std::fs::read(&path).unwrap(), b"original\n");
        }
    }

    #[test]
    fn save_into_a_missing_directory_returns_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-such-dir").join("a.txt");
        assert!(save_text_atomic(&path, "x").is_err());
        assert!(!path.exists());
        assert!(!dir.path().join("no-such-dir").exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_executable_and_private_modes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for mode in [0o755, 0o600] {
            let path = dir.path().join(format!("m{mode}.sh"));
            std::fs::write(&path, b"old").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            save_text_atomic(&path, "new").unwrap();
            let saved_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(saved_mode, mode, "保存必须保持原访问/执行位");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        }
    }

    #[cfg(unix)]
    #[test]
    fn save_rejects_symlink_target_without_touching_outside_file() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "outside").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&outside_file, &link).unwrap();
        assert!(save_text_atomic(&link, "changed").is_err());
        assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "outside");
    }

    #[test]
    fn save_with_context_reports_committed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"old\n").unwrap();
        let outcome = save_text_with_context(&path, "new\n", Some(dir.path()), None).unwrap();
        assert_eq!(outcome, SaveOutcome::Committed);
        assert_eq!(std::fs::read(&path).unwrap(), b"new\n");
    }

    #[cfg(unix)]
    #[test]
    fn save_with_project_root_rejects_parent_symlink_escape() {
        use std::os::unix::fs::symlink;
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), b"outside\n").unwrap();
        symlink(outside.path(), project.path().join("link")).unwrap();
        let result = save_text_with_context(
            &project.path().join("link/a.txt"),
            "changed\n",
            Some(project.path()),
            None,
        );
        assert!(result.is_err(), "父目录符号链接必须被拒绝");
        assert_eq!(
            std::fs::read(outside.path().join("a.txt")).unwrap(),
            b"outside\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_with_context_does_not_create_lock_files_in_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"old\n").unwrap();
        save_text_with_context(&path, "new\n", Some(dir.path()), None).unwrap();
        let lock_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".dozer-write-")
            })
            .collect();
        assert!(lock_files.is_empty(), "锁文件不应落在用户项目里");
    }

    #[test]
    fn save_conflicts_when_another_writer_changed_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"old\n").unwrap();
        let opened = dozer_file_io::ProjectRoot::open(dir.path())
            .unwrap()
            .open_file("a.txt")
            .unwrap();
        let expected = dozer_file_io::Revision::of(&opened.metadata);
        std::fs::write(&path, b"external\n").unwrap();
        let result = dozer_file_io::write_atomic(
            &dozer_file_io::ProjectRoot::open(dir.path()).unwrap(),
            "a.txt",
            b"new\n",
            &dozer_file_io::WriteOptions {
                expected_revision: Some(expected),
                ..dozer_file_io::WriteOptions::default()
            },
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external\n");
    }
}
