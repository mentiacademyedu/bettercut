//! Shaping and rasterization — the one implementation §26.1 requires.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};

use crate::error::TextError;
use crate::mask::Mask;
use crate::style::{Alignment, FontFamily, Rgba, TextStyle};

/// A rasterized piece of text: sRGB-encoded RGBA with straight alpha, tightly
/// packed, ready to upload as a texture (§21a).
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
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
        }
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
        let style = style.sanitized();
        if text.trim().is_empty() {
            return Err(TextError::Empty);
        }

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

        let glyphs = self.glyph_mask(&mut buffer, width as usize, height as usize, margin);
        if glyphs.is_empty() {
            // Shaping succeeded but nothing was drawn: every character is
            // missing from every font available. Saying "empty" is honest —
            // there is no picture to composite.
            return Err(TextError::Empty);
        }

        Ok(compose(
            &glyphs,
            &style,
            width,
            height,
            margin,
            text_width,
            text_height,
        ))
    }

    /// Draw the shaped glyphs into a coverage mask.
    ///
    /// White and fully opaque, so the alpha the rasterizer reports *is* the
    /// coverage. Colour is applied afterwards, once, to whichever of the
    /// stroke, shadow and fill wants it.
    fn glyph_mask(
        &mut self,
        buffer: &mut Buffer,
        width: usize,
        height: usize,
        margin: f32,
    ) -> Mask {
        let mut mask = Mask::new(width, height);
        let offset = margin.round() as i32;
        let white = cosmic_text::Color::rgba(255, 255, 255, 255);

        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            white,
            |x, y, w, h, color| {
                let alpha = color.a();
                if alpha == 0 {
                    return;
                }
                for dy in 0..h as i32 {
                    for dx in 0..w as i32 {
                        let px = x + dx + offset;
                        let py = y + dy + offset;
                        if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                            continue;
                        }
                        mask.cover(px as usize, py as usize, alpha);
                    }
                }
            },
        );

        mask
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
fn compose(
    glyphs: &Mask,
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

    blend_mask(&mut pixels, glyphs, style.color);

    TextBitmap {
        width,
        height,
        pixels,
    }
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
