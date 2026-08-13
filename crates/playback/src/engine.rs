//! The playback engine (§47a).
//!
//! Turns "what time is it?" into "which pixels and which samples?".
//!
//! # The clock runs the show
//!
//! §20a.1: the audio device advances the clock, the clock chooses the video
//! frame, and late frames are dropped rather than queued. Nothing here consults
//! a wall-clock timer to decide what to show.
//!
//! # Where frames come from
//!
//! Three places, checked in order: the frame cache (§18), then §47a.3's
//! decode-ahead ring, then an inline decode. The ring is filled by a thread
//! started when playback starts and stopped when it stops — a paused editor has
//! nothing to run ahead of, and holding a decode thread for it works against
//! §81's idle budget.
//!
//! Scrubbing still decodes inline, which is correct: the user is jumping around
//! rather than moving forward, so there is nothing to predict. That path is
//! what §13.1's all-intra proxies make cheap.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_media::{FfmpegDecoder, MediaAsset, NeverCancelled, SeekMode, VideoFrame};
use bettercut_project_format::Project;
use bettercut_timeline::{Clip, Sequence, Transform};

use crate::audio_source::AudioSource;
use crate::cache::{FrameCache, FrameKey};
use crate::error::PlaybackError;
use crate::frame_source::FrameSource;

/// One video layer to draw, resolved for a given instant.
pub struct ResolvedLayer {
    pub clip: ClipId,
    pub track: TrackId,
    pub frame: Arc<VideoFrame>,
    pub transform: Transform,
    pub opacity: f32,
    pub color: bettercut_timeline::ColorAdjust,
    /// §45's blur amount, 0–100.
    pub blur: f32,
}

/// One audio clip audible at a given instant.
#[derive(Debug, Clone, Copy)]
pub struct AudibleClip {
    pub clip: ClipId,
    pub media: MediaId,
    /// Where in the source the audible span starts.
    pub source_start: MediaTime,
    pub gain: f32,
    /// Offset from the start of the requested block, in timeline ticks.
    pub offset: TimelineTime,
}

/// Samples pushed to the device per round. 480 frames is 10 ms at 48 kHz:
/// small enough that a seek takes effect promptly, large enough that the
/// per-block overhead is irrelevant.
const AUDIO_BLOCK_FRAMES: usize = 480;

/// Where preview frames are read from (§14).
///
/// > Editing uses proxy media. Export uses original media.
///
/// Held as runtime state rather than in the project, because whether a proxy
/// exists is a fact about *this machine's* cache. Storing it in the `.vproj`
/// would mean a project opened on another computer claims proxies that are not
/// there — and §66's missing-media handling would have to cover cache files
/// too, for no benefit.
#[derive(Clone)]
pub struct ProxySource {
    pub cache: std::sync::Arc<bettercut_cache::CacheStore>,
    pub height: u32,
}

/// Two sources are the same when they name the same cache and height.
///
/// Compared by pointer rather than by contents: a `CacheStore` has no
/// meaningful value equality, and the question being asked is "is this the same
/// source I already configured?", which pointer identity answers exactly.
impl PartialEq for ProxySource {
    fn eq(&self, other: &Self) -> bool {
        self.height == other.height && std::sync::Arc::ptr_eq(&self.cache, &other.cache)
    }
}

impl ProxySource {
    /// The proxy for `media`, if the cache actually holds one.
    pub(crate) fn path_for(&self, media: MediaId) -> Option<std::path::PathBuf> {
        let path = self.cache.layout().proxy_file(media, self.height);
        path.exists().then_some(path)
    }
}

pub struct PlaybackEngine {
    /// The decoder pool. The *same* type the decode-ahead thread runs, which
    /// is the point: this used to be a second, hand-rolled copy of it here,
    /// and the copy is what let §47a.2's sequential-read rule be implemented
    /// in one of the two places and missed in the other.
    frames: FrameSource,
    /// Kept only to size the decode-ahead budget; the pool owns the rest.
    proxy: Option<ProxySource>,
    cache: FrameCache,
    /// FFmpeg threads per decoder (§15.1), from `HardwareProfile`.
    decoder_threads: u32,

