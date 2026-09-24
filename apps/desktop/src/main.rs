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
    // A panic leaves a report for the next launch to show (`bettercut_ui::crash`).
    bettercut_ui::crash::install_hook(bettercut_ui::crash::crash_dir());

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting bettercut");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0])
            // Files dragged in from the desktop are imported (see
            // `bettercut_ui::file_drop`). Said explicitly rather than left to
            // the default, because it is a feature people rely on.
            .with_drag_and_drop(true)
            .with_title("bettercut")
            // The window's and the taskbar's icon, drawn at start.
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: bettercut_ui::icon::rgba(64),
                width: 64,
                height: 64,
            })),
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
///
/// To the console and to a file: a release build has no console, so without
/// the file a tester's logs went nowhere. The run before is kept beside it.
fn init_logging() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));

    let path = bettercut_ui::crash::log_file();
    let file = path.parent().and_then(|dir| {
        std::fs::create_dir_all(dir).ok()?;
        let _ = std::fs::rename(&path, path.with_extension("previous.log"));
        std::fs::File::create(&path).ok()
    });

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        // Plain text: colour codes are noise in a file someone attaches.
        .with_ansi(false);
    match file {
        Some(file) => builder
            .with_writer(std::io::stderr.and(std::sync::Mutex::new(file)))
            .init(),
        // No file to write to: the console alone, as before.
        None => builder.init(),
    }
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
    proxies: bettercut_ui::MediaJobs,
    /// The performance mode the running session was configured with, so a
    /// change can be detected and applied (§43).
    applied_mode: bettercut_editor_core::project_format::PerformanceMode,
    /// Title is recomputed only when it changes; setting it every frame would
    /// churn the window manager.
    last_title: String,
    /// The microphone, while a voiceover is being recorded.
    voiceover: bettercut_ui::voiceover::VoiceoverRecorder,
    /// Where the playhead was when the last scrub burst was played, so one
    /// burst is played per move rather than per frame.
    last_scrub: Option<bettercut_editor_core::foundation::TimelineTime>,
}

