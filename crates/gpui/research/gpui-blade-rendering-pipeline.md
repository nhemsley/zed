# GPUI-to-Blade Rendering Pipeline

This document traces the code paths from GPUI's high-level rendering API down to the Blade GPU renderer, identifying what would need to be separated to enable direct render-to-texture without the Window/App context overhead.

---

## Overview: The Rendering Pipeline

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           GPUI Layer                                    │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│  │   Element   │───▶│   Window    │───▶│    Frame    │                 │
│  │  .paint()   │    │  .draw()    │    │   .scene    │                 │
│  └─────────────┘    └─────────────┘    └─────────────┘                 │
│         │                  │                  │                         │
│         ▼                  ▼                  ▼                         │
│  paint_quad()        draw_roots()      Scene::insert_primitive()       │
│  paint_path()        present()         Scene::batches()                │
│  paint_glyph()                                                          │
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
│            ├── MacWindow ──────▶ MetalRenderer (similar)               │
│            └── TexturedSurfaceWindow ──▶ render_scene_to_texture()     │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          Blade Layer                                    │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                      BladeRenderer                               │   │
│  │  - gpu: Arc<gpu::Context>                                        │   │
│  │  - surface: gpu::Surface (for window) OR texture (for offscreen)│   │
│  │  - pipelines: BladePipelines                                     │   │
│  │  - atlas: Arc<BladeAtlas>                                        │   │
│  │  - command_encoder: gpu::CommandEncoder                          │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│                              ▼                                          │
│            BladeRenderer::draw(scene: &Scene)                          │
│                    │                                                    │
│                    ├── Quads ──────▶ quads pipeline                    │
│                    ├── Shadows ────▶ shadows pipeline                  │
│                    ├── Paths ──────▶ path_rasterization + paths        │
│                    ├── Underlines ─▶ underlines pipeline               │
│                    ├── MonoSprites ▶ mono_sprites pipeline (glyphs)    │
│                    ├── PolySprites ▶ poly_sprites pipeline (images)    │
│                    └── Surfaces ───▶ surfaces pipeline (video)         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Layer 1: GPUI Elements and Window

### Element Paint Methods

**File:** `vendor/zed/crates/gpui/src/window.rs`

These methods are called during the paint phase and add primitives to the Scene:

| Method | Lines | What It Does |
|--------|-------|--------------|
| `Window::paint_quad()` | L2995-3013 | Adds a `Quad` primitive (rectangles, backgrounds, borders) |
| `Window::paint_path()` | L3016-3031 | Adds a `Path` primitive (vector shapes) |
| `Window::paint_underline()` | L3033-3066 | Adds an `Underline` primitive |
| `Window::paint_strikethrough()` | L3068-3101 | Adds an `Underline` primitive (reused) |
| `Window::paint_glyph()` | L3104-3164 | Adds a `MonochromeSprite` (text glyph) |
| `Window::paint_emoji()` | L3166-3219 | Adds a `PolychromeSprite` (emoji) |
| `Window::paint_svg()` | L3221-3285 | Adds a `MonochromeSprite` (SVG icon) |
| `Window::paint_image()` | L3287-3337 | Adds a `PolychromeSprite` (image) |
| `Window::paint_surface()` | L3339-3355 | Adds a `PaintSurface` (video frame, macOS only) |
| `Window::paint_layer()` | L2938-2960 | Pushes/pops a layer for z-ordering |
| `Window::paint_shadows()` | L2962-2993 | Adds `Shadow` primitives |

### Example: paint_quad Implementation

```rust
// vendor/zed/crates/gpui/src/window.rs L2995-3013
pub fn paint_quad(&mut self, quad: PaintQuad) {
    self.invalidator.debug_assert_paint();

    let scale_factor = self.scale_factor();
    let content_mask = self.content_mask();
    let opacity = self.element_opacity();
    
    // Adds directly to the Frame's Scene
    self.next_frame.scene.insert_primitive(Quad {
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
```

**Dependencies on Window:**
- `self.scale_factor()` - display scale
- `self.content_mask()` - clipping bounds
- `self.element_opacity()` - opacity stack
- `self.next_frame.scene` - the Scene to add to

---

## Layer 2: Window Draw and Present

**File:** `vendor/zed/crates/gpui/src/window.rs`

### Window::draw() (L2064-2135)

The main draw entry point - called once per frame:

```rust
pub fn draw(&mut self, cx: &mut App) -> ArenaClearNeeded {
    self.invalidate_entities();
    cx.entities.clear_accessed();
    
    // ... input handler management ...
    
    if !cx.mode.skip_drawing() {
        self.draw_roots(cx);  // <-- Layout and paint all elements
    }
    
    // ... frame finishing, focus handling ...
    
    self.needs_present.set(true);
    ArenaClearNeeded
}
```

### Window::present() (L2160-2164)

Actually sends the scene to the GPU:

```rust
fn present(&self) {
    self.platform_window.draw(&self.rendered_frame.scene);  // <-- KEY: Scene goes to platform
    self.needs_present.set(false);
    profiling::finish_frame!();
}
```

### Window::draw_roots() (L2165-2210)

Orchestrates the element tree rendering:

```rust
fn draw_roots(&mut self, cx: &mut App) {
    self.invalidator.set_phase(DrawPhase::Prepaint);
    
    // Layout all root elements
    let mut root_element = self.root.as_ref().unwrap().clone().into_any();
    root_element.prepaint_as_root(Point::default(), root_size.into(), self, cx);
    
    // Handle deferred draws, tooltips, etc.
    self.prepaint_deferred_draws(&sorted_deferred_draws, cx);
    
    self.invalidator.set_phase(DrawPhase::Paint);
    
    // Paint all elements
    root_element.paint(self, cx);
    self.paint_deferred_draws(&sorted_deferred_draws, cx);
}
```

---

## Layer 3: Platform Window Trait

**File:** `vendor/zed/crates/gpui/src/platform.rs` (L488-590)

```rust
pub(crate) trait PlatformWindow: HasWindowHandle + HasDisplayHandle {
    // ... many methods for window management ...
    
    /// THE KEY METHOD: Sends a Scene to the GPU for rendering
    fn draw(&self, scene: &Scene);
    
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas>;
    fn gpu_specs(&self) -> Option<GpuSpecs>;
    // ...
}
```

### Implementations:

| Platform | File | How draw() Works |
|----------|------|------------------|
| Wayland | `platform/linux/wayland/window.rs` L1292-1297 | `state.renderer.draw(scene)` → BladeRenderer |
| X11 | `platform/linux/x11/window.rs` | Same pattern |
| macOS | `platform/mac/window.rs` | MetalRenderer (similar structure) |
| TexturedSurface | `platform/linux/textured_surface/window.rs` L367-373 | `state.render_scene_to_texture(scene)` |

---

## Layer 4: Blade Renderer

**File:** `vendor/zed/crates/gpui/src/platform/blade/blade_renderer.rs`

### BladeRenderer Structure (L328-345)

```rust
pub struct BladeRenderer {
    gpu: Arc<gpu::Context>,
    surface: gpu::Surface,              // Window surface (for presentation)
    surface_config: gpu::SurfaceConfig,
    command_encoder: gpu::CommandEncoder,
    last_sync_point: Option<gpu::SyncPoint>,
    pipelines: BladePipelines,          // All GPU shader pipelines
    instance_belt: BufferBelt,          // GPU buffer allocation
    atlas: Arc<BladeAtlas>,             // Texture atlas for sprites
    atlas_sampler: gpu::Sampler,
    path_intermediate_texture: gpu::Texture,      // For path MSAA
    path_intermediate_texture_view: gpu::TextureView,
    path_intermediate_msaa_texture: Option<gpu::Texture>,
    path_intermediate_msaa_texture_view: Option<gpu::TextureView>,
    rendering_parameters: RenderingParameters,
}
```

### BladeRenderer::draw() (L644-919)

The core GPU rendering method - **this is what we want to reuse**:

```rust
pub fn draw(&mut self, scene: &Scene) {
    self.command_encoder.start();
    self.atlas.before_frame(&mut self.command_encoder);

    // 1. Acquire frame from surface (WINDOW-SPECIFIC)
    let frame = self.surface.acquire_frame();
    self.command_encoder.init_texture(frame.texture());

    let globals = GlobalParams {
        viewport_size: [width, height],
        premultiplied_alpha: /* ... */,
        pad: 0,
    };

    // 2. Begin render pass targeting the frame
    let mut pass = self.command_encoder.render(
        "main",
        gpu::RenderTargetSet {
            colors: &[gpu::RenderTarget {
                view: frame.texture_view(),  // <-- TARGET: window surface
                init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                finish_op: gpu::FinishOp::Store,
            }],
            depth_stencil: None,
        },
    );

    // 3. Process each primitive batch
    for batch in scene.batches() {
        match batch {
            PrimitiveBatch::Quads(quads) => {
                let instance_buf = unsafe { self.instance_belt.alloc_typed(quads, &self.gpu) };
                let mut encoder = pass.with(&self.pipelines.quads);
                encoder.bind(0, &ShaderQuadsData { globals, b_quads: instance_buf });
                encoder.draw(0, 4, 0, quads.len() as u32);
            }
            PrimitiveBatch::Shadows(shadows) => { /* similar */ }
            PrimitiveBatch::Paths(paths) => {
                // Paths require intermediate texture for MSAA
                drop(pass);
                self.draw_paths_to_intermediate(paths, width, height);
                pass = self.command_encoder.render(/* resume main pass */);
                // ... composite paths from intermediate
            }
            PrimitiveBatch::Underlines(underlines) => { /* similar */ }
            PrimitiveBatch::MonochromeSprites { texture_id, sprites } => { /* similar */ }
            PrimitiveBatch::PolychromeSprites { texture_id, sprites } => { /* similar */ }
            PrimitiveBatch::Surfaces(surfaces) => { /* macOS video */ }
        }
    }
    drop(pass);

    // 4. Present to window (WINDOW-SPECIFIC)
    self.command_encoder.present(frame);
    let sync_point = self.gpu.submit(&mut self.command_encoder);
    
    // 5. Cleanup
    self.instance_belt.flush(&sync_point);
    self.atlas.after_frame(&sync_point);
    self.wait_for_gpu();
}
```

---

## Layer 5: Scene and Primitives

**File:** `vendor/zed/crates/gpui/src/scene.rs`

### Scene Structure (L24-35)

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

### PrimitiveBatch Enum (L435-449)

What the renderer iterates over:

```rust
pub(crate) enum PrimitiveBatch<'a> {
    Shadows(&'a [Shadow]),
    Quads(&'a [Quad]),
    Paths(&'a [Path<ScaledPixels>]),
    Underlines(&'a [Underline]),
    MonochromeSprites { texture_id: AtlasTextureId, sprites: &'a [MonochromeSprite] },
    PolychromeSprites { texture_id: AtlasTextureId, sprites: &'a [PolychromeSprite] },
    Surfaces(&'a [PaintSurface]),
}
```

---
