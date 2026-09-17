//! 项目面板的 view:主视图/footer/scaffold 进度弹层/删除确认/文档链接段。

use byteui::interaction::icons;
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, button, column, container, row, text};
use std::path::PathBuf;

use super::*;

/// 面板主入口——`project` 为 `None` 时内核不会真正走到这里
/// (`left_panel_area` 对 `PanelKind::Project` 无条件调用本函数,但
/// `App::view()` 顶层只在有聚焦项目时才会渲染到这个分支),这里仍保留一次
/// 防御性判断,风格对齐 Files 试点。这是配对布局里**可收起的 list 侧**
/// (与 `project_preview_pane` 配对,`app.rs` 的 `PanelKind::Project` 分支
/// 里 `app.list_collapsed(PanelKind::Project)` 收起的就是这块;上面这句
/// 旧注释说"单栏不配对"已经不对,大概是配对/收起功能后补的遗留)。
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

    content = content.push(crate::chrome::homespace::home_panel_head(
        icons::IconKind::Briefcase,
        "项目",
    ));

    let editing = ws_state.name_edit_focused();
    let name_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if ws_state.name_editing.is_some() {
            container(byteui::interaction::context_menu::wrap(
                byteui::form::input_text::view(
                    "",
                    ws_state.name_editing.as_deref().unwrap_or(""),
                    false,
                    Some(name_field_id()),
                    false,
                    Some(Message::NameEditSubmit),
                    true,
                    Message::NameEditInput,
                ),
                Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: name_field_id(),
                    secure: false,
                })),
            ))
            .padding([8, 12])
            .width(Length::Fill)
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: if editing {
                            byteui::theme::color::current().gold
                        } else {
                            byteui::theme::color::current().border
                        },
                        width: 1.5,
                        radius: 8.0.into(),
                    },
                    ..iced_widget::container::Style::default()
                },
            )
            .into()
        } else {
            button(
                text(p.name.clone())
                    .size(byteui::theme::font::title())
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
            let editor: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                iced_widget::text_editor(editing)
                    .id(description_field_id())
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
                    .into();
            byteui::interaction::context_menu::wrap(
                editor,
                Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: description_field_id(),
                    secure: false,
                })),
            )
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
            button(text(label).size(byteui::theme::font::body()).color(color))
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
                .size(byteui::theme::font::label())
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
                    .size(byteui::theme::font::label())
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
                .size(byteui::theme::font::caption())
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
                    .size(byteui::theme::font::label())
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
                    .size(byteui::theme::font::caption())
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
                        .size(byteui::theme::font::caption())
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
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    let body = column![content, project_footer_bar()].spacing(0);

    let base =
        container(body)
            .width(width)
            .height(Length::Fill)
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    // 可收起的 list 侧用 `bg`,不用 `panel`——`panel` 是配对
                    // 布局里不可收起那侧(`project_preview_pane`)的色,此前
                    // 这里写成 `panel` 在深色主题因 `bg == panel` 看不出来,
                    // 浅色/深色拆开后穿帮(2026-09-16 用户反馈"浅色主题-
                    // 项目面板的可收起部分颜色不对")。其它配对面板(Files/
                    // Todo/SSH/Database/Agent/Conversations)要么走
                    // `theme::region::project_pane()`(已改 BG)要么直接硬编
                    // 码 `bg`,只有这里当时手滑写成了 `panel`。
                    background: Some(byteui::theme::color::current().bg.into()),
                    border: outer,
                    ..iced_widget::container::Style::default()
                },
            );

    // 「删除项目」确认框 / 「修复项目」进度弹窗**不**在这里叠(此前的
    // panel-level `stack!` 只在本面板的 `width` 范围内居中,而不是整个
    // 软件窗体——2026-09-15 改为窗口级 overlay,由 `app.rs` 顶层
    // `popped` 分支挂载,同 `todo::clear_confirm_popup` 的既有口径,见
    // `project_delete_confirm_popup`/`scaffold_progress_popup` 文档。
    base.into()
}

/// 项目信息面板底部 footer-bar,结构与文件树面板的 `git_footer_bar` 一致:
/// 1px `BORDER` 分隔线 + `padding([6, 8])` 容器。当前放「修复项目 / 删除项目」
/// 两个并排圆角按钮:「修复项目」触发 `RepairProject`(见 `spawn_repair_run`,
/// 弹出逐步骤进度弹窗),「删除项目」触发 `DeleteProjectRequest` 打开三选一
/// 确认弹窗(见 `project_delete_confirm_popup`)。
/// footer-bar 按钮的居中图标 + 文字内容:`button` 的 `layout::padded` 不会
/// 把 Shrink 宽度的内容居中(只贴左上角),要靠外层 `container` 自己撑满
/// `Length::Fill` 再 `align_x(Center)` 才能让图标+文字这组内容整体居中。
fn footer_button_label<'a>(
    icon: icons::IconKind,
    label: &'a str,
    color: iced_widget::core::Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        row![
            icons::view(icon, byteui::theme::icon_size::row(), color),
            text(label).size(byteui::theme::font::label()).color(color),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .into()
}

