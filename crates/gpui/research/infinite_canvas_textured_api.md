# InfiniteCanvas Textured Items API Design

> Clean API for rendering data-driven items as streamed textures on an infinite canvas.

## Primary Use Case

**Git History Viewer**: Display file changes across commits on an infinite canvas, where each
item shows a diff for a specific file in a specific commit. Items are identified by their
natural domain key (`commit_hash:file_name`), rendered as textures for efficient pan/zoom,
and laid out automatically by the canvas.

## Goal

Provide the simplest possible API for rendering a collection of data items as textured elements on an infinite canvas, hiding all the complexity of:

- Background thread management
- `Application::textured()` setup
- Pixel streaming via channels
- BGRA→RGBA conversion
- Texture caching
- Layout/positioning algorithms
- Pan/zoom with viewport culling

## Proposed API

### Basic Usage

```rust
struct Card {
    id: String,
    title: String,
    content: String,
    color: u32,
}

let cards: Vec<Card> = load_cards();

InfiniteCanvas::new("canvas")
    .camera(Camera::default())
    .options(CanvasOptions::new().show_grid(true))
    .textured_items(cards, |card| {
        div()
            .p_4()
            .bg(rgb(card.color))
            .rounded_lg()
            .child(div().text_xl().child(&card.title))
            .child(div().text_sm().child(&card.content))
    })
```

That's it. The user provides:
1. **Data** - A collection of items
2. **Render function** - How to turn each item into an element

The canvas handles everything else.

## API Signature

```rust
impl<D> InfiniteCanvas<D> {
    /// Render items as streamed textures.
    ///
    /// Each item is rendered in a background `Application::textured()` thread,
    /// and the resulting pixels are streamed back and displayed as images.
    ///
    /// # Arguments
    /// * `items` - Collection of data items to render
    /// * `render` - Function that creates an element from each item
    ///
    /// # Type Parameters
    /// * `T` - The data type for each item
    /// * `F` - The render function type
    /// * `E` - The element type returned by the render function
    pub fn textured_items<T, F, E>(
        self,
        items: impl IntoIterator<Item = T>,
        render: F,
    ) -> Self
    where
        T: TexturedItemData,                    // Has ID, Send + 'static
        F: Fn(&T) -> E + Send + Clone + 'static, // Render function
        E: IntoElement,                          // Returns an element
    {
        // Implementation handles all the plumbing
    }
}
```

## Supporting Traits

### TexturedItemData

Items need to provide an ID for caching/invalidation:

```rust
pub trait TexturedItemData: Send + 'static {
    type Id: Hash + Eq + Clone + Send + 'static;
    
    fn id(&self) -> Self::Id;
}

// Blanket impl for types with an `id` field of type String
impl<T> TexturedItemData for T
where
    T: Send + 'static,
    T: HasId,
{
    type Id = String;
    
    fn id(&self) -> Self::Id {
        self.id.clone()
    }
}

// Or manual impl
impl TexturedItemData for Card {
    type Id = String;
    
    fn id(&self) -> Self::Id {
        self.id.clone()
    }
}
```

### Alternative: No Trait Required

If we want to avoid requiring a trait, we can use a tuple or wrapper:

```rust
// Option 1: Require ID in a tuple
.textured_items(cards.iter().map(|c| (c.id.clone(), c)), |(id, card)| {
    div().child(&card.title)
})

// Option 2: Separate ID extractor
.textured_items_with_id(cards, |c| &c.id, |card| {
    div().child(&card.title)
})

// Option 3: Use index as implicit ID (simplest, but less stable)
.textured_items(cards, |card| {
    div().child(&card.title)
})  // Uses index 0, 1, 2... as IDs
```

## Layout Options

The canvas needs to know how to position items. Options:

### Option A: Automatic Layout (Default)

Canvas uses a built-in layout algorithm (grid, pack, etc.):

```rust
InfiniteCanvas::new("canvas")
    .textured_items(cards, |card| { ... })
    .layout(Layout::Grid { columns: 4, gap: px(16.) })
```

### Option B: Explicit Positions

User provides positions in the data:

```rust
struct Card {
    id: String,
    position: Point<Pixels>,  // User specifies position
    // ...
}

impl TexturedItemData for Card {
    fn id(&self) -> String { self.id.clone() }
    fn position(&self) -> Option<Point<Pixels>> { Some(self.position) }
}
```

### Option C: Layout Closure

User provides a layout function:

```rust
InfiniteCanvas::new("canvas")
    .textured_items(cards, |card| { ... })
    .layout_with(|items| {
        // Returns positions for each item
        items.iter().enumerate().map(|(i, item)| {
            let col = i % 4;
            let row = i / 4;
            (item.id(), point(px(col * 300), px(row * 200)))
        }).collect()
    })
```

## Sizing Options

Items need sizes for layout. Options:

### Option A: Measure Content (Default)

Canvas measures each item's content at a given width:

```rust
InfiniteCanvas::new("canvas")
    .textured_items(cards, |card| { ... })
    .item_width(px(300.))  // Fixed width, measure height
```

### Option B: Fixed Size

All items same size:

```rust
.textured_items(cards, |card| { ... })
.item_size(size(px(300.), px(200.)))
```

### Option C: Per-Item Size

Size from data:

```rust
struct Card {
    size: Size<Pixels>,
    // ...
}

// Or via closure
.textured_items_sized(cards, |card| card.preferred_size(), |card| { ... })
```

## Real-World Example: Git History Viewer

This is the primary use case driving this design - a git history viewer where each
canvas item shows a file change within a commit.

