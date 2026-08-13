//! SSH 远程主机面板 · 阶段 1:主机连接管理 + 认证 + 连接测试(含标准 SSH
//! host key 验证)。没有 App 级 `AppState`——SSH 不像数据库面板那样有
//! "驱动类型可开关"的全局概念,只有 `WorkspaceState` 挂每个 `Workspace`。
//! SSH 终端(阶段 2)/SFTP(阶段 3)留后续,见
//! `docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`。

use crate::icons;
use crate::theme;
use iced_widget::core::Element;
use iced_widget::{button, column, container, row, text, text_input};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// SSH 面板自己 tab 条上的 tab 种类。同一台主机可以同时开一个 `Terminal`
/// tab 和(阶段 3 起)一个 `Sftp` tab,两者独立存在、独立连接——`(host_id,
/// SshTabKind)` 是一个 SSH 面板 tab 的完整身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SshTabKind {
    Terminal,
    Sftp,
}

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
    /// 信任 host key 后要不要自动重开终端(而不是阶段 1 默认的"重新测试
    /// 连接")——存的是"上一次点'终端'按钮、连接过程中撞上未知 key 那台
    /// 主机的 id"。`TrustHostKey` 分支里核对是否等于本次信任的 host id,
    /// 相等才重开终端,否则落回测试连接。字段只有一个槽位,极少数"两台
    /// 主机同时未知 key、按点终端的顺序信任"场景下,后点的会覆盖先点的
    /// 意图,顶多导致"该开终端却重新测试连接"这种安全的降级,不会误开
    /// 错主机的终端或者崩溃(设计文档 §6 的论证)。
    reopen_after_trust: Option<String>,
    /// 主机卡片上当前鼠标悬停的图标按钮——`(host_id, 按钮位)`。按钮位
    /// 约定:`0`=文件传输、`1`=终端、`2`=设置(见 `host_card`)。悬停
    /// 状态存这里而不是 `App::hover_anims`,因为 `host_card` 是挂在
    /// `WorkspaceState` 上的纯函数,读不到 `&App`;卡片按钮只需"进/出"
    /// 二态高亮即可,不需要侧栏 rail 那种带渐变时长的动画。
    hover_action: Option<(String, u8)>,
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
    /// 主机卡片当前悬停的图标按钮(`(host_id, 按钮位)`),给 `view()` 传进
    /// `host_card` 算每颗按钮的高亮。
    pub(crate) fn hover_action(&self) -> &Option<(String, u8)> {
        &self.hover_action
    }
    /// 记"点了终端按钮的这台主机,如果接下来撞上未知 host key,信任后要
    /// 自动重开终端"(内核 `App::update` 的 `OpenSshTab` 拦截分支调用;
    /// 字段本身私有,不能让内核直接赋值)。
    pub(crate) fn record_reopen_after_trust(&mut self, host_id: String) {
        self.reopen_after_trust = Some(host_id);
    }
}

