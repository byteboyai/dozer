//! 顶栏设置齿轮弹窗:主题选择 + Git 账户连接(GitHub/GitLab/Gitee,PAT)。
//! 设计见 `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 渲染宿主是独立原生窗口 `platform::settings_overlay::SettingsOverlay`。

use crate::extensions::settings_apps;
use crate::git_accounts::{self, GitProvider};
use crate::ui_text::text;
use bytehost_client::AppHostApi;
use bytehost_git_accounts::{
    AccountSnapshot, Admission, CommitOutcome, ConnectedAccount, GitAccountError, RequestTicket,
    SecretToken,
};
use bytehost_settings::git::{GitIntent, GitPageState};
use bytehost_settings::{PageDescriptor, PageId, PageRegistry, SettingsState};
use bytehost_settings_ui::{LanguageOption, Shell, Text};
use byteui::theme::color::ColorScheme;
use dozer_model::{Locale, LocalePreference, LocaleSnapshot};
use dozer_services::DaemonService;
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{Space, button, column, row};
use std::cell::RefCell;
use std::collections::HashMap;

dozer_core::scope!(LOG, module, "settings");

/// "高级"区块(停止/重新启动 dozerd)的状态机。`Idle`/`Stopped` 各自内嵌
/// `error: Option<String>`——失败态就是"回到默认态/已停止态,附一行错误
/// 文案",不单独建一个 `Failed` 变体(spec「UI 设计」:停止失败按钮恢复
/// 默认态、重启失败保留"重新启动"按钮,两者语义上就是各自基础态的一个
/// 变体,不是第三种独立状态)。
#[derive(Debug, Clone, PartialEq)]
pub enum AdvancedState {
    Idle { error: Option<String> },
    FetchingCount,
    ConfirmingStop { session_count: u32 },
    Stopping,
    Stopped { error: Option<String> },
    RestartingDozerd,
}

/// 设置弹窗左栏切换 tab:主题 / Git 账户 / 高级 / 应用 / 语言。点击切换右侧
/// 内容区,样式与「新建项目」弹窗左侧 tab 一致(`dialog_tab_style` + caption 字号)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // 产品的强类型入口保留全部历史 tab。
pub enum SettingsTab {
    Theme,
    Git,
    Advanced,
    /// 应用宿主:运行时探测、已安装应用、安装流程(bytehost A4c)。
    Apps,
    /// 界面语言:跟随系统 / English / 简体中文。
    Language,
}

/// 「语言」页的保存状态由共享 `SelectionState` 承载;保留别名让调用点不必改名。
#[allow(unused_imports)] // 调用点 / 测试继续用 `LanguageStatus` 这个名字。
pub use bytehost_settings::SaveStatus as LanguageStatus;

/// 「语言」页状态:已确认值 + 乐观待选值 + 保存结果。
pub type LanguageState = bytehost_settings::SelectionState<LocalePreference>;

/// 产品注册的页面顺序:Theme / Git / Advanced / Apps / Language(默认 Theme)。
/// Inner page controls consume Escape before closing the owning dialog.
pub(crate) fn escape_message(state: &State) -> Option<Message> {
    if state
        .core
        .current()
        .is_some_and(|page| page.as_str() == bytehost_settings::APPS_ID)
    {
        if let Some(message) = state.apps.escape_message() {
            return Some(Message::Apps(message));
        }
    }
    if state
        .core
        .current()
        .is_some_and(|page| page.as_str() == "dozer.advanced")
        && matches!(state.advanced, AdvancedState::ConfirmingStop { .. })
    {
        return Some(Message::AdvancedStopCancel);
    }
    None
}

pub fn product_registry() -> PageRegistry {
    PageRegistry::new(vec![
        PageDescriptor::built_in(
            PageId::new(bytehost_settings::THEME_ID).expect("static id"),
            "Theme",
        )
        .with_text_key("settings.tab.theme"),
        PageDescriptor::built_in(
            PageId::new(bytehost_settings::GIT_ID).expect("static id"),
            "Git Accounts",
        )
        .with_text_key("settings.tab.git"),
        PageDescriptor::custom(
            PageId::new("dozer.advanced").expect("static id"),
            "settings.tab.advanced",
            "Advanced",
        ),
        PageDescriptor::built_in(
            PageId::new(bytehost_settings::APPS_ID).expect("static id"),
            "Apps",
        )
        .with_text_key("settings.tab.apps"),
        PageDescriptor::built_in(
            PageId::new(bytehost_settings::LANGUAGE_ID).expect("static id"),
            "Language",
        )
        .with_text_key("settings.tab.language"),
    ])
    .expect("Dozer 设置页注册必须有效")
    // 历史直达入口 `dozer.git` 继续可用,但导航里只有一页。
    .with_alias(
        PageId::new("dozer.git").expect("static legacy id"),
        PageId::new(bytehost_settings::GIT_ID).expect("static id"),
    )
    .expect("legacy dozer.git alias must resolve")
}

/// 产品枚举 → 共享页面 ID。
pub fn page_id(tab: SettingsTab) -> PageId {
    let raw = match tab {
        SettingsTab::Theme => bytehost_settings::THEME_ID,
        SettingsTab::Git => bytehost_settings::GIT_ID,
        SettingsTab::Advanced => "dozer.advanced",
        SettingsTab::Apps => bytehost_settings::APPS_ID,
        SettingsTab::Language => bytehost_settings::LANGUAGE_ID,
    };
    PageId::new(raw).expect("static page id")
}

/// 共享页面 ID → 产品枚举(未知页返回 `None`)。
#[allow(dead_code)]
pub fn tab_of(page: &PageId) -> Option<SettingsTab> {
    match page.as_str() {
        bytehost_settings::THEME_ID => Some(SettingsTab::Theme),
        bytehost_settings::GIT_ID => Some(SettingsTab::Git),
        "dozer.advanced" => Some(SettingsTab::Advanced),
        bytehost_settings::APPS_ID => Some(SettingsTab::Apps),
        bytehost_settings::LANGUAGE_ID => Some(SettingsTab::Language),
        _ => None,
    }
}

