//! Getting a composited frame off the GPU.
//!
//! §5 forbids a RAM round trip *per frame in the preview path*, and it is right
//! to: that tax is what the target hardware cannot afford while playing.
//!
//! Export is the other case. The frame has to reach an encoder that reads
//! system memory, so the download is not overhead — it is the delivery. What
//! §5 asks for here is that it happen once per frame rather than twice, and
//! that the buffer be reused rather than reallocated: an 8 MB staging buffer
//! allocated per frame is 480 MB a minute through the driver (§68, §73).

use bettercut_renderer::wgpu;
use bettercut_timeline::Resolution;

/// A reusable staging buffer sized for one frame.
pub struct Readback {
    buffer: wgpu::Buffer,
    resolution: Resolution,
    /// Bytes per row, rounded up to wgpu's 256-byte copy alignment.
    padded_row: u32,
}

impl Readback {
    pub fn new(device: &wgpu::Device, resolution: Resolution) -> Self {
        let padded_row = (resolution.width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export readback"),
            size: u64::from(padded_row) * u64::from(resolution.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            resolution,
            padded_row,
        }
    }

    /// Copy the target texture into system memory, tightly packed.
    ///
    /// The padding wgpu requires on each row is removed here rather than passed
    /// on, because the encoder wants a plain image and every caller would
    /// otherwise have to know about the alignment rule.
    pub fn read(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Vec<u8> {
        let (width, height) = (self.resolution.width, self.resolution.height);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("export readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        // Export is not racing a frame deadline, so waiting for the copy is the
        // simple correct thing. A pipelined version would overlap the encode of
        // frame N with the render of N+1; that is a speed-up to make once the
        // whole path is measured, not a correctness question.
        if let Err(err) = device.poll(wgpu::PollType::wait_indefinitely()) {
            tracing::error!(?err, "readback poll failed; frame will be black");
            return vec![0; (width * height * 4) as usize];
        }

        let row_bytes = (width * 4) as usize;
        let mut out = Vec::with_capacity(row_bytes * height as usize);
        match slice.get_mapped_range() {
            Ok(mapped) => {
                for y in 0..height as usize {
                    let start = y * self.padded_row as usize;
                    out.extend_from_slice(&mapped[start..start + row_bytes]);
                }
                drop(mapped);
            }
            Err(err) => {
                tracing::error!(?err, "could not map the readback buffer");
                out.resize(row_bytes * height as usize, 0);
            }
        }
        self.buffer.unmap();
        out
    }
}
