//! 文件树右键"搜索"弹窗：作用域(目录子树/单文件)内的全文内容搜索。瞬态弹窗，
//! 不挂左侧图标栏，满窗 SCRIM+卡片形制(此前"文件预览可写编辑"弹窗同款)。
//! 不做搜索历史/
//! 索引/后台预扫描，见
//! `docs/superpowers/specs/2026-08-12-tree-search-in-context-menu-design.md`。

use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Color, Element, Length, Rectangle};
use iced_widget::{button, column, container, row, scrollable, text};
use std::path::{Path, PathBuf};

/// 搜索作用域：右键目标。
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    /// 目录整棵子树。
    Dir(PathBuf),
    /// 单文件。
    File(PathBuf),
}

/// 一条命中。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line_no: u64,
    pub line_text: String,
}

/// 收集单条命中行的 `Sink`：拷出路径/行号/命中行文本。
struct HitSink {
    path: PathBuf,
    hits: Vec<SearchHit>,
}

impl Sink for HitSink {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        // `mat.bytes()` 是整行(含尾随换行的原始字节),直接 lossy 转成展示文本。
        self.hits.push(SearchHit {
            path: self.path.clone(),
            line_no: mat.line_number().unwrap_or(0),
            line_text: String::from_utf8_lossy(mat.bytes()).into_owned(),
        });
        Ok(true)
    }
}

/// 已命中文件路径 → 该文件的命中行,各 worker 线程自己攒,最后合并。
type HitMap = std::collections::BTreeMap<String, Vec<SearchHit>>;

/// 每个 worker 线程一个的 visitor:持有自己的 `Searcher`(不跨线程共享,
/// 复用其内部缓冲),把本线程扫到的命中写进共享的 `HitMap`(加锁只发生在
/// "一个文件扫完、确实有命中"这一次,不在逐行热路径上)。二进制文件靠
/// `Searcher` 默认的 NUL 检测早退,不在此显式处理。
struct HitVisitor<'a> {
    matcher: &'a grep_regex::RegexMatcher,
    shared: &'a std::sync::Mutex<HitMap>,
    searcher: Searcher,
}

impl<'a> ignore::ParallelVisitor for HitVisitor<'a> {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        let de = match entry {
            Ok(de) => de,
            Err(e) => {
                tracing::warn!("搜索遍历跳过错误条目: {e}");
                return ignore::WalkState::Continue;
            }
        };
        // 只处理常规文件;目录由 walker 自己继续下钻。
        if !de.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            return ignore::WalkState::Continue;
        }
        let path = de.path().to_path_buf();
        let mut sink = HitSink {
            path: path.clone(),
            hits: Vec::new(),
        };
        if self
            .searcher
            .search_path(self.matcher, &path, &mut sink)
            .is_err()
        {
            return ignore::WalkState::Continue; // 读不了的(权限/消失)跳过,不 panic
        }
        if !sink.hits.is_empty() {
            self.shared
                .lock()
                .unwrap()
                .insert(path.display().to_string(), sink.hits);
        }
        ignore::WalkState::Continue
    }
}

/// 为每个 worker 线程构造 `HitVisitor`。builder 被 `WalkParallel::visit`
/// 在每个线程启动时调用一次。
struct HitVisitorBuilder<'a> {
    matcher: &'a grep_regex::RegexMatcher,
    shared: &'a std::sync::Mutex<HitMap>,
}

impl<'a> ignore::ParallelVisitorBuilder<'a> for HitVisitorBuilder<'a> {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 'a> {
        Box::new(HitVisitor {
            matcher: self.matcher,
            shared: self.shared,
            searcher: SearcherBuilder::new().line_number(true).build(),
        })
    }
}

