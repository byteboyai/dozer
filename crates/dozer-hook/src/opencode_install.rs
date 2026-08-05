//! OpenCode 插件安装：把 `dozer.ts`/`dozer-translate.ts` 写进
//! `~/.config/opencode/plugins/`（只放复数目录——spike
//! `2026-07-31-opencode-plugin-spike-findings.md` Step 2 实测确认单数
//! `plugin/` 目录会导致同一事件被同时加载两次触发两次，后续代码不碰
//! 它）。跟 Claude/CodeBuddy 的 JSON 增量合并不同，这两个文件是 Dozer
//! 独占的文件名，不与用户/其他工具共享同一份配置——install 直接整体
//! 覆盖写，uninstall 直接删，不需要合并逻辑。
//!
//! `dozer.ts` 里 spawn `dozer-hook` 用的二进制路径在安装时被替换成
//! `current_exe()` 的绝对路径（跟 Claude/CodeBuddy 安装器把 exe 路径写
//! 进 hook command 字符串是同一手法），避免依赖插件运行时的 PATH。

use std::path::{Path, PathBuf};

const DOZER_TS_TEMPLATE: &str = include_str!("../opencode_plugin/dozer.ts");
const TRANSLATE_TS: &str = include_str!("../opencode_plugin/dozer-translate.ts");
const HOOK_BIN_PLACEHOLDER: &str = "__DOZER_HOOK_BIN_PATH__";

/// 插件目录路径，`DOZER_OPENCODE_PLUGIN_DIR` 覆盖用于测试（跟
/// `install.rs` 的 `DOZER_CLAUDE_SETTINGS`/`DOZER_CODEBUDDY_SETTINGS`
/// 同一套手法）。
pub fn plugins_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DOZER_OPENCODE_PLUGIN_DIR") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home)
        .join(".config")
        .join("opencode")
        .join("plugins")
}

pub fn run_at(dir: &Path, install: bool) -> i32 {
    if install {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("建目录失败: {e}");
            return 1;
        }
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "dozer-hook".into());
        let dozer_ts = DOZER_TS_TEMPLATE.replace(HOOK_BIN_PLACEHOLDER, &exe);
        if let Err(e) = std::fs::write(dir.join("dozer.ts"), dozer_ts) {
            eprintln!("写 dozer.ts 失败: {e}");
            return 1;
        }
        if let Err(e) = std::fs::write(dir.join("dozer-translate.ts"), TRANSLATE_TS) {
            eprintln!("写 dozer-translate.ts 失败: {e}");
            return 1;
        }
        println!("已安装: {}", dir.display());
    } else {
        for name in ["dozer.ts", "dozer-translate.ts"] {
            let p = dir.join(name);
            if p.exists()
                && let Err(e) = std::fs::remove_file(&p)
            {
                eprintln!("删 {} 失败: {e}", p.display());
                return 1;
            }
        }
        println!("已卸载: {}", dir.display());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_both_files_with_exe_path_substituted() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert!(
            !dozer_ts.contains("__DOZER_HOOK_BIN_PATH__"),
            "占位符必须被替换成真实 exe 路径"
        );
        assert!(dozer_ts.contains("DozerPlugin"));
        let translate_ts = std::fs::read_to_string(dir.path().join("dozer-translate.ts")).unwrap();
        assert!(translate_ts.contains("onSessionCreated"));
    }

    #[test]
    fn install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert_eq!(run_at(dir.path(), true), 0);
        assert!(dir.path().join("dozer.ts").exists());
    }

    #[test]
    fn uninstall_removes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert_eq!(run_at(dir.path(), false), 0);
        assert!(!dir.path().join("dozer.ts").exists());
        assert!(!dir.path().join("dozer-translate.ts").exists());
    }

    #[test]
    fn uninstall_on_missing_dir_does_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("not-created-yet");
        assert_eq!(run_at(&nested, false), 0);
    }

    #[test]
    fn plugins_dir_honors_env_override() {
        unsafe { std::env::set_var("DOZER_OPENCODE_PLUGIN_DIR", "/tmp/probe-opencode-plugins") };
        assert_eq!(plugins_dir(), PathBuf::from("/tmp/probe-opencode-plugins"));
        unsafe { std::env::remove_var("DOZER_OPENCODE_PLUGIN_DIR") };
    }
}
