//! Typewritter's widgets — one file per component.
//!
//! Every type here implements [`Component`](crate::ui::Component): it
//! measures itself, tracks its own dirty state, and draws into the rect the
//! layout hands it. None of them knows where it sits on screen, and none of
//! them can draw outside its own region — the [`Region`](crate::ui::Region)
//! scissor sees to that.
//!
//! A component owns the metrics that belong to it, so `title_bar::HEIGHT`
//! and `file_tree::WIDTH` live next to the code that draws them rather than
//! in a table the shell has to keep in step.
//!
//! The ones with shared state hold an [`Rc`](std::rc::Rc) to it (the vault,
//! the open tabs) and mutate it directly in `sync`; the shell notices via
//! [`Tabs::revision`](crate::tabs::Tabs::revision) and rebuilds whatever
//! views the change invalidated.

pub mod backdrop;
pub mod breadcrumb;
pub mod context_menu;
pub mod dialog;
pub mod document_surface;
pub mod editor;
pub mod empty_state;
pub mod file_tree;
pub mod format_bar;
pub mod math_menu;
pub mod onboarding;
pub mod palette;
pub mod popup;
pub mod search;
pub mod sidenotes;
pub mod slash_menu;
pub mod status_line;
pub mod tab_strip;
pub mod theme_switch;
pub mod title_bar;
pub mod topics;

pub use backdrop::Backdrop;
pub use breadcrumb::Breadcrumb;
pub use context_menu::ContextMenu;
pub use dialog::Dialog;
pub use editor::Editor;
pub use file_tree::FileTree;
pub use format_bar::FormatBar;
pub use math_menu::MathMenu;
pub use onboarding::Onboarding;
pub use palette::Palette;
pub use search::Finder;
pub use sidenotes::SidenoteMargin;
pub use slash_menu::SlashMenu;
pub use status_line::StatusLine;
pub use tab_strip::TabStrip;
pub use title_bar::TitleBar;
pub use topics::Topics;
