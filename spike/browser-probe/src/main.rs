//! Digger 浏览器面板 Task 0 探针(一次性,随时可删)。
//!
//!   cargo run                      自动跑 S4 / S1 / S3 / S6,打印 `PROBE ...` 行后退出
//!   cargo run -- --interactive     S2 手动模式:一个带 wry 子视图的窗口,按页面上的提示试键盘/输入法
//!   cargo run -- --interactive --edit-menu   同上,但先装一个带 Cut/Copy/Paste/Select All 的 Edit 菜单
//!
//! 不访问外网:只加载本地 file:// 与不可达的 127.0.0.1:9;不调用 getUserMedia(不触发系统摄像头提示)。

use iced::widget::{column, operation, text, text_input};
use iced::{Element, Subscription, Task, window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::NSObjectProtocol;
use objc2_web_kit::{WKFrameInfo, WKPermissionDecision, WKSecurityOrigin, WKUIDelegate};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use wry::{WebView, WebViewBuilder};

const DIR: &str = "/tmp/browser-probe";

thread_local! {
    static VIEWS: RefCell<Vec<WebView>> = const { RefCell::new(Vec::new()) };
    static KEEP: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) };
}
static LAST_PANIC: Mutex<String> = Mutex::new(String::new());

fn out(line: impl AsRef<str>) {
    println!("PROBE {}", line.as_ref());
}

// ───────────────────────── S3:代理 UI 委托(拒绝媒体捕获,其余转发给 wry 原委托) ─────────────────────────

struct ProxyIvars {
    original: Retained<AnyObject>,
}

define_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ProxyIvars]
    struct DenyMediaProxy;

    unsafe impl NSObjectProtocol for DenyMediaProxy {}

    unsafe impl WKUIDelegate for DenyMediaProxy {
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn request_media_capture_permission(
            &self,
            _webview: Option<&AnyObject>,
            _origin: Option<&WKSecurityOrigin>,
            _frame: Option<&WKFrameInfo>,
            _capture_type: isize,
            decision_handler: &block2::Block<dyn Fn(isize)>,
        ) {
            decision_handler.call((WKPermissionDecision::Deny.0,));
        }
    }

    impl DenyMediaProxy {
        /// wry 原委托实现了新窗口、文件选择等可选方法;WebKit 先问 `respondsToSelector:` 再调,
        /// 所以这里既要转发调用,也要如实回答"能不能响应"。
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target(&self, _sel: Sel) -> Option<&AnyObject> {
            let original: &AnyObject = &self.ivars().original;
            Some(original)
        }

        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, sel: Sel) -> Bool {
            let own: Bool = unsafe { msg_send![super(self), respondsToSelector: sel] };
            if own.as_bool() {
                return own;
            }
            unsafe { msg_send![&*self.ivars().original, respondsToSelector: sel] }
        }
    }
);

