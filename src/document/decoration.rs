//! The marks a line carries besides its glyphs: the box behind a code
//! span, a badge's chip, a highlight's bar, a done task's strike, and the
//! bullet, number or checkbox hung in a list item's gutter.
//!
//! Drawn through a [`Canvas`], so the editor and a PDF export run *this*
//! code rather than two copies of it — the same reason [`math_paint`] is
//! shared. None of these is a hard shape on its own; what makes them one
//! module is that they are all read off the same thing, a line's
//! [`Painted`] pieces, and the arithmetic that turns pieces into marks is
//! exactly the kind that two painters drift on. A badge whose label is
//! inset by one pad and whose box is grown by two, a mark that must merge
//! across the runs it spans, a checkbox tick at five derived offsets: none
//! of those is a number a second painter would rediscover.
//!
//! What stays out is anything about a session. There is no caret here, no
//! selection, no hover, and no glow — the last of those is a second layer
//! the screen composites and a page has no equivalent of, so the editor
//! draws it from [`highlight_bars`] rather than this module growing a
//! notion of one.
//!
//! [`math_paint`]: super::math_paint

use crate::canvas::{self, Canvas};
use crate::components::editor::CODE_ROUNDING;
use crate::layout::Rect;
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

use super::layout::{CHECK_GAP, CHECK_SIZE, NUMBER_GUTTER, NUMBER_SIZE};
use super::{BadgeColor, Block, ListMarker, Style};

/// Padding of the tint behind an inline code span. Tighter than the
/// [`BLOCK_PAD`] a fenced block gets, so a `` `run` `` mid-sentence doesn't
/// push the line apart.
///
/// [`BLOCK_PAD`]: crate::components::editor::BLOCK_PAD
const INLINE_PAD: (f32, f32) = (8.0, 4.0);
/// A bullet's dot: inset from the content column and sized to read as a
/// mark, not as a glyph. `BULLET_INSET` is from the column's left edge to
/// the dot's centre.
const BULLET_INSET: f32 = 11.0;
const BULLET_RADIUS: f32 = 2.5;
/// A task checkbox's corner radius — between the row radius and a code
/// span's, so it reads as a control, not as a box of text.
const CHECK_RADIUS: f32 = 4.5;
const CHECK_ROUNDING: Rounding = Rounding::uniform(CHECK_RADIUS);
/// A done task's strike: centred on the line's own centre — the text is
/// drawn v-centred on the same point, so the rule crosses the x-height.
const STRIKE_THICKNESS: f32 = 1.4;
/// The highlight bar: an underline, not a wash, so the glyphs keep the
/// page's own contrast. `DROP` is measured down from the line's centre.
const HIGHLIGHT_DROP: f32 = 8.0;
const HIGHLIGHT_THICKNESS: f32 = 2.5;
const HIGHLIGHT_PAD: f32 = 2.0;
pub const HIGHLIGHT_ROUNDING: Rounding = Rounding::uniform(1.25);
/// A badge's box: its corner radius, and the radius its outline is stroked
/// at — half a pixel tighter, because the outline is inset by half a pixel
/// so its stroke lands inside the fill rather than straddling its edge.
const BADGE_RADIUS: f32 = 4.0;
const BADGE_OUTLINE_RADIUS: f32 = 3.5;

/// One measured piece of a visual line: `(text, style, x, width)`, where
/// `x` and `width` are the *label's* — a badge's box is wider than what it
/// is drawn around, and every mark here is measured off the label.
pub type Painted = (String, Style, f32, f32);

/// One run of a line, measured: where its label starts and how wide the
/// label sets, given the pen position and the advance `layout::advance`
/// reserved for it.
///
/// The two differ only for a badge, which reserves a pad on each side of
/// its label for the chip drawn around it. Both painters build their pieces
/// through here so the label and the box can never be measured from two
/// different readings of that pad.
pub fn piece(text: String, style: Style, cursor: f32, advance: f32, scale: f32) -> Painted {
    let pad = if style.badge {
        theme::BADGE_PAD * scale
    } else {
        0.0
    };
    (text, style, cursor + pad, advance - pad * 2.0)
}

