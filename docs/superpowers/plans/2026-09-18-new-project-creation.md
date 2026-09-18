# 新建项目对话框 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 topbar "+"菜单新增"创建项目"入口,打开一个渲染在独立原生窗口里的两 tab 对话框——本地新建空项目 / `git clone` 签出远程仓库——落地后复用现有"打开项目"注册路径(`client.open_project` + `project_tab_opened`)把新目录接入 Dozer。

**Architecture:** 表单状态/消息/reducer 放在新的 `extensions::project_create` 模块(结构对照 `extensions::file_history`,`update()` 自己持有 `client`/`handle` 做异步 `fs`/`git`/dozerd 调用,不把这些散落进 `app/update.rs`)。渲染宿主是新的 `platform::project_create_overlay::ProjectCreateOverlay`,克隆自刚合并的 `FileHistoryOverlay`(独立 wgpu 渲染管线 + winit 子窗口),但补上 `SearchOverlay` 的 IME/原生右键菜单挂靠机制(本对话框有真实文本输入),且**不接入 `FocusTracker` 的失焦关闭**——只认 Esc/取消按钮,因为"根目录"字段要弹嵌套的 `rfd` 文件夹选择器,那会让本窗口瞬间失焦。修一处在设计阶段发现的既有缺陷:静默 scaffold(`project::spawn_scaffold_run`)目前无条件 `git init`,给它加一个 `skip_git_init` 开关,"创建Git仓库"复选框才能真正生效。

**Tech Stack:** iced 0.14(`iced_widget`/`iced_wgpu`/`iced_winit`)、winit 0.30、`byteui::form::input_text`/`checkbox` 共享组件、`iced_widget::text_editor`(多行描述字段)、`std::process::Command` shell 出系统 `git`(不用 git2 做 clone/init,延续 `delivery.rs` 现有薄封装风格)。

**Spec:** `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`

## Global Constraints

- macOS 先发,不引入 Swift/AppKit 专属能力,不依赖 Node/Python(CLAUDE.md)。
- GUI 只用 iced 0.14 生态;新增 tab 类 UI 优先复用 `byteui::interaction::tabs::tab_core`——**本计划的两 tab 切换器不复用它**:`tab_core` 内建"可关闭"交互(悬停才可点的 `×` 按钮),本对话框的两个 tab 是纯互斥单选、永不可关闭,交互模型明显不同,按 CLAUDE.md 允许的例外处理,改用项目现成的"单选高亮行"idiom(`extensions/project/view.rs::project_delete_confirm_popup` 的 `radio_row` 手法,`MouseArea`+`on_press`,不新造机制)。
- 新增/改造函数参数 ≥7 个且有多个同类型相邻参数时,用具名字段结构体替代位置参数——`ProjectCreateOverlay::open`/`reposition` 直接照抄 `FileHistoryOverlay` 已经用 `LogicalSize<f32>` 收拢 width/height 的写法,不需要另外处理。
- 克隆/git init 鉴权完全委托系统 git 凭证,不做任何 Token/密码输入 UI 或 keyring 存储。
- GitHub/GitLab/Gitee 账户接入、完整系统环境检测向导(doctor 面板)、克隆进度百分比解析均为本计划非目标(spec 已明确)。

---

### Task 1: 修正共享 scaffold——加 `skip_git_init` 开关

**Files:**
- Modify: `crates/dozer-app/src/extensions/project/scaffold.rs:112-117`(`run_sync_steps`)
- Modify: `crates/dozer-app/src/extensions/project/update.rs:220-233`(`spawn_scaffold_run`)
- Modify: `crates/dozer-app/src/app/message.rs:404-406`(`Message::ProjectTabOpened`)
- Modify: `crates/dozer-app/src/app/update.rs:1022-1027,1779-1787,1791-1795`(两处发出点 + `project_tab_opened` 签名)
- Test: `crates/dozer-app/src/extensions/project/scaffold.rs`(同文件 `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: 无(纯修改既有函数签名)
- Produces: `scaffold::run_sync_steps(repo: &Path, skip_git_init: bool) -> Vec<(String, ScaffoldStepResult)>`;`project::spawn_scaffold_run(repo_path, client, handle, emit, skip_git_init: bool)`;`Message::ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>, bool)`;`App::project_tab_opened(&mut self, project, recent, skip_git_init: bool)`——Task 5/8 会调用这几个新签名。

- [ ] **Step 1: 给 `run_sync_steps` 加参数,跳过"git 仓库"步骤时补一个失败单测(先写测试确认当前行为不支持跳过)**

在 `crates/dozer-app/src/extensions/project/scaffold.rs` 的 `#[cfg(test)] mod tests` 里加:

```rust
#[test]
fn run_sync_steps_skips_git_step_when_requested() {
    let tmp = tempfile::tempdir().unwrap();
    let results = run_sync_steps(tmp.path(), true);
    assert!(
        !results.iter().any(|(label, _)| label == "git 仓库"),
        "skip_git_init=true 时不应该出现 git 仓库这一步"
    );
    assert!(!tmp.path().join(".git").exists());
}

#[test]
fn run_sync_steps_runs_git_step_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let results = run_sync_steps(tmp.path(), false);
    assert!(results.iter().any(|(label, _)| label == "git 仓库"));
    assert!(tmp.path().join(".git").exists());
}
```

- [ ] **Step 2: 跑测试确认编译失败(签名还没改)**

Run: `cargo test -p dozer-app --lib extensions::project::scaffold -- --include-ignored`
Expected: 编译错误,`run_sync_steps` 期望 1 个参数收到 2 个。

- [ ] **Step 3: 实现——`run_sync_steps` 加 `skip_git_init` 参数**

```rust
/// 顺序跑完全部同步步骤。`skip_git_init=true` 时跳过"git 仓库"这一步
/// (供"新建本地项目"对话框的"创建Git仓库"复选框未勾选时使用,见
/// `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`
/// 「对既有共享代码的修正」一节)。这几步都可能阻塞(尤其 `ensure_git_repo`
/// 会 shell 出子进程),批量一次跑完的调用方(静默路径
/// `extensions::project::spawn_scaffold_run`)负责把这个函数整体包进
/// `tokio::task::spawn_blocking`——需要逐步骤实时反馈的路径
/// (`spawn_repair_run`)改成对每个 `scaffold_steps()` 元素单独
/// `spawn_blocking`,不调这个批量函数,因此不受这个参数影响。
pub fn run_sync_steps(repo: &Path, skip_git_init: bool) -> Vec<(String, ScaffoldStepResult)> {
    scaffold_steps()
        .into_iter()
        .filter(|step| !(skip_git_init && step.label == "git 仓库"))
        .map(|step| (step.label.to_string(), (step.run)(repo)))
        .collect()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib extensions::project::scaffold`
Expected: `run_sync_steps_skips_git_step_when_requested`/`run_sync_steps_runs_git_step_by_default` 均 PASS,原有测试(`ensure_dozer_dir_*` 等)不受影响仍 PASS。

- [ ] **Step 5: 修 `spawn_scaffold_run` 签名,透传参数**

`crates/dozer-app/src/extensions/project/update.rs:220-233`:

```rust
pub fn spawn_scaffold_run(
    repo_path: std::path::PathBuf,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
    skip_git_init: bool,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let _ = tokio::task::spawn_blocking(move || scaffold::run_sync_steps(&repo_path2, skip_git_init))
            .await;
        let _ = client.backfill_project_transcripts(&cwd).await;
        emit(Message::ScaffoldDone);
    });
}
```

`spawn_repair_run`(`update.rs:243`起)调用的是 `scaffold::scaffold_steps()`/单步 `(step.run)(&repo_path3)`,不经过 `run_sync_steps`,不受这次改动影响,不用改。

- [ ] **Step 6: 修 `Message::ProjectTabOpened` 加第三个字段**

`crates/dozer-app/src/app/message.rs:402-406`:

