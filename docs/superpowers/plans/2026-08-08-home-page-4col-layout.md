# 首页落地页四栏布局重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把首页落地页(`AppPage::Home`,点顶栏 Dozer 页签进入)从静态两栏布局
(`home_page` → `row![home_sidebar, home_recents_column]`)改造成与工作区一致的
`left_icon_rail / left_zone / right_zone / right_icon_rail` 四栏结构:左栏默认
"项目列表" pane,可切到合并后的"Recents" pane(原"最近的文件"/"最近的对话"两卡);
右栏固定"浏览器" pane(复用 `extensions::browser`,全局态,不绑定项目)。首页相关
代码同时从 `workspace.rs` 拆到新文件 `homespace.rs`。

**Architecture:** 新文件 `crates/dozer-app/src/homespace.rs`(`mod homespace;` 加入
`main.rs`),承载首页专属类型(`HomeLeftView`/`HomeRightView`/`HomeRecentFile`/
`HomeRecentConversation`)与全部首页视图构建函数(`home_page` 及其子函数)。
`App` struct/`Message` 枚举/`App::update` 的 Home 消息处理逻辑仍留在
`workspace.rs`(单一数据源+集中调度,比照 `preview.rs`/`conversation.rs` 的既有
拆分先例)。右栏浏览器直接复用 `extensions::browser::{State, Message, update,
view}` 的第二份独立实例(`app.home_browser`,`project_id` 恒 `None`)。

**Tech Stack:** Rust workspace;iced 0.14;`extensions::browser`(已有依赖,直接
复用不改造)。

## Global Constraints

- **在独立分支上开发,不直接提交到 main**:建分支 `feature/home-page-4col-layout`
  (若用 `superpowers:using-git-worktrees` 建 worktree,分支名同此)。全部任务在
  这个分支上提交;完成后提请代码审阅,审阅通过后再合并回 main。不要求/不默认
  直接在 main 上开发。
- 设计文档:`docs/superpowers/specs/2026-08-08-home-page-4col-layout-design.md`
  (有疑问以它为准;过程中已订正两处:①不复用 `divider_bar(Divider::LeftRight,
  ..)`,新写不接拖拽的 `home_divider`;②测试策略订正为"`App::update` 级别行为
  不可测——现状没有 `App` 测试夹具"，仅可测纯函数/枚举默认值)。
- 首页 `home_left_view`/`home_right_view` **不持久化**,不进 `ShellLayout`/
  `layout.json`;每次 `Message::TopBarHome` 进首页都重置为默认值。
- 首页四栏**不做拖拽调宽、不做收起/放大**——`home_left_zone` 固定宽
  `theme::geometry::h0_sidebar_width()`,`home_right_zone` 为 `Length::Fill`。
- 项目列表 / Recents 这两个 pane **不做成 `extensions::` 模块**,保持普通 `fn`
  (只读 `App` 字段、不接收自己的 `Message`)。**不**接入工作区自己的
  `LeftView`/`RightView` rail——已与用户确认这次只在首页用。
- **不做 `App`/`Workspace` 拆分**——只拆首页相关代码到 `homespace.rs`,不动
  `workspace.rs` 其余内容的文件归属。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app &&
  cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过(无警告)。
- 不要为任何过渡态添加 `#[allow(dead_code)]`——本计划按"每个任务结束时新增代码
  已被完整使用、无死代码警告"来切分任务边界,如果某一步出现 dead_code 警告,
  说明任务边界切错了,不是加 allow 的信号。

---

### Task 1: 拆出 `homespace.rs`,搬迁纯类型/纯函数(`HomeRecentFile`/`HomeRecentConversation`/`load_home_recents`)

纯文件搬家,不改任何行为、不改任何签名可见性以外的东西——独立可审的第一步。

**Files:**
- Create: `crates/dozer-app/src/homespace.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod homespace;`)
- Modify: `crates/dozer-app/src/workspace.rs`(删除旧定义+旧测试,改用 import)

**Interfaces:**
- Produces(`homespace.rs`,后续任务复用):
  - `pub(crate) struct HomeRecentFile { pub(crate) path: PathBuf, pub(crate) project_name: String, pub(crate) modified_ms: u64 }`
  - `pub(crate) struct HomeRecentConversation { pub(crate) project_name: String, pub(crate) meta: ConversationMeta }`
  - `pub(crate) fn load_home_recents(projects: &[ProjectInfo]) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>)`

- [ ] **Step 1: 创建 `homespace.rs`,写入类型与纯函数**

`crates/dozer-app/src/workspace.rs` 现有以下代码(7028-7088 行,行号可能因你的
副本已有细微改动而漂移,以下面这段内容本身为准去定位):

```rust
/// H0"最近的文件"卡一行(跨项目合并前的中间表示；D4)。`Message::
/// HomeRecentsLoaded` 的载荷用到它，因此至少是 `pub(crate)`(见 `private_interfaces`)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentFile {
    path: PathBuf,
    project_name: String,
    modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentConversation {
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

删除这段(结构体两个 + 函数一个),连同它们各自的紧邻文档注释一起删。

紧接着,`workspace.rs` 的 `#[cfg(test)] mod tests` 块里有 3 个测试(用
`load_home_recents_empty_input_returns_empty_vecs`/
`load_home_recents_merges_and_sorts_across_projects`/
`load_home_recents_truncates_files_to_top_4` 定位):

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
        assert_eq!(
            files.len(),
            4,
            "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)"
        );
    }
