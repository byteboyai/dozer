# Dozer H0：项目中心落地页 + 顶栏 Dozer 页签化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把顶栏独立的"Dozer"按钮改造成与项目页签同视觉语言的常驻页签，并把点它进入的首页从占位实现换成规格 §3(⓪c) 描述的"项目中心"（左栏最近项目列表 + 右侧"最近的文件"/"最近的对话"两卡）。

**Architecture:** 全部改动落在 `crates/dozer-app/src/workspace.rs`（+ `workspace_geometry.rs` 一个新几何 token + `workspace.json` 一行 JSON）。新增 `App.recent_projects`/`home_recent_files`/`home_recent_conversations`/`home_recents_loaded` 四个字段与 `Message::HomeRecentsLoaded` 一个消息变体；点顶栏 Dozer 页签时同步切页 + 异步 `spawn_blocking` 一次纯 IO（git 改动文件 mtime + 各项目对话目录）刷新两张卡。不改 `Workspace.recent_projects`（项目栏"未打开项目"兜底列表用，语义与生命周期都不同，保持独立）。

**Tech Stack:** Rust、iced 0.14（`iced_widget`）、tokio（`spawn_blocking` 隔离阻塞 git/fs 调用）、既有 `delivery`/`conversation` 模块的纯 IO 函数。

## Global Constraints

- 一期范围严格按规格 §0.1：左栏项目列表 + 右侧"最近的文件""最近的对话"两卡；**不做**日历、社区教程墙（Figma 画了但规格原文"随后补"）。
- "最近的文件"数据源 = 复用 `delivery::file_statuses` 取 git 改动/未跟踪文件 + 文件系统 mtime；**不**新增"最近打开/编辑文件"的全局持久化追踪（规格 §0.2）。
- D1：顶栏 Dozer 页签是独立渲染函数 `dozer_home_tab`，视觉语言与 `project_tab_item` 一致（激活态 CARD 底 + BORDER 描边），无关闭按钮、`Length::Shrink` 固定内容宽、高度吃满 `top_bar_height()`，恒在最左、不参与 `project_tabs_row` 的拥挤收窄。
- D2：新增 `App.recent_projects: Vec<ProjectInfo>`，与 `Workspace.recent_projects` 并存、互不影响；`App::bootstrap()` 与 `Message::ProjectTabOpened` 处理函数负责同步。
- D3：H0 侧栏项目卡点击复用现成的 `Message::ProjectSelect(id)`，不新增消息。
- D4：进入 Home 时异步刷新一次，新增 `Message::HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>)`；纯 IO 内核 `load_home_recents(projects: &[ProjectInfo]) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>)` 签名固定、可单测。对话卡不做"进行中/已验收 vN"状态字，只显示"标题 · 项目名 · agent · 相对时间"。
- D5：相对时间文案抽出纯函数 `relative_time_text(modified_ms: u64, now_ms: u64) -> String`，`conversation_sub` 改调它；H0 三处（项目卡活跃时间、文件卡、对话卡）都复用，不写三份重复 switch。
- D6：新几何 token 一律走 `workspace.json` 的 `geometry` 节点 + `workspace_geometry.rs` 的"JSON 基准 × `icon_size::scale()`"惯例，不在 `home_page` 里散落字面量。本次新增 `h0_sidebar_width = 248`。
- D7：搜索框、"更多项目"按钮是视觉占位，不接线、不新增 `Message` 变体或 `on_press`；"＋新增项目"复用现有 `Message::ProjectTabPickFolder`。
- 主题色一律用 `crate::theme` 现有常量（`GOLD`/`CREAM`/`BODY`/`DIM`/`CARD`/`BORDER`/`RED`/`GREEN`），不写新字面量颜色（项目根 `CLAUDE.md`："主题 ByteBoy2077"裁决）。
- GUI 只用 iced 0.14 生态，不引入新依赖、不新增 crate（项目根 `CLAUDE.md` 裁决）。
- **已知与人工验收草案的差异（需向用户报备，不在本计划范围内静默实现）**：规格 §5 人工验收第 2 步提到项目卡应显示"git 分支"，但 §2 D2 明确"H0 侧栏项目卡片直接读 `app.recent_projects`"（即 `ProjectInfo{ id, path, name, last_active_ms }`，没有 branch 字段），§2 D4 也没有为 branch 设计加载路径。本计划按 D2/D4 已确认的数据源实现（卡片显示名称/相对时间/路径，不显示 git 分支）；如需要 git 分支，需要额外一轮确认（往 `load_home_recents` 里加一次 `delivery::branch(repo)` 调用是最小的后续增量，但那是本计划之外的范围扩张，不在这里顺手加）。

---

### Task 1: 新增 `h0_sidebar_width` 几何 token

**Files:**
- Modify: `crates/dozer-app/assets/theme/workspace.json`
- Modify: `crates/dozer-app/src/workspace_geometry.rs`

