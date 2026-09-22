//! 启动回填用的目录发现:扫描六家按项目建目录的 agent 的存储根目录下所有
//! 项目子目录,外加 Codex 的按日期目录,列出全部 `.jsonl` 文件。跟
//! `dozer_core::agent_paths` 的方向相反——那边是"已知 cwd → 算出该项目的
//! 存储目录"(单项目查询用),这里是"不知道有哪些项目 → 枚举存储根目录下所有
//! 子目录"(启动时全量摄取用)。Codex 不按项目建目录,项目归属只能读文件头
//! `session_meta.payload.cwd` 逐个判断。

use dozer_core::agent_paths::{
    aider_project_dir_in, claude_project_dir_in, codebuddy_project_dir_in, codex_sessions_dir_in,
    goose_project_dir_in, home_dir, opencode_project_dir_in, v8agent_project_dir_in,
};
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

const AGENT_ROOTS: [(AgentKind, &str); 6] = [
    (AgentKind::Claude, ".claude"),
    (AgentKind::Codebuddy, ".codebuddy"),
    (AgentKind::Opencode, ".dozer/agents/opencode"),
    (AgentKind::Goose, ".dozer/agents/goose"),
    (AgentKind::Aider, ".dozer/agents/aider"),
    (AgentKind::V8agent, ".v8agent"),
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

/// 递归列出一个目录树下全部 `.jsonl` 文件。四家 agent 的 transcript 是
/// `<root>/projects/<项目>/s.jsonl` 两层(直接用 `jsonl_files_in` 就够了),
/// Codex 是 `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` 三层嵌套,得递归。
fn jsonl_files_recursive(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(jsonl_files_recursive(&path));
        } else if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    out
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
    // Codex 不按项目建目录,扫整个 sessions 根(`~/.codex/sessions/YYYY/MM/DD/`)
    // 全部摄取——项目归属不在目录层级,而是写在每份文件头的
    // `session_meta.payload.cwd` 里,由 `ingest_session` 落进 `conversations.cwd`
    // 列(见 `agent_paths::codex_sessions_dir_in` 注释)。
    for f in jsonl_files_recursive(&codex_sessions_dir_in(home)) {
        out.push((AgentKind::Codex, f));
    }
    out
}

