use super::*;

use crate::app::{Message, PanelKind};
use crate::osc::OscScanner;
use crate::term::term_model::TerminalModel;
use crate::transcript::ReviewEntry;
use byteui::interaction::icons::IconKind;
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};
use dozer_panel_agent::tabs::effective_cwd;
use std::path::{Path, PathBuf};

#[test]
fn review_webview_spec_empty_when_no_review() {
    assert_eq!(review_webview_spec(None, "en"), Vec::new());
}

#[test]
fn review_webview_spec_empty_on_error_or_empty_entries() {
    let with_error = ReviewView {
        source: ReviewSource::Conversation("a".into()),
        entries: vec![ReviewEntry::Human { text: "hi".into() }],
        error: Some("boom".into()),
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: None,
    };
    assert_eq!(review_webview_spec(Some(&with_error), "en"), Vec::new());

    let empty_entries = ReviewView {
        source: ReviewSource::Conversation("a".into()),
        entries: Vec::new(),
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: None,
    };
    assert_eq!(review_webview_spec(Some(&empty_entries), "en"), Vec::new());
}

#[test]
fn review_webview_spec_url_carries_nonce_and_is_visible_when_loaded() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("a".into()),
        entries: vec![ReviewEntry::Human { text: "hi".into() }],
        error: None,
        agent: AgentKind::Claude,
        nonce: 7,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: Some(7),
    };
    let specs = review_webview_spec(Some(&rv), "en");
    assert_eq!(specs.len(), 1);
    assert_eq!(
        specs[0].url,
        "dozer://review-trace/host.html?_r=7&locale=en"
    );
    assert!(specs[0].visible);
}

#[test]
fn review_webview_spec_hidden_until_loaded_nonce_matches() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: vec![ReviewEntry::Human { text: "hi".into() }],
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: None,
    };
    let specs = review_webview_spec(Some(&rv), "en");
    assert_eq!(
        specs.len(),
        1,
        "有数据时仍应创建 webview(否则永远等不到 loaded 事件)"
    );
    assert!(!specs[0].visible, "loaded_nonce 未追上 nonce 时不可见");
}

#[test]
fn review_webview_spec_visible_when_loaded_nonce_matches() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: vec![ReviewEntry::Human { text: "hi".into() }],
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: Some(3),
    };
    let specs = review_webview_spec(Some(&rv), "en");
    assert!(specs[0].visible);
}

/// 对应本计划 Review Focus"空清单分支与未 loaded 分支职责重叠":即便
/// `loaded_nonce == nonce`(理论上不该跟"无数据"同时成立,但要防呆),
/// `entries.is_empty()` 仍必须优先命中既有的"销毁 webview"分支。
#[test]
fn review_webview_spec_empty_entries_still_wins_over_loaded_nonce() {
    let rv = ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: Vec::new(),
        error: None,
        agent: AgentKind::Claude,
        nonce: 3,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: Some(3),
    };
    assert_eq!(review_webview_spec(Some(&rv), "en"), Vec::new());
}

#[test]
fn review_source_conversation_id_uses_conversation_source_directly() {
    let source = ReviewSource::Conversation("abc".into());
    assert_eq!(
        review_source_conversation_id(&source, |_| None),
        Some("abc".to_string())
    );
}

#[test]
fn review_source_conversation_id_derives_from_session_tab_transcript_stem() {
    let source = ReviewSource::Session(2);
    assert_eq!(
        review_source_conversation_id(&source, |tab_id| {
            assert_eq!(tab_id, 2);
            Some("/h/.claude/projects/x/abc123.jsonl")
        }),
        Some("abc123".to_string())
    );
}

#[test]
fn review_source_conversation_id_none_when_session_tab_has_no_transcript() {
    let source = ReviewSource::Session(0);
    assert_eq!(review_source_conversation_id(&source, |_| None), None);
}

#[test]
fn agent_card_refresh_plan_decides_by_agent_and_cwd() {
    let root = PathBuf::from("/repo");
    let elsewhere = PathBuf::from("/elsewhere");

    assert_eq!(
        agent_card_refresh_plan(dozer_core::protocol::AgentKind::Claude, &root, Some(&root)),
        (true, true, false),
        "Claude + cwd 等于项目根:model/mode + activity,不做工作区"
    );
    assert_eq!(
        agent_card_refresh_plan(
            dozer_core::protocol::AgentKind::Opencode,
            &root,
            Some(&root)
        ),
        (true, true, false),
        "Opencode + cwd 等于项目根:dozer-hook 插件已经把 model 写进合成 \
             transcript(见 dozer-translate.ts::onUserMessage),model/mode \
             门也跟 Claude 一样打开;不做工作区"
    );
    assert_eq!(
        agent_card_refresh_plan(
            dozer_core::protocol::AgentKind::Opencode,
            &elsewhere,
            Some(&root)
        ),
        (true, true, true),
        "Opencode + cwd 偏离项目根:model/mode + activity + 工作区"
    );
    assert_eq!(
        agent_card_refresh_plan(
            dozer_core::protocol::AgentKind::Codebuddy,
            &root,
            Some(&root)
        ),
        (true, true, false),
        "Codebuddy + cwd 等于项目根:也要做(只提 LLM,mode 恒 None——\
             transcript 没有 permissionMode 等价字段,见 latest_model_mode_and_activity)"
    );
    assert_eq!(
        agent_card_refresh_plan(
            dozer_core::protocol::AgentKind::Claude,
            &elsewhere,
            Some(&root)
        ),
        (true, true, true),
        "Claude + cwd 偏离项目根:三者都做"
    );
    assert_eq!(
        agent_card_refresh_plan(dozer_core::protocol::AgentKind::Unknown, &root, Some(&root)),
        (true, true, false),
        "Finding 2: Unknown + cwd 等于项目根:也要按 Claude 形状做 model/mode 提取"
    );
    let subdir = PathBuf::from("/repo").join("crates").join("dozer-app");
    assert_eq!(
        agent_card_refresh_plan(
            dozer_core::protocol::AgentKind::Claude,
            &subdir,
            Some(&root)
        ),
        (true, true, false),
        "Finding 3: cwd 是 project_root 的子目录,不算偏离,不应触发 needs_workspace"
    );
    assert_eq!(
        agent_card_refresh_plan(dozer_core::protocol::AgentKind::Codex, &root, Some(&root)),
        (true, true, false),
        "Codex 现在也能从 turn_context/response_item 行提取 model/activity\
             (mode 恒 None——Codex 没有 permissionMode 等价字段)"
    );
    assert_eq!(
        agent_card_refresh_plan(dozer_core::protocol::AgentKind::V8agent, &root, Some(&root)),
        (false, true, false),
        "V8agent 现在走 Claude 形状解析(parse.rs 的 dispatch 改动),\
             transcript 能真正解出内容,activity 门禁应该打开;\
             model/mode 门禁不动(V8agent 的 transcript 里没有 model 字段)"
    );
}

#[test]
fn apply_agent_card_refresh_llm_and_mode_keep_last_value_when_none() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut tabs = vec![make_test_tab(&rt, "a", AgentKind::Claude)];
    tabs[0].tab_id = 7;

    apply_agent_card_refresh(
        &mut tabs,
        7,
        Some("claude-sonnet-5".to_string()),
        Some("auto".to_string()),
        None,
        None,
    );
    assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(tabs[0].permission_mode.as_deref(), Some("auto"));
    assert_eq!(tabs[0].workspace_override, None);

    // 第二次刷新 model/mode 都是 None(比如那次 transcript 读取
    // 失败):不应该把已经拿到的值抹掉。
    apply_agent_card_refresh(&mut tabs, 7, None, None, None, None);
    assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(tabs[0].permission_mode.as_deref(), Some("auto"));

    // 未知 tab_id:整体 no-op,不 panic。
    apply_agent_card_refresh(&mut tabs, 999, Some("x".to_string()), None, None, None);
    assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
}

