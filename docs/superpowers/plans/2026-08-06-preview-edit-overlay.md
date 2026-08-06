# 预览编辑弹层 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给预览 tab 加"编辑"按钮，点击后以应用内模态弹层打开一个简单的原生文本编辑器（iced `text_editor` widget），可编辑并保存回磁盘；同时把 file tab 的渲染路径从内联拼 flyfish URL 抽成一个具名决策点，为未来接入 flyfish 以外的预览方式留口子。

**Architecture:** `PreviewPane`（`crates/dozer-app/src/preview.rs`，纯数据不碰 wry/iced）新增 `reload_nonce` 计数器 + `bump_reload()` + `is_editable_extension()` 三个小改动。`Workspace`（`crates/dozer-app/src/workspace.rs`）新增 `edit_session: Option<EditSession>` 字段承载编辑态（内含 `iced_widget::text_editor::Content`），新增 6 个 `Message::PreviewEdit*` 变体，对应 6 个 `Workspace::preview_edit_*` 纯方法（可脱离 tokio/wry 单测），`App::update()` 只做一行转发。视图侧：`preview_pane()` 的 tab chip 按扩展名白名单追加编辑按钮；`App::view()` 顶层 `stack!` 分支链新增编辑弹层分支（复用 `delete_confirm_popup` 的 dismiss+popup 套路）；编辑态时 `preview_desired()` 把所有 webview 强制 `visible=false`（复用 `TabKind::Acceptance` 已验证过的隐藏机制）。

**Tech Stack:** Rust, iced 0.14 (`iced_widget::text_editor`), 现有 wry webview 预览管线不变。

## Global Constraints

- 只用 iced 0.14 生态，不引入新依赖（`text_editor` 已在现有 `iced_widget` features 里可用，无需改 `Cargo.toml`）。
- 不做语法高亮、行号、多光标——"simple text editor"。
- 不做内容嗅探判断文本/二进制，纯扩展名白名单。
- 不做插件注册表/trait 体系；这次只留一个具名函数级决策点（`flyfish_url`）。
- 不做外部改动冲突检测：保存时直接覆盖磁盘文件。
- 中文注释/commit message 风格与仓库现状一致（简体中文，`//!`/`///` doc 注释）。

---

## Task 1: `preview.rs` — flyfish URL 决策点抽取 + 保存后刷新缓存的 nonce

**Files:**
- Modify: `crates/dozer-app/src/preview.rs:102-108`（`push_tab`）、`crates/dozer-app/src/preview.rs:7-12`（`PreviewTab` struct）、`crates/dozer-app/src/preview.rs:210-232`（`desired_webviews`）
- Test: `crates/dozer-app/src/preview.rs`（同文件 `#[cfg(test)] mod tests`，追加测试）

**Interfaces:**
- Produces: `PreviewTab.reload_nonce: u64`（新字段，默认 0）；`PreviewPane::bump_reload(&mut self, tab_id: usize)`（按 `PreviewTab.id` 查找并 `+1`）；`desired_webviews()` 的 URL 在 `reload_nonce > 0` 时带 `&_r={nonce}` 查询参数。Task 3 会调用 `ws.preview.bump_reload(session.tab_id)`。

- [ ] **Step 1: 写失败测试——`bump_reload` 让 URL 带上 `_r` 参数**

在 `crates/dozer-app/src/preview.rs` 的 `mod tests` 里追加（紧邻现有 `desired_webviews_builds_urls_and_visibility` 测试之后）：

```rust
    #[test]
    fn bump_reload_appends_query_param_and_only_affects_target_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        p.bump_reload(id0);
        let specs = p.desired_webviews();
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&_r=1"
        );
        assert_eq!(specs[1].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md");
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&_r=2"
        );
        // 未知 id 是 no-op,不 panic。
        p.bump_reload(9999);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib preview::tests::bump_reload_appends_query_param_and_only_affects_target_tab`
Expected: FAIL，报 `no method named 'bump_reload' found`。

- [ ] **Step 3: 实现——加字段 + `bump_reload` + 抽取 `flyfish_url`**

把 `crates/dozer-app/src/preview.rs:7-12` 的 `PreviewTab` 改成：

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    /// 保存编辑后 `+1`,驱动 `desired_webviews()` 换 URL 逼 `sync_webview_pool`
    /// 重新 `load_url`(同 URL 不会重载,flyfish 的 WKWebView 会一直显示
    /// 保存前的旧内容)。
    pub reload_nonce: u64,
}
```

把 `crates/dozer-app/src/preview.rs:102-108` 的 `push_tab` 改成：

```rust
    fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
        });
        self.active = self.tabs.len() - 1;
        id
    }
