//! 原子写入:跨进程锁 + 同目录临时文件 + 元数据保持 + rename + 目录同步。
//!
//! 提交结果分为三类,调用方必须区分:
//!
//! * `Committed`:rename 与目录同步都成功;
//! * `CommittedDurabilityUnknown`:rename 已成功,但目录同步失败——内容已经是
//!   新内容,不能再说“原文件保持不变”;
//! * `AtomicWriteError::NotCommitted`:rename 之前失败,原内容未被本次写入替换。

use crate::lock::WriteLock;
use crate::metadata::{FileMetadata, Revision};
use crate::path::ProjectRoot;
use crate::{FileError, Result};
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

pub struct WriteOptions<'a> {
    /// 产品锁根。`None` 时使用当前用户隔离的临时锁根,不在项目内写锁文件。
    pub lock_root: Option<&'a Path>,
    /// 读取基线时拿到的 revision;`Some` 时提交前必须仍然一致。
    pub expected_revision: Option<Revision>,
    /// 目标不存在时是否允许创建(默认 0600)。
    pub create: bool,
    pub lock_timeout: Duration,
}

impl Default for WriteOptions<'_> {
    fn default() -> Self {
        Self {
            lock_root: None,
            expected_revision: None,
            create: true,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtomicWriteOutcome {
    Committed {
        revision: Revision,
    },
    /// 内容已替换,但目录 fsync 失败;上层应重读并提示持久性未确认。
    CommittedDurabilityUnknown {
        revision: Revision,
        warning: String,
    },
}

#[derive(Debug)]
pub enum AtomicWriteError {
    /// rename 前失败,原文件未被本次写入替换。
    NotCommitted(FileError),
    /// rename 成功但持久性未确认;调用方不能自动重写。
    CommittedDurabilityUnknown { revision: Revision, warning: String },
}

impl std::fmt::Display for AtomicWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AtomicWriteError::NotCommitted(error) => write!(f, "{error}"),
            AtomicWriteError::CommittedDurabilityUnknown { warning, .. } => {
                write!(f, "内容已提交,但持久性未确认: {warning}")
            }
        }
    }
}

impl std::error::Error for AtomicWriteError {}

impl From<FileError> for AtomicWriteError {
    fn from(error: FileError) -> Self {
        AtomicWriteError::NotCommitted(error)
    }
}

impl From<std::io::Error> for AtomicWriteError {
    fn from(error: std::io::Error) -> Self {
        AtomicWriteError::NotCommitted(FileError::from(error))
    }
}

/// 写 `root` 下 `relative` 路径。调用方负责把正文编码成最终字节。
pub fn write_atomic(
    root: &ProjectRoot,
    relative: &str,
    bytes: &[u8],
    options: &WriteOptions<'_>,
) -> std::result::Result<AtomicWriteOutcome, AtomicWriteError> {
    let parent = root.parent_handle(relative)?;
    let lock_root = options
        .lock_root
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::lock::default_lock_root);
    let target_key = parent.parent_path.join(parent.name());
    let _lock = match WriteLock::acquire(&lock_root, &target_key, options.lock_timeout) {
        Ok(lock) => lock,
        Err(error) => return Err(AtomicWriteError::NotCommitted(error)),
    };

    #[cfg(unix)]
    {
        write_atomic_unix(&parent, bytes, options)
    }
    #[cfg(not(unix))]
    {
        write_atomic_fallback(&parent, bytes, options)
    }
}

