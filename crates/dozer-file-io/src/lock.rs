//! 跨进程 advisory 写锁。
//!
//! Dozer app 与 daemon 使用同一个产品锁根;锁文件名由规范化目标键的稳定
//! FNV 哈希生成。锁文件不删除:删除会让不同进程在“新 inode”上各自加锁。
//! 这只串行化遵守该锁的 Dozer 写入,不是外部编辑器的强制锁。

use crate::{FileError, Result};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct WriteLock {
    _file: File,
}

impl WriteLock {
    pub fn try_acquire(lock_root: &Path, target_key: &Path) -> Result<Option<Self>> {
        let path = lock_path(lock_root, target_key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        #[cfg(unix)]
        {
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(Some(Self { _file: file }));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                Ok(None)
            } else {
                Err(FileError::Io(error))
            }
        }
        #[cfg(not(unix))]
        {
            Ok(Some(Self { _file: file }))
        }
    }

    /// 在预算内等待独占锁。预算从调用时开始计算,不随重试重置。
    pub fn acquire(lock_root: &Path, target_key: &Path, budget: Duration) -> Result<Self> {
        let deadline = Instant::now() + budget;
        loop {
            if let Some(lock) = Self::try_acquire(lock_root, target_key)? {
                return Ok(lock);
            }
            if Instant::now() >= deadline {
                return Err(FileError::LockTimeout);
            }
            std::thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

/// 默认锁根:产品状态目录不可用时,用当前用户隔离的临时目录,避免在用户
/// 项目里留下 `.dozer-write-*.lock` 未跟踪文件。app/daemon 同一用户共享。
pub fn default_lock_root() -> PathBuf {
    #[cfg(unix)]
    let uid = unsafe { libc::geteuid() };
    #[cfg(not(unix))]
    let uid = 0u32;
    std::env::temp_dir().join(format!("dozer-write-locks-{uid}"))
}

fn lock_path(lock_root: &Path, target_key: &Path) -> PathBuf {
    let hash = crate::fnv1a(target_key.as_os_str().as_encoded_bytes());
    lock_root.join(format!(".dozer-write-{hash:016x}.lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_writer_times_out_while_first_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let first = WriteLock::acquire(dir.path(), &target, Duration::from_millis(50)).unwrap();
        let err = WriteLock::acquire(dir.path(), &target, Duration::from_millis(50)).unwrap_err();
        assert!(matches!(err, FileError::LockTimeout));
        drop(first);
        WriteLock::acquire(dir.path(), &target, Duration::from_millis(50)).unwrap();
    }

    #[test]
    fn different_targets_do_not_block_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let first =
            WriteLock::acquire(dir.path(), &dir.path().join("a"), Duration::from_millis(50))
                .unwrap();
        WriteLock::acquire(dir.path(), &dir.path().join("b"), Duration::from_millis(50)).unwrap();
        drop(first);
    }
}
