//! 顶栏设置齿轮弹窗:主题选择 + Git 账户连接(GitHub/GitLab/Gitee,PAT)。
//! 设计见 `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 渲染宿主是独立原生窗口 `platform::settings_overlay::SettingsOverlay`。

use crate::extensions::settings_apps;
use crate::git_accounts::{self, GitAccountsState, GitProvider};
use bytehost_client::AppHostApi;
use byteui::interaction::icons;
use byteui::theme::color::ColorScheme;
use dozer_model::{Locale, LocalePreference, LocaleSnapshot};
use dozer_services::DaemonService;
use iced_widget::core::{Alignment, Border, Element, Length, Padding};
use iced_widget::{MouseArea, Space, button, column, container, row, text};
use std::collections::HashMap;

dozer_core::scope!(LOG, module, "settings");

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectState {
    NotConnected,
    Editing {
        token: String,
        busy: bool,
        error: Option<dozer_model::LocalizedText>,
    },
    Connected {
        username: String,
    },
}

impl ConnectState {
    fn from_accounts(accounts: &GitAccountsState, provider: GitProvider) -> ConnectState {
        match accounts.get(provider) {
            Some(acc) => ConnectState::Connected {
                username: acc.username.clone(),
            },
            None => ConnectState::NotConnected,
        }
    }
}

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
pub enum SettingsTab {
    Theme,
    Git,
    Advanced,
    /// 应用宿主:运行时探测、已安装应用、安装流程(bytehost A4c)。
    Apps,
    /// 界面语言:跟随系统 / English / 简体中文。
    Language,
}

/// 「语言」页的保存状态(i18n Task 05)。只承载"这次提交"的瞬时反馈;真正
/// 生效的偏好/语言以 daemon 权威快照为准(见 `LocalizedText` 与 `App.locale`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum LanguageStatus {
    /// 空闲:显示当前已确认的选择。
    #[default]
    Idle,
    /// 已提交,等待 daemon 回包(此间禁用选项,防重复提交)。
    Saving,
    /// 保存失败(可重试)。
    Failed(String),
    /// revision 冲突:语言在别处被改,已采纳服务端快照,提示重选。
    Conflict,
    /// 当前服务不支持语言设置(旧 daemon)。
    Unsupported,
}

/// 「语言」页状态:待确认的乐观选择 + 保存状态。
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageState {
    /// 已确认的当前偏好(打开设置时由 `App` 从权威快照注入;成功/冲突后更新)。
    pub confirmed: LocalePreference,
    /// 用户刚点选、尚未被 daemon 确认的偏好(乐观显示;确认/失败/冲突后清空)。
    pub pending: Option<LocalePreference>,
    pub status: LanguageStatus,
}

impl LanguageState {
    /// 当前应显示为"已选"的偏好:有乐观值用乐观值,否则用已确认值。
    pub fn shown(&self) -> &LocalePreference {
        self.pending.as_ref().unwrap_or(&self.confirmed)
    }
}

impl Default for LanguageState {
    fn default() -> Self {
        LanguageState {
            confirmed: LocalePreference::System,
            pending: None,
            status: LanguageStatus::Idle,
        }
    }
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
    pub github: ConnectState,
    pub gitlab: ConnectState,
    pub gitee: ConnectState,
    /// 正在进行的 `ConnectSubmit` 异步任务句柄,按 provider 存一份。
    /// `ConnectCancel` 用它真正中止任务,防止取消后 token 仍被异步写进
    /// Keychain/本地文件(代码评审 finding:Cancel doesn't stop in-flight
    /// connect task)。
    connect_tasks: HashMap<GitProvider, tokio::task::AbortHandle>,
    pub advanced: AdvancedState,
    /// 左栏当前选中的 tab(主题 / Git 账户 / 高级),决定右侧内容区显示哪一组。
    pub selected: SettingsTab,
    /// 左栏 tab 的 hover 态(已不驱动任何视觉——「新建项目」左侧
    /// `sidebar_entry` 本身无 hover 高亮,设置弹窗左栏与之保持一致,所以
    /// 这里仅保留事件埋点,样式上不浮 TAB_HOVER 胶囊、标题也不 DIM→GOLD)。
    tab_hover: Option<SettingsTab>,
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
    /// 应用页改了已安装集合或运行状态,主窗口的应用宿主需要立刻刷新(`App::update` 的包装函数取走)。
    host_changed: bool,
}

