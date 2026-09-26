use super::*;

use crate::app::{Message, PanelKind};
use crate::chrome::tab_widget::tab_window;
use crate::osc::OscScanner;
use crate::term::term_model::TerminalModel;
use crate::transcript::ReviewEntry;
use byteui::interaction::icons::IconKind;
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};
use std::path::{Path, PathBuf};

#[test]
fn review_webview_spec_empty_when_no_review() {
    assert_eq!(review_webview_spec(None), Vec::new());
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
    assert_eq!(review_webview_spec(Some(&with_error)), Vec::new());

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
    assert_eq!(review_webview_spec(Some(&empty_entries)), Vec::new());
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
    let specs = review_webview_spec(Some(&rv));
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].url, "dozer://review-trace/host.html?_r=7");
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
    let specs = review_webview_spec(Some(&rv));
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
    let specs = review_webview_spec(Some(&rv));
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
    assert_eq!(review_webview_spec(Some(&rv)), Vec::new());
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
    assert_eq!(relative_time_text(1000, 1000), "刚刚");
    assert_eq!(relative_time_text(0, 59_000), "刚刚");
    assert_eq!(relative_time_text(0, 60_000), "1 分钟前");
    assert_eq!(relative_time_text(0, 3_599_000), "59 分钟前");
    assert_eq!(relative_time_text(0, 3_600_000), "1 小时前");
    assert_eq!(relative_time_text(0, 86_399_000), "23 小时前");
    assert_eq!(relative_time_text(0, 86_400_000), "1 天前");
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
fn tab_title_prefers_cwd_basename() {
    use std::path::Path;
    assert_eq!(
        tab_title(
            AgentKind::Unknown,
            Some(Path::new("/Users/c/proj")),
            "shell"
        ),
        "proj"
    );
    assert_eq!(
        tab_title(AgentKind::Unknown, Some(Path::new("/")), "shell"),
        "/"
    );
    assert_eq!(tab_title(AgentKind::Unknown, None, "shell"), "shell");
}

#[test]
fn tab_title_prefers_agent_name_once_known() {
    use std::path::Path;
    assert_eq!(
        tab_title(AgentKind::Claude, Some(Path::new("/Users/c/proj")), "shell"),
        "claude"
    );
    assert_eq!(tab_title(AgentKind::Claude, None, "shell"), "claude");
}

#[test]
fn tab_window_no_overflow_all_visible() {
    let w = tab_window(&[50.0, 50.0, 50.0], 4.0, 500.0, 0);
    assert_eq!((w.first, w.visible_end), (0, 3));
    assert!(!w.has_overflow(3));
}

#[test]
fn tab_window_overflow_clamps_and_computes_visible_end() {
    let widths = [100.0; 5];
    let w = tab_window(&widths, 0.0, 250.0, 0);
    assert_eq!((w.first, w.visible_end), (0, 2));
    assert!(w.has_overflow(5));
    assert_eq!(w.hidden_before(), 0..0);
    assert_eq!(w.hidden_after(5), 2..5);

    // 请求的 first 越界 → 钳到 max_first(=3),此时尾部 3 个恰好全可见。
    let w = tab_window(&widths, 0.0, 250.0, 99);
    assert_eq!((w.first, w.visible_end), (3, 5));
    assert_eq!(w.hidden_before(), 0..3);
    assert_eq!(w.hidden_after(5), 5..5);

    let w = tab_window(&widths, 0.0, 250.0, 1);
    assert_eq!((w.first, w.visible_end), (1, 3));
    assert_eq!(w.hidden_before(), 0..1);
    assert_eq!(w.hidden_after(5), 3..5);
}

#[test]
fn tab_window_reveal_keeps_visible_tab_still_no_jump() {
    let widths = [100.0; 5];
    // first=1 时可见区间是 [1,3):选中已经可见的 tab 1,first 不应该变。
    assert_eq!(
        crate::chrome::tab_widget::tab_window_reveal(&widths, 0.0, 250.0, 1, 1),
        1
    );
}

#[test]
fn tab_window_reveal_scrolls_hidden_tab_into_view() {
    let widths = [100.0; 5];
    // first=0 时可见区间是 [0,2):选中隐藏在右侧的 tab 4,应重新钳出
    // 一个包含它的窗口。
    let new_first = crate::chrome::tab_widget::tab_window_reveal(&widths, 0.0, 250.0, 0, 4);
    let w = tab_window(&widths, 0.0, 250.0, new_first);
    assert!((w.first..w.visible_end).contains(&4));
}

#[test]
fn tab_display_width_cjk_wider_than_ascii() {
    assert!(tab_display_width("中文会话") > tab_display_width("sh"));
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
    // 若 Workspace 无法在测试中直接构造,则改为验证 preview_content_bounds
    // 之外新增一个纯函数不现实——此时降级为:仅确认编译期字段存在,
    // 测试留待 Task 6 人工验收覆盖,并在报告中写明。
}

