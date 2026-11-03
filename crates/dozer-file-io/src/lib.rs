//! Dozer 受控文件 IO。
//!
//! 这一层只做四件事:
//!
//! 1. 用目录句柄 + `O_NOFOLLOW` 逐级打开项目内文件,拒绝末级/父级符号链接;
//! 2. 用文件身份与元数据形成 revision,给调用方做冲突检查;
//! 3. 用产品锁根上的跨进程 advisory 写锁串行化 Dozer 自己的写入;
//! 4. 同目录临时文件 + 原子替换,并保持权限/owner/group/xattr/ACL。
//!
//! 明确不保证:抵抗同用户恶意进程在最终检查与 rename 之间写入、抵抗主动
//! 脱离进程组的后代、以及跨平台 ACL 语义完全一致。目录句柄固定的是授权时
//! 的对象身份;拥有同用户权限、能重挂载或把整个已打开目录移出项目的进程
//! 不在本层保证内。

pub mod lock;
pub mod metadata;
pub mod path;
pub mod write;

pub use metadata::{FileMetadata, Revision};
pub use path::{OpenedFile, ParentHandle, ProjectRoot};
pub use write::{AtomicWriteError, AtomicWriteOutcome, WriteOptions, write_atomic};

use std::io;

/// 受控文件访问的统一错误。产品层负责映射成自己的 wire/本地化形状。
#[derive(Debug)]
pub enum FileError {
    /// 路径不是项目内非空相对路径(绝对路径、`..`、根/前缀)。
    PathEscape,
    Limit(String),
    /// 路径任一级是符号链接。
    SymlinkRejected,
    /// 目标不存在(允许创建时由写入路径单独处理)。
    NotFound,
    /// 存在但不是普通文件(目录、设备、socket 等),或硬链接/编码不支持。
    UnsupportedFile(String),
    /// 必要元数据无法保持(ACL/xattr/owner/group 等)。
    UnsupportedMetadata(String),
    /// revision 在读取与提交之间变化。
    Conflict,
    /// 等待跨进程写锁超过预算。
    LockTimeout,
    /// 底层 IO 错误。
    Io(io::Error),
}

impl FileError {
    pub fn kind_name(&self) -> &'static str {
        match self {
            FileError::PathEscape => "path_escape",
            FileError::Limit(_) => "limit",
            FileError::SymlinkRejected => "symlink_rejected",
            FileError::NotFound => "not_found",
            FileError::UnsupportedFile(_) => "unsupported_file",
            FileError::UnsupportedMetadata(_) => "unsupported_metadata",
            FileError::Conflict => "conflict",
            FileError::LockTimeout => "lock_timeout",
            FileError::Io(_) => "io",
        }
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Limit(reason) => write!(f, "Limit: {reason}"),
            FileError::PathEscape => write!(f, "路径必须位于项目内且不能包含 .. 或绝对路径"),
            FileError::SymlinkRejected => write!(f, "拒绝沿符号链接读写项目文件"),
            FileError::NotFound => write!(f, "文件不存在"),
            FileError::UnsupportedFile(reason) => write!(f, "不支持的文件: {reason}"),
            FileError::UnsupportedMetadata(reason) => {
                write!(f, "无法保留必要的文件元数据: {reason}")
            }
            FileError::Conflict => write!(f, "文件在读取后被其他写入修改"),
            FileError::LockTimeout => write!(f, "等待文件写锁超时"),
            FileError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<io::Error> for FileError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            return FileError::NotFound;
        }
        #[cfg(unix)]
        {
            match error.raw_os_error() {
                Some(libc::ELOOP) => FileError::SymlinkRejected,
                _ => FileError::Io(error),
            }
        }
        #[cfg(not(unix))]
        {
            FileError::Io(error)
        }
    }
}

pub type Result<T> = std::result::Result<T, FileError>;

/// 简单的 FNV-1a,用于稳定的锁文件名与 revision 组合。不能用
/// `DefaultHasher` 做跨进程键(不同版本的实现细节不作为协议)。
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
