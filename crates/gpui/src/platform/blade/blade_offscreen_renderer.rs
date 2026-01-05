//! Offscreen renderer for render-to-texture support.
//!
//! This module provides `BladeOffscreenRenderer`, a lightweight renderer that shares
//! expensive GPU resources with the main `BladeRenderer` while maintaining its own
//! per-instance state for safe concurrent/reentrant rendering.

use super::BladeAtlas;
use super::blade_renderer::{
    RenderContext, SharedRenderResources, create_msaa_texture_if_needed,
    create_path_intermediate_texture, render_batches,
};
use blade_graphics as gpu;
use blade_util::{BufferBelt, BufferBeltDescriptor};
use std::collections::HashMap;
use std::sync::Arc;

use crate::platform::{
    OffscreenTextureId, OffscreenTextureInfo, PlatformOffscreenRenderer, TextureData,
};
use crate::{DevicePixels, Scene, Size};

/// A lightweight offscreen renderer that can render scenes to textures.
///
/// This renderer shares expensive GPU resources (pipelines, atlas, etc.) with the main
/// `BladeRenderer` while maintaining its own command encoder and instance buffers for
/// isolation. This allows render-to-texture operations without interfering with the
/// main rendering pipeline.
///
/// # Usage
///
/// ```ignore
/// // Create an offscreen renderer from the main renderer
/// let offscreen = blade_renderer.create_offscreen_renderer(max_size)?;
///
/// // Create a texture to render to
/// let texture = offscreen.create_texture(width, height);
///
/// // Render a scene to the texture
/// offscreen.draw_to_texture(&scene, &texture);
///
/// // Clean up when done
/// offscreen.destroy();
/// ```
pub struct BladeOffscreenRenderer {
    // Shared resources (from main renderer)
    gpu: Arc<gpu::Context>,
    pipelines: Arc<BladePipelines>,
    atlas: Arc<BladeAtlas>,
    atlas_sampler: gpu::Sampler,
    rendering_parameters: RenderingParameters,

    // Per-instance state
    command_encoder: gpu::CommandEncoder,
    instance_belt: BufferBelt,
    path_intermediate_texture: gpu::Texture,
    path_intermediate_texture_view: gpu::TextureView,
    path_intermediate_msaa_texture: Option<gpu::Texture>,
    path_intermediate_msaa_texture_view: Option<gpu::TextureView>,

    // Texture management
    textures: HashMap<u64, OffscreenTexture>,
    next_texture_id: u64,

    // Configuration
    max_texture_size: gpu::Extent,
    surface_format: gpu::TextureFormat,
}

// Re-import types from blade_renderer that we need
use super::blade_renderer::{BladePipelines, RenderingParameters};

/// A GPU texture that can be rendered to by `BladeOffscreenRenderer`.
pub struct OffscreenTexture {
    pub(crate) texture: gpu::Texture,
    pub(crate) view: gpu::TextureView,
    pub(crate) size: Size<u32>,
}

impl OffscreenTexture {
    /// Returns the size of the texture in pixels.
    pub fn size(&self) -> Size<u32> {
        self.size
    }

    /// Returns the raw GPU texture handle.
    pub fn raw_texture(&self) -> gpu::Texture {
        self.texture
    }

    /// Returns the raw GPU texture view handle.
    pub fn raw_view(&self) -> gpu::TextureView {
        self.view
    }
}