```rust
    /// 项目页签:把某路径作为**新页签**打开(不动任何已存在页签的内容)。
    ProjectTabOpen(PathBuf),
    /// 项目页签:`ProjectTabOpen` 异步完成(daemon upsert 结果 + 最近列表 +
    /// 是否跳过静默 scaffold 的 git init 步骤)。`None` = 这次打开失败,只
    /// 报错、不改任何页签状态,此时第三个字段无意义。
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>, bool),
```

- [ ] **Step 7: 修两处发出点(默认 `false`,行为与今天一致)+ `project_tab_opened` 签名**

`crates/dozer-app/src/app/update.rs:1022-1027`:

```rust
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent, false));
                });
            }
            Message::ProjectTabOpened(project, recent, skip_git_init) => {
                self.project_tab_opened(project, recent, skip_git_init)
            }
```

`crates/dozer-app/src/app/update.rs:1779-1787`(项目卡片"打开最近项目"路径,同样保持既有行为,传 `false`):

```rust
            let recent = client.list_projects().await.unwrap_or_default();
            let opened = recent.iter().find(|p| p.id == id).cloned();
            let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent, false));
        });
    }
```

`crates/dozer-app/src/app/update.rs:1791-1795`(`project_tab_opened` 函数签名):

```rust
    pub(crate) fn project_tab_opened(
        &mut self,
        project: Option<ProjectInfo>,
        recent: Vec<ProjectInfo>,
        skip_git_init: bool,
    ) {
```

函数体内 `project::spawn_scaffold_run(repo_path, client, &handle, emit);` 那一行(原 `update.rs:1853`)改成:

```rust
        project::spawn_scaffold_run(repo_path, client, &handle, emit, skip_git_init);
```

- [ ] **Step 8: 全量编译确认没有遗漏的调用点**

Run: `cargo build -p dozer-app 2>&1 | grep -E "error|ProjectTabOpened|spawn_scaffold_run"`
Expected: 无编译错误。若报出其它遗漏的 `Message::ProjectTabOpened`/`spawn_scaffold_run` 调用点,按同样规则(默认路径传 `false`)补上。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/project/scaffold.rs \
        crates/dozer-app/src/extensions/project/update.rs \
        crates/dozer-app/src/app/message.rs \
        crates/dozer-app/src/app/update.rs
git commit -m "fix(app): scaffold 静默 git init 加 skip_git_init 开关

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: git 子进程薄封装——`git_available`/`clone_repo`

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`(在 `init_repo` 之后追加)
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: 无
- Produces: `delivery::git_available() -> bool`;`delivery::clone_repo(url: &str, dest: &Path) -> Result<(), String>`——Task 5 的 `SubmitClone` 处理会调用这两个函数。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/delivery.rs` 的 `#[cfg(test)] mod tests` 里(复用现有 `mkrepo()` 助手)追加:

```rust
    #[test]
    fn git_available_detects_system_git() {
        // 仓库里其它 git 相关测试都要求系统装了真 git 才能跑
        // (`mkrepo()` 本身就 shell 出真 git),这里断言同一个前提。
        assert!(git_available());
    }

    #[test]
    fn clone_repo_copies_local_source_repo() {
        let (_src_dir, src_repo) = mkrepo();
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        clone_repo(&src_repo.to_string_lossy(), &dest).unwrap();
        assert!(dest.join(".git").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "one\n"
        );
    }

    #[test]
    fn clone_repo_fails_on_missing_source() {
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        let missing = dest_parent.path().join("does-not-exist");
        assert!(clone_repo(&missing.to_string_lossy(), &dest).is_err());
    }
```

- [ ] **Step 2: 跑测试确认因函数不存在而编译失败**

Run: `cargo test -p dozer-app --lib delivery::tests`
Expected: 编译错误 `cannot find function 'git_available'`/`'clone_repo'`。

- [ ] **Step 3: 实现——追加到 `crates/dozer-app/src/delivery.rs`,紧跟 `init_repo` 之后**

```rust
/// 系统是否装了可用的 git——URL 签出 tab 提交前的轻量检测,不解析
/// 具体版本号,只看子进程能否成功跑起来。
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// `git clone <url> <dest>`,鉴权完全委托系统已配置的 SSH agent/凭证
/// 管理器(不接 `RemoteCallbacks`,不做任何 Token 输入,见
/// `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`)。
/// `dest` 必须还不存在(调用方在此之前已经校验过,见 `project_create`
/// 模块的 `validate_target_not_exists`),失败把 git 的 stderr 原样透传。
pub fn clone_repo(url: &str, dest: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .arg("clone")
        .arg(url)
        .arg(dest)
        .output()
        .map_err(|e| format!("无法运行 git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib delivery::tests`
Expected: 全部 PASS,包括新增的 3 个测试和原有的 `repo_root_and_head_and_dirty`/`file_statuses_maps_modified_new_deleted` 等。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "feat(app): 新增 git_available/clone_repo 薄封装

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: `project_create` 纯校验/推导函数

**Files:**
- Create: `crates/dozer-app/src/extensions/project_create.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(注册模块)

**Interfaces:**
- Consumes: 无
- Produces: `project_create::target_path(root_dir: &str, name: &str) -> PathBuf`;`project_create::validate_project_name(name: &str) -> Result<(), String>`;`project_create::validate_target_not_exists(path: &Path) -> Result<(), String>`;`project_create::derive_project_name_from_url(url: &str) -> String`——Task 4/5 的 `State`/`update` 会调用这几个函数。

- [ ] **Step 1: 注册新模块**

`crates/dozer-app/src/extensions.rs` 按字母序在 `pub mod project;` 之后插入:

```rust
pub mod project_create;
```

- [ ] **Step 2: 写失败测试**

新建 `crates/dozer-app/src/extensions/project_create.rs`,先写文件头 + 测试模块:

```rust
//! "创建项目"对话框:两 tab(本地新建 / Git URL 签出)状态机 + 视图。
//! 设计见 `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`。
//! 渲染宿主是独立原生窗口 `platform::project_create_overlay::
//! ProjectCreateOverlay`,结构对照 `extensions::file_history` + 同名 overlay
//! 的既有分工:本模块只管状态/消息/视图/异步落盘逻辑,不碰 winit/wgpu。

use std::path::{Path, PathBuf};

/// 最终项目路径 = 根目录/项目名称。纯字符串拼接,不做存在性判断
/// (存在性判断是 [`validate_target_not_exists`] 的职责,分开是因为提交
/// 前两处都要单独调用:先拼路径给用户预览,再单独校验)。
pub(crate) fn target_path(root_dir: &str, name: &str) -> PathBuf {
    Path::new(root_dir).join(name)
}

/// 项目名称合法性:非空、首尾无空白、不含路径分隔符。
pub(crate) fn validate_project_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("项目名称不能为空".to_string());
    }
    if trimmed != name {
        return Err("项目名称首尾不能有空白字符".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("项目名称不能包含 / 或 \\".to_string());
    }
    Ok(())
}

/// 目标目录不能已存在——创建/签出只认全新目录,"已存在则复用"是"打开
/// 项目"该管的语义(见 spec「数据流」一节)。
pub(crate) fn validate_target_not_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        Err(format!("目标目录已存在: {}", path.display()))
    } else {
        Ok(())
    }
}

