//! Frame-based input state, fed from winit events.
//!
//! winit delivers input as a stream of one-shot events; almost every caller
//! actually wants to ask *state* questions ("is Ctrl held right now?",
//! "where is the mouse?") at an arbitrary point in a frame. [`Input`] sits
//! between the two: [`Input::handle_event`] absorbs the event stream,
//! everything else is a cheap state query callable from anywhere.
//!
//! The frame contract is the one thing a caller has to get right:
//!
//! 1. forward every `WindowEvent` to [`Input::handle_event`] (and every
//!    `DeviceEvent` to [`Input::handle_device_event`], if raw motion is
//!    wanted),
//! 2. read whatever state the frame needs,
//! 3. call [`Input::end_frame`] exactly once at the end of the frame.
//!
//! `end_frame` is what makes the edge-triggered queries (`is_key_pressed`,
//! `is_key_released`, `mouse_delta`, `scroll_delta`, `text`) mean "since the
//! last frame" rather than "since the program started". Skipping it leaves
//! every edge query permanently true.
//!
//! Composed text — dead keys, system input methods (macOS's dead-key
//! sequences, CJK IMEs, the hold-key accent popover) — arrives as
//! `WindowEvent::Ime` and is folded into the same per-frame `text()` buffer
//! as ordinary typing. winit only delivers those events when IME is allowed,
//! which `Window::set_ime_allowed(true)` (set in `main.rs`) turns on.
//!
//! **Stuck-key safety.** Losing focus mid-chord (alt-tab while holding
//! Ctrl) is the classic way input state desyncs: the OS delivers the press
//! but the release goes to whoever has focus now. [`WindowEvent::Focused`]`(false)`
//! therefore force-releases every held key, button, and modifier — see
//! [`Input::release_all`]. Positions are kept (the cursor didn't move), only
//! held-ness is dropped.

use std::collections::HashSet;

