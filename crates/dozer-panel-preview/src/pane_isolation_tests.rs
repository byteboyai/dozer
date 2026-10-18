//! 库级新增测试(Task 3):两个窗格互不影响、越界操作不 panic。

use super::*;
use std::path::PathBuf;

#[test]
fn two_panes_do_not_share_state() {
    let mut files = PreviewPane::default();
    let mut project = PreviewPane::default();
    files.open_path(PathBuf::from("/tmp/files-only.md"));
    project.open_path(PathBuf::from("/tmp/project-a.md"));
    project.open_path(PathBuf::from("/tmp/project-b.md"));
    assert_eq!(files.tabs().len(), 2, "占位 + 1");
    assert_eq!(project.tabs().len(), 3, "占位 + 2");
    files.open_find_on_active(false, Some(1));
    assert!(files.find_bar_open());
    assert!(!project.find_bar_open(), "Files 开 Find 不影响 Project");
    project.close(1);
    assert_eq!(files.tabs().len(), 2, "关 Project 的 tab 不动 Files");
}

#[test]
fn reorder_out_of_range_is_a_noop() {
    let mut p = PreviewPane::default();
    p.open_path(PathBuf::from("/tmp/a.md"));
    p.open_path(PathBuf::from("/tmp/b.md"));
    let before: Vec<usize> = p.tabs().iter().map(|t| t.id).collect();
    p.reorder(1, 99);
    p.reorder(99, 1);
    p.reorder(0, 2);
    p.reorder(2, 0);
    let after: Vec<usize> = p.tabs().iter().map(|t| t.id).collect();
    assert_eq!(before, after);
}

#[test]
fn close_clamps_the_active_index_and_never_panics_on_the_last_tab() {
    let mut p = PreviewPane::default();
    p.open_path(PathBuf::from("/tmp/a.md"));
    p.open_path(PathBuf::from("/tmp/b.md"));
    p.close(2);
    assert!(p.active_idx() < p.tabs().len());
    p.close(1);
    p.close(0);
    p.close(0);
    assert_eq!(p.tabs().len(), 1, "恒留一个占位");
    assert_eq!(p.active_idx(), 0);
}
