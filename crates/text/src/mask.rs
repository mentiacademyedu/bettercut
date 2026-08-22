//! Coverage masks, and the two operations the decorations need.
//!
//! A mask is one byte of coverage per pixel — what a glyph rasterizer produces
//! before anything is coloured in. Keeping the shapes separate from the colours
//! is what lets a stroke and a shadow be derived from the *same* glyphs rather
//! than shaped twice at different offsets.

/// One byte of coverage per pixel.
#[derive(Debug, Clone)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
}

impl Mask {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            alpha: vec![0; width * height],
        }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.alpha[y * self.width + x]
    }

    /// Cover a pixel, keeping whatever was already there.
    ///
    /// Max rather than add: two glyphs overlapping — an accent over a letter,
    /// or a tight script — must not produce a brighter patch where they cross.
    #[inline]
    pub fn cover(&mut self, x: usize, y: usize, alpha: u8) {
        let slot = &mut self.alpha[y * self.width + x];
        *slot = (*slot).max(alpha);
    }

    pub fn is_empty(&self) -> bool {
        self.alpha.iter().all(|&a| a == 0)
    }

    /// Grow the covered area outwards by `radius` pixels (§26's stroke).
    ///
    /// Done with a distance transform rather than by maxing over a disc around
    /// every pixel: the disc costs O(radius²) per pixel and a 16-pixel outline
    /// on a title-sized bitmap is hundreds of millions of operations, which the
    /// preview would have to pay on every keystroke. Two sweeps cost the same
    /// whatever the radius.
    ///
    /// The distance also comes out fractional, which gives the outline a soft
    /// edge for free — a hard threshold would leave it visibly jagged next to
    /// the antialiased glyphs it surrounds.
    pub fn dilated(&self, radius: f32) -> Self {
        if radius <= 0.0 {
            return self.clone();
        }
        let distance = self.distance_field();
        let mut out = Self::new(self.width, self.height);
        for (slot, (&d, &inside)) in out
            .alpha
            .iter_mut()
            .zip(distance.iter().zip(self.alpha.iter()))
        {
            // Half a pixel of feather, centred on the radius, so the edge lands
            // where an antialiased rasterizer would have put it.
            let coverage = (radius + 0.5 - d).clamp(0.0, 1.0);
            *slot = ((coverage * 255.0) as u8).max(inside);
        }
        out
    }

    /// Distance from every pixel to the nearest covered one, in pixels.
    ///
    /// Chamfer 3-4: two sweeps over the image propagating the best distance
    /// from already-visited neighbours. Within a couple of percent of true
    /// Euclidean distance, which is far below what a soft outline edge can
    /// show, and linear in the number of pixels.
    fn distance_field(&self) -> Vec<f32> {
        // Scaled integers: 3 for an orthogonal step, 4 for a diagonal one,
        // divided by 3 at the end. Integer arithmetic throughout so the two
        // sweeps cannot disagree by a rounding bit.
        const ORTHOGONAL: u32 = 3;
        const DIAGONAL: u32 = 4;
        const FAR: u32 = u32::MAX / 4;

        let (w, h) = (self.width, self.height);
        let mut d: Vec<u32> = self
            .alpha
            .iter()
            // Half coverage is the boundary: below it the pixel is outside the
            // glyph, above it inside. The fractional part is recovered by the
            // feather in `dilated`, not here.
            .map(|&a| if a >= 128 { 0 } else { FAR })
            .collect();

        let relax = |d: &mut Vec<u32>, here: usize, other: usize, cost: u32| {
            let candidate = d[other].saturating_add(cost);
            if candidate < d[here] {
                d[here] = candidate;
            }
        };

        for y in 0..h {
            for x in 0..w {
                let here = y * w + x;
                if y > 0 {
                    relax(&mut d, here, here - w, ORTHOGONAL);
                    if x > 0 {
                        relax(&mut d, here, here - w - 1, DIAGONAL);
                    }
                    if x + 1 < w {
                        relax(&mut d, here, here - w + 1, DIAGONAL);
                    }
                }
                if x > 0 {
                    relax(&mut d, here, here - 1, ORTHOGONAL);
                }
            }
        }

        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let here = y * w + x;
                if y + 1 < h {
                    relax(&mut d, here, here + w, ORTHOGONAL);
                    if x > 0 {
                        relax(&mut d, here, here + w - 1, DIAGONAL);
                    }
                    if x + 1 < w {
                        relax(&mut d, here, here + w + 1, DIAGONAL);
                    }
                }
                if x + 1 < w {
                    relax(&mut d, here, here + 1, ORTHOGONAL);
                }
            }
        }

        d.into_iter()
            .map(|v| v as f32 / ORTHOGONAL as f32)
            .collect()
    }

    /// Blur the coverage (§26's shadow).
    ///
    /// Three box blurs, which is the standard approximation of a Gaussian —
    /// the third pass is already within a percent or so of the real thing, and
    /// a true Gaussian kernel would cost more for a difference nobody can see
    /// behind a piece of text.
    pub fn blurred(&self, radius: f32) -> Self {
        if radius <= 0.5 {
            return self.clone();
        }
        // Each box pass covers about a third of the total spread.
        let box_radius = (radius / 3.0).round().max(1.0) as usize;
        let mut current = self.clone();
        for _ in 0..3 {
            current = current.box_blurred(box_radius);
        }
        current
    }

    fn box_blurred(&self, radius: usize) -> Self {
        let (w, h) = (self.width, self.height);
        let mut horizontal = Self::new(w, h);
        let window = (radius * 2 + 1) as u32;

        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for offset in 0..window {
                    // Clamped at the edges, so a shadow that runs off the
                    // bitmap fades rather than wrapping round to the far side.
                    let sx = (x + offset as usize).saturating_sub(radius).min(w - 1);
                    sum += self.at(sx, y) as u32;
                }
                // Rounded, not truncated. Three passes of a truncating divide
                // each lose up to a whole level, and the losses compound —
                // a soft shadow would come out systematically darker than the
                // one asked for, and a faint one would vanish outright.
                horizontal.alpha[y * w + x] = ((sum + window / 2) / window) as u8;
            }
        }

        let mut out = Self::new(w, h);
        for x in 0..w {
            for y in 0..h {
                let mut sum = 0u32;
                for offset in 0..window {
                    let sy = (y + offset as usize).saturating_sub(radius).min(h - 1);
                    sum += horizontal.at(x, sy) as u32;
                }
                out.alpha[y * w + x] = ((sum + window / 2) / window) as u8;
            }
        }
        out
    }

    /// Copy this mask into a new one shifted by `(dx, dy)`.
    ///
    /// Whole pixels only. A shadow offset by a fraction of a pixel is a
    /// difference nobody can see through the blur that follows it.
    pub fn offset(&self, dx: i32, dy: i32) -> Self {
        let mut out = Self::new(self.width, self.height);
        for y in 0..self.height {
            let ty = y as i32 + dy;
            if ty < 0 || ty >= self.height as i32 {
                continue;
            }
            for x in 0..self.width {
                let tx = x as i32 + dx;
                if tx < 0 || tx >= self.width as i32 {
                    continue;
                }
                out.alpha[ty as usize * self.width + tx as usize] = self.at(x, y);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single covered pixel, for shapes small enough to reason about by hand.
    fn dot(width: usize, height: usize, x: usize, y: usize) -> Mask {
        let mut mask = Mask::new(width, height);
        mask.cover(x, y, 255);
        mask
    }

    #[test]
    fn covering_keeps_the_stronger_value() {
        let mut mask = Mask::new(2, 1);
        mask.cover(0, 0, 200);
        mask.cover(0, 0, 100);
        assert_eq!(mask.at(0, 0), 200, "a weaker glyph erased a stronger one");
    }

    /// The point of the distance transform: growing by *n* reaches *n* pixels,
    /// not n² work and not a square.
    #[test]
    fn dilating_grows_the_covered_area() {
        let grown = dot(21, 21, 10, 10).dilated(4.0);

        assert_eq!(grown.at(10, 10), 255, "the centre stopped being covered");
        assert!(grown.at(10, 6) > 0, "did not reach 4 pixels up");
        assert_eq!(grown.at(10, 2), 0, "reached 8 pixels up");
    }

    /// A square dilation would reach as far diagonally as it does straight up,
    /// which is what a max-over-a-box gives and what makes an outline look
    /// boxy at the corners.
    #[test]
    fn dilating_is_round_not_square() {
        let grown = dot(41, 41, 20, 20).dilated(8.0);

        assert!(grown.at(20, 13) > 0, "did not reach 7 pixels straight up");
        assert_eq!(
            grown.at(27, 27),
            0,
            "reached the corner of a 7-pixel box, so the outline is square"
        );
    }

    #[test]
    fn dilating_by_nothing_changes_nothing() {
        let mask = dot(9, 9, 4, 4);
        assert_eq!(mask.dilated(0.0).alpha, mask.alpha);
    }

    /// Blur spreads coverage outwards and takes the peak down with it. A blur
    /// that kept the peak would be a glow, not a shadow.
    /// A filled blob rather than a single pixel: one pixel of coverage spread
    /// over a nine-pixel radius really does come to less than one level
    /// everywhere, so it would be testing arithmetic rather than blur.
    #[test]
    fn blurring_spreads_and_softens() {
        let mut mask = Mask::new(41, 41);
        for y in 16..25 {
            for x in 16..25 {
                mask.cover(x, y, 255);
            }
        }
        let blurred = mask.blurred(9.0);

        assert!(
            blurred.at(20, 20) < 255,
            "the peak survived the blur untouched"
        );
        assert!(blurred.at(20, 8) > 0, "nothing spread past the blob");
        assert!(
            blurred.at(20, 8) < blurred.at(20, 14),
            "further from the source was not dimmer"
        );
    }

    /// Coverage must not wrap from one edge to the other. A shadow falling off
    /// the bottom reappearing at the top would be a memorable bug.
    #[test]
    fn blurring_does_not_wrap_around_the_edges() {
        let mut mask = Mask::new(21, 21);
        for x in 0..21 {
            mask.cover(x, 20, 255); // a line along the bottom
        }
        let blurred = mask.blurred(6.0);
        assert_eq!(blurred.at(10, 0), 0, "the bottom edge bled into the top");
    }

    #[test]
    fn offsetting_moves_coverage_and_drops_what_falls_off() {
        let shifted = dot(9, 9, 4, 4).offset(2, -3);
        assert_eq!(shifted.at(6, 1), 255);
        assert_eq!(shifted.at(4, 4), 0);

        let gone = dot(9, 9, 0, 0).offset(-4, 0);
        assert!(gone.is_empty(), "coverage pushed off the edge came back");
    }
}
