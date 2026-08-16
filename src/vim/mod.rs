//! Vim keymap core. It parses keys, but never edits a document itself.

pub mod motion;

use crate::document::{Document, FlatPos, FlatRange};

pub use crate::document::TextObject;
pub use motion::Motion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

/// Visual and command modes live beside the two legacy shell modes. The
/// associated constants keep old exhaustive shell matches source-compatible;
/// [`Vim::visual_mode`] and [`Vim::command_active`] expose actual state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualMode {
    Char,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    VisualChar,
    VisualLine,
    Command,
}

#[allow(non_upper_case_globals)]
impl Mode {
    pub const VisualChar: Mode = Mode::Normal;
    pub const VisualLine: Mode = Mode::Normal;
    pub const Command: Mode = Mode::Normal;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Escape,
    Backspace,
    Enter,
    Control(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Yank,
    Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorTarget {
    Motion(Motion),
    TextObject(TextObject),
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualAction {
    Enter(VisualMode),
    Move(Motion, usize),
    Operate(Operator, usize),
    TextObject(TextObject),
    Exit,
}

/// Full parser output. `Vim::key` retains old shell-facing [`Action`] output;
/// new integrations should use `key_extended`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtendedAction {
    None,
    Move(Motion, usize),
    InsertAt(Motion),
    Edit(Edit, usize),
    Enter(Mode),
    OperatorPending(Operator, usize),
    Operate(Operator, OperatorTarget, usize),
    Visual(VisualAction),
    CommandInput(Key),
    Undo,
    Redo,
    Repeat,
    Search { forward: bool, count: usize },
    Substitute(usize),
    Paste { before: bool, count: usize },
    Leader,
    Passthrough,
}

/// Legacy action surface consumed by current shell. Keep variants stable
/// until shell switches to [`ExtendedAction`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Move(Motion, usize),
    InsertAt(Motion),
    Edit(Edit, usize),
    Enter(Mode),
    Leader,
    Passthrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    DeleteChar,
    DeleteLine,
    OpenBelow,
    OpenAbove,
}

const MAX_COUNT: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Prefix(char),
    Operator(Operator, usize),
    OperatorPrefix(Operator, usize),
    TextObject(Operator, bool, usize),
    Find {
        direction: i8,
        till: bool,
        count: usize,
    },
    OperatorFind {
        operator: Operator,
        operator_count: usize,
        direction: i8,
        till: bool,
        count: usize,
    },
}

pub struct Vim {
    mode: Mode,
    visual: Option<VisualMode>,
    command: bool,
    count: Option<usize>,
    pending: Option<Pending>,
    last_find: Option<(i8, bool, char)>,
    visual_anchor: Option<FlatPos>,
}

impl Default for Vim {
    fn default() -> Self {
        Self::new()
    }
}

impl Vim {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            visual: None,
            command: false,
            count: None,
            pending: None,
            last_find: None,
            visual_anchor: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn visual_mode(&self) -> Option<VisualMode> {
        self.visual
    }

    pub fn current_mode(&self) -> VimMode {
        if self.command {
            VimMode::Command
        } else if let Some(visual) = self.visual {
            match visual {
                VisualMode::Char => VimMode::VisualChar,
                VisualMode::Line => VimMode::VisualLine,
            }
        } else {
            match self.mode {
                Mode::Normal => VimMode::Normal,
                Mode::Insert => VimMode::Insert,
            }
        }
    }

    pub fn command_active(&self) -> bool {
        self.command
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.visual = None;
        self.visual_anchor = None;
        self.command = false;
        self.clear_pending();
    }

    pub fn start_visual(&mut self, mode: VisualMode, anchor: FlatPos) {
        self.mode = Mode::Normal;
        self.visual = Some(mode);
        self.visual_anchor = Some(anchor);
        self.clear_pending();
    }

    pub fn visual_anchor(&self) -> Option<FlatPos> {
        self.visual_anchor
    }

    pub fn visual_range(&self, current: FlatPos) -> Option<FlatRange> {
        self.visual_anchor
            .map(|anchor| FlatRange::new(anchor, current).normalized())
    }

