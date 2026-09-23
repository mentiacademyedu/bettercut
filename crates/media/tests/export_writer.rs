//! Writing a real file (Milestone 6, §0.1, §60 criterion 10).
//!
//! The unit tests around the encoder answer "will one open". This answers the
//! question that actually matters: does the writer produce a file that decodes,
//! at the right size, rate, and length — and does the picture survive the round
//! trip through YUV 4:2:0 and back.
//!
//! Everything is written to a temporary directory and removed afterwards. §74:
//! never modify source media — and never leave litter in the user's tree.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use bettercut_media::{
    CancellationToken, ExportFormat, FfmpegDecoder, FfmpegProber, MediaDecoder, MediaProber,
    NeverCancelled, VideoCodec, VideoWriter,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FRAMES: usize = 48;

/// A scratch file that removes itself, so a failing test does not leave a
/// half-written MP4 behind for the next run to trip over.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("bettercut-export-test-{name}.mp4"));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn format(channels: usize) -> ExportFormat {
    ExportFormat {
        transparent: false,
        width: WIDTH,
        height: HEIGHT,
        frame_rate: bettercut_foundation::FrameRate::FILM_24,
        codec: VideoCodec::H264,
        bitrate: None,
        rate_control: bettercut_media::RateControl::Variable,
        channels,
        audio_bitrate: None,
        threads: 2,
    }
}

/// Frame `index` of a moving picture: a bright block that slides left to right
/// over a fixed background.
///
/// Moving matters. A constant picture encodes to almost nothing and would still
/// produce a valid file if the writer sent the same frame every time, so the
/// motion is what makes "the video is 48 distinct frames" checkable.
/// Chapters given to the writer come back from the file, titles and times.
#[test]
fn chapters_are_written_into_the_file_and_read_back() {
    let scratch = Scratch::new("chapters");
    let marks = vec![
        bettercut_media::ChapterMark {
            title: "Intro".to_owned(),
            start: bettercut_foundation::MediaTime::ZERO,
            end: bettercut_foundation::MediaTime::from_millis(500),
        },
        bettercut_media::ChapterMark {
            title: "The rest".to_owned(),
            start: bettercut_foundation::MediaTime::from_millis(500),
            end: bettercut_foundation::MediaTime::from_seconds(2),
        },
    ];
    let mut writer =
        VideoWriter::create_with_chapters(scratch.path(), format(0), &marks).expect("writer");
    for index in 0..FRAMES {
        writer.push_frame(&frame(index)).expect("frame");
    }
    writer.finish().expect("finish");

    let back = bettercut_media::probe_chapters(scratch.path()).expect("probe");
    assert_eq!(back, marks, "the chapters did not survive the file");
}

/// A file written without chapters has none, and says so plainly.
#[test]
fn a_file_without_chapters_has_none() {
    let scratch = Scratch::new("no-chapters");
    let mut writer = VideoWriter::create(scratch.path(), format(0)).expect("writer");
    for index in 0..4 {
        writer.push_frame(&frame(index)).expect("frame");
    }
    writer.finish().expect("finish");
    assert!(
        bettercut_media::probe_chapters(scratch.path())
            .expect("probe")
            .is_empty()
    );
}

fn frame(index: usize) -> Vec<u8> {
    let mut data = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
    let block = WIDTH / 6;
    let left = (index as u32 * (WIDTH - block) / FRAMES as u32).min(WIDTH - block);

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let at = ((y * WIDTH + x) * 4) as usize;
            let inside = x >= left && x < left + block && y > HEIGHT / 4 && y < HEIGHT * 3 / 4;
            let pixel = if inside {
                [235, 235, 235, 255]
            } else {
                [24, 40, 72, 255]
            };
            data[at..at + 4].copy_from_slice(&pixel);
        }
    }
    data
}

/// A second of a quiet sine, so the audio track has something in it that is not
/// silence — a silent track encodes and decodes even when the mixing is wrong.
fn tone(samples: usize, channels: usize) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|channel| {
            (0..samples)
                .map(|i| {
                    let phase = i as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU;
                    // Channels differ, so a mix-up between them is visible.
                    phase.sin() * if channel == 0 { 0.25 } else { 0.15 }
                })
                .collect()
        })
        .collect()
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