**Interfaces:**
- Produces: `pub fn h0_sidebar_width() -> f32`（供 Task 7 的 `home_sidebar` 用）。

- [ ] **Step 1: 在 `workspace.json` 的 `geometry` 节点末尾加一行**

在 `crates/dozer-app/assets/theme/workspace.json` 里找到：

```json
    "menu_pad_v": 6.0,
    "menu_pad_h": 10.0
  }
}
```

改成：

```json
    "menu_pad_v": 6.0,
    "menu_pad_h": 10.0,
    "h0_sidebar_width": 248.0
  }
}
```

- [ ] **Step 2: `Geometry` 结构体加字段**

在 `crates/dozer-app/src/workspace_geometry.rs` 的 `struct Geometry` 里，`menu_pad_h: f32,` 那一行后面加：

```rust
    /// H0 项目中心左栏固定宽（设计基准 248，Figma 同值）。
    h0_sidebar_width: f32,
```

- [ ] **Step 3: 加访问函数**

在文件末尾 `pub fn menu_pad_h()` 函数后面（`#[cfg(test)] mod tests` 之前）加：

```rust
/// H0 项目中心左栏固定宽（逻辑像素），已含全局 scale。
pub fn h0_sidebar_width() -> f32 {
    GEOMETRY.h0_sidebar_width * icon_size::scale()
}
```

- [ ] **Step 4: 补防漂移锚测试**

在 `mod tests` 的 `values_match_pre_migration_literals` 测试末尾（`assert_eq!(menu_pad_h(), 10.0);` 之后）加一行：

```rust
        assert_eq!(h0_sidebar_width(), 248.0);
```

- [ ] **Step 5: 编译并跑测试**

Run: `cargo test -p dozer-app workspace_geometry:: -- --nocapture`
Expected: 新增的 `values_match_pre_migration_literals` 断言通过（该测试函数已存在，只是多加了一行断言）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/assets/theme/workspace.json crates/dozer-app/src/workspace_geometry.rs
git commit -m "feat(dozer-app): 新增 h0_sidebar_width 几何 token"
```

---

### Task 2: 从 `conversation_sub` 抽出 `relative_time_text` 纯函数（D5）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`conversation_sub` 定义处，约第 6366 行；测试模块）

**Interfaces:**
- Produces: `fn relative_time_text(modified_ms: u64, now_ms: u64) -> String`（Task 6/7 的 H0 卡片复用）。
- Consumes: 无（纯函数）。

- [ ] **Step 1: 写失败测试**

在 `workspace.rs` 的 `#[cfg(test)] mod tests` 块里，`conversation_sub_line_format` 测试函数附近加：

```rust
    #[test]
    fn relative_time_text_boundaries() {
        assert_eq!(relative_time_text(1000, 1000), "刚刚");
        assert_eq!(relative_time_text(0, 59_000), "刚刚");
        assert_eq!(relative_time_text(0, 60_000), "1 分钟前");
        assert_eq!(relative_time_text(0, 3_599_000), "59 分钟前");
        assert_eq!(relative_time_text(0, 3_600_000), "1 小时前");
        assert_eq!(relative_time_text(0, 86_399_000), "23 小时前");
        assert_eq!(relative_time_text(0, 86_400_000), "1 天前");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app relative_time_text_boundaries -- --nocapture`
Expected: FAIL（`relative_time_text` 未定义，编译错误）。

- [ ] **Step 3: 抽出函数，`conversation_sub` 改调它**

把 `crates/dozer-app/src/workspace.rs` 里的：

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

替换成：

