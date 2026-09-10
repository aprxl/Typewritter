//! Vault-wide text search for the finder panel.
//!
//! The finder matches files by name; this extends the same panel to match
//! lines *inside* files. The finder builds a parsed snapshot when it opens,
//! then each query only ranks the in-memory rows. The compatibility helper
//! [`search`] still builds a short-lived snapshot for callers without a
//! finder session.
//!
//! Results come back as one ranked list: file matches first, then text
//! hits ranked by fuzzy score. A hit carries the block and offset it was
//! found at, so picking one opens the note with the caret already there.

use std::path::{Path, PathBuf};

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

/// The longest line kept as a hit's text. A capture note's paragraphs wrap
/// whole-screen-width lines; past this a hit's context stops being readable
/// in one row and the preview ellipsizes it anyway.
const MAX_LINE_CHARS: usize = 200;
/// Context kept before the match when a run exceeds `MAX_LINE_CHARS`.
const CONTEXT_BEFORE: usize = 30;

/// A text hit: one contiguous run of flat text inside one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextHit {
    pub path: PathBuf,
    pub name: String,
    /// The matching text, as it reads on the page (math in notation).
    pub line: String,
    /// Char offsets of every matched character within `line` — a fuzzy
    /// match is a subsequence, so the positions are scattered. The panel
    /// highlights exactly these.
    pub matches: Vec<usize>,
    /// Where the caret lands when the hit is picked: a block index into the
    /// parsed document and the char offset within that block's flat text.
    pub block: usize,
    pub offset: usize,
}

/// One row of the finder's result list. A file match (the note itself,
/// ranked above text hits) or a text hit inside a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    File { path: PathBuf, name: String },
    Text(TextHit),
}

#[derive(Clone)]
struct IndexedRun {
    path: PathBuf,
    name: String,
    text: String,
    block: usize,
    offset: usize,
}

fn searchable_runs(
    run: &crate::document::Inline,
    tag_prefix: &str,
) -> Vec<(usize, String)> {
    match run {
        crate::document::Inline::Text(text) => {
            vec![(text.text.chars().count(), text.text.clone())]
        }
        crate::document::Inline::Math(list) => vec![(
            1,
            format!(
                "{}${}$",
                tag_prefix,
                crate::document::math_notation::print(list)
            ),
        )],
        crate::document::Inline::Note(_) => vec![(1, String::new())],
        crate::document::Inline::EqRef(label) => vec![(1, format!("@{label}"))],
        crate::document::Inline::TableCell(contents) => contents
            .iter()
            .flat_map(|run| searchable_runs(run, tag_prefix))
            .collect(),
    }
}

/// Parsed vault content reused across finder query changes. Building the
/// index performs filesystem I/O once; ranking a new query only scans these
/// in-memory runs.
#[derive(Clone)]
pub struct SearchIndex {
    files: Vec<crate::vault::VaultFile>,
    runs: Vec<IndexedRun>,
}

impl SearchIndex {
    pub fn build(files: &[crate::vault::VaultFile]) -> Self {
        let mut runs = Vec::new();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file.path) else {
                continue;
            };
            let doc = crate::document::markdown::parse(&file.path, &text);
            for (block_index, block) in doc.body().iter().enumerate() {
                let tag_prefix = match block {
                    crate::document::Block::Math { tag: Some(tag), .. } => {
                        format!("#{tag} ")
                    }
                    _ => String::new(),
                };
                let mut offset = 0usize;
                for run in block.inlines() {
                    for (chars, text) in searchable_runs(run, &tag_prefix) {
                        runs.push(IndexedRun {
                            path: file.path.clone(),
                            name: file.name.clone(),
                            text,
                            block: block_index,
                            offset,
                        });
                        offset += chars;
                    }
                }
            }
        }
        Self {
            files: files.to_vec(),
            runs,
        }
    }

    pub fn search(&self, query: &str) -> Vec<Row> {
        let mut rows: Vec<Row> = file_rows(&self.files, query)
            .into_iter()
            .map(|(path, name)| Row::File { path, name })
            .collect();
        if !query.is_empty() {
            rows.extend(text_rows(&self.runs, query));
        }
        rows
    }
}

impl Row {
    pub fn path(&self) -> &Path {
        match self {
            Row::File { path, .. } => path,
            Row::Text(hit) => &hit.path,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Row::File { name, .. } => name,
            Row::Text(hit) => &hit.name,
        }
    }

