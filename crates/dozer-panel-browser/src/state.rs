//! 浏览器面板状态:页签、收藏夹缓存、tab hover 动画 / tooltip 计时。

use crate::Tabs;
use crate::adapter::BrowserWebviewSpec;
use crate::update::NavAction;
use dozer_model::BookmarkInfo;
use std::collections::HashMap;

/// 单个 tab 标题/关闭按钮的 hover 动画状态机,与顶栏 `HoverAnim` 同款
/// (每拍残余 50%,约 80ms 收敛,snap 0.01):浏览器面板独立 `State`,自带
/// 进度机,借全局 HOVER_ANIM 定时 redraw 推进(见 `State::advance_hover_anims`)。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TabHover {
    progress: f32,
    target: f32,
}

impl TabHover {
    pub(crate) fn set(&mut self, hovered: bool) {
        self.target = if hovered { 1.0 } else { 0.0 };
    }
    fn advance(&mut self) {
        let next = self.progress + (self.target - self.progress) * 0.5;
        self.progress = if (next - self.target).abs() < 0.01 {
            self.target
        } else {
            next
        };
    }
    fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.001
    }
}

/// 浏览器面板的全部状态。挂在每个 `Workspace` 上(不像 Git Log 挂在
/// `App` 上)——项目切换靠 `Workspace` 自身生命周期天然隔离,不需要
/// 手动同步/清空逻辑。
pub struct State {
    pub requests: dozer_panel_host::RequestTrackers,
    pub(crate) tabs: Tabs,
    pub(crate) error: Option<dozer_model::LocalizedText>,
    pub(crate) bookmarks: Vec<BookmarkInfo>,
    pub(crate) bookmarks_open: bool,
    /// tab 标题/关闭按钮的 hover 进度,键 `(tab 序号, 是否关闭按钮)`。
    pub(crate) hover: HashMap<(usize, bool), TabHover>,
    /// 页签标题 tooltip 的悬停计时起点:键为 tab 序号(关闭按钮不计,只认
    /// 页签整体悬停)。进入页签记 `Instant::now()`,离开即清除;悬停满 2s
    /// 后视图层据此弹标题全称 tooltip(见 `hover_tooltip_ready`)。与顶栏/
    /// 终端页签的计时分开存(`App::hover_tooltip_starts`),因为浏览器面板
    /// 走自己这套 hover 状态机(`hover` 而非全局 `HoverId`)。
    pub(crate) tooltip_starts: HashMap<usize, std::time::Instant>,
    /// 地址栏刚获得焦点时置位,请求内核在下一帧对地址栏做一次"全选"(
    /// `main.rs` 的 draw 循环用 `interface.operate` 跑 `text_input::
    /// select_all`)。一次性位,消费即复位(见 `take_addr_select_all_pending`)。
    pub(crate) addr_select_all_pending: bool,
    /// 页内查找条;`None` = 没打开(见 `find.rs`)。
    pub(crate) find: Option<crate::find::FindState>,
    /// 宿主要对激活页签执行的查找动作(`take_find_requests` 取走)。
    pub(crate) find_requests: Vec<crate::find::FindRequest>,
}

impl Default for State {
    /// 浏览器面板默认状态:开一个 `about:blank` 标签页,而不是空标签栏——
    /// 面板首次呈现就有一块可看的内容,地址栏也提示"输入网址"开始浏览。
    fn default() -> Self {
        Self::with_initial_url("about:blank")
    }
}

impl State {
    /// 构造一个带初始 tab 的浏览器状态——用于首页全局浏览器默认打开某个
    /// 站点(见 `app::App::home_browser` 初始化)。默认(`State::default`)
    /// 本就带一个 `about:blank` 标签页,这里把它换成给定的真实 URL,保持
    /// "单一初始标签页"语义,不带多余空标签。
    pub fn with_initial_url(url: &str) -> Self {
        let mut s = State {
            requests: Default::default(),
            tabs: Tabs::default(),
            error: None,
            bookmarks: Vec::new(),
            bookmarks_open: false,
            hover: HashMap::new(),
            tooltip_starts: HashMap::new(),
            addr_select_all_pending: false,
            find: None,
            find_requests: Vec::new(),
        };
        s.tabs.open_url(url.to_string());
        s
    }

    /// 地址栏是否持有 iced 真实焦点(内核 `App::browser_addr_focused` 键盘
    /// 路由用)。
    pub fn addr_focused(&self) -> bool {
        self.tabs.addr_focused()
    }

    /// 地址栏当前草稿(只读)。宿主用它把输入框内容与"右键菜单 / 粘贴"等原生交互对账。
    pub fn addr_text(&self) -> &str {
        self.tabs.addr_buffer()
    }

    /// 收藏夹侧栏当前是否展开——`app.rs::App::shell_state()` 读这个填
    /// `ShellState::browser_bookmarks_open`,几何计算据此决定网页 webview
    /// 是否要让出侧栏宽度。
    pub fn bookmarks_open(&self) -> bool {
        self.bookmarks_open
    }

