# One-Shot Rendering Architecture for GPUI

> **Last Updated**: Based on GPUI architecture as of commit 392c78ea5d (main branch)

This document describes the architecture needed to render GPUI views without a running event loop or visible window - "one-shot" rendering for export to images or textures.

## Overview

One-shot rendering allows capturing GPUI UI as pixels without:
- Starting the platform event loop
- Creating a visible window
- Running continuously

### Use Cases

1. **Screenshot Export**: Save UI state to PNG/JPEG files
2. **Thumbnail Generation**: Create preview images of documents/components
3. **Visual Testing**: Capture UI for regression testing
4. **Print/PDF**: High-resolution rendering for documents
5. **Texture Generation**: Create textures for 3D embedding (covered separately)

---

## Current GPUI Rendering Flow

Understanding the normal flow is essential for designing one-shot rendering:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Normal Window Rendering                      │
│                                                                  │
│  Application::run()                                              │
│       │                                                          │
│       ▼                                                          │
│  Platform Event Loop (runs continuously)                         │
│       │                                                          │
│       ▼                                                          │
│  Window::draw()                                                  │
│       │                                                          │
│       ├──▶ Prepaint Phase                                        │
│       │         └── Layout (Taffy flexbox)                       │
│       │         └── Hit testing setup                            │
│       │         └── Deferred draws                               │
│       │                                                          │
│       ├──▶ Paint Phase                                           │
│       │         └── Scene building                               │
│       │         └── Primitive insertion (quads, glyphs, etc.)    │
│       │                                                          │
│       └──▶ Focus Phase                                           │
│                 └── Focus change notifications                   │
│       │                                                          │
│       ▼                                                          │
│  Window::present()                                               │
│       │                                                          │
│       ▼                                                          │
│  BladeRenderer::draw(&scene)                                     │
│       │                                                          │
│       ├── Rasterize paths to atlas                               │
│       ├── Acquire frame from surface                             │
│       ├── Render batches to GPU                                  │
│       └── Present to display                                     │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Components Required

| Component | Normal Source | One-Shot Alternative |
|-----------|--------------|---------------------|
| Event Loop | Platform | Not needed |
| Window Surface | Platform window | Offscreen texture |
| GPU Context | Via window | Standalone |
| Text System | `App.text_system: Arc<TextSystem>` | Must initialize |
| Window Text System | `WindowTextSystem` with `LineLayoutCache` | Must provide |
| Atlas | `BladeRenderer.atlas: Arc<BladeAtlas>` | Must provide |
| Layout Engine | `Window.layout_engine: TaffyLayoutEngine` | Must provide |
| Entity System | `App.entities: EntityMap` | Minimal store |
| Scene | `Window.next_frame.scene: Scene` | Must build |

**Important**: Element methods (`request_layout`, `prepaint`, `paint`) all require both `&mut Window` and `&mut App` parameters. The one-shot context must provide compatible interfaces for both.

---

## One-Shot Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────────────┐
│                      OneShotRenderer                             │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Initialization                         │   │
│  │                                                           │   │
│  │   ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │   │
│  │   │     GPU     │  │    Text     │  │     Asset       │  │   │
│  │   │   Context   │  │   System    │  │     Source      │  │   │
│  │   │  (headless) │  │  (fonts)    │  │  (images, etc.) │  │   │
│  │   └─────────────┘  └─────────────┘  └─────────────────┘  │   │
│  │          │                │                 │             │   │
│  │          └────────────────┼─────────────────┘             │   │
│  │                           │                               │   │
│  │                           ▼                               │   │
│  │                  ┌─────────────────┐                      │   │
│  │                  │  BladeAtlas     │                      │   │
│  │                  │  (glyph cache)  │                      │   │
│  │                  └─────────────────┘                      │   │
│  │                           │                               │   │
│  │                           ▼                               │   │
│  │                  ┌─────────────────┐                      │   │
│  │                  │ HeadlessRenderer│                      │   │
│  │                  │ (no window)     │                      │   │
│  │                  └─────────────────┘                      │   │
│  │                                                           │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│                              ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │              render_to_pixels(size, build_view)           │   │
│  │                                                           │   │
│  │   1. Create OneShotContext                                │   │
│  │        ├── Layout engine (Taffy)                          │   │
│  │        ├── Scene builder                                  │   │
│  │        ├── Entity store (minimal)                         │   │
│  │        └── Text system reference                          │   │
│  │                                                           │   │
│  │   2. Build view via closure                               │   │
│  │        let view = build_view(&mut ctx);                   │   │
│  │                                                           │   │
│  │   3. Convert to element                                   │   │
│  │        let element = view.render(&mut ctx);               │   │
│  │                                                           │   │
│  │   4. Layout phase                                         │   │
│  │        element.layout_as_root(size, &mut ctx);            │   │
│  │                                                           │   │
│  │   5. Prepaint phase                                       │   │
│  │        element.prepaint(&mut ctx);                        │   │
│  │                                                           │   │
│  │   6. Paint phase                                          │   │
│  │        element.paint(&mut ctx);                           │   │
│  │        // Scene is now populated                          │   │
│  │                                                           │   │
│  │   7. Extract scene                                        │   │
│  │        let scene = ctx.take_scene();                      │   │
│  │                                                           │   │
│  │   8. Create render target                                 │   │
│  │        let target = renderer.create_target(size);         │   │
│  │                                                           │   │
│  │   9. GPU render                                           │   │
│  │        renderer.render_scene(&target, &scene);            │   │
│  │                                                           │   │
│  │  10. Read pixels                                          │   │
│  │        let pixels = renderer.read_pixels(&target);        │   │
│  │                                                           │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│                              ▼                                   │
│                        Vec<u8> (RGBA)                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Component Design

### Architecture Note: Window and App Separation

In GPUI, element rendering requires two separate context types:

1. **`App`** - Global application state:
   - Entity management (`entities: EntityMap`)
   - Text system (`text_system: Arc<TextSystem>`)
   - SVG renderer, asset source
   - Global observers and effects

