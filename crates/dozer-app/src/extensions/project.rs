//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数。阶段 1 扩展化
//! 重构项目,设计见 `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`。

pub mod links;

use crate::delivery::WorktreeInfo;
use crate::workspace::AddrEvent;
use crate::{icons, theme};
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, text};

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    /// git remote 的 fetch URL(`delivery::remote_url`)。无 remote/非 git → None。
    remote_url: Option<String>,
    /// 磁盘占用字节数(排除构建产物)。None=尚未算出来。
    disk_usage_bytes: Option<u64>,
    /// 项目描述(`.dozer/description.md` 内容)。None=尚未写入。
    description: Option<String>,
    /// 项目名称行内编辑态(None=未在编辑)。
    name_editing: Option<String>,
    /// 文档/Agent 记忆虚拟链接。
    links: links::LinksState,
    error: Option<String>,
}

impl WorkspaceState {
    /// 打开一个新项目时构造。`description`/`links` 由调用方在构造之前分别
    /// 调 `project_meta::load_description`/`links::load_or_discover` 拿到
    /// (同现有 `files::WorkspaceState::new(FileTree::new(..))` 那种"调用方
    /// 先算好再传入"的既有模式)。
    pub fn new(description: Option<String>, links: links::LinksState) -> Self {
        Self {
            description,
            links,
            ..Self::default()
        }
    }

    /// 供内核 `worktree_strip`(Git Log 视图外层装饰,不属于
    /// `extensions::git_log`)读取——`worktrees` 数据来自组合 git 刷新,但
    /// 消费方是 Git Log 视图,归属判断见设计文档"关键语义确认"。
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }

    /// 供内核 main.rs 键盘路由判断"项目名称是否在自绘编辑态"。
    pub fn name_editing_is_some(&self) -> bool {
        self.name_editing.is_some()
    }

    /// 供内核 `Workspace::blur_inputs` 调用——点击输入框外时取消名称编辑
    /// (不保存半输入)。
    pub fn cancel_name_edit(&mut self) {
        self.name_editing = None;
    }
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`worktrees`/
/// `remote_url`)、验收次数、daemon 改名结果。`GitRefreshed`/
/// `AcceptanceCountLoaded`/`NameRenamed` 由内核分发,带 `project_id`,走
/// `with_project`;其余是用户交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>, Option<String>),
    AcceptanceCountLoaded(i64, Option<u64>),
    /// 磁盘占用统计结果(排除构建产物后的字节数)。
    DiskUsageLoaded(i64, u64),
    /// daemon 改名结果。带 `project_id`,走 `with_project` 路由。
    NameRenamed(i64, Result<dozer_core::protocol::ProjectInfo, String>),
    NameEditStart,
    NameEditEvent(AddrEvent),
}

/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files/Acceptance);改名需要 daemon 往返,走
/// `handle`/`emit`。
#[allow(clippy::too_many_arguments)]
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    current_name: &str,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, worktrees, remote_url) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.worktrees = worktrees;
            ws_state.remote_url = remote_url;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
        Message::DiskUsageLoaded(_, bytes) => {
            ws_state.disk_usage_bytes = Some(bytes);
        }
        Message::NameEditStart => {
            ws_state.name_editing = Some(current_name.to_string());
        }
        Message::NameEditEvent(ev) => match ev {
            AddrEvent::Text(s) => {
                if let Some(buf) = &mut ws_state.name_editing {
                    buf.push_str(&s);
                }
            }
            AddrEvent::Backspace => {
                if let Some(buf) = &mut ws_state.name_editing {
                    buf.pop();
                }
            }
            AddrEvent::Cancel => ws_state.name_editing = None,
            AddrEvent::Submit => {
                let Some(raw) = ws_state.name_editing.clone() else {
                    return;
                };
                let name = raw.trim().to_string();
                if name.is_empty() || name == current_name {
                    // 空名或未改动:直接关闭编辑框,不发请求。
                    ws_state.name_editing = None;
                    return;
                }
                let client = client.clone();
                handle.spawn(async move {
                    let result = client
                        .rename_project(project_id, &name)
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
                    emit(Message::NameRenamed(project_id, result));
                });
            }
        },
        Message::NameRenamed(_, result) => match result {
            Ok(_) => {
                ws_state.name_editing = None;
                ws_state.error = None;
            }
            Err(e) => {
                ws_state.error = Some(format!("改名失败: {e}"));
                // 保留编辑态原始输入,允许重试。
            }
        },
    }
}

/// 项目卡分支标签:`分支` / `分支*`(脏)/ `—`(非 git)。
fn project_branch_label(branch: Option<&str>, dirty: bool) -> String {
    match branch {
        Some(b) if dirty => format!("{b}*"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    }
}

/// 磁盘占用统计的排除名单——跟 `crates/dozer-app/src/project.rs::HIDDEN`
/// (文件树"要不要显示这一行")语义不同,这里是"算不算项目真实内容",不复用
/// 那份常量。
pub const DISK_USAGE_EXCLUDE: [&str; 7] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "__pycache__",
];

