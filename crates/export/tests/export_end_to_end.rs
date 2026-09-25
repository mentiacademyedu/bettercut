//! Export, end to end, against real media (§60 criterion 10).
//!
//! Builds a project the way the editor does, exports it, and decodes the result
//! back. This is the criterion the whole MVP has been working towards, so the
//! assertions are about what a user would notice: the file exists, it is the
//! right length, it has the right number of frames, the picture is the picture
//! they edited, and the effects they applied are in it.
//!
//! Skips itself when no GPU adapter is available — export needs one, and §52.1's
//! low-end target and any headless box may have none.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_export::{ExportProgress, ExportSettings, VideoCodec, codec_is_available, export};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{FfmpegDecoder, FfmpegProber, MediaDecoder, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::{
    AnimatedParameter, AudioClip, Interpolation, Keyframe, Resolution, SourceRange, TimelineRange,
    Transition, TransitionKind, VideoClip,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

/// Write a short video whose picture changes a great deal frame to frame.
///
/// The committed fixture is a near-static test pattern: its content signature
/// moves by 0.7% across two seconds, so "did the picture stop?" cannot be
/// asked of it — a freeze that did nothing would pass. This writes two seconds
/// of a white bar sweeping across black instead, where consecutive frames
/// differ enormously and a held frame is unmistakable.
fn moving_source(path: &Path) {
    use bettercut_media::{ExportFormat, VideoWriter};

    let (width, height) = (320_u32, 180_u32);
    let mut writer = VideoWriter::create(
        path,
        ExportFormat {
            transparent: false,
            prores: false,
            width,
            height,
            frame_rate: FrameRate::FPS_30,
            codec: VideoCodec::H264,
            bitrate: Some(2_000_000),
            rate_control: bettercut_export::RateControl::Variable,
            channels: 0,
            audio_bitrate: None,
            threads: 2,
        },
    )
    .expect("open the generated source");

    let frames = 60;
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for frame in 0..frames {
        rgba.fill(0);
        // A bar a fifth of the width, sweeping left to right.
        let bar = width / 5;
        let left = (frame * (width - bar)) / (frames - 1);
        for y in 0..height {
            for x in left..left + bar {
                let at = ((y * width + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&[235, 235, 235, 255]);
            }
        }
        writer.push_frame(&rgba).expect("write a frame");
    }
    writer.finish().expect("finish the generated source");
}

/// A scratch output that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        Self::with_extension(name, "mp4")
    }
    fn with_extension(name: &str, extension: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("bettercut-export-e2e-{name}.{extension}"));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The fixture on V1 and A1 from timeline zero, as dragging it in would do.
