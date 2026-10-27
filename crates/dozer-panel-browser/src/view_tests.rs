//! 视图构建测试:用一个测试用 `FakeDisplay` 实现 `BrowserDisplay`,断言各布局组合都能构建,
//! 且收藏夹侧栏收起时根本不向宿主要分隔条。

use crate::adapter::{BrowserDisplay, BrowserRegion, TabView, chrome_top_px};
use crate::view::view;
use crate::{State, Tabs};
use dozer_model::{BookmarkInfo, BookmarkScope};
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{Theme, button, text};
use std::cell::Cell;

type Renderer = iced_renderer::Renderer;

#[derive(Default)]
struct FakeDisplay {
    tab_calls: Cell<usize>,
    tab_divider_calls: Cell<usize>,
    bookmarks_divider_calls: Cell<usize>,
}

impl BrowserDisplay for FakeDisplay {
    fn region(&self) -> BrowserRegion {
        BrowserRegion {
            background: None,
            padding: Padding::new(4.0),
            gap: 2.0,
        }
    }

    fn locale(&self) -> &str {
        "en"
    }

    fn line_height_factor(&self) -> f32 {
        1.2
    }

    fn tab_divider<'a, M: 'a>(&self) -> Element<'a, M, Theme, Renderer> {
        self.tab_divider_calls.set(self.tab_divider_calls.get() + 1);
        iced_widget::Space::new().into()
    }

    fn tab<'a, M: Clone + 'a>(
        &self,
        v: TabView,
        on_select: M,
        _on_close: M,
        _title_hover: impl Fn(bool) -> M + 'a,
        _close_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, Theme, Renderer> {
        self.tab_calls.set(self.tab_calls.get() + 1);
        button(text(v.title)).on_press(on_select).into()
    }

    fn bookmarks_divider<'a, M: Clone + 'a>(&self, on_drag: M) -> Element<'a, M, Theme, Renderer> {
        self.bookmarks_divider_calls
            .set(self.bookmarks_divider_calls.get() + 1);
        button(text("|")).on_press(on_drag).into()
    }
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

fn state_with(bookmarks_open: bool) -> State {
    let mut s = State::with_initial_url("https://a.com");
    s.tabs.open_url("https://b.com".into());
    s.bookmarks = vec![
        bm(1, BookmarkScope::Global, None, "https://a.com"),
        bm(2, BookmarkScope::Project, Some(7), "https://b.com"),
    ];
    s.error = Some(dozer_model::LocalizedText::keyed(
        crate::WEB_OWNER,
        "error.local_path",
        "Opening local files is not supported in the browser",
    ));
    s.bookmarks_open = bookmarks_open;
    s
}

/// 评审关注点 3:视图里页签栏+地址栏让给 webview 的高度只有一个源头。
#[test]
fn chrome_height_has_one_source() {
    assert_eq!(
        chrome_top_px(),
        byteui::theme::geometry::browser_chrome_top_px()
    );
}

#[test]
fn view_builds_for_every_bookmarks_and_mirror_combination() {
    for bookmarks_open in [false, true] {
        for mirror in [false, true] {
            for project_id in [None, Some(7)] {
                let display = FakeDisplay::default();
                let state = state_with(bookmarks_open);
                let _el: Element<'_, crate::Message, Theme, Renderer> = view(
                    &display,
                    &state,
                    project_id,
                    0.7,
                    Length::Fill,
                    Border::default(),
                    mirror,
                );
                assert_eq!(display.tab_calls.get(), 2, "每个页签向宿主要一次页签控件");
                assert_eq!(display.tab_divider_calls.get(), 1);
                assert_eq!(
                    display.bookmarks_divider_calls.get(),
                    usize::from(bookmarks_open),
                    "收藏夹侧栏收起时不得向宿主要分隔条 (open={bookmarks_open}, mirror={mirror})"
                );
            }
        }
    }
}

#[test]
fn view_builds_with_no_tabs() {
    let display = FakeDisplay::default();
    let state = State {
        tabs: Tabs::default(),
        ..State::default()
    };
    let _el = view(
        &display,
        &state,
        None,
        0.7,
        Length::Fill,
        Border::default(),
        false,
    );
    assert_eq!(display.tab_calls.get(), 0);
    assert_eq!(display.bookmarks_divider_calls.get(), 0);
}
