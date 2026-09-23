//! View state. None of this is project data and none of it is saved.

use std::collections::{HashMap, HashSet};

use bettercut_editor_core::foundation::{ClipId, TimelineTime, TrackId};
use bettercut_editor_core::timeline::{SnapTarget, TimelineRange};

/// What a drag on a clip is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
    /// Ctrl+Alt: the clip stays put and plays a different part of its file.
    Slip,
    /// Ctrl+Alt on a cut between two touching clips: the cut moves, one clip
    /// growing as the other shrinks.
    Roll,
    /// Ctrl+Shift: the clip moves and its neighbours give way, so nothing
    /// after it moves at all.
    Slide,
}

/// A roll in progress: which cut, and where it is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollDrag {
    /// The clip before the cut.
    pub left: ClipId,
    /// Where the cut was when the drag began.
    pub cut: TimelineTime,
    /// Where it will be, already held to what both files allow.
    pub to: TimelineTime,
}

/// A drag in progress.
///
/// The edit is **not** dispatched until the mouse is released. Dispatching per
/// frame would push sixty commands onto the undo stack for one gesture, so a
/// single drag has to undo in one step — which means holding a preview here and
/// committing once (§11).
#[derive(Debug, Clone)]
pub struct DragState {
    pub clip: ClipId,
    pub source_track: TrackId,
    pub mode: DragMode,
    /// Ticks between the clip's start and where the pointer grabbed it, so the
    /// clip does not jump to centre itself on the cursor.
    pub grab_offset: i64,
    /// Where the clip was before the drag, for the preview and for cancelling.
    pub original: TimelineRange,
    /// Live preview position.
    pub preview: TimelineRange,
    /// Set once the pointer actually moves.
    ///
    /// The drag is captured on *press*, not on egui's `drag_started`, because
    /// by the time a drag is recognised the pointer may already have left the
    /// clip — which is exactly what happens with a fast mouse on a short clip.
    /// A press that never moves is a click, and this flag tells them apart.
    pub moved: bool,
    /// Track under the pointer right now — may differ from `source_track`.
    pub target_track: TrackId,
    /// True when the target track cannot accept this clip (wrong kind).
    pub target_invalid: bool,
    /// What the preview snapped to, if anything. Drawn as a guide line.
    pub snapped_to: Option<SnapTarget>,
    /// §12's linked partners — a video's sound, or a sound's picture — each
    /// with its track and where it was when the drag began.
    ///
    /// Captured at the press, because the editor moves them on release and the
    /// drag's own preview is the only thing that can show where they are going
    /// in the meantime. Without it the sound jumps when the mouse is let go,
    /// which reads as a glitch rather than as a link.
    pub partners: Vec<(TrackId, TimelineRange)>,
    /// A slip so far, in ticks of source (later when positive), already held
    /// inside the file. Zero for every other kind of drag.
    pub slip: i64,
    /// Set for a roll; `None` for every other kind of drag.
    pub roll: Option<RollDrag>,
    /// Shift held on a move: the drop pushes whatever is there along rather
    /// than having to land in free space.
    pub insert: bool,
    /// Shift held on a trim: this edge moves alone, leaving its linked partner
    /// where it is — the J and L cuts (`editor_core::split_edit`).
    pub alone: bool,
}

impl DragState {
    /// Where a partner will land, given where the dragged clip is going.
    ///
    /// By the same *delta*, exactly as the editor applies it (§12), so the
    /// ghost is where the clip will actually end up rather than an
    /// approximation of it.
    pub fn partner_preview(&self, partner: TimelineRange) -> TimelineRange {
        let shift = |t: TimelineTime, by: i64| TimelineTime::from_ticks(t.ticks() + by);
        match self.mode {
            DragMode::Move => {
                let by = self.preview.start.ticks() - self.original.start.ticks();
                TimelineRange {
                    start: shift(partner.start, by),
                    end: shift(partner.end, by),
                }
            }
            DragMode::TrimStart => TimelineRange {
                start: shift(
                    partner.start,
                    self.preview.start.ticks() - self.original.start.ticks(),
                ),
                end: partner.end,
            },
            DragMode::TrimEnd => TimelineRange {
                start: partner.start,
                end: shift(
                    partner.end,
                    self.preview.end.ticks() - self.original.end.ticks(),
                ),
            },
            // A slip moves nothing on the timeline, the partner included.
            // The partners' own ends roll too, but the ghost of the clip
            // being held already shows the cut; theirs stay where they are.
            DragMode::Slip | DragMode::Roll => partner,
            // A slide carries its partner along, exactly as a move does.
            DragMode::Slide => {
                let by = self.preview.start.ticks() - self.original.start.ticks();
                TimelineRange {
                    start: shift(partner.start, by),
                    end: shift(partner.end, by),
                }
            }
        }
    }
}

/// Zoom ladder, in ticks per pixel.
///
/// **Integers, deliberately.** §74 forbids floating point in timeline position
/// arithmetic, and the pixel↔tick mapping is exactly that. With an integer
/// ladder, `tick_of_x(x_of_tick(t)) == t` up to one pixel, always, with no
/// accumulated error as the user zooms in and out repeatedly.
///
/// At 960,000 ticks/second, 32,000 ticks/px is 30 pixels per second.
pub const ZOOM_LEVELS: [i64; 16] = [
    500, 1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 64_000, 128_000, 256_000, 512_000, 960_000,
    1_920_000, 3_840_000, 7_680_000, 15_360_000,
];

/// Index into [`ZOOM_LEVELS`]: 32,000 ticks/px = 30 px/second.
pub const DEFAULT_ZOOM_INDEX: usize = 6;

/// A transient message in the status bar.
#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

/// What playback is actually doing, for the System panel (§49, §52).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackStats {
    pub playing: bool,
    /// §47a.4: frames arriving too late to show. Non-zero while playing means
    /// the machine is not keeping up.
    pub dropped_frames: u64,
    /// §20a: the audio device ran out of samples. Audible, and the one number
    /// that must stay at zero.
    pub underruns: u32,
    /// §20a.4: samples the limiter had to clamp — the mix is too hot.
    pub limited_samples: u64,
    /// §20a: the last block's peak level per side, 0.0 to 1.0 or beyond.
    pub peaks: (f32, f32),
    /// What the mix measures while it plays: the last three seconds and the
    /// last 400 ms, in LUFS (`bettercut_audio::loudness`). `None` where there
    /// is nothing to report — stopped, or below the silence gate.
    pub loudness: (Option<f32>, Option<f32>),
    /// §47a.3: frames served from the decode-ahead ring rather than decoded
    /// inline. Zero while playing means decode-ahead is not helping.
    pub prefetch_hits: u64,
    /// Frames currently waiting in that ring.
    pub ring_frames: usize,
    /// §16's preview scale in force right now.
    pub quality: &'static str,
}

/// How long a lane meter takes to fall by a factor of e, in seconds.
///
/// Fast enough to follow a line ending, slow enough that the eye reads a level
/// rather than a flicker. Rise is instant: a meter that lags the sound is
/// worse than none, because it points at the wrong word.
pub const METER_FALL_SECONDS: f32 = 0.35;

/// One meter's next reading: the live level if it is louder, otherwise the
/// old one on its way down.
///
/// `seconds` is the time since the last frame, so the fall is the same however
/// fast the interface is drawing.
pub fn fallen(shown: f32, live: f32, seconds: f32) -> f32 {
    if !live.is_finite() || !shown.is_finite() {
        return 0.0;
    }
    if live >= shown {
        return live.max(0.0);
    }
    let decay = (-seconds.max(0.0) / METER_FALL_SECONDS).exp();
    let next = shown * decay;
    // Below this nothing is drawn anyway, and a level that never quite reaches
    // zero is a meter that never quite goes out.
    if next < 1e-4 {
        live.max(0.0)
    } else {
        next.max(live)
    }
}