fn project_with_fixture() -> Project {
    let mut project = Project::new("Export");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe the fixture");
    let duration = asset.duration;
    let media = project.add_media(asset);

    let source = SourceRange::new(MediaTime::ZERO, duration).expect("non-empty");
    let sequence = project.active_mut().expect("sequence");
    sequence.video_tracks[0]
        .insert(VideoClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");
    sequence.audio_tracks[0]
        .insert(AudioClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");
    project
}

/// The first second of whatever the project holds.
fn one_second() -> TimelineRange {
    TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(1)).expect("non-empty")
}

fn settings(path: &Path, range: TimelineRange) -> ExportSettings {
    ExportSettings {
        transparent: false,
        prores: false,
        path: path.to_path_buf(),
        resolution: Resolution::new(640, 360),
        // The fixture is 29.97, and exporting at its own rate is the ordinary
        // case; `a_different_frame_rate_retimes_the_output` covers the other.
        frame_rate: FrameRate::NTSC_29_97,
        codec: VideoCodec::H264,
        bitrate: None,
        rate_control: bettercut_export::RateControl::Variable,
        range,
        threads: 2,
        sound_only: false,
        picture_only: false,
        gif: false,
        image_sequence: false,
        loudness_target: None,
        audio_bitrate: None,
    }
}

/// Export needs a GPU. Nothing here can run without one, so the whole file
/// checks once and skips politely.
fn gpu_available() -> bool {
    let instance = bettercut_renderer::wgpu::Instance::new(
        bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    pollster::block_on(
        instance.request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok()
}

macro_rules! gpu_or_skip {
    () => {
        if !gpu_available() {
            eprintln!("no GPU adapter; skipping");
            return;
        }
    };
}

/// Serialises the tests that open a real encoder.
///
/// Opening a hardware encoder initialises a vendor runtime — Media Foundation,
/// the AMD or NVIDIA driver — and doing many at once wedges them. The media
/// crate holds a lock around it for the same reason, but that lock is
/// per-process and cargo runs every test binary as its own process, so the
/// binaries still overlap each other.
///
/// This brings each binary down to one encoder open at a time. A handful of
/// binaries at once is fine; twenty is what hung the suite indefinitely.
static ENCODER: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the lock for the rest of the current test.
fn encoder_guard() -> std::sync::MutexGuard<'static, ()> {
    // A panic in another test says nothing about this one, and a poisoned lock
    // would turn one failure into every failure.
    ENCODER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn run(project: &Project, settings: &ExportSettings) -> bettercut_export::ExportSummary {
    let sequence = project.active().expect("sequence");
    let mut seen = Vec::new();
    let summary = export(
        project,
        sequence,
        settings,
        &mut |progress: ExportProgress| seen.push(progress.frames_done),
        &NeverCancelled,
    )
    .expect("export");

    // Progress must actually be reported, and monotonically: a bar that jumps
    // backwards is worse than no bar.
    assert!(!seen.is_empty(), "no progress was reported");
    assert!(
        seen.windows(2).all(|pair| pair[1] > pair[0]),
        "progress went backwards: {seen:?}"
    );
    assert_eq!(
        seen.last().copied(),
        Some(summary.frames),
        "the last progress report did not match the frames written"
    );
    summary
}

/// §60 criterion 10, in one test.
/// The sequence's marks become the file's chapters, cut to the exported
/// stretch and counted from its first frame. Chapters are whole seconds, as
/// the report's are, so the marks sit on whole seconds.
#[test]
fn marks_become_the_files_chapters() {
    let _guard = encoder_guard();
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let mut project = project_with_fixture();
    {
        let sequence = project.active_mut().unwrap();
        let mut middle = bettercut_timeline::Marker::at(TimelineTime::from_seconds(1));
        middle.label = "Middle".to_owned();
        sequence.markers.push(middle);
    }
    let scratch = Scratch::new("chapters");
    let two_seconds =
        TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(2)).unwrap();
    let settings = settings(scratch.path(), two_seconds);
    let _summary = run(&project, &settings);

    let chapters = bettercut_media::probe_chapters(scratch.path()).expect("probe");
    let titles: Vec<&str> = chapters.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, vec!["Intro", "Middle"], "{chapters:?}");
    assert_eq!(chapters[0].start, bettercut_foundation::MediaTime::ZERO);
    assert_eq!(
        chapters[0].end,
        bettercut_foundation::MediaTime::from_seconds(1)
    );
    assert_eq!(
        chapters[1].start,
        bettercut_foundation::MediaTime::from_seconds(1)
    );
    assert!(chapters[1].end > chapters[1].start, "{chapters:?}");
}

/// The chapter list itself: cut to the stretch, re-based to its start, and
/// empty without a mark inside it.
#[test]
fn chapter_marks_are_cut_to_the_stretch_and_rebased() {
    let mut project = project_with_fixture();
    let sequence = project.active_mut().unwrap();
    assert!(bettercut_export::chapter_marks(sequence, one_second()).is_empty());

    sequence
        .markers
        .push(bettercut_timeline::Marker::at(TimelineTime::from_seconds(
            1,
        )));
    let around = TimelineRange::new(
        TimelineTime::from_millis(500),
        TimelineTime::from_millis(1_500),
    )
    .unwrap();
    let marks = bettercut_export::chapter_marks(sequence, around);
    assert_eq!(marks.len(), 2, "{marks:?}");
    assert_eq!(marks[0].title, "Intro");
    assert_eq!(marks[0].start, bettercut_foundation::MediaTime::ZERO);
    assert_eq!(
        marks[0].end,
        bettercut_foundation::MediaTime::from_millis(500)
    );
    assert_eq!(
        marks[1].start,
        bettercut_foundation::MediaTime::from_millis(500)
    );
    assert_eq!(
        marks[1].end,
        bettercut_foundation::MediaTime::from_millis(1_000)
    );
}

#[test]
fn a_project_exports_to_a_playable_file() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("basic");
    let project = project_with_fixture();

    let summary = run(&project, &settings(scratch.path(), one_second()));
    eprintln!(
        "encoded {} frames with {} (hardware: {})",
        summary.frames, summary.encoder, summary.hardware
    );

    let asset = FfmpegProber
        .probe(scratch.path())
        .expect("probe the export");
    assert_eq!((asset.width, asset.height), (640, 360));
    assert!(asset.video_codec.is_some());
    assert!(asset.audio_codec.is_some(), "the audio track is missing");

    // One second at 29.97 is 30 frames.
    assert_eq!(summary.frames, 30);
    let seconds = asset.duration.ticks() as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    assert!(
        (0.9..=1.2).contains(&seconds),
        "expected about a second, got {seconds:.3}"
    );
}

/// Every frame the exporter counted has to be in the file. This is the check
/// that caught B-frame reordering silently dropping one.
#[test]
fn every_counted_frame_reaches_the_file() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("frames");
    let project = project_with_fixture();

    let summary = run(&project, &settings(scratch.path(), one_second()));

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open the export");

    let mut decoded = 0_u64;
    while decoder
        .decode_frame(&NeverCancelled)
        .expect("decode")
        .is_some()
    {
        decoded += 1;
        assert!(decoded <= summary.frames * 2, "the file will not end");
    }

    assert_eq!(decoded, summary.frames);
}

