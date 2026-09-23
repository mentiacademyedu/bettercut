//! Shaping and rasterization — the one implementation §26.1 requires.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};

use crate::error::TextError;
use crate::mask::Mask;
use crate::style::{Alignment, FontFamily, Rgba, TextStyle};

/// A rasterized piece of text: sRGB-encoded RGBA with straight alpha, tightly
/// packed, ready to upload as a texture (§21a).
/// The smallest a shrunk title goes, in pixels: below this it is not a title
/// but a smudge, and running off the edge at least says what it said.
pub const MIN_FIT_SIZE: f32 = 8.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBitmap {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, no padding.
    pub pixels: Vec<u8>,
}

impl TextBitmap {
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        let i = ((y * self.width + x) * 4) as usize;
        Rgba::new(
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        )
    }
}

/// The largest bitmap we will produce, in pixels on a side. Comfortably past
/// 4K, and small enough that the allocation cannot be a denial of service from
/// a hand-edited project file.
const MAX_DIMENSION: u32 = 8192;

/// Shapes and rasterizes text (§26.1).
///
/// Holds the font database and the glyph cache, both of which are expensive to
/// build and cheap to reuse, so this should be kept alive rather than made per
/// frame.
pub struct TextRenderer {
    fonts: FontSystem,
    glyphs: SwashCache,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Build a renderer over the fonts installed on this machine.
    ///
    /// §26 says to bundle fonts in `assets/fonts/` and not to depend on system
    /// ones, because a template must look the same everywhere. That matters
    /// once there are templates (Milestone 11) and once there is a font picker;
    /// until then a bundled family would be the *only* family on offer, which
    /// is a worse answer than the machine's own fonts. [`Self::with_fonts`] is
    /// the seam that closes this when the assets exist.
    pub fn new() -> Self {
        let mut fonts = FontSystem::new();
        // Fonts the user imported, so every renderer — the preview's and every
        // export's — can draw them.
        let imported = user_fonts_dir();
        if imported.is_dir() {
            fonts.db_mut().load_fonts_dir(&imported);
        }
        Self {
            fonts,
            glyphs: SwashCache::new(),
        }
    }

    /// Make the font file at `path` available to this renderer, returning the
    /// families it added.
    pub fn add_font_file(&mut self, path: &std::path::Path) -> Result<Vec<String>, TextError> {
        let bytes = std::fs::read(path).map_err(|e| TextError::Font(e.to_string()))?;
        let families = font_families_in(&bytes)?;
        self.fonts.db_mut().load_font_data(bytes);
        Ok(families)
    }

