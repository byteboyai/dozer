use crate::{
    Message,
    adapter::*,
    i18n::t,
    snapshot::ReviewSource,
    state::WorkspaceState,
    view::{lh, text},
};
use iced_widget::container;
use iced_widget::core::Length;
pub fn load_more_button<'a, M: Clone + 'a>(
    conversation_id: &'a str,
    after_turn_index: i64,
    app: &'a impl PanelDisplay<M>,
) -> iced_widget::core::Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let locale = app.locale();
    let btn = iced_widget::button(
        text(t("load_more", "Load more…", locale))
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(app.wrap(Message::DetailLoadMore(
        conversation_id.to_string(),
        after_turn_index,
    )))
    .padding(6)
    .style(|_t, _s| iced_widget::button::Style {
        background: None,
        text_color: byteui::theme::color::current().dim,
        ..iced_widget::button::Style::default()
    });
    container(btn)
        .width(Length::Fill)
        .align_x(iced_widget::core::Alignment::Center)
        .into()
}

pub fn review_fallback<'a, M: Clone + 'a>(
    mut content: iced_widget::Column<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
    state: &'a WorkspaceState,
    app: &'a impl PanelDisplay<M>,
) -> iced_widget::Column<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let locale = app.locale();
    let Some(rv) = &state.review else {
        return content;
    };
    if let Some(err) = &rv.error {
        return content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().red),
        );
    }
    if rv.entries.is_empty() {
        content = content.push(lh(
            app.pane_style(ConversationsPane::Detail).line_height,
            text(t("review.no_conversation", "No conversation", locale))
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim),
        ));
    }
    // 有内容时:真正的渲染由 review_content_pane 区域叠加的 wry webview
    // 负责(dozer://review-trace/host.html,数据经 dozer://review-trace/
    // data.json 拉取),这里不再手写 iced Column——2026-08-21 webview
    // trace 改造,见
    // docs/superpowers/plans/2026-08-21-review-content-webview-trace.md。
    // 对话面板 session 详情(2026-08-27):条目渲染走 webview,但"加载更多"
    // 按钮补在 iced 滚动体末尾——点它发 `DetailLoadMore`,由库内 update
    // 追加下一页回合,而不是替换首屏。
    let ReviewSource::Conversation(conversation_id) = &rv.source;
    if !rv.entries.is_empty() {
        content = content.push(load_more_button(
            conversation_id,
            rv.entries.len() as i64,
            app,
        ));
    }
    content
}
