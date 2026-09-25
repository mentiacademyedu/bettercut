//! The preview: playback clock, compositor, and the texture egui paints.
//!
//! This is where §4.1's central claim finally pays off. The compositor renders
//! into a `wgpu::Texture`; `egui-wgpu` registers it as a `TextureId`; the
//! preview panel paints it like any other image. No frame copy, no IPC, no
//! overlay window — and panels and dialogs can overlap it freely.
//!
//! §20a.1 runs the loop: the audio device advances the clock, the clock picks
//! the playhead, and the picture follows.

use bettercut_audio::PlaybackClock;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_playback::{PlaybackEngine, SyncDecision, plan_frame};
use bettercut_renderer::{Compositor, GpuDescription, Layer, PreviewQuality, RenderConfig};
use eframe::egui_wgpu::RenderState;

/// Everything needed to show moving pictures.
pub struct Preview {
    compositor: Compositor,
    /// Colour lookup tables already offered to the compositor, so each file is
    /// read once — and a newly imported one is picked up on the next frame.
    tried_luts: std::collections::HashSet<bettercut_editor_core::foundation::LutId>,
    texture_id: egui::TextureId,
    render_state: RenderState,

    engine: PlaybackEngine,
    clock: PlaybackClock,
    /// §20a.2's mixer thread, which owns the device's ring buffer. `None` when
    /// there is no audio device, which is not a failure (§50) — the picture
    /// still plays.
    mixer: Option<bettercut_playback::MixerThread>,
    /// The last snapshot sent to it, so a new one goes only when the audio
    /// changed. A slider drag on a video clip is a project change sixty times
    /// a second, and none of them concern the mixer.
    sent_plan: Option<bettercut_playback::AudioPlan>,

    quality: PreviewQuality,

    /// The adapter wgpu chose (§49). Kept so the System panel and any bug
    /// report can name it — performance claims are meaningless without it.
    gpu: GpuDescription,

    /// Height of the proxies being read, if any — decode-ahead is sized
    /// against the frames actually decoded, not the sequence resolution (§14).
    proxy_height: Option<u32>,

    /// Last position actually composited, so a still frame is not re-rendered
    /// sixty times a second while paused (§81's idle target).
    last_rendered: Option<TimelineTime>,
    /// Drawing every clip without its grade, for a before/after compare.
    compare_original: bool,
    /// Showing the graded picture and the original side by side: the frame is
    /// composited twice, and the ungraded one is kept for the panel to draw on
    /// the other side of the divider.
    compare_split: bool,
    /// The kept frame, registered with egui. Made the first time a split is
    /// asked for.
    snapshot_id: Option<egui::TextureId>,
    /// Consecutive dropped frames, feeding §17's quality reduction.
    consecutive_drops: u32,
    /// Consecutive frames presented on time, feeding §17's recovery.
    consecutive_on_time: u32,
    /// When quality last changed. §17: *"do not change quality more than once
    /// per second, or the preview will visibly oscillate."*
    last_quality_change: std::time::Instant,
    dropped_total: u64,
    has_content: bool,
    /// Playback state last frame, so stopping can trigger a full-quality
    /// redraw of the frame the user is left looking at.
    was_playing: bool,

    /// Source size of each layer in the last composited frame.
    ///
    /// The preview's drag handles need to know how big a layer's picture is to
    /// work out where it landed. For footage that is the media's own size,
    /// which the project already knows — but §26's titles are rasterized, and
    /// how wide "Hello" comes out is not something the model can answer. So the
    /// render records it, and the overlay reads it back.
    layer_sizes: Vec<(bettercut_editor_core::foundation::ClipId, u32, u32)>,

    /// A J/L shuttle at any speed but normal forward play (`crate::shuttle`):
    /// the rate, and when the playhead was last moved for it. Silent, and
    /// driven from here rather than by the audio clock.
    shuttle: Option<(i32, std::time::Instant)>,

    /// Normal playback repeats the in-to-out range — or the whole sequence,
    /// without marks — instead of stopping at its end.
    looping: bool,
}

