# Solution: TexturedSurface Platform for GPUI

**Date**: Based on GPUI main branch (commit 392c78ea5d)  
**Status**: Architecture Design

---

## TL;DR

Create a new `TexturedSurface` platform client/window for Linux that:
1. Renders to a GPU texture instead of a window surface
2. Re-uses the existing `BladeRenderer` (with minor additions)
3. Leaves existing `HeadlessClient` unchanged (used by CLI tools, remote server, benchmarks)
4. Provides pixel readback for PNG export or 3D embedding

---

## Why Not Modify HeadlessClient?

The existing `HeadlessClient` is used by several tools that explicitly don't want windows:

| Crate | Usage |
|-------|-------|
| `edit_prediction_cli` | ML inference CLI tool |
| `eval` | Evaluation/benchmarking |
| `fs_benchmarks` | Filesystem benchmarks |
| `project_benchmarks` | Project loading benchmarks |
| `remote_server` | SSH remote server (explicitly no GUI) |
| `worktree_benchmarks` | Worktree scanning benchmarks |

These tools use `Application::headless()` specifically because they:
- Run in environments without displays (CI, SSH, servers)
- Don't need any window/rendering functionality
- Want minimal resource usage

Modifying `HeadlessClient` to support windows would:
- Add complexity to a simple "no windows" abstraction
- Risk breaking existing use cases
- Muddy the semantic meaning of "headless"

**Solution**: Create a new `TexturedSurfaceClient` that IS capable of rendering, but to textures instead of display surfaces.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Platform Selection                            │
│                                                                  │
│   current_platform(headless: bool) -> Rc<dyn Platform>          │
│                          │                                       │
│         ┌────────────────┼────────────────┐                     │
│         ▼                ▼                ▼                     │
│   ┌──────────┐    ┌──────────────┐  ┌─────────────────┐        │
│   │ Wayland  │    │    X11       │  │   Headless      │        │
│   │ Client   │    │   Client     │  │   Client        │        │
│   │          │    │              │  │ (no windows)    │        │
│   └──────────┘    └──────────────┘  └─────────────────┘        │
│                                                                  │
│   NEW: Application::textured() -> Rc<dyn Platform>              │
│                          │                                       │
│                          ▼                                       │
│                 ┌─────────────────────┐                         │
│                 │ TexturedSurface     │                         │
│                 │ Client              │                         │
│                 │ (renders to texture)│                         │
│                 └─────────────────────┘                         │
│                          │                                       │
│                          ▼                                       │
│                 ┌─────────────────────┐                         │
│                 │ TexturedSurface     │                         │
│                 │ Window              │                         │
│                 │ (GPU render target) │                         │
│                 └─────────────────────┘                         │
│                          │                                       │
│                          ▼                                       │
│                 ┌─────────────────────┐                         │
│                 │ BladeRenderer       │                         │
│                 │ (extended for       │                         │
│                 │  texture targets)   │                         │
│                 └─────────────────────┘                         │
└─────────────────────────────────────────────────────────────────┘
```

---

## Component Design

### 1. TexturedSurfaceClient

A new platform client that supports window creation but renders to textures:

```rust
// In gpui/src/platform/linux/textured_surface/client.rs

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use calloop::{EventLoop, LoopHandle};

use crate::platform::linux::LinuxClient;
use crate::platform::{LinuxCommon, PlatformWindow};
use crate::{
    AnyWindowHandle, CursorStyle, DisplayId, LinuxKeyboardLayout, 
    PlatformDisplay, PlatformKeyboardLayout, WindowParams,
};

use super::TexturedSurfaceWindow;

pub struct TexturedSurfaceClientState {
    pub(crate) loop_handle: LoopHandle<'static, TexturedSurfaceClient>,
    pub(crate) event_loop: Option<calloop::EventLoop<'static, TexturedSurfaceClient>>,
    pub(crate) common: LinuxCommon,
    pub(crate) windows: Vec<Rc<RefCell<TexturedSurfaceWindowState>>>,
}

#[derive(Clone)]
pub(crate) struct TexturedSurfaceClient(Rc<RefCell<TexturedSurfaceClientState>>);

