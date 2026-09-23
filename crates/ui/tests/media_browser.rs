//! The media browser's search, kind filter and "remove unused".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::state::{MediaFilter, MediaSort, MediaView, media_name_matches};
use egui::{Pos2, RawInput, Rect, vec2};

#[test]
fn every_word_typed_must_be_in_the_name() {
    assert!(media_name_matches("Beach Sunset 04.MP4", ""));
    assert!(media_name_matches("Beach Sunset 04.MP4", "sunset"));
    assert!(media_name_matches("Beach Sunset 04.MP4", "04 beach"));
    assert!(!media_name_matches("Beach Sunset 04.MP4", "beach night"));
    assert!(MediaFilter::Photos.accepts(MediaKind::Image));
    assert!(!MediaFilter::Sound.accepts(MediaKind::Video));
    assert!(MediaFilter::All.accepts(MediaKind::Audio));
}

fn library() -> Editor {
    let (mut editor, _events) = Editor::new_project("Library");
    for (kind, name) in [
        (MediaKind::Video, "C:/media/beach sunset.mp4"),
        (MediaKind::Video, "C:/media/city night.mp4"),
        (MediaKind::Audio, "C:/media/sunset song.mp3"),
    ] {
        editor.import_media(MediaAsset::new(kind, name, MediaTime::from_seconds(5)));
    }
    editor
}

/// Draw the browser and return what it wrote.
fn words(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1200.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::panels::media_browser(ui, editor, state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push('\n');
            }
        }
    }
    words
}

#[test]
fn the_list_follows_the_search_and_the_kind() {
    let mut editor = library();
    let mut state = UiState::default();

    let all = words(&mut editor, &mut state);
    assert!(
        all.contains("city night.mp4") && all.contains("Remove 3 unused"),
        "{all}"
    );
    assert!(
        !all.contains("of 3 files"),
        "an unfiltered list said it was filtered"
    );

    state.media_search = "sunset".to_owned();
    let found = words(&mut editor, &mut state);
    assert!(found.contains("beach sunset.mp4") && found.contains("sunset song.mp3"));
    assert!(!found.contains("city night.mp4"), "{found}");
    assert!(found.contains("2 of 3 files"), "{found}");

    state.media_kind = MediaFilter::Sound;
    let sound = words(&mut editor, &mut state);
    assert!(sound.contains("sunset song.mp3") && !sound.contains("beach sunset.mp4"));

    state.media_search = "nothing like this".to_owned();
    assert!(words(&mut editor, &mut state).contains("No file matches"));
}

/// Skimming a thumbnail: the pointer's share of the width picks the tile and
/// the moment in the file; outside the thumbnail, or with no tiles, nothing.
#[test]
fn hovering_across_a_thumbnail_skims_the_file() {
    use bettercut_editor_core::foundation::MediaTime;
    use bettercut_ui::panels::scrub_at;

    let ten = MediaTime::from_seconds(10);
    assert_eq!(scrub_at(0.0, 20, ten), Some((0, MediaTime::ZERO)));
    assert_eq!(
        scrub_at(0.5, 20, ten),
        Some((10, MediaTime::from_seconds(5)))
    );
    assert_eq!(
        scrub_at(1.0, 20, ten),
        Some((19, ten)),
        "the last tile at the right edge"
    );
    assert_eq!(scrub_at(-0.1, 20, ten), None);
    assert_eq!(scrub_at(1.1, 20, ten), None);
    assert_eq!(scrub_at(0.5, 0, ten), None);
}

/// The order files are listed in, as the words the browser drew.
fn order(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.ends_with(".mp4") || line.ends_with(".mp3"))
        .collect()
}

#[test]
fn sorting_by_name_orders_the_list_and_pressing_it_again_turns_it_around() {
    let mut editor = library();
    let mut state = UiState::default();

    // As imported, to begin with.
    let added = words(&mut editor, &mut state);
    assert_eq!(
        order(&added),
        vec!["beach sunset.mp4", "city night.mp4", "sunset song.mp3"]
    );

    state.media_sort = MediaSort::Name;
    state.media_sort_reversed = true;
    let reversed = words(&mut editor, &mut state);
    assert_eq!(
        order(&reversed),
        vec!["sunset song.mp3", "city night.mp4", "beach sunset.mp4"]
    );
}

#[test]
fn sorting_by_kind_puts_the_pictures_before_the_sound() {
    let mut editor = library();
    let mut state = UiState::default();
    state.media_sort = MediaSort::Kind;
    let text = words(&mut editor, &mut state);
    assert_eq!(
        order(&text),
        vec!["beach sunset.mp4", "city night.mp4", "sunset song.mp3"]
    );

    state.media_sort_reversed = true;
    let text = words(&mut editor, &mut state);
    assert_eq!(order(&text)[0], "sunset song.mp3");
}