/// The export has to contain the *edit*, not the source. A clip at half opacity
/// over black must come out darker than the same clip at full — otherwise the
/// exporter is copying the file rather than rendering the timeline.
#[test]
fn the_export_contains_the_edit_rather_than_the_source() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let project = project_with_fixture();

    let bright = Scratch::new("bright");
    run(&project, &settings(bright.path(), one_second()));

    let mut dimmed = project.clone();
    {
        let sequence = dimmed.active_mut().expect("sequence");
        let id = sequence.video_tracks[0].clips()[0].id;
        sequence.video_tracks[0].get_mut(id).expect("clip").opacity = 0.25;
    }
    let dark = Scratch::new("dark");
    run(&dimmed, &settings(dark.path(), one_second()));

    let full = mean_luma(bright.path());
    let quarter = mean_luma(dark.path());
    assert!(
        quarter < full * 0.6,
        "opacity did not reach the export: {full:.1} at full, {quarter:.1} at a quarter"
    );
}

/// §24: an animated parameter has to export as the animation, not as a
/// constant. A fade that renders in the preview and exports flat is the exact
/// failure §46 exists to prevent.
#[test]
fn an_animated_fade_exports_as_a_fade() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("fade");
    let mut project = project_with_fixture();
    {
        let sequence = project.active_mut().expect("sequence");
        let id = sequence.video_tracks[0].clips()[0].id;
        let clip = sequence.video_tracks[0].get_mut(id).expect("clip");
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(MediaTime::ZERO, 1.0, Interpolation::Linear),
        );
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(MediaTime::from_seconds(1), 0.0, Interpolation::Linear),
        );
    }

    run(&project, &settings(scratch.path(), one_second()));

    // Decode and compare the first frames against the last: the picture should
    // be fading towards black over the second.
    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");

    let mut lumas = Vec::new();
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        lumas.push(frame_luma(&frame));
    }

    assert!(lumas.len() >= 20, "too few frames to judge a fade");
    let first = lumas[..5].iter().sum::<f64>() / 5.0;
    let last = lumas[lumas.len() - 5..].iter().sum::<f64>() / 5.0;
    assert!(
        last < first * 0.5,
        "the clip did not fade: {first:.1} at the start, {last:.1} at the end"
    );
}

/// §25's flash, all the way to the file.
///
/// The only layer in the editor with no source file behind it: the white is
/// generated rather than decoded, and the export resolves it through its own
/// arm. Everything else about the flash is covered without a GPU — this is the
/// one part where "the export builds the layer too" is a separate claim from
/// "`layer_requests` asked for it".
#[test]
fn a_flash_exports_as_a_flash() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("flash");

    let mut project = project_with_fixture();
    {
        let sequence = project.active_mut().expect("sequence");
        // Cut the one clip in half and flash across the join, so both sides
        // have the same footage and any brightening is the transition.
        let id = sequence.video_tracks[0].clips()[0].id;
        let whole = sequence.video_tracks[0].get(id).expect("clip").source;
        let media = sequence.video_tracks[0].get(id).expect("clip").media_id;
        let half = MediaTime::from_ticks(whole.duration().ticks() / 2);

        let first = sequence.video_tracks[0].get_mut(id).expect("clip");
        first.source = SourceRange::new(whole.start, half).expect("non-empty");
        first.timeline =
            TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_ticks(half.ticks()))
                .expect("non-empty");
        first.transition_out = Some(Transition::new(
            TransitionKind::Flash,
            TimelineTime::from_millis(600),
        ));

        let second = VideoClip::new(
            media,
            TimelineTime::from_ticks(half.ticks()),
            SourceRange::new(half, whole.end).expect("non-empty"),
        )
        .expect("valid");
        sequence.video_tracks[0].insert(second).expect("no overlap");
    }

    let cut = TimelineTime::from_ticks(
        project.active().expect("sequence").video_tracks[0].clips()[1]
            .timeline
            .start
            .ticks(),
    );
    let range = TimelineRange::new(
        TimelineTime::from_ticks(cut.ticks() - TimelineTime::from_millis(300).ticks()),
        TimelineTime::from_ticks(cut.ticks() + TimelineTime::from_millis(300).ticks()),
    )
    .expect("non-empty");

    run(&project, &settings(scratch.path(), range));

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");

    let mut lumas = Vec::new();
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        lumas.push(frame_luma(&frame));
    }
    assert!(lumas.len() >= 10, "too few frames to judge a flash");

    // The window is centred on the cut, so the brightest frame belongs in the
    // middle of it and has to be close to white.
    let (brightest_at, brightest) = lumas
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .expect("frames");
    let edges = (lumas[0] + lumas[lumas.len() - 1]) / 2.0;

    assert!(
        *brightest > 240.0,
        "the flash never reached white: peak {brightest:.1}"
    );
    assert!(
        *brightest > edges + 40.0,
        "no brighter in the middle than at the edges: {brightest:.1} against {edges:.1}"
    );
    let middle = lumas.len() / 2;
    assert!(
        brightest_at.abs_diff(middle) <= lumas.len() / 4,
        "the flash peaked at frame {brightest_at} of {}, not near the cut",
        lumas.len()
    );
}

