# bytehost A6b:gateway 反向代理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A6 的第二片:gateway 把已通过 Host 与令牌校验的请求,**反向代理**给进程型应用监听的回环端口(`127.0.0.1:<端口>`),含**流式应答**(SSE/长轮询)与 **WebSocket 升级**隧道。**只有机制**:`Gateway::add_upstream(id, addr)` 是新入口,谁来调(`AppManager`/监管线程)是 A6c 的事。

**Architecture:** `gateway/proxy.rs`(新):纯函数(逐跳头剔除、`bh_session` Cookie 剔除、同源检查、`X-Forwarded-*`)+ 异步 `forward`(hyper `client::conn::http1`,连接/响应头各有超时,101 时双向 `copy_bidirectional`)。`gateway/mod.rs`:`State` 多一张 `upstreams` 表;`respond` 拆成 `authenticate` + `route`(返回 `Routed::Reply | Routed::Proxy`),accept 循环的 service 改为异步:路由仍在 `spawn_blocking`(静态文件读盘),代理分支走 `proxy::forward`;`serve_connection` 加 `.with_upgrades()`。认证(Host + 按应用派生的令牌 Cookie)对静态与代理**完全共用同一条路径**,未认证请求永远到不了应用。

**Tech Stack:** Rust、hyper 1(`server`+`client`+`http1`)、hyper-util、http-body-util、tokio;测试用 std 线程写的假上游。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§5 gateway、§6 进程/运行时)。

## A6 切分(见 A6a 计划)

| 片 | 状态 |
|---|---|
| A6a 监管核心 | 已合并(评审的 4 条 Important 另行修复) |
| **A6b(本计划)** | 可执行 |
| A6c 接入 AppManager/dozerd/GUI(调 `add_upstream`、`validate_port_env`) | 待写 |
| A6d 运行时安装 | 待写 |

## Global Constraints

- **认证共用**:Host 校验 → 令牌(按应用派生的 `bh_session` HttpOnly SameSite=Strict Cookie)→ 才路由。代理分支不得有任何绕过认证的入口;未认证请求**永远不到达应用**(测试里用假上游的"收到的请求"记录断言为空)。
- **会话 Cookie 不泄露给应用**:转发时从 `Cookie` 头里剔除 `bh_session`,其余 Cookie 原样转发;`Host` 保持 `<id>.localhost:<端口>` 原样;`X-Forwarded-For/Host/Proto` 由 gateway 设置,客户端自带的一律丢弃。
- **写方法与 WebSocket 升级必须同源**:请求带 `Origin` 且不等于应用自己的 origin → 403,且不连应用。普通 GET/HEAD 不检查。
- **逐跳头不转发**(`Connection`/`Keep-Alive`/`TE`/`Trailer`/`Transfer-Encoding`/`Upgrade`/`Proxy-*` 以及 `Connection` 里列出的头);只有 WebSocket 升级才重新声明 `Connection: Upgrade`/`Upgrade`。
- **应用不可达 502,连得上但迟迟不给响应头 504**(默认连接 3s / 响应头 60s,`Limits` 可调);响应正文开始之后不设总超时(SSE 要能长连)。
- **不加 CSP/缓存头**:进程型应用的出站网络强制等级是 `Advisory`,响应头由应用自己负责。
- 静态站点行为**完全不变**(既有 gateway 测试全部不改地通过);`add_site`/`add_upstream` 互斥(同一应用 id 同时只在一张表里)。
- `bytehost-apps` 默认 feature 依赖不变(只有 serde 家族);hyper 新增的 `client` feature 只在 `server` feature 下。
- 测试里不要用"绑 0 再释放"造肯定关闭的端口以外的断言;假上游一律绑 `127.0.0.1:0` 并持有监听。

## Review Focus

- 请求走私/头注入:`Connection` 头里列出的自定义头、重复的 `Cookie` 头、大小写不同的 `X-Forwarded-*` 是否都被正确处理(单测已钉);上游响应里的 `Transfer-Encoding`/`Connection` 是否被剥掉再交给 hyper 重新编码。
- WebSocket 隧道**没有空闲超时**(与 SSE 同理);客户端断开后上游那条连接是否会被关闭(`copy_bidirectional` 任一侧 EOF 即结束)。
- 同一连接上 `keep_alive(false)` 仍成立:每个应答后关连接,不做上游连接池(**已知局限**:每个请求一次回环 TCP 连接,对本机应用可接受)。
- 只支持 HTTP/1.1 上游(应用若只讲 h2c 会 502)。
- `Origin` 检查依赖浏览器如实发 `Origin`;非浏览器客户端不带 `Origin` 时放行——它们本来就拿不到 `SameSite=Strict` 的 Cookie。

## 文件结构

- 新增 `crates/bytehost-apps/src/gateway/proxy.rs` — 代理机制(含单测)
- 新增 `crates/bytehost-apps/src/gateway/proxy_tests.rs` — 端到端测试(假上游)
- 修改 `crates/bytehost-apps/src/gateway/mod.rs`、`crates/bytehost-apps/Cargo.toml`

---

### Task 1: `proxy.rs` — 头处理与转发机制

**Files:** Create `crates/bytehost-apps/src/gateway/proxy.rs`;Modify `Cargo.toml`(hyper features 加 `client`)、`gateway/mod.rs`(加 `mod proxy;`)。

**Interfaces:**
- Produces:`proxy::Body`、`boxed_reply(Response<Full<Bytes>>) -> Response<Body>`、`forward(req, upstream: SocketAddr, own_origin: &str, cookie_name: &str, connect_timeout, header_timeout) -> Response<Body>`,以及纯函数 `strip_session_cookie`/`origin_allowed`/`upstream_headers`/`client_headers`。