impl State {
    /// 打开设置弹窗时调用——从本地 `git_accounts.json` 重建三家的连接
    /// 展示态(不重新校验 token 有效性,只是回显上次连接成功记下的用户名)。
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
        let accounts = git_accounts::load();
        State {
            github: ConnectState::from_accounts(&accounts, GitProvider::GitHub),
            gitlab: ConnectState::from_accounts(&accounts, GitProvider::GitLab),
            gitee: ConnectState::from_accounts(&accounts, GitProvider::Gitee),
            connect_tasks: HashMap::new(),
            advanced: advanced_state_for_daemon(daemon_unavailable),
            selected: tab,
            tab_hover: None,
            close_hover: false,
            outbox: Default::default(),
            apps: Default::default(),
            language: LanguageState::default(),
            host_changed: false,
        }
    }

    /// 取走"应用宿主需要刷新"标记(`App::update` 的包装函数调用)。
    pub(crate) fn take_host_changed(&mut self) -> bool {
        std::mem::take(&mut self.host_changed)
    }

    /// 取走待发提示(`App::drain_outboxes` 调用)。
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }

    fn slot_mut(&mut self, provider: GitProvider) -> &mut ConnectState {
        match provider {
            GitProvider::GitHub => &mut self.github,
            GitProvider::GitLab => &mut self.gitlab,
            GitProvider::Gitee => &mut self.gitee,
        }
    }
}