2. **`Window`** - Per-window rendering state:
   - Layout engine (`layout_engine: TaffyLayoutEngine`)
   - Scene (`next_frame.scene: Scene`)
   - Scale factor, viewport size
   - Content mask stack, element state

All element methods have signatures like:
```rust
fn prepaint(&mut self, window: &mut Window, cx: &mut App);
fn paint(&mut self, window: &mut Window, cx: &mut App);
```

For one-shot rendering, we need to either:
1. Create minimal implementations of both `Window` and `App`
2. Or create a unified context that can be split into both interfaces

### OneShotRenderer

The main entry point for one-shot rendering:

```rust
/// Renderer for one-shot (non-continuous) GPUI rendering
pub struct OneShotRenderer {
    /// GPU context (can be created without window)
    gpu: Arc<blade_graphics::Context>,
    
    /// Text system for font rendering
    text_system: Arc<TextSystem>,
    
    /// Asset source for images, SVGs, etc.
    asset_source: Arc<dyn AssetSource>,
    
    /// Texture atlas for glyph caching
    atlas: BladeAtlas,
    
    /// Headless renderer (no window surface)
    renderer: HeadlessBladeRenderer,
    
    /// Default scale factor (can be overridden per-render)
    default_scale_factor: f32,
}

impl OneShotRenderer {
    /// Create a new one-shot renderer with an asset source
    pub fn new(asset_source: impl AssetSource + 'static) -> Result<Self> {
        Self::with_options(asset_source, OneShotOptions::default())
    }
    
    /// Create with custom options
    pub fn with_options(
        asset_source: impl AssetSource + 'static,
        options: OneShotOptions,
    ) -> Result<Self> {
        // 1. Create GPU context without window
        let gpu = Arc::new(blade_graphics::Context::init(blade_graphics::ContextDesc {
            validation: options.gpu_validation,
            capture: false,
            overlay: false,
        })?);
        
        // 2. Create text system
        let text_system = Arc::new(TextSystem::new(Arc::clone(&gpu)));
        
        // 3. Load fonts if requested
        if options.load_system_fonts {
            text_system.load_system_fonts()?;
        }
        
        // 4. Create atlas
        let atlas = BladeAtlas::new(&gpu)?;
        
        // 5. Create headless renderer
        let renderer = HeadlessBladeRenderer::new(
            Arc::clone(&gpu),
            options.default_texture_format,
        )?;
        
        Ok(Self {
            gpu,
            text_system,
            asset_source: Arc::new(asset_source),
            atlas,
            renderer,
            default_scale_factor: options.default_scale_factor,
        })
    }
    
    /// Load a font from bytes
    pub fn load_font(&self, font_data: impl Into<Cow<'static, [u8]>>) -> Result<()> {
        self.text_system.add_fonts(vec![font_data.into()])
    }
    
    /// Render a view to RGBA pixels
    pub fn render_to_pixels<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        build_view: impl FnOnce(&mut OneShotViewContext<V>) -> V,
    ) -> Result<Vec<u8>> {
        self.render_to_pixels_with_scale(size, self.default_scale_factor, build_view)
    }
    
    /// Render with explicit scale factor
    pub fn render_to_pixels_with_scale<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        scale_factor: f32,
        build_view: impl FnOnce(&mut OneShotViewContext<V>) -> V,
    ) -> Result<Vec<u8>> {
        // Create the dual context (provides both Window-like and App-like interfaces)
        let mut ctx = OneShotContext::new(
            &self.text_system,
            &self.asset_source,
            size,
            scale_factor,
        );
        
        // Build the view - need to provide Context<V> which wraps App
        let mut view_ctx = OneShotViewContext::new(&mut ctx);
        let view = build_view(&mut view_ctx);
        
        // Get the element - Render::render requires Window and Context<V>
        // Note: view.render() signature is:
        //   fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement
        let mut element = {
            let (window, app) = ctx.as_window_and_app();
            view.render(window, app).into_any()
        };
        
        // Layout phase - layout_as_root requires Window and App
        let available_space = size.map(|d| AvailableSpace::Definite(Pixels(d.0 as f32 / scale_factor)));
        {
            let (window, app) = ctx.as_window_and_app();
            element.layout_as_root(available_space, window, app);
        }
        
        // Prepaint phase
        {
            let (window, app) = ctx.as_window_and_app();
            element.prepaint(window, app);
        }
        
        // Paint phase (populates scene)
        {
            let (window, app) = ctx.as_window_and_app();
            element.paint(window, app);
        }
        
        // Extract scene
        let scene = ctx.take_scene();
        
        // Create render target
        let target = self.renderer.create_render_target(size)?;
        
        // Rasterize paths to intermediate texture (required for path rendering)
        self.renderer.rasterize_paths(scene.paths(), &mut self.atlas)?;
        
        // Render scene to texture
        self.renderer.render_scene(&target, &scene, &self.atlas)?;
        
        // Read back pixels
        self.renderer.read_pixels(&target)
    }
    
    /// Render to PNG file
    pub fn render_to_png<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        build_view: impl FnOnce(&mut OneShotViewContext<V>) -> V,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let pixels = self.render_to_pixels(size, build_view)?;
        save_rgba_png(path, size, &pixels)
    }
}

/// Options for OneShotRenderer creation
pub struct OneShotOptions {
    /// Enable GPU validation (debug builds)
    pub gpu_validation: bool,
    /// Load system fonts automatically
    pub load_system_fonts: bool,
    /// Default scale factor for HiDPI
    pub default_scale_factor: f32,
    /// Default texture format
    pub default_texture_format: gpu::TextureFormat,
}

impl Default for OneShotOptions {
    fn default() -> Self {
        Self {
            gpu_validation: cfg!(debug_assertions),
            load_system_fonts: true,
            default_scale_factor: 1.0,
            default_texture_format: gpu::TextureFormat::Rgba8UnormSrgb,
        }
    }
}
```

### OneShotContext

A context that provides both `Window`-like and `App`-like interfaces needed for element layout and painting.