impl BladeOffscreenRenderer {
    /// Creates a new offscreen renderer that shares resources with the main renderer.
    ///
    /// # Arguments
    ///
    /// * `shared` - Shared resources from the main `BladeRenderer`
    /// * `max_texture_size` - Maximum size of textures this renderer will create
    /// * `surface_format` - The texture format to use (should match main renderer)
    pub(super) fn new(
        shared: &SharedRenderResources,
        max_texture_size: gpu::Extent,
        surface_format: gpu::TextureFormat,
    ) -> Self {
        let command_encoder = shared.gpu.create_command_encoder(gpu::CommandEncoderDesc {
            name: "offscreen renderer",
            buffer_count: 2,
        });

        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40, // Vulkan `minStorageBufferOffsetAlignment` on Intel Xe
        });

        // Create path intermediate textures sized for max texture size
        let (path_intermediate_texture, path_intermediate_texture_view) =
            create_path_intermediate_texture(
                &shared.gpu,
                surface_format,
                max_texture_size.width,
                max_texture_size.height,
            );

        let (path_intermediate_msaa_texture, path_intermediate_msaa_texture_view) =
            create_msaa_texture_if_needed(
                &shared.gpu,
                surface_format,
                max_texture_size.width,
                max_texture_size.height,
                shared.rendering_parameters.path_sample_count,
            )
            .unzip();

        Self {
            gpu: Arc::clone(&shared.gpu),
            pipelines: Arc::clone(&shared.pipelines),
            atlas: Arc::clone(&shared.atlas),
            atlas_sampler: shared.atlas_sampler,
            rendering_parameters: shared.rendering_parameters.clone(),
            command_encoder,
            instance_belt,
            path_intermediate_texture,
            path_intermediate_texture_view,
            path_intermediate_msaa_texture,
            path_intermediate_msaa_texture_view,
            textures: HashMap::new(),
            next_texture_id: 1,
            max_texture_size,
            surface_format,
        }
    }

    /// Creates a new texture that can be rendered to.
    ///
    /// # Arguments
    ///
    /// * `width` - Width of the texture in pixels
    /// * `height` - Height of the texture in pixels
    ///
    /// # Panics
    ///
    /// Panics if the requested size exceeds `max_texture_size`.
    pub fn create_texture_raw(&self, width: u32, height: u32) -> OffscreenTexture {
        assert!(
            width <= self.max_texture_size.width && height <= self.max_texture_size.height,
            "Requested texture size {}x{} exceeds max size {}x{}",
            width,
            height,
            self.max_texture_size.width,
            self.max_texture_size.height
        );

        let texture = self.gpu.create_texture(gpu::TextureDesc {
            name: "offscreen target",
            format: self.surface_format,
            size: gpu::Extent {
                width,
                height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::COPY
                | gpu::TextureUsage::RESOURCE
                | gpu::TextureUsage::TARGET,
            external: None,
        });

        let view = self.gpu.create_texture_view(
            texture,
            gpu::TextureViewDesc {
                name: "offscreen target view",
                format: self.surface_format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );

        OffscreenTexture {
            texture,
            view,
            size: Size { width, height },
        }
    }

    /// Renders a scene to the specified texture.
    ///
    /// This method renders the scene using the same rendering logic as the main
    /// `BladeRenderer`, but outputs to the provided texture instead of the screen.
    ///
    /// # Arguments
    ///
    /// * `scene` - The scene to render
    /// * `texture` - The target texture to render to
    ///
    /// # Note
    ///
    /// This method does NOT handle macOS video surfaces (`PrimitiveBatch::Surfaces`),
    /// as they require platform-specific resources that are only available in the
    /// main renderer.
    pub fn draw_to_texture(&mut self, scene: &Scene, texture: &OffscreenTexture) {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);
        self.command_encoder.init_texture(texture.texture);

        let viewport_size = [texture.size.width as f32, texture.size.height as f32];

        // Create render context for this frame
        let mut ctx = RenderContext {
            command_encoder: &mut self.command_encoder,
            instance_belt: &mut self.instance_belt,
            path_intermediate_texture: self.path_intermediate_texture,
            path_intermediate_texture_view: self.path_intermediate_texture_view,
            path_intermediate_msaa_texture: self.path_intermediate_msaa_texture,
            path_intermediate_msaa_texture_view: self.path_intermediate_msaa_texture_view,
        };

        // Create a temporary SharedRenderResources view for render_batches
        // This is a bit awkward but avoids changing render_batches signature
        let shared = SharedRenderResources {
            gpu: Arc::clone(&self.gpu),
            pipelines: Arc::clone(&self.pipelines),
            atlas: Arc::clone(&self.atlas),
            atlas_sampler: self.atlas_sampler,
            rendering_parameters: self.rendering_parameters.clone(),
        };

        // Render all batches using shared logic
        // Offscreen textures use premultiplied alpha
        render_batches(
            scene,
            texture.view,
            viewport_size,
            true, // premultiplied_alpha
            &mut ctx,
            &shared,
        );

        let sync_point = self.gpu.submit(&mut self.command_encoder);

        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);

        // Wait for GPU to complete
        self.gpu.wait_for(&sync_point, 10000);
    }

    /// Destroys a texture created by this renderer.
    ///
    /// # Arguments
    ///
    /// * `texture` - The texture to destroy
    pub fn destroy_texture(&self, texture: OffscreenTexture) {
        self.gpu.destroy_texture_view(texture.view);
        self.gpu.destroy_texture(texture.texture);
    }

    /// Returns the maximum texture size this renderer supports.
    pub fn max_texture_size(&self) -> gpu::Extent {
        self.max_texture_size
    }

    /// Returns a reference to the shared atlas.
    pub fn atlas(&self) -> &Arc<BladeAtlas> {
        &self.atlas
    }

    /// Reads a texture's contents back from the GPU as RGBA data.
    ///
    /// This method copies the texture data from GPU memory to CPU memory,
    /// converting from the internal BGRA format to standard RGBA.
    ///
    /// # Arguments
    ///
    /// * `texture_id` - The ID of the texture to read
    ///
    /// # Returns
    ///
    /// The texture data as RGBA bytes, or an error if the texture doesn't exist
    /// or the read operation fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let texture_data = offscreen_renderer.read_texture(texture_id)?;
    /// // Save to PNG using the image crate:
    /// image::save_buffer(
    ///     "output.png",
    ///     texture_data.as_bytes(),
    ///     texture_data.width,
    ///     texture_data.height,
    ///     image::ColorType::Rgba8,
    /// )?;
    /// ```
    pub fn read_texture(&mut self, texture_id: OffscreenTextureId) -> anyhow::Result<TextureData> {
        let Some(texture) = self.textures.get(&texture_id.0) else {
            anyhow::bail!("Texture {:?} not found", texture_id);
        };

        let width = texture.size.width;
        let height = texture.size.height;
        let bytes_per_pixel = 4u32; // BGRA8
        let bytes_per_row = width * bytes_per_pixel;
        // Align to 256 bytes as required by some GPU APIs
        let aligned_bytes_per_row = (bytes_per_row + 255) & !255;
        let buffer_size = (aligned_bytes_per_row * height) as u64;

        // Create a buffer to hold the texture data
        let readback_buffer = self.gpu.create_buffer(gpu::BufferDesc {
            name: "texture readback",
            size: buffer_size,
            memory: gpu::Memory::Shared,
        });

        // Copy texture to buffer using transfer encoder
        self.command_encoder.start();
        {
            let mut transfers = self.command_encoder.transfer("texture readback");
            transfers.copy_texture_to_buffer(
                gpu::TexturePiece {
                    texture: texture.texture,
                    mip_level: 0,
                    array_layer: 0,
                    origin: [0, 0, 0],
                },
                gpu::BufferPiece {
                    buffer: readback_buffer,
                    offset: 0,
                },
                aligned_bytes_per_row,
                gpu::Extent {
                    width,
                    height,
                    depth: 1,
                },
            );
        }

        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.gpu.wait_for(&sync_point, 10000);

        // Read the buffer data - for Shared memory buffers, data() returns the mapped pointer
        let buffer_data = unsafe {
            let ptr = readback_buffer.data();
            std::slice::from_raw_parts(ptr as *const u8, buffer_size as usize)
        };

        // Convert from BGRA to RGBA and handle row alignment
        let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let row_start = (y * aligned_bytes_per_row) as usize;
            for x in 0..width {
                let pixel_start = row_start + (x * 4) as usize;
                // BGRA -> RGBA
                rgba_data.push(buffer_data[pixel_start + 2]); // R
                rgba_data.push(buffer_data[pixel_start + 1]); // G
                rgba_data.push(buffer_data[pixel_start]); // B
                rgba_data.push(buffer_data[pixel_start + 3]); // A
            }
        }

        // Destroy the buffer (Shared memory doesn't need explicit unmap)
        self.gpu.destroy_buffer(readback_buffer);

        Ok(TextureData::new(rgba_data, width, height))
    }

    /// Destroys this offscreen renderer and releases its resources.
    ///
    /// This should be called when the renderer is no longer needed.
    pub fn destroy(&mut self) {
        // Wait for any pending GPU work
        // Note: We don't have a last_sync_point like BladeRenderer, but the
        // wait in draw_to_texture should have completed any pending work.

        // Destroy all managed textures
        for (_, texture) in self.textures.drain() {
            self.gpu.destroy_texture_view(texture.view);
            self.gpu.destroy_texture(texture.texture);
        }

        self.instance_belt.destroy(&self.gpu);
        self.gpu.destroy_command_encoder(&mut self.command_encoder);
        self.gpu.destroy_texture(self.path_intermediate_texture);
        self.gpu
            .destroy_texture_view(self.path_intermediate_texture_view);

        if let Some(msaa_texture) = self.path_intermediate_msaa_texture {
            self.gpu.destroy_texture(msaa_texture);
        }
        if let Some(msaa_view) = self.path_intermediate_msaa_texture_view {
            self.gpu.destroy_texture_view(msaa_view);
        }

        // Note: We don't destroy shared resources (gpu, pipelines, atlas, sampler)
        // as they are owned by the main BladeRenderer
    }
}

