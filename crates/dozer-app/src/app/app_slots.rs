//! 应用面板槽(bytehost A3):`PanelKind::App(AppSlot)` 里的 `AppSlot` 是进程内的小整数,
//! 指向一张只增不减的 id 表。这样 `PanelKind` 仍是 `Copy + Hash`,拖拽/悬停/选中等现有代码
//! 原样复用;应用 id 本身(磁盘/落盘/标题用)经 [`AppSlot::id`] 取回。

use std::sync::Mutex;

/// 应用在进程内的槽号(不落盘;落盘用 id 字符串,见 `PanelKind` 的 serde)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppSlot(u16);

static TABLE: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// 宿主侧的 id 形状检查(与 `bytehost-apps` 的 `AppId` 同口径,但 host 不依赖那个 crate):
/// 1–63 个 `[a-z0-9-]`,不以 `-` 开头或结尾。
pub(crate) fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 63
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

impl AppSlot {
    /// 取(必要时分配)`id` 的槽;`id` 形状非法或槽表已满(65536 个)返回 `None`。
    /// 同一个 id 永远得到同一个槽。表只增不减(每个 id 只泄漏一次短字符串)。
    pub(crate) fn intern(id: &str) -> Option<Self> {
        if !valid_app_id(id) {
            return None;
        }
        let mut t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = t.iter().position(|s| *s == id) {
            return Some(Self(i as u16));
        }
        let i = u16::try_from(t.len()).ok()?;
        t.push(Box::leak(id.to_owned().into_boxed_str()));
        Some(Self(i))
    }

    /// 这个槽对应的应用 id。
    pub fn id(self) -> &'static str {
        let t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        t[usize::from(self.0)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_id_same_slot_and_roundtrips() {
        let a = AppSlot::intern("slot-test-a").unwrap();
        let b = AppSlot::intern("slot-test-b").unwrap();
        assert_ne!(a, b);
        assert_eq!(AppSlot::intern("slot-test-a"), Some(a));
        assert_eq!(a.id(), "slot-test-a");
        assert_eq!(b.id(), "slot-test-b");
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in ["", "-a", "a-", "A", "a_b", "a.b", "a/b", &"x".repeat(64)] {
            assert!(AppSlot::intern(bad).is_none(), "{bad:?}");
        }
        assert!(AppSlot::intern(&"x".repeat(63)).is_some());
    }
}
