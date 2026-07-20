# Dozer P1j 项目对话列表 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 右一 AI 栏给出当前项目的对话列表（扫 Claude 目录，扁平，活对话标"● 当前"置顶），点任一条进左二审阅 tab（复用 P1i 引擎）；移除 P1i 冗余的终端"审阅"按钮，入口统一到列表。

**Architecture:** 扫描 `~/.claude/projects/<cwd换->` 圈成"ClaudeCode 对话来源"（纯 IO + 纯解析分离，GUI 侧 spawn_blocking）；`ConversationMeta` 带 `agent` 字段为多 agent 留维度；P1i 的 `ReviewView.source_tab_id` 泛化为 `ReviewSource{Session|File}`（File 源=历史快照不刷新）；右一 AI 栏（现 `pane` 占位）改真 pane，视图切换 `[对话|Agents]`，对话列表卡按 Figma S4 样式（当前行金框高亮）。dozerd 不参与（哑管道）。

**Tech Stack:** Rust、serde_json（transcript 首行解析）、iced 0.14、tokio、tempfile（测试）。

**Spec:** `docs/superpowers/specs/2026-07-20-dozer-p1j-conversation-list-design.md`
**Figma 参考:** node 71:2（S4 会话列表）/ 71:183（S5 审阅详情）——AI 栏视图切换 + 对话卡（当前金框、绿点、标题+`claude·时间·规模`）。

## Global Constraints

- dozerd 只在会话活着时传 transcript_path（P1i 已有），不扫不解析（哑管道）；扫描/解析全 GUI 侧 spawn_blocking。
- 主题 ByteBoy2077：AI 栏底 `theme::PANEL`、对话卡 `theme::CARD`/圆角 10、标题 `theme::CREAM`、副行 `theme::DIM`、当前行金框 `theme::GOLD` + 绿点 `theme::GREEN`；视图切换激活 pill 底 `theme::CARD`/文字 `theme::CREAM`、非激活 `theme::DIM`。
- `agent` 字段现恒 `"claude"`（多 agent 留门）；扫描逻辑圈在 conversation.rs，核心列表只吃 `ConversationMeta`。
- 收敛 P1i：移除终端 tab 的"审阅"按钮，入口=右一对话列表。
- 测试全 headless；AI 栏 UI 归末任务人工验收。
- commit 中文、结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；每 Task 收尾 `cargo clippy --all-targets && cargo fmt` 零警告。

---

### Task 1: P1i 收敛——ReviewSource 泛化 + 移除终端审阅按钮

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces:
  - `ReviewSource { Session(usize), File(PathBuf) }`（Clone/PartialEq）
  - `ReviewView.source: ReviewSource`（替换 `source_tab_id: usize`）
  - `spawn_review_load(&self, source: ReviewSource, path: String)`
  - `Message::ReviewLoaded(ReviewSource, Result<Vec<ReviewEntry>, String>)`（首参 usize→ReviewSource）
  - `review_should_refresh_on_turn(source: &ReviewSource, tab_id: usize) -> bool`（纯函数）

- [x] **Step 1: 写失败测试**（`workspace.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn review_refresh_only_for_matching_session() {
        use std::path::PathBuf;
        assert!(review_should_refresh_on_turn(&ReviewSource::Session(3), 3));
        assert!(!review_should_refresh_on_turn(&ReviewSource::Session(3), 4));
        assert!(!review_should_refresh_on_turn(
            &ReviewSource::File(PathBuf::from("/t/x.jsonl")),
            3
        ));
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app review_refresh_only`
Expected: 编译错误（`ReviewSource`/`review_should_refresh_on_turn` 未定义）。

- [x] **Step 3: 实现**

`workspace.rs`：

1. `ReviewView` 上方加枚举 + 纯函数：

```rust
/// 审阅内容的来源（P1j）：活会话 tab（回合结束刷新）或历史对话文件（快照不刷新）。
#[derive(Clone, PartialEq)]
pub enum ReviewSource {
    Session(usize),
    File(PathBuf),
}

/// 回合结束时该审阅视图是否应重解析：仅当它是该会话的活审阅。
fn review_should_refresh_on_turn(source: &ReviewSource, tab_id: usize) -> bool {
    matches!(source, ReviewSource::Session(id) if *id == tab_id)
}
```

2. `ReviewView` 的 `pub source_tab_id: usize,` 改为 `pub source: ReviewSource,`。