/// Every mark this line's runs carry, under the text and in the editor's
/// own order: code boxes, then chips, then highlight bars, then a done
/// task's strike.
///
/// `top` and `height` are the line's; `baseline` is its vertical centre,
/// which is where text is drawn from and what every mark that is not sized
/// to the whole line hangs off.
pub fn runs(
    canvas: &mut dyn Canvas,
    pieces: &[Painted],
    block: &Block,
    top: f32,
    height: f32,
    baseline: f32,
    scale: f32,
) {
    // A run inside a fenced block already sits on the slab its painter drew
    // under the whole fence; only a code span in prose needs its own box.
    for (start, end) in spans(pieces, |piece| piece.1.code && !block.is_code()) {
        canvas.draw_rectangle(
            (start - INLINE_PAD.0, top + INLINE_PAD.1),
            (
                end - start + INLINE_PAD.0 * 2.0,
                height - INLINE_PAD.1 * 2.0,
            ),
            theme::code(),
            CODE_ROUNDING,
        );
    }

    // A chip's box is centred on the line rather than sized to it: the
    // label is 9pt, and a box grown to the leading would read as a code
    // block, not as a tag.
    for (start, end, color) in badge_spans(pieces) {
        let badge = Rect {
            x: start - theme::BADGE_PAD * scale,
            y: baseline - theme::BADGE_HEIGHT * 0.5,
            width: end - start + theme::BADGE_PAD * 2.0 * scale,
            height: theme::BADGE_HEIGHT,
        };
        canvas.draw_rectangle(
            badge.position(),
            badge.size(),
            theme::fade(theme::badge_ink(color), 0.12),
            Rounding::uniform(BADGE_RADIUS),
        );
        canvas::rounded_outline(
            canvas,
            badge.inset(0.5),
            BADGE_OUTLINE_RADIUS,
            1.0,
            theme::fade(theme::badge_ink(color), 0.28),
        );
    }

    for (at, size) in highlight_bars(pieces, baseline) {
        canvas.draw_rectangle(at, size, theme::highlight(), HIGHLIGHT_ROUNDING);
    }

    // A done task is quiet twice over: dim ink (via `layout::text_style`)
    // and a real drawn rule through the run.
    if matches!(
        block,
        Block::ListItem {
            marker: ListMarker::Task { done: true },
            ..
        }
    ) && let (Some(first), Some(last)) = (pieces.first(), pieces.last())
    {
        canvas::rule(
            canvas,
            (first.2 - 1.0, baseline - STRIKE_THICKNESS * 0.5),
            last.2 + last.3 - first.2 + 2.0,
            STRIKE_THICKNESS,
            theme::dim(),
        );
    }
}

/// Where each highlight bar on this line lands, as `(at, size)`.
///
/// Separate from the drawing because the screen draws each bar twice: once
/// crisp on the text layer, and once fatter and fainter on the glow layer
/// whose blur turns it into the falloff. A page has no such layer, and the
/// editor has no business re-deriving the geometry for its second copy.
pub fn highlight_bars(pieces: &[Painted], baseline: f32) -> Vec<((f32, f32), (f32, f32))> {
    spans(pieces, |piece| piece.1.highlight)
        .into_iter()
        .map(|(start, end)| {
            (
                (start - HIGHLIGHT_PAD, baseline + HIGHLIGHT_DROP),
                (end - start + HIGHLIGHT_PAD * 2.0, HIGHLIGHT_THICKNESS),
            )
        })
        .collect()
}

/// A list item's marker, hung in the gutter left of the content column.
///
/// Virtual like a heading's auto-number: drawn, never laid out, so the
/// caret cannot reach it and it never shifts the text it labels. Bullets
/// are drawn glyphs, numbers are right-aligned mono in a fixed column, and
/// a task's box is a control — filled and ticked when done. `content_x` is
/// the content column the item's text hangs from; `baseline` the line's
/// vertical centre.
pub fn marker(
    canvas: &mut dyn Canvas,
    marker: &ListMarker,
    content_x: f32,
    baseline: f32,
    scale: f32,
) {
    match marker {
        ListMarker::Bullet => {
            // Optically centred: a dot a hair above the true centre reads
            // as aligned with lowercase text.
            canvas.draw_circle(
                (content_x - BULLET_INSET * scale, baseline - 1.0),
                BULLET_RADIUS * scale,
                theme::accent(),
            );
        }
        ListMarker::Number(n) => {
            canvas.draw_text(
                &n.to_string(),
                (content_x - NUMBER_GUTTER, baseline),
                &TextStyle::mono(NUMBER_SIZE, theme::non_text()),
                theme::RIGHT,
            );
        }
        ListMarker::Task { done } => {
            let size = CHECK_SIZE * scale;
            let gap = CHECK_GAP * scale;
            let box_ = Rect {
                x: content_x - gap - size,
                y: baseline - size * 0.5,
                width: size,
                height: size,
            };
            if *done {
                canvas.draw_rectangle(
                    box_.position(),
                    box_.size(),
                    theme::fade(theme::accent(), 0.18),
                    CHECK_ROUNDING,
                );
                canvas::rounded_outline(
                    canvas,
                    box_.inset(0.5),
                    CHECK_RADIUS - 0.5,
                    1.0,
                    theme::accent(),
                );
                canvas::polyline(
                    canvas,
                    &[
                        (box_.x + size * 0.24, box_.y + size * 0.52),
                        (box_.x + size * 0.42, box_.y + size * 0.70),
                        (box_.x + size * 0.76, box_.y + size * 0.30),
                    ],
                    theme::accent(),
                    1.6,
                );
            } else {
                canvas::rounded_outline(
                    canvas,
                    box_.inset(0.5),
                    CHECK_RADIUS - 0.5,
                    1.0,
                    theme::non_text(),
                );
            }
        }
    }
}

