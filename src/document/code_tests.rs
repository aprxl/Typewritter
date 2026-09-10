use super::super::{layout, markdown};
use super::*;
use std::path::Path;

fn doc(text: &str) -> Document {
    markdown::parse(Path::new("code.md"), text)
}
fn range(block: usize, start: usize, end: usize) -> FlatRange {
    FlatRange::new(
        FlatPos {
            block,
            offset: start,
        },
        FlatPos { block, offset: end },
    )
}

#[test]
fn all_ten_bundled_queries_highlight_real_code() {
    for (language, source) in Language::ALL.into_iter().zip([
        "int main(void) { return 42; }",
        "class Foo { public: int x = 42; };",
        "fn main() { let x = 42; }",
        "local x = 42 -- hello",
        "def foo():\n    return 42",
        "const x = 42;",
        "const x: number = 42;",
        "class Foo { int x = 42; }",
        "class Foo { int x = 42; }",
        "package main\nfunc main() { x := 42 }",
    ]) {
        let ink = highlight(language, source);
        assert_eq!(ink.len(), source.chars().count());
        assert!(ink.iter().any(Option::is_some), "{language:?}");
        let at = source[..source.find("42").unwrap()].chars().count();
        assert!(ink[at].is_some(), "number in {language:?}");
    }
}

#[test]
fn multiline_parser_and_unicode_ranges() {
    let document = doc("```rust\n/* comment\néλ */\nlet x = \"界\";\n```");
    let colors = colors(document.body());
    assert!(colors[1].iter().all(|ink| *ink == Some(Ink::Syntax(0))));
    assert_eq!(colors[1].len(), 5);
    assert!(colors[2].contains(&Some(Ink::Syntax(1))));
    assert!(
        highlight(Language::Rust, "fn broken( { \"界")
            .iter()
            .any(Option::is_some)
    );
}

#[test]
fn diy_colors_roundtrip_and_follow_edits() {
    let mut document = doc("```python\nα = 42\nprint(α)\n```");
    document.set_code_options(range(0, 0, 1), None, true);
    document.set_code_color(range(0, 0, 1), Some(MathHue::Rose));
    let ink = colors(document.body());
    assert_eq!(ink[0][0], Some(Ink::Manual(MathHue::Rose)));
    assert!(ink[0][1..].iter().all(Option::is_none));
    assert!(ink[1].iter().all(Option::is_none));
    let saved = markdown::serialize(&document);
    let mut reopened = doc(&saved);
    assert_eq!(document.body(), reopened.body());
    assert_eq!(saved, markdown::serialize(&reopened));
    reopened.caret.offset = 0;
    reopened.caret.style = reopened.body()[0].inlines()[0].style();
    reopened.insert_text("β");
    assert_eq!(
        colors(reopened.body())[0][1],
        Some(Ink::Manual(MathHue::Rose))
    );
    reopened.set_code_options(range(0, 0, 1), Some(Language::Python), false);
    assert!(
        colors(reopened.body())[0]
            .iter()
            .all(|ink| !matches!(ink, Some(Ink::Manual(_))))
    );
}

#[test]
fn inline_colors_do_not_change_code_delimiters_or_prose() {
    let mut document = doc("Before `α + β -->` after.");
    document.set_code_options(range(0, 7, 8), Some(Language::Python), false);
    assert!(colors(document.body())[0][..7].iter().all(Option::is_none));
    document.set_code_options(range(0, 7, 8), None, true);
    document.set_code_color(range(0, 7, 8), Some(MathHue::Sky));
    document.set_code_color(range(0, 11, 12), Some(MathHue::Rose));
    let saved = markdown::serialize(&document);
    assert!(saved.starts_with("Before `α + β -->` after."));
    assert_eq!(doc(&saved).body(), document.body());
    assert_eq!(document.code_extent(range(0, 11, 12)), range(0, 7, 16));
}

#[test]
fn blank_manual_fence_retains_mode_when_typing_and_splitting() {
    let mut document = doc("```\n\n```");
    document.set_code_options(range(0, 0, 0), None, true);
    assert!(document.body()[0].inlines()[0].style().syntax.manual);
    let saved = markdown::serialize(&document);
    let reopened = doc(&saved);
    assert!(reopened.body()[0].inlines()[0].style().syntax.manual);
    document.insert_text("name");
    document.newline();
    assert!(document.body()[1].inlines()[0].style().syntax.manual);
}

#[test]
fn manual_words_share_click_and_brush_targets_across_color_boundaries() {
    let mut document = doc("```rust\nhello + world\n```");
    document.set_code_options(range(0, 0, 1), None, true);
    document.set_code_color(range(0, 1, 3), Some(MathHue::Rose));
    let measure = |text: &str, _: &crate::theme::TextStyle| text.chars().count() as f32 * 10.0;
    let layout = layout::layout(&document, 500.0, &measure);
    let y = layout.blocks[0].lines[0].y + 5.0;
    let expected = layout::ContextHit::Range {
        range: range(0, 0, 5),
        kind: layout::RangeKind::CodeWord,
    };
    assert_eq!(
        layout.hit_context(22.0, y, &measure),
        Some(expected.clone())
    );
    assert!(
        layout
            .hit_contexts_in_circle(22.0, y, 3.0, 500.0, &measure)
            .contains(&expected)
    );
    assert_eq!(
        layout.hit_context(62.0, y, &measure),
        Some(layout::ContextHit::Range {
            range: range(0, 6, 7),
            kind: layout::RangeKind::CodeWord
        })
    );
}

