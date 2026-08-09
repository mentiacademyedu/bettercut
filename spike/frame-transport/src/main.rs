//! Milestone 0 — Frame Transport Spike (§82).
//!
//! THROWAWAY CODE. No product architecture. Its only job is to answer four
//! questions before any real code is written:
//!
//!   1. Does the §5 path work?  decode -> GPU texture -> wgpu composite ->
//!      egui `TextureId` -> screen, with no overlay window and no IPC.
//!   2. Can it hold 60 fps while compositing two 1080p layers?
//!   3. Does an audio-derived clock (§20a.1) keep video in sync?
//!   4. Is egui's text quality acceptable for the product (§4.1)?
//!
//! What it does NOT yet cover: hardware decode -> texture interop, which needs
//! FFmpeg. The per-frame upload path here is the §5 *software fallback*, so the
//! numbers it reports are the pessimistic bound.
//!
//! Controls: Space play/pause · U toggle per-frame upload · R reset clock

mod audio;
mod compositor;
mod frames;

use std::collections::VecDeque;
use std::time::Instant;

use eframe::egui;

const COMPOSITE_W: u32 = 1920;
const COMPOSITE_H: u32 = 1080;
const OVERLAY_W: u32 = 960;
const OVERLAY_H: u32 = 540;
const FPS: f64 = 30.0;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Milestone 0 — Frame Transport Spike"),
        ..Default::default()
    };

    eframe::run_native(
        "frame-transport-spike",
        options,
        Box::new(|cc| Ok(Box::new(SpikeApp::new(cc)?))),
    )
}

/// A rolling window of samples, reported as average and worst case. Averages
/// alone hide judder; the 99th-percentile frame is what a user actually feels.
struct Rolling {
    samples: VecDeque<f64>,
    capacity: usize,
}

impl Rolling {
    fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn push(&mut self, value: f64) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    fn avg(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f64>() / self.samples.len() as f64
    }

    fn worst(&self) -> f64 {
        self.samples.iter().copied().fold(0.0, f64::max)
    }

    fn p99(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<f64> = self.samples.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[(sorted.len() as f64 * 0.99) as usize % sorted.len()]
    }
}

struct SpikeApp {
    compositor: compositor::Compositor,
    source: frames::SyntheticSource,
    clock: audio::PlaybackClock,

    /// Re-upload layer A every frame (software fallback path) vs. leave it
    /// resident (what hardware-decode interop would give us).
    upload_each_frame: bool,
    expand_limited_range: bool,

    last_video_frame: Option<u64>,
    dropped_frames: u64,
    repeated_frames: u64,
    presented_frames: u64,

    gen_ms: Rolling,
    upload_ms: Rolling,
    submit_ms: Rolling,
    ui_frame_ms: Rolling,
    drift_ms: Rolling,

    last_ui_frame: Instant,
    gpu_wait_ms: f64,
    measure_gpu_next: bool,
    started: Instant,
    last_report: Instant,
}