impl TexturedSurfaceClient {
    pub(crate) fn new() -> Self {
        let event_loop = EventLoop::try_new().unwrap();
        let (common, main_receiver) = LinuxCommon::new(event_loop.get_signal());
        let handle = event_loop.handle();

        handle
            .insert_source(main_receiver, |event, _, _: &mut TexturedSurfaceClient| {
                if let calloop::channel::Event::Msg(runnable) = event {
                    runnable.run();
                }
            })
            .ok();

        TexturedSurfaceClient(Rc::new(RefCell::new(TexturedSurfaceClientState {
            event_loop: Some(event_loop),
            loop_handle: handle,
            common,
            windows: Vec::new(),
        })))
    }
}

impl LinuxClient for TexturedSurfaceClient {
    fn with_common<R>(&self, f: impl FnOnce(&mut LinuxCommon) -> R) -> R {
        f(&mut self.0.borrow_mut().common)
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(LinuxKeyboardLayout::new("us".into()))
    }

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        // Return a virtual display for layout purposes
        vec![Rc::new(TexturedSurfaceDisplay::new())]
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(TexturedSurfaceDisplay::new()))
    }

    fn display(&self, _id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(TexturedSurfaceDisplay::new()))
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        params: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        // Create a window that renders to texture!
        let window = TexturedSurfaceWindow::new(handle, params, self.clone())?;
        Ok(Box::new(window))
    }

    fn compositor_name(&self) -> &'static str {
        "textured_surface"
    }

    // Most other methods are no-ops (clipboard, cursor, etc.)
    fn set_cursor_style(&self, _style: CursorStyle) {}
    fn open_uri(&self, _uri: &str) {}
    fn reveal_path(&self, _path: std::path::PathBuf) {}
    fn write_to_primary(&self, _item: crate::ClipboardItem) {}
    fn write_to_clipboard(&self, _item: crate::ClipboardItem) {}
    fn read_from_primary(&self) -> Option<crate::ClipboardItem> { None }
    fn read_from_clipboard(&self) -> Option<crate::ClipboardItem> { None }

    fn run(&self) {
        let mut event_loop = self
            .0
            .borrow_mut()
            .event_loop
            .take()
            .expect("App is already running");

        event_loop.run(None, &mut self.clone(), |_| {}).ok();
    }
}
```

### 2. TexturedSurfaceWindow

A window that renders to a GPU texture instead of a display surface:

```rust
// In gpui/src/platform/linux/textured_surface/window.rs

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blade_graphics as gpu;
use raw_window_handle as rwh;

use crate::{
    AnyWindowHandle, Bounds, DevicePixels, GpuSpecs, Pixels, PlatformAtlas,
    PlatformInputHandler, PlatformWindow, Point, PromptLevel, RequestFrameOptions,
    Scene, Size, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowParams, px,
};
use crate::platform::blade::{BladeAtlas, BladeContext};

use super::TexturedSurfaceClient;

pub struct TexturedSurfaceWindowState {
    handle: AnyWindowHandle,
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    
    // GPU resources
    gpu: Arc<gpu::Context>,
    render_target: gpu::Texture,
    render_target_view: gpu::TextureView,
    
    // Renderer components (shared with BladeRenderer)
    atlas: Arc<BladeAtlas>,
    command_encoder: gpu::CommandEncoder,
    pipelines: BladePipelines,
    instance_belt: BufferBelt,
    
    // Path rendering intermediate textures
    path_intermediate_texture: gpu::Texture,
    path_intermediate_texture_view: gpu::TextureView,
    
    // Rendered pixels (available after draw)
    rendered_pixels: Option<Vec<u8>>,
    
    // Callbacks (mostly unused for texture rendering)
    input_handler: Option<PlatformInputHandler>,
}

pub(crate) struct TexturedSurfaceWindow(Rc<RefCell<TexturedSurfaceWindowState>>);

