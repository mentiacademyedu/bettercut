//! Writing an MP4 from frames something else has already encoded.
//!
//! The web build's export: the browser's encoders (WebCodecs) turn pictures
//! and sound into H.264 and AAC or Opus, and say nothing about files. This
//! puts what they made into a file a phone, a player or an upload page reads.
//!
//! Kept small on purpose: one video track and at most one sound track, one
//! sample per chunk, the samples of each track one after another, and the
//! index (`moov`) *before* them, so the file starts playing before it has
//! all arrived. Offsets are always 64-bit, so the index is the same size
//! whatever they turn out to be and can be written before the data it
//! points into.

/// One track's samples, in decode order.
#[derive(Debug, Clone)]
pub struct Mp4Track {
    pub kind: Mp4TrackKind,
    /// Units per second for `Mp4Sample::duration`.
    pub timescale: u32,
    pub samples: Vec<Mp4Sample>,
}

#[derive(Debug, Clone)]
pub enum Mp4TrackKind {
    /// H.264. `avcc` is the decoder configuration record as an encoder hands
    /// it over (WebCodecs' `decoderConfig.description` in `avc` format).
    Video { width: u32, height: u32, avcc: Vec<u8> },
    /// AAC. `config` is the AudioSpecificConfig.
    Aac { channels: u16, sample_rate: u32, config: Vec<u8>, bitrate: u32 },
    /// Opus, which is always 48 kHz.
    Opus { channels: u16, pre_skip: u16 },
}

/// One encoded frame of picture or sound.
#[derive(Debug, Clone)]
pub struct Mp4Sample {
    pub data: Vec<u8>,
    /// In the track's timescale.
    pub duration: u32,
    /// A keyframe: a player can start here.
    pub sync: bool,
}

/// The movie's own clock, for the header's overall length.
const MOVIE_TIMESCALE: u32 = 1000;

/// The tracks as one MP4 file.
pub fn mux(tracks: &[Mp4Track]) -> Vec<u8> {
    let ftyp = ftyp();
    // Built once with offsets counted from zero to learn the index's size,
    // then again with the real ones: 64-bit offsets keep the size the same.
    let sizes = moov(tracks, 0).len() as u64;
    let data_start = ftyp.len() as u64 + sizes + 16;
    let moov = moov(tracks, data_start);
    debug_assert_eq!(moov.len() as u64, sizes);

    let data: u64 = tracks
        .iter()
        .flat_map(|t| &t.samples)
        .map(|s| s.data.len() as u64)
        .sum();
    let mut out = Vec::with_capacity((data_start + data) as usize);
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    // `mdat` with a 64-bit size, so a file past 4 GB needs nothing else.
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(b"mdat");
    out.extend_from_slice(&(16 + data).to_be_bytes());
    for track in tracks {
        for sample in &track.samples {
            out.extend_from_slice(&sample.data);
        }
    }
    out
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A box with a version and flags before its body.
fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(body.len() + 4);
    inner.extend_from_slice(&((u32::from(version) << 24) | flags).to_be_bytes());
    inner.extend_from_slice(body);
    boxed(kind, &inner)
}

fn ftyp() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"isom");
    body.extend_from_slice(&512u32.to_be_bytes());
    for brand in [b"isom", b"iso2", b"avc1", b"mp41"] {
        body.extend_from_slice(brand);
    }
    boxed(b"ftyp", &body)
}

/// The identity matrix, as every header that has one wants it.
const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn push_matrix(body: &mut Vec<u8>) {
    for value in MATRIX {
        body.extend_from_slice(&value.to_be_bytes());
    }
}

fn track_ticks(track: &Mp4Track) -> u64 {
    track.samples.iter().map(|s| u64::from(s.duration)).sum()
}

fn in_movie_time(track: &Mp4Track) -> u32 {
    (track_ticks(track) * u64::from(MOVIE_TIMESCALE) / u64::from(track.timescale.max(1))) as u32
}

fn moov(tracks: &[Mp4Track], data_start: u64) -> Vec<u8> {
    let mut body = mvhd(tracks);
    let mut offset = data_start;
    for (index, track) in tracks.iter().enumerate() {
        body.extend_from_slice(&trak(track, index as u32 + 1, offset));
        offset += track
            .samples
            .iter()
            .map(|s| s.data.len() as u64)
            .sum::<u64>();
    }
    boxed(b"moov", &body)
}

fn mvhd(tracks: &[Mp4Track]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&[0; 8]); // created, modified
    body.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
    let duration = tracks.iter().map(in_movie_time).max().unwrap_or(0);
    body.extend_from_slice(&duration.to_be_bytes());
    body.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    body.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    body.extend_from_slice(&[0; 10]);
    push_matrix(&mut body);
    body.extend_from_slice(&[0; 24]);
    body.extend_from_slice(&(tracks.len() as u32 + 1).to_be_bytes());
    full(b"mvhd", 0, 0, &body)
}

