//! Typewritter — entry point.
//!
//! Window and event loop only: everything on screen belongs to
//! [`Shell`](typewritter::shell::Shell), which owns the layout tree, the
//! components, and one Atomos layer per region.
//!
//! First launch (and `--onboard`) opens the welcome screen. Its button
//! raises the native folder picker and remembers the chosen vault.
//!
//! Frames are demand-driven — the loop sleeps until input changes state or
//! an animation asks for another one — and within a frame, only the
//! regions that actually changed redraw. The status line's counters are
//! the visible proof of both.
//!
//! Keys: vim modes drive the editor (Normal/Insert; `h j k l`, `w b e`,
//! `0 ^ $`, `gg`/`G`, `x`, `dd`, `o`/`O`, `i a I A`), and the space leader
//! opens the command palette every binding is routed through.
//! `Ctrl+Shift+C` toggles focus mode; the toolbar also exposes focus,
//! sidebar, search, and appearance. Drag panel dividers to resize them.

use std::sync::Arc;
use std::time::Instant;

use typewritter::config::Config;
use typewritter::document::math_style;
use typewritter::layout::Rect;
use typewritter::renderer::Renderer;
use typewritter::shell::Shell;
use typewritter::{frame, input};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    shell: Option<Shell>,
    input: input::Input,
    scheduler: frame::FrameScheduler,
    /// Last minimum pushed to the window, so it is only set on a change.
    window_minimum: (f32, f32),
    /// The vault config, chosen during onboarding in `main`.
    config: Option<Config>,
    /// When the shell next wants a frame for an animation that is currently
    /// holding still — see [`Shell::wake_at`].
    wake_at: Option<Instant>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            window: None,
            renderer: None,
            shell: None,
            // Replaced with the window's real scale factor in `resumed`.
            input: input::Input::new(1.0),
            scheduler: frame::FrameScheduler::new(),
            window_minimum: (0.0, 0.0),
            config: None,
            wake_at: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Typewritter")
                        .with_inner_size(LogicalSize::new(1280, 800)),
                )
                .expect("failed to create window"),
        );

        // Dead keys and system input methods (macOS's dead-key sequences, CJK
        // IMEs, the hold-key accent popover) deliver their text through
        // `WindowEvent::Ime`, and winit only sends those events when IME is
        // allowed — it is off by default. Without this, a composed character is
        // simply dropped.
        window.set_ime_allowed(true);

        let mut renderer = pollster::block_on(Renderer::new(window.clone()));

        // The shell opens the vault (or starts on boarding) from here.
        self.shell = Some(Shell::new(&mut renderer, self.config.clone()));
        self.input = input::Input::new(window.scale_factor());
        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    /// Raw pointer motion only arrives as a device event.
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        self.input.handle_device_event(&event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        // Only events that genuinely changed input state wake the renderer.
        if self.input.handle_event(&event) {
            self.scheduler.request_redraw();
        }
        if let Some(r) = &mut self.renderer {
            r.handle_event(&event);
        }

        match event {
            WindowEvent::CloseRequested => {
                // Save every dirty tab before exiting — unconditionally, with
                // no prompt and no "discard" path. The reader's notes are the
                // reader's, this is a local vault with git sync planned, and a
                // dialog between a student and their closing laptop is a way
                // to lose work, not a way to protect it.
                let saved = self.shell.as_mut().is_none_or(|shell| shell.save_all());
                if saved {
                    event_loop.exit();
                } else {
                    self.scheduler.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    // The one place physical pixels are correct.
                    r.resize(size.width, size.height);
                }
                // Neither resize nor rescale flows through `Input`.
                self.scheduler.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(r) = &mut self.renderer {
                    r.set_scale_factor(scale_factor);
                }
                self.scheduler.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                // At the *top* of the frame, so anything this frame asks
                // for survives to the next `about_to_wait`.
                self.scheduler.begin_frame();
                self.frame();
                // Exactly once per frame, after everything has read state.
                self.input.end_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The scheduler knows "a frame is owed now" and "nothing is owed";
        // a blink is neither. It wants one frame at a known future instant
        // and none before it, so the deadline is handled here rather than
        // taught to the scheduler.
        //
        // Once it passes the frame is owed like any other, and the wake is
        // cleared: the frame sets the next one. Leaving it set would make
        // the `WaitUntil` below a deadline already in the past, which is a
        // spin, not a sleep.
        if self.wake_at.is_some_and(|at| Instant::now() >= at) {
            self.wake_at = None;
            self.scheduler.request_redraw();
        }
        if self.scheduler.should_draw_now()
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        // `Wait` is the only case the deadline may replace — anything owed
        // now must not be delayed behind an animation.
        let flow = match (self.scheduler.control_flow(), self.wake_at) {
            (ControlFlow::Wait, Some(at)) => ControlFlow::WaitUntil(at),
            (flow, _) => flow,
        };
        event_loop.set_control_flow(flow);
    }
}

impl App {
    fn frame(&mut self) {
        let (Some(window), Some(renderer), Some(shell)) =
            (&self.window, &mut self.renderer, &mut self.shell)
        else {
            return;
        };

        let size = window.inner_size().to_logical::<f32>(window.scale_factor());
        let viewport = Rect::new(0.0, 0.0, size.width, size.height);

        // A compositor can report a zero-sized (or absurdly small) window mid
        // resize. Solving the layout against it collapses every region's rect,
        // which clears every layer and leaves the clear colour on screen — the
        // black flash. There is nothing to draw at this size, so skip the frame
        // entirely and keep the last good one on screen.
        if size.width < 1.0 || size.height < 1.0 {
            return;
        }

        let frametime = renderer.get_frametime();
        if shell.update(&self.input, viewport, frametime, renderer) {
            self.scheduler.request_redraw();
        }
        self.wake_at = shell.wake_at();

        // The shell's minimum is what keeps regions from being squeezed
        // into each other: below it, the layout would have to overflow.
        let minimum = shell.min_window_size();
        if minimum != self.window_minimum {
            self.window_minimum = minimum;
            window.set_min_inner_size(Some(LogicalSize::new(minimum.0, minimum.1)));
        }

        if let Err(e) = renderer.render() {
            eprintln!("render error: {e}");
        }
    }
}

/// Trims the Vulkan loader's driver scan before wgpu creates its instance.
///
/// The loader opens *every* ICD manifest on the machine at instance
/// creation, not just the one it ends up using — on a stock Linux desktop
/// that is lavapipe, virtio, gfxstream and a couple of legacy hardware
/// drivers alongside the real one. Lavapipe alone maps libLLVM. Measured on
/// this machine: 8 MB of resident memory for drivers this app can never
/// pick.
///
/// Only the ones that are never the right answer for a windowed editor on a
/// real GPU are disabled — the software rasterizer, the two VM guest
/// drivers, a pre-Broadwell Intel driver superseded by `intel_icd`, and an
/// Apple Silicon driver that cannot appear on the same box as any of them.
/// An explicit choice in the environment always wins.
fn trim_vulkan_drivers() {
    const SELECTORS: [&str; 2] = ["VK_LOADER_DRIVERS_SELECT", "VK_LOADER_DRIVERS_DISABLE"];
    if SELECTORS.iter().any(|k| std::env::var_os(k).is_some()) {
        return;
    }
    // SAFETY: `main`, before the event loop and before wgpu, so this process
    // is still single-threaded and no other thread can be reading the
    // environment concurrently.
    unsafe {
        std::env::set_var(
            "VK_LOADER_DRIVERS_DISABLE",
            "*lvp*,*virtio*,*gfxstream*,*hasvk*,*asahi*",
        );
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    trim_vulkan_drivers();

    // `--onboard` discards the saved config; first run has none anyway.
    // Either way the shell starts on the onboarding splash and raises the
    // folder dialog from there — the dialog never comes out of thin air.
    let onboard = std::env::args().any(|arg| arg == "--onboard");
    let config = Config::load().filter(|_| !onboard);

    // Before the first frame: symbol styling is read from every math draw
    // call, and installing it later would mean one frame drawn in colours
    // the reader replaced.
    if let Some(config) = &config {
        math_style::install(config.math.clone());
    }

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App {
        config,
        ..App::default()
    };
    event_loop.run_app(&mut app)
}
