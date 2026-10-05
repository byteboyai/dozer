# bytehost A4b1:应用面板的 webview 机制 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `PanelKind::App` 面板能承载一个受限的 wry webview:id 段、几何、期望清单、**单独的受限构建路径**(无 `dozer://` 协议、IPC 白名单、仅本 origin 导航、每应用数据存储)、焦点路由。本片只做机制;谁来写"要加载的地址"(列表轮询、启动流程、面板状态机、不可用提示页)是 A4b2。

**Architecture:** 新模块 `app_webview.rs` 放纯判断(`AppOrigin`/导航策略、id 段、存储标识)与 `AppViews`(每应用当前地址,内存、含秘密)。`App::preview_desired` 对 `PanelKind::App` 直接产出 spec(无需工作区);`runtime.rs::sync_webview_pool` 在创建分支里按 id 段改走 `build_app_webview`;焦点用新消息 `AppWebViewFocused(id)` 反查槽。

**Tech Stack:** Rust、iced 0.14、wry 0.55(`with_navigation_handler`/`with_data_store_identifier`/`with_new_window_req_handler`)、`url` 2(dozer-app 已依赖)。**不新增依赖**。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(A4 行、§6.3/§6.4);探针结论见记忆 `bytehost-app-host-progress`(第三方应用 webview 不能复用现有池的特权)。

## Global Constraints

- **应用代码不可信**:应用 webview **不得**注册 `dozer://` 协议、不得处理 `focus/mouseup/zoom_*` 之外的 IPC 消息、不得打开新窗口、不得处理下载。
- 导航策略唯一真相是 `AppOrigin::allows_navigation`;wry 回调拿不到主/子框架,所以对每次导航生效。静态应用的出站网络**仍是 `Advisory`**(WebRTC/WebSocket 不被导航策略管,靠 CSP)——本片不得在文档里宣称 `Enforced`。
- 启动地址含一次性令牌(**秘密**):只放内存,不写日志、不落盘;被拒导航只记 scheme+host。
- `data_store_identifier` 算法一经发布**不能改**(测试钉死向量,且由独立的 Python 实现交叉验证)。
- 字体/Toast/日志规则照常;本片不新增面向用户的瞬时消息。`LOG` 来源沿用 `runtime`(module)。
- WebView 恒在 iced 之上:应用面板上方若出现 iced 内浮层,须经 `preview_desired` 的 `visible` 隐藏(沿用 `app_modal_open`;其余内浮层是否覆盖应用面板留给 A4b2 的面板 UI 评估)。

## Review Focus

- 导航绕过:`http://<id>.localhost@evil.com:端口/`、`<id>.localhost.evil.com`、端口不同、`https`、`file:`、`dozer:`、`data:`、`javascript:`、别家的 `blob:`——全部被拒(`navigation_stays_inside_the_apps_own_origin`)。
- 池里**没有**任何路径能让应用 webview 拿到 `dozer://` 协议:`build_app_webview` 里没有 `with_custom_protocol`(代码审查项,无自动化测试)。
- 地址形状不对时 fail closed:`AppOrigin::from_url` 为 `None` 就不创建(代码审查项)。
- 应用面板获得焦点时不被误判成 Files/Project/Browser 的 webview:`AppWebViewFocused` 路径 + `active_preview_tab_has_native_editor(App)` 恒为假。
- 放大态、收起、另一侧放大覆盖时应用 webview 矩形为零(`app_view_is_hidden_*`、`a_maximized_app_view_*`)。
- 命中测试:点在应用列内被路由给该应用(`clicks_inside_an_app_column_*`)。

---

### Task 1: `app_webview` 纯模块 + id 段 + 槽号互转

**Files:**
- Create: `crates/dozer-app/src/app_webview.rs`
- Modify: `crates/dozer-app/src/main.rs`(`mod app_webview;`)、`crates/dozer-app/src/app/mod.rs`(导出 `valid_app_id`)、`crates/dozer-app/src/app/app_slots.rs`(`index`/`from_index`)、`crates/dozer-app/src/app/app.rs`(只加常量 `APP_CONTENT_ID_OFFSET = 8_000_000`,其余 hunk 属于 Task 2)

**Interfaces:**
- Produces: `AppOrigin::{from_url, app_id, allows_navigation}`、`data_store_identifier(&str) -> [u8; 16]`、`is_app_webview_id`/`webview_id(AppSlot)`/`slot_for_webview_id`、`AppViews::{set_url, clear, url, spec}`、`app_webview_spec`、`AppSlot::{index, from_index}`、`APP_CONTENT_ID_OFFSET`。

- [x] **Step 1: 写失败测试。** 新文件里的测试模块(7 个)先写;`the_data_store_identifier_is_stable_and_per_app` 先放一个**错误的占位向量**。
- [x] **Step 2: RED。** `cargo test -p dozer-app -- app_webview` → 编译失败(模块不存在),补骨架后 `the_data_store_identifier_*` 失败。
- [x] **Step 3: 独立验证钉死的向量**:`python3 -c "O=0x6c62272e07bb014262b821756295c58d;P=0x0000000001000000000000000000013b;h=O\nfor b in b'bytehost-app:excalidraw': h^=b;h=(h*P)%(1<<128)\nprint(h.to_bytes(16,'big').hex())"` → `30cd3bc2160471a7ee03a52d8c2e59db`,把它写进测试。
- [x] **Step 4: 实现。**