    /// Audio decoders, kept separate from the video ones.
    ///
    /// A decoder owns one demuxer with one read position. Sharing it between
    /// the video path (which seeks to wherever the playhead is) and the audio
    /// path (which streams forward) would make them fight over that position,
    /// and the symptom would be stuttering sound.
    audio_sources: HashMap<MediaId, AudioSource>,
    /// Timeline position audio has been pushed up to.
    audio_filled_to: TimelineTime,
    master_gain: f32,
    limited_samples: u64,

    /// The decode-ahead thread (§47a.3). `None` until playback starts, because
    /// a paused editor has nothing to decode ahead of.
    prefetcher: Option<crate::prefetcher::Prefetcher>,
    /// Frames served from the ring rather than decoded inline — the number
    /// that says whether decode-ahead is doing anything.
    prefetch_hits: u64,
}

impl PlaybackEngine {
    pub fn new(cache_bytes: usize, decoder_threads: u32) -> Self {
        Self {
            frames: FrameSource::new(decoder_threads),
            proxy: None,
            cache: FrameCache::new(cache_bytes),
            decoder_threads: decoder_threads.max(1),
            audio_sources: HashMap::new(),
            audio_filled_to: TimelineTime::ZERO,
            master_gain: 1.0,
            limited_samples: 0,
            prefetcher: None,
            prefetch_hits: 0,
        }
    }

    /// Start decoding ahead of the playhead (§47a.3).
    ///
    /// Called when playback starts. Idempotent — starting twice would run two
    /// decode threads over the same files, which §74 forbids and which would be
    /// slower than one.
    pub fn start_prefetch(&mut self, budget_bytes: usize) {
        if self.prefetcher.is_some() {
            return;
        }
        self.prefetcher = Some(crate::prefetcher::Prefetcher::start(
            budget_bytes,
            self.decoder_threads,
        ));
    }

    /// Stop decoding ahead. Called when playback stops: a paused editor should
    /// not hold a decode thread or its share of the frame budget (§81).
    pub fn stop_prefetch(&mut self) {
        self.prefetcher = None;
    }

    pub fn prefetch_hits(&self) -> u64 {
        self.prefetch_hits
    }

    /// Frames currently waiting in the ring.
    pub fn prefetched_frames(&self) -> usize {
        self.prefetcher.as_ref().map_or(0, |p| p.buffer().len())
    }

    /// Abandon decode-ahead work and start a new generation (§47a.5).
    ///
    /// Called on any seek: the frames queued for the old position are worthless
    /// and the bytes they hold are needed for the new one.
    pub fn reset_prefetch(&mut self) {
        if let Some(prefetcher) = self.prefetcher.as_ref() {
            prefetcher.reset();
        }
    }

    /// Queue the next `span` of frames for the decode thread (§47a.3).
    ///
    /// Cheap: it walks the visible tracks at frame intervals and resolves which
    /// media each instant needs. No decoding happens here — that is the whole
    /// point — so this is safe to call every UI frame while playing.
    pub fn prefetch_ahead(
        &mut self,
        project: &Project,
        sequence: &Sequence,
        from: TimelineTime,
        span: TimelineTime,
    ) {
        let Some(prefetcher) = self.prefetcher.as_ref() else {
            return;
        };

        let interval = sequence.ticks_per_frame().max(1);
        let generation = prefetcher.generation();
        let mut items = Vec::new();

        let mut tick = from.ticks();
        let end = from.ticks().saturating_add(span.ticks());
        while tick < end {
            let at = TimelineTime::from_ticks(tick);
            for track in &sequence.video_tracks {
                if !track.enabled {
                    continue;
                }
                let Some(clip) = track.clip_at(at) else {
                    continue;
                };
                let Some(asset) = project.media_asset(clip.media_id) else {
                    continue;
                };
                let source = source_time_of(clip.timeline().start, clip.source().start, at);
                let key = FrameKey {
                    media: asset.id,
                    timestamp: source,
                };
                // Skip what is already decoded: re-planning every frame would
                // otherwise ask for the same second of work sixty times a
                // second.
                if self.cache.contains(&key) || prefetcher.buffer().contains(&key) {
                    continue;
                }
                items.push((asset.clone(), source));
            }
            tick = tick.saturating_add(interval);
        }

        if items.is_empty() {
            return;
        }

        prefetcher.submit(crate::prefetcher::Plan {
            generation,
            proxy: self.proxy.clone(),
            items,
        });
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain.clamp(0.0, 4.0);
    }