#[cfg(unix)]
fn write_atomic_unix(
    parent: &crate::path::ParentHandle,
    bytes: &[u8],
    options: &WriteOptions<'_>,
) -> std::result::Result<AtomicWriteOutcome, AtomicWriteError> {
    let dir_fd = parent.dir().as_raw_fd();
    let name = CString::new(parent.name().as_bytes())
        .map_err(|_| AtomicWriteError::NotCommitted(FileError::PathEscape))?;

    let baseline = open_at(dir_fd, &name, libc::O_RDONLY)?;
    let (baseline_file, baseline_meta) = match baseline {
        Some(file) => {
            let metadata = FileMetadata::from_metadata(&file.metadata()?);
            if !metadata.is_regular_file() {
                return Err(AtomicWriteError::NotCommitted(FileError::UnsupportedFile(
                    "目标不是普通文件".into(),
                )));
            }
            if metadata.nlink > 1 {
                return Err(AtomicWriteError::NotCommitted(FileError::UnsupportedFile(
                    "目标有多个硬链接,拒绝原子替换".into(),
                )));
            }
            (Some(file), Some(metadata))
        }
        None => {
            if !options.create {
                return Err(AtomicWriteError::NotCommitted(FileError::NotFound));
            }
            if options.expected_revision.is_some() {
                return Err(AtomicWriteError::NotCommitted(FileError::NotFound));
            }
            (None, None)
        }
    };

    if let (Some(expected), Some(meta)) = (options.expected_revision, baseline_meta.as_ref())
        && Revision::of(meta) != expected
    {
        return Err(AtomicWriteError::NotCommitted(FileError::Conflict));
    }

    let mode = match baseline_meta.as_ref() {
        Some(meta) => meta.preserved_mode(),
        None => 0o600,
    };

    let (tmp_name, mut tmp_file) = create_temp(dir_fd)?;
    let mut guard = TempGuard {
        dir_fd,
        name: tmp_name.clone(),
        armed: true,
    };

    let write_result = (|| -> Result<()> {
        tmp_file.write_all(bytes)?;
        tmp_file.flush()?;
        tmp_file.sync_all()?;
        let raw = tmp_file.as_raw_fd();
        if unsafe { libc::fchmod(raw, mode as libc::mode_t) } != 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法恢复权限位: {}",
                std::io::Error::last_os_error()
            )));
        }
        if let Some(meta) = baseline_meta.as_ref() {
            if unsafe { libc::fchown(raw, meta.uid, meta.gid) } != 0 {
                let error = std::io::Error::last_os_error();
                // A temp file can inherit a directory's group. Check its actual
                // owner/group rather than assuming effective uid/gid describes it.
                let actual = FileMetadata::from_metadata(&tmp_file.metadata()?);
                if actual.uid != meta.uid || actual.gid != meta.gid {
                    return Err(FileError::UnsupportedMetadata(format!(
                        "无法恢复 owner/group: {error}"
                    )));
                }
            }
            if let Some(source) = baseline_file.as_ref() {
                #[cfg(test)]
                fault(FaultStage::Metadata)?;
                copy_metadata(source, &tmp_file)?;
            }
        }
        Ok(())
    })();

    if let Err(error) = write_result {
        return Err(AtomicWriteError::NotCommitted(error));
    }

    // 提交前在同一把写锁下重新检查目标身份与 revision。外部编辑器不受这把
    // 锁约束,所以这一步只是缩小窗口,不是内容 CAS。
    let current = open_at(dir_fd, &name, libc::O_RDONLY)?;
    match (&baseline_meta, current) {
        (Some(before), Some(file)) => {
            let after = FileMetadata::from_metadata(&file.metadata()?);
            if after.dev != before.dev
                || after.ino != before.ino
                || Revision::of(&after) != Revision::of(before)
            {
                return Err(AtomicWriteError::NotCommitted(FileError::Conflict));
            }
        }
        (None, None) => {}
        _ => return Err(AtomicWriteError::NotCommitted(FileError::Conflict)),
    }

    // Final metadata and bytes must be durable before committing the new name.
    tmp_file.sync_all()?;
    let revision = Revision::of(&FileMetadata::from_metadata(&tmp_file.metadata()?));
    #[cfg(test)]
    fault(FaultStage::BeforeRename)?;
    if unsafe { libc::renameat(dir_fd, guard.name.as_ptr(), dir_fd, name.as_ptr()) } != 0 {
        return Err(AtomicWriteError::NotCommitted(FileError::Io(
            std::io::Error::last_os_error(),
        )));
    }
    guard.armed = false;

    // Use the committed file handle's revision; reopening the path could fail or
    // observe an unrelated replacement after a successful rename.
    #[cfg(test)]
    if let Err(error) = fault(FaultStage::DirectorySync) {
        return Err(AtomicWriteError::CommittedDurabilityUnknown {
            revision,
            warning: error.to_string(),
        });
    }
    if let Err(error) = parent.dir().sync_all() {
        return Err(AtomicWriteError::CommittedDurabilityUnknown {
            revision,
            warning: format!("目录同步失败: {error}"),
        });
    }
    Ok(AtomicWriteOutcome::Committed { revision })
}

