//! 每个项目各自的面板布局(左右视图选择 + 收起态 + 尺寸 `PanelDims`)的本地
//! 持久化——按项目 id 记到 `config_dir()/panel_layouts.json`,与全局窗口尺寸
//! `layout.json` 分离。这样切换项目时只换当前项目的面板状态、不会盖掉别的
//! 项目(见切换项目 bug)。
//!
//! `load()`/`save()` 是真实调用方用的入口(固定读写 `panel_layouts.json`);
//! `load_from`/`save_to` 接收显式路径,供单测指向临时文件,不碰用户真实配置
//! 目录。

use crate::app::{PanelDims, PanelLayout, sanitize_panel_dims};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("panel_layouts.json")
}

pub fn load() -> HashMap<i64, PanelLayout> {
    load_from(&default_path(), &crate::layout::default_path())
}

pub fn save(map: &HashMap<i64, PanelLayout>) -> std::io::Result<()> {
    save_to(&default_path(), map)
}

/// 从旧版全局 `layout.json` 粗读迁移来源(改版前 `ShellLayout` 直接持有
/// `left_width` + 四个 split)。迁移后 `ShellLayout` 反序列化会忽略这些字段,
/// 所以这里单独把原始文件当作 JSON 对象读,缺字段的退回 `PanelDims::default()`。
fn legacy_global_dims(layout_path: &Path) -> PanelDims {
    std::fs::read_to_string(layout_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|v| PanelDims {
            left_width: v.get("left_width").and_then(num).unwrap_or_default(),
            files_split: v.get("files_split").and_then(num).unwrap_or_default(),
            project_split: v.get("project_split").and_then(num).unwrap_or_default(),
            ssh_split: v
                .get("ssh_split")
                .and_then(num)
                .unwrap_or(byteui::theme::geometry::default_split_ratio()),
            todo_split: v.get("todo_split").and_then(num).unwrap_or_default(),
            git_log_split: v.get("git_log_split").and_then(num).unwrap_or_default(),
            git_log_file_diff_split: v
                .get("git_log_file_diff_split")
                .and_then(num)
                .unwrap_or_default(),
            agent_split: v.get("agent_split").and_then(num).unwrap_or_default(),
            conversations_split: v
                .get("conversations_split")
                .and_then(num)
                .unwrap_or_default(),
            browser_bookmarks_split: v
                .get("browser_bookmarks_split")
                .and_then(num)
                .unwrap_or(byteui::theme::geometry::default_split_ratio()),
        })
        .unwrap_or_default()
}

fn num(v: &serde_json::Value) -> Option<f32> {
    v.as_f64().map(|f| f as f32)
}

/// 一个项目"从未存过尺寸"的判据:`#[serde(default)]` 对老 `panel_layouts.json`
/// 缺 `dims` 字段时补的是 `PanelDims::default()`(全默认),与"真存成默认值"
/// 不可区分——但迁移语义一致(都用全局旧值/默认回填,不覆盖),无需区分。
fn dims_missing(d: PanelDims) -> bool {
    d == PanelDims::default()
}

fn load_from(path: &Path, layout_path: &Path) -> HashMap<i64, PanelLayout> {
    // 迁移源:旧全局 layout.json 里用户攒下的左宽 + 四个 split。只作"该项目
    // 从未存过尺寸"时的首次回填,已存尺寸的项目绝不覆盖(见全局约束)。
    let legacy = legacy_global_dims(layout_path);
    let map: HashMap<i64, PanelLayout> = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    map.into_iter()
        .map(|(id, mut pl)| {
            if dims_missing(pl.dims) {
                // 首次:用旧全局值回填(其中缺字段/为 0 的再落 `default_panel_dims`,
                // 由 `sanitize_panel_dims` 把 0 夹到合法下限)。
                let seed = PanelDims {
                    left_width: legacy.left_width.max(PanelDims::default().left_width),
                    ..PanelDims::default()
                };
                pl.dims = seed;
            }
            pl.dims = sanitize_panel_dims(pl.dims);
            (id, pl)
        })
        .collect()
}

fn save_to(path: &Path, map: &HashMap<i64, PanelLayout>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(map).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LeftView, PanelDims, RightView};

    #[test]
    fn load_from_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let layout_path = dir.path().join("layout.json");
        assert!(load_from(&path, &layout_path).is_empty());
    }

    #[test]
    fn load_from_corrupt_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel_layouts.json");
        std::fs::write(&path, "not valid json").unwrap();
        let layout_path = dir.path().join("layout.json");
        assert!(load_from(&path, &layout_path).is_empty());
    }

    #[test]
    fn save_then_load_round_trips_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("panel_layouts.json");
        let mut map = HashMap::new();
        map.insert(
            1,
            PanelLayout {
                left_view: LeftView::Project,
                right_view: RightView::Acceptance,
                left_collapsed: true,
                right_collapsed: false,
                dims: PanelDims::default(),
            },
        );
        map.insert(
            2,
            PanelLayout {
                left_view: LeftView::Files,
                right_view: RightView::Agent,
                left_collapsed: false,
                right_collapsed: true,
                dims: PanelDims::default(),
            },
        );
        save_to(&path, &map).unwrap();
        let layout_path = dir.path().join("layout.json");
        assert_eq!(load_from(&path, &layout_path), map);
    }

    #[test]
    fn per_project_layouts_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel_layouts.json");
        let mut map = HashMap::new();
        map.insert(
            1,
            PanelLayout {
                left_view: LeftView::Project,
                right_view: RightView::Acceptance,
                left_collapsed: true,
                right_collapsed: false,
                dims: PanelDims::default(),
            },
        );
        save_to(&path, &map).unwrap();
        // 只存了项目 1,项目 2 不存在(读回应是 None),互不影响。
        let layout_path = dir.path().join("layout.json");
        let loaded = load_from(&path, &layout_path);
        assert!(loaded.contains_key(&1));
        assert!(!loaded.contains_key(&2));
    }
}