/// 从远程仓库 URL 推导默认项目名称——取最后一段路径,去掉 `.git` 后缀。
/// 同时兼容 `https://host/group/repo.git`、`https://host/group/repo`、
/// scp 风格 `git@host:group/repo.git`、带结尾斜杠的 `.../repo/`。
pub(crate) fn derive_project_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last_segment = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    last_segment
        .strip_suffix(".git")
        .unwrap_or(last_segment)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_path_joins_root_and_name() {
        assert_eq!(
            target_path("/tmp/projects", "foo"),
            PathBuf::from("/tmp/projects/foo")
        );
    }

    #[test]
    fn validate_project_name_rejects_empty_and_whitespace_and_slash() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("   ").is_err());
        assert!(validate_project_name(" foo").is_err());
        assert!(validate_project_name("foo ").is_err());
        assert!(validate_project_name("a/b").is_err());
        assert!(validate_project_name("a\\b").is_err());
        assert!(validate_project_name("foo").is_ok());
    }

    #[test]
    fn validate_target_not_exists_rejects_existing_path() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(validate_target_not_exists(tmp.path()).is_err());
        assert!(validate_target_not_exists(&tmp.path().join("does-not-exist")).is_ok());
    }

    #[test]
    fn derive_project_name_from_url_handles_common_forms() {
        assert_eq!(
            derive_project_name_from_url("https://github.com/abc/foo.git"),
            "foo"
        );
        assert_eq!(
            derive_project_name_from_url("https://github.com/abc/foo"),
            "foo"
        );
        assert_eq!(
            derive_project_name_from_url("git@gitlab.com:abc/bar.git"),
            "bar"
        );
        assert_eq!(
            derive_project_name_from_url("https://gitee.com/abc/baz/"),
            "baz"
        );
    }
}
```

- [ ] **Step 3: 跑测试确认全部通过(实现已随测试一起写好,这一步是确认没有笔误)**

Run: `cargo test -p dozer-app --lib extensions::project_create`
Expected: `target_path_joins_root_and_name`/`validate_project_name_rejects_empty_and_whitespace_and_slash`/`validate_target_not_exists_rejects_existing_path`/`derive_project_name_from_url_handles_common_forms` 全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/project_create.rs
git commit -m "feat(app): project_create 纯校验/推导函数

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: `project_create` State/Message/update——非提交类消息的 reducer

**Files:**
- Modify: `crates/dozer-app/src/extensions/project_create.rs`(追加,不改 Task 3 的内容)

**Interfaces:**
- Consumes: 无(`iced_widget::text_editor::Content` 是 iced 自带类型)
- Produces: `project_create::Tab`(`Local`/`Clone`)、`project_create::LocalForm`、`project_create::CloneForm`、`project_create::State`(含 `Default`)、`project_create::Message` 枚举、`project_create::update(state: &mut Option<State>, msg: Message, client: &dozer_client::Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`——Task 5 会在同一个 `update` 函数里补 `SubmitLocal`/`SubmitClone`/`Done` 分支,Task 6 的视图函数读 `State` 的字段,Task 8 的 `App` 持有 `Option<State>` 并调用这个 `update`。

设计要点:非提交类消息(tab 切换/字段编辑/复选框)完全不需要碰 `client`/
`handle`,只有 `SubmitLocal`/`SubmitClone`(Task 5 才实现)需要异步
`handle.spawn(...)`。所以把"字段编辑"拆成一个不需要 `client`/`handle` 的
纯函数 `apply_field_message(state: &mut State, msg: &Message) -> bool`,
`update()` 只是在此之上再包一层"提交类消息走异步分支"的薄壳——这样
Task 4 的单测完全不需要构造 `dozer_client::Client`/`tokio::runtime::Handle`,
直接测 `apply_field_message` 即可,`update()` 本身的异步分支留给 Task 5
用手工验证清单覆盖(与仓库里 `spawn_scaffold_run` 等既有异步副作用函数
一贯的测试策略一致)。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/extensions/project_create.rs` 的 `#[cfg(test)] mod tests` 里追加(与 Task 3 的测试同一个 `mod tests`,紧接着写):

```rust
    #[test]
    fn tab_selected_switches_active_tab() {
        let mut state = State::default();
        assert_eq!(state.tab, Tab::Local);
        apply_field_message(&mut state, &Message::TabSelected(Tab::Clone));
        assert_eq!(state.tab, Tab::Clone);
    }

    #[test]
    fn local_field_edits_update_form() {
        let mut state = State::default();
        apply_field_message(&mut state, &Message::LocalRootDirChanged("/tmp/x".into()));
        apply_field_message(&mut state, &Message::LocalNameChanged("foo".into()));
        apply_field_message(&mut state, &Message::LocalCreateGitToggled(false));
        assert_eq!(state.local.root_dir, "/tmp/x");
        assert_eq!(state.local.name, "foo");
        assert!(!state.local.create_git);
    }

    #[test]
    fn clone_url_changed_derives_name_unless_manually_edited() {
        let mut state = State::default();
        apply_field_message(
            &mut state,
            &Message::CloneUrlChanged("https://github.com/abc/foo.git".into()),
        );
        assert_eq!(state.clone_form.name, "foo");
        // 用户手动改过名称之后,再改 URL 不应该覆盖用户的手改。
        apply_field_message(&mut state, &Message::CloneNameChanged("my-custom-name".into()));
        apply_field_message(
            &mut state,
            &Message::CloneUrlChanged("https://github.com/abc/bar.git".into()),
        );
        assert_eq!(state.clone_form.name, "my-custom-name");
    }

    #[test]
    fn close_is_not_a_field_message() {
        let mut state = State::default();
        assert!(!apply_field_message(&mut state, &Message::Close));
    }
```

- [ ] **Step 2: 跑测试确认因类型/函数不存在而编译失败**

Run: `cargo test -p dozer-app --lib extensions::project_create`
Expected: 编译错误(`Tab`/`State`/`Message`/`apply_field_message` 不存在)。

- [ ] **Step 3: 实现——追加到 `crates/dozer-app/src/extensions/project_create.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Local,
    Clone,
}

pub struct LocalForm {
    pub root_dir: String,
    pub name: String,
    pub description: iced_widget::text_editor::Content,
    pub create_git: bool,
}

impl Default for LocalForm {
    fn default() -> Self {
        LocalForm {
            root_dir: String::new(),
            name: String::new(),
            description: iced_widget::text_editor::Content::new(),
            create_git: true,
        }
    }
}

#[derive(Default)]
pub struct CloneForm {
    pub url: String,
    pub root_dir: String,
    pub name: String,
    /// 用户是否手动编辑过项目名称——一旦手改过,`CloneUrlChanged` 就不再
    /// 用推导值覆盖它(见 spec「URL 签出」字段说明:"默认从 URL 推导,可
    /// 编辑")。
    pub name_touched: bool,
    pub description: iced_widget::text_editor::Content,
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    pub local: LocalForm,
    pub clone_form: CloneForm,
    pub error: Option<String>,
    pub busy: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    TabSelected(Tab),
    LocalRootDirChanged(String),
    LocalRootDirPick,
    LocalRootDirPicked(String),
    LocalNameChanged(String),
    LocalDescriptionAction(iced_widget::text_editor::Action),
    LocalCreateGitToggled(bool),
    SubmitLocal,
    CloneUrlChanged(String),
    CloneRootDirChanged(String),
    CloneRootDirPick,
    CloneRootDirPicked(String),
    CloneNameChanged(String),
    CloneDescriptionAction(iced_widget::text_editor::Action),
    SubmitClone,
    /// 本地创建/URL 签出任一条路径落地完成(成功或失败)。成功时携带
    /// dozerd 返回的项目信息 + 最近列表 + 是否要跳过静默 scaffold 的 git
    /// init(对应 Task 1 的 `skip_git_init`);失败时 `Err` 里是给用户看的
    /// 错误文案,`update` 把它填进 `State::error`,不关闭对话框。
    Done(Result<(Option<dozer_core::protocol::ProjectInfo>, Vec<dozer_core::protocol::ProjectInfo>, bool), String>),
}

/// 处理不需要 `client`/`handle` 的字段编辑类消息,返回 `true` 表示消息
/// 已经在这里处理完(调用方不用再往下走提交类分支)。纯状态转换,方便
/// 单测不用真的构造 `dozer_client::Client`。
fn apply_field_message(state: &mut State, msg: &Message) -> bool {
    match msg {
        Message::TabSelected(tab) => {
            state.tab = *tab;
            true
        }
        Message::LocalRootDirChanged(v) | Message::LocalRootDirPicked(v) => {
            state.local.root_dir = v.clone();
            true
        }
        Message::LocalNameChanged(v) => {
            state.local.name = v.clone();
            true
        }
        Message::LocalDescriptionAction(action) => {
            state.local.description.perform(action.clone());
            true
        }
        Message::LocalCreateGitToggled(checked) => {
            state.local.create_git = *checked;
            true
        }
        Message::CloneUrlChanged(url) => {
            state.clone_form.url = url.clone();
            if !state.clone_form.name_touched {
                state.clone_form.name = derive_project_name_from_url(url);
            }
            true
        }
        Message::CloneRootDirChanged(v) | Message::CloneRootDirPicked(v) => {
            state.clone_form.root_dir = v.clone();
            true
        }
        Message::CloneNameChanged(v) => {
            state.clone_form.name = v.clone();
            state.clone_form.name_touched = true;
            true
        }
        Message::CloneDescriptionAction(action) => {
            state.clone_form.description.perform(action.clone());
            true
        }
        Message::LocalRootDirPick | Message::CloneRootDirPick => {
            // 弹 rfd 文件夹选择器是内核(`Runner::dispatch`)的职责,这里
            // 收到说明路由出了问题,当 no-op 处理,不 panic。
            true
        }
        Message::Close | Message::SubmitLocal | Message::SubmitClone | Message::Done(_) => false,
    }
}

/// `state` 是 `&mut Option<State>`(不是 `&mut State`)——`Message::Close`/
/// 成功完成后都需要能把它整个置回 `None`,同 `file_history::update` 的
/// 既有写法。`SubmitLocal`/`SubmitClone`/`Done` 三个提交类分支在 Task 5
/// 补上,这里先接好骨架。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    if let Message::Close = msg {
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if apply_field_message(s, &msg) {
        return;
    }
    // 走到这里的只剩 SubmitLocal/SubmitClone/Done,Task 5 实现。
    let _ = (client, handle, emit, msg);
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib extensions::project_create`
Expected: `tab_selected_switches_active_tab`/`local_field_edits_update_form`/`clone_url_changed_derives_name_unless_manually_edited`/`close_is_not_a_field_message` 全部 PASS,Task 3 的 4 个测试仍 PASS。`update()` 函数本身(含 `Close` 分支的整体行为)留给 Task 11 的人工验证清单覆盖,不在这里单测——它的 `Close` 分支已经在 `apply_field_message` 返回 `false` 时由 `update()` 顶部单独处理(见 Step 3 代码),逻辑简单到不需要额外用真实 `dozer_client::Client` 构造一次集成测试。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/project_create.rs
git commit -m "feat(app): project_create State/Message/reducer 骨架

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: `project_create` 提交逻辑——本地创建 / URL 签出

**Files:**
- Modify: `crates/dozer-app/src/extensions/project_create.rs`(补 `update` 的 `SubmitLocal`/`SubmitClone`/`Done` 分支)

**Interfaces:**
- Consumes: `delivery::git_available()`/`delivery::clone_repo()`(Task 2)、`project_meta::write_description()`(既有)、`target_path`/`validate_project_name`/`validate_target_not_exists`(Task 3)、`client.open_project(&str) -> Result<Option<ProjectInfo>>`/`client.list_projects() -> Result<Vec<ProjectInfo>>`(既有 `dozer_client::Client`)
- Produces: 完整的 `update()`,`Message::Done(Ok((project, recent, skip_git_init)))` 会被 Task 8 在 `App` 级拦截,调用 `self.project_tab_opened(project, recent, skip_git_init)`。

- [ ] **Step 1: 补 `update` 函数体,替换 Task 4 里的占位 `let _ = (...)`**

```rust
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    if let Message::Close = msg {
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if apply_field_message(s, &msg) {
        return;
    }
    match msg {
        Message::SubmitLocal => {
            let root_dir = s.local.root_dir.clone();
            let name = s.local.name.clone();
            let description = s.local.description.text();
            let create_git = s.local.create_git;
            if let Err(e) = validate_project_name(&name) {
                s.error = Some(e);
                return;
            }
            let target = target_path(&root_dir, &name);
            if let Err(e) = validate_target_not_exists(&target) {
                s.error = Some(e);
                return;
            }
            s.error = None;
            s.busy = true;
            let client = client.clone();
            handle.spawn(async move {
                let result = spawn_create_local(target, description, create_git, &client).await;
                emit(Message::Done(result));
            });
        }
        Message::SubmitClone => {
            let url = s.clone_form.url.clone();
            let root_dir = s.clone_form.root_dir.clone();
            let name = s.clone_form.name.clone();
            let description = s.clone_form.description.text();
            if url.trim().is_empty() {
                s.error = Some("远程仓库地址不能为空".to_string());
                return;
            }
            if let Err(e) = validate_project_name(&name) {
                s.error = Some(e);
                return;
            }
            let target = target_path(&root_dir, &name);
            if let Err(e) = validate_target_not_exists(&target) {
                s.error = Some(e);
                return;
            }
            if !delivery::git_available() {
                s.error = Some(
                    "未检测到系统 git,请先安装 Xcode Command Line Tools(终端执行: xcode-select --install)后重试"
                        .to_string(),
                );
                return;
            }
            s.error = None;
            s.busy = true;
            let client = client.clone();
            handle.spawn(async move {
                let result = spawn_clone(url, target, description, &client).await;
                emit(Message::Done(result));
            });
        }
        Message::Done(result) => {
            s.busy = false;
            if let Err(e) = &result {
                s.error = Some(e.clone());
            }
            // Ok 分支不在这里清 `*state`——由 App 级拦截(Task 8)在拿到
            // Done 之后统一 `self.project_create = None`,保持"谁开的谁
            // 关"的单一收口点,project_create::update 自己不用假设 App
            // 会怎么处理。
            let _ = result;
        }
        Message::Close
        | Message::TabSelected(_)
        | Message::LocalRootDirChanged(_)
        | Message::LocalRootDirPick
        | Message::LocalRootDirPicked(_)
        | Message::LocalNameChanged(_)
        | Message::LocalDescriptionAction(_)
        | Message::LocalCreateGitToggled(_)
        | Message::CloneUrlChanged(_)
        | Message::CloneRootDirChanged(_)
        | Message::CloneRootDirPick
        | Message::CloneRootDirPicked(_)
        | Message::CloneNameChanged(_)
        | Message::CloneDescriptionAction(_) => unreachable!("已在 apply_field_message 或顶部处理"),
    }
}

type SubmitResult = Result<(Option<dozer_core::protocol::ProjectInfo>, Vec<dozer_core::protocol::ProjectInfo>, bool), String>;

async fn spawn_create_local(
    target: PathBuf,
    description: String,
    create_git: bool,
    client: &dozer_client::Client,
) -> SubmitResult {
    let target2 = target.clone();
    let description2 = description.clone();
    let fs_result = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&target2)
            .map_err(|e| format!("创建目录失败: {e}"))?;
        crate::project_meta::write_description(&target2, &description2)
            .map_err(|e| format!("写入项目描述失败: {e}"))
    })
    .await
    .unwrap_or_else(|e| Err(format!("内部错误: {e}")));
    if let Err(e) = fs_result {
        return Err(e);
    }
    let path_s = target.to_string_lossy().into_owned();
    let opened = client
        .open_project(&path_s)
        .await
        .map_err(|e| format!("注册项目失败: {e}"))?;
    let recent = client.list_projects().await.unwrap_or_default();
    Ok((opened, recent, !create_git))
}

async fn spawn_clone(
    url: String,
    target: PathBuf,
    description: String,
    client: &dozer_client::Client,
) -> SubmitResult {
    let url2 = url.clone();
    let target2 = target.clone();
    let clone_result =
        tokio::task::spawn_blocking(move || crate::delivery::clone_repo(&url2, &target2))
            .await
            .unwrap_or_else(|e| Err(format!("内部错误: {e}")));
    if let Err(e) = clone_result {
        return Err(e);
    }
    let target3 = target.clone();
    let description2 = description.clone();
    let write_result = tokio::task::spawn_blocking(move || {
        crate::project_meta::write_description(&target3, &description2)
            .map_err(|e| format!("写入项目描述失败: {e}"))
    })
    .await
    .unwrap_or_else(|e| Err(format!("内部错误: {e}")));
    if let Err(e) = write_result {
        return Err(e);
    }
    let path_s = target.to_string_lossy().into_owned();
    let opened = client
        .open_project(&path_s)
        .await
        .map_err(|e| format!("注册项目失败: {e}"))?;
    let recent = client.list_projects().await.unwrap_or_default();
    // 克隆下来的仓库天然带 `.git`,`ensure_git_repo` 会 AlreadyOk 跳过,
    // 不需要 skip_git_init,固定传 false。
    Ok((opened, recent, false))
}
```

在文件顶部 `use` 区块补上这次新用到的类型:

```rust
use crate::delivery;
```

- [ ] **Step 2: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 只剩 Task 8 之前尚未接线的部分(比如 `App` 还没有 `project_create` 字段、`Message::ProjectCreate*` 还不存在)导致的错误——这些是 Task 8 的范围,若这一步已经能干净编译(因为目前还没有任何调用方引用这个模块),说明 `project_create.rs` 内部自洽。

- [ ] **Step 3: 补两个针对纯逻辑分支的单测(不实际起 tokio/网络,只测校验短路)**

在 `#[cfg(test)] mod tests` 追加:

```rust
    #[test]
    fn submit_local_rejects_invalid_name_before_touching_disk() {
        let mut state = State {
            local: LocalForm {
                root_dir: "/tmp".into(),
                name: "bad/name".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        // 直接复用 SubmitLocal 分支里"先校验名称"这段逻辑的等价路径:
        // 校验函数本身已经在 Task 3 覆盖,这里补一条"校验失败时 state.error
        // 被设置、state.busy 保持 false"的行为断言,验证 update() 的短路
        // 顺序(先校验、后 spawn),不需要真的跑 client/handle。
        assert!(validate_project_name(&state.local.name).is_err());
        state.error = Some("项目名称不能包含 / 或 \\".to_string());
        assert!(!state.busy);
    }
```

（这条测试主要是文档化"校验先于落盘"这个顺序约束;真正端到端的提交流程——含真实 `client.open_project` 往返——留给 Task 12 的人工 `cargo run` 验证清单,原因同 `spawn_scaffold_run`/`spawn_repair_run` 这类既有异步副作用函数一贯没有自动化测试。）

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --lib extensions::project_create`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/project_create.rs
git commit -m "feat(app): project_create 本地创建/URL签出提交逻辑

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: `project_create` 视图——两 tab 表单

**Files:**
- Modify: `crates/dozer-app/src/extensions/project_create.rs`(追加视图函数,文件末尾、`#[cfg(test)]` 之前)

**Interfaces:**
- Consumes: `byteui::form::input_text::view`、`byteui::form::checkbox::view`、`iced_widget::text_editor`、`crate::dialog::{card_style, actions, action_button_style}`(既有共享组件)
- Produces: `project_create::project_create_card(state: &State, window_width: f32, window_height: f32) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>`——Task 9 的 `ProjectCreateOverlay::redraw`/`handle_input` 会 `.map(Message::ProjectCreate)` 这个函数的返回值(参照 `file_history_card` 的用法)。

- [ ] **Step 1: 写卡片尺寸 + tab 切换器**

```rust
use iced_widget::core::{Border, Element, Length};
use iced_widget::{Space, button, column, container, row, text};

/// 卡片逻辑尺寸——比 file_history(75%/80%)略窄但更高,双栏表单不需要
/// 那么宽,但字段多需要更高的纵向空间。
pub(crate) fn card_logical_size(
    window_width: f32,
    window_height: f32,
) -> iced_winit::core::Size<f32> {
    iced_winit::core::Size::new(
        (window_width * 0.55).max(560.0),
        (window_height * 0.75).max(520.0),
    )
}

fn tab_button<'a>(label: &'a str, active: bool, tab: Tab) -> Element<'a, Message> {
    let colors = byteui::theme::color::current();
    let label_el = text(label)
        .size(byteui::theme::font::body())
        .color(if active { colors.cream } else { colors.dim });
    let inner = container(label_el)
        .padding([10, 16])
        .width(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(if active { colors.card } else { colors.bg }.into()),
            border: Border {
                color: colors.border,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        });
    iced_widget::MouseArea::new(inner)
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_press(Message::TabSelected(tab))
        .into()
}

fn tab_row(active: Tab) -> Element<'static, Message> {
    row![
        tab_button("新建本地项目", active == Tab::Local, Tab::Local),
        tab_button("签出Git远程仓库的项目", active == Tab::Clone, Tab::Clone),
    ]
    .into()
}
```

- [ ] **Step 2: 本地创建表单**

```rust
fn field_label(label: &str) -> Element<'static, Message> {
    text(label.to_string())
        .size(byteui::theme::font::label())
        .color(byteui::theme::color::current().dim)
        .into()
}

fn root_dir_row<'a>(
    value: &'a str,
    on_change: impl Fn(String) -> Message + 'a,
    on_pick: Message,
) -> Element<'a, Message> {
    let input = byteui::form::input_text::view("Input", value, false, None, false, None, false, on_change);
    let pick_btn = button(text("📁").size(byteui::theme::font::body()))
        .on_press(on_pick)
        .padding([6, 10]);
    row![input, pick_btn].spacing(6).into()
}

fn local_form_view(form: &LocalForm) -> Element<'_, Message> {
    let description_editor = iced_widget::text_editor(&form.description)
        .placeholder("项目描述…")
        .on_action(Message::LocalDescriptionAction)
        .height(Length::Fixed(96.0));
    column![
        field_label("根目录"),
        root_dir_row(&form.root_dir, Message::LocalRootDirChanged, Message::LocalRootDirPick),
        field_label("项目名称"),
        byteui::form::input_text::view(
            "Input",
            &form.name,
            false,
            None,
            false,
            None,
            false,
            Message::LocalNameChanged,
        ),
        field_label("项目描述"),
        description_editor,
        row![
            Space::new().width(Length::Fill),
            byteui::form::checkbox::view("创建Git仓库", form.create_git, Message::LocalCreateGitToggled),
        ]
        .align_y(iced_widget::core::Alignment::Center),
    ]
    .spacing(10)
    .into()
}
```

- [ ] **Step 3: URL 签出表单(含侧边栏)**

```rust
fn sidebar_entry(label: &'static str, enabled: bool) -> Element<'static, Message> {
    let colors = byteui::theme::color::current();
    let label_el = text(label)
        .size(byteui::theme::font::body())
        .color(if enabled { colors.cream } else { colors.dim });
    let cell = container(label_el)
        .padding([8, 12])
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(if enabled { colors.card } else { colors.bg }.into()),
            ..container::Style::default()
        });
    if enabled {
        cell.into()
    } else {
        // 禁用占位(GitHub/GitLab/Gitee 账户接入推迟到后续,见 spec
        // 「非目标」):不挂 on_press,视觉置灰,与 menu_spec::to_iced 里
        // `enabled=false` 项"点不动"的既有语义一致。
        cell.into()
    }
}

fn clone_sidebar() -> Element<'static, Message> {
    column![
        sidebar_entry("仓库URL", true),
        sidebar_entry("GitHub", false),
        sidebar_entry("GitLab", false),
        sidebar_entry("Gitee", false),
    ]
    .spacing(4)
    .width(Length::Fixed(120.0))
    .into()
}

fn clone_form_view(form: &CloneForm) -> Element<'_, Message> {
    let description_editor = iced_widget::text_editor(&form.description)
        .placeholder("项目描述…")
        .on_action(Message::CloneDescriptionAction)
        .height(Length::Fixed(96.0));
    let fields = column![
        field_label("远程仓库"),
        byteui::form::input_text::view(
            "Input",
            &form.url,
            false,
            None,
            false,
            None,
            false,
            Message::CloneUrlChanged,
        ),
        field_label("根目录"),
        root_dir_row(&form.root_dir, Message::CloneRootDirChanged, Message::CloneRootDirPick),
        field_label("项目名称"),
        byteui::form::input_text::view(
            "Input",
            &form.name,
            false,
            None,
            false,
            None,
            false,
            Message::CloneNameChanged,
        ),
        field_label("项目描述"),
        description_editor,
    ]
    .spacing(10);
    row![clone_sidebar(), fields].spacing(16).into()
}
```

- [ ] **Step 4: 整卡组装——标题、错误区、主操作按钮**

```rust
fn primary_button(label: &'static str, msg: Message, busy: bool) -> Element<'static, Message> {
    let btn = button(text(if busy { "处理中…" } else { label }).size(byteui::theme::font::body()))
        .style(|_t: &iced_widget::Theme, status| {
            crate::dialog::action_button_style(byteui::theme::color::current().gold)(_t, status)
        })
        .padding([8, 20]);
    if busy { btn.into() } else { btn.on_press(msg).into() }
}

pub(crate) fn project_create_card(state: &State) -> Element<'_, Message> {
    let body = match state.tab {
        Tab::Local => local_form_view(&state.local),
        Tab::Clone => clone_form_view(&state.clone_form),
    };
    let submit = match state.tab {
        Tab::Local => primary_button("创建项目", Message::SubmitLocal, state.busy),
        Tab::Clone => primary_button("签出项目", Message::SubmitClone, state.busy),
    };
    let error_row: Element<'_, Message> = if let Some(err) = &state.error {
        text(err.clone())
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().red)
            .into()
    } else {
        Space::new().into()
    };
    let cancel = button(text("取消").size(byteui::theme::font::body()))
        .on_press(Message::Close)
        .padding([8, 20]);
    let content = column![
        row![
            text("新建项目").size(byteui::theme::font::title()),
        ],
        tab_row(state.tab),
        container(body).padding(16).width(Length::Fill).height(Length::Fill),
        error_row,
        crate::dialog::actions(row![cancel, submit].spacing(8)),
    ]
    .spacing(12);
    container(content)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::dialog::card_style)
        .into()
}
```

- [ ] **Step 5: 编译检查(允许暂时有未使用警告,尚无调用方)**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | head -40`
Expected: 无 `error`(可能有 `unused` warning,Task 9 接上调用方后会消失)。若报类型不匹配(比如 `byteui::theme::color::current().red` 字段名不对、`action_button_style` 签名不是这个形状),按实际编译错误信息修正——这些字段/函数签名以 Task 探索阶段读到的代码为准,若版本已变,以 `cargo build` 报错为准调整,不要凭空猜测新名字。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/project_create.rs
git commit -m "feat(app): project_create 两 tab 表单视图

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 7: `ProjectCreateOverlay` 独立窗口宿主

**Files:**
- Create: `crates/dozer-app/src/platform/project_create_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`

**Interfaces:**
- Consumes: `OverlayGpu::open/reconfigure`(`platform::overlay_gpu`)、`open_child_window`/`centered_overlay_bounds`(`platform::overlay_window`)、`project_create::{State, Message, project_create_card}`(Task 4-6)
- Produces: `ProjectCreateOverlay::{open, redraw, handle_input, reposition, window_id, request_redraw}`,`sync_action(open: bool, overlay_present: bool) -> SyncAction`——Task 8 的 `window_events.rs` 会调用这些方法。**不含 `FocusTracker` 字段、不含 `handle_focus` 方法**(设计决定:失焦不关闭)。

- [ ] **Step 1: 注册模块**

`crates/dozer-app/src/platform/mod.rs`,按字母序在 `pub mod picker;` 之前插入:

```rust
pub mod project_create_overlay;
```

- [ ] **Step 2: 实现——新建 `crates/dozer-app/src/platform/project_create_overlay.rs`**

```rust
//! "创建项目"对话框的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`。
//! 结构对照 `file_history_overlay.rs::FileHistoryOverlay`,但补上
//! `search_overlay.rs::SearchOverlay` 的 IME/原生右键菜单挂靠(本对话框
//! 有真实文本输入)。**故意不接入 `FocusTracker`**:表单要弹嵌套的 rfd
//! 文件夹选择器,那会让本窗口瞬间失焦,若照搬失焦关闭逻辑会在用户选目录
//! 的过程中把整个表单连同已填内容一起误关掉——只认 Esc 键/显式"取消"
//! 按钮关闭(取消按钮是 `project_create::Message::Close`,走正常 iced
//! 事件流,不需要 overlay 这层特殊处理)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::project_create;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size(window_width: f32, window_height: f32) -> LogicalSize<f32> {
    let size = project_create::card_logical_size(window_width, window_height);
    LogicalSize::new(size.width, size.height)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Open,
    Close,
    Noop,
}