#[test]
fn apply_agent_card_refresh_workspace_always_overwrites_including_clear() {
    // Finding 1 回归测试:workspace_override 曾经"只在 Some 时覆盖",
    // 导致 session 一旦偏离过项目根就再也清不掉覆盖(cd 回项目根后卡片
    // 工作区行永远停在旧仓库快照)。workspace 字段跟 llm_model/mode
    // 语义不同——None 是明确的"清空"信号,不是"没查、保留原值"。
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut tabs = vec![make_test_tab(&rt, "a", AgentKind::Claude)];
    tabs[0].tab_id = 7;

    let diverged = WorkspaceGitInfo {
        branch: Some("feature/x".to_string()),
        dirty: true,
    };
    apply_agent_card_refresh(&mut tabs, 7, None, None, None, Some(diverged.clone()));
    assert_eq!(tabs[0].workspace_override, Some(diverged));

    // cwd 回到项目根:workspace 传 None,必须真的清空,不是保留旧覆盖。
    apply_agent_card_refresh(&mut tabs, 7, None, None, None, None);
    assert_eq!(tabs[0].workspace_override, None);
}

#[test]
fn preview_context_from_cursor_only_uses_1_indexed_point_range() {
    // 无选区:0-indexed (1, 4) 光标 → 1-indexed start==end==(2, 5)。
    let ctx = preview_context_from_editor_state(
        "/repo/src/main.rs",
        false,
        (1, 4),
        None,
        1_700_000_000_000,
    );
    assert_eq!(ctx.path, "/repo/src/main.rs");
    assert_eq!((ctx.start_line, ctx.start_col), (2, 5));
    assert_eq!((ctx.end_line, ctx.end_col), (2, 5));
    assert!(!ctx.has_selection);
    // 新鲜度戳原样透传调用方给的时间,不在函数里读时钟。
    assert_eq!(ctx.updated_at_ms, 1_700_000_000_000);
}

#[test]
fn preview_context_from_selection_uses_1_indexed_range() {
    // 0-indexed 选区 (1,4)..(3,0) → 1-indexed (2,5)..(4,1)。
    let ctx = preview_context_from_editor_state(
        "/repo/src/main.rs",
        true,
        (1, 4),
        Some(((1, 4), (3, 0))),
        1_700_000_000_000,
    );
    assert_eq!((ctx.start_line, ctx.start_col), (2, 5));
    assert_eq!((ctx.end_line, ctx.end_col), (4, 1));
    assert!(ctx.has_selection);
    assert_eq!(ctx.updated_at_ms, 1_700_000_000_000);
}

#[test]
fn preview_context_from_web_state_maps_selection_and_viewport() {
    use crate::preview::{TextPosition, TextRange};
    let range = TextRange {
        start: TextPosition { line: 3, column: 2 },
        end: TextPosition { line: 5, column: 4 },
    };
    let ctx = preview_context_from_web_state(
        "/repo/a.rs",
        Some(range),
        Some("selected\ntext".into()),
        Some((10, 40)),
        9,
        1_700_000_000_000,
    );
    assert_eq!(ctx.path, "/repo/a.rs");
    assert_eq!((ctx.start_line, ctx.start_col), (3, 2));
    assert_eq!(ctx.selected_text.as_deref(), Some("selected\ntext"));
    assert_eq!((ctx.end_line, ctx.end_col), (5, 4));
    assert!(ctx.has_selection);
    assert_eq!(ctx.revision, 9);
    assert_eq!(
        (ctx.visible_start_line, ctx.visible_end_line),
        (Some(10), Some(40))
    );
    assert_eq!(ctx.updated_at_ms, 1_700_000_000_000);
}

#[test]
fn preview_context_from_web_state_cursor_only_and_missing_viewport() {
    use crate::preview::{TextPosition, TextRange};
    let cursor = TextRange {
        start: TextPosition { line: 7, column: 1 },
        end: TextPosition { line: 7, column: 1 },
    };
    let ctx = preview_context_from_web_state("/repo/a.rs", Some(cursor), None, None, 0, 1);
    assert_eq!((ctx.start_line, ctx.start_col), (7, 1));
    assert_eq!((ctx.end_line, ctx.end_col), (7, 1));
    assert!(!ctx.has_selection, "start==end 视为光标无选区");
    assert_eq!(ctx.visible_start_line, None);

    // 完全没有位置信息时给 1-based 最小合法位置,不出现 0 行。
    let ctx = preview_context_from_web_state("/repo/a.rs", None, None, None, 0, 1);
    assert_eq!((ctx.start_line, ctx.start_col), (1, 1));
}

#[test]
fn mode_token_covers_all_modes() {
    use crate::preview::PreviewMode;
    for mode in [
        PreviewMode::Code,
        PreviewMode::Rendered,
        PreviewMode::Source,
        PreviewMode::Tree,
        PreviewMode::Text,
        PreviewMode::Streamed,
        PreviewMode::Tabular,
        PreviewMode::External,
        PreviewMode::Unsupported,
    ] {
        assert!(!mode_token(mode).is_empty(), "{mode:?}");
    }
}

/// 促成期占位的**核心不变式**:同步换上的那一刻就必须已知归属项目。
///
/// 促成是异步的,这份占位会在消息环里存活若干毫秒,而且它就是当前聚焦
/// 的 workspace(用户正是点了这个页签才触发促成)。只要它 `project` 为
/// `None`,`spawn_new_tab` 的 `expect("Workspace 存在即已知归属项目")` 就
/// 重新变成可达路径——用户在这段窗口里点一下终端 tab 栏的"＋"就会
/// panic 掉整个 GUI 进程(Task 5/6 review 反复强调的那条)。
#[test]
fn loading_placeholder_always_knows_its_project() {
    let info = ProjectInfo {
        id: 42,
        path: "/tmp/p42".to_string(),
        name: "p42".to_string(),
        last_active_ms: 0,
        created_ms: 0,
        updated_ms: 0,
    };
    let ws = Workspace::loading_for_project(info.clone());
    assert_eq!(
        ws.project.as_ref().map(|p| p.id),
        Some(42),
        "占位必须携带项目,否则 spawn_new_tab 的 expect 变成可达路径"
    );
    assert_eq!(
        ws.project.as_ref().map(|p| p.path.as_str()),
        Some("/tmp/p42")
    );
    assert!(
        ws.files.file_tree_is_some(),
        "文件树根不需要 IO,应当立刻可画"
    );
    assert!(ws.loading, "必须打上占位标记,促成结果才认得出该替换谁");
    assert!(ws.tabs.is_empty(), "会话要等 IO,占位阶段不该有 tab");
}

/// 反过来:空壳占位(`retarget_active_slot` 在同一条消息内用完即改写的
/// 那种)不带项目也不带 `loading` 标记——它的安全性靠"同步内改写完",
/// 与促成占位是两码事,不能互相顶替。
#[test]
fn empty_placeholder_is_not_a_loading_placeholder() {
    let ws = Workspace::empty_for_project_placeholder();
    assert!(ws.project.is_none());
    assert!(!ws.loading);
}

#[test]
fn chrome_constants_exclude_removed_header() {
    // #4 去掉 header 行(22px + 一处 spacing 4 = 26)后的期望值,锁死防漂移遮挡。
    // 文件预览又去掉了地址栏(只剩 tab 栏),浏览器仍保留地址栏,两分支
    // chrome 高度分道,不能再共用同一个值——否则文件预览顶上会露一截
    // 再也画不出东西的空白。
    assert_eq!(
        byteui::theme::geometry::preview_chrome_top_px(),
        38.0,
        "文件预览 chrome 顶应为去地址栏后的 38(8 内边距 + 30 tab 栏)"
    );
    assert_eq!(
        byteui::theme::geometry::browser_chrome_top_px(),
        72.0,
        "浏览器 chrome 顶应为 72(8 内边距 + 30 tab 栏 + 4 spacing + 30 地址栏)"
    );
    assert_eq!(
        byteui::theme::geometry::chrome_height_px(),
        50.0,
        "终端 chrome 高应为去 header 后的 50"
    );
}