/// 递归求和 `root` 下所有文件大小,跳过名字命中 `exclude` 的目录(整个子树
/// 跳过,不下钻)。读不到的条目(权限/符号链接死链)跳过不计入,不中断整体
/// 计算。
pub fn dir_size_excluding(root: &std::path::Path, exclude: &[&str]) -> u64 {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if exclude.contains(&name.as_str()) {
                continue;
            }
            total += dir_size_excluding(&entry.path(), exclude);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// 面板主入口(单栏,不与任何其它面板配对——同 GitLog/Usage)。`project` 为
/// `None` 时内核不会真正走到这里(`left_panel_area` 对 `LeftView::Project`
/// 无条件调用本函数,但 `App::view()` 顶层只在有聚焦项目时才会渲染到这个
/// 分支),这里仍保留一次防御性判断,风格对齐 Files 试点。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project: Option<&'a ProjectInfo>,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(p) = project else {
        return container(iced_widget::Space::new())
            .width(width)
            .height(Length::Fill)
            .into();
    };

    let mut content = column![].spacing(12).padding(14);

    content = content.push(crate::homespace::home_panel_head(
        icons::IconKind::Briefcase,
        "项目",
    ));

    let name_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(buf) = &ws_state.name_editing {
            container(
                text(format!("{buf}▏"))
                    .size(theme::font::title())
                    .color(theme::color::CREAM),
            )
            .padding([2, 4])
            .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::CARD.into()),
                border: Border {
                    color: theme::color::CREAM,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..iced_widget::container::Style::default()
            })
            .into()
        } else {
            button(
                text(p.name.clone())
                    .size(theme::font::title())
                    .color(theme::color::CREAM),
            )
            .on_press(Message::NameEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: theme::color::CREAM,
                ..iced_widget::button::Style::default()
            })
            .into()
        };
    content = content.push(name_row);

    let label = project_branch_label(ws_state.branch.as_deref(), ws_state.dirty);
    let bcolor = if ws_state.dirty {
        theme::color::GOLD
    } else {
        theme::color::BODY
    };
    content = content.push(
        row![
            icons::view(
                icons::IconKind::GitBranch,
                crate::theme::icon_size::row(),
                bcolor
            ),
            text(label).size(theme::font::label()).color(bcolor),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        content = content.push(
            text(format!("{n} 次验收"))
                .size(theme::font::caption())
                .color(theme::color::GOLD),
        );
    }

    let usage_label = ws_state
        .disk_usage_bytes
        .map(|b| format!("文件存储 ({} MB)", b / 1_000_000))
        .unwrap_or_else(|| "文件存储".to_string());
    content = content.push(
        text(usage_label)
            .size(theme::font::label())
            .color(theme::color::DIM),
    );

    content = content.push(
        text("根目录")
            .size(theme::font::label())
            .color(theme::color::DIM),
    );
    content = content.push(
        text(p.path.clone())
            .size(theme::font::caption())
            .color(theme::color::BODY),
    );
    if let Some(url) = &ws_state.remote_url {
        content = content.push(
            text("Git 仓库")
                .size(theme::font::label())
                .color(theme::color::DIM),
        );
        content = content.push(
            text(url.clone())
                .size(theme::font::caption())
                .color(theme::color::BODY),
        );
    }

    if let Some(err) = &ws_state.error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(theme::font::label())
                .color(theme::color::RED),
        );
    }

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::PANEL.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ws() -> WorkspaceState {
        WorkspaceState::default()
    }

    fn test_client() -> dozer_client::Client {
        dozer_client::Client::new(std::path::PathBuf::from("/tmp/dozer-project-test.sock"))
    }

    #[test]
    fn git_refreshed_updates_four_fields() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(
                1,
                Some("main".to_string()),
                true,
                vec![],
                Some("https://x.git".into()),
            ),
            1,
            "名字",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.branch.as_deref(), Some("main"));
        assert!(ws.dirty);
        assert_eq!(ws.worktrees().len(), 0);
        assert_eq!(ws.remote_url.as_deref(), Some("https://x.git"));
    }

    #[test]
    fn acceptance_count_loaded_sets_field() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::AcceptanceCountLoaded(1, Some(3)),
            1,
            "名字",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.project_acceptance_count, Some(3));
    }

    #[test]
    fn project_card_branch_label() {
        assert_eq!(project_branch_label(Some("main"), false), "main");
        assert_eq!(project_branch_label(Some("main"), true), "main*");
        assert_eq!(project_branch_label(None, false), "—");
    }

    #[test]
    fn name_edit_start_prefills_current_name() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::NameEditStart,
            1,
            "旧名字",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.name_editing.as_deref(), Some("旧名字"));
    }

    #[test]
    fn name_edit_submit_same_name_closes_without_request() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::NameEditStart,
            1,
            "同名",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        update(
            &mut ws,
            Message::NameEditEvent(AddrEvent::Submit),
            1,
            "同名",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.name_editing.is_none());
    }

    #[test]
    fn name_renamed_ok_clears_editing_state() {
        let mut ws = new_ws();
        ws.name_editing = Some("新名字".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let project = dozer_core::protocol::ProjectInfo {
            id: 1,
            path: "/repo".into(),
            name: "新名字".into(),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        };
        update(
            &mut ws,
            Message::NameRenamed(1, Ok(project)),
            1,
            "旧名字",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.name_editing.is_none());
        assert!(ws.error.is_none());
    }

    #[test]
    fn name_renamed_err_keeps_editing_state_and_sets_error() {
        let mut ws = new_ws();
        ws.name_editing = Some("新名字".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::NameRenamed(1, Err("连接失败".into())),
            1,
            "旧名字",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.name_editing.as_deref(), Some("新名字"));
        assert!(ws.error.is_some());
    }

    #[test]
    fn dir_size_excluding_sums_files_and_skips_excluded_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "12345").unwrap(); // 5 bytes
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("big.bin"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("b.txt"), "12").unwrap(); // 2 bytes
        let size = dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE);
        assert_eq!(size, 7); // 5 + 2, target 整个跳过
    }

    #[test]
    fn dir_size_excluding_empty_dir_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE), 0);
    }

    #[test]
    fn disk_usage_loaded_sets_field() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::DiskUsageLoaded(1, 12345),
            1,
            "名字",
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.disk_usage_bytes, Some(12345));
    }
}
