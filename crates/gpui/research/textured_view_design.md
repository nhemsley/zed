# TexturedView Design

> A GPUI View for rendering elements to GPU textures in background threads.

## Overview

`TexturedView` is a GPUI View that renders arbitrary GPUI content to a texture
using `Application::textured()` on a background thread. The resulting pixels
are streamed back to the main thread and displayed as an image.

## Why a View, not an Element?

`TexturedView` needs:
- **Persistent state** - thread handle, channel, cached texture
- **Async updates** - receive frames from background thread
- **Lifecycle management** - spawn thread on creation, cleanup on drop
- **Notification** - trigger repaint when new frames arrive

These requirements make it a **View** (stateful, entity-backed) rather than an
**Element** (stateless, recreated each frame).

| Aspect | Element | View | TexturedView |
|--------|---------|------|--------------|
| State | Stateless | Stateful | ✅ Needs state |
| Identity | Optional ElementId | EntityId | ✅ Needs identity |
| Lifecycle | Per-frame | Persists | ✅ Thread lifecycle |
| Async | Awkward | Natural | ✅ Channel polling |

## Why in GPUI?

This belongs in gpui-proper (not infinite-canvas) because:

1. **Fundamental primitive** - Element → texture is a general rendering capability
2. **Uses GPUI internals** - `Application::textured()`, `Window::resize()`, layout system
3. **Reusable** - Other crates beyond infinite-canvas may need offscreen rendering
4. **Platform abstraction** - Handles Linux/FreeBSD-specific textured surface code

## API Design

### Basic Usage

```rust
use gpui::TexturedView;

// Simplest case: fixed size, render once
let view = cx.new(|cx| {
    TexturedView::new(size(px(300.), px(200.)), cx, || {
        div()
            .bg(rgb(0x3498db))
            .size_full()
            .child("Hello, Texture!")
    })
});

// With ItemSizing for measured height
let view = cx.new(|cx| {
    TexturedView::measured(px(300.), cx, || {
        div()
            .p_4()
            .child("Content that determines height")
    })
});

// Streaming mode (continuous updates)
let view = cx.new(|cx| {
    TexturedView::streaming(size(px(400.), px(300.)), cx, move || {
        // This closure is called each frame by the background thread
        animated_content()
    })
});
```

### Core Struct

```rust
/// A View that renders content to a GPU texture in a background thread.
///
/// The content is rendered using `Application::textured()` and the resulting
/// pixels are streamed back and displayed as an image.
pub struct TexturedView<F> {
    /// Function that creates the element to render
    render_fn: F,
    /// How to determine the texture size
    sizing: ItemSizing,
    /// Rendering mode
    mode: RenderMode,
    /// Channel to receive rendered frames
    frame_receiver: Receiver<RenderedFrame>,
    /// Handle to background render thread
    thread_handle: Option<JoinHandle<()>>,
    /// Current texture (latest frame)
    current_texture: Option<Arc<RenderImage>>,
    /// Measured size (for FixedWidth mode)
    measured_size: Option<Size<Pixels>>,
    /// Error state
    error: Option<TextureError>,
}

/// How to determine texture dimensions
pub enum ItemSizing {
    /// Fixed dimensions - no measurement needed
    Fixed { size: Size<Pixels> },
    
    /// Fixed width, height measured from content.
    /// Uses GPUI's layout system to measure before rendering.
    /// `estimated_height` used for layout before measurement completes.
    FixedWidth {
        width: Pixels,
        estimated_height: Pixels,
    },
    
    /// Caller provides size explicitly per-instance
    Explicit { size: Size<Pixels> },
}

/// How often to re-render
pub enum RenderMode {
    /// Render once, cache result
    Once,
    /// Continuously stream frames
    Streaming { target_fps: u32 },
}

/// Frame data sent from background thread
struct RenderedFrame {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}
```

### View Implementation