fn probe_s3(window: &dyn window::Window, tx: Sender<String>, url: &str) -> String {
    use wry::WebViewExtMacOS;
    let mtm = MainThreadMarker::new().expect("main thread");
    let t2 = tx.clone();
    let view = match WebViewBuilder::new()
        .with_url(url)
        .with_bounds(rect())
        .with_document_title_changed_handler(move |t| {
            let _ = t2.send(format!("S3-live page title {t}"));
        })
        .build_as_child(&window)
    {
        Ok(v) => v,
        Err(e) => return format!("S3 build failed: {e}"),
    };
    let wk = view.webview();
    let original: Option<Retained<AnyObject>> = unsafe { msg_send![&*wk, UIDelegate] };
    let Some(original) = original else {
        return "S3 wry 没有设置 UIDelegate(意外)".into();
    };
    let wry_class = original.class().name().to_string_lossy().into_owned();
    // 先问原委托(wry)对「媒体捕获」「新窗口」「文件选择」三个可选方法的响应能力。
    let sels = [
        ("media-capture", sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:)),
        ("new-window", sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:)),
        ("file-panel", sel!(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:)),
    ];
    let before: Vec<String> = sels
        .iter()
        .map(|(n, s)| {
            let r: Bool = unsafe { msg_send![&*original, respondsToSelector: *s] };
            format!("{n}={}", r.as_bool())
        })
        .collect();
    let proxy = mtm
        .alloc::<DenyMediaProxy>()
        .set_ivars(ProxyIvars { original: original.clone() });
    let proxy: Retained<DenyMediaProxy> = unsafe { msg_send![super(proxy), init] };
    // WKWebView 对 UIDelegate 是 weak 引用:代理必须自己活着。
    let proxy_any: Retained<AnyObject> = unsafe { Retained::cast_unchecked(proxy.clone()) };
    unsafe {
        let _: () = msg_send![&*wk, setUIDelegate: &*proxy_any];
    }
    KEEP.with(|k| k.borrow_mut().push(proxy_any.clone()));
    let installed: Option<Retained<AnyObject>> = unsafe { msg_send![&*wk, UIDelegate] };
    let is_ours = installed
        .as_ref()
        .is_some_and(|d| d.class().name().to_string_lossy().contains("DenyMediaProxy"));
    let after: Vec<String> = sels
        .iter()
        .map(|(n, s)| {
            let r: Bool = unsafe { msg_send![&*proxy_any, respondsToSelector: *s] };
            format!("{n}={}", r.as_bool())
        })
        .collect();
    // 直接以 WebKit 的方式调用代理的媒体捕获方法,看它给什么决定(不触发真实采集)。
    let decided = std::cell::Cell::new(None::<isize>);
    {
        let blk = block2::RcBlock::new(|d: isize| decided.set(Some(d)));
        let _: () = unsafe {
            msg_send![
                &*proxy_any,
                webView: std::ptr::null::<AnyObject>(),
                requestMediaCapturePermissionForOrigin: std::ptr::null::<AnyObject>(),
                initiatedByFrame: std::ptr::null::<AnyObject>(),
                type: 0isize,
                decisionHandler: &*blk
            ]
        };
    }
    // 对照:原 wry 委托的决定(**只读源码,不调用**:它无条件 Grant,调用会走到真实采集路径)。
    let _ = tx;
    VIEWS.with(|v| v.borrow_mut().push(view));
    format!(
        "S3 wry原委托类={wry_class}; 原响应=[{}]; 代理已生效={is_ours}; 代理响应=[{}]; 代理对媒体捕获的决定={:?} (Grant=1, Deny=2)",
        before.join(","),
        after.join(","),
        decided.get()
    )
}

// ───────────────────────── S1:URL 处理与 file:// ─────────────────────────

fn rect() -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(20.0, 120.0).into(),
        size: wry::dpi::LogicalSize::new(420.0, 160.0).into(),
    }
}

fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let hook = LAST_PANIC.lock().unwrap().clone();
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        format!("PANIC {msg} {hook}")
    })
}

fn probe_url(window: &dyn window::Window, tx: Sender<String>, label: &str, url: &str, via_load_url: bool) -> String {
    let label_owned = label.to_string();
    let t1 = tx.clone();
    let l1 = label_owned.clone();
    let l2 = label_owned.clone();
    let built = guarded(|| {
        let mut b = WebViewBuilder::new()
            .with_bounds(rect())
            .with_on_page_load_handler(move |ev, u| {
                let kind = match ev { wry::PageLoadEvent::Started => "Started", wry::PageLoadEvent::Finished => "Finished" };
                let _ = t1.send(format!("{l1} load {kind} {u}"));
            })
            .with_document_title_changed_handler(move |t| {
                let _ = tx.send(format!("{l2} title {t}"));
            });
        if !via_load_url {
            b = b.with_url(url);
        } else {
            b = b.with_url("about:blank");
        }
        b.build_as_child(&window)
    });
    let view = match built {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return format!("{label} build Err: {e}"),
        Err(p) => return format!("{label} create {p}"),
    };
    let mut note = String::from("created");
    if via_load_url {
        match guarded(|| view.load_url(url)) {
            Ok(Ok(())) => note.push_str(" load_url=Ok"),
            Ok(Err(e)) => note.push_str(&format!(" load_url=Err({e})")),
            Err(p) => note.push_str(&format!(" load_url={p}")),
        }
    }
    VIEWS.with(|v| v.borrow_mut().push(view));
    format!("{label} {note}")
}

