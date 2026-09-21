//! Mixing the timeline's audio, off the UI thread (§20a.2).
//!
//! ```text
//! Decode threads → resample → mixer thread → ring buffer → cpal callback
//! ```
//!
//! ## Why a thread of its own
//!
//! The device's ring buffer holds 150 ms. Mixing used to run on the UI thread,
//! once per frame, which is fine until the UI thread stops — and on Windows it
//! stops whenever the window is dragged or resized, because the message loop
//! blocks for as long as the mouse is held. A hundred and fifty milliseconds
//! later the sound cuts out. Decoding audio also ran there, so a slow decode
//! showed up as a dropped video frame. §20a.2's diagram has always put this on
//! a thread of its own.
//!
//! ## What the thread may know
//!
//! §54 makes the editor core the sole owner of the project, so the mixer cannot
//! read it. It gets an [`AudioPlan`] instead: a snapshot of the audio tracks and
//! the assets they read, sent whenever it changes. Cloning a few dozen clips is
//! microseconds; the alternative — sharing the project behind a lock — would
//! put a lock between the UI and the sound, which is how audio glitches under
//! load are made.
//!
//! ## One implementation, two callers
//!
//! [`AudioMixer::mix_block`] is what both the preview's thread and the export
//! call (§46). Export used to carry its own copy of the loop, and the copies had
//! already diverged: the preview resampled a sped-up clip and the export
//! did not, so an exported 2× clip played its sound at normal speed and cut it
//! off half way. One function cannot disagree with itself.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use bettercut_foundation::{MediaId, MediaTime, TICKS_PER_AUDIO_SAMPLE, TimelineTime};
use bettercut_media::{FfmpegDecoder, MediaAsset};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioTrack, Sequence};

use crate::audio_source::AudioSource;
use crate::engine::resolve_audio_tracks;

/// Samples pushed to the device per round. 480 frames is 10 ms at 48 kHz:
/// small enough that a seek takes effect promptly, large enough that the
/// per-block overhead is irrelevant.
pub const BLOCK_FRAMES: usize = 480;

/// Everything the mixer needs to know about the project, and nothing else.
///
/// A snapshot, because §54 lets nothing but the editor core touch the project
/// and the mixer runs on another thread. Compared with `PartialEq` so the
/// preview sends a new one only when the audio actually changed — dragging a
/// video clip's opacity slider is a project change, and not one the mixer
/// needs to hear about sixty times a second.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlan {
    pub tracks: Vec<AudioTrack>,
    /// Only the assets the tracks read, by id.
    pub assets: HashMap<MediaId, MediaAsset>,
    /// §20a.4's master gain. In the plan, so the preview's thread and the
    /// export apply the same number from the same place (§46).
    pub master_volume: f32,
}

impl Default for AudioPlan {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            assets: HashMap::new(),
            master_volume: 1.0,
        }
    }
}

impl AudioPlan {
    pub fn of(project: &Project, sequence: &Sequence) -> Self {
        let mut tracks = sequence.audio_tracks.clone();
        // A compound clip's own sound, laid out where the compound plays. The
        // mixer never learns what a compound is: it is handed ordinary tracks
        // of ordinary clips (`crate::compound`).
        tracks.extend(crate::compound::audio_tracks(project, sequence, 0));
        let assets = tracks
            .iter()
            .flat_map(|t| t.clips())
            .filter_map(|c| project.media_asset(c.media_id))
            .map(|asset| (asset.id, asset.clone()))
            .collect();
        Self {
            tracks,
            assets,
            master_volume: sequence.master_volume,
        }
    }
}