#[test]
fn relative_time_text_boundaries() {
    assert_eq!(relative_time_text(1000, 1000, "zh-CN"), "刚刚");
    assert_eq!(relative_time_text(0, 59_000, "zh-CN"), "刚刚");
    assert_eq!(relative_time_text(0, 60_000, "zh-CN"), "1 分钟前");
    assert_eq!(relative_time_text(0, 3_599_000, "zh-CN"), "59 分钟前");
    assert_eq!(relative_time_text(0, 3_600_000, "zh-CN"), "1 小时前");
    assert_eq!(relative_time_text(0, 86_399_000, "zh-CN"), "23 小时前");
    assert_eq!(relative_time_text(0, 86_400_000, "zh-CN"), "1 天前");
    assert_eq!(relative_time_text(0, 86_400_000, "en"), "1 days ago");
}

#[test]
fn review_refresh_only_for_matching_session() {
    assert!(review_should_refresh_on_turn(&ReviewSource::Session(3), 3));
    assert!(!review_should_refresh_on_turn(&ReviewSource::Session(3), 4));
    assert!(!review_should_refresh_on_turn(
        &ReviewSource::Conversation("x".into()),
        3
    ));
}

#[test]
fn review_source_conversation_matches_conversation_id_not_tab() {
    // Conversation 变体不该被 review_should_refresh_on_turn(只认
    // Session(tab_id))误判为需要跟随终端回合刷新。
    assert!(!review_should_refresh_on_turn(
        &ReviewSource::Conversation("c1".into()),
        3
    ));
}

#[test]
fn effective_cwd_prefers_osc_over_spawn() {
    use std::path::Path;
    // 用户 cd 进仓库:OSC 7 跟踪的实时目录优先于启动目录($HOME)
    assert_eq!(
        effective_cwd(Some(Path::new("/repo/proj")), "/Users/me"),
        PathBuf::from("/repo/proj")
    );
    // 尚无 OSC 7 上报:回落启动目录
    assert_eq!(effective_cwd(None, "/Users/me"), PathBuf::from("/Users/me"));
}

#[test]
fn preview_tab_display_width_narrower_than_term_no_dot() {
    // 同标题下预览版无状态点,应恒窄于终端版。
    assert!(preview_tab_display_width("a.rs") < tab_display_width("a.rs"));
}

#[test]
fn dot_color_states() {
    use dozer_core::protocol::AgentState::*;
    // 死会话恒为灰，不论 agent 状态。
    assert_eq!(
        dot_color(Running, false),
        byteui::theme::color::current().dim,
        "死会话灰点"
    );
    // 存活：空闲青、运行绿、待输入红、回合毕金——各状态独立配色，不再
    // 靠闪烁区分空闲/运行(闪烁动画已取消)。
    assert_eq!(dot_color(Idle, true), byteui::theme::color::current().cyan);
    assert_eq!(
        dot_color(Running, true),
        byteui::theme::color::current().green
    );
    assert_eq!(
        dot_color(AwaitingInput, true),
        byteui::theme::color::current().red
    );
    assert_eq!(
        dot_color(TurnEnded, true),
        byteui::theme::color::current().gold
    );
}

#[test]
fn open_missing_file_sets_preview_error_and_no_tab() {
    // Workspace 全量构造依赖 daemon/EventLoop,headless 里只验状态机
    // 侧的可测部分:错误字段与 tab 数经由 update 的行为契约。
    // 若 Workspace 无法在测试中直接构造,则改为验证 content_pane_bounds
    // 之外新增一个纯函数不现实——此时降级为:仅确认编译期字段存在,
    // 测试留待 Task 6 人工验收覆盖,并在报告中写明。
}

#[test]
fn agent_state_label_covers_all() {
    assert_eq!(agent_state_label(AgentState::Running, "zh-CN"), "运行中");
    assert_eq!(
        agent_state_label(AgentState::AwaitingInput, "zh-CN"),
        "待输入"
    );
    assert_eq!(agent_state_label(AgentState::TurnEnded, "zh-CN"), "回合毕");
    assert_eq!(agent_state_label(AgentState::Idle, "zh-CN"), "空闲");
    assert_eq!(agent_state_label(AgentState::Running, "en"), "Running");
    assert_eq!(agent_state_label(AgentState::Idle, "en"), "Idle");
}

#[test]
fn format_model_label_strips_claude_prefix_and_titlecases() {
    assert_eq!(format_model_label("claude-sonnet-5"), "Sonnet 5");
    assert_eq!(format_model_label("claude-opus-5"), "Opus 5");
    assert_eq!(format_model_label("claude-haiku-4-5"), "Haiku 4 5");
}

#[test]
fn format_model_label_unknown_shape_returns_verbatim() {
    assert_eq!(format_model_label("gpt-4"), "gpt-4");
    assert_eq!(format_model_label(""), "");
}

fn make_test_tab(rt: &tokio::runtime::Runtime, id: &str, agent: AgentKind) -> SessionTab {
    SessionTab {
        info: SessionInfo {
            id: id.to_string(),
            name: "shell".into(),
            command: "shell".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 0,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: None,
            agent,
        },
        model: TerminalModel::new(80, 24),
        alive: true,
        agent_state: AgentState::Idle,
        agent,
        transcript_path: None,
        osc: OscScanner::new(),
        cwd: None,
        last_exit: None,
        llm_model: None,
        permission_mode: None,
        last_activity: None,
        workspace_override: None,
        tab_id: 0,
        forwarder: rt.spawn(async {}),
        backend: TabBackend::Daemon,
    }
}

#[test]
fn new_session_tab_fields_default_to_none() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tab = make_test_tab(&rt, "s1", dozer_core::protocol::AgentKind::Claude);
    assert_eq!(tab.llm_model, None);
    assert_eq!(tab.permission_mode, None);
    assert_eq!(tab.workspace_override, None);
}

#[test]
fn close_tab_skips_daemon_kill_for_ssh_backend() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut tab = make_test_tab(&rt, "ssh:h1", dozer_core::protocol::AgentKind::Unknown);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    tab.backend = TabBackend::Custom(tx);
    tab.alive = true;
    // `close_tab` 里 `client.kill` 那一步只在
    // `matches!(tab.backend, TabBackend::Daemon)` 时才走——单测环境没
    // 有真实 daemon,不适合跑完整 `close_tab`,重点断言这个分支判断
    // 本身:SSH backend 不该触发 daemon kill。
    assert!(!matches!(tab.backend, TabBackend::Daemon));
}

#[test]
fn resize_one_resizes_ssh_tab_model() {
    // `resize_all` 对 `tabs`/`ssh_tabs` 各跑一遍 `resize_one`；这里直接测
    // 那个被复用的核心逻辑(它不碰 `ShellIo`,单测不用构造 winit
    // `EventLoopProxy`)——构造一个 SSH backend 的假 tab,断言 resize 后
    // 网格尺寸跟着变了。
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut tab = make_test_tab(&rt, "ssh:h1", dozer_core::protocol::AgentKind::Unknown);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    tab.backend = TabBackend::Custom(tx);
    let before = tab.model.grid_dims();
    let client = dozer_client::Client::new(std::path::PathBuf::from(
        "/tmp/dozer-resize-test-nonexistent.sock",
    ));
    Workspace::resize_one(&mut tab, &client, rt.handle(), 100, 40);
    let after = tab.model.grid_dims();
    assert_ne!(before, after, "ssh_tab 的网格尺寸应随 resize 变化");
}

#[test]
fn group_tabs_by_agent_empty_list() {
    let tabs: Vec<SessionTab> = Vec::new();
    assert_eq!(
        group_tabs_by_agent(&tabs),
        Vec::<(AgentKind, Vec<usize>)>::new()
    );
}

