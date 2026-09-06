//! Code annotations belong to text runs, so editing and undo carry them naturally.
//! Automatic colors are derived separately, never written back into the document.
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use super::math_style::MathHue;
use super::{Block, Document, FlatPos, FlatRange, Inline};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    C,
    Cpp,
    Rust,
    Lua,
    Python,
    JavaScript,
    TypeScript,
    Java,
    CSharp,
    Go,
}

impl Language {
    pub const ALL: [Self; 10] = [
        Self::C,
        Self::Cpp,
        Self::Rust,
        Self::Lua,
        Self::Python,
        Self::JavaScript,
        Self::TypeScript,
        Self::Java,
        Self::CSharp,
        Self::Go,
    ];
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "c" => Self::C,
            "cpp" | "c++" | "cc" | "cxx" => Self::Cpp,
            "rust" | "rs" => Self::Rust,
            "lua" => Self::Lua,
            "python" | "py" => Self::Python,
            "javascript" | "js" => Self::JavaScript,
            "typescript" | "ts" => Self::TypeScript,
            "java" => Self::Java,
            "csharp" | "c_sharp" | "c#" | "cs" => Self::CSharp,
            "go" | "golang" => Self::Go,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Rust => "rust",
            Self::Lua => "lua",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Go => "go",
        }
    }
    fn config(self) -> &'static HighlightConfiguration {
        &CONFIGS[self as usize]
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeStyle {
    pub language: Option<Language>,
    pub manual: bool,
    pub color: Option<MathHue>,
}
impl CodeStyle {
    pub const PLAIN: Self = Self {
        language: None,
        manual: false,
        color: None,
    };
}

const CAPTURES: &[&str] = &[
    "comment",
    "string",
    "number",
    "constant",
    "keyword",
    "type",
    "function",
    "property",
    "operator",
    "variable",
    "punctuation",
    "attribute",
    "label",
    "constructor",
    "tag",
];
static CONFIGS: LazyLock<Vec<HighlightConfiguration>> = LazyLock::new(|| {
    Language::ALL
        .into_iter()
        .map(|language| {
            let (grammar, query) = match language {
                Language::C => (
                    tree_sitter_c::LANGUAGE,
                    tree_sitter_c::HIGHLIGHT_QUERY.to_owned(),
                ),
                Language::Cpp => (
                    tree_sitter_cpp::LANGUAGE,
                    format!(
                        "{}\n{}",
                        tree_sitter_c::HIGHLIGHT_QUERY,
                        tree_sitter_cpp::HIGHLIGHT_QUERY
                    ),
                ),
                Language::Rust => (
                    tree_sitter_rust::LANGUAGE,
                    tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
                ),
                Language::Lua => (
                    tree_sitter_lua::LANGUAGE,
                    tree_sitter_lua::HIGHLIGHTS_QUERY.to_owned(),
                ),
                Language::Python => (
                    tree_sitter_python::LANGUAGE,
                    tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
                ),
                Language::JavaScript => (
                    tree_sitter_javascript::LANGUAGE,
                    tree_sitter_javascript::HIGHLIGHT_QUERY.to_owned(),
                ),
                Language::TypeScript => (
                    tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
                    format!(
                        "{}\n{}",
                        tree_sitter_javascript::HIGHLIGHT_QUERY,
                        tree_sitter_typescript::HIGHLIGHTS_QUERY
                    ),
                ),
                Language::Java => (
                    tree_sitter_java::LANGUAGE,
                    tree_sitter_java::HIGHLIGHTS_QUERY.to_owned(),
                ),
                Language::CSharp => (
                    tree_sitter_c_sharp::LANGUAGE,
                    tree_sitter_c_sharp::HIGHLIGHTS_QUERY.to_owned(),
                ),
                Language::Go => (
                    tree_sitter_go::LANGUAGE,
                    tree_sitter_go::HIGHLIGHTS_QUERY.to_owned(),
                ),
            };
            let mut config =
                HighlightConfiguration::new(grammar.into(), language.name(), &query, "", "")
                    .expect("bundled highlight query must match its grammar");
            config.configure(CAPTURES);
            config
        })
        .collect()
});

/// Theme-independent ink, stored per character in the layout snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Syntax(usize),
    Manual(MathHue),
}
pub fn capture(index: usize) -> &'static str {
    CAPTURES[index]
}

