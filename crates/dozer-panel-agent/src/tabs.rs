//! 页签/会话的纯逻辑与字节流去向类型(从 `dozer-app` 的 `workspace`
//! 搬入;S6c-1 先搬纯函数与 `TabBackend`,会话状态在 Task 4 接入)。

use dozer_model::{AgentKind, AgentState, SessionInfo};
use dozer_panel_term::model::TerminalModel;
use dozer_panel_term::osc::OscScanner;
use dozer_services::SessionService;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// UI → 会话后端的写指令。SSH 通道同时承载数据与尺寸,所以不是裸
/// `Vec<u8>`(设计 §6.2 的 `Custom(mpsc::UnboundedSender<Vec<u8>>)` 由此
/// 细化,见计划 D7)。
#[derive(Debug)]
pub enum TabOut {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// 一个终端 tab 的字节流去向。
#[derive(Debug)]
pub enum TabBackend {
    /// 守护进程托管的本地 PTY。
    Daemon,
    /// 自定义通道(SSH channel 等),UI 侧写操作经 `out` 送进泵任务。
    Custom(tokio::sync::mpsc::UnboundedSender<TabOut>),
}

/// 单个会话的工作区展示态,来自 bytegit 的 `Repo::head`(分支)与 `Repo::is_dirty`。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceGitInfo {
    pub branch: Option<String>,
    pub dirty: bool,
}

/// 一个 tab 对应一个守护进程会话(或 SSH 通道)。
pub struct SessionTab {
    pub info: SessionInfo,
    pub model: TerminalModel,
    pub alive: bool,
    /// 会话内 agent 的最新状态(hook 事件驱动;初值来自
    /// `SessionInfo.agent_state`)。
    pub agent_state: AgentState,
    /// 会话归属的 agent(初值来自 `SessionInfo.agent`,随状态事件更新)。
    pub agent: AgentKind,
    pub transcript_path: Option<String>,
    /// OSC 扫描器(每 tab 独立,序列可跨 chunk)。
    pub osc: OscScanner,
    /// OSC 7 上报的当前目录;tab 标题优先显示其 basename。
    pub cwd: Option<PathBuf>,
    /// OSC 133;D 上报的最近命令退出码;非零时终端栏红字提示。
    pub last_exit: Option<i32>,
    /// 从 transcript 尾部提取的模型 id(渲染时经 `format_model_label`)。
    pub llm_model: Option<String>,
    /// 同上,来自 transcript 顶层 `permissionMode`。
    pub permission_mode: Option<String>,
    /// transcript 尾部提取的最后一句人类发言/AI 回复摘要。
    pub last_activity: Option<String>,
    /// 工作区分支/脏标覆盖:仅当 `effective_cwd()` 偏离项目根时填充。
    pub workspace_override: Option<WorkspaceGitInfo>,
    /// 稳定 id,异步事件按它路由,不受 Vec 位置变化影响。
    pub tab_id: usize,
    /// attach 数据流的转发任务句柄。关 tab 时 `abort()`——任务是
    /// `mpsc::UnboundedReceiver<SessionEvent>` 的唯一持有者,中断即 detach。
    pub forwarder: tokio::task::JoinHandle<()>,
    /// 字节流去向。
    pub backend: TabBackend,
}

/// 会话当前有效工作目录:OSC 7 跟踪的实时 cwd 优先,回落到会话启动目录。
pub fn effective_cwd(osc_cwd: Option<&Path>, spawn_cwd: &str) -> PathBuf {
    match osc_cwd {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(spawn_cwd),
    }
}

impl SessionTab {
    pub fn effective_cwd(&self) -> PathBuf {
        effective_cwd(self.cwd.as_deref(), &self.info.cwd)
    }

