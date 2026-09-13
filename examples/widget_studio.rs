//! Native widget review in an isolated, disposable vault. Uses the real shell,
//! editor, input and GPU painter. No saved user configuration is read or changed.
//! `cargo run --example widget_studio` (optionally `-- --dark`). Open Widgets.md.
//! `--export-preview` renders both themes to target/widget-preview-*.pdf and exits.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use typewritter::{
    components::editor::{Editor, Metrics},
    config::Config,
    input::Input,
    layout::Rect,
    renderer::Renderer,
    shell::Shell,
    theme::{self, Theme},
    ui::Component,
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[derive(Default)]
struct Studio {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    shell: Option<Shell>,
    input: Option<Input>,
    wake: Option<Instant>,
}

impl ApplicationHandler for Studio {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        let vault =
            std::env::temp_dir().join(format!("typewritter-widget-studio-{}", std::process::id()));
        std::fs::create_dir_all(&vault).expect("create preview vault");
        std::fs::write(
            vault.join("Widgets.md"),
            include_str!("fixtures/widgets.md"),
        )
        .expect("write preview note");
        let export_preview = std::env::args().any(|arg| arg == "--export-preview");
        let window = Arc::new(
            events
                .create_window(
                    Window::default_attributes()
                        .with_visible(!export_preview)
                        .with_title("Typewritter · Widget studio")
                        .with_inner_size(LogicalSize::new(1280, 900)),
                )
                .expect("preview window"),
        );
        let mut renderer = pollster::block_on(Renderer::new(window.clone()));
        if export_preview {
            let layer = renderer.new_layer_top(typewritter::renderer::LayerInvalidation::Manual);
            let document = typewritter::document::markdown::parse(
                &vault.join("Widgets.md"),
                include_str!("fixtures/widgets.md"),
            );
            for (name, theme) in [("light", Theme::LIGHT), ("dark", Theme::DARK)] {
                theme::set(theme.clone());
                let layout =
                    typewritter::document::layout::layout(&document, 704.0, &|text, style| {
                        theme::width(&layer, text, style)
                    });
                let caret = layout
                    .source
                    .iter()
                    .position(|block| block.is_widget())
                    .map(|block| typewritter::document::Caret {
                        block,
                        inline: 0,
                        offset: 0,
                        style: typewritter::document::Style::PLAIN,
                    });
                let mut editor = Editor::new(
                    std::rc::Rc::new(layout),
                    caret,
                    0.0,
                    true,
                    typewritter::document::Style::PLAIN,
                    Metrics::PAGE,
                );
                layer.clear();
                let rect = Rect::new(0.0, 0.0, 1280.0, 900.0);
                layer.set_clip_rect(Some((rect.position(), rect.size())));
                editor.draw(&layer, rect);
                renderer.render().expect("native widget scene");
                typewritter::export::export_pdf(
                    &document,
                    &layer,
                    &std::path::PathBuf::from(format!("target/widget-preview-{name}.pdf")),
                    typewritter::export::Options {
                        theme,
                        ..Default::default()
                    },
                )
                .expect("preview export");
            }
            events.exit();
            return;
        }
        self.shell = Some(Shell::new(
            &mut renderer,
            Some(Config {
                vault,
                ..Config::default()
            }),
        ));
        self.input = Some(Input::new(window.scale_factor()));
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let (Some(window), Some(renderer), Some(shell), Some(input)) = (
            &self.window,
            &mut self.renderer,
            &mut self.shell,
            &mut self.input,
        ) else {
            return;
        };
        if input.handle_event(&event) {
            window.request_redraw();
        }
        renderer.handle_event(&event);
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::RedrawRequested => {
                let size = window.inner_size().to_logical::<f32>(window.scale_factor());
                let animating = shell.update(
                    input,
                    Rect::new(0.0, 0.0, size.width, size.height),
                    renderer.get_frametime(),
                    renderer,
                );
                self.wake = if animating {
                    Some(Instant::now() + Duration::from_millis(16))
                } else {
                    shell.wake_at()
                };
                renderer.render().expect("native widget render");
                if renderer.has_pending_resize() {
                    window.request_redraw();
                }
                input.end_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if self.wake.is_some_and(|wake| wake <= Instant::now()) {
            self.wake = None;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        events.set_control_flow(self.wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    theme::set(if std::env::args().any(|arg| arg == "--dark") {
        Theme::DARK
    } else {
        Theme::LIGHT
    });
    EventLoop::new()?.run_app(&mut Studio::default())
}
