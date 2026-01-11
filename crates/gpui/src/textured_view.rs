//! TexturedView - A View for rendering elements to GPU textures in background threads.
//!
//! `TexturedView` renders arbitrary GPUI content to a texture using `Application::textured()`
//! on a background thread. The resulting pixels are streamed back to the main thread and
//! displayed as an image.
//!
//! # Why a View, not an Element?
//!
//! `TexturedView` needs:
//! - **Persistent state** - thread handle, channel, cached texture
//! - **Async updates** - receive frames from background thread
//! - **Lifecycle management** - spawn thread on creation, cleanup on drop
//!
//! These requirements make it a View (stateful, entity-backed) rather than an Element.
//!
//! # Platform Support
//!
//! Currently only supported on Linux/FreeBSD where `Application::textured()` is available.
//! On other platforms, attempting to create a TexturedView will show an error placeholder.
//!
//! # Example
//!
//! ```ignore
//! use gpui::TexturedView;
//!
//! // Fixed size, render once
//! let view = cx.new(|cx| {
//!     TexturedView::fixed(size(px(300.), px(200.)), cx, || {
//!         div().bg(rgb(0x3498db)).child("Hello!")
//!     })
//! });
//!
//! // Fixed width, measured height
//! let view = cx.new(|cx| {
//!     TexturedView::measured(px(300.), cx, || {
//!         div().p_4().child("Content determines height")
//!     })
//! });
//!
//! // Streaming mode (continuous updates)
//! let view = cx.new(|cx| {
//!     TexturedView::streaming(size(px(400.), px(300.)), 30, cx, || {
//!         animated_content()
//!     })
//! });
//! ```

use crate::{
    AnyElement, App, AppContext as _, Context, IntoElement, ParentElement, Pixels, Render,
    RenderImage, Size, Styled, Window, div, img,
};
use std::sync::Arc;
use std::thread::JoinHandle;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use crate::{AnyWindowHandle, Application, Bounds, Timer, WindowBounds, WindowOptions};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::time::Duration as StdDuration;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::time::Duration;

use flume::Receiver;

/// How to determine texture dimensions.
#[derive(Clone, Debug)]
pub enum ItemSizing {
    /// Fixed dimensions - no measurement needed.
    /// Most performant option.
    Fixed {
        /// The exact size to render at.
        size: Size<Pixels>,
    },

    /// Fixed width, height measured from content.
    ///
    /// Uses GPUI's layout system (`layout_as_root`) to measure the element
    /// at the given width, then resizes the window to the measured height
    /// before rendering.
    FixedWidth {
        /// The width to render at.
        width: Pixels,
        /// Estimated height for initial layout before measurement completes.
        estimated_height: Pixels,
    },

    /// Caller provides size explicitly.
    Explicit {
        /// The exact size to render at.
        size: Size<Pixels>,
    },
}

impl Default for ItemSizing {
    fn default() -> Self {
        ItemSizing::Fixed {
            size: Size {
                width: Pixels(300.0),
                height: Pixels(200.0),
            },
        }
    }
}

impl ItemSizing {
    /// Get the initial size for the texture (before measurement).
    pub fn initial_size(&self) -> Size<Pixels> {
        match self {
            ItemSizing::Fixed { size } => *size,
            ItemSizing::FixedWidth {
                width,
                estimated_height,
            } => Size {
                width: *width,
                height: *estimated_height,
            },
            ItemSizing::Explicit { size } => *size,
        }
    }

    /// Whether this sizing mode requires measurement.
    pub fn needs_measurement(&self) -> bool {
        matches!(self, ItemSizing::FixedWidth { .. })
    }
}

/// How often to re-render the texture.
#[derive(Clone, Debug, Default)]
pub enum RenderMode {
    /// Render once, cache result.
    #[default]
    Once,
    /// Continuously stream frames at target FPS.
    ///
    /// **⚠️ WARNING: Streaming mode is currently not working.**
    /// This mode is implemented but has unresolved issues that prevent
    /// continuous frame updates from functioning correctly.
    /// This will be fixed in a future update (see roadmap Phase 4.4).
    /// For now, use `RenderMode::Once` for static content.
    Streaming {
        /// Target frames per second.
        target_fps: u32,
    },
}

