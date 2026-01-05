# Render-to-Texture Methodology

## Goal

Enable rendering of GPUI Elements directly to GPU textures, going through the full layout → prepaint → paint pipeline. This is useful for:

- Caching canvas items as textures for efficient zoom/pan
- Generating thumbnails
- Offscreen rendering for image export
- Any scenario where you want GPUI rendering to a texture instead of a window

## Design Principles

1. **Minimize changes to GPUI** - Work with existing structures, don't fork
2. **Reuse existing rendering code** - BladeRenderer already does 95% of what we need
3. **Use the full Element pipeline** - Layout → Prepaint → Paint for proper rendering
4. **Share GPU resources** - Use the same atlas, pipelines, and GPU context as the main renderer

---

## Current Architecture

### Rendering Pipeline Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           GPUI Layer                                    │
│  ┌─────────────────┐    ┌─────────────┐    ┌─────────────┐             │
│  │     Element     │───▶│   Window    │───▶│    Frame    │             │
│  │ request_layout()│    │   .draw()   │    │   .scene    │             │
│  │   prepaint()    │    │ draw_roots()│    │             │             │
│  │    paint()      │    │  present()  │    │             │             │
│  └─────────────────┘    └─────────────┘    └─────────────┘             │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        Platform Layer                                   │
│  ┌──────────────────────┐                                               │
│  │   PlatformWindow     │◀─── trait with fn draw(&self, scene: &Scene) │
│  │   (trait)            │                                               │
│  └──────────────────────┘                                               │
│            │                                                            │
│            ├── WaylandWindow ──▶ BladeRenderer::draw()                 │
│            ├── X11Window ──────▶ BladeRenderer::draw()                 │
│            └── MacWindow ──────▶ MetalRenderer (similar)               │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          Blade Layer                                    │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                      BladeRenderer                               │   │
│  │  - gpu: Arc<gpu::Context>                                        │   │
│  │  - surface: gpu::Surface (window surface)                        │   │
│  │  - pipelines: BladePipelines                                     │   │
│  │  - atlas: Arc<BladeAtlas>                                        │   │
│  │  - command_encoder: gpu::CommandEncoder                          │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
```

### Element Lifecycle

The `Element` trait defines three phases:

```rust
pub trait Element: 'static + IntoElement {
    type RequestLayoutState: 'static;
    type PrepaintState: 'static;

    // Phase 1: Request layout from Taffy, returns LayoutId
    fn request_layout(&mut self, ...) -> (LayoutId, Self::RequestLayoutState);
    
    // Phase 2: After layout, commit bounds for hitboxes
    fn prepaint(&mut self, bounds: Bounds<Pixels>, ...) -> Self::PrepaintState;
    
    // Phase 3: Actually paint to the Scene
    fn paint(&mut self, bounds: Bounds<Pixels>, ...);
}
```

### Window::draw_roots() Flow (L2167-2240)

```rust
fn draw_roots(&mut self, cx: &mut App) {
    self.invalidator.set_phase(DrawPhase::Prepaint);
    
    // 1. Layout and prepaint root element
    let mut root_element = self.root.as_ref().unwrap().clone().into_any();
    root_element.prepaint_as_root(Point::default(), root_size.into(), self, cx);
    
    // 2. Handle deferred draws (tooltips, etc.)
    self.prepaint_deferred_draws(&sorted_deferred_draws, cx);
    
    // 3. Paint phase
    self.invalidator.set_phase(DrawPhase::Paint);
    root_element.paint(self, cx);
    self.paint_deferred_draws(&sorted_deferred_draws, cx);
}
```

### AnyElement::prepaint_as_root() (L639-651)

```rust
pub fn prepaint_as_root(
    &mut self,
    origin: Point<Pixels>,
    available_space: Size<AvailableSpace>,
    window: &mut Window,
    cx: &mut App,
) -> Option<FocusHandle> {
    // First: perform layout
    self.layout_as_root(available_space, window, cx);
    // Then: prepaint at the given origin
    window.with_absolute_element_offset(origin, |window| self.prepaint(window, cx))
}
```

### BladeRenderer::draw() (L644-919)

The current method implicitly uses `self.surface`:

```rust
pub fn draw(&mut self, scene: &Scene) {
    // 1. Acquire frame from window surface
    let frame = self.surface.acquire_frame();
    
    // 2. Get viewport size from surface config
    let viewport_size = [
        self.surface_config.size.width as f32,
        self.surface_config.size.height as f32,
    ];
    
    // 3. Get alpha mode from surface
    let premultiplied_alpha = match self.surface.info().alpha { ... };
    
    // 4. Render to frame.texture_view()
    let mut pass = self.command_encoder.render(..., frame.texture_view(), ...);
    
    // 5. Process all batches (quads, shadows, paths, sprites, etc.)
    for batch in scene.batches() { ... }
    
    // 6. Present to window
    self.command_encoder.present(frame);
}
```

### Surface-Specific Code Points in BladeRenderer

| Line | Code | Purpose |
|------|------|---------|
| L650-652 | `self.surface.acquire_frame()` | Get window frame |
| L656-657 | `self.surface_config.size` | Viewport dimensions |
| L658 | `self.surface.info().alpha` | Alpha blending mode |
| L670 | `frame.texture_view()` | Render target |
| L718 | `frame.texture_view()` | Resume after paths |
| L910 | `present(frame)` | Display on window |

**Key insight:** The batch processing loop (L679-907) is completely target-agnostic. It just needs a texture view and viewport size.

---

## Key Data Structures

### Window (L833-894)

```rust
pub struct Window {
    pub(crate) platform_window: Box<dyn PlatformWindow>,
    sprite_atlas: Arc<dyn PlatformAtlas>,
    text_system: Arc<WindowTextSystem>,
    layout_engine: Option<TaffyLayoutEngine>,
    
