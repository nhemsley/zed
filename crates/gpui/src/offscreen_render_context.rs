//! Offscreen rendering context for building scenes without a Window.
//!
//! This module provides `OffscreenRenderContext`, a minimal rendering context
//! that can build GPUI scenes for render-to-texture operations without requiring
//! the full `Window` infrastructure.
//!
//! # Feature Flag
//!
//! This module is only available when the `render-to-texture` feature is enabled.
//!
//! # Usage
//!
//! ```ignore
//! // Create render context with shared resources from window
//! let mut ctx = OffscreenRenderContext::new(
//!     size(px(256.0), px(256.0)),
//!     window.scale_factor(),
//!     window.text_system().clone(),
//!     window.sprite_atlas().clone(),
//! );
//!
//! // Paint primitives
//! ctx.paint_quad(PaintQuad {
//!     bounds: Bounds::new(point(px(10.0), px(10.0)), size(px(100.0), px(50.0))),
//!     background: blue().into(),
//!     ..Default::default()
//! });
//!
//! // Get scene and render to texture
//! let scene = ctx.take_scene();
//! offscreen_renderer.draw_scene_to_texture(&scene, texture_id);
//! ```

use std::borrow::Cow;
use std::sync::Arc;

use crate::geometry::IsZero;
use crate::scene::Scene;
use crate::{
    Background, Bounds, ContentMask, FontId, GlyphId, Hsla,
    MonochromeSprite, PaintQuad, Path, Pixels, PlatformAtlas, Point, Quad, RenderGlyphParams,
    SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, Size, TransformationMatrix, Underline,
    UnderlineStyle, WindowTextSystem, px,
};

/// A minimal rendering context for offscreen element rendering.
///
/// This provides the essential infrastructure to render GPUI primitives
/// to a Scene without the full Window overhead. Useful for:
/// - Rendering thumbnails
/// - Caching UI to textures
/// - Infinite canvas tile rendering
///
/// Unlike `Window`, this context does NOT support:
/// - Focus management
/// - Hit testing
/// - Input handlers
/// - Persistent element state
/// - Deferred draws (tooltips, popovers)
pub struct OffscreenRenderContext {
    // Rendering parameters
    scale_factor: f32,
    viewport_size: Size<Pixels>,

    // Content mask stack (for clipping)
    content_mask_stack: Vec<ContentMask<Pixels>>,

    // Opacity stack
    opacity_stack: Vec<f32>,

    // The scene being built
    scene: Scene,

    // Text system (shared)
    text_system: Arc<WindowTextSystem>,

    // Sprite atlas for glyph caching (shared)
    sprite_atlas: Arc<dyn PlatformAtlas>,
}

impl OffscreenRenderContext {
    /// Create a new offscreen render context.
    ///
    /// # Arguments
    ///
    /// * `viewport_size` - Size of the rendering viewport in logical pixels
    /// * `scale_factor` - Display scale factor (e.g., 2.0 for retina)
    /// * `text_system` - Shared text system for shaping and rasterizing text
    /// * `sprite_atlas` - Shared sprite atlas for caching glyphs
    pub fn new(
        viewport_size: Size<Pixels>,
        scale_factor: f32,
        text_system: Arc<WindowTextSystem>,
        sprite_atlas: Arc<dyn PlatformAtlas>,
    ) -> Self {
        let content_mask = ContentMask {
            bounds: Bounds {
                origin: Point::new(px(0.0), px(0.0)),
                size: viewport_size,
            },
        };

        Self {
            scale_factor,
            viewport_size,
            content_mask_stack: vec![content_mask],
            opacity_stack: vec![1.0],
            scene: Scene::default(),
            text_system,
            sprite_atlas,
        }
    }

    /// Returns the scale factor for this context.
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Returns the viewport size in logical pixels.
    pub fn viewport_size(&self) -> Size<Pixels> {
        self.viewport_size
    }

    /// Returns the current content mask (for clipping).
    pub fn content_mask(&self) -> ContentMask<Pixels> {
        self.content_mask_stack
            .last()
            .cloned()
            .unwrap_or_else(|| ContentMask {
                bounds: Bounds {
                    origin: Point::new(px(0.0), px(0.0)),
                    size: self.viewport_size,
                },
            })
    }

    /// Returns the current cumulative opacity.
    pub fn element_opacity(&self) -> f32 {
        self.opacity_stack.iter().product()
    }

    /// Access the text system for text shaping.
    pub fn text_system(&self) -> &Arc<WindowTextSystem> {
        &self.text_system
    }

    /// Access the sprite atlas.
    pub fn sprite_atlas(&self) -> &Arc<dyn PlatformAtlas> {
        &self.sprite_atlas
    }

    /// Push a content mask onto the stack (for clipping).
    pub fn push_content_mask(&mut self, mask: ContentMask<Pixels>) {
        self.content_mask_stack.push(mask);
    }

