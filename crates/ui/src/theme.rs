//! The design system: colours, type, spacing, sizes and how controls look in
//! each state, defined once so no panel can drift from the rest.
//!
//! §58: "Keep the interface simple. Avoid exposing hundreds of controls
//! simultaneously." One palette, one type scale, one set of sizes.
//!
//! # The shape of it
//!
//! Three grounds, darkest at the back: the window behind everything, the
//! panels on it, and the controls on the panels. One accent — a soft blue —
//! for what is chosen and for the main action, and nothing else coloured
//! unless it carries meaning (a clip's kind, a warning, the playhead).
//! Controls are quiet until the pointer is on them; borders are hairlines,
//! there to separate rather than to decorate.

use egui::Color32;

use std::sync::atomic::{AtomicBool, Ordering};


// ── Sizes ─────────────────────────────────────────────────────────────────
//
// A four-point grid: every gap is one of these, so rows line up from panel
// to panel.

/// The smallest gap: between an icon and its word, between paired controls.
pub const SPACE_XS: f32 = 2.0;
pub const SPACE_S: f32 = 4.0;
pub const SPACE_M: f32 = 8.0;
pub const SPACE_L: f32 = 12.0;
pub const SPACE_XL: f32 = 16.0;

/// Corner radii: small for controls, medium for cards and menus, large for
/// windows. Kept small — this is a tool, not a web page.
pub const RADIUS_SMALL: u8 = 4;
pub const RADIUS: u8 = 6;
pub const RADIUS_LARGE: u8 = 8;

/// How tall a control is: a button, a field, a slider's row.
pub const CONTROL_HEIGHT: f32 = 26.0;
/// A square icon-only button.
pub const ICON_BUTTON: f32 = 28.0;
/// The toolbar's height, and the transport bar's under the preview.
pub const BAR_HEIGHT: f32 = 40.0;

/// The type scale, in points. Body text is 13: dense enough for an editor,
/// large enough to read without leaning in.
pub const TEXT_SMALL: f32 = 11.0;
pub const TEXT_BODY: f32 = 13.0;
pub const TEXT_HEADING: f32 = 15.0;
pub const TEXT_MONO: f32 = 12.5;
/// The playhead's timecode under the preview: the number read most often.
pub const TEXT_TIMECODE: f32 = 15.0;

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
    /// The preview's surround.
    pub canvas: Color32,
    /// A text field's well: darker than the panel it sits in.
    pub field: Color32,
    /// A control's resting fill, and its hairline edge.
    pub control: Color32,
    pub control_edge: Color32,
    /// Under the pointer.
    pub hover: Color32,
    pub hover_edge: Color32,
    /// While the button is held down.
    pub pressed: Color32,
    /// Hairlines between regions and sections.
    pub border: Color32,
    pub accent: Color32,
    /// The accent as text: lighter in the dark theme, deeper in the light.
    pub accent_text: Color32,
    /// Primary text, the brightest text, secondary text and the faintest.
    pub text: Color32,
    pub text_strong: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
}