**Key insight**: GPUI's `TaffyLayoutEngine::compute_layout` signature is:
```rust
pub fn compute_layout(
    &mut self,
    id: LayoutId,
    available_space: Size<AvailableSpace>,
    window: &mut Window,  // Required!
    cx: &mut App,         // Required!
)
```

This means we need to provide compatible interfaces for both types.

```rust
/// Context for one-shot rendering that can provide both Window and App interfaces
/// 
/// This is a unified context that internally manages the state normally split
/// between Window and App, and can present itself as either when needed.
pub struct OneShotContext {
    // === App-like state ===
    /// Text system (shared across all windows normally)
    text_system: Arc<TextSystem>,
    
    /// Asset source for images, SVGs
    asset_source: Arc<dyn AssetSource>,
    
    /// SVG renderer
    svg_renderer: SvgRenderer,
    
    /// Minimal entity store
    entities: OneShotEntityStore,
    
    // === Window-like state ===
    /// Per-window text system with line layout cache
    window_text_system: WindowTextSystem,
    
    /// Layout engine (Taffy)
    layout_engine: TaffyLayoutEngine,
    
    /// Current frame being built
    next_frame: OneShotFrame,
    
    /// Scale factor for this render
    scale_factor: f32,
    
    /// Viewport size in device pixels
    viewport_size_device: Size<DevicePixels>,
    
    /// Viewport size in logical pixels
    viewport_size: Size<Pixels>,
    
    /// Content mask stack
    content_mask_stack: Vec<ContentMask<Pixels>>,
    
    /// Element opacity stack
    opacity_stack: Vec<f32>,
    
    /// Text style stack
    text_style_stack: Vec<TextStyleRefinement>,
    
    /// Element offset stack (for absolute positioning)
    element_offset_stack: Vec<Point<Pixels>>,
}

/// Minimal frame state for one-shot rendering
pub struct OneShotFrame {
    /// Scene being built
    pub scene: Scene,
    
    /// Hitboxes (may not be needed for one-shot)
    pub hitboxes: Vec<Hitbox>,
    
    /// Dispatch tree (minimal for one-shot)
    pub dispatch_tree: DispatchTree,
}

impl OneShotContext {
    pub fn new(
        text_system: &Arc<TextSystem>,
        asset_source: &Arc<dyn AssetSource>,
        size: Size<DevicePixels>,
        scale_factor: f32,
    ) -> Self {
        let viewport_size = Size {
            width: px(size.width.0 as f32 / scale_factor),
            height: px(size.height.0 as f32 / scale_factor),
        };
        
        // Create per-window text system with its own line layout cache
        let window_text_system = WindowTextSystem::new(Arc::clone(text_system));
        
        Self {
            // App-like state
            text_system: Arc::clone(text_system),
            asset_source: Arc::clone(asset_source),
            svg_renderer: SvgRenderer::new(Arc::clone(asset_source)),
            entities: OneShotEntityStore::new(),
            
            // Window-like state
            window_text_system,
            layout_engine: TaffyLayoutEngine::new(),
            next_frame: OneShotFrame {
                scene: Scene::default(),
                hitboxes: Vec::new(),
                dispatch_tree: DispatchTree::default(),
            },
            scale_factor,
            viewport_size_device: size,
            viewport_size,
            content_mask_stack: vec![ContentMask {
                bounds: Bounds {
                    origin: Point::zero(),
                    size: viewport_size,
                },
            }],
            opacity_stack: vec![1.0],
            text_style_stack: Vec::new(),
            element_offset_stack: vec![Point::zero()],
        }
    }
    
    /// Take the built scene
    pub fn take_scene(&mut self) -> Scene {
        std::mem::take(&mut self.next_frame.scene)
    }
    
    /// Get mutable references to window-like and app-like interfaces
    /// 
    /// This is the key method that allows OneShotContext to work with
    /// GPUI's element system which expects both Window and App.
    pub fn as_window_and_app(&mut self) -> (&mut OneShotWindow, &mut OneShotApp) {
        // Safety: We're returning two non-overlapping parts of self
        // This would need careful implementation to avoid aliasing
        todo!("Implement safe splitting of context into Window and App interfaces")
    }
    
    /// Get viewport size
    pub fn viewport_size(&self) -> Size<Pixels> {
        self.viewport_size
    }
    
    /// Get scale factor
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
    
    /// Get current content mask
    pub fn content_mask(&self) -> ContentMask<Pixels> {
        self.content_mask_stack.last().cloned().unwrap_or_default()
    }
    
    /// Get current opacity
    pub fn opacity(&self) -> f32 {
        self.opacity_stack.iter().product()
    }
    
    // === Scene Building Methods ===
    
    pub fn paint_quad(&mut self, quad: PaintQuad) {
        let scale = self.scale_factor;
        let content_mask = self.content_mask();
        let opacity = self.opacity();
        
        self.scene.insert_primitive(Quad {
            order: 0,
            bounds: quad.bounds.scale(scale),
            content_mask: content_mask.scale(scale),
            background: quad.background.opacity(opacity),
            border_color: quad.border_color.opacity(opacity),
            corner_radii: quad.corner_radii.scale(scale),
            border_widths: quad.border_widths.scale(scale),
            transformation: quad.transformation,
        });
    }
    
    pub fn paint_glyph(
        &mut self,
        origin: Point<Pixels>,
        glyph: &ShapedGlyph,
        color: Hsla,
    ) -> Result<()> {
        // Similar to Window::paint_glyph but simplified
        let scale = self.scale_factor;
        let content_mask = self.content_mask().scale(scale);
        let opacity = self.opacity();
        
        // Rasterize glyph if needed (via text system)
        let tile = self.text_system.rasterize_glyph(glyph, scale)?;
        
        self.scene.insert_primitive(MonochromeSprite {
            order: 0,
            pad: 0,
            bounds: Bounds {
                origin: (origin * scale).floor(),
                size: tile.bounds.size.map(Into::into),
            },
            content_mask,
            color: color.opacity(opacity),
            tile,
        });
        
        Ok(())
    }
    
    // ... other paint methods (shadows, underlines, images, etc.)
}
```