/// One edit with everything on it at once.
///
/// Every feature here has its own test, and each of those puts one thing on an
/// otherwise plain project. What none of them covers is the combination: a cut
/// carrying a flash while the shot either side of it is animating, a title over
/// the top of both, a background showing through where the picture has been
/// scaled away, a grade on one clip and a mask on the other.
///
/// This is the shape of an actual edit, and the failures worth finding now are
/// interactions rather than features — an effect that works alone and blanks
/// the frame beside another one.
#[test]
fn a_whole_edit_exports() {
    use bettercut_timeline::{
        BlendMode, ClipMotion, Mask, MaskShape, Motion, MotionKind, TextAnimation, TextClip,
        Transform, Vec2,
    };

    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("whole-edit");

    let mut project = project_with_fixture();
    {
        let sequence = project.active_mut().expect("sequence");

        // A background that can only show where the picture does not cover.
        sequence.master.background = [0.1, 0.2, 0.5];

        let first_id = sequence.video_tracks[0].clips()[0].id;
        let whole = sequence.video_tracks[0].get(first_id).expect("clip").source;
        let media = sequence.video_tracks[0]
            .get(first_id)
            .expect("clip")
            .media_id;
        let half = MediaTime::from_ticks(whole.duration().ticks() / 2);

        // First half: scaled in so the background shows, graded, animating in,
        // and flashing at the cut.
        let first = sequence.video_tracks[0].get_mut(first_id).expect("clip");
        first.source = SourceRange::new(whole.start, half).expect("non-empty");
        first.timeline =
            TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_ticks(half.ticks()))
                .expect("non-empty");
        first.transform = Transform {
            scale: Vec2::new(0.8, 0.8),
            ..Transform::default()
        };
        // Motion blur *and* a backdrop, together. Each is drawn from the same
        // layer the other rewrites, and getting that wrong duplicates the shot
        // behind itself — which is a picture, not an error, so only an actual
        // frame catches it.
        first.motion_blur = true;
        first.backdrop = bettercut_timeline::Backdrop::Blur;
        first.color.saturation = 1.4;
        first.motion = ClipMotion {
            intro: Some(Motion::new(
                MotionKind::Fade,
                TimelineTime::from_millis(300),
            )),
            outro: None,
        };
        first.transition_out = Some(Transition::new(
            TransitionKind::Flash,
            TimelineTime::from_millis(400),
        ));

        // Second half: masked and blended, animating out.
        let mut second = VideoClip::new(
            media,
            TimelineTime::from_ticks(half.ticks()),
            SourceRange::new(half, whole.end).expect("non-empty"),
        )
        .expect("valid");
        second.mask = Some(Mask {
            shape: MaskShape::Ellipse,
            ..Mask::default()
        });
        second.blend = BlendMode::Screen;
        second.motion = ClipMotion {
            intro: None,
            outro: Some(Motion::new(
                MotionKind::SlideLeft,
                TimelineTime::from_millis(300),
            )),
        };
        sequence.video_tracks[0].insert(second).expect("no overlap");

        // A title over the lot, with its own entrance.
        // Shorter than the exported range on purpose: the tail check below
        // has to see the *picture*, and a title across the whole film supplies
        // enough variation on its own to hide the shot vanishing behind it.
        let mut title = TextClip::with_duration(
            "Everything",
            TimelineTime::from_millis(600),
            TimelineTime::from_millis(300),
        )
        .expect("valid title");
        title.animation = TextAnimation {
            scroll: None,
            intro: Some(Motion::new(MotionKind::Pop, TimelineTime::from_millis(300))),
            outro: None,
            looping: None,
        };
        sequence.text_tracks[0].insert(title).expect("empty track");
    }

    // Straddling the cut at 1 s, because that is where the interactions are:
    // the flash, the end of one clip and the start of the other. The fixture
    // is two seconds long, so an export of its first second would have shown
    // the first clip alone and proved very little.
    let across_the_cut = TimelineRange::new(
        TimelineTime::from_millis(600),
        TimelineTime::from_millis(1400),
    )
    .expect("non-empty");
    let summary = run(&project, &settings(scratch.path(), across_the_cut));
    assert!(summary.frames >= 20, "a frame went missing under load");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    assert_eq!((asset.width, asset.height), (640, 360));
    assert!(asset.video_codec.is_some());

    // Nothing blanked: every frame has to carry some light, and none of them
    // may be the flat background either — that would mean the picture stopped
    // being drawn somewhere in the stack.
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");
    let mut lumas = Vec::new();
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        lumas.push((frame_luma(&frame), frame_spread(&frame)));
    }

    assert!(
        lumas.len() >= 18,
        "too few frames came back: {}",
        lumas.len()
    );
    let brightest = lumas.iter().map(|frame| frame.0).fold(0.0_f64, f64::max);
    assert!(
        brightest > 200.0,
        "the flash never reached white with everything else on: {brightest:.1}"
    );

    // Picture, not just colour. A mean alone cannot tell a shot from a flat
    // background — the background here is a mid blue, which passes any
    // "not black" check while the film has quietly stopped being drawn. Spread
    // can: a photograph varies across the frame and a flat fill does not.
    //
    // Measured over the last few frames, which is where the second clip is:
    // the flash's own peak is legitimately flat white, so "every frame varies"
    // would be false for the right reason.
    let tail = &lumas[lumas.len() - 5..];
    let flattest = tail.iter().map(|frame| frame.1).fold(f64::MAX, f64::min);
    assert!(
        flattest > 20.0,
        "the end of the film is a flat colour — something stopped drawing: {tail:?}"
    );
}

