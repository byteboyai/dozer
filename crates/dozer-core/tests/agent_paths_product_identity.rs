//! S6a 复审 R3(方案 A):journal 三家(opencode/goose/aider)的根目录随
//! 产品身份推导。本文件先安装 Digger 身份再断言路径——install 是进程级,
//! 所以整个文件独立成集成测试,与 install_* 系列同做法。

use dozer_core::agent_paths::{
    aider_project_dir_in, goose_project_dir_in, opencode_project_dir_in, project_session_dir_in,
    projects_root_in,
};
use dozer_core::paths::{ProductDirs, ProductId};
use std::path::Path;

fn digger(root: &Path) -> ProductId {
    ProductId::custom(
        "digger",
        "DIGGER_SESSION_ID",
        "digger-hook",
        "diggerd.sock",
        "digger.db",
        ProductDirs::Fixed {
            config: root.join("config"),
            state: root.join("state"),
        },
    )
}

#[test]
fn journal_roots_follow_the_installed_product_identity() {
    let root = tempfile::tempdir().unwrap();
    dozer_core::paths::install(digger(root.path())).expect("安装 Digger 身份");

    let home = Path::new("/home/u");
    let cwd = Path::new("/a/b");
    // 三家都落在 ~/.digger/agents/<agent> 下
    assert_eq!(
        opencode_project_dir_in(home, cwd),
        home.join(".digger/agents/opencode/projects/-a-b")
    );
    assert_eq!(
        goose_project_dir_in(home, cwd),
        home.join(".digger/agents/goose/projects/-a-b")
    );
    assert_eq!(
        aider_project_dir_in(home, cwd),
        home.join(".digger/agents/aider/projects/-a-b")
    );
    for (agent, name) in [
        (dozer_core::protocol::AgentKind::Opencode, "opencode"),
        (dozer_core::protocol::AgentKind::Goose, "goose"),
        (dozer_core::protocol::AgentKind::Aider, "aider"),
    ] {
        assert_eq!(
            projects_root_in(agent, home),
            Some(home.join(format!(".digger/agents/{name}/projects"))),
            "{agent:?}"
        );
        assert_eq!(
            project_session_dir_in(agent, home, cwd),
            Some(home.join(format!(".digger/agents/{name}/projects/-a-b"))),
            "{agent:?}"
        );
    }

    // 与 Dozer 的任何路径互不包含
    let dozer_paths = [
        home.join(".dozer/agents/opencode/projects/-a-b"),
        home.join(".dozer/agents/goose/projects/-a-b"),
        home.join(".dozer/agents/aider/projects/-a-b"),
    ];
    for d in dozer_paths {
        for p in [
            opencode_project_dir_in(home, cwd),
            goose_project_dir_in(home, cwd),
            aider_project_dir_in(home, cwd),
        ] {
            assert!(!p.starts_with(&d) && !d.starts_with(&p), "{p:?} vs {d:?}");
        }
    }

    // 范围界定:非 journal 家(Claude/CodeBuddy/v8agent)的目录不随产品变
    assert_eq!(
        dozer_core::agent_paths::claude_project_dir_in(home, cwd),
        home.join(".claude/projects/-a-b")
    );
    assert_eq!(
        dozer_core::agent_paths::codebuddy_project_dir_in(home, cwd),
        home.join(".codebuddy/projects/a-b")
    );
    assert_eq!(
        dozer_core::agent_paths::v8agent_project_dir_in(home, cwd),
        home.join(".v8agent/projects/-a-b")
    );
}
