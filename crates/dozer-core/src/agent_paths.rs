//! agent 各自的会话存储目录路径计算(从 `dozer-app/src/conversation.rs`
//! 搬出,供 dozerd 摄取扫描与 dozer-app `links.rs` 的记忆目录探测共用)。

use std::path::{Path, PathBuf};

pub fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn project_key(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}

/// Claude Code 自己的目录命名规则比 `project_key` 更激进:除了 `/` 之外,
/// 路径里的 `_`(以及其他非字母数字字符)也会被换成 `-`——实测
/// `~/.claude/projects/` 下的真实目录名核实(如本地目录
/// `.../Anrong/anrong_finagent` 对应的真实会话目录是
/// `...-Anrong-anrong-finagent`,下划线也被换成了短横线)。这条规则只
/// 属于 Claude Code 自己的存储约定,不能挪去改 `project_key`——
/// opencode/v8agent 用 `project_key` 是 dozerd 自己定的存储目录,已有数据
/// 用的是斜杠替换版本,改了会让既有目录对不上。
fn claude_project_key(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// CodeBuddy 自己的目录命名规则跟 Claude 不一样:Claude 把开头的 `/` 也
/// 一并换成 `-`(留下开头一个 `-`),CodeBuddy 是先去掉开头 `/` 再替换
/// 剩余的 `/`(不留开头 `-`)——实测 `~/.codebuddy/projects/` 下的真实
/// 目录名核实。
fn codebuddy_project_key(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-")
}

fn project_dir_in(home: &Path, agent_root: &str, cwd: &Path) -> PathBuf {
    home.join(agent_root)
        .join("projects")
        .join(project_key(cwd))
}

pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    claude_project_dir_in(&home_dir(), cwd)
}

pub fn claude_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(claude_project_key(cwd))
}

pub fn codebuddy_project_dir(cwd: &Path) -> PathBuf {
    codebuddy_project_dir_in(&home_dir(), cwd)
}

pub fn codebuddy_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    home.join(".codebuddy")
        .join("projects")
        .join(codebuddy_project_key(cwd))
}

pub fn opencode_project_dir(cwd: &Path) -> PathBuf {
    opencode_project_dir_in(&home_dir(), cwd)
}

pub fn opencode_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".dozer/agents/opencode", cwd)
}

pub fn v8agent_project_dir(cwd: &Path) -> PathBuf {
    v8agent_project_dir_in(&home_dir(), cwd)
}

/// v8agent 用 Claude 同款编码（`project_key`：斜杠换成短横线，保留开头的
/// `-`）——v8agent 没有 CodeBuddy 那种"去掉开头斜杠"的特殊需求。
pub fn v8agent_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".v8agent", cwd)
}

/// Goose 的 hook journal 存储目录:`~/.dozer/agents/goose/projects/<cwd-key>/`
/// 下,每个 Dozer 会话一份 `<dozer-session-id>.jsonl`(见 spec D4)。这是
/// Dozer 自有的 journal,不读也不写 Goose 自己的 `sessions.db`。
pub fn goose_project_dir(cwd: &Path) -> PathBuf {
    goose_project_dir_in(&home_dir(), cwd)
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn goose_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    project_dir_in(home, ".dozer/agents/goose", cwd)
}

/// Codex 的 transcript 存储根目录。**故意没有** `codex_project_dir_in`:
/// Codex 不按项目建目录,而是 `sessions/YYYY/MM/DD/rollout-*.jsonl` 按日期
/// 三层嵌套(实测本机 43 份 rollout 全是这个布局,见 spike 记录
/// `docs/superpowers/specs/2026-08-07-codex-spike-findings.md`),项目归属只
/// 写在每份文件头部 `session_meta.payload.cwd` 里。所以"按项目查 Codex 会话"
/// 算不出目录,只能靠摄取时把 cwd 落进 `conversations.cwd` 列、查询侧按 cwd
/// 过滤(dozerd `transcripts/mod.rs::list_conversations_in`)。
pub fn codex_sessions_dir() -> PathBuf {
    codex_sessions_dir_in(&home_dir())
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn codex_sessions_dir_in(home: &Path) -> PathBuf {
    home.join(".codex").join("sessions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn claude_project_dir_maps_underscores_to_dashes_too() {
        let d = claude_project_dir(Path::new("/a/b/anrong_finagent"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.claude/projects/-a-b-anrong-finagent")
        );
    }

    #[test]
    fn opencode_dir_keeps_underscores_unlike_claude() {
        let d = opencode_project_dir(Path::new("/a/b/anrong_finagent"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.dozer/agents/opencode/projects/-a-b-anrong_finagent")
        );
    }

    #[test]
    fn codebuddy_dir_uses_codebuddy_root() {
        let d = codebuddy_project_dir(Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.codebuddy/projects/a-b-c"));
    }

    #[test]
    fn opencode_dir_lives_under_dozer_data_dir() {
        let d = opencode_project_dir(Path::new("/a/b/c"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.dozer/agents/opencode/projects/-a-b-c")
        );
    }

    #[test]
    fn v8agent_dir_uses_claude_style_encoding_under_its_own_root() {
        let d = v8agent_project_dir(Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.v8agent/projects/-a-b-c"));
    }

    #[test]
    fn goose_project_dir_lives_under_dozer_data_dir() {
        let d = goose_project_dir_in(Path::new("/home/u"), Path::new("/a/b/c"));
        assert_eq!(
            d,
            PathBuf::from("/home/u/.dozer/agents/goose/projects/-a-b-c")
        );
    }

    /// Codex 的存储根不参与 `project_key` 编码(它压根没有"项目子目录"这一
    /// 层,会话按日期嵌套)。锁定这个形状,免得以后有人顺手补一个
    /// `codex_project_dir_in`、算出一个磁盘上根本不存在的目录当查询键。
    #[test]
    fn codex_sessions_dir_is_a_flat_root_without_project_key() {
        let d = codex_sessions_dir_in(Path::new("/home/u"));
        assert_eq!(d, PathBuf::from("/home/u/.codex/sessions"));
    }
}