#[cfg(unix)]
fn open_at(dir_fd: RawFd, name: &CString, flags: libc::c_int) -> Result<Option<File>> {
    let fd = unsafe {
        libc::openat(
            dir_fd,
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd >= 0 {
        return Ok(Some(unsafe { File::from_raw_fd(fd) }));
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOENT) => Ok(None),
        Some(libc::ELOOP) => Err(FileError::SymlinkRejected),
        _ => Err(FileError::Io(error)),
    }
}

#[cfg(unix)]
fn create_temp(dir_fd: RawFd) -> Result<(CString, File)> {
    for attempt in 0..32u32 {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let name = format!(".dozer-tmp-{}-{unique}-{attempt}", std::process::id());
        let c_name = CString::new(name.clone()).expect("temp name has no nul");
        let fd = unsafe {
            libc::openat(
                dir_fd,
                c_name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        };
        if fd >= 0 {
            return Ok((c_name, unsafe { File::from_raw_fd(fd) }));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            continue;
        }
        return Err(FileError::Io(error));
    }
    Err(FileError::Io(std::io::Error::other(
        "无法创建唯一的临时文件",
    )))
}

#[cfg(unix)]
struct TempGuard {
    dir_fd: RawFd,
    name: CString,
    armed: bool,
}

#[cfg(unix)]
impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            unsafe {
                libc::unlinkat(self.dir_fd, self.name.as_ptr(), 0);
            }
        }
    }
}

#[cfg(unix)]
fn copy_metadata(source: &File, target: &File) -> Result<()> {
    copy_xattrs(source, target)?;
    copy_acl(source, target)?;
    Ok(())
}

#[cfg(all(unix, target_os = "macos"))]
fn copy_xattrs(source: &File, target: &File) -> Result<()> {
    let src = source.as_raw_fd();
    let dst = target.as_raw_fd();
    let size = unsafe { libc::flistxattr(src, std::ptr::null_mut(), 0, 0) };
    if size < 0 {
        let error = std::io::Error::last_os_error();
        if is_unsupported_xattr(&error) {
            return Ok(());
        }
        return Err(FileError::UnsupportedMetadata(format!(
            "无法列出 xattr: {error}"
        )));
    }
    if size == 0 {
        return Ok(());
    }
    let mut names = vec![0u8; size as usize];
    let size = unsafe { libc::flistxattr(src, names.as_mut_ptr().cast(), names.len(), 0) };
    if size < 0 {
        return Err(FileError::UnsupportedMetadata(format!(
            "无法读取 xattr 列表: {}",
            std::io::Error::last_os_error()
        )));
    }
    for name in names[..size as usize].split(|byte| *byte == 0) {
        if name.is_empty() {
            continue;
        }
        let c_name = CString::new(name)
            .map_err(|_| FileError::UnsupportedMetadata("xattr 名非法".into()))?;
        let value_size =
            unsafe { libc::fgetxattr(src, c_name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
        if value_size < 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法读取 xattr {}: {}",
                String::from_utf8_lossy(name),
                std::io::Error::last_os_error()
            )));
        }
        let mut value = vec![0u8; value_size as usize];
        let value_size = unsafe {
            libc::fgetxattr(
                src,
                c_name.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if value_size < 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法读取 xattr {}: {}",
                String::from_utf8_lossy(name),
                std::io::Error::last_os_error()
            )));
        }
        let rc = unsafe {
            libc::fsetxattr(
                dst,
                c_name.as_ptr(),
                value.as_ptr().cast(),
                value_size as usize,
                0,
                0,
            )
        };
        if rc != 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法恢复 xattr {}: {}",
                String::from_utf8_lossy(name),
                std::io::Error::last_os_error()
            )));
        }
    }
    Ok(())
}