    // Rendering context
    scale_factor: f32,
    pub(crate) content_mask_stack: Vec<ContentMask<Pixels>>,
    pub(crate) element_opacity: f32,
    pub(crate) element_offset_stack: Vec<Point<Pixels>>,
    
    // Frames
    pub(crate) rendered_frame: Frame,
    pub(crate) next_frame: Frame,
    
    // ... other fields
}
```

### Frame (L675-696)

```rust
pub(crate) struct Frame {
    pub(crate) scene: Scene,           // <-- The primitives go here
    pub(crate) hitboxes: Vec<Hitbox>,
    pub(crate) dispatch_tree: DispatchTree,
    pub(crate) mouse_listeners: Vec<Option<AnyMouseListener>>,
    // ... other frame state
}
```

### Scene (L24-35)

```rust
pub(crate) struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub(crate) shadows: Vec<Shadow>,
    pub(crate) quads: Vec<Quad>,
    pub(crate) paths: Vec<Path<ScaledPixels>>,
    pub(crate) underlines: Vec<Underline>,
    pub(crate) monochrome_sprites: Vec<MonochromeSprite>,
    pub(crate) polychrome_sprites: Vec<PolychromeSprite>,
    pub(crate) surfaces: Vec<PaintSurface>,
}
```

### Paint Methods on Window

| Method | Lines | What It Does |
|--------|-------|--------------|
| `paint_quad()` | L2997-3013 | Adds a `Quad` primitive |
| `paint_path()` | L3018-3030 | Adds a `Path` primitive |
| `paint_shadows()` | L2964-2986 | Adds `Shadow` primitives |
| `paint_glyph()` | L3106-3158 | Adds a `MonochromeSprite` (text) |
| `paint_image()` | L3289-3335 | Adds a `PolychromeSprite` (image) |

All paint methods require:
- `self.scale_factor()` - display scale
- `self.content_mask()` - clipping bounds  
- `self.element_opacity()` - opacity stack
- `self.next_frame.scene` - the Scene to add to

---

## Proposed Changes

### Phase 1: Refactor BladeRenderer (Low Risk)

#### 1.1 Extract `draw_to_target()` as Private Method

```rust
impl BladeRenderer {
    /// Render a scene to a specific target (internal implementation)
    fn draw_to_target(
        &mut self,
        scene: &Scene,
        target_view: gpu::TextureView,
        viewport_size: [f32; 2],
        premultiplied_alpha: bool,
    ) -> gpu::SyncPoint {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);

