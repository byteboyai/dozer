//! 启动回填用的目录发现:扫描三家 agent 的存储根目录下所有项目子目录,
//! 列出全部 `.jsonl` 文件。跟 `dozer_core::agent_paths` 的方向相反——
//! 那边是"已知 cwd → 算出该项目的存储目录"(单项目查询用),这里是
//! "不知道有哪些项目 → 枚举存储根目录下所有子目录"(启动时全量摄取用)。

use dozer_core::agent_paths::{
    claude_project_dir_in, codebuddy_project_dir_in, home_dir, opencode_project_dir_in,
};
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

const AGENT_ROOTS: [(AgentKind, &str); 3] = [
    (AgentKind::Claude, ".claude"),
    (AgentKind::Codebuddy, ".codebuddy"),
    (AgentKind::Opencode, ".dozer/agents/opencode"),
];

pub(crate) fn jsonl_files_in(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .collect()
}

pub fn discover_all_transcript_files() -> Vec<(AgentKind, PathBuf)> {
    discover_all_transcript_files_in(&home_dir())
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn discover_all_transcript_files_in(home: &std::path::Path) -> Vec<(AgentKind, PathBuf)> {
    let mut out = Vec::new();
    for (agent, root) in AGENT_ROOTS {
        let projects_dir = home.join(root).join("projects");
        let Ok(rd) = std::fs::read_dir(&projects_dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            for f in jsonl_files_in(&dir) {
                out.push((agent, f));
            }
        }
    }
    out
}

/// 按项目收窄的文件发现:只看这一个项目在三家 agent 各自存储目录下的
/// `.jsonl` 文件,跟 `discover_all_transcript_files_in`(扫全部项目)相反
/// 方向——这个是"已知 cwd,只要这一个项目的"(见 `agent_paths` 模块头
/// 注释里"已知 cwd → 算出该项目的存储目录"这条)。
pub fn discover_project_transcript_files(cwd: &std::path::Path) -> Vec<(AgentKind, PathBuf)> {
    discover_project_transcript_files_in(&home_dir(), cwd)
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn discover_project_transcript_files_in(
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> Vec<(AgentKind, PathBuf)> {
    let mut out = Vec::new();
    for (agent, dir) in [
        (AgentKind::Claude, claude_project_dir_in(home, cwd)),
        (AgentKind::Codebuddy, codebuddy_project_dir_in(home, cwd)),
        (AgentKind::Opencode, opencode_project_dir_in(home, cwd)),
    ] {
        for f in jsonl_files_in(&dir) {
            out.push((agent, f));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_jsonl_files_across_three_agent_roots() {
        let home = tempfile::tempdir().unwrap();
        let claude_proj = home.path().join(".claude/projects/-a-b-c");
        let codebuddy_proj = home.path().join(".codebuddy/projects/a-b-c");
        std::fs::create_dir_all(&claude_proj).unwrap();
        std::fs::create_dir_all(&codebuddy_proj).unwrap();
        std::fs::write(claude_proj.join("s1.jsonl"), "{}").unwrap();
        std::fs::write(codebuddy_proj.join("s2.jsonl"), "{}").unwrap();
        std::fs::write(claude_proj.join("not-jsonl.txt"), "x").unwrap();

        let mut found = discover_all_transcript_files_in(home.path());
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 2);
        // 按路径字典序:`.claude` < `.codebuddy`(第 3 个字符 l < o)。
        assert_eq!(found[0].0, AgentKind::Claude);
        assert_eq!(found[1].0, AgentKind::Codebuddy);
    }

    #[test]
    fn missing_agent_root_yields_no_entries_for_it() {
        let home = tempfile::tempdir().unwrap();
        assert!(discover_all_transcript_files_in(home.path()).is_empty());
    }

    #[test]
    fn discover_project_transcript_files_scans_only_that_projects_agent_dirs() {
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj/a");
        let claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        let codebuddy_dir = dozer_core::agent_paths::codebuddy_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codebuddy_dir).unwrap();
        std::fs::write(claude_dir.join("s1.jsonl"), "{}").unwrap();
        std::fs::write(codebuddy_dir.join("s2.jsonl"), "{}").unwrap();
        // 别的项目的目录不该被扫进来。
        let other_cwd = std::path::Path::new("/proj/b");
        let other_claude_dir =
            dozer_core::agent_paths::claude_project_dir_in(home.path(), other_cwd);
        std::fs::create_dir_all(&other_claude_dir).unwrap();
        std::fs::write(other_claude_dir.join("s3.jsonl"), "{}").unwrap();

        let mut found = discover_project_transcript_files_in(home.path(), cwd);
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, AgentKind::Claude);
        assert_eq!(found[1].0, AgentKind::Codebuddy);
    }

    #[test]
    fn discover_project_transcript_files_empty_when_no_agent_dirs_exist() {
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj/never-opened");
        assert!(discover_project_transcript_files_in(home.path(), cwd).is_empty());
    }
}