thread_local! {
    /// 当前渲染帧的 locale:共享视图只拿到 `(key, fallback)` 解析函数,
    /// 由产品在进入渲染前写入,渲染结束恢复。
    static SETTINGS_LOCALE: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(crate) fn with_locale<R>(locale: &str, f: impl FnOnce() -> R) -> R {
    let previous = SETTINGS_LOCALE.replace(locale.to_owned());
    let result = f();
    SETTINGS_LOCALE.replace(previous);
    result
}

/// 按 byteui 的 `{"scheme":"dark"|"light"}` 格式落盘,并把错误交给调用方展示。
pub(crate) fn persist_theme(scheme: ColorScheme) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Persisted {
        scheme: ColorScheme,
    }
    let path = crate::theme::color_theme_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&Persisted { scheme })
        .map_err(|e| format!("theme serialization failed: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// 共享视图的文案解析:沿用 Dozer 现有 `settings.*` 词表;共享新增 key 落英文 fallback。
pub(crate) fn shared_text(key: &str, fallback: &str) -> String {
    SETTINGS_LOCALE.with(|locale| {
        let locale = locale.borrow();
        crate::i18n::host_text(key, fallback, locale.as_str())
    })
}

/// 语言写入结果(由 `App::update` 发起 `set_locale` 后回灌,统一走设置消息通道)。
#[derive(Debug, Clone, PartialEq)]
pub enum LanguageSaveOutcome {
    /// 提交成功,携带落盘后的权威快照。
    Applied(LocaleSnapshot),
    /// revision 冲突,携带服务端当前快照。
    Conflict(LocaleSnapshot),
    /// 保存失败(网络/IO)。
    Failed(String),
    /// 服务不支持语言设置。
    Unsupported,
}

pub struct State {
    /// 共享 Git 页状态(输入、账户、请求身份)。视图与规则只有一份;
    /// 本产品只提供服务与文案注入。
    pub git: GitPageState,
    /// 正在进行的 Git 异步任务句柄,按 provider 存一份。取消 / 关窗时
    /// 真正中止任务;服务层的 `Admission` 仍然负责"已开始的任务不许落盘"。
    git_tasks: HashMap<GitProvider, tokio::task::AbortHandle>,
    pub advanced: AdvancedState,
    /// 共享导航状态(打开 / 当前页 / 窗口代数)。页序与默认页由注册表决定。
    pub core: SettingsState,
    /// 产品注册的页面(Theme / Git / Advanced / Apps / Language)。
    pub registry: PageRegistry,
    /// 右上角关闭图标按钮的悬停态——本弹窗渲染在独立原生窗口
    /// (`SettingsOverlay`),不接入 `App` 的全局 `hover_anims` 定时动画表
    /// (那套动画的自驱 redraw 只唤醒主窗口),所以这里退化成瞬时二值而非
    /// 平滑过渡,复用 `icon_button_entry` 本身的 hover 着色逻辑即可。
    close_hover: bool,
    /// 待发提示(断开账户失败等一次性反馈),`App::update` 的包装函数排空成 Toast。
    pub(crate) outbox: crate::extensions::toast::Outbox,
    /// 「应用」页的状态(运行时探测/已安装应用/安装流程)。
    pub apps: settings_apps::State,
    /// 「语言」页的状态(乐观选择 + 保存状态)。
    pub language: LanguageState,
    /// 主题偏好写入失败原因;显示在主题页,成功保存后清除。
    pub theme_error: Option<String>,
    /// 应用页改了已安装集合或运行状态,主窗口的应用宿主需要立刻刷新(`App::update` 的包装函数取走)。
    host_changed: bool,
}

impl State {
    /// 打开设置弹窗时调用——建立导航与页面状态。Git 账户事实不再从本地
    /// JSON 直接回显,而是由产品在进入 Git 页时调用共享 `query` 获取。
    ///
    /// `daemon_unavailable` 是 `App.daemon_unavailable`(整程序共享的"daemon 连不上"
    /// 状态)。每次开 Settings 都会重建一份 `State`,如果 `advanced` 一律从
    /// `Idle` 起步,用户停止 dozerd 后关窗再开就只剩"停止 dozerd"按钮,
    /// "重新启动 dozerd"这个入口永久丢失(spec 2026-09-19:"可随时重新
    /// 启动"),再点一次只会拿到一行连接报错。所以这里按 daemon 当前是否
    /// 可达推导初值——真相源是可达性,不是上次 UI 停在哪。`ensure_daemon`
    /// 幂等(daemon 其实活着时 `list()` 立刻成功返回 `Ok`),即便传进来的是
    /// 过期的 `daemon_unavailable` 也不会误伤。
    pub fn load(daemon_unavailable: bool) -> State {
        Self::load_with_tab(daemon_unavailable, SettingsTab::Theme)
    }

    /// 同 [`State::load`],但预选某个左栏 tab(如从应用面板「去设置安装」直接落在「应用」页)。
    pub fn load_with_tab(daemon_unavailable: bool, tab: SettingsTab) -> State {
        Self::load_with_page(daemon_unavailable, Some(page_id(tab)))
    }

    /// 按共享 `PageId` 打开(未知页由注册表回退到主题页)。
    pub fn load_with_page(daemon_unavailable: bool, page: Option<PageId>) -> State {
        Self::load_with_page_for_owner(daemon_unavailable, page, 1)
    }

    /// 按共享对话框会话号创建请求 owner。同一对话框内的重试保留 owner;
    /// 新会话使用新 owner,旧请求的回包与持久化准入都不会影响新会话。
    pub fn load_with_page_for_owner(
        daemon_unavailable: bool,
        page: Option<PageId>,
        owner: u64,
    ) -> State {
        let registry = product_registry();
        let mut core = SettingsState::new();
        core.open(&registry, page.as_ref());
        State {
            git: GitPageState::new(owner),
            git_tasks: HashMap::new(),
            advanced: advanced_state_for_daemon(daemon_unavailable),
            core,
            registry,
            close_hover: false,
            outbox: Default::default(),
            apps: Default::default(),
            language: LanguageState::default(),
            theme_error: None,
            host_changed: false,
        }
    }

    /// 内容会话结束:中止所有在途任务并清掉敏感输入。账户事实保留在
    /// 持久化层,重开时由权威查询恢复。
    pub(crate) fn shutdown(&mut self) {
        for (_, task) in self.git_tasks.drain() {
            task.abort();
        }
        self.git.close();
    }

    /// 取走"应用宿主需要刷新"标记(`App::update` 的包装函数调用)。
    pub(crate) fn take_host_changed(&mut self) -> bool {
        std::mem::take(&mut self.host_changed)
    }

    /// 取走待发提示(`App::drain_outboxes` 调用)。
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }
}

/// 开窗时"高级"区块的初始态:`daemon_unavailable` 有值 = daemon 不可达 = 该给
/// "重新启动 dozerd";否则给默认的"停止 dozerd"。抽成纯函数是为了不起
/// daemon 就能测两个分支。
fn advanced_state_for_daemon(daemon_unavailable: bool) -> AdvancedState {
    match daemon_unavailable {
        true => AdvancedState::Stopped { error: None },
        false => AdvancedState::Idle { error: None },
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    CloseHover(bool),
    ThemeSelected(ColorScheme),
    /// 共享 Git 页的类型化意图(连接 / 取消 / 断开 / 重试 / 打开 PAT 页)。
    Git(GitIntent),
    /// 敏感 Token 输入。`SecretToken` 的 Debug 打码、Drop 清零。
    GitToken(GitProvider, SecretToken),
    /// 权威账户查询回包。
    GitQueryLoaded(RequestTicket, Result<AccountSnapshot, GitAccountError>),
    /// 连接回包。
    GitConnectLoaded(
        GitProvider,
        RequestTicket,
        Result<(ConnectedAccount, CommitOutcome), GitAccountError>,
    ),
    /// 断开回包。
    GitDisconnectLoaded(
        GitProvider,
        RequestTicket,
        Result<CommitOutcome, GitAccountError>,
    ),
    AdvancedStopClicked,
    AdvancedSessionCountReady(Result<u32, String>),
    AdvancedStopCancel,
    AdvancedStopConfirm,
    AdvancedStopResult(Result<(), String>),
    AdvancedRestartClicked,
    AdvancedRestartResult(Result<(), String>),
    /// 左栏 tab 切换:点击某组 tab 切到对应内容区(产品入口保留的强类型 API)。
    TabSelected(SettingsTab),
    /// 共享导航点击:页面 ID 形式,允许未来新增产品页。
    PageSelected(PageId),
    /// 「应用」页的全部消息(见 `settings_apps`)。
    Apps(settings_apps::Message),
    /// 用户点选一个语言偏好(跟随系统 / English / 简体中文)。真正的 `set_locale`
    /// 由 `App::update` 发起(需要当前权威 revision 与系统候选),这里只做乐观显示。
    LanguageSelected(LocalePreference),
    /// 语言写入结果回灌(见 `LanguageSaveOutcome`)。
    LanguageSaveOutcome(LanguageSaveOutcome),
}

/// 处理不需要 `handle`(异步)的消息,返回 `true` 表示已处理完。纯状态
/// 转换,方便单测不用真的起 tokio 任务(同 `project_create::
/// apply_field_message` 的既有拆分手法)。
fn apply_sync_message(state: &mut State, msg: &Message) -> bool {
    match msg {
        Message::CloseHover(hovered) => {
            state.close_hover = *hovered;
            true
        }
        Message::ThemeSelected(scheme) => {
            // 即时生效;落盘失败必须可见,不能假装"已保存"(主题可保留到本次进程结束)。
            byteui::theme::color::set_scheme(*scheme);
            state.theme_error = persist_theme(*scheme).err();
            true
        }
        Message::GitToken(provider, token) => {
            state.git.set_token(*provider, token.expose().to_owned());
            true
        }
        Message::AdvancedStopCancel => {
            state.advanced = AdvancedState::Idle { error: None };
            true
        }
        Message::AdvancedSessionCountReady(result) => {
            state.advanced = match result {
                Ok(n) => AdvancedState::ConfirmingStop { session_count: *n },
                Err(e) => AdvancedState::Idle {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::AdvancedStopResult(result) => {
            state.advanced = match result {
                Ok(()) => AdvancedState::Stopped { error: None },
                Err(e) => AdvancedState::Idle {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::AdvancedRestartResult(result) => {
            state.advanced = match result {
                Ok(()) => AdvancedState::Idle { error: None },
                Err(e) => AdvancedState::Stopped {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::TabSelected(tab) => {
            let id = page_id(*tab);
            state.core.select(&state.registry, &id);
            true
        }
        Message::PageSelected(id) => {
            state.core.select(&state.registry, id);
            true
        }
        Message::LanguageSelected(pref) => {
            // 乐观显示用户的选择,进入 Saving;保存中忽略新点击由共享状态层保证。
            state.language.select(pref.clone());
            true
        }
        Message::LanguageSaveOutcome(outcome) => {
            match outcome {
                LanguageSaveOutcome::Applied(snapshot) => {
                    state.language.applied(snapshot.preference.clone());
                }
                LanguageSaveOutcome::Conflict(snapshot) => {
                    state.language.conflict(snapshot.preference.clone());
                }
                LanguageSaveOutcome::Failed(e) => {
                    state.language.failed(e.clone());
                }
                LanguageSaveOutcome::Unsupported => {
                    state.language.unsupported();
                }
            }
            true
        }
        Message::Close
        | Message::Git(_)
        | Message::GitQueryLoaded(..)
        | Message::GitConnectLoaded(..)
        | Message::GitDisconnectLoaded(..)
        | Message::AdvancedStopClicked
        | Message::AdvancedStopConfirm
        | Message::AdvancedRestartClicked
        // `Apps` 在 `update` 里先于本函数分流,到不了这里。
        | Message::Apps(_) => false,
    }
}

/// `state` 是 `&mut Option<State>`(不是 `&mut State`)——`Message::Close`
/// 需要能把它整个置回 `None`,同 `file_history::update`/`project_create::
/// update` 的既有写法。
pub fn update<A>(
    state: &mut Option<State>,
    msg: Message,
    daemon: std::sync::Arc<dyn DaemonService>,
    apps: A,
    admission: std::sync::Arc<Admission>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) where
    A: AppHostApi + Clone + Send + Sync + 'static,
{
    if let Message::Close = msg {
        if let Some(s) = state.as_mut() {
            for (_, task) in s.git_tasks.drain() {
                task.abort();
            }
            admission.cancel_owner(s.git.owner());
            s.git.close();
        }
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if let Message::Apps(apps_msg) = msg {
        run_apps_message(s, apps_msg, apps, handle, emit.clone());
        return;
    }
    // 切到「应用」/「Git」页要现拉权威数据(其余页纯本地)。
    let opened_apps = matches!(&msg, Message::TabSelected(SettingsTab::Apps))
        || matches!(&msg, Message::PageSelected(id) if id.as_str() == bytehost_settings::APPS_ID);
    let opened_git = matches!(&msg, Message::TabSelected(SettingsTab::Git))
        || matches!(&msg, Message::PageSelected(id) if id.as_str() == bytehost_settings::GIT_ID);
    // Git 回包要按请求身份应用,并可能在取消后重新查询;在纯状态分流前处理。
    match msg {
        Message::GitQueryLoaded(ticket, result) => {
            match result {
                Ok(snapshot) => {
                    s.git.apply_query(ticket, snapshot);
                }
                Err(error) => {
                    s.git.query_failed(ticket, error);
                }
            }
            return;
        }
        Message::GitConnectLoaded(provider, ticket, result) => {
            s.git_tasks.remove(&provider);
            if apply_git_connect_result(s, ticket, result) {
                spawn_git_query(s, handle, emit.clone());
            }
            return;
        }
        Message::GitDisconnectLoaded(provider, ticket, result) => {
            s.git_tasks.remove(&provider);
            if apply_git_disconnect_result(s, ticket, result) {
                spawn_git_query(s, handle, emit.clone());
            }
            return;
        }
        msg => {
            if apply_sync_message(s, &msg) {
                if opened_apps {
                    run_apps_message(
                        s,
                        settings_apps::Message::Opened,
                        apps,
                        handle,
                        emit.clone(),
                    );
                } else if opened_git {
                    spawn_git_query(s, handle, emit.clone());
                }
                return;
            }
            match msg {
                Message::Git(intent) => {
                    run_git_intent(s, intent, admission, handle, emit.clone());
                    return;
                }
                Message::AdvancedStopClicked => {
                    if !matches!(s.advanced, AdvancedState::Idle { .. }) {
                        return;
                    }
                    s.advanced = AdvancedState::FetchingCount;
                    handle.spawn(async move {
                        let result = daemon
                            .list_sessions()
                            .await
                            .map(|sessions| count_live_sessions(&sessions))
                            .map_err(|e| e.message);
                        emit(Message::AdvancedSessionCountReady(result));
                    });
                    return;
                }
                Message::AdvancedStopConfirm => {
                    if !matches!(s.advanced, AdvancedState::ConfirmingStop { .. }) {
                        return;
                    }
                    s.advanced = AdvancedState::Stopping;
                    handle.spawn(async move {
                        let result = daemon.shutdown_daemon().await.map_err(|e| e.message);
                        emit(Message::AdvancedStopResult(result));
                    });
                    return;
                }
                Message::AdvancedRestartClicked => {
                    if !matches!(s.advanced, AdvancedState::Stopped { .. }) {
                        return;
                    }
                    s.advanced = AdvancedState::RestartingDozerd;
                    handle.spawn(async move {
                        let result = daemon.ensure_running().await.map_err(|e| e.message);
                        emit(Message::AdvancedRestartResult(result));
                    });
                    return;
                }
                _ => unreachable!("已在 apply_sync_message 或上面处理"),
            }
        }
    }
}

/// 应用连接回包。返回 `true` 表示需要重新查询权威账户事实
/// (被取消 / 被取代的请求已经完成持久化,事实不能静默丢弃)。
fn apply_git_connect_result(
    state: &mut State,
    ticket: RequestTicket,
    result: Result<(ConnectedAccount, CommitOutcome), GitAccountError>,
) -> bool {
    match result {
        Ok((account, outcome)) => {
            if state
                .git
                .connect_succeeded(ticket, account, outcome.clone())
            {
                false
            } else if outcome.credential_committed {
                state.git.observe_orphan_commit(&outcome);
                true
            } else {
                false
            }
        }
        Err(error) => {
            state.git.connect_failed(ticket, error);
            false
        }
    }
}

fn apply_git_disconnect_result(
    state: &mut State,
    ticket: RequestTicket,
    result: Result<CommitOutcome, GitAccountError>,
) -> bool {
    match result {
        Ok(outcome) => {
            if state.git.disconnect_succeeded(ticket, outcome.clone()) {
                false
            } else if outcome.credential_committed {
                state.git.observe_orphan_commit(&outcome);
                true
            } else {
                false
            }
        }
        Err(error) => {
            state.git.disconnect_failed(ticket, error);
            false
        }
    }
}

fn spawn_git_query(
    state: &mut State,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let ticket = state.git.begin_query();
    handle.spawn(async move {
        let service = git_accounts::service();
        emit(Message::GitQueryLoaded(ticket, service.query()));
    });
}

/// 执行共享 Git 页的类型化意图。产品只提供服务注入与任务生命周期。
fn run_git_intent(
    state: &mut State,
    intent: GitIntent,
    admission: std::sync::Arc<Admission>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    match intent {
        GitIntent::Connect { provider } => {
            state.git.begin_edit(provider);
        }
        GitIntent::Cancel { provider } => {
            if let Some(ticket) = state.git.cancel(provider) {
                admission.cancel(ticket);
            }
            if let Some(task) = state.git_tasks.remove(&provider) {
                task.abort();
            }
        }
        GitIntent::OpenTokenPage { provider } => {
            let _ = std::process::Command::new("open")
                .arg(provider.token_creation_url())
                .spawn();
        }
        GitIntent::Submit { provider } => {
            let Some(ticket) = state.git.begin_connect(provider) else {
                return;
            };
            admission.register(ticket);
            if let Some(old) = state.git_tasks.remove(&provider) {
                old.abort();
            }
            // 防御性复制:任务执行时页面可能已经关闭 / 输入已清零。
            let token = SecretToken::new(state.git.token(provider));
            let admission_for_task = admission.clone();
            let emit_for_task = emit.clone();
            let join = handle.spawn(async move {
                let service = git_accounts::service_with_admission(admission_for_task);
                let result = service
                    .connect_with_account_at(provider, &token, Some(ticket))
                    .await;
                emit_for_task(Message::GitConnectLoaded(provider, ticket, result));
            });
            state.git_tasks.insert(provider, join.abort_handle());
        }
        GitIntent::Disconnect { provider } => {
            let Some(ticket) = state.git.begin_disconnect(provider) else {
                return;
            };
            admission.register(ticket);
            if let Some(old) = state.git_tasks.remove(&provider) {
                old.abort();
            }
            let admission_for_task = admission.clone();
            let emit_for_task = emit.clone();
            let join = handle.spawn(async move {
                let service = git_accounts::service_with_admission(admission_for_task);
                let result = service.disconnect_at(provider, Some(ticket));
                emit_for_task(Message::GitDisconnectLoaded(provider, ticket, result));
            });
            state.git_tasks.insert(provider, join.abort_handle());
        }
        GitIntent::RetryQuery => spawn_git_query(state, handle, emit),
    }
}

/// 「应用」页的消息:先过纯状态机,再执行它吐出的副作用(发请求/弹 Toast/标记主窗口刷新)。
fn run_apps_message<A>(
    s: &mut State,
    msg: settings_apps::Message,
    apps: A,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) where
    A: AppHostApi + Clone + Send + Sync + 'static,
{
    use crate::extensions::app_host::Failure;
    use crate::extensions::settings_apps::{ActKind, Effect, Message as M};
    use bytehost_apps::id::AppId;
    use bytehost_apps::plan::{Provenance, TrustLevel};

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let effects = s.apps.update(msg, now_ms);
    let emit = std::sync::Arc::new(std::sync::Mutex::new(emit));
    let send = {
        let emit = std::sync::Arc::clone(&emit);
        move |m: M| {
            if let Ok(emit) = emit.lock() {
                emit(Message::Apps(m));
            }
        }
    };
    for effect in effects {
        let api = apps.clone();
        let send = send.clone();
        match effect {
            Effect::Probe => {
                handle.spawn(async move {
                    let r = api.app_probe_runtimes().await;
                    send(M::ProbesLoaded(r));
                });
            }
            Effect::List => {
                handle.spawn(async move {
                    let r = api.app_list().await;
                    send(M::ListLoaded(r));
                });
            }
            // 选目录/选压缩包对话框是阻塞的原生窗口,由 `window_events` 拦截
            // `InstallClicked`/`PickArchive` 在窗口层执行;这里只是记录意图。
            Effect::PickSource | Effect::PickArchive => {}
            Effect::Plan(source) => {
                handle.spawn(async move {
                    let r = api
                        .app_plan(source, Provenance::Local, TrustLevel::Trusted)
                        .await
                        .map(Box::new);
                    send(M::PlanLoaded(r));
                });
            }
            Effect::Install { approved, source } => {
                handle.spawn(async move {
                    let r = api.app_install(*approved, source).await;
                    send(M::InstallDone(r));
                });
            }
            Effect::Stop(id) => {
                handle.spawn(async move {
                    let r = match AppId::new(&id) {
                        Ok(app) => api.app_stop(app).await,
                        Err(e) => Err(Failure::Transport(format!("{e}"))),
                    };
                    send(M::ActionDone(id, ActKind::Stop, r));
                });
            }
            Effect::Rollback(id) => {
                handle.spawn(async move {
                    let r = match AppId::new(&id) {
                        Ok(app) => api.app_rollback(app).await,
                        Err(e) => Err(Failure::Transport(format!("{e}"))),
                    };
                    send(M::RolledBack(id, r));
                });
            }
            Effect::Uninstall(id, mode) => {
                handle.spawn(async move {
                    let r = match AppId::new(&id) {
                        Ok(app) => api.app_uninstall(app, mode).await,
                        Err(e) => Err(Failure::Transport(format!("{e}"))),
                    };
                    send(M::ActionDone(id, ActKind::Uninstall, r));
                });
            }
            Effect::HostChanged => s.host_changed = true,
            Effect::Notice { level, text, key } => {
                let level = crate::extensions::app_host::notice_level(level);
                s.outbox
                    .push_keyed(LOG, level, crate::extensions::toast::literal(text), key);
            }
            Effect::RuntimePlan(runtime) => {
                handle.spawn(async move {
                    let r = api.app_runtime_plan(runtime).await.map(Box::new);
                    send(M::RuntimePlanLoaded(r));
                });
            }
            Effect::InstallRuntime(plan) => {
                handle.spawn(async move {
                    let r = api.app_install_runtime(*plan).await;
                    send(M::RuntimeInstallStarted(r));
                });
            }
            Effect::UninstallRuntime(runtime, version) => {
                handle.spawn(async move {
                    let r = api.app_uninstall_runtime(runtime, &version).await;
                    send(M::RuntimeUninstallDone(r));
                });
            }
            Effect::ProbeAfter(duration) => {
                handle.spawn(async move {
                    tokio::time::sleep(duration).await;
                    send(M::PollProbes);
                });
            }
            Effect::FetchLogs(id, max_lines) => {
                handle.spawn(async move {
                    let r = match AppId::new(&id) {
                        Ok(app) => api.app_logs(app, max_lines).await,
                        Err(e) => Err(Failure::Transport(format!("{e}"))),
                    };
                    send(M::LogsLoaded(id, r));
                });
            }
        }
    }
    // 有展开着的日志查看器:排一拍 `Tick` 定时刷新(收起后不再排,不后台空转)。
    if s.apps.arm_tick() {
        let send = send.clone();
        handle.spawn(async move {
            tokio::time::sleep(crate::extensions::app_logs::REFRESH_INTERVAL).await;
            send(M::Tick);
        });
    }
}

/// 确认弹窗正文里"N 个正在运行的 agent 会话"的 N(spec 2026-09-19 §「UI
/// 设计」:插入的是"当前**存活**会话数")。`ListSessions` 返回的是 registry
/// 全量快照——daemon 侧从不摘除已退出的会话,所以直接 `len()` 会把用户早就
/// 关掉的 tab 也算进"正在运行",既虚报数字又让 N==0 那条分支几乎永远走不到。
/// 抽成纯函数是为了不起 daemon 就能测。
fn count_live_sessions(sessions: &[dozer_model::SessionInfo]) -> u32 {
    sessions.iter().filter(|s| s.alive).count() as u32
}

fn advanced_row<'a>(
    state: &'a AdvancedState,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let t = |key: &str, fallback: &str| crate::i18n::host_text(key, fallback, locale);
    let hint = |s: String| text(s).size(byteui::theme::font::label()).color(colors.dim);
    let error_line =
        |e: &Option<String>| -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
            match e {
                Some(msg) => text(msg.clone())
                    .size(byteui::theme::font::label())
                    .color(colors.red)
                    .into(),
                None => Space::new().into(),
            }
        };
    let stop_hint = || {
        t(
            "settings.advanced.stop_hint",
            "Stopping ends all running agent sessions and generates summaries; you can restart anytime.",
        )
    };
    match state {
        AdvancedState::Idle { error } => {
            let btn = button(
                text(t("settings.advanced.stop_dozerd", "Stop dozerd"))
                    .size(byteui::theme::font::body()),
            )
            .on_press(Message::AdvancedStopClicked)
            .padding([6, 14])
            .style(crate::extensions::project_create::action_button_hover_style(colors.cream));
            column![
                hint(stop_hint()),
                row![Space::new().width(Length::Fill), btn],
                error_line(error),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::FetchingCount => {
            let btn = button(
                text(t("settings.advanced.checking", "Checking…"))
                    .size(byteui::theme::font::body()),
            )
            .padding([6, 14]);
            row![hint(stop_hint()), Space::new().width(Length::Fill), btn]
                .spacing(6)
                .align_y(Alignment::Center)
                .into()
        }
        AdvancedState::ConfirmingStop { session_count } => {
            let body = if *session_count == 0 {
                t(
                    "settings.advanced.stop_confirm_none",
                    "No sessions are running; dozerd will stop immediately.",
                )
            } else {
                crate::i18n::host_text_args(
                    "settings.advanced.stop_confirm_some",
                    "This ends the {session_count} running agent sessions and generates summaries (may take about a minute).",
                    locale,
                    &[("session_count", &session_count.to_string())],
                )
            };
            let cancel = button(
                text(t("settings.advanced.cancel", "Cancel")).size(byteui::theme::font::body()),
            )
            .on_press(Message::AdvancedStopCancel)
            .padding([6, 14])
            .style(crate::extensions::project_create::action_button_hover_style(colors.dim));
            let confirm = button(
                text(t("settings.advanced.confirm_stop", "Confirm Stop"))
                    .size(byteui::theme::font::body())
                    .color(colors.red),
            )
            .on_press(Message::AdvancedStopConfirm)
            .padding([6, 14])
            .style(crate::extensions::project_create::action_button_hover_style(colors.red));
            column![
                hint(body),
                row![Space::new().width(Length::Fill), cancel, confirm].spacing(8),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::Stopping => {
            let btn = button(
                text(t(
                    "settings.advanced.stopping",
                    "Stopping… (waiting for session summaries, up to about a minute)",
                ))
                .size(byteui::theme::font::body()),
            )
            .padding([6, 14]);
            row![Space::new().width(Length::Fill), btn].into()
        }
        AdvancedState::Stopped { error } => {
            let btn = button(
                text(t("settings.advanced.restart_dozerd", "Restart dozerd"))
                    .size(byteui::theme::font::body()),
            )
            .on_press(Message::AdvancedRestartClicked)
            .padding([6, 14])
            .style(crate::extensions::project_create::action_button_hover_style(colors.cream));
            column![
                hint(t(
                    "settings.advanced.stopped_hint",
                    "dozerd is stopped; some features are unavailable."
                )),
                row![Space::new().width(Length::Fill), btn],
                error_line(error),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::RestartingDozerd => {
            let btn = button(
                text(t("settings.advanced.starting", "Starting…"))
                    .size(byteui::theme::font::body()),
            )
            .padding([6, 14]);
            row![Space::new().width(Length::Fill), btn].into()
        }
    }
}

/// 设置弹窗左栏单枚切换 tab(主题 / Git 账户 / 高级),样式与「新建项目」
/// 弹窗左侧 `sidebar_entry` 完全一致:caption 字号,选中 CARD 实底、未选中
/// BG 实底,8 圆角胶囊(无边框),选中 CREAM、未选中 DIM,无 hover 配色漂移
/// (与「新建项目」左侧 tab 同一套观感)。
fn theme_selected(scheme: ColorScheme) -> Message {
    Message::ThemeSelected(scheme)
}

fn language_selected(preference: LocalePreference) -> Message {
    Message::LanguageSelected(preference)
}

fn page_selected(id: &PageId) -> Message {
    Message::PageSelected(id.clone())
}

fn close_hover(hovered: bool) -> Message {
    Message::CloseHover(hovered)
}

/// 设置卡片:共享容器负责标题 / 导航 / 关闭 / 滚动;产品按 `PageId` 渲染内容。
pub fn settings_card<'a>(
    state: &'a State,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    with_locale(locale, || build_card(state))
}

fn build_card<'a>(
    state: &'a State,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let current = state.core.current();
    let page = current.map(PageId::as_str).unwrap_or_default();

    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = match page {
        bytehost_settings::THEME_ID => {
            let extra: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = state
                .theme_error
                .as_ref()
                .map(|reason| {
                    vec![
                        text(crate::i18n::host_text_args(
                            "settings.theme.save_failed",
                            "Could not save the theme ({reason}). It is active until restart; pick again to retry.",
                            &current_locale(),
                            &[("reason", reason.as_str())],
                        ))
                        .size(byteui::theme::font::label())
                        .color(colors.red)
                        .into(),
                    ]
                })
                .unwrap_or_default();
            bytehost_settings_ui::theme_page(
                byteui::theme::color::current_scheme(),
                &bytehost_settings_ui::theme::ALL,
                Text::new(&shared_text),
                &theme_selected,
                extra,
            )
        }
        bytehost_settings::GIT_ID => {
            // 共享视图:三 provider 行、遮盖输入、连接 / 取消 / 断开、
            // 忙碌、失败 / 部分提交提示与重试只有这一份实现。
            bytehost_settings_ui::git_accounts_page(bytehost_settings_ui::GitAccountsPage {
                state: &state.git,
                on_token: &|provider, value| Message::GitToken(provider, SecretToken::new(value)),
                on_intent: &Message::Git,
                text: Text::new(&shared_text),
            })
        }
        "dozer.advanced" => column![
            text(shared_text("settings.tab.advanced", "Advanced"))
                .size(byteui::theme::font::subtitle())
                .color(colors.cream),
            advanced_row(&state.advanced, &current_locale()),
        ]
        .spacing(14)
        .into(),
        bytehost_settings::APPS_ID => {
            column![settings_apps::view(&state.apps, &current_locale()).map(Message::Apps)]
                .spacing(14)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
        bytehost_settings::LANGUAGE_ID => {
            let options = language_options();
            bytehost_settings_ui::language_page(
                &state.language,
                &options,
                Text::new(&shared_text),
                &language_selected,
                Vec::new(),
            )
        }
        _ => Space::new().into(),
    };

    let dynamic: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if page == bytehost_settings::APPS_ID {
            body
        } else {
            column![body].spacing(14).into()
        };

    bytehost_settings_ui::settings_shell(Shell {
        pages: state.registry.pages(),
        current,
        content: dynamic,
        on_select: &page_selected,
        on_close: Message::Close,
        on_close_hover: Some(&close_hover),
        close_hovered: state.close_hover,
        text: Text::new(&shared_text),
        // 应用页自带滚动;其余页由共享容器包一层。
        scroll_content: page != bytehost_settings::APPS_ID,
        width: Length::Fill,
    })
}

/// 当前渲染帧的 locale(产品页里的 `host_text` 调用仍然直接用它)。
fn current_locale() -> String {
    SETTINGS_LOCALE.with(|locale| locale.borrow().clone())
}

/// 语言选项:跟随系统 / English / 简体中文。
fn language_options() -> [LanguageOption<LocalePreference>; 3] {
    [
        LanguageOption::new(
            LocalePreference::System,
            "settings.language.follow_system",
            "Follow System",
        ),
        LanguageOption::new(
            LocalePreference::Explicit {
                locale: Locale(Locale::EN.into()),
            },
            "settings.language.english",
            "English",
        ),
        LanguageOption::new(
            LocalePreference::Explicit {
                locale: Locale(Locale::ZH_CN.into()),
            },
            "settings.language.simplified_chinese",
            "Simplified Chinese",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧测试只关心「未连接」的初始态;Git 行为现在由共享 `GitPageState`
    /// 的专门测试覆盖,这里保留三参数签名避免无关测试大改。
    #[derive(Debug, Clone, PartialEq)]
    enum ConnectState {
        NotConnected,
    }

    fn test_state(_github: ConnectState, _gitlab: ConnectState, _gitee: ConnectState) -> State {
        let registry = product_registry();
        let mut core = SettingsState::new();
        core.open(&registry, None);
        State {
            git: GitPageState::new(1),
            git_tasks: HashMap::new(),
            advanced: AdvancedState::Idle { error: None },
            core,
            registry,
            close_hover: false,
            outbox: Default::default(),
            apps: Default::default(),
            language: LanguageState::default(),
            theme_error: None,
            host_changed: false,
        }
    }

    #[test]
    fn escape_cancels_the_inner_apps_confirmation_before_the_settings_dialog() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let apps = PageId::new(bytehost_settings::APPS_ID).unwrap();
        state.core.select(&state.registry, &apps);
        state.apps.flow = settings_apps::Flow::ConfirmUninstall {
            id: "demo".into(),
            name: "Demo".into(),
        };
        let Some(Message::Apps(message)) = escape_message(&state) else {
            panic!("confirmation must consume Escape");
        };
        assert!(state.apps.update(message, 0).is_empty());
        assert!(matches!(state.apps.flow, settings_apps::Flow::Idle));
        assert!(state.core.is_open());
        assert!(escape_message(&state).is_none());
    }

    #[test]
    fn escape_cancels_advanced_stop_confirmation_before_closing_settings() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state
            .core
            .select(&state.registry, &PageId::new("dozer.advanced").unwrap());
        state.advanced = AdvancedState::ConfirmingStop { session_count: 2 };
        assert!(matches!(
            escape_message(&state),
            Some(Message::AdvancedStopCancel)
        ));
    }

    fn snap(pref: LocalePreference, active: &str, rev: u64) -> LocaleSnapshot {
        LocaleSnapshot {
            preference: pref,
            active_locale: Locale(active.into()),
            revision: rev,
            ..LocaleSnapshot::default_snapshot()
        }
    }

    #[test]
    fn registry_keeps_the_five_pages_in_dozer_order_and_defaults_to_theme() {
        let registry = product_registry();
        let ids: Vec<_> = registry.pages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "bytehost.theme",
                "bytehost.git",
                "dozer.advanced",
                "bytehost.apps",
                "bytehost.language"
            ]
        );
        assert_eq!(
            registry.default_page().map(PageId::as_str),
            Some("bytehost.theme")
        );
        let mut state = State::load(false);
        state
            .core
            .open(&registry, Some(&PageId::new("missing.page").unwrap()));
        assert_eq!(
            state.core.current().map(PageId::as_str),
            Some("bytehost.theme")
        );
    }

    #[test]
    fn legacy_dozer_git_entry_resolves_to_the_shared_git_page() {
        let registry = product_registry();
        let legacy = PageId::new("dozer.git").expect("legacy id");
        let shared = PageId::new(bytehost_settings::GIT_ID).expect("shared id");
        assert_eq!(registry.alias_target(&legacy), Some(&shared));
        assert_eq!(registry.resolve(Some(&legacy)), Some(&shared));
        // 导航里仍然只有一页 Git。
        assert_eq!(
            registry
                .pages()
                .iter()
                .filter(|page| page.id.as_str().ends_with(".git"))
                .count(),
            1
        );
    }

    #[test]
    fn page_ids_map_to_the_product_tabs_and_back() {
        for tab in [
            SettingsTab::Theme,
            SettingsTab::Git,
            SettingsTab::Advanced,
            SettingsTab::Apps,
            SettingsTab::Language,
        ] {
            assert_eq!(tab_of(&page_id(tab)), Some(tab));
        }
    }

    #[test]
    fn language_selected_sets_pending_and_saving() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let pref = LocalePreference::Explicit {
            locale: Locale(Locale::ZH_CN.into()),
        };
        apply_sync_message(&mut state, &Message::LanguageSelected(pref.clone()));
        assert_eq!(state.language.pending(), Some(&pref));
        assert_eq!(state.language.status(), &LanguageStatus::Saving);
        // 乐观显示优先于已确认值。
        assert_eq!(state.language.shown(), &pref);
    }

    #[test]
    fn language_selected_ignored_while_saving() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let zh = LocalePreference::Explicit {
            locale: Locale(Locale::ZH_CN.into()),
        };
        let en = LocalePreference::Explicit {
            locale: Locale(Locale::EN.into()),
        };
        apply_sync_message(&mut state, &Message::LanguageSelected(zh.clone()));
        apply_sync_message(&mut state, &Message::LanguageSelected(en));
        assert_eq!(state.language.pending(), Some(&zh));
        assert_eq!(state.language.status(), &LanguageStatus::Saving);
    }

    #[test]
    fn language_applied_clears_pending_and_updates_confirmed() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let zh = LocalePreference::Explicit {
            locale: Locale(Locale::ZH_CN.into()),
        };
        apply_sync_message(&mut state, &Message::LanguageSelected(zh.clone()));
        apply_sync_message(
            &mut state,
            &Message::LanguageSaveOutcome(LanguageSaveOutcome::Applied(snap(
                zh.clone(),
                "zh-CN",
                3,
            ))),
        );
        assert_eq!(state.language.pending(), None);
        assert_eq!(state.language.status(), &LanguageStatus::Idle);
        assert_eq!(state.language.confirmed(), &zh);
        assert_eq!(state.language.shown(), &zh);
    }

    #[test]
    fn language_failed_keeps_pending_for_retry() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let zh = LocalePreference::Explicit {
            locale: Locale(Locale::ZH_CN.into()),
        };
        apply_sync_message(&mut state, &Message::LanguageSelected(zh.clone()));
        apply_sync_message(
            &mut state,
            &Message::LanguageSaveOutcome(LanguageSaveOutcome::Failed("io".into())),
        );
        // 乐观选择保留(可重试),但状态是失败。
        assert_eq!(state.language.pending(), Some(&zh));
        assert!(matches!(state.language.status(), LanguageStatus::Failed(_)));
    }

    #[test]
    fn language_conflict_adopts_server_and_prompts() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let zh = LocalePreference::Explicit {
            locale: Locale(Locale::ZH_CN.into()),
        };
        let en = LocalePreference::Explicit {
            locale: Locale(Locale::EN.into()),
        };
        state.language.update_confirmed(en.clone());
        apply_sync_message(&mut state, &Message::LanguageSelected(zh));
        // 服务端其实已是 en(revision 被别处推进)。
        apply_sync_message(
            &mut state,
            &Message::LanguageSaveOutcome(LanguageSaveOutcome::Conflict(snap(en.clone(), "en", 9))),
        );
        assert_eq!(state.language.pending(), None);
        assert_eq!(state.language.status(), &LanguageStatus::Conflict);
        assert_eq!(state.language.confirmed(), &en);
    }

    #[test]
    fn language_unsupported_clears_pending() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.language.update_confirmed(LocalePreference::System);
        apply_sync_message(
            &mut state,
            &Message::LanguageSelected(LocalePreference::System),
        );
        apply_sync_message(
            &mut state,
            &Message::LanguageSaveOutcome(LanguageSaveOutcome::Unsupported),
        );
        assert_eq!(state.language.pending(), None);
        assert_eq!(state.language.status(), &LanguageStatus::Unsupported);
    }

    #[test]
    fn language_shown_falls_back_to_confirmed() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let zh = LocalePreference::Explicit {
            locale: Locale(Locale::ZH_CN.into()),
        };
        state.language.update_confirmed(zh.clone());
        assert_eq!(state.language.shown(), &zh);
    }

    #[test]
    fn connect_intent_opens_editing_row() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        run_git_intent(
            &mut state,
            GitIntent::Connect {
                provider: GitProvider::GitHub,
            },
            std::sync::Arc::new(Admission::new()),
            &tokio::runtime::Handle::current(),
            |_| {},
        );
        assert!(state.git.is_editing(GitProvider::GitHub));
        assert!(!state.git.is_editing(GitProvider::GitLab));
    }

    #[test]
    fn token_message_updates_editing_draft() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.git.begin_edit(GitProvider::GitHub);
        apply_sync_message(
            &mut state,
            &Message::GitToken(
                GitProvider::GitHub,
                SecretToken::new("SYNTHETIC-NOT-A-REAL-TOKEN"),
            ),
        );
        assert_eq!(
            state.git.token(GitProvider::GitHub),
            "SYNTHETIC-NOT-A-REAL-TOKEN"
        );
    }

    #[test]
    fn cancel_revokes_admission_and_aborts_the_task() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.git.begin_edit(GitProvider::GitHub);
        state.git.set_token(GitProvider::GitHub, "tok");
        let ticket = state.git.begin_connect(GitProvider::GitHub).unwrap();
        let admission = std::sync::Arc::new(Admission::new());
        admission.register(ticket);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let join = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        state
            .git_tasks
            .insert(GitProvider::GitHub, join.abort_handle());

        run_git_intent(
            &mut state,
            GitIntent::Cancel {
                provider: GitProvider::GitHub,
            },
            std::sync::Arc::clone(&admission),
            &tokio::runtime::Handle::current(),
            |_| {},
        );
        assert!(!state.git.is_busy(GitProvider::GitHub));
        assert!(!state.git.is_editing(GitProvider::GitHub));
        assert_eq!(state.git.token(GitProvider::GitHub), "");
        assert!(!admission.admit(ticket));
        assert!(!state.git_tasks.contains_key(&GitProvider::GitHub));
    }

    #[test]
    fn cancelled_request_that_committed_requests_an_authoritative_query() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.git.begin_edit(GitProvider::GitHub);
        state.git.set_token(GitProvider::GitHub, "tok");
        let ticket = state.git.begin_connect(GitProvider::GitHub).unwrap();
        state.git.cancel(GitProvider::GitHub);

        let needs_query = apply_git_connect_result(
            &mut state,
            ticket,
            Ok((
                ConnectedAccount {
                    username: "octocat".into(),
                },
                CommitOutcome::complete(GitProvider::GitHub),
            )),
        );
        assert!(needs_query, "已持久化的迟到结果不能被静默丢弃");
        assert!(state.git.account(GitProvider::GitHub).is_none());
    }

    #[test]
    fn connect_result_ok_sets_connected() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.git.begin_edit(GitProvider::GitHub);
        state.git.set_token(GitProvider::GitHub, "tok");
        let ticket = state.git.begin_connect(GitProvider::GitHub).unwrap();
        let needs_query = apply_git_connect_result(
            &mut state,
            ticket,
            Ok((
                ConnectedAccount {
                    username: "octocat".into(),
                },
                CommitOutcome::complete(GitProvider::GitHub),
            )),
        );
        assert!(!needs_query);
        assert_eq!(
            state.git.account(GitProvider::GitHub).unwrap().username,
            "octocat"
        );
        assert_eq!(state.git.token(GitProvider::GitHub), "");
    }

    #[test]
    fn connect_result_err_keeps_editing_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.git.begin_edit(GitProvider::GitHub);
        state.git.set_token(GitProvider::GitHub, "tok");
        let ticket = state.git.begin_connect(GitProvider::GitHub).unwrap();
        apply_git_connect_result(&mut state, ticket, Err(GitAccountError::TokenInvalid));
        assert_eq!(
            state.git.error(GitProvider::GitHub),
            Some(&GitAccountError::TokenInvalid)
        );
        assert!(!state.git.is_busy(GitProvider::GitHub));
    }

    #[test]
    fn close_is_not_a_sync_message() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        assert!(!apply_sync_message(&mut state, &Message::Close));
    }

    #[test]
    fn advanced_stop_cancel_returns_to_idle() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::ConfirmingStop { session_count: 2 };
        apply_sync_message(&mut state, &Message::AdvancedStopCancel);
        assert_eq!(state.advanced, AdvancedState::Idle { error: None });
    }

    #[test]
    fn advanced_session_count_ready_ok_opens_confirm() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::FetchingCount;
        apply_sync_message(&mut state, &Message::AdvancedSessionCountReady(Ok(3)));
        assert_eq!(
            state.advanced,
            AdvancedState::ConfirmingStop { session_count: 3 }
        );
    }

    #[test]
    fn advanced_session_count_ready_err_returns_to_idle_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::FetchingCount;
        apply_sync_message(
            &mut state,
            &Message::AdvancedSessionCountReady(Err("daemon 断开".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Idle {
                error: Some("daemon 断开".into())
            }
        );
    }

    #[test]
    fn advanced_stop_result_ok_marks_stopped() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::Stopping;
        apply_sync_message(&mut state, &Message::AdvancedStopResult(Ok(())));
        assert_eq!(state.advanced, AdvancedState::Stopped { error: None });
    }

    #[test]
    fn advanced_stop_result_err_returns_to_idle_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::Stopping;
        apply_sync_message(
            &mut state,
            &Message::AdvancedStopResult(Err("等待 dozerd 停止超时".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Idle {
                error: Some("等待 dozerd 停止超时".into())
            }
        );
    }

    #[test]
    fn advanced_restart_result_ok_returns_to_idle() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::RestartingDozerd;
        apply_sync_message(&mut state, &Message::AdvancedRestartResult(Ok(())));
        assert_eq!(state.advanced, AdvancedState::Idle { error: None });
    }

    #[test]
    fn advanced_restart_result_err_stays_stopped_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::RestartingDozerd;
        apply_sync_message(
            &mut state,
            &Message::AdvancedRestartResult(Err("无法连接 dozerd".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Stopped {
                error: Some("无法连接 dozerd".into())
            }
        );
    }

    #[test]
    fn count_live_sessions_ignores_dead_sessions_in_registry_snapshot() {
        let session = |id: &str, alive: bool| dozer_model::SessionInfo {
            id: id.to_string(),
            name: id.to_string(),
            command: "/bin/sh".to_string(),
            cwd: "/tmp".to_string(),
            alive,
            created_ms: 0,
            agent_state: dozer_model::AgentState::Idle,
            transcript_path: None,
            project_id: Some(1),
            agent: dozer_model::AgentKind::Unknown,
        };

        assert_eq!(count_live_sessions(&[]), 0);
        assert_eq!(count_live_sessions(&[session("a", true)]), 1);
        assert_eq!(count_live_sessions(&[session("a", false)]), 0);
        assert_eq!(
            count_live_sessions(&[
                session("a", true),
                session("b", false),
                session("c", true),
                session("d", false),
            ]),
            2,
            "registry 快照里的死会话不该被算成「正在运行」"
        );
    }

    #[test]
    fn advanced_async_messages_are_not_sync_messages() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedStopClicked
        ));
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedStopConfirm
        ));
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedRestartClicked
        ));
    }

    #[test]
    fn git_page_state_is_owned_by_the_dialog_session() {
        let state = State::load_with_page_for_owner(false, None, 42);
        assert_eq!(state.git.owner(), 42);
        assert!(!state.git.is_loading());
    }

    #[test]
    fn close_message_releases_the_content_session() {
        let rt = runtime();
        let mut st = open_state(AdvancedState::Idle { error: None });
        let _ = run(
            &mut st,
            Message::Close,
            &FakeDaemon::default(),
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(st.is_none());
    }

    #[test]
    fn git_query_result_updates_the_shared_page_state() {
        let rt = runtime();
        let mut st = open_state(AdvancedState::Idle { error: None });
        let ticket = st.as_mut().unwrap().git.begin_query();
        let mut accounts = bytehost_git_accounts::GitAccountsState::default();
        accounts.set(
            GitProvider::GitHub,
            Some(ConnectedAccount {
                username: "octocat".into(),
            }),
        );
        let _ = run(
            &mut st,
            Message::GitQueryLoaded(
                ticket,
                Ok(AccountSnapshot {
                    accounts,
                    stale_providers: Vec::new(),
                    unreconciled_providers: Vec::new(),
                }),
            ),
            &FakeDaemon::default(),
            &FakeApps::unscripted(),
            &rt,
        );
        assert_eq!(
            st.unwrap()
                .git
                .account(GitProvider::GitHub)
                .unwrap()
                .username,
            "octocat"
        );
    }

    #[test]
    fn token_input_debug_is_redacted() {
        let message = Message::GitToken(
            GitProvider::GitHub,
            SecretToken::new("SYNTHETIC-NOT-A-REAL-TOKEN"),
        );
        assert!(!format!("{message:?}").contains("SYNTHETIC-NOT-A-REAL-TOKEN"));
        assert!(format!("{message:?}").contains("***"));
    }

    #[test]
    fn advanced_state_for_daemon_offers_restart_only_when_daemon_unreachable() {
        assert_eq!(
            advanced_state_for_daemon(false),
            AdvancedState::Idle { error: None },
            "daemon 可达时应该给「停止 dozerd」"
        );
        assert_eq!(
            advanced_state_for_daemon(true),
            AdvancedState::Stopped { error: None },
            "daemon 不可达时必须给「重新启动 dozerd」,否则关窗重开后入口丢失"
        );
    }

    #[test]
    fn disconnect_success_clears_the_account() {
        let mut s = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        s.git.begin_edit(GitProvider::GitHub);
        s.git.set_token(GitProvider::GitHub, "tok");
        let connect = s.git.begin_connect(GitProvider::GitHub).unwrap();
        s.git.connect_succeeded(
            connect,
            ConnectedAccount {
                username: "octocat".into(),
            },
            CommitOutcome::complete(GitProvider::GitHub),
        );
        let ticket = s.git.begin_disconnect(GitProvider::GitHub).unwrap();
        assert!(!apply_git_disconnect_result(
            &mut s,
            ticket,
            Ok(CommitOutcome::complete(GitProvider::GitHub)),
        ));
        assert!(s.git.account(GitProvider::GitHub).is_none());
    }

    #[test]
    fn disconnect_partial_metadata_warning_stays_visible() {
        let mut s = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        s.git.begin_edit(GitProvider::GitHub);
        s.git.set_token(GitProvider::GitHub, "tok");
        let connect = s.git.begin_connect(GitProvider::GitHub).unwrap();
        s.git.connect_succeeded(
            connect,
            ConnectedAccount {
                username: "octocat".into(),
            },
            CommitOutcome::complete(GitProvider::GitHub),
        );
        let ticket = s.git.begin_disconnect(GitProvider::GitHub).unwrap();
        let mut outcome = CommitOutcome::complete(GitProvider::GitHub);
        outcome.metadata_committed = false;
        outcome.warning = Some(GitAccountError::MetadataWrite {
            detail: "disk full".into(),
        });
        apply_git_disconnect_result(&mut s, ticket, Ok(outcome));
        assert!(s.git.account(GitProvider::GitHub).is_none());
        assert!(matches!(
            s.git.notice(),
            Some(GitAccountError::MetadataWrite { .. })
        ));
    }

    // ---- 服务接缝:Advanced 页(DaemonService)与「应用」页(AppHostApi 泛型)----

    use crate::panel_host::testing::{TIMEOUT, runtime};
    use crate::service_fakes::{FakeApps, FakeDaemon, session};
    use bytehost_apps::proto::{AppReply, AppRequest};
    use bytehost_client::AppApiError;
    use dozer_services::{ServiceError, ServiceErrorKind};
    use std::sync::{Arc, mpsc};

    fn daemon(fake: &FakeDaemon) -> Arc<dyn dozer_services::DaemonService> {
        fake.clone().into_services().daemon
    }

    fn open_state(advanced: AdvancedState) -> Option<State> {
        let mut s = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        s.advanced = advanced;
        Some(s)
    }

    fn run(
        state: &mut Option<State>,
        msg: Message,
        fake: &FakeDaemon,
        apps: &FakeApps,
        rt: &tokio::runtime::Runtime,
    ) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        update(
            state,
            msg,
            daemon(fake),
            apps.clone(),
            std::sync::Arc::new(Admission::new()),
            rt.handle(),
            move |m| {
                let _ = tx.send(m);
            },
        );
        rx
    }

    const QUIET: std::time::Duration = std::time::Duration::from_millis(300);

    #[test]
    fn stop_clicked_counts_only_alive_sessions() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_sessions(Ok(vec![
            session("a", true),
            session("b", false),
            session("c", true),
        ]));
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(
            &mut st,
            Message::AdvancedStopClicked,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(matches!(
            st.as_ref().unwrap().advanced,
            AdvancedState::FetchingCount
        ));
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedSessionCountReady(r) => assert_eq!(r, Ok(2)),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn stop_clicked_is_ignored_unless_idle() {
        let rt = runtime();
        let fake = FakeDaemon::default();
        let mut st = open_state(AdvancedState::Stopping);
        let rx = run(
            &mut st,
            Message::AdvancedStopClicked,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(rx.recv_timeout(QUIET).is_err());
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn stop_clicked_failure_text_is_the_service_message_verbatim() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_sessions(Err(ServiceError::new(
            ServiceErrorKind::Unavailable,
            "daemon 断开",
        )));
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(
            &mut st,
            Message::AdvancedStopClicked,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedSessionCountReady(r) => assert_eq!(r, Err("daemon 断开".to_string())),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn stop_confirm_calls_shutdown_only_from_confirming_state() {
        let rt = runtime();
        let fake = FakeDaemon::default();
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(
            &mut st,
            Message::AdvancedStopConfirm,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(rx.recv_timeout(QUIET).is_err());
        assert!(fake.calls().is_empty());

        let mut st = open_state(AdvancedState::ConfirmingStop { session_count: 2 });
        let rx = run(
            &mut st,
            Message::AdvancedStopConfirm,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(matches!(
            st.as_ref().unwrap().advanced,
            AdvancedState::Stopping
        ));
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::AdvancedStopResult(Ok(()))
        ));
        assert_eq!(fake.calls(), vec!["shutdown_daemon()".to_string()]);
    }

    #[test]
    fn stop_confirm_failure_text_is_verbatim() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_stop(Err(ServiceError::new(
            ServiceErrorKind::Protocol,
            "等待 dozerd 停止超时",
        )));
        let mut st = open_state(AdvancedState::ConfirmingStop { session_count: 0 });
        let rx = run(
            &mut st,
            Message::AdvancedStopConfirm,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedStopResult(r) => {
                assert_eq!(r, Err("等待 dozerd 停止超时".to_string()))
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn restart_calls_ensure_running_only_from_stopped_and_keeps_error_text() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_ensure(Err(ServiceError::new(
            ServiceErrorKind::Unavailable,
            "无法连接 dozerd（已重试 3 次）：x",
        )));
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(
            &mut st,
            Message::AdvancedRestartClicked,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(rx.recv_timeout(QUIET).is_err());
        assert!(fake.calls().is_empty());

        let mut st = open_state(AdvancedState::Stopped { error: None });
        let rx = run(
            &mut st,
            Message::AdvancedRestartClicked,
            &fake,
            &FakeApps::unscripted(),
            &rt,
        );
        assert!(matches!(
            st.as_ref().unwrap().advanced,
            AdvancedState::RestartingDozerd
        ));
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedRestartResult(r) => {
                assert_eq!(r, Err("无法连接 dozerd（已重试 3 次）：x".to_string()))
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["ensure_running()".to_string()]);
    }

    fn apps_ok() -> FakeApps {
        FakeApps::new(|req| match req {
            AppRequest::List => Ok(AppReply::Apps { apps: vec![] }),
            AppRequest::ProbeRuntimes => Ok(AppReply::Runtimes { runtimes: vec![] }),
            _ => Err(AppApiError::Transport("未预设".into())),
        })
    }

    #[test]
    fn opening_the_apps_tab_probes_runtimes_and_lists_apps() {
        let rt = runtime();
        let apps = apps_ok();
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(
            &mut st,
            Message::TabSelected(SettingsTab::Apps),
            &FakeDaemon::default(),
            &apps,
            &rt,
        );
        let mut got = 0;
        while got < 2 {
            match rx.recv_timeout(TIMEOUT).unwrap() {
                Message::Apps(_) => got += 1,
                other => panic!("unexpected: {other:?}"),
            }
        }
        let reqs = apps.requests();
        assert!(reqs.iter().any(|r| r.contains("ProbeRuntimes")), "{reqs:?}");
        assert!(reqs.iter().any(|r| r.starts_with("List")), "{reqs:?}");
    }

    #[test]
    fn stop_with_an_invalid_id_fails_as_transport_and_sends_no_request() {
        let rt = runtime();
        let apps = FakeApps::unscripted();
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(
            &mut st,
            Message::Apps(settings_apps::Message::StopClicked("Bad Id!".into())),
            &FakeDaemon::default(),
            &apps,
            &rt,
        );
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Apps(settings_apps::Message::ActionDone(
                id,
                _,
                Err(AppApiError::Transport(_)),
            )) => {
                assert_eq!(id, "Bad Id!")
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(apps.requests().is_empty(), "非法 id 不发请求");
    }

    #[test]
    fn invalid_ids_for_rollback_uninstall_and_logs_send_no_requests() {
        use bytehost_apps::registry::UninstallMode;
        use settings_apps::{Flow, Message as M};
        let rt = runtime();
        let id = "Bad Id!";
        for case in 0..3 {
            let apps = FakeApps::unscripted();
            let mut st = open_state(AdvancedState::Idle { error: None });
            let msg = match case {
                0 => {
                    st.as_mut().unwrap().apps.flow = Flow::ConfirmRollback {
                        id: id.into(),
                        name: "invalid".into(),
                        to: bytehost_apps::id::Version::new(0, 1, 0),
                    };
                    M::RollbackConfirmed(id.into())
                }
                1 => {
                    st.as_mut()
                        .unwrap()
                        .apps
                        .update(M::UninstallClicked(id.into()), 0);
                    M::UninstallConfirmed(UninstallMode::Program)
                }
                _ => M::ShowLogs(id.into()),
            };
            let rx = run(
                &mut st,
                Message::Apps(msg),
                &FakeDaemon::default(),
                &apps,
                &rt,
            );
            let expected = bytehost_apps::id::AppId::new(id).unwrap_err().to_string();
            match (case, rx.recv_timeout(TIMEOUT).unwrap()) {
                (0, Message::Apps(M::RolledBack(got, Err(AppApiError::Transport(e)))))
                | (
                    1,
                    Message::Apps(M::ActionDone(
                        got,
                        settings_apps::ActKind::Uninstall,
                        Err(AppApiError::Transport(e)),
                    )),
                )
                | (2, Message::Apps(M::LogsLoaded(got, Err(AppApiError::Transport(e))))) => {
                    assert_eq!(got, id);
                    assert_eq!(e, expected);
                }
                (_, other) => panic!("unexpected: {other:?}"),
            }
            assert!(
                apps.requests().is_empty(),
                "case {case}: invalid id must not send requests"
            );
        }
    }

    #[test]
    fn install_forwards_the_approved_plan_and_source_unchanged() {
        use bytehost_apps::id::{AppId, Version};
        use bytehost_apps::permissions::{Permissions, diff_permissions};
        use bytehost_apps::plan::{InstallPlan, Provenance, TrustLevel};
        let rt = runtime();
        let plan = InstallPlan {
            app_id: AppId::new("excalidraw").unwrap(),
            name: "Excalidraw".into(),
            version: Version::new(0, 17, 0),
            upgrading_from: None,
            provenance: Provenance::Local,
            trust: TrustLevel::Trusted,
            runtime_kind: "static_web".into(),
            manifest_digest: "m".into(),
            source_digest: "s".into(),
            requested: Permissions::default(),
            enforcement: vec![],
            permission_diff: diff_permissions(&Permissions::default(), &Permissions::default()),
            will_run: vec!["不执行任何命令".into()],
            source_info: Default::default(),
        };
        let expected_plan = plan.clone();
        let mut st = open_state(AdvancedState::Idle { error: None });
        {
            let s = st.as_mut().unwrap();
            s.apps.update(settings_apps::Message::InstallClicked, 0);
            s.apps.update(
                settings_apps::Message::SourcePicked(Some("/src/app".into())),
                0,
            );
            s.apps
                .update(settings_apps::Message::PlanLoaded(Ok(Box::new(plan))), 0);
        }
        let apps = FakeApps::new(move |req| match req {
            AppRequest::Install { approved, source } => {
                assert_eq!(approved.plan(), &expected_plan);
                assert_eq!(
                    source,
                    &bytehost_apps::proto::AppSource::LocalDir {
                        path: "/src/app".into()
                    }
                );
                Ok(AppReply::Done)
            }
            _ => Err(AppApiError::Transport("未预设".into())),
        });
        let rx = run(
            &mut st,
            Message::Apps(settings_apps::Message::ApproveClicked),
            &FakeDaemon::default(),
            &apps,
            &rt,
        );
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::Apps(settings_apps::Message::InstallDone(Ok(())))
        ));
        let reqs = apps.requests();
        assert_eq!(reqs.len(), 1, "{reqs:?}");
        assert!(reqs[0].starts_with("Install"), "{reqs:?}");
        assert!(
            reqs[0].contains("excalidraw"),
            "批准的计划原样送入: {reqs:?}"
        );
        assert!(reqs[0].contains("/src/app"), "来源原样送入: {reqs:?}");
    }
}