```

把 `crates/dozer-app/src/preview.rs:210-232` 的 `desired_webviews` 改成（新增 `flyfish_url` 辅助函数 + 在其后追加 `bump_reload` 方法）：

```rust
    /// webview 期望清单:每文件/网页 tab 一个,仅激活者可见(设计 D2)；
    /// 验收 tab 不产 webview,且它激活时其余 webview 全隐藏(iced 直绘 pane)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        // 验收 tab 是 iced 直绘的覆盖层,它激活时其余 webview 全隐藏。
        let overlay_active = self.acceptance_active();
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                let url = match &tab.kind {
                    TabKind::File(path) => {
                        let mut u = flyfish_url(path);
                        if tab.reload_nonce > 0 {
                            u.push_str(&format!("&_r={}", tab.reload_nonce));
                        }
                        u
                    }
                    TabKind::Web { url } => url.clone(),
                    TabKind::Acceptance => return None,
                };
                Some(WebviewSpec {
                    id: tab.id,
                    url,
                    visible: idx == self.active && !overlay_active,
                })
            })
            .collect()
    }

    /// 编辑保存后调用:按 `PreviewTab.id` 找到对应 tab,推进它的 reload
    /// 计数器。未知 id 是 no-op(tab 可能已被关闭)。
    pub fn bump_reload(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.reload_nonce += 1;
        }
    }