```

删除这 3 个测试(从 workspace.rs 的测试模块里)。

创建 `crates/dozer-app/src/homespace.rs`,内容:

```rust
// crates/dozer-app/src/homespace.rs
//! 首页落地页(`AppPage::Home`,点顶栏 Dozer 页签进入)专属代码:类型定义与
//! 全部视图构建函数。`App` struct 字段声明/`Message` 枚举/`App::update` 的
//! 消息处理逻辑仍留在 `workspace.rs`(单一数据源+集中调度),拆分边界比照
//! `preview.rs`/`conversation.rs` 的既有先例——本文件不持有 `App`/`Workspace`
//! 的 `impl` 块。见 `docs/superpowers/specs/2026-08-08-home-page-4col-layout-design.md`。

use crate::conversation::{self, ConversationMeta};
use crate::delivery;
use dozer_core::protocol::ProjectInfo;
use std::path::PathBuf;

/// H0"最近的文件"卡一行(跨项目合并前的中间表示；D4)。`Message::
/// HomeRecentsLoaded` 的载荷用到它，因此至少是 `pub(crate)`(见 `private_interfaces`)。
/// 字段本身也是 `pub(crate)`——`workspace.rs` 里画卡片的视图函数要读它们。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentFile {
    pub(crate) path: PathBuf,
    pub(crate) project_name: String,
    pub(crate) modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentConversation {
    pub(crate) project_name: String,
    pub(crate) meta: ConversationMeta,
}

/// D4 纯 IO 内核：对给定项目列表分别取"最近改动的文件"(git 改动/未跟踪 +
/// fs mtime)与"最近的对话"(三个 agent 来源已聚合、按 mtime 倒序)，跨项目
/// 合并后各自按时间倒序，取前 4 条 / 前 3 条(对齐 Figma 卡片行数)。
///
/// 必须在 `spawn_blocking` 里跑，不能在 UI 线程直呼——内部既有阻塞 git
/// 子进程调用，也有阻塞文件系统调用。签名固定(`&[ProjectInfo]` 输入，两个
/// `Vec` 输出)方便 headless 单测：不需要 daemon 连接或 winit `EventLoopProxy`。
/// `pub(crate)`——`workspace.rs` 的 `Message::TopBarHome` 处理器在
/// `spawn_blocking` 闭包里直接调用它。
pub(crate) fn load_home_recents(
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            files.len(),
            4,
            "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)"
        );
    }
}
```

- [ ] **Step 2: `main.rs` 加 `mod homespace;`**

`crates/dozer-app/src/main.rs` 开头的 `mod` 列表按字母序排列:

```rust
mod assets;
mod clipboard_image;
mod conversation;
mod delivery;
mod extensions;
mod fonts;
mod git_watch;
mod goal;
mod icons;
```

在 `mod goal;` 和 `mod icons;` 之间插入 `mod homespace;`:

```rust
mod goal;
mod homespace;
mod icons;
```

- [ ] **Step 3: `workspace.rs` 改用 import**

在 `workspace.rs` 顶部现有的 `use crate::...` 块里(`use crate::goal;` 之后、
`use crate::icons;` 之前那一带,具体看现有顺序即可,不强求字母序插入位置精确)
加一行:

```rust
use crate::homespace::{self, HomeRecentFile, HomeRecentConversation, load_home_recents};
```

(`self` 现在暂时用不到,但下一个任务马上就要用 `homespace::HomeLeftView` 这种
带前缀的写法——先加上避免下个任务再改一次 `use` 行。)

- [ ] **Step 4: 编译确认**

```bash
cargo build -p dozer-app
```

预期:编译通过。`workspace.rs` 里原本调用 `load_home_recents(...)` 的那处
(`Message::TopBarHome` 处理器)、以及读 `HomeRecentFile`/`HomeRecentConversation`
字段的 `home_recent_files_card`/`home_recent_conversations_card` 两个视图函数,
现在都是跨模块引用——字段已经在 Step 1 标了 `pub(crate)`,应该编译干净。

- [ ] **Step 5: 测试确认**

```bash
cargo test -p dozer-app load_home_recents
```

预期:3 个测试全部 PASS(从新位置 `homespace.rs` 跑)。

```bash
cargo test -p dozer-app
```

预期:全量测试(现在是 82 个,原 85 减去搬走的 3 个 `load_home_recents_*`,
`homespace.rs` 自己的模块另计)全部 PASS,无编译错误。

- [ ] **Step 6: lint/格式确认**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:干净通过,无警告。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/homespace.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): extract home-page pure types into homespace.rs"
```

---

### Task 2: 首页四栏化——新增图标、状态/消息接线、视图重写

这是核心任务:新增两个 rail 图标、`HomeLeftView`/`HomeRightView` 状态、对应
`Message`/`App::update` 接线,并把首页视图从两栏改写成四栏(项目列表/Recents 可切、
浏览器常驻右栏)。这几件事互相依赖(新图标要有地方用、新状态要有视图读)才能落到
"编译通过且零死代码警告"的状态,因此不再拆更细的独立任务——但内部仍按可验证的
步骤推进。