### OneShotEntityStore

Minimal entity storage for views that need to create child entities:

```rust
/// Minimal entity store for one-shot rendering
/// 
/// Supports basic entity creation and access but not subscriptions,
/// observations, or other reactive features.
pub struct OneShotEntityStore {
    entities: HashMap<EntityId, Box<dyn Any + Send>>,
    next_id: u64,
}

impl OneShotEntityStore {
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            next_id: 0,
        }
    }
    
    /// Create a new entity
    pub fn new_entity<T: 'static + Send>(&mut self, value: T) -> Entity<T> {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        self.entities.insert(id, Box::new(value));
        Entity::from_id(id)
    }
    
    /// Read an entity
    pub fn read<T: 'static>(&self, entity: &Entity<T>) -> Option<&T> {
        self.entities.get(&entity.entity_id())
            .and_then(|boxed| boxed.downcast_ref())
    }
    
    /// Update an entity
    pub fn update<T: 'static, R>(
        &mut self,
        entity: &Entity<T>,
        f: impl FnOnce(&mut T) -> R,
    ) -> Option<R> {
        self.entities.get_mut(&entity.entity_id())
            .and_then(|boxed| boxed.downcast_mut())
            .map(f)
    }
}
```

### HeadlessBladeRenderer

A variant of BladeRenderer that doesn't require a window surface.

**Note**: The current `BladeRenderer::new` requires:
```rust
pub fn new<I: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle>(
    context: &BladeContext,
    window: &I,
    config: BladeSurfaceConfig,
) -> anyhow::Result<Self>
```

The headless version must work without these window handles.

**Additional complexity**: BladeRenderer now has intermediate textures for path rendering:
- `path_intermediate_texture` / `path_intermediate_texture_view`
- `path_intermediate_msaa_texture` / `path_intermediate_msaa_texture_view` (optional, for MSAA)
- `RenderingParameters` loaded from environment

```rust
/// GPU renderer that works without a window surface
pub struct HeadlessBladeRenderer {
    gpu: Arc<blade_graphics::Context>,
    command_encoder: blade_graphics::CommandEncoder,
    pipelines: BladePipelines,
    instance_belt: BufferBelt,
    atlas_sampler: gpu::Sampler,
    default_format: gpu::TextureFormat,
    
    // Path rendering intermediate textures (sized to max render target)
    path_intermediate_texture: gpu::Texture,
    path_intermediate_texture_view: gpu::TextureView,
    path_intermediate_msaa_texture: Option<gpu::Texture>,
    path_intermediate_msaa_texture_view: Option<gpu::TextureView>,
    
    // Rendering parameters (path sample count, etc.)
    rendering_parameters: RenderingParameters,
}

impl HeadlessBladeRenderer {
    pub fn new(
        gpu: Arc<blade_graphics::Context>,
        default_format: gpu::TextureFormat,
    ) -> Result<Self> {
        // Load shaders
        let shader_source = include_str!("shaders.wgsl");
        let shader = gpu.create_shader(gpu::ShaderDesc {
            source: shader_source,
        });
        
        // Create pipelines (same as BladeRenderer but without surface-specific config)
        let pipelines = BladePipelines::new(&gpu, &shader, default_format);
        
        // Create instance belt for dynamic data
        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 16,
        });
        
        // Create atlas sampler
        let atlas_sampler = gpu.create_sampler(gpu::SamplerDesc {
            name: "atlas",
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });
        
        Ok(Self {
            gpu,
            command_encoder: blade_graphics::CommandEncoder::new(),
            pipelines,
            instance_belt,
            atlas_sampler,
            path_tiles: HashMap::new(),
            default_format,
        })
    }
    
    /// Create a render target texture
    pub fn create_render_target(&mut self, size: Size<DevicePixels>) -> Result<RenderTarget> {
        let texture = self.gpu.create_texture(gpu::TextureDesc {
            name: "render_target",
            format: self.default_format,
            size: gpu::Extent {
                width: size.width.0 as u32,
                height: size.height.0 as u32,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::TARGET | gpu::TextureUsage::RESOURCE | gpu::TextureUsage::COPY,
        });
        
        let view = self.gpu.create_texture_view(
            texture,
            gpu::TextureViewDesc {
                name: "render_target_view",
                format: self.default_format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );
        
        Ok(RenderTarget { texture, view, size })
    }
    
    /// Rasterize paths to intermediate texture
    /// 
    /// This mirrors BladeRenderer::draw_paths_to_intermediate which renders
    /// paths to an intermediate texture before compositing to the final target.
    pub fn rasterize_paths(
        &mut self,
        paths: &[Path<ScaledPixels>],
        viewport_width: f32,
        viewport_height: f32,
    ) -> Result<()> {
        // Similar to BladeRenderer::draw_paths_to_intermediate
        // 1. Clear intermediate texture
        // 2. For each path batch, render to intermediate with MSAA if enabled
        // 3. Resolve MSAA if used
        // Paths are then composited in render_scene using the paths pipeline
    }
    
    /// Render a scene to a render target
    pub fn render_scene(
        &mut self,
        target: &RenderTarget,
        scene: &Scene,
        atlas: &BladeAtlas,
    ) -> Result<()> {
        self.command_encoder.start();
        self.command_encoder.init_texture(target.texture);
        
        let globals = GlobalParams {
            viewport_size: [
                target.size.width.0 as f32,
                target.size.height.0 as f32,
            ],
            premultiplied_alpha: 1, // Always premultiplied for offscreen
            pad: 0,
        };
        
        if let mut pass = self.command_encoder.render(
            "one_shot_render",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: target.view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        ) {
            // Render all batches (same logic as BladeRenderer::draw)
            self.render_batches(&mut pass, scene, atlas, globals);
        }
        
        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.instance_belt.flush(&sync_point);
        self.gpu.wait_for(&sync_point, 10000); // Wait for completion
        
        Ok(())
    }
    
    /// Read pixels from a render target
    pub fn read_pixels(&mut self, target: &RenderTarget) -> Result<Vec<u8>> {
        let size = target.size;
        let bytes_per_pixel = 4u64; // RGBA8
        let row_pitch = size.width.0 as u64 * bytes_per_pixel;
        let buffer_size = row_pitch * size.height.0 as u64;
        
        // Create staging buffer
        let staging = self.gpu.create_buffer(gpu::BufferDesc {
            name: "pixel_readback",
            size: buffer_size,
            memory: gpu::Memory::Shared,
        });
        
        // Copy texture to buffer
        self.command_encoder.start();
        {
            let mut transfer = self.command_encoder.transfer("readback");
            transfer.copy_texture_to_buffer(
                gpu::TexturePiece {
                    texture: target.texture,
                    mip_level: 0,
                    array_layer: 0,
                    origin: [0, 0, 0],
                },
                staging.into(),
                row_pitch as u32,
                gpu::Extent {
                    width: size.width.0 as u32,
                    height: size.height.0 as u32,
                    depth: 1,
                },
            );
        }
        
        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.gpu.wait_for(&sync_point, 10000);
        
        // Read data from staging buffer
        let pixels = unsafe {
            std::slice::from_raw_parts(
                staging.data() as *const u8,
                buffer_size as usize,
            ).to_vec()
        };
        
        // Cleanup
        self.gpu.destroy_buffer(staging);
        
        Ok(pixels)
    }
    
    fn render_batches(
        &mut self,
        pass: &mut gpu::RenderCommandEncoder,
        scene: &Scene,
        atlas: &BladeAtlas,
        globals: GlobalParams,
    ) {
        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Quads(quads) => {
                    let instance_buf = unsafe {
                        self.instance_belt.alloc_typed(quads, &self.gpu)
                    };
                    let mut encoder = pass.with(&self.pipelines.quads);
                    encoder.bind(0, &ShaderQuadsData {
                        globals,
                        b_quads: instance_buf,
                    });
                    encoder.draw(0, 4, 0, quads.len() as u32);
                }
                PrimitiveBatch::Shadows(shadows) => {
                    // Similar to quads...
                }
                PrimitiveBatch::Paths(paths) => {
                    // Render path sprites from atlas...
                }
                PrimitiveBatch::Underlines(underlines) => {
                    // Similar to quads...
                }
                PrimitiveBatch::MonochromeSprites { texture_id, sprites } => {
                    // Render monochrome sprites (glyphs, SVGs)...
                }
                PrimitiveBatch::PolychromeSprites { texture_id, sprites } => {
                    // Render polychrome sprites (emoji, images)...
                }
                PrimitiveBatch::Surfaces(_) => {
                    // Video surfaces not supported in one-shot mode
                }
            }
        }
    }
}

/// A render target texture
pub struct RenderTarget {
    pub texture: gpu::Texture,
    pub view: gpu::TextureView,
    pub size: Size<DevicePixels>,
}
```