    /// Build a renderer over exactly these font files and nothing else.
    ///
    /// Used by tests that need a known font, and the route §26's bundled fonts
    /// will take.
    pub fn with_fonts(files: impl IntoIterator<Item = Vec<u8>>) -> Self {
        let sources = files
            .into_iter()
            .map(|bytes| cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(bytes)));
        Self {
            fonts: FontSystem::new_with_fonts(sources),
            glyphs: SwashCache::new(),
        }
    }

    /// How many faces are available. Diagnostics, and how a test tells "no
    /// fonts on this machine" from "shaping produced nothing".
    pub fn font_count(&self) -> usize {
        self.fonts.db().len()
    }

    /// The font families installed on this machine, sorted, without duplicates.
    ///
    /// A *family* rather than a face: a face is "Arial Bold Italic", and weight
    /// and slant are already separate controls (§26). Listing faces would offer
    /// the same font four times and let the user pick a combination the weight
    /// buttons then contradict.
    ///
    /// Families whose name begins with `@` are skipped. Windows ships vertical
    /// writing-mode variants of every CJK font under that prefix — `@MS Gothic`
    /// beside `MS Gothic` — and they are the same font rotated, which is not a
    /// choice anyone means to make from a list.
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .db()
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .filter(|name| !name.starts_with('@') && !name.trim().is_empty())
            .collect();

        // Case-insensitively, so "Arial" and "arial" — which some systems
        // report separately — collapse, and the list reads alphabetically
        // rather than with every lowercase name exiled to the end.
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup_by_key(|name| name.to_lowercase());
        names
    }

    /// Shape `text` and draw it (§26.1).
    ///
    /// The bitmap is exactly as large as the result needs, decorations
    /// included, so the caller positions it by its own size rather than having
    /// to know how far a shadow reached.
    pub fn rasterize(&mut self, text: &str, style: &TextStyle) -> Result<TextBitmap, TextError> {
        self.rasterize_revealed(text, style, None)
    }

    /// [`Self::rasterize`], drawing only the first `visible` characters.
    ///
    /// The typewriter entrance (`timeline::motion`). The layout is the whole
    /// text's — same size of bitmap, same line breaks, every letter where it
    /// will finally be — and the characters not yet reached are simply not
    /// drawn. Laying out only the typed part instead would re-centre the line
    /// with every letter, and the words would crawl sideways as they appeared.
    pub fn rasterize_revealed(
        &mut self,
        text: &str,
        style: &TextStyle,
        visible: Option<usize>,
    ) -> Result<TextBitmap, TextError> {
        self.rasterize_marked(text, style, visible, None)
    }

    /// [`Self::rasterize_revealed`], with the characters in `mark` filled in
    /// the mark's colour instead of the style's — the word being said, in a
    /// caption that highlights each word as it comes.
    ///
    /// Only the fill changes. The layout, the outline and the shadow are the
    /// whole text's, so the highlight moving from word to word never nudges a
    /// letter.
    /// How wide `text` is at `style`'s size with no wrapping: its longest
    /// line, in pixels.
    fn natural_width(&mut self, text: &str, style: &TextStyle) -> Result<f32, TextError> {
        let metrics = Metrics::new(style.size, style.size * style.line_height);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let attrs = attributes(style);
        buffer.set_size(None, None);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let (width, _) = measure(&buffer);
        if width < 1.0 {
            return Err(TextError::Empty);
        }
        Ok(width)
    }

    pub fn rasterize_marked(
        &mut self,
        text: &str,
        style: &TextStyle,
        visible: Option<usize>,
        mark: Option<Mark>,
    ) -> Result<TextBitmap, TextError> {
        let style = style.sanitized();
        if text.trim().is_empty() {
            return Err(TextError::Empty);
        }
        // Shrink to fit: the longest line measured at the size asked for, and
        // the size brought down by exactly the ratio it overshoots, so the
        // line lands on the width. Measured again after each step: glyphs
        // are hinted to whole pixels, so one ratio lands a few pixels over,
        // and a second or third settles it.
        let style = if style.shrink_to_fit
            && let Some(limit) = style.wrap_width
        {
            let mut fitted = style.clone();
            fitted.wrap_width = None;
            for _ in 0..3 {
                let natural = self.natural_width(text, &fitted)?;
                if natural <= limit || limit <= 0.0 || fitted.size <= MIN_FIT_SIZE {
                    break;
                }
                fitted.size = (fitted.size * limit / natural).max(MIN_FIT_SIZE);
            }
            if fitted.size < style.size {
                fitted
            } else {
                style
            }
        } else {
            style
        };

        let metrics = Metrics::new(style.size, style.size * style.line_height);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let attrs = attributes(&style);
        let align = match style.align {
            Alignment::Left => cosmic_text::Align::Left,
            Alignment::Center => cosmic_text::Align::Center,
            Alignment::Right => cosmic_text::Align::Right,
        };

        // Twice, and the two passes are not the same. The first has no width
        // limit beyond the wrap setting, so it reports how wide the text
        // actually wants to be; the second sets that as the width so alignment
        // has something to align *within*. Without it, centring several lines
        // would centre them in a box of unknown size.
        buffer.set_size(style.wrap_width, None);
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);

        let (text_width, text_height) = measure(&buffer);
        if text_width < 1.0 || text_height < 1.0 {
            return Err(TextError::Empty);
        }

        buffer.set_size(Some(text_width), None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        // Room for whatever reaches outside the glyphs. The background is not
        // in here: it is measured from the text box, so its padding is added
        // separately below.
        let stroke_reach = style.stroke.map_or(0.0, |s| s.width);
        let shadow_reach = style.shadow.map_or(0.0, |s| {
            s.offset_x.abs().max(s.offset_y.abs()) + s.blur * 1.5
        });
        let background_reach = style.background.map_or(0.0, |b| b.padding);
        let margin = stroke_reach.max(shadow_reach).max(background_reach).ceil() + 2.0;

        let width = (text_width + margin * 2.0).ceil() as u32;
        let height = (text_height + margin * 2.0).ceil() as u32;
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(TextError::TooLarge { width, height });
        }

        let cutoff = visible.map(|chars| Cutoff::after(text, chars));
        let marked_span = mark
            .as_ref()
            .filter(|m| m.chars.start < m.chars.end)
            .map(|m| {
                (
                    Cutoff::after(text, m.chars.start),
                    Cutoff::after(text, m.chars.end),
                )
            });
        let (glyphs, marked, colour_glyphs) = self.glyph_mask(
            &mut buffer,
            width as usize,
            height as usize,
            margin,
            cutoff,
            marked_span,
        );
        if glyphs.is_empty() {
            // Shaping succeeded but nothing was drawn: every character is
            // missing from every font available. Saying "empty" is honest —
            // there is no picture to composite.
            return Err(TextError::Empty);
        }

        let marked = mark.zip(marked).map(|(m, mask)| (mask, m.color));
        let flat = compose(
            &glyphs,
            marked.as_ref(),
            colour_glyphs.as_deref(),
            &style,
            width,
            height,
            margin,
            text_width,
            text_height,
        );
        Ok(arc(flat, style.curve))
    }

    /// Draw the shaped glyphs into a coverage mask, and keep the pixels of any
    /// glyph that brought its own colours.
    ///
    /// White and fully opaque, so the alpha the rasterizer reports *is* the
    /// coverage. Colour is applied afterwards, once, to whichever of the
    /// stroke, shadow and fill wants it.
    ///
    /// The exception is an emoji: those are drawn from a colour bitmap or a
    /// layered outline in the font, and a yellow face filled in the title's
    /// colour is a shape nobody recognises. Asking for white and watching what
    /// comes back is how they are told apart — a glyph whose pixels are not
    /// white painted them itself, and they are kept to be drawn over the fill.
    fn glyph_mask(
        &mut self,
        buffer: &mut Buffer,
        width: usize,
        height: usize,
        margin: f32,
        cutoff: Option<Cutoff>,
        marked_span: Option<(Cutoff, Cutoff)>,
    ) -> (Mask, Option<Mask>, Option<Vec<u8>>) {
        let mut mask = Mask::new(width, height);
        let mut marked = marked_span.map(|_| Mask::new(width, height));
        // Only made once something colourful is actually drawn: ordinary text
        // should not pay for a second full-size buffer.
        let mut colours: Option<Vec<u8>> = None;
        let offset = margin.round() as i32;
        let white = cosmic_text::Color::rgba(255, 255, 255, 255);

        // What `Buffer::draw` does, written out so each glyph can be asked
        // whether it has been reached yet. Same positions, same colour, same
        // cache: with no cutoff the result is identical.
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                if cutoff.is_some_and(|c| !c.shows(run.line_i, glyph.start)) {
                    continue;
                }
                let physical = glyph.physical((0.0, run.line_y), 1.0);
                let color = glyph.color_opt.unwrap_or(white);
                // In the mark: reached its start, not yet its end.
                let in_mark = marked_span.is_some_and(|(from, to)| {
                    !from.shows(run.line_i, glyph.start) && to.shows(run.line_i, glyph.start)
                });
                self.glyphs.with_pixels(
                    &mut self.fonts,
                    physical.cache_key,
                    color,
                    |x, y, pixel| {
                        let alpha = pixel.a();
                        let px = physical.x + x + offset;
                        let py = physical.y + y + offset;
                        if alpha == 0
                            || px < 0
                            || py < 0
                            || px >= width as i32
                            || py >= height as i32
                        {
                            return;
                        }
                        mask.cover(px as usize, py as usize, alpha);
                        if pixel.r() != 255 || pixel.g() != 255 || pixel.b() != 255 {
                            let colours =
                                colours.get_or_insert_with(|| vec![0_u8; width * height * 4]);
                            let at = (py as usize * width + px as usize) * 4;
                            colours[at] = pixel.r();
                            colours[at + 1] = pixel.g();
                            colours[at + 2] = pixel.b();
                            colours[at + 3] = alpha;
                        }
                        if in_mark && let Some(marked) = marked.as_mut() {
                            marked.cover(px as usize, py as usize, alpha);
                        }
                    },
                );
            }
        }

        (mask, marked, colours)
    }
}

