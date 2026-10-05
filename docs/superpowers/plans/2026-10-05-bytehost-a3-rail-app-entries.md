# bytehost A3:图标栏应用条目 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 图标栏能出现、拖动、切换"应用"条目(`PanelKind::App`),并有一个与已安装应用集合对齐的纯函数;应用自己的 wry 视图、数据来源留给 A4。

**Architecture:** `PanelKind` 新增 `App(AppSlot)`,`AppSlot` 是进程内只增不减的 id 表的下标(保持 `PanelKind: Copy + Hash`,拖拽/悬停/选中代码零改动复用)。落盘用 `"app:<id>"` 字符串(内置面板编码逐字不变)。`PanelCatalog::accepts` 只校验内置面板,应用条目由 `RailLayout::sync_apps` 与已安装集合对齐。host 提供 `App::sync_installed_apps`,A3 里没有调用者(A4 接 dozerd 的应用列表)。

**Tech Stack:** Rust、iced 0.14、serde、现有 `panel_registry`/`chrome::rail`。**不新增依赖**(dozer-app 仍不依赖 `bytehost-apps`,host 侧自带 id 形状检查)。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(A3 行)、`2026-10-04-bytehost-extraction-roadmap.md`。

## Global Constraints

- 内置面板的落盘编码不变:`"Files"`、`"GroupChat"` 等变体名字符串(旧 `layout.json`/`panel_layouts.json` 必须原样可读)。
- 每条栏至少保留一个**内置**面板(应用条目会随卸载消失,栏不能变空)。
- 面板日志来源名规则不受影响:`App` 不是面板日志来源(`state.rs` 的 `log_name_tests` 里只是占位)。
- 不引入任何 webview:`PanelKind::App` 的 webview 矩形恒为空,`fire_panel_switch_in` 是 no-op。
- 字体/Toast/日志规则照常:本计划不新增面向用户的瞬时消息。
- 新增图标:byteui v0.4.1 没有应用图标,A3 用 `IconKind::LayoutList` 临时顶替(A4 再换)。

## Review Focus

- 磁盘里留着已卸载应用的 `app:<id>`:加载不 panic、不整体回落默认布局(`accepts` 容忍),随后 `sync_apps` 清掉。
- 项目存的 `left_view/right_view` 指向已卸载应用:`adopt_panel_layout` 退到该栏第一个面板,不悬空。
- 把一条栏的最后一个内置面板拖走(栏里还有应用):必须被拒,否则卸载后栏为空。
- 同一应用在两栏各出现一次(坏数据):`accepts` 判非法,回落默认。
- 畸形 id(`app:../x`、大写、空):反序列化报错而不是造出槽。

---

### Task 1: `AppSlot` + `PanelKind::App` + serde + 穷举分支

**Files:**
- Create: `crates/dozer-app/src/app/app_slots.rs`
- Modify: `crates/dozer-app/src/app/mod.rs`、`app/state.rs`、`app/layout.rs`、`app/update.rs`、`app/view.rs`、`panel_layouts.rs`、`webview_geometry.rs`、`panel_registry.rs`(仅测试里的穷举 match)

**Interfaces:**
- Produces: `AppSlot::intern(&str) -> Option<AppSlot>`(幂等,非法 id/表满返回 `None`)、`AppSlot::id(self) -> &'static str`、`PanelKind::App(AppSlot)`、`PanelDims.app_split`、`app_placeholder_pane`(占位内容)。

- [ ] **Step 1: 写失败测试。** 新建 `app_slots.rs`(下面的测试模块先写,`intern`/`id` 先 `todo!()`),并在 `state.rs` 末尾加 `panel_kind_serde_tests`(见 Step 3 的 diff 里的测试模块)。
- [ ] **Step 2: 运行确认 RED。** `cargo test -p dozer-app -- slot panel_kind_serde` → 编译失败(`PanelKind::App` 不存在)/测试失败。
- [ ] **Step 3: 实现。** 新建文件内容:

```rust
//! 应用面板槽(bytehost A3):`PanelKind::App(AppSlot)` 里的 `AppSlot` 是进程内的小整数,
//! 指向一张只增不减的 id 表。这样 `PanelKind` 仍是 `Copy + Hash`,拖拽/悬停/选中等现有代码
//! 原样复用;应用 id 本身(磁盘/落盘/标题用)经 [`AppSlot::id`] 取回。

use std::sync::Mutex;

/// 应用在进程内的槽号(不落盘;落盘用 id 字符串,见 `PanelKind` 的 serde)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppSlot(u16);

static TABLE: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// 宿主侧的 id 形状检查(与 `bytehost-apps` 的 `AppId` 同口径,但 host 不依赖那个 crate):
/// 1–63 个 `[a-z0-9-]`,不以 `-` 开头或结尾。
pub(crate) fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 63
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

impl AppSlot {
    /// 取(必要时分配)`id` 的槽;`id` 形状非法或槽表已满(65536 个)返回 `None`。
    /// 同一个 id 永远得到同一个槽。表只增不减(每个 id 只泄漏一次短字符串)。
    pub(crate) fn intern(id: &str) -> Option<Self> {
        if !valid_app_id(id) {
            return None;
        }
        let mut t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = t.iter().position(|s| *s == id) {
            return Some(Self(i as u16));
        }
        let i = u16::try_from(t.len()).ok()?;
        t.push(Box::leak(id.to_owned().into_boxed_str()));
        Some(Self(i))
    }

    /// 这个槽对应的应用 id。
    pub fn id(self) -> &'static str {
        let t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        t[usize::from(self.0)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_id_same_slot_and_roundtrips() {
        let a = AppSlot::intern("slot-test-a").unwrap();
        let b = AppSlot::intern("slot-test-b").unwrap();
        assert_ne!(a, b);
        assert_eq!(AppSlot::intern("slot-test-a"), Some(a));
        assert_eq!(a.id(), "slot-test-a");
        assert_eq!(b.id(), "slot-test-b");
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in ["", "-a", "a-", "A", "a_b", "a.b", "a/b", &"x".repeat(64)] {
            assert!(AppSlot::intern(bad).is_none(), "{bad:?}");
        }
        assert!(AppSlot::intern(&"x".repeat(63)).is_some());
    }
}
```

其余文件的改动(`mod.rs`/`state.rs`/`layout.rs`/`update.rs`/`view.rs`/`panel_layouts.rs`/`webview_geometry.rs`;`registry` 只取其中 `every_kind` 测试 match 那一行 `| App(_)`):