---

## Usage Examples

### Basic Usage

```rust
use gpui::{div, rgb, px, OneShotRenderer, Size, DevicePixels, Styled, ParentElement};

fn main() -> anyhow::Result<()> {
    // Create renderer with default options
    let mut renderer = OneShotRenderer::new(EmptyAssetSource)?;
    
    // Render a simple view
    renderer.render_to_png(
        Size::new(DevicePixels(800), DevicePixels(600)),
        |cx| {
            div()
                .size_full()
                .bg(rgb(0x1e2127))
                .child(
                    div()
                        .p_4()
                        .bg(rgb(0x2b2e33))
                        .rounded_md()
                        .child("Hello, One-Shot Rendering!")
                )
        },
        "output.png",
    )?;
    
    Ok(())
}
```

### With Custom Fonts

```rust
let mut renderer = OneShotRenderer::with_options(
    MyAssetSource,
    OneShotOptions {
        load_system_fonts: false, // Don't load system fonts
        ..Default::default()
    },
)?;

// Load custom font
let font_data = include_bytes!("../assets/fonts/Inter-Regular.ttf");
renderer.load_font(font_data.as_slice())?;

// Now render with custom font
renderer.render_to_png(size, |cx| {
    div().text_sm().child("Rendered with Inter font")
}, "output.png")?;
```

### HiDPI Rendering

```rust
// Render at 2x scale for Retina displays
let pixels = renderer.render_to_pixels_with_scale(
    Size::new(DevicePixels(1600), DevicePixels(1200)), // 2x size
    2.0, // 2x scale factor
    |cx| my_view(cx),
)?;
// Result: 1600x1200 pixels, but UI is sized for 800x600 logical pixels
```

### With Entity State

```rust
struct Counter {
    count: i32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_4()
            .child(format!("Count: {}", self.count))
    }
}

renderer.render_to_png(size, |cx| {
    // The closure receives a context that can create entities
    Counter { count: 42 }
}, "counter.png")?;
```

---

## Limitations

### Not Supported in One-Shot Mode

1. **Async Operations**: Data loading, network requests
2. **Animations**: Time-based animations won't advance
3. **User Input**: No mouse/keyboard handling
4. **Focus Management**: Focus state is not meaningful
5. **Subscriptions/Observations**: Reactive updates don't work
6. **Platform Integration**: Clipboard, drag-drop, system dialogs
7. **Video Surfaces**: CVPixelBuffer surfaces (macOS video)
8. **Window Chrome**: Title bars, shadows, resize handles
9. **Effects System**: `cx.push_effect()` and deferred updates
10. **Inspector**: Debug inspector UI (requires full window)

### Architectural Challenges

The biggest challenge is that GPUI's element system is deeply coupled to having both `Window` and `App`:

```rust
// Every element method requires both contexts
fn paint(&mut self, window: &mut Window, cx: &mut App);

// Layout computation requires both
layout_engine.compute_layout(layout_id, available_space, window, cx);
```

---