/// Which end of a clip a fade handle belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeEdge {
    In,
    Out,
}

/// A fade handle in the middle of a drag.
#[derive(Debug, Clone, Copy)]
pub struct FadeDrag {
    pub clip: ClipId,
    pub edge: FadeEdge,
    /// The clip's span when the drag began: the pointer is measured from its
    /// edge, and the clip does not move while its fade is being dragged.
    pub range: bettercut_editor_core::timeline::TimelineRange,
    /// Set once the pointer moves, so the first change starts the undo step
    /// and the rest of the drag joins it.
    pub moved: bool,
    /// How far, in ticks, the press was from where the fade actually ends.
    ///
    /// A handle at no fade sits a few pixels inside the corner so it can be
    /// grabbed; measuring the fade from the pointer rather than from the grab
    /// would jump it by those pixels the moment the drag began.
    pub grab_offset: i64,
}

/// Dragging one point of a track's volume line.
#[derive(Debug, Clone)]
pub struct TrackEnvelopeDrag {
    pub track: bettercut_editor_core::foundation::TrackId,
    /// Which point of the line is being moved.
    pub index: usize,
    /// The line as it is now, which each frame of the drag rewrites.
    pub points: Vec<bettercut_editor_core::timeline::VolumePoint>,
    /// The lane the line is drawn across, which says what a height means.
    pub rect: egui::Rect,
    /// Whether anything has moved yet, so the first write opens an undo step
    /// and the rest join it.
    pub moved: bool,
}

/// Dragging one point of a sound clip's volume envelope (§24).
///
/// The whole envelope is carried, as it was when the drag started, because the
/// envelope is written as one command: each frame moves one point in this copy
/// and sends the lot. Carrying the *original* also means the drag is built
/// against the shape before it began, which is what lets the frames coalesce
/// into one undo step.
#[derive(Debug, Clone)]
pub struct EnvelopeDrag {
    pub clip: ClipId,
    /// Which point is being moved.
    pub index: usize,
    /// The envelope as it is now, in timeline instants and levels.
    pub points: Vec<(bettercut_editor_core::foundation::TimelineTime, f32)>,
    /// The clip's rect when the drag began, which is what turns the pointer's
    /// height into a level. Captured once: after the first frame the pointer
    /// has usually left the dot, and the draw pass then reports no hit to read
    /// a rect from.
    pub rect: egui::Rect,
    /// Set once the pointer moves, so the first frame starts the undo step and
    /// the rest join it. A press that never moves is not an edit at all.
    pub moved: bool,
}

/// A keyframe being dragged on the animation graph (`panels::keyframe_graph`).
///
/// The edit is dispatched on release, like every other drag, so moving a key
/// across the graph is one undo step rather than one per frame.
#[derive(Debug, Clone, Copy)]
pub struct KeyframeDrag {
    pub clip: ClipId,
    pub parameter: bettercut_editor_core::timeline::AnimatedParameter,
    /// Where the key was when the drag began — how it is found again.
    pub from: bettercut_editor_core::foundation::MediaTime,
    /// Where it is being dragged to, and what value it will hold.
    pub to: bettercut_editor_core::foundation::MediaTime,
    pub value: f32,
    /// Set once the pointer moves: a press that never moves is a click.
    pub moved: bool,
}

/// A rubber-band selection in progress.
///
/// Held as screen positions rather than a time range because it is drawn as a
/// rectangle: converting to time happens once, on release, when it is used.
#[derive(Debug, Clone, Copy)]
pub struct Marquee {
    pub origin: egui::Pos2,
    pub current: egui::Pos2,
    /// Ctrl was held when the drag started: add to the selection rather than
    /// replacing it, matching Ctrl+click.
    pub additive: bool,
    /// Set once the pointer actually moves. A press that never moves is a
    /// click, and must still move the playhead rather than clearing it.
    pub moved: bool,
}

impl Marquee {
    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_two_pos(self.origin, self.current)
    }
}

/// Where a right-click landed on the timeline.
///
/// Three cases, because the useful actions differ completely: a clip, a track's
/// header, or empty canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTarget {
    Clip {
        clip: ClipId,
        track: TrackId,
    },
    TrackHeader {
        track: TrackId,
    },
    Empty {
        at: bettercut_editor_core::foundation::TimelineTime,
        /// The lane the click landed on, when it landed on one — what "Close
        /// Gap" closes a gap on.
        track: Option<TrackId>,
    },
}

/// What the media browser sorts its files by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaSort {
    /// The order they were imported in — what the browser has always shown,
    /// and the one that keeps a shoot in the order it was shot.
    #[default]
    Added,
    Name,
    Duration,
    /// Video, then sound, then photos: the files for one job in the order they
    /// are usually reached for.
    Kind,
}

impl MediaSort {
    pub const ALL: [Self; 4] = [Self::Added, Self::Name, Self::Duration, Self::Kind];

    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Name => "Name",
            Self::Duration => "Length",
            Self::Kind => "Kind",
        }
    }
}

/// How the media browser lays its files out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaView {
    /// A card each, with a thumbnail to skim through.
    #[default]
    Cards,
    /// One line each: more files on screen at once, which is what a long
    /// import needs.
    List,
}

impl MediaView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cards => "Cards",
            Self::List => "List",
        }
    }
}

/// Which kind of file the media browser shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaFilter {
    #[default]
    All,
    Video,
    Sound,
    Photos,
}

impl MediaFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Video, Self::Sound, Self::Photos];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Video => "Video",
            Self::Sound => "Sound",
            Self::Photos => "Photos",
        }
    }

    /// Whether a file of `kind` passes.
    pub fn accepts(self, kind: bettercut_editor_core::media::MediaKind) -> bool {
        use bettercut_editor_core::media::MediaKind;
        match self {
            Self::All => true,
            Self::Video => kind == MediaKind::Video,
            Self::Sound => kind == MediaKind::Audio,
            Self::Photos => kind == MediaKind::Image,
        }
    }
}

/// Whether a file named `name` matches what was typed: every word of `query`
/// somewhere in the name, in any order and any case. An empty query matches
/// everything.
pub fn media_name_matches(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    query
        .split_whitespace()
        .all(|word| name.contains(&word.to_lowercase()))
}

/// How tall the timeline's lanes are.
///
/// Compact fits a many-track edit on a laptop screen; tall gives waveforms and
/// filmstrips room to be read. A view setting, not part of the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaneHeight {
    Compact,
    #[default]
    Normal,
    Tall,
}

impl LaneHeight {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Normal, Self::Tall];

    /// Pixels, not counting the gap between lanes.
    pub fn pixels(self) -> f32 {
        match self {
            Self::Compact => 36.0,
            Self::Normal => crate::theme::TRACK_HEIGHT,
            Self::Tall => 100.0,
        }
    }

    /// The button's letter and what it means.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Compact => ("S", "Short tracks: more of them on screen"),
            Self::Normal => ("M", "Normal tracks"),
            Self::Tall => ("L", "Tall tracks: bigger waveforms and pictures"),
        }
    }
}

/// How big the preview draws the picture.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PreviewZoom {
    /// As large as fits, the whole frame in view.
    #[default]
    Fit,
    /// A share of the sequence's own pixels: 1.0 is one frame pixel to one
    /// screen pixel.
    Scale(f32),
}

impl PreviewZoom {
    /// What the menu offers, in order.
    pub const CHOICES: [Self; 4] = [
        Self::Fit,
        Self::Scale(0.5),
        Self::Scale(1.0),
        Self::Scale(2.0),
    ];

    pub fn label(self) -> String {
        match self {
            Self::Fit => "Fit".to_owned(),
            Self::Scale(scale) => format!("{:.0}%", scale * 100.0),
        }
    }