/// Where imported fonts are kept for this user: every renderer loads them.
pub fn user_fonts_dir() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("bettercut")
        .join("fonts")
}

/// The family names of the faces in a font file's bytes, or why it is not a
/// font this can draw with.
pub fn font_families_in(bytes: &[u8]) -> Result<Vec<String>, TextError> {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(bytes.to_vec());
    let mut names: Vec<String> = db
        .faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    names.sort();
    names.dedup();
    if names.is_empty() {
        return Err(TextError::Font(
            "that file is not a font this editor can use (.ttf, .otf or .ttc)".to_owned(),
        ));
    }
    Ok(names)
}

/// Copy the font at `path` into `folder` so it is loaded from now on, and
/// return where it went and the families it holds. A file of the same name
/// already there is replaced.
pub fn import_font_into(
    path: &std::path::Path,
    folder: &std::path::Path,
) -> Result<(std::path::PathBuf, Vec<String>), TextError> {
    let bytes = std::fs::read(path).map_err(|e| TextError::Font(e.to_string()))?;
    let families = font_families_in(&bytes)?;
    let name = path
        .file_name()
        .ok_or_else(|| TextError::Font("the font file has no name".to_owned()))?;
    std::fs::create_dir_all(folder).map_err(|e| TextError::Font(e.to_string()))?;
    let destination = folder.join(name);
    std::fs::write(&destination, bytes).map_err(|e| TextError::Font(e.to_string()))?;
    Ok((destination, families))
}

