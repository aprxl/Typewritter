//! The single table every keybinding lives in, and the source the command
//! palette lists them from.
//!
//! Sits under `shell/` rather than `components/` so `run` can reach the
//! shell's own private methods directly, the same way `input.rs` does —
//! there is nothing here a widget would ever reuse.
//!
//! Every chord below carries at least Ctrl. That's not a style preference:
//! it's what makes a table entry structurally unable to collide with a
//! bare vim key, now or as either table grows — no coordination between
//! the two is required to keep it that way.

use winit::keyboard::{KeyCode, ModifiersState};

use super::Shell;
use crate::components::dialog::Prompt;
use crate::components::palette;
use crate::document::BadgeColor;
use crate::document::math::{AccentKind, BigOp, SymbolRole};
use crate::input::Input;

#[derive(Clone, Copy)]
pub struct Chord {
    pub mods: ModifiersState,
    pub key: KeyCode,
}

impl Chord {
    /// "Ctrl+N", "Ctrl+Shift+C" — built from whichever modifier bits are
    /// actually set, so a chord that grows a modifier later doesn't need
    /// its label rewritten by hand.
    pub fn label(&self) -> String {
        let mut label = String::new();
        if self.mods.control_key() {
            label.push_str("Ctrl+");
        }
        if self.mods.alt_key() {
            label.push_str("Alt+");
        }
        if self.mods.shift_key() {
            label.push_str("Shift+");
        }
        if self.mods.super_key() {
            label.push_str("Super+");
        }
        label.push_str(key_label(self.key));
        label
    }
}

fn key_label(key: KeyCode) -> &'static str {
    match key {
        KeyCode::KeyN => "N",
        KeyCode::KeyS => "S",
        KeyCode::KeyW => "W",
        KeyCode::KeyD => "D",
        KeyCode::KeyO => "O",
        KeyCode::KeyC => "C",
        KeyCode::KeyX => "X",
        KeyCode::KeyV => "V",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        _ => "?",
    }
}

pub struct Command {
    /// Stable identifier, checked by this file's own tests and used by
    /// `input.rs` to resolve the onboarding vault-open command out of the
    /// table instead of matching its chord literally.
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub chord: Option<Chord>,
    pub run: fn(&mut Shell),
}

const CTRL: ModifiersState = ModifiersState::CONTROL;
const CTRL_SHIFT: ModifiersState = ModifiersState::CONTROL.union(ModifiersState::SHIFT);