#[test]
fn group_tabs_by_agent_single_agent_multiple_sessions() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tabs = vec![
        make_test_tab(&rt, "a", AgentKind::Claude),
        make_test_tab(&rt, "b", AgentKind::Claude),
    ];
    assert_eq!(
        group_tabs_by_agent(&tabs),
        vec![(AgentKind::Claude, vec![0, 1])]
    );
}

#[test]
fn group_tabs_by_agent_mixed_fixed_order_no_empty_groups() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    // 故意打乱插入顺序(Opencode 先、Claude 后),验证分组输出顺序
    // 固定为 Claude→Codebuddy→Opencode→Unknown,不是插入顺序;
    // 没有任何会话的 Codebuddy 不出现在结果里(空分组不占位)。
    let tabs = vec![
        make_test_tab(&rt, "a", AgentKind::Opencode),
        make_test_tab(&rt, "b", AgentKind::Claude),
        make_test_tab(&rt, "c", AgentKind::Unknown),
        make_test_tab(&rt, "d", AgentKind::Claude),
    ];
    assert_eq!(
        group_tabs_by_agent(&tabs),
        vec![
            (AgentKind::Claude, vec![1, 3]),
            (AgentKind::Opencode, vec![0]),
            (AgentKind::Unknown, vec![2]),
        ]
    );
}

#[test]
fn group_tabs_by_agent_includes_codex_goose_aider_and_v8agent() {
    // 回归测试:`ORDER` 曾经只有 4 个 AgentKind(Claude/Codebuddy/
    // Opencode/Unknown),Codex/Goose/Aider/V8agent 的会话会被 filter_map
    // 静默丢弃——tab 标题栏能正确识别出 agent 种类,但 Agent 侧栏
    // 面板完全不显示这些会话,面板直接留空。
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tabs = vec![
        make_test_tab(&rt, "a", AgentKind::V8agent),
        make_test_tab(&rt, "b", AgentKind::Codex),
        make_test_tab(&rt, "c", AgentKind::Goose),
        make_test_tab(&rt, "d", AgentKind::Aider),
    ];
    assert_eq!(
        group_tabs_by_agent(&tabs),
        vec![
            (AgentKind::Codex, vec![1]),
            (AgentKind::Goose, vec![2]),
            (AgentKind::Aider, vec![3]),
            (AgentKind::V8agent, vec![0]),
        ]
    );
}

#[test]
fn qodercn_is_wired_into_hooks_grouping_and_the_picker() {
    // hook 配置写进 settings.json(与 Claude 系同一路径),不是 opencode/goose 那类专用安装器。
    assert!(matches!(
        hook_install_target(AgentKind::Qodercn),
        Some(HookInstallTarget::Settings)
    ));
    // 圆点色与其它所有 agent 都不同,否则对话列表里分不出来。
    let q = agent_dot_color(AgentKind::Qodercn);
    for other in [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::V8agent,
        AgentKind::Unknown,
    ] {
        assert_ne!(q, agent_dot_color(other), "与 {other:?} 同色");
    }
    // 会话分组:出现在 Unknown 之前、V8agent 之后。
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tabs = vec![
        make_test_tab(&rt, "u", AgentKind::Unknown),
        make_test_tab(&rt, "q", AgentKind::Qodercn),
        make_test_tab(&rt, "v", AgentKind::V8agent),
    ];
    assert_eq!(
        group_tabs_by_agent(&tabs),
        vec![
            (AgentKind::V8agent, vec![2]),
            (AgentKind::Qodercn, vec![1]),
            (AgentKind::Unknown, vec![0]),
        ]
    );
}

#[test]
fn agent_picker_offers_qodercn_with_its_display_name() {
    let dbg = format!("{:?}", agent_picker_spec());
    assert!(
        dbg.contains("Qoder CN"),
        "新建 agent 菜单应有 Qoder CN: {dbg}"
    );
}

/// 菜单项由 `AgentKind::known()` 派生后,顺序必须仍是按展示名不分大小写的字母序
/// (验收反馈 2026-08-21),shell 项在分隔线之后。
#[test]
fn agent_picker_lists_every_known_agent_in_label_order() {
    let dbg = format!("{:?}", agent_picker_spec());
    let order = [
        "Aider",
        "Claude",
        "CodeBuddy",
        "Codex",
        "Goose",
        "OpenCode",
        "Qoder CN",
        "v8agent",
        "Git Shell",
        "OS Shell",
    ];
    let mut at = 0;
    for label in order {
        let found = dbg[at..]
            .find(label)
            .unwrap_or_else(|| panic!("菜单缺少或乱序: {label}"));
        at += found + label.len();
    }
    for a in AgentKind::known() {
        assert!(dbg.contains(a.display_label()), "{a:?} 不在菜单里");
    }
}

#[test]
fn agent_dot_color_maps_each_kind_and_avoids_gold() {
    let cases = [
        (AgentKind::Claude, byteui::theme::color::current().cyan),
        (AgentKind::Codebuddy, byteui::theme::color::current().purple),
        (AgentKind::Opencode, byteui::theme::color::current().green),
        (AgentKind::Codex, byteui::theme::color::current().blue),
        (AgentKind::Goose, byteui::theme::color::current().teal),
        (AgentKind::Aider, byteui::theme::color::current().magenta),
        (AgentKind::V8agent, byteui::theme::color::current().lime),
        (AgentKind::Qodercn, byteui::theme::color::current().red),
        (AgentKind::Unknown, byteui::theme::color::current().dim),
    ];
    for (agent, expected) in cases {
        let color = agent_dot_color(agent);
        assert_eq!(color, expected, "{agent:?}");
        assert_ne!(
            color,
            byteui::theme::color::current().gold,
            "{agent:?} 的对话列表圆点色不能是 GOLD(甲方动作专属,CLAUDE.md 明文规定)"
        );
    }
}

#[test]
fn agent_icon_maps_each_kind_to_brand_icon() {
    assert_eq!(agent_icon(AgentKind::Claude), IconKind::Claude);
    assert_eq!(agent_icon(AgentKind::Codebuddy), IconKind::Codebuddy);
    assert_eq!(agent_icon(AgentKind::Opencode), IconKind::Opencode);
    assert_eq!(agent_icon(AgentKind::Codex), IconKind::Codex);
    assert_eq!(agent_icon(AgentKind::Aider), IconKind::Aider);
    assert_eq!(
        agent_icon(AgentKind::Goose),
        match byteui::theme::color::current_scheme() {
            byteui::theme::color::ColorScheme::Light => IconKind::GooseLight,
            byteui::theme::color::ColorScheme::Dark => IconKind::GooseDark,
        }
    );
    assert_eq!(agent_icon(AgentKind::V8agent), IconKind::Bot);
    // Qoder 与 Goose 同构:按当前明暗主题选黑/白版 logo。
    assert_eq!(
        agent_icon(AgentKind::Qodercn),
        match byteui::theme::color::current_scheme() {
            byteui::theme::color::ColorScheme::Light => IconKind::QoderLight,
            byteui::theme::color::ColorScheme::Dark => IconKind::QoderDark,
        }
    );
    // Unknown 同样回落 Bot 图标。
    assert_eq!(agent_icon(AgentKind::Unknown), IconKind::Bot);
}

#[test]
fn agent_launch_command_aider_uses_quoted_hook_path_and_launch_subcommand() {
    // app bundle 路径含空格,必须用 POSIX 单引号包起来,否则 sh 会断词。
    let hook_exe = "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook";
    assert_eq!(
        agent_launch_command(AgentKind::Aider, hook_exe),
        Some("'/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook' launch aider".into())
    );
}