fn write_fixtures() {
    let _ = std::fs::create_dir_all(DIR);
    let page = |name: &str| {
        format!(
            "<!doctype html><meta charset=utf-8><title>LOADED:{name}</title><link rel=stylesheet href=s.css>\
<body>hello <img src=i.svg onload=\"document.title+='|IMG-OK'\" onerror=\"document.title+='|IMG-ERR'\">\
<script>window.addEventListener('load',()=>{{document.title+='|BG:'+getComputedStyle(document.body).backgroundColor;}});\
fetch('file://{DIR}/other.txt').then(r=>r.text()).then(t=>document.title+='|FETCH-OK:'+t.trim()).catch(e=>document.title+='|FETCH-ERR');</script>"
        )
    };
    for name in ["a.html", "My File.html", "中文.html"] {
        let _ = std::fs::write(format!("{DIR}/{name}"), page(name));
    }
    let _ = std::fs::write(
        format!("{DIR}/media.html"),
        "<!doctype html><meta charset=utf-8><title>media</title><script>(async()=>{const r=[];\
if(!navigator.mediaDevices){document.title='MEDIA:no-mediaDevices(secure=' + window.isSecureContext + ')';return}\
for(const c of [{video:true},{audio:true}]){try{const s=await navigator.mediaDevices.getUserMedia(c);s.getTracks().forEach(t=>t.stop());r.push(JSON.stringify(c)+':GRANTED')}catch(e){r.push(JSON.stringify(c)+':'+e.name)}}\
document.title='MEDIA:'+r.join(',')+'(secure='+window.isSecureContext+')'})()</script>",
    );
    let _ = std::fs::write(format!("{DIR}/s.css"), "body{background:#123456}");
    let _ = std::fs::write(format!("{DIR}/other.txt"), "SECRET-FROM-OTHER-FILE");
    let _ = std::fs::write(format!("{DIR}/i.svg"), "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='red'/></svg>");
    let _ = std::fs::write(
        format!("{DIR}/keys.html"),
        "<!doctype html><meta charset=utf-8><title>keys</title><body style='font:14px sans-serif'>\
<p>S2 手动测试:依次在下面试 ① 打字 ② ⌘A ③ ⌘C ④ ⌘V ⑤ ⌘X ⑥ 中文输入法(拼音+选词)。事件会写进窗口标题并打印到终端。</p>\
<input id=i style='width:360px' placeholder='input'><br><br><textarea id=t style='width:360px;height:70px'>hello world</textarea>\
<script>const log=(m)=>{document.title='EV:'+m};\
for(const id of ['i','t']){const e=document.getElementById(id);\
['keydown','paste','copy','cut','compositionstart','compositionend','focus'].forEach(n=>e.addEventListener(n,ev=>log(id+':'+n+(ev.key?':'+ev.key:'')+(ev.metaKey?'+meta':''))));}</script>",
    );
}

// ───────────────────────── iced 应用 ─────────────────────────