/// 断开 Git 账户的收尾:`saved` 是"把账户置空写回本地非敏感记录"的结果,`delete_token`
/// 是删钥匙串 token 的动作(惰性传入,以便"本地记录没写成就不能先删 token"可测)。
///
/// 顺序沿用既有裁决:**先写本地记录,成功了才删钥匙串 token**——反过来做的话,本地文件
/// 写失败会留下"钥匙串已删、json 仍写着已连接"这种更糟的不一致态(代码评审 finding:
/// Disconnect drops failed save, leaves stale state)。写失败就整个放弃这次断开,保留
/// 原有已连接展示,并且现在**告诉用户**(此前只写一条日志,用户看到的是点了没反应)。
pub(crate) fn finish_disconnect(
    state: &mut State,
    provider: GitProvider,
    saved: Result<(), std::io::Error>,
    delete_token: impl FnOnce() -> Result<(), crate::git_accounts::GitAccountError>,
) {
    match saved {
        Ok(()) => {
            if let Err(e) = delete_token() {
                state.outbox.push(
                    LOG,
                    crate::extensions::toast::Level::Warning,
                    dozer_model::LocalizedText::keyed(
                        crate::i18n::HOST_OWNER,
                        "error.disconnect_token_delete_failed",
                        "The account was disconnected, but the token could not be removed from the system keychain: {detail}",
                    )
                    .with_arg("detail", e.stable()),
                );
            }
            *state.slot_mut(provider) = ConnectState::NotConnected;
        }
        Err(e) => {
            state.outbox.push(
                LOG,
                crate::extensions::toast::Level::Error,
                dozer_model::LocalizedText::keyed(
                    crate::i18n::HOST_OWNER,
                    "error.disconnect_local_save_failed",
                    "Could not disconnect the account: writing the local record failed ({detail}); the existing connection is kept",
                )
                .with_arg("detail", e.to_string()),
            );
        }
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
    ConnectClicked(GitProvider),
    TokenChanged(GitProvider, String),
    ConnectCancel(GitProvider),
    OpenTokenPage(GitProvider),
    ConnectSubmit(GitProvider),
    ConnectResult(GitProvider, Result<String, dozer_model::LocalizedText>),
    Disconnect(GitProvider),
    AdvancedStopClicked,
    AdvancedSessionCountReady(Result<u32, String>),
    AdvancedStopCancel,
    AdvancedStopConfirm,
    AdvancedStopResult(Result<(), String>),
    AdvancedRestartClicked,
    AdvancedRestartResult(Result<(), String>),
    /// 左栏 tab 切换:点击某组 tab 切到对应内容区。
    TabSelected(SettingsTab),
    /// 左栏 tab 的 hover 进入/离开,`Some(tab)` 进入、`None` 离开。
    TabHover(Option<SettingsTab>),
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
            byteui::theme::color::set_scheme(*scheme);
            byteui::theme::color::persist_scheme(&crate::theme::color_theme_path());
            true
        }
        Message::ConnectClicked(provider) => {
            *state.slot_mut(*provider) = ConnectState::Editing {
                token: String::new(),
                busy: false,
                error: None,
            };
            true
        }
        Message::TokenChanged(provider, token) => {
            if let ConnectState::Editing { token: t, .. } = state.slot_mut(*provider) {
                *t = token.clone();
            }
            true
        }
        Message::ConnectCancel(provider) => {
            // 真正中止掉还在跑的 validate_token/set_token/save 任务——
            // 只改 UI 状态不够,那个任务不知道自己被"取消"了,还是会把
            // token 写进 Keychain(代码评审 finding:Cancel doesn't stop
            // in-flight connect task)。
            if let Some(task) = state.connect_tasks.remove(provider) {
                task.abort();
            }
            *state.slot_mut(*provider) = ConnectState::NotConnected;
            true
        }
        Message::OpenTokenPage(provider) => {
            let _ = std::process::Command::new("open")
                .arg(provider.token_creation_url())
                .spawn();
            true
        }
        Message::Disconnect(provider) => {
            // 先写本地非敏感记录,成功了才删 Keychain token——反过来做的话,
            // 本地文件写失败会留下"Keychain 已删、json 仍写着已连接"这种
            // 更糟的不一致态(代码评审 finding:Disconnect drops failed
            // save, leaves stale state)。写失败就整个放弃这次断开,保留
            // 原有已连接展示并告知用户(见 `finish_disconnect`)。
            let mut accounts = git_accounts::load();
            accounts.set(*provider, None);
            let saved = git_accounts::save(&accounts);
            finish_disconnect(state, *provider, saved, || {
                git_accounts::delete_token(*provider)
            });
            true
        }
        Message::ConnectResult(provider, result) => {
            // 任务已经跑完,不管结果如何都清掉记着的句柄。
            state.connect_tasks.remove(provider);
            // 结果到达时若已经不是 Editing 态(用户已经 Cancel 或又开了
            // 新一轮),说明这是一份过期结果,丢弃不应用——否则会出现
            // "已经点了取消,连接状态却又被悄悄改回已连接"的情况(代码
            // 评审 finding:Cancel doesn't stop in-flight connect task)。
            if matches!(state.slot_mut(*provider), ConnectState::Editing { .. }) {
                match result {
                    Ok(username) => {
                        *state.slot_mut(*provider) = ConnectState::Connected {
                            username: username.clone(),
                        };
                    }
                    Err(e) => {
                        if let ConnectState::Editing { busy, error, .. } = state.slot_mut(*provider)
                        {
                            *busy = false;
                            *error = Some(e.clone());
                        }
                    }
                }
            }
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
            state.selected = *tab;
            true
        }
        Message::TabHover(h) => {
            state.tab_hover = *h;
            true
        }
        Message::LanguageSelected(pref) => {
            // 乐观显示用户的选择,进入 Saving;防重复提交:已保存中忽略新点击。
            if state.language.status == LanguageStatus::Saving {
                return true;
            }
            state.language.pending = Some(pref.clone());
            state.language.status = LanguageStatus::Saving;
            true
        }
        Message::LanguageSaveOutcome(outcome) => {
            match outcome {
                LanguageSaveOutcome::Applied(snapshot) => {
                    state.language.confirmed = snapshot.preference.clone();
                    state.language.pending = None;
                    state.language.status = LanguageStatus::Idle;
                }
                LanguageSaveOutcome::Conflict(snapshot) => {
                    // 语言在别处被改:采纳服务端快照,提示重选。
                    state.language.confirmed = snapshot.preference.clone();
                    state.language.pending = None;
                    state.language.status = LanguageStatus::Conflict;
                }
                LanguageSaveOutcome::Failed(e) => {
                    // 保存失败:保留乐观选择但显示错误,允许重试。
                    state.language.status = LanguageStatus::Failed(e.clone());
                }
                LanguageSaveOutcome::Unsupported => {
                    state.language.pending = None;
                    state.language.status = LanguageStatus::Unsupported;
                }
            }
            true
        }
        Message::Close
        | Message::ConnectSubmit(_)
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
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) where
    A: AppHostApi + Clone + Send + Sync + 'static,
{
    if let Message::Close = msg {
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if let Message::Apps(apps_msg) = msg {
        run_apps_message(s, apps_msg, apps, handle, emit.clone());
        return;
    }
    // 切到「应用」页要现拉探测与列表(其余页纯本地,没有加载)。
    let opened_apps = matches!(msg, Message::TabSelected(SettingsTab::Apps));
    if apply_sync_message(s, &msg) {
        if opened_apps {
            run_apps_message(
                s,
                settings_apps::Message::Opened,
                apps,
                handle,
                emit.clone(),
            );
        }
        return;
    }
    let provider = match msg {
        Message::ConnectSubmit(provider) => provider,
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
    };
    let token = match s.slot_mut(provider) {
        ConnectState::Editing { token, busy, error } => {
            if token.trim().is_empty() {
                return;
            }
            *busy = true;
            *error = None;
            token.clone()
        }
        _ => return,
    };
    // 防御性清理:正常 UI 流程不会在上一次提交还没出结果时再提交一次
    // (按钮在 busy 态会被禁用),但如果真的发生了,先中止旧任务再开新的,
    // 不留两个任务同时跑。
    if let Some(old) = s.connect_tasks.remove(&provider) {
        old.abort();
    }
    let join_handle = handle.spawn(async move {
        let result = git_accounts::validate_token(provider, &token)
            .await
            .map_err(|e| e.localized());
        let result = result.and_then(|username| {
            git_accounts::set_token(provider, &token).map_err(|e| e.localized())?;
            let mut accounts = git_accounts::load();
            accounts.set(
                provider,
                Some(git_accounts::ConnectedAccount {
                    username: username.clone(),
                }),
            );
            git_accounts::save(&accounts).map_err(|e| {
                dozer_model::LocalizedText::keyed(
                    crate::i18n::HOST_OWNER,
                    "error.git_account_local_save_failed",
                    "Could not write the local record: {detail}",
                )
                .with_arg("detail", e.to_string())
            })?;
            Ok(username)
        });
        emit(Message::ConnectResult(provider, result));
    });
    s.connect_tasks.insert(provider, join_handle.abort_handle());
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

fn scheme_row<'a>(
    label: impl Into<String>,
    scheme: ColorScheme,
    current: ColorScheme,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let selected = scheme == current;
    let mark: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        text(if selected { "✓" } else { " " })
            .size(byteui::theme::font::body())
            .into();
    crate::chrome::menu::item_row_fill(
        Some(mark),
        label,
        if selected {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        },
        Some(Message::ThemeSelected(scheme)),
    )
}

/// 一个语言选项行(单选):左侧 ✓ 标记,右侧标签。点击提交该偏好。
fn language_option_row<'a>(
    label: String,
    selected: bool,
    disabled: bool,
    pref: LocalePreference,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let mark: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        text(if selected { "✓" } else { " " })
            .size(byteui::theme::font::body())
            .color(if disabled { colors.dim } else { colors.gold })
            .into();
    let color = if disabled {
        colors.dim
    } else if selected {
        colors.gold
    } else {
        colors.body
    };
    let msg = if disabled {
        None
    } else {
        Some(Message::LanguageSelected(pref))
    };
    crate::chrome::menu::item_row_fill(Some(mark), label, color, msg)
}

