//! GUI 侧语言快照缓存(spec 2026-10-12-dozer-i18n-design §4.2)。
//!
//! 只保存**最后一次确认**的 daemon 权威快照,用于 daemon 离线时启动即显示
//! 正确语言。**不是第二个配置真相源**:不参与写入决策,daemon 一旦可读就以
//! daemon 为准覆盖。独立于 daemon 的 `locale_settings.json`(那是权威持久化)。
//!
//! `load()`/`save()` 用固定路径;`load_from`/`save_to` 收显式路径供单测。

use dozer_model::LocaleSnapshot;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("locale_cache.json")
}

/// 读取缓存的最后确认快照;缺失/损坏/版本不符时返回 `None`(调用方回落系统探测)。
pub fn load() -> Option<LocaleSnapshot> {
    load_from(&default_path())
}

pub fn load_from(path: &Path) -> Option<LocaleSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    let snap: LocaleSnapshot = serde_json::from_str(&text).ok()?;
    if snap.schema_version != LocaleSnapshot::SCHEMA_VERSION {
        return None;
    }
    Some(snap)
}

/// 写入缓存(尽力而为;失败只记诊断,不影响运行)。
pub fn save(snapshot: &LocaleSnapshot) {
    save_to(&default_path(), snapshot);
}

pub fn save_to(path: &Path, snapshot: &LocaleSnapshot) {
    let Ok(text) = serde_json::to_string_pretty(snapshot) else {
        return;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        dozer_core::log_warn!(LOG, "创建语言缓存目录失败: {e}");
        return;
    }
    if let Err(e) = std::fs::write(path, text) {
        dozer_core::log_warn!(LOG, "写入语言缓存失败: {e}");
    }
}

dozer_core::scope!(LOG, module, "shell");

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_model::{Locale, LocalePreference};

    fn snap(rev: u64, active: &str) -> LocaleSnapshot {
        LocaleSnapshot {
            schema_version: 1,
            preference: LocalePreference::Explicit {
                locale: Locale(active.into()),
            },
            active_locale: Locale(active.into()),
            revision: rev,
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dz-locale-cache-{tag}-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn roundtrips_snapshot() {
        let p = tmp("roundtrip");
        let s = snap(3, "zh-CN");
        save_to(&p, &s);
        assert_eq!(load_from(&p), Some(s));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_file_is_none() {
        assert_eq!(load_from(&tmp("missing")), None);
    }

    #[test]
    fn corrupt_or_wrong_version_is_none() {
        let p = tmp("corrupt");
        std::fs::write(&p, "{ not json").unwrap();
        assert_eq!(load_from(&p), None);
        std::fs::write(
            &p,
            r#"{"schema_version":999,"preference":{"kind":"system"},"active_locale":"en","revision":0}"#,
        )
        .unwrap();
        assert_eq!(load_from(&p), None, "版本不符不得采信");
        let _ = std::fs::remove_file(&p);
    }
}
