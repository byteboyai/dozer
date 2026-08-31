//! 验收(甲方验收)面板:项目目标标准复选、变更文件列表 + 自带的精简
//! unified diff 预览、验收意见、通过/打回。阶段 1 扩展化重构第六个试点,
//! 但跟前五个不同——这是一次产品级 UX 改动(从 `PreviewPane` 里靠横幅
//! 点开的特殊 tab 提升为独立 rail 面板),设计见
//! `docs/superpowers/specs/2026-08-08-acceptance-pane-design.md`。
use crate::delivery::FileChange;
use crate::goal::Goal;
use dozer_client::Client;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Length, Rectangle};
use iced_widget::{button, column, container, row, text};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// 挂在每个 Workspace 上的验收面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    session: Option<AcceptanceSession>,
}

impl WorkspaceState {
    pub fn session(&self) -> Option<&AcceptanceSession> {
        self.session.as_ref()
    }

    /// 意见框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn comment_focused(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.comment_focused)
    }

    /// 每帧渲染循环读走 `CaptureCommentFocus` 查到的真实焦点态后写进来。
    pub fn set_comment_focused(&mut self, focused: bool) {
        if let Some(s) = &mut self.session {
            s.comment_focused = focused;
        }
    }

    /// 供内核 `Open` 拦截处理后落地新会话——不经过 `Message::Loaded`,
    /// 因为 `Loaded` 本身就是通过 `update` 落地的(见 `update` 的
    /// `Loaded` 分支)。这个方法留给测试/未来直接构造场景用,`update`
    /// 内部处理 `Loaded` 时直接赋值字段,不调用它——两条路径做同一件事,
    /// 保留这个方法只是为了让"新建会话"这个操作有一个命名清楚的入口。
    #[cfg(test)]
    fn set_session_for_test(&mut self, session: AcceptanceSession) {
        self.session = Some(session);
    }

    /// 供内核 `Reject` 拦截处理完成后调用——清空验收会话状态,不经过
    /// `Message`/`update`(同 Todo 试点"内核完成终端相关部分后直调扩展
    /// 普通函数"的处理方式)。
    pub fn clear_session(&mut self) {
        self.session = None;
    }

    /// 供内核 `Reject` 拦截处理的"来源会话已结束"降级路径调用——不清空
    /// 当前会话,只在上面写错误文案,让用户看到(现有 `acceptance_reject`
    /// 的降级路径)。
    pub fn set_error(&mut self, e: String) {
        if let Some(session) = &mut self.session {
            session.error = Some(e);
        }
    }
}

/// 意见框真 `text_input` 的 `widget::Id`,供 `CaptureCommentFocus` 匹配
/// 真实焦点态、main.rs 程序化聚焦与键盘路由查询。
pub fn comment_field_id() -> Id {
    Id::new("acceptance-comment-box")
}

