// crates/dozer-app/src/keymap.rs
//! 键盘/IME 输入 → 终端字节序列的纯函数翻译层（P1c T5）。
//!
//! 不依赖 iced 或任何窗口状态，只消费 `winit::keyboard` 的值类型，
//! 因此可以在 headless 环境下直接单测；`main.rs` 在 `WindowEvent`
//! 处理里调用本模块，把结果发给 `Message::TermInput`。
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// 单个键盘事件（`Key` + 修饰键状态）→ 应写入 PTY 的字节序列。
/// `None` 表示这个键本身不产生终端输入（纯功能键/尚未支持的键位）。
///
/// - 字符键：无 Ctrl 时原样 UTF-8；按住 Ctrl 时按字母映射到控制码
///   （Ctrl+A..Z → 0x01..0x1a，例如 Ctrl+C → 0x03 ETX）。
/// - Enter → `\r`；Backspace → `0x7f`（DEL）；Tab → `\t`；Esc → `0x1b`。
/// - 方向键 → CSI 序列 `\x1b[A/B/C/D`（上/下/右/左）。
pub fn key_to_bytes(key: &Key, modifiers: &ModifiersState) -> Option<Vec<u8>> {
    match key {
        Key::Character(s) => {
            if modifiers.control_key() {
                ctrl_char_to_bytes(s)
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        Key::Named(named) => named_key_to_bytes(*named),
        _ => None,
    }
}

/// Ctrl+字母 → 控制码：`(A..Z 的序号 + 1)`，即经典终端 Ctrl 键映射
/// （Ctrl+A=0x01 ... Ctrl+Z=0x1a）。非字母字符暂不支持，返回 `None`。
fn ctrl_char_to_bytes(s: &str) -> Option<Vec<u8>> {
    let c = s.chars().next()?;
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let upper = c.to_ascii_uppercase();
    Some(vec![(upper as u8) - b'A' + 1])
}

fn named_key_to_bytes(named: NamedKey) -> Option<Vec<u8>> {
    match named {
        NamedKey::Enter => Some(vec![b'\r']),
        NamedKey::Backspace => Some(vec![0x7f]),
        NamedKey::Tab => Some(vec![b'\t']),
        NamedKey::Escape => Some(vec![0x1b]),
        NamedKey::ArrowUp => Some(b"\x1b[A".to_vec()),
        NamedKey::ArrowDown => Some(b"\x1b[B".to_vec()),
        NamedKey::ArrowRight => Some(b"\x1b[C".to_vec()),
        NamedKey::ArrowLeft => Some(b"\x1b[D".to_vec()),
        _ => None,
    }
}

/// IME 组合提交的整段文本，原样转 UTF-8 字节（不做控制码翻译——IME
/// 提交内容是已经完成组合的可显示文本，不会是控制字符）。
pub fn ime_commit_to_bytes(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    #[test]
    fn printable_char_passes_utf8() {
        assert_eq!(
            key_to_bytes(&Key::Character("a".into()), &ModifiersState::empty()),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn enter_is_cr_backspace_is_del() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::Enter), &ModifiersState::empty()),
            Some(vec![b'\r'])
        );
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::Backspace), &ModifiersState::empty()),
            Some(vec![0x7f])
        );
    }

    #[test]
    fn arrows_are_csi() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowUp), &ModifiersState::empty()),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowLeft), &ModifiersState::empty()),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn ctrl_c_is_etx() {
        assert_eq!(
            key_to_bytes(&Key::Character("c".into()), &ModifiersState::CONTROL),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ime_commit_is_raw_utf8() {
        assert_eq!(ime_commit_to_bytes("你好"), "你好".as_bytes().to_vec());
    }
}
