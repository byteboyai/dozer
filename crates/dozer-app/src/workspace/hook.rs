//! Workspace 的 hook 安装与杂项辅助:`should_summarize_on_close`/
//! `ensure_hook_installed`/`ensure_mcp_installed`/`preview_context_from_editor_state`,
//! 以及 `tab_title`/`dot_color`/`agent_icon` 等纯函数与 `fetch_project_restore`/
//! `forward_events` 异步桥。

use crate::app::{Message, ProjectId};
use byteui::interaction::icons::IconKind;
use dozer_client::{Client, TermEvent};
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo};
use iced_widget::core::Color;
use iced_winit::winit::event_loop::EventLoopProxy;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use super::*;

/// 关闭 tab 时是否应该走"总结后关闭"而不是直接 `Kill`——仅对话摄取管线
/// 已覆盖、且当前存活、且走 daemon 后端的四家 agent(spec
/// 2026-08-27)。
pub(crate) fn should_summarize_on_close(
    agent: AgentKind,
    alive: bool,
    backend: &TabBackend,
) -> bool {
    alive
        && matches!(backend, TabBackend::Daemon)
        && matches!(
            agent,
            AgentKind::Claude | AgentKind::Codebuddy | AgentKind::Opencode | AgentKind::V8agent
        )
}

/// `on_tab_attached` 里决定要不要回应 OSC 10/11 终端探测查询——`info.agent`
/// 在新建会话这一刻几乎总还是 daemon 侧的默认值,真实 agent 要靠 hook 事后
/// 上报,而那必然晚于 agent CLI 进程启动瞬间就发出的查询;picker 选中的
/// `picked_agent` 才是这一刻唯一"确定即将变成谁"的信号,`info.agent` 仅在
/// picker 未给出提示(如 SSH attach、或重连时 hook 已先一步上报过)时兜底。
pub(crate) fn should_answer_dynamic_color(
    picked_agent: Option<AgentKind>,
    info_agent: AgentKind,
) -> bool {
    picked_agent == Some(AgentKind::Opencode) || info_agent == AgentKind::Opencode
}

/// 已接入 `dozer-hook` 安装器的 agent 集合。刻意穷尽 match 而不是拿
/// `agent.label()` 当 catch-all 参数：`install::settings_path_for` 对未识别
/// 的 agent 名一律落回 Claude 的 `settings.json`路径，如果不显式排除
/// Kilo/V8agent，误调用会把 "kilo"/"v8agent" 的 hook 命令写进 Claude 的
/// settings.json，顶掉真正的 claude hook 条目。两者排除的原因不同：Kilo
/// 是真实缺口(没有任何 hook 上报机制)；V8agent 走的是完全不同的路子——
/// `v8agent-cli` 在 `DOZER_SESSION_ID` 存在时直接通过 UDS socket 上报
/// `Request::HookEvent`(见 `dozer-core::protocol`),不依赖这套"往
/// agent 自己的配置文件里写 hook 命令"的安装机制，所以这里返回 `None`
/// 对 V8agent 而言是正确行为，不是待办事项(spec
/// `docs/superpowers/specs/2026-08-24-v8agent-integration-design.md`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookInstallTarget {
    Settings,
    Opencode,
}

pub(crate) fn hook_install_target(agent: AgentKind) -> Option<HookInstallTarget> {
    match agent {
        AgentKind::Claude | AgentKind::Codebuddy | AgentKind::Codex => {
            Some(HookInstallTarget::Settings)
        }
        AgentKind::Opencode => Some(HookInstallTarget::Opencode),
        AgentKind::Unknown | AgentKind::Kilo | AgentKind::V8agent => None,
    }
}

/// `exe` 所在目录下名为 `dozer-hook` 的同级二进制路径（跟
/// `main.rs::spawn_dozerd` 定位同级 `dozerd` 的手法一致——发行版把
/// `dozer-hook` 跟 `dozer`/`dozerd` 一起装进同一个 `Contents/MacOS/`）。
/// 抽成纯函数只是为了能不依赖 `current_exe()` 直接单测。
pub(crate) fn dozer_hook_binary_path(exe: &Path) -> PathBuf {
    match exe.parent() {
        Some(dir) => dir.join("dozer-hook"),
        None => PathBuf::from("dozer-hook"),
    }
}

