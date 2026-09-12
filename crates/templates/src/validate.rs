//! Checking a template before anything uses it (§64, §65).
//!
//! §64: "Treat downloaded templates as untrusted." A template is data, never
//! code, and every field of it is checked here before the engine sees it.
//!
//! Two properties matter more than any individual rule:
//!
//! * **Nothing panics.** Any byte sequence at all produces either a template or
//!   a list of problems. A template picker that crashes on one bad file in a
//!   folder takes the rest of the folder down with it.
//! * **Every problem is reported, not just the first.** Whoever wrote the file
//!   fixes it in one pass instead of one error per attempt.
//!
//! Out-of-range numbers are *rejected*, not clamped. Clamping is right for a
//! slider, where the user is looking at the result; in a template it would
//! quietly render something other than what the author wrote, and they would
//! find out from a user.

use bettercut_foundation::{Rational, TimelineTime};
use bettercut_timeline::{
    AnimatedParameter, MAX_FADE, MAX_MOTION, MAX_SPEED, MIN_MOTION, MIN_SPEED, MIN_TRANSITION,
    Motion, MotionKind, Movement, TextAnimation, Transform, TransitionKind, Vec2,
};

use crate::format::{ElementFile, TemplateFile, TransformFile};
use crate::template::{Element, Slot, SlotKind, Template};

/// The format version this build reads. §30: "Versioning is essential."
pub const SCHEMA_VERSION: u32 = 1;

/// Resource bounds (§64: size-capped before use).
///
/// Generous for any real template, and small enough that a malicious one
/// cannot make applying it allocate without limit.
pub const MAX_DURATION_SECONDS: f64 = 600.0;
pub const MAX_SLOTS: usize = 32;
pub const MAX_ELEMENTS: usize = 256;
pub const MAX_TEXT_CHARS: usize = 2_000;
/// Tracks an element may name. Applying a template creates tracks up to this.
pub const MAX_TRACKS: usize = 8;
/// A template file larger than this is refused before it is parsed.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Bounds on position (normalized frame units, as `Transform` has them) and
/// rotation (degrees), which the editor itself leaves unbounded.
pub const MAX_POSITION: f32 = 10.0;
pub const MAX_ROTATION: f32 = 3600.0;

/// Something wrong with a template, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// A path into the file: `elements[3].duration`, `slots[0].id`.
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.at, self.message)
        }
    }
}

/// Read and check a template from its JSON text.
pub fn parse(json: &str) -> Result<Template, Vec<Problem>> {
    if json.len() > MAX_FILE_BYTES {
        return Err(vec![problem(
            "",
            format!(
                "the file is {} KB; templates are limited to {} KB",
                json.len() / 1024,
                MAX_FILE_BYTES / 1024
            ),
        )]);
    }

    // The version first, on its own, so a template from a newer build gets
    // "made for a newer version" rather than a confusing complaint about a
    // field this version has never heard of.
    if let Ok(probe) = serde_json::from_str::<serde_json::Value>(json)
        && let Some(version) = probe.get("schema_version").and_then(|v| v.as_u64())
        && version > u64::from(SCHEMA_VERSION)
    {
        return Err(vec![problem(
            "schema_version",
            format!(
                "made for template format {version}; this version of the editor reads \
                 format {SCHEMA_VERSION}"
            ),
        )]);
    }

    let file: TemplateFile = serde_json::from_str(json)
        .map_err(|err| vec![problem("", format!("not a valid template file: {err}"))])?;
    validate(file)
}

