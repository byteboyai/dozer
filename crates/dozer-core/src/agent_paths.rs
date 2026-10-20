//! agent 各自的会话存储目录路径计算(从 `dozer-app/src/conversation.rs`
//! 搬出,供 dozerd 摄取扫描与 dozer-app `links.rs` 的记忆目录探测共用)。

use std::path::{Path, PathBuf};

use crate::protocol::AgentKind;

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

/// 本产品存放自有 agent journal 的根目录(方案 A,用户 2026-10-20 裁决):
/// Dozer 是 <home>/.dozer/agents/<agent>,Digger 是
/// <home>/.digger/agents/<agent>——根目录名随产品身份推导,两个产品
/// 同机时 journal 互不混目录。只有本产品自己写 journal 的三家
/// (opencode/goose/aider)从这里取路径;Claude/CodeBuddy/Codex/v8agent
/// 读的是各 agent 自己的目录(如 ~/.claude/projects),天然被所有产品
/// 共享,不属于产品数据目录,不走这里。项目目录里的 .dozer/*.toml
/// 等项目级配置同理,不属于本函数管辖。
fn product_agents_root_in(home: &Path, agent: &str) -> PathBuf {
    home.join(format!(".{}", crate::paths::product().name))
        .join("agents")
        .join(agent)
}

/// journal 三家(opencode/goose/aider)共用的项目子目录构造。
fn journal_project_dir_in(home: &Path, agent: &str, cwd: &Path) -> PathBuf {
    product_agents_root_in(home, agent)
        .join("projects")
        .join(project_key(cwd))
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
    journal_project_dir_in(home, "opencode", cwd)
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
    journal_project_dir_in(home, "goose", cwd)
}

/// Aider 一个 Dozer 会话的四条路径(见 spec D2)。`chat_history`/`input_history`
/// 喂给 aider CLI,`canonical` 是 Dozer 自有 transcript(唯一进 dozerd parser),
/// `bridge_state` 是同步 cursor/已输出 hash/schema version。四条都是相邻
/// `PathBuf`,用具名结构体承载而不是四个位置参数,避免调用方顺序传错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiderSessionPaths {
    pub chat_history: PathBuf,
    pub input_history: PathBuf,
    pub canonical: PathBuf,
    pub bridge_state: PathBuf,
}

pub fn aider_project_dir(cwd: &Path) -> PathBuf {
    aider_project_dir_in(&home_dir(), cwd)
}

/// `home` 显式传入版本,测试用(不碰 `HOME` 环境变量)。
pub fn aider_project_dir_in(home: &Path, cwd: &Path) -> PathBuf {
    journal_project_dir_in(home, "aider", cwd)
}

