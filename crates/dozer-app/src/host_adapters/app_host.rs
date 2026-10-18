//! 应用面板(bytehost A4b2)的**产品侧 Effect 执行器**适配层。
//!
//! 面板状态机本体(轮询/订阅/摘要把戏)已在共享 crate `bytehost-panel::host`
//! ([`crate::extensions::app_host::State`]),与 Digger 共用。状态机吐出的
//! [`crate::extensions::app_host::Effect`] 是**宿主无关**的意图枚举;把它绑到
//! dozer 的取数/WebView/栏位/Toast 上,是**产品侧适配**,天然属于 host 侧适配层,
//! 故放本层而非 `app/app.rs`(面板边界门禁 `R-HOST-EXECUTORS` 要求 `app/` 下不再
//! 出现 `run_<面板>_effects` 执行器;host 只给 `PanelIo`;见
//! `scripts/audit/check_panel_boundary.py`)。
//!
//! 语义与迁移前 `App::run_app_host_effects` **逐字一致**:同一批 Effect、同一
//! `proxy` 回传消息形状、同一 Toast(键去重)、同一"启动地址是秘密,不写日志"约定。

use crate::app::{App, Message};
use crate::extensions::app_host::{Act, Effect, Message as M};
use crate::extensions::toast::Level;

use bytehost_client::AppHostApi;

dozer_core::scope!(LOG, module, "app_host");

/// "连接中断"与"已恢复"共用一个去重键:恢复提示顶替还没消失的中断提示。
const CONNECTION_TOAST_KEY: &str = "app-host-connection";

/// 订阅断线/恢复要弹的 Toast(纯映射,不碰 `App`,便于测试)。
struct ConnectionToast {
    level: Level,
    /// 宿主文案资源键(`locales/host/*.json`)。
    key: &'static str,
    /// 英文兜底。
    fallback: &'static str,
}

/// 状态机的 `ConnectionLost`/`ConnectionRestored` 怎么告诉用户(验收 3.2:运行中 dozerd 被 kill -9 后界面静默)。
///
/// 这里选 Toast;"dozerd 断着"既是事件也是状态,也可以映射成顶栏徽标——状态机只发中立效果,换映射只改这里。
/// `daemon_unavailable`:用户在设置里主动停了 dozerd(或启动就连不上)时顶栏徽标已经在说明,订阅随之断开
/// 不再叠一条"连接中断";"已恢复"不受此限。
fn connection_toast(effect: &Effect, daemon_unavailable: bool) -> Option<ConnectionToast> {
    match effect {
        Effect::ConnectionLost if !daemon_unavailable => Some(ConnectionToast {
            level: Level::Warning,
            key: "app.host_connection_lost",
            fallback: "Lost connection to dozerd, reconnecting…",
        }),
        Effect::ConnectionRestored => Some(ConnectionToast {
            level: Level::Success,
            key: "app.host_connection_restored",
            fallback: "Reconnected to dozerd",
        }),
        _ => None,
    }
}

