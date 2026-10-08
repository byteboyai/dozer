//! 规格 §2.3 的验收场景 S1–S8。`characterizes_` 开头的测试钉住的是**缺口现状**,不是期望行为。

use super::fake_dozerd::{Behavior, FakeDozerd};
use super::fake_panel_host::FakePanelHost;
use super::host::{Duty, ProbeHost};
use super::{NOW_MS, payload, summary};
use crate::conversation::ConversationMeta;
use crate::extensions::usage::{self, ConversationUsage, Message};
use crate::panel_host::testing::{TIMEOUT, runtime};
use dozer_core::protocol::AgentKind;
use iced_widget::core::{Border, Length};
use std::collections::BTreeMap;

const PROJECT: i64 = 7;

fn two_agent_rows() -> Behavior {
    Behavior::Rows(vec![
        (summary("c1", AgentKind::Claude, NOW_MS), payload(4)),
        (summary("c2", AgentKind::Codex, NOW_MS), payload(2)),
    ])
}

fn row(agent: AgentKind, turns: u32) -> (ConversationMeta, ConversationUsage) {
    (
        ConversationMeta::from_summary(&summary("x", agent, NOW_MS)),
        ConversationUsage::from(&payload(turns)),
    )
}

fn repo_with_one_commit() -> bytegit::testutil::TempRepo {
    let repo = bytegit::testutil::TempRepo::new();
    repo.commit_file("a.txt", "x", "c1");
    repo
}

/// S1:切入 → 假服务返回 2 个会话 → `Loaded` → 恰好推出一条 payload。
#[test]
fn s1_activate_loads_rows_and_pushes_one_payload() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.webview_ready();

    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    assert!(host.ws.loading(), "切入立即置“统计中”");
    host.tick();
    assert_eq!(host.surface.count(), 0, "加载中不推送");

    host.deliver(rx.recv_timeout(TIMEOUT).expect("Loaded 应在超时内到达"));
    assert!(!host.ws.loading());
    assert_eq!(host.ws.rows().len(), 2);
    assert_eq!(host.ws.git_commits(), 1, "TempRepo 恰有 1 个提交");

    host.tick();
    host.tick();
    assert_eq!(
        host.surface.kinds(),
        vec!["all_agents".to_string()],
        "未变化不重发"
    );
    assert_eq!(
        dozerd.cwds(),
        vec![repo.path().to_string_lossy().into_owned()]
    );

    // 视图也能在假 host 上构造(规格 §1:面板只靠 PanelHost 契约就能画)。
    let fake = FakePanelHost::default();
    let _c = usage::content_pane(&fake, &host.ws, Length::Fill, Border::default());
    let _l = usage::list_pane(&host.ws, Length::Fill, Border::default());
    assert_eq!(fake.collapse_buttons_asked(), 1);

    // 这个场景里 host 替面板做的事,就是缺口报告“host 能力清单”的底稿。
    let duties = host.duty_set();
    for d in [
        Duty::BuildActivationCtx,
        Duty::RouteLoaded,
        Duty::FramePush,
        Duty::SkipWhileLoading,
        Duty::ParseIpcAndMarkReady,
    ] {
        assert!(duties.contains(&d), "S1 应触发 host 职责 {d:?}");
        assert!(!d.host_site().is_empty(), "{d:?} 必须能指回真实 host 位置");
    }
}

/// S2:切换 agent 筛选 → payload 变化,且只在变化时推一次。
#[test]
fn s2_agent_filter_changes_payload_once() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.webview_ready();
    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
    host.tick();

    host.deliver(Message::AgentFilterSet(Some(AgentKind::Claude)));
    host.tick();
    host.tick();
    assert_eq!(
        host.surface.kinds(),
        vec!["all_agents".to_string(), "single_agent".to_string()]
    );
}

/// S3:项目没有路径 → 置“统计中”但不发请求(迁移前行为)。
#[test]
fn s3_no_project_path_marks_loading_but_never_calls_the_service() {
    let rt = runtime();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    let rx = host.activate(&rt, &dozerd, None);
    assert!(host.ws.loading());
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err()
    );
    assert!(dozerd.cwds().is_empty());
}

/// S4:服务不可达/报错 → 面板不 panic,结果与“没有数据”无法区分。
#[test]
fn s4_characterizes_g4_service_failure_looks_like_no_data() {
    for behavior in [Behavior::HangUp, Behavior::ErrorReply("boom".into())] {
        let rt = runtime();
        let repo = repo_with_one_commit();
        let dozerd = FakeDozerd::start(&rt, behavior);
        let mut host = ProbeHost::new(PROJECT);
        host.surface.attach();
        host.webview_ready();
        let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
        host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
        assert!(!host.ws.loading(), "失败后仍要退出“统计中”");
        assert!(host.ws.rows().is_empty());
        assert_eq!(
            host.ws.git_commits(),
            1,
            "git 这条独立数据源不受 dozerd 失败影响"
        );
        host.tick();
        assert_eq!(
            host.surface.kinds(),
            vec!["empty".to_string()],
            "缺口 G4:服务失败被 unwrap_or_default 吞掉,用户看到的是“无数据”而非错误;修复后应翻转本测试"
        );
    }
}

