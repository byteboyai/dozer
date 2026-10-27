//! 状态机 / 收藏夹 / `update` 的单元测试(自 `dozer-app::extensions::browser` 原样搬来,断言不变)。

use crate::bookmarks::{
    OPTIMISTIC_BOOKMARK_ID, bookmark_status, optimistic_add, optimistic_remove,
};
use crate::testing::{FakeBookmark, TIMEOUT, runtime};
use crate::{Message, State, Tabs, WEB_OWNER, request_bookmarks_refresh, update};
use dozer_model::{BookmarkInfo, BookmarkScope};
use dozer_services::ServiceErrorKind;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

fn service_for_test() -> std::sync::Arc<dyn dozer_services::BookmarkService> {
    FakeBookmark::default().into_service()
}

fn bm(id: i64, scope: BookmarkScope, project_id: Option<i64>, url: &str) -> BookmarkInfo {
    BookmarkInfo {
        id,
        scope,
        project_id,
        url: url.into(),
        title: url.into(),
        created_ms: 0,
    }
}

#[test]
fn bookmark_status_detects_global_and_project_independently() {
    let list = vec![
        bm(1, BookmarkScope::Global, None, "https://a.com"),
        bm(2, BookmarkScope::Project, Some(7), "https://a.com"),
    ];
    let status = bookmark_status(&list, "https://a.com", Some(7));
    assert_eq!(status.global, Some(1));
    assert_eq!(status.project, Some(2));
    assert!(status.is_bookmarked());
}

#[test]
fn bookmark_status_project_none_when_no_project_open() {
    let list = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
    let status = bookmark_status(&list, "https://a.com", None);
    assert_eq!(status.global, Some(1));
    assert_eq!(status.project, None);
}

#[test]
fn bookmark_status_project_scoped_to_current_project_only() {
    let list = vec![bm(1, BookmarkScope::Project, Some(7), "https://a.com")];
    let status = bookmark_status(&list, "https://a.com", Some(8));
    assert_eq!(status.project, None, "不该看到别的项目的收藏");
}

#[test]
fn is_bookmarked_false_when_neither_side_has_it() {
    let list: Vec<BookmarkInfo> = vec![];
    assert!(!bookmark_status(&list, "https://a.com", Some(1)).is_bookmarked());
}

#[test]
fn optimistic_add_appends_new_entry() {
    let mut list = vec![];
    optimistic_add(
        &mut list,
        BookmarkScope::Global,
        None,
        "https://a.com",
        "A",
        100,
    );
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, OPTIMISTIC_BOOKMARK_ID);
    assert_eq!(list[0].title, "A");
}

#[test]
fn optimistic_add_is_idempotent_for_same_scope_and_url() {
    let mut list = vec![];
    optimistic_add(
        &mut list,
        BookmarkScope::Global,
        None,
        "https://a.com",
        "A",
        100,
    );
    optimistic_add(
        &mut list,
        BookmarkScope::Global,
        None,
        "https://a.com",
        "改名",
        200,
    );
    assert_eq!(list.len(), 1, "已存在则不重复插入");
    assert_eq!(list[0].title, "A");
}

#[test]
fn optimistic_add_allows_same_url_in_different_scope() {
    let mut list = vec![];
    optimistic_add(
        &mut list,
        BookmarkScope::Global,
        None,
        "https://a.com",
        "A",
        100,
    );
    optimistic_add(
        &mut list,
        BookmarkScope::Project,
        Some(1),
        "https://a.com",
        "A",
        100,
    );
    assert_eq!(list.len(), 2);
}

#[test]
fn optimistic_remove_filters_by_id() {
    let mut list = vec![
        bm(1, BookmarkScope::Global, None, "https://a.com"),
        bm(2, BookmarkScope::Global, None, "https://b.com"),
    ];
    optimistic_remove(&mut list, 1);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, 2);
}