/// The whole point: a file that exists, decodes, and says what it should.
#[test]
fn a_written_file_probes_as_the_format_it_was_asked_for() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("format");

    let mut writer = VideoWriter::create(scratch.path(), format(2)).expect("create");
    eprintln!("encoder: {}", writer.encoder().label);

    for index in 0..FRAMES {
        writer.push_frame(&frame(index)).expect("push frame");
        // 2000 samples per frame at 24 fps is 48,000 a second — the rate
        // §20a.3 fixes everything at.
        writer.push_audio(&tone(2000, 2)).expect("push audio");
    }
    writer.finish().expect("finish");

    let asset = FfmpegProber
        .probe(scratch.path())
        .expect("probe the result");
    assert_eq!(asset.width, WIDTH);
    assert_eq!(asset.height, HEIGHT);
    assert!(
        asset.audio_codec.is_some(),
        "audio was written but the file has no audio stream"
    );

    // 48 frames at 24 fps is two seconds. Containers round, so this is a range.
    let seconds = asset.duration.ticks() as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    assert!(
        (1.9..=2.2).contains(&seconds),
        "expected about 2 seconds, got {seconds:.3}"
    );
}

/// Every frame pushed has to come back out. An encoder that was never flushed
/// loses its tail, which is the classic version of this bug: the file opens,
/// plays, and quietly ends early.
#[test]
fn every_frame_survives_to_the_file() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("frames");

    let mut writer = VideoWriter::create(scratch.path(), format(0)).expect("create");
    for index in 0..FRAMES {
        writer.push_frame(&frame(index)).expect("push frame");
    }
    assert_eq!(writer.frames_written(), FRAMES as i64);
    writer.finish().expect("finish");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open the written file");

    let mut decoded = 0;
    while let Some(_frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        decoded += 1;
        if decoded > FRAMES * 2 {
            panic!("the file has more frames than were written");
        }
    }

    assert_eq!(
        decoded, FRAMES,
        "wrote {FRAMES} frames and decoded {decoded} back"
    );
}

/// The picture has to arrive. A writer that mixed up strides, or handed the
/// encoder an uninitialised buffer, still produces a valid file — so this
/// checks the block is where it was put, at the start and at the end.
#[test]
fn the_picture_moves_the_way_it_was_drawn() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("picture");

    let mut writer = VideoWriter::create(scratch.path(), format(0)).expect("create");
    for index in 0..FRAMES {
        writer.push_frame(&frame(index)).expect("push frame");
    }
    writer.finish().expect("finish");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");

    let mut first = None;
    let mut last = None;
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        let centre = brightest_column(&frame);
        if first.is_none() {
            first = Some(centre);
        }
        last = Some(centre);
    }

    let (first, last) = (first.expect("no frames"), last.expect("no frames"));
    assert!(
        first < WIDTH / 3,
        "the block should start on the left, found it at column {first}"
    );
    assert!(
        last > WIDTH * 2 / 3,
        "the block should end on the right, found it at column {last}"
    );
}

/// Where the brightest column of a decoded frame is.
fn brightest_column(frame: &bettercut_media::VideoFrame) -> u32 {
    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("expected a system-memory frame");
    };

    let row = frame.height / 2;
    let mut best = (0_u32, 0_u32);
    for x in 0..frame.width {
        let at = (row * stride + x * 4) as usize;
        let luma = u32::from(data[at]) + u32::from(data[at + 1]) + u32::from(data[at + 2]);
        if luma > best.1 {
            best = (x, luma);
        }
    }
    best.0
}

/// H.264 4:2:0 cannot represent an odd dimension, and quietly exporting one
/// pixel smaller than the sequence would be worse than refusing.
#[test]
fn an_odd_resolution_is_refused_before_anything_is_written() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("odd");
    let odd = ExportFormat {
        width: 321,
        ..format(0)
    };

    assert!(VideoWriter::create(scratch.path(), odd).is_err());
    assert!(
        !scratch.path().exists() || std::fs::metadata(scratch.path()).unwrap().len() == 0,
        "a refused export should not leave a file behind"
    );
}