/// Characters drawn in their own colour: a range of character indices into the
/// text, newlines counted, end exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub chars: std::ops::Range<usize>,
    pub color: Rgba,
}

/// Where a partial reveal stops, in the terms the layout reports glyphs in:
/// which line of the text (split at newlines), and a byte offset into it.
#[derive(Debug, Clone, Copy)]
struct Cutoff {
    line: usize,
    byte: usize,
}

impl Cutoff {
    /// Just after the first `chars` characters of `text`.
    fn after(text: &str, chars: usize) -> Self {
        let mut remaining = chars;
        for (line, content) in text.split('\n').enumerate() {
            let count = content.chars().count();
            if remaining <= count {
                let byte = content
                    .char_indices()
                    .nth(remaining)
                    .map_or(content.len(), |(i, _)| i);
                return Self { line, byte };
            }
            // The newline itself counts as a character typed.
            remaining -= count + 1;
        }
        Self {
            line: usize::MAX,
            byte: 0,
        }
    }

    fn shows(self, line: usize, byte: usize) -> bool {
        line < self.line || (line == self.line && byte < self.byte)
    }
}

fn attributes(style: &TextStyle) -> Attrs<'_> {
    let family = match &style.family {
        FontFamily::SansSerif => Family::SansSerif,
        FontFamily::Serif => Family::Serif,
        FontFamily::Monospace => Family::Monospace,
        FontFamily::Named(name) => Family::Name(name),
    };
    let mut attrs = Attrs::new()
        .family(family)
        .weight(Weight(style.weight.value()));
    if style.italic {
        attrs = attrs.style(cosmic_text::Style::Italic);
    }
    if style.letter_spacing != 0.0 {
        attrs = attrs.letter_spacing(style.letter_spacing);
    }
    attrs
}

/// How wide and tall the shaped text is.
///
/// The width is the widest line rather than the box it was laid out in: a
/// centred single word in a wide buffer would otherwise produce a bitmap that
/// is mostly empty, and the caller would have to know to ignore it.
fn measure(buffer: &Buffer) -> (f32, f32) {
    let mut width: f32 = 0.0;
    let mut height: f32 = 0.0;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height = height.max(run.line_top + run.line_height);
    }
    (width.ceil(), height.ceil())
}