```

紧邻 `encode_component` 函数之后（原 `desired_webviews` 之前）新增：

```rust
/// `TabKind::File` → flyfish 渲染 URL 的唯一决策点。目前只有这一条渲染
/// 路径;把它从内联拼接抽成具名函数,是为将来"某些扩展名不走 flyfish、
/// 走 Acceptance 式 iced 原生 pane"的分叉预留一个函数级插入点——不引入
/// trait/注册表,YAGNI。
fn flyfish_url(path: &std::path::Path) -> String {
    format!(
        "dozer://flyfish/host.html?p={}",
        encode_component(&path.to_string_lossy())
    )
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib preview::`
Expected: 全部 PASS,包括新的 `bump_reload_appends_query_param_and_only_affects_target_tab` 和原有的 `desired_webviews_builds_urls_and_visibility`（后者验证 `flyfish_url` 抽取没有改变原有 URL 拼接行为）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): PreviewTab 加 reload_nonce,保存后驱动预览重载

抽出 flyfish_url() 决策点(渲染路径可插拔的第一步);desired_webviews()
在 nonce>0 时给 URL 追加 &_r= 参数,逼 sync_webview_pool 走 load_url
分支——否则编辑保存后同路径 URL 不变,flyfish 的 WKWebView 会一直显示
保存前的旧内容。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `preview.rs` — 编辑按钮的扩展名白名单

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`（紧邻 `flyfish_url`，Task 1 新增的函数之后）
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: 无（纯路径判断，不依赖 Task 1 的改动）。
- Produces: `pub fn is_editable_extension(path: &std::path::Path) -> bool`。Task 4 会在 `preview_pane()` 的 tab chip 渲染里调用它决定是否画"编辑"按钮。

- [ ] **Step 1: 写失败测试**

追加到 `crates/dozer-app/src/preview.rs` 的 `mod tests`：

```rust
    #[test]
    fn is_editable_extension_covers_common_text_types() {
        assert!(is_editable_extension(Path::new("main.rs")));
        assert!(is_editable_extension(Path::new("Cargo.toml")));
        assert!(is_editable_extension(Path::new("README.md")));
        assert!(is_editable_extension(Path::new("notes.TXT")), "大小写不敏感");
        assert!(is_editable_extension(Path::new("package.json")));
        assert!(is_editable_extension(Path::new("ci.yaml")));
        assert!(is_editable_extension(Path::new("ci.yml")));
        assert!(is_editable_extension(Path::new("run.sh")));
        assert!(is_editable_extension(Path::new("app.py")));
        assert!(is_editable_extension(Path::new("index.js")));
        assert!(is_editable_extension(Path::new("index.ts")));
        assert!(is_editable_extension(Path::new("index.tsx")));
        assert!(is_editable_extension(Path::new("index.jsx")));
        assert!(is_editable_extension(Path::new("page.html")));
        assert!(is_editable_extension(Path::new("style.css")));
        assert!(is_editable_extension(Path::new("data.xml")));
        assert!(is_editable_extension(Path::new("out.log")));
        assert!(is_editable_extension(Path::new("nginx.conf")));
        assert!(is_editable_extension(Path::new(".gitignore")), "点开头的无扩展名文件要特判");
    }

    #[test]
    fn is_editable_extension_rejects_unknown_and_binary_like() {
        assert!(!is_editable_extension(Path::new("logo.png")));
        assert!(!is_editable_extension(Path::new("archive.zip")));
        assert!(!is_editable_extension(Path::new("LICENSE")), "无扩展名不在白名单里");
        assert!(!is_editable_extension(Path::new("Makefile")));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib preview::tests::is_editable_extension`
Expected: FAIL，`cannot find function 'is_editable_extension'`。

- [ ] **Step 3: 实现**

紧邻 Task 1 新增的 `flyfish_url` 函数之后，在 `crates/dozer-app/src/preview.rs` 新增：

```rust
/// "编辑"按钮的显示范围:纯扩展名白名单,不做内容嗅探(YAGNI,见设计文档
/// "范围外")。`.gitignore` 这类点开头、`Path::extension()` 认不出扩展名
/// 的文件单独特判文件名。
pub fn is_editable_extension(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "rs" | "toml"
            | "md"
            | "txt"
            | "json"
            | "yaml"
            | "yml"
            | "sh"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "html"
            | "css"
            | "xml"
            | "log"
            | "conf"
    )
}
```

`mod tests` 顶部已有 `use super::*; use std::path::PathBuf;`——需要确认 `Path`（非 `PathBuf`）也在作用域内，若没有则在测试模块的 `use` 里补上 `use std::path::Path;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib preview::`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 预览域加编辑按钮的扩展名白名单判断

is_editable_extension() 决定哪些文件类型的预览 tab 会出现"编辑"按钮,
纯扩展名匹配,不做内容嗅探。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `workspace.rs` — `EditSession` 数据模型 + `Workspace` 编辑方法 + `Message` 接线

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:41`（导入）、`:1160`（紧邻 `AcceptanceView` 新增 `EditSession`）、`:1347-1407`（`Workspace` struct 加字段）、`:1635-1667`（`empty_for_project_placeholder` 初始化）、`:1018`（`Message` 枚举新增变体）、`:3536-3543`（`update()` 新增 match arms）
- Test: `crates/dozer-app/src/workspace.rs:7672` 起的 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::preview::is_editable_extension`（Task 2）、`ws.preview.bump_reload`（Task 1）、`ws.preview.tabs()`/`TabKind::File`（既有 `preview.rs` API）。
- Produces: `Workspace.edit_session: Option<EditSession>`；`Workspace::preview_edit_open(&mut self, idx: usize)`、`preview_edit_action(&mut self, action: iced_widget::text_editor::Action)`、`preview_edit_save(&mut self)`、`preview_edit_close_request(&mut self)`、`preview_edit_confirm_discard(&mut self)`、`preview_edit_confirm_cancel(&mut self)`；`Message::PreviewEditOpen(usize)`、`PreviewEditAction(text_editor::Action)`、`PreviewEditSave`、`PreviewEditCloseRequest`、`PreviewEditConfirmDiscard`、`PreviewEditConfirmCancel`。Task 4/5 的视图代码会读 `ws.edit_session`、发这 6 个 `Message`。

### Step 1: 写失败测试（覆盖打开/脏标记/保存/关闭确认的完整状态机）

追加到 `crates/dozer-app/src/workspace.rs:7672` 起的 `mod tests`（`use super::*;` 已在模块顶部，需要额外 `use iced_widget::text_editor;`）：

```rust
    fn write_temp_file(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn preview_edit_open_reads_file_and_starts_clean_session() {
        let (_dir, path) = write_temp_file("a.rs", "fn main() {}");
        let mut ws = Workspace::empty_for_project_placeholder();
        let tab_id = ws.preview.open_path(path.clone());
        ws.preview_edit_open(0);
        let session = ws.edit_session.as_ref().expect("应打开编辑会话");
        assert_eq!(session.tab_id, tab_id);
        assert_eq!(session.path, path);
        assert_eq!(session.content.text(), "fn main() {}");
        assert!(!session.dirty);
        assert!(session.error.is_none());
        assert!(!session.confirm_discard);
    }

    #[test]
    fn preview_edit_open_missing_file_reports_error_and_does_not_open() {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(PathBuf::from("/nonexistent/does-not-exist.rs"));
        ws.preview_edit_open(0);
        assert!(ws.edit_session.is_none());
        assert!(ws.preview_error.is_some());
    }

    #[test]
    fn preview_edit_open_out_of_range_index_is_noop() {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview_edit_open(0);
        assert!(ws.edit_session.is_none());
        assert!(ws.preview_error.is_none());
    }

    #[test]
    fn preview_edit_action_marks_dirty_only_on_edit_actions() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        ws.preview_edit_open(0);
        // 非编辑动作(光标移动)不置脏。
        ws.preview_edit_action(text_editor::Action::Move(text_editor::Motion::Right));
        assert!(!ws.edit_session.as_ref().unwrap().dirty);
        // 编辑动作置脏。
        ws.preview_edit_action(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        assert!(ws.edit_session.as_ref().unwrap().dirty);
        assert_eq!(ws.edit_session.as_ref().unwrap().content.text(), "!hi");
    }

    #[test]
    fn preview_edit_save_writes_disk_clears_dirty_and_bumps_reload() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        let tab_id = ws.preview.open_path(path.clone());
        ws.preview_edit_open(0);
        ws.preview_edit_action(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        ws.preview_edit_save();
        assert!(!ws.edit_session.as_ref().unwrap().dirty);
        assert!(ws.edit_session.as_ref().unwrap().error.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!hi");
        let specs = ws.preview.desired_webviews();
        let spec = specs.iter().find(|s| s.id == tab_id).unwrap();
        assert!(spec.url.contains("&_r=1"), "保存后应推进 reload nonce: {}", spec.url);
    }

    #[test]
    fn preview_edit_close_request_without_dirty_closes_immediately() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        ws.preview_edit_open(0);
        ws.preview_edit_close_request();
        assert!(ws.edit_session.is_none());
    }

    #[test]
    fn preview_edit_close_request_with_dirty_asks_confirm_then_discard_or_cancel() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        ws.preview_edit_open(0);
        ws.preview_edit_action(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        ws.preview_edit_close_request();
        assert!(ws.edit_session.as_ref().unwrap().confirm_discard, "脏改动关闭要先确认");
        assert!(ws.edit_session.is_some(), "确认前不能真的关掉");

        ws.preview_edit_confirm_cancel();
        assert!(!ws.edit_session.as_ref().unwrap().confirm_discard, "取消要回到编辑态");
        assert!(ws.edit_session.as_ref().unwrap().dirty, "取消不丢改动");

        ws.preview_edit_close_request();
        ws.preview_edit_confirm_discard();
        assert!(ws.edit_session.is_none(), "确认放弃要真正关闭");
    }
```

### Step 2: 跑测试确认失败

Run: `cargo test -p dozer-app --lib workspace::tests::preview_edit`
Expected: FAIL（编译错误：`EditSession`/`preview_edit_open` 等尚未定义）。

### Step 3: 实现

**3a. 导入**（`crates/dozer-app/src/workspace.rs:41`，在已有 `use crate::preview::{...}` 里加一项）：

```rust
use crate::preview::{AddrTarget, PreviewPane, TabKind, WebviewSpec, is_editable_extension};
```

**3b. `EditSession` 结构体**（紧邻 `crates/dozer-app/src/workspace.rs:1160` 的 `pub struct AcceptanceView { ... }` 之前插入）：

```rust
/// 预览编辑弹层的进行中会话(全局至多一个;弹层是应用级模态)。
pub struct EditSession {
    /// `PreviewTab.id`(webview 池用的稳定 id,不是 `tabs` vec 下标——见
    /// `Workspace::preview_edit_open` 的取值处)。
    pub tab_id: usize,
    pub path: PathBuf,
    pub content: iced_widget::text_editor::Content,
    pub dirty: bool,
    /// 打开失败(理论上不会,打开前已判过存在)或保存失败的错误文案。
    pub error: Option<String>,
    /// 脏改动下点关闭:先弹二次确认,不直接丢。
    pub confirm_discard: bool,
}
```

**3c. `Workspace` struct 加字段**（`crates/dozer-app/src/workspace.rs:1407` 的 `agent_picker_open: bool,` 之后插入）：

```rust
    /// 预览编辑弹层进行中的会话;`None` = 未打开。
    edit_session: Option<EditSession>,
```

（注意：其余字段都是私有——`preview`/`acceptance` 等也是私有字段，靠 `impl Workspace` 内部方法或同 crate 内 `pub(crate)`/字段本身在同 module 可见性访问。`workspace.rs` 里的 view 函数都在同一文件内，能直接读私有字段，和 `ws.tree_delete_confirm`/`ws.acceptance` 现有用法一致，不需要 `pub`。)