    /// 把一段会话输出送进 OSC 扫描器并落地状态(观察式,不改写字节)。
    pub fn ingest_osc(&mut self, bytes: &[u8]) {
        for ev in self.osc.feed(bytes) {
            match ev {
                dozer_panel_term::osc::OscEvent::Cwd(p) => self.cwd = Some(p),
                dozer_panel_term::osc::OscEvent::CmdStart => self.last_exit = None,
                dozer_panel_term::osc::OscEvent::CmdExit(code) => self.last_exit = Some(code),
            }
        }
    }
}

/// 把字节写进一个 tab(按后端分流)。
pub fn send_to_tab(
    tab: &SessionTab,
    svc: &Arc<dyn SessionService>,
    handle: &tokio::runtime::Handle,
    bytes: Vec<u8>,
) {
    if !tab.alive {
        return;
    }
    match &tab.backend {
        TabBackend::Daemon => {
            let svc = Arc::clone(svc);
            let id = tab.info.id.clone();
            handle.spawn(async move {
                if let Err(e) = svc.write(id, bytes).await {
                    tracing::warn!("写入终端失败: {e}");
                }
            });
        }
        TabBackend::Custom(out) => {
            let _ = out.send(TabOut::Data(bytes));
        }
    }
}

/// 单个 tab 的尺寸同步:本地改模型 + 远端 resize。
pub fn resize_tab(
    tab: &mut SessionTab,
    svc: &Arc<dyn SessionService>,
    handle: &tokio::runtime::Handle,
    cols: u16,
    rows: u16,
) {
    // `alacritty_terminal` 不支持 0 行/0 列网格(S6b 已记录的既有缺陷);
    // 产品调用方本就用 `cols > 0 && rows > 0` 门禁,这里再兜一层避免 panic。
    if cols > 0 && rows > 0 {
        tab.model.resize(cols, rows);
    }
    if !tab.alive {
        return;
    }
    match &tab.backend {
        TabBackend::Daemon => {
            let svc = Arc::clone(svc);
            let id = tab.info.id.clone();
            handle.spawn(async move {
                if let Err(e) = svc.resize(id, cols, rows).await {
                    tracing::warn!("同步终端尺寸到 daemon 失败: {e}");
                }
            });
        }
        TabBackend::Custom(out) => {
            let _ = out.send(TabOut::Resize { cols, rows });
        }
    }
}

/// 关闭 tab 时是否应该走"总结后关闭"而不是直接 `Kill`——仅对话摄取管线
/// 已覆盖、且当前存活、且走 daemon 后端的 agent(spec 2026-08-27)。
pub fn should_summarize_on_close(agent: AgentKind, alive: bool, backend: &TabBackend) -> bool {
    alive
        && matches!(backend, TabBackend::Daemon)
        && matches!(
            agent,
            AgentKind::Claude
                | AgentKind::Codebuddy
                | AgentKind::Opencode
                | AgentKind::V8agent
                | AgentKind::Goose
                | AgentKind::Aider
                | AgentKind::Codex
        )
}

/// tab 标题:已识别出 agent(hook 上报)则显 agent 名;否则回落到 OSC 7 的
/// cwd basename,再无 cwd 才回落会话名。
pub fn tab_title(agent: AgentKind, cwd: Option<&Path>, fallback: &str) -> String {
    if agent != AgentKind::Unknown {
        return agent.label().to_string();
    }
    match cwd {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned()),
        None => fallback.to_string(),
    }
}

/// 文本显示宽度的基础单元和:CJK 字符按全宽计 2,其余按半宽计 1。
pub fn text_width_units(s: &str) -> f32 {
    s.chars()
        .map(|c| if (c as u32) > 0x2E80 { 2.0 } else { 1.0 })
        .sum()
}