    /// The next step in or out from here, for Ctrl + scroll. From Fit, in
    /// goes to 100% and out stays at Fit.
    pub fn step(self, inwards: bool) -> Self {
        let scales = [0.25_f32, 0.5, 1.0, 2.0, 4.0];
        match (self, inwards) {
            (Self::Fit, true) => Self::Scale(1.0),
            (Self::Fit, false) => Self::Fit,
            (Self::Scale(now), true) => scales
                .iter()
                .find(|s| **s > now + 1e-3)
                .map_or(self, |s| Self::Scale(*s)),
            (Self::Scale(now), false) => scales
                .iter()
                .rev()
                .find(|s| **s < now - 1e-3)
                .map_or(Self::Fit, |s| Self::Scale(*s)),
        }
    }
}

/// Lines drawn over the preview to help place things. A view setting, never
/// in the picture or the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewGuide {
    #[default]
    Off,
    /// The rule of thirds.
    Thirds,
    /// Action- and title-safe margins: 90% and 80% of the frame.
    TitleSafe,
    /// Where a phone app draws its own buttons and captions over a vertical
    /// video: keep words and faces out of the shaded parts.
    Social,
    /// A small cross in the middle of the frame, for eyelines, symmetry and
    /// lining a shot up with the one before it.
    ///
    /// A cross rather than two lines across the whole picture: the middle is
    /// what is being looked at, and lines through the subject's face are in
    /// the way of the thing they are meant to help place.
    Centre,
}

impl PreviewGuide {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Thirds,
        Self::TitleSafe,
        Self::Social,
        Self::Centre,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "No guides",
            Self::Thirds => "Thirds",
            Self::TitleSafe => "Title safe",
            Self::Social => "Phone app UI",
            Self::Centre => "Centre",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Off => "Just the picture",
            Self::Thirds => "Lines at a third and two thirds, for placing faces and horizons",
            Self::TitleSafe => "Keep action inside the outer box and words inside the inner one",
            Self::Social => "Shades where TikTok, Reels and Shorts put their buttons and captions",
            Self::Centre => "A cross through the middle, for eyelines and symmetry",
        }
    }
}

#[derive(Debug)]
pub struct UiState {
    zoom_index: usize,
    /// How far a movement travels when one is given
    /// (`bettercut_timeline::MovementStrength`), and whether a montage
    /// alternates its direction. Both are choices about the *next* movement,
    /// not state of the edit, so they live here.
    pub movement_strength: bettercut_editor_core::timeline::MovementStrength,
    pub movement_alternates: bool,
    /// Guide lines over the preview.
    pub preview_guide: PreviewGuide,
    /// The playhead's timecode drawn over the preview — for a screen
    /// recording sent for notes. Never in the export.
    pub preview_timecode: bool,
    /// How big the preview draws the picture, and how far it has been panned
    /// from the middle, in screen points.
    pub preview_zoom: PreviewZoom,
    pub preview_pan: egui::Vec2,
    /// How tall the timeline's lanes are drawn.
    pub lane_height: LaneHeight,
    /// Lanes drawn at a height of their own — one tall for its waveform
    /// while the rest stay short. Anything not here follows `lane_height`.
    pub lane_heights: HashMap<TrackId, LaneHeight>,
    /// Timeline tick at the left edge of the canvas.
    pub scroll_ticks: i64,

    pub selected_clips: HashSet<ClipId>,
    pub selected_track: Option<TrackId>,

    pub status: Option<StatusMessage>,

    /// Font families offered by §26's family picker.
    ///
    /// Read once from the rasterizer, because it is a fact about the machine
    /// rather than about the project and enumerating font directories is not
    /// something to do while drawing a frame. Empty until the preview exists,
    /// which leaves the picker with the three generic names — still usable,
    /// and it fills in on the next launch.
    pub font_families: Vec<String>,
    /// A font file picked with Import Font, waiting to be copied in and loaded.
    pub font_import: Option<std::path::PathBuf>,
    /// A font just imported, to set on the title being edited.
    pub font_imported: Option<String>,

    /// What has been typed into the family picker's filter.
    ///
    /// A machine can have three hundred families and the one you want is
    /// rarely near the top.
    pub font_filter: String,

    /// Live playback counters, mirrored each frame while a preview exists.
    ///
    /// §52's benchmarks and §81's targets are numbers, and the first report
    /// from real hardware came back as "it felt smooth" — because nothing put
    /// these on screen. They are counted either way; showing them costs a few
    /// lines and turns an adjective into evidence.
    pub playback: Option<PlaybackStats>,
    /// What the interface remembers about itself (`crate::prefs`): the theme,
    /// the recent colours.
    pub prefs: crate::prefs::UserPrefs,
    /// Which colour button last changed, and when: a drag through the picker
    /// is one pick (`crate::swatches`).
    pub swatch_last: Option<(egui::Id, f64)>,
    /// What the meters in the sound track heads show, per lane, per side.
    ///
    /// Not the live reading but a falling one: a peak meter that followed the
    /// mix exactly would flicker at sixty frames a second and read as noise.
    /// It rises the instant the sound does and falls back over a moment, which
    /// is what makes a level readable (`settle_lane_levels`).
    pub lane_levels: Vec<(f32, f32)>,

    /// §10 "Snapping". On by default; hold Alt during a drag to bypass it,
    /// which is the convention every editor uses and the fastest way to place
    /// something deliberately off-grid.
    pub snapping: bool,

    /// The drag in progress, if any.
    pub drag: Option<DragState>,

    /// Which page of the Inspector's clip section is showing.
    pub inspector_tab: crate::panels::InspectorTab,

    /// A move or scale being dragged out on the preview.
    pub preview_drag: Option<crate::preview_overlay::PreviewDrag>,

    /// The composited picture no longer matches the project.
    ///
    /// The preview re-renders when the playhead moves, which is the common
    /// case and cheap to detect. It cannot see an edit that changes how a clip
    /// looks without moving anything — a slider, a drag on the picture, a
    /// keyframe, a hidden track — so the event stream says so and the shell
    /// clears it.
    pub preview_is_stale: bool,

    /// A rubber-band selection being dragged out (§10 "Multi-select clips").
    pub marquee: Option<Marquee>,

    /// The Export window's state (Milestone 6). Kept here rather than in the
    /// panel so it survives between frames and so the shell, which owns the
    /// job scheduler, can read what the user chose.
    pub export_dialog: crate::export_dialog::ExportDialog,

    /// The Templates window (§31). Here so a half-filled template survives
    /// closing the window to go and import the clip it was missing.
    pub template_dialog: crate::template_dialog::TemplateDialog,

    /// The Shortcuts window (`?` or F1).
    pub shortcuts_open: bool,
    /// What is typed in its search box. Kept while the window is closed, so
    /// reopening it to check the same key again needs no retyping.
    pub shortcut_search: String,

    /// A volume point being dragged on the timeline (§24).
    pub envelope_drag: Option<EnvelopeDrag>,
    /// Dragging one point of a *track's* volume line
    /// (`bettercut_timeline::track_volume`). The line is carried as it was
    /// when the drag began, for the reason [`EnvelopeDrag`] gives.
    pub track_envelope_drag: Option<TrackEnvelopeDrag>,

    /// A press on the overview strip is panning the view; the timeline itself
    /// must ignore that drag.
    pub overview_drag: bool,

    /// Which parameter the Animation tab's graph is showing. `None` means the
    /// first one the clip animates, so the graph always has something in it.
    pub graph_parameter: Option<bettercut_editor_core::timeline::AnimatedParameter>,
    /// A key being dragged on that graph.
    pub graph_drag: Option<KeyframeDrag>,

    /// A fade handle being dragged at a sound clip's corner.
    pub fade_drag: Option<FadeDrag>,