/// 新开 agent 会话前静默注册该 agent 的 hook（幂等、恒静默——同
/// `dozer-hook install` 自身"绝不因失败拖慢/打断会话"的错误处理哲学）。
/// 在此之前 hook 注册是一步用户必须自己发现并手动执行的 CLI 命令
/// （`dozer-hook install <agent>`），Claude 之外的 agent 几乎没人知道要
/// 跑它，于是 Agent 面板里 name/status 永远停在 `Unknown`/`Idle`。阻塞
/// 文件 I/O，调用方须包一层 `spawn_blocking`。
///
/// 必须用 `run_at_with_exe`/`opencode_install::run_at_with_exe` 显式传入
/// exe 路径，不能调不带 `_with_exe` 的版本：那两个版本内部读
/// `std::env::current_exe()`，在这里（`dozer-app` 进程内直接函数调用，
/// 不是 spawn 一个独立的 `dozer-hook` 子进程）会拿到 `dozer-app` 自己的
/// 可执行文件路径，写出一条指向错误二进制的 hook 命令——2026-08 线上
/// 事故：`~/.claude/settings.json` 堆出重复的坏 hook，agent 名字/光标
/// 状态全靠 `dozer-hook` 转发的事件才能更新，全断了。
pub(crate) fn ensure_hook_installed(agent: AgentKind) {
    let Some(target) = hook_install_target(agent) else {
        return;
    };
    let exe =
        dozer_hook_binary_path(&std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")));
    let exe = exe.to_string_lossy();
    match target {
        HookInstallTarget::Settings => {
            let label = agent.label();
            let _ = dozer_hook::install::run_at_with_exe(
                &dozer_hook::install::settings_path_for(label),
                label,
                true,
                &exe,
            );
        }
        HookInstallTarget::Opencode => {
            let _ = dozer_hook::opencode_install::run_at_with_exe(
                &dozer_hook::opencode_install::plugins_dir(),
                true,
                &exe,
            );
        }
    }
}

/// `dozer-mcp` 自动注册:仿 `ensure_hook_installed` 同一套幂等/静默/失败
/// 只 warn 的哲学。V8agent 走完全不同的路(见 `hook_install_target` 文档
/// 注释同款理由)——它自己硬编码检测 `DOZER_SESSION_ID` 后自动挂载
/// `dozer-mcp serve`,不读任何配置文件,这里对它直接 no-op。
pub(crate) fn ensure_mcp_installed(agent: AgentKind) {
    let Some((path, _)) = dozer_mcp::install::config_path_for(agent.label()) else {
        return;
    };
    let exe =
        dozer_hook_binary_path(&std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")))
            .parent()
            .map(|dir| dir.join("dozer-mcp"))
            .unwrap_or_else(|| PathBuf::from("dozer-mcp"));
    let exe = exe.to_string_lossy();
    let _ = dozer_mcp::install::run_at_with_exe(&path, agent.label(), true, &exe);
}

/// picker 选择项 → attach 成功后自动键入 PTY 的初始命令。`Agent(Some(a))`
/// 复用 `agent_cli_command`(键入 agent CLI);`Agent(None)` 不键入(纯 Shell);
/// `Git` 键入 `git status`——新开的 shell 已在项目根,直接看仓库状态。
/// 抽成纯函数是为了能 headless 单测(同 `agent_cli_command` 的惯例)。
pub(crate) fn picker_launch_command(launch: PickerLaunch) -> Option<String> {
    match launch {
        PickerLaunch::Agent(Some(agent)) => agent_cli_command(agent).map(str::to_owned),
        PickerLaunch::Agent(None) => None,
        PickerLaunch::Git => Some("git status".to_string()),
    }
}

/// tab 标题：已识别出 agent（hook 上报）则显 agent 名（如 "claude"）；
/// 否则回落到 OSC 7 的 cwd basename，再无 cwd 才回落会话名。
pub(crate) fn tab_title(agent: AgentKind, cwd: Option<&Path>, fallback: &str) -> String {
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

/// 文本显示宽度的基础单元和：CJK 字符按全宽计 2，其余按半宽计 1。
/// `tab_display_width`/`preview_tab_display_width` 共用。
pub(crate) fn text_width_units(s: &str) -> f32 {
    s.chars()
        .map(|c| if (c as u32) > 0x2E80 { 2.0 } else { 1.0 })
        .sum()
}

/// 终端 tab 估算显示宽（逻辑像素）：状态点+名称+关闭×+pill padding 的粗估。
/// 不追求精确——估偏几像素只会让翻页边界差一个 tab。宽度随标题实际长度
/// 增长(无上限),渲染侧亦有对应 `panel_tab` 的按内容伸缩。
pub(crate) fn tab_display_width(title: &str) -> f32 {
    // 状态点●+spacing ≈ 18, 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    18.0 + text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// 预览 tab 估算显示宽：同 `tab_display_width` 但无状态点。同按标题实际长度
/// 估算,不设上限(与渲染侧 `panel_tab` 按内容伸缩对齐,翻页窗口数学按真实
/// 宽度算,标题多宽估多宽)。
pub(crate) fn preview_tab_display_width(title: &str) -> f32 {
    // 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// agent 四态中文（终端状态栏用）。
pub(crate) fn agent_state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Running => "运行中",
        AgentState::AwaitingInput => "待输入",
        AgentState::TurnEnded => "回合毕",
        AgentState::Idle => "空闲",
    }
}

/// tab 前状态点配色：死会话灰；存活按 agent 状态——绿=空闲/运行、
/// 紫蓝=待输入、金=回合结束（金是甲方动作专属色：该出手了）。运行态
/// 与空闲态同为绿，靠 `tab_item` 里的闪烁区分（工作中才闪）。
pub(crate) fn dot_color(state: AgentState, alive: bool) -> Color {
    if !alive {
        return byteui::theme::color::current().dim;
    }
    match state {
        AgentState::Idle => byteui::theme::color::current().cyan,
        AgentState::Running => byteui::theme::color::current().green,
        AgentState::AwaitingInput => byteui::theme::color::current().red,
        AgentState::TurnEnded => byteui::theme::color::current().gold,
    }
}

/// `claude-sonnet-5` → `Sonnet 5`:去掉 `claude-` 前缀,按 `-` 分词、每
/// 词首字母大写、空格拼接。不以 `claude-` 开头的原样返回(不确定形状,
/// 不强行摘,避免拍出乱码;不维护会过期的型号对照表)。
pub(crate) fn format_model_label(raw: &str) -> String {
    match raw.strip_prefix("claude-") {
        Some(rest) => rest
            .split('-')
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => raw.to_string(),
    }
}

/// agent → 对话列表圆点颜色。避开 `byteui::theme::color::current().gold`(甲方动作专属色,
/// CLAUDE.md 明文规定,不能被 agent 分类语义借用)。
pub(crate) fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => byteui::theme::color::current().cyan,
        AgentKind::Codebuddy => byteui::theme::color::current().purple,
        AgentKind::Opencode => byteui::theme::color::current().green,
        AgentKind::Codex => byteui::theme::color::current().orange,
        AgentKind::Kilo => byteui::theme::color::current().blue,
        AgentKind::V8agent => byteui::theme::color::current().lime,
        AgentKind::Unknown => byteui::theme::color::current().dim,
    }
}

/// agent → 品牌图标(新建 agent 菜单用)。颜色由调用方按 `agent_dot_color`
/// 同款语义传入,使图标色与圆点色一致,避免引入新配色维度。
pub(crate) fn agent_icon(agent: AgentKind) -> IconKind {
    match agent {
        AgentKind::Claude => IconKind::Claude,
        AgentKind::Codebuddy => IconKind::Codebuddy,
        AgentKind::Opencode => IconKind::Opencode,
        // 暂无确认可用的品牌素材，回落通用图标（spec §8/§6 明确允许）。
        AgentKind::Codex | AgentKind::Kilo | AgentKind::V8agent | AgentKind::Unknown => {
            IconKind::Bot
        }
    }
}

/// 由(路径, 是否有选区, 0-indexed 光标位置, 0-indexed 选区范围, 当前时间)
/// 组装 1-indexed 的 `PreviewContext`。抽成纯函数是为了不依赖真实
/// `CodeEditor`/`PreviewPane` 就能单测坐标转换这一层逻辑——`now_ms` 由调
/// 用方传进来而不是在这里读 `SystemTime::now()`,正是为了保住这份纯度。
pub(crate) fn preview_context_from_editor_state(
    path: &str,
    has_selection: bool,
    cursor: (usize, usize),
    selection: Option<((usize, usize), (usize, usize))>,
    now_ms: u64,
) -> dozer_core::protocol::PreviewContext {
    let (start, end) = if has_selection {
        selection.unwrap_or((cursor, cursor))
    } else {
        (cursor, cursor)
    };
    dozer_core::protocol::PreviewContext {
        path: path.to_string(),
        start_line: start.0 as u32 + 1,
        start_col: start.1 as u32 + 1,
        end_line: end.0 as u32 + 1,
        end_col: end.1 as u32 + 1,
        has_selection,
        updated_at_ms: now_ms,
    }
}

/// 促成的 **IO 段**:把一个项目在 daemon 上的存活会话逐一 attach 下来,连同
/// 最近项目列表打包成 [`ProjectRestore`]。整段只碰 `Client`,不碰任何 GUI
/// 类型,所以可以在 tokio 线程池上跑(`App::ensure_loaded` 正是这么用的)。
pub(crate) async fn fetch_project_restore(client: &Client, project: ProjectInfo) -> ProjectRestore {
    let mut sessions = Vec::new();
    match client.list().await {
        Ok(list) => {
            // 只认归属本项目的会话:daemon 现在按 `project_id` 给会话分家
            // (P2a Task 1-3),并行打开的别的项目的终端不该跑到这一份
            // `Workspace` 的 tab 栏里来。
            //
            // 迁移期孤儿会话——Task 1-3 落地**之前**建的、`project_id`
            // 为 `None` 的存活会话——会被这条 filter 一并排除,从此不出现
            // 在任何项目的 tab 栏里,直到 dozerd 重启把它们清掉为止(在此
            // 期间它们仍占着 PTY)。这是**有意为之**,不是漏判:规格把
            // "迁移期孤儿会话怎么处理"显式挂起、留给实现计划阶段决定,
            // 本期不做孤儿会话的找回入口。日后有人发现"重启 daemon 前
            // 有几个会话凭空消失了",答案就在这一行。
            for info in list
                .into_iter()
                .filter(|s| s.alive && s.project_id == Some(project.id))
            {
                match client.attach(&info.id, 0).await {
                    Ok((snapshot, _next_offset, rx)) => sessions.push((info, snapshot, rx)),
                    Err(e) => {
                        tracing::warn!(session = %info.id, "attach 失败，跳过该会话恢复: {e}")
                    }
                }
            }
        }
        Err(e) => tracing::warn!("list 失败，跳过启动恢复: {e}"),
    }
    // 最近项目列表（git 分支/脏在窗口起来后异步补）。"当前项目"不再
    // 向 daemon 打听——daemon 侧的"活跃项目"概念已随 P2a Task 1-3 删除
    // （多项目并行下没有唯一活跃项目），改由调用方(`App`)指定。
    let recent_projects = client.list_projects().await.unwrap_or_default();
    ProjectRestore {
        project,
        recent_projects,
        sessions,
    }
}

/// 单个 attach 数据流的转发循环：持有 `rx`（唯一持有者），逐条转发到
/// UI 线程（经 `EventLoopProxy`）。tab 关闭时 `JoinHandle::abort` 打断
/// 这个循环，`rx` 随任务栈一起析构——这就是 detach 的物理落点。
///
/// `project_id` 是这条流归属的项目:`tab_id` 只在单个 `Workspace` 内唯一,
/// 每个项目都从 0 起编,所以转发出去的消息必须自带项目归属,否则后台项目的
/// 输出会被喂进前台项目同号的 tab(见 [`ProjectId`])。
pub(crate) async fn forward_events(
    project_id: ProjectId,
    tab_id: usize,
    mut rx: mpsc::UnboundedReceiver<TermEvent>,
    proxy: EventLoopProxy<Message>,
) {
    while let Some(event) = rx.recv().await {
        let message = match event {
            TermEvent::Output(bytes) => Message::TermOutput(project_id, tab_id, bytes),
            TermEvent::Exited(_code) => {
                let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                return;
            }
            TermEvent::Disconnected => {
                let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                return;
            }
            TermEvent::Lagged => {
                tracing::warn!(tab_id, "终端事件滞后（lagged），可能丢失部分历史输出");
                continue;
            }
            TermEvent::Agent {
                agent,
                state,
                transcript_path,
            } => Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path),
        };
        if proxy.send_event(message).is_err() {
            // UI 线程（EventLoop）已经关闭，没有必要继续转发。
            return;
        }
    }
}