- [ ] **Step 1: 写测试(`proxy.rs` 底部 `mod tests`,代码见下方完整文件)并确认它们因函数不存在而编译失败。**
- [ ] **Step 2: 写实现(完整文件如下)。**

````rust
//! 进程型应用的反向代理(bytehost A6b):gateway 通过了 Host 校验与令牌校验之后,把请求原样转发给应用监听的
//! 回环端口(`127.0.0.1:<端口>`),应答(含**流式**正文,SSE/长轮询)与 **WebSocket 升级**都透传。
//!
//! 与静态站点不同的几点,每一点都是有意的:
//! - **所有方法**都放行(应用有自己的 API),但**写方法与 WebSocket 升级必须同源**:带 `Origin` 头且不是应用自己的
//!   origin 就 403(令牌 Cookie 是 `SameSite=Strict`,这是纵深防御);
//! - **不向应用泄露 gateway 的会话 Cookie**(`bh_session` 只给 gateway 自己看);其余头原样转发,**`Host` 保持原样**
//!   (`<id>.localhost:<端口>`——应用按 Host 生成绝对地址、校验 WebSocket 的 Origin 时才对得上),并补
//!   `X-Forwarded-For/Host/Proto`;
//! - **不加 CSP、不加缓存头**:进程型应用的出站网络强制等级是 `Advisory`(见 `runtime::enforcement_for`),
//!   页面与响应头由应用自己负责;
//! - 应用没起来/崩了返回 **502**,连得上但迟迟不出响应头返回 **504**(正文流开始之后不再有总超时)。

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

pub(super) type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub(super) type Body = BoxBody<Bytes, BoxError>;

/// 把纯应答(`Full`)装进统一的正文类型。
pub(super) fn boxed_reply(r: Response<Full<Bytes>>) -> Response<Body> {
    r.map(|b| b.map_err(|never| match never {}).boxed())
}

fn gateway_error(status: StatusCode, msg: &'static str) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from_static(msg.as_bytes())));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    boxed_reply(r)
}

/// 逐跳头(RFC 7230 §6.1):只对一跳有效,代理不能原样转发。
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.contains(&name.as_str())
}

/// `Connection` 头里点名的额外逐跳头(`Connection: close, X-Foo`)。
fn connection_listed(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// 请求是不是 WebSocket 升级:`Upgrade: websocket` 且 `Connection` 里有 `upgrade`。
pub(super) fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    upgrade && connection_listed(headers).iter().any(|t| t == "upgrade")
}

/// 去掉 Cookie 头里 gateway 自己的会话 Cookie,其余原样保留;剩下什么都没有时返回 `None`(整头不转发)。
pub(super) fn strip_session_cookie(header_value: &str, cookie_name: &str) -> Option<String> {
    let kept: Vec<&str> = header_value
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|p| p.split_once('=').map(|(k, _)| k.trim()) != Some(cookie_name))
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// 写方法与 WebSocket 升级必须同源:有 `Origin` 头就必须等于应用自己的 origin(`null` 与别家都不行);
/// 没有 `Origin`(curl、同源的简单 GET)放行——能走到这里的请求都已经带着令牌 Cookie。
pub(super) fn origin_allowed(
    method: &Method,
    headers: &HeaderMap,
    websocket: bool,
    own_origin: &str,
) -> bool {
    let must_be_same_origin = websocket || (method != Method::GET && method != Method::HEAD);
    if !must_be_same_origin {
        return true;
    }
    match headers.get(header::ORIGIN) {
        None => true,
        Some(v) => v.to_str().is_ok_and(|o| o == own_origin),
    }
}