// Implement PlatformOffscreenRenderer trait
impl PlatformOffscreenRenderer for BladeOffscreenRenderer {
    fn create_texture(&mut self, size: Size<DevicePixels>) -> OffscreenTextureInfo {
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;

        let texture = self.create_texture_raw(width, height);
        let id = self.next_texture_id;
        self.next_texture_id += 1;

        let info = OffscreenTextureInfo {
            id: OffscreenTextureId(id),
            size,
        };

        self.textures.insert(id, texture);
        info
    }

    fn draw_to_texture(&mut self, scene: &Scene, texture_id: OffscreenTextureId) {
        let Some(texture) = self.textures.get(&texture_id.0) else {
            log::error!(
                "Attempted to draw to non-existent texture: {:?}",
                texture_id
            );
            return;
        };

        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);
        self.command_encoder.init_texture(texture.texture);

        let viewport_size = [texture.size.width as f32, texture.size.height as f32];

        // Create render context for this frame
        let mut ctx = RenderContext {
            command_encoder: &mut self.command_encoder,
            instance_belt: &mut self.instance_belt,
            path_intermediate_texture: self.path_intermediate_texture,
            path_intermediate_texture_view: self.path_intermediate_texture_view,
            path_intermediate_msaa_texture: self.path_intermediate_msaa_texture,
            path_intermediate_msaa_texture_view: self.path_intermediate_msaa_texture_view,
        };

