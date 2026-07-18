# Dozer P1d:左二资产预览 pane 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 左二从占位变成真实资产预览 pane:Flyfish 离线渲染文件 + localhost/外部网页 tab + 极简地址栏,wry 子视图叠加。

**Architecture:** 预览域状态机(`preview.rs`,纯数据 headless 全测)驱动 main.rs 事件环里的 webview 生命周期同步(建/毁/显隐/bounds/导航);静态资产与本地文件经 wry 自定义协议 `dozer://flyfish/...` 同源供给(无 HTTP 端口);内容区矩形用解析式换算(P1c `terminal_pane_pixel_size` 同手法)。

**Tech Stack:** wry 0.55.1(`build_as_child` + `with_custom_protocol`)、Flyfish `@file-viewer/web-full`(vendored IIFE dist)、rfd 0.15(原生文件选择器)、iced 0.14(既有)。

**规格来源:** specs/2026-07-18-dozer-p1d-preview-design.md(含裁决 D1-D5);spike GO 报告 specs/2026-07-15-spike-report-webview.md。

## Global Constraints

- wry 统一 **0.55.1**(spike 验证版本);webview 副作用只在 main.rs 消息分发环执行(spike 约束 2:纯视图层不碰句柄)。
- 主题 ByteBoy2077:bg `#0a0e16`、金 `#F2D94E`(甲方动作)、奶油 `#FFE5B4`、青 `#47DEF0`;theme.rs 色表锁定,禁止改动。
- 一期格式口径:Markdown/图片/PDF/代码/diff/localhost 网页;Office 长尾随 Flyfish 自带、不逐格式 QA;diff 只留 `TabKind` 预留位(P1f)。
- 自定义协议同源约束:文件端点必须在 `dozer://flyfish/` 命名空间下(macOS Origin = `dozer://flyfish`),且 URL 保留文件扩展名(Flyfish 靠扩展名选管线)。
- Flyfish full 包 vendored 体积 > ~80MB 时按管线精简到口径内格式(设计 D5)。
- 核心不依赖 Node/Python:npm 只是开发期取资产的工具,产物是静态文件进仓库。
- 每 Task 结束:`cargo test -p dozer-app` 全绿 + `cargo clippy -p dozer-app --all-targets` 零警告 + `cargo fmt`。

---

### Task 1: Flyfish 资产 vendore + host 页

**Files:**
- Create: `crates/dozer-app/assets/flyfish/host.html`
- Create: `crates/dozer-app/assets/flyfish/flyfish-file-viewer-web-full.iife.js`(vendored)
- Create: `crates/dozer-app/assets/flyfish/file-viewer/`(vendored 资产目录:workers/WASM/字体)

**Interfaces:**
- Consumes: 无(纯资产任务)。
- Produces: `assets/flyfish/` 目录契约——`host.html?p=<百分号编码的绝对路径>` 打开即渲染该文件;相对资产 `file-viewer/` 与 IIFE bundle 同目录(Flyfish 的部署约定:资产默认在 `<部署根>/file-viewer/`)。Task 3 的协议应答、Task 2 的 URL 拼装都依赖此布局。

- [x] **Step 1: 下载并解包 @file-viewer/web-full**

```bash
cd /tmp && VER=$(curl -s https://registry.npmjs.org/@file-viewer%2Fweb-full | python3 -c "import json,sys;print(json.load(sys.stdin)['dist-tags']['latest'])")
echo "version: $VER"
curl -sL "https://registry.npmjs.org/@file-viewer/web-full/-/web-full-$VER.tgz" -o web-full.tgz
tar xzf web-full.tgz && ls package/dist/ | head -20 && du -sh package/dist/
```

Expected: 列出 dist 内容,含 `flyfish-file-viewer-web-full.iife.js` 与 `file-viewer/` 目录(名称若有出入,以实际为准并在后续步骤同步修正引用);打印体积。

- [x] **Step 2: 体积裁决(设计 D5)**

若 `du -sh` > 80MB:检查 `package/dist/file-viewer/` 子目录(Flyfish 按管线分目录),删除不在口径内的管线目录(保留 markdown/图片/PDF/代码文本相关;CAD/EDA/3D/geo/email/archive 等删除),记录删除清单到 commit message。若 ≤ 80MB:全量保留。

- [x] **Step 3: 落位到仓库**

```bash
mkdir -p /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/crates/dozer-app/assets/flyfish
cp -R /tmp/package/dist/* /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/crates/dozer-app/assets/flyfish/
echo "$VER" > /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/crates/dozer-app/assets/flyfish/VENDORED_VERSION
```

- [x] **Step 4: 写 host 页**

`crates/dozer-app/assets/flyfish/host.html`(仓库自写,Dozer↔Flyfish 唯一耦合点):

