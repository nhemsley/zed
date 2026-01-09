//! Implementation Sketch: InfiniteCanvas textured_items API
//!
//! This file demonstrates how the `textured_items` API would be implemented internally.
//! It shows the integration between InfiniteCanvas and the textured rendering system.
//!
//! Note: This is a SKETCH - actual implementation would live in infinite-canvas crate.

use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use flume::{Receiver, Sender};
use gpui::{
    div, img, point, px, size, AnyElement, AnyWindowHandle, App, Application, Bounds, Context,
    Element, ElementId, GlobalElementId, IntoElement, LayoutId, Pixels, Point, RenderImage,
    Render, RenderOnce, Size, Timer, Window, WindowBounds, WindowOptions,
};
use image::{Frame, RgbaImage};
use smallvec::smallvec;

// ============================================================================
// Public API Types
// ============================================================================

/// Trait for data items that can be rendered as textures.
///
/// Items must be sendable to background threads and have a stable identity.
pub trait TexturedItemData: Send + 'static + Clone {
    type Id: Hash + Eq + Clone + Send + 'static;

    /// Unique identifier for caching and invalidation.
    fn id(&self) -> Self::Id;
}

/// Layout algorithms for positioning items on the canvas.
#[derive(Clone, Debug)]
pub enum Layout {
    /// Grid layout with fixed columns
    Grid { columns: usize, gap: Pixels },
    /// Pack layout (bin packing)
    Pack { padding: Pixels },
    /// Manual positions (items provide their own positions)
    Manual,
}

impl Default for Layout {
    fn default() -> Self {
        Layout::Grid {
            columns: 4,
            gap: px(16.),
        }
    }
}

// ============================================================================
// InfiniteCanvas Extension for Textured Items
// ============================================================================

/// Configuration for textured item rendering.
pub struct TexturedItemsConfig<T, F> {
    items: Vec<T>,
    render: F,
    item_width: Pixels,
    layout: Layout,
    render_scale: f32,
}

impl<T, F, E> TexturedItemsConfig<T, F>
where
    T: TexturedItemData,
    F: Fn(&T) -> E + Send + Clone + 'static,
    E: IntoElement,
{
    pub fn new(items: impl IntoIterator<Item = T>, render: F) -> Self {
        Self {
            items: items.into_iter().collect(),
            render,
            item_width: px(300.),
            layout: Layout::default(),
            render_scale: 1.0,
        }
    }

    /// Set the width for item layout (height is measured from content).
    pub fn item_width(mut self, width: Pixels) -> Self {
        self.item_width = width;
        self
    }

    /// Set the layout algorithm.
    pub fn layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }

    /// Set render scale (> 1.0 for higher quality at zoom).
    pub fn render_scale(mut self, scale: f32) -> Self {
        self.render_scale = scale;
        self
    }
}

// ============================================================================
// InfiniteCanvas with Textured Items Support
// ============================================================================

/// Extended InfiniteCanvas that supports textured item rendering.
pub struct TexturedInfiniteCanvas<T, F>
where
    T: TexturedItemData,
{
    id: &'static str,
    config: Option<TexturedItemsConfig<T, F>>,
    // ... other canvas fields (camera, options, etc.)
}

impl<T, F, E> TexturedInfiniteCanvas<T, F>
where
    T: TexturedItemData,
    F: Fn(&T) -> E + Send + Clone + 'static,
    E: IntoElement,
{
    pub fn new(id: &'static str) -> Self {
        Self { id, config: None }
    }

    /// Add textured items to the canvas.
    ///
    /// # Arguments
    /// * `items` - Collection of data items
    /// * `render` - Function to create an element from each item
    ///
    /// # Example
    /// ```ignore
    /// InfiniteCanvas::new("canvas")
    ///     .textured_items(cards, |card| {
    ///         div()
    ///             .bg(rgb(card.color))
    ///             .child(&card.title)
    ///     })
    /// ```
    pub fn textured_items(
        mut self,
        items: impl IntoIterator<Item = T>,
        render: F,
    ) -> Self {
        self.config = Some(TexturedItemsConfig::new(items, render));
        self
    }

    /// Set item width for layout.
    pub fn item_width(mut self, width: Pixels) -> Self {
        if let Some(config) = &mut self.config {
            config.item_width = width;
        }
        self
    }

    /// Set layout algorithm.
    pub fn layout(mut self, layout: Layout) -> Self {
        if let Some(config) = &mut self.config {
            config.layout = layout;
        }
        self
    }
}

