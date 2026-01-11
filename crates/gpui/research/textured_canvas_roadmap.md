# Textured Infinite Canvas - Implementation Roadmap

> Roadmap for implementing textured rendering support in GPUI and infinite-canvas.

## Known Issues ⚠️

- **Streaming mode (`RenderMode::Streaming`) is not working** - The streaming mode that allows continuous frame updates is currently broken. This will be addressed in Phase 4.4. For now, use `RenderMode::Once` for static content.

## Current State (What Works ✅)

| Component | Location | Status |
|-----------|----------|--------|
| `Application::textured()` | `gpui/src/app.rs` | ✅ Working |
| `TexturedSurfaceWindow` | `gpui/src/platform/linux/textured_surface/` | ✅ Working |
| `Window::resize()` | `textured_surface/window.rs` | ✅ Added |
| Streaming example | `gpui/examples/multi_app_textured.rs` | ✅ Working |
| Research docs | `gpui/research/` | ✅ Complete |

## Phase 1: TexturedView in GPUI 🔨

**Goal**: Core GPUI primitive for rendering any element to a texture in a background thread.

### Tasks

- [x] **1.1** Create `gpui/src/textured_view.rs` ✅
  - [x] `TexturedView<F>` struct with state (thread, channel, texture)
  - [x] `ItemSizing` enum (Fixed, FixedWidth, Explicit)
  - [x] `RenderMode` enum (Once, Streaming)
  - [x] Implement `Render` trait

- [x] **1.2** Background render thread logic ✅
  - [x] Spawn `Application::textured()` thread
  - [x] Measurement flow for `FixedWidth` (layout → resize → render)
  - [x] Pixel streaming via flume channel
  - [x] BGRA → RGBA conversion
  - [x] Thread cleanup on Drop

- [x] **1.3** Integration with GPUI ✅
  - [x] Add to `gpui/src/gpui.rs` exports
  - [x] Platform feature gates (Linux/FreeBSD only for now)
  - [x] Error handling for unsupported platforms

- [x] **1.4** Basic example ✅
  - [x] `gpui/examples/textured_view.rs` - simple usage
  - [x] Demonstrate all three `ItemSizing` modes (Fixed, Measured, Streaming)

### Deliverable ✅
```rust
// User can write:
let view = cx.new(|cx| {
    TexturedView::measured(px(300.), cx, || {
        div().bg(rgb(0x3498db)).child("Hello!")
    })
});
```

**Implemented in:** `gpui/src/textured_view.rs`
**Example:** `gpui/examples/textured_view.rs`

**Status: COMPLETE** ✅ (All 4 tasks done)

---

## Phase 2: Infinite Canvas Integration 🔨

**Goal**: Update infinite-canvas to use async textured rendering for canvas items.

### Tasks

- [x] **2.1** Clean up infinite-canvas cruft ✅
  - [x] Remove old synchronous `render_element_to_texture_impl`
  - [x] Replace with async `window.spawn()` pattern (matching `TexturedView`)

- [x] **2.2** Update `TexturedCanvasItemsProvider` ✅
  - [x] Async background rendering with flume channels
  - [x] Add `ItemSizing` support (`with_sizing()`, `set_sizing()`)
  - [x] Remove hardcoded 800x600 - uses `sizing.initial_size()`
  - [x] Multiple concurrent renders (`set_max_concurrent_renders()`)
  - [x] Import `AppContext` trait for proper method resolution

- [ ] **2.3** Canvas item API (DEFERRED to Phase 4)
  - [ ] Simple `textured_items(items, render_fn)` API on `InfiniteCanvas`
  - [ ] Per-item sizing via trait or closure
  - [ ] Invalidation support

- [x] **2.4** Examples ✅
  - [x] Update `infinite-canvas/examples/textured.rs`
  - [x] Code file cards with syntax highlighting (Dracula theme)
  - [x] Mouse-centered zoom
  - [x] Removed gpui-component dependency (incompatible API)

### Deliverable ✅
```rust
// Current API (provider-based):
let mut provider = TexturedCanvasItemsProvider::with_sizing(ItemSizing::FixedWidth {
    width: px(280.0),
    estimated_height: px(150.0),
});
provider.set_max_concurrent_renders(4);
provider.add_item("main.rs", || code_file_card("main.rs", code_parts));
```

**Status: MOSTLY COMPLETE** ✅ (2.3 deferred - convenience API can be added later)

---

## Phase 3: Color-Preserving Downscale 🔨

**Goal**: Preserve syntax highlighting colors when zoomed out.

### Tasks

- [x] **3.1** Implement saturation-preserving downscale ✅
  - [x] `most_saturated_color()` algorithm - `most_saturated_in_block()`
  - [x] Background separation variant - `furthest_from_bg_in_block()`
  - [x] Min/Max pooling variants for different backgrounds
  - [ ] Integrate into rendering pipeline (TODO)