/// 在 scope 内做字面子串(大小写不敏感)搜索，按文件分组返回，组内按行号升序。
/// 返回 `Vec<(绝对路径字符串, 该文件的命中)>`，键恒为整段绝对路径，相对展示交给
/// view 层用 `project_root` 换算。空查询直接返回空结果，不发起搜索。
///
/// 目录作用域用 `ignore::WalkBuilder::build_parallel` 递归(尊重 `.gitignore`
/// 与隐藏文件;`require_git(false)` 让目录即便不在 git 仓库内也应用根目录的
/// `.gitignore`，同 ripgrep 的 `--no-require-git` 口径)，多核并行扫——
/// 此前是"先串行 WalkBuilder 收集全部路径、再逐个串行 search_path"，大仓库下
/// 只有单核忙。文件作用域仍只搜那一个文件。二进制/非 UTF-8 行用 lossy
/// 转换，不 panic。
pub fn search_scope(scope: &Scope, query: &str) -> Result<Vec<(String, Vec<SearchHit>)>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(true)
        .build(query)
        .map_err(|e| format!("搜索模式无效: {e}"))?;

    // 文件作用域:单文件,无需并行遍历,直接扫。
    if let Scope::File(p) = scope {
        let mut searcher = SearcherBuilder::new().line_number(true).build();
        let mut sink = HitSink {
            path: p.clone(),
            hits: Vec::new(),
        };
        if searcher.search_path(&matcher, p, &mut sink).is_ok() && !sink.hits.is_empty() {
            return Ok(vec![(p.display().to_string(), sink.hits)]);
        }
        return Ok(Vec::new());
    }

    // 目录作用域:并行遍历 + 每线程各自搜索。
    let shared: std::sync::Mutex<HitMap> = std::sync::Mutex::new(HitMap::new());
    let Scope::Dir(dir) = scope else {
        unreachable!()
    };
    let mut builder = ignore::WalkBuilder::new(dir);
    builder.require_git(false);
    let mut visitor_builder = HitVisitorBuilder {
        matcher: &matcher,
        shared: &shared,
    };
    builder.build_parallel().visit(&mut visitor_builder);

    let by_file = shared.into_inner().unwrap();
    Ok(by_file.into_iter().collect())
}

/// 挂每个 `Workspace` 的搜索弹窗状态。瞬态、不持久化;各自 Workspace 独立。
#[derive(Debug, Default)]
pub struct WorkspaceState {
    open: bool,
    scope: Option<Scope>,
    query: String,
    /// 查询框是否持有 iced 内部真实焦点,每帧由 `CaptureQueryFocus` 写入。
    query_focused: bool,
    /// 一次性标记:弹窗打开(`open()`)时置真——不再等用户点一下查询框才
    /// 进编辑态,打开即自动聚焦,省掉这次迁移前就存在的多余一次点击
    /// (原版 `QueryEditing(true)` 需要额外点击触发,见本计划 Architecture
    /// 一节的说明)。
    query_focus_pending: bool,
    running: bool,
    results: Vec<(String, Vec<SearchHit>)>,
    has_searched: bool,
    error: Option<String>,
}

impl WorkspaceState {
    pub fn is_open(&self) -> bool {
        self.open
    }
    /// 查询框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn query_focused(&self) -> bool {
        self.query_focused
    }

    /// 每帧渲染循环读走 `CaptureQueryFocus` 查到的真实焦点态后写进来。
    pub fn set_query_focused(&mut self, focused: bool) {
        self.query_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记。
    pub fn take_query_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.query_focus_pending)
    }
}

/// 查询框真 `text_input` 的 `widget::Id`,供 `CaptureQueryFocus` 匹配
/// 真实焦点态、main.rs 程序化聚焦与键盘路由查询。
pub fn query_field_id() -> Id {
    Id::new("search-query-box")
}

