//! A lower third over each selected picture, reading its name: the location
//! card on each shot of a travel video, the speaker on each answer of an
//! interview. Placed in one step and retyped afterwards.

use bettercut_foundation::ClipId;
use bettercut_timeline::TimelineRange;

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Put a lower-third title over each picture clip in `clips`, as long as
    /// the clip and reading its name (its own, or its file's). Clips whose
    /// stretch of the title lane is already taken are skipped. One undo
    /// step; returns how many titles went in.
    pub fn title_each_clip(&mut self, clips: &[ClipId]) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let active = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let lane = active.text_tracks.first().ok_or(EditorError::NoTextTrack)?;
        let track = lane.id;
        let mut taken: Vec<TimelineRange> = lane.clips().iter().map(|c| c.timeline).collect();

        let mut pictures: Vec<(ClipId, TimelineRange, String)> = clips
            .iter()
            .filter_map(|clip| {
                let picture = self.video_clip(*clip)?;
                let name = self.clip_name(*clip).unwrap_or_else(|| {
                    self.project()
                        .media_asset(picture.media_id)
                        .map_or_else(|| "Title".to_owned(), |m| m.display_name().to_owned())
                });
                Some((*clip, picture.timeline, name))
            })
            .collect();
        pictures.sort_by_key(|(_, span, _)| span.start);
        pictures.dedup_by_key(|(clip, _, _)| *clip);

        let mut commands = Vec::new();
        for (_, span, name) in pictures {
            if taken.iter().any(|t| t.overlaps(span)) {
                continue;
            }
            let mut title =
                bettercut_timeline::TextClip::with_duration(name, span.start, span.duration())?;
            title.style = bettercut_text::TextStyle::title(bettercut_text::TitleLook::LowerThird);
            self.fit_to_frame(&mut title);
            taken.push(span);
            commands.push(Command::AddText {
                sequence,
                track,
                clip: Box::new(title),
            });
        }
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Title Each Clip", commands)?;
        }
        Ok(count)
    }
}