/// 「语言」页:跟随系统 / English / 简体中文 三选一,附保存状态行。
///
/// - 选项的"已选"态来自 `LanguageState::shown`(乐观值优先)。
/// - `Saving` 期间禁用全部选项防重复提交(spec Task 05)。
/// - 保存失败/冲突/不支持时在选项下方给一行说明;失败保留乐观选择可重试。
fn language_page<'a>(
    state: &LanguageState,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let t = |key: &str, fallback: &str| crate::i18n::host_text(key, fallback, locale);
    let saving = state.status == LanguageStatus::Saving;
    let shown = state.shown().clone();
    let title = text(t("settings.language.title", "Language"))
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);

    let rows = column![
        language_option_row(
            t("settings.language.follow_system", "Follow System"),
            shown == LocalePreference::System,
            saving,
            LocalePreference::System,
        ),
        language_option_row(
            t("settings.language.english", "English"),
            shown
                == LocalePreference::Explicit {
                    locale: Locale(Locale::EN.into())
                },
            saving,
            LocalePreference::Explicit {
                locale: Locale(Locale::EN.into())
            },
        ),
        language_option_row(
            t("settings.language.simplified_chinese", "Simplified Chinese"),
            shown
                == LocalePreference::Explicit {
                    locale: Locale(Locale::ZH_CN.into())
                },
            saving,
            LocalePreference::Explicit {
                locale: Locale(Locale::ZH_CN.into())
            },
        ),
    ]
    .spacing(6);

    let status_line: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        match &state.status {
            LanguageStatus::Idle => None,
            LanguageStatus::Saving => Some(
                text(t("settings.language.saving", "Saving…"))
                    .size(byteui::theme::font::caption())
                    .color(colors.dim)
                    .into(),
            ),
            LanguageStatus::Failed(_e) => Some(
                text(t(
                    "settings.language.save_failed",
                    "Couldn't save. Try again.",
                ))
                .size(byteui::theme::font::caption())
                .color(colors.gold)
                .into(),
            ),
            LanguageStatus::Conflict => Some(
                text(t(
                    "settings.language.conflict",
                    "Language changed elsewhere — review your choice.",
                ))
                .size(byteui::theme::font::caption())
                .color(colors.gold)
                .into(),
            ),
            LanguageStatus::Unsupported => Some(
                text(t(
                    "settings.language.unsupported",
                    "This service doesn't support language settings.",
                ))
                .size(byteui::theme::font::caption())
                .color(colors.dim)
                .into(),
            ),
        };

    let mut col = column![title, rows].spacing(14);
    if let Some(line) = status_line {
        col = col.push(line);
    }
    col.into()
}

