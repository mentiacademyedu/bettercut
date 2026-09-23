//! What a file is, in a few lines: the answer to "why does this clip look
//! soft" or "why won't this play smoothly" is usually here — a 720p source,
//! a 10-bit HDR file, a 25 fps clip in a 30 fps edit.

use bettercut_editor_core::media::{MediaAsset, MediaKind};

/// A file's details, one fact a line, for a hover.
pub fn details(asset: &MediaAsset) -> String {
    let mut lines = Vec::new();
    match asset.kind {
        MediaKind::Video | MediaKind::Image => {
            let mut picture = format!("{}×{}", asset.width, asset.height);
            if let Some(rate) = asset.frame_rate {
                picture.push_str(&format!(" · {rate} fps"));
            }
            if asset.color.needs_normalization() {
                picture.push_str(if asset.color.transfer.is_hdr() {
                    " · HDR"
                } else {
                    " · 10-bit"
                });
            }
            lines.push(picture);
        }
        MediaKind::Audio => {}
    }
    if let Some(codec) = &asset.video_codec {
        lines.push(format!("picture: {codec}"));
    }
    match (
        &asset.audio_codec,
        asset.audio_sample_rate,
        asset.audio_channels,
    ) {
        (Some(codec), rate, channels) => {
            let mut sound = format!("sound: {codec}");
            if let Some(rate) = rate {
                sound.push_str(&format!(" · {:.1} kHz", f64::from(rate) / 1000.0));
            }
            if let Some(channels) = channels {
                sound.push_str(match channels {
                    1 => " · mono",
                    2 => " · stereo",
                    _ => " · surround",
                });
            }
            lines.push(sound);
        }
        (None, ..) if asset.kind == MediaKind::Video => lines.push("no sound".to_owned()),
        _ => {}
    }
    if !asset.is_still() && !asset.duration.is_zero() {
        lines.push(format!(
            "length {}",
            bettercut_editor_core::foundation::TimelineTime::from_ticks(asset.duration.ticks())
                .format_timecode()
        ));
    }
    if asset.file_size > 0 {
        lines.push(size(asset.file_size));
    }
    if let Some(sequence) = asset.sequence {
        lines.push(format!("{} numbered stills", sequence.count));
    }
    if asset.deinterlace {
        lines.push("deinterlaced".to_owned());
    }
    if !asset.path.as_os_str().is_empty() {
        lines.push(asset.path.display().to_string());
    }
    lines.join("\n")
}

/// Bytes as a person reads them.
fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= 1024.0 * MB {
        format!("{:.1} GB", bytes / (1024.0 * MB))
    } else if bytes >= MB {
        format!("{:.0} MB", bytes / MB)
    } else {
        format!("{:.0} KB", (bytes / 1024.0).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::foundation::{FrameRate, MediaTime};

    #[test]
    fn a_video_says_what_it_is() {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            "C:/media/trip.mp4",
            MediaTime::from_seconds(90),
        )
        .with_video(1920, 1080, FrameRate::PAL_25)
        .with_audio(48_000, 2);
        asset.video_codec = Some("h264".to_owned());
        asset.audio_codec = Some("aac".to_owned());
        asset.file_size = 150 * 1024 * 1024;
        let text = details(&asset);
        assert!(text.contains("1920×1080 · 25 fps"), "{text}");
        assert!(text.contains("picture: h264"), "{text}");
        assert!(text.contains("sound: aac · 48.0 kHz · stereo"), "{text}");
        assert!(text.contains("150 MB"), "{text}");
        assert!(text.contains("trip.mp4"), "{text}");
    }

    #[test]
    fn a_silent_video_says_so() {
        let asset = MediaAsset::new(MediaKind::Video, "C:/a.mp4", MediaTime::from_seconds(5));
        assert!(details(&asset).contains("no sound"));
    }

    #[test]
    fn sizes_read_as_people_read_them() {
        assert_eq!(size(500), "1 KB");
        assert_eq!(size(5 * 1024 * 1024), "5 MB");
        assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
