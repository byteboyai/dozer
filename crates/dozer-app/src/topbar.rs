// crates/dozer-app/src/topbar.rs
//! 顶栏:Dozer Home 按钮 + 项目页签行(拥挤时均分收窄 + 拖拽换位 + "＋"
//! 新增项目菜单)。跟 Rail/Terminal/tab_widget/webview_geometry 那四轮
//! 不同,这里的函数唯一调用方就是 `app.rs` 自己的 `view()`——没有别的
//! 模块跨模块调用,纯粹是给这块本来就自成一体的 view 代码单独开一个
//! 文件,不是因为多处共享。项目页签的拖拽换位(`TabGroup::Project`)
//! 沿用现状,内联自己的 `MouseArea::on_move` 接线(不复用
//! `terminal.rs::tab_drag_surface`——那个已确认是终端专属,各 tab 组
//! 各自接自己的拖拽线)。
//!
//! `project_tab_switch`/`project_tab_opened` 等真正打开/关闭项目的
//! 内核编排方法不在这里,留在 `app.rs`。

use crate::app::{
    App, AppPage, HoverId, Message, TabGroup, TopbarButton, WorkspaceSlot, controlled_tooltip,
    project_dot, top_bar_font,
};
use crate::theme;
use crate::workspace::Workspace;
use byteui::interaction::{icons, tabs};
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo};
use iced_widget::core::border::Radius;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::tooltip;
use iced_widget::{MouseArea, button, column, container, responsive, row, stack, text};