/// `TrustHostKey` 里"信任后是重开终端还是重新测试连接"的纯判定:id 命中
/// `reopen_after_trust`(上次点终端撞未知 key 的那台)才重开终端并清槽位,
/// 否则落回测试连接。拆出来是为了能被无网络单测直接测,不碰
/// `known_hosts`。
pub(crate) fn pick_retry_message(reopen: &mut Option<String>, id: &str) -> Message {
    if reopen.as_deref() == Some(id) {
        *reopen = None;
        Message::OpenSshTab(id.to_string(), SshTabKind::Terminal)
    } else {
        Message::TestConnection(id.to_string())
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

/// 从 Keychain 读某台主机的密码/私钥口令,读不到按无密码处理(不 panic,
/// 同阶段 1 `TestConnection` 分支的既有口径)。`spawn_ssh_tab`/`update`
/// 里的 `TestConnection` 分支共用这个,不重复写 `.ok().and_then(..)`。
pub(crate) fn keyring_password(project_id: i64, host_id: &str) -> Option<String> {
    keyring_entry(project_id, host_id)
        .ok()
        .and_then(|e| e.get_password().ok())
}

/// 合成 SSH tab 的 `SessionInfo`(`dozer_core::protocol::SessionInfo` 没有
/// `Default` impl,9 个字段全要给值)。`command`/`created_ms` 现状全仓库
/// 没有任何地方读取,给有意义但不影响功能的值,不留空。
pub(crate) fn synth_session_info(
    host: &SshHost,
    project_id: i64,
) -> dozer_core::protocol::SessionInfo {
    dozer_core::protocol::SessionInfo {
        id: format!("ssh:{}", host.id),
        name: format!("ssh: {}", host.name),
        command: format!("ssh {}@{}:{}", host.username, host.host, host.port),
        cwd: "~".to_string(),
        alive: true,
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        agent_state: dozer_core::protocol::AgentState::Idle,
        transcript_path: None,
        project_id: Some(project_id),
        agent: dozer_core::protocol::AgentKind::Unknown,
    }
}

#[derive(Debug)]
pub(crate) enum SshError {
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

pub(crate) struct TestHandler {
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

/// SSH 握手共享段。connect + host key 校验(`TestHandler::check_server_key`)加认证,返回 `Handle` 本身。
///
/// 调用方若要继续开 channel(阶段 2 终端),`Handle` 必须留在作用域内全程存活到 channel 读写半都不再用为止(见设计文档"背景"末尾;不能在这个函数里把 `Handle` 提前丢弃只返回别的东西)。`test_connection` 只需要确认握手成功,用完直接让 `handle` 在函数结尾正常析构(不开 channel,没有"提前丢弃"的风险)。`pub(crate)` 是因为阶段 2 的 `Workspace::spawn_ssh_tab`(在 `workspace.rs`,另一个模块)要直接调它来建终端连接。
pub(crate) async fn handshake(
    host: &SshHost,
    password: Option<String>,
) -> Result<russh::client::Handle<TestHandler>, SshError> {
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
        russh::client::AuthResult::Success => Ok(handle),
        russh::client::AuthResult::Failure { .. } => Err(SshError::AuthFailed),
    }
}

async fn test_connection(host: SshHost, password: Option<String>) -> Result<(), SshError> {
    handshake(&host, password).await?;
    Ok(())
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
    /// 测试过程中发现 host key 变了(known_hosts 里有同算法但公钥不同的
    /// 记录,可能中间人攻击)——直接拒绝,没有"信任并继续"这个选项。
    /// 携带 `project_id`、`host_id`、指纹文案。
    KeyChanged(i64, String, String),
    /// 用户确认信任某台主机的 host key:写入 known_hosts,然后重新发起
    /// 一次 `TestConnection`。
    TrustHostKey(String),
    /// 点主机卡片"终端"/"文件传输"图标:内核 `App::update` 里有专门的
    /// 拦截分支(见 `workspace.rs`),真正的 tab 创建逻辑在那边——这个
    /// 变体在 `ssh::update` 自己的 `match` 里只是穷尽匹配需要,不会真的
    /// 走到这里(内核在通配 `Message::Ssh(msg)` 之前就拦截了)。
    OpenSshTab(String, SshTabKind),
    /// 关闭 SSH 面板某个 tab(点 tab 条的 ✕)。同上,内核拦截。
    CloseSshTab(String, SshTabKind),
    /// 切换 SSH 面板当前显示哪个 tab(点 tab 条里非当前的一个)。同上,
    /// 内核拦截(需要 `&mut Workspace` 设 `ssh_active`)。
    SelectSshTab(String, SshTabKind),
    /// 主机卡片图标按钮的鼠标悬停进/出:更新 `ws_state.hover_action`,
    /// 驱动卡片上对应按钮的 DIM→GOLD 高亮。图标按钮的 `on_enter`/`on_exit`
    /// 事件由 `icons::icon_button_entry` 接好,这里只落状态。
    HoverAction(Option<(String, u8)>),
    /// 终端连接失败的异步结果,带 `project_id`(异步结果不能假设聚焦
    /// 项目没变,同 `TestConnectionResult`)。同上,内核在 `ssh::update`
    /// 之前会先做 `pending`/`ssh_out_pending` 清理,这里只负责落卡片
    /// 状态(与 `TestConnectionResult`/`UnknownKeyDetected`/`KeyChanged`
    /// 共用同一列卡片状态,不新增第二列)。
    TerminalConnectFailed(i64, String, usize, String),
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
        Message::HoverAction(v) => ws_state.hover_action = v,
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
            let password = keyring_password(project_id, &id);
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
                    Ok(Err(SshError::KeyChanged { fingerprint })) => {
                        // 同上面的 UnknownHostKey:先发 `KeyChanged`(带指纹,
                        // 让 UI 落成明确的"指纹已变化·拒绝连接"状态),再发
                        // 一条 `TestConnectionResult` 兜底文案。
                        emit(Message::KeyChanged(
                            project_id,
                            id.clone(),
                            fingerprint.clone(),
                        ));
                        emit(Message::TestConnectionResult(
                            project_id,
                            id,
                            Err(format!("主机指纹已变化({fingerprint}),拒绝连接")),
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
                    // `KeyChanged` 同理:上面 `KeyChanged` 分支已经把它落成
                    // 明确的"指纹已变化"状态,也不能被泛化错误覆盖。
                    if matches!(
                        ws_state.test_status.get(&id),
                        Some(TestStatus::UnknownHostKey { .. } | TestStatus::KeyChanged { .. })
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
        Message::KeyChanged(_project_id, id, fingerprint) => {
            ws_state
                .test_status
                .insert(id, TestStatus::KeyChanged { fingerprint });
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
            // 写完 known_hosts,按"是不是上次点终端撞未知 key 的那台主机"
            // 决定重开终端还是重新测试连接(设计文档 §6)。
            //
            // `OpenSshTab` 不能走下面 `update(ws_state, ..)` 这条本地递归
            // 调用——`update` 只有 `&mut WorkspaceState`,够不到
            // `spawn_ssh_tab` 需要的 `&mut Workspace`,递归调用只会落进
            // `ssh::update` 自己那个空转的 `OpenSshTab(..) => {}` 分支,
            // 终端永远不会真的打开(而且此时 `pending_unknown_keys` 已经
            // 被上面 `remove` 清空,再点一次"信任并重试"也无法重试)。必须
            // 走 `emit` 把消息送回事件循环,才能命中内核 `App::update` 里
            // 那条专门调 `Workspace::spawn_ssh_tab` 的拦截分支。
            let retry = pick_retry_message(&mut ws_state.reopen_after_trust, &id);
            match retry {
                Message::OpenSshTab(..) => emit(retry),
                other => update(ws_state, other, project_id, repo_path, handle, emit),
            }
        }
        Message::OpenSshTab(..) | Message::CloseSshTab(..) | Message::SelectSshTab(..) => {
            // 内核 `App::update` 在通配 `Message::Ssh(msg)` 之前拦截,
            // 这里理论上到不了;写出来只是为了 `match` 穷尽。
        }
        Message::TerminalConnectFailed(_project_id, host_id, _tab_id, err) => {
            // 内核已经在转发之前清过 pending/ssh_out_pending,这里只需要
            // 落卡片状态——与 TestConnectionResult 的 Err 分支同一套过期
            // 防线(不得覆盖 UnknownHostKey/KeyChanged)。
            if matches!(
                ws_state.test_status.get(&host_id),
                Some(TestStatus::UnknownHostKey { .. } | TestStatus::KeyChanged { .. })
            ) {
                return;
            }
            ws_state.test_status.insert(host_id, TestStatus::Err(err));
        }
    }
}

fn set_draft(ws_state: &mut WorkspaceState, f: impl FnOnce(&mut SshHostDraft)) {
    if let Some(draft) = ws_state.editing.as_mut() {
        f(draft);
    }
}

fn host_card<'a>(
    host: &'a SshHost,
    status: &'a TestStatus,
    hover_action: &'a Option<(String, u8)>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 卡片上三个图标按钮的悬停高亮:只有"正在 hover 的那颗"是满 GOLD,
    // 其余 DIM。按钮位约定:0=文件传输、1=终端、2=设置。
    let is_hover = |idx: u8| {
        hover_action
            .as_ref()
            .is_some_and(|(h, i)| h == &host.id && *i == idx)
    };
    let icon_btn = |kind: icons::IconKind, on_select: Message, tooltip: &'a str, idx: u8| {
        crate::icons::icon_button_entry(
            kind,
            crate::theme::icon_size::row(),
            /* active */ false,
            /* hover_t */ if is_hover(idx) { 1.0 } else { 0.0 },
            /* card */ true,
            crate::theme::icon_size::row() + 10.0,
            /* interactive */ true,
            on_select,
            /* on_hover */
            move |hovered| {
                Message::HoverAction(if hovered {
                    Some((host.id.clone(), idx))
                } else {
                    None
                })
            },
            tooltip,
        )
    };

    let mut actions = row![
        icon_btn(
            crate::icons::IconKind::FolderSync,
            Message::OpenSshTab(host.id.clone(), SshTabKind::Sftp),
            "文件传输",
            0,
        ),
        icon_btn(
            crate::icons::IconKind::Terminal,
            Message::OpenSshTab(host.id.clone(), SshTabKind::Terminal),
            "终端",
            1,
        ),
        icon_btn(
            crate::icons::IconKind::Settings,
            Message::EditHostStart(host.id.clone()),
            "设置",
            2,
        ),
    ]
    .spacing(6);
    if matches!(status, TestStatus::UnknownHostKey { .. }) {
        actions = actions.push(
            button(
                text("信任并重试")
                    .size(theme::font::caption())
                    .color(theme::color::GOLD),
            )
            .on_press(Message::TrustHostKey(host.id.clone()))
            .padding([4, 8])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                border: iced_widget::core::Border {
                    color: theme::color::GOLD,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..button::Style::default()
            }),
        );
    }

    container(
        column![
            row![
                text(host.name.clone())
                    .size(theme::font::body())
                    .color(theme::color::CREAM),
                text(format!("{}@{}:{}", host.username, host.host, host.port))
                    .size(theme::font::caption_sm())
                    .color(theme::color::DIM),
            ]
            .spacing(8),
            actions,
        ]
        .spacing(8),
    )
    .padding(10)
    .width(iced_widget::core::Length::Fill)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(theme::color::CARD.into()),
        border: iced_widget::core::Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}

/// 单选圆点:选中态 `GOLD` 实心 + `GOLD` 描边,未选中态空心 `BORDER`
/// 描边。iced 没有原生 radio 部件,手绘一个圆形 `container` + `MouseArea`。
fn radio_dot<'a>(
    label: &'a str,
    selected: bool,
    on_select: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let dot = container(iced_widget::Space::new())
        .width(iced_widget::core::Length::Fixed(10.0))
        .height(iced_widget::core::Length::Fixed(10.0))
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: if selected {
                    Some(theme::color::GOLD.into())
                } else {
                    None
                },
                border: iced_widget::core::Border {
                    color: if selected {
                        theme::color::GOLD
                    } else {
                        theme::color::BORDER
                    },
                    width: 1.5,
                    radius: 5.0.into(),
                },
                ..iced_widget::container::Style::default()
            },
        );
    let ring = container(dot)
        .width(iced_widget::core::Length::Fixed(16.0))
        .height(iced_widget::core::Length::Fixed(16.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    iced_widget::MouseArea::new(
        row![
            ring,
            text(label)
                .size(theme::font::body())
                .color(theme::color::CREAM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_press(on_select)
    .into()
}

fn host_form<'a>(
    draft: &'a SshHostDraft,
    status: &'a TestStatus,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![
        text_input("主机名称", &draft.name)
            .on_input(Message::DraftNameChanged)
            .size(theme::font::body()),
        text_input("Host", &draft.host)
            .on_input(Message::DraftHostChanged)
            .size(theme::font::body()),
        text_input("port(22)", &draft.port)
            .on_input(Message::DraftPortChanged)
            .size(theme::font::body()),
        text_input("user name", &draft.username)
            .on_input(Message::DraftUsernameChanged)
            .size(theme::font::body()),
        row![
            radio_dot(
                "密码",
                !draft.use_private_key,
                Message::DraftAuthMethodToggled(false)
            ),
            radio_dot(
                "私钥",
                draft.use_private_key,
                Message::DraftAuthMethodToggled(true)
            ),
        ]
        .spacing(20),
    ]
    .spacing(10);

    if draft.use_private_key {
        col = col.push(
            text_input("私钥文件路径,如 ~/.ssh/id_ed25519", &draft.key_path)
                .on_input(Message::DraftKeyPathChanged)
                .size(theme::font::body()),
        );
        col = col.push(
            text_input("私钥口令(留空则不修改/无口令)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(theme::font::body()),
        );
    } else {
        col = col.push(
            text_input("password(留空则不修改)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(theme::font::body()),
        );
    }

    let text_btn = |label: &'a str, color: iced_widget::core::Color, msg: Message| {
        button(text(label).size(theme::font::label()).color(color))
            .on_press(msg)
            .padding([6, 12])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: Some(theme::color::BG.into()),
                border: iced_widget::core::Border {
                    color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: color,
                ..button::Style::default()
            })
    };

    let mut buttons = row![text_btn(
        "测试连接",
        theme::color::CREAM,
        Message::TestConnection(draft.id.clone().unwrap_or_default(),)
    )]
    .spacing(6);
    // 新建主机(没有 id)时"测试连接"点了也是 no-op(TestConnection 在
    // ws_state.hosts 里查不到这个空字符串 id,直接 return——见
    // ssh::update 的既有实现),"删除"按钮干脆不渲染,没有可删的对象。
    if let Some(id) = &draft.id {
        buttons = buttons.push(text_btn(
            "删除",
            theme::color::RED,
            Message::DeleteHost(id.clone()),
        ));
    }
    buttons = buttons.push(text_btn("保存", theme::color::GOLD, Message::DraftSave));
    buttons = buttons.push(text_btn("取消", theme::color::DIM, Message::DraftCancel));
    col = col.push(buttons);

    let (status_text, status_color) = match status {
        TestStatus::Idle => (String::new(), theme::color::DIM),
        TestStatus::Testing => ("测试中…".to_string(), theme::color::DIM),
        TestStatus::Ok => ("✓ 连接成功".to_string(), theme::color::GREEN),
        TestStatus::UnknownHostKey { fingerprint } => {
            (format!("⚠ 未知主机,指纹 {fingerprint}"), theme::color::GOLD)
        }
        TestStatus::KeyChanged { fingerprint } => (
            format!("✗ 主机指纹已变化({fingerprint}),拒绝连接"),
            theme::color::RED,
        ),
        TestStatus::Err(e) => (format!("✗ {e}"), theme::color::RED),
    };
    if !status_text.is_empty() {
        col = col.push(
            text(status_text)
                .size(theme::font::caption_sm())
                .color(status_color),
        );
    }
    if matches!(status, TestStatus::UnknownHostKey { .. })
        && let Some(id) = &draft.id
    {
        col = col.push(text_btn(
            "信任并重试",
            theme::color::GOLD,
            Message::TrustHostKey(id.clone()),
        ));
    }

    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fill)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(theme::color::CARD.into()),
            border: iced_widget::core::Border {
                color: theme::color::GOLD,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}

pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: iced_widget::core::Length,
    outer: iced_widget::core::Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![crate::homespace::home_panel_head(
        crate::icons::IconKind::Server,
        "主机"
    )]
    .spacing(12);

    if ws_state.hosts().is_empty() {
        col = col.push(
            text("还没有主机")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else {
        for h in ws_state.hosts() {
            col = col.push(host_card(
                h,
                ws_state.test_status(&h.id),
                ws_state.hover_action(),
            ));
        }
    }

    if let Some(draft) = ws_state.editing() {
        let status = draft
            .id
            .as_deref()
            .map(|id| ws_state.test_status(id))
            .unwrap_or(&TestStatus::Idle);
        col = col.push(host_form(draft, status));
    }

    col = col.push(
        button(
            text("＋添加")
                .size(theme::font::body())
                .color(theme::color::GOLD),
        )
        .on_press(Message::AddHostStart)
        .padding([8, 16])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::color::BG.into()),
            border: iced_widget::core::Border {
                color: theme::color::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            text_color: theme::color::GOLD,
            ..button::Style::default()
        }),
    );

    container(col.padding(16))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::BG.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
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

    #[test]
    fn synth_session_info_shape() {
        let host = host("h1");
        let info = synth_session_info(&host, 7);
        assert_eq!(info.id, "ssh:h1");
        assert!(info.name.starts_with("ssh: "));
        assert_eq!(info.project_id, Some(7));
        assert!(info.alive);
        assert_eq!(info.agent, dozer_core::protocol::AgentKind::Unknown);
    }

    #[test]
    fn trust_host_key_reopens_terminal_only_when_ids_match() {
        let mut reopen = Some("A".to_string());
        // id 命中 → 重开终端并清槽位。
        assert!(matches!(
            pick_retry_message(&mut reopen, "A"),
            Message::OpenSshTab(_, SshTabKind::Terminal)
        ));
        assert_eq!(reopen, None);
        // 槽位清空后(id 不再匹配),信任别的/同一台都应落回测试连接。
        let mut reopen2 = Some("B".to_string());
        assert!(matches!(
            pick_retry_message(&mut reopen2, "A"),
            Message::TestConnection(_)
        ));
        assert_eq!(reopen2.as_deref(), Some("B"));
        // 与 reopen_after_trust 无关的直接测试连接路径。
        assert!(matches!(
            pick_retry_message(&mut None, "A"),
            Message::TestConnection(_)
        ));
    }
}
