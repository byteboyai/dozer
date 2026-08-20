//! SSH 远程主机面板 · 阶段 1:主机连接管理 + 认证 + 连接测试(含标准 SSH
//! host key 验证)。没有 App 级 `AppState`——SSH 不像数据库面板那样有
//! "驱动类型可开关"的全局概念,只有 `WorkspaceState` 挂每个 `Workspace`。
//! SSH 终端(阶段 2)/SFTP(阶段 3)留后续,见
//! `docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`。

use crate::app::{App, HoverId, ssh_tab_hover_key};
use byteui::interaction::icons;
use iced_widget::core::Element;
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, stack, text};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 阶段 3:SFTP 文件传输(数据模型 + 连接生命周期 + 消息路由 + 渲染)。
/// 独立子模块避免 `ssh.rs` 继续膨胀(镜像 `extensions/project/links.rs`)。
pub mod sftp;

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
    /// 约定:`0`=文件传输、`1`=终端、`2`=设置、`3`=删除(见 `host_card`)。悬停
    /// 状态存这里而不是 `App::hover_anims`,因为 `host_card` 是挂在
    /// `WorkspaceState` 上的纯函数,读不到 `&App`;卡片按钮只需"进/出"
    /// 二态高亮即可,不需要侧栏 rail 那种带渐变时长的动画。
    hover_action: Option<(String, u8)>,
    /// 已登录过主机的操作系统(发行版)信息——`(host_id, "ubuntu 22.04.5
    /// LTS")`。只有在主机登录(终端/SFTP 建连成功)后才填充,展示在主机
    /// 卡片名字后面: `cndb (ubuntu 22.04.5 LTS)`。不落盘:跨会话启动时
    /// 主机需要重新登录才会重新采集。
    os_info: HashMap<String, String>,
    /// 正在等用户确认删除的那台主机 `host_id`。`Some` 时主机面板顶部覆盖
    /// 一层确认对话框(注意点删除垃圾桶图标只是把 id 记到这里,真正删记录
    /// 要等用户确认后 `DeleteHost` 才执行)。`None` = 没有待确认的删除。
    delete_confirm: Option<String>,
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
    /// 主机登录后采集到的操作系统(发行版)信息。`None` = 还没登录过
    /// (或登录后采集失败),卡片不展示 OS 段。
    pub fn os_info(&self, host_id: &str) -> Option<&str> {
        self.os_info.get(host_id).map(String::as_str)
    }
    /// 正在等确认删除的主机 `host_id`(`None` = 没有)。给 `view()` 判是否
    /// 覆盖确认对话框。
    pub fn delete_confirm(&self) -> Option<&str> {
        self.delete_confirm.as_deref()
    }
    /// 记"用户点了垃圾桶,想删这台主机"——只记待确认态,真正删除要等
    /// 确认框里的确认按钮(走 `Message::DeleteHost`)。
    pub(crate) fn request_delete(&mut self, host_id: String) {
        self.delete_confirm = Some(host_id);
    }
    /// 取消删除确认(点对话框外的遮罩/取消按钮),清掉待确认态。
    pub(crate) fn cancel_delete(&mut self) {
        self.delete_confirm = None;
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
    /// SFTP 子系统错误(阶段 3)。`russh_sftp::Error` 自带 `Display`,但
    /// 没有现成的 `Into<russh::Error>`/`Into<std::io::Error>`,单独一个
    /// String 变体最干净,错误文案直接给用户看。
    Sftp(String),
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
            SshError::Sftp(e) => write!(f, "SFTP: {e}"),
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

/// 在一条已建好的连接(`handle`)上开一个临时 session channel,执行
/// `cat /etc/os-release`,从输出里解析出操作系统发行版字符串(优先
/// `PRETTY_NAME`)。在主机"登录成功"之后调用 —— `Workspace::spawn_ssh_tab`
/// 的终端任务持有活的 `handle`,内联调用它采集 OS 后 `emit FetchOsResult`。
///
/// 返回值是 `Result<String, String>`(只留错误文案),让调用方不需要把
/// `russh::Error`/IO 错误全部带出去 —— 采集失败只是一条"不展示 OS 段"
/// 的降级,不值得为它撑起一套错误类型。
pub(crate) async fn fetch_os_info(
    handle: &russh::client::Handle<TestHandler>,
) -> Result<String, String> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| e.to_string())?;
    channel
        .exec(true, "cat /etc/os-release")
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = String::new();
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Data { data }) => {
                buf.push_str(&String::from_utf8_lossy(&data));
            }
            Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => break,
            Some(_) => continue,
        }
    }
    // `PRETTY_NAME="Ubuntu 22.04.5 LTS"` → `Ubuntu 22.04.5 LTS`。找不到
    // (非 systemd 发行版也可能没有该字段)就退化成提取 `NAME`/`VERSION`
    // 拼起来;两者都缺才报错。
    let parse = |key: &str| -> Option<String> {
        buf.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim_matches('"').trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(p) = parse("PRETTY_NAME=") {
        return Ok(p);
    }
    if let (Some(name), Some(version)) = (parse("NAME="), parse("VERSION=")) {
        return Ok(format!("{name} {version}"));
    }
    Err("无法识别操作系统".to_string())
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
    /// 点主机卡片垃圾桶图标:进入"待确认删除"态(把 host_id 记进
    /// `delete_confirm`,面板顶部弹确认对话框,此时还没删任何东西)。
    DeleteHostRequest(String),
    /// 关闭删除确认对话框而不删除(点遮罩或取消按钮)。
    DeleteHostCancel,
    /// 用户确认删除:真正从 `hosts` 里移除记录并清理附属状态/磁盘。
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
    /// 主机登录成功后采集操作系统信息(`cat /etc/os-release` 的
    /// `PRETTY_NAME`)的异步结果。采集在 `Workspace::spawn_ssh_tab`/
    /// SFTP 建连成功后的异步任务里内联做(那里持有活的 `handle`),
    /// 完成后经 `emit` 送回这条消息落卡片状态。携带 `project_id`(同
    /// `TestConnectionResult` 的理由)+ `host_id` + 发行版字符串
    /// (`ubuntu 22.04.5 LTS`)。`Err` 时卡片保持不展示 OS 段。
    FetchOsResult(i64, String, Result<String, String>),
    /// SFTP tab 内部交互,嵌套消息(见 `sftp::Message`)。内核按 host_id
    /// 路由到对应 `ws.sftp_tabs` 条目,不会转发到这个模块自己的
    /// `update()`(同 `OpenSshTab`/`CloseSshTab` 的既有拦截模式——大部分
    /// `sftp::Message` 变体要么需要 `&mut Workspace`(发起异步 IO 命令),
    /// 要么需要直接改 `ws.sftp_tabs`,`ssh::update` 只有 `&mut ws.ssh`
    /// 够不到)。
    Sftp(sftp::Message),
    /// 主机卡片的鼠标悬停进/出:内核 `App::update` 里拦截,转成
    /// `Message::Hover(HoverId, bool)` 驱动统一的卡片悬停动画,不会
    /// 转发到 `update`。
    Hover(HoverId, bool),
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
        // 卡片悬停由内核 `App::update` 拦截转发到 `set_hover`,不会到这。
        Message::Hover(_, _) => {}
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
        Message::DeleteHostRequest(id) => ws_state.request_delete(id),
        Message::DeleteHostCancel => ws_state.cancel_delete(),
        Message::DeleteHost(id) => {
            ws_state.hosts.retain(|h| h.id != id);
            ws_state.test_status.remove(&id);
            ws_state.pending_unknown_keys.remove(&id);
            // 这台已经被删了,确认对话框也该一起收起来(id 匹配才清,避免
            // 清掉用户刚点开的、针对另一台主机的待确认态)。
            if ws_state.delete_confirm.as_deref() == Some(id.as_str()) {
                ws_state.delete_confirm = None;
            }
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
        Message::OpenSshTab(..)
        | Message::CloseSshTab(..)
        | Message::SelectSshTab(..)
        | Message::Sftp(..) => {
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
        Message::FetchOsResult(_project_id, host_id, result) => {
            // 只在成功时写入——失败保持"不展示 OS 段"。不因后来的失败
            // 覆盖掉已采集到的成功结果(主机可能开过终端、也开过 SFTP,
            // 采集发生的顺序不定)。
            if let Ok(os) = result {
                ws_state.os_info.insert(host_id, os);
            }
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
    os_info: Option<&'a str>,
    hovered: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 卡片上三个图标按钮的悬停高亮:只有"正在 hover 的那颗"是满 GOLD,
    // 其余 DIM。按钮位约定:0=文件传输、1=终端、2=设置、3=删除。
    let is_hover = |idx: u8| {
        hover_action
            .as_ref()
            .is_some_and(|(h, i)| h == &host.id && *i == idx)
    };
    let icon_btn = |kind: icons::IconKind, on_select: Message, tooltip: &'a str, idx: u8| {
        byteui::interaction::icons::icon_button_entry(
            kind,
            byteui::theme::icon_size::row(),
            /* active */ false,
            /* dim */ false,
            /* hover_t */ if is_hover(idx) { 1.0 } else { 0.0 },
            /* card */ true,
            byteui::theme::icon_size::row() + 10.0,
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
            byteui::interaction::icons::IconKind::FolderSync,
            Message::OpenSshTab(host.id.clone(), SshTabKind::Sftp),
            "文件传输",
            0,
        ),
        icon_btn(
            byteui::interaction::icons::IconKind::Terminal,
            Message::OpenSshTab(host.id.clone(), SshTabKind::Terminal),
            "终端",
            1,
        ),
        icon_btn(
            byteui::interaction::icons::IconKind::Settings,
            Message::EditHostStart(host.id.clone()),
            "设置",
            2,
        ),
        icon_btn(
            byteui::interaction::icons::IconKind::Trash,
            Message::DeleteHostRequest(host.id.clone()),
            "删除",
            3,
        ),
    ]
    .spacing(6);
    if matches!(status, TestStatus::UnknownHostKey { .. }) {
        actions = actions.push(
            button(
                text("信任并重试")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().gold),
            )
            .on_press(Message::TrustHostKey(host.id.clone()))
            .padding([4, 8])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                border: iced_widget::core::Border {
                    color: byteui::theme::color::current().gold,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..button::Style::default()
            }),
        );
    }

    // 主机卡片三行:①名字(已登录过的主机展示 `名 (发行版)`)②连接信息
    // `user@host:port` ③操作图标按钮整组右对齐。
    let name = match os_info {
        Some(os) => format!("{} ({os})", host.name),
        None => host.name.clone(),
    };
    let card = container(
        column![
            text(name)
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream),
            text(format!("{}@{}:{}", host.username, host.host, host.port))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
            container(actions)
                .width(iced_widget::core::Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Right),
        ]
        .spacing(6)
        .align_x(iced_widget::core::alignment::Horizontal::Left),
    )
    .padding(10)
    .width(iced_widget::core::Length::Fill)
    .style(move |_t: &iced_widget::Theme| {
        byteui::interaction::cards::container_card(
            false,
            hovered,
            byteui::theme::color::current().card,
        )
    });
    MouseArea::new(card)
        .on_enter(Message::Hover(
            HoverId::HostCard(ssh_tab_hover_key(&host.id)),
            true,
        ))
        .on_exit(Message::Hover(
            HoverId::HostCard(ssh_tab_hover_key(&host.id)),
            false,
        ))
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
                    Some(byteui::theme::color::current().gold.into())
                } else {
                    None
                },
                border: iced_widget::core::Border {
                    color: if selected {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().border
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
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream),
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
        byteui::form::input_text::view(
            "主机名称",
            &draft.name,
            false,
            None,
            false,
            None,
            Message::DraftNameChanged,
        ),
        byteui::form::input_text::view(
            "Host",
            &draft.host,
            false,
            None,
            false,
            None,
            Message::DraftHostChanged,
        ),
        byteui::form::input_text::view(
            "port(22)",
            &draft.port,
            false,
            None,
            false,
            None,
            Message::DraftPortChanged,
        ),
        byteui::form::input_text::view(
            "user name",
            &draft.username,
            false,
            None,
            false,
            None,
            Message::DraftUsernameChanged,
        ),
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
        col = col.push(byteui::form::input_text::view(
            "私钥文件路径,如 ~/.ssh/id_ed25519",
            &draft.key_path,
            false,
            None,
            false,
            None,
            Message::DraftKeyPathChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "私钥口令(留空则不修改/无口令)",
            &draft.password,
            true,
            None,
            false,
            None,
            Message::DraftPasswordChanged,
        ));
    } else {
        col = col.push(byteui::form::input_text::view(
            "password(留空则不修改)",
            &draft.password,
            true,
            None,
            false,
            None,
            Message::DraftPasswordChanged,
        ));
    }

    let text_btn = |label: &'a str, color: iced_widget::core::Color, msg: Message| {
        button(text(label).size(byteui::theme::font::label()).color(color))
            .on_press(msg)
            .padding([6, 12])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: iced_widget::core::Border {
                    color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: color,
                ..button::Style::default()
            })
    };

    // 按钮分两组:"测试连接"靠左;保存/取消(以及编辑态才有的删除)靠右,
    // 中间用 `Fill` 空位把两组顶到卡片两端。
    let left = row![text_btn(
        "测试连接",
        byteui::theme::color::current().cream,
        Message::TestConnection(draft.id.clone().unwrap_or_default(),)
    )]
    .spacing(6);
    // 新建主机(没有 id)时"测试连接"点了也是 no-op(TestConnection 在
    // ws_state.hosts 里查不到这个空字符串 id,直接 return——见
    // ssh::update 的既有实现),"删除"按钮干脆不渲染,没有可删的对象。
    let mut right = row![].spacing(6);
    if let Some(id) = &draft.id {
        right = right.push(text_btn(
            "删除",
            byteui::theme::color::current().red,
            Message::DeleteHost(id.clone()),
        ));
    }
    right = right.push(text_btn(
        "保存",
        byteui::theme::color::current().cream,
        Message::DraftSave,
    ));
    right = right.push(text_btn(
        "取消",
        byteui::theme::color::current().dim,
        Message::DraftCancel,
    ));
    let buttons = row![
        left,
        iced_widget::Space::new().width(iced_widget::core::Length::Fill),
        right,
    ]
    .spacing(6)
    .align_y(iced_widget::core::alignment::Vertical::Center);
    col = col.push(buttons);

    let (status_text, status_color) = match status {
        TestStatus::Idle => (String::new(), byteui::theme::color::current().dim),
        TestStatus::Testing => ("测试中…".to_string(), byteui::theme::color::current().dim),
        TestStatus::Ok => (
            "✓ 连接成功".to_string(),
            byteui::theme::color::current().green,
        ),
        TestStatus::UnknownHostKey { fingerprint } => (
            format!("⚠ 未知主机,指纹 {fingerprint}"),
            byteui::theme::color::current().gold,
        ),
        TestStatus::KeyChanged { fingerprint } => (
            format!("✗ 主机指纹已变化({fingerprint}),拒绝连接"),
            byteui::theme::color::current().red,
        ),
        TestStatus::Err(e) => (format!("✗ {e}"), byteui::theme::color::current().red),
    };
    if !status_text.is_empty() {
        col = col.push(
            text(status_text)
                .size(byteui::theme::font::caption_sm())
                .color(status_color),
        );
    }
    if matches!(status, TestStatus::UnknownHostKey { .. })
        && let Some(id) = &draft.id
    {
        col = col.push(text_btn(
            "信任并重试",
            byteui::theme::color::current().gold,
            Message::TrustHostKey(id.clone()),
        ));
    }

    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fill)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            border: iced_widget::core::Border {
                color: byteui::theme::color::current().gold,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}

