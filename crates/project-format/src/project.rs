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

    #[serde(default)]
    pub settings: ProjectSettings,

    /// Which sequence the UI is editing. `None` only for an empty project.
    #[serde(default)]
    pub active_sequence: Option<SequenceId>,
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
            settings: ProjectSettings::default(),
            active_sequence: Some(active),
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
    pub fn add_media(&mut self, asset: MediaAsset) -> MediaId {
        if let Some(existing) = self.media.iter().find(|m| m.path == asset.path) {
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