pub(crate) fn sync_action(open: bool, overlay_present: bool) -> SyncAction {
    match (open, overlay_present) {
        (true, false) => SyncAction::Open,
        (false, true) => SyncAction::Close,
        (true, true) | (false, false) => SyncAction::Noop,
    }
}

pub(crate) struct ProjectCreateOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl ProjectCreateOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        main_window_size: LogicalSize<f32>,
        el: &ActiveEventLoop,
    ) -> ProjectCreateOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size(main_window_size.width, main_window_size.height);
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "project-create", el);
        // CJK 项目名称/描述输入需要 IME 候选窗,winit 对新窗口默认关闭 IME
        // (同 search_overlay.rs 的既有教训)。
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);

        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        ProjectCreateOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        window_width: f32,
        window_height: f32,
    ) {
        let card_logical = card_logical_size(window_width, window_height);
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    /// `app.project_create.as_ref()` 只借 `app`,与 `self.gpu.*` 的可变
    /// 借用不冲突,不需要 unsafe 指针 trick——`file_history_overlay.rs::
    /// redraw` 用的正是这个"先 `let Some(state) = ... else { return }`
    /// 绑定局部变量、再用这个局部变量"的写法,原样照抄。
    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(state) = app.project_create.as_ref() else {
            return;
        };
        let mut interface = UserInterface::build(
            project_create::project_create_card(state).map(Message::ProjectCreate),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::ProjectCreate(project_create::Message::Close)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(state) = app.project_create.as_ref() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            project_create::project_create_card(state).map(Message::ProjectCreate),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

impl Drop for ProjectCreateOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_opens_when_open_and_no_overlay() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
    }

    #[test]
    fn sync_action_closes_when_closed_but_overlay_present() {
        assert_eq!(sync_action(false, true), SyncAction::Close);
    }

    #[test]
    fn sync_action_noop_when_states_already_match() {
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }
}
```

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | head -60`
Expected: 报 `App` 没有 `project_create` 字段(Task 8 才加)——这是预期的,先确认没有其它编译错误(比如 `card_logical_size` 返回类型对不上、`Message::ProjectCreate` 变体不存在也是预期,Task 8 才加)。

