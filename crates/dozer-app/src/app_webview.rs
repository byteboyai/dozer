//! 应用面板的 webview(bytehost A4b1):已安装应用(`PanelKind::App`)在 rail 面板里用 wry 子视图加载
//! `http://<app-id>.localhost:<端口>/`。**应用代码不可信**(第三方或 agent 生成),所以本模块只放三类东西:
//!
//! - 纯判断:这个 URL 是否仍在该应用自己的 origin 内([`AppOrigin::allows_navigation`],导航策略的唯一真相)、
//!   池里的 webview id 段、每应用存储标识 —— 这些**纯策略**都在 `bytehost-webview` crate(Tauri 宿主 Digger 复用同一份);
//! - 状态:每个应用面板"当前要加载的地址"([`AppViews`],A4b2 的状态机往里写);
//! - 期望清单项的构造([`app_webview_spec`])。
//!
//! 真正创建 webview(**不装** `dozer://` 协议、IPC 只留白名单)在 `runtime.rs::build_app_webview`。

use std::collections::HashMap;

use crate::app::{APP_CONTENT_ID_OFFSET, AppSlot};
use crate::preview::WebviewSpec;
use bytehost_apps::proto::{AppDisplayI18n, DisplayTitleScope};

/// 纯安全策略 crate:origin 导航白名单、存储标识、IPC nonce/注入脚本、存储清除排队。
pub(crate) use bytehost_webview::{
    AppIpc, AppOrigin, StoreRemovalOutcome, StoreRemovals, app_init_script, data_store_identifier,
    i18n, new_ipc_nonce, supports_store_removal,
};

/// 池里这个 id 是不是应用 webview(`APP_CONTENT_ID_OFFSET + 槽号`,段内最多 65536 个)。
pub(crate) fn is_app_webview_id(id: usize) -> bool {
    (APP_CONTENT_ID_OFFSET..APP_CONTENT_ID_OFFSET + 65_536).contains(&id)
}

pub(crate) fn webview_id(slot: AppSlot) -> usize {
    APP_CONTENT_ID_OFFSET + slot.index()
}

/// `webview_id` 的反函数;不在应用段内返回 `None`。
pub(crate) fn slot_for_webview_id(id: usize) -> Option<AppSlot> {
    if !is_app_webview_id(id) {
        return None;
    }
    AppSlot::from_index(id - APP_CONTENT_ID_OFFSET)
}

/// 每个应用面板"当前要加载的地址"。地址带一次性令牌(**秘密**):只存在内存里,不写日志、不落盘;
/// 清掉它(应用停了/卸载)就会让池里对应的 webview 被回收。A4b1 里没有写入者,A4b2 的面板状态机写。
#[derive(Debug, Default)]
pub(crate) struct AppViews {
    urls: HashMap<AppSlot, String>,
    /// slot → 应用外壳标题 / 语言桥的 i18n 投影(T6,来自 `AppSummary.display_i18n`)。
    /// 每次 `ListLoaded` 全量覆盖(列表是权威来源);外壳标题按当前 UI 语言解析,
    /// 语言桥据此推首帧 / 变更。
    display_i18n: HashMap<AppSlot, AppDisplayI18n>,
    /// slot → 最近一次**成功**推送到该应用 webview 语言桥的 `display_generation`。
    /// 用于同文档语言变化的幂等重推:仅当 generation 严格大于已推送值时才再推,
    /// 推送失败时留空 → 下一帧重试(有界、无忙循环);旧文档(旧 webview id)不会
    /// 被误推。与「新文档首帧推送」(`locale_push_script`,覆盖 initial/live)区分开。
    locale_pushed: HashMap<AppSlot, u64>,
}

impl AppViews {
    pub(crate) fn set_url(&mut self, slot: AppSlot, url: String) {
        self.urls.insert(slot, url);
    }

    pub(crate) fn clear(&mut self, slot: AppSlot) {
        self.urls.remove(&slot);
    }

    pub(crate) fn url(&self, slot: AppSlot) -> Option<&str> {
        self.urls.get(&slot).map(String::as_str)
    }

    /// 用一份列表结果全量刷新 i18n 投影(`id` 已可解析为 `AppSlot` 才收纳)。
    pub(crate) fn sync_display_i18n(&mut self, summaries: &[bytehost_apps::proto::AppSummary]) {
        self.display_i18n.clear();
        self.locale_pushed.clear();
        for s in summaries {
            if let (Some(slot), Some(di)) = (AppSlot::intern(s.id.as_str()), s.display_i18n.clone())
            {
                self.display_i18n.insert(slot, di);
            }
        }
    }