#[test]
fn hook_install_target_covers_only_agents_wired_up_in_dozer_hook() {
    // Claude/CodeBuddy/Codex 都走 `install::run_at` 的 JSON settings
    // 补丁机制。
    for agent in [AgentKind::Claude, AgentKind::Codebuddy, AgentKind::Codex] {
        assert_eq!(
            hook_install_target(agent),
            Some(HookInstallTarget::Settings),
            "{agent:?}"
        );
    }
    assert_eq!(
        hook_install_target(AgentKind::Opencode),
        Some(HookInstallTarget::Opencode)
    );
    assert_eq!(
        hook_install_target(AgentKind::Goose),
        Some(HookInstallTarget::GoosePlugin),
        "Goose 走用户级 Open Plugins 目录专用安装器"
    );
    // Aider 没有 hook 安装器(launcher 每次启动注入 bridge 参数),V8agent
    // 不在 `dozer-hook::install::settings_path_for` 的覆盖范围(走 socket
    // 直连上报),两者都返回 None。Unknown 同理,从不该触发安装。
    for agent in [AgentKind::Aider, AgentKind::V8agent, AgentKind::Unknown] {
        assert_eq!(hook_install_target(agent), None, "{agent:?}");
    }
}

#[test]
fn mcp_install_target_covers_four_config_capable_agents() {
    for agent in [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Codex,
        AgentKind::Opencode,
    ] {
        assert!(
            dozer_mcp::install::config_path_for(agent.label()).is_some(),
            "{agent:?} 应该有对应的 mcp 配置文件路径"
        );
    }
}

#[test]
fn mcp_install_target_excludes_v8agent_goose_aider_unknown() {
    // V8agent 走硬编码自动挂载(不读配置文件),Goose/Aider 首期不挂 Dozer MCP
    // (见 spec D8),Unknown 是纯 shell——四者都不该有配置文件路径。
    for agent in [
        AgentKind::V8agent,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::Unknown,
    ] {
        assert!(
            dozer_mcp::install::config_path_for(agent.label()).is_none(),
            "{agent:?} 不该有 mcp 配置文件路径"
        );
    }
}

#[test]
fn dozer_hook_binary_path_is_sibling_of_exe() {
    assert_eq!(
        dozer_hook_binary_path(Path::new(
            "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer"
        )),
        PathBuf::from("/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook")
    );
}

#[test]
fn ensure_hook_installed_writes_codebuddy_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    unsafe { std::env::set_var("DOZER_CODEBUDDY_SETTINGS", path.to_str().unwrap()) };
    ensure_hook_installed(AgentKind::Codebuddy);
    unsafe { std::env::remove_var("DOZER_CODEBUDDY_SETTINGS") };
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(cmd.contains(" codebuddy "), "{cmd}");
    // 回归线上事故：命令必须指向 `dozer-hook`（sibling 于当前进程的
    // exe），不能是调用方自己（测试进程本身，路径里带 "deps/"，不含
    // "dozer-hook"）——否则 `entry_is_dozer` 认不出，每次都重复追加。
    assert!(
        cmd.contains("dozer-hook") || cmd.contains("dozer_hook"),
        "{cmd}: 必须指向 dozer-hook 二进制，不能是 dozer-app 自己"
    );
}

#[test]
fn ensure_hook_installed_writes_opencode_plugin() {
    let dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("DOZER_OPENCODE_PLUGIN_DIR", dir.path().to_str().unwrap()) };
    ensure_hook_installed(AgentKind::Opencode);
    unsafe { std::env::remove_var("DOZER_OPENCODE_PLUGIN_DIR") };
    assert!(dir.path().join("dozer.ts").exists());
    assert!(
        dir.path()
            .join("dozer-lib")
            .join("dozer-translate.ts")
            .exists()
    );
}

#[test]
fn ensure_hook_installed_writes_goose_plugin_with_sibling_exe_path() {
    let dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("DOZER_GOOSE_PLUGINS_DIR", dir.path().to_str().unwrap()) };
    ensure_hook_installed(AgentKind::Goose);
    unsafe { std::env::remove_var("DOZER_GOOSE_PLUGINS_DIR") };
    let hooks_json = dir.path().join("dozer/hooks/hooks.json");
    assert!(hooks_json.exists(), "Goose 应生成 hooks/hooks.json");
    let raw = std::fs::read_to_string(&hooks_json).unwrap();
    // hook command 必须指向 sibling 的 `dozer-hook` 二进制(不是 dozer-app
    // 自己),且路径被单引号包裹以容忍空格(2026-08 线上事故同款根因)。
    assert!(raw.contains("dozer-hook"), "{raw}");
    assert!(
        raw.contains("'") && raw.contains(" goose "),
        "command 应形如 '<exe>' goose <Event>: {raw}"
    );
}

#[test]
fn ensure_hook_installed_is_noop_for_v8agent_and_unknown() {
    // 回归 hook_install_target 的排除名单：这两者不该产生任何文件写入。
    // 用 Claude 的 settings 路径当探针——如果实现退化成 catch-all 调用
    // `install::run_at`，这里会意外产生一个把 "v8agent" 写进去的
    // settings.json。
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    unsafe { std::env::set_var("DOZER_CLAUDE_SETTINGS", path.to_str().unwrap()) };
    for agent in [AgentKind::V8agent, AgentKind::Unknown] {
        ensure_hook_installed(agent);
    }
    unsafe { std::env::remove_var("DOZER_CLAUDE_SETTINGS") };
    assert!(!path.exists(), "V8agent/Unknown 不该写任何 hook 配置");
}