/// The mixing itself: owns the audio decoders, knows nothing of threads.
///
/// Decoders are kept apart from the video ones, as they always were: one
/// decoder has one read position, and the picture seeking while the sound
/// streams forward would fight over it.
pub struct AudioMixer {
    /// Each decoder beside the path it was opened on, so a relinked file
    /// (§66) is noticed — the id stays the same when the path changes.
    sources: HashMap<MediaId, (AudioSource, std::path::PathBuf)>,
    decoder_threads: u32,
    /// Each cleaned clip's clean-up, with the amount it was made for and the
    /// timeline tick its next sample belongs at. A jump — a seek, a scrub, a
    /// loop — or a changed amount starts it afresh; continuous playback and an
    /// export carry it on, sample for sample.
    cleaners: HashMap<bettercut_foundation::ClipId, (bettercut_audio::VoiceCleaner, u32, i64)>,
    /// Each equalised clip's filters, with the settings they were made for and
    /// the tick their next sample belongs at — started afresh on a jump or a
    /// changed setting, exactly as the clean-ups are.
    equalizers: HashMap<bettercut_foundation::ClipId, (bettercut_audio::Equalizer, [u32; 4], i64)>,
    /// Each clip's echo or reverb, the same way: its delay lines carry on
    /// through continuous playback and start empty after a jump.
    /// Each voice-changed clip's pitch shifter, the same way again.
    pitchers: HashMap<bettercut_foundation::ClipId, (bettercut_audio::PitchShifter, u32, i64)>,
    /// Each levelled clip's compressor, the same way.
    levellers: HashMap<bettercut_foundation::ClipId, (bettercut_audio::Leveller, u32, i64)>,
    /// Each de-essed clip's band-pass and detector, the same way again.
    de_essers: HashMap<bettercut_foundation::ClipId, (bettercut_audio::DeEsser, u32, i64)>,
    spaces: HashMap<
        bettercut_foundation::ClipId,
        (bettercut_audio::Space, bettercut_timeline::ClipSpace, i64),
    >,
    /// The loudest sample each sound lane put into the last block, per side.
    ///
    /// Gathered as the block is mixed rather than measured afterwards: once
    /// the lanes are summed there is no way back to which of them was loud,
    /// and re-mixing a lane on its own to find out would be a second mix that
    /// could disagree with the first (§46).
    lane_peaks: Vec<(f32, f32)>,
}

impl AudioMixer {
    pub fn new(decoder_threads: u32) -> Self {
        Self {
            sources: HashMap::new(),
            decoder_threads: decoder_threads.max(1),
            cleaners: HashMap::new(),
            equalizers: HashMap::new(),
            spaces: HashMap::new(),
            pitchers: HashMap::new(),
            levellers: HashMap::new(),
            de_essers: HashMap::new(),
            lane_peaks: Vec::new(),
        }
    }

    /// Drop decoders the plan no longer justifies.
    ///
    /// A clip deleted from the timeline leaves its decoder open, holding a file
    /// handle and FFmpeg's buffers, for as long as the mixer lives; and a file
    /// relinked to a new path keeps being read from the old one. The plan is the
    /// only thing the mixer knows about the project, so it is where both are
    /// noticed.
    pub fn retain_current(&mut self, plan: &AudioPlan) {
        self.sources.retain(|media, (_, opened_at)| {
            plan.assets
                .get(media)
                .is_some_and(|asset| &asset.path == opened_at)
        });
    }

