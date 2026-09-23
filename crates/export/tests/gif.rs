//! Animated GIF export (`bettercut_export::gif`).
//!
//! Read back with a small decoder written here from the format — a file the
//! writer and its own reader agree on could still be one nothing else opens,
//! so the reader follows the spec rather than the writer: blocks in order,
//! extensions skipped by length, LZW sub-blocks joined and decoded.
//!
//! The end-to-end test opens a GPU and skips itself without one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_export::{
    ExportSettings, GifWriter, MAX_COLOURS, VideoCodec, export, frame_delay, quantize,
};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime, ticks_per_frame};
use bettercut_media::{FfmpegProber, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::{Resolution, SourceRange, TimelineRange, VideoClip};

/// One decoded frame.
struct Frame {
    delay: u16,
    width: u16,
    height: u16,
    rgb: Vec<[u8; 3]>,
}

/// Everything a reader needs from the file.
struct Decoded {
    width: u16,
    height: u16,
    loops_forever: bool,
    frames: Vec<Frame>,
}

fn decode(bytes: &[u8]) -> Decoded {
    assert_eq!(&bytes[..6], b"GIF89a", "not a GIF89a file");
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let (width, height) = (u16_at(6), u16_at(8));
    let packed = bytes[10];
    let mut at = 13;
    let mut global = Vec::new();
    if packed & 0x80 != 0 {
        let size = 3 << ((packed & 7) + 1);
        global = bytes[at..at + size]
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        at += size;
    }

    let sub_blocks = |at: &mut usize| {
        let mut data = Vec::new();
        loop {
            let len = usize::from(bytes[*at]);
            *at += 1;
            if len == 0 {
                return data;
            }
            data.extend_from_slice(&bytes[*at..*at + len]);
            *at += len;
        }
    };

    let mut loops_forever = false;
    let mut delay = 0;
    let mut frames = Vec::new();
    loop {
        match bytes[at] {
            0x21 => {
                let label = bytes[at + 1];
                at += 2;
                let data = sub_blocks(&mut at);
                if label == 0xF9 {
                    delay = u16::from_le_bytes([data[1], data[2]]);
                } else if label == 0xFF && data.starts_with(b"NETSCAPE2.0") {
                    // The loop count is the next sub-block, already joined on.
                    loops_forever =
                        data.len() >= 14 && data[11] == 1 && data[12] == 0 && data[13] == 0;
                }
            }
            0x2C => {
                let (w, h) = (u16_at(at + 5), u16_at(at + 7));
                let flags = bytes[at + 9];
                at += 10;
                let mut table = global.clone();
                if flags & 0x80 != 0 {
                    let size = 3 << ((flags & 7) + 1);
                    table = bytes[at..at + size]
                        .chunks(3)
                        .map(|c| [c[0], c[1], c[2]])
                        .collect();
                    at += size;
                }
                let min_code = bytes[at];
                at += 1;
                let data = sub_blocks(&mut at);
                let indices = weezl::decode::Decoder::new(weezl::BitOrder::Lsb, min_code)
                    .decode(&data)
                    .expect("LZW data decodes");
                assert_eq!(indices.len(), usize::from(w) * usize::from(h));
                frames.push(Frame {
                    delay,
                    width: w,
                    height: h,
                    rgb: indices.iter().map(|i| table[usize::from(*i)]).collect(),
                });
            }
            0x3B => break,
            other => panic!("unexpected block {other:#x} at {at}"),
        }
    }
    Decoded {
        width,
        height,
        loops_forever,
        frames,
    }
}

fn rgba(pixels: &[[u8; 3]]) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|[r, g, b]| [*r, *g, *b, 255])
        .collect()
}

/// A picture with few colours comes back exactly.
#[test]
fn a_few_colours_come_back_exactly() {
    let colours = [[255, 0, 0], [0, 200, 40], [10, 20, 250], [255, 255, 255]];
    let pixels: Vec<[u8; 3]> = (0..64).map(|i| colours[i % 4]).collect();

    let mut writer = GifWriter::new(Vec::new(), 8, 8).unwrap();
    writer.push(&rgba(&pixels), 10).unwrap();
    let bytes = writer.finish().unwrap();

    let decoded = decode(&bytes);
    assert_eq!((decoded.width, decoded.height), (8, 8));
    assert!(decoded.loops_forever, "the loop extension is missing");
    assert_eq!(decoded.frames.len(), 1);
    assert_eq!((decoded.frames[0].width, decoded.frames[0].height), (8, 8));
    assert_eq!(decoded.frames[0].delay, 10);
    assert_eq!(decoded.frames[0].rgb, pixels);
}

