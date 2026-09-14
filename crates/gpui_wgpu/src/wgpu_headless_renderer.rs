use crate::{GpuContext, WgpuAtlas, WgpuRenderer, WgpuSurfaceConfig};
use anyhow::{Context as _, Result};
use gpui::{
    DevicePixels, ExternalTexture, ExternalTextureId, PlatformAtlas, PlatformHeadlessRenderer,
    Scene, Size,
};
use std::sync::Arc;

struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: Size<DevicePixels>,
    /// Fresh per allocation, so hosts notice when the target was recreated.
    external_id: ExternalTextureId,
}

/// A [`WgpuRenderer`] that draws into a texture it owns instead of a window
/// surface. The texture is (re)allocated lazily to the size of each frame.
pub struct WgpuHeadlessRenderer {
    renderer: WgpuRenderer,
    target: Option<OffscreenTarget>,
}

impl WgpuHeadlessRenderer {
    /// Creates a renderer sharing `gpu_context`. An empty context is
    /// initialized headlessly (no display connection).
    #[cfg(not(target_family = "wasm"))]
    pub fn new(gpu_context: GpuContext) -> Result<Self> {
        Self::new_with_instance(gpu_context, None)
    }

    /// Like [`Self::new`], but an empty context is initialized from `instance`,
    /// which should be bound to the display server if real windows will share
    /// the context later.
    #[cfg(not(target_family = "wasm"))]
    pub fn new_with_instance(
        gpu_context: GpuContext,
        instance: Option<wgpu::Instance>,
    ) -> Result<Self> {
        let renderer = WgpuRenderer::new_offscreen(
            gpu_context,
            instance,
            WgpuSurfaceConfig {
                size: Size {
                    width: DevicePixels(1),
                    height: DevicePixels(1),
                },
                transparent: true,
                preferred_present_mode: None,
            },
        )?;
        Ok(Self {
            renderer,
            target: None,
        })
    }

    /// The texture holding the most recent frame, if any frame was rendered.
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|target| &target.texture)
    }

    /// A view of the most recent frame's texture, suitable for sampling from
    /// another renderer on the same device.
    pub fn texture_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|target| &target.view)
    }

    pub fn texture_size(&self) -> Option<Size<DevicePixels>> {
        self.target.as_ref().map(|target| target.size)
    }

    /// The current target as a texture another renderer on this device can
    /// register with its atlas and sample. `None` until a frame was rendered.
    pub fn external_texture(&self) -> Option<ExternalTexture> {
        let target = self.target.as_ref()?;
        Some(ExternalTexture {
            id: target.external_id,
            size: target.size,
            premultiplied_alpha: self.renderer.premultiplied_alpha(),
            handle: Arc::new(target.view.clone()),
        })
    }

    pub fn wgpu_atlas(&self) -> &Arc<WgpuAtlas> {
        self.renderer.sprite_atlas()
    }

    pub fn device_lost(&self) -> bool {
        self.renderer.device_lost()
    }

    pub fn gpu_specs(&self) -> gpui::GpuSpecs {
        self.renderer.gpu_specs()
    }

    pub fn supports_dual_source_blending(&self) -> bool {
        self.renderer.supports_dual_source_blending()
    }

    fn ensure_target(&mut self, size: Size<DevicePixels>) -> Result<&OffscreenTarget> {
        let max = self.renderer.max_texture_size() as i32;
        let size = Size {
            width: DevicePixels(size.width.0.clamp(1, max)),
            height: DevicePixels(size.height.0.clamp(1, max)),
        };
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.size != size)
        {
            let texture = self
                .renderer
                .device()
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("offscreen_target"),
                    size: wgpu::Extent3d {
                        width: size.width.0 as u32,
                        height: size.height.0 as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.renderer.color_format(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.target = Some(OffscreenTarget {
                texture,
                view,
                size,
                external_id: ExternalTextureId::next(),
            });
        }
        self.target
            .as_ref()
            .context("offscreen target was not created")
    }

    /// Copies the most recently rendered frame back to the CPU as RGBA rows
    /// with premultiplied alpha, without rendering again.
    pub fn read_frame(&self) -> Result<image::RgbaImage> {
        let target = self
            .target
            .as_ref()
            .context("no frame has been rendered yet")?;
        let width = target.size.width.0 as u32;
        let height = target.size.height.0 as u32;
        let bytes_per_pixel = 4u32;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let padded_bytes_per_row =
            unpadded_bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

        let device = self.renderer.device();
        let queue = self.renderer.queue();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen_readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("offscreen_readback_encoder"),
        });
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).ok();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .context("waiting for offscreen readback")?;
        receiver
            .recv()
            .context("readback mapping callback never fired")?
            .context("mapping offscreen readback buffer")?;

        let swap_red_blue = match self.renderer.color_format() {
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
            _ => false,
        };
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in mapped.chunks_exact(padded_bytes_per_row as usize) {
            let row = &row[..unpadded_bytes_per_row as usize];
            if swap_red_blue {
                for pixel in row.chunks_exact(4) {
                    pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            } else {
                pixels.extend_from_slice(row);
            }
        }
        drop(mapped);
        buffer.unmap();

        image::RgbaImage::from_raw(width, height, pixels)
            .context("readback size did not match target size")
    }
}

