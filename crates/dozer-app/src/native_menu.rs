//! macOS 原生右键菜单(NSMenu),替代会被 wry webview 遮挡的 iced 弹层
//! 弹层。详见 `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`。

#![cfg(target_os = "macos")]

use byteui::interaction::icons::IconKind;
use iced_widget::core::Color;

/// 一条原生菜单描述——调用方只管拼数据,不碰 AppKit。
pub enum Item<Msg> {
    Entry {
        icon: Option<IconKind>,
        label: String,
        color: Color,
        enabled: bool,
        msg: Msg,
    },
    Separator,
}

/// 把内嵌 Lucide SVG(`IconKind::bytes()`)按给定颜色栅格化成
/// `size_px × size_px` 的位图——渲染出来的 alpha 通道当遮罩,RGB 统一替换
/// 成 `color`(同 iced 侧 `svg::Style{color}` 的着色语义,忽略 SVG 自身
/// 颜色)。纯函数,不碰 AppKit,可在任何线程/CI 里跑。
fn render_icon_pixmap(kind: IconKind, color: Color, size_px: u32) -> tiny_skia::Pixmap {
    let opt = usvg::Options::default();
    let tree =
        usvg::Tree::from_data(kind.bytes(), &opt).expect("内嵌 Lucide SVG 资源必须能解析");
    let native_size = tree.size().to_int_size();
    let scale = size_px as f32 / native_size.width().max(1) as f32;
    let mut pixmap = tiny_skia::Pixmap::new(size_px, size_px).expect("size_px 非零");
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let (r, g, b) = (
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8,
    );
    for pixel in pixmap.pixels_mut() {
        let a = pixel.alpha();
        *pixel = tiny_skia::PremultipliedColorU8::from_rgba(
            (r as u16 * a as u16 / 255) as u8,
            (g as u16 * a as u16 / 255) as u8,
            (b as u16 * a as u16 / 255) as u8,
            a,
        )
        .expect("premultiply 计算结果 r/g/b <= a 恒成立");
    }
    pixmap
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files 右键菜单用到的图标都能正常栅格化成非空、非全透明的位图——
    /// 不要求逐像素比对(信任 resvg 渲染正确性),只验证"栅格化流程本身
    /// 没有 panic/返回空图",这是把 `render_icon_pixmap` 接进真正
    /// AppKit 代码之前最基本的保障。
    #[test]
    fn render_icon_pixmap_produces_nonempty_bitmap_for_menu_icons() {
        for kind in [
            IconKind::Search,
            IconKind::Trash,
            IconKind::Copy,
            IconKind::ClipboardPaste,
            IconKind::Scissors,
            IconKind::SelectAll,
        ] {
            let pixmap = render_icon_pixmap(kind, Color::WHITE, 16);
            assert_eq!(pixmap.width(), 16);
            assert_eq!(pixmap.height(), 16);
            assert!(
                pixmap.pixels().iter().any(|p| p.alpha() > 0),
                "{kind:?} 栅格化结果不该是全透明的空图"
            );
        }
    }

    /// 着色遮罩语义:同一个图标用两种颜色栅格化,凡是不透明的像素,RGB
    /// 必须跟随传入颜色变化(忽略 SVG 自带颜色,只用其覆盖度当遮罩)——
    /// 对应 `iced` 侧 `svg::Style{color}` 的既有着色语义,原生菜单图标要
    /// 和 iced 里同一批图标看起来一致。
    #[test]
    fn render_icon_pixmap_recolors_by_alpha_mask() {
        let red = render_icon_pixmap(IconKind::Trash, Color::from_rgb(1.0, 0.0, 0.0), 16);
        let blue = render_icon_pixmap(IconKind::Trash, Color::from_rgb(0.0, 0.0, 1.0), 16);
        let idx = red
            .pixels()
            .iter()
            .position(|p| p.alpha() > 200)
            .expect("图标至少有一个接近不透明的像素");
        let red_px = red.pixels()[idx];
        let blue_px = blue.pixels()[idx];
        assert!(red_px.red() > red_px.blue(), "红色着色后该像素应偏红");
        assert!(blue_px.blue() > blue_px.red(), "蓝色着色后该像素应偏蓝");
    }
}
