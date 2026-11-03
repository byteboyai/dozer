//! 文件身份与 revision。
//!
//! revision 只取 `dev/ino/size/mtime` 等元数据,不复制正文;用于发现“读取后
//! 被替换/改写”的常见情况。它不是文件系统内容 CAS:同尺寸且 mtime 未变的
//! 外部原地改写不在保证内,规格已明确记录。

use std::fs::Metadata;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMetadata {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
}

impl FileMetadata {
    pub fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                mode: metadata.mode(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                dev: metadata.dev(),
                ino: metadata.ino(),
                nlink: metadata.nlink(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                mode: 0o600,
                uid: 0,
                gid: 0,
                dev: 0,
                ino: 0,
                nlink: 1,
            }
        }
    }

    pub fn is_regular_file(&self) -> bool {
        #[cfg(unix)]
        {
            self.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFREG)
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    /// 访问/执行位;setuid/setgid 永远不保留。新文件默认 0600。
    pub fn preserved_mode(&self) -> u32 {
        self.mode & 0o777
    }
}

/// 文件 revision。`identity` 由上述元数据混合而成;与正文无关,开销稳定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Revision(u64);

impl Revision {
    pub fn of(metadata: &FileMetadata) -> Self {
        let modified = metadata
            .modified
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| (duration.as_secs(), duration.subsec_nanos()))
            .unwrap_or((0, 0));
        let mut bytes = Vec::with_capacity(64);
        for value in [
            metadata.dev,
            metadata.ino,
            metadata.len,
            modified.0,
            u64::from(modified.1),
            u64::from(metadata.mode),
            u64::from(metadata.uid),
            u64::from(metadata.gid),
            metadata.nlink,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Revision(crate::fnv1a(&bytes))
    }

    pub fn value(self) -> u64 {
        self.0
    }
}
