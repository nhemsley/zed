# `gpui-oneshot-render` — structure

7 commits, diffed against `origin/main`. This is the branch `textured-view` is later
built on top of (they share the same 7 commits at the base). Where
`gpui-render-to-texture` refactors `BladeRenderer` to add an offscreen path
*alongside* normal window rendering, this branch takes a different, more
radical approach: it adds an entirely new Linux platform backend whose
windows never touch a real display surface at all — `draw()` always renders
to an offscreen GPU texture and buffers the pixels for readback. "One-shot"
refers to using this to render a view once and export it, with no event loop
needed.

## `crates/gpui/src/platform/linux/textured_surface/` *(new directory)*

A full parallel `LinuxClient`/`PlatformWindow` implementation, analogous in
shape to the existing `wayland`/`x11` backends but with no compositor
connection at all:

- **`mod.rs`** — wires the three submodules together.
- **`client.rs`** — `TexturedSurfaceClient` implements `LinuxClient` using a
  plain `calloop::EventLoop` (no Wayland/X11 connection). Clipboard, cursor,
  screen capture, `open_uri`, etc. are all no-ops or `None`/errors — this client
  only exists to construct `TexturedSurfaceWindow`s.
- **`display.rs`** — `TexturedSurfaceDisplay` is a fake 1920x1080
  `PlatformDisplay` with a zeroed UUID, just enough to satisfy layout code that
  asks a window which display it's on.
- **`window.rs`** (the bulk of the branch, ~1193 lines) — `TexturedSurfaceWindow`
  /`TexturedSurfaceWindowState`: a complete, independent Blade rendering setup
  (its own `gpu::Context`, `BladeAtlas`, `command_encoder`, `instance_belt`,
  path-intermediate/MSAA textures, and a `TexturedSurfacePipelines` bundle with
  quads/shadows/paths/underlines/mono-sprites/poly-sprites pipelines —
  everything `BladeRenderer` has *except* a swapchain). Notably, this
  duplicates the pipeline/shader setup and the per-batch draw loop rather than
  sharing it with `BladeRenderer` (contrast with `gpui-render-to-texture`,
  which extracted `render_batches` precisely to avoid this). `PlatformWindow::draw`
  calls `render_scene_to_texture` then `read_pixels_to_buffer`, storing the
  result in a `rendered_pixels: Option<Vec<u8>>` field returned later by a new
  `read_pixels()` trait method. `resize` recreates all GPU textures at the new
  size (needed because, unlike a window, there's no OS-driven resize event).

## `crates/gpui/src/`

- **`platform.rs`** — adds `textured_platform()` (Linux/FreeBSD only),
  constructing a `TexturedSurfaceClient`, and a default `PlatformWindow::read_pixels() -> Option<Vec<u8>>`
  (`None` for every existing backend) that `TexturedSurfaceWindow` overrides.
- **`platform/linux.rs`** — registers the `textured_surface` module.
- **`app.rs`** — adds `Application::textured()`, parallel to the existing
  `Application::headless()`, building an `App` on top of `textured_platform()`.
  Doc comment lists the intended use cases: one-shot PNG export, thumbnails,
  embedding GPUI in 3D, visual testing.
- **`window.rs`** — adds `Window::read_pixels()` (forwards to the platform
  window; documented as BGRA, Linux/`TexturedSurfaceWindow`-only for now) and
  `Window::draw_and_present(cx)`, a convenience that does `refresh` → `draw` →
  `present` in one call and returns whether it succeeded — useful because a
  one-shot caller has no event loop driving those steps automatically.

## `crates/gpui/examples/` and `crates/gpui/research/` (crate-local)

- **`textured_surface.rs`** — minimal example: `Application::textured()`, open a
  window, render one view, save `read_pixels()` to a PNG.
- **`multi_textured_surface.rs`** — renders the same window twice with different
  state and combines both captures into a single output PNG, demonstrating
  that a `TexturedSurfaceWindow` can be redrawn repeatedly, not just once.
- **`multi_app_textured.rs`** — the most involved example: runs a second GPUI
  `Application::textured()` instance on a *background thread*, streams its
  rendered frames over a `flume` channel, and displays them inside a normal
  foreground window via `img()`. This requires disabling GPUI's main-thread
  assertion in `App::new_app`, which the example's doc comment calls out
  explicitly — a real caveat of this whole approach.
- **`research/infinite_canvas_textured_api.md`** and **`research/infinite_canvas_textured_impl.rs`**
  — a design doc + implementation sketch for a hypothetical `InfiniteCanvas`
  `textured_items` API (motivating use case: a git-history viewer showing one
  texture per commit/file diff), built on top of the `Application::textured()` +
  background-thread + channel pattern demonstrated in `multi_app_textured.rs`.
  Explicitly marked as a sketch, not the real implementation.

## `research/` (repo root, not crate-local)

Five standalone investigation documents, evidently written before/alongside
the implementation to work out the design:

- **`one-shot-rendering-architecture.md`** — the general problem statement: how
  to capture GPUI UI as pixels without an event loop or visible window.
- **`SOLUTION-one-shot-rendering.md`** — proposes reusing the existing (public)
  `Application::headless()` plus a `HeadlessBladeRenderer`; concludes "no
  feature flags needed, minimal GPUI changes required." This is *not* the path
  actually taken — the branch instead adds the new `textured_surface` platform
  and `Application::textured()`, i.e. `SOLUTION-textured-surface.md`'s
  proposal, not this one.
- **`SOLUTION-textured-surface.md`** — the design doc matching what was actually
  built: a new `TexturedSurface` platform client/window for Linux, reusing
  `BladeRenderer`'s ideas without disturbing the existing headless client used
  by CLI tools/remote server/benchmarks.
- **`embedding-gpui-in-3d.md`** — broader motivation doc: using texture-rendered
  GPUI as UI panels inside 3D/VR/AR scenes, in-game UI, digital twins, etc.
- **`render-to-texture-critique.md`** — a critical review of an *earlier*
  render-to-texture attempt (labeled "Branch: nhemsley/gpui-render-to-texture",
  commit `9ad72c6`) whose complaint — "outputs a hardcoded gradient image
  regardless of what view is provided" — matches exactly what
  `diffs/gitui-render-to-texture/structure.md` documents about that branch's
  `TextureRenderer::render_to_png` stub. This document appears to be the
  motivation for abandoning that approach and starting the `textured_surface`
  design pursued here.

## Overall shape

A genuinely working end-to-end path (unlike `gitui-render-to-texture`'s stub):
real views render to real pixels via a self-contained new platform backend.
The cost is duplication — a second, parallel copy of Blade's pipeline/shader
plumbing and draw loop that has to be kept in sync with `BladeRenderer` by
hand — and a Linux-only, non-reentrant, whole-new-platform design rather than
an addition to the existing renderer. The `research/` docs make the design
reasoning explicit, including an explicit rejection of the simpler
"extend `Application::headless()`" alternative in favor of this new platform.