**3d. `empty_for_project_placeholder()` 初始化**（`crates/dozer-app/src/workspace.rs:1666` 的 `loading: false,` 之前插入）：

```rust
            edit_session: None,
```

**3e. `Message` 枚举新增变体**（`crates/dozer-app/src/workspace.rs:1018` 的 `PreviewCloseTab(usize),` 之后插入）：

```rust
    /// 预览:点 tab chip 上的"编辑"按钮,携带 tab 下标(渲染时发出,和
    /// `PreviewSelectTab`/`PreviewCloseTab` 同一约定)。
    PreviewEditOpen(usize),
    /// 预览编辑弹层:`text_editor` widget 的编辑动作回调。
    PreviewEditAction(iced_widget::text_editor::Action),
    /// 预览编辑弹层:"保存"按钮 / ⌘S。
    PreviewEditSave,
    /// 预览编辑弹层:×按钮 / 点遮罩——脏改动会先转成二次确认,不直接关。
    PreviewEditCloseRequest,
    /// 预览编辑弹层二次确认:"放弃改动"。
    PreviewEditConfirmDiscard,
    /// 预览编辑弹层二次确认:"取消"(回到编辑态)。
    PreviewEditConfirmCancel,
```

**3f. `Workspace` 编辑方法**（紧邻 `crates/dozer-app/src/workspace.rs` 的 `fn tab_by_id_mut` 之后，即原 1708-1710 行之后，插入到 `impl Workspace` 块内）：