impl SpikeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("eframe is not running on the wgpu backend")?;

        // §82 asks for this to be recorded: which adapter actually got used.
        let info = rs.adapter.get_info();
        println!(
            "adapter: {} | backend: {:?} | device type: {:?} | driver: {} {}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        );

        let overlay = frames::build_overlay(OVERLAY_W, OVERLAY_H);
        let compositor = compositor::Compositor::new(
            rs,
            COMPOSITE_W,
            COMPOSITE_H,
            &overlay,
            OVERLAY_W,
            OVERLAY_H,
        );

        let clock = audio::PlaybackClock::start();
        println!("audio: {}", clock.description());

        // Start playing immediately so the spike measures itself without anyone
        // having to press a key; the numbers go to stdout every 2 s.
        clock.clock().set_playing(true);
        println!(
            "{:>6} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6}",
            "t(s)", "ui_avg", "ui_p99", "ui_max", "gen", "upload", "submit", "drop", "drift"
        );

        Ok(Self {
            compositor,
            source: frames::SyntheticSource::new(COMPOSITE_W, COMPOSITE_H, FPS),
            clock,
            upload_each_frame: true,
            expand_limited_range: false,
            last_video_frame: None,
            dropped_frames: 0,
            repeated_frames: 0,
            presented_frames: 0,
            gen_ms: Rolling::new(240),
            upload_ms: Rolling::new(240),
            submit_ms: Rolling::new(240),
            ui_frame_ms: Rolling::new(240),
            drift_ms: Rolling::new(240),
            last_ui_frame: Instant::now(),
            gpu_wait_ms: 0.0,
            measure_gpu_next: false,
            started: Instant::now(),
            last_report: Instant::now(),
        })
    }

    /// §20a.1: the video frame is chosen to match the audio clock. Nothing here
    /// consults a wall-clock timer.
    fn advance_video(&mut self) {
        let position = self.clock.clock().position_secs();
        let target_frame = (position * FPS) as u64;

        match self.last_video_frame {
            Some(last) if target_frame == last => {
                // Display refresh outran the video frame rate. Presenting the
                // same frame again is correct, not a drop.
                self.repeated_frames += 1;
                return;
            }
            Some(last) if target_frame > last + 1 => {
                // §47a.4: we are behind. Skip straight to the current frame
                // rather than decoding the ones we missed.
                self.dropped_frames += target_frame - last - 1;
            }
            _ => {}
        }

        self.last_video_frame = Some(target_frame);
        self.presented_frames += 1;

        let t0 = Instant::now();
        let frame = self.source.frame(target_frame);
        let generate = t0.elapsed();

        let upload = if self.upload_each_frame {
            let t1 = Instant::now();
            self.compositor.upload_frame(frame);
            Some(t1.elapsed())
        } else {
            None
        };

        self.gen_ms.push(generate.as_secs_f64() * 1000.0);
        if let Some(upload) = upload {
            self.upload_ms.push(upload.as_secs_f64() * 1000.0);
        }

        // The presented video time vs. the audio clock (§20a.5).
        let video_time = target_frame as f64 / FPS;
        self.drift_ms.push((video_time - position) * 1000.0);
    }

    fn composite(&mut self) {
        let t = self.started.elapsed().as_secs_f32();
        let params = compositor::CompositeParams {
            // Drift the overlay so the composite cannot be optimized away and
            // so motion is visible on the second layer too.
            overlay_offset: [
                0.28 + 0.10 * (t * 0.6).sin(),
                0.24 + 0.08 * (t * 0.45).cos(),
            ],
            overlay_scale: [0.44, 0.44],
            overlay_opacity: 0.85,
            expand_limited_range: self.expand_limited_range,
        };

        let t0 = Instant::now();
        self.compositor.composite(params);
        self.submit_ms.push(t0.elapsed().as_secs_f64() * 1000.0);

        if self.measure_gpu_next {
            let t1 = Instant::now();
            self.compositor.wait_for_gpu();
            self.gpu_wait_ms = t1.elapsed().as_secs_f64() * 1000.0;
            self.measure_gpu_next = false;
        }
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let frame_start = Instant::now();
        self.ui_frame_ms
            .push(self.last_ui_frame.elapsed().as_secs_f64() * 1000.0);
        self.last_ui_frame = frame_start;

        ctx.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                let playing = self.clock.clock().is_playing();
                self.clock.clock().set_playing(!playing);
            }
            if i.key_pressed(egui::Key::U) {
                self.upload_each_frame = !self.upload_each_frame;
            }
            if i.key_pressed(egui::Key::R) {
                self.clock.reset();
                self.last_video_frame = None;
                self.dropped_frames = 0;
                self.repeated_frames = 0;
                self.presented_frames = 0;
            }
        });

        self.clock.tick();
        let playing = self.clock.clock().is_playing();

        if playing {
            self.advance_video();
        } else if self.last_video_frame.is_none() {
            // Paused at start: still show frame 0 (§17 "paused frame = quality").
            self.advance_video();
        }

        self.composite();

        self.draw_hud(ui);
        self.draw_preview(ui);
        self.report(playing);

        // §81: repaint only while playing. Paused, eframe idles at ~0 fps.
        if playing {
            ctx.request_repaint();
        }
    }
}

impl SpikeApp {
    /// Periodic stdout summary, so the run produces a record for the ADR
    /// instead of a number someone had to read off the screen.
    fn report(&mut self, playing: bool) {
        if !playing || self.last_report.elapsed().as_secs_f64() < 2.0 {
            return;
        }
        self.last_report = Instant::now();
        println!(
            "{:>6.1} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>6} {:>6.1}",
            self.started.elapsed().as_secs_f64(),
            self.ui_frame_ms.avg(),
            self.ui_frame_ms.p99(),
            self.ui_frame_ms.worst(),
            self.gen_ms.avg(),
            self.upload_ms.avg(),
            self.submit_ms.avg(),
            self.dropped_frames,
            self.drift_ms.avg(),
        );
    }

