//! ⌘V 粘贴截图的兜底路径。
//!
//! `iced_core::clipboard::Clipboard::read` 签名固定是
//! `fn read(&self, kind: Kind) -> Option<String>`——其后端
//! `window_clipboard` 也完全没有图片分支。系统剪贴板里只有图片、没有
//! 文本表示时（例如截图工具直接"拷贝"而非存文件），`main.rs` 里的
//! `clipboard.read` 会静默返回 `None`；⌘ 组合键又一律不透传给 PTY
//! （见 `main.rs` 里 `modifiers.super_key()` 分支的注释），于是粘贴
//! 变成彻底的无操作，PTY 那端（含 claude 等 CLI）什么都收不到。
//!
//! 修法照抄 iTerm2 等终端对"粘贴图片"的通用约定：把剪贴板图片落成
//! 临时 PNG 文件，粘贴这个文件路径的文本；claude 会把粘贴内容识别成
//! 图片文件路径来当附件加载（跟拖拽图片进去等价），不需要 Dozer 自己
//! 实现任何图片协议。

use std::io;
use std::path::PathBuf;

/// 读系统剪贴板里的图片，写成临时 PNG，返回文件路径。剪贴板没有图片、
/// 或读取/编码失败时返回 `None`——调用方（`main.rs`）已经确认过
/// `clipboard.read` 文本路径为空，这里失败就是真的没图，直接放弃。
pub fn read_pasteboard_image_as_temp_file() -> Option<PathBuf> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    write_temp_png(&image.bytes, image.width, image.height).ok()
}

fn write_temp_png(rgba: &[u8], width: usize, height: usize) -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("dozer-paste-{}.png", uuid::Uuid::new_v4()));
    let file = std::fs::File::create(&path)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(rgba).map_err(io::Error::other)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_temp_png_produces_decodable_png_file() {
        // 2x2 白色像素，RGBA8。
        let rgba = vec![255u8; 4 * 2 * 2];
        let path = write_temp_png(&rgba, 2, 2).expect("encode succeeds");

        let bytes = std::fs::read(&path).expect("temp file exists");
        assert_eq!(
            &bytes[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            "文件以 PNG magic bytes 开头"
        );

        let decoder = png::Decoder::new(bytes.as_slice());
        let reader = decoder.read_info().expect("valid png header");
        let info = reader.info();
        assert_eq!((info.width, info.height), (2, 2));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_temp_png_path_is_under_system_temp_dir() {
        let rgba = vec![0u8; 4];
        let path = write_temp_png(&rgba, 1, 1).expect("encode succeeds");
        assert!(path.starts_with(std::env::temp_dir()));
        assert_eq!(path.extension().unwrap(), "png");
        std::fs::remove_file(&path).ok();
    }
}
