//! ByteBoy2077 语法高亮器:实现 `iced_core::text::Highlighter` trait,直接吃
//! `crate::preview::dozer_syntax_theme()`(锚定终端 16 色面板的自定义
//! syntect 主题),不用 `iced_highlighter` 自带的 5 个内置主题——它的
//! `Settings.theme` 是个封闭枚举,没有"传入任意 syntect Theme"的公开口子,
//! 没法把 ByteBoy2077 配色灌进去。
//!
//! 实现结构照抄 `iced_highlighter` 自己的源码(参照
//! `~/.cargo/registry/.../iced_highlighter-0.14.0/src/lib.rs`,同一份
//! syntect/two-face 依赖版本),只把主题来源换成单一的 dozer 主题——不再
//! 需要 `Settings.theme` 字段,`Settings` 只剩 `token`(语法名)。

use iced_widget::core::font;
use iced_widget::core::text::highlighter::{self, Format};
use iced_widget::core::{Color, Font};

use std::ops::Range;
use std::sync::LazyLock;

use syntect::highlighting;
use syntect::parsing;

static SYNTAXES: LazyLock<parsing::SyntaxSet> = LazyLock::new(two_face::syntax::extra_no_newlines);
static THEME: LazyLock<highlighting::Theme> = LazyLock::new(crate::preview::dozer_syntax_theme);

const LINES_PER_SNAPSHOT: usize = 50;

/// [`Highlighter`] 的配置:只有语法 token(文件扩展名对应的 syntect 语言名,
/// 见 `preview::extension_to_syntax`)——主题固定是 dozer 自己的
/// `dozer_syntax_theme()`,不是可配置项。
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub token: String,
}

/// 语法高亮器,drop-in 对应 `iced_highlighter::Highlighter` 但主题固定。
#[derive(Debug)]
pub struct Highlighter {
    syntax: &'static parsing::SyntaxReference,
    highlighter: highlighting::Highlighter<'static>,
    caches: Vec<(parsing::ParseState, parsing::ScopeStack)>,
    current_line: usize,
}

impl highlighter::Highlighter for Highlighter {
    type Settings = Settings;
    type Highlight = Highlight;

    type Iterator<'a> = Box<dyn Iterator<Item = (Range<usize>, Self::Highlight)> + 'a>;

    fn new(settings: &Self::Settings) -> Self {
        let syntax = SYNTAXES
            .find_syntax_by_token(&settings.token)
            .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());

        let highlighter = highlighting::Highlighter::new(&THEME);

        let parser = parsing::ParseState::new(syntax);
        let stack = parsing::ScopeStack::new();

        Highlighter {
            syntax,
            highlighter,
            caches: vec![(parser, stack)],
            current_line: 0,
        }
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        self.syntax = SYNTAXES
            .find_syntax_by_token(&new_settings.token)
            .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());

        self.highlighter = highlighting::Highlighter::new(&THEME);

        self.change_line(0);
    }

    fn change_line(&mut self, line: usize) {
        let snapshot = line / LINES_PER_SNAPSHOT;

        if snapshot <= self.caches.len() {
            self.caches.truncate(snapshot);
            self.current_line = snapshot * LINES_PER_SNAPSHOT;
        } else {
            self.caches.truncate(1);
            self.current_line = 0;
        }

        let (parser, stack) = self.caches.last().cloned().unwrap_or_else(|| {
            (
                parsing::ParseState::new(self.syntax),
                parsing::ScopeStack::new(),
            )
        });

        self.caches.push((parser, stack));
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        if self.current_line / LINES_PER_SNAPSHOT >= self.caches.len() {
            let (parser, stack) = self.caches.last().expect("caches must not be empty");

            self.caches.push((parser.clone(), stack.clone()));
        }

        self.current_line += 1;

        let (parser, stack) = self.caches.last_mut().expect("caches must not be empty");

        let ops = parser.parse_line(line, &SYNTAXES).unwrap_or_default();

        Box::new(scope_iterator(ops, line, stack, &self.highlighter))
    }

    fn current_line(&self) -> usize {
        self.current_line
    }
}

fn scope_iterator<'a>(
    ops: Vec<(usize, parsing::ScopeStackOp)>,
    line: &str,
    stack: &'a mut parsing::ScopeStack,
    highlighter: &'a highlighting::Highlighter<'static>,
) -> impl Iterator<Item = (Range<usize>, Highlight)> + 'a {
    ScopeRangeIterator {
        ops,
        line_length: line.len(),
        index: 0,
        last_str_index: 0,
    }
    .filter_map(move |(range, scope)| {
        let _ = stack.apply(&scope);

        if range.is_empty() {
            None
        } else {
            Some((
                range,
                Highlight(highlighter.style_mod_for_stack(&stack.scopes)),
            ))
        }
    })
}

/// 单个高亮片段(照抄 `iced_highlighter::Highlight`)。
#[derive(Debug)]
pub struct Highlight(highlighting::StyleModifier);

impl Highlight {
    pub fn color(&self) -> Option<Color> {
        self.0
            .foreground
            .map(|color| Color::from_rgba8(color.r, color.g, color.b, color.a as f32 / 255.0))
    }

    pub fn font(&self) -> Option<Font> {
        self.0.font_style.and_then(|style| {
            let bold = style.contains(highlighting::FontStyle::BOLD);
            let italic = style.contains(highlighting::FontStyle::ITALIC);

            if bold || italic {
                Some(Font {
                    weight: if bold {
                        font::Weight::Bold
                    } else {
                        font::Weight::Normal
                    },
                    style: if italic {
                        font::Style::Italic
                    } else {
                        font::Style::Normal
                    },
                    ..Font::MONOSPACE
                })
            } else {
                None
            }
        })
    }

    pub fn to_format(&self) -> Format<Font> {
        Format {
            color: self.color(),
            font: self.font(),
        }
    }
}

struct ScopeRangeIterator {
    ops: Vec<(usize, parsing::ScopeStackOp)>,
    line_length: usize,
    index: usize,
    last_str_index: usize,
}

impl Iterator for ScopeRangeIterator {
    type Item = (Range<usize>, parsing::ScopeStackOp);

    fn next(&mut self) -> Option<Self::Item> {
        if self.index > self.ops.len() {
            return None;
        }

        let next_str_i = if self.index == self.ops.len() {
            self.line_length
        } else {
            self.ops[self.index].0
        };

        let range = self.last_str_index..next_str_i;
        self.last_str_index = next_str_i;

        let op = if self.index == 0 {
            parsing::ScopeStackOp::Noop
        } else {
            self.ops[self.index - 1].1.clone()
        };

        self.index += 1;
        Some((range, op))
    }
}
