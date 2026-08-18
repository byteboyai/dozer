//! 文件树右键"搜索"弹窗：作用域(目录子树/单文件)内的全文内容搜索。瞬态弹窗，
//! 不挂 `LeftView`/左侧图标栏，形制参考文件编辑弹层 `edit_modal`。不做搜索历史/
//! 索引/后台预扫描，见
//! `docs/superpowers/specs/2026-08-12-tree-search-in-context-menu-design.md`。

use crate::theme;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{MouseArea, button, column, container, row, scrollable, stack, text};
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

/// 在 scope 内做字面子串(大小写不敏感)搜索，按文件分组返回，组内按行号升序。
/// 返回 `Vec<(绝对路径字符串, 该文件的命中)>`，键恒为整段绝对路径，相对展示交给
/// view 层用 `project_root` 换算。空查询直接返回空结果，不发起搜索。
///
/// 目录作用域用 `ignore::WalkBuilder` 递归(尊重 `.gitignore` 与隐藏文件;
/// `require_git(false)` 让目录即便不在 git 仓库内也应用根目录的 `.gitignore`,
/// 同 ripgrep 的 `--no-require-git` 口径);
/// 文件作用域只搜那一个文件。二进制/非 UTF-8 行用 lossy 转换，不 panic。
pub fn search_scope(scope: &Scope, query: &str) -> Result<Vec<(String, Vec<SearchHit>)>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(true)
        .build(query)
        .map_err(|e| format!("搜索模式无效: {e}"))?;

    let mut searcher = SearcherBuilder::new().line_number(true).build();

    // 作用域内所有待搜文件(绝对路径)。
    let mut files: Vec<PathBuf> = Vec::new();
    match scope {
        Scope::File(p) => files.push(p.clone()),
        Scope::Dir(p) => {
            for entry in ignore::WalkBuilder::new(p).require_git(false).build() {
                match entry {
                    Ok(de) if de.file_type().map(|ft| ft.is_file()).unwrap_or(false) => {
                        files.push(de.path().to_path_buf());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("搜索遍历跳过错误条目: {e}");
                    }
                }
            }
        }
    }

    // 已发现命中文件的路径 → 其结果组(用绝对路径字符串作键)。
    let mut by_file: std::collections::BTreeMap<String, Vec<SearchHit>> =
        std::collections::BTreeMap::new();
    for path in files {
        let mut sink = HitSink {
            path: path.clone(),
            hits: Vec::new(),
        };
        if searcher.search_path(&matcher, &path, &mut sink).is_err() {
            continue; // 读不了的(权限/消失)跳过,不 panic
        }
        if !sink.hits.is_empty() {
            by_file.insert(path.display().to_string(), sink.hits);
        }
    }
    Ok(by_file.into_iter().collect())
}

/// 挂每个 `Workspace` 的搜索弹窗状态。瞬态、不持久化;各自 Workspace 独立。
#[derive(Debug, Default)]
pub struct WorkspaceState {
    open: bool,
    scope: Option<Scope>,
    query: String,
    query_editing: bool,
    running: bool,
    results: Vec<(String, Vec<SearchHit>)>,
    has_searched: bool,
    error: Option<String>,
}

impl WorkspaceState {
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn query_editing(&self) -> bool {
        self.query_editing
    }
}

/// 搜索弹窗的消息:均由内核包装转发(见 `workspace.rs` 内核的 `Message::Search`)。
#[derive(Debug, Clone)]
pub enum Message {
    SearchOpen(Scope),
    SearchClose,
    /// 自绘查询框的按键事件(main.rs 拦截层在编辑态下路由进来,同
    /// `files::Message::SearchEvent` 的口径)。
    QueryEvent(crate::workspace::AddrEvent),
    /// 自绘输入框进入/离开编辑态(main.rs 据此决定是否把按键路由成
    /// `QueryEvent` 而不是下钻到 PTY)。
    QueryEditing(bool),
    /// 回车 / 点"搜索"→ 启动异步搜索。
    QuerySubmit,
    /// 异步结果回灌。带 `project_id`,理由同数据库/SSH面板(异步结果不能假设
    /// 聚焦项目没变)。
    SearchResults(i64, Result<Vec<(String, Vec<SearchHit>)>, String>),
    /// 点击命中 → 内核拦截映射为 `PreviewOpenPath`(本模块只声明,不进 update)。
    Pick(SearchHit),
}