    /// Samples the limiter had to clamp (§20a.4). Non-zero means the mix is
    /// genuinely too hot and the user should be told, not just left to hear it.
    pub fn limited_samples(&self) -> u64 {
        self.limited_samples
    }

    /// Restart audio from `position`.
    ///
    /// Called on seek and on play. Without it the engine would keep filling
    /// from wherever it had got to and the sound would lag the picture by the
    /// whole buffer depth.
    pub fn reset_audio(&mut self, position: TimelineTime) {
        self.audio_filled_to = position;
    }

    /// Top the device buffer up from the sequence's audio tracks.
    ///
    /// Returns how many frames were pushed. Called every UI frame while
    /// playing; the ring buffer holds 150 ms, so a UI frame or two of jitter is
    /// absorbed without a gap.
    ///
    /// **Interim placement.** §20a.2's diagram puts this on a dedicated mixer
    /// thread. It runs on the UI thread for now because the project lives
    /// there and §54 makes the editor core its sole owner; moving it needs a
    /// snapshot of the audible clips rather than the project itself. The rules
    /// that actually protect the sound - no allocation, no locks, no blocking
    /// in the *callback* - are already satisfied, and an underrun here is
    /// counted and audible rather than silent corruption.
    pub fn fill_audio(
        &mut self,
        project: &Project,
        sequence: &Sequence,
        sink: &mut bettercut_audio::AudioSink,
    ) -> usize {
        let channels = sink.channels();
        if channels == 0 {
            return 0;
        }

        let block_ticks = TimelineTime::from_ticks(
            AUDIO_BLOCK_FRAMES as i64 * bettercut_foundation::TICKS_PER_AUDIO_SAMPLE,
        );
        let mut pushed = 0;
        let mut interleaved = vec![0.0_f32; AUDIO_BLOCK_FRAMES * channels];

        while sink.vacant_frames() >= AUDIO_BLOCK_FRAMES {
            interleaved.fill(0.0);
            let block_start = self.audio_filled_to;

            for audible in Self::resolve_audio(sequence, block_start, block_ticks) {
                let Some(asset) = project.media_asset(audible.media) else {
                    continue;
                };
                let Ok(source) = self.audio_source_for(asset) else {
                    continue;
                };

                let offset_frames = (audible.offset.ticks()
                    / bettercut_foundation::TICKS_PER_AUDIO_SAMPLE)
                    as usize;
                let wanted = AUDIO_BLOCK_FRAMES.saturating_sub(offset_frames);
                if wanted == 0 {
                    continue;
                }

                match source.read(audible.source_start, wanted) {
                    Ok(planes) if !planes.is_empty() => {
                        bettercut_audio::mix_into(
                            &mut interleaved,
                            channels,
                            &planes,
                            offset_frames,
                            bettercut_audio::MixParams {
                                clip_gain: audible.gain,
                                // Track gain and pan arrive with the mixer UI;
                                // the stage exists in the §20a.4 order already.
                                ..Default::default()
                            },
                        );
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(%err, "audio read failed; that clip is silent");
                    }
                }
            }

            // §20a.4's final stage: master gain, then the limiter.
            self.limited_samples +=
                bettercut_audio::finish(&mut interleaved, self.master_gain) as u64;

            let accepted = sink.push(&interleaved);
            pushed += accepted / channels;
            self.audio_filled_to += block_ticks;

            if accepted < interleaved.len() {
                // The device did not take the whole block; stop rather than
                // spin. Next frame will find room.
                break;
            }
        }

        pushed
    }