impl TexturedSurfaceWindow {
    pub fn new(
        handle: AnyWindowHandle,
        params: WindowParams,
        _client: TexturedSurfaceClient,
    ) -> anyhow::Result<Self> {
        // Initialize GPU context without a window surface
        let gpu = Arc::new(gpu::Context::init(gpu::ContextDesc {
            validation: cfg!(debug_assertions),
            capture: false,
            overlay: false,
        })?);
        
        let size = params.bounds.size;
        let device_size = gpu::Extent {
            width: size.width.0 as u32,
            height: size.height.0 as u32,
            depth: 1,
        };
        
        // Create render target texture (what we render TO)
        let render_target = gpu.create_texture(gpu::TextureDesc {
            name: "textured_surface_target",
            format: gpu::TextureFormat::Rgba8UnormSrgb,
            size: device_size,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::TARGET | gpu::TextureUsage::COPY,
        });
        
        let render_target_view = gpu.create_texture_view(
            render_target,
            gpu::TextureViewDesc {
                name: "textured_surface_target_view",
                format: gpu::TextureFormat::Rgba8UnormSrgb,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );
        
        // Create command encoder
        let command_encoder = gpu.create_command_encoder(gpu::CommandEncoderDesc {
            name: "textured_surface",
            buffer_count: 2,
        });
        
        // Create atlas
        let atlas = Arc::new(BladeAtlas::new(&gpu));
        
        // Create pipelines - need SurfaceInfo equivalent for format
        let surface_info = gpu::SurfaceInfo {
            format: gpu::TextureFormat::Rgba8UnormSrgb,
            alpha: gpu::AlphaMode::PreMultiplied,
        };
        let pipelines = BladePipelines::new(&gpu, surface_info, 4); // 4x MSAA
        
        // Create instance belt
        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40,
        });
        
        // Create path intermediate textures
        let (path_intermediate_texture, path_intermediate_texture_view) =
            create_path_intermediate_texture(
                &gpu,
                gpu::TextureFormat::Rgba8UnormSrgb,
                device_size.width,
                device_size.height,
            );
        
        Ok(Self(Rc::new(RefCell::new(TexturedSurfaceWindowState {
            handle,
            bounds: params.bounds,
            scale_factor: 1.0,
            gpu,
            render_target,
            render_target_view,
            atlas,
            command_encoder,
            pipelines,
            instance_belt,
            path_intermediate_texture,
            path_intermediate_texture_view,
            rendered_pixels: None,
            input_handler: None,
        }))))
    }
    
    /// Read the rendered pixels after draw()
    pub fn read_pixels(&self) -> Option<Vec<u8>> {
        self.0.borrow().rendered_pixels.clone()
    }
    
    /// Get the render target texture view (for 3D embedding)
    pub fn texture_view(&self) -> gpu::TextureView {
        self.0.borrow().render_target_view
    }
}

impl PlatformWindow for TexturedSurfaceWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().bounds
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Windowed(self.bounds())
    }

    fn is_maximized(&self) -> bool {
        false
    }

    fn content_size(&self) -> Size<Pixels> {
        self.bounds().size
    }

    fn scale_factor(&self) -> f32 {
        self.0.borrow().scale_factor
    }

    fn appearance(&self) -> WindowAppearance {
        WindowAppearance::Dark
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(TexturedSurfaceDisplay::new()))
    }

    fn mouse_position(&self) -> Point<Pixels> {
        Point::zero()
    }

    fn modifiers(&self) -> crate::Modifiers {
        Default::default()
    }

    fn set_input_handler(&mut self, handler: PlatformInputHandler) {
        self.0.borrow_mut().input_handler = Some(handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[&str],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {}
    fn is_active(&self) -> bool { true }
    fn is_hovered(&self) -> bool { false }
    fn set_title(&mut self, _title: &str) {}
    fn set_background_appearance(&mut self, _appearance: WindowBackgroundAppearance) {}
    fn minimize(&self) {}
    fn zoom(&self) {}
    fn toggle_fullscreen(&self) {}
    fn is_fullscreen(&self) -> bool { false }

    fn on_request_frame(&self, _callback: Box<dyn FnMut(RequestFrameOptions)>) {}
    fn on_input(&self, _callback: Box<dyn FnMut(crate::PlatformInput) -> crate::DispatchEventResult>) {}
    fn on_active_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}
    fn on_hover_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}
    fn on_resize(&self, _callback: Box<dyn FnMut(Size<Pixels>, f32)>) {}
    fn on_moved(&self, _callback: Box<dyn FnMut()>) {}
    fn on_should_close(&self, _callback: Box<dyn FnMut() -> bool>) {}
    fn on_close(&self, _callback: Box<dyn FnOnce()>) {}
    fn on_appearance_changed(&self, _callback: Box<dyn FnMut()>) {}

    fn draw(&self, scene: &Scene) {
        let mut state = self.0.borrow_mut();
        
        // Render scene to our texture target instead of a surface
        state.render_scene_to_texture(scene);
        
        // Read pixels back for later retrieval
        state.read_pixels_to_buffer();
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.borrow().atlas.clone()
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        let state = self.0.borrow();
        let device_info = state.gpu.device_information();
        Some(GpuSpecs {
            is_software_emulated: device_info.device_type == gpu::DeviceType::Cpu,
            device_name: device_info.name.clone(),
            driver_name: device_info.driver_name.clone(),
            driver_info: device_info.driver_info.clone(),
        })
    }
    
    // ... other PlatformWindow methods with default/no-op implementations
}