static COMMENT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)上一帧捕获到的意见框真 `text_input` 焦点态,同
/// `files::take_tree_edit_focused` 的桥接手法(含消费式复位,避免意见框
/// 不可见的帧卡死上一次 `true` 永久堵死终端键盘转发)。
pub fn take_comment_focused() -> bool {
    std::mem::replace(&mut *COMMENT_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把命中 `comment_field_id` 的真
/// `text_input` 是否持有 iced 焦点写进 `COMMENT_FOCUSED`。`traverse`
/// 必须调用传入闭包(见 [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureCommentFocus;
impl Operation<()> for CaptureCommentFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&comment_field_id()) {
            *COMMENT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 一次进行中的验收(现有 `AcceptanceView` 的搬家版本)。
pub struct AcceptanceSession {
    repo: PathBuf,
    source_tab_id: usize,
    goal: Option<Goal>,
    changes: Vec<FileChange>,
    checked: Vec<bool>,
    /// 当前展开了 diff 的文件下标(对应 `changes` 的下标)。
    expanded: HashSet<usize>,
    /// diff 懒加载缓存:未展开过或仍在加载中的文件不在这个 map 里。
    diffs: HashMap<usize, Result<String, String>>,
    comment: String,
    /// 意见框是否持有 iced 内部真实焦点,每帧由 `CaptureCommentFocus` 写入。
    comment_focused: bool,
    error: Option<String>,
    accepted_version: Option<u32>,
}

impl AcceptanceSession {
    /// 供内核 `Reject` 拦截处理读取——要往哪个来源会话写打回意见。
    pub fn source_tab_id(&self) -> usize {
        self.source_tab_id
    }

    /// 供内核 `Reject` 拦截处理读取——打回意见文本。
    pub fn comment(&self) -> &str {
        &self.comment
    }
}

/// 对应现有 8 个 `Acceptance*` 变体去前缀搬来,新增 `ToggleDiff`/
/// `DiffLoaded` 两条支撑 diff 手风琴。`Open`/`Reject` 内核拦截,不进
/// `update`(见下)。
#[derive(Debug, Clone)]
pub enum Message {
    Open(usize),
    Loaded(i64, PathBuf, usize, Option<Goal>, Vec<FileChange>),
    Toggle(usize),
    ToggleDiff(usize),
    DiffLoaded(i64, usize, Result<String, String>),
    /// 意见框草稿变化(iced `text_input::on_input`,每次给全量当前字符串)。
    CommentInput(String),
    /// 意见框被右键:内核拦截,不进 `update`——转发成顶层
    /// `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    Accept,
    Reject,
    Done(i64, Result<u32, String>),
}

/// 处理 `Open`/`Reject` 之外的全部消息。内核在到达这里之前已经拦截了
/// 这两条(需要终端会话域能力),它们传进来会 `unreachable!`(同 Files
/// 试点 `CopyPath` 的处理方式)。
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Open(..) => unreachable!("由内核拦截处理,见 Message::Open 文档"),
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 acceptance::Message::TextInputMenuOpen 文档")
        }
        Message::Loaded(_, repo, source_tab_id, goal, changes) => {
            let n = goal.as_ref().map(|g| g.criteria.len()).unwrap_or(0);
            ws_state.session = Some(AcceptanceSession {
                repo,
                source_tab_id,
                goal,
                changes,
                checked: vec![false; n],
                expanded: HashSet::new(),
                diffs: HashMap::new(),
                comment: String::new(),
                comment_focused: false,
                error: None,
                accepted_version: None,
            });
        }
        Message::Toggle(i) => {
            if let Some(session) = &mut ws_state.session
                && let Some(c) = session.checked.get_mut(i)
            {
                *c = !*c;
            }
        }
        Message::ToggleDiff(i) => {
            let Some(session) = &mut ws_state.session else {
                return;
            };
            if !session.expanded.insert(i) {
                // 已经展开过 → 这次是收起。
                session.expanded.remove(&i);
                return;
            }
            if session.diffs.contains_key(&i) {
                return; // 已有缓存,不重新加载。
            }
            let Some(fc) = session.changes.get(i) else {
                return;
            };
            let repo = session.repo.clone();
            let path = fc.path.clone();
            handle.spawn(async move {
                let result =
                    tokio::task::spawn_blocking(move || crate::delivery::file_diff(&repo, &path))
                        .await
                        .unwrap_or_else(|e| Err(format!("任务失败: {e}")));
                emit(Message::DiffLoaded(project_id, i, result));
            });
        }
        Message::DiffLoaded(_, i, result) => {
            if let Some(session) = &mut ws_state.session {
                session.diffs.insert(i, result);
            }
        }
        Message::CommentInput(s) => {
            if let Some(session) = &mut ws_state.session {
                session.comment = s;
            }
        }
        Message::Accept => {
            let Some(session) = &mut ws_state.session else {
                return;
            };
            session.error = None;
            let repo = session.repo.clone();
            let goal_title = session
                .goal
                .as_ref()
                .map(|g| g.title.clone())
                .unwrap_or_default();
            let checked: Vec<String> = session
                .goal
                .as_ref()
                .map(|g| {
                    g.criteria
                        .iter()
                        .zip(&session.checked)
                        .filter(|(_, c)| **c)
                        .map(|(s, _)| s.clone())
                        .collect()
                })
                .unwrap_or_default();
            let comment = session.comment.clone();
            let client = client.clone();
            handle.spawn(async move {
                let repo2 = repo.clone();
                let accepted =
                    tokio::task::spawn_blocking(move || crate::delivery::accept(&repo2)).await;
                let result = match accepted {
                    Ok(Ok(n)) => {
                        let ref_name = format!("{}{n}", crate::delivery::ACCEPTED_REF_PREFIX);
                        let ts_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        if let Err(e) = client
                            .record_acceptance(
                                &repo.to_string_lossy(),
                                &goal_title,
                                &checked,
                                "accepted",
                                &comment,
                                &ref_name,
                                ts_ms,
                            )
                            .await
                        {
                            Err(format!("已沉淀 v{n},但记录落库失败: {e}"))
                        } else {
                            Ok(n)
                        }
                    }
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(e) => Err(format!("任务失败: {e}")),
                };
                emit(Message::Done(project_id, result));
            });
        }
        Message::Reject => unreachable!("由内核拦截处理,见 Message::Reject 文档"),
        Message::Done(_, result) => {
            if let Some(session) = &mut ws_state.session {
                match result {
                    Ok(n) => session.accepted_version = Some(n),
                    Err(e) => session.error = Some(e),
                }
            }
        }
    }
}

