# Render-to-Texture: OffscreenRenderer Approach

## Overview

This document details the approach of creating a separate `BladeOffscreenRenderer` that shares expensive GPU resources with the main `BladeRenderer` while maintaining its own per-instance state for safe concurrent/reentrant rendering.

## Problem: BladeRenderer is Not Reentrant

The current `BladeRenderer` has several stateful components that prevent reentrant calls:

| Component | Issue |
|-----------|-------|
| `command_encoder` | Only one recording session at a time |
| `instance_belt` | Allocation tracking, `flush()` timing sensitive |
| `path_intermediate_texture` | Shared temp render target for path MSAA |
| `last_sync_point` | Would be overwritten in nested calls |

If we call `draw_to_texture()` during a `draw()` call (e.g., from a paint callback), the state would be corrupted.

## Solution: Separate OffscreenRenderer

Create a lightweight `BladeOffscreenRenderer` that:
- **Shares** expensive resources with the main renderer
- **Has its own** per-instance state for isolation
- **Reuses** the exact same rendering logic

---

## Resource Analysis

### Shareable Resources (Expensive)

| Resource | Type | Notes |
|----------|------|-------|
| `gpu` | `Arc<gpu::Context>` | Already Arc, thread-safe |
| `pipelines` | `BladePipelines` | Needs to be wrapped in Arc |
| `atlas` | `Arc<BladeAtlas>` | Already Arc, has internal Mutex |
| `atlas_sampler` | `gpu::Sampler` | Stateless GPU resource |
| `rendering_parameters` | `RenderingParameters` | Small, can clone or share |

### Per-Instance Resources (Cheap)

| Resource | Cost | Notes |
|----------|------|-------|
| `command_encoder` | Cheap | Just a GPU handle |
| `instance_belt` | Cheap | Starts empty, grows on demand |
| `path_intermediate_texture` | Moderate | GPU allocation, sized to target |
| `path_intermediate_msaa_texture` | Moderate | Optional, for MSAA paths |

### Not Needed for Offscreen

| Resource | Why |
|----------|-----|
| `surface` | Only for window presentation |
| `surface_config` | Only for window |
| `last_sync_point` | Optional, for sync tracking |

---

## Proposed Structures

### SharedRenderResources

Extract shared resources into a separate struct:

```rust
/// GPU resources that can be shared between renderers.
/// These are expensive to create and should be reused.
pub struct SharedRenderResources {
    pub gpu: Arc<gpu::Context>,
    pub pipelines: Arc<BladePipelines>,
    pub atlas: Arc<BladeAtlas>,
    pub atlas_sampler: gpu::Sampler,
    pub rendering_parameters: RenderingParameters,
}

impl SharedRenderResources {
    pub fn new(gpu: Arc<gpu::Context>, surface_info: gpu::SurfaceInfo) -> Self {
        let pipelines = Arc::new(BladePipelines::new(
            &gpu,
            surface_info,
            RenderingParameters::from_env_default().path_sample_count,
        ));
        let atlas = Arc::new(BladeAtlas::new(&gpu));
        let atlas_sampler = gpu.create_sampler(gpu::SamplerDesc {
            name: "shared sampler",
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });
        
        Self {
            gpu,
            pipelines,
            atlas,
            atlas_sampler,
            rendering_parameters: RenderingParameters::from_env_default(),
        }
    }
}
```

### Modified BladeRenderer

```rust
pub struct BladeRenderer {
    // Shared resources
    shared: Arc<SharedRenderResources>,
    
    // Window-specific
    surface: gpu::Surface,
    surface_config: gpu::SurfaceConfig,
    
    // Per-instance state
    command_encoder: gpu::CommandEncoder,
    instance_belt: BufferBelt,
    path_intermediate_texture: gpu::Texture,
    path_intermediate_texture_view: gpu::TextureView,
    path_intermediate_msaa_texture: Option<gpu::Texture>,
    path_intermediate_msaa_texture_view: Option<gpu::TextureView>,
    last_sync_point: Option<gpu::SyncPoint>,
    
    #[cfg(target_os = "macos")]
    core_video_texture_cache: CVMetalTextureCache,
}

impl BladeRenderer {
    /// Get shared resources for creating offscreen renderers
    pub fn shared_resources(&self) -> Arc<SharedRenderResources> {
        Arc::clone(&self.shared)
    }
    
    /// Create an offscreen renderer that shares resources with this renderer
    pub fn create_offscreen_renderer(
        &self,
        max_texture_size: Size<DevicePixels>,
    ) -> BladeOffscreenRenderer {
        BladeOffscreenRenderer::new(
            Arc::clone(&self.shared),
            max_texture_size,
        )
    }
}
```