## Existing Test Infrastructure: A Key Discovery

> **Important**: The test infrastructure is gated behind the `test-support` feature flag in GPUI's `Cargo.toml`. To use it for one-shot rendering, you would need to either:
> 1. Enable `test-support` feature (includes `TestAppContext`, `TestPlatform`, etc.)
> 2. Propose upstreaming a separate `headless` feature that exposes similar functionality

GPUI already has test infrastructure that solves many of these problems:

### TestAppContext and VisualTestContext

```rust
// From gpui/src/app/test_context.rs

/// A TestAppContext is provided to tests created with `#[gpui::test]`
pub struct TestAppContext {
    pub app: Rc<AppCell>,
    pub background_executor: BackgroundExecutor,
    pub foreground_executor: ForegroundExecutor,
    pub dispatcher: TestDispatcher,
    test_platform: Rc<TestPlatform>,
    text_system: Arc<TextSystem>,
    // ...
}

/// Provides Window and App for visual testing
pub struct VisualTestContext {
    pub cx: TestAppContext,
    window: AnyWindowHandle,
}

impl VisualTestContext {
    /// Provides a `Window` and `App` for the duration of the closure
    pub fn update<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.window, |_, window, cx| f(window, cx))
            .unwrap()
    }
}
```

### TestPlatform and TestWindow

```rust
// From gpui/src/platform/test/platform.rs
pub(crate) struct TestPlatform {
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    pub text_system: Arc<dyn PlatformTextSystem>,
    // ...
}

// From gpui/src/platform/test/window.rs  
impl PlatformWindow for TestWindow {
    fn draw(&self, _scene: &crate::Scene) {}  // No-op!
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> { ... }
    // ...
}
```

### Key Insight

The test infrastructure already provides:
1. **Real `App` and `Window`** - Not mocks, actual GPUI types
2. **TestPlatform** - A platform that works without a real display
3. **TestWindow** - A `PlatformWindow` implementation with no-op `draw()`
4. **TestAtlas** - A fake sprite atlas for testing

**This means we can potentially leverage this existing infrastructure for one-shot rendering!**

---

## Overcoming the Window/App Coupling

This is the core architectural challenge. `Window` and `App` are concrete structs, not traits. All element methods require references to both. Here's a detailed analysis of the options:

### Analysis: What Do Elements Actually Need?

From examining the codebase, elements use `Window` and `App` for:

**From `Window`:**
- `window.request_layout(style, children, cx)` → layout engine
- `window.request_measured_layout(style, measure_fn)` → layout with custom measurement  
- `window.compute_layout(layout_id, available_space, cx)` → triggers Taffy layout
- `window.layout_bounds(layout_id)` → get computed bounds
- `window.scale_factor()` → for pixel scaling
- `window.viewport_size()` → viewport dimensions
- `window.text_system()` → `WindowTextSystem` for text shaping
- `window.content_mask()` → current clipping region
- `window.paint_quad()`, `window.paint_glyph()`, etc. → scene building
- `window.element_id_stack` → for element identification

**From `App` (`cx`):**
- `cx.text_system()` → global `TextSystem` (fonts, line wrappers)
- `cx.notify(entity)` → mark entity as needing redraw (reactive)
- `cx.entities` → entity storage for `Entity<T>` access
- `cx.spawn()` → async task spawning (not needed for one-shot)

**Key insight from `TaffyLayoutEngine::compute_layout`:**
```rust
// The measure callback receives both window and cx
(node_context.measure)(known_dimensions, available_space, window, cx)

// Measure functions typically use:
// - window.text_system().shape_text() for text measurement
// - cx.text_system().line_wrapper() for line wrapping
```

### Option 1: Modify GPUI to Use Traits (Invasive)

Extract the required functionality into traits:

```rust
pub trait LayoutContext {
    fn scale_factor(&self) -> f32;
    fn text_system(&self) -> &WindowTextSystem;
    fn request_layout(&mut self, style: Style, children: impl Iterator<Item = LayoutId>) -> LayoutId;
    // ... other layout methods
}

pub trait PaintContext {
    fn paint_quad(&mut self, quad: PaintQuad);
    fn paint_glyph(&mut self, ...);
    fn content_mask(&self) -> ContentMask<Pixels>;
    // ... other paint methods
}

pub trait AppContext {
    fn text_system(&self) -> &TextSystem;
    fn notify(&mut self, entity: EntityId);
    // ... other app methods
}
```

**Pros:**
- Clean abstraction
- One-shot can implement these traits independently
- Future-proof for other use cases

**Cons:**
- Requires significant changes to GPUI core
- All element implementations would need updating
- Upstream maintenance burden
- Breaking change for existing code

### Option 2: Create Real Window + App (Pragmatic)

Actually create real `Window` and `App` instances, but in a "headless" configuration:

```rust
pub struct OneShotRenderer {
    app: Rc<AppCell>,  // Real App
}

impl OneShotRenderer {
    pub fn new(asset_source: impl AssetSource) -> Result<Self> {
        // Create a real App with headless platform
        let platform = HeadlessPlatform::new();
        let app = App::new_app(
            Rc::new(platform),
            Arc::new(asset_source),
            Arc::new(NullHttpClient),
        );
        
        Ok(Self { app })
    }
    