/// 内核在 `Open` 拦截处理里调用(已经清空了 `SessionTab.delivery_pending`、
/// 算好了 `cwd`,这两步是终端会话域操作,不在这个函数里做)。异步读
/// `delivery::repo_root`/`goal::parse_goal`/`delivery::changes`,完成后
/// `emit(Loaded(..))`。现有 `Message::AcceptanceOpen` 处理器里
/// `io.handle.spawn` 那部分逻辑的搬家版本。
pub fn spawn_open(
    project_id: i64,
    tab_id: usize,
    cwd: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let loaded = tokio::task::spawn_blocking(move || {
            let repo = crate::delivery::repo_root(&cwd)?;
            let goal = std::fs::read_to_string(crate::goal::goal_path(&repo))
                .ok()
                .and_then(|md| crate::goal::parse_goal(&md));
            let changes = crate::delivery::changes(&repo);
            Some((repo, goal, changes))
        })
        .await
        .ok()
        .flatten();
        if let Some((repo, goal, changes)) = loaded {
            emit(Message::Loaded(project_id, repo, tab_id, goal, changes));
        }
    });
}

/// 面板主入口。`session` 为 `None` 时是空态(还没有进行中的验收);有
/// `session` 且 `accepted_version.is_some()` 时只显示"已沉淀"提示(现有
/// `acceptance_content` 提前 return 那部分逻辑)。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(session) = ws_state.session() else {
        return container(
            text("没有待验收的交付——完成一轮 agent 会话后,点这个图标就能看到")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim),
        )
        .width(width)
        .height(Length::Fill)
        .padding(20)
        .into();
    };

    let mut content = column![].spacing(12).padding(14);

    if let Some(n) = session.accepted_version {
        content = content.push(
            text(format!("✓ 已沉淀 v{n}"))
                .size(byteui::theme::font::title())
                .color(byteui::theme::color::current().gold),
        );
        return container(content).width(width).height(Length::Fill).into();
    }

    match &session.goal {
        Some(g) => {
            content = content.push(
                text(g.title.clone())
                    .size(byteui::theme::font::title())
                    .color(byteui::theme::color::current().cream),
            );
            for (i, c) in g.criteria.iter().enumerate() {
                let checked = session.checked.get(i).copied().unwrap_or(false);
                content = content.push(
                    button(
                        text(format!("{} {c}", if checked { "✓" } else { "○" }))
                            .size(byteui::theme::font::body())
                            .color(if checked {
                                byteui::theme::color::current().gold
                            } else {
                                byteui::theme::color::current().body
                            }),
                    )
                    .on_press(Message::Toggle(i))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: byteui::theme::color::current().body,
                        ..button::Style::default()
                    }),
                );
            }
        }
        None => {
            content = content.push(
                text("未定标——先在仓库写 .dozer/goal.md（首行目标,\n- [ ] 列表为标准）")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            );
        }
    }

    content = content.push(
        text("变更文件")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    );
    for (i, fc) in session.changes.iter().enumerate() {
        let line = match (fc.added, fc.removed) {
            (Some(a), Some(r)) => format!("{}  +{a} −{r}", fc.path),
            _ => format!("{}  (新)", fc.path),
        };
        content = content.push(
            button(
                text(line)
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cyan),
            )
            .on_press(Message::ToggleDiff(i))
            .width(Length::Fill)
            .style(|_t, _s| button::Style {
                background: None,
                text_color: byteui::theme::color::current().cyan,
                ..button::Style::default()
            }),
        );
        if session.expanded.contains(&i) {
            content = content.push(diff_view(session.diffs.get(&i)));
        }
    }

    let editing = session.comment_focused;
    content = content.push(
        container(byteui::interaction::context_menu::wrap(
            byteui::form::input_text::view(
                "验收意见…（打回时注回会话）",
                &session.comment,
                false,
                Some(comment_field_id()),
                false,
                None,
                true,
                Message::CommentInput,
            ),
            Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                id: comment_field_id(),
                secure: false,
            })),
        ))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().term_bg.into()),
            border: Border {
                color: if editing {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().border
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        }),
    );

    content = content.push(
        row![
            button(
                text("通过·沉淀")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().bg)
            )
            .on_press(Message::Accept)
            .style(|_t, _s| button::Style {
                background: Some(byteui::theme::color::current().gold.into()),
                text_color: byteui::theme::color::current().bg,
                border: Border {
                    color: byteui::theme::color::current().gold,
                    width: 1.0,
                    radius: 2.0.into()
                },
                ..button::Style::default()
            }),
            button(
                text("打回并注回")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().red)
            )
            .on_press(Message::Reject)
            .style(|_t, _s| button::Style {
                background: None,
                text_color: byteui::theme::color::current().red,
                border: Border {
                    color: byteui::theme::color::current().red,
                    width: 1.0,
                    radius: 2.0.into()
                },
                ..button::Style::default()
            }),
        ]
        .spacing(8),
    );

    if let Some(err) = &session.error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: None,
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 手风琴展开的 diff 内容:`Ok(patch)` 交给共享的逐行染色渲染
/// (`diff_render::colored_diff_lines`,`+`/`-`/上下文三色),`Err(e)` 显示红字,
/// `None`(还没加载完)显示"加载中…"。
fn diff_view<'a>(
    diff: Option<&'a Result<String, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match diff {
        None => text("加载中…")
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim)
            .into(),
        Some(Err(e)) => text(format!("⚠ {e}"))
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().red)
            .into(),
        Some(Ok(patch)) => crate::diff_render::colored_diff_lines(patch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> Client {
        Client::new(std::path::PathBuf::from(
            "/tmp/dozer-acceptance-test-nonexistent.sock",
        ))
    }

    fn sample_session() -> AcceptanceSession {
        AcceptanceSession {
            repo: PathBuf::from("/tmp/repo"),
            source_tab_id: 0,
            goal: None,
            changes: vec![
                FileChange {
                    path: "a.rs".into(),
                    added: Some(1),
                    removed: None,
                },
                FileChange {
                    path: "b.rs".into(),
                    added: Some(2),
                    removed: Some(1),
                },
            ],
            checked: vec![],
            expanded: HashSet::new(),
            diffs: HashMap::new(),
            comment: String::new(),
            comment_focused: false,
            error: None,
            accepted_version: None,
        }
    }

    fn ws_with_session() -> WorkspaceState {
        let mut ws_state = WorkspaceState::default();
        ws_state.set_session_for_test(sample_session());
        ws_state
    }

    #[tokio::test]
    async fn toggle_flips_checked() {
        let mut ws_state = WorkspaceState::default();
        ws_state.set_session_for_test(AcceptanceSession {
            checked: vec![false, false],
            ..sample_session()
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::Toggle(1),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.session().unwrap().checked, vec![false, true]);
    }

    #[tokio::test]
    async fn toggle_diff_expands_then_collapses_without_reload() {
        let mut ws_state = ws_with_session();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::ToggleDiff(0),
            1,
            &test_client(),
            &handle,
            |_| panic!("异步任务在测试里不需要真正跑,这里只断言同步状态"),
        );
        // 注意:上面这行会真的 spawn 一个读 /tmp/repo 的异步任务(大概率
        // 失败,emit 不会被调用,因为 /tmp/repo 不是真实仓库)——测试只关心
        // 同步部分:`expanded` 立刻加入 0。
        assert!(ws_state.session().unwrap().expanded.contains(&0));
        update(
            &mut ws_state,
            Message::ToggleDiff(0),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert!(
            !ws_state.session().unwrap().expanded.contains(&0),
            "第二次点收起"
        );
    }

    #[tokio::test]
    async fn diff_loaded_writes_cache() {
        let mut ws_state = ws_with_session();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::DiffLoaded(1, 0, Ok("+line".to_string())),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.session().unwrap().diffs.get(&0),
            Some(Ok("+line".to_string())).as_ref()
        );
    }

    #[tokio::test]
    async fn done_ok_sets_accepted_version_err_sets_error() {
        let mut ws_state = ws_with_session();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::Done(1, Ok(3)),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.session().unwrap().accepted_version, Some(3));
        let mut ws_state = ws_with_session();
        update(
            &mut ws_state,
            Message::Done(1, Err("boom".to_string())),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.session().unwrap().error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    #[should_panic(expected = "由内核拦截处理")]
    async fn open_reaching_update_panics() {
        let mut ws_state = WorkspaceState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::Open(0),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
    }

    #[tokio::test]
    #[should_panic(expected = "由内核拦截处理")]
    async fn reject_reaching_update_panics() {
        let mut ws_state = WorkspaceState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::Reject,
            1,
            &test_client(),
            &handle,
            |_| {},
        );
    }
}
