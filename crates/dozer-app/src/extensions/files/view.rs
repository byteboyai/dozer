//! Files 面板 view:文件树列表/行内编辑/git footer/分支选择器/右键菜单/
//! 删除与移动确认浮层。

use crate::menu_spec::{MenuSpec, MenuSpecItem};
use crate::theme;
use byteui::interaction::icons;
use iced_widget::core::{Element, Length, Padding};
use iced_widget::{column, container, text};

use super::*;

pub(crate) use dozer_panel_files::dialogs::delete_confirm_spec;
pub(crate) use dozer_panel_files::view::{files_move_card, view};

/// 触发错误;同时给当前分支行追加 "(Uncommitted)" 提示。
pub fn branch_picker_popup<'a>(
    ws_state: &'a WorkspaceState,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws_state.git_loaded || !ws_state.git_is_repo || !ws_state.branch_picker_open {
        return iced_widget::space::Space::new().into();
    }
    let current = ws_state.current_branch.as_deref();
    // 当前分支是否带未提交改动(dirty)?是则锁定其余分支(禁用切换)并给
    // 当前分支行追加 "(Uncommitted)"。
    let is_dirty = ws_state.git_dirty();
    // dirty → 除当前分支外的其余分支全部置灰禁用。
    let lock_others = is_dirty;
    // 面板项/间隔统一走 `crate::chrome::menu`(样式基准即文件树右键菜单)。
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    if ws_state.git_branches.is_empty() {
        items.push(
            text(t("git.no_local_branches", "No local branches", locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    for name in &ws_state.git_branches {
        let is_current = Some(name.as_str()) == current;
        // 当前分支 GOLD 高亮 + 指示点;其余分支:dirty 锁定时 DIM 置灰,否则
        // 常规 BODY(同上下文菜单项文字)。
        let color = if is_current {
            byteui::theme::color::current().gold
        } else if lock_others {
            byteui::theme::color::current().dim
        } else {
            byteui::theme::color::current().body
        };
        let indicator: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if is_current {
                text("● ")
                    .size(byteui::theme::font::body())
                    .color(color)
                    .into()
            } else {
                iced_widget::space::Space::new()
                    .width(Length::Fixed(18.0))
                    .into()
            };
        let label = {
            let mut n = name.clone();
            if is_current && is_dirty {
                n.push_str("(Uncommitted)");
            }
            n
        };
        items.push(crate::chrome::menu::item_row(
            Some(indicator),
            label,
            color,
            // dirty 锁定时,非当前分支不可点(不挂 `on_press`)。
            (is_current || !lock_others).then(|| Message::BranchSwitch(name.clone())),
        ));
    }
    let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::chrome::menu::shell_frosted(items, Length::Shrink);

    // 把下拉框钉到 git 底栏正上方:左缘对齐文件面板(左图标栏 + project_pane
    // 左 padding),底缘对齐 git 底栏顶部(footbar 高 + project_pane 底 padding
    // + git 底栏自身高)。外层容器铺满全窗,靠 `Padding{left,bottom}` + 子原件
    // `align_x(Start)`/`align_y(End)` 把它推到左下角(仅 `bottom` padding 而不
    // `align_y(End)` 时,Shrink 高子原件会落在内容区**顶部**,菜单就跑到窗口
    // 最上方去了——与右键菜单 `top` 定位同源,方向相反)。宽度用 `Shrink` 让
    // 菜单贴合最宽项,不会铺满窗口右缘。
    let (left, bottom) = branch_picker_popup_offset(ws_state);
    iced_widget::Container::new(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom,
            left,
        })
        .align_x(iced_widget::core::Alignment::Start)
        .align_y(iced_widget::core::Alignment::End)
        .into()
}

/// `branch_picker_popup` 的基准偏移:左缘=左图标栏宽 + project_pane 左 padding;
/// 底缘=footbar 高 + project_pane 底 padding + git 底栏高。二者都吃全局 scale,
/// 随主题/缩放联动,不写死像素。
fn branch_picker_popup_offset(ws_state: &WorkspaceState) -> (f32, f32) {
    let rail = byteui::theme::geometry::icon_rail_width();
    let pane = theme::region::project_pane();
    let left = rail + pane.padding.left;
    // git 底栏高度:顶部分隔 1px + 栏内容(icon_box + 上下 padding 6) + 栏间
    // spacing 4 + 可能的 git_error 一行;project_pane gap 计入把下拉钉紧底栏。
    let git_bar_top_line = 1.0;
    let git_bar_vpad = 6.0 * 2.0;
    let bar_h = byteui::theme::icon_size::row() + 12.0;
    let error_line = if ws_state.git_error.is_some() {
        18.0
    } else {
        0.0
    };
    let git_bar_h = git_bar_top_line + bar_h + git_bar_vpad + 4.0 + error_line;
    let bottom =
        byteui::theme::geometry::footbar_height() + pane.padding.bottom + git_bar_h + pane.gap;
    (left, bottom)
}

/// `context_menu_popup` 的原生菜单版本——纯数据组装,不碰渲染/AppKit,和
/// 旧版共用完全相同的条件分支(是否目录/是否根/是否有剪贴内容),方便
/// 单测覆盖,行为上二者应保持一致。仅 macOS 编译(`native_menu` 是平台专属
/// 模块),非 mac 平台继续走 `context_menu_popup` 的 iced 弹层。数据组装
/// 收拢到 `context_menu_spec()`,这里只做 native 转换。
#[cfg(target_os = "macos")]
pub(crate) fn agent_context_reference_text(
    relative_path: &str,
    is_dir: bool,
    locale: &str,
) -> String {
    if is_dir {
        t_args(
            "agent.ref_dir",
            "Include directory {relative_path}/ in your working context.",
            locale,
            &[("relative_path", relative_path)],
        )
    } else {
        t_args(
            "agent.ref_file",
            "Include file {relative_path} in your working context.",
            locale,
            &[("relative_path", relative_path)],
        )
    }
}

/// 预览内选区"发送给 Agent"的引用文本——坐标沿用 Phase 1 `apply_precise_edit`
/// 同一套 1-based line/col 格式,agent 若要直接照做修改可以跳过
/// `locate_in_file` 直接调 `apply_precise_edit`。不用代码围栏包裹
/// `selected_text`,原样拼接,避免选中内容本身含反引号/围栏时破坏格式。
pub(crate) fn selection_reference_text(
    relative_path: &str,
    range: crate::preview::TextRange,
    selected_text: &str,
    locale: &str,
) -> String {
    t_args(
        "agent.ref_selection",
        "Refer to {relative_path}:{sl}:{sc}-{el}:{ec} this content:\n\n{selected_text}",
        locale,
        &[
            ("relative_path", relative_path),
            ("sl", range.start.line.to_string().as_str()),
            ("sc", range.start.column.to_string().as_str()),
            ("el", range.end.line.to_string().as_str()),
            ("ec", range.end.column.to_string().as_str()),
            ("selected_text", selected_text),
        ],
    )
}

/// 文件树右键菜单内容——native(`context_menu_items`)和 iced fallback
/// (`context_menu_popup`)共用同一份数据,按"顶部操作组 / 中间主操作组 /
/// 底部工具组"三段组装,条件分支只写一遍。
///
/// 结构(2026-09-18 调整):
/// - 文件夹:顶部 = 搜索 / 新建文件 / 新建文件夹(三者紧贴,组前无分隔线);
///   中间 = 复制 / 粘贴(仅目录) / 删除 / 重命名(非根才有);底部 = 复制目录
///   名称(文件为"复制文件名称") / 复制相对路径 / 复制绝对路径 / 用外部软件
///   打开(目录为"在Finder中打开") / 从磁盘重新加载。
/// - 文件:顶部 = 回滚(undo-2) / 历史(file-clock),仅 git 仓库文件才有;
///   中间 = 复制 / 删除 / 重命名(文件不显示"粘贴"——粘贴是"粘贴进目标
///   目录",对文件无语义);底部同文件夹。
/// - 三组之间用分隔线隔开;顶部组为空(非 git 文件)时不画组前分隔线,避免
/// 右键菜单浮层本体:纵向按钮列表,`container` 用 `Padding{top,left,..}`
/// 手算定位到点击坐标——`Stack` 各层共享同一份 bounds,不像原生系统菜单
/// 那样自带绝对定位,这是本仓一贯的手算像素定位风格(`ime_cursor_area`/
/// `content_pane_bounds` 同款)。
pub fn context_menu_popup<'a>(
    app_state: &'a AppState,
    _locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = &app_state.context_menu else {
        return column![].into();
    };
    let list = menu_items_to_iced(app_state.context_menu_entries.clone(), Length::Shrink);
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: menu.y,
            left: menu.x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

/// 库产出的菜单条目 → iced 浮层(非 mac 的 fallback)。样式与原生菜单同一
/// 条规则:禁用项不挂 `on_press`。
pub fn menu_items_to_iced<'a>(
    items: Vec<dozer_panel_files::menu::MenuItem<Message>>,
    width: Length,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    use dozer_panel_files::menu::MenuItem;
    let body: Vec<_> = items
        .into_iter()
        .map(|item| match item {
            MenuItem::Entry {
                icon,
                label,
                enabled,
                msg,
            } => {
                let color = if enabled {
                    byteui::theme::color::current().body
                } else {
                    byteui::theme::color::current().dim
                };
                let leading = crate::chrome::menu::icon_leading(icon, color);
                if enabled {
                    crate::chrome::menu::item_row(leading, label, color, Some(msg))
                } else {
                    crate::chrome::menu::item_locked(icon, label, color)
                }
            }
            MenuItem::Separator => crate::chrome::menu::separator(),
        })
        .collect();
    crate::chrome::menu::shell_frosted(body, width)
}

pub fn tab_context_menu_popup<'a>(
    app_state: &'a AppState,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = app_state.tab_context_menu() else {
        return column![].into();
    };
    let _ = locale;
    let list = menu_items_to_iced(app_state.tab_context_menu_entries.clone(), Length::Shrink);
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: menu.y,
            left: menu.x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

