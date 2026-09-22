//! 预览 tab(仅文件类)按项目本地持久化——重启/切项目后认回上次打开的
//! 文件。`load`/`save` 是真实调用方入口(固定读写
//! `dozer_core::paths::config_dir()/preview_state/<project_id>.json`);
//! `load_from`/`save_to` 接收显式路径,供单测指向临时文件。
//!
//! Phase A 起 schema 前向兼容:旧 `{paths, active_path}` 仍可读入并迁移成
//! [`PersistedPreviewTab`] 描述符;保存后只写新版。本阶段仍按现有方式加载
//! 内容,不提前引入 Suspended 行为(那是 Phase C)。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::preview::PreviewMode;

/// 当前 schema 版本。
pub const PREVIEW_STATE_VERSION: u32 = 1;

/// 一个持久化的预览 tab 描述符。Phase A 只填 `path`/`mode`/`active` 三项,
/// 其余字段允许缺省,留给 Phase C 恢复光标/选区/滚动/折叠。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PersistedPreviewTab {
    pub path: PathBuf,
    /// 用户在该 tab 上持久选择的 mode(未知/失效值安全落回默认)。
    #[serde(
        default,
        deserialize_with = "deserialize_mode",
        skip_serializing_if = "Option::is_none"
    )]
    pub mode: Option<PreviewMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<TextPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<TextRange>,
    /// 逻辑滚动锚点(top line),不持久化像素值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_anchor: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<FoldRange>,
    /// 上次持久化时的文件 revision。
    #[serde(default)]
    pub revision: u64,
    pub active: bool,
}

/// 文本位置(1-based,与对外协议一致)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

/// 文本选区。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}

/// 折叠区(行范围)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldRange {
    pub from_line: u32,
    pub to_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewState {
    /// 0 = 旧 schema(无该字段),load 时归一为当前版本。
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub tabs: Vec<PersistedPreviewTab>,
    #[serde(default)]
    pub active_path: Option<PathBuf>,
    /// 旧 schema(v0)遗留字段,只读兼容;保存恒为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathBuf>,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            version: PREVIEW_STATE_VERSION,
            tabs: Vec::new(),
            active_path: None,
            paths: Vec::new(),
        }
    }
}

impl PreviewState {
    /// 旧版 `{paths, active_path}` -> 新 descriptor;已是新版则只更新 version。
    fn normalize(mut self) -> Self {
        if self.tabs.is_empty() && !self.paths.is_empty() {
            let active = self.active_path.clone();
            self.tabs = self
                .paths
                .drain(..)
                .map(|path| {
                    let is_active = active.as_deref() == Some(path.as_path());
                    PersistedPreviewTab {
                        path,
                        active: is_active,
                        ..Default::default()
                    }
                })
                .collect();
        }
        self.version = PREVIEW_STATE_VERSION;
        self
    }
}

/// 宽容解析 mode:未知字符串回退 `None`,由路由落回默认,不 panic。
fn deserialize_mode<'de, D>(deserializer: D) -> Result<Option<PreviewMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.and_then(|s| PreviewMode::from_persisted(&s)))
}

fn state_path(project_id: i64) -> PathBuf {
    dozer_core::paths::config_dir()
        .join("preview_state")
        .join(format!("{project_id}.json"))
}

pub fn load(project_id: i64) -> PreviewState {
    load_from(&state_path(project_id))
}

pub fn save(project_id: i64, state: &PreviewState) -> std::io::Result<()> {
    save_to(&state_path(project_id), state)
}

fn load_from(path: &Path) -> PreviewState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PreviewState>(&s).ok())
        .unwrap_or_default()
        .normalize()
}

fn save_to(path: &Path, state: &PreviewState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 保存只写新版:清空 legacy paths,version 固定当前值。
    let mut out = state.clone();
    out.version = PREVIEW_STATE_VERSION;
    out.paths.clear();
    let s = serde_json::to_string_pretty(&out).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(load_from(&path), PreviewState::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert_eq!(load_from(&path), PreviewState::default());
    }

    #[test]
    fn legacy_schema_is_migrated_to_descriptors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        std::fs::write(
            &path,
            r#"{"paths":["/repo/a.rs","/repo/README.md"],"active_path":"/repo/README.md"}"#,
        )
        .unwrap();
        let state = load_from(&path);
        assert_eq!(state.version, PREVIEW_STATE_VERSION);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.tabs[0].path, PathBuf::from("/repo/a.rs"));
        assert!(!state.tabs[0].active);
        assert_eq!(state.tabs[1].path, PathBuf::from("/repo/README.md"));
        assert!(state.tabs[1].active);
        assert!(state.tabs[0].mode.is_none());
    }

    #[test]
    fn new_schema_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("preview_state.json");
        let state = PreviewState {
            version: PREVIEW_STATE_VERSION,
            tabs: vec![
                PersistedPreviewTab {
                    path: PathBuf::from("/repo/main.rs"),
                    mode: Some(PreviewMode::Code),
                    active: false,
                    ..Default::default()
                },
                PersistedPreviewTab {
                    path: PathBuf::from("/repo/README.md"),
                    mode: Some(PreviewMode::Source),
                    cursor: Some(TextPosition { line: 3, column: 7 }),
                    selection: Some(TextRange {
                        start: TextPosition { line: 3, column: 1 },
                        end: TextPosition { line: 3, column: 7 },
                    }),
                    scroll_anchor: Some(120),
                    folds: vec![FoldRange {
                        from_line: 10,
                        to_line: 20,
                    }],
                    revision: 4,
                    active: true,
                },
            ],
            active_path: Some(PathBuf::from("/repo/README.md")),
            paths: Vec::new(),
        };
        save_to(&path, &state).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, state);
    }

    #[test]
    fn missing_optional_fields_default_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"tabs":[{"path":"/repo/a.rs","active":true}]}"#,
        )
        .unwrap();
        let state = load_from(&path);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].mode, None);
        assert_eq!(state.tabs[0].revision, 0);
        assert!(state.tabs[0].folds.is_empty());
    }

    #[test]
    fn unknown_mode_falls_back_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"tabs":[{"path":"/repo/a.rs","mode":"hologram","active":false}]}"#,
        )
        .unwrap();
        let state = load_from(&path);
        assert_eq!(state.tabs[0].mode, None, "未知 mode 安全落回 None");
    }

    #[test]
    fn saved_file_only_contains_new_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        let legacy = PreviewState {
            version: 0,
            tabs: Vec::new(),
            active_path: Some(PathBuf::from("/repo/a.rs")),
            paths: vec![PathBuf::from("/repo/a.rs")],
        };
        save_to(&path, &legacy).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("\"paths\""));
        assert!(raw.contains("\"version\""));
    }
}
