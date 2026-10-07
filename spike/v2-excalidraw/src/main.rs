//! V2:把一个静态 Web 应用目录交给**真实的** bytehost gateway(按 `network.outbound = none` 算出的 CSP),
//! 在与 Dozer 应用 webview 同样受限的 wry webview 里加载,等页面稳定后收集探测结果,打一行
//! `RESULT {...}` 到 stdout 后退出。
//!
//! 用法: v2-excalidraw-spike --dir <静态站点目录> [--id excalidraw] [--store 7] [--port 24680] [--wait 10] [--net none|any]
//!       v2-excalidraw-spike --purge --store 7        (清除该存储标识的 WebView 数据存储)
//! 注意:origin 含端口,**要验证跨次运行的持久化必须固定 `--port`**(默认 0 = 每次随机,存储天然不连续)。
//! 探测项:CSP 违规、`EXCALIDRAW_ASSET_PATH`、字体加载状态、画布是否出现、Service Worker、
//! 剪贴板读写、`window.open`、blob 下载、localStorage 写入、被拒绝的导航与下载。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytehost_apps::gateway::{Gateway, GatewayConfig, csp_for};
use bytehost_apps::id::AppId;
use bytehost_apps::permissions::{NetworkPerm, Outbound, Permissions};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{WebViewBuilder, WebViewBuilderExtDarwin};

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
        .unwrap_or(default.into())
}

/// 页面脚本之前注入:收集 CSP 违规与脚本错误。
const INIT: &str = r#"
(function(){
  var r = { csp: [], errors: [], rejections: [] };
  try { r.lsAtStart = Object.keys(localStorage).slice(0,20); } catch (e) { r.lsAtStart = 'err:' + e; }
  document.addEventListener('securitypolicyviolation', function(e){
    r.csp.push(e.violatedDirective + ' <- ' + String(e.blockedURI).slice(0,120));
  });
  window.addEventListener('error', function(e){ r.errors.push(String(e.message).slice(0,160)); });
  window.addEventListener('unhandledrejection', function(e){ r.rejections.push(String(e.reason).slice(0,160)); });
  window.__probe = r;
})();
"#;

const COLLECT: &str = r#"
(async function(){
  var out = Object.assign({}, window.__probe);
  out.href = location.href;
  out.title = document.title;
  out.assetPath = typeof window.EXCALIDRAW_ASSET_PATH + ':' + JSON.stringify(window.EXCALIDRAW_ASSET_PATH);
  out.excalidrawRoot = !!document.querySelector('.excalidraw');
  out.canvases = document.querySelectorAll('canvas').length;
  out.isSecureContext = window.isSecureContext;
  out.swApi = 'serviceWorker' in navigator;
  try { out.swRegs = (await navigator.serviceWorker.getRegistrations()).length; } catch (e) { out.swRegs = 'err:' + e; }
  try { out.fonts = Array.from(document.fonts).map(function(f){ return f.family + ':' + f.status; }).slice(0,40); } catch (e) { out.fonts = 'err:' + e; }
  out.fontCheck = {};
  ['Excalifont','Virgil','Nunito','Assistant','Cascadia'].forEach(function(n){ try { out.fontCheck[n] = document.fonts.check('16px "' + n + '"'); } catch (e) { out.fontCheck[n] = 'err'; } });
  out.fontLoad = {};
  for (const n of ['Excalifont','Virgil','Cascadia','Nunito','Comic Shanns','Assistant','Lilita One','Liberation Sans','Xiaolai']) {
    try {
      var got = await Promise.race([document.fonts.load('16px "' + n + '"'), new Promise(function(r){ setTimeout(function(){ r('timeout'); }, 4000); })]);
      out.fontLoad[n] = got === 'timeout' ? 'timeout' : (got.length + ' faces, status=' + got.map(function(f){return f.status;}).join(','));
    } catch (e) { out.fontLoad[n] = 'err:' + String(e).slice(0,60); }
  }
  try { await navigator.clipboard.writeText('v2'); out.clipWrite = 'ok'; } catch (e) { out.clipWrite = String(e).slice(0,100); }
  try { await navigator.clipboard.readText(); out.clipRead = 'ok'; } catch (e) { out.clipRead = String(e).slice(0,100); }
  try { var w = window.open('about:blank'); out.windowOpen = w ? 'opened' : 'null'; } catch (e) { out.windowOpen = String(e).slice(0,100); }
  try { localStorage.setItem('__v2', '1'); out.localStorage = localStorage.getItem('__v2'); } catch (e) { out.localStorage = String(e).slice(0,100); }
  try { out.lsKeys = Object.keys(localStorage).slice(0,20); } catch (e) {}
  try { var a = document.createElement('a'); a.href = URL.createObjectURL(new Blob(['x'])); a.download = 'v2.txt'; document.body.appendChild(a); a.click(); out.download = 'clicked'; } catch (e) { out.download = String(e).slice(0,100); }
  window.ipc.postMessage(JSON.stringify(out));
})();
"#;