/// Check a parsed file and produce the validated template.
pub fn validate(file: TemplateFile) -> Result<Template, Vec<Problem>> {
    let mut problems = Vec::new();

    if file.schema_version != SCHEMA_VERSION {
        problems.push(problem(
            "schema_version",
            format!(
                "format {} is not one this version reads (it reads {SCHEMA_VERSION})",
                file.schema_version
            ),
        ));
    }

    if !is_identifier(&file.id) {
        problems.push(problem(
            "id",
            "must be 1–64 characters of a–z, 0–9, `_` and `-`",
        ));
    }
    if file.name.trim().is_empty() || file.name.chars().count() > 100 {
        problems.push(problem("name", "must be 1–100 characters"));
    }
    if file.category.trim().is_empty() {
        problems.push(problem("category", "is required"));
    }

    let duration = seconds(&mut problems, "duration", file.duration, false);

    // ---- slots ----------------------------------------------------------
    if file.slots.len() > MAX_SLOTS {
        problems.push(problem(
            "slots",
            format!("{} slots; the limit is {MAX_SLOTS}", file.slots.len()),
        ));
    }
    let mut slots: Vec<Slot> = Vec::new();
    for (index, slot) in file.slots.iter().enumerate() {
        let at = format!("slots[{index}]");
        if !is_identifier(&slot.id) {
            problems.push(problem(
                format!("{at}.id"),
                "must be 1–64 characters of a–z, 0–9, `_` and `-`",
            ));
        }
        if slots.iter().any(|s| s.id == slot.id) {
            problems.push(problem(
                format!("{at}.id"),
                format!("`{}` is used twice", slot.id),
            ));
        }
        let Some(kind) = SlotKind::parse(&slot.kind) else {
            problems.push(problem(
                format!("{at}.type"),
                format!(
                    "`{}` is not a slot type (video, image, text, audio, logo)",
                    slot.kind
                ),
            ));
            continue;
        };
        if let Some(text) = &slot.default_text
            && text.chars().count() > MAX_TEXT_CHARS
        {
            problems.push(problem(format!("{at}.default_text"), "is too long"));
        }
        if slot.default_text.is_some() && kind != SlotKind::Text {
            problems.push(problem(
                format!("{at}.default_text"),
                "only a text slot has default text",
            ));
        }
        slots.push(Slot {
            id: slot.id.clone(),
            kind,
            label: if slot.label.trim().is_empty() {
                slot.id.clone()
            } else {
                slot.label.clone()
            },
            default_text: slot.default_text.clone(),
        });
    }

    // ---- elements -------------------------------------------------------
    if file.elements.len() > MAX_ELEMENTS {
        problems.push(problem(
            "elements",
            format!(
                "{} elements; the limit is {MAX_ELEMENTS}",
                file.elements.len()
            ),
        ));
    }
    if file.elements.is_empty() {
        problems.push(problem("elements", "a template must place something"));
    }

    let mut elements = Vec::new();
    for (index, element) in file.elements.iter().enumerate() {
        let at = format!("elements[{index}]");
        if let Some(element) = element_of(&mut problems, &at, element, &slots, duration) {
            elements.push(element);
        }
    }

    // A slot no element uses is a box in the interface that does nothing when
    // filled — worse than no box.
    for slot in &slots {
        if !elements.iter().any(|e| e.slot() == Some(slot.id.as_str())) {
            problems.push(problem(
                format!("slots.{}", slot.id),
                "no element uses this slot",
            ));
        }
    }

    if problems.is_empty() {
        Ok(Template {
            id: file.id,
            name: file.name,
            category: file.category,
            description: file.description,
            duration,
            slots,
            elements,
        })
    } else {
        Err(problems)
    }
}