/// 给应用的请求头:去逐跳头、去会话 Cookie,补 `X-Forwarded-*`;WebSocket 升级时显式带回 `Upgrade`/`Connection`。
pub(super) fn upstream_headers(src: &HeaderMap, cookie_name: &str, websocket: bool) -> HeaderMap {
    let listed = connection_listed(src);
    let mut out = HeaderMap::new();
    for (name, value) in src {
        if is_hop_by_hop(name)
            || listed.iter().any(|t| t == name.as_str())
            || name == header::COOKIE
            || name.as_str().starts_with("x-forwarded-")
        {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    let cookies: Vec<String> = src
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| strip_session_cookie(v, cookie_name))
        .collect();
    if !cookies.is_empty()
        && let Ok(v) = HeaderValue::from_str(&cookies.join("; "))
    {
        out.insert(header::COOKIE, v);
    }
    out.insert("x-forwarded-for", HeaderValue::from_static("127.0.0.1"));
    out.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    if let Some(host) = src.get(header::HOST) {
        out.insert("x-forwarded-host", host.clone());
    }
    if websocket {
        out.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        out.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    }
    out
}

/// 给浏览器的应答头:去逐跳头(`Connection`/`Transfer-Encoding` 等由 hyper 自己管);WebSocket 101 保留 `Upgrade`/`Connection`。
fn client_headers(src: &HeaderMap, keep_upgrade: bool) -> HeaderMap {
    let listed = connection_listed(src);
    let mut out = HeaderMap::new();
    for (name, value) in src {
        let upgrade_header = name == header::UPGRADE || name == header::CONNECTION;
        if (is_hop_by_hop(name) && !(keep_upgrade && upgrade_header))
            || (!keep_upgrade && listed.iter().any(|t| t == name.as_str()))
        {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    out
}

/// 转发一个已经通过 gateway 校验的请求。
pub(super) async fn forward(
    req: Request<Incoming>,
    upstream: SocketAddr,
    own_origin: &str,
    cookie_name: &str,
    connect_timeout: Duration,
    header_timeout: Duration,
) -> Response<Body> {
    let websocket = is_websocket_upgrade(req.headers());
    if !origin_allowed(req.method(), req.headers(), websocket, own_origin) {
        return gateway_error(StatusCode::FORBIDDEN, "cross-origin request refused");
    }
    let mut req = req;
    let on_client_upgrade = websocket.then(|| hyper::upgrade::on(&mut req));
    let (parts, body) = req.into_parts();

    let mut builder = Request::builder().method(parts.method.clone()).uri(
        parts
            .uri
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/"),
    );
    *builder.headers_mut().expect("builder 没出错") =
        upstream_headers(&parts.headers, cookie_name, websocket);
    let Ok(up_req) = builder.body(body.map_err(|e| Box::new(e) as BoxError).boxed()) else {
        return gateway_error(StatusCode::BAD_REQUEST, "bad request");
    };

    let stream = match tokio::time::timeout(connect_timeout, TcpStream::connect(upstream)).await {
        Ok(Ok(s)) => s,
        _ => return gateway_error(StatusCode::BAD_GATEWAY, "application is not reachable"),
    };
    let Ok((mut sender, conn)) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
    else {
        return gateway_error(StatusCode::BAD_GATEWAY, "application is not reachable");
    };
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
    });
    let mut resp = match tokio::time::timeout(header_timeout, sender.send_request(up_req)).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            return gateway_error(StatusCode::BAD_GATEWAY, "application closed the connection");
        }
        Err(_) => {
            return gateway_error(
                StatusCode::GATEWAY_TIMEOUT,
                "application did not answer in time",
            );
        }
    };

    if websocket
        && resp.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(on_client) = on_client_upgrade
    {
        let on_upstream = hyper::upgrade::on(&mut resp);
        tokio::spawn(async move {
            if let (Ok(client), Ok(app)) = tokio::join!(on_client, on_upstream) {
                let (mut client, mut app) = (TokioIo::new(client), TokioIo::new(app));
                let _ = tokio::io::copy_bidirectional(&mut client, &mut app).await;
            }
        });
        let mut out = Response::new(Full::new(Bytes::new()).map_err(|n| match n {}).boxed());
        *out.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
        *out.headers_mut() = client_headers(resp.headers(), true);
        return out;
    }

    let (rparts, rbody) = resp.into_parts();
    let mut out = Response::new(rbody.map_err(|e| Box::new(e) as BoxError).boxed());
    *out.status_mut() = rparts.status;
    *out.headers_mut() = client_headers(&rparts.headers, false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn the_gateway_session_cookie_never_reaches_the_application() {
        assert_eq!(
            strip_session_cookie("a=1; bh_session=SECRET; b=2", "bh_session").as_deref(),
            Some("a=1; b=2")
        );
        assert_eq!(
            strip_session_cookie("bh_session=SECRET", "bh_session"),
            None
        );
        assert_eq!(strip_session_cookie("", "bh_session"), None);
        // 同名 Cookie 出现多次都要去掉;名字相近的不能误伤
        assert_eq!(
            strip_session_cookie("bh_session=A; bh_session2=keep; bh_session=B", "bh_session")
                .as_deref(),
            Some("bh_session2=keep")
        );
    }

    #[test]
    fn hop_by_hop_headers_and_connection_listed_headers_are_not_forwarded() {
        let src = headers(&[
            ("host", "demo.localhost:24000"),
            ("connection", "keep-alive, x-secret-hop"),
            ("keep-alive", "timeout=5"),
            ("x-secret-hop", "1"),
            ("te", "trailers"),
            ("transfer-encoding", "chunked"),
            ("accept", "text/html"),
            ("x-custom", "kept"),
        ]);
        let out = upstream_headers(&src, "bh_session", false);
        for gone in [
            "connection",
            "keep-alive",
            "x-secret-hop",
            "te",
            "transfer-encoding",
        ] {
            assert!(!out.contains_key(gone), "{gone}");
        }
        assert_eq!(out["accept"], "text/html");
        assert_eq!(out["x-custom"], "kept");
        assert_eq!(out["host"], "demo.localhost:24000", "Host 保持原样");
    }

    #[test]
    fn forwarded_headers_are_set_by_the_gateway_not_the_client() {
        let src = headers(&[
            ("host", "demo.localhost:24000"),
            ("x-forwarded-for", "6.6.6.6"),
            ("x-forwarded-host", "evil.example"),
            ("x-forwarded-proto", "https"),
        ]);
        let out = upstream_headers(&src, "bh_session", false);
        assert_eq!(out["x-forwarded-for"], "127.0.0.1");
        assert_eq!(out["x-forwarded-host"], "demo.localhost:24000");
        assert_eq!(out["x-forwarded-proto"], "http");
        assert_eq!(out.get_all("x-forwarded-for").iter().count(), 1);
    }

    #[test]
    fn cookies_are_forwarded_minus_the_session_cookie() {
        let src = headers(&[
            ("cookie", "theme=dark; bh_session=SECRET"),
            ("cookie", "bh_session=ALSO"),
        ]);
        let out = upstream_headers(&src, "bh_session", false);
        assert_eq!(out["cookie"], "theme=dark");
        let only = upstream_headers(
            &headers(&[("cookie", "bh_session=SECRET")]),
            "bh_session",
            false,
        );
        assert!(!only.contains_key("cookie"));
    }

    #[test]
    fn websocket_upgrades_are_detected_precisely_and_re_asserted_upstream() {
        assert!(is_websocket_upgrade(&headers(&[
            ("upgrade", "WebSocket"),
            ("connection", "keep-alive, Upgrade")
        ])));
        assert!(
            !is_websocket_upgrade(&headers(&[("upgrade", "websocket")])),
            "没有 Connection: upgrade"
        );
        assert!(!is_websocket_upgrade(&headers(&[
            ("upgrade", "h2c"),
            ("connection", "upgrade")
        ])));
        assert!(!is_websocket_upgrade(&headers(&[(
            "connection",
            "upgrade"
        )])));
        let src = headers(&[
            ("upgrade", "websocket"),
            ("connection", "Upgrade"),
            ("sec-websocket-key", "k"),
        ]);
        let out = upstream_headers(&src, "bh_session", true);
        assert_eq!(out["upgrade"], "websocket");
        assert_eq!(out["connection"], "Upgrade");
        assert_eq!(out["sec-websocket-key"], "k");
        assert!(!upstream_headers(&src, "bh_session", false).contains_key("upgrade"));
    }

    #[test]
    fn writes_and_websockets_must_be_same_origin_but_plain_reads_are_not_checked() {
        let own = "http://demo.localhost:24000";
        let evil = headers(&[("origin", "http://evil.example")]);
        let same = headers(&[("origin", own)]);
        let null = headers(&[("origin", "null")]);
        let none = HeaderMap::new();
        for m in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            assert!(!origin_allowed(&m, &evil, false, own), "{m}");
            assert!(!origin_allowed(&m, &null, false, own), "{m} null");
            assert!(origin_allowed(&m, &same, false, own), "{m}");
            assert!(origin_allowed(&m, &none, false, own), "{m} 无 Origin");
        }
        assert!(
            !origin_allowed(&Method::GET, &evil, true, own),
            "跨源 WebSocket 升级"
        );
        assert!(origin_allowed(&Method::GET, &same, true, own));
        assert!(
            origin_allowed(&Method::GET, &evil, false, own),
            "普通 GET 不查 Origin"
        );
        assert!(origin_allowed(&Method::HEAD, &evil, false, own));
    }

    #[test]
    fn client_headers_drop_hop_by_hop_but_a_101_keeps_the_upgrade_pair() {
        let src = headers(&[
            ("connection", "Upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-accept", "x"),
            ("transfer-encoding", "chunked"),
            ("content-type", "text/plain"),
        ]);
        let normal = client_headers(&src, false);
        assert!(
            !normal.contains_key("connection")
                && !normal.contains_key("upgrade")
                && !normal.contains_key("transfer-encoding")
        );
        assert_eq!(normal["content-type"], "text/plain");
        let ws = client_headers(&src, true);
        assert_eq!(ws["connection"], "Upgrade");
        assert_eq!(ws["upgrade"], "websocket");
        assert_eq!(ws["sec-websocket-accept"], "x");
        assert!(!ws.contains_key("transfer-encoding"));
    }
}
````