```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Dozer Preview</title>
<style>
  html, body { margin: 0; height: 100%; background: #0a0e16; }
  flyfish-file-viewer { display: block; height: 100%; }
</style>
<script src="./flyfish-file-viewer-web-full.iife.js"></script>
</head>
<body>
<script>
  // ?p= 携带百分号编码的绝对路径;viewer src 用相对 URL 指向同源
  // __file__ 端点,encodeURI 保留 '/' 与扩展名(Flyfish 靠扩展名选管线)。
  const p = decodeURIComponent(new URLSearchParams(location.search).get('p') || '');
  const el = document.createElement('flyfish-file-viewer');
  el.setAttribute('src', '__file__' + encodeURI(p));
  el.setAttribute('theme', 'dark');
  document.body.appendChild(el);
</script>
</body>
</html>
```

- [x] **Step 5: 验证与提交**

```bash
ls crates/dozer-app/assets/flyfish/host.html crates/dozer-app/assets/flyfish/flyfish-file-viewer-web-full.iife.js
git add crates/dozer-app/assets docs
git commit -m "feat(预览): vendore Flyfish web-full dist + host 页（版本与体积裁决见正文）

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: preview.rs 预览域状态机

**Files:**
- Create: `crates/dozer-app/src/preview.rs`
- Modify: `crates/dozer-app/src/main.rs`(`mod preview;` 一行,加在 `mod keymap;` 之前)

**Interfaces:**
- Consumes: 无。
- Produces(Task 4/5 依赖,签名精确):
  - `pub struct PreviewPane`(`Default`):`tabs() -> &[PreviewTab]`、`active_idx() -> usize`、`open_path(PathBuf) -> usize`(返回 tab id)、`open_url(String) -> usize`、`select(usize)`、`close(usize)`、`addr_editing() -> bool`、`addr_buffer() -> &str`、`addr_begin()`、`addr_text(&str)`、`addr_backspace()`、`addr_cancel()`、`addr_submit() -> Option<AddrTarget>`、`desired_webviews() -> Vec<WebviewSpec>`
  - `pub struct PreviewTab { pub id: usize, pub kind: TabKind, pub title: String }`
  - `pub enum TabKind { File(PathBuf), Web { url: String } }`(注释注明 `Diff` 变体 P1f 补)
  - `pub enum AddrTarget { File(PathBuf), Url(String) }`
  - `pub struct WebviewSpec { pub id: usize, pub url: String, pub visible: bool }`
  - `pub fn encode_component(s: &str) -> String`(RFC3986:unreserved 之外全部 `%XX`)

- [x] **Step 1: 写失败测试**

`preview.rs` 尾部:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn open_select_close_tabs() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let id1 = p.open_url("http://localhost:3000".into());
        assert_eq!(p.tabs().len(), 2);
        assert_eq!(p.active_idx(), 1, "新开 tab 即激活");
        assert_ne!(id0, id1);
        assert_eq!(p.tabs()[0].title, "a.md");
        assert_eq!(p.tabs()[1].title, "localhost:3000");
        p.select(0);
        assert_eq!(p.active_idx(), 0);
        p.close(0);
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.active_idx(), 0);
    }

    #[test]
    fn addr_edit_and_submit_parses_path_vs_url() {
        let mut p = PreviewPane::default();
        p.addr_begin();
        assert!(p.addr_editing());
        for c in "/tmp/设计 稿.pdf".chars() {
            p.addr_text(&c.to_string());
        }
        assert_eq!(
            p.addr_submit(),
            Some(AddrTarget::File(PathBuf::from("/tmp/设计 稿.pdf")))
        );
        assert!(!p.addr_editing());

        p.addr_begin();
        p.addr_text("localhost:3000/x");
        assert_eq!(
            p.addr_submit(),
            Some(AddrTarget::Url("http://localhost:3000/x".into()))
        );

        p.addr_begin();
        p.addr_text("https://example.com");
        assert_eq!(
            p.addr_submit(),
            Some(AddrTarget::Url("https://example.com".into()))
        );

        p.addr_begin();
        p.addr_text("abc");
        p.addr_backspace();
        p.addr_backspace();
        p.addr_backspace();
        assert_eq!(p.addr_submit(), None, "空输入不产生动作");

        p.addr_begin();
        p.addr_text("x");
        p.addr_cancel();
        assert!(!p.addr_editing());
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a b.md"));
        p.open_url("http://localhost:3000".into());
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa%20b.md"
        );
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(specs[1].url, "http://localhost:3000");
        assert!(specs[1].visible);
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app preview::`
Expected: 编译错误(类型/方法不存在)。

- [x] **Step 3: 最小实现**

```rust
// crates/dozer-app/src/preview.rs
//! 预览域状态机(P1d):左二 tabs、地址栏编辑态、webview 期望清单。
//! 纯数据,不碰 wry/iced——webview 副作用由 main.rs 对照
//! `desired_webviews()` 差集执行(spike 约束:句柄只活在事件分发环)。
use std::path::PathBuf;

/// 一个预览 tab。`TabKind::Diff` 变体留给 P1f(验收闭环)补。
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    File(PathBuf),
    Web { url: String },
}

/// 地址栏提交的解析结果:绝对路径 → 文件预览;其余按 URL 处理
/// (无 scheme 自动补 `http://`,localhost 场景免敲协议头)。
#[derive(Debug, Clone, PartialEq)]
pub enum AddrTarget {
    File(PathBuf),
    Url(String),
}

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
}

