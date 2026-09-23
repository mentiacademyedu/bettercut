//! The project aggregate (§8).

use bettercut_foundation::{MediaId, ProjectId, SequenceId};
use bettercut_media::MediaAsset;
use bettercut_timeline::Sequence;
use serde::{Deserialize, Serialize};

use crate::error::ProjectError;
use crate::settings::ProjectSettings;

/// Everything a project contains. Media references, timeline positions, and
/// settings — never media content (§2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,

    pub media: Vec<MediaAsset>,
    pub sequences: Vec<Sequence>,

    /// Colour lookup tables the project uses, by where they live on disk. The
    /// tables themselves are read from there, not copied in: a 33³ table is
    /// larger than the rest of a typical project put together.
    #[serde(default)]
    pub luts: Vec<LutAsset>,

    #[serde(default)]
    pub settings: ProjectSettings,

    /// Which sequence the UI is editing. `None` only for an empty project.
    #[serde(default)]
    pub active_sequence: Option<SequenceId>,

    /// Notes about the edit — what is left to do, what the client said —
    /// kept in the project file so they travel with it. Free text: a list
    /// with its own rules would be one more thing to learn. Defaulted: older
    /// projects have none.
    #[serde(default)]
    pub notes: String,
}

/// An imported `.cube` file (`bettercut_timeline::lut`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LutAsset {
    pub id: bettercut_foundation::LutId,
    /// What the menu calls it: the file's name, without the extension.
    pub name: String,
    pub path: std::path::PathBuf,
}

impl Project {
    /// A new project with one 1080p30 sequence, so the timeline is never empty
    /// on first launch.
    pub fn new(name: impl Into<String>) -> Self {
        let sequence = Sequence::default_hd();
        let active = sequence.id;
        Self {
            id: ProjectId::new(),
            name: name.into(),
            media: Vec::new(),
            sequences: vec![sequence],
            luts: Vec::new(),
            settings: ProjectSettings::default(),
            active_sequence: Some(active),
            notes: String::new(),
        }
    }