```rust
/// 相对时间文案：刚刚/N 分钟前/N 小时前/N 天前（D5，从 `conversation_sub`
/// 抽出为独立纯函数）。H0 项目卡"活跃时间"、文件卡、对话卡三处复用，
/// 不要三份重复 switch。
fn relative_time_text(modified_ms: u64, now_ms: u64) -> String {
    let ago = now_ms.saturating_sub(modified_ms) / 1000; // 秒
    if ago < 60 {
        "刚刚".to_string()
    } else if ago < 3600 {
        format!("{} 分钟前", ago / 60)
    } else if ago < 86400 {
        format!("{} 小时前", ago / 3600)
    } else {
        format!("{} 天前", ago / 86400)
    }
}

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
fn conversation_sub(agent: &str, modified_ms: u64, size_bytes: u64, now_ms: u64) -> String {
    let when = relative_time_text(modified_ms, now_ms);
    let size = if size_bytes >= 1024 * 1024 {
        format!("{:.1}MB", size_bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{}KB", (size_bytes / 1024).max(1))
    };
    format!("{agent} · {when} · {size}")
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app relative_time_text_boundaries conversation_sub_line_format -- --nocapture`
Expected: 两个测试都 PASS（`conversation_sub_line_format` 是已有测试，验证抽取没有改变外部行为）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): 从 conversation_sub 抽出 relative_time_text 纯函数"
```

---

### Task 3: `App.recent_projects` 字段 + 与 daemon 的 `list_projects()` 同步（D2）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`struct App`、`new_shell`、`App::bootstrap`、`Message::ProjectTabOpened` 处理函数）

**Interfaces:**
- Produces: `App.recent_projects: Vec<ProjectInfo>` 字段（Task 6/7 的 spawn 任务与 `home_sidebar` 读取）。
- Consumes: 无新接口，复用现有 `Client::list_projects()`。

- [ ] **Step 1: `struct App` 加字段**

在 `crates/dozer-app/src/workspace.rs` 的 `struct App` 里，找到：

```rust
    /// 当前顶层页面(工作区 / 首页)。默认 `Workspace`;点顶栏 Dozer 切到
    /// `Home`,打开/切换项目切回 `Workspace`。
    current_page: AppPage,
}
```

改成：

```rust
    /// 当前顶层页面(工作区 / 首页)。默认 `Workspace`;点顶栏 Dozer 切到
    /// `Home`,打开/切换项目切回 `Workspace`。
    current_page: AppPage,
    /// H0 项目中心侧栏用的"最近项目"列表(D2)。与 `Workspace.recent_projects`
    /// 语义相同但字段独立——避免为了 H0 牵连项目栏"未打开项目"兜底列表那条
    /// 无关路径。`App::bootstrap()`/`Message::ProjectTabOpened` 处理函数负责
    /// 让它跟 daemon 的 `list_projects()` 结果保持同步。
    recent_projects: Vec<ProjectInfo>,
}
```

- [ ] **Step 2: `new_shell` 初始化为空**

在 `new_shell` 函数里，找到：

```rust
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
            current_page: AppPage::Workspace,
        }
    }
```

改成：

```rust
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
        }
    }
```

- [ ] **Step 3: `App::bootstrap()` 同步一行**

在 `App::bootstrap()` 里找到：

```rust
        let known = io.client.list_projects().await.unwrap_or_default();
        let sessions = io.client.list().await.unwrap_or_default();
```

改成：

```rust
        let known = io.client.list_projects().await.unwrap_or_default();
        app.recent_projects = known.clone();
        let sessions = io.client.list().await.unwrap_or_default();