```diff
diff --git a/crates/dozer-app/src/app/mod.rs b/crates/dozer-app/src/app/mod.rs
index e4275eba..d60c8e61 100644
--- a/crates/dozer-app/src/app/mod.rs
+++ b/crates/dozer-app/src/app/mod.rs
@@ -13,4 +13,5 @@
 #[allow(clippy::module_inception)]
 mod app;
+mod app_slots;
 mod layout;
 mod message;
@@ -20,4 +21,5 @@ mod view;
 
 pub(crate) use app::*;
+pub use app_slots::AppSlot;
 pub(crate) use layout::*;
 pub(crate) use message::*;
diff --git a/crates/dozer-app/src/app/state.rs b/crates/dozer-app/src/app/state.rs
index 77b1b68e..96d50f72 100644
--- a/crates/dozer-app/src/app/state.rs
+++ b/crates/dozer-app/src/app/state.rs
@@ -12,4 +12,5 @@ use crate::panel_host::HoverSlot;
 use crate::workspace::Workspace;
 
+use super::AppSlot;
 use super::layout::{PanelDims, ShellLayout};
 
@@ -19,5 +20,5 @@ use super::layout::{PanelDims, ShellLayout};
 /// 不改名。Stage 1(这次)只做了类型统一 + 数据模型,渲染/交互仍各自
 /// 按 `left_view`/`right_view` 字段走(Stage 2/4 才遍历 `RailLayout`)。
-#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
 pub enum PanelKind {
     Files,
@@ -33,4 +34,55 @@ pub enum PanelKind {
     Usage,
     CodeHealth,
+    /// 一个已安装的第三方应用(bytehost A3)。条目由 `RailLayout::sync_apps` 按已安装应用集合增删,
+    /// 不在 `PanelCatalog` 的描述符里。
+    App(AppSlot),
+}
+
+/// 落盘/日志用的面板名:内置面板沿用变体名(与旧的 derive 输出逐字一致),应用面板是 `app:<id>`。
+const BUILTIN_NAMES: [(PanelKind, &str); 12] = [
+    (PanelKind::Files, "Files"),
+    (PanelKind::GitLog, "GitLog"),
+    (PanelKind::Todo, "Todo"),
+    (PanelKind::Project, "Project"),
+    (PanelKind::Database, "Database"),
+    (PanelKind::Ssh, "Ssh"),
+    (PanelKind::Web, "Web"),
+    (PanelKind::Agent, "Agent"),
+    (PanelKind::GroupChat, "GroupChat"),
+    (PanelKind::Conversations, "Conversations"),
+    (PanelKind::Usage, "Usage"),
+    (PanelKind::CodeHealth, "CodeHealth"),
+];
+
+impl Serialize for PanelKind {
+    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
+        match self {
+            PanelKind::App(slot) => s.serialize_str(&format!("app:{}", slot.id())),
+            other => {
+                let name = BUILTIN_NAMES
+                    .iter()
+                    .find(|(k, _)| k == other)
+                    .map(|(_, n)| *n)
+                    .expect("BUILTIN_NAMES 覆盖全部内置面板");
+                s.serialize_str(name)
+            }
+        }
+    }
+}
+
+impl<'de> Deserialize<'de> for PanelKind {
+    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
+        let name = String::deserialize(d)?;
+        if let Some(id) = name.strip_prefix("app:") {
+            return AppSlot::intern(id)
+                .map(PanelKind::App)
+                .ok_or_else(|| serde::de::Error::custom(format!("非法的应用面板 id {id:?}")));
+        }
+        BUILTIN_NAMES
+            .iter()
+            .find(|(_, n)| *n == name)
+            .map(|(k, _)| *k)
+            .ok_or_else(|| serde::de::Error::custom(format!("未知面板 {name:?}")))
+    }
 }
 
@@ -358,4 +410,5 @@ mod log_name_tests {
             PanelKind::Usage => "usage",
             PanelKind::CodeHealth => "code_health",
+            PanelKind::App(_) => "app",
         }
     }
@@ -444,2 +497,40 @@ mod log_name_tests {
     }
 }
+
+#[cfg(test)]
+mod panel_kind_serde_tests {
+    use super::{AppSlot, PanelKind};
+
+    #[test]
+    fn builtin_panels_keep_their_legacy_variant_name_encoding() {
+        assert_eq!(
+            serde_json::to_string(&PanelKind::GroupChat).unwrap(),
+            "\"GroupChat\""
+        );
+        assert_eq!(
+            serde_json::from_str::<PanelKind>("\"CodeHealth\"").unwrap(),
+            PanelKind::CodeHealth
+        );
+    }
+
+    #[test]
+    fn app_panels_roundtrip_as_app_colon_id() {
+        let k = PanelKind::App(AppSlot::intern("serde-app").unwrap());
+        let json = serde_json::to_string(&k).unwrap();
+        assert_eq!(json, "\"app:serde-app\"");
+        assert_eq!(serde_json::from_str::<PanelKind>(&json).unwrap(), k);
+    }
+
+    #[test]
+    fn unknown_or_malformed_names_are_rejected() {
+        for bad in [
+            "\"Nope\"",
+            "\"app:\"",
+            "\"app:Bad_Id\"",
+            "\"app:../x\"",
+            "\"files\"",
+        ] {
+            assert!(serde_json::from_str::<PanelKind>(bad).is_err(), "{bad}");
+        }
+    }
+}
diff --git a/crates/dozer-app/src/app/layout.rs b/crates/dozer-app/src/app/layout.rs
index bd19892f..ae0e8f67 100644
--- a/crates/dozer-app/src/app/layout.rs
+++ b/crates/dozer-app/src/app/layout.rs
@@ -120,4 +120,6 @@ pub struct PanelDims {
     /// 拿剩下的。语义同 `usage_split`(默认"内容在前、列表在后")。
     pub group_chat_split: f32,
+    /// 应用面板(bytehost A3)占位:A3 里应用面板没有配对分栏,只为让 `split`/`split_mut` 全覆盖。
+    pub app_split: f32,
 }
 
@@ -151,4 +153,5 @@ pub(crate) fn default_panel_dims() -> PanelDims {
         codehealth_split: byteui::theme::geometry::default_split_ratio(),
         group_chat_split: byteui::theme::geometry::default_split_ratio(),
+        app_split: byteui::theme::geometry::default_split_ratio(),
     }
 }
@@ -167,4 +170,5 @@ impl PanelDims {
             PanelKind::Agent => self.agent_split,
             PanelKind::GroupChat => self.group_chat_split,
+            PanelKind::App(_) => self.app_split,
             PanelKind::Conversations => self.conversations_split,
             PanelKind::Usage => self.usage_split,
@@ -184,4 +188,5 @@ impl PanelDims {
             PanelKind::Agent => &mut self.agent_split,
             PanelKind::GroupChat => &mut self.group_chat_split,
+            PanelKind::App(_) => &mut self.app_split,
             PanelKind::Conversations => &mut self.conversations_split,
             PanelKind::Usage => &mut self.usage_split,
@@ -203,5 +208,5 @@ impl PanelDims {
             PanelKind::GroupChat => Some(self.group_chat_list_collapsed),
             PanelKind::Usage => Some(self.usage_list_collapsed),
-            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth => None,
+            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth | PanelKind::App(_) => None,
         }
     }
@@ -218,5 +223,5 @@ impl PanelDims {
             PanelKind::GroupChat => Some(&mut self.group_chat_list_collapsed),
             PanelKind::Usage => Some(&mut self.usage_list_collapsed),
-            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth => None,
+            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth | PanelKind::App(_) => None,
         }
     }
@@ -329,4 +334,5 @@ pub fn sanitize_panel_dims(d: PanelDims) -> PanelDims {
         codehealth_split: clamp_split(d.codehealth_split),
         group_chat_split: clamp_split(d.group_chat_split),
+        app_split: clamp_split(d.app_split),
     }
 }
@@ -632,5 +638,6 @@ fn default_list_first(kind: PanelKind) -> bool {
         | PanelKind::GitLog
         | PanelKind::Web => true,
-        PanelKind::Agent
+        PanelKind::App(_)
+        | PanelKind::Agent
         | PanelKind::GroupChat
         | PanelKind::Conversations
@@ -1078,5 +1085,7 @@ mod drag_characterization_tests {
             PanelKind::GroupChat => dims.group_chat_list_collapsed = true,
             PanelKind::Usage => dims.usage_list_collapsed = true,
-            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth => return false,
+            PanelKind::GitLog | PanelKind::Web | PanelKind::CodeHealth | PanelKind::App(_) => {
+                return false;
+            }
         }
         true
diff --git a/crates/dozer-app/src/app/update.rs b/crates/dozer-app/src/app/update.rs
index b0afdc06..99e9293d 100644
--- a/crates/dozer-app/src/app/update.rs
+++ b/crates/dozer-app/src/app/update.rs
@@ -5202,5 +5202,5 @@ impl App {
                 crate::extensions::group_chat::on_activate(&mut ws.group_chat);
             }),
-            PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
+            PanelKind::Files | PanelKind::Web | PanelKind::Agent | PanelKind::App(_) => {}
         }
     }
diff --git a/crates/dozer-app/src/app/view.rs b/crates/dozer-app/src/app/view.rs
index 45034325..f74fa2f4 100644
--- a/crates/dozer-app/src/app/view.rs
+++ b/crates/dozer-app/src/app/view.rs
@@ -1134,7 +1134,28 @@ pub(crate) fn panel_body<'a>(
             }
         }
+        PanelKind::App(slot) => app_placeholder_pane(slot, zone_pane_border(zone, PaneCorner::All)),
     }
 }
 
+/// 应用面板的占位内容(bytehost A3):rail 条目与切换已经通了,应用自己的 wry 视图由 A4 接入。
+/// 只显示应用 id,不挂任何 webview(矩形恒为空,见 `webview_geometry`)。
+fn app_placeholder_pane<'a>(
+    slot: AppSlot,
+    border: Border,
+) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
+    container(
+        text(format!("应用 {}", slot.id()))
+            .size(byteui::theme::font::body())
+            .color(byteui::theme::color::current().dim),
+    )
+    .center(Length::Fill)
+    .style(move |_t: &iced_widget::Theme| container::Style {
+        background: Some(byteui::theme::color::current().panel.into()),
+        border,
+        ..container::Style::default()
+    })
+    .into()
+}
+
 /// 左面板区:按当前左视图组合"项目树+文件预览"配对或单个 Web 预览面板;
 /// 收起时渲染成空元素(不占宽度)。
diff --git a/crates/dozer-app/src/panel_layouts.rs b/crates/dozer-app/src/panel_layouts.rs
index f8ef5e68..e9c6dbf8 100644
--- a/crates/dozer-app/src/panel_layouts.rs
+++ b/crates/dozer-app/src/panel_layouts.rs
@@ -78,4 +78,5 @@ fn legacy_global_dims(layout_path: &Path) -> PanelDims {
             // 群聊面板两栏分栏同上。
             group_chat_split: byteui::theme::geometry::default_split_ratio(),
+            app_split: byteui::theme::geometry::default_split_ratio(),
         })
         .unwrap_or_default()
diff --git a/crates/dozer-app/src/webview_geometry.rs b/crates/dozer-app/src/webview_geometry.rs
index 918bef3c..d1f3a2cf 100644
--- a/crates/dozer-app/src/webview_geometry.rs
+++ b/crates/dozer-app/src/webview_geometry.rs
@@ -127,5 +127,6 @@ pub fn preview_content_bounds_for(
             | PanelKind::GroupChat
             | PanelKind::Usage
-            | PanelKind::CodeHealth => (0.0, 0.0, 0.0, 0.0),
+            | PanelKind::CodeHealth
+            | PanelKind::App(_) => (0.0, 0.0, 0.0, 0.0),
             // 审阅内容放大态:跟非放大态同一份 `!mirrored` 理由,只是
             // x0/avail_w/avail_h 换成放大盒子的换算(同 Files/Project 放大
@@ -259,7 +260,9 @@ pub fn preview_content_bounds_for(
         // Stage 4a 跨栏拖拽:该侧视图可为另一栏面板,纯 iced 绘制、该侧
         // 无 webview 可摆,装空矩形。
-        PanelKind::Agent | PanelKind::GroupChat | PanelKind::Usage | PanelKind::CodeHealth => {
-            (0.0, 0.0, 0.0, 0.0)
-        }
+        PanelKind::Agent
+        | PanelKind::GroupChat
+        | PanelKind::Usage
+        | PanelKind::CodeHealth
+        | PanelKind::App(_) => (0.0, 0.0, 0.0, 0.0),
         // 审阅内容(2026-08-21 webview trace 改造):跟 Files/Project 同款
         // "配对列宽 + preview chrome 高度"算法,但 `mirrored` 要取反——
```

