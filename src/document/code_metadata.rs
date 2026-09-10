//! File-local code annotations, stored in one trailing Markdown comment.
//! Character coordinates are checked against the saved block text before applying.
use super::code::CodeStyle;
use super::{Document, FlatPos, FlatRange, Focus, Inline};
use serde::{Deserialize, Serialize};

const OPEN: &str = "\n\n<!-- typewritter-code\n";
const CLOSE: &str = "\n-->";

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Metadata {
    entries: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    note: Option<String>,
    block: usize,
    text: String,
    start: usize,
    end: usize,
    style: CodeStyle,
}

pub(super) fn detach(text: &str) -> (&str, Metadata) {
    let trimmed = text.trim_end_matches(['\r', '\n']);
    if let Some((body, suffix)) = trimmed.rsplit_once(OPEN)
        && let Some(data) = suffix.strip_suffix(CLOSE)
        && let Ok(metadata) = serde_json::from_str(data)
    {
        return (body, metadata);
    }
    (text, Metadata::default())
}

pub(super) fn append(doc: &Document, text: &mut String) {
    let mut metadata = Metadata::default();
    let scopes = std::iter::once((None, doc.body())).chain(
        doc.notes
            .iter()
            .map(|note| (Some(note.label.clone()), note.body.as_slice())),
    );
    for (note, blocks) in scopes {
        for (block, source) in blocks.iter().enumerate() {
            // A table row's content is its cells, not a run list: the flat
            // text this pass walks cannot address it, and nothing inside a
            // cell can carry a code brush. Skipping keeps `block` aligned
            // with the scope index `apply` reads back.
            if source.is_table() {
                continue;
            }
            let mut start = 0;
            let content = source
                .inlines()
                .iter()
                .map(Inline::text)
                .collect::<String>();
            for run in source.inlines() {
                let end = start + run.text().chars().count();
                if (source.is_code() || run.style().code) && run.style().syntax != CodeStyle::PLAIN
                {
                    metadata.entries.push(Entry {
                        note: note.clone(),
                        block,
                        text: content.clone(),
                        start,
                        end,
                        style: run.style().syntax,
                    });
                }
                start = end;
            }
        }
    }
    if !metadata.entries.is_empty() {
        text.push_str(OPEN);
        // `>` can only occur inside a JSON string here. Escape the comment
        // terminator so literal code cannot close the metadata comment early.
        text.push_str(
            &serde_json::to_string(&metadata)
                .expect("code metadata is serializable")
                .replace("-->", "--\\u003E"),
        );
        text.push_str(CLOSE);
    }
}

pub(super) fn apply(doc: &mut Document, metadata: Metadata) {
    for entry in metadata.entries {
        doc.focus = match entry.note {
            None => Focus::Body,
            Some(label) => match doc.notes.iter().position(|note| note.label == label) {
                Some(index) => Focus::Note(index),
                None => continue,
            },
        };
        let Some(block) = doc.scope().get(entry.block) else {
            continue;
        };
        if block.is_table() {
            continue;
        }
        let text = block.inlines().iter().map(Inline::text).collect::<String>();
        if text != entry.text || entry.start > entry.end || entry.end > text.chars().count() {
            continue;
        }
        doc.edit_code_styles(
            FlatRange::new(
                FlatPos {
                    block: entry.block,
                    offset: entry.start,
                },
                FlatPos {
                    block: entry.block,
                    offset: entry.end,
                },
            ),
            |style| *style = entry.style,
        );
    }
    doc.focus = Focus::Body;
    doc.caret.block = 0;
    doc.caret.inline = 0;
    doc.caret.offset = 0;
    doc.caret.style = super::Style::PLAIN;
    doc.dirty = false;
}

/// Remove annotations from a temporary serialization copy, then merge runs
/// so manual color boundaries never turn into extra backtick delimiters.
pub(super) fn plain(doc: &Document) -> Document {
    let mut doc = doc.clone();
    for block in doc
        .body
        .iter_mut()
        .chain(doc.notes.iter_mut().flat_map(|note| &mut note.body))
    {
        for run in block.inlines_mut() {
            if let Inline::Text(text) = run {
                text.style.syntax = CodeStyle::PLAIN;
            }
        }
        let mut merged: Vec<Inline> = Vec::new();
        for run in std::mem::take(block.inlines_mut()) {
            match (merged.last_mut(), run) {
                (Some(Inline::Text(previous)), Inline::Text(current))
                    if previous.style.code && previous.style == current.style =>
                {
                    previous.text.push_str(&current.text)
                }
                (_, run) => merged.push(run),
            }
        }
        *block.inlines_mut() = merged;
    }
    doc
}
