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
/// - 方向键：`app_cursor` 为 `false`（正常模式）时发 CSI 序列
///   `\x1b[A/B/C/D`；为 `true`（DECCKM/application cursor mode，
///   shell 行编辑器常用）时发 SS3 序列 `\x1bOA/B/C/D`——由调用方传入
///   当前活跃 tab 的 `TerminalModel::app_cursor_mode()`。
pub fn key_to_bytes(key: &Key, modifiers: &ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    match key {
        Key::Character(s) => {
            if modifiers.control_key() {
                ctrl_char_to_bytes(s)
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        Key::Named(NamedKey::Space) => {
            // 空格是具名键（不是 `Key::Character(" ")`），winit 0.30 下
            // 之前完全没有映射分支——空格键因此被静默丢弃。Ctrl+Space
            // 按经典终端惯例映射到 NUL（0x00），与 Ctrl+A..Z 的控制码
            // 序列是同一套约定的延伸。
            if modifiers.control_key() {
                Some(vec![0x00])
            } else {
                Some(vec![b' '])
            }
        }
        Key::Named(named) => named_key_to_bytes(*named, app_cursor),
        _ => None,
    }
}

/// Ctrl+字符 → 控制码，经典终端 Ctrl 键映射：
/// - 字母：`(A..Z 的序号 + 1)`（Ctrl+A=0x01 ... Ctrl+Z=0x1a）；
/// - 标点：`[`→ESC(0x1b)、`\`→FS(0x1c)、`]`→GS(0x1d)、`^`→RS(0x1e)、
///   `_`/`/`→US(0x1f)、`@`→NUL；
/// - 已是控制字符：原样透传（平台层可能已把 Ctrl 组合翻译成控制字符本身，
///   此时再按字母映射会把它当"非字母"丢弃）。
fn ctrl_char_to_bytes(s: &str) -> Option<Vec<u8>> {
    let c = s.chars().next()?;
    if c.is_ascii_control() {
        return Some(vec![c as u8]);
    }
    if c.is_ascii_alphabetic() {
        let upper = c.to_ascii_uppercase();
        return Some(vec![(upper as u8) - b'A' + 1]);
    }
    match c {
        '@' => Some(vec![0x00]),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        '^' => Some(vec![0x1e]),
        '_' | '/' => Some(vec![0x1f]),
        _ => None,
    }
}

fn named_key_to_bytes(named: NamedKey, app_cursor: bool) -> Option<Vec<u8>> {
    match named {
        NamedKey::Enter => Some(vec![b'\r']),
        NamedKey::Backspace => Some(vec![0x7f]),
        NamedKey::Tab => Some(vec![b'\t']),
        NamedKey::Escape => Some(vec![0x1b]),
        NamedKey::ArrowUp => Some(if app_cursor {
            b"\x1bOA".to_vec()
        } else {
            b"\x1b[A".to_vec()
        }),
        NamedKey::ArrowDown => Some(if app_cursor {
            b"\x1bOB".to_vec()
        } else {
            b"\x1b[B".to_vec()
        }),
        NamedKey::ArrowRight => Some(if app_cursor {
            b"\x1bOC".to_vec()
        } else {
            b"\x1b[C".to_vec()
        }),
        NamedKey::ArrowLeft => Some(if app_cursor {
            b"\x1bOD".to_vec()
        } else {
            b"\x1b[D".to_vec()
        }),
        _ => None,
    }
}

/// IME 组合提交的整段文本，原样转 UTF-8 字节（不做控制码翻译——IME
/// 提交内容是已经完成组合的可显示文本，不会是控制字符）。
pub fn ime_commit_to_bytes(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// 拖入终端的文件路径 → shell 转义后的输入字节（尾随一个空格，便于
/// 连续拖入多个路径，Terminal.app 同款行为）。只含 shell 安全字符的
/// 路径原样输出；其余单引号包裹，内部单引号按 `'\''` 转义。
pub fn dropped_path_to_bytes(path: &str) -> Vec<u8> {
    let shell_safe = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-~+".contains(c));
    let mut s = if shell_safe {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    };
    s.push(' ');
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    #[test]
    fn printable_char_passes_utf8() {
        assert_eq!(
            key_to_bytes(&Key::Character("a".into()), &ModifiersState::empty(), false),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn enter_is_cr_backspace_is_del() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Enter),
                &ModifiersState::empty(),
                false
            ),
            Some(vec![b'\r'])
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Backspace),
                &ModifiersState::empty(),
                false
            ),
            Some(vec![0x7f])
        );
    }

    #[test]
    fn arrows_are_csi() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                &ModifiersState::empty(),
                false
            ),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowLeft),
                &ModifiersState::empty(),
                false
            ),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn ctrl_c_is_etx() {
        assert_eq!(
            key_to_bytes(&Key::Character("c".into()), &ModifiersState::CONTROL, false),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ctrl_char_already_control_passes_through() {
        // 平台层若已把 Ctrl+C 翻译成控制字符本身（logical_key = "\u{3}"），
        // 不能再走字母映射（非字母会被丢弃），必须原样透传。
        assert_eq!(
            key_to_bytes(
                &Key::Character("\u{3}".into()),
                &ModifiersState::CONTROL,
                false
            ),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ctrl_punctuation_maps_to_control_codes() {
        for (ch, code) in [("[", 0x1b_u8), ("\\", 0x1c), ("]", 0x1d), ("_", 0x1f)] {
            assert_eq!(
                key_to_bytes(&Key::Character(ch.into()), &ModifiersState::CONTROL, false),
                Some(vec![code]),
                "Ctrl+{ch}"
            );
        }
    }

    #[test]
    fn dropped_path_is_shell_escaped() {
        // 纯净路径原样 + 尾随空格（Terminal.app 同款，便于连续拖多个）
        assert_eq!(dropped_path_to_bytes("/tmp/plain"), b"/tmp/plain ".to_vec());
        // 含空格/中文 → 单引号包裹
        assert_eq!(
            dropped_path_to_bytes("/tmp/a b.txt"),
            b"'/tmp/a b.txt' ".to_vec()
        );
        assert_eq!(
            dropped_path_to_bytes("/tmp/设计 稿.png"),
            "'/tmp/设计 稿.png' ".as_bytes().to_vec()
        );
        // 路径里的单引号 → '\'' 转义
        assert_eq!(
            dropped_path_to_bytes("/tmp/it's.txt"),
            b"'/tmp/it'\\''s.txt' ".to_vec()
        );
    }

    #[test]
    fn ime_commit_is_raw_utf8() {
        assert_eq!(ime_commit_to_bytes("你好"), "你好".as_bytes().to_vec());
    }

    #[test]
    fn space_maps_to_byte() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Space),
                &ModifiersState::empty(),
                false
            ),
            Some(vec![b' '])
        );
    }

    #[test]
    fn ctrl_space_is_nul() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Space),
                &ModifiersState::CONTROL,
                false
            ),
            Some(vec![0x00])
        );
    }

    #[test]
    fn arrows_are_ss3_when_app_cursor_mode() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                &ModifiersState::empty(),
                true
            ),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowDown),
                &ModifiersState::empty(),
                true
            ),
            Some(b"\x1bOB".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowRight),
                &ModifiersState::empty(),
                true
            ),
            Some(b"\x1bOC".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowLeft),
                &ModifiersState::empty(),
                true
            ),
            Some(b"\x1bOD".to_vec())
        );
    }
}