- [x] **3.2** Add `DownscaleMode` option ✅
  - [x] `Linear` (default) - standard averaging
  - [x] `MostSaturated` - preserves colorful syntax tokens
  - [x] `FurthestFromBackground` - preserves foreground text
  - [x] `MinPool` - darkest pixel wins (dark text on light bg)
  - [x] `MaxPool` - brightest pixel wins (light text on dark bg)

- [x] **3.3** UI for mode selection ✅
  - [x] Dropdown in textured.rs example
  - [x] `DownscaleMode::all()` and `display_name()` for UI

- [ ] **3.4** Integration (TODO)
  - [ ] Pre-computed LOD in TexturedCanvasItemsProvider
  - [ ] GPU shader alternative
  - [ ] Automatic mode selection based on zoom level

### Deliverable ✅
```rust
// Downscale functions available:
let (pixels, w, h) = downscale_pixels(
    &rgba_data, width, height, 
    scale_factor,
    DownscaleMode::MostSaturated,
    bg_color,
);

// UI dropdown for mode selection in example
```

**Status: MOSTLY COMPLETE** ✅ (algorithms done, integration TODO)

---

## Phase 4: Performance & Polish 🔮

**Goal**: Production-ready performance and ergonomics.

### Tasks

- [ ] **4.1** Thread pooling
  - [ ] Global thread pool for TexturedViews
  - [ ] Configurable concurrency limit
  - [ ] Priority queue for visible items

- [ ] **4.2** Texture caching
  - [ ] LRU cache for rendered textures
  - [ ] Cache invalidation API
  - [ ] Memory limits

- [ ] **4.3** Viewport culling optimization
  - [ ] Only render visible canvas items
  - [ ] Hybrid: nearby items at low priority
  - [ ] Cancel pending renders for off-screen items

- [ ] **4.4** Streaming improvements
  - [ ] **Fix streaming mode** - Currently broken, needs debugging
  - [ ] Delta updates (only changed regions)
  - [ ] Frame skipping under load
  - [ ] Backpressure handling

---

## Phase 5: Platform Expansion 🔮

**Goal**: Support macOS and Windows.

### Tasks

- [ ] **5.1** macOS textured surface
  - [ ] Metal-based offscreen rendering
  - [ ] Integrate with existing Metal renderer

- [ ] **5.2** Windows textured surface
  - [ ] D3D11/12-based offscreen rendering
  - [ ] Integrate with existing renderer

- [ ] **5.3** Fallback mode
  - [ ] Synchronous main-thread rendering
  - [ ] For platforms without background support

---

## Timeline Estimate

| Phase | Effort | Priority | Status |
|-------|--------|----------|--------|
| Phase 1: TexturedView | 1-2 weeks | 🔴 High | ✅ DONE |
| Phase 2: Canvas Integration | 1 week | 🔴 High | ✅ DONE |
| Phase 3: Color Downscale | 3-5 days | 🟡 Medium | ✅ DONE (integration TODO) |
| Phase 4: Performance | 1-2 weeks | 🟡 Medium | 🔨 Next |
| Phase 5: Platform Expansion | 2-4 weeks | 🟢 Low | |

**MVP** = Phase 1 + Phase 2 ✅ COMPLETE

---

## Key Design Decisions

### Decided ✅

1. **TexturedView is a View, not Element** - needs persistent state
2. **Lives in gpui-proper** - fundamental primitive, not canvas-specific
3. **ItemSizing enum**: Fixed, FixedWidth, Explicit
4. **Window resize for measurement** - implemented in TexturedSurfaceWindow
5. **Flume channels for pixel streaming** - proven in multi_app_textured.rs

### Open Questions ❓

1. **Thread pool vs per-view threads?**
   - Per-view: simpler, more isolated
   - Pool: better resource management
   - *Leaning*: Start per-view, add pool in Phase 4

2. **How to handle render closure captures?**
   - Must be `Send + Clone + 'static`
   - Clone data into closure, or use `Arc`
   - *Decision needed during implementation*

3. **Invalidation granularity?**
   - Manual only vs automatic (data change detection)
   - *Leaning*: Manual first, automatic later

---

## Related Documents

- `gpui/research/textured_view_design.md` - TexturedView architecture
- `gpui/research/infinite_canvas_textured_api.md` - Canvas API design  
- `gpui/research/infinite_canvas_textured_impl.rs` - Implementation sketch
- `gpui/research/scaling-images.md` - Color-preserving downscale
- `gpui/examples/multi_app_textured.rs` - Working streaming prototype

---

## Success Metrics

1. **Functional**: Can render 1000+ canvas items as textures
2. **Performance**: Smooth pan/zoom at 60fps with texture streaming
3. **Quality**: Syntax highlighting visible at 10% zoom
4. **Ergonomic**: Simple API (< 10 lines for basic usage)