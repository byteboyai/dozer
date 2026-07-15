use anyhow::Context;
use std::path::{Path, PathBuf};

pub fn join_config(home: &Path) -> PathBuf {
    home.join(".config").join("dozer").join("config.toml")
}

pub fn join_state(home: &Path) -> PathBuf {
    home.join(".local").join("state").join("dozer")
}

fn home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().context("could not determine home directory")
}

pub fn config_file() -> anyhow::Result<PathBuf> {
    Ok(join_config(&home()?))
}

pub fn state_dir() -> anyhow::Result<PathBuf> {
    Ok(join_state(&home()?))
}

pub fn pid_file(id: &str) -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join(format!("{id}.pid")))
}

pub fn log_file(id: &str) -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join(format!("{id}.log")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_is_xdg_style() {
        let home = Path::new("/home/u");
        assert_eq!(
            join_config(home),
            Path::new("/home/u/.config/dozer/config.toml")
        );
    }

    #[test]
    fn state_path_is_xdg_style() {
        let home = Path::new("/home/u");
        assert_eq!(join_state(home), Path::new("/home/u/.local/state/dozer"));
    }
}