```rust
use gpui::*;
use infinite_canvas::prelude::*;

#[derive(Clone)]
struct FileChange {
    commit_hash: String,
    file_name: String,
    diff_content: String,
    additions: usize,
    deletions: usize,
}

impl TexturedItemData for FileChange {
    type Id = String;
    
    fn id(&self) -> String {
        // Natural composite key from domain data
        format!("{}:{}", self.commit_hash, self.file_name)
    }
}

struct GitHistoryView {
    file_changes: Vec<FileChange>,
}

impl Render for GitHistoryView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        InfiniteCanvas::new("git-history")
            .camera(Camera::default())
            .options(CanvasOptions::new().show_grid(true))
            .textured_items(self.file_changes.clone(), |change| {
                div()
                    .p_4()
                    .bg(rgb(0x1e1e2e))
                    .rounded_lg()
                    .flex()
                    .flex_col()
                    .gap_2()
                    // Commit hash (short)
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x888888))
                            .child(&change.commit_hash[..8])
                    )
                    // File name
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(white())
                            .child(&change.file_name)
                    )
                    // Stats (+/-)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(rgb(0x2ecc71))
                                    .child(format!("+{}", change.additions))
                            )
                            .child(
                                div()
                                    .text_color(rgb(0xe74c3c))
                                    .child(format!("-{}", change.deletions))
                            )
                    )
                    // Diff content
                    .child(
                        div()
                            .text_xs()
                            .font_family("monospace")
                            .text_color(rgb(0xcccccc))
                            .child(&change.diff_content)
                    )
            })
            .item_width(px(350.))
            .layout(Layout::Grid { columns: 4, gap: px(16.) })
    }
}
```

### Invalidation

When a commit is amended or a file changes, invalidate by the natural key:

```rust
// Re-render a specific file change
canvas.invalidate(&format!("{}:{}", commit_hash, file_name));

// Or re-render all items for a commit
for file in files_in_commit {
    canvas.invalidate(&format!("{}:{}", commit_hash, file));
}
```

---

## Generic Example

```rust
use gpui::*;
use infinite_canvas::prelude::*;

#[derive(Clone)]
struct Card {
    id: String,
    title: String,
    paragraphs: Vec<String>,
    color: u32,
}

impl TexturedItemData for Card {
    type Id = String;
    fn id(&self) -> String { self.id.clone() }
}

struct MyApp {
    cards: Vec<Card>,
}

impl Render for MyApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        InfiniteCanvas::new("my-canvas")
            .camera(Camera::default())
            .options(
                CanvasOptions::new()
                    .show_grid(true)
                    .min_zoom(0.1)
                    .max_zoom(5.0)
            )
            .textured_items(self.cards.clone(), |card| {
                // This runs in a background thread!
                // The element is rendered to a texture and streamed back.
                div()
                    .p_4()
                    .bg(rgb(card.color))
                    .rounded_lg()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(white())
                            .child(&card.title)
                    )
                    .children(card.paragraphs.iter().map(|p| {
                        div()
                            .text_sm()
                            .text_color(rgba(0xffffffcc))
                            .child(p.clone())
                    }))
            })
            .item_width(px(300.))  // Layout at 300px width
            .layout(Layout::Pack { padding: px(16.) })
    }
}
```

## Internal Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Main Application                        │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              InfiniteCanvas Element                  │   │
│  │  - Handles pan/zoom input                           │   │
│  │  - Manages TexturedItemManager                      │   │
│  │  - Renders visible items as img() elements          │   │
│  └─────────────────────────────────────────────────────┘   │
│                           │                                 │
│                           │ polls for frames                │
│                           ▼                                 │
│  ┌─────────────────────────────────────────────────────┐   │
│  │            TexturedItemManager                       │   │
│  │  - Tracks which items need rendering                │   │
│  │  - Spawns background renderers for visible items    │   │
│  │  - Caches rendered textures                         │   │
│  │  - Handles invalidation                             │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                           │
           flume channels  │  (pixels + metadata)
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              Background Renderer Threads                    │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐        │
│  │ App:textured │ │ App:textured │ │ App:textured │  ...   │
│  │   Item #1    │ │   Item #2    │ │   Item #3    │        │
│  └──────────────┘ └──────────────┘ └──────────────┘        │
│                                                             │
│  Each runs Application::textured().run() with:             │
│  - TexturedViewWrapper<UserElement>                        │
│  - Renders once, sends pixels, quits                       │
└─────────────────────────────────────────────────────────────┘
```

## Open Questions

1. **Streaming vs One-Shot**: Should items continuously stream (for animations) or render once?
   - Default: one-shot for static content
   - Option: `.streaming()` for continuous updates

2. **Invalidation**: How does user trigger re-render of an item?
   - `canvas.invalidate(item_id)`?
   - Automatic when data changes (requires Eq/PartialEq)?

3. **Render Scale**: Support for rendering at different resolutions?
   - `.render_scale(2.0)` for crisp textures at any zoom?

4. **Memory Limits**: LRU cache for textures?
   - `.max_cached_textures(100)`?

5. **Error Handling**: What happens if a background render fails?
   - Show placeholder? Retry? Callback?

## Implementation Phases

### Phase 1: Basic API
- `textured_items(items, render)` working
- One-shot rendering (no streaming)
- Simple grid layout
- No caching (re-render on pan/zoom)

### Phase 2: Caching & Performance
- Texture cache with LRU eviction
- Only render visible items
- Invalidation API

### Phase 3: Advanced Features
- Streaming mode for animations
- Render scale support
- Multiple layout algorithms
- Per-item sizing

## Related Files

- `gpui/research/textured_view_design.md` - Lower-level TexturedView API
- `gpui/examples/multi_app_textured.rs` - Working streaming prototype
- `infinite-canvas/examples/textured2.rs` - Current sketch with manual plumbing