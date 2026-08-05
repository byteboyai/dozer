// crates/dozer-app/src/fonts.rs
//! 字体库消毒（T7 验收反馈修复）。
//!
//! macOS 自带的 "GB18030 Bitmap"（`NISC18030.ttf`）是纯位图字体，
//! cosmic-text 解析它的字形 advance 会得到 `inf`；而新版 macOS 把
//! PingFang 等主力中文字体移进了系统资产目录（fontdb 索引不到按名回退
//! 的首选项），CJK 回退经常撞上这颗毒字体——一旦命中，该行从第一个中文
//! 字符起所有字形都被排到 x=inf，整行"消失/乱掉"。
//!
//! 对策：应用启动时把它从本进程的 fontdb 里剔除。字体库里仍有 Hiragino
//! Sans GB / Heiti / Songti / Arial Unicode MS 等大量可缩放 CJK 字体可供
//! 回退，剔除只影响本进程渲染，不动系统。

/// 内嵌的 JetBrains Mono（variable 字体，SIL OFL 授权，可随程序分发）。
/// 经 `include_bytes!` 编译进二进制，macOS 打包无需额外拷贝资源。
/// 代码/终端字体统一走它（见 `code_font`）；UI 字体保持系统默认，不用它。
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono[wght].ttf");

/// 代码/终端字体的字族名（与字体文件内声明的 family 一致）。
pub const CODE_FONT_FAMILY: &str = "JetBrains Mono";

/// 把内嵌的 JetBrains Mono 注册进本进程的 cosmic-text fontdb，使
/// `Family::Name("JetBrains Mono")` 可被 `code_font`/`term_view` 解析。
/// 必须在第一次文本排版之前调用（`main()` 入口处，与 `sanitize_font_db`
/// 一起）。重复调用无害——同一份字节重复加载只是覆盖同名 face。
pub fn load_embedded_fonts() {
    let mut font_system = iced_wgpu::graphics::text::font_system()
        .write()
        .expect("锁定 font system");
    let db = font_system.raw().db_mut();
    db.load_font_data(JETBRAINS_MONO.to_vec());
}

/// 代码/终端字体的统一描述符：JetBrains Mono（variable，bold 等变体由
/// 调用方设 `weight` 走 wght 轴）。所有"等宽代码"场景的唯一真相源——
/// 当前终端(`term_view::cell_font`)使用它；未来代码展示面板也用它。
pub fn code_font() -> iced_widget::core::Font {
    iced_widget::core::Font::with_name(CODE_FONT_FAMILY)
}

/// 从本进程的 cosmic-text fontdb 里剔除已知会毒化 CJK 回退的位图字体。
/// 必须在第一次文本排版之前调用（`main()` 入口处）；重复调用无害。
pub fn sanitize_font_db() {
    let mut font_system = iced_wgpu::graphics::text::font_system()
        .write()
        .expect("锁定 font system");
    let db = font_system.raw().db_mut();
    let poisoned: Vec<_> = db
        .faces()
        .filter(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.contains("GB18030"))
        })
        .map(|face| face.id)
        .collect();
    for id in poisoned {
        tracing::info!(?id, "剔除 GB18030 Bitmap 位图字体（CJK 回退毒源）");
        db.remove_face(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure_width(content: &str) -> f32 {
        use iced_wgpu::core::text::{Paragraph as _, Shaping, Text, Wrapping};
        use iced_wgpu::core::{Font, Pixels, Size};
        use iced_wgpu::graphics::text::Paragraph;

        let p = Paragraph::with_text(Text {
            content,
            bounds: Size::new(10_000.0, 10_000.0),
            size: Pixels(13.0),
            line_height: 1.4.into(),
            font: Font::MONOSPACE,
            align_x: Default::default(),
            align_y: iced_wgpu::core::alignment::Vertical::Top,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
        });
        p.min_bounds().width
    }

    #[test]
    fn cjk_advance_is_finite_after_sanitize() {
        sanitize_font_db();
        let w = measure_width("a你好b");
        assert!(w.is_finite(), "CJK 行宽仍为非有限值: {w}");
        assert!(w > 0.0);
    }

    #[test]
    fn gb18030_bitmap_is_removed_from_db() {
        sanitize_font_db();
        let mut fs = iced_wgpu::graphics::text::font_system()
            .write()
            .expect("font system");
        let poisoned = fs
            .raw()
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n.contains("GB18030")));
        assert!(!poisoned, "GB18030 Bitmap 仍在 fontdb 中");
    }

    #[test]
    fn jetbrains_mono_is_registered_after_load() {
        load_embedded_fonts();
        let mut fs = iced_wgpu::graphics::text::font_system()
            .write()
            .expect("font system");
        let registered = fs.raw().db().faces().any(|f| {
            f.families
                .iter()
                .any(|(name, _)| name.contains("JetBrains Mono"))
        });
        assert!(registered, "JetBrains Mono 未注册进 fontdb");
    }

    #[test]
    fn code_font_is_jetbrains_mono() {
        use iced_widget::core::font::Family;
        assert_eq!(code_font().family, Family::Name("JetBrains Mono"));
    }
}