fn element_of(
    problems: &mut Vec<Problem>,
    at: &str,
    element: &ElementFile,
    slots: &[Slot],
    template_duration: TimelineTime,
) -> Option<Element> {
    let before = problems.len();

    // Every element occupies a span inside the template.
    let (start_s, duration_s) = match element {
        ElementFile::Clip {
            start, duration, ..
        }
        | ElementFile::Text {
            start, duration, ..
        }
        | ElementFile::Audio {
            start, duration, ..
        } => (*start, *duration),
    };
    let start = seconds(problems, &format!("{at}.start"), start_s, true);
    let duration = seconds(problems, &format!("{at}.duration"), duration_s, false);
    // Only once both ends are known good — a bad template duration is already
    // reported, and would otherwise make every element "run past" it too.
    if problems.len() == before
        && template_duration != TimelineTime::ZERO
        && start.ticks() + duration.ticks() > template_duration.ticks() + TICK_TOLERANCE
    {
        problems.push(problem(at, "runs past the end of the template"));
    }

    let slot_of = |problems: &mut Vec<Problem>, id: &str, allowed: &[SlotKind]| match slots
        .iter()
        .find(|s| s.id == id)
    {
        None => {
            problems.push(problem(
                format!("{at}.slot"),
                format!("no slot called `{id}`"),
            ));
        }
        Some(slot) if !allowed.contains(&slot.kind) => {
            problems.push(problem(
                format!("{at}.slot"),
                format!(
                    "`{id}` is a slot for {}, which this element cannot use",
                    slot.kind.label().to_lowercase()
                ),
            ));
        }
        Some(_) => {}
    };

    let element = match element {
        ElementFile::Clip {
            slot,
            track,
            transform,
            opacity,
            speed,
            transition_out,
            movement,
            ..
        } => {
            let movement = match movement.as_deref() {
                None => Movement::None,
                Some(name) => Movement::parse(name).unwrap_or_else(|| {
                    problems.push(problem(
                        format!("{at}.movement"),
                        format!("`{name}` is not a movement (none, zoom_in, zoom_out)"),
                    ));
                    Movement::None
                }),
            };
            slot_of(
                problems,
                slot,
                &[SlotKind::Video, SlotKind::Image, SlotKind::Logo],
            );
            track_in_range(problems, at, *track);
            let transform = transform_of(problems, at, transform);
            let opacity = opacity.unwrap_or(1.0);
            ranged(
                problems,
                &format!("{at}.opacity"),
                opacity,
                AnimatedParameter::Opacity,
            );

            let speed = match speed {
                None => Rational::ONE,
                Some(value) => speed_of(problems, &format!("{at}.speed"), *value),
            };

            let transition_out = transition_out.as_ref().and_then(|t| {
                let kind = match t.kind.as_str() {
                    "crossfade" => Some(TransitionKind::Crossfade),
                    "fade_through_black" => Some(TransitionKind::FadeThroughBlack),
                    other => {
                        problems.push(problem(
                            format!("{at}.transition_out.kind"),
                            format!(
                                "`{other}` is not a transition (crossfade, fade_through_black)"
                            ),
                        ));
                        None
                    }
                };
                let length = seconds(
                    problems,
                    &format!("{at}.transition_out.duration"),
                    t.duration,
                    false,
                );
                if length != TimelineTime::ZERO && length < MIN_TRANSITION {
                    problems.push(problem(
                        format!("{at}.transition_out.duration"),
                        "is shorter than the shortest transition the editor makes",
                    ));
                }
                kind.map(|kind| (kind, length))
            });

            Element::Clip {
                slot: slot.clone(),
                start,
                duration,
                track: *track,
                transform,
                opacity,
                speed,
                transition_out,
                movement,
            }
        }
        ElementFile::Text {
            slot,
            text,
            transform,
            style,
            animation,
            ..
        } => {
            let animation = animation_of(problems, at, animation.as_ref());
            match (slot, text) {
                (Some(id), _) => slot_of(problems, id, &[SlotKind::Text]),
                (None, None) => {
                    problems.push(problem(at, "a text element needs a `slot` or fixed `text`"))
                }
                (None, Some(_)) => {}
            }
            let words = text
                .clone()
                .or_else(|| {
                    slot.as_ref()
                        .and_then(|id| slots.iter().find(|s| &s.id == id))
                        .and_then(|s| s.default_text.clone())
                })
                .unwrap_or_default();
            if words.chars().count() > MAX_TEXT_CHARS {
                problems.push(problem(format!("{at}.text"), "is too long"));
            }
            let transform = transform_of(problems, at, transform);

            // The text crate's own limits, applied rather than restated: a
            // style that `sanitized` would change has a number outside what
            // the editor can draw, and is reported rather than quietly
            // clamped. `NaN` fails the comparison too, which is the point.
            let style = style_of(problems, at, style.as_ref());
            let sanitized = style.sanitized();
            if sanitized != style {
                problems.push(problem(
                    format!("{at}.style"),
                    format!(
                        "has a value outside what can be drawn (size {}–{}, line height \
                         0.5–4, letter spacing −0.5–2, outline, shadow and background \
                         within the size)",
                        bettercut_text::MIN_SIZE,
                        bettercut_text::MAX_SIZE
                    ),
                ));
            }

            Element::Text {
                slot: slot.clone(),
                text: words,
                start,
                duration,
                transform,
                style: sanitized,
                animation,
            }
        }
        ElementFile::Audio {
            slot,
            track,
            volume,
            fade_in,
            fade_out,
            ..
        } => {
            let mut fade = |name: &str, value: Option<f64>| {
                let Some(value) = value else {
                    return TimelineTime::ZERO;
                };
                let at = format!("{at}.{name}");
                let length = seconds(problems, &at, value, true);
                if length > MAX_FADE {
                    problems.push(problem(at, "is longer than the 30 second limit"));
                }
                length
            };
            let fade_in = fade("fade_in", *fade_in);
            let fade_out = fade("fade_out", *fade_out);
            slot_of(problems, slot, &[SlotKind::Audio]);
            track_in_range(problems, at, *track);
            let volume = volume.unwrap_or(1.0);
            if !volume.is_finite() || !(0.0..=4.0).contains(&volume) {
                problems.push(problem(format!("{at}.volume"), "must be between 0 and 4"));
            }
            Element::Audio {
                slot: slot.clone(),
                start,
                duration,
                track: *track,
                volume,
                fade_in,
                fade_out,
            }
        }
    };

    (problems.len() == before).then_some(element)
}