3. `Message::ReviewLoaded(usize, Result<Vec<ReviewEntry>, String>)` 改为 `ReviewLoaded(ReviewSource, Result<Vec<ReviewEntry>, String>)`。

4. `Message::ReviewOpen(tab_id)` handler：`source_tab_id: tab_id` 改 `source: ReviewSource::Session(tab_id)`；`spawn_review_load(tab_id, path)` 改 `spawn_review_load(ReviewSource::Session(tab_id), path)`。

5. `spawn_review_load` 签名与体改为按源：

```rust
    fn spawn_review_load(&self, source: ReviewSource, path: String) {
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                std::fs::read_to_string(&path)
                    .map(|s| transcript::parse_transcript(&s))
                    .map_err(|e| format!("无法读取会话记录: {e}"))
            })
            .await
            .unwrap_or_else(|e| Err(format!("解析任务失败: {e}")));
            let _ = proxy.send_event(Message::ReviewLoaded(source, result));
        });
    }
```

6. `Message::ReviewLoaded` handler：按源匹配当前审阅（替换 `rv.source_tab_id == tab_id`）：

```rust
            Message::ReviewLoaded(source, result) => {
                if let Some(rv) = &mut self.review
                    && rv.source == source
                {
                    match result {
                        Ok(entries) => {
                            rv.entries = entries;
                            rv.error = None;
                        }
                        Err(e) => rv.error = Some(e),
                    }
                }
            }
```

7. `AgentStateChanged` 的 TurnEnded 重解析块：把 `rv.source_tab_id == tab_id` 改为 `review_should_refresh_on_turn(&rv.source, tab_id)`；`spawn_review_load` 调用改 `spawn_review_load(ReviewSource::Session(tab_id), path)`（该块内已知 tab_id、path）。

8. **移除终端审阅按钮**（D7）：删除 `terminal_pane` 里"会话审阅入口（P1i）"整段（`if let Some(tab) = ws.tabs.get(ws.active) && review_available(...) { ... ReviewOpen(tab.tab_id) ... }`）。`review_available` 函数保留（Task 4 AI 栏"● 当前"判定复用；若 clippy 报 dead_code，本任务末尾对其加 `#[allow(dead_code)]`，Task 4 移除）。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app review_refresh_only && cargo test -p dozer-app`
Expected: 全绿。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(审阅): ReviewView.source_tab_id 泛化为 ReviewSource{Session|File} + 移除终端审阅按钮(P1j 收敛)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: conversation.rs——ClaudeCode 对话来源（扫描 + 解析）

**Files:**
- Create: `crates/dozer-app/src/conversation.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod transcript;` 后加 `mod conversation;`）

**Interfaces:**
- Produces:
  - `ConversationMeta { path: PathBuf, title: String, modified_ms: u64, size_bytes: u64, agent: String }`（Debug/Clone/PartialEq）
  - `claude_project_dir(cwd: &Path) -> PathBuf`
  - `conversation_title(jsonl_head: &str) -> Option<String>`（纯）
  - `list_conversations(dir: &Path) -> Vec<ConversationMeta>`（IO，mtime 倒序）
  - `is_current_conversation(meta_path: &Path, open_transcripts: &[String]) -> bool`（纯）

- [x] **Step 1: 写失败测试**（`conversation.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn title_from_first_string_user_turn() {
        let head = concat!(
            "{\"type\":\"mode\",\"mode\":\"x\"}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n"
        );
        assert_eq!(conversation_title(head).as_deref(), Some("改一下 README"));
        assert_eq!(conversation_title("{\"type\":\"mode\"}\nbad\n"), None);
    }

    #[test]
    fn is_current_matches_open_transcript() {
        use std::path::Path;
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation(Path::new("/t/a.jsonl"), &opens));
        assert!(!is_current_conversation(Path::new("/t/c.jsonl"), &opens));
    }

    #[test]
    fn list_sorts_by_mtime_desc_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("one.jsonl");
        let p2 = dir.path().join("two.jsonl");
        std::fs::write(&p1, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第一个\"}}\n").unwrap();
        std::fs::write(&p2, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二个\"}}\n").unwrap();
        // 让 p2 更新（mtime 更大）——filetime 不引依赖,靠写入顺序 + 短睡
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p2, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二个\"}}\n").unwrap();
        let list = list_conversations(dir.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "第二个", "mtime 倒序:后写的在前");
        assert_eq!(list[0].agent, "claude");
        assert!(list[0].size_bytes > 0);
        assert!(list_conversations(std::path::Path::new("/no/such/dir")).is_empty());
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app conversation`
Expected: 编译错误（模块不存在）。

- [x] **Step 3: 实现**

`crates/dozer-app/src/conversation.rs`：

```rust
//! ClaudeCode 对话来源（P1j）：扫描 `~/.claude/projects/<cwd换->` 列出该项目
//! 的历史对话(JSONL),取首句人类发言当标题。纯 IO + 纯解析;dozerd 不参与。
//! `agent` 字段为多 agent 留维度(现恒 "claude");核心列表只吃 ConversationMeta。

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMeta {
    pub path: PathBuf,
    pub title: String,
    pub modified_ms: u64,
    pub size_bytes: u64,
    pub agent: String,
}

