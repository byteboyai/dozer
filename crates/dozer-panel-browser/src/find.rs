//! 页内查找条:状态、宿主要执行的请求、查找条的高度、焦点捕获。视图在 `view.rs`,消息在 `update.rs`。
//!
//! 库不碰 wry:用户操作变成 [`FindRequest`] 放进 `State` 的发件箱,宿主在每次 `update` 之后用
//! [`State::take_find_requests`] 取走,对**激活页签**的 webview 执行(`WKWebView.findString:` 等),
//! 查找结束再用 `Message::FindResult` 把"有没有命中"送回来。

use crate::State;
use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};

/// 查找输入框的稳定 id:宿主用它对输入框做 `focus` / `select_all`(`State::take_find_focus_pending`),
/// [`CaptureFindFocus`] 也靠它在 widget 树里认出这一个。
pub fn find_field_id() -> Id {
    Id::new("browser-find-box")
}

static FIND_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)查找输入框上一帧是否持有 iced 真实焦点——宿主渲染循环每帧跑完
/// [`CaptureFindFocus`] 后立刻调用,把结果塞进当前浏览器状态(`State::set_find_focused`),
/// 键盘路由再据此决定"放行给标准 iced 管线"还是"继续往 ⌘ 组合键 / 终端字节转发走"。
/// 消费式复位的理由同 `addr_focus::take_addr_focused`:查找条不可见的帧不能把上一次的
/// `true` 卡死在里面,否则终端会永久收不到键盘。
pub fn take_find_focused() -> bool {
    std::mem::replace(&mut *FIND_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 [`find_field_id`] 命中的 `text_input` 当前是否持有 iced
/// 焦点写进 `FIND_FOCUSED`。`traverse` **必须**调用传入的 `operate` 闭包才会继续递归子节点——
/// 查找条嵌在 `row!`/`container!` 里,空 `traverse` 会让 `Row`/`Column` 的 `operate()` 直接跳过
/// 子节点、`focusable()` 永远触达不到(同 `addr_focus::CaptureAddrFocus` 修过的 Critical bug)。
pub struct CaptureFindFocus;
impl Operation<()> for CaptureFindFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&find_field_id()) {
            *FIND_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 宿主要对激活页签做的查找动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindRequest {
    /// 查 `query`;`backwards` 往前找。同一个词再来一次就是"下一个 / 上一个"(从当前命中之后继续,回绕)。
    Search { query: String, backwards: bool },
    /// 清掉页面里的命中选区(关掉查找条 / 清空查找词)。
    Clear,
}

/// 查找条的状态;`State.find` 为 `None` 表示查找条没打开。
#[derive(Debug, Clone, Default)]
pub(crate) struct FindState {
    pub(crate) query: String,
    /// 最近一次查找有没有命中;还没查过 / 查找词变了 / 换了页签是 `None`。
    pub(crate) matched: Option<bool>,
    /// 刚打开,请宿主把焦点给查找输入框(一次性,消费即复位)。
    pub(crate) focus_pending: bool,
    /// 查找输入框是否持有 iced 真实焦点;宿主每帧用 [`CaptureFindFocus`] 查到后经
    /// [`State::set_find_focused`] 写进来(见 `State::find_focused`)。
    pub(crate) focused: bool,
}

/// 查找条本体(输入框那一行)的高度:与地址栏的输入框同高。视图和几何**共用这一个值**。
pub fn find_bar_height_px() -> f32 {
    byteui::theme::geometry::tab_button_size() + 4.0
}

/// 查找行下方的留白(与地址栏行的下内边距同值)。
const FIND_BAR_BOTTOM_PAD: f32 = 8.0;

/// 查找条打开时,网页要比不开多让出的高度:查找行 + 下留白 + 列布局里它与上一行之间的 `gap`
/// (`BrowserRegion::gap`,宿主的区域样式给)。宿主的网页几何在 `find_open()` 时加这一项。
pub fn find_bar_extra_px(region_gap: f32) -> f32 {
    find_bar_height_px() + FIND_BAR_BOTTOM_PAD + region_gap.max(0.0)
}

/// 视图用:查找行下留白。
pub(crate) fn find_bar_bottom_pad() -> f32 {
    FIND_BAR_BOTTOM_PAD
}

impl State {
    /// 查找条是否打开(宿主的网页几何、键盘路由据此)。
    pub fn find_open(&self) -> bool {
        self.find.is_some()
    }

    /// 当前查找词(没打开时为空)。
    pub fn find_query(&self) -> &str {
        self.find.as_ref().map_or("", |f| f.query.as_str())
    }

    /// 最近一次查找有没有命中。
    pub fn find_matched(&self) -> Option<bool> {
        self.find.as_ref().and_then(|f| f.matched)
    }

    /// 查找输入框是否持有 iced 真实焦点(宿主的键盘路由据此:真 → 放行给标准 iced 管线交
    /// `text_input` 自己处理,假 → 继续走 ⌘ 组合键 / 终端字节转发)。查找条没打开恒为 `false`——
    /// 少了这一项,用户在查找条里打的每个字都会被顺带转发进当前终端(同地址栏 `addr_focused`)。
    pub fn find_focused(&self) -> bool {
        self.find.as_ref().is_some_and(|f| f.focused)
    }

    /// 每帧渲染循环调用:把 [`CaptureFindFocus`] / [`take_find_focused`] 问到的真实焦点态写进来。
    /// 查找条没打开时什么都不做(没有输入框可聚焦)。
    pub fn set_find_focused(&mut self, focused: bool) {
        if let Some(find) = self.find.as_mut() {
            find.focused = focused;
        }
    }

    /// 关掉查找条(Esc 或关闭按钮):顺带请宿主清掉页面里的命中选区。
    pub fn close_find_bar(&mut self) {
        self.find_close_bar();
    }

    /// 取走宿主要执行的查找请求(消费即清空)。
    pub fn take_find_requests(&mut self) -> Vec<FindRequest> {
        std::mem::take(&mut self.find_requests)
    }

    /// 取走"请把焦点给查找输入框"的一次性标志。
    pub fn take_find_focus_pending(&mut self) -> bool {
        self.find
            .as_mut()
            .is_some_and(|f| std::mem::take(&mut f.focus_pending))
    }

    /// 宿主的 ⌘F(应用主菜单里的 Find…):打开查找条并请输入框拿焦点;已经打开就只是再次请求聚焦。
    pub fn open_find_bar(&mut self) {
        self.find_open_bar();
    }

    pub(crate) fn find_open_bar(&mut self) {
        match self.find.as_mut() {
            Some(find) => find.focus_pending = true,
            None => {
                self.find = Some(FindState {
                    focus_pending: true,
                    ..FindState::default()
                });
            }
        }
    }

    pub(crate) fn find_close_bar(&mut self) {
        if self.find.take().is_some() {
            self.find_requests.push(FindRequest::Clear);
        }
    }

    pub(crate) fn find_set_query(&mut self, query: String) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        find.matched = None;
        let request = if query.is_empty() {
            FindRequest::Clear
        } else {
            FindRequest::Search {
                query: query.clone(),
                backwards: false,
            }
        };
        find.query = query;
        self.find_requests.push(request);
    }

    pub(crate) fn find_step(&mut self, backwards: bool) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        if find.query.is_empty() {
            return;
        }
        self.find_requests.push(FindRequest::Search {
            query: find.query.clone(),
            backwards,
        });
    }

    /// 宿主送回的查找结果。只认和当前查找词一致的(查找是异步的,慢的旧结果不能覆盖新词的状态)。
    pub(crate) fn find_apply_result(&mut self, query: &str, matched: bool) {
        if let Some(find) = self.find.as_mut().filter(|f| f.query == query) {
            find.matched = Some(matched);
        }
    }

    /// 换页签 / 页签关闭后,上一个页面的命中状态不再适用。
    pub(crate) fn find_forget_result(&mut self) {
        if let Some(find) = self.find.as_mut() {
            find.matched = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeFocus(bool);
    impl Focusable for FakeFocus {
        fn is_focused(&self) -> bool {
            self.0
        }
        fn focus(&mut self) {}
        fn unfocus(&mut self) {}
    }

    fn open() -> State {
        let mut s = State::default();
        s.find_open_bar();
        s
    }

    #[test]
    fn opening_the_bar_asks_the_host_to_focus_the_input_once() {
        let mut s = State::default();
        assert!(!s.find_open());
        s.find_open_bar();
        assert!(s.find_open());
        assert!(s.take_find_focus_pending());
        assert!(!s.take_find_focus_pending(), "一次性");
        // 已经打开时再按 ⌘F:再次请求聚焦(用户想回到输入框),查找词保留。
        s.find_set_query("abc".into());
        s.find_open_bar();
        assert_eq!(s.find_query(), "abc");
        assert!(s.take_find_focus_pending());
    }

    #[test]
    fn typing_searches_forward_and_clearing_clears_the_highlight() {
        let mut s = open();
        s.find_set_query("ab".into());
        assert_eq!(
            s.take_find_requests(),
            vec![FindRequest::Search {
                query: "ab".into(),
                backwards: false
            }]
        );
        assert!(s.take_find_requests().is_empty(), "取走即清空");
        s.find_set_query(String::new());
        assert_eq!(s.take_find_requests(), vec![FindRequest::Clear]);
        assert_eq!(s.find_matched(), None);
    }

    #[test]
    fn next_and_previous_repeat_the_same_query_in_each_direction() {
        let mut s = open();
        s.find_step(false);
        assert!(s.take_find_requests().is_empty(), "没有查找词:不查");
        s.find_set_query("x".into());
        let _ = s.take_find_requests();
        s.find_step(false);
        s.find_step(true);
        assert_eq!(
            s.take_find_requests(),
            vec![
                FindRequest::Search {
                    query: "x".into(),
                    backwards: false
                },
                FindRequest::Search {
                    query: "x".into(),
                    backwards: true
                }
            ]
        );
    }

    /// 查找是异步的:慢的旧结果(查找词已经变了)不能覆盖新词的状态。
    #[test]
    fn a_stale_result_for_an_older_query_is_ignored() {
        let mut s = open();
        s.find_set_query("ab".into());
        s.find_set_query("abc".into());
        s.find_apply_result("ab", true);
        assert_eq!(s.find_matched(), None);
        s.find_apply_result("abc", false);
        assert_eq!(s.find_matched(), Some(false));
        // 查找条关了以后迟到的结果:什么都不改。
        s.find_close_bar();
        s.find_apply_result("abc", true);
        assert_eq!(s.find_matched(), None);
    }

    #[test]
    fn closing_clears_the_highlight_once_and_reopening_starts_empty() {
        let mut s = open();
        s.find_set_query("x".into());
        let _ = s.take_find_requests();
        s.find_close_bar();
        assert_eq!(s.take_find_requests(), vec![FindRequest::Clear]);
        s.find_close_bar();
        assert!(s.take_find_requests().is_empty(), "已经关了:不重复清");
        s.find_open_bar();
        assert_eq!(s.find_query(), "");
    }

    #[test]
    fn switching_pages_forgets_the_previous_pages_result_but_keeps_the_query() {
        let mut s = open();
        s.find_set_query("x".into());
        s.find_apply_result("x", true);
        s.find_forget_result();
        assert_eq!(s.find_matched(), None);
        assert_eq!(s.find_query(), "x");
    }

    /// 网页几何和视图共用 `find_bar_height_px`:额外高度 = 查找行 + 下留白 + gap,gap 为负当 0。
    #[test]
    fn the_extra_height_is_the_bar_plus_its_padding_and_the_gap() {
        let base = find_bar_height_px() + find_bar_bottom_pad();
        assert_eq!(find_bar_extra_px(0.0), base);
        assert_eq!(find_bar_extra_px(6.0), base + 6.0);
        assert_eq!(find_bar_extra_px(-5.0), base);
        assert!(find_bar_height_px() > 0.0);
    }

    /// 焦点态跟着查找条一起生灭:没打开时宿主写进来的 `true` 不认(没有输入框可聚焦),
    /// 关掉查找条即复位。宿主的键盘路由读 `find_focused` 决定"放行给 iced"还是
    /// "继续往 ⌘ 组合键 / 终端字节转发走"——卡死在 `true` 就等于终端永久收不到键盘。
    #[test]
    fn the_focus_flag_lives_and_dies_with_the_bar() {
        let mut s = State::default();
        assert!(!s.find_focused());
        s.set_find_focused(true);
        assert!(!s.find_focused(), "查找条没打开:不认焦点");
        s.find_open_bar();
        s.set_find_focused(true);
        assert!(s.find_focused());
        s.find_close_bar();
        assert!(!s.find_focused(), "关掉即复位");
    }

    /// `CaptureFindFocus` 只认 `find_field_id()` 那一个输入框,`traverse` 必须把 `operate` 调回自己
    /// (否则嵌在 `row!`/`container!` 里的查找框永远触达不到),结果由 `take_find_focused()`
    /// 消费式读走。
    #[test]
    fn capture_only_records_the_find_input_and_take_consumes_it() {
        let mut op = CaptureFindFocus;
        op.traverse(&mut |o| {
            o.focusable(
                Some(&find_field_id()),
                Rectangle::default(),
                &mut FakeFocus(true),
            )
        });
        assert!(take_find_focused(), "traverse 没递归就不会记录焦点");
        assert!(!take_find_focused(), "消费式:读走即复位");
        op.focusable(
            Some(&Id::new("browser-addr-box")),
            Rectangle::default(),
            &mut FakeFocus(true),
        );
        assert!(!take_find_focused(), "别的输入框不算");
        op.focusable(None, Rectangle::default(), &mut FakeFocus(true));
        assert!(!take_find_focused(), "没有 id 不算");
        op.focusable(
            Some(&find_field_id()),
            Rectangle::default(),
            &mut FakeFocus(false),
        );
        assert!(!take_find_focused());
    }
}
