//! 系统钥匙串写入的可注入抽象。`keyring` 4.x 没有可用的 mock 后端(全局替换存储会
//! 牵连别的测试),所以把"写/删一条凭据"抽成 `SecretStore`:生产走 `KeyringStore`,
//! 测试用 `fake::FakeStore`,失败路径因此可测。
//!
//! 此前 SSH/数据库面板对钥匙串的写入与删除都是 `let _ = entry.set_password(..)`,
//! 失败时界面显示"已保存",之后连接报一个莫名其妙的认证错误。

use crate::extensions::toast::{Level, Outbox};
use dozer_core::log::Scope;

/// 一条凭据在钥匙串里的定位。
pub(crate) struct SecretRef<'a> {
    pub service: &'a str,
    pub account: &'a str,
}

pub(crate) trait SecretStore {
    fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String>;
    /// 条目本来就不存在视为成功:密钥认证的主机、没填密码的数据源从没存过密码,
    /// 删除它们不该报错(同 `git_accounts::delete_token` 的既有口径)。
    fn delete(&self, at: &SecretRef<'_>) -> Result<(), String>;
}

/// 生产实现:走系统钥匙串(`keyring`)。
pub(crate) struct KeyringStore;

impl SecretStore for KeyringStore {
    fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String> {
        keyring::Entry::new(at.service, at.account)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| e.to_string())
    }

    fn delete(&self, at: &SecretRef<'_>) -> Result<(), String> {
        match keyring::Entry::new(at.service, at.account) {
            Ok(entry) => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        }
    }
}

/// 保存一条凭据;失败推 **Error** 提示(用户否则以为已保存,之后连接报认证错误)。
/// 文案只含底层错误原因,**不含秘密本身**。
pub(crate) fn save(
    store: &dyn SecretStore,
    outbox: &mut Outbox,
    scope: Scope,
    at: &SecretRef<'_>,
    secret: &str,
) {
    if let Err(e) = store.set(at, secret) {
        outbox.push(
            scope,
            Level::Error,
            format!("密码未能保存到系统钥匙串: {e}"),
        );
    }
}

/// 删除一条凭据;失败推 **Warning**(界面已显示删除,钥匙串里却还留着密钥)。
pub(crate) fn remove(
    store: &dyn SecretStore,
    outbox: &mut Outbox,
    scope: Scope,
    at: &SecretRef<'_>,
) {
    if let Err(e) = store.delete(at) {
        outbox.push(
            scope,
            Level::Warning,
            format!("系统钥匙串里的凭据未能删除: {e}"),
        );
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::cell::RefCell;

    /// 记录每次调用并返回预设结果的假钥匙串。
    pub(crate) struct FakeStore {
        pub(crate) set_result: Result<(), String>,
        pub(crate) delete_result: Result<(), String>,
        pub(crate) calls: RefCell<Vec<String>>,
    }

    impl FakeStore {
        pub(crate) fn ok() -> Self {
            Self {
                set_result: Ok(()),
                delete_result: Ok(()),
                calls: RefCell::new(Vec::new()),
            }
        }

        pub(crate) fn failing(reason: &str) -> Self {
            Self {
                set_result: Err(reason.to_string()),
                delete_result: Err(reason.to_string()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl SecretStore for FakeStore {
        fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("set {}/{} {}", at.service, at.account, secret));
            self.set_result.clone()
        }

        fn delete(&self, at: &SecretRef<'_>) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("delete {}/{}", at.service, at.account));
            self.delete_result.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeStore;
    use super::*;

    dozer_core::scope!(TEST_SSH, panel, "ssh");

    fn at() -> SecretRef<'static> {
        SecretRef {
            service: "dozer-ssh",
            account: "7:h1",
        }
    }

    #[test]
    fn save_ok_pushes_nothing_and_calls_the_store_once() {
        let store = FakeStore::ok();
        let mut outbox = Outbox::default();
        save(&store, &mut outbox, TEST_SSH, &at(), "pw-secret");
        assert!(outbox.take().is_empty());
        assert_eq!(*store.calls.borrow(), vec!["set dozer-ssh/7:h1 pw-secret"]);
    }

    #[test]
    fn save_failure_is_an_error_toast_that_never_contains_the_secret() {
        let store = FakeStore::failing("钥匙串已锁定");
        let mut outbox = Outbox::default();
        save(&store, &mut outbox, TEST_SSH, &at(), "pw-secret");
        let got = outbox.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, Level::Error);
        assert_eq!(got[0].scope, TEST_SSH);
        assert_eq!(got[0].text, "密码未能保存到系统钥匙串: 钥匙串已锁定");
        assert!(!got[0].text.contains("pw-secret"), "文案不得含秘密本身");
    }

    #[test]
    fn remove_ok_pushes_nothing() {
        let store = FakeStore::ok();
        let mut outbox = Outbox::default();
        remove(&store, &mut outbox, TEST_SSH, &at());
        assert!(outbox.take().is_empty());
        assert_eq!(*store.calls.borrow(), vec!["delete dozer-ssh/7:h1"]);
    }

    #[test]
    fn remove_failure_is_a_warning_not_an_error() {
        let store = FakeStore::failing("拒绝访问");
        let mut outbox = Outbox::default();
        remove(&store, &mut outbox, TEST_SSH, &at());
        let got = outbox.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, Level::Warning);
        assert_eq!(got[0].text, "系统钥匙串里的凭据未能删除: 拒绝访问");
    }

    /// 真钥匙串:删除一个不存在的条目必须当成功(密钥认证的主机从没存过密码)。
    /// 会访问系统钥匙串,默认不跑;验收时手工 `cargo test -p dozer-app -- --ignored
    /// keyring_store_delete_of_missing_entry_is_ok`。
    #[test]
    #[ignore = "touches the real system keychain"]
    fn keyring_store_delete_of_missing_entry_is_ok() {
        let account = format!("dozer-test-never-exists-{}", std::process::id());
        let at = SecretRef {
            service: "dozer-test",
            account: &account,
        };
        assert_eq!(KeyringStore.delete(&at), Ok(()));
    }
}