- [ ] **Step 4: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app -- slot panel_kind_serde` → 全过(5 个)。
- [ ] **Step 5: 变异检查。** 把序列化改成 `s.serialize_str(slot.id())` → `app_panels_roundtrip_as_app_colon_id` 必须 FAILED;还原。
- [ ] **Step 6: Commit。** `git add crates/dozer-app && git commit -m "feat(dozer-app): PanelKind::App + AppSlot interning (A3 task 1)"`

### Task 2: 清单容忍 + `sync_apps` + 拖拽守卫 + `panel_meta`

**Files:** Modify `crates/dozer-app/src/panel_registry.rs`、`crates/dozer-app/src/chrome/rail.rs`

**Interfaces:**
- Consumes: Task 1 的 `AppSlot`/`PanelKind::App`。
- Produces: `RailLayout::sync_apps(&mut self, installed: &[AppSlot], default_side: Side) -> bool`、`RailLayout::view_or_first(&self, side: Side, view: PanelKind) -> PanelKind`、`panel_registry::APP_DEFAULT_SIDE`(= 左栏)、`catalog.default_side(App)`、`accepts` 只校内置面板。

- [ ] **Step 1: 写失败测试。** 下面 diff 里 `panel_registry.rs` 的 `accepts_ignores_app_entries_but_rejects_duplicates_among_them` 与 `rail.rs` 的 5 个 A3 测试(`sync_apps_*`、`view_or_first_*`、`cross_apply_never_strands_*`、`app_entries_have_title_icon_and_default_side`)。
- [ ] **Step 2: RED。** `cargo test -p dozer-app -- sync_apps view_or_first cross_apply_never app_entries accepts_ignores` → 失败(函数不存在/行为不对)。
- [ ] **Step 3: 实现:**

```diff
diff --git a/crates/dozer-app/src/panel_registry.rs b/crates/dozer-app/src/panel_registry.rs
index 33b636dd..22414c73 100644
--- a/crates/dozer-app/src/panel_registry.rs
+++ b/crates/dozer-app/src/panel_registry.rs
@@ -85,4 +85,7 @@ impl PanelCatalog {
     /// 面板默认挂在哪条栏;没注册的面板返回 `None`。
     pub(crate) fn default_side(&self, id: PanelKind) -> Option<Side> {
+        if matches!(id, PanelKind::App(_)) {
+            return Some(APP_DEFAULT_SIDE);
+        }
         if self.default_left.contains(&id) {
             Some(Side::Left)
@@ -99,10 +102,31 @@ impl PanelCatalog {
     }
 
-    /// `rail` 是不是一个合法布局:两栏都非空,且恰好是全部已注册面板(每个一次,不多不少)。
+    /// `rail` 是不是一个合法布局:**内置面板**两栏各至少一个,且合计恰好是全部已注册面板(每个一次,
+    /// 不多不少)。应用条目(`PanelKind::App`)不在清单里、数量随安装变化,所以不计入这些检查——
+    /// 只要求它们不重复;它们与已安装集合的对齐由 `RailLayout::sync_apps` 负责。
     pub(crate) fn accepts(&self, rail: &RailLayout) -> bool {
-        if rail.left.is_empty() || rail.right.is_empty() {
+        let builtin = |panels: &[PanelKind]| -> Vec<PanelKind> {
+            panels
+                .iter()
+                .copied()
+                .filter(|k| !matches!(k, PanelKind::App(_)))
+                .collect()
+        };
+        let (left, right) = (builtin(&rail.left), builtin(&rail.right));
+        if left.is_empty() || right.is_empty() {
+            return false;
+        }
+        let mut apps: Vec<PanelKind> = rail
+            .left
+            .iter()
+            .chain(rail.right.iter())
+            .copied()
+            .filter(|k| matches!(k, PanelKind::App(_)))
+            .collect();
+        apps.sort_by_key(|k| format!("{k:?}"));
+        if apps.windows(2).any(|w| w[0] == w[1]) {
             return false;
         }
-        let mut all: Vec<PanelKind> = rail.left.iter().chain(rail.right.iter()).copied().collect();
+        let mut all: Vec<PanelKind> = left.into_iter().chain(right).collect();
         all.sort_by_key(|k| format!("{k:?}"));
         if all.windows(2).any(|w| w[0] == w[1]) {
@@ -113,4 +137,7 @@ impl PanelCatalog {
 }
 
+/// 新安装的应用在图标栏里默认挂的栏(A3 固定左栏;要做成产品可配置再提进 `PanelCatalog`)。
+pub(crate) const APP_DEFAULT_SIDE: Side = Side::Left;
+
 static CATALOG: OnceLock<PanelCatalog> = OnceLock::new();
 
@@ -288,5 +315,5 @@ mod tests {
             match k {
                 Files | GitLog | Todo | Project | Database | Ssh | Web | Agent | GroupChat
-                | Conversations | Usage | CodeHealth => {}
+                | Conversations | Usage | CodeHealth | App(_) => {}
             }
         }
@@ -303,3 +330,24 @@ mod tests {
         }
     }
+    #[test]
+    fn accepts_ignores_app_entries_but_rejects_duplicates_among_them() {
+        use PanelKind::*;
+        let c = small();
+        let a = PanelKind::App(crate::app::AppSlot::intern("reg-app").unwrap());
+        assert!(
+            c.accepts(&rail(&[Todo, Files, a], &[Agent])),
+            "应用条目不计入清单"
+        );
+        assert!(c.accepts(&rail(&[Todo, Files], &[Agent, a])));
+        assert!(
+            !c.accepts(&rail(&[Todo, Files, a], &[Agent, a])),
+            "同一个应用出现两次"
+        );
+        assert!(
+            !c.accepts(&rail(&[a], &[Todo, Files, Agent])),
+            "左栏只有应用、没有内置面板"
+        );
+        assert_eq!(c.default_side(a), Some(APP_DEFAULT_SIDE));
+        assert!(c.descriptor(a).is_none(), "应用不在描述符里");
+    }
 }
diff --git a/crates/dozer-app/src/chrome/rail.rs b/crates/dozer-app/src/chrome/rail.rs
index cd376260..342b4ace 100644
--- a/crates/dozer-app/src/chrome/rail.rs
+++ b/crates/dozer-app/src/chrome/rail.rs
@@ -8,5 +8,5 @@
 //! `docs/superpowers/specs/2026-08-21-rail-extraction-pilot-design.md`。
 
-use crate::app::{App, HoverId, MaximizedPane, Message, PanelKind, Side};
+use crate::app::{App, AppSlot, HoverId, MaximizedPane, Message, PanelKind, Side};
 use crate::theme;
 use byteui::interaction::icons;
@@ -103,4 +103,40 @@ impl RailLayout {
 }
 
+impl RailLayout {
+    /// 让图标栏里的应用条目与已安装应用集合一致(bytehost A3):不在 `installed` 里的应用条目被移除,
+    /// 已安装但还没有条目的应用追加到 `default_side` 栏末尾(`installed` 的顺序);已有条目的位置与
+    /// 顺序保持不变。返回是否有改动。内置面板不受影响。
+    pub(crate) fn sync_apps(&mut self, installed: &[AppSlot], default_side: Side) -> bool {
+        let mut changed = false;
+        for side in [Side::Left, Side::Right] {
+            let panels = self.side_mut(side);
+            let before = panels.len();
+            panels.retain(|k| match k {
+                PanelKind::App(slot) => installed.contains(slot),
+                _ => true,
+            });
+            changed |= panels.len() != before;
+        }
+        for slot in installed {
+            let kind = PanelKind::App(*slot);
+            if !self.left.contains(&kind) && !self.right.contains(&kind) {
+                self.side_mut(default_side).push(kind);
+                changed = true;
+            }
+        }
+        changed
+    }
+
+    /// `view` 若是一个已不在 `side` 栏里的面板(应用被卸载),退到该栏第一个面板;否则原样返回。
+    pub(crate) fn view_or_first(&self, side: Side, view: PanelKind) -> PanelKind {
+        let panels = self.side(side);
+        if panels.contains(&view) {
+            view
+        } else {
+            panels.first().copied().unwrap_or(view)
+        }
+    }
+}
+
 /// 面板当前是否偏离了默认栏(`side_of(kind) != default_side()`)。抽成
 /// 自由函数单纯是为了让单元测试不必构造一个完整 `App`(它需要 `Client`/
@@ -264,4 +300,13 @@ pub(crate) fn rail_cross_apply(
         return None;
     }
+    // 每条栏至少保留一个内置面板(应用条目会随卸载消失,栏不能因此变空)。
+    let moved = source_panels[source_index];
+    let builtin_left = source_panels
+        .iter()
+        .filter(|k| !matches!(k, PanelKind::App(_)))
+        .count();
+    if !matches!(moved, PanelKind::App(_)) && builtin_left <= 1 {
+        return None;
+    }
     let kind = rail.side_mut(source_side).remove(source_index);
     let target_index = target_index.min(rail.side(target_side).len());
@@ -552,4 +597,8 @@ pub(crate) fn icon_rail(
 /// 面板 → (图标, 图标栏 tooltip 文案),取自面板清单(见 `product::dozer_catalog`)。
 fn panel_meta(kind: PanelKind) -> (icons::IconKind, &'static str) {
+    // 应用面板不在清单里:标题是应用 id,图标 A3 先用通用的列表图标(应用自带图标由 A4 接入)。
+    if let PanelKind::App(slot) = kind {
+        return (icons::IconKind::LayoutList, slot.id());
+    }
     let d = crate::panel_registry::catalog()
         .descriptor(kind)
@@ -1299,3 +1348,96 @@ mod tests {
         assert_eq!(sanitize_rail_layout(rail), RailLayout::default());
     }
+    // ---- bytehost A3:应用条目 ----
+
+    fn slot(id: &str) -> AppSlot {
+        AppSlot::intern(id).unwrap()
+    }
+
+    #[test]
+    fn sync_apps_appends_new_apps_to_the_default_side_in_installed_order() {
+        let mut rail = RailLayout::default();
+        let builtin_left = rail.left.clone();
+        let (a, b) = (slot("rail-a"), slot("rail-b"));
+        assert!(rail.sync_apps(&[a, b], Side::Left));
+        assert_eq!(
+            &rail.left[builtin_left.len()..],
+            &[PanelKind::App(a), PanelKind::App(b)]
+        );
+        assert_eq!(
+            &rail.left[..builtin_left.len()],
+            &builtin_left[..],
+            "内置面板顺序不动"
+        );
+        assert!(
+            !rail.sync_apps(&[a, b], Side::Left),
+            "幂等:再同步一次没有改动"
+        );
+    }
+
+    #[test]
+    fn sync_apps_keeps_user_placement_and_drops_uninstalled() {
+        let mut rail = RailLayout::default();
+        let (a, b) = (slot("rail-keep-a"), slot("rail-keep-b"));
+        rail.sync_apps(&[a, b], Side::Left);
+        // 用户把 a 拖到了右栏最前。
+        rail.left.retain(|k| *k != PanelKind::App(a));
+        rail.right.insert(0, PanelKind::App(a));
+        assert!(rail.sync_apps(&[a], Side::Left), "b 被卸载,有改动");
+        assert_eq!(rail.right[0], PanelKind::App(a), "仍在用户放的位置");
+        assert!(!rail.left.contains(&PanelKind::App(b)));
+        assert!(!rail.right.contains(&PanelKind::App(b)));
+        assert_eq!(
+            rail.left.len() + rail.right.len(),
+            12 + 1,
+            "内置 12 个 + 1 个应用"
+        );
+    }
+
+    #[test]
+    fn view_or_first_falls_back_only_when_the_view_vanished() {
+        let mut rail = RailLayout::default();
+        let a = slot("rail-view-a");
+        rail.sync_apps(&[a], Side::Left);
+        assert_eq!(
+            rail.view_or_first(Side::Left, PanelKind::App(a)),
+            PanelKind::App(a)
+        );
+        rail.sync_apps(&[], Side::Left);
+        assert_eq!(
+            rail.view_or_first(Side::Left, PanelKind::App(a)),
+            rail.left[0]
+        );
+        assert_eq!(
+            rail.view_or_first(Side::Left, PanelKind::Todo),
+            PanelKind::Todo
+        );
+    }
+
+    #[test]
+    fn cross_apply_never_strands_a_side_with_only_apps() {
+        let mut rail = RailLayout {
+            left: vec![PanelKind::Files, PanelKind::App(slot("rail-strand"))],
+            right: vec![PanelKind::Agent],
+        };
+        assert_eq!(
+            rail_cross_apply(&mut rail, Side::Left, 0, Side::Right, 0),
+            None,
+            "Files 是左栏最后一个内置面板,搬走会让左栏只剩应用"
+        );
+        assert_eq!(rail.left.len(), 2, "未改动");
+        assert_eq!(
+            rail_cross_apply(&mut rail, Side::Left, 1, Side::Right, 0),
+            Some(PanelKind::App(slot("rail-strand"))),
+            "应用条目可以随意搬"
+        );
+    }
+
+    #[test]
+    fn app_entries_have_title_icon_and_default_side() {
+        let a = slot("rail-meta");
+        let (icon, title) = panel_meta(PanelKind::App(a));
+        assert_eq!(title, "rail-meta");
+        assert_eq!(icon, icons::IconKind::LayoutList);
+        assert_eq!(PanelKind::App(a).default_side(), Side::Left);
+    }
 }
```

- [ ] **Step 4: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app -- sync_apps view_or_first cross_apply_never app_entries accepts_ignores` → 全过。
- [ ] **Step 5: 变异检查(每条都必须让对应测试 FAILED,再还原):** (a) `PanelKind::App(slot) => installed.contains(slot),` 改成 `PanelKind::App(_) => true,`;(b) `if !matches!(moved, PanelKind::App(_)) && builtin_left <= 1` 改成 `if false`;(c) `apps.windows(2).any(|w| w[0] == w[1])` 改成 `false`。
- [ ] **Step 6: Commit。** `git commit -am "feat(dozer-app): rail app entries — sync_apps, catalog tolerance, drag guard (A3 task 2)"`

### Task 3: host 同步入口 + 项目视图退路

**Files:** Modify `crates/dozer-app/src/app/app.rs`

**Interfaces:**
- Consumes: Task 2 的 `sync_apps`/`view_or_first`/`APP_DEFAULT_SIDE`。
- Produces: `App::sync_installed_apps(&mut self, installed: &[AppSlot])`(`#[allow(dead_code)]`,A4 才有调用者);`adopt_panel_layout` 对已消失应用的视图退到该栏第一个面板。

- [ ] **Step 1: 实现**(这一步的 `App` 需要 `Client`/事件循环,没有廉价夹具——逻辑全在 Task 2 已测的纯函数里,这里只是接线):

```diff
diff --git a/crates/dozer-app/src/app/app.rs b/crates/dozer-app/src/app/app.rs
index 4b66828c..3dc71a6e 100644
--- a/crates/dozer-app/src/app/app.rs
+++ b/crates/dozer-app/src/app/app.rs
@@ -2526,6 +2526,8 @@ impl App {
     pub(crate) fn adopt_panel_layout(&mut self, id: i64) {
         let pl = self.panel_layouts.get(&id).copied().unwrap_or_default();
-        self.left_view = pl.left_view;
-        self.right_view = pl.right_view;
+        // 项目存的视图可能是一个后来被卸载的应用:退到该栏第一个面板。
+        let rail = &self.shell_layout.rail_layout;
+        self.left_view = rail.view_or_first(Side::Left, pl.left_view);
+        self.right_view = rail.view_or_first(Side::Right, pl.right_view);
         // 磁盘数据可能来自 `end_rail_drag` 修复前写入的坏状态:两侧
         // active view 撞成同一个 `kind`,渲染时同一面板画两遍,复现过
@@ -2548,4 +2550,22 @@ impl App {
     }
 
+    /// 让图标栏里的应用条目与已安装应用集合一致(bytehost A3):卸载的应用条目消失,新装的追加到默认栏末尾。
+    /// 当前正显示着已消失应用的那一侧退到该栏第一个面板。有改动才存盘。A3 里还没有调用者——A4 在拿到
+    /// dozerd 的应用列表后调它。
+    #[allow(dead_code)]
+    pub(crate) fn sync_installed_apps(&mut self, installed: &[AppSlot]) {
+        let changed = self
+            .shell_layout
+            .rail_layout
+            .sync_apps(installed, crate::panel_registry::APP_DEFAULT_SIDE);
+        if !changed {
+            return;
+        }
+        let rail = &self.shell_layout.rail_layout;
+        self.left_view = rail.view_or_first(Side::Left, self.left_view);
+        self.right_view = rail.view_or_first(Side::Right, self.right_view);
+        self.on_shell_layout_changed();
+    }
+
     /// 把整份 `panel_layouts`(所有项目的面板布局)异步写盘。
     pub(crate) fn spawn_panel_layouts_save(&mut self) {
```

- [ ] **Step 2: 全量验证。** `cargo fmt -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo test -p dozer-app` → 预期仅 `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 与 `extensions::git_log::tests::build_marks_head_branch_and_labels` 两个**已在 main 上失败**的环境相关测试红(执行前先在 main 上确认),其余全绿(基线 1855 + 本计划新增 11 个)。
- [ ] **Step 3: Commit。** `git commit -am "feat(dozer-app): App::sync_installed_apps + stale-view fallback (A3 task 3)"`
- [ ] **Step 4: 文档。** 在 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` 的 A3 行后加一句"A3 已落地:见本计划;`App::sync_installed_apps` 待 A4 接线"。

## 已知局限(写在计划里,不是缺陷)

- A3 合并后图标栏里**看不到**应用条目:没有调用者喂已安装集合(A4)。启动时磁盘里若有 `app:<id>` 条目会原样显示占位页,直到 A4 的首次同步。
- 占位图标是 `LayoutList`;应用自带图标与 wry 视图在 A4。
- `PanelKind::App` 的全局槽表只增不减(每个 id 泄漏一次短字符串);65536 个 id 之后 `intern` 返回 `None`,调用方须跳过该应用。
