//! Typewritter's platform layer — vendored from Atomos (see `AGENTS.md`).
//!
//! Kept as a library target rather than modules of the binary so the
//! platform's full API is genuinely public: unused-yet draw calls, easings,
//! and input queries are API surface, not dead code.

pub mod animation;
pub mod frame;
pub mod input;
pub mod renderer;

// Typewritter's own.
pub mod canvas;
pub mod clipboard;
pub mod components;
pub mod config;
pub mod document;
pub mod export;
pub mod layout;
pub mod prose;
pub mod search;
pub mod shell;
pub mod tabs;
pub mod theme;
pub mod ui;
pub mod vault;
pub mod vim;
