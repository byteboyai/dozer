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

/// 额外 vendored 的语法(注释感知的 JSONC / JSON5),放在 `assets/syntaxes/`
/// 下,用 `include_str!` 编译进二进制。two-face 的 bundle 自带的 `JSON`
/// 语法是旧版、不认 `//` / `/* */` 注释,所以这里单独接一份 JSONC 语法给
/// `.jsonc` / `.json5` 用(syntect 的 `add_from_str` 在 5.x 已被移进
/// `SyntaxSetBuilder`,所以走 `into_builder()` → `add()` → `build()` 这条路,
/// 见 `can_add_more_syntaxes_with_builder` 测试的用法)。
const EXTRA_SYNTAXES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/syntaxes");

static SYNTAXES: LazyLock<parsing::SyntaxSet> = LazyLock::new(|| {
    let mut builder = two_face::syntax::extra_no_newlines().into_builder();
    // `false` = 输入行不含换行,与 `extra_no_newlines` 保持一致(否则 `\n`
    // 相关的正则会被做等价替换,两套语法行为错开)。
    builder
        .add_from_folder(EXTRA_SYNTAXES_DIR, false)
        .expect("加载 assets/syntaxes 下的额外语法失败");
    builder.build()
});
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

#[cfg(test)]
mod repro {
    use super::*;
    use iced_widget::core::text::highlighter::Highlighter as _;

    #[test]
    fn repro_large_json5() {
        let path = "/Users/chrischiang/Projects/WorkProjects/Anrong/anrong_fincalc/anrong_fincalc_rule/financial_items/AFI_2025.json5";
        let text = std::fs::read_to_string(path).unwrap();
        let settings = Settings {
            token: "json".into(),
        };
        let mut h = Highlighter::new(&settings);
        h.change_line(0);
        for line in text.lines() {
            let it = h.highlight_line(line);
            for _ in it {}
        }
    }

    #[test]
    fn repro_content_with_text() {
        let path = "/Users/chrischiang/Projects/WorkProjects/Anrong/anrong_fincalc/anrong_fincalc_rule/financial_items/AFI_2025.json5";
        let text = std::fs::read_to_string(path).unwrap();
        eprintln!("lines={} bytes={}", text.lines().count(), text.len());
        let content =
            iced_widget::text_editor::Content::<iced_renderer::Renderer>::with_text(&text);
        eprintln!("content line_count={}", content.line_count());
    }

    #[test]
    fn repro_full_render() {
        crate::assets::fonts::load_embedded_fonts();
        crate::assets::fonts::sanitize_font_db();
        let path = "/Users/chrischiang/Projects/WorkProjects/Anrong/anrong_fincalc/anrong_fincalc_rule/financial_items/AFI_2025.json5";
        let text = std::fs::read_to_string(path).unwrap();

        use iced_widget::core::text::editor::Editor as _;
        use iced_widget::core::text::highlighter::Highlighter as _;

        let mut hl = Highlighter::new(&Settings {
            token: "json".into(),
        });
        let mut editor = iced_renderer::graphics::text::Editor::with_text(&text);

        let font = crate::assets::fonts::code_font();
        let size = iced_widget::core::Pixels(14.0);
        let line_height =
            iced_widget::core::text::LineHeight::Absolute(iced_widget::core::Pixels(21.0));
        let wrapping = iced_widget::core::text::Wrapping::default();
        let bounds = iced_widget::core::Size::new(1200.0, 800.0);

        editor.update(bounds, font, size, line_height, wrapping, &mut hl);
        editor.highlight(font, &mut hl, |h| h.to_format());
        eprintln!("full render ok");
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

#[cfg(test)]
mod jsonc_tests {
    use super::*;
    use syntect::parsing::{ParseState, ScopeStack};

    #[test]
    fn jsonc_and_json5_resolve_to_jsonc_grammar() {
        // `.jsonc` / `.json5` 必须落到 vendored 的 JSONC 语法(注释感知)。
        let jsonc = SYNTAXES
            .find_syntax_by_token("jsonc")
            .expect("jsonc token must resolve");
        assert_eq!(jsonc.name, "JSONC", "jsonc 必须映射到 JSONC 语法");

        let json5 = SYNTAXES
            .find_syntax_by_token("json5")
            .expect("json5 token must resolve");
        assert_eq!(json5.name, "JSONC", "json5 必须映射到 JSONC 语法");

        // 严格 `.json` 仍走 two-face bundle 自带的纯 JSON 语法(不认注释)。
        let json = SYNTAXES
            .find_syntax_by_token("json")
            .expect("json token must resolve");
        assert_eq!(json.name, "JSON", ".json 必须保留 bundle 的 JSON 语法");
    }

    #[test]
    fn jsonc_highlights_line_and_block_comments() {
        let syntax = SYNTAXES.find_syntax_by_token("jsonc").unwrap();
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let sample = "{\n  // line comment\n  \"k\": /* block */ \"v\",\n  \"n\": 1\n}\n";
        let mut saw_comment = false;
        for line in sample.lines() {
            let ops = state.parse_line(line, &SYNTAXES).expect("parse line");
            for (_off, op) in ops {
                let _ = stack.apply(&op);
                if stack
                    .scopes
                    .iter()
                    .any(|s| s.build_string().starts_with("comment."))
                {
                    saw_comment = true;
                }
            }
        }
        assert!(saw_comment, "JSONC 必须给 // 与 /* */ 注释着色");
    }
}
