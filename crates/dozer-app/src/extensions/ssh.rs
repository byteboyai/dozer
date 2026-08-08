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

#[derive(Debug)]
enum SshError {
    Russh(russh::Error),
    UnknownHostKey {
        fingerprint: String,
        key_bytes: Vec<u8>,
    },
    KeyChanged {
        fingerprint: String,
    },
    AuthFailed,
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshError::Russh(e) => write!(f, "{e}"),
            SshError::UnknownHostKey { .. } => write!(f, "未知主机,需要确认指纹"),
            SshError::KeyChanged { fingerprint } => {
                write!(
                    f,
                    "主机指纹已变化({fingerprint}),可能存在中间人攻击风险,拒绝连接"
                )
            }
            SshError::AuthFailed => write!(f, "认证失败(密码或密钥不正确)"),
        }
    }
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Russh(e)
    }
}

struct TestHandler {
    host: String,
    port: u16,
}

impl russh::client::Handler for TestHandler {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();
        // rusSSH 0.62 的 `known_hosts::check_known_hosts` 语义(已对照其源码
        // `keys/known_hosts.rs` 核实,与 0.51 文档不一致):
        //   Ok(true)  -> known_hosts 里有记录且算法/内容都匹配;
        //   Ok(false) -> 没有匹配记录(全新主机,或记录算法不同)——对应"未知
        //                主机,需要走信任流程";
        //   Err(KeyChanged) -> 有同算法记录但公钥不同——host key 变了,可能
        //                中间人攻击,必须拒绝。
        // 注意这里的 `Ok(false)`/`Err` 划分与早期文档猜测相反,已按实际
        // API 修正,不能把"未知主机"和"key 变化"混成一个错误。
        match russh::keys::known_hosts::check_known_hosts(&self.host, self.port, server_public_key)
        {
            Ok(true) => Ok(true),
            Ok(false) => Err(SshError::UnknownHostKey {
                fingerprint,
                key_bytes: server_public_key
                    .to_bytes()
                    .map_err(|_| SshError::AuthFailed)?,
            }),
            Err(_) => Err(SshError::KeyChanged { fingerprint }),
        }
    }
}

