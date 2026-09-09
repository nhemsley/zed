# `gpui-render-to-texture` — structure

6 commits, diffed against `origin/main`. The most architecturally invasive of the
four branches: rather than bolting an offscreen path onto `BladeRenderer`, it
refactors the renderer's internals so the same draw logic can run against
either the window surface or a standalone texture, then builds a full public
API (`Window::create_offscreen_renderer`, `OffscreenRenderContext`) on top.
Gated behind a new `render-to-texture` Cargo feature, off by default.

## `crates/gpui/src/platform/blade/`

- **`blade_renderer.rs`** (largest single diff, ~915 changed lines) — this is a
  refactor commit ("Extract `SharedRenderResources` and `render_batches`"), not
  new functionality. It pulls the resources every draw call needs
  (`gpu`, `pipelines`, `atlas`, `atlas_sampler`, `rendering_parameters`) into a
  `SharedRenderResources` struct, and pulls the per-frame mutable state
  (`command_encoder`, `instance_belt`, path-intermediate textures) into a
  `RenderContext<'a>` struct. The per-`PrimitiveBatch` draw loop that used to
  be inlined in `BladeRenderer::draw` is extracted into a free function
  `render_batches(scene, target_view, viewport_size, premultiplied_alpha, ctx,
  shared)`, along with helper free functions `create_path_intermediate_texture`
  and `create_msaa_texture_if_needed`. `BladeRenderer::draw` becomes a thin
  caller of `render_batches`. This is what lets the new offscreen renderer
  reuse the *exact* same batch-drawing code instead of duplicating it (contrast
  with `gitui-render-to-texture`, which copy-pasted the loop).
  Also adds `BladeRenderer::create_offscreen_renderer(max_size)`, which builds a
  `BladeOffscreenRenderer` sharing its `Arc`-wrapped GPU resources.