#[cfg(all(unix, target_os = "linux"))]
fn copy_xattrs(source: &File, target: &File) -> Result<()> {
    let src = source.as_raw_fd();
    let dst = target.as_raw_fd();
    let size = unsafe { libc::flistxattr(src, std::ptr::null_mut(), 0) };
    if size < 0 {
        let error = std::io::Error::last_os_error();
        if is_unsupported_xattr(&error) {
            return Ok(());
        }
        return Err(FileError::UnsupportedMetadata(format!(
            "无法列出 xattr: {error}"
        )));
    }
    if size == 0 {
        return Ok(());
    }
    let mut names = vec![0u8; size as usize];
    let size = unsafe { libc::flistxattr(src, names.as_mut_ptr().cast(), names.len()) };
    if size < 0 {
        return Err(FileError::UnsupportedMetadata(format!(
            "无法读取 xattr 列表: {}",
            std::io::Error::last_os_error()
        )));
    }
    for name in names[..size as usize].split(|byte| *byte == 0) {
        if name.is_empty() {
            continue;
        }
        let c_name = CString::new(name)
            .map_err(|_| FileError::UnsupportedMetadata("xattr 名非法".into()))?;
        let value_size = unsafe { libc::fgetxattr(src, c_name.as_ptr(), std::ptr::null_mut(), 0) };
        if value_size < 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法读取 xattr {}: {}",
                String::from_utf8_lossy(name),
                std::io::Error::last_os_error()
            )));
        }
        let mut value = vec![0u8; value_size as usize];
        let value_size = unsafe {
            libc::fgetxattr(src, c_name.as_ptr(), value.as_mut_ptr().cast(), value.len())
        };
        if value_size < 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法读取 xattr {}: {}",
                String::from_utf8_lossy(name),
                std::io::Error::last_os_error()
            )));
        }
        let rc = unsafe {
            libc::fsetxattr(
                dst,
                c_name.as_ptr(),
                value.as_ptr().cast(),
                value_size as usize,
                0,
            )
        };
        if rc != 0 {
            return Err(FileError::UnsupportedMetadata(format!(
                "无法恢复 xattr {}: {}",
                String::from_utf8_lossy(name),
                std::io::Error::last_os_error()
            )));
        }
    }
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn copy_xattrs(_source: &File, _target: &File) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn is_unsupported_xattr(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOTSUP) | Some(libc::EOPNOTSUPP)
    )
}

#[cfg(target_os = "macos")]
mod macos_acl {
    use std::ffi::c_void;

    pub const ACL_TYPE_EXTENDED: libc::c_int = 0x100;
    pub const ACL_FIRST_ENTRY: libc::c_int = 0;

    unsafe extern "C" {
        pub fn acl_get_fd_np(fd: libc::c_int, type_: libc::c_int) -> *mut c_void;
        pub fn acl_get_entry(
            acl: *mut c_void,
            entry_id: libc::c_int,
            entry_p: *mut *mut c_void,
        ) -> libc::c_int;
        pub fn acl_set_fd_np(fd: libc::c_int, acl: *mut c_void, type_: libc::c_int) -> libc::c_int;
        pub fn acl_free(obj: *mut c_void) -> libc::c_int;
    }
}