#[test]
fn optimistic_remove_unknown_id_is_noop() {
    let mut list = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
    optimistic_remove(&mut list, 999);
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn update_open_url_clears_error() {
    let mut state = State {
        error: Some(dozer_model::LocalizedText::literal(
            WEB_OWNER,
            "stale error",
        )),
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::OpenUrl("http://a.com".into()),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert!(state.error.is_none());
    assert_eq!(state.tabs.tabs().len(), 2);
}

#[tokio::test]
async fn update_title_loaded_applies_html_title() {
    let mut state = State {
        requests: Default::default(),
        tabs: Tabs::default(),
        error: None,
        bookmarks: Vec::new(),
        bookmarks_open: false,
        hover: Default::default(),
        tooltip_starts: Default::default(),
        addr_select_all_pending: false,
    };
    let id = state.tabs.open_url("https://example.com".into());
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::TitleLoaded(id, "Example Home".into()),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(state.tabs.tabs()[0].title, "Example Home");
    // 空标题不覆盖 URL 标题。
    update(
        &mut state,
        Message::TitleLoaded(id, "".into()),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(state.tabs.tabs()[0].title, "Example Home");
}

#[tokio::test]
async fn update_addr_submit_opens_in_active_tab() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::AddrInput("http://a.com".to_string()),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    update(
        &mut state,
        Message::AddrSubmit,
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(
        state.tabs.tabs().len(),
        1,
        "默认带一个 about:blank(下标 0),提交应在该激活 tab 打开而不新增"
    );
    assert_eq!(state.tabs.tabs()[0].url, "http://a.com");
}

#[tokio::test]
async fn update_addr_submit_local_path_sets_error_without_opening_tab() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::AddrInput("/tmp/x".to_string()),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    update(
        &mut state,
        Message::AddrSubmit,
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(
        state.error,
        Some(dozer_model::LocalizedText::keyed(
            WEB_OWNER,
            "error.local_path",
            "Opening local files is not supported in the browser"
        ))
    );
    assert_eq!(
        state.tabs.tabs().len(),
        1,
        "仍是默认的 about:blank,未新增 tab"
    );
}

#[tokio::test]
async fn update_star_click_toggles_bookmark() {
    let mut state = State {
        error: Some(dozer_model::LocalizedText::literal(
            WEB_OWNER,
            "stale error",
        )),
        ..State::default()
    };
    state.tabs.open_url("http://a.com".into());
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();

    // 未收藏 → 点击星标直接加入全局收藏(并清掉旧错误)。
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    update(
        &mut state,
        Message::StarClick,
        Some(1),
        &service,
        &handle,
        move |m| sink.lock().unwrap().push(m),
    );
    assert!(state.error.is_none());
    // 内核会把这些派发消息继续喂回 update(模拟 dispatch)。
    for m in emitted.lock().unwrap().drain(..) {
        update(&mut state, m, Some(1), &service, &handle, |_| {});
    }
    assert_eq!(state.bookmarks.len(), 1);
    assert_eq!(state.bookmarks[0].scope, BookmarkScope::Global);
    assert_eq!(state.bookmarks[0].project_id, None);

    // 已收藏 → 再点星标取消收藏。
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    update(
        &mut state,
        Message::StarClick,
        Some(1),
        &service,
        &handle,
        move |m| sink.lock().unwrap().push(m),
    );
    for m in emitted.lock().unwrap().drain(..) {
        update(&mut state, m, Some(1), &service, &handle, |_| {});
    }
    assert!(state.bookmarks.is_empty());
}

#[tokio::test]
async fn update_bookmarks_toggle_flips_panel() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarksToggle,
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert!(state.bookmarks_open);
}

#[tokio::test]
async fn update_bookmark_add_without_active_tab_is_noop() {
    // `State::default()` 现在自带一个 about:blank 标签页,不再是空标签;
    // 要测"无激活 tab 时收藏是 no-op"的守卫分支,显式构造一个空标签状态。
    let mut state = State {
        requests: Default::default(),
        tabs: Tabs::default(),
        error: None,
        bookmarks: Vec::new(),
        bookmarks_open: false,
        hover: HashMap::new(),
        tooltip_starts: HashMap::new(),
        addr_select_all_pending: false,
    };
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarkAdd(BookmarkScope::Global),
        Some(1),
        &service,
        &handle,
        |_| panic!("无 tab 时不该 emit"),
    );
    assert!(state.bookmarks.is_empty());
}

#[tokio::test]
async fn update_bookmark_add_global_optimistically_inserts() {
    let mut state = State::default();
    state.tabs.open_url("http://a.com".into());
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarkAdd(BookmarkScope::Global),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(state.bookmarks.len(), 1);
    assert_eq!(state.bookmarks[0].scope, BookmarkScope::Global);
    assert_eq!(state.bookmarks[0].project_id, None);
}

#[tokio::test]
async fn update_bookmark_add_project_scope_without_project_id_is_local_only() {
    let mut state = State::default();
    state.tabs.open_url("http://a.com".into());
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarkAdd(BookmarkScope::Project),
        None,
        &service,
        &handle,
        |_| panic!("project_id 为 None 时不该 emit 网络请求"),
    );
    assert_eq!(state.bookmarks.len(), 1, "本地乐观更新仍然发生");
}

