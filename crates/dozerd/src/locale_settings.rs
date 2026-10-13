//! 界面语言设置的持久化(spec 2026-10-12-dozer-i18n-design §4.2)。
//!
//! daemon 是唯一写入者;快照落在 `config_dir()/locale_settings.json`。写入用
//! "同目录临时文件 + rename"的原子替换,进程内用 `Mutex` 串行化,`revision`
//! 单调递增供客户端做 CAS。文件缺失按默认快照;文件损坏/不可读时记日志并回落
//! 默认快照——设置坏了不该让 daemon 起不来,下次写入会覆盖修复。

use anyhow::{Context, Result};
use dozer_core::protocol::{Locale, LocalePreference, LocaleSnapshot, LocaleWriteOutcome};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

dozer_core::scope!(LOG, module, "locale_settings");

pub struct LocaleSettingsStore {
    path: PathBuf,
    inner: Mutex<LocaleSnapshot>,
}

impl LocaleSettingsStore {
    /// 打开(或初始化)持久化文件。缺失 → 默认快照;损坏/不可读 → 记日志并回落默认。
    pub fn open(path: &Path) -> Result<Self> {
        let snapshot = match std::fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<LocaleSnapshot>(&raw) {
                Ok(s) => s,
                Err(e) => {
                    dozer_core::log_warn!(
                        LOG,
                        error = %e,
                        path = %path.display(),
                        "语言设置文件损坏,回落默认快照"
                    );
                    LocaleSnapshot::default_snapshot()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                LocaleSnapshot::default_snapshot()
            }
            Err(e) => {
                dozer_core::log_warn!(
                    LOG,
                    error = %e,
                    path = %path.display(),
                    "语言设置文件不可读,回落默认快照"
                );
                LocaleSnapshot::default_snapshot()
            }
        };
        Ok(Self {
            path: path.to_path_buf(),
            inner: Mutex::new(snapshot),
        })
    }

    /// 默认路径:`config_dir()/locale_settings.json`。
    pub fn default_path() -> PathBuf {
        dozer_core::paths::config_dir().join("locale_settings.json")
    }

    /// 当前权威快照。`Poisoned` 时取内层值,不让一次 panic 永久废掉设置。
    pub fn get(&self) -> LocaleSnapshot {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 提交一次设置变更(CAS by `revision`)。
    ///
    /// - `expected_revision` 与当前不符 → `Conflict`,带最新快照,不改动任何状态;
    /// - 相符但偏好与解析结果都无变化 → 直接回 `Applied`(当前快照),不落盘、不推进
    ///   revision(避免无意义轮询写盘);
    /// - 相符且有变化 → 计算 `active_locale`(`System` 用 `system_candidates` 解析,
    ///   `Explicit` 用其携带语言),`revision + 1`,原子落盘,成功才更新内存并返回 `Applied`。
    ///
    /// 落盘失败返回 `Err`,内存保持旧值,避免"内存说改了、磁盘没改"。
    pub fn set(
        &self,
        preference: LocalePreference,
        expected_revision: u64,
        system_candidates: Vec<String>,
    ) -> Result<LocaleWriteOutcome> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.revision != expected_revision {
            return Ok(LocaleWriteOutcome::Conflict {
                snapshot: guard.clone(),
            });
        }
        let active = resolve_active(&preference, &system_candidates);
        if guard.preference == preference && guard.active_locale == active {
            // 无变化:不推进 revision,直接确认当前状态。
            return Ok(LocaleWriteOutcome::Applied {
                snapshot: guard.clone(),
            });
        }
        // revision 溢出:明确失败,绝不回绕(0 会被误当"全新")。
        let next_revision = guard
            .revision
            .checked_add(1)
            .context("语言设置 revision 溢出")?;
        let next = LocaleSnapshot {
            schema_version: LocaleSnapshot::SCHEMA_VERSION,
            preference,
            active_locale: active,
            revision: next_revision,
        };
        write_atomic(&self.path, &next).context("写入语言设置")?;
        *guard = next.clone();
        Ok(LocaleWriteOutcome::Applied { snapshot: next })
    }
}