/// Maximal runs of adjacent pieces matching `pred`, as `(start_x, end_x)`.
/// Adjacent pieces merge into one span so a decoration split across runs —
/// a bold word inside a highlight, two code spans touching — reads as a
/// single mark rather than beading up at every seam.
fn spans(pieces: &[Painted], pred: impl Fn(&Painted) -> bool) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        if !pred(&pieces[i]) {
            i += 1;
            continue;
        }
        let mut end = i;
        while pieces.get(end + 1).is_some_and(&pred) {
            end += 1;
        }
        out.push((pieces[i].2, pieces[end].2 + pieces[end].3));
        i = end + 1;
    }
    out
}

/// [`spans`] for badges, which merge only while the colour holds: two chips
/// of different colours touching are two chips, not one two-tone box.
fn badge_spans(pieces: &[Painted]) -> Vec<(f32, f32, BadgeColor)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        if !pieces[i].1.badge {
            i += 1;
            continue;
        }
        let color = pieces[i].1.badge_color;
        let mut end = i;
        while pieces
            .get(end + 1)
            .is_some_and(|piece| piece.1.badge && piece.1.badge_color == color)
        {
            end += 1;
        }
        out.push((pieces[i].2, pieces[end].2 + pieces[end].3, color));
        i = end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_decorated_pieces_merge_into_one_span() {
        let mark = Style {
            highlight: true,
            ..Style::PLAIN
        };
        let bold_mark = Style { bold: true, ..mark };
        let piece = |style, x: f32, w: f32| (String::new(), style, x, w);
        let pieces = vec![
            piece(Style::PLAIN, 0.0, 10.0),
            // Two marked runs touching — a bold word inside the mark. One
            // bar, not two, or the seam shows as a notch.
            piece(mark, 10.0, 20.0),
            piece(bold_mark, 30.0, 15.0),
            piece(Style::PLAIN, 45.0, 5.0),
            piece(mark, 50.0, 8.0),
        ];
        assert_eq!(
            spans(&pieces, |p| p.1.highlight),
            vec![(10.0, 45.0), (50.0, 58.0)]
        );
        assert!(spans(&pieces, |p| p.1.code).is_empty());
    }

    #[test]
    fn badge_outlines_split_when_their_colors_differ() {
        let orange = Style {
            badge: true,
            ..Style::PLAIN
        };
        let blue = Style {
            badge: true,
            badge_color: BadgeColor::Blue,
            ..Style::PLAIN
        };
        let pieces = vec![
            ("A".into(), orange, 4.0, 10.0),
            ("B".into(), orange, 22.0, 10.0),
            ("C".into(), blue, 40.0, 10.0),
        ];
        assert_eq!(
            badge_spans(&pieces),
            vec![
                (4.0, 32.0, BadgeColor::Orange),
                (40.0, 50.0, BadgeColor::Blue)
            ]
        );
    }

    #[test]
    fn a_badges_label_is_inset_inside_the_box_it_is_drawn_in() {
        // The advance `layout::advance` reserves covers label + two pads;
        // the label starts one pad in, and the chip drawn around it in
        // `runs` spans the whole advance again.
        let style = Style {
            badge: true,
            ..Style::PLAIN
        };
        let (_, _, x, width) = piece("TODO".into(), style, 100.0, 60.0, 1.0);
        assert_eq!(x, 100.0 + theme::BADGE_PAD);
        assert_eq!(width, 60.0 - theme::BADGE_PAD * 2.0);
        assert_eq!(x + width + theme::BADGE_PAD, 160.0, "the advance is spent");
    }

    #[test]
    fn a_plain_run_reserves_exactly_what_it_sets() {
        let (_, _, x, width) = piece("word".into(), Style::PLAIN, 100.0, 60.0, 1.0);
        assert_eq!((x, width), (100.0, 60.0));
    }

    #[test]
    fn a_highlight_bar_hangs_below_the_line_it_marks() {
        let mark = Style {
            highlight: true,
            ..Style::PLAIN
        };
        let pieces = vec![("word".to_string(), mark, 10.0, 40.0)];
        let bars = highlight_bars(&pieces, 100.0);
        assert_eq!(
            bars,
            vec![(
                (10.0 - HIGHLIGHT_PAD, 100.0 + HIGHLIGHT_DROP),
                (40.0 + HIGHLIGHT_PAD * 2.0, HIGHLIGHT_THICKNESS),
            )],
            "the bar is under the baseline and overhangs the run each side"
        );
    }
}