impl PlatformHeadlessRenderer for WgpuHeadlessRenderer {
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<image::RgbaImage> {
        self.render_scene(scene, size)?;
        self.read_frame()
    }

    fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<()> {
        let view = self.ensure_target(size)?.view.clone();
        let size = self
            .target
            .as_ref()
            .context("offscreen target was not created")?
            .size;
        self.renderer.render_offscreen(scene, &view, size)
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }

    fn external_texture(&self) -> Option<ExternalTexture> {
        WgpuHeadlessRenderer::external_texture(self)
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use gpui::{
        Background, BorderStyle, Bounds, ContentMask, Corners, Edges, Hsla, Quad, ScaledPixels,
        point, size,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn renders_a_quad_offscreen() {
        let gpu_context: GpuContext = Rc::new(RefCell::new(None));
        let mut renderer = match WgpuHeadlessRenderer::new(gpu_context) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("skipping: no usable GPU adapter ({error:#})");
                return;
            }
        };

        let target_size = size(DevicePixels(8), DevicePixels(8));
        let full = Bounds {
            origin: point(ScaledPixels(0.), ScaledPixels(0.)),
            size: size(ScaledPixels(8.), ScaledPixels(8.)),
        };
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            order: 0,
            border_style: BorderStyle::default(),
            bounds: Bounds {
                origin: point(ScaledPixels(0.), ScaledPixels(0.)),
                size: size(ScaledPixels(4.), ScaledPixels(4.)),
            },
            content_mask: ContentMask { bounds: full },
            background: Background::from(Hsla {
                h: 0.,
                s: 1.,
                l: 0.5,
                a: 1.,
            }),
            border_color: Hsla::transparent_black(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
        });
        scene.finish();

        let image = renderer
            .render_scene_to_image(&scene, target_size)
            .expect("offscreen render should succeed");
        assert_eq!(image.dimensions(), (8, 8));

        let inside = image.get_pixel(1, 1).0;
        assert!(
            inside[0] > 200 && inside[1] < 40 && inside[2] < 40 && inside[3] > 200,
            "{inside:?}"
        );
        let outside = image.get_pixel(6, 6).0;
        assert_eq!(outside[3], 0, "{outside:?}");

        // A second frame at a different size must reallocate the target.
        let image = renderer
            .render_scene_to_image(&scene, size(DevicePixels(3), DevicePixels(5)))
            .expect("resized offscreen render should succeed");
        assert_eq!(image.dimensions(), (3, 5));
    }
}