/// Split a visual segment only at ink changes; source and caret coordinates
/// remain those of the original segment, including wrapped Unicode text.
pub fn painted<'a>(
    layout: &super::layout::DocLayout,
    block: usize,
    segment: &super::layout::Segment,
    text: &'a str,
) -> Vec<(&'a str, crate::theme::TextStyle)> {
    let source = &layout.source[block];
    let offset = source.inlines()[..segment.inline]
        .iter()
        .map(|run| run.text().chars().count())
        .sum::<usize>()
        + segment.start;
    let colors = &layout.code_colors[block][offset..offset + segment.len];
    let mut spans = Vec::new();
    let mut start = 0;
    let mut previous = colors.first().copied().flatten();
    for (index, (byte, _)) in text.char_indices().enumerate() {
        if colors[index] != previous {
            let mut style = super::layout::text_style(source, segment.style, layout.scale);
            if let Some(ink) = previous {
                style.color = crate::theme::code_ink(ink);
            }
            spans.push((&text[start..byte], style));
            start = byte;
            previous = colors[index];
        }
    }
    let mut style = super::layout::text_style(source, segment.style, layout.scale);
    if let Some(ink) = previous {
        style.color = crate::theme::code_ink(ink);
    }
    spans.push((&text[start..], style));
    spans
}

pub fn highlight(language: Language, text: &str) -> Vec<Option<Ink>> {
    let mut highlighter = Highlighter::new();
    let mut result = vec![None; text.len()];
    let mut stack = Vec::new();
    if let Ok(events) =
        highlighter.highlight(language.config(), text.as_bytes(), None, None, |_| None)
    {
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(ink)) => stack.push(ink.0),
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    result[start..end].fill(stack.last().copied().map(Ink::Syntax))
                }
                Err(_) => return vec![None; text.chars().count()],
            }
        }
    }
    text.char_indices().map(|(byte, _)| result[byte]).collect()
}

pub fn block_extent(blocks: &[Block], at: usize) -> std::ops::RangeInclusive<usize> {
    let mut first = at;
    while first > 0 && matches!(blocks[first], Block::CodeLine { first: false, .. }) {
        first -= 1;
    }
    let mut last = at;
    while matches!(
        blocks.get(last + 1),
        Some(Block::CodeLine { first: false, .. })
    ) {
        last += 1;
    }
    first..=last
}