        // Create a temporary SharedRenderResources view for render_batches
        let shared = SharedRenderResources {
            gpu: Arc::clone(&self.gpu),
            pipelines: Arc::clone(&self.pipelines),
            atlas: Arc::clone(&self.atlas),
            atlas_sampler: self.atlas_sampler,
            rendering_parameters: self.rendering_parameters.clone(),
        };

        // Render all batches using shared logic
        // Offscreen textures use premultiplied alpha
        render_batches(
            scene,
            texture.view,
            viewport_size,
            true, // premultiplied_alpha
            &mut ctx,
            &shared,
        );

        let sync_point = self.gpu.submit(&mut self.command_encoder);

        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);

        // Wait for GPU to complete
        self.gpu.wait_for(&sync_point, 10000);
    }

    fn destroy_texture(&mut self, texture_id: OffscreenTextureId) {
        if let Some(texture) = self.textures.remove(&texture_id.0) {
            self.gpu.destroy_texture_view(texture.view);
            self.gpu.destroy_texture(texture.texture);
        } else {
            log::warn!(
                "Attempted to destroy non-existent texture: {:?}",
                texture_id
            );
        }
    }

    fn max_texture_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.max_texture_size.width as i32),
            height: DevicePixels(self.max_texture_size.height as i32),
        }
    }

    fn read_texture(&mut self, texture_id: OffscreenTextureId) -> anyhow::Result<TextureData> {
        // Delegate to the inherent method
        BladeOffscreenRenderer::read_texture(self, texture_id)
    }

    fn destroy(&mut self) {
        // Delegate to the inherent method
        BladeOffscreenRenderer::destroy(self);
    }
}

// Safety: BladeOffscreenRenderer can be sent between threads because:
// - Arc<gpu::Context> is Send + Sync
// - Arc<BladePipelines> is Send + Sync
// - Arc<BladeAtlas> is Send + Sync
// - gpu::Sampler is Copy and thread-safe
// - Other fields are owned and don't have thread-local state
unsafe impl Send for BladeOffscreenRenderer {}
