pub mod card;
pub mod list;
pub mod property;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn all_data_components_construct_without_panic() {
        let _: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            card::view("title", Some("subtitle"), false, false);
        let leading: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            iced_widget::text("*").into();
        let _ = list::item(Some(leading), "label", None);
        let _: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            property::view(vec![("key", "value")]);
    }
}