    /// Mix `frames` of the plan's audio, starting at `position`, into `out`
    /// (interleaved, `channels` wide). Adds to `out`; the caller zeroes it.
    ///
    /// Master gain and the limiter are the caller's, applied once to the whole
    /// block — both callers take the gain from `plan.master_volume`.
    pub fn mix_block(
        &mut self,
        plan: &AudioPlan,
        position: TimelineTime,
        frames: usize,
        channels: usize,
        out: &mut [f32],
    ) {
        if channels == 0 || frames == 0 {
            return;
        }
        let duration = TimelineTime::from_ticks(frames as i64 * TICKS_PER_AUDIO_SAMPLE);

        // This block's readings only: a meter holding a level from a block
        // that has already been heard is a meter telling the truth about
        // the past.
        for lane in &mut self.lane_peaks {
            *lane = (0.0, 0.0);
        }
        for audible in resolve_audio_tracks(&plan.tracks, position, duration) {
            let Some(asset) = plan.assets.get(&audible.media) else {
                continue; // §66: missing media is silence, not a failure
            };
            let offset = (audible.offset.ticks() / TICKS_PER_AUDIO_SAMPLE) as usize;
            // Up to the end of the block — or of the clip, when that comes
            // first. Read to the block's end, a clip ending mid-block went on
            // sounding past its own end, over whatever followed it: up to a
            // tenth of a second in an export, whose blocks are that long.
            let wanted = frames
                .saturating_sub(offset)
                .min(usize::try_from(audible.fades.remaining).unwrap_or(0));
            if wanted == 0 {
                continue;
            }

            // at 2× the block needs twice as many source frames,
            // resampled back down to the number asked for. `input_frames_needed`
            // includes the one extra frame the last interpolation reads.
            let rate = audible.speed.as_f64();
            let to_read = if audible.speed.is_one() {
                wanted
            } else {
                bettercut_audio::input_frames_needed(wanted, rate)
            };

            // Backwards: the same number of frames, read from the window that
            // *ends* where the span begins, then turned round — so the first
            // sample played is the latest one, and it runs back from there.
            let read_from = if audible.reversed {
                MediaTime::from_ticks(
                    (audible.source_start.ticks() - to_read as i64 * TICKS_PER_AUDIO_SAMPLE).max(0),
                )
            } else {
                audible.source_start
            };
            // Made sound has no file to open: the samples come from the
            // source time asked for (`bettercut_media::generated_sound`), so a
            // tone is the same sound played, scrubbed or exported.
            let read = match asset.generated_sound {
                Some(sound) => Ok(sound.read(read_from, to_read)),
                None => match self.source_for(asset) {
                    Some(source) => source.read(read_from, to_read),
                    None => continue,
                },
            };
            match read {
                Ok(mut planes) if !planes.is_empty() => {
                    if audible.reversed {
                        for plane in &mut planes {
                            plane.reverse();
                        }
                    }
                    let mut planes = if audible.speed.is_one() {
                        planes
                    } else {
                        bettercut_audio::resample(&planes, wanted, rate)
                    };
                    // The channels first: every stage after this should hear
                    // the voice in both ears, not an empty side.
                    audible.channels.apply(&mut planes);
                    // The voice changed next, so the tone controls shape the
                    // voice as it will be heard.
                    // The voice changer, plus — when the clip asks for its
                    // pitch held — as many semitones back as the re-timing
                    // moved it. One shifter does both: they are the same
                    // operation, and running two would grain the sound twice.
                    let corrected = if audible.keep_pitch && !audible.speed.is_one() {
                        -12.0 * (audible.speed.as_f64() as f32).log2()
                    } else {
                        0.0
                    };
                    let shift = audible.pitch + corrected;
                    if shift != 0.0 {
                        let starts_at = position.ticks() + audible.offset.ticks();
                        let bits = shift.to_bits();
                        let fresh = match self.pitchers.get(&audible.clip) {
                            Some((_, setting, next)) => *setting != bits || *next != starts_at,
                            None => true,
                        };
                        if fresh && let Some(shifter) = bettercut_audio::PitchShifter::new(shift) {
                            self.pitchers
                                .insert(audible.clip, (shifter, bits, starts_at));
                        }
                        if let Some((shifter, _, next)) = self.pitchers.get_mut(&audible.clip) {
                            shifter.process(&mut planes);
                            let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
                            *next = starts_at + frames as i64 * TICKS_PER_AUDIO_SAMPLE;
                        }
                    }
                    // The tone first, then the clean-up listening to it.
                    if !audible.eq.is_flat() {
                        let starts_at = position.ticks() + audible.offset.ticks();
                        let eq = audible.eq;
                        let key = [
                            eq.low_cut.to_bits(),
                            eq.high_cut.to_bits(),
                            eq.presence.to_bits(),
                            eq.hum.to_bits(),
                        ];
                        let fresh = match self.equalizers.get(&audible.clip) {
                            Some((_, settings, next)) => *settings != key || *next != starts_at,
                            None => true,
                        };
                        if fresh
                            && let Some(equalizer) = bettercut_audio::Equalizer::with_hum(
                                eq.low_cut,
                                eq.high_cut,
                                eq.presence,
                                eq.hum,
                            )
                        {
                            self.equalizers
                                .insert(audible.clip, (equalizer, key, starts_at));
                        }
                        if let Some((equalizer, _, next)) = self.equalizers.get_mut(&audible.clip) {
                            equalizer.process(&mut planes);
                            let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
                            *next = starts_at + frames as i64 * TICKS_PER_AUDIO_SAMPLE;
                        }
                    }
                    if audible.denoise > 0.0 {
                        let starts_at = position.ticks() + audible.offset.ticks();
                        let bits = audible.denoise.to_bits();
                        let fresh = match self.cleaners.get(&audible.clip) {
                            Some((_, amount, next)) => *amount != bits || *next != starts_at,
                            None => true,
                        };
                        if fresh
                            && let Some(cleaner) =
                                bettercut_audio::VoiceCleaner::new(audible.denoise)
                        {
                            self.cleaners
                                .insert(audible.clip, (cleaner, bits, starts_at));
                        }
                        if let Some((cleaner, _, next)) = self.cleaners.get_mut(&audible.clip) {
                            cleaner.process(&mut planes);
                            let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
                            *next = starts_at + frames as i64 * TICKS_PER_AUDIO_SAMPLE;
                        }
                    }
                    // Levelled after the clean-up, so the background it pulled
                    // down is not lifted back up, and before the room.
                    if audible.leveller > 0.0 {
                        let starts_at = position.ticks() + audible.offset.ticks();
                        let bits = audible.leveller.to_bits();
                        let fresh = match self.levellers.get(&audible.clip) {
                            Some((_, setting, next)) => *setting != bits || *next != starts_at,
                            None => true,
                        };
                        if fresh
                            && let Some(leveller) = bettercut_audio::Leveller::new(audible.leveller)
                        {
                            self.levellers
                                .insert(audible.clip, (leveller, bits, starts_at));
                        }
                        if let Some((leveller, _, next)) = self.levellers.get_mut(&audible.clip) {
                            leveller.process(&mut planes);
                            let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
                            *next = starts_at + frames as i64 * TICKS_PER_AUDIO_SAMPLE;
                        }
                    }
                    // De-essed after the leveller, because the leveller is
                    // what lifts an ess into a problem: dipping first would
                    // leave it to be raised again.
                    if audible.de_ess > 0.0 {
                        let starts_at = position.ticks() + audible.offset.ticks();
                        let bits = audible.de_ess.to_bits();
                        let fresh = match self.de_essers.get(&audible.clip) {
                            Some((_, setting, next)) => *setting != bits || *next != starts_at,
                            None => true,
                        };
                        if fresh
                            && let Some(de_esser) = bettercut_audio::DeEsser::new(audible.de_ess)
                        {
                            self.de_essers
                                .insert(audible.clip, (de_esser, bits, starts_at));
                        }
                        if let Some((de_esser, _, next)) = self.de_essers.get_mut(&audible.clip) {
                            de_esser.process(&mut planes);
                            let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
                            *next = starts_at + frames as i64 * TICKS_PER_AUDIO_SAMPLE;
                        }
                    }
                    // The space last: a room around the cleaned-up voice, not
                    // a clean-up fighting the room's tail.
                    if !audible.space.is_dry() {
                        let starts_at = position.ticks() + audible.offset.ticks();
                        let space = audible.space;
                        let fresh = match self.spaces.get(&audible.clip) {
                            Some((_, settings, next)) => *settings != space || *next != starts_at,
                            None => true,
                        };
                        let place = match space.kind {
                            bettercut_timeline::SpaceKind::Echo => {
                                Some(bettercut_audio::Place::Echo)
                            }
                            bettercut_timeline::SpaceKind::Room => {
                                Some(bettercut_audio::Place::Room)
                            }
                            bettercut_timeline::SpaceKind::Hall => {
                                Some(bettercut_audio::Place::Hall)
                            }
                            bettercut_timeline::SpaceKind::Dry => None,
                        };
                        if fresh
                            && let Some(place) = place
                            && let Some(processor) = bettercut_audio::Space::new(place, space.mix)
                        {
                            self.spaces
                                .insert(audible.clip, (processor, space, starts_at));
                        }
                        if let Some((processor, _, next)) = self.spaces.get_mut(&audible.clip) {
                            processor.process(&mut planes);
                            let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
                            *next = starts_at + frames as i64 * TICKS_PER_AUDIO_SAMPLE;
                        }
                    }
                    let heard = bettercut_audio::mix_into(
                        out,
                        channels,
                        &planes,
                        offset,
                        bettercut_audio::MixParams {
                            clip_gain: audible.gain,
                            fades: audible.fades,
                            automation: audible.automation,
                            track_gain: audible.track_gain,
                            track_automation: audible.track_automation,
                            track_pan: audible.track_pan,
                        },
                    );
                    if self.lane_peaks.len() <= audible.lane {
                        self.lane_peaks.resize(audible.lane + 1, (0.0, 0.0));
                    }
                    // The loudest of the lane's clips, not the last of them:
                    // two clips crossfading are both that lane, and a meter
                    // that showed only the second would dip through every cut.
                    let lane = &mut self.lane_peaks[audible.lane];
                    *lane = (lane.0.max(heard.0), lane.1.max(heard.1));
                }
                Ok(_) => {}
                Err(err) => {
                    // §50: one clip that will not decode is silent, not fatal.
                    tracing::warn!(%err, "audio read failed; that clip is silent");
                }
            }
        }
    }