**Files:**
- Create: `crates/dozer-app/assets/icons/layout-list.svg`
- Create: `crates/dozer-app/assets/icons/history.svg`
- Modify: `crates/dozer-app/src/icons.rs`
- Modify: `crates/dozer-app/src/homespace.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes(来自 Task 1):`homespace::HomeRecentFile`/`HomeRecentConversation`/
  `load_home_recents`;`workspace.rs` 现有 `App`/`Message`/`rail_icon_button`/
  `zone_pane_border`/`PaneCorner`/`relative_time_text`/`lh`/`RailButton`/
  `HoverId`/`browser::{State, Message, update, view}`。
- Produces:`homespace::HomeLeftView`/`HomeRightView`(`pub(crate)`,`Message`
  变体要用);`workspace.rs` 新增 `Message::HomeLeftIconSelect`/
  `HomeRightIconSelect`/`HomeBrowser`,`App` 新字段 `home_left_view`/
  `home_right_view`/`home_browser`。

- [ ] **Step 1: 新增两个 Lucide 图标资源**

创建 `crates/dozer-app/assets/icons/layout-list.svg`(项目列表 pane 图标,
Lucide `layout-list`):

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <rect width="7" height="7" x="3" y="3" rx="1" />
  <rect width="7" height="7" x="3" y="14" rx="1" />
  <path d="M14 4h7" />
  <path d="M14 9h7" />
  <path d="M14 15h7" />
  <path d="M14 20h7" />
</svg>
```

创建 `crates/dozer-app/assets/icons/history.svg`(Recents pane 图标,Lucide
`history`):

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
  <path d="M3 3v5h5" />
  <path d="M12 7v5l4 2" />
</svg>
```

(两个文件都去掉了 Lucide 原始 SVG 头部的 license 注释与 `class="lucide
lucide-xxx"` 属性,格式对齐仓库里其它已有图标,如 `crates/dozer-app/assets/
icons/info.svg`。)

浏览器 pane 复用已有的 `icons::IconKind::Globe`(与工作区 `LeftView::Web` 同一
图标资源,语义都是"浏览器",视觉复用没问题),不需要新增第三个图标文件。

- [ ] **Step 2: `icons.rs` 加两个 `IconKind` 变体**

`crates/dozer-app/src/icons.rs` 的 `IconKind` 枚举(10 行起),在 `Terminal`
变体之后、`Dozer` 变体之前加:

```rust
    /// 首页左栏"项目列表" pane rail 图标(Lucide layout-list)。
    LayoutList,
    /// 首页左栏"Recents" pane rail 图标(Lucide history)。
    History,
```

`bytes()` 方法里(72 行起的大 `match`),对应加两条,插在 `Terminal` 分支之后、
`Dozer` 分支之前:

```rust
            IconKind::LayoutList => include_bytes!("../assets/icons/layout-list.svg"),
            IconKind::History => include_bytes!("../assets/icons/history.svg"),
```

- [ ] **Step 3: `homespace.rs` 加 `HomeLeftView`/`HomeRightView`**

在 `homespace.rs` 顶部 `use` 块之后、`HomeRecentFile` 定义之前插入:

```rust
/// 首页左栏当前显示哪个 pane。语义、命名对齐工作区 `LeftView`,但这是独立
/// 枚举——首页导航态不与工作区共用,也不持久化(每次 `Message::TopBarHome`
/// 进首页都重置为默认值,见 `workspace.rs` 对应处理器)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HomeLeftView {
    #[default]
    ProjectList,
    Recents,
}

/// 首页右栏当前显示哪个 pane。目前只有 `Browser` 一个变体,为将来扩展占位
/// (呼应"以后再加其它 pane"的既定方向)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HomeRightView {
    #[default]
    Browser,
}
```

(`pub(crate)`,不是 `pub`——`Message` 枚举的变体类型只需要 crate 内可见,同
`HomeRecentFile` 现状的可见性理由一致,见 `private_interfaces` lint。)

在 `homespace.rs` 已有的 `#[cfg(test)] mod tests { use super::*; .. }` 块里
(Task 1 建的,现在有 3 个 `load_home_recents_*` 测试)追加 2 个测试,断言
`Default` 实现:

```rust
    #[test]
    fn home_left_view_defaults_to_project_list() {
        assert_eq!(HomeLeftView::default(), HomeLeftView::ProjectList);
    }

    #[test]
    fn home_right_view_defaults_to_browser() {
        assert_eq!(HomeRightView::default(), HomeRightView::Browser);
    }
```

- [ ] **Step 4: `App` 新增字段**

`workspace.rs` 里 `pub struct App { .. }` 定义中,找到这一段(紧邻在一起):

```rust
    recent_projects: Vec<ProjectInfo>,
    /// H0"最近的文件"卡数据(D4);`Message::TopBarHome` 时异步刷新。
    home_recent_files: Vec<HomeRecentFile>,
    /// H0"最近的对话"卡数据(D4);语义同上。
    home_recent_conversations: Vec<HomeRecentConversation>,
    /// 是否已经收到过至少一次 `HomeRecentsLoaded`——区分"还在加载"与"加载完
    /// 但结果为空"，两张卡据此决定画"加载中…"还是空状态文案(spec §4)。
    home_recents_loaded: bool,
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
```