```rust
    /// 打开预览编辑弹层:按 tab 下标取路径读盘。下标越界或该 tab 不是
    /// `TabKind::File` 时静默 no-op(按钮本就只在 file tab 上画,正常路径
    /// 走不到这两种情况)。读盘失败写 `preview_error`,不开弹层。
    fn preview_edit_open(&mut self, idx: usize) {
        let Some(tab) = self.preview.tabs().get(idx) else {
            return;
        };
        let TabKind::File(path) = &tab.kind else {
            return;
        };
        let path = path.clone();
        let tab_id = tab.id;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.preview_error = None;
                self.edit_session = Some(EditSession {
                    tab_id,
                    path,
                    content: iced_widget::text_editor::Content::with_text(&text),
                    dirty: false,
                    error: None,
                    confirm_discard: false,
                });
            }
            Err(e) => {
                self.preview_error = Some(format!("打开编辑失败: {e}"));
            }
        }
    }

    /// 转发 `text_editor` 的编辑动作;只有真正的编辑(增删字符,非光标
    /// 移动/选区/滚动)才置脏。没有打开编辑会话时 no-op。
    fn preview_edit_action(&mut self, action: iced_widget::text_editor::Action) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        let is_edit = action.is_edit();
        session.content.perform(action);
        if is_edit {
            session.dirty = true;
        }
    }

    /// 保存当前编辑会话到磁盘,成功则清脏并推进该 tab 的 reload nonce
    /// (逼预览 webview 重新加载,否则用户会看到保存前的旧内容)。失败写
    /// `session.error`,弹层不关。没有打开编辑会话时 no-op。
    fn preview_edit_save(&mut self) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        match std::fs::write(&session.path, session.content.text()) {
            Ok(()) => {
                session.dirty = false;
                session.error = None;
                let tab_id = session.tab_id;
                self.preview.bump_reload(tab_id);
            }
            Err(e) => {
                session.error = Some(format!("保存失败: {e}"));
            }
        }
    }

    /// 请求关闭编辑弹层:有未保存改动则转成二次确认,否则直接关。没有
    /// 打开编辑会话时 no-op。
    fn preview_edit_close_request(&mut self) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        if session.dirty {
            session.confirm_discard = true;
        } else {
            self.edit_session = None;
        }
    }

    /// 二次确认:确认放弃未保存改动,真正关闭。
    fn preview_edit_confirm_discard(&mut self) {
        self.edit_session = None;
    }

    /// 二次确认:取消,回到编辑态(改动不丢)。
    fn preview_edit_confirm_cancel(&mut self) {
        if let Some(session) = self.edit_session.as_mut() {
            session.confirm_discard = false;
        }
    }
```

**3g. `App::update()` 接线**（`crates/dozer-app/src/workspace.rs:3536-3543` 的 `Message::PreviewCloseTab(idx) => { ... }` 之后插入）：

```rust
            Message::PreviewEditOpen(idx) => {
                self.with_focused_project(move |ws, _io| ws.preview_edit_open(idx));
            }
            Message::PreviewEditAction(action) => {
                self.with_focused_project(move |ws, _io| ws.preview_edit_action(action));
            }
            Message::PreviewEditSave => {
                self.with_focused_project(|ws, _io| ws.preview_edit_save());
            }
            Message::PreviewEditCloseRequest => {
                self.with_focused_project(|ws, _io| ws.preview_edit_close_request());
            }
            Message::PreviewEditConfirmDiscard => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_discard());
            }
            Message::PreviewEditConfirmCancel => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_cancel());
            }
```

**3h.** `is_editable_extension` 此时还没有调用点（Task 4 才用），Task 3a 的导入会触发 `unused import` 警告。这是预期的中间态，无需处理：`crates/dozer-app/src/main.rs` 没有 `#![deny(warnings)]` 之类的 crate 级 lint，`cargo build`/`cargo test` 不会因未用到的导入而失败；Task 3 的验证步骤也不跑 `clippy -D warnings`。等 Task 4 接上调用点，这条警告自然消失。

### Step 4: 跑测试确认通过

Run: `cargo test -p dozer-app --lib workspace::tests::preview_edit`
Expected: 全部 PASS（7 个新测试）。

Run: `cargo build -p dozer-app`
Expected: 编译通过（若 3h 提到的 warning-as-error 触发，按 3h 说明调整导入位置）。

### Step 5: Commit

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 预览编辑弹层的数据模型 + 状态机

新增 EditSession + Workspace.edit_session,6 个 preview_edit_* 方法
(打开读盘/编辑动作转发/保存清脏+推进 reload nonce/关闭前脏改动二次
确认),对应 6 个 Message::PreviewEdit* 变体在 update() 里做一行转发。
纯 Workspace 方法,不依赖 tokio/wry,可脱离运行时单测。