pub fn view<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    width: iced_widget::core::Length,
    outer: iced_widget::core::Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let head =
        crate::homespace::home_panel_head(byteui::interaction::icons::IconKind::Server, "主机");

    let mut list = column![].spacing(12).padding(8);

    if ws_state.hosts().is_empty() {
        list = list.push(
            text("还没有主机")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        for h in ws_state.hosts() {
            let hovered = app.hover_progress(HoverId::HostCard(ssh_tab_hover_key(&h.id))) > 0.0;
            list = list.push(host_card(
                h,
                ws_state.test_status(&h.id),
                ws_state.hover_action(),
                ws_state.os_info(&h.id),
                hovered,
            ));
        }
    }

    if let Some(draft) = ws_state.editing() {
        let status = draft
            .id
            .as_deref()
            .map(|id| ws_state.test_status(id))
            .unwrap_or(&TestStatus::Idle);
        list = list.push(host_form(draft, status));
    }

    let scroll = Scrollable::new(list)
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fill)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

    // 顶部面板标题固定、中间主机列表可滚动、底部「＋添加」footer-bar 固定
    // 在面板最下方——footer-bar 不随主机列表滚动,始终可见(见 `ssh_footer_bar`)。
    let body = column![head, scroll, ssh_footer_bar()].spacing(0);

    let base = container(body)
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        );

    // 有"待确认删除的主机"时,在面板上叠一层半透明遮罩 + 确认对话框;
    // 点遮罩(或对话框的取消)回 `DeleteHostCancel` 收起,确认才真删。
    if let Some(host_id) = ws_state.delete_confirm() {
        let dismiss = MouseArea::new(
            container(column![])
                .width(iced_widget::core::Length::Fill)
                .height(iced_widget::core::Length::Fill)
                .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(byteui::theme::color::current().scrim.into()),
                    ..iced_widget::container::Style::default()
                }),
        )
        .on_press(Message::DeleteHostCancel);
        return stack![base, dismiss, delete_confirm_popup(ws_state, host_id)]
            .width(width)
            .height(iced_widget::core::Length::Fill)
            .into();
    }

    base.into()
}

