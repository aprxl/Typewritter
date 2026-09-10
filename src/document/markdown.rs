//! Markdown parsing and serialization.
//!
//! `parse` builds a [`Document`](crate::document::Document) from Markdown
//! text; `serialize` writes a document back out deterministically.
//!
//! The grammar this implements is deliberately small (see `tasks/02`):
//! headings (levels 1–4), paragraphs, bold, italic, inline code spans,
//! fenced code blocks, `[[BADGE]]` chips, and `==highlights==`. Anything
//! unrecognized is literal paragraph text — there is no error path.
//!
//! Every marker is flat — content inside a matched run is literal — with
//! one exception: `==` is a background rather than a weight, so emphasis
//! nests inside it and the mark is distributed over the runs that come
//! back.
//!
//! Deliberate choice: paragraphs **re-flow to a single source line** on
//! save. A multi-line paragraph is joined with single spaces when parsed
//! and always written as one line, so there is exactly one canonical
//! spelling per document (SPEC §9).

use std::io;
use std::path::Path;

use super::math_notation;
use super::{
    BadgeColor, Block, Caret, Document, Focus, Inline, ListMarker, Sidenote, Style, Text, table,
};

/// Scan for the next unescaped occurrence of `marker` at or after `start`.
/// The char immediately before a match must be non-whitespace. Escaped
/// chars (`\x`) are skipped so `\*` never closes a run. Returns the byte-
/// agnostic char index of the closer.
fn find_closer(chars: &[char], marker: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += if i + 1 < chars.len() { 2 } else { 1 };
            continue;
        }
        if chars[i..].starts_with(marker) && (i == 0 || !chars[i - 1].is_whitespace()) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_dollar_closer(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += if i + 1 < chars.len() { 2 } else { 1 };
        } else if chars[i] == '$' {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

/// Remove escape backslashes. `\*` → `*`, `\\` → `\`, `\#` → `#`, and any
/// other `\x` collapses to `x` (serialize never emits those).
fn unescape(s: &[char]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == '\\' && i + 1 < s.len() {
            out.push(s[i + 1]);
            i += 2;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Whether `name` is a legal equation-label name: `[A-Za-z0-9_-]+`.
fn is_eq_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The length of an equation reference at the start of `chars`: `@eq:` plus
/// a legal label name, or `None` when this `@` is ordinary text.
fn eq_ref_at(chars: &[char]) -> Option<usize> {
    if chars.len() < 5 || chars[1] != 'e' || chars[2] != 'q' || chars[3] != ':' {
        return None;
    }
    let mut n = 4;
    while n < chars.len()
        && (chars[n].is_ascii_alphanumeric() || chars[n] == '_' || chars[n] == '-')
    {
        n += 1;
    }
    if n > 4 { Some(n) } else { None }
}

/// Parse a fence info string: `tw-math v1`, optionally followed by one
/// equation tag `#eq:<name>`. Returns whether the fence is a math block and
/// its label (`eq:<name>`, without the `#`). Any other spelling stays a
/// code fence, so unknown versions and foreign files always open.
fn parse_math_info(info: &str) -> (bool, Option<String>) {
    let mut tokens = info.split_whitespace();
    if tokens.next() != Some("tw-math") || tokens.next() != Some("v1") {
        return (false, None);
    }
    let mut tag = None;
    for token in tokens {
        if let Some(name) = token.strip_prefix("#eq:")
            && is_eq_name(name)
            && tag.is_none()
        {
            tag = Some(format!("eq:{name}"));
        }
    }
    (true, tag)
}

/// Parse the inline grammar into a `Vec<Inline>`. Runs are flat — markers
/// found inside a matched run are literal content, never nested.
fn parse_inline(s: &str) -> Vec<Inline> {
    let chars: Vec<char> = s.chars().collect();
    let mut runs: Vec<Inline> = Vec::new();
    let mut text_buf = String::new();
    let mut i = 0;

    let push_plain = |runs: &mut Vec<Inline>, buf: &mut String| {
        if !buf.is_empty() {
            runs.push(Inline::Text(Text {
                text: std::mem::take(buf),
                style: Style::PLAIN,
            }));
        }
    };

    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if i + 1 < chars.len() {
                    text_buf.push(chars[i + 1]);
                    i += 2;
                } else {
                    text_buf.push('\\');
                    i += 1;
                }
            }
            '$' => {
                if let Some(cl) = find_dollar_closer(&chars, i + 1) {
                    push_plain(&mut runs, &mut text_buf);
                    let inner: String = chars[i + 1..cl].iter().collect();
                    runs.push(Inline::Math(math_notation::parse(&inner)));
                    i = cl + 1;
                } else {
                    text_buf.push('$');
                    i += 1;
                }
            }
            '*' => {
                let n = {
                    let mut k = i;
                    while k < chars.len() && chars[k] == chars[i] {
                        k += 1;
                    }
                    k - i
                };
                // Map the run of marker chars to the longest recognised
                // marker: `***`, `**`, or `*`.
                let m = if n >= 3 {
                    3
                } else if n == 2 {
                    2
                } else {
                    1
                };
                let style = Style {
                    bold: m >= 2,
                    italic: m == 1 || m == 3,
                    ..Style::PLAIN
                };
                let marker: Vec<char> = vec!['*'; m];
                // Opening marker must be followed by a non-whitespace char.
                let after = i + m;
                let opener = after < chars.len() && !chars[after].is_whitespace();
                let content = if opener {
                    find_closer(&chars, &marker, after)
                } else {
                    None
                };
                if let Some(cl) = content {
                    let inner = &chars[after..cl];
                    if !inner.is_empty() {
                        push_plain(&mut runs, &mut text_buf);
                        runs.push(Inline::Text(Text {
                            text: unescape(inner),
                            style,
                        }));
                        i = cl + m;
                        continue;
                    }
                }
                // Not an opener, or no closer / empty content: literal.
                for _ in 0..n {
                    text_buf.push(chars[i]);
                }
                i += n;
            }
            '`' => {
                // Inline code span: scan for the closing backtick. Content
                // between backticks is taken verbatim (no unescape, no
                // emphasis scanning).
                let after = i + 1;
                let closer = chars[after..].iter().position(|&c| c == '`');
                if let Some(cl) = closer {
                    push_plain(&mut runs, &mut text_buf);
                    let inner: String = chars[after..after + cl].iter().collect();
                    runs.push(Inline::Text(Text {
                        text: inner,
                        style: Style {
                            code: true,
                            ..Style::PLAIN
                        },
                    }));
                    i = after + cl + 1;
                    continue;
                }
                // No closer: literal backtick.
                text_buf.push('`');
                i += 1;
            }
            '=' if chars.get(i + 1) == Some(&'=') => {
                let after = i + 2;
                let opener = chars.get(after).is_some_and(|c| !c.is_whitespace());
                let closer = if opener {
                    find_closer(&chars, &['=', '='], after)
                } else {
                    None
                };
                match closer.filter(|cl| *cl > after) {
                    Some(cl) => {
                        push_plain(&mut runs, &mut text_buf);
                        // Recursive, unlike every other marker here: a
                        // highlight is a background, so emphasis nests
                        // inside it rather than being literal.
                        let inner: String = chars[after..cl].iter().collect();
                        for run in parse_inline(&inner) {
                            match run {
                                Inline::Text(mut t) => {
                                    t.style.highlight = !t.style.is_boxed();
                                    runs.push(Inline::Text(t));
                                }
                                Inline::Math(list) => runs.push(Inline::Math(list)),
                                Inline::Note(label) => runs.push(Inline::Note(label)),
                                Inline::EqRef(label) => runs.push(Inline::EqRef(label)),
                            }
                        }
                        i = cl + 2;
                    }
                    None => {
                        text_buf.push_str("==");
                        i += 2;
                    }
                }
            }
            '[' if chars.get(i + 1) == Some(&'^') => {
                // A footnote anchor: `[^label]`. The label is the author's
                // and is carried verbatim into `Inline::Note`. No closing
                // bracket means it is ordinary prose.
                let after = i + 2;
                match chars[after..].iter().position(|&c| c == ']') {
                    Some(cl) => {
                        let label: String = chars[after..after + cl].iter().collect();
                        if label.is_empty() {
                            text_buf.push_str("[^]");
                        } else {
                            push_plain(&mut runs, &mut text_buf);
                            runs.push(Inline::Note(label));
                        }
                        i = after + cl + 1;
                    }
                    None => {
                        text_buf.push_str("[^");
                        i += 2;
                    }
                }
            }
            '[' if chars.get(i + 1) == Some(&'[') => {
                // A badge's label is verbatim, like a code span: it is a
                // short shout (`PS`, `TODO`), never emphasised prose.
                let after = i + 2;
                let closer = chars[after..]
                    .windows(2)
                    .position(|w| w == [']', ']'])
                    .map(|p| after + p);
                match closer.filter(|cl| *cl > after) {
                    Some(cl) => {
                        push_plain(&mut runs, &mut text_buf);
                        let raw: String = chars[after..cl].iter().collect();
                        let (badge_color, label) = if let Some(label) =
                            raw.strip_prefix("blue|").filter(|label| !label.is_empty())
                        {
                            (BadgeColor::Blue, label)
                        } else if let Some(label) =
                            raw.strip_prefix("green|").filter(|label| !label.is_empty())
                        {
                            (BadgeColor::Green, label)
                        } else if let Some(label) = raw
                            .strip_prefix("purple|")
                            .filter(|label| !label.is_empty())
                        {
                            (BadgeColor::Purple, label)
                        } else {
                            (BadgeColor::Orange, raw.as_str())
                        };
                        runs.push(Inline::Text(Text {
                            text: label.to_string(),
                            style: Style {
                                badge: true,
                                badge_color,
                                ..Style::PLAIN
                            },
                        }));
                        i = cl + 2;
                    }
                    None => {
                        text_buf.push_str("[[");
                        i += 2;
                    }
                }
            }
            '@' => {
                // An equation reference: `@eq:<name>`. Anything else after
                // the `@` is literal text — no error path, like every other
                // marker here.
                if let Some(len) = eq_ref_at(&chars[i..]) {
                    push_plain(&mut runs, &mut text_buf);
                    // The label is everything after the `@`: `eq:<name>`, the
                    // tag without its `#`.
                    let label: String = chars[i + 1..i + len].iter().collect();
                    runs.push(Inline::EqRef(label));
                    i += len;
                } else {
                    text_buf.push('@');
                    i += 1;
                }
            }
            c => {
                text_buf.push(c);
                i += 1;
            }
        }
    }
    push_plain(&mut runs, &mut text_buf);
    runs
}