```

- [ ] **Step 4: `Message::ProjectTabOpened` 处理函数顶部同步**

在 `update()` 里找到：

```rust
            Message::ProjectTabOpened(project, recent) => {
                // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
```

改成：

```rust
            Message::ProjectTabOpened(project, recent) => {
                self.recent_projects = recent.clone();
                // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
```

（后续分支里原有的 `ws.recent_projects = recent;` / `ws.recent_projects = recent;` 不用动——那两处写的是 `Workspace.recent_projects`，`recent: Vec<ProjectInfo>` 派生 `Clone`，这里 `.clone()` 一次不影响后面继续 move。）

- [ ] **Step 5: 编译检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过（这一步只加字段和同步赋值，还没有任何读取方，`#![allow(dead_code)]` 未声明的话编译器可能警告字段未读——若报 `field is never read` warning，属预期，Task 7 会消费它，不必现在解决）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): 新增 App.recent_projects 并与 daemon list_projects 同步"
```

---

### Task 4: `HomeRecentFile`/`HomeRecentConversation` 类型 + `load_home_recents` 纯 IO 函数（D4 前半）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（新增私有类型、新增函数、新增测试）

**Interfaces:**
- Consumes: `delivery::repo_root(&Path) -> Option<PathBuf>`、`delivery::file_statuses(&Path) -> HashMap<PathBuf, FileStatus>`、`conversation::list_all_conversations(&Path) -> Vec<ConversationMeta>`、`relative_time_text`（Task 2 产出）。
- Produces: `struct HomeRecentFile { path: PathBuf, project_name: String, modified_ms: u64 }`、`struct HomeRecentConversation { project_name: String, meta: ConversationMeta }`、`fn load_home_recents(projects: &[ProjectInfo]) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>)`（Task 5 的消息处理与 Task 7 的渲染都要用这三样，字段/签名不能改名）。

- [ ] **Step 1: 写失败测试**

在 `workspace.rs` 的 `#[cfg(test)] mod tests` 块里加：

```rust
    #[test]
    fn load_home_recents_empty_input_returns_empty_vecs() {
        let (files, convs) = load_home_recents(&[]);
        assert!(files.is_empty());
        assert!(convs.is_empty());
    }

    #[test]
    fn load_home_recents_merges_and_sorts_across_projects() {
        let proj_a = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj_a.path())
            .status()
            .unwrap();
        std::fs::write(proj_a.path().join("a.txt"), "changed").unwrap();

        let proj_b = tempfile::tempdir().unwrap(); // 非 git 目录,没有改动可报告

        let projects = vec![
            ProjectInfo {
                id: 1,
                path: proj_a.path().to_string_lossy().into_owned(),
                name: "proj-a".into(),
                last_active_ms: 0,
            },
            ProjectInfo {
                id: 2,
                path: proj_b.path().to_string_lossy().into_owned(),
                name: "proj-b".into(),
                last_active_ms: 0,
            },
        ];

        let (files, convs) = load_home_recents(&projects);
        assert_eq!(files.len(), 1, "只有项目 A(git repo)贡献一条改动文件");
        assert_eq!(files[0].project_name, "proj-a");
        assert!(files[0].path.ends_with("a.txt"));
        // 这里不额外造一个带假 Claude 对话目录的项目去断言"合并进 convs"：
        // `conversation::list_all_conversations` 内部读真实 `HOME` 环境变量
        // (`conversation.rs::home_dir`)，没有注入点；`conversation.rs` 自己的
        // 测试也因为同样原因(cargo test 多线程、mutate HOME 不安全)绕开了
        // 真实入口，转而在 `project_dir_in` 这一层验证目录拼接+合并排序(见
        // `list_all_conversations_merges_three_dirs_sorted_by_mtime`)。三个
        // agent 目录的合并/排序逻辑已经在那条测试里覆盖，这里只需确认
        // "没有可达对话目录时 convs 为空、不 panic"这一层 `load_home_recents`
        // 自己的收尾逻辑。
        assert!(convs.is_empty(), "两个项目都没有可达的 agent 对话目录");
    }

    #[test]
    fn load_home_recents_truncates_files_to_top_4() {
        let proj = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj.path())
            .status()
            .unwrap();
        for i in 0..6 {
            std::fs::write(proj.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let projects = vec![ProjectInfo {
            id: 1,
            path: proj.path().to_string_lossy().into_owned(),
            name: "proj".into(),
            last_active_ms: 0,
        }];
        let (files, _convs) = load_home_recents(&projects);
        assert_eq!(files.len(), 4, "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app load_home_recents -- --nocapture`
Expected: FAIL（`load_home_recents`/`HomeRecentFile`/`HomeRecentConversation` 未定义，编译错误）。

- [ ] **Step 3: 加类型与实现**

在 `crates/dozer-app/src/workspace.rs` 里，紧挨着 `conversation_sub` 函数（Task 2 已改过的那个）后面加：

```rust
/// H0"最近的文件"卡一行(跨项目合并前的中间表示；D4，本地私有类型)。
#[derive(Debug, Clone, PartialEq)]
struct HomeRecentFile {
    path: PathBuf,
    project_name: String,
    modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上。
#[derive(Debug, Clone, PartialEq)]
struct HomeRecentConversation {
    project_name: String,
    meta: ConversationMeta,
}

/// D4 纯 IO 内核：对给定项目列表分别取"最近改动的文件"(git 改动/未跟踪 +
/// fs mtime)与"最近的对话"(三个 agent 来源已聚合、按 mtime 倒序)，跨项目
/// 合并后各自按时间倒序，取前 4 条 / 前 3 条(对齐 Figma 卡片行数)。
///
/// 必须在 `spawn_blocking` 里跑，不能在 UI 线程直呼——内部既有阻塞 git
/// 子进程调用，也有阻塞文件系统调用。签名固定(`&[ProjectInfo]` 输入，两个
/// `Vec` 输出)方便 headless 单测：不需要 daemon 连接或 winit `EventLoopProxy`。
fn load_home_recents(
    projects: &[ProjectInfo],
) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>) {
    let mut files: Vec<HomeRecentFile> = Vec::new();
    let mut convs: Vec<HomeRecentConversation> = Vec::new();
    for p in projects {
        let cwd = PathBuf::from(&p.path);
        if let Some(repo) = delivery::repo_root(&cwd) {
            for (path, _status) in delivery::file_statuses(&repo) {
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue; // 路径已在磁盘消失(用户手动删了),静默跳过(spec §4)
                };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                files.push(HomeRecentFile {
                    path,
                    project_name: p.name.clone(),
                    modified_ms,
                });
            }
        }
        for meta in conversation::list_all_conversations(&cwd) {
            convs.push(HomeRecentConversation {
                project_name: p.name.clone(),
                meta,
            });
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    files.truncate(4);
    convs.sort_by_key(|c| std::cmp::Reverse(c.meta.modified_ms));
    convs.truncate(3);
    (files, convs)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app load_home_recents -- --nocapture`
Expected: 三个测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): 新增 load_home_recents 纯 IO 内核(H0 最近文件/对话)"
```

---

### Task 5: 接线 `Message::HomeRecentsLoaded` + `TopBarHome` 异步刷新（D4 后半）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`enum Message`、`struct App`、`new_shell`、`update()` 里的 `TopBarHome` 分支）

**Interfaces:**
- Consumes: Task 3 的 `App.recent_projects`、Task 4 的 `load_home_recents`/`HomeRecentFile`/`HomeRecentConversation`。
- Produces: `App.home_recent_files: Vec<HomeRecentFile>`、`App.home_recent_conversations: Vec<HomeRecentConversation>`、`App.home_recents_loaded: bool`（Task 7 的 `home_page` 读取）；`Message::HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>)`。

- [ ] **Step 1: `enum Message` 加变体**

在 `crates/dozer-app/src/workspace.rs` 的 `enum Message` 里找到：

```rust
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// 项目页签:点已存在的页签 → 前台化该项目。只改"当前是哪个页签",
    /// 不结束任何会话、不改写任何 `Workspace` 的内容。
    ProjectTabSwitch(i64),