    fn audio_source_for(&mut self, asset: &MediaAsset) -> Result<&mut AudioSource, PlaybackError> {
        let threads = self.decoder_threads;
        match self.audio_sources.entry(asset.id) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let decoder = FfmpegDecoder::new(threads)?;
                let source = AudioSource::open(asset, Box::new(decoder))?;
                Ok(entry.insert(source))
            }
        }
    }

    pub fn cache(&self) -> &FrameCache {
        &self.cache
    }

    pub fn last_seek_error(&self) -> TimelineTime {
        TimelineTime::from_ticks(self.frames.last_seek_error().ticks())
    }

    /// Drop cached frames and decoders for one asset.
    /// Read preview frames from proxies where they exist (§14).
    pub fn set_proxy_source(&mut self, proxy: Option<ProxySource>) {
        self.proxy = proxy.clone();
        // Every open decoder points at whichever copy was current when it was
        // opened, and every cached frame came from one of them.
        self.frames.set_proxy_source(proxy);
        self.cache.clear();
        // Frames already decoded ahead came from the old copy too.
        self.reset_prefetch();
    }

    /// Forget everything cached or open for one asset.
    ///
    /// Called when a proxy finishes encoding (§13): the frames already cached
    /// are from the original and still correct, but the point is to start using
    /// the proxy, and keeping both wastes the budget.
    pub fn invalidate(&mut self, media: MediaId) {
        self.cache.invalidate_media(media);
        self.frames.invalidate(media);
        self.audio_sources.remove(&media);
    }

    pub fn set_cache_bytes(&mut self, bytes: usize) {
        self.cache.set_max_bytes(bytes);
    }

    /// Everything visible at `position`, bottom track first (§22).
    ///
    /// A track that fails to decode is **skipped, not fatal**: §50 requires a
    /// media failure to mark the clip unavailable and let the session continue.
    /// One corrupt file must not black out the other tracks.
    pub fn resolve_video(
        &mut self,
        project: &Project,
        sequence: &Sequence,
        position: TimelineTime,
    ) -> Vec<ResolvedLayer> {
        let mut layers = Vec::new();

        for track in &sequence.video_tracks {
            if !track.enabled {
                continue; // hidden tracks are skipped by the renderer (§8)
            }
            let Some(clip) = track.clip_at(position) else {
                continue;
            };
            let Some(asset) = project.media_asset(clip.media_id) else {
                continue;
            };

            let source_time = source_time_of(clip.timeline().start, clip.source().start, position);
            // §24: animated parameters override the static ones, resolved in
            // the timeline crate so preview and export cannot disagree (§46).
            let look = clip.look_at(source_time);

            match self.frame_at(asset, source_time) {
                Ok(frame) => layers.push(ResolvedLayer {
                    clip: clip.id,
                    track: track.id,
                    frame,
                    transform: look.transform,
                    opacity: look.opacity,
                    color: look.color,
                    blur: look.blur,
                }),
                Err(err) => {
                    tracing::warn!(
                        file = %asset.file_name,
                        %err,
                        "could not decode a frame; skipping this track"
                    );
                }
            }
        }

        layers
    }

    /// Which audio clips are audible in `[position, position + duration)`.
    ///
    /// Returns descriptions rather than samples: the mixer thread owns the
    /// decoding, and this runs wherever the project happens to be borrowed.
    pub fn resolve_audio(
        sequence: &Sequence,
        position: TimelineTime,
        duration: TimelineTime,
    ) -> Vec<AudibleClip> {
        let end = position + duration;
        let mut audible = Vec::new();

        for track in &sequence.audio_tracks {
            if !track.enabled {
                continue; // muted (§8)
            }
            let range = bettercut_timeline::TimelineRange {
                start: position,
                end,
            };
            for clip in track.clips_in_range(range) {
                // Where this clip begins inside the requested block.
                let offset = if clip.timeline.start > position {
                    clip.timeline.start - position
                } else {
                    TimelineTime::ZERO
                };
                let from = position.max(clip.timeline.start);
                let source_start = source_time_of(clip.timeline.start, clip.source.start, from);

                audible.push(AudibleClip {
                    clip: clip.id,
                    media: clip.media_id,
                    source_start,
                    gain: clip.gain,
                    offset,
                });
            }
        }

        audible
    }

    /// Decode (or recall) the frame of `asset` at `source_time`.
    fn frame_at(
        &mut self,
        asset: &MediaAsset,
        source_time: MediaTime,
    ) -> Result<Arc<VideoFrame>, PlaybackError> {
        let key = FrameKey {
            media: asset.id,
            timestamp: source_time,
        };
        if let Some(frame) = self.cache.get(&key) {
            return Ok(frame);
        }

        // §47a.3: the decode-ahead thread may already have this. Taking it
        // frees its bytes back to the ring, which is what releases the decode
        // thread to work further ahead.
        if let Some(prefetcher) = self.prefetcher.as_ref()
            && let Some(frame) = prefetcher.buffer().take(&key)
        {
            self.prefetch_hits += 1;
            self.cache.insert(key, Arc::clone(&frame));
            return Ok(frame);
        }

        // Nothing had it, so decode here and now — on the UI thread, which is
        // why keeping this cheap matters. `FrameSource` reads forward without
        // seeking when the frame wanted is simply the next one (§47a.2), and
        // falls back to a `Precise` seek when the playhead has actually jumped.
        //
        // Scrubbing will want `Scrub` here once §17's quality ladder is wired
        // up; `Precise` is the right fallback for stepping and for stopping.
        let frame = self
            .frames
            .decode(asset, source_time, SeekMode::Precise, &NeverCancelled)?;

        // Key the cache by the frame's *actual* timestamp as well, so a later
        // request landing on the same frame is a hit even if it asks for a
        // slightly different instant.
        self.cache.insert(key, Arc::clone(&frame));
        Ok(frame)
    }
}

