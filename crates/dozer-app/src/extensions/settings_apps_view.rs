//! 设置「应用」页的 iced 视图:直接复用 `bytehost-settings-ui` 的共享管理页。
//!
//! Dozer 只提供三样产品输入:
//! - 现有 `settings.apps.*` 文案解析器(经 `shared_text`,保留中英文资源);
//! - 本产品的日志滚动 `Id`(与 `app_logs` 的贴底跟随共用);
//! - 原始 `bytehost_panel::install::Message`(不做任何重新包装)。
//!
//! 安装状态机、权限文本、忙碌 / 日志 / 错误状态全部在共享状态与共享视图中,
//! 本文件不再保留第二份实现。

use bytehost_panel::install::InstallFlowState as State;
use bytehost_settings_ui::{AppsView, Text};
use iced_widget::core::Element;

use super::settings::{shared_text, with_locale};

pub fn view<'a>(
    state: &'a State,
    locale: &str,
) -> Element<'a, super::settings_apps::Message, iced_widget::Theme, iced_renderer::Renderer> {
    with_locale(locale, || {
        bytehost_settings_ui::apps::view(AppsView {
            state,
            text: Text::new(&shared_text),
            map: &|message| message,
            log_scroll_id: &crate::extensions::app_logs::scroll_id,
            extra_top: Vec::new(),
            extra_bottom: Vec::new(),
            app_notes: None,
            app_leading_actions: None,
        })
    })
}