尚无 UI 入口(下个任务接线 tab 按钮 + 弹层渲染)。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `workspace.rs` — 隐藏 webview + tab chip 编辑按钮

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:2936-2944`（`App::preview_desired`）、`crates/dozer-app/src/workspace.rs:6536-6583`（`preview_pane` 的 tab chip 循环）

**Interfaces:**
- Consumes: `is_editable_extension`（Task 2）、`ws.edit_session`/`Message::PreviewEditOpen`（Task 3）、`ws.preview.desired_webviews()`（既有）。
- Produces: 无新公共接口——这是把 Task 2/3 的产物接进渲染路径的纯粘合任务，用 `cargo build`/手动验证把关（iced 的 `view()`/`App::preview_desired` 没有先例单测，见下方说明）。

**这个任务没有独立单测**：`preview_desired` 是 `App` 的方法，`App` 需要 tokio `Handle`/`EventLoopProxy`/daemon `Client` 才能构造，`crates/dozer-app/src/workspace.rs` 现有的唯一 `#[cfg(test)] mod tests`（7672 行起）也只测 `Workspace`/纯函数，从没测过 `App` 级方法或 tab 渲染——这次不新开先例。用编译通过 + Task 6 的手动验证兜底,与该文件里其余 view/`App::update` 代码的验证方式一致。

- [ ] **Step 1: `preview_desired` 编辑态强制隐藏 webview**

把 `crates/dozer-app/src/workspace.rs:2936-2944` 的：

```rust
    pub fn preview_desired(&self) -> Vec<WebviewSpec> {
        if self.left_view != LeftView::Files {
            return Vec::new();
        }
        match self.active_workspace() {
            Some(ws) => ws.preview.desired_webviews(),
            None => Vec::new(),
        }
    }
```

改成：

```rust
    pub fn preview_desired(&self) -> Vec<WebviewSpec> {
        if self.left_view != LeftView::Files {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let specs = ws.preview.desired_webviews();
        // 编辑弹层开着时,应用级模态盖住了预览区,原生 wry 子视图不听 iced
        // 绘制顺序摆布,必须显式 visible=false 才能真正藏起来——与
        // `TabKind::Acceptance` 隐藏其余 webview 的机制完全一致
        // (`PreviewPane::desired_webviews` 内部的 `acceptance_active`)。
        if ws.edit_session.is_some() {
            specs
                .into_iter()
                .map(|mut s| {
                    s.visible = false;
                    s
                })
                .collect()
        } else {
            specs
        }
    }
```

`WebviewSpec` 的字段需要是可写的——检查 `crates/dozer-app/src/preview.rs` 里 `WebviewSpec` 的 `visible` 字段是不是 `pub`（已确认是 `pub visible: bool`，见 `preview.rs:37`，这里 `s.visible = false` 可以直接赋值）。

- [ ] **Step 2: tab chip 加编辑按钮**

把 `crates/dozer-app/src/workspace.rs:6536-6583` 的 tab chip 构造（`preview_pane()` 内的 `.map(|(idx, tab)| { ... })` 闭包体）：

```rust
        .map(|(idx, tab)| {
            let active = idx == ws.preview.active_idx();
            let select = button(lh(text(tab.title.clone())
                .size(workspace_font::subtitle())
                .color(theme::CREAM)))
            .on_press(Message::PreviewSelectTab(idx))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            });
            let close = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
                .on_press(Message::PreviewCloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                });
            container(
                row![select, close]
                    .spacing(2)
                    .align_y(iced_widget::core::Alignment::Center),
            )
```

改成（在 `select`/`close` 之间插入可选的 `edit` 按钮）：

```rust
        .map(|(idx, tab)| {
            let active = idx == ws.preview.active_idx();
            let select = button(lh(text(tab.title.clone())
                .size(workspace_font::subtitle())
                .color(theme::CREAM)))
            .on_press(Message::PreviewSelectTab(idx))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            });
            let editable = matches!(&tab.kind, TabKind::File(path) if is_editable_extension(path));
            let edit: Option<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
                if editable {
                    Some(
                        button(icons::view(
                            icons::IconKind::Rename,
                            crate::icon_size::row(),
                            theme::DIM,
                        ))
                        .on_press(Message::PreviewEditOpen(idx))
                        .padding(0)
                        .style(|_t, _s| button::Style {
                            background: None,
                            text_color: theme::DIM,
                            ..button::Style::default()
                        })
                        .into(),
                    )
                } else {
                    None
                };
            let close = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
                .on_press(Message::PreviewCloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                });
            let mut chip_row = row![select].spacing(2);
            if let Some(edit) = edit {
                chip_row = chip_row.push(edit);
            }
            chip_row = chip_row.push(close);
            container(chip_row.align_y(iced_widget::core::Alignment::Center))
```