/// 按偏好求出实际生效语言:显式偏好直接用其语言;`System` 从候选里解析,无匹配回落 `en`。
pub fn resolve_active(preference: &LocalePreference, system_candidates: &[String]) -> Locale {
    match preference {
        LocalePreference::Explicit { locale } => locale.clone(),
        LocalePreference::System => {
            Locale(dozer_i18n::resolve_from_candidates(system_candidates).to_string())
        }
    }
}

/// 原子写:同目录临时文件写全 + fsync + rename 覆盖目标名。`rename` 在同文件系统内是原子的,
/// 读者要么看到旧文件、要么看到新文件,不会读到半截 JSON。
fn write_atomic(path: &Path, snapshot: &LocaleSnapshot) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).context("建配置目录")?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).context("建临时文件")?;
    let json = serde_json::to_string_pretty(snapshot).context("序列化语言设置")?;
    tmp.write_all(json.as_bytes()).context("写临时文件")?;
    tmp.write_all(b"\n").context("写临时文件")?;
    tmp.as_file_mut().sync_all().context("fsync 临时文件")?;
    tmp.persist(path).context("rename 覆盖语言设置")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(s: &str) -> Locale {
        Locale(s.to_string())
    }

    fn store() -> (tempfile::TempDir, LocaleSettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locale_settings.json");
        let store = LocaleSettingsStore::open(&path).unwrap();
        (dir, store)
    }

    #[test]
    fn missing_file_yields_default_snapshot() {
        let (_d, store) = store();
        let s = store.get();
        assert_eq!(s, LocaleSnapshot::default_snapshot());
        assert_eq!(s.revision, 0);
        assert_eq!(s.preference, LocalePreference::System);
        assert_eq!(s.active_locale.as_str(), "en");
    }

    #[test]
    fn explicit_set_applies_and_persists_revision() {
        let (_d, store) = store();
        let out = store
            .set(
                LocalePreference::Explicit {
                    locale: loc("zh-CN"),
                },
                0,
                vec![],
            )
            .unwrap();
        match out {
            LocaleWriteOutcome::Applied { snapshot } => {
                assert_eq!(snapshot.revision, 1);
                assert_eq!(snapshot.active_locale.as_str(), "zh-CN");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(store.get().revision, 1);
    }

    #[test]
    fn set_with_stale_revision_conflicts_and_does_not_change() {
        let (_d, store) = store();
        let _ = store
            .set(
                LocalePreference::Explicit {
                    locale: loc("zh-CN"),
                },
                0,
                vec![],
            )
            .unwrap();
        let out = store
            .set(LocalePreference::Explicit { locale: loc("en") }, 0, vec![])
            .unwrap();
        match out {
            LocaleWriteOutcome::Conflict { snapshot } => {
                assert_eq!(snapshot.revision, 1);
                assert_eq!(snapshot.active_locale.as_str(), "zh-CN");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(store.get().revision, 1, "冲突不改状态");
    }

    #[test]
    fn system_preference_resolves_from_candidates() {
        let (_d, store) = store();
        let out = store
            .set(
                LocalePreference::System,
                0,
                vec!["fr-FR".into(), "zh-HK".into(), "zh-CN".into()],
            )
            .unwrap();
        match out {
            LocaleWriteOutcome::Applied { snapshot } => {
                assert_eq!(snapshot.preference, LocalePreference::System);
                assert_eq!(snapshot.active_locale.as_str(), "zh-CN");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn system_without_supported_candidates_falls_back_to_en() {
        let active = resolve_active(&LocalePreference::System, &["C".into(), "POSIX".into()]);
        assert_eq!(active.as_str(), "en");
    }

    #[test]
    fn snapshot_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locale_settings.json");
        {
            let store = LocaleSettingsStore::open(&path).unwrap();
            store
                .set(
                    LocalePreference::Explicit {
                        locale: loc("zh-CN"),
                    },
                    0,
                    vec![],
                )
                .unwrap();
        }
        let reopened = LocaleSettingsStore::open(&path).unwrap();
        let s = reopened.get();
        assert_eq!(s.revision, 1);
        assert_eq!(s.active_locale.as_str(), "zh-CN");
    }

    #[test]
    fn corrupt_file_recovers_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locale_settings.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = LocaleSettingsStore::open(&path).unwrap();
        assert_eq!(store.get(), LocaleSnapshot::default_snapshot());
        // 之后再写应能正常覆盖坏文件。
        let out = store
            .set(
                LocalePreference::Explicit {
                    locale: loc("zh-CN"),
                },
                0,
                vec![],
            )
            .unwrap();
        assert!(matches!(out, LocaleWriteOutcome::Applied { .. }));
        let reopened = LocaleSettingsStore::open(&path).unwrap();
        assert_eq!(reopened.get().active_locale.as_str(), "zh-CN");
    }

    #[test]
    fn concurrent_cas_only_one_wins() {
        use std::sync::Arc;
        let (_d, store) = store();
        let store = Arc::new(store);
        let mut handles = vec![];
        for i in 0..8 {
            let s = store.clone();
            handles.push(std::thread::spawn(move || {
                let locale = if i % 2 == 0 { "zh-CN" } else { "en" };
                s.set(
                    LocalePreference::Explicit {
                        locale: loc(locale),
                    },
                    0,
                    vec![],
                )
                .unwrap()
            }));
        }
        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let applied = outcomes
            .iter()
            .filter(|o| matches!(o, LocaleWriteOutcome::Applied { .. }))
            .count();
        let conflicts = outcomes
            .iter()
            .filter(|o| matches!(o, LocaleWriteOutcome::Conflict { .. }))
            .count();
        assert_eq!(applied, 1, "只有一个能拿到 revision 0");
        assert_eq!(conflicts, 7);
        assert_eq!(store.get().revision, 1);
    }

    #[test]
    fn no_op_set_does_not_advance_revision() {
        let (_d, store) = store();
        let first = store
            .set(
                LocalePreference::Explicit {
                    locale: loc("zh-CN"),
                },
                0,
                vec![],
            )
            .unwrap();
        match first {
            LocaleWriteOutcome::Applied { snapshot } => assert_eq!(snapshot.revision, 1),
            other => panic!("unexpected: {other:?}"),
        }
        // 相同偏好再次提交:revision 不变,也不重写文件。
        let again = store
            .set(
                LocalePreference::Explicit {
                    locale: loc("zh-CN"),
                },
                1,
                vec![],
            )
            .unwrap();
        match again {
            LocaleWriteOutcome::Applied { snapshot } => {
                assert_eq!(snapshot.revision, 1, "无变化不推进 revision");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn revision_overflow_fails_instead_of_wrapping() {
        let (_d, store) = store();
        {
            let mut g = store.inner.lock().unwrap();
            g.revision = u64::MAX;
            g.preference = LocalePreference::Explicit { locale: loc("en") };
            g.active_locale = loc("en");
        }
        // 有变化(zh-CN ≠ en)且 revision 已到顶:明确失败,不回绕。
        let err = store
            .set(
                LocalePreference::Explicit {
                    locale: loc("zh-CN"),
                },
                u64::MAX,
                vec![],
            )
            .unwrap_err();
        assert!(err.to_string().contains("溢出"));
        assert_eq!(store.get().revision, u64::MAX, "失败不改内存");
    }

    #[test]
    fn revision_increases_monotonically_across_sets() {
        let (_d, store) = store();
        let mut rev = 0;
        for i in 0..5 {
            let locale = if i % 2 == 0 { "zh-CN" } else { "en" };
            let out = store
                .set(
                    LocalePreference::Explicit {
                        locale: loc(locale),
                    },
                    rev,
                    vec![],
                )
                .unwrap();
            match out {
                LocaleWriteOutcome::Applied { snapshot } => rev = snapshot.revision,
                other => panic!("unexpected: {other:?}"),
            }
            assert_eq!(rev, i + 1);
        }
    }
}