/// cwd → Claude 存储目录：`~/.claude/projects/<cwd 中 '/' 换 '-'>`。
pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let key = cwd.to_string_lossy().replace('/', "-");
    PathBuf::from(home)
        .join(".claude")
        .join("projects")
        .join(key)
}

/// transcript 首段 → 首句人类发言(首个字符串型 user content)。纯函数。
pub fn conversation_title(jsonl_head: &str) -> Option<String> {
    for line in jsonl_head.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("user")
            && let Some(text) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
        {
            return Some(text.to_string());
        }
    }
    None
}

/// 该对话是否是当前活会话(其路径在打开着的 transcript 集合中)。纯函数。
pub fn is_current_conversation(meta_path: &Path, open_transcripts: &[String]) -> bool {
    let p = meta_path.to_string_lossy();
    open_transcripts.iter().any(|o| o.as_str() == p)
}

/// 列出目录下所有 .jsonl 为对话(mtime 倒序)。读失败/非目录返回空。
/// 标题只读文件前若干字节以省 IO。
pub fn list_conversations(dir: &Path) -> Vec<ConversationMeta> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<ConversationMeta> = rd
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                return None;
            }
            let meta = e.metadata().ok()?;
            let size_bytes = meta.len();
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            // 只读前 16KB 找标题
            let head = read_head(&path, 16 * 1024);
            let title = conversation_title(&head).unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "(无标题对话)".into())
            });
            Some(ConversationMeta {
                path,
                title,
                modified_ms,
                size_bytes,
                agent: "claude".into(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    out
}

/// 读文件前 n 字节为 String(lossy)。
fn read_head(path: &Path, n: usize) -> String {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..read]).into_owned()
}
```

`main.rs` 的 `mod transcript;` 后加 `mod conversation;`。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app conversation`
Expected: 4 测试通过。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/conversation.rs crates/dozer-app/src/main.rs
git commit -m "feat(对话): conversation.rs ClaudeCode 对话来源——扫目录/首句标题/mtime 倒序/agent 留门

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: workspace 状态——对话列表刷新 + AiView + ConversationOpen

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 1 `ReviewSource`；Task 2 `conversation::{ConversationMeta, claude_project_dir, list_conversations}`。
- Produces:
  - `Workspace` 字段 `conversations: Vec<ConversationMeta>`、`ai_view: AiView`
  - `AiView { Conversations, Agents }`（Copy/PartialEq，默认 Conversations）
  - `Message::{ConversationsRefreshed(Vec<ConversationMeta>), ConversationOpen(PathBuf), AiViewSwitch(AiView)}`
  - `Workspace::spawn_conversations_refresh(&self)`
  - `open_transcript_paths(&self) -> Vec<String>`（供 UI 判"● 当前"）

- [x] **Step 1: 写失败测试**（`workspace.rs` tests 追加——纯逻辑）

```rust
    #[test]
    fn ai_view_default_is_conversations() {
        assert_eq!(AiView::default(), AiView::Conversations);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app ai_view_default`
Expected: 编译错误（`AiView` 未定义）。

- [x] **Step 3: 实现**

`workspace.rs`：

1. `use crate::conversation::{self, ConversationMeta};`

2. 枚举（放 `ReviewSource` 附近）：

```rust
/// 右一 AI 栏当前视图（P1j）。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum AiView {
    #[default]
    Conversations,
    Agents,
}
```

3. `Workspace` 加字段（两处构造初始化 `conversations: Vec::new(), ai_view: AiView::default(),`）：

