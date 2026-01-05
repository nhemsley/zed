# OffscreenRenderContext Design

This document outlines the design for `OffscreenRenderContext`, a minimal rendering context that can render GPUI elements to textures without requiring a full `Window` infrastructure.

---

## Goals

1. **Render elements to textures** for use cases like infinite canvas thumbnails
2. **Minimal dependencies** - only what's needed for text rendering and basic primitives
3. **Share GPU resources** with the main window renderer (atlas, pipelines)
4. **No Window overhead** - no focus, hit testing, input handlers, etc.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    OffscreenRenderContext                               │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  Minimal Element Rendering                                       │   │
│  │  - scale_factor: f32                                             │   │
│  │  - viewport_size: Size<Pixels>                                   │   │
│  │  - content_mask: ContentMask<Pixels>                             │   │
│  │  - scene: Scene                                                  │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  Layout & Text (shared/borrowed)                                 │   │
│  │  - layout_engine: TaffyLayoutEngine                              │   │
│  │  - text_system: Arc<WindowTextSystem>                            │   │
│  │  - sprite_atlas: Arc<dyn PlatformAtlas>                          │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│                              ▼                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  Output: Scene                                                   │   │
│  │  - Contains all primitives (Quads, MonochromeSprites, etc.)      │   │
│  │  - Ready to pass to BladeOffscreenRenderer                       │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                    BladeOffscreenRenderer                               │
│  - draw_to_texture(&scene, texture_id)                                  │
│  - Uses shared GPU resources (pipelines, atlas)                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Comparison: Window vs OffscreenRenderContext

| Feature | Window | OffscreenRenderContext |
|---------|--------|------------------------|
| Scale factor | ✅ From display | ✅ Provided at creation |
| Viewport size | ✅ From window | ✅ Provided at creation |
| Content mask | ✅ Stack-based | ✅ Simple (full viewport) |
| Scene | ✅ In Frame | ✅ Owned directly |
| Layout engine | ✅ TaffyLayoutEngine | ✅ Own instance |
| Text system | ✅ WindowTextSystem | ✅ Shared Arc |
| Sprite atlas | ✅ From platform | ✅ Shared Arc |
| Focus handling | ✅ Full | ❌ Not needed |
| Hit testing | ✅ Full | ❌ Not needed |
| Input handlers | ✅ Full | ❌ Not needed |
| Element state | ✅ Persistent | ⚠️ Transient only |
| Deferred draws | ✅ Tooltips, etc. | ❌ Not needed |

---

## Required Components

### 1. Scene (needs to be accessible)

Currently `pub(crate)`. Options:
- Make `Scene` public (simple)
- Keep internal, provide accessor methods

```rust
// Current: gpui/src/scene.rs
pub(crate) struct Scene { ... }

// Option A: Make public
pub struct Scene { ... }

// Option B: Keep internal, expose via OffscreenRenderContext
impl OffscreenRenderContext {
    pub(crate) fn scene(&self) -> &Scene { &self.scene }
}
```

### 2. Text System Access

`WindowTextSystem` wraps `TextSystem` and provides caching:

```rust
// gpui/src/text_system.rs
pub struct WindowTextSystem {
    line_layout_cache: LineLayoutCache,
    text_system: Arc<TextSystem>,
}
```

For offscreen rendering, we can either:
- Share the Window's `WindowTextSystem` (if rendering from window context)
- Create a new one (for standalone rendering)

### 3. Sprite Atlas

The atlas is already `Arc<dyn PlatformAtlas>` and shared:

```rust
// From BladeRenderer
pub fn sprite_atlas(&self) -> &Arc<BladeAtlas> {
    &self.shared.atlas
}
```

This is used for caching rasterized glyphs. The offscreen renderer already has access to it.

### 4. Layout Engine

`TaffyLayoutEngine` is relatively lightweight:

```rust
// gpui/src/taffy.rs
pub(crate) struct TaffyLayoutEngine(taffy::TaffyTree<()>);
```

Each `OffscreenRenderContext` can have its own instance.

---

## Proposed API

### OffscreenRenderContext Structure

```rust
/// A minimal rendering context for offscreen element rendering.
/// 
/// This provides the essential infrastructure to render GPUI elements
/// to a Scene without the full Window overhead.
#[cfg(feature = "render-to-texture")]
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
    
    // Layout engine (owned)
    layout_engine: TaffyLayoutEngine,
    
    // Text system (shared)
    text_system: Arc<WindowTextSystem>,
    
    // Sprite atlas for glyph caching (shared)
    sprite_atlas: Arc<dyn PlatformAtlas>,
}
```

### Creation

```rust
impl OffscreenRenderContext {
    /// Create a new offscreen render context.
    /// 
    /// # Arguments
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
                origin: Point::zero(),
                size: viewport_size,
            },
        };
        
        Self {
            scale_factor,
            viewport_size,
            content_mask_stack: vec![content_mask],
            opacity_stack: vec![1.0],
            scene: Scene::default(),
            layout_engine: TaffyLayoutEngine::new(),
            text_system,
            sprite_atlas,
        }
    }
}
```

### Paint Methods (Simplified from Window)

```rust
impl OffscreenRenderContext {
    /// Current scale factor
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
    
    /// Current content mask (for clipping)
    pub fn content_mask(&self) -> ContentMask<Pixels> {
        self.content_mask_stack.last().cloned().unwrap_or_default()
    }
    
    /// Current opacity
    pub fn element_opacity(&self) -> f32 {
        self.opacity_stack.iter().product()
    }
    
    /// Access the text system
    pub fn text_system(&self) -> &Arc<WindowTextSystem> {
        &self.text_system
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
    
    /// Paint a text glyph
    pub fn paint_glyph(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
    ) -> Result<()> {
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
            let tile = self.sprite_atlas.get_or_insert_with(
                &params.clone().into(),
                &mut || {
                    let (size, bytes) = self.text_system.rasterize_glyph(&params)?;
                    Ok(Some((size, Cow::Owned(bytes))))
                },
            )?.expect("Callback only errors or returns Some");
            
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
    
    /// Get the built scene (consumes self or returns reference)
    pub fn into_scene(self) -> Scene {
        self.scene
    }
    
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    
    /// Clear the scene for reuse
    pub fn clear(&mut self) {
        self.scene.clear();
        self.layout_engine.clear();
    }
}
```

### Element Rendering

For rendering elements, we need a simplified version of the layout/prepaint/paint flow:

```rust
impl OffscreenRenderContext {
    /// Render an element to this context's scene.
    /// 
    /// This performs layout, prepaint, and paint phases.
    pub fn render_element<E: IntoElement>(
        &mut self,
        element: E,
        available_space: Size<AvailableSpace>,
        cx: &mut App,
    ) {
        let mut element = element.into_any_element();
        
        // Layout phase
        let layout_id = element.request_layout(self, cx);
        self.layout_engine.compute_layout(layout_id, available_space);
        
        // Prepaint phase  
        let bounds = self.layout_engine.layout_bounds(layout_id);
        element.prepaint(bounds.origin, self, cx);
        
        // Paint phase
        element.paint(self, cx);
    }
}
```

**Note:** This requires elements to accept a trait or generic context rather than specifically `&mut Window`. This is a larger refactor.

---

## Implementation Strategy

### Phase 1: Scene Accessibility

1. Make `Scene` public (or provide controlled access)
2. Ensure `Scene::insert_primitive()` is accessible
3. Test that `BladeOffscreenRenderer::draw_to_texture()` works with a manually-built Scene

### Phase 2: OffscreenRenderContext Basics

1. Create `OffscreenRenderContext` struct with:
   - Basic paint methods (`paint_quad`, `paint_glyph`)
   - Text system integration
   - Atlas sharing