```rust
impl<F, E> TexturedView<F>
where
    F: Fn() -> E + Send + Clone + 'static,
    E: IntoElement,
{
    /// Create a new TexturedView with fixed size
    pub fn new(size: Size<Pixels>, cx: &mut Context<Self>, render_fn: F) -> Self {
        Self::with_sizing(ItemSizing::Fixed { size }, RenderMode::Once, cx, render_fn)
    }
    
    /// Create a TexturedView with measured height
    pub fn measured(width: Pixels, cx: &mut Context<Self>, render_fn: F) -> Self {
        Self::with_sizing(
            ItemSizing::FixedWidth { width, estimated_height: px(200.) },
            RenderMode::Once,
            cx,
            render_fn,
        )
    }
    
    /// Create a streaming TexturedView
    pub fn streaming(size: Size<Pixels>, cx: &mut Context<Self>, render_fn: F) -> Self {
        Self::with_sizing(ItemSizing::Fixed { size }, RenderMode::Streaming { target_fps: 30 }, cx, render_fn)
    }
    
    fn with_sizing(sizing: ItemSizing, mode: RenderMode, cx: &mut Context<Self>, render_fn: F) -> Self {
        let (sender, receiver) = flume::bounded(4);
        
        // Spawn background render thread
        let thread_handle = spawn_render_thread(sizing.clone(), mode.clone(), render_fn.clone(), sender);
        
        Self {
            render_fn,
            sizing,
            mode,
            frame_receiver: receiver,
            thread_handle: Some(thread_handle),
            current_texture: None,
            measured_size: None,
            error: None,
        }
    }
    
    /// Force re-render (invalidate cached texture)
    pub fn invalidate(&mut self, cx: &mut Context<Self>) {
        // Kill existing thread, spawn new one
        // ...
        cx.notify();
    }
    
    /// Poll for new frames from background thread
    fn poll_frames(&mut self, cx: &mut Context<Self>) {
        while let Ok(frame) = self.frame_receiver.try_recv() {
            // Convert BGRA to RGBA
            let mut rgba = frame.pixels;
            for chunk in rgba.chunks_exact_mut(4) {
                chunk.swap(0, 2);
            }
            
            if let Some(buffer) = RgbaImage::from_raw(frame.width, frame.height, rgba) {
                self.current_texture = Some(Arc::new(RenderImage::new(smallvec![Frame::new(buffer)])));
                self.measured_size = Some(size(px(frame.width as f32), px(frame.height as f32)));
            }
        }
        
        // For streaming mode, schedule next poll
        if matches!(self.mode, RenderMode::Streaming { .. }) {
            cx.notify();
        }
    }
}

impl<F, E> Render for TexturedView<F>
where
    F: Fn() -> E + Send + Clone + 'static,
    E: IntoElement,
{
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Poll for new frames
        self.poll_frames(cx);
        
        // Display current texture or placeholder
        if let Some(texture) = &self.current_texture {
            img(texture.clone())
                .size_full()
                .into_any_element()
        } else if let Some(error) = &self.error {
            div()
                .size_full()
                .bg(rgb(0xff0000))
                .child(format!("Error: {:?}", error))
                .into_any_element()
        } else {
            // Loading placeholder
            div()
                .size_full()
                .bg(rgb(0x333333))
                .child("Loading...")
                .into_any_element()
        }
    }
}

impl<F> Drop for TexturedView<F> {
    fn drop(&mut self) {
        // Signal background thread to stop and wait for it
        if let Some(handle) = self.thread_handle.take() {
            // Thread will quit when sender is dropped
            let _ = handle.join();
        }
    }
}
```

## Internal Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Main Thread                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              TexturedView (Entity)                   │   │
│  │  - Owns thread handle, channel receiver             │   │
│  │  - Polls for frames in render()                     │   │
│  │  - Displays texture via img()                       │   │
│  │  - Cleans up thread in Drop                         │   │
│  └─────────────────────────────────────────────────────┘   │
│                           │                                 │
│                           │ flume channel                   │
│                           ▼                                 │
└─────────────────────────────────────────────────────────────┘
                           │
                           │ RenderedFrame { pixels, size }
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              Background Render Thread                       │
│  ┌─────────────────────────────────────────────────────┐   │
│  │            Application::textured()                   │   │
│  │                                                      │   │
│  │  1. Create window (initial size from ItemSizing)    │   │
│  │  2. For FixedWidth: measure → resize → render       │   │
│  │  3. For Fixed: render directly                      │   │
│  │  4. read_pixels(), send via channel                 │   │
│  │  5. Quit (Once) or loop (Streaming)                 │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Measurement Flow (FixedWidth Mode)