    /// What each sound lane put into the last block mixed, per side, in lane
    /// order. Shorter than the sequence's lanes when the ones below it were
    /// silent, which is what a lane with nothing under the playhead is.
    pub fn lane_peaks(&self) -> &[(f32, f32)] {
        &self.lane_peaks
    }

    fn source_for(&mut self, asset: &MediaAsset) -> Option<&mut AudioSource> {
        let threads = self.decoder_threads;
        match self.sources.entry(asset.id) {
            std::collections::hash_map::Entry::Occupied(entry) => Some(&mut entry.into_mut().0),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let decoder = FfmpegDecoder::new(threads).ok()?;
                match AudioSource::open(asset, Box::new(decoder)) {
                    Ok(source) => Some(&mut entry.insert((source, asset.path.clone())).0),
                    Err(err) => {
                        tracing::warn!(file = %asset.file_name, %err, "could not open audio");
                        None
                    }
                }
            }
        }
    }

    /// How many decoders are open. Diagnostics and tests.
    pub fn open_sources(&self) -> usize {
        self.sources.len()
    }
}

enum Message {
    Plan(Arc<AudioPlan>),
    Seek(TimelineTime),
    Playing(bool),
    /// Play a short burst from here: the playhead is being dragged (§10's
    /// scrub). Only while stopped — during playback the sound is already the
    /// answer to where the playhead is.
    Scrub(TimelineTime),
}