    pub fn render_to_pixels<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        build_view: impl FnOnce(&mut Context<V>) -> V,
    ) -> Result<Vec<u8>> {
        let mut app = self.app.borrow_mut();
        
        // Create a headless window (no platform window, just the GPUI Window struct)
        let window = app.open_headless_window(size)?;
        
        window.update(&mut app, |window, cx| {
            // Build and render the view using real GPUI infrastructure
            let view = cx.new(build_view);
            let mut element = view.render(window, cx).into_any();
            
            // Use real layout/paint cycle
            element.layout_as_root(size.into(), window, cx);
            element.prepaint(window, cx);
            element.paint(window, cx);
            
            // Extract scene from window.next_frame.scene
            let scene = std::mem::take(&mut window.next_frame.scene);
            
            // Render scene to texture
            self.render_scene_to_texture(&scene, size)
        })
    }
}
```

**Requires adding to GPUI:**
```rust
impl App {
    /// Create a window without a platform window (for headless rendering)
    pub fn open_headless_window(&mut self, size: Size<DevicePixels>) -> Result<WindowId> {
        // Create Window struct with:
        // - Mock PlatformWindow that does nothing
        // - Real layout engine, text system, scene
        // - No event handling or display
    }
}
```

**Pros:**
- Uses real GPUI code paths - guaranteed compatibility
- All elements work correctly
- Minimal changes to GPUI core
- Text system, layout, entities all work normally

**Cons:**
- Still requires some GPUI modifications (headless window support)
- Carries more overhead than strictly necessary
- Need to handle or stub reactive features (notify, observers)

### Option 3: Fork Window Fields (Contained)

Create a `OneShotWindow` struct that contains the same fields as `Window` that are needed for rendering, but nothing else:

```rust
/// A minimal Window-like struct for one-shot rendering
pub struct OneShotWindow {
    // Fields copied from Window that are needed for rendering:
    pub(crate) text_system: Arc<WindowTextSystem>,
    pub(crate) viewport_size: Size<Pixels>,
    pub(crate) layout_engine: Option<TaffyLayoutEngine>,
    pub(crate) element_id_stack: SmallVec<[ElementId; 32]>,
    pub(crate) text_style_stack: Vec<TextStyleRefinement>,
    pub(crate) element_offset_stack: Vec<Point<Pixels>>,
    pub(crate) element_opacity: f32,
    pub(crate) content_mask_stack: Vec<ContentMask<Pixels>>,
    pub(crate) next_frame: Frame,
    scale_factor: f32,
    rem_size: Pixels,
}

impl OneShotWindow {
    // Implement the same methods as Window that elements use:
    pub fn request_layout(&mut self, style: Style, children: impl Iterator<Item = LayoutId>, cx: &mut OneShotApp) -> LayoutId { ... }
    pub fn layout_bounds(&mut self, layout_id: LayoutId) -> Bounds<Pixels> { ... }
    pub fn paint_quad(&mut self, quad: PaintQuad) { ... }
    pub fn scale_factor(&self) -> f32 { ... }
    pub fn text_system(&self) -> &WindowTextSystem { ... }
    // ... etc
}
```

**The problem:** Element code is compiled against concrete `Window` and `App` types:
```rust
fn paint(&mut self, window: &mut Window, cx: &mut App);
//                         ^^^^^^ This is the concrete Window struct
```

You cannot pass `&mut OneShotWindow` where `&mut Window` is expected.

**Workaround - Unsafe transmutation (NOT RECOMMENDED):**
```rust
// DON'T DO THIS - undefined behavior
let one_shot_window: &mut OneShotWindow = ...;
let window: &mut Window = unsafe { std::mem::transmute(one_shot_window) };
```

This would be undefined behavior because `OneShotWindow` doesn't have the same memory layout as `Window`.

### Option 4: Render Without Elements (Limited)

Bypass the element system entirely and build scenes directly:

```rust
impl OneShotRenderer {
    pub fn render_scene(&mut self, size: Size<DevicePixels>) -> Scene {
        let mut scene = Scene::default();
        
        // Build scene primitives directly
        scene.insert_primitive(Quad {
            bounds: Bounds::new(Point::zero(), size.map(|d| ScaledPixels(d.0 as f32))),
            background: Background::Color(Hsla::blue()),
            // ...
        });
        
        // For text, manually shape and add glyphs
        let shaped = self.text_system.shape_line(...);
        for glyph in shaped.glyphs() {
            scene.insert_primitive(MonochromeSprite { ... });
        }
        
        scene
    }
}
```

**Pros:**
- No GPUI modifications needed
- Full control over what gets rendered

**Cons:**
- Loses all the benefits of the element system (layout, styling, components)
- Must manually implement layout (or use Taffy directly)
- Can't use existing GPUI components
- Defeats the purpose of "render GPUI views to texture"

### Option 5: Compile-Time Feature Flag (Cleanest Long-Term)

Add a feature flag to GPUI that enables alternative context types:

```rust
// In gpui/src/lib.rs
#[cfg(feature = "headless")]
mod headless;

#[cfg(feature = "headless")]
pub use headless::{HeadlessApp, HeadlessWindow, HeadlessRenderer};
```

The headless module would contain properly integrated versions that share code with the main implementation.

**Pros:**
- Clean integration with GPUI
- Can be upstreamed
- No runtime overhead when not used
- Full compatibility with element system

**Cons:**
- Requires GPUI core changes
- More initial implementation work
- Need to keep in sync with GPUI changes

---

## Recommended Approach

**For MVP: Leverage Existing Test Infrastructure**

The discovery of `TestAppContext` and `TestPlatform` changes the recommendation significantly:

```rust
pub struct OneShotRenderer {
    cx: TestAppContext,
}

impl OneShotRenderer {
    pub fn new() -> Result<Self> {
        // Use existing test infrastructure!
        let dispatcher = TestDispatcher::new(StdRng::seed_from_u64(0));
        let cx = TestAppContext::build(dispatcher, None);
        Ok(Self { cx })
    }
    
    pub fn render_to_pixels<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Result<Vec<u8>> {
        // Create a test window with the desired size
        let window = self.cx.add_window(|window, cx| build_view(window, cx));
        
        // Get a VisualTestContext for this window
        let mut vcx = VisualTestContext::from_window(window.into(), &self.cx);
        
        // Draw the window to generate the scene
        vcx.update(|window, cx| {
            window.draw(cx);
            
            // Extract the scene
            let scene = std::mem::take(&mut window.rendered_frame.scene);
            
            // Render scene to texture using HeadlessBladeRenderer
            self.render_scene_to_pixels(&scene, size)
        })
    }
}
```

**Why this is the best approach:**

1. **Zero GPUI modifications needed** - Uses existing public/internal test APIs
2. **Battle-tested** - This is how all GPUI tests work
3. **Full compatibility** - Real `Window` and `App`, all elements work
4. **Text system works** - `TestPlatform` provides a real text system
5. **Entity system works** - Full `EntityMap` support

**Implementation steps:**

1. Expose `TestAppContext::build()` or similar for non-test use (or use `#[cfg(feature = "test-support")]`)
2. Create `HeadlessBladeRenderer` that renders `Scene` to texture (no window surface)
3. Wire up the one-shot API to extract scenes and render them