/// 信息面板可收纳的最小宽度:拖拽 `Divider::ProjectSplit` 时,低于此宽度
/// 应直接把信息面板收起(等同点一次 footer 上方的收起按钮),而不是把下面
/// `project_footer_bar` 的「修复项目/删除项目」两个按钮挤到显示不全。
/// 数值按 `footer_button_label`/`project_footer_bar` 自身的间距/内边距公式
/// 推出,不是拍脑袋常量——改这两个函数的图标/文案/间距/内边距时要跟着调:
/// 单按钮 ≈ 图标(`icon_size::row`) + 图标文字间距(6) + 四字 CJK 文案
/// (按 `font::label` 每字约 1em 估) + 按钮左右 padding(8×2);两个按钮
/// + 按钮间 spacing(6) + 外层 footer container 左右 padding(8×2)。
pub(crate) fn footer_min_width() -> f32 {
    let icon = byteui::theme::icon_size::row();
    let label_px = byteui::theme::font::label() as f32;
    let button_w = icon + 6.0 + label_px * 4.0 + 16.0;
    button_w * 2.0 + 6.0 + 16.0
}

fn project_footer_bar() -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let repair = button(footer_button_label(
        icons::IconKind::BriefcaseMedical,
        "修复项目",
        byteui::theme::color::current().dim,
    ))
    .on_press(Message::RepairProject)
    .width(Length::Fill)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: crate::dialog::action_button_border_color(s),
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().dim,
        ..iced_widget::button::Style::default()
    });

    let delete = button(footer_button_label(
        icons::IconKind::FolderX,
        "删除项目",
        byteui::theme::color::current().red,
    ))
    .on_press(Message::DeleteProjectRequest)
    .width(Length::Fill)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: crate::dialog::action_button_border_color(s),
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

    let col = column![top_line, bar].spacing(4);

    container(col)
        .width(Length::Fill)
        .padding([6, 8])
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// "修复项目"弹窗单行:左侧状态符号 + 步骤名 + 右侧简短详情文字。
fn scaffold_step_row<'a>(
    label: &'a str,
    state: &'a ScaffoldStepState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (glyph, glyph_color, detail): (&str, iced_widget::core::Color, String) = match state {
        ScaffoldStepState::Pending => ("○", byteui::theme::color::current().dim, String::new()),
        ScaffoldStepState::Running => (
            "…",
            byteui::theme::color::current().gold,
            "进行中".to_string(),
        ),
        ScaffoldStepState::Done(scaffold::ScaffoldStepResult::AlreadyOk) => (
            "✓",
            byteui::theme::color::current().green,
            "已是最新".to_string(),
        ),
        ScaffoldStepState::Done(scaffold::ScaffoldStepResult::Created(msg)) => {
            ("✓", byteui::theme::color::current().green, msg.clone())
        }
        ScaffoldStepState::Done(scaffold::ScaffoldStepResult::Failed(msg)) => {
            ("✗", byteui::theme::color::current().red, msg.clone())
        }
    };
    row![
        text(glyph)
            .size(byteui::theme::font::body())
            .color(glyph_color),
        text(label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream)
            .width(Length::Fixed(140.0)),
        text(detail)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

fn scaffold_backfill_row(
    state: &BackfillStepState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (glyph, glyph_color, detail) = match state {
        BackfillStepState::Pending => (
            "○".to_string(),
            byteui::theme::color::current().dim,
            String::new(),
        ),
        BackfillStepState::Running(p) => (
            "…".to_string(),
            byteui::theme::color::current().gold,
            format!("{}/{}", p.completed, p.total),
        ),
        BackfillStepState::Done(p) if p.total == 0 => (
            "✓".to_string(),
            byteui::theme::color::current().green,
            "无需补".to_string(),
        ),
        BackfillStepState::Done(p) => (
            "✓".to_string(),
            byteui::theme::color::current().green,
            format!("{}/{}", p.completed, p.total),
        ),
    };
    row![
        text(glyph)
            .size(byteui::theme::font::body())
            .color(glyph_color),
        text("补总结")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream)
            .width(Length::Fixed(140.0)),
        text(detail)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

/// "修复项目"进度弹窗:视觉模板同 `project_delete_confirm_popup`(卡片 +
/// 底部按钮)。进行中时"关闭"按钮不可点(`on_press_maybe`),全部完成
/// (`ScaffoldRunState::all_done`)才激活。窗口级 overlay,由 `app.rs`
/// 挂载(见其调用点注释),`pub` 是为了让那边能调到。
pub fn scaffold_progress_popup(
    ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(run) = &ws_state.scaffold_run else {
        return container(column![]).into();
    };
    let mut rows = column![].spacing(10);
    for (label, state) in &run.steps {
        rows = rows.push(scaffold_step_row(label, state));
    }
    rows = rows.push(scaffold_backfill_row(&run.backfill));

    let done = run.all_done();
    let close_label = if done { "关闭" } else { "进行中…" };
    let close_btn = button(
        text(close_label)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .on_press_maybe(done.then_some(Message::ScaffoldPopupClose))
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().dim,
    ));

    let title = row![
        icons::view(
            icons::IconKind::Briefcase,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("修复项目")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let card = column![
        title,
        rows,
        // 之前 `container(close_btn).align_x(Right)` 没给容器显式宽度,
        // 默认 `Shrink`——容器跟按钮本身一样宽,`align_x` 无从对齐起,视觉
        // 上就是贴左(2026-09-15 用户反馈)。改用 `dialog::actions`(同其它
        // 弹窗底部按钮行的既有约定),内部套了 `width(Fill)` 才会真正靠右。
        crate::dialog::actions(row![close_btn]),
    ]
    .spacing(14);

    // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定)——此前固定
    // 360px,窗口变宽变窄时弹窗大小不跟着变,跟其它弹窗的写死像素值互相
    // 不一致。
    let dialog = container(card)
        .width(crate::dialog::width(window_width))
        .padding(16)
        .style(crate::dialog::card_style);

    // 之前这里直接返回卡片本体,在外层 `stack!` 里默认贴左上角——同类
    // "删除项目"确认弹窗(`project_delete_confirm_popup`)/SSH 删主机确认
    // (`ssh.rs::delete_confirm_popup`)都套了一层 `Fill` + `Center` 才居中,
    // 这里漏了这一层,视觉上不像"普通弹窗"(2026-09-15 修复)。
    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 「删除项目」三选一确认弹窗,视觉模板同 `ssh.rs::delete_confirm_popup`
/// (卡片 + 取消/确认按钮)。单选行复用 `ssh.rs::radio_dot` 的视觉语言
/// (选中态 GOLD 实心描边,未选中态空心 BORDER 描边)——ssh 的 `radio_dot`
/// 绑定在 `ssh::Message` 上、跨模块复用不了类型,这里就地画一份同样的视觉。
/// 窗口级 overlay,由 `app.rs` 挂载(见其调用点注释),`pub` 是为了让那边
/// 能调到。
pub fn project_delete_confirm_popup(
    ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let selected = ws_state
        .delete_pending
        .unwrap_or(delete::DeleteScope::DozerOnly);

    let radio_row = |scope: delete::DeleteScope, label: &'static str| {
        let is_selected = selected == scope;
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(10.0))
            .height(Length::Fixed(10.0))
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: if is_selected {
                        Some(byteui::theme::color::current().gold.into())
                    } else {
                        None
                    },
                    border: if is_selected {
                        Border {
                            color: byteui::theme::color::current().gold,
                            width: 1.5,
                            radius: 5.0.into(),
                        }
                    } else {
                        Border {
                            color: byteui::theme::color::current().border,
                            width: 1.5,
                            radius: 5.0.into(),
                        }
                    },
                    ..iced_widget::container::Style::default()
                },
            );
        let ring = container(dot)
            .width(Length::Fixed(16.0))
            .height(Length::Fixed(16.0))
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center);
        MouseArea::new(
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
        .on_press(Message::DeleteProjectScopeSelect(scope))
    };

    let cancel = button(
        text("取消")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::DeleteProjectCancel)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().dim,
    ));
    let confirm = button(
        text("删除")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::DeleteProjectConfirm)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().red,
    ));

    let title = row![
        icons::view(
            icons::IconKind::FolderX,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("删除项目")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let dialog = container(
        column![
            title,
            text("选择删除范围,操作会把对应内容移入系统回收站(可找回)。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            column![
                radio_row(delete::DeleteScope::DozerOnly, "只删 dozer 关联与缓存文件"),
                radio_row(
                    delete::DeleteScope::WithAgentCache,
                    "以上 + 所有 agent 缓存数据"
                ),
                radio_row(
                    delete::DeleteScope::WithProjectFiles,
                    "以上 + 项目文件与版本仓库"
                ),
            ]
            .spacing(8),
            crate::dialog::actions(row![cancel, confirm].spacing(8)),
        ]
        .spacing(12),
    )
    // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定)——此前没给
    // 显式宽度,靠内容(三行单选文案)撑开,窗口变宽变窄时弹窗大小不跟着变。
    .width(crate::dialog::width(window_width))
    .padding(16)
    .style(crate::dialog::card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
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
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream),
            iced_widget::space::horizontal(),
            button(
                text("+")
                    .size(byteui::theme::font::label())
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
                    .size(byteui::theme::font::body())
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
                            .size(byteui::theme::font::caption())
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
