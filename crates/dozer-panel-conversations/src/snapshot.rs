use crate::review::ReviewEntry;
use dozer_model::AgentKind;
/// `dozer://review-trace/data.json` 的响应体形状——`web/review-trace`(Preact host,2026-09-25 起)按这个结构消费
/// (`entries`/`agent_label`/`summary_title`/`summary_text`/`summary_time`
/// 顶层字段,对应该 crate 的 `src/types.ts::TraceData`)。总结区(标题/
/// 全文/时间)可缺省。
#[derive(serde::Serialize)]
pub struct ReviewSnapshot<'a> {
    pub entries: &'a [ReviewEntry],
    /// `AgentKind::label()`(如 `"claude"`)——AI 气泡的头像名字标签用它
    /// 替代写死的"AI"。
    pub agent_label: &'static str,
    /// session 详情顶部总结区标题/全文/相对时间(2026-08-27 起标题/全文;
    /// 2026-08-28 加时间;没有总结时为 `None`)。
    pub summary_title: Option<String>,
    pub summary_text: Option<String>,
    pub summary_time: Option<String>,
}
/// 审阅内容的来源:对话面板的
/// conversation 详情,整段摊平加载。
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewSource {
    /// 对话面板的 session 详情:`conversation_id`,整段摊平加载,不再有
    /// "回合区间"概念(2026-08-27,取代 `FileRange`)。
    Conversation(String),
}
/// 会话审阅详情的数据与 webview 加载水位。
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 这份 transcript 归属的 agent——面板里"AI"气泡的头像/名字标签用它
    /// 而不是写死的"AI"(P2b 多 agent 支持)。
    pub agent: AgentKind,
    /// 审阅 webview 的重新加载水位:每次 `ReviewLoaded` 成功都从
    /// 面板状态的 `review_nonce` 拷一份新值,写进 `dozer://review-trace/
    /// host.html?_r=<nonce>` 的查询参数,逼 wry 在内容变化时重新导航
    /// 拉取(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。
    pub nonce: u64,
    /// session 总结标题/全文(有则展示,无则详情页只显示回合列表)。
    /// 数据来自打开详情时已加载好的 `SessionRow`,不为此单独发请求。
    pub summary_title: Option<String>,
    pub summary_text: Option<String>,
    /// 总结区展示的相对时间(如 "3 分钟前"),打开详情那一刻用
    /// `SessionRow.last_ts` 算一次定格,不随详情页停留时长实时跳字
    /// (2026-08-28 新增)。
    pub summary_time: Option<String>,
    /// 当前 `nonce` 对应的内容是否已经被 review-trace webview 报回
    /// `ReviewTraceEvent::DocumentLoaded`——`None`/不等于 `nonce` 都算"未
    /// loaded"。换审阅目标时 `nonce` 自增,这个字段不需要显式清空,天然因
    /// 为不等于新 `nonce` 而回到"未 loaded"(见 `review_webview_spec`)。
    pub loaded_nonce: Option<u64>,
}
/// Mark the current nonce loaded when the host reports document_loaded.
/// The event carries no nonce, so late events retain the legacy current-nonce behavior.
pub fn mark_review_loaded(review: &mut Option<ReviewView>) {
    if let Some(rv) = review {
        rv.loaded_nonce = Some(rv.nonce);
    }
}
/// Refresh the open conversation summary and return a new serialized snapshot.
/// Advance the nonce only when the matching row changes the displayed summary.
pub fn refresh_open_conversation_summary(
    review: &mut Option<ReviewView>,
    review_nonce: &mut u64,
    row: &dozer_model::SessionRow,
) -> Option<String> {
    let rv = review.as_mut()?;
    if !matches!(&rv.source, ReviewSource::Conversation(cid) if cid == &row.conversation_id) {
        return None;
    }
    if rv.summary_title.as_deref() == Some(row.display_title.as_str())
        && rv.summary_text == row.summary
    {
        return None;
    }

    rv.summary_title = Some(row.display_title.clone());
    rv.summary_text = row.summary.clone();
    *review_nonce = review_nonce.wrapping_add(1);
    rv.nonce = *review_nonce;
    let snapshot = ReviewSnapshot {
        entries: &rv.entries,
        agent_label: rv.agent.label(),
        summary_title: rv.summary_title.clone(),
        summary_text: rv.summary_text.clone(),
        summary_time: rv.summary_time.clone(),
    };
    serde_json::to_string(&snapshot).ok()
}
/// `review_content_pane` 该渲染哪一态(空/loading/内容三态)。抽成纯函数
/// 供 headless 单测——视图函数本身只管把结果画出来。
#[derive(Debug, PartialEq)]
pub enum ReviewPaneState {
    Empty,
    Loading,
    Ready,
}

