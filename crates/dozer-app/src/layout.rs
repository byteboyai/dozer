//! 外壳布局（左面板区宽度、三个配对视图各自的内部分割比例、左右视图选择
//! 与收起态）的本地持久化——应用级偏好，不属于任何项目/窗口。
//! `load()`/`save()` 是真实调用方用的入口（固定读写
//! `dozer_core::paths::config_dir()/layout.json`）；`load_from`/`save_to`
//! 接收显式路径，供单测指向临时文件，不碰用户真实配置目录。

use crate::app::{ShellLayout, sanitize_shell_layout};
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("layout.json")
}

pub fn load() -> ShellLayout {
    load_from(&default_path())
}

pub fn save(layout: &ShellLayout) -> std::io::Result<()> {
    save_to(&default_path(), layout)
}

/// 读盘并**消毒**:磁盘上的值不一定是本程序写的(手改过、别的版本写的、
/// 半截写坏的),分割比例若正好是 0.0/1.0,渲染侧的 `FillPortion(0)` 会让
/// 配对里的一块彻底消失(0 权重拿不到任何空间);`left_width` 若小于最小
/// 区宽,左面板区会挤成一条缝。正常拖拽路径本就被夹在合法范围内,这里只是
/// 把"不是正常路径写进来的值"挡在渲染之前。
fn load_from(path: &Path) -> ShellLayout {
    let loaded: ShellLayout = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    sanitize_shell_layout(loaded)
}

fn save_to(path: &Path, layout: &ShellLayout) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(layout).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(load_from(&path), ShellLayout::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert_eq!(load_from(&path), ShellLayout::default());
    }

    /// 手改过/别的版本写的 layout.json:缺字段靠 `#[serde(default)]` 补齐
    /// (不再整份读失败把用户攒的宽度全重置);窗口尺寸非法值靠
    /// `sanitize_shell_layout` 夹回合法范围。`ShellLayout` 迁走面板尺寸后
    /// 只剩窗口尺寸,老 layout.json 里可能还带着 `left_width`/`files_split`
    /// 等已迁移走的字段,serde 忽略未知字段、缺字段补默认,不应整份失败。
    #[test]
    fn load_from_foreign_json_fills_defaults_and_sanitizes_window_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        // 老文件带已迁移走的尺寸字段 + 越界的 window_width(0)。
        std::fs::write(
            &path,
            r#"{"left_width": 12.0, "files_split": 0.0, "window_width": 0.0}"#,
        )
        .unwrap();
        let l = load_from(&path);
        let (init_w, init_h) = crate::theme::geometry::initial_window_size();
        assert_eq!(l.window_width, init_w, "0 窗口宽应退化成初始尺寸");
        assert_eq!(l.window_height, init_h);
        // 已迁移走的字段在 `ShellLayout` 里已不存在,反序列化应直接忽略。
    }

    #[test]
    fn shell_layout_default_has_sane_values() {
        let l = ShellLayout::default();
        assert!(l.window_width >= crate::theme::geometry::min_window_width());
        assert!(l.window_height > 0.0);
    }

    #[test]
    fn shell_layout_save_then_load_round_trips_window_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("shell_layout.json");
        let layout = ShellLayout {
            window_width: 1600.0,
            window_height: 1000.0,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }

    #[test]
    fn shell_layout_persists_window_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell_layout.json");
        let layout = ShellLayout {
            window_width: 1800.0,
            window_height: 1100.0,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }

    /// 老 `layout.json` 缺 `window_width`/`window_height`(改动前写的文件):
    /// `#[serde(default)]` 补 0.0,`sanitize_shell_layout` 的 `> 0.0` 判断
    /// 把它退化成 `INITIAL_WINDOW_SIZE`,而不是任由一个 0×0 的窗口建出来。
    #[test]
    fn load_from_json_missing_window_size_falls_back_to_initial() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        std::fs::write(&path, r#"{"left_width": 500.0}"#).unwrap();
        let l = load_from(&path);
        let (init_w, init_h) = crate::theme::geometry::initial_window_size();
        assert_eq!(l.window_width, init_w);
        assert_eq!(l.window_height, init_h);
    }
}