impl TexturedSurfaceWindowState {
    fn render_scene_to_texture(&mut self, scene: &Scene) {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);
        
        // Initialize render target
        self.command_encoder.init_texture(self.render_target);
        
        let globals = GlobalParams {
            viewport_size: [
                self.bounds.size.width.0,
                self.bounds.size.height.0,
            ],
            premultiplied_alpha: 1,
            pad: 0,
        };
        
        // Render to texture instead of surface
        let mut pass = self.command_encoder.render(
            "textured_surface_main",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: self.render_target_view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );
        
        // Render all batches (same logic as BladeRenderer::draw)
        for batch in scene.batches() {
            self.render_batch(&mut pass, batch, globals);
        }
        
        drop(pass);
        
        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);
        
        // Wait for GPU to finish
        self.gpu.wait_for(&sync_point, 10000);
    }
    
    fn render_batch(
        &mut self,
        pass: &mut gpu::RenderCommandEncoder,
        batch: &PrimitiveBatch,
        globals: GlobalParams,
    ) {
        // Same implementation as BladeRenderer - render quads, shadows, paths, etc.
        // This could be refactored to share code with BladeRenderer
    }
    
    fn read_pixels_to_buffer(&mut self) {
        let size = self.bounds.size;
        let bytes_per_pixel = 4u64;
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
                    texture: self.render_target,
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
        
        // Read pixels
        let pixels = unsafe {
            std::slice::from_raw_parts(
                staging.data() as *const u8,
                buffer_size as usize,
            ).to_vec()
        };
        
        self.gpu.destroy_buffer(staging);
        self.rendered_pixels = Some(pixels);
    }
}
```

### 3. Integration with Application

```rust
// In gpui/src/app.rs - add new constructor

impl Application {
    /// Build an app that renders to textures instead of display surfaces.
    /// 
    /// This mode supports opening windows and full GPUI rendering, but
    /// renders to GPU textures that can be read back as pixels.
    /// 
    /// Use this for:
    /// - One-shot rendering to PNG/images
    /// - Generating thumbnails
    /// - Embedding GPUI in 3D environments
    /// - Visual testing
    #[cfg(target_os = "linux")]
    pub fn textured() -> Self {
        Self(App::new_app(
            Rc::new(TexturedSurfaceClient::new()),
            Arc::new(()),
            Arc::new(NullHttpClient),
        ))
    }
}
```

### 4. Platform Selection Update

```rust
// In gpui/src/platform.rs

#[cfg(target_os = "linux")]
pub(crate) fn current_platform(headless: bool) -> Rc<dyn Platform> {
    if headless {
        return Rc::new(HeadlessClient::new());  // Unchanged!
    }
    
    match guess_compositor() {
        #[cfg(feature = "wayland")]
        "Wayland" => Rc::new(WaylandClient::new()),
        #[cfg(feature = "x11")]
        _ => Rc::new(X11Client::new().expect("Failed to create X11 client")),
    }
}