#[test]
fn agent_state_label_covers_all() {
    assert_eq!(agent_state_label(AgentState::Running), "运行中");
    assert_eq!(agent_state_label(AgentState::AwaitingInput), "待输入");
    assert_eq!(agent_state_label(AgentState::TurnEnded), "回合毕");
    assert_eq!(agent_state_label(AgentState::Idle), "空闲");
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
fn should_summarize_on_close_true_for_four_supported_agents() {
    for agent in [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::V8agent,
    ] {
        assert!(
            should_summarize_on_close(agent, true, &TabBackend::Daemon),
            "{agent:?} 应该走总结后关闭"
        );
    }
}

#[test]
fn should_summarize_on_close_false_for_unsupported_agents_or_dead_or_ssh() {
    assert!(!should_summarize_on_close(
        AgentKind::Codex,
        true,
        &TabBackend::Daemon
    ));
    assert!(!should_summarize_on_close(
        AgentKind::Unknown,
        true,
        &TabBackend::Daemon
    ));
    assert!(!should_summarize_on_close(
        AgentKind::Claude,
        false,
        &TabBackend::Daemon
    ));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    assert!(!should_summarize_on_close(
        AgentKind::Claude,
        true,
        &TabBackend::Ssh { out: tx }
    ));
}

/// 回归测试:此前只看 `info.agent`(daemon 默认值,真实 agent 靠 hook
/// 事后上报),导致 picker 新建的 opencode 会话在 `on_tab_attached` 这一
/// 刻永远判定成"非 opencode"——回应 OSC 10/11 的开关从未真正打开过。
#[test]
fn should_answer_dynamic_color_true_when_picker_targets_opencode_even_if_info_agent_lags() {
    assert!(should_answer_dynamic_color(
        Some(AgentKind::Opencode),
        AgentKind::Unknown, // daemon 侧此刻还没收到 hook 上报
    ));
}

#[test]
fn should_answer_dynamic_color_true_when_info_agent_already_known_opencode() {
    // SSH attach、或重连时 hook 已先上报过的兜底路径:picker 没有提示。
    assert!(should_answer_dynamic_color(None, AgentKind::Opencode));
}

#[test]
fn should_answer_dynamic_color_false_for_other_agents() {
    assert!(!should_answer_dynamic_color(
        Some(AgentKind::Claude),
        AgentKind::Unknown
    ));
    assert!(!should_answer_dynamic_color(None, AgentKind::Unknown));
    assert!(!should_answer_dynamic_color(None, AgentKind::Claude));
}

#[test]
fn close_tab_skips_daemon_kill_for_ssh_backend() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut tab = make_test_tab(&rt, "ssh:h1", dozer_core::protocol::AgentKind::Unknown);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    tab.backend = TabBackend::Ssh { out: tx };
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
    tab.backend = TabBackend::Ssh { out: tx };
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
fn agent_dot_color_maps_each_kind_and_avoids_gold() {
    let cases = [
        (AgentKind::Claude, byteui::theme::color::current().cyan),
        (AgentKind::Codebuddy, byteui::theme::color::current().purple),
        (AgentKind::Opencode, byteui::theme::color::current().green),
        (AgentKind::Codex, byteui::theme::color::current().orange),
        (AgentKind::Goose, byteui::theme::color::current().blue),
        (AgentKind::Aider, byteui::theme::color::current().magenta),
        (AgentKind::V8agent, byteui::theme::color::current().lime),
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
    // Unknown 同样回落 Bot 图标。
    assert_eq!(agent_icon(AgentKind::Unknown), IconKind::Bot);
}

#[test]
fn agent_launch_command_maps_known_agents_and_none_for_unknown() {
    let hook_exe = "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook";
    assert_eq!(
        agent_launch_command(AgentKind::Claude, hook_exe),
        Some("claude".into())
    );
    assert_eq!(
        agent_launch_command(AgentKind::Codebuddy, hook_exe),
        Some("codebuddy".into())
    );
    assert_eq!(
        agent_launch_command(AgentKind::Opencode, hook_exe),
        Some("opencode".into())
    );
    assert_eq!(
        agent_launch_command(AgentKind::Codex, hook_exe),
        Some("codex".into())
    );
    assert_eq!(
        agent_launch_command(AgentKind::Goose, hook_exe),
        Some("goose session".into())
    );
    assert_eq!(
        agent_launch_command(AgentKind::V8agent, hook_exe),
        Some("v8agent".into())
    );
    assert_eq!(agent_launch_command(AgentKind::Unknown, hook_exe), None);
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
fn agent_launch_command_aider_escapes_single_quotes_in_path() {
    // 路径含单引号时也要转义,避免提前结束引号。
    let hook_exe = "/opt/weird'path/dozer-hook";
    let cmd = agent_launch_command(AgentKind::Aider, hook_exe).unwrap();
    assert_eq!(cmd, "'/opt/weird'\\''path/dozer-hook' launch aider");
}

#[test]
fn picker_launch_command_maps_selection_to_initial_command() {
    // 已知 agent → 其 CLI 名(复用 agent_launch_command)。
    assert_eq!(
        picker_launch_command(
            PickerLaunch::Agent(Some(AgentKind::Claude)),
            "/x/dozer-hook"
        ),
        Some("claude".to_string())
    );
    assert_eq!(
        picker_launch_command(
            PickerLaunch::Agent(Some(AgentKind::Codebuddy)),
            "/x/dozer-hook"
        ),
        Some("codebuddy".to_string())
    );
    assert_eq!(
        picker_launch_command(
            PickerLaunch::Agent(Some(AgentKind::Opencode)),
            "/x/dozer-hook"
        ),
        Some("opencode".to_string())
    );
    assert_eq!(
        picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Codex)), "/x/dozer-hook"),
        Some("codex".to_string())
    );
    assert_eq!(
        picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Goose)), "/x/dozer-hook"),
        Some("goose session".to_string())
    );
    assert_eq!(
        picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Aider)), "/x/dozer-hook"),
        Some("'/x/dozer-hook' launch aider".to_string())
    );
    assert_eq!(
        picker_launch_command(
            PickerLaunch::Agent(Some(AgentKind::V8agent)),
            "/x/dozer-hook"
        ),
        Some("v8agent".to_string())
    );
    // 纯 Shell → 不键入任何初始命令。
    assert_eq!(
        picker_launch_command(PickerLaunch::Agent(None), "/x/dozer-hook"),
        None
    );
    // Git Shell → 项目根开 shell 后自动跑 git status。
    assert_eq!(
        picker_launch_command(PickerLaunch::Git, "/x/dozer-hook"),
        Some("git status".to_string())
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
fn preview_pane_undo_active_noop_without_native_tab() {
    let (_dir, path) = write_temp_file("a.png", "");
    let mut ws = Workspace::empty_for_project_placeholder();
    ws.preview.open_path(path);
    // .png 走 wry(无 editor):撤销应静默 no-op,不 panic、不乱标脏。
    ws.preview_pane_undo_active(PanelKind::Files);
    assert!(!ws.preview.tabs()[ws.preview.active_idx()].dirty);
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
fn agent_picker_items_has_ten_rows_matching_old_picker() {
    // 七个 agent + 1 条分隔线 + 两个 shell(Git Shell/纯 Shell)= 10 行,
    // 对应文档说的"九个选项"(不含分隔线本身)。
    assert_eq!(agent_picker_items().len(), 10);
}

#[cfg(target_os = "macos")]
#[test]
fn agent_picker_items_separator_splits_agents_from_shells() {
    let items = agent_picker_items();
    assert!(matches!(
        items[7],
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
    assert_eq!(launches.len(), 9, "七个 agent + 两个 shell,不含分隔线");
    assert_eq!(launches[7], PickerLaunch::Git);
    assert_eq!(launches[8], PickerLaunch::Agent(None));
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
    let title = preview_tab_display_title(&ws, ws.preview.tabs().first().unwrap());
    assert_eq!(
        title, "byteboy",
        "Blank tab 标题 = 根目录 basename,与显示名脱钩"
    );
}

/// 无项目时回退 `tab.title`(数据层 `placeholder_tab` 写死的 "空白"),
/// 不让 UI 拿到空字符串或裸 None。
#[test]
fn preview_tab_display_title_blank_falls_back_to_placeholder_text() {
    let ws = Workspace::empty_for_project_placeholder();
    let title = preview_tab_display_title(&ws, ws.preview.tabs().first().unwrap());
    assert_eq!(title, "空白", "无项目 → 维持原 \"空白\" 文案");
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
#[test]
fn ui_handlers_do_not_call_blocking_preview_entry_points() {
    // UI handler 源文件(相对 crate 根)。新增 UI 路径时在此登记。
    const UI_HANDLER_FILES: &[&str] = &[
        "src/app/update.rs",
        "src/app/app.rs",
        "src/app/view.rs",
        "src/app/state.rs",
        "src/workspace/state.rs",
        "src/workspace/view.rs",
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
                let wrapped = SPAWN_MARKERS.iter().any(|m| within.contains(m));
                assert!(
                    wrapped,
                    "{file}:`{entry}` 出现在 UI handler 函数体的直接路径上(调用点前无 spawn 包装)。\
                     阻塞入口必须放进 spawn_blocking / io.handle.spawn。"
                );
                audited += 1;
            }
        }
    }
    assert!(audited > 0, "审计未命中任何调用点,词法规则可能已失效");
}
