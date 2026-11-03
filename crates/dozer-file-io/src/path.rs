//! 项目根与受控路径打开。
//!
//! 项目根先规范化并固定目录句柄;每一级目录用 `openat(..., O_NOFOLLOW |
//! O_DIRECTORY)` 打开,末级文件用 `O_NOFOLLOW` 打开并检查是普通文件。这样
//! 路径检查与真正打开是同一个对象,不存在“先 canonicalize、再按旧路径重开”
//! 的替换窗口。

use crate::{FileError, FileMetadata, Result};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

/// 一个已固定的项目根。`canonical_path` 只用于展示/锁根定位,真正打开文件
/// 始终从目录句柄出发。
#[derive(Debug)]
pub struct ProjectRoot {
    canonical_path: PathBuf,
    #[cfg(unix)]
    dir: File,
}

/// 已受控打开的文件。
#[derive(Debug)]
pub struct OpenedFile {
    file: File,
    pub metadata: FileMetadata,
    pub relative_path: PathBuf,
}

impl OpenedFile {
    pub fn read_all(mut self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub fn read_limited(mut self, max: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.file
            .by_ref()
            .take(max.saturating_add(1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > max {
            return Err(FileError::Limit(format!("file exceeds {max} bytes")));
        }
        Ok(bytes)
    }
    pub fn read_prefix(mut self, max: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.file
            .by_ref()
            .take(max as u64)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub fn file(&self) -> &File {
        &self.file
    }
}

/// 目标文件的父目录句柄 + 末级名字。写入路径需要它来执行 `renameat`。
#[derive(Debug)]
pub struct ParentHandle {
    dir: File,
    name: OsString,
    /// 父目录的规范化路径(从项目根拼接,仅用于默认锁根/诊断)。
    pub parent_path: PathBuf,
    pub relative_path: PathBuf,
}

impl ParentHandle {
    pub fn dir(&self) -> &File {
        &self.dir
    }

    pub fn name(&self) -> &OsStr {
        &self.name
    }
}

impl ProjectRoot {
    pub fn open(root: &Path) -> Result<Self> {
        let canonical_path = root.canonicalize().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                FileError::NotFound
            } else {
                FileError::Io(error)
            }
        })?;
        if !canonical_path.is_dir() {
            return Err(FileError::UnsupportedFile("项目根不是目录".into()));
        }
        #[cfg(unix)]
        {
            let dir = File::open(&canonical_path)?;
            Ok(Self {
                canonical_path,
                dir,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self { canonical_path })
        }
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    /// 受控打开一个项目内相对路径。
    pub fn open_file(&self, relative: &str) -> Result<OpenedFile> {
        let components = relative_components(relative)?;
        #[cfg(unix)]
        {
            let (parent, name) = self.open_parent_components(&components)?;
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err(open_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let metadata = FileMetadata::from_metadata(&file.metadata()?);
            if !metadata.is_regular_file() {
                return Err(FileError::UnsupportedFile("目标不是普通文件".into()));
            }
            Ok(OpenedFile {
                file,
                metadata,
                relative_path: components.iter().collect(),
            })
        }
        #[cfg(not(unix))]
        {
            let path = self
                .canonical_path
                .join(components.iter().collect::<PathBuf>());
            reject_symlinks(&self.canonical_path, &path)?;
            let file = File::open(&path).map_err(FileError::from)?;
            let metadata = FileMetadata::from_metadata(&file.metadata()?);
            if !metadata.is_regular_file() {
                return Err(FileError::UnsupportedFile("目标不是普通文件".into()));
            }
            Ok(OpenedFile {
                file,
                metadata,
                relative_path: components.iter().collect(),
            })
        }
    }

    pub fn read(&self, relative: &str) -> Result<Vec<u8>> {
        self.open_file(relative)?.read_all()
    }

    pub fn read_prefix(&self, relative: &str, max: usize) -> Result<Vec<u8>> {
        self.open_file(relative)?.read_prefix(max)
    }

    /// 打开父目录并返回末级名字。用于写入/原子替换。
    pub fn parent_handle(&self, relative: &str) -> Result<ParentHandle> {
        let components = relative_components(relative)?;
        let (parent_dir, name) = self.open_parent_components(&components)?;
        #[cfg(unix)]
        let name = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(name.into_bytes())
        };
        let mut parent_path = self.canonical_path.clone();
        for component in &components[..components.len() - 1] {
            parent_path.push(component);
        }
        Ok(ParentHandle {
            dir: parent_dir,
            name,
            parent_path,
            relative_path: components.iter().collect(),
        })
    }

    #[cfg(unix)]
    fn open_parent_components(&self, components: &[OsString]) -> Result<(File, std::ffi::CString)> {
        let mut dir = self.dir.try_clone()?;
        for component in &components[..components.len() - 1] {
            let name = cstring(component)?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(open_error());
            }
            dir = unsafe { File::from_raw_fd(fd) };
        }
        let name = cstring(components.last().expect("non-empty components"))?;
        Ok((dir, name))
    }

    #[cfg(not(unix))]
    fn open_parent_components(&self, components: &[OsString]) -> Result<(File, OsString)> {
        let path: PathBuf = components.iter().collect();
        let parent = self
            .canonical_path
            .join(path.parent().unwrap_or_else(|| Path::new("")));
        reject_symlinks(&self.canonical_path, &parent)?;
        let dir = File::open(&parent)?;
        Ok((
            dir,
            components
                .last()
                .expect("non-empty components")
                .to_os_string(),
        ))
    }
}

/// 把 agent 传入的路径拆成安全组件:拒绝绝对路径、`..`、根/前缀;跳过无意义
/// 的 `.`;空路径也拒绝。
pub fn relative_components(relative: &str) -> Result<Vec<OsString>> {
    if relative.is_empty() {
        return Err(FileError::PathEscape);
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(FileError::PathEscape);
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                if value.is_empty() {
                    return Err(FileError::PathEscape);
                }
                components.push(value.to_os_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(FileError::PathEscape);
            }
        }
    }
    if components.is_empty() {
        return Err(FileError::PathEscape);
    }
    Ok(components)
}

#[cfg(unix)]
fn cstring(value: &OsStr) -> Result<std::ffi::CString> {
    std::ffi::CString::new(value.as_bytes()).map_err(|_| FileError::PathEscape)
}

#[cfg(unix)]
fn open_error() -> FileError {
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOENT) => FileError::NotFound,
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => FileError::SymlinkRejected,
        _ => FileError::Io(error),
    }
}