/// The compact list is for seeing more files at once, so it says the kind of
/// each and still offers the one action that matters — putting it on the
/// timeline.
#[test]
fn the_list_view_names_every_file_and_its_kind() {
    let mut editor = library();
    let mut state = UiState::default();
    state.media_view = MediaView::List;

    let text = words(&mut editor, &mut state);
    assert_eq!(
        order(&text),
        vec!["beach sunset.mp4", "city night.mp4", "sunset song.mp3"]
    );
    assert!(text.contains("Sound"), "{text}");
    assert!(text.contains("Video"), "{text}");
    // The card-only controls are not drawn.
    assert!(!text.contains("Add to timeline"), "{text}");
}

/// "Find in Media" on a clip: whatever the browser was showing, the file the
/// clip plays ends up on screen on its own.
#[test]
fn revealing_a_clips_file_clears_the_filters_and_searches_for_it() {
    let mut editor = library();
    let media = editor
        .project()
        .media
        .iter()
        .find(|asset| asset.file_name.contains("city night"))
        .map(|asset| (asset.id, asset.display_name().to_owned()))
        .unwrap();
    let clip = editor.place_media(media.0).unwrap()[0];
    let mut state = UiState::default();

    // Looking at something else entirely.
    state.media_kind = MediaFilter::Sound;
    state.media_search = "beach".to_owned();

    let found = editor.media_of_clip(clip).unwrap();
    assert_eq!(found, media.0);
    let name = editor
        .project()
        .media_asset(found)
        .unwrap()
        .display_name()
        .to_owned();
    state.reveal_media(found, &name);

    assert_eq!(state.media_kind, MediaFilter::All);
    assert_eq!(state.media_reveal, Some(media.0));
    let text = words(&mut editor, &mut state);
    assert!(text.contains("city night.mp4"), "{text}");
    assert!(!text.contains("beach sunset.mp4"), "{text}");
}

/// Marking a part of a file: the first click is the in-point, the second the
/// out-point, and a third starts again.
#[test]
fn two_clicks_mark_a_part_of_a_file() {
    use bettercut_editor_core::foundation::MediaTime;

    let editor = library();
    let media = editor.project().media[0].id;
    let mut state = UiState::default();
    assert_eq!(state.marked_part(media), None);

    // One click: an in-point, and nothing to place yet.
    assert_eq!(state.mark_media(media, MediaTime::from_seconds(2)), None);
    assert_eq!(state.marked_part(media), None);

    // The second click closes it.
    assert_eq!(
        state.mark_media(media, MediaTime::from_seconds(5)),
        Some((MediaTime::from_seconds(2), MediaTime::from_seconds(5)))
    );
    assert_eq!(
        state.marked_part(media),
        Some((MediaTime::from_seconds(2), MediaTime::from_seconds(5)))
    );

    // A third starts again rather than moving an end at random.
    assert_eq!(state.mark_media(media, MediaTime::from_seconds(9)), None);
    assert_eq!(state.marked_part(media), None);

    // Clicked the other way round, the part is still the span between them.
    state.clear_media_marks(media);
    state.mark_media(media, MediaTime::from_seconds(8));
    assert_eq!(
        state.mark_media(media, MediaTime::from_seconds(3)),
        Some((MediaTime::from_seconds(3), MediaTime::from_seconds(8)))
    );

    // And forgetting the marks places the whole file again.
    state.clear_media_marks(media);
    assert_eq!(state.marked_part(media), None);
}

/// The star filter shows only the takes worth keeping.
#[test]
fn the_star_filter_hides_the_unrated_takes() {
    let mut editor = library();
    let starred = editor.project().media[1].id;
    editor.set_media_rating(starred, 4).unwrap();
    let mut state = UiState::default();

    let all = words(&mut editor, &mut state);
    assert!(
        all.contains("beach sunset.mp4") && all.contains("city night.mp4"),
        "{all}"
    );

    state.media_stars = 3;
    let few = words(&mut editor, &mut state);
    assert!(
        few.contains("city night.mp4"),
        "the starred take is missing: {few}"
    );
    assert!(
        !few.contains("beach sunset.mp4"),
        "an unrated take is still shown: {few}"
    );

    // And the stars themselves are drawn, lit as far as the rating goes.
    assert!(few.contains('★'), "{few}");
}

/// "Not used yet" shows only the files nothing in the edit uses.
#[test]
fn the_unused_filter_hides_what_is_already_in_the_edit() {
    let mut editor = library();
    let used = editor.project().media[1].id;
    editor.place_media(used).unwrap();
    let mut state = UiState::default();

    state.media_unused_only = true;
    let left = words(&mut editor, &mut state);
    assert!(left.contains("beach sunset.mp4"), "{left}");
    assert!(
        !left.contains("city night.mp4"),
        "a used file is still shown: {left}"
    );
}
