//! A document outline is derived from heading blocks; nothing in the document
//! model stores it. Headings nest by insertion order: each heading belongs
//! under the nearest preceding heading with a strictly smaller level. This
//! keeps documents that start at H3 or skip levels sensible instead of
//! inventing empty levels in their structure or numbering.

use super::{Block, Inline};

/// One heading, placed in the outline.
#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    /// Index into `Document::blocks`.
    pub block: usize,
    /// The heading's own level, 1 to 4 — what it is drawn as.
    pub level: u8,
    /// How deep it sits in the outline, counting from 0. Not the same as
    /// `level`: a document that opens at H3 still has a depth-0 root.
    pub depth: usize,
    /// The auto-number, e.g. "2.1.1". Derived from `depth`, so it never
    /// shows a gap for a level the document skipped.
    pub number: String,
    /// The heading's flat text.
    pub text: String,
}

/// The heading outline of `blocks`, in document order. Non-heading blocks
/// are skipped; the result is flat, with `depth` carrying the shape, which is
/// what every consumer of it actually wants to draw.
pub fn outline(blocks: &[Block]) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut levels = Vec::new();
    let mut counters = Vec::new();

    for (block, content) in blocks.iter().enumerate() {
        let Block::Heading { level, .. } = content else {
            continue;
        };

        while levels.last().is_some_and(|open| *open >= *level) {
            levels.pop();
        }

        let depth = levels.len();
        levels.push(*level);
        counters.truncate(depth + 1);
        counters.resize(depth + 1, 0);
        counters[depth] += 1;

        let text = content
            .inlines()
            .iter()
            .map(|run| match run {
                Inline::Text(text) => text.text.as_str(),
                // Math is opaque and contributes no heading prose.
                Inline::Math(_) => "",
                Inline::Note(_) => "",
                Inline::EqRef(_) => "",
                Inline::TableCell(_) => unreachable!("heading blocks cannot contain table cells"),
            })
            .collect();

        nodes.push(Node {
            block,
            level: *level,
            depth,
            number: counters
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("."),
            text,
        });
    }

    nodes
}

fn last_at_or_before(nodes: &[Node], block: usize) -> Option<usize> {
    nodes.iter().rposition(|node| node.block <= block)
}

/// The chain from a root down to the heading `block` sits under, outermost
/// first. Empty when `block` is above every heading in the document.
///
/// This is the breadcrumb's path and nothing else: it answers "where am I",
/// so a block that *is* a heading counts as being under itself.
pub fn trail(nodes: &[Node], block: usize) -> Vec<&Node> {
    let Some(index) = last_at_or_before(nodes, block) else {
        return Vec::new();
    };

    let mut result = vec![&nodes[index]];
    let mut depth = nodes[index].depth;
    for node in nodes[..index].iter().rev() {
        if node.depth < depth {
            result.push(node);
            depth = node.depth;
        }
    }
    result.reverse();
    result
}

/// The index into `nodes` of the heading `block` sits under, if any — the
/// outline row to show as current.
pub fn active(nodes: &[Node], block: usize) -> Option<usize> {
    last_at_or_before(nodes, block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Style, Text};

    fn heading(level: u8, text: &str) -> Block {
        Block::Heading {
            level,
            folded: false,
            content: vec![Inline::Text(Text {
                text: text.to_owned(),
                style: Style::PLAIN,
            })],
        }
    }

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::Text(Text {
            text: text.to_owned(),
            style: Style::PLAIN,
        })])
    }

    #[test]
    fn headings_nest_by_insertion_order_not_by_level() {
        let blocks = vec![
            heading(1, "A"),
            heading(2, "B"),
            heading(2, "C"),
            heading(1, "D"),
            heading(3, "E"),
        ];
        let nodes = outline(&blocks);

        assert_eq!(
            nodes.iter().map(|node| node.depth).collect::<Vec<_>>(),
            vec![0, 1, 1, 0, 1]
        );
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.number.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "1.1", "1.2", "2", "2.1"]
        );
    }

    #[test]
    fn a_heading_with_no_smaller_level_before_it_is_a_root() {
        let blocks = vec![heading(3, "A"), heading(2, "B"), heading(1, "C")];
        let nodes = outline(&blocks);

        assert_eq!(
            nodes.iter().map(|node| node.depth).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.number.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );
    }

    #[test]
    fn a_document_that_opens_deep_still_numbers_from_one() {
        let blocks = vec![heading(4, "A"), heading(4, "B")];
        let nodes = outline(&blocks);

        assert_eq!(
            nodes.iter().map(|node| node.depth).collect::<Vec<_>>(),
            vec![0, 0]
        );
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.number.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }

    #[test]
    fn paragraphs_between_headings_are_skipped_but_block_indices_are_not() {
        let blocks = vec![
            para("before"),
            heading(1, "A"),
            para("middle"),
            para("more"),
            heading(2, "B"),
        ];
        let nodes = outline(&blocks);

        assert_eq!(
            nodes.iter().map(|node| node.block).collect::<Vec<_>>(),
            vec![1, 4]
        );
    }

    #[test]
    fn the_trail_names_every_ancestor_of_the_caret_s_block() {
        let blocks = vec![
            heading(1, "A"),
            heading(2, "B"),
            para("body"),
            heading(2, "C"),
        ];
        let nodes = outline(&blocks);

        assert_eq!(
            trail(&nodes, 2)
                .iter()
                .map(|node| node.text.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert_eq!(
            trail(&nodes, 0)
                .iter()
                .map(|node| node.text.as_str())
                .collect::<Vec<_>>(),
            vec!["A"]
        );
    }

    #[test]
    fn a_block_above_every_heading_has_no_trail() {
        let blocks = vec![para("before"), heading(1, "A")];
        let nodes = outline(&blocks);

        assert_eq!(trail(&nodes, 0), Vec::<&Node>::new());
        assert_eq!(active(&nodes, 0), None);
    }
}
