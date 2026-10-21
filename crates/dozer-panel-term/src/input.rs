//! Centered key / IME / dropped-file input translation (moved from
//! `dozer-app/src/keymap.rs`, S6c-1).
//!
//! The library must not depend on `winit`, so callers convert their toolkit's
//! key values into the neutral [`TermKey`] / [`TermMods`] first. The byte
//! sequences produced are identical to the legacy `keymap` functions.

use std::path::Path;

/// A neutral key value: either printable text (`Key::Character` in winit) or
/// one of the named keys the legacy table handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermKey {
    Char(String),
    Named(TermNamedKey),
}

/// Named keys covered by the legacy table — no more, no less.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermNamedKey {
    Space,
    Enter,
    Backspace,
    Tab,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowRight,
    ArrowLeft,
}

/// Modifier state at the moment of the key event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TermMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

/// One keyboard event → bytes to write to the PTY. `None` means the key
/// itself produces no terminal input.
///
/// - Character keys: raw UTF-8 without Ctrl; with Ctrl, letters map to control
///   codes (Ctrl+A..Z → 0x01..0x1a) and classic punctuation follows the same
///   convention.
/// - Enter → `\r`; Backspace → `0x7f` (DEL); Tab → `\t`; Esc → `0x1b`.
/// - Arrows: CSI (`\x1b[A`…) in normal mode, SS3 (`\x1bOA`…) when
///   `app_cursor` (DECCKM / application cursor mode) is on.
pub fn key_to_bytes(key: &TermKey, mods: TermMods, app_cursor: bool) -> Option<Vec<u8>> {
    match key {
        TermKey::Char(s) => {
            if mods.ctrl {
                ctrl_char_to_bytes(s)
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        TermKey::Named(TermNamedKey::Space) => {
            // Space is a named key (not `Character(" ")`). Ctrl+Space follows
            // the classic terminal convention and maps to NUL, the same family
            // as Ctrl+A..Z.
            if mods.ctrl {
                Some(vec![0x00])
            } else {
                Some(vec![b' '])
            }
        }
        TermKey::Named(named) => named_key_to_bytes(*named, app_cursor),
    }
}

/// Ctrl+character → control code, classic terminal mapping:
/// - letters: `(A..Z index + 1)` (Ctrl+A=0x01 … Ctrl+Z=0x1a);
/// - punctuation: `[`→ESC(0x1b)、`\`→FS(0x1c)、`]`→GS(0x1d)、`^`→RS(0x1e)、
///   `_`/`/`→US(0x1f)、`@`→NUL;
/// - already-control characters pass through unchanged (the platform layer may
///   have translated Ctrl+C into the control character itself; remapping it as
///   a letter would drop it as "non-alphabetic").
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

fn named_key_to_bytes(named: TermNamedKey, app_cursor: bool) -> Option<Vec<u8>> {
    match named {
        TermNamedKey::Enter => Some(vec![b'\r']),
        TermNamedKey::Backspace => Some(vec![0x7f]),
        TermNamedKey::Tab => Some(vec![b'\t']),
        TermNamedKey::Escape => Some(vec![0x1b]),
        TermNamedKey::ArrowUp => Some(if app_cursor {
            b"\x1bOA".to_vec()
        } else {
            b"\x1b[A".to_vec()
        }),
        TermNamedKey::ArrowDown => Some(if app_cursor {
            b"\x1bOB".to_vec()
        } else {
            b"\x1b[B".to_vec()
        }),
        TermNamedKey::ArrowRight => Some(if app_cursor {
            b"\x1bOC".to_vec()
        } else {
            b"\x1b[C".to_vec()
        }),
        TermNamedKey::ArrowLeft => Some(if app_cursor {
            b"\x1bOD".to_vec()
        } else {
            b"\x1b[D".to_vec()
        }),
        // `Space` never reaches this helper (handled in `key_to_bytes`), but
        // its terminal identity is the space byte.
        TermNamedKey::Space => Some(vec![b' ']),
    }
}

/// IME composition commit text → UTF-8 bytes, unchanged (no control-code
/// translation; IME commits are displayable text).
pub fn ime_commit(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// Dropped file path → shell-escaped input bytes with a trailing space (so
/// consecutive drops concatenate; Terminal.app behaviour). Paths made only of
/// shell-safe characters are emitted as-is; the rest are single-quoted with
/// inner quotes escaped as `'\''`.
pub fn dropped_path_bytes(path: &Path) -> Vec<u8> {
    let path = path.to_string_lossy();
    let shell_safe = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-~+".contains(c));
    let mut s = if shell_safe {
        path.into_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    };
    s.push(' ');
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> TermMods {
        TermMods::default()
    }

    fn ctrl() -> TermMods {
        TermMods {
            ctrl: true,
            ..TermMods::default()
        }
    }

    fn key(ch: &str) -> TermKey {
        TermKey::Char(ch.into())
    }

    #[test]
    fn printable_char_passes_utf8() {
        assert_eq!(key_to_bytes(&key("a"), empty(), false), Some(b"a".to_vec()));
    }

    #[test]
    fn enter_is_cr_backspace_is_del() {
        assert_eq!(
            key_to_bytes(&TermKey::Named(TermNamedKey::Enter), empty(), false),
            Some(vec![b'\r'])
        );
        assert_eq!(
            key_to_bytes(&TermKey::Named(TermNamedKey::Backspace), empty(), false),
            Some(vec![0x7f])
        );
    }

    #[test]
    fn arrows_are_csi() {
        assert_eq!(
            key_to_bytes(&TermKey::Named(TermNamedKey::ArrowUp), empty(), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_to_bytes(&TermKey::Named(TermNamedKey::ArrowLeft), empty(), false),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn ctrl_c_is_etx() {
        assert_eq!(key_to_bytes(&key("c"), ctrl(), false), Some(vec![0x03]));
    }

    #[test]
    fn ctrl_char_already_control_passes_through() {
        // If the platform layer already translated Ctrl+C into the control
        // character itself (logical_key = "\u{3}"), it must pass through
        // unchanged instead of being dropped as non-alphabetic.
        assert_eq!(key_to_bytes(&key("\u{3}"), ctrl(), false), Some(vec![0x03]));
    }

    #[test]
    fn ctrl_punctuation_maps_to_control_codes() {
        for (ch, code) in [("[", 0x1b_u8), ("\\", 0x1c), ("]", 0x1d), ("_", 0x1f)] {
            assert_eq!(
                key_to_bytes(&key(ch), ctrl(), false),
                Some(vec![code]),
                "Ctrl+{ch}"
            );
        }
    }

    #[test]
    fn dropped_path_is_shell_escaped() {
        assert_eq!(
            dropped_path_bytes(Path::new("/tmp/plain")),
            b"/tmp/plain ".to_vec()
        );
        assert_eq!(
            dropped_path_bytes(Path::new("/tmp/a b.txt")),
            b"'/tmp/a b.txt' ".to_vec()
        );
        assert_eq!(
            dropped_path_bytes(Path::new("/tmp/设计 稿.png")),
            "'/tmp/设计 稿.png' ".as_bytes().to_vec()
        );
        assert_eq!(
            dropped_path_bytes(Path::new("/tmp/it's.txt")),
            b"'/tmp/it'\\''s.txt' ".to_vec()
        );
    }

    #[test]
    fn ime_commit_is_raw_utf8() {
        assert_eq!(ime_commit("你好"), "你好".as_bytes().to_vec());
    }

    #[test]
    fn space_maps_to_byte() {
        assert_eq!(
            key_to_bytes(&TermKey::Named(TermNamedKey::Space), empty(), false),
            Some(vec![b' '])
        );
    }

    #[test]
    fn ctrl_space_is_nul() {
        assert_eq!(
            key_to_bytes(&TermKey::Named(TermNamedKey::Space), ctrl(), false),
            Some(vec![0x00])
        );
    }

    #[test]
    fn arrows_are_ss3_when_app_cursor_mode() {
        for (named, seq) in [
            (TermNamedKey::ArrowUp, b"\x1bOA".to_vec()),
            (TermNamedKey::ArrowDown, b"\x1bOB".to_vec()),
            (TermNamedKey::ArrowRight, b"\x1bOC".to_vec()),
            (TermNamedKey::ArrowLeft, b"\x1bOD".to_vec()),
        ] {
            assert_eq!(
                key_to_bytes(&TermKey::Named(named), empty(), true),
                Some(seq),
                "{named:?}"
            );
        }
    }
}