```

改成：

```rust
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// H0 项目中心:`Message::TopBarHome` 发起的异步刷新完成(最近改动的文件、
    /// 最近的对话两份列表;D4)。
    HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>),
    /// 项目页签:点已存在的页签 → 前台化该项目。只改"当前是哪个页签",
    /// 不结束任何会话、不改写任何 `Workspace` 的内容。
    ProjectTabSwitch(i64),
```

- [ ] **Step 2: `struct App` 加字段**

在 `struct App` 里找到 Task 3 刚加的：

```rust
    recent_projects: Vec<ProjectInfo>,
}
```

改成：

```rust
    recent_projects: Vec<ProjectInfo>,
    /// H0"最近的文件"卡数据(D4);`Message::TopBarHome` 时异步刷新。
    home_recent_files: Vec<HomeRecentFile>,
    /// H0"最近的对话"卡数据(D4);语义同上。
    home_recent_conversations: Vec<HomeRecentConversation>,
    /// 是否已经收到过至少一次 `HomeRecentsLoaded`——区分"还在加载"与"加载完
    /// 但结果为空"，两张卡据此决定画"加载中…"还是空状态文案(spec §4)。
    home_recents_loaded: bool,
}
```

- [ ] **Step 3: `new_shell` 初始化**

在 `new_shell` 里找到 Task 3 刚加的：

```rust
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
        }
    }
```

改成：

```rust
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
        }
    }
```

- [ ] **Step 4: `TopBarHome` 分支发起异步刷新 + 新增 `HomeRecentsLoaded` 分支**

在 `update()` 里找到：

```rust
            Message::TopBarHome => {
                self.current_page = AppPage::Home;
            }
```

改成：

```rust
            Message::TopBarHome => {
                self.current_page = AppPage::Home;
                self.home_recents_loaded = false;
                let projects: Vec<ProjectInfo> =
                    self.recent_projects.iter().take(5).cloned().collect();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let (files, convs) =
                        tokio::task::spawn_blocking(move || load_home_recents(&projects))
                            .await
                            .unwrap_or_default();
                    let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
                });
            }
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
```

- [ ] **Step 5: 编译检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过（`home_recent_files`/`home_recent_conversations` 尚无读取方，可能有 `field is never read` warning，Task 7 会消费，不必现在解决）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): 接线 HomeRecentsLoaded,TopBarHome 异步刷新最近文件/对话"
```

---

### Task 6: `dozer_home_tab` 渲染函数 + 接入 `top_bar()`（D1）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（新增函数、`top_bar()`）

**Interfaces:**
- Consumes: 现有 `icons::view`、`theme::*`、`workspace_font::*`、`workspace_geometry::top_bar_height()`、`top_bar_font()`、`AppPage`。
- Produces: `fn dozer_home_tab(active: bool) -> Element<'_, Message, ...>`。

- [ ] **Step 1: 加渲染函数**

在 `crates/dozer-app/src/workspace.rs` 里，`fn top_bar_font()` 函数后面（`fn top_bar(app: &App)` 之前）加：

```rust
/// 顶栏"Dozer"页签(D1)：视觉语言与 `project_tab_item` 一致(激活态 CARD 底
/// + BORDER 描边)，但没有关闭按钮、恒在最左、不参与 `project_tabs_row` 的
/// 拥挤收窄——与当前项目页签行"＋"按钮同款的"固定位不参与收窄"处理。
fn dozer_home_tab<'a>(
    active: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let label = row![
        icons::view(
            icons::IconKind::Home,
            crate::icon_size::rail(),
            if active { theme::CREAM } else { theme::DIM },
        ),
        text("Dozer")
            .font(top_bar_font())
            .size(workspace_font::body())
            .color(if active { theme::CREAM } else { theme::DIM }),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let select = button(label)
        .on_press(Message::TopBarHome)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        });

    container(select)
        .padding([0, 14])
        .height(Length::Fixed(workspace_geometry::top_bar_height()))
        .width(Length::Shrink)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
}
```

- [ ] **Step 2: `top_bar()` 改调它**