impl App {
    fn new(open: Option<std::path::PathBuf>, cc: &eframe::CreationContext<'_>) -> Self {
        let mut ui = UiState::default();
        // The one place the real list is read and written: everything else,
        // tests included, keeps an in-memory one.
        ui.recent = bettercut_ui::recent::RecentProjects::stored_in(
            bettercut_ui::recent::RecentProjects::default_file(),
        );
        ui.keymap =
            bettercut_ui::keymap::Keymap::stored_in(bettercut_ui::keymap::Keymap::default_file());
        ui.title_styles = bettercut_ui::title_styles::UserTitleStyles::stored_in(
            bettercut_ui::title_styles::UserTitleStyles::default_file(),
        );
        // What the interface remembers about itself: the theme, before the
        // first frame is drawn in the wrong one.
        ui.prefs = bettercut_ui::prefs::UserPrefs::stored_in(
            bettercut_ui::prefs::UserPrefs::default_file(),
        );
        // A first run says hello; after "Don't show this again" it does not.
        ui.welcome_open = !ui.prefs.seen_welcome;
        // A crash last time: say so, with the report to send.
        ui.crash_report = bettercut_ui::crash::pending(&bettercut_ui::crash::crash_dir());
        // After an update, what is new — once; then this build is remembered.
        ui.whats_new_open =
            bettercut_ui::whats_new::should_show(&ui.prefs.last_version, ui.prefs.seen_welcome);
        if ui.prefs.last_version != bettercut_ui::whats_new::VERSION {
            ui.prefs.last_version = bettercut_ui::whats_new::VERSION.to_owned();
            let _ = ui.prefs.save();
        }
        bettercut_ui::theme::set_light(ui.prefs.light_theme);
        bettercut_ui::theme::apply(&cc.egui_ctx);
        cc.egui_ctx
            .set_zoom_factor(bettercut_ui::prefs::sane_scale(ui.prefs.interface_scale));
        let autosave_seconds = ui.prefs.autosave_seconds;
        ui.export_presets = bettercut_ui::export_presets::UserExports::stored_in(
            bettercut_ui::export_presets::UserExports::default_file(),
        );
        ui.user_looks = bettercut_ui::looks::UserLooks::stored_in(
            bettercut_ui::looks::UserLooks::default_file(),
        );

        // A project that fails to open must not stop the app from starting:
        // §50 says a failure is reported and the session continues.
        let (editor, events) = match open {
            Some(path) => match Editor::open(&path) {
                Ok(loaded) => {
                    tracing::info!(path = %path.display(), "opened project from command line");
                    ui.recent.touch(&path);
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

        // How often the recovery journal snapshots: the machine's setting.
        let mut editor = editor;
        editor.set_autosave_seconds(autosave_seconds);

        // §44: the decoder thread cap and cache budget come from the detected
        // hardware, not from a constant.
        let hardware = editor.hardware();
        // §43: the *project's* mode drives these, not the machine's suggestion.
        // A loaded project carries the user's choice, and honouring the
        // recommendation instead would quietly override it.
        let mode = editor.project().settings.performance_mode;
        let preview =
            cc.wgpu_render_state.as_ref().and_then(
                |render_state| match bettercut_ui::Preview::new(
                    render_state,
                    hardware.frame_cache_bytes(mode),
                    hardware.ffmpeg_threads_per_job(mode),
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
            Some(preview) => {
                ui.gpu = Some(preview.gpu().clone());
                // §26's font list. Read once, here, because enumerating the
                // machine's font directories is not something to do while
                // drawing a frame — and the rasterizer has already done it.
                ui.font_families = preview.font_families();
                tracing::info!(families = ui.font_families.len(), "fonts available");
            }
            None => tracing::warn!("running without a preview renderer"),
        }

        // §39: look for work from a session that did not shut down cleanly.
        // Checked after the project is loaded so the prompt can say what it
        // would replace, and never applied without the user's say-so (§39.5).
        ui.pending_recovery = bettercut_editor_core::recover(
            bettercut_editor_core::RecoveryPaths::for_project(editor.path(), "previous"),
        )
        .or_else(|| bettercut_editor_core::scan_unsaved().into_iter().next());

        // Abandoned sessions accumulate — one per editor, and only a clean
        // shutdown removes its own — so a machine that has crashed a few times,
        // or has run the test suite, can have thousands waiting.
        //
        // On a thread, because this is housekeeping and housekeeping has no
        // business holding the window shut: deleting them took nine seconds
        // here, ahead of anything being drawn.
        //
        // *After* the scan, and that order is the whole safety argument. The
        // scan is synchronous and has already read what it found into memory
        // by the time this starts, so a session it offers survives its own
        // files being pruned underneath it — which can happen, because a
        // session old enough to prune is still one the scan will offer. The
        // thread is detached and may be killed at exit; pruning is idempotent
        // and picks up where it left off next launch.
        std::thread::spawn(bettercut_editor_core::prune_unsaved);

        if ui.pending_recovery.is_some() {
            tracing::info!("found recoverable work from a previous session");
        }

        // §15/§67: both limits come from the detected hardware and the
        // project's performance mode, never from constants.
        let cache = bettercut_cache::CacheStore::new(
            bettercut_cache::CacheLayout::default_location(),
            editor.project().settings.cache_limit_bytes,
        );
        let mut proxies = bettercut_ui::MediaJobs::new(
            cache,
            mode,
            hardware.max_heavy_jobs(mode),
            hardware.ffmpeg_threads_per_job(mode),
        );

        let mut preview = preview;
        if let Some(preview) = preview.as_mut() {
            preview.set_proxy_source(proxies.source());
        }
        // The browser reads thumbnails straight from the same cache.
        ui.thumbnails.attach(
            proxies.source().cache,
            bettercut_ui::media_jobs::THUMBNAIL_WIDTH,
        );
        ui.waveforms.attach(proxies.source().cache);
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
            applied_mode: mode,
            last_title,
            voiceover: Default::default(),
            last_scrub: None,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // §56: drain the whole queue once per frame, never per event.
        bettercut_ui::consume_events(self.events.drain(), &self.editor, &mut self.ui);

        // Closing with unsaved work asks first (`save_prompt`); once it is
        // answered, `quit_now` lets the close through.
        if self.ui.quit_now {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ui.ctx().input(|i| i.viewport().close_requested()) && self.editor.is_dirty() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ui.pending_switch = Some(bettercut_ui::save_prompt::Switch::Quit);
            self.ui.needs_repaint = true;
        }

        // An edit that changed how a clip looks without moving the playhead —
        // a slider, a drag on the picture, a keyframe — leaves the composited
        // frame stale. The events were drained above; act on them before the
        // preview decides whether it has work to do.
        if std::mem::take(&mut self.ui.preview_is_stale)
            && let Some(preview) = self.preview.as_mut()
        {
            preview.invalidate_render();
        }

        if let Some(preview) = self.preview.as_mut() {
            preview.set_compare_original(self.ui.compare_original);
            preview.set_compare_split(self.ui.compare_split);

            // §10's scrub: the playhead moved while stopped — dragged along
            // the ruler, nudged a frame, jumped to a marker — so a short burst
            // of the mix from there is played. Noticed here rather than at
            // every place that moves it, because every one of them should be
            // audible.
            let at = self.editor.playhead();
            if self.ui.audio_scrub && Some(at) != self.last_scrub && !preview.is_playing() {
                preview.scrub_audio(at);
            }
            self.last_scrub = Some(at);
        }

        // Playback work happens before drawing, so the frame painted this pass
        // is the one the clock is asking for rather than the previous one.
        let playing = match self.preview.as_mut() {
            Some(preview) => preview.update(&mut self.editor),
            None => false,
        };

        // A font picked with Import Font: copied into the fonts folder, then
        // loaded into the preview so the title can use it at once. Every export
        // loads the folder when it starts.
        if let Some(path) = self.ui.font_import.take() {
            let folder = bettercut_editor_core::text::user_fonts_dir();
            match bettercut_editor_core::text::import_font_into(&path, &folder) {
                Ok((copied, families)) => {
                    if let Some(preview) = self.preview.as_mut() {
                        if let Err(err) = preview.add_font_file(&copied) {
                            self.ui.error(err);
                        }
                        self.ui.font_families = preview.font_families();
                    }
                    self.ui.info(format!("Font added: {}", families.join(", ")));
                    self.ui.font_imported = families.into_iter().next();
                    self.ui.needs_repaint = true;
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // A voiceover started or stopped from the transport.
        self.voiceover
            .sync(&mut self.editor, &mut self.ui, self.preview.as_mut());

        // §52: mirror the playback counters so the System panel can show them.
        self.ui.playback = self.preview.as_ref().map(bettercut_ui::Preview::stats);

        // §20a: and what each sound lane is putting into the mix, for the
        // meters in the track heads. Taken every frame, because a meter that
        // is only read while something else happens is a meter that sticks.
        let live = self
            .preview
            .as_ref()
            .map(bettercut_ui::Preview::lane_levels)
            .unwrap_or_default();
        let since = ui.ctx().input(|i| i.stable_dt);
        self.ui.settle_lane_levels(&live, since);
        // A meter on its way down needs frames to come down in. Playback asks
        // for those anyway; this is for the moment after it stops.
        if self
            .ui
            .lane_levels
            .iter()
            .any(|(left, right)| *left > 0.0 || *right > 0.0)
        {
            ui.ctx().request_repaint();
        }

        // §13: pick up proxies that finished encoding, and queue any new
        // imports. Both are cheap when nothing has changed.
        let update = self.proxies.poll();
        for media in &update.ready {
            if let Some(preview) = self.preview.as_mut() {
                preview.proxy_ready(*media);
            }
        }
        // A file now read differently (deinterlaced, or no longer): its
        // decoders and thumbnails start again.
        for media in std::mem::take(&mut self.ui.media_reopen) {
            if let Some(preview) = self.preview.as_mut() {
                preview.proxy_ready(media);
            }
            self.ui.thumbnails.invalidate(media);
        }
        for media in &update.thumbnails {
            // Drop the remembered "no thumbnail yet" so it loads next frame.
            self.ui.thumbnails.invalidate(*media);
        }
        for media in &update.waveforms {
            self.ui.waveforms.invalidate(*media);
        }
        for message in update.messages {
            self.ui.info(message);
        }
        for (track, at, path) in update.bounced {
            match self.editor.place_bounce(track, &path, at) {
                Ok(_) => self.ui.info("Track bounced to one clip"),
                Err(err) => self.ui.error(err.to_string()),
            }
        }
        for (range, path, fingerprint) in update.rendered {
            match self.editor.import_baked(&path) {
                Ok(media) => match self.editor.record_render(range, media, fingerprint) {
                    Ok(()) => {
                        // Re-baking a stretch writes over the file the same id
                        // already names, so any decoder holding it open has to
                        // let go — the same reopen a finished proxy asks for.
                        if let Some(preview) = self.preview.as_mut() {
                            preview.proxy_ready(media);
                        }
                        self.ui.info("Rendered");
                    }
                    Err(err) => self.ui.error(err.to_string()),
                },
                Err(err) => self
                    .ui
                    .error(format!("Could not read the render back: {err}")),
            }
        }
        if let Some((clip, grade, label)) = update.colour_match {
            match self.editor.set_clip_grade(clip, grade, label) {
                Ok(()) => self.ui.info(if label == "Auto Level" {
                    "Levels set"
                } else {
                    "Colour matched"
                }),
                Err(err) => self.ui.error(err.to_string()),
            }
        }
        for (incoming, width, height, rgba) in update.trim_frames {
            self.ui
                .trim
                .arrived(ui.ctx(), incoming, width, height, &rgba);
            ui.ctx().request_repaint();
        }
        if let Some((at, width, height, rgba)) = update.scope_frame {
            self.ui.scopes.arrived(at, width, height, &rgba);
            ui.ctx().request_repaint();
        }
        if update.scope_failed {
            self.ui.scopes.pending = false;
        }
        if let Some((width, height, rgba)) = update.copied_frame {
            ui.ctx()
                .copy_image(egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba,
                ));
            self.ui.info(format!("Frame copied ({width}×{height})"));
        }
        for failure in update.failures {
            self.ui.error(failure);
        }
        if !update.ready.is_empty() || !update.thumbnails.is_empty() || !update.waveforms.is_empty()
        {
            self.ui.needs_repaint = true;
        }

        // §42: keep the status bar honest about work still running. Repainting
        // while jobs are in flight is what makes the bar move at all.
        self.ui.proxy_progress = self
            .proxies
            .overall_progress()
            .map(|fraction| (self.proxies.active_jobs(), fraction));

        let export = self.proxies.export_progress();
        self.ui.export_progress = export.map(|(_, fraction)| fraction);
        // The queue window's requests, then what it should show next.
        if let Some(index) = self.ui.export_queue.remove.take() {
            self.proxies.remove_waiting_export(index);
        }
        if let Some((from, to)) = self.ui.export_queue.move_to.take() {
            self.proxies.move_waiting_export(from, to);
        }
        if std::mem::take(&mut self.ui.export_queue.clear) {
            let removed = self.proxies.clear_waiting_exports();
            if removed > 0 {
                self.ui
                    .info(format!("Took {removed} export(s) off the queue"));
            }
        }
        self.ui.export_queue.running = self.proxies.running_export_label();
        self.ui.export_queue.waiting = self.proxies.waiting_export_labels();
        if self.ui.export_stop_requested {
            self.ui.export_stop_requested = false;
            if let Some((id, _)) = export {
                self.proxies.cancel(id);
            }
        }

        if self.ui.proxy_progress.is_some() || self.ui.export_progress.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }

        bettercut_ui::draw(ui, &mut self.editor, &mut self.ui, self.preview.as_mut());

        // The Export window is drawn here rather than inside `draw`, because
        // starting an export needs the job scheduler and the scheduler belongs
        // to the shell. §74: it runs on a worker, never on this thread.
        let exporting = self.proxies.export_in_flight().is_some();
        let mut dialog = std::mem::take(&mut self.ui.export_dialog);
        let requested = bettercut_ui::export_dialog::show(
            ui.ctx(),
            &self.editor,
            &mut self.ui,
            &mut dialog,
            exporting,
        );
        self.ui.export_dialog = dialog;

        let files = requested.len();
        for (index, request) in requested.into_iter().enumerate() {
            // An extra shape exports a reshaped copy, so the edit on screen
            // never changes shape under the user.
            let copied = self
                .editor
                .export_copy_of(request.sequence, request.shape)
                .and_then(|(mut project, sequence)| {
                    if let Some(lane) = request.stem {
                        bettercut_editor_core::stems::isolate_sound_lane(
                            &mut project,
                            sequence,
                            lane,
                        )?;
                    }
                    Ok((project, sequence))
                });
            match copied {
                Ok((project, sequence)) => {
                    let cover_path = (!request.settings.sound_only).then(|| {
                        bettercut_editor_core::cover::cover_path_for(&request.settings.path)
                    });
                    let job =
                        bettercut_export::ExportJob::new(&project, sequence, request.settings);
                    if index == 0 {
                        let label = job.label_for_status();
                        self.ui.info(match files {
                            1 => label,
                            n => format!("{label}, then {} more file(s)", n - 1),
                        });
                    }
                    // The cover, beside the file, when one was chosen — a
                    // picture of the same copy the export reads.
                    let cover = project.sequence(sequence).and_then(|s| s.cover_frame);
                    if let (Some(at), Some(path)) = (cover, cover_path) {
                        self.proxies.submit_still(bettercut_export::StillJob::new(
                            project.clone(),
                            sequence,
                            at,
                            path,
                        ));
                    }
                    self.proxies.submit_export(job);
                    self.ui.needs_repaint = true;
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // A saved frame renders on the scheduler for the same reason: it opens
        // a GPU device and seeks a decoder, neither of which the UI waits on.
        if let Some((path, at)) = self.ui.still_request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    let job = bettercut_export::StillJob::new(project, sequence, at, path);
                    self.ui
                        .info(format!("Saving the frame at {}", at.format_timecode()));
                    self.proxies.submit_still(job);
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // Render in place: the marked stretch baked to a file on the
        // scheduler, like any other export (§74), and remembered when it
        // lands (below, with the other finished jobs).
        if let Some(range) = self.ui.render_request.take() {
            match bettercut_ui::render::render_job(&self.editor, range) {
                Ok((job, path, fingerprint)) => {
                    self.ui.info("Rendering the marked stretch");
                    self.proxies.submit_render(job, range, path, fingerprint);
                }
                Err(err) => self.ui.error(err),
            }
        }

        // Bouncing a sound lane to one clip: mixed on the scheduler, put down
        // when it lands (below, with the other finished jobs).
        if let Some(track) = self.ui.bounce_request.take() {
            match bettercut_ui::bounce::bounce_job(&self.editor, track) {
                Ok(job) => self.proxies.submit_bounce(job),
                Err(err) => self.ui.error(err),
            }
        }

        // A contact sheet: a frame per tile, so it goes on the scheduler
        // like every other render (§74).
        if let Some(path) = self.ui.contact_sheet_request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    let range = project.sequence(sequence).and_then(|active| {
                        active.marked_range().or_else(|| {
                            bettercut_editor_core::timeline::TimelineRange::new(
                                bettercut_editor_core::foundation::TimelineTime::ZERO,
                                active.duration(),
                            )
                            .ok()
                        })
                    });
                    match range {
                        Some(range) => {
                            let settings = bettercut_export::SheetSettings::of(path, range);
                            self.ui.info("Making the contact sheet\u{2026}");
                            self.proxies.submit_still_sheet(
                                bettercut_export::ContactSheetJob::new(project, sequence, settings),
                            );
                        }
                        None => self.ui.error("There is nothing in this sequence yet"),
                    }
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // A frame for the clipboard: rendered like a saved one, then copied
        // when it arrives (below, with the other finished jobs).
        if let Some(at) = self.ui.copy_frame_request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    self.ui
                        .info(format!("Copying the frame at {}", at.format_timecode()));
                    self.proxies
                        .submit_frame_grab(bettercut_export::FrameGrabJob::new(
                            project, sequence, at,
                        ));
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // Both sides of the cut, for the trim window: two renders, asked for
        // once per cut.
        if let Some((outgoing, incoming)) = self.ui.trim.request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    for (is_incoming, at) in [(false, outgoing), (true, incoming)] {
                        self.proxies.submit_trim_grab(
                            is_incoming,
                            bettercut_export::FrameGrabJob::new(project.clone(), sequence, at),
                        );
                    }
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // The scopes' frame, when the Scopes window has settled on one.
        if let Some(at) = self.ui.scopes.request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    self.proxies.submit_scope_grab(
                        at,
                        bettercut_export::FrameGrabJob::new(project, sequence, at),
                    );
                }
                Err(_) => self.ui.scopes.pending = false,
            }
        }

        // A colour match: worked out in the background, applied when it
        // arrives (below), as one undo step.
        if let Some(clip) = self.ui.auto_level_request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    self.ui.info("Setting levels from the clip's own frame");
                    self.proxies
                        .submit_colour_match(bettercut_export::ColourMatchJob::auto_level(
                            project, sequence, clip,
                        ));
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }
        if let Some((clip, at)) = self.ui.colour_match_request.take() {
            match self.editor.export_copy(None) {
                Ok((project, sequence)) => {
                    self.ui
                        .info("Matching colour to the frame under the playhead");
                    self.proxies
                        .submit_colour_match(bettercut_export::ColourMatchJob::new(
                            project, sequence, clip, at,
                        ));
                }
                Err(err) => self.ui.error(err.to_string()),
            }
        }

        // Milestone 12's scene detection, started here for the same reason as export —
        // it needs the scheduler, and the scheduler belongs to the shell.
        if let Some(clip) = self.ui.scene_request.take() {
            let threads = self
                .editor
                .hardware()
                .ffmpeg_threads_per_job(self.editor.project().settings.performance_mode);
            match bettercut_ui::scene_dialog::start(&self.editor, &mut self.proxies, clip, threads)
            {
                Ok(dialog) => self.ui.scenes = Some(dialog),
                Err(message) => self.ui.error(message),
            }
            self.ui.needs_repaint = true;
        }
        if let Some(job) = self.ui.scene_cancel.take() {
            self.proxies.cancel(job);
        }

        // After drawing, because both the import button and the quality setting
        // live in panels this call would otherwise be a frame behind.
        // §43: a mode change must reach the parts that can follow it live. The
        // frame cache can be resized; the job pool's concurrency limit is fixed
        // for the session, because restarting the scheduler would cancel work
        // already running. That limit takes effect next launch.
        let mode = self.editor.project().settings.performance_mode;
        if mode != self.applied_mode {
            self.applied_mode = mode;
            let bytes = self.editor.hardware().frame_cache_bytes(mode);
            if let Some(preview) = self.preview.as_mut() {
                preview.set_cache_bytes(bytes);
            }
            tracing::info!(
                ?mode,
                cache_mb = bytes / (1024 * 1024),
                "performance mode changed"
            );
        }

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

        // §39: this is the orderly shutdown `Journal::discard` is documented
        // for, and nothing else reaches it. Without this every clean quit left
        // its session behind, so the next launch offered to recover work the
        // user already had — and, for a saved project, left a `recovery`
        // directory sitting beside their project file for good.
        //
        // `shutdown` keeps the data when there are unsaved changes. That
        // decision belongs with the journal, not here.
        self.editor.shutdown();
    }
}