impl Preview {
    pub fn new(
        render_state: &RenderState,
        cache_bytes: usize,
        decoder_threads: u32,
    ) -> Result<Self, bettercut_renderer::RenderError> {
        let resolution =
            PreviewQuality::default().apply(bettercut_editor_core::timeline::Resolution::HD_1080);
        let compositor = Compositor::new(
            render_state.device.clone(),
            render_state.queue.clone(),
            RenderConfig::preview(resolution),
        )?;

        // `present_view`, not `target_view`: egui applies its own gamma decode
        // and expects raw sRGB bytes, so an sRGB-aware view would be
        // linearized twice and the picture would come out very dark.
        let texture_id = render_state.renderer.write().register_native_texture(
            &render_state.device,
            compositor.present_view(),
            bettercut_renderer::wgpu::FilterMode::Linear,
        );

        let gpu = GpuDescription::describe(&render_state.adapter);
        tracing::info!(
            gpu = %gpu.name,
            kind = gpu.kind.label(),
            backend = %gpu.backend,
            driver = %gpu.driver,
            hardware_id = %gpu.hardware_id,
            surface = ?render_state.target_format,
            "graphics adapter selected"
        );
        if gpu.kind.is_software() {
            // §50: keep running, but say why it will be slow rather than
            // letting the user conclude the editor is simply bad.
            tracing::warn!("no hardware GPU found; rendering in software");
        }

        let (clock, sink) = PlaybackClock::open();
        tracing::info!(audio = %clock.describe(), "playback ready");

        let mixer = sink.and_then(|sink| {
            match bettercut_playback::MixerThread::spawn(sink, decoder_threads) {
                Ok(mixer) => Some(mixer),
                Err(err) => {
                    // §50: no mixer means no sound, not no editor.
                    tracing::error!(%err, "could not start the audio mixer; playback will be silent");
                    None
                }
            }
        });

        Ok(Self {
            compositor,
            tried_luts: std::collections::HashSet::new(),
            texture_id,
            render_state: render_state.clone(),
            engine: PlaybackEngine::new(cache_bytes, decoder_threads),
            clock,
            mixer,
            sent_plan: None,
            quality: PreviewQuality::default(),
            gpu,
            proxy_height: None,
            last_rendered: None,
            compare_original: false,
            compare_split: false,
            snapshot_id: None,
            consecutive_drops: 0,
            consecutive_on_time: 0,
            last_quality_change: std::time::Instant::now(),
            dropped_total: 0,
            has_content: false,
            was_playing: false,
            layer_sizes: Vec::new(),
            shuttle: None,
            looping: false,
        })
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture_id
    }

    /// The colour of one pixel of the picture on screen, in sRGB (§45).
    ///
    /// For the eyedropper: a green screen is named by pointing at it, not by
    /// hunting for it on a colour wheel.
    pub fn read_pixel(&self, x: u32, y: u32) -> Option<[f32; 3]> {
        self.compositor.read_pixel(x, y)
    }

    pub fn is_playing(&self) -> bool {
        self.clock.is_playing() || self.shuttle.is_some()
    }

    /// How big the source picture for `clip` was in the last composited frame.
    ///
    /// `None` when it was not on screen, or before the first render. §26's
    /// titles have no size until they have been rasterized, so the overlay
    /// simply has no handles to draw for one yet — which resolves itself on the
    /// next frame.
    pub fn source_size(
        &self,
        clip: bettercut_editor_core::foundation::ClipId,
    ) -> Option<(u32, u32)> {
        // The last layer with the id: a backdrop is drawn first under the
        // same clip, and a picture backdrop is another shape entirely.
        self.layer_sizes
            .iter()
            .rev()
            .find(|(id, _, _)| *id == clip)
            .map(|&(_, width, height)| (width, height))
    }

    /// The font families available for §26's text overlays.
    /// Make an imported font available to the titles on screen, and redraw.
    pub fn add_font_file(&mut self, path: &std::path::Path) -> Result<Vec<String>, String> {
        let families = self.engine.add_font_file(path).map_err(|e| e.to_string())?;
        self.last_rendered = None;
        Ok(families)
    }

    pub fn font_families(&self) -> Vec<String> {
        self.engine.font_families()
    }

    pub fn has_content(&self) -> bool {
        self.has_content
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_total
    }

    pub fn underruns(&self) -> u32 {
        self.clock.clock().underruns()
    }