/// A partial audio block at the end must be padded, not dropped — otherwise up
/// to 21 ms goes missing off the end of every export.
#[test]
fn a_partial_audio_block_at_the_end_is_still_written() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("audio-tail");

    let mut writer = VideoWriter::create(scratch.path(), format(2)).expect("create");
    for index in 0..FRAMES {
        writer.push_frame(&frame(index)).expect("push frame");
    }
    // 100 samples: far less than AAC's 1024-sample frame, so this only reaches
    // the file if the remainder is flushed.
    writer.push_audio(&tone(100, 2)).expect("push audio");
    writer.finish().expect("finish");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    assert!(
        asset.audio_codec.is_some(),
        "the trailing partial block was dropped, leaving no audio stream"
    );
}

/// Silent export is a real case — a sequence with no audio tracks — and must
/// not produce an empty audio stream that players stumble over.
#[test]
fn a_silent_export_has_no_audio_stream() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("silent");

    let mut writer = VideoWriter::create(scratch.path(), format(0)).expect("create");
    for index in 0..8 {
        writer.push_frame(&frame(index)).expect("push frame");
    }
    // Audio pushed to a silent writer is ignored rather than an error: the
    // caller mixes the timeline without knowing whether anything was audible.
    writer.push_audio(&tone(2000, 2)).expect("ignored");
    writer.finish().expect("finish");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    assert!(asset.video_codec.is_some());
    assert!(
        asset.audio_codec.is_none(),
        "a silent export wrote an audio track"
    );
}

/// Timestamps have to be integers all the way through (§9). Over a long export
/// a float PTS accumulates drift, and the symptom is audio that slides out of
/// sync only near the end — the hardest kind to notice before shipping.
#[test]
fn ntsc_rates_produce_an_exact_duration() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("ntsc");
    let ntsc = ExportFormat {
        frame_rate: bettercut_foundation::FrameRate::NTSC_29_97,
        ..format(0)
    };

    let mut writer = VideoWriter::create(scratch.path(), ntsc).expect("create");
    // 300 frames at 29.97 is 10.01 seconds.
    for index in 0..300 {
        writer.push_frame(&frame(index % FRAMES)).expect("push");
    }
    writer.finish().expect("finish");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let seconds = asset.duration.ticks() as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    assert!(
        (9.9..=10.2).contains(&seconds),
        "expected about 10.01 seconds at 29.97 fps, got {seconds:.4}"
    );
}

/// Cancellation is the caller's job, but the writer must survive being dropped
/// part-way through without leaving the process in a bad state (§48).
#[test]
fn abandoning_a_writer_mid_export_is_safe() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("abandoned");
    {
        let mut writer = VideoWriter::create(scratch.path(), format(2)).expect("create");
        for index in 0..6 {
            writer.push_frame(&frame(index)).expect("push");
        }
        // Dropped without `finish`: no trailer, so the file is unplayable —
        // which is correct. What matters is that nothing leaks or panics.
    }

    let _: Result<_, _> = FfmpegProber.probe(scratch.path());
}

/// Unused, but kept honest: the token type the export loop will pass through.
const _: fn() = || {
    fn assert_token<T: CancellationToken>() {}
    assert_token::<NeverCancelled>();
};

/// A frame smaller than the format is a caller bug, not something to pad.
#[test]
fn a_short_frame_is_refused() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("short");
    let mut writer = VideoWriter::create(scratch.path(), format(0)).expect("create");
    assert!(writer.push_frame(&[0_u8; 16]).is_err());
}

/// Sanity on the fixture itself: if the block did not move, the picture test
/// above would pass for the wrong reason.
#[test]
fn the_fixture_actually_moves() {
    let _encoder = encoder_guard();
    let start = frame(0);
    let end = frame(FRAMES - 1);
    assert_ne!(start, end, "the fixture is the same at both ends");
    assert_eq!(start.len(), (WIDTH * HEIGHT * 4) as usize);
}

/// Kept next to the tests it explains: the writer takes a path, so the caller
/// decides where exports land. §74 forbids touching source media, and an
/// export that defaulted to the source directory would invite exactly that.
#[test]
fn the_writer_writes_only_where_it_is_told() {
    let _encoder = encoder_guard();
    let scratch = Scratch::new("location");
    let mut writer = VideoWriter::create(scratch.path(), format(0)).expect("create");
    writer.push_frame(&frame(0)).expect("push");
    writer.finish().expect("finish");

    assert!(scratch.path().exists());
    assert!(
        std::fs::metadata(scratch.path()).unwrap().len() > 0,
        "the file is empty"
    );
}