/// A mask has to survive the *export's* own layer building.
///
/// §46 is enforced by the golden frames, but only from the layer inwards: they
/// hand both configurations the same `Layer` values and compare pixels. The
/// step before that — turning a `LayerRequest` into a layer — is written out
/// once in the preview and once in the export, and nothing compared the two.
/// Dropping the mask from the export alone passed every test in the workspace.
///
/// So this checks the one thing a mask is for: the picture stops at its edge.
#[test]
fn a_mask_reaches_the_exported_file() {
    use bettercut_timeline::{Mask, MaskShape};

    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("masked");

    let mut project = project_with_fixture();
    {
        let sequence = project.active_mut().expect("sequence");
        // A background nothing in the footage looks like, so "masked away" is
        // a colour rather than an absence.
        sequence.master.background = [1.0, 0.0, 0.0];
        let id = sequence.video_tracks[0].clips()[0].id;
        sequence.video_tracks[0].get_mut(id).expect("clip").mask = Some(Mask {
            shape: MaskShape::Ellipse,
            size: [0.25, 0.25],
            ..Mask::default()
        });
    }

    run(&project, &settings(scratch.path(), one_second()));

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");
    let frame = decoder
        .decode_frame(&NeverCancelled)
        .expect("decode")
        .expect("a frame");

    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("expected a system-memory frame");
    };
    let at = |x: u32, y: u32| {
        let i = (y * stride + x * 4) as usize;
        [data[i], data[i + 1], data[i + 2]]
    };

    // Outside a quarter-size ellipse in the middle: the background, which is
    // red. Inside it: the shot, which is not.
    let corner = at(4, 4);
    assert!(
        corner[0] > 180 && corner[1] < 80 && corner[2] < 80,
        "the corner is not the background — the mask never reached the export: {corner:?}"
    );

    let middle = at(frame.width / 2, frame.height / 2);
    assert!(
        !(middle[0] > 180 && middle[1] < 80 && middle[2] < 80),
        "the middle is background too — the mask removed everything: {middle:?}"
    );
}

/// §14: export reads the original, never the proxy. A project whose proxy is a
/// different resolution must still export at the sequence's resolution — and
/// more importantly, from the full-quality source.
#[test]
fn export_uses_the_sequence_resolution_not_the_sources() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("resolution");
    let project = project_with_fixture();

    let mut settings = settings(scratch.path(), one_second());
    settings.resolution = Resolution::new(320, 180);
    run(&project, &settings);

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    assert_eq!((asset.width, asset.height), (320, 180));
}

/// A cancelled export leaves nothing behind. A partial MP4 sitting where the
/// user asked for their video is worse than no file: it looks finished.
#[test]
fn cancelling_removes_the_partial_file() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("cancelled");
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");

    struct StopAfter(std::sync::atomic::AtomicU32);
    impl bettercut_media::CancellationToken for StopAfter {
        fn is_cancelled(&self) -> bool {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 3
        }
    }

    let result = export(
        &project,
        sequence,
        &settings(scratch.path(), one_second()),
        &mut |_| {},
        &StopAfter(std::sync::atomic::AtomicU32::new(0)),
    );

    let err = result.expect_err("the export should have been cancelled");
    assert!(err.is_cancellation(), "unexpected failure: {err}");
    assert!(
        !scratch.path().exists(),
        "a cancelled export left a file behind"
    );
}

/// An empty range is refused rather than producing a zero-length file.
#[test]
fn an_empty_range_is_refused() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("empty");
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");

    let empty = TimelineRange {
        start: TimelineTime::from_seconds(1),
        end: TimelineTime::from_seconds(1),
    };
    assert!(
        export(
            &project,
            sequence,
            &settings(scratch.path(), empty),
            &mut |_| {},
            &NeverCancelled,
        )
        .is_err()
    );
}

/// Mean brightness of the whole file, for comparing two exports.
fn mean_luma(path: &Path) -> f64 {
    let asset = FfmpegProber.probe(path).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");

    let mut total = 0.0;
    let mut frames = 0.0;
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        total += frame_luma(&frame);
        frames += 1.0;
    }
    if frames == 0.0 { 0.0 } else { total / frames }
}

/// Mean of the green channel, sampled sparsely — enough to compare exposures
/// without decoding into a full histogram.
/// How much a frame varies across itself, as the range of its sampled luma.
///
/// The companion to [`frame_luma`], which cannot tell a photograph from a flat
/// fill of the same average brightness — and a flat fill is exactly what a
/// frame becomes when something in the stack stops drawing over a background.
fn frame_spread(frame: &bettercut_media::VideoFrame) -> f64 {
    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("expected a system-memory frame");
    };

    let (mut low, mut high) = (f64::MAX, 0.0_f64);
    for y in (0..frame.height).step_by(4) {
        for x in (0..frame.width).step_by(4) {
            let at = (y * stride + x * 4) as usize;
            let value = f64::from(data[at + 1]);
            low = low.min(value);
            high = high.max(value);
        }
    }
    if low > high { 0.0 } else { high - low }
}