    /// Build one half-open operator range from motion semantics and its final
    /// destination. Keeping this here prevents reverse operators from
    /// accidentally including the original caret twice.
    pub fn operator_range(
        doc: &Document,
        original: FlatPos,
        motion: Motion,
        current: FlatPos,
    ) -> FlatRange {
        if matches!(
            motion,
            Motion::Up
                | Motion::Down
                | Motion::FirstLine
                | Motion::LastLine
                | Motion::ParagraphBack
                | Motion::ParagraphForward
        ) {
            return doc.line_range(
                original.block.min(current.block),
                original.block.max(current.block),
            );
        }

        let forward = (original.block, original.offset) <= (current.block, current.offset);
        let includes_destination = matches!(
            motion,
            Motion::WordEnd
                | Motion::LineStart
                | Motion::LineEnd
                | Motion::FindForward(_)
                | Motion::TillForward(_)
                | Motion::FindBack(_)
                | Motion::TillBack(_)
        );
        let after = |position: FlatPos| {
            if position.offset < doc.block_len(position.block) {
                doc.position(position.block, position.offset + 1)
            } else {
                position
            }
        };
        if forward {
            FlatRange::new(
                original,
                if includes_destination {
                    after(current)
                } else {
                    current
                },
            )
        } else {
            FlatRange::new(
                current,
                if includes_destination {
                    after(original)
                } else {
                    original
                },
            )
        }
    }

    /// Compatibility parser used by current shell. Rich actions are exposed
    /// through [`key_extended`], while common old actions remain identical.
    pub fn key(&mut self, key: Key) -> Action {
        match self.key_extended(key) {
            ExtendedAction::None
            | ExtendedAction::OperatorPending(..)
            | ExtendedAction::Operate(Operator::Yank, ..)
            | ExtendedAction::Operate(Operator::Change, ..)
            | ExtendedAction::Visual(..)
            | ExtendedAction::CommandInput(_)
            | ExtendedAction::Undo
            | ExtendedAction::Redo
            | ExtendedAction::Repeat
            | ExtendedAction::Search { .. }
            | ExtendedAction::Substitute(_)
            | ExtendedAction::Paste { .. } => Action::None,
            ExtendedAction::Operate(Operator::Delete, OperatorTarget::Line, count) => {
                Action::Edit(Edit::DeleteLine, count)
            }
            ExtendedAction::Operate(..) => Action::None,
            ExtendedAction::Move(motion, count) => Action::Move(motion, count),
            ExtendedAction::InsertAt(motion) => Action::InsertAt(motion),
            ExtendedAction::Edit(edit, count) => Action::Edit(edit, count),
            ExtendedAction::Enter(mode) => Action::Enter(mode),
            ExtendedAction::Leader => Action::Leader,
            ExtendedAction::Passthrough => Action::Passthrough,
        }
    }

    pub fn key_extended(&mut self, key: Key) -> ExtendedAction {
        if self.mode == Mode::Insert {
            return match key {
                Key::Escape => {
                    self.set_mode(Mode::Normal);
                    ExtendedAction::Enter(Mode::Normal)
                }
                _ => ExtendedAction::Passthrough,
            };
        }
        if self.command {
            return self.key_command(key);
        }
        if self.visual.is_some() {
            return self.key_visual(key);
        }
        self.key_normal(key)
    }

    fn key_command(&mut self, key: Key) -> ExtendedAction {
        match key {
            Key::Escape => {
                self.command = false;
                ExtendedAction::Enter(Mode::Normal)
            }
            Key::Enter => {
                self.command = false;
                ExtendedAction::Enter(Mode::Normal)
            }
            _ => ExtendedAction::CommandInput(key),
        }
    }