    /// Everything worth reporting about a playback session (§52, §81).
    ///
    /// Gathered in one call so the panel showing it cannot drift from the
    /// engine, and so a bug report is a screenshot rather than a description.
    pub fn stats(&self) -> crate::state::PlaybackStats {
        crate::state::PlaybackStats {
            playing: self.clock.is_playing(),
            dropped_frames: self.dropped_total,
            underruns: self.clock.clock().underruns(),
            limited_samples: self.mixer.as_ref().map_or(0, |m| m.limited_samples()),
            peaks: self.mixer.as_ref().map_or((0.0, 0.0), |m| m.peaks()),
            loudness: self.mixer.as_ref().map_or((None, None), |m| m.loudness()),
            prefetch_hits: self.engine.prefetch_hits(),
            ring_frames: self.engine.prefetched_frames(),
            quality: match self.render_quality() {
                bettercut_renderer::PreviewQuality::Full => "full",
                bettercut_renderer::PreviewQuality::Half => "half",
                bettercut_renderer::PreviewQuality::Quarter
                | bettercut_renderer::PreviewQuality::Auto => "quarter",
            },
        }
    }

    /// What each sound lane is putting into the mix right now, per side, in
    /// lane order (`bettercut_playback::lane_meters`). Empty without a mixer,
    /// and all silence while playback is stopped.
    pub fn lane_levels(&self) -> Vec<(f32, f32)> {
        self.mixer
            .as_ref()
            .map(|m| m.lane_peaks())
            .unwrap_or_default()
    }

    pub fn audio_description(&self) -> String {
        self.clock.describe()
    }

    /// Whether playback loops (`crate::shuttle::loop_range`).
    pub fn is_looping(&self) -> bool {
        self.looping
    }

    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// How fast and which way the playhead is moving: 0 stopped, 1 normal
    /// playback, otherwise a shuttle rate (`crate::shuttle`).
    pub fn shuttle_rate(&self) -> i32 {
        match self.shuttle {
            Some((rate, _)) => rate,
            None => i32::from(self.clock.is_playing()),
        }
    }

    /// Move at `rate`: stopped, normal playback with sound, or a silent
    /// shuttle at any other speed or backwards.
    pub fn set_shuttle(&mut self, editor: &Editor, rate: i32) {
        match rate {
            0 => self.set_playing(editor, false),
            1 => self.set_playing(editor, true),
            _ => {
                self.set_playing(editor, false);
                self.shuttle = Some((rate, std::time::Instant::now()));
            }
        }
    }

    /// Start or stop playback (§55 `playback.play` / `playback.pause`).
    ///
    /// Either way, a shuttle stops: Space during a fast forward means stop,
    /// and play means play normally.
    pub fn set_playing(&mut self, editor: &Editor, playing: bool) {
        self.shuttle = None;
        if playing {
            // Start the clock and the audio fill from wherever the playhead is,
            // or the sound would resume from where it last stopped.
            self.clock.seek_to(editor.playhead());
            // The plan first, so the first block mixed is from the timeline as
            // it is now rather than as it was when playback last stopped.
            self.send_plan(editor);
            if let Some(mixer) = &self.mixer {
                mixer.seek(editor.playhead());
            }
            // §47a.3: decode ahead only while playing. A paused editor has
            // nothing to run ahead of, and holding a decode thread and its
            // share of the frame budget for nothing works against §81.
            self.engine.start_prefetch(self.prefetch_budget(editor));
        } else {
            self.engine.stop_prefetch();
        }
        if let Some(mixer) = &self.mixer {
            mixer.set_playing(playing);
        }
        self.clock.set_playing(playing);
    }

    /// Bytes to reserve for decode-ahead (§47a.3, §81).
    ///
    /// Sized from the frames that will actually be decoded — the proxy when
    /// there is one (§14), which is the case this exists to serve. Half a
    /// second rather than a full one: §47a.3 allows 0.5–1.0 s, and at 1080p the
    /// upper end would want ~250 MB, half of §81's entire budget.
    fn prefetch_budget(&self, editor: &Editor) -> usize {
        let Some(sequence) = editor.active_sequence() else {
            return 8 * 1024 * 1024;
        };
        let resolution = sequence.resolution;
        let height = self
            .proxy_height
            .unwrap_or(resolution.height)
            .min(resolution.height.max(1));
        let width = if resolution.height > 0 {
            (u64::from(height) * u64::from(resolution.width) / u64::from(resolution.height)) as u32
        } else {
            height
        };

        let frame = (width as usize).saturating_mul(height as usize) * 4;
        bettercut_playback::budget_for(frame, sequence.frame_rate.as_f64(), 0.5)
    }