// NEW: Separate function for textured mode
#[cfg(target_os = "linux")]
pub(crate) fn textured_platform() -> Rc<dyn Platform> {
    Rc::new(TexturedSurfaceClient::new())
}
```

---

## Extending BladeRenderer (Alternative Approach)

Instead of duplicating rendering code in `TexturedSurfaceWindow`, we could extend `BladeRenderer` to support texture targets:

```rust
// In gpui/src/platform/blade/blade_renderer.rs

impl BladeRenderer {
    // Existing: renders to window surface
    pub fn draw(&mut self, scene: &Scene) { ... }
    
    // NEW: renders to a texture target
    pub fn draw_to_texture(
        &mut self,
        scene: &Scene,
        target: &RenderTarget,
    ) {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);
        self.command_encoder.init_texture(target.texture);
        
        let globals = GlobalParams {
            viewport_size: [target.size.width as f32, target.size.height as f32],
            premultiplied_alpha: 1,
            pad: 0,
        };
        
        // Use the same rendering code but target the texture
        let mut pass = self.command_encoder.render(
            "render_to_texture",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: target.view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );
        
        self.render_batches(&mut pass, scene, globals);
        
        drop(pass);
        
        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);
    }
    
    // NEW: read pixels from a render target
    pub fn read_pixels(&mut self, target: &RenderTarget) -> Vec<u8> {
        // ... pixel readback implementation
    }
    
    // Refactored: shared batch rendering logic
    fn render_batches(
        &mut self,
        pass: &mut gpu::RenderCommandEncoder,
        scene: &Scene,
        globals: GlobalParams,
    ) {
        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Quads(quads) => { ... }
                PrimitiveBatch::Shadows(shadows) => { ... }
                // ... etc
            }
        }
    }
}

pub struct RenderTarget {
    pub texture: gpu::Texture,
    pub view: gpu::TextureView,
    pub size: Size<u32>,
}
```

---

## Public API

```rust
// In gpui/src/lib.rs or gpui/src/textured.rs

/// Render a GPUI view to pixels without displaying a window
pub fn render_to_pixels<V: Render>(
    size: Size<DevicePixels>,
    scale_factor: f32,
    asset_source: impl AssetSource,
    build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
) -> anyhow::Result<Vec<u8>> {
    let app = Application::textured().with_assets(asset_source);
    let result = Rc::new(RefCell::new(None));
    let result_clone = Rc::clone(&result);
    
    app.run(move |cx| {
        let bounds = Bounds::new(
            Point::zero(),
            Size {
                width: px(size.width.0 as f32 / scale_factor),
                height: px(size.height.0 as f32 / scale_factor),
            },
        );
        
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| build_view(window, cx)),
        )?;
        
        window.update(cx, |_, window, cx| {
            window.draw(cx);
        })?;
        
        // Get pixels from the textured window
        let pixels = window.read_with(cx, |_, window| {
            window.platform_window
                .as_any()
                .downcast_ref::<TexturedSurfaceWindow>()
                .and_then(|w| w.read_pixels())
        })?.ok_or_else(|| anyhow!("Failed to read pixels"))?;
        
        *result_clone.borrow_mut() = Some(pixels);
        cx.quit();
        Ok(())
    });
    
    result.borrow_mut().take().ok_or_else(|| anyhow!("Rendering failed"))
}

/// Render a GPUI view to a PNG file
pub fn render_to_png<V: Render>(
    size: Size<DevicePixels>,
    scale_factor: f32,
    asset_source: impl AssetSource,
    build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    path: impl AsRef<Path>,
) -> anyhow::Result<()> {
    let pixels = render_to_pixels(size, scale_factor, asset_source, build_view)?;
    
    let img = image::RgbaImage::from_raw(
        size.width.0 as u32,
        size.height.0 as u32,
        pixels,
    ).ok_or_else(|| anyhow!("Failed to create image"))?;
    
    img.save(path)?;
    Ok(())
}
```

---

## File Structure

```
crates/gpui/src/platform/linux/
├── mod.rs                          # Add: pub mod textured_surface
├── headless/                       # UNCHANGED
│   └── client.rs
├── textured_surface/               # NEW
│   ├── mod.rs
│   ├── client.rs                   # TexturedSurfaceClient
│   ├── window.rs                   # TexturedSurfaceWindow
│   └── display.rs                  # TexturedSurfaceDisplay (virtual)
├── wayland/
│   └── ...
└── x11/
    └── ...