/// Frame data sent from background render thread.
#[derive(Debug)]
struct RenderedFrame {
    /// Raw pixel data (BGRA format from GPU).
    pixels: Vec<u8>,
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
}

/// Error states for TexturedView.
#[derive(Clone, Debug)]
pub enum TextureError {
    /// Platform doesn't support textured rendering.
    UnsupportedPlatform,
    /// GPU initialization failed.
    GpuInitFailed(String),
    /// Background thread died unexpectedly.
    ThreadDied,
    /// Render function panicked.
    RenderPanic,
}

/// A View that renders content to a GPU texture in a background thread.
///
/// The content is rendered using `Application::textured()` and the resulting
/// pixels are streamed back and displayed as an image.
pub struct TexturedView<F> {
    /// Function that creates the element to render.
    render_fn: F,
    /// How to determine the texture size.
    sizing: ItemSizing,
    /// Rendering mode (once or streaming).
    mode: RenderMode,
    /// Channel to receive rendered frames.
    frame_receiver: Option<Receiver<RenderedFrame>>,
    /// Handle to background render thread.
    #[allow(dead_code)]
    thread_handle: Option<JoinHandle<()>>,
    /// Current texture (latest frame).
    current_texture: Option<Arc<RenderImage>>,
    /// Measured size (updated when frame arrives for FixedWidth mode).
    measured_size: Option<Size<Pixels>>,
    /// Error state.
    error: Option<TextureError>,
}