```rust
    /// 当前项目的对话列表（扫 Claude 目录；P1j）。
    conversations: Vec<ConversationMeta>,
    /// 右一 AI 栏当前视图。
    ai_view: AiView,
```

4. `Message` 追加：

```rust
    /// 对话列表刷新结果（扫描完成）。
    ConversationsRefreshed(Vec<ConversationMeta>),
    /// 点对话列表某条 → 审阅该历史对话。
    ConversationOpen(PathBuf),
    /// 右一 AI 栏视图切换。
    AiViewSwitch(AiView),
```

5. `impl Workspace` 加方法：

```rust
    /// 异步扫当前项目的对话目录 → ConversationsRefreshed（GUI 侧 spawn_blocking）。
    fn spawn_conversations_refresh(&self) {
        let Some(p) = &self.project else {
            return;
        };
        let cwd = PathBuf::from(&p.path);
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let list = tokio::task::spawn_blocking(move || {
                conversation::list_conversations(&conversation::claude_project_dir(&cwd))
            })
            .await
            .unwrap_or_default();
            let _ = proxy.send_event(Message::ConversationsRefreshed(list));
        });
    }

    /// 打开着的会话 transcript 路径集合（UI 判"● 当前"用）。
    pub fn open_transcript_paths(&self) -> Vec<String> {
        self.tabs
            .iter()
            .filter_map(|t| t.transcript_path.clone())
            .collect()
    }
```

6. `update` 追加分支：

```rust
            Message::ConversationsRefreshed(list) => {
                self.conversations = list;
            }
            Message::ConversationOpen(path) => {
                self.review = Some(ReviewView {
                    source: ReviewSource::File(path.clone()),
                    entries: Vec::new(),
                    error: None,
                    expanded: std::collections::HashSet::new(),
                });
                self.preview.open_review();
                self.spawn_review_load(ReviewSource::File(path.clone()), path.to_string_lossy().into_owned());
            }
            Message::AiViewSwitch(v) => {
                self.ai_view = v;
            }
```

7. 刷新接线：`Message::ProjectOpened` handler 尾部（`self.spawn_project_git_refresh();` 之后）加 `self.spawn_conversations_refresh();`；`Message::DeliveryChecked` handler 里（回合结束刷新 git 处，`self.spawn_project_git_refresh();` 之后）加 `self.spawn_conversations_refresh();`。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app ai_view_default && cargo build -p dozer-app`
Expected: 测试过；编译通过（UI 未接，`conversations`/`ai_view`/`open_transcript_paths` 可能 dead_code——Task 4 消费；本任务末尾对未用项加 `#[allow(dead_code)]`，Task 4 移除）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(对话): workspace 对话列表状态——扫描刷新/AiView/ConversationOpen(ReviewSource::File)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: 右一 AI 栏 UI——视图切换 + 对话列表（替换占位）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 2/3 全部；Task 1 `review_available`（"● 当前"复用）；`conversation::is_current_conversation`。
- Produces: `ai_pane(ws) -> Element`（替换 `view()` 的 `col4`）；纯函数 `conversation_sub(agent, modified_ms, size_bytes, now_ms) -> String`（副行文案，便于测）。

- [x] **Step 1: 写失败测试**（`workspace.rs` tests 追加）

```rust
    #[test]
    fn conversation_sub_line_format() {
        // 78KB, 刚刚
        let s = conversation_sub("claude", 1000, 78 * 1024, 1000);
        assert!(s.starts_with("claude · "), "含 agent 前缀: {s}");
        assert!(s.ends_with("· 78KB"), "含规模: {s}");
        // MB 级
        let s2 = conversation_sub("claude", 1000, 8 * 1024 * 1024, 1000);
        assert!(s2.ends_with("· 8.0MB"), "MB 规模: {s2}");
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app conversation_sub_line`
Expected: 编译错误（`conversation_sub` 未定义）。

- [x] **Step 3: 实现**

`workspace.rs`：

1. 纯函数（文件级）：

```rust
/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
fn conversation_sub(agent: &str, modified_ms: u64, size_bytes: u64, now_ms: u64) -> String {
    let ago = now_ms.saturating_sub(modified_ms) / 1000; // 秒
    let when = if ago < 60 {
        "刚刚".to_string()
    } else if ago < 3600 {
        format!("{} 分钟前", ago / 60)
    } else if ago < 86400 {
        format!("{} 小时前", ago / 3600)
    } else {
        format!("{} 天前", ago / 86400)
    };
    let size = if size_bytes >= 1024 * 1024 {
        format!("{:.1}MB", size_bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{}KB", (size_bytes / 1024).max(1))
    };
    format!("{agent} · {when} · {size}")
}
```

