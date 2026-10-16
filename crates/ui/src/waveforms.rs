//! Loaded waveforms, ready for the timeline to draw (§12, §53).
//!
//! The same shape as [`crate::thumbnails::ThumbnailStore`], and for the same
//! reason: the timeline redraws at 60 fps while scrolling, so it must not touch
//! the filesystem per clip per frame. Both the answer and the *absence* of one
//! are remembered, and the absence is cleared when analysis reports success.
//!
//! Waveforms stay in RAM as plain peaks rather than becoming textures — the
//! timeline draws them as line segments into its own `Painter`, so there is
//! nothing to upload.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_cache::{CacheStore, PEAKS_PER_SECOND, Peak, Waveform};
use bettercut_editor_core::foundation::{AUDIO_SAMPLE_RATE, MediaId, MediaTime};
use bettercut_editor_core::media::{GeneratedSound, MediaAsset};

#[derive(Default)]
pub struct WaveformStore {
    cache: Option<Arc<CacheStore>>,
    /// `None` means "looked, not there yet".
    loaded: HashMap<MediaId, Option<Arc<Waveform>>>,
    /// Sounds made rather than read, and what each was when its peaks were
    /// worked out ([`Self::note_generated`]).
    generated: HashMap<MediaId, (GeneratedSound, MediaTime)>,
    /// The web build: where each file the page was given is, so its peaks
    /// are read from the page instead of the cache.
    #[cfg(target_arch = "wasm32")]
    browser_files: HashMap<MediaId, std::path::PathBuf>,
}

impl WaveformStore {
    pub fn attach(&mut self, cache: Arc<CacheStore>) {
        self.cache = Some(cache);
        self.loaded.clear();
        self.generated.clear();
    }

    /// Analysis finished, so a remembered absence is now wrong.
    pub fn invalidate(&mut self, media: MediaId) {
        self.loaded.remove(&media);
        self.generated.remove(&media);
    }

    /// Sounds made rather than read — a tone, a sound effect — have no file
    /// to analyse, so their peaks are worked out from the sound itself: once
    /// each, and again when the sound or its length is changed. Cheap enough
    /// to call every frame with every asset.
    pub fn note_generated(&mut self, media: &[MediaAsset]) {
        for asset in media {
            let Some(sound) = asset.generated_sound else {
                continue;
            };
            let made = (sound, asset.duration);
            if self.generated.get(&asset.id) == Some(&made) {
                continue;
            }
            self.generated.insert(asset.id, made);
            self.loaded.insert(
                asset.id,
                generated_peaks(sound, asset.duration).map(Arc::new),
            );
        }
    }

    /// The peaks for an asset, loading them on first request.
    ///
    /// `None` while analysis has not finished — the caller draws a plain block,
    /// which is what the timeline looked like before waveforms existed.
    pub fn get(&mut self, media: MediaId) -> Option<Arc<Waveform>> {
        if !self.loaded.contains_key(&media) {
            let loaded = self.load(media);
            self.loaded.insert(media, loaded);
        }
        self.loaded.get(&media).and_then(Clone::clone)
    }

    /// The web build: `media` is the page's file at `path`, whose peaks the
    /// page works out. Every "not yet" is forgotten, so whatever has arrived
    /// since is picked up.
    #[cfg(target_arch = "wasm32")]
    pub fn browser_file(&mut self, media: MediaId, path: &std::path::Path) {
        self.browser_files.insert(media, path.to_path_buf());
        self.loaded.retain(|_, waveform| waveform.is_some());
    }