pub const COMMANDS: &[Command] = &[
    Command {
        id: "file.new",
        title: "New note",
        group: "File",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyN,
        }),
        run: |shell| {
            shell.open_dialog(Prompt::NewNote {
                input: Vec::new(),
                caret: 0,
            });
        },
    },
    Command {
        id: "folder.new",
        title: "New folder",
        group: "File",
        chord: Some(Chord {
            mods: CTRL_SHIFT,
            key: KeyCode::KeyN,
        }),
        run: |shell| {
            shell.open_dialog(Prompt::NewFolder {
                input: Vec::new(),
                caret: 0,
            });
        },
    },
    Command {
        id: "file.save",
        title: "Save note",
        group: "File",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyS,
        }),
        // Saving clears the dirty flags; the revision bump that follows is
        // what the views rebuild from.
        run: |shell| {
            let _ = shell.docs.borrow_mut().save_active();
        },
    },
    Command {
        id: "file.open",
        title: "Open File",
        group: "File",
        chord: None,
        run: |shell| shell.open_finder(),
    },
    Command {
        id: "file.close",
        title: "Close note",
        group: "File",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyW,
        }),
        run: |shell| shell.docs.borrow_mut().close_active(),
    },
    Command {
        id: "file.delete",
        title: "Delete note",
        group: "File",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyD,
        }),
        run: |shell| shell.ask_delete_selected(),
    },
    Command {
        id: "edit.copy",
        title: "Copy",
        group: "Edit",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyC,
        }),
        run: |shell| shell.copy_selection(false),
    },
    Command {
        id: "edit.cut",
        title: "Cut",
        group: "Edit",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyX,
        }),
        run: |shell| shell.copy_selection(true),
    },
    Command {
        id: "edit.paste",
        title: "Paste",
        group: "Edit",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyV,
        }),
        run: |shell| shell.paste_clipboard(),
    },
    Command {
        id: "vault.open",
        title: "Open vault folder…",
        group: "Vault",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyO,
        }),
        run: |shell| shell.open_picker(),
    },
    // Ctrl+1..4 used to toggle the four regions. Those chords now pick a
    // row in the finder (Ctrl+1..5), the only thing those keys do; the
    // toggles themselves were reachable only from here and went with them.
    Command {
        id: "view.capture",
        title: "Capture mode",
        group: "View",
        chord: Some(Chord {
            mods: CTRL_SHIFT,
            key: KeyCode::KeyC,
        }),
        // Spec §3.2: one key to bare text and back — open if any panel is
        // closed, close all of them if every panel is already open.
        run: |shell| {
            let any_open = shell.panels().iter().any(|p| p.open);
            for panel in shell.panels_mut() {
                panel.set_open(!any_open);
            }
        },
    },
    Command {
        id: "view.stats",
        title: "Toggle render stats",
        group: "View",
        chord: None,
        run: |shell| shell.show_stats = !shell.show_stats,
    },
    Command {
        id: "view.rows",
        title: "Toggle row hit bands",
        group: "View",
        chord: None,
        run: |shell| shell.debug_rows = !shell.debug_rows,
    },
    Command {
        id: "format.body",
        title: "Body text",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_heading(None),
    },
    Command {
        id: "format.h1",
        title: "Heading 1",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_heading(Some(1)),
    },
    Command {
        id: "format.h2",
        title: "Heading 2",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_heading(Some(2)),
    },
    Command {
        id: "format.h3",
        title: "Heading 3",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_heading(Some(3)),
    },
    Command {
        id: "format.h4",
        title: "Heading 4",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_heading(Some(4)),
    },
    Command {
        id: "format.bold",
        title: "Bold",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_bold(),
    },
    Command {
        id: "format.italic",
        title: "Italic",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_italic(),
    },
    Command {
        id: "format.code",
        title: "Code block",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_code(true),
    },
    Command {
        id: "format.divider",
        title: "Divider",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().insert_divider(),
    },
    Command {
        // Spec §5: a sidenote is a block like the rest, reachable from the
        // same `/` menu. No chord: the obvious ones are taken, and the menu
        // is enough on its own.
        id: "format.sidenote",
        title: "Sidenote",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().insert_sidenote(),
    },
    Command {
        id: "note.edit",
        title: "Edit sidenote",
        group: "Format",
        chord: None,
        run: |shell| shell.edit_note_at_caret(),
    },
    Command {
        // SPEC §6.1: <leader>m once a leader-binding mechanism exists.
        id: "format.math",
        title: "Math",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().insert_inline_math(),
    },
    Command {
        id: "format.math_block",
        title: "Math block",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().insert_math_block(),
    },
    Command {
        // One command for both directions: a tagged equation loses its tag,
        // an untagged one gains the next free `#eq:N`. The numbers on the
        // page are derived at layout time in document order, so nothing
        // stored here can drift from what the reader sees.
        id: "format.math_tag",
        title: "Equation tag",
        group: "Format",
        chord: None,
        run: |shell| {
            shell.docs.borrow_mut().toggle_math_tag();
        },
    },
    Command {
        id: "format.inline_code",
        title: "Inline code",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_code(),
    },
    Command {
        id: "format.badge",
        title: "Badge",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_badge(),
    },
    Command {
        id: "format.highlight",
        title: "Highlight",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_highlight(),
    },
    Command {
        id: "context.bold",
        title: "Bold",
        group: "Format",
        chord: None,
        run: |shell| shell.context_toggle_bold(),
    },
    Command {
        id: "context.italic",
        title: "Italic",
        group: "Format",
        chord: None,
        run: |shell| shell.context_toggle_italic(),
    },
    Command {
        id: "context.highlight",
        title: "Highlight",
        group: "Format",
        chord: None,
        run: |shell| shell.context_toggle_highlight(),
    },
    Command {
        id: "context.inline_code",
        title: "Inline code",
        group: "Type",
        chord: None,
        run: |shell| shell.context_toggle_inline_code(),
    },
    Command {
        id: "context.badge",
        title: "Badge",
        group: "Type",
        chord: None,
        run: |shell| shell.context_toggle_badge(),
    },
    Command {
        id: "context.body",
        title: "Body text",
        group: "Type",
        chord: None,
        run: |shell| shell.context_set_heading(None),
    },
    Command {
        id: "context.badge.orange",
        title: "Orange",
        group: "Color",
        chord: None,
        run: |shell| shell.context_set_badge_color(BadgeColor::Orange),
    },
    Command {
        id: "context.badge.blue",
        title: "Blue",
        group: "Color",
        chord: None,
        run: |shell| shell.context_set_badge_color(BadgeColor::Blue),
    },
    Command {
        id: "context.badge.green",
        title: "Green",
        group: "Color",
        chord: None,
        run: |shell| shell.context_set_badge_color(BadgeColor::Green),
    },
    Command {
        id: "context.badge.purple",
        title: "Purple",
        group: "Color",
        chord: None,
        run: |shell| shell.context_set_badge_color(BadgeColor::Purple),
    },
    Command {
        id: "context.symbol.variable",
        title: "Variable",
        group: "Role",
        chord: None,
        run: |shell| shell.context_set_math_role(SymbolRole::Variable),
    },
    Command {
        id: "context.symbol.constant",
        title: "Constant",
        group: "Role",
        chord: None,
        run: |shell| shell.context_set_math_role(SymbolRole::Constant),
    },
    Command {
        id: "context.symbol.function",
        title: "Function",
        group: "Role",
        chord: None,
        run: |shell| shell.context_set_math_role(SymbolRole::Function),
    },
    Command {
        id: "context.variant.plain",
        title: "Plain",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("plain"),
    },
    Command {
        id: "context.variant.bold",
        title: "Bold",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("bold"),
    },
    Command {
        id: "context.variant.italic",
        title: "Italic",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("italic"),
    },
    Command {
        id: "context.variant.bold_italic",
        title: "Bold italic",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("bold_italic"),
    },
    Command {
        id: "context.variant.sans",
        title: "Sans serif",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("sans"),
    },
    Command {
        id: "context.variant.sans_bold",
        title: "Sans bold",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("sans_bold"),
    },
    Command {
        id: "context.variant.sans_italic",
        title: "Sans italic",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("sans_italic"),
    },
    Command {
        id: "context.variant.sans_bold_italic",
        title: "Sans bold italic",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("sans_bold_italic"),
    },
    Command {
        id: "context.variant.monospace",
        title: "Monospace",
        group: "Variant",
        chord: None,
        run: |shell| shell.context_set_math_variant("monospace"),
    },
    Command {
        id: "context.group.parentheses",
        title: "Parentheses",
        group: "Delimiter",
        chord: None,
        run: |shell| shell.context_set_math_delimiter('('),
    },
    Command {
        id: "context.group.brackets",
        title: "Square brackets",
        group: "Delimiter",
        chord: None,
        run: |shell| shell.context_set_math_delimiter('['),
    },
    Command {
        id: "context.group.bars",
        title: "Absolute value",
        group: "Delimiter",
        chord: None,
        run: |shell| shell.context_set_math_delimiter('|'),
    },
    Command {
        id: "context.group.double_bars",
        title: "Norm",
        group: "Delimiter",
        chord: None,
        run: |shell| shell.context_set_math_delimiter('‖'),
    },
    Command {
        id: "context.group.angles",
        title: "Angle brackets",
        group: "Delimiter",
        chord: None,
        run: |shell| shell.context_set_math_delimiter('⟨'),
    },
    Command {
        id: "context.accent.vector",
        title: "Vector arrow",
        group: "Accent",
        chord: None,
        run: |shell| shell.context_set_math_accent(AccentKind::Vector),
    },
    Command {
        id: "context.accent.dot",
        title: "Dot",
        group: "Accent",
        chord: None,
        run: |shell| shell.context_set_math_accent(AccentKind::Dot),
    },
    Command {
        id: "context.accent.ddot",
        title: "Double dot",
        group: "Accent",
        chord: None,
        run: |shell| shell.context_set_math_accent(AccentKind::DoubleDot),
    },
    Command {
        id: "context.accent.dddot",
        title: "Triple dot",
        group: "Accent",
        chord: None,
        run: |shell| shell.context_set_math_accent(AccentKind::TripleDot),
    },
    Command {
        id: "context.accent.hat",
        title: "Hat",
        group: "Accent",
        chord: None,
        run: |shell| shell.context_set_math_accent(AccentKind::Hat),
    },
    Command {
        id: "context.accent.bar",
        title: "Bar",
        group: "Accent",
        chord: None,
        run: |shell| shell.context_set_math_accent(AccentKind::Bar),
    },
    Command {
        id: "context.op.sum",
        title: "Sum",
        group: "Operator",
        chord: None,
        run: |shell| shell.context_set_math_big_op(BigOp::Sum),
    },
    Command {
        id: "context.op.product",
        title: "Product",
        group: "Operator",
        chord: None,
        run: |shell| shell.context_set_math_big_op(BigOp::Prod),
    },
    Command {
        id: "context.op.integral",
        title: "Integral",
        group: "Operator",
        chord: None,
        run: |shell| shell.context_set_math_big_op(BigOp::Integral),
    },
    Command {
        id: "context.op.ring_integral",
        title: "Ring integral",
        group: "Operator",
        chord: None,
        run: |shell| shell.context_set_math_big_op(BigOp::ContourIntegral),
    },
    Command {
        id: "context.op.limit",
        title: "Limit",
        group: "Operator",
        chord: None,
        run: |shell| shell.context_set_math_big_op(BigOp::Limit),
    },
];