/// The dark palette: three grounds stepping up from near-black, hairline
/// borders, one soft blue accent, and clip colours deep enough that their
/// thumbnails and waveforms read on them.
pub const DARK: Palette = Palette {
    background: Color32::from_rgb(17, 19, 24),
    panel: Color32::from_rgb(26, 29, 35),
    timeline_background: Color32::from_rgb(20, 22, 27),
    track_header: Color32::from_rgb(29, 32, 39),
    track_lane: Color32::from_rgb(23, 25, 31),
    track_lane_alt: Color32::from_rgb(25, 27, 33),
    grid_line: Color32::from_rgb(40, 44, 52),
    ruler_text: Color32::from_rgb(139, 146, 160),
    video_clip: Color32::from_rgb(44, 84, 150),
    video_clip_top: Color32::from_rgb(92, 134, 214),
    audio_clip: Color32::from_rgb(28, 98, 82),
    audio_clip_top: Color32::from_rgb(64, 178, 140),
    clip_text: Color32::from_rgb(238, 241, 247),
    selection: Color32::from_rgb(247, 190, 80),
    playhead: Color32::from_rgb(242, 88, 88),
    marker: Color32::from_rgb(110, 210, 140),
    keyframe: Color32::from_rgb(130, 196, 255),
    automation: Color32::from_rgb(250, 222, 140),
    transition: Color32::from_rgb(214, 222, 238),
    text_clip: Color32::from_rgb(96, 74, 158),
    text_clip_top: Color32::from_rgb(160, 130, 232),
    in_out_mark: Color32::from_rgb(116, 140, 250),
    in_out_span: Color32::from_rgba_premultiplied(18, 24, 52, 44),
    track_automation: Color32::from_rgb(138, 210, 250),
    rendered: Color32::from_rgb(90, 190, 120),
    rendered_stale: Color32::from_rgb(214, 162, 72),
    fade_handle: Color32::from_rgb(242, 244, 248),
    adjustment_clip: Color32::from_rgb(140, 102, 44),
    adjustment_clip_top: Color32::from_rgb(214, 162, 82),
    disabled: Color32::from_rgb(98, 104, 116),
    error_text: Color32::from_rgb(244, 120, 120),
    caution: Color32::from_rgb(234, 182, 92),
    ok_text: Color32::from_rgb(132, 204, 150),
    canvas: Color32::from_rgb(11, 12, 15),
    field: Color32::from_rgb(18, 20, 25),
    control: Color32::from_rgb(38, 42, 50),
    control_edge: Color32::from_rgb(46, 50, 59),
    hover: Color32::from_rgb(49, 54, 65),
    hover_edge: Color32::from_rgb(62, 68, 81),
    pressed: Color32::from_rgb(58, 64, 78),
    border: Color32::from_rgb(40, 44, 52),
    accent: Color32::from_rgb(104, 128, 246),
    accent_text: Color32::from_rgb(150, 170, 255),
    text: Color32::from_rgb(229, 231, 237),
    text_strong: Color32::from_rgb(246, 247, 250),
    text_muted: Color32::from_rgb(156, 163, 175),
    text_faint: Color32::from_rgb(94, 100, 112),
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
    canvas: Color32::from_rgb(214, 217, 223),
    field: Color32::from_rgb(255, 255, 255),
    control: Color32::from_rgb(236, 238, 242),
    control_edge: Color32::from_rgb(214, 218, 225),
    hover: Color32::from_rgb(224, 228, 235),
    hover_edge: Color32::from_rgb(198, 204, 214),
    pressed: Color32::from_rgb(210, 216, 226),
    border: Color32::from_rgb(218, 222, 229),
    accent: Color32::from_rgb(76, 100, 226),
    accent_text: Color32::from_rgb(52, 76, 196),
    text: Color32::from_rgb(30, 34, 42),
    text_strong: Color32::from_rgb(12, 14, 18),
    text_muted: Color32::from_rgb(98, 106, 120),
    text_faint: Color32::from_rgb(150, 156, 168),
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
    palette().background
}
pub fn panel() -> Color32 {
    palette().panel
}
pub fn timeline_background() -> Color32 {
    palette().timeline_background
}
pub fn track_header() -> Color32 {
    palette().track_header
}
pub fn track_lane() -> Color32 {
    palette().track_lane
}
pub fn track_lane_alt() -> Color32 {
    palette().track_lane_alt
}
pub fn grid_line() -> Color32 {
    palette().grid_line
}
pub fn ruler_text() -> Color32 {
    palette().ruler_text
}
pub fn video_clip() -> Color32 {
    palette().video_clip
}
pub fn video_clip_top() -> Color32 {
    palette().video_clip_top
}
pub fn audio_clip() -> Color32 {
    palette().audio_clip
}
pub fn audio_clip_top() -> Color32 {
    palette().audio_clip_top
}
/// Text that reads on a fill of `colour`: near-black on a light one, white
/// on a dark one. For a clip drawn in its own colours, where the theme's text
/// colour can land dark on dark.
pub fn text_on(colour: Color32) -> Color32 {
    let [r, g, b, _] = colour.to_array();
    let luma = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
    if luma > 150.0 {
        Color32::from_gray(24)
    } else {
        Color32::WHITE
    }
}