在 `fn top_bar(app: &App) -> Element<...>` 里找到：

```rust
    // Dozer 字标做成按钮:home 图标 + 文字,点它进首页(`AppPage::Home`)。
    let title = button(
        row![
            icons::view(
                icons::IconKind::Home,
                crate::icon_size::rail(),
                theme::CREAM,
            ),
            text("Dozer")
                .font(top_bar_font())
                .size(workspace_font::body())
                .color(theme::CREAM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::TopBarHome)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::CREAM,
        ..button::Style::default()
    });
```

改成：

```rust
    // Dozer 页签:视觉与右侧项目页签一致,恒在最左、不参与拥挤收窄(D1)。
    let title = dozer_home_tab(app.current_page == AppPage::Home);
```

（下面 `let bar = row![title, tabs, right]...` 不用改，`title` 变量名沿用。）

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Dozer 顶栏按钮改造成常驻页签(dozer_home_tab)"
```

---

### Task 7: 重写 `home_page()` 为项目中心两栏布局（D1-D7 组装）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（整体替换 `home_page`，新增 `home_sidebar`/`home_recents_column`/`home_recent_files_card`/`home_recent_conversations_card` 四个私有渲染函数）

**Interfaces:**
- Consumes: `App.recent_projects`/`home_recent_files`/`home_recent_conversations`/`home_recents_loaded`（Task 3/5）、`workspace_geometry::h0_sidebar_width()`（Task 1）、`relative_time_text`（Task 2）、`icons::icon_for_file`、`Message::ProjectSelect`/`ProjectTabPickFolder`（复用现有）。
- Produces: 无新公开接口，纯渲染。

- [ ] **Step 1: 整体替换 `home_page`，并新增四个辅助渲染函数**

把 `crates/dozer-app/src/workspace.rs` 里现有的：

```rust
/// 首页落地页(点顶栏 Dozer 进入):品牌区 + 打开项目入口 + 已开项目列表。
/// 风格沿用 ByteBoy2077 主题。打开/切换项目会自动退回工作区视图。
fn home_page(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![]
        .spacing(16)
        .align_x(iced_widget::core::Alignment::Center);

    // 品牌区:home 图标 + 大号 Dozer 字标 + 标语
    col = col.push(
        row![
            icons::view(icons::IconKind::Home, 48.0, theme::GOLD),
            text("Dozer")
                .font(top_bar_font())
                .size((workspace_font::title() as f32) * 2.0)
                .color(theme::CREAM),
        ]
        .spacing(12)
        .align_y(iced_widget::core::Alignment::Center),
    );
    col = col.push(
        text("甲方侧 AI 治理与验收层")
            .size(workspace_font::subtitle())
            .color(theme::DIM),
    );

    // 打开项目入口(复用顶栏"＋"的文件夹选择落地路径)
    col = col.push(
        button(
            text("打开项目…")
                .size(workspace_font::body())
                .color(theme::GOLD),
        )
        .on_press(Message::ProjectTabPickFolder)
        .padding([8, 16])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            text_color: theme::GOLD,
            ..button::Style::default()
        }),
    );

    // 已开项目列表:点卡片即前台化该项目并退回工作区
    let entries = project_tab_entries(app);
    if !entries.is_empty() {
        let mut list = column![]
            .spacing(8)
            .align_x(iced_widget::core::Alignment::Center);
        for entry in entries {
            let mut label = row![]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center);
            if let Some((color, blinking)) = entry.dot {
                let color = if blinking && !app.blink_on {
                    Color { a: 0.15, ..color }
                } else {
                    color
                };
                label = label.push(text("●").size(workspace_font::caption_sm()).color(color));
            }
            label = label.push(
                text(entry.name)
                    .size(workspace_font::body())
                    .color(theme::CREAM),
            );
            let card = button(label)
                .on_press(Message::ProjectTabSwitch(entry.id))
                .padding([8, 16])
                .style(|_t: &iced_widget::Theme, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::CREAM,
                    ..button::Style::default()
                });
            list = list.push(card);
        }
        col = col.push(
            text("已打开的项目")
                .size(workspace_font::caption())
                .color(theme::DIM),
        );
        col = col.push(list);
    }

    if let Some(err) = &app.daemon_error {
        col = col.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::body())
                .color(theme::RED),
        );
    }

    container(col.padding(48))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::Alignment::Center)
        .align_y(iced_widget::core::Alignment::Center)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(chrome_style::background().into()),
            ..container::Style::default()
        })
        .into()
}
```

整段替换成：

```rust
/// 首页落地页(点顶栏 Dozer 进入)：规格 §3(⓪c) H0 帧——左栏"我的项目"列表
/// + 右侧"最近的文件"/"最近的对话"两卡(D1-D7)。风格沿用 ByteBoy2077 主题。
/// 点某张最近项目卡会自动退回工作区视图(`Message::ProjectSelect`)。
fn home_page(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let body = row![home_sidebar(app, now_ms), home_recents_column(app, now_ms)]
        .spacing(24)
        .height(Length::Fill);

    let mut col = column![body].spacing(16).height(Length::Fill);
    if let Some(err) = &app.daemon_error {
        col = col.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::body())
                .color(theme::RED),
        );
    }

    container(col.padding(24))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(chrome_style::background().into()),
            ..container::Style::default()
        })
        .into()
}