static QUERY_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)上一帧捕获到的查询框真 `text_input` 焦点态,同
/// `extensions::files::take_search_focused` 的消费式复位手法,避免查询框
/// 不可见的帧卡死上一次 `true` 永久堵死终端键盘转发。
pub fn take_query_focused() -> bool {
    std::mem::replace(&mut *QUERY_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把命中 `query_field_id` 的真
/// `text_input` 是否持有 iced 焦点写进 `QUERY_FOCUSED`。`traverse`
/// 必须调用传入闭包(见 [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureQueryFocus;
impl Operation<()> for CaptureQueryFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&query_field_id()) {
            *QUERY_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 搜索弹窗的消息:均由内核包装转发(见 `workspace.rs` 内核的 `Message::Search`)。
#[derive(Debug, Clone)]
pub enum Message {
    SearchOpen(Scope),
    SearchClose,
    /// 查询框草稿变化(iced `text_input::on_input`)。
    QueryInput(String),
    /// 回车 / 点"搜索"→ 启动异步搜索。
    QuerySubmit,
    /// 异步结果回灌。带 `project_id`,理由同数据库/SSH面板(异步结果不能假设
    /// 聚焦项目没变)。
    SearchResults(i64, Result<Vec<(String, Vec<SearchHit>)>, String>),
    /// 点击命中 → 内核拦截映射为 `PreviewOpenPath`(本模块只声明,不进 update)。
    Pick(SearchHit),
    /// 查询框被右键:内核拦截,不进 `update`——转发成顶层
    /// `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
}

/// 打开弹窗并预填作用域。keyword 草稿保留上次(同项目内复用),结果清空。
pub fn open(ws: &mut WorkspaceState, scope: Scope) {
    ws.open = true;
    ws.scope = Some(scope);
    ws.running = false;
    ws.has_searched = false;
    ws.results.clear();
    ws.error = None;
    // 弹窗一打开就把查询框标记为"待聚焦",main.rs 渲染循环据此程序化聚焦
    // ——省掉迁移前"开了弹窗还要点一下查询框才能打字"的多余一步。
    ws.query_focus_pending = true;
}

pub fn update(
    ws: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::SearchOpen(scope) => open(ws, scope),
        Message::SearchClose => ws.open = false,
        Message::QueryInput(s) => ws.query = s,
        Message::QuerySubmit => {
            let Some(scope) = ws.scope.clone() else {
                return;
            };
            let query = ws.query.clone();
            if query.trim().is_empty() {
                return;
            }
            ws.running = true;
            ws.error = None;
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || search_scope(&scope, &query))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::SearchResults(project_id, result));
            });
        }
        Message::SearchResults(_project_id, result) => {
            ws.running = false;
            match result {
                Ok(results) => {
                    ws.results = results;
                    ws.has_searched = true;
                }
                Err(e) => ws.error = Some(e),
            }
        }
        // `Pick` 由内核拦截映射为预览打开,不进这里。
        Message::Pick(_) => {}
        // `TextInputMenuOpen` 由内核拦截映射为右键菜单,不进这里。
        Message::TextInputMenuOpen(_) => {}
    }
}

/// 查询框(真正的 iced `text_input`,经由 `byteui::form::input_text::view` 渲染)。
/// 弹窗一打开就自动聚焦(见 `open()` 置位的一次性聚焦标记)。
fn query_box(
    ws: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = ws.query_focused() || ws.running;
    byteui::interaction::context_menu::wrap(
        byteui::form::input_text::view(
            "搜索内容…",
            &ws.query,
            false,
            Some(query_field_id()),
            active,
            Some(Message::QuerySubmit),
            false,
            Message::QueryInput,
        ),
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: query_field_id(),
            secure: false,
        })),
    )
}

/// 把绝对路径裁成相对项目根的展示路径;`project_root` 取不到(理论上只有没开
/// 项目却打开弹窗这种到不了的状态)时退回显示原路径。
fn rel_to_root<'a>(path: &'a Path, project_root: Option<&'a Path>) -> &'a Path {
    match project_root {
        Some(root) => path.strip_prefix(root).unwrap_or(path),
        None => path,
    }
}