```
Background Thread:
┌─────────────────────────────────────────────────────────────┐
│ 1. Create window at (width × estimated_height)              │
│ 2. Create element from render closure                       │
│ 3. layout_as_root(Definite(width), MinContent)             │
│    → Returns actual Size<Pixels>                            │
│ 4. window.resize(actual_size)                               │
│    → Recreates GPU textures at correct dimensions           │
│ 5. Re-layout at actual size                                │
│ 6. prepaint + paint                                         │
│ 7. read_pixels() → send to main thread                     │
└─────────────────────────────────────────────────────────────┘
```

This works because we added `resize()` support to `TexturedSurfaceWindow`:
- `gpui/src/platform/linux/textured_surface/window.rs`
- Destroys old GPU textures, creates new ones at new size

## State Machine

```
                    ┌──────────────┐
         new()      │   Pending    │
        ───────────►│  (no frame)  │
                    └──────┬───────┘
                           │ frame received
                           ▼
                    ┌──────────────┐
                    │    Ready     │◄────┐
                    │ (has frame)  │     │ new frame (streaming)
                    └──────┬───────┘─────┘
                           │
                           │ invalidate()
                           ▼
                    ┌──────────────┐
                    │  Rerendering │
                    │  (pending)   │
                    └──────────────┘
```

## Error Handling

```rust
pub enum TextureError {
    /// Platform doesn't support textured rendering
    UnsupportedPlatform,
    /// GPU initialization failed
    GpuInitFailed(String),
    /// Render closure panicked
    RenderPanic,
    /// Background thread died unexpectedly
    ThreadDied,
}
```

Fallback behavior:
1. Show placeholder/error element
2. Log error
3. Optionally retry

## Platform Support

| Platform | Support | Notes |
|----------|---------|-------|
| Linux | ✅ | Full support via `Application::textured()` |
| FreeBSD | ✅ | Full support via `Application::textured()` |
| macOS | ❌ | Would need Metal-based textured surface |
| Windows | ❌ | Would need D3D-based textured surface |

On unsupported platforms, `TexturedView` could:
1. Render synchronously in main thread (slower but works)
2. Return error and show placeholder
3. Compile-time feature gate

## File Location

```
gpui/src/
├── elements/
│   ├── mod.rs
│   ├── div.rs
│   ├── img.rs
│   └── ...
└── textured_view.rs  ← NEW (or textured_view/ module)
```

## Usage from infinite-canvas

```rust
// infinite-canvas creates TexturedViews for canvas items
impl CanvasRenderer {
    fn create_item_view(&self, item: &CanvasItem, cx: &mut WindowContext) -> View<TexturedView<_>> {
        cx.new(|cx| {
            TexturedView::measured(item.width, cx, {
                let data = item.data.clone();
                let render_fn = self.render_fn.clone();
                move || render_fn(&data)
            })
        })
    }
}
```

## Comparison with multi_app_textured.rs

The existing `multi_app_textured.rs` example demonstrates the low-level pattern:
- Manual thread spawning
- Manual channel management
- Manual BGRA→RGBA conversion
- Custom `BackgroundRenderer` view

`TexturedView` encapsulates all of this into a reusable View:
- Automatic thread lifecycle
- Built-in channel handling
- Automatic pixel format conversion
- Works with any render closure

## Open Questions

1. **Thread pooling?**
   - Current: one thread per TexturedView
   - Alternative: global thread pool for all TexturedViews
   - Trade-off: simplicity vs resource efficiency

2. **Texture caching across invalidations?**
   - Keep old texture while re-rendering?
   - Fade transition between old and new?

3. **Size reporting to parent?**
   - For FixedWidth, measured size isn't known until render completes
   - How to update parent layout when size changes?

## Related Files

- `gpui/src/platform/linux/textured_surface/window.rs` - Resize support ✅
- `gpui/examples/multi_app_textured.rs` - Working streaming example ✅
- `gpui/research/infinite_canvas_textured_api.md` - Higher-level canvas API design
- `gpui/research/infinite_canvas_textured_impl.rs` - Implementation sketch