/// Put the layers together: background, then shadow, then stroke, then fill.
///
/// That order is the one that reads correctly. The shadow belongs behind the
/// text but in front of its background box, or a caption's shadow would vanish
/// the moment the box was turned on; the stroke belongs behind the fill, or a
/// thick outline would eat into the letterforms it is supposed to surround.
#[allow(clippy::too_many_arguments)]
fn compose(
    glyphs: &Mask,
    marked: Option<&(Mask, Rgba)>,
    colour_glyphs: Option<&[u8]>,
    style: &TextStyle,
    width: u32,
    height: u32,
    margin: f32,
    text_width: f32,
    text_height: f32,
) -> TextBitmap {
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];

    if let Some(background) = style.background {
        let left = margin - background.padding;
        let top = margin - background.padding;
        let right = margin + text_width + background.padding;
        let bottom = margin + text_height + background.padding;
        fill_rounded_rect(
            &mut pixels,
            width,
            height,
            left,
            top,
            right,
            bottom,
            background.corner_radius,
            background.color,
        );
    }

    if let Some(shadow) = style.shadow {
        let cast = glyphs
            .offset(
                shadow.offset_x.round() as i32,
                shadow.offset_y.round() as i32,
            )
            .blurred(shadow.blur);
        blend_mask(&mut pixels, &cast, shadow.color);
    }

    if let Some(stroke) = style.stroke
        && stroke.width > 0.0
    {
        blend_mask(&mut pixels, &glyphs.dilated(stroke.width), stroke.color);
    }

    // Where the fill's colour comes from: flat, or shaded down the text from
    // the style's colour to the gradient's, measured over the letters' own
    // box so the whole range shows however much margin the decorations need.
    let fill_at = |y: usize| -> Rgba {
        match style.gradient {
            None => style.color,
            Some(bottom) => {
                let t = ((y as f32 + 0.5 - margin) / text_height.max(1.0)).clamp(0.0, 1.0);
                let mix =
                    |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
                Rgba::new(
                    mix(style.color.r, bottom.r),
                    mix(style.color.g, bottom.g),
                    mix(style.color.b, bottom.b),
                    mix(style.color.a, bottom.a),
                )
            }
        }
    };

    match marked {
        None if style.gradient.is_none() => blend_mask(&mut pixels, glyphs, style.color),
        None => {
            for y in 0..glyphs.height {
                let fill = fill_at(y);
                for x in 0..glyphs.width {
                    let coverage = glyphs.at(x, y);
                    if coverage == 0 {
                        continue;
                    }
                    let source_alpha = (coverage as f32 / 255.0) * (fill.a as f32 / 255.0);
                    blend_pixel(&mut pixels, (y * glyphs.width + x) * 4, fill, source_alpha);
                }
            }
        }
        // One fill, each pixel in the colour of the letter it belongs to, so
        // the lit word is exactly as heavy as it was unlit. Drawing the mark
        // over the fill instead would blend the soft edges twice.
        Some((mask, colour)) => {
            for y in 0..glyphs.height {
                for x in 0..glyphs.width {
                    let coverage = glyphs.at(x, y);
                    if coverage == 0 {
                        continue;
                    }
                    let fill = if mask.at(x, y) > 0 {
                        *colour
                    } else {
                        fill_at(y)
                    };
                    let source_alpha = (coverage as f32 / 255.0) * (fill.a as f32 / 255.0);
                    blend_pixel(&mut pixels, (y * glyphs.width + x) * 4, fill, source_alpha);
                }
            }
        }
    }

    // Emoji last, in their own colours, over the fill that stood in for them.
    // The stroke and shadow beneath still followed their shape, which is what
    // makes an emoji with an outline look like the rest of the line.
    if let Some(colours) = colour_glyphs {
        for y in 0..height as usize {
            for x in 0..width as usize {
                let at = (y * width as usize + x) * 4;
                let alpha = colours[at + 3];
                if alpha == 0 {
                    continue;
                }
                let colour = Rgba::new(colours[at], colours[at + 1], colours[at + 2], 255);
                let source_alpha = (f32::from(alpha) / 255.0) * (f32::from(style.color.a) / 255.0);
                blend_pixel(&mut pixels, at, colour, source_alpha);
            }
        }
    }

    TextBitmap {
        width,
        height,
        pixels,
    }
}