    /// Move the clock to follow a user-driven seek.
    pub fn seek_to(&mut self, position: TimelineTime) {
        // A shuttle carries on from where the playhead was moved to.
        if let Some((_, since)) = &mut self.shuttle {
            *since = std::time::Instant::now();
        }
        self.clock.seek_to(position);
        if let Some(mixer) = &self.mixer {
            mixer.seek(position);
        }
        // §47a.5: frames queued for where the playhead was are worthless, and
        // their bytes are needed for where it is going.
        self.engine.reset_prefetch();
        // Force a redraw: the picture must change even though we are paused.
        self.last_rendered = None;
    }

    /// Re-composite on the next update, even though the playhead has not moved.
    ///
    /// The picture is otherwise cached against the playhead position, which is
    /// right for scrubbing and wrong for every edit that changes how a clip
    /// looks while standing still — a slider, a drag on the preview, a
    /// keyframe, a track hidden. Those all reach here through §56's event
    /// stream rather than by this module watching the project.
    pub fn invalidate_render(&mut self) {
        self.last_rendered = None;
    }

    /// Show every clip without its grade and effects, to compare with the
    /// original — or with them again. Redraws when it changes.
    pub fn set_compare_original(&mut self, on: bool) {
        if self.compare_original != on {
            self.compare_original = on;
            self.last_rendered = None;
        }
    }

    /// Play a short burst of the mix from `at` — what makes dragging the
    /// playhead audible. Does nothing while playing, or with no sound device.
    pub fn scrub_audio(&self, at: TimelineTime) {
        if self.clock.is_playing() {
            return;
        }
        if let Some(mixer) = self.mixer.as_ref() {
            mixer.scrub(at);
        }
    }

    /// Show the original beside the graded picture rather than instead of it.
    pub fn set_compare_split(&mut self, on: bool) {
        if self.compare_split != on {
            self.compare_split = on;
            self.last_rendered = None;
        }
    }

    /// The kept, ungraded frame, for the panel to draw beside the live one.
    /// `None` until a split has actually been composited.
    pub fn snapshot_id(&self) -> Option<egui::TextureId> {
        self.snapshot_id
    }

    /// One frame of playback work. Returns whether a repaint is needed.
    pub fn update(&mut self, editor: &mut Editor) -> bool {
        self.clock.tick();

        if let Some((rate, since)) = self.shuttle {
            let now = std::time::Instant::now();
            let end = editor
                .active_sequence()
                .map_or(TimelineTime::ZERO, |s| s.duration());
            let (position, at_edge) =
                crate::shuttle::advance(editor.playhead(), rate, now - since, end);
            editor.set_playhead(position);
            self.shuttle = (!at_edge).then_some((rate, now));
        }

        let playing = self.clock.is_playing() || self.shuttle.is_some();
        if self.clock.is_playing() {
            self.pump_audio(editor);
            self.pump_prefetch(editor);

            // §20a.1: the picture follows the clock, never the other way round.
            let position = self.clock.position();
            editor.set_playhead(position);

            let range = editor
                .active_sequence()
                .and_then(crate::shuttle::loop_range);
            match range {
                // Round again: the clock and the sound jump back together,
                // and playback carries on.
                Some(range) if self.looping => {
                    if let Some(start) = crate::shuttle::loop_restart(position, range) {
                        editor.set_playhead(start);
                        self.seek_to(start);
                    }
                }
                _ => {
                    if let Some(end) = editor.active_sequence().map(|s| s.duration())
                        && position >= end
                    {
                        self.clock.set_playing(false);
                    }
                }
            }
        }

        // Stopping is a reason to redraw: the frame on screen was rendered at
        // playback quality, and now there is time to do it properly.
        if self.was_playing && !playing {
            self.last_rendered = None;
        }
        self.was_playing = playing;

        let position = editor.playhead();
        let needs_render = self.last_rendered != Some(position);
        if needs_render {
            self.render(editor, position);
        }

        playing || needs_render
    }

    /// The scale to render at right now (§16, §17).
    ///
    /// §17's reduction exists to keep *playback* smooth — it trades resolution
    /// for the ability to hit the frame deadline. A paused preview has no
    /// deadline, so the trade buys nothing and costs everything: the still
    /// frame the user is actually looking at, and scrubbing to, would sit at a
    /// quarter of each dimension. On a 1080p sequence that is 480x270 stretched
    /// across the panel, which reads as "the preview is broken" rather than as
    /// an adaptive quality system doing its job.
    fn render_quality(&self) -> PreviewQuality {
        if self.clock.is_playing() {
            self.quality
        } else {
            PreviewQuality::Full
        }
    }