pub fn clip_text() -> Color32 {
    palette().clip_text
}
pub fn selection() -> Color32 {
    palette().selection
}
pub fn playhead() -> Color32 {
    palette().playhead
}
pub fn marker() -> Color32 {
    palette().marker
}
pub fn keyframe() -> Color32 {
    palette().keyframe
}
pub fn automation() -> Color32 {
    palette().automation
}
pub fn transition() -> Color32 {
    palette().transition
}
pub fn text_clip() -> Color32 {
    palette().text_clip
}
pub fn text_clip_top() -> Color32 {
    palette().text_clip_top
}
pub fn in_out_mark() -> Color32 {
    palette().in_out_mark
}
pub fn in_out_span() -> Color32 {
    palette().in_out_span
}
pub fn track_automation() -> Color32 {
    palette().track_automation
}
pub fn rendered() -> Color32 {
    palette().rendered
}
pub fn rendered_stale() -> Color32 {
    palette().rendered_stale
}
pub fn fade_handle() -> Color32 {
    palette().fade_handle
}
pub fn adjustment_clip() -> Color32 {
    palette().adjustment_clip
}
pub fn adjustment_clip_top() -> Color32 {
    palette().adjustment_clip_top
}
pub fn disabled() -> Color32 {
    palette().disabled
}
pub fn error_text() -> Color32 {
    palette().error_text
}
pub fn caution() -> Color32 {
    palette().caution
}
pub fn ok_text() -> Color32 {
    palette().ok_text
}

/// Width of the track-name column on the left of the timeline.
pub const TRACK_HEADER_WIDTH: f32 = 156.0;
/// Height of the timecode ruler above the tracks.
pub const RULER_HEIGHT: f32 = 28.0;
/// Height of the overview strip along the bottom of the timeline — the whole
/// edit at a glance, with the visible part marked on it.
pub const OVERVIEW_HEIGHT: f32 = 30.0;
/// A lane at the normal height; `state::LaneHeight` offers smaller and larger.
pub const TRACK_HEIGHT: f32 = 58.0;
pub const TRACK_GAP: f32 = 2.0;
pub const CLIP_CORNER_RADIUS: u8 = 5;