#[tokio::test]
async fn update_bookmark_remove_filters_local_cache() {
    let mut state = State {
        bookmarks: vec![bm(1, BookmarkScope::Global, None, "https://a.com")],
        ..State::default()
    };
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarkRemove(1),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert!(state.bookmarks.is_empty());
}

#[tokio::test]
async fn update_bookmarks_loaded_replaces_local_cache() {
    let mut state = State {
        bookmarks: vec![bm(1, BookmarkScope::Global, None, "https://stale.com")],
        ..State::default()
    };
    let fresh = vec![bm(2, BookmarkScope::Global, None, "https://fresh.com")];
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarksLoaded(Some(1), Ok(fresh.clone())),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(state.bookmarks, fresh);
}

#[tokio::test]
async fn update_bookmarks_mutated_error_sets_state_error() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    update(
        &mut state,
        Message::BookmarksMutated(Some(1), Err("boom".to_string())),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(
        state.error,
        Some(crate::i18n::panel_keyed_args(
            WEB_OWNER,
            "error.bookmarks_op_failed",
            "Bookmark operation failed: {e}",
            &[("e", "boom")],
        ))
    );
}

#[test]
fn state_accessors_delegate_to_tabs() {
    let mut state = State::default();
    assert!(!state.addr_focused());
    // 默认带一个 about:blank 标签页,它就是激活 tab
    assert_eq!(state.active_webview_id(), Some(0));
    assert_eq!(state.desired_webviews().len(), 1);
    state.tabs.open_url("http://a.com".into());
    state.set_addr_focused(true);
    assert!(state.addr_focused());
    state.set_addr_focused(false);
    assert!(!state.addr_focused());
    // 新开的 a.com 成为激活 tab,空标签仍在列表里
    assert_eq!(state.active_webview_id(), Some(1));
    assert_eq!(state.desired_webviews().len(), 2);
}

fn collect() -> (impl Fn(Message) + Send + 'static, mpsc::Receiver<Message>) {
    let (tx, rx) = mpsc::channel();
    (
        move |m| {
            let _ = tx.send(m);
        },
        rx,
    )
}

fn fake_service(fake: &FakeBookmark) -> std::sync::Arc<dyn dozer_services::BookmarkService> {
    fake.clone().into_service()
}