    /// 按 `locale` 解析后的本地化外壳标题;无投影或解析落空返回 `None`(调用方回落 `app.name`)。
    pub(crate) fn localized_title(&self, slot: AppSlot, locale: &str) -> Option<String> {
        self.display_i18n.get(&slot).and_then(|d| {
            d.resolve_title(&DisplayTitleScope::Presentation, locale)
                .map(str::to_owned)
        })
    }

    /// 该 slot 应用 webview 的**语言桥首帧 / 变更脚本**:`display_i18n` 缺失或 `webview_locale == none`
    /// (不装桥)返回 `None`。脚本经 `evaluate_script` 下发到 `window.__bytehost_i18n_set`(只读桥)。
    ///
    /// 用于「新文档首帧」(`AppWebViewLocaleReady` / navigation 完成),`initial` 与 `live` 都会收到
    /// 当时最新语言;同文档语言变化由 [`Self::live_locale_push_script`] 负责(`initial` 不热改)。
    pub(crate) fn locale_push_script(
        &self,
        slot: AppSlot,
        ui_locale: &str,
        revision: u64,
    ) -> Option<String> {
        let di = self.display_i18n.get(&slot)?;
        if !i18n::should_install_bridge(di.webview_locale) {
            return None;
        }
        let snapshot = i18n::build_i18n_snapshot(di, ui_locale, revision);
        Some(i18n::i18n_set_locale_script(&snapshot))
    }

    /// 同文档语言变化(宿主 `display_generation` 递增)时的推送脚本。仅 `live` 应用接收同文档热更新;
    /// `initial` 只在「新文档」首帧推送(`locale_push_script`),**不**接收同文档变化;`none` 不装桥。
    /// 同时按已推送代次做幂等:`revision` 不大于已成功推送值时返回 `None`(避免重复下发)。
    pub(crate) fn live_locale_push_script(
        &self,
        slot: AppSlot,
        ui_locale: &str,
        revision: u64,
    ) -> Option<String> {
        let di = self.display_i18n.get(&slot)?;
        if di.webview_locale != i18n::WebViewLocale::Live {
            return None;
        }
        if self
            .locale_pushed
            .get(&slot)
            .is_some_and(|&g| g >= revision)
        {
            return None;
        }
        let snapshot = i18n::build_i18n_snapshot(di, ui_locale, revision);
        Some(i18n::i18n_set_locale_script(&snapshot))
    }

    /// 标记某 slot 的语言桥已成功收到 `revision` 代次的快照;失败不应调用,以便下帧重试。
    pub(crate) fn mark_locale_pushed(&mut self, slot: AppSlot, revision: u64) {
        self.locale_pushed.insert(slot, revision);
    }

    /// 该应用面板的 webview 期望项;没有地址(未运行)返回 `None`。
    pub(crate) fn spec(&self, slot: AppSlot, visible: bool) -> Option<WebviewSpec> {
        self.url(slot)
            .map(|url| app_webview_spec(slot, url, visible, self.display_i18n.get(&slot).cloned()))
    }
}

/// 当前 macOS 主版本号(纯策略 crate 不碰 objc2,这一层留给宿主)。
pub(crate) fn host_os_major() -> isize {
    objc2_foundation::NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
}

/// `StoreRemovals::take_ready` 的适配:纯策略按"应用 id"问池里有没有它的 webview,
/// dozer 的池是按槽(`AppSlot`)编号的,这里把 id 折成槽再问。
pub(crate) fn take_store_removals_ready(
    q: &mut StoreRemovals,
    now: std::time::Instant,
    webview_in_pool: impl Fn(AppSlot) -> bool,
) -> Vec<(String, [u8; 16])> {
    q.take_ready(now, |id| AppSlot::intern(id).is_some_and(&webview_in_pool))
}