fn trak(track: &Mp4Track, id: u32, offset: u64) -> Vec<u8> {
    let mut body = tkhd(track, id);
    body.extend_from_slice(&mdia(track, offset));
    boxed(b"trak", &body)
}

fn tkhd(track: &Mp4Track, id: u32) -> Vec<u8> {
    let (sound, width, height) = match track.kind {
        Mp4TrackKind::Video { width, height, .. } => (false, width, height),
        _ => (true, 0, 0),
    };
    let mut body = Vec::new();
    body.extend_from_slice(&[0; 8]);
    body.extend_from_slice(&id.to_be_bytes());
    body.extend_from_slice(&[0; 4]);
    body.extend_from_slice(&in_movie_time(track).to_be_bytes());
    body.extend_from_slice(&[0; 8]);
    body.extend_from_slice(&[0; 4]); // layer, alternate group
    body.extend_from_slice(&(if sound { 0x0100u16 } else { 0 }).to_be_bytes());
    body.extend_from_slice(&[0; 2]);
    push_matrix(&mut body);
    body.extend_from_slice(&(width << 16).to_be_bytes());
    body.extend_from_slice(&(height << 16).to_be_bytes());
    // Enabled, in the movie, in the preview.
    full(b"tkhd", 0, 0x7, &body)
}

fn mdia(track: &Mp4Track, offset: u64) -> Vec<u8> {
    let video = matches!(track.kind, Mp4TrackKind::Video { .. });

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&[0; 8]);
    mdhd.extend_from_slice(&track.timescale.to_be_bytes());
    mdhd.extend_from_slice(&(track_ticks(track) as u32).to_be_bytes());
    mdhd.extend_from_slice(&0x55c4u16.to_be_bytes()); // "und"
    mdhd.extend_from_slice(&[0; 2]);

    let mut hdlr = vec![0; 4];
    hdlr.extend_from_slice(if video { b"vide" } else { b"soun" });
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.extend_from_slice(if video {
        b"VideoHandler\0".as_slice()
    } else {
        b"SoundHandler\0".as_slice()
    });

    let header = if video {
        full(b"vmhd", 0, 1, &[0; 8])
    } else {
        full(b"smhd", 0, 0, &[0; 4])
    };
    let dref = {
        let mut body = 1u32.to_be_bytes().to_vec();
        // Flag 1: the data is in this file.
        body.extend_from_slice(&full(b"url ", 0, 1, &[]));
        full(b"dref", 0, 0, &body)
    };
    let mut minf = header;
    minf.extend_from_slice(&boxed(b"dinf", &dref));
    minf.extend_from_slice(&stbl(track, offset));

    let mut body = full(b"mdhd", 0, 0, &mdhd);
    body.extend_from_slice(&full(b"hdlr", 0, 0, &hdlr));
    body.extend_from_slice(&boxed(b"minf", &minf));
    boxed(b"mdia", &body)
}

fn stbl(track: &Mp4Track, offset: u64) -> Vec<u8> {
    let mut body = stsd(track);

    // Durations, run-length coded: almost always one run.
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for sample in &track.samples {
        match runs.last_mut() {
            Some((count, duration)) if *duration == sample.duration => *count += 1,
            _ => runs.push((1, sample.duration)),
        }
    }
    let mut stts = (runs.len() as u32).to_be_bytes().to_vec();
    for (count, duration) in runs {
        stts.extend_from_slice(&count.to_be_bytes());
        stts.extend_from_slice(&duration.to_be_bytes());
    }
    body.extend_from_slice(&full(b"stts", 0, 0, &stts));

    // Keyframes, when not every sample is one (no table means all are).
    if track.samples.iter().any(|s| !s.sync) {
        let keys: Vec<u32> = (1..)
            .zip(&track.samples)
            .filter(|(_, s)| s.sync)
            .map(|(n, _)| n)
            .collect();
        let mut stss = (keys.len() as u32).to_be_bytes().to_vec();
        for key in keys {
            stss.extend_from_slice(&key.to_be_bytes());
        }
        body.extend_from_slice(&full(b"stss", 0, 0, &stss));
    }

    // One sample per chunk.
    let mut stsc = 1u32.to_be_bytes().to_vec();
    for value in [1u32, 1, 1] {
        stsc.extend_from_slice(&value.to_be_bytes());
    }
    body.extend_from_slice(&full(b"stsc", 0, 0, &stsc));

    let mut stsz = 0u32.to_be_bytes().to_vec();
    stsz.extend_from_slice(&(track.samples.len() as u32).to_be_bytes());
    for sample in &track.samples {
        stsz.extend_from_slice(&(sample.data.len() as u32).to_be_bytes());
    }
    body.extend_from_slice(&full(b"stsz", 0, 0, &stsz));

    let mut co64 = (track.samples.len() as u32).to_be_bytes().to_vec();
    let mut at = offset;
    for sample in &track.samples {
        co64.extend_from_slice(&at.to_be_bytes());
        at += sample.data.len() as u64;
    }
    body.extend_from_slice(&full(b"co64", 0, 0, &co64));

    boxed(b"stbl", &body)
}