#[test]
fn refresh_emits_the_list_or_the_service_error() {
    let rt = runtime();
    let fake = FakeBookmark::default().with_items(vec![bm(
        1,
        BookmarkScope::Global,
        None,
        "https://a.com",
    )]);
    let (emit, rx) = collect();
    request_bookmarks_refresh(
        &Default::default(),
        Some(1),
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::BookmarksLoaded(Some(1), Ok(v)) => assert_eq!(v.len(), 1),
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(fake.calls(), vec!["list_bookmarks(Some(1))".to_string()]);

    let fake =
        FakeBookmark::default().with_list_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
    let (emit, rx) = collect();
    request_bookmarks_refresh(
        &Default::default(),
        None,
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::BookmarksLoaded(None, Err(e)) => {
            assert_eq!(
                (e.kind, e.message.as_str()),
                (ServiceErrorKind::Rejected, "daemon 错误: boom")
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn a_failed_load_keeps_the_cache_and_unavailable_stays_silent() {
    let rt = runtime();
    let cached = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
    let mut state = State {
        bookmarks: cached.clone(),
        ..State::default()
    };
    let (emit, _rx) = collect();
    update(
        &mut state,
        Message::BookmarksLoaded(
            Some(1),
            Err(dozer_services::ServiceError::new(
                ServiceErrorKind::Unavailable,
                "daemon 断开",
            )),
        ),
        Some(1),
        &fake_service(&FakeBookmark::default()),
        rt.handle(),
        emit,
    );
    assert_eq!(state.bookmarks, cached, "读失败不得清空缓存");
    assert!(
        state.error.is_none(),
        "Unavailable 由顶栏徽标表达,不写内联错误"
    );
}

#[test]
fn a_rejected_load_keeps_the_cache_and_sets_the_inline_error() {
    let rt = runtime();
    let cached = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
    let mut state = State {
        bookmarks: cached.clone(),
        ..State::default()
    };
    let (emit, _rx) = collect();
    update(
        &mut state,
        Message::BookmarksLoaded(
            Some(1),
            Err(dozer_services::ServiceError::new(
                ServiceErrorKind::Rejected,
                "daemon 错误: boom",
            )),
        ),
        Some(1),
        &fake_service(&FakeBookmark::default()),
        rt.handle(),
        emit,
    );
    assert_eq!(state.bookmarks, cached);
    assert_eq!(
        state.error,
        Some(crate::i18n::panel_keyed_args(
            WEB_OWNER,
            "error.bookmarks_load_failed",
            "Failed to load bookmarks: {e}",
            &[("e", "daemon 错误: boom")],
        ))
    );
}

#[test]
fn add_passes_scope_target_project_url_and_title_to_the_service() {
    let rt = runtime();
    let fake = FakeBookmark::default();
    let mut state = State::default();
    let id = state.tabs.open_url("http://a.com".into());
    let _ = id;
    let (url, title) = {
        let t = &state.tabs.tabs()[state.tabs.active_idx()];
        (t.url.clone(), t.title.clone())
    };
    let (emit, rx) = collect();
    update(
        &mut state,
        Message::BookmarkAdd(BookmarkScope::Project),
        Some(7),
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::BookmarksMutated(Some(7), Ok(()))
    ));
    assert_eq!(
        fake.calls(),
        vec![format!("add_bookmark(Project,Some(7),{url},{title})")]
    );

    let fake = FakeBookmark::default();
    let (emit, rx) = collect();
    update(
        &mut state,
        Message::BookmarkAdd(BookmarkScope::Global),
        Some(7),
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::BookmarksMutated(Some(7), Ok(()))
    ));
    assert_eq!(
        fake.calls(),
        vec![format!("add_bookmark(Global,None,{url},{title})")],
        "Global 收藏的目标项目恒为 None"
    );
}

#[test]
fn add_failure_text_is_the_service_message_unchanged() {
    let rt = runtime();
    let fake =
        FakeBookmark::default().with_write_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
    let mut state = State::default();
    state.tabs.open_url("http://a.com".into());
    let (emit, rx) = collect();
    update(
        &mut state,
        Message::BookmarkAdd(BookmarkScope::Global),
        Some(1),
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::BookmarksMutated(Some(1), Err(text)) => assert_eq!(text, "daemon 错误: boom"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn remove_calls_the_service_and_mutated_triggers_one_list() {
    let rt = runtime();
    let fake = FakeBookmark::default();
    let mut state = State {
        bookmarks: vec![bm(3, BookmarkScope::Global, None, "https://a.com")],
        ..State::default()
    };
    let (emit, rx) = collect();
    update(
        &mut state,
        Message::BookmarkRemove(3),
        Some(1),
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::BookmarksMutated(Some(1), Ok(()))
    ));
    assert_eq!(fake.calls(), vec!["remove_bookmark(3)".to_string()]);

    let (emit, rx) = collect();
    update(
        &mut state,
        Message::BookmarksMutated(Some(1), Ok(())),
        Some(1),
        &fake_service(&fake),
        rt.handle(),
        emit,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::BookmarksLoaded(Some(1), Ok(_))
    ));
    assert_eq!(
        fake.calls().last().map(String::as_str),
        Some("list_bookmarks(Some(1))")
    );
}

#[test]
fn stale_bookmark_reads_and_reopened_panel_mutations_are_ignored() {
    let rt = runtime();
    let service = fake_service(&FakeBookmark::default());
    let mut state = State::default();
    let old = state.requests.begin(Some(1), "bookmarks");
    let new = state.requests.begin(Some(1), "bookmarks");
    let apply = |state: &mut State, msg| update(state, msg, Some(1), &service, rt.handle(), |_| {});
    apply(
        &mut state,
        Message::AsyncResult(
            new.clone(),
            Box::new(Message::BookmarksLoaded(
                Some(1),
                Err(dozer_services::ServiceError::new(
                    ServiceErrorKind::Rejected,
                    "current",
                )),
            )),
        ),
    );
    apply(
        &mut state,
        Message::AsyncResult(
            old.clone(),
            Box::new(Message::BookmarksLoaded(
                Some(1),
                Err(dozer_services::ServiceError::new(
                    ServiceErrorKind::Rejected,
                    "old",
                )),
            )),
        ),
    );
    assert_eq!(
        state
            .error
            .as_ref()
            .unwrap()
            .args
            .get("e")
            .map(String::as_str),
        Some("current")
    );
    apply(
        &mut state,
        Message::AsyncResult(
            new,
            Box::new(Message::BookmarksLoaded(
                Some(1),
                Ok(vec![bm(2, BookmarkScope::Global, None, "https://new")]),
            )),
        ),
    );
    apply(
        &mut state,
        Message::AsyncResult(old, Box::new(Message::BookmarksLoaded(Some(1), Ok(vec![])))),
    );
    assert_eq!(state.bookmarks[0].id, 2);
    let mut home = State::default();
    let home_request = home.requests.begin(None, "bookmarks");
    update(
        &mut home,
        Message::AsyncResult(
            home_request,
            Box::new(Message::BookmarksLoaded(
                None,
                Ok(vec![bm(3, BookmarkScope::Global, None, "https://home")]),
            )),
        ),
        None,
        &service,
        rt.handle(),
        |_| {},
    );
    assert_eq!(home.bookmarks[0].id, 3);
    let mut reopened = State::default();
    apply(
        &mut reopened,
        Message::AsyncResult(
            state.requests.scope(Some(1)),
            Box::new(Message::BookmarksMutated(
                Some(1),
                Err("old mutation".into()),
            )),
        ),
    );
    assert!(reopened.error.is_none());
    let global = state.requests.begin(None, "bookmarks");
    assert_eq!(dozer_panel_host::project_of(global.scope()), None);
    apply(
        &mut reopened,
        Message::AsyncResult(global, Box::new(Message::BookmarksLoaded(None, Ok(vec![])))),
    );
    assert!(reopened.error.is_none());
}

#[tokio::test]
async fn async_result_with_foreign_scope_is_dropped() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    let cached = vec![bm(1, BookmarkScope::Global, None, "https://keep")];
    state.bookmarks = cached.clone();
    // 令牌属于项目 2,而当前实例是项目 1:结果必须被丢弃。
    let foreign = state.requests.begin(Some(2), "bookmarks");
    update(
        &mut state,
        Message::AsyncResult(
            foreign,
            Box::new(Message::BookmarksLoaded(
                Some(2),
                Ok(vec![bm(9, BookmarkScope::Project, Some(2), "https://x")]),
            )),
        ),
        Some(1),
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(state.bookmarks, cached);
}

#[test]
fn global_instance_never_shows_project_bookmark_status() {
    let list = vec![bm(5, BookmarkScope::Project, Some(3), "https://a.com")];
    let status = bookmark_status(&list, "https://a.com", None);
    assert_eq!(status.project, None);
    assert_eq!(status.global, None);
}

// 已知真实缺陷(搬迁前就存在):`CloseTab` 不重排/清理按下标索引的 hover / tooltip_starts,
// 关中间页签后后面的页签继承悬停状态。按任务要求不在搬迁里修复,单独成提交后去掉 ignore。
#[ignore = "known bug: CloseTab leaves index-keyed hover/tooltip state stale"]
#[tokio::test]
async fn closing_middle_tab_clears_hover_and_tooltip_state() {
    let mut state = State::default();
    let handle = tokio::runtime::Handle::current();
    let service = service_for_test();
    state.tabs.open_url("http://b.com".into());
    state.tabs.open_url("http://c.com".into());
    assert_eq!(state.tab_count(), 3);
    update(
        &mut state,
        Message::Hover(1, false, true),
        None,
        &service,
        &handle,
        |_| {},
    );
    // 把悬停计时起点回拨到 3s 前,模拟"已悬停满 2s"。
    state.tooltip_starts.insert(
        1,
        std::time::Instant::now() - std::time::Duration::from_secs(3),
    );
    for _ in 0..8 {
        state.advance_hover_anims();
    }
    assert!(state.hover_progress(1, false) > 0.9);
    assert!(state.hover_tooltip_ready(1));

    update(
        &mut state,
        Message::CloseTab(1),
        None,
        &service,
        &handle,
        |_| {},
    );
    assert_eq!(state.tab_count(), 2);
    assert_eq!(
        state.hover_progress(1, false),
        0.0,
        "新的第 2 个页签不该继承被关页签的悬停进度"
    );
    assert!(
        !state.hover_tooltip_ready(1),
        "新的第 2 个页签不该继承被关页签的 tooltip 计时"
    );
}