    /// Pop a content mask from the stack.
    pub fn pop_content_mask(&mut self) {
        self.content_mask_stack.pop();
    }

    /// Push an opacity value onto the stack.
    pub fn push_opacity(&mut self, opacity: f32) {
        self.opacity_stack.push(opacity);
    }

    /// Pop an opacity value from the stack.
    pub fn pop_opacity(&mut self) {
        self.opacity_stack.pop();
    }

    /// Push a layer for z-ordering.
    pub fn push_layer(&mut self, bounds: Bounds<Pixels>) {
        self.scene.push_layer(bounds.scale(self.scale_factor));
    }

    /// Pop a layer.
    pub fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    /// Paint a quad (rectangle with background, border, etc.)
    pub fn paint_quad(&mut self, quad: PaintQuad) {
        let scale_factor = self.scale_factor();
        let content_mask = self.content_mask();
        let opacity = self.element_opacity();

        self.scene.insert_primitive(Quad {
            order: 0,
            bounds: quad.bounds.scale(scale_factor),
            content_mask: content_mask.scale(scale_factor),
            background: quad.background.opacity(opacity),
            border_color: quad.border_color.opacity(opacity),
            corner_radii: quad.corner_radii.scale(scale_factor),
            border_widths: quad.border_widths.scale(scale_factor),
            border_style: quad.border_style,
        });
    }

    /// Paint a path (vector shape).
    pub fn paint_path(&mut self, mut path: Path<Pixels>, color: impl Into<Background>) {
        let scale_factor = self.scale_factor();
        let content_mask = self.content_mask();
        let opacity = self.element_opacity();

        path.content_mask = content_mask;
        let color: Background = color.into();
        path.color = color.opacity(opacity);

        self.scene.insert_primitive(path.scale(scale_factor));
    }

    /// Paint an underline.
    pub fn paint_underline(
        &mut self,
        origin: Point<Pixels>,
        width: Pixels,
        style: &UnderlineStyle,
    ) {
        let scale_factor = self.scale_factor();
        let content_mask = self.content_mask();
        let opacity = self.element_opacity();

        let height = if style.wavy {
            style.thickness * 3.0
        } else {
            style.thickness
        };
        let bounds = Bounds {
            origin,
            size: Size { width, height },
        };

        self.scene.insert_primitive(Underline {
            order: 0,
            pad: 0,
            bounds: bounds.scale(scale_factor),
            content_mask: content_mask.scale(scale_factor),
            color: style.color.unwrap_or_default().opacity(opacity),
            thickness: style.thickness.scale(scale_factor),
            wavy: style.wavy as u32,
        });
    }

    /// Paint a text glyph.
    ///
    /// The y component of the origin is the baseline of the glyph.
    pub fn paint_glyph(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
    ) -> anyhow::Result<()> {
        let scale_factor = self.scale_factor();
        let glyph_origin = origin.scale(scale_factor);

        let subpixel_variant = Point {
            x: (glyph_origin.x.0.fract() * SUBPIXEL_VARIANTS_X as f32).floor() as u8,
            y: (glyph_origin.y.0.fract() * SUBPIXEL_VARIANTS_Y as f32).floor() as u8,
        };

        let params = RenderGlyphParams {
            font_id,
            glyph_id,
            font_size,
            subpixel_variant,
            scale_factor,
            is_emoji: false,
        };

        let raster_bounds = self.text_system.raster_bounds(&params)?;
        if !raster_bounds.is_zero() {
            let tile = self
                .sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let (size, bytes) = self.text_system.rasterize_glyph(&params)?;
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
                .expect("Callback above only errors or returns Some");

            let bounds = Bounds {
                origin: glyph_origin.map(|px| px.floor()) + raster_bounds.origin.map(Into::into),
                size: tile.bounds.size.map(Into::into),
            };
            let content_mask = self.content_mask().scale(scale_factor);
            let opacity = self.element_opacity();

            self.scene.insert_primitive(MonochromeSprite {
                order: 0,
                pad: 0,
                bounds,
                content_mask,
                color: color.opacity(opacity),
                tile,
                transformation: TransformationMatrix::unit(),
            });
        }
        Ok(())
    }

    /// Take the built scene, replacing it with an empty scene.
    ///
    /// Use this to get the scene for rendering to a texture.
    pub fn take_scene(&mut self) -> Scene {
        let mut scene = Scene::default();
        std::mem::swap(&mut self.scene, &mut scene);
        scene.finish();
        scene
    }

    /// Get a reference to the current scene.
    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Get a mutable reference to the current scene.
    pub(crate) fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// Clear the scene for reuse.
    pub fn clear(&mut self) {
        self.scene.clear();
        self.content_mask_stack.clear();
        self.opacity_stack.clear();

        // Reset to defaults
        self.content_mask_stack.push(ContentMask {
            bounds: Bounds {
                origin: Point::new(px(0.0), px(0.0)),
                size: self.viewport_size,
            },
        });
        self.opacity_stack.push(1.0);
    }
}
