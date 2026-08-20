pub mod checkbox;
pub mod input_text;
pub mod select;
pub mod switch;
pub mod text_area;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    #[allow(dead_code)]
    enum Msg {
        Toggled(bool),
        Input(String),
        Selected(&'static str),
        Edited(iced_widget::text_editor::Action),
    }

    #[test]
    fn all_form_components_construct_without_panic() {
        let _ = checkbox::view("label", false, Msg::Toggled);
        let _ = switch::view("label", true, Msg::Toggled);
        let _ = input_text::view(
            "placeholder",
            "value",
            false,
            None,
            false,
            None,
            false,
            Msg::Input,
        );
        let options: &[&str] = &["a", "b"];
        let _ = select::view(options, Some(&"a"), Msg::Selected);
        let content = iced_widget::text_editor::Content::new();
        let _ = text_area::view(&content, "placeholder", Msg::Edited);
    }
}
