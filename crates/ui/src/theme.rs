//! Colours and metrics.
//!
//! §58: "Keep the interface simple. Avoid exposing hundreds of controls
//! simultaneously." One palette, defined once, so panels cannot drift apart.

use egui::Color32;

use std::sync::atomic::{AtomicBool, Ordering};

/// Every colour the interface draws with, as one set, so a theme is a
/// palette and not thirty scattered numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub background: Color32,
    pub panel: Color32,
    pub timeline_background: Color32,
    pub track_header: Color32,
    pub track_lane: Color32,
    pub track_lane_alt: Color32,
    pub grid_line: Color32,
    pub ruler_text: Color32,
    pub video_clip: Color32,
    pub video_clip_top: Color32,
    pub audio_clip: Color32,
    pub audio_clip_top: Color32,
    pub clip_text: Color32,
    pub selection: Color32,
    pub playhead: Color32,
    pub marker: Color32,
    pub keyframe: Color32,
    pub automation: Color32,
    pub transition: Color32,
    pub text_clip: Color32,
    pub text_clip_top: Color32,
    pub in_out_mark: Color32,
    pub in_out_span: Color32,
    pub track_automation: Color32,
    pub rendered: Color32,
    pub rendered_stale: Color32,
    pub fade_handle: Color32,
    pub adjustment_clip: Color32,
    pub adjustment_clip_top: Color32,
    pub disabled: Color32,
    pub error_text: Color32,
    pub caution: Color32,
    pub ok_text: Color32,
}

/// The dark palette: what the editor has always looked like.
pub const DARK: Palette = Palette {
    background: Color32::from_rgb(24, 25, 28),
    panel: Color32::from_rgb(31, 33, 37),
    timeline_background: Color32::from_rgb(20, 21, 24),
    track_header: Color32::from_rgb(35, 37, 42),
    track_lane: Color32::from_rgb(27, 29, 33),
    track_lane_alt: Color32::from_rgb(30, 32, 37),
    grid_line: Color32::from_rgb(45, 48, 54),
    ruler_text: Color32::from_rgb(150, 155, 165),
    video_clip: Color32::from_rgb(64, 116, 190),
    video_clip_top: Color32::from_rgb(86, 142, 219),
    audio_clip: Color32::from_rgb(58, 150, 118),
    audio_clip_top: Color32::from_rgb(78, 178, 142),
    clip_text: Color32::from_rgb(238, 242, 248),
    selection: Color32::from_rgb(255, 196, 84),
    playhead: Color32::from_rgb(238, 92, 92),
    marker: Color32::from_rgb(120, 214, 120),
    keyframe: Color32::from_rgb(126, 200, 255),
    automation: Color32::from_rgb(250, 226, 138),
    transition: Color32::from_rgb(214, 226, 240),
    text_clip: Color32::from_rgb(126, 96, 178),
    text_clip_top: Color32::from_rgb(152, 120, 206),
    in_out_mark: Color32::from_rgb(120, 200, 255),
    in_out_span: Color32::from_rgba_premultiplied(20, 40, 60, 40),
    track_automation: Color32::from_rgb(138, 214, 250),
    rendered: Color32::from_rgb(90, 190, 110),
    rendered_stale: Color32::from_rgb(200, 160, 70),
    fade_handle: Color32::from_rgb(240, 240, 240),
    adjustment_clip: Color32::from_rgb(168, 124, 52),
    adjustment_clip_top: Color32::from_rgb(198, 152, 74),
    disabled: Color32::from_rgb(96, 100, 108),
    error_text: Color32::from_rgb(240, 120, 120),
    caution: Color32::from_rgb(230, 180, 90),
    ok_text: Color32::from_rgb(140, 200, 150),
};

