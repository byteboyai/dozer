//! 报告版本与扫描范围元数据（spec 2026-09-21「报告版本与扫描范围」）。
//!
//! 一次扫描产出带 `schema_version`、扫描状态、文件清单和语言摘要的可解释
//! 范围报告；旧 JSON 通过 serde 默认值迁移为 v1 报告（`schema_version` 缺省
//! 时为 1，UI 标注“旧版报告，重新扫描可查看变化与范围”）。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 报告 schema 版本。首版带版本号取 2；读取旧 JSON 时 `#[serde(default)]`
/// 回落为 1（见 `schema_version_or_one` 与 `ProjectReport` 的字段定义）。
pub const SCHEMA_VERSION: u32 = 2;

/// 扫描完整度：完整 / 部分完成（个别文件失败但仍继续）/ 失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Complete,
    Partial,
    Failed,
}

impl Default for ScanStatus {
    /// 旧 JSON 缺 `status` 字段时回落 `Complete`：旧报告都是成功扫描产出的。
    fn default() -> Self {
        Self::Complete
    }
}

/// 跳过某个文件的成因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    #[default]
    Ignored,
    UnsupportedLanguage,
    Generated,
    NonUtf8,
    ReadFailed,
    ParseFailed,
}

/// 一个被跳过的文件及其原因。路径为相对项目根目录的规范化路径。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: SkipReason,
}

/// 一种已发现语言的摘要：文件数与是否做了语义分析。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanguageSummary {
    /// 语言标识（小写，如 `"rust"`）。
    pub language: String,
    pub files: usize,
    /// `true` = 该语言产生语义发现（函数/复杂度/框架规则）；
    /// `false` = 仅进入范围统计，未做结构分析（spec「受支持语言分两层」）。
    pub analyzed: bool,
}

/// 一次扫描的元数据。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanMetadata {
    /// 扫描开始时刻（Unix 毫秒）。
    pub started_at_ms: u64,
    /// 扫描耗时（毫秒）。
    pub duration_ms: u64,
    pub status: ScanStatus,
    /// 遍历发现的所有候选文件数（未被 ignore 规则排除前）。
    pub discovered_files: usize,
    /// 真正做了分析的语义文件数。
    pub analyzed_files: usize,
    /// 被排除（ignore/生成文件/不支持的构建目录等）的文件数。
    pub excluded_files: usize,
    /// 跳过并记录原因的文件（读取/解析失败等）。
    pub skipped_files: Vec<SkippedFile>,
    /// 语言统计摘要。
    pub languages: Vec<LanguageSummary>,
}

/// 扫描时的 Git 基准信息。由应用层（`codehealth::git_hotspots`）在扫描前
/// 采集后填入报告；`dozer-codehealth` 自身是纯分析库，不做 git I/O。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitSnapshot {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub dirty: bool,
}

/// 旧报告（schema_version 字段缺失）读取时的回落版本。
pub const LEGACY_SCHEMA_VERSION: u32 = 1;

/// 读取 `schema_version` 的辅助：`Option` 为 `None`（旧 JSON 缺字段）回落 1。
pub fn schema_version_or_one(v: Option<u32>) -> u32 {
    v.unwrap_or(LEGACY_SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ScanStatus::Partial).unwrap(),
            "\"partial\""
        );
        assert_eq!(
            serde_json::to_string(&ScanStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn skip_reason_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SkipReason::UnsupportedLanguage).unwrap(),
            "\"unsupported_language\""
        );
        assert_eq!(
            serde_json::to_string(&SkipReason::NonUtf8).unwrap(),
            "\"non_utf8\""
        );
    }

    #[test]
    fn schema_version_or_one_falls_back_for_missing() {
        assert_eq!(schema_version_or_one(None), 1);
        assert_eq!(schema_version_or_one(Some(2)), 2);
    }

    #[test]
    fn scan_metadata_round_trips() {
        let meta = ScanMetadata {
            started_at_ms: 1,
            duration_ms: 2,
            status: ScanStatus::Complete,
            discovered_files: 10,
            analyzed_files: 8,
            excluded_files: 2,
            skipped_files: vec![SkippedFile {
                path: PathBuf::from("src/bad.rs"),
                reason: SkipReason::ParseFailed,
            }],
            languages: vec![LanguageSummary {
                language: "rust".into(),
                files: 8,
                analyzed: true,
            }],
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: ScanMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
    }
}
