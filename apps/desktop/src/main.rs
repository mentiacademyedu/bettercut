//! bettercut — the desktop application shell (§4, §83.3).
//!
//! One process, one language, one wgpu device (§4). This binary does three
//! things and nothing else:
//!
//!   1. sets up logging (§49),
//!   2. owns the `Editor` and the `UiState`,
//!   3. pumps events and asks `bettercut-ui` to draw a frame.
//!
//! All editing logic lives below it (§86). Nothing here touches `Project`.

// Release builds are GUI apps: no console window should flash on launch.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bettercut_editor_core::{Editor, EventReceiver};
use bettercut_ui::UiState;

fn main() -> eframe::Result {
    init_logging();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting bettercut");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("bettercut"),
        ..Default::default()
    };

    // A project path may be given on the command line, which is also what a
    // file association or a drag-onto-the-exe sends.
    let requested = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    eframe::run_native(
        "bettercut",
        options,
        Box::new(move |cc| {
            bettercut_ui::theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(requested, cc)))
        }),
    )
}

/// §49: structured logs, `INFO` by default, overridable with `RUST_LOG`.
fn init_logging() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        .init();
}

struct App {
    editor: Editor,
    events: EventReceiver,
    ui: UiState,
    /// `None` when wgpu is unavailable. §21 keeps a CPU fallback only for
    /// machines where wgpu fails to initialize at all; until that exists, the
    /// editor still runs — it just cannot show pictures (§50).
    preview: Option<bettercut_ui::Preview>,
    /// Proxy generation (§13). Owned here because it spans the editor (which
    /// media exist), the cache (what is already generated) and the preview
    /// (which copy to read).
    proxies: bettercut_ui::ProxyManager,
    /// Title is recomputed only when it changes; setting it every frame would
    /// churn the window manager.
    last_title: String,
}

impl App {
    fn new(open: Option<std::path::PathBuf>, cc: &eframe::CreationContext<'_>) -> Self {
        let mut ui = UiState::default();

        // A project that fails to open must not stop the app from starting:
        // §50 says a failure is reported and the session continues.
        let (editor, events) = match open {
            Some(path) => match Editor::open(&path) {
                Ok(loaded) => {
                    tracing::info!(path = %path.display(), "opened project from command line");
                    loaded
                }
                Err(err) => {
                    tracing::error!(path = %path.display(), %err, "could not open project");
                    ui.error(format!("Could not open {}: {err}", path.display()));
                    Editor::new_project("Untitled")
                }
            },
            None => Editor::new_project("Untitled"),
        };

        // §44: the decoder thread cap and cache budget come from the detected
        // hardware, not from a constant.
        let hardware = editor.hardware();
        let preview =
            cc.wgpu_render_state.as_ref().and_then(
                |render_state| match bettercut_ui::Preview::new(
                    render_state,
                    hardware.frame_cache_bytes(),
                    hardware.ffmpeg_threads_per_job(),
                ) {
                    Ok(preview) => Some(preview),
                    Err(err) => {
                        tracing::error!(%err, "could not start the preview renderer");
                        ui.error(format!("Preview unavailable: {err}"));
                        None
                    }
                },
            );

        match preview.as_ref() {
            Some(preview) => ui.gpu = Some(preview.gpu().clone()),
            None => tracing::warn!("running without a preview renderer"),
        }

        // §39: look for work from a session that did not shut down cleanly.
        // Checked after the project is loaded so the prompt can say what it
        // would replace, and never applied without the user's say-so (§39.5).
        ui.pending_recovery = bettercut_editor_core::recover(
            bettercut_editor_core::RecoveryPaths::for_project(editor.path(), "previous"),
        )
        .or_else(|| bettercut_editor_core::scan_unsaved().into_iter().next());

        if ui.pending_recovery.is_some() {
            tracing::info!("found recoverable work from a previous session");
        }

        // §15/§67: both limits come from the detected hardware and the
        // project's performance mode, never from constants.
        let cache = bettercut_cache::CacheStore::new(
            bettercut_cache::CacheLayout::default_location(),
            editor.project().settings.cache_limit_bytes,
        );
        let mut proxies = bettercut_ui::ProxyManager::new(
            cache,
            editor.project().settings.performance_mode,
            hardware.max_heavy_jobs(),
            hardware.ffmpeg_threads_per_job(),
        );

        let mut preview = preview;
        if let Some(preview) = preview.as_mut() {
            preview.set_proxy_source(proxies.source());
        }
        // Anything already in the project may need a proxy (§13).
        for message in proxies.scan(&editor) {
            ui.info(message);
        }

        let last_title = editor.window_title();
        Self {
            editor,
            events,
            ui,
            preview,
            proxies,
            last_title,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // §56: drain the whole queue once per frame, never per event.
        bettercut_ui::consume_events(self.events.drain(), &self.editor, &mut self.ui);

        // Playback work happens before drawing, so the frame painted this pass
        // is the one the clock is asking for rather than the previous one.
        let playing = match self.preview.as_mut() {
            Some(preview) => preview.update(&mut self.editor),
            None => false,
        };

        // §13: pick up proxies that finished encoding, and queue any new
        // imports. Both are cheap when nothing has changed.
        let update = self.proxies.poll();
        for media in &update.ready {
            if let Some(preview) = self.preview.as_mut() {
                preview.proxy_ready(*media);
            }
        }
        for message in update.messages {
            self.ui.info(message);
        }
        for failure in update.failures {
            self.ui.error(failure);
        }
        if !update.ready.is_empty() {
            self.ui.needs_repaint = true;
        }

        // §42: keep the status bar honest about work still running. Repainting
        // while jobs are in flight is what makes the bar move at all.
        self.ui.proxy_progress = self
            .proxies
            .overall_progress()
            .map(|fraction| (self.proxies.active_jobs(), fraction));
        if self.ui.proxy_progress.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }

        bettercut_ui::draw(ui, &mut self.editor, &mut self.ui, self.preview.as_mut());

        // After drawing, because both the import button and the quality setting
        // live in panels this call would otherwise be a frame behind.
        let (new_source, messages) = self.proxies.sync(&self.editor);
        if let Some(source) = new_source
            && let Some(preview) = self.preview.as_mut()
        {
            preview.set_proxy_source(source);
        }
        for message in messages {
            self.ui.info(message);
        }

        // §20a.1: while playing, keep asking for frames. The clock, not this
        // loop, decides what is shown.
        if playing {
            ui.ctx().request_repaint();
        }

        let title = self.editor.window_title();
        if title != self.last_title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        // §54/§81: repaint only when something changed. Idle CPU near zero is a
        // performance target, not a nicety — the target hardware is thermally
        // limited. Playback will add a steady repaint request in Milestone 4.
        if std::mem::take(&mut self.ui.needs_repaint) {
            ui.ctx().request_repaint();
        }
    }

    /// §48: a half-encoded proxy is worthless, and waiting for one to finish
    /// would hold the window open. Cancel and leave the temp file for the next
    /// run to overwrite — the cache is disposable by construction (§67).
    fn on_exit(&mut self) {
        self.proxies.cancel_all();
    }
}