pub fn colors(blocks: &[Block]) -> Vec<Vec<Option<Ink>>> {
    let mut colors: Vec<Vec<Option<Ink>>> = blocks
        .iter()
        .map(|block| {
            vec![
                None;
                block
                    .inlines()
                    .iter()
                    .map(|run| run.text().chars().count())
                    .sum()
            ]
        })
        .collect();
    let mut block = 0;
    while block < blocks.len() {
        if let Block::CodeLine { lang, .. } = &blocks[block] {
            let extent = block_extent(blocks, block);
            let end = *extent.end();
            let text = blocks[block..=end]
                .iter()
                .map(|b| b.inlines().iter().map(Inline::text).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            let settings = blocks[block].inlines()[0].style().syntax;
            let language = settings
                .language
                .or_else(|| lang.as_deref().and_then(Language::from_name));
            if !settings.manual
                && let Some(language) = language
            {
                let ink = highlight(language, &text);
                let mut start = 0;
                for row in &mut colors[block..=end] {
                    let len = row.len();
                    row.copy_from_slice(&ink[start..start + len]);
                    start += len + 1;
                }
            }
            block = end + 1;
        } else {
            let mut offset = 0;
            let runs = blocks[block].inlines();
            let mut i = 0;
            while i < runs.len() {
                let style = runs[i].style();
                let mut text = runs[i].text().to_owned();
                i += 1;
                if style.code {
                    while i < runs.len()
                        && runs[i].style().code
                        && runs[i].style().syntax.language == style.syntax.language
                        && runs[i].style().syntax.manual == style.syntax.manual
                    {
                        text.push_str(runs[i].text());
                        i += 1;
                    }
                    if !style.syntax.manual
                        && let Some(language) = style.syntax.language
                    {
                        let ink = highlight(language, &text);
                        colors[block][offset..offset + ink.len()].copy_from_slice(&ink);
                    }
                }
                offset += text.chars().count();
            }
            block += 1;
        }
    }
    for (block, row) in blocks.iter().zip(&mut colors) {
        let mut offset = 0;
        for run in block.inlines() {
            let len = run.text().chars().count();
            if (block.is_code() || run.style().code) && run.style().syntax.manual {
                row[offset..offset + len].fill(run.style().syntax.color.map(Ink::Manual));
            }
            offset += len;
        }
    }
    colors
}

impl Document {
    pub fn code_style_at(&self, position: FlatPos) -> Option<CodeStyle> {
        let block = self.scope().get(position.block)?;
        let (inline, _) = self.flat_to_pos(position.block, position.offset);
        Some(block.inlines().get(inline)?.style().syntax)
    }

    /// The full fence or contiguous inline code span containing a code target.
    pub fn code_extent(&self, range: FlatRange) -> FlatRange {
        let range = range.normalized();
        let Some(block) = self.scope().get(range.start.block) else {
            return range;
        };
        if block.is_code() {
            let extent = block_extent(self.scope(), range.start.block);
            return FlatRange::new(
                FlatPos {
                    block: *extent.start(),
                    offset: 0,
                },
                FlatPos {
                    block: *extent.end(),
                    offset: self.block_len(*extent.end()),
                },
            );
        }
        let mut offset = 0;
        let mut start = None;
        for run in block.inlines() {
            if run.style().code {
                start.get_or_insert(offset);
            } else if let Some(from) = start.take()
                && range.start.offset >= from
                && range.start.offset < offset
            {
                return FlatRange::new(
                    FlatPos {
                        block: range.start.block,
                        offset: from,
                    },
                    FlatPos {
                        block: range.start.block,
                        offset,
                    },
                );
            }
            offset += run.text().chars().count();
        }
        if let Some(from) = start
            && range.start.offset >= from
        {
            return FlatRange::new(
                FlatPos {
                    block: range.start.block,
                    offset: from,
                },
                FlatPos {
                    block: range.start.block,
                    offset,
                },
            );
        }
        range
    }

    pub fn set_code_options(&mut self, range: FlatRange, language: Option<Language>, manual: bool) {
        let range = self.code_extent(range);
        if range.start.block >= self.scope().len() {
            return;
        }
        let language = if manual {
            let block = &self.scope()[range.start.block];
            let (inline, _) = self.flat_to_pos(range.start.block, range.start.offset);
            block.inlines()[inline]
                .style()
                .syntax
                .language
                .or_else(|| match block {
                    Block::CodeLine { lang, .. } => lang.as_deref().and_then(Language::from_name),
                    _ => None,
                })
        } else {
            language
        };
        if let Block::CodeLine { lang, .. } = &mut self.scope_mut()[range.start.block] {
            *lang = language.map(|language| language.name().to_owned());
        }
        self.edit_code_styles(range, |style| {
            style.language = language;
            style.manual = manual;
        });
    }

    pub fn set_code_color(&mut self, range: FlatRange, color: Option<MathHue>) {
        self.edit_code_styles(range, |style| {
            if style.manual {
                style.color = color;
            }
        });
    }

    pub(super) fn edit_code_styles(&mut self, range: FlatRange, edit: impl Fn(&mut CodeStyle)) {
        let range = range.normalized();
        if range.start.block >= self.scope().len() {
            return;
        }
        let range = FlatRange::new(
            self.position(range.start.block, range.start.offset),
            self.position(range.end.block, range.end.offset),
        );
        let caret_block = self.caret.block;
        let caret_flat = self.caret_flat(caret_block);
        for block in range.start.block..=range.end.block {
            let from = if block == range.start.block {
                range.start.offset
            } else {
                0
            };
            let to = if block == range.end.block {
                range.end.offset
            } else {
                self.block_len(block)
            };
            let is_code = self.scope()[block].is_code();
            if from == to && self.block_len(block) == 0 && is_code {
                for run in self.scope_mut()[block].inlines_mut() {
                    if let Inline::Text(text) = run {
                        edit(&mut text.style.syntax);
                    }
                }
                continue;
            }
            let mut runs = self.slice_runs(block, 0, from);
            let mut selected = self.slice_runs(block, from, to);
            for run in &mut selected {
                if let Inline::Text(text) = run
                    && (is_code || text.style.code)
                {
                    edit(&mut text.style.syntax);
                }
            }
            runs.extend(selected);
            runs.extend(self.slice_runs(block, to, self.block_len(block)));
            *self.scope_mut()[block].inlines_mut() = runs;
        }
        self.dirty = true;
        self.enforce();
        self.restore_caret_flat(caret_block, caret_flat);
        self.refresh_context();
    }
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod tests;