/// H0 左栏(固定宽 `h0_sidebar_width`)：品牌区 + "我的项目" + 搜索占位(D7)
/// + 最近项目卡(取 `app.recent_projects` 前 5 条,D2/D3) + "更多项目"占位
/// (D7) + "＋新增项目"(复用 `Message::ProjectTabPickFolder`)。
/// `app.recent_projects` 为空时画"还没有项目"兜底文案,不崩(spec §4)。
fn home_sidebar(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![].spacing(16);

    col = col.push(
        row![
            icons::view(icons::IconKind::Home, crate::icon_size::rail(), theme::GOLD),
            text("Dozer")
                .font(top_bar_font())
                .size(workspace_font::subtitle())
                .color(theme::CREAM),
            text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(workspace_font::caption_sm())
                .color(theme::DIM),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    );

    col = col.push(
        text("我的项目")
            .size(workspace_font::caption())
            .color(theme::DIM),
    );

    // 搜索框:视觉占位,不接线(D7；precedent:顶栏 ⌘K 搜索框同款"先视觉后接线")。
    col = col.push(
        container(
            row![
                icons::view(icons::IconKind::Search, crate::icon_size::row(), theme::DIM),
                text("搜索项目…")
                    .size(workspace_font::body())
                    .color(theme::DIM),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        }),
    );

    if app.recent_projects.is_empty() {
        col = col.push(
            text("还没有项目")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        let mut list = column![].spacing(8);
        for p in app.recent_projects.iter().take(5) {
            let card = button(
                column![
                    lh(text(p.name.clone())
                        .size(workspace_font::body())
                        .color(theme::CREAM)),
                    lh(text(relative_time_text(p.last_active_ms, now_ms))
                        .size(workspace_font::caption_sm())
                        .color(theme::DIM)),
                    lh(text(p.path.clone())
                        .size(workspace_font::caption_sm())
                        .color(theme::DIM)),
                ]
                .spacing(2),
            )
            .on_press(Message::ProjectSelect(p.id))
            .width(Length::Fill)
            .padding(10)
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: Some(theme::CARD.into()),
                text_color: theme::CREAM,
                border: Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..button::Style::default()
            });
            list = list.push(card);
        }
        col = col.push(
            Scrollable::new(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new())),
        );
    }

    // "更多项目":视觉占位,不接线——对应的"全部项目列表"视图现在不存在,
    // 属于后续增量(D7)。
    col = col.push(
        container(
            text("更多项目")
                .size(workspace_font::caption())
                .color(theme::DIM),
        )
        .padding([6, 0]),
    );

    col = col.push(
        button(
            text("＋新增项目")
                .size(workspace_font::body())
                .color(theme::GOLD),
        )
        .on_press(Message::ProjectTabPickFolder)
        .padding([8, 16])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            text_color: theme::GOLD,
            ..button::Style::default()
        }),
    );

    container(col)
        .width(Length::Fixed(workspace_geometry::h0_sidebar_width()))
        .height(Length::Fill)
        .into()
}

/// H0 右侧："最近的文件"/"最近的对话"两卡并排(D4)。
fn home_recents_column(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    row![
        home_recent_files_card(app, now_ms),
        home_recent_conversations_card(app, now_ms)
    ]
    .spacing(24)
    .height(Length::Fill)
    .into()
}

/// "最近的文件"卡：`app.home_recents_loaded` 为 false 时(刚点进 Home 还没等
/// 到异步结果)画"加载中…"，避免第一帧空白跳变(spec §4)。
fn home_recent_files_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text("最近的文件")
            .size(workspace_font::subtitle())
            .color(theme::CREAM)
    ]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else if app.home_recent_files.is_empty() {
        col = col.push(
            text("暂无最近改动的文件")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        for f in &app.home_recent_files {
            let filename = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.display().to_string());
            let row_el = row![
                icons::view(
                    icons::icon_for_file(&filename),
                    crate::icon_size::row(),
                    theme::DIM
                ),
                column![
                    lh(text(filename.clone())
                        .size(workspace_font::body())
                        .color(theme::CREAM)),
                    lh(text(format!(
                        "{} · {}",
                        f.project_name,
                        relative_time_text(f.modified_ms, now_ms)
                    ))
                    .size(workspace_font::caption_sm())
                    .color(theme::DIM)),
                ]
                .spacing(2),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center);
            col = col.push(
                container(row_el)
                    .padding(10)
                    .width(Length::Fill)
                    .style(|_t: &iced_widget::Theme| container::Style {
                        background: Some(theme::CARD.into()),
                        border: Border {
                            color: theme::BORDER,
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        ..container::Style::default()
                    }),
            );
        }
    }

    container(col)
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .into()
}

