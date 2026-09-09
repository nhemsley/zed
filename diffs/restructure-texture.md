# Texture Rendering Restructure — Preliminary Notes

> Status: preliminary. This document only organizes the considerations raised
> so far. No code has been read yet to produce it. The actual implementation
> plan will be written as a follow-up, informed by this document plus a real
> pass over the relevant code.

## Scope

**Wayland only, for now.** X11, macOS, and Windows are out of scope for this
round of work. All of the compositor-interaction and event-pumping
investigation below should be scoped to the Wayland backend; other platforms
can be revisited later once the Wayland design is proven.

## Goal

Support rendering to an **arbitrary number of textures, per window**, where
each rendered texture behaves like its own "window" for input purposes — i.e.
it can receive and handle mouse/keyboard events directed at it, not just be a
passive pixel buffer.

This is a step beyond every branch reviewed so far in `diffs/` (`gitui-render-to-texture`,
`gpui-render-to-texture`, `gpui-oneshot-render`, `textured-view`), all of which
treat a texture as a one-way output (scene in, pixels out) with no input
routing back to the content that produced it.

## Open questions to investigate before planning

### 1. Event system & event pumping

- How GPUI's event loop currently pumps input events to a single real
  `Window`, and what would need to change for one OS-level window to fan input
  out to N independently-addressable render targets.
- Whether "pumping" (the loop that drives redraw/dispatch) is tied to a single
  window/surface today, and what changes if a window owns multiple textures
  that each need their own layout/paint/dispatch cycle.
- How hit-testing, focus, and hover state would be scoped per-texture rather
  than per-window.

### 2. Compositor interaction

- How the existing Wayland backend talks to the compositor today (per the
  [Scope](#scope) note above, this is Wayland-only for now), and what
  assumptions that integration makes that a multi-texture-per-window model
  might break (e.g. one surface → one swapchain → one frame callback).
- Whether/how compositor-driven events (resize, frame callbacks, damage) need
  to be redistributed across multiple textures owned by one window.

### 3. Multiple render targets per window

- What "per window" should mean structurally: is a texture a child render
  target of a window, or does each texture need its own lightweight
  window-like context (as the `textured-view`/`gpui-oneshot-render` branches
  prototype with `TexturedSurfaceWindow`)?
- How this interacts with the render-to-texture mechanisms already explored
  (see the four existing branch summaries in `diffs/`) — in particular
  `gpui-render-to-texture`'s shared `render_batches`/`SharedRenderResources`
  extraction, which is the more reusable foundation of the two approaches
  seen so far.

### 4. Per-texture input handling

- Each texture-backed unit needs to receive mouse/keyboard events addressed to
  it specifically (not just to the owning window), and dispatch them through
  GPUI's normal input-handling pipeline (focus, hit test, `on_action`, IME,
  etc.) as if it were its own window.
- Needs a mapping from "this input event, in window coordinates" to "which
  texture it belongs to, and where within that texture."

## Reference implementation: `gpuix`

- `gpuix` bridges GPUI and JavaScript/React and may already solve a similar
  problem (embedding/addressing GPUI content from an external
  host with its own event/composition model).
- Action item: vendor `gpuix` into the repo (e.g. under `vendor/`) purely for
  reference while designing this, not as a dependency.

## Rendering & caching strategy

Two rendering modes should both be supported eventually, not just one:

- **Oneshot rendering** — render once on demand (as in `gpui-oneshot-render`
  / `textured-view`), for content that doesn't change every frame.
- **Normal render + cache** — render through the regular continuous
  layout/paint pipeline, but cache the resulting texture, and **skip layout
  and render entirely** on subsequent frames when the cached output is still
  valid — i.e. an explicit invalidation/dirty-tracking step gates whether a
  given texture's content is recomputed or reused as-is.

Both modes need to coexist with the per-texture input handling above: even a
cached/un-rendered texture still needs to receive and react to input (which
may itself be what invalidates the cache).

## Relationship to prior work

See the branch-by-branch breakdowns already in this directory:

- [`diffs/gitui-render-to-texture/structure.md`](gitui-render-to-texture/structure.md)
- [`diffs/gpui-render-to-texture/structure.md`](gpui-render-to-texture/structure.md)
- [`diffs/gpui-oneshot-render/structure.md`](gpui-oneshot-render/structure.md)
- [`diffs/textured-view/structure.md`](textured-view/structure.md)

All four solve "render GPUI content to a texture." None solve "treat that
texture as an addressable input target." That gap is the actual subject of
this restructure.

## Next steps

1. Vendor `gpuix` for reference.
2. Read the event pumping / dispatch code and the platform-compositor
   integration code named above.
3. Turn this document into a concrete plan: proposed types/APIs, which
   existing branch's rendering foundation to build on, and how input routing
   and cache invalidation are actually implemented.