fn provider_row<'a>(
    provider: GitProvider,
    state: &'a ConnectState,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let t = |key: &str, fallback: &str| crate::i18n::host_text(key, fallback, locale);
    let title = text(provider.display_name())
        .size(byteui::theme::font::body())
        .color(colors.cream);
    match state {
        ConnectState::NotConnected => {
            let status = text(t("settings.git.not_connected", "Not connected"))
                .size(byteui::theme::font::label())
                .color(colors.dim);
            let connect_btn = button(
                text(t("settings.git.connect", "Connect")).size(byteui::theme::font::body()),
            )
            .on_press(Message::ConnectClicked(provider))
            .padding([6, 14])
            .style(crate::extensions::project_create::action_button_hover_style(colors.cream));
            row![title, status, Space::new().width(Length::Fill), connect_btn]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
        }
        ConnectState::Connected { username } => {
            let status = text(crate::i18n::host_text_args(
                "settings.git.connected",
                "Connected: {username}",
                locale,
                &[("username", username.as_str())],
            ))
            .size(byteui::theme::font::label())
            .color(colors.gold);
            let disconnect_btn = button(
                text(t("settings.git.disconnect", "Disconnect")).size(byteui::theme::font::body()),
            )
            .on_press(Message::Disconnect(provider))
            .padding([6, 14])
            .style(crate::extensions::project_create::action_button_hover_style(colors.dim));
            row![
                title,
                status,
                Space::new().width(Length::Fill),
                disconnect_btn
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        }
        ConnectState::Editing { token, busy, error } => {
            let input = byteui::form::input_text::view_on_bg(
                "Personal Access Token",
                token,
                true,
                None,
                false,
                None,
                false,
                move |v| Message::TokenChanged(provider, v),
            );
            let hint = iced_widget::MouseArea::new(
                text(t("settings.git.generate_pat", "No PAT? Generate one"))
                    .size(byteui::theme::font::label())
                    .color(colors.gold),
            )
            .interaction(iced_widget::core::mouse::Interaction::Pointer)
            .on_press(Message::OpenTokenPage(provider));
            let confirm_label = if *busy {
                t("settings.git.validating", "Validating…")
            } else {
                t("settings.git.confirm", "Confirm")
            };
            let confirm = button(text(confirm_label).size(byteui::theme::font::body()))
                .style(crate::extensions::project_create::action_button_hover_style(colors.cream));
            let confirm = if *busy {
                confirm
            } else {
                confirm.on_press(Message::ConnectSubmit(provider))
            };
            let cancel =
                button(text(t("settings.git.cancel", "Cancel")).size(byteui::theme::font::body()))
                    .on_press(Message::ConnectCancel(provider))
                    .style(
                        crate::extensions::project_create::action_button_hover_style(colors.dim),
                    );
            let error_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                if let Some(e) = error {
                    text(crate::i18n::host_render(e, locale))
                        .size(byteui::theme::font::label())
                        .color(colors.red)
                        .into()
                } else {
                    Space::new().into()
                };
            column![
                title,
                input,
                row![hint, Space::new().width(Length::Fill), cancel, confirm].spacing(8),
                error_row,
            ]
            .spacing(6)
            .into()
        }
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
    locale: &'a str,
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
fn settings_tab_button<'a>(
    label: impl Into<String>,
    active: bool,
    _hover_t: f32,
    on_press: Message,
    on_enter: impl Fn(bool) -> Message + 'a,
    on_exit: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let label_el = text(label.into())
        .size(byteui::theme::font::caption())
        .color(if active { colors.cream } else { colors.dim });
    let cell = container(label_el)
        .padding([8, 12])
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(if active { colors.card } else { colors.bg }.into()),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    MouseArea::new(cell)
        .on_press(on_press)
        .on_enter(on_enter(true))
        .on_exit(on_exit(false))
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .into()
}

pub fn settings_card<'a>(
    state: &'a State,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let current = byteui::theme::color::current_scheme();
    let t = |key: &str, fallback: &str| crate::i18n::host_text(key, fallback, locale);
    let theme_title = text(t("settings.tab.theme", "Theme"))
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    let git_title = text(t("settings.git.title", "Git Accounts"))
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    let close = button(
        text(t("settings.close", "Close"))
            .size(byteui::theme::font::label())
            .color(colors.dim),
    )
    .on_press(Message::Close)
    .padding([8, 20])
    .style(crate::extensions::project_create::action_button_hover_style(colors.dim));
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
        t("settings.close", "Close"),
    );

    let advanced_title = text(t("settings.tab.advanced", "Advanced"))
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    // 标题行:settings 图标 + 奶油色标题文本,右上角 X 关闭按钮——与"新建
    // 项目"弹窗同一套 header 结构(图标 + 标题 + 右对齐关闭按钮),
    // 关闭按钮 hover 动画与 project_create 一致。
    let title = row![
        icons::view(
            icons::IconKind::Settings,
            byteui::theme::font::title() as f32,
            colors.cream,
        ),
        text(t("settings.title", "Settings"))
            .size(byteui::theme::font::title())
            .color(colors.cream),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let header =
        row![title, Space::new().width(Length::Fill), close_icon].align_y(Alignment::Center);

    // 左栏三组切换 tab(主题 / Git 账户 / 高级),样式与「新建项目」弹窗左侧
    // `sidebar_entry` 完全一致(选中 CARD 实底、未选中 BG 实底、8 圆角、无
    // 边框、无 hover 配色漂移)。
    let tab_hover_t = |tab: SettingsTab| -> f32 {
        if state.tab_hover == Some(tab) {
            1.0
        } else {
            0.0
        }
    };
    let tab_col = column![
        settings_tab_button(
            t("settings.tab.theme", "Theme"),
            state.selected == SettingsTab::Theme,
            tab_hover_t(SettingsTab::Theme),
            Message::TabSelected(SettingsTab::Theme),
            |h| Message::TabHover(if h { Some(SettingsTab::Theme) } else { None }),
            |_| Message::TabHover(None),
        ),
        settings_tab_button(
            t("settings.tab.git", "Git Accounts"),
            state.selected == SettingsTab::Git,
            tab_hover_t(SettingsTab::Git),
            Message::TabSelected(SettingsTab::Git),
            |h| Message::TabHover(if h { Some(SettingsTab::Git) } else { None }),
            |_| Message::TabHover(None),
        ),
        settings_tab_button(
            t("settings.tab.advanced", "Advanced"),
            state.selected == SettingsTab::Advanced,
            tab_hover_t(SettingsTab::Advanced),
            Message::TabSelected(SettingsTab::Advanced),
            |h| Message::TabHover(if h { Some(SettingsTab::Advanced) } else { None }),
            |_| Message::TabHover(None),
        ),
        settings_tab_button(
            t("settings.tab.apps", "Apps"),
            state.selected == SettingsTab::Apps,
            tab_hover_t(SettingsTab::Apps),
            Message::TabSelected(SettingsTab::Apps),
            |h| Message::TabHover(if h { Some(SettingsTab::Apps) } else { None }),
            |_| Message::TabHover(None),
        ),
        settings_tab_button(
            t("settings.tab.language", "Language"),
            state.selected == SettingsTab::Language,
            tab_hover_t(SettingsTab::Language),
            Message::TabSelected(SettingsTab::Language),
            |h| { Message::TabHover(if h { Some(SettingsTab::Language) } else { None }) },
            |_| Message::TabHover(None),
        ),
    ]
    .spacing(8)
    .width(Length::Fill);

    // 右侧内容区:只渲染左栏选中那一组,其余两组收起(互斥视图切换,与
    // 「新建项目」弹窗的 Tab::Local/Clone 同一思路)。
    let body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = match state
        .selected
    {
        SettingsTab::Theme => column![
            theme_title,
            scheme_row(
                t("settings.theme.dark", "Dark · ByteBoy2077"),
                ColorScheme::Dark,
                current
            ),
            scheme_row(
                t("settings.theme.light", "Light · ByteBoy2077-Light"),
                ColorScheme::Light,
                current
            ),
        ],
        SettingsTab::Git => column![
            git_title,
            provider_row(GitProvider::GitHub, &state.github, locale),
            provider_row(GitProvider::GitLab, &state.gitlab, locale),
            provider_row(GitProvider::Gitee, &state.gitee, locale),
        ],
        SettingsTab::Advanced => column![advanced_title, advanced_row(&state.advanced, locale)],
        SettingsTab::Apps => {
            column![crate::extensions::settings_apps::view(&state.apps, locale).map(Message::Apps)]
        }
        SettingsTab::Language => column![language_page(&state.language, locale)],
    }
    .spacing(14)
    .into();

    // 左栏 tab + 1px 分割线 + 右侧内容区;分割线左右各 16,与内容左缘对齐。
    let main = row![
        container(tab_col).width(Length::Fixed(140.0)),
        container(iced_widget::Space::new())
            .width(Length::Fixed(1.0))
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().border.into()),
                ..container::Style::default()
            }),
        container(body).width(Length::Fill).padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 16.0,
        }),
    ]
    .spacing(16)
    .align_y(iced_widget::core::Alignment::Start)
    .height(Length::Fill);

    let content = column![header, main, byteui::feedback::dialog::actions(row![close])].spacing(14);

    container(content)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(byteui::feedback::dialog::card_style)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试专用构造:`connect_tasks` 是任务生命周期相关的簿记字段,跟这些
    /// 纯状态转换测试无关,统一给默认值,避免每个测试都重复写。
    fn test_state(github: ConnectState, gitlab: ConnectState, gitee: ConnectState) -> State {
        State {
            github,
            gitlab,
            gitee,
            connect_tasks: HashMap::new(),
            advanced: AdvancedState::Idle { error: None },
            selected: SettingsTab::Theme,
            tab_hover: None,
            close_hover: false,
            outbox: Default::default(),
            apps: Default::default(),
            language: LanguageState::default(),
            host_changed: false,
        }
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
        assert_eq!(state.language.pending, Some(pref.clone()));
        assert_eq!(state.language.status, LanguageStatus::Saving);
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
        assert_eq!(state.language.pending, Some(zh));
        assert_eq!(state.language.status, LanguageStatus::Saving);
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
        assert_eq!(state.language.pending, None);
        assert_eq!(state.language.status, LanguageStatus::Idle);
        assert_eq!(state.language.confirmed, zh);
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
        assert_eq!(state.language.pending, Some(zh));
        assert!(matches!(state.language.status, LanguageStatus::Failed(_)));
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
        state.language.confirmed = en.clone();
        apply_sync_message(&mut state, &Message::LanguageSelected(zh));
        // 服务端其实已是 en(revision 被别处推进)。
        apply_sync_message(
            &mut state,
            &Message::LanguageSaveOutcome(LanguageSaveOutcome::Conflict(snap(en.clone(), "en", 9))),
        );
        assert_eq!(state.language.pending, None);
        assert_eq!(state.language.status, LanguageStatus::Conflict);
        assert_eq!(state.language.confirmed, en);
    }

    #[test]
    fn language_unsupported_clears_pending() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.language.confirmed = LocalePreference::System;
        apply_sync_message(
            &mut state,
            &Message::LanguageSelected(LocalePreference::System),
        );
        apply_sync_message(
            &mut state,
            &Message::LanguageSaveOutcome(LanguageSaveOutcome::Unsupported),
        );
        assert_eq!(state.language.pending, None);
        assert_eq!(state.language.status, LanguageStatus::Unsupported);
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
        state.language.confirmed = zh.clone();
        assert_eq!(state.language.shown(), &zh);
    }

    #[test]
    fn connect_clicked_opens_editing_row() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(&mut state, &Message::ConnectClicked(GitProvider::GitHub));
        assert!(matches!(state.github, ConnectState::Editing { .. }));
        assert_eq!(state.gitlab, ConnectState::NotConnected);
    }

    #[test]
    fn token_changed_updates_editing_draft() {
        let mut state = test_state(
            ConnectState::Editing {
                token: String::new(),
                busy: false,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(
            &mut state,
            &Message::TokenChanged(GitProvider::GitHub, "ghp_xxx".into()),
        );
        assert_eq!(
            state.github,
            ConnectState::Editing {
                token: "ghp_xxx".into(),
                busy: false,
                error: None,
            }
        );
    }

    #[test]
    fn connect_cancel_resets_to_not_connected() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(&mut state, &Message::ConnectCancel(GitProvider::GitHub));
        assert_eq!(state.github, ConnectState::NotConnected);
    }

    #[test]
    fn connect_cancel_aborts_in_flight_task_and_discards_late_result() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let join = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        state
            .connect_tasks
            .insert(GitProvider::GitHub, join.abort_handle());
        apply_sync_message(&mut state, &Message::ConnectCancel(GitProvider::GitHub));
        assert_eq!(state.github, ConnectState::NotConnected);
        assert!(!state.connect_tasks.contains_key(&GitProvider::GitHub));
        // 取消之后,哪怕之前那次任务的结果晚一步才送达,也不应该把状态
        // 又悄悄改回 Connected(代码评审 finding:Cancel doesn't stop
        // in-flight connect task)。
        apply_sync_message(
            &mut state,
            &Message::ConnectResult(GitProvider::GitHub, Ok("octocat".into())),
        );
        assert_eq!(state.github, ConnectState::NotConnected);
    }

    #[test]
    fn connect_result_ok_sets_connected() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(
            &mut state,
            &Message::ConnectResult(GitProvider::GitHub, Ok("octocat".into())),
        );
        assert_eq!(
            state.github,
            ConnectState::Connected {
                username: "octocat".into()
            }
        );
    }

    #[test]
    fn connect_result_err_keeps_editing_with_error() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let err = dozer_model::LocalizedText::keyed(
            crate::i18n::HOST_OWNER,
            "error.token_invalid",
            "The token is invalid or has expired",
        );
        apply_sync_message(
            &mut state,
            &Message::ConnectResult(GitProvider::GitHub, Err(err.clone())),
        );
        assert_eq!(
            state.github,
            ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: Some(err),
            }
        );
    }

    #[test]
    fn open_token_page_spawns_without_touching_blur_state() {
        // 失焦即关闭已移除,`OpenTokenPage` 只剩"拉起浏览器"这一个副作用;
        // 这里只验证它被当作已处理消息(返回 true),不再有状态位可断言。
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        assert!(apply_sync_message(
            &mut state,
            &Message::OpenTokenPage(GitProvider::GitHub)
        ));
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

    /// GitHub 已连接、其余未连接的状态(复用上面的 `test_state`,不重复列字段)。
    fn blank_state() -> State {
        test_state(
            ConnectState::Connected {
                username: "octocat".into(),
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        )
    }

    #[test]
    fn disconnect_success_clears_the_slot_and_stays_silent() {
        let mut s = blank_state();
        finish_disconnect(&mut s, GitProvider::GitHub, Ok(()), || Ok(()));
        assert_eq!(s.github, ConnectState::NotConnected);
        assert!(s.take_outbox().is_empty());
    }

    #[test]
    fn disconnect_token_delete_failure_still_disconnects_but_warns() {
        let mut s = blank_state();
        finish_disconnect(&mut s, GitProvider::GitHub, Ok(()), || {
            Err(crate::git_accounts::GitAccountError::KeychainDelete {
                detail: "access denied".to_string(),
            })
        });
        assert_eq!(s.github, ConnectState::NotConnected, "本地记录已断开");
        let got = s.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, crate::extensions::toast::Level::Warning);
        let rendered = crate::i18n::host_render_stable(&got[0].text);
        assert!(rendered.contains("access denied"), "{rendered}");
        assert_eq!(got[0].scope.name, "settings");
    }

    #[test]
    fn disconnect_save_failure_keeps_connected_state_and_tells_the_user() {
        let mut s = blank_state();
        let mut token_delete_called = false;
        finish_disconnect(
            &mut s,
            GitProvider::GitHub,
            Err(std::io::Error::other("disk full")),
            || {
                token_delete_called = true;
                Ok(())
            },
        );
        assert!(
            matches!(s.github, ConnectState::Connected { .. }),
            "写不进本地记录就整个放弃这次断开"
        );
        assert!(
            !token_delete_called,
            "本地记录没写成,不能先把钥匙串 token 删了"
        );
        let got = s.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, crate::extensions::toast::Level::Error);
        assert!(crate::i18n::host_render_stable(&got[0].text).contains("disk full"));
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