/// 顶栏 Home 按钮(D1)：Lucide house(`IconKind::Home`) + "Dozer"文字,视觉、
/// 高度、选中态样式与右侧项目页签(`project_tab_item`)完全一致——同一份
/// `tab_h`、同一套 hover 胶囊/选中态实底+底部强调线,只是没有状态点和关闭
/// 按钮。恒在最左、不参与 `project_tabs_row` 的拥挤收窄——与当前项目页签
/// 行"＋"按钮同款的"固定位不参与收窄"处理。点它进首页(`AppPage::Home`)。
///
/// `active` 由调用方传入 `current_page == AppPage::Home`,与项目页签的
/// `active_project_id == Some(id)` 是两套独立状态,靠调用方各自互斥地计算
/// (见 `top_bar`/`project_tabs_row`),不然会出现两边同时"选中"的视觉冲突。
fn dozer_home_tab<'a>(
    active: bool,
    title_hover_t: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 与 `project_tab_item` 用同一份高度公式,保证两者视觉同高、顶边对齐。
    let sq = byteui::theme::icon_size::rail() + 14.0;
    let tab_h = (byteui::theme::geometry::top_bar_height() + sq) / 2.0;

    // 标题(图标 + "Dozer" 文字)颜色:选中态恒为金 `#F2D94E`(甲方动作专属色,
    // 与项目页签一致);未选中态静止 DIM,hover 时随 `title_hover_t` 平滑过渡
    // 到金(同一套悬停动画,见 `HoverId::HomeTab`)。
    let title_color = if active {
        byteui::theme::color::current().gold
    } else {
        byteui::theme::color::mix(
            byteui::theme::color::current().dim,
            byteui::theme::color::current().gold,
            title_hover_t,
        )
    };

    // `height(Fill)` + `align_y(Center)` 缺一不可:与 `project_tab_item` 同一处
    // iced 按钮布局 quirk——`button` 只加 padding、不回收多余竖向空间,内层
    // `row` 的 `align_y(Center)` 因此形同虚设,必须让这层 `container` 撑满按钮
    // 内容区、自己吃掉那截空间才能真正居中,否则 icon + "Dozer" 贴顶。这里
    // `width(Fill)` 与该项目页签同款,内层 icon / 文字才会稳稳落在按钮垂直中线。
    let label = container(
        row![
            icons::view(
                icons::IconKind::Home,
                byteui::theme::icon_size::home(),
                title_color
            ),
            text("Dozer")
                .font(top_bar_font())
                .size(byteui::theme::font::body())
                .color(title_color),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(iced_widget::core::Alignment::Center);

    // 未选中态 hover 时画一条与项目页签同款的胶囊背景(`TAB_HOVER`);选中态
    // 不参与 hover 提亮,同 `project_tab_item::select`。`width(Shrink)` 让这枚
    // 品牌页签只包住 icon + "Dozer" 本身,不抢顶栏横向空间(项目页签是
    // `Fill` 因为它要均分页签行宽度)。
    let select = button(label)
        .on_press(Message::TopBarHome)
        .width(Length::Shrink)
        .height(Length::Fixed(tab_h))
        .padding([0, 14])
        .style(move |_t: &iced_widget::Theme, s| {
            let mut st = button::Style {
                background: None,
                text_color: title_color,
                ..button::Style::default()
            };
            if !active && let button::Status::Hovered = s {
                st.background = Some(byteui::theme::color::current().tab_hover.into());
                st.border = Border {
                    radius: 8.0.into(),
                    ..Border::default()
                };
            }
            st
        });
    // 标题文字的 hover 变色走 `MouseArea` + `HoverId::HomeTab`(与项目页签的
    // `ProjectTabItem` 同款叠层:`MouseArea` 只抓 enter/exit 事件,按下仍由
    // 底层 `select` 按钮处理),驱动 `title_color` 从 DIM 平滑过渡到 GOLD。
    let select = MouseArea::new(select)
        .on_enter(Message::Hover(HoverId::HomeTab, true))
        .on_exit(Message::Hover(HoverId::HomeTab, false));

    // 选中态:实底背景(左上/右上圆角) + 底部 1px 强调线,与 `project_tab_item`
    // 同一手法——`stack!` 叠加而非 `column!`,避免强调线瓜分 `select` 的
    // `Fixed` 高度导致文字居中基准跟项目页签错位(见该函数同一处注释)。
    let inner: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = if active {
        container(stack![
            select,
            container(
                container(iced_widget::space::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(|_t: &iced_widget::Theme| container::Style {
                        background: Some(byteui::theme::color::current().tab_active_border.into()),
                        ..container::Style::default()
                    }),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ])
        .into()
    } else {
        select.into()
    };

    let tab_box = container(inner)
        .height(Length::Fixed(tab_h))
        .clip(true)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(byteui::theme::color::current().tab_active_bg.into()),
                    border: Border {
                        radius: Radius {
                            top_left: 8.0,
                            top_right: 8.0,
                            ..Radius::default()
                        },
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        });

    // 外层贴底对齐,与项目页签在 `project_tabs_row` 里的贴底方式一致
    // (那边靠 `responsive` 闭包最外层 `container(...).height(Fill).align_y(End)`,
    // 见该函数注释),这样两者的顶边才能真正对齐,而不是像旧版那样一个居中
    // 一个贴底、靠公式凑巧对齐。
    container(tab_box)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Bottom)
        .into()
}

pub(crate) fn top_bar(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // Dozer 字标做成按钮:house 图标(`IconKind::Home`) + "Dozer"文字,
    // 点它进首页(`AppPage::Home`)。Dozer 页签:视觉与右侧项目页签一致,
    // 恒在最左、不参与拥挤收窄(D1)。
    let title = dozer_home_tab(
        app.current_page == AppPage::Home,
        app.hover_progress(HoverId::HomeTab),
    );

    // 页签行占满标题与右侧之间的全部空间。裁剪与翻页在 `project_tabs_row`
    // 内部做(只裁页签本身,箭头与"＋"钉在裁剪区外),这里**不能**再套一层
    // `clip`——那会把"＋"和箭头一起裁掉,正是要修的问题。
    // 贴底对齐在 `project_tabs_row` 内部(`responsive` 闭包里)完成,这里
    // 套 `align_y` 对它不起作用,见该函数内注释。
    let tabs = container(project_tabs_row(app)).width(Length::Fill);

    let mut right = row![].spacing(10);
    // 目前尚未接入设置面板,先只还原视觉,`interactive: false` 不挂
    // on_press——没有对应 Message 变体可派发。
    right = right.push(icons::icon_button_entry(
        icons::IconKind::Settings,
        byteui::theme::icon_size::rail(),
        false,
        false,
        app.hover_progress(HoverId::Topbar(TopbarButton::Settings)),
        false,
        byteui::theme::geometry::tab_button_size(),
        false,
        Message::Noop,
        move |hovered| Message::Hover(HoverId::Topbar(TopbarButton::Settings), hovered),
        "设置",
    ));

    let region = theme::region::top_bar();
    let bar = row![title, tabs, right]
        .spacing(region.gap)
        .padding(region.padding)
        .height(Length::Fixed(byteui::theme::geometry::top_bar_height()))
        .align_y(iced_widget::core::Alignment::Center);

    // 双击顶栏空白处缩放窗口(原生标题栏没了之后,系统"双击标题栏缩放"
    // 手势只在它认为仍是标题栏的那一条区域生效,顶栏其余空白靠这层背景
    // MouseArea 手动补上)。放在 `bar` 下面这一层——iced 的点击命中是
    // 子先父后/上先下后,`bar` 里真正的按钮(页签/加号/箭头)会先吃掉
    // 落在它们身上的点击,双击事件只有落在没有任何控件的空白处才会穿透
    // 到这层背景,不会误吞正常的页签交互。
    let background = MouseArea::new(
        container(iced_widget::Space::new())
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_double_click(Message::TopBarDoubleClick);

    container(stack![background, bar])
        .width(Length::Fill)
        .height(Length::Fixed(byteui::theme::geometry::top_bar_height()))
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 一个项目页签渲染需要的三样东西:名字、状态点、id。抽出来是为了让宽度
/// 估算(`tab_window` 要各页签宽)与渲染读同一份数据,不各遍历一次
/// `project_order` 走岔。
fn project_tab_entries(app: &App) -> Vec<ProjectTabEntry> {
    app.project_order
        .iter()
        // `project_order` 与 `projects` 理论上恒一致;真出现孤儿 id 时跳过
        // 渲染而不是 panic——顺序表是要写盘的,不值得为一条脏数据崩掉 GUI。
        .filter_map(|id| app.projects.get(id).map(|slot| (*id, slot)))
        .map(|(id, slot)| match slot {
            // `Stub` 没有会话列表,但启动恢复时按 daemon 的会话快照算过一次
            // 状态点(见 [`stub_activity`]),用那份"重启时已知"的结果。
            WorkspaceSlot::Stub { info, activity } => ProjectTabEntry {
                id,
                name: info.name.clone(),
                dot: *activity,
            },
            WorkspaceSlot::Loaded(ws) => ProjectTabEntry {
                id,
                name: ws
                    .project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "加载中…".to_string()),
                dot: project_tab_dot(ws),
            },
        })
        .collect()
}

/// 见 [`project_tab_entries`]。
struct ProjectTabEntry {
    id: i64,
    name: String,
    /// 状态点颜色;`None` = 不画状态点。
    dot: Option<Color>,
}

/// 顶栏项目页签行:固定默认宽 + 拥挤时均分收窄的页签 + 紧跟最后一片页签之后的"＋"。
///
/// 每片页签的宽度按如下规则算(`project_tab_max_width()` 即"默认/合适宽"):
/// 页签少、每片都能容下默认宽时,统一用默认宽(左对齐,右侧留白,不撑爆);
/// 页签多到塞不下默认宽时,按可用宽均分,每片窄于默认宽(随实际拥挤程度收窄)。
/// 这样少数页签始终是固定的"默认宽度",只有真挤了才缩。可用宽在布局期由
///
/// `responsive` 实时拿到(不引入窗口尺寸依赖),再扣掉"＋"按钮与各处 gap。
fn project_tabs_row(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // `active_project_id` 记的是"最后聚焦的项目",跟 Dozer Home 页签是否被
    // 选中的 `current_page` 是两套独立状态(见 `dozer_home_tab` 注释)——切去
    // Home 时 `active_project_id` 不会被清空(方便切回来时记得原项目),所以
    // 页签的"选中"视觉要额外拿 `current_page` 挡一道,否则 Home 和某个项目
    // 页签会同时高亮。
    let active_project_id = (app.current_page == AppPage::Workspace)
        .then_some(app.active_project_id)
        .flatten();
    let entries = project_tab_entries(app);
    let n = entries.len();

    // `responsive` 在每轮布局把页签区可用宽交给闭包,闭包据此算每片宽。
    responsive(move |size| {
        let gap = byteui::theme::geometry::project_tab_gap();
        let default_w = byteui::theme::geometry::project_tab_max_width();
        // 预留"＋"按钮与其紧跟最后一片页签的 gap(页签内部还有 n-1 道 gap),
        // 避免页签在拥挤时压到"＋"上。
        let reserved =
            byteui::theme::geometry::project_tab_add_button_width() + (n as f32 + 1.0) * gap;
        let avail = (size.width - reserved).max(0.0);
        // 每片目标宽:少页签用默认宽(固定);多到塞不下默认宽才均分收窄。
        let per_tab = if n == 0 {
            default_w
        } else {
            let fit = avail / n as f32;
            if fit >= default_w { default_w } else { fit }
        };
        let per_tab = per_tab.max(0.0);

        let mut tabs = row![]
            .spacing(gap)
            .align_y(iced_widget::core::Alignment::Center)
            .width(Length::Shrink); // 固定宽,不撑满;右侧留白把"＋"顶到最右
        // 分割竖线高度:顶栏高的约 45%,在行内 `align_y(Center)` 自然垂直居中。
        let sep_h = byteui::theme::geometry::top_bar_height() * 0.45;
        let make_sep = || {
            container(iced_widget::space::Space::new())
                .width(Length::Fixed(1.0))
                .height(Length::Fixed(sep_h))
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(byteui::theme::color::current().border.into()),
                    ..container::Style::default()
                })
        };
        for (i, entry) in entries.iter().enumerate() {
            let active = active_project_id == Some(entry.id);
            let close_hover_t = app.hover_progress(HoverId::ProjectTabClose(entry.id));
            let title_hover_t = app.hover_progress(HoverId::ProjectTabItem(entry.id));
            let item = project_tab_item(
                entry.id,
                entry.name.clone(),
                entry.dot,
                active,
                close_hover_t,
                title_hover_t,
                app.hover_tooltip_ready(HoverId::ProjectTabItem(entry.id)),
            );
            // 固定宽:少页签时为默认宽,挤时为均分窄宽(Chrome 式收窄)。
            let cell = container(item).width(Length::Fixed(per_tab));
            // 拖拽换位:按住页签(选中处理已把 `tab_drag` 置位)后光标扫过哪个
            // 页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(TabGroup::Project);
            let mut surface = MouseArea::new(cell).on_move(move |_| Message::TabDragMove {
                group: TabGroup::Project,
                index: i,
            });
            if armed {
                surface = surface.interaction(mouse::Interaction::Grabbing);
            }
            tabs = tabs.push(surface);
            // 仅当"当前"与"下一个"页签都未选中时,二者之间插一条小竖线做
            // 分割;只要相邻任意一侧是选中态,那一侧就不画(选中页签左右都
            // 干净,既不被竖线打断,也把"当前页签"在视觉上独立出来)。
            if i + 1 < n {
                let next_active = active_project_id == Some(entries[i + 1].id);
                if !active && !next_active {
                    tabs = tabs.push(make_sep());
                }
            }
        }
        // 页签组与"＋"之间也补一条尾分割线(同"挨着选中页签不画"规则——
        // 最后一片页签被选中时不画,保持选中页签右侧干净)。
        if let Some(last) = entries.last()
            && active_project_id != Some(last.id)
        {
            tabs = tabs.push(make_sep());
        }

        let add = icons::icon_button_entry(
            icons::IconKind::SquarePlus,
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(HoverId::Topbar(TopbarButton::AddProject)),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            Message::ProjectAddMenuToggle,
            move |hovered| Message::Hover(HoverId::Topbar(TopbarButton::AddProject), hovered),
            "新建项目",
        );

        // 页签(固定宽,左对齐) + "＋"紧邻最后一片页签之后(不再用弹性留白把
        // 它顶到最右——它隶属于页签区,跟在最后一片页签后面,像浏览器新建
        // 页签的 ＋)。
        // 贴底必须在这里(闭包*内部*)包一层 `Length::Fill` + `align_y(End)`
        // 才生效——`responsive` 自身默认已是 Fill×Fill,闭包返回的内容在
        // `Responsive::layout` 里直接贴 (0,0) 摆放,外层 `top_bar()` 包多少层
        // `container(...).align_y(..)` 都摸不到它,曾经这样试过没用。
        container(
            row![tabs, add]
                .spacing(gap)
                .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::Alignment::End)
        .into()
    })
    .into()
}