/// 终端 tab 估算显示宽(逻辑像素):状态点+名称+关闭×+pill padding 的粗估。
/// 估偏几像素只会让翻页边界差一个 tab。
pub fn tab_display_width(title: &str) -> f32 {
    // 状态点●+spacing ≈ 18, 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    18.0 + text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// `[会话已结束]` 尾行标记:CREAM 字 / CARD 底。颜色值镜像
/// `byteui::theme::color::current().cream` / `card` —— `TerminalModel` 只吃
/// ANSI truecolor 转义序列,认不出 `iced::Color`。
pub fn exited_marker() -> Vec<u8> {
    const FG: (u8, u8, u8) = (0xFF, 0xE5, 0xB4); // CREAM
    const BG: (u8, u8, u8) = (0x12, 0x20, 0x2a); // CARD
    format!(
        "\r\n\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m[会话已结束]\x1b[0m\r\n",
        FG.0, FG.1, FG.2, BG.0, BG.1, BG.2
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn tab_title_prefers_cwd_basename() {
        assert_eq!(
            tab_title(
                AgentKind::Unknown,
                Some(Path::new("/Users/c/proj")),
                "shell"
            ),
            "proj"
        );
        assert_eq!(
            tab_title(AgentKind::Unknown, Some(Path::new("/")), "shell"),
            "/"
        );
        assert_eq!(tab_title(AgentKind::Unknown, None, "shell"), "shell");
    }

    #[test]
    fn tab_title_prefers_agent_name_once_known() {
        assert_eq!(
            tab_title(AgentKind::Claude, Some(Path::new("/Users/c/proj")), "shell"),
            "claude"
        );
        assert_eq!(tab_title(AgentKind::Claude, None, "shell"), "claude");
    }

    /// S6c-1 新增:根目录、非 UTF-8 cwd 不会产生空标题。空 `fallback`
    /// 原样返回是**现状行为**(生产调用点传的会话名恒非空),按"行为不变"
    /// 约束保留,这里一并钉住。
    #[test]
    fn tab_title_is_never_empty() {
        assert_eq!(tab_title(AgentKind::Unknown, None, ""), "");
        assert!(!tab_title(AgentKind::Unknown, Some(Path::new("/")), "").is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xffbad"));
            assert!(!tab_title(AgentKind::Unknown, Some(bad), "").is_empty());
        }
    }

    #[test]
    fn tab_display_width_cjk_wider_than_ascii() {
        assert!(tab_display_width("中文会话") > tab_display_width("sh"));
    }

    #[test]
    fn should_summarize_on_close_true_for_four_supported_agents() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Opencode,
            AgentKind::V8agent,
        ] {
            assert!(
                should_summarize_on_close(agent, true, &TabBackend::Daemon),
                "{agent:?} 应该走总结后关闭"
            );
        }
    }

    #[test]
    fn should_summarize_on_close_false_for_unsupported_agents_or_dead_or_ssh() {
        assert!(should_summarize_on_close(
            AgentKind::Codex,
            true,
            &TabBackend::Daemon
        ));
        assert!(!should_summarize_on_close(
            AgentKind::Unknown,
            true,
            &TabBackend::Daemon
        ));
        // qodercn 没有 headless 适配器(L1):关闭时不能走"总结后关闭"。
        assert!(!should_summarize_on_close(
            AgentKind::Qodercn,
            true,
            &TabBackend::Daemon
        ));
        assert!(!should_summarize_on_close(
            AgentKind::Claude,
            false,
            &TabBackend::Daemon
        ));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!should_summarize_on_close(
            AgentKind::Claude,
            true,
            &TabBackend::Custom(tx)
        ));
    }

    /// S6c-1 新增:每个 AgentKind × alive × 后端全组合,`Custom` 与 `!alive`
    /// 恒为 false;`Daemon && alive` 的结果与现状一致(支持的七家)。
    #[test]
    fn should_summarize_on_close_is_false_for_custom_backends_and_dead_tabs_for_every_agent_kind() {
        for agent in AgentKind::ALL {
            for alive in [true, false] {
                let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<TabOut>();
                assert!(
                    !should_summarize_on_close(agent, alive, &TabBackend::Custom(tx)),
                    "{agent:?} alive={alive} Custom 不应总结"
                );
                if !alive {
                    assert!(
                        !should_summarize_on_close(agent, false, &TabBackend::Daemon),
                        "{agent:?} 已退出不应总结"
                    );
                }
            }
        }
    }
}