#[cfg(target_os = "macos")]
fn copy_acl(source: &File, target: &File) -> Result<()> {
    use macos_acl::*;
    let acl = unsafe { acl_get_fd_np(source.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let error = std::io::Error::last_os_error();
        // Darwin reports absence of an extended ACL as ENOENT even for an open file.
        if error.raw_os_error() == Some(libc::ENOENT) {
            return Ok(());
        }
        return Err(FileError::UnsupportedMetadata(format!(
            "无法读取 ACL: {error}"
        )));
    }
    let mut entry: *mut std::ffi::c_void = std::ptr::null_mut();
    // Darwin returns 0 for an entry; POSIX/Linux's 1-success convention does not apply.
    let has_entry = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &mut entry) } == 0;
    if !has_entry {
        unsafe {
            acl_free(acl);
        }
        return Ok(());
    }
    let rc = unsafe { acl_set_fd_np(target.as_raw_fd(), acl, ACL_TYPE_EXTENDED) };
    let error = std::io::Error::last_os_error();
    unsafe {
        acl_free(acl);
    }
    if rc != 0 {
        return Err(FileError::UnsupportedMetadata(format!(
            "无法恢复 ACL: {error}"
        )));
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn copy_acl(_source: &File, _target: &File) -> Result<()> {
    // Linux 的 POSIX ACL 走 system.posix_acl_* xattr,已由 copy_xattrs 处理。
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    fn project() -> (tempfile::TempDir, ProjectRoot) {
        let dir = tempfile::tempdir().unwrap();
        let root = ProjectRoot::open(dir.path()).unwrap();
        (dir, root)
    }

    fn options() -> WriteOptions<'static> {
        WriteOptions {
            lock_root: None,
            expected_revision: None,
            create: true,
            lock_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn creates_new_file_with_0600() {
        let (dir, root) = project();
        let outcome = write_atomic(&root, "new.txt", b"hello", &options()).unwrap();
        assert!(matches!(outcome, AtomicWriteOutcome::Committed { .. }));
        assert_eq!(std::fs::read(dir.path().join("new.txt")).unwrap(), b"hello");
        assert_eq!(
            std::fs::metadata(dir.path().join("new.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn preserves_0755_and_0600() {
        let (dir, root) = project();
        for mode in [0o755, 0o600] {
            let path = dir.path().join(format!("m{mode}.sh"));
            std::fs::write(&path, b"old").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            write_atomic(
                &root,
                path.file_name().unwrap().to_str().unwrap(),
                b"new",
                &options(),
            )
            .unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                mode
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"new");
        }
    }

    #[test]
    fn rejects_terminal_symlink_and_keeps_outside_bytes() {
        let (dir, root) = project();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, b"outside").unwrap();
        symlink(&outside_file, dir.path().join("link.txt")).unwrap();
        let error = write_atomic(&root, "link.txt", b"changed", &options()).unwrap_err();
        assert!(matches!(
            error,
            AtomicWriteError::NotCommitted(FileError::SymlinkRejected)
        ));
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside");
    }

    #[test]
    fn rejects_hardlinked_target() {
        let (dir, root) = project();
        std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
        std::fs::hard_link(dir.path().join("a.txt"), dir.path().join("b.txt")).unwrap();
        let error = write_atomic(&root, "a.txt", b"new", &options()).unwrap_err();
        assert!(matches!(
            error,
            AtomicWriteError::NotCommitted(FileError::UnsupportedFile(_))
        ));
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(dir.path().join("b.txt")).unwrap(), b"old");
    }

    #[test]
    fn conflict_when_revision_changed() {
        let (dir, root) = project();
        std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
        let opened = root.open_file("a.txt").unwrap();
        let revision = Revision::of(&opened.metadata);
        std::fs::write(dir.path().join("a.txt"), b"changed externally").unwrap();
        let error = write_atomic(
            &root,
            "a.txt",
            b"new",
            &WriteOptions {
                expected_revision: Some(revision),
                ..options()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AtomicWriteError::NotCommitted(FileError::Conflict)
        ));
        assert_eq!(
            std::fs::read(dir.path().join("a.txt")).unwrap(),
            b"changed externally"
        );
    }

    #[test]
    fn missing_target_with_expected_revision_is_not_found() {
        let (_dir, root) = project();
        let error = write_atomic(
            &root,
            "missing.txt",
            b"new",
            &WriteOptions {
                expected_revision: Some(Revision::of(&FileMetadata {
                    len: 0,
                    modified: None,
                    mode: 0o600,
                    uid: 0,
                    gid: 0,
                    dev: 0,
                    ino: 0,
                    nlink: 1,
                })),
                ..options()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AtomicWriteError::NotCommitted(FileError::NotFound)
        ));
    }

    #[test]
    fn write_does_not_leave_temp_files() {
        let (dir, root) = project();
        write_atomic(&root, "a.txt", b"new", &options()).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.contains("dozer-tmp") || name.contains(".dozer-write-")
            })
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn write_preserves_owner_and_group() {
        let (dir, root) = project();
        std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
        let before = std::fs::metadata(dir.path().join("a.txt")).unwrap();
        write_atomic(&root, "a.txt", b"new", &options()).unwrap();
        let after = std::fs::metadata(dir.path().join("a.txt")).unwrap();
        assert_eq!(after.uid(), before.uid());
        assert_eq!(after.gid(), before.gid());
    }

    #[cfg(unix)]
    #[test]
    fn write_clears_setuid_and_setgid_bits() {
        let (dir, root) = project();
        let path = dir.path().join("tool");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o6755)).unwrap();
        write_atomic(&root, "tool", b"new", &options()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o755, "提权位不能被恢复到新文件");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn write_preserves_user_xattrs() {
        use std::ffi::CString;
        let (dir, root) = project();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"old").unwrap();
        let c_path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        let name = CString::new("user.dozer-test").unwrap();
        let value = b"kept";
        #[cfg(target_os = "macos")]
        let set = unsafe {
            libc::setxattr(
                c_path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        #[cfg(target_os = "linux")]
        let set = unsafe {
            libc::setxattr(
                c_path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        assert_eq!(set, 0, "{}", std::io::Error::last_os_error());
        write_atomic(&root, "a.txt", b"new", &options()).unwrap();

        let mut buffer = [0u8; 16];
        #[cfg(target_os = "macos")]
        let size = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
                0,
            )
        };
        #[cfg(target_os = "linux")]
        let size = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        assert_eq!(size, value.len() as isize);
        assert_eq!(&buffer[..value.len()], value);
    }
}

#[cfg(not(unix))]
fn write_atomic_fallback(
    parent: &crate::path::ParentHandle,
    bytes: &[u8],
    options: &WriteOptions<'_>,
) -> std::result::Result<AtomicWriteOutcome, AtomicWriteError> {
    let target = parent.parent_path.join(parent.name());
    let expected = options.expected_revision;
    if let Some(expected) = expected {
        let metadata = std::fs::metadata(&target).map_err(|error| {
            AtomicWriteError::NotCommitted(if error.kind() == std::io::ErrorKind::NotFound {
                FileError::NotFound
            } else {
                FileError::Io(error)
            })
        })?;
        if Revision::of(&FileMetadata::from_metadata(&metadata)) != expected {
            return Err(AtomicWriteError::NotCommitted(FileError::Conflict));
        }
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let tmp = parent
        .parent_path
        .join(format!(".dozer-tmp-{}-{unique}", std::process::id()));
    std::fs::write(&tmp, bytes)
        .map_err(|error| AtomicWriteError::NotCommitted(FileError::Io(error)))?;
    if let Err(error) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AtomicWriteError::NotCommitted(FileError::Io(error)));
    }
    let revision = std::fs::metadata(&target)
        .map(|metadata| Revision::of(&FileMetadata::from_metadata(&metadata)))
        .map_err(|error| AtomicWriteError::NotCommitted(FileError::Io(error)))?;
    Ok(AtomicWriteOutcome::Committed { revision })
}