/// Apply the app's visual style. Called at startup and when the theme
/// changes.
///
/// The editor defaults to a dark theme: a preview is judged against its
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
        use egui::{FontFamily, FontId, Stroke, TextStyle};

        style.text_styles = [
            (TextStyle::Small, FontId::new(TEXT_SMALL, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(TEXT_BODY, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(TEXT_BODY, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(TEXT_HEADING, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(TEXT_MONO, FontFamily::Monospace)),
        ]
        .into();

        style.visuals = if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        let p = palette();
        style.visuals.panel_fill = p.panel;
        style.visuals.window_fill = p.panel;
        style.visuals.extreme_bg_color = p.field;
        style.visuals.faint_bg_color = p.control;
        style.visuals.code_bg_color = p.control;
        style.visuals.override_text_color = None;

        // Compact, on the four-point grid: an editor shows a lot at once, and
        // every pixel of padding is a pixel taken from the picture.
        style.spacing.button_padding = egui::vec2(SPACE_M, SPACE_S);
        style.spacing.item_spacing = egui::vec2(SPACE_S + 2.0, SPACE_S + 2.0);
        style.spacing.interact_size = egui::vec2(40.0, CONTROL_HEIGHT);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(12);
        style.spacing.slider_rail_height = 4.0;
        style.spacing.combo_height = 320.0;
        style.spacing.icon_width = 14.0;
        style.spacing.icon_width_inner = 8.0;
        // Thin scroll bars that widen under the pointer, over the content
        // rather than beside it, as a modern desktop app has them.
        style.spacing.scroll = egui::style::ScrollStyle::thin();

        // One accent, used for what is chosen and for the main action, so the
        // eye has one colour to follow. Everything else stays neutral.
        style.visuals.selection.bg_fill = p.accent;
        // The words on a chosen button sit on the accent itself, so white in
        // either theme.
        style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        style.visuals.hyperlink_color = accent_text();
        style.visuals.text_cursor.stroke = Stroke::new(1.5, accent_text());

        // Controls: a quiet fill and a hairline edge at rest, lifted under the
        // pointer, the accent while pressed. Text in the secondary colour at
        // rest and the primary under the pointer, so a row of buttons reads
        // as a row rather than as twenty things shouting.
        let radius = egui::CornerRadius::same(RADIUS_SMALL);
        let widgets = &mut style.visuals.widgets;

        widgets.noninteractive.bg_fill = p.panel;
        widgets.noninteractive.weak_bg_fill = p.panel;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        widgets.noninteractive.corner_radius = radius;

        widgets.inactive.weak_bg_fill = p.control;
        widgets.inactive.bg_fill = p.control;
        widgets.inactive.bg_stroke = Stroke::new(1.0, p.control_edge);
        widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
        widgets.inactive.corner_radius = radius;
        widgets.inactive.expansion = 0.0;

        widgets.hovered.weak_bg_fill = p.hover;
        widgets.hovered.bg_fill = p.hover;
        widgets.hovered.bg_stroke = Stroke::new(1.0, p.hover_edge);
        widgets.hovered.fg_stroke = Stroke::new(1.5, p.text_strong);
        widgets.hovered.corner_radius = radius;
        widgets.hovered.expansion = 0.0;

        widgets.active.weak_bg_fill = p.pressed;
        widgets.active.bg_fill = p.accent;
        widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
        widgets.active.fg_stroke = Stroke::new(1.5, p.text_strong);
        widgets.active.corner_radius = radius;
        widgets.active.expansion = 0.0;

        widgets.open.weak_bg_fill = p.hover;
        widgets.open.bg_fill = p.hover;
        widgets.open.bg_stroke = Stroke::new(1.0, p.hover_edge);
        widgets.open.fg_stroke = Stroke::new(1.0, p.text_strong);
        widgets.open.corner_radius = radius;

        style.visuals.window_corner_radius = egui::CornerRadius::same(RADIUS_LARGE);
        style.visuals.menu_corner_radius = egui::CornerRadius::same(RADIUS);
        style.visuals.window_stroke = Stroke::new(1.0, p.border);
        style.visuals.window_shadow = egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if light { 40 } else { 110 }),
        };
        style.visuals.popup_shadow = egui::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(if light { 32 } else { 90 }),
        };
        style.visuals.window_highlight_topmost = false;
        style.visuals.collapsing_header_frame = false;
        style.visuals.indent_has_left_vline = false;
        style.visuals.striped = false;
        style.visuals.slider_trailing_fill = true;
        style.visuals.handle_shape = egui::style::HandleShape::Circle;
        style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    });
}

/// The fill and the icon colour for an icon button in `response`'s state:
/// nothing behind it at rest, a lifted fill under the pointer, the accent's
/// wash when it is switched on.
pub fn icon_button_colours(
    response: &egui::Response,
    selected: bool,
    enabled: bool,
) -> (Color32, Color32) {
    let p = palette();
    if !enabled {
        return (Color32::TRANSPARENT, p.text_faint);
    }
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered();
    match (selected, pressed, hovered) {
        (true, _, true) => (p.accent.gamma_multiply(0.42), p.text_strong),
        (true, _, false) => (p.accent.gamma_multiply(0.3), accent_text()),
        (false, true, _) => (p.pressed, p.text_strong),
        (false, false, true) => (p.hover, p.text_strong),
        (false, false, false) => (Color32::TRANSPARENT, p.text_muted),
    }
}

