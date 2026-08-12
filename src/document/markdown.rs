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

use super::{Block, Caret, Document, Inline, Style, Text};

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
                            let Inline::Text(mut t) = run;
                            t.style.highlight = !t.style.is_boxed();
                            runs.push(Inline::Text(t));
                        }
                        i = cl + 2;
                    }
                    None => {
                        text_buf.push_str("==");
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
                        runs.push(Inline::Text(Text {
                            text: chars[after..cl].iter().collect(),
                            style: Style {
                                badge: true,
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

/// Build a `Document` from Markdown text. `path` only seeds `Document`'s
/// `path`/`name` fields. Pure — no IO.
pub fn parse(path: &Path, text: &str) -> Document {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut fence_had_lines = false;
    let mut fence_lang: Option<String> = None;

    let flush_para = |blocks: &mut Vec<Block>, para: &mut Vec<String>| {
        if !para.is_empty() {
            let joined = para.join(" ");
            blocks.push(Block::Paragraph(parse_inline(&joined)));
            para.clear();
        }
    };

    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if in_fence {
            if line.trim() == "```" {
                if !fence_had_lines {
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
            continue;
        }
        if line.trim().starts_with("```") {
            flush_para(&mut blocks, &mut para);
            let info = line.trim()[3..].trim();
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
                content: inlines,
            });
        } else if line.trim().is_empty() {
            flush_para(&mut blocks, &mut para);
        } else {
            para.push(line.to_string());
        }
    }
    flush_para(&mut blocks, &mut para);

    if in_fence {
        // Unterminated fence: everything after opener is already code lines.
        // The closing fence block is implicit at EOF; nothing extra to do.
    }

    if blocks.is_empty() {
        blocks.push(empty_paragraph());
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());

    Document {
        blocks,
        path: path.to_path_buf(),
        name,
        dirty: false,
        caret: Caret {
            block: 0,
            inline: 0,
            offset: 0,
            style: Style::PLAIN,
        },
    }
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
            // Only the doubled forms are markers, so only those need
            // escaping — prose is full of lone `=` and `[`, and escaping
            // every one of them would make the file unreadable on disk.
            c @ ('=' | '[') if chars.get(i + 1) == Some(&c) => {
                out.push('\\');
                out.push(c);
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
        format!("[[{escaped}]]")
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
    let one = |style: Style, text: &str| {
        if style.is_boxed() {
            wrap_run(style, text)
        } else {
            wrap_run(style, &escape_run_text(text))
        }
    };
    let mut out = String::new();
    let mut i = 0;
    while i < runs.len() {
        let Inline::Text(t) = &runs[i];
        if !t.style.highlight {
            out.push_str(&one(t.style, &t.text));
            i += 1;
            continue;
        }
        let mut end = i;
        while runs.get(end + 1).is_some_and(|r| r.style().highlight) {
            end += 1;
        }
        out.push_str("==");
        for run in &runs[i..=end] {
            let Inline::Text(t) = run;
            out.push_str(&one(
                Style {
                    highlight: false,
                    ..t.style
                },
                &t.text,
            ));
        }
        out.push_str("==");
        i = end + 1;
    }
    out
}

/// Write a document back out, deterministically.
pub fn serialize(doc: &Document) -> String {
    // Empty document (one empty paragraph) serializes to ""; "" parses
    // back to one empty paragraph, so "" is the fixpoint. An empty
    // Heading must NOT take this path — it would lose its heading-ness.
    let empty = doc.blocks.len() == 1 && matches!(doc.blocks[0], Block::Paragraph(_)) && {
        let runs = doc.blocks[0].inlines();
        runs.len() == 1 && matches!(runs[0], Inline::Text(Text { ref text, .. }) if text.is_empty())
    };
    if empty {
        return String::new();
    }

    let mut out = String::new();
    let mut i = 0;
    let mut first = true;
    while i < doc.blocks.len() {
        if !first {
            out.push_str("\n\n");
        }
        first = false;
        match &doc.blocks[i] {
            Block::CodeLine { lang, .. } => {
                let opener = match lang {
                    Some(l) => format!("```{l}\n"),
                    None => "```\n".to_string(),
                };
                out.push_str(&opener);
                let mut j = i;
                loop {
                    let line: String = doc.blocks[j].inlines().iter().map(Inline::text).collect();
                    out.push_str(&line);
                    out.push('\n');
                    j += 1;
                    match doc.blocks.get(j) {
                        Some(Block::CodeLine { first: false, .. }) => continue,
                        _ => break,
                    }
                }
                out.push_str("```");
                i = j;
            }
            Block::Heading { level, content } => {
                out.push_str(&"#".repeat(*level as usize));
                out.push(' ');
                out.push_str(&serialize_runs(content));
                i += 1;
            }
            Block::Divider(_) => {
                out.push_str("---");
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
                out.push_str(&line);
                i += 1;
            }
        }
    }
    out.push('\n');
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
        d.blocks = blocks;
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

    fn para(runs: Vec<Inline>) -> Block {
        Block::Paragraph(runs)
    }

    fn head(level: u8, runs: Vec<Inline>) -> Block {
        Block::Heading {
            level,
            content: runs,
        }
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
            assert_eq!(back.blocks, d.blocks, "round-trip failed for: {text:?}");
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
    fn a_rule_round_trips() {
        let d = parse(Path::new("x"), "before\n\n---\n\nafter\n");
        assert!(d.blocks[1].is_divider());
        assert_eq!(serialize(&d), "before\n\n---\n\nafter\n");
        assert_eq!(parse(Path::new("x"), &serialize(&d)).blocks, d.blocks);
    }

    #[test]
    fn a_paragraph_of_dashes_is_not_a_rule() {
        let d = parse(Path::new("x"), "\\---\n");
        assert!(matches!(d.blocks[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "---");
        assert_eq!(serialize(&d), "\\---\n");
    }

    #[test]
    fn parse_edge_cases() {
        let np = |t: &str| para(vec![plain(t)]);

        // `#5` is a paragraph (no space after #).
        assert_eq!(parse(Path::new("x"), "#5\n").blocks, vec![np("#5")]);
        // `#` alone is a paragraph (no content).
        assert_eq!(parse(Path::new("x"), "#\n").blocks, vec![np("#")]);
        // `#### x` is a level-4 heading.
        assert_eq!(
            parse(Path::new("x"), "#### x\n").blocks,
            vec![Block::Heading {
                level: 4,
                content: vec![plain("x")]
            }]
        );
        // `##### x` — too many hashes, a paragraph.
        assert_eq!(
            parse(Path::new("x"), "##### x\n").blocks,
            vec![np("##### x")]
        );
        // No closer → literal.
        assert_eq!(
            parse(Path::new("x"), "*no close\n").blocks,
            vec![np("*no close")]
        );
        assert_eq!(
            parse(Path::new("x"), "**no close\n").blocks,
            vec![np("**no close")]
        );
        // Opening marker must be followed by non-space: `* space*` is plain
        // (a single literal `*`), and the trailing lone `*` is literal too.
        assert_eq!(
            parse(Path::new("x"), "* space*\n").blocks,
            vec![np("* space*")]
        );
        // Escaped literal star.
        assert_eq!(
            parse(Path::new("x"), "\\*literal\n").blocks,
            vec![np("*literal")]
        );
        // No nesting: bold run with literal inner markers.
        assert_eq!(
            parse(Path::new("x"), "**a *b* c**\n").blocks,
            vec![para(vec![bold("a *b* c")])]
        );
        // CRLF tolerance.
        assert_eq!(
            parse(Path::new("x"), "line one\r\nline two\r\n").blocks,
            vec![np("line one line two")]
        );
        // Blank-line-only file → one empty paragraph.
        assert_eq!(
            parse(Path::new("x"), "\n\n\n").blocks,
            vec![empty_paragraph()]
        );
        // Multi-line paragraph joins to one line.
        assert_eq!(
            parse(Path::new("x"), "a\nb\n\nc\n").blocks,
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
        assert_eq!(back.blocks, d.blocks);
        // Second round-trip stays stable too.
        let back2 = parse(Path::new("x"), &serialize(&back));
        assert_eq!(back2.blocks, d.blocks);
    }

    #[test]
    fn empty_heading_round_trip() {
        // A lone empty Heading must stay a Heading, not get demoted to a
        // Paragraph by the "empty document" fast path.
        let d = doc_with(vec![head(1, vec![plain("")])]);
        let text = serialize(&d);
        let back = parse(Path::new("x"), &text);
        assert_eq!(back.blocks, d.blocks);
        assert!(matches!(back.blocks[0], Block::Heading { level: 1, .. }));
    }

    #[test]
    fn empty_heading_followed_by_paragraph_round_trip() {
        // Exercises parse_heading's fix without the empty-document fast
        // path (two blocks, so that path never triggers).
        let d = doc_with(vec![head(2, vec![plain("")]), para(vec![plain("body")])]);
        let text = serialize(&d);
        let back = parse(Path::new("x"), &text);
        assert_eq!(back.blocks, d.blocks);
    }

    #[test]
    fn save_load_round_trip_and_nul_rejection() {
        let mut path = std::env::temp_dir();
        path.push(format!("typewritter_md_test_{}.md", std::process::id()));

        let mut d = Document::new(&path);
        d.blocks = vec![
            head(1, vec![plain("Title")]),
            para(vec![plain("Some "), bold("bold"), plain(" text")]),
        ];
        d.save().unwrap();
        assert!(!d.is_dirty());

        let loaded = Document::load(&path).unwrap();
        assert_eq!(loaded.blocks, d.blocks);

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
        let runs = d.blocks[0].inlines();
        assert!(runs[0].style().badge);
        assert_eq!(runs[0].text(), "PS");
        assert!(runs[1].style().is_plain());
        assert_eq!(serialize(&d), text);
    }

    #[test]
    fn badge_markers_with_no_closer_or_no_label_are_literal() {
        for text in ["[[TODO but no closer\n", "[[]] empty\n", "a [ b [ c\n"] {
            let d = parse(Path::new("n.md"), text);
            assert!(
                d.blocks[0].inlines().iter().all(|r| !r.style().badge),
                "{text:?} should not have produced a badge"
            );
            // Serializing re-escapes the literal marker, so compare the
            // model rather than the bytes: `[[` on disk is `\[[` once it
            // has been through the editor, exactly as a literal `*` is.
            let out = serialize(&d);
            assert_eq!(parse(Path::new("n.md"), &out).blocks, d.blocks);
        }
    }

    #[test]
    fn a_highlight_nests_emphasis_rather_than_swallowing_it() {
        let text = "the ==**whole** story== here\n";
        let d = parse(Path::new("n.md"), text);
        let runs = d.blocks[0].inlines();
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
        for inline in d.blocks[0].inlines() {
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
        assert_eq!(parse(Path::new("n.md"), &out).blocks, d.blocks);
    }

    // ---- inline code span tests -----------------------------------------

    #[test]
    fn inline_code_span_round_trip() {
        let text = "the `foo()` call\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(
            d.blocks,
            vec![para(vec![plain("the "), code("foo()"), plain(" call")])]
        );
        assert!(d.blocks[0].inlines()[1].style().code);
        let back = serialize(&d);
        assert_eq!(back, "the `foo()` call\n");
    }

    #[test]
    fn inline_code_span_and_asterisks() {
        // `*` inside a code span is literal, not emphasis.
        let text = "`a*b*c`\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.blocks[0].inlines().len(), 1);
        assert!(d.blocks[0].inlines()[0].style().code);
        assert_eq!(d.blocks[0].inlines()[0].text(), "a*b*c");
        // Round-trip.
        let back = serialize(&d);
        assert_eq!(back, "`a*b*c`\n");
    }

    #[test]
    fn inline_code_span_no_closer_is_literal() {
        let text = "`no close\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks, vec![para(vec![plain("`no close")])]);
    }

    #[test]
    fn inline_code_span_adjacent_backticks_empty_span() {
        let text = "``\n";
        let d = parse(Path::new("x"), text);
        // Two adjacent backticks: empty code span. Produces an empty
        // code-styled run, which survives as a valid (empty) run.
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.blocks[0].inlines().len(), 1);
        assert!(d.blocks[0].inlines()[0].style().code);
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
        assert_eq!(back.blocks, d.blocks);
    }

    // ---- fenced code block tests ---------------------------------------

    #[test]
    fn fenced_code_block_three_lines() {
        let text = "```\nline one\nline two\nline three\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 3);
        for (i, block) in d.blocks.iter().enumerate() {
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
        d.blocks[block].inlines().iter().map(Inline::text).collect()
    }

    #[test]
    fn fenced_code_contains_markdown_special_chars_verbatim() {
        let text = "```\n# not a heading\n**not bold**\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 2);
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
        assert_eq!(d.blocks.len(), 1);
        assert!(d.blocks[0].is_code());
        assert_eq!(d.blocks[0].inlines().len(), 1);
        assert!(d.blocks[0].inlines()[0].text().is_empty());
        // Round-trip: empty code line serializes as an empty line inside
        // the fence.
        let back = serialize(&d);
        assert_eq!(back, "```\n\n```\n");
    }

    #[test]
    fn fenced_code_paragraph_code_paragraph_round_trip() {
        let text = "intro\n\n```\ncode a\ncode b\n```\n\noutro\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 4);
        assert!(!d.blocks[0].is_code());
        assert_eq!(text_of_block(&d, 0), "intro");
        assert!(d.blocks[1].is_code());
        assert_eq!(text_of_block(&d, 1), "code a");
        assert!(d.blocks[2].is_code());
        assert_eq!(text_of_block(&d, 2), "code b");
        assert!(!d.blocks[3].is_code());
        assert_eq!(text_of_block(&d, 3), "outro");
        // Round-trip: exactly one newline sep before and after the group.
        let back = serialize(&d);
        assert_eq!(back, "intro\n\n```\ncode a\ncode b\n```\n\noutro\n");
    }

    #[test]
    fn fenced_code_unterminated_parses_without_panic() {
        let text = "before\n```\nline one\nline two\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 3);
        assert!(!d.blocks[0].is_code());
        assert!(d.blocks[1].is_code());
        assert!(d.blocks[2].is_code());
        assert_eq!(text_of_block(&d, 1), "line one");
        assert_eq!(text_of_block(&d, 2), "line two");
    }

    #[test]
    fn fenced_code_adjacent_fences_stay_separate() {
        let text = "```\na\n```\n\n```\nb\nc\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 3);
        assert!(d.blocks.iter().all(Block::is_code));
        // First fence: one line, first block has first:true.
        assert!(matches!(
            d.blocks[0],
            Block::CodeLine {
                first: true,
                lang: None,
                ..
            }
        ));
        assert_eq!(text_of_block(&d, 0), "a");
        // Second fence: starts a new group, first block has first:true.
        assert!(matches!(
            d.blocks[1],
            Block::CodeLine {
                first: true,
                lang: None,
                ..
            }
        ));
        assert_eq!(text_of_block(&d, 1), "b");
        // Continuation of second fence: first:false.
        assert!(matches!(
            d.blocks[2],
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
        assert_eq!(d.blocks.len(), 3);
        assert!(d.blocks[0].is_code());
        assert!(d.blocks[0].inlines()[0].text().is_empty());
        assert!(d.blocks[1].is_code());
        assert!(d.blocks[1].inlines()[0].text().is_empty());
        assert!(d.blocks[2].is_code());
        assert!(d.blocks[2].inlines()[0].text().is_empty());
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_tagged_opener() {
        let text = "prose before\n```lua\ncode line\n```\nprose after\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 3);
        assert!(!d.blocks[0].is_code());
        assert_eq!(text_of_block(&d, 0), "prose before");
        assert!(d.blocks[1].is_code());
        assert_eq!(text_of_block(&d, 1), "code line");
        assert!(!d.blocks[2].is_code());
        assert_eq!(text_of_block(&d, 2), "prose after");
    }

    #[test]
    fn fenced_code_tagged_language_round_trips() {
        let text = "prose\n\n```lua\ncode\n```\n\nmore\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 3);
        assert!(!d.blocks[0].is_code());
        assert_eq!(text_of_block(&d, 0), "prose");
        assert!(matches!(
            d.blocks[1],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "lua"
        ));
        assert_eq!(text_of_block(&d, 1), "code");
        assert!(!d.blocks[2].is_code());
        assert_eq!(text_of_block(&d, 2), "more");
        // Full round-trip: tag survives serialize.
        let back = serialize(&d);
        assert_eq!(back, text);
    }

    #[test]
    fn fenced_code_untagged_bare_fence_round_trips() {
        let text = "```\ncode\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 1);
        assert!(matches!(
            d.blocks[0],
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
        assert_eq!(d.blocks.len(), 1);
        assert!(matches!(
            d.blocks[0],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "lua"
        ));
        assert!(d.blocks[0].inlines()[0].text().is_empty());
        // Empty code line serializes as an empty line inside the fence.
        let back = serialize(&d);
        assert_eq!(back, "```lua\n\n```\n");
    }

    #[test]
    fn fenced_code_two_tagged_fences_different_langs() {
        let text = "```lua\na\n```\n\n```rust\nb\n```\n";
        let d = parse(Path::new("x"), text);
        assert_eq!(d.blocks.len(), 2);
        assert!(matches!(
            d.blocks[0],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "lua"
        ));
        assert_eq!(text_of_block(&d, 0), "a");
        assert!(matches!(
            d.blocks[1],
            Block::CodeLine { first: true, lang: Some(ref l), .. } if l == "rust"
        ));
        assert_eq!(text_of_block(&d, 1), "b");
        let back = serialize(&d);
        assert_eq!(back, text);
    }
}