    fn key_visual(&mut self, key: Key) -> ExtendedAction {
        if matches!(key, Key::Escape) {
            self.visual = None;
            self.visual_anchor = None;
            self.clear_pending();
            return ExtendedAction::Visual(VisualAction::Exit);
        }
        let Key::Char(c) = key else {
            return ExtendedAction::None;
        };
        if c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
            self.count = Some(
                self.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(c.to_digit(10).unwrap() as usize)
                    .min(MAX_COUNT),
            );
            return ExtendedAction::None;
        }
        if let Some(Pending::Find {
            direction,
            till,
            count,
        }) = self.pending
        {
            self.pending = None;
            if c.is_ascii() {
                self.last_find = Some((direction, till, c));
                return ExtendedAction::Visual(VisualAction::Move(
                    find_motion(direction, till, c),
                    count,
                ));
            }
        }
        if let Some(Pending::TextObject(_, around, _)) = self.pending {
            self.pending = None;
            let object = match (around, c) {
                (_, 'w') => Some(if around {
                    TextObject::AroundWord
                } else {
                    TextObject::InnerWord
                }),
                (_, 'p') => Some(if around {
                    TextObject::AroundParagraph
                } else {
                    TextObject::InnerParagraph
                }),
                (false, '"') => Some(TextObject::InnerQuote),
                (false, '(') => Some(TextObject::InnerParen),
                (_, 'h') => Some(TextObject::InnerHeading),
                _ => None,
            };
            return object.map_or(ExtendedAction::None, |object| {
                ExtendedAction::Visual(VisualAction::TextObject(object))
            });
        }
        if c == 'v' {
            if self.visual == Some(VisualMode::Char) {
                self.visual = None;
                self.visual_anchor = None;
                return ExtendedAction::Visual(VisualAction::Exit);
            }
            self.visual = Some(VisualMode::Char);
            return ExtendedAction::Visual(VisualAction::Enter(VisualMode::Char));
        }
        if c == 'V' {
            if self.visual == Some(VisualMode::Line) {
                self.visual = None;
                self.visual_anchor = None;
                return ExtendedAction::Visual(VisualAction::Exit);
            }
            self.visual = Some(VisualMode::Line);
            return ExtendedAction::Visual(VisualAction::Enter(VisualMode::Line));
        }
        if is_motion_key(c) {
            let count = self.take_count();
            if let Some(motion) = parse_motion(c) {
                return ExtendedAction::Visual(VisualAction::Move(motion, count));
            }
        }
        match c {
            'd' => {
                self.visual = None;
                self.visual_anchor = None;
                ExtendedAction::Visual(VisualAction::Operate(Operator::Delete, self.take_count()))
            }
            'y' => {
                self.visual = None;
                self.visual_anchor = None;
                ExtendedAction::Visual(VisualAction::Operate(Operator::Yank, self.take_count()))
            }
            'c' => {
                self.visual = None;
                self.visual_anchor = None;
                ExtendedAction::Visual(VisualAction::Operate(Operator::Change, self.take_count()))
            }
            'f' | 't' | 'F' | 'T' => {
                self.pending = Some(Pending::Find {
                    direction: if c.is_uppercase() { -1 } else { 1 },
                    till: c.eq_ignore_ascii_case(&'t'),
                    count: self.take_count(),
                });
                ExtendedAction::None
            }
            'i' | 'a' => {
                self.pending = Some(Pending::TextObject(Operator::Change, c == 'a', 1));
                ExtendedAction::None
            }
            _ => ExtendedAction::None,
        }
    }

    fn key_normal(&mut self, key: Key) -> ExtendedAction {
        if let Some(pending) = self.pending {
            return self.key_pending(pending, key);
        }
        if let Key::Control(c) = key {
            return match c {
                'r' => ExtendedAction::Redo,
                _ => ExtendedAction::Passthrough,
            };
        }
        let Key::Char(c) = key else {
            self.clear_pending();
            return ExtendedAction::None;
        };
        if c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
            let digit = c.to_digit(10).unwrap() as usize;
            self.count = Some(
                self.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit)
                    .min(MAX_COUNT),
            );
            return ExtendedAction::None;
        }
        if matches!(c, 'g') {
            self.pending = Some(Pending::Prefix(c));
            return ExtendedAction::None;
        }
        if let Some(operator) = operator(c) {
            self.pending = Some(Pending::Operator(operator, self.take_count()));
            return ExtendedAction::OperatorPending(operator, 1);
        }
        let count = self.take_count();
        match c {
            'h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | '0' | '^' | '$' | 'G' | '{' | '}' => {
                ExtendedAction::Move(parse_motion(c).unwrap(), count)
            }
            'f' | 't' | 'F' | 'T' => {
                self.pending = Some(Pending::Find {
                    direction: if c.is_uppercase() { -1 } else { 1 },
                    till: c.eq_ignore_ascii_case(&'t'),
                    count,
                });
                ExtendedAction::None
            }
            ';' | ',' => self.repeat_find(c == ';', count),
            'v' => {
                self.visual = Some(VisualMode::Char);
                ExtendedAction::Visual(VisualAction::Enter(VisualMode::Char))
            }
            'V' => {
                self.visual = Some(VisualMode::Line);
                ExtendedAction::Visual(VisualAction::Enter(VisualMode::Line))
            }
            'i' | 'a' => ExtendedAction::InsertAt(if c == 'a' {
                Motion::Append
            } else {
                Motion::Here
            }),
            'I' => ExtendedAction::InsertAt(Motion::FirstNonBlank),
            'A' => ExtendedAction::InsertAt(Motion::LineEnd),
            'o' => ExtendedAction::Edit(Edit::OpenBelow, count),
            'O' => ExtendedAction::Edit(Edit::OpenAbove, count),
            'x' => ExtendedAction::Edit(Edit::DeleteChar, count),
            's' => ExtendedAction::Substitute(count),
            'u' => ExtendedAction::Undo,
            '.' => ExtendedAction::Repeat,
            'n' => ExtendedAction::Search {
                forward: true,
                count,
            },
            'N' => ExtendedAction::Search {
                forward: false,
                count,
            },
            'p' => ExtendedAction::Paste {
                before: false,
                count,
            },
            'P' => ExtendedAction::Paste {
                before: true,
                count,
            },
            '/' | '?' => {
                self.command = true;
                ExtendedAction::Enter(Mode::Normal)
            }
            ' ' => ExtendedAction::Leader,
            _ => ExtendedAction::Passthrough,
        }
    }

    fn key_pending(&mut self, pending: Pending, key: Key) -> ExtendedAction {
        match pending {
            Pending::Prefix(prefix) => {
                self.pending = None;
                let count = self.take_count();
                match (prefix, key) {
                    ('g', Key::Char('g')) => ExtendedAction::Move(Motion::FirstLine, count),
                    _ => ExtendedAction::None,
                }
            }
            Pending::Operator(operator, operator_count) => {
                if let Key::Char(c) = key {
                    if c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
                        self.count = Some(
                            self.count
                                .unwrap_or(0)
                                .saturating_mul(10)
                                .saturating_add(c.to_digit(10).unwrap() as usize)
                                .min(MAX_COUNT),
                        );
                        return ExtendedAction::None;
                    }
                    if let Some(target) = parse_motion(c) {
                        self.pending = None;
                        let motion_count = self.take_count();
                        return ExtendedAction::Operate(
                            operator,
                            OperatorTarget::Motion(target),
                            operator_count.saturating_mul(motion_count),
                        );
                    }
                    if c == 'i' || c == 'a' {
                        self.pending =
                            Some(Pending::TextObject(operator, c == 'a', operator_count));
                        return ExtendedAction::None;
                    }
                    if matches!(c, 'f' | 't' | 'F' | 'T') {
                        self.pending = Some(Pending::OperatorFind {
                            operator,
                            operator_count,
                            direction: if c.is_uppercase() { -1 } else { 1 },
                            till: c.eq_ignore_ascii_case(&'t'),
                            count: self.take_count(),
                        });
                        return ExtendedAction::None;
                    }
                    if c == 'g' {
                        self.pending = Some(Pending::OperatorPrefix(operator, operator_count));
                        return ExtendedAction::None;
                    }
                    if c == 'd' || c == 'y' || c == 'c' {
                        self.pending = None;
                        return ExtendedAction::Operate(
                            operator,
                            OperatorTarget::Line,
                            operator_count.saturating_mul(self.take_count()),
                        );
                    }
                }
                self.pending = None;
                ExtendedAction::None
            }
            Pending::OperatorPrefix(operator, operator_count) => {
                self.pending = None;
                if key == Key::Char('g') {
                    return ExtendedAction::Operate(
                        operator,
                        OperatorTarget::Motion(Motion::FirstLine),
                        operator_count.saturating_mul(self.take_count()),
                    );
                }
                ExtendedAction::None
            }
            Pending::TextObject(operator, around, operator_count) => {
                let Key::Char(c) = key else {
                    self.pending = None;
                    return ExtendedAction::None;
                };
                let object = match (around, c) {
                    (_, 'w') => Some(if around {
                        TextObject::AroundWord
                    } else {
                        TextObject::InnerWord
                    }),
                    (_, 'p') => Some(if around {
                        TextObject::AroundParagraph
                    } else {
                        TextObject::InnerParagraph
                    }),
                    (false, '"') => Some(TextObject::InnerQuote),
                    (false, '(') => Some(TextObject::InnerParen),
                    (_, 'h') => Some(TextObject::InnerHeading),
                    _ => None,
                };
                self.pending = None;
                object.map_or(ExtendedAction::None, |object| {
                    ExtendedAction::Operate(
                        operator,
                        OperatorTarget::TextObject(object),
                        operator_count.saturating_mul(self.take_count()),
                    )
                })
            }
            Pending::Find {
                direction,
                till,
                count,
            } => {
                let Key::Char(target) = key else {
                    self.pending = None;
                    return ExtendedAction::None;
                };
                self.pending = None;
                self.last_find = Some((direction, till, target));
                ExtendedAction::Move(find_motion(direction, till, target), count)
            }
            Pending::OperatorFind {
                operator,
                operator_count,
                direction,
                till,
                count,
            } => {
                if let Key::Char(c) = key
                    && c.is_ascii_digit()
                    && (c != '0' || self.count.is_some())
                {
                    let next = self
                        .count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(c.to_digit(10).unwrap() as usize)
                        .min(MAX_COUNT);
                    self.pending = Some(Pending::OperatorFind {
                        operator,
                        operator_count,
                        direction,
                        till,
                        count: next,
                    });
                    self.count = None;
                    return ExtendedAction::None;
                }
                let Key::Char(target) = key else {
                    self.pending = None;
                    return ExtendedAction::None;
                };
                self.pending = None;
                self.last_find = Some((direction, till, target));
                ExtendedAction::Operate(
                    operator,
                    OperatorTarget::Motion(find_motion(direction, till, target)),
                    operator_count.saturating_mul(count),
                )
            }
        }
    }

    fn repeat_find(&self, forward: bool, count: usize) -> ExtendedAction {
        let Some((direction, till, target)) = self.last_find else {
            return ExtendedAction::None;
        };
        let direction = if forward { direction } else { -direction };
        ExtendedAction::Move(find_motion(direction, till, target), count)
    }

    fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1)
    }

    fn clear_pending(&mut self) {
        self.count = None;
        self.pending = None;
    }
}

