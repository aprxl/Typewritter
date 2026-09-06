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
use crate::document::math::{AccentKind, BigOp, SymbolRole};
use crate::document::math_style::{HighlightShape, MathHue};
use crate::document::{BadgeColor, ListMarker};
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

/// One command per hue and per highlight shape. `Command::run` is a plain
/// function pointer rather than a closure, so each needs a body of its own;
/// the title comes from the same `label()` the menu reads, so a rename
/// cannot leave the two spellings disagreeing.
macro_rules! hue_command {
    ($id:literal, $hue:ident) => {
        Command {
            id: $id,
            title: MathHue::$hue.label(),
            group: "Colour",
            chord: None,
            run: |shell| shell.context_set_math_hue(MathHue::$hue),
        }
    };
}

macro_rules! shape_command {
    ($id:literal, $shape:ident) => {
        Command {
            id: $id,
            title: HighlightShape::$shape.label(),
            group: "Highlight",
            chord: None,
            run: |shell| shell.context_set_math_shape(HighlightShape::$shape),
        }
    };
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
        // what the views rebuild from. Failures remain visible in the status
        // line while the tab stays open for retry.
        run: |shell| shell.save_active(),
    },
    Command {
        id: "file.open",
        title: "Open file",
        group: "File",
        chord: None,
        run: |shell| shell.open_finder(),
    },
    Command {
        id: "file.export_pdf",
        title: "Export as PDF",
        group: "File",
        // No chord: it opens a file dialog and can take a moment on a long
        // note, which is not something a hand should be able to trip into.
        chord: None,
        run: |shell| shell.export_pdf(),
    },
    Command {
        id: "file.close",
        title: "Close note",
        group: "File",
        chord: Some(Chord {
            mods: CTRL,
            key: KeyCode::KeyW,
        }),
        run: |shell| shell.close_active(),
    },
    Command {
        id: "file.discard",
        title: "Discard changes and close note",
        group: "File",
        // Deliberately palette-only: discarding is destructive and should
        // never be a one-chord accident.
        chord: None,
        run: |shell| shell.discard_active(),
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
    Command {
        id: "view.sidebar",
        title: "Toggle sidebar",
        group: "View",
        chord: None,
        run: Shell::toggle_sidebar,
    },
    Command {
        id: "view.theme",
        title: "Switch appearance",
        group: "View",
        chord: None,
        run: |shell| {
            let bar = shell.layout.rect(shell.regions[shell.title_region].node());
            let switch = crate::components::theme_switch::switch_rect(bar);
            shell.request_theme_swap((
                switch.x + switch.width / 2.0,
                switch.y + switch.height / 2.0,
            ));
        },
    },
    Command {
        id: "view.focus",
        title: "Toggle focus mode",
        group: "View",
        chord: Some(Chord {
            mods: CTRL_SHIFT,
            key: KeyCode::KeyC,
        }),
        run: Shell::toggle_focus,
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
        id: "fold.toggle",
        title: "Toggle fold at heading",
        group: "Fold",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_fold(),
    },
    Command {
        id: "fold.open_all",
        title: "Open all folds",
        group: "Fold",
        chord: None,
        run: |shell| shell.docs.borrow_mut().open_all_folds(),
    },
    Command {
        id: "fold.close_all",
        title: "Close all folds",
        group: "Fold",
        chord: None,
        run: |shell| shell.docs.borrow_mut().close_all_folds(),
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
        id: "format.bullet_list",
        title: "Bulleted list",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().set_list(Some(ListMarker::Bullet)),
    },
    Command {
        id: "format.numbered_list",
        title: "Numbered list",
        group: "Format",
        chord: None,
        run: |shell| {
            shell
                .docs
                .borrow_mut()
                .set_list(Some(ListMarker::Number(1)))
        },
    },
    Command {
        id: "format.task_list",
        title: "Task list",
        group: "Format",
        chord: None,
        run: |shell| {
            shell
                .docs
                .borrow_mut()
                .set_list(Some(ListMarker::Task { done: false }))
        },
    },
    Command {
        // The checkbox is also clickable; this is the keyboard route.
        id: "format.task_toggle",
        title: "Toggle task",
        group: "Format",
        chord: None,
        run: |shell| shell.docs.borrow_mut().toggle_task(),
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
        id: "context.code.c",
        title: "C",
        group: "Language",
        chord: None,
        run: |shell| shell.context_code_options(Some(crate::document::code::Language::C), false),
    },
    Command {
        id: "context.code.cpp",
        title: "C++",
        group: "Language",
        chord: None,
        run: |shell| shell.context_code_options(Some(crate::document::code::Language::Cpp), false),
    },
    Command {
        id: "context.code.rust",
        title: "Rust",
        group: "Language",
        chord: None,
        run: |shell| shell.context_code_options(Some(crate::document::code::Language::Rust), false),
    },
    Command {
        id: "context.code.lua",
        title: "Lua",
        group: "Language",
        chord: None,
        run: |shell| shell.context_code_options(Some(crate::document::code::Language::Lua), false),
    },
    Command {
        id: "context.code.python",
        title: "Python",
        group: "Language",
        chord: None,
        run: |shell| {
            shell.context_code_options(Some(crate::document::code::Language::Python), false)
        },
    },
    Command {
        id: "context.code.javascript",
        title: "JavaScript",
        group: "Language",
        chord: None,
        run: |shell| {
            shell.context_code_options(Some(crate::document::code::Language::JavaScript), false)
        },
    },
    Command {
        id: "context.code.typescript",
        title: "TypeScript",
        group: "Language",
        chord: None,
        run: |shell| {
            shell.context_code_options(Some(crate::document::code::Language::TypeScript), false)
        },
    },
    Command {
        id: "context.code.java",
        title: "Java",
        group: "Language",
        chord: None,
        run: |shell| shell.context_code_options(Some(crate::document::code::Language::Java), false),
    },
    Command {
        id: "context.code.csharp",
        title: "C#",
        group: "Language",
        chord: None,
        run: |shell| {
            shell.context_code_options(Some(crate::document::code::Language::CSharp), false)
        },
    },
    Command {
        id: "context.code.go",
        title: "Go",
        group: "Language",
        chord: None,
        run: |shell| shell.context_code_options(Some(crate::document::code::Language::Go), false),
    },
    Command {
        id: "context.code.settings",
        title: "Language and mode…",
        group: "Highlighting",
        chord: None,
        run: |shell| shell.context_code_settings(),
    },
    Command {
        id: "context.code.plain",
        title: "Plain code",
        group: "Highlighting",
        chord: None,
        run: |shell| shell.context_code_options(None, false),
    },
    Command {
        id: "context.code.manual",
        title: "DIY colors",
        group: "Highlighting",
        chord: None,
        run: |shell| shell.context_code_options(None, true),
    },
    Command {
        id: "context.code.color.rose",
        title: "Rose",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Rose)),
    },
    Command {
        id: "context.code.color.coral",
        title: "Coral",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Coral)),
    },
    Command {
        id: "context.code.color.amber",
        title: "Amber",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Amber)),
    },
    Command {
        id: "context.code.color.olive",
        title: "Olive",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Olive)),
    },
    Command {
        id: "context.code.color.green",
        title: "Green",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Green)),
    },
    Command {
        id: "context.code.color.teal",
        title: "Teal",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Teal)),
    },
    Command {
        id: "context.code.color.sky",
        title: "Sky",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Sky)),
    },
    Command {
        id: "context.code.color.indigo",
        title: "Indigo",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Indigo)),
    },
    Command {
        id: "context.code.color.violet",
        title: "Violet",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Violet)),
    },
    Command {
        id: "context.code.color.magenta",
        title: "Magenta",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(Some(MathHue::Magenta)),
    },
    Command {
        id: "context.code.color.clear",
        title: "Clear color",
        group: "Code color",
        chord: None,
        run: |shell| shell.context_code_color(None),
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
    hue_command!("context.hue.rose", Rose),
    hue_command!("context.hue.coral", Coral),
    hue_command!("context.hue.amber", Amber),
    hue_command!("context.hue.olive", Olive),
    hue_command!("context.hue.green", Green),
    hue_command!("context.hue.teal", Teal),
    hue_command!("context.hue.sky", Sky),
    hue_command!("context.hue.indigo", Indigo),
    hue_command!("context.hue.violet", Violet),
    hue_command!("context.hue.magenta", Magenta),
    shape_command!("context.shape.fill", Fill),
    shape_command!("context.shape.outline", Outline),
    shape_command!("context.shape.both", Both),
    Command {
        id: "context.symbol.automatic",
        title: "Back to automatic",
        group: "Symbol",
        chord: None,
        run: |shell| shell.context_reset_math_style(),
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

pub const INLINE_CODE_MENU: &[&str] = &[
    "context.code.plain",
    "context.code.manual",
    "context.code.c",
    "context.code.cpp",
    "context.code.rust",
    "context.code.lua",
    "context.code.python",
    "context.code.javascript",
    "context.code.typescript",
    "context.code.java",
    "context.code.csharp",
    "context.code.go",
    "context.inline_code",
];

pub const CODE_BLOCK_MENU: &[&str] = &[
    "context.code.plain",
    "context.code.manual",
    "context.code.c",
    "context.code.cpp",
    "context.code.rust",
    "context.code.lua",
    "context.code.python",
    "context.code.javascript",
    "context.code.typescript",
    "context.code.java",
    "context.code.csharp",
    "context.code.go",
    "context.body",
];
pub const CODE_COLOR_MENU: &[&str] = &[
    "context.code.color.rose",
    "context.code.color.coral",
    "context.code.color.amber",
    "context.code.color.olive",
    "context.code.color.green",
    "context.code.color.teal",
    "context.code.color.sky",
    "context.code.color.indigo",
    "context.code.color.violet",
    "context.code.color.magenta",
    "context.code.color.clear",
    "context.code.settings",
];

pub const SYMBOL_ROLE_MENU: &[&str] = &[
    "context.symbol.variable",
    "context.symbol.constant",
    "context.symbol.function",
];

/// The ten identity hues, in wheel order — the same order `MathHue::ALL`
/// lists them, so the swatch grid reads as a colour wheel rather than an
/// alphabetised list.
pub const SYMBOL_HUE_MENU: &[&str] = &[
    "context.hue.rose",
    "context.hue.coral",
    "context.hue.amber",
    "context.hue.olive",
    "context.hue.green",
    "context.hue.teal",
    "context.hue.sky",
    "context.hue.indigo",
    "context.hue.violet",
    "context.hue.magenta",
];

pub const SYMBOL_SHAPE_MENU: &[&str] = &[
    "context.shape.fill",
    "context.shape.outline",
    "context.shape.both",
];

/// Putting a symbol back on the hue its letter falls on and the shape its
/// role asks for. One row, because a reader who has customised nothing
/// should still be able to see what automatic looks like.
pub const SYMBOL_AUTOMATIC_MENU: &[&str] = &["context.symbol.automatic"];

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
                .find(|c| c.id == "view.focus")
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