/// 结果列表:文件分组标题 + 组内命中行,点击命中行发 `Message::Pick`。
fn results_list<'a>(
    ws: &'a WorkspaceState,
    project_root: Option<&'a Path>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(4);
    for (path_str, hits) in &ws.results {
        let rel = rel_to_root(Path::new(path_str), project_root);
        col = col.push(
            text(rel.display().to_string())
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
        );
        for hit in hits {
            col = col.push(
                button(
                    text(format!(":{}: {}", hit.line_no, hit.line_text))
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().cream),
                )
                .on_press(Message::Pick(hit.clone()))
                .width(Length::Fill)
                .padding([4, 8])
                .style(|_t: &iced_widget::Theme, s| {
                    let bg = match s {
                        button::Status::Hovered | button::Status::Pressed => {
                            byteui::theme::color::current().tab_hover
                        }
                        _ => byteui::theme::color::current().bg,
                    };
                    button::Style {
                        background: Some(bg.into()),
                        text_color: byteui::theme::color::current().cream,
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 4.0.into(),
                        },
                        ..button::Style::default()
                    }
                }),
            );
        }
    }
    scrollable(container(col).padding(8))
        .width(Length::Fill)
        .height(Length::Fixed(360.0))
        .into()
}

/// 搜索弹窗的卡片本体(标题行 + 查询行 + 结果列表),无遮罩、无外层定位——
/// 这次拆分是为了让独立 overlay 窗口(`platform/search_overlay.rs`)能直接
/// 复用同一份视图逻辑,只是换一个宿主(独立窗口取代 `App::view()` 的
/// `stack!` 层)。`height(Length::Fill)`:调用方现在总是给一块已经量好的
/// 画布(要么是旧路径里 `container(dialog)` 分配的区域,要么是新路径里
/// 整扇 overlay 窗口的画布),不需要 `Length::Shrink` 那种"在更大画布里
/// 收缩适配内容"的语义。
pub(crate) fn search_card<'a>(
    ws: &'a WorkspaceState,
    project_root: Option<&'a Path>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.open {
        return column![].into();
    }

    let scope_label = match &ws.scope {
        Some(Scope::Dir(p)) => {
            let rel = rel_to_root(p, project_root);
            format!("目录: {}", rel.display())
        }
        Some(Scope::File(p)) => {
            let rel = rel_to_root(p, project_root);
            format!("文件: {}", rel.display())
        }
        None => String::new(),
    };

    let title_row = row![
        text(scope_label)
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)
        )
        .on_press(Message::SearchClose)
        .padding(0)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..button::Style::default()
        }),
    ]
    .align_y(iced_widget::core::Alignment::Center);

    let submit_btn = button(
        text(if ws.running { "搜索中…" } else { "搜索" })
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().gold),
    )
    .on_press(Message::QuerySubmit)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().gold,
    ));

    let query_row = row![query_box(ws), submit_btn]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center);

    let mut body = column![title_row, query_row]
        .width(Length::Fill)
        .spacing(8)
        .height(Length::Fill);

    if ws.running {
        body = body.push(byteui::feedback::math_curve::loading_hint(
            byteui::feedback::math_curve::Curve::RoseThree,
            "搜索中…",
            48.0,
        ));
    } else if ws.has_searched && ws.results.is_empty() && ws.error.is_none() {
        body = body.push(
            text("无匹配")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else if !ws.results.is_empty() {
        body = body.push(results_list(ws, project_root));
    }
    if let Some(err) = &ws.error {
        body = body.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    container(body.padding(16))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::dialog::card_style)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(root: &Path, rel: &str, content: &str) -> PathBuf {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn empty_query_yields_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let f = place(dir.path(), "a.txt", "needle\n");
        let hits = search_scope(&Scope::File(f), "  ").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn single_file_matches_with_line_number() {
        let dir = tempfile::tempdir().unwrap();
        let f = place(dir.path(), "a.txt", "first line\nneedle here\nthird line\n");
        let hits = search_scope(&Scope::File(f), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.len(), 1);
        assert_eq!(hits[0].1[0].line_no, 2);
        assert!(hits[0].1[0].line_text.contains("needle here"));
    }

    #[test]
    fn search_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let f = place(dir.path(), "a.txt", "HELLO world\n");
        let hits = search_scope(&Scope::File(f), "hello").unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn dir_scope_recurses_and_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        place(dir.path(), ".gitignore", "ignored.txt\n");
        place(dir.path(), "keep.txt", "needle keep\n");
        place(dir.path(), "ignored.txt", "needle ignored\n");
        place(dir.path(), "nested/sub.txt", "needle nested\n");
        let hits = search_scope(&Scope::Dir(dir.path().to_path_buf()), "needle").unwrap();
        // 只命中 keep.txt 与 nested/sub.txt,忽略文件不参与。
        let keys: Vec<&str> = hits.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.iter().any(|k| k.ends_with("keep.txt")));
        assert!(keys.iter().any(|k| k.ends_with("nested/sub.txt")));
        assert!(!keys.iter().any(|k| k.ends_with("ignored.txt")));
    }

    #[test]
    fn dir_scope_parallel_groups_by_file_and_orders_lines() {
        // 并行遍历下:结果仍按文件路径(BTreeMap 键)升序分组,组内按行号升序,
        // 且同一文件的多条命中不丢、不漏。
        let dir = tempfile::tempdir().unwrap();
        place(dir.path(), "a.txt", "needle one\nplain\nneedle two\n");
        place(dir.path(), "b.txt", "no hit here\n");
        place(dir.path(), "c.txt", "needle three\n");
        let hits = search_scope(&Scope::Dir(dir.path().to_path_buf()), "needle").unwrap();
        // b.txt 无命中,不出现;命中文件按路径升序。
        assert_eq!(hits.len(), 2, "只应有 a.txt / c.txt 两组,得 {hits:?}");
        assert!(hits[0].0.ends_with("a.txt"));
        assert!(hits[1].0.ends_with("c.txt"));
        // a.txt 两条命中,行号升序。
        assert_eq!(hits[0].1.len(), 2);
        assert!(hits[0].1[0].line_no < hits[0].1[1].line_no);
    }

    #[test]
    fn binary_utf8_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let f = place(dir.path(), "bin.dat", "needle text\n");
        std::fs::write(&f, [0xFF, 0x21, b'\n']).unwrap(); // 非法 UTF-8 + 非 ASCII 第一字节
        let hits = search_scope(&Scope::File(f), "anything").unwrap();
        // 不 panic 即可;二进制内容不命中查询词,结果可为空。
        let _ = hits;
    }

    #[test]
    fn file_scope_does_not_recurse() {
        let dir = tempfile::tempdir().unwrap();
        let f = place(dir.path(), "a.txt", "needle\n");
        // 在 scope 外另外造一个会命中的文件,证明文件作用域不扫它。
        place(dir.path(), "b.txt", "needle b\n");
        let hits = search_scope(&Scope::File(f), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].0.ends_with("a.txt"));
    }

    #[test]
    fn open_sets_query_focus_pending() {
        let mut ws = WorkspaceState::default();
        assert!(!ws.is_open());
        let scope = Scope::Dir("/tmp".into());
        open(&mut ws, scope);
        assert!(ws.is_open());
        assert!(
            ws.query_focus_pending,
            "弹窗一打开就应置查询框的待聚焦标记(打开即自动聚焦)"
        );
    }

    #[test]
    fn query_input_messages_replace_query() {
        let mut ws = WorkspaceState::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::QueryInput("hello".into()),
            1,
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.query, "hello");
        update(
            &mut ws,
            Message::QueryInput("world".into()),
            1,
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.query, "world", "每次给全量,不是追加");
    }
}
