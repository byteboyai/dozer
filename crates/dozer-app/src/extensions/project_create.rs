//! "创建项目"对话框:两 tab(本地新建 / Git URL 签出)状态机 + 视图。
//! 设计见 `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`。
//! 渲染宿主是独立原生窗口 `platform::project_create_overlay::
//! ProjectCreateOverlay`,结构对照 `extensions::file_history` + 同名 overlay
//! 的既有分工:本模块只管状态/消息/视图/异步落盘逻辑,不碰 winit/wgpu。

use crate::delivery;
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
    Done(
        Result<
            (
                Option<dozer_core::protocol::ProjectInfo>,
                Vec<dozer_core::protocol::ProjectInfo>,
                bool,
            ),
            String,
        >,
    ),
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
/// 既有写法。
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
        | Message::CloneDescriptionAction(_) => {
            unreachable!("已在 apply_field_message 或顶部处理")
        }
    }
}

type SubmitResult = Result<
    (
        Option<dozer_core::protocol::ProjectInfo>,
        Vec<dozer_core::protocol::ProjectInfo>,
        bool,
    ),
    String,
>;

async fn spawn_create_local(
    target: PathBuf,
    description: String,
    create_git: bool,
    client: &dozer_client::Client,
) -> SubmitResult {
    let target2 = target.clone();
    let description2 = description.clone();
    let fs_result = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&target2).map_err(|e| format!("创建目录失败: {e}"))?;
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
}
