use crate::agent::status_line;
use crate::config::Config;
use crate::process::is_installed;

pub fn base_tools() -> Vec<&'static str> {
    vec!["cargo", "python", "git", "uv", "mlx_lm", "comfyui"]
}

pub fn run(cfg: &Config) {
    for tool in base_tools() {
        println!("{}", status_line(tool, is_installed(tool)));
    }
    for (name, cmd) in &cfg.agents {
        println!("{}", status_line(name, is_installed(cmd)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_tools_include_core_toolchain() {
        let tools = base_tools();
        assert!(tools.contains(&"cargo"));
        assert!(tools.contains(&"git"));
        assert!(tools.contains(&"uv"));
    }
}
