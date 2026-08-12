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
        id: "vault.open",
        title: "Open vault folder…",
        group: "Vault",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyO,
        }),
        run: |shell| shell.open_picker(),
    },
    Command {
        id: "view.tree",
        title: "Toggle file tree",
        group: "View",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::Digit1,
        }),
        run: |shell| shell.panels_mut()[0].toggle(),
    },
    Command {
        id: "view.sidenotes",
        title: "Toggle sidenotes",
        group: "View",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::Digit2,
        }),
        run: |shell| shell.panels_mut()[1].toggle(),
    },
    Command {
        id: "view.topics",
        title: "Toggle topics",
        group: "View",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::Digit3,
        }),
        run: |shell| shell.panels_mut()[2].toggle(),
    },
    Command {
        id: "view.status",
        title: "Toggle status line",
        group: "View",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::Digit4,
        }),
        run: |shell| shell.panels_mut()[3].toggle(),
    },
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
pub fn entries() -> Vec<palette::Entry> {
    COMMANDS
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
    COMMANDS.iter().filter(|c| c.group == "Format").collect()
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
        assert_eq!(entries.len(), COMMANDS.len());
        for (entry, command) in entries.iter().zip(COMMANDS) {
            assert_eq!(entry.title, command.title);
        }
    }

    #[test]
    fn editor_commands_are_only_format_entries_in_table_order() {
        let in_table: Vec<&Command> = COMMANDS.iter().filter(|c| c.group == "Format").collect();
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
    }
}