fn operator(c: char) -> Option<Operator> {
    match c {
        'd' => Some(Operator::Delete),
        'y' => Some(Operator::Yank),
        'c' => Some(Operator::Change),
        _ => None,
    }
}

fn is_motion_key(c: char) -> bool {
    parse_motion(c).is_some()
}

fn parse_motion(c: char) -> Option<Motion> {
    Some(match c {
        'h' => Motion::Left,
        'j' => Motion::Down,
        'k' => Motion::Up,
        'l' => Motion::Right,
        'w' => Motion::WordForward,
        'b' => Motion::WordBack,
        'e' => Motion::WordEnd,
        '0' => Motion::LineStart,
        '^' => Motion::FirstNonBlank,
        '$' => Motion::LineEnd,
        'G' => Motion::LastLine,
        '{' => Motion::ParagraphBack,
        '}' => Motion::ParagraphForward,
        _ => return None,
    })
}

fn find_motion(direction: i8, till: bool, target: char) -> Motion {
    match (direction, till) {
        (1, false) => Motion::FindForward(target),
        (1, true) => Motion::TillForward(target),
        (-1, false) => Motion::FindBack(target),
        (-1, true) => Motion::TillBack(target),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, Inline, Style, Text};
    use std::path::Path;

    fn range_doc(text: &str) -> Document {
        let mut doc = Document::new(Path::new("test.md"));
        *doc.body_mut() = vec![Block::Paragraph(vec![Inline::Text(Text {
            text: text.into(),
            style: Style::PLAIN,
        })])];
        doc
    }

    #[test]
    fn counts_and_legacy_combos_survive() {
        let mut vim = Vim::new();
        assert_eq!(vim.key(Key::Char('3')), Action::None);
        assert_eq!(vim.key(Key::Char('l')), Action::Move(Motion::Right, 3));
        assert_eq!(vim.key(Key::Char('g')), Action::None);
        assert_eq!(vim.key(Key::Char('g')), Action::Move(Motion::FirstLine, 1));
        assert_eq!(vim.key(Key::Char('2')), Action::None);
        assert_eq!(vim.key(Key::Char('d')), Action::None);
        assert_eq!(vim.key(Key::Char('d')), Action::Edit(Edit::DeleteLine, 2));
    }

    #[test]
    fn operators_text_objects_find_and_visual_parse() {
        let mut vim = Vim::new();
        assert_eq!(
            vim.key_extended(Key::Char('d')),
            ExtendedAction::OperatorPending(Operator::Delete, 1)
        );
        assert_eq!(vim.key_extended(Key::Char('i')), ExtendedAction::None);
        assert_eq!(
            vim.key_extended(Key::Char('w')),
            ExtendedAction::Operate(
                Operator::Delete,
                OperatorTarget::TextObject(TextObject::InnerWord),
                1
            )
        );
        assert_eq!(vim.key_extended(Key::Char('f')), ExtendedAction::None);
        assert_eq!(
            vim.key_extended(Key::Char('x')),
            ExtendedAction::Move(Motion::FindForward('x'), 1)
        );
        assert_eq!(
            vim.key_extended(Key::Char(';')),
            ExtendedAction::Move(Motion::FindForward('x'), 1)
        );
        assert_eq!(
            vim.key_extended(Key::Char('v')),
            ExtendedAction::Visual(VisualAction::Enter(VisualMode::Char))
        );
        assert_eq!(vim.visual_mode(), Some(VisualMode::Char));
        assert_eq!(vim.current_mode(), VimMode::VisualChar);
        assert_eq!(
            vim.key_extended(Key::Char('/')),
            ExtendedAction::None,
            "visual mode consumes slash as an unmapped key"
        );
        vim.key_extended(Key::Escape);
        assert_eq!(
            vim.key_extended(Key::Char('/')),
            ExtendedAction::Enter(Mode::Normal)
        );
        assert_eq!(vim.current_mode(), VimMode::Command);
    }

    #[test]
    fn pending_counts_multiply_after_operators() {
        let mut vim = Vim::new();
        for key in ['d', '2', 'w'] {
            let action = vim.key_extended(Key::Char(key));
            if key == 'w' {
                assert_eq!(
                    action,
                    ExtendedAction::Operate(
                        Operator::Delete,
                        OperatorTarget::Motion(Motion::WordForward),
                        2
                    )
                );
            }
        }

        let mut vim = Vim::new();
        for key in ['2', 'd', '3', 'w'] {
            let action = vim.key_extended(Key::Char(key));
            if key == 'w' {
                assert_eq!(
                    action,
                    ExtendedAction::Operate(
                        Operator::Delete,
                        OperatorTarget::Motion(Motion::WordForward),
                        6
                    )
                );
            }
        }

        let mut vim = Vim::new();
        for key in ['d', 'f', '2', 'x'] {
            let action = vim.key_extended(Key::Char(key));
            if key == 'x' {
                assert_eq!(
                    action,
                    ExtendedAction::Operate(
                        Operator::Delete,
                        OperatorTarget::Motion(Motion::FindForward('x')),
                        2
                    )
                );
            }
        }

        let mut vim = Vim::new();
        let mut last = ExtendedAction::None;
        for key in ['d', '2', 'd'] {
            last = vim.key_extended(Key::Char(key));
        }
        assert_eq!(
            last,
            ExtendedAction::Operate(Operator::Delete, OperatorTarget::Line, 2)
        );

        let mut vim = Vim::new();
        assert_eq!(
            vim.key_extended(Key::Char('d')),
            ExtendedAction::OperatorPending(Operator::Delete, 1)
        );
        assert_eq!(vim.key_extended(Key::Char('g')), ExtendedAction::None);
        assert_eq!(
            vim.key_extended(Key::Char('g')),
            ExtendedAction::Operate(
                Operator::Delete,
                OperatorTarget::Motion(Motion::FirstLine),
                1
            )
        );
    }

    #[test]
    fn visual_counts_move_and_toggle() {
        let mut vim = Vim::new();
        assert_eq!(
            vim.key_extended(Key::Char('v')),
            ExtendedAction::Visual(VisualAction::Enter(VisualMode::Char))
        );
        assert_eq!(vim.key_extended(Key::Char('2')), ExtendedAction::None);
        assert_eq!(
            vim.key_extended(Key::Char('l')),
            ExtendedAction::Visual(VisualAction::Move(Motion::Right, 2))
        );
        assert_eq!(
            vim.key_extended(Key::Char('v')),
            ExtendedAction::Visual(VisualAction::Exit)
        );

        assert_eq!(
            vim.key_extended(Key::Char('V')),
            ExtendedAction::Visual(VisualAction::Enter(VisualMode::Line))
        );
        assert_eq!(vim.key_extended(Key::Char('2')), ExtendedAction::None);
        assert_eq!(
            vim.key_extended(Key::Char('j')),
            ExtendedAction::Visual(VisualAction::Move(Motion::Down, 2))
        );
        assert_eq!(
            vim.key_extended(Key::Char('V')),
            ExtendedAction::Visual(VisualAction::Exit)
        );
    }

    #[test]
    fn operator_ranges_match_inclusive_exclusive_and_reverse_motions() {
        let doc = range_doc("abcx");
        let original = FlatPos {
            block: 0,
            offset: 0,
        };
        assert_eq!(
            doc.range_text(Vim::operator_range(
                &doc,
                original,
                Motion::FindForward('x'),
                FlatPos {
                    block: 0,
                    offset: 3
                },
            )),
            "abcx"
        );
        assert_eq!(
            doc.range_text(Vim::operator_range(
                &doc,
                original,
                Motion::TillForward('x'),
                FlatPos {
                    block: 0,
                    offset: 2
                },
            )),
            "abc"
        );
        assert_eq!(
            doc.range_text(Vim::operator_range(
                &doc,
                FlatPos {
                    block: 0,
                    offset: 2
                },
                Motion::Left,
                FlatPos {
                    block: 0,
                    offset: 1
                },
            )),
            "b"
        );

        let mut backwards = range_doc("one two");
        backwards.set_flat_position(FlatPos {
            block: 0,
            offset: 4,
        });
        let start = backwards.caret_position();
        motion::apply(&mut backwards, Motion::WordBack, 1);
        assert_eq!(
            backwards.range_text(Vim::operator_range(
                &backwards,
                start,
                Motion::WordBack,
                backwards.caret_position()
            )),
            "one "
        );

        let mut lines = range_doc("first");
        lines
            .body_mut()
            .push(Block::Paragraph(vec![Inline::Text(Text {
                text: "second".into(),
                style: Style::PLAIN,
            })]));
        assert_eq!(
            lines.range_text(Vim::operator_range(
                &lines,
                FlatPos {
                    block: 1,
                    offset: 2,
                },
                Motion::FirstLine,
                FlatPos {
                    block: 0,
                    offset: 0
                },
            )),
            "first\nsecond"
        );

        let mut words = range_doc("one two");
        words.set_flat_position(FlatPos {
            block: 0,
            offset: 4,
        });
        let start = words.caret_position();
        motion::apply(&mut words, Motion::WordEnd, 1);
        assert_eq!(
            words.range_text(Vim::operator_range(
                &words,
                start,
                Motion::WordEnd,
                words.caret_position()
            )),
            "two"
        );
    }
}