（`row!` 宏返回的 `Row` 后续仍要 `.padding([2, 4]).style(...)` 等——那部分在原代码里紧跟在 `container(...)` 之后，不用动，`container(chip_row.align_y(...))` 替换的只是原来 `container(row![select, close].spacing(2).align_y(...))` 这一段表达式，外层 `.padding([2, 4])` 等链式调用保持不变。）

（`is_editable_extension` 的导入已在 Task 3a 加过，这里不用重复加。）

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过，无 warning（未使用导入等）。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Expected: 通过。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 预览 tab 按扩展名白名单加编辑按钮,编辑态隐藏 webview

is_editable_extension 为真的 file tab 在 chip 里追加编辑图标按钮
(复用 Rename 的铅笔图标),点击发 PreviewEditOpen。preview_desired()
在 edit_session 打开时把整池 webview 强制 visible=false,复用
Acceptance tab 已验证过的隐藏机制。

弹层本身的渲染下个任务补(目前点编辑按钮只是置了状态,还画不出来)。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `workspace.rs` — 编辑弹层渲染

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:4056`（`App::view()` 的 `popped` 分支链）、`crates/dozer-app/src/workspace.rs:58-60`（`iced_widget` 导入加 `text_editor`）
- Create（同文件内新增函数，不新建文件）: `edit_modal()`、`edit_discard_confirm_popup()`，放在 `crates/dozer-app/src/workspace.rs:7167` 的 `delete_confirm_popup` 函数之后

**Interfaces:**
- Consumes: `ws.edit_session`（Task 3）、`Message::PreviewEdit*`（Task 3）、`crate::fonts::code_font()`（既有）。
- Produces: 无新公共接口——纯视图收尾。同 Task 4，没有先例支持给 iced `view()` 函数写单测，用编译通过 + Task 6 手动验证把关。

- [ ] **Step 1: 导入 `text_editor`**

把 `crates/dozer-app/src/workspace.rs:58-60` 的：

```rust
use iced_widget::{
    MouseArea, Scrollable, button, column, container, responsive, row, scrollable, stack, text,
};
```

改成：

```rust
use iced_widget::{
    MouseArea, Scrollable, button, column, container, responsive, row, scrollable, stack, text,
    text_editor,
};
```

- [ ] **Step 2: `App::view()` 顶层 stack 分支链新增编辑弹层分支**

把 `crates/dozer-app/src/workspace.rs:4056` 起的：

```rust
        let popped = if ws.tree_delete_confirm.is_some() {
```

改成（在它前面插入一条新分支，原 `if` 变成 `else if`）：

```rust
        let popped = if ws.edit_session.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewEditCloseRequest);
            let confirm_discard = ws
                .edit_session
                .as_ref()
                .is_some_and(|s| s.confirm_discard);
            if confirm_discard {
                stack![base, dismiss, edit_modal(ws), edit_discard_confirm_popup()]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                stack![base, dismiss, edit_modal(ws)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        } else if ws.tree_delete_confirm.is_some() {
```

- [ ] **Step 3: `edit_modal` + `edit_discard_confirm_popup` 函数**

紧邻 `crates/dozer-app/src/workspace.rs:7167` 的 `delete_confirm_popup` 结尾（该函数以 `}` 收尾后）新增：

```rust
/// 文本编辑弹层:标题行(文件名+关闭)+ `text_editor` 主体(等宽字体)+
/// 错误位 + 保存/关闭按钮。宽高吃满大部分屏幕("放大窗口"的产品意图,
/// 不是小弹窗),四周留 `40.0` 边距,与 `maximize_overlay` 的
/// `scrim_padding` 同一量级,视觉上是同一族"大号应用内模态"。
fn edit_modal(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(session) = &ws.edit_session else {
        return column![].into();
    };
    let name = session
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.path.display().to_string());

    let title_row = row![
        text(name).size(workspace_font::subtitle()).color(theme::CREAM),
        iced_widget::space::horizontal(),
        button(text("×").size(workspace_font::subtitle()).color(theme::DIM))
            .on_press(Message::PreviewEditCloseRequest)
            .padding(0)
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::DIM,
                ..button::Style::default()
            }),
    ]
    .align_y(iced_widget::core::Alignment::Center);

    let editor: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
        text_editor(&session.content)
            .on_action(Message::PreviewEditAction)
            .font(crate::fonts::code_font())
            .size(workspace_font::body())
            .height(Length::Fill)
            .into();

    let mut body = column![title_row, editor].spacing(8);

    if let Some(err) = &session.error {
        body = body.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::body())
                .color(theme::RED),
        );
    }

    let close_btn = button(text("关闭").size(workspace_font::body()).color(theme::CREAM))
        .on_press(Message::PreviewEditCloseRequest)
        .padding([6, 12])
        .style(|_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        });
    let save_btn = button(text("保存").size(workspace_font::body()).color(theme::CREAM))
        .on_press(Message::PreviewEditSave)
        .padding([6, 12])
        .style(|_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border {
                color: theme::CREAM,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        });
    body = body.push(row![close_btn, save_btn].spacing(8));

    let dialog = container(body.padding(16))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

    container(dialog)
        .padding(40.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::SCRIM.into()),
            ..container::Style::default()
        })
        .into()
}