/// A full gradient is brought down to 256 colours without any pixel landing
/// far from where it was.
#[test]
fn a_gradient_is_brought_down_to_256_close_colours() {
    let (width, height) = (256_usize, 128_usize);
    let pixels: Vec<[u8; 3]> = (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            [x as u8, (y * 2) as u8, ((x + y) / 2) as u8]
        })
        .collect();
    let (palette, indices) = quantize(&rgba(&pixels));
    assert!(palette.len() <= MAX_COLOURS);
    assert_eq!(indices.len(), pixels.len());

    let errors: Vec<u32> = pixels
        .iter()
        .zip(&indices)
        .flat_map(|(pixel, index)| {
            let chosen = palette[usize::from(*index)];
            (0..3).map(move |c| i32::from(pixel[c]).abs_diff(i32::from(chosen[c])))
        })
        .collect();
    let worst = errors.iter().copied().max().unwrap();
    let mean = errors.iter().map(|e| f64::from(*e)).sum::<f64>() / errors.len() as f64;
    // Splitting at the median gives a worst of 8 and a mean of about 3.4 here;
    // a lopsided split leaves some boxes far too wide.
    assert!(worst <= 12, "a pixel moved {worst} levels in one channel");
    assert!(mean <= 4.0, "pixels moved {mean:.2} levels on average");
    assert!(
        palette.len() > 200,
        "only {} colours were used",
        palette.len()
    );
}

/// Delays follow the timeline: at 15 fps a second is 100 centiseconds over
/// fifteen frames, however the rounding falls, and never drifts.
#[test]
fn delays_add_up_to_the_timeline() {
    let tpf = ticks_per_frame(FrameRate::new(15, 1).unwrap()).unwrap();
    let delays: Vec<u16> = (0..15).map(|i| frame_delay(i, tpf)).collect();
    assert_eq!(delays.iter().map(|d| u32::from(*d)).sum::<u32>(), 100);
    assert!(delays.iter().all(|d| *d == 6 || *d == 7), "{delays:?}");
    // Each frame starts at the nearest centisecond: the second frame is due
    // at 6.67 cs, which is 7.
    assert_eq!(delays[0], 7);
    let minute: u32 = (0..900).map(|i| u32::from(frame_delay(i, tpf))).sum();
    assert_eq!(minute, 6_000, "a minute at 15 fps drifted");

    let tpf_10 = ticks_per_frame(FrameRate::new(10, 1).unwrap()).unwrap();
    assert_eq!(frame_delay(0, tpf_10), 10);
}

/// A frame smaller than the GIF is refused rather than read past its end.
#[test]
fn a_short_frame_is_refused() {
    let mut writer = GifWriter::new(Vec::new(), 8, 8).unwrap();
    assert!(writer.push(&[0; 16], 10).is_err());
}

// ---- end to end ------------------------------------------------------------

