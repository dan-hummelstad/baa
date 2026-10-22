//! CEF reclaims the IOSurface it hands to `on_accelerated_paint` as soon as the
//! callback returns, so each frame is blitted on the GPU into a surface we own.
//! That costs a fraction of a millisecond; the CPU path costs a 17 MB copy plus
//! a texture upload per frame at 2560x1722.

use core_foundation::{base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString};
use core_video::{
    metal_texture::CVMetalTextureGetTexture,
    metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::{kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferKeys},
};
use foreign_types::ForeignTypeRef as _;
use metal::{CommandQueue, MTLOrigin, MTLPixelFormat, MTLSize, TextureRef};

// ponytail: 3 buffers assumes gpui draws at most one frame behind. Ceiling: a
// stalled renderer could see a buffer rewritten under it (one torn frame).
const POOL: usize = 3;

pub(crate) struct GpuCopy {
    queue: CommandQueue,
    cache: CVMetalTextureCache,
    pool: Vec<CVPixelBuffer>,
    next: usize,
}

impl GpuCopy {
    pub(crate) fn new() -> Option<Self> {
        let device = metal::Device::system_default()?;
        let queue = device.new_command_queue();
        let cache = CVMetalTextureCache::new(None, device, None).ok()?;
        Some(Self { queue, cache, pool: Vec::new(), next: 0 })
    }

    /// A copy of `source` (BGRA, CEF's) that stays valid after the callback.
    pub(crate) fn copy(&mut self, source: &CVPixelBuffer) -> Option<CVPixelBuffer> {
        let (width, height) = (source.get_width(), source.get_height());
        if self.pool.first().is_none_or(|b| b.get_width() != width || b.get_height() != height) {
            self.pool = (0..POOL).map(|_| owned_buffer(width, height)).collect::<Option<_>>()?;
        }
        let target = self.pool[self.next].clone();
        self.next = (self.next + 1) % POOL;

        let texture = |buffer: &CVPixelBuffer| {
            self.cache.create_texture_from_image(
                buffer.as_concrete_TypeRef(),
                None,
                MTLPixelFormat::BGRA8Unorm,
                width,
                height,
                0,
            )
        };
        let (from, to) = (texture(source).ok()?, texture(&target).ok()?);
        let metal_texture = |t: &core_video::metal_texture::CVMetalTexture| unsafe {
            TextureRef::from_ptr(CVMetalTextureGetTexture(t.as_concrete_TypeRef()) as *mut _)
        };
        let origin = MTLOrigin { x: 0, y: 0, z: 0 };
        let commands = self.queue.new_command_buffer();
        let blit = commands.new_blit_command_encoder();
        blit.copy_from_texture(
            metal_texture(&from),
            0,
            0,
            origin,
            MTLSize { width: width as u64, height: height as u64, depth: 1 },
            metal_texture(&to),
            0,
            0,
            origin,
        );
        blit.end_encoding();
        commands.commit();
        // CEF may reuse the source as soon as we return.
        commands.wait_until_completed();
        Some(target)
    }
}

fn owned_buffer(width: usize, height: usize) -> Option<CVPixelBuffer> {
    let options = CFDictionary::from_CFType_pairs(&[
        (
            CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
            CFDictionary::<CFString, CFString>::from_CFType_pairs(&[]).as_CFType(),
        ),
        (CFString::from(CVPixelBufferKeys::MetalCompatibility), CFBoolean::true_value().as_CFType()),
    ]);
    CVPixelBuffer::new(kCVPixelFormatType_32BGRA, width, height, Some(&options)).ok()
}