- [ ] **Step 3: 运行 `cargo test -p bytehost-apps --all-features gateway::proxy::` — Expected:7 个单测通过。**
- [ ] **Step 4: Commit** `feat(bytehost-apps): gateway proxy mechanism — header hygiene, same-origin check, forward with upgrades (A6b task 1)`

### Task 2: gateway 接线与端到端测试

**Files:** Modify `gateway/mod.rs`;Create `gateway/proxy_tests.rs`。

**Interfaces:**
- Consumes:Task 1 的 `proxy::{forward, boxed_reply}`。
- Produces:`Gateway::add_upstream(&AppId, SocketAddr)`;`Limits.upstream_connect_timeout`/`upstream_header_timeout`;`remove_site`/`has_site` 同时覆盖两张表。

- [ ] **Step 1: 先写 `proxy_tests.rs`(完整文件如下)并在 `mod.rs` 的测试段前加 `#[cfg(test)] mod proxy_tests;`;运行 `cargo test -p bytehost-apps --all-features gateway::proxy_tests` — Expected:编译失败(`add_upstream` 不存在)。**

````rust
//! 反向代理的端到端测试:真 gateway + 一个手写的假上游(std 线程),全部走回环。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

/// 假上游:把收到的每个请求的原始头部记下来;按路径给出不同行为。
struct Upstream {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

fn read_head(s: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut b).ok()? == 0 {
            return None;
        }
        buf.push(b[0]);
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let len = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    Some((head, body))
}

