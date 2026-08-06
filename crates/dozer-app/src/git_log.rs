// crates/dozer-app/src/git_log.rs
//! spike(2026-08-06): 验证第三方 Rust 库 `gleisbau`(git-graph 的布局引擎,
//! 拆出来的独立 crate)能否喂出可在 iced Canvas 里原生画出的提交图数据,
//! 探路"要不要在 Dozer 里做一个类似 VS Code Git Graph 的面板"。
//!
//! 只验证数据链路是否走得通,不追求 curve/fork 的像素级还原:每条 track
//! 画一根直线,commit 是线上的一个圆点,父子关系用直线连接(不是贝塞尔)。
//! 验证通过、决定转正时,再补动画/交互/性能优化。
use crate::theme;
use crate::workspace::Message;
use crate::workspace_font;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::alignment;
use iced_widget::core::{Color, Element, Font, Length, Pixels, Point, Rectangle, Vector};
use iced_widget::{column, container, row, scrollable, text};
use std::path::{Path, PathBuf};

/// 一次性拉多少个 commit——够看出分叉/合并的形状,又不至于让 revwalk +
/// 分支归属分析在大仓库上明显卡顿(spike 阶段没做分页/增量)。
const MAX_COMMITS: usize = 200;

const ROW_HEIGHT: f32 = 22.0;
const COL_WIDTH: f32 = 14.0;
const DOT_RADIUS: f32 = 3.5;
const LEFT_MARGIN: f32 = 12.0;
const TEXT_GAP: f32 = 12.0;
const LINE_WIDTH: f32 = 1.6;

/// 与主题色轮换配色的 track 调色板——不用 gleisbau 自带的 CSS 颜色名,
/// 省掉一个颜色名解析器,顺便让图和 ByteBoy2077 主题保持一致。
const TRACK_COLORS: [Color; 5] = [
    theme::CYAN,
    theme::GREEN,
    theme::GOLD,
    theme::PURPLE,
    theme::RED,
];

fn track_color(color_idx: usize) -> Color {
    TRACK_COLORS[color_idx % TRACK_COLORS.len()]
}

/// 一个 commit 在图上的位置与连线目标,构造完就是自持有数据(不挂
/// `git2::Repository`/`Commit` 的生命周期),可以直接存进 `App` 缓存。
pub struct CommitRow {
    column: usize,
    color_idx: usize,
    short_sha: String,
    summary: String,
    /// 父 commit 的 (row, column, color_idx),用于画连线;可能落在
    /// `MAX_COMMITS` 截断范围之外——那种父 commit 不出现在 `rows` 里,
    /// 此处已被过滤掉。
    parents: Vec<(usize, usize, usize)>,
}

pub struct GitLogSnapshot {
    repo_path: PathBuf,
    rows: Vec<CommitRow>,
    max_column: usize,
}

impl GitLogSnapshot {
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }
}

/// `Settings` 无内置构造函数——`BranchSettingsDef::simple()` 只给"分支命名
/// 分类"这半份(persistence/order/颜色分类的字符串正则源),真正的
/// `Settings`(含格式/字符集/合并模式等)得手工拼。选 `simple()` 而非
/// `git_flow()`:Dozer worktree 分支名随意,不套 gitflow 那套 main/develop/
/// feature 假设更稳妥(见调研备忘)。
fn default_settings() -> Result<gleisbau::settings::Settings, String> {
    use gleisbau::settings::{
        BranchOrder, BranchSettings, BranchSettingsDef, Characters, MergePatterns, Settings,
    };
    let branches = BranchSettings::from(BranchSettingsDef::simple()).map_err(|e| e.to_string())?;
    Ok(Settings {
        reverse_commit_order: false,
        debug: false,
        compact: false,
        colored: false,
        include_remote: true,
        format: gleisbau::print::format::CommitFormat::OneLine,
        wrapping: None,
        characters: Characters::thin(),
        branch_order: BranchOrder::ShortestFirst(true),
        branches,
        merge_patterns: MergePatterns::default(),
    })
}