fn frame_luma(frame: &bettercut_media::VideoFrame) -> f64 {
    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("expected a system-memory frame");
    };

    let mut total = 0.0;
    let mut count = 0.0;
    for y in (0..frame.height).step_by(4) {
        for x in (0..frame.width).step_by(4) {
            let at = (y * stride + x * 4) as usize;
            total += f64::from(data[at + 1]);
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { total / count }
}

/// Exporting at a rate other than the sequence's re-times the output: the same
/// second of timeline becomes a different number of frames.
#[test]
fn a_different_frame_rate_retimes_the_output() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::new("retimed");
    let project = project_with_fixture();

    let mut settings = settings(scratch.path(), one_second());
    settings.frame_rate = FrameRate::FPS_60;
    let summary = run(&project, &settings);

    assert_eq!(summary.frames, 60, "a second at 60 fps is 60 frames");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let seconds = asset.duration.ticks() as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    assert!(
        (0.9..=1.2).contains(&seconds),
        "re-timing changed the duration: {seconds:.3}"
    );
}

/// An explicit bitrate has to reach the encoder.
///
/// Deliberately *not* asserting proportionality. The bitrate is a ceiling, not
/// a quota: a capped-VBR encoder spends what the picture needs and no more, so
/// a simple fixture at 16 Mb/s does not produce sixteen times the bytes of the
/// same fixture at 1. Measured here at about 1.7x, which is the shape of the
/// real behaviour — what matters is that the number reaches the encoder at all,
/// and a decorative field would show no difference whatsoever.
#[test]
fn an_explicit_bitrate_changes_the_file_size() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let project = project_with_fixture();

    let small = Scratch::new("small-bitrate");
    let mut low = settings(small.path(), one_second());
    low.bitrate = Some(1_000_000);
    run(&project, &low);

    let large = Scratch::new("large-bitrate");
    let mut high = settings(large.path(), one_second());
    high.bitrate = Some(16_000_000);
    run(&project, &high);

    let low_bytes = std::fs::metadata(small.path()).expect("small").len();
    let high_bytes = std::fs::metadata(large.path()).expect("large").len();
    assert!(
        high_bytes > low_bytes * 5 / 4,
        "the bitrate did not reach the encoder: {low_bytes} vs {high_bytes} bytes"
    );
}

/// H.265 when the machine can write it. Skipped rather than failed elsewhere:
/// there is no software fallback by design (§0.1 forbids x265), so a machine
/// without a GPU encoder legitimately cannot run this.
#[test]
fn h265_exports_when_the_machine_can_write_it() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let resolution = Resolution::new(640, 360);
    if !codec_is_available(VideoCodec::H265, resolution, FrameRate::NTSC_29_97) {
        eprintln!("no H.265 encoder on this machine; skipping");
        return;
    }

    let scratch = Scratch::new("h265");
    let project = project_with_fixture();
    let mut settings = settings(scratch.path(), one_second());
    settings.codec = VideoCodec::H265;
    let summary = run(&project, &settings);

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let codec = asset.video_codec.expect("no video stream");
    assert!(
        codec.contains("hevc") || codec.contains("h265"),
        "asked for H.265 and got {codec}"
    );
    assert_eq!(summary.frames, 30);
}

/// A ProRes master: `prores_ks` is in the LGPL build and runs on the CPU, so
/// this runs on every machine.
#[test]
fn prores_exports_a_mov() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let scratch = Scratch::with_extension("prores", "mov");
    let project = project_with_fixture();
    let mut settings = settings(scratch.path(), one_second());
    settings.prores = true;
    let summary = run(&project, &settings);

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let codec = asset.video_codec.expect("no video stream");
    assert!(codec.contains("prores"), "asked for ProRes and got {codec}");
    assert_eq!(summary.frames, 30);
}

/// AV1 when the machine can write it (an RTX 40, Arc or RX 7000); skipped
/// elsewhere, as H.265 is, since no software AV1 encoder is linked.
#[test]
fn av1_exports_when_the_machine_can_write_it() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let resolution = Resolution::new(640, 360);
    if !codec_is_available(VideoCodec::Av1, resolution, FrameRate::NTSC_29_97) {
        eprintln!("no AV1 encoder on this machine; skipping");
        return;
    }

    let scratch = Scratch::new("av1");
    let project = project_with_fixture();
    let mut settings = settings(scratch.path(), one_second());
    settings.codec = VideoCodec::Av1;
    let summary = run(&project, &settings);

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let codec = asset.video_codec.expect("no video stream");
    assert!(codec.contains("av1"), "asked for AV1 and got {codec}");
    assert_eq!(summary.frames, 30);
}

