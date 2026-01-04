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