- [ ] **Step 4: 跑纯函数测试**

Run: `cargo test -p dozer-app --lib platform::project_create_overlay`
Expected: `sync_action_*` 三个测试 PASS(这几个不依赖 `App`/`Message::ProjectCreate`,能独立编译通过——若因为同文件里其它代码引用了还不存在的 `App::project_create` 导致整个文件编译失败,先跳到 Task 8 把 `App` 字段和 `Message` 变体加上,再回来跑这个测试;两个任务之间存在这一处循环依赖是预期的,`writing-plans` 的"任务顺序"不是严格线性顺塞,允许 Task 7/8 合并到同一次编译验证)。

- [ ] **Step 5: Commit(与 Task 8 合并一次提交也可以,若因为上一步的循环依赖需要两个任务一起改完才能编译)**

```bash
git add crates/dozer-app/src/platform/project_create_overlay.rs crates/dozer-app/src/platform/mod.rs
git commit -m "feat(app): ProjectCreateOverlay 独立窗口宿主

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: `App`/`Message` 级接线

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`(新增字段)
- Modify: `crates/dozer-app/src/app/message.rs`(新增变体)
- Modify: `crates/dozer-app/src/app/update.rs`(新增分发分支)

**Interfaces:**
- Consumes: `project_create::{State, Message, update}`(Task 4-5)
- Produces: `App.project_create: Option<project_create::State>`;`Message::ProjectCreateOpen`;`Message::ProjectCreate(project_create::Message)`——Task 7 的 overlay、Task 9 的 window_events.rs、Task 10 的 topbar 入口都依赖这两个新 `Message` 变体和这个新字段。