fn gpu_available() -> bool {
    let instance = bettercut_renderer::wgpu::Instance::new(
        bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    pollster::block_on(
        instance.request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("bettercut-gif-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const SOURCE: Resolution = Resolution {
    width: 320,
    height: 180,
};

/// Two seconds of a white bar sweeping left to right across black.
fn sweeping_bar(path: &Path) {
    use bettercut_media::{ExportFormat, VideoWriter};

    let (width, height) = (SOURCE.width, SOURCE.height);
    let mut writer = VideoWriter::create(
        path,
        ExportFormat {
            transparent: false,
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
    let bar = width / 5;
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for frame in 0..60 {
        rgba.fill(0);
        let left = (frame * (width - bar)) / 59;
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

/// A GIF export writes a looping file at the size and rate asked for, and the
/// picture in it is the edit: the bar moves right from frame to frame.
#[test]
fn a_gif_export_is_the_edit_at_the_asked_size_and_rate() {
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let source = Scratch::new("source.mp4");
    sweeping_bar(&source.0);

    let mut project = Project::new("Gif");
    let asset = FfmpegProber.probe(&source.0).expect("probe");
    let duration = asset.duration;
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = SOURCE;
    sequence.frame_rate = FrameRate::FPS_30;
    sequence.video_tracks[0]
        .insert(
            VideoClip::new(
                media,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, duration).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let sequence = project.active().unwrap().clone();

    let out = Scratch::new("out.gif");
    let mut settings = ExportSettings::for_sequence(out.0.clone(), &sequence);
    settings.gif = true;
    settings.resolution = Resolution {
        width: 160,
        height: 90,
    };
    settings.frame_rate = FrameRate::new(10, 1).unwrap();
    settings.range = TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(1)).unwrap();

    let summary =
        export(&project, &sequence, &settings, &mut |_| {}, &NeverCancelled).expect("export a GIF");
    assert_eq!(summary.frames, 10);

    let decoded = decode(&std::fs::read(&out.0).expect("read the GIF"));
    assert_eq!((decoded.width, decoded.height), (160, 90));
    assert!(decoded.loops_forever);
    assert_eq!(decoded.frames.len(), 10);
    assert!(decoded.frames.iter().all(|frame| frame.delay == 10));

    // Where the bar's centre is along the middle row, frame by frame.
    let centres: Vec<usize> = decoded
        .frames
        .iter()
        .map(|frame| {
            let row = &frame.rgb[45 * 160..46 * 160];
            let bright: Vec<usize> = (0..160).filter(|x| row[*x][0] > 128).collect();
            assert!(!bright.is_empty(), "no bar in a frame");
            (bright[0] + bright[bright.len() - 1]) / 2
        })
        .collect();
    assert!(
        centres.windows(2).all(|pair| pair[1] > pair[0]),
        "the bar did not move right across the loop: {centres:?}"
    );
}

/// An image sequence writes one numbered PNG a frame, at the asked size, in a
/// folder named after the file — and the pictures are the edit.
#[test]
fn an_image_sequence_is_one_png_a_frame() {
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let source = Scratch::new("frames-source.mp4");
    sweeping_bar(&source.0);

    let mut project = Project::new("Frames");
    let asset = FfmpegProber.probe(&source.0).expect("probe");
    let duration = asset.duration;
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = SOURCE;
    sequence.frame_rate = FrameRate::FPS_30;
    sequence.video_tracks[0]
        .insert(
            VideoClip::new(
                media,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, duration).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let sequence = project.active().unwrap().clone();

    let out = std::env::temp_dir().join(format!("bettercut-frames-{}.png", std::process::id()));
    let folder = bettercut_export::frames_folder(&out);
    let _ = std::fs::remove_dir_all(&folder);
    let mut settings = ExportSettings::for_sequence(out.clone(), &sequence);
    settings.image_sequence = true;
    settings.resolution = Resolution {
        width: 160,
        height: 90,
    };
    settings.frame_rate = FrameRate::new(5, 1).unwrap();
    settings.range = TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(1)).unwrap();

    let summary = export(&project, &sequence, &settings, &mut |_| {}, &NeverCancelled)
        .expect("export the frames");
    assert_eq!(summary.frames, 5);
    assert_eq!(summary.path, folder);

    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let stem = out.file_stem().unwrap().to_string_lossy().into_owned();
    let expected: Vec<String> = (1..=5).map(|i| format!("{stem}_{i:05}.png")).collect();
    assert_eq!(names, expected);

    // The bar moves right from the first picture to the last.
    let centre = |index: u64| {
        let decoder = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(bettercut_export::frame_file(&out, index)).unwrap(),
        ));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (160, 90));
        let row = 45 * 160 * 4;
        let bright: Vec<usize> = (0..160).filter(|x| pixels[row + x * 4] > 128).collect();
        (bright[0] + bright[bright.len() - 1]) / 2
    };
    let centres: Vec<usize> = (0..5).map(centre).collect();
    assert!(
        centres[4] > centres[0],
        "the pictures are not the edit: {centres:?}"
    );
    let _ = std::fs::remove_dir_all(&folder);
}