/// How much sound one scrub burst is. Long enough to recognise a word or a
/// beat, short enough that dragging does not lag behind the hand.
const SCRUB_MS: usize = 80;

/// §20a.2's mixer thread, feeding the device's ring buffer.
///
/// Owns the [`bettercut_audio::AudioSink`] — the producer side of the ring —
/// and an [`AudioMixer`]. Talks to the preview only through messages, so there
/// is no lock anywhere between the UI and the sound.
pub struct MixerThread {
    sender: Option<Sender<Message>>,
    handle: Option<JoinHandle<()>>,
    limited: Arc<AtomicU64>,
    pushed: Arc<AtomicU64>,
    /// The last block's peak level per side, as `f32` bits in an atomic.
    ///
    /// Atomics rather than a lock because the mixer thread must never wait on
    /// the interface (§20a.2, §54), and a meter that misses a block is a meter
    /// that is one block stale — which nobody can see.
    peak_left: Arc<AtomicU32>,
    peak_right: Arc<AtomicU32>,
    /// The mix's loudness over the last three seconds and the last 400 ms, in
    /// LUFS, as `f32` bits — `f32::NAN` for "nothing to report", which is what
    /// silence and a stopped mixer both are (`bettercut_audio::loudness`).
    short_term: Arc<AtomicU32>,
    momentary: Arc<AtomicU32>,
    /// What each sound lane is putting into the mix, for the meters in the
    /// track heads (`crate::lane_meters`).
    lanes: Arc<crate::lane_meters::LaneMeters>,
}

