//! J/K/L shuttle through a real [`Preview`]: the playhead moves at the rate,
//! backwards too, stops at the start, and Space stops a shuttle rather than
//! starting ordinary playback on top of it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::Preview;
use eframe::egui_wgpu::{self, RenderState, wgpu};

fn render_state() -> Option<RenderState> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("shuttle test"),
        ..Default::default()
    }))
    .ok()?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
    Some(RenderState {
        adapter,
        available_adapters: Vec::new(),
        instance,
        device,
        queue,
        target_format: format,
        renderer: std::sync::Arc::new(egui::mutex::RwLock::new(renderer)),
        surface_config: egui_wgpu::SurfaceConfig {
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: None,
        },
    })
}

/// A 20-second timeline. The file does not exist, which the preview treats as
/// a gap (§66) — the shuttle moves the playhead all the same.
fn editor() -> Editor {
    let (mut editor, _events) = Editor::new_project("Shuttle");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/nowhere/clip.mp4",
        MediaTime::from_seconds(20),
    ));
    editor.place_media(media).unwrap();
    editor
}

#[test]
fn a_shuttle_moves_the_playhead_at_its_rate_and_stops_at_the_start() {
    let Some(render_state) = render_state() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut preview = Preview::new(&render_state, 64 * 1024 * 1024, 1).unwrap();
    let mut editor = editor();
    editor.set_playhead(TimelineTime::from_seconds(5));
    preview.seek_to(editor.playhead());
    preview.update(&mut editor);

    preview.set_shuttle(&editor, 4);
    assert_eq!(preview.shuttle_rate(), 4);
    assert!(preview.is_playing(), "a shuttle counts as moving");
    std::thread::sleep(Duration::from_millis(250));
    preview.update(&mut editor);
    // A quarter of a second at 4x is a second; a slow machine only adds.
    let moved = editor.playhead().ticks() - TimelineTime::from_seconds(5).ticks();
    assert!(
        moved >= TimelineTime::from_seconds(1).ticks()
            && moved < TimelineTime::from_seconds(3).ticks(),
        "moved {moved} ticks at 4x in 250 ms"
    );

    // Backwards, fast enough to hit the start and stop there.
    preview.set_shuttle(&editor, -8);
    std::thread::sleep(Duration::from_millis(900));
    preview.update(&mut editor);
    assert_eq!(editor.playhead(), TimelineTime::ZERO);
    assert_eq!(
        preview.shuttle_rate(),
        0,
        "the shuttle did not stop at the start"
    );
    assert!(!preview.is_playing());

    // Space during a shuttle stops it.
    editor.set_playhead(TimelineTime::from_seconds(10));
    preview.seek_to(editor.playhead());
    preview.set_shuttle(&editor, -2);
    preview.set_playing(&editor, false);
    assert_eq!(preview.shuttle_rate(), 0);
    let held = editor.playhead();
    std::thread::sleep(Duration::from_millis(100));
    preview.update(&mut editor);
    assert_eq!(editor.playhead(), held, "a stopped shuttle kept moving");
}

/// With looping on, playing past the out mark goes back to the in mark and
/// keeps playing; with it off, playback runs on past the mark.
#[test]
fn looped_playback_goes_round_the_marked_range() {
    let Some(render_state) = render_state() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut preview = Preview::new(&render_state, 64 * 1024 * 1024, 1).unwrap();
    let mut editor = editor();
    let (mark_in, mark_out) = (
        TimelineTime::from_seconds(2),
        TimelineTime::from_millis(2_300),
    );
    editor.set_mark_in(mark_in).unwrap();
    editor.set_mark_out(mark_out).unwrap();

    preview.set_looping(true);
    editor.set_playhead(mark_in);
    preview.seek_to(mark_in);
    preview.set_playing(&editor, true);
    std::thread::sleep(Duration::from_millis(500));
    preview.update(&mut editor);
    let at = editor.playhead();
    assert!(
        at >= mark_in && at < mark_out,
        "looped playback left the marked range: {}",
        at.format_timecode()
    );
    assert!(preview.is_playing(), "looping stopped playback");

    preview.set_looping(false);
    editor.set_playhead(mark_in);
    preview.seek_to(mark_in);
    std::thread::sleep(Duration::from_millis(500));
    preview.update(&mut editor);
    assert!(
        editor.playhead() >= mark_out,
        "with looping off, playback should run past the out mark"
    );
    preview.set_playing(&editor, false);
}