/// A title's entrance and exit, each a known motion of an allowed length.
fn animation_of(
    problems: &mut Vec<Problem>,
    at: &str,
    raw: Option<&crate::format::AnimationFile>,
) -> TextAnimation {
    let Some(raw) = raw else {
        return TextAnimation::default();
    };
    let mut motion = |end: &str, file: Option<&crate::format::MotionFile>| {
        let file = file?;
        let at = format!("{at}.animation.{end}");
        let kind = match file.kind.as_str() {
            "fade" => Some(MotionKind::Fade),
            "slide_up" => Some(MotionKind::SlideUp),
            "slide_down" => Some(MotionKind::SlideDown),
            "pop" => Some(MotionKind::Pop),
            "typewriter" => Some(MotionKind::Typewriter),
            other => {
                problems.push(problem(
                    format!("{at}.kind"),
                    format!(
                        "`{other}` is not a motion (fade, slide_up, slide_down, pop, typewriter)"
                    ),
                ));
                None
            }
        };
        let length = seconds(problems, &format!("{at}.duration"), file.duration, false);
        if length != TimelineTime::ZERO && !(MIN_MOTION..=MAX_MOTION).contains(&length) {
            problems.push(problem(
                format!("{at}.duration"),
                format!(
                    "must be between {} and {} seconds",
                    MIN_MOTION.ticks() as f64 / 960_000.0,
                    MAX_MOTION.ticks() as f64 / 960_000.0
                ),
            ));
        }
        kind.map(|kind| Motion::new(kind, length))
    };
    TextAnimation {
        intro: motion("in", raw.intro.as_ref()),
        outro: motion("out", raw.outro.as_ref()),
    }
}

/// A text style from only the settings the author changed.
///
/// Laid over a new title's style, so `{ "size": 96 }` is a whole style — an
/// author should not have to restate the colour and line height to make a
/// title bigger. A setting the style does not have is refused, like any other
/// unknown field: a misspelt `"colour"` silently ignored would leave the author
/// wondering why their text stayed white.
fn style_of(
    problems: &mut Vec<Problem>,
    at: &str,
    raw: Option<&serde_json::Value>,
) -> bettercut_text::TextStyle {
    let base = bettercut_text::TextStyle::default();
    let Some(raw) = raw else {
        return base;
    };
    let Some(fields) = raw.as_object() else {
        problems.push(problem(format!("{at}.style"), "must be an object"));
        return base;
    };
    let Ok(serde_json::Value::Object(mut merged)) = serde_json::to_value(&base) else {
        return base;
    };
    for (key, value) in fields {
        if merged.contains_key(key) {
            merged.insert(key.clone(), value.clone());
        } else {
            problems.push(problem(
                format!("{at}.style.{key}"),
                "is not a text style setting",
            ));
        }
    }
    serde_json::from_value(serde_json::Value::Object(merged)).unwrap_or_else(|err| {
        problems.push(problem(format!("{at}.style"), err.to_string()));
        base
    })
}

/// A tick either side of an authored second, for the "runs past the end" check.
/// Seconds as `f64` convert to ticks with a rounding, and an element authored to
/// end exactly on the template's end must not fail by one tick.
const TICK_TOLERANCE: i64 = 1;

/// Seconds, as authored, to ticks (§30) — refusing what cannot be a time.
fn seconds(problems: &mut Vec<Problem>, at: &str, value: f64, zero_allowed: bool) -> TimelineTime {
    if !value.is_finite() {
        problems.push(problem(at, "is not a number"));
        return TimelineTime::ZERO;
    }
    if value < 0.0 || (!zero_allowed && value == 0.0) {
        problems.push(problem(
            at,
            if zero_allowed {
                "cannot be negative"
            } else {
                "must be more than zero"
            },
        ));
        return TimelineTime::ZERO;
    }
    if value > MAX_DURATION_SECONDS {
        problems.push(problem(
            at,
            format!("{value} s is longer than the {MAX_DURATION_SECONDS} s limit"),
        ));
        return TimelineTime::ZERO;
    }
    // The one float-to-ticks conversion, here and only here (§9, §30).
    TimelineTime::from_ticks((value * 960_000.0).round() as i64)
}