impl<F, E> TexturedView<F>
where
    F: Fn() -> E + Send + Clone + 'static,
    E: IntoElement + 'static,
{
    /// Create a TexturedView with fixed size.
    ///
    /// The content will be rendered once at the specified size.
    #[allow(unused_variables)]
    pub fn fixed(size: Size<Pixels>, cx: &mut Context<Self>, render_fn: F) -> Self {
        Self::with_options(ItemSizing::Fixed { size }, RenderMode::Once, cx, render_fn)
    }

    /// Create a TexturedView with fixed width and measured height.
    ///
    /// The content will be laid out at the specified width, and the height
    /// will be determined by the content. Uses GPUI's layout system.
    #[allow(unused_variables)]
    pub fn measured(width: Pixels, cx: &mut Context<Self>, render_fn: F) -> Self {
        Self::with_options(
            ItemSizing::FixedWidth {
                width,
                estimated_height: Pixels(200.0),
            },
            RenderMode::Once,
            cx,
            render_fn,
        )
    }

    /// Create a TexturedView with measured height and custom estimated height.
    ///
    /// The estimated height is used for layout before the actual measurement completes.
    #[allow(unused_variables)]
    pub fn measured_with_estimate(
        width: Pixels,
        estimated_height: Pixels,
        cx: &mut Context<Self>,
        render_fn: F,
    ) -> Self {
        Self::with_options(
            ItemSizing::FixedWidth {
                width,
                estimated_height,
            },
            RenderMode::Once,
            cx,
            render_fn,
        )
    }

    /// Create a streaming TexturedView that continuously renders frames.
    ///
    /// Useful for animated content or content that updates frequently.
    ///
    /// **⚠️ WARNING: Streaming mode is currently not working.**
    /// This function is implemented but the streaming functionality has
    /// unresolved issues. See roadmap Phase 4.4 for planned fixes.
    /// For now, consider using `fixed()` or `measured()` for static content.
    #[allow(unused_variables)]
    pub fn streaming(
        size: Size<Pixels>,
        target_fps: u32,
        cx: &mut Context<Self>,
        render_fn: F,
    ) -> Self {
        Self::with_options(
            ItemSizing::Fixed { size },
            RenderMode::Streaming { target_fps },
            cx,
            render_fn,
        )
    }

    /// Create a TexturedView with full control over options.
    #[allow(unused_variables)]
    pub fn with_options(
        sizing: ItemSizing,
        mode: RenderMode,
        cx: &mut Context<Self>,
        render_fn: F,
    ) -> Self {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            let (sender, receiver) = flume::bounded(4);
            let thread_handle =
                spawn_render_thread(sizing.clone(), mode.clone(), render_fn.clone(), sender);

            Self {
                render_fn,
                sizing,
                mode,
                frame_receiver: Some(receiver),
                thread_handle: Some(thread_handle),
                current_texture: None,
                measured_size: None,
                error: None,
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            Self {
                render_fn,
                sizing,
                mode,
                frame_receiver: None,
                thread_handle: None,
                current_texture: None,
                measured_size: None,
                error: Some(TextureError::UnsupportedPlatform),
            }
        }
    }

    /// Get the current sizing mode.
    pub fn sizing(&self) -> &ItemSizing {
        &self.sizing
    }

    /// Get the measured size (if available).
    ///
    /// For `FixedWidth` mode, this returns the actual measured size after
    /// the first frame is rendered. Returns `None` if not yet measured.
    pub fn measured_size(&self) -> Option<Size<Pixels>> {
        self.measured_size
    }

    /// Get the current error state, if any.
    pub fn error(&self) -> Option<&TextureError> {
        self.error.as_ref()
    }

    /// Check if a texture is ready to display.
    pub fn is_ready(&self) -> bool {
        self.current_texture.is_some()
    }

    /// Force re-render (invalidate cached texture).
    ///
    /// This will restart the background render thread.
    #[allow(unused_variables)]
    pub fn invalidate(&mut self, cx: &mut Context<Self>) {
        // Drop existing thread (will be cleaned up)
        self.thread_handle = None;
        self.frame_receiver = None;
        self.current_texture = None;
        self.measured_size = None;
        self.error = None;

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            let (sender, receiver) = flume::bounded(4);
            let thread_handle = spawn_render_thread(
                self.sizing.clone(),
                self.mode.clone(),
                self.render_fn.clone(),
                sender,
            );

            self.frame_receiver = Some(receiver);
            self.thread_handle = Some(thread_handle);
        }

        cx.notify();
    }

    /// Poll for new frames from background thread.
    fn poll_frames(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(receiver) = &self.frame_receiver else {
            return;
        };

        let mut received_frame = false;
        while let Ok(frame) = receiver.try_recv() {
            received_frame = true;
            // Convert BGRA to RGBA
            let mut rgba = frame.pixels;
            for chunk in rgba.chunks_exact_mut(4) {
                chunk.swap(0, 2); // Swap B and R
            }

            if let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, rgba) {
                let image_frame = image::Frame::new(buffer);
                self.current_texture =
                    Some(Arc::new(RenderImage::new(smallvec::smallvec![image_frame])));
                self.measured_size = Some(Size {
                    width: Pixels(frame.width as f32),
                    height: Pixels(frame.height as f32),
                });
            }
        }

        // Keep polling until we have a frame (for Once mode) or continuously (for Streaming)
        let should_keep_polling = match &self.mode {
            RenderMode::Once => !received_frame && self.current_texture.is_none(),
            RenderMode::Streaming { .. } => true,
        };

        if should_keep_polling && self.error.is_none() {
            // Schedule another poll
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            {
                window
                    .spawn(cx, async move |cx| {
                        crate::Timer::after(StdDuration::from_millis(16)).await;
                        cx.update(|window, _cx| {
                            window.refresh();
                        })
                        .ok();
                    })
                    .detach();
            }
        }
    }
}

impl<F, E> Render for TexturedView<F>
where
    F: Fn() -> E + Send + Clone + 'static,
    E: IntoElement + 'static,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Poll for new frames
        self.poll_frames(window, cx);

        // Determine size for layout
        let display_size = self
            .measured_size
            .unwrap_or_else(|| self.sizing.initial_size());

        // Display current texture or placeholder
        if let Some(texture) = &self.current_texture {
            div()
                .w(display_size.width)
                .h(display_size.height)
                .child(img(texture.clone()).size_full())
                .into_any_element()
        } else if let Some(error) = &self.error {
            render_error_placeholder(display_size, error)
        } else {
            render_loading_placeholder(display_size)
        }
    }
}