2. `ai_pane` 渲染（放 `project_pane` 附近；`view()` 的 `col4` 改 `ai_pane(self)`）：

```rust
/// 右一 AI 栏（P1j）：视图切换 [对话|Agents] + 对话列表（当前行金框高亮）。
fn ai_pane(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut content = column![
        row![
            text("AI").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8)
    ]
    .spacing(8);

    // 视图切换 [对话 | Agents]
    let mk_pill = |label: &'static str, v: AiView| {
        let active = ws.ai_view == v;
        button(
            text(label)
                .size(13)
                .color(if active { theme::CREAM } else { theme::DIM }),
        )
        .on_press(Message::AiViewSwitch(v))
        .style(move |_t, _s| button::Style {
            background: if active { Some(theme::CARD.into()) } else { None },
            text_color: if active { theme::CREAM } else { theme::DIM },
            border: Border { color: theme::BORDER, width: 0.0, radius: 6.0.into() },
            ..button::Style::default()
        })
    };
    content = content.push(
        row![
            mk_pill("对话", AiView::Conversations),
            mk_pill("Agents", AiView::Agents)
        ]
        .spacing(6),
    );

    match ws.ai_view {
        AiView::Conversations => {
            let opens = ws.open_transcript_paths();
            let active_n = ws
                .conversations
                .iter()
                .filter(|c| conversation::is_current_conversation(&c.path, &opens))
                .count();
            content = content.push(
                row![
                    text("对话").size(11).color(theme::DIM),
                    text(format!("{} 条 · {} 活跃", ws.conversations.len(), active_n))
                        .size(11)
                        .color(theme::DIM),
                ]
                .spacing(6),
            );
            if ws.conversations.is_empty() {
                content = content.push(text("暂无对话记录").size(13).color(theme::DIM));
            }
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            // 当前活跃置顶：先活后历史，各自内部保持 mtime 序（列表已 mtime 倒序）
            let mut ordered: Vec<&ConversationMeta> = ws.conversations.iter().collect();
            ordered.sort_by_key(|c| !conversation::is_current_conversation(&c.path, &opens));
            for c in ordered {
                let current = conversation::is_current_conversation(&c.path, &opens);
                let title_color = theme::CREAM;
                let sub = if current {
                    format!("● 当前 · {}", conversation_sub(&c.agent, c.modified_ms, c.size_bytes, now_ms))
                } else {
                    conversation_sub(&c.agent, c.modified_ms, c.size_bytes, now_ms)
                };
                let sub_color = if current { theme::GREEN } else { theme::DIM };
                let card = button(
                    column![
                        text(c.title.clone()).size(13).color(title_color),
                        text(sub).size(10).color(sub_color),
                    ]
                    .spacing(4),
                )
                .on_press(Message::ConversationOpen(c.path.clone()))
                .width(Length::Fill)
                .style(move |_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::CREAM,
                    border: Border {
                        color: if current { theme::GOLD } else { theme::BORDER },
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    ..button::Style::default()
                });
                content = content.push(card);
            }
        }
        AiView::Agents => {
            content = content.push(text("Agents（后续）").size(13).color(theme::DIM));
        }
    }

    container(content.padding(12))
        .width(Length::Fixed(AI_COL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            border: Border { color: theme::BORDER, width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}
```

3. `view()`：`let col4 = pane("AI · P1e", AI_COL_WIDTH, theme::PANEL);` 改为 `let col4 = ai_pane(self);`。

4. 移除 Task 1/3 里临时加的 `#[allow(dead_code)]`（`review_available`/`conversations`/`ai_view`/`open_transcript_paths` 现均被消费）。

- [x] **Step 4: 跑测试确认通过 + 冒烟**

```bash
cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 测试全绿、零警告；app 存活 8 秒无 panic。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(对话): 右一 AI 栏对话列表 UI——视图切换 + 当前行金框高亮 + 点击进审阅(替换占位)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: 全量回归 + P1i+P1j 端到端人工验收 + 落档

**Files:**
- Create: `docs/superpowers/specs/2026-07-20-p1ij-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（§3 需求 6 回填 + §7 AI 栏修订标注）
- Modify: 本计划文件 + `docs/superpowers/plans/2026-07-19-dozer-p1i-conversation-review.md`（勾选）