    /// 每帧渲染循环调用:把 `CaptureAddrFocus` 问到的真实焦点态写进来;
    /// 焦点从假变真时顺带清掉上一次提交失败留下的错误提示(同旧版
    /// `AddrClick` 里的 `state.error = None`)。
    pub fn set_addr_focused(&mut self, focused: bool) {
        if !self.tabs.addr_focused() && focused {
            self.error = None;
            // 焦点刚获得:请求内核下一帧对地址栏做一次全选(见
            // `take_addr_select_all_pending`),让用户单击即选中整条网址。
            self.addr_select_all_pending = true;
        }
        self.tabs.set_addr_focused(focused);
    }

    /// 取走"地址栏需全选"的一次性标记(消费即复位),内核 draw 循环据此
    /// 跑 `text_input::select_all`(`main.rs`)。
    pub fn take_addr_select_all_pending(&mut self) -> bool {
        std::mem::take(&mut self.addr_select_all_pending)
    }

    /// 当前激活 tab 的 webview id(内核 `App::active_browser_webview_id`
    /// 焦点路由用)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.active_webview_id()
    }

    /// 页签总数(内核 `App` 拖拽换位的越界保护用)。
    pub fn tab_count(&self) -> usize {
        self.tabs.tabs().len()
    }

    /// 当前激活页签下标(内核 `App` 拖拽换位的源记录用)。
    pub fn active_tab_idx(&self) -> usize {
        self.tabs.active_idx()
    }

    /// webview 期望清单(内核 `App::browser_desired` 用,供 main.rs 同步
    /// webview 池)。
    pub fn desired_webviews(&self) -> Vec<BrowserWebviewSpec> {
        self.tabs.desired_webviews()
    }

    /// 推进所有 tab 的 hover 动画一拍(每拍残余 50%,约 80ms 收敛),与顶栏
    /// `App::advance_hover_anims` 同款。由内核在全局 HOVER_ANIM 定时里调用,
    /// 浏览器面板靠它复用顶栏那套自驱 redraw(见 `any_hover_active`)。
    pub fn advance_hover_anims(&mut self) {
        for h in self.hover.values_mut() {
            h.advance();
        }
    }

    /// 是否还有 tab hover 动画在进行中(进度未到目标)——内核
    /// `App::any_hover_anim_active` 据此决定是否继续排下一拍定时唤醒。
    pub fn any_hover_active(&self) -> bool {
        self.hover.values().any(TabHover::active)
    }

    /// 维护某页签的 tooltip 悬停计时:进入记起点、离开清除(满 2s 由
    /// `hover_tooltip_ready` 判断)。与 `hover` 动画进度同源触发,但计时是
    /// 独立的一份(见 `tooltip_starts` 字段注释)。
    pub(crate) fn set_tab_tooltip(&mut self, idx: usize, hovered: bool) {
        if hovered {
            self.tooltip_starts.insert(idx, std::time::Instant::now());
        } else {
            self.tooltip_starts.remove(&idx);
        }
    }

    /// 某页签悬停是否已持续满 `HOVER_TOOLTIP_DELAY`(2s):满则视图层弹标题
    /// 全称 tooltip(见 `panel_tab` 的 `show_tooltip` 参数)。
    pub(crate) fn hover_tooltip_ready(&self, idx: usize) -> bool {
        self.tooltip_starts
            .get(&idx)
            .is_some_and(|start| start.elapsed() >= crate::adapter::TAB_TOOLTIP_DELAY)
    }

    /// 距下一个 tooltip 计时满 2s 的最短剩余时间——内核 `App::next_tooltip_wake`
    /// 据此排下次唤醒,做到"恰好满 2s 才重绘"。
    pub fn next_tooltip_wake(&self) -> Option<std::time::Duration> {
        self.tooltip_starts
            .values()
            .filter_map(|start| crate::adapter::TAB_TOOLTIP_DELAY.checked_sub(start.elapsed()))
            .min()
    }

    /// 取某 tab 标题(idx, is_close=false)或关闭按钮(idx, is_close=true)的
    /// 当前 hover 进度(0..=1),给 `view` 做颜色插值。
    pub(crate) fn hover_progress(&self, idx: usize, is_close: bool) -> f32 {
        self.hover
            .get(&(idx, is_close))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 星标按钮(当前 tab 收藏/取消收藏)的 hover 进度。用哨兵键
    /// `(STAR_HOVER_KEY, false)` 区分于真实 tab(真实 tab 序号不可能
    /// 等于 `usize::MAX`)。
    pub(crate) fn star_hover(&self) -> f32 {
        self.hover
            .get(&(STAR_HOVER_KEY, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 拖拽换位:把 `from` 处的 tab 移到 `to`,并重排 index-keyed 的 hover 键
    /// (键 `(idx, is_close)`)。哨兵键 `STAR_HOVER_KEY` 不受影响。
    pub fn reorder_tab(&mut self, from: usize, to: usize) {
        self.tabs.reorder(from, to);
        // 源与目标之间所有条目顺移一位,把它们的 hover 键跟着挪。
        let lo = from.min(to);
        let hi = from.max(to);
        if lo != hi {
            let mut remap = Vec::new();
            for i in lo..=hi {
                for is_close in [false, true] {
                    if let Some(h) = self.hover.remove(&(i, is_close)) {
                        // 换位后:与 from 同侧锚定更直观——若 i==from 移到的
                        // 是新位 to;否则若从右往左(hi<原次序)各 i 左移,反
                        // 之右移。这里直接让"所在槽位的 hover"跟着槽位走最省心:
                        let new_i = if i == from {
                            to
                        } else if from < to {
                            i - 1
                        } else {
                            i + 1
                        };
                        remap.push((new_i, is_close, h));
                    }
                }
            }
            for (i, is_close, h) in remap {
                self.hover.insert((i, is_close), h);
            }
            // tooltip 计时同样按页签序号索引,跟着槽位一起挪。
            let mut starts = Vec::new();
            for i in lo..=hi {
                if let Some(t) = self.tooltip_starts.remove(&i) {
                    let new_i = if i == from {
                        to
                    } else if from < to {
                        i - 1
                    } else {
                        i + 1
                    };
                    starts.push((new_i, t));
                }
            }
            self.tooltip_starts.extend(starts);
        }
    }

    /// 关闭页签:关掉 `idx` 的同时丢弃它的 hover / tooltip 状态,并把更高序号页签的
    /// 按序号索引的状态整体前移一位(哨兵键不受影响)。
    pub fn close_tab(&mut self, idx: usize) {
        let old_len = self.tabs.tabs().len();
        self.tabs.close(idx);
        if idx >= old_len {
            return;
        }
        let hover = std::mem::take(&mut self.hover);
        self.hover = hover
            .into_iter()
            .filter_map(|((i, c), h)| {
                if i >= old_len || i < idx {
                    Some(((i, c), h))
                } else if i == idx {
                    None
                } else {
                    Some(((i - 1, c), h))
                }
            })
            .collect();
        let starts = std::mem::take(&mut self.tooltip_starts);
        self.tooltip_starts = starts
            .into_iter()
            .filter_map(|(i, t)| match i.cmp(&idx) {
                std::cmp::Ordering::Less => Some((i, t)),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some((i - 1, t)),
            })
            .collect();
    }

    /// 收藏夹下拉按钮的 hover 进度,哨兵键 `(STAR_HOVER_KEY, true)`。
    pub(crate) fn bookmark_hover(&self) -> f32 {
        self.hover
            .get(&(STAR_HOVER_KEY, true))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 后退/前进/刷新三颗导航按钮的 hover 进度,各自独立哨兵键
    /// `(NAV_*_KEY, false)`,与真实 tab 序号和星标/收藏夹键都不冲突。
    pub(crate) fn nav_hover(&self, action: NavAction) -> f32 {
        let key = match action {
            NavAction::Back => NAV_BACK_KEY,
            NavAction::Forward => NAV_FORWARD_KEY,
            NavAction::Refresh => NAV_REFRESH_KEY,
        };
        self.hover
            .get(&(key, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 查找条按钮的 hover 进度(键见 `FIND_*_KEY`)。
    pub(crate) fn find_button_hover(&self, key: usize) -> f32 {
        self.hover
            .get(&(key, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// tab 栏末尾"新标签页"(+)按钮的 hover 进度,哨兵键
    /// `(NEW_TAB_KEY, false)`,远离真实 tab 序号空间。
    pub(crate) fn new_tab_hover(&self) -> f32 {
        self.hover
            .get(&(NEW_TAB_KEY, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }
}

/// 浏览器面板"星标/收藏夹"两个工具栏按钮的 hover 哨兵键——真实 tab 序号
/// 从 0 递增,不可能等于 `usize::MAX`,用它作键不与 tab 冲突。
pub(crate) const STAR_HOVER_KEY: usize = usize::MAX;
/// 后退/前进/刷新三颗导航按钮的 hover 哨兵键,依次紧挨 `STAR_HOVER_KEY`
/// 往下排,同样远离真实 tab 序号空间。
pub(crate) const NAV_BACK_KEY: usize = usize::MAX - 1;
pub(crate) const NAV_FORWARD_KEY: usize = usize::MAX - 2;
pub(crate) const NAV_REFRESH_KEY: usize = usize::MAX - 3;
/// tab 栏末尾"新标签页"(+)按钮的 hover 哨兵键,继续沿用上面的哨兵键空间。
pub(crate) const NEW_TAB_KEY: usize = usize::MAX - 4;
/// 查找条上一个 / 下一个 / 关闭按钮的 hover 哨兵键。
pub(crate) const FIND_PREV_KEY: usize = usize::MAX - 5;
pub(crate) const FIND_NEXT_KEY: usize = usize::MAX - 6;
pub(crate) const FIND_CLOSE_KEY: usize = usize::MAX - 7;