async fn test_connection(host: SshHost, password: Option<String>) -> Result<(), SshError> {
    let config = std::sync::Arc::new(russh::client::Config::default());
    let handler = TestHandler {
        host: host.host.clone(),
        port: host.port,
    };
    let mut handle =
        russh::client::connect(config, (host.host.as_str(), host.port), handler).await?;

    let auth_result = match &host.auth {
        AuthMethod::Password => {
            let password = password.unwrap_or_default();
            handle
                .authenticate_password(&host.username, &password)
                .await?
        }
        AuthMethod::PrivateKey { key_path } => {
            let key = russh::keys::load_secret_key(key_path, password.as_deref())
                .map_err(|_| SshError::AuthFailed)?;
            let hash_alg = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(
                    &host.username,
                    russh::keys::PrivateKeyWithHashAlg::new(std::sync::Arc::new(key), hash_alg),
                )
                .await?
        }
    };
    match auth_result {
        russh::client::AuthResult::Success => Ok(()),
        russh::client::AuthResult::Failure { .. } => Err(SshError::AuthFailed),
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    AddHostStart,
    EditHostStart(String),
    DraftNameChanged(String),
    DraftHostChanged(String),
    DraftPortChanged(String),
    DraftUsernameChanged(String),
    DraftAuthMethodToggled(bool), // true = 用私钥,false = 用密码
    DraftKeyPathChanged(String),
    DraftPasswordChanged(String),
    DraftSave,
    DraftCancel,
    DeleteHost(String),
    TestConnection(String),
    /// 异步测试结果。带 `project_id`,理由同数据库面板(异步结果不能假设
    /// 聚焦项目没变)。`Result` 的 `Err` 用一个精简过的字符串描述——真正
    /// 携带指纹/公钥字节的是下面 `UnknownKeyDetected`,这两条经常一起触发
    /// (一次测试失败,同时是"未知主机"这个具体原因)。
    TestConnectionResult(i64, String, Result<(), String>),
    /// 测试过程中遇到未知 host key,携带指纹文案(给 UI 显示)+ 公钥原始
    /// 字节(存进 `pending_unknown_keys`,点"信任并重试"时要用)。
    UnknownKeyDetected(i64, String, String, Vec<u8>),
    /// 用户确认信任某台主机的 host key:写入 known_hosts,然后重新发起
    /// 一次 `TestConnection`。
    TrustHostKey(String),
}

pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::AddHostStart => ws_state.editing = Some(SshHostDraft::default()),
        Message::EditHostStart(id) => {
            if let Some(h) = ws_state.hosts.iter().find(|h| h.id == id) {
                let (use_private_key, key_path) = match &h.auth {
                    AuthMethod::Password => (false, String::new()),
                    AuthMethod::PrivateKey { key_path } => (true, key_path.clone()),
                };
                ws_state.editing = Some(SshHostDraft {
                    id: Some(h.id.clone()),
                    name: h.name.clone(),
                    host: h.host.clone(),
                    port: h.port.to_string(),
                    username: h.username.clone(),
                    use_private_key,
                    key_path,
                    password: String::new(),
                });
            }
        }
        Message::DraftNameChanged(v) => set_draft(ws_state, |d| d.name = v),
        Message::DraftHostChanged(v) => set_draft(ws_state, |d| d.host = v),
        Message::DraftPortChanged(v) => set_draft(ws_state, |d| d.port = v),
        Message::DraftUsernameChanged(v) => set_draft(ws_state, |d| d.username = v),
        Message::DraftAuthMethodToggled(v) => set_draft(ws_state, |d| d.use_private_key = v),
        Message::DraftKeyPathChanged(v) => set_draft(ws_state, |d| d.key_path = v),
        Message::DraftPasswordChanged(v) => set_draft(ws_state, |d| d.password = v),
        Message::DraftSave => {
            let Some(draft) = ws_state.editing.take() else {
                return;
            };
            if draft.name.trim().is_empty() || draft.host.trim().is_empty() {
                ws_state.editing = Some(draft);
                return;
            }
            let id = draft
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let auth = if draft.use_private_key {
                AuthMethod::PrivateKey {
                    key_path: draft.key_path.trim().to_string(),
                }
            } else {
                AuthMethod::Password
            };
            let host = SshHost {
                id: id.clone(),
                name: draft.name.clone(),
                host: draft.host.trim().to_string(),
                port: draft.port.parse().unwrap_or(22),
                username: draft.username.trim().to_string(),
                auth,
            };
            if let Some(pos) = ws_state.hosts.iter().position(|h| h.id == id) {
                ws_state.hosts[pos] = host;
            } else {
                ws_state.hosts.push(host);
            }
            if !draft.password.is_empty()
                && let Ok(entry) = keyring_entry(project_id, &id)
            {
                let _ = entry.set_password(&draft.password);
            }
            if let Err(e) = save_hosts(repo_path, &ws_state.hosts) {
                tracing::warn!("写入 ssh_hosts.json 失败: {e}");
            }
        }
        Message::DraftCancel => ws_state.editing = None,
        Message::DeleteHost(id) => {
            ws_state.hosts.retain(|h| h.id != id);
            ws_state.test_status.remove(&id);
            ws_state.pending_unknown_keys.remove(&id);
            if let Ok(entry) = keyring_entry(project_id, &id) {
                let _ = entry.delete_credential();
            }
            if let Err(e) = save_hosts(repo_path, &ws_state.hosts) {
                tracing::warn!("写入 ssh_hosts.json 失败: {e}");
            }
        }
        Message::TestConnection(id) => {
            let Some(host) = ws_state.hosts.iter().find(|h| h.id == id).cloned() else {
                return;
            };
            ws_state.test_status.insert(id.clone(), TestStatus::Testing);
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            handle.spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    test_connection(host, password),
                )
                .await;
                match result {
                    Ok(Ok(())) => {
                        emit(Message::TestConnectionResult(project_id, id, Ok(())));
                    }
                    Ok(Err(SshError::UnknownHostKey {
                        fingerprint,
                        key_bytes,
                    })) => {
                        // `emit` 是 `Fn`,不是 `FnOnce`——调用它走 `&self`,
                        // 不会把它消费掉,同一个 `async move` 块里连着调
                        // 两次不需要先 `.clone()` 一份。
                        emit(Message::UnknownKeyDetected(
                            project_id,
                            id.clone(),
                            fingerprint.clone(),
                            key_bytes,
                        ));
                        emit(Message::TestConnectionResult(
                            project_id,
                            id,
                            Err(format!("未知主机,指纹 {fingerprint}——需要确认信任")),
                        ));
                    }
                    Ok(Err(e)) => {
                        emit(Message::TestConnectionResult(
                            project_id,
                            id,
                            Err(e.to_string()),
                        ));
                    }
                    Err(_) => {
                        emit(Message::TestConnectionResult(
                            project_id,
                            id,
                            Err("连接超时(5秒)".to_string()),
                        ));
                    }
                }
            });
        }
        Message::TestConnectionResult(_project_id, id, result) => {
            let status = match result {
                Ok(()) => TestStatus::Ok,
                Err(e) => {
                    // `UnknownKeyDetected` 消息(如果这次测试确实是"未知
                    // 主机"这个原因)已经在下面这个分支之前处理过、把
                    // `UnknownHostKey` 状态写进去了——这里如果状态已经是
                    // `UnknownHostKey` 就不要用泛化的 `Err(String)` 盖掉它。
                    if matches!(
                        ws_state.test_status.get(&id),
                        Some(TestStatus::UnknownHostKey { .. })
                    ) {
                        return;
                    }
                    TestStatus::Err(e)
                }
            };
            ws_state.test_status.insert(id, status);
        }
        Message::UnknownKeyDetected(_project_id, id, fingerprint, key_bytes) => {
            ws_state
                .test_status
                .insert(id.clone(), TestStatus::UnknownHostKey { fingerprint });
            ws_state.pending_unknown_keys.insert(id, key_bytes);
        }
        Message::TrustHostKey(id) => {
            let Some(key_bytes) = ws_state.pending_unknown_keys.remove(&id) else {
                return;
            };
            let Some(host) = ws_state.hosts.iter().find(|h| h.id == id).cloned() else {
                return;
            };
            if let Ok(key) = russh::keys::ssh_key::PublicKey::from_bytes(&key_bytes) {
                let _ = russh::keys::known_hosts::learn_known_hosts(&host.host, host.port, &key);
            }
            // 写完 known_hosts,重新发起一次测试——这次 `check_server_key`
            // 应该在 `check_known_hosts` 里能查到刚写入的记录。
            update(
                ws_state,
                Message::TestConnection(id),
                project_id,
                repo_path,
                handle,
                emit,
            );
        }
    }
}

fn set_draft(ws_state: &mut WorkspaceState, f: impl FnOnce(&mut SshHostDraft)) {
    if let Some(draft) = ws_state.editing.as_mut() {
        f(draft);
    }
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