// ============================================================================
// Internal: Textured Item Manager
// ============================================================================

/// Manages the lifecycle of textured item rendering.
struct TexturedItemManager<T: TexturedItemData> {
    /// Rendered textures by item ID
    textures: HashMap<T::Id, TexturedItemState>,
    /// Channel to receive rendered frames
    frame_receiver: Receiver<RenderedFrame<T::Id>>,
    /// Channel sender (cloned for each background renderer)
    frame_sender: Sender<RenderedFrame<T::Id>>,
    /// Active render threads
    active_renders: HashMap<T::Id, JoinHandle<()>>,
}

#[derive(Clone)]
enum TexturedItemState {
    /// Not yet rendered
    Pending,
    /// Currently being rendered
    Rendering,
    /// Rendered and cached
    Ready {
        image: Arc<RenderImage>,
        size: Size<Pixels>,
    },
    /// Render failed
    Failed(String),
}

struct RenderedFrame<Id> {
    id: Id,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl<T: TexturedItemData> TexturedItemManager<T> {
    fn new() -> Self {
        let (sender, receiver) = flume::bounded(32);
        Self {
            textures: HashMap::new(),
            frame_receiver: receiver,
            frame_sender: sender,
            active_renders: HashMap::new(),
        }
    }

    /// Poll for completed renders.
    fn poll(&mut self) -> bool {
        let mut received = false;

        while let Ok(frame) = self.frame_receiver.try_recv() {
            received = true;

            // Convert BGRA to RGBA
            let mut rgba = frame.pixels;
            for chunk in rgba.chunks_exact_mut(4) {
                chunk.swap(0, 2);
            }

            if let Some(buffer) = RgbaImage::from_raw(frame.width, frame.height, rgba) {
                self.textures.insert(
                    frame.id.clone(),
                    TexturedItemState::Ready {
                        image: Arc::new(RenderImage::new(smallvec![Frame::new(buffer)])),
                        size: size(px(frame.width as f32), px(frame.height as f32)),
                    },
                );
            }

            // Clean up thread handle
            self.active_renders.remove(&frame.id);
        }

        received
    }

    /// Request rendering of an item.
    fn request_render<F, E>(&mut self, item: &T, item_size: Size<Pixels>, render: F)
    where
        F: Fn(&T) -> E + Send + Clone + 'static,
        E: IntoElement,
        T: Clone,
    {
        let id = item.id();

        // Skip if already rendering or ready
        if matches!(
            self.textures.get(&id),
            Some(TexturedItemState::Rendering) | Some(TexturedItemState::Ready { .. })
        ) {
            return;
        }

        self.textures.insert(id.clone(), TexturedItemState::Rendering);

        let item_clone = item.clone();
        let sender = self.frame_sender.clone();
        let id_for_thread = id.clone();

        let handle = thread::spawn(move || {
            render_item_to_texture(item_clone, item_size, render, sender, id_for_thread);
        });

        self.active_renders.insert(id, handle);
    }

    /// Get the current state of an item's texture.
    fn get_state(&self, id: &T::Id) -> Option<&TexturedItemState> {
        self.textures.get(id)
    }

    /// Invalidate an item (force re-render).
    fn invalidate(&mut self, id: &T::Id) {
        self.textures.remove(id);
    }

    /// Invalidate all items.
    fn invalidate_all(&mut self) {
        self.textures.clear();
    }
}

// ============================================================================
// Internal: Background Rendering
// ============================================================================

/// Render a single item to a texture in a background thread.
fn render_item_to_texture<T, F, E, Id>(
    item: T,
    item_size: Size<Pixels>,
    render: F,
    sender: Sender<RenderedFrame<Id>>,
    id: Id,
) where
    T: Send + 'static,
    F: Fn(&T) -> E + Send + 'static,
    E: IntoElement,
    Id: Send + 'static,
{
    Application::textured().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, item_size, cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| TexturedRenderWrapper {
                    item,
                    render,
                    sender,
                    id,
                    window_handle: None,
                    rendered: false,
                })
            },
        )
        .ok();
    });
}