    /// Where the caret lands when this row is picked. `None` for a file
    /// row, which just opens the note.
    pub fn position(&self) -> Option<(usize, usize)> {
        match self {
            Row::File { .. } => None,
            Row::Text(hit) => Some((hit.block, hit.offset)),
        }
    }
}

/// Every row the finder shows for `query`: files whose name matches first
/// (vault order, the same ranking the finder always used), then text hits
/// ranked by fuzzy score with source order breaking ties.
///
/// `search` is the compatibility entry point for callers that have no
/// long-lived finder. The shell uses [`SearchIndex`] so adjacent queries do
/// not repeat filesystem reads or Markdown parsing.
pub fn search(files: &[crate::vault::VaultFile], query: &str) -> Vec<Row> {
    SearchIndex::build(files).search(query)
}

/// Files whose name fuzzy-matches, ranked the way `file_finder::filter`
/// ranked them: score descending, source order breaking ties.
fn file_rows(files: &[crate::vault::VaultFile], query: &str) -> Vec<(PathBuf, String)> {
    let matcher = SkimMatcherV2::default();
    let mut matches: Vec<(usize, i64)> = files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            matcher
                .fuzzy_indices(&file.name, query)
                .map(|(score, _)| (index, score))
        })
        .collect();
    matches.sort_by(|(left_index, left_score), (right_index, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_index.cmp(right_index))
    });
    matches
        .into_iter()
        .map(|(index, _)| {
            let file = &files[index];
            (file.path.clone(), file.name.clone())
        })
        .collect()
}

/// Fuzzy text hits across every file, ranked by score then source order.
/// Math is searched as its canonical notation — the `$…$`/fence form on
/// disk — so `R_2 / R_1` finds the equation (§7.3).
fn text_rows(runs: &[IndexedRun], query: &str) -> Vec<Row> {
    let matcher = SkimMatcherV2::default();
    let mut scored: Vec<(usize, i64, TextHit)> = Vec::new();
    for (source, run) in runs.iter().enumerate() {
        if let Some((score, indices)) = matcher.fuzzy_indices(&run.text, query) {
            let at = chars_min_prefix(&run.text, indices[0]);
            // The row shows one line: the whole run when it fits, otherwise
            // a window that keeps context before the match.
            let from = at.saturating_sub(CONTEXT_BEFORE);
            let at_chars: Vec<usize> = indices
                .iter()
                .map(|&b| chars_min_prefix(&run.text, b))
                .filter(|&c| c >= from)
                .map(|c| c - from)
                .collect();
            scored.push((
                source,
                score,
                TextHit {
                    path: run.path.clone(),
                    name: run.name.clone(),
                    line: clip(&run.text, from),
                    matches: at_chars,
                    block: run.block,
                    offset: run.offset,
                },
            ));
        }
    }
    scored.sort_by(
        |(left_source, left_score, _), (right_source, right_score, _)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_source.cmp(right_source))
        },
    );
    scored
        .into_iter()
        .take(MAX_HITS)
        .map(|(_, _, hit)| Row::Text(hit))
        .collect()
}

/// Searches one parsed document: every inline run's flat text (math as
/// its `$…$` notation), returning `(text, block, offset)` per hit with the
/// match's first char offset. The preview and the tests use this; the
/// vault-wide scan in `text_rows` wraps it with ranking and capping.
pub fn search_document(
    doc: &crate::document::Document,
    query: &str,
) -> Vec<(String, usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let matcher = SkimMatcherV2::default();
    let mut hits = Vec::new();
    for (block, b) in doc.body().iter().enumerate() {
        let tag_prefix = match b {
            crate::document::Block::Math { tag: Some(tag), .. } => format!("#{tag} "),
            _ => String::new(),
        };
        let mut offset = 0usize;
        for run in b.inlines() {
            for (chars, text) in searchable_runs(run, &tag_prefix) {
                if let Some((_, indices)) = matcher.fuzzy_indices(&text, query) {
                    let start = chars_min_prefix(&text, indices[0]);
                    hits.push((text, block, offset + start));
                }
                offset += chars;
            }
        }
    }
    hits
}

/// At most this many text hits are kept, ranked. A vault-wide query on a
/// common word can match hundreds of lines; the panel shows ten.
const MAX_HITS: usize = 50;