改成(字段全部标 `pub(crate)`——`homespace.rs` 的视图函数要读它们;并在
`home_recents_loaded` 之后、`git_log` 之前插入 3 个新字段):

```rust
    pub(crate) recent_projects: Vec<ProjectInfo>,
    /// H0"最近的文件"卡数据(D4);`Message::TopBarHome` 时异步刷新。
    pub(crate) home_recent_files: Vec<HomeRecentFile>,
    /// H0"最近的对话"卡数据(D4);语义同上。
    pub(crate) home_recent_conversations: Vec<HomeRecentConversation>,
    /// 是否已经收到过至少一次 `HomeRecentsLoaded`——区分"还在加载"与"加载完
    /// 但结果为空"，两张卡据此决定画"加载中…"还是空状态文案(spec §4)。
    pub(crate) home_recents_loaded: bool,
    /// 首页左栏当前显示哪个 pane(项目列表/Recents)。不持久化,每次
    /// `Message::TopBarHome` 进首页都重置为默认值——见
    /// `homespace::HomeLeftView`。
    pub(crate) home_left_view: homespace::HomeLeftView,
    /// 首页右栏当前显示哪个 pane(目前只有 Browser)。语义同上。
    pub(crate) home_right_view: homespace::HomeRightView,
    /// 首页全局浏览器面板状态,不挂在任何 `Workspace` 上;`view`/`update`
    /// 调用时 `project_id` 恒传 `None`(全局收藏夹作用域)。
    pub(crate) home_browser: browser::State,
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
```

**注意**:`recent_projects: Vec<ProjectInfo>,` 这个字段名在 `workspace.rs` 里
出现 3 次(`ProjectRestore`/`App`/`Workspace` 三个不同 struct 各有一个同名
字段,语义互不相同)。只改 **`App`** 这一个——用它紧邻的文档注释"H0 项目中心
侧栏用的"最近项目"列表(D2)。与 `Workspace.recent_projects` 语义相同但字段
独立"来定位,不要动 `ProjectRestore`/`Workspace` 里的同名字段。

再找到 `daemon_error: Option<String>,` 这一行(`App` struct 内,注释"daemon
连接失败,或某次会话操作失败时的错误文案"那条,不是 `new_shell` 函数参数里
同名的那个),改成 `pub(crate) daemon_error: Option<String>,`。

- [ ] **Step 5: `App::new_shell` 初始化新字段**

`workspace.rs` 里 `fn new_shell(..)` 函数体,找到:

```rust
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
            git_log: git_log::State::default(),
```

改成:

```rust
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
            home_left_view: homespace::HomeLeftView::default(),
            home_right_view: homespace::HomeRightView::default(),
            home_browser: browser::State::default(),
            git_log: git_log::State::default(),
```

- [ ] **Step 6: `Message` 枚举新增 3 个变体**

`workspace.rs` 的 `pub enum Message { .. }` 里找到:

```rust
    /// H0 项目中心:`Message::TopBarHome` 发起的异步刷新完成(最近改动的文件、
    /// 最近的对话两份列表;D4)。
    HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>),
```

紧接着(在这条和下一条 `ProjectTabSwitch` 之间)插入:

```rust
    /// 首页左图标栏:切换 `HomeLeftView`(项目列表/Recents)。首页没有
    /// collapse 概念,恒有一个 pane 显示,不像工作区 `LeftIconSelect` 那样
    /// 需要处理"点已选中图标收起面板区"的分支。
    HomeLeftIconSelect(homespace::HomeLeftView),
    /// 首页右图标栏:切换 `HomeRightView`(目前只有 Browser)。
    HomeRightIconSelect(homespace::HomeRightView),
    /// 首页全局浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。路由到 `app.home_browser`,
    /// `project_id` 恒传 `None`;与工作区 `Message::Browser` 路由到
    /// `ws.browser` 是两条独立路径,互不影响。
    HomeBrowser(browser::Message),
```

- [ ] **Step 7: `App::update` 新增 3 个处理器**

`workspace.rs` 的 `App::update` 里找到 `Message::HomeRecentsLoaded` 分支:

```rust
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
```

紧接着插入:

```rust
            Message::HomeLeftIconSelect(v) => {
                self.home_left_view = v;
            }
            Message::HomeRightIconSelect(v) => {
                self.home_right_view = v;
            }
            Message::HomeBrowser(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::HomeBrowser(m));
                };
                browser::update(&mut self.home_browser, msg, None, &client, &handle, emit);
            }
```

- [ ] **Step 8: 编译确认(状态/消息接线已完整,视图还没接上,这一步预期还是编译失败或有警告——先跳过,直接进 Step 9 一起验证)**

这一步不单独验证——`home_left_view`/`home_right_view`/`home_browser` 三个新
字段与 3 个新 `Message` 变体在 Step 9 视图重写完成前处于"写了但没被任何视图
函数读到/构造"的状态,单独编译会出 `dead_code` 警告(字段从未被读、变体从未被
构造)。这是本任务内部的过渡态,不是任务边界,不需要在这里停下来验证"干净"
——直接继续 Step 9,让视图重写把它们用起来,最终在 Step 11 一次性验证零警告。

- [ ] **Step 9: 新增 `home_divider`**

