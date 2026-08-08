//! SSH 远程主机面板 · 阶段 1:主机连接管理 + 认证 + 连接测试(含标准 SSH
//! host key 验证)。没有 App 级 `AppState`——SSH 不像数据库面板那样有
//! "驱动类型可开关"的全局概念,只有 `WorkspaceState` 挂每个 `Workspace`。
//! SSH 终端(阶段 2)/SFTP(阶段 3)留后续,见
//! `docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthMethod {
    Password,
    PrivateKey { key_path: String },
}

/// 一个 SSH 主机(不含密码/私钥口令)。`.dozer/ssh_hosts.json` 存
/// `Vec<SshHost>`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshHost {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

/// 某台主机当前的连接测试状态,画在卡片上。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TestStatus {
    #[default]
    Idle,
    Testing,
    Ok,
    /// 首次连接,host key 未知——`fingerprint` 给 UI 显示,用户确认后发
    /// `Message::TrustHostKey(host_id)` 才会真的写入 known_hosts 并重连。
    UnknownHostKey {
        fingerprint: String,
    },
    /// host key 变了(可能中间人攻击)——直接拒绝,没有"信任并继续"这个
    /// 选项(同设计文档"目标"第 6 条)。
    KeyChanged {
        fingerprint: String,
    },
    Err(String),
}

#[derive(Debug, Clone, Default)]
pub struct SshHostDraft {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub use_private_key: bool,
    pub key_path: String,
    pub password: String,
}

/// 挂在每个 `Workspace` 上:当前项目配置的 SSH 主机列表 + 编辑态 + 每台
/// 主机的测试状态 + "未知 host key 待信任"暂存(测试返回
/// `TestStatus::UnknownHostKey` 时,真正的服务器公钥字节存这里,点"信任
/// 并重试"才用得到——不放进 `TestStatus`本身,因为 `TestStatus` 要
/// `PartialEq`/`Clone` 且传给 view 层只需要展示指纹文案,不需要带着一份
/// 公钥字节到处传)。
#[derive(Debug, Default)]
pub struct WorkspaceState {
    hosts: Vec<SshHost>,
    editing: Option<SshHostDraft>,
    test_status: HashMap<String, TestStatus>,
    pending_unknown_keys: HashMap<String, Vec<u8>>,
}

impl WorkspaceState {
    pub fn hosts(&self) -> &[SshHost] {
        &self.hosts
    }
    pub fn editing(&self) -> Option<&SshHostDraft> {
        self.editing.as_ref()
    }
    pub fn test_status(&self, host_id: &str) -> &TestStatus {
        self.test_status.get(host_id).unwrap_or(&TestStatus::Idle)
    }
}

pub fn hosts_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("ssh_hosts.json")
}

pub fn reload_from_disk(ws_state: &mut WorkspaceState, repo: &Path) {
    let path = hosts_path(repo);
    ws_state.hosts = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
}

fn save_hosts(repo: &Path, hosts: &[SshHost]) -> std::io::Result<()> {
    let dir = repo.join(".dozer");
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(hosts).expect("Vec<SshHost> 总能序列化");
    std::fs::write(hosts_path(repo), json)
}

fn keyring_entry(project_id: i64, host_id: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dozer-ssh", &format!("{project_id}:{host_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str) -> SshHost {
        SshHost {
            id: id.into(),
            name: format!("test-{id}"),
            host: "example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Password,
        }
    }

    #[test]
    fn reload_from_disk_missing_file_yields_empty_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.hosts().is_empty());
    }

    #[test]
    fn save_then_reload_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = vec![host("a"), host("b")];
        save_hosts(dir.path(), &hosts).unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert_eq!(ws_state.hosts(), hosts.as_slice());
    }

    #[test]
    fn private_key_auth_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = host("k");
        h.auth = AuthMethod::PrivateKey {
            key_path: "/home/user/.ssh/id_ed25519".into(),
        };
        save_hosts(dir.path(), &[h.clone()]).unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert_eq!(ws_state.hosts()[0].auth, h.auth);
    }

    #[test]
    fn reload_from_disk_corrupt_file_yields_empty_hosts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(hosts_path(dir.path()), "not json").unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.hosts().is_empty());
    }
}