```diff
diff --git a/crates/dozer-app/src/app_webview.rs b/crates/dozer-app/src/app_webview.rs
new file mode 100644
index 00000000..9050808b
--- /dev/null
+++ b/crates/dozer-app/src/app_webview.rs
@@ -0,0 +1,248 @@
+//! 应用面板的 webview(bytehost A4b1):已安装应用(`PanelKind::App`)在 rail 面板里用 wry 子视图加载
+//! `http://<app-id>.localhost:<端口>/`。**应用代码不可信**(第三方或 agent 生成),所以本模块只放三类东西:
+//!
+//! - 纯判断:这个 URL 是否仍在该应用自己的 origin 内([`AppOrigin::allows_navigation`],导航策略的唯一真相)、
+//!   池里的 webview id 段、每应用存储标识;
+//! - 状态:每个应用面板"当前要加载的地址"([`AppViews`],A4b2 的状态机往里写);
+//! - 期望清单项的构造([`app_webview_spec`])。
+//!
+//! 真正创建 webview(**不装** `dozer://` 协议、IPC 只留白名单)在 `runtime.rs::build_app_webview`。
+
+use std::collections::HashMap;
+
+use crate::app::{APP_CONTENT_ID_OFFSET, AppSlot};
+use crate::preview::WebviewSpec;
+
+/// 池里这个 id 是不是应用 webview(`APP_CONTENT_ID_OFFSET + 槽号`,段内最多 65536 个)。
+pub(crate) fn is_app_webview_id(id: usize) -> bool {
+    (APP_CONTENT_ID_OFFSET..APP_CONTENT_ID_OFFSET + 65_536).contains(&id)
+}
+
+pub(crate) fn webview_id(slot: AppSlot) -> usize {
+    APP_CONTENT_ID_OFFSET + slot.index()
+}
+
+/// `webview_id` 的反函数;不在应用段内返回 `None`。
+pub(crate) fn slot_for_webview_id(id: usize) -> Option<AppSlot> {
+    if !is_app_webview_id(id) {
+        return None;
+    }
+    AppSlot::from_index(id - APP_CONTENT_ID_OFFSET)
+}
+
+/// 一个应用的 origin(`http://<host>:<port>`)。从应用的**站点地址**解析(启动地址里带的令牌查询串不影响)。
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct AppOrigin {
+    host: String,
+    port: u16,
+}
+
+impl AppOrigin {
+    /// 只接受 `http://<id>.localhost:<端口>/…`:主机必须是 `<合法 app id>.localhost`,端口必须显式给出,
+    /// 不允许用户名/密码。其余(含 `https`、裸 `localhost`、IP)一律 `None`——应用只会由 gateway 以这个形状发布。
+    pub(crate) fn from_url(url: &str) -> Option<Self> {
+        let parsed = url::Url::parse(url).ok()?;
+        if parsed.scheme() != "http" || !parsed.username().is_empty() || parsed.password().is_some()
+        {
+            return None;
+        }
+        let host = parsed.host_str()?.to_ascii_lowercase();
+        let id = host.strip_suffix(".localhost")?;
+        if !crate::app::valid_app_id(id) {
+            return None;
+        }
+        Some(Self {
+            host,
+            port: parsed.port()?,
+        })
+    }
+
+    /// 应用 id(主机名 `<id>.localhost` 去掉后缀)。
+    pub(crate) fn app_id(&self) -> &str {
+        self.host.strip_suffix(".localhost").unwrap_or(&self.host)
+    }
+
+    /// 导航策略:**只放行本 origin**。wry 的回调拿不到"主框架还是子框架",对每一次导航都问,所以子框架同样适用:
+    /// 本 origin 的 `http` 地址、`about:blank`/`about:srcdoc`(应用常用的空 iframe)、
+    /// 以及本 origin 创建的 `blob:`。其余一律拒绝(别的站点、`file:`、`dozer:`、`data:`、`javascript:` ……)。
+    pub(crate) fn allows_navigation(&self, url: &str) -> bool {
+        if url == "about:blank" || url == "about:srcdoc" {
+            return true;
+        }
+        let inner = url.strip_prefix("blob:").unwrap_or(url);
+        let Ok(parsed) = url::Url::parse(inner) else {
+            return false;
+        };
+        parsed.scheme() == "http"
+            && parsed.username().is_empty()
+            && parsed.password().is_none()
+            && parsed
+                .host_str()
+                .is_some_and(|h| h.eq_ignore_ascii_case(&self.host))
+            && parsed.port() == Some(self.port)
+    }
+}
+
+/// 每应用的 WKWebView 数据存储标识(macOS 14+ 的 `WKWebsiteDataStore(forIdentifier:)`):由应用 id 确定性
+/// 派生,**不能改算法**——改了所有应用的本地数据都会"消失"(测试钉死了一个向量)。FNV-1a 128 位,无新依赖。
+pub(crate) fn data_store_identifier(app_id: &str) -> [u8; 16] {
+    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
+    const PRIME: u128 = 0x0000000001000000000000000000013b;
+    let mut h = OFFSET;
+    for b in b"bytehost-app:".iter().chain(app_id.as_bytes()) {
+        h ^= u128::from(*b);
+        h = h.wrapping_mul(PRIME);
+    }
+    h.to_be_bytes()
+}
+
+/// 每个应用面板"当前要加载的地址"。地址带一次性令牌(**秘密**):只存在内存里,不写日志、不落盘;
+/// 清掉它(应用停了/卸载)就会让池里对应的 webview 被回收。A4b1 里没有写入者,A4b2 的面板状态机写。
+#[derive(Debug, Default)]
+pub(crate) struct AppViews {
+    urls: HashMap<AppSlot, String>,
+}
+
+impl AppViews {
+    #[allow(dead_code)] // A4b2 的启动流程调用
+    pub(crate) fn set_url(&mut self, slot: AppSlot, url: String) {
+        self.urls.insert(slot, url);
+    }
+
+    #[allow(dead_code)] // A4b2
+    pub(crate) fn clear(&mut self, slot: AppSlot) {
+        self.urls.remove(&slot);
+    }
+
+    pub(crate) fn url(&self, slot: AppSlot) -> Option<&str> {
+        self.urls.get(&slot).map(String::as_str)
+    }
+
+    /// 该应用面板的 webview 期望项;没有地址(未运行)返回 `None`。
+    pub(crate) fn spec(&self, slot: AppSlot, visible: bool) -> Option<WebviewSpec> {
+        self.url(slot)
+            .map(|url| app_webview_spec(slot, url, visible))
+    }
+}
+
+pub(crate) fn app_webview_spec(slot: AppSlot, url: &str, visible: bool) -> WebviewSpec {
+    WebviewSpec {
+        id: webview_id(slot),
+        url: url.to_owned(),
+        visible,
+        editor_binding: None,
+        reserve: None,
+        loading_generation: None,
+        park_offscreen: false,
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn origin() -> AppOrigin {
+        AppOrigin::from_url("http://excalidraw.localhost:20001/?bh_token=secret").unwrap()
+    }
+
+    #[test]
+    fn origin_is_parsed_only_from_the_gateway_shaped_address() {
+        assert_eq!(
+            origin(),
+            AppOrigin::from_url("http://EXCALIDRAW.localhost:20001/other").unwrap(),
+            "大小写与路径/查询串不影响 origin"
+        );
+        for bad in [
+            "https://excalidraw.localhost:20001/",
+            "http://excalidraw.localhost/",
+            "http://localhost:20001/",
+            "http://127.0.0.1:20001/",
+            "http://Bad_Id.localhost:20001/",
+            "http://-a.localhost:20001/",
+            "http://u:p@excalidraw.localhost:20001/",
+            "dozer://flyfish/host.html",
+            "not a url",
+        ] {
+            assert!(AppOrigin::from_url(bad).is_none(), "{bad}");
+        }
+    }
+
+    #[test]
+    fn the_app_id_is_the_host_without_the_localhost_suffix() {
+        assert_eq!(origin().app_id(), "excalidraw");
+    }
+
+    #[test]
+    fn navigation_stays_inside_the_apps_own_origin() {
+        let o = origin();
+        for ok in [
+            "http://excalidraw.localhost:20001/",
+            "http://excalidraw.localhost:20001/a/b?x=1#h",
+            "HTTP://Excalidraw.Localhost:20001/up",
+            "about:blank",
+            "about:srcdoc",
+            "blob:http://excalidraw.localhost:20001/6f1c",
+        ] {
+            assert!(o.allows_navigation(ok), "{ok}");
+        }
+        for bad in [
+            "http://excalidraw.localhost:20002/",
+            "http://other.localhost:20001/",
+            "http://excalidraw.localhost.evil.com:20001/",
+            "http://evil.com/",
+            "https://excalidraw.localhost:20001/",
+            "http://u:p@excalidraw.localhost:20001/",
+            "http://excalidraw.localhost@evil.com:20001/",
+            "file:///etc/passwd",
+            "dozer://flyfish/host.html",
+            "data:text/html,<script>1</script>",
+            "javascript:alert(1)",
+            "blob:http://evil.com/6f1c",
+            "blob:https://excalidraw.localhost:20001/6f1c",
+            "about:config",
+            "",
+        ] {
+            assert!(!o.allows_navigation(bad), "{bad}");
+        }
+    }
+
+    #[test]
+    fn webview_ids_live_in_their_own_range_and_invert() {
+        let slot = AppSlot::intern("wv-ids").unwrap();
+        let id = webview_id(slot);
+        assert!(is_app_webview_id(id));
+        assert_eq!(slot_for_webview_id(id), Some(slot));
+        assert!(!is_app_webview_id(crate::app::GROUP_CHAT_CONTENT_ID_OFFSET));
+        assert!(!is_app_webview_id(APP_CONTENT_ID_OFFSET - 1));
+        assert!(!is_app_webview_id(APP_CONTENT_ID_OFFSET + 65_536));
+        assert_eq!(slot_for_webview_id(0), None);
+    }
+
+    /// 钉死:这个标识决定每个应用的持久存储,算法一变所有应用数据都"丢"。
+    #[test]
+    fn the_data_store_identifier_is_stable_and_per_app() {
+        assert_eq!(
+            data_store_identifier("excalidraw"),
+            // 同一算法的独立实现(Python)算出的值:30cd3bc2160471a7ee03a52d8c2e59db
+            [
+                0x30, 0xcd, 0x3b, 0xc2, 0x16, 0x04, 0x71, 0xa7, 0xee, 0x03, 0xa5, 0x2d, 0x8c, 0x2e,
+                0x59, 0xdb
+            ]
+        );
+        assert_ne!(data_store_identifier("a"), data_store_identifier("b"));
+    }
+
+    #[test]
+    fn views_produce_a_spec_only_while_an_address_is_set() {
+        let slot = AppSlot::intern("wv-views").unwrap();
+        let mut views = AppViews::default();
+        assert!(views.spec(slot, true).is_none());
+        views.set_url(slot, "http://wv-views.localhost:20001/?bh_token=t".into());
+        let spec = views.spec(slot, false).unwrap();
+        assert_eq!(spec.id, webview_id(slot));
+        assert!(!spec.visible);
+        assert!(spec.editor_binding.is_none() && spec.reserve.is_none());
+        views.clear(slot);
+        assert!(views.spec(slot, true).is_none());
+    }
+}
diff --git a/crates/dozer-app/src/app/app_slots.rs b/crates/dozer-app/src/app/app_slots.rs
index 6b03e1e9..5f6b6a43 100644
--- a/crates/dozer-app/src/app/app_slots.rs
+++ b/crates/dozer-app/src/app/app_slots.rs
@@ -39,4 +39,15 @@ impl AppSlot {
     }
 
+    /// 槽号(进程内、不落盘;给 webview id 段用)。
+    pub(crate) fn index(self) -> usize {
+        usize::from(self.0)
+    }
+
+    /// `index` 的反函数;槽还没分配返回 `None`。
+    pub(crate) fn from_index(index: usize) -> Option<Self> {
+        let t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
+        (index < t.len()).then_some(Self(index as u16))
+    }
+
     /// 这个槽对应的应用 id。
     pub fn id(self) -> &'static str {
diff --git a/crates/dozer-app/src/app/mod.rs b/crates/dozer-app/src/app/mod.rs
index d60c8e61..7a1e753b 100644
--- a/crates/dozer-app/src/app/mod.rs
+++ b/crates/dozer-app/src/app/mod.rs
@@ -22,4 +22,5 @@ mod view;
 pub(crate) use app::*;
 pub use app_slots::AppSlot;
+pub(crate) use app_slots::valid_app_id;
 pub(crate) use layout::*;
 pub(crate) use message::*;
diff --git a/crates/dozer-app/src/main.rs b/crates/dozer-app/src/main.rs
index 268300f1..6b759bce 100644
--- a/crates/dozer-app/src/main.rs
+++ b/crates/dozer-app/src/main.rs
@@ -1,3 +1,4 @@
 mod app;
+mod app_webview;
 mod assets;
 mod capabilities;
```

另在 `app/app.rs` 的 `GROUP_CHAT_CONTENT_ID_OFFSET` 之后加:

```rust
/// 应用面板 webview 的 id 段:`APP_CONTENT_ID_OFFSET + 槽号`(`AppSlot::index`,最多 65536 个)。
pub(crate) const APP_CONTENT_ID_OFFSET: usize = 8_000_000;
```

- [x] **Step 5: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app -- app_webview slot` → 全过(21 个)。
- [x] **Step 6: 变异检查(每条必须让对应测试 FAILED,再还原):** (a) 删掉 `&& parsed.port() == Some(self.port)`;(b) 把 `strip_prefix("blob:")` 那行改成 `let inner = url;`;(c) 把 `from_url` 里的用户名/密码检查删掉。
- [x] **Step 7: Commit。** `git commit -am "feat(dozer-app): app_webview — origin policy, id range, per-app store id (A4b1 task 1)"`

### Task 2: 几何 + 期望清单 + 命中测试 + 键盘闸门

**Files:** Modify `crates/dozer-app/src/webview_geometry.rs`、`crates/dozer-app/src/app/app.rs`

**Interfaces:**
- Consumes: Task 1 的 `AppViews`、`webview_id`、`APP_CONTENT_ID_OFFSET`。
- Produces: `preview_content_bounds_for` 对 `App` 返回整条面板区(无 chrome;放大态占满放大盒子);`is_in_preview_column` 对 `App` 命中;`App.app_views`;`App::preview_desired` 为 `App` 面板产出 spec;`App::active_preview_webview_id(App(slot))`;`active_preview_tab_has_native_editor(App)` 恒假。

- [x] **Step 1: 写失败测试。** `webview_geometry.rs` 的 4 个:`app_view_spans_the_whole_zone_with_no_chrome`、`app_view_is_hidden_when_collapsed_or_covered_by_the_other_sides_maximize`、`a_maximized_app_view_fills_the_maximize_box`、`clicks_inside_an_app_column_are_routed_to_that_app`(含 `app_state` 夹具:`rail_layout.sync_apps` 把应用放进栏里,否则 `side_of` 会 `unreachable!`)。
- [x] **Step 2: RED。** `cargo test -p dozer-app -- app_view clicks_inside` → 失败(App 的几何目前是零矩形/未命中)。
- [x] **Step 3: 实现。**

```diff
diff --git a/crates/dozer-app/src/webview_geometry.rs b/crates/dozer-app/src/webview_geometry.rs
index d1f3a2cf..d44e1394 100644
--- a/crates/dozer-app/src/webview_geometry.rs
+++ b/crates/dozer-app/src/webview_geometry.rs
@@ -98,4 +98,11 @@ pub fn preview_content_bounds_for(
                 (x, y, w, h)
             }
+            // 应用面板(bytehost A4b1):单栏、无 chrome,放大态占满整条放大盒子。
+            PanelKind::App(_) => {
+                let h = (avail_h - 8.0).max(0.0);
+                let x = x0 + 8.0;
+                let w = (avail_w - 16.0).max(0.0);
+                (x, y0, w, h)
+            }
             // Git 提交图是原生 Canvas 绘制,不挂 webview 子视图。
             PanelKind::GitLog
@@ -127,6 +134,5 @@ pub fn preview_content_bounds_for(
             | PanelKind::GroupChat
             | PanelKind::Usage
-            | PanelKind::CodeHealth
-            | PanelKind::App(_) => (0.0, 0.0, 0.0, 0.0),
+            | PanelKind::CodeHealth => (0.0, 0.0, 0.0, 0.0),
             // 审阅内容放大态:跟非放大态同一份 `!mirrored` 理由,只是
             // x0/avail_w/avail_h 换成放大盒子的换算(同 Files/Project 放大
@@ -260,9 +266,20 @@ pub fn preview_content_bounds_for(
         // Stage 4a 跨栏拖拽:该侧视图可为另一栏面板,纯 iced 绘制、该侧
         // 无 webview 可摆,装空矩形。
-        PanelKind::Agent
-        | PanelKind::GroupChat
-        | PanelKind::Usage
-        | PanelKind::CodeHealth
-        | PanelKind::App(_) => (0.0, 0.0, 0.0, 0.0),
+        PanelKind::Agent | PanelKind::GroupChat | PanelKind::Usage | PanelKind::CodeHealth => {
+            (0.0, 0.0, 0.0, 0.0)
+        }
+        // 应用面板(bytehost A4b1):单栏、无配对,占满整条面板区(同 `Web` 关掉收藏夹时的算法,
+        // 只是没有地址栏 chrome)。
+        PanelKind::App(_) => {
+            let y = y_top(0.0);
+            let h = h_for(y);
+            let zone_raw_w = match side {
+                Side::Left => left_zone_width(window_width, state),
+                Side::Right => right_zone_width(window_width, state),
+            };
+            let x = zone_x0 + 8.0 + m.left;
+            let w = (zone_raw_w - 16.0 - m.left - m.right).max(0.0);
+            (x, y, w, h)
+        }
         // 审阅内容(2026-08-21 webview trace 改造):跟 Files/Project 同款
         // "配对列宽 + preview chrome 高度"算法,但 `mirrored` 要取反——
@@ -846,5 +863,5 @@ pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Op
                     }
                 }
-                PanelKind::Web => x >= x0 && x < x0 + avail_w,
+                PanelKind::Web | PanelKind::App(_) => x >= x0 && x < x0 + avail_w,
                 PanelKind::Project => {
                     if state.dims.project_list_collapsed {
@@ -879,5 +896,5 @@ pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Op
                 }
             }
-            PanelKind::Web => x >= zone_x0 && x < zone_x0 + zone_w,
+            PanelKind::Web | PanelKind::App(_) => x >= zone_x0 && x < zone_x0 + zone_w,
             PanelKind::Project => {
                 if state.dims.project_list_collapsed {
@@ -984,4 +1001,79 @@ mod tests {
     }
 
+    // ---- bytehost A4b1:应用面板 ----
+
+    fn app_state(id: &str) -> (ShellState, crate::app::AppSlot) {
+        let slot = crate::app::AppSlot::intern(id).unwrap();
+        let mut state = test_state();
+        state
+            .layout
+            .rail_layout
+            .sync_apps(&[slot], crate::app::Side::Left);
+        state.left_view = PanelKind::App(slot);
+        (state, slot)
+    }
+
+    #[test]
+    fn app_view_spans_the_whole_zone_with_no_chrome() {
+        let (state, _) = app_state("geo-app-a");
+        let (x, y, w, h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
+        let m = theme::region::left_zone().margin;
+        assert_eq!(x, byteui::theme::geometry::icon_rail_width() + 8.0 + m.left);
+        assert_eq!(w, left_zone_width(1440.0, &state) - 16.0 - m.left - m.right);
+        assert_eq!(
+            y,
+            byteui::theme::geometry::top_bar_height() + m.top,
+            "没有 chrome 高度"
+        );
+        assert_eq!(
+            h,
+            900.0 - y - m.bottom - byteui::theme::geometry::status_bar_height(),
+            "底部扣 footbar"
+        );
+    }
+
+    #[test]
+    fn app_view_is_hidden_when_collapsed_or_covered_by_the_other_sides_maximize() {
+        let (state, _) = app_state("geo-app-b");
+        let collapsed = ShellState {
+            left_collapsed: true,
+            ..state.clone()
+        };
+        assert_eq!(
+            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &collapsed),
+            (0.0, 0.0, 0.0, 0.0)
+        );
+        let covered = ShellState {
+            maximized: Some(MaximizedPane::Right),
+            ..state
+        };
+        assert_eq!(
+            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &covered),
+            (0.0, 0.0, 0.0, 0.0)
+        );
+    }
+
+    #[test]
+    fn a_maximized_app_view_fills_the_maximize_box() {
+        let (state, _) = app_state("geo-app-c");
+        let state = ShellState {
+            maximized: Some(MaximizedPane::Left),
+            ..state
+        };
+        let (x, y, w, h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
+        let (x0, avail_w) = maximized_box_x_range(1440.0);
+        assert_eq!((x, w), (x0 + 8.0, avail_w - 16.0));
+        assert!(y > 0.0 && h > 0.0);
+    }
+
+    #[test]
+    fn clicks_inside_an_app_column_are_routed_to_that_app() {
+        let (state, slot) = app_state("geo-app-d");
+        assert_eq!(
+            is_in_preview_column(400.0, 1440.0, &state),
+            Some(PanelKind::App(slot))
+        );
+    }
+
     #[test]
     fn preview_content_bounds_files_collapsed_spans_whole_zone() {
diff --git a/crates/dozer-app/src/app/app.rs b/crates/dozer-app/src/app/app.rs
index 3dc71a6e..c2674e86 100644
--- a/crates/dozer-app/src/app/app.rs
+++ b/crates/dozer-app/src/app/app.rs
@@ -466,4 +466,6 @@ pub struct App {
     /// 群聊内容区 webview 推送状态(App 级,固定单槽)。
     pub(crate) group_chat_webview: crate::extensions::group_chat::WebviewPushState,
+    /// 应用面板当前要加载的地址(bytehost A4b1,见 `app_webview`)。
+    pub(crate) app_views: crate::app_webview::AppViews,
     /// 数据库面板 App 级状态(哪些驱动类型在"新增数据源"下拉里可选,
     /// 启动时读盘)——见 `extensions::database::AppState`。
@@ -658,4 +660,6 @@ pub(crate) const TODO_CONTENT_ID_OFFSET: usize = 6_000_000;
 /// 偏移冲突)。
 pub(crate) const GROUP_CHAT_CONTENT_ID_OFFSET: usize = 7_000_000;
+/// 应用面板 webview 的 id 段:`APP_CONTENT_ID_OFFSET + 槽号`(`AppSlot::index`,最多 65536 个)。
+pub(crate) const APP_CONTENT_ID_OFFSET: usize = 8_000_000;
 
 /// `wait_for_pending_exit_tasks` 允许在飞的关 tab 收尾请求跑完的总预算。
@@ -852,4 +856,5 @@ impl App {
             todo_webview: crate::extensions::todo::WebviewPushState::default(),
             group_chat_webview: crate::extensions::group_chat::WebviewPushState::default(),
+            app_views: crate::app_webview::AppViews::default(),
             database: database::AppState::load(),
             footbar: footbar::AppState::default(),
@@ -2400,4 +2405,11 @@ impl App {
     /// main.rs 焦点路由取句柄用。
     pub fn active_preview_webview_id(&self, kind: PanelKind) -> Option<usize> {
+        // 应用面板的 webview 不属于任何工作区(同一个应用跨项目共用),直接按槽号定 id。
+        if let PanelKind::App(slot) = kind {
+            return self
+                .app_views
+                .url(slot)
+                .map(|_| crate::app_webview::webview_id(slot));
+        }
         self.active_workspace()?.active_preview_webview_id(kind)
     }
@@ -3178,4 +3190,8 @@ impl App {
     /// 开着一个原生 tab 就会把按键错误地拦下来。
     pub fn active_preview_tab_has_native_editor(&self, kind: PanelKind) -> bool {
+        // 应用面板没有原生预览 tab;`preview_pane` 对未知面板会退到 Files 的窗格,会误拦应用的键盘。
+        if matches!(kind, PanelKind::App(_)) {
+            return false;
+        }
         self.active_workspace()
             .map(|ws| ws.active_preview_tab_has_native_editor(kind))
@@ -3586,4 +3602,21 @@ impl App {
                 continue;
             }
+            // 应用面板(bytehost A4b1):单栏 webview,地址由 `app_views` 给(A4b2 的启动流程写入,没写
+            // 之前没有 spec,面板显示占位页)。被 iced 内浮层盖住时同样要隐藏。
+            if let PanelKind::App(slot) = kind {
+                let bounds = webview_geometry::preview_content_bounds_for(
+                    side,
+                    window_width,
+                    window_height,
+                    &self.shell_state(),
+                );
+                if bounds.2 > 0.0
+                    && bounds.3 > 0.0
+                    && let Some(spec) = self.app_views.spec(slot, !app_modal_open)
+                {
+                    out.push((spec, bounds));
+                }
+                continue;
+            }
             if kind == PanelKind::GroupChat {
                 // 已失败(加载超时/渲染异常)时不挂载,原生占位页接管。群列表列
```

- [x] **Step 4: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app -- app_view clicks_inside` → 4 个全过。
- [x] **Step 5: 变异检查:** 把命中测试那行改回 `PanelKind::Web => ...`(去掉 `| PanelKind::App(_)`)→ `clicks_inside_an_app_column_*` 必须 FAILED;还原。
- [x] **Step 6: Commit。** `git commit -am "feat(dozer-app): app panel webview geometry, spec production and hit routing (A4b1 task 2)"`

### Task 3: 受限构建路径 + 焦点消息

**Files:** Modify `crates/dozer-app/src/runtime.rs`、`app/message.rs`、`app/update.rs`、`platform/window_events.rs`、文档(规格 A4 行、`CLAUDE.md`)

**Interfaces:**
- Consumes: Task 1 的 `AppOrigin`/`data_store_identifier`/`is_app_webview_id`/`slot_for_webview_id`;Task 2 的 spec 产出。
- Produces: `build_app_webview`(私有)、`APP_INIT_SCRIPT`、`Message::AppWebViewFocused(usize)`。

- [ ] **Step 1: 实现。**(创建 wry webview 需要窗口,没有廉价夹具;策略本体已在 Task 1 测过,这里是接线。)

```diff
diff --git a/crates/dozer-app/src/runtime.rs b/crates/dozer-app/src/runtime.rs
index cb6a2d35..e743d51c 100644
--- a/crates/dozer-app/src/runtime.rs
+++ b/crates/dozer-app/src/runtime.rs
@@ -218,4 +218,73 @@ pub(crate) struct PoolSyncOutcome {
 }
 
+/// 应用 webview 里注入的脚本:只转发焦点/拖拽松开/缩放三类按键与鼠标事件(与预览 webview 的前三件套同款,
+/// 但**没有**查找、光标样式、标题回报——应用页面自己管这些)。
+const APP_INIT_SCRIPT: &str = "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);document.addEventListener('mouseup',function(){window.ipc.postMessage('mouseup')},true);document.addEventListener('keydown',function(e){if(e.ctrlKey){var c=e.code,k=e.key;if(c==='Equal'||k==='+'||k==='='){e.preventDefault();window.ipc.postMessage('zoom_in');}else if(c==='Minus'||k==='-'){e.preventDefault();window.ipc.postMessage('zoom_out');}else if(c==='Digit1'||k==='1'){e.preventDefault();window.ipc.postMessage('zoom_reset');}}},true);";
+
+/// 创建一个应用 webview(`app_webview` 模块文档说明了信任模型)。与 `sync_webview_pool` 里给预览/浏览器
+/// 用的构建路径的区别——**应用代码不可信**:
+/// - 不注册 `dozer://` 自定义协议(预览 host/审阅快照/允许文件都只在那条协议后面);
+/// - IPC 只认 `focus`/`mouseup`/`zoom_*` 四类白名单消息,其余一律丢弃;
+/// - 导航只放行本应用 origin(`AppOrigin::allows_navigation`),`window.open`/新窗口一律拒绝,
+///   下载不处理(没有 download handler = 取消);
+/// - 每应用独立的 WKWebsiteDataStore(macOS 14+;更老的系统 wry 会退回默认存储)。
+///
+/// `spec.url` 不是该形状的应用地址时**不创建**(返回 `None`,记日志):fail closed。
+fn build_app_webview(
+    window: &winit::window::Window,
+    spec: &crate::preview::WebviewSpec,
+    bounds: wry::Rect,
+    proxy: winit::event_loop::EventLoopProxy<Message>,
+) -> Option<wry::WebView> {
+    let Some(origin) = crate::app_webview::AppOrigin::from_url(&spec.url) else {
+        dozer_core::log_error!(LOG, "应用 webview 的地址不是应用站点形状,拒绝创建");
+        return None;
+    };
+    let webview_id = spec.id;
+    let app_id = origin.app_id().to_owned();
+    let ipc_proxy = proxy;
+    let built = wry::WebViewBuilder::new()
+        .with_url(&spec.url)
+        .with_bounds(bounds)
+        .with_visible(spec.visible)
+        .with_allow_link_preview(false)
+        .with_data_store_identifier(crate::app_webview::data_store_identifier(&app_id))
+        .with_initialization_script(APP_INIT_SCRIPT)
+        .with_ipc_handler(move |req| {
+            let message = match req.body().as_str() {
+                "mouseup" => Message::WebViewMouseUp,
+                "focus" => Message::AppWebViewFocused(webview_id),
+                "zoom_in" => Message::ZoomIn,
+                "zoom_out" => Message::ZoomOut,
+                "zoom_reset" => Message::ZoomReset,
+                _ => return,
+            };
+            let _ = ipc_proxy.send_event(message);
+        })
+        .with_navigation_handler({
+            let app_id = app_id.clone();
+            move |url| {
+                let allowed = origin.allows_navigation(&url);
+                if !allowed {
+                    // 只记 scheme+host:被拒的地址可能带任意查询串。
+                    let target = url::Url::parse(&url)
+                        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
+                        .unwrap_or_else(|_| "无法解析的地址".into());
+                    dozer_core::log_warn!(LOG, app = %app_id, target = %target, "应用尝试离开自己的 origin,已拒绝");
+                }
+                allowed
+            }
+        })
+        .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
+        .build_as_child(window);
+    match built {
+        Ok(view) => Some(view),
+        Err(e) => {
+            dozer_core::log_error!(LOG, app = %app_id, "创建应用 webview 失败: {e}");
+            None
+        }
+    }
+}
+
 pub(crate) fn sync_webview_pool(
     window: &winit::window::Window,
@@ -409,4 +478,11 @@ pub(crate) fn sync_webview_pool(
                 let _ = view.set_visible(spec.visible || spec.park_offscreen);
             }
+            None if crate::app_webview::is_app_webview_id(spec.id) => {
+                // 应用面板(第三方/agent 生成的代码):走**单独的受限构建路径**,不装 `dozer://` 协议。
+                if let Some(view) = build_app_webview(window, &spec, bounds, proxy.clone()) {
+                    let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
+                    pool.insert(spec.id, (view, spec.url.clone()));
+                }
+            }
             None => {
                 let allowed = std::sync::Arc::clone(&allowed_files);
diff --git a/crates/dozer-app/src/app/message.rs b/crates/dozer-app/src/app/message.rs
index b6165e03..581ba9b1 100644
--- a/crates/dozer-app/src/app/message.rs
+++ b/crates/dozer-app/src/app/message.rs
@@ -563,4 +563,7 @@ pub enum Message {
     /// 判断归谁。
     WebViewFocused,
+    /// 应用 webview(`app_webview`)里的 mousedown:带 webview id,据此反查是哪个应用面板拿到了键盘焦点
+    /// (共用的 `WebViewFocused` 不带面板信息,会被误判成 Files/Project/Browser)。
+    AppWebViewFocused(usize),
     /// 子 webview 上的鼠标松开(winit 收不到,JS 经 IPC 发来)。目的是结束
     /// 页签拖拽:若用户把 tab 从 iced 表层一路拖进 webview 并在这里松开,
diff --git a/crates/dozer-app/src/app/update.rs b/crates/dozer-app/src/app/update.rs
index dfaacd4f..942a3afa 100644
--- a/crates/dozer-app/src/app/update.rs
+++ b/crates/dozer-app/src/app/update.rs
@@ -3939,4 +3939,6 @@ impl App {
             // App::update 无需处理。
             Message::WebViewFocused => {}
+            // 同上:只在 main.rs 的 dispatch 里设 pending_focus。
+            Message::AppWebViewFocused(_) => {}
             // 鼠标在子 webview 上松开(见 `WebViewMouseUp` 文档):一并结束页签
             // 拖拽,避免"松开还能继续拖"。
diff --git a/crates/dozer-app/src/platform/window_events.rs b/crates/dozer-app/src/platform/window_events.rs
index 573aec1f..a24b346f 100644
--- a/crates/dozer-app/src/platform/window_events.rs
+++ b/crates/dozer-app/src/platform/window_events.rs
@@ -2157,4 +2157,14 @@ impl Runner {
             *current_focus = FocusIntent::Terminal;
             app.blur_preview_editors();
+        } else if let Message::AppWebViewFocused(id) = &message {
+            // 应用 webview 拿到焦点:意图是"那个应用面板的 webview"(池里的 id 由槽号决定,见
+            // `app_webview::webview_id`);id 不在应用段(不会发生)就交回终端。
+            let intent = match crate::app_webview::slot_for_webview_id(*id) {
+                Some(slot) => FocusIntent::Preview(PanelKind::App(slot)),
+                None => FocusIntent::Terminal,
+            };
+            *pending_focus = Some(intent);
+            *current_focus = intent;
+            app.blur_preview_editors();
         } else if matches!(message, Message::WebViewFocused) {
             // 子 webview 上的 mousedown winit 收不到,JS 经 IPC 发来这条
```

- [x] **Step 2: 静态核对(代码审查项,逐条打勾)。** `build_app_webview` 里 **没有** `with_custom_protocol`、没有 `with_download_*`;IPC 的 `match` 只有 5 个分支且 `_ => return`;`from_url` 为 `None` 就返回 `None`。
- [x] **Step 3: 全量门禁。** `cargo fmt --check -p dozer-app && bash scripts/check-log-scope.sh && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets` → 仅 `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 与(环境相关的)`extensions::git_log::tests::build_marks_head_branch_and_labels` 这两个**已在 main 上失败**的测试红(执行前先在 main 上复核);clippy 无本片新增警告。
- [~] **Step 4: 手工冒烟(不提交)——本次未执行(无 GUI/dozerd 环境,agent 无法跑 wry 窗口)。**  临时在 `App::new` 里读环境变量 `DOZER_DEV_APP="<id>=<launch-url>"`,对 `AppSlot::intern(id)` 调 `app_views.set_url`,并手动 `sync_installed_apps`;用 `dozerd` 里装一个静态应用(`manifest.toml` + 一个含 `<a href="https://example.com">`、`window.open`、`<iframe src="about:blank">` 的页面),确认:页面能加载并带键盘焦点;点外链被拒(日志有"应用尝试离开自己的 origin");`window.open` 无反应;`about:blank` iframe 正常;放大/收起/切到别的面板时 webview 隐藏。结果写进最终汇报。测完撤销临时代码。
- [x] **Step 5: Commit + 文档。** `git commit -am "feat(dozer-app): restricted app webview builder + focus routing (A4b1 task 3)"`;同一提交里把规格 A4 行补上"A4b1 已完成:应用 webview 机制,见 `plans/2026-10-05-bytehost-a4b1-app-webview-mechanism.md`";`CLAUDE.md` 的 bytehost-apps 行后补一条关键裁决:**应用 webview 走 `runtime.rs::build_app_webview`,不得复用预览/浏览器的构建路径(那条路径装了 `dozer://` 协议与完整 IPC)**。

## 已知局限

- A4b1 合并后没有写入者调用 `AppViews::set_url`,应用面板仍显示占位页;A4b2 接线。
- 子框架与主框架同一策略:应用页面里嵌第三方 iframe(如 YouTube 嵌入)会被拒——一期有意收紧。
- 应用面板在首页(`AppPage::Home`)与无活动工作区时不产出 spec(沿用 `preview_desired` 的入口判断);要不要让应用跨项目常驻是 A4b2/产品决定。
- 其他 iced 内浮层(tab 溢出下拉、右键菜单)是否会被应用 webview 盖住:A4b1 只处理 `text_input_menu`(`app_modal_open`),其余在 A4b2 做面板 UI 时按 `preview_desired` 的惯例补。
- 下载一律拒绝(**显式** `with_download_started_handler(|_, _| false)`;本计划初稿写的"没有 download handler = 取消"是错的,见下方"评审后修订");是否需要"保存导出文件"留给产品决定(Excalidraw 导出 PNG 会用到,A5 验收时看)。

## 评审后修订(2026-10-05,独立安全评审 + 一轮修复,分支 `a4b1-fix`)

评审发现计划初稿对 wry 默认值的两个假设是错的,已修:

- **C1 下载默认放行**:wry 0.55.1 的默认 `download_started_handler` 返回 `true`,且部分下载路径(`shouldPerformDownload`)在导航策略**之前**就被放行,应用页面可以不经提示把文件写进 `~/Downloads`。修:`with_download_started_handler(|_, _| false)`;同时显式 `with_devtools(false)`(wry 默认在 debug 构建里开)。`build_app_webview_pins_the_restrictive_settings` 用源码扫描钉住这些配置(wry 默认值会反过来,没有廉价夹具能真的建一个 webview)。
- **C2 页面可抢键盘焦点/刷全局缩放**:页面自己调 `window.ipc.postMessage('focus')` 就能让应用 webview 成为 first responder(终端键盘被它收走),`zoom_in` 同理改写并落盘全局缩放。修:每个应用 webview 创建时生成随机 nonce,只写进注入脚本闭包;消息体必须是 `<nonce>:<动词>`(`AppIpc::parse`),注入脚本先把 `postMessage` 绑到局部变量(页面之后改写也截不到)、且只转发 `isTrusted` 的真实用户事件。已用 node 做了行为验证:合成事件不发送、页面劫持后的 `postMessage` 截不到 nonce。
- **I1 媒体权限**:wry 对摄像头/麦克风请求**无条件批准**(`wry_web_view_ui_delegate.rs`),目前仅靠 macOS TCC 挡着(Dozer 没有声明用途)。写进规格 §6.4;将来 Dozer 若加麦克风/摄像头功能,必须先给应用 webview 加授权闸门。

**留给 A4b2 的(已记录,不在本次修复内):**
- 应用 webview 在切面板/收起/另一侧放大/回首页/无项目/**每次切项目**(`webviews.clear()`)时被销毁重载,页面内状态丢失;A4b2 要决定隐藏常驻还是豁免应用 id。
- 同一槽的 URL 换成另一个 origin 时,旧 webview 持有旧 origin 的导航策略会拒绝新地址、面板停在失败态;应在 `AppOrigin` 变化时重建。
- Minor:`about:blank#x`/`about:blank?…` 被精确匹配拒绝;macOS 14 以前每应用存储退回默认存储。
- 预览/浏览器共用构建路径同样有"下载默认放行"与"媒体权限批准"——不在应用宿主范围,另行评估。
