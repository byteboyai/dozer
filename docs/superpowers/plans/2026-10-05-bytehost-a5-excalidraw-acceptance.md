# bytehost A5:Excalidraw 端到端验收(含 V2 验证) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成规格 §7 的 A5:用**真实 Excalidraw 静态构建**把"安装 → 审批 → 图标栏出现 → 打开 → 画 → 退出重开仍在 → 卸载"这条线跑通并留下可复查的证据。动手前先做了 V2 验证(下面的"V2 实测结论"),它暴露了三件会让验收直接失败的事,本计划把它们变成任务:**打包配方**(否则字体全坏)、**卸载"含数据"清 WebView 存储**(否则画还在)、**验收 8 的重新表述**(一期没有 container runtime)。

**Architecture:** 不改 gateway/协议。新增:(1) 打包脚本把官方 Docker 镜像里的 Excalidraw 构建改造成能在严格 CSP 下运行的应用目录;(2) 一次性 V2 探针(`spike/v2-excalidraw`)——真实 gateway + 受限 wry webview,可重复跑;(3) dozer-app 里 `StoreRemovals`:卸载"连数据一起删"后,等该应用的 webview 离开池,再调 wry 的 `WebView::remove_data_store`;(4) 验收报告。

**Tech Stack:** Python 3(打包)、docker(只用来取文件)、Rust(spike + dozer-app)、wry 0.55。**dozer-app/bytehost-apps 不新增依赖**;spike 自带 `[workspace]`。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` §7(验收 1–8)、V2(§5 前置验证)。

## V2 实测结论(2026-10-05,在本机真实跑出来的,不是推测)

测试对象:`excalidraw/excalidraw:latest`(摘要 `sha256:f7ee194addd607bf831d2af0f0a34463dd4225e426cf35199ef0b12a803398e9`)的 `/usr/share/nginx/html`,45MB,根路径绝对资源(`/assets/…`),所以放在 `<id>.localhost:端口/` 站点根目录**不需要处理 base path**。放进**真实 gateway**(按 `network.outbound = none` 算出的 CSP)、受限 wry webview(A4b1 同款:拒下载、拒新窗口、仅本 origin 导航)。

| 项 | 原样构建 | 打包后(Task 1) |
|---|---|---|
| 渲染 | ✅ 画布出现 | ✅ |
| `index.html` 的 5 个内联 `<script>` | ❌ 全被 `script-src 'self'` 拦截(主题初始化、`EXCALIDRAW_ASSET_PATH` 都没执行) | ✅ 外置成同序的 `/bh-inline-N.js`;统计脚本/Excalidraw+ 跳转不要 |
| `EXCALIDRAW_ASSET_PATH` | ❌ `undefined` → 字体走 CDN → **246 次 font-src 违规,Excalifont/Virgil/Nunito 全部加载失败** | ✅ `["/"]`,字体从本站取 |
| UI 字体 Assistant | ❌ 构建后的 CSS 把 4 个字重写死指向官方 CDN;静态构建里只带了 Regular | ✅ 打包时联网补下另外 3 个字重并改写 CSS(下载失败退到 Regular,由浏览器合成字重) |
| 绘图字体加载(`document.fonts.load`) | ❌ | ✅ Excalifont、Virgil、Cascadia、Nunito、Comic Shanns、Assistant、Lilita One、Liberation Sans 均 `loaded`(中文 Xiaolai 按需分片加载,探针里 0 faces 属正常) |
| 残余 CSP 违规 | — | 只剩 `https://esm.sh/@excalidraw/…/fonts/…` 的**回退源**(库在 `src` 列表里附带的备用地址,字体实际已从本站加载):已知噪音 |
| localStorage | ✅ 绘图状态存在 `excalidraw`/`excalidraw-state` 键 | ✅ |
| Service Worker | ✅ 可注册 | ✅ |
| `window.open` | 返回 `null`(被拒) | ✅ |
| 剪贴板读写 | `NotAllowedError`(探针不在用户手势里,**不能下结论**) | 需手工 |
| blob 下载 | 点击后下载处理器没被调用(**不能下结论**) | 需手工 |

**两个对验收有决定性影响的发现:**
1. **绘图存在 WebView 自己的 `WKWebsiteDataStore` 里(localStorage),不在应用的 `data/` 目录里。** 验收 3 的"卸载含数据后清空"靠删目录做不到;wry 提供 `WebView::remove_data_store(&id, cb)`(要求先 drop 所有用这个存储的 webview;主线程 + 事件循环)。探针验证:固定端口 + 同存储标识时数据跨进程保留;调用 `remove_data_store` 后再启动,`localStorage` 为空。**→ Task 3。**
2. **origin 含端口:端口一变,localStorage 就丢**(探针默认端口 0 时每次都是新 origin,所以"上次的数据没了")。生产里端口已持久化且固定(A4a),验收时必须确认没被换掉。

自行构建 Excalidraw 的备注:上游 `package.json` 的 `engines.node` 是 `18 – 22`,本机 Node 24 会被 yarn 直接拒绝;且 `yarn install` 要拉整个 monorepo(本机经代理时反复重试失败)。所以打包配方改用官方 Docker 镜像取现成构建——不需要 Node。

## Global Constraints