    /// The Captions window: the list of captions, editable in place.
    pub captions_open: bool,
    /// The Markers window (`crate::marker_list`).
    pub markers_open: bool,
    /// A sequence name being typed in its tab's menu, applied when the menu
    /// closes or the field is left.
    pub sequence_name_draft: Option<(bettercut_editor_core::foundation::SequenceId, String)>,
    /// A clip note being typed in the Inspector, and which clip it is for —
    /// applied when the field is left, or when another clip is selected.
    pub note_draft: Option<(ClipId, String)>,
    /// What the media browser is filtered to: typed words, and a kind.
    pub media_search: String,
    pub media_kind: MediaFilter,
    /// Show only files with at least this many stars. Zero shows them all.
    pub media_stars: u8,
    /// Show only files no clip in the project uses yet.
    pub media_unused_only: bool,
    /// Text to put on the clipboard at the end of the frame, for actions
    /// that run where there is no `egui::Context` to hand.
    pub copy_out: Option<String>,
    /// What is waiting on "Save changes?" (`crate::save_prompt`).
    pub pending_switch: Option<crate::save_prompt::Switch>,
    /// Set while going on after "Don't Save": the unsaved-work check passes.
    pub discard_ok: bool,
    /// The question was answered for closing the window: close it now.
    pub quit_now: bool,
    /// The status bar's Undo was pressed.
    pub undo_request: bool,
    /// The status message as last drawn, whether it came with a new undo
    /// step, and the history's depth last frame (`panels::status_bar`).
    pub status_seen: Option<String>,
    pub status_from_edit: bool,
    pub last_undo_depth: usize,
    /// The first-run welcome window (`crate::welcome`).
    pub welcome_open: bool,
    /// The after-an-update window (`crate::whats_new`).
    pub whats_new_open: bool,
    /// The command palette (Ctrl+K): open, what is typed, and which row is
    /// highlighted.
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_pick: usize,
    /// The palette's recently run actions, most recent first.
    pub palette_recent: Vec<&'static str>,
    /// The Captions window's find and replace fields, and whether case counts.
    pub caption_find: String,
    pub caption_replace: String,
    pub caption_match_case: bool,

    /// What the browser sorts by, and whether that order is reversed.
    pub media_sort: MediaSort,
    pub media_sort_reversed: bool,
    /// Cards with thumbnails, or one line per file.
    pub media_view: MediaView,
    /// The part of each file marked in the browser: where the in-point is,
    /// and the out-point once a second click has set one (`Editor::
    /// place_media_range`). Session state — a mark is a way of looking at a
    /// file, not a change to the project.
    pub media_marks: HashMap<
        bettercut_editor_core::foundation::MediaId,
        (
            bettercut_editor_core::foundation::MediaTime,
            Option<bettercut_editor_core::foundation::MediaTime>,
        ),
    >,

    /// A file to point out in the browser — "find this clip's file" (§66's
    /// relink is the other way round: this one starts from the timeline).
    /// Drawn with a frame around it and scrolled to, until the search changes.
    pub media_reveal: Option<bettercut_editor_core::foundation::MediaId>,
    /// Which bin the media browser shows: `None` every file, `Some(None)`
    /// the unfiled ones, `Some(Some(name))` one bin.
    pub media_bin: Option<Option<String>>,
    /// A new bin's name being typed in a file's menu.
    pub new_bin_draft: String,
    /// A track name being typed in the track menu, and which track it is for —
    /// applied when the field is left or the menu closes, as one undo step.
    pub track_name_draft: Option<(TrackId, String)>,
    /// The inset size last picked in "Picture in Picture", offered next time.
    pub pip_size: bettercut_editor_core::PipSize,
    /// A marker name being typed, and which marker it is for — applied when
    /// the field is left, so a rename is one undo step.
    pub marker_draft: Option<(bettercut_editor_core::foundation::TimelineTime, String)>,
    /// A timecode being typed into the readout (`crate::timecode_entry`).
    pub timecode_draft: Option<String>,
    /// A clip's name being typed in its menu, and which clip it is for.
    pub clip_name_draft: Option<(ClipId, String)>,
    /// Where the typed timecode asks the playhead to go: taken by the
    /// transport, which is what has the preview to seek.
    pub jump_to: Option<TimelineTime>,

    /// A frame to save as a PNG, and the instant it was asked for at. Taken by
    /// the shell, which owns the job scheduler the still renders on (§74).
    pub still_request: Option<(std::path::PathBuf, TimelineTime)>,
    /// A contact sheet to render (`bettercut_export::contact_sheet`), taken by
    /// the shell like a still.
    pub contact_sheet_request: Option<std::path::PathBuf>,
    /// The sound-in-detail window (`crate::waveform_view`).
    pub waveform_view: crate::waveform_view::WaveformView,
    /// The storyboard window (`crate::storyboard`).
    pub storyboard_open: bool,
    /// What changed since an earlier save (`crate::version_changes`).
    pub version_changes: crate::version_changes::ChangesView,
    /// The trim window (`crate::trim_view`).
    pub trim: crate::trim_view::TrimView,
    /// A frame to render and put on the clipboard, asked for by "Copy Frame".
    pub copy_frame_request: Option<TimelineTime>,
    /// A stretch to bake (`editor_core::render_in_place`). Taken by the shell,
    /// which owns the scheduler the bake runs on, exactly as a still is.
    pub render_request: Option<bettercut_editor_core::timeline::TimelineRange>,
    /// A sound lane to mix down to one clip (`editor_core::bounce`), taken by
    /// the shell for the same reason.
    pub bounce_request: Option<TrackId>,
    /// What the timeline's find box holds.
    pub find_query: String,
    /// The preview shows every clip without its grade and effects.
    pub compare_original: bool,
    /// Hearing the sound while the playhead is dragged (§10's scrub). On by
    /// default: finding the moment a word lands is what scrubbing is for.
    pub audio_scrub: bool,

    /// What the mix last measured, to show beside the targets
    /// (`crate::loudness`).
    pub loudness_measured: Option<f32>,
    /// The delivery target the live meter is read against, in LUFS. Set by
    /// choosing one in the master row; -14 to begin with, which is what the
    /// sites most of this is made for turn everything down to.
    pub loudness_target: f32,

    /// Dragging the four corners of the selected clip (§45's corner pin)
    /// rather than scaling it.
    pub corner_pin_mode: bool,

    /// Showing the original and the graded picture at once, split down the
    /// frame, and where that divider sits (0–1 across the picture).
    pub compare_split: bool,
    pub compare_split_at: f32,
    /// The preview fills the screen, with every panel hidden (F, Escape out).
    pub fullscreen: bool,
    /// The Lower Third menu's name, role and bar colour, kept while typing.
    pub lower_third: (String, String, [u8; 3]),
    /// Words to read aloud, waiting for the app to start the voice.
    pub speech_request: Option<crate::speech::SpeechRequest>,
    /// The voices installed on this machine, once they have been listed.
    pub speech_voices: Option<Vec<String>>,
    /// The Read Aloud menu has been opened, so the voices are worth listing.
    pub speech_voices_wanted: bool,
    /// The Record Voice button was pressed: start or stop recording.
    pub voiceover_toggle: bool,
    /// While recording: seconds so far and the latest level, 0–1.
    pub voiceover_live: Option<(f64, f32)>,
    /// What the window was last told, so the fullscreen request is sent once
    /// per change rather than every frame.
    pub fullscreen_applied: bool,
    /// A file's name being typed in the media browser, not yet applied.
    pub media_name_draft: Option<(bettercut_editor_core::foundation::MediaId, String)>,
    /// The Scopes window: open or not, what it last measured, what it wants.
    pub scopes: crate::scopes::ScopesState,
    /// A clip to colour match to the frame at the given time, asked for by
    /// "Match Colour to Playhead".
    pub colour_match_request: Option<(bettercut_editor_core::foundation::ClipId, TimelineTime)>,
    /// A clip to grade to a neutral exposure and balance, in the background
    /// like a colour match (`bettercut_export::auto_level`).
    pub auto_level_request: Option<bettercut_editor_core::foundation::ClipId>,
    /// Files whose decoders must be reopened — the way they are read
    /// changed (deinterlacing) — taken by the shell, which has the preview.
    pub media_reopen: Vec<bettercut_editor_core::foundation::MediaId>,

