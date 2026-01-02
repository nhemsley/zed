# Solution: One-Shot Rendering for GPUI

**Date**: Based on GPUI main branch (commit 392c78ea5d)  
**Status**: Architecture Defined, Ready for Implementation

---

## TL;DR

GPUI already has `Application::headless()` as **public API**. We just need to:
1. Add `HeadlessWindow` support to headless platforms
2. Create `HeadlessBladeRenderer` that renders `Scene` to texture
3. Extract scene after `window.draw()` and render to pixels

No feature flags needed. Minimal GPUI changes required.

---

## The Problem

GPUI's element system requires concrete `Window` and `App` types:
```rust
fn paint(&mut self, window: &mut Window, cx: &mut App);
```

You cannot substitute your own types because these aren't traits - they're concrete structs.

---

## The Solution: Extend Application::headless()

### Discovery: Headless Mode Already Exists

```rust
// This is ALREADY public API in gpui/src/app.rs!
impl Application {
    pub fn headless() -> Self {
        Self(App::new_app(
            current_platform(true),  // On Linux, returns HeadlessClient
            Arc::new(()),
            Arc::new(NullHttpClient),
        ))
    }
}
```

On Linux, `HeadlessClient` already provides:
- Event loop without display
- No-op clipboard, cursor, etc.
- Full text system
- Background/foreground executors

**The only missing piece:** `HeadlessClient::open_window()` currently bails instead of creating a window.

---

## Implementation Plan

### Step 1: Add HeadlessWindow (Linux)

```rust
// In gpui/src/platform/linux/headless/window.rs (NEW FILE)

pub(crate) struct HeadlessWindow {
    handle: AnyWindowHandle,
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    sprite_atlas: Arc<dyn PlatformAtlas>,
}

impl PlatformWindow for HeadlessWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
    
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
    
    // Most methods are no-ops
    fn draw(&self, _scene: &Scene) {
        // No-op: we extract scene manually for rendering
    }
    
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.sprite_atlas.clone()
    }
    
    // ... other PlatformWindow methods (mostly no-ops)
}
```

### Step 2: Update HeadlessClient

```rust
// In gpui/src/platform/linux/headless/client.rs

impl LinuxClient for HeadlessClient {
    fn open_window(
        &self,
        handle: AnyWindowHandle,
        params: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        // CHANGE: Instead of bailing, create a HeadlessWindow
        Ok(Box::new(HeadlessWindow::new(handle, params)))
    }
}
```

### Step 3: Create HeadlessBladeRenderer

```rust
// In gpui/src/platform/blade/headless_renderer.rs (NEW FILE)

/// GPU renderer that works without a window surface
pub struct HeadlessBladeRenderer {
    gpu: Arc<gpu::Context>,
    command_encoder: gpu::CommandEncoder,
    pipelines: BladePipelines,
    instance_belt: BufferBelt,
    atlas: Arc<BladeAtlas>,
    // ... other fields
}

impl HeadlessBladeRenderer {
    pub fn new() -> Result<Self> {
        // Create GPU context without window
        let gpu = Arc::new(gpu::Context::init(gpu::ContextDesc {
            validation: cfg!(debug_assertions),
            capture: false,
            overlay: false,
        })?);
        
        // Initialize pipelines, atlas, etc.
        // ...
    }
    
    pub fn render_scene_to_pixels(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<Vec<u8>> {
        // 1. Create render target texture
        let target = self.create_render_target(size)?;
        
        // 2. Render scene to texture
        self.render_scene(&target, scene)?;
        
        // 3. Read pixels back to CPU
        self.read_pixels(&target)
    }
}
```

### Step 4: Public API

