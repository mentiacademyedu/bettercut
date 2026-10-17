//! Animated WebP unpacked into steady frames (`animated_webp`): each of the
//! file's frames held for as long as it says, its transparency kept.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use bettercut_media::animated_webp::{is_animated, unpack};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/animated.webp")
}

/// The pixel at (x, y) of a PNG frame, as RGBA.
fn pixel(path: &std::path::Path, x: u32, y: u32) -> [u8; 4] {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba, "transparency lost");
    let at = ((y * info.width + x) * 4) as usize;
    [buffer[at], buffer[at + 1], buffer[at + 2], buffer[at + 3]]
}

#[test]
fn the_frames_are_held_for_as_long_as_the_file_says() {
    assert!(is_animated(&fixture()));
    let folder = tempfile::tempdir().unwrap();
    let first = unpack(&fixture(), folder.path()).unwrap();
    assert_eq!(first, folder.path().join("frame_00001.png"));

    // 100 + 300 + 200 ms at 30 a second: 18 frames.
    let frames = std::fs::read_dir(folder.path()).unwrap().count();
    assert_eq!(frames, 18);

    let frame = |n: u32| folder.path().join(format!("frame_{n:05}.png"));
    // Red for 100 ms (three frames), green for 300 (nine), blue for 200.
    for (n, colour) in [
        (1, [255, 0, 0]),
        (3, [255, 0, 0]),
        (4, [0, 255, 0]),
        (12, [0, 255, 0]),
        (13, [0, 0, 255]),
        (18, [0, 0, 255]),
    ] {
        let [r, g, b, a] = pixel(&frame(n), 8, 8);
        assert_eq!([r, g, b], colour, "frame {n}");
        assert_eq!(a, 255);
    }
    assert_eq!(
        pixel(&frame(5), 50, 8)[3],
        0,
        "the clear half came out solid"
    );

    // A second import finds them already there.
    assert_eq!(unpack(&fixture(), folder.path()).unwrap(), first);
}
