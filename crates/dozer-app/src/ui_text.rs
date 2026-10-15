//! 自由流式布局(卡片/详情/设置/列表说明)用的 `text`:与 `iced_widget::text`
//! 唯一区别是默认 `Wrapping::WordOrGlyph`。iced 默认 `Wrapping::Word` 只在空白处
//! 断行,路径、URL、长标识符这类无空格长串放不下就直接外溢出容器(Web 面板的
//! CSS 没有这个问题);`WordOrGlyph` 先按词,词本身超宽时再按字符断。
//!
//! **不要用在固定行高/单行裁切的 UI 上**(文件树行、表格单元格、git 图行、页签、
//! 顶栏、footbar):那里折行会撑破行高,继续用 `iced_widget::text`,需要时显式
//! `.wrapping(Wrapping::None)`。

use iced_widget::Text;
use iced_widget::core::text::{IntoFragment, Renderer, Wrapping};

pub(crate) fn text<'a, Theme, R>(content: impl IntoFragment<'a>) -> Text<'a, Theme, R>
where
    Theme: iced_widget::text::Catalog + 'a,
    R: Renderer,
{
    Text::new(content).wrapping(Wrapping::WordOrGlyph)
}