/// `IconKind::MessageSquare` 强化"同一类操作"的视觉关联。注意消息类型是
/// `crate::app::Message`(顶层),不是本文件裸 `Message`(= `files::Message`)
/// ——触发源是 `EditorEvent` 分发而不是文件树右键。
pub(crate) fn preview_selection_context_menu_spec(
    menu: &crate::app::PreviewSelectionContextMenu,
    locale: &str,
) -> MenuSpec<crate::app::Message> {
    let dim = byteui::theme::color::current().dim;
    vec![MenuSpecItem::Entry {
        icon: Some(icons::IconKind::MessageSquare),
        icon_color: None,
        label: t("menu.send_to_agent", "Send to Agent", locale),
        color: if menu.agent_terminal_visible {
            byteui::theme::color::current().body
        } else {
            dim
        },
        enabled: menu.agent_terminal_visible,
        msg: crate::app::Message::SendSelectionToAgent {
            project_id: menu.project_id,
            path: menu.path.clone(),
            range: menu.range,
            selected_text: menu.selected_text.clone(),
        },
    }]
}

#[cfg(target_os = "macos")]
pub fn preview_selection_context_menu_items(
    menu: &crate::app::PreviewSelectionContextMenu,
    locale: &str,
) -> Vec<crate::chrome::native_menu::Item<crate::app::Message>> {
    crate::menu_spec::to_native(preview_selection_context_menu_spec(menu, locale))
}

