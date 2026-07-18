//! zsh shell 集成：把 OSC 7/133 发射钩子经 ZDOTDIR 包装注入交互式
//! zsh 会话（kitty/ghostty 同款姿势；spec P1e D2）。

use anyhow::{Context, Result};
use std::path::PathBuf;

const ZSHENV: &str = include_str!("../assets/zdotdir/.zshenv");
const ZSHRC: &str = include_str!("../assets/zdotdir/.zshrc");

/// 把包装 zdotdir 落盘到 state 目录（幂等，内容变更时覆写）。
pub fn ensure_zdotdir() -> Result<PathBuf> {
    let dir = dozer_core::paths::state_dir().join("zdotdir");
    std::fs::create_dir_all(&dir).context("创建 zdotdir")?;
    for (name, content) in [(".zshenv", ZSHENV), (".zshrc", ZSHRC)] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
            std::fs::write(&path, content).with_context(|| format!("写 {name}"))?;
        }
    }
    Ok(dir)
}

/// 会话是否应注入 shell 集成：command 是 zsh 且未被 DOZER_SHELL_INTEGRATION=0 关闭。
pub fn should_inject(command: &str) -> bool {
    let is_zsh = std::path::Path::new(command)
        .file_name()
        .is_some_and(|n| n == "zsh");
    is_zsh && std::env::var("DOZER_SHELL_INTEGRATION").as_deref() != Ok("0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_zdotdir_writes_wrapper_files_idempotently() {
        let dir = ensure_zdotdir().unwrap();
        let zshrc = std::fs::read_to_string(dir.join(".zshrc")).unwrap();
        assert!(zshrc.contains("133;D"), "带退出码发射钩子");
        assert!(zshrc.contains("7;file://"), "带 cwd 发射钩子");
        let zshenv = std::fs::read_to_string(dir.join(".zshenv")).unwrap();
        assert!(zshenv.contains("DOZER_ORIG_ZDOTDIR"));
        // 幂等：重复调用不报错，内容不变
        let dir2 = ensure_zdotdir().unwrap();
        assert_eq!(dir, dir2);
        assert_eq!(zshrc, std::fs::read_to_string(dir.join(".zshrc")).unwrap());
    }

    #[test]
    fn should_inject_only_for_zsh() {
        assert!(should_inject("/bin/zsh"));
        assert!(should_inject("zsh"));
        assert!(!should_inject("/bin/bash"));
        assert!(!should_inject("/bin/sh"));
    }
}