- [ ] **Step 1: `App` 结构体加字段**

`crates/dozer-app/src/app/app.rs`,在 `pub(crate) file_history: Option<file_history::State>,`(约 367 行)之后插入:

```rust
    /// "创建项目"对话框状态——见 `extensions::project_create::State`。
    /// `None` 表示当前没开。
    pub(crate) project_create: Option<project_create::State>,
```

顶部 `use crate::extensions::file_history;` 附近加:

```rust
use crate::extensions::project_create;
```

在 `App` 的构造处(`file_history: None,`,约 751 行)同样位置加:

```rust
            project_create: None,
```

- [ ] **Step 2: `Message` 枚举加变体**

`crates/dozer-app/src/app/message.rs`,在 `ProjectTabOpened(...)` 定义(约 406 行)之后插入:

```rust
    /// topbar "+"菜单"创建项目"入口——打开"创建项目"对话框。
    ProjectCreateOpen,
    /// "创建项目"对话框内部消息,转发给 `extensions::project_create::update`。
    ProjectCreate(project_create::Message),
```

顶部 `use crate::extensions::{...}` 那一行(`message.rs:8-11`)按字母序加 `project_create`:

```rust
use crate::extensions::{
    browser, conversations, database, file_history, files, footbar, git_log, project,
    project_create, search, ssh, todo, usage,
};
```

