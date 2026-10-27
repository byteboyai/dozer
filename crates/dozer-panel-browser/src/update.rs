//! 浏览器面板的消息与 `update`。

use crate::WEB_OWNER;
use crate::adapter;
use crate::bookmarks::{bookmark_status, optimistic_add, optimistic_remove};
use crate::state::State;
use dozer_model::{BookmarkInfo, BookmarkScope};
use dozer_panel_host::RequestTokenWrapExt;
use dozer_services::{BookmarkService, ServiceErrorKind};
use std::sync::Arc;

/// 浏览器导航动作(后退/前进/刷新)。实际导航由 main.rs 的 `dispatch`
/// 拦截 `Message::Nav` 后对激活 webview 的 `wry::WebView` 句柄执行(句柄
/// 在 main.rs 的 `browser_webviews` 池里,浏览器 `State` 摸不到);这里
/// 只是纯语义枚举,不带任何状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavAction {
    Back,
    Forward,
    Refresh,
}

/// 浏览器面板自己的消息类型——内核(`workspace.rs`)只认一个包装变体
/// `Message::Browser(extensions::browser::Message)`,这个模块本身不
/// import 顶层 `Message`。原通用的 `AddrEvent` 文本输入事件类型已随
/// Todo MARKDOWN 键盘路由移除(2026-09-01 SQLite 迁移),不再引用。
#[derive(Debug, Clone)]
pub enum Message {
    AsyncResult(bytehost_core::request::RequestToken, Box<Message>),
    OpenUrl(String),
    /// 页面加载完成后,渲染进程经 IPC 报回 HTML `document.title`(`id` 是
    /// webview/tab id,`title` 是页面标题)——覆盖 `open_url` 初建的 URL 标题。
    /// `about:blank` 等空标题页面不覆盖(见 `Tabs::apply_title` 的空标题守卫)。
    TitleLoaded(usize, String),
    /// 网页内超链接/`window.open` 等让 webview 自行导航后,渲染进程经 IPC
    /// 报回的目标 URL(`id` 是 webview/tab id,`url` 是 webview 实际加载到的
    /// 地址)。让浏览器面板从"导航由地址栏主动发起"扩展到"跟随页面内部
    /// 跳转",否则点超链接后面板仍停留在旧 URL。
    Loaded(usize, String),
    SelectTab(usize),
    CloseTab(usize),
    /// 后退/前进/刷新按钮。`browser::update` 里是 no-op,真正的 webview
    /// 导航由 main.rs `dispatch` 拦截执行(见 `NavAction`)。
    Nav(NavAction),
    /// 拖拽换位:光标扫过页签 `idx` 时由 tab 的 `MouseArea::on_move` 发出,
    /// `App::update` 翻译成 `TabDragMove`(浏览器组在这里完成换位)。
    DragHover(usize),
    /// 地址栏草稿变化(iced `text_input::on_input`,每次按键给全量当前
    /// 字符串)。
    AddrInput(String),
    AddrSubmit,
    /// 地址栏被右键:内核拦截,不进 `update`——转发成顶层
    /// `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(adapter::TextInputTarget),
    StarClick,
    BookmarkAdd(BookmarkScope),
    BookmarkRemove(i64),
    BookmarksToggle,
    /// `project_id` 用 `None` 表示首页全局浏览器/无项目工作区的全局收藏。
    /// 收藏夹刷新结果。读失败时 `update` 保持本地缓存不动(此前失败被 `unwrap_or_default()` 吞成空列表整体替换)。
    BookmarksLoaded(
        Option<i64>,
        Result<Vec<BookmarkInfo>, dozer_services::ServiceError>,
    ),
    BookmarksMutated(Option<i64>, Result<(), String>),
    /// tab 标题/关闭按钮的 hover 进入/离开(idx = tab 序号, bool = 是否关闭
    /// 按钮, 最后 bool = 进入/离开)。浏览器面板有独立 `State`,无法复用顶栏
    /// 全局 `App::hover_progress`,自己维护一套进度机(见 `State::hover`)。
    Hover(usize, bool, bool),
    /// 收藏夹侧栏分割线开始拖:扩展发不了 app 级拖拽消息,由内核代发,见
    /// `App::update` 里 `Message::Browser(Message::ColumnDragStart)` 分支
    /// (同 `extensions::git_log::Message::ColumnDragStart` 的处理方式)。
    ColumnDragStart,
}