**For Long-Term: New `headless` Feature Flag**

Once the MVP works, consider upstreaming a proper `headless` feature to GPUI that:
- Makes test infrastructure available without the `test-support` feature (which pulls in test dependencies)
- Adds explicit `render_to_texture()` API
- Documents headless rendering as a supported use case
- Separates concerns: `test-support` for testing, `headless` for rendering

**Cargo.toml addition would look like:**
```toml
[features]
headless = [
    # Minimal dependencies for headless rendering
    "blade-graphics",
    "blade-macros",
    # ... other GPU deps
]
```

### Workarounds

| Limitation | Workaround |
|------------|------------|
| Async data | Pre-load data before rendering |
| Animations | Render specific animation frame by setting state |
| Focus styling | Manually set focus state in view |
| Dynamic content | Multiple renders with different state |

---

## Implementation Roadmap

### Phase 1: Foundation

1. Create `HeadlessBladeRenderer` that doesn't require window
2. Extract shared rendering code from `BladeRenderer::draw()`
3. Add basic `OneShotContext` with layout and scene building

### Phase 2: Text System Integration

1. Allow text system initialization without window
2. Support custom font loading
3. Glyph rasterization to atlas

### Phase 3: Full Element Support

1. All element types (div, text, images, SVG)
2. Flexbox layout via Taffy
3. Content masking and opacity

### Phase 4: API Polish

1. Clean public API (`OneShotRenderer`)
2. Error handling and validation
3. Documentation and examples

### Phase 5: Testing

1. Visual regression tests
2. Performance benchmarks
3. Memory leak detection

---

## Related Documents

- [Render-to-Texture Critique](./render-to-texture-critique.md) - Analysis of current implementation
- [Embedding GPUI in 3D](./embedding-gpui-in-3d.md) - Future architecture for 3D integration

---

---

## Summary

### The Core Problem
GPUI's element system is tightly coupled to concrete `Window` and `App` types - these are structs, not traits. Every element method requires `&mut Window` and `&mut App`:

```rust
fn paint(&mut self, window: &mut Window, cx: &mut App);
```

### The Solution
GPUI already has test infrastructure (`TestAppContext`, `TestPlatform`, `TestWindow`) that provides real `Window` and `App` instances without requiring a display. This infrastructure:

1. Uses a `TestPlatform` that works without windowing system
2. Creates `TestWindow` instances with no-op `draw()` methods  
3. Provides full text system, layout engine, and entity support
4. Is battle-tested across the entire GPUI test suite

### Implementation Path

1. **Enable `test-support` feature** (or create new `headless` feature)
2. **Use `TestAppContext`** to get real `App` and `Window`
3. **Create `HeadlessBladeRenderer`** that renders `Scene` to texture without window surface
4. **Extract scene after `window.draw()`** and render to pixels

### Key Files to Reference
- `gpui/src/app/test_context.rs` - `TestAppContext`, `VisualTestContext`
- `gpui/src/platform/test/platform.rs` - `TestPlatform`
- `gpui/src/platform/test/window.rs` - `TestWindow`
- `gpui/src/platform/blade/blade_renderer.rs` - Rendering to GPU

---

## Appendix: Current GPUI Architecture Notes

### Key Type Signatures (as of main branch)

```rust
// BladeRenderer requires window handle
impl BladeRenderer {
    pub fn new<I: HasWindowHandle + HasDisplayHandle>(
        context: &BladeContext,
        window: &I,
        config: BladeSurfaceConfig,
    ) -> anyhow::Result<Self>
}

// Window::draw returns ArenaClearNeeded
impl Window {
    pub fn draw(&mut self, cx: &mut App) -> ArenaClearNeeded
}

// DrawPhase enum
pub(crate) enum DrawPhase {
    None,
    Prepaint,
    Paint,
    Focus,
}

// Scene is pub(crate)
pub(crate) struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    // ... fields ...
}

// WindowTextSystem wraps TextSystem with per-window cache
pub struct WindowTextSystem {
    line_layout_cache: LineLayoutCache,
    text_system: Arc<TextSystem>,
}
```

### GpuiMode

There's a `GpuiMode` enum in App that includes `skip_drawing()` - this could potentially be leveraged:

```rust
pub(crate) mode: GpuiMode,

// In Window::draw:
if !cx.mode.skip_drawing() {
    self.draw_roots(cx);
}
```

---

## Appendix: Key GPUI Types

| Type | Purpose | Location |
|------|---------|----------|
| `Scene` | Collection of GPU primitives | `scene.rs` |
| `PrimitiveBatch` | Batched draw commands | `scene.rs` |
| `Quad` | Rectangle primitive | `scene.rs` |
| `MonochromeSprite` | Single-color sprite (glyphs) | `scene.rs` |
| `PolychromeSprite` | Full-color sprite (images) | `scene.rs` |
| `BladeAtlas` | GPU texture cache | `blade_atlas.rs` |
| `BladeRenderer` | GPU rendering (requires window) | `blade_renderer.rs` |
| `TextSystem` | Global font and glyph management | `text_system.rs` |
| `WindowTextSystem` | Per-window text with line cache | `text_system.rs` |
| `TaffyLayoutEngine` | Flexbox layout | `taffy.rs` |
| `App` | Global application state | `app.rs` |
| `Window` | Per-window rendering state | `window.rs` |
| `DrawPhase` | Current phase of rendering | `window.rs` |
| `RenderingParameters` | GPU rendering config | `blade_renderer.rs` |