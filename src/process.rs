use anyhow::Context;
use std::os::unix::fs::PermissionsExt;
use sysinfo::{Pid, System};

use crate::config::Model;

pub fn mlx_args(m: &Model) -> Vec<String> {
    vec![
        "--model".into(),
        m.model.clone(),
        "--host".into(),
        m.host.clone(),
        "--port".into(),
        m.port.to_string(),
    ]
}

pub fn command_on_path(cmd: &str, path_var: &str) -> bool {
    std::env::split_paths(path_var).any(|dir| {
        let candidate = dir.join(cmd);
        candidate.is_file()
            && std::fs::metadata(&candidate)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    })
}

pub fn is_installed(cmd: &str) -> bool {
    match std::env::var("PATH") {
        Ok(p) => command_on_path(cmd, &p),
        Err(_) => false,
    }
}

pub fn read_pid(id: &str) -> anyhow::Result<Option<u32>> {
    let path = crate::paths::pid_file(id)?;
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("reading pid file {}", path.display()))?;
    Ok(s.trim().parse::<u32>().ok())
}

pub fn write_pid(id: &str, pid: u32) -> anyhow::Result<()> {
    let dir = crate::paths::state_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating state dir {}", dir.display()))?;
    std::fs::write(crate::paths::pid_file(id)?, pid.to_string())?;
    Ok(())
}

pub fn clear_pid(id: &str) -> anyhow::Result<()> {
    let path = crate::paths::pid_file(id)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn is_running(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
    );
    sys.process(Pid::from_u32(pid)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Model;

    #[test]
    fn builds_mlx_args_in_order() {
        let m = Model {
            name: "Q".into(),
            command: "mlx_lm.server".into(),
            model: "/m".into(),
            host: "127.0.0.1".into(),
            port: 7101,
        };
        assert_eq!(
            mlx_args(&m),
            vec!["--model", "/m", "--host", "127.0.0.1", "--port", "7101"]
        );
    }

    #[test]
    fn command_on_path_finds_existing_dir_entry() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let path_var = dir.to_string_lossy().to_string();

        // Positive case: executable file → true
        let exec_file = dir.join("dozer_test_bin_marker");
        std::fs::write(&exec_file, b"x").unwrap();
        std::fs::set_permissions(
            &exec_file,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        assert!(command_on_path("dozer_test_bin_marker", &path_var));

        // Negative case: missing file → false
        assert!(!command_on_path("definitely_not_here_xyz", &path_var));

        // Bonus negative case: non-executable file → false
        let noexec_file = dir.join("dozer_test_noexec_marker");
        std::fs::write(&noexec_file, b"x").unwrap();
        std::fs::set_permissions(
            &noexec_file,
            std::os::unix::fs::PermissionsExt::from_mode(0o644),
        )
        .unwrap();
        assert!(!command_on_path("dozer_test_noexec_marker", &path_var));

        let _ = std::fs::remove_file(&exec_file);
        let _ = std::fs::remove_file(&noexec_file);
    }
}