/// 处理浏览器面板的全部消息。`project_id` 由内核每次调用时从
/// `ws.project.as_ref().map(|p| p.id)` 现取传入(`State` 本身不存这个,
/// 避免状态冗余/漂移)。`service` 用于收藏夹的 dozerd RPC 往返——这是跟
/// Git Log 试点 `update` 签名的主要差异(Git Log 只有本地 `git2` 阻塞
/// 调用,不需要网络)。
pub fn update(
    state: &mut State,
    msg: Message,
    project_id: Option<i64>,
    service: &Arc<dyn BookmarkService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let mut msg = msg;
    while let Message::AsyncResult(token, inner) = msg {
        if token.scope() != &dozer_panel_host::scope_of(project_id)
            || !state.requests.accepts(&token)
        {
            return;
        }
        msg = *inner;
    }
    let emit = state
        .requests
        .scope(project_id)
        .wrap(emit, Message::AsyncResult);
    match msg {
        Message::AsyncResult(..) => unreachable!("nested async result"),
        Message::OpenUrl(url) => {
            state.error = None;
            state.tabs.open_url(url);
        }
        Message::TitleLoaded(id, title) => state.tabs.apply_title(id, title),
        Message::Loaded(id, url) => state.tabs.apply_url(id, url),
        Message::SelectTab(idx) => state.tabs.select(idx),
        Message::DragHover(_) => {} // 拖拽换位在 `App::update` 翻译后处理,不落到这里
        // 导航按钮:真正的 webview 历史导航/刷新由 main.rs `dispatch` 拦截
        // `Message::Browser(Nav(..))` 执行,这里不碰状态(no-op)。
        Message::Nav(_) => {}
        Message::CloseTab(idx) => {
            state.close_tab(idx);
        }
        Message::Hover(idx, is_close, hovered) => {
            state.hover.entry((idx, is_close)).or_default().set(hovered);
            // 标题 tooltip 计时:进入即记起点、离开即清(满 2s 由
            // `hover_tooltip_ready` 判断,与关闭按钮的 hover 无关,只认页签
            // 整体悬停)。
            state.set_tab_tooltip(idx, hovered);
        }
        Message::AddrInput(s) => state.tabs.set_addr_buffer(s),
        Message::AddrSubmit => match state.tabs.addr_submit() {
            Ok(Some(url)) => {
                // 地址栏提交:在当前激活 tab 打开(复用 tab),不新开——
                // 新建走 "+" 按钮(`OpenUrl`)。原实现 recurse 进
                // `Message::OpenUrl` 会新开 tab,这里是行为变更,见计划。
                state.tabs.navigate_active(url);
            }
            Ok(None) => {}
            Err(message) => state.error = Some(message),
        },

        Message::StarClick => {
            state.error = None;
            let Some(url) = state
                .tabs
                .tabs()
                .get(state.tabs.active_idx())
                .map(|t| t.url.clone())
            else {
                return;
            };
            let status = bookmark_status(&state.bookmarks, &url, project_id);
            if !status.is_bookmarked() {
                // 未收藏 → 点击星标直接加入**全局**收藏(Chrome/Safari 同款
                // 单点切换语义),委托 `BookmarkAdd` 处理(乐观插入 + dozerd RPC)。
                emit(Message::BookmarkAdd(BookmarkScope::Global));
                return;
            }
            // 已收藏(选中态)→ 再点即全部移出收藏。复用 `BookmarkRemove`
            // 的处理(乐观移除 + dozerd RPC),逐 scope 派发。
            for id in status.global.into_iter().chain(status.project) {
                emit(Message::BookmarkRemove(id));
            }
        }
        Message::BookmarksToggle => state.bookmarks_open = !state.bookmarks_open,
        Message::BookmarkAdd(scope) => {
            state.requests.invalidate("bookmarks");
            let Some(tab) = state.tabs.tabs().get(state.tabs.active_idx()) else {
                return;
            };
            let url = tab.url.clone();
            let title = tab.title.clone();
            let target_project_id = match scope {
                BookmarkScope::Global => None,
                BookmarkScope::Project => project_id,
            };
            let created_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            optimistic_add(
                &mut state.bookmarks,
                scope,
                target_project_id,
                &url,
                &title,
                created_ms,
            );
            // 全局收藏在首页/无项目工作区也能持久化:Project scope 仍需要真实
            // project_id;Global scope 即使没有 project_id 也发 RPC。
            // `BookmarksMutated(Option<i64>, ...)` 会把结果路由回对应浏览器。
            if matches!(scope, BookmarkScope::Project) && project_id.is_none() {
                return;
            }
            let service = service.clone();
            handle.spawn(async move {
                let res = service
                    .add_bookmark(scope, target_project_id, url, title)
                    .await
                    .map_err(|e| e.message);
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarkRemove(id) => {
            state.requests.invalidate("bookmarks");
            optimistic_remove(&mut state.bookmarks, id);
            let service = service.clone();
            handle.spawn(async move {
                let res = service.remove_bookmark(id).await.map_err(|e| e.message);
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarksLoaded(_, Ok(bookmarks)) => state.bookmarks = bookmarks,
        Message::BookmarksLoaded(_, Err(e)) => {
            // 读失败:缓存保持不动(此前 `unwrap_or_default()` 会把失败变成空列表整体替换)。
            if e.kind == ServiceErrorKind::Unavailable {
                tracing::debug!(target: "dozer::panel::web", "收藏夹读取失败(dozerd 不可用): {}", e.message);
            } else {
                state.error = Some(crate::i18n::panel_keyed_args(
                    WEB_OWNER,
                    "error.bookmarks_load_failed",
                    "Failed to load bookmarks: {e}",
                    &[("e", e.message.as_str())],
                ));
            }
        }
        Message::BookmarksMutated(_, res) => {
            if let Err(message) = res {
                state.error = Some(crate::i18n::panel_keyed_args(
                    WEB_OWNER,
                    "error.bookmarks_op_failed",
                    "Bookmark operation failed: {e}",
                    &[("e", message.as_str())],
                ));
            }
            request_bookmarks_refresh(&state.requests, project_id, service, handle, emit);
        }
        Message::ColumnDragStart => {
            debug_assert!(
                false,
                "ColumnDragStart 由内核在 Message::Browser 分支里直接处理\
                 (转成 app 级拖拽消息),不会转发到这里"
            );
        }
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 browser::Message::TextInputMenuOpen 文档")
        }
    }
}

/// 现有 `Workspace::spawn_bookmarks_refresh` 的搬家版本:异步拉取
/// 收藏夹合集。`project_id = Some(pid)` 时返回"全局 + pid 项目";`None` 时
/// 只返回全局收藏(用于首页全局浏览器/无项目工作区)。
pub fn request_bookmarks_refresh(
    requests: &dozer_panel_host::RequestTrackers,
    project_id: Option<i64>,
    service: &Arc<dyn BookmarkService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let emit = requests
        .begin(project_id, "bookmarks")
        .wrap(emit, Message::AsyncResult);
    let service = service.clone();
    handle.spawn(async move {
        let result = service.list_bookmarks(project_id).await;
        emit(Message::BookmarksLoaded(project_id, result));
    });
}

impl Message {
    pub fn result_project(&self) -> Option<i64> {
        match self {
            Self::AsyncResult(token, _) => dozer_panel_host::project_of(token.scope()),
            _ => None,
        }
    }
}

#[cfg(test)]
impl Message {
    // Existing payload tests inspect service conversion separately from delivery guards.
    pub(crate) fn result_payload(self) -> Self {
        match self {
            Self::AsyncResult(_, inner) => inner.result_payload(),
            other => other,
        }
    }
}