/// S5:webview 销毁重建 → 同一内容重发。
#[test]
fn s5_webview_rebuild_resends_the_same_content() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.webview_ready();
    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
    host.tick();
    assert_eq!(host.surface.count(), 1);

    host.surface.detach();
    host.tick();
    host.surface.attach();
    host.tick();
    assert_eq!(host.surface.count(), 1, "新实例 ready 之前不推送");

    host.webview_ready();
    host.tick();
    assert_eq!(
        host.surface.kinds(),
        vec!["all_agents".to_string(), "all_agents".to_string()],
        "Ready 清空 last_sent,强制重发当前内容"
    );
}

/// S6:较早的结果后到 → 面板无法识别过期;跨项目结果面板自己也不区分。
#[test]
fn s6_characterizes_g3_panel_cannot_tell_stale_results() {
    let mut host = ProbeHost::new(PROJECT);
    let newer = Message::Loaded(
        PROJECT,
        vec![row(AgentKind::Claude, 1), row(AgentKind::Codex, 2)],
        0,
        BTreeMap::new(),
    );
    let older = Message::Loaded(
        PROJECT,
        vec![row(AgentKind::Claude, 1)],
        0,
        BTreeMap::new(),
    );
    host.deliver(newer);
    host.deliver(older);
    assert_eq!(
        host.ws.rows().len(),
        1,
        "缺口 G3:后到的旧结果覆盖了新结果——Loaded 没有序号/revision,面板自己无从判断;修复后应翻转本测试"
    );

    // 跨项目:host 的 `with_project` 负责丢弃;面板 `update` 本身不看 project_id。
    host.deliver(Message::Loaded(
        99,
        vec![row(AgentKind::Claude, 1)],
        0,
        BTreeMap::new(),
    ));
    assert!(host.duty_set().contains(&Duty::DropOtherProject));
    let mut bare = usage::WorkspaceState::default();
    usage::update(
        &mut bare,
        Message::Loaded(99, vec![row(AgentKind::Claude, 1)], 0, BTreeMap::new()),
    );
    assert_eq!(
        bare.rows().len(),
        1,
        "缺口 G3:没有 host 路由时,面板会收下任何项目的结果"
    );
}

/// S7:`Message` 里有只能由 host 处理的变体,误投就 panic。
#[test]
#[should_panic(expected = "由内核拦截处理")]
fn s7_characterizes_g5_toggle_list_collapse_must_never_reach_the_panel() {
    let mut ws = usage::WorkspaceState::default();
    usage::update(&mut ws, Message::ToggleListCollapse);
}

#[test]
#[should_panic(expected = "由内核拦截处理")]
fn s7_characterizes_g5_hover_must_never_reach_the_panel() {
    use crate::app::{HoverId, PanelKind};
    let mut ws = usage::WorkspaceState::default();
    usage::update(
        &mut ws,
        Message::Hover(HoverId::list_collapse(PanelKind::Usage), true),
    );
}

/// S7 的另一面:夹具里 host 拦截后,这两条消息不会进面板。
#[test]
fn s7_host_intercepts_collapse_and_hover() {
    use crate::app::{HoverId, PanelKind};
    let mut host = ProbeHost::new(PROJECT);
    host.deliver(Message::ToggleListCollapse);
    host.deliver(Message::Hover(
        HoverId::list_collapse(PanelKind::Usage),
        true,
    ));
    let duties = host.duty_set();
    assert!(duties.contains(&Duty::InterceptCollapse));
    assert!(duties.contains(&Duty::InterceptHover));
}

/// S8:夹具(不含本文件)不引用 `App`/`Workspace`,也不引用 `usage` 以外的任何面板。
#[test]
fn s8_probe_fixture_names_no_other_panel_and_no_app_or_workspace() {
    let files = [
        ("fake_dozerd.rs", include_str!("fake_dozerd.rs")),
        ("surface.rs", include_str!("surface.rs")),
        ("fake_panel_host.rs", include_str!("fake_panel_host.rs")),
        ("host.rs", include_str!("host.rs")),
    ];
    for (name, src) in files {
        for line in src.lines().filter(|l| l.trim_start().starts_with("use ")) {
            for panel in line.split("extensions::").skip(1) {
                let first: String = panel
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                assert_eq!(
                    first, "usage",
                    "{name}: 引用了别的面板 `{first}`(违反 R2): {line}"
                );
            }
            for token in line.split(|c: char| !c.is_alphanumeric() && c != '_') {
                assert!(
                    token != "App" && token != "Workspace",
                    "{name}: 引用了 host 内核类型 `{token}`(违反 R1): {line}"
                );
            }
        }
    }
}