struct App {
    win: Option<window::Id>,
    rx: Receiver<String>,
    tx: Sender<String>,
    a: String,
    b: String,
    interactive: bool,
    edit_menu: bool,
    urls: Vec<(&'static str, String)>,
}

#[derive(Debug, Clone)]
enum Msg {
    Win(Option<window::Id>),
    Poll,
    InputA(String),
    InputB(String),
    Focused(&'static str, bool),
    Next(usize),
    UrlDone(usize, String),
    S3Done(String),
    S6Done(String),
    Finish,
}

fn boot() -> (App, Task<Msg>) {
    write_fixtures();
    std::panic::set_hook(Box::new(|info| {
        *LAST_PANIC.lock().unwrap() = format!("{info}");
    }));
    let (tx, rx) = channel();
    let args: Vec<String> = std::env::args().collect();
    let urls = vec![
        ("f_plain", format!("file://{DIR}/a.html")),
        ("f_space_raw", format!("file://{DIR}/My File.html")),
        ("f_space_enc", format!("file://{DIR}/My%20File.html")),
        ("f_cjk_raw", format!("file://{DIR}/中文.html")),
        ("f_cjk_enc", format!("file://{DIR}/%E4%B8%AD%E6%96%87.html")),
        ("h_space_raw", "http://127.0.0.1:9/a b?x=1 y".to_string()),
        ("scheme_less_space", "exa mple.com".to_string()),
    ];
    (
        App {
            win: None,
            rx,
            tx,
            a: String::new(),
            b: String::new(),
            interactive: args.iter().any(|a| a == "--interactive"),
            edit_menu: args.iter().any(|a| a == "--edit-menu"),
            urls,
        },
        window::oldest().map(Msg::Win),
    )
}

fn sleep(ms: u64) -> Task<()> {
    Task::perform(async move { tokio::time::sleep(Duration::from_millis(ms)).await }, |_| ())
}

fn update(app: &mut App, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Win(id) => {
            app.win = id;
            let Some(id) = id else { return Task::none() };
            if std::env::args().any(|a| a == "--s3-live") {
                let tx = app.tx.clone();
                return window::run(id, move |w| probe_s3(&w, tx, &"http://127.0.0.1:18765/media.html".to_string()))
                    .map(Msg::S3Done);
            }
            if app.interactive {
                let tx = app.tx.clone();
                let edit = app.edit_menu;
                return window::run(id, move |w| {
                    if edit {
                        install_edit_menu();
                    }
                    let t2 = tx.clone();
                    let view = WebViewBuilder::new()
                        .with_url(format!("file://{DIR}/keys.html"))
                        .with_bounds(rect())
                        .with_document_title_changed_handler(move |t| {
                            let _ = t2.send(format!("page title {t}"));
                        })
                        .build_as_child(&w);
                    match view {
                        Ok(v) => VIEWS.with(|x| x.borrow_mut().push(v)),
                        Err(e) => out(format!("S2 build failed: {e}")),
                    }
                })
                .map(|_| Msg::Poll);
            }
            // S4:iced 层的焦点读取(无需人工操作)
            let focus_a = operation::focus::<Msg>("a");
            let read_a = operation::is_focused("a").map(|f| Msg::Focused("after focus(a): a", f));
            let read_b = operation::is_focused("b").map(|f| Msg::Focused("after focus(a): b", f));
            let focus_b = operation::focus::<Msg>("b");
            let read_a2 = operation::is_focused("a").map(|f| Msg::Focused("after focus(b): a", f));
            let read_b2 = operation::is_focused("b").map(|f| Msg::Focused("after focus(b): b", f));
            Task::batch([focus_a.chain(Task::batch([read_a, read_b])).chain(focus_b).chain(Task::batch([read_a2, read_b2]))])
                .chain(sleep(600).map(|_| Msg::Next(0)))
        }
        Msg::Poll => {
            while let Ok(line) = app.rx.try_recv() {
                out(line);
            }
            Task::none()
        }
        Msg::InputA(s) => {
            app.a = s;
            Task::none()
        }
        Msg::InputB(s) => {
            app.b = s;
            Task::none()
        }
        Msg::Focused(tag, f) => {
            out(format!("S4 {tag}={f}"));
            Task::none()
        }
        Msg::Next(i) => {
            let Some(id) = app.win else { return Task::none() };
            if i >= app.urls.len() * 2 {
                let tx = app.tx.clone();
                return window::run(id, move |w| probe_s3(&w, tx, "about:blank")).map(Msg::S3Done);
            }
            let (label, url) = app.urls[i / 2].clone();
            let via_load = i % 2 == 1;
            let label_full = format!("S1 {label}/{}", if via_load { "load_url" } else { "with_url" });
            let tx = app.tx.clone();
            window::run(id, move |w| {
                // 每个用例先清掉上一个视图,避免叠在一起。
                VIEWS.with(|v| v.borrow_mut().clear());
                probe_url(&w, tx, &label_full, &url, via_load)
            })
            .map(move |r| Msg::UrlDone(i, r))
        }
        Msg::UrlDone(i, r) => {
            out(r);
            sleep(2500).map(move |_| Msg::Next(i + 1))
        }
        Msg::S3Done(r) => {
            out(r);
            if std::env::args().any(|a| a == "--s3-live") {
                return sleep(5000).map(|_| Msg::Finish);
            }
            let Some(id) = app.win else { return Task::none() };
            window::run(id, move |w| probe_s6(&w)).map(Msg::S6Done)
        }
        Msg::S6Done(r) => {
            out(r);
            sleep(500).map(|_| Msg::Finish)
        }
        Msg::Finish => {
            while let Ok(line) = app.rx.try_recv() {
                out(line);
            }
            out("DONE");
            iced::exit()
        }
    }
}

fn webcontent_rss_kb() -> u64 {
    let o = std::process::Command::new("ps").args(["-axo", "rss=,comm="]).output().ok();
    let s = o.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    s.lines()
        .filter(|l| l.contains("WebContent"))
        .filter_map(|l| l.split_whitespace().next()?.parse::<u64>().ok())
        .sum()
}

fn probe_s6(window: &dyn window::Window) -> String {
    VIEWS.with(|v| v.borrow_mut().clear());
    std::thread::sleep(Duration::from_millis(800));
    let own = |pid: u32| -> u64 {
        let o = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().ok();
        o.and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(0)
    };
    let pid = std::process::id();
    let (wc0, own0) = (webcontent_rss_kb(), own(pid));
    let mut times = Vec::new();
    for _ in 0..8 {
        let t = Instant::now();
        let v = WebViewBuilder::new()
            .with_url(format!("file://{DIR}/a.html"))
            .with_bounds(rect())
            .build_as_child(&window);
        times.push(t.elapsed().as_millis());
        if let Ok(v) = v {
            VIEWS.with(|x| x.borrow_mut().push(v));
        }
    }
    std::thread::sleep(Duration::from_millis(2500));
    let (wc1, own1) = (webcontent_rss_kb(), own(pid));
    VIEWS.with(|v| v.borrow_mut().clear());
    std::thread::sleep(Duration::from_millis(1500));
    let wc2 = webcontent_rss_kb();
    format!(
        "S6 8个页签: 创建耗时(ms)={times:?}; 本进程RSS {own0}->{own1} KB; 系统内WebContent进程RSS合计 {wc0}->{wc1} KB(增 {} KB,含其它应用的波动); 全部销毁后 {wc2} KB",
        wc1 as i64 - wc0 as i64
    )
}

fn install_edit_menu() {
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    let main = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    main.addItem(&app_item);
    let edit_item = NSMenuItem::new(mtm);
    main.addItem(&edit_item);
    let edit = unsafe { NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Edit")) };
    for (title, action, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    edit_item.setSubmenu(Some(&edit));
    app.setMainMenu(Some(&main));
    out("S2 已安装 Edit 菜单(Cut/Copy/Paste/Select All,nil 目标)");
}

fn view(app: &App) -> Element<'_, Msg> {
    column![
        text(if app.interactive {
            "S2 手动模式:在下方 webview 的输入框里试键盘/输入法;下面两个是 iced 的输入框"
        } else {
            "探针运行中(S4/S1/S3/S6)…"
        }),
        text_input("a", &app.a).id("a").on_input(Msg::InputA),
        text_input("b", &app.b).id("b").on_input(Msg::InputB),
    ]
    .spacing(8)
    .padding(12)
    .into()
}

fn subscription(_: &App) -> Subscription<Msg> {
    iced::time::every(Duration::from_millis(200)).map(|_| Msg::Poll)
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        .subscription(subscription)
        .window_size((480.0, 330.0))
        .run()
}