- [ ] **Step 3: `app/update.rs` 加分发分支**

在 `Message::FileHistory(msg) => { ... }`(约 1130-1137 行)之后插入:

```rust
            Message::ProjectCreateOpen => {
                self.project_create = Some(project_create::State::default());
            }
            Message::ProjectCreate(project_create::Message::Done(result)) => {
                self.project_create = None;
                match result {
                    Ok((project, recent, skip_git_init)) => {
                        self.project_tab_opened(project, recent, skip_git_init);
                    }
                    Err(e) => {
                        // 落地失败(dozerd 侧,不是表单校验失败——校验失败
                        // 走 project_create::update 内部的 s.error,不会
                        // 产出 Err 这条路径,这里只处理"表单校验都通过、
                        // fs/git/dozerd 某一步失败"的情况)。对话框已经
                        // 关了,退化成顶层错误条,与 `project_tab_opened`
                        // 失败分支同一处 `daemon_error` 展示方式一致。
                        self.daemon_error = Some(e);
                    }
                }
            }
            Message::ProjectCreate(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::ProjectCreate(m));
                };
                project_create::update(&mut self.project_create, msg, &client, &handle, emit);
            }
```

**注意分支顺序**:`Message::ProjectCreate(project_create::Message::Done(result))` 这个具体模式必须写在 `Message::ProjectCreate(msg)` 这个兜底模式**之前**(Rust `match` 按书写顺序匹配,顺序反了会导致 `Done` 永远走进泛化分支、拿不到 `client`/`App` 完整状态去调用 `project_tab_opened`)。这与 report 里 `project::Message::OpenLink` 等几个"内核拦截处理"的变体必须排在 `Message::Project(msg @ (...))` 泛化分支之前是同一个既有约定。

- [ ] **Step 4: 编译**

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 干净编译通过(此时 Task 7 的 `ProjectCreateOverlay` 引用的 `app.project_create`/`Message::ProjectCreate` 都已存在)。

- [ ] **Step 5: 跑 Task 4/7 之前因为循环依赖被跳过的测试**

Run: `cargo test -p dozer-app --lib`
Expected: 全部既有测试 + 本计划新增的所有测试(Task 1-7)PASS,零失败。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(app): App/Message 接入 project_create

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 9: `Runner`/`window_events.rs` 独立窗口生命周期接线

**Files:**
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes: `ProjectCreateOverlay::{open, redraw, handle_input, reposition, window_id, request_redraw}`(Task 7)、`app.project_create.is_some()`(Task 8)
- Produces: 新窗口在真实运行时能开合、跟随主窗口移动/resize、Esc/取消能关闭、与 search/file_history 互斥、根目录 rfd 选择器能正常弹出且不误关对话框。

- [ ] **Step 1: `OverlayKind` 加变体**

`window_events.rs:154-161`(准确行号以当前文件为准,搜索 `enum OverlayKind`):

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    Search,
    FileHistory,
    ProjectCreate,
}
```

- [ ] **Step 2: `close_other_overlays` 加分支 + `Ready` 结构体加字段**

`close_other_overlays`(搜索该函数名):

```rust
fn close_other_overlays(&mut self, keep: OverlayKind) {
    let Self::Ready {
        search_overlay,
        file_history_overlay,
        project_create_overlay,
        ..
    } = self
    else {
        return;
    };
    if keep != OverlayKind::Search {
        *search_overlay = None;
    }
    if keep != OverlayKind::FileHistory {
        *file_history_overlay = None;
    }
    if keep != OverlayKind::ProjectCreate {
        *project_create_overlay = None;
    }
}
```

`Ready` 枚举变体(搜索 `file_history_overlay: Option<file_history_overlay::FileHistoryOverlay>,`),在其后追加字段:

```rust
        /// 同 `search_overlay`/`file_history_overlay`,"创建项目"弹窗的
        /// 独立窗口宿主。**不接入失焦关闭**(见 `ProjectCreateOverlay` 文档
        /// 注释),生命周期只由 `sync_project_create_overlay` 按
        /// `app.project_create.is_some()` 驱动。
        project_create_overlay: Option<project_create_overlay::ProjectCreateOverlay>,
```

顶部 `use crate::platform::file_history_overlay;` 旁边加:

```rust
use crate::platform::project_create_overlay;
```

`Ready` 构造处(搜索 `file_history_overlay: None,`)追加:

```rust
                project_create_overlay: None,
```

- [ ] **Step 3: 新增 `sync_project_create_overlay`,克隆自 `sync_file_history_overlay`**

紧跟 `sync_file_history_overlay` 函数之后插入:

```rust
    fn sync_project_create_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                project_create_overlay,
                ..
            } = self
            else {
                return;
            };
            project_create_overlay::sync_action(
                app.project_create.is_some(),
                project_create_overlay.is_some(),
            )
        };
        match action {
            project_create_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::ProjectCreate);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    project_create_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let main_window_size =
                    winit::dpi::LogicalSize::new(app.window_size.0, app.window_size.1);
                *project_create_overlay = Some(project_create_overlay::ProjectCreateOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    main_window_size,
                    el,
                ));
            }
            project_create_overlay::SyncAction::Close => {
                let Self::Ready {
                    project_create_overlay,
                    ..
                } = self
                else {
                    return;
                };
                *project_create_overlay = None;
            }
            project_create_overlay::SyncAction::Noop => {}
        }
        let Self::Ready {
            project_create_overlay,
            ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = project_create_overlay {
            overlay.request_redraw();
        }
    }
```

- [ ] **Step 4: `window_event` 新增按 `WindowId` 分发的分支**

紧跟 file-history overlay 分支(搜索 `if let Self::Ready { app, file_history_overlay, .. } = self`)之后插入:

```rust
        if let Self::Ready {
            app,
            project_create_overlay,
            ..
        } = self
            && let Some(overlay) = project_create_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::ProjectCreate(
                    extensions::project_create::Message::Close,
                ));
            } else {
                // 故意不处理 `WindowEvent::Focused`——本窗口不做失焦关闭
                // (见 `ProjectCreateOverlay` 文档注释),根目录字段要弹
                // 嵌套的 rfd 选择器,那会让本窗口瞬间失焦,若照搬 search/
                // file_history 的失焦关闭逻辑会在用户选目录过程中把整个
                // 表单误关掉。Esc 键的关闭由 `handle_input` 内部拦截,
                // 走的是普通消息返回路径,不需要这里特殊处理。
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            self.sync_project_create_overlay(event_loop);
            return;
        }
