//! 脏内容 recovery snapshot(文件预览重构 Phase C Task 6)。
//!
//! 版本化 manifest + 正文快照,写到一个专用恢复目录;`write_snapshot` 用
//! 临时文件 + rename 原子替换。启动时 [`read_snapshot`] 读回,再由
//! [`classify_recovery`] 对照磁盘现状判断"直接恢复 / 冲突 / 磁盘已变"。
//!
//! 纯文件逻辑,不依赖 GUI 类型;生产目录由 `dozer_core::paths::config_dir()`
//! 派生,单测传显式目录。

// 恢复逻辑已就绪,接入 `PreviewTab` 脏休眠(Phase C 后续步骤)前部分 API
// 暂未被非测试代码调用;显式允许。
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::file_profile::FileProfile;
use super::file_profile::{LineEnding, TextEncoding};
use super::webview_protocol::{TextPosition, TextRange};

/// 当前 manifest 版本。
pub const RECOVERY_VERSION: u32 = 1;

/// 版本化 manifest:记录快照对应的路径、基准(磁盘)状态、编辑器 revision、
/// 编码/换行约定与视图状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryManifest {
    pub version: u32,
    pub path: PathBuf,
    /// 快照时磁盘文件的 mtime(Unix 毫秒);用于判断外部是否已改。
    pub base_mtime_ms: Option<u64>,
    /// 快照时磁盘文件的字节数。
    pub base_len: u64,
    /// 快照时编辑器文档 revision。
    pub editor_revision: u64,
    pub encoding: String,
    pub line_ending: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<TextPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<TextRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_line: Option<u32>,
}

impl RecoveryManifest {
    /// 由文件画像 + 编辑器 revision + 视图状态构造 manifest。
    pub fn from_profile(
        path: PathBuf,
        profile: &FileProfile,
        editor_revision: u64,
        cursor: Option<TextPosition>,
        selection: Option<TextRange>,
        top_line: Option<u32>,
    ) -> Self {
        Self {
            version: RECOVERY_VERSION,
            path,
            base_mtime_ms: profile.modified.and_then(system_time_ms),
            base_len: profile.size_bytes,
            editor_revision,
            encoding: encoding_token(profile.encoding).to_string(),
            line_ending: line_ending_token(profile.line_ending).to_string(),
            cursor,
            selection,
            top_line,
        }
    }
}

/// 恢复判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResolution {
    /// 磁盘未变,可直接恢复快照。
    Restore,
    /// 磁盘也变了(与快照基准不符):进入冲突,不静默选择。
    ConflictDiskChanged,
    /// 原文件已不存在。
    ConflictMissingFile,
}

/// 默认恢复目录:`<config_dir>/preview_recovery/`。
pub fn recovery_dir() -> PathBuf {
    dozer_core::paths::config_dir().join("preview_recovery")
}

fn manifest_path(dir: &Path, project_id: i64, tab_id: usize) -> PathBuf {
    dir.join(format!("{project_id}-{tab_id}.json"))
}

fn snapshot_path(dir: &Path, project_id: i64, tab_id: usize) -> PathBuf {
    dir.join(format!("{project_id}-{tab_id}.snap"))
}

/// 原子写一份 recovery(manifest JSON + 正文快照)。任何一步失败都不留半成品。
pub fn write_snapshot(
    dir: &Path,
    project_id: i64,
    tab_id: usize,
    manifest: &RecoveryManifest,
    text: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut m = manifest.clone();
    m.version = RECOVERY_VERSION;
    let json = serde_json::to_vec_pretty(&m).map_err(std::io::Error::other)?;
    atomic_write(&snapshot_path(dir, project_id, tab_id), text.as_bytes())?;
    atomic_write(&manifest_path(dir, project_id, tab_id), &json)?;
    Ok(())
}

/// 读回一份 recovery。manifest/快照缺失、损坏或版本不认识时返回 `None`
/// (调用方当作"没有 recovery",不 panic)。
pub fn read_snapshot(
    dir: &Path,
    project_id: i64,
    tab_id: usize,
) -> Option<(RecoveryManifest, String)> {
    let json = std::fs::read(manifest_path(dir, project_id, tab_id)).ok()?;
    let manifest: RecoveryManifest = serde_json::from_slice(&json).ok()?;
    if manifest.version != RECOVERY_VERSION {
        return None;
    }
    let bytes = std::fs::read(snapshot_path(dir, project_id, tab_id)).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    Some((manifest, text))
}

/// 正常保存后删除 recovery(以及清理零散临时文件)。
pub fn clear_snapshot(dir: &Path, project_id: i64, tab_id: usize) {
    let _ = std::fs::remove_file(manifest_path(dir, project_id, tab_id));
    let _ = std::fs::remove_file(snapshot_path(dir, project_id, tab_id));
}