    /// Projects opened or saved lately, for the Open menu. In memory only
    /// until the shell points it at the user's stored list.
    pub recent: crate::recent::RecentProjects,
    /// The user's own keys for the shortcuts.
    pub keymap: crate::keymap::Keymap,
    /// The speed curve being drawn in the Speed tab, and how many pieces it
    /// is cut into when applied. Kept across clips, because the curve someone
    /// drew is usually the curve they want on the next shot too.
    pub speed_curve: bettercut_editor_core::SpeedCurve,
    pub speed_curve_pieces: usize,
    /// The point being dragged on that curve.
    pub speed_curve_drag: Option<usize>,

    /// Grades the user saved to reuse (`crate::looks`).
    pub user_looks: crate::looks::UserLooks,
    /// Title styles of this user's own (`crate::title_styles`), and the name
    /// being typed for the next one.
    pub title_styles: crate::title_styles::UserTitleStyles,
    pub title_style_name_draft: String,
    /// Export settings of this user's own (`crate::export_presets`), and the
    /// name being typed for the next one.
    pub export_presets: crate::export_presets::UserExports,
    pub export_preset_draft: String,
    /// What is typed in the "save this look" box, while it is open.
    pub look_name_draft: String,
    /// A shortcut waiting for its new key, from the Shortcuts window.
    pub rebinding: Option<egui::Key>,
    /// The Shortcuts window shows a key button on every row, to change it.
    pub editing_keys: bool,

    /// The History window (Ctrl+H): every step, and a click to return to one.
    pub history_open: bool,
    /// The Notes window (`crate::notes_panel`), and the text being typed in
    /// it — applied as one step when the window is left.
    pub notes_open: bool,
    pub notes_draft: Option<String>,
    /// The undo depth the History window last drew, so it scrolls to the
    /// current step only when that moves.
    pub history_seen_depth: Option<usize>,
    /// Words typed into the History window's search box.
    pub history_search: String,

    /// Which caption is being typed into, so the keystrokes after the first
    /// join the same undo step and a different caption starts its own.
    pub caption_typing: Option<ClipId>,

    /// Which clip the Effects tab is folded for.
    ///
    /// The sections open on what *this* clip is using, and egui remembers a
    /// header's fold by its id rather than by what is selected — so without
    /// this, choosing a masked clip and then a plain one would leave the mask
    /// section hanging open over a clip that has none.
    pub effects_for: Option<ClipId>,

    /// The eyedropper is armed: the next click on the picture names the green
    /// screen for this clip rather than moving anything (§45).
    ///
    /// Carries the clip so a click after the selection changed cannot key the
    /// wrong one — arming it is about the clip whose key is being set up.
    pub picking_key: Option<ClipId>,

    /// Crop mode: the preview shows this clip's crop edges to drag, in place
    /// of its move, scale and rotate handles (§22).
    ///
    /// A mode rather than always-on handles, because four more handles on
    /// every selected clip would crowd the ones people use most, and a crop is
    /// set once and left. Carries the clip for the reason `picking_key` does:
    /// a selection that changed underneath must not crop the wrong shot.
    pub cropping: Option<ClipId>,

    /// How the next slideshow is made (§33): kept between uses, so someone who
    /// prefers hard cuts and five seconds a photo sets that once.
    pub slideshow: bettercut_editor_core::slideshow::Slideshow,

    /// The groups the next Paste Attributes will take, ticked in its menu.
    pub paste_groups: Vec<bettercut_editor_core::attributes::AttributeGroup>,
    /// A look taken off a clip, waiting to be put onto others (§45).
    ///
    /// The values rather than the clip it came from, so deleting that clip
    /// afterwards does not empty the clipboard — and so the paste is the same
    /// edit whenever it happens.
    pub copied_look: Option<Vec<bettercut_editor_core::ClipProperty>>,
    /// A clip's keyframes, copied to paste onto others.
    pub copied_animation: Option<bettercut_editor_core::animation_copy::CopiedAnimation>,

    /// The Remove Silences window (§78), with its suggestion.
    pub silence: Option<crate::silence_dialog::SilenceDialog>,
    /// The "find the good bits" window (`crate::highlight_dialog`).
    pub highlights: Option<crate::highlight_dialog::HighlightDialog>,

    /// The Find Cuts window (§45), once detection has been started.
    pub scenes: Option<crate::scene_dialog::SceneDialog>,

    /// A clip the user asked to have its cuts found. Picked up by the shell,
    /// because starting a job needs the scheduler and the scheduler is not the
    /// interface's to hold — the same route the Export window takes.
    pub scene_request: Option<ClipId>,

    /// A detection to stop, for the same reason.
    pub scene_cancel: Option<bettercut_jobs::JobId>,

    /// Work found from a session that did not shut down cleanly (§39).
    ///
    /// Held rather than applied: §39.5 says never overwrite the original
    /// project automatically, so the user decides.
    pub pending_recovery: Option<bettercut_editor_core::RecoverableSession>,

    /// Set by any interaction that changed something, so the app knows to
    /// repaint rather than repainting unconditionally (§54, §81).
    pub needs_repaint: bool,

    /// What the last right-click landed on (§58).
    ///
    /// Captured at click time and held while the menu is open: the pointer
    /// moves onto the menu itself immediately, so re-hit-testing when an item
    /// is chosen would act on whatever happens to be under the cursor then.
    pub context: Option<ContextTarget>,

    /// Poster thumbnails, uploaded on demand (§12).
    pub thumbnails: crate::thumbnails::ThumbnailStore,

    /// Audio peaks for drawing waveforms on audio clips (§12, §53).
    pub waveforms: crate::waveforms::WaveformStore,

    /// The graphics adapter in use (§49), set once at startup.
    ///
    /// `None` when wgpu failed entirely and there is no preview (§50).
    pub gpu: Option<bettercut_renderer::GpuDescription>,

    /// Proxies being generated: how many, and how far along (§42).
    ///
    /// Mirrored here rather than reached for through the `ProxyManager`,
    /// because §42 wants background work *visible* and the status bar should
    /// not have to know what a job scheduler is.
    pub proxy_progress: Option<(usize, f32)>,