- 运行时仍是 `network.outbound = "none"`、`downloads = "deny"`、`popups = "deny"`、`filesystem.data = "none"`、`clipboard = "none"`(`scripts/bytehost/excalidraw/manifest.toml`,有测试钉住"申请的权限全是最严")。**打包阶段才联网**(下载 3 个字体),运行时不联网。
- 卸载"仅程序"(`UninstallMode::Program`)**不得**清 WebView 存储——重装后画必须还在;只有"连数据一起删"才清。
- `remove_data_store` 必须在该应用 webview 已离开池之后调用(wry 的硬性要求),由 `StoreRemovals::take_ready` 保证。
- 不改 gateway 的 CSP(它对无网络权限的应用是对的:一个内联脚本都不放行);**让应用适配宿主**,不是反过来放宽。
- 验收 8 在一期按下面"重新表述"执行(没有 container runtime)。

## Review Focus

- `package.py` 的自检失败时必须非零退出(不得悄悄产出一个字体全坏的包)。
- 打包阶段下载字体**没有校验摘要**(只来自固定 CDN 路径):记入"已知局限",要不要钉 sha256 由你定。
- "含数据"卸载的意图记在 `App.purge_intents`(不在设置状态里):确认卸载后立刻关设置窗口也要清。
- 卸载**失败**时不清存储(意图要被丢掉),且只对 `Uninstall` 结果生效(停止的结果不能触发清除)。
- `take_ready` 必须在 webview 还在池里时扣住,放行后只清一次。
- 数据存储标识算法没变(`data_store_identifier`,A4b1 已钉死向量)——清的就是创建时用的那一个。

---

### Task 1: Excalidraw 打包配方 + 清单测试

**Files:**
- Create: `scripts/bytehost/excalidraw/package.py`、`package.sh`、`manifest.toml`
- Modify: `crates/bytehost-apps/src/manifest.rs`(一个测试)

**Interfaces:**
- Produces: `package.sh <输出应用目录> [镜像]` → `<输出>/manifest.toml` + `<输出>/web/`(可直接当 `LocalDir` 安装源);`package.py` 自检失败即 `sys.exit` 非零。

- [ ] **Step 1: 写失败测试。** `manifest.rs` 里 `the_shipped_excalidraw_manifest_parses_and_asks_for_nothing`(`#[cfg(feature = "manifest-toml")]`,用 `include_str!` 读 `scripts/bytehost/excalidraw/manifest.toml`)。先不创建清单文件 → 编译失败(RED)。
- [ ] **Step 2: 实现。** 新文件:

`scripts/bytehost/excalidraw/manifest.toml`:

```toml
schema_version = 1
min_host_version = "0.1.0"
id = "excalidraw"
name = "Excalidraw"
version = "0.1.0"

[presentation]
entrypoint = "main"

[entrypoints.main]
type = "web"
path = "/"

[runtime]
kind = "static_web"
source = "web/"

# 全部取最严:不联网(字体/库都已随包)、不使用宿主数据目录(绘图存在 WebView 自己的 localStorage 里)。
[permissions]
clipboard = "none"
downloads = "deny"
popups = "deny"

[permissions.network]
outbound = "none"

[permissions.filesystem]
data = "none"
```

`scripts/bytehost/excalidraw/package.py`:

```python
#!/usr/bin/env python3
"""把 excalidraw.com 的生产静态构建(官方 Docker 镜像 `/usr/share/nginx/html`)整理成能在 bytehost 严格 CSP
(`script-src 'self'`,无 inline;`connect-src/font-src 'self'`)下工作的应用目录:

- 所有内联 `<script>` 原样外置成 `/bh-inline-N.js`(顺序不变),但**去掉**三类外联行为:统计脚本(simpleanalytics)、
  Excalidraw+ 自动跳转、以及指向 CDN 的 `EXCALIDRAW_ASSET_PATH`(改成 `["/"]`,字体与库从本站取);
- 去掉指向外部站点的 `<link rel=preload/preconnect>`(它们本来就会被 CSP 拦掉,只会刷违规日志)。
- 构建出的 CSS 里 UI 字体 Assistant 的 `@font-face` 写死指向官方 CDN(`.../oss/fonts/Assistant/*.woff2`),而静态构建里只带了
  Regular 一个:打包时把 CDN 前缀改成本站 `/`,并从 CDN **下载**缺的另外三个字重(打包是联网的构建步骤,运行时仍是 `network: none`);
  下载失败就用本地的 `Assistant-Regular.woff2` 顶替(字重由浏览器合成,没有 CSP 违规)。
输出布局(可直接作为 `LocalDir` 安装源):`<输出目录>/manifest.toml` + `<输出目录>/web/`(站点)。
用法: package.py <官方镜像里的 html 目录> <输出应用目录>
"""
import os, re, shutil, sys, urllib.request

src, app = sys.argv[1], sys.argv[2]
out = f"{app}/web"
shutil.rmtree(app, ignore_errors=True)
os.makedirs(app)
shutil.copytree(src, out)
html = open(f"{out}/index.html", encoding="utf-8").read()

n = 0
def externalize(m):
    global n
    attrs, body = m.group(1) or "", m.group(2)
    if "src=" in attrs:  # 本来就是外联脚本(模块入口)
        return m.group(0)
    text = body.strip()
    if not text:
        return ""
    if "simpleanalyticscdn" in text or "excplus-autoredirect" in text:
        return ""  # 统计 + Excalidraw+ 跳转:不要
    if "EXCALIDRAW_ASSET_PATH" in text:
        text = 'window.EXCALIDRAW_ASSET_PATH = ["/"];'
    n += 1
    name = f"bh-inline-{n}.js"
    open(f"{out}/{name}", "w", encoding="utf-8").write(text + "\n")
    return f'<script src="/{name}"></script>'

html = re.sub(r"<script((?:\s[^>]*)?)>(.*?)</script>", externalize, html, flags=re.S)
html = re.sub(r'<link rel="(?:preload|preconnect)"[^>]*https://[^>]*>\s*', "", html)
open(f"{out}/index.html", "w", encoding="utf-8").write(html)
left = re.findall(r"<script(?:\s[^>]*)?>(?!\s*</script>)[^<]", html)
ext = len(re.findall(r'https://', html))
print(f"externalized {n} inline scripts; remaining inline: {len(left)}; remaining https:// mentions in index.html: {ext}")

CDN = "https://excalidraw.nyc3.cdn.digitaloceanspaces.com/oss/"
for css in [f for f in os.listdir(f"{out}/assets") if f.endswith(".css")]:
    path = f"{out}/assets/{css}"
    text = open(path, encoding="utf-8").read()
    wanted = sorted(set(re.findall(re.escape(CDN) + r"(fonts/[A-Za-z]+/[^\"')\s]+)", text)))
    for rel in wanted:
        dest = f"{out}/{rel}"
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        if os.path.exists(dest):
            continue
        try:
            with urllib.request.urlopen(CDN + rel, timeout=20) as r, open(dest, "wb") as f:
                f.write(r.read())
            print("fetched", rel)
        except Exception as e:  # 离线:用 Regular 顶替
            fallback = f"{out}/Assistant-Regular.woff2"
            if os.path.exists(fallback):
                shutil.copy(fallback, dest)
                print("fallback", rel, "<-", "Assistant-Regular.woff2", f"({e})")
    open(path, "w", encoding="utf-8").write(text.replace(CDN, "/"))

# 自检:打包产物必须满足严格 CSP(`script-src 'self'`、字体只能来自本站)——不满足就失败,不静默放过。
problems = []
if left:
    problems.append("index.html 里还有内联脚本")
for css in [f for f in os.listdir(f"{out}/assets") if f.endswith(".css")]:
    if CDN in open(f"{out}/assets/{css}", encoding="utf-8").read():
        problems.append(f"{css} 里还有指向 CDN 的字体地址")
for need in ["fonts/Assistant/Assistant-Regular.woff2", "fonts/Excalifont", "fonts/Virgil", "fonts/Nunito"]:
    if not os.path.exists(f"{out}/{need}"):
        problems.append(f"缺少 {need}")
if problems:
    sys.exit("打包自检失败: " + "; ".join(problems))

shutil.copy(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.toml"), f"{app}/manifest.toml")
print("OK:", app)
```

`scripts/bytehost/excalidraw/package.sh`(`chmod +x`):

```sh
#!/bin/sh
# 用官方 Docker 镜像里的 Excalidraw 静态构建打包出 bytehost 应用目录。
# 用法: package.sh <输出应用目录> [镜像,默认 excalidraw/excalidraw:latest]
# 需要 docker(只用来取文件,不运行容器)与 python3;打包时会联网下载 3 个 Assistant 字重。
set -eu
OUT="${1:?用法: package.sh <输出应用目录> [镜像]}"
IMAGE="${2:-excalidraw/excalidraw:latest}"
HERE="$(cd "$(dirname "$0")" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
docker pull "$IMAGE" >/dev/null
C="$(docker create "$IMAGE")"
docker cp "$C:/usr/share/nginx/html" "$TMP/html"
docker rm "$C" >/dev/null
echo "镜像摘要: $(docker image inspect --format '{{index .RepoDigests 0}}' "$IMAGE")"
python3 "$HERE/package.py" "$TMP/html" "$OUT"
```

测试:

```diff
diff --git a/crates/bytehost-apps/src/manifest.rs b/crates/bytehost-apps/src/manifest.rs
index 9114b31f..bc4af1ca 100644
--- a/crates/bytehost-apps/src/manifest.rs
+++ b/crates/bytehost-apps/src/manifest.rs
@@ -346,4 +346,16 @@ mod tests {
     const HOST: Version = Version::new(0, 1, 0);
 
+    /// 随仓库提供的 Excalidraw 打包清单(`scripts/bytehost/excalidraw/manifest.toml`)必须能被真实解析器接受
+    /// (含 `deny_unknown_fields`),并且申请的权限全部取最严——Excalidraw 不联网、不需要宿主数据目录。
+    #[cfg(feature = "manifest-toml")]
+    #[test]
+    fn the_shipped_excalidraw_manifest_parses_and_asks_for_nothing() {
+        let text = include_str!("../../../scripts/bytehost/excalidraw/manifest.toml");
+        let m = Manifest::from_toml(text, &HOST).expect("清单应通过校验");
+        assert_eq!(m.id.as_str(), "excalidraw");
+        assert_eq!(m.permissions, Permissions::default(), "所有权限都应是最严");
+        assert!(matches!(m.runtime, Runtime::StaticWeb { ref source } if source == "web/"));
+    }
+
     fn valid() -> Manifest {
         Manifest {
```

- [ ] **Step 3: GREEN。** `cargo test -p bytehost-apps --all-features shipped_excalidraw` → 通过;`cargo test -p bytehost-apps shipped_excalidraw`(默认 feature)→ 编译通过、0 个测试(该测试在无 `manifest-toml` 时不存在)。
- [ ] **Step 4: 变异检查。** 把 `manifest.toml` 里 `outbound = "none"` 改成 `"any"` → 测试 FAILED;还原。
- [ ] **Step 5: 跑一次打包。** `sh scripts/bytehost/excalidraw/package.sh /tmp/excalidraw-app` → 末行 `OK: /tmp/excalidraw-app`;`ls /tmp/excalidraw-app` 有 `manifest.toml` 与 `web/`;`web/index.html` 里没有内联 `<script>`。离线时应看到 `fallback …` 行而不是失败。
- [ ] **Step 6: Commit。** `git add scripts/bytehost crates/bytehost-apps && git commit -m "feat(bytehost): Excalidraw packaging recipe (CSP-safe, fonts local) + manifest test (A5 task 1)"`

### Task 2: V2 探针(可重复的自动验证)

**Files:**
- Create: `spike/v2-excalidraw/Cargo.toml`、`spike/v2-excalidraw/src/main.rs`、`scripts/bytehost/excalidraw/verify.py`
- Modify: 根 `Cargo.toml`(`exclude` 加 `spike/v2-excalidraw`)

**Interfaces:**
- Produces: `cargo run -q -- --dir <站点目录> [--port N] [--store N] [--wait S] [--net none|any]`(stdout 一行 `RESULT {…}`);`cargo run -q -- --purge --store N`(清存储);`verify.py <应用目录>` 断言 V2 通过条件。

- [ ] **Step 1: 写失败测试。** `verify.py` 本身就是测试;先只建 `spike` 骨架(`main` 里 `todo!()`)→ `verify.py` 拿不到结果、非零退出(RED)。
- [ ] **Step 2: 实现。**

`spike/v2-excalidraw/Cargo.toml`:

```toml
[package]
name = "v2-excalidraw-spike"
version = "0.0.0"
edition = "2024"
publish = false

# bytehost V2 验证:真实 Excalidraw 静态构建在**真实 gateway(含 CSP)+ 受限 wry webview** 里的行为。
# 一次性工具,随时可删;自带 [workspace](不进主锁文件)。
[workspace]

[dependencies]
bytehost-apps = { path = "../../crates/bytehost-apps", features = ["server"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
wry = "0.55.1"
tao = "0.35.3"
serde_json = "1"
```

`spike/v2-excalidraw/src/main.rs`:

```rust
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
```

`scripts/bytehost/excalidraw/verify.py`(`chmod +x`):

```python
#!/usr/bin/env python3
"""V2 自动验证:把打包好的应用目录交给真实 gateway(严格 CSP)与受限 wry webview(`spike/v2-excalidraw`),
断言:无 inline 脚本违规、资源路径指向本站、画布出现、全部绘图字体能从本站加载、`window.open` 被拒、
Service Worker 注册、localStorage 可写。会短暂弹出一个窗口(约 20 秒)。
用法: verify.py <输出应用目录> [--fixed-port 24681]
退出码非 0 = 有断言失败(同时打印整份探测结果)。
"""
import json, subprocess, sys, os

app = sys.argv[1]
port = sys.argv[sys.argv.index("--fixed-port") + 1] if "--fixed-port" in sys.argv else "0"
spike = os.path.join(os.path.dirname(__file__), "../../../spike/v2-excalidraw")
proc = subprocess.run(
    ["cargo", "run", "-q", "--", "--dir", f"{app}/web", "--port", port, "--wait", "10"],
    cwd=spike, capture_output=True, text=True, timeout=600,
)
line = next((l for l in proc.stdout.splitlines() if l.startswith("RESULT ")), None)
if line is None:
    sys.exit("没有拿到探测结果:\n" + proc.stderr[-2000:])
r = json.loads(line[7:])
fails = []
def need(cond, msg):
    if not cond:
        fails.append(msg)
need(not any(v.startswith("script-src") for v in r["csp"]), "有内联/外部脚本被 CSP 拦截")
need(r["assetPath"] == 'object:["/"]', "EXCALIDRAW_ASSET_PATH 不是 [/]: " + r["assetPath"])
need(r["canvases"] >= 1 and r["excalidrawRoot"], "Excalidraw 没有渲染出来")
for fam in ["Excalifont", "Virgil", "Cascadia", "Nunito", "Comic Shanns", "Assistant", "Lilita One", "Liberation Sans"]:
    need(r["fontLoad"].get(fam, "").endswith("status=loaded"), f"字体 {fam} 没能从本站加载: {r['fontLoad'].get(fam)}")
need(r["windowOpen"] == "null", f"window.open 没被拒: {r['windowOpen']}")
need(r["swRegs"] == 1, f"Service Worker 注册数 {r['swRegs']}")
need(r["localStorage"] == "1", "localStorage 不可写")
need(r["deniedNavigations"] == [], f"有导航被拒: {r['deniedNavigations']}")
noise = [v for v in r["csp"] if not v.startswith("font-src <- https://esm.sh/@excalidraw/excalidraw/dist/prod/fonts/")]
need(noise == [], f"除 esm.sh 字体回退源之外还有 CSP 违规: {noise[:5]}")
print(json.dumps({k: r[k] for k in ["assetPath", "canvases", "fontLoad", "windowOpen", "swRegs", "download", "downloadAttempts", "clipWrite", "clipRead"]}, ensure_ascii=False, indent=1))
if fails:
    print("FAIL:\n- " + "\n- ".join(fails))
    sys.exit(1)
print("V2 OK(esm.sh 字体回退源产生的 CSP 违规是已知噪音:字体已从本站加载)")
```

根 `Cargo.toml`:

```diff
diff --git a/Cargo.toml b/Cargo.toml
index 2611899a..346583a4 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -4,5 +4,5 @@ members = ["crates/*", "spike/*"]
 # origin-gateway 是 bytehost 应用宿主的一次性 origin 验证(自带 [workspace],不进主锁文件)
 # plantuml-js 是文件预览 PlantUML 的 Node/JS 引擎 spike(npm 工程,无 Cargo.toml)
-exclude = ["spike/origin-gateway", "spike/plantuml-js"]
+exclude = ["spike/origin-gateway", "spike/plantuml-js", "spike/v2-excalidraw"]
 
 [workspace.package]
```

- [ ] **Step 3: GREEN。** `python3 scripts/bytehost/excalidraw/verify.py /tmp/excalidraw-app` → 末行 `V2 OK(…)`,退出码 0(会弹一个窗口约 20 秒)。
- [ ] **Step 4: 反向验证(确认探针确实能抓到问题)。** 对**未打包**的原始构建跑探针(`cargo run -q -- --dir <docker 取出的 html 目录> --wait 10`):结果里 `csp` 应含 5 条 `script-src-elem <- inline` 与数百条 `font-src` 违规,`assetPath` 为 `undefined:undefined`——这就是 V2 表格"原样构建"一列;若这里居然通过,说明探针没在检查 CSP,要先修探针。
- [ ] **Step 5: 持久化与清除(Task 3 的前置证据)。** 固定端口 + 存储标识连跑:`--port 24680 --store 9` 第一次 `lsAtStart` 为空 → 第二次含 `excalidraw`、`excalidraw-state` 等键 → `--purge --store 9` → 第三次又为空。三次的 `lsAtStart` 记进验收报告。
- [ ] **Step 6: Commit。** `git add spike/v2-excalidraw scripts/bytehost Cargo.toml && git commit -m "feat(bytehost): V2 probe (real gateway + restricted wry) and verify script (A5 task 2)"`

### Task 3: 卸载"连数据一起删"清 WebView 数据存储

**Files:** Modify `crates/dozer-app/src/app_webview.rs`、`app/app.rs`、`app/update.rs`、`platform/window_events.rs`

**Interfaces:**
- Consumes: A4b1 的 `data_store_identifier`、`webview_id`、`AppViews::clear`;A4c 的 `settings_apps::{Message::UninstallConfirmed, Message::ActionDone, ActKind::Uninstall, Flow::ConfirmUninstall}`。
- Produces: `StoreRemovals::{request, take_ready}`、`App.purge_intents`/`App.store_removals`、`App::purge_app_data_store`/`take_ready_store_removals`;窗口层在 `sync_webview_pool` 之后调 `WebView::remove_data_store`。

- [ ] **Step 1: 写失败测试。** `app_webview.rs` 的 `store_removal_waits_until_the_apps_webview_has_left_the_pool`、`store_removal_only_releases_apps_that_are_ready_and_keeps_the_rest_queued`。先给 `StoreRemovals` 一个返回空的 `take_ready` 骨架 → 测试失败(RED)。
- [ ] **Step 2: 实现。**

```diff
diff --git a/crates/dozer-app/src/app_webview.rs b/crates/dozer-app/src/app_webview.rs
index a5614590..459b564b 100644
--- a/crates/dozer-app/src/app_webview.rs
+++ b/crates/dozer-app/src/app_webview.rs
@@ -173,4 +173,40 @@ else if(c==='Digit1'||k==='1'){{e.preventDefault();send('zoom_reset')}}}},true)}
 }
 
+/// 待清除的应用 WKWebsiteDataStore(卸载"连数据一起删"时用)。**清除前必须先让使用它的 webview 离开池**
+/// (wry:`remove_data_store` 要求先 drop 所有用这个存储的 WebView),所以这里只排队,
+/// 窗口层每帧在 `sync_webview_pool` 之后调 [`StoreRemovals::take_ready`],只放行"池里已经没有它的 webview"的那些。
+#[derive(Debug, Default)]
+pub(crate) struct StoreRemovals {
+    pending: Vec<String>,
+}
+
+impl StoreRemovals {
+    /// 排队清除某应用的数据存储(同一个应用重复排队只留一份)。
+    pub(crate) fn request(&mut self, app_id: &str) {
+        if !self.pending.iter().any(|p| p == app_id) {
+            self.pending.push(app_id.to_owned());
+        }
+    }
+
+    /// 取走已经可以清除的:`webview_in_pool(slot)` 为假(那个应用的 webview 已不在池里)的才放行,
+    /// 其余继续排队等下一帧。`(应用 id, 存储标识)`。
+    pub(crate) fn take_ready(
+        &mut self,
+        webview_in_pool: impl Fn(AppSlot) -> bool,
+    ) -> Vec<(String, [u8; 16])> {
+        let mut ready = Vec::new();
+        self.pending.retain(|id| {
+            // 没有槽(从没被面板用过)就一定没有 webview,可以直接清。
+            let in_pool = AppSlot::intern(id).is_some_and(&webview_in_pool);
+            if in_pool {
+                return true;
+            }
+            ready.push((id.clone(), data_store_identifier(id)));
+            false
+        });
+        ready
+    }
+}
+
 pub(crate) fn app_webview_spec(slot: AppSlot, url: &str, visible: bool) -> WebviewSpec {
     WebviewSpec {
@@ -373,4 +409,34 @@ mod tests {
     }
 
+    #[test]
+    fn store_removal_waits_until_the_apps_webview_has_left_the_pool() {
+        let mut q = StoreRemovals::default();
+        let slot = AppSlot::intern("purge-a").unwrap();
+        q.request("purge-a");
+        q.request("purge-a");
+        assert!(
+            q.take_ready(|s| s == slot).is_empty(),
+            "webview 还在池里:不能清"
+        );
+        let ready = q.take_ready(|_| false);
+        assert_eq!(
+            ready,
+            vec![("purge-a".to_string(), data_store_identifier("purge-a"))]
+        );
+        assert!(q.take_ready(|_| false).is_empty(), "只清一次");
+    }
+
+    #[test]
+    fn store_removal_only_releases_apps_that_are_ready_and_keeps_the_rest_queued() {
+        let mut q = StoreRemovals::default();
+        let busy = AppSlot::intern("purge-busy").unwrap();
+        q.request("purge-busy");
+        q.request("purge-free");
+        let ready = q.take_ready(|s| s == busy);
+        assert_eq!(ready.len(), 1);
+        assert_eq!(ready[0].0, "purge-free");
+        assert_eq!(q.take_ready(|_| false).len(), 1, "busy 的下一帧放行");
+    }
+
     #[test]
     fn views_produce_a_spec_only_while_an_address_is_set() {
diff --git a/crates/dozer-app/src/app/app.rs b/crates/dozer-app/src/app/app.rs
index d086e64d..c48f7fe9 100644
--- a/crates/dozer-app/src/app/app.rs
+++ b/crates/dozer-app/src/app/app.rs
@@ -470,4 +470,8 @@ pub struct App {
     /// 已安装应用的列表轮询与每个应用面板的状态机(bytehost A4b2,见 `extensions::app_host`)。
     pub(crate) app_host: crate::extensions::app_host::State,
+    /// 用户在卸载确认里选了"连数据一起删"、请求已发出但结果还没回来的应用 id(设置窗口关了也不丢)。
+    pub(crate) purge_intents: std::collections::HashSet<String>,
+    /// 待清除的应用 WKWebsiteDataStore(见 `app_webview::StoreRemovals`)。
+    pub(crate) store_removals: crate::app_webview::StoreRemovals,
     /// 数据库面板 App 级状态(哪些驱动类型在"新增数据源"下拉里可选,
     /// 启动时读盘)——见 `extensions::database::AppState`。
@@ -860,4 +864,6 @@ impl App {
             app_views: crate::app_webview::AppViews::default(),
             app_host: crate::extensions::app_host::State::default(),
+            purge_intents: Default::default(),
+            store_removals: Default::default(),
             database: database::AppState::load(),
             footbar: footbar::AppState::default(),
@@ -1708,4 +1714,21 @@ impl App {
     }
 
+    /// 应用已被"连数据一起删"卸载:收回它的 webview(让池释放),再排队清除它的数据存储——
+    /// localStorage/IndexedDB 在 WKWebsiteDataStore 里,不在应用的 `data/` 目录里,光删目录清不掉画。
+    pub(crate) fn purge_app_data_store(&mut self, app_id: &str) {
+        if let Some(slot) = AppSlot::intern(app_id) {
+            self.app_views.clear(slot);
+        }
+        self.store_removals.request(app_id);
+    }
+
+    /// 窗口层每帧在池同步之后调:取走现在可以清除的数据存储(它们的 webview 已不在池里)。
+    pub(crate) fn take_ready_store_removals(
+        &mut self,
+        webview_in_pool: impl Fn(AppSlot) -> bool,
+    ) -> Vec<(String, [u8; 16])> {
+        self.store_removals.take_ready(webview_in_pool)
+    }
+
     /// 当前可见(未收起、未被另一侧放大盖住)的应用面板——左右两栏可以同时各显示一个应用面板。
     pub(crate) fn visible_app_slots(&self) -> Vec<AppSlot> {
diff --git a/crates/dozer-app/src/app/update.rs b/crates/dozer-app/src/app/update.rs
index 26c997d9..fb2e8f2e 100644
--- a/crates/dozer-app/src/app/update.rs
+++ b/crates/dozer-app/src/app/update.rs
@@ -3915,4 +3915,27 @@ impl App {
                 let restart_succeeded =
                     matches!(msg, settings::Message::AdvancedRestartResult(Ok(())));
+                // 卸载"连数据一起删":在请求发出前记下意图(id 取自确认步骤),结果回来时据此清数据存储。
+                // 意图存在 `App` 上而不是设置状态里,所以确认后立刻关窗也不丢。
+                if let settings::Message::Apps(
+                    crate::extensions::settings_apps::Message::UninstallConfirmed(
+                        bytehost_apps::registry::UninstallMode::ProgramAndData,
+                    ),
+                ) = &msg
+                    && let Some(crate::extensions::settings_apps::Flow::ConfirmUninstall {
+                        id, ..
+                    }) = self.settings.as_ref().map(|s| &s.apps.flow)
+                {
+                    self.purge_intents.insert(id.clone());
+                }
+                let uninstall_result = match &msg {
+                    settings::Message::Apps(
+                        crate::extensions::settings_apps::Message::ActionDone(
+                            id,
+                            crate::extensions::settings_apps::ActKind::Uninstall,
+                            result,
+                        ),
+                    ) => Some((id.clone(), result.is_ok())),
+                    _ => None,
+                };
                 // 设置窗口已经关了(状态是 `None`)时才到达的安装/停止/卸载结果:状态机不在了,事情照样发生了,
                 // 仍要刷新应用宿主、把结果告诉用户(见 `settings_apps::orphan_result_effects`)。
@@ -3930,4 +3953,10 @@ impl App {
                 };
                 settings::update(&mut self.settings, msg, &client, &handle, emit);
+                if let Some((id, ok)) = uninstall_result
+                    && self.purge_intents.remove(&id)
+                    && ok
+                {
+                    self.purge_app_data_store(&id);
+                }
                 for effect in orphan_effects {
                     match effect {
diff --git a/crates/dozer-app/src/platform/window_events.rs b/crates/dozer-app/src/platform/window_events.rs
index 5a9705df..2944bf33 100644
--- a/crates/dozer-app/src/platform/window_events.rs
+++ b/crates/dozer-app/src/platform/window_events.rs
@@ -62,4 +62,6 @@ use crate::preview;
 use crate::theme;
 
+dozer_core::scope!(LOG, module, "platform");
+
 /// 按 iced 本帧算出的 `mouse_interaction` 刷新窗口光标。
 ///
@@ -1142,4 +1144,19 @@ impl Runner {
             true,
         );
+        // 卸载"连数据一起删"的应用:它的 webview 已经不在池里了才能清它的 WKWebsiteDataStore
+        // (wry 要求先 drop 所有用这个存储的 WebView;`remove_data_store` 需在主线程且有事件循环)。
+        for (app_id, store) in app.take_ready_store_removals(|slot| {
+            webviews.contains_key(&crate::app_webview::webview_id(slot))
+        }) {
+            use wry::WebViewExtDarwin;
+            wry::WebView::remove_data_store(&store, move |result| match result {
+                Ok(()) => {
+                    dozer_core::log_info!(LOG, app = %app_id, "已清除应用的 WebView 数据存储")
+                }
+                Err(e) => {
+                    dozer_core::log_warn!(LOG, app = %app_id, error = %e, "清除应用的 WebView 数据存储失败")
+                }
+            });
+        }
     }
```

- [ ] **Step 3: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app store_removal` → 2 个通过。
- [ ] **Step 4: 变异检查。** 把 `take_ready` 里 `if in_pool { return true; }` 改成 `if false && in_pool { … }` → 两个测试 FAILED;还原。
- [ ] **Step 5: 静态核对(无 App 夹具,逐条打勾)。** 意图只在 `UninstallConfirmed(ProgramAndData)` 且当前流程是 `ConfirmUninstall` 时记录;只在 `ActionDone(_, Uninstall, Ok)` 时清,`Err` 或非 `Uninstall` 的结果都只丢弃意图;`purge_app_data_store` 先 `app_views.clear(slot)` 再排队;窗口层只在 `take_ready` 放行(webview 不在池里)后才调 `remove_data_store`。
- [ ] **Step 6: 全量门禁。** `cargo fmt --check -p dozer-app -p bytehost-apps && bash scripts/check-log-scope.sh && bash scripts/check-bytehost-apps-deps.sh && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets` → 测试全过,仅既有的 `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 红(执行前先在 main 上复核);原型里 1924 通过 + 那 1 个;clippy 里本任务相关文件无新警告。
- [ ] **Step 7: Commit。** `git commit -am "feat(dozer-app): purge the app's WebView data store when uninstalling with data (A5 task 3)"`

### Task 4: 真实 GUI 验收 + 报告 + 文档

**Files:** Create `docs/superpowers/specs/2026-10-05-bytehost-a5-acceptance-report.md`;Modify 规格(V2/A5 行)、`CLAUDE.md`

- [ ] **Step 1: 准备。** `sh scripts/bytehost/excalidraw/package.sh ~/excalidraw-app`;`cargo run -p dozer-app`(确保 dozerd 是本分支构建的);**验收前先做 A4b1/A4b2/A4c 欠的三份手工验收**(它们是 A5 的地基,见各自计划的"手工验收"一节),把结果写进报告。
- [ ] **Step 2: 按下表逐项验收,每项写"通过/失败 + 证据(日志、截图、命令输出)"。**

| # | 规格验收 | 怎么验 | 自动化覆盖 |
|---|---|---|---|
| 1 | 安装→画→退出 **GUI** 重开→画还在 | 设置→应用→安装应用…→选 `~/excalidraw-app`:审批卡应显示"出站网络/下载/弹窗/剪贴板"等(此清单只申请最严,可能显示"不申请任何权限")、来源"本地目录"、运行方式 `static_web`;批准并安装→图标栏出现→点它→启动→画几笔→**退出整个 Dozer(dozerd 不退)**→重开→进该应用:画还在。**确认端口没变**(`~/Library/…/bytehost/gateway.json`) | 持久化机制由 Task 2 Step 5 的探针证明;GUI 全流程只能手工 |
| 2 | 篡改源码后用旧批准安装被拒 | 审批卡停留时改 `~/excalidraw-app/web/index.html` 再点批准:流程内显示"应用源码在审批之后发生了变化" | ✅ `bytehost-apps` 的 `a_source_or_manifest_changed_after_approval_is_refused_and_leaves_nothing_behind` 与 `plan.rs` 的 verify 测试 |
| 3 | 卸载"仅程序"保留数据,重装后画还在;"含数据"后清空 | 画一笔→设置→卸载→「保留数据」→重装同一目录→打开:画还在;再画→卸载→「连数据一起删」→重装→打开:空白 | 目录层 ✅ `uninstalling_the_program_stops_serving_and_keeps_user_data` 等;WebView 存储层:Task 2 Step 5(清除机制)+ Task 3(队列逻辑);GUI 串起来只能手工 |
| 4 | 第二个应用与 Excalidraw 的 localStorage/Cookie 互不可见 | 装一个任意静态页(写 `localStorage.x=1` 的小页面)→在两个应用里互相读 | V1 spike 已证明同端口多 origin 隔离;GUI 手工复核 |
| 5 | 伪造 `Host: evil.example` 被拒 | `curl -i -H 'Host: evil.example' http://127.0.0.1:<端口>/` → 4xx | ✅ gateway 的 `the_host_header_must_be_exactly_a_valid_app_dot_localhost_with_the_gateways_port` |
| 6 | manifest 出现未知权限字段时计划失败 | 在 `manifest.toml` 加 `[permissions] telepathy = "allow"`,安装→流程内红字失败 | ✅ `unknown_fields_are_a_parse_error_at_the_top_level_and_in_nested_tables` |
| 7 | **GUI 退出后 gateway 仍在服务**;dozerd 停后端口不再服务、状态 `Stopped`;重启 dozerd 按 `desired` 恢复 | 应用运行中:退出 Dozer → `curl -i -H 'Host: excalidraw.localhost:<端口>' http://127.0.0.1:<端口>/`(带令牌的首次导航地址不可用,所以这里期望 **403/需令牌的响应**而不是连接失败——响应本身证明端口仍在服务)→ 设置→高级→停止 dozerd(或 `kill`)→ 同一条 curl **连接失败** → 重启 dozerd → 应用自动回到运行 | ✅ `a_shutdown_request_takes_the_apps_down_but_keeps_what_the_user_wanted`、`apps_wanted_running_come_back_after_a_dozerd_restart_and_stopped_ones_stay_stopped`;"GUI 退出不影响 dozerd"是进程结构,只能手工 |
| 8 | **重新表述(一期无 container runtime):** 声明了 `kind = "container"`(或 python/node)的应用在安装时被**明确拒绝**、不静默降级 | 把测试清单的 `[runtime]` 改成 `kind = "container"` + `image = "x"` → 安装→流程内显示"暂不支持 container 类型的应用(一期只支持 static_web)" | ✅ `only_static_web_is_supported_in_phase_one`;"面板里出现宿主提示页"的原始表述要等有 container adapter 才有意义,届时再验 |

- [ ] **Step 3: V2 手工补项**(探针不在用户手势里,结论不了的两项):在 Excalidraw 里 ① 选中图形 `⌘C` 再 `⌘V`(应用内剪贴板)→ 能复制粘贴;② 菜单里"导出为 PNG/保存到…"→ 预期**什么都不发生或被拒**(A4b1 一律拒下载;Excalidraw 没有"另存"的替代路径,这是已知功能损失);③ 切换主题、画文字(检查手写体字形正确);④ 画中文(Xiaolai 分片按需加载)。结论写进报告。
- [ ] **Step 4: 写验收报告**(新文件,`docs/superpowers/specs/2026-10-05-bytehost-a5-acceptance-report.md`):V2 表格 + 8 项结果 + 欠的三份手工验收结果 + 发现的缺陷清单(每条标严重度)。失败项不隐瞒,要么当场修要么列入缺陷。
- [ ] **Step 5: Commit + 文档。** 规格 V2 行标"已验证:见 A5 计划与验收报告";A5 行标完成(**只有 Step 2 的 8 项都有结论才能写"已完成"**);`CLAUDE.md` 的 bytehost-apps 行补一句"Excalidraw 打包配方在 `scripts/bytehost/excalidraw/`(`package.sh` + `verify.py`);应用适配宿主的严格 CSP,不放宽 CSP"。

## 已知局限

- **Excalidraw 的导出/保存不可用**(A4b1 拒绝所有下载;Excalidraw 的"保存/导出"走 `<a download>`/File System Access)。一期验收不依赖导出;要不要给应用一个"保存文件"能力(`downloads = user_confirm` 的真正实现)是后续的产品决定。
- **打包阶段联网下载 3 个字体且未校验摘要**:来源是固定 CDN 路径;如需可重复构建应把 sha256 钉进 `package.py`。
- **`esm.sh` 字体回退源的 CSP 违规日志**是已知噪音(字体已从本站加载),`verify.py` 显式放行这一类。
- 官方镜像的版本是 `latest`;`package.sh` 会打印实际摘要,报告里要记下它。需要可重复时把镜像钉成摘要。
- 剪贴板与下载的**自动**探测在 wry 里拿不到用户手势,只能手工验(Step 3)。
- 清除 WebView 存储要求 macOS 14+(`remove_data_store`);更老系统每应用存储本来就退回默认存储,"含数据"卸载清不掉该应用的页面数据(A4b1 已记录同一限制)。