/// A single empty paragraph — the model's "no content" block.
fn empty_paragraph() -> Block {
    Block::Paragraph(vec![Inline::Text(Text {
        text: String::new(),
        style: Style::PLAIN,
    })])
}

/// A thematic break: three or more `-` alone on a line. Only `-` is
/// accepted — `***` and `___` mean the same thing in CommonMark, but this
/// app writes `---`, and accepting one spelling keeps the round trip exact.
fn is_divider_line(line: &str) -> bool {
    let line = line.trim();
    line.len() >= 3 && line.chars().all(|c| c == '-')
}

/// Whether a serialized line would re-read as a list item's marker —
/// `- `, `* `, `+ `, or `N.`/`N) ` — and so needs a leading escape. Only a
/// line's *start* can claim it: mid-prose `-` and `1.` are ordinary text.
fn reads_as_list_item(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return true;
    }
    let bytes = line.as_bytes();
    let mut digits = 0;
    while digits < bytes.len() && bytes[digits].is_ascii_digit() {
        digits += 1;
    }
    digits > 0
        && digits < bytes.len()
        && (bytes[digits] == b'.' || bytes[digits] == b')')
        && bytes.get(digits + 1) == Some(&b' ')
}

/// Parse a line as a heading `#{1,4}\s+...`. Returns `(level, content)`.
fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut count = 0;
    while i < bytes.len() && bytes[i] == b'#' && count < 4 {
        count += 1;
        i += 1;
    }
    if count == 0 || i >= line.len() || !bytes[i].is_ascii_whitespace() {
        return None;
    }
    let rest = line[i + 1..].trim_start();
    Some((count as u8, rest))
}

/// The kinds of list marker a line can carry. The ordinal an ordered item
/// was typed with is tolerated (`N.`/`N)`) but never stored — the item's
/// place in its run is the only number there is, and serialization
/// renumbers the run anyway.
enum ListKind {
    Bullet,
    Ordered,
    Task { done: bool },
}

/// Parse a line as a list item — one line, never an indented continuation:
/// `- `/`* `/`+ ` bullets, `- [ ]` / `- [x]` tasks (also `* [ ]`, and either
/// case of the check), or `N.`/`N)` ordered. Returns the marker kind and
/// the content after it.
fn parse_list_item(line: &str) -> Option<(ListKind, &str)> {
    let (marker, rest) = line.split_once(' ')?;
    if matches!(marker, "-" | "*" | "+") {
        let rest = rest.trim_start();
        if let Some(content) = rest.strip_prefix("[ ]") {
            return Some((ListKind::Task { done: false }, content.trim_start()));
        }
        for (prefix, done) in [("[x]", true), ("[X]", true)] {
            if let Some(content) = rest.strip_prefix(prefix) {
                return Some((ListKind::Task { done }, content.trim_start()));
            }
        }
        return Some((ListKind::Bullet, rest));
    }
    let last = marker.chars().last()?;
    if marker.len() > 1
        && (last == '.' || last == ')')
        && marker[..marker.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        return Some((ListKind::Ordered, rest));
    }
    None
}

/// A footnote definition line `[^label]: body`. Returns `(label, body)` where
/// `body` is the text after the marker, trimmed of leading whitespace. The
/// label is the author's, read verbatim.
fn parse_definition(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let label = &rest[..close];
    let body = rest[close + 2..].trim_start();
    Some((label, body))
}

/// A footnote definition body as one paragraph block. A definition with no
/// text still yields one empty run, because every block must hold one.
fn note_block(body: &str) -> Block {
    let mut runs = parse_inline(body);
    if runs.is_empty() {
        runs.push(Inline::Text(Text {
            text: String::new(),
            style: Style::PLAIN,
        }));
    }
    Block::Paragraph(runs)
}

/// One pipe-table row. A leading and trailing pipe are optional on disk; the
/// serializer always writes both. `\\|` belongs to a cell rather than ending
/// it, and remains escaped for `parse_inline` to turn back into literal text.
fn parse_table_cells(line: &str, columns: Option<usize>) -> Option<Vec<String>> {
    let line = line.trim();
    if !line.contains('|') {
        return None;
    }
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            cell.push('\\');
            cell.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '|' {
            cells.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(ch);
        }
    }
    if escaped {
        cell.push('\\');
    }
    cells.push(cell.trim().to_string());
    if cells.is_empty() || columns.is_some_and(|columns| columns != cells.len()) {
        None
    } else {
        Some(cells)
    }
}

/// The Markdown delimiter row which makes a pipe row a table header.
fn table_divider_columns(line: &str) -> Option<usize> {
    let cells = parse_table_cells(line, None)?;
    (!cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':');
            cell.len() >= 3 && cell.chars().all(|ch| ch == '-')
        }))
    .then_some(cells.len())
}

fn table_cells(cells: Vec<String>) -> Vec<table::Cell> {
    cells
        .into_iter()
        .map(|text| {
            let mut contents = parse_inline(&text);
            if contents.is_empty() {
                contents.push(Inline::Text(Text {
                    text: unescape(&text.chars().collect::<Vec<_>>()),
                    style: Style::PLAIN,
                }));
            }
            table::Cell::from_runs(contents)
        })
        .collect()
}

fn table_rows(blocks: &[Block], first: usize) -> usize {
    (first..blocks.len())
        .take_while(|&index| {
            index == first
                || matches!(
                    blocks.get(index),
                    Some(Block::TableRow { first: false, .. })
                )
        })
        .count()
}

fn apply_table_settings(blocks: &mut [Block], first: usize, settings: table::TableSettings) {
    let rows = table_rows(blocks, first);
    let columns = blocks[first].cells().len();
    let shared = std::sync::Arc::new(settings.normalized(columns, rows));
    for row in &mut blocks[first..first + rows] {
        if let Block::TableRow {
            settings: row_settings,
            ..
        } = row
        {
            *row_settings = std::sync::Arc::clone(&shared);
        }
    }
}

/// Parse the presentation comment placed immediately after a table. The
/// Markdown remains a standard GFM table without it; absent or malformed
/// metadata simply gets the default full grid and equal tracks.
fn parse_table_metadata(line: &str) -> Option<table::TableSettings> {
    let body = line
        .trim()
        .strip_prefix("<!-- typewritter-table v1 ")?
        .strip_suffix(" -->")?;
    let mut lines = None;
    let mut columns = None;
    let mut rows = None;
    for part in body.split_whitespace() {
        if let Some(bits) = part.strip_prefix("lines=") {
            lines = table::TableLines::from_bits(bits);
        } else if let Some(values) = part.strip_prefix("cols=") {
            columns = Some(
                values
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<Vec<f32>, _>>()
                    .ok()?,
            );
        } else if let Some(values) = part.strip_prefix("rows=") {
            rows = Some(
                values
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<Vec<f32>, _>>()
                    .ok()?,
            );
        }
    }
    Some(table::TableSettings {
        lines: lines?,
        column_shares: columns?,
        row_heights: rows?,
    })
}