在 `workspace.rs` 里找到 `divider_bar` 函数(6664 行起)。**不要**在首页调用
它——它的 `Divider::LeftRight` 分支画的是可拖拽热区,`on_press` 直接派发
`Message::ColumnDragStart(Divider::LeftRight)`,接入工作区自己的拖宽状态机
(`self.dragging`/`self.shell_layout.left_width`);首页不做拖拽调宽(见
Global Constraints),复用会导致拖动这条分隔线时意外改写工作区宽度状态。

在 `divider_bar` 函数定义之后新增一个不接交互的静态分隔函数:

```rust
/// 首页左右栏之间的静态分隔(不可拖拽)。**不能**复用 `divider_bar
/// (Divider::LeftRight, ..)`——那个分支的 `MouseArea::on_press` 直接派发
/// `Message::ColumnDragStart`,接入工作区自己的拖宽状态机;首页没有拖拽
/// 调宽的产品需求(spec"目标"第 6 条),原样复用会在首页意外改写工作区
/// 的 `shell_layout` 宽度状态。这里只留一块与 `divider_width()` 同宽的
/// 空白——`Divider::LeftRight` 分支本身也不画任何可见的线/背景色,视觉
/// 效果与之一致,只是去掉了 `MouseArea`/`on_press`。
fn home_divider<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    iced_widget::Space::new()
        .width(Length::Fixed(theme::geometry::divider_width()))
        .height(Length::Fill)
        .into()
}
```

- [ ] **Step 10: 视图重写——`home_page` 四栏化 + 新增 rail/zone 函数,搬进 `homespace.rs`**

`workspace.rs` 现有这几个函数(4472-4804 行一带,以函数名定位,不依赖行号):
`home_page`、`home_sidebar`、`home_recents_column`、`home_recent_files_card`、
`home_recent_conversations_card`、以及 Step 9 刚写的 `home_divider`。把它们
**全部剪切**到 `homespace.rs`(追加在 `load_home_recents` 函数/测试模块之前),
按下面的新内容替换(不是逐字搬运——`home_page`/`home_sidebar`/
`home_recents_column` 三个的内容有实质改动,`home_recent_files_card`/
`home_recent_conversations_card` 只改最后的 `width`/`height` 两行,
`home_divider` 原样搬):

`homespace.rs` 顶部 `use` 块需要补充(加在已有的 `use crate::conversation::..`
等几行旁边):

```rust
use crate::extensions::browser;
use crate::icons;
use crate::theme;
use crate::workspace::{
    App, HoverId, Message, PaneCorner, RailButton, lh, rail_icon_button, relative_time_text,
    zone_pane_border,
};
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
```

(`Color`/`Font`/`iced_widget::core::mouse` 都不需要——移过来的这几个函数里没有
直接用到 `Color`/`Font` 类型本身,颜色一律走 `theme::color::X` 常量;
`home_divider` 也不再有 `MouseArea`/`interaction`,这是它和 `divider_bar`
唯一的行为差异。)

把 `workspace.rs` 里 `rail_icon_button`/`zone_pane_border`/`relative_time_text`
三个自由函数(以及 `PaneCorner` 枚举)的定义,分别加上 `pub(crate)`:

- `fn rail_icon_button<'a>(` → `pub(crate) fn rail_icon_button<'a>(`
- `fn zone_pane_border(zone: theme::region::RegionStyle, corner: PaneCorner) -> Border {` → `pub(crate) fn zone_pane_border(...)`(签名不变,只加可见性)
- `fn relative_time_text(modified_ms: u64, now_ms: u64) -> String {` → `pub(crate) fn relative_time_text(...)`
- `enum PaneCorner {` → `pub(crate) enum PaneCorner {`

`lh` 已经是 `pub(crate)`(`workspace.rs:6332` 现状),`RailButton`/`HoverId`
已经是 `pub`,`App::hover_progress` 已经是 `pub fn`——这 4 个不用改。

现在把剪切出来的函数,在 `homespace.rs` 里按以下内容重写/新增(`home_divider`
就是 Step 9 写的那个,原样贴过来,不用改):

```rust
/// 首页落地页(点顶栏 Dozer 进入):四栏结构,镜像工作区
/// `left_icon_rail / left_zone / right_zone / right_icon_rail`(见
/// `workspace::left_icon_rail`/`left_panel_area` 等),但不做拖拽调宽/收起/
/// 放大——首页没有这个产品需求。左栏默认"项目列表"(`HomeLeftView::
/// ProjectList`),右栏固定"浏览器"(全局态,不绑定项目)。
pub(crate) fn home_page(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let body = row![
        home_left_icon_rail(app),
        home_left_zone(app, now_ms),
        home_divider(),
        home_right_zone(app),
        home_right_icon_rail(app),
    ]
    .height(Length::Fill);

    let mut col = column![body].spacing(16).height(Length::Fill);
    if let Some(err) = &app.daemon_error {
        col = col.push(
            text(format!("⚠ {err}"))
                .size(theme::font::body())
                .color(theme::color::RED),
        );
    }

    container(col.padding(24))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::region::background().into()),
            ..container::Style::default()
        })
        .into()
}

/// 首页左右栏之间的静态分隔(不可拖拽)。**不能**复用 `workspace::divider_bar
/// (Divider::LeftRight, ..)`——那个分支的 `MouseArea::on_press` 直接派发
/// `Message::ColumnDragStart`,接入工作区自己的拖宽状态机;首页没有拖拽
/// 调宽的产品需求(spec"目标"第 6 条),原样复用会在首页意外改写工作区
/// 的 `shell_layout` 宽度状态。这里只留一块与 `divider_width()` 同宽的
/// 空白——`Divider::LeftRight` 分支本身也不画任何可见的线/背景色,视觉
/// 效果与之一致,只是去掉了 `MouseArea`/`on_press`。
fn home_divider<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    iced_widget::Space::new()
        .width(Length::Fixed(theme::geometry::divider_width()))
        .height(Length::Fill)
        .into()
}

/// 首页左图标栏:项目列表 / Recents 两个图标。没有 collapse 概念(见
/// `Message::HomeLeftIconSelect` 处理器注释),点哪个就切到哪个,恒有一个
/// pane 显示。
fn home_left_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = theme::region::left_icon_rail();
    let content = column![
        MouseArea::new(rail_icon_button(
            icons::IconKind::LayoutList,
            app.home_left_view == HomeLeftView::ProjectList,
            app.hover_progress(HoverId::Rail(RailButton::HomeProjectList)),
            Message::HomeLeftIconSelect(HomeLeftView::ProjectList),
        ))
        .on_enter(Message::Hover(
            HoverId::Rail(RailButton::HomeProjectList),
            true
        ))
        .on_exit(Message::Hover(
            HoverId::Rail(RailButton::HomeProjectList),
            false
        )),
        MouseArea::new(rail_icon_button(
            icons::IconKind::History,
            app.home_left_view == HomeLeftView::Recents,
            app.hover_progress(HoverId::Rail(RailButton::HomeRecents)),
            Message::HomeLeftIconSelect(HomeLeftView::Recents),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::HomeRecents), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::HomeRecents), false)),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 首页右图标栏:只有浏览器一个图标(`HomeRightView` 目前只有一个变体)。