    /// The running export and its progress, plus whether the user pressed
    /// stop. The shell owns the scheduler, so the button sets a flag here and
    /// the shell acts on it.
    pub export_progress: Option<f32>,
    pub export_stop_requested: bool,
    /// The export queue window and what it shows.
    pub export_queue: crate::export_queue::ExportQueueState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            zoom_index: DEFAULT_ZOOM_INDEX,
            lane_height: LaneHeight::Normal,
            lane_heights: HashMap::new(),
            movement_strength: bettercut_editor_core::timeline::MovementStrength::default(),
            movement_alternates: true,
            preview_guide: PreviewGuide::Off,
            preview_timecode: false,
            preview_zoom: PreviewZoom::Fit,
            preview_pan: egui::Vec2::ZERO,
            scroll_ticks: 0,
            selected_clips: HashSet::new(),
            selected_track: None,
            status: None,
            font_families: Vec::new(),
            font_import: None,
            font_imported: None,
            font_filter: String::new(),
            playback: None,
            prefs: crate::prefs::UserPrefs::default(),
            swatch_last: None,
            lane_levels: Vec::new(),
            snapping: true,
            drag: None,
            inspector_tab: crate::panels::InspectorTab::default(),
            preview_drag: None,
            preview_is_stale: false,
            marquee: None,
            export_dialog: crate::export_dialog::ExportDialog::default(),
            template_dialog: crate::template_dialog::TemplateDialog::default(),
            shortcuts_open: false,
            shortcut_search: String::new(),
            envelope_drag: None,
            track_envelope_drag: None,
            overview_drag: false,
            graph_parameter: None,
            graph_drag: None,
            fade_drag: None,
            captions_open: false,
            markers_open: false,
            pip_size: bettercut_editor_core::PipSize::Medium,
            track_name_draft: None,
            media_search: String::new(),
            note_draft: None,
            sequence_name_draft: None,
            media_kind: MediaFilter::All,
            media_stars: 0,
            media_unused_only: false,
            copy_out: None,
            pending_switch: None,
            discard_ok: false,
            quit_now: false,
            undo_request: false,
            status_seen: None,
            status_from_edit: false,
            last_undo_depth: 0,
            welcome_open: false,
            whats_new_open: false,
            palette_open: false,
            palette_query: String::new(),
            palette_pick: 0,
            palette_recent: Vec::new(),
            caption_find: String::new(),
            caption_replace: String::new(),
            caption_match_case: false,
            media_sort: MediaSort::default(),
            media_sort_reversed: false,
            media_view: MediaView::default(),
            media_reveal: None,
            media_marks: HashMap::new(),
            media_bin: None,
            new_bin_draft: String::new(),
            marker_draft: None,
            timecode_draft: None,
            clip_name_draft: None,
            jump_to: None,
            still_request: None,
            contact_sheet_request: None,
            waveform_view: crate::waveform_view::WaveformView::default(),
            storyboard_open: false,
            version_changes: crate::version_changes::ChangesView::default(),
            trim: crate::trim_view::TrimView::default(),
            render_request: None,
            bounce_request: None,
            copy_frame_request: None,
            find_query: String::new(),
            compare_original: false,
            audio_scrub: true,
            loudness_measured: None,
            loudness_target: -14.0,
            corner_pin_mode: false,
            compare_split: false,
            compare_split_at: 0.5,
            fullscreen: false,
            voiceover_toggle: false,
            speech_request: None,
            lower_third: (
                String::new(),
                String::new(),
                bettercut_editor_core::lower_third::LOWER_THIRD_ACCENT,
            ),
            speech_voices: None,
            speech_voices_wanted: false,
            voiceover_live: None,
            fullscreen_applied: false,
            media_name_draft: None,
            scopes: crate::scopes::ScopesState::default(),
            colour_match_request: None,
            auto_level_request: None,
            media_reopen: Vec::new(),
            recent: crate::recent::RecentProjects::default(),
            keymap: crate::keymap::Keymap::default(),
            speed_curve: bettercut_editor_core::SpeedCurve::default(),
            speed_curve_pieces: 6,
            speed_curve_drag: None,
            user_looks: crate::looks::UserLooks::default(),
            title_styles: crate::title_styles::UserTitleStyles::default(),
            title_style_name_draft: String::new(),
            export_presets: crate::export_presets::UserExports::default(),
            export_preset_draft: String::new(),
            look_name_draft: String::new(),
            rebinding: None,
            editing_keys: false,
            history_open: false,
            notes_open: false,
            notes_draft: None,
            history_seen_depth: None,
            history_search: String::new(),
            caption_typing: None,
            effects_for: None,
            picking_key: None,
            cropping: None,
            slideshow: bettercut_editor_core::slideshow::Slideshow::default(),
            copied_look: None,
            paste_groups: bettercut_editor_core::attributes::AttributeGroup::ALL.to_vec(),
            copied_animation: None,
            silence: None,
            highlights: None,
            scenes: None,
            scene_request: None,
            scene_cancel: None,
            pending_recovery: None,
            needs_repaint: true,
            context: None,
            thumbnails: crate::thumbnails::ThumbnailStore::default(),
            waveforms: crate::waveforms::WaveformStore::default(),
            gpu: None,
            proxy_progress: None,
            export_progress: None,
            export_stop_requested: false,
            export_queue: crate::export_queue::ExportQueueState::default(),
        }
    }
}

impl UiState {
    /// How tall `track`'s lane is drawn: its own height, or the timeline's.
    pub fn lane_height_for(&self, track: TrackId) -> f32 {
        self.lane_heights
            .get(&track)
            .copied()
            .unwrap_or(self.lane_height)
            .pixels()
    }

    /// Give `track` a height of its own, or `None` to follow the others.
    pub fn set_lane_height(&mut self, track: TrackId, height: Option<LaneHeight>) {
        match height {
            Some(height) => {
                self.lane_heights.insert(track, height);
            }
            None => {
                self.lane_heights.remove(&track);
            }
        }
        self.needs_repaint = true;
    }

    pub fn ticks_per_pixel(&self) -> i64 {
        ZOOM_LEVELS[self.zoom_index.min(ZOOM_LEVELS.len() - 1)]
    }

    pub fn zoom_in(&mut self) {
        if self.zoom_index > 0 {
            self.zoom_index -= 1;
            self.needs_repaint = true;
        }
    }

    pub fn zoom_out(&mut self) {
        if self.zoom_index + 1 < ZOOM_LEVELS.len() {
            self.zoom_index += 1;
            self.needs_repaint = true;
        }
    }

    pub fn can_zoom_in(&self) -> bool {
        self.zoom_index > 0
    }

    pub fn can_zoom_out(&self) -> bool {
        self.zoom_index + 1 < ZOOM_LEVELS.len()
    }

    /// Pixels per second at the current zoom, for the ruler's tick spacing.
    pub fn pixels_per_second(&self) -> f32 {
        bettercut_editor_core::foundation::TICKS_PER_SECOND as f32 / self.ticks_per_pixel() as f32
    }

    /// Zoom so that `duration` fits in `width` pixels, picking the nearest
    /// ladder step that is not too tight.
    pub fn zoom_to_fit(&mut self, duration: TimelineTime, width: f32) {
        if duration.is_zero() || width <= 1.0 {
            return;
        }
        let wanted = (duration.ticks() as f32 / width).ceil() as i64;
        self.zoom_index = ZOOM_LEVELS
            .iter()
            .position(|&level| level >= wanted)
            .unwrap_or(ZOOM_LEVELS.len() - 1);
        self.scroll_ticks = 0;
        self.needs_repaint = true;
    }

    /// Zoom and scroll so `range` fills the `width`-pixel lanes, with a tenth
    /// of its length spare either side so its edges are not on the frame.
    /// The nearest ladder step that fits, as [`Self::zoom_to_fit`] picks.
    pub fn zoom_to_range(&mut self, range: TimelineRange, width: f32) {
        let length = range.duration().ticks();
        if length <= 0 || width <= 1.0 {
            return;
        }
        let margin = length / 10;
        let wanted = ((length + 2 * margin) as f32 / width).ceil() as i64;
        self.zoom_index = ZOOM_LEVELS
            .iter()
            .position(|&level| level >= wanted)
            .unwrap_or(ZOOM_LEVELS.len() - 1);
        self.scroll_ticks = (range.start.ticks() - margin).max(0);
        self.needs_repaint = true;
    }

    /// Zoom in on one clip, with a little room either side.
    pub fn zoom_to_clip(
        &mut self,
        editor: &bettercut_editor_core::Editor,
        clip: ClipId,
        width: f32,
    ) {
        if let Some(range) = editor
            .active_sequence()
            .and_then(|sequence| sequence.clip_span(clip))
            .map(|span| span.timeline)
        {
            self.zoom_to_range(range, width);
        }
    }

    /// Zoom in on the selected clips — or, with nothing selected, out to the
    /// whole edit. One key for "show me what I am working on".
    pub fn zoom_to_selection_or_fit(&mut self, editor: &bettercut_editor_core::Editor, width: f32) {
        match self.selection_range(editor) {
            Some(range) => self.zoom_to_range(range, width),
            None => {
                let duration = editor
                    .active_sequence()
                    .map_or(TimelineTime::ZERO, |s| s.duration());
                self.zoom_to_fit(duration, width);
            }
        }
    }