        let globals = GlobalParams {
            viewport_size,
            premultiplied_alpha: if premultiplied_alpha { 1 } else { 0 },
            pad: 0,
        };

        let mut pass = self.command_encoder.render(
            "main",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: target_view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );

        // Existing batch processing loop (unchanged)
        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Quads(quads) => { /* unchanged */ }
                PrimitiveBatch::Shadows(shadows) => { /* unchanged */ }
                PrimitiveBatch::Paths(paths) => {
                    drop(pass);
                    self.draw_paths_to_intermediate(paths, viewport_size[0], viewport_size[1]);
                    // Resume with saved target_view
                    pass = self.command_encoder.render(
                        "main",
                        gpu::RenderTargetSet {
                            colors: &[gpu::RenderTarget {
                                view: target_view,  // Use passed-in view
                                init_op: gpu::InitOp::Load,
                                finish_op: gpu::FinishOp::Store,
                            }],
                            depth_stencil: None,
                        },
                    );
                    // ... rest of paths handling
                }
                // ... other batches unchanged ...
            }
        }
        
        drop(pass);
        
        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);
        
        sync_point
    }
}
```

#### 1.2 Refactor `draw()` to Use `draw_to_target()`

```rust
impl BladeRenderer {
    /// Render to window surface (existing behavior, refactored)
    pub fn draw(&mut self, scene: &Scene) {
        let frame = {
            profiling::scope!("acquire frame");
            self.surface.acquire_frame()
        };
        self.command_encoder.start();
        self.command_encoder.init_texture(frame.texture());
        
        let viewport_size = [
            self.surface_config.size.width as f32,
            self.surface_config.size.height as f32,
        ];
        let premultiplied_alpha = matches!(
            self.surface.info().alpha,
            gpu::AlphaMode::PreMultiplied
        );
        
        let sync_point = self.draw_to_target(
            scene,
            frame.texture_view(),
            viewport_size,
            premultiplied_alpha,
        );
        
        self.command_encoder.start();
        self.command_encoder.present(frame);
        self.gpu.submit(&mut self.command_encoder);
        
        self.wait_for_gpu();
        self.last_sync_point = Some(sync_point);
    }
}
```

#### 1.3 Add `draw_to_texture()` Public Method

```rust
impl BladeRenderer {
    /// Render to an offscreen texture (NEW)
    pub fn draw_to_texture(
        &mut self,
        scene: &Scene,
        texture_view: gpu::TextureView,
        size: Size<DevicePixels>,
    ) -> gpu::SyncPoint {
        let viewport_size = [size.width.0 as f32, size.height.0 as f32];
        
        self.draw_to_target(
            scene,
            texture_view,
            viewport_size,
            true, // Offscreen textures typically use premultiplied alpha
        )
    }
}
```

#### 1.4 Add Texture Creation Helper

```rust
impl BladeRenderer {
    /// Create an offscreen render target texture
    pub fn create_render_texture(
        &self, 
        size: Size<DevicePixels>
    ) -> (gpu::Texture, gpu::TextureView) {
        let texture = self.gpu.create_texture(gpu::TextureDesc {
            name: "offscreen_render_target",
            format: gpu::TextureFormat::Bgra8UnormSrgb,
            size: gpu::Extent {
                width: size.width.0 as u32,
                height: size.height.0 as u32,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::TARGET | gpu::TextureUsage::COPY,
            external: None,
        });
        
        let view = self.gpu.create_texture_view(
            texture,
            gpu::TextureViewDesc {
                name: "offscreen_render_target_view",
                format: gpu::TextureFormat::Bgra8UnormSrgb,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );
        
        (texture, view)
    }
    
    /// Get access to the GPU context for advanced usage
    pub fn gpu(&self) -> &Arc<gpu::Context> {
        &self.gpu
    }
}
```

---

### Phase 2: Add Window Methods for Isolated Scene Building

#### 2.1 Add `render_element_to_scene()`

This method renders an element through the full pipeline but to an isolated Scene:

```rust
impl Window {
    /// Render an element tree to a Scene, performing layout, prepaint, and paint.
    /// Returns the scene and the computed size of the element.
    pub fn render_element_to_scene(
        &mut self,
        element: impl IntoElement,
        available_space: Size<AvailableSpace>,
        cx: &mut App,
    ) -> (Scene, Size<Pixels>) {
        // 1. Save current scene state
        let saved_scene = std::mem::take(&mut self.next_frame.scene);
        
        // 2. Save and reset content mask to full coverage
        let saved_content_mask = std::mem::take(&mut self.content_mask_stack);
        
        // 3. Convert to AnyElement
        let mut any_element = element.into_element().into_any();
        
        // 4. Layout the element
        let size = any_element.layout_as_root(available_space, self, cx);
        
        // 5. Set up content mask for the element's bounds
        self.content_mask_stack.push(ContentMask {
            bounds: Bounds {
                origin: Point::default(),
                size,
            },
        });
        
        // 6. Prepaint (at origin 0,0)
        self.invalidator.set_phase(DrawPhase::Prepaint);
        any_element.prepaint_at(Point::default(), self, cx);
        
        // 7. Paint
        self.invalidator.set_phase(DrawPhase::Paint);
        any_element.paint(self, cx);
        
        // 8. Finish the scene (sorts primitives by draw order)
        self.next_frame.scene.finish();
        
        // 9. Extract the scene and restore state
        let scene = std::mem::replace(&mut self.next_frame.scene, saved_scene);
        self.content_mask_stack = saved_content_mask;
        
        (scene, size)
    }
}
```

#### 2.2 Add `render_element_to_texture()` (Higher-Level API)

```rust
impl Window {
    /// Render an element tree to a texture.
    /// 
    /// This performs the full layout → prepaint → paint pipeline
    /// on the given element, then renders the resulting Scene to a texture.
    pub fn render_element_to_texture(
        &mut self,
        element: impl IntoElement,
        available_space: Size<AvailableSpace>,
        cx: &mut App,
    ) -> Result<RenderToTextureResult> {
        // 1. Build the scene
        let (scene, size) = self.render_element_to_scene(element, available_space, cx);
        
        // 2. Calculate device pixel size
        let device_size = Size {
            width: DevicePixels((size.width.0 * self.scale_factor) as i32),
            height: DevicePixels((size.height.0 * self.scale_factor) as i32),
        };
        
        // 3. Render to texture via platform window
        let texture_id = self.platform_window.render_scene_to_texture(&scene, device_size)?;
        
        Ok(RenderToTextureResult {
            texture_id,
            size,
            device_size,
        })
    }
}

pub struct RenderToTextureResult {
    pub texture_id: TextureId,
    pub size: Size<Pixels>,
    pub device_size: Size<DevicePixels>,
}
```

---

### Phase 3: Expose Renderer Through PlatformWindow

#### 3.1 Add to `PlatformWindow` Trait

```rust
// In platform.rs
pub(crate) trait PlatformWindow: HasWindowHandle + HasDisplayHandle {
    // ... existing methods ...
    
    /// Render a scene to a new texture, returning a texture ID
    fn render_scene_to_texture(
        &self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<TextureId>;
    
    /// Optional: Direct renderer access for advanced usage
    fn with_renderer<R>(&self, f: impl FnOnce(&mut dyn Renderer) -> R) -> Option<R> {
        None
    }
}
```

#### 3.2 Implement for Wayland/X11

```rust
// In platform/linux/wayland/window.rs
impl PlatformWindow for WaylandWindow {
    fn render_scene_to_texture(
        &self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<TextureId> {
        let mut state = self.0.borrow_mut();
        
        // Create texture
        let (texture, view) = state.renderer.create_render_texture(size);
        
        // Render scene to texture
        let _sync_point = state.renderer.draw_to_texture(scene, view, size);
        
        // Register texture for later use (e.g., in atlas or texture registry)
        let texture_id = state.register_render_texture(texture, view, size);
        
        Ok(texture_id)
    }
}
```

---

### Phase 4: Texture Compositing

After rendering to a texture, we need to draw it back in the main scene.

#### Option A: Use Existing PolychromeSprite

Register the texture in the atlas and use as a `PolychromeSprite`:

```rust
// Add to atlas with a custom key type
pub enum AtlasKey {
    Glyph(RenderGlyphParams),
    Svg(RenderSvgParams),
    Image(RenderImageParams),
    RenderTexture(TextureId),  // NEW
}
```

#### Option B: Add TextureQuad Primitive

Create a new primitive type for raw texture rendering:

```rust
pub(crate) struct TextureQuad {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub texture_view: gpu::TextureView,
    pub corner_radii: Corners<ScaledPixels>,
    pub opacity: f32,
}
```

This would require:
1. Adding to `Primitive` enum
2. Adding to `PrimitiveBatch` enum
3. Adding rendering logic in `BladeRenderer::draw()`
4. Adding corresponding shader code

#### Option C: Paint Helper Method

Add a convenience method to Window:

```rust
impl Window {
    pub fn paint_rendered_texture(
        &mut self,
        texture_id: TextureId,
        bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
    ) -> Result<()> {
        // Use the texture as a PolychromeSprite
        let tile = self.sprite_atlas.get_render_texture(texture_id)?;
        
        self.next_frame.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false,
            opacity: self.element_opacity(),
            bounds: bounds.scale(self.scale_factor()),
            content_mask: self.content_mask().scale(self.scale_factor()),
            corner_radii: corner_radii.scale(self.scale_factor()),
            tile,
        });
        
        Ok(())
    }
}
```

---

## Usage Pattern for Infinite Canvas

```rust
impl InfiniteCanvas {
    fn render_item_to_texture(
        &mut self,
        item: &CanvasItem,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<CachedTexture> {
        // 1. Build the element for this item
        let element = item.render(cx);
        
        // 2. Render to texture through the full pipeline
        let result = window.render_element_to_texture(
            element,
            item.available_space(),
            cx,
        )?;
        
        Ok(CachedTexture {
            texture_id: result.texture_id,
            size: result.size,
            device_size: result.device_size,
        })
    }
    
    fn paint_cached_item(
        &self,
        cached: &CachedTexture,
        transform: TransformationMatrix,
        window: &mut Window,
    ) -> Result<()> {
        // Calculate transformed bounds
        let bounds = transform.apply(Bounds {
            origin: Point::default(),
            size: cached.size,
        });
        
        // Paint the cached texture
        window.paint_rendered_texture(
            cached.texture_id,
            bounds,
            Corners::default(),
        )
    }
}
```

---

## Challenges and Solutions

### Challenge 1: Accessing the Renderer

The `BladeRenderer` is owned by the platform window implementation (e.g., `WaylandWindowState`), not directly accessible from `Window`.

**Solution:** Add `render_scene_to_texture()` method to `PlatformWindow` trait, delegating to the internal renderer.

### Challenge 2: Building a Scene Without Full Window Context

The `paint_*` methods on `Window` add to `self.next_frame.scene` and use window state.

**Solution:** The `render_element_to_scene()` method temporarily swaps out the scene, allowing isolated rendering while reusing all the Window infrastructure.

### Challenge 3: Text and Images Require Atlas

Glyphs and images are stored in the `BladeAtlas`. The atlas is shared across frames.

**Solution:** Since we're rendering through Window, the atlas is already available. The `render_element_to_scene()` method uses the same atlas as normal rendering.

### Challenge 4: Element State Isolation

Elements may have state (hover, focus, etc.) that depends on the window context.

**Solution:** For render-to-texture, we're rendering a "snapshot" - element state is based on whatever is current when rendering. For canvas items, this is typically the "rest" state.

### Challenge 5: Deferred Draws

Tooltips, menus, etc. use deferred draws that may not make sense for texture rendering.

**Solution:** The `render_element_to_scene()` method doesn't process deferred draws - those are window-level concepts.

### Challenge 6: Content Mask Handling

The content mask determines clipping bounds.

**Solution:** Set up a content mask matching the element's bounds before painting, then restore the original.

---

## Implementation Plan

### Phase 1: Refactor BladeRenderer (Low Risk)

1. Extract `draw_to_target()` as private method
2. Reimplement `draw()` to call `draw_to_target()`
3. **Test:** Verify all existing rendering still works
4. Add `draw_to_texture()` public method
5. Add `create_render_texture()` helper

### Phase 2: Add Window Scene Building Methods

1. Add `render_element_to_scene()` method
2. **Test:** Verify elements render correctly to isolated scenes
3. Test with simple shapes (quads, paths)
4. Test with text (requires atlas)
5. Test with images (requires atlas)

### Phase 3: Expose Through PlatformWindow

1. Add `render_scene_to_texture()` to `PlatformWindow` trait
2. Implement for Wayland
3. Implement for X11
4. Add `Window::render_element_to_texture()` high-level API
5. **Test:** End-to-end texture rendering

### Phase 4: Texture Compositing

1. Choose approach (PolychromeSprite vs TextureQuad)
2. Implement texture registration in atlas
3. Add `Window::paint_rendered_texture()` helper
4. **Test:** Round-trip rendering (element → texture → display)

### Phase 5: Integration

1. Integrate with infinite canvas
2. Add texture caching/invalidation logic
3. Performance optimization
4. **Test:** Canvas with many cached items

---

## File Changes Summary

| File | Changes |
|------|---------|
| `platform/blade/blade_renderer.rs` | Add `draw_to_target()`, `draw_to_texture()`, `create_render_texture()`, `gpu()` |
| `platform.rs` | Add `render_scene_to_texture()` to `PlatformWindow` trait |
| `platform/linux/wayland/window.rs` | Implement `render_scene_to_texture()` |
| `platform/linux/x11/window.rs` | Implement `render_scene_to_texture()` |
| `window.rs` | Add `render_element_to_scene()`, `render_element_to_texture()`, `paint_rendered_texture()` |
| `scene.rs` | Possibly add `TextureQuad` primitive (Phase 4, Option B) |
| `platform/blade/blade_atlas.rs` | Add render texture registration (Phase 4) |

---

## Testing Strategy

### Unit Tests

1. **BladeRenderer refactor:** Ensure `draw()` produces identical output after refactor
2. **Scene building:** Verify `render_element_to_scene()` produces valid scenes
3. **Texture creation:** Verify textures are created with correct format and size

### Integration Tests

1. **Simple elements:** Render div, text, images to texture
2. **Complex elements:** Nested layouts, transforms
3. **Round-trip:** Render to texture, then display texture

### Visual Tests

1. **Comparison:** Rendered texture should match direct rendering
2. **Scaling:** Test at different scale factors
3. **Clipping:** Verify content masks work correctly

---

## References

- `gpui/src/platform/blade/blade_renderer.rs` - Main renderer (L644-919)
- `gpui/src/window.rs` - Window struct (L833-894), draw methods (L2066-2240), paint methods (L2964-3370)
- `gpui/src/scene.rs` - Scene and primitives (L24-35, L435-449)
- `gpui/src/element.rs` - Element trait (L51-100), AnyElement (L572-651)
- `gpui/src/platform.rs` - PlatformWindow trait (L480-590), PlatformAtlas (L832-839)