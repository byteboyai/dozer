//! 键盘/IME/拖入文件 → 终端字节序列的薄适配层(S6c-1 抽库后)。
//!
//! 纯函数与测试都在共享库 `dozer-panel-term::input`;本文件只负责把
//! `winit::keyboard` 的值翻译成库的中立类型,对外签名与抽库前一致,
//! `platform/window_events.rs` 的调用点不需要改动。

use dozer_panel_term::input::{self, TermKey, TermMods, TermNamedKey};
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// 单个键盘事件 → 应写入 PTY 的字节序列。`None` 表示这个键本身不产生
/// 终端输入(纯功能键/尚未支持的键位)。行为与抽库前逐字一致。
pub fn key_to_bytes(key: &Key, modifiers: &ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    let key = match key {
        Key::Character(s) => TermKey::Char(s.to_string()),
        Key::Named(named) => TermKey::Named(term_named_key(*named)?),
        _ => return None,
    };
    input::key_to_bytes(&key, term_mods(modifiers), app_cursor)
}

/// IME 组合提交文本 → UTF-8 字节。
pub fn ime_commit_to_bytes(text: &str) -> Vec<u8> {
    input::ime_commit(text)
}

/// 拖入终端的路径 → shell 转义后的输入字节(尾随空格)。
pub fn dropped_path_to_bytes(path: &str) -> Vec<u8> {
    input::dropped_path_bytes(std::path::Path::new(path))
}

/// winit 具名键 → 库的中立具名键。**只映射旧表处理过的键**;其余返回
/// `None`(与旧 `named_key_to_bytes` 的 `_ => None` 等价)。
pub(crate) fn term_named_key(named: NamedKey) -> Option<TermNamedKey> {
    match named {
        NamedKey::Space => Some(TermNamedKey::Space),
        NamedKey::Enter => Some(TermNamedKey::Enter),
        NamedKey::Backspace => Some(TermNamedKey::Backspace),
        NamedKey::Tab => Some(TermNamedKey::Tab),
        NamedKey::Escape => Some(TermNamedKey::Escape),
        NamedKey::ArrowUp => Some(TermNamedKey::ArrowUp),
        NamedKey::ArrowDown => Some(TermNamedKey::ArrowDown),
        NamedKey::ArrowRight => Some(TermNamedKey::ArrowRight),
        NamedKey::ArrowLeft => Some(TermNamedKey::ArrowLeft),
        _ => None,
    }
}

fn term_mods(modifiers: &ModifiersState) -> TermMods {
    TermMods {
        ctrl: modifiers.control_key(),
        alt: modifiers.alt_key(),
        shift: modifiers.shift_key(),
        super_key: modifiers.super_key(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧表处理过的每个 winit 具名键都必须有对应的中立具名键——否则
    /// 适配层会静默丢弃该键。
    #[test]
    fn every_winit_named_key_handled_by_the_legacy_table_has_a_term_named_key() {
        for named in [
            NamedKey::Space,
            NamedKey::Enter,
            NamedKey::Backspace,
            NamedKey::Tab,
            NamedKey::Escape,
            NamedKey::ArrowUp,
            NamedKey::ArrowDown,
            NamedKey::ArrowRight,
            NamedKey::ArrowLeft,
        ] {
            assert!(term_named_key(named).is_some(), "{named:?} 被静默丢弃");
            assert!(
                key_to_bytes(&Key::Named(named), &ModifiersState::empty(), false).is_some(),
                "{named:?} 不产生终端输入"
            );
        }
        // 旧表没有的键仍然不产生输入。
        assert!(term_named_key(NamedKey::F5).is_none());
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::F5), &ModifiersState::empty(), false),
            None
        );
    }

    /// 适配层对一组键位矩阵的输出与旧实现的期望值逐项一致(期望值是
    /// 在抽库前的旧实现上跑出后写死的)。
    #[test]
    fn winit_adapter_matches_the_legacy_function_for_a_key_matrix() {
        let empty = ModifiersState::empty();
        let ctrl = ModifiersState::CONTROL;
        let shift = ModifiersState::SHIFT;
        let alt = ModifiersState::ALT;
        let cases: Vec<(Key, ModifiersState, bool, Option<Vec<u8>>)> = vec![
            (
                Key::Character("a".into()),
                empty,
                false,
                Some(b"a".to_vec()),
            ),
            (
                Key::Character("A".into()),
                shift,
                false,
                Some(b"A".to_vec()),
            ),
            (Key::Character("c".into()), ctrl, false, Some(vec![0x03])),
            (
                Key::Character("\u{3}".into()),
                ctrl,
                false,
                Some(vec![0x03]),
            ),
            (Key::Character("[".into()), ctrl, false, Some(vec![0x1b])),
            (Key::Character("\\".into()), ctrl, false, Some(vec![0x1c])),
            (Key::Character("]".into()), ctrl, false, Some(vec![0x1d])),
            (Key::Character("_".into()), ctrl, false, Some(vec![0x1f])),
            (Key::Character("b".into()), alt, false, Some(b"b".to_vec())),
            (Key::Named(NamedKey::Space), empty, false, Some(vec![b' '])),
            (Key::Named(NamedKey::Space), ctrl, false, Some(vec![0x00])),
            (Key::Named(NamedKey::Enter), empty, false, Some(vec![b'\r'])),
            (
                Key::Named(NamedKey::Backspace),
                empty,
                false,
                Some(vec![0x7f]),
            ),
            (Key::Named(NamedKey::Tab), empty, false, Some(vec![b'\t'])),
            (Key::Named(NamedKey::Escape), empty, false, Some(vec![0x1b])),
            (
                Key::Named(NamedKey::ArrowUp),
                empty,
                false,
                Some(b"\x1b[A".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowDown),
                empty,
                false,
                Some(b"\x1b[B".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowRight),
                empty,
                false,
                Some(b"\x1b[C".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowLeft),
                empty,
                false,
                Some(b"\x1b[D".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowUp),
                empty,
                true,
                Some(b"\x1bOA".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowDown),
                empty,
                true,
                Some(b"\x1bOB".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowRight),
                empty,
                true,
                Some(b"\x1bOC".to_vec()),
            ),
            (
                Key::Named(NamedKey::ArrowLeft),
                empty,
                true,
                Some(b"\x1bOD".to_vec()),
            ),
            (Key::Named(NamedKey::F5), empty, false, None),
        ];
        for (key, mods, app_cursor, expected) in cases {
            assert_eq!(
                key_to_bytes(&key, &mods, app_cursor),
                expected,
                "{key:?} mods={mods:?} app_cursor={app_cursor}"
            );
        }
        assert_eq!(ime_commit_to_bytes("你好"), "你好".as_bytes().to_vec());
        assert_eq!(
            dropped_path_to_bytes("/tmp/a b.txt"),
            b"'/tmp/a b.txt' ".to_vec()
        );
    }
}