/// The least bend worth drawing: below this the text is left straight.
const LEAST_CURVE: f32 = 0.01;

/// `bitmap` bent round a circle by `curve` (-1–1): the middle of the text runs
/// along the arc at its full length, so letters keep their size along the
/// line, and the top of the text is on the outside of an arch. Positive arches
/// up, negative sags; ±1 is half a circle.
///
/// A warp of the finished picture rather than placing each glyph, so the
/// outline, shadow and box all bend with the letters exactly as they were
/// drawn.
pub fn arc(bitmap: TextBitmap, curve: f32) -> TextBitmap {
    if !curve.is_finite() || curve.abs() < LEAST_CURVE {
        return bitmap;
    }
    let (w, h) = (bitmap.width as f32, bitmap.height as f32);
    let sweep = curve.abs().min(1.0) * std::f32::consts::PI;
    let radius = w / sweep;
    let up = curve > 0.0;

    // Where a source point lands, with the circle's centre at the origin and
    // y down: the text's middle row on `radius`, its top further out.
    let place = |x: f32, y: f32| -> (f32, f32) {
        let angle = (x - w / 2.0) / radius;
        let r = if up {
            radius + (h / 2.0 - y)
        } else {
            radius + (y - h / 2.0)
        };
        let across = r * angle.sin();
        let down = r * angle.cos();
        (across, if up { -down } else { down })
    };

    // The bent picture's bounds, from points round the source's edge.
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let steps = 64;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        for (x, y) in [(t * w, 0.0), (t * w, h), (0.0, t * h), (w, t * h)] {
            let (px, py) = place(x, y);
            min_x = min_x.min(px);
            max_x = max_x.max(px);
            min_y = min_y.min(py);
            max_y = max_y.max(py);
        }
    }
    let out_w = ((max_x - min_x).ceil() as u32 + 2).min(MAX_DIMENSION);
    let out_h = ((max_y - min_y).ceil() as u32 + 2).min(MAX_DIMENSION);
    let mut pixels = vec![0u8; out_w as usize * out_h as usize * 4];

    // Every output pixel, back to where it came from, read with a bilinear
    // blend so the bent edges stay smooth.
    for oy in 0..out_h {
        for ox in 0..out_w {
            let px = ox as f32 + 0.5 + min_x - 1.0;
            let py = oy as f32 + 0.5 + min_y - 1.0;
            let away = if up { -py } else { py };
            let r = (px * px + away * away).sqrt();
            let angle = px.atan2(away);
            if angle.abs() > sweep / 2.0 + 0.5 / radius {
                continue;
            }
            let sx = angle * radius + w / 2.0 - 0.5;
            let sy = if up {
                h / 2.0 - (r - radius)
            } else {
                h / 2.0 + (r - radius)
            } - 0.5;
            if sx < -1.0 || sy < -1.0 || sx > w || sy > h {
                continue;
            }
            let sample = bilinear(&bitmap, sx, sy);
            let at = (oy as usize * out_w as usize + ox as usize) * 4;
            pixels[at..at + 4].copy_from_slice(&sample);
        }
    }
    TextBitmap {
        width: out_w,
        height: out_h,
        pixels,
    }
}