/// `--purge <store>`:只做一件事——对该存储标识调 `WebView::remove_data_store`(验证"卸载含数据"能真的清掉
/// localStorage/IndexedDB;wry 要求先 drop 所有使用它的 webview,这里一个都没建)。
fn purge(store: u8) {
    use wry::WebViewExtDarwin;
    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let mut sid = [0u8; 16];
    sid[0] = store;
    let _window = WindowBuilder::new()
        .with_title("purge")
        .build(&event_loop)
        .expect("window");
    let p = proxy.clone();
    wry::WebView::remove_data_store(&sid, move |r| {
        let _ = p.send_event(format!("{r:?}"));
    });
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        if let Event::UserEvent(msg) = event {
            println!("PURGE {msg}");
            *flow = ControlFlow::Exit;
        }
    });
}

fn main() {
    if std::env::args().any(|a| a == "--purge") {
        return purge(arg("--store", "7").parse().unwrap_or(7));
    }
    let dir = PathBuf::from(arg("--dir", ""));
    assert!(dir.join("index.html").is_file(), "--dir 里要有 index.html");
    let app = arg("--id", "excalidraw");
    let store: u8 = arg("--store", "7").parse().unwrap_or(7);
    let wait: u64 = arg("--wait", "10").parse().unwrap_or(10);
    let net = arg("--net", "none");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let id = AppId::new(app.clone()).unwrap();
    let grants = Permissions {
        network: NetworkPerm {
            outbound: if net == "any" {
                Outbound::Any
            } else {
                Outbound::None
            },
        },
        ..Permissions::default()
    };
    let csp = csp_for(&grants);
    eprintln!("CSP: {csp:?}");
    let gateway = rt
        .block_on(Gateway::start(GatewayConfig {
            port: arg("--port", "0").parse().unwrap_or(0),
        }))
        .expect("gateway");
    gateway.add_site(&id, dir, csp);
    let url = gateway.launch_url(&id);
    eprintln!(
        "port {} launch {}",
        gateway.port(),
        url.split('?').next().unwrap_or("")
    );

    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("v2 probe")
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 760.0))
        .build(&event_loop)
        .expect("window");

    let denied_navs = Arc::new(Mutex::new(Vec::<String>::new()));
    let downloads = Arc::new(Mutex::new(Vec::<String>::new()));
    let (nav_log, dl_log) = (denied_navs.clone(), downloads.clone());
    let origin_prefix = format!("http://{app}.localhost:{}/", gateway.port());
    let ipc_proxy = proxy.clone();
    let mut sid = [0u8; 16];
    sid[0] = store;
    let webview = WebViewBuilder::new()
        .with_url(&url)
        .with_data_store_identifier(sid)
        .with_initialization_script(INIT)
        .with_ipc_handler(move |req| {
            let _ = ipc_proxy.send_event(req.body().clone());
        })
        .with_navigation_handler(move |u| {
            let ok = u.starts_with(&origin_prefix)
                || u == "about:blank"
                || u.starts_with("blob:http://");
            if !ok {
                nav_log.lock().unwrap().push(u.chars().take(120).collect());
            }
            ok
        })
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(move |u, _| {
            dl_log.lock().unwrap().push(u.chars().take(120).collect());
            false
        })
        .with_devtools(false)
        .build(&window)
        .expect("webview");

    let timer = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(wait));
        let _ = timer.send_event("__COLLECT__".into());
    });
    let hard = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(wait + 20));
        let _ = hard.send_event("{\"timeout\":true}".into());
    });

    let _keep = (rt, gateway);
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(msg) if msg == "__COLLECT__" => {
                let _ = webview.evaluate_script(COLLECT);
            }
            Event::UserEvent(json) => {
                let mut v: serde_json::Value =
                    serde_json::from_str(&json).unwrap_or(serde_json::json!({"raw": json}));
                v["deniedNavigations"] = serde_json::json!(*denied_navs.lock().unwrap());
                v["downloadAttempts"] = serde_json::json!(*downloads.lock().unwrap());
                println!("RESULT {v}");
                *flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