/// The command whose chord `input` just matched, if any.
pub fn matching(input: &Input) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| {
        command
            .chord
            .is_some_and(|chord| input.is_shortcut_pressed(chord.mods, chord.key))
    })
}

/// The palette's rows, in table order. The shell uses this same function
/// both to draw the list and to resolve a selection back to a `Command`, so
/// a filtered index can never point at the wrong entry.
pub fn palette_commands() -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|command| !command.id.starts_with("context."))
        .collect()
}

pub fn entries() -> Vec<palette::Entry> {
    palette_commands()
        .iter()
        .map(|command| palette::Entry {
            title: command.title.to_string(),
            group: command.group.to_string(),
            hint: command.chord.map(|c| c.label()).unwrap_or_default(),
        })
        .collect()
}

/// The slash menu's rows: Format-group commands only — insert a heading,
/// toggle bold, etc. Never file/vault/view operations; those stay in the
/// full palette. Same table-order guarantee as `entries()`, but pre-
/// filtered, so a selection index still resolves back to the right
/// `Command` via `editor_commands()[i]`.
pub fn editor_commands() -> Vec<&'static Command> {
    palette_commands()
        .into_iter()
        .filter(|c| c.group == "Format")
        .collect()
}

/// Palette-style entries for the slash menu, mirroring how `entries()`
/// maps `COMMANDS` into `Vec<palette::Entry>`.
pub fn editor_entries() -> Vec<palette::Entry> {
    editor_commands()
        .iter()
        .map(|command| palette::Entry {
            title: command.title.to_string(),
            group: command.group.to_string(),
            hint: command.chord.map(|c| c.label()).unwrap_or_default(),
        })
        .collect()
}

