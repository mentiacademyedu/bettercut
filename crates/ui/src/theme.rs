//! Colours and metrics.
//!
//! §58: "Keep the interface simple. Avoid exposing hundreds of controls
//! simultaneously." One palette, defined once, so panels cannot drift apart.

use egui::Color32;

pub const BACKGROUND: Color32 = Color32::from_rgb(24, 25, 28);
pub const PANEL: Color32 = Color32::from_rgb(31, 33, 37);
pub const TIMELINE_BACKGROUND: Color32 = Color32::from_rgb(20, 21, 24);
pub const TRACK_HEADER: Color32 = Color32::from_rgb(35, 37, 42);
pub const TRACK_LANE: Color32 = Color32::from_rgb(27, 29, 33);
pub const TRACK_LANE_ALT: Color32 = Color32::from_rgb(30, 32, 37);
pub const GRID_LINE: Color32 = Color32::from_rgb(45, 48, 54);
pub const RULER_TEXT: Color32 = Color32::from_rgb(150, 155, 165);

pub const VIDEO_CLIP: Color32 = Color32::from_rgb(64, 116, 190);
pub const VIDEO_CLIP_TOP: Color32 = Color32::from_rgb(86, 142, 219);
pub const AUDIO_CLIP: Color32 = Color32::from_rgb(58, 150, 118);
pub const AUDIO_CLIP_TOP: Color32 = Color32::from_rgb(78, 178, 142);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(238, 242, 248);
pub const SELECTION: Color32 = Color32::from_rgb(255, 196, 84);

pub const PLAYHEAD: Color32 = Color32::from_rgb(238, 92, 92);
/// Keyframes, in the inspector and on the clip (§24). Deliberately not the
/// selection colour: a key and a selected clip are often on screen together.
pub const KEYFRAME: Color32 = Color32::from_rgb(126, 200, 255);

/// §25's transitions, drawn over the cut. Pale and cool so it reads as an
/// overlay on the clips rather than as a third clip between them.
pub const TRANSITION: Color32 = Color32::from_rgb(214, 226, 240);

/// §26's text overlays. A different hue from the video and audio clips, because
/// a title is a different kind of thing and the lane is read at a glance.
pub const TEXT_CLIP: Color32 = Color32::from_rgb(126, 96, 178);
pub const TEXT_CLIP_TOP: Color32 = Color32::from_rgb(152, 120, 206);
pub const DISABLED: Color32 = Color32::from_rgb(96, 100, 108);
pub const ERROR_TEXT: Color32 = Color32::from_rgb(240, 120, 120);
pub const OK_TEXT: Color32 = Color32::from_rgb(140, 200, 150);

/// Width of the track-name column on the left of the timeline.
pub const TRACK_HEADER_WIDTH: f32 = 148.0;
/// Height of the timecode ruler above the tracks.
pub const RULER_HEIGHT: f32 = 26.0;
pub const TRACK_HEIGHT: f32 = 58.0;
pub const TRACK_GAP: f32 = 2.0;
pub const CLIP_CORNER_RADIUS: u8 = 4;

/// Apply the app's visual style. Called once at startup.
///
/// The editor commits to a dark theme: a preview is judged against its
/// surroundings, and a bright shell shifts how footage looks (§21a is about
/// getting colour right — the chrome should not fight it).
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);

    // Applied to both theme slots so the app looks the same even if something
    // else flips the preference.
    ctx.all_styles_mut(|style| {
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = PANEL;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = TIMELINE_BACKGROUND;

        // Roomier controls: the brief is an editor whose controls are easy to
        // understand, not one that fits the most knobs per square inch.
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.interact_size.y = 26.0;
    });
}