/// The bitmap at a fractional pixel, premultiplied for the blend so a soft
/// edge does not pick up the colour of the transparent pixels beside it.
fn bilinear(bitmap: &TextBitmap, x: f32, y: f32) -> [u8; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let read = |ix: f32, iy: f32| -> [f32; 4] {
        if ix < 0.0 || iy < 0.0 || ix >= bitmap.width as f32 || iy >= bitmap.height as f32 {
            return [0.0; 4];
        }
        let at = (iy as usize * bitmap.width as usize + ix as usize) * 4;
        let p = &bitmap.pixels[at..at + 4];
        let a = f32::from(p[3]) / 255.0;
        [
            f32::from(p[0]) * a,
            f32::from(p[1]) * a,
            f32::from(p[2]) * a,
            a,
        ]
    };
    let (a, b, c, d) = (
        read(x0, y0),
        read(x0 + 1.0, y0),
        read(x0, y0 + 1.0),
        read(x0 + 1.0, y0 + 1.0),
    );
    let mut out = [0.0_f32; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = top + (bottom - top) * fy;
    }
    let alpha = out[3];
    if alpha <= 0.0 {
        return [0; 4];
    }
    [
        (out[0] / alpha).round().clamp(0.0, 255.0) as u8,
        (out[1] / alpha).round().clamp(0.0, 255.0) as u8,
        (out[2] / alpha).round().clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Source-over, straight alpha.
///
/// In sRGB rather than linear light, deliberately. Glyph coverage is what a
/// font's hinting and antialiasing were tuned against, and blending it in
/// linear light makes text visibly thinner than the designer intended. The
/// finished bitmap is an ordinary sRGB image, and the renderer converts it to
/// linear at upload like every other source (§21a).
fn blend_mask(pixels: &mut [u8], mask: &Mask, color: Rgba) {
    for y in 0..mask.height {
        for x in 0..mask.width {
            let coverage = mask.at(x, y);
            if coverage == 0 {
                continue;
            }
            let source_alpha = (coverage as f32 / 255.0) * (color.a as f32 / 255.0);
            blend_pixel(pixels, (y * mask.width + x) * 4, color, source_alpha);
        }
    }
}

fn blend_pixel(pixels: &mut [u8], index: usize, color: Rgba, source_alpha: f32) {
    if source_alpha <= 0.0 {
        return;
    }
    let dest_alpha = pixels[index + 3] as f32 / 255.0;
    let out_alpha = source_alpha + dest_alpha * (1.0 - source_alpha);
    if out_alpha <= 0.0 {
        return;
    }

    for channel in 0..3 {
        let source = [color.r, color.g, color.b][channel] as f32;
        let dest = pixels[index + channel] as f32;
        let value = (source * source_alpha + dest * dest_alpha * (1.0 - source_alpha)) / out_alpha;
        pixels[index + channel] = value.round().clamp(0.0, 255.0) as u8;
    }
    pixels[index + 3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// A filled box with rounded corners, antialiased at the edges.
///
/// Written here rather than pulled in as a drawing library: it is one shape,
/// and §74 asks for a reason before a dependency arrives.
#[allow(clippy::too_many_arguments)]
fn fill_rounded_rect(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
    color: Rgba,
) {
    let radius = radius
        .min((right - left) / 2.0)
        .min((bottom - top) / 2.0)
        .max(0.0);

    let x0 = left.floor().max(0.0) as u32;
    let y0 = top.floor().max(0.0) as u32;
    let x1 = right.ceil().min(width as f32) as u32;
    let y1 = bottom.ceil().min(height as f32) as u32;

    for y in y0..y1 {
        for x in x0..x1 {
            // Pixel centre, so the antialiasing is symmetric about the edge.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            // Signed distance to the rounded rectangle: negative inside,
            // positive outside, and the corners measured to their centre of
            // curvature. It has to be *signed* — clamping the components at
            // zero measures only how far outside a pixel is, which leaves
            // every interior pixel reading as exactly on the edge, and the
            // whole box comes out at half coverage.
            let cx = (px - (left + right) / 2.0).abs() - ((right - left) / 2.0 - radius);
            let cy = (py - (top + bottom) / 2.0).abs() - ((bottom - top) / 2.0 - radius);
            let corner = (cx.max(0.0).powi(2) + cy.max(0.0).powi(2)).sqrt();
            let outside = corner + cx.max(cy).min(0.0) - radius;

            let coverage = (0.5 - outside).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let index = ((y * width + x) * 4) as usize;
            blend_pixel(pixels, index, color, coverage * (color.a as f32 / 255.0));
        }
    }
}