    pub fn sequence(&self, id: SequenceId) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }

    pub fn sequence_mut(&mut self, id: SequenceId) -> Option<&mut Sequence> {
        self.sequences.iter_mut().find(|s| s.id == id)
    }

    /// The sequence the UI is editing, falling back to the first one.
    pub fn active(&self) -> Option<&Sequence> {
        self.active_sequence
            .and_then(|id| self.sequence(id))
            .or_else(|| self.sequences.first())
    }

    pub fn active_mut(&mut self) -> Option<&mut Sequence> {
        // Resolve the id first so the borrow of `self` ends before the mutable one.
        let id = self.active().map(|s| s.id)?;
        self.sequence_mut(id)
    }

    pub fn lut_asset(&self, id: bettercut_foundation::LutId) -> Option<&LutAsset> {
        self.luts.iter().find(|lut| lut.id == id)
    }

    pub fn media_asset(&self, id: MediaId) -> Option<&MediaAsset> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn media_asset_mut(&mut self, id: MediaId) -> Option<&mut MediaAsset> {
        self.media.iter_mut().find(|m| m.id == id)
    }

    /// Add an asset, or return the existing one if this path is already imported.
    ///
    /// Re-importing the same file must not duplicate it, or its proxies and
    /// thumbnails get generated twice (§19).
    ///
    /// A *made* entry — a colour clip, a tone — has no file, so it cannot be
    /// the same file as anything: two of them are one entry only when they
    /// are the same made thing. Matching them by their (empty) path would
    /// hand a beep the black picture's id, which is what once happened.
    pub fn add_media(&mut self, asset: MediaAsset) -> MediaId {
        let same = |m: &MediaAsset| {
            if asset.path.as_os_str().is_empty() {
                m.path.as_os_str().is_empty()
                    && m.generated == asset.generated
                    && m.generated_sound == asset.generated_sound
            } else {
                m.path == asset.path
            }
        };
        if let Some(existing) = self.media.iter().find(|m| same(m)) {
            return existing.id;
        }
        let id = asset.id;
        self.media.push(asset);
        id
    }

    /// Remove an asset. Refused while any clip still references it — silently
    /// dropping it would leave clips pointing at nothing.
    pub fn remove_media(&mut self, id: MediaId) -> Result<MediaAsset, ProjectError> {
        if self.media_is_used(id) {
            return Err(ProjectError::MediaInUse(id));
        }
        let index = self
            .media
            .iter()
            .position(|m| m.id == id)
            .ok_or(ProjectError::MediaNotFound(id))?;
        Ok(self.media.remove(index))
    }

    pub fn media_is_used(&self, id: MediaId) -> bool {
        self.sequences.iter().any(|s| {
            s.video_tracks
                .iter()
                .any(|t| t.clips().iter().any(|c| c.media_id == id))
                || s.audio_tracks
                    .iter()
                    .any(|t| t.clips().iter().any(|c| c.media_id == id))
                || s.watermark.is_some_and(|w| w.media == id)
        })
    }

    /// Mark assets whose files have gone missing (§66). Returns how many.
    pub fn refresh_missing_media(&mut self) -> usize {
        let mut missing = 0;
        for asset in &mut self.media {
            asset.missing = !asset.exists();
            if asset.missing {
                missing += 1;
            }
        }
        missing
    }

    pub fn clip_count(&self) -> usize {
        self.sequences.iter().map(Sequence::clip_count).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaTime, TimelineTime};
    use bettercut_media::MediaKind;
    use bettercut_timeline::{SourceRange, VideoClip};

    /// A made picture and a made sound have no file, and so are never the
    /// same file: each keeps its own entry. The same made thing twice is one.
    #[test]
    fn made_entries_are_told_apart_by_what_they_are() {
        let mut project = Project::new("Made");
        let black = project.add_media(MediaAsset::generated(
            bettercut_media::Generated::solid([0, 0, 0]),
            16,
            9,
        ));
        let beep = project.add_media(MediaAsset::generated_sound(
            bettercut_media::GeneratedSound::LINE_UP,
            bettercut_foundation::MediaTime::from_seconds(1),
        ));
        let black_again = project.add_media(MediaAsset::generated(
            bettercut_media::Generated::solid([0, 0, 0]),
            16,
            9,
        ));
        let red = project.add_media(MediaAsset::generated(
            bettercut_media::Generated::solid([255, 0, 0]),
            16,
            9,
        ));

        assert_ne!(black, beep, "a beep was handed the black picture's entry");
        assert_eq!(black, black_again, "the same black twice made two entries");
        assert_ne!(black, red);
        assert_eq!(project.media.len(), 3);
    }

    fn asset(path: &str) -> MediaAsset {
        MediaAsset::new(MediaKind::Video, path, MediaTime::from_seconds(10))
    }

    #[test]
    fn a_new_project_has_one_active_sequence() {
        let p = Project::new("Untitled");
        assert_eq!(p.sequences.len(), 1);
        assert!(p.active().is_some());
        assert_eq!(p.active_sequence, Some(p.sequences[0].id));
        assert_eq!(p.clip_count(), 0);
    }

    #[test]
    fn importing_the_same_path_twice_reuses_the_asset() {
        let mut p = Project::new("p");
        let first = p.add_media(asset("C:/media/a.mp4"));
        let second = p.add_media(asset("C:/media/a.mp4"));
        assert_eq!(first, second);
        assert_eq!(p.media.len(), 1);
    }

    #[test]
    fn media_in_use_cannot_be_removed() {
        let mut p = Project::new("p");
        let media_id = p.add_media(asset("C:/media/a.mp4"));

        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).expect("valid");
        let clip = VideoClip::new(media_id, TimelineTime::ZERO, source).expect("valid");
        p.active_mut().expect("has sequence").video_tracks[0]
            .insert(clip)
            .expect("no overlap");

        assert!(p.media_is_used(media_id));
        assert!(matches!(
            p.remove_media(media_id),
            Err(ProjectError::MediaInUse(_))
        ));
    }

    #[test]
    fn unused_media_can_be_removed() {
        let mut p = Project::new("p");
        let id = p.add_media(asset("C:/media/a.mp4"));
        assert!(p.remove_media(id).is_ok());
        assert!(p.media.is_empty());
        assert!(matches!(
            p.remove_media(id),
            Err(ProjectError::MediaNotFound(_))
        ));
    }

    #[test]
    fn missing_media_is_detected() {
        let mut p = Project::new("p");
        p.add_media(asset("C:/definitely/not/here.mp4"));
        assert_eq!(p.refresh_missing_media(), 1);
        assert!(p.media[0].missing);
    }
}