- **`blade_offscreen_renderer.rs`** *(new)* — `BladeOffscreenRenderer`: owns its
  *own* `command_encoder`, `instance_belt`, and path-intermediate/MSAA textures
  (so it can run without racing the main renderer's per-frame state) but holds
  `Arc` clones of the main renderer's `gpu`, `pipelines`, and `atlas`. Manages a
  `HashMap<u64, OffscreenTexture>` of textures it owns. `draw_to_texture` calls
  the shared `render_batches` free function with `premultiplied_alpha: true`.
  `read_texture` copies a texture to a `Shared`-memory staging buffer, waits on
  the GPU, and does BGRA→RGBA conversion by hand while respecting row-pitch
  alignment (`(bytes_per_row + 255) & !255`), unlike `gitui-render-to-texture`'s
  raw byte copy. Notably does **not** handle `PrimitiveBatch::Surfaces` (macOS
  video surfaces) — documented as a known gap. Implements the crate's
  `PlatformOffscreenRenderer` trait and asserts `unsafe impl Send`.
- **`blade.rs`** — registers the new submodule and re-exports it, gated by
  `#[cfg(feature = "render-to-texture")]`.

## `crates/gpui/src/platform/`

- **`offscreen_rendering.rs`** *(new)* — the platform-agnostic vocabulary:
  `OffscreenTextureId`, `OffscreenTextureInfo`, `TextureData` (owns RGBA bytes
  plus `pixel_at`/`as_bytes`/`into_bytes` helpers), and the `PlatformOffscreenRenderer`
  trait (`create_texture`, `draw_to_texture`, `read_texture`, `destroy_texture`,
  `max_texture_size`, `destroy`) that `BladeOffscreenRenderer` implements.
- **`platform.rs`** — wires the module in behind the feature flag and adds
  `PlatformWindow::create_offscreen_renderer(max_texture_size) -> Option<Box<dyn
  PlatformOffscreenRenderer>>` with a default `None` implementation, so
  platforms that don't support it (macOS, Windows) simply opt out.
- **`linux/wayland/window.rs`**, **`linux/x11/window.rs`** — each implements
  `create_offscreen_renderer` by forwarding to the Blade renderer's method,
  wrapping the result in `Box::new(...)`. macOS and Windows are left with the
  trait default (`None`), so render-to-texture is Linux-only in this branch.

## `crates/gpui/src/`

- **`offscreen_render_context.rs`** *(new)* — `OffscreenRenderContext`: a
  `Window`-free rendering surface. Holds `scale_factor`, `viewport_size`, a
  content-mask stack, an opacity stack, a `Scene`, and shared `Arc<WindowTextSystem>` /
  `Arc<dyn PlatformAtlas>` handles borrowed from a real window. Exposes
  `paint_quad`, `paint_path`, `paint_underline`, `paint_glyph`, `push/pop_layer`,
  `push/pop_opacity`, `push/pop_content_mask`, and `take_scene`/`clear`. This is
  deliberately much thinner than `Window`: no focus, hit-testing, input
  handlers, persistent element state, or deferred draws — callers assemble
  primitives directly rather than going through the `Element` layout/prepaint/paint
  pipeline.
- **`window.rs`** — adds `Window::sprite_atlas()` (needed so `OffscreenRenderContext`
  can borrow it), and, behind the feature flag: `Window::create_offscreen_renderer`
  (delegates to `platform_window`, wraps in the new `OffscreenRenderer`) and
  `Window::create_offscreen_render_context` (constructs an `OffscreenRenderContext`
  sharing this window's text system/atlas/scale factor). Also defines the public
  `OffscreenRenderer` wrapper type, whose methods (`create_texture`,
  `draw_scene_to_texture`, `destroy_texture`, `read_texture`, `max_texture_size`,
  `destroy`) all forward to the boxed `PlatformOffscreenRenderer` trait object.
- **`scene.rs`** — makes `Scene` itself `pub` (was `pub(crate)`) with doc comments,
  since `OffscreenRenderContext::take_scene()` needs to hand a `Scene` back to
  application code across the crate boundary. `insert_primitive` moves the
  other direction, from `pub` to `pub(crate)`, so only GPUI's own paint helpers
  (not arbitrary callers) can push raw primitives in.
- **`gpui.rs`** — `mod`/`pub use` for `offscreen_render_context`.

## `crates/gpui/`

- **`Cargo.toml`** — adds the `render-to-texture = []` feature and registers the
  `render_to_texture` example.
- **`examples/render_to_texture.rs`** *(new)* — opens a hidden 100x100 window
  purely to get a live `Window`/GPU context, then uses
  `window.create_offscreen_renderer` + `create_offscreen_render_context` to
  paint three colored rounded rects onto a 256x256 texture, reads it back, and
  saves it as a PNG via `image::save_buffer`. Unlike `gitui-render-to-texture`'s
  example, this one actually exercises the full pipeline end-to-end.
- **`research/*.md`** *(new, 4 files)* — design notes written to justify and
  record the refactor:
  - `gpui-blade-rendering-pipeline.md` traces Element→Window→Frame→Scene→Blade
    to identify what needed separating.
  - `offscreen-render-context.md` is the design doc for `OffscreenRenderContext`.
  - `render-to-texture-methodology.md` states the guiding principles ("minimize
    changes to GPUI", "reuse existing rendering code", "share GPU resources").
  - `rtt-option-offscreen-renderer.md` argues for the separate-renderer-with-shared-resources
    approach by pointing out `BladeRenderer` is not reentrant (its
    `command_encoder`, `instance_belt`, and `path_intermediate_texture` are
    single-session state), which is exactly the problem `BladeOffscreenRenderer`
    solves by owning its own copies of that per-frame state.

## Overall shape

This branch treats render-to-texture as a first-class, shared-resource,
reentrant-safe subsystem: refactor the renderer to expose reusable pieces,
define platform-agnostic traits/types, implement them for Linux, and layer a
complete `Window`-level API and working example on top. It is the only branch
of the four that both (a) shares real rendering code between window and
texture paths instead of duplicating it, and (b) ships an example that
actually renders real content instead of placeholder pixels.