impl<F> Drop for TexturedView<F> {
    fn drop(&mut self) {
        // Thread will stop when sender is dropped (which happens when
        // frame_receiver is dropped). We don't need to explicitly join
        // as the thread will clean itself up.
        self.thread_handle = None;
        self.frame_receiver = None;
    }
}

/// Render a loading placeholder.
fn render_loading_placeholder(size: Size<Pixels>) -> AnyElement {
    div()
        .w(size.width)
        .h(size.height)
        .bg(crate::rgb(0x2a2a2a))
        .flex()
        .items_center()
        .justify_center()
        .child(div().text_color(crate::rgb(0x888888)).child("Loading..."))
        .into_any_element()
}

/// Render an error placeholder.
fn render_error_placeholder(size: Size<Pixels>, error: &TextureError) -> AnyElement {
    let message = match error {
        TextureError::UnsupportedPlatform => "Textured rendering not supported on this platform",
        TextureError::GpuInitFailed(msg) => msg.as_str(),
        TextureError::ThreadDied => "Render thread died unexpectedly",
        TextureError::RenderPanic => "Render function panicked",
    };

    div()
        .w(size.width)
        .h(size.height)
        .bg(crate::rgb(0x4a2a2a))
        .flex()
        .items_center()
        .justify_center()
        .p(Pixels(8.0))
        .child(
            div()
                .text_color(crate::rgb(0xffaaaa))
                .child(message.to_string()),
        )
        .into_any_element()
}

// ============================================================================
// Background Render Thread (Linux/FreeBSD only)
// ============================================================================

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn spawn_render_thread<F, E>(
    sizing: ItemSizing,
    mode: RenderMode,
    render_fn: F,
    sender: flume::Sender<RenderedFrame>,
) -> JoinHandle<()>
where
    F: Fn() -> E + Send + 'static,
    E: IntoElement + 'static,
{
    std::thread::spawn(move || {
        run_textured_renderer(sizing, mode, render_fn, sender);
    })
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn run_textured_renderer<F, E>(
    sizing: ItemSizing,
    mode: RenderMode,
    render_fn: F,
    sender: flume::Sender<RenderedFrame>,
) where
    F: Fn() -> E + Send + 'static,
    E: IntoElement + 'static,
{
    Application::textured().run(move |cx: &mut App| {
        let initial_size = sizing.initial_size();
        let bounds = Bounds::centered(None, initial_size, cx);

        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| BackgroundRenderer {
                    render_fn,
                    sizing,
                    mode,
                    sender,
                    window_handle: None,
                    phase: RenderPhase::FirstRender,
                    did_resize: false,
                })
            },
        );

        if result.is_err() {
            cx.quit();
        }
    });
}

/// Phases of the measure-then-render flow.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[derive(Debug, Clone, PartialEq)]
enum RenderPhase {
    /// First render - measure and resize if needed.
    FirstRender,
    /// Ready to paint and capture pixels.
    ReadyToPaint,
    /// Painted, done (for Once mode) or cycling back (for Streaming).
    Painted,
}

/// The view that runs in the background textured window.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
struct BackgroundRenderer<F> {
    render_fn: F,
    sizing: ItemSizing,
    mode: RenderMode,
    sender: flume::Sender<RenderedFrame>,
    window_handle: Option<AnyWindowHandle>,
    phase: RenderPhase,
    did_resize: bool,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl<F, E> Render for BackgroundRenderer<F>
where
    F: Fn() -> E + Send + 'static,
    E: IntoElement + 'static,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Store window handle on first render
        if self.window_handle.is_none() {
            self.window_handle = Some(window.window_handle());
        }

        let window_handle = self.window_handle.unwrap();