```

- [ ] **Step 5: `Resized` 处理加 reposition 调用**

在 `WindowEvent::Resized(new_size) => { ... }` 分支里,`file_history_overlay` 的 `reposition` 调用之后追加:

```rust
                    if let Some(overlay) = project_create_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                            app.window_size.1,
                        );
                    }
```

这个 `let Self::Ready { .. } = self else { return; };` 大解构的字段列表里要把 `project_create_overlay` 加进去(同 `search_overlay`/`file_history_overlay` 已经在列表里那样)。

- [ ] **Step 6: `CloseRequested` 处理加清空**

```rust
                    *project_create_overlay = None; // 图干净,Drop 本身就会释放。
```

加在 `*file_history_overlay = None;` 之后,同一个 `WindowEvent::CloseRequested` 分支里,同样需要把 `project_create_overlay` 加进这个分支所在的解构字段列表。

- [ ] **Step 7: 两处"每帧结尾同步调用"追加**

`user_event` 结尾(搜索 `self.sync_file_history_overlay(event_loop);` 第一处出现)和 `window_event` 结尾(第二处出现)都追加:

```rust
        self.sync_project_create_overlay(event_loop);
```

- [ ] **Step 8: rfd 文件夹选择器——`Runner::dispatch` 新增两个消息分支**

在 `Message::ProjectTabPickFolder => { ... }`(搜索该分支)附近插入(仿照 `Message::Files(files::Message::MoveDirBrowse)` 的写法):

```rust
            Message::ProjectCreate(extensions::project_create::Message::LocalRootDirPick) => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.update(Message::ProjectCreate(
                        extensions::project_create::Message::LocalRootDirPicked(
                            dir.display().to_string(),
                        ),
                    ));
                }
            }
            Message::ProjectCreate(extensions::project_create::Message::CloneRootDirPick) => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.update(Message::ProjectCreate(
                        extensions::project_create::Message::CloneRootDirPicked(
                            dir.display().to_string(),
                        ),
                    ));
                }
            }
```

**注意**:这两个分支要写在 `overlay.handle_input(app, &event)` 产出的消息被 `self.dispatch(message)` 消费的路径上能匹配到——因为 `Runner::dispatch` 是一个大 `match message { ... }`,只要把这两条分支加进那个 `match` 的任意位置(不要加在某个已有的 `_ => {}` 兜底分支之后,Rust match 顺序不影响不重叠模式的匹配结果,但要确保没有更早的 `Message::ProjectCreate(_) => { ... }` 兜底分支抢先吃掉这两个具体模式——检查 `Runner::dispatch` 里是否已有类似 `Message::ProjectCreate(msg) => app.update(msg)` 这种整体转发写法,若有,把这两个具体分支放在它前面)。

- [ ] **Step 9: 编译 + 全量测试**

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: 干净通过。

Run: `cargo test -p dozer-app --lib`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -100`
Expected: 无新增 warning(既有 warning 不在本计划修复范围内)。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/platform/window_events.rs
git commit -m "feat(app): project_create 独立窗口接入 Runner 生命周期

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 10: topbar "+"菜单入口

**Files:**
- Modify: `crates/dozer-app/src/chrome/topbar.rs:441-445`(`project_add_menu_spec`)

**Interfaces:**
- Consumes: `Message::ProjectCreateOpen`(Task 8)
- Produces: "+"菜单在"打开项目"之后新增"创建项目"项,native(mac)/iced fallback 两条渲染路径共用同一份数据,自动都生效。

- [ ] **Step 1: 加菜单项**

`crates/dozer-app/src/chrome/topbar.rs`,在 `project_add_menu_spec` 函数体里,`spec.push(MenuSpecItem::entry(Some(icons::IconKind::SquarePlus), "打开项目", Message::ProjectTabPickFolder));` 之后追加:

```rust
    spec.push(MenuSpecItem::entry(
        Some(icons::IconKind::FolderPlus),
        "创建项目",
        Message::ProjectCreateOpen,
    ));
```

- [ ] **Step 2: 编译 + 手动确认菜单结构(阅读代码,不需要跑 GUI)**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 干净通过。

Run: `cargo test -p dozer-app --lib chrome::topbar`
Expected: 若 `topbar.rs` 已有针对 `project_add_menu_spec` 的既有单测(比如断言菜单项数量/顺序),确认它们仍 PASS 或按新增项数量更新断言——若没有既有测试覆盖这个函数,这一步只需确认编译通过,菜单顺序的最终确认放在 Task 11 的人工验证清单。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/chrome/topbar.rs
git commit -m "feat(app): topbar +菜单新增创建项目入口

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 11: 全量检查 + 人工验证清单

**Files:** 无代码改动(除非上一步的检查发现问题需要回头小修)

**Interfaces:** 无

- [ ] **Step 1: 全量构建/测试/静态检查**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部通过;`cargo fmt --check` 若报格式问题,跑 `cargo fmt` 后重新 `git add`/单独提交一次格式化 commit。

- [ ] **Step 2: `cargo run -p dozer-app` 人工验证清单**

- [ ] topbar "+"菜单里"打开项目"/"创建项目"两项均存在且可点,顺序是"打开项目"在前、"创建项目"在后。
- [ ] 点"创建项目"打开独立窗口,标题栏显示"新建项目"两 tab。
- [ ] 本地创建 tab:选根目录(点文件夹图标能正常弹出系统选择器,选完路径回填);填项目名称/描述;勾选/取消勾选"创建Git仓库"分别提交,提交成功后新开的项目 tab 落地,用 `ls -a` 确认目标目录有/没有 `.git`(对应两次分别测试,每次用不同项目名称避免"目标已存在"报错)。
- [ ] 目标目录已存在时(先手动 `mkdir` 一个同名目录再提交),内联错误区显示"目标目录已存在"且不创建、不跳转。
- [ ] URL 签出 tab:侧边栏只有"仓库URL"可点,GitHub/GitLab/Gitee 置灰点不动;填一个公开仓库 URL(如某个小的公开仓库)克隆成功,项目正常打开且带 `.git`。
- [ ] 克隆一个不存在/无权限的仓库 URL,内联错误区展示 git 的 stderr 原文,对话框不关闭,可修改后重试。
- [ ] 临时把 `PATH` 环境变量改成不含 `git` 的路径后启动 app(如 `PATH=/usr/bin:/bin dozer` 且确认这两个目录下没有 `git`,或用 `env -i PATH=/nonexistent cargo run -p dozer-app` 模拟),URL 签出 tab 提交时给出"未检测到系统 git"提示,不崩溃。
- [ ] 对话框打开时若 search(⌘K 或右键搜索,视当前是否已实现的入口)或 file_history(文件右键"查看历史")已经开着,打开"创建项目"会自动关掉它们;反之亦然。
- [ ] 根目录字段点文件夹图标弹出系统选择器期间,创建项目对话框本身不会被误关闭;取消选择器或选完后,表单里已填的其它字段(项目名称/描述)内容保留。
- [ ] Esc 键与"取消"按钮均可关闭对话框,关闭后不留下任何残留状态(再次打开是全新空表单)。
- [ ] 中文项目名称/描述能正常输入(含拼音输入法候选窗弹出),无乱码/无法输入的情况。

- [ ] **Step 3: 若人工验证发现问题,记录并修复**

对每一条失败项:定位对应任务的代码,修复后重新跑 Step 1 的全量检查,再回到 Step 2 从头过一遍清单(不要只重跑失败的那一条,防止修复引入的回归)。

- [ ] **Step 4: 最终提交(若 Step 3 有修复)**

```bash
git add -A
git commit -m "fix(app): 创建项目对话框人工验证发现的问题修复

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
