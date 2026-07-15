use crate::config::Model;

pub fn status_line(m: &Model, running: bool) -> String {
    if running {
        format!("{:<16}Running  {}:{}", m.name, m.host, m.port)
    } else {
        format!("{:<16}Stopped", m.name)
    }
}

use crate::config::Config;
use crate::error::ShyError;
use crate::process::{self, is_running, mlx_args};
use anyhow::Context;
use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn lookup<'a>(cfg: &'a Config, id: &str) -> anyhow::Result<&'a crate::config::Model> {
    cfg.models
        .get(id)
        .ok_or_else(|| ShyError::ModelNotFound(id.to_string()).into())
}

pub fn list(cfg: &Config) {
    for m in cfg.models.values() {
        println!("{}", m.name);
    }
}

fn running_pid(id: &str) -> anyhow::Result<Option<u32>> {
    match process::read_pid(id)? {
        Some(pid) if is_running(pid) => Ok(Some(pid)),
        _ => Ok(None),
    }
}

pub fn start(cfg: &Config, id: &str) -> anyhow::Result<()> {
    let m = lookup(cfg, id)?;
    if running_pid(id)?.is_some() {
        println!("{} already running", m.name);
        return Ok(());
    }
    let log_path = crate::paths::log_file(id)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening log {}", log_path.display()))?;
    let err_log = log.try_clone()?;
    let child = Command::new(&m.command)
        .args(mlx_args(m))
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log))
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("spawning {}", m.command))?;
    process::write_pid(id, child.id())?;
    println!(
        "Started {} (pid {}) on {}:{}",
        m.name,
        child.id(),
        m.host,
        m.port
    );
    Ok(())
}

pub fn stop(cfg: &Config, id: &str) -> anyhow::Result<()> {
    let m = lookup(cfg, id)?;
    match process::read_pid(id)? {
        Some(pid) if is_running(pid) => {
            Command::new("kill")
                .arg(pid.to_string())
                .status()
                .with_context(|| format!("killing pid {pid}"))?;
            process::clear_pid(id)?;
            println!("Stopped {}", m.name);
            Ok(())
        }
        _ => {
            process::clear_pid(id)?;
            Err(ShyError::ProcessNotRunning(id.to_string()).into())
        }
    }
}

pub fn restart(cfg: &Config, id: &str) -> anyhow::Result<()> {
    let _ = stop(cfg, id);
    start(cfg, id)
}

pub fn status(cfg: &Config) -> anyhow::Result<()> {
    for (id, m) in &cfg.models {
        let running = running_pid(id)?.is_some();
        println!("{}", status_line(m, running));
    }
    Ok(())
}

pub fn logs(id: &str) -> anyhow::Result<()> {
    let path = crate::paths::log_file(id)?;
    Command::new("tail")
        .arg("-f")
        .arg(&path)
        .status()
        .with_context(|| format!("tailing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Model;

    fn model() -> Model {
        Model {
            name: "Qwen3.6".into(),
            command: "mlx_lm.server".into(),
            model: "/m".into(),
            host: "127.0.0.1".into(),
            port: 7101,
        }
    }

    #[test]
    fn running_line_shows_endpoint() {
        assert_eq!(
            status_line(&model(), true),
            "Qwen3.6         Running  127.0.0.1:7101"
        );
    }

    #[test]
    fn stopped_line() {
        assert_eq!(status_line(&model(), false), "Qwen3.6         Stopped");
    }
}