#[test]
fn annotations_roundtrip_with_crlf_and_in_margin_notes() {
    let mut document = doc("See[^a].\n\n[^a]: `foo + bar`");
    document.focus = super::super::Focus::Note(0);
    document.set_code_options(range(0, 0, 3), None, true);
    document.set_code_color(range(0, 0, 3), Some(MathHue::Teal));
    let saved = markdown::serialize(&document).replace('\n', "\r\n");
    assert_eq!(doc(&saved).notes, document.notes);
}

#[test]
fn stale_or_out_of_bounds_metadata_never_colors_other_text() {
    let mut document = doc("`hello`");
    document.set_code_options(range(0, 0, 5), None, true);
    document.set_code_color(range(0, 0, 5), Some(MathHue::Teal));
    let saved = markdown::serialize(&document);
    let changed = saved.replacen("`hello`", "`world`", 1);
    assert!(colors(doc(&changed).body())[0].iter().all(Option::is_none));
    let invalid = saved.replace("\"end\":5", "\"end\":999999");
    assert!(colors(doc(&invalid).body())[0].iter().all(Option::is_none));
}

#[test]
fn inline_code_selection_stops_at_prose_without_a_space() {
    let mut document = doc("pre`foo`post");
    document.set_code_options(range(0, 3, 6), None, true);
    let measure = |text: &str, _: &crate::theme::TextStyle| text.chars().count() as f32 * 10.0;
    let layout = layout::layout(&document, 500.0, &measure);
    let y = layout.blocks[0].lines[0].y + 5.0;
    let expected = layout::ContextHit::Range {
        range: range(0, 3, 6),
        kind: layout::RangeKind::CodeWord,
    };
    assert_eq!(
        layout.hit_context(42.0, y, &measure),
        Some(expected.clone())
    );
    assert_eq!(
        layout.hit_contexts_in_circle(42.0, y, 2.0, 500.0, &measure),
        vec![expected]
    );
}

#[test]
fn colors_do_not_add_arrow_key_stops() {
    let mut document = doc("`hello`");
    document.set_code_options(range(0, 0, 5), None, true);
    document.set_code_color(range(0, 1, 3), Some(MathHue::Rose));
    document.set_caret(0, 0, 1);
    document.move_right();
    assert_eq!(document.caret_flat(0), 2);
    document.set_caret(0, 2, 0);
    document.move_left();
    assert_eq!(document.caret_flat(0), 2);
}

#[test]
fn inline_padding_survives_color_splits_and_caret_roundtrips_at_each_scale() {
    let mut document = doc("pre`hello`post");
    document.set_code_options(range(0, 3, 8), None, true);
    document.set_code_color(range(0, 4, 6), Some(MathHue::Rose));
    for scale in [0.7, 1.0, 1.5] {
        let measure =
            |text: &str, _: &crate::theme::TextStyle| text.chars().count() as f32 * 10.0 * scale;
        let laid = layout::layout_blocks(document.body(), 500.0, scale, &measure);
        let segments = &laid.blocks[0].lines[0].segments;
        let total: f32 = segments.iter().map(|s| s.padding.0 + s.padding.1).sum();
        assert_eq!(
            total,
            2.0 * (layout::INLINE_CODE_INSET + layout::INLINE_CODE_GAP)
        );
        assert_eq!(
            segments[2].padding,
            (0.0, 0.0),
            "color boundaries reserve no extra space"
        );
        for flat in 0..=12 {
            let (inline, offset) = document.flat_to_pos(0, flat);
            let caret = super::super::Caret {
                block: 0,
                inline,
                offset,
                style: super::super::Style::PLAIN,
            };
            let (x, y, _) = laid.caret_pos(caret, &measure);
            let hit = laid.hit(x, y, &measure);
            let found: usize = document.body()[0].inlines()[..hit.inline]
                .iter()
                .map(|r| r.text().chars().count())
                .sum::<usize>()
                + hit.offset;
            assert_eq!(found, flat, "caret at {flat}, scale {scale}");
        }
    }
}

#[test]
fn wrapping_reserves_code_background_on_every_visual_line() {
    let document = doc("a `foo bar baz` end");
    let measure = |text: &str, _: &crate::theme::TextStyle| text.chars().count() as f32 * 10.0;
    let laid = layout::layout(&document, 95.0, &measure);
    assert!(laid.blocks[0].lines.len() >= 3);
    for line in &laid.blocks[0].lines {
        let mut cursor = 0.0;
        for segment in &line.segments {
            let run = &document.body()[0].inlines()[segment.inline];
            let text: String = run
                .text()
                .chars()
                .skip(segment.start)
                .take(segment.len)
                .collect();
            cursor += segment.advance(run, &text, &document.body()[0], 1.0, &measure);
            // Trailing prose whitespace may hang past the column; code boxes may not.
            if segment.style.code {
                assert!(
                    cursor <= 95.0,
                    "code padding must participate in wrapping: {cursor}"
                );
            }
        }
    }
}
