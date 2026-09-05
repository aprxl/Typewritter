//! Inline runs and line breaking for a paragraph.
//!
//! A paragraph is a sequence of [`Run`]s — styled text, a bordered chip, a
//! raised footnote marker — broken into words, measured, and greedily
//! packed into lines. Greedy is what every editor does on the typing path;
//! the paragraph-at-a-time optimum (Knuth–Plass) is for typesetters that
//! can afford to look ahead, and we cannot.
//!
//! Measurement is passed in rather than reached for, so the breaking rule
//! is testable without a GPU — [`Paragraph::wrap`] is the whole algorithm
//! and takes a plain closure.
//!
//! This is deliberately *inline* layout only: it knows about words, widths,
//! and one line height. Where paragraphs sit relative to each other is the
//! caller's business, and what the words say will be the document model's.

use crate::renderer::{Color, Layer, Rounding};
use crate::theme::{self, TextStyle};

/// One stretch of a paragraph.
pub enum Run {
    /// Styled text. Breaks between words.
    Text(String, TextStyle),
    /// Styled text lifted off the baseline by `rise` — footnote markers.
    Raised(String, TextStyle, f32),
    /// A bordered label that never breaks: `IDEAL`, `TODO`, `PS`.
    Chip(Chip),
}

impl Run {
    pub fn text(text: &str, style: TextStyle) -> Self {
        Self::Text(text.into(), style)
    }

    pub fn raised(text: &str, style: TextStyle, rise: f32) -> Self {
        Self::Raised(text.into(), style, rise)
    }
}

pub struct Chip {
    pub label: String,
    pub color: Color,
    pub background: Option<Color>,
    pub icon: Option<&'static str>,
}

impl Chip {
    const HEIGHT: f32 = 16.0;
    const PADDING: f32 = 5.0;
    const ICON: f32 = 9.0;

    pub fn new(label: &str, color: Color) -> Self {
        Self {
            label: label.into(),
            color,
            background: None,
            icon: None,
        }
    }

    pub fn filled(mut self, background: Color) -> Self {
        self.background = Some(background);
        self
    }

    pub fn with_icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    fn style(&self) -> TextStyle {
        TextStyle::mono(9.0, self.color.clone()).tracked(0.1)
    }

    fn draw(&self, layer: &Layer, at: (f32, f32), width: f32) {
        let top = at.1 - Self::HEIGHT / 2.0;
        let color = self.color.clone();
        if let Some(background) = &self.background {
            layer.draw_rectangle(
                (at.0, top),
                (width, Self::HEIGHT),
                background.clone(),
                Rounding::NONE,
            );
        }
        // A 1px outline as four rules: cheaper than a stroked path, and
        // the design's chips are square.
        theme::rule(layer, (at.0, top), width, 1.0, color.clone());
        theme::rule(
            layer,
            (at.0, top + Self::HEIGHT - 1.0),
            width,
            1.0,
            color.clone(),
        );
        theme::vertical_rule(layer, (at.0, top), Self::HEIGHT, 1.0, color.clone());
        theme::vertical_rule(
            layer,
            (at.0 + width - 1.0, top),
            Self::HEIGHT,
            1.0,
            color.clone(),
        );

        let mut x = at.0 + Self::PADDING;
        if let Some(path) = self.icon {
            theme::icon(
                layer,
                path,
                (x, at.1 - Self::ICON / 2.0),
                Self::ICON,
                color,
                3.0,
            );
            x += Self::ICON + 4.0;
        }
        theme::draw(layer, &self.label, (x, at.1), &self.style(), theme::LEFT);
    }
}

/// An atom of a line: never split, optionally preceded by a space.
struct Piece {
    kind: Kind,
    width: f32,
    space_before: f32,
}

enum Kind {
    Word {
        text: String,
        style: TextStyle,
        rise: f32,
    },
    /// Index of the [`Run::Chip`] this came from.
    Chip(usize),
}

/// Where the wrapper decided a piece goes.
struct Placement {
    x: f32,
    line: usize,
}

pub struct Paragraph {
    pub runs: Vec<Run>,
    pub line_height: f32,
}

impl Paragraph {
    pub fn new(line_height: f32, runs: Vec<Run>) -> Self {
        Self { runs, line_height }
    }

    /// Draws wrapped to `width`, `top_left` being the top-left of the first
    /// line's box. Returns the height used, so a caller can stack
    /// paragraphs without knowing how many lines each took.
    pub fn draw(&self, layer: &Layer, top_left: (f32, f32), width: f32) -> f32 {
        let pieces = self.pieces(&|text, style| theme::width(layer, text, style));
        let placements = Self::wrap(&pieces, width);

        let (x, top) = top_left;
        for (piece, placement) in pieces.iter().zip(&placements) {
            let baseline = top + self.line_height * (placement.line as f32 + 0.5);
            match &piece.kind {
                Kind::Word { text, style, rise } => theme::draw(
                    layer,
                    text,
                    (x + placement.x, baseline - rise),
                    style,
                    theme::LEFT,
                ),
                Kind::Chip(index) => {
                    let Run::Chip(chip) = &self.runs[*index] else {
                        continue;
                    };
                    chip.draw(layer, (x + placement.x, baseline), piece.width);
                }
            }
        }

        let lines = placements.last().map_or(0, |p| p.line + 1);
        lines as f32 * self.line_height
    }