/// RFC3986 严格百分号编码:unreserved(字母/数字/`-._~`)之外全部 %XX。
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Default)]
pub struct PreviewPane {
    tabs: Vec<PreviewTab>,
    active: usize,
    next_id: usize,
    addr_editing: bool,
    addr_buffer: String,
}

impl PreviewPane {
    pub fn tabs(&self) -> &[PreviewTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    pub fn open_path(&mut self, path: PathBuf) -> usize {
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.push_tab(TabKind::File(path), title)
    }

    pub fn open_url(&mut self, url: String) -> usize {
        let title = url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or(&url)
            .to_string();
        self.push_tab(TabKind::Web { url: url.clone() }, title)
    }

    fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(PreviewTab { id, kind, title });
        self.active = self.tabs.len() - 1;
        id
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
        }
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
    }

    pub fn addr_editing(&self) -> bool {
        self.addr_editing
    }

    pub fn addr_buffer(&self) -> &str {
        &self.addr_buffer
    }

    /// 进入地址栏编辑:预填当前激活网页 tab 的 URL(文件 tab 不预填)。
    pub fn addr_begin(&mut self) {
        self.addr_editing = true;
        self.addr_buffer = match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Web { url }) => url.clone(),
            _ => String::new(),
        };
    }

    pub fn addr_text(&mut self, s: &str) {
        self.addr_buffer.push_str(s);
    }

    pub fn addr_backspace(&mut self) {
        self.addr_buffer.pop();
    }

    pub fn addr_cancel(&mut self) {
        self.addr_editing = false;
        self.addr_buffer.clear();
    }

    pub fn addr_submit(&mut self) -> Option<AddrTarget> {
        self.addr_editing = false;
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        if input.starts_with('/') {
            return Some(AddrTarget::File(PathBuf::from(input)));
        }
        if let Some(rest) = input.strip_prefix("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            return Some(AddrTarget::File(PathBuf::from(home).join(rest)));
        }
        if input.contains("://") {
            return Some(AddrTarget::Url(input.to_string()));
        }
        Some(AddrTarget::Url(format!("http://{input}")))
    }

    /// webview 期望清单:每 tab 一个,仅激活者可见(设计 D2)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| WebviewSpec {
                id: tab.id,
                url: match &tab.kind {
                    TabKind::File(path) => format!(
                        "dozer://flyfish/host.html?p={}",
                        encode_component(&path.to_string_lossy())
                    ),
                    TabKind::Web { url } => url.clone(),
                },
                visible: idx == self.active,
            })
            .collect()
    }
}
```

并在 `main.rs` 模块声明区加 `mod preview;`(紧邻 `mod keymap;`)。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app preview::`
Expected: 4 passed。

- [x] **Step 5: Commit**

```bash
cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app
git add crates/dozer-app/src && git commit -m "feat(预览): PreviewPane 状态机——tabs/地址栏解析/webview 期望清单（headless 全测）

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: assets.rs 自定义协议应答

**Files:**
- Create: `crates/dozer-app/src/assets.rs`
- Modify: `crates/dozer-app/src/main.rs`(`mod assets;` 一行)

**Interfaces:**
- Consumes: Task 1 的 `assets/flyfish/` 目录布局。
- Produces(Task 5 依赖):
  - `pub struct ProtocolReply { pub status: u16, pub mime: &'static str, pub body: Vec<u8> }`
  - `pub fn handle_protocol(assets_root: &Path, allowed: &HashSet<PathBuf>, uri: &str) -> ProtocolReply`
  - `pub fn assets_root() -> PathBuf`(= `CARGO_MANIFEST_DIR/assets/flyfish`;一期 dev 形态,打包分发后移)
  - `pub fn percent_decode(s: &str) -> Option<String>`

- [x] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dozer-assets-test-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("sub"));
        fs::write(dir.join("host.html"), b"<html>").unwrap();
        fs::write(dir.join("sub").join("a.js"), b"js").unwrap();
        dir
    }

    #[test]
    fn serves_vendored_asset_with_mime() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), "dozer://flyfish/host.html");
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>");
        let r = handle_protocol(&root, &HashSet::new(), "dozer://flyfish/sub/a.js?v=1");
        assert_eq!((r.status, r.mime), (200, "text/javascript"), "query 应被剥离");
    }

    #[test]
    fn rejects_traversal_and_unknown() {
        let root = scratch();
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), "dozer://flyfish/../etc/passwd").status,
            404
        );
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), "dozer://other/x").status,
            404
        );
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), "dozer://flyfish/nope.js").status,
            404
        );
    }

    #[test]
    fn file_endpoint_requires_allowlist() {
        let root = scratch();
        let f = root.join("victim.md");
        fs::write(&f, b"# hi").unwrap();
        let uri = format!(
            "dozer://flyfish/__file__{}",
            f.to_string_lossy().replace(' ', "%20")
        );
        // 不在白名单 → 404
        assert_eq!(handle_protocol(&root, &HashSet::new(), &uri).status, 404);
        // 在白名单 → 200 + 按扩展名给 mime
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, &uri);
        assert_eq!((r.status, r.mime), (200, "text/markdown"));
        assert_eq!(r.body, b"# hi");
    }

    #[test]
    fn percent_decode_roundtrip() {
        assert_eq!(percent_decode("%2Fa%20b").as_deref(), Some("/a b"));
        assert_eq!(percent_decode("%E4%BD%A0").as_deref(), Some("你"));
        assert_eq!(percent_decode("plain").as_deref(), Some("plain"));
        assert!(percent_decode("%GG").is_none());
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app assets::`
Expected: 编译错误。

- [x] **Step 3: 最小实现**

```rust
// crates/dozer-app/src/assets.rs
//! `dozer://` 自定义协议应答(P1d,纯函数,headless 全测)。
//!
//! 布局(设计 §2):
//! - `dozer://flyfish/<path>`          → vendored Flyfish dist 静态字节;
//! - `dozer://flyfish/__file__/<abs>`  → 本地文件字节,仅白名单(用户显式
//!   打开过的路径)。文件端点与 host 页同命名空间——wry 自定义协议在
//!   macOS 的 Origin 是 `dozer://flyfish`,跨命名空间 fetch 被 CORS 拦;
//!   URL 保留原始扩展名,Flyfish 靠它选择渲染管线。
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct ProtocolReply {
    pub status: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
}

fn not_found() -> ProtocolReply {
    ProtocolReply {
        status: 404,
        mime: "text/plain",
        body: b"not found".to_vec(),
    }
}

/// 一期 dev 形态:资产直接从仓库源码树读(cargo run 即用,host.html 可
/// 热改)。打包分发(app bundle 内资源目录)是后续任务,不在 P1d。
pub fn assets_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/flyfish"))
}

pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hi = (hex[0] as char).to_digit(16)?;
            let lo = (hex[1] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "md" | "markdown" => "text/markdown",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

pub fn handle_protocol(
    assets_root: &Path,
    allowed: &HashSet<PathBuf>,
    uri: &str,
) -> ProtocolReply {
    // 剥离 scheme 与 query;只服务 flyfish 命名空间。
    let Some(rest) = uri.strip_prefix("dozer://") else {
        return not_found();
    };
    let rest = rest.split('?').next().unwrap_or(rest);
    let Some(path) = rest.strip_prefix("flyfish/") else {
        return not_found();
    };

    // 本地文件端点:__file__/<百分号编码的绝对路径>(编码保留 '/')。
    if let Some(encoded) = path.strip_prefix("__file__") {
        let Some(decoded) = percent_decode(encoded) else {
            return not_found();
        };
        let file = PathBuf::from(decoded);
        if !allowed.contains(&file) {
            return not_found();
        }
        return match std::fs::read(&file) {
            Ok(body) => ProtocolReply {
                status: 200,
                mime: mime_for(&file),
                body,
            },
            Err(_) => not_found(),
        };
    }

    // vendored 资产:先逐段解码,再拒绝路径穿越——编码形态也拦得住:
    // %2e%2e 解码成 '..' 后才比较;%2F 解码出的 '/' 直接判拒。
    // (审阅修正:初版先比较后解码,%2e%2e 可绕过,PoC 已实证。)
    let mut full = assets_root.to_path_buf();
    for seg in path.split('/') {
        let Some(seg) = percent_decode(seg) else {
            return not_found();
        };
        if seg.is_empty() || seg == ".." || seg == "." || seg.contains('/') || seg.contains('\0') {
            return not_found();
        }
        full.push(seg);
    }
    match std::fs::read(&full) {
        Ok(body) => ProtocolReply {
            status: 200,
            mime: mime_for(&full),
            body,
        },
        Err(_) => not_found(),
    }
}
```

并在 `main.rs` 加 `mod assets;`。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app assets::`
Expected: 4 passed。

- [x] **Step 5: Commit**

```bash
cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app
git add crates/dozer-app/src && git commit -m "feat(预览): dozer:// 协议应答——vendored 资产 + 白名单文件端点（headless 全测）

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: workspace 预览 pane UI + 内容区 bounds 公式

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(Message 扩展、Workspace 字段、`preview_pane()` 替换 col2 占位、`preview_content_bounds()`)

**Interfaces:**
- Consumes: Task 2 `PreviewPane` 全部方法、`AddrTarget`、`WebviewSpec`。
- Produces(Task 5 依赖):
  - `Message` 新变体:`PreviewOpenPath(PathBuf)`、`PreviewOpenUrl(String)`、`PreviewSelectTab(usize)`、`PreviewCloseTab(usize)`、`PreviewAddrClick`、`PreviewAddrEvent(AddrEvent)`、`PreviewPickFile`
  - `pub enum AddrEvent { Text(String), Backspace, Submit, Cancel }`(定义在 workspace.rs)
  - `Workspace::preview_addr_editing(&self) -> bool`
  - `Workspace::preview_desired(&self) -> Vec<WebviewSpec>`
  - `Workspace::allowed_files(&self) -> Arc<Mutex<HashSet<PathBuf>>>`(clone 给协议闭包)
  - `pub fn preview_content_bounds(window_width: f32, window_height: f32) -> (f32, f32, f32, f32)`(逻辑像素 x/y/w/h)

- [x] **Step 1: 写失败测试**

`workspace.rs` 尾部新增测试模块(该文件此前无测试):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_content_bounds_is_inside_col2() {
        let (x, y, w, h) = preview_content_bounds(1440.0, 900.0);
        // 左二起点 = 左一宽 240 + pane 内边距;宽 = (1440-240-280)/2 附近
        assert!(x > PROJECT_COL_WIDTH && x < PROJECT_COL_WIDTH + 20.0, "x={x}");
        assert!((420.0..=470.0).contains(&w), "w={w}");
        assert!(y > 60.0 && y < 130.0, "y={y}(表头+tab 栏+地址栏之下)");
        assert!(h > 700.0 && h < 900.0 - y, "h={h}");
    }

    #[test]
    fn preview_content_bounds_never_negative() {
        let (_, _, w, h) = preview_content_bounds(100.0, 50.0);
        assert!(w >= 0.0 && h >= 0.0);
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app workspace::`
Expected: 编译错误(`preview_content_bounds` 不存在)。

- [x] **Step 3: 实现**

workspace.rs 变更点(完整代码):

1. 头部 use 增加:

```rust
use crate::preview::{AddrTarget, PreviewPane, WebviewSpec};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
```

2. 常量(放在 `CHROME_HEIGHT_PX` 之后)与 bounds 公式:

```rust
/// 左二内容区上方的 chrome 高度:pane 上内边距 8 + 表头行 22 + tab 栏 30
/// + 地址栏 30 + 三处 spacing 4*3。与终端 pane 的 CHROME 同为估算值,
/// 差几像素只影响 webview 与边框的贴合度,不影响可用性。
const PREVIEW_CHROME_TOP_PX: f32 = 8.0 + 22.0 + 30.0 + 30.0 + 12.0;

/// 窗口逻辑尺寸 → 左二内容区矩形(逻辑像素 x/y/w/h)。列宽公式与
/// `terminal_pane_pixel_size` 同源:左一/左四固定宽,预览与终端均分 Fill。
pub fn preview_content_bounds(window_width: f32, window_height: f32) -> (f32, f32, f32, f32) {
    let fill_width = (window_width - PROJECT_COL_WIDTH - AI_COL_WIDTH).max(0.0);
    let x = PROJECT_COL_WIDTH + 8.0;
    let y = PREVIEW_CHROME_TOP_PX;
    let w = (fill_width / 2.0 - 16.0).max(0.0);
    let h = (window_height - y - 8.0).max(0.0);
    (x, y, w, h)
}
```

3. `Message` 新变体(追加到 enum 尾部,含文档注释)与 `AddrEvent`:

```rust
    /// 预览:打开本地文件为新 tab(路径已由入口侧确认存在)。
    PreviewOpenPath(PathBuf),
    /// 预览:打开 URL 为新网页 tab。
    PreviewOpenUrl(String),
    /// 预览:切换 tab(vec 位置)。
    PreviewSelectTab(usize),
    /// 预览:关闭 tab(vec 位置)。
    PreviewCloseTab(usize),
    /// 预览:点击地址栏,进入编辑态(此后键盘输入路由到地址栏)。
    PreviewAddrClick,
    /// 预览:地址栏编辑事件(main.rs 键盘拦截层翻译后送入)。
    PreviewAddrEvent(AddrEvent),
    /// 预览:"打开文件…"按钮 → rfd 原生选择器(main.rs 侧执行,选中后
    /// 回送 PreviewOpenPath)。
    PreviewPickFile,
```

```rust
/// 地址栏编辑事件:由 main.rs 的键盘拦截层在 `preview_addr_editing()`
/// 为真时翻译产生(字符/退格/回车/Esc),不经过 keymap 的 PTY 字节翻译。
#[derive(Debug, Clone)]
pub enum AddrEvent {
    Text(String),
    Backspace,
    Submit,
    Cancel,
}
```

4. `Workspace` 字段增加(`daemon_error` 之后)并在两个构造函数里初始化:

```rust
    /// 预览域状态机(P1d)。
    preview: PreviewPane,
    /// `dozer://flyfish/__file__` 端点的文件白名单;与 main.rs 的协议
    /// 闭包共享(Arc),打开文件时插入。
    allowed_files: Arc<Mutex<HashSet<PathBuf>>>,
```

(两个构造函数各加 `preview: PreviewPane::default(), allowed_files: Arc::new(Mutex::new(HashSet::new())),`)

5. `update` 新增 match 分支(尾部):

```rust
            Message::PreviewOpenPath(path) => {
                if !path.is_file() {
                    self.daemon_error = Some(format!("文件不存在或不可读: {}", path.display()));
                    return;
                }
                self.allowed_files.lock().expect("allowed_files 锁").insert(path.clone());
                self.preview.open_path(path);
            }
            Message::PreviewOpenUrl(url) => {
                self.preview.open_url(url);
            }
            Message::PreviewSelectTab(idx) => self.preview.select(idx),
            Message::PreviewCloseTab(idx) => self.preview.close(idx),
            Message::PreviewAddrClick => self.preview.addr_begin(),
            Message::PreviewAddrEvent(ev) => match ev {
                AddrEvent::Text(s) => self.preview.addr_text(&s),
                AddrEvent::Backspace => self.preview.addr_backspace(),
                AddrEvent::Cancel => self.preview.addr_cancel(),
                AddrEvent::Submit => match self.preview.addr_submit() {
                    Some(AddrTarget::File(path)) => self.update(Message::PreviewOpenPath(path)),
                    Some(AddrTarget::Url(url)) => self.update(Message::PreviewOpenUrl(url)),
                    None => {}
                },
            },
            Message::PreviewPickFile => {} // 副作用在 main.rs(rfd 模态需窗口句柄侧执行)
```

6. 访问器(impl Workspace 内):

```rust
    /// 地址栏是否在编辑态(main.rs 据此路由键盘:真 → AddrEvent,
    /// 假 → keymap → PTY)。
    pub fn preview_addr_editing(&self) -> bool {
        self.preview.addr_editing()
    }

    /// 当前应存在的 webview 清单(main.rs 差集同步)。
    pub fn preview_desired(&self) -> Vec<WebviewSpec> {
        self.preview.desired_webviews()
    }

    /// 协议闭包共享的文件白名单句柄。
    pub fn allowed_files(&self) -> Arc<Mutex<HashSet<PathBuf>>> {
        Arc::clone(&self.allowed_files)
    }
```

7. `view()` 的 col2 从占位换成 `preview_pane(self)`,并实现视图函数(放在 `terminal_pane` 之前;样式全部复用既有 helper 手法):

```rust
/// 左二预览 pane:表头 + tab 栏 + 地址栏;内容区本体是 wry webview
/// 子视图(不在 iced 树里),这里只留占位背景——无 tab 时显示提示文案。
fn preview_pane(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = text("预览 · P1d").size(13).color(theme::CREAM);

    // tab 栏:每 tab 选择按钮 + 关闭 ×,尾接"打开文件…"。
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = ws
        .preview
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let active = idx == ws.preview.active_idx();
            let select = button(text(tab.title.clone()).size(12).color(theme::CREAM))
                .on_press(Message::PreviewSelectTab(idx))
                .style(move |_t, _s| button::Style {
                    background: Some(if active { theme::CARD } else { theme::PANEL }.into()),
                    text_color: theme::CREAM,
                    border: Border {
                        color: if active { theme::CREAM } else { theme::BORDER },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    ..button::Style::default()
                });
            let close = button(text("×").size(12).color(theme::DIM))
                .on_press(Message::PreviewCloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                });
            row![select, close].spacing(2).into()
        })
        .collect();
    items.push(
        button(text("打开文件…").size(12).color(theme::CREAM))
            .on_press(Message::PreviewPickFile)
            .style(|_t, _s| button::Style {
                background: Some(theme::CARD.into()),
                text_color: theme::CREAM,
                border: Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..button::Style::default()
            })
            .into(),
    );
    let tab_bar = row(items).spacing(4);

    // 地址栏:自绘(非 text_input——键盘路由走 main.rs 拦截层,与终端
    // 的键盘模型保持同一套显式焦点语义)。编辑态 GOLD 描边 + 光标条。
    let editing = ws.preview.addr_editing();
    let addr_text = if editing {
        format!("{}▏", ws.preview.addr_buffer())
    } else {
        "输入 localhost 端口、URL 或文件路径…".to_string()
    };
    let addr = button(
        text(addr_text)
            .size(12)
            .color(if editing { theme::CREAM } else { theme::DIM }),
    )
    .on_press(Message::PreviewAddrClick)
    .width(Length::Fill)
    .style(move |_t, _s| button::Style {
        background: Some(theme::TERM_BG.into()),
        text_color: theme::CREAM,
        border: Border {
            color: if editing { theme::GOLD } else { theme::BORDER },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..button::Style::default()
    });

    let mut content = column![header, tab_bar, addr].spacing(4);
    if ws.preview.tabs().is_empty() {
        content = content.push(
            container(
                text("暂无预览——打开文件或输入地址")
                    .size(13)
                    .color(theme::DIM),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(8))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
```

`view()` 中 `let col2 = pane("预览 · P1d", 0.0, theme::PANEL);` 改为 `let col2 = preview_pane(self);`。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app`
Expected: 全绿(既有 42 + 新增 2)。

- [x] **Step 5: Commit**

```bash
cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app
git add crates/dozer-app/src && git commit -m "feat(预览): workspace 接入预览域——消息/视图/内容区 bounds 公式

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: main.rs webview 生命周期 + 键盘路由 + rfd

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`(deps 增 `wry = "0.55.1"`、`rfd = "0.15"`)
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: Task 2 `WebviewSpec`;Task 3 `assets::{handle_protocol, assets_root}`;Task 4 全部消息与 `preview_content_bounds`/`preview_addr_editing`/`allowed_files`。
- Produces: 运行态行为契约(Task 6 人工验收依据)——tab 状态变更后 webview 集合与之同步;resize 后 bounds 跟随;地址栏编辑态键盘不进 PTY。

- [x] **Step 1: 加依赖**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
cargo add wry@0.55.1 rfd@0.15 -p dozer-app
cargo build -p dozer-app
```

Expected: 编译通过(rfd 若 0.15 不存在则用 `cargo add rfd -p dozer-app` 取最新,记录实际版本)。

- [x] **Step 2: 实现接线**(集成层,无新 headless 测试;正确性由 Task 2-4 测试 + Task 6 人工验收覆盖)

main.rs 变更点:

1. `Runner::Ready` 增加字段:

```rust
            /// 预览 webview 池:tab id → (句柄, 当前已加载 URL)。句柄只在
            /// 本事件环存取(spike 约束 2);URL 缓存用于导航去重。
            webviews: std::collections::HashMap<usize, (wry::WebView, String)>,
```

(`resumed()` 构造 `Ready` 时初始化 `webviews: std::collections::HashMap::new(),`)

2. 同步函数(impl Runner 内新增方法;在 `user_event` 尾部、`window_event` 的消息处理循环之后、`Resized` 分支之后各调用一次 `self.sync_previews();`):

```rust
        /// 把 workspace 的 webview 期望清单同步到真实 wry 子视图:
        /// 建缺失、毁多余、对齐可见性与 bounds、URL 变更时导航。
        fn sync_previews(&mut self) {
            let Self::Ready {
                window, workspace, webviews, ..
            } = self
            else {
                return;
            };
            let specs = workspace.preview_desired();
            let desired_ids: std::collections::HashSet<usize> =
                specs.iter().map(|s| s.id).collect();
            webviews.retain(|id, _| desired_ids.contains(id));

            let size = window.inner_size();
            let scale = window.scale_factor();
            let logical_w = size.width as f32 / scale as f32;
            let logical_h = size.height as f32 / scale as f32;
            let (x, y, w, h) =
                workspace::preview_content_bounds(logical_w, logical_h);
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
            };

            for spec in specs {
                match webviews.get_mut(&spec.id) {
                    Some((view, loaded_url)) => {
                        if *loaded_url != spec.url {
                            if let Err(e) = view.load_url(&spec.url) {
                                tracing::warn!("预览导航失败: {e}");
                            }
                            *loaded_url = spec.url.clone();
                        }
                        let _ = view.set_bounds(bounds);
                        let _ = view.set_visible(spec.visible);
                    }
                    None => {
                        let allowed = workspace.allowed_files();
                        let root = assets::assets_root();
                        let built = wry::WebViewBuilder::new()
                            .with_url(&spec.url)
                            .with_bounds(bounds)
                            .with_visible(spec.visible)
                            .with_custom_protocol("dozer".into(), move |_id, request| {
                                let allowed = allowed.lock().expect("allowed_files 锁");
                                let reply = assets::handle_protocol(
                                    &root,
                                    &allowed,
                                    &request.uri().to_string(),
                                );
                                wry::http::Response::builder()
                                    .status(reply.status)
                                    .header("Content-Type", reply.mime)
                                    .body(std::borrow::Cow::Owned(reply.body))
                                    .expect("构造协议应答")
                            })
                            .build_as_child(window.as_ref());
                        match built {
                            Ok(view) => {
                                webviews.insert(spec.id, (view, spec.url.clone()));
                            }
                            Err(e) => tracing::error!("创建预览 webview 失败: {e}"),
                        }
                    }
                }
            }
        }
```

(顶部 `use` 区无需新增——`wry::`/`assets::` 全限定引用。)

3. 键盘路由:`on_window_event` 里 ⌘ 守卫之后、终端翻译之前插入地址栏路由:

```rust
            // 地址栏编辑态:键盘直达地址栏(不经 keymap、不进 PTY)。
            if workspace.preview_addr_editing() {
                let addr_event = match event {
                    WindowEvent::KeyboardInput {
                        event,
                        is_synthetic: false,
                        ..
                    } if event.state == ElementState::Pressed => {
                        use winit::keyboard::{Key, NamedKey};
                        match &event.logical_key {
                            Key::Character(s) => Some(workspace::AddrEvent::Text(s.to_string())),
                            Key::Named(NamedKey::Space) => {
                                Some(workspace::AddrEvent::Text(" ".into()))
                            }
                            Key::Named(NamedKey::Backspace) => {
                                Some(workspace::AddrEvent::Backspace)
                            }
                            Key::Named(NamedKey::Enter) => Some(workspace::AddrEvent::Submit),
                            Key::Named(NamedKey::Escape) => Some(workspace::AddrEvent::Cancel),
                            _ => None,
                        }
                    }
                    WindowEvent::Ime(Ime::Commit(text)) => {
                        Some(workspace::AddrEvent::Text(text.clone()))
                    }
                    _ => None,
                };
                if let Some(ev) = addr_event {
                    workspace.update(Message::PreviewAddrEvent(ev));
                    window.request_redraw();
                }
                return;
            }
```

4. rfd:`user_event`/消息处理循环里,`Message::PreviewPickFile` 在送入 `workspace.update` 之前拦截执行(两处消息入口都要;放一个 Runner 方法):

```rust
        /// PreviewPickFile 的副作用:原生文件选择器(模态,UI 线程短暂
        /// 阻塞可接受)。选中 → 直接转成 PreviewOpenPath 送 workspace。
        fn dispatch(&mut self, message: Message) {
            let Self::Ready { workspace, window, .. } = self else { return };
            match message {
                Message::PreviewPickFile => {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        workspace.update(Message::PreviewOpenPath(path));
                    }
                }
                other => workspace.update(other),
            }
            window.request_redraw();
        }
```

`user_event` 的 `workspace.update(event)` 与 `window_event` 消息循环里的 `workspace.update(message)` 均改为 `self.dispatch(...)` 形态(注意借用:`dispatch` 是 `&mut self` 方法,调用点需先结束对 `Ready` 字段的解构借用——`window_event` 中把 `for message in messages { workspace.update(message); }` 改为循环外收集后逐条 `self.dispatch(message)`,循环体移出解构块;实施时以借用检查通过为准,行为不变)。

5. `Resized` 分支尾部追加 `// bounds 同步由本函数末尾的 sync_previews 统一执行`(实际调用在 window_event 末尾统一加)。

- [x] **Step 3: 编译 + 冒烟**

```bash
cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app && cargo test -p dozer-app
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 测试全绿、零警告;app 存活 8 秒无 panic。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app Cargo.lock
git commit -m "feat(预览): webview 生命周期同步 + 地址栏键盘路由 + rfd 文件选择器

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: 全量回归 + 端到端人工验收 + 落档

**Files:**
- Modify: `docs/superpowers/specs/2026-07-18-dozer-p1d-preview-design.md`(验收结果落档,或另立 acceptance 文件按 P1c 惯例:`docs/superpowers/specs/2026-07-18-p1d-acceptance.md`)
- Modify: 本计划文件(勾选 checkbox)

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录;P1e 起点状态。

- [x] **Step 1: 全量回归**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
```

Expected: workspace 全绿零警告。

- [x] **Step 2: 用户人工验收(逐项 ✓/✗;验收权在用户,实施方不代签)**

```markdown
# P1d 人工验收清单(用户实机执行)
1. `cargo run -p dozer-app`:左二出现 tab 栏 + 地址栏 + 占位提示,四栏布局不破
2. "打开文件…"选 .md → Flyfish 渲染 Markdown,ByteBoy2077 底色不刺眼
3. 地址栏输入绝对路径打开 .png/.pdf/.rs 各一 → 三种管线正确渲染
4. 起个本地服务(如 `python3 -m http.server 3000`),地址栏输 `localhost:3000` → 网页 tab 可滚动可点击
5. 地址栏输 `https://example.com` → 外部 URL 正常
6. tab 切换:文件 ↔ 网页来回切,内容与滚动位保持;× 关闭后不留幽灵 webview
7. 窗口拖拽缩放:webview 矩形跟随左二,不遮终端/不露缝
8. 焦点:点进网页表单可输中文(IME);点回终端敲键正常;地址栏编辑态键盘不漏进终端
9. 断网(关 Wi-Fi)重复 2/3 → 文件预览完全离线可用
```

- [x] **Step 3: 结果落档 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-18-p1d-acceptance.md`(格式沿用 P1c:逐项结果表 + 反馈修复流水;含 ✗ 项与处置);全部通过后规格 §3 需求 2 追加"P1d 达成(<日期>),验收记录见 specs/2026-07-18-p1d-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1d 预览 pane 人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review(已执行)

1. **规格覆盖**:设计 §1 目标 1(文件预览)由 T1/T2/T3/T5 覆盖、目标 2(网页+地址栏)由 T2/T4/T5、目标 3(tab 管理)由 T2/T4、目标 4(bounds 契约)由 T4 公式 + T5 同步、目标 5(离线/不持久化)由 T3 协议 + T6 清单第 9 项;裁决 D1(rfd+地址栏)T4/T5、D2(每 tab webview)T2 `desired_webviews`+T5 池、D3(vendore+协议)T1/T3、D4(diff 只留变体注释)T2、D5(体积裁决)T1 Step 2。错误处理四条:不可读文件(T4 update 分支)、不支持格式(Flyfish 兜底,T6 清单 3)、网页加载失败(WKWebView 原生,T6 清单 5 顺带)、白名单外 404(T3 测试)。
2. **占位符扫描**:T5 Step 2 第 4 点的借用调整给了行为契约与调整方向(借用检查通过、行为不变),属对齐指引而非 TBD;其余任务代码完整。
3. **类型一致性**:`WebviewSpec{id,url,visible}`、`AddrTarget::{File,Url}`、`AddrEvent::{Text,Backspace,Submit,Cancel}`、`handle_protocol(&Path,&HashSet<PathBuf>,&str)->ProtocolReply{status,mime,body}`、`preview_content_bounds(f32,f32)->(f32,f32,f32,f32)` 在 T2/T3/T4/T5 间交叉引用一致;`encode_component` 编码与 T3 `percent_decode` 测试互逆;host.html 的 `__file__`+`encodeURI` 与 T3 端点解析对齐(encodeURI 保留 `/`,端点按整串 percent_decode)。
