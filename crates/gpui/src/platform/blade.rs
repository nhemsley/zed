#[cfg(target_os = "macos")]
mod apple_compat;
mod blade_atlas;
mod blade_context;
mod blade_renderer;

use crate::{
    DevicePixels, Size, RenderTargetId, Result
};

#[cfg(target_os = "macos")]
pub(crate) use apple_compat::*;
pub(crate) use blade_atlas::*;
pub(crate) use blade_context::*;
pub(crate) use blade_renderer::*;

/// Read pixels from a render target texture
pub fn read_pixels_from_render_target(
    renderer: &mut BladeRenderer,
    target_id: RenderTargetId,
    size: Size<DevicePixels>
) -> Result<Vec<u8>> {
    renderer.read_pixels_from_render_target(target_id, size)
}