    /// The breaking rule: put each piece on the current line if it fits,
    /// otherwise start a new one. A piece wider than the whole column goes
    /// on a line by itself and overhangs rather than vanishing.
    fn wrap(pieces: &[Piece], width: f32) -> Vec<Placement> {
        let mut placements = Vec::with_capacity(pieces.len());
        let (mut cursor, mut line, mut first) = (0.0f32, 0usize, true);

        for piece in pieces {
            let space = if first { 0.0 } else { piece.space_before };
            if !first && cursor + space + piece.width > width {
                cursor = 0.0;
                line += 1;
            } else {
                cursor += space;
            }
            placements.push(Placement { x: cursor, line });
            cursor += piece.width;
            first = false;
        }
        placements
    }

    fn pieces(&self, measure: &dyn Fn(&str, &TextStyle) -> f32) -> Vec<Piece> {
        let mut pieces = Vec::new();
        for (index, run) in self.runs.iter().enumerate() {
            match run {
                Run::Text(text, style) => Self::words(&mut pieces, measure, text, style, 0.0),
                Run::Raised(text, style, rise) => {
                    Self::words(&mut pieces, measure, text, style, *rise)
                }
                Run::Chip(chip) => {
                    let style = chip.style();
                    let icon = chip.icon.map_or(0.0, |_| Chip::ICON + 4.0);
                    pieces.push(Piece {
                        kind: Kind::Chip(index),
                        width: measure(&chip.label, &style) + Chip::PADDING * 2.0 + icon,
                        space_before: measure(" ", &style),
                    });
                }
            }
        }
        pieces
    }

    fn words(
        pieces: &mut Vec<Piece>,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
        text: &str,
        style: &TextStyle,
        rise: f32,
    ) {
        let space = measure(" ", style);
        // A run that starts with whitespace still needs a gap after
        // whatever preceded it; `split_whitespace` alone eats that.
        let leading = text.starts_with(char::is_whitespace);
        for (i, word) in text.split_whitespace().enumerate() {
            pieces.push(Piece {
                width: measure(word, style),
                space_before: if i == 0 && !leading { 0.0 } else { space },
                kind: Kind::Word {
                    text: word.to_string(),
                    style: style.clone(),
                    rise,
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph 10 wide, so line breaks are countable by hand.
    fn fake_measure(text: &str, _: &TextStyle) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    fn plain(text: &str) -> Paragraph {
        Paragraph::new(
            30.0,
            vec![Run::text(text, TextStyle::sans(17.5, theme::ink()))],
        )
    }

    #[test]
    fn words_pack_greedily_and_break_at_the_column_edge() {
        let paragraph = plain("aaa bbb ccc ddd");
        let pieces = paragraph.pieces(&fake_measure);
        // Each word is 30 wide, each space 10: "aaa bbb" is 70, adding
        // "ccc" would be 110.
        let placements = Paragraph::wrap(&pieces, 100.0);

        assert_eq!(placements[0].line, 0);
        assert_eq!(placements[1].line, 0);
        assert_eq!(placements[2].line, 1, "third word starts a new line");
        assert_eq!(placements[3].line, 1);
        assert_eq!(placements[2].x, 0.0, "a wrapped line starts at the left");
        assert_eq!(placements[1].x, 40.0, "space is 10 wide, word is 30");
    }

    #[test]
    fn a_run_boundary_mid_sentence_keeps_its_space() {
        // How the design's bold terms are written: "…into the", "bold",
        // " terminal". Without the leading space the words would collide.
        let style = TextStyle::sans(17.5, theme::ink());
        let paragraph = Paragraph::new(
            30.0,
            vec![
                Run::text("into the", style.clone()),
                Run::text(" non-inverting", style.clone().bold()),
                Run::text(" terminal", style),
            ],
        );
        let pieces = paragraph.pieces(&fake_measure);

        // "into"/"the" are pieces 0-1; the bold run is piece 2; " terminal"
        // is piece 3.
        assert_eq!(
            pieces[2].space_before, 10.0,
            "the authored leading space is what keeps `the non-inverting` apart"
        );
        assert_eq!(pieces[3].space_before, 10.0, "leading space is honoured");
    }

    #[test]
    fn a_piece_wider_than_the_column_still_gets_drawn() {
        let paragraph = plain("short supercalifragilistic");
        let pieces = paragraph.pieces(&fake_measure);
        let placements = Paragraph::wrap(&pieces, 100.0);

        assert_eq!(placements[1].line, 1);
        assert_eq!(placements[1].x, 0.0, "overhangs rather than disappearing");
    }
}
