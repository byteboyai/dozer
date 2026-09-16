//! Claude Code IDE 集成桥接:让 Dozer 模拟 VS Code/JetBrains 的 IDE 握手协议,
//! 把 `preview_context::PreviewContextStore` 里的数据喂给 Claude Code CLI 的
//! 自动上下文机制(聊天记录里的 `In <file>` 提示条)。设计依据见
//! `docs/superpowers/specs/2026-09-16-claude-code-ide-bridge-design.md`。

use dozer_core::protocol::PreviewContext;
use std::path::{Path, PathBuf};

pub(crate) fn resolve_lock_dir(
    override_dir: Option<String>,
    claude_config_dir: Option<String>,
    home: Option<String>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return PathBuf::from(dir);
    }
    if let Some(dir) = claude_config_dir {
        return PathBuf::from(dir).join("ide");
    }
    let home = home.unwrap_or_else(|| "/".to_string());
    PathBuf::from(home).join(".claude").join("ide")
}

pub(crate) fn lock_dir() -> PathBuf {
    resolve_lock_dir(
        std::env::var("DOZER_CLAUDE_IDE_LOCK_DIR").ok(),
        std::env::var("CLAUDE_CONFIG_DIR").ok(),
        std::env::var("HOME").ok(),
    )
}

pub(crate) fn generate_token() -> String {
    // 128 位随机值,32 个十六进制字符,无连字符——复用仓库已有的 uuid 依赖,
    // 不为这一个用途单独引入 rand crate。
    uuid::Uuid::new_v4().simple().to_string()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LockFileContents {
    pub(crate) pid: u32,
    pub(crate) port: u16,
    pub(crate) workspace_folders: Vec<String>,
    pub(crate) ide_name: String,
    pub(crate) token: String,
}

pub(crate) fn write_lock_file(
    dir: &Path,
    port: u16,
    workspace_root: &str,
    token: &str,
    pid: u32,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let contents = LockFileContents {
        pid,
        port,
        workspace_folders: vec![workspace_root.to_string()],
        ide_name: "Dozer".to_string(),
        token: token.to_string(),
    };
    let path = dir.join(format!("{port}.lock"));
    let json = serde_json::to_string_pretty(&contents).expect("LockFileContents 序列化不应失败");
    std::fs::write(&path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

pub(crate) fn remove_lock_file(path: &Path) {
    // 幂等:文件不存在也不算错误(项目可能已经被清扫过)。
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn override_dir_wins_over_everything() {
        let got = resolve_lock_dir(
            Some("/override".to_string()),
            Some("/claude-config".to_string()),
            Some("/home/u".to_string()),
        );
        assert_eq!(got, PathBuf::from("/override"));
    }

    #[test]
    fn claude_config_dir_wins_over_home() {
        let got = resolve_lock_dir(
            None,
            Some("/claude-config".to_string()),
            Some("/home/u".to_string()),
        );
        assert_eq!(got, PathBuf::from("/claude-config/ide"));
    }

    #[test]
    fn falls_back_to_home_dot_claude_ide() {
        let got = resolve_lock_dir(None, None, Some("/home/u".to_string()));
        assert_eq!(got, PathBuf::from("/home/u/.claude/ide"));
    }
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn token_is_32_lowercase_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokens_are_not_repeated_across_calls() {
        assert_ne!(generate_token(), generate_token());
    }
}

#[cfg(test)]
mod lock_file_tests {
    use super::*;

    #[test]
    fn write_then_read_round_trips_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lock_file(dir.path(), 54321, "/repo/root", "abc123token", 999).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: LockFileContents = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed,
            LockFileContents {
                pid: 999,
                port: 54321,
                workspace_folders: vec!["/repo/root".to_string()],
                ide_name: "Dozer".to_string(),
                token: "abc123token".to_string(),
            }
        );
    }

    #[test]
    fn lock_file_path_is_named_after_port() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lock_file(dir.path(), 12345, "/repo", "t", 1).unwrap();
        assert_eq!(path.file_name().unwrap(), "12345.lock");
    }

    #[cfg(unix)]
    #[test]
    fn file_and_dir_permissions_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = write_lock_file(dir.path(), 1, "/repo", "t", 1).unwrap();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        let dir_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn remove_lock_file_is_noop_when_missing() {
        // 不应 panic 或返回需要处理的错误
        remove_lock_file(Path::new("/tmp/dozer-ide-bridge-test-does-not-exist.lock"));
    }
}

pub(crate) fn selection_result(ctx: Option<&PreviewContext>) -> serde_json::Value {
    match ctx {
        Some(c) => serde_json::json!({
            "success": true,
            "filePath": c.path,
            "selection": {
                "start": {
                    "line": c.start_line.saturating_sub(1),
                    "character": c.start_col.saturating_sub(1),
                },
                "end": {
                    "line": c.end_line.saturating_sub(1),
                    "character": c.end_col.saturating_sub(1),
                },
                "isEmpty": !c.has_selection,
            },
            "text": "",
        }),
        None => serde_json::json!({
            "success": false,
            "filePath": null,
            "selection": null,
            "text": "",
        }),
    }
}

pub(crate) fn open_editors_result(ctx: Option<&PreviewContext>) -> serde_json::Value {
    match ctx {
        Some(c) => serde_json::json!({
            "tabs": [{"filePath": c.path, "isActive": true}]
        }),
        None => serde_json::json!({"tabs": []}),
    }
}

pub(crate) fn diagnostics_result() -> serde_json::Value {
    serde_json::json!({"diagnostics": []})
}

#[cfg(test)]
mod mapping_tests {
    use super::*;

    fn ctx() -> PreviewContext {
        PreviewContext {
            path: "/repo/src/main.rs".to_string(),
            start_line: 12,
            start_col: 3,
            end_line: 12,
            end_col: 3,
            has_selection: false,
            updated_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn selection_result_with_context_maps_1_indexed_to_0_indexed() {
        let got = selection_result(Some(&ctx()));
        assert_eq!(
            got,
            serde_json::json!({
                "success": true,
                "filePath": "/repo/src/main.rs",
                "selection": {
                    "start": {"line": 11, "character": 2},
                    "end": {"line": 11, "character": 2},
                    "isEmpty": true,
                },
                "text": "",
            })
        );
    }

    #[test]
    fn selection_result_has_selection_true_sets_is_empty_false() {
        let mut c = ctx();
        c.has_selection = true;
        c.end_line = 14;
        c.end_col = 1;
        let got = selection_result(Some(&c));
        assert_eq!(got["selection"]["isEmpty"], false);
        assert_eq!(got["selection"]["end"]["line"], 13);
        assert_eq!(got["selection"]["end"]["character"], 0);
    }

    #[test]
    fn selection_result_without_context_reports_failure() {
        let got = selection_result(None);
        assert_eq!(
            got,
            serde_json::json!({
                "success": false,
                "filePath": null,
                "selection": null,
                "text": "",
            })
        );
    }

    #[test]
    fn open_editors_result_with_context_lists_one_tab() {
        let got = open_editors_result(Some(&ctx()));
        assert_eq!(
            got,
            serde_json::json!({
                "tabs": [{"filePath": "/repo/src/main.rs", "isActive": true}]
            })
        );
    }

    #[test]
    fn open_editors_result_without_context_is_empty() {
        let got = open_editors_result(None);
        assert_eq!(got, serde_json::json!({"tabs": []}));
    }

    #[test]
    fn diagnostics_result_is_always_empty() {
        assert_eq!(diagnostics_result(), serde_json::json!({"diagnostics": []}));
    }
}