fn home_right_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = theme::region::right_icon_rail();
    let content = column![
        MouseArea::new(rail_icon_button(
            icons::IconKind::Globe,
            app.home_right_view == HomeRightView::Browser,
            app.hover_progress(HoverId::Rail(RailButton::HomeBrowser)),
            Message::HomeRightIconSelect(HomeRightView::Browser),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::HomeBrowser), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::HomeBrowser), false)),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 首页左栏:按 `app.home_left_view` 切换项目列表/Recents 两个 pane。固定宽
/// `h0_sidebar_width()`,不支持拖拽调宽(见 `home_page` 文档)。
fn home_left_zone(app: &App, now_ms: u64) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let zone = theme::region::left_zone();
    let inner: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
        match app.home_left_view {
            HomeLeftView::ProjectList => home_project_list_view(app, now_ms),
            HomeLeftView::Recents => home_recents_view(app, now_ms),
        };
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(zone.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: zone.background.map(Into::into),
            border: zone.border.unwrap_or_default(),
            ..container::Style::default()
        });
    let m = zone.margin;
    container(zone_box)
        .width(Length::Fixed(theme::geometry::h0_sidebar_width()))
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 首页右栏:恒显示全局浏览器 pane(`app.home_browser`,`project_id` 传
/// `None`)。铺满剩余宽度,不支持拖拽调宽。
fn home_right_zone(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let zone = theme::region::right_zone();
    let inner = browser::view(
        &app.home_browser,
        None,
        Length::Fill,
        zone_pane_border(zone, PaneCorner::All),
    )
    .map(Message::HomeBrowser);
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(zone.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: zone.background.map(Into::into),
            border: zone.border.unwrap_or_default(),
            ..container::Style::default()
        });
    let m = zone.margin;
    container(zone_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 首页左栏"项目列表" pane(`HomeLeftView::ProjectList`):搜索框占位(D7)、
/// 最近项目卡(取 `app.recent_projects` 前 5 条,D2/D3)、"更多项目"占位(D7)、
/// "＋新增项目"(复用 `Message::ProjectTabPickFolder`)。不画品牌行——顶栏
/// 本身已有 `dozer_home_tab` 品牌页签,这里重复画属于视觉冗余。
/// `app.recent_projects` 为空时画"还没有项目"兜底文案,不崩(spec §4)。
fn home_project_list_view(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![].spacing(16);

    col = col.push(
        text("我的项目")
            .size(theme::font::caption())
            .color(theme::color::DIM),
    );

    // 搜索框:视觉占位,不接线(D7；precedent:顶栏 ⌘K 搜索框同款"先视觉后接线")。
    col = col.push(
        container(
            row![
                icons::view(
                    icons::IconKind::Search,
                    crate::theme::icon_size::row(),
                    theme::color::DIM
                ),
                text("搜索项目…")
                    .size(theme::font::body())
                    .color(theme::color::DIM),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        }),
    );

    if app.recent_projects.is_empty() {
        col = col.push(
            text("还没有项目")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else {
        let mut list = column![].spacing(8);
        for p in app.recent_projects.iter().take(5) {
            let card = button(
                column![
                    lh(text(p.name.clone())
                        .size(theme::font::body())
                        .color(theme::color::CREAM)),
                    lh(text(relative_time_text(p.last_active_ms, now_ms))
                        .size(theme::font::caption_sm())
                        .color(theme::color::DIM)),
                    lh(text(p.path.clone())
                        .size(theme::font::caption_sm())
                        .color(theme::color::DIM)),
                ]
                .spacing(2),
            )
            .on_press(Message::ProjectSelect(p.id))
            .width(Length::Fill)
            .padding(10)
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: Some(theme::color::CARD.into()),
                text_color: theme::color::CREAM,
                border: Border {
                    color: theme::color::BORDER,
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
                .direction(scrollable::Direction::Vertical(
                    crate::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| crate::scrollbar::scrollbar_style()),
        );
    }

    // "更多项目":视觉占位,不接线——对应的"全部项目列表"视图现在不存在,
    // 属于后续增量(D7)。
    col = col.push(
        container(
            text("更多项目")
                .size(theme::font::caption())
                .color(theme::color::DIM),
        )
        .padding([6, 0]),
    );

    col = col.push(
        button(
            text("＋新增项目")
                .size(theme::font::body())
                .color(theme::color::GOLD),
        )
        .on_press(Message::ProjectTabPickFolder)
        .padding([8, 16])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            text_color: theme::color::GOLD,
            ..button::Style::default()
        }),
    );

    container(col).width(Length::Fill).height(Length::Fill).into()
}

/// 首页左栏"Recents" pane(`HomeLeftView::Recents`):合并原"最近的文件"/
/// "最近的对话"两卡。左栏宽度固定较窄(`h0_sidebar_width()`),两卡挤不下
/// 并排,改上下堆叠(两张卡自己的外层容器相应把 `width`/`height` 的
/// `FillPortion` 轴对调,见 `home_recent_files_card`/
/// `home_recent_conversations_card`)。
fn home_recents_view(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    column![
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
            .size(theme::font::subtitle())
            .color(theme::color::CREAM)
    ]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else if app.home_recent_files.is_empty() {
        col = col.push(
            text("暂无最近改动的文件")
                .size(theme::font::body())
                .color(theme::color::DIM),
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
                    crate::theme::icon_size::row(),
                    theme::color::DIM
                ),
                column![
                    lh(text(filename.clone())
                        .size(theme::font::body())
                        .color(theme::color::CREAM)),
                    lh(text(format!(
                        "{} · {}",
                        f.project_name,
                        relative_time_text(f.modified_ms, now_ms)
                    ))
                    .size(theme::font::caption_sm())
                    .color(theme::color::DIM)),
                ]
                .spacing(2),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center);
            col = col.push(container(row_el).padding(10).width(Length::Fill).style(
                |_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::color::CARD.into()),
                    border: Border {
                        color: theme::color::BORDER,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                },
            ));
        }
    }

    container(col)
        .width(Length::Fill)
        .height(Length::FillPortion(1))
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
            .size(theme::font::subtitle())
            .color(theme::color::CREAM)
    ]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else if app.home_recent_conversations.is_empty() {
        col = col.push(
            text("暂无对话记录")
                .size(theme::font::body())
                .color(theme::color::DIM),
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
                            .size(theme::font::body())
                            .color(theme::color::CREAM)),
                        lh(text(sub)
                            .size(theme::font::caption_sm())
                            .color(theme::color::DIM)),
                    ]
                    .spacing(4),
                )
                .padding(10)
                .width(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::color::CARD.into()),
                    border: Border {
                        color: theme::color::BORDER,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                }),
            );
        }
    }

    container(col)
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .into()
}
```

以上贴的内容里,`home_recent_files_card`/`home_recent_conversations_card`
和原版唯一的区别就是函数末尾:原来是
`.width(Length::FillPortion(1)).height(Length::Fill)`(两卡并排在 `row!`
里,靠 `width` 的 `FillPortion` 分横向空间),现在改成
`.width(Length::Fill).height(Length::FillPortion(1))`(两卡上下堆叠在
`column!` 里,改靠 `height` 的 `FillPortion` 分纵向空间)——**这是一处容易
漏改的关键点**,漏改的话两卡在 `home_recents_view` 里会挤成异常的宽高比,
不会报错但视觉会明显不对(用人工验收步骤 Step 12 能立刻发现)。

`home_project_list_view` 与原 `home_sidebar` 的区别:删掉了最前面画品牌行
(Dozer 图标 + "Dozer" 文字 + 版本号)的那个 `col.push(row![icons::view(..
IconKind::Dozer..), text("Dozer")..., text(format!("v{}"...))...])` 块,
函数末尾 `container(col).width(Length::Fixed(theme::geometry::
h0_sidebar_width()))` 改成 `container(col).width(Length::Fill)`(外层宽度
现在由 `home_left_zone` 统一控制)。

`workspace.rs` 里 Step 9 加的 `home_divider`(整个函数)现在应该已经不在
`workspace.rs` 了(搬进了上面 `homespace.rs` 的内容),如果你是先在
`workspace.rs` 写了 `home_divider` 再整体剪切,记得连它一起剪切干净,不要
留一份重复定义在 `workspace.rs`。

- [ ] **Step 11: `workspace.rs` 收尾——加 `RailButton` 新变体,调用改前缀**

`workspace.rs` 的 `pub enum RailButton { .. }` 加 3 个变体(在
`RightAcceptance` 之后):

```rust
    /// 首页左栏"项目列表" pane 图标。
    HomeProjectList,
    /// 首页左栏"Recents" pane 图标。
    HomeRecents,
    /// 首页右栏"浏览器" pane 图标。
    HomeBrowser,