fn table_metadata(settings: &table::TableSettings) -> String {
    let columns = settings
        .column_shares
        .iter()
        .map(|value| format!("{value:.6}"))
        .collect::<Vec<_>>()
        .join(",");
    let rows = settings
        .row_heights
        .iter()
        .map(|value| format!("{value:.2}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "<!-- typewritter-table v1 lines={} cols={columns} rows={rows} -->",
        settings.lines.bits()
    )
}

fn serialize_table_cell(cell: &table::Cell) -> String {
    cell.lines()
        .iter()
        .map(|line| serialize_runs(line))
        .collect::<Vec<_>>()
        .join("<br>")
        .replace('|', "\\|")
}

/// Build a `Document` from Markdown text. `path` only seeds `Document`'s
/// `path`/`name` fields. Pure — no IO.
pub fn parse(path: &Path, text: &str) -> Document {
    let normalized;
    let text = if text.contains("\r\n") {
        normalized = text.replace("\r\n", "\n");
        normalized.as_str()
    } else {
        text
    };
    let (text, metadata) = super::code_metadata::detach(text);
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut fence_had_lines = false;
    let mut fence_lang: Option<String> = None;
    let mut fence_math = false;
    let mut fence_tag: Option<String> = None;
    let mut math_body = String::new();
    let mut definitions: Vec<(String, Block)> = Vec::new();
    // Pipe tables are read as a contiguous group after their header divider.
    // The optional Typewritter comment immediately following the last row
    // restores presentation state without making the Markdown itself opaque.
    let mut open_table: Option<(usize, usize)> = None;
    // The running ordinal of the ordered run being read. Reset by every
    // non-ordered-item block, so a run numbers 1,2,3… the way serialization
    // writes it.
    let mut ordinal = 0u32;

    let flush_para = |blocks: &mut Vec<Block>, para: &mut Vec<String>| {
        if !para.is_empty() {
            let joined = para.join(" ");
            blocks.push(Block::Paragraph(parse_inline(&joined)));
            para.clear();
        }
    };

    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some((first, columns)) = open_table {
            if let Some(cells) = parse_table_cells(line, Some(columns)) {
                let rows = table_rows(&blocks, first) + 1;
                let settings = blocks[first]
                    .table_settings()
                    .expect("open table starts on a table row")
                    .clone()
                    .normalized(columns, rows);
                let shared = std::sync::Arc::clone(
                    blocks[first]
                        .table_settings_arc()
                        .expect("open table starts on a table row"),
                );
                blocks.push(Block::TableRow {
                    cells: table_cells(cells),
                    first: false,
                    settings: shared,
                });
                apply_table_settings(&mut blocks, first, settings);
                continue;
            }
            if let Some(settings) = parse_table_metadata(line) {
                apply_table_settings(&mut blocks, first, settings);
                open_table = None;
                continue;
            }
            open_table = None;
        }
        if in_fence {
            if line.trim() == "```" {
                if fence_math {
                    blocks.push(Block::Math {
                        list: vec![Inline::Math(math_notation::parse(&math_body))],
                        tag: fence_tag.take(),
                    });
                    math_body.clear();
                } else if !fence_had_lines {
                    let empty = Inline::Text(Text {
                        text: String::new(),
                        style: Style {
                            code: true,
                            ..Style::PLAIN
                        },
                    });
                    blocks.push(Block::CodeLine {
                        content: vec![empty],
                        first: true,
                        lang: fence_lang.clone(),
                    });
                }
                in_fence = false;
                fence_had_lines = false;
                fence_lang = None;
                fence_math = false;
                fence_tag = None;
            } else {
                if fence_math {
                    math_body.push_str(line);
                } else {
                    let run = Inline::Text(Text {
                        text: line.to_string(),
                        style: Style {
                            code: true,
                            ..Style::PLAIN
                        },
                    });
                    let first = !fence_had_lines;
                    let lang = if first { fence_lang.clone() } else { None };
                    blocks.push(Block::CodeLine {
                        content: vec![run],
                        first,
                        lang,
                    });
                    fence_had_lines = true;
                }
            }
            continue;
        }
        if line.trim().starts_with("```") {
            flush_para(&mut blocks, &mut para);
            let info = line.trim()[3..].trim();
            // Unknown tw-math versions remain code so newer files always open.
            let (math, tag) = parse_math_info(info);
            fence_math = math;
            fence_tag = tag;
            math_body.clear();
            fence_lang = if info.is_empty() {
                None
            } else {
                Some(info.to_string())
            };
            in_fence = true;
            fence_had_lines = false;
            continue;
        }
        if is_divider_line(line) {
            flush_para(&mut blocks, &mut para);
            blocks.push(Block::Divider(vec![Inline::Text(Text {
                text: String::new(),
                style: Style::PLAIN,
            })]));
            continue;
        }
        // The delimiter row claims the one pending pipe row as a table
        // header. We do this before ordinary paragraph parsing, so a table
        // is recognised even though the streaming parser has no look-ahead.
        if let Some(columns) = table_divider_columns(line)
            && para.len() == 1
            && let Some(cells) = parse_table_cells(&para[0], Some(columns))
        {
            para.clear();
            let settings = std::sync::Arc::new(table::TableSettings::new(columns, 1));
            let first = blocks.len();
            blocks.push(Block::TableRow {
                cells: table_cells(cells),
                first: true,
                settings,
            });
            open_table = Some((first, columns));
            continue;
        }
        // A definition is collected out of the block stream into the
        // document's notes, so it is never a stray paragraph mid-prose.
        if let Some((label, body)) = parse_definition(line) {
            flush_para(&mut blocks, &mut para);
            definitions.push((label.to_string(), note_block(body)));
            continue;
        }
        if let Some((level, content)) = parse_heading(line) {
            flush_para(&mut blocks, &mut para);
            let mut inlines = parse_inline(content);
            if inlines.is_empty() {
                // Every block keeps at least one run, even if empty (see
                // `empty_paragraph`), so a Heading with no text isn't
                // demoted to having zero runs.
                inlines.push(Inline::Text(Text {
                    text: String::new(),
                    style: Style::PLAIN,
                }));
            }
            blocks.push(Block::Heading {
                level,
                folded: false,
                content: inlines,
            });
        } else if let Some((kind, content)) = parse_list_item(line) {
            flush_para(&mut blocks, &mut para);
            // An ordered run continues only through consecutive ordered
            // items; a bullet or task item between them restarts it, as
            // does any non-list block.
            if !matches!(
                blocks.last(),
                Some(Block::ListItem {
                    marker: ListMarker::Number(_),
                    ..
                })
            ) {
                ordinal = 0;
            }
            ordinal += 1;
            let marker = match kind {
                ListKind::Bullet => ListMarker::Bullet,
                ListKind::Task { done } => ListMarker::Task { done },
                ListKind::Ordered => ListMarker::Number(ordinal),
            };
            let mut runs = parse_inline(content);
            if runs.is_empty() {
                // Every block keeps at least one run, even if empty.
                runs.push(Inline::Text(Text {
                    text: String::new(),
                    style: Style::PLAIN,
                }));
            }
            blocks.push(Block::ListItem {
                marker,
                content: runs,
            });
        } else if line.trim().is_empty() {
            flush_para(&mut blocks, &mut para);
        } else {
            para.push(line.to_string());
        }
    }
    flush_para(&mut blocks, &mut para);

    if in_fence {
        // Unterminated fences still produce their content; the file must open.
        if fence_math {
            blocks.push(Block::Math {
                list: vec![Inline::Math(math_notation::parse(&math_body))],
                tag: fence_tag,
            });
        }
    }

    if blocks.is_empty() {
        blocks.push(empty_paragraph());
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());

    // A note is "anchored" only when its label actually occurs in the prose.
    // A definition with no anchor is still kept — it arrived from disk and
    // must survive — but it is flagged so `prune_runs` leaves it alone.
    let anchored_labels: Vec<&str> = blocks
        .iter()
        .flat_map(Block::inlines)
        .filter_map(|run| match run {
            Inline::Note(label) => Some(label.as_str()),
            _ => None,
        })
        .collect();
    let notes = definitions
        .into_iter()
        .map(|(label, body)| Sidenote {
            anchored: anchored_labels.contains(&label.as_str()),
            label,
            body: vec![body],
        })
        .collect();

    let mut doc = Document {
        body: blocks,
        path: path.to_path_buf(),
        name,
        dirty: false,
        caret: Caret {
            block: 0,
            inline: 0,
            offset: 0,
            style: Style::PLAIN,
        },
        math: None,
        notes,
        focus: Focus::Body,
    };
    super::code_metadata::apply(&mut doc, metadata);
    doc
}