```rust
// In gpui/src/lib.rs or new gpui/src/headless.rs

pub struct OneShotRenderer {
    app: Application,
    renderer: HeadlessBladeRenderer,
}

impl OneShotRenderer {
    pub fn new(asset_source: impl AssetSource) -> Result<Self> {
        let app = Application::headless().with_assets(asset_source);
        let renderer = HeadlessBladeRenderer::new()?;
        Ok(Self { app, renderer })
    }
    
    pub fn render_to_pixels<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        scale_factor: f32,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Result<Vec<u8>> {
        let pixels = Rc::new(RefCell::new(None));
        let pixels_clone = Rc::clone(&pixels);
        
        self.app.run(|cx| {
            // Open headless window
            let window = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(
                        Bounds::new(Point::zero(), size.into())
                    )),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| build_view(window, cx))
            )?;
            
            // Trigger draw cycle
            window.update(cx, |window, cx| {
                window.draw(cx);
                
                // Extract scene
                let scene = std::mem::take(&mut window.rendered_frame.scene);
                
                // Render to pixels
                let result = self.renderer.render_scene_to_pixels(&scene, size)?;
                *pixels_clone.borrow_mut() = Some(result);
                Ok(())
            })
        })?;
        
        pixels.borrow_mut().take().ok_or_else(|| anyhow!("Rendering failed"))
    }
    
    pub fn render_to_png<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        scale_factor: f32,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let pixels = self.render_to_pixels(size, scale_factor, build_view)?;
        let img = image::RgbaImage::from_raw(
            size.width.0 as u32,
            size.height.0 as u32,
            pixels
        ).ok_or_else(|| anyhow!("Failed to create image"))?;
        img.save(path)?;
        Ok(())
    }
}
```

---

## Usage Example

```rust
use gpui::{OneShotRenderer, div, rgb, Render, Styled, ParentElement};

fn main() -> anyhow::Result<()> {
    let mut renderer = OneShotRenderer::new(MyAssetSource)?;
    
    renderer.render_to_png(
        Size::new(DevicePixels(800), DevicePixels(600)),
        2.0, // scale factor
        |window, cx| {
            // Build your view here - full GPUI API available
            MyView::new(cx)
        },
        "output.png",
    )?;
    
    Ok(())
}

struct MyView;

impl Render for MyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e2127))
            .child("Hello, One-Shot Rendering!")
    }
}
```

---

## Platform Support

### Linux (Primary Implementation)
- ✅ `HeadlessClient` already exists
- ✅ `Application::headless()` works
- 🔲 Need to add `HeadlessWindow`
- 🔲 Need to add `HeadlessBladeRenderer`

### macOS
- ✅ `Application::headless()` exists
- 🔲 Need to add `HeadlessWindow` for macOS
- 🔲 May need Metal-specific headless renderer

### Windows
- ✅ `Application::headless()` exists
- 🔲 Need to add `HeadlessWindow` for Windows
- 🔲 May need DirectX-specific headless renderer

---

## Files to Modify/Create

### New Files
- `crates/gpui/src/platform/linux/headless/window.rs` - HeadlessWindow impl
- `crates/gpui/src/platform/blade/headless_renderer.rs` - HeadlessBladeRenderer
- `crates/gpui/src/headless.rs` - Public OneShotRenderer API

### Modified Files
- `crates/gpui/src/platform/linux/headless/client.rs` - open_window() impl
- `crates/gpui/src/platform/linux/headless/mod.rs` - export HeadlessWindow
- `crates/gpui/src/lib.rs` - export headless module

---

## Alternative: Use Test Infrastructure

If modifying headless mode is not desired, can use existing test infrastructure:

```rust
// Requires enabling "test-support" feature
pub struct OneShotRenderer {
    cx: TestAppContext,
    renderer: HeadlessBladeRenderer,
}

impl OneShotRenderer {
    pub fn new() -> Result<Self> {
        let dispatcher = TestDispatcher::new(StdRng::seed_from_u64(0));
        let cx = TestAppContext::build(dispatcher, None);
        let renderer = HeadlessBladeRenderer::new()?;
        Ok(Self { cx, renderer })
    }
    // ... similar API
}
```

**Trade-off**: Requires `test-support` feature (pulls in test dependencies).

---

## Benefits of This Approach

1. **Minimal Changes**: Only adds HeadlessWindow, doesn't modify core GPUI
2. **No Feature Flags**: Uses existing public `Application::headless()` API
3. **Full Compatibility**: Real Window and App, all elements work
4. **Battle-Tested Foundation**: Builds on existing headless infrastructure
5. **Clean Architecture**: Separates rendering from display concerns
6. **Platform Extensible**: Can add to macOS/Windows following same pattern

---

## References

- [Full Architecture Analysis](./one-shot-rendering-architecture.md)
- [Original Commit Critique](./render-to-texture-critique.md)
- [3D Embedding Architecture](./embedding-gpui-in-3d.md)

**Key GPUI Files:**
- `gpui/src/app.rs` - `Application::headless()`
- `gpui/src/platform/linux/headless/client.rs` - `HeadlessClient`
- `gpui/src/platform/test/window.rs` - `TestWindow` (reference impl)
- `gpui/src/platform/blade/blade_renderer.rs` - GPU rendering