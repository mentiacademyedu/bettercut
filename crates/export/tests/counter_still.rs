//! A timer title through the export's own render path: the number in the file
//! is the number for that instant (§46).
//!
//! Skips itself without a GPU.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_export::render_still;
use bettercut_foundation::TimelineTime;
use bettercut_media::NeverCancelled;
use bettercut_project_format::Project;
use bettercut_timeline::{Counter, Resolution, TextClip};

fn gpu_available() -> bool {
    let instance = bettercut_renderer::wgpu::Instance::new(
        bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    pollster::block_on(
        instance.request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok()
}

fn project_with(counter: Option<Counter>) -> Project {
    let mut project = Project::new("Timer");
    let sequence = project.active_mut().unwrap();
    sequence.resolution = Resolution {
        width: 320,
        height: 180,
    };
    let mut title =
        TextClip::with_duration("Timer", TimelineTime::ZERO, TimelineTime::from_seconds(10))
            .unwrap();
    title.counter = counter;
    sequence.text_tracks[0].insert(title).unwrap();
    project
}

fn still_at(project: &Project, seconds: i64) -> Vec<u8> {
    let sequence = project.active().unwrap();
    render_still(
        project,
        sequence,
        TimelineTime::from_seconds(seconds),
        &NeverCancelled,
    )
    .expect("render")
    .1
}

#[test]
fn an_exported_timer_shows_the_number_for_its_instant() {
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let timer = project_with(Some(Counter::countdown(TimelineTime::from_seconds(10))));
    let at_start = still_at(&timer, 0);
    let later = still_at(&timer, 6);
    assert!(
        at_start.iter().any(|b| *b > 128),
        "nothing was drawn for the timer"
    );
    assert_ne!(at_start, later, "the exported number did not change");

    let plain = project_with(None);
    assert_eq!(
        still_at(&plain, 0),
        still_at(&plain, 6),
        "an ordinary title changed on its own"
    );
}