#[cfg(not(unix))]
fn reject_symlinks(root: &Path, target: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    let relative = target
        .strip_prefix(root)
        .map_err(|_| FileError::PathEscape)?;
    for component in relative.components() {
        current.push(component);
        if std::fs::symlink_metadata(&current)
            .map_err(FileError::from)?
            .file_type()
            .is_symlink()
        {
            return Err(FileError::SymlinkRejected);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn root() -> (tempfile::TempDir, ProjectRoot) {
        let dir = tempfile::tempdir().unwrap();
        let root = ProjectRoot::open(dir.path()).unwrap();
        (dir, root)
    }

    #[test]
    fn reads_regular_file() {
        let (dir, root) = root();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        assert_eq!(root.read("a.txt").unwrap(), b"hello");
    }

    #[test]
    fn accepts_dot_components() {
        let (dir, root) = root();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.txt"), b"hello").unwrap();
        assert_eq!(root.read("./sub/./a.txt").unwrap(), b"hello");
    }

    #[test]
    fn rejects_absolute_parent_and_empty_paths() {
        let (_dir, root) = root();
        assert!(matches!(
            root.read("/etc/passwd"),
            Err(FileError::PathEscape)
        ));
        assert!(matches!(root.read("../a.txt"), Err(FileError::PathEscape)));
        assert!(matches!(root.read("a/../../b"), Err(FileError::PathEscape)));
        assert!(matches!(root.read(""), Err(FileError::PathEscape)));
    }

    #[test]
    fn rejects_terminal_symlink_even_when_target_is_inside() {
        let (dir, root) = root();
        std::fs::write(dir.path().join("real.txt"), b"inside").unwrap();
        symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();
        assert!(matches!(
            root.read("link.txt"),
            Err(FileError::SymlinkRejected)
        ));
    }

    #[test]
    fn rejects_terminal_symlink_pointing_outside_and_keeps_target_bytes() {
        let (dir, root) = root();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, b"outside").unwrap();
        symlink(&outside_file, dir.path().join("link.txt")).unwrap();
        assert!(matches!(
            root.read("link.txt"),
            Err(FileError::SymlinkRejected)
        ));
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside");
    }

    #[test]
    fn rejects_parent_directory_symlink() {
        let (dir, root) = root();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), b"outside").unwrap();
        symlink(outside.path(), dir.path().join("sub")).unwrap();
        assert!(matches!(
            root.read("sub/a.txt"),
            Err(FileError::SymlinkRejected)
        ));
    }

    #[test]
    fn rejects_directory_as_terminal_target() {
        let (dir, root) = root();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(matches!(
            root.read("sub"),
            Err(FileError::UnsupportedFile(_)) | Err(FileError::Io(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_fifo_without_blocking_on_open() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let (dir, root) = root();
        let path = dir.path().join("pipe");
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(matches!(
            root.read("pipe"),
            Err(FileError::UnsupportedFile(_))
        ));
    }

    #[test]
    fn read_prefix_is_bounded() {
        let (dir, root) = root();
        std::fs::write(dir.path().join("a.txt"), vec![b'x'; 1000]).unwrap();
        assert_eq!(root.read_prefix("a.txt", 64).unwrap().len(), 64);
    }

    #[test]
    fn parent_handle_exposes_canonical_parent() {
        let (dir, root) = root();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let handle = root.parent_handle("sub/a.txt").unwrap();
        assert_eq!(handle.name(), OsStr::new("a.txt"));
        assert_eq!(
            handle.parent_path,
            dir.path().canonicalize().unwrap().join("sub")
        );
    }
}

#[cfg(all(test, unix))]
mod identity_race_tests {
    use super::*;
    #[test]
    fn replacing_opened_leaf_with_outside_symlink_does_not_redirect_read() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = root_dir.path().join("a");
        std::fs::write(&target, b"inside").unwrap();
        std::fs::write(outside.path().join("secret"), b"outside").unwrap();
        let root = ProjectRoot::open(root_dir.path()).unwrap();
        let opened = root.open_file("a").unwrap();
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), &target).unwrap();
        assert_eq!(opened.read_all().unwrap(), b"inside");
        assert!(matches!(
            root.open_file("a"),
            Err(FileError::SymlinkRejected)
        ));
        assert_eq!(
            std::fs::read(outside.path().join("secret")).unwrap(),
            b"outside"
        );
    }
}

#[cfg(test)]
mod growth_limit_tests {
    use super::*;
    #[test]
    fn limit_is_checked_while_reading_an_already_open_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        std::fs::write(&path, b"a").unwrap();
        let root = ProjectRoot::open(dir.path()).unwrap();
        let opened = root.open_file("a").unwrap();
        assert_eq!(opened.metadata.len, 1);
        std::fs::write(path, b"grown after metadata").unwrap();
        assert!(matches!(opened.read_limited(1), Err(FileError::Limit(_))));
    }
}
