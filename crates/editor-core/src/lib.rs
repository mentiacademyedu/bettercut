//! Authoritative project state, commands, undo/redo, and events (§54–§56).
//!
//! The UI depends on this crate. This crate does not depend on the UI (§86).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod animation_copy;
pub mod attributes;
pub mod beat_photos;
pub mod boomerang;
pub mod bounce;
pub mod caption_replace;
pub mod censor;
pub mod clip_lengths;
pub mod collect;
pub mod colour_clips;
pub mod command;
pub mod compare;
pub mod compound;
pub mod count_in;
pub mod cover;
pub mod crop_shape;
pub mod editor;
pub mod error;
pub mod event;
pub mod every_cut;
pub mod extend_next;
pub mod fade_selection;
pub mod filters;
pub mod find;
pub mod fit_fill;
pub mod fit_music;
pub mod freeze_punch;
pub mod from_selection;
pub mod gaps;
pub mod hardware;
pub mod history;
pub mod image_sequence;
pub mod insert_drag;
pub mod journal;
pub mod keep_only;
pub mod keyframe_edit;
pub mod lane_move;
pub mod lane_tidy;
pub mod lanes;
pub mod lift;
pub mod loop_clip;
pub mod lower_third;
pub mod magnetic;
pub mod many_clips;
pub mod marker_file;
pub mod match_frame;
pub mod multicam;
pub mod name_titles;
pub mod ops;
pub mod pip;
pub mod pivot;
pub mod rate_stretch;
pub mod recovery;
pub mod render_in_place;
pub mod replace;
pub mod reshape;
pub mod rewind;
pub mod roll;
pub mod rotate;
pub mod save_template;
pub mod shuffle;
pub mod slide;
pub mod slideshow;
pub mod slip;
pub mod speed_curve;
pub mod speed_ramp;
pub mod split_edit;
pub mod split_even;
pub mod split_markers;
pub mod split_screen;
pub mod stabilise;
pub mod stems;
pub mod stickers;
pub mod storyboard;
pub mod swap;
pub mod sync_lock;
pub mod sync_sound;
pub mod template_apply;
pub mod three_point;
pub mod tone;
pub mod track_motion;
pub mod trim_black;
pub mod trim_window;
pub mod versions;
pub mod voiceover;
pub mod where_used;

pub use bettercut_timeline::{Movement, ZOOM_AMOUNT};
pub use command::{
    ClipPayload, ClipProperty, Command, CommandGroup, EditorCommand, ResolutionRepr, SettingChange,
    TextProperty, TrackFlag, TrackKindRepr, TrackPayload, TrimEdge,
};
pub use editor::Editor;
pub use error::EditorError;
pub use event::{Event, EventReceiver, EventSender, event_channel};
pub use hardware::HardwareProfile;
pub use history::{DEFAULT_HISTORY_LIMIT, History};
pub use journal::{Journal, RecoveryPaths};
pub use pip::{PipCorner, PipSize};
pub use recovery::{
    KEEP_UNSAVED_AT_MOST, KEEP_UNSAVED_FOR, RecoverableSession, prune_unsaved, recover,
    scan_unsaved, stale_sessions, unsaved_sessions,
};
pub use reshape::{SHAPES, Shape};
pub use speed_curve::SpeedCurve;
pub use speed_ramp::SpeedRamp;
pub use split_screen::SplitLayout;
pub use swap::Neighbour;
pub use template_apply::{AppliedTemplate, SlotFill};

// Re-exported so the UI crate needs one dependency for the whole core, and so
// there is one obvious place to look when the layering question comes up (§86).
pub use bettercut_captions as captions;
pub use bettercut_foundation as foundation;
pub use bettercut_media as media;
pub use bettercut_project_format as project_format;
pub use bettercut_templates as templates;
pub use bettercut_text as text;
pub use bettercut_timeline as timeline;