/// 按项目收窄的文件发现:只看这一个项目在四家按项目建目录的 agent 各自
/// 存储目录下的 `.jsonl` 文件,以及 Codex 里 cwd 命中该项目的 rollout 文件,
/// 跟 `discover_all_transcript_files_in`(扫全部项目)相反方向——这个是
/// "已知 cwd,只要这一个项目的"(见 `agent_paths` 模块头注释里"已知 cwd →
/// 算出该项目的存储目录"这条)。
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
        (AgentKind::Goose, goose_project_dir_in(home, cwd)),
        (AgentKind::Aider, aider_project_dir_in(home, cwd)),
        (AgentKind::V8agent, v8agent_project_dir_in(home, cwd)),
    ] {
        for f in jsonl_files_in(&dir) {
            out.push((agent, f));
        }
    }
    // Codex 没有"项目子目录"这一层(目录按日期建),只能扫整个 sessions 根、
    // 逐个读文件头 `session_meta.payload.cwd` 判断是否属于这个项目——所以
    // 这一家在这里读 `super::codex_session_cwd`(其余四家不需要读文件就能靠
    // 目录判断)。
    let cwd_str = cwd.to_string_lossy();
    for f in jsonl_files_recursive(&codex_sessions_dir_in(home)) {
        if super::codex_session_cwd(&f).as_deref() == Some(cwd_str.as_ref()) {
            out.push((AgentKind::Codex, f));
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

    /// 回归测试：`AGENT_ROOTS` 曾经只有三家(Claude/Codebuddy/Opencode)，
    /// V8agent 的项目子目录永远不会被启动全量回填扫到。
    #[test]
    fn discovers_v8agent_files_alongside_the_other_three_agents() {
        let home = tempfile::tempdir().unwrap();
        let v8agent_proj = home.path().join(".v8agent/projects/-a-b-c");
        std::fs::create_dir_all(&v8agent_proj).unwrap();
        std::fs::write(v8agent_proj.join("s1.jsonl"), "{}").unwrap();

        let found = discover_all_transcript_files_in(home.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::V8agent);
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

    #[test]
    fn discover_project_transcript_files_includes_v8agent() {
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj/a");
        let v8agent_dir = dozer_core::agent_paths::v8agent_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&v8agent_dir).unwrap();
        std::fs::write(v8agent_dir.join("s1.jsonl"), "{}").unwrap();

        let found = discover_project_transcript_files_in(home.path(), cwd);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::V8agent);
    }

    #[test]
    fn discover_all_transcript_files_in_includes_goose() {
        let home = tempfile::tempdir().unwrap();
        let goose_dir = dozer_core::agent_paths::goose_project_dir_in(
            home.path(),
            std::path::Path::new("/proj/a"),
        );
        std::fs::create_dir_all(&goose_dir).unwrap();
        std::fs::write(goose_dir.join("ds.jsonl"), "{}").unwrap();

        let mut found = discover_all_transcript_files_in(home.path());
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::Goose);
    }

    #[test]
    fn discover_project_transcript_files_in_includes_goose_by_dir() {
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj/a");
        let goose_dir = dozer_core::agent_paths::goose_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&goose_dir).unwrap();
        std::fs::write(goose_dir.join("ds.jsonl"), "{}").unwrap();

        let found = discover_project_transcript_files_in(home.path(), cwd);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::Goose);
    }

    #[test]
    fn discover_all_transcript_files_in_includes_aider() {
        let home = tempfile::tempdir().unwrap();
        let aider_dir = dozer_core::agent_paths::aider_project_dir_in(
            home.path(),
            std::path::Path::new("/proj/a"),
        );
        std::fs::create_dir_all(&aider_dir).unwrap();
        std::fs::write(aider_dir.join("ds.jsonl"), "{}").unwrap();
        // Aider 的四条路径里只有 .jsonl 该被捞进来。
        std::fs::write(aider_dir.join("ds.chat.md"), "# aider").unwrap();
        std::fs::write(aider_dir.join("ds.bridge.json"), "{}").unwrap();

        let mut found = discover_all_transcript_files_in(home.path());
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::Aider);
        assert!(found[0].1.ends_with("ds.jsonl"));
    }

    #[test]
    fn discover_project_transcript_files_in_includes_aider_by_dir() {
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj/a");
        let aider_dir = dozer_core::agent_paths::aider_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&aider_dir).unwrap();
        std::fs::write(aider_dir.join("ds.jsonl"), "{}").unwrap();

        let found = discover_project_transcript_files_in(home.path(), cwd);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::Aider);
    }

    /// Codex 不按项目建目录(sessions 按日期三层嵌套),`discover_all_*` 必须
    /// 递归扫整个 sessions 根把 rollout 文件都捞出来——启动全量回填才会吃到
    /// Codex 历史会话,否则只有 hook 事件触发的实时会话进库。
    #[test]
    fn discover_all_transcript_files_in_includes_codex_recursively() {
        let home = tempfile::tempdir().unwrap();
        let session_dir = home.path().join(".codex/sessions/2026/09/21");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(session_dir.join("rollout-1.jsonl"), "{}").unwrap();
        // 非 jsonl 文件不该被捞进来。
        std::fs::write(session_dir.join("rollout-2.txt"), "{}").unwrap();

        let mut found = discover_all_transcript_files_in(home.path());
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::Codex);
    }

    /// Codex 按项目发现只能读文件头 `session_meta.payload.cwd` 判断归属,
    /// 别把 cwd 是其它项目的 rollout 也塞进来。
    #[test]
    fn discover_project_transcript_files_in_includes_codex_by_cwd() {
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj/a");
        let session_dir = home.path().join(".codex/sessions/2026/09/21");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("rollout-a.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/proj/a\"}}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("rollout-b.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/proj/b\"}}\n",
        )
        .unwrap();

        let mut found = discover_project_transcript_files_in(home.path(), cwd);
        found.sort_by_key(|(_, p)| p.to_string_lossy().into_owned());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, AgentKind::Codex);
        assert!(found[0].1.ends_with("rollout-a.jsonl"));
    }
}