/// 顶栏"＋新增项目"按钮的最近项目选择菜单:与 homespace 项目列表同源
/// (`app.recent_projects`,按 `updated_ms` 降序——同
/// `homespace::paginate_recent_projects` 的排序口径,这里不分页,一次
/// 列全),已经开着页签的项目从列表里去掉(点了也只是切过去,不如干脆
/// 不列,少一次无意义点击)。列表下面跟一条分隔线 + "新建项目"项
/// (`Message::ProjectTabPickFolder`,同顶栏按钮原有功能——rfd 文件夹
/// 选择),菜单项列表为空时不画多余的孤立分隔线。样式走 `crate::menu`
/// 标准右键菜单原语(同文件树右键菜单基准)。
///
/// 定位:"＋"按钮自己的 x 随已开页签数量浮动(`project_tabs_row` 里页签
/// 是 `Shrink` 宽、"＋"紧跟在最后一片页签之后),不像 `agent_picker_popup`
/// 那样能靠一个固定 padding 蒙对(试过左对齐、右对齐两版固定 padding,
/// 页签数量一变都会跑偏)。改用 `todo::set_calendar_anchor`/
/// `set_dispatch_anchor` 同款手法:开菜单那一刻的 `App::last_cursor`
/// (点击"＋"时的光标逻辑坐标)记进 `project_add_menu_anchor`,菜单锚定
/// 在那个真实坐标上,跟窗口边界钳制一次防止超出右/下边缘(同
/// `todo_calendar_overlay`)。
pub(crate) fn project_add_menu_popup(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !app.project_add_menu_open {
        return column![].into();
    }
    let mut projects: Vec<&ProjectInfo> = app
        .recent_projects
        .iter()
        .filter(|p| !app.projects.contains_key(&p.id))
        .collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.updated_ms));

    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        projects
            .into_iter()
            .map(|p| {
                crate::menu::item::<Message>(None, p.name.clone(), Message::ProjectSelect(p.id))
            })
            .collect();
    if !items.is_empty() {
        items.push(crate::menu::separator());
    }
    items.push(crate::menu::item::<Message>(
        Some(icons::IconKind::SquarePlus),
        "新建项目",
        Message::ProjectTabPickFolder,
    ));

    let row_count = items.len();
    let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell(
            items,
            Length::Fixed(byteui::theme::geometry::menu_item_width()),
        );

    // 全窗口容器 + padding 把弹层推到锚点,窗口边界钳制,手法同
    // `todo_calendar_overlay`。菜单宽是固定值(`menu_item_width`);高是
    // 估算——单行高 ≈ 上下 padding + 正文字号(粗估行高,不追求精确到
    // 像素,与日历弹层"估算尺寸"同一个容忍度),行数含分隔线当一整行算
    // (分隔线矮很多,整体估算偏大一点点,钳制会稍微保守但不会算少导致
    // 真的超出窗口)。
    let (ax, ay) = app.project_add_menu_anchor;
    let (window_w, window_h) = app.window_size;
    let pop_w = byteui::theme::geometry::menu_item_width();
    let region = theme::region::context_menu();
    let item_h = region.padding.top + region.padding.bottom + byteui::theme::font::body() as f32;
    let pop_h = region.padding.top
        + region.padding.bottom
        + row_count as f32 * item_h
        + (row_count.saturating_sub(1)) as f32 * region.gap;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));

    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