/// H.264 is always available — `libopenh264` is linked in — and asking must not
/// cost a driver probe. The export window asks on the UI thread when it opens,
/// and §74 is explicit that the interface must not stall behind FFmpeg.
#[test]
fn asking_about_a_codec_is_cheap_after_the_first_time() {
    let _encoder = encoder_guard();
    let resolution = Resolution::new(1920, 1080);
    let rate = FrameRate::FPS_30;

    assert!(
        codec_is_available(VideoCodec::H264, resolution, rate),
        "H.264 has a linked software encoder, so it is available everywhere"
    );

    // Warm whatever the first call costs, then measure the second.
    let _ = codec_is_available(VideoCodec::H265, resolution, rate);
    let started = std::time::Instant::now();
    for _ in 0..50 {
        let _ = codec_is_available(VideoCodec::H265, resolution, rate);
        let _ = codec_is_available(VideoCodec::H264, resolution, rate);
    }
    let elapsed = started.elapsed();

    // A single uncached probe is around half a second here. A hundred cached
    // ones must be nowhere near that.
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "100 availability checks took {elapsed:?}; the answer is not being cached"
    );
}

/// CBR pads simple footage to hold the rate; VBR spends less. The same clip at
/// the same bitrate must therefore come out visibly larger under CBR — which is
/// also the check that `rate_control` reaches the encoder at all.
#[test]
fn constant_rate_control_produces_a_larger_file_than_variable() {
    let _encoder = encoder_guard();
    gpu_or_skip!();
    let project = project_with_fixture();

    let vbr_file = Scratch::new("vbr");
    let mut vbr = settings(vbr_file.path(), one_second());
    vbr.bitrate = Some(8_000_000);
    vbr.rate_control = bettercut_export::RateControl::Variable;
    run(&project, &vbr);

    let cbr_file = Scratch::new("cbr");
    let mut cbr = settings(cbr_file.path(), one_second());
    cbr.bitrate = Some(8_000_000);
    cbr.rate_control = bettercut_export::RateControl::Constant;
    run(&project, &cbr);

    let vbr_bytes = std::fs::metadata(vbr_file.path()).expect("vbr").len();
    let cbr_bytes = std::fs::metadata(cbr_file.path()).expect("cbr").len();
    assert!(
        cbr_bytes > vbr_bytes,
        "constant rate control did not reach the encoder: \
         {vbr_bytes} bytes variable, {cbr_bytes} constant"
    );
}

/// Root-mean-square level of every audio sample in a file.
fn audio_rms(path: &Path) -> f64 {
    let asset = FfmpegProber.probe(path).expect("probe the export");
    let mut decoder = FfmpegDecoder::new(1).expect("decoder");
    decoder.open(&asset).expect("open the export");

    let (mut sum, mut count) = (0.0_f64, 0_u64);
    while let Some(buffer) = decoder.decode_audio(&NeverCancelled).expect("decode audio") {
        for plane in &buffer.planes {
            for sample in plane {
                sum += f64::from(*sample) * f64::from(*sample);
                count += 1;
            }
        }
    }
    assert!(count > 0, "the export has no audio at all");
    (sum / count as f64).sqrt()
}

/// §20a.4's master gain reaches the file, not only the speakers.
///
/// The whole-video volume sits in the Inspector beside the whole-video picture
/// controls, which all export — a volume that changed what played and not what
/// was exported would be the one control there that silently did not, and a
/// preview/export mismatch §46 forbids outright.
#[test]
fn the_whole_video_volume_reaches_the_export() {
    gpu_or_skip!();
    let _guard = encoder_guard();

    let level_at = |volume: f32, name: &str| {
        let mut project = project_with_fixture();
        project.active_mut().expect("sequence").master_volume = volume;
        let out = Scratch::new(name);
        run(&project, &settings(out.path(), one_second()));
        audio_rms(out.path())
    };

    let full = level_at(1.0, "volume-full");
    let quarter = level_at(0.25, "volume-quarter");

    assert!(
        full > 0.01,
        "the full-volume export is silent (rms {full:.4})"
    );
    let ratio = quarter / full;
    assert!(
        (0.18..=0.32).contains(&ratio),
        "at 25% volume the export's level was {ratio:.2} of full; the master volume \
         is not reaching the file"
    );
}