/// Aider session ID 校验:只接受 Dozer 现有安全字符集(字母数字 + `-`,覆盖
/// UUID 与 `task-<id>-<ts>` 形态)。任何 `/`、`\`、`.`(含 `..`)、NUL、空格
/// 或空串都会被拒绝——`aider_session_paths` 用它防止路径穿越(见 spec §6)。
pub fn validate_aider_session_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 由 `home`/`cwd`/`session_id` 算出 Aider 会话的四条路径。session_id 非法
/// (路径分隔符/`..`/NUL/空串)返回 `None`,不 panic。
pub fn aider_session_paths(home: &Path, cwd: &Path, session_id: &str) -> Option<AiderSessionPaths> {
    if !validate_aider_session_id(session_id) {
        return None;
    }
    let dir = aider_project_dir_in(home, cwd);
    Some(AiderSessionPaths {
        chat_history: dir.join(format!("{session_id}.chat.md")),
        input_history: dir.join(format!("{session_id}.input.history")),
        canonical: dir.join(format!("{session_id}.jsonl")),
        bridge_state: dir.join(format!("{session_id}.bridge.json")),
    })
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

/// 各家 agent 的"按项目建子目录"根:`<home>/<root>/projects`。供启动全量扫描枚举所有项目。
/// 无通配 `match`:新增 `AgentKind` 变体时必须在这里表态。
pub fn projects_root_in(agent: AgentKind, home: &Path) -> Option<PathBuf> {
    // journal 三家走产品根目录(方案 A);其余各家读 agent 自己的目录,
    // 与产品无关。
    Some(match agent {
        AgentKind::Opencode => product_agents_root_in(home, "opencode").join("projects"),
        AgentKind::Goose => product_agents_root_in(home, "goose").join("projects"),
        AgentKind::Aider => product_agents_root_in(home, "aider").join("projects"),
        AgentKind::Claude => home.join(".claude").join("projects"),
        AgentKind::Codebuddy => home.join(".codebuddy").join("projects"),
        AgentKind::V8agent => home.join(".v8agent").join("projects"),
        // Codex 按日期建目录(见 `codex_sessions_dir_in`);Qodercn 目前只做到 L1,
        // 没有 transcript 解析;Unknown 不是 agent。
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Qodercn => return None,
    })
}

/// 某项目(`cwd`)在某家 agent 下的会话目录。`None` 的集合与 `projects_root_in` 一致。
/// 各家的目录编码规则不同(见 `claude_project_key` / `codebuddy_project_key`),不能统一。
pub fn project_session_dir_in(agent: AgentKind, home: &Path, cwd: &Path) -> Option<PathBuf> {
    Some(match agent {
        AgentKind::Claude => claude_project_dir_in(home, cwd),
        AgentKind::Codebuddy => codebuddy_project_dir_in(home, cwd),
        AgentKind::Opencode => opencode_project_dir_in(home, cwd),
        AgentKind::Goose => goose_project_dir_in(home, cwd),
        AgentKind::Aider => aider_project_dir_in(home, cwd),
        AgentKind::V8agent => v8agent_project_dir_in(home, cwd),
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Qodercn => return None,
    })
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

    #[test]
    fn aider_session_paths_build_four_paths_under_project_dir() {
        let p = aider_session_paths(Path::new("/home/u"), Path::new("/a/b/c"), "sess-1").unwrap();
        let dir = PathBuf::from("/home/u/.dozer/agents/aider/projects/-a-b-c");
        assert_eq!(p.chat_history, dir.join("sess-1.chat.md"));
        assert_eq!(p.input_history, dir.join("sess-1.input.history"));
        assert_eq!(p.canonical, dir.join("sess-1.jsonl"));
        assert_eq!(p.bridge_state, dir.join("sess-1.bridge.json"));
    }

    #[test]
    fn validate_aider_session_id_accepts_uuid_and_task_forms() {
        assert!(validate_aider_session_id(
            "3f2cf51f-9d1e-4c3b-8f0a-1b2c3d4e5f6a"
        ));
        assert!(validate_aider_session_id("task-42-1790030000000"));
    }

    #[test]
    fn validate_aider_session_id_rejects_unsafe_forms() {
        assert!(!validate_aider_session_id(""));
        assert!(!validate_aider_session_id("../etc/passwd"));
        assert!(!validate_aider_session_id("a/b"));
        assert!(!validate_aider_session_id("a\\b"));
        assert!(!validate_aider_session_id("a..b"));
        assert!(!validate_aider_session_id("a b"));
        assert!(!validate_aider_session_id("a\u{0}b"));
    }

    #[test]
    fn aider_session_paths_rejects_unsafe_session_id() {
        assert!(aider_session_paths(Path::new("/home/u"), Path::new("/a"), "../x").is_none());
    }

    /// Codex 的存储根不参与 `project_key` 编码(它压根没有"项目子目录"这一
    /// 层,会话按日期嵌套)。锁定这个形状,免得以后有人顺手补一个
    /// `codex_project_dir_in`、算出一个磁盘上根本不存在的目录当查询键。
    #[test]
    fn codex_sessions_dir_is_a_flat_root_without_project_key() {
        let d = codex_sessions_dir_in(Path::new("/home/u"));
        assert_eq!(d, PathBuf::from("/home/u/.codex/sessions"));
    }

    #[test]
    fn project_session_dir_sits_under_projects_root_for_every_agent() {
        let home = Path::new("/h");
        let cwd = Path::new("/a/b");
        for a in AgentKind::ALL {
            let root = projects_root_in(a, home);
            let dir = project_session_dir_in(a, home, cwd);
            assert_eq!(
                root.is_some(),
                dir.is_some(),
                "{a:?}: 两个表的覆盖范围必须一致"
            );
            if let (Some(r), Some(d)) = (root, dir) {
                assert_eq!(d.parent(), Some(r.as_path()), "{a:?}");
            }
        }
    }

    #[test]
    fn agents_without_per_project_dirs_are_exactly_unknown_codex_qodercn() {
        let home = Path::new("/h");
        let none: Vec<_> = AgentKind::ALL
            .into_iter()
            .filter(|a| projects_root_in(*a, home).is_none())
            .collect();
        assert_eq!(
            none,
            vec![AgentKind::Unknown, AgentKind::Codex, AgentKind::Qodercn]
        );
    }
}