/// 单个项目页签:状态点(可选)+ 项目名的切换按钮 + 关闭按钮。结构与终端
/// `tab_item` 一致(两个平级按钮包在一个 container 里,不做按钮套按钮)。
#[allow(clippy::too_many_arguments)]
fn project_tab_item<'a>(
    id: i64,
    name: String,
    dot: Option<Color>,
    active: bool,
    close_hover_t: f32,
    title_hover_t: f32,
    show_tooltip: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 页签背景圆角半径参考 Dozer 按钮(圆角正方形)的边长 `sq`,但实际背景高
    // 用更高的 `tab_h`——页签贴底(见 `project_tabs_row` 的 `align_y(End)`)、
    // 底部留白必须是 0,可 Dozer 按钮在顶栏里是居中的,顶部留白
    // `(top_bar_height-sq)/2` 不为 0;要让页签顶边跟 Dozer 按钮背景顶边对齐,
    // 页签背景就不能也用 `sq` 这个高度贴底(那样顶边会比 Dozer 的更低),
    // 必须把高度补到 `(top_bar_height+sq)/2`,贴底后顶部留白才恰好等于
    // Dozer 按钮那份 `(top_bar_height-sq)/2`。
    let sq = byteui::theme::icon_size::rail() + 14.0;
    let tab_h = (byteui::theme::geometry::top_bar_height() + sq) / 2.0;
    // 关闭按钮用与顶栏其它图标按钮(tab 箭头 / 最大化)同尺寸的方形命中区。
    let close_sz = byteui::theme::geometry::tab_button_size();
    // 组合 hover:鼠标悬停标题或关闭按钮任一,都应让胶囊背景浮现、× 显形。
    // 不能只依赖 select 按钮的 `button::Status::Hovered`——× 叠在 select 之上,
    // 悬停 × 时底层 select 拿不到 `Hovered`,胶囊会凭空消失。
    let hover = title_hover_t.max(close_hover_t).clamp(0.0, 1.0);
    let hovered = hover > 0.001;
    let mut label = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(color) = dot {
        label = label.push(byteui::feedback::status::dot(color));
    }
    label = label.push(
        text(name.clone())
            .font(top_bar_font())
            .size(byteui::theme::font::body())
            // 长标题不换行:iced `Text` 默认 `Wrapping::Word`(不是曾经误以为
            // 的 `None`),不显式关掉的话,标题区被压窄时会真的折成两行,而
            // 不是靠下面 `label` 的 `.clip(true)` 单行截断——这正是"tab 标题
            // 处理较长内容时不应换行"这条验收反馈的根因。
            .wrapping(iced_widget::core::text::Wrapping::None)
            .color(if active {
                // 选中态标题恒为金 `#F2D94E`(甲方动作专属色)。
                byteui::theme::color::current().gold
            } else {
                // 未选中态:静止 DIM,hover 时平滑过渡到金(见 `ProjectTabItem`)。
                byteui::theme::color::mix(
                    byteui::theme::color::current().dim,
                    byteui::theme::color::current().gold,
                    title_hover_t,
                )
            }),
    );
    // 标签行撑满并裁剪:页签被 `FillPortion` 压窄时长名在此截断(Chrome 式
    // 无限收窄),不会把关闭按钮挤出去。
    // `height(Fill)` + `align_y(Center)` 缺一不可:`button` 的布局只加
    // padding、不回收多余竖向空间(iced_widget::button::layout 用
    // `layout::padded`,内容按 padding 定位后剩余空间原样留在下方),内层
    // `row` 的 `align_y(Center)` 因此形同虚设——必须让这层 `container` 撑满
    // 按钮内容区、自己吃掉那截空间才能真正居中,否则页签文字贴顶,与
    // `main.rs::center_traffic_lights` 摆在顶栏正中的交通灯对不齐。
    let label = container(label.align_y(iced_widget::core::Alignment::Center))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::Alignment::Center)
        // 右侧留白给叠在页签之上的关闭按钮:长名在此截断,不会跑到 × 底下。
        // 左侧留白是这里单独加的,不是靠下面 `tab_row` 的外层 padding——
        // `capsule`(悬停胶囊背景)和这层 `label` 是 `stack!` 里的平级层,共用
        // `tab_row` 那份外层 padding 定的同一个起点,只调外层 padding 只会让
        // 胶囊和文字**一起**往右挪,两者间距不变;点点因此贴着胶囊圆角左缘
        // (验收反馈截图)。真正拉开点点与胶囊边缘间距,得单独加在 `label`
        // 自己的 padding 上。
        .padding(Padding {
            right: close_sz + 4.0,
            left: 6.0,
            ..Padding::ZERO
        })
        .clip(true);

    // 选中/关闭的接线逻辑收在 `tabs::tab_core`(2026-08-12 抽取)——mousedown
    // 即选中+备拖、关闭按钮仅悬停时可点这两条规则只在一处维护。`hovered`
    // 已经在上方算好(`let hovered = hover > 0.001;`),就是原来关闭按钮挂
    // `on_press` 的判定条件,直接复用。宽高原来靠 `MouseArea` 里的 `container`
    // 撑(Fill + Fixed(tab_h)),`MouseArea` 自身不认宽高,照抄内容尺寸。
    let close_base = byteui::theme::color::mix(
        byteui::theme::color::current().dim,
        byteui::theme::color::current().gold,
        close_hover_t,
    );
    let close_color = Color {
        a: hover,
        ..close_base
    };
    let (select, close) = tabs::tab_core(tabs::TabCoreArgs {
        content: container(label)
            .width(Length::Fill)
            .height(Length::Fixed(tab_h))
            .into(),
        close_sz,
        close_color,
        close_interactive: hovered,
        on_select: Message::ProjectTabSwitch(id),
        on_close: Message::ProjectTabClose(id),
        on_select_hover: move |hovered| Message::Hover(HoverId::ProjectTabItem(id), hovered),
        on_close_hover: move |hovered| Message::Hover(HoverId::ProjectTabClose(id), hovered),
    });

    // 页签主体(select)为底层、关闭按钮为上层叠在其右:关闭按钮视觉上落在
    // 页签背景里,而非独立的相邻按钮。两层都 `Fill` 撑满整条顶栏高,select
    // 用容器垂直居中、close 用容器靠右居中;横向内缩 14。
    let select_layer = container(select)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    let close_layer = container(close)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Right)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    // 悬停胶囊:与 select 按钮同高同圆角、铺满整片页签(含右缘 × 区),由组合
    // hover 进度驱动透明度——只在悬停页签时浮现,且 × 落在其内部。选中态已有
    // 实底背景(TAB_ACTIVE_BG),不再叠胶囊。
    let capsule = container(iced_widget::space::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(tab_h))
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if !active && hover > 0.0 {
                Some(
                    Color {
                        a: hover,
                        ..byteui::theme::color::current().tab_hover
                    }
                    .into(),
                )
            } else {
                None
            },
            border: if !active && hover > 0.0 {
                Border {
                    radius: 8.0.into(),
                    ..Border::default()
                }
            } else {
                Border::default()
            },
            ..container::Style::default()
        });
    let capsule_layer = container(capsule)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    let tab_row = container(stack![capsule_layer, select_layer, close_layer])
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 12.0,
        });

    // 激活态:实底背景(左上/右上圆角) + 底部 1px 强调线
    // (`#dcc9a3` = `TAB_ACTIVE_BORDER`),不要外边框;未激活态:无背景、无边框
    // (仅 hover 时画胶囊,见上)。强调线用 `stack!` 叠在 `tab_row` 之上(贴底
    // 对齐),不能用 `column!` 把它当 `tab_row` 的兄弟项——`column!` 会从
    // `tab_row` 的 `Fill` 高度里瓜分掉这 1px,导致选中页签的 `select`
    // 按钮比未选中页签矮 1px,标题文字的居中基准跟着偏,与未选中页签的
    // 标题对不上(貌似"没对齐"的根因)。`stack!` 的每一层都吃满同一块
    // 区域,不会互相抢空间。
    let inner = if active {
        container(stack![
            tab_row,
            container(
                container(iced_widget::space::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(|_t: &iced_widget::Theme| container::Style {
                        background: Some(byteui::theme::color::current().tab_active_border.into()),
                        ..container::Style::default()
                    }),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
    } else {
        tab_row
    };

    // 背景高 `tab_h`(见上,比 `sq` 高),贴底放进 `project_tabs_row` 的行里后
    // 顶部留白与 Dozer 按钮背景顶部留白相等,视觉上两者顶边对齐,底部则贴到
    // 顶栏下沿(页签式,与内容区无缝衔接)。未激活态同样高 `tab_h`、无背景;
    // 这里的 `align_y` 对贴底本身不起作用(那层在 `project_tabs_row` 的
    // `container(...).align_y(End)` 完成),留着只是 iced `container` 布局
    // 惯例、无空间可分配时是无操作。
    let el: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = container(inner)
        .height(Length::Fixed(tab_h))
        .width(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(byteui::theme::color::current().tab_active_bg.into()),
                    // 仅左上/右上圆角,底部 1px 强调线由 inner 承载(见上)。
                    border: Border {
                        radius: Radius {
                            top_left: 8.0,
                            top_right: 8.0,
                            ..Radius::default()
                        },
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into();
    // 顶栏页签在屏幕顶部,tooltip 用 `Bottom` 弹在页签下方,免出屏。仅当
    // 悬停满 2s(`show_tooltip`)才显示标题全称。
    controlled_tooltip(el, name, tooltip::Position::Bottom, show_tooltip)
}

/// 一个项目页签的后台活动指示点:取该项目所有**存活**会话里最值得关注的
/// 那个状态。返回状态点颜色;`None` = 没有存活会话,不画点。
fn project_tab_dot(ws: &Workspace) -> Option<Color> {
    let alive: Vec<(AgentState, AgentKind)> = ws
        .tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| (t.agent_state, t.agent))
        .collect();
    project_dot(&alive)
}