/// The editor's context menu, in the order it is drawn. Ids rather than a
/// second table: a menu row and its chord then cannot describe different
/// behaviour from the palette entry with the same name.
pub const EDITOR_MENU: &[&str] = &["edit.cut", "edit.copy", "edit.paste"];

pub const WORD_MENU: &[&str] = &[
    "context.bold",
    "context.italic",
    "context.highlight",
    "context.inline_code",
    "context.badge",
];

pub const BADGE_MENU: &[&str] = &[
    "context.badge.orange",
    "context.badge.blue",
    "context.badge.green",
    "context.badge.purple",
    "context.badge",
];

pub const INLINE_CODE_MENU: &[&str] = &["context.inline_code", "context.badge"];

pub const CODE_BLOCK_MENU: &[&str] = &["context.body"];

pub const SYMBOL_ROLE_MENU: &[&str] = &[
    "context.symbol.variable",
    "context.symbol.constant",
    "context.symbol.function",
];

pub const GROUP_MENU: &[&str] = &[
    "context.group.parentheses",
    "context.group.brackets",
    "context.group.bars",
    "context.group.double_bars",
    "context.group.angles",
];

pub const ACCENT_MENU: &[&str] = &[
    "context.accent.vector",
    "context.accent.dot",
    "context.accent.ddot",
    "context.accent.dddot",
    "context.accent.hat",
    "context.accent.bar",
];

pub const BIG_OP_MENU: &[&str] = &[
    "context.op.sum",
    "context.op.product",
    "context.op.integral",
    "context.op.ring_integral",
    "context.op.limit",
];

/// The file tree's context menu.
pub const TREE_MENU: &[&str] = &["file.new", "folder.new", "file.delete"];

/// The file tree's context menu over empty space, where there is no file to
/// act on — only the two ways to add one. Deleting is deliberately absent
/// rather than shown greyed out: a row that can never do anything is noise.
pub const TREE_ROOT_MENU: &[&str] = &["file.new", "folder.new"];