/// The light palette: the same roles on a white ground, for a bright room
/// or a bright screen. Clips keep their hues, a shade deeper so their light
/// text still reads; the accents come down from neon to ink.
pub const LIGHT: Palette = Palette {
    background: Color32::from_rgb(244, 245, 248),
    panel: Color32::from_rgb(252, 252, 253),
    timeline_background: Color32::from_rgb(232, 234, 238),
    track_header: Color32::from_rgb(224, 227, 232),
    track_lane: Color32::from_rgb(240, 241, 244),
    track_lane_alt: Color32::from_rgb(234, 236, 240),
    grid_line: Color32::from_rgb(200, 204, 211),
    ruler_text: Color32::from_rgb(84, 90, 100),
    video_clip: Color32::from_rgb(96, 146, 216),
    video_clip_top: Color32::from_rgb(70, 120, 192),
    audio_clip: Color32::from_rgb(84, 176, 142),
    audio_clip_top: Color32::from_rgb(58, 148, 114),
    clip_text: Color32::from_rgb(22, 26, 32),
    selection: Color32::from_rgb(214, 132, 0),
    playhead: Color32::from_rgb(208, 58, 58),
    marker: Color32::from_rgb(44, 150, 44),
    keyframe: Color32::from_rgb(30, 120, 200),
    automation: Color32::from_rgb(160, 132, 10),
    transition: Color32::from_rgb(64, 84, 112),
    text_clip: Color32::from_rgb(136, 108, 186),
    text_clip_top: Color32::from_rgb(108, 80, 158),
    in_out_mark: Color32::from_rgb(30, 120, 200),
    in_out_span: Color32::from_rgba_premultiplied(30, 60, 120, 40),
    track_automation: Color32::from_rgb(30, 120, 180),
    rendered: Color32::from_rgb(44, 150, 74),
    rendered_stale: Color32::from_rgb(176, 126, 30),
    fade_handle: Color32::from_rgb(40, 40, 44),
    adjustment_clip: Color32::from_rgb(196, 146, 66),
    adjustment_clip_top: Color32::from_rgb(170, 124, 50),
    disabled: Color32::from_rgb(128, 134, 144),
    error_text: Color32::from_rgb(188, 48, 48),
    caution: Color32::from_rgb(166, 108, 16),
    ok_text: Color32::from_rgb(40, 128, 70),
};

static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// Choose the light palette (or the dark one). Takes effect for everything
/// drawn after; call [`apply`] too, so egui's own widgets follow.
pub fn set_light(light: bool) {
    LIGHT_MODE.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT_MODE.load(Ordering::Relaxed)
}

/// The palette in force.
pub fn palette() -> Palette {
    if is_light() { LIGHT } else { DARK }
}

pub fn background() -> Color32 {
    if is_light() {
        LIGHT.background
    } else {
        DARK.background
    }
}
pub fn panel() -> Color32 {
    if is_light() { LIGHT.panel } else { DARK.panel }
}
pub fn timeline_background() -> Color32 {
    if is_light() {
        LIGHT.timeline_background
    } else {
        DARK.timeline_background
    }
}
pub fn track_header() -> Color32 {
    if is_light() {
        LIGHT.track_header
    } else {
        DARK.track_header
    }
}
pub fn track_lane() -> Color32 {
    if is_light() {
        LIGHT.track_lane
    } else {
        DARK.track_lane
    }
}
pub fn track_lane_alt() -> Color32 {
    if is_light() {
        LIGHT.track_lane_alt
    } else {
        DARK.track_lane_alt
    }
}
pub fn grid_line() -> Color32 {
    if is_light() {
        LIGHT.grid_line
    } else {
        DARK.grid_line
    }
}
pub fn ruler_text() -> Color32 {
    if is_light() {
        LIGHT.ruler_text
    } else {
        DARK.ruler_text
    }
}
pub fn video_clip() -> Color32 {
    if is_light() {
        LIGHT.video_clip
    } else {
        DARK.video_clip
    }
}
pub fn video_clip_top() -> Color32 {
    if is_light() {
        LIGHT.video_clip_top
    } else {
        DARK.video_clip_top
    }
}
pub fn audio_clip() -> Color32 {
    if is_light() {
        LIGHT.audio_clip
    } else {
        DARK.audio_clip
    }
}
pub fn audio_clip_top() -> Color32 {
    if is_light() {
        LIGHT.audio_clip_top
    } else {
        DARK.audio_clip_top
    }
}
pub fn clip_text() -> Color32 {
    if is_light() {
        LIGHT.clip_text
    } else {
        DARK.clip_text
    }
}
pub fn selection() -> Color32 {
    if is_light() {
        LIGHT.selection
    } else {
        DARK.selection
    }
}
pub fn playhead() -> Color32 {
    if is_light() {
        LIGHT.playhead
    } else {
        DARK.playhead
    }
}
pub fn marker() -> Color32 {
    if is_light() {
        LIGHT.marker
    } else {
        DARK.marker
    }
}
pub fn keyframe() -> Color32 {
    if is_light() {
        LIGHT.keyframe
    } else {
        DARK.keyframe
    }
}
pub fn automation() -> Color32 {
    if is_light() {
        LIGHT.automation
    } else {
        DARK.automation
    }
}
pub fn transition() -> Color32 {
    if is_light() {
        LIGHT.transition
    } else {
        DARK.transition
    }
}
pub fn text_clip() -> Color32 {
    if is_light() {
        LIGHT.text_clip
    } else {
        DARK.text_clip
    }
}
pub fn text_clip_top() -> Color32 {
    if is_light() {
        LIGHT.text_clip_top
    } else {
        DARK.text_clip_top
    }
}
pub fn in_out_mark() -> Color32 {
    if is_light() {
        LIGHT.in_out_mark
    } else {
        DARK.in_out_mark
    }
}
pub fn in_out_span() -> Color32 {
    if is_light() {
        LIGHT.in_out_span
    } else {
        DARK.in_out_span
    }
}
pub fn track_automation() -> Color32 {
    if is_light() {
        LIGHT.track_automation
    } else {
        DARK.track_automation
    }
}
pub fn rendered() -> Color32 {
    if is_light() {
        LIGHT.rendered
    } else {
        DARK.rendered
    }
}
pub fn rendered_stale() -> Color32 {
    if is_light() {
        LIGHT.rendered_stale
    } else {
        DARK.rendered_stale
    }
}
pub fn fade_handle() -> Color32 {
    if is_light() {
        LIGHT.fade_handle
    } else {
        DARK.fade_handle
    }
}
pub fn adjustment_clip() -> Color32 {
    if is_light() {
        LIGHT.adjustment_clip
    } else {
        DARK.adjustment_clip
    }
}
pub fn adjustment_clip_top() -> Color32 {
    if is_light() {
        LIGHT.adjustment_clip_top
    } else {
        DARK.adjustment_clip_top
    }
}
pub fn disabled() -> Color32 {
    if is_light() {
        LIGHT.disabled
    } else {
        DARK.disabled
    }
}
pub fn error_text() -> Color32 {
    if is_light() {
        LIGHT.error_text
    } else {
        DARK.error_text
    }
}
pub fn caution() -> Color32 {
    if is_light() {
        LIGHT.caution
    } else {
        DARK.caution
    }
}
pub fn ok_text() -> Color32 {
    if is_light() {
        LIGHT.ok_text
    } else {
        DARK.ok_text
    }
}