### BladeOffscreenRenderer

```rust
/// Lightweight renderer for offscreen texture rendering.
/// Shares expensive resources with the main BladeRenderer.
pub struct BladeOffscreenRenderer {
    // Shared with main renderer
    shared: Arc<SharedRenderResources>,
    
    // Per-instance state (isolated from main renderer)
    command_encoder: gpu::CommandEncoder,
    instance_belt: BufferBelt,
    path_intermediate_texture: gpu::Texture,
    path_intermediate_texture_view: gpu::TextureView,
    path_intermediate_msaa_texture: Option<gpu::Texture>,
    path_intermediate_msaa_texture_view: Option<gpu::TextureView>,
    
    // Offscreen-specific
    max_texture_size: Size<DevicePixels>,
}

impl BladeOffscreenRenderer {
    pub fn new(
        shared: Arc<SharedRenderResources>,
        max_texture_size: Size<DevicePixels>,
    ) -> Self {
        let command_encoder = shared.gpu.create_command_encoder(gpu::CommandEncoderDesc {
            name: "offscreen",
            buffer_count: 2,
        });
        
        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40,
        });
        
        // Create path intermediate textures sized for max texture
        let (path_intermediate_texture, path_intermediate_texture_view) =
            create_path_intermediate_texture(
                &shared.gpu,
                gpu::TextureFormat::Bgra8UnormSrgb,
                max_texture_size.width.0 as u32,
                max_texture_size.height.0 as u32,
            );
        
        let (path_intermediate_msaa_texture, path_intermediate_msaa_texture_view) =
            create_msaa_texture_if_needed(
                &shared.gpu,
                gpu::TextureFormat::Bgra8UnormSrgb,
                max_texture_size.width.0 as u32,
                max_texture_size.height.0 as u32,
                shared.rendering_parameters.path_sample_count,
            )
            .unzip();
        
        Self {
            shared,
            command_encoder,
            instance_belt,
            path_intermediate_texture,
            path_intermediate_texture_view,
            path_intermediate_msaa_texture,
            path_intermediate_msaa_texture_view,
            max_texture_size,
        }
    }
    
    /// Create a render target texture
    pub fn create_texture(&self, size: Size<DevicePixels>) -> (gpu::Texture, gpu::TextureView) {
        let texture = self.shared.gpu.create_texture(gpu::TextureDesc {
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
        
        let view = self.shared.gpu.create_texture_view(
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
    
    /// Render a scene to a texture
    pub fn draw_to_texture(
        &mut self,
        scene: &Scene,
        texture_view: gpu::TextureView,
        size: Size<DevicePixels>,
    ) -> gpu::SyncPoint {
        let viewport_size = [size.width.0 as f32, size.height.0 as f32];
        
        render_batches(
            scene,
            texture_view,
            viewport_size,
            true, // premultiplied_alpha for offscreen
            &mut self.command_encoder,
            &mut self.instance_belt,
            self.path_intermediate_texture_view,
            &self.shared.gpu,
            &self.shared.pipelines,
            &self.shared.atlas,
            self.shared.atlas_sampler,
            &self.shared.rendering_parameters,
        )
    }
    
    pub fn destroy(&mut self) {
        self.shared.gpu.destroy_command_encoder(&mut self.command_encoder);
        self.instance_belt.destroy(&self.shared.gpu);
        self.shared.gpu.destroy_texture(self.path_intermediate_texture);
        self.shared.gpu.destroy_texture_view(self.path_intermediate_texture_view);
        if let Some(tex) = self.path_intermediate_msaa_texture {
            self.shared.gpu.destroy_texture(tex);
        }
        if let Some(view) = self.path_intermediate_msaa_texture_view {
            self.shared.gpu.destroy_texture_view(view);
        }
    }
}
```

---

## Shared Rendering Logic

Extract the batch processing loop into a shared function:

```rust
/// Core rendering logic shared between BladeRenderer and BladeOffscreenRenderer.
/// 
/// This function renders all batches in a scene to the specified target.
fn render_batches(
    scene: &Scene,
    target_view: gpu::TextureView,
    viewport_size: [f32; 2],
    premultiplied_alpha: bool,
    // Per-instance resources (mutable)
    command_encoder: &mut gpu::CommandEncoder,
    instance_belt: &mut BufferBelt,
    path_intermediate_texture_view: gpu::TextureView,
    // Shared resources (immutable)
    gpu: &gpu::Context,
    pipelines: &BladePipelines,
    atlas: &BladeAtlas,
    atlas_sampler: gpu::Sampler,
    rendering_parameters: &RenderingParameters,
) -> gpu::SyncPoint {
    command_encoder.start();
    atlas.before_frame(command_encoder);

    let globals = GlobalParams {
        viewport_size,
        premultiplied_alpha: if premultiplied_alpha { 1 } else { 0 },
        pad: 0,
    };

    let mut pass = command_encoder.render(
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

    for batch in scene.batches() {
        match batch {
            PrimitiveBatch::Quads(quads) => {
                let instance_buf = unsafe { instance_belt.alloc_typed(quads, gpu) };
                let mut encoder = pass.with(&pipelines.quads);
                encoder.bind(0, &ShaderQuadsData { globals, b_quads: instance_buf });
                encoder.draw(0, 4, 0, quads.len() as u32);
            }
            PrimitiveBatch::Shadows(shadows) => {
                let instance_buf = unsafe { instance_belt.alloc_typed(shadows, gpu) };
                let mut encoder = pass.with(&pipelines.shadows);
                encoder.bind(0, &ShaderShadowsData { globals, b_shadows: instance_buf });
                encoder.draw(0, 4, 0, shadows.len() as u32);
            }
            PrimitiveBatch::Paths(paths) => {
                // ... path rendering with path_intermediate_texture_view
                // (same logic as current draw(), but using passed-in resources)
            }
            PrimitiveBatch::Underlines(underlines) => {
                // ... same pattern
            }
            PrimitiveBatch::MonochromeSprites { texture_id, sprites } => {
                let tex_info = atlas.get_texture_info(texture_id);
                let instance_buf = unsafe { instance_belt.alloc_typed(sprites, gpu) };
                let mut encoder = pass.with(&pipelines.mono_sprites);
                encoder.bind(0, &ShaderMonoSpritesData {
                    globals,
                    gamma_ratios: rendering_parameters.gamma_ratios,
                    grayscale_enhanced_contrast: rendering_parameters.grayscale_enhanced_contrast,
                    t_sprite: tex_info.raw_view,
                    s_sprite: atlas_sampler,
                    b_mono_sprites: instance_buf,
                });
                encoder.draw(0, 4, 0, sprites.len() as u32);
            }
            PrimitiveBatch::PolychromeSprites { texture_id, sprites } => {
                // ... same pattern
            }
            PrimitiveBatch::Surfaces(surfaces) => {
                // ... macOS video surfaces (skip for offscreen)
            }
        }
    }
    
    drop(pass);

    let sync_point = gpu.submit(command_encoder);
    instance_belt.flush(&sync_point);
    atlas.after_frame(&sync_point);

    sync_point
}
```

---

## Refactored BladeRenderer::draw()

```rust
impl BladeRenderer {
    pub fn draw(&mut self, scene: &Scene) {
        let frame = {
            profiling::scope!("acquire frame");
            self.surface.acquire_frame()
        };
        self.command_encoder.init_texture(frame.texture());

        let viewport_size = [
            self.surface_config.size.width as f32,
            self.surface_config.size.height as f32,
        ];
        let premultiplied_alpha = matches!(
            self.surface.info().alpha,
            gpu::AlphaMode::PreMultiplied
        );

        let sync_point = render_batches(
            scene,
            frame.texture_view(),
            viewport_size,
            premultiplied_alpha,
            &mut self.command_encoder,
            &mut self.instance_belt,
            self.path_intermediate_texture_view,
            &self.shared.gpu,
            &self.shared.pipelines,
            &self.shared.atlas,
            self.shared.atlas_sampler,
            &self.shared.rendering_parameters,
        );

        // Present to window (not done in render_batches)
        self.command_encoder.start();
        self.command_encoder.present(frame);
        self.shared.gpu.submit(&mut self.command_encoder);

        self.wait_for_gpu();
        self.last_sync_point = Some(sync_point);
    }
}
```