/// 执行应用面板状态机吐出的副作用:发请求(结果经 `proxy` 回到
/// `Message::AppHost`)、同步 rail、写/清启动地址、弹 Toast。
/// **启动地址是秘密,不写日志。**
pub(crate) fn apply_effects(app: &mut App, effects: Vec<Effect>) {
    for effect in effects {
        match effect {
            Effect::FetchList => {
                let (client, proxy) = (app.client.clone(), app.proxy.clone());
                app.handle.spawn(async move {
                    let result = client.app_list().await;
                    let _ = proxy.send_event(Message::AppHost(M::ListLoaded(result)));
                });
            }
            Effect::FetchLaunchUrl(slot) => {
                let Ok(id) = bytehost_apps::id::AppId::new(slot.id()) else {
                    continue;
                };
                let (client, proxy) = (app.client.clone(), app.proxy.clone());
                app.handle.spawn(async move {
                    let result = client.app_launch_url(id).await;
                    let _ = proxy.send_event(Message::AppHost(M::LaunchUrlLoaded(slot, result)));
                });
            }
            Effect::StartApp(slot) | Effect::StopApp(slot) => {
                let act = if matches!(effect, Effect::StartApp(_)) {
                    Act::Start
                } else {
                    Act::Stop
                };
                let Ok(id) = bytehost_apps::id::AppId::new(slot.id()) else {
                    continue;
                };
                let (client, proxy) = (app.client.clone(), app.proxy.clone());
                app.handle.spawn(async move {
                    let result = match act {
                        Act::Start => client.app_start(id).await.map(|_url| ()),
                        Act::Stop => client.app_stop(id).await,
                    };
                    let _ = proxy.send_event(Message::AppHost(M::ActionDone(slot, act, result)));
                });
            }
            Effect::SyncRail(slots) => app.sync_installed_apps(&slots),
            Effect::SetUrl(slot, url) => app.app_views.set_url(slot, url),
            Effect::ClearUrl(slot) => app.app_views.clear(slot),
            Effect::FetchLogs(slot, max_lines) => {
                let Ok(id) = bytehost_apps::id::AppId::new(slot.id()) else {
                    continue;
                };
                let (client, proxy) = (app.client.clone(), app.proxy.clone());
                app.handle.spawn(async move {
                    let result = client.app_logs(id, max_lines).await;
                    let _ = proxy.send_event(Message::AppHost(M::LogsLoaded(slot, result)));
                });
            }
            Effect::Subscribe => {
                // 订阅成功 → `Subscribed`(随后转 `Changed`/`SubscriptionLost`);
                // 失败(连不上 dozerd、宿主不可用)→ 直接 `SubscriptionLost`,走退避重订阅。
                // 失败只写日志(`module` 来源),不弹 Toast——这是"当前处于某状态",不是一次性事件。
                let (client, proxy) = (app.client.clone(), app.proxy.clone());
                app.handle.spawn(async move {
                    match client.subscribe().await {
                        Ok(mut rx) => {
                            let _ = proxy.send_event(Message::AppHost(M::Subscribed));
                            while let Some(change) = rx.recv().await {
                                let msg = match change {
                                    dozer_client::AppChange::Changed(_)
                                    | dozer_client::AppChange::Resync => M::Changed,
                                    dozer_client::AppChange::Disconnected => M::SubscriptionLost,
                                };
                                if proxy.send_event(Message::AppHost(msg)).is_err() {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            dozer_core::log_warn!(LOG, error = %e, "应用变更订阅失败,将退避重试");
                            let _ = proxy.send_event(Message::AppHost(M::SubscriptionLost));
                        }
                    }
                });
            }
            Effect::OpenSettingsApps => {
                app.settings = Some(crate::extensions::settings::State::load_with_tab(
                    app.daemon_unavailable.is_some(),
                    crate::extensions::settings::SettingsTab::Apps,
                ));
                // 触发「应用」页的数据加载(切到 Apps 页会现拉探测与列表)。
                let _ = app.proxy.send_event(Message::Settings(
                    crate::extensions::settings::Message::TabSelected(
                        crate::extensions::settings::SettingsTab::Apps,
                    ),
                ));
            }
            Effect::ConnectionLost | Effect::ConnectionRestored => {
                if let Some(t) = connection_toast(&effect, app.daemon_unavailable.is_some()) {
                    app.push_toast_keyed(
                        LOG,
                        t.level,
                        dozer_model::LocalizedText::keyed(
                            crate::i18n::HOST_OWNER,
                            t.key,
                            t.fallback,
                        ),
                        CONNECTION_TOAST_KEY,
                    );
                }
            }
            Effect::Notice { level, text, key } => {
                let level = crate::extensions::app_host::notice_level(level);
                app.push_toast_keyed(LOG, level, crate::extensions::toast::literal(text), &key);
            }
            // `PanelEffect` 是 `non_exhaustive`:宿主库新增的效果在这里先忽略并留痕,不让升级编译失败。
            // 不打印效果内容(`SetUrl` 携带启动地址,是秘密)。
            _ => {
                dozer_core::log_warn!(LOG, "收到本版本不认识的应用面板效果,已忽略");
            }
        }
    }
}

/// 应用宿主的消息入口 + 到点轮询:驱动状态机并执行其吐出的副作用。两条调用点
/// (`ResumeTimeReached` 的到点轮询、面板消息)共用本函数,保持与迁移前一致。
pub(crate) fn update(app: &mut App, msg: M) {
    // T6:列表结果里带出每个应用的 `display_i18n`,收进 `AppViews`(外壳标题解析 +
    // WebView 语言桥推送都靠它)。列表是权威来源,全量覆盖。
    if let M::ListLoaded(Ok(summaries)) = &msg {
        app.app_views.sync_display_i18n(summaries);
    }
    let effects = app
        .app_host
        .update(msg, std::time::Instant::now(), &app.visible_app_slots());
    apply_effects(app, effects);
}

/// `ResumeTimeReached` 时调用:到点就拉一次已安装应用列表,并驱动订阅的发起/重订阅与日志刷新。
pub(crate) fn poll_if_due(app: &mut App) {
    let now = std::time::Instant::now();
    let visible = app.visible_app_slots();
    let mut effects = app.app_host.subscribe_if_due(now);
    effects.extend(app.app_host.poll_if_due(now, &visible));
    effects.extend(app.app_host.tick_logs(now, &visible));
    apply_effects(app, effects);
}

#[cfg(test)]
mod tests {
    use super::{CONNECTION_TOAST_KEY, connection_toast};
    use crate::extensions::app_host::Effect;
    use crate::extensions::toast::Level;

    /// 验收 3.2:运行中 dozerd 被 kill -9,界面完全静默。状态机只发中立的 `ConnectionLost`,
    /// 适配层决定怎么告诉用户(这里是带 key 的 Toast)。
    #[test]
    fn a_lost_connection_becomes_a_keyed_warning_toast() {
        let t = connection_toast(&Effect::ConnectionLost, false).expect("要弹提示");
        assert_eq!(t.level, Level::Warning);
        assert_eq!(t.key, "app.host_connection_lost");
        assert_eq!(CONNECTION_TOAST_KEY, "app-host-connection");
    }

    /// 用户在设置里主动停了 dozerd(顶栏徽标已经在说明)时,订阅随之断开不能再叠一条"连接中断"。
    #[test]
    fn a_lost_connection_is_not_announced_when_dozerd_was_stopped_on_purpose() {
        assert!(connection_toast(&Effect::ConnectionLost, true).is_none());
    }

    /// "已恢复"无论徽标在不在都要说:它顶替同 key 的"连接中断"。
    #[test]
    fn a_restored_connection_is_a_success_toast_even_when_the_badge_is_on() {
        for badge in [false, true] {
            let t = connection_toast(&Effect::ConnectionRestored, badge).expect("要弹提示");
            assert_eq!(t.level, Level::Success);
            assert_eq!(t.key, "app.host_connection_restored");
        }
    }

    #[test]
    fn other_effects_do_not_toast() {
        assert!(connection_toast(&Effect::FetchList, false).is_none());
        assert!(connection_toast(&Effect::Subscribe, true).is_none());
    }

    /// 两条文案在中英文资源里都存在(缺 key 会静默回落到英文兜底,用户看到的就不是中文)。
    #[test]
    fn both_connection_messages_exist_in_every_locale() {
        for effect in [Effect::ConnectionLost, Effect::ConnectionRestored] {
            let t = connection_toast(&effect, false).unwrap();
            for locale in ["zh-CN", "en"] {
                let text = crate::i18n::host_text(t.key, "__MISSING__", locale);
                assert_ne!(text, "__MISSING__", "{} 缺 {locale} 文案", t.key);
                assert!(!text.is_empty());
            }
        }
    }
}
