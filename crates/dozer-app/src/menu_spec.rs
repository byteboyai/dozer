//! 平台无关的菜单内容中间表示。`native_menu`(mac 专属 NSMenu)和
//! `crate::chrome::menu`(iced 弹层，非 mac fallback)如果各自独立组装同一份
//! 菜单数据，容易踩文案/行为漂移的坑——2026-09-17 迁移顶栏"＋新增项目"
//! 菜单时发现，最后一项在 native/iced 两版都写"新建项目"，而"＋"按钮
//! 自己的 tooltip 写"打开项目"，它们触发的其实是同一个
//! `Message::ProjectTabPickFolder`（rfd 文件夹选择器，语义是"打开已有
//! 目录"不是"新建"），菜单文案已统一改成"打开项目"。这类漂移正是同一份
//! 数据手写多遍的产物：菜单内容只在这里组装一次，两个渲染后端各自从
//! 同一份 `MenuSpec` 转换消费。

use byteui::interaction::icons::IconKind;
use iced_widget::core::{Color, Element, Length};

/// 一条菜单内容——跟 `native_menu::Item` 字段完全对应，但不依赖
/// `native_menu` 模块（那个模块整体 `#[cfg(target_os = "macos")]`），
/// 因此这个类型能在所有平台编译，`to_iced` 才能在非 mac 平台使用。
#[derive(Debug, Clone, PartialEq)]
pub enum MenuSpecItem<Msg> {
    Entry {
        icon: Option<IconKind>,
        icon_color: Option<Color>,
        label: String,
        color: Color,
        enabled: bool,
        msg: Msg,
    },
    Separator,
}

/// 一整条菜单的内容。
pub type MenuSpec<Msg> = Vec<MenuSpecItem<Msg>>;

impl<Msg> MenuSpecItem<Msg> {
    /// 常规可点项：图标/文字都用主题 BODY 色。
    pub fn entry(icon: Option<IconKind>, label: impl Into<String>, msg: Msg) -> Self {
        let body = byteui::theme::color::current().body;
        Self::Entry {
            icon,
            icon_color: None,
            label: label.into(),
            color: body,
            enabled: true,
            msg,
        }
    }

    /// 图标独立着色项（agent 选择器等双色菜单用）：文字仍用 BODY 色。
    pub fn entry_tinted(
        icon: IconKind,
        icon_color: Color,
        label: impl Into<String>,
        msg: Msg,
    ) -> Self {
        let body = byteui::theme::color::current().body;
        Self::Entry {
            icon: Some(icon),
            icon_color: Some(icon_color),
            label: label.into(),
            color: body,
            enabled: true,
            msg,
        }
    }

    /// 分组分隔线。
    pub fn separator() -> Self {
        Self::Separator
    }
}

/// 转成 `native_menu::show` 要的原生菜单条目——纯数据搬运，字段一一对应。
#[cfg(target_os = "macos")]
pub fn to_native<Msg>(spec: MenuSpec<Msg>) -> Vec<crate::chrome::native_menu::Item<Msg>> {
    spec.into_iter()
        .map(|item| match item {
            MenuSpecItem::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled,
                msg,
            } => crate::chrome::native_menu::Item::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled,
                msg,
            },
            MenuSpecItem::Separator => crate::chrome::native_menu::Item::Separator,
        })
        .collect()
}

/// 转成 `crate::chrome::menu` 的 iced 弹层——非 mac 平台的 fallback 渲染路径。
/// `enabled: false` 的项走 `menu::item_locked`（不挂 `on_press`，`msg`
/// 字段被丢弃，语义同 native 侧"禁用项点不动"）；`enabled: true` 走
/// `menu::item_row`。图标着色跟 `native_menu::show` 内部同一条规则：
/// `icon_color` 缺省时跟随文字 `color`。
pub fn to_iced<'a, Msg: 'a + Clone>(
    spec: MenuSpec<Msg>,
    width: Length,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let items = spec
        .into_iter()
        .map(|item| match item {
            MenuSpecItem::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled,
                msg,
            } => {
                let tint = icon_color.unwrap_or(color);
                if enabled {
                    crate::chrome::menu::item_row(
                        crate::chrome::menu::icon_leading(icon, tint),
                        label,
                        color,
                        Some(msg),
                    )
                } else {
                    crate::chrome::menu::item_locked(icon, label, color)
                }
            }
            MenuSpecItem::Separator => crate::chrome::menu::separator(),
        })
        .collect();
    crate::chrome::menu::shell_frosted(items, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestMsg {
        A,
        B,
    }

    #[test]
    fn entry_defaults_to_body_color_and_enabled() {
        let item = MenuSpecItem::entry(None, "标签", TestMsg::A);
        assert!(matches!(
            item,
            MenuSpecItem::Entry {
                icon: None,
                icon_color: None,
                enabled: true,
                msg: TestMsg::A,
                ..
            }
        ));
    }

    #[test]
    fn entry_tinted_sets_icon_color_independent_of_text_color() {
        let red = Color::from_rgb(1.0, 0.0, 0.0);
        let item = MenuSpecItem::entry_tinted(IconKind::Trash, red, "删除", TestMsg::B);
        match item {
            MenuSpecItem::Entry {
                icon: Some(IconKind::Trash),
                icon_color: Some(c),
                color,
                ..
            } => {
                assert_eq!(c, red);
                assert_eq!(color, byteui::theme::color::current().body);
            }
            _ => panic!("expected tinted entry"),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn to_native_preserves_order_and_field_values() {
        let spec: MenuSpec<TestMsg> = vec![
            MenuSpecItem::entry(None, "第一项", TestMsg::A),
            MenuSpecItem::separator(),
            MenuSpecItem::entry(None, "第二项", TestMsg::B),
        ];
        let items = to_native(spec);
        assert_eq!(items.len(), 3);
        assert!(matches!(
            items[1],
            crate::chrome::native_menu::Item::Separator
        ));
        assert!(matches!(
            items[2],
            crate::chrome::native_menu::Item::Entry {
                msg: TestMsg::B,
                ..
            }
        ));
    }
}