fn transform_of(problems: &mut Vec<Problem>, at: &str, raw: &TransformFile) -> Transform {
    let mut transform = Transform::default();
    if let Some([x, y]) = raw.position {
        ranged(
            problems,
            &format!("{at}.transform.position[0]"),
            x,
            AnimatedParameter::PositionX,
        );
        ranged(
            problems,
            &format!("{at}.transform.position[1]"),
            y,
            AnimatedParameter::PositionY,
        );
        transform.position = Vec2::new(x, y);
    }
    if let Some(scale) = raw.scale {
        ranged(
            problems,
            &format!("{at}.transform.scale"),
            scale,
            AnimatedParameter::ScaleX,
        );
        transform.scale = Vec2::new(scale, scale);
    }
    if let Some(degrees) = raw.rotation {
        ranged(
            problems,
            &format!("{at}.transform.rotation"),
            degrees,
            AnimatedParameter::Rotation,
        );
        transform.rotation_degrees = degrees;
    }
    transform
}

/// Inside the range the editor's own control for this parameter allows.
///
/// From `AnimatedParameter::limits` rather than restated, so a template can
/// never set a value the Inspector could not show.
fn ranged(problems: &mut Vec<Problem>, at: &str, value: f32, parameter: AnimatedParameter) {
    if !value.is_finite() {
        problems.push(problem(at, "is not a number"));
        return;
    }
    // The editor leaves position and rotation unbounded; a template gets a
    // sanity bound anyway. Nothing useful sits ten frame-widths off-screen,
    // and a value that far out loses the precision `f32` has near the canvas.
    let (low, high) = parameter.limits().unwrap_or(match parameter {
        AnimatedParameter::Rotation => (-MAX_ROTATION, MAX_ROTATION),
        _ => (-MAX_POSITION, MAX_POSITION),
    });
    if value < low || value > high {
        problems.push(problem(at, format!("{value} is outside {low}–{high}")));
    }
}

fn speed_of(problems: &mut Vec<Problem>, at: &str, value: f64) -> Rational {
    // Hundredths, as the Speed tab's slider has them, converted to an exact
    // ratio so nothing downstream multiplies positions by a float (§74).
    //
    // Range-checked in hundredths *before* anything is multiplied: an
    // authored 1e308 saturates to `i64::MAX` on the cast, and comparing that
    // as a ratio would overflow.
    let hundredths = if value.is_finite() {
        (value * 100.0).round()
    } else {
        f64::NAN
    };
    let (low, high) = (MIN_SPEED.as_f64() * 100.0, MAX_SPEED.as_f64() * 100.0);
    match Rational::new(hundredths as i64, 100) {
        Some(ratio) if hundredths >= low.round() && hundredths <= high.round() => ratio,
        _ => {
            problems.push(problem(
                at,
                format!(
                    "{value} is outside {}–{}",
                    MIN_SPEED.as_f64(),
                    MAX_SPEED.as_f64()
                ),
            ));
            Rational::ONE
        }
    }
}

fn track_in_range(problems: &mut Vec<Problem>, at: &str, track: usize) {
    if track >= MAX_TRACKS {
        problems.push(problem(
            format!("{at}.track"),
            format!("track {track} is past the limit of {MAX_TRACKS}"),
        ));
    }
}

/// Lowercase letters, digits, `_` and `-`, 1–64 of them.
///
/// Strict because an id becomes part of a path when a template is installed
/// (§64: nothing may reach outside the bundle), and a strict alphabet is the
/// simplest way to be sure no id can be a path at all.
fn is_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn problem(at: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        at: at.into(),
        message: message.into(),
    }
}

/// §64: whether a path found in a template stays inside its bundle.
///
/// Relative, forward-slash, no `..`, no drive letter, no UNC, no leading slash.
/// Written out rather than delegated to `Path` because `Path`'s idea of
/// "absolute" depends on which operating system is reading it — `C:\x` is
/// relative on Linux — and a template made on one machine is opened on another.
pub fn is_bundle_relative(path: &str) -> bool {
    if path.is_empty() || path.len() > 255 {
        return false;
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return false;
    }
    // `C:`, `D:` — a drive-qualified path, absolute or not.
    if path.as_bytes().get(1) == Some(&b':') {
        return false;
    }
    if path.contains('\0') {
        return false;
    }
    path.split('/')
        .all(|part| !part.is_empty() && part != ".." && part != ".")
}
