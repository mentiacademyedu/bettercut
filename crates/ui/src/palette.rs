//! The command palette (Ctrl+K): type a few letters of an action, Enter runs
//! it.
//!
//! The menus have grown long, and a thing that can only be found by
//! right-clicking the right spot is a thing a new user never finds. Here
//! every common action is one search away, whatever is under the mouse.
//! Actions that need a selection say so and do nothing without one.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, TimelineTime};

use crate::shortcuts;
use crate::state::UiState;
use crate::theme;

/// One thing the palette can do.
pub struct Action {
    pub name: &'static str,
    pub hint: &'static str,
    pub run: fn(&mut Editor, &mut UiState),
}

fn selected(state: &UiState) -> Vec<ClipId> {
    state.selected_clips.iter().copied().collect()
}

/// Report a count-returning edit: nothing, how many, or the error.
fn report(
    state: &mut UiState,
    result: Result<usize, bettercut_editor_core::EditorError>,
    none: &str,
    done: &str,
) {
    match result {
        Ok(0) => state.info(none),
        Ok(n) => {
            state.needs_repaint = true;
            state.info(format!("{done}: {n}"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Everything the palette offers, in the order shown before anything is typed.
pub const ACTIONS: &[Action] = &[
    Action {
        name: "Split at Playhead",
        hint: "S",
        run: |e, s| shortcuts::split_at_playhead(e, s),
    },
    Action {
        name: "Split Every Lane at Playhead",
        hint: "Shift+S",
        run: |e, s| {
            let under = e.clips_under_playhead();
            report(
                s,
                e.split_at_playhead(&under),
                "Nothing under the playhead",
                "Split",
            );
        },
    },
    Action {
        name: "Undo",
        hint: "Ctrl+Z",
        run: |e, s| {
            if let Err(err) = e.undo() {
                s.info(err.to_string())
            }
        },
    },
    Action {
        name: "Redo",
        hint: "Ctrl+Shift+Z",
        run: |e, s| {
            if let Err(err) = e.redo() {
                s.info(err.to_string())
            }
        },
    },
    Action {
        name: "Save",
        hint: "Ctrl+S",
        run: |e, s| crate::panels::save_project(e, s),
    },
    Action {
        name: "Export",
        hint: "the export window",
        run: |e, s| s.export_dialog.open(e),
    },
    Action {
        name: "Select All",
        hint: "Ctrl+A",
        run: |e, s| shortcuts::select_from(e, s, TimelineTime::ZERO, None),
    },
    Action {
        name: "Invert Selection",
        hint: "everything not selected",
        run: |e, s| shortcuts::invert_selection(e, s),
    },
    Action {
        name: "Delete Selection",
        hint: "Delete",
        run: |e, s| shortcuts::delete_selection(e, s),
    },
    Action {
        name: "Duplicate Selection",
        hint: "Ctrl+D",
        run: |e, s| shortcuts::duplicate_selection(e, s),
    },
    Action {
        name: "Close Gaps on Every Lane",
        hint: "butt every clip up",
        run: |e, s| report(s, e.close_gaps_everywhere(), "No gaps", "Gaps closed"),
    },
    Action {
        name: "Clean Up Slivers",
        hint: "clips and gaps under two frames",
        run: |e, s| match e.clean_up_slivers(2) {
            Ok((0, 0)) => s.info("No slivers"),
            Ok((clips, gaps)) => {
                s.clear_selection();
                s.needs_repaint = true;
                s.info(format!("Removed {clips} sliver(s), closed {gaps} gap(s)"));
            }
            Err(err) => s.error(err.to_string()),
        },
    },
    Action {
        name: "Remove Empty Lanes",
        hint: "lanes with nothing on them",
        run: |e, s| report(s, e.remove_empty_lanes(), "No empty lanes", "Lanes removed"),
    },
    Action {
        name: "Turn All Lanes On",
        hint: "every hidden lane shown",
        run: |e, s| {
            let result =
                e.set_all_tracks_flag(bettercut_editor_core::command::TrackFlag::Enabled, true);
            report(s, result, "Every lane is on", "Lanes turned on");
        },
    },
    Action {
        name: "Clear All Solos",
        hint: "hear every lane again",
        run: |e, s| {
            let result =
                e.set_all_tracks_flag(bettercut_editor_core::command::TrackFlag::Solo, false);
            report(s, result, "No lane is soloed", "Solos cleared");
        },
    },
    Action {
        name: "Add Markers Every 10 s",
        hint: "a marker grid",
        run: |e, s| {
            report(
                s,
                e.add_markers_every(TimelineTime::from_seconds(10)),
                "No room for markers",
                "Markers added",
            )
        },
    },
    Action {
        name: "Mark In and Out Around Selection",
        hint: "the range of the selected clips",
        run: |e, s| {
            let chosen = selected(s);
            match e.mark_clips(&chosen) {
                Ok(Some(_)) => s.needs_repaint = true,
                Ok(None) => s.info("Select the clips to mark around"),
                Err(err) => s.error(err.to_string()),
            }
        },
    },
    Action {
        name: "Set Length 3 s",
        hint: "selected pictures and titles",
        run: |e, s| {
            let chosen = selected(s);
            report(
                s,
                e.set_clip_lengths(&chosen, TimelineTime::from_seconds(3)),
                "Select pictures or titles first",
                "Clips set to 3 s",
            );
        },
    },
    Action {
        name: "Speed 2x",
        hint: "selected clips, with their sound",
        run: |e, s| {
            let chosen = selected(s);
            let double = bettercut_editor_core::foundation::Rational::new(2, 1)
                .unwrap_or(bettercut_editor_core::foundation::Rational::ONE);
            report(
                s,
                e.set_clips_speed(&chosen, double),
                "Select clips with motion first",
                "Clips at 2x",
            );
        },
    },
    Action {
        name: "Reverse Selection",
        hint: "selected clips play backwards",
        run: |e, s| {
            let chosen = selected(s);
            report(
                s,
                e.set_clips_reversed(&chosen, true),
                "Select clips with motion first",
                "Clips reversed",
            );
        },
    },
    Action {
        name: "Remove Sound of Selection",
        hint: "keep the pictures",
        run: |e, s| {
            let chosen = selected(s);
            report(
                s,
                e.remove_sound_of(&chosen),
                "No sound linked to the selection",
                "Sound clips removed",
            );
        },
    },
    Action {
        name: "Title Each Selected Clip",
        hint: "a lower third reading each name",
        run: |e, s| {
            let chosen = selected(s);
            report(
                s,
                e.title_each_clip(&chosen),
                "Select pictures first",
                "Titles added",
            );
        },
    },
    Action {
        name: "Mixed Transitions on First Lane",
        hint: "a different one at each cut",
        run: |e, s| {
            let Some(track) = e
                .active_sequence()
                .and_then(|q| q.video_tracks.first().map(|t| t.id))
            else {
                return;
            };
            match e.mixed_transition_every_cut(track) {
                Ok((0, _)) => s.info("No cuts with room for a transition"),
                Ok((n, _)) => {
                    s.needs_repaint = true;
                    s.info(format!("Transitions on {n} cut(s)"));
                }
                Err(err) => s.error(err.to_string()),
            }
        },
    },
    Action {
        name: "Zoom to Fit",
        hint: "the whole edit across the timeline",
        run: |e, s| {
            let lanes = (s.screen_width - crate::theme::TRACK_HEADER_WIDTH).max(200.0);
            let duration = e
                .active_sequence()
                .map_or(TimelineTime::ZERO, |q| q.duration());
            s.zoom_to_fit(duration, lanes);
            s.needs_repaint = true;
        },
    },
    Action {
        name: "Zoom to Selection",
        hint: "the selected clips, or the whole edit",
        run: |e, s| {
            let lanes = (s.screen_width - crate::theme::TRACK_HEADER_WIDTH).max(200.0);
            s.zoom_to_selection_or_fit(e, lanes);
            s.needs_repaint = true;
        },
    },
    Action {
        name: "Add Title",
        hint: "at the playhead",
        run: |e, s| added(s, e.add_text("Title")),
    },
    Action {
        name: "Add Colour Background",
        hint: "a gradient under everything, at the playhead",
        run: |e, s| {
            let colour = bettercut_editor_core::media::Generated::Colour {
                top: [40, 60, 150],
                bottom: [10, 10, 40],
            };
            added(s, e.add_colour_clip(colour));
        },
    },
    Action {
        name: "Add Whoosh",
        hint: "a sound effect at the playhead",
        run: |e, s| {
            let result = e.add_generated_sound(
                bettercut_editor_core::media::GeneratedSound::Whoosh,
                TimelineTime::from_seconds(1),
            );
            added(s, result);
        },
    },
    Action {
        name: "New Project",
        hint: "asks first if there is unsaved work",
        run: |e, s| crate::panels::new_project(e, s),
    },
    Action {
        name: "Open Project",
        hint: "a .vproj file",
        run: |e, s| crate::panels::open_project(e, s),
    },
    Action {
        name: "Save As",
        hint: "save under a new name and keep working in it",
        run: |e, s| crate::panels::save_project_as(e, s),
    },
    Action {
        name: "Save a Copy",
        hint: "write a copy and stay in this project",
        run: |e, s| crate::panels::save_copy(e, s),
    },
    Action {
        name: "Collect Files",
        hint: "the project and every file it uses, into one folder",
        run: |e, s| crate::panels::collect_files(e, s),
    },
    Action {
        name: "Import Captions",
        hint: "an .srt or .vtt file",
        run: |e, s| crate::panels::import_captions(e, s),
    },
    Action {
        name: "Storyboard",
        hint: "the edit as cards to reorder",
        run: |_, s| s.storyboard_open = true,
    },
    Action {
        name: "Notes",
        hint: "the notes on this project's clips",
        run: |_, s| s.notes_open = true,
    },
    Action {
        name: "Scopes",
        hint: "waveform, vectorscope and histogram",
        run: |_, s| s.scopes.open = true,
    },
    Action {
        name: "Export Queue",
        hint: "exports waiting and done",
        run: |_, s| s.export_queue.open = true,
    },
    Action {
        name: "Toggle Snapping",
        hint: "N",
        run: |_, s| s.snapping = !s.snapping,
    },
    Action {
        name: "Full Screen Preview",
        hint: "F",
        run: |_, s| s.fullscreen = !s.fullscreen,
    },
    Action {
        name: "Captions Window",
        hint: "type, fix and tidy captions",
        run: |_, s| s.captions_open = true,
    },
    Action {
        name: "Markers Window",
        hint: "every marker in a list",
        run: |_, s| s.markers_open = true,
    },
    Action {
        name: "History",
        hint: "Ctrl+H",
        run: |_, s| s.history_open = true,
    },
    Action {
        name: "Keyboard Shortcuts",
        hint: "?",
        run: |_, s| s.shortcuts_open = true,
    },
    Action {
        name: "Export Captions",
        hint: ".srt, .vtt or a transcript",
        run: |e, s| crate::panels::export_captions(e, s),
    },
    Action {
        name: "Welcome",
        hint: "the four steps to a finished video",
        run: |_, s| s.welcome_open = true,
    },
    Action {
        name: "Report a Bug on GitHub",
        hint: "opens a new issue with the version and system filled in",
        run: |e, s| {
            let body = crate::bug_report::issue_body(&crate::bug_report::details(e));
            s.open_url = Some(crate::bug_report::new_issue_url("", &body));
            s.info("Opening a new issue in your browser — nothing is sent until you submit it");
        },
    },
    Action {
        name: "Copy Details for a Bug Report",
        hint: "version, system and project shape — no file names",
        run: |e, s| {
            s.copy_out = Some(crate::bug_report::details(e));
            s.info("Copied — paste it into your message and say what happened");
        },
    },
    Action {
        name: "Show Video",
        hint: "the inspector's Video tab: where the clip is: size, position, opacity",
        run: |_, s| s.inspector_tab = crate::panels::InspectorTab::Video,
    },
    Action {
        name: "Show Effects",
        hint: "the inspector's Effects tab: blend, mask, keys, blur and the rest",
        run: |_, s| s.inspector_tab = crate::panels::InspectorTab::Effects,
    },
    Action {
        name: "Show Colours",
        hint: "the inspector's Colours tab: brightness, contrast, wheels, curves",
        run: |_, s| s.inspector_tab = crate::panels::InspectorTab::Colours,
    },
    Action {
        name: "Show Audio",
        hint: "the inspector's Audio tab: volume, clean-up, EQ",
        run: |_, s| s.inspector_tab = crate::panels::InspectorTab::Audio,
    },
    Action {
        name: "Show Speed",
        hint: "the inspector's Speed tab: faster, slower, reversed, ramps",
        run: |_, s| s.inspector_tab = crate::panels::InspectorTab::Speed,
    },
    Action {
        name: "Show Animation",
        hint: "the inspector's Animation tab: the keyframes on the clip",
        run: |_, s| s.inspector_tab = crate::panels::InspectorTab::Animation,
    },
    Action {
        name: "Show Media",
        hint: "the left panel's Media tab: the files in the project",
        run: |_, s| s.library_tab = crate::library::LibraryTab::Media,
    },
    Action {
        name: "Show Text Styles",
        hint: "the left panel's Text tab: titles to add, in each style",
        run: |_, s| s.library_tab = crate::library::LibraryTab::Text,
    },
    Action {
        name: "Show Stickers",
        hint: "the left panel's Stickers tab: symbols to put over the picture",
        run: |_, s| s.library_tab = crate::library::LibraryTab::Stickers,
    },
    Action {
        name: "Show Transitions",
        hint: "the left panel's Transitions tab: for the cut after the selected clip",
        run: |_, s| s.library_tab = crate::library::LibraryTab::Transitions,
    },
    Action {
        name: "Show Filters",
        hint: "the left panel's Filters tab: one-click looks for the selected clips",
        run: |_, s| s.library_tab = crate::library::LibraryTab::Filters,
    },
    Action {
        name: "Connect an AI Assistant",
        hint: "Claude or another assistant, editing with you in this window",
        run: |_, s| s.assistant_open = true,
    },
    Action {
        name: "What's New",
        hint: "the new things in this version",
        run: |_, s| s.whats_new_open = true,
    },
    Action {
        name: "Try a Sample",
        hint: "a short edit to practise on",
        run: |e, s| crate::sample::open(e, s),
    },
    Action {
        name: "Remove Empty Captions",
        hint: "the ones nobody typed into",
        run: |e, s| report(s, e.remove_empty_captions(), "No empty captions", "Removed"),
    },
    Action {
        name: "Close Caption Gaps",
        hint: "no blinking between lines",
        run: |e, s| {
            report(
                s,
                e.close_caption_gaps(TimelineTime::from_millis(500)),
                "No short gaps",
                "Gaps closed",
            )
        },
    },
    Action {
        name: "Split Long Captions",
        hint: "lines too long to read at once",
        run: |e, s| {
            let result =
                e.split_long_captions(bettercut_editor_core::caption_replace::LONG_CAPTION);
            report(s, result, "No line is that long", "Split");
        },
    },
    Action {
        name: "Captions in Capitals",
        hint: "the short-video look",
        run: |e, s| {
            let result =
                e.set_caption_case(bettercut_editor_core::caption_replace::LetterCase::Upper);
            report(s, result, "Already in capitals", "Changed");
        },
    },
    Action {
        name: "Duplicate Onto Lane Above",
        hint: "the one selected picture",
        run: |e, s| match one(s) {
            Some(clip) => match e.duplicate_onto_lane_above(clip) {
                Ok(copy) => {
                    s.select_only(copy);
                    s.needs_repaint = true;
                }
                Err(err) => s.error(err.to_string()),
            },
            None => s.info("Select one picture clip first"),
        },
    },
    Action {
        name: "Extend to Next Clip",
        hint: "fill the gap after the selected clip",
        run: |e, s| match one(s) {
            Some(clip) => {
                if let Err(err) = e.extend_to_next(clip) {
                    s.error(err.to_string());
                }
            }
            None => s.info("Select one clip first"),
        },
    },
    Action {
        name: "Hold First Frame 2 s",
        hint: "the selected shot standing still first",
        run: |e, s| match one(s) {
            Some(clip) => match e.hold_first_frame(clip, TimelineTime::from_seconds(2)) {
                Ok(held) => s.select_only(held),
                Err(err) => s.error(err.to_string()),
            },
            None => s.info("Select one picture clip first"),
        },
    },
    Action {
        name: "Reverse Clip Order",
        hint: "the selected run, last shot first",
        run: |e, s| {
            let chosen = selected(s);
            if let Err(err) = e.reverse_clip_order(&chosen) {
                s.error(err.to_string());
            }
        },
    },
];

/// Select what was just added, or say why nothing was.
fn added(state: &mut UiState, result: Result<ClipId, bettercut_editor_core::EditorError>) {
    match result {
        Ok(clip) => {
            state.select_only(clip);
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// The one selected clip, when exactly one is.
fn one(state: &UiState) -> Option<ClipId> {
    (state.selected_clips.len() == 1)
        .then(|| state.selected_clips.iter().next().copied())
        .flatten()
}

/// The actions matching `query`, in their listed order.
pub fn matching(query: &str) -> Vec<&'static Action> {
    ACTIONS
        .iter()
        .filter(|a| crate::history_panel::matches(a.name, query))
        .collect()
}

/// Whether `query` reads as a place to go rather than an action's name:
/// it opens with a digit or a sign, as a timecode does.
pub fn is_timecode_query(query: &str) -> bool {
    query
        .trim_start()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '+' || c == '-')
}

/// The named markers matching `query`, in time order, at most five: the
/// palette's way to a place in a long edit by what happens there.
pub fn marker_matches(editor: &Editor, query: &str) -> Vec<(String, TimelineTime)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    editor
        .markers()
        .iter()
        .filter(|m| !m.label.trim().is_empty())
        .filter(|m| crate::history_panel::matches(&m.label, query))
        .take(5)
        .map(|m| (m.label.clone(), m.time))
        .collect()
}

/// The recent projects whose names match `query`, at most five, newest
/// first — the way back to yesterday's edit without the Open dialog.
pub fn project_matches(
    recent: &crate::recent::RecentProjects,
    query: &str,
) -> Vec<std::path::PathBuf> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    recent
        .paths()
        .iter()
        .filter(|path| {
            let (name, _) = crate::recent::menu_label(path);
            crate::history_panel::matches(&name, query)
        })
        .take(5)
        .cloned()
        .collect()
}

/// How many recently run actions the palette keeps at the top.
pub const RECENT: usize = 5;

/// [`matching`], with the actions in `recent` (most recent first) moved to
/// the front: a job done over and over is Ctrl+K, Enter.
pub fn ordered(query: &str, recent: &[&'static str]) -> Vec<&'static Action> {
    let mut found = matching(query);
    found.sort_by_key(|a| {
        recent
            .iter()
            .position(|r| *r == a.name)
            .unwrap_or(usize::MAX)
    });
    found
}

/// Remember `name` as the most recent action.
pub fn remember(recent: &mut Vec<&'static str>, name: &'static str) {
    recent.retain(|r| *r != name);
    recent.insert(0, name);
    recent.truncate(RECENT);
}

/// Run the action called `name`. Returns whether there was one.
pub fn run(editor: &mut Editor, state: &mut UiState, name: &str) -> bool {
    match ACTIONS.iter().find(|a| a.name == name) {
        Some(action) => {
            (action.run)(editor, state);
            true
        }
        None => false,
    }
}

/// Draw the palette, if open, and run what is chosen.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.palette_open {
        return;
    }
    // What the zoom actions measure against: they run without a context.
    state.screen_width = ctx.content_rect().width();
    // Recent actions from an earlier run, the first time the palette opens:
    // only names this build still has.
    if state.palette_recent.is_empty() && !state.prefs.palette_recent.is_empty() {
        state.palette_recent = state
            .prefs
            .palette_recent
            .iter()
            .filter_map(|name| {
                ACTIONS
                    .iter()
                    .find(|a| a.name == name.as_str())
                    .map(|a| a.name)
            })
            .collect();
    }
    let found = ordered(&state.palette_query, &state.palette_recent);
    let recent = state.palette_recent.clone();
    // A timecode typed here is a place to go, as in the readout.
    let rate = editor
        .active_sequence()
        .map_or(bettercut_editor_core::foundation::FrameRate::PAL_25, |s| {
            s.frame_rate
        });
    let jump = if is_timecode_query(&state.palette_query) {
        crate::timecode_entry::parse(&state.palette_query, rate)
    } else {
        None
    };
    let mut go = false;
    let places = marker_matches(editor, &state.palette_query);
    let mut to_marker: Option<TimelineTime> = None;
    let projects = project_matches(&state.recent, &state.palette_query);
    // Clips by name, a title's words or a note — the editor's own search,
    // held to a handful so the actions stay in view.
    let clips: Vec<_> = editor
        .find_clips(&state.palette_query)
        .into_iter()
        .take(5)
        .collect();
    let mut to_clip: Option<(ClipId, TimelineTime)> = None;
    let mut to_project: Option<std::path::PathBuf> = None;
    let pick = state.palette_pick.min(found.len().saturating_sub(1));
    let (down, up, enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let mut chosen: Option<&'static Action> = None;
    let mut pick = if down {
        (pick + 1).min(found.len().saturating_sub(1))
    } else if up {
        pick.saturating_sub(1)
    } else {
        pick
    };

    egui::Window::new("Do something")
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
        .fixed_size(egui::vec2(420.0, 0.0))
        .show(ctx, |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.palette_query)
                    .hint_text("Type an action — split, gaps, captions — or a time to go to")
                    .desired_width(f32::INFINITY),
            );
            field.request_focus();
            if field.changed() {
                pick = 0;
            }
            ui.add_space(4.0);
            if jump.is_some()
                && ui
                    .selectable_label(true, format!("Go to {}", state.palette_query.trim()))
                    .clicked()
            {
                go = true;
            }
            for (label, at) in &places {
                if ui
                    .selectable_label(
                        found.is_empty() && jump.is_none(),
                        format!("Marker · {label}"),
                    )
                    .on_hover_text(at.format_timecode())
                    .clicked()
                {
                    to_marker = Some(*at);
                }
            }
            for found in &clips {
                if ui
                    .selectable_label(false, format!("Clip · {}", found.label))
                    .on_hover_text(found.start.format_timecode())
                    .clicked()
                {
                    to_clip = Some((found.clip, found.start));
                }
            }
            for path in &projects {
                let (name, folder) = crate::recent::menu_label(path);
                if ui
                    .selectable_label(false, format!("Open · {name}"))
                    .on_hover_text(folder)
                    .clicked()
                {
                    to_project = Some(path.clone());
                }
            }
            if found.is_empty()
                && jump.is_none()
                && places.is_empty()
                && projects.is_empty()
                && clips.is_empty()
            {
                ui.label(egui::RichText::new("Nothing by that name").color(theme::disabled()));
            }
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (index, action) in found.iter().enumerate() {
                        let row = ui.horizontal(|ui| {
                            let label = ui.selectable_label(index == pick, action.name);
                            if recent.contains(&action.name) {
                                ui.label(
                                    egui::RichText::new("recent")
                                        .small()
                                        .color(theme::disabled()),
                                );
                            }
                            ui.label(
                                egui::RichText::new(crate::keys::keys(action.hint))
                                    .small()
                                    .color(theme::disabled()),
                            );
                            label
                        });
                        if row.inner.clicked() {
                            chosen = Some(action);
                        }
                        if index == pick && (down || up) {
                            row.inner.scroll_to_me(None);
                        }
                    }
                });
            ui.label(
                egui::RichText::new("Up and Down to choose · Enter to run · Esc to close")
                    .small()
                    .color(theme::disabled()),
            );
        });

    if enter {
        if jump.is_some() {
            go = true;
        } else if found.is_empty()
            && let Some((_, at)) = places.first()
        {
            to_marker = Some(*at);
        } else {
            chosen = chosen.or_else(|| found.get(pick).copied());
        }
    }
    if let Some((clip, start)) = to_clip {
        state.select_only(clip);
        state.jump_to = Some(start);
        state.palette_open = false;
        state.palette_query.clear();
        state.palette_pick = 0;
        state.needs_repaint = true;
        return;
    }
    if let Some(path) = to_project {
        state.palette_open = false;
        state.palette_query.clear();
        state.palette_pick = 0;
        // Through the same door as the Open Recent menu: unsaved work is
        // asked about first.
        crate::panels::open_project_at(editor, state, &path);
        state.needs_repaint = true;
        return;
    }
    if let Some(at) = to_marker {
        state.jump_to = Some(at);
        state.palette_open = false;
        state.palette_query.clear();
        state.palette_pick = 0;
        state.needs_repaint = true;
        return;
    }
    if go && let Some(jump) = jump {
        state.jump_to = Some(crate::timecode_entry::resolve(
            jump,
            editor.playhead(),
            editor.start_timecode(),
        ));
        state.palette_open = false;
        state.palette_query.clear();
        state.palette_pick = 0;
        state.needs_repaint = true;
        return;
    }
    state.palette_pick = pick;
    if let Some(action) = chosen {
        remember(&mut state.palette_recent, action.name);
        state.prefs.palette_recent = state
            .palette_recent
            .iter()
            .map(|n| (*n).to_owned())
            .collect();
        let _ = state.prefs.save();
        state.palette_open = false;
        state.palette_query.clear();
        state.palette_pick = 0;
        (action.run)(editor, state);
        state.needs_repaint = true;
    } else if escape {
        state.palette_open = false;
        state.palette_query.clear();
        state.palette_pick = 0;
    }
    state.needs_repaint = true;
}
