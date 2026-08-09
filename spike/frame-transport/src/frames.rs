//! Milestone 0 spike — synthetic frame source.
//!
//! Stands in for the decoder until FFmpeg is available. The point is to measure
//! the *transport* (RAM -> GPU -> composite -> egui -> screen), so this
//! deliberately does almost no work per frame: it memcpys rows out of a
//! pre-built pattern. Whatever time the HUD attributes to "upload" is therefore
//! upload, not the frame generator.
//!
//! The pattern is built to make dropped and duplicated frames *visible*:
//!   - a scrolling vertical bar (judder shows as a stutter in the slide)
//!   - a frame-number bar chart (a repeated frame is obvious)
//!   - a full-white flash frame once per second (lines up with the audio click)

pub struct SyntheticSource {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Two screen-widths of pattern so a horizontal scroll is a pure row memcpy.
    pattern: Vec<u8>,
    staging: Vec<u8>,
}

impl SyntheticSource {
    pub fn new(width: u32, height: u32, fps: f64) -> Self {
        let (w, h) = (width as usize, height as usize);
        let pw = w * 2;
        let mut pattern = vec![0u8; pw * h * 4];

        for y in 0..h {
            let v = y as f32 / h as f32;
            for x in 0..pw {
                let u = x as f32 / pw as f32;
                // A smooth gradient plus 64-px grid lines: the grid makes any
                // scaling or filtering error in the preview immediately visible.
                let grid = (x % 64 == 0 || y % 64 == 0) as u8;
                let r = (40.0 + 190.0 * u) as u8;
                let g = (30.0 + 150.0 * v) as u8;
                let b = (90.0 + 120.0 * (1.0 - u) * (1.0 - v)) as u8;

                let i = (y * pw + x) * 4;
                pattern[i] = r.saturating_add(grid * 50);
                pattern[i + 1] = g.saturating_add(grid * 50);
                pattern[i + 2] = b.saturating_add(grid * 50);
                pattern[i + 3] = 255;
            }
        }

        Self {
            width,
            height,
            fps,
            pattern,
            staging: vec![0u8; w * h * 4],
        }
    }

    pub fn frame_bytes(&self) -> usize {
        self.staging.len()
    }

    /// Produce the frame at `index` into the staging buffer, as a decoder would
    /// hand back a decoded frame in system RAM.
    pub fn frame(&mut self, index: u64) -> &[u8] {
        let (w, h) = (self.width as usize, self.height as usize);
        let pw = w * 2;

        // Scroll one screen-width every two seconds.
        let scroll = ((index as f64 * (w as f64 / (self.fps * 2.0))) as usize) % w;

        for y in 0..h {
            let src = (y * pw + scroll) * 4;
            let dst = y * w * 4;
            self.staging[dst..dst + w * 4].copy_from_slice(&self.pattern[src..src + w * 4]);
        }

        // Flash frame once per second, aligned with the audio click. If video
        // and audio drift, you see the flash and hear the click at different
        // moments — an A/V sync bug you can perceive without a measurement.
        let fps_i = self.fps.round() as u64;
        if fps_i > 0 && index % fps_i == 0 {
            for px in self.staging.chunks_exact_mut(4) {
                px[0] = 255;
                px[1] = 255;
                px[2] = 255;
            }
        }

        // Frame-counter bar: 16 px tall, length = (index % 120) columns.
        let bar_len = ((index % 120) as usize * w / 120).min(w);
        for y in 0..16.min(h) {
            for x in 0..bar_len {
                let i = (y * w + x) * 4;
                self.staging[i] = 255;
                self.staging[i + 1] = 40;
                self.staging[i + 2] = 40;
            }
        }

        &self.staging
    }
}

/// A translucent overlay layer, uploaded once and left resident on the GPU —
/// the "already a texture" half of §5, against which the per-frame upload path
/// is compared.
pub fn build_overlay(width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut buf = vec![0u8; w * h * 4];

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let radius = (w.min(h) as f32) * 0.42;

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            // Soft-edged disc, so the composite exercises real alpha blending
            // rather than a binary mask.
            let alpha = ((radius - dist) / 24.0).clamp(0.0, 1.0);
            let checker = (((x / 32) + (y / 32)) % 2) as f32;

            let i = (y * w + x) * 4;
            buf[i] = (60.0 + 180.0 * checker) as u8;
            buf[i + 1] = (200.0 - 120.0 * checker) as u8;
            buf[i + 2] = 240;
            buf[i + 3] = (alpha * 200.0) as u8;
        }
    }

    buf
}