---

## Pipeline Compatibility

Pipelines are compiled for a specific texture format. For sharing to work:

1. **Window surface format**: Typically `Bgra8UnormSrgb`
2. **Offscreen texture format**: Must match → `Bgra8UnormSrgb`

This is handled by `create_texture()` using the same format.

If different formats are needed in the future, we'd need separate pipeline sets.

---

## Usage Patterns

### Creating an Offscreen Renderer

```rust
// From PlatformWindow implementation
impl WaylandWindow {
    fn create_offscreen_renderer(&self) -> BladeOffscreenRenderer {
        let state = self.0.borrow();
        state.renderer.create_offscreen_renderer(
            Size {
                width: DevicePixels(4096),  // Max texture size
                height: DevicePixels(4096),
            }
        )
    }
}
```

### Rendering to Texture

```rust
fn render_element_to_texture(
    &mut self,
    element: impl IntoElement,
    available_space: Size<AvailableSpace>,
    window: &mut Window,
    cx: &mut App,
) -> Result<CachedTexture> {
    // 1. Build scene (safe to do anytime)
    let (scene, size) = window.render_element_to_scene(element, available_space, cx);
    
    // 2. Get or create offscreen renderer
    let offscreen = self.get_offscreen_renderer(window);
    
    // 3. Create texture
    let device_size = size.scale(window.scale_factor());
    let (texture, view) = offscreen.create_texture(device_size);
    
    // 4. Render scene to texture
    let sync_point = offscreen.draw_to_texture(&scene, view, device_size);
    
    Ok(CachedTexture {
        texture,
        view,
        size,
        sync_point,
    })
}
```

### Pooling Offscreen Renderers

For efficiency, maintain a pool:

```rust
pub struct OffscreenRendererPool {
    shared: Arc<SharedRenderResources>,
    available: Vec<BladeOffscreenRenderer>,
    max_texture_size: Size<DevicePixels>,
}

impl OffscreenRendererPool {
    pub fn acquire(&mut self) -> BladeOffscreenRenderer {
        self.available.pop().unwrap_or_else(|| {
            BladeOffscreenRenderer::new(
                Arc::clone(&self.shared),
                self.max_texture_size,
            )
        })
    }
    
    pub fn release(&mut self, renderer: BladeOffscreenRenderer) {
        self.available.push(renderer);
    }
}
```

---

## Timing Considerations

Even with separate renderers, there are timing considerations:

### Atlas Synchronization

The `atlas.before_frame()` and `atlas.after_frame()` calls manage pending uploads:

```rust
// In BladeAtlas
pub fn before_frame(&self, gpu_encoder: &mut gpu::CommandEncoder) {
    let mut lock = self.0.lock();
    lock.flush(gpu_encoder);  // Flush pending uploads
}

pub fn after_frame(&self, sync_point: &gpu::SyncPoint) {
    let mut lock = self.0.lock();
    lock.upload_belt.flush(sync_point);  // Mark uploads complete
}
```

Since the atlas is shared, these calls happen for both renderers. This should be safe because:
- The Mutex ensures serialized access
- Each `before_frame` flushes pending uploads to that encoder
- Each `after_frame` marks that sync point's uploads complete

However, for safety, texture rendering should ideally happen:
- After window `draw()` completes, OR
- Before window `draw()` starts

### Recommended Flow

```
1. Window paint phase builds scenes
   - Main window scene → window.next_frame.scene
   - Texture scenes → collected separately

2. Window present
   - renderer.draw(window_scene)
   - GPU processes window frame

3. Texture rendering (after window frame)
   - For each texture scene:
     - offscreen.draw_to_texture(scene, texture)
   - GPU processes texture renders

4. Next frame
   - Use cached textures as sprites
```

---

## Implementation Plan

### Step 1: Extract SharedRenderResources ✅

- [x] Create `SharedRenderResources` struct
- [x] Modify `BladeRenderer` to use `shared: SharedRenderResources`
- [x] Update all methods to access via `self.shared.*`
- [x] **Test:** Verify window rendering unchanged (`cargo test -p gpui` passes)

### Step 2: Extract render_batches() ✅