fn spawn_upstream() -> Upstream {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { return };
            let seen = seen2.clone();
            std::thread::spawn(move || {
                let Some((head, body)) = read_head(&mut s) else {
                    return;
                };
                seen.lock().unwrap().push(head.clone());
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                match path.as_str() {
                    "/echo" => {
                        let payload =
                            format!("{head}\n--body--\n{}", String::from_utf8_lossy(&body));
                        let _ = write!(
                            s,
                            "HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nX-App: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                            payload.len()
                        );
                    }
                    "/sse" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n8\r\ndata: 1\n\r\n");
                        let _ = s.flush();
                        std::thread::sleep(Duration::from_millis(700));
                        let _ = s.write_all(b"8\r\ndata: 2\n\r\n0\r\n\r\n");
                    }
                    "/ws" => {
                        let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: abc\r\n\r\n");
                        let mut b = [0u8; 64];
                        while let Ok(n) = s.read(&mut b) {
                            if n == 0 || s.write_all(&b[..n]).is_err() {
                                break;
                            }
                        }
                    }
                    "/slow" => std::thread::sleep(Duration::from_secs(3)),
                    _ => {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
    Upstream { addr, seen }
}

struct Fixture {
    gw: Gateway,
    host: String,
    cookie: String,
    port: u16,
}

async fn fixture(up: Option<SocketAddr>, limits: Limits) -> Fixture {
    let gw = Gateway::start_with_limits(GatewayConfig { port: 0 }, limits)
        .await
        .unwrap();
    let id = AppId::new("pyapp").unwrap();
    gw.add_upstream(&id, up.unwrap_or_else(|| "127.0.0.1:1".parse().unwrap()));
    let port = gw.port();
    Fixture {
        host: format!("pyapp.localhost:{port}"),
        cookie: format!("bh_session={}", app_token(&gw.state.token, &id)),
        gw,
        port,
    }
}

/// 发一个原始请求(带 Host 与会话 Cookie),读到连接关闭。
fn exchange(f: &Fixture, req_line: &str, extra: &str, body: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", f.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "{req_line} HTTP/1.1\r\nHost: {}\r\nCookie: {}; theme=dark\r\n{extra}Content-Length: {}\r\n\r\n{body}", f.host, f.cookie, body.len()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    String::from_utf8_lossy(&out).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_is_forwarded_with_its_body_and_the_apps_status_and_headers_come_back() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    let resp = exchange(
        &f,
        "POST /echo",
        "Content-Type: application/json\r\n",
        "{\"a\":1}",
    );
    assert!(resp.starts_with("HTTP/1.1 201"), "{resp}");
    assert!(resp.to_ascii_lowercase().contains("x-app: yes"));
    assert!(resp.contains("{\"a\":1}"), "正文要到达应用");
    let seen = up.seen.lock().unwrap()[0].to_ascii_lowercase();
    assert!(
        seen.contains(&format!("host: {}", f.host)),
        "Host 保持原样: {seen}"
    );
    assert!(
        seen.contains("x-forwarded-proto: http") && seen.contains("x-forwarded-for: 127.0.0.1")
            || seen.contains("x-forwarded-for"),
        "{seen}"
    );
    assert!(
        !seen.contains("bh_session"),
        "会话 Cookie 不能泄露给应用: {seen}"
    );
    assert!(seen.contains("theme=dark"), "其他 Cookie 原样转发: {seen}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthenticated_requests_never_reach_the_app() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    let mut s = TcpStream::connect(("127.0.0.1", f.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET /echo HTTP/1.1\r\nHost: {}\r\n\r\n", f.host).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    let mut s = TcpStream::connect(("127.0.0.1", f.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET /echo HTTP/1.1\r\nHost: pyapp.localhost:{}\r\nCookie: bh_session=wrong\r\n\r\n",
        f.port
    )
    .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    assert!(up.seen.lock().unwrap().is_empty(), "未认证请求不得到达应用");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cross_origin_write_is_refused_before_it_reaches_the_app() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    let resp = exchange(&f, "POST /echo", "Origin: http://evil.example\r\n", "x");
    assert!(resp.starts_with("HTTP/1.1 403"), "{resp}");
    assert!(up.seen.lock().unwrap().is_empty());
    let same = format!("Origin: http://{}\r\n", f.host);
    assert!(exchange(&f, "POST /echo", &same, "x").starts_with("HTTP/1.1 201"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_response_reaches_the_client_before_the_app_finishes() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    let mut s = TcpStream::connect(("127.0.0.1", f.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET /sse HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\n\r\n",
        f.host, f.cookie
    )
    .unwrap();
    let start = Instant::now();
    let mut got = Vec::new();
    let mut b = [0u8; 256];
    while !String::from_utf8_lossy(&got).contains("data: 1") {
        let n = s.read(&mut b).unwrap();
        assert!(n > 0, "连接过早关闭");
        got.extend_from_slice(&b[..n]);
    }
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "第一块应立即到达,而不是等应用结束: {:?}",
        start.elapsed()
    );
    let mut rest = Vec::new();
    let _ = s.read_to_end(&mut rest);
    assert!(String::from_utf8_lossy(&rest).contains("data: 2"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_websocket_upgrade_becomes_a_two_way_tunnel() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    let mut s = TcpStream::connect(("127.0.0.1", f.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET /ws HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: k\r\nSec-WebSocket-Version: 13\r\nOrigin: http://{}\r\n\r\n", f.host, f.cookie, f.host).unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(s.read(&mut b).unwrap(), 1);
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
    assert!(
        head.starts_with("http/1.1 101") && head.contains("sec-websocket-accept: abc"),
        "{head}"
    );
    s.write_all(b"ping-frame").unwrap();
    let mut echo = [0u8; 10];
    s.read_exact(&mut echo).unwrap();
    assert_eq!(&echo, b"ping-frame");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cross_origin_websocket_upgrade_is_refused() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    let resp = exchange(
        &f,
        "GET /ws",
        "Connection: Upgrade\r\nUpgrade: websocket\r\nOrigin: http://evil.example\r\n",
        "",
    );
    assert!(resp.starts_with("HTTP/1.1 403"), "{resp}");
    assert!(up.seen.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_upstream_is_502_and_a_silent_one_is_504() {
    // 502:端口上没人监听
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    let f = fixture(Some(dead_addr), Limits::default()).await;
    assert!(exchange(&f, "GET /echo", "", "").starts_with("HTTP/1.1 502"));
    // 504:连得上但不给响应头
    let up = spawn_upstream();
    let limits = Limits {
        upstream_header_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let f = fixture(Some(up.addr), limits).await;
    let start = Instant::now();
    assert!(exchange(&f, "GET /slow", "", "").starts_with("HTTP/1.1 504"));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_the_app_stops_proxying_to_it() {
    let up = spawn_upstream();
    let f = fixture(Some(up.addr), Limits::default()).await;
    assert!(f.gw.remove_site(&AppId::new("pyapp").unwrap()));
    assert!(exchange(&f, "GET /echo", "", "").starts_with("HTTP/1.1 404"));
    assert!(up.seen.lock().unwrap().is_empty());
}
````

- [ ] **Step 2: 按下面的 diff 修改 `Cargo.toml` 与 `gateway/mod.rs`(这是原型上的真实改动;`Routed`/`Authed`/`authenticate`/`route`/`serve_static` 取代原来的 `respond`,保留一个 `#[cfg(test)] respond` 包装让既有单测不改)。**

````diff
diff --git a/crates/bytehost-apps/Cargo.toml b/crates/bytehost-apps/Cargo.toml
index 1467be63..92a77655 100644
--- a/crates/bytehost-apps/Cargo.toml
+++ b/crates/bytehost-apps/Cargo.toml
@@ -33,7 +33,7 @@ serde_json.workspace = true
 toml = { version = "0.8", optional = true }
 sha2 = { version = "0.10", optional = true }
 tokio = { workspace = true, optional = true }
-hyper = { version = "1", optional = true, features = ["server", "http1"] }
+hyper = { version = "1", optional = true, features = ["server", "client", "http1"] }
 hyper-util = { version = "0.1", optional = true, features = ["tokio"] }
 http-body-util = { version = "0.1", optional = true }
 bytes = { version = "1", optional = true }
diff --git a/crates/bytehost-apps/src/gateway/mod.rs b/crates/bytehost-apps/src/gateway/mod.rs
index c76fadce..d1954dc7 100644
--- a/crates/bytehost-apps/src/gateway/mod.rs
+++ b/crates/bytehost-apps/src/gateway/mod.rs
@@ -11,10 +11,12 @@
 //! 4. **方法**:只允许 GET/HEAD;
 //! 5. **路径解析**(`static_files::resolve`):任何写法都不能走出站点根目录。
 
+mod proxy;
 mod static_files;
 
 use std::collections::HashMap;
 use std::convert::Infallible;
+use std::net::SocketAddr;
 use std::path::{Path, PathBuf};
 use std::sync::{Arc, RwLock};
 
@@ -65,6 +67,10 @@ pub struct Limits {
     pub header_read_timeout: std::time::Duration,
     /// 同时处理的连接数上限;超出的连接被立即丢弃。
     pub max_connections: usize,
+    /// 反向代理:连应用的回环端口最多等多久(超时 502)。
+    pub upstream_connect_timeout: std::time::Duration,
+    /// 反向代理:连上之后等应用给出**响应头**最多多久(超时 504);响应正文开始流动之后不再有总超时(SSE/长轮询)。
+    pub upstream_header_timeout: std::time::Duration,
 }
 
 impl Default for Limits {
@@ -72,6 +78,8 @@ impl Default for Limits {
         Self {
             header_read_timeout: std::time::Duration::from_secs(10),
             max_connections: 128,
+            upstream_connect_timeout: std::time::Duration::from_secs(3),
+            upstream_header_timeout: std::time::Duration::from_secs(60),
         }
     }
 }
@@ -93,9 +101,23 @@ struct State {
     port: u16,
     token: String,
     sites: RwLock<HashMap<String, Site>>,
+    /// 进程型应用:`<app-id>` → 应用监听的回环地址(反向代理的目标)。与 `sites` 互斥(同一个应用只有一种)。
+    upstreams: RwLock<HashMap<String, SocketAddr>>,
 }
 
 impl State {
+    fn upstreams_read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, SocketAddr>> {
+        self.upstreams
+            .read()
+            .unwrap_or_else(std::sync::PoisonError::into_inner)
+    }
+
+    fn upstreams_write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, SocketAddr>> {
+        self.upstreams
+            .write()
+            .unwrap_or_else(std::sync::PoisonError::into_inner)
+    }
+
     /// 站点表的读/写锁:持锁线程 panic 会毒化它,但表里没有需要保持的不变量,
     /// 后续请求不能因此全部 panic——取回内部数据继续用。
     fn sites_read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Site>> {
@@ -172,6 +194,7 @@ impl Gateway {
                 uuid::Uuid::new_v4().simple()
             ),
             sites: RwLock::new(HashMap::new()),
+            upstreams: RwLock::new(HashMap::new()),
         });
         let shutdown = Arc::new(Notify::new());
         let task = tokio::spawn(accept_loop(
@@ -193,14 +216,26 @@ impl Gateway {
 
     /// 注册(或替换)一个应用的静态站点。
     pub fn add_site(&self, id: &AppId, root: PathBuf, csp: Option<String>) {
+        self.state.upstreams_write().remove(id.as_str());
         self.state
             .sites_write()
             .insert(id.as_str().to_string(), Site { root, csp });
     }
 
-    /// 注销站点;返回之前是否注册过。
+    /// 注册(或替换)一个进程型应用:请求通过校验后反向代理到 `upstream`(应用监听的 `127.0.0.1:<端口>`)。
+    /// 若这个 id 之前是静态站点,会被替换。
+    pub fn add_upstream(&self, id: &AppId, upstream: SocketAddr) {
+        self.state.sites_write().remove(id.as_str());
+        self.state
+            .upstreams_write()
+            .insert(id.as_str().to_string(), upstream);
+    }
+
+    /// 注销站点或上游;返回之前是否注册过。
     pub fn remove_site(&self, id: &AppId) -> bool {
-        self.state.sites_write().remove(id.as_str()).is_some()
+        let was_static = self.state.sites_write().remove(id.as_str()).is_some();
+        let was_proxy = self.state.upstreams_write().remove(id.as_str()).is_some();
+        was_static || was_proxy
     }
 
     /// `stop`/`shutdown` 已经完成(accept 循环已退出)。测试与上层用它判断"这个 gateway 确实停了",
@@ -213,11 +248,8 @@ impl Gateway {
     }
 
     pub fn has_site(&self, id: &AppId) -> bool {
-        self.state
-            .sites
-            .read()
-            .expect("sites 锁")
-            .contains_key(id.as_str())
+        self.state.sites_read().contains_key(id.as_str())
+            || self.state.upstreams_read().contains_key(id.as_str())
     }
 
     /// 不含令牌的站点地址(可以放进事件/日志)。
@@ -279,7 +311,10 @@ async fn accept_loop(
                                 req.headers().get_all(header::HOST).iter().count(),
                                 req.uri().authority().is_some(),
                             ) {
-                                return Ok::<_, Infallible>(plain(status, "bad request"));
+                                return Ok::<_, Infallible>(proxy::boxed_reply(plain(
+                                    status,
+                                    "bad request",
+                                )));
                             }
                             let method = req.method().clone();
                             let host = header_str(&req, header::HOST);
@@ -289,11 +324,39 @@ async fn accept_loop(
                                 .path_and_query()
                                 .map(|p| p.as_str().to_string())
                                 .unwrap_or_else(|| "/".to_string());
-                            let reply = tokio::task::spawn_blocking(move || {
-                                respond(&state, &method, host.as_deref(), &target, cookie.as_deref())
+                            let routing_state = state.clone();
+                            let routed = tokio::task::spawn_blocking(move || {
+                                route(
+                                    &routing_state,
+                                    &method,
+                                    host.as_deref(),
+                                    &target,
+                                    cookie.as_deref(),
+                                )
                             })
                             .await
-                            .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "internal error"));
+                            .unwrap_or_else(|_| {
+                                Routed::Reply(plain(
+                                    StatusCode::INTERNAL_SERVER_ERROR,
+                                    "internal error",
+                                ))
+                            });
+                            let reply = match routed {
+                                Routed::Reply(r) => proxy::boxed_reply(r),
+                                Routed::Proxy(upstream) => {
+                                    let own_origin =
+                                        format!("{}://{}", "http", header_str(&req, header::HOST).unwrap_or_default());
+                                    proxy::forward(
+                                        req,
+                                        upstream,
+                                        &own_origin,
+                                        COOKIE_NAME,
+                                        limits.upstream_connect_timeout,
+                                        limits.upstream_header_timeout,
+                                    )
+                                    .await
+                                }
+                            };
                             Ok::<_, Infallible>(reply)
                         }
                     });
@@ -302,7 +365,10 @@ async fn accept_loop(
                     http.timer(hyper_util::rt::TokioTimer::new())
                         .header_read_timeout(limits.header_read_timeout)
                         .keep_alive(false);
-                    let _ = http.serve_connection(TokioIo::new(stream), service).await;
+                    let _ = http
+                        .serve_connection(TokioIo::new(stream), service)
+                        .with_upgrades()
+                        .await;
                 });
             }
         }
@@ -382,17 +448,36 @@ fn split_token(query: Option<&str>) -> (Option<String>, Option<String>) {
     (token, (!rest.is_empty()).then(|| rest.join("&")))
 }
 
-/// 处理一个请求。纯同步、不碰网络,便于直接测试;`target` 是请求行里的 path+query。
-fn respond(
+/// 认证之后的去向。
+enum Routed {
+    /// 直接答复(静态文件、重定向、各种错误)。
+    Reply(Reply),
+    /// 反向代理到进程型应用监听的回环地址(含 WebSocket 升级)。
+    Proxy(SocketAddr),
+}
+
+/// 步骤 1–2:Host 与令牌。要么直接得到一个答复(421/403/302),要么得到通过校验的应用与拆好的路径/查询。
+enum Authed<'a> {
+    Done(Reply),
+    App {
+        app: AppId,
+        path: &'a str,
+        query: Option<&'a str>,
+    },
+}
+
+fn authenticate<'a>(
     state: &State,
-    method: &Method,
     host: Option<&str>,
-    target: &str,
+    target: &'a str,
     cookie: Option<&str>,
-) -> Reply {
+) -> Authed<'a> {
     // 1. Host
     let Some(app) = app_from_host(host, state.port) else {
-        return plain(StatusCode::MISDIRECTED_REQUEST, "misdirected request");
+        return Authed::Done(plain(
+            StatusCode::MISDIRECTED_REQUEST,
+            "misdirected request",
+        ));
     };
     let (path, query) = match target.split_once('?') {
         Some((p, q)) => (p, Some(q)),
@@ -403,7 +488,7 @@ fn respond(
     let (query_token, rest_query) = split_token(query);
     if let Some(t) = &query_token {
         if !ct_eq(t, &expected) {
-            return plain(StatusCode::FORBIDDEN, "forbidden");
+            return Authed::Done(plain(StatusCode::FORBIDDEN, "forbidden"));
         }
         // Location 永远是站内路径:开头的多个 `/`、`\\` 折成一个(`//evil.com/` 会被浏览器当成另一个 origin)
         let local_path = format!("/{}", path.trim_start_matches(['/', '\\']));
@@ -425,13 +510,42 @@ fn respond(
             .expect("令牌是十六进制"),
         );
         harden(r.headers_mut());
-        return r;
+        return Authed::Done(r);
     }
     let authed = cookie.is_some_and(|c| cookie_values(c, COOKIE_NAME).any(|v| ct_eq(v, &expected)));
     if !authed {
-        return plain(StatusCode::FORBIDDEN, "forbidden");
+        return Authed::Done(plain(StatusCode::FORBIDDEN, "forbidden"));
+    }
+    Authed::App { app, path, query }
+}
+
+/// 处理一个请求:纯同步、不碰网络(反向代理的转发在 `proxy::forward`,这里只决定"去哪");`target` 是请求行里的 path+query。
+fn route(
+    state: &State,
+    method: &Method,
+    host: Option<&str>,
+    target: &str,
+    cookie: Option<&str>,
+) -> Routed {
+    let (app, path, query) = match authenticate(state, host, target, cookie) {
+        Authed::Done(reply) => return Routed::Reply(reply),
+        Authed::App { app, path, query } => (app, path, query),
+    };
+    // 3. 站点:进程型应用走反向代理(不限方法——应用有自己的 API;写方法/WebSocket 的同源检查在 `proxy::forward`)
+    if let Some(addr) = state.upstreams_read().get(app.as_str()).copied() {
+        return Routed::Proxy(addr);
     }
-    // 3. 站点
+    Routed::Reply(serve_static(state, &app, method, path, query))
+}
+
+/// 静态站点的步骤 3–5:站点存在、方法、路径解析。
+fn serve_static(
+    state: &State,
+    app: &AppId,
+    method: &Method,
+    path: &str,
+    query: Option<&str>,
+) -> Reply {
     let Some(site) = state.sites_read().get(app.as_str()).cloned() else {
         return plain(StatusCode::NOT_FOUND, "no such app");
     };
@@ -466,6 +580,21 @@ fn respond(
     }
 }
 
+/// 测试辅助:只看直接答复(静态路径与各种错误);进程型应用的路由结果在测试里单独断言。
+#[cfg(test)]
+fn respond(
+    state: &State,
+    method: &Method,
+    host: Option<&str>,
+    target: &str,
+    cookie: Option<&str>,
+) -> Reply {
+    match route(state, method, host, target, cookie) {
+        Routed::Reply(r) => r,
+        Routed::Proxy(_) => plain(StatusCode::BAD_GATEWAY, "proxied"),
+    }
+}
+
 fn serve_file(file: &Path, site: &Site, head_only: bool) -> Reply {
     let Ok(bytes) = std::fs::read(file) else {
         return plain(StatusCode::NOT_FOUND, "not found");
@@ -500,6 +629,9 @@ fn serve_file(file: &Path, site: &Site, head_only: bool) -> Reply {
     r
 }
 
+#[cfg(test)]
+mod proxy_tests;
+
 #[cfg(test)]
 mod tests {
     use super::*;
@@ -523,6 +655,7 @@ mod tests {
             port,
             token: TOKEN.to_string(),
             sites: RwLock::new(sites),
+            upstreams: RwLock::new(HashMap::new()),
         }
     }
 
@@ -986,6 +1119,7 @@ mod tests {
         let limits = Limits {
             header_read_timeout: Duration::from_millis(300),
             max_connections: 64,
+            ..Limits::default()
         };
         let gw = Gateway::start_with_limits(GatewayConfig { port: 0 }, limits)
             .await
@@ -1020,6 +1154,7 @@ mod tests {
         let limits = Limits {
             header_read_timeout: Duration::from_secs(10),
             max_connections: 2,
+            ..Limits::default()
         };
         let gw = Gateway::start_with_limits(GatewayConfig { port: 0 }, limits)
             .await
````

- [ ] **Step 3: 运行 `cargo test -p bytehost-apps --all-features gateway` — Expected:45 通过(30 个既有 + 7 个 `proxy::tests` + 8 个 `proxy_tests` 端到端)。**
- [ ] **Step 4: 门禁** — `cargo fmt --check -p bytehost-apps`;`cargo clippy -p bytehost-apps --all-targets -- -D warnings` 与加 `--all-features` 各一遍;`bash scripts/check-bytehost-apps-deps.sh`;`cargo test -p bytehost-apps --all-features` 全绿(Expected:201 通过);`gateway` 连跑 6 遍稳定。
- [ ] **Step 5: 变异检查(串行,从快照还原)** — 逐个应当被测试杀死:去掉同源检查、不剔除会话 Cookie、不剔除逐跳头、响应头超时改成 30s、`route` 不返回 `Proxy`、去掉 `.with_upgrades()`。(`x-forwarded-for` 的 `insert→append` 是等价变异体:前面已丢弃客户端自带的,不必杀。)
- [ ] **Step 6: Commit** `feat(bytehost-apps): gateway reverse proxy for process apps — upstream registry, async service, ws tunnel (A6b task 2)`

## 已知局限(A6c 及以后)

- 无上游连接池;仅 HTTP/1.1;WebSocket/SSE 无空闲超时;`add_upstream` 无人调用,A6c 接线(应用 Running 且健康后 `add_upstream`,停止/失败时 `remove_site`)。
- 代理分支不限制请求/响应体大小(本机应用,且已认证);如要限制放 A6c。
- `own_origin` 用请求自带的 `Host`(已通过 Host 校验,等于 `<id>.localhost:<端口>`)拼成 `http://…`。