    fn load(&self, media: MediaId) -> Option<Arc<Waveform>> {
        #[cfg(target_arch = "wasm32")]
        if let Some(path) = self.browser_files.get(&media) {
            let peaks = bettercut_editor_core::media::browser::peaks(path)?;
            let peaks = peaks
                .chunks_exact(2)
                .map(|pair| bettercut_cache::Peak {
                    min: pair[0],
                    max: pair[1],
                })
                .collect();
            return Waveform::new(bettercut_cache::PEAKS_PER_SECOND, peaks)
                .ok()
                .map(Arc::new);
        }
        let cache = self.cache.as_ref()?;
        let path = cache.layout().waveform_file(media);
        if !path.exists() {
            return None;
        }

        match Waveform::read(&path) {
            Ok(waveform) => Some(Arc::new(waveform)),
            Err(err) => {
                // A corrupt entry is regenerable; drop it and move on.
                tracing::warn!(%err, path = %path.display(), "discarding a bad waveform");
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.values().filter(|v| v.is_some()).count()
    }
}

/// The peaks of a made sound, `length` long. A tone or silence is the same
/// level all the way, so it is drawn flat; an effect is short, and its own
/// samples are read.
fn generated_peaks(sound: GeneratedSound, length: MediaTime) -> Option<Waveform> {
    let seconds = length.as_seconds_f64().max(0.0);
    let count = (seconds * f64::from(PEAKS_PER_SECOND)).ceil() as usize;
    let level = |amplitude: f32| (amplitude.clamp(0.0, 1.0) * 127.0).round() as i8;
    let peaks = match sound.natural_length() {
        None => {
            let height = level(sound.amplitude());
            vec![
                Peak {
                    min: -height,
                    max: height,
                };
                count
            ]
        }
        Some(natural) => {
            let per_peak = (AUDIO_SAMPLE_RATE / i64::from(PEAKS_PER_SECOND)).max(1) as usize;
            let sounding = (natural.min(seconds) * f64::from(PEAKS_PER_SECOND)).ceil() as usize;
            let samples = sound.read(MediaTime::ZERO, sounding * per_peak);
            let plane = samples.first().map_or(&[][..], Vec::as_slice);
            (0..count)
                .map(|index| {
                    let from = (index * per_peak).min(plane.len());
                    let to = ((index + 1) * per_peak).min(plane.len());
                    plane[from..to]
                        .iter()
                        .fold(Peak::default(), |peak, sample| Peak {
                            min: peak.min.min(-level(-sample)),
                            max: peak.max.max(level(*sample)),
                        })
                })
                .collect()
        }
    };
    Waveform::new(PEAKS_PER_SECOND, peaks).ok()
}

impl std::fmt::Debug for WaveformStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaveformStore")
            .field("known", &self.loaded.len())
            .field("loaded", &self.loaded_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::media::MediaKind;

    fn made(sound: GeneratedSound, seconds: i64) -> MediaAsset {
        let mut asset = MediaAsset::new(MediaKind::Audio, "", MediaTime::from_seconds(seconds));
        asset.generated_sound = Some(sound);
        asset
    }

    #[test]
    fn a_tone_is_drawn_flat_at_its_level() {
        let asset = made(GeneratedSound::LINE_UP, 3);
        let mut store = WaveformStore::default();
        store.note_generated(std::slice::from_ref(&asset));
        let waveform = store.get(asset.id).expect("peaks for a tone");
        assert_eq!(waveform.peaks.len(), 3 * PEAKS_PER_SECOND as usize);
        let height = waveform.peaks[100].max;
        assert!(height > 10, "the line-up tone, at -18 dB, drew {height}");
        assert!(
            waveform
                .peaks
                .iter()
                .all(|peak| peak.max == height && peak.min == -height)
        );
    }

    #[test]
    fn an_effect_sounds_then_falls_silent() {
        let asset = made(GeneratedSound::Pop, 1);
        let mut store = WaveformStore::default();
        store.note_generated(std::slice::from_ref(&asset));
        let waveform = store.get(asset.id).expect("peaks for an effect");
        assert!(waveform.peaks[2].magnitude() > 0.05, "the pop drew nothing");
        assert_eq!(
            waveform.peaks[150].magnitude(),
            0.0,
            "silence after the pop drew something"
        );
    }

    #[test]
    fn a_changed_sound_is_drawn_again() {
        let mut asset = made(GeneratedSound::LINE_UP, 1);
        let mut store = WaveformStore::default();
        store.note_generated(std::slice::from_ref(&asset));
        let loud = store.get(asset.id).unwrap().peaks[10].max;
        asset.generated_sound = Some(GeneratedSound::Tone {
            hz: 1_000,
            level_db: -40.0,
        });
        store.note_generated(std::slice::from_ref(&asset));
        assert!(store.get(asset.id).unwrap().peaks[10].max < loud);
    }
}