/// 编辑弹层的二次确认:脏改动状态下点关闭,叠在 `edit_modal` 之上。
/// 视觉风格与 `delete_confirm_popup` 一致。
fn edit_discard_confirm_popup<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>
{
    let dialog = container(
        column![
            text("放弃未保存的改动?")
                .size(workspace_font::subtitle())
                .color(theme::CREAM),
            text("关闭后这次编辑不会被保存。")
                .size(workspace_font::label())
                .color(theme::DIM),
            row![
                button(text("取消").size(workspace_font::body()).color(theme::CREAM))
                    .on_press(Message::PreviewEditConfirmCancel)
                    .padding([6, 12])
                    .style(|_t, _s| button::Style {
                        background: Some(theme::CARD.into()),
                        text_color: theme::CREAM,
                        border: Border {
                            color: theme::BORDER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..button::Style::default()
                    }),
                button(text("放弃改动").size(workspace_font::body()).color(theme::RED))
                    .on_press(Message::PreviewEditConfirmDiscard)
                    .padding([6, 12])
                    .style(|_t, _s| button::Style {
                        background: Some(theme::CARD.into()),
                        text_color: theme::RED,
                        border: Border {
                            color: theme::RED,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..button::Style::default()
                    }),
            ]
            .spacing(8),
        ]
        .spacing(8),
    )
    .padding(16)
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    });

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
```

- [ ] **Step 4: 编译检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Expected: 通过。

Run: `cargo fmt --check -p dozer-app`
Expected: 通过（若不通过，跑 `cargo fmt -p dozer-app` 后重新 diff 确认没有改坏逻辑再提交）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 预览编辑弹层的实际渲染

App::view() 顶层 stack 分支链新增 edit_session 分支,复用
delete_confirm_popup 的 dismiss+popup 套路。edit_modal 是大号应用内
模态(SCRIM 遮罩 + 40px 边距的居中卡片),内嵌 text_editor(JetBrains
Mono),标题行文件名+关闭,底部关闭/保存按钮,错误文案红字。脏改动关闭
时叠 edit_discard_confirm_popup 二次确认,视觉风格同 delete_confirm_popup。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: 手动端到端验证

**Files:** 无代码改动，纯验证。

- [ ] **Step 1: 全量测试 + lint**

```bash
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check -p dozer-app
```

Expected: 全部通过。

- [ ] **Step 2: 启动 GUI**

```bash
cargo run -p dozer-app
```

打开一个项目（含至少一个 `.rs`/`.md` 之类的文本文件、一个 `.png` 之类的图片文件）。

- [ ] **Step 3: 走一遍完整交互路径，逐项确认**

1. 在项目树点开一个 `.rs`/`.md` 文件 → 预览 tab 出现，flyfish 正常渲染内容。
2. 该 tab chip 上出现"编辑"图标按钮（铅笔）；点开图片文件（`.png`）的 tab chip 上**不**出现编辑按钮。
3. 点编辑按钮 → 应用内大号模态弹出，内容与文件磁盘内容一致，flyfish 预览的 webview 应完全不可见（被弹层盖住/隐藏）。
4. 在编辑器里敲几个字符 → 无异常（无崩溃、无 IME 问题、光标/输入正常）。
5. 点弹层×（不保存）→ 应弹出"放弃未保存的改动?"二次确认。
6. 点"取消" → 回到编辑器，刚才的改动还在。
7. 再点×→"放弃改动" → 弹层关闭，磁盘文件内容不变，flyfish 预览重新可见且显示的还是编辑前的旧内容（因为没保存）。
8. 重新点编辑按钮 → 改动没了（上次放弃了，重新读盘是原内容），确认改动，点"保存" → 无错误提示。
9. 点"关闭"（此时应已不脏，不弹二次确认）→ 弹层关闭，flyfish 预览重新可见，且内容已经是保存后的新内容（验证 §3 的 reload nonce 生效，不是 stale 缓存）。
10. 重启 GUI（`cargo run -p dozer-app` 重跑）→ 之前保存的改动仍在磁盘上（普通文件系统持久化，非本功能范围但顺手确认没有意外回滚）。

- [ ] **Step 4: 记录结果**

若全部符合预期：完成，无需提交（本任务不改代码）。
若有偏差：回到对应 Task 定位问题，修复后重新走 Step 1-3。