    /// The span the selected clips cover, first start to last end.
    pub fn selection_range(&self, editor: &bettercut_editor_core::Editor) -> Option<TimelineRange> {
        let sequence = editor.active_sequence()?;
        let spans: Vec<TimelineRange> = self
            .selected_clips
            .iter()
            .filter_map(|clip| sequence.clip_span(*clip).map(|span| span.timeline))
            .collect();
        let start = spans.iter().map(|s| s.start).min()?;
        let end = spans.iter().map(|s| s.end).max()?;
        TimelineRange::new(start, end).ok()
    }

    /// Horizontal scroll, in pixels. Clamped at the start of the timeline.
    pub fn scroll_by_pixels(&mut self, pixels: f32) {
        let delta = (pixels as i64).saturating_mul(self.ticks_per_pixel());
        let next = self.scroll_ticks.saturating_add(delta).max(0);
        if next != self.scroll_ticks {
            self.scroll_ticks = next;
            self.needs_repaint = true;
        }
    }

    /// Scroll so `t` is visible, with a margin, if it currently is not.
    pub fn scroll_to_reveal(&mut self, t: TimelineTime, width_pixels: f32) {
        let span = (width_pixels as i64).saturating_mul(self.ticks_per_pixel());
        let margin = span / 8;
        let left = self.scroll_ticks;
        let right = self.scroll_ticks.saturating_add(span);

        if t.ticks() < left {
            self.scroll_ticks = (t.ticks() - margin).max(0);
            self.needs_repaint = true;
        } else if t.ticks() > right {
            self.scroll_ticks = (t.ticks() - span + margin).max(0);
            self.needs_repaint = true;
        }
    }

    /// Put `t` in the middle of the view — what clicking the overview strip
    /// does. Clamped at the start, so a click near zero shows the beginning
    /// rather than scrolling to a negative tick.
    pub fn center_view_on(&mut self, t: TimelineTime, width_pixels: f32) {
        let span = (width_pixels as i64).saturating_mul(self.ticks_per_pixel());
        let next = (t.ticks() - span / 2).max(0);
        if next != self.scroll_ticks {
            self.scroll_ticks = next;
            self.needs_repaint = true;
        }
    }

    /// Keep the playhead on screen while playing, a page at a time.
    ///
    /// Not the same as [`Self::scroll_to_reveal`], which brings a point just
    /// inside the edge it went past. Doing that sixty times a second would
    /// scroll the timeline by a pixel per frame with the playhead pinned to the
    /// right edge — the picture moving under a stationary line, which is
    /// horrible to watch and impossible to read. This pages instead: when the
    /// playhead reaches the last eighth of the view, the view jumps forward so
    /// it starts again near the left.
    pub fn follow_playhead(&mut self, t: TimelineTime, width_pixels: f32) {
        let span = (width_pixels as i64).saturating_mul(self.ticks_per_pixel());
        if span <= 0 {
            return;
        }
        let margin = span / 8;
        let left = self.scroll_ticks;
        let trigger = left.saturating_add(span - margin);

        if t.ticks() < left || t.ticks() >= trigger {
            self.scroll_ticks = (t.ticks() - margin).max(0);
            self.needs_repaint = true;
        }
    }

    /// Show `media` in the browser: the filters are cleared and the search set
    /// to its name, so the file is on screen whatever was being looked at, and
    /// it is marked until the search is changed.
    ///
    /// The name rather than the id, because the search is what the user can
    /// see and then edit — a hidden "only this file" filter would leave them
    /// with a browser that will not show anything else and no clue why.
    pub fn reveal_media(&mut self, media: bettercut_editor_core::foundation::MediaId, name: &str) {
        self.media_search = name.to_owned();
        self.media_kind = MediaFilter::All;
        self.media_bin = None;
        self.media_reveal = Some(media);
        self.needs_repaint = true;
    }

    /// Mark a file at `at`: the in-point first, the out-point next, and a
    /// third click starts again. Returns the marked part, when there is one.
    ///
    /// Two clicks rather than two buttons because the thumbnail is already
    /// being skimmed through with the pointer — the mark belongs where the eye
    /// is, not in a row of controls underneath.
    pub fn mark_media(
        &mut self,
        media: bettercut_editor_core::foundation::MediaId,
        at: bettercut_editor_core::foundation::MediaTime,
    ) -> Option<(
        bettercut_editor_core::foundation::MediaTime,
        bettercut_editor_core::foundation::MediaTime,
    )> {
        self.needs_repaint = true;
        match self.media_marks.get(&media).copied() {
            // A fresh in-point, or starting again after a part was marked.
            None | Some((_, Some(_))) => {
                self.media_marks.insert(media, (at, None));
                None
            }
            Some((from, None)) => {
                let (from, to) = if at < from { (at, from) } else { (from, at) };
                self.media_marks.insert(media, (from, Some(to)));
                Some((from, to))
            }
        }
    }

    /// The part of `media` marked, once both ends are set.
    pub fn marked_part(
        &self,
        media: bettercut_editor_core::foundation::MediaId,
    ) -> Option<(
        bettercut_editor_core::foundation::MediaTime,
        bettercut_editor_core::foundation::MediaTime,
    )> {
        match self.media_marks.get(&media).copied() {
            Some((from, Some(to))) if to > from => Some((from, to)),
            _ => None,
        }
    }

    /// Forget a file's marks, so the whole of it is placed again.
    pub fn clear_media_marks(&mut self, media: bettercut_editor_core::foundation::MediaId) {
        if self.media_marks.remove(&media).is_some() {
            self.needs_repaint = true;
        }
    }

    pub fn select_only(&mut self, clip: ClipId) {
        self.selected_clips.clear();
        self.selected_clips.insert(clip);
        self.needs_repaint = true;
    }

    pub fn toggle_selection(&mut self, clip: ClipId) {
        if !self.selected_clips.remove(&clip) {
            self.selected_clips.insert(clip);
        }
        self.needs_repaint = true;
    }

    pub fn clear_selection(&mut self) {
        if !self.selected_clips.is_empty() {
            self.selected_clips.clear();
            self.needs_repaint = true;
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage {
            text: text.into(),
            is_error: false,
        });
        self.needs_repaint = true;
    }

    /// Snap tolerance in ticks, from a fixed pixel distance.
    ///
    /// Constant in *pixels* so snapping grabs from the same visual distance
    /// whether zoomed to frames or to minutes. Integer throughout (§74).
    pub fn snap_tolerance(&self) -> TimelineTime {
        const GRAB_PIXELS: i64 = 8;
        TimelineTime::from_ticks(GRAB_PIXELS * self.ticks_per_pixel())
    }

    /// Take this frame's lane levels, letting the old ones fall towards them
    /// (`fallen`). `seconds` is the time since the last frame.
    pub fn settle_lane_levels(&mut self, live: &[(f32, f32)], seconds: f32) {
        if self.lane_levels.len() < live.len() {
            self.lane_levels.resize(live.len(), (0.0, 0.0));
        }
        for (lane, shown) in self.lane_levels.iter_mut().enumerate() {
            let (left, right) = live.get(lane).copied().unwrap_or((0.0, 0.0));
            *shown = (
                fallen(shown.0, left, seconds),
                fallen(shown.1, right, seconds),
            );
        }
    }

    /// One lane's meter reading, per side. Silence for a lane that has none.
    pub fn lane_level(&self, lane: usize) -> (f32, f32) {
        self.lane_levels.get(lane).copied().unwrap_or((0.0, 0.0))
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!(%text, "ui error");
        self.status = Some(StatusMessage {
            text,
            is_error: true,
        });
        self.needs_repaint = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: i64, end: i64) -> TimelineRange {
        TimelineRange {
            start: TimelineTime::from_seconds(start),
            end: TimelineTime::from_seconds(end),
        }
    }