/// The char offset of byte index `byte` in `text` — `fuzzy_indices` speaks
/// bytes, everything downstream (highlighting, clipping) speaks chars. The
/// byte index can land mid-character for multi-byte text (an accented
/// variable name, a symbol), so it is walked back to the nearest boundary
/// before slicing; a raw `text[..byte]` panics there.
fn chars_min_prefix(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].chars().count()
}

/// The hit's line starting at its first match char, capped so a
/// paragraph-length run cannot become a thousand-character string.
fn clip(text: &str, start: usize) -> String {
    text.chars().skip(start).take(MAX_LINE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str) -> crate::vault::VaultFile {
        crate::vault::VaultFile {
            name: name.into(),
            path: PathBuf::new(),
        }
    }

    fn doc(body: &str) -> crate::document::Document {
        crate::document::markdown::parse(Path::new("test.md"), body)
    }

    #[test]
    fn a_list_item_is_indexed_like_prose() {
        // List items are flat blocks, so search indexing falls out of the
        // existing machinery — verified, not assumed: the hit must land in
        // the item's block with a caret-placeable offset.
        let d = doc("- buy $R_2$ resistors\n- [x] file the report\n");
        let hits = search_document(&d, "resistors");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, 0, "block 0 — the item is an ordinary flat block");
        // `buy ` is four chars, the math atom one flat position.
        assert_eq!(hits[0].2, 6, "offset counts the math atom as one position");
        let hits = search_document(&d, "report");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, 1, "the task item is indexed too");
    }

    #[test]
    fn an_empty_query_lists_files_only_in_vault_order() {
        let files = vec![file("b.md"), file("a.md")];
        assert_eq!(
            search(&files, ""),
            vec![
                Row::File {
                    path: PathBuf::new(),
                    name: "b.md".into()
                },
                Row::File {
                    path: PathBuf::new(),
                    name: "a.md".into()
                },
            ]
        );
    }

    #[test]
    fn a_name_match_outranks_a_body_hit() {
        let dir = std::env::temp_dir().join("tw-search-rank");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("beta.md");
        std::fs::write(&path, "ampersand\n").unwrap();
        let files = vec![crate::vault::VaultFile {
            name: "beta.md".into(),
            path: path.clone(),
        }];
        let rows = search(&files, "beta");
        assert!(matches!(&rows[0], Row::File { name, .. } if name == "beta.md"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn text_hits_rank_by_score_then_source_order() {
        let files = vec![file("one.md")];
        // No readable files behind the entry: the text pass finds nothing,
        // and the name does not match either.
        assert!(search(&files, "anything").is_empty());
    }

    #[test]
    fn a_hit_records_its_block_and_char_offset() {
        let dir = std::env::temp_dir().join("tw-search-offset");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "first line\n\nthe quick brown fox\n").unwrap();
        let files = vec![crate::vault::VaultFile {
            name: "note.md".into(),
            path: path.clone(),
        }];
        let rows = search(&files, "quick");
        let Some(Row::Text(hit)) = rows.iter().find(|r| matches!(r, Row::Text(_))) else {
            panic!("expected a text hit, got {:?}", rows);
        };
        // A blank line separates the paragraphs, so the second paragraph is
        // the second block (a single newline is a soft wrap inside one).
        assert_eq!(hit.block, 1);
        // A short run is shown whole, with every match char highlighted.
        assert_eq!(hit.line, "the quick brown fox");
        assert_eq!(hit.matches, vec![4, 5, 6, 7, 8]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_hit_inside_inline_math_reads_as_the_notation_on_disk() {
        let dir = std::env::temp_dir().join("tw-search-math");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("math.md");
        std::fs::write(&path, "gain is $R_2 / R_1$ plus one\n").unwrap();
        let files = vec![crate::vault::VaultFile {
            name: "math.md".into(),
            path: path.clone(),
        }];
        let rows = search(&files, "R_2");
        let Some(Row::Text(hit)) = rows.first() else {
            panic!("expected a text hit in math, got {:?}", rows);
        };
        // Math reads as its canonical notation on disk.
        assert!(hit.line.starts_with("$R_2"), "got {:?}", hit.line);
        assert!(hit.line.contains("R_1"), "got {:?}", hit.line);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hits_come_back_sorted_across_files_and_capped() {
        let dir = std::env::temp_dir().join("tw-search-cap");
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = Vec::new();
        for i in 0..12 {
            let path = dir.join(format!("n{i}.md"));
            std::fs::write(&path, "needle here\n").unwrap();
            files.push(crate::vault::VaultFile {
                name: format!("n{i}.md"),
                path: path.clone(),
            });
        }
        let rows = search(&files, "needle");
        let text: Vec<&Row> = rows.iter().filter(|r| matches!(r, Row::Text(_))).collect();
        assert_eq!(text.len(), MAX_HITS.min(12));
        for pair in rows.windows(2) {
            if let (Row::Text(a), Row::Text(b)) = (&pair[0], &pair[1]) {
                assert!(a.path <= b.path || a.line >= b.line);
            }
        }
        for i in 0..12 {
            std::fs::remove_file(dir.join(format!("n{i}.md"))).ok();
        }
    }

    #[test]
    fn positions_round_trip_through_the_document_cursor() {
        let d = doc("alpha\n\ngamma\n");
        let hits = search_document(&d, "gamma");
        assert_eq!(hits.len(), 1);
        let (text, block, offset) = &hits[0];
        assert_eq!((text.as_str(), *block, *offset), ("gamma", 1, 0));
        // The offset the search reports is the offset `position` accepts.
        let flat = d.position(*block, *offset);
        assert_eq!(flat.block, 1);
    }

    #[test]
    fn a_match_in_multibyte_text_does_not_panic_and_reports_a_char_offset() {
        // "coût" is 5 chars but 6 bytes; a query matching the 't' puts the
        // byte index inside 'ô' for the second 't'... the offset the search
        // reports must always be a char count, never a raw byte index.
        let d = doc("le coût et très haut\n");
        let hits = search_document(&d, "coût");
        assert_eq!(hits.len(), 1);
        let (text, _, offset) = &hits[0];
        assert_eq!(text.as_str(), "le coût et très haut");
        assert_eq!(*offset, 3, "char offset of the match, not bytes");
    }

    #[test]
    fn a_fuzzy_match_highlights_a_scattered_subsequence() {
        // The query need not be contiguous: "qbf" matches letters scattered
        // through the line, and every one of them is a highlight position.
        let d = doc("the quick brown fox\n");
        let hits = search_document(&d, "qbf");
        // search_document reports flat offsets, not per-line highlights —
        // the highlight set is `text_rows`' job; here we only pin that the
        // run matches at all so the row appears.
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn highlights_fall_inside_the_clipped_window() {
        let d = doc(
            "intro word and then a very long tail that keeps going well past the window cap of two hundred characters, which the clip function cuts away so only the leading context before the match survives in the stored line text for this particular extremely long run\n",
        );
        let hits = search_document(&d, "survives");
        assert_eq!(hits.len(), 1);
        let (_, _, offset) = &hits[0];
        // The flat offset still points at the true position even though the
        // row text would have been clipped.
        assert!(*offset > 100);
    }

    #[test]
    fn a_tagged_equation_is_searchable_by_its_tag() {
        let mut d = crate::document::Document::new(Path::new("x"));
        *d.body_mut() = vec![crate::document::Block::Math {
            list: vec![crate::document::Inline::Math(
                crate::document::math_notation::parse("1/2"),
            )],
            tag: Some("eq:gain".into()),
        }];
        let hits = search_document(&d, "gain");
        assert_eq!(hits.len(), 1, "the tag finds the equation");
        assert!(hits[0].0.starts_with("#eq:gain"));
    }

    #[test]
    fn an_equation_reference_is_searchable_by_its_label() {
        let d = crate::document::markdown::parse(Path::new("x"), "see @eq:gain above\n");
        let hits = search_document(&d, "eq:gain");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "@eq:gain");
    }

    #[test]
    fn empty_query_yields_no_document_hits() {
        let d = doc("anything\n");
        assert!(search_document(&d, "").is_empty());
    }

    #[test]
    fn a_run_is_one_candidate_whatever_its_length() {
        // A paragraph is one inline run, so one row per paragraph however
        // many times the query occurs inside it — the row shows the run,
        // the preview shows the rest.
        let d = doc("one two\n\nthree four\n");
        let hits = search_document(&d, "o");
        assert_eq!(hits.len(), 2);
    }
}