/// The commands `ids` name, in the order given. An id with no command is
/// skipped rather than panicking — a menu is a view of the table, and a
/// renamed command must not take the app down with it.
pub fn menu(ids: &[&str]) -> Vec<&'static Command> {
    ids.iter()
        .filter_map(|id| COMMANDS.iter().find(|command| command.id == *id))
        .collect()
}

pub fn menu_entries(ids: &[&str]) -> Vec<palette::Entry> {
    menu(ids)
        .iter()
        .map(|command| palette::Entry {
            title: command.title.to_string(),
            group: command.group.to_string(),
            hint: command.chord.map(|c| c.label()).unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bound_chord_carries_at_least_ctrl() {
        for command in COMMANDS {
            if let Some(chord) = command.chord {
                assert!(
                    chord.mods.control_key(),
                    "{} has a chord without Ctrl, which could collide with a bare vim key",
                    command.id
                );
            }
        }
    }

    #[test]
    fn chord_labels_read_as_the_spec_names_them() {
        assert_eq!(
            COMMANDS
                .iter()
                .find(|c| c.id == "file.new")
                .unwrap()
                .chord
                .unwrap()
                .label(),
            "Ctrl+N"
        );
        assert_eq!(
            COMMANDS
                .iter()
                .find(|c| c.id == "view.capture")
                .unwrap()
                .chord
                .unwrap()
                .label(),
            "Ctrl+Shift+C"
        );
    }

    #[test]
    fn entries_stay_in_table_order_so_a_filtered_index_is_never_off_by_one() {
        let entries = entries();
        let commands = palette_commands();
        assert_eq!(entries.len(), commands.len());
        for (entry, command) in entries.iter().zip(commands) {
            assert_eq!(entry.title, command.title);
        }
        assert!(!entries.iter().any(|entry| entry.group == "Role"));
    }

    #[test]
    fn editor_commands_are_only_format_entries_in_table_order() {
        let in_table: Vec<&Command> = palette_commands()
            .into_iter()
            .filter(|c| c.group == "Format")
            .collect();
        let editor = editor_commands();
        assert_eq!(editor.len(), in_table.len());
        for (e, t) in editor.iter().zip(in_table.iter()) {
            assert_eq!(e.id, t.id);
        }
        assert!(
            editor.iter().all(|c| c.group == "Format"),
            "the slash menu must never list a non-Format command"
        );
        let editor_titles: Vec<String> = editor_entries().iter().map(|e| e.title.clone()).collect();
        let expected: Vec<String> = in_table.iter().map(|c| c.title.to_string()).collect();
        assert_eq!(editor_titles, expected);
        assert!(editor.iter().any(|command| command.id == "format.math"));
        assert!(
            editor
                .iter()
                .any(|command| command.id == "format.math_block")
        );
    }

    #[test]
    fn equation_tag_appears_in_the_editors_slash_menu() {
        assert!(
            editor_commands()
                .iter()
                .any(|command| command.id == "format.math_tag"),
            "tagging an equation is one keystroke away in Insert mode"
        );
    }

    #[test]
    fn sidenote_appears_in_the_editors_slash_menu() {
        // The `/` menu lists every Format-group command; a sidenote is one
        // of them, so creating one is one keystroke away in Insert mode.
        assert!(
            editor_commands()
                .iter()
                .any(|command| command.id == "format.sidenote")
        );
    }

    #[test]
    fn edit_sidenote_is_in_the_command_table_and_names_a_real_command() {
        let command = COMMANDS.iter().find(|command| command.id == "note.edit");
        assert!(command.is_some(), "note.edit must be in COMMANDS");
        let command = command.unwrap();
        assert_eq!(command.title, "Edit sidenote");
        assert_eq!(command.group, "Format");
        assert!(command.chord.is_none());
        assert_eq!(menu(&["note.edit"]).len(), 1);
        assert!(
            editor_commands()
                .iter()
                .any(|entry| entry.id == "note.edit")
        );
    }

    #[test]
    fn every_menu_id_names_a_real_command() {
        for ids in [
            EDITOR_MENU,
            WORD_MENU,
            BADGE_MENU,
            INLINE_CODE_MENU,
            CODE_BLOCK_MENU,
            SYMBOL_ROLE_MENU,
            GROUP_MENU,
            ACCENT_MENU,
            BIG_OP_MENU,
            TREE_MENU,
            TREE_ROOT_MENU,
        ] {
            assert_eq!(menu(ids).len(), ids.len());
        }
    }
}