/// 删除主机的确认对话框:居中卡片,列出要删的主机名,确认(红)才执行
/// `DeleteHost`,取消/遮罩只清待确认态。视觉参照文件树面板的
/// `delete_confirm_popup`(CARD 底 + 圆角描边 + 取消/确认两个圆角按钮)。
fn delete_confirm_popup<'a>(
    ws_state: &'a WorkspaceState,
    host_id: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let name = ws_state
        .hosts()
        .iter()
        .find(|h| h.id == host_id)
        .map(|h| h.name.as_str())
        .unwrap_or(host_id);
    let cancel = button(
        text("取消")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::DeleteHostCancel)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().cream,
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });
    let confirm = button(
        text("删除")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::DeleteHost(host_id.to_string()))
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().red,
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().red,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let dialog = container(
        column![
            text(format!("删除主机 \"{name}\"?"))
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().cream),
            text("这会永久删除这台主机的连接记录。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            row![cancel, confirm].spacing(8),
        ]
        .spacing(8),
    )
    .padding(16)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(byteui::theme::color::current().card.into()),
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..iced_widget::container::Style::default()
    });

    container(dialog)
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 主机面板底部 footer-bar:1px `BORDER` 分隔线 + `padding([6, 8])` 容器,
/// 结构与项目面板的 `project_footer_bar` / 文件树面板的 `git_footer_bar`
/// 一致。当前放「＋添加」单个按钮,底部固定,不随主机列表滚动。
fn ssh_footer_bar() -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let add_btn = button(
        text("＋添加")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().gold),
    )
    .on_press(Message::AddHostStart)
    .padding([8, 16])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().gold,
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: byteui::theme::color::current().gold,
        ..button::Style::default()
    });

    let bar = row![add_btn].align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..iced_widget::container::Style::default()
        });

    container(column![top_line, bar].spacing(4))
        .width(iced_widget::core::Length::Fill)
        .padding([6, 8])
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            ..iced_widget::container::Style::default()
        })
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
