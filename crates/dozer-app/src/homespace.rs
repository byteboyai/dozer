// crates/dozer-app/src/homespace.rs
//! 首页落地页(`AppPage::Home`,点顶栏 Dozer 页签进入)专属代码:类型定义与
//! 全部视图构建函数。`App` struct 字段声明/`Message` 枚举/`App::update` 的
//! 消息处理逻辑仍留在 `workspace.rs`(单一数据源+集中调度),拆分边界比照
//! `preview.rs`/`conversation.rs` 的既有先例——本文件不持有 `App`/`Workspace`
//! 的 `impl` 块。见 `docs/superpowers/specs/2026-08-08-home-page-4col-layout-design.md`。

use crate::conversation::{self, ConversationMeta};
use crate::delivery;
use dozer_core::protocol::ProjectInfo;
use std::path::PathBuf;

/// H0"最近的文件"卡一行(跨项目合并前的中间表示；D4)。`Message::
/// HomeRecentsLoaded` 的载荷用到它，因此至少是 `pub(crate)`(见 `private_interfaces`)。
/// 字段本身也是 `pub(crate)`——`workspace.rs` 里画卡片的视图函数要读它们。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentFile {
    pub(crate) path: PathBuf,
    pub(crate) project_name: String,
    pub(crate) modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentConversation {
    pub(crate) project_name: String,
    pub(crate) meta: ConversationMeta,
}

/// D4 纯 IO 内核：对给定项目列表分别取"最近改动的文件"(git 改动/未跟踪 +
/// fs mtime)与"最近的对话"(三个 agent 来源已聚合、按 mtime 倒序)，跨项目
/// 合并后各自按时间倒序，取前 4 条 / 前 3 条(对齐 Figma 卡片行数)。
///
/// 必须在 `spawn_blocking` 里跑，不能在 UI 线程直呼——内部既有阻塞 git
/// 子进程调用，也有阻塞文件系统调用。签名固定(`&[ProjectInfo]` 输入，两个
/// `Vec` 输出)方便 headless 单测：不需要 daemon 连接或 winit `EventLoopProxy`。
/// `pub(crate)`——`workspace.rs` 的 `Message::TopBarHome` 处理器在
/// `spawn_blocking` 闭包里直接调用它。
pub(crate) fn load_home_recents(
    projects: &[ProjectInfo],
) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>) {
    let mut files: Vec<HomeRecentFile> = Vec::new();
    let mut convs: Vec<HomeRecentConversation> = Vec::new();
    for p in projects {
        let cwd = PathBuf::from(&p.path);
        if let Some(repo) = delivery::repo_root(&cwd) {
            for (path, _status) in delivery::file_statuses(&repo) {
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue; // 路径已在磁盘消失(用户手动删了),静默跳过(spec §4)
                };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                files.push(HomeRecentFile {
                    path,
                    project_name: p.name.clone(),
                    modified_ms,
                });
            }
        }
        for meta in conversation::list_all_conversations(&cwd) {
            convs.push(HomeRecentConversation {
                project_name: p.name.clone(),
                meta,
            });
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    files.truncate(4);
    convs.sort_by_key(|c| std::cmp::Reverse(c.meta.modified_ms));
    convs.truncate(3);
    (files, convs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_home_recents_empty_input_returns_empty_vecs() {
        let (files, convs) = load_home_recents(&[]);
        assert!(files.is_empty());
        assert!(convs.is_empty());
    }

    #[test]
    fn load_home_recents_merges_and_sorts_across_projects() {
        let proj_a = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj_a.path())
            .status()
            .unwrap();
        std::fs::write(proj_a.path().join("a.txt"), "changed").unwrap();

        let proj_b = tempfile::tempdir().unwrap(); // 非 git 目录,没有改动可报告

        let projects = vec![
            ProjectInfo {
                id: 1,
                path: proj_a.path().to_string_lossy().into_owned(),
                name: "proj-a".into(),
                last_active_ms: 0,
            },
            ProjectInfo {
                id: 2,
                path: proj_b.path().to_string_lossy().into_owned(),
                name: "proj-b".into(),
                last_active_ms: 0,
            },
        ];

        let (files, convs) = load_home_recents(&projects);
        assert_eq!(files.len(), 1, "只有项目 A(git repo)贡献一条改动文件");
        assert_eq!(files[0].project_name, "proj-a");
        assert!(files[0].path.ends_with("a.txt"));
        assert!(convs.is_empty(), "两个项目都没有可达的 agent 对话目录");
    }

    #[test]
    fn load_home_recents_truncates_files_to_top_4() {
        let proj = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj.path())
            .status()
            .unwrap();
        for i in 0..6 {
            std::fs::write(proj.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let projects = vec![ProjectInfo {
            id: 1,
            path: proj.path().to_string_lossy().into_owned(),
            name: "proj".into(),
            last_active_ms: 0,
        }];
        let (files, _convs) = load_home_recents(&projects);
        assert_eq!(
            files.len(),
            4,
            "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)"
        );
    }
}
