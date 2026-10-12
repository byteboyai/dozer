//! 规格 §2.3 的验收场景 S1–S8。`characterizes_` 开头的测试钉住的是**缺口现状**,不是期望行为。

use super::fake_dozerd::{Behavior, FakeDozerd};
use super::fake_panel_host::FakePanelHost;
use super::fake_usage::FakeUsage;
use super::host::{Duty, ProbeHost};
use super::{NOW_MS, payload, summary};
use crate::extensions::usage::{self, ConversationUsage, Message};
use crate::panel_host::testing::{TIMEOUT, runtime};
use dozer_core::protocol::AgentKind;
use dozer_model::ConversationMeta;
use dozer_services::ServiceErrorKind;
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
    // 原生实例进池先于一帧 tick(宿主每帧 reconcile → 建 webview → 之后才收 ready)。
    host.tick();
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
    host.tick();
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

/// S4a:进程内服务失败 → 面板退出“统计中”、git 数据照常、恰有一条 Error Toast,而不是静默的“无数据”(G4 已修)。
#[test]
fn s4a_service_failure_is_reported_not_swallowed() {
    for kind in [ServiceErrorKind::Unavailable, ServiceErrorKind::Rejected] {
        let rt = runtime();
        let repo = repo_with_one_commit();
        let fake = FakeUsage::failing(kind, "boom");
        let mut host = ProbeHost::new(PROJECT);
        host.surface.attach();
        host.tick();
        host.webview_ready();
        let rx = host.activate_with(
            &rt,
            fake.clone().into_services(),
            Some(repo.path().to_path_buf()),
        );
        host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
        assert!(!host.ws.loading());
        assert!(host.ws.rows().is_empty());
        assert_eq!(host.ws.git_commits(), 1);
        let toasts = host.drain_toasts();
        assert_eq!(toasts.len(), 1, "{kind:?}");
        assert!(toasts[0].text.contains("boom"));
        assert_eq!(fake.calls().len(), 1);
    }
}

/// S4b:同一件事端到端——真实 `Client` 对 `FakeDozerd` 的 HangUp / ErrorReply 也走到 Toast。
#[test]
fn s4b_real_client_failures_surface_as_toasts() {
    for behavior in [Behavior::HangUp, Behavior::ErrorReply("boom".into())] {
        let rt = runtime();
        let repo = repo_with_one_commit();
        let dozerd = FakeDozerd::start(&rt, behavior);
        let mut host = ProbeHost::new(PROJECT);
        let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
        host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
        assert!(!host.ws.loading());
        assert_eq!(host.drain_toasts().len(), 1);
    }
}

/// S5b(R1):同一帧"销毁+重建"(槽号不变、原生实例已换,如项目切换整池 clear
/// 后同帧重建)→ 必须开新代数:旧实例的 ready/已送达状态不得继承,新实例须重发。
#[test]
fn s5b_same_frame_replacement_opens_new_generation_and_resends() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.tick();
    host.webview_ready();
    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
    host.tick();
    assert_eq!(host.surface.count(), 1, "旧实例已推一次");
    let old = host.push.id();

    // 同帧销毁并重建:attach 保持 true,replace 一次。
    host.surface.replace();
    host.tick();
    assert_ne!(host.push.id(), old, "原生实例替换必须换代数");
    assert_eq!(
        host.surface.count(),
        1,
        "新实例握手前不推(旧实例状态不继承)"
    );

    host.webview_ready();
    host.tick();
    assert_eq!(
        host.surface.kinds(),
        vec!["all_agents".to_string(), "all_agents".to_string()],
        "新实例 Ready 后必须重发快照"
    );
}

/// S5:webview 销毁重建 → 同一内容重发。
#[test]
fn s5_webview_rebuild_resends_the_same_content() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.tick();
    host.webview_ready();
    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
    host.tick();
    assert_eq!(host.surface.count(), 1);

    // 离池 → Destroyed 并递增代数;重新进池 → 新实例,Ready 前不推。
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
        "新实例 Ready 握手后强制重发当前内容"
    );
}

/// S6a(G3b 已修复,2026-10-08):较早发起的刷新后到 → 面板自己按代数丢弃,不再覆盖新结果。
#[test]
fn s6_stale_refresh_is_dropped_by_the_panel_itself() {
    let mut host = ProbeHost::new(PROJECT);
    let first = host.ws.begin_refresh();
    let second = host.ws.begin_refresh();
    let newer = Message::Loaded(
        PROJECT,
        second,
        vec![row(AgentKind::Claude, 1), row(AgentKind::Codex, 2)],
        0,
        BTreeMap::new(),
    );
    let older = Message::Loaded(
        PROJECT,
        first,
        vec![row(AgentKind::Claude, 1)],
        0,
        BTreeMap::new(),
    );
    host.deliver(newer);
    host.deliver(older);
    assert_eq!(host.ws.rows().len(), 2, "较早发起的刷新后到必须被面板丢弃");
}

/// S6b:跨项目路由仍由 host 负责;面板 `update` 本身不看 project_id(缺口 G3a)。
#[test]
fn s6_characterizes_g3a_panel_does_not_check_the_project_id() {
    let mut host = ProbeHost::new(PROJECT);
    host.deliver(Message::Loaded(
        99,
        0,
        vec![row(AgentKind::Claude, 1)],
        0,
        BTreeMap::new(),
    ));
    assert!(host.duty_set().contains(&Duty::DropOtherProject));
    let mut bare = usage::WorkspaceState::default();
    usage::update(
        &mut bare,
        Message::Loaded(99, 0, vec![row(AgentKind::Claude, 1)], 0, BTreeMap::new()),
    );
    assert_eq!(
        bare.rows().len(),
        1,
        "缺口 G3a:没有 host 路由时,面板会收下任何项目的结果;修复后应翻转本测试"
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