impl MixerThread {
    /// Start the thread. It idles, blocked on its channel, until told to play.
    pub fn spawn(sink: bettercut_audio::AudioSink, decoder_threads: u32) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let limited = Arc::new(AtomicU64::new(0));
        let pushed = Arc::new(AtomicU64::new(0));
        let peak_left = Arc::new(AtomicU32::new(0));
        let peak_right = Arc::new(AtomicU32::new(0));
        let short_term = Arc::new(AtomicU32::new(f32::NAN.to_bits()));
        let momentary = Arc::new(AtomicU32::new(f32::NAN.to_bits()));
        let lanes = Arc::new(crate::lane_meters::LaneMeters::new());

        let handle = {
            let limited = Arc::clone(&limited);
            let pushed = Arc::clone(&pushed);
            let left = Arc::clone(&peak_left);
            let right = Arc::clone(&peak_right);
            let short = Arc::clone(&short_term);
            let moment = Arc::clone(&momentary);
            let lane_meters = Arc::clone(&lanes);
            std::thread::Builder::new()
                .name("bettercut-mixer".to_owned())
                .spawn(move || {
                    run(
                        receiver,
                        sink,
                        decoder_threads,
                        Reports {
                            limited: &limited,
                            pushed: &pushed,
                            peaks: (&left, &right),
                            loudness: (&short, &moment),
                            lanes: &lane_meters,
                        },
                    );
                })?
        };

        Ok(Self {
            sender: Some(sender),
            handle: Some(handle),
            limited,
            pushed,
            peak_left,
            peak_right,
            short_term,
            momentary,
            lanes,
        })
    }

    /// Replace what the thread mixes from.
    pub fn set_plan(&self, plan: AudioPlan) {
        self.send(Message::Plan(Arc::new(plan)));
    }

    /// Mix from `position` next. Called on play and on every seek, or the
    /// sound would carry on from wherever the mixer had got to.
    pub fn seek(&self, position: TimelineTime) {
        self.send(Message::Seek(position));
    }

    pub fn set_playing(&self, playing: bool) {
        self.send(Message::Playing(playing));
    }

    /// Play a short burst of the mix from `position` — what makes dragging the
    /// playhead audible (§10's scrub).
    ///
    /// Ignored while playing: the sound is already saying where the playhead
    /// is, and a burst over the top of it would be two mixes at once.
    pub fn scrub(&self, position: TimelineTime) {
        self.send(Message::Scrub(position));
    }

    /// §20a.4: samples the limiter had to clamp. Non-zero means the mix is too
    /// hot.
    pub fn limited_samples(&self) -> u64 {
        self.limited.load(Ordering::Relaxed)
    }

    /// What each sound lane put into the last block, per side, in lane order
    /// (`crate::lane_meters`). All silence while playback is stopped.
    pub fn lane_peaks(&self) -> Vec<(f32, f32)> {
        self.lanes.all()
    }

    /// The last block's peak level per side, 0.0 to 1.0 or beyond.
    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_right.load(Ordering::Relaxed)),
        )
    }

    /// What the mix is measuring right now: the last three seconds and the
    /// last 400 ms, in LUFS. `None` where there is nothing to report — the
    /// mixer is stopped, or what is playing is below the standard's silence
    /// gate.
    pub fn loudness(&self) -> (Option<f32>, Option<f32>) {
        let read = |slot: &AtomicU32| {
            let value = f32::from_bits(slot.load(Ordering::Relaxed));
            value.is_finite().then_some(value)
        };
        (read(&self.short_term), read(&self.momentary))
    }

    /// Frames pushed to the device since the thread started. Diagnostics, and
    /// how a test tells a running mixer from one that is only alive.
    pub fn frames_pushed(&self) -> u64 {
        self.pushed.load(Ordering::Relaxed)
    }

    fn send(&self, message: Message) {
        if let Some(sender) = &self.sender
            && sender.send(message).is_err()
        {
            // The thread is gone — it only exits on its own if it panicked.
            // Nothing the caller can do; the preview carries on silent rather
            // than taking the editor down with it (§50).
            tracing::error!("the audio mixer thread has stopped");
        }
    }
}