/// 供测试/诊断使用:读取整个文件并返回字节与 revision。
pub fn read_with_revision(root: &ProjectRoot, relative: &str) -> Result<(Vec<u8>, Revision)> {
    let opened = root.open_file(relative)?;
    let revision = Revision::of(&opened.metadata);
    let mut file = opened.file().try_clone()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok((bytes, revision))
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultStage {
    Metadata,
    BeforeRename,
    DirectorySync,
}
#[cfg(test)]
thread_local! {static FAULT:std::cell::Cell<Option<FaultStage>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
fn fault(stage: FaultStage) -> std::io::Result<()> {
    if FAULT.get() == Some(stage) {
        FAULT.set(None);
        Err(std::io::Error::other("injected write fault"))
    } else {
        Ok(())
    }
}
#[cfg(all(test, unix))]
mod commit_fault_tests {
    use super::*;
    #[test]
    fn metadata_copy_failure_does_not_commit() {
        let dir = tempfile::tempdir().unwrap();
        let root = ProjectRoot::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("a"), b"old").unwrap();
        FAULT.set(Some(FaultStage::Metadata));
        assert!(matches!(
            write_atomic(&root, "a", b"new", &WriteOptions::default()),
            Err(AtomicWriteError::NotCommitted(_))
        ));
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn before_rename_failure_preserves_original_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let root = ProjectRoot::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("a"), b"old").unwrap();
        FAULT.set(Some(FaultStage::BeforeRename));
        assert!(matches!(
            write_atomic(&root, "a", b"new", &WriteOptions::default()),
            Err(AtomicWriteError::NotCommitted(_))
        ));
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn directory_sync_failure_reports_committed_without_retry() {
        let dir = tempfile::tempdir().unwrap();
        let root = ProjectRoot::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("a"), b"old").unwrap();
        FAULT.set(Some(FaultStage::DirectorySync));
        assert!(matches!(
            write_atomic(&root, "a", b"new", &WriteOptions::default()),
            Err(AtomicWriteError::CommittedDurabilityUnknown { .. })
        ));
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod acl_regression_tests {
    use super::*;
    #[test]
    fn extended_acl_survives_atomic_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        std::fs::write(&path, b"old").unwrap();
        let status = std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let acl = || {
            let output = std::process::Command::new("/bin/ls")
                .arg("-le")
                .arg(&path)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .skip(1)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let before = acl();
        assert!(!before.is_empty());
        let root = ProjectRoot::open(dir.path()).unwrap();
        write_atomic(&root, "a", b"new", &WriteOptions::default()).unwrap();
        assert_eq!(acl(), before);
    }
}

#[cfg(all(test, unix))]
mod cross_process_tests {
    use super::*;
    #[test]
    #[ignore = "helper invoked by cross_process_revision_conflict"]
    fn writer_helper() {
        let root_path =
            std::path::PathBuf::from(std::env::var_os("DOZER_TEST_WRITE_ROOT").unwrap());
        let label = std::env::var("DOZER_TEST_WRITE_LABEL").unwrap();
        let root = ProjectRoot::open(&root_path).unwrap();
        let baseline = Revision::of(&root.open_file("a").unwrap().metadata);
        std::fs::write(root_path.join(format!("ready-{label}")), b"").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !root_path.join("start").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let bytes = label.repeat(512 * 1024);
        let outcome = write_atomic(
            &root,
            "a",
            bytes.as_bytes(),
            &WriteOptions {
                expected_revision: Some(baseline),
                create: false,
                ..Default::default()
            },
        );
        let result = match outcome {
            Ok(AtomicWriteOutcome::Committed { .. }) => "committed",
            Err(AtomicWriteError::NotCommitted(FileError::Conflict)) => "conflict",
            other => panic!("{other:?}"),
        };
        std::fs::write(root_path.join(format!("result-{label}")), result).unwrap();
    }
    #[test]
    fn cross_process_revision_conflict() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"old").unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut workers = Vec::new();
        for label in ["a", "b"] {
            workers.push(
                std::process::Command::new(&executable)
                    .args([
                        "--exact",
                        "write::cross_process_tests::writer_helper",
                        "--ignored",
                    ])
                    .env("DOZER_TEST_WRITE_ROOT", dir.path())
                    .env("DOZER_TEST_WRITE_LABEL", label)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !dir.path().join("ready-a").exists() || !dir.path().join("ready-b").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::fs::write(dir.path().join("start"), b"").unwrap();
        for worker in &mut workers {
            assert!(worker.wait().unwrap().success());
        }
        let mut outcomes = [
            std::fs::read_to_string(dir.path().join("result-a")).unwrap(),
            std::fs::read_to_string(dir.path().join("result-b")).unwrap(),
        ];
        outcomes.sort();
        assert_eq!(outcomes, ["committed", "conflict"]);
        let bytes = std::fs::read(dir.path().join("a")).unwrap();
        assert_eq!(bytes.len(), 512 * 1024);
        assert!(bytes.iter().all(|b| *b == bytes[0]));
    }
}