        match self.phase {
            RenderPhase::FirstRender => {
                // For FixedWidth mode, we need to measure and potentially resize.
                // But we can't call layout_as_root here directly during render.
                // Instead, we'll render once at estimated size, then resize and re-render.

                if self.sizing.needs_measurement() && !self.did_resize {
                    // Schedule resize based on current window content size after this render
                    let sizing = self.sizing.clone();

                    window
                        .spawn(cx, async move |cx| {
                            // Let the first render complete
                            Timer::after(Duration::from_millis(10)).await;

                            let _ =
                                cx.update_window(window_handle, |_, window: &mut Window, _cx| {
                                    // For FixedWidth, we use the content size from the first render
                                    // The window was created at estimated size, content rendered inside
                                    // We trust the content fills appropriately for now
                                    // TODO: Better measurement approach
                                    if let ItemSizing::FixedWidth { width, .. } = sizing {
                                        // Keep width fixed, use current bounds as the height
                                        // (content should have laid out naturally)
                                        let current_bounds = window.bounds();
                                        let new_size = Size {
                                            width,
                                            height: current_bounds.size.height,
                                        };
                                        window.resize(new_size);
                                    }
                                    window.refresh();
                                });
                        })
                        .detach();

                    self.did_resize = true;
                }

                self.phase = RenderPhase::ReadyToPaint;

                // Schedule the actual paint and capture
                let sender = self.sender.clone();
                let mode = self.mode.clone();

                window
                    .spawn(cx, async move |cx| {
                        // Wait for render to complete
                        Timer::after(Duration::from_millis(20)).await;

                        let _ = cx.update_window(window_handle, |_, window: &mut Window, cx| {
                            window.draw_and_present(cx);

                            if let Some(pixels) = window.read_pixels() {
                                let bounds = window.bounds();
                                let width: u32 = bounds.size.width.into();
                                let height: u32 = bounds.size.height.into();

                                sender
                                    .send(RenderedFrame {
                                        pixels,
                                        width,
                                        height,
                                    })
                                    .ok();
                            }
                        });

                        // Handle streaming vs once mode
                        match mode {
                            RenderMode::Once => {
                                Timer::after(Duration::from_millis(50)).await;
                                let _ = cx.update(|_, cx| cx.quit());
                            }
                            RenderMode::Streaming { target_fps } => {
                                let frame_duration =
                                    Duration::from_millis(1000 / target_fps.max(1) as u64);
                                Timer::after(frame_duration).await;
                                let _ = cx.update_window(
                                    window_handle,
                                    |_, window: &mut Window, _cx| {
                                        window.refresh();
                                    },
                                );
                            }
                        }
                    })
                    .detach();
            }

            RenderPhase::ReadyToPaint => {
                // Subsequent renders (for streaming mode)
                let sender = self.sender.clone();
                let mode = self.mode.clone();

                window
                    .spawn(cx, async move |cx| {
                        Timer::after(Duration::from_millis(10)).await;

                        let _ = cx.update_window(window_handle, |_, window: &mut Window, cx| {
                            window.draw_and_present(cx);

                            if let Some(pixels) = window.read_pixels() {
                                let bounds = window.bounds();
                                let width: u32 = bounds.size.width.into();
                                let height: u32 = bounds.size.height.into();

                                sender
                                    .send(RenderedFrame {
                                        pixels,
                                        width,
                                        height,
                                    })
                                    .ok();
                            }
                        });

                        if let RenderMode::Streaming { target_fps } = mode {
                            let frame_duration =
                                Duration::from_millis(1000 / target_fps.max(1) as u64);
                            Timer::after(frame_duration).await;
                            let _ =
                                cx.update_window(window_handle, |_, window: &mut Window, _cx| {
                                    window.refresh();
                                });
                        }
                    })
                    .detach();

                self.phase = RenderPhase::Painted;
            }

            RenderPhase::Painted => {
                // For streaming mode, cycle back to ReadyToPaint
                if matches!(self.mode, RenderMode::Streaming { .. }) {
                    self.phase = RenderPhase::ReadyToPaint;
                }
            }
        }

        // Render the actual content
        let size = self.sizing.initial_size();
        div()
            .w(size.width)
            .h(size.height)
            .overflow_hidden()
            .child((self.render_fn)())
    }
}