crates/gpui/src/platform/blade/
├── blade_renderer.rs               # MODIFIED: add draw_to_texture(), read_pixels()
└── ...

crates/gpui/src/
├── app.rs                          # MODIFIED: add Application::textured()
├── platform.rs                     # MODIFIED: add textured_platform()
└── lib.rs                          # MODIFIED: export render_to_pixels(), render_to_png()
```

---

## Implementation Steps

### Phase 1: Basic Infrastructure
1. Create `textured_surface/` directory structure
2. Implement `TexturedSurfaceClient` (minimal, based on HeadlessClient)
3. Implement `TexturedSurfaceDisplay` (virtual display)
4. Add `Application::textured()` constructor

### Phase 2: Window and Rendering
1. Implement `TexturedSurfaceWindow` with GPU context
2. Add render target texture creation
3. Implement `draw()` to render scene to texture
4. Implement pixel readback

### Phase 3: BladeRenderer Integration
1. Refactor `BladeRenderer::draw()` to extract shared `render_batches()`
2. Add `draw_to_texture()` method
3. Add `read_pixels()` method
4. Update `TexturedSurfaceWindow` to use shared code

### Phase 4: Public API
1. Add `render_to_pixels()` function
2. Add `render_to_png()` function
3. Add documentation and examples
4. Add tests

---

## Comparison with Previous Approach

| Aspect | Modify HeadlessClient | New TexturedSurfaceClient |
|--------|----------------------|---------------------------|
| HeadlessClient changes | Yes | No |
| Risk to existing tools | Medium | None |
| Code separation | Mixed concerns | Clean separation |
| Semantic clarity | Confusing ("headless" with windows?) | Clear ("textured surface") |
| Implementation effort | Lower | Slightly higher |
| Future extensibility | Limited | Good (can add features) |

---

## Future: 3D Embedding

The `TexturedSurfaceWindow` design naturally supports 3D embedding:

```rust
// Get the texture directly for use in a 3D engine
let texture_view = textured_window.texture_view();

// In your 3D render loop:
// 1. Inject input events into the GPUI window
textured_window.inject_mouse_event(transformed_position, button);

// 2. Update GPUI if dirty
window.update(cx, |_, window, cx| {
    if window.needs_present() {
        window.draw(cx);
    }
});

// 3. Sample the texture in your 3D shader
// The texture_view can be bound to your 3D pipeline
```

For continuous 3D embedding (not one-shot), the window would:
- Keep the GPU context alive
- Not read pixels back to CPU (expensive)
- Allow direct texture sampling by the 3D engine
- Support input injection for interactivity

---

## Summary

### What We're Building
A new `TexturedSurfaceClient` and `TexturedSurfaceWindow` platform for Linux that:
- Renders GPUI content to GPU textures instead of display surfaces
- Supports pixel readback for PNG export
- Leaves existing `HeadlessClient` completely unchanged
- Can be extended for 3D embedding use cases

### Why This Approach
- **No risk** to existing headless tools (CLI, benchmarks, remote server)
- **Clean semantics**: "headless" means no windows, "textured surface" means render to texture
- **Re-uses BladeRenderer**: Minimal code duplication
- **Extensible**: Easy to add features like continuous rendering, texture sharing

### Key Files
- `gpui/src/platform/linux/textured_surface/client.rs` - Platform client
- `gpui/src/platform/linux/textured_surface/window.rs` - Window with texture target
- `gpui/src/platform/blade/blade_renderer.rs` - Extended with `draw_to_texture()`
- `gpui/src/app.rs` - `Application::textured()` constructor

---

## References

- [Full Architecture Analysis](./one-shot-rendering-architecture.md)
- [Original Commit Critique](./render-to-texture-critique.md)
- [3D Embedding Architecture](./embedding-gpui-in-3d.md)