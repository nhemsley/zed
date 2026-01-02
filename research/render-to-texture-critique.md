# Critique of GPUI Render-to-Texture Implementation

**Commit:** 9ad72c6
**Branch:** nhemsley/gpui-render-to-texture
**Date:** May 2025

## Executive Summary

This commit attempts to add GPUI render-to-texture capability - allowing GPUI views to be rendered to offscreen textures and exported as PNG files. While the low-level GPU infrastructure is partially implemented, the high-level integration with GPUI's rendering pipeline is missing entirely. The current implementation outputs a hardcoded gradient image regardless of what view is provided.

---

## Table of Contents

1. [Goals and Use Cases](#goals-and-use-cases)
2. [Components Added](#components-added)
3. [Critical Problems](#critical-problems)
4. [GPUI Rendering Pipeline Analysis](#gpui-rendering-pipeline-analysis)
5. [One-Shot Rendering Architecture](#one-shot-rendering-architecture)
6. [Embedding GPUI in 3D Space](#embedding-gpui-in-3d-space)
7. [Recommendations](#recommendations)

---

## Goals and Use Cases

### Intended Functionality

The commit aims to enable:

1. **Headless Rendering**: Generate images from GPUI views without displaying windows
2. **Screenshot Export**: Capture UI state to PNG files programmatically
3. **Thumbnail Generation**: Create preview images of UI components
4. **Testing**: Visual regression testing of UI components

### Potential Future Use Cases

1. **3D Embedding**: Render GPUI interfaces as textures in 3D environments
2. **Video Generation**: Render UI animations frame-by-frame
3. **Print/PDF Export**: High-resolution rendering for documents
4. **Remote Display**: Stream rendered UI over network

---

## Components Added

| File | Purpose | Status |
|------|---------|--------|
| `texture_renderer.rs` | High-level API for rendering views to PNG | ❌ Placeholder only |
| `blade_renderer.rs` changes | Low-level GPU render target support | ⚠️ Partial |
| `shaders.wgsl` | New `fs_render_target` fragment shader | ✅ Complete |
| `render_to_texture.rs` | Example demonstrating usage | ❌ Cannot work |

### File Details

#### `crates/gpui/src/texture_renderer.rs`

New module providing `TextureRenderer` struct with:
- `new()` - Creates renderer with Application instance
- `render_to_png()` - Intended to render view to PNG file
- `begin_session()` - Stub for batch rendering sessions
- `WindowRenderExt` trait - Extension for window capture

#### `crates/gpui/src/platform/blade/blade_renderer.rs`

Additions to `BladeRenderer`:
- `render_targets: HashMap<RenderTargetId, RenderTargetTexture>` - Storage for render targets
- `create_render_target()` - Creates GPU texture for offscreen rendering
- `render_to_texture()` - Renders a Scene to specified target
- `render_scene_to_target()` - Internal rendering implementation
- `read_pixels_from_render_target()` - Copies pixel data from GPU to CPU
- `draw_render_target()` - Draws render target texture to screen

#### `crates/gpui/src/platform/blade/shaders.wgsl`

New shader function:
- `fs_render_target` - Fragment shader for sampling render target textures

#### `crates/gpui/src/gpui.rs`

New public types:
- `RenderTargetId` - Identifier for render targets
- `RenderTargetTexture` - Struct holding GPU texture resources
- `RenderTarget` trait - Interface for render target operations

---

## Critical Problems

### Problem 1: TextureRenderer Generates Fake Output

The `render_to_png` method accepts a view builder closure but never uses it:

```rust
// From texture_renderer.rs lines 22-57
pub fn render_to_png<V, F, P>(
    &mut self,
    size: Size<DevicePixels>,
    _build_view: F,  // ← UNUSED! Note the underscore prefix
    output_path: P
) -> Result<()>
where
    V: Render + 'static,
    F: FnOnce(&mut Context<V>) -> V,
    P: AsRef<Path>,
{
    // ...
    // Create a simple gradient pattern (NOT real GPUI rendering!)
    for y in 0..size.height.0 {
        for x in 0..size.width.0 {
            let r = (x as f32 / size.width.0 as f32 * 255.0) as u8;
            let g = (y as f32 / size.height.0 as f32 * 255.0) as u8;
            let b = 128u8;
            let a = 255u8;
            // ...
        }
    }
```

**Impact**: Any view passed to `render_to_png` is completely ignored. The output is always an identical red-green gradient.

### Problem 2: No Connection to GPUI Rendering Pipeline

The `TextureRenderer`:

1. Creates an `Application` but never calls `run()`
2. Never creates a `Window` or equivalent context
3. Has no access to text system, layout engine, or entity system
4. Never generates a `Scene`
5. Never calls the `BladeRenderer` render target methods it's supposedly using

**Impact**: The entire GPUI element system is bypassed.

### Problem 3: BladeRenderer Requires Window Surface

The `BladeRenderer::new()` constructor requires a window:

```rust
// From blade_renderer.rs lines 382-386
pub fn new(context: BladeContext, window: &dyn RawWindow) -> Result<Self> {
    // ...
    let surface = context
        .gpu
        .create_surface_configured(window, surface_config)
```

The render target methods are implemented on `BladeRenderer`, but you cannot create a `BladeRenderer` without a window.

**Impact**: Purely off
screen/headless rendering is impossible with the current architecture.

### Problem 4: Massive Code Duplication

`render_scene_to_target()` duplicates the entire `draw()` rendering loop (~150 lines):

```rust
// From blade_renderer.rs - render_scene_to_target()
for batch in scene.batches() {
    match batch {
        PrimitiveBatch::Quads(quads) => {
            let instance_buf = unsafe { self.instance_belt.alloc_typed(quads, &self.gpu) };
            let mut encoder = pass.with(&self.pipelines.quads);
            // ... identical to draw() ...
        }
        // ... all other batch types duplicated ...
    }
}
```

Compare with nearly identical code in `draw()`:

```rust
// From blade_renderer.rs - draw()
for batch in scene.batches() {
    match batch {
        PrimitiveBatch::Quads(quads) => {
            let instance_buf = unsafe { self.instance_belt.alloc_typed(quads, &self.gpu) };
            let mut encoder = pass.with(&self.pipelines.quads);
            // ... same code ...
        }
        // ...
    }
}
```

**Impact**: Bug fixes or improvements to rendering must be applied twice. Code will inevitably drift out of sync.

### Problem 5: Example Cannot Function

The example creates a view but it's never rendered:

```rust
// From render_to_texture.rs lines 19-25
renderer.render_to_png(size, |cx| {
    // This closure is NEVER CALLED
    ExampleView::new("Hello, render-to-texture!", cx)
}, output_path)?;
```

The closure expects `&mut Context<ExampleView>`, but:
- No `App` is running to provide entity management
- No entity has been created for `ExampleView`
- The closure is simply discarded

**Impact**: The example will compile and run but produce incorrect output.

---

## GPUI Rendering Pipeline Analysis

### Normal Window Rendering Flow

Understanding GPUI's rendering pipeline is essential for fixing this implementation:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application::run()                        │
│                          │                                   │
│                          ▼                                   │
│              ┌───────────────────────┐                      │
│              │   Platform Event Loop  │                      │
│              └───────────┬───────────┘                      │
│                          │                                   │
│                          ▼                                   │
│              ┌───────────────────────┐                      │
│              │    Window::draw()     │                      │
│              └───────────┬───────────┘                      │
│                          │                                   │
│         ┌────────────────┼────────────────┐                 │
│         ▼                ▼                ▼                 │
│   ┌──────────┐    ┌──────────┐    ┌──────────┐            │
│   │ Prepaint │    │  Paint   │    │  Focus   │            │
│   │  Phase   │    │  Phase   │    │  Phase   │            │
│   └────┬─────┘    └────┬─────┘    └──────────┘            │
│        │               │                                    │
│        ▼               ▼                                    │
│   ┌──────────┐    ┌──────────┐                             │
│   │  Layout  │    │  Scene   │                             │
│   │ (Taffy)  │    │ Building │                             │
│   └──────────┘    └────┬─────┘                             │
│                        │                                    │
│                        ▼                                    │
│              ┌───────────────────────┐                      │
│              │   Window::present()   │                      │
│              └───────────┬───────────┘                      │
│                          │                                   │
│                          ▼                                   │
│              ┌───────────────────────┐                      │
│              │ BladeRenderer::draw() │                      │
│              └───────────┬───────────┘                      │
│                          │                                   │
│                          ▼                                   │
│              ┌───────────────────────┐                      │
│              │    GPU Rendering      │                      │
│              │   (to window surface) │                      │
│              └───────────────────────┘                      │
└─────────────────────────────────────────────────────────────┘
```

### Key Components Required for Rendering

| Component | Purpose | Location |
|-----------|---------|----------|
| `App` | Entity management, global state | `app.rs` |
| `Window` | Layout engine, element arena, scene | `window.rs` |
| `TextSystem` | Font loading, glyph rasterization | `text_system.rs` |
| `BladeAtlas` | Texture caching for glyphs/images | `blade_atlas.rs` |
| `LayoutEngine` | Taffy-based flexbox layout | Via `Window` |
| `Scene` | Batched GPU draw commands | `scene.rs` |
| `BladeRenderer` | GPU command execution | `blade_renderer.rs` |

### What TextureRenderer Is Missing

```
Current TextureRenderer:
┌─────────────────────────┐
│   TextureRenderer       │
│   ┌─────────────────┐   │
│   │   Application   │   │  ← Created but not run
│   └─────────────────┘   │
│           ╳             │  ← No Window
│           ╳             │  ← No TextSystem access
│           ╳             │  ← No Layout
│           ╳             │  ← No Scene
│           ╳             │  ← No BladeRenderer access
│   ┌─────────────────┐   │
│   │ Gradient Loop   │   │  ← Fake output
│   └─────────────────┘   │
└─────────────────────────┘
```

---

## One-Shot Rendering Architecture

For true one-shot rendering (no event loop, render once and exit), a different architecture is needed.

### Requirements Analysis

| Requirement | Normal Window | One-Shot Render |
|-------------|---------------|-----------------|
| Event loop | Required | Not needed |
| Window surface | Required | Not needed |
| GPU context | Via window | Standalone |
| Text system | Via App | Must initialize |
| Layout engine | Via Window | Must provide |
| Entity system | Via App | Simplified or full |
| Scene | Built during paint | Must build |

### Proposed Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    OneShotRenderer                           │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                   Initialization                        │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐ │ │
│  │  │   GPU    │  │  Text    │  │  Asset Source        │ │ │
│  │  │ Context  │  │  System  │  │  (fonts, images)     │ │ │
│  │  └──────────┘  └──────────┘  └──────────────────────┘ │ │
│  └────────────────────────────────────────────────────────┘ │
│                           │                                  │
│                           ▼                                  │
│  ┌────────────────────────────────────────────────────────┐ │
│  │              render_to_png(size, build_view)            │ │
│  │                                                          │ │
│  │  1. Create OneShotContext                               │ │
│  │     ├── Minimal entity store                            │ │
│  │     ├── Layout engine                                   │ │
│  │     └── Scene builder                                   │ │
│  │                                                          │ │
│  │  2. Build view                                          │ │
│  │     let view = build_view(&mut ctx);                    │ │
│  │                                                          │ │
│  │  3. Layout                                              │ │
│  │     element.layout_as_root(size, &mut ctx);             │ │
│  │                                                          │ │
│  │  4. Paint                                               │ │
│  │     element.prepaint(&mut ctx);                         │ │
│  │     element.paint(&mut ctx);                            │ │
│  │                                                          │ │
│  │  5. Extract Scene                                       │ │
│  │     let scene = ctx.take_scene();                       │ │
│  │                                                          │ │
│  │  6. GPU Render                                          │ │
│  │     renderer.render_scene_to_texture(target, &scene);   │ │
│  │                                                          │ │
│  │  7. Readback                                            │ │
│  │     let pixels = renderer.read_pixels(target);          │ │
│  │                                                          │ │
│  │  8. Save PNG                                            │ │
│  │     save_rgba_png(path, size, &pixels);                 │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Proposed API

```rust
/// A minimal context for one-shot rendering without an event loop
pub struct OneShotRenderer {
    gpu: Arc<blade_graphics::Context>,
    text_system: TextSystem,
    asset_source: Arc<dyn AssetSource>,
    renderer: HeadlessBladeRenderer,
    atlas: BladeAtlas,
}

impl OneShotRenderer {
    /// Create a new one-shot renderer
    /// 
    /// This initializes GPU, text system, and atlas without requiring a window.
    pub fn new(asset_source: impl AssetSource) -> Result<Self> {
        // Create GPU context without a window
        let gpu = Arc::new(blade_graphics::Context::init(blade_graphics::ContextDesc {
            validation: cfg!(debug_assertions),
            capture: false,
            overlay: false,
        })?);
        
        // Initialize text system
        let text_system = TextSystem::new(/* ... */);
        
        // Create atlas for glyph caching
        let atlas = BladeAtlas::new(&gpu)?;
        
        // Create headless renderer
        let renderer = HeadlessBladeRenderer::new(Arc::clone(&gpu), &atlas)?;
        
        Ok(Self {
            gpu,
            text_system,
            asset_source: Arc::new(asset_source),
            renderer,
            atlas,
        })
    }
    
    /// Render a view to a PNG file
    pub fn render_to_png<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        scale_factor: f32,
        build_view: impl FnOnce(&mut OneShotContext) -> V,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let pixels = self.render_to_pixels(size, scale_factor, build_view)?;
        save_rgba_png(path, size, &pixels)
    }
    
    /// Render a view to raw RGBA pixels
    pub fn render_to_pixels<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        scale_factor: f32,
        build_view: impl FnOnce(&mut OneShotContext) -> V,
    ) -> Result<Vec<u8>> {
        // 1. Create minimal context
        let mut ctx = OneShotContext::new(
            &self.text_system,
            &self.asset_source,
            size,
            scale_factor,
        );
        
        // 2. Build the view
        let view = build_view(&mut ctx);
        let mut element = view.render(&mut ctx.window_context()).into_any();
        
        // 3. Layout phase
        let available_space = size.map(|d| AvailableSpace::Definite(d.into()));
        element.layout_as_root(available_space, &mut ctx.window_context());
        
        // 4. Prepaint phase
        element.prepaint(&mut ctx.window_context());
        
        // 5. Paint phase - generates Scene
        element.paint(&mut ctx.window_context());
        
        // 6. Extract scene
        let scene = ctx.take_scene();
        
        // 7. Create render target
        let target = self.renderer.create_render_target(size)?;
        
        // 8. Render scene to texture
        self.renderer.render_scene(&target, &scene, &mut self.atlas)?;
        
        // 9. Read pixels back
        self.renderer.read_pixels(&target)
    }
}

/// Minimal context for one-shot rendering
pub struct OneShotContext<'a> {
    text_system: &'a TextSystem,
    asset_source: &'a Arc<dyn AssetSource>,
    layout_engine: TaffyLayoutEngine,
    scene: Scene,
    scale_factor: f32,
    viewport_size: Size<Pixels>,
    // Minimal entity store for views that need it
    entities: OneShotEntityStore,
}
```

### Challenges and Solutions

#### Challenge 1: Text System Initialization

The text system needs fonts loaded before it can render text.

**Solution**: 
```rust
impl OneShotRenderer {
    pub fn with_system_fonts(mut self) -> Result<Self> {
        self.text_system.load_system_fonts()?;
        Ok(self)
    }
    
    pub fn with_font(mut self, font_data: &[u8]) -> Result<Self> {
        self.text_system.load_font(font_data)?;
        Ok(self)
    }
}
```

#### Challenge 2: Scale Factor Without Display

Without a display, there's no "native" scale factor.

**Solution**: Make it explicit in the API:
```rust
renderer.render_to_png(
    Size::new(DevicePixels(800), DevicePixels(600)),
    2.0,  // Explicit scale factor for HiDPI
    |cx| MyView::new(cx),
    "output.png",
)?;
```

#### Challenge 3: Async Operations in Views

Some views may initiate async data loading.

**Solutions**:
1. **Sync-only mode**: Only support synchronous rendering
2. **Block on async**: Provide a way to wait for async operations
3. **Placeholder rendering**: Render with loading states, allow re-render

```rust
// Option 2: Block on async
renderer.render_to_png_blocking(
    size,
    scale_factor,
    |cx| {
        let view = AsyncView::new(cx);
        cx.block_on_pending_tasks();  // Wait for data to load
        view
    },
    path,
)?;
```

#### Challenge 4: Entity References

Views often hold `Entity<T>` references to other entities.

**Solution**: Minimal entity store that supports basic operations:
```rust
struct OneShotEntityStore {
    entities: HashMap<EntityId, Box<dyn Any>>,
    next_id: EntityId,
}

impl OneShotEntityStore {
    fn new_entity<T: 'static>(&mut self, value: T) -> Entity<T> {
        let id = self.next_id;
        self.next_id = EntityId(id.0 + 1);
        self.entities.insert(id, Box::new(value));
        Entity::from_id(id)
    }
}
```

---

## Embedding GPUI in 3D Space

A future goal is to render GPUI interfaces as textures that can be displayed in 3D environments.

### Use Cases

1. **VR/AR Interfaces**: 2D UI panels floating in 3D space
2. **In-Game UI**: Computer screens, control panels, HUDs in game worlds
3. **Digital Twins**: Interactive dashboards on 3D equipment models
4. **Spatial Computing**: Mixed reality applications
5. **CAD/Design Tools**: Property panels rendered on 3D objects

### Architectural Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      3D Application                              │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                  3D Rendering Engine                        │ │
│  │              (Bevy, wgpu, Unity, Unreal, etc.)             │ │
│  │                                                              │ │
│  │   ┌─────────────────────────────────────────────────────┐  │ │
│  │   │              GPUI Surface Manager                    │  │ │
│  │   │                                                       │  │ │
│  │   │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  │  │ │
│  │   │  │  Surface 1  │  │  Surface 2  │  │  Surface N  │  │  │ │
│  │   │  │  (Menu)     │  │  (Console)  │  │  (HUD)      │  │  │ │
│  │   │  │             │  │             │  │             │  │  │ │
│  │   │  │ ┌─────────┐ │  │ ┌─────────┐ │  │ ┌─────────┐ │  │  │ │
│  │   │  │ │ Texture │ │  │ │ Texture │ │  │ │ Texture │ │  │  │ │
│  │   │  │ └─────────┘ │  │ └─────────┘ │  │ └─────────┘ │  │  │ │
│  │   │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  │  │ │
│  │   │         │                │                │         │  │ │
│  │   └─────────┼────────────────┼────────────────┼─────────┘  │ │
│  │             │                │                │            │ │
│  │             ▼                ▼                ▼            │ │
│  │   ┌─────────────────────────────────────────────────────┐  │ │
│  │   │              3D Scene Graph                          │  │ │
│  │   │                                                       │  │ │
│  │   │    [Quad with      [Curved         [Screen-space    │  │ │
│  │   │     Texture 1]      Surface]        Overlay]        │  │ │
│  │   │                                                       │  │ │
│  │   └─────────────────────────────────────────────────────┘  │ │
│  │                           │                                │ │
│  │                           ▼                                │ │
│  │   ┌─────────────────────────────────────────────────────┐  │ │
│  │   │                  Final Render                        │  │ │
│  │   └─────────────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### Technical Challenges

#### Challenge 1: GPU Texture Sharing

The 3D engine and GPUI both use the GPU. Texture data must be shared efficiently.

| Approach | Pros | Cons |
|----------|------|------|
| **Shared GPU Context** | Zero-copy, best performance | Deep integration required |
| **CPU Roundtrip** | Simple, portable | Slow (GPU→CPU→GPU copy) |
| **Interop APIs** | Efficient, no deep integration | Platform-specific |

**Recommended**: Start with CPU roundtrip for correctness, optimize to shared context later.

```rust
// CPU Roundtrip approach
impl GpuiSurface {
    fn update_3d_texture(&mut self, engine: &mut Engine3D) {
        if self.dirty {
            // Render GPUI to internal texture
            self.gpui_renderer.render(&self.scene);
            
            // Read pixels to CPU
            let pixels = self.gpui_renderer.read_pixels();
            
            // Upload to 3D engine's texture
            engine.update_texture(self.texture_id, &pixels);
            
            self.dirty = false;
        }
    }
}

// Shared context approach (optimal)
impl GpuiSurface {
    fn get_texture_view(&self) -> &wgpu::TextureView {
        // GPUI and 3D engine share the same wgpu device
        // Direct texture access, no copy needed
        &self.gpui_texture_view
    }
}
```

#### Challenge 2: Input Transformation

Mouse clicks in 3D space must be transformed to 2D GPUI coordinates.

```rust
/// Represents a GPUI surface positioned in 3D space
struct GpuiSurface3D {
    /// The GPUI content
    surface: EmbeddedGpuiSurface,
    /// 3D transform of the surface
    transform: Matrix4<f32>,
    /// Physical size in world units
    world_size: Size<f32>,
}

impl GpuiSurface3D {
    /// Convert a 3D ray (e.g., from mouse raycast) to GPUI coordinates
    fn ray_to_gpui_coords(&self, ray: Ray3D) -> Option<Point<Pixels>> {
        // 1. Compute intersection with surface plane
        let plane = self.compute_plane();
        let hit_point = ray.intersect_plane(&plane)?;
        
        // 2. Transform world point to local surface coordinates
        let local = self.transform.inverse() * hit_point;
        
        // 3. Convert to UV (0..1 range)
        let u = (local.x / self.world_size.width + 0.5).clamp(0.0, 1.0);
        let v = (local.y / self.world_size.height + 0.5).clamp(0.0, 1.0);
        
        // 4. Convert to pixel coordinates
        Some(Point::new(
            px(u * self.surface.size.width.0 as f32),
            px(v * self.surface.size.height.0 as f32),
        ))
    }
    
    /// Handle a 3D input event
    fn handle_3d_input(&mut self, event: Input3DEvent) {
        match event {
            Input3DEvent::RayHover(ray) => {
                if let Some(pos) = self.ray_to_gpui_coords(ray) {
                    self.surface.inject_mouse_move(pos);
                }
            }
            Input3DEvent::RayClick(ray, button) => {
                if let Some(pos) = self.ray_to_gpui_coords(ray) {
                    self.surface.inject_mouse_click(pos, button);
                }
            }
            Input3DEvent::KeyPress(key) => {
                self.surface.inject_key_event(key);
            }
        }
    }
}
```

#### Challenge 3: Render Timing

When should GPUI surfaces re-render?

| Mode | Description | Use Case |
|------|-------------|----------|
| **Continuous** | Re-render every frame | Animations, real-time data |
| **On-Demand** | Re-render when dirty | Static UI, battery saving |
| **Throttled** | Max N renders per second | Balance of both |

```rust
enum RenderMode {
    Continuous,
    OnDemand,
    Throttled { max_fps: u32 },
}

impl GpuiSurfaceManager {
    fn update(&mut self, dt: Duration) {
        for surface in &mut self.surfaces {
            let should_render = match surface.render_mode {
                RenderMode::Continuous => true,
                RenderMode::OnDemand => surface.is_dirty(),
                RenderMode::Throttled { max_fps } => {
                    surface.time_since_render += dt;
                    let min_interval = Duration::from_secs_f32(1.0 / max_fps as f32);
                    surface.is_dirty() && surface.time_since_render >= min_interval
                }
            };
            
            if should_render {
                surface.render();
                surface.time_since_render = Duration::ZERO;
            }
        }
    }
}
```

#### Challenge 4: Focus Management

With multiple GPUI surfaces in 3D:

- Which surface receives keyboard input?
- How does focus transfer between surfaces?
- What about global keyboard shortcuts?

```rust
struct GpuiSurfaceManager {
    surfaces: Vec<GpuiSurface3D>,
    focused_surface: Option<SurfaceId>,
}

impl GpuiSurfaceManager {
    fn focus_surface(&mut self, id: SurfaceId) {
        // Blur previous surface
        if let Some(prev) = self.focused_surface {
            if let Some(surface) = self.get_surface_mut(prev) {
                surface.on_blur();
            }
        }
        
        // Focus new surface
        self.focused_surface = Some(id);
        if let Some(surface) = self.get_surface_mut(id) {
            surface.on_focus();
        }
    }
    
    fn dispatch_keyboard(&mut self, event: KeyEvent) -> bool {
        // First, check for global shortcuts
        if self.handle_global_shortcut(&event) {
            return true;
        }
        
        // Then, dispatch to focused surface
        if let Some(id) = self.focused_surface {
            if let Some(surface) = self.get_surface_mut(id) {
                return surface.inject_key_event(event);
            }
        }
        
        false
    }
}
```

### Proposed Embedded Surface API

```rust
/// A GPUI surface that can be embedded in external 3D renderers
pub struct EmbeddedGpuiSurface {
    /// Unique identifier
    id: SurfaceId,
    /// Root view being rendered
    root: AnyView,
    /// Size in device pixels
    size: Size<DevicePixels>,
    /// Scale factor for rendering
    scale_factor: f32,
    /// GPU texture containing rendered content
    texture: GpuTexture,
    /// Whether content needs re-rendering
    dirty: bool,
    /// Current input state
    input_state: InputState,
    /// Render mode
    render_mode: RenderMode,
}

impl EmbeddedGpuiSurface {
    /// Create a new embedded surface with a root view
    pub fn new<V: Render>(
        size: Size<DevicePixels>,
        scale_factor: f32,
        build_root: impl FnOnce(&mut Context<V>) -> V,
        renderer: &mut EmbeddedRenderer,
    ) -> Result<Self> {
        // Initialize surface with GPU resources
        // Build root view and perform initial render
    }
    
    /// Resize the surface
    pub fn resize(&mut self, new_size: Size<DevicePixels>) {
        if self.size != new_size {
            self.size = new_size;
            self.dirty = true;
            // Reallocate GPU texture
        }
    }
    
    /// Inject a mouse move event
    pub fn inject_mouse_move(&mut self, position: Point<Pixels>) {
        self.input_state.mouse_position = position;
        // Trigger hover state updates
        self.dirty = true;
    }
    
    /// Inject a mouse click event
    pub fn inject_mouse_click(&mut self, position: Point<Pixels>, button: MouseButton) {
        self.input_state.mouse_position = position;
        // Dispatch click to element at position
        self.dirty = true;
    }
    
    /// Inject a keyboard event
    pub fn inject_key_event(&mut self, event: KeyEvent) -> bool {
        // Route to focused element, return true if handled
    }
    
    /// Mark the surface as needing re-render
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    
    /// Check if surface needs rendering
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    
    /// Render if dirty, return texture handle
    pub fn render_if_needed(&mut self, renderer: &mut EmbeddedRenderer) -> &GpuTexture {
        if self.dirty {
            self.render(renderer);
            self.dirty = false;
        }
        &self.texture
    }
    
    /// Force render regardless of dirty state
    pub fn render(&mut self, renderer: &mut EmbeddedRenderer) {
        // Full GPUI render cycle
    }
    
    /// Get the texture for sampling in 3D renderer
    pub fn texture(&self) -> &GpuTexture {
        &self.texture
    }
}
```

### Integration Example: Bevy

```rust
use bevy::prelude::*;
use gpui_embedded::{EmbeddedGpuiSurface, GpuiRenderer};

/// Bevy component wrapping a GPUI surface
#[derive(Component)]
struct GpuiPanel {
    surface: EmbeddedGpuiSurface,
}

/// Resource holding the GPUI renderer
#[derive(Resource)]
struct GpuiRenderResource {
    renderer: GpuiRenderer,
}

/// System: Render dirty GPUI surfaces
fn render_gpui_surfaces(
    mut panels: Query<&mut GpuiPanel>,
    mut gpui: ResMut<GpuiRenderResource>,
) {
    for mut panel in panels.iter_mut() {
        panel.surface.render_if_needed(&mut gpui.renderer);
    }
}

/// System: Handle input raycasting to GPUI panels
fn gpui_input_system(
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut panels: Query<(&mut GpuiPanel, &GlobalTransform, &Handle<Mesh>)>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    meshes: Res<Assets<Mesh>>,
) {
    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    
    for (camera, camera_transform) in cameras.iter() {
        let Some(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
            continue;
        };
        
        for (mut panel, transform, mesh_handle) in panels.iter_mut() {
            let Some(mesh) = meshes.get(mesh_handle) else { continue };
            
            // Raycast against panel mesh
            if let Some(hit) = raycast_mesh(&ray, mesh, transform) {
                // Convert to GPUI coordinates
                let gpui_pos = world_to_gpui_coords(&hit, transform, &panel.surface);
                
                if mouse_button.just_pressed(MouseButton::Left) {
                    panel.surface.inject_mouse_click(gpui_pos, MouseButton::Left);
                } else {
                    panel.surface.inject_mouse_move(gpui_pos);
                }
            }
        }
    }
}

/// Plugin to add GPUI support to Bevy
pub struct GpuiPlugin;

impl Plugin for GpuiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GpuiRenderResource {
            renderer: GpuiRenderer::new().expect("Failed to create GPUI renderer"),
        })
        .add_systems(Update, (render_gpui_surfaces, gpui_input_system));
    }
}
```

---

## Recommendations

### Immediate Fixes Required

1. **Delete or Fix `TextureRenderer`**
   
   The current implementation is actively misleading. Either:
   - Remove it entirely until proper implementation is ready
   - Add clear documentation that it's a non-functional placeholder
   - Implement it correctly

2. **Refactor `BladeRenderer` Rendering Loop**
   
   Extract the scene rendering code into a shared method:
   
   ```rust
   impl BladeRenderer {
       /// Render a scene to the given render target view
       fn render_scene_to_target(
           &mut self,
           scene: &Scene,
           target_view: gpu::TextureView,
           target_size: Size<DevicePixels>,
           globals: GlobalParams,
       ) {
           // Single implementation of the render loop
       }
       
       pub fn draw(&mut self, scene: &Scene) {
           // ... setup ...
           self.render_scene_to_target(scene, frame.texture_view(), size, globals);
           // ... present ...
       }
       
       pub fn render_to_texture(&mut self, target_id: RenderTargetId, scene: &Scene) -> Result<()> {
           // ... setup ...
           self.render_scene_to_target(scene, target.view, target.size, globals);
           // ... no present needed ...
       }
   }
   ```

3. **Create `HeadlessBladeRenderer`**
   
   A variant that can be constructed without a window surface:
   
   ```rust
   impl HeadlessBladeRenderer {
       pub fn new(gpu: Arc<blade_graphics::Context>) -> Result<Self> {
           // Initialize without window surface
           // Create atlas, pipelines, etc.
       }
   }
   ```

### Short-Term Goals

4. **Implement Minimal `OneShotContext`**
   
   Create the minimal infrastructure needed for one-shot rendering:
   - Layout engine (Taffy)
   - Scene builder
   - Simplified text system access
   - Basic entity support

5. **Add Integration Tests**
   
   Verify that actual GPUI content is rendered:
   
   ```rust
   #[test]
   fn test_render_to_texture_produces_correct_output() {
       let renderer = OneShotRenderer::new(TestAssets).unwrap();
       let pixels = renderer.render_to_pixels(
           Size::new(DevicePixels(100), DevicePixels(100)),
           1.0,
           |cx| {
               div().bg(rgb(0xFF0000)).size_full()
           },
       ).unwrap();
       
       // Verify pixels are red, not gradient
       assert_eq!(pixels[0..4], [255, 0, 0, 255]);
   }
   ```

### Long-Term Goals

6. **Design Embedded Surface API**
   
   Create a clean API for embedding GPUI in external renderers, considering:
   - Texture sharing strategies
   - Input routing
   - Focus management
   - Render scheduling

7. **Document Architecture**
   
   Provide clear documentation on:
   - How GPUI rendering works internally
   - How to extend for new use cases
   - Performance considerations

---

## Appendix: File Reference

### Files Modified in Commit 9ad72c6

| File | Lines Added | Purpose |
|------|-------------|---------|
| `crates/gpui/Cargo.toml` | 6 | Added PNG feature to image crate |
| `crates/gpui/examples/render_to_texture.rs` | 69 | Example (non-functional) |
| `crates/gpui/src/gpui.rs` | 35 | New types and trait |
| `crates/gpui/src/platform/blade.rs` | 13 | Re-exports and helper function |
| `crates/gpui/src/platform/blade/blade_renderer.rs` | 550 | Render target implementation |
| `crates/gpui/src/platform/blade/shaders.wgsl` | 16 | New fragment shader |
| `crates/gpui/src/texture_renderer.rs` | 122 | High-level API (placeholder) |

### Key Existing Files for Reference

| File | Purpose |
|------|---------|
| `crates/gpui/src/window.rs` | Window rendering lifecycle |
| `crates/gpui/src/scene.rs` | Scene structure and batching |
| `crates/gpui/src/app.rs` | Application and entity management |
| `crates/gpui/src/elements/*.rs` | Element system |
| `crates/gpui/src/text_system.rs` | Text rendering |