- [x] Create `RenderContext` struct for per-instance mutable state
- [x] Extract `draw_paths_to_intermediate()` as standalone function
- [x] Create standalone `render_batches()` function
- [x] Refactor `BladeRenderer::draw()` to use it
- [x] Handle macOS Surfaces separately via `draw_surfaces()`
- [x] **Test:** Verify window rendering unchanged (`cargo test -p gpui` passes)

### Step 3: Create BladeOffscreenRenderer ✅

- [x] Implement `BladeOffscreenRenderer` struct
- [x] Add `new()` constructor that takes shared resources
- [x] Add `create_texture()` method
- [x] Add `draw_to_texture()` method using `render_batches()`
- [x] Add `destroy()` method
- [x] Add `BladeRenderer::create_offscreen_renderer()` factory method
- [x] Add `render-to-texture` feature flag to Cargo.toml
- [x] Feature-gate offscreen renderer module and methods
- [ ] **Test:** Render simple scenes to texture (deferred to Step 4 integration)

### Step 4: Expose Through Platform ✅

- [x] Add `PlatformOffscreenRenderer` trait to `platform.rs`
- [x] Add `OffscreenTextureId` and `OffscreenTextureInfo` types
- [x] Add `create_offscreen_renderer()` method to `PlatformWindow` trait
- [x] Implement `PlatformOffscreenRenderer` for `BladeOffscreenRenderer`
- [x] Implement `create_offscreen_renderer()` for Wayland
- [x] Implement `create_offscreen_renderer()` for X11
- [x] Add `OffscreenRenderer` wrapper type in `window.rs`
- [x] Add `Window::create_offscreen_renderer()` method
- [ ] Add `Window::render_element_to_texture()` helper (optional, for convenience)
- [ ] **Test:** End-to-end texture rendering

### Step 5: Add Pooling (Optional)

- [ ] Create `OffscreenRendererPool`
- [ ] Integrate with platform window
- [ ] **Test:** Multiple texture renders

---

## File Changes

| File | Changes |
|------|---------|
| `platform/blade/blade_renderer.rs` | Add `SharedRenderResources`, extract `render_batches()`, add `create_offscreen_renderer()`, wrap `BladePipelines` in `Arc` |
| `platform/blade/blade_offscreen_renderer.rs` | New file: `BladeOffscreenRenderer`, `OffscreenTexture`, impl `PlatformOffscreenRenderer` (feature-gated) |
| `platform/blade.rs` | Export `blade_offscreen_renderer` module (feature-gated) |
| `Cargo.toml` | Add `render-to-texture` feature flag |
| `platform.rs` | Add `PlatformOffscreenRenderer` trait, `OffscreenTextureId`, `OffscreenTextureInfo`, add `create_offscreen_renderer()` to `PlatformWindow` |
| `platform/linux/wayland/window.rs` | Implement `create_offscreen_renderer()` for `WaylandWindow` |
| `platform/linux/x11/window.rs` | Implement `create_offscreen_renderer()` for `X11Window` |
| `window.rs` | Add `OffscreenRenderer` wrapper, add `Window::create_offscreen_renderer()` |

---

## Advantages of This Approach

1. **Safe reentrancy** - Each renderer has isolated state
2. **Resource efficient** - Expensive resources shared
3. **Code reuse** - Same rendering logic via `render_batches()`
4. **Flexible timing** - Can render textures at any point
5. **Scalable** - Can create multiple offscreen renderers if needed
6. **Feature-gated** - Opt-in via `render-to-texture` feature flag
7. **Platform-agnostic API** - `PlatformOffscreenRenderer` trait enables cross-platform support
8. **Clean public API** - `OffscreenRenderer` wrapper hides internal `Scene` type

## Potential Issues

1. **Atlas contention** - Multiple renderers accessing atlas simultaneously
   - Mitigated by Mutex in BladeAtlas
2. **GPU memory** - Each offscreen renderer has path intermediate textures
   - Mitigated by pooling and reasonable max sizes
3. **Pipeline format lock-in** - All renderers must use same texture format
   - Acceptable for our use case

---

## References

- `gpui/src/platform/blade/blade_renderer.rs` - Current renderer implementation
- `gpui/src/platform/blade/blade_atlas.rs` - Atlas with Mutex for thread safety
- `gpui/research/render-to-texture-methodology.md` - Overall RTT design