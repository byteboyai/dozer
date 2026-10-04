# origin-gateway spike(一次性验证,随时可删)

验证 bytehost"应用宿主"的 gateway 方案:WKWebView(wry 0.55.1,macOS 26.6.2)里,三种"每应用独立 origin"的做法各自能做什么。
设计依据:`docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` §5。

```sh
cd spike/origin-gateway && cargo build && sh run_matrix.sh   # 会短暂弹出几个小窗口;结果写 results.jsonl
```

三种方案:`custom`(wry 自定义协议 `app-excalidraw://localhost/`)、`localhost`(`http://excalidraw.localhost:18765/`)、
`loopback`(`http://127.0.0.1:18765/`)。每种跑 write → read(同存储标识)→ read(另一个存储标识)。
存储标识用 `WebViewBuilderExtDarwin::with_data_store_identifier`。

## 结果(2026-10-04 实测)

| 能力 | custom | `*.localhost` | `127.0.0.1` |
|---|---|---|---|
| fetch GET / POST(同源相对路径) | ✅ | ✅ | ✅ |
| 同源 WebSocket | ❌(`ws error`) | ✅ | ✅ |
| 跨源 WebSocket 到回环服务 | ✅ | — | — |
| SSE(`EventSource`) | ⚠️ 只能一次性给完(wry 应答是整块 `Vec`,没有流式) | ✅(服务端逐条发送) | ✅ |
| `localStorage` / IndexedDB 跨重启 | ✅ | ✅ | ✅ |
| Cookie | ❌(设不上) | ✅ | ✅ |
| Service Worker | ❌(只允许 http/https) | ✅ | ✅ |
| `isSecureContext` / `crypto.subtle` / `navigator.clipboard` | ✅ | ✅ | ✅ |
| 不同 `data_store_identifier` 是否隔离 localStorage/IndexedDB/Cookie | ✅ 隔离 | ✅ 隔离 | ✅ 隔离 |

补充实验(`loopback`,同存储标识):
- 端口从 18765 换成 18766:localStorage、IndexedDB **丢失**(origin 含端口),但 **cookie 还在**(cookie 不区分端口)。

## 没有覆盖的

- 自定义协议的**真流式**应答(SSE 一次性给完只能证明协议层能返回 `text/event-stream`);
- 非 macOS(Windows WebView2、Linux WebKitGTK);
- macOS 14 以下(`with_data_store_identifier` 需要 14+);
- 不带存储标识时的默认数据存储行为;
- CSP、下载、弹窗、`window.open`、剪贴板的**实际读写**(只测了 API 存在);
- 外部浏览器 / DNS rebinding 的真实攻击面(只靠规格里的 Host 校验约束,未做攻击实验)。

## V1:同一端口、多个应用(2026-10-04 实测)

`--app <id>` 让页面跑在 `http://<id>.localhost:<端口>/`;两个应用 `alpha`、`beta` 共用端口 18770、**共用同一个存储标识 31**:

| 操作 | 结果 |
|---|---|
| alpha 写 localStorage/IndexedDB/Cookie(标记 `A-1`) | 写入成功 |
| beta(同端口、同存储标识)读 | 三者**全部为空** |
| alpha 再读 | 三者都是 `A-1` |
| beta 写入 `B-1` 后 alpha 再读 | 仍是 `A-1`(没被覆盖) |
| beta 用另一个存储标识读 | 空 |

结论:**不同 `<id>.localhost` 主机名本身就把 localStorage、IndexedDB、Cookie 隔开了**(origin 不同 + Cookie 是 host-only),`data_store_identifier` 是在此之上额外的一层;两个应用各自的 Service Worker 作用域也各是自己的 origin。同一端口下的 Host 路由在 WKWebView 里没有问题。
