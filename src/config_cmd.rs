use anyhow::Context;

pub fn show() -> anyhow::Result<()> {
    crate::config::load_or_init()?;
    let path = crate::paths::config_file()?;
    let s =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    print!("{s}");
    Ok(())
}

pub fn editor() -> String {
    std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())
}

pub fn edit() -> anyhow::Result<()> {
    crate::config::load_or_init()?;
    let path = crate::paths::config_file()?;
    std::process::Command::new(editor())
        .arg(&path)
        .status()
        .with_context(|| format!("editing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_defaults_to_vi_when_unset() {
        // SAFETY: single-threaded test
        unsafe {
            std::env::remove_var("EDITOR");
        }
        assert_eq!(editor(), "vi");
    }
}