fn stsd(track: &Mp4Track) -> Vec<u8> {
    let entry = match &track.kind {
        Mp4TrackKind::Video {
            width,
            height,
            avcc,
        } => {
            let mut body = vec![0; 6];
            body.extend_from_slice(&1u16.to_be_bytes()); // data reference
            body.extend_from_slice(&[0; 16]);
            body.extend_from_slice(&(*width as u16).to_be_bytes());
            body.extend_from_slice(&(*height as u16).to_be_bytes());
            body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi
            body.extend_from_slice(&0x0048_0000u32.to_be_bytes());
            body.extend_from_slice(&[0; 4]);
            body.extend_from_slice(&1u16.to_be_bytes()); // one frame a sample
            body.extend_from_slice(&[0; 32]); // compressor name
            body.extend_from_slice(&0x0018u16.to_be_bytes()); // colour, no alpha
            body.extend_from_slice(&0xffffu16.to_be_bytes());
            body.extend_from_slice(&boxed(b"avcC", avcc));
            // BT.709, limited range — what the frames were converted to
            // before encoding — said outright, so no player has to guess.
            let mut colour = b"nclx".to_vec();
            colour.extend_from_slice(&1u16.to_be_bytes()); // primaries: BT.709
            colour.extend_from_slice(&1u16.to_be_bytes()); // transfer: BT.709
            colour.extend_from_slice(&1u16.to_be_bytes()); // matrix: BT.709
            colour.push(0); // limited range
            body.extend_from_slice(&boxed(b"colr", &colour));
            boxed(b"avc1", &body)
        }
        Mp4TrackKind::Aac {
            channels,
            sample_rate,
            config,
            bitrate,
        } => {
            let mut body = sound_entry(*channels, *sample_rate);
            body.extend_from_slice(&esds(config, *bitrate));
            boxed(b"mp4a", &body)
        }
        Mp4TrackKind::Opus { channels, pre_skip } => {
            let mut body = sound_entry(*channels, 48_000);
            let mut dops = vec![0, *channels as u8];
            dops.extend_from_slice(&pre_skip.to_be_bytes());
            dops.extend_from_slice(&48_000u32.to_be_bytes());
            dops.extend_from_slice(&[0, 0, 0]); // no gain, mapping family 0
            body.extend_from_slice(&boxed(b"dOps", &dops));
            boxed(b"Opus", &body)
        }
    };
    let mut body = 1u32.to_be_bytes().to_vec();
    body.extend_from_slice(&entry);
    full(b"stsd", 0, 0, &body)
}

/// The part every sound sample entry starts with.
fn sound_entry(channels: u16, sample_rate: u32) -> Vec<u8> {
    let mut body = vec![0; 6];
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&[0; 8]);
    body.extend_from_slice(&channels.to_be_bytes());
    body.extend_from_slice(&16u16.to_be_bytes());
    body.extend_from_slice(&[0; 4]);
    body.extend_from_slice(&(sample_rate << 16).to_be_bytes());
    body
}

/// An MPEG-4 descriptor: a tag, its length, its contents.
fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    // Length in seven-bit groups, high groups flagged; four always, which
    // every reader takes.
    let length = body.len() as u32;
    for shift in [21, 14, 7] {
        out.push(0x80 | ((length >> shift) & 0x7f) as u8);
    }
    out.push((length & 0x7f) as u8);
    out.extend_from_slice(body);
    out
}