/// "最近的对话"卡：语义同 `home_recent_files_card`。裁剪掉"进行中/已验收
/// vN"状态字(D4)，只显示"标题 · 项目名 · agent · 相对时间"。
fn home_recent_conversations_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text("最近的对话")
            .size(workspace_font::subtitle())
            .color(theme::CREAM)
    ]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else if app.home_recent_conversations.is_empty() {
        col = col.push(
            text("暂无对话记录")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        for c in &app.home_recent_conversations {
            let sub = format!(
                "{} · {} · {}",
                c.project_name,
                c.meta.agent.label(),
                relative_time_text(c.meta.modified_ms, now_ms)
            );
            col = col.push(
                container(
                    column![
                        lh(text(c.meta.title.clone())
                            .size(workspace_font::body())
                            .color(theme::CREAM)),
                        lh(text(sub).size(workspace_font::caption_sm()).color(theme::DIM)),
                    ]
                    .spacing(4),
                )
                .padding(10)
                .width(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                }),
            );
        }
    }

    container(col)
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .into()
}
```

**注意**：`project_tab_entries` 函数本身**不要删**——`project_tabs_row`（顶栏页签行渲染）还在用它，只是 `home_page` 不再调用它。

- [ ] **Step 2: 编译检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过，无 `unused` 相关的新警告（`home_recent_files`/`home_recent_conversations`/`recent_projects` 现在都被 `home_page` 读取了）。

- [ ] **Step 3: `cargo clippy` + `cargo fmt` 检查**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 都通过（`cargo fmt` 若报格式差异，跑 `cargo fmt -p dozer-app` 直接改好再复检）。

- [ ] **Step 4: 跑全量测试**

Run: `cargo test -p dozer-app`
Expected: 全部 PASS，包括 Task 1/2/4 新增的测试与既有测试（`conversation_sub_line_format` 等）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): H0 项目中心落地页(左栏最近项目 + 最近文件/对话两卡)"
```

---

### Task 8: 人工验收（规格 §5）+ 全量校验

**Files:** 无代码改动；本任务只跑既有命令 + 手动操作已构建的 app。

- [ ] **Step 1: 全量构建 + lint + 测试**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt --check && cargo test -p dozer-app`
Expected: 全部通过。

- [ ] **Step 2: 跑起 GUI**

Run: `cargo run -p dozer-app`
Expected: 正常启动到工作区视图（默认页）。

- [ ] **Step 3: 按规格 §5"人工验收(草案)"逐条走一遍**

1. 顶栏最左出现"Dozer"页签，样式与右侧项目页签一致（激活态描边+底色），当前在工作区视图时它是未激活态（`theme::DIM` 文字色，透明背景）。
2. 点它 → 切到项目中心页：左栏 248px 显示项目 logo/版本、"我的项目"、搜索框、最多 5 张最近项目卡（名称/相对时间/路径——**git 分支本轮不显示，见 Global Constraints 里记的差异**）、"更多项目"+"＋新增项目"两个按钮；右侧先短暂"加载中…"，随后出现"最近的文件"（本仓最近 git 改动的文件，按时间倒序）与"最近的对话"（本仓 Claude 对话历史，按时间倒序）两张卡。
3. 点某张最近项目卡：若该项目已开着页签 → 直接切过去（不影响任何已有终端会话）；若没开 → 新开一个页签并前台化。
4. 点"＋新增项目" → 走现有文件夹选择流程，与顶栏"＋"/项目栏按钮行为一致。
5. 再点一次顶栏 Dozer 页签（此时已经在 Home）→ 数据重新拉一遍（两张卡会短暂回到"加载中…"再刷新，体感内容可能因磁盘变化而更新），不报错、不重复叠加。
6. 关掉所有项目页签，回到"一个项目都没打开"的状态 → Home 页仍然正常渲染（左栏"还没有项目"兜底文案，右侧两卡各自"暂无…"或空），不 panic。

- [ ] **Step 4: 记录验收结果**

若第 3 步（人工验收）发现任何一条不符，回到对应 Task 修正、重新跑 Step 1-3，不要带着已知不符收尾。全部符合后再进行下一步。

- [ ] **Step 5: 最终提交前状态检查**

Run: `git status --short`
Expected: 只有本计划涉及的文件被改动（`workspace.rs`/`workspace_geometry.rs`/`workspace.json`）；若看到无关文件被 stage，先确认是不是本地既有的未提交改动（例如本仓当前分支上已有的 `term_model.rs`/`term_view.rs`/`project.rs`/图标资源等无关改动），不要一并提交。
