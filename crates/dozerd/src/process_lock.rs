//! 原子单实例进程锁。
//!
//! 在日志初始化、数据库迁移、apps 对账和 socket 清理之前取得并持有到进程
//! 完全退出。锁文件不 unlink:删除会让两个进程在“新 inode”上各自加锁。
//! `ensure_single_instance` 的 connect 探测仍保留,用于清理陈旧 socket;但
//! “谁能启动”由本锁裁决,connect 超时不再是死亡证明。

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ProcessLock {
    _file: File,
    path: PathBuf,
}

impl ProcessLock {
    /// 非阻塞取得排他锁;已被占用时立即返回可读错误,不等待。
    pub fn acquire(lock_path: &Path) -> Result<Self> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).context("创建进程锁目录")?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .with_context(|| format!("打开进程锁 {}", lock_path.display()))?;
        #[cfg(unix)]
        {
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    anyhow::bail!(
                        "另一个 dozerd 实例已持有进程锁（{}），拒绝启动",
                        lock_path.display()
                    );
                }
                return Err(error).context("取得进程锁");
            }
        }
        Ok(Self {
            _file: file,
            path: lock_path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_holder_is_rejected_and_lock_file_is_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dozerd.lock");
        let first = ProcessLock::acquire(&path).unwrap();
        assert!(path.exists());
        let err = ProcessLock::acquire(&path).unwrap_err().to_string();
        assert!(err.contains("已持有进程锁"), "{err}");
        drop(first);
        assert!(path.exists(), "锁文件不能在退出时 unlink");
        let mut second = ProcessLock::acquire(&path);
        for _ in 0..50 {
            if second.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            second = ProcessLock::acquire(&path);
        }
        let second = second.unwrap_or_else(|error| panic!("second acquire failed: {error}"));
        assert_eq!(second.path(), path);
    }
}