/// The one accent: selection, the main action, what is switched on.
pub fn accent() -> Color32 {
    palette().accent
}

/// The accent as text on the panel colour: a little lighter, to read.
pub fn accent_text() -> Color32 {
    palette().accent_text
}

/// Primary text: names, values, what is being read.
pub fn text() -> Color32 {
    palette().text
}

/// Secondary text: labels beside controls, captions, hints.
pub fn text_muted() -> Color32 {
    palette().text_muted
}

/// The brightest text: what is chosen, under the pointer.
pub fn text_strong() -> Color32 {
    palette().text_strong
}

/// The faintest text: hints that are there to be found, not read.
pub fn text_faint() -> Color32 {
    palette().text_faint
}

/// Hairline borders between regions.
pub fn border() -> Color32 {
    palette().border
}

/// A control's resting fill.
pub fn control() -> Color32 {
    palette().control
}

/// The fill under the pointer.
pub fn hover() -> Color32 {
    palette().hover
}

/// The preview's surround: darker than any panel, so the picture is the
/// brightest thing on the screen.
pub fn canvas() -> Color32 {
    palette().canvas
}

/// The screen's main action — Export — in the accent, so there is one thing
/// to find when the work is done.
pub fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(text)
            .color(Color32::WHITE)
            .family(strong_family()),
    )
    .fill(accent())
    .stroke(egui::Stroke::NONE)
    .min_size(egui::vec2(0.0, ICON_BUTTON))
}

/// The family for strong text: the system's semibold when it was found
/// (`crate::fonts`), the ordinary face otherwise.
pub fn strong_family() -> egui::FontFamily {
    if crate::fonts::strong_bound() {
        egui::FontFamily::Name(crate::fonts::STRONG.into())
    } else {
        egui::FontFamily::Proportional
    }
}

/// Text in the strong family at `size`.
pub fn strong(text: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(text).family(strong_family()).size(size)
}

/// A control's name, at the start of its row and a fixed width, so the
/// sliders beside it line up in one column: label, slider, value.
pub fn row_label(ui: &mut egui::Ui, text: impl Into<String>) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(LABEL_WIDTH, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(LABEL_WIDTH);
            ui.add(
                egui::Label::new(egui::RichText::new(text.into()).color(text_muted())).truncate(),
            );
        },
    );
}

/// How wide a row's label column is.
pub const LABEL_WIDTH: f32 = 96.0;

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

/// A section's heading: small capitals in the secondary text colour with a
/// hairline beneath, so a long panel reads as a few named parts.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(SPACE_M);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(TEXT_SMALL)
            .family(strong_family())
            .color(text_muted()),
    );
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0, border());
    ui.add_space(SPACE_XS);
}

/// A bar's frame — the toolbar, the transport, the timeline's tools, the
/// status line: `fill`, a little room left and right, nothing else. The
/// panel's own separator line is the edge.
pub fn bar_frame(fill: Color32) -> egui::Frame {
    egui::Frame::NONE
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(10, 0))
}

/// A side panel's frame: the panel ground and even margins.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(panel())
        .inner_margin(egui::Margin::same(10))
}

/// A thin vertical rule between groups in a bar: the toolbar's, the
/// transport's, the timeline's.
pub fn bar_divider(ui: &mut egui::Ui) {
    let height = ICON_BUTTON - 10.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SPACE_M + 1.0, height), egui::Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(1.0, border()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reads_on_any_fill() {
        assert_eq!(text_on(Color32::from_rgb(12, 12, 40)), Color32::WHITE);
        assert_eq!(text_on(Color32::from_rgb(52, 72, 168)), Color32::WHITE);
        assert_ne!(text_on(Color32::from_rgb(250, 240, 200)), Color32::WHITE);
        assert_ne!(text_on(Color32::WHITE), Color32::WHITE);
    }

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