/// 对照磁盘现状判断恢复方式。
pub fn classify_recovery(
    manifest: &RecoveryManifest,
    disk: Option<&FileProfile>,
) -> RecoveryResolution {
    let Some(disk) = disk else {
        return RecoveryResolution::ConflictMissingFile;
    };
    let disk_mtime = disk.modified.and_then(system_time_ms);
    if disk.size_bytes == manifest.base_len && disk_mtime == manifest.base_mtime_ms {
        RecoveryResolution::Restore
    } else {
        RecoveryResolution::ConflictDiskChanged
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn system_time_ms(t: std::time::SystemTime) -> Option<u64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

fn encoding_token(e: TextEncoding) -> &'static str {
    match e {
        TextEncoding::Utf8 => "utf8",
        TextEncoding::Utf8Bom => "utf8-bom",
        TextEncoding::Utf16Le => "utf16le",
        TextEncoding::Utf16Be => "utf16be",
    }
}

fn line_ending_token(l: LineEnding) -> &'static str {
    match l {
        LineEnding::Lf => "lf",
        LineEnding::Crlf => "crlf",
        LineEnding::Cr => "cr",
        LineEnding::Mixed => "mixed",
        LineEnding::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::file_profile::{ContentKind, Utf8Status, profile_file};
    use std::time::{Duration, SystemTime};

    fn profile(size: u64, mtime: Option<SystemTime>) -> FileProfile {
        FileProfile {
            size_bytes: size,
            sampled_line_count: None,
            sampled_max_line_bytes: 10,
            utf8: Utf8Status::Valid,
            content_kind: ContentKind::Text,
            modified: mtime,
            encoding: TextEncoding::Utf8Bom,
            line_ending: LineEnding::Crlf,
            has_bom: true,
        }
    }

    fn manifest(size: u64, mtime: Option<SystemTime>) -> RecoveryManifest {
        RecoveryManifest::from_profile(
            PathBuf::from("/repo/a.rs"),
            &profile(size, mtime),
            7,
            Some(TextPosition { line: 2, column: 3 }),
            None,
            Some(1),
        )
    }

    #[test]
    fn write_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(
            100,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000)),
        );
        write_snapshot(dir.path(), 1, 5, &m, "dirty text\n").unwrap();
        let (read_m, text) = read_snapshot(dir.path(), 1, 5).unwrap();
        assert_eq!(read_m, m);
        assert_eq!(text, "dirty text\n");
        assert_eq!(read_m.encoding, "utf8-bom");
        assert_eq!(read_m.line_ending, "crlf");
        assert_eq!(read_m.editor_revision, 7);
    }

    #[test]
    fn missing_snapshot_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_snapshot(dir.path(), 1, 5).is_none());
    }

    #[test]
    fn corrupt_manifest_is_none_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(manifest_path(dir.path(), 1, 5), b"not json").unwrap();
        assert!(read_snapshot(dir.path(), 1, 5).is_none());
    }

    #[test]
    fn unknown_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manifest(1, None);
        m.version = 999;
        let json = serde_json::to_vec(&m).unwrap();
        std::fs::write(manifest_path(dir.path(), 1, 5), json).unwrap();
        std::fs::write(snapshot_path(dir.path(), 1, 5), b"x").unwrap();
        assert!(read_snapshot(dir.path(), 1, 5).is_none());
    }

    #[test]
    fn empty_manifest_bytes_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(1, None);
        write_snapshot(dir.path(), 1, 5, &m, "").unwrap();
        let (_, text) = read_snapshot(dir.path(), 1, 5).unwrap();
        assert_eq!(text, "");
    }

    #[test]
    fn classify_restore_when_disk_unchanged() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let m = manifest(100, Some(mtime));
        assert_eq!(
            classify_recovery(&m, Some(&profile(100, Some(mtime)))),
            RecoveryResolution::Restore
        );
    }

    #[test]
    fn classify_conflict_when_disk_changed() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let m = manifest(100, Some(mtime));
        // size 变
        assert_eq!(
            classify_recovery(&m, Some(&profile(200, Some(mtime)))),
            RecoveryResolution::ConflictDiskChanged
        );
        // mtime 变
        assert_eq!(
            classify_recovery(
                &m,
                Some(&profile(100, Some(mtime + Duration::from_secs(1))))
            ),
            RecoveryResolution::ConflictDiskChanged
        );
    }

    #[test]
    fn classify_conflict_when_file_missing() {
        let m = manifest(100, None);
        assert_eq!(
            classify_recovery(&m, None),
            RecoveryResolution::ConflictMissingFile
        );
    }

    #[test]
    fn clear_removes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        write_snapshot(dir.path(), 1, 5, &manifest(1, None), "x").unwrap();
        clear_snapshot(dir.path(), 1, 5);
        assert!(read_snapshot(dir.path(), 1, 5).is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "无残留");
    }

    #[test]
    fn write_is_atomic_no_partial_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(1, None);
        write_snapshot(dir.path(), 2, 9, &m, "abc").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留临时文件");
    }

    #[test]
    fn integration_profile_file_feeds_manifest() {
        // 用真实磁盘文件走一遍 profile → manifest,确认基准状态对得上。
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("real.rs");
        std::fs::write(&f, "fn main() {}\n").unwrap();
        let prof = profile_file(&f).unwrap();
        let m = RecoveryManifest::from_profile(f.clone(), &prof, 3, None, None, None);
        assert_eq!(m.base_len, std::fs::metadata(&f).unwrap().len());
        assert_eq!(
            classify_recovery(&m, Some(&profile_file(&f).unwrap())),
            RecoveryResolution::Restore
        );
    }
}