/// 打开弹窗并预填作用域。keyword 草稿保留上次(同项目内复用),结果清空。
pub fn open(ws: &mut WorkspaceState, scope: Scope) {
    ws.open = true;
    ws.scope = Some(scope);
    ws.running = false;
    ws.has_searched = false;
    ws.results.clear();
    ws.error = None;
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
        Message::QueryEvent(ev) => {
            // 只在查询框编辑态处理按键(点击盒子进入编辑态后,main.rs 才把
            // 按键路由成这个变体);未进入时收到属异常,直接忽略。
            if !ws.query_editing {
                return;
            }
            match ev {
                crate::workspace::AddrEvent::Text(s) => ws.query.push_str(&s),
                crate::workspace::AddrEvent::Backspace => {
                    ws.query.pop();
                }
                crate::workspace::AddrEvent::Cancel => ws.query_editing = false,
                crate::workspace::AddrEvent::Submit => {
                    ws.query_editing = false;
                    update(ws, Message::QuerySubmit, project_id, handle, emit);
                }
            }
        }
        Message::QueryEditing(b) => ws.query_editing = b,
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
        // `Pick` 由内核拦截映射为预览打开,不进这里;`QueryEditing` 由 view 的
        // 输入框 toggle 事件驱动(见 `search_modal`),不在 update 里落地。
        Message::Pick(_) => {}
    }
}

/// 自绘查询输入框(同 files 顶栏搜索框/树内行编辑):编辑态显示草稿 + "▏"光标,
/// 非编辑态显示已提交关键字或占位提示。点击进入编辑态发 `QueryEditing(true)`。
fn query_box(
    ws: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let body = if ws.query.is_empty() && !ws.query_editing {
        text("搜索内容…")
            .size(theme::font::body())
            .color(byteui::theme::color::current().dim)
    } else {
        let caret = if ws.query_editing { "▏" } else { "" };
        text(format!("{}{}", ws.query, caret))
            .size(theme::font::body())
            .color(byteui::theme::color::current().cream)
    };
    let active = ws.query_editing || ws.running;
    button(body)
        .on_press(Message::QueryEditing(true))
        .width(Length::Fill)
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: Border {
                color: if active {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().border
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        })
        .into()
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
                .size(theme::font::label())
                .color(byteui::theme::color::current().dim),
        );
        for hit in hits {
            col = col.push(
                button(
                    text(format!(":{}: {}", hit.line_no, hit.line_text))
                        .size(theme::font::body())
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

/// 搜索弹窗本体:形制照抄 `workspace.rs::edit_modal`——满载 `SCRIM` 遮罩 +
/// `CARD` 对话框。由 `App::view()` 顶层浮层链的 `stack!` 里调用;未打开时返回空元素。
pub fn search_modal<'a>(
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
            .size(theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(theme::font::subtitle())
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
            .size(theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::QuerySubmit)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: byteui::theme::color::current().cream,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let query_row = row![query_box(ws), submit_btn]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center);

    let mut body = column![title_row, query_row]
        .width(Length::Fill)
        .spacing(8)
        .height(Length::Shrink);

    if ws.has_searched && ws.results.is_empty() && ws.error.is_none() {
        body = body.push(
            text("无匹配")
                .size(theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else if !ws.results.is_empty() {
        body = body.push(results_list(ws, project_root));
    }
    if let Some(err) = &ws.error {
        body = body.push(
            text(format!("⚠ {err}"))
                .size(theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    let dialog = container(body.padding(16))
        .width(Length::Fixed(560.0))
        .height(Length::Shrink)
        .max_height(640.0)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

    // 全窗 `SCRIM` 遮罩做成可点击的目标:点在卡片**外**(遮罩上)即 `SearchClose`。
    // 卡片本体是上层 `stack!` 的兄弟元素(自适配宽高、垂直/水平居中),不盖住
    // 遮罩的点击——所以"点遮罩关闭"能成立(与只靠 ×/Esc 的 `edit_modal` 略不同,
    // 但更贴合 spec 验收"点遮罩都能关闭")。
    let scrim = MouseArea::new(
        container(column![])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().scrim.into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::SearchClose);

    stack![
        scrim,
        container(dialog)
            .padding(40.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
    ]
    .width(Length::Fill)
    .height(Length::Fill)
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
}