pub(crate) fn app_webview_spec(
    slot: AppSlot,
    url: &str,
    visible: bool,
    display_i18n: Option<AppDisplayI18n>,
) -> WebviewSpec {
    WebviewSpec {
        id: webview_id(slot),
        url: url.to_owned(),
        visible,
        editor_binding: None,
        reserve: None,
        loading_generation: None,
        surface: None,
        park_offscreen: false,
        display_i18n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview_ids_live_in_their_own_range_and_invert() {
        let slot = AppSlot::intern("wv-ids").unwrap();
        let id = webview_id(slot);
        assert!(is_app_webview_id(id));
        assert_eq!(slot_for_webview_id(id), Some(slot));
        assert!(!is_app_webview_id(crate::app::GROUP_CHAT_CONTENT_ID_OFFSET));
        assert!(!is_app_webview_id(APP_CONTENT_ID_OFFSET - 1));
        assert!(!is_app_webview_id(APP_CONTENT_ID_OFFSET + 65_536));
        assert_eq!(slot_for_webview_id(0), None);
    }

    /// `build_app_webview` 是创建 wry webview 的唯一应用路径,没有廉价夹具能真的建一个;
    /// 这些是它**必须**写出的、且 wry 默认值会反过来的配置,用源码扫描钉住
    /// (wry 0.55 默认:下载放行、debug 下开发者工具开、媒体权限自动批准)。
    #[test]
    fn build_app_webview_pins_the_restrictive_settings() {
        let src = include_str!("runtime.rs");
        let start = src.find("fn build_app_webview(").unwrap();
        let end = start + src[start..].find("\n}\n").unwrap();
        let body = &src[start..end];
        for required in [
            ".with_download_started_handler(|_, _| false)",
            ".with_devtools(false)",
            ".with_new_window_req_handler(",
            ".with_navigation_handler(",
            "AppIpc::parse(",
            // T6:语言桥——初始化脚本按清单 `[i18n].webview_locale` 装桥骨架(第二参),
            // 导航完成后经事件环回传 `AppWebViewLocaleReady` 触发 evaluate 下发真实快照。
            "app_init_script(&nonce,",
            ".with_on_page_load_handler(",
            "Message::AppWebViewLocaleReady(",
        ] {
            assert!(body.contains(required), "缺少 {required}");
        }
        for forbidden in [
            "with_custom_protocol",
            "with_download_completed_handler",
            "APP_INIT_SCRIPT",
        ] {
            assert!(!body.contains(forbidden), "不得出现 {forbidden}");
        }
    }

    /// 浏览器面板的网页是**不可信内容**,但目前和可信预览共用同一个构建器分支(根治是改用 `web_child`,见
    /// `docs/superpowers/plans/2026-10-28-dozer-browser-untrusted-webview.md`)。在那之前,浏览器分支必须自己带上
    /// 这几道闸:导航只放行白名单(否则页面能把自己导航到 `dozer://…` 变成宿主来源)、下载拒绝(这个分支里
    /// wry 默认放行)、`target=_blank` 的地址也过白名单。真机复现:没有这些时 `location.href = "dozer://review-trace/data.json"`
    /// 会成功,地址栏变成 `dozer://…`。
    #[test]
    fn the_browser_webview_branch_gates_navigation_downloads_and_new_windows() {
        let src = include_str!("runtime.rs");
        let start = src
            .find("// 浏览器面板 webview 额外挂")
            .expect("浏览器钩子段落");
        let end = start
            + src[start..]
                .find("// `dozer://` 自定义协议只对可信宿主安装")
                .expect("协议段落");
        let block = &src[start..end];
        for required in [
            ".with_navigation_handler(",
            "allows_browse_navigation(",
            ".with_download_started_handler(|_, _| false)",
            "allows_new_tab(",
        ] {
            assert!(block.contains(required), "浏览器分支缺少 {required}");
        }
        // 白名单必须在导航处理器里真的被调用,而不是只出现在注释里。
        let nav = &block[block.find(".with_navigation_handler(").unwrap()..];
        assert!(
            nav[..nav.len().min(300)].contains("allows_browse_navigation("),
            "导航处理器必须调用白名单"
        );
        // 新窗口请求:只有白名单放行的地址才发 `BrowserNewWindow`。
        let win = &block[block.find(".with_new_window_req_handler(").unwrap()..];
        let allow_at = win.find("allows_new_tab(").expect("新窗口白名单");
        let send_at = win.find("BrowserNewWindow").expect("新窗口消息");
        assert!(allow_at < send_at, "必须先过白名单再发 BrowserNewWindow");
    }

    /// T6:`AppViews` 捕获 `display_i18n` 后,外壳标题按 locale 解析、语言桥推送脚本按
    /// `build_i18n_snapshot` + `i18n_set_locale_script` 生成;`webview_locale = none` 不装桥。
    #[test]
    fn display_i18n_is_captured_resolved_and_pushed() {
        use bytehost_apps::i18n::WebViewLocale;
        use bytehost_apps::id::{AppId, Version};
        use bytehost_apps::proto::{
            AppDisplayI18n, AppDisplayTitle, AppSummary, DisplayTitleScope,
        };
        use bytehost_apps::state::{DesiredState, ObservedState};
        use std::collections::BTreeMap;

        let slot = AppSlot::intern("i18n-app").unwrap();
        let di = AppDisplayI18n {
            default_locale: "en".into(),
            supported_locales: vec!["en".into(), "zh-CN".into()],
            webview_locale: WebViewLocale::Live,
            titles: vec![AppDisplayTitle {
                scope: DisplayTitleScope::Presentation,
                key: "app.title".into(),
                fallback: "Notes".into(),
                values: [
                    ("en".to_string(), "Notes".to_string()),
                    ("zh-CN".to_string(), "笔记".to_string()),
                ]
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            }],
        };
        let summary = AppSummary {
            id: AppId::new("i18n-app").unwrap(),
            name: "i18n-app".into(),
            version: Version::new(1, 0, 0),
            desired: DesiredState::Stopped,
            observed: ObservedState::Stopped,
            url: None,
            issue: None,
            previous_version: None,
            rollback_note: None,
            display_i18n: Some(di.clone()),
        };
        let mut views = AppViews::default();
        views.sync_display_i18n(std::slice::from_ref(&summary));
        // 外壳标题按当前 locale 解析。
        assert_eq!(
            views.localized_title(slot, "zh-CN").as_deref(),
            Some("笔记")
        );
        assert_eq!(views.localized_title(slot, "en").as_deref(), Some("Notes"));
        // 语言桥首帧/变更脚本:JSON 数据注入,可解析回等价快照。
        let script = views
            .locale_push_script(slot, "zh-CN", 7)
            .expect("live 应产生推送脚本");
        assert!(script.contains("window.__bytehost_i18n_set("));
        assert!(script.contains("\"ui_locale\":\"zh-CN\""));
        assert!(script.contains("\"app_locale\":\"zh-CN\""));
        assert!(script.contains("\"locale_revision\":\"7\""));
        // `webview_locale = none` 不装桥:产生不了脚本。
        let di_none = AppDisplayI18n {
            webview_locale: WebViewLocale::None,
            ..di
        };
        views.sync_display_i18n(&[AppSummary {
            id: AppId::new("i18n-app").unwrap(),
            display_i18n: Some(di_none),
            ..summary.clone()
        }]);
        assert!(
            views.locale_push_script(slot, "zh-CN", 7).is_none(),
            "none 不应装桥"
        );
        // 空列表清掉投影。
        views.sync_display_i18n(&[]);
        assert!(views.localized_title(slot, "zh-CN").is_none());
    }

    #[test]
    fn views_produce_a_spec_only_while_an_address_is_set() {
        let slot = AppSlot::intern("wv-views").unwrap();
        let mut views = AppViews::default();
        assert!(views.spec(slot, true).is_none());
        views.set_url(slot, "http://wv-views.localhost:20001/?bh_token=t".into());
        let spec = views.spec(slot, false).unwrap();
        assert_eq!(spec.id, webview_id(slot));
        assert!(!spec.visible);
        assert!(spec.editor_binding.is_none() && spec.reserve.is_none());
        views.clear(slot);
        assert!(views.spec(slot, true).is_none());
    }

    /// 应用 webview 的导航策略与每应用存储标识只在 `bytehost-webview` 里有**一份**:dozer 宿主不得
    /// 再抄一份(`allows_navigation` 与 `data_store_identifier` 都从该 crate 引入,见本文件顶部的 `use`)。
    #[test]
    fn no_second_copy_of_the_navigation_policy() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // 本文件是这条门禁的宿主,自身含这两个字面量(在断言里)——跳过它。
        let this = std::path::Path::new(file!()).file_name();
        let mut checked = 0;
        for entry in walk_rs(&root) {
            if entry.file_name() == this {
                continue;
            }
            let src = std::fs::read_to_string(&entry).unwrap();
            assert!(
                !src.contains("fn allows_navigation"),
                "{} 里出现了导航策略的第二份实现;唯一真相在 bytehost-webview",
                entry.display()
            );
            assert!(
                !src.contains("fn data_store_identifier"),
                "{} 里出现了每应用存储标识的第二份实现;唯一真相在 bytehost-webview",
                entry.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "没有扫到任何源文件,测试本身失效");
    }

    fn walk_rs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walk_rs(&path));
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        out
    }
}
