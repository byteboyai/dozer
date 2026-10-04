//! 一次性验证:WKWebView 里三种"每应用独立 origin"方案(wry 自定义协议 / `<app>.localhost:端口` /
//! `127.0.0.1:端口`)的真实能力——fetch、WebSocket、SSE、localStorage/IndexedDB/cookie 跨重启持久化、
//! Service Worker、跨源可读性、`with_data_store_identifier` 的存储隔离。
//!
//! 用法: origin-gateway-spike --mode custom|localhost|loopback --phase write|read [--store N] [--port P]
//! 结果以一行 JSON 打到 stdout(`RESULT {...}`)。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;
use wry::WebViewBuilderExtDarwin;

const PAGE: &str = include_str!("page.html");
const APP: &str = "excalidraw";
const SW: &str = "self.addEventListener('install',e=>self.skipWaiting());self.addEventListener('activate',e=>self.clients.claim());";

struct Cfg {
    mode: String,
    phase: String,
    store: u8,
    port: u16,
    marker: String,
}

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or(default.into())
}

fn page(cfg: &Cfg) -> String {
    PAGE.replace("__MODE__", &cfg.mode)
        .replace("__PHASE__", &cfg.phase)
        .replace("__LOOP__", &format!("127.0.0.1:{}", cfg.port))
        .replace("__MARKER__", &cfg.marker)
}

/// 两种承载共用的路由:(状态, content-type, 正文)。
fn route(cfg: &Cfg, method: &str, path: &str, body: &[u8], host: &str) -> (u16, &'static str, Vec<u8>) {
    match path {
        "/" | "/index.html" => (200, "text/html; charset=utf-8", page(cfg).into_bytes()),
        "/sw.js" => (200, "application/javascript", SW.as_bytes().to_vec()),
        "/ping" => (200, "application/json", format!("{{\"ok\":true,\"host\":{:?}}}", host).into_bytes()),
        "/echo" if method == "POST" => (200, "text/plain", body.to_vec()),
        // 自定义协议没有流式应答,SSE 只能一次性给完
        "/events" => (200, "text/event-stream", b"data: 1\n\ndata: 2\n\ndata: 3\n\n".to_vec()),
        _ => (404, "text/plain", b"not found".to_vec()),
    }
}

fn serve_http(cfg: std::sync::Arc<Cfg>, listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        let cfg = cfg.clone();
        thread::spawn(move || handle_conn(&cfg, stream));
    }
}

fn handle_conn(cfg: &Cfg, mut stream: TcpStream) {
    let mut buf = [0u8; 8192];
    // peek 到完整请求头,不消费(WebSocket 升级要把整条流交给 tungstenite)
    let mut n = 0;
    for _ in 0..200 {
        n = stream.peek(&mut buf).unwrap_or(0);
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let head_end = match buf[..n].windows(4).position(|w| w == b"\r\n\r\n") {
        Some(i) => i + 4,
        None => return,
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let lower = head.to_ascii_lowercase();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let host = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("host:").map(|v| v.trim().to_string()))
        .unwrap_or_default();
    if lower.contains("upgrade: websocket") && path == "/ws" {
        if let Ok(mut ws) = tungstenite::accept(stream) {
            if let Ok(m) = ws.read() {
                let _ = ws.send(m);
            }
            let _ = ws.close(None);
        }
        return;
    }
    let mut consume = vec![0u8; head_end];
    let _ = stream.read_exact(&mut consume);
    let clen = lower
        .lines()
        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
        .unwrap_or(0);
    let mut body = vec![0u8; clen];
    let _ = stream.read_exact(&mut body);
    if path == "/events" {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n");
        for i in 1..=3 {
            let _ = stream.write_all(format!("data: {i}\n\n").as_bytes());
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(100));
        }
        return;
    }
    let (status, ct, out) = route(cfg, method, path, &body, &host);
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        out.len()
    );
    let _ = stream.write_all(&out);
}

fn main() {
    let cfg = std::sync::Arc::new(Cfg {
        mode: arg("--mode", "loopback"),
        phase: arg("--phase", "write"),
        store: arg("--store", "1").parse().unwrap_or(1),
        port: arg("--port", "18765").parse().unwrap_or(18765),
        marker: arg("--marker", "M-default"),
    });
    // 所有模式都起本机 HTTP/WS 服务(自定义协议模式用它测"跨源回环 WebSocket")
    let listener = TcpListener::bind(("127.0.0.1", cfg.port)).expect("bind");
    {
        let cfg = cfg.clone();
        thread::spawn(move || serve_http(cfg, listener));
    }
    let url = match cfg.mode.as_str() {
        "custom" => format!("app-{APP}://localhost/"),
        "localhost" => format!("http://{APP}.localhost:{}/", cfg.port),
        _ => format!("http://127.0.0.1:{}/", cfg.port),
    };

    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new().with_title("origin spike").with_inner_size(tao::dpi::LogicalSize::new(480.0, 320.0)).build(&event_loop).expect("window");

    let ipc_proxy = proxy.clone();
    let proto_cfg = cfg.clone();
    let mut id = [0u8; 16];
    id[0] = cfg.store;
    let builder = WebViewBuilder::new()
        .with_data_store_identifier(id)
        .with_url(&url)
        .with_ipc_handler(move |req| {
            let _ = ipc_proxy.send_event(req.body().clone());
        })
        .with_custom_protocol(format!("app-{APP}"), move |_id, req| {
            let path = req.uri().path().to_string();
            let (status, ct, body) = route(&proto_cfg, req.method().as_str(), &path, req.body(), "app-custom");
            wry::http::Response::builder()
                .status(status)
                .header("Content-Type", ct)
                .body(std::borrow::Cow::Owned(body))
                .unwrap()
        });
    let _webview = builder.build(&window).expect("webview");

    let timeout_proxy = proxy.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(25));
        let _ = timeout_proxy.send_event("{\"timeout\":true}".into());
    });

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::UserEvent(json) => {
                println!("RESULT {json}");
                *flow = ControlFlow::Exit;
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