/// 对 `repo_path` 跑一次 `gleisbau` 布局,产出可渲染快照。同步执行——
/// `MAX_COMMITS` 量级下 revwalk + 分支归属分析是毫秒级,spike 阶段不值得
/// 为此引入异步往返。
pub fn build(repo_path: &Path) -> Result<GitLogSnapshot, String> {
    let repository = gleisbau::get_repo(repo_path, false).map_err(|e| e.message().to_string())?;
    let settings = std::rc::Rc::new(default_settings()?);
    let graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(settings)
        .with_max_count(MAX_COMMITS)
        .build()?;

    let mut max_column = 0usize;
    let rows = graph
        .tracks
        .commits
        .iter()
        .map(|commit| {
            let b_idx = commit
                .branch_trace
                .ok_or_else(|| "commit 缺少 branch_trace".to_string())?;
            let column = graph
                .layout
                .track_visual(b_idx)
                .and_then(|v| v.column)
                .unwrap_or(0);
            max_column = max_column.max(column);
            let color_idx = b_idx.index();
            let git_commit = graph
                .commit(commit.oid)
                .map_err(|e| e.message().to_string())?;
            let full_sha = commit.oid.to_string();
            let short_sha = full_sha.chars().take(7).collect();
            let summary = git_commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let parents = commit
                .parents
                .iter()
                .filter_map(|poid| {
                    let p_idx = *graph.tracks.indices.get(poid)?;
                    let p_commit = graph.tracks.commits.get(p_idx)?;
                    let p_b_idx = p_commit.branch_trace?;
                    let p_column = graph
                        .layout
                        .track_visual(p_b_idx)
                        .and_then(|v| v.column)
                        .unwrap_or(0);
                    Some((p_idx, p_column, p_b_idx.index()))
                })
                .collect();
            Ok(CommitRow {
                column,
                color_idx,
                short_sha,
                summary,
                parents,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(GitLogSnapshot {
        repo_path: repo_path.to_path_buf(),
        rows,
        max_column,
    })
}

struct GitLogCanvas<'a> {
    snapshot: &'a GitLogSnapshot,
}

fn row_center(row: usize, column: usize) -> Point {
    Point::new(
        LEFT_MARGIN + column as f32 * COL_WIDTH,
        ROW_HEIGHT * 0.5 + row as f32 * ROW_HEIGHT,
    )
}

impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for GitLogCanvas<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_widget::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_widget::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let text_x = LEFT_MARGIN + (self.snapshot.max_column + 1) as f32 * COL_WIDTH + TEXT_GAP;

        // 先画连线,commit 圆点和文字盖在上面。
        for (row_idx, commit) in self.snapshot.rows.iter().enumerate() {
            let from = row_center(row_idx, commit.column);
            for &(p_row, p_col, p_color_idx) in &commit.parents {
                let to = row_center(p_row, p_col);
                let path = canvas::Path::line(from, to);
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_color(track_color(p_color_idx))
                        .with_width(LINE_WIDTH),
                );
            }
        }

        for (row_idx, commit) in self.snapshot.rows.iter().enumerate() {
            let center = row_center(row_idx, commit.column);
            let color = track_color(commit.color_idx);
            frame.fill(&canvas::Path::circle(center, DOT_RADIUS), color);

            frame.with_save(|frame| {
                frame.translate(Vector::new(
                    text_x,
                    row_idx as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5,
                ));
                frame.fill_text(canvas::Text {
                    content: format!("{}  {}", commit.short_sha, commit.summary),
                    position: Point::ORIGIN,
                    color: theme::CREAM,
                    size: Pixels(workspace_font::body() as f32),
                    align_y: alignment::Vertical::Center,
                    font: Font::MONOSPACE,
                    ..canvas::Text::default()
                });
            });
        }

        vec![frame.into_geometry()]
    }
}

/// 渲染整块提交图面板:有数据画 Canvas,出错画错误文案,两者皆无(比如
/// 尚未打开项目)画空状态提示。纯函数——不碰 `App`/`Workspace` 内部状态,
/// 调用方(`workspace.rs`)负责取数据、决定何时重建缓存。
pub fn view<'a>(
    snapshot: Option<&'a GitLogSnapshot>,
    error: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    if let Some(err) = error {
        return container(text(format!("git log 读取失败: {err}")).color(theme::RED))
            .padding(16)
            .into();
    }
    let Some(snapshot) = snapshot else {
        return container(text("未打开项目").color(theme::DIM))
            .padding(16)
            .into();
    };
    if snapshot.rows.is_empty() {
        return container(text("没有可显示的提交").color(theme::DIM))
            .padding(16)
            .into();
    }
    let height = ROW_HEIGHT * snapshot.rows.len() as f32;
    let canvas: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
        Canvas::new(GitLogCanvas { snapshot })
            .width(Length::Fill)
            .height(Length::Fixed(height))
            .into();
    let header = row![
        text(snapshot.repo_path.display().to_string())
            .size(workspace_font::caption())
            .color(theme::DIM)
    ]
    .padding([4, 8]);
    column![
        header,
        scrollable(canvas).width(Length::Fill).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// spike 验证核心问题:`gleisbau` 能否对 Dozer 自己这个真实、有分叉/合并
    /// 历史的仓库跑出合理的布局数据。跑 `cargo test -p dozer-app git_log::tests
    /// -- --nocapture` 看打印的前 20 行,人工核对 column/parents 是否符合直觉
    /// (主线一列到底,feature 分支另开列,merge commit 有多个 parent 边)。
    #[test]
    fn build_against_real_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/dozer-app 应有两层上级目录到仓库根");
        let snapshot = build(repo_root).expect("gleisbau 应能解析 dozer 自己的仓库");

        assert!(!snapshot.rows.is_empty(), "真实仓库应至少有一个 commit");
        assert!(
            snapshot.max_column < 50,
            "正常仓库的分支列数不该失控般大: {}",
            snapshot.max_column
        );

        for (row_idx, row) in snapshot.rows.iter().take(20).enumerate() {
            println!(
                "row={row_idx} col={} color={} sha={} parents={:?} summary={:?}",
                row.column, row.color_idx, row.short_sha, row.parents, row.summary
            );
        }

        // merge commit(有 ≥2 个 parent)理应至少出现一次——Dozer 仓库历史里
        // 确实有过 merge(如 2429d15),不是纯线性历史;若这条断了,说明布局
        // 丢了合并边,数据链路没走通。
        assert!(
            snapshot.rows.iter().any(|r| r.parents.len() >= 2),
            "200 个 commit 窗口内应能看到至少一个 merge"
        );
    }
}