2. Test rendering simple primitives:
   ```rust
   let mut ctx = OffscreenRenderContext::new(...);
   ctx.paint_quad(PaintQuad { ... });
   let scene = ctx.into_scene();
   offscreen_renderer.draw_to_texture(&scene, texture_id);
   ```

### Phase 3: Element Integration (Future)

Options for making elements renderable to `OffscreenRenderContext`:

**Option A: Trait Abstraction**
```rust
trait RenderContext {
    fn scale_factor(&self) -> f32;
    fn paint_quad(&mut self, quad: PaintQuad);
    fn paint_glyph(&mut self, ...);
    // etc.
}

impl RenderContext for Window { ... }
impl RenderContext for OffscreenRenderContext { ... }
```

**Option B: Enum Dispatch**
```rust
enum AnyRenderContext<'a> {
    Window(&'a mut Window),
    Offscreen(&'a mut OffscreenRenderContext),
}
```

**Option C: Keep Separate**
- Elements paint to Window only
- Manual Scene building for offscreen
- Higher-level helpers for common cases

---

## Usage Example

```rust
// Create offscreen renderer from window
let mut offscreen_renderer = window.create_offscreen_renderer(
    size(DevicePixels(1024), DevicePixels(1024))
)?;

// Create texture
let texture_info = offscreen_renderer.create_texture(
    size(DevicePixels(256), DevicePixels(256))
);

// Create render context (borrowing shared resources)
let mut render_ctx = OffscreenRenderContext::new(
    size(px(256.0), px(256.0)),
    window.scale_factor(),
    window.text_system().clone(),
    window.sprite_atlas().clone(),
);

// Paint primitives
render_ctx.paint_quad(PaintQuad {
    bounds: Bounds::new(point(px(10.0), px(10.0)), size(px(100.0), px(50.0))),
    background: blue().into(),
    ..Default::default()
});

// Render text
let line = render_ctx.text_system().shape_line("Hello", px(16.0), &runs, None);
line.paint(&mut render_ctx, point(px(20.0), px(30.0)), px(16.0))?;

// Get scene and render to texture
let scene = render_ctx.into_scene();
offscreen_renderer.draw_scene_to_texture(&scene, texture_info.id);

// Use texture_info.id in your canvas...
```

---

## Dependencies Summary

| Component | Source | Sharing |
|-----------|--------|---------|
| `Scene` | Created fresh | Owned |
| `TaffyLayoutEngine` | Created fresh | Owned |
| `WindowTextSystem` | From Window or new | `Arc` shared |
| `PlatformAtlas` | From BladeRenderer | `Arc` shared |
| GPU pipelines | From BladeRenderer | `Arc` shared (via offscreen renderer) |

---

## Open Questions

1. **Element State**: Should `OffscreenRenderContext` support persistent element state? For thumbnails, probably not needed.

2. **ShapedLine Painting**: `ShapedLine::paint()` currently takes `&mut Window`. Need to either:
   - Add overload taking `&mut OffscreenRenderContext`
   - Create trait abstraction
   - Manually iterate glyphs

3. **Text Style Stack**: Window maintains `text_style_stack`. For simple thumbnails, can use explicit styles.

4. **Content Mask Stack**: Needed for clipping. Include in OffscreenRenderContext.

5. **Where to Put It**: 
   - `gpui/src/offscreen_render_context.rs`?
   - `gpui/src/platform/offscreen_render_context.rs`?
   - Feature-gated under `render-to-texture`

---

## Next Steps

1. [ ] Make `Scene` accessible for offscreen rendering
2. [ ] Implement basic `OffscreenRenderContext` with `paint_quad`
3. [ ] Add text rendering support (`paint_glyph`)
4. [ ] Create integration test rendering to texture
5. [ ] Document API and add to example
6. [ ] (Future) Consider element abstraction for direct element rendering