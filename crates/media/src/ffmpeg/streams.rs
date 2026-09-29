//! Which picture and which sound in a file are *the* picture and sound.
//!
//! Taking the first of each kind is wrong more often than it looks:
//!
//! * A recent iPhone records Spatial Audio in Apple's APAC codec, which
//!   nothing outside Apple decodes, next to an ordinary AAC track. APAC is
//!   often first; the video would come in silent.
//! * A file with cover art carries the picture as a "video" stream, usually
//!   first; a music video would come in as a single still.
//! * Cinematic-mode iPhone video and camera files carry depth, preview or
//!   timecode streams beside the real one.
//!
//! So the choice is made once, here, by rules that can be read and tested:
//! only streams FFmpeg can decode, never cover art or a stream that only makes
//! sense as part of another, the one the file marks as default first, and
//! then the largest picture or the one with the most useful channels.

use rusty_ffmpeg::ffi;

use super::InputContext;

/// What the choice needs to know about one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamInfo {
    pub index: i32,
    pub kind: StreamKind,
    /// FFmpeg has a decoder for it.
    pub decodable: bool,
    /// The file marks it as the one to play.
    pub default: bool,
    /// Cover art: never the picture itself.
    pub cover: bool,
    /// Only a part of another picture (one tile of a HEIF photo): used only
    /// when there is nothing whole.
    pub part: bool,
    /// Width times height, for pictures.
    pub area: u64,
    /// Channels, for sound.
    pub channels: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Other,
}

/// The picture stream and the sound stream to use, by index.
pub fn choose(streams: &[StreamInfo]) -> (Option<i32>, Option<i32>) {
    let video = streams
        .iter()
        .filter(|s| s.kind == StreamKind::Video && s.decodable && !s.cover)
        // `max_by_key` keeps the last of equals; reversed, so the first wins.
        .rev()
        .max_by_key(|s| (!s.part, s.default, s.area))
        .map(|s| s.index);
    let audio = streams
        .iter()
        .filter(|s| s.kind == StreamKind::Audio && s.decodable)
        .rev()
        // Default first; then stereo or mono over many channels, which is
        // what an edit is made from, and anything over nothing.
        .max_by_key(|s| (s.default, s.channels > 0 && s.channels <= 2))
        .map(|s| s.index);
    (video, audio)
}

/// The streams of `input`, described for [`choose`].
pub(crate) fn describe(input: &InputContext) -> Vec<StreamInfo> {
    input
        .streams()
        .into_iter()
        .filter_map(|stream| {
            // SAFETY: `streams()` filtered nulls; each stream and its
            // parameters live as long as `input`.
            unsafe {
                let s = &*stream;
                let par = s.codecpar.as_ref()?;
                let kind = if par.codec_type == ffi::AVMEDIA_TYPE_VIDEO {
                    StreamKind::Video
                } else if par.codec_type == ffi::AVMEDIA_TYPE_AUDIO {
                    StreamKind::Audio
                } else {
                    StreamKind::Other
                };
                let disposition = s.disposition;
                Some(StreamInfo {
                    index: s.index,
                    kind,
                    decodable: !ffi::avcodec_find_decoder(par.codec_id).is_null(),
                    default: disposition & ffi::AV_DISPOSITION_DEFAULT as i32 != 0,
                    cover: disposition & ffi::AV_DISPOSITION_ATTACHED_PIC as i32 != 0,
                    part: disposition & ffi::AV_DISPOSITION_DEPENDENT as i32 != 0,
                    area: u64::from(par.width.max(0) as u32) * u64::from(par.height.max(0) as u32),
                    channels: par.ch_layout.nb_channels.clamp(0, i32::from(u16::MAX)) as u16,
                })
            }
        })
        .collect()
}

/// The picture and sound streams of `input` to use.
pub(crate) fn chosen(input: &InputContext) -> (Option<i32>, Option<i32>) {
    choose(&describe(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(index: i32, area: u64) -> StreamInfo {
        StreamInfo {
            index,
            kind: StreamKind::Video,
            decodable: true,
            default: false,
            cover: false,
            part: false,
            area,
            channels: 0,
        }
    }

    fn audio(index: i32, channels: u16) -> StreamInfo {
        StreamInfo {
            index,
            kind: StreamKind::Audio,
            decodable: true,
            default: false,
            cover: false,
            part: false,
            area: 0,
            channels,
        }
    }

    /// An iPhone's Spatial Audio (APAC, which cannot be decoded) marked as
    /// the default, beside a stereo AAC: the AAC is what plays.
    #[test]
    fn undecodable_spatial_audio_gives_way_to_the_aac() {
        let streams = [
            video(0, 1920 * 1080),
            StreamInfo {
                decodable: false,
                default: true,
                ..audio(1, 4)
            },
            audio(2, 2),
        ];
        assert_eq!(choose(&streams), (Some(0), Some(2)));
    }

    /// Cover art first and the real picture second: the real picture.
    #[test]
    fn cover_art_is_never_the_picture() {
        let streams = [
            StreamInfo {
                cover: true,
                default: true,
                ..video(0, 3000 * 3000)
            },
            video(1, 1280 * 720),
            audio(2, 2),
        ];
        assert_eq!(choose(&streams).0, Some(1));
        // A song with only its cover: no picture at all, just the sound.
        assert_eq!(choose(&[streams[0], streams[2]]), (None, Some(2)));
    }

    /// The default wins; among equals the largest picture; among equals of
    /// that, the first.
    #[test]
    fn default_then_largest_then_first() {
        let depth = video(1, 640 * 360);
        let main = video(0, 1920 * 1080);
        assert_eq!(choose(&[depth, main]).0, Some(0));
        let marked = StreamInfo {
            default: true,
            ..depth
        };
        assert_eq!(choose(&[main, marked]).0, Some(1));
        assert_eq!(choose(&[video(3, 100), video(4, 100)]).0, Some(3));
    }

    /// A tiled photo has only tiles: the first tile stands for the picture,
    /// but a whole picture beside them always wins.
    #[test]
    fn tiles_only_when_nothing_is_whole() {
        let tile = |i| StreamInfo {
            part: true,
            ..video(i, 512 * 512)
        };
        assert_eq!(choose(&[tile(0), tile(1)]).0, Some(0));
        assert_eq!(choose(&[tile(0), video(1, 256 * 256)]).0, Some(1));
    }

    /// Stereo over a many-channel mix when neither is marked default.
    #[test]
    fn stereo_over_many_channels() {
        assert_eq!(choose(&[audio(0, 6), audio(1, 2)]).1, Some(1));
        assert_eq!(choose(&[audio(0, 2), audio(1, 2)]).1, Some(0));
    }
}