    /// Keep the decode-ahead ring topped up (§47a.3).
    ///
    /// Planning is cheap — it resolves which media each upcoming instant needs
    /// and skips anything already decoded — so running it every frame is
    /// simpler and more responsive than tracking how far ahead we last planned.
    fn pump_prefetch(&mut self, editor: &Editor) {
        let Some(sequence) = editor.active_sequence() else {
            return;
        };
        // Half a second, matching what the budget was sized for.
        let span = TimelineTime::from_millis(500);
        let project = editor.project();
        self.engine
            .prefetch_ahead(project, sequence, self.clock.position(), span);
    }

    /// Keep the mixer's snapshot current (§20a.2).
    ///
    /// The mixing itself happens on the mixer thread; all that is left here is
    /// telling it when the audio changed, so an edit made during playback is
    /// heard. Compared first, because sending is not free and most frames
    /// change nothing it cares about.
    fn pump_audio(&mut self, editor: &Editor) {
        self.send_plan(editor);
    }

    fn send_plan(&mut self, editor: &Editor) {
        let Some(mixer) = &self.mixer else {
            return;
        };
        let Some(sequence) = editor.active_sequence() else {
            return;
        };
        let plan = bettercut_playback::AudioPlan::of(editor.project(), sequence);
        if self.sent_plan.as_ref() != Some(&plan) {
            mixer.set_plan(plan.clone());
            self.sent_plan = Some(plan);
        }
    }

    fn render(&mut self, editor: &Editor, position: TimelineTime) {
        let Some(sequence) = editor.active_sequence() else {
            return;
        };

        // Keep the preview's aspect matched to the sequence, scaled by §17's
        // quality setting while playing and full while paused.
        let wanted = self.render_quality().apply(sequence.resolution);
        match self.compositor.set_resolution(wanted) {
            Ok(true) => self.reregister_texture(),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(%err, "could not resize the preview");
                return;
            }
        }

        let resolved = self
            .engine
            .resolve_video(editor.project(), sequence, position);
        self.has_content = !resolved.is_empty();

        self.layer_sizes.clear();
        self.layer_sizes.extend(
            resolved
                .iter()
                .map(|layer| (layer.clip, layer.frame.width, layer.frame.height)),
        );

        // §20a.5 / §47a.4: decide what to do with what we got. With a single
        // still frame there is nothing to drop, but the accounting is what
        // drives §17's quality reduction once playback is under load.
        if self.clock.is_playing() {
            let interval = TimelineTime::from_ticks(sequence.ticks_per_frame());
            let clock_time = self.clock.position();
            let frame_time = resolved.first().map_or(clock_time, |_| position);

            let plan = plan_frame(frame_time, clock_time, interval, self.consecutive_drops);
            match plan.decision {
                SyncDecision::Drop => {
                    self.consecutive_drops += 1;
                    self.consecutive_on_time = 0;
                    self.dropped_total += 1;
                }
                SyncDecision::Present | SyncDecision::Hold => {
                    self.consecutive_drops = 0;
                    self.consecutive_on_time += 1;
                }
            }
            if plan.drift_is_alarming {
                tracing::warn!(
                    drift_ticks = plan.drift.ticks(),
                    "sustained A/V drift beyond the §20a.5 threshold"
                );
            }
            if plan.should_reduce_quality {
                self.reduce_quality();
            } else if self.consecutive_on_time > 120 {
                // Two seconds of clean playback: try climbing back up (§17).
                self.raise_quality();
            }
        }

        // Side by side: the original first, kept, and then the graded frame
        // over it — two composites of the same instant.
        if self.compare_split && !self.compare_original {
            self.composite(editor, sequence, position, &resolved, true);
            if let Err(err) = self.compositor.keep_snapshot() {
                tracing::warn!(%err, "could not keep the frame to compare against");
            } else if self.snapshot_id.is_none()
                && let Some(view) = self.compositor.snapshot_view()
            {
                self.snapshot_id =
                    Some(self.render_state.renderer.write().register_native_texture(
                        &self.render_state.device,
                        view,
                        bettercut_renderer::wgpu::FilterMode::Linear,
                    ));
            }
        }