/// Width of the track-name column on the left of the timeline.
pub const TRACK_HEADER_WIDTH: f32 = 148.0;
/// Height of the timecode ruler above the tracks.
pub const RULER_HEIGHT: f32 = 26.0;
/// Height of the overview strip along the bottom of the timeline — the whole
/// edit at a glance, with the visible part marked on it.
pub const OVERVIEW_HEIGHT: f32 = 30.0;
/// A lane at the normal height; `state::LaneHeight` offers smaller and larger.
pub const TRACK_HEIGHT: f32 = 58.0;
pub const TRACK_GAP: f32 = 2.0;
pub const CLIP_CORNER_RADIUS: u8 = 4;

/// Apply the app's visual style. Called once at startup.
///
/// The editor commits to a dark theme: a preview is judged against its
/// surroundings, and a bright shell shifts how footage looks (§21a is about
/// getting colour right — the chrome should not fight it).
pub fn apply(ctx: &egui::Context) {
    let light = is_light();
    ctx.set_theme(if light {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });

    // Applied to both theme slots so the app looks the same even if something
    // else flips the preference.
    ctx.all_styles_mut(|style| {
        style.visuals = if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        style.visuals.panel_fill = panel();
        style.visuals.window_fill = panel();
        style.visuals.extreme_bg_color = timeline_background();

        // Roomier controls: the brief is an editor whose controls are easy to
        // understand, not one that fits the most knobs per square inch.
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.interact_size.y = 26.0;

        // One accent, used for what is chosen and for the main action, so the
        // eye has one colour to follow. Everything else stays neutral.
        let accent = accent();
        style.visuals.selection.bg_fill = accent;
        style.visuals.selection.stroke = egui::Stroke::new(1.0, accent_text());
        style.visuals.hyperlink_color = accent_text();

        // Flat controls: a button is a quiet shape until the pointer is on
        // it. Twenty filled grey boxes in a row all shout at once.
        let radius = egui::CornerRadius::same(6);
        let quiet = if light {
            egui::Color32::from_rgb(226, 228, 233)
        } else {
            egui::Color32::from_rgb(40, 42, 48)
        };
        let lifted = if light {
            egui::Color32::from_rgb(212, 215, 222)
        } else {
            egui::Color32::from_rgb(52, 55, 62)
        };
        let widgets = &mut style.visuals.widgets;
        widgets.inactive.weak_bg_fill = quiet;
        widgets.inactive.bg_fill = quiet;
        widgets.inactive.bg_stroke = egui::Stroke::NONE;
        widgets.inactive.corner_radius = radius;
        widgets.hovered.weak_bg_fill = lifted;
        widgets.hovered.bg_fill = lifted;
        widgets.hovered.bg_stroke = egui::Stroke::NONE;
        widgets.hovered.corner_radius = radius;
        widgets.active.weak_bg_fill = accent;
        widgets.active.bg_fill = accent;
        widgets.active.bg_stroke = egui::Stroke::NONE;
        widgets.active.corner_radius = radius;
        widgets.open.weak_bg_fill = lifted;
        widgets.open.bg_fill = lifted;
        widgets.open.bg_stroke = egui::Stroke::NONE;
        widgets.open.corner_radius = radius;
        widgets.noninteractive.corner_radius = radius;
        // The lines between panels and sections: there, but not a border.
        widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, grid_line());

        style.visuals.window_corner_radius = egui::CornerRadius::same(10);
        style.visuals.menu_corner_radius = egui::CornerRadius::same(8);
        style.visuals.window_stroke = egui::Stroke::new(1.0, grid_line());
    });
}