fn esds(config: &[u8], bitrate: u32) -> Vec<u8> {
    let mut decoder = vec![0x40, 0x15, 0, 0, 0]; // AAC, sound stream, no buffer size
    decoder.extend_from_slice(&bitrate.to_be_bytes());
    decoder.extend_from_slice(&bitrate.to_be_bytes());
    decoder.extend_from_slice(&descriptor(0x05, config));

    let mut stream = vec![0, 0, 0]; // stream id, flags
    stream.extend_from_slice(&descriptor(0x04, &decoder));
    stream.extend_from_slice(&descriptor(0x06, &[0x02]));
    full(b"esds", 0, 0, &descriptor(0x03, &stream))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boxes directly inside `bytes`: kind, and where the body is.
    fn children(bytes: &[u8]) -> Vec<([u8; 4], std::ops::Range<usize>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 8 <= bytes.len() {
            let mut size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as u64;
            let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
            let mut header = 8;
            if size == 1 {
                size = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap());
                header = 16;
            }
            let end = at + size as usize;
            assert!(end <= bytes.len(), "{} runs past its parent", kind.escape_ascii());
            out.push((kind, at + header..end));
            at = end;
        }
        assert_eq!(at, bytes.len(), "boxes do not fill their parent");
        out
    }

    fn find<'a>(bytes: &'a [u8], path: &[&[u8; 4]]) -> &'a [u8] {
        let mut here = bytes;
        for kind in path {
            let (_, range) = children(here)
                .into_iter()
                .find(|(k, _)| k == *kind)
                .unwrap_or_else(|| panic!("no {}", kind.escape_ascii()));
            here = &here[range];
        }
        here
    }

    fn sample(byte: u8, len: usize, sync: bool) -> Mp4Sample {
        Mp4Sample {
            data: vec![byte; len],
            duration: 1001,
            sync,
        }
    }

    fn movie() -> Vec<Mp4Track> {
        vec![
            Mp4Track {
                kind: Mp4TrackKind::Video {
                    width: 1280,
                    height: 720,
                    avcc: vec![1, 0x64, 0, 0x1f, 0xff, 0xe0, 0],
                },
                timescale: 30_000,
                samples: vec![sample(1, 50, true), sample(2, 20, false), sample(3, 30, false)],
            },
            Mp4Track {
                kind: Mp4TrackKind::Aac {
                    channels: 2,
                    sample_rate: 48_000,
                    config: vec![0x11, 0x90],
                    bitrate: 192_000,
                },
                timescale: 48_000,
                samples: vec![
                    Mp4Sample {
                        data: vec![9; 7],
                        duration: 1024,
                        sync: true,
                    };
                    4
                ],
            },
        ]
    }

    #[test]
    fn the_file_is_the_index_then_the_samples() {
        let file = mux(&movie());
        let top: Vec<[u8; 4]> = children(&file).into_iter().map(|(k, _)| k).collect();
        assert_eq!(top, [*b"ftyp", *b"moov", *b"mdat"]);
        let traks = children(find(&file, &[b"moov"]))
            .into_iter()
            .filter(|(k, _)| k == b"trak")
            .count();
        assert_eq!(traks, 2);
    }

    #[test]
    fn every_offset_points_at_its_sample() {
        let tracks = movie();
        let file = mux(&tracks);
        let moov = find(&file, &[b"moov"]);
        let traks: Vec<_> = children(moov)
            .into_iter()
            .filter(|(k, _)| k == b"trak")
            .map(|(_, r)| &moov[r])
            .collect();
        for (trak, track) in traks.iter().zip(&tracks) {
            let co64 = find(trak, &[b"mdia", b"minf", b"stbl", b"co64"]);
            let count = u32::from_be_bytes(co64[4..8].try_into().unwrap()) as usize;
            assert_eq!(count, track.samples.len());
            for (n, sample) in track.samples.iter().enumerate() {
                let at = 8 + n * 8;
                let offset = u64::from_be_bytes(co64[at..at + 8].try_into().unwrap()) as usize;
                assert_eq!(&file[offset..offset + sample.data.len()], &sample.data[..]);
            }
        }
    }

    #[test]
    fn only_keyframes_are_marked_as_places_to_start() {
        let file = mux(&movie());
        let trak = find(&file, &[b"moov", b"trak"]);
        let stss = find(trak, &[b"mdia", b"minf", b"stbl", b"stss"]);
        assert_eq!(&stss[4..], &[0, 0, 0, 1, 0, 0, 0, 1]);
    }

    #[test]
    fn lengths_are_in_each_tracks_own_time() {
        let file = mux(&movie());
        let mvhd = find(&file, &[b"moov", b"mvhd"]);
        // 3 × 1001 / 30000 s = 100 ms; 4 × 1024 / 48000 s = 85 ms.
        let duration = u32::from_be_bytes(mvhd[16..20].try_into().unwrap());
        assert_eq!(duration, 100);
    }

    #[test]
    fn opus_is_described_for_a_reader() {
        let tracks = vec![Mp4Track {
            kind: Mp4TrackKind::Opus {
                channels: 2,
                pre_skip: 312,
            },
            timescale: 48_000,
            samples: vec![sample(5, 10, true)],
        }];
        let file = mux(&tracks);
        let stsd = find(&file, &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd"]);
        // Past the entry count: the one sample entry, an `Opus`.
        assert_eq!(&stsd[12..16], b"Opus");
        let dops = &stsd[8 + 8 + 28..];
        assert_eq!(&dops[4..8], b"dOps");
        assert_eq!(u16::from_be_bytes([dops[10], dops[11]]), 312);
    }
}