/// Wrapper that handles the texture capture plumbing.
/// The inner item and render function know nothing about texturing.
struct TexturedRenderWrapper<T, F, Id> {
    item: T,
    render: F,
    sender: Sender<RenderedFrame<Id>>,
    id: Id,
    window_handle: Option<AnyWindowHandle>,
    rendered: bool,
}

impl<T, F, E, Id> Render for TexturedRenderWrapper<T, F, Id>
where
    F: Fn(&T) -> E + 'static,
    E: IntoElement,
    Id: Clone + Send + 'static,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.window_handle.is_none() {
            self.window_handle = Some(window.window_handle());
        }

        if !self.rendered {
            self.rendered = true;

            let sender = self.sender.clone();
            let id = self.id.clone();
            let window_handle = self.window_handle.unwrap();

            window
                .spawn(cx, async move |cx| {
                    Timer::after(std::time::Duration::from_millis(10)).await;

                    cx.update_window(window_handle, |_, window, cx| {
                        window.draw_and_present(cx);

                        if let Some(pixels) = window.read_pixels() {
                            let bounds = window.bounds();
                            let width: u32 = bounds.size.width.into();
                            let height: u32 = bounds.size.height.into();

                            sender
                                .send(RenderedFrame {
                                    id,
                                    pixels,
                                    width,
                                    height,
                                })
                                .ok();
                        }
                    })
                    .ok();

                    Timer::after(std::time::Duration::from_millis(50)).await;
                    cx.update(|cx| cx.quit()).ok();
                })
                .detach();
        }

        // Render the actual content using the user's render function
        div().size_full().child((self.render)(&self.item))
    }
}

// ============================================================================
// Internal: Layout Algorithms
// ============================================================================

struct LayoutResult<Id> {
    positions: HashMap<Id, Point<Pixels>>,
    sizes: HashMap<Id, Size<Pixels>>,
}

fn compute_layout<T: TexturedItemData>(
    items: &[T],
    layout: &Layout,
    item_width: Pixels,
    // In real impl, would measure content here
) -> LayoutResult<T::Id> {
    let mut positions = HashMap::new();
    let mut sizes = HashMap::new();

    match layout {
        Layout::Grid { columns, gap } => {
            let item_height = px(200.); // Would be measured in real impl

            for (i, item) in items.iter().enumerate() {
                let col = i % *columns;
                let row = i / *columns;

                let x = px(col as f32) * (item_width + *gap);
                let y = px(row as f32) * (item_height + *gap);

                positions.insert(item.id(), point(x, y));
                sizes.insert(item.id(), size(item_width, item_height));
            }
        }
        Layout::Pack { padding } => {
            // Simple horizontal packing for sketch
            let mut x = *padding;
            let item_height = px(200.);

            for item in items {
                positions.insert(item.id(), point(x, *padding));
                sizes.insert(item.id(), size(item_width, item_height));
                x += item_width + *padding;
            }
        }
        Layout::Manual => {
            // User provides positions via trait
            // Not implemented in sketch
        }
    }

    LayoutResult { positions, sizes }
}

// ============================================================================
// Example Usage (What user code looks like)
// ============================================================================

#[cfg(test)]
mod example {
    use super::*;

    #[derive(Clone)]
    struct Card {
        id: String,
        title: String,
        content: String,
        color: u32,
    }

    impl TexturedItemData for Card {
        type Id = String;

        fn id(&self) -> String {
            self.id.clone()
        }
    }

    fn example_usage() {
        let cards = vec![
            Card {
                id: "card-1".into(),
                title: "Hello".into(),
                content: "World".into(),
                color: 0x3498db,
            },
            Card {
                id: "card-2".into(),
                title: "Foo".into(),
                content: "Bar".into(),
                color: 0xe74c3c,
            },
        ];

        // This is what user code looks like - beautifully simple!
        let _canvas = TexturedInfiniteCanvas::new("my-canvas")
            .textured_items(cards, |card| {
                div()
                    .p(px(16.))
                    // .bg(rgb(card.color))  // Would use actual gpui
                    .child(&card.title)
                    .child(&card.content)
            })
            .item_width(px(300.))
            .layout(Layout::Grid {
                columns: 3,
                gap: px(16.),
            });
    }
}