/// Escape the literal text of a run: `\` → `\\`, `*` → `\*`.
fn escape_run_text(t: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => out.push_str("\\\\"),
            '*' => out.push_str("\\*"),
            '`' => out.push_str("\\`"),
            '$' => out.push_str("\\$"),
            // Only the doubled forms are markers, so only those need
            // escaping — prose is full of lone `=` and `[`, and escaping
            // every one of them would make the file unreadable on disk.
            c @ ('=' | '[') if chars.get(i + 1) == Some(&c) => {
                out.push('\\');
                out.push(c);
            }
            // `[^` opens a footnote anchor; escape the bracket so a literal
            // `[^note]` in prose re-reads as text, not as an anchor.
            '[' if chars.get(i + 1) == Some(&'^') => {
                out.push_str("\\[");
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// Serialize one run, wrapping it in the marker for its style.
fn wrap_run(style: Style, escaped: &str) -> String {
    if style.code {
        format!("`{escaped}`")
    } else if style.badge {
        match style.badge_color {
            BadgeColor::Orange => format!("[[{escaped}]]"),
            BadgeColor::Blue => format!("[[blue|{escaped}]]"),
            BadgeColor::Green => format!("[[green|{escaped}]]"),
            BadgeColor::Purple => format!("[[purple|{escaped}]]"),
        }
    } else if style.bold && style.italic {
        format!("***{escaped}***")
    } else if style.bold {
        format!("**{escaped}**")
    } else if style.italic {
        format!("*{escaped}*")
    } else {
        escaped.to_string()
    }
}

/// Serialize a block's inline runs (each escaped then wrapped).
///
/// Highlight is the one style spanning runs rather than sitting on one: a
/// stretch of adjacent marked runs shares a single pair of `==` markers,
/// since re-opening them at every emphasis change would both read badly on
/// disk and parse back as several marks.
fn serialize_runs(runs: &[Inline]) -> String {
    // A boxed run's text is verbatim on the way in, so it must be verbatim
    // on the way out too.
    let one = |run: &Inline, style: Style| match run {
        Inline::Text(t) => {
            if style.is_boxed() {
                wrap_run(style, &t.text)
            } else {
                wrap_run(style, &escape_run_text(&t.text))
            }
        }
        Inline::Math(list) => format!("${}$", math_notation::print(list)),
        Inline::Note(label) => format!("[^{label}]"),
        Inline::EqRef(label) => format!("@{label}"),
    };
    let mut out = String::new();
    let mut i = 0;
    while i < runs.len() {
        if !runs[i].style().highlight {
            out.push_str(&one(&runs[i], runs[i].style()));
            i += 1;
            continue;
        }
        let mut end = i;
        while runs.get(end + 1).is_some_and(|r| r.style().highlight) {
            end += 1;
        }
        out.push_str("==");
        for run in &runs[i..=end] {
            out.push_str(&one(
                run,
                Style {
                    highlight: false,
                    ..run.style()
                },
            ));
        }
        out.push_str("==");
        i = end + 1;
    }
    out
}

/// Write a document back out, deterministically.
pub fn serialize(doc: &Document) -> String {
    let plain = super::code_metadata::plain(doc);
    let mut text = serialize_plain(&plain);
    super::code_metadata::append(doc, &mut text);
    text
}

fn serialize_plain(doc: &Document) -> String {
    // Empty document (one empty paragraph) serializes to ""; "" parses
    // back to one empty paragraph, so "" is the fixpoint. An empty
    // Heading must NOT take this path — it would lose its heading-ness.
    let empty = doc.body().len() == 1 && matches!(doc.body()[0], Block::Paragraph(_)) && {
        let runs = doc.body()[0].inlines();
        runs.len() == 1 && matches!(runs[0], Inline::Text(Text { ref text, .. }) if text.is_empty())
    };
    if empty && doc.notes.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut i = 0;
    let mut first = true;
    // The running ordinal of the ordered run being written, reset by every
    // non-ordered-item block. The stored `Number` is never printed; the
    // run's sequence is the number.
    let mut ordinal = 0u32;
    while i < doc.body().len() {
        if !first {
            out.push_str("\n\n");
        }
        first = false;
        if !matches!(
            &doc.body()[i],
            Block::ListItem {
                marker: ListMarker::Number(_),
                ..
            }
        ) {
            ordinal = 0;
        }
        match &doc.body()[i] {
            Block::TableRow {
                first: true,
                settings,
                ..
            } => {
                let first_table_row = &doc.body()[i];
                let columns = first_table_row.cells().len();
                let mut end = i + 1;
                while matches!(
                    doc.body().get(end),
                    Some(Block::TableRow { first: false, .. })
                ) {
                    end += 1;
                }
                let rows = end - i;
                let settings = (**settings).clone().normalized(columns, rows);
                let write_row = |out: &mut String, row: &Block| {
                    out.push('|');
                    for cell in row.cells() {
                        out.push(' ');
                        out.push_str(&serialize_table_cell(cell));
                        out.push_str(" |");
                    }
                };
                write_row(&mut out, first_table_row);
                out.push('\n');
                out.push('|');
                for _ in 0..columns {
                    out.push_str(" --- |");
                }
                for row in &doc.body()[i + 1..end] {
                    out.push('\n');
                    write_row(&mut out, row);
                }
                out.push('\n');
                out.push_str(&table_metadata(&settings));
                i = end;
            }
            // A malformed external edit can leave a continuation row without
            // its first row. It still serializes as a harmless paragraph
            // rather than panicking or dropping the reader's text.
            Block::TableRow {
                cells,
                first: false,
                ..
            } => {
                out.push_str(
                    &cells
                        .iter()
                        .map(serialize_table_cell)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                i += 1;
            }
            Block::CodeLine { lang, .. } => {
                let opener = match lang {
                    Some(l) => format!("```{l}\n"),
                    None => "```\n".to_string(),
                };
                out.push_str(&opener);
                let mut j = i;
                loop {
                    let line: String = doc.body()[j].inlines().iter().map(Inline::text).collect();
                    out.push_str(&line);
                    out.push('\n');
                    j += 1;
                    match doc.body().get(j) {
                        Some(Block::CodeLine { first: false, .. }) => continue,
                        _ => break,
                    }
                }
                out.push_str("```");
                i = j;
            }
            Block::Heading {
                level,
                folded: _,
                content,
            } => {
                out.push_str(&"#".repeat(*level as usize));
                out.push(' ');
                out.push_str(&serialize_runs(content));
                i += 1;
            }
            Block::Divider(_) => {
                out.push_str("---");
                i += 1;
            }
            Block::Math { list: runs, tag } => {
                let list = match runs.as_slice() {
                    [Inline::Math(list)] => list,
                    _ => unreachable!("enforced math block invariant"),
                };
                out.push_str("```tw-math v1");
                if let Some(tag) = tag {
                    out.push_str(" #");
                    out.push_str(tag);
                }
                out.push('\n');
                out.push_str(&math_notation::print(list));
                out.push_str("\n```");
                i += 1;
            }
            Block::ListItem { marker, content } => {
                let mut line = serialize_runs(content);
                // A bullet whose content would read as a task box (or an
                // empty one) is escaped so it re-reads as the bullet it is.
                if matches!(marker, ListMarker::Bullet)
                    && (line.starts_with("[ ]")
                        || line.starts_with("[x]")
                        || line.starts_with("[X]"))
                {
                    line.insert(0, '\\');
                }
                match marker {
                    ListMarker::Bullet => out.push_str("- "),
                    ListMarker::Task { done } => {
                        out.push_str(if *done { "- [x] " } else { "- [ ] " });
                    }
                    ListMarker::Number(_) => {
                        ordinal += 1;
                        out.push_str(&ordinal.to_string());
                        out.push_str(". ");
                    }
                }
                out.push_str(&line);
                i += 1;
            }
            Block::Paragraph(runs) => {
                let mut line = serialize_runs(runs);
                // Guard a paragraph that would otherwise re-read as a
                // heading (e.g. text `# foo`).
                if line.starts_with('#') {
                    line.insert(0, '\\');
                }
                // Guard a paragraph that would otherwise re-read as a
                // divider (e.g. text `---`).
                if is_divider_line(&line) {
                    line.insert(0, '\\');
                }
                // …or as a list item (e.g. text `- tea` or `1) note`).
                if reads_as_list_item(&line) {
                    line.insert(0, '\\');
                }
                out.push_str(&line);
                i += 1;
            }
        }
    }

    if doc.notes.is_empty() {
        out.push('\n');
    } else {
        // Definitions go at the end of the file, in the order their anchors
        // appear in the prose; a definition nothing anchors (a stray one
        // from disk) follows, in its own stored order, so both kinds survive
        // a round trip unchanged.
        let mut anchored: Vec<&str> = Vec::new();
        for block in doc.body() {
            for run in block.inlines() {
                if let Inline::Note(label) = run
                    && !anchored.contains(&label.as_str())
                {
                    anchored.push(label);
                }
            }
        }
        let mut order: Vec<&Sidenote> = Vec::new();
        for label in &anchored {
            if let Some(note) = doc.notes.iter().find(|note| note.label == *label) {
                order.push(note);
            }
        }
        for note in &doc.notes {
            if !anchored.contains(&note.label.as_str()) {
                order.push(note);
            }
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        for note in order {
            out.push_str("[^");
            out.push_str(&note.label);
            out.push_str("]: ");
            out.push_str(&serialize_runs(note.body[0].inlines()));
            out.push('\n');
        }
    }
    out
}

impl Document {
    /// Read a file off disk into a `Document`. `None` if unreadable or if
    /// the bytes contain a NUL (the binary marker — images/PDFs won't open).
    pub fn load(path: &Path) -> Option<Document> {
        let bytes = std::fs::read(path).ok()?;
        if bytes.contains(&0) {
            return None;
        }
        let text = String::from_utf8_lossy(&bytes);
        Some(parse(path, &text))
    }

    /// Write this document's canonical Markdown to `self.path` and clear
    /// `dirty` on success.
    pub fn save(&mut self) -> io::Result<()> {
        std::fs::write(&self.path, serialize(self))?;
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(blocks: Vec<Block>) -> Document {
        let mut d = Document::new(Path::new("notes/test.md"));
        *d.body_mut() = blocks;
        d
    }

    fn run(t: &str, style: Style) -> Inline {
        Inline::Text(Text {
            text: t.into(),
            style,
        })
    }

    fn plain(t: &str) -> Inline {
        run(t, Style::PLAIN)
    }

    /// `$` opens an expression in the editor, so `$$` is the escape hatch
    /// that types a literal one. That literal has to survive the file: it is
    /// written escaped, and must come back as text rather than reopening as
    /// math on the next load.
    #[test]
    fn a_literal_dollar_round_trips_as_text() {
        let doc = doc_with(vec![Block::Paragraph(vec![plain("costs $40 and $12")])]);
        let text = serialize(&doc);
        assert!(text.contains("\\$40"), "written escaped: {text}");
        let back = parse(Path::new("notes/test.md"), &text);
        assert_eq!(
            back.body(),
            doc.body(),
            "a literal dollar must not reopen as math"
        );
    }

    fn bold(t: &str) -> Inline {
        run(
            t,
            Style {
                bold: true,
                ..Style::PLAIN
            },
        )
    }

    fn italic(t: &str) -> Inline {
        run(
            t,
            Style {
                italic: true,
                ..Style::PLAIN
            },
        )
    }

    fn bold_italic(t: &str) -> Inline {
        run(
            t,
            Style {
                bold: true,
                italic: true,
                ..Style::PLAIN
            },
        )
    }

    fn code(t: &str) -> Inline {
        run(
            t,
            Style {
                code: true,
                ..Style::PLAIN
            },
        )
    }

    fn math(t: &str) -> Inline {
        Inline::Math(math_notation::parse(t))
    }

    fn para(runs: Vec<Inline>) -> Block {
        Block::Paragraph(runs)
    }

    fn head(level: u8, runs: Vec<Inline>) -> Block {
        Block::Heading {
            level,
            folded: false,
            content: runs,
        }
    }

    #[test]
    fn folded_state_never_reaches_disk() {
        let text = "# Top\n\nbody\n";
        let plain = parse(Path::new("x"), text);
        let printed_plain = serialize(&plain);

        let mut folded = parse(Path::new("x"), text);
        if let Block::Heading { folded: state, .. } = &mut folded.body_mut()[0] {
            *state = true;
        }
        assert_eq!(
            serialize(&folded),
            printed_plain,
            "the flag is editor state only"
        );
        // And re-parsing the file opens unfolded, whatever was on disk.
        assert!(!parse(Path::new("x"), &printed_plain).body()[0].is_folded());
    }

    #[test]
    fn ast_round_trip() {
        let fixtures = vec![
            doc_with(vec![empty_paragraph()]),
            doc_with(vec![para(vec![plain("hello world")])]),
            doc_with(vec![head(1, vec![plain("title")])]),
            doc_with(vec![head(2, vec![plain("title")])]),
            doc_with(vec![head(3, vec![plain("title")])]),
            doc_with(vec![head(4, vec![plain("title")])]),
            doc_with(vec![para(vec![bold("x")])]),
            doc_with(vec![para(vec![italic("x")])]),
            doc_with(vec![para(vec![bold_italic("x")])]),
            // Literal `*` and `\`.
            doc_with(vec![para(vec![plain("a * b \\ c")])]),
            // Leading `#` in a paragraph.
            doc_with(vec![para(vec![plain("#foo")])]),
            // Adjacent same-style (bold) runs stay distinct via markers.
            doc_with(vec![para(vec![bold("a"), bold("b")])]),
            // `* spaced out *` markers stay literal.
            doc_with(vec![para(vec![plain("* spaced out *")])]),
        ];
        for d in fixtures {
            let text = serialize(&d);
            let back = parse(Path::new("notes/test.md"), &text);
            assert_eq!(back.body(), d.body(), "round-trip failed for: {text:?}");
        }
    }

    #[test]
    fn canonical_stability() {
        let sources = vec![
            "hello world\n",
            "# title\n",
            "**bold** and *italic*\n",
            "a\n\nb\n",
            "multi\nline\nparagraph\n",
            "* not a marker\n",
            "#foo\n",
            "\n",
            "",
        ];
        for s in sources {
            let once = serialize(&parse(
                Path::new("x"),
                &serialize(&parse(Path::new("x"), s)),
            ));
            let ast = serialize(&parse(Path::new("x"), s));
            assert_eq!(once, ast, "canonical stability failed for {s:?}");
        }
    }

    #[test]
    fn pipe_tables_round_trip_with_local_grid_and_track_metadata() {
        let text = "| Left | Right |\n| --- | --- |\n| a\\|b | c |\n<!-- typewritter-table v1 lines=101011 cols=0.250000,0.750000 rows=42.00,31.00 -->\n";
        let document = parse(Path::new("table.md"), text);
        assert_eq!(document.body().len(), 2);
        assert!(document.body().iter().all(Block::is_table));
        assert!(document.body()[0].table_first());
        assert_eq!(
            super::super::cell_text(&document.body()[1].cells()[0]),
            "a|b"
        );
        let settings = document.body()[0].table_settings().unwrap();
        assert!(!settings.lines.bottom);
        assert!(settings.lines.left);
        assert!(!settings.lines.right);
        assert_eq!(settings.column_shares, vec![0.25, 0.75]);
        assert_eq!(settings.row_heights, vec![42.0, 31.0]);
        assert_eq!(serialize(&document), text);
        assert_eq!(
            parse(Path::new("table.md"), &serialize(&document)).body(),
            document.body()
        );
    }

    #[test]
    fn a_plain_gfm_table_receives_deterministic_local_defaults() {
        let document = parse(
            Path::new("table.md"),
            "| A | B |\n| --- | --- |\n| C | D |\n",
        );
        let saved = serialize(&document);
        assert!(saved.contains("<!-- typewritter-table v1 lines=111111"));
        assert_eq!(parse(Path::new("table.md"), &saved).body(), document.body());
    }

    #[test]
    fn a_formatted_table_cell_remains_one_structural_cell_on_disk() {
        let text = "| **Left** | Right |\n| --- | --- |\n<!-- typewritter-table v1 lines=111111 cols=0.500000,0.500000 rows=38.00 -->\n";
        let document = parse(Path::new("table.md"), text);
        let [Inline::Text(cell)] = document.body()[0].cells()[0].runs() else {
            panic!("the formatted cell contains one text run")
        };
        assert!(cell.style.bold);
        assert_eq!(document.body()[0].cells().len(), 2);
        assert_eq!(serialize(&document), text);
    }

    #[test]
    fn a_math_table_cell_round_trips_as_one_structural_cell() {
        let text = "| $x/2$ | Right |\n| --- | --- |\n<!-- typewritter-table v1 lines=111111 cols=0.500000,0.500000 rows=38.00 -->\n";
        let document = parse(Path::new("table.md"), text);
        assert!(matches!(
            document.body()[0].cells()[0].runs(),
            [Inline::Math(_)]
        ));
        assert_eq!(document.body()[0].cells().len(), 2);
        assert_eq!(serialize(&document), text);
        assert_eq!(
            parse(Path::new("table.md"), &serialize(&document)).body(),
            document.body()
        );
    }

    #[test]
    fn mixed_table_cell_formatting_and_math_round_trip_in_one_cell() {
        let text = "| **bold** plain $x$ tail | Right |\n| --- | --- |\n<!-- typewritter-table v1 lines=111111 cols=0.500000,0.500000 rows=38.00 -->\n";
        let document = parse(Path::new("table.md"), text);
        let contents = document.body()[0].cells()[0].runs();
        assert!(
            matches!(&contents[0], Inline::Text(text) if text.text == "bold" && text.style.bold)
        );
        assert!(contents.iter().any(|run| matches!(run, Inline::Math(_))));
        assert_eq!(serialize(&document), text);
        assert_eq!(
            parse(Path::new("table.md"), &serialize(&document)).body(),
            document.body()
        );
    }

    #[test]
    fn a_rule_round_trips() {
        let d = parse(Path::new("x"), "before\n\n---\n\nafter\n");
        assert!(d.body()[1].is_divider());
        assert_eq!(serialize(&d), "before\n\n---\n\nafter\n");
        assert_eq!(parse(Path::new("x"), &serialize(&d)).body(), d.body());
    }

    #[test]
    fn a_math_block_round_trips_through_its_fence() {
        let d = doc_with(vec![Block::Math {
            list: vec![math("1/2")],
            tag: None,
        }]);
        assert_eq!(serialize(&d), "```tw-math v1\n1/2\n```\n");
        assert_eq!(parse(Path::new("x"), &serialize(&d)).body(), d.body());
    }

    #[test]
    fn inline_math_round_trips_through_dollars() {
        let d = doc_with(vec![para(vec![
            plain("before "),
            math("a/b"),
            plain(" after"),
        ])]);
        let text = serialize(&d);
        assert_eq!(text, "before $a/b$ after\n");
        assert_eq!(parse(Path::new("x"), &text).body(), d.body());
    }

    #[test]
    fn a_prose_dollar_is_escaped_not_parsed() {
        let d = doc_with(vec![para(vec![plain("price $5")])]);
        assert_eq!(serialize(&d), "price \\$5\n");
        assert_eq!(parse(Path::new("x"), &serialize(&d)).body(), d.body());
    }

    #[test]
    fn an_unknown_tw_math_version_stays_a_code_block() {
        let d = parse(Path::new("x"), "```tw-math v2\nx\n```\n");
        assert!(d.body()[0].is_code());
        assert_eq!(serialize(&d), "```tw-math v2\nx\n```\n");
    }

    #[test]
    fn a_paragraph_of_dashes_is_not_a_rule() {
        let d = parse(Path::new("x"), "\\---\n");
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "---");
        assert_eq!(serialize(&d), "\\---\n");
    }

    #[test]
    fn parse_edge_cases() {
        let np = |t: &str| para(vec![plain(t)]);

        // `#5` is a paragraph (no space after #).
        assert_eq!(parse(Path::new("x"), "#5\n").body(), vec![np("#5")]);
        // `#` alone is a paragraph (no content).
        assert_eq!(parse(Path::new("x"), "#\n").body(), vec![np("#")]);
        // `#### x` is a level-4 heading.
        assert_eq!(
            parse(Path::new("x"), "#### x\n").body(),
            vec![Block::Heading {
                level: 4,
                folded: false,
                content: vec![plain("x")]
            }]
        );
        // `##### x` — too many hashes, a paragraph.
        assert_eq!(
            parse(Path::new("x"), "##### x\n").body(),
            vec![np("##### x")]
        );
        // No closer → literal.
        assert_eq!(
            parse(Path::new("x"), "*no close\n").body(),
            vec![np("*no close")]
        );
        assert_eq!(
            parse(Path::new("x"), "**no close\n").body(),
            vec![np("**no close")]
        );
        // Opening marker must be followed by non-space — but a line-leading
        // `* ` is now a bullet, so `* space*` reads as one item whose lone
        // trailing `*` is literal text.
        assert_eq!(
            parse(Path::new("x"), "* space*\n").body(),
            vec![item(ListMarker::Bullet, "space*")]
        );
        // Escaped literal star.
        assert_eq!(
            parse(Path::new("x"), "\\*literal\n").body(),
            vec![np("*literal")]
        );
        // No nesting: bold run with literal inner markers.
        assert_eq!(
            parse(Path::new("x"), "**a *b* c**\n").body(),
            vec![para(vec![bold("a *b* c")])]
        );
        // CRLF tolerance.
        assert_eq!(
            parse(Path::new("x"), "line one\r\nline two\r\n").body(),
            vec![np("line one line two")]
        );
        // Blank-line-only file → one empty paragraph.
        assert_eq!(
            parse(Path::new("x"), "\n\n\n").body(),
            vec![empty_paragraph()]
        );
        // Multi-line paragraph joins to one line.
        assert_eq!(
            parse(Path::new("x"), "a\nb\n\nc\n").body(),
            vec![np("a b"), np("c")]
        );
    }

    #[test]
    fn underscore_is_literal_text() {
        // `_` is not a marker: it must survive parse -> serialize -> parse
        // unchanged, e.g. SPEC §9's own math example `A = 1 + R_2 / R_1`.
        let d = doc_with(vec![para(vec![plain("A = 1 + R_2 / R_1")])]);
        let text = serialize(&d);
        assert_eq!(text, "A = 1 + R_2 / R_1\n");
        let back = parse(Path::new("x"), &text);
        assert_eq!(back.body(), d.body());
        // Second round-trip stays stable too.
        let back2 = parse(Path::new("x"), &serialize(&back));
        assert_eq!(back2.body(), d.body());
    }

    #[test]
    fn empty_heading_round_trip() {
        // A lone empty Heading must stay a Heading, not get demoted to a
        // Paragraph by the "empty document" fast path.
        let d = doc_with(vec![head(1, vec![plain("")])]);
        let text = serialize(&d);
        let back = parse(Path::new("x"), &text);
        assert_eq!(back.body(), d.body());
        assert!(matches!(back.body()[0], Block::Heading { level: 1, .. }));
    }

    #[test]
    fn empty_heading_followed_by_paragraph_round_trip() {
        // Exercises parse_heading's fix without the empty-document fast
        // path (two blocks, so that path never triggers).
        let d = doc_with(vec![head(2, vec![plain("")]), para(vec![plain("body")])]);
        let text = serialize(&d);
        let back = parse(Path::new("x"), &text);
        assert_eq!(back.body(), d.body());
    }

    #[test]
    fn save_load_round_trip_and_nul_rejection() {
        let mut path = std::env::temp_dir();
        path.push(format!("typewritter_md_test_{}.md", std::process::id()));

        let mut d = Document::new(&path);
        *d.body_mut() = vec![
            head(1, vec![plain("Title")]),
            para(vec![plain("Some "), bold("bold"), plain(" text")]),
        ];
        d.save().unwrap();
        assert!(!d.is_dirty());

        let loaded = Document::load(&path).unwrap();
        assert_eq!(loaded.body(), d.body());

        // NUL byte → refuse to load.
        std::fs::write(&path, b"a\x00b").unwrap();
        assert!(Document::load(&path).is_none());

        let _ = std::fs::remove_file(&path);
    }

    // ---- badge and highlight tests ---------------------------------------

    #[test]
    fn a_badge_is_a_verbatim_label_that_round_trips() {
        let text = "[[PS]] the same divider shows up next week\n";
        let d = parse(Path::new("n.md"), text);
        let runs = d.body()[0].inlines();
        assert!(runs[0].style().badge);
        assert_eq!(runs[0].text(), "PS");
        assert!(runs[1].style().is_plain());
        assert_eq!(serialize(&d), text);
    }

    #[test]
    fn coloured_badges_round_trip_while_plain_markers_stay_orange() {
        let text = "[[TODO]] [[blue|INFO]] [[green|DONE]] [[purple|IDEA]]\n";
        let d = parse(Path::new("n.md"), text);
        let colors: Vec<_> = d.body()[0]
            .inlines()
            .iter()
            .filter(|run| run.style().badge)
            .map(|run| run.style().badge_color)
            .collect();
        assert_eq!(
            colors,
            vec![
                BadgeColor::Orange,
                BadgeColor::Blue,
                BadgeColor::Green,
                BadgeColor::Purple,
            ]
        );
        assert_eq!(serialize(&d), text);
    }

    #[test]
    fn badge_markers_with_no_closer_or_no_label_are_literal() {
        for text in ["[[TODO but no closer\n", "[[]] empty\n", "a [ b [ c\n"] {
            let d = parse(Path::new("n.md"), text);
            assert!(
                d.body()[0].inlines().iter().all(|r| !r.style().badge),
                "{text:?} should not have produced a badge"
            );
            // Serializing re-escapes the literal marker, so compare the
            // model rather than the bytes: `[[` on disk is `\[[` once it
            // has been through the editor, exactly as a literal `*` is.
            let out = serialize(&d);
            assert_eq!(parse(Path::new("n.md"), &out).body(), d.body());
        }
    }

    #[test]
    fn a_highlight_nests_emphasis_rather_than_swallowing_it() {
        let text = "the ==**whole** story== here\n";
        let d = parse(Path::new("n.md"), text);
        let runs = d.body()[0].inlines();
        let marked: Vec<_> = runs.iter().filter(|r| r.style().highlight).collect();
        assert_eq!(marked.len(), 2, "`**whole**` and ` story` are both marked");
        assert!(marked[0].style().bold, "the emphasis survives the mark");
        assert!(!marked[1].style().bold);
        assert_eq!(serialize(&d), text);
    }

    #[test]
    fn a_highlight_never_lands_on_a_boxed_run() {
        // `is_boxed` styles draw their own box; a mark on top would fight
        // it, so the inner run keeps its own style and the mark is dropped.
        let d = parse(Path::new("n.md"), "==a `run` b==\n");
        for inline in d.body()[0].inlines() {
            assert!(!(inline.style().highlight && inline.style().is_boxed()));
        }
    }

    #[test]
    fn doubled_markers_in_prose_are_escaped_on_the_way_out() {
        // Only the doubled forms are markers, so a lone `=` or `[` stays
        // bare — a note full of `[Q12]` must not turn into backslash soup.
        let d = doc_with(vec![para(vec![plain("x [[y]] z == w [q] a = b")])]);
        let out = serialize(&d);
        assert_eq!(out, "x \\[[y]] z \\== w [q] a = b\n");
        assert_eq!(parse(Path::new("n.md"), &out).body(), d.body());
    }

    // ---- inline code span tests -----------------------------------------

    #[test]
    fn inline_code_span_round_trip() {
        let text = "the `foo()` call\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(
            d.body(),
            vec![para(vec![plain("the "), code("foo()"), plain(" call")])]
        );
        assert!(d.body()[0].inlines()[1].style().code);
        let back = serialize(&d);
        assert_eq!(back, "the `foo()` call\n");
    }

    #[test]
    fn inline_code_span_and_asterisks() {
        // `*` inside a code span is literal, not emphasis.
        let text = "`a*b*c`\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 1);
        assert_eq!(d.body()[0].inlines().len(), 1);
        assert!(d.body()[0].inlines()[0].style().code);
        assert_eq!(d.body()[0].inlines()[0].text(), "a*b*c");
        // Round-trip.
        let back = serialize(&d);
        assert_eq!(back, "`a*b*c`\n");
    }

    #[test]
    fn inline_code_span_no_closer_is_literal() {
        let text = "`no close\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body(), vec![para(vec![plain("`no close")])]);
    }

    #[test]
    fn inline_code_span_adjacent_backticks_empty_span() {
        let text = "``\n";
        let d = parse(Path::new("x"), text);
        // Two adjacent backticks: empty code span. Produces an empty
        // code-styled run, which survives as a valid (empty) run.
        assert_eq!(d.body().len(), 1);
        assert_eq!(d.body()[0].inlines().len(), 1);
        assert!(d.body()[0].inlines()[0].style().code);
    }

    // ---- literal backtick in plain text --------------------------------

    #[test]
    fn literal_backtick_in_plain_text_round_trips() {
        let d = doc_with(vec![para(vec![plain("use the ` key")])]);
        let text = serialize(&d);
        // The literal backtick must be escaped on save to avoid being
        // misread as a code-span opener on reload.
        assert_eq!(text, "use the \\` key\n");
        let back = parse(Path::new("x"), &text);
        assert_eq!(back.body(), d.body());
    }

    // ---- fenced code block tests ---------------------------------------

    #[test]
    fn fenced_code_block_three_lines() {
        let text = "```\nline one\nline two\nline three\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 3);
        for (i, block) in d.body().iter().enumerate() {
            assert!(block.is_code(), "block {i} should be CodeLine");
        }
        assert_eq!(text_of_block(&d, 0), "line one");
        assert_eq!(text_of_block(&d, 1), "line two");
        assert_eq!(text_of_block(&d, 2), "line three");
        // Round-trip.
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    fn text_of_block(d: &Document, block: usize) -> String {
        d.body()[block].inlines().iter().map(Inline::text).collect()
    }

    #[test]
    fn fenced_code_contains_markdown_special_chars_verbatim() {
        let text = "```\n# not a heading\n**not bold**\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 2);
        assert_eq!(text_of_block(&d, 0), "# not a heading");
        assert_eq!(text_of_block(&d, 1), "**not bold**");
        // Round-trip.
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_empty_fence_round_trips() {
        let text = "```\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 1);
        assert!(d.body()[0].is_code());
        assert_eq!(d.body()[0].inlines().len(), 1);
        assert!(d.body()[0].inlines()[0].text().is_empty());
        // Round-trip: empty code line serializes as an empty line inside
        // the fence.
        let back = serialize(&d);
        assert_eq!(back, "```\n\n```\n");
    }

    #[test]
    fn fenced_code_paragraph_code_paragraph_round_trip() {
        let text = "intro\n\n```\ncode a\ncode b\n```\n\noutro\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 4);
        assert!(!d.body()[0].is_code());
        assert_eq!(text_of_block(&d, 0), "intro");
        assert!(d.body()[1].is_code());
        assert_eq!(text_of_block(&d, 1), "code a");
        assert!(d.body()[2].is_code());
        assert_eq!(text_of_block(&d, 2), "code b");
        assert!(!d.body()[3].is_code());
        assert_eq!(text_of_block(&d, 3), "outro");
        // Round-trip: exactly one newline sep before and after the group.
        let back = serialize(&d);
        assert_eq!(back, "intro\n\n```\ncode a\ncode b\n```\n\noutro\n");
    }

    #[test]
    fn fenced_code_unterminated_parses_without_panic() {
        let text = "before\n```\nline one\nline two\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 3);
        assert!(!d.body()[0].is_code());
        assert!(d.body()[1].is_code());
        assert!(d.body()[2].is_code());
        assert_eq!(text_of_block(&d, 1), "line one");
        assert_eq!(text_of_block(&d, 2), "line two");
    }

    #[test]
    fn fenced_code_adjacent_fences_stay_separate() {
        let text = "```\na\n```\n\n```\nb\nc\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 3);
        assert!(d.body().iter().all(Block::is_code));
        // First fence: one line, first block has first:true.
        assert!(matches!(
            d.body()[0],
            Block::CodeLine {
                first: true,
                lang: None,
                ..
            }
        ));
        assert_eq!(text_of_block(&d, 0), "a");
        // Second fence: starts a new group, first block has first:true.
        assert!(matches!(
            d.body()[1],
            Block::CodeLine {
                first: true,
                lang: None,
                ..
            }
        ));
        assert_eq!(text_of_block(&d, 1), "b");
        // Continuation of second fence: first:false.
        assert!(matches!(
            d.body()[2],
            Block::CodeLine {
                first: false,
                lang: None,
                ..
            }
        ));
        assert_eq!(text_of_block(&d, 2), "c");
        // Full round-trip: serialize reproduces the exact original text.
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_blank_lines_inside_are_kept() {
        let text = "```\n\n\n\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 3);
        assert!(d.body()[0].is_code());
        assert!(d.body()[0].inlines()[0].text().is_empty());
        assert!(d.body()[1].is_code());
        assert!(d.body()[1].inlines()[0].text().is_empty());
        assert!(d.body()[2].is_code());
        assert!(d.body()[2].inlines()[0].text().is_empty());
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_tagged_opener() {
        let text = "prose before\n```lua\ncode line\n```\nprose after\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 3);
        assert!(!d.body()[0].is_code());
        assert_eq!(text_of_block(&d, 0), "prose before");
        assert!(d.body()[1].is_code());
        assert_eq!(text_of_block(&d, 1), "code line");
        assert!(!d.body()[2].is_code());
        assert_eq!(text_of_block(&d, 2), "prose after");
    }

    #[test]
    fn fenced_code_tagged_language_round_trips() {
        let text = "prose\n\n```lua\ncode\n```\n\nmore\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 3);
        assert!(!d.body()[0].is_code());
        assert_eq!(text_of_block(&d, 0), "prose");
        assert!(matches!(
            d.body()[1],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "lua"
        ));
        assert_eq!(text_of_block(&d, 1), "code");
        assert!(!d.body()[2].is_code());
        assert_eq!(text_of_block(&d, 2), "more");
        // Full round-trip: tag survives serialize.
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_untagged_bare_fence_round_trips() {
        let text = "```\ncode\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 1);
        assert!(matches!(
            d.body()[0],
            Block::CodeLine {
                first: true,
                lang: None,
                ..
            }
        ));
        assert_eq!(text_of_block(&d, 0), "code");
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_empty_tagged_fence_round_trips() {
        let text = "```lua\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 1);
        assert!(matches!(
            d.body()[0],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "lua"
        ));
        assert!(d.body()[0].inlines()[0].text().is_empty());
        // Empty code line serializes as an empty line inside the fence.
        let back = serialize(&d);
        assert_eq!(back, "```lua\n\n```\n");
    }

    #[test]
    fn fenced_code_two_tagged_fences_different_langs() {
        let text = "```lua\na\n```\n\n```rust\nb\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.body().len(), 2);
        assert!(matches!(
            d.body()[0],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "lua"
        ));
        assert_eq!(text_of_block(&d, 0), "a");
        assert!(matches!(
            d.body()[1],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "rust"
        ));
        assert_eq!(text_of_block(&d, 1), "b");
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    // ---- sidenote (footnote) tests --------------------------------------

    #[test]
    fn a_sidenote_round_trips_through_its_footnote_syntax() {
        let text = "The gain is stable[^1] across the band.\n\n[^1]: measured 10.94 at 1 kHz, bench rig B\n";
        let d = parse(Path::new("n.md"), text);
        let runs = d.body()[0].inlines();
        assert!(matches!(runs[1], Inline::Note(ref l) if l == "1"));
        assert_eq!(d.notes.len(), 1);
        assert_eq!(d.notes[0].label, "1");
        assert!(d.notes[0].anchored);
        assert_eq!(
            d.notes[0].body,
            vec![para(vec![plain("measured 10.94 at 1 kHz, bench rig B")])]
        );
        assert_eq!(serialize(&d), text);
    }

    #[test]
    fn definitions_are_written_in_anchor_order() {
        let mut d = doc_with(vec![para(vec![
            plain("first "),
            Inline::Note("2".into()),
            plain(" then "),
            Inline::Note("1".into()),
            plain(" done"),
        ])]);
        d.notes = vec![
            Sidenote {
                label: "1".into(),
                body: vec![para(vec![plain("one")])],
                anchored: true,
            },
            Sidenote {
                label: "2".into(),
                body: vec![para(vec![plain("two")])],
                anchored: true,
            },
        ];
        // The anchors appear "2" then "1" in the prose, so the definitions
        // are written in that order, not the notes' stored order.
        assert_eq!(
            serialize(&d),
            "first [^2] then [^1] done\n\n[^2]: two\n[^1]: one\n"
        );
    }

    #[test]
    fn an_anchor_with_no_definition_survives_a_round_trip() {
        let text = "A note[^1] with no body\n";
        let d = parse(Path::new("n.md"), text);
        assert!(matches!(d.body()[0].inlines()[1], Inline::Note(ref l) if l == "1"));
        assert!(d.notes.is_empty());
        let out = serialize(&d);
        assert_eq!(out, text);
        let back = parse(Path::new("n.md"), &out);
        assert!(matches!(back.body()[0].inlines()[1], Inline::Note(ref l) if l == "1"));
    }

    #[test]
    fn a_definition_with_no_anchor_survives_a_round_trip() {
        let text = "body\n\n[^1]: a stray note\n";
        let d = parse(Path::new("n.md"), text);
        assert_eq!(d.notes.len(), 1);
        assert!(!d.notes[0].anchored);
        assert_eq!(d.notes[0].label, "1");
        assert_eq!(d.notes[0].body, vec![para(vec![plain("a stray note")])]);
        let out = serialize(&d);
        assert_eq!(out, text);
        let back = parse(Path::new("n.md"), &out);
        assert_eq!(back.notes.len(), 1);
        assert!(!back.notes[0].anchored);
    }

    #[test]
    fn a_note_body_keeps_its_emphasis_and_its_expression() {
        let text = "text[^1]\n\n[^1]: the **gain** is $a/b$ here\n";
        let d = parse(Path::new("n.md"), text);
        assert_eq!(
            d.notes[0].body,
            vec![para(vec![
                plain("the "),
                bold("gain"),
                plain(" is "),
                math("a/b"),
                plain(" here"),
            ])]
        );
        assert_eq!(serialize(&d), text);
    }

    // ---- equation tags and references ------------------------------------

    #[test]
    fn a_tagged_math_block_round_trips_through_its_fence() {
        let text = "```tw-math v1 #eq:gain\n1/2\n```\n";
        let d = parse(Path::new("x"), text);
        assert!(matches!(
            d.body()[0],
            Block::Math {
                tag: Some(ref tag),
                ..
            } if tag == "eq:gain"
        ));
        assert_eq!(serialize(&d), text);
        assert_eq!(parse(Path::new("x"), &serialize(&d)).body(), d.body());
    }

    #[test]
    fn a_tag_stays_put_through_the_editor_round_trip() {
        let d = doc_with(vec![
            Block::Math {
                list: vec![math("1/2")],
                tag: Some("eq:gain".into()),
            },
            para(vec![
                plain("see "),
                Inline::EqRef("eq:gain".into()),
                plain(" here"),
            ]),
        ]);
        let text = serialize(&d);
        assert!(text.starts_with("```tw-math v1 #eq:gain\n"), "got {text:?}");
        assert!(text.contains("@eq:gain"), "got {text:?}");
        assert_eq!(parse(Path::new("x"), &text).body(), d.body());
    }

    #[test]
    fn an_equation_reference_round_trips_through_its_at_spelling() {
        let text = "by @eq:gain above\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(
            d.body(),
            vec![para(vec![
                plain("by "),
                Inline::EqRef("eq:gain".into()),
                plain(" above")
            ])]
        );
        assert_eq!(serialize(&d), text);
        assert_eq!(parse(Path::new("x"), &serialize(&d)).body(), d.body());
    }

    #[test]
    fn an_at_sign_that_is_not_a_reference_is_literal() {
        for text in [
            "@eq: no space name\n", // `@eq:` alone then space
            "@user mentioned\n",    // wrong keyword
            "@ eq:gain spaced\n",   // space after @
            "a lone @\n",           // bare @
            "@eq:\n",               // no name at all
        ] {
            let d = parse(Path::new("x"), text);
            assert!(
                d.body()[0]
                    .inlines()
                    .iter()
                    .all(|r| !matches!(r, Inline::EqRef(_))),
                "{text:?} should not have produced a reference"
            );
            let out = serialize(&d);
            assert_eq!(parse(Path::new("x"), &out).body(), d.body(), "{text:?}");
        }
        // A reference may carry digits, underscores, and dashes.
        let d = parse(Path::new("x"), "@eq:Gain_2-x\n");
        assert!(matches!(
            d.body()[0].inlines()[0],
            Inline::EqRef(ref l) if l == "eq:Gain_2-x"
        ));
    }

    #[test]
    fn a_fence_with_an_unknown_version_never_becomes_math() {
        // A tag on an unknown version does not rescue it into math.
        let d = parse(Path::new("x"), "```tw-math v2 #eq:gain\nx\n```\n");
        assert!(d.body()[0].is_code());
        assert_eq!(serialize(&d), "```tw-math v2 #eq:gain\nx\n```\n");
    }

    #[test]
    fn a_tag_with_an_illegal_name_is_dropped_not_error() {
        let d = parse(Path::new("x"), "```tw-math v1 #eq:bad!name\nx\n```\n");
        assert!(matches!(d.body()[0], Block::Math { tag: None, .. }));
        // Serializing writes the canonical spelling: no tag.
        assert_eq!(serialize(&d), "```tw-math v1\nx\n```\n");
    }

    // ---- list item tests -------------------------------------------------

    fn item(marker: ListMarker, t: &str) -> Block {
        Block::ListItem {
            marker,
            content: vec![plain(t)],
        }
    }

    /// Round trip is mandatory for every new spelling: parse then serialize
    /// returns the same blocks, serialize is byte-stable, and the tolerant
    /// variants all land on the one canonical spelling.
    #[test]
    fn list_spellings_parse_and_round_trip() {
        let cases = [
            // (source, expected marker of block 0, expected content)
            ("- tea\n", ListMarker::Bullet, "tea"),
            ("* tea\n", ListMarker::Bullet, "tea"),
            ("+ tea\n", ListMarker::Bullet, "tea"),
            (
                "- [ ] buy milk\n",
                ListMarker::Task { done: false },
                "buy milk",
            ),
            (
                "* [ ] buy milk\n",
                ListMarker::Task { done: false },
                "buy milk",
            ),
            (
                "- [x] buy milk\n",
                ListMarker::Task { done: true },
                "buy milk",
            ),
            (
                "- [X] buy milk\n",
                ListMarker::Task { done: true },
                "buy milk",
            ),
            ("1. tea\n", ListMarker::Number(1), "tea"),
            ("12) tea\n", ListMarker::Number(1), "tea"),
            ("- [ ]\n", ListMarker::Task { done: false }, ""),
            ("1. \n", ListMarker::Number(1), ""),
        ];
        for (source, marker, content) in cases {
            let d = parse(Path::new("x"), source);
            assert_eq!(
                d.body(),
                vec![item(marker, content)],
                "parse failed for {source:?}"
            );
            let text = serialize(&d);
            assert_eq!(
                parse(Path::new("x"), &text).body(),
                d.body(),
                "round-trip failed for {source:?}"
            );
            assert_eq!(
                serialize(&parse(Path::new("x"), &text)),
                text,
                "canonical stability failed for {source:?}"
            );
        }
    }

    #[test]
    fn list_markers_serialize_to_one_spelling() {
        for source in ["* tea\n", "+ tea\n"] {
            assert_eq!(serialize(&parse(Path::new("x"), source)), "- tea\n");
        }
        assert_eq!(
            serialize(&parse(Path::new("x"), "- [X] done\n")),
            "- [x] done\n"
        );
        // The typed ordinal is never stored: the run is renumbered.
        let d = parse(Path::new("x"), "5. a\n6. b\n");
        assert_eq!(
            d.body(),
            vec![
                item(ListMarker::Number(1), "a"),
                item(ListMarker::Number(2), "b"),
            ]
        );
        assert_eq!(serialize(&d), "1. a\n\n2. b\n");
    }

    #[test]
    fn an_ordered_run_restarts_after_a_non_list_block() {
        let d = parse(Path::new("x"), "1. a\n\npara\n\n2. b\n");
        assert_eq!(
            d.body(),
            vec![
                item(ListMarker::Number(1), "a"),
                para(vec![plain("para")]),
                item(ListMarker::Number(1), "b"),
            ]
        );
        assert_eq!(serialize(&d), "1. a\n\npara\n\n1. b\n");
    }

    #[test]
    fn a_bullet_or_task_item_ends_an_ordered_run() {
        let d = parse(Path::new("x"), "1. a\n- b\n2. c\n");
        assert_eq!(
            d.body(),
            vec![
                item(ListMarker::Number(1), "a"),
                item(ListMarker::Bullet, "b"),
                item(ListMarker::Number(1), "c"),
            ]
        );
    }

    #[test]
    fn list_content_keeps_its_inline_grammar() {
        let d = parse(Path::new("x"), "- **bold** step and $a/b$\n");
        assert_eq!(
            d.body(),
            vec![Block::ListItem {
                marker: ListMarker::Bullet,
                content: vec![bold("bold"), plain(" step and "), math("a/b")],
            }]
        );
        assert_eq!(serialize(&d), "- **bold** step and $a/b$\n");
    }

    #[test]
    fn list_like_paragraphs_are_escaped_on_the_way_out() {
        // A paragraph whose text reads as a list item gets a leading
        // escape, exactly like the `#` and `---` guards.
        let d = doc_with(vec![para(vec![plain("- tea")])]);
        let out = serialize(&d);
        assert_eq!(out, "\\- tea\n");
        assert_eq!(parse(Path::new("x"), &out).body(), d.body());

        let d = doc_with(vec![para(vec![plain("1) note")])]);
        let out = serialize(&d);
        assert_eq!(out, "\\1) note\n");
        assert_eq!(parse(Path::new("x"), &out).body(), d.body());
    }

    #[test]
    fn a_bullet_whose_content_reads_as_a_task_box_is_escaped() {
        let d = doc_with(vec![item(ListMarker::Bullet, "[ ] not a task")]);
        let out = serialize(&d);
        assert_eq!(out, "- \\[ ] not a task\n");
        assert_eq!(parse(Path::new("x"), &out).body(), d.body());
    }

    #[test]
    fn a_list_line_does_not_join_a_paragraph() {
        let d = parse(Path::new("x"), "intro\n- tea\n\noutro\n");
        assert_eq!(
            d.body(),
            vec![
                para(vec![plain("intro")]),
                item(ListMarker::Bullet, "tea"),
                para(vec![plain("outro")]),
            ]
        );
    }

    #[test]
    fn a_bracket_that_is_not_a_footnote_is_ordinary_text() {
        // `[^` with no closing bracket is ordinary text.
        let d = parse(Path::new("n.md"), "see [^ the thing\n");
        assert_eq!(d.body(), vec![para(vec![plain("see [^ the thing")])]);
        // A plain `[link]` is text, not a footnote.
        let d = parse(Path::new("n.md"), "a [link] here\n");
        assert_eq!(d.body(), vec![para(vec![plain("a [link] here")])]);
        // A literal `[^1]` in prose is escaped on the way out so it re-reads
        // as text rather than as an anchor.
        let d = doc_with(vec![para(vec![plain("see [^1] done")])]);
        let out = serialize(&d);
        assert_eq!(out, "see \\[^1] done\n");
        assert_eq!(parse(Path::new("n.md"), &out).body(), d.body());
    }
}
