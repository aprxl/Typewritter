//! The OS clipboard.
//!
//! Clipboard access stays here so the document and tab model remain pure and
//! deterministic. The connection is thread-local because display connections
//! belong to the thread that created them, and it is initialized once because
//! opening one can fail on headless systems.

use std::cell::RefCell;

thread_local! {
    // A failed connection is remembered as None instead of being retried on
    // every keystroke.
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> =
        RefCell::new(arboard::Clipboard::new().ok());
}

/// The clipboard's text, or `None` when there is none — or when there is no
/// clipboard at all. A headless test run, a missing display server and an
/// empty clipboard are all the same answer here: nothing to paste.
pub fn get() -> Option<String> {
    CLIPBOARD.with(|clipboard| {
        clipboard
            .borrow_mut()
            .as_mut()
            .and_then(|clipboard| clipboard.get_text().ok())
    })
}

/// Publishes `text`. Failure is silent by design: a note-taking app must not
/// interrupt writing because a clipboard owner went away.
///
/// On X11 the clipboard's contents belong to the owning process, so text
/// copied out of Typewritter disappears when Typewritter exits unless a
/// clipboard manager is running. That is platform behaviour, not a bug to
/// work around here.
pub fn set(text: &str) {
    CLIPBOARD.with(|clipboard| {
        if let Some(clipboard) = clipboard.borrow_mut().as_mut() {
            let _ = clipboard.set_text(text);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{get, set};

    #[test]
    fn the_clipboard_never_panics() {
        set("x");
        let _ = get();
    }
}