use winit::{
    event::{DeviceEvent, ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
};

/// Logical pixels one "line" of scrolling is worth, used to put
/// [`MouseScrollDelta::PixelDelta`] (touchpads, high-resolution wheels) on
/// the same scale as [`MouseScrollDelta::LineDelta`] (classic notched
/// wheels) so callers only deal with one unit.
// ponytail: one constant instead of surfacing both units separately —
// revisit if smooth-scroll physics ever needs the true pixel deltas.
const PIXELS_PER_LINE: f32 = 20.0;

/// How many characters [`Input::text`] will buffer within a single frame
/// before dropping the rest. A frame's worth of real typing is a handful of
/// characters; anything beyond this is a stuck key or a paste-like IME
/// burst, and an unbounded buffer would just be a slow leak.
const MAX_TEXT_PER_FRAME: usize = 256;

/// Whether the current modifiers should suppress `KeyEvent::text` from being
/// appended as typed text. `Ctrl+F`, `Alt+F`, `Super+F` are shortcuts, not
/// characters — but Ctrl+Alt held *together* is not a shortcut chord, it's
/// how X11 and Windows report AltGr, which non-US layouts (French, German,
/// Spanish, Swiss, …) use to type `@ \ ~ { } [ ] €` and friends. So exactly
/// one of Ctrl/Alt alone suppresses; both together does not. Super always
/// suppresses (no legitimate typed-text case involves it). Shift alone still
/// types (it just selects the shifted character), so it's left out.
fn suppresses_text(modifiers: ModifiersState) -> bool {
    modifiers.super_key() || (modifiers.control_key() ^ modifiers.alt_key())
}

/// Aggregated input state for the current frame. See the module doc for the
/// event-forwarding/`end_frame` contract.
#[derive(Debug)]
pub struct Input {
    /// Physical-to-logical divisor, kept in sync from
    /// `WindowEvent::ScaleFactorChanged` so mouse positions come out in the
    /// same logical pixels the renderer's `draw_*` API takes.
    scale_factor: f64,

    keys_down: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    keys_typed: HashSet<KeyCode>,
    keys_released: HashSet<KeyCode>,
    modifiers: ModifiersState,

    buttons_down: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    buttons_released: HashSet<MouseButton>,

    mouse_position: (f32, f32),
    /// `None` until the first `CursorMoved`, so the first movement doesn't
    /// report a delta measured from a fictional origin.
    last_mouse_position: Option<(f32, f32)>,
    mouse_delta: (f32, f32),
    raw_mouse_delta: (f32, f32),
    scroll_delta: (f32, f32),

    text: String,

    focused: bool,
    cursor_in_window: bool,

    /// One state snapshot per input event, in OS delivery order. The top-level
    /// value remains the aggregate frame state used by components; shell
    /// commands and edits consume these snapshots so distinct events cannot
    /// be reordered or collapsed by the frame boundary.
    events: Vec<Input>,
}

// The whole query surface is the point of this type; `main.rs` only drives
// the event/frame plumbing, so most queries have no caller until editor code
// lands. Same situation (and same treatment) as the renderer's draw API.
#[allow(dead_code)]
impl Input {
    /// `scale_factor` is the window's current DPI scale (`window.scale_factor()`);
    /// it is kept current from `ScaleFactorChanged` afterwards.
    pub fn new(scale_factor: f64) -> Self {
        Self {
            scale_factor,
            keys_down: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_typed: HashSet::new(),
            keys_released: HashSet::new(),
            modifiers: ModifiersState::empty(),
            buttons_down: HashSet::new(),
            buttons_pressed: HashSet::new(),
            buttons_released: HashSet::new(),
            mouse_position: (0.0, 0.0),
            last_mouse_position: None,
            mouse_delta: (0.0, 0.0),
            raw_mouse_delta: (0.0, 0.0),
            scroll_delta: (0.0, 0.0),
            text: String::new(),
            focused: true,
            cursor_in_window: false,
            events: Vec::new(),
        }
    }

    /// Event-local views in the exact order winit delivered them. Each view
    /// carries held state as it was at that event, but only that event's
    /// edges, text, pointer delta, and scroll delta.
    pub fn events(&self) -> impl Iterator<Item = &Input> {
        self.events.iter()
    }

    fn snapshot(&self) -> Input {
        Input {
            scale_factor: self.scale_factor,
            keys_down: self.keys_down.clone(),
            keys_pressed: HashSet::new(),
            keys_typed: HashSet::new(),
            keys_released: HashSet::new(),
            modifiers: self.modifiers,
            buttons_down: self.buttons_down.clone(),
            buttons_pressed: HashSet::new(),
            buttons_released: HashSet::new(),
            mouse_position: self.mouse_position,
            last_mouse_position: self.last_mouse_position,
            mouse_delta: (0.0, 0.0),
            raw_mouse_delta: (0.0, 0.0),
            scroll_delta: (0.0, 0.0),
            text: String::new(),
            focused: self.focused,
            cursor_in_window: self.cursor_in_window,
            events: Vec::new(),
        }
    }

    fn handle_key(
        &mut self,
        code: Option<KeyCode>,
        state: ElementState,
        repeat: bool,
        text: Option<&str>,
    ) {
        let mut event_text = String::new();
        if let Some(text) = text
            && state.is_pressed()
            && !suppresses_text(self.modifiers)
            && self.text.len() + text.len() <= MAX_TEXT_PER_FRAME
        {
            event_text.extend(text.chars().filter(|c| !c.is_control()));
            self.text.push_str(&event_text);
        }

        if let Some(code) = code {
            match state {
                ElementState::Pressed => {
                    self.keys_typed.insert(code);
                    if !repeat {
                        self.keys_pressed.insert(code);
                    }
                    self.keys_down.insert(code);
                }
                ElementState::Released => {
                    self.keys_down.remove(&code);
                    self.keys_released.insert(code);
                }
            }
        }

        let mut snapshot = self.snapshot();
        snapshot.text = event_text;
        if let Some(code) = code {
            match state {
                ElementState::Pressed => {
                    snapshot.keys_typed.insert(code);
                    if !repeat {
                        snapshot.keys_pressed.insert(code);
                    }
                }
                ElementState::Released => {
                    snapshot.keys_released.insert(code);
                }
            }
        }
        self.events.push(snapshot);
    }

    // ---- event intake ----------------------------------------------------

    /// Absorb one `WindowEvent`. Safe to call with every event — anything
    /// input-irrelevant is ignored.
    ///
    /// Returns whether the event actually changed input state, which is the
    /// signal a frame scheduler needs: `Moved`, `Occluded`, `ThemeChanged`
    /// and friends reach this method but must not wake the renderer.
    pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                // Layout-independent identity: `KeyCode::KeyW` is the
                // physical W position regardless of the active layout, which
                // is what keybinds should be expressed against.
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    // No usable physical identity, but a keyboard event is
                    // user intent regardless — it may still have produced
                    // text above.
                    PhysicalKey::Unidentified(_) => None,
                };
                self.handle_key(code, event.state, event.repeat, event.text.as_deref());
                true
            }

            WindowEvent::Ime(ime) => {
                let mut event_text = String::new();
                if let Ime::Commit(text) = ime
                    && self.text.len() + text.len() <= MAX_TEXT_PER_FRAME
                {
                    event_text.extend(text.chars().filter(|c| !c.is_control()));
                    self.text.push_str(&event_text);
                }
                let mut snapshot = self.snapshot();
                snapshot.text = event_text;
                self.events.push(snapshot);
                true
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                let snapshot = self.snapshot();
                self.events.push(snapshot);
                true
            }

            WindowEvent::MouseInput { state, button, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.buttons_down.insert(*button);
                        self.buttons_pressed.insert(*button);
                    }
                    ElementState::Released => {
                        self.buttons_down.remove(button);
                        self.buttons_released.insert(*button);
                    }
                }
                let mut snapshot = self.snapshot();
                match state {
                    ElementState::Pressed => {
                        snapshot.buttons_pressed.insert(*button);
                    }
                    ElementState::Released => {
                        snapshot.buttons_released.insert(*button);
                    }
                }
                self.events.push(snapshot);
                true
            }

            WindowEvent::CursorMoved { position, .. } => {
                let next = (
                    (position.x / self.scale_factor) as f32,
                    (position.y / self.scale_factor) as f32,
                );
                let delta = self
                    .last_mouse_position
                    .map_or((0.0, 0.0), |last| (next.0 - last.0, next.1 - last.1));
                self.mouse_delta.0 += delta.0;
                self.mouse_delta.1 += delta.1;
                self.mouse_position = next;
                self.last_mouse_position = Some(next);
                self.cursor_in_window = true;
                let mut snapshot = self.snapshot();
                snapshot.mouse_delta = delta;
                self.events.push(snapshot);
                true
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        self.scroll_delta.0 += x;
                        self.scroll_delta.1 += y;
                        (*x, *y)
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        let lines = (
                            position.x as f32 / PIXELS_PER_LINE,
                            position.y as f32 / PIXELS_PER_LINE,
                        );
                        self.scroll_delta.0 += lines.0;
                        self.scroll_delta.1 += lines.1;
                        lines
                    }
                };
                let mut snapshot = self.snapshot();
                snapshot.scroll_delta = lines;
                self.events.push(snapshot);
                true
            }

            WindowEvent::CursorEntered { .. } => {
                self.cursor_in_window = true;
                let snapshot = self.snapshot();
                self.events.push(snapshot);
                true
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_in_window = false;
                // The next `CursorMoved` after re-entry is a teleport, not a
                // drag — forget the old anchor so it doesn't report one huge
                // delta across the gap.
                self.last_mouse_position = None;
                let snapshot = self.snapshot();
                self.events.push(snapshot);
                true
            }

            WindowEvent::Focused(focused) => {
                let released_keys = self.keys_down.clone();
                let released_buttons = self.buttons_down.clone();
                self.focused = *focused;
                if !focused {
                    self.release_all();
                }
                let mut snapshot = self.snapshot();
                if !focused {
                    snapshot.keys_released = released_keys;
                    snapshot.buttons_released = released_buttons;
                }
                self.events.push(snapshot);
                true
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = *scale_factor;
                // Stale logical coordinates measured against the old factor
                // would produce a phantom delta at the next move.
                self.last_mouse_position = None;
                let snapshot = self.snapshot();
                self.events.push(snapshot);
                true
            }

            _ => false,
        }
    }

    /// Absorb one `DeviceEvent`, for raw (unaccelerated, not clipped to the
    /// window) mouse motion — see [`Input::raw_mouse_delta`]. Optional:
    /// skip wiring this up entirely if only cursor-space motion is needed.
    pub fn handle_device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.raw_mouse_delta.0 += delta.0 as f32;
            self.raw_mouse_delta.1 += delta.1 as f32;
        }
    }

    /// Force-release everything currently held, reporting each as a release
    /// edge this frame so callers with press/release pairing (drag handlers,
    /// held-key repeat) unwind cleanly instead of waiting forever for a
    /// release event that will never arrive.
    ///
    /// Called automatically on focus loss; also useful manually when the
    /// app takes over input itself (opening a modal, entering a capture
    /// mode).
    pub fn release_all(&mut self) {
        self.keys_released.extend(self.keys_down.drain());
        self.buttons_released.extend(self.buttons_down.drain());
        self.modifiers = ModifiersState::empty();
        self.mouse_delta = (0.0, 0.0);
        self.raw_mouse_delta = (0.0, 0.0);
        self.scroll_delta = (0.0, 0.0);
    }

    /// End the current frame: clear everything that means "since last
    /// frame". Held state (`is_key_down`, `mouse_position`, `modifiers`)
    /// survives; edges and deltas do not. Call exactly once per frame,
    /// after the frame has read what it needs.
    pub fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_typed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.raw_mouse_delta = (0.0, 0.0);
        self.scroll_delta = (0.0, 0.0);
        self.text.clear();
        self.events.clear();
    }

    // ---- keyboard --------------------------------------------------------

    /// Is this key held right now? (Level-triggered — true every frame the
    /// key stays down.)
    pub fn is_key_down(&self, key: KeyCode) -> bool {
        self.keys_down.contains(&key)
    }

    /// Did this key go down *this frame*? (Edge-triggered, excludes OS
    /// auto-repeat — one `true` per physical press.)
    pub fn is_key_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    /// [`Input::is_key_pressed`], but *including* OS auto-repeat — the
    /// "should this fire again?" question for held-key actions like
    /// backspace or arrow-key cursor movement, where the OS's own repeat
    /// rate is the behaviour users expect.
    pub fn is_key_typed(&self, key: KeyCode) -> bool {
        self.keys_typed.contains(&key)
    }

    /// Did this key come up this frame? (Edge-triggered.)
    pub fn is_key_released(&self, key: KeyCode) -> bool {
        self.keys_released.contains(&key)
    }

    /// Every key currently held, in arbitrary order.
    pub fn keys_down(&self) -> impl Iterator<Item = KeyCode> + '_ {
        self.keys_down.iter().copied()
    }

    /// The current modifier state (shift/ctrl/alt/super), as a bitflag set.
    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }

    pub fn shift(&self) -> bool {
        self.modifiers.shift_key()
    }

    pub fn ctrl(&self) -> bool {
        self.modifiers.control_key()
    }

    pub fn alt(&self) -> bool {
        self.modifiers.alt_key()
    }

    /// The Super/Command/Windows key.
    pub fn super_key(&self) -> bool {
        self.modifiers.super_key()
    }

    /// Did `key` go down this frame with *exactly* `modifiers` held? The
    /// exactness matters: `Ctrl+S` must not also fire on `Ctrl+Shift+S`,
    /// which is usually a different command.
    ///
    /// ```ignore
    /// if input.is_shortcut_pressed(ModifiersState::CONTROL, KeyCode::KeyS) { save(); }
    /// ```
    pub fn is_shortcut_pressed(&self, modifiers: ModifiersState, key: KeyCode) -> bool {
        self.modifiers == modifiers && self.is_key_pressed(key)
    }

    /// [`Input::is_shortcut_pressed`]'s level-triggered counterpart — is
    /// this exact chord held right now?
    pub fn is_shortcut_down(&self, modifiers: ModifiersState, key: KeyCode) -> bool {
        self.modifiers == modifiers && self.is_key_down(key)
    }

    /// Characters typed this frame, control characters stripped — what to
    /// insert into a document. Empty on frames with no typing.
    ///
    /// This is deliberately separate from the [`KeyCode`] queries: `KeyCode`
    /// is a physical position (right for keybinds), this is layout- and
    /// modifier-resolved text (right for typing). A French layout's
    /// `KeyCode::KeyQ` types "a"; only one of those two answers is ever the
    /// one wanted.
    pub fn text(&self) -> &str {
        &self.text
    }

    // ---- mouse -----------------------------------------------------------

    pub fn is_mouse_down(&self, button: MouseButton) -> bool {
        self.buttons_down.contains(&button)
    }

    pub fn is_mouse_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }

    pub fn is_mouse_released(&self, button: MouseButton) -> bool {
        self.buttons_released.contains(&button)
    }

    /// Every mouse button currently held, in arbitrary order.
    pub fn buttons_down(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.buttons_down.iter().copied()
    }

    /// Cursor position in **logical** pixels, in the same coordinate space
    /// the renderer's `draw_*` methods take, so a hit-test is a direct
    /// comparison against drawn geometry with no conversion.
    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_position
    }

    /// How far the cursor moved this frame, in logical pixels. Clipped to
    /// the window and subject to OS pointer acceleration — the right
    /// measure for dragging UI. Zero on frames with no movement.
    pub fn mouse_delta(&self) -> (f32, f32) {
        self.mouse_delta
    }

    /// Raw device motion this frame, unaccelerated and unclipped, from
    /// `DeviceEvent::MouseMotion`. Non-zero only if
    /// [`Input::handle_device_event`] is wired up. Units are device-defined,
    /// not pixels.
    pub fn raw_mouse_delta(&self) -> (f32, f32) {
        self.raw_mouse_delta
    }

    /// Scroll accumulated this frame in lines — positive `y` is scroll-up /
    /// content-down, positive `x` is scroll-right, matching winit. Pixel
    /// deltas from touchpads are converted at [`PIXELS_PER_LINE`].
    pub fn scroll_delta(&self) -> (f32, f32) {
        self.scroll_delta
    }

    // ---- window ----------------------------------------------------------

    /// Does the window have keyboard focus? Held input is force-released
    /// when this goes false, so a chord can't survive an alt-tab.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Is the cursor over the window? [`Input::mouse_position`] keeps its
    /// last known value while this is false, so check it before treating a
    /// position as a live hover.
    pub fn is_cursor_in_window(&self) -> bool {
        self.cursor_in_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::{ExtendedAction, Key, Mode, Vim};
    use winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, TouchPhase},
    };

    /// `KeyEvent` can't be built outside winit (its `platform_specific`
    /// field is crate-private), so keyboard cases seed `keys_down` directly
    /// — the state these tests care about is what `release_all`/`end_frame`
    /// do to it, not the event decoding on the way in.
    fn input() -> Input {
        Input::new(1.0)
    }

    fn cursor(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        }
    }

    fn click(state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        }
    }

    #[test]
    fn button_edges_last_exactly_one_frame() {
        let mut input = input();
        input.handle_event(&click(ElementState::Pressed));
        assert!(input.is_mouse_down(MouseButton::Left));
        assert!(input.is_mouse_pressed(MouseButton::Left));

        input.end_frame();
        assert!(
            input.is_mouse_down(MouseButton::Left),
            "held state must survive the frame"
        );
        assert!(
            !input.is_mouse_pressed(MouseButton::Left),
            "press edge must not repeat"
        );

        input.handle_event(&click(ElementState::Released));
        assert!(!input.is_mouse_down(MouseButton::Left));
        assert!(input.is_mouse_released(MouseButton::Left));
    }

    #[test]
    fn focus_loss_releases_everything_held() {
        let mut input = input();
        input.keys_down.insert(KeyCode::ControlLeft);
        input.keys_down.insert(KeyCode::KeyS);
        input.modifiers = ModifiersState::CONTROL;
        input.handle_event(&click(ElementState::Pressed));
        input.end_frame();

        input.handle_event(&WindowEvent::Focused(false));

        assert!(!input.is_focused());
        assert_eq!(
            input.keys_down().count(),
            0,
            "keys must not stay stuck across focus loss"
        );
        assert!(!input.is_mouse_down(MouseButton::Left));
        assert!(input.modifiers().is_empty());
        // Reported as edges so press/release pairing unwinds instead of hanging.
        assert!(input.is_key_released(KeyCode::KeyS));
        assert!(input.is_mouse_released(MouseButton::Left));
    }

    #[test]
    fn altgr_ctrl_alt_together_does_not_suppress_text() {
        assert!(!suppresses_text(
            ModifiersState::CONTROL.union(ModifiersState::ALT)
        ));
    }

    #[test]
    fn ctrl_or_alt_alone_suppresses_text() {
        assert!(suppresses_text(ModifiersState::CONTROL));
        assert!(suppresses_text(ModifiersState::ALT));
    }

    #[test]
    fn super_suppresses_text() {
        assert!(suppresses_text(ModifiersState::SUPER));
    }

    #[test]
    fn mouse_position_is_logical_and_delta_is_per_frame() {
        let mut input = Input::new(2.0);
        input.handle_event(&cursor(200.0, 100.0));
        assert_eq!(
            input.mouse_position(),
            (100.0, 50.0),
            "physical px must be scaled down"
        );
        assert_eq!(
            input.mouse_delta(),
            (0.0, 0.0),
            "first move has no previous anchor"
        );

        input.end_frame();
        input.handle_event(&cursor(220.0, 100.0));
        assert_eq!(input.mouse_delta(), (10.0, 0.0));

        input.end_frame();
        assert_eq!(input.mouse_delta(), (0.0, 0.0));
    }

    #[test]
    fn leaving_the_window_drops_the_delta_anchor() {
        let mut input = input();
        input.handle_event(&cursor(10.0, 10.0));
        input.end_frame();

        input.handle_event(&WindowEvent::CursorLeft {
            device_id: DeviceId::dummy(),
        });
        input.handle_event(&cursor(900.0, 600.0));

        assert!(input.is_cursor_in_window());
        assert_eq!(
            input.mouse_delta(),
            (0.0, 0.0),
            "re-entry is a teleport, not a drag"
        );
    }

    #[test]
    fn scroll_accumulates_both_units_into_lines() {
        let mut input = input();
        let wheel = |delta| WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta,
            phase: TouchPhase::Moved,
        };
        input.handle_event(&wheel(MouseScrollDelta::LineDelta(0.0, 2.0)));
        input.handle_event(&wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0,
            f64::from(PIXELS_PER_LINE),
        ))));

        assert_eq!(input.scroll_delta(), (0.0, 3.0));
        input.end_frame();
        assert_eq!(input.scroll_delta(), (0.0, 0.0));
    }

    #[test]
    fn an_ime_commit_becomes_typed_text() {
        let mut input = input();
        input.handle_event(&WindowEvent::Ime(Ime::Commit("ê".into())));
        assert_eq!(input.text(), "ê");
        input.end_frame();
        assert_eq!(input.text(), "", "a commit is per-frame like any typing");
    }

    #[test]
    fn an_ime_preedit_inserts_nothing() {
        let mut input = input();
        input.handle_event(&WindowEvent::Ime(Ime::Preedit("^".into(), Some((0, 1)))));
        assert_eq!(input.text(), "", "a preedit is in progress, not committed");
    }

    #[test]
    fn event_views_preserve_key_order_and_repetition() {
        let mut input = input();
        input.handle_key(Some(KeyCode::KeyA), ElementState::Pressed, false, Some("a"));
        input.handle_key(Some(KeyCode::Backspace), ElementState::Pressed, false, None);
        input.handle_key(Some(KeyCode::Backspace), ElementState::Pressed, true, None);
        input.handle_key(Some(KeyCode::KeyB), ElementState::Pressed, false, Some("b"));

        let events: Vec<&Input> = input.events().collect();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].text(), "a");
        assert!(events[1].is_key_typed(KeyCode::Backspace));
        assert!(events[1].is_key_pressed(KeyCode::Backspace));
        assert!(events[2].is_key_typed(KeyCode::Backspace));
        assert!(!events[2].is_key_pressed(KeyCode::Backspace));
        assert_eq!(events[3].text(), "b");

        // Aggregate frame queries intentionally remain available for widgets
        // that only care whether something happened during this frame.
        assert_eq!(input.text(), "ab");
        assert!(input.is_key_typed(KeyCode::Backspace));
    }

    #[test]
    fn event_views_capture_modifiers_at_the_key_event() {
        let mut input = input();
        input.modifiers = ModifiersState::CONTROL;
        input.handle_key(Some(KeyCode::KeyS), ElementState::Pressed, false, None);
        input.modifiers = ModifiersState::empty();

        let event = input.events().next().unwrap();
        assert!(event.is_shortcut_pressed(ModifiersState::CONTROL, KeyCode::KeyS));
        assert!(!input.is_shortcut_pressed(ModifiersState::CONTROL, KeyCode::KeyS));
    }

    #[test]
    fn event_views_expire_with_the_frame() {
        let mut input = input();
        input.handle_event(&WindowEvent::Ime(Ime::Commit("a".into())));
        assert_eq!(input.events().count(), 1);
        input.end_frame();
        assert_eq!(input.events().count(), 0);
    }

    #[test]
    fn ordered_views_apply_a_mode_change_before_the_following_character() {
        let mut input = input();
        input.handle_key(Some(KeyCode::KeyI), ElementState::Pressed, false, Some("i"));
        input.handle_key(Some(KeyCode::KeyA), ElementState::Pressed, false, Some("a"));
        let mut vim = Vim::new();
        let mut inserted = String::new();

        for event in input.events() {
            for character in event.text().chars() {
                match vim.key_extended(Key::Char(character)) {
                    ExtendedAction::Enter(mode) => vim.set_mode(mode),
                    ExtendedAction::InsertAt(_) => vim.set_mode(Mode::Insert),
                    ExtendedAction::Passthrough => inserted.push(character),
                    _ => {}
                }
            }
        }

        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(inserted, "a");
    }
}
