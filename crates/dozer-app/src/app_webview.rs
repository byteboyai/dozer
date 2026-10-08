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

/// 纯安全策略 crate:origin 导航白名单、存储标识、IPC nonce/注入脚本、存储清除排队。
pub(crate) use bytehost_webview::{
    AppIpc, AppOrigin, StoreRemovalOutcome, StoreRemovals, app_init_script, data_store_identifier,
    new_ipc_nonce, supports_store_removal,
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

    /// 该应用面板的 webview 期望项;没有地址(未运行)返回 `None`。
    pub(crate) fn spec(&self, slot: AppSlot, visible: bool) -> Option<WebviewSpec> {
        self.url(slot)
            .map(|url| app_webview_spec(slot, url, visible))
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

pub(crate) fn app_webview_spec(slot: AppSlot, url: &str, visible: bool) -> WebviewSpec {
    WebviewSpec {
        id: webview_id(slot),
        url: url.to_owned(),
        visible,
        editor_binding: None,
        reserve: None,
        loading_generation: None,
        park_offscreen: false,
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
