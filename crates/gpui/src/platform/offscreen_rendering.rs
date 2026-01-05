//! Offscreen rendering support for render-to-texture operations.
//!
//! This module provides the platform-agnostic types and traits for rendering
//! GPUI scenes to GPU textures without displaying them on screen.
//!
//! # Feature Flag
//!
//! This module is only available when the `render-to-texture` feature is enabled.

use crate::{DevicePixels, Scene, Size};

/// A unique identifier for an offscreen texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OffscreenTextureId(pub u64);

impl OffscreenTextureId {
    /// Creates a new texture ID from a raw value.
    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw ID value.
    pub fn as_raw(&self) -> u64 {
        self.0
    }
}

/// Information about an offscreen texture.
#[derive(Debug, Clone)]
pub struct OffscreenTextureInfo {
    /// The unique identifier for this texture.
    pub id: OffscreenTextureId,
    /// The size of the texture in device pixels.
    pub size: Size<DevicePixels>,
}

/// Raw texture data read back from the GPU.
///
/// Contains RGBA pixel data that can be used for saving to files,
/// further processing, or any other purpose.
#[derive(Debug, Clone)]
pub struct TextureData {
    /// RGBA pixel data, row-major order, 4 bytes per pixel.
    /// The data is in standard RGBA format (not premultiplied).
    pub data: Vec<u8>,
    /// Width of the texture in pixels.
    pub width: u32,
    /// Height of the texture in pixels.
    pub height: u32,
}

impl TextureData {
    /// Creates a new TextureData from raw components.
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        debug_assert_eq!(
            data.len(),
            (width * height * 4) as usize,
            "Data length must match width * height * 4 bytes"
        );
        Self {
            data,
            width,
            height,
        }
    }

    /// Returns the size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns the raw RGBA data as a slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consumes self and returns the raw RGBA data.
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Returns the pixel at the given coordinates as [R, G, B, A].
    ///
    /// # Panics
    ///
    /// Panics if the coordinates are out of bounds.
    pub fn pixel_at(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "Pixel coordinates out of bounds");
        let idx = ((y * self.width + x) * 4) as usize;
        [
            self.data[idx],
            self.data[idx + 1],
            self.data[idx + 2],
            self.data[idx + 3],
        ]
    }
}

/// A platform-agnostic trait for offscreen rendering to textures.
///
/// This trait enables rendering GPUI scenes to GPU textures without displaying
/// them on screen. Implementations are provided by platform-specific backends.
pub(crate) trait PlatformOffscreenRenderer: Send {
    /// Creates a new texture that can be rendered to.
    ///
    /// # Arguments
    ///
    /// * `size` - The size of the texture in device pixels
    ///
    /// # Returns
    ///
    /// Information about the created texture, including its ID.
    fn create_texture(&mut self, size: Size<DevicePixels>) -> OffscreenTextureInfo;

    /// Renders a scene to the specified texture.
    ///
    /// # Arguments
    ///
    /// * `scene` - The scene to render
    /// * `texture_id` - The ID of the texture to render to
    fn draw_to_texture(&mut self, scene: &Scene, texture_id: OffscreenTextureId);

    /// Reads a texture's contents back from the GPU.
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
    /// let texture_data = renderer.read_texture(texture_id)?;
    /// // Use the image crate to save to PNG:
    /// image::save_buffer(
    ///     "output.png",
    ///     &texture_data.data,
    ///     texture_data.width,
    ///     texture_data.height,
    ///     image::ColorType::Rgba8,
    /// )?;
    /// ```
    fn read_texture(&mut self, texture_id: OffscreenTextureId) -> anyhow::Result<TextureData>;

    /// Destroys a texture, freeing its GPU resources.
    ///
    /// # Arguments
    ///
    /// * `texture_id` - The ID of the texture to destroy
    fn destroy_texture(&mut self, texture_id: OffscreenTextureId);

    /// Returns the maximum texture size supported by this renderer.
    fn max_texture_size(&self) -> Size<DevicePixels>;

    /// Destroys the offscreen renderer and releases all its resources.
    fn destroy(&mut self);
}
