//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数。阶段 1 扩展化
//! 重构项目,设计见 `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`。

pub mod links;

use crate::delivery::WorktreeInfo;
use crate::theme;
use crate::workspace::AddrEvent;
use byteui::interaction::icons;
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, button, column, container, row, text};
use std::path::PathBuf;

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    /// git remote 的 fetch URL 列表(`delivery::remote_url`)。空 = 无 remote/
    /// 非 git(面板据此显示"未设置")。
    remote_url: Vec<String>,
    /// 磁盘占用字节数(排除构建产物)。None=尚未算出来。
    disk_usage_bytes: Option<u64>,
    /// 项目描述(`.dozer/description.md` 内容)。None=尚未写入。
    description: Option<String>,
    /// 项目名称行内编辑态(None=未在编辑)。
    name_editing: Option<String>,
    /// 项目描述编辑态(None=未在编辑)。采用 iced 原生 `text_editor::Content`。
    description_editing: Option<iced_widget::text_editor::Content>,
    /// 文档/Agent 记忆虚拟链接。
    links: links::LinksState,
    /// 已展开状态目录 → 其子项列表(就地展开/收起)。
    expanded_link_dirs: std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
    /// 链接区(项目文档 / Agent 记忆)当前选中项路径。单击文件(`OpenLink`)/
    /// 目录(`LinkDirToggle`)/展开子项、右击行(`LinkContextMenu`)都会选中,
    /// 用来整行高亮——参考文件树 `files::WorkspaceState::tree_selected`。
    /// `None`=无选中(新开项目默认)。
    selected_link: Option<PathBuf>,
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

    /// 项目级分支名(Project 面板/Agent 卡片工作区行共用读口)。
    pub(crate) fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// 项目级脏标(同上)。
    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    /// 供内核 main.rs 键盘路由判断"项目名称是否在自绘编辑态"。
    pub fn name_editing_is_some(&self) -> bool {
        self.name_editing.is_some()
    }

    /// 供内核 `App::blur_inputs` 调用——失焦时取出当前编辑中的名称缓冲。
    /// 返回 `Some(raw)` 时由内核发起 daemon 改名(改动且非空才真正发请求,
    /// 见 `App::blur_inputs`);`None` 表示未处于编辑态,无需处理。
    pub fn take_name_edit(&mut self) -> Option<String> {
        self.name_editing.take()
    }

    /// 供内核 `project_preview_open_path`/`project_link_context_menu` 调用——
    /// 打开链接预览 / 打开删除右键菜单的同时把该路径标记为「选中」行(参考
    /// 文件树 `files::WorkspaceState::set_tree_selected`)。
    pub fn set_selected_link(&mut self, path: PathBuf) {
        self.selected_link = Some(path);
    }

    /// 供内核 `project_link_context_menu` 解析右击行的路径(按区 + 下标),
    /// 用于把该行标记为「选中」。返回 `None` 表示下标越界(菜单本就不该弹)。
    pub fn link_path_at(&self, target: links::LinkTarget, index: usize) -> Option<PathBuf> {
        self.links.list(target).get(index).map(|e| e.path.clone())
    }

    /// 供内核 `Workspace::blur_inputs` 调用——失焦时把当前编辑态直接写盘
    /// (描述保存不需要网络往返,不用等 `Message` 走一圈)。
    pub fn submit_description_edit_on_blur(&mut self, repo_path: &std::path::Path) {
        let Some(content) = self.description_editing.take() else {
            return;
        };
        let text = content.text().trim().to_string();
        if crate::project_meta::write_description(repo_path, &text).is_ok() {
            self.description = if text.is_empty() { None } else { Some(text) };
        }
        // 写失败这里不重试(失焦场景不适合弹错误态阻塞用户),下次进入面板
        // 仍能看到 `description` 字段的旧值,不会丢用户输入太久——这是已知
        // 的简化,写盘失败几率很低(权限问题会在其它写操作里更早暴露)。
    }
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`worktrees`/
/// `remote_url`)、验收次数、daemon 改名结果。`GitRefreshed`/
/// `AcceptanceCountLoaded`/`NameRenamed` 由内核分发,带 `project_id`,走
/// `with_project`;其余是用户交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>, Vec<String>),
    AcceptanceCountLoaded(i64, Option<u64>),
    /// 磁盘占用统计结果(排除构建产物后的字节数)。
    DiskUsageLoaded(i64, u64),
    /// daemon 改名结果。带 `project_id`,走 `with_project` 路由。
    NameRenamed(i64, Result<dozer_core::protocol::ProjectInfo, String>),
    NameEditStart,
    NameEditEvent(AddrEvent),
    DescriptionEditStart,
    DescriptionEditAction(iced_widget::text_editor::Action),
    /// 预留:当前描述靠 `submit_description_edit_on_blur` 直接写盘(见该文档
    /// 注释),这个变体/`update` 分支留给以后可能加的显式"保存"按钮,现在还没
    /// 生产代码构造它,故 `#[allow(dead_code)]`。
    #[allow(dead_code)]
    DescriptionEditSubmit,
    LinkAdd {
        target: links::LinkTarget,
        path: PathBuf,
        kind: links::LinkKind,
    },
    LinkRemove {
        target: links::LinkTarget,
        index: usize,
    },
    LinkDirToggle {
        /// 保留在消息签名里(与 `LinkAdd`/`LinkRemove` 对齐);本期展开/收起
        /// 只按 `path` 操作,还没按 `target` 分流,故 `#[allow(dead_code)]`。
        #[allow(dead_code)]
        target: links::LinkTarget,
        path: PathBuf,
    },
    /// 行内右键:由内核拦截,把目标项(区 + 下标)记进 App 级右键菜单浮层态,
    /// 渲染删除菜单,见 `files::Message::ContextMenuOpen` 文档同款写法。
    LinkContextMenu {
        target: links::LinkTarget,
        index: usize,
    },
    /// 内核拦截处理,见 `files::Message::OpenFile` 文档同款写法。
    OpenLink(PathBuf),
    /// 仅选中(不展开、不打开):用于「项目文档 / Agent 记忆」里已展开目录的
    /// 子目录项——它们是只读单层展示,单击只高亮、不触发二次展开(展开已在
    /// 父级目录 `LinkDirToggle` 完成)。进 `update` 直接写 `selected_link`。
    LinkSelect {
        /// 保留在签名里与 `LinkDirToggle` 对齐;本期选中只按 `path`,未分流。
        #[allow(dead_code)]
        target: links::LinkTarget,
        path: PathBuf,
    },
    /// 单颗"＋"按钮:由内核 rfd 弹 OS 文件浏览器(根目录在项目根),选中的
    /// 文件/目录由内核判 `is_dir()` 定 `LinkKind`,再回送 `LinkAdd`。
    Pick(links::LinkTarget),
    /// footer-bar「修复项目」按钮(UI 占位,逻辑后续接入)。
    RepairProject,
    /// footer-bar「删除项目」按钮(UI 占位,逻辑后续接入)。
    DeleteProject,
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
    repo_path: &std::path::Path,
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
        Message::DescriptionEditStart => {
            let initial = ws_state.description.clone().unwrap_or_default();
            ws_state.description_editing =
                Some(iced_widget::text_editor::Content::with_text(&initial));
        }
        Message::DescriptionEditAction(action) => {
            if let Some(content) = &mut ws_state.description_editing {
                content.perform(action);
            }
        }
        Message::DescriptionEditSubmit => {
            let Some(content) = ws_state.description_editing.take() else {
                return;
            };
            let text = content.text().trim().to_string();
            match crate::project_meta::write_description(repo_path, &text) {
                Ok(()) => {
                    ws_state.description = if text.is_empty() { None } else { Some(text) };
                    ws_state.error = None;
                }
                Err(e) => {
                    ws_state.error = Some(format!("保存失败: {e}"));
                    ws_state.description_editing = Some(content); // 保留编辑态允许重试
                }
            }
        }
        Message::LinkAdd { target, path, kind } => {
            let already_present = ws_state.links.list(target).iter().any(|e| e.path == path);
            if !already_present {
                ws_state
                    .links
                    .list_mut(target)
                    .push(links::LinkEntry { path, kind });
                match links::save(repo_path, &ws_state.links) {
                    Ok(()) => ws_state.error = None,
                    Err(e) => {
                        ws_state.links.list_mut(target).pop();
                        ws_state.error = Some(format!("保存失败: {e}"));
                    }
                }
            }
        }
        Message::LinkRemove { target, index } => {
            let list = ws_state.links.list_mut(target);
            if index >= list.len() {
                return;
            }
            let removed = list.remove(index);
            if let Err(e) = links::save(repo_path, &ws_state.links) {
                ws_state.links.list_mut(target).insert(index, removed);
                ws_state.error = Some(format!("保存失败: {e}"));
            } else {
                ws_state.error = None;
            }
        }
        Message::LinkDirToggle { path, .. } => {
            ws_state.selected_link = Some(path.clone());
            if ws_state.expanded_link_dirs.remove(&path).is_none() {
                let rows = links::read_dir_row(&path);
                ws_state.expanded_link_dirs.insert(path, rows);
            }
        }
        Message::LinkSelect { path, .. } => {
            ws_state.selected_link = Some(path);
        }
        Message::OpenLink(_) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
        Message::Pick(_) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
        Message::LinkContextMenu { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::ContextMenuOpen 文档")
        }
        // footer-bar 占位按钮:逻辑后续接入,暂不做任何处理。
        Message::RepairProject => {}
        Message::DeleteProject => {}
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

    let mut content = column![]
        .spacing(12)
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill);

    content = content.push(crate::homespace::home_panel_head(
        icons::IconKind::Briefcase,
        "项目",
    ));

    let name_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(buf) = &ws_state.name_editing {
            container(
                text(format!("{buf}▏"))
                    .size(theme::font::title())
                    .color(byteui::theme::color::current().cream),
            )
            .padding([8, 12])
            .width(Length::Fill)
            .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().card.into()),
                border: Border {
                    color: byteui::theme::color::current().gold,
                    width: 1.5,
                    radius: 8.0.into(),
                },
                ..iced_widget::container::Style::default()
            })
            .into()
        } else {
            button(
                text(p.name.clone())
                    .size(theme::font::title())
                    .color(byteui::theme::color::current().cream),
            )
            .on_press(Message::NameEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: byteui::theme::color::current().cream,
                ..iced_widget::button::Style::default()
            })
            .into()
        };
    content = content.push(name_row);

    let description_block: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(editing) = &ws_state.description_editing {
            iced_widget::text_editor(editing)
                .placeholder("项目描述信息…")
                .on_action(Message::DescriptionEditAction)
                .height(Length::Fixed(96.0))
                .style(|_t, _s| iced_widget::text_editor::Style {
                    background: byteui::theme::color::current().card.into(),
                    border: Border {
                        color: byteui::theme::color::current().gold,
                        width: 1.5,
                        radius: 8.0.into(),
                    },
                    placeholder: byteui::theme::color::current().dim,
                    value: byteui::theme::color::current().cream,
                    selection: byteui::theme::color::current().gold,
                })
                .into()
        } else {
            let label = ws_state
                .description
                .clone()
                .unwrap_or_else(|| "点击添加项目描述…".to_string());
            let color = if ws_state.description.is_some() {
                byteui::theme::color::current().body
            } else {
                byteui::theme::color::current().dim
            };
            button(text(label).size(theme::font::body()).color(color))
                .on_press(Message::DescriptionEditStart)
                .padding([10, 12])
                .width(Length::Fill)
                .style(|_t, _s| iced_widget::button::Style {
                    background: Some(byteui::theme::color::current().desc_bg.into()),
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    text_color: byteui::theme::color::current().body,
                    ..iced_widget::button::Style::default()
                })
                .into()
        };
    content = content.push(description_block);

    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        content = content.push(
            text(format!("{n} 次验收"))
                .size(theme::font::caption())
                .color(byteui::theme::color::current().gold),
        );
    }

    let usage_label = ws_state
        .disk_usage_bytes
        .map(|b| format!("文件存储 ({} MB)", b / 1_000_000))
        .unwrap_or_else(|| "文件存储".to_string());
    content = content.push(
        row![
            icons::view(
                icons::IconKind::CircleSmall,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream
            ),
            text(usage_label)
                .size(theme::font::label())
                .color(byteui::theme::color::current().cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    // 「项目文档」下的文件树项都包在 iced button 里,button 默认左内边距 10px;
    // 为与之左对齐,标签行统一左缩 10px,值文本缩进到与文件树文件名同列。
    let tree_indent = 10.0;
    let value_indent = tree_indent + byteui::theme::icon_size::row() + 6.0;
    content = content.push(
        container(
            row![
                icons::view(
                    icons::IconKind::FolderDot,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text("项目根目录")
                    .size(theme::font::label())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding(iced_widget::core::Padding::new(0.0).left(tree_indent)),
    );
    content = content.push(
        row![
            iced_widget::Space::new().width(Length::Fixed(value_indent)),
            text(shorten_path(&p.path))
                .size(theme::font::caption())
                .color(byteui::theme::color::current().body),
        ]
        .align_y(iced_widget::core::Alignment::Center),
    );
    content = content.push(
        container(
            row![
                icons::view(
                    icons::IconKind::FolderRoot,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text("Git 远程仓库")
                    .size(theme::font::label())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding(iced_widget::core::Padding::new(0.0).left(tree_indent)),
    );
    if ws_state.remote_url.is_empty() {
        content = content.push(
            row![
                iced_widget::Space::new().width(Length::Fixed(value_indent)),
                text("未设置")
                    .size(theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            ]
            .align_y(iced_widget::core::Alignment::Center),
        );
    } else {
        for url in &ws_state.remote_url {
            content = content.push(
                row![
                    iced_widget::Space::new().width(Length::Fixed(value_indent)),
                    text(url.clone())
                        .size(theme::font::caption())
                        .color(byteui::theme::color::current().body),
                ]
                .align_y(iced_widget::core::Alignment::Center),
            );
        }
    }

    content = content.push(links_section(
        "项目文档",
        links::LinkTarget::Docs,
        &ws_state.links,
        &ws_state.expanded_link_dirs,
        &ws_state.selected_link,
    ));
    content = content.push(links_section(
        "Agent 记忆",
        links::LinkTarget::Memory,
        &ws_state.links,
        &ws_state.expanded_link_dirs,
        &ws_state.selected_link,
    ));

    if let Some(err) = &ws_state.error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    let body = column![content, project_footer_bar(),].spacing(0);

    container(body)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().panel.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 项目信息面板底部 footer-bar,结构与文件树面板的 `git_footer_bar` 一致:
/// 1px `BORDER` 分隔线 + `padding([6, 8])` 容器。当前放「修复项目 / 删除项目」
/// 两个并排圆角按钮,行为仅为 UI 占位(`RepairProject` / `DeleteProject`),
/// 实际逻辑后续接入。
fn project_footer_bar() -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let repair = button(
        text("修复项目")
            .size(theme::font::label())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::RepairProject)
    .width(Length::Fill)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().cream,
        ..iced_widget::button::Style::default()
    });

    let delete = button(
        text("删除项目")
            .size(theme::font::label())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::DeleteProject)
    .width(Length::Fill)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: byteui::theme::color::current().red,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().red,
        ..iced_widget::button::Style::default()
    });

    let bar = row![repair, delete]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..iced_widget::container::Style::default()
        });

    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding([6, 8])
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// 把可能很长的绝对路径压缩成一行可读字符串:超过 `MAX` 个字符时保留首尾、
/// 中间用 `…` 代替,避免项目面板被长路径撑破。
fn shorten_path(p: &str) -> String {
    const MAX: usize = 48;
    let chars: Vec<char> = p.chars().collect();
    if chars.len() <= MAX {
        return p.to_string();
    }
    let keep = MAX - 1;
    let head_len = keep / 2;
    let tail_len = keep - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    format!("{head}…{tail}")
}

fn links_section<'a>(
    title: &'static str,
    target: links::LinkTarget,
    links_state: &'a links::LinksState,
    expanded: &'a std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
    selected_link: &'a Option<PathBuf>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(6);
    col = col.push(
        row![
            icons::view(
                icons::IconKind::CircleSmall,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream
            ),
            text(title)
                .size(theme::font::label())
                .color(byteui::theme::color::current().cream),
            iced_widget::space::horizontal(),
            button(
                text("+")
                    .size(theme::font::label())
                    .color(byteui::theme::color::current().dim)
            )
            .on_press(Message::Pick(target))
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: byteui::theme::color::current().dim,
                ..iced_widget::button::Style::default()
            }),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    for (i, entry) in links_state.list(target).iter().enumerate() {
        let name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.path.to_string_lossy().into_owned());
        let row_icon = if entry.kind == links::LinkKind::Dir {
            icons::IconKind::Folder
        } else {
            icons::icon_for_file(&name)
        };
        let click_msg = if entry.kind == links::LinkKind::Dir {
            Message::LinkDirToggle {
                target,
                path: entry.path.clone(),
            }
        } else {
            Message::OpenLink(entry.path.clone())
        };
        // 删除改由右键菜单(`LinkContextMenu`)触发,行内不再挂 × 按钮。
        let is_selected = selected_link.as_deref() == Some(entry.path.as_path());
        let row_btn = button(
            row![
                icons::view(
                    row_icon,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(name)
                    .size(theme::font::body())
                    .color(byteui::theme::color::current().body),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(click_msg)
        .style(move |_t, _s| iced_widget::button::Style {
            background: if is_selected {
                Some(byteui::theme::color::current().card.into())
            } else {
                None
            },
            text_color: byteui::theme::color::current().body,
            ..iced_widget::button::Style::default()
        });
        col = col.push(
            MouseArea::new(row_btn).on_right_press(Message::LinkContextMenu { target, index: i }),
        );
        if entry.kind == links::LinkKind::Dir
            && let Some(rows) = expanded.get(&entry.path)
        {
            for row_entry in rows {
                // 展开子项同样可点选(参考文件树每行都可选中):文件→打开预览,
                // 目录→仅选中(只读单层,不二次展开);单击即高亮。
                let child_path = row_entry.path.clone();
                let child_click = if row_entry.is_dir {
                    Message::LinkSelect {
                        target,
                        path: child_path.clone(),
                    }
                } else {
                    Message::OpenLink(child_path.clone())
                };
                let child_is_selected = selected_link.as_deref() == Some(child_path.as_path());
                let child_btn = button(
                    row![
                        iced_widget::space::Space::new().width(Length::Fixed(20.0)),
                        icons::view(
                            if row_entry.is_dir {
                                icons::IconKind::Folder
                            } else {
                                icons::icon_for_file(&row_entry.name)
                            },
                            byteui::theme::icon_size::row(),
                            byteui::theme::color::current().dim
                        ),
                        text(row_entry.name.clone())
                            .size(theme::font::caption())
                            .color(byteui::theme::color::current().dim),
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::Alignment::Center),
                )
                .on_press(child_click)
                .style(move |_t, _s| iced_widget::button::Style {
                    background: if child_is_selected {
                        Some(byteui::theme::color::current().card.into())
                    } else {
                        None
                    },
                    text_color: byteui::theme::color::current().dim,
                    ..iced_widget::button::Style::default()
                });
                col = col.push(child_btn);
            }
        }
    }
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ws() -> WorkspaceState {
        WorkspaceState::default()
    }

    fn test_repo_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dozer-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
                vec!["https://x.git".to_string()],
            ),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.branch.as_deref(), Some("main"));
        assert!(ws.dirty);
        assert_eq!(ws.worktrees().len(), 0);
        assert_eq!(ws.remote_url.as_slice(), ["https://x.git"]);
    }

    #[test]
    fn branch_and_dirty_accessors_read_current_state() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(1, Some("main".to_string()), true, vec![], vec![]),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.branch(), Some("main"));
        assert!(ws.dirty());
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
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.project_acceptance_count, Some(3));
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
            &test_repo_path(),
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
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        update(
            &mut ws,
            Message::NameEditEvent(AddrEvent::Submit),
            1,
            "同名",
            &test_repo_path(),
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
            &test_repo_path(),
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
            &test_repo_path(),
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
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.disk_usage_bytes, Some(12345));
    }

    #[test]
    fn description_edit_start_prefills_from_field() {
        let mut ws = new_ws();
        ws.description = Some("已有描述".to_string());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::DescriptionEditStart,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let content = ws.description_editing.expect("进入编辑态");
        assert_eq!(content.text(), "已有描述");
    }

    #[test]
    fn description_edit_submit_writes_to_disk_and_clears_editing() {
        let mut ws = new_ws();
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let action = iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Paste(
            std::sync::Arc::new("新描述内容".to_string()),
        ));
        update(
            &mut ws,
            Message::DescriptionEditStart,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        update(
            &mut ws,
            Message::DescriptionEditAction(action),
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        update(
            &mut ws,
            Message::DescriptionEditSubmit,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.description_editing.is_none());
        assert_eq!(ws.description.as_deref(), Some("新描述内容"));
        let on_disk = crate::project_meta::load_description(dir.path());
        assert_eq!(on_disk.as_deref(), Some("新描述内容"));
    }

    #[test]
    fn description_edit_submit_empty_clears_field_and_file() {
        let mut ws = new_ws();
        let dir = tempfile::tempdir().unwrap();
        crate::project_meta::write_description(dir.path(), "旧描述").unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::DescriptionEditStart,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        // 清空编辑内容:直接提交空编辑器。
        update(
            &mut ws,
            Message::DescriptionEditSubmit,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.description_editing.is_none());
        assert!(ws.description.is_none());
        assert!(crate::project_meta::load_description(dir.path()).is_none());
    }

    #[test]
    fn link_add_appends_and_writes_disk() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkAdd {
                target: links::LinkTarget::Docs,
                path: PathBuf::from("/repo/README.md"),
                kind: links::LinkKind::File,
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.links.docs.len(), 1);
        let loaded = links::load(repo.path()).unwrap();
        assert_eq!(loaded.docs.len(), 1);
    }

    #[test]
    fn link_add_dedupes_existing_path() {
        let mut ws = new_ws();
        ws.links.docs.push(links::LinkEntry {
            path: PathBuf::from("/repo/README.md"),
            kind: links::LinkKind::File,
        });
        let repo = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkAdd {
                target: links::LinkTarget::Docs,
                path: PathBuf::from("/repo/README.md"),
                kind: links::LinkKind::File,
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.links.docs.len(), 1);
    }

    #[test]
    fn link_remove_deletes_and_writes_disk() {
        let mut ws = new_ws();
        ws.links.memory.push(links::LinkEntry {
            path: PathBuf::from("/home/.claude/memory"),
            kind: links::LinkKind::Dir,
        });
        let repo = tempfile::tempdir().unwrap();
        links::save(repo.path(), &ws.links).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkRemove {
                target: links::LinkTarget::Memory,
                index: 0,
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.links.memory.is_empty());
        assert!(links::load(repo.path()).unwrap().memory.is_empty());
    }

    #[test]
    fn link_dir_toggle_expands_then_collapses() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join("docs")).unwrap();
        std::fs::write(repo.path().join("docs").join("a.md"), "").unwrap();
        let docs_path = repo.path().join("docs");
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkDirToggle {
                target: links::LinkTarget::Docs,
                path: docs_path.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.expanded_link_dirs.get(&docs_path).map(|r| r.len()),
            Some(1)
        );
        update(
            &mut ws,
            Message::LinkDirToggle {
                target: links::LinkTarget::Docs,
                path: docs_path.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(!ws.expanded_link_dirs.contains_key(&docs_path));
    }

    #[test]
    fn link_dir_toggle_selects_the_entry() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join("docs")).unwrap();
        let docs_path = repo.path().join("docs");
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkDirToggle {
                target: links::LinkTarget::Docs,
                path: docs_path.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.selected_link.as_deref(), Some(docs_path.as_path()));
    }

    #[test]
    fn link_select_only_marks_selection_without_expanding() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        let child = repo.path().join("docs").join("sub");
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkSelect {
                target: links::LinkTarget::Docs,
                path: child.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.selected_link.as_deref(), Some(child.as_path()));
        assert!(ws.expanded_link_dirs.is_empty());
    }
}