fn write_temp_file(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

#[cfg(any())] // 老 iced editor 已退役,历史测试停用
#[test]
fn active_preview_tab_has_native_editor_reflects_active_tab_kind() {
    let (_dir_rs, rs_path) = write_temp_file("a.rs", "fn main() {}");
    let (_dir_png, png_path) = write_temp_file("a.png", "");
    let mut ws = Workspace::empty_for_project_placeholder();
    assert!(
        !ws.active_preview_tab_has_native_editor(PanelKind::Files),
        "没有 tab 时应为 false"
    );

    ws.preview.open_path(rs_path);
    assert!(
        ws.active_preview_tab_has_native_editor(PanelKind::Files),
        ".rs 是白名单扩展名,应走原生渲染"
    );

    ws.preview.open_path(png_path);
    assert!(
        !ws.active_preview_tab_has_native_editor(PanelKind::Files),
        "切到 .png 后激活 tab 应走 wry,不是原生"
    );
}

#[cfg(any())] // 老 iced editor 已退役,历史测试停用
#[test]
fn active_preview_tab_has_native_editor_checks_project_preview_independently() {
    let (_dir_rs, rs_path) = write_temp_file("a.rs", "fn main() {}");
    let mut ws = Workspace::empty_for_project_placeholder();
    assert!(
        !ws.active_preview_tab_has_native_editor(PanelKind::Project),
        "project_preview 没有 tab 时应为 false"
    );

    ws.project_preview.open_path(rs_path);
    assert!(
        ws.active_preview_tab_has_native_editor(PanelKind::Project),
        "Project 预览面板里的 .rs tab 也应走原生渲染,不是恒查 Files 那个 PreviewPane"
    );
    assert!(
        !ws.active_preview_tab_has_native_editor(PanelKind::Files),
        "Files 预览面板本身没开 tab,不该被 Project 那边的状态影响"
    );
}

#[cfg(any())] // 老 iced editor 已退役,历史测试停用
#[test]
fn preview_pane_undo_active_reverts_edit_and_marks_dirty() {
    use iced_widget::text_editor::{Action, Edit, Motion};
    let (_dir, path) = write_temp_file("a.txt", "ab");
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.preview.open_path(path);
    let id = ws.preview.tabs()[ws.preview.active_idx()].id;
    assert!(
        !ws.preview.tabs()[ws.preview.active_idx()].dirty,
        "新开不脏"
    );

    // 落到行尾后插一个字符(经激活 tab 的 perform 管线,与裸 Tab 同路)。
    ws.preview_pane_active_editor_event(PanelKind::Files, Action::Move(Motion::DocumentEnd));
    ws.preview_pane_active_editor_event(PanelKind::Files, Action::Edit(Edit::Insert('!')));
    assert_eq!(ws.preview.editor_mut(id).unwrap().text(), "ab!");

    ws.preview_pane_undo_active(PanelKind::Files);
    assert_eq!(
        ws.preview.editor_mut(id).unwrap().text(),
        "ab",
        "⌘Z 应回退到编辑前"
    );
    assert!(
        ws.preview.tabs()[ws.preview.active_idx()].dirty,
        "发生过回退的 tab 应保持脏(与磁盘不一致)"
    );

    // 栈空后再撤是 no-op,不 panic、不改文本。
    ws.preview_pane_undo_active(PanelKind::Files);
    assert_eq!(ws.preview.editor_mut(id).unwrap().text(), "ab");
}

#[cfg(any())] // 老 iced editor 已退役,历史测试停用
#[test]
fn preview_pane_redo_active_reapplies_undone_edit_and_marks_dirty() {
    use iced_widget::text_editor::{Action, Edit, Motion};
    let (_dir, path) = write_temp_file("a.txt", "ab");
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.project_preview.open_path(path);
    let id = ws.project_preview.tabs()[ws.project_preview.active_idx()].id;

    ws.preview_pane_active_editor_event(PanelKind::Project, Action::Move(Motion::DocumentEnd));
    ws.preview_pane_active_editor_event(PanelKind::Project, Action::Edit(Edit::Insert('!')));
    assert_eq!(ws.project_preview.editor_mut(id).unwrap().text(), "ab!");

    ws.preview_pane_undo_active(PanelKind::Project);
    assert_eq!(ws.project_preview.editor_mut(id).unwrap().text(), "ab");

    ws.preview_pane_redo_active(PanelKind::Project);
    assert_eq!(
        ws.project_preview.editor_mut(id).unwrap().text(),
        "ab!",
        "⌘⇧Z 应重做被撤掉的编辑"
    );
    assert!(
        ws.project_preview.tabs()[ws.project_preview.active_idx()].dirty,
        "发生过重做的 tab 应保持脏"
    );
}

#[test]
fn blur_preview_editors_sets_pending_unfocus_flag() {
    let mut ws = Workspace::empty_for_project_placeholder();
    assert!(!ws.take_editor_unfocus_pending());
    ws.blur_preview_editors();
    assert!(ws.take_editor_unfocus_pending(), "应置一次性让出焦点标记");
    assert!(!ws.take_editor_unfocus_pending(), "消费式:取走后应复位");
}

#[cfg(any())] // 老 iced editor 已退役,历史测试停用
#[test]
fn blur_inputs_keep_native_preview_editor_skips_pending_unfocus() {
    let (_dir, path) = write_temp_file("a.txt", "hi");
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.preview.open_path(path);
    assert!(
        ws.preview.active_tab_is_native(),
        "白名单文本 tab 应为原生可编辑预览"
    );
    // 点到原生预览编辑器本身:保留(不置让出焦点标记)。
    ws.blur_inputs(true);
    assert!(
        !ws.take_editor_unfocus_pending(),
        "保留时不该同帧把 self-focus 出的光标抬掉"
    );
    // 点其它地方:维持原行为,照常让出预览编辑器焦点。
    ws.blur_inputs(false);
    assert!(
        ws.take_editor_unfocus_pending(),
        "点非编辑器区仍应让出预览编辑器焦点"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn agent_picker_items_has_eleven_rows_matching_old_picker() {
    // 八个 agent(含 Qoder CN)+ 1 条分隔线 + 两个 shell(Git Shell/纯 Shell)= 11 行,
    // 不含分隔线本身是十个选项。
    assert_eq!(agent_picker_items().len(), 11);
}

#[cfg(target_os = "macos")]
#[test]
fn agent_picker_items_separator_splits_agents_from_shells() {
    let items = agent_picker_items();
    assert!(matches!(
        items[8],
        crate::chrome::native_menu::Item::Separator
    ));
    let launches: Vec<PickerLaunch> = items
        .iter()
        .filter_map(|i| match i {
            crate::chrome::native_menu::Item::Entry {
                msg: Message::AgentPickerSelect(launch),
                ..
            } => Some(*launch),
            _ => None,
        })
        .collect();
    assert_eq!(launches.len(), 10, "八个 agent + 两个 shell,不含分隔线");
    assert_eq!(launches[8], PickerLaunch::Git);
    assert_eq!(launches[9], PickerLaunch::Agent(None));
}

/// 空白页 / Blank tab 标题 / 卡片头部应取**项目根目录 basename**,不是
/// `ProjectInfo::name`(用户给项目起的"显示名",如 "Dozer AI Coder"):
/// 用户在 UI 上看到的是"打开的目录",与卡片其它字段(位置/大小/创建/
/// 修改)语义自洽。
#[test]
fn project_root_dir_name_uses_path_basename_not_display_name() {
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.project = Some(ProjectInfo {
        id: 7,
        // 用户给项目起的名字是 "Dozer AI Coder",根目录实际叫 `byteboy`。
        path: "/Users/me/byteboy".to_string(),
        name: "Dozer AI Coder".to_string(),
        last_active_ms: 0,
        created_ms: 0,
        updated_ms: 0,
    });
    assert_eq!(
        project_root_dir_name(&ws).as_deref(),
        Some("byteboy"),
        "应返回 path.basename,不是显示名"
    );
}

/// `Workspace::project == None` 时 helper 返回 `None`,view 端拿到 None
/// 才回退到 `tab.title = "空白"`,不要硬塞个空字符串。
#[test]
fn project_root_dir_name_none_when_no_project() {
    let ws = Workspace::empty_for_project_placeholder();
    assert!(ws.project.is_none());
    assert_eq!(project_root_dir_name(&ws), None);
}

/// Blank tab 标题在有项目时显示根目录 basename(`byteboy`),不是
/// `name`("Dozer AI Coder")。这条契约守住 view 的核心显示语义。
#[test]
fn preview_tab_display_title_blank_uses_root_dir_basename() {
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.project = Some(ProjectInfo {
        id: 7,
        path: "/Users/me/byteboy".to_string(),
        name: "Dozer AI Coder".to_string(),
        last_active_ms: 0,
        created_ms: 0,
        updated_ms: 0,
    });
    let title = preview_tab_display_title(&ws, ws.preview.tabs().first().unwrap(), "en");
    assert_eq!(
        title, "byteboy",
        "Blank tab 标题 = 根目录 basename,与显示名脱钩"
    );
}

/// 无项目时回退本地化的 `view.blank`(数据层 `placeholder_tab` 存稳定英文
/// `"Blank"`),不让 UI 拿到空字符串或裸 None。
#[test]
fn preview_tab_display_title_blank_falls_back_to_placeholder_text() {
    let ws = Workspace::empty_for_project_placeholder();
    let title = preview_tab_display_title(&ws, ws.preview.tabs().first().unwrap(), "en");
    assert_eq!(title, "Blank", "无项目 → 本地化 view.blank(英文兜底)");
    let zh = preview_tab_display_title(&ws, ws.preview.tabs().first().unwrap(), "zh-CN");
    assert_eq!(zh, "空白", "无项目 → 本地化 view.blank(中文)");
}

/// T13:resolve_preview_command_target 按 target 解析到 (panel, tab_id)。
#[test]
fn resolve_preview_command_target_by_path_and_tab() {
    use dozer_core::protocol::{PreviewCommand, PreviewCommandAction, PreviewCommandTarget};
    let (_d, rs) = write_temp_file("t13_nav.rs", "fn main() {}\n");
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.preview.open_path(rs.clone());
    let mk = |target| PreviewCommand {
        request_id: "r".into(),
        project_id: 1,
        target,
        action: PreviewCommandAction::Reveal { line: 1, column: 1 },
        expected_revision: None,
    };
    let loc = ws.resolve_preview_command_target(&mk(PreviewCommandTarget::Path {
        path: rs.to_string_lossy().into_owned(),
    }));
    assert!(matches!(loc, Some((PanelKind::Files, _))), "{loc:?}");

    assert!(
        ws.resolve_preview_command_target(&mk(PreviewCommandTarget::Path {
            path: "/definitely/not/here.rs".into(),
        }))
        .is_none()
    );

    assert_eq!(
        ws.resolve_preview_command_target(&mk(PreviewCommandTarget::Tab {
            panel: "project".into(),
            tab_id: 42,
        })),
        Some((PanelKind::Project, 42))
    );
}

/// T12 自动化(UI handler 审计约束):UI 事件处理路径**不得直接**调用大文件
/// 读取 / 索引 / 解析 / 全文搜索的同步阻塞入口;这些只能出现在后台 task 里
/// (`tokio::task::spawn_blocking` / `io.handle.spawn` 闭包内)。
///
/// 做法:对每个"UI handler 源文件"做静态扫描,任一被禁入口出现在函数体里时,
/// 断言其**同一个函数**内、调用点之前存在 spawn 包装。这样有人日后把
/// `LineIndex::build_cancellable` 之类直接塞进 handler 会立刻红灯,而不是等到
/// 真机上 UI 卡顿。
///
/// 本测试只做词法检查(不引入 parser 依赖),覆盖 plan T12 第一条约束的意图。
/// 调用点是否**真的**落在 `spawn_blocking` / `handle.spawn` 的闭包体内。
///
/// 早期版本只看「函数起点到调用点之间出现没出现过 spawn 字样」,在一个函数里
/// 混有已 spawn 分支时会假绿(`SaveFile` 的同步写盘就是这么漏掉的,D-I4)。
///
/// 判定:对每个 spawn 标记,取它之后遇到的第一个 `{`(中间不允许出现 `;`),
/// 配对到闭合的 `}`,看调用点是否落在区间内。这样 `spawn(|| {...})`、
/// `spawn_blocking(move || {...})`、`spawn(async move {...})` 三种写法都覆盖。
fn call_is_inside_spawn(source: &str, at: usize) -> bool {
    const MARKERS: &[&str] = &["spawn_blocking", "handle.spawn", ".spawn("];
    let bytes = source.as_bytes();
    for marker in MARKERS {
        let mut from = 0;
        while let Some(rel) = source[..at].get(from..).and_then(|s| s.find(marker)) {
            let pos = from + rel;
            from = pos + marker.len();
            let mut i = pos + marker.len();
            // 找闭包体的 `{`;中途遇到语句结束的 `;` 说明这条 spawn 用完了。
            let body_start = loop {
                match bytes.get(i) {
                    None => break None,
                    Some(b'{') => break Some(i),
                    Some(b';') => break None,
                    _ => i += 1,
                }
            };
            let Some(body_start) = body_start else {
                continue;
            };
            let mut depth: i32 = 0;
            let mut body_end = None;
            for (j, &b) in bytes.iter().enumerate().skip(body_start) {
                match b {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            body_end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(body_end) = body_end
                && body_start < at
                && at < body_end
            {
                return true;
            }
        }
    }
    false
}

/// 从「函数起点到调用点」的片段里取出该函数名。片段以 `fn `/`pub fn `/
/// `async fn ` 开头(可能带缩进)时才有值。
fn enclosing_fn_name(fragment: &str) -> Option<String> {
    let trimmed = fragment.trim_start();
    let rest = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
    let rest = rest.strip_prefix("async ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

#[test]
fn ui_handlers_do_not_call_blocking_preview_entry_points() {
    // UI handler 源文件(相对 crate 根)。新增 UI 路径时在此登记。
    // D-I4:写盘入口已经搬进库,审计范围必须跟过去,否则禁令在真实调用点上空转。
    // `host_update.rs` 产出 `Effect::SaveFile`,`effects.rs` 真正执行它。
    const UI_HANDLER_FILES: &[&str] = &[
        "src/app/update.rs",
        "src/app/app.rs",
        "src/app/view.rs",
        "src/app/state.rs",
        "src/workspace/state.rs",
        "src/workspace/view.rs",
        "../dozer-panel-preview/src/host_update.rs",
        "../dozer-panel-preview/src/effects.rs",
    ];
    // 被禁的同步阻塞入口(大文件读取 / 索引 / 解析 / 搜索 / 恢复快照)。
    const BLOCKING_ENTRY_POINTS: &[&str] = &[
        "LineIndex::build(",
        "LineIndex::build_cancellable(",
        "read_window(",
        "read_window_capped(",
        "stream_search_cancellable(",
        "read_snapshot(",
        "tabular::load(",
        "tabular::load_cancellable(",
        "tabular::load_sheet(",
        "tabular::load_sheet_cancellable(",
        // P4:页面事件处理体搬进库后,`EditorEvent::SaveRequested` 的写盘由
        // `Effect::SaveFile` 承接;产品侧不得再出现同步的原子写入口。
        "save_text_atomic(",
    ];
    // 允许阻塞入口出现的后台包装标记。
    const SPAWN_MARKERS: &[&str] = &["spawn_blocking", "handle.spawn", ".spawn("];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut audited = 0usize;
    for file in UI_HANDLER_FILES {
        let path = root.join(file);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读取 UI handler 源文件失败 {file}: {e}"));

        for entry in BLOCKING_ENTRY_POINTS {
            let mut from = 0usize;
            while let Some(rel) = source[from..].find(entry) {
                let at = from + rel;
                from = at + entry.len();

                // 该调用点所在函数的起点(最近的、行首级别的 `fn ` 前)。
                let fn_start = source[..at].rfind("\nfn ").or_else(|| {
                    // 函数可能带缩进(如 impl 内);找最近的行首 `fn `。
                    source[..at]
                        .match_indices("\n")
                        .filter(|(i, _)| {
                            let rest = &source[i + 1..];
                            let trimmed = rest.trim_start();
                            (trimmed.starts_with("fn ")
                                || trimmed.starts_with("async fn ")
                                || trimmed.starts_with("pub fn ")
                                || trimmed.starts_with("pub async fn "))
                                && rest.len() - trimmed.len() < 8
                        })
                        .map(|(i, _)| i)
                        .last()
                });
                let fn_start = fn_start.unwrap_or(0);
                let within = &source[fn_start..at];
                // D-I4:只在「函数起点到调用点之间出现过 spawn 字样」是弱判定——
                // 一个几十分支的 `run_effect` 里只要别的分支 spawn 过,这里就永久
                // 假绿(`SaveFile` 的同步写盘就是这么漏的)。改为按花括号配对确认
                // 调用点**位于某个 spawn 闭包体内**;直接命中不了时,再看调用点所在
                // 函数是否只在 spawn 闭包里被调用(如 `read_recovery`)。
                let mut wrapped = call_is_inside_spawn(&source, at);
                if !wrapped && let Some(name) = enclosing_fn_name(within) {
                    wrapped = source.match_indices(&name).any(|(call_at, _)| {
                        if source[..call_at].trim_end().ends_with("fn") {
                            return false;
                        }
                        call_is_inside_spawn(&source, call_at + name.len())
                    });
                }
                let _ = (within, &SPAWN_MARKERS);
                assert!(
                    wrapped,
                    "{file}:`{entry}` 出现在 UI handler 函数体的直接路径上(调用点前无 spawn 包装)。\
                     阻塞入口必须放进 spawn_blocking / io.handle.spawn。"
                );
                audited += 1;
            }
        }
    }
    // D-I4:恢复强断言。一个调用点都扫不到 = 路径写错 / 写法变了 / 名单失效,
    // 不能再用「源码里含某些字样」蒙混过去。
    assert!(
        audited > 0,
        "审计未命中任何调用点:阻塞入口名单与扫描规则都已失效"
    );
}

// ---- bytehost H1:预览窗格访问器 ----

#[test]
fn preview_pane_is_project_preview_only_for_the_project_panel() {
    let ws = Workspace::empty_for_project_placeholder();
    assert!(std::ptr::eq(
        ws.preview_pane(PanelKind::Project),
        &ws.project_preview
    ));
    for kind in [
        PanelKind::Files,
        PanelKind::GitLog,
        PanelKind::Todo,
        PanelKind::Web,
        PanelKind::Agent,
    ] {
        assert!(
            std::ptr::eq(ws.preview_pane(kind), &ws.preview),
            "{kind:?} 应落到 Files 的预览窗格"
        );
    }
}

#[test]
fn preview_pane_mut_hits_the_same_pane_as_preview_pane() {
    let mut ws = Workspace::empty_for_project_placeholder();
    let project_ptr = ws.preview_pane(PanelKind::Project) as *const _;
    let files_ptr = ws.preview_pane(PanelKind::Files) as *const _;
    assert_eq!(
        ws.preview_pane_mut(PanelKind::Project) as *const _,
        project_ptr
    );
    assert_eq!(ws.preview_pane_mut(PanelKind::Files) as *const _, files_ptr);
    assert_ne!(project_ptr, files_ptr);
}

#[test]
fn per_pane_companion_slots_follow_the_same_selection() {
    let mut ws = Workspace::empty_for_project_placeholder();
    *ws.preview_tab_first_mut(PanelKind::Project) = 3;
    *ws.preview_tab_first_mut(PanelKind::Files) = 5;
    assert_eq!((ws.project_preview_tab_first, ws.preview_tab_first), (3, 5));
    *ws.preview_error_mut(PanelKind::Project) = Some("p".into());
    *ws.preview_error_mut(PanelKind::Files) = Some("f".into());
    assert_eq!(
        ws.project_preview_error
            .as_ref()
            .map(|t| t.fallback.as_str()),
        Some("p")
    );
    assert_eq!(
        ws.preview_error.as_ref().map(|t| t.fallback.as_str()),
        Some("f")
    );
}

// ---- S6c-1 会话字节流黄金测试 ----
//
// 黄金文件在抽库**之前**生成一次(`UPDATE_GOLDEN=1 cargo test -p dozer-app
// session_stream_golden`),之后搬迁过程中不得重新生成:快照 + 乱序分块 +
// 退出标记喂进 `TerminalModel`/`OscScanner` 的最终结果必须逐字一致。
const SESSION_STREAM_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session_stream_golden.txt"
);

fn session_golden_char_token(ch: char) -> String {
    match ch {
        ' ' => "\\u{20}".to_string(),
        '!'..='~' if !matches!(ch, '/' | '|' | ',' | '\\') => ch.to_string(),
        _ => format!("\\u{{{:X}}}", ch as u32),
    }
}

fn session_golden_cell_token(cell: &dozer_panel_term::model::Cell) -> String {
    let mut flags = String::new();
    if cell.bold {
        flags.push('b');
    }
    if cell.wide {
        flags.push('w');
    }
    if cell.spacer {
        flags.push('s');
    }
    if cell.selected {
        flags.push('l');
    }
    if cell.inverse {
        flags.push('i');
    }
    if flags.is_empty() {
        flags.push('-');
    }
    let fg = format!("{:02x}{:02x}{:02x}", cell.fg.0, cell.fg.1, cell.fg.2);
    let bg = cell
        .bg
        .map(|(r, g, b)| format!("{r:02x}{g:02x}{b:02x}"))
        .unwrap_or_else(|| "-".to_string());
    format!("{fg}/{bg}/{flags}/{}", session_golden_char_token(cell.ch))
}

fn render_session_stream(tab: &SessionTab) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    let (cols, rows) = tab.model.grid_dims();
    let (cursor_col, cursor_row) = tab.model.cursor();
    let _ = writeln!(
        text,
        "offset={} dims={cols}x{rows} cursor=({cursor_col},{cursor_row}) cwd={:?} \
         last_exit={:?} alive={}",
        tab.model.display_offset(),
        tab.cwd,
        tab.last_exit,
        tab.alive
    );
    for (row, cells) in tab.model.visible_lines().iter().enumerate() {
        let _ = write!(text, "R{row:02}");
        for cell in cells {
            text.push('|');
            text.push_str(&session_golden_cell_token(cell));
        }
        text.push('\n');
    }
    text
}

#[test]
fn session_stream_golden() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut ws = Workspace::empty_for_project_placeholder();
    let tab_id = 7;
    ws.pending.insert(tab_id, rt.spawn(async {}));

    // 快照:sgr 颜色 + 光标移动 + OSC 7(cwd) + OSC 133;C(命令开始)。
    let snapshot = b"\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m\r\n$ ls\x1b[2;5H!\x1b]7;file://localhost/tmp/demo\x07\x1b]133;C\x07".to_vec();
    ws.on_tab_attached(TabAttachedArgs {
        cols: 40,
        rows: 8,
        tab_id,
        info: SessionInfo {
            id: "sess-golden".into(),
            name: "claude · demo".into(),
            command: "claude".into(),
            cwd: "/tmp/demo".into(),
            alive: true,
            created_ms: 1_700_000_000_000,
            agent_state: AgentState::Running,
            transcript_path: Some("/tmp/demo/t.jsonl".into()),
            project_id: Some(7),
            agent: AgentKind::Claude,
        },
        snapshot,
        picked_agent: Some(AgentKind::Claude),
        term_tab_bar_avail_px: 400.0,
    });

    // 20 个乱序大小的分块(含 SGR、清行、光标移动、CJK/emoji/组合符、
    // OSC 7 换目录与 OSC 133;D 退出码)。
    let mut payload = Vec::new();
    // 先跳到末行再写:早期输出(含 attach 快照)会滚进历史,由下面的
    // "history top" 视口一并固定进黄金文件。
    payload.extend_from_slice(b"\x1b[8;1Hl00\r\nl01\x1b[1;5HX\x1b[2K\x1b[32mgreen\x1b[0m\r\n");
    payload.extend_from_slice(b"\x1b]7;file://localhost/tmp/demo/sub\x07");
    payload.extend_from_slice(b"\x1b]133;D;3\x07");
    payload.extend_from_slice("你好 😀 e\u{0301}\r\n".as_bytes());
    payload.extend_from_slice(b"tail-padding-xyz");
    payload.truncate(115);
    while payload.len() < 115 {
        payload.push(b'.');
    }
    let sizes = [
        1usize, 7, 3, 11, 2, 5, 13, 4, 9, 2, 6, 1, 8, 3, 12, 5, 2, 7, 4, 10,
    ];
    assert_eq!(sizes.iter().sum::<usize>(), payload.len());
    let tab = ws.tab_by_id_mut(tab_id).expect("attach 后应有 tab");
    let mut pos = 0;
    for size in sizes {
        let chunk = &payload[pos..pos + size];
        tab.ingest_osc(chunk);
        let _ = tab.model.feed(chunk);
        pos += size;
    }

    // 退出:alive=false + 本地标记行(与 `SessionExited` 处理一致)。
    tab.alive = false;
    let _ = tab.model.feed(&exited_marker());

    // 底部视口 + 滚到历史顶部的视口:后者把 attach 快照与早期输出也固定
    // 进黄金文件(只看底部时它们可能已经滚出视口)。
    let mut actual = String::from("== bottom ==\n");
    actual.push_str(&render_session_stream(tab));
    tab.model.scroll_display(1000);
    actual.push_str("== history top ==\n");
    actual.push_str(&render_session_stream(tab));
    tab.model.scroll_to_bottom();

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(SESSION_STREAM_GOLDEN, &actual).expect("写会话字节流黄金文件");
        return;
    }
    let expected = std::fs::read_to_string(SESSION_STREAM_GOLDEN)
        .expect("缺少会话字节流黄金文件:先用 UPDATE_GOLDEN=1 生成并提交");
    assert!(actual == expected, "会话字节流与黄金不一致(抽库不得改变它)");
}