    fn draw_preview(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(18)))
            .show(ui, |ui| {
                let available = ui.available_size();
                let aspect = COMPOSITE_W as f32 / COMPOSITE_H as f32;

                let mut size = egui::vec2(available.x, available.x / aspect);
                if size.y > available.y {
                    size = egui::vec2(available.y * aspect, available.y);
                }

                let (rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
                let image_rect = egui::Rect::from_center_size(rect.center(), size);

                // THE point of the spike: the compositor's output texture is
                // painted directly by egui. No copy, no overlay window (§4.1).
                ui.painter().image(
                    self.compositor.texture_id,
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Panels overlapping the preview is exactly what the Tauri
                // approach could not do. Demonstrate it.
                egui::Window::new("Overlaps the preview")
                    .default_pos(image_rect.center())
                    .collapsible(true)
                    .show(&ctx, |ui| {
                        ui.label("This window is drawn over the video texture.");
                        ui.label("Under a native child window it could not be.");
                    });
            });
    }

    fn draw_hud(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("hud")
            .default_size(380.0)
            .show(ui, |ui| {
                let playing = self.clock.clock().is_playing();

                ui.heading("Frame transport spike");
                ui.label(egui::RichText::new("Milestone 0 · §82").weak());
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button(if playing { "⏸ Pause" } else { "▶ Play" }).clicked() {
                        self.clock.clock().set_playing(!playing);
                    }
                    if ui.button("⏮ Reset").clicked() {
                        self.clock.reset();
                        self.last_video_frame = None;
                        self.dropped_frames = 0;
                        self.repeated_frames = 0;
                        self.presented_frames = 0;
                    }
                    if ui.button("Measure GPU").clicked() {
                        self.measure_gpu_next = true;
                    }
                });

                ui.checkbox(
                    &mut self.upload_each_frame,
                    "Upload frame every frame (software path)",
                );
                ui.checkbox(
                    &mut self.expand_limited_range,
                    "Expand limited range 16-235 → 0-255 (§21a)",
                );

                ui.separator();
                ui.strong("Clock");
                ui.monospace(format!(
                    "audio position   {:>8.3} s",
                    self.clock.clock().position_secs()
                ));
                ui.monospace(format!(
                    "video frame      {:>8}",
                    self.last_video_frame.unwrap_or(0)
                ));
                let drift = self.drift_ms.avg();
                let drift_color = if drift.abs() > 40.0 {
                    egui::Color32::from_rgb(230, 80, 80)
                } else {
                    egui::Color32::from_rgb(120, 200, 120)
                };
                ui.colored_label(
                    drift_color,
                    egui::RichText::new(format!("A/V drift        {drift:>8.1} ms  (limit ±40)"))
                        .monospace(),
                );

                ui.separator();
                ui.strong("Timing (avg / p99 / worst, ms)");
                let row = |ui: &mut egui::Ui, name: &str, r: &Rolling| {
                    ui.monospace(format!(
                        "{name:<10} {:>6.2} {:>6.2} {:>6.2}",
                        r.avg(),
                        r.p99(),
                        r.worst()
                    ));
                };
                row(ui, "ui frame", &self.ui_frame_ms);
                row(ui, "gen", &self.gen_ms);
                row(ui, "upload", &self.upload_ms);
                row(ui, "submit", &self.submit_ms);
                ui.monospace(format!("gpu wait   {:>6.2}", self.gpu_wait_ms));

                let fps = if self.ui_frame_ms.avg() > 0.0 {
                    1000.0 / self.ui_frame_ms.avg()
                } else {
                    0.0
                };
                ui.monospace(format!("ui fps     {fps:>6.1}"));

                ui.separator();
                ui.strong("Frames");
                ui.monospace(format!("presented  {:>8}", self.presented_frames));
                ui.monospace(format!("dropped    {:>8}", self.dropped_frames));
                ui.monospace(format!("repeated   {:>8}", self.repeated_frames));
                ui.monospace(format!(
                    "bytes/frame {:>7.2} MB",
                    self.source.frame_bytes() as f64 / (1024.0 * 1024.0)
                ));
                if self.upload_each_frame {
                    let mb_s = self.source.frame_bytes() as f64 * FPS / (1024.0 * 1024.0);
                    ui.monospace(format!("upload rate {:>7.1} MB/s", mb_s));
                }
                ui.monospace(format!(
                    "silent cbs {:>8}",
                    self.clock.clock().silent_callbacks()
                ));

                ui.separator();
                ui.collapsing("Text quality sample (§4.1)", |ui| {
                    ui.label(
                        "Judge this text. If it is not acceptable for the product, \
                         the UI shell changes to Slint or GPUI — cheaper now than at \
                         Milestone 8.",
                    );
                    ui.add_space(4.0);
                    for size in [11.0, 13.0, 16.0, 22.0] {
                        ui.label(
                            egui::RichText::new(format!(
                                "{size:.0}px — Timeline 00:01:23.456 · Handbrake, Wg jj il1I0O"
                            ))
                            .size(size),
                        );
                    }
                    ui.monospace("monospace 0123456789 il1I O0 .,:;");
                });

                ui.separator();
                ui.label(egui::RichText::new("Space play/pause · U upload · R reset").weak());
            });
    }
}