        self.composite(editor, sequence, position, &resolved, self.compare_original);
        self.last_rendered = Some(position);
    }

    /// Composite one frame, graded or as the footage came.
    ///
    /// Split out so the same instant can be drawn twice — once each way — for
    /// the side-by-side comparison.
    fn composite(
        &mut self,
        editor: &Editor,
        sequence: &bettercut_editor_core::timeline::Sequence,
        position: TimelineTime,
        resolved: &[bettercut_playback::engine::ResolvedLayer],
        original: bool,
    ) {
        let master = sequence.master;
        let layers: Vec<Layer<'_>> = resolved
            .iter()
            .map(|resolved| Layer {
                frame: &resolved.frame,
                look: if original {
                    resolved.look.ungraded()
                } else {
                    resolved.look
                },
            })
            .collect();

        // Adjustment lanes, graded over the pictures actually resolved — the
        // same two functions the export calls, so what is graded here is what
        // is graded there (§46).
        let beneath =
            bettercut_playback::graded_beneath(sequence, resolved.iter().map(|layer| layer.track));
        let grades: Vec<bettercut_renderer::Grade> =
            bettercut_playback::adjustments_at(sequence, position)
                .into_iter()
                .map(|look| bettercut_renderer::Grade { beneath, look })
                .collect();

        self.compositor
            .set_grain_seed(bettercut_playback::grain_seed(sequence, position));
        let compositor = &mut self.compositor;
        bettercut_playback::load_luts(editor.project(), &mut self.tried_luts, |id, lut| {
            compositor.load_lut(id, lut);
        });
        // The original has no adjustment grades and no master grade either.
        let (grades, master) = if original {
            (
                Vec::new(),
                bettercut_editor_core::timeline::MasterLook {
                    background: master.background,
                    transform: master.transform,
                    opacity: master.opacity,
                    ..bettercut_editor_core::timeline::MasterLook::default()
                },
            )
        } else {
            (grades, master)
        };
        if let Err(err) = self.compositor.composite_graded(&layers, &grades, master) {
            tracing::warn!(%err, "compositing failed");
        }
    }

    /// §17: step down a quality level, at most one step at a time.
    fn reduce_quality(&mut self) {
        let next = match self.quality {
            PreviewQuality::Full => PreviewQuality::Half,
            PreviewQuality::Half | PreviewQuality::Auto => PreviewQuality::Quarter,
            PreviewQuality::Quarter => return, // already at the floor
        };
        self.change_quality(next, "reducing");
    }

    /// §17: climb back up once playback has been comfortable for a while.
    ///
    /// Without this the preview only ever gets worse: one rough patch early in
    /// a session would leave it at quarter resolution for the rest of it.
    fn raise_quality(&mut self) {
        let next = match self.quality {
            PreviewQuality::Quarter | PreviewQuality::Auto => PreviewQuality::Half,
            PreviewQuality::Half => PreviewQuality::Full,
            PreviewQuality::Full => return, // already at the ceiling
        };
        self.change_quality(next, "raising");
    }

    /// Apply a quality change, respecting §17's hysteresis.
    ///
    /// > Add hysteresis - do not change quality more than once per second, or
    /// > the preview will visibly oscillate.
    ///
    /// Without it, a machine sitting right on the edge flips between levels
    /// every few frames, which looks far worse than simply staying low.
    fn change_quality(&mut self, next: PreviewQuality, direction: &str) {
        if self.last_quality_change.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        tracing::info!(from = ?self.quality, to = ?next, "{direction} preview quality");
        self.quality = next;
        self.last_quality_change = std::time::Instant::now();
        self.consecutive_drops = 0;
        self.consecutive_on_time = 0;
    }

    /// Point playback at the proxy cache (§14).
    pub fn set_proxy_source(&mut self, source: bettercut_playback::ProxySource) {
        self.proxy_height = Some(source.height);
        self.engine.set_proxy_source(Some(source));
        self.last_rendered = None;
    }

    /// Resize the frame cache (§18), after a performance-mode change.
    pub fn set_cache_bytes(&mut self, bytes: usize) {
        self.engine.set_cache_bytes(bytes);
    }

    /// The adapter wgpu is rendering with.
    pub fn gpu(&self) -> &GpuDescription {
        &self.gpu
    }

    /// A proxy finished encoding: reopen this asset so preview picks it up.
    pub fn proxy_ready(&mut self, media: bettercut_editor_core::foundation::MediaId) {
        self.engine.invalidate(media);
        self.last_rendered = None;
    }

    /// The texture object changed, so egui's registration has to be updated or
    /// the panel keeps painting the old one.
    fn reregister_texture(&mut self) {
        self.render_state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &self.render_state.device,
                self.compositor.present_view(),
                bettercut_renderer::wgpu::FilterMode::Linear,
                self.texture_id,
            );
    }
}