```

`workspace.rs` 的 `App::view` 方法里,原来是:

```rust
        if self.current_page == AppPage::Home {
            return column![top, home_page(self)].into();
        }
```

改成(加 `homespace::` 前缀,因为 `home_page` 现在定义在 `homespace.rs`):

```rust
        if self.current_page == AppPage::Home {
            return column![top, homespace::home_page(self)].into();
        }
```

确认 `workspace.rs` 顶部 Task 1 加的那行 `use crate::homespace::{self,
HomeRecentFile, HomeRecentConversation, load_home_recents};` 里的 `self` 现在
派上用场了(`homespace::home_page(self)` 这个调用需要它)。

再确认 `homespace.rs` 里没有重复定义 `HomeRecentFile`/`HomeRecentConversation`
——Step 10 新增的 `home_project_list_view`/`home_recents_view`/两个卡片函数
应该是通过 `super::` 或直接同模块访问 Task 1 已经在同一个 `homespace.rs` 文件
里定义好的 `HomeRecentFile`/`HomeRecentConversation`/`load_home_recents`,不
需要任何额外 `use`(同一个文件内的项互相可见,不需要 `use self::..`)。

- [ ] **Step 12: 全量编译/测试/lint/格式验证**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全部干净通过,**零警告**(包括 Task 2 Step 1-8 新增的图标变体、
`App` 字段、`Message` 变体、`RailButton` 变体——到这一步应该全部被 Step
10-11 的视图代码实际用上了,不应该再有任何 dead_code 提示)。如果出现
dead_code 警告,回头检查是不是漏接了某处(最常见的漏接点:忘了把
`workspace.rs` 里原来对 `home_page`/`home_sidebar` 等旧函数名的调用点改成
`homespace::home_page`,或者 Step 10 剪切时漏删了 `workspace.rs` 里的旧函数
定义导致新旧两份同时存在、新的那份没被调用)。

- [ ] **Step 13: 人工验收**

```bash
cargo run -p dozer-app
```

按以下清单人工检查(与设计文档"测试策略"人工验收清单一致):

1. 点顶栏 Dozer 页签进首页:应看到左图标栏(2 个图标:项目列表/Recents)、
   左栏(默认项目列表内容:"我的项目"标题+搜索框占位+最近项目卡列表+"更多
   项目"占位+"＋新增项目"按钮,**不再有** Dozer 图标+"Dozer"+版本号那一行)、
   中间静态分隔、右栏(浏览器,能看到地址栏/tab 栏)、右图标栏(1 个浏览器
   图标)。四栏整体视觉(rail 背景色、zone 圆角/边框、留白间距)应与切到
   工作区页面时的四栏观感一致。
2. 点左图标栏"Recents"图标:左栏切成"最近的文件"卡(上)+"最近的对话"卡
   (下)上下堆叠,两卡各自内容(文件名/项目名/相对时间,对话标题/项目名/
   agent/相对时间)完整可读,没有挤压变形。再点回"项目列表"图标,切回去。
3. 右栏浏览器:能在地址栏输入网址打开页面,能点星标收藏(全局收藏夹,
   不需要打开任何项目)。
4. 从首页点一张最近项目卡切进某个项目的工作区,再点顶栏 Dozer 页签切回
   首页:左右栏应该都回到默认态(项目列表 + 浏览器),不残留上次切换到
   Recents 的状态。
5. 全新用户场景(`app.recent_projects` 为空):项目列表 pane 显示"还没有
   项目"文案,不崩溃。
6. 如果能触发 daemon 连接失败(`daemon_error` 有值)的场景:首页仍能正常
   渲染四栏,底部会多一行红色错误文案。

- [ ] **Step 14: Commit**

```bash
git add crates/dozer-app/assets/icons/layout-list.svg crates/dozer-app/assets/icons/history.svg crates/dozer-app/src/icons.rs crates/dozer-app/src/homespace.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): rebuild home page as 4-column icon-rail shell"
```

---

### Task 3: 全 workspace 收尾验证

**Files:** 无新改动,只跑验证命令。

- [ ] **Step 1: 全 workspace 构建**

```bash
cargo build
```

预期:整个 workspace(不只 `dozer-app`)编译通过——`homespace.rs` 是新文件,
确认没有漏掉任何跨 crate 引用问题(理论上不会有,`homespace.rs` 只被
`dozer-app` 自己用)。

- [ ] **Step 2: 全 workspace 测试**

```bash
cargo test
```

预期:全部 PASS。

- [ ] **Step 3: 全 workspace lint/格式**

```bash
cargo clippy --all-targets
cargo fmt --check
```

预期:干净通过,零警告。

- [ ] **Step 4: 确认分支状态,准备提请审阅**

```bash
git status
git log --oneline main..HEAD
```

确认工作区干净(无未提交改动)、当前分支(`feature/home-page-4col-layout`)
领先 `main` 的提交只有 Task 1/2 那两个 commit,没有意外混入其它工作。之后
按 Global Constraints 提请代码审阅,审阅通过后再合并回 `main`——不在这个
计划里自动合并。