impl Drop for MixerThread {
    fn drop(&mut self) {
        // Dropping the sender ends the thread's `recv`, which is its signal to
        // stop. Joined, so a new mixer never runs alongside the old one.
        self.sender.take();
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!("the audio mixer thread panicked");
        }
    }
}

/// Everything the mixer thread tells the interface, all of it atomic.
///
/// One bundle rather than seven arguments, because they travel together and
/// always will: every one of them is a number the thread writes and nobody
/// waits on (§20a.2, §54).
struct Reports<'a> {
    limited: &'a AtomicU64,
    pushed: &'a AtomicU64,
    /// The mix's peak level per side, as `f32` bits.
    peaks: (&'a AtomicU32, &'a AtomicU32),
    /// Short-term and momentary loudness, as `f32` bits.
    loudness: (&'a AtomicU32, &'a AtomicU32),
    /// What each sound lane is putting in (`crate::lane_meters`).
    lanes: &'a crate::lane_meters::LaneMeters,
}

fn run(
    receiver: Receiver<Message>,
    mut sink: bettercut_audio::AudioSink,
    decoder_threads: u32,
    reports: Reports<'_>,
) {
    let Reports {
        limited,
        pushed,
        peaks,
        loudness,
        lanes,
    } = reports;
    let mut mixer = AudioMixer::new(decoder_threads);
    let mut plan = Arc::new(AudioPlan::default());
    let mut playing = false;
    let mut filled_to = TimelineTime::ZERO;
    // The live meter, made once: §20a.2 forbids allocating on this thread,
    // and this is the only place its ring is sized.
    let mut meter = bettercut_audio::loudness::LiveLoudness::new(2);
    let report = |meter: &bettercut_audio::loudness::LiveLoudness| {
        let store = |slot: &AtomicU32, value: Option<f32>| {
            slot.store(value.unwrap_or(f32::NAN).to_bits(), Ordering::Relaxed);
        };
        store(loudness.0, meter.short_term());
        store(loudness.1, meter.momentary());
    };
    // Where the last scrub asked to be heard from, if one is waiting.
    let mut scrub_at: Option<TimelineTime> = None;

    let channels = sink.channels();
    // Allocated once, here. The real-time rule against allocating (§20a.2)
    // binds the device callback, not this thread — but a mixer that allocates
    // per block is one that stutters under memory pressure for no reason.
    let mut block = vec![0.0_f32; BLOCK_FRAMES * channels.max(1)];
    let block_ticks = TimelineTime::from_ticks(BLOCK_FRAMES as i64 * TICKS_PER_AUDIO_SAMPLE);

    loop {
        // Paused: block until told something, at no cost. Playing: wake often
        // enough that the 150 ms ring never runs low — every 5 ms is thirty
        // chances to top it up before it could empty.
        let first = if playing {
            match receiver.recv_timeout(Duration::from_millis(5)) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match receiver.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            }
        };

        // Everything queued, in order: a seek sent after a plan must land
        // after it.
        for message in first
            .into_iter()
            .chain(std::iter::from_fn(|| receiver.try_recv().ok()))
        {
            match message {
                Message::Plan(next) => {
                    mixer.retain_current(&next);
                    plan = next;
                }
                Message::Seek(position) => filled_to = position,
                Message::Playing(now) => playing = now,
                Message::Scrub(position) => scrub_at = Some(position),
            }
        }

        if !playing || channels == 0 {
            // Silence reads as silence: a meter frozen at the last level of a
            // paused mix looks like sound that is not there.
            peaks.0.store(0.0_f32.to_bits(), Ordering::Relaxed);
            peaks.1.store(0.0_f32.to_bits(), Ordering::Relaxed);
            // Nor is any lane: one left holding its last block is a lane that
            // looks like it is still playing.
            lanes.silence();
            // And a stopped mixer is measuring nothing, rather than still
            // showing what was playing when it stopped.
            meter.reset();
            report(&meter);

            // A scrub: one short burst from where the playhead now is. Only
            // the *latest* one — a fast drag asks for dozens a second, and
            // playing every one of them would fall further behind the hand
            // with each.
            if let Some(position) = scrub_at.take()
                && channels > 0
            {
                let frames = SCRUB_MS * sink.sample_rate() as usize / 1000;
                // Nothing queued but this: the ring is otherwise idle while
                // stopped, and a backlog of bursts is exactly what makes a
                // scrub feel detached from the pointer.
                if sink.vacant_frames() >= frames {
                    let mut burst = vec![0.0_f32; frames * channels];
                    mixer.mix_block(&plan, position, frames, channels, &mut burst);
                    let clamped = bettercut_audio::finish(&mut burst, plan.master_volume) as u64;
                    if clamped > 0 {
                        limited.fetch_add(clamped, Ordering::Relaxed);
                    }
                    // Faded in and out, or every burst starts and ends with a
                    // click — which is all a listener would hear.
                    let fade = (frames / 8).max(1);
                    for frame in 0..frames {
                        let gain = (frame as f32 / fade as f32)
                            .min((frames - frame) as f32 / fade as f32)
                            .clamp(0.0, 1.0);
                        for channel in 0..channels {
                            burst[frame * channels + channel] *= gain;
                        }
                    }
                    let (left, right) = bettercut_audio::peaks(&burst, channels);
                    peaks.0.store(left.to_bits(), Ordering::Relaxed);
                    peaks.1.store(right.to_bits(), Ordering::Relaxed);
                    lanes.publish(mixer.lane_peaks());
                    sink.push(&burst);
                }
            }
            continue;
        }
        // Playing: whatever scrub was asked for is answered by the playback
        // itself.
        scrub_at = None;

        while sink.vacant_frames() >= BLOCK_FRAMES {
            block.fill(0.0);
            mixer.mix_block(&plan, filled_to, BLOCK_FRAMES, channels, &mut block);

            // §20a.4's final stage: master gain, then the limiter.
            let clamped = bettercut_audio::finish(&mut block, plan.master_volume) as u64;
            if clamped > 0 {
                limited.fetch_add(clamped, Ordering::Relaxed);
            }

            let (left, right) = bettercut_audio::peaks(&block, channels);
            peaks.0.store(left.to_bits(), Ordering::Relaxed);
            peaks.1.store(right.to_bits(), Ordering::Relaxed);
            // The lanes that made that block, before it is summed away.
            lanes.publish(mixer.lane_peaks());

            // The same block the device is about to hear, through the meter:
            // what is reported is what is played (§46).
            meter.push_interleaved(&block, channels);
            report(&meter);

            let accepted = sink.push(&block);
            pushed.fetch_add((accepted / channels) as u64, Ordering::Relaxed);
            filled_to += block_ticks;

            if accepted < block.len() {
                // The device has not drained yet. Try again on the next wake
                // rather than spinning here.
                break;
            }
        }
    }
}
