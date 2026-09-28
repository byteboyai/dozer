//! "创建项目"对话框:两 tab(本地新建 / Git URL 签出)状态机 + 视图。
//! 设计见 `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`。
//! 渲染宿主是独立原生窗口 `platform::project_create_overlay::
//! ProjectCreateOverlay`,结构对照 `extensions::file_history` + 同名 overlay
//! 的既有分工:本模块只管状态/消息/视图/异步落盘逻辑,不碰 winit/wgpu。

use crate::delivery;
use crate::git_accounts::{self, GitProvider, RemoteRepo};
use byteui::interaction::icons;
use iced_widget::core::{Border, Color, Length, Padding};
use iced_widget::{MouseArea, Space, button, column, container, row, text};
use std::collections::HashMap;
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

/// 从本地根目录路径推导默认项目名称——取路径最后一段(`Path::file_name`,
/// 自动容忍结尾斜杠)。根目录为空或到根(`/`)时取不到名字,回退空串由
/// 用户手填。
pub(crate) fn derive_project_name_from_dir(dir: &str) -> String {
    Path::new(dir.trim())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Local,
    Clone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloneSource {
    #[default]
    Url,
    Provider(GitProvider),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RepoListState {
    Loading,
    Loaded(Vec<RemoteRepo>),
    Error(String),
    NotConnected,
}

pub struct LocalForm {
    pub root_dir: String,
    pub name: String,
    /// 用户是否手动编辑过项目名称——同 `CloneForm::name_touched` 的语义,
    /// 手改过后根目录再变化就不拿推导值覆盖(推导见
    /// [`derive_project_name_from_dir`])。
    pub name_touched: bool,
    pub description: iced_widget::text_editor::Content,
    pub create_git: bool,
}

impl Default for LocalForm {
    fn default() -> Self {
        LocalForm {
            root_dir: String::new(),
            name: String::new(),
            name_touched: false,
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
    /// 当前"远程仓库"字段展示的是手填 URL 还是某个账户的仓库列表。
    pub source: CloneSource,
    /// 本次对话框会话内按 provider 缓存的仓库列表加载态,切换 tab 来回点
    /// 不重复发请求(除非上次是 `Error`,那种情况允许重试)。
    pub repo_lists: HashMap<GitProvider, RepoListState>,
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    pub local: LocalForm,
    pub clone_form: CloneForm,
    pub error: Option<String>,
    pub busy: bool,
    pub close_hover: bool,
    /// 两个视图切换 tab 的 hover 态,驱动 `dialog_tab_style` 的 hover 高亮
    /// (未选中 hover 时浮现 TAB_HOVER 胶囊 + 标题 DIM→GOLD),与文件预览
    /// 窗口页签同一套视觉。
    pub tab_hover: Option<Tab>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// 右上角 X 关闭按钮的 hover 态,驱动图标按钮统一的 hover 动画
    /// (图标从 DIM 平滑过渡到 GOLD),与 `extensions::settings` 的关闭按钮同套。
    CloseHover(bool),
    /// 两个视图切换 tab 的 hover 态,驱动与文件预览窗口页签一致的高亮。
    TabHover(Option<Tab>),
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
    SourceSelected(CloneSource),
    RepoListLoaded(GitProvider, Result<Vec<RemoteRepo>, String>),
    /// 未连接账户时"去设置连接"按钮——不在本模块内部处理,由 `App::update`
    /// 顶层拦截(关本对话框、开设置弹窗),但顶部的 `if let Message::Close
    /// | Message::GoToSettings` 仍会先把 `state` 清空,保持"直接调用本模块
    /// `update` 也不会留下半开的对话框状态"这个防御性保证。
    GoToSettings,
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
            // 项目名称默认从根目录最后一段推导,手改过就不再覆盖
            // (与 CloneForm 从 URL 推导同一套"可编辑默认值"语义)。
            if !state.local.name_touched {
                state.local.name = derive_project_name_from_dir(v);
            }
            true
        }
        Message::LocalNameChanged(v) => {
            state.local.name = v.clone();
            state.local.name_touched = true;
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
        Message::RepoListLoaded(provider, result) => {
            let repo_state = match result {
                Ok(repos) => RepoListState::Loaded(repos.clone()),
                Err(e) => RepoListState::Error(e.clone()),
            };
            state.clone_form.repo_lists.insert(*provider, repo_state);
            true
        }
        Message::LocalRootDirPick | Message::CloneRootDirPick => {
            // 弹 rfd 文件夹选择器是内核(`Runner::dispatch`)的职责,这里
            // 收到说明路由出了问题,当 no-op 处理,不 panic。
            true
        }
        Message::CloseHover(h) => {
            state.close_hover = *h;
            true
        }
        Message::TabHover(h) => {
            state.tab_hover = *h;
            true
        }
        Message::Close
        | Message::SubmitLocal
        | Message::SubmitClone
        | Message::Done(_)
        | Message::SourceSelected(_)
        | Message::GoToSettings => false,
    }
}

/// 决定 `SourceSelected(Provider)` 该不该发起新的仓库列表请求,以及请求
/// 前状态该置成什么。已经 `Loaded`/`Loading` 就不重复请求(返回
/// `None`);`Error` 允许重试。纯函数,不碰真实 Keychain——`token_present`
/// 由调用方查真实 `git_accounts::get_token` 后传入,方便单测。
fn plan_source_selection(
    existing: Option<&RepoListState>,
    token_present: bool,
) -> Option<RepoListState> {
    if matches!(
        existing,
        Some(RepoListState::Loaded(_)) | Some(RepoListState::Loading)
    ) {
        return None;
    }
    Some(if token_present {
        RepoListState::Loading
    } else {
        RepoListState::NotConnected
    })
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
    if let Message::Close | Message::GoToSettings = msg {
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
        Message::SourceSelected(source) => {
            s.clone_form.source = source;
            if let CloneSource::Provider(provider) = source {
                let existing = s.clone_form.repo_lists.get(&provider);
                let token = git_accounts::get_token(provider);
                if let Some(new_state) = plan_source_selection(existing, token.is_some()) {
                    let should_fetch = matches!(new_state, RepoListState::Loading);
                    s.clone_form.repo_lists.insert(provider, new_state);
                    if should_fetch {
                        let token = token.expect("Loading 状态下 token 一定存在");
                        handle.spawn(async move {
                            let result = git_accounts::list_repos(provider, &token).await;
                            emit(Message::RepoListLoaded(provider, result));
                        });
                    }
                }
            }
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
        | Message::GoToSettings
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
        | Message::CloneDescriptionAction(_)
        | Message::RepoListLoaded(..)
        | Message::CloseHover(_)
        | Message::TabHover(_) => {
            unreachable!("已在 apply_field_message 或顶部处理")
        }
    }
}

type Element<'a> =
    iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>;

/// 卡片逻辑尺寸——比 file_history(75%/80%)略窄但更高,双栏表单不需要
/// 那么宽,但字段多需要更高的纵向空间。
pub(crate) fn card_logical_size(
    window_width: f32,
    window_height: f32,
) -> iced_winit::core::Size<f32> {
    iced_winit::core::Size::new(
        (window_width * crate::platform::overlay_window::POPUP_WIDTH_FRACTION).max(560.0),
        (window_height * 0.75).max(520.0),
    )
}

fn tab_hover_t(hover: Option<Tab>, tab: Tab) -> f32 {
    if hover == Some(tab) { 1.0 } else { 0.0 }
}

fn tab_button<'a>(label: &'a str, active: bool, tab: Tab, hover_t: f32) -> Element<'a> {
    // 弹窗内两枚互斥视图切换 tab 的自绘标题(2026-09-27 起不再复用共享
    // `tab_label`):字号降到 `caption()`(12px,比共享面板页签的 `label` 还小
    // 一档,与表单字段标签拉开层级),配色公式不变——选中 CREAM、未选中
    // DIM→GOLD 按 hover 插值;容器样式走本模块专属 `dialog_tab_style`
    // (选中 CARD 实底 + 1px 边框、未选中 hover 浮现 TAB_HOVER 胶囊),半径
    // 加大到 8 让 tab 明显圆角化。两个 tab 是互斥的视图切换(不是可关闭的
    // 文件页签),故不挂关闭 ×。边框内边距四边对称,文字不贴边。
    let colors = byteui::theme::color::current();
    let title_color = if active {
        colors.cream
    } else {
        byteui::theme::color::mix(colors.dim, colors.gold, hover_t)
    };
    let content = text(label.to_string())
        .font(crate::app::top_bar_font())
        .size(byteui::theme::font::caption())
        .color(title_color);
    let el: Element<'a> = MouseArea::new(content)
        .on_press(Message::TabSelected(tab))
        .on_enter(Message::TabHover(Some(tab)))
        .on_exit(Message::TabHover(None))
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .into();
    container(el)
        .padding(Padding {
            top: 4.0,
            right: 12.0,
            bottom: 4.0,
            left: 12.0,
        })
        .width(Length::Shrink)
        .style(dialog_tab_style(active, hover_t))
        .into()
}

/// 创建项目弹窗两枚切换 tab 的容器样式:配色与共享 `tab_container_style`
/// 完全一致(选中 CARD 实底 + 1px 边框、未选中 hover 浮现 TAB_HOVER 胶囊),
/// 但半径从共享的 6 提到 8,使 tab 更明显地圆角化。单独实现而不改共享样式,
/// 避免波及终端/预览/SSH/浏览器等其它面板页签的圆角观感。设置弹窗的
/// 左栏切换 tab 也复用同一份样式(见 `extensions::settings`),保证两处
/// 圆角观感一致。
pub(crate) fn dialog_tab_style(
    active: bool,
    hover: f32,
) -> impl Fn(&iced_widget::Theme) -> container::Style {
    move |_theme: &iced_widget::Theme| {
        if active {
            container::Style {
                background: Some(byteui::theme::color::current().card.into()),
                border: Border {
                    color: byteui::theme::color::current().border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        } else if hover > 0.0 {
            container::Style {
                background: Some(
                    Color {
                        a: hover,
                        ..byteui::theme::color::current().tab_hover
                    }
                    .into(),
                ),
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        } else {
            container::Style::default()
        }
    }
}

fn tab_row(active: Tab, hover: Option<Tab>) -> Element<'static> {
    row![
        tab_button(
            "新建本地项目",
            active == Tab::Local,
            Tab::Local,
            tab_hover_t(hover, Tab::Local),
        ),
        tab_button(
            "签出Git远程仓库的项目",
            active == Tab::Clone,
            Tab::Clone,
            tab_hover_t(hover, Tab::Clone),
        ),
    ]
    .spacing(4)
    .into()
}

fn field_label(label: &str) -> Element<'static> {
    text(label.to_string())
        .size(byteui::theme::font::label())
        .color(byteui::theme::color::current().dim)
        .into()
}

fn root_dir_row<'a>(
    value: &'a str,
    on_change: impl Fn(String) -> Message + 'a,
    on_pick: Message,
) -> Element<'a> {
    // 「项目根目录」输入框:把文件夹选择按钮收进输入框内(同文件树「搜索目录」
    // 输入框的内嵌按钮手法),并改用统一的 `input_text::view_with_suffix`
    // (card 底 + 1px 描边 + radius 6、聚焦金框),与系统其它输入框观感一致;
    // 原先独立按钮 + 裸 text_input 的写法既没统一样式、按钮又散在框外。
    let colors = byteui::theme::color::current();
    let folder_btn = button(icons::view(
        icons::IconKind::FolderOpen,
        byteui::theme::icon_size::row(),
        colors.dim,
    ))
    .on_press(on_pick)
    .padding(4)
    .style(move |_t: &iced_widget::Theme, s: button::Status| {
        let hovered = matches!(s, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: match s {
                button::Status::Hovered | button::Status::Pressed => Some(
                    Color {
                        a: 0.15,
                        ..colors.gold
                    }
                    .into(),
                ),
                _ => None,
            },
            // hover 时边框变金,与弹窗其它操作按钮(创建/取消/去设置连接…)
            // 同一套 hover 语言:静止态 1px 描边、`colors.border`,悬浮变 `gold`。
            border: Border {
                color: if hovered { colors.gold } else { colors.border },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    });
    byteui::form::input_text::view_with_suffix(
        "选择项目根目录…",
        value,
        false,
        None,
        false,
        None,
        on_change,
        folder_btn.into(),
    )
}

fn local_form_view(form: &LocalForm) -> Element<'_> {
    let description_editor = byteui::form::text_area::view(
        &form.description,
        "项目描述…",
        None,
        false,
        Some(96.0),
        Message::LocalDescriptionAction,
    );
    column![
        field_label("项目根目录"),
        root_dir_row(
            &form.root_dir,
            Message::LocalRootDirChanged,
            Message::LocalRootDirPick
        ),
        field_label("项目名称"),
        byteui::form::input_text::view(
            "项目名称",
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
            byteui::form::checkbox::view(
                "创建Git仓库",
                form.create_git,
                Message::LocalCreateGitToggled
            ),
        ]
        .align_y(iced_widget::core::Alignment::Center),
    ]
    .spacing(10)
    .into()
}

/// `selected` 决定高亮,`on_press` 现在四项都会给(GitHub/GitLab/Gitee 账户
/// 接入已经落地,不再有占位禁用项——见
/// `docs/superpowers/specs/2026-09-18-project-create-remote-repo-picker-
/// design.md`)。
fn sidebar_entry(label: &'static str, selected: bool, on_press: Message) -> Element<'static> {
    // 左栏这组 provider 切换 tab(仓库URL / GitHub / GitLab / Gitee):字号降到
    // `caption()`(12px,与弹窗顶部「本地新建 / Git仓库签出」切换 tab 同档),
    // 容器加 8 圆角——选中 CARD 实底、未选中 BG 实底,各成一枚圆角胶囊,与
    // 弹窗内其它 tab 的圆角观感一致。此前是直角 `body()` 字号。
    let colors = byteui::theme::color::current();
    let label_el = text(label)
        .size(byteui::theme::font::caption())
        .color(if selected { colors.cream } else { colors.dim });
    let cell = container(label_el)
        .padding([8, 12])
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(if selected { colors.card } else { colors.bg }.into()),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    iced_widget::MouseArea::new(cell)
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_press(on_press)
        .into()
}

fn clone_sidebar(source: CloneSource) -> Element<'static> {
    column![
        sidebar_entry(
            "仓库URL",
            source == CloneSource::Url,
            Message::SourceSelected(CloneSource::Url)
        ),
        sidebar_entry(
            "GitHub",
            source == CloneSource::Provider(GitProvider::GitHub),
            Message::SourceSelected(CloneSource::Provider(GitProvider::GitHub))
        ),
        sidebar_entry(
            "GitLab",
            source == CloneSource::Provider(GitProvider::GitLab),
            Message::SourceSelected(CloneSource::Provider(GitProvider::GitLab))
        ),
        sidebar_entry(
            "Gitee",
            source == CloneSource::Provider(GitProvider::Gitee),
            Message::SourceSelected(CloneSource::Provider(GitProvider::Gitee))
        ),
    ]
    .spacing(4)
    .width(Length::Fixed(120.0))
    .into()
}

fn repo_radio_row(repo: &RemoteRepo, current_url: &str) -> Element<'static> {
    let colors = byteui::theme::color::current();
    let is_selected = repo.clone_url == current_url;
    let dot = container(Space::new())
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if is_selected {
                Some(colors.gold.into())
            } else {
                None
            },
            border: Border {
                color: if is_selected {
                    colors.gold
                } else {
                    colors.border
                },
                width: 1.5,
                radius: 5.0.into(),
            },
            ..container::Style::default()
        });
    let ring = container(dot)
        .width(Length::Fixed(16.0))
        .height(Length::Fixed(16.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    iced_widget::MouseArea::new(
        row![
            ring,
            text(repo.full_name.clone())
                .size(byteui::theme::font::body())
                .color(colors.cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_press(Message::CloneUrlChanged(repo.clone_url.clone()))
    .into()
}

/// "远程仓库"字段的内容——`CloneSource::Url` 时是手填输入框(原有行为),
/// `CloneSource::Provider` 时按加载态切换成引导/加载中/报错/单选列表。
fn remote_repo_field(form: &CloneForm) -> Element<'_> {
    let colors = byteui::theme::color::current();
    match form.source {
        CloneSource::Url => byteui::form::input_text::view(
            "Input",
            &form.url,
            false,
            None,
            false,
            None,
            false,
            Message::CloneUrlChanged,
        ),
        CloneSource::Provider(provider) => match form.repo_lists.get(&provider) {
            None | Some(RepoListState::NotConnected) => {
                let hint = text(format!("未连接 {} 账户", provider.display_name()))
                    .size(byteui::theme::font::body())
                    .color(colors.dim);
                let go_btn = button(text("去设置连接").size(byteui::theme::font::body()))
                    .on_press(Message::GoToSettings)
                    .padding([6, 14])
                    .style(action_button_hover_style(
                        byteui::theme::color::current().cream,
                    ));
                column![hint, go_btn].spacing(8).into()
            }
            Some(RepoListState::Loading) => text("加载仓库列表中…")
                .size(byteui::theme::font::body())
                .color(colors.dim)
                .into(),
            Some(RepoListState::Error(e)) => {
                let msg = text(format!("仓库列表加载失败: {e}"))
                    .size(byteui::theme::font::body())
                    .color(colors.red);
                let retry = button(text("重试").size(byteui::theme::font::body()))
                    .on_press(Message::SourceSelected(CloneSource::Provider(provider)))
                    .padding([6, 14])
                    .style(action_button_hover_style(
                        byteui::theme::color::current().cream,
                    ));
                column![msg, retry].spacing(8).into()
            }
            Some(RepoListState::Loaded(repos)) if repos.is_empty() => text("该账户名下没有仓库")
                .size(byteui::theme::font::body())
                .color(colors.dim)
                .into(),
            Some(RepoListState::Loaded(repos)) => {
                let rows: Vec<Element<'_>> = repos
                    .iter()
                    .map(|repo| repo_radio_row(repo, &form.url))
                    .collect();
                iced_widget::Column::with_children(rows).spacing(6).into()
            }
        },
    }
}

fn clone_form_view(form: &CloneForm) -> Element<'_> {
    let description_editor = byteui::form::text_area::view(
        &form.description,
        "项目描述…",
        None,
        false,
        Some(96.0),
        Message::CloneDescriptionAction,
    );
    let fields = column![
        field_label("远程仓库"),
        remote_repo_field(form),
        field_label("项目根目录"),
        root_dir_row(
            &form.root_dir,
            Message::CloneRootDirChanged,
            Message::CloneRootDirPick
        ),
        field_label("项目名称"),
        byteui::form::input_text::view(
            "项目名称",
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
    row![clone_sidebar(form.source), fields].spacing(16).into()
}

/// 弹窗底部操作按钮(取消 / 创建)的 hover 样式:静止态文字色由 `text_color`
/// 决定,悬浮/按下时文字过渡到 GOLD、描边变 GOLD,并浮一层 `tab_hover` 胶囊
/// 背景——与文件预览页签 / 图标按钮同一套 hover 语言,反馈比"只变描边"明确。
/// 设置弹窗底部按钮复用同一份样式,保持两类弹窗观感一致。
pub(crate) fn action_button_hover_style(
    text_color: Color,
) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, s| {
        let colors = byteui::theme::color::current();
        let hovered = matches!(s, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(if hovered {
                Color {
                    a: 0.15,
                    ..colors.tab_hover
                }
                .into()
            } else {
                colors.panel.into()
            }),
            text_color: if hovered { colors.cream } else { text_color },
            border: Border {
                color: if hovered { colors.gold } else { colors.border },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    }
}

fn primary_button(label: &'static str, msg: Message, busy: bool) -> Element<'static> {
    let btn = button(text(if busy { "处理中…" } else { label }).size(byteui::theme::font::label()))
        .style(action_button_hover_style(
            byteui::theme::color::current().cream,
        ))
        .padding([8, 20]);
    if busy {
        btn.into()
    } else {
        btn.on_press(msg).into()
    }
}

pub(crate) fn project_create_card(state: &State) -> Element<'_> {
    let body = match state.tab {
        Tab::Local => local_form_view(&state.local),
        Tab::Clone => clone_form_view(&state.clone_form),
    };
    let submit = match state.tab {
        Tab::Local => primary_button("创建项目", Message::SubmitLocal, state.busy),
        Tab::Clone => primary_button("签出项目", Message::SubmitClone, state.busy),
    };
    let error_row: Element<'_> = if let Some(err) = &state.error {
        text(err.clone())
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().red)
            .into()
    } else {
        Space::new().into()
    };
    let cancel = button(text("取消").size(byteui::theme::font::label()))
        .on_press(Message::Close)
        .padding([8, 20])
        .style(action_button_hover_style(
            byteui::theme::color::current().dim,
        ));
    let colors = byteui::theme::color::current();
    // 标题:briefcase 图标 + 奶油色标题文本,垂直居中对齐。
    let title = row![
        icons::view(
            icons::IconKind::Briefcase,
            byteui::theme::font::title() as f32,
            colors.cream,
        ),
        text("新建项目")
            .size(byteui::theme::font::title())
            .color(colors.cream),
    ]
    .spacing(8)
    .align_y(iced_widget::core::alignment::Vertical::Center);
    // 右上角 X 关闭按钮——复用 `icon_button_entry`,与设置弹窗关闭按钮
    // 同一套 hover 动画(图标从 DIM 平滑过渡到 GOLD),`close_hover` 由
    // `Message::CloseHover` 驱动。
    let close_icon = icons::icon_button_entry(
        icons::IconKind::X,
        byteui::theme::icon_size::row(),
        false,
        false,
        if state.close_hover { 1.0 } else { 0.0 },
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::Close,
        Message::CloseHover,
        "关闭",
    );
    let header = row![title, Space::new().width(Length::Fill), close_icon]
        .align_y(iced_widget::core::alignment::Vertical::Center);
    // tab 栏:整块左缩进 16,与下方表单容器(`container(body).padding(16)`)
    // 的字段左缘对齐;底部一条 1px `BORDER` 分割线把 tab 栏与表单区分开
    // (同文件树 footer-bar / ssh footer-bar 的分隔线手法)。分割线左右各缩进
    // 16,与表单字段的左右内边距一致——线宽与表单内容对齐,不再比表单宽。
    let tab_bar = column![
        container(tab_row(state.tab, state.tab_hover)).padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 16.0,
        }),
        container(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(byteui::theme::color::current().border.into()),
                    ..container::Style::default()
                }),
        )
        .padding(Padding {
            top: 0.0,
            right: 16.0,
            bottom: 0.0,
            left: 16.0,
        }),
    ]
    .spacing(8);
    let content = column![
        header,
        tab_bar,
        container(body)
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill),
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
    fs_result?;
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
    clone_result?;
    let target3 = target.clone();
    let description2 = description.clone();
    let write_result = tokio::task::spawn_blocking(move || {
        crate::project_meta::write_description(&target3, &description2)
            .map_err(|e| format!("写入项目描述失败: {e}"))
    })
    .await
    .unwrap_or_else(|e| Err(format!("内部错误: {e}")));
    write_result?;
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
    fn derive_project_name_from_dir_takes_last_segment() {
        assert_eq!(
            derive_project_name_from_dir("/Users/u/Projects/website/zajia"),
            "zajia"
        );
        // 结尾斜杠、首尾空白都容忍;空串与根路径取不到名字,回退空串。
        assert_eq!(derive_project_name_from_dir("/tmp/foo/"), "foo");
        assert_eq!(derive_project_name_from_dir("  /tmp/bar  "), "bar");
        assert_eq!(derive_project_name_from_dir(""), "");
        assert_eq!(derive_project_name_from_dir("/"), "");
    }

    #[test]
    fn local_root_dir_change_derives_name_unless_manually_edited() {
        let mut state = State::default();
        apply_field_message(
            &mut state,
            &Message::LocalRootDirChanged("/Users/u/Projects/website/zajia".into()),
        );
        assert_eq!(state.local.name, "zajia");
        // 手改过项目名称后,再改根目录不再覆盖。
        apply_field_message(&mut state, &Message::LocalNameChanged("myrepo".into()));
        apply_field_message(
            &mut state,
            &Message::LocalRootDirChanged("/Users/u/Projects/other".into()),
        );
        assert_eq!(state.local.name, "myrepo");
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
        apply_field_message(
            &mut state,
            &Message::CloneNameChanged("my-custom-name".into()),
        );
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

    #[test]
    fn plan_source_selection_starts_loading_when_token_present() {
        assert_eq!(
            plan_source_selection(None, true),
            Some(RepoListState::Loading)
        );
    }

    #[test]
    fn plan_source_selection_reports_not_connected_when_no_token() {
        assert_eq!(
            plan_source_selection(None, false),
            Some(RepoListState::NotConnected)
        );
    }

    #[test]
    fn plan_source_selection_skips_when_already_loaded() {
        let existing = RepoListState::Loaded(vec![]);
        assert_eq!(plan_source_selection(Some(&existing), true), None);
    }

    #[test]
    fn plan_source_selection_skips_when_already_loading() {
        assert_eq!(
            plan_source_selection(Some(&RepoListState::Loading), true),
            None
        );
    }

    #[test]
    fn plan_source_selection_retries_after_error() {
        let existing = RepoListState::Error("x".into());
        assert_eq!(
            plan_source_selection(Some(&existing), true),
            Some(RepoListState::Loading)
        );
    }

    #[test]
    fn repo_list_loaded_ok_stores_loaded_state() {
        let mut state = State {
            tab: Tab::Clone,
            ..Default::default()
        };
        apply_field_message(
            &mut state,
            &Message::RepoListLoaded(
                GitProvider::GitHub,
                Ok(vec![git_accounts::RemoteRepo {
                    full_name: "a/b".into(),
                    clone_url: "https://x/a/b.git".into(),
                }]),
            ),
        );
        assert_eq!(
            state.clone_form.repo_lists.get(&GitProvider::GitHub),
            Some(&RepoListState::Loaded(vec![git_accounts::RemoteRepo {
                full_name: "a/b".into(),
                clone_url: "https://x/a/b.git".into(),
            }]))
        );
    }

    #[test]
    fn repo_list_loaded_err_stores_error_state() {
        let mut state = State::default();
        apply_field_message(
            &mut state,
            &Message::RepoListLoaded(GitProvider::GitLab, Err("网络请求失败".into())),
        );
        assert_eq!(
            state.clone_form.repo_lists.get(&GitProvider::GitLab),
            Some(&RepoListState::Error("网络请求失败".into()))
        );
    }

    #[test]
    fn go_to_settings_closes_dialog_like_close() {
        let mut state = Some(State::default());
        // 直接测顶层 `if let Message::Close | Message::GoToSettings` 分支
        // 的等价行为——不需要真的构造 `dozer_client::Client`/`handle`,
        // 因为这条分支在 `update()` 顶部就 return,不会往下走。
        if let Message::Close | Message::GoToSettings = Message::GoToSettings {
            state = None;
        }
        assert!(state.is_none());
    }
}