**Interfaces:**
- Consumes: 全部前序任务 + P1i。
- Produces: 用户签字的验收记录（P1i 引擎 + P1j 入口合一）；下一阶段起点。

- [x] **Step 1: 全量回归 + 冒烟**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 全绿零警告；app 存活 8 秒。**验收前 `pkill dozerd` 重启新 daemon;需 `dozer-hook install` 且新起 claude 会话(transcript_path 靠 hook)。**

- [x] **Step 2: 用户人工验收（逐项 ✓/✗，验收权在用户）**

```markdown
# P1i+P1j 人工验收清单（用户实机执行）
1. 打开本仓 → 右一 AI 栏默认"对话"视图,列出历史对话(标题=首句、时间倒序、`claude·时间·规模`)
2. 在 Dozer 里跑 claude 聊一句 → 该会话对话在列表顶部标"● 当前"(金框高亮)
3. 点一条历史对话 → 左二审阅 tab 结构化呈现那次对话(人类锚点 ▎+ AI 回合折叠,点▸过程展开)
4. 切"Agents"视图 → 显占位;切回"对话" → 列表还在
5. 让当前 claude 再答一轮 → 回合结束后:当前对话刷新、历史对话内容不动(File 源快照)
6. 终端 tab 不再有"审阅"按钮(入口已统一到右一列表)
7. 非本项目 cwd/无记录 → 右一"暂无对话记录",不崩
8. 关 app 重开 → 打开项目后对话列表恢复
```

- [x] **Step 3: 结果落档 + Commit**

验收记录写 `docs/superpowers/specs/2026-07-20-p1ij-acceptance.md`（P1i 引擎 + P1j 入口，合一验收；沿用格式）；勾选 P1i 计划残留 checkbox；规格 §3 需求 6 追加"P1i+P1j 达成（<日期>，对话可审阅性：transcript 适配器 + 审阅 tab（人类锚点/折叠）+ 右一对话列表（扫 Claude 目录、活对话置顶、点击进审阅）；提问置顶/产物联动/多 agent/两层分组随后续），验收记录见 specs/2026-07-20-p1ij-acceptance.md"；§7 左四 AI 栏追加"P1j 修订：AI 栏加对话列表并置于 agent 前（高频关注点优先）"。

```bash
git add docs
git commit -m "验收：P1i+P1j 对话可审阅性人工验收记录 + 规格回填(§3 需求6 + §7 AI栏修订)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **Spec 覆盖**：§0.1 扁平扫描=T2；§0.1 多 agent 留门(agent 字段 + 来源圈层)=T2；§0.2 右一位置/切换=T4 `ai_pane`+`AiView`；§0.3 取代终端入口=T1 移除按钮 + T4 列表入口；§0.4 §7 修订=T5 落档;D1 目录映射=T2;D2 列举/标题=T2;D3 活标记=T3 `open_transcript_paths`+T4 `is_current_conversation`;D4 视图切换器=T4;D5 刷新节奏=T3 接 ProjectOpened/DeliveryChecked;D6 ReviewSource::File=T1+T3;D7 dozerd 不参与=全 GUI spawn_blocking。错误处理五条：无目录空列表(T2 read_dir 失败返空 + T4 "暂无对话记录")、无标题回落文件名(T2)、点开读失败红字(P1i ReviewLoaded Err，T1 保留)、无当前项目(T4 header "未打开项目" + spawn 空)、hook 没装无● 当前(T4 opens 空则无高亮)。
2. **占位符扫描**：无 TBD；T1/T3 的临时 `#[allow(dead_code)]` 有明确移除时点(T4 Step 3.4)。
3. **类型一致性**：`ReviewSource{Session,File}`、`ReviewView.source`、`spawn_review_load(ReviewSource,String)`、`Message::ReviewLoaded(ReviewSource,..)`、`review_should_refresh_on_turn(&ReviewSource,usize)`、`ConversationMeta{path,title,modified_ms,size_bytes,agent}`、`claude_project_dir/conversation_title/list_conversations/is_current_conversation`、`AiView{Conversations,Agents}`、`Message::{ConversationsRefreshed,ConversationOpen,AiViewSwitch}`、`spawn_conversations_refresh`、`open_transcript_paths`、`conversation_sub(&str,u64,u64,u64)`、`ai_pane` 在 T1-T4 交叉一致。