/// The one accent: selection, the main action, what is switched on.
pub fn accent() -> Color32 {
    if is_light() {
        Color32::from_rgb(47, 110, 214)
    } else {
        Color32::from_rgb(58, 118, 216)
    }
}

/// The accent as text on the panel colour: a little lighter, to read.
pub fn accent_text() -> Color32 {
    if is_light() {
        Color32::from_rgb(34, 90, 190)
    } else {
        Color32::from_rgb(120, 170, 245)
    }
}

/// The screen's main action — Export — in the accent, so there is one thing
/// to find when the work is done.
pub fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).strong()).fill(accent())
}

/// A control's name, at the start of its row and a fixed width, so the
/// sliders beside it line up in one column: label, slider, value.
pub fn row_label(ui: &mut egui::Ui, text: impl Into<String>) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(96.0, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(96.0);
            ui.add(
                egui::Label::new(egui::RichText::new(text.into()).color(ruler_text())).truncate(),
            );
        },
    );
}

/// A slider with its name before it rather than after — label, slider,
/// value, the way a row is read. The response is the slider's own, so
/// `changed`, `dragged` and hover text behave as they would on the slider.
pub fn labeled(text: impl Into<String>, slider: egui::Slider<'_>) -> Labeled<'_> {
    Labeled {
        text: text.into(),
        slider,
    }
}

/// A slider and its name, drawn as one row; made by [`labeled`].
pub struct Labeled<'a> {
    text: String,
    slider: egui::Slider<'a>,
}

impl egui::Widget for Labeled<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.horizontal(|ui| {
            row_label(ui, self.text);
            ui.add(self.slider)
        })
        .inner
    }
}

/// A section's heading: small capitals in the accent, as the inspector's
/// groups are titled, so a long panel reads as a few named parts.
/// Where a window the person opens first appears: centred, just below the
/// toolbar. Without it egui puts a window at the very top-left, over Play,
/// New and Open — which is where seven of them used to open. It is only the
/// first place: a window moved stays where it was put.
pub fn placed<'a>(window: egui::Window<'a>, ctx: &egui::Context) -> egui::Window<'a> {
    let top = ctx.content_rect().center_top();
    window
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(top + egui::vec2(0.0, 72.0))
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(11.5)
            .strong()
            .color(accent_text()),
    );
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two palettes are two palettes: light on a light ground, dark on a
    /// dark one, and no role left the same in both by accident.
    #[test]
    fn the_light_palette_is_light_and_differs_everywhere() {
        assert!(LIGHT.background.r() > 200 && DARK.background.r() < 60);
        assert!(LIGHT.clip_text.r() < 60 && DARK.clip_text.r() > 200);
        assert_ne!(LIGHT, DARK);
    }

    /// The switch is what every token reads.
    #[test]
    fn the_switch_changes_what_the_tokens_say() {
        set_light(false);
        assert_eq!(background(), DARK.background);
        set_light(true);
        assert_eq!(background(), LIGHT.background);
        assert_eq!(palette(), LIGHT);
        set_light(false);
    }
}