    fn drag(mode: DragMode, original: TimelineRange, preview: TimelineRange) -> DragState {
        DragState {
            clip: ClipId::new(),
            source_track: TrackId::new(),
            mode,
            grab_offset: 0,
            original,
            preview,
            moved: true,
            target_track: TrackId::new(),
            target_invalid: false,
            snapped_to: None,
            partners: Vec::new(),
            slip: 0,
            roll: None,
            insert: false,
            alone: false,
        }
    }

    /// §12: the partner's ghost has to land where the editor will put it, by
    /// the same delta. A ghost somewhere else would be worse than none.
    /// A meter rises the instant the sound does: lagging it would point at the
    /// wrong word.
    #[test]
    fn a_meter_rises_at_once() {
        assert_eq!(fallen(0.0, 0.8, 1.0 / 60.0), 0.8);
        assert_eq!(fallen(0.4, 0.4, 1.0 / 60.0), 0.4);
    }

    /// And falls back over a moment, by the same amount however fast the
    /// interface is drawing.
    #[test]
    fn a_meter_falls_at_the_same_rate_whatever_the_frame_rate() {
        let slow = fallen(1.0, 0.0, 0.1);
        let mut fast = 1.0;
        for _ in 0..10 {
            fast = fallen(fast, 0.0, 0.01);
        }
        assert!((slow - fast).abs() < 1e-4, "{slow} then {fast}");
        // One time constant is a fall to about a third.
        let one = fallen(1.0, 0.0, METER_FALL_SECONDS);
        assert!((one - 0.368).abs() < 0.01, "{one}");
    }

    /// It reaches nothing rather than creeping towards it forever: a bar that
    /// never quite goes out reads as sound that is not there.
    #[test]
    fn a_meter_reaches_silence() {
        let mut level = 1.0;
        for _ in 0..200 {
            level = fallen(level, 0.0, 1.0 / 60.0);
        }
        assert_eq!(level, 0.0);
    }

    /// Nonsense in is silence out, never a bar drawn off the top of the head.
    #[test]
    fn a_meter_ignores_what_is_not_a_level() {
        assert_eq!(fallen(0.5, f32::NAN, 0.016), 0.0);
        assert_eq!(fallen(f32::INFINITY, 0.5, 0.016), 0.0);
        assert_eq!(
            fallen(0.5, -1.0, 0.016),
            0.5 * (-0.016_f32 / METER_FALL_SECONDS).exp()
        );
    }

    /// The lanes are kept lane by lane, and one that falls silent falls rather
    /// than sticking where it was.
    #[test]
    fn lane_levels_are_kept_per_lane() {
        let mut state = UiState::default();
        state.settle_lane_levels(&[(0.9, 0.1), (0.2, 0.2)], 0.016);
        assert_eq!(state.lane_level(0), (0.9, 0.1));
        assert_eq!(state.lane_level(1), (0.2, 0.2));
        // A lane nobody reported is silence, not a panic.
        assert_eq!(state.lane_level(7), (0.0, 0.0));

        state.settle_lane_levels(&[(0.0, 0.0)], 0.016);
        let (left, _) = state.lane_level(0);
        assert!(
            left < 0.9 && left > 0.0,
            "the first lane did not fall: {left}"
        );
        assert!(
            state.lane_level(1).0 < 0.2,
            "a lane that stopped being reported held its level"
        );
    }

    #[test]
    fn a_moved_partner_lands_by_the_same_delta() {
        let d = drag(DragMode::Move, range(0, 10), range(4, 14));
        assert_eq!(d.partner_preview(range(0, 10)), range(4, 14));
        // A partner that had drifted keeps its offset, as the editor does.
        assert_eq!(d.partner_preview(range(1, 9)), range(5, 13));
    }

    #[test]
    fn a_trimmed_partner_moves_only_the_same_edge() {
        let start = drag(DragMode::TrimStart, range(0, 10), range(3, 10));
        assert_eq!(start.partner_preview(range(0, 10)), range(3, 10));

        let end = drag(DragMode::TrimEnd, range(0, 10), range(0, 6));
        assert_eq!(end.partner_preview(range(0, 10)), range(0, 6));
    }

    #[test]
    fn a_slipped_partner_stays_where_it_is() {
        let mut slip = drag(DragMode::Slip, range(2, 6), range(2, 6));
        slip.slip = 48_000;
        assert_eq!(slip.partner_preview(range(2, 6)), range(2, 6));
    }

    #[test]
    fn default_zoom_is_thirty_pixels_per_second() {
        let s = UiState::default();
        assert_eq!(s.ticks_per_pixel(), 32_000);
        assert!((s.pixels_per_second() - 30.0).abs() < 0.001);
    }

    #[test]
    fn zoom_is_clamped_at_both_ends() {
        let mut s = UiState::default();
        for _ in 0..50 {
            s.zoom_in();
        }
        assert_eq!(s.ticks_per_pixel(), ZOOM_LEVELS[0]);
        assert!(!s.can_zoom_in());

        for _ in 0..50 {
            s.zoom_out();
        }
        assert_eq!(s.ticks_per_pixel(), ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]);
        assert!(!s.can_zoom_out());
    }

    #[test]
    fn scrolling_never_goes_before_zero() {
        let mut s = UiState::default();
        s.scroll_by_pixels(-1000.0);
        assert_eq!(s.scroll_ticks, 0);
    }

    #[test]
    fn zoom_to_fit_picks_a_level_that_shows_the_whole_duration() {
        let mut s = UiState::default();
        let ten_minutes = TimelineTime::from_seconds(600);
        s.zoom_to_fit(ten_minutes, 1000.0);

        let visible = 1000_i64 * s.ticks_per_pixel();
        assert!(
            visible >= ten_minutes.ticks(),
            "zoom {} shows only {visible} of {} ticks",
            s.ticks_per_pixel(),
            ten_minutes.ticks()
        );
    }

    #[test]
    fn zoom_to_fit_ignores_an_empty_timeline() {
        let mut s = UiState::default();
        let before = s.ticks_per_pixel();
        s.zoom_to_fit(TimelineTime::ZERO, 1000.0);
        assert_eq!(s.ticks_per_pixel(), before);
    }

    #[test]
    fn revealing_scrolls_only_when_offscreen() {
        let mut s = UiState::default();
        // 1000 px at 32,000 ticks/px = 32,000,000 ticks visible.
        s.scroll_to_reveal(TimelineTime::from_ticks(1_000_000), 1000.0);
        assert_eq!(s.scroll_ticks, 0, "already visible");

        s.scroll_to_reveal(TimelineTime::from_ticks(100_000_000), 1000.0);
        assert!(s.scroll_ticks > 0);
    }

    /// While playing, the view pages forward rather than creeping a pixel at a
    /// time with the playhead stuck to the right edge.
    #[test]
    fn following_the_playhead_pages_the_view() {
        let mut s = UiState::default();
        let span = 1000 * s.ticks_per_pixel();
        let margin = span / 8;

        // Inside the view and not near the edge: nothing moves.
        s.follow_playhead(TimelineTime::from_ticks(span / 2), 1000.0);
        assert_eq!(s.scroll_ticks, 0);

        // Reaching the last eighth pages forward, leaving the playhead near
        // the left edge.
        let at = span - margin;
        s.follow_playhead(TimelineTime::from_ticks(at), 1000.0);
        assert_eq!(s.scroll_ticks, at - margin);

        // And it does not page again immediately.
        let scrolled = s.scroll_ticks;
        s.follow_playhead(TimelineTime::from_ticks(at + 10), 1000.0);
        assert_eq!(s.scroll_ticks, scrolled);

        // Jumping backwards — the playhead moved behind the view — brings it
        // back into sight.
        s.follow_playhead(TimelineTime::ZERO, 1000.0);
        assert_eq!(s.scroll_ticks, 0);
    }

    #[test]
    fn selection_toggles() {
        let mut s = UiState::default();
        let a = ClipId::new();
        s.select_only(a);
        assert!(s.selected_clips.contains(&a));
        s.toggle_selection(a);
        assert!(s.selected_clips.is_empty());
    }
}