/// Map a timeline instant onto a position inside a clip's source media.
///
/// No speed change in the MVP (§59), so a tick on the timeline is a tick in the
/// source and this is a straight offset.
pub fn source_time_of(
    timeline_start: TimelineTime,
    source_start: MediaTime,
    position: TimelineTime,
) -> MediaTime {
    let into_clip = position.ticks() - timeline_start.ticks();
    MediaTime::from_ticks(source_start.ticks() + into_clip.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_at_the_clip_start_maps_to_the_source_in_point() {
        let mapped = source_time_of(
            TimelineTime::from_seconds(10),
            MediaTime::from_seconds(3),
            TimelineTime::from_seconds(10),
        );
        assert_eq!(mapped, MediaTime::from_seconds(3));
    }

    #[test]
    fn moving_along_the_timeline_moves_equally_far_into_the_source() {
        let mapped = source_time_of(
            TimelineTime::from_seconds(10),
            MediaTime::from_seconds(3),
            TimelineTime::from_seconds(14),
        );
        assert_eq!(
            mapped,
            MediaTime::from_seconds(7),
            "timeline and source advanced by different amounts"
        );
    }

    /// A trimmed clip shows later material, and the mapping has to follow the
    /// in-point rather than assuming the clip starts at the media's start.
    #[test]
    fn a_trimmed_clip_maps_from_its_in_point() {
        let mapped = source_time_of(
            TimelineTime::ZERO,
            MediaTime::from_seconds(60),
            TimelineTime::from_seconds(5),
        );
        assert_eq!(mapped, MediaTime::from_seconds(65));
    }

    #[test]
    fn a_position_before_the_clip_clamps_to_its_in_point() {
        let mapped = source_time_of(
            TimelineTime::from_seconds(10),
            MediaTime::from_seconds(3),
            TimelineTime::ZERO,
        );
        assert_eq!(
            mapped,
            MediaTime::from_seconds(3),
            "mapped to before the start of the media"
        );
    }
}