/// 非 mac 的 iced fallback 渲染——恒可调用(内部自己判空),与
/// `context_menu_popup` 同一约定。mac 上 `preview_context_menu` 恒为
/// `None`(原生菜单不存状态),此函数在 mac 编译但不被走到。
pub fn preview_selection_context_menu_popup<'a>(
    preview_context_menu: &'a Option<crate::app::PreviewSelectionContextMenu>,
    locale: &'a str,
) -> Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = preview_context_menu else {
        return column![].into();
    };
    let spec = preview_selection_context_menu_spec(menu, locale);
    let list = crate::menu_spec::to_iced(spec, Length::Shrink);
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: menu.y,
            left: menu.x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_context_reference_text_for_file() {
        assert_eq!(
            agent_context_reference_text("src/main.rs", false, "en"),
            "Include file src/main.rs in your working context."
        );
    }

    #[test]
    fn agent_context_reference_text_for_dir() {
        assert_eq!(
            agent_context_reference_text("research", true, "en"),
            "Include directory research/ in your working context."
        );
    }

    #[test]
    fn agent_context_reference_text_keeps_unicode_and_spaces_verbatim() {
        assert_eq!(
            agent_context_reference_text("我的 报告/draft v2.md", false, "en"),
            "Include file 我的 报告/draft v2.md in your working context."
        );
    }

    #[test]
    fn selection_reference_text_formats_range_and_body() {
        let range = crate::preview::TextRange {
            start: crate::preview::TextPosition { line: 3, column: 1 },
            end: crate::preview::TextPosition { line: 5, column: 4 },
        };
        let text = selection_reference_text("src/main.rs", range, "let x = 1;", "en");
        assert_eq!(
            text,
            "Refer to src/main.rs:3:1-5:4 this content:\n\nlet x = 1;"
        );
    }

    #[test]
    fn selection_reference_text_does_not_mangle_backticks_or_newlines() {
        let range = crate::preview::TextRange {
            start: crate::preview::TextPosition { line: 1, column: 1 },
            end: crate::preview::TextPosition { line: 2, column: 1 },
        };
        let body = "```diff\n- old\n+ new\n```";
        let text = selection_reference_text("a.md", range, body, "en");
        assert!(text.ends_with(body), "选中内容需原样保留,不被模板转义/截断");
    }

    fn sample_selection_menu(
        agent_terminal_visible: bool,
    ) -> crate::app::PreviewSelectionContextMenu {
        crate::app::PreviewSelectionContextMenu {
            project_id: 7,
            x: 0.0,
            y: 0.0,
            panel: crate::app::PanelKind::Files,
            tab_id: 1,
            path: std::path::PathBuf::from("a.rs"),
            range: crate::preview::TextRange {
                start: crate::preview::TextPosition { line: 1, column: 1 },
                end: crate::preview::TextPosition { line: 1, column: 2 },
            },
            selected_text: "x".into(),
            agent_terminal_visible,
        }
    }

    #[test]
    fn preview_selection_menu_disabled_without_agent_terminal() {
        let menu = sample_selection_menu(false);
        let spec = preview_selection_context_menu_spec(&menu, "en");
        let enabled = spec.iter().find_map(|item| match item {
            MenuSpecItem::Entry { enabled, .. } => Some(*enabled),
            _ => None,
        });
        assert_eq!(enabled, Some(false), "无可见终端时该菜单项应置灰");
    }

    #[test]
    fn preview_selection_menu_enabled_with_agent_terminal() {
        let menu = sample_selection_menu(true);
        let spec = preview_selection_context_menu_spec(&menu, "en");
        let enabled = spec.iter().find_map(|item| match item {
            MenuSpecItem::Entry { enabled, .. } => Some(*enabled),
            _ => None,
        });
        assert_eq!(enabled, Some(true));
    }

    /// 回归测试:菜单点击产生的 `SendSelectionToAgent` 必须带上建菜单时
    /// 捕获的 `project_id`(发起这次右键的项目),而不是留给 handler 处理
    /// 消息时再去猜"当前激活的是哪个项目"——后者在非 mac 的非阻塞弹层下,
    /// 用户切换项目标签后点击,会把内容送错项目。
    #[test]
    fn preview_selection_menu_click_carries_originating_project_id() {
        let menu = sample_selection_menu(true);
        let spec = preview_selection_context_menu_spec(&menu, "en");
        let msg = spec.iter().find_map(|item| match item {
            MenuSpecItem::Entry { msg, .. } => Some(msg.clone()),
            _ => None,
        });
        assert!(matches!(
            msg,
            Some(crate::app::Message::SendSelectionToAgent { project_id: 7, .. })
        ));
    }
}