pub fn review_pane_state(rv: Option<&ReviewView>) -> ReviewPaneState {
    let Some(rv) = rv else {
        return ReviewPaneState::Empty;
    };
    // error / 空 entries 两种情况下 `review_webview_spec` 会销毁 webview
    // (永远等不到 `document_loaded`),`loaded_nonce` 永远是 None——必须归入
    // Ready 让 `review_content` 显示"⚠ 错误" / "暂无对话"文案,否则会永久
    // 卡在"加载中…"占位。这跟"根本没有数据"、"正在加载"是两回事。
    if rv.error.is_some() || rv.entries.is_empty() || rv.loaded_nonce == Some(rv.nonce) {
        ReviewPaneState::Ready
    } else {
        ReviewPaneState::Loading
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewWebviewSpec {
    pub url: String,
    pub visible: bool,
    pub park_offscreen: bool,
}
pub fn review_webview_spec(
    review: Option<&ReviewView>,
    locale: &str,
    scheme: &str,
) -> Option<ReviewWebviewSpec> {
    let rv = review?;
    if rv.error.is_some() || rv.entries.is_empty() {
        return None;
    }
    let loaded = rv.loaded_nonce == Some(rv.nonce);
    Some(ReviewWebviewSpec {
        url: format!(
            "{scheme}://review-trace/host.html?_r={}&locale={}",
            rv.nonce, locale
        ),
        visible: loaded,
        park_offscreen: !loaded,
    })
}
pub fn current_review_conversation_id(review: Option<&ReviewView>) -> Option<&str> {
    let ReviewSource::Conversation(id) = &review?.source;
    Some(id)
}

#[cfg(test)]
mod review_trace_tests {
    use super::*;
    use crate::review::ReviewEntry;
    use crate::snapshot::ReviewSource;
    use dozer_model::AgentKind;

    fn review(nonce: u64, loaded_nonce: Option<u64>) -> ReviewView {
        ReviewView {
            source: ReviewSource::Conversation("c1".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce,
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce,
        }
    }

    #[test]
    fn sets_loaded_nonce_to_current_nonce() {
        let mut review = Some(review(5, None));
        mark_review_loaded(&mut review);
        assert_eq!(review.as_ref().unwrap().loaded_nonce, Some(5));
    }

    /// 对应 Review Focus"过期事件":事件到达时 nonce 已经自增,`loaded_nonce`
    /// 只能追到**当前**值(6)而非事件本该对应的旧值——已知限制,事件不携带
    /// nonce。这条测试记录该行为,避免未来被误当成未测疏漏。
    #[test]
    fn stale_event_still_advances_to_current_nonce_known_limitation() {
        let mut review = Some(review(6, None));
        mark_review_loaded(&mut review);
        assert_eq!(review.as_ref().unwrap().loaded_nonce, Some(6));
    }

    #[test]
    fn no_op_when_review_is_none() {
        let mut review: Option<ReviewView> = None;
        mark_review_loaded(&mut review);
        assert!(review.is_none());
    }

    #[test]
    fn refreshed_summary_rebuilds_snapshot_and_advances_nonce() {
        let mut review = Some(review(5, Some(5)));
        let mut nonce = 5;
        let row = dozer_model::SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 1,
            display_title: "新标题".into(),
            summary: Some("新总结".into()),
            summary_status: Some(dozer_model::SummaryStatus::AiGenerated),
            task_id: None,
        };

        let json = refresh_open_conversation_summary(&mut review, &mut nonce, &row)
            .expect("总结变化应重建快照");

        let rv = review.as_ref().unwrap();
        assert_eq!(rv.summary_title.as_deref(), Some("新标题"));
        assert_eq!(rv.summary_text.as_deref(), Some("新总结"));
        assert_eq!(rv.nonce, 6);
        assert_eq!(nonce, 6);
        assert_eq!(
            rv.loaded_nonce,
            Some(5),
            "旧加载水位应使新 WebView 进入加载态"
        );
        assert!(json.contains("新标题"));
        assert!(json.contains("新总结"));
    }

    #[test]
    fn unchanged_summary_does_not_reload_webview() {
        let mut value = review(5, Some(5));
        value.summary_title = Some("已有标题".into());
        value.summary_text = Some("已有总结".into());
        let mut review = Some(value);
        let mut nonce = 5;
        let row = dozer_model::SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 1,
            display_title: "已有标题".into(),
            summary: Some("已有总结".into()),
            summary_status: Some(dozer_model::SummaryStatus::AiGenerated),
            task_id: None,
        };

        assert!(refresh_open_conversation_summary(&mut review, &mut nonce, &row).is_none());
        assert_eq!(nonce, 5);
        assert_eq!(review.as_ref().unwrap().nonce, 5);
    }
}

#[cfg(test)]
mod review_pane_state_tests {
    use super::*;
    use crate::review::ReviewEntry;

    fn rv(nonce: u64, loaded_nonce: Option<u64>) -> ReviewView {
        ReviewView {
            source: ReviewSource::Conversation("c1".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce,
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce,
        }
    }

    #[test]
    fn none_is_empty() {
        assert_eq!(review_pane_state(None), ReviewPaneState::Empty);
    }

    #[test]
    fn unloaded_nonce_is_loading() {
        assert_eq!(
            review_pane_state(Some(&rv(3, None))),
            ReviewPaneState::Loading
        );
        assert_eq!(
            review_pane_state(Some(&rv(3, Some(2)))),
            ReviewPaneState::Loading
        );
    }

    #[test]
    fn matching_nonce_is_ready() {
        assert_eq!(
            review_pane_state(Some(&rv(3, Some(3)))),
            ReviewPaneState::Ready
        );
    }

    /// 首次加载失败:`review_webview_spec` 会销毁 webview(永远等不到
    /// `document_loaded`),`loaded_nonce` 停在 None——必须走 Ready 让
    /// `review_content` 显示"⚠ 错误"文案,不能永久卡在"加载中…"。
    #[test]
    fn error_is_ready_even_without_loaded_nonce() {
        let mut r = rv(3, None);
        r.error = Some("transcript 解析失败".into());
        assert_eq!(review_pane_state(Some(&r)), ReviewPaneState::Ready);
    }

    /// 首次加载成功但会话为空:同样销毁 webview、无事件回传,必须走
    /// Ready 显示"暂无对话",不能卡在"加载中…"。
    #[test]
    fn empty_entries_is_ready_even_without_loaded_nonce() {
        let mut r = rv(3, None);
        r.entries = Vec::new();
        assert_eq!(review_pane_state(Some(&r)), ReviewPaneState::Ready);
    }
}

#[cfg(test)]
mod legacy_spec_tests {
    use super::*;
    fn review_webview_spec(rv: Option<&ReviewView>, locale: &str) -> Vec<ReviewWebviewSpec> {
        super::review_webview_spec(rv, locale, "dozer")
            .into_iter()
            .collect()
    }
    fn review_source_conversation_id<'a>(
        source: &ReviewSource,
        _path: impl FnOnce(usize) -> Option<&'a str>,
    ) -> Option<String> {
        let ReviewSource::Conversation(id) = source;
        let rv = ReviewView {
            source: ReviewSource::Conversation(id.clone()),
            entries: vec![],
            error: None,
            agent: AgentKind::Claude,
            nonce: 0,
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce: None,
        };
        current_review_conversation_id(Some(&rv)).map(str::to_owned)
    }
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
}