/// §46 for the features added since the MVP: a held frame, a fade on the
/// sound, a title with an entrance, and a track's own volume, all in one
/// export — with both the hold and the fade measured in the finished file.
///
/// The picture comes from a generated source (see `moving_source`) because the
/// committed fixture barely changes: an earlier version of this test asserted
/// the hold against that fixture and passed with the freeze removed from the
/// engine, which is no test at all. The sound still comes from the fixture,
/// since the generated file is silent.
#[test]
fn a_hold_a_fade_and_an_animated_title_export() {
    use bettercut_timeline::{Motion, MotionKind, TextAnimation, TextClip};

    let _encoder = encoder_guard();
    gpu_or_skip!();
    let source = Scratch::new("moving-source");
    moving_source(source.path());
    let scratch = Scratch::new("everything");

    // The generated picture on V1, the fixture's sound on A1.
    let mut project = project_with_fixture();
    let moving = FfmpegProber
        .probe(source.path())
        .expect("probe the generated source");
    let moving = project.add_media(moving);
    let sequence = project.active_mut().expect("sequence");

    let existing = sequence.video_tracks[0].clips()[0].id;
    sequence.video_tracks[0].remove(existing).expect("remove");

    let half = TimelineTime::from_millis(500);
    sequence.video_tracks[0]
        .insert(
            VideoClip::new(
                moving,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, MediaTime::from_millis(500)).expect("valid"),
            )
            .expect("valid"),
        )
        .expect("empty track");

    let mut held = VideoClip::new(
        moving,
        half,
        SourceRange::new(MediaTime::from_millis(500), MediaTime::from_millis(540)).expect("valid"),
    )
    .expect("valid");
    held.frozen = true;
    held.timeline = TimelineRange::new(half, TimelineTime::from_seconds(1)).expect("valid");
    sequence.video_tracks[0].insert(held).expect("no overlap");

    // Sound: trimmed to the exported second — a fade sits at the clip's own
    // end, so a two-second clip would fade outside the range and prove
    // nothing — then faded out under the hold, on a track turned down.
    let sound = sequence.audio_tracks[0].clips()[0].id;
    sequence.audio_tracks[0]
        .trim_end(sound, TimelineTime::from_seconds(1), None)
        .expect("trim");
    sequence.audio_tracks[0]
        .get_mut(sound)
        .expect("clip")
        .fade_out = TimelineTime::from_millis(400);
    sequence.audio_tracks[0].gain = 0.7;

    // A title that fades in over the first 300 ms, so it is steady by the time
    // the compared frames are taken.
    let mut title =
        TextClip::with_duration("Held", TimelineTime::ZERO, TimelineTime::from_seconds(1))
            .expect("valid");
    title.animation = TextAnimation {
        scroll: None,
        intro: Some(Motion::new(
            MotionKind::Fade,
            TimelineTime::from_millis(300),
        )),
        outro: None,
        looping: None,
    };
    sequence.text_tracks[0].insert(title).expect("empty track");

    let summary = run(&project, &settings(scratch.path(), one_second()));
    assert_eq!(summary.frames, 30);

    let asset = FfmpegProber
        .probe(scratch.path())
        .expect("probe the export");
    assert!(asset.audio_codec.is_some(), "the sound was lost");

    // The hold: 300 ms inside it against 300 ms of the sweep before it.
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");
    let mut frames = Vec::new();
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        frames.push((frame.timestamp, sample(&frame)));
    }
    let at = |ms: i64| {
        let want = MediaTime::from_millis(ms);
        frames
            .iter()
            .min_by_key(|(t, _)| (t.ticks() - want.ticks()).abs())
            .map(|(_, pixels)| pixels.clone())
            .expect("a frame")
    };
    let holding = difference(&at(650), &at(950));
    let sweeping = difference(&at(100), &at(400));
    assert!(
        sweeping > 20.0,
        "the generated source did not move ({sweeping:.1}), so this proves nothing"
    );
    assert!(
        holding * 5.0 < sweeping,
        "the hold moved: {holding:.1} across 300 ms of hold, against {sweeping:.1} \
         across 300 ms of the sweep"
    );

    // The fade: the last quarter-second is deep inside it, so it is a fraction
    // of the first.
    let (first, last) = audio_rms_ends(scratch.path(), TimelineTime::from_millis(250));
    assert!(
        last * 4.0 < first,
        "the fade did not reach the file: {first:.4} then {last:.4}"
    );
}

/// A thin sample of a frame's pixels, in order, for comparing one picture
/// against another.
///
/// Position matters: an earlier version of this summed the sampled bytes, and
/// a bar sweeping across the picture summed to the same number wherever it was,
/// so a moving picture read as a still one.
fn sample(frame: &bettercut_media::VideoFrame) -> Vec<u8> {
    let bettercut_media::FrameStorage::System { data, .. } = &frame.storage else {
        panic!("expected a system-memory frame");
    };
    data.iter().step_by(101).copied().collect()
}

/// How different two sampled frames are, 0 to 255. Compressed frames of the
/// same picture land near zero rather than exactly on it.
fn difference(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "frames of different sizes");
    let total: u32 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u32::from(x.abs_diff(*y)))
        .sum();
    f64::from(total) / a.len() as f64
}

/// The level of the first and last `edge` of a file's audio, for judging a
/// fade rather than an overall volume.
fn audio_rms_ends(path: &Path, edge: TimelineTime) -> (f64, f64) {
    let asset = FfmpegProber.probe(path).expect("probe the export");
    let mut decoder = FfmpegDecoder::new(1).expect("decoder");
    decoder.open(&asset).expect("open the export");

    let ends_after = asset.duration.ticks() - edge.ticks();
    let (mut early, mut late) = ((0.0_f64, 0_u64), (0.0_f64, 0_u64));
    while let Some(buffer) = decoder.decode_audio(&NeverCancelled).expect("decode audio") {
        let at = buffer.timestamp.ticks();
        let part = if at < edge.ticks() {
            &mut early
        } else if at >= ends_after {
            &mut late
        } else {
            continue;
        };
        for plane in &buffer.planes {
            for sample in plane {
                part.0 += f64::from(*sample) * f64::from(*sample);
                part.1 += 1;
            }
        }
    }
    let rms = |(sum, count): (f64, u64)| {
        if count == 0 {
            0.0
        } else {
            (sum / count as f64).sqrt()
        }
    };
    assert!(early.1 > 0 && late.1 > 0, "the export has no audio at all");
    (rms(early), rms(late))
